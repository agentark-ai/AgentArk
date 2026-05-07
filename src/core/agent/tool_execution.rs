use super::*;
use anyhow::Context as _;
use std::sync::atomic::{AtomicBool, Ordering};

static GEPA_IDLE_WORKER_ACTIVE: AtomicBool = AtomicBool::new(false);

fn build_executor_client() -> Option<crate::clients::ExecutorClient> {
    let role = std::env::var("AGENTARK_STACK_ROLE")
        .ok()
        .map(|value| value.trim().to_ascii_lowercase());
    if !matches!(role.as_deref(), Some("control-plane" | "control")) {
        return None;
    }
    let client =
        crate::clients::ExecutorClient::new(crate::clients::ExecutorClientConfig::from_env())
            .ok()?;
    client.bearer_token()?;
    Some(client)
}

fn notification_tool_should_dispatch_for_surface(
    channel: &str,
    authorization: Option<&crate::actions::ActionAuthorizationContext>,
) -> bool {
    if let Some(authorization) = authorization {
        return matches!(
            authorization.surface,
            crate::actions::ActionExecutionSurface::Chat
                | crate::actions::ActionExecutionSurface::Api
        );
    }

    !matches!(
        channel.trim().to_ascii_lowercase().as_str(),
        "scheduler" | "watcher" | "background"
    )
}

fn list_watchers_status_label(status: &crate::core::watcher::WatcherStatus) -> &'static str {
    match status {
        crate::core::watcher::WatcherStatus::Active => "active",
        crate::core::watcher::WatcherStatus::Paused => "paused",
        crate::core::watcher::WatcherStatus::Triggered => "triggered",
        crate::core::watcher::WatcherStatus::TimedOut => "timed_out",
        crate::core::watcher::WatcherStatus::Cancelled => "cancelled",
        crate::core::watcher::WatcherStatus::Failed { .. } => "failed",
    }
}

fn list_watchers_live_row(watcher: &crate::core::watcher::Watcher) -> serde_json::Value {
    let status_error = match &watcher.status {
        crate::core::watcher::WatcherStatus::Failed { error } => Some(error.clone()),
        _ => None,
    };
    serde_json::json!({
        "id": watcher.id.to_string(),
        "description": watcher.description,
        "poll_action": watcher.poll_action,
        "poll_arguments": watcher.poll_arguments,
        "condition": watcher.condition,
        "status": list_watchers_status_label(&watcher.status),
        "status_error": status_error,
        "interval_secs": watcher.interval_secs,
        "timeout_secs": watcher.timeout_secs,
        "poll_count": watcher.poll_count,
        "created_at": watcher.created_at.to_rfc3339(),
        "last_poll_at": watcher.last_poll_at.as_ref().map(|value| value.to_rfc3339()),
        "next_poll_not_before": watcher.next_poll_not_before.as_ref().map(|value| value.to_rfc3339()),
        "notify_channel": watcher.notify_channel,
        "on_trigger": watcher.on_trigger,
        "trigger_result": watcher.trigger_result,
        "last_result": watcher.last_result,
        "last_error": watcher.last_error,
        "last_poll_outcome": watcher.last_poll_outcome,
        "notification_attempts": watcher.notification_attempts,
        "history_only": false,
    })
}

fn list_watchers_history_error_is_notification_summary_failure(error: Option<&str>) -> bool {
    let Some(error) = error else {
        return false;
    };
    let lower = error.to_ascii_lowercase();
    lower.contains("watcher notification")
        || lower.contains("notification summary")
        || lower.contains("follow-up summary")
}

fn list_watchers_history_row(
    state: crate::core::automation::AutomationSupervisorState,
) -> serde_json::Value {
    let created_at = state
        .created_at
        .clone()
        .or_else(|| state.last_run_at.clone())
        .or_else(|| state.last_success_at.clone());
    let notification_summary_failure = state.status == "failed"
        && list_watchers_history_error_is_notification_summary_failure(state.last_error.as_deref());
    let status = if notification_summary_failure {
        "triggered".to_string()
    } else {
        state.status.clone()
    };
    let status_error = if notification_summary_failure {
        None
    } else {
        state.last_error.clone()
    };
    let last_poll_outcome = match status.as_str() {
        "triggered" => Some("matched"),
        "failed" | "timed_out" => Some("error"),
        _ => None,
    };
    serde_json::json!({
        "id": state.automation_id,
        "description": state.title,
        "poll_action": state.action,
        "poll_arguments": serde_json::Value::Null,
        "condition": serde_json::Value::Null,
        "status": status,
        "status_error": status_error,
        "interval_secs": serde_json::Value::Null,
        "timeout_secs": serde_json::Value::Null,
        "poll_count": state.attempt_count,
        "created_at": created_at,
        "last_poll_at": state.last_run_at,
        "next_poll_not_before": state.next_retry_at,
        "notify_channel": serde_json::Value::Null,
        "on_trigger": serde_json::Value::Null,
        "trigger_result": serde_json::Value::Null,
        "last_result": serde_json::Value::Null,
        "last_error": status_error,
        "last_poll_outcome": last_poll_outcome,
        "notification_attempts": Vec::<serde_json::Value>::new(),
        "history_only": true,
    })
}

fn list_watchers_row_matches_filter(row: &serde_json::Value, filter: &str) -> bool {
    filter == "all"
        || row
            .get("status")
            .and_then(|value| value.as_str())
            .is_some_and(|status| status == filter)
}

fn resolve_gepa_candidates_path(
    project_root: &std::path::Path,
    run_id: Option<&str>,
    candidates_path: Option<&str>,
) -> Result<std::path::PathBuf> {
    if let Some(path) = candidates_path
        .map(str::trim)
        .filter(|value| !value.is_empty())
    {
        return resolve_gepa_workspace_path(project_root, path);
    }
    if let Some(run_id) = run_id.map(str::trim).filter(|value| !value.is_empty()) {
        return Ok(
            crate::core::self_evolve::gepa_bridge::default_candidates_path(project_root, run_id),
        );
    }
    anyhow::bail!("GEPA import requires candidates_path or gepa_run_id");
}

fn resolve_gepa_workspace_path(
    project_root: &std::path::Path,
    raw_path: &str,
) -> Result<std::path::PathBuf> {
    let root = project_root
        .canonicalize()
        .unwrap_or_else(|_| project_root.to_path_buf());
    let candidate = {
        let path = std::path::PathBuf::from(raw_path.trim());
        if path.is_absolute() {
            path
        } else {
            root.join(path)
        }
    };
    let mut normalized = std::path::PathBuf::new();
    for component in candidate.components() {
        match component {
            std::path::Component::Prefix(prefix) => normalized.push(prefix.as_os_str()),
            std::path::Component::RootDir => normalized.push(component.as_os_str()),
            std::path::Component::CurDir => {}
            std::path::Component::ParentDir => {
                normalized.pop();
            }
            std::path::Component::Normal(value) => normalized.push(value),
        }
    }
    if !normalized.starts_with(&root) {
        anyhow::bail!("GEPA artifact paths must stay inside the AgentArk workspace");
    }
    Ok(normalized)
}

#[derive(Debug, Clone, PartialEq, Eq)]
struct UserSafeToolFailure {
    message: String,
    failure_class: crate::core::FailureClass,
    retryable: bool,
    operational_outcome: &'static str,
}

#[derive(Debug)]
struct TypedToolErrorFields {
    code: String,
    scope: Option<String>,
    detail: Option<String>,
}

#[derive(Debug, Clone, serde::Serialize)]
pub(crate) struct GepaIdleCheck {
    pub(crate) idle: bool,
    pub(crate) quiet_window_seconds: i64,
    pub(crate) reasons: Vec<String>,
}

fn gepa_job_value_failed(value: &serde_json::Value) -> bool {
    value
        .get("status")
        .and_then(|status| status.as_str())
        .map(|status| matches!(status, "failed" | "timed_out" | "error"))
        .unwrap_or(false)
}

fn gepa_effective_status(value: &serde_json::Value) -> String {
    value
        .get("result")
        .and_then(|result| result.get("status"))
        .or_else(|| value.get("status"))
        .and_then(|status| status.as_str())
        .unwrap_or("completed")
        .to_string()
}

fn gepa_effective_reason(value: &serde_json::Value) -> Option<String> {
    let inner = value.get("result");
    inner
        .and_then(|result| result.get("error"))
        .or_else(|| inner.and_then(|result| result.get("stderr_tail")))
        .or_else(|| inner.and_then(|result| result.get("message")))
        .or_else(|| value.get("error"))
        .or_else(|| value.get("stderr_tail"))
        .or_else(|| value.get("message"))
        .and_then(|value| value.as_str())
        .map(|value| value.to_string())
}

fn typed_tool_error_fields(error: &anyhow::Error) -> Option<TypedToolErrorFields> {
    if let Some(error) = error.downcast_ref::<crate::actions::ActionError>() {
        return Some(TypedToolErrorFields {
            code: error.code(),
            scope: Some(error.err_prefix()),
            detail: Some(error.message().to_string()),
        });
    }

    if let Some(error) = error.downcast_ref::<crate::channels::ChannelError>() {
        return Some(TypedToolErrorFields {
            code: error.code().to_string(),
            scope: Some(error.channel().to_string()),
            detail: Some(error.message().to_string()),
        });
    }

    error
        .downcast_ref::<crate::security::SecurityError>()
        .map(|error| TypedToolErrorFields {
            code: error.code(),
            scope: None,
            detail: None,
        })
}

#[derive(Debug, Clone)]
struct ToolArgumentValidationFailure {
    message: String,
    missing_fields: Vec<String>,
    partial_inference: serde_json::Map<String, serde_json::Value>,
}

fn present_user_safe_tool_failure(error: &anyhow::Error) -> Option<UserSafeToolFailure> {
    match error.downcast_ref::<crate::runtime::ToolPathAccessError>() {
        Some(crate::runtime::ToolPathAccessError::OutsideAllowedRoots { .. }) => {
            Some(UserSafeToolFailure {
                message: "The requested file path is not available in this runtime. It can only access files inside the workspace and configured data directories.".to_string(),
                failure_class: crate::core::FailureClass::Validation,
                retryable: true,
                operational_outcome: "validation_error",
            })
        }
        None => None,
    }
}

fn code_execute_has_input_files(arguments: &serde_json::Value) -> bool {
    arguments
        .get("files")
        .and_then(|value| value.as_array())
        .is_some_and(|files| !files.is_empty())
        || arguments
            .get("file_payloads")
            .and_then(|value| value.as_array())
            .is_some_and(|files| !files.is_empty())
}

fn code_execute_uses_data_path_without_inputs(arguments: &serde_json::Value, code: &str) -> bool {
    !code_execute_has_input_files(arguments)
        && (code.contains("/data/") || code.contains(r#""/data""#) || code.contains(r#"'/data'"#))
}

fn summarize_file_write_stream_payload(arguments: &serde_json::Value) -> serde_json::Value {
    let path = arguments
        .get("path")
        .and_then(|value| value.as_str())
        .unwrap_or("")
        .trim()
        .to_string();
    let content = arguments
        .get("content")
        .and_then(|value| value.as_str())
        .unwrap_or("")
        .to_string();
    serde_json::json!({
        "kind": "file_write",
        "path": path,
        "content": content,
        "file_bytes": content.len(),
        "line_count": content.lines().count()
    })
}

pub(crate) fn tool_start_context_id_key(call: &crate::core::llm::ToolCall) -> Option<String> {
    let id = call.id.trim();
    if id.is_empty() {
        None
    } else {
        Some(format!("id:{}", id))
    }
}

pub(crate) fn tool_start_context_signature_key(call: &crate::core::llm::ToolCall) -> String {
    format!("sig:{}", Agent::tool_call_signature(call))
}

fn tool_start_context_for_call<'a>(
    call: &crate::core::llm::ToolCall,
    contexts: &'a HashMap<String, serde_json::Value>,
) -> Option<&'a serde_json::Value> {
    tool_start_context_id_key(call)
        .as_deref()
        .and_then(|key| contexts.get(key))
        .or_else(|| contexts.get(&tool_start_context_signature_key(call)))
}

fn merge_tool_start_payload(
    base: Option<serde_json::Value>,
    context: Option<&serde_json::Value>,
) -> Option<serde_json::Value> {
    let mut merged = match base {
        Some(serde_json::Value::Object(map)) => map,
        Some(value) => {
            let mut map = serde_json::Map::new();
            map.insert("payload".to_string(), value);
            map
        }
        None => serde_json::Map::new(),
    };

    if let Some(context_obj) = context.and_then(|value| value.as_object()) {
        for (key, value) in context_obj {
            merged.insert(key.clone(), value.clone());
        }
    }

    if merged.is_empty() {
        None
    } else {
        Some(serde_json::Value::Object(merged))
    }
}

const APP_DEPLOY_STREAM_FILES_TOTAL_MAX_BYTES: usize = 512 * 1024;
const APP_DEPLOY_STREAM_FILE_MAX_BYTES: usize = 256 * 1024;
const APP_REGISTRY_PREVIEW_TOTAL_MAX_BYTES: usize = 192 * 1024;
const APP_REGISTRY_PREVIEW_FILE_MAX_BYTES: usize = 96 * 1024;
const APP_QUALITY_SETTLE_SECS: u64 = 5;
const APP_QUALITY_COVERAGE_THRESHOLD: f32 = 0.64;

fn phase_status_payload(
    tool_name: &str,
    phase: &str,
    label: &str,
    detail: &str,
    elapsed_secs: u64,
) -> serde_json::Value {
    serde_json::json!({
        "kind": "phase_status",
        "phase": phase,
        "label": label,
        "detail": detail,
        "elapsed_secs": elapsed_secs,
        "stream_key": format!("phase-status:{}:{}", tool_name, phase),
    })
}

fn trace_json_data(value: serde_json::Value) -> Option<String> {
    if value.is_null() {
        None
    } else {
        serde_json::to_string_pretty(&value).ok()
    }
}

async fn push_trace_step(
    trace_ref: &Arc<RwLock<ExecutionTrace>>,
    icon: &str,
    title: impl Into<String>,
    detail: impl Into<String>,
    step_type: &str,
    data: Option<serde_json::Value>,
    duration_ms: Option<u64>,
) {
    trace_ref.write().await.steps.push(ExecutionStep {
        icon: icon.to_string(),
        title: title.into(),
        detail: detail.into(),
        step_type: step_type.to_string(),
        data: data.and_then(trace_json_data),
        timestamp: chrono::Utc::now(),
        duration_ms,
    });
}

fn json_changed_keys(previous_raw: Option<&[u8]>, next: &serde_json::Value) -> Vec<String> {
    let previous = previous_raw
        .and_then(|raw| serde_json::from_slice::<serde_json::Value>(raw).ok())
        .unwrap_or(serde_json::Value::Null);

    let (Some(previous_obj), Some(next_obj)) = (previous.as_object(), next.as_object()) else {
        if previous == *next {
            return Vec::new();
        }
        return vec!["policy".to_string()];
    };

    let mut changed = previous_obj
        .keys()
        .chain(next_obj.keys())
        .cloned()
        .collect::<HashSet<_>>()
        .into_iter()
        .filter(|key| previous_obj.get(key) != next_obj.get(key))
        .collect::<Vec<_>>();
    changed.sort();
    changed
}

#[derive(Debug, Clone)]
pub(crate) struct ToolCallOutput {
    pub name: String,
    pub content: String,
}

#[derive(Debug, Clone, Default)]
pub(crate) struct ToolExecutionBatch {
    pub outputs: Vec<ToolCallOutput>,
    pub outcomes: Vec<crate::core::ToolOutcome>,
}

#[derive(Debug, Clone, PartialEq, Eq)]
struct GmailScanRenderedMessage {
    from: String,
    subject: String,
    date: String,
    snippet: String,
}

fn gmail_sender_display_name(from: &str) -> String {
    let trimmed = from.trim();
    if let Some((name, _rest)) = trimmed.split_once('<') {
        let cleaned = name.trim().trim_matches('"').trim();
        if !cleaned.is_empty() {
            return cleaned.to_string();
        }
    }
    trimmed.to_string()
}

fn parse_gmail_scan_messages(raw: &str) -> Vec<GmailScanRenderedMessage> {
    raw.split("\n\n")
        .filter_map(|block| {
            let mut from = String::new();
            let mut subject = String::new();
            let mut date = String::new();
            let mut snippet = String::new();

            for line in block.lines() {
                let trimmed = line.trim();
                if let Some(value) = trimmed.strip_prefix("- From: ") {
                    from = value.trim().to_string();
                } else if let Some(value) = trimmed.strip_prefix("Subject: ") {
                    subject = value.trim().to_string();
                } else if let Some(value) = trimmed.strip_prefix("Date: ") {
                    date = value.trim().to_string();
                } else if let Some(value) = trimmed.strip_prefix("Snippet: ") {
                    snippet = value.trim().to_string();
                }
            }

            if from.is_empty() && subject.is_empty() && date.is_empty() && snippet.is_empty() {
                None
            } else {
                Some(GmailScanRenderedMessage {
                    from,
                    subject,
                    date,
                    snippet,
                })
            }
        })
        .collect()
}

fn format_gmail_scan_exact_results(
    mode: crate::actions::gmail::GmailScanMode,
    args: Option<&crate::actions::gmail::GmailScanArgs>,
    messages: &[GmailScanRenderedMessage],
) -> String {
    let count = messages.len();
    let heading = match mode {
        crate::actions::gmail::GmailScanMode::Recent => {
            format!(
                "Here are your latest {} email{}:",
                count,
                if count == 1 { "" } else { "s" }
            )
        }
        crate::actions::gmail::GmailScanMode::Search => {
            let query = args
                .and_then(|value| value.query.as_deref())
                .map(str::trim)
                .filter(|value| !value.is_empty());
            if let Some(query) = query {
                format!(
                    "Here are the matching emails for `{}`:",
                    query.replace('`', "'")
                )
            } else {
                format!(
                    "Here are the matching {} email{}:",
                    count,
                    if count == 1 { "" } else { "s" }
                )
            }
        }
        _ => format!(
            "Here are the {} email{}:",
            count,
            if count == 1 { "" } else { "s" }
        ),
    };

    let items = messages
        .iter()
        .enumerate()
        .map(|(index, message)| {
            let sender = gmail_sender_display_name(&message.from);
            let subject = if message.subject.trim().is_empty() {
                "(No subject)"
            } else {
                message.subject.trim()
            };
            let mut lines = vec![format!("{}. **{}** - {}", index + 1, sender, subject)];
            if !message.date.trim().is_empty() {
                lines.push(format!("   Date: {}", message.date.trim()));
            }
            if !message.snippet.trim().is_empty() {
                lines.push(format!("   {}", message.snippet.trim()));
            }
            lines.join("\n")
        })
        .collect::<Vec<_>>()
        .join("\n");

    format!("{}\n\n{}", heading, items)
}

#[derive(Debug, Clone)]
struct AppSemanticFingerprint {
    title_tokens: std::collections::HashSet<String>,
    keyword_tokens: std::collections::HashSet<String>,
    file_tokens: std::collections::HashSet<String>,
    is_static: bool,
}

#[derive(Debug, Clone)]
struct AppDuplicateMatch {
    app: serde_json::Value,
    match_kind: &'static str,
    score: f32,
    reason: String,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum DuplicateAppResolution {
    ReuseExisting,
    ReplaceExisting,
    NeedsClarification,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum RaisedCredentialPromptKind {
    RawSecret,
    IntegrationAuth,
}

impl RaisedCredentialPromptKind {
    fn as_str(self) -> &'static str {
        match self {
            Self::RawSecret => "raw_secret",
            Self::IntegrationAuth => "integration_auth",
        }
    }
}

impl Agent {
    /// Validate a tool call's arguments against the action schema, repairing
    /// missing required fields semantically when possible.
    ///
    /// Two repair stages run in order:
    ///   1. Action-specific static repair (today: `code_execute` language
    ///      detection from code syntax). Stays inline here per the project
    ///      rule against action-name conditionals living in
    ///      `argument_repair.rs`.
    ///   2. Generic LLM-driven inference of any required field still missing,
    ///      conditioned on the user's intent (user message + routing summary
    ///      + turn-plan goals) supplied via `repair_context`. Memoized per
    ///      `(action, missing-set, payload)` for the turn so identical
    ///      attempts do not re-call the model.
    ///
    /// Returns `None` when the call is valid; otherwise an error string the
    /// agent loop will feed back to the LLM. When LLM repair partially
    /// succeeded, the error string includes a JSON `Inference context:` tail
    /// so the next-turn LLM has actionable context, not just "missing X".
    async fn repair_and_validate_tool_call_arguments(
        &self,
        call: &mut crate::core::llm::ToolCall,
        action: Option<&crate::actions::ActionDef>,
        repair_context: &super::argument_repair::ArgumentRepairContext,
        repair_memo: &mut super::argument_repair::RepairMemo,
        repair_iteration: usize,
    ) -> Option<ToolArgumentValidationFailure> {
        let mut payload = call.arguments.as_object().cloned().unwrap_or_default();

        // Stage 1: action-specific static repair colocated with its helper.
        // Generic logic lives in argument_repair; per-action conditionals do
        // not.
        if call.name == "code_execute"
            && !super::argument_repair::required_argument_present(payload.get("language"))
        {
            if let Some(language) = payload
                .get("code")
                .and_then(|value| value.as_str())
                .and_then(Self::infer_code_execute_language_from_code)
            {
                payload.insert(
                    "language".to_string(),
                    serde_json::Value::String(language.to_string()),
                );
            }
        }

        let Some(action) = action else {
            call.arguments = serde_json::Value::Object(payload);
            return None;
        };

        let initial_missing = super::argument_repair::missing_required_fields(action, &payload);

        // Stage 2: generic, intent-driven LLM inference for any missing
        // required field. Memoized for the turn.
        let mut partial_inference: serde_json::Map<String, serde_json::Value> =
            serde_json::Map::new();
        let mut repaired_missing: Option<Vec<String>> = None;
        if !initial_missing.is_empty() {
            let memo_key =
                super::argument_repair::repair_memo_key(&action.name, &initial_missing, &payload);
            let budget_key = format!("{}::{}", action.name, repair_iteration);
            if repair_memo.lookup(&memo_key).is_some()
                || repair_memo.claim_attempt_scope(budget_key)
            {
                let outcome = self
                    .fill_missing_required_fields_via_inference(
                        action,
                        &mut payload,
                        repair_context,
                        &initial_missing,
                        repair_memo,
                    )
                    .await;
                partial_inference = outcome.partial_inference;
                payload = outcome.repaired_payload;
                repaired_missing = Some(outcome.still_missing);
            }
        }

        call.arguments = serde_json::Value::Object(payload.clone());

        let still_missing = repaired_missing
            .unwrap_or_else(|| super::argument_repair::missing_required_fields(action, &payload));

        if still_missing.is_empty() {
            if let Some(schema_error) =
                super::argument_repair::shallow_schema_violation(action, &call.arguments)
            {
                return Some(ToolArgumentValidationFailure {
                    message: format!(
                        "Tool '{}' could not run yet: {} Retry this tool with valid arguments, or use a better matching authorized action if one is available.",
                        call.name, schema_error
                    ),
                    missing_fields: Vec::new(),
                    partial_inference: serde_json::Map::new(),
                });
            }
            return None;
        }

        // Surface an enriched error so the next-turn LLM sees what was
        // partially inferred, not just "missing X". Keeps phrasing-agnostic:
        // the payload contains the structured field set, not free-text hints.
        let mut error_payload = serde_json::Map::new();
        error_payload.insert(
            "tool".to_string(),
            serde_json::Value::String(call.name.clone()),
        );
        error_payload.insert(
            "missing_fields".to_string(),
            serde_json::Value::Array(
                still_missing
                    .iter()
                    .map(|name| serde_json::Value::String(name.clone()))
                    .collect(),
            ),
        );
        if !partial_inference.is_empty() {
            error_payload.insert(
                "partial_inference".to_string(),
                serde_json::Value::Object(partial_inference.clone()),
            );
        }
        let payload_text = serde_json::Value::Object(error_payload).to_string();

        Some(ToolArgumentValidationFailure {
            message: format!(
                "Tool '{}' could not run yet: missing required field(s): {}. Retry this tool with the required fields, or use a better matching authorized action if one is available. Inference context: {}",
                call.name,
                still_missing.join(", "),
                payload_text
            ),
            missing_fields: still_missing,
            partial_inference,
        })
    }

    fn canonicalize_json_value(value: &serde_json::Value) -> serde_json::Value {
        match value {
            serde_json::Value::Object(map) => {
                let mut keys: Vec<&String> = map.keys().collect();
                keys.sort_unstable();
                let mut ordered = serde_json::Map::new();
                for key in keys {
                    if let Some(inner) = map.get(key) {
                        ordered.insert(key.clone(), Self::canonicalize_json_value(inner));
                    }
                }
                serde_json::Value::Object(ordered)
            }
            serde_json::Value::Array(items) => serde_json::Value::Array(
                items
                    .iter()
                    .map(Self::canonicalize_json_value)
                    .collect::<Vec<_>>(),
            ),
            _ => value.clone(),
        }
    }

    async fn raise_missing_secret_chat_prompt(
        &self,
        missing: &crate::runtime::MissingSecretPlaceholder,
        conversation_id: Option<&str>,
        tool_name: &str,
        trace_id: Option<&str>,
        stream_tx: Option<&tokio::sync::mpsc::Sender<StreamEvent>>,
    ) -> Option<(String, String)> {
        let conversation_id = conversation_id
            .map(str::trim)
            .filter(|value| !value.is_empty())?;
        let storage_key = missing.prompt_storage_key();
        if storage_key.trim().is_empty() {
            return None;
        }

        let manifests = self.integration_auth_manifests().await;
        let mut prompt_kind = RaisedCredentialPromptKind::RawSecret;
        let mut prompt_label = storage_key.clone();
        match crate::core::integration_auth::resolve_secret_key_among_manifests(
            &storage_key,
            &manifests,
        ) {
            crate::core::integration_auth::ReverseLookupOutcome::Unique(manifest) => {
                let has_form_fields = match &manifest.mode {
                    crate::core::integration_auth::AuthMode::Secrets { fields }
                    | crate::core::integration_auth::AuthMode::Hybrid { fields, .. } => {
                        !fields.is_empty()
                    }
                    crate::core::integration_auth::AuthMode::OAuth2AuthorizationCode(_)
                    | crate::core::integration_auth::AuthMode::OAuth2DeviceCode(_) => false,
                };
                if has_form_fields {
                    self.remember_integration_auth_chat_prompt(
                        conversation_id,
                        &manifest.integration_id,
                        Some(tool_name),
                        trace_id,
                    )
                    .await;
                    prompt_kind = RaisedCredentialPromptKind::IntegrationAuth;
                    prompt_label = manifest.display_name;
                } else {
                    self.remember_raw_secret_chat_prompt(
                        conversation_id,
                        &storage_key,
                        Some(tool_name),
                        trace_id,
                    )
                    .await;
                }
            }
            crate::core::integration_auth::ReverseLookupOutcome::Ambiguous { key, candidates } => {
                tracing::warn!(
                    "Ambiguous auth manifest reverse lookup for key '{}': {:?}",
                    key,
                    candidates
                );
                self.remember_raw_secret_chat_prompt(
                    conversation_id,
                    &storage_key,
                    Some(tool_name),
                    trace_id,
                )
                .await;
            }
            crate::core::integration_auth::ReverseLookupOutcome::None => {
                self.remember_raw_secret_chat_prompt(
                    conversation_id,
                    &storage_key,
                    Some(tool_name),
                    trace_id,
                )
                .await;
            }
        }

        let user_content = if matches!(prompt_kind, RaisedCredentialPromptKind::IntegrationAuth) {
            format!(
                "I need credentials for {} before `{}` can continue. Use the secure form that appeared in this chat; the value is stored encrypted and is not sent to the assistant.",
                prompt_label, tool_name
            )
        } else {
            format!(
                "I need one credential before `{}` can continue. Use the secure form that appeared in this chat; the value is stored encrypted and is not sent to the assistant.",
                tool_name
            )
        };
        if let Some(tx) = stream_tx {
            queue_stream_event(
                tx,
                StreamEvent::ToolResult {
                    name: tool_name.to_string(),
                    content: user_content.clone(),
                },
            );
        }
        let outcome_content = serde_json::json!({
            "status": "needs_credentials",
            "prompt_kind": prompt_kind.as_str(),
            "secret_key": storage_key,
            "tool_name": tool_name,
        })
        .to_string();
        Some((user_content, outcome_content))
    }

    pub(crate) fn tool_call_signature(call: &crate::core::llm::ToolCall) -> String {
        let normalized_name = call.name.trim().to_ascii_lowercase();
        if normalized_name == "watch" {
            let target = call
                .arguments
                .get("watcher_id")
                .and_then(|value| value.as_str())
                .map(str::trim)
                .filter(|value| !value.is_empty())
                .map(|value| format!("target:{}|", value))
                .unwrap_or_default();
            return format!(
                "watch:{}{}",
                target,
                crate::core::watcher::watcher_tool_call_signature_from_arguments(&call.arguments)
            );
        }
        if normalized_name == "schedule_task" {
            let target = call
                .arguments
                .get("task_id")
                .and_then(|value| value.as_str())
                .map(str::trim)
                .filter(|value| !value.is_empty())
                .map(|value| format!("target:{}|", value))
                .unwrap_or_default();
            let description = call
                .arguments
                .get("task")
                .and_then(|value| value.as_str())
                .unwrap_or("scheduled task");
            let action_name = call
                .arguments
                .get("action")
                .and_then(|value| value.as_str())
                .unwrap_or("");
            let action_arguments = call
                .arguments
                .get("action_arguments")
                .cloned()
                .unwrap_or_else(|| serde_json::json!({}));
            let cron_expr = call.arguments.get("cron").and_then(|value| value.as_str());
            let at_time = call.arguments.get("at").and_then(|value| value.as_str());
            return format!(
                "schedule_task:{}{}",
                target,
                crate::core::task::task_request_signature_from_fields(
                    action_name,
                    description,
                    &action_arguments,
                    cron_expr,
                    at_time
                )
            );
        }
        let canonical_args = Self::canonicalize_json_value(&call.arguments);
        let args = serde_json::to_string(&canonical_args).unwrap_or_else(|_| "{}".to_string());
        format!("{}:{}", normalized_name, args)
    }

    pub(crate) async fn record_self_tune_autonomous_success(&self) {
        crate::core::self_tune::record_autonomous_success(&self.storage).await;
    }

    pub(crate) async fn record_self_tune_user_rejection(&self) {
        crate::core::self_tune::record_user_rejection(&self.storage).await;
    }

    async fn handle_delegate_tool_call(
        &self,
        arguments: &serde_json::Value,
        request_channel: &str,
        trace_ref: &Arc<RwLock<ExecutionTrace>>,
        stream_tx: Option<&tokio::sync::mpsc::Sender<StreamEvent>>,
    ) -> String {
        let Some(task) = arguments
            .get("task")
            .and_then(|value| value.as_str())
            .map(str::trim)
            .filter(|value| !value.is_empty())
        else {
            return serde_json::json!({
                "ok": false,
                "error": "Delegation requires a non-empty task."
            })
            .to_string();
        };

        let context = arguments
            .get("context")
            .and_then(|value| value.as_str())
            .map(str::trim)
            .filter(|value| !value.is_empty())
            .unwrap_or("");
        let final_output = arguments
            .get("final_output")
            .and_then(|value| value.as_str())
            .map(str::trim)
            .filter(|value| !value.is_empty());
        let mut delegated_message = task.to_string();
        if !context.is_empty() {
            delegated_message.push_str("\n\nContext:\n");
            delegated_message.push_str(context);
        }
        if let Some(shape) = final_output {
            delegated_message.push_str("\n\nDesired final output: ");
            delegated_message.push_str(shape);
        }

        let mut actions = self
            .runtime
            .list_enabled_actions()
            .await
            .unwrap_or_default();
        self.append_dynamic_integration_actions(&mut actions).await;

        let active_prompt_bundle = self
            .active_prompt_bundle_for_message(&delegated_message)
            .await;
        let active_specialist_prompt_bundle = self
            .active_specialist_prompt_bundle_for_message(&delegated_message)
            .await;
        let mut decision = self
            .route_query(&delegated_message, &actions, &active_prompt_bundle)
            .await;
        decision.needs_delegation = true;
        if decision.sub_agents.len() < 2 {
            decision.sub_agents = self.forced_swarm_specs(&delegated_message, &actions);
        }
        decision.confidence = decision.confidence.max(0.96);
        decision.reasoning = if decision.reasoning.trim().is_empty() {
            "Explicit multi-agent capability execution.".to_string()
        } else {
            format!(
                "{} | Explicit multi-agent capability execution.",
                decision.reasoning.trim()
            )
        };

        let system_prompt = match self
            .build_system_prompt(&[], Some(&active_prompt_bundle))
            .await
        {
            Ok(prompt) => prompt,
            Err(error) => {
                return serde_json::json!({
                    "ok": false,
                    "error": format!("Failed to build delegation prompt: {error}")
                })
                .to_string();
            }
        };

        let delegation_id = uuid::Uuid::new_v4().to_string();
        let empty_memories: Vec<crate::core::PromptMemory> = Vec::new();
        let specialists = self
            .swarm
            .as_ref()
            .map(|manager| manager.specialists.clone());
        let action_scope_hints = self
            .runtime
            .list_action_scope_hints()
            .await
            .unwrap_or_default();
        let selected_model_slot_id = self
            .user_selected_model_slot_id
            .read()
            .ok()
            .and_then(|guard| guard.clone());

        match self
            .task_router
            .execute(
                &decision,
                crate::core::task_router::TaskRouterExecuteContext {
                    delegation_id: &delegation_id,
                    conversation_id: None,
                    channel: Some(request_channel),
                    message: &delegated_message,
                    system_prompt: &system_prompt,
                    prompt_bundle: &active_prompt_bundle,
                    specialist_prompt_bundle: &active_specialist_prompt_bundle,
                    configured_model_slots: &self.config.model_pool.slots,
                    model_pool: &self.model_pool,
                    primary_model_id: &self.primary_model_id,
                    user_selected_model_slot_id: selected_model_slot_id.as_deref(),
                    smart_routing: self.config.model_pool.smart_routing,
                    primary_llm: &self.llm,
                    specialists: &specialists,
                    memories: &empty_memories,
                    actions: &actions,
                    action_scope_hints: &action_scope_hints,
                    trace: trace_ref,
                    token_tx: stream_tx,
                    swarm_activity: Some(&self.swarm_activity),
                    storage: Some(&self.storage),
                },
            )
            .await
        {
            Ok(crate::core::task_router::TaskRouterResult::Delegated(result)) => {
                let agents = result
                    .agent_results
                    .iter()
                    .map(|item| {
                        item.agent_name
                            .clone()
                            .unwrap_or_else(|| item.agent_type.clone())
                    })
                    .collect::<Vec<_>>();
                let degradation = result
                    .degradation
                    .iter()
                    .map(|note| {
                        serde_json::json!({
                            "kind": &note.kind,
                            "summary": &note.summary,
                            "detail": &note.detail,
                        })
                    })
                    .collect::<Vec<_>>();
                serde_json::json!({
                    "ok": true,
                    "status": "completed",
                    "kind": "delegate",
                    "delegation_id": delegation_id,
                    "delegation_status": result.delegation_status.as_str(),
                    "agents_used": agents,
                    "degradation": degradation,
                    "final_result": crate::security::redact_pii(&result.final_response.content),
                })
                .to_string()
            }
            Ok(crate::core::task_router::TaskRouterResult::Direct) => serde_json::json!({
                "ok": false,
                "error": "Delegation resolved to a direct path without delegated work."
            })
            .to_string(),
            Err(error) => serde_json::json!({
                "ok": false,
                "error": format!("Delegation failed: {error}")
            })
            .to_string(),
        }
    }

    fn find_json_object_bounds(raw: &str) -> Option<(usize, usize)> {
        let mut depth = 0i32;
        let mut start: Option<usize> = None;
        let mut in_string = false;
        let mut escaped = false;

        for (idx, ch) in raw.char_indices() {
            if in_string {
                if escaped {
                    escaped = false;
                    continue;
                }
                if ch == '\\' {
                    escaped = true;
                    continue;
                }
                if ch == '"' {
                    in_string = false;
                }
                continue;
            }

            match ch {
                '"' => in_string = true,
                '{' => {
                    if depth == 0 {
                        start = Some(idx);
                    }
                    depth += 1;
                }
                '}' => {
                    if depth > 0 {
                        depth -= 1;
                        if depth == 0 {
                            if let Some(s) = start {
                                return Some((s, idx + ch.len_utf8()));
                            }
                        }
                    }
                }
                _ => {}
            }
        }
        None
    }

    fn parse_json_object_str(raw: &str) -> Option<serde_json::Value> {
        let mut candidate = raw.trim().to_string();
        if candidate.is_empty() {
            return None;
        }

        for _ in 0..5 {
            let trimmed = candidate.trim();
            if trimmed.is_empty() {
                return None;
            }

            if let Ok(parsed) = serde_json::from_str::<serde_json::Value>(trimmed) {
                match parsed {
                    serde_json::Value::Object(_) => return Some(parsed),
                    serde_json::Value::String(s) => {
                        candidate = s;
                        continue;
                    }
                    _ => {}
                }
            }

            if let Some((start, end)) = Self::find_json_object_bounds(trimmed) {
                if let Ok(parsed) = serde_json::from_str::<serde_json::Value>(&trimmed[start..end])
                {
                    if parsed.is_object() {
                        return Some(parsed);
                    }
                }
            }

            if trimmed.starts_with('"') && trimmed.ends_with('"') {
                if let Ok(unwrapped) = serde_json::from_str::<String>(trimmed) {
                    candidate = unwrapped;
                    continue;
                }
            }

            if trimmed.contains("\\\"") {
                let rebuilt = trimmed.replace("\\\"", "\"");
                if rebuilt != trimmed {
                    candidate = rebuilt;
                    continue;
                }
            }

            break;
        }

        None
    }

    fn extract_files_object(
        value: &serde_json::Value,
    ) -> Option<serde_json::Map<String, serde_json::Value>> {
        if let Some(obj) = value.as_object() {
            return Some(obj.clone());
        }

        if let Some(raw) = value.as_str() {
            if let Some(parsed) = Self::parse_json_object_str(raw) {
                return parsed.as_object().cloned();
            }
        }

        let rows = value.as_array()?;
        let mut out = serde_json::Map::new();
        for row in rows {
            let Some(item) = row.as_object() else {
                continue;
            };
            let name = item
                .get("name")
                .and_then(|v| v.as_str())
                .or_else(|| item.get("filename").and_then(|v| v.as_str()))
                .or_else(|| item.get("path").and_then(|v| v.as_str()))
                .map(|v| v.trim())
                .unwrap_or("");
            if name.is_empty() {
                continue;
            }
            let content = item
                .get("content")
                .and_then(|v| v.as_str())
                .or_else(|| item.get("text").and_then(|v| v.as_str()))
                .or_else(|| item.get("body").and_then(|v| v.as_str()))
                .unwrap_or("");
            out.insert(
                name.to_string(),
                serde_json::Value::String(content.to_string()),
            );
        }
        if out.is_empty() {
            None
        } else {
            Some(out)
        }
    }

    fn safe_app_relative_file_key(key: &str) -> Option<String> {
        let normalized = key.trim().replace('\\', "/");
        if normalized.is_empty() || normalized.starts_with('/') || normalized.contains('\0') {
            return None;
        }
        let mut parts = Vec::new();
        for part in normalized.split('/') {
            if part.is_empty() || part == "." {
                continue;
            }
            if part == ".." {
                return None;
            }
            parts.push(part);
        }
        if parts.is_empty() {
            None
        } else {
            Some(parts.join("/"))
        }
    }

    fn app_bundle_file_key_from_flat_entry(key: &str, content: &str) -> Option<String> {
        let trimmed_content = content.trim();
        if trimmed_content.is_empty() {
            return None;
        }

        let normalized_key = Self::safe_app_relative_file_key(key)?;
        if Self::looks_like_filename_like_key(&normalized_key) {
            return Some(normalized_key);
        }
        None
    }

    fn merge_recoverable_flat_files_into_files(
        files: &mut serde_json::Map<String, serde_json::Value>,
        obj: &serde_json::Map<String, serde_json::Value>,
    ) {
        for (key, value) in obj {
            if Self::KNOWN_METADATA_KEYS.contains(&key.as_str()) {
                continue;
            }
            if matches!(
                key.as_str(),
                "files"
                    | "file_map"
                    | "source_files"
                    | "project_files"
                    | "artifacts"
                    | "payload"
                    | "arguments"
                    | "args"
                    | "input"
                    | "params"
                    | "tool_input"
                    | "tool_arguments"
            ) {
                continue;
            }
            let Some(content) = value.as_str() else {
                continue;
            };
            let Some(filename) = Self::app_bundle_file_key_from_flat_entry(key, content) else {
                continue;
            };
            files
                .entry(filename)
                .or_insert_with(|| serde_json::Value::String(content.to_string()));
        }
    }

    pub(crate) fn normalize_app_deploy_arguments(
        arguments: &serde_json::Value,
    ) -> serde_json::Value {
        let mut nested = if let Some(obj) = arguments.as_object() {
            if let Some(files_value) = obj.get("files") {
                if let Some(mut files_obj) = Self::extract_files_object(files_value) {
                    Self::merge_recoverable_flat_files_into_files(&mut files_obj, obj);
                    let mut normalized = obj.clone();
                    normalized.insert("files".to_string(), serde_json::Value::Object(files_obj));
                    return serde_json::Value::Object(normalized);
                }
            }

            let mut found: Option<serde_json::Value> = None;
            for key in [
                "payload",
                "arguments",
                "args",
                "input",
                "params",
                "tool_input",
                "tool_arguments",
            ] {
                if let Some(candidate) = obj.get(key) {
                    if candidate.is_object() {
                        found = Some(candidate.clone());
                        break;
                    }
                    if let Some(s) = candidate.as_str() {
                        if let Some(parsed) = Self::parse_json_object_str(s) {
                            found = Some(parsed);
                            break;
                        }
                    }
                }
            }
            found
        } else if let Some(s) = arguments.as_str() {
            Self::parse_json_object_str(s)
        } else {
            None
        };

        let Some(mut normalized) = nested.take() else {
            // Last resort: try to recover files from the top-level object itself.
            if let Some(obj) = arguments.as_object() {
                if let Some(recovered) = Self::recover_files_from_flat_args(obj) {
                    return recovered;
                }
            }
            return arguments.clone();
        };

        if let Some(nested_obj) = normalized.as_object_mut() {
            if let Some(files_value) = nested_obj.get("files").cloned() {
                if let Some(files_obj) = Self::extract_files_object(&files_value) {
                    let mut files_obj = files_obj;
                    Self::merge_recoverable_flat_files_into_files(&mut files_obj, nested_obj);
                    nested_obj.insert("files".to_string(), serde_json::Value::Object(files_obj));
                }
            } else {
                // Try known aliases for the files field.
                let mut recovered = false;
                for alias in [
                    "file_map",
                    "source_files",
                    "project_files",
                    "artifacts",
                    "code",
                    "sources",
                    "data",
                ] {
                    if let Some(alias_value) = nested_obj.get(alias).cloned() {
                        if let Some(files_obj) = Self::extract_files_object(&alias_value) {
                            nested_obj
                                .insert("files".to_string(), serde_json::Value::Object(files_obj));
                            recovered = true;
                            break;
                        }
                    }
                }
                // If still no files, try single-content keys that models commonly use.
                if !recovered {
                    if let Some(files_obj) = Self::recover_files_from_single_content_key(nested_obj)
                    {
                        nested_obj
                            .insert("files".to_string(), serde_json::Value::Object(files_obj));
                    } else if let Some(files_obj) =
                        Self::recover_files_from_flat_string_values(nested_obj)
                    {
                        nested_obj
                            .insert("files".to_string(), serde_json::Value::Object(files_obj));
                    }
                }
            }
        }

        if let (Some(root), Some(nested_obj)) = (arguments.as_object(), normalized.as_object_mut())
        {
            for &key in Self::KNOWN_METADATA_KEYS {
                if nested_obj.get(key).is_none() {
                    if let Some(v) = root.get(key) {
                        nested_obj.insert(key.to_string(), v.clone());
                    }
                }
            }
            if let Some(files) = nested_obj
                .get_mut("files")
                .and_then(|value| value.as_object_mut())
            {
                Self::merge_recoverable_flat_files_into_files(files, root);
            }
        }

        normalized
    }

    /// Known app_deploy metadata keys: these are NOT file content.
    const KNOWN_METADATA_KEYS: &'static [&'static str] = &[
        "app_id",
        "mode",
        "title",
        "repo_url",
        "repo_ref",
        "repo_subdir",
        "service_mode",
        "deploy_target",
        "external_deploy_target",
        "production",
        "vercel_project_mode",
        "vercel_project_id",
        "vercel_team_id",
        "build_command",
        "output_dir",
        "file_patches",
        "delete_paths",
        "source_dir",
        "source_paths",
        "entry_command",
        "install_command",
        "runtime_image",
        "runtime_preference",
        "runtime_required",
        "runtime_reason",
        "expose_public",
        "access_guard",
        "access_password",
        "access_key",
        "required_inputs",
        "required_secrets",
        "required_env",
        "required_config",
        "config",
        "replace_existing",
        "allow_duplicate",
        "conversation_id",
        "_conversation_id",
        "_streamed_app_delivery",
        "name",
    ];

    fn looks_like_filename_like_key(key: &str) -> bool {
        let key = key.trim();
        !key.is_empty()
            && (key.contains('.')
                || key.contains('/')
                || key.contains('\\')
                || key.eq_ignore_ascii_case("index"))
    }

    fn looks_like_app_file_content(content: &str) -> bool {
        let trimmed = content.trim();
        if trimmed.is_empty() {
            return false;
        }

        let lower = trimmed.to_ascii_lowercase();
        if lower.starts_with("<!doctype html")
            || lower.starts_with("<html")
            || lower.starts_with("<head")
            || lower.starts_with("<body")
            || lower.contains("</")
        {
            return true;
        }

        if lower.starts_with("import ")
            || lower.starts_with("from ")
            || lower.starts_with("export ")
            || lower.starts_with("const ")
            || lower.starts_with("let ")
            || lower.starts_with("var ")
            || lower.starts_with("function ")
            || lower.starts_with("class ")
            || lower.starts_with("def ")
            || lower.starts_with("package ")
        {
            return true;
        }

        if lower.contains("body {")
            || lower.contains(":root")
            || lower.contains("@media")
            || lower.contains(".container")
            || lower.contains("html {")
        {
            return true;
        }

        if trimmed.starts_with('{') || trimmed.starts_with('[') {
            return serde_json::from_str::<serde_json::Value>(trimmed).is_ok();
        }

        false
    }

    /// Recover files from a single content key like `content`, `code`, `html`, `source`.
    /// Only recover content that looks like actual source/markup, not generic prose.
    fn recover_files_from_single_content_key(
        obj: &serde_json::Map<String, serde_json::Value>,
    ) -> Option<serde_json::Map<String, serde_json::Value>> {
        for key in ["content", "code", "html", "source", "body", "text", "page"] {
            if let Some(value) = obj.get(key).and_then(|v| v.as_str()) {
                if value.len() > 20 && Self::looks_like_app_file_content(value) {
                    let filename = Self::infer_filename_from_content(value);
                    let mut files = serde_json::Map::new();
                    files.insert(filename, serde_json::Value::String(value.to_string()));
                    return Some(files);
                }
            }
        }
        None
    }

    /// Recover files when the model placed string values at the top level that look like
    /// file content (not known metadata keys). E.g. `{"index.html": "<html>...", "title": "App"}`.
    fn recover_files_from_flat_string_values(
        obj: &serde_json::Map<String, serde_json::Value>,
    ) -> Option<serde_json::Map<String, serde_json::Value>> {
        let mut files = serde_json::Map::new();
        for (key, value) in obj {
            if Self::KNOWN_METADATA_KEYS.contains(&key.as_str()) {
                continue;
            }
            // Skip known wrapper/alias keys already handled elsewhere.
            if matches!(
                key.as_str(),
                "files"
                    | "file_map"
                    | "source_files"
                    | "project_files"
                    | "artifacts"
                    | "payload"
                    | "arguments"
                    | "args"
                    | "input"
                    | "params"
                    | "tool_input"
                    | "tool_arguments"
            ) {
                continue;
            }
            if let Some(s) = value.as_str() {
                if let Some(filename) = Self::app_bundle_file_key_from_flat_entry(key, s) {
                    files.insert(filename, serde_json::Value::String(s.to_string()));
                }
            }
        }
        if files.is_empty() {
            None
        } else {
            Some(files)
        }
    }

    /// Recover files from a top-level object that has no `files` key and no nested wrapper.
    fn recover_files_from_flat_args(
        obj: &serde_json::Map<String, serde_json::Value>,
    ) -> Option<serde_json::Value> {
        // First try single-content recovery.
        if let Some(files_obj) = Self::recover_files_from_single_content_key(obj) {
            let mut result = obj.clone();
            result.insert("files".to_string(), serde_json::Value::Object(files_obj));
            return Some(serde_json::Value::Object(result));
        }
        // Then try flat string values.
        if let Some(files_obj) = Self::recover_files_from_flat_string_values(obj) {
            let mut result = serde_json::Map::new();
            result.insert("files".to_string(), serde_json::Value::Object(files_obj));
            // Carry over metadata keys.
            for key in Self::KNOWN_METADATA_KEYS {
                if let Some(v) = obj.get(*key) {
                    result.insert(key.to_string(), v.clone());
                }
            }
            return Some(serde_json::Value::Object(result));
        }
        None
    }

    /// Infer a reasonable filename from content when the model didn't provide one.
    fn infer_filename_from_content(content: &str) -> String {
        let lower = content.trim_start().to_ascii_lowercase();
        if lower.starts_with("<!doctype html")
            || lower.starts_with("<html")
            || lower.starts_with("<head")
            || lower.starts_with("<body")
        {
            "index.html".to_string()
        } else if lower.starts_with("import ") || lower.starts_with("from ") {
            if lower.contains("fastapi") || lower.contains("flask") || lower.contains("django") {
                "app.py".to_string()
            } else {
                "app.js".to_string()
            }
        } else if lower.starts_with("const ")
            || lower.starts_with("function ")
            || lower.starts_with("var ")
        {
            "app.js".to_string()
        } else if lower.starts_with("body")
            || lower.starts_with("*")
            || lower.starts_with(".")
            || lower.starts_with("#")
        {
            "style.css".to_string()
        } else {
            "index.html".to_string()
        }
    }

    async fn repair_app_deploy_arguments_after_validation_error(
        &self,
        current_args: &serde_json::Value,
        error_text: &str,
        repair_context: &super::argument_repair::ArgumentRepairContext,
        stream_tx: Option<&tokio::sync::mpsc::Sender<StreamEvent>>,
    ) -> Option<serde_json::Value> {
        let Some(current_obj) = current_args.as_object() else {
            return None;
        };

        if let Some(tx) = stream_tx {
            queue_stream_event(
                tx,
                StreamEvent::ToolProgress {
                    name: "app_deploy".to_string(),
                    content: "Repairing app bundle from validator diagnostics.".to_string(),
                    payload: Some(phase_status_payload(
                        "app_deploy",
                        "repairing",
                        "Repairing",
                        "Repairing app bundle from validator diagnostics.",
                        0,
                    )),
                },
            );
        }

        let prompt = serde_json::json!({
            "request_context": repair_context.build_request_text(),
            "validation_error": error_text,
            "current_app_deploy_arguments": current_args,
            "required_output": {
                "type": "complete app_deploy arguments JSON object",
                "rules": [
                    "Preserve the user's requested product, content, visual direction, interactions, and delivery intent.",
                    "If the current arguments are empty or omit a deployable source, synthesize the deployable source from the request context instead of asking the user to provide tool payload fields.",
                    "Do not use generic placeholder or fallback asset contents.",
                    "For generated app bundles, return a complete files object for the intended stack. Every local reference from HTML, CSS, scripts, package metadata, or runtime config must either have matching bundled file content or be removed/inlined consistently.",
                    "If markup is malformed, return a complete replacement document and all referenced assets.",
                    "Return JSON only. Do not include markdown or explanatory prose."
                ]
            }
        });

        let response = self
            .supervised_internal_chat(
                "automation",
                "app_deploy_bundle_repair",
                "app_deploy_bundle_repair",
                &ModelRole::Primary,
                self.llm_candidates_for_role(&ModelRole::Primary),
                "You repair app deployment arguments after a structural bundle validator failure. Return only strict JSON containing complete corrected app_deploy arguments. Base the repair on the validator error, current file graph, and user intent context.",
                &prompt.to_string(),
                &[],
                &[],
                120_000,
                2,
            )
            .await?;

        let parsed = extract_json_object_from_text(&response.content)?;
        let candidate = parsed
            .get("arguments")
            .and_then(|value| value.as_object())
            .map(|obj| serde_json::Value::Object(obj.clone()))
            .unwrap_or(parsed);
        let Some(candidate_obj) = candidate.as_object() else {
            return None;
        };

        let mut merged = current_obj.clone();
        for (key, value) in candidate_obj {
            merged.insert(key.clone(), value.clone());
        }
        let normalized = Self::normalize_app_deploy_arguments(&serde_json::Value::Object(merged));
        let normalized_obj = normalized.as_object()?;
        let has_repo = normalized_obj
            .get("repo_url")
            .and_then(|value| value.as_str())
            .map(str::trim)
            .is_some_and(|value| !value.is_empty());
        let has_valid_files = normalized_obj
            .get("files")
            .and_then(|value| value.as_object())
            .map(|files| !files.is_empty() && files.values().all(|value| value.is_string()))
            .unwrap_or(false);
        let has_valid_staged_source = normalized_obj
            .get("source_dir")
            .and_then(|value| value.as_str())
            .map(str::trim)
            .is_some_and(|value| !value.is_empty())
            && normalized_obj
                .get("source_paths")
                .and_then(|value| value.as_array())
                .map(|paths| {
                    !paths.is_empty()
                        && paths.iter().all(|path| {
                            path.as_str()
                                .map(str::trim)
                                .is_some_and(|value| !value.is_empty())
                        })
                })
                .unwrap_or(false);
        if !has_repo && !has_valid_files && !has_valid_staged_source {
            return None;
        }
        if serde_json::to_string(&normalized).ok()? == serde_json::to_string(current_args).ok()? {
            return None;
        }
        Some(normalized)
    }

    fn summarize_app_deploy_stream_payload(arguments: &serde_json::Value) -> serde_json::Value {
        let normalized = Self::normalize_app_deploy_arguments(arguments);
        let Some(obj) = normalized.as_object() else {
            return normalized;
        };

        let mut summary = serde_json::Map::new();
        for key in [
            "app_id",
            "mode",
            "title",
            "repo_url",
            "repo_ref",
            "repo_subdir",
            "service_mode",
            "delete_paths",
            "source_dir",
            "source_paths",
            "entry_command",
            "install_command",
            "runtime_image",
            "runtime_preference",
            "runtime_required",
            "runtime_reason",
            "expose_public",
            "access_guard",
        ] {
            if let Some(value) = obj.get(key) {
                summary.insert(key.to_string(), value.clone());
            }
        }

        if let Some(files) = obj.get("files").and_then(|v| v.as_object()) {
            let mut file_names: Vec<String> = files.keys().cloned().collect();
            file_names.sort_unstable();
            let total_file_count = file_names.len();
            let truncated = total_file_count > 120;
            if truncated {
                file_names.truncate(120);
            }
            let total_bytes: usize = files
                .values()
                .filter_map(|v| v.as_str())
                .map(|s| s.len())
                .sum();

            summary.insert(
                "file_count".to_string(),
                serde_json::json!(total_file_count),
            );
            summary.insert("file_names".to_string(), serde_json::json!(file_names));
            summary.insert("file_bytes".to_string(), serde_json::json!(total_bytes));
            let mut included_bytes = 0usize;
            let mut streamed_files = serde_json::Map::new();
            let mut omitted_contents = false;
            let mut content_names: Vec<&String> = files.keys().collect();
            content_names.sort_unstable();
            for name in content_names {
                let Some(content) = files.get(name).and_then(|value| value.as_str()) else {
                    continue;
                };
                let content_bytes = content.len();
                if content_bytes > APP_DEPLOY_STREAM_FILE_MAX_BYTES
                    || included_bytes.saturating_add(content_bytes)
                        > APP_DEPLOY_STREAM_FILES_TOTAL_MAX_BYTES
                {
                    omitted_contents = true;
                    continue;
                }
                included_bytes = included_bytes.saturating_add(content_bytes);
                streamed_files.insert(name.clone(), serde_json::json!(content));
            }
            if !streamed_files.is_empty() {
                summary.insert(
                    "files".to_string(),
                    serde_json::Value::Object(streamed_files),
                );
            }
            if omitted_contents {
                summary.insert("file_contents_omitted".to_string(), serde_json::json!(true));
                summary.insert(
                    "file_content_limit_bytes".to_string(),
                    serde_json::json!(APP_DEPLOY_STREAM_FILES_TOTAL_MAX_BYTES),
                );
            }
            if truncated {
                summary.insert("file_names_truncated".to_string(), serde_json::json!(true));
            }
        }

        if let Some(file_patches) = obj.get("file_patches").and_then(|v| v.as_array()) {
            let mut patch_paths = file_patches
                .iter()
                .filter_map(|entry| entry.get("path").and_then(|value| value.as_str()))
                .map(|value| value.to_string())
                .collect::<Vec<_>>();
            patch_paths.sort_unstable();
            summary.insert(
                "patch_count".to_string(),
                serde_json::json!(patch_paths.len()),
            );
            summary.insert("patch_paths".to_string(), serde_json::json!(patch_paths));
        }

        serde_json::Value::Object(summary)
    }

    fn extract_output_route_components(url: &str) -> Option<(String, String)> {
        let path = if url.starts_with("http://") || url.starts_with("https://") {
            match reqwest::Url::parse(url) {
                Ok(parsed) => parsed.path().to_string(),
                Err(_) => return None,
            }
        } else {
            url.to_string()
        };
        let marker = "/api/outputs/";
        let idx = path.find(marker)?;
        let tail = &path[idx + marker.len()..];
        let mut parts = tail.splitn(2, '/');
        let exec_id = parts.next()?.trim().to_string();
        let filename = parts.next()?.trim().to_string();
        if exec_id.is_empty() || filename.is_empty() {
            return None;
        }
        let filename = match urlencoding::decode(&filename) {
            Ok(v) => v.to_string(),
            Err(_) => filename,
        };
        Some((exec_id, filename))
    }

    async fn load_video_bytes(&self, source_url: &str, max_bytes: usize) -> Result<Vec<u8>> {
        if source_url.starts_with("data:") {
            if let Some(comma_idx) = source_url.find(',') {
                let (meta, payload) = source_url.split_at(comma_idx);
                let payload = &payload[1..];
                if meta.contains(";base64") {
                    use base64::Engine;
                    let bytes = base64::engine::general_purpose::STANDARD
                        .decode(payload.as_bytes())
                        .map_err(|e| anyhow::anyhow!("Failed to decode data URL video: {}", e))?;
                    if bytes.len() > max_bytes {
                        anyhow::bail!(
                            "Video too large for channel delivery: {} bytes (max {})",
                            bytes.len(),
                            max_bytes
                        );
                    }
                    return Ok(bytes);
                }
            }
            anyhow::bail!("Unsupported data URL video format");
        }

        if let Some((exec_id, filename)) = Self::extract_output_route_components(source_url) {
            if uuid::Uuid::parse_str(&exec_id).is_ok()
                && !filename.contains('/')
                && !filename.contains('\\')
                && !filename.contains("..")
            {
                let path = self.data_dir.join("outputs").join(exec_id).join(filename);
                let bytes = tokio::fs::read(&path).await?;
                if bytes.len() > max_bytes {
                    anyhow::bail!(
                        "Video too large for channel delivery: {} bytes (max {})",
                        bytes.len(),
                        max_bytes
                    );
                }
                return Ok(bytes);
            }
        }

        if source_url.starts_with("http://") || source_url.starts_with("https://") {
            let client = reqwest::Client::builder()
                .timeout(std::time::Duration::from_secs(90))
                .build()?;
            let resp = client.get(source_url).send().await?;
            if !resp.status().is_success() {
                anyhow::bail!("Failed to fetch video URL (status {})", resp.status());
            }
            if let Some(len) = resp.content_length() {
                if len > max_bytes as u64 {
                    anyhow::bail!(
                        "Video too large for channel delivery: {} bytes (max {})",
                        len,
                        max_bytes
                    );
                }
            }
            let bytes = resp.bytes().await?.to_vec();
            if bytes.len() > max_bytes {
                anyhow::bail!(
                    "Video too large for channel delivery: {} bytes (max {})",
                    bytes.len(),
                    max_bytes
                );
            }
            return Ok(bytes);
        }

        anyhow::bail!("Unsupported video URL format for delivery")
    }

    async fn build_app_runtime_failure_hint(&self, app_id: &str) -> Option<String> {
        if self.app_registry.is_static(app_id).await {
            return None;
        }
        let app_dir = self.app_registry.get_dir(app_id).await?;
        let current_port = self.app_registry.get_port(app_id).await;
        let log_tail = crate::actions::app::read_local_runtime_log_tail(&app_dir, 4096).await;

        if current_port.is_none() {
            if log_tail.is_empty() {
                return Some(
                    "Dynamic app runtime is not active (process/container likely exited)."
                        .to_string(),
                );
            }
            return Some(format!(
                "Dynamic app runtime is not active. Recent runtime logs:\n{}",
                log_tail
            ));
        }

        if log_tail.is_empty() {
            None
        } else {
            Some(format!("Recent runtime logs:\n{}", log_tail))
        }
    }

    fn detect_app_runtime_error_marker(content: &str) -> Option<&'static str> {
        let needles: [(&str, &str); 12] = [
            ("error loading", "error loading"),
            ("failed to load", "failed to load"),
            ("failed to fetch", "failed to fetch"),
            ("something went wrong", "something went wrong"),
            ("application error", "application error"),
            ("could not fetch", "could not fetch"),
            ("unable to fetch", "unable to fetch"),
            ("network error", "network error"),
            (
                "cross-origin request blocked",
                "cross-origin request blocked",
            ),
            ("runtime error", "runtime error"),
            ("uncaught exception", "uncaught exception"),
            ("exception:", "exception"),
        ];
        for (needle, label) in needles {
            if content.contains(needle) {
                return Some(label);
            }
        }
        None
    }

    fn detect_http_probe_runtime_error_marker(
        content_type: &str,
        body: &str,
    ) -> Option<&'static str> {
        let lower_content_type = content_type.to_ascii_lowercase();
        let lower_body = body.to_ascii_lowercase();
        let is_html_app_shell = lower_content_type.contains("html")
            || lower_body.contains("<!doctype html")
            || lower_body.contains("<html");
        if is_html_app_shell {
            return None;
        }
        Self::detect_app_runtime_error_marker(&lower_body)
    }

    fn resolve_duplicate_app(match_kind: &str, existing_running: bool) -> DuplicateAppResolution {
        if match_kind == "exact_files" && existing_running {
            DuplicateAppResolution::ReuseExisting
        } else if match_kind == "exact_files" {
            DuplicateAppResolution::ReplaceExisting
        } else {
            DuplicateAppResolution::NeedsClarification
        }
    }

    async fn stop_and_remove_existing_app(
        &self,
        app_id: &str,
        app_title: Option<&str>,
    ) -> Result<()> {
        if app_id.is_empty()
            || app_id.len() > 64
            || !app_id
                .chars()
                .all(|c| c.is_ascii_alphanumeric() || c == '-' || c == '_')
        {
            anyhow::bail!("refusing to remove invalid app id '{}'", app_id);
        }

        let app_dir = self
            .app_registry
            .get_dir(app_id)
            .await
            .unwrap_or_else(|| self.data_dir.join("apps").join(app_id));

        let mut executor_deleted_files = false;
        if let Some(executor) = build_executor_client() {
            let response = executor
                .request(
                    reqwest::Method::DELETE,
                    &format!("/internal/v1/apps/{}", app_id),
                )
                .send()
                .await?;
            if !response.status().is_success() {
                let payload = response
                    .json::<serde_json::Value>()
                    .await
                    .unwrap_or_else(|_| serde_json::json!({}));
                anyhow::bail!(
                    "{}",
                    payload
                        .get("message")
                        .and_then(|value| value.as_str())
                        .unwrap_or("executor refused app delete")
                );
            }
            executor_deleted_files = true;
            let _ = self.app_registry.stop(app_id).await;
        } else {
            self.app_registry.stop(app_id).await?;
        }

        match tokio::fs::remove_dir_all(&app_dir).await {
            Ok(_) => {}
            Err(error)
                if executor_deleted_files && error.kind() == std::io::ErrorKind::NotFound => {}
            Err(error) if error.kind() == std::io::ErrorKind::NotFound => {}
            Err(error) => {
                anyhow::bail!(
                    "failed to remove app directory '{}': {}",
                    app_dir.display(),
                    error
                );
            }
        }

        if let Err(error) = self
            .storage
            .delete_app_notifications(app_id, app_title)
            .await
        {
            tracing::warn!(
                "failed to delete app notifications during replacement for {}: {}",
                app_id,
                error
            );
        }

        Ok(())
    }

    fn normalize_app_title(value: &str) -> String {
        value
            .to_ascii_lowercase()
            .chars()
            .map(|c| {
                if c.is_ascii_alphanumeric() || c.is_ascii_whitespace() {
                    c
                } else {
                    ' '
                }
            })
            .collect::<String>()
            .split_whitespace()
            .collect::<Vec<_>>()
            .join(" ")
    }

    fn is_generic_title(title: &str) -> bool {
        let mut token_count = 0usize;
        let generic = [
            "app",
            "dashboard",
            "tool",
            "site",
            "website",
            "web",
            "page",
            "project",
            "demo",
        ];
        for token in title.split_whitespace() {
            token_count += 1;
            if !generic.iter().any(|g| g == &token) {
                return false;
            }
        }
        token_count > 0
    }

    fn is_probably_text_bytes(bytes: &[u8]) -> bool {
        if bytes.is_empty() {
            return false;
        }
        let sample_len = std::cmp::min(bytes.len(), 4096);
        let sample = &bytes[..sample_len];
        if sample.contains(&0) {
            return false;
        }
        let control_count = sample
            .iter()
            .filter(|b| {
                let c = **b;
                c < 0x20 && c != b'\n' && c != b'\r' && c != b'\t'
            })
            .count();
        (control_count as f32 / sample_len as f32) <= 0.12
    }

    fn extract_semantic_excerpt_from_bytes(bytes: &[u8], max_chars: usize) -> Option<String> {
        if max_chars == 0 || !Self::is_probably_text_bytes(bytes) {
            return None;
        }
        let excerpt = String::from_utf8_lossy(bytes)
            .chars()
            .take(max_chars)
            .collect::<String>();
        if excerpt.trim().is_empty() {
            None
        } else {
            Some(excerpt)
        }
    }

    fn app_token_stopword(token: &str) -> bool {
        matches!(
            token,
            "the"
                | "and"
                | "for"
                | "with"
                | "from"
                | "into"
                | "this"
                | "that"
                | "are"
                | "was"
                | "were"
                | "you"
                | "your"
                | "http"
                | "https"
                | "www"
                | "com"
                | "org"
                | "net"
                | "api"
                | "app"
                | "apps"
                | "dashboard"
                | "tool"
                | "page"
                | "static"
                | "dynamic"
        )
    }

    fn append_semantic_tokens(
        target: &mut std::collections::HashSet<String>,
        text: &str,
        max_tokens: usize,
    ) {
        if target.len() >= max_tokens {
            return;
        }
        for raw in text.split(|c: char| !c.is_ascii_alphanumeric()) {
            let token = raw.trim().to_ascii_lowercase();
            if token.len() < 3 || Self::app_token_stopword(&token) {
                continue;
            }
            target.insert(token);
            if target.len() >= max_tokens {
                break;
            }
        }
    }

    fn jaccard_similarity(
        left: &std::collections::HashSet<String>,
        right: &std::collections::HashSet<String>,
    ) -> f32 {
        if left.is_empty() || right.is_empty() {
            return 0.0;
        }
        let inter = left.intersection(right).count() as f32;
        let union = left.union(right).count() as f32;
        if union <= f32::EPSILON {
            0.0
        } else {
            inter / union
        }
    }

    fn compact_app_lookup_key(value: &str) -> String {
        value
            .to_ascii_lowercase()
            .chars()
            .filter(|c| c.is_ascii_alphanumeric())
            .collect()
    }

    fn should_skip_app_inventory_dir(name: &str) -> bool {
        matches!(
            name.trim().to_ascii_lowercase().as_str(),
            ".git"
                | ".hg"
                | ".svn"
                | "__pycache__"
                | ".mypy_cache"
                | ".pytest_cache"
                | ".ruff_cache"
                | ".turbo"
                | ".next"
                | ".venv"
                | "node_modules"
                | "_deps"
                | "target"
        )
    }

    fn should_skip_app_inventory_file(path: &str) -> bool {
        let normalized = path.trim().replace('\\', "/").to_ascii_lowercase();
        let name = normalized.rsplit('/').next().unwrap_or(normalized.as_str());
        name == ".agentark_runtime_env"
            || name == ".env"
            || name.starts_with(".env.")
            || name.ends_with(".pem")
            || name.ends_with(".key")
            || name.ends_with(".p12")
            || name.ends_with(".pfx")
            || name == "secrets.json"
            || name == "credentials.json"
    }

    fn preferred_app_file_rank(path: &str) -> usize {
        let lower = path.trim().replace('\\', "/").to_ascii_lowercase();
        match lower.as_str() {
            ".app_meta.json" => 0,
            "app.py" => 1,
            "main.py" => 2,
            "server.py" => 3,
            "server.js" | "server.ts" => 4,
            "index.html" => 5,
            "package.json" => 6,
            "requirements.txt" => 7,
            "pyproject.toml" => 8,
            "vite.config.ts" | "vite.config.js" => 9,
            "src/main.tsx" | "src/main.jsx" => 10,
            "src/app.tsx" | "src/app.jsx" => 11,
            "src/index.tsx" | "src/index.jsx" => 12,
            "readme.md" => 13,
            _ if lower.ends_with("/app.py") => 14,
            _ if lower.ends_with("/main.py") => 15,
            _ if lower.ends_with("/server.py") || lower.ends_with("/server.js") => 16,
            _ if lower.ends_with("/index.html") => 17,
            _ if lower.ends_with("/package.json") => 18,
            _ if lower.ends_with("/requirements.txt") => 19,
            _ if lower.ends_with(".html") => 30,
            _ if lower.ends_with(".py") => 31,
            _ if lower.ends_with(".ts") || lower.ends_with(".tsx") => 32,
            _ if lower.ends_with(".js") || lower.ends_with(".jsx") => 33,
            _ if lower.ends_with(".css") => 34,
            _ => 100,
        }
    }

    pub(super) fn score_deployed_app_match(
        query: &str,
        app_id: &str,
        title: &str,
    ) -> Option<(f32, String)> {
        let query = query.trim();
        if query.is_empty() {
            return None;
        }
        let normalized_query = Self::normalize_app_title(query);
        let normalized_title = Self::normalize_app_title(title);
        let compact_query = Self::compact_app_lookup_key(query);
        let compact_title = Self::compact_app_lookup_key(title);
        let compact_id = Self::compact_app_lookup_key(app_id);

        if !compact_query.is_empty() && compact_query == compact_id {
            return Some((1.0, "exact_id".to_string()));
        }
        if !normalized_query.is_empty() && normalized_query == normalized_title {
            return Some((0.99, "exact_title".to_string()));
        }
        if !compact_query.is_empty() && compact_id.contains(&compact_query) {
            return Some((0.94, "id_substring".to_string()));
        }
        if !compact_query.is_empty() && compact_title.contains(&compact_query) {
            return Some((0.92, "title_substring".to_string()));
        }

        let mut query_tokens = std::collections::HashSet::new();
        Self::append_semantic_tokens(&mut query_tokens, &normalized_query, 18);
        let mut app_tokens = std::collections::HashSet::new();
        Self::append_semantic_tokens(&mut app_tokens, &normalized_title, 24);
        Self::append_semantic_tokens(&mut app_tokens, app_id, 28);
        if query_tokens.is_empty() || app_tokens.is_empty() {
            return None;
        }
        let overlap = Self::jaccard_similarity(&query_tokens, &app_tokens);
        if overlap < 0.20 {
            None
        } else {
            Some((0.35 + (overlap * 0.55), "token_overlap".to_string()))
        }
    }

    pub(super) fn rank_deployed_apps(
        query: &str,
        apps: &[serde_json::Value],
    ) -> Vec<(f32, String, serde_json::Value, String)> {
        let mut ranked_apps: Vec<(f32, String, serde_json::Value, String)> = apps
            .iter()
            .map(|app| {
                let app_id = app
                    .get("id")
                    .and_then(|v| v.as_str())
                    .unwrap_or("")
                    .to_string();
                let title = app
                    .get("title")
                    .and_then(|v| v.as_str())
                    .unwrap_or("App")
                    .to_string();
                let (score, reason) = if query.is_empty() {
                    (0.0, "listed".to_string())
                } else {
                    Self::score_deployed_app_match(query, &app_id, &title)
                        .unwrap_or((0.0, "no_match".to_string()))
                };
                (score, app_id, app.clone(), reason)
            })
            .collect();
        ranked_apps.sort_by(|a, b| {
            b.0.partial_cmp(&a.0)
                .unwrap_or(std::cmp::Ordering::Equal)
                .then_with(|| a.1.cmp(&b.1))
        });
        ranked_apps
    }

    pub(super) fn select_best_ranked_app<'a>(
        query: &str,
        ranked_apps: &'a [(f32, String, serde_json::Value, String)],
    ) -> Option<&'a (f32, String, serde_json::Value, String)> {
        if query.is_empty() {
            return if ranked_apps.len() == 1 {
                ranked_apps.first()
            } else {
                None
            };
        }

        ranked_apps.first().filter(|(score, _, _, reason)| {
            if matches!(reason.as_str(), "exact_id" | "exact_title") {
                return true;
            }

            let next_score = ranked_apps.get(1).map(|row| row.0).unwrap_or(0.0);
            let close_competing_match = next_score >= 0.45
                && ((*score - next_score) < 0.08
                    || (*score >= 0.55 && (*score - next_score) < 0.12));
            if close_competing_match {
                return false;
            }
            *score >= 0.55 || (*score >= 0.30 && (*score - next_score) >= 0.10)
        })
    }

    fn summarize_ranked_apps_for_user(
        ranked_apps: &[(f32, String, serde_json::Value, String)],
        limit: usize,
    ) -> Vec<serde_json::Value> {
        let local_base = Self::user_facing_local_base_url();
        ranked_apps
            .iter()
            .take(limit)
            .map(|(score, _, app, reason)| {
                let relative_url = app.get("url").and_then(|v| v.as_str()).unwrap_or("/apps/");
                let local_url =
                    Self::absolutize_public_url(Some(local_base.as_str()), relative_url);
                serde_json::json!({
                    "id": app.get("id").and_then(|v| v.as_str()).unwrap_or(""),
                    "title": app.get("title").and_then(|v| v.as_str()).unwrap_or("App"),
                    "running": app.get("running").and_then(|v| v.as_bool()).unwrap_or(false),
                    "local_url": local_url,
                    "match_score": score,
                    "match_reason": reason,
                })
            })
            .collect()
    }

    async fn collect_app_file_inventory(
        &self,
        app_dir: &std::path::Path,
        max_files: usize,
    ) -> (Vec<serde_json::Value>, usize, u64, bool) {
        let root = app_dir.to_path_buf();
        let capped_max = max_files.clamp(1, 200);
        (tokio::task::spawn_blocking(move || {
            let mut rows: Vec<(usize, String, u64)> = Vec::new();
            let mut total_files = 0usize;
            let mut total_bytes = 0u64;

            let walker = walkdir::WalkDir::new(&root)
                .into_iter()
                .filter_entry(|entry| {
                    if !entry.file_type().is_dir() {
                        return true;
                    }
                    entry
                        .file_name()
                        .to_str()
                        .map(|name| !Self::should_skip_app_inventory_dir(name))
                        .unwrap_or(true)
                });

            for entry in walker {
                let Ok(entry) = entry else {
                    continue;
                };
                if !entry.file_type().is_file() {
                    continue;
                }
                let relative = entry
                    .path()
                    .strip_prefix(&root)
                    .unwrap_or(entry.path())
                    .to_string_lossy()
                    .replace('\\', "/");
                if Self::should_skip_app_inventory_file(&relative) {
                    continue;
                }
                total_files += 1;
                let len = entry.metadata().map(|m| m.len()).unwrap_or(0);
                total_bytes = total_bytes.saturating_add(len);
                rows.push((Self::preferred_app_file_rank(&relative), relative, len));
            }

            rows.sort_by(|a, b| a.0.cmp(&b.0).then_with(|| a.1.cmp(&b.1)));
            let truncated = rows.len() > capped_max;
            let files = rows
                .into_iter()
                .take(capped_max)
                .map(|(_, path, bytes)| serde_json::json!({ "path": path, "bytes": bytes }))
                .collect::<Vec<_>>();
            (files, total_files, total_bytes, truncated)
        })
        .await)
            .unwrap_or_default()
    }

    async fn collect_app_file_previews(
        &self,
        app_dir: &std::path::Path,
        files: &[serde_json::Value],
    ) -> (serde_json::Map<String, serde_json::Value>, bool) {
        let mut previews = serde_json::Map::new();
        let mut included_bytes = 0usize;
        let mut omitted = false;
        let Ok(root) = tokio::fs::canonicalize(app_dir).await else {
            return (previews, true);
        };
        for row in files {
            let Some(relative) = row
                .get("path")
                .and_then(|value| value.as_str())
                .map(str::trim)
                .filter(|value| !value.is_empty())
            else {
                continue;
            };
            let bytes = row
                .get("bytes")
                .and_then(|value| value.as_u64())
                .unwrap_or(0) as usize;
            if bytes > APP_REGISTRY_PREVIEW_FILE_MAX_BYTES
                || included_bytes.saturating_add(bytes) > APP_REGISTRY_PREVIEW_TOTAL_MAX_BYTES
            {
                omitted = true;
                continue;
            }
            if Self::should_skip_app_inventory_file(relative) {
                omitted = true;
                continue;
            }
            let candidate = app_dir.join(relative);
            let Ok(canonical) = tokio::fs::canonicalize(&candidate).await else {
                continue;
            };
            if !canonical.starts_with(&root) {
                omitted = true;
                continue;
            }
            let Ok(raw) = tokio::fs::read(&canonical).await else {
                continue;
            };
            if raw.len() > APP_REGISTRY_PREVIEW_FILE_MAX_BYTES
                || included_bytes.saturating_add(raw.len()) > APP_REGISTRY_PREVIEW_TOTAL_MAX_BYTES
            {
                omitted = true;
                continue;
            }
            let Ok(content) = String::from_utf8(raw) else {
                omitted = true;
                continue;
            };
            included_bytes = included_bytes.saturating_add(content.len());
            previews.insert(relative.to_string(), serde_json::json!(content));
        }
        (previews, omitted)
    }

    async fn build_deployed_app_registry_inspection(
        &self,
        app: &serde_json::Value,
        include_files: bool,
        include_logs: bool,
    ) -> Option<serde_json::Value> {
        let app_id = app
            .get("id")
            .and_then(|v| v.as_str())
            .map(str::trim)
            .filter(|v| !v.is_empty())?;
        let title = app
            .get("title")
            .and_then(|v| v.as_str())
            .unwrap_or("App")
            .to_string();
        let app_dir = self.app_registry.get_dir(app_id).await?;
        let meta_path = app_dir.join(".app_meta.json");
        let meta = self.load_app_metadata(app_id).await;

        let local_base = Self::user_facing_local_base_url();
        let relative_url = app.get("url").and_then(|v| v.as_str()).unwrap_or("/apps/");
        let local_url = Self::absolutize_public_url(Some(local_base.as_str()), relative_url);
        let relative_access_url = app
            .get("access_url")
            .and_then(|v| v.as_str())
            .unwrap_or(relative_url);
        let local_access_url =
            Self::absolutize_public_url(Some(local_base.as_str()), relative_access_url);
        let access_guard_enabled = app
            .get("access_guard_enabled")
            .and_then(|v| v.as_bool())
            .unwrap_or(false);
        let access_key = if access_guard_enabled {
            app.get("access_password")
                .or_else(|| app.get("access_key"))
                .and_then(|v| v.as_str())
                .unwrap_or("")
                .to_string()
        } else {
            String::new()
        };
        let required_inputs = meta
            .as_ref()
            .map(crate::actions::app::parse_required_inputs)
            .unwrap_or_default()
            .into_iter()
            .map(|item| {
                serde_json::json!({
                    "key": item.key,
                    "sensitive": item.sensitive,
                })
            })
            .collect::<Vec<_>>();
        let config_keys = meta
            .as_ref()
            .and_then(|m| m.get("config_values").and_then(|v| v.as_object()))
            .map(|obj| obj.keys().cloned().collect::<Vec<_>>())
            .unwrap_or_default();

        let (files, file_count, file_bytes, file_list_truncated) = if include_files {
            self.collect_app_file_inventory(&app_dir, 48).await
        } else {
            (Vec::new(), 0, 0, false)
        };
        let suggested_read_files = files
            .iter()
            .filter_map(|row| row.get("path").and_then(|v| v.as_str()))
            .take(8)
            .map(ToString::to_string)
            .collect::<Vec<_>>();
        let (file_previews, file_previews_omitted) = if include_files {
            self.collect_app_file_previews(&app_dir, &files).await
        } else {
            (serde_json::Map::new(), false)
        };
        let recent_runtime_logs = if include_logs
            && !app
                .get("is_static")
                .and_then(|v| v.as_bool())
                .unwrap_or(true)
        {
            if let Some(executor) = build_executor_client() {
                match executor.app_logs(app_id, 4096).await {
                    Ok(logs) if !logs.logs.trim().is_empty() => Some(logs.logs),
                    _ => {
                        let tail =
                            crate::actions::app::read_local_runtime_log_tail(&app_dir, 4096).await;
                        if tail.trim().is_empty() {
                            None
                        } else {
                            Some(tail)
                        }
                    }
                }
            } else {
                let tail = crate::actions::app::read_local_runtime_log_tail(&app_dir, 4096).await;
                if tail.trim().is_empty() {
                    None
                } else {
                    Some(tail)
                }
            }
        } else {
            None
        };

        let mut out = serde_json::json!({
            "id": app_id,
            "title": title,
            "app_dir": app_dir.to_string_lossy().to_string(),
            "metadata_path": meta_path.to_string_lossy().to_string(),
          "url": relative_url,
          "local_url": local_url,
          "access_url": relative_access_url,
          "local_access_url": local_access_url,
          "enabled": app.get("enabled").and_then(|v| v.as_bool()).unwrap_or(true),
          "running": app.get("running").and_then(|v| v.as_bool()).unwrap_or(false),
          "is_static": app.get("is_static").and_then(|v| v.as_bool()).unwrap_or(true),
          "runtime_mode": app.get("runtime_mode").and_then(|v| v.as_str()).unwrap_or("unknown"),
          "created_at": app.get("created_at").and_then(|v| v.as_str()).unwrap_or(""),
          "port": app.get("port").and_then(|v| v.as_u64()),
          "access_guard_enabled": access_guard_enabled,
          "access_key": access_key,
          "access_password": access_key,
          "entry_command": meta.as_ref().and_then(|m| m.get("entry_command").and_then(|v| v.as_str())),
          "install_command": meta.as_ref().and_then(|m| m.get("install_command").and_then(|v| v.as_str())),
          "runtime_preference": meta.as_ref().and_then(|m| m.get("runtime_preference").and_then(|v| v.as_str())),
          "runtime_required": meta.as_ref().and_then(|m| m.get("runtime_required").and_then(|v| v.as_bool())).unwrap_or(false),
          "runtime_reason": meta.as_ref().and_then(|m| m.get("runtime_reason").and_then(|v| v.as_str())),
          "expose_public": meta.as_ref().and_then(|m| m.get("expose_public").and_then(|v| v.as_bool())).unwrap_or(false),
            "runtime_image": meta.as_ref().and_then(|m| m.get("runtime_image").and_then(|v| v.as_str())),
            "repo_url": meta.as_ref().and_then(|m| m.get("repo_url").and_then(|v| v.as_str())),
            "repo_ref": meta.as_ref().and_then(|m| m.get("repo_ref").and_then(|v| v.as_str())),
            "repo_bundle_id": meta.as_ref().and_then(|m| m.get("repo_bundle_id").and_then(|v| v.as_str())),
            "repo_service_kind": meta.as_ref().and_then(|m| m.get("repo_service_kind").and_then(|v| v.as_str())),
            "repo_service_dir": meta.as_ref().and_then(|m| m.get("repo_service_dir").and_then(|v| v.as_str())),
            "required_inputs": required_inputs,
            "config_keys": config_keys,
            "file_count": file_count,
            "file_bytes": file_bytes,
            "suggested_read_files": suggested_read_files,
            "suggested_actions": ["file_read", "file_write", "app_restart", "app_stop", "app_delete", "http_get"],
        });
        if let Some(obj) = out.as_object_mut() {
            if !app
                .get("is_static")
                .and_then(|value| value.as_bool())
                .unwrap_or(true)
            {
                if let Some(executor) = build_executor_client() {
                    if let Ok(status) = executor.app_status(app_id).await {
                        obj.insert("running".to_string(), serde_json::json!(status.running));
                        obj.insert(
                            "runtime_mode".to_string(),
                            serde_json::json!(status
                                .runtime_mode
                                .unwrap_or_else(|| "stopped".to_string())),
                        );
                        obj.insert(
                            "port".to_string(),
                            status
                                .port
                                .map(serde_json::Value::from)
                                .unwrap_or(serde_json::Value::Null),
                        );
                        obj.insert(
                            "is_isolated_runtime".to_string(),
                            serde_json::json!(status.is_isolated_runtime),
                        );
                    }
                }
            }
            if include_files {
                obj.insert("files".to_string(), serde_json::json!(files));
                if !file_previews.is_empty() {
                    obj.insert(
                        "file_previews".to_string(),
                        serde_json::Value::Object(file_previews),
                    );
                }
                obj.insert(
                    "file_list_truncated".to_string(),
                    serde_json::json!(file_list_truncated),
                );
                obj.insert(
                    "file_previews_omitted".to_string(),
                    serde_json::json!(file_previews_omitted),
                );
            }
            if let Some(log_tail) = recent_runtime_logs {
                obj.insert(
                    "recent_runtime_logs".to_string(),
                    serde_json::json!(log_tail),
                );
            }
        }
        Some(out)
    }

    pub(crate) async fn load_conversation_workspace_snapshot(
        &self,
        conversation_id: &str,
    ) -> Option<serde_json::Value> {
        let recent_artifact = self.load_recent_artifact_context(conversation_id).await?;
        if !recent_artifact.artifact_type.eq_ignore_ascii_case("app") {
            return None;
        }
        let target_app_id = recent_artifact.artifact_id.trim();
        if target_app_id.is_empty() {
            return None;
        }
        let apps = self.app_registry.list().await;
        let app = apps.iter().find(|candidate| {
            candidate
                .get("id")
                .and_then(|value| value.as_str())
                .map(str::trim)
                == Some(target_app_id)
        })?;
        self.build_deployed_app_registry_inspection(app, true, false)
            .await
    }

    async fn load_app_metadata(&self, app_id: &str) -> Option<serde_json::Value> {
        let app_dir = self.app_registry.get_dir(app_id).await?;
        let meta_path = app_dir.join(".app_meta.json");
        tokio::fs::read(&meta_path)
            .await
            .ok()
            .and_then(|bytes| serde_json::from_slice(&bytes).ok())
            .filter(|value: &serde_json::Value| value.is_object())
    }

    async fn find_repo_bundle_apps(
        &self,
        bundle_id: &str,
    ) -> Vec<(serde_json::Value, serde_json::Value)> {
        let needle = bundle_id.trim();
        if needle.is_empty() {
            return Vec::new();
        }
        let apps = self.app_registry.list().await;
        let mut matched = Vec::new();
        for app in apps {
            let Some(app_id) = app.get("id").and_then(|v| v.as_str()) else {
                continue;
            };
            let Some(meta) = self.load_app_metadata(app_id).await else {
                continue;
            };
            if meta
                .get("repo_bundle_id")
                .and_then(|v| v.as_str())
                .is_some_and(|value| value == needle)
            {
                matched.push((app, meta));
            }
        }
        matched
    }

    async fn cleanup_repo_bundle_artifacts(&self, bundle_id: &str) -> Result<()> {
        let cleaned = bundle_id.trim();
        if cleaned.is_empty() {
            return Ok(());
        }
        let bundle_dir = self.data_dir.join("repo-deployments").join(cleaned);
        match tokio::fs::remove_dir_all(&bundle_dir).await {
            Ok(_) => Ok(()),
            Err(error) if error.kind() == std::io::ErrorKind::NotFound => Ok(()),
            Err(error) => Err(anyhow::anyhow!(
                "Failed to remove repo deployment bundle '{}': {}",
                cleaned,
                error
            )),
        }
    }

    fn sample_overlap_tokens(
        left: &std::collections::HashSet<String>,
        right: &std::collections::HashSet<String>,
        max_items: usize,
    ) -> Vec<String> {
        let mut overlap: Vec<String> = left.intersection(right).cloned().collect();
        overlap.sort_unstable();
        overlap.into_iter().take(max_items).collect()
    }

    fn build_requested_app_fingerprint(
        arguments: &serde_json::Value,
    ) -> Option<AppSemanticFingerprint> {
        let requested_files = arguments.get("files").and_then(|v| v.as_object())?;
        let requested_title = arguments
            .get("title")
            .and_then(|v| v.as_str())
            .map(Self::normalize_app_title)
            .unwrap_or_default();
        let is_static = arguments
            .get("entry_command")
            .and_then(|v| v.as_str())
            .is_none();

        let mut title_tokens = std::collections::HashSet::new();
        Self::append_semantic_tokens(&mut title_tokens, &requested_title, 24);

        let mut file_tokens = std::collections::HashSet::new();
        let mut keyword_tokens = std::collections::HashSet::new();
        for (path, value) in requested_files {
            Self::append_semantic_tokens(&mut file_tokens, path, 80);
            if let Some(content) = value.as_str() {
                // Bound extraction cost on very large generated files.
                let excerpt = content.chars().take(20_000).collect::<String>();
                Self::append_semantic_tokens(&mut keyword_tokens, &excerpt, 420);
            }
        }

        Some(AppSemanticFingerprint {
            title_tokens,
            keyword_tokens,
            file_tokens,
            is_static,
        })
    }

    async fn build_existing_app_fingerprint(
        &self,
        app_id: &str,
        app: &serde_json::Value,
    ) -> Option<AppSemanticFingerprint> {
        let app_dir = self.app_registry.get_dir(app_id).await?;
        let title = app
            .get("title")
            .and_then(|v| v.as_str())
            .map(Self::normalize_app_title)
            .unwrap_or_default();
        let is_static = app
            .get("is_static")
            .and_then(|v| v.as_bool())
            .unwrap_or(true);

        let mut title_tokens = std::collections::HashSet::new();
        Self::append_semantic_tokens(&mut title_tokens, &title, 24);

        let mut file_tokens = std::collections::HashSet::new();
        let mut keyword_tokens = std::collections::HashSet::new();
        let mut dirs = vec![app_dir.clone()];
        let mut files_seen = 0usize;
        let mut char_budget = 120_000usize;

        while let Some(dir) = dirs.pop() {
            let mut entries = match tokio::fs::read_dir(&dir).await {
                Ok(v) => v,
                Err(_) => continue,
            };
            while let Ok(Some(entry)) = entries.next_entry().await {
                let path = entry.path();
                let metadata = match entry.metadata().await {
                    Ok(m) => m,
                    Err(_) => continue,
                };
                if metadata.is_dir() {
                    dirs.push(path);
                    continue;
                }
                files_seen += 1;
                if files_seen > 64 {
                    break;
                }
                let relative = path
                    .strip_prefix(&app_dir)
                    .unwrap_or(path.as_path())
                    .to_string_lossy()
                    .replace('\\', "/");
                if Self::should_skip_app_inventory_file(&relative) {
                    continue;
                }
                Self::append_semantic_tokens(&mut file_tokens, &relative, 120);
                if char_budget == 0 {
                    continue;
                }
                if metadata.len() > 1_000_000 {
                    continue;
                }
                let Ok(content) = tokio::fs::read(&path).await else {
                    continue;
                };
                let take_chars = std::cmp::min(char_budget, 24_000);
                let Some(excerpt) = Self::extract_semantic_excerpt_from_bytes(&content, take_chars)
                else {
                    continue;
                };
                char_budget = char_budget.saturating_sub(excerpt.chars().count());
                Self::append_semantic_tokens(&mut keyword_tokens, &excerpt, 520);
            }
        }

        Some(AppSemanticFingerprint {
            title_tokens,
            keyword_tokens,
            file_tokens,
            is_static,
        })
    }

    fn score_app_similarity(
        requested: &AppSemanticFingerprint,
        existing: &AppSemanticFingerprint,
    ) -> (f32, String) {
        let title_score = Self::jaccard_similarity(&requested.title_tokens, &existing.title_tokens);
        let keyword_score =
            Self::jaccard_similarity(&requested.keyword_tokens, &existing.keyword_tokens);
        let file_score = Self::jaccard_similarity(&requested.file_tokens, &existing.file_tokens);
        let runtime_bonus = if requested.is_static == existing.is_static {
            0.05
        } else {
            0.0
        };
        let score =
            (0.35 * title_score) + (0.40 * keyword_score) + (0.20 * file_score) + runtime_bonus;
        let overlaps =
            Self::sample_overlap_tokens(&requested.keyword_tokens, &existing.keyword_tokens, 5);
        let overlap_text = if overlaps.is_empty() {
            "no strong shared keywords".to_string()
        } else {
            format!("shared keywords: {}", overlaps.join(", "))
        };
        let reason = format!(
            "{} | title {:.0}%, content {:.0}%, files {:.0}%",
            overlap_text,
            title_score * 100.0,
            keyword_score * 100.0,
            file_score * 100.0
        );
        (score, reason)
    }

    async fn app_files_match_existing(
        &self,
        app_id: &str,
        requested_files: &serde_json::Map<String, serde_json::Value>,
    ) -> bool {
        if requested_files.is_empty() {
            return false;
        }
        let Some(app_dir) = self.app_registry.get_dir(app_id).await else {
            return false;
        };
        for (relative_path, content_value) in requested_files {
            let Some(expected) = content_value.as_str() else {
                return false;
            };
            if relative_path.contains("..")
                || relative_path.starts_with('/')
                || relative_path.starts_with('\\')
            {
                return false;
            }
            let file_path = app_dir.join(relative_path);
            let actual = match tokio::fs::read_to_string(&file_path).await {
                Ok(v) => v,
                Err(_) => return false,
            };
            if actual != expected {
                return false;
            }
        }
        true
    }

    async fn validate_rendered_app_against_request(
        &self,
        request_context: &str,
        rendered: &crate::integrations::browser::PageContent,
        stream_tx: Option<&tokio::sync::mpsc::Sender<StreamEvent>>,
    ) -> Result<(bool, String)> {
        let request_context = request_context.trim();
        if request_context.is_empty() {
            return Ok((
                true,
                "No request context was available for semantic acceptance validation.".to_string(),
            ));
        }

        if let Some(tx) = stream_tx {
            let detail = "Checking the rendered app against the requested outcome.";
            queue_stream_event(
                tx,
                StreamEvent::ToolProgress {
                    name: "app_deploy".to_string(),
                    content: detail.to_string(),
                    payload: Some(phase_status_payload(
                        "app_deploy",
                        "validating_acceptance",
                        "Validating acceptance",
                        detail,
                        0,
                    )),
                },
            );
        }

        let elements = rendered
            .elements
            .iter()
            .take(60)
            .map(|element| {
                serde_json::json!({
                    "tag": &element.tag,
                    "type": &element.r#type,
                    "text": &element.text,
                    "name": &element.name,
                    "id": &element.id,
                    "href": &element.href,
                    "x": element.x,
                    "y": element.y,
                })
            })
            .collect::<Vec<_>>();
        let diagnostics = rendered
            .diagnostics
            .iter()
            .take(60)
            .map(|entry| {
                serde_json::json!({
                    "kind": &entry.kind,
                    "severity": &entry.severity,
                    "message": &entry.message,
                    "url": &entry.url,
                    "resource_type": &entry.resource_type,
                })
            })
            .collect::<Vec<_>>();
        let prompt = serde_json::json!({
            "request_context": request_context,
            "rendered_page": {
                "title": &rendered.title,
                "url": &rendered.url,
                "body_text": safe_truncate(&rendered.body_text, 6000),
                "interactive_elements": elements,
                "browser_diagnostics": diagnostics,
            },
            "judgement_rules": [
                "Judge semantically from the request and rendered-page evidence, not by exact wording alone.",
                "Pass only when the visible page and interactive elements provide concrete evidence that the requested product, content, workflow, controls, and fallback states are present.",
                "Treat browser diagnostics, runtime errors, blocked embeds, missing controls, placeholder-only content, or an empty/locked page as acceptance failures unless the rendered page visibly handles that condition as requested.",
                "Do not assume hidden source code works if the rendered evidence does not show the requested behavior or controls.",
                "Return JSON only."
            ],
            "required_output": {
                "passed": "boolean",
                "summary": "short evidence-based sentence",
                "missing_or_broken": ["items that are absent, broken, or not evidenced"]
            }
        });

        let response = self
            .supervised_internal_chat(
                "automation",
                "app_deploy_acceptance_validation",
                "app_deploy_acceptance_validation",
                &ModelRole::Fast,
                self.llm_candidates_for_role(&ModelRole::Fast),
                "You validate a deployed app against the user's requested outcome using rendered DOM evidence, interactive element labels, and browser diagnostics. Be strict: successful deployment requires evidence that the requested app is actually present and usable, not just that a nonblank page loaded. Return strict JSON only.",
                &prompt.to_string(),
                &[],
                &[],
                30_000,
                1,
            )
            .await
            .ok_or_else(|| anyhow::anyhow!("acceptance validation model returned no response"))?;

        let parsed = extract_json_object_from_text(&response.content).ok_or_else(|| {
            anyhow::anyhow!("acceptance validation response did not contain a JSON object")
        })?;
        let passed = parsed
            .get("passed")
            .and_then(|value| value.as_bool())
            .unwrap_or(false);
        let summary = parsed
            .get("summary")
            .and_then(|value| value.as_str())
            .map(str::trim)
            .filter(|value| !value.is_empty())
            .unwrap_or(if passed {
                "Rendered page satisfies the requested outcome."
            } else {
                "Rendered page does not provide enough evidence of the requested outcome."
            });
        let missing = parsed
            .get("missing_or_broken")
            .and_then(|value| value.as_array())
            .map(|items| {
                items
                    .iter()
                    .filter_map(|item| item.as_str().map(str::trim))
                    .filter(|item| !item.is_empty())
                    .take(6)
                    .map(ToString::to_string)
                    .collect::<Vec<_>>()
            })
            .unwrap_or_default();
        let detail = if missing.is_empty() {
            summary.to_string()
        } else {
            format!("{} Missing/broken: {}", summary, missing.join("; "))
        };
        Ok((passed, safe_truncate(&detail, 900)))
    }

    async fn find_existing_duplicate_app(
        &self,
        arguments: &serde_json::Value,
    ) -> Option<AppDuplicateMatch> {
        let requested_title = arguments
            .get("title")
            .and_then(|v| v.as_str())
            .map(Self::normalize_app_title)
            .unwrap_or_default();
        let requested_files = arguments.get("files").and_then(|v| v.as_object())?;
        let requested_fingerprint = Self::build_requested_app_fingerprint(arguments)?;
        let apps = self.app_registry.list().await;
        let mut best_fuzzy: Option<AppDuplicateMatch> = None;
        const SIMILARITY_THRESHOLD: f32 = 0.58;

        for app in apps {
            let app_id = app
                .get("id")
                .and_then(|v| v.as_str())
                .map(str::trim)
                .unwrap_or("");
            if app_id.is_empty() {
                continue;
            }
            let existing_title = app
                .get("title")
                .and_then(|v| v.as_str())
                .map(Self::normalize_app_title)
                .unwrap_or_default();
            let title_match = !requested_title.is_empty()
                && !existing_title.is_empty()
                && !Self::is_generic_title(&requested_title)
                && requested_title == existing_title;
            let files_match = self.app_files_match_existing(app_id, requested_files).await;
            if files_match {
                return Some(AppDuplicateMatch {
                    app,
                    match_kind: "exact_files",
                    score: 1.0,
                    reason: "files are identical".to_string(),
                });
            }
            if title_match {
                return Some(AppDuplicateMatch {
                    app,
                    match_kind: "exact_title",
                    score: 0.92,
                    reason: "title matches exactly".to_string(),
                });
            }

            let Some(existing_fingerprint) =
                self.build_existing_app_fingerprint(app_id, &app).await
            else {
                continue;
            };
            let (score, reason) =
                Self::score_app_similarity(&requested_fingerprint, &existing_fingerprint);
            if score < SIMILARITY_THRESHOLD {
                continue;
            }
            let should_replace = best_fuzzy.as_ref().map(|m| score > m.score).unwrap_or(true);
            if should_replace {
                best_fuzzy = Some(AppDuplicateMatch {
                    app,
                    match_kind: "fuzzy",
                    score,
                    reason,
                });
            }
        }

        best_fuzzy
    }

    async fn validate_and_capture_app_preview(
        &self,
        app_url_with_key: &str,
        app_id: &str,
        app_type: &str,
        app_access_key: Option<&str>,
        stream_tx: Option<&tokio::sync::mpsc::Sender<StreamEvent>>,
    ) -> Result<(
        Option<String>,
        bool,
        usize,
        String,
        Option<crate::integrations::browser::PageContent>,
    )> {
        let validation_started = std::time::Instant::now();
        tracing::debug!(
            target: "agentark.turn_timing",
            stage = "app_deploy_validation_start",
            app_id = %app_id,
            app_type = %app_type,
            has_access_key = app_access_key
                .map(str::trim)
                .is_some_and(|value| !value.is_empty()),
            "app deploy validation timing start"
        );
        let http_client = Self::build_internal_control_client().ok();
        let internal_probe_url = if app_url_with_key.starts_with("http://")
            || app_url_with_key.starts_with("https://")
        {
            app_url_with_key.to_string()
        } else if app_url_with_key.starts_with('/') {
            format!("{}{}", Self::internal_api_base_url(), app_url_with_key)
        } else {
            format!("{}/{}", Self::internal_api_base_url(), app_url_with_key)
        };

        let Some(client) = http_client else {
            tracing::debug!(
                target: "agentark.turn_timing",
                stage = "app_deploy_validation_total",
                app_id = %app_id,
                app_type = %app_type,
                duration_ms = validation_started.elapsed().as_millis() as u64,
                success = false,
                issue = "http_client_unavailable",
                "app deploy validation timing total"
            );
            return Ok((
                None,
                false,
                0,
                "Local structural validation could not run because the HTTP probe client is unavailable."
                    .to_string(),
                None,
            ));
        };

        if let Some(tx) = stream_tx {
            let detail = "Checking the local deployed app with a structural HTTP probe.";
            queue_stream_event(
                tx,
                StreamEvent::ToolProgress {
                    name: "app_deploy".to_string(),
                    content: detail.to_string(),
                    payload: Some(phase_status_payload(
                        "app_deploy",
                        "validating",
                        "Validating",
                        detail,
                        0,
                    )),
                },
            );
        }

        let mut probe_request = client.get(&internal_probe_url);
        if let Some(access_key) = app_access_key
            .map(str::trim)
            .filter(|value| !value.is_empty())
        {
            probe_request = probe_request.header("x-agentark-app-key", access_key);
        }

        let probe_started = std::time::Instant::now();
        let mut last_error = match probe_request.send().await {
            Ok(resp) if resp.status().is_success() => {
                let status = resp.status();
                let content_type = resp
                    .headers()
                    .get(reqwest::header::CONTENT_TYPE)
                    .and_then(|value| value.to_str().ok())
                    .unwrap_or("")
                    .to_ascii_lowercase();
                let body = match resp.text().await {
                    Ok(body) => body,
                    Err(error) => {
                        tracing::debug!(
                            target: "agentark.turn_timing",
                            stage = "app_deploy_validation_probe",
                            app_id = %app_id,
                            app_type = %app_type,
                            duration_ms = probe_started.elapsed().as_millis() as u64,
                            success = false,
                            status = %status,
                            issue = "body_read_failed",
                            "app deploy validation probe failed"
                        );
                        return Ok((
                            None,
                            false,
                            1,
                            format!(
                                "HTTP probe reached the app with status {} but the response body could not be read: {}",
                                status, error
                            ),
                            None,
                        ));
                    }
                };
                match Self::validate_structural_app_probe_body(
                    status,
                    app_type,
                    &content_type,
                    &body,
                ) {
                    Ok(detail) => {
                        tracing::debug!(
                            target: "agentark.turn_timing",
                            stage = "app_deploy_validation_total",
                            app_id = %app_id,
                            app_type = %app_type,
                            duration_ms = validation_started.elapsed().as_millis() as u64,
                            probe_duration_ms = probe_started.elapsed().as_millis() as u64,
                            success = true,
                            status = %status,
                            "app deploy validation timing total"
                        );
                        return Ok((None, true, 1, detail, None));
                    }
                    Err(detail) => detail,
                }
            }
            Ok(resp) => {
                tracing::debug!(
                    target: "agentark.turn_timing",
                    stage = "app_deploy_validation_probe",
                    app_id = %app_id,
                    app_type = %app_type,
                    duration_ms = probe_started.elapsed().as_millis() as u64,
                    success = false,
                    status = %resp.status(),
                    "app deploy validation probe failed"
                );
                format!("HTTP probe failed with status {}", resp.status())
            }
            Err(error) => {
                tracing::debug!(
                    target: "agentark.turn_timing",
                    stage = "app_deploy_validation_probe",
                    app_id = %app_id,
                    app_type = %app_type,
                    duration_ms = probe_started.elapsed().as_millis() as u64,
                    success = false,
                    error = %safe_truncate(&error.to_string(), 240),
                    "app deploy validation probe failed"
                );
                format!("HTTP probe request failed: {}", error)
            }
        };

        if let Some(runtime_hint) = self.build_app_runtime_failure_hint(app_id).await {
            last_error = format!("{}\n{}", last_error, runtime_hint);
        }
        tracing::debug!(
            target: "agentark.turn_timing",
            stage = "app_deploy_validation_total",
            app_id = %app_id,
            app_type = %app_type,
            duration_ms = validation_started.elapsed().as_millis() as u64,
            success = false,
            detail = %safe_truncate(&last_error, 240),
            "app deploy validation timing total"
        );
        Ok((None, false, 1, last_error, None))
    }

    fn validate_structural_app_probe_body(
        status: reqwest::StatusCode,
        app_type: &str,
        content_type: &str,
        body: &str,
    ) -> std::result::Result<String, String> {
        let body_trimmed = body.trim();
        let content_type_label = if content_type.trim().is_empty() {
            "unknown"
        } else {
            content_type.trim()
        };
        if body_trimmed.is_empty() {
            return Err(format!(
                "HTTP probe returned an empty body with status {}",
                status
            ));
        }

        let lower = body.to_ascii_lowercase();
        let lower_content_type = content_type.to_ascii_lowercase();
        let mime_is_html = lower_content_type.contains("html");
        let body_has_html_marker =
            lower.contains("<!doctype html") || lower.contains("<html") || lower.contains("<body");
        let is_html_response = mime_is_html || body_has_html_marker;
        let expects_html = app_type.trim().eq_ignore_ascii_case("static");
        if expects_html && !is_html_response {
            return Err(format!(
                "HTTP probe reached the static app with status {} but did not receive HTML content (content-type: {})",
                status, content_type_label
            ));
        }
        if is_html_response
            && !(lower.contains("<!doctype html")
                || lower.contains("<html")
                || lower.contains("<body")
                || lower.contains("<main")
                || lower.contains("<div"))
        {
            return Err(format!(
                "HTTP probe reached the app with status {} but the HTML body did not contain a document/root signal",
                status
            ));
        }
        if lower.contains("agentark app guard") {
            return Err(
                "HTTP probe reached the app guard page instead of the deployed app.".to_string(),
            );
        }
        if is_html_response {
            if let Some(tag_name) = crate::actions::app::detect_unclosed_html_raw_text_element(body)
            {
                return Err(format!(
                    "HTTP probe reached malformed HTML: unclosed <{}> block",
                    tag_name
                ));
            }
        } else if let Some(marker) =
            Self::detect_http_probe_runtime_error_marker(content_type, body)
        {
            return Err(format!(
                "HTTP probe body reports runtime error marker: {}",
                marker
            ));
        }

        Ok(format!(
            "Local structural HTTP probe passed with status {} (content-type: {}).",
            status, content_type_label
        ))
    }

    fn rendered_app_content_has_dom_signal(
        content: &crate::integrations::browser::PageContent,
    ) -> bool {
        !content.body_text.trim().is_empty() || !content.elements.is_empty()
    }

    #[cfg(feature = "image")]
    fn app_screenshot_has_visual_signal(bytes: &[u8]) -> bool {
        let Ok(image) = image::load_from_memory(bytes) else {
            return !bytes.is_empty();
        };
        let rgba = image.to_rgba8();
        let (width, height) = rgba.dimensions();
        if width == 0 || height == 0 {
            return false;
        }

        let step_x = (width / 64).max(1);
        let step_y = (height / 64).max(1);
        let mut samples = 0u32;
        let mut opaque_samples = 0u32;
        let mut min_rgb = [u8::MAX; 3];
        let mut max_rgb = [u8::MIN; 3];

        let mut y = 0;
        while y < height {
            let mut x = 0;
            while x < width {
                let pixel = rgba.get_pixel(x, y).0;
                if pixel[3] > 8 {
                    opaque_samples = opaque_samples.saturating_add(1);
                }
                for idx in 0..3 {
                    min_rgb[idx] = min_rgb[idx].min(pixel[idx]);
                    max_rgb[idx] = max_rgb[idx].max(pixel[idx]);
                }
                samples = samples.saturating_add(1);
                x = x.saturating_add(step_x);
            }
            y = y.saturating_add(step_y);
        }

        samples > 0
            && opaque_samples > 0
            && (0..3).any(|idx| max_rgb[idx].saturating_sub(min_rgb[idx]) > 12)
    }

    #[cfg(not(feature = "image"))]
    fn app_screenshot_has_visual_signal(bytes: &[u8]) -> bool {
        !bytes.is_empty()
    }

    async fn app_meta_updated_at(app_dir: &std::path::Path) -> Option<String> {
        tokio::fs::read(app_dir.join(".app_meta.json"))
            .await
            .ok()
            .and_then(|bytes| serde_json::from_slice::<serde_json::Value>(&bytes).ok())
            .and_then(|meta| {
                meta.get("updated_at")
                    .and_then(|v| v.as_str())
                    .map(str::to_string)
            })
    }

    async fn write_app_json_file_atomic(
        app_dir: &std::path::Path,
        file_name: &str,
        value: &serde_json::Value,
    ) -> Result<()> {
        tokio::fs::create_dir_all(app_dir).await?;
        let target = app_dir.join(file_name);
        let tmp = app_dir.join(format!(
            ".{}.{}.tmp",
            file_name,
            uuid::Uuid::new_v4().simple()
        ));
        let bytes = serde_json::to_vec_pretty(value)?;
        tokio::fs::write(&tmp, bytes).await?;
        match tokio::fs::rename(&tmp, &target).await {
            Ok(_) => Ok(()),
            Err(rename_error) => {
                if target.exists() {
                    let _ = tokio::fs::remove_file(&target).await;
                    tokio::fs::rename(&tmp, &target).await.with_context(|| {
                        format!(
                            "replace {} after rename failure: {}",
                            target.display(),
                            rename_error
                        )
                    })
                } else {
                    Err(rename_error).with_context(|| format!("write {}", target.display()))
                }
            }
        }
    }

    async fn persist_app_sub_goals(&self, app_id: &str, sub_goals: &[String]) -> Result<()> {
        let Some(app_dir) = self.app_registry.get_dir(app_id).await else {
            return Ok(());
        };
        let items = sub_goals
            .iter()
            .map(|goal| goal.trim())
            .filter(|goal| !goal.is_empty())
            .take(20)
            .enumerate()
            .map(|(idx, goal)| {
                serde_json::json!({
                    "id": format!("goal_{}", idx + 1),
                    "summary": safe_truncate(goal, 300),
                })
            })
            .collect::<Vec<_>>();
        let payload = serde_json::json!({
            "schema_version": 1,
            "app_id": app_id,
            "created_at": chrono::Utc::now().to_rfc3339(),
            "items": items,
        });
        Self::write_app_json_file_atomic(
            &app_dir,
            crate::actions::app::APP_SUB_GOALS_FILE,
            &payload,
        )
        .await
    }

    async fn queue_app_quality_check(
        &self,
        app_id: &str,
        app_url_with_key: &str,
        app_type: &str,
        request_context: &str,
        sub_goals: &[String],
    ) {
        let Some(app_dir) = self.app_registry.get_dir(app_id).await else {
            return;
        };
        let app_updated_at = Self::app_meta_updated_at(&app_dir).await;
        if let Err(error) = self.persist_app_sub_goals(app_id, sub_goals).await {
            tracing::debug!(app_id = %app_id, error = %error, "app_sub_goals_persist_failed");
        }
        let pending = serde_json::json!({
            "schema_version": 1,
            "status": "pending",
            "app_id": app_id,
            "app_type": app_type,
            "app_updated_at": app_updated_at.clone(),
            "started_at": chrono::Utc::now().to_rfc3339(),
            "completed_at": serde_json::Value::Null,
            "advisory_only": true,
            "control_effect": "none",
            "detail": "A background quality check is queued for this app.",
        });
        if let Err(error) = Self::write_app_json_file_atomic(
            &app_dir,
            crate::actions::app::APP_QUALITY_REPORT_FILE,
            &pending,
        )
        .await
        {
            tracing::debug!(app_id = %app_id, error = %error, "app_quality_pending_write_failed");
        }

        let agent = self.clone();
        let app_id = app_id.to_string();
        let app_url_with_key = app_url_with_key.to_string();
        let app_type = app_type.to_string();
        let request_context = request_context.to_string();
        let sub_goals = sub_goals.to_vec();
        crate::spawn_logged!(
            "src/core/agent/tool_execution.rs:app_quality_check",
            async move {
                if let Err(error) = agent
                    .run_app_quality_check(
                        app_id,
                        app_url_with_key,
                        app_type,
                        request_context,
                        sub_goals,
                        app_updated_at,
                    )
                    .await
                {
                    tracing::debug!(error = %error, "app_quality_check_failed");
                }
            }
        );
    }

    async fn run_app_quality_check(
        &self,
        app_id: String,
        app_url_with_key: String,
        app_type: String,
        request_context: String,
        sub_goals: Vec<String>,
        expected_updated_at: Option<String>,
    ) -> Result<()> {
        let Some(app_dir) = self.app_registry.get_dir(&app_id).await else {
            return Ok(());
        };
        if Self::app_meta_updated_at(&app_dir).await != expected_updated_at {
            return Ok(());
        }

        let report = self
            .build_app_quality_report(
                &app_id,
                &app_url_with_key,
                &app_type,
                &request_context,
                &sub_goals,
                expected_updated_at.as_deref(),
            )
            .await;
        if Self::app_meta_updated_at(&app_dir).await != expected_updated_at {
            return Ok(());
        }
        Self::write_app_json_file_atomic(
            &app_dir,
            crate::actions::app::APP_QUALITY_REPORT_FILE,
            &report,
        )
        .await
    }

    async fn build_app_quality_report(
        &self,
        app_id: &str,
        app_url_with_key: &str,
        app_type: &str,
        request_context: &str,
        sub_goals: &[String],
        app_updated_at: Option<&str>,
    ) -> serde_json::Value {
        let started_at = chrono::Utc::now().to_rfc3339();
        let browser_url = Self::browser_validation_app_url(app_url_with_key);
        let integration = crate::integrations::browser::BrowserIntegration::new();
        let mut diagnostics = Vec::<serde_json::Value>::new();
        let mut screenshot_url: Option<String> = None;
        let mut rendered_content: Option<crate::integrations::browser::PageContent> = None;

        let browser_result: Result<()> = async {
            let sidecar_session = integration.create_session().await?;
            let result: Result<()> = async {
                let (final_url, title) =
                    integration.navigate(&sidecar_session, &browser_url).await?;
                tokio::time::sleep(std::time::Duration::from_secs(APP_QUALITY_SETTLE_SECS)).await;
                let content = integration.get_content(&sidecar_session).await?;
                let screenshot = integration
                    .screenshot(&sidecar_session)
                    .await
                    .unwrap_or_default();
                if !screenshot.is_empty() && Self::app_screenshot_has_visual_signal(&screenshot) {
                    if let Ok(url) = self
                        .persist_app_preview_screenshot(app_id, &screenshot)
                        .await
                    {
                        screenshot_url = Some(url);
                    }
                }
                diagnostics.push(serde_json::json!({
                    "kind": "navigation",
                    "severity": "info",
                    "message": "Browser quality pass completed navigation.",
                    "url": final_url,
                    "title": title,
                }));
                rendered_content = Some(content);
                Ok(())
            }
            .await;
            let _ = integration.close_session(&sidecar_session).await;
            result
        }
        .await;

        if let Err(error) = browser_result {
            diagnostics.push(serde_json::json!({
                "kind": "browser",
                "severity": "warning",
                "message": safe_truncate(&error.to_string(), 500),
            }));
        }

        let mut browser_ok = false;
        let mut runtime_marker: Option<String> = None;
        if let Some(content) = rendered_content.as_ref() {
            let combined = format!("{}\n{}", content.title, content.body_text).to_lowercase();
            runtime_marker = Self::detect_app_runtime_error_marker(&combined).map(str::to_string);
            browser_ok =
                Self::rendered_app_content_has_dom_signal(content) && runtime_marker.is_none();
            for entry in content.diagnostics.iter().take(60) {
                diagnostics.push(serde_json::json!({
                    "kind": &entry.kind,
                    "severity": &entry.severity,
                    "message": safe_truncate(&entry.message, 320),
                    "url": safe_truncate(&entry.url, 260),
                    "resource_type": &entry.resource_type,
                }));
            }
        }

        let (judge_passed, judge_detail) = match rendered_content.as_ref() {
            Some(content) => self
                .validate_rendered_app_against_request(request_context, content, None)
                .await
                .unwrap_or_else(|error| {
                    (
                        false,
                        format!("Advisory model quality check failed: {}", error),
                    )
                }),
            None => (
                false,
                "Browser content was not available for advisory model quality check.".to_string(),
            ),
        };

        let coverage = match rendered_content.as_ref() {
            Some(content) => self.build_app_coverage_report(sub_goals, content).await,
            None => serde_json::json!({
                "status": "unavailable",
                "reason": "Browser content was not available.",
                "items": [],
            }),
        };
        let coverage_missing = coverage
            .get("missing")
            .and_then(|value| value.as_u64())
            .unwrap_or(0);
        let coverage_total = coverage
            .get("total")
            .and_then(|value| value.as_u64())
            .unwrap_or(0);
        let mut concerns = Vec::<String>::new();
        if !browser_ok {
            concerns.push(
                "The background browser pass did not prove a usable rendered page.".to_string(),
            );
        }
        if let Some(marker) = runtime_marker.as_deref() {
            concerns.push(format!("Rendered page reported runtime marker: {}", marker));
        }
        if !judge_passed {
            concerns.push(safe_truncate(&judge_detail, 700));
        }
        if coverage_missing > 0 {
            concerns.push(format!(
                "{} of {} requested items were not clearly evidenced in the rendered page.",
                coverage_missing, coverage_total
            ));
        }
        let status = if rendered_content.is_none() {
            "error"
        } else if concerns.is_empty() {
            "passed"
        } else {
            "concerns"
        };

        serde_json::json!({
            "schema_version": 1,
            "status": status,
            "app_id": app_id,
            "app_type": app_type,
            "app_updated_at": app_updated_at,
            "started_at": started_at,
            "completed_at": chrono::Utc::now().to_rfc3339(),
            "advisory_only": true,
            "control_effect": "none",
            "browser_url": browser_url,
            "browser_ok": browser_ok,
            "screenshot_url": screenshot_url,
            "judge": {
                "passed": judge_passed,
                "summary": safe_truncate(&judge_detail, 900),
            },
            "judge_concerns": concerns,
            "coverage": coverage,
            "diagnostics": diagnostics,
        })
    }

    async fn build_app_coverage_report(
        &self,
        sub_goals: &[String],
        rendered: &crate::integrations::browser::PageContent,
    ) -> serde_json::Value {
        let goals = sub_goals
            .iter()
            .map(|goal| goal.trim())
            .filter(|goal| !goal.is_empty())
            .take(20)
            .map(ToString::to_string)
            .collect::<Vec<_>>();
        if goals.is_empty() {
            return serde_json::json!({
                "status": "not_requested",
                "total": 0,
                "covered": 0,
                "missing": 0,
                "items": [],
            });
        }
        let Some(embedder) = self.embedding_client.as_ref().cloned() else {
            let items = goals
                .iter()
                .enumerate()
                .map(|(idx, goal)| {
                    serde_json::json!({
                        "id": format!("goal_{}", idx + 1),
                        "summary": safe_truncate(goal, 300),
                        "covered": serde_json::Value::Null,
                        "score": serde_json::Value::Null,
                    })
                })
                .collect::<Vec<_>>();
            return serde_json::json!({
                "status": "embedding_unavailable",
                "total": goals.len(),
                "covered": 0,
                "missing": goals.len(),
                "items": items,
            });
        };
        let rendered_text = Self::coverage_rendered_text(rendered);
        if rendered_text.trim().is_empty() {
            return serde_json::json!({
                "status": "no_rendered_text",
                "total": goals.len(),
                "covered": 0,
                "missing": goals.len(),
                "items": [],
            });
        }
        let mut texts = Vec::with_capacity(goals.len() + 1);
        texts.push(rendered_text);
        texts.extend(goals.iter().cloned());
        let embeddings = match embedder.embed_texts(&texts).await {
            Ok(values) if values.len() == texts.len() => values,
            Ok(_) => {
                return serde_json::json!({
                    "status": "embedding_mismatch",
                    "total": goals.len(),
                    "covered": 0,
                    "missing": goals.len(),
                    "items": [],
                });
            }
            Err(error) => {
                return serde_json::json!({
                    "status": "embedding_error",
                    "reason": safe_truncate(&error.to_string(), 300),
                    "total": goals.len(),
                    "covered": 0,
                    "missing": goals.len(),
                    "items": [],
                });
            }
        };
        let page_embedding = &embeddings[0];
        let mut covered_count = 0usize;
        let items = goals
            .iter()
            .enumerate()
            .map(|(idx, goal)| {
                let score = crate::core::document_search::normalized_embedding_similarity(
                    page_embedding.as_slice(),
                    embeddings[idx + 1].as_slice(),
                )
                .unwrap_or(0.0);
                let covered = score >= APP_QUALITY_COVERAGE_THRESHOLD;
                if covered {
                    covered_count += 1;
                }
                serde_json::json!({
                    "id": format!("goal_{}", idx + 1),
                    "summary": safe_truncate(goal, 300),
                    "covered": covered,
                    "score": ((score * 1000.0).round() / 1000.0),
                })
            })
            .collect::<Vec<_>>();
        serde_json::json!({
            "status": if covered_count == goals.len() { "passed" } else { "partial" },
            "threshold": APP_QUALITY_COVERAGE_THRESHOLD,
            "total": goals.len(),
            "covered": covered_count,
            "missing": goals.len().saturating_sub(covered_count),
            "items": items,
        })
    }

    fn coverage_rendered_text(rendered: &crate::integrations::browser::PageContent) -> String {
        let mut text = String::new();
        if !rendered.title.trim().is_empty() {
            text.push_str(&rendered.title);
            text.push('\n');
        }
        if !rendered.body_text.trim().is_empty() {
            text.push_str(&safe_truncate(&rendered.body_text, 8000));
            text.push('\n');
        }
        for element in rendered.elements.iter().take(80) {
            for value in [&element.text, &element.name, &element.id, &element.href] {
                let trimmed = value.trim();
                if !trimmed.is_empty() {
                    text.push_str(&safe_truncate(trimmed, 160));
                    text.push('\n');
                }
            }
        }
        safe_truncate(&text, 12000)
    }

    async fn append_moltbook_tool_activity(
        &self,
        sub_action: &str,
        args: &serde_json::Value,
        result: Option<&serde_json::Value>,
        error: Option<&str>,
    ) {
        let mut events: Vec<serde_json::Value> = self
            .storage
            .get(MOLTBOOK_ACTIVITY_LOG_KEY)
            .await
            .ok()
            .flatten()
            .and_then(|raw| serde_json::from_slice::<Vec<serde_json::Value>>(&raw).ok())
            .unwrap_or_default();

        let urls = collect_moltbook_urls(sub_action, args, result);
        let action_kind = moltbook_action_kind(sub_action);

        let mut details = serde_json::json!({
            "source": "tool_call",
            "sub_action": sub_action,
            "action_kind": action_kind,
            "urls": urls
        });
        if let Some(post_id) = args.get("post_id").and_then(|v| v.as_str()) {
            details["post_id"] = serde_json::Value::String(post_id.to_string());
        }
        if let Some(submolt) = args.get("submolt").and_then(|v| v.as_str()) {
            details["submolt"] = serde_json::Value::String(submolt.to_string());
        }
        if let Some(query) = args.get("query").and_then(|v| v.as_str()) {
            details["query_preview"] = serde_json::Value::String(safe_truncate(query, 120));
        }
        if let Some(content) = args.get("content").and_then(|v| v.as_str()) {
            details["content_chars"] = serde_json::Value::from(content.chars().count() as u64);
            details["content_preview"] = serde_json::Value::String(safe_truncate(content, 220));
        }
        if let Some(title) = args.get("title").and_then(|v| v.as_str()) {
            details["title_preview"] = serde_json::Value::String(safe_truncate(title, 120));
        }
        if let Some(err) = error {
            details["error"] = serde_json::Value::String(safe_truncate(err, 300));
        }
        if let Some(post_id) = result
            .and_then(|r| r.get("post"))
            .and_then(|p| p.get("id"))
            .and_then(|v| v.as_str())
        {
            details["result_post_id"] = serde_json::Value::String(post_id.to_string());
        }

        events.push(serde_json::json!({
            "id": uuid::Uuid::new_v4().to_string(),
            "run_id": uuid::Uuid::new_v4().to_string(),
            "timestamp": chrono::Utc::now().to_rfc3339(),
            "level": if error.is_some() { "error" } else { "info" },
            "action": format!("tool_{}", sub_action),
            "details": details
        }));
        if events.len() > MOLTBOOK_ACTIVITY_LOG_LIMIT {
            let drop = events.len() - MOLTBOOK_ACTIVITY_LOG_LIMIT;
            events.drain(0..drop);
        }
        if let Ok(bytes) = serde_json::to_vec(&events) {
            let _ = self.storage.set(MOLTBOOK_ACTIVITY_LOG_KEY, &bytes).await;
        }
    }

    async fn fire_action_hook(
        &self,
        trigger: crate::hooks::HookTrigger,
        channel: &str,
        action_name: &str,
        message_hint: Option<&str>,
        response: Option<&str>,
        event_id: &str,
    ) {
        self.hooks
            .fire(
                trigger.clone(),
                crate::hooks::HookContext {
                    event_id: Some(event_id.to_string()),
                    trigger: match trigger {
                        crate::hooks::HookTrigger::PreMessage => "pre_message".to_string(),
                        crate::hooks::HookTrigger::PostMessage => "post_message".to_string(),
                        crate::hooks::HookTrigger::PreAction => "pre_action".to_string(),
                        crate::hooks::HookTrigger::PostAction => "post_action".to_string(),
                        crate::hooks::HookTrigger::OnConsolidate => "on_consolidate".to_string(),
                        crate::hooks::HookTrigger::OnError => "on_error".to_string(),
                    },
                    channel: channel.to_string(),
                    message: message_hint
                        .map(|m| self.sanitize_stream_preview(&safe_truncate(m, 500))),
                    response: response
                        .map(|r| self.sanitize_stream_preview(&safe_truncate(r, 1500))),
                    action: Some(action_name.to_string()),
                    timestamp: chrono::Utc::now().to_rfc3339(),
                },
            )
            .await;
    }

    async fn execute_direct_list_watchers_tool(
        &self,
        arguments: &serde_json::Value,
    ) -> Result<String> {
        let filter = arguments
            .get("filter")
            .and_then(|value| value.as_str())
            .map(|value| value.trim().to_ascii_lowercase())
            .filter(|value| !value.is_empty())
            .unwrap_or_else(|| "active".to_string());
        let limit = arguments
            .get("limit")
            .and_then(|value| value.as_u64())
            .unwrap_or(20)
            .clamp(1, 100) as usize;

        let (mut watchers, supervisor_states) = tokio::join!(
            self.watcher_manager.list(),
            crate::core::list_automation_supervisor_states(&self.storage)
        );
        watchers.sort_by(|left, right| right.created_at.cmp(&left.created_at));
        let live_ids = watchers
            .iter()
            .map(|watcher| watcher.id.to_string())
            .collect::<std::collections::HashSet<_>>();

        let mut rows = watchers
            .iter()
            .map(list_watchers_live_row)
            .collect::<Vec<_>>();
        rows.extend(
            supervisor_states
                .unwrap_or_default()
                .into_iter()
                .filter(|state| {
                    state.automation_kind == "watcher" && !live_ids.contains(&state.automation_id)
                })
                .map(list_watchers_history_row),
        );
        rows.retain(|row| list_watchers_row_matches_filter(row, &filter));
        rows.sort_by(|left, right| {
            let left_created = left
                .get("created_at")
                .and_then(|value| value.as_str())
                .unwrap_or_default();
            let right_created = right
                .get("created_at")
                .and_then(|value| value.as_str())
                .unwrap_or_default();
            right_created.cmp(left_created)
        });
        if rows.len() > limit {
            rows.truncate(limit);
        }

        if rows.is_empty() {
            return Ok(format!("No {} watcher(s) found.", filter));
        }

        Ok(serde_json::to_string_pretty(&serde_json::json!({
            "filter": filter,
            "count": rows.len(),
            "watchers": rows,
        }))?)
    }

    pub(crate) async fn execute_action_with_hooks(
        &self,
        action_name: &str,
        arguments: &serde_json::Value,
        channel: &str,
        message_hint: Option<&str>,
        authorization: Option<&crate::actions::ActionAuthorizationContext>,
    ) -> Result<String> {
        let event_id = uuid::Uuid::new_v4().to_string();
        self.fire_action_hook(
            crate::hooks::HookTrigger::PreAction,
            channel,
            action_name,
            message_hint,
            None,
            &event_id,
        )
        .await;

        let execution = if action_name.eq_ignore_ascii_case("notify_user")
            && notification_tool_should_dispatch_for_surface(channel, authorization)
        {
            self.execute_direct_notify_user_tool(arguments).await
        } else if action_name.eq_ignore_ascii_case("list_watchers") {
            self.execute_direct_list_watchers_tool(arguments).await
        } else if let Some(auth_context) = authorization {
            self.runtime
                .execute_action_with_context(action_name, arguments, auth_context)
                .await
        } else {
            self.runtime.execute_action(action_name, arguments).await
        };

        match execution {
            Ok(result) => {
                self.fire_action_hook(
                    crate::hooks::HookTrigger::PostAction,
                    channel,
                    action_name,
                    message_hint,
                    Some(&result),
                    &event_id,
                )
                .await;
                Ok(result)
            }
            Err(e) => {
                let err_text =
                    crate::actions::ensure_structured_action_error_text(action_name, e.to_string());
                let typed_error = typed_tool_error_fields(&e);
                if let Some(typed_error) = &typed_error {
                    tracing::debug!(
                        action = action_name,
                        channel,
                        error_code = typed_error.code.as_str(),
                        error_scope = typed_error.scope.as_deref().unwrap_or(""),
                        error_detail = typed_error.detail.as_deref().unwrap_or(""),
                        "action failed with typed error"
                    );
                }
                self.fire_action_hook(
                    crate::hooks::HookTrigger::OnError,
                    channel,
                    action_name,
                    message_hint,
                    Some(&err_text),
                    &event_id,
                )
                .await;
                if typed_error.is_some() {
                    Err(e)
                } else if let Some(error) =
                    crate::actions::parse_structured_action_error_text(&err_text)
                {
                    Err(error.into_anyhow())
                } else {
                    Err(crate::actions::structured_action_error_for_action(
                        action_name,
                        crate::actions::ActionErrorReason::Failed,
                        err_text,
                    ))
                }
            }
        }
    }

    fn sanitize_stream_preview(&self, text: &str) -> String {
        let result = crate::security::sanitize_model_input_text(
            text,
            &self.config.model_privacy,
            crate::security::ModelInputContext::Diagnostic,
            false,
        );
        safe_truncate(
            &crate::security::render_model_input_fallback(
                &result,
                crate::security::ModelInputContext::Diagnostic,
            ),
            300,
        )
    }

    async fn load_public_base_url(&self) -> Option<String> {
        let config_base = self
            .config
            .public_apps
            .base_url
            .as_deref()
            .map(str::trim)
            .filter(|value| !value.is_empty())
            .map(|value| value.trim_end_matches('/').to_string());
        if config_base.is_some() {
            return config_base;
        }
        if self.config.deployment_mode == crate::core::config::DeploymentMode::InternetFacing {
            if let Some(bind_addr) = self
                .config
                .public_apps
                .bind_addr
                .as_deref()
                .map(str::trim)
                .filter(|value| !value.is_empty())
            {
                let normalized = if bind_addr.starts_with("0.0.0.0:") {
                    format!("localhost:{}", bind_addr.trim_start_matches("0.0.0.0:"))
                } else if bind_addr == "0.0.0.0" {
                    "localhost".to_string()
                } else if bind_addr.starts_with("[::]:") {
                    format!("localhost:{}", bind_addr.trim_start_matches("[::]:"))
                } else if bind_addr == "[::]" || bind_addr == "::" {
                    "localhost".to_string()
                } else if bind_addr.starts_with("127.0.0.1:") || bind_addr == "127.0.0.1" {
                    bind_addr.replacen("127.0.0.1", "localhost", 1)
                } else {
                    bind_addr.to_string()
                };
                return Some(format!("http://{}", normalized.trim_end_matches('/')));
            }
        }

        self.storage
            .get("public_base_url")
            .await
            .ok()
            .flatten()
            .and_then(|bytes| String::from_utf8(bytes).ok())
            .map(|s| s.trim().trim_end_matches('/').to_string())
            .filter(|s| !s.is_empty())
            .or_else(|| {
                std::env::var("AGENTARK_PUBLIC_BASE_URL")
                    .ok()
                    .map(|s| s.trim().trim_end_matches('/').to_string())
                    .filter(|s| !s.is_empty())
            })
    }

    fn has_configured_public_base_url(&self) -> bool {
        self.config
            .public_apps
            .base_url
            .as_deref()
            .map(str::trim)
            .is_some_and(|value| !value.is_empty())
            || (self.config.deployment_mode == crate::core::config::DeploymentMode::InternetFacing
                && self
                    .config
                    .public_apps
                    .bind_addr
                    .as_deref()
                    .map(str::trim)
                    .is_some_and(|value| !value.is_empty()))
    }

    fn internal_api_base_url() -> String {
        crate::core::net::internal_api_base_url()
    }

    fn user_facing_local_base_url() -> String {
        let internal = Self::internal_api_base_url();
        let Ok(mut parsed) = reqwest::Url::parse(&internal) else {
            return internal;
        };
        if let Some(host) = parsed.host_str() {
            let normalized = host.trim().to_ascii_lowercase();
            if normalized == "0.0.0.0" || normalized == "::" || normalized == "127.0.0.1" {
                let _ = parsed.set_host(Some("localhost"));
            }
        }
        parsed.to_string().trim_end_matches('/').to_string()
    }

    fn url_path_and_query(url: &reqwest::Url) -> String {
        let mut path = url.path().to_string();
        if path.is_empty() {
            path.push('/');
        }
        if let Some(query) = url.query() {
            path.push('?');
            path.push_str(query);
        }
        path
    }

    fn url_host_is_local_or_wildcard(host: &str) -> bool {
        let normalized = host.trim().trim_end_matches('.').to_ascii_lowercase();
        normalized == "localhost"
            || normalized.ends_with(".localhost")
            || normalized == "0.0.0.0"
            || normalized == "::"
            || normalized == "[::]"
            || normalized
                .parse::<std::net::IpAddr>()
                .map(|ip| ip.is_loopback() || ip.is_unspecified())
                .unwrap_or(false)
    }

    fn app_url_points_to_internal_local_bind(url: &reqwest::Url) -> bool {
        if !url.path().starts_with("/apps/") && url.path() != "/apps" {
            return false;
        }
        let Ok(internal) = reqwest::Url::parse(&Self::internal_api_base_url()) else {
            return false;
        };
        if url.scheme() != internal.scheme()
            || url.port_or_known_default() != internal.port_or_known_default()
        {
            return false;
        }
        let Some(target_host) = url.host_str() else {
            return false;
        };
        let Some(internal_host) = internal.host_str() else {
            return false;
        };
        target_host.eq_ignore_ascii_case(internal_host)
            || (Self::url_host_is_local_or_wildcard(target_host)
                && Self::url_host_is_local_or_wildcard(internal_host))
    }

    fn browser_validation_app_url(app_url_with_key: &str) -> String {
        let raw = app_url_with_key.trim();
        let local_base = Self::user_facing_local_base_url();
        if let Ok(parsed) = reqwest::Url::parse(raw) {
            if Self::app_url_points_to_internal_local_bind(&parsed) {
                return Self::absolutize_public_url(
                    Some(local_base.as_str()),
                    &Self::url_path_and_query(&parsed),
                );
            }
            return raw.to_string();
        }
        Self::absolutize_public_url(Some(local_base.as_str()), raw)
    }

    fn build_internal_control_client() -> Result<reqwest::Client> {
        crate::core::net::build_internal_control_client(5)
    }

    async fn ensure_public_tunnel_base_url(
        &self,
        app_id: Option<&str>,
        stream_tx: Option<&tokio::sync::mpsc::Sender<StreamEvent>>,
    ) -> Option<String> {
        let cached_public_base_url = self.load_public_base_url().await;
        if let Some(existing) = cached_public_base_url.as_deref() {
            if self.has_configured_public_base_url() {
                return Some(existing.to_string());
            }
        }
        let client = match Self::build_internal_control_client() {
            Ok(c) => c,
            Err(e) => {
                tracing::debug!("Tunnel client init failed: {}", e);
                return None;
            }
        };
        let base_url = Self::internal_api_base_url();

        let mut start_req = client.post(format!("{}/tunnel/start", base_url));
        if let Some(key) = self.api_key.as_ref().filter(|k| !k.trim().is_empty()) {
            start_req = start_req.bearer_auth(key);
        }
        let requested_app_id = app_id
            .map(str::trim)
            .filter(|value| !value.is_empty())
            .map(|value| value.to_string());
        let start_payload = match requested_app_id.as_deref() {
            Some(app_id) => serde_json::json!({ "app_id": app_id }),
            None => serde_json::json!({}),
        };
        start_req = start_req.json(&start_payload);
        let start_accepted = match start_req.send().await {
            Ok(resp) => {
                if !resp.status().is_success() {
                    tracing::debug!("Tunnel start request returned {}", resp.status());
                    false
                } else {
                    true
                }
            }
            Err(e) => {
                tracing::debug!("Tunnel start request failed: {}", e);
                return None;
            }
        };

        if start_accepted {
            if let Some(tx) = stream_tx {
                let detail = "Starting remote access for app sharing...".to_string();
                queue_stream_event(
                    tx,
                    StreamEvent::ToolProgress {
                        name: "app_deploy".to_string(),
                        content: detail.clone(),
                        payload: Some(phase_status_payload(
                            "app_deploy",
                            "link_setup",
                            "Link setup",
                            &detail,
                            0,
                        )),
                    },
                );
            }
        } else {
            return None;
        }

        for _ in 0..10 {
            let mut status_req = client.get(format!("{}/tunnel/status", base_url));
            if let Some(key) = self.api_key.as_ref().filter(|k| !k.trim().is_empty()) {
                status_req = status_req.bearer_auth(key);
            }
            if let Ok(resp) = status_req.send().await {
                if resp.status().is_success() {
                    if let Ok(payload) = resp.json::<serde_json::Value>().await {
                        let selected_app_matches = match requested_app_id.as_deref() {
                            Some(app_id) => {
                                let selected_matches = payload
                                    .get("selected_app_id")
                                    .and_then(|v| v.as_str())
                                    .map(str::trim)
                                    == Some(app_id);
                                let exposed_matches = payload
                                    .get("exposed_app_ids")
                                    .and_then(|v| v.as_array())
                                    .map(|ids| {
                                        ids.iter().any(|value| {
                                            value.as_str().map(str::trim) == Some(app_id)
                                        })
                                    })
                                    .unwrap_or(false);
                                selected_matches || exposed_matches
                            }
                            None => true,
                        };
                        if let Some(url) = payload
                            .get("url")
                            .and_then(|v| v.as_str())
                            .map(|v| v.trim().trim_end_matches('/').to_string())
                            .filter(|v| !v.is_empty())
                        {
                            if !selected_app_matches {
                                tokio::time::sleep(std::time::Duration::from_secs(1)).await;
                                continue;
                            }
                            let _ = self.storage.set("public_base_url", url.as_bytes()).await;
                            return Some(url);
                        }
                    }
                }
            }
            tokio::time::sleep(std::time::Duration::from_secs(1)).await;
        }

        None
    }

    fn trigger_arkpulse_refresh(&self, reason: &'static str) {
        let api_key = self.api_key.clone();
        let base_url = Self::internal_api_base_url();
        crate::spawn_logged!("src/core/agent/tool_execution.rs:3560", async move {
            let client = match crate::core::net::build_internal_control_client(4) {
                Ok(c) => c,
                Err(e) => {
                    tracing::debug!("ArkPulse refresh client init failed: {}", e);
                    return;
                }
            };
            let mut req = client.post(format!("{}/arkpulse/trigger", base_url));
            if let Some(key) = api_key.as_ref().filter(|k| !k.trim().is_empty()) {
                req = req.bearer_auth(key);
            }
            match req.send().await {
                Ok(resp) => {
                    tracing::debug!(
                        "ArkPulse refresh trigger after {} returned {}",
                        reason,
                        resp.status()
                    );
                }
                Err(e) => {
                    tracing::debug!("ArkPulse refresh trigger after {} failed: {}", reason, e);
                }
            }
        });
    }

    fn absolutize_public_url(public_base_url: Option<&str>, url: &str) -> String {
        if url.starts_with("http://")
            || url.starts_with("https://")
            || url.starts_with("data:")
            || url.starts_with("blob:")
        {
            return url.to_string();
        }
        if let Some(base) = public_base_url {
            if url.starts_with('/') {
                return format!("{}{}", base, url);
            }
            return format!("{}/{}", base, url);
        }
        url.to_string()
    }

    fn default_tool_integration_aliases() -> HashMap<String, String> {
        let mut aliases = HashMap::new();
        aliases.insert("github".to_string(), "github".to_string());
        aliases.insert("notion".to_string(), "notion".to_string());
        aliases.insert("twitter".to_string(), "twitter".to_string());
        aliases.insert("onepassword".to_string(), "onepassword".to_string());
        aliases.insert("places".to_string(), "google_places".to_string());
        aliases.insert("twilio".to_string(), "twilio".to_string());
        aliases.insert("ordering".to_string(), "ordering".to_string());
        aliases.insert("garmin".to_string(), "garmin".to_string());
        aliases.insert("whoop".to_string(), "whoop".to_string());
        aliases.insert("ga4".to_string(), "ga4".to_string());
        aliases.insert("gsc".to_string(), "gsc".to_string());
        aliases.insert(
            "social_analytics".to_string(),
            "social_analytics".to_string(),
        );
        aliases.insert("moltbook".to_string(), "moltbook".to_string());
        aliases
    }

    fn merge_tool_integration_aliases(
        aliases: &mut HashMap<String, String>,
        value: &serde_json::Value,
    ) {
        let Some(obj) = value.as_object() else {
            return;
        };
        for (tool_name, integration_id_value) in obj {
            let Some(integration_id) = integration_id_value.as_str() else {
                continue;
            };
            let tool_name = tool_name.trim();
            let integration_id = integration_id.trim();
            if tool_name.is_empty() || integration_id.is_empty() {
                continue;
            }
            aliases.insert(tool_name.to_string(), integration_id.to_string());
        }
    }

    async fn load_tool_integration_aliases(&self) -> HashMap<String, String> {
        let mut aliases = Self::default_tool_integration_aliases();

        if let Ok(raw_env) = std::env::var("AGENTARK_TOOL_INTEGRATION_ALIASES") {
            if let Ok(value) = serde_json::from_str::<serde_json::Value>(&raw_env) {
                Self::merge_tool_integration_aliases(&mut aliases, &value);
            } else {
                tracing::warn!("Invalid AGENTARK_TOOL_INTEGRATION_ALIASES JSON ignored");
            }
        }

        if let Ok(Some(raw)) = self.storage.get(TOOL_INTEGRATION_ALIASES_KEY).await {
            if let Ok(value) = serde_json::from_slice::<serde_json::Value>(&raw) {
                Self::merge_tool_integration_aliases(&mut aliases, &value);
            } else {
                tracing::warn!("Invalid '{}' JSON ignored", TOOL_INTEGRATION_ALIASES_KEY);
            }
        }

        let enabled_ids: HashSet<String> =
            self.integrations.ready_ids().await.into_iter().collect();
        for integration_id in &enabled_ids {
            aliases
                .entry(integration_id.clone())
                .or_insert_with(|| integration_id.clone());
        }
        aliases.retain(|_, integration_id| enabled_ids.contains(integration_id));

        aliases
    }

    pub(crate) fn resolve_tool_integration_id(
        &self,
        tool_name: &str,
        aliases: &HashMap<String, String>,
    ) -> Option<String> {
        aliases.get(tool_name).cloned()
    }

    pub(crate) async fn execute_integration_tool_call(
        &self,
        call: &crate::core::llm::ToolCall,
        trace_ref: &Arc<RwLock<ExecutionTrace>>,
        stream_tx: Option<&tokio::sync::mpsc::Sender<StreamEvent>>,
        request_channel: &str,
        integration_id: &str,
    ) -> String {
        let sub_action = call
            .arguments
            .get("action")
            .and_then(|v| v.as_str())
            .unwrap_or("default");
        let resolved_args = self
            .runtime
            .resolve_secret_placeholders(&call.name, &call.arguments)
            .unwrap_or_else(|_| call.arguments.clone());
        let hook_event_id = uuid::Uuid::new_v4().to_string();
        let hook_hint = action_message_hint(&resolved_args);
        self.fire_action_hook(
            crate::hooks::HookTrigger::PreAction,
            request_channel,
            &call.name,
            hook_hint.as_deref(),
            None,
            &hook_event_id,
        )
        .await;

        match self
            .integrations
            .execute(integration_id, sub_action, &resolved_args)
            .await
        {
            Ok(result) => {
                if integration_id == "moltbook" {
                    self.append_moltbook_tool_activity(
                        sub_action,
                        &resolved_args,
                        Some(&result),
                        None,
                    )
                    .await;
                    let (title, detail, step_type, data) = build_moltbook_trace_result_step(
                        sub_action,
                        &resolved_args,
                        Some(&result),
                        None,
                    );
                    trace_ref.write().await.steps.push(ExecutionStep {
                        icon: "[ok]".to_string(),
                        title,
                        detail,
                        step_type,
                        data,
                        timestamp: chrono::Utc::now(),
                        duration_ms: None,
                    });
                }
                let formatted =
                    serde_json::to_string_pretty(&result).unwrap_or_else(|_| result.to_string());
                self.fire_action_hook(
                    crate::hooks::HookTrigger::PostAction,
                    request_channel,
                    &call.name,
                    hook_hint.as_deref(),
                    Some(&formatted),
                    &hook_event_id,
                )
                .await;
                if let Some(tx) = stream_tx {
                    queue_stream_event(
                        tx,
                        StreamEvent::ToolResult {
                            name: call.name.clone(),
                            content: self.sanitize_stream_preview(&formatted),
                        },
                    );
                }
                formatted
            }
            Err(e) => {
                if integration_id == "moltbook" {
                    self.append_moltbook_tool_activity(
                        sub_action,
                        &resolved_args,
                        None,
                        Some(&e.to_string()),
                    )
                    .await;
                    let error_text = e.to_string();
                    let (title, detail, step_type, data) = build_moltbook_trace_result_step(
                        sub_action,
                        &resolved_args,
                        None,
                        Some(&error_text),
                    );
                    trace_ref.write().await.steps.push(ExecutionStep {
                        icon: "[warn]".to_string(),
                        title,
                        detail,
                        step_type,
                        data,
                        timestamp: chrono::Utc::now(),
                        duration_ms: None,
                    });
                }
                tracing::error!("{} integration error: {}", call.name, e);
                self.fire_action_hook(
                    crate::hooks::HookTrigger::OnError,
                    request_channel,
                    &call.name,
                    hook_hint.as_deref(),
                    Some(&e.to_string()),
                    &hook_event_id,
                )
                .await;
                let formatted = format!("Error from {}: {}", call.name, e);
                if let Some(tx) = stream_tx {
                    queue_stream_event(
                        tx,
                        StreamEvent::ToolResult {
                            name: call.name.clone(),
                            content: formatted.clone(),
                        },
                    );
                }
                formatted
            }
        }
    }

    fn integration_capability_labels(caps: Vec<crate::integrations::Capability>) -> Vec<String> {
        caps.into_iter()
            .map(|cap| match cap {
                crate::integrations::Capability::Read => "read".to_string(),
                crate::integrations::Capability::Write => "write".to_string(),
                crate::integrations::Capability::Subscribe => "subscribe".to_string(),
                crate::integrations::Capability::Search => "search".to_string(),
                crate::integrations::Capability::Delete => "delete".to_string(),
                crate::integrations::Capability::Notify => "notify".to_string(),
            })
            .collect()
    }

    fn browser_integration_action_schema() -> serde_json::Value {
        serde_json::json!({
            "type": "object",
            "properties": {
                "action": {
                    "type": "string",
                    "enum": [
                        "create_session",
                        "navigate",
                        "screenshot",
                        "click",
                        "type_text",
                        "scroll",
                        "get_content",
                        "close_session"
                    ],
                    "description": "Browser operation to execute. Use only the listed actions."
                },
                "session_id": {
                    "type": "string",
                    "description": "Existing browser session id. Optional when AgentArk can reuse the current chat's live browser session."
                },
                "url": {
                    "type": "string",
                    "description": "Public http/https URL for `navigate`."
                },
                "selector": {
                    "type": "string",
                    "description": "CSS selector for `click` or `type_text`."
                },
                "text": {
                    "type": "string",
                    "description": "Text to type, or visible text target for `click`."
                },
                "clear": {
                    "type": "boolean",
                    "description": "Whether `type_text` should clear the target first."
                },
                "x": {
                    "type": "integer",
                    "description": "Optional x coordinate for `click`."
                },
                "y": {
                    "type": "integer",
                    "description": "Optional y coordinate for `click`."
                },
                "direction": {
                    "type": "string",
                    "enum": ["up", "down"],
                    "description": "Scroll direction for `scroll`."
                },
                "amount": {
                    "type": "integer",
                    "description": "Optional scroll amount for `scroll`."
                }
            },
            "required": ["action"],
            "additionalProperties": true
        })
    }

    fn build_integration_action_def(
        &self,
        tool_name: &str,
        integration_id: &str,
        integration: &dyn crate::integrations::Integration,
    ) -> crate::actions::ActionDef {
        if integration_id == "vercel" {
            return crate::actions::ActionDef {
                name: tool_name.to_string(),
                description: "Vercel is available as an app deployment provider. Use app_deploy with deploy_target=\"vercel_direct\" or the protected Apps publish endpoint; do not pass Vercel tokens in tool arguments.".to_string(),
                version: "1.0.0".to_string(),
                input_schema: serde_json::json!({
                    "type": "object",
                    "properties": {},
                    "additionalProperties": false
                }),
                capabilities: vec!["app_hosting".to_string(), "write".to_string()],
                sandbox_mode: None,
                source: crate::actions::ActionSource::System,
                file_path: None,
                authorization: crate::actions::ActionAuthorization {
                    access: crate::actions::ActionAccessMetadata {
                        integration_ids: vec![integration_id.to_string()],
                        ..Default::default()
                    },
                    ..Default::default()
                },
            };
        }
        let (capabilities, description, input_schema) = if integration_id == "browser" {
            (
                vec![
                    "browser".to_string(),
                    "read".to_string(),
                    "write".to_string(),
                ],
                format!(
                    "Integration tool '{}' routed to '{}'. {} Use this only for explicit manual browser control such as creating a session, navigating to a known URL, clicking, typing, scrolling, taking a screenshot, or reading page content. Do not use it for general public web search, current events, or news lookups; use built-in `web_search`, `research`, or `browse` instead.",
                    tool_name,
                    integration_id,
                    integration.description()
                ),
                Self::browser_integration_action_schema(),
            )
        } else {
            let capabilities = Self::integration_capability_labels(integration.capabilities());
            let search_routing_hint = if capabilities.iter().any(|cap| cap == "search") {
                " Prefer this connector only when the user explicitly wants this service, account, community, repository, or workspace. For general public web facts, current events, or news, prefer built-in `web_search` or `research`."
            } else {
                ""
            };
            (
                capabilities,
                format!(
                    "Integration tool '{}' routed to '{}'. {}{} Pass an 'action' field and any connector-specific parameters.",
                    tool_name,
                    integration_id,
                    integration.description(),
                    search_routing_hint
                ),
                serde_json::json!({
                    "type": "object",
                    "properties": {
                        "action": {
                            "type": "string",
                            "description": "Connector operation to execute"
                        }
                    },
                    "additionalProperties": true
                }),
            )
        };
        crate::actions::ActionDef {
            name: tool_name.to_string(),
            description,
            version: "1.0.0".to_string(),
            input_schema,
            capabilities,
            sandbox_mode: None,
            source: crate::actions::ActionSource::System,
            file_path: None,
            authorization: crate::actions::ActionAuthorization {
                access: crate::actions::ActionAccessMetadata {
                    integration_ids: vec![integration_id.to_string()],
                    ..Default::default()
                },
                ..Default::default()
            },
        }
    }

    pub(crate) async fn append_dynamic_integration_actions(
        &self,
        actions: &mut Vec<crate::actions::ActionDef>,
    ) {
        let mut existing: HashSet<String> = actions.iter().map(|a| a.name.clone()).collect();
        let mut covered_integration_ids: HashSet<String> = actions
            .iter()
            .flat_map(|action| action.authorization.access.integration_ids.clone())
            .collect();
        let integration_aliases = self.load_tool_integration_aliases().await;
        let mut ready_ids = HashSet::new();
        for integration_id in integration_aliases.values() {
            if self.integrations.is_ready(integration_id).await {
                ready_ids.insert(integration_id.clone());
            }
        }

        for integration_id in &ready_ids {
            let Some(integration) = self.integrations.get(integration_id) else {
                continue;
            };
            if covered_integration_ids.contains(integration_id) {
                continue;
            }
            if existing.insert(integration_id.to_string()) {
                actions.push(self.build_integration_action_def(
                    integration_id,
                    integration_id,
                    integration,
                ));
                covered_integration_ids.insert(integration_id.to_string());
            }
        }

        for (tool_name, integration_id) in integration_aliases {
            if !ready_ids.contains(&integration_id) {
                continue;
            }
            if !existing.insert(tool_name.clone()) {
                continue;
            }
            let Some(integration) = self.integrations.get(&integration_id) else {
                continue;
            };
            actions.push(self.build_integration_action_def(
                &tool_name,
                &integration_id,
                integration,
            ));
        }
    }

    async fn legacy_tool_call_allowed_by_safety(
        safety: &crate::safety::SafetyEngine,
        call: &crate::core::llm::ToolCall,
        authorization: Option<&crate::actions::ActionAuthorizationContext>,
    ) -> Result<bool> {
        safety
            .is_allowed_with_authorization(&call.name, &call.arguments, authorization)
            .await
    }

    pub(crate) async fn restart_deployed_app_from_metadata(
        &self,
        app_id: &str,
        title_override: Option<&str>,
    ) -> Result<serde_json::Value> {
        let app_id = app_id.trim();
        if app_id.is_empty() {
            anyhow::bail!("Missing app_id");
        }

        let app_dir = if let Some(path) = self.app_registry.get_dir(app_id).await {
            path
        } else {
            let fallback = self.data_dir.join("apps").join(app_id);
            if !fallback.exists() {
                anyhow::bail!("App '{}' not found", app_id);
            }
            fallback
        };

        let meta_path = app_dir.join(".app_meta.json");
        let mut meta: serde_json::Value = match tokio::fs::read(&meta_path).await {
            Ok(bytes) => serde_json::from_slice(&bytes).unwrap_or_else(|_| serde_json::json!({})),
            Err(_) => serde_json::json!({}),
        };
        if !meta.is_object() {
            meta = serde_json::json!({});
        }

        let override_title = title_override
            .map(str::trim)
            .filter(|value| !value.is_empty())
            .map(ToString::to_string);
        let mut meta_changed = false;
        if let Some(title) = override_title.as_ref() {
            if meta.get("title").and_then(|v| v.as_str()) != Some(title.as_str()) {
                meta["title"] = serde_json::Value::String(title.clone());
                meta_changed = true;
            }
        }
        let title = override_title.clone().unwrap_or_else(|| {
            meta.get("title")
                .and_then(|v| v.as_str())
                .unwrap_or(app_id)
                .to_string()
        });
        let entry_command = meta
            .get("entry_command")
            .and_then(|v| v.as_str())
            .map(|s| s.to_string());
        let install_command = meta
            .get("install_command")
            .and_then(|v| v.as_str())
            .map(|s| s.to_string());
        let runtime_image = meta
            .get("runtime_image")
            .and_then(|v| v.as_str())
            .map(|s| s.to_string());
        let runtime_preference = crate::actions::app::runtime_preference_from_opt(
            meta.get("runtime_preference").and_then(|v| v.as_str()),
        );
        let required_inputs = crate::actions::app::parse_required_inputs(&meta);
        let config_values: std::collections::HashMap<String, String> = meta
            .get("config_values")
            .and_then(|v| v.as_object())
            .map(|obj| {
                obj.iter()
                    .filter_map(|(k, v)| {
                        let value = match v {
                            serde_json::Value::String(s) => s.clone(),
                            serde_json::Value::Bool(b) => b.to_string(),
                            serde_json::Value::Number(n) => n.to_string(),
                            _ => return None,
                        };
                        Some((k.clone(), value))
                    })
                    .collect()
            })
            .unwrap_or_default();
        let expose_public = meta
            .get("expose_public")
            .and_then(|v| v.as_bool())
            .unwrap_or(false);
        let access_guard_enabled = meta
            .get("access_guard_enabled")
            .and_then(|v| v.as_bool())
            .unwrap_or(false);
        let public_access_guard_enabled = access_guard_enabled || expose_public;
        let access_key = if public_access_guard_enabled {
            self.app_registry
                .access_key(app_id)
                .await
                .unwrap_or_else(crate::actions::app::generate_access_key)
        } else {
            String::new()
        };

        if meta.get("access_guard_enabled").and_then(|v| v.as_bool()) != Some(access_guard_enabled)
            || meta.get("access_key").is_some()
        {
            meta["access_guard_enabled"] = serde_json::Value::Bool(access_guard_enabled);
            if let Some(obj) = meta.as_object_mut() {
                obj.remove("access_key");
            }
            meta_changed = true;
        }
        if meta.get("enabled").and_then(|v| v.as_bool()) != Some(true) {
            meta["enabled"] = serde_json::Value::Bool(true);
            meta_changed = true;
        }
        if meta_changed {
            let _ = tokio::fs::write(
                &meta_path,
                serde_json::to_vec_pretty(&meta).unwrap_or_default(),
            )
            .await;
        }
        let _ = self.app_registry.set_enabled(app_id, true).await;
        let relative_url = format!("/apps/{}/", app_id);
        let local_base = Self::user_facing_local_base_url();
        let local_url = Self::absolutize_public_url(Some(local_base.as_str()), &relative_url);
        let relative_access_url = if access_guard_enabled {
            self.app_registry
                .issue_access_url(app_id)
                .await
                .unwrap_or_else(|| relative_url.clone())
        } else {
            relative_url.clone()
        };
        let local_access_url =
            Self::absolutize_public_url(Some(local_base.as_str()), &relative_access_url);

        if let Some(executor) = build_executor_client() {
            let response = executor
                .request(
                    reqwest::Method::POST,
                    &format!("/internal/v1/apps/{}/restart", app_id),
                )
                .json(&crate::clients::AppLifecycleRequest {
                    title: override_title.clone(),
                    query: None,
                })
                .send()
                .await?;
            if !response.status().is_success() {
                let payload = response
                    .json::<serde_json::Value>()
                    .await
                    .unwrap_or_else(|_| serde_json::json!({}));
                anyhow::bail!(
                    "{}",
                    payload
                        .get("message")
                        .and_then(|value| value.as_str())
                        .unwrap_or("executor restart failed")
                );
            }
            let payload = response
                .json::<serde_json::Value>()
                .await
                .unwrap_or_else(|_| serde_json::json!({}));
            let raw = payload
                .get("raw")
                .cloned()
                .unwrap_or_else(|| serde_json::json!({}));
            let mode = raw
                .get("mode")
                .and_then(|value| value.as_str())
                .unwrap_or("dynamic");
            return Ok(serde_json::json!({
                "status": "restarted",
                "type": mode,
                "app_id": app_id,
                "title": raw.get("title").and_then(|value| value.as_str()).unwrap_or(title.as_str()),
                "url": relative_url,
                "local_url": local_url,
                "access_url": relative_access_url,
                "local_access_url": local_access_url,
                "access_guard_enabled": access_guard_enabled,
                "public_access_guard_enabled": public_access_guard_enabled,
                "access_key": access_key.clone(),
                "access_password": access_key,
                "expose_public": expose_public,
                "port": raw.get("port").cloned().unwrap_or(serde_json::Value::Null),
                "runtime_preference": raw
                    .get("runtime_mode")
                    .and_then(|value| value.as_str())
                    .unwrap_or("executor"),
                "apps_page_hint": crate::actions::app::APP_DEPLOY_CONTROL_HINT,
            }));
        }

        self.app_registry.stop_runtime(app_id).await?;

        if let Some(entry_command) = entry_command {
            let Some(port) = self.app_registry.find_available_port().await else {
                anyhow::bail!("No available app port");
            };
            let llm_env = self.app_model_env_vars();

            let (resolved_env, missing_sensitive, missing_config) =
                crate::actions::app::resolve_required_env_values(
                    &self.config_dir,
                    &self.data_dir,
                    &required_inputs,
                    &llm_env,
                    &config_values,
                )
                .await?;

            if !missing_sensitive.is_empty() || !missing_config.is_empty() {
                let mut missing_all = missing_sensitive.clone();
                for item in &missing_config {
                    if !missing_all.iter().any(|existing| existing == item) {
                        missing_all.push(item.clone());
                    }
                }
                let required_secret_keys: Vec<String> = required_inputs
                    .iter()
                    .filter(|required| required.sensitive)
                    .map(|required| required.key.clone())
                    .collect();
                let required_config_keys: Vec<String> = required_inputs
                    .iter()
                    .filter(|required| !required.sensitive)
                    .map(|required| required.key.clone())
                    .collect();
                return Ok(serde_json::json!({
                    "status": "needs_secrets",
                    "app_id": app_id,
                    "title": title,
                    "url": relative_url,
                    "local_url": local_url,
                    "missing_env": missing_sensitive,
                    "missing_config": missing_config,
                    "missing_inputs": missing_all,
                    "required_inputs": required_inputs,
                    "required_secrets": required_secret_keys.clone(),
                    "required_env": required_secret_keys,
                    "required_config": required_config_keys,
                    "apps_page_hint": crate::actions::app::APP_DEPLOY_CONTROL_HINT,
                    "message": "Missing required inputs. Use the secure credential form in chat or Settings for sensitive values; provide config for non-sensitive values."
                }));
            }

            let runtime_handle = crate::actions::app::launch_dynamic_runtime(
                crate::actions::app::DynamicRuntimeLaunch {
                    app_id,
                    app_dir: &app_dir,
                    entry_command: &entry_command,
                    install_command: install_command.as_deref(),
                    port,
                    extra_env: &resolved_env,
                    runtime_image: runtime_image.as_deref(),
                    runtime_preference,
                    stream_tx: None,
                },
            )
            .await?;

            let (child, container_id, runtime_label) = match runtime_handle {
                crate::actions::app::DynamicRuntimeHandle::Container(container_id) => {
                    (None, Some(container_id), "container")
                }
                crate::actions::app::DynamicRuntimeHandle::Process(child) => {
                    (Some(*child), None, "local_process")
                }
            };
            let diagnostics_dir = app_dir.clone();

            self.app_registry
                .register_dynamic(
                    app_id.to_string(),
                    crate::actions::app::DynamicAppRegistration {
                        title: title.clone(),
                        app_dir,
                        child,
                        container_id,
                        port,
                        access_key: access_key.clone(),
                        access_guard_enabled,
                        expose_public,
                        enabled: true,
                        last_accessed: None,
                    },
                )
                .await;
            tokio::time::sleep(std::time::Duration::from_secs(2)).await;
            if !self.app_registry.runtime_is_alive(app_id).await {
                let logs =
                    crate::actions::app::read_local_runtime_log_tail(&diagnostics_dir, 4096).await;
                if logs.is_empty() {
                    anyhow::bail!("App process stopped shortly after restart.");
                }
                anyhow::bail!(
                    "App process stopped shortly after restart. Recent runtime logs:\n{}",
                    logs
                );
            }

            return Ok(serde_json::json!({
                "status": "restarted",
                "type": "dynamic",
                "runtime": runtime_label,
                "app_id": app_id,
                "title": title,
                "url": relative_url,
                "local_url": local_url,
                "access_url": relative_access_url,
                "local_access_url": local_access_url,
                "access_guard_enabled": access_guard_enabled,
                "public_access_guard_enabled": public_access_guard_enabled,
                "access_key": access_key.clone(),
                "access_password": access_key,
                "expose_public": expose_public,
                "port": port,
                "runtime_preference": runtime_preference.as_str(),
                "apps_page_hint": crate::actions::app::APP_DEPLOY_CONTROL_HINT,
            }));
        }

        self.app_registry
            .register_stored(
                app_id.to_string(),
                crate::actions::app::StoredAppRegistration {
                    title: title.clone(),
                    app_dir,
                    is_static: true,
                    access_key: access_key.clone(),
                    access_guard_enabled,
                    expose_public,
                    enabled: true,
                    last_accessed: None,
                },
            )
            .await;
        Ok(serde_json::json!({
            "status": "restarted",
            "type": "static",
            "app_id": app_id,
            "title": title,
            "url": relative_url,
            "local_url": local_url,
            "access_url": relative_access_url,
            "local_access_url": local_access_url,
            "access_guard_enabled": access_guard_enabled,
            "public_access_guard_enabled": public_access_guard_enabled,
            "access_key": access_key.clone(),
            "access_password": access_key,
            "expose_public": expose_public,
            "runtime_preference": runtime_preference.as_str(),
            "apps_page_hint": crate::actions::app::APP_DEPLOY_CONTROL_HINT,
        }))
    }

    pub(crate) async fn handle_app_restart_tool_call(
        &self,
        call: &crate::core::llm::ToolCall,
        stream_tx: Option<&tokio::sync::mpsc::Sender<StreamEvent>>,
        _request_channel: &str,
        conversation_id: Option<&str>,
    ) -> Result<String> {
        let explicit_app_id = call
            .arguments
            .get("app_id")
            .and_then(|v| v.as_str())
            .unwrap_or("")
            .trim()
            .to_string();
        let title_override = call
            .arguments
            .get("title")
            .and_then(|v| v.as_str())
            .unwrap_or("")
            .trim()
            .to_string();
        let query = call
            .arguments
            .get("query")
            .and_then(|v| v.as_str())
            .unwrap_or("")
            .trim()
            .to_string();

        let mut resolved_app_id = explicit_app_id.clone();
        if resolved_app_id.is_empty() {
            let apps = self.app_registry.list().await;
            if apps.is_empty() {
                let out = serde_json::json!({
                    "app_id": serde_json::Value::Null,
                    "query": if query.is_empty() { serde_json::Value::Null } else { serde_json::json!(query) },
                    "status": "not_found",
                    "message": "No deployed apps are currently registered."
                });
                let formatted = serde_json::to_string_pretty(&out)?;
                if let Some(tx) = stream_tx {
                    queue_stream_event(
                        tx,
                        StreamEvent::ToolResult {
                            name: call.name.clone(),
                            content: formatted.clone(),
                        },
                    );
                }
                return Ok(formatted);
            }

            let ranked_apps = Self::rank_deployed_apps(&query, &apps);
            let best_match = Self::select_best_ranked_app(&query, &ranked_apps);
            if let Some((_, app_id, _, _)) = best_match {
                resolved_app_id = app_id.clone();
            } else {
                let app_summaries = Self::summarize_ranked_apps_for_user(&ranked_apps, 10);
                let out = serde_json::json!({
                    "app_id": serde_json::Value::Null,
                    "query": if query.is_empty() { serde_json::Value::Null } else { serde_json::json!(query) },
                    "status": "not_found",
                    "apps": app_summaries,
                    "message": if query.is_empty() {
                        "app_restart needs an app_id or query to identify which deployed app to restart."
                    } else {
                        "No single deployed app matched the restart request."
                    }
                });
                let formatted = serde_json::to_string_pretty(&out)?;
                if let Some(tx) = stream_tx {
                    queue_stream_event(
                        tx,
                        StreamEvent::ToolResult {
                            name: call.name.clone(),
                            content: formatted.clone(),
                        },
                    );
                }
                return Ok(formatted);
            }
        }

        let out = self
            .restart_deployed_app_from_metadata(
                &resolved_app_id,
                if title_override.is_empty() {
                    None
                } else {
                    Some(title_override.as_str())
                },
            )
            .await?;
        if out
            .get("status")
            .and_then(|v| v.as_str())
            .is_some_and(|status| status == "restarted")
        {
            self.trigger_arkpulse_refresh("app_restart");
            if let Some(cid) = conversation_id {
                let title = out.get("title").and_then(|v| v.as_str()).unwrap_or("App");
                let canonical_url = format!("/apps/{}/", resolved_app_id);
                self.persist_last_deployed_app_context(
                    cid,
                    &resolved_app_id,
                    title,
                    &canonical_url,
                )
                .await;
            }
        }

        let formatted = serde_json::to_string_pretty(&out)?;
        if let Some(tx) = stream_tx {
            queue_stream_event(
                tx,
                StreamEvent::ToolResult {
                    name: call.name.clone(),
                    content: formatted.clone(),
                },
            );
        }
        Ok(formatted)
    }

    pub(crate) async fn handle_app_stop_tool_call(
        &self,
        call: &crate::core::llm::ToolCall,
        stream_tx: Option<&tokio::sync::mpsc::Sender<StreamEvent>>,
        _request_channel: &str,
    ) -> Result<String> {
        let explicit_app_id = call
            .arguments
            .get("app_id")
            .and_then(|v| v.as_str())
            .unwrap_or("")
            .trim()
            .to_string();
        let query = call
            .arguments
            .get("query")
            .and_then(|v| v.as_str())
            .unwrap_or("")
            .trim()
            .to_string();
        let bundle_id = call
            .arguments
            .get("bundle_id")
            .and_then(|v| v.as_str())
            .unwrap_or("")
            .trim()
            .to_string();

        let out = if !bundle_id.is_empty() {
            let bundle_apps = self.find_repo_bundle_apps(&bundle_id).await;
            if bundle_apps.is_empty() {
                serde_json::json!({
                    "status": "not_found",
                    "bundle_id": bundle_id,
                    "message": "No deployed repo bundle matched that bundle_id."
                })
            } else {
                let mut results = Vec::new();
                let mut disabled_count = 0usize;
                let mut failed_count = 0usize;
                for (app, _meta) in bundle_apps {
                    let app_id = app
                        .get("id")
                        .and_then(|v| v.as_str())
                        .unwrap_or("")
                        .to_string();
                    let title = app
                        .get("title")
                        .and_then(|v| v.as_str())
                        .unwrap_or("App")
                        .to_string();
                    let disable_result = if app
                        .get("is_static")
                        .and_then(|v| v.as_bool())
                        .unwrap_or(false)
                    {
                        Ok(())
                    } else if let Some(executor) = build_executor_client() {
                        executor
                            .request(
                                reqwest::Method::POST,
                                &format!("/internal/v1/apps/{}/stop", app_id),
                            )
                            .json(&crate::clients::AppLifecycleRequest {
                                title: None,
                                query: None,
                            })
                            .send()
                            .await
                            .and_then(reqwest::Response::error_for_status)
                            .map(|_| ())
                            .map_err(anyhow::Error::from)
                    } else {
                        self.app_registry.stop_runtime(&app_id).await
                    };
                    match disable_result {
                        Ok(_) => match self.app_registry.set_enabled(&app_id, false).await {
                            Ok(_) => {
                                disabled_count += 1;
                                results.push(serde_json::json!({
                                    "app_id": app_id,
                                    "title": title,
                                    "status": "disabled"
                                }));
                            }
                            Err(error) => {
                                failed_count += 1;
                                results.push(serde_json::json!({
                                    "app_id": app_id,
                                    "title": title,
                                    "status": "failed",
                                    "error": error.to_string()
                                }));
                            }
                        },
                        Err(error) => {
                            failed_count += 1;
                            results.push(serde_json::json!({
                                "app_id": app_id,
                                "title": title,
                                "status": "failed",
                                "error": error.to_string()
                            }));
                        }
                    }
                }
                if disabled_count > 0 {
                    self.trigger_arkpulse_refresh("app_disable");
                }
                serde_json::json!({
                    "status": if failed_count == 0 { "disabled" } else { "partial_failure" },
                    "bundle_id": bundle_id,
                    "disabled_count": disabled_count,
                    "failed_count": failed_count,
                    "apps": results,
                })
            }
        } else {
            let apps = self.app_registry.list().await;
            if apps.is_empty() {
                serde_json::json!({
                    "status": "not_found",
                    "app_id": serde_json::Value::Null,
                    "query": if query.is_empty() { serde_json::Value::Null } else { serde_json::json!(query) },
                    "message": "No deployed apps are currently registered."
                })
            } else {
                let mut resolved_app_id = explicit_app_id.clone();
                if resolved_app_id.is_empty() {
                    let ranked_apps = Self::rank_deployed_apps(&query, &apps);
                    let best_match = Self::select_best_ranked_app(&query, &ranked_apps);
                    if let Some((_, app_id, _, _)) = best_match {
                        resolved_app_id = app_id.clone();
                    } else {
                        let app_summaries = Self::summarize_ranked_apps_for_user(&ranked_apps, 10);
                        let out = serde_json::json!({
                            "status": "not_found",
                            "app_id": serde_json::Value::Null,
                            "query": if query.is_empty() { serde_json::Value::Null } else { serde_json::json!(query) },
                            "apps": app_summaries,
                            "message": if query.is_empty() {
                                "app_stop needs an app_id, query, or bundle_id to identify which deployed app to stop."
                            } else {
                                "No single deployed app matched the stop request."
                            }
                        });
                        let formatted = serde_json::to_string_pretty(&out)?;
                        if let Some(tx) = stream_tx {
                            queue_stream_event(
                                tx,
                                StreamEvent::ToolResult {
                                    name: call.name.clone(),
                                    content: formatted.clone(),
                                },
                            );
                        }
                        return Ok(formatted);
                    }
                }

                if let Some(app) = apps.iter().find(|row| {
                    row.get("id").and_then(|v| v.as_str()) == Some(resolved_app_id.as_str())
                }) {
                    let title = app.get("title").and_then(|v| v.as_str()).unwrap_or("App");
                    let disable_result = if app
                        .get("is_static")
                        .and_then(|v| v.as_bool())
                        .unwrap_or(false)
                    {
                        Ok(())
                    } else if let Some(executor) = build_executor_client() {
                        executor
                            .request(
                                reqwest::Method::POST,
                                &format!("/internal/v1/apps/{}/stop", resolved_app_id),
                            )
                            .json(&crate::clients::AppLifecycleRequest {
                                title: None,
                                query: None,
                            })
                            .send()
                            .await
                            .and_then(reqwest::Response::error_for_status)
                            .map(|_| ())
                            .map_err(anyhow::Error::from)
                    } else {
                        self.app_registry.stop_runtime(&resolved_app_id).await
                    };
                    match disable_result {
                        Ok(_) => match self.app_registry.set_enabled(&resolved_app_id, false).await
                        {
                            Ok(_) => {
                                self.trigger_arkpulse_refresh("app_disable");
                                serde_json::json!({
                                    "status": "disabled",
                                    "app_id": resolved_app_id,
                                    "title": title,
                                })
                            }
                            Err(error) => serde_json::json!({
                                "status": "failed",
                                "app_id": resolved_app_id,
                                "title": title,
                                "error": error.to_string(),
                            }),
                        },
                        Err(error) => serde_json::json!({
                            "status": "failed",
                            "app_id": resolved_app_id,
                            "title": title,
                            "error": error.to_string(),
                        }),
                    }
                } else {
                    serde_json::json!({
                        "status": "not_found",
                        "app_id": resolved_app_id,
                        "message": "The requested app is not currently deployed."
                    })
                }
            }
        };

        let formatted = serde_json::to_string_pretty(&out)?;
        if let Some(tx) = stream_tx {
            queue_stream_event(
                tx,
                StreamEvent::ToolResult {
                    name: call.name.clone(),
                    content: formatted.clone(),
                },
            );
        }
        Ok(formatted)
    }

    pub(crate) async fn handle_app_delete_tool_call(
        &self,
        call: &crate::core::llm::ToolCall,
        stream_tx: Option<&tokio::sync::mpsc::Sender<StreamEvent>>,
        _request_channel: &str,
    ) -> Result<String> {
        let explicit_app_id = call
            .arguments
            .get("app_id")
            .and_then(|v| v.as_str())
            .unwrap_or("")
            .trim()
            .to_string();
        let query = call
            .arguments
            .get("query")
            .and_then(|v| v.as_str())
            .unwrap_or("")
            .trim()
            .to_string();
        let bundle_id = call
            .arguments
            .get("bundle_id")
            .and_then(|v| v.as_str())
            .unwrap_or("")
            .trim()
            .to_string();

        let out = if !bundle_id.is_empty() {
            let bundle_apps = self.find_repo_bundle_apps(&bundle_id).await;
            if bundle_apps.is_empty() {
                serde_json::json!({
                    "status": "not_found",
                    "bundle_id": bundle_id,
                    "message": "No deployed repo bundle matched that bundle_id."
                })
            } else {
                let mut results = Vec::new();
                let mut deleted_count = 0usize;
                let mut failed_count = 0usize;
                for (app, _meta) in bundle_apps {
                    let app_id = app
                        .get("id")
                        .and_then(|v| v.as_str())
                        .unwrap_or("")
                        .to_string();
                    let title = app
                        .get("title")
                        .and_then(|v| v.as_str())
                        .unwrap_or("App")
                        .to_string();
                    match self
                        .stop_and_remove_existing_app(&app_id, Some(title.as_str()))
                        .await
                    {
                        Ok(()) => {
                            deleted_count += 1;
                            results.push(serde_json::json!({
                                "app_id": app_id,
                                "title": title,
                                "status": "deleted"
                            }));
                        }
                        Err(error) => {
                            failed_count += 1;
                            results.push(serde_json::json!({
                                "app_id": app_id,
                                "title": title,
                                "status": "failed",
                                "error": error.to_string()
                            }));
                        }
                    }
                }
                if deleted_count > 0 {
                    self.trigger_arkpulse_refresh("app_delete");
                }
                if self.find_repo_bundle_apps(&bundle_id).await.is_empty() {
                    let _ = self.cleanup_repo_bundle_artifacts(&bundle_id).await;
                }
                serde_json::json!({
                    "status": if failed_count == 0 { "deleted" } else { "partial_failure" },
                    "bundle_id": bundle_id,
                    "deleted_count": deleted_count,
                    "failed_count": failed_count,
                    "apps": results,
                })
            }
        } else {
            let apps = self.app_registry.list().await;
            if apps.is_empty() {
                serde_json::json!({
                    "status": "not_found",
                    "app_id": serde_json::Value::Null,
                    "query": if query.is_empty() { serde_json::Value::Null } else { serde_json::json!(query) },
                    "message": "No deployed apps are currently registered."
                })
            } else {
                let mut resolved_app_id = explicit_app_id.clone();
                if resolved_app_id.is_empty() {
                    let ranked_apps = Self::rank_deployed_apps(&query, &apps);
                    let best_match = Self::select_best_ranked_app(&query, &ranked_apps);
                    if let Some((_, app_id, _, _)) = best_match {
                        resolved_app_id = app_id.clone();
                    } else {
                        let app_summaries = Self::summarize_ranked_apps_for_user(&ranked_apps, 10);
                        let out = serde_json::json!({
                            "status": "not_found",
                            "app_id": serde_json::Value::Null,
                            "query": if query.is_empty() { serde_json::Value::Null } else { serde_json::json!(query) },
                            "apps": app_summaries,
                            "message": if query.is_empty() {
                                "app_delete needs an app_id, query, or bundle_id to identify which deployed app to remove."
                            } else {
                                "No single deployed app matched the delete request."
                            }
                        });
                        let formatted = serde_json::to_string_pretty(&out)?;
                        if let Some(tx) = stream_tx {
                            queue_stream_event(
                                tx,
                                StreamEvent::ToolResult {
                                    name: call.name.clone(),
                                    content: formatted.clone(),
                                },
                            );
                        }
                        return Ok(formatted);
                    }
                }

                if let Some(app) = apps.iter().find(|row| {
                    row.get("id").and_then(|v| v.as_str()) == Some(resolved_app_id.as_str())
                }) {
                    let title = app
                        .get("title")
                        .and_then(|v| v.as_str())
                        .unwrap_or("App")
                        .to_string();
                    let bundle_for_cleanup = self
                        .load_app_metadata(&resolved_app_id)
                        .await
                        .and_then(|meta| {
                            meta.get("repo_bundle_id")
                                .and_then(|v| v.as_str())
                                .map(ToString::to_string)
                        });
                    match self
                        .stop_and_remove_existing_app(&resolved_app_id, Some(title.as_str()))
                        .await
                    {
                        Ok(()) => {
                            self.trigger_arkpulse_refresh("app_delete");
                            if let Some(bundle_id) = bundle_for_cleanup.as_deref() {
                                if self.find_repo_bundle_apps(bundle_id).await.is_empty() {
                                    let _ = self.cleanup_repo_bundle_artifacts(bundle_id).await;
                                }
                            }
                            serde_json::json!({
                                "status": "deleted",
                                "app_id": resolved_app_id,
                                "title": title,
                            })
                        }
                        Err(error) => serde_json::json!({
                            "status": "failed",
                            "app_id": resolved_app_id,
                            "title": title,
                            "error": error.to_string(),
                        }),
                    }
                } else {
                    serde_json::json!({
                        "status": "not_found",
                        "app_id": resolved_app_id,
                        "message": "The requested app is not currently deployed."
                    })
                }
            }
        };

        let formatted = serde_json::to_string_pretty(&out)?;
        if let Some(tx) = stream_tx {
            queue_stream_event(
                tx,
                StreamEvent::ToolResult {
                    name: call.name.clone(),
                    content: formatted.clone(),
                },
            );
        }
        Ok(formatted)
    }
    /// Handle self-evolve tool call with policy-first evolution defaults.
    async fn gepa_idle_check(
        &self,
        quiet_window_seconds: i64,
        allow_current_request: bool,
    ) -> GepaIdleCheck {
        let mut reasons = Vec::new();
        let active_request_threshold = if allow_current_request { 1 } else { 0 };
        let active_requests = self.active_message_request_count();
        if active_requests > active_request_threshold {
            reasons.push(format!("foreground_requests={}", active_requests));
        }

        if quiet_window_seconds > 0 {
            if let Some(last_activity) = self.last_activity_at() {
                let idle_for = (chrono::Utc::now() - last_activity).num_seconds();
                if idle_for < quiet_window_seconds {
                    reasons.push(format!(
                        "quiet_window_pending={}s",
                        quiet_window_seconds.saturating_sub(idle_for)
                    ));
                }
            }
        }

        match self.storage.lease_status_summary().await {
            Ok(summary) => {
                if summary.active_task_leases > 0 {
                    reasons.push(format!("active_task_leases={}", summary.active_task_leases));
                }
                if summary.active_watcher_leases > 0 {
                    reasons.push(format!(
                        "active_watcher_leases={}",
                        summary.active_watcher_leases
                    ));
                }
                if summary.active_run_leases > 0 {
                    reasons.push(format!("active_run_leases={}", summary.active_run_leases));
                }
                if summary.pending_task_backlog > 0 {
                    reasons.push(format!(
                        "pending_task_backlog={}",
                        summary.pending_task_backlog
                    ));
                }
                if summary.watcher_poll_backlog > 0 {
                    reasons.push(format!(
                        "watcher_poll_backlog={}",
                        summary.watcher_poll_backlog
                    ));
                }
            }
            Err(error) => reasons.push(format!("lease_status_unknown={}", error)),
        }

        let background_active = self
            .background_sessions
            .list()
            .await
            .into_iter()
            .filter(|session| {
                matches!(
                    session.status,
                    crate::core::background_session::BackgroundSessionStatus::Active
                )
            })
            .count();
        if background_active > 0 {
            reasons.push(format!("active_background_sessions={}", background_active));
        }

        let browser_active = self.browser_sessions.active_count();
        if browser_active > 0 {
            reasons.push(format!("active_browser_sessions={}", browser_active));
        }

        let runtime_containers = self.runtime.active_container_count().await;
        if runtime_containers > 0 {
            reasons.push(format!("active_runtime_containers={}", runtime_containers));
        }

        if let Some(swarm) = self.swarm.as_ref() {
            let status = swarm.status().await;
            if status.active_agents > 0 {
                reasons.push(format!("active_swarm_agents={}", status.active_agents));
            }
        }

        let active_dynamic_apps = self
            .app_registry
            .list()
            .await
            .into_iter()
            .filter(|app| {
                let running = app
                    .get("running")
                    .and_then(|value| value.as_bool())
                    .unwrap_or(false);
                let restoring = app
                    .get("restoring")
                    .and_then(|value| value.as_bool())
                    .unwrap_or(false);
                let is_static = app
                    .get("is_static")
                    .and_then(|value| value.as_bool())
                    .unwrap_or(false);
                (running || restoring) && !is_static
            })
            .count();
        if active_dynamic_apps > 0 {
            reasons.push(format!("active_dynamic_apps={}", active_dynamic_apps));
        }

        GepaIdleCheck {
            idle: reasons.is_empty(),
            quiet_window_seconds,
            reasons,
        }
    }

    async fn queue_gepa_job(
        &self,
        kind: crate::core::self_evolve::gepa_bridge::GepaJobKind,
        request: &str,
        run_id: Option<String>,
        export_path: Option<String>,
        candidates_path: Option<String>,
        quiet_window_seconds: i64,
        promotion: crate::core::self_evolve::gepa_bridge::GepaPromotionSettings,
        optimizer_timeout_seconds: u64,
        import_after_run: bool,
    ) -> Result<String> {
        let job = crate::core::self_evolve::gepa_bridge::PendingGepaJob {
            job_id: format!("gepa-job-{}", uuid::Uuid::new_v4().simple()),
            kind,
            request: request.to_string(),
            run_id,
            export_path,
            candidates_path,
            quiet_window_seconds,
            promotion,
            optimizer_timeout_seconds,
            max_attempts: 3,
            attempt_count: 0,
            last_error: None,
            import_after_run,
            started_at: None,
            finished_at: None,
            created_at: chrono::Utc::now().to_rfc3339(),
        };
        let path = crate::core::self_evolve::gepa_bridge::write_pending_job(
            &self.find_project_root(),
            &job,
        )
        .await?;
        self.spawn_gepa_idle_worker();
        Ok(path)
    }

    pub(crate) fn spawn_gepa_idle_worker(&self) {
        if GEPA_IDLE_WORKER_ACTIVE.swap(true, Ordering::AcqRel) {
            return;
        }
        let agent = self.clone();
        tokio::spawn(async move {
            let result = agent.run_gepa_idle_worker().await;
            if let Err(error) = result {
                tracing::warn!("GEPA pending job worker failed: {}", error);
            }
            GEPA_IDLE_WORKER_ACTIVE.store(false, Ordering::Release);
        });
    }

    pub(crate) async fn gepa_background_idle_check(
        &self,
        quiet_window_seconds: i64,
    ) -> GepaIdleCheck {
        self.gepa_idle_check(quiet_window_seconds, false).await
    }

    pub(crate) async fn queue_gepa_seed_run(
        &self,
        request: &str,
        quiet_window_seconds: i64,
    ) -> Result<String> {
        self.queue_gepa_job(
            crate::core::self_evolve::gepa_bridge::GepaJobKind::Run,
            request,
            None,
            None,
            None,
            quiet_window_seconds,
            crate::core::self_evolve::gepa_bridge::GepaPromotionSettings::default(),
            crate::core::self_evolve::gepa_bridge::default_gepa_optimizer_timeout_seconds(),
            true,
        )
        .await
    }

    async fn run_gepa_idle_worker(&self) -> Result<()> {
        let project_root = self.find_project_root();
        let retention =
            crate::core::self_evolve::gepa_bridge::prune_gepa_artifacts(&project_root).await?;
        if retention.run_dirs_removed > 0
            || retention.status_files_removed > 0
            || retention.stale_running_jobs_requeued > 0
        {
            tracing::info!(
                "GEPA artifact maintenance removed {} run dirs, removed {} status files, requeued {} stale jobs",
                retention.run_dirs_removed,
                retention.status_files_removed,
                retention.stale_running_jobs_requeued
            );
        }

        loop {
            if !crate::core::self_evolve::gepa_bridge::has_pending_jobs(&project_root).await? {
                return Ok(());
            }
            let idle = self.gepa_idle_check(60, false).await;
            if idle.idle {
                let processed = self.process_pending_gepa_jobs().await?;
                if !processed.is_empty()
                    && crate::core::self_evolve::gepa_bridge::has_pending_jobs(&project_root)
                        .await?
                {
                    tokio::time::sleep(std::time::Duration::from_secs(20)).await;
                }
                continue;
            }
            tokio::time::sleep(std::time::Duration::from_secs(20)).await;
        }
    }

    async fn process_pending_gepa_jobs(&self) -> Result<Vec<serde_json::Value>> {
        let project_root = self.find_project_root();
        let mut results = Vec::new();
        loop {
            let Some((running_path, job)) =
                crate::core::self_evolve::gepa_bridge::claim_next_pending_job(&project_root)
                    .await?
            else {
                break;
            };
            let idle = self
                .gepa_idle_check(job.quiet_window_seconds.max(0), false)
                .await;
            if !idle.idle {
                let _ = crate::core::self_evolve::gepa_bridge::requeue_claimed_job(
                    &project_root,
                    &running_path,
                    job,
                    "GEPA worker claimed the job but runtime was no longer idle",
                )
                .await?;
                break;
            }
            let job_for_failure = job.clone();
            match self.execute_gepa_job(job).await {
                Ok(value) => {
                    if gepa_job_value_failed(&value) {
                        let message = value
                            .get("stderr_tail")
                            .or_else(|| value.get("error"))
                            .and_then(|value| value.as_str())
                            .unwrap_or("GEPA job returned a failed status");
                        let (status, status_path) =
                            crate::core::self_evolve::gepa_bridge::fail_claimed_job(
                                &project_root,
                                &running_path,
                                job_for_failure,
                                message,
                            )
                            .await?;
                        let failed = serde_json::json!({
                            "status": status,
                            "status_path": status_path.display().to_string(),
                            "result": value,
                        });
                        self.store_gepa_last_result(&failed).await;
                        results.push(failed);
                    } else {
                        let status_path =
                            crate::core::self_evolve::gepa_bridge::complete_claimed_job(
                                &project_root,
                                &running_path,
                                job_for_failure,
                                &value,
                            )
                            .await?;
                        let completed = serde_json::json!({
                            "status": "completed",
                            "status_path": status_path.display().to_string(),
                            "result": value,
                        });
                        self.store_gepa_last_result(&completed).await;
                        results.push(completed);
                    }
                }
                Err(error) => {
                    tracing::warn!("Pending GEPA job {:?} failed: {}", running_path, error);
                    let (status, status_path) =
                        crate::core::self_evolve::gepa_bridge::fail_claimed_job(
                            &project_root,
                            &running_path,
                            job_for_failure,
                            &error.to_string(),
                        )
                        .await?;
                    let failed = serde_json::json!({
                        "status": status,
                        "status_path": status_path.display().to_string(),
                        "error": error.to_string(),
                    });
                    self.store_gepa_last_result(&failed).await;
                    results.push(failed);
                }
            }
        }
        Ok(results)
    }

    async fn store_gepa_last_result(&self, value: &serde_json::Value) {
        if let Ok(bytes) = serde_json::to_vec(value) {
            let _ = self
                .storage
                .set(
                    crate::core::self_evolve::gepa_bridge::GEPA_OPTIMIZER_LAST_RESULT_KEY,
                    &bytes,
                )
                .await;
        }
        let mode = value
            .get("mode")
            .or_else(|| value.get("result").and_then(|result| result.get("mode")))
            .and_then(|value| value.as_str())
            .unwrap_or("");
        if mode == "gepa_status" {
            return;
        }
        let status = gepa_effective_status(value);
        let now = chrono::Utc::now().to_rfc3339();
        let mut state =
            crate::core::self_evolve::gepa_bridge::load_gepa_auto_run_state(&self.storage).await;
        state.last_status = Some(status.clone());
        state.last_reason = gepa_effective_reason(value).or_else(|| Some(status.clone()));
        if status == "queued" {
            state.last_queued_at = Some(now);
        } else if matches!(
            status.as_str(),
            "completed" | "failed" | "timed_out" | "blocked"
        ) {
            state.last_completed_at = Some(now);
        }
        let _ =
            crate::core::self_evolve::gepa_bridge::save_gepa_auto_run_state(&self.storage, &state)
                .await;
    }

    async fn execute_gepa_job(
        &self,
        job: crate::core::self_evolve::gepa_bridge::PendingGepaJob,
    ) -> Result<serde_json::Value> {
        let project_root = self.find_project_root();
        match job.kind {
            crate::core::self_evolve::gepa_bridge::GepaJobKind::Export => {
                let result = crate::core::self_evolve::gepa_bridge::export_optimization_bundle(
                    &self.storage,
                    &project_root,
                    &job.request,
                    job.promotion.replay_log_limit,
                )
                .await?;
                Ok(serde_json::to_value(result)?)
            }
            crate::core::self_evolve::gepa_bridge::GepaJobKind::Run => {
                let export_path = if let Some(path) = job.export_path.as_ref() {
                    resolve_gepa_workspace_path(&project_root, path)?
                } else {
                    let export = crate::core::self_evolve::gepa_bridge::export_optimization_bundle(
                        &self.storage,
                        &project_root,
                        &job.request,
                        job.promotion.replay_log_limit,
                    )
                    .await?;
                    std::path::PathBuf::from(export.export_path)
                };
                let candidates_path = job
                    .candidates_path
                    .as_ref()
                    .map(|path| resolve_gepa_workspace_path(&project_root, path))
                    .transpose()?
                    .or_else(|| {
                        job.run_id.as_deref().map(|run_id| {
                            crate::core::self_evolve::gepa_bridge::default_candidates_path(
                                &project_root,
                                run_id,
                            )
                        })
                    })
                    .unwrap_or_else(|| {
                        export_path
                            .parent()
                            .unwrap_or_else(|| std::path::Path::new("."))
                            .join("candidates.jsonl")
                    });
                let runtime = match crate::core::self_evolve::gepa_bridge::gepa_optimizer_runtime(
                    &self.storage,
                    &project_root,
                    &self.config,
                    &self.primary_model_id,
                )
                .await
                {
                    Ok(runtime) => runtime,
                    Err(error) => {
                        return Ok(serde_json::json!({
                            "status": "blocked",
                            "mode": "gepa_run",
                            "error": error.to_string(),
                        }));
                    }
                };
                let budget_run_id = job.run_id.as_deref().unwrap_or(&job.job_id);
                if let Err(error) = crate::core::self_evolve::gepa_bridge::reserve_gepa_budget(
                    &self.storage,
                    budget_run_id,
                    "reserved",
                )
                .await
                {
                    return Ok(serde_json::json!({
                        "status": "blocked",
                        "mode": "gepa_run",
                        "error": error.to_string(),
                    }));
                }
                let result = crate::core::self_evolve::gepa_bridge::run_python_optimizer(
                    &export_path,
                    &candidates_path,
                    job.optimizer_timeout_seconds,
                    &runtime,
                )
                .await?;
                let mut value = serde_json::to_value(&result)?;
                if job.import_after_run && result.status == "completed" {
                    let import_result = self
                        .execute_gepa_import(
                            &job.request,
                            &candidates_path,
                            job.promotion.apply_promotion,
                            job.promotion.canary_rollout_percent,
                            job.promotion.canary_min_samples_per_version,
                            job.promotion.canary_min_success_gain,
                            job.promotion.canary_max_sign_test_p_value,
                            job.promotion.replay_log_limit,
                        )
                        .await?;
                    if let serde_json::Value::Object(obj) = &mut value {
                        obj.insert("import_result".to_string(), import_result);
                    }
                }
                Ok(value)
            }
            crate::core::self_evolve::gepa_bridge::GepaJobKind::Import => {
                let candidates_path = resolve_gepa_candidates_path(
                    &project_root,
                    job.run_id.as_deref(),
                    job.candidates_path.as_deref(),
                )?;
                self.execute_gepa_import(
                    &job.request,
                    &candidates_path,
                    job.promotion.apply_promotion,
                    job.promotion.canary_rollout_percent,
                    job.promotion.canary_min_samples_per_version,
                    job.promotion.canary_min_success_gain,
                    job.promotion.canary_max_sign_test_p_value,
                    job.promotion.replay_log_limit,
                )
                .await
            }
        }
    }

    async fn execute_gepa_import(
        &self,
        request: &str,
        candidates_path: &std::path::Path,
        apply_promotion: bool,
        canary_rollout_percent: u8,
        canary_min_samples_per_version: usize,
        canary_min_success_gain: f64,
        canary_max_sign_test_p_value: f64,
        replay_log_limit: u64,
    ) -> Result<serde_json::Value> {
        let project_root = self.find_project_root();
        let imported =
            crate::core::self_evolve::gepa_bridge::import_candidates(candidates_path).await?;
        let mut results = Vec::new();

        if !imported.prompt_candidates.is_empty() {
            let current_prompt_raw = self
                .storage
                .get(crate::core::self_evolve::PROMPT_BUNDLE_PROFILE_KEY)
                .await
                .ok()
                .flatten();
            let engine = crate::core::self_evolve::PromptEvolutionEngine::new(
                crate::core::self_evolve::PromptEvolutionConfig {
                    project_root: project_root.clone(),
                    max_candidates: imported.prompt_candidates.len().max(1),
                    ..Default::default()
                },
                self.llm.clone(),
            );
            let result = engine
                .evaluate_external_prompt_candidates(
                    request,
                    current_prompt_raw.as_deref(),
                    imported.prompt_candidates,
                )
                .await?;

            let mut promotion_applied = false;
            let mut canary_state: Option<
                crate::core::self_evolve::strategy_runtime::CanaryRolloutState,
            > = None;
            let mut replay_result: Option<
                crate::core::self_evolve::strategy_runtime::ReplayEvaluationResult,
            > = None;
            if result.promoted && apply_promotion {
                if let Some(bundle) = result.promoted_prompt_bundle.as_ref() {
                    let candidate_serialized = serde_json::to_vec(bundle)?;
                    if let Some(existing_baseline) = current_prompt_raw.as_ref() {
                        let _ = self
                            .storage
                            .set(
                                crate::core::self_evolve::PROMPT_BUNDLE_BASELINE_SNAPSHOT_KEY,
                                existing_baseline,
                            )
                            .await;
                    }
                    let baseline_bundle_version = current_prompt_raw
                        .as_ref()
                        .and_then(|raw| {
                            crate::core::self_evolve::prompt_evolution::parse_prompt_bundle_profile(
                                raw,
                            )
                            .map(|bundle| bundle.version)
                        })
                        .unwrap_or_else(|| result.baseline_version.clone());
                    let baseline_version =
                        crate::core::self_evolve::prompt_evolution::compose_prompt_version(
                            &baseline_bundle_version,
                        );
                    let candidate_version =
                        crate::core::self_evolve::prompt_evolution::compose_prompt_version(
                            &result.candidate_version,
                        );
                    self.storage
                        .set(
                            crate::core::self_evolve::PROMPT_BUNDLE_PROFILE_CANARY_KEY,
                            &candidate_serialized,
                        )
                        .await?;
                    let state = crate::core::self_evolve::strategy_runtime::CanaryRolloutState {
                        enabled: true,
                        baseline_version,
                        candidate_version,
                        rollout_percent: canary_rollout_percent,
                        min_samples_per_version: canary_min_samples_per_version,
                        min_success_gain: canary_min_success_gain,
                        max_sign_test_p_value: canary_max_sign_test_p_value,
                        activated_at: Some(chrono::Utc::now().to_rfc3339()),
                    };
                    self.storage
                        .set(
                            crate::core::self_evolve::PROMPT_BUNDLE_CANARY_STATE_KEY,
                            &serde_json::to_vec(&state)?,
                        )
                        .await?;
                    canary_state = Some(state.clone());
                    if let Ok(runs) = self
                        .storage
                        .list_recent_experience_runs_any_scope(replay_log_limit)
                        .await
                    {
                        replay_result = Some(
                            crate::core::self_evolve::strategy_runtime::evaluate_experience_canary_by_prompt_version(
                                &runs,
                                &state.baseline_version,
                                &state.candidate_version,
                                state.min_samples_per_version,
                                state.min_success_gain,
                                state.max_sign_test_p_value,
                            ),
                        );
                    }
                    promotion_applied = true;
                }
            }
            let mut value = serde_json::to_value(&result)?;
            if let serde_json::Value::Object(obj) = &mut value {
                obj.insert("mode".to_string(), serde_json::json!("gepa_import_prompt"));
                obj.insert(
                    "promotion_applied".to_string(),
                    serde_json::json!(promotion_applied),
                );
                obj.insert(
                    "runtime_promotion_applied".to_string(),
                    serde_json::json!(promotion_applied),
                );
                obj.insert(
                    "apply_promotion_requested".to_string(),
                    serde_json::json!(apply_promotion),
                );
                obj.insert(
                    "promotion_requires_user_acceptance".to_string(),
                    serde_json::json!(true),
                );
                obj.insert(
                    "promotion_mode".to_string(),
                    serde_json::json!(if promotion_applied {
                        "canary"
                    } else if result.promoted {
                        "pending_user_acceptance"
                    } else {
                        "none"
                    }),
                );
                obj.insert(
                    "candidate_review_id".to_string(),
                    serde_json::json!(result.lineage_entry_id.clone()),
                );
                obj.insert(
                    "candidate_source_path".to_string(),
                    serde_json::json!(candidates_path.display().to_string()),
                );
                obj.insert(
                    "rollback_available".to_string(),
                    serde_json::json!(promotion_applied),
                );
                obj.insert(
                    "canary_state".to_string(),
                    serde_json::to_value(&canary_state).unwrap_or(serde_json::Value::Null),
                );
                obj.insert(
                    "replay_evaluation".to_string(),
                    serde_json::to_value(&replay_result).unwrap_or(serde_json::Value::Null),
                );
            }
            if let Ok(bytes) = serde_json::to_vec(&value) {
                let _ = self
                    .storage
                    .set(
                        crate::core::self_evolve::PROMPT_BUNDLE_LAST_RESULT_KEY,
                        &bytes,
                    )
                    .await;
            }
            results.push(value);
        }

        if !imported.specialist_prompt_candidates.is_empty() {
            let current_specialist_raw = self
                .storage
                .get(crate::core::self_evolve::SPECIALIST_PROMPT_BUNDLE_PROFILE_KEY)
                .await
                .ok()
                .flatten();
            let engine = crate::core::self_evolve::SpecialistPromptEvolutionEngine::new(
                crate::core::self_evolve::SpecialistPromptEvolutionConfig {
                    project_root: project_root.clone(),
                    max_candidates: imported.specialist_prompt_candidates.len().max(1),
                    ..Default::default()
                },
                self.llm.clone(),
            );
            let result = engine
                .evaluate_external_specialist_prompt_candidates(
                    request,
                    current_specialist_raw.as_deref(),
                    imported.specialist_prompt_candidates,
                )
                .await?;
            let mut promotion_applied = false;
            let mut canary_state: Option<
                crate::core::self_evolve::strategy_runtime::CanaryRolloutState,
            > = None;
            let mut replay_result: Option<
                crate::core::self_evolve::strategy_runtime::ReplayEvaluationResult,
            > = None;
            if result.promoted && apply_promotion {
                if let Some(bundle) = result.promoted_specialist_bundle.as_ref() {
                    let candidate_serialized = serde_json::to_vec(bundle)?;
                    if let Some(existing_baseline) = current_specialist_raw.as_ref() {
                        let _ = self
                            .storage
                            .set(
                                crate::core::self_evolve::SPECIALIST_PROMPT_BUNDLE_BASELINE_SNAPSHOT_KEY,
                                existing_baseline,
                            )
                            .await;
                    }
                    let baseline_bundle_version = current_specialist_raw
                        .as_ref()
                        .and_then(|raw| {
                            crate::core::self_evolve::specialist_prompt_evolution::parse_specialist_prompt_bundle_profile(raw)
                                .map(|bundle| bundle.version)
                        })
                        .unwrap_or_else(|| result.baseline_version.clone());
                    let baseline_version =
                        crate::core::self_evolve::specialist_prompt_evolution::compose_specialist_prompt_version(
                            &baseline_bundle_version,
                        );
                    let candidate_version =
                        crate::core::self_evolve::specialist_prompt_evolution::compose_specialist_prompt_version(
                            &result.candidate_version,
                        );
                    self.storage
                        .set(
                            crate::core::self_evolve::SPECIALIST_PROMPT_BUNDLE_PROFILE_CANARY_KEY,
                            &candidate_serialized,
                        )
                        .await?;
                    let state = crate::core::self_evolve::strategy_runtime::CanaryRolloutState {
                        enabled: true,
                        baseline_version,
                        candidate_version,
                        rollout_percent: canary_rollout_percent,
                        min_samples_per_version: canary_min_samples_per_version,
                        min_success_gain: canary_min_success_gain,
                        max_sign_test_p_value: canary_max_sign_test_p_value,
                        activated_at: Some(chrono::Utc::now().to_rfc3339()),
                    };
                    self.storage
                        .set(
                            crate::core::self_evolve::SPECIALIST_PROMPT_BUNDLE_CANARY_STATE_KEY,
                            &serde_json::to_vec(&state)?,
                        )
                        .await?;
                    canary_state = Some(state.clone());
                    if let Ok(runs) = self
                        .storage
                        .list_recent_experience_runs_any_scope(replay_log_limit)
                        .await
                    {
                        replay_result = Some(
                            crate::core::self_evolve::strategy_runtime::evaluate_experience_canary_by_metadata_version(
                                &runs,
                                "specialist_prompt_version",
                                &state.baseline_version,
                                &state.candidate_version,
                                state.min_samples_per_version,
                                state.min_success_gain,
                                state.max_sign_test_p_value,
                            ),
                        );
                    }
                    promotion_applied = true;
                }
            }
            let mut value = serde_json::to_value(&result)?;
            if let serde_json::Value::Object(obj) = &mut value {
                obj.insert(
                    "mode".to_string(),
                    serde_json::json!("gepa_import_specialist_prompt"),
                );
                obj.insert(
                    "promotion_applied".to_string(),
                    serde_json::json!(promotion_applied),
                );
                obj.insert(
                    "runtime_promotion_applied".to_string(),
                    serde_json::json!(promotion_applied),
                );
                obj.insert(
                    "apply_promotion_requested".to_string(),
                    serde_json::json!(apply_promotion),
                );
                obj.insert(
                    "promotion_requires_user_acceptance".to_string(),
                    serde_json::json!(true),
                );
                obj.insert(
                    "promotion_mode".to_string(),
                    serde_json::json!(if promotion_applied {
                        "canary"
                    } else if result.promoted {
                        "pending_user_acceptance"
                    } else {
                        "none"
                    }),
                );
                obj.insert(
                    "candidate_review_id".to_string(),
                    serde_json::json!(result.lineage_entry_id.clone()),
                );
                obj.insert(
                    "candidate_source_path".to_string(),
                    serde_json::json!(candidates_path.display().to_string()),
                );
                obj.insert(
                    "rollback_available".to_string(),
                    serde_json::json!(promotion_applied),
                );
                obj.insert(
                    "canary_state".to_string(),
                    serde_json::to_value(&canary_state).unwrap_or(serde_json::Value::Null),
                );
                obj.insert(
                    "replay_evaluation".to_string(),
                    serde_json::to_value(&replay_result).unwrap_or(serde_json::Value::Null),
                );
            }
            if let Ok(bytes) = serde_json::to_vec(&value) {
                let _ = self
                    .storage
                    .set(
                        crate::core::self_evolve::SPECIALIST_PROMPT_BUNDLE_LAST_RESULT_KEY,
                        &bytes,
                    )
                    .await;
            }
            results.push(value);
        }

        if !imported.prompt_fragment_candidates.is_empty() {
            let current_fragment_raw = self
                .storage
                .get(crate::core::prompt_fragments::PROMPT_FRAGMENT_BUNDLE_PROFILE_KEY)
                .await
                .ok()
                .flatten();
            let result =
                crate::core::self_evolve::prompt_fragment_evolution::evaluate_external_prompt_fragment_candidates(
                    project_root.clone(),
                    request,
                    current_fragment_raw.as_deref(),
                    imported.prompt_fragment_candidates,
                )
                .await?;

            let mut promotion_applied = false;
            let mut canary_state: Option<
                crate::core::self_evolve::strategy_runtime::CanaryRolloutState,
            > = None;
            let mut replay_result: Option<
                crate::core::self_evolve::strategy_runtime::ReplayEvaluationResult,
            > = None;
            if result.promoted && apply_promotion {
                if let Some(bundle) = result.promoted_prompt_fragment_bundle.as_ref() {
                    let candidate_serialized = serde_json::to_vec(bundle)?;
                    if let Some(existing_baseline) = current_fragment_raw.as_ref() {
                        let _ = self
                            .storage
                            .set(
                                crate::core::prompt_fragments::PROMPT_FRAGMENT_BUNDLE_BASELINE_SNAPSHOT_KEY,
                                existing_baseline,
                            )
                            .await;
                    }
                    let baseline_bundle_version = current_fragment_raw
                        .as_ref()
                        .and_then(|raw| {
                            crate::core::prompt_fragments::parse_prompt_fragment_bundle_profile(raw)
                                .map(|bundle| bundle.version)
                        })
                        .unwrap_or_else(|| result.baseline_version.clone());
                    let baseline_version =
                        crate::core::prompt_fragments::compose_prompt_fragment_version(
                            &baseline_bundle_version,
                        );
                    let candidate_version =
                        crate::core::prompt_fragments::compose_prompt_fragment_version(
                            &result.candidate_version,
                        );
                    self.storage
                        .set(
                            crate::core::prompt_fragments::PROMPT_FRAGMENT_BUNDLE_PROFILE_CANARY_KEY,
                            &candidate_serialized,
                        )
                        .await?;
                    let state = crate::core::self_evolve::strategy_runtime::CanaryRolloutState {
                        enabled: true,
                        baseline_version,
                        candidate_version,
                        rollout_percent: canary_rollout_percent,
                        min_samples_per_version: canary_min_samples_per_version,
                        min_success_gain: canary_min_success_gain,
                        max_sign_test_p_value: canary_max_sign_test_p_value,
                        activated_at: Some(chrono::Utc::now().to_rfc3339()),
                    };
                    self.storage
                        .set(
                            crate::core::prompt_fragments::PROMPT_FRAGMENT_BUNDLE_CANARY_STATE_KEY,
                            &serde_json::to_vec(&state)?,
                        )
                        .await?;
                    canary_state = Some(state.clone());
                    if let Ok(traces) = self
                        .storage
                        .list_execution_trace_summaries(None, replay_log_limit, 0)
                        .await
                    {
                        replay_result = Some(
                            crate::core::self_evolve::strategy_runtime::evaluate_trace_prompt_telemetry_canary_by_version(
                                &traces,
                                "prompt_fragment_version",
                                &state.baseline_version,
                                &state.candidate_version,
                                state.min_samples_per_version,
                                state.min_success_gain,
                                state.max_sign_test_p_value,
                            ),
                        );
                    }
                    promotion_applied = true;
                }
            }
            let mut value = serde_json::to_value(&result)?;
            if let serde_json::Value::Object(obj) = &mut value {
                obj.insert(
                    "mode".to_string(),
                    serde_json::json!("gepa_import_prompt_fragment"),
                );
                obj.insert(
                    "promotion_applied".to_string(),
                    serde_json::json!(promotion_applied),
                );
                obj.insert(
                    "runtime_promotion_applied".to_string(),
                    serde_json::json!(promotion_applied),
                );
                obj.insert(
                    "apply_promotion_requested".to_string(),
                    serde_json::json!(apply_promotion),
                );
                obj.insert(
                    "promotion_requires_user_acceptance".to_string(),
                    serde_json::json!(true),
                );
                obj.insert(
                    "promotion_mode".to_string(),
                    serde_json::json!(if promotion_applied {
                        "canary"
                    } else if result.promoted {
                        "pending_user_acceptance"
                    } else {
                        "none"
                    }),
                );
                obj.insert(
                    "candidate_review_id".to_string(),
                    serde_json::json!(result.lineage_entry_id.clone()),
                );
                obj.insert(
                    "candidate_source_path".to_string(),
                    serde_json::json!(candidates_path.display().to_string()),
                );
                obj.insert(
                    "rollback_available".to_string(),
                    serde_json::json!(promotion_applied),
                );
                obj.insert(
                    "canary_state".to_string(),
                    serde_json::to_value(&canary_state).unwrap_or(serde_json::Value::Null),
                );
                obj.insert(
                    "replay_evaluation".to_string(),
                    serde_json::to_value(&replay_result).unwrap_or(serde_json::Value::Null),
                );
            }
            if let Ok(bytes) = serde_json::to_vec(&value) {
                let _ = self
                    .storage
                    .set(
                        crate::core::prompt_fragments::PROMPT_FRAGMENT_BUNDLE_LAST_RESULT_KEY,
                        &bytes,
                    )
                    .await;
            }
            results.push(value);
        }

        Ok(serde_json::json!({
            "status": "completed",
            "mode": "gepa_import",
            "candidate_source_path": candidates_path.display().to_string(),
            "promotion_requires_user_acceptance": true,
            "summary": imported.summary,
            "results": results,
        }))
    }

    pub(crate) async fn handle_self_evolve_tool_call(
        &self,
        call: &crate::core::llm::ToolCall,
        trace_ref: &Arc<RwLock<ExecutionTrace>>,
        stream_tx: Option<&tokio::sync::mpsc::Sender<StreamEvent>>,
    ) -> Result<String> {
        // Check if self-evolve is enabled in settings.
        let enabled = self
            .storage
            .get(crate::core::self_evolve::strategy_runtime::SELF_EVOLVE_ENABLED_KEY)
            .await
            .ok()
            .flatten()
            .and_then(|raw| String::from_utf8(raw).ok())
            .map(|s| !s.trim().eq_ignore_ascii_case("false"))
            .unwrap_or(true)
            && crate::core::learning::load_learning_enabled(&self.storage).await;
        if !enabled {
            return Ok(serde_json::json!({
                "status": "disabled",
                "message": "Self-evolution is currently disabled. Enable it in Settings > Advanced > ArkEvolve."
            })
            .to_string());
        }

        let request = call
            .arguments
            .get("request")
            .and_then(|v| v.as_str())
            .unwrap_or("")
            .to_string();

        let mode = call
            .arguments
            .get("mode")
            .and_then(|v| v.as_str())
            .unwrap_or("policy")
            .trim()
            .to_ascii_lowercase();
        if request.is_empty() && mode != "gepa_status" {
            return Ok(serde_json::json!({
                "status": "error",
                "message": "Missing 'request' parameter - describe what should evolve"
            })
            .to_string());
        }
        let allow_code_writes = call
            .arguments
            .get("allow_code_writes")
            .and_then(|v| v.as_bool())
            .unwrap_or(false);
        let apply_promotion_requested = call
            .arguments
            .get("apply_promotion")
            .and_then(|v| v.as_bool())
            .unwrap_or(false);
        let apply_promotion = false;
        let canary_rollout_percent = call
            .arguments
            .get("canary_rollout_percent")
            .and_then(|v| v.as_u64())
            .map(|v| v.clamp(1, 100) as u8)
            .unwrap_or(20);
        let canary_min_samples_per_version = call
            .arguments
            .get("canary_min_samples_per_version")
            .and_then(|v| v.as_u64())
            .map(|v| v.clamp(5, 20_000) as usize)
            .unwrap_or(25);
        let canary_min_success_gain = call
            .arguments
            .get("canary_min_success_gain")
            .and_then(|v| v.as_f64())
            .map(|v| v.clamp(0.0, 0.5))
            .unwrap_or(0.03);
        let canary_max_sign_test_p_value = call
            .arguments
            .get("canary_max_sign_test_p_value")
            .and_then(|v| v.as_f64())
            .map(|v| v.clamp(0.0001, 1.0))
            .unwrap_or(0.10);
        let replay_log_limit = call
            .arguments
            .get("replay_log_limit")
            .and_then(|v| v.as_u64())
            .map(|v| v.clamp(100, 100_000))
            .unwrap_or(4_000);
        let gepa_quiet_window_seconds = call
            .arguments
            .get("gepa_quiet_window_seconds")
            .and_then(|v| v.as_i64())
            .map(|v| v.clamp(0, 3600))
            .unwrap_or(60);
        let gepa_optimizer_timeout_seconds = call
            .arguments
            .get("gepa_optimizer_timeout_seconds")
            .and_then(|v| v.as_u64())
            .map(|v| v.clamp(30, 6 * 60 * 60))
            .unwrap_or_else(
                crate::core::self_evolve::gepa_bridge::default_gepa_optimizer_timeout_seconds,
            );
        let gepa_promotion = crate::core::self_evolve::gepa_bridge::GepaPromotionSettings {
            apply_promotion,
            canary_rollout_percent,
            canary_min_samples_per_version,
            canary_min_success_gain,
            canary_max_sign_test_p_value,
            replay_log_limit,
        };
        let gepa_run_id = call
            .arguments
            .get("gepa_run_id")
            .and_then(|v| v.as_str())
            .map(str::trim)
            .filter(|v| !v.is_empty())
            .map(ToString::to_string);
        let gepa_export_path = call
            .arguments
            .get("export_path")
            .and_then(|v| v.as_str())
            .map(str::trim)
            .filter(|v| !v.is_empty())
            .map(ToString::to_string);
        let gepa_candidates_path = call
            .arguments
            .get("candidates_path")
            .and_then(|v| v.as_str())
            .map(str::trim)
            .filter(|v| !v.is_empty())
            .map(ToString::to_string);
        let gepa_import_after_run = call
            .arguments
            .get("import_after_run")
            .and_then(|v| v.as_bool())
            .unwrap_or(false);

        push_trace_step(
            trace_ref,
            "[evolve]",
            "Self-Evolve Request",
            format!(
                "Requested {} evolution for {}.",
                if mode == "code" || mode == "codebase" {
                    "code"
                } else if mode == "prompt" {
                    "prompt"
                } else {
                    "policy"
                },
                crate::branding::PRODUCT_NAME
            ),
            "thinking",
            Some(serde_json::json!({
                "trace_kind": "self_evolve.request",
                "request": request.clone(),
                "mode": mode.clone(),
                "allow_code_writes": allow_code_writes,
                "apply_promotion": apply_promotion,
                "apply_promotion_requested": apply_promotion_requested,
                "promotion_requires_user_acceptance": true,
                "canary_rollout_percent": canary_rollout_percent,
                "canary_min_samples_per_version": canary_min_samples_per_version,
                "canary_min_success_gain": canary_min_success_gain,
                "canary_max_sign_test_p_value": canary_max_sign_test_p_value,
                "replay_log_limit": replay_log_limit,
                "gepa_quiet_window_seconds": gepa_quiet_window_seconds,
                "gepa_optimizer_timeout_seconds": gepa_optimizer_timeout_seconds,
                "gepa_run_id": gepa_run_id.clone(),
                "export_path": gepa_export_path.clone(),
                "candidates_path": gepa_candidates_path.clone(),
                "import_after_run": gepa_import_after_run,
            })),
            None,
        )
        .await;

        if let Some(tx) = stream_tx {
            queue_stream_event(
                tx,
                StreamEvent::ToolStart {
                    name: "self_evolve".to_string(),
                    payload: None,
                },
            );
        }

        tracing::info!(
            "Self-evolve request mode={} request={}",
            mode,
            &request[..request.len().min(100)]
        );

        let project_root = self.find_project_root();
        let llm = self.llm.clone();

        match mode.as_str() {
            "gepa_status" => {
                let maintenance =
                    crate::core::self_evolve::gepa_bridge::prune_gepa_artifacts(&project_root)
                        .await?;
                if crate::core::self_evolve::gepa_bridge::has_pending_jobs(&project_root).await? {
                    self.spawn_gepa_idle_worker();
                }
                let snapshot =
                    crate::core::self_evolve::gepa_bridge::queue_status_snapshot(&project_root, 25)
                        .await?;
                let value = serde_json::json!({
                    "status": "completed",
                    "mode": "gepa_status",
                    "maintenance": maintenance,
                    "queue": snapshot,
                });
                self.store_gepa_last_result(&value).await;
                Ok(serde_json::to_string_pretty(&value)?)
            }
            "gepa_export" | "gepa_run" | "gepa_import" => {
                let idle = self.gepa_idle_check(gepa_quiet_window_seconds, true).await;
                let kind = match mode.as_str() {
                    "gepa_export" => crate::core::self_evolve::gepa_bridge::GepaJobKind::Export,
                    "gepa_run" => crate::core::self_evolve::gepa_bridge::GepaJobKind::Run,
                    _ => crate::core::self_evolve::gepa_bridge::GepaJobKind::Import,
                };
                if !idle.idle {
                    let pending_path = self
                        .queue_gepa_job(
                            kind,
                            &request,
                            gepa_run_id.clone(),
                            gepa_export_path.clone(),
                            gepa_candidates_path.clone(),
                            gepa_quiet_window_seconds,
                            gepa_promotion.clone(),
                            gepa_optimizer_timeout_seconds,
                            gepa_import_after_run,
                        )
                        .await?;
                    let value = serde_json::json!({
                        "status": "queued",
                        "mode": mode,
                        "pending_job_path": pending_path,
                        "idle_check": idle,
                        "message": "GEPA work was queued and will start after AgentArk has been idle for the configured quiet window."
                    });
                    self.store_gepa_last_result(&value).await;
                    return Ok(serde_json::to_string_pretty(&value)?);
                }

                let value = match mode.as_str() {
                    "gepa_export" => {
                        let result =
                            crate::core::self_evolve::gepa_bridge::export_optimization_bundle(
                                &self.storage,
                                &project_root,
                                &request,
                                replay_log_limit,
                            )
                            .await?;
                        serde_json::to_value(result)?
                    }
                    "gepa_run" => {
                        let mut budget_run_id = gepa_run_id.clone();
                        let export_path = if let Some(path) = gepa_export_path.as_ref() {
                            resolve_gepa_workspace_path(&project_root, path)?
                        } else {
                            let export =
                                crate::core::self_evolve::gepa_bridge::export_optimization_bundle(
                                    &self.storage,
                                    &project_root,
                                    &request,
                                    replay_log_limit,
                                )
                                .await?;
                            budget_run_id = Some(export.run_id.clone());
                            std::path::PathBuf::from(export.export_path)
                        };
                        let candidates_path = gepa_candidates_path
                            .as_ref()
                            .map(|path| resolve_gepa_workspace_path(&project_root, path))
                            .transpose()?
                            .or_else(|| {
                                gepa_run_id.as_deref().map(|run_id| {
                                    crate::core::self_evolve::gepa_bridge::default_candidates_path(
                                        &project_root,
                                        run_id,
                                    )
                                })
                            })
                            .unwrap_or_else(|| {
                                export_path
                                    .parent()
                                    .unwrap_or_else(|| std::path::Path::new("."))
                                    .join("candidates.jsonl")
                            });
                        match crate::core::self_evolve::gepa_bridge::gepa_optimizer_runtime(
                            &self.storage,
                            &project_root,
                            &self.config,
                            &self.primary_model_id,
                        )
                        .await
                        {
                            Ok(runtime) => {
                                let budget_run_id = budget_run_id
                                    .unwrap_or_else(|| uuid::Uuid::new_v4().to_string());
                                if let Err(error) =
                                    crate::core::self_evolve::gepa_bridge::reserve_gepa_budget(
                                        &self.storage,
                                        &budget_run_id,
                                        "reserved",
                                    )
                                    .await
                                {
                                    serde_json::json!({
                                        "status": "blocked",
                                        "mode": "gepa_run",
                                        "error": error.to_string(),
                                    })
                                } else {
                                    let run_result =
                                        crate::core::self_evolve::gepa_bridge::run_python_optimizer(
                                            &export_path,
                                            &candidates_path,
                                            gepa_optimizer_timeout_seconds,
                                            &runtime,
                                        )
                                        .await?;
                                    let mut value = serde_json::to_value(&run_result)?;
                                    if gepa_import_after_run && run_result.status == "completed" {
                                        let import_result = self
                                            .execute_gepa_import(
                                                &request,
                                                &candidates_path,
                                                apply_promotion,
                                                canary_rollout_percent,
                                                canary_min_samples_per_version,
                                                canary_min_success_gain,
                                                canary_max_sign_test_p_value,
                                                replay_log_limit,
                                            )
                                            .await?;
                                        if let serde_json::Value::Object(obj) = &mut value {
                                            obj.insert("import_result".to_string(), import_result);
                                        }
                                    }
                                    value
                                }
                            }
                            Err(error) => serde_json::json!({
                                "status": "blocked",
                                "mode": "gepa_run",
                                "error": error.to_string(),
                            }),
                        }
                    }
                    _ => {
                        let candidates_path = resolve_gepa_candidates_path(
                            &project_root,
                            gepa_run_id.as_deref(),
                            gepa_candidates_path.as_deref(),
                        )?;
                        self.execute_gepa_import(
                            &request,
                            &candidates_path,
                            apply_promotion,
                            canary_rollout_percent,
                            canary_min_samples_per_version,
                            canary_min_success_gain,
                            canary_max_sign_test_p_value,
                            replay_log_limit,
                        )
                        .await?
                    }
                };
                self.store_gepa_last_result(&value).await;
                Ok(serde_json::to_string_pretty(&value)?)
            }
            "policy" | "strategy" | "policy_strategy" => {
                let policy_start = std::time::Instant::now();
                let current_policy_raw = self
                    .storage
                    .get(crate::core::self_evolve::ROUTING_COMPLEXITY_POLICY_KEY)
                    .await
                    .ok()
                    .flatten();
                let config = crate::core::self_evolve::PolicyEvolutionConfig {
                    project_root,
                    ..Default::default()
                };
                let evolve_engine =
                    crate::core::self_evolve::PolicyEvolutionEngine::new(config, llm);
                let result = evolve_engine
                    .evolve_routing_policy(&request, current_policy_raw.as_deref())
                    .await?;

                let mut promotion_applied = false;
                let mut canary_state: Option<
                    crate::core::self_evolve::strategy_runtime::CanaryRolloutState,
                > = None;
                let mut replay_result: Option<
                    crate::core::self_evolve::strategy_runtime::ReplayEvaluationResult,
                > = None;
                let mut promoted_directly_to_baseline = false;
                if result.promoted && apply_promotion {
                    if let Some(policy_json) = result.promoted_policy.as_ref() {
                        let candidate_serialized = serde_json::to_vec(policy_json)?;
                        if let Some(existing_baseline) = current_policy_raw.as_ref() {
                            let _ = self
                                .storage
                                .set(
                                    crate::core::self_evolve::strategy_runtime::ROUTING_COMPLEXITY_POLICY_BASELINE_SNAPSHOT_KEY,
                                    existing_baseline,
                                )
                                .await;
                        }
                        let baseline_version = self
                            .storage
                            .get(
                                crate::core::self_evolve::strategy_runtime::ROUTING_COMPLEXITY_CANARY_STATE_KEY,
                            )
                            .await
                            .ok()
                            .flatten()
                            .and_then(|raw| {
                                serde_json::from_slice::<
                                    crate::core::self_evolve::strategy_runtime::CanaryRolloutState,
                                >(&raw)
                                .ok()
                                .map(|s| s.baseline_version)
                            })
                            .unwrap_or_else(|| "routing-policy-default-v1".to_string());
                        let candidate_version =
                            format!("routing-candidate-{}", result.lineage_entry_id);

                        self.storage
                            .set(
                                crate::core::self_evolve::strategy_runtime::ROUTING_COMPLEXITY_POLICY_CANARY_KEY,
                                &candidate_serialized,
                            )
                            .await?;
                        let state =
                            crate::core::self_evolve::strategy_runtime::CanaryRolloutState {
                                enabled: true,
                                baseline_version: baseline_version.clone(),
                                candidate_version: candidate_version.clone(),
                                rollout_percent: canary_rollout_percent,
                                min_samples_per_version: canary_min_samples_per_version,
                                min_success_gain: canary_min_success_gain,
                                max_sign_test_p_value: canary_max_sign_test_p_value,
                                activated_at: Some(chrono::Utc::now().to_rfc3339()),
                            };
                        let state_bytes = serde_json::to_vec(&state)?;
                        self.storage
                            .set(
                                crate::core::self_evolve::strategy_runtime::ROUTING_COMPLEXITY_CANARY_STATE_KEY,
                                &state_bytes,
                            )
                            .await?;
                        canary_state = Some(state.clone());

                        if let Ok(logs) = self
                            .storage
                            .list_operational_logs_by_event("tool_call", replay_log_limit)
                            .await
                        {
                            let replay_eval =
                                crate::core::self_evolve::strategy_runtime::evaluate_canary_by_policy_version(
                                    &logs,
                                    &state.baseline_version,
                                    &state.candidate_version,
                                    state.min_samples_per_version,
                                    state.min_success_gain,
                                    state.max_sign_test_p_value,
                                );
                            if replay_eval.promote {
                                self.storage
                                    .set(
                                        crate::core::self_evolve::ROUTING_COMPLEXITY_POLICY_KEY,
                                        &candidate_serialized,
                                    )
                                    .await?;
                                let mut disabled_state = state.clone();
                                disabled_state.enabled = false;
                                let disabled_bytes = serde_json::to_vec(&disabled_state)?;
                                self.storage
                                    .set(
                                        crate::core::self_evolve::strategy_runtime::ROUTING_COMPLEXITY_CANARY_STATE_KEY,
                                        &disabled_bytes,
                                    )
                                    .await?;
                                promoted_directly_to_baseline = true;
                                canary_state = Some(disabled_state);
                            }
                            replay_result = Some(replay_eval);
                        }
                        promotion_applied = true;
                    }
                }

                if let Some(tx) = stream_tx {
                    let status_msg = if result.promoted {
                        if promotion_applied {
                            if promoted_directly_to_baseline {
                                format!(
                                    "Policy evolution complete: promoted candidate (gain {:.4}, p={:.4}), replay gate passed, baseline updated immediately",
                                    result.accuracy_gain, result.p_value
                                )
                            } else {
                                format!(
                                    "Policy evolution complete: promoted candidate (gain {:.4}, p={:.4}) activated in canary mode ({}%)",
                                    result.accuracy_gain,
                                    result.p_value,
                                    canary_state
                                        .as_ref()
                                        .map(|s| s.rollout_percent)
                                        .unwrap_or(canary_rollout_percent)
                                )
                            }
                        } else {
                            format!(
                                "Policy evolution complete: candidate passed promotion gate (gain {:.4}, p={:.4}) but not applied",
                                result.accuracy_gain, result.p_value
                            )
                        }
                    } else {
                        format!(
                            "Policy evolution complete: {}",
                            result.promotion_gate_summary
                        )
                    };
                    queue_stream_event(
                        tx,
                        StreamEvent::ToolResult {
                            name: "self_evolve".to_string(),
                            content: status_msg,
                        },
                    );
                }

                let changed_fields = result
                    .promoted_policy
                    .as_ref()
                    .map(|policy| json_changed_keys(current_policy_raw.as_deref(), policy))
                    .unwrap_or_default();
                let policy_step_type = if result.success && result.promoted {
                    "success"
                } else if result.success {
                    "info"
                } else {
                    "error"
                };
                let policy_detail = if result.success {
                    format!(
                        "Evaluated {} candidate policies. Accuracy {:.0}% -> {:.0}% with gate: {}",
                        result.evaluated_candidates,
                        result.baseline_accuracy * 100.0,
                        result.best_candidate_accuracy * 100.0,
                        result.promotion_gate_summary
                    )
                } else {
                    format!(
                        "Policy evolution failed: {}",
                        result.error.as_deref().unwrap_or("unknown error")
                    )
                };
                push_trace_step(
                    trace_ref,
                    "[evolve]",
                    "Policy Evolution Evaluated",
                    policy_detail,
                    policy_step_type,
                    Some(serde_json::json!({
                        "trace_kind": "self_evolve.policy.result",
                        "request": request.clone(),
                        "mode": "policy",
                        "target_key": result.target_key.clone(),
                        "success": result.success,
                        "promoted": result.promoted,
                        "evaluated_candidates": result.evaluated_candidates,
                        "baseline_accuracy": result.baseline_accuracy,
                        "best_candidate_accuracy": result.best_candidate_accuracy,
                        "accuracy_gain": result.accuracy_gain,
                        "wins": result.wins,
                        "losses": result.losses,
                        "p_value": result.p_value,
                        "candidate_source": result.candidate_source.clone(),
                        "promotion_gate": result.promotion_gate.clone(),
                        "promotion_gate_summary": result.promotion_gate_summary.clone(),
                        "promotion_gate_report": result.promotion_gate_report.clone(),
                        "lineage_entry_id": result.lineage_entry_id.clone(),
                        "lineage_archive_path": result.lineage_archive_path.clone(),
                        "notes": result.notes.clone(),
                        "error": result.error.clone(),
                        "changed_fields": changed_fields.clone(),
                        "promoted_policy": result.promoted_policy.clone(),
                    })),
                    Some(policy_start.elapsed().as_millis() as u64),
                )
                .await;

                let promotion_mode = if promoted_directly_to_baseline {
                    "baseline"
                } else if promotion_applied {
                    "canary"
                } else {
                    "none"
                };
                let promotion_detail = if promoted_directly_to_baseline {
                    "Replay evaluation promoted the candidate directly to baseline.".to_string()
                } else if promotion_applied {
                    format!(
                        "Candidate activated in canary mode at {}% rollout.",
                        canary_state
                            .as_ref()
                            .map(|state| state.rollout_percent)
                            .unwrap_or(canary_rollout_percent)
                    )
                } else if result.promoted {
                    "Candidate passed the promotion gate but was not applied.".to_string()
                } else {
                    format!("No promotion applied. {}", result.promotion_gate_summary)
                };
                push_trace_step(
                    trace_ref,
                    if promotion_applied { "[ok]" } else { "[info]" },
                    "Policy Promotion Decision",
                    promotion_detail,
                    if promotion_applied { "success" } else { "info" },
                    Some(serde_json::json!({
                        "trace_kind": "self_evolve.policy.promotion",
                        "request": request.clone(),
                        "promotion_applied": promotion_applied,
                        "apply_promotion_requested": apply_promotion_requested,
                        "promotion_mode": promotion_mode,
                        "promoted_directly_to_baseline": promoted_directly_to_baseline,
                        "canary_state": canary_state.clone(),
                        "replay_evaluation": replay_result.clone(),
                    })),
                    None,
                )
                .await;

                let mut value = serde_json::to_value(&result)?;
                if let serde_json::Value::Object(obj) = &mut value {
                    obj.insert("mode".to_string(), serde_json::json!("policy"));
                    obj.insert(
                        "promotion_applied".to_string(),
                        serde_json::json!(promotion_applied),
                    );
                    obj.insert(
                        "apply_promotion_requested".to_string(),
                        serde_json::json!(apply_promotion_requested),
                    );
                    obj.insert(
                        "promotion_mode".to_string(),
                        serde_json::json!(if promoted_directly_to_baseline {
                            "baseline"
                        } else if promotion_applied {
                            "canary"
                        } else {
                            "none"
                        }),
                    );
                    obj.insert(
                        "canary_state".to_string(),
                        serde_json::to_value(&canary_state).unwrap_or(serde_json::Value::Null),
                    );
                    obj.insert(
                        "replay_evaluation".to_string(),
                        serde_json::to_value(&replay_result).unwrap_or(serde_json::Value::Null),
                    );
                }
                if let Ok(last_bytes) = serde_json::to_vec(&value) {
                    let _ = self
                        .storage
                        .set(
                            crate::core::self_evolve::strategy_runtime::SELF_EVOLVE_LAST_RESULT_KEY,
                            &last_bytes,
                        )
                        .await;
                }
                // Return human-friendly summary instead of raw JSON
                let summary = if result.success {
                    if result.promoted {
                        let mode_label = if promoted_directly_to_baseline {
                            "applied immediately"
                        } else if promotion_applied {
                            "activated in canary mode for gradual rollout"
                        } else {
                            "ready but not yet applied"
                        };
                        format!(
                            "Self-evolution completed successfully.\n\n\
                            I evaluated {} candidate strategies and found an improvement.\n\
                            - Accuracy improved from {:.0}% to {:.0}% ({} wins, {} losses)\n\
                            - The improved strategy has been {}\n\n\
                            Your agent's decision-making is now more accurate.",
                            result.evaluated_candidates,
                            result.baseline_accuracy * 100.0,
                            result.best_candidate_accuracy * 100.0,
                            result.wins,
                            result.losses,
                            mode_label,
                        )
                    } else {
                        format!(
                            "Self-evolution completed. I evaluated {} candidate strategies \
                            but none outperformed the current approach (accuracy: {:.0}%). \
                            No changes were made.",
                            result.evaluated_candidates,
                            result.baseline_accuracy * 100.0,
                        )
                    }
                } else {
                    format!(
                        "Self-evolution ran but encountered an issue: {}",
                        result.error.as_deref().unwrap_or("unknown error")
                    )
                };
                Ok(summary)
            }
            "prompt" => {
                let prompt_start = std::time::Instant::now();
                let current_prompt_raw = self
                    .storage
                    .get(crate::core::self_evolve::PROMPT_BUNDLE_PROFILE_KEY)
                    .await
                    .ok()
                    .flatten();
                let config = crate::core::self_evolve::PromptEvolutionConfig {
                    project_root,
                    ..Default::default()
                };
                let evolve_engine =
                    crate::core::self_evolve::PromptEvolutionEngine::new(config, llm);
                let result = evolve_engine
                    .evolve_prompt_bundle(&request, current_prompt_raw.as_deref())
                    .await?;

                let mut promotion_applied = false;
                let mut canary_state: Option<
                    crate::core::self_evolve::strategy_runtime::CanaryRolloutState,
                > = None;
                let mut replay_result: Option<
                    crate::core::self_evolve::strategy_runtime::ReplayEvaluationResult,
                > = None;
                let mut promoted_directly_to_baseline = false;
                if result.promoted && apply_promotion {
                    if let Some(bundle) = result.promoted_prompt_bundle.as_ref() {
                        let candidate_serialized = serde_json::to_vec(bundle)?;
                        if let Some(existing_baseline) = current_prompt_raw.as_ref() {
                            let _ = self
                                .storage
                                .set(
                                    crate::core::self_evolve::PROMPT_BUNDLE_BASELINE_SNAPSHOT_KEY,
                                    existing_baseline,
                                )
                                .await;
                        }
                        let baseline_bundle_version = current_prompt_raw
                            .as_ref()
                            .and_then(|raw| {
                                crate::core::self_evolve::prompt_evolution::parse_prompt_bundle_profile(raw)
                                    .map(|bundle| bundle.version)
                            })
                            .unwrap_or_else(|| result.baseline_version.clone());
                        let baseline_version =
                            crate::core::self_evolve::prompt_evolution::compose_prompt_version(
                                &baseline_bundle_version,
                            );
                        let candidate_version =
                            crate::core::self_evolve::prompt_evolution::compose_prompt_version(
                                &result.candidate_version,
                            );

                        self.storage
                            .set(
                                crate::core::self_evolve::PROMPT_BUNDLE_PROFILE_CANARY_KEY,
                                &candidate_serialized,
                            )
                            .await?;
                        let state =
                            crate::core::self_evolve::strategy_runtime::CanaryRolloutState {
                                enabled: true,
                                baseline_version: baseline_version.clone(),
                                candidate_version: candidate_version.clone(),
                                rollout_percent: canary_rollout_percent,
                                min_samples_per_version: canary_min_samples_per_version,
                                min_success_gain: canary_min_success_gain,
                                max_sign_test_p_value: canary_max_sign_test_p_value,
                                activated_at: Some(chrono::Utc::now().to_rfc3339()),
                            };
                        let state_bytes = serde_json::to_vec(&state)?;
                        self.storage
                            .set(
                                crate::core::self_evolve::PROMPT_BUNDLE_CANARY_STATE_KEY,
                                &state_bytes,
                            )
                            .await?;
                        canary_state = Some(state.clone());

                        if let Ok(runs) = self
                            .storage
                            .list_recent_experience_runs_any_scope(replay_log_limit)
                            .await
                        {
                            let replay_eval = crate::core::self_evolve::strategy_runtime::evaluate_experience_canary_by_prompt_version(
                                &runs,
                                &state.baseline_version,
                                &state.candidate_version,
                                state.min_samples_per_version,
                                state.min_success_gain,
                                state.max_sign_test_p_value,
                            );
                            if replay_eval.promote {
                                self.storage
                                    .set(
                                        crate::core::self_evolve::PROMPT_BUNDLE_PROFILE_KEY,
                                        &candidate_serialized,
                                    )
                                    .await?;
                                let mut disabled_state = state.clone();
                                disabled_state.enabled = false;
                                let disabled_bytes = serde_json::to_vec(&disabled_state)?;
                                self.storage
                                    .set(
                                        crate::core::self_evolve::PROMPT_BUNDLE_CANARY_STATE_KEY,
                                        &disabled_bytes,
                                    )
                                    .await?;
                                promoted_directly_to_baseline = true;
                                canary_state = Some(disabled_state);
                            }
                            replay_result = Some(replay_eval);
                        }
                        promotion_applied = true;
                    }
                }

                if let Some(tx) = stream_tx {
                    let status_msg = if result.promoted {
                        if promotion_applied {
                            if promoted_directly_to_baseline {
                                format!(
                                    "Prompt evolution complete: promoted candidate (gain {:.4}, p={:.4}), replay gate passed, baseline updated immediately",
                                    result.score_gain, result.p_value
                                )
                            } else {
                                format!(
                                    "Prompt evolution complete: promoted candidate (gain {:.4}, p={:.4}) activated in canary mode ({}%)",
                                    result.score_gain,
                                    result.p_value,
                                    canary_state
                                        .as_ref()
                                        .map(|s| s.rollout_percent)
                                        .unwrap_or(canary_rollout_percent)
                                )
                            }
                        } else {
                            format!(
                                "Prompt evolution complete: candidate passed promotion gate (gain {:.4}, p={:.4}) but not applied",
                                result.score_gain, result.p_value
                            )
                        }
                    } else {
                        format!(
                            "Prompt evolution complete: {}",
                            result.promotion_gate_summary
                        )
                    };
                    queue_stream_event(
                        tx,
                        StreamEvent::ToolResult {
                            name: "self_evolve".to_string(),
                            content: status_msg,
                        },
                    );
                }

                push_trace_step(
                    trace_ref,
                    if result.success && result.promoted {
                        "[evolve]"
                    } else if result.success {
                        "[info]"
                    } else {
                        "[error]"
                    },
                    "Prompt Evolution Evaluated",
                    if result.success {
                        format!(
                            "Evaluated {} prompt candidates. Score {:.0}% -> {:.0}% with gate: {}",
                            result.evaluated_candidates,
                            result.baseline_score * 100.0,
                            result.best_candidate_score * 100.0,
                            result.promotion_gate_summary
                        )
                    } else {
                        format!(
                            "Prompt evolution failed: {}",
                            result.error.as_deref().unwrap_or("unknown error")
                        )
                    },
                    if result.success && result.promoted {
                        "success"
                    } else if result.success {
                        "info"
                    } else {
                        "error"
                    },
                    Some(serde_json::json!({
                        "trace_kind": "self_evolve.prompt.result",
                        "request": request.clone(),
                        "mode": "prompt",
                        "target_key": result.target_key.clone(),
                        "success": result.success,
                        "promoted": result.promoted,
                        "evaluated_candidates": result.evaluated_candidates,
                        "baseline_version": result.baseline_version.clone(),
                        "candidate_version": result.candidate_version.clone(),
                        "baseline_score": result.baseline_score,
                        "best_candidate_score": result.best_candidate_score,
                        "score_gain": result.score_gain,
                        "baseline_router_score": result.baseline_router_score,
                        "best_candidate_router_score": result.best_candidate_router_score,
                        "baseline_synthesis_score": result.baseline_synthesis_score,
                        "best_candidate_synthesis_score": result.best_candidate_synthesis_score,
                        "baseline_router_invalid_json_rate": result.baseline_router_invalid_json_rate,
                        "candidate_router_invalid_json_rate": result.candidate_router_invalid_json_rate,
                        "wins": result.wins,
                        "losses": result.losses,
                        "p_value": result.p_value,
                        "candidate_source": result.candidate_source.clone(),
                        "optimized_surfaces": result.optimized_surfaces.clone(),
                        "promotion_gate": result.promotion_gate.clone(),
                        "promotion_gate_summary": result.promotion_gate_summary.clone(),
                        "promotion_gate_report": result.promotion_gate_report.clone(),
                        "lineage_entry_id": result.lineage_entry_id.clone(),
                        "lineage_archive_path": result.lineage_archive_path.clone(),
                        "notes": result.notes.clone(),
                        "diff_summary": result.diff_summary.clone(),
                        "promoted_prompt_bundle": result.promoted_prompt_bundle.clone(),
                        "error": result.error.clone(),
                    })),
                    Some(prompt_start.elapsed().as_millis() as u64),
                )
                .await;

                let promotion_mode = if promoted_directly_to_baseline {
                    "baseline"
                } else if promotion_applied {
                    "canary"
                } else {
                    "none"
                };
                let promotion_detail = if promoted_directly_to_baseline {
                    "Replay evaluation promoted the prompt bundle directly to baseline.".to_string()
                } else if promotion_applied {
                    format!(
                        "Prompt candidate activated in canary mode at {}% rollout.",
                        canary_state
                            .as_ref()
                            .map(|state| state.rollout_percent)
                            .unwrap_or(canary_rollout_percent)
                    )
                } else if result.promoted {
                    "Prompt candidate passed the promotion gate but was not applied.".to_string()
                } else {
                    format!(
                        "No prompt promotion applied. {}",
                        result.promotion_gate_summary
                    )
                };
                push_trace_step(
                    trace_ref,
                    if promotion_applied { "[ok]" } else { "[info]" },
                    "Prompt Promotion Decision",
                    promotion_detail,
                    if promotion_applied { "success" } else { "info" },
                    Some(serde_json::json!({
                        "trace_kind": "self_evolve.prompt.promotion",
                        "request": request.clone(),
                        "promotion_applied": promotion_applied,
                        "apply_promotion_requested": apply_promotion_requested,
                        "promotion_mode": promotion_mode,
                        "promoted_directly_to_baseline": promoted_directly_to_baseline,
                        "baseline_version": result.baseline_version.clone(),
                        "candidate_version": result.candidate_version.clone(),
                        "optimized_surfaces": result.optimized_surfaces.clone(),
                        "diff_summary": result.diff_summary.clone(),
                        "canary_state": canary_state.clone(),
                        "replay_evaluation": replay_result.clone(),
                    })),
                    None,
                )
                .await;

                let mut value = serde_json::to_value(&result)?;
                if let serde_json::Value::Object(obj) = &mut value {
                    obj.insert("mode".to_string(), serde_json::json!("prompt"));
                    obj.insert(
                        "promotion_applied".to_string(),
                        serde_json::json!(promotion_applied),
                    );
                    obj.insert(
                        "apply_promotion_requested".to_string(),
                        serde_json::json!(apply_promotion_requested),
                    );
                    obj.insert(
                        "promotion_mode".to_string(),
                        serde_json::json!(promotion_mode),
                    );
                    obj.insert(
                        "canary_state".to_string(),
                        serde_json::to_value(&canary_state).unwrap_or(serde_json::Value::Null),
                    );
                    obj.insert(
                        "replay_evaluation".to_string(),
                        serde_json::to_value(&replay_result).unwrap_or(serde_json::Value::Null),
                    );
                }
                if let Ok(last_bytes) = serde_json::to_vec(&value) {
                    let _ = self
                        .storage
                        .set(
                            crate::core::self_evolve::PROMPT_BUNDLE_LAST_RESULT_KEY,
                            &last_bytes,
                        )
                        .await;
                }

                let summary = if result.success {
                    if result.promoted {
                        let mode_label = if promoted_directly_to_baseline {
                            "applied immediately"
                        } else if promotion_applied {
                            "activated in canary mode for gradual rollout"
                        } else {
                            "ready but not yet applied"
                        };
                        format!(
                            "Prompt evolution completed successfully.\n\n\
                            I evaluated {} prompt bundles and found an improvement.\n\
                            - Combined benchmark score improved from {:.0}% to {:.0}% ({} wins, {} losses)\n\
                            - Router invalid JSON rate changed from {:.1}% to {:.1}%\n\
                            - The improved prompt bundle has been {}\n\n\
                            {} is now testing a better routing, primary response, and delegated synthesis prompt set.",
                            result.evaluated_candidates,
                            result.baseline_score * 100.0,
                            result.best_candidate_score * 100.0,
                            result.wins,
                            result.losses,
                            result.baseline_router_invalid_json_rate * 100.0,
                            result.candidate_router_invalid_json_rate * 100.0,
                            mode_label,
                            crate::branding::PRODUCT_NAME
                        )
                    } else {
                        format!(
                            "Prompt evolution completed. I evaluated {} prompt bundles but none outperformed the current prompt bundle (score: {:.0}%). No changes were made.",
                            result.evaluated_candidates,
                            result.baseline_score * 100.0,
                        )
                    }
                } else {
                    format!(
                        "Prompt evolution ran but encountered an issue: {}",
                        result.error.as_deref().unwrap_or("unknown error")
                    )
                };
                Ok(summary)
            }
            "specialist_prompt" => {
                self.handle_specialist_prompt_evolution(
                    &request,
                    project_root,
                    llm,
                    trace_ref,
                    stream_tx,
                    apply_promotion,
                    canary_rollout_percent,
                    canary_min_samples_per_version,
                    canary_min_success_gain,
                    canary_max_sign_test_p_value,
                    replay_log_limit,
                )
                .await
            }
            "code" | "codebase" => {
                if !allow_code_writes {
                    push_trace_step(
                        trace_ref,
                        "[warn]",
                        "Code Evolution Blocked",
                        &format!(
                            "Code evolution requires explicit `allow_code_writes=true` before {} will modify its own code.",
                            crate::branding::PRODUCT_NAME
                        ),
                        "warning",
                        Some(serde_json::json!({
                            "trace_kind": "self_evolve.code.blocked",
                            "request": request.clone(),
                            "mode": "code",
                            "allow_code_writes": allow_code_writes,
                        })),
                        None,
                    )
                    .await;
                    return Ok(serde_json::json!({
                        "status": "blocked",
                        "mode": "code",
                        "message": "Code evolution is disabled by default. Re-run self_evolve with mode='code' and allow_code_writes=true after policy evolution is stable."
                    })
                    .to_string());
                }

                let code_start = std::time::Instant::now();
                let config = crate::core::self_evolve::SelfEvolveConfig {
                    max_iterations: 25,
                    max_build_fix_cycles: 5,
                    project_root,
                };
                let evolve_agent = crate::core::self_evolve::SelfEvolveAgent::new(config, llm);
                let result = evolve_agent.execute(&request).await?;

                if let Some(tx) = stream_tx {
                    let status_msg = if result.success {
                        let mut msg = format!(
                            "Code evolution complete: {} files changed in {} iterations",
                            result.files_changed.len(),
                            result.iterations_used
                        );
                        if result.push_recommended {
                            msg.push_str(
                                ". Local changes are ready; ask the user whether to push to remote.",
                            );
                        }
                        msg
                    } else {
                        format!(
                            "Code evolution failed: {}",
                            result.error.as_deref().unwrap_or("unknown error")
                        )
                    };
                    queue_stream_event(
                        tx,
                        StreamEvent::ToolResult {
                            name: "self_evolve".to_string(),
                            content: status_msg,
                        },
                    );
                }

                push_trace_step(
                    trace_ref,
                    if result.success { "[ok]" } else { "[error]" },
                    if result.success {
                        "Code Evolution Completed"
                    } else {
                        "Code Evolution Failed"
                    },
                    if result.success {
                        format!(
                            "Changed {} files over {} iteration(s).",
                            result.files_changed.len(),
                            result.iterations_used
                        )
                    } else {
                        format!(
                            "Code evolution failed after {} iteration(s).",
                            result.iterations_used
                        )
                    },
                    if result.success { "success" } else { "error" },
                    Some(serde_json::json!({
                        "trace_kind": "self_evolve.code.result",
                        "request": request.clone(),
                        "mode": "code",
                        "success": result.success,
                        "diff_summary": result.diff_summary.clone(),
                        "files_changed": result.files_changed.clone(),
                        "iterations_used": result.iterations_used,
                        "error": result.error.clone(),
                        "security_warnings": result.security_warnings.clone(),
                        "push_recommended": result.push_recommended,
                        "push_suggestion": result.push_suggestion.clone(),
                    })),
                    Some(code_start.elapsed().as_millis() as u64),
                )
                .await;

                Ok(serde_json::to_string_pretty(&result)?)
            }
            _ => {
                push_trace_step(
                    trace_ref,
                    "[error]",
                    "Self-Evolve Mode Rejected",
                    format!("Unsupported self_evolve mode '{}'.", mode),
                    "error",
                    Some(serde_json::json!({
                        "trace_kind": "self_evolve.mode_error",
                        "request": request.clone(),
                        "mode": mode.clone(),
                    })),
                    None,
                )
                .await;
                Ok(serde_json::json!({
                    "status": "error",
                    "message": format!(
                        "Unsupported self_evolve mode '{}'. Use mode='policy' (default), mode='prompt', mode='specialist_prompt', mode='gepa_export', mode='gepa_run', mode='gepa_import', mode='gepa_status', or mode='code'.",
                        mode
                    ),
                })
                .to_string())
            }
        }
    }

    async fn handle_specialist_prompt_evolution(
        &self,
        request: &str,
        project_root: std::path::PathBuf,
        llm: crate::core::llm::LlmClient,
        trace_ref: &Arc<RwLock<ExecutionTrace>>,
        stream_tx: Option<&tokio::sync::mpsc::Sender<StreamEvent>>,
        apply_promotion: bool,
        canary_rollout_percent: u8,
        canary_min_samples_per_version: usize,
        canary_min_success_gain: f64,
        canary_max_sign_test_p_value: f64,
        replay_log_limit: u64,
    ) -> Result<String> {
        let specialist_start = std::time::Instant::now();
        let current_specialist_raw = self
            .storage
            .get(crate::core::self_evolve::SPECIALIST_PROMPT_BUNDLE_PROFILE_KEY)
            .await
            .ok()
            .flatten();
        let config = crate::core::self_evolve::SpecialistPromptEvolutionConfig {
            project_root,
            ..Default::default()
        };
        let evolve_engine =
            crate::core::self_evolve::SpecialistPromptEvolutionEngine::new(config, llm);
        let result = evolve_engine
            .evolve_specialist_prompt_bundle(request, current_specialist_raw.as_deref())
            .await?;

        let mut promotion_applied = false;
        let mut canary_state: Option<
            crate::core::self_evolve::strategy_runtime::CanaryRolloutState,
        > = None;
        let mut replay_result: Option<
            crate::core::self_evolve::strategy_runtime::ReplayEvaluationResult,
        > = None;
        let mut promoted_directly_to_baseline = false;
        if result.promoted && apply_promotion {
            if let Some(bundle) = result.promoted_specialist_bundle.as_ref() {
                let candidate_serialized = serde_json::to_vec(bundle)?;
                if let Some(existing_baseline) = current_specialist_raw.as_ref() {
                    let _ = self
                        .storage
                        .set(
                            crate::core::self_evolve::SPECIALIST_PROMPT_BUNDLE_BASELINE_SNAPSHOT_KEY,
                            existing_baseline,
                        )
                        .await;
                }
                let baseline_bundle_version = current_specialist_raw
                    .as_ref()
                    .and_then(|raw| {
                        crate::core::self_evolve::specialist_prompt_evolution::parse_specialist_prompt_bundle_profile(raw)
                            .map(|bundle| bundle.version)
                    })
                    .unwrap_or_else(|| result.baseline_version.clone());
                let baseline_version =
                    crate::core::self_evolve::specialist_prompt_evolution::compose_specialist_prompt_version(
                        &baseline_bundle_version,
                    );
                let candidate_version =
                    crate::core::self_evolve::specialist_prompt_evolution::compose_specialist_prompt_version(
                        &result.candidate_version,
                    );

                self.storage
                    .set(
                        crate::core::self_evolve::SPECIALIST_PROMPT_BUNDLE_PROFILE_CANARY_KEY,
                        &candidate_serialized,
                    )
                    .await?;
                let state = crate::core::self_evolve::strategy_runtime::CanaryRolloutState {
                    enabled: true,
                    baseline_version: baseline_version.clone(),
                    candidate_version: candidate_version.clone(),
                    rollout_percent: canary_rollout_percent,
                    min_samples_per_version: canary_min_samples_per_version,
                    min_success_gain: canary_min_success_gain,
                    max_sign_test_p_value: canary_max_sign_test_p_value,
                    activated_at: Some(chrono::Utc::now().to_rfc3339()),
                };
                let state_bytes = serde_json::to_vec(&state)?;
                self.storage
                    .set(
                        crate::core::self_evolve::SPECIALIST_PROMPT_BUNDLE_CANARY_STATE_KEY,
                        &state_bytes,
                    )
                    .await?;
                canary_state = Some(state.clone());

                if let Ok(runs) = self
                    .storage
                    .list_recent_experience_runs_any_scope(replay_log_limit)
                    .await
                {
                    let replay_eval = crate::core::self_evolve::strategy_runtime::evaluate_experience_canary_by_metadata_version(
                        &runs,
                        "specialist_prompt_version",
                        &state.baseline_version,
                        &state.candidate_version,
                        state.min_samples_per_version,
                        state.min_success_gain,
                        state.max_sign_test_p_value,
                    );
                    if replay_eval.promote {
                        self.storage
                            .set(
                                crate::core::self_evolve::SPECIALIST_PROMPT_BUNDLE_PROFILE_KEY,
                                &candidate_serialized,
                            )
                            .await?;
                        let mut disabled_state = state.clone();
                        disabled_state.enabled = false;
                        let disabled_bytes = serde_json::to_vec(&disabled_state)?;
                        self.storage
                            .set(
                                crate::core::self_evolve::SPECIALIST_PROMPT_BUNDLE_CANARY_STATE_KEY,
                                &disabled_bytes,
                            )
                            .await?;
                        promoted_directly_to_baseline = true;
                        canary_state = Some(disabled_state);
                    }
                    replay_result = Some(replay_eval);
                }
                promotion_applied = true;
            }
        }

        if let Some(tx) = stream_tx {
            let status_msg = if result.promoted {
                if promotion_applied {
                    if promoted_directly_to_baseline {
                        format!(
                            "Specialist prompt evolution complete: promoted candidate (gain {:.4}, p={:.4}), replay gate passed, baseline updated immediately",
                            result.score_gain, result.p_value
                        )
                    } else {
                        format!(
                            "Specialist prompt evolution complete: promoted candidate (gain {:.4}, p={:.4}) activated in canary mode ({}%)",
                            result.score_gain,
                            result.p_value,
                            canary_state
                                .as_ref()
                                .map(|s| s.rollout_percent)
                                .unwrap_or(canary_rollout_percent)
                        )
                    }
                } else {
                    format!(
                        "Specialist prompt evolution complete: candidate passed promotion gate (gain {:.4}, p={:.4}) but not applied",
                        result.score_gain, result.p_value
                    )
                }
            } else {
                format!(
                    "Specialist prompt evolution complete: {}",
                    result.promotion_gate_summary
                )
            };
            queue_stream_event(
                tx,
                StreamEvent::ToolResult {
                    name: "self_evolve".to_string(),
                    content: status_msg,
                },
            );
        }

        push_trace_step(
            trace_ref,
            if result.success && result.promoted {
                "[evolve]"
            } else if result.success {
                "[info]"
            } else {
                "[error]"
            },
            "Specialist Prompt Evolution Evaluated",
            if result.success {
                format!(
                    "Evaluated {} specialist prompt candidates. Score {:.0}% -> {:.0}% with gate: {}",
                    result.evaluated_candidates,
                    result.baseline_score * 100.0,
                    result.best_candidate_score * 100.0,
                    result.promotion_gate_summary
                )
            } else {
                format!(
                    "Specialist prompt evolution failed: {}",
                    result.error.as_deref().unwrap_or("unknown error")
                )
            },
            if result.success && result.promoted {
                "success"
            } else if result.success {
                "info"
            } else {
                "error"
            },
            Some(serde_json::json!({
                "trace_kind": "self_evolve.specialist_prompt.result",
                "request": request,
                "mode": "specialist_prompt",
                "success": result.success,
                "promoted": result.promoted,
                "evaluated_candidates": result.evaluated_candidates,
                "baseline_version": result.baseline_version.clone(),
                "candidate_version": result.candidate_version.clone(),
                "baseline_score": result.baseline_score,
                "best_candidate_score": result.best_candidate_score,
                "score_gain": result.score_gain,
                "wins": result.wins,
                "losses": result.losses,
                "p_value": result.p_value,
                "candidate_source": result.candidate_source.clone(),
                "optimized_surfaces": result.optimized_surfaces.clone(),
                "promotion_gate": result.promotion_gate.clone(),
                "promotion_gate_summary": result.promotion_gate_summary.clone(),
                "promotion_gate_report": result.promotion_gate_report.clone(),
                "lineage_entry_id": result.lineage_entry_id.clone(),
                "lineage_archive_path": result.lineage_archive_path.clone(),
                "notes": result.notes.clone(),
                "diff_summary": result.diff_summary.clone(),
                "promoted_specialist_bundle": result.promoted_specialist_bundle.clone(),
                "error": result.error.clone(),
            })),
            Some(specialist_start.elapsed().as_millis() as u64),
        )
        .await;

        let promotion_mode = if promoted_directly_to_baseline {
            "baseline"
        } else if promotion_applied {
            "canary"
        } else {
            "none"
        };
        push_trace_step(
            trace_ref,
            if promotion_applied { "[ok]" } else { "[info]" },
            "Specialist Prompt Promotion Decision",
            if promoted_directly_to_baseline {
                "Replay evaluation promoted the specialist prompt bundle directly to baseline."
                    .to_string()
            } else if promotion_applied {
                format!(
                    "Specialist prompt candidate activated in canary mode at {}% rollout.",
                    canary_state
                        .as_ref()
                        .map(|state| state.rollout_percent)
                        .unwrap_or(canary_rollout_percent)
                )
            } else if result.promoted {
                "Specialist prompt candidate passed the promotion gate but was not applied."
                    .to_string()
            } else {
                format!(
                    "No specialist prompt promotion applied. {}",
                    result.promotion_gate_summary
                )
            },
            if promotion_applied { "success" } else { "info" },
            Some(serde_json::json!({
                "trace_kind": "self_evolve.specialist_prompt.promotion",
                "request": request,
                "promotion_applied": promotion_applied,
                "apply_promotion_requested": apply_promotion,
                "promotion_mode": promotion_mode,
                "promoted_directly_to_baseline": promoted_directly_to_baseline,
                "baseline_version": result.baseline_version.clone(),
                "candidate_version": result.candidate_version.clone(),
                "optimized_surfaces": result.optimized_surfaces.clone(),
                "diff_summary": result.diff_summary.clone(),
                "canary_state": canary_state.clone(),
                "replay_evaluation": replay_result.clone(),
            })),
            None,
        )
        .await;

        let mut value = serde_json::to_value(&result)?;
        if let serde_json::Value::Object(obj) = &mut value {
            obj.insert("mode".to_string(), serde_json::json!("specialist_prompt"));
            obj.insert(
                "promotion_applied".to_string(),
                serde_json::json!(promotion_applied),
            );
            obj.insert(
                "apply_promotion_requested".to_string(),
                serde_json::json!(apply_promotion),
            );
            obj.insert(
                "promotion_mode".to_string(),
                serde_json::json!(promotion_mode),
            );
            obj.insert(
                "canary_state".to_string(),
                serde_json::to_value(&canary_state).unwrap_or(serde_json::Value::Null),
            );
            obj.insert(
                "replay_evaluation".to_string(),
                serde_json::to_value(&replay_result).unwrap_or(serde_json::Value::Null),
            );
        }
        if let Ok(last_bytes) = serde_json::to_vec(&value) {
            let _ = self
                .storage
                .set(
                    crate::core::self_evolve::SPECIALIST_PROMPT_BUNDLE_LAST_RESULT_KEY,
                    &last_bytes,
                )
                .await;
        }

        if result.success {
            if result.promoted {
                let mode_label = if promoted_directly_to_baseline {
                    "applied immediately"
                } else if promotion_applied {
                    "activated in canary mode for gradual rollout"
                } else {
                    "ready but not yet applied"
                };
                Ok(format!(
                    "Specialist prompt evolution completed successfully.\n\n\
                    I evaluated {} specialist-role prompt bundles and found an improvement.\n\
                    - Combined benchmark score improved from {:.0}% to {:.0}% ({} wins, {} losses)\n\
                    - The improved specialist-role prompt bundle has been {}\n\n\
                    {} is now testing sharper specialist prompts for delegated agents.",
                    result.evaluated_candidates,
                    result.baseline_score * 100.0,
                    result.best_candidate_score * 100.0,
                    result.wins,
                    result.losses,
                    mode_label,
                    crate::branding::PRODUCT_NAME
                ))
            } else {
                Ok(format!(
                    "Specialist prompt evolution completed. I evaluated {} specialist-role prompt bundles but none outperformed the current specialist bundle (score: {:.0}%). No changes were made.",
                    result.evaluated_candidates,
                    result.baseline_score * 100.0,
                ))
            }
        } else {
            Ok(format!(
                "Specialist prompt evolution ran but encountered an issue: {}",
                result.error.as_deref().unwrap_or("unknown error")
            ))
        }
    }

    /// Determine the project root (where Cargo.toml lives).
    fn find_project_root(&self) -> std::path::PathBuf {
        // In Docker, the app is at /app
        let app_path = std::path::Path::new("/app");
        if app_path.join("Cargo.toml").exists() {
            return app_path.to_path_buf();
        }
        // In development, walk up from current dir
        if let Ok(cwd) = std::env::current_dir() {
            let mut dir = cwd.as_path();
            loop {
                if dir.join("Cargo.toml").exists() {
                    return dir.to_path_buf();
                }
                match dir.parent() {
                    Some(parent) => dir = parent,
                    None => break,
                }
            }
        }
        // Fallback
        std::path::PathBuf::from(".")
    }

    /// Legacy monolithic tool execution path. New dispatchers route through
    /// modular handlers and can gradually replace this implementation.
    ///
    /// `repair_context` carries the user message + routing intent + active
    /// turn-plan goals so the per-call argument validator can semantically
    /// infer missing required fields via `infer_missing_action_arguments`.
    /// `repair_memo` lives the lifetime of one user turn (created in
    /// `agent_loop.rs` before its iteration loop) and short-circuits
    /// identical retries within the turn.
    pub(crate) async fn execute_tool_calls_legacy(
        &self,
        response: &crate::core::llm::LlmResponse,
        trace_ref: &Arc<RwLock<ExecutionTrace>>,
        stream_tx: Option<tokio::sync::mpsc::Sender<StreamEvent>>,
        request_channel: &str,
        conversation_id: Option<&str>,
        project_id: Option<&str>,
        authorization: Option<&crate::actions::ActionAuthorizationContext>,
        repair_context: &super::argument_repair::ArgumentRepairContext,
        repair_memo: &mut super::argument_repair::RepairMemo,
        repair_iteration: usize,
        repair_convergence_counter: &mut HashMap<String, u32>,
        repair_clarification: &mut Option<super::argument_repair::ArgumentRepairClarification>,
        tool_start_contexts: &HashMap<String, serde_json::Value>,
    ) -> Result<String> {
        if response.tool_calls.is_empty() {
            return Ok(response.content.clone());
        }

        let mut results = Vec::new();
        let last_conversation_id = self.last_conversation_id.read().await.clone();
        let conversation_id = conversation_id
            .filter(|value| !value.trim().is_empty())
            .or_else(|| {
                last_conversation_id
                    .as_deref()
                    .filter(|value| !value.trim().is_empty())
            });
        let project_id = project_id.filter(|value| !value.trim().is_empty());
        let sanitize_stream = |s: &str| -> String { self.sanitize_stream_preview(s) };
        let public_base_url = self.load_public_base_url().await;
        let integration_aliases = self.load_tool_integration_aliases().await;
        let absolutize_url =
            |url: &str| -> String { Self::absolutize_public_url(public_base_url.as_deref(), url) };

        // Deduplicate repeated tool calls (same name + identical args) so app_deploy
        // and other side-effecting actions do not run twice from merged paths.
        let mut seen_signatures: HashSet<String> = HashSet::new();
        let mut unique_calls: Vec<&crate::core::llm::ToolCall> = Vec::new();
        for call in &response.tool_calls {
            let sig = Self::tool_call_signature(call);
            if seen_signatures.insert(sig) {
                unique_calls.push(call);
            }
        }

        let enabled_action_map = self
            .runtime
            .list_enabled_actions()
            .await
            .unwrap_or_default()
            .into_iter()
            .map(|action| (action.name.clone(), action))
            .collect::<HashMap<_, _>>();

        for call in unique_calls {
            let mut call = call.clone();
            let action_for_call = enabled_action_map.get(&call.name).cloned();
            if let Some(validation_failure) = self
                .repair_and_validate_tool_call_arguments(
                    &mut call,
                    action_for_call.as_ref(),
                    repair_context,
                    repair_memo,
                    repair_iteration,
                )
                .await
            {
                if !validation_failure.missing_fields.is_empty() {
                    let signature = super::argument_repair::missing_fields_signature(
                        &call.name,
                        &validation_failure.missing_fields,
                    );
                    let count = repair_convergence_counter.entry(signature).or_insert(0);
                    *count = count.saturating_add(1);
                    if *count >= 2 {
                        let clarification = super::argument_repair::ArgumentRepairClarification {
                            action_name: call.name.clone(),
                            missing_fields: validation_failure.missing_fields.clone(),
                            partial_inference: validation_failure.partial_inference.clone(),
                        };
                        let payload = clarification.payload();
                        let payload_text = payload.to_string();
                        if let Some(ref tx) = stream_tx {
                            queue_stream_event(
                                tx,
                                StreamEvent::ToolResult {
                                    name: call.name.clone(),
                                    content: payload_text.clone(),
                                },
                            );
                        }
                        *repair_clarification = Some(clarification);
                        results.push(payload_text);
                        break;
                    }
                }
                if let Some(ref tx) = stream_tx {
                    queue_stream_event(
                        tx,
                        StreamEvent::ToolResult {
                            name: call.name.clone(),
                            content: validation_failure.message.clone(),
                        },
                    );
                }
                results.push(render_tool_completion_marker_with_data(
                    &call.name,
                    "failed",
                    &validation_failure.message,
                    serde_json::json!({
                        "missing_fields": validation_failure.missing_fields,
                        "partial_inference": validation_failure.partial_inference,
                    }),
                ));
                continue;
            }

            if let Some(ref tx) = stream_tx {
                let base_payload = if call.name == "app_deploy" {
                    Some(Self::summarize_app_deploy_stream_payload(&call.arguments))
                } else if call.name == "file_write" {
                    Some(summarize_file_write_stream_payload(&call.arguments))
                } else {
                    None
                };
                let payload = merge_tool_start_payload(
                    base_payload,
                    tool_start_context_for_call(&call, tool_start_contexts),
                );
                queue_stream_event(
                    tx,
                    StreamEvent::ToolStart {
                        name: call.name.clone(),
                        payload,
                    },
                );
            }

            // Check safety policy
            let allowed =
                Self::legacy_tool_call_allowed_by_safety(&self.safety, &call, authorization)
                    .await?;
            if !allowed {
                let blocked = format!("Tool '{}' blocked by safety policy", call.name);
                if let Some(ref tx) = stream_tx {
                    queue_stream_event(
                        tx,
                        StreamEvent::ToolResult {
                            name: call.name.clone(),
                            content: blocked.clone(),
                        },
                    );
                }
                results.push(blocked);
                continue;
            }

            // Handle generate_image via integrations (not runtime)
            if call.name == "generate_image" {
                // Inject configured model if not specified in the call
                let mut args = call.arguments.clone();
                if args.get("model").and_then(|v| v.as_str()).is_none() {
                    if let Some(ref model) = self.config.media_gen.image_model {
                        args["model"] = serde_json::Value::String(model.clone());
                    }
                }
                match self
                    .integrations
                    .execute("media_gen", "generate_image", &args)
                    .await
                {
                    Ok(result) => {
                        if let Some(url) = result.get("url").and_then(|v| v.as_str()) {
                            let provider = result
                                .get("provider")
                                .and_then(|v| v.as_str())
                                .unwrap_or("unknown");
                            let time_ms = result
                                .get("generation_time_ms")
                                .and_then(|v| v.as_u64())
                                .unwrap_or(0);
                            let formatted = format!(
                                "[IMAGE_RESULT]{}\n[/IMAGE_RESULT]\n*Generated by {} in {}ms*",
                                url, provider, time_ms
                            );
                            if let Some(ref tx) = stream_tx {
                                queue_stream_event(
                                    tx,
                                    StreamEvent::ToolResult {
                                        name: call.name.clone(),
                                        content: format!(
                                            "Generated image via {} ({}ms)",
                                            provider, time_ms
                                        ),
                                    },
                                );
                            }
                            results.push(formatted);
                        } else {
                            let formatted = format!("Image generated: {}", result);
                            if let Some(ref tx) = stream_tx {
                                queue_stream_event(
                                    tx,
                                    StreamEvent::ToolResult {
                                        name: call.name.clone(),
                                        content: sanitize_stream(&formatted),
                                    },
                                );
                            }
                            results.push(formatted);
                        }
                    }
                    Err(e) => {
                        tracing::error!("Image generation error: {}", e);
                        let formatted = format!("Error generating image: {}", e);
                        if let Some(ref tx) = stream_tx {
                            queue_stream_event(
                                tx,
                                StreamEvent::ToolResult {
                                    name: call.name.clone(),
                                    content: formatted.clone(),
                                },
                            );
                        }
                        results.push(formatted);
                    }
                }
                continue;
            }

            // Handle provider-based video generation via integrations (not runtime)
            if call.name == "generate_video" {
                match self
                    .integrations
                    .execute("media_gen", "generate_video", &call.arguments)
                    .await
                {
                    Ok(result) => {
                        if let Some(url) = result.get("url").and_then(|v| v.as_str()) {
                            let provider = result
                                .get("provider")
                                .and_then(|v| v.as_str())
                                .unwrap_or("unknown");
                            let model = result
                                .get("model")
                                .and_then(|v| v.as_str())
                                .unwrap_or("default");
                            let mut source_url = url.to_string();
                            let mut video_bytes: Option<Vec<u8>> = None;

                            // Convert data URLs into persisted output files so links remain usable.
                            if source_url.starts_with("data:") {
                                match self.load_video_bytes(&source_url, 80 * 1024 * 1024).await {
                                    Ok(bytes) => {
                                        video_bytes = Some(bytes.clone());
                                        match self
                                            .persist_output_binary("provider_video", "mp4", &bytes)
                                            .await
                                        {
                                            Ok(local_url) => source_url = local_url,
                                            Err(e) => tracing::warn!(
                                                "Failed to persist provider data URL video: {}",
                                                e
                                            ),
                                        }
                                    }
                                    Err(e) => {
                                        tracing::warn!(
                                            "Failed to decode provider data URL video: {}",
                                            e
                                        );
                                    }
                                }
                            }

                            let rendered_url = absolutize_url(&source_url);
                            // Direct attachment where reliable: Telegram and WhatsApp(Baileys).
                            let mut delivered_to_channel = false;
                            let whatsapp_baileys = self
                                .config
                                .whatsapp
                                .as_ref()
                                .map(|cfg| {
                                    matches!(
                                        cfg.mode,
                                        crate::channels::whatsapp::WhatsAppMode::Baileys
                                    )
                                })
                                .unwrap_or(false);
                            let should_direct_send = request_channel == "telegram"
                                || (request_channel == "whatsapp" && whatsapp_baileys);

                            if should_direct_send {
                                if video_bytes.is_none() {
                                    if let Ok(bytes) =
                                        self.load_video_bytes(&source_url, 80 * 1024 * 1024).await
                                    {
                                        video_bytes = Some(bytes);
                                    }
                                }
                                if let Some(bytes) = video_bytes.as_ref() {
                                    let caption =
                                        format!("Video generated by {} ({})", provider, model);
                                    if crate::channels::send_video_to_channel(
                                        self,
                                        request_channel,
                                        bytes,
                                        &caption,
                                        Some(&rendered_url),
                                    )
                                    .await
                                    .is_ok()
                                    {
                                        delivered_to_channel = true;
                                    }
                                }
                            }

                            let formatted = if matches!(request_channel, "telegram" | "whatsapp") {
                                if delivered_to_channel {
                                    format!("Video sent to this chat.\nDownload: {}", rendered_url)
                                } else {
                                    format!(
                                        "Video generated via {} ({}): {}",
                                        provider, model, rendered_url
                                    )
                                }
                            } else {
                                format!(
                                    "[VIDEO_RESULT]{}\n[/VIDEO_RESULT]\n*Generated by {} ({})*",
                                    rendered_url, provider, model
                                )
                            };
                            if let Some(ref tx) = stream_tx {
                                queue_stream_event(
                                    tx,
                                    StreamEvent::ToolResult {
                                        name: call.name.clone(),
                                        content: format!("Generated video via {}", provider),
                                    },
                                );
                            }
                            results.push(formatted);
                        } else {
                            let formatted = format!("Video generated: {}", result);
                            if let Some(ref tx) = stream_tx {
                                queue_stream_event(
                                    tx,
                                    StreamEvent::ToolResult {
                                        name: call.name.clone(),
                                        content: sanitize_stream(&formatted),
                                    },
                                );
                            }
                            results.push(formatted);
                        }
                    }
                    Err(e) => {
                        tracing::error!("Provider video generation error: {}", e);
                        let formatted = format!("Error generating video: {}", e);
                        if let Some(ref tx) = stream_tx {
                            queue_stream_event(
                                tx,
                                StreamEvent::ToolResult {
                                    name: call.name.clone(),
                                    content: formatted.clone(),
                                },
                            );
                        }
                        results.push(formatted);
                    }
                }
                continue;
            }

            // Handle browser automation - starts a background session
            if call.name == "browser_auto" {
                let sub_action = call
                    .arguments
                    .get("action")
                    .and_then(|v| v.as_str())
                    .unwrap_or("start_session");

                if sub_action == "start_session" {
                    let task_desc = call
                        .arguments
                        .get("task")
                        .and_then(|v| v.as_str())
                        .unwrap_or("Browse the web");
                    let channel = call
                        .arguments
                        .get("channel")
                        .and_then(|v| v.as_str())
                        .unwrap_or("web");

                    if !self.browser_sessions.is_available().await {
                        tracing::warn!(
                            "Browser automation unavailable: Playwright sidecar not reachable"
                        );
                        let formatted = r#"{"error": "browser_unavailable", "detail": "Playwright sidecar is not running"}"#.to_string();
                        if let Some(ref tx) = stream_tx {
                            queue_stream_event(
                                tx,
                                StreamEvent::ToolResult {
                                    name: call.name.clone(),
                                    content: formatted.clone(),
                                },
                            );
                        }
                        results.push(formatted);
                        continue;
                    }

                    // Create a notification callback that sends messages to the user's channel
                    let chat_id = call
                        .arguments
                        .get("chat_id")
                        .and_then(|v| v.as_str())
                        .unwrap_or("")
                        .to_string();
                    let notify_channel = channel.to_string();
                    let agent_config = self.config.clone();
                    let storage_clone = self.storage.clone();
                    let encrypted_storage_clone = self.encrypted_storage.clone();
                    let notification_store = self.notification_store();
                    let local_ui_base = Self::user_facing_local_base_url();
                    let public_ui_base = self.load_public_base_url().await;
                    let notify_conversation_id = call
                        .arguments
                        .get("conversation_id")
                        .and_then(|v| v.as_str())
                        .map(str::trim)
                        .filter(|s| !s.is_empty())
                        .map(str::to_string)
                        .or_else(|| {
                            conversation_id
                                .map(str::trim)
                                .filter(|s| !s.is_empty())
                                .map(str::to_string)
                        });
                    let browser_session_chat_binding =
                        notify_conversation_id.clone().or_else(|| {
                            (!chat_id.trim().is_empty())
                                .then(|| format!("{}:{}", notify_channel, chat_id))
                        });
                    let notify_fn: std::sync::Arc<
                        dyn Fn(crate::core::browser_session::BrowserSessionNotification)
                            + Send
                            + Sync,
                    > = std::sync::Arc::new(move |notification| {
                        let config = agent_config.clone();
                        let channel = notify_channel.clone();
                        let chat_id = chat_id.clone();
                        let storage = storage_clone.clone();
                        let encrypted_storage = encrypted_storage_clone.clone();
                        let notification_store = notification_store.clone();
                        let conversation_id = notify_conversation_id.clone();
                        let local_ui_base = local_ui_base.clone();
                        let public_ui_base = public_ui_base.clone();
                        let session_id = notification.session_id.clone();
                        let msg = notification.message.clone();
                        let _screenshot = notification.screenshot.clone(); // screenshots sent via channel-specific methods
                        crate::spawn_logged!("src/core/agent/tool_execution.rs:9568", async move {
                            let handoff_relative = format!("/ui/browser-handoff/{}", session_id);
                            let absolutize = |base: &str| -> String {
                                if handoff_relative.starts_with('/') {
                                    format!("{}{}", base.trim_end_matches('/'), handoff_relative)
                                } else {
                                    format!("{}/{}", base.trim_end_matches('/'), handoff_relative)
                                }
                            };
                            let local_handoff_url = absolutize(&local_ui_base);
                            let public_handoff_url =
                                public_ui_base.as_ref().map(|base| absolutize(base));
                            let conversation_handoff_message = if let Some(public_url) =
                                public_handoff_url.as_ref()
                            {
                                if public_url != &local_handoff_url {
                                    format!(
                                        "{}\nOpen live handoff: [Open browser handoff]({})\nLocal fallback: [Open local handoff]({})",
                                        msg, public_url, local_handoff_url
                                    )
                                } else {
                                    format!(
                                        "{}\nOpen live handoff: [Open browser handoff]({})",
                                        msg, public_url
                                    )
                                }
                            } else {
                                format!(
                                    "{}\nOpen live handoff: [Open browser handoff]({})",
                                    msg, local_handoff_url
                                )
                            };
                            let delivery_message = match notification.kind {
                                    crate::core::browser_session::BrowserSessionNotificationKind::NeedsInput => {
                                        if let Some(public_url) = public_handoff_url.as_ref() {
                                            if public_url != &local_handoff_url {
                                                format!(
                                                    "{}\nOpen live handoff: {}\nLocal fallback: {}",
                                                    msg, public_url, local_handoff_url
                                                )
                                            } else {
                                                format!("{}\nOpen live handoff: {}", msg, public_url)
                                            }
                                        } else {
                                            format!(
                                                "{}\nOpen live handoff: {}",
                                                msg, local_handoff_url
                                            )
                                        }
                                    }
                                    _ => msg.clone(),
                                };
                            if matches!(
                                notification.kind,
                                crate::core::browser_session::BrowserSessionNotificationKind::Failed
                                    | crate::core::browser_session::BrowserSessionNotificationKind::Closed
                            ) {
                                let notif = crate::storage::entities::notification::Model {
                                    id: uuid::Uuid::new_v4().to_string(),
                                    title: "Browser Automation".to_string(),
                                    body: delivery_message.clone(),
                                    level: "warning".to_string(),
                                    source: "browser".to_string(),
                                    read: false,
                                    created_at: chrono::Utc::now().to_rfc3339(),
                                };
                                let _ = storage.insert_notification(&notif).await;
                            }

                            // Also append to conversation so browser prompts are visible in chat thread.
                            if let Some(cid) = conversation_id.as_deref() {
                                let body = match notification.kind {
                                        crate::core::browser_session::BrowserSessionNotificationKind::NeedsInput => Some(
                                            format!(
                                                "[Browser automation] {}",
                                                conversation_handoff_message
                                            ),
                                        ),
                                        crate::core::browser_session::BrowserSessionNotificationKind::Notice
                                        | crate::core::browser_session::BrowserSessionNotificationKind::Completed
                                        | crate::core::browser_session::BrowserSessionNotificationKind::Closed
                                        | crate::core::browser_session::BrowserSessionNotificationKind::Failed => {
                                            Some(format!("[Browser automation] {}", delivery_message))
                                        }
                                        crate::core::browser_session::BrowserSessionNotificationKind::Progress => None,
                                    };
                                if let Some(body) = body {
                                    let asst_msg = crate::storage::entities::message::Model {
                                        id: uuid::Uuid::new_v4().to_string(),
                                        conversation_id: cid.to_string(),
                                        role: "assistant".to_string(),
                                        content: body,
                                        timestamp: chrono::Utc::now().to_rfc3339(),
                                        model_used: Some("browser_auto".to_string()),
                                        trace_id: None,
                                    };
                                    if encrypted_storage
                                        .insert_message_encrypted(&asst_msg)
                                        .await
                                        .is_ok()
                                    {
                                        notification_store.broadcast_event(
                                            crate::core::agent::NotificationEvent {
                                                kind: "conversation.changed".to_string(),
                                                id: asst_msg.id.clone(),
                                                title: "Conversation updated".to_string(),
                                                body: cid.to_string(),
                                                level: "info".to_string(),
                                                source: "browser".to_string(),
                                                read: true,
                                                created_at: asst_msg.timestamp.clone(),
                                            },
                                        );
                                    }
                                }
                            }

                            // Send to Telegram if configured
                            #[cfg(feature = "telegram")]
                                if channel == "telegram"
                                    && !matches!(
                                        notification.kind,
                                        crate::core::browser_session::BrowserSessionNotificationKind::Progress
                                    )
                                {
                                    if let Some(tg) = &config.telegram {
                                        if !tg.bot_token.is_empty() {
                                            let target = if !chat_id.is_empty() {
                                                chat_id.parse::<i64>().unwrap_or(0)
                                            } else if let Some(first) = tg.allowed_users.first() {
                                                *first
                                            } else {
                                                0
                                            };
                                            if target != 0 {
                                                use teloxide::requests::Requester;
                                                let bot = teloxide::Bot::new(&tg.bot_token);
                                                let _ = bot
                                                    .send_message(
                                                        teloxide::types::ChatId(target),
                                                        &delivery_message,
                                                    )
                                                    .await;
                                            }
                                        }
                                    }
                                }
                            let _ = channel; // suppress unused warning on non-telegram builds
                        });
                    });

                    let llm_clone = self.llm.clone();
                    match self
                        .browser_sessions
                        .start_session(
                            task_desc,
                            channel,
                            browser_session_chat_binding.as_deref(),
                            llm_clone,
                            notify_fn,
                        )
                        .await
                    {
                        Ok(started) => {
                            let session_id = started.session_id;
                            tracing::info!(
                                "Browser session started: session={}, task_len={}, reused={}",
                                &session_id[..8],
                                task_desc.len(),
                                started.reused_existing
                            );
                            // Return structured data - let the LLM craft the user message
                            if let Some(ref tx) = stream_tx {
                                queue_stream_event(
                                    tx,
                                    StreamEvent::ToolResult {
                                        name: call.name.clone(),
                                        content: format!(
                                            "Browser session started: {}",
                                            &session_id[..8]
                                        ),
                                    },
                                );
                            }
                            results.push(format!(
                                r#"{{"status": "session_started", "session_id": "{}", "task": "{}", "reused": {}}}"#,
                                session_id, task_desc.replace('"', "'"), started.reused_existing
                            ));
                        }
                        Err(e) => {
                            tracing::error!("Browser session start failed: error={}", e);
                            let is_limit_error = e
                                .to_string()
                                .contains("Maximum 2 concurrent browser sessions");
                            if let Some(ref tx) = stream_tx {
                                queue_stream_event(
                                    tx,
                                    StreamEvent::ToolResult {
                                        name: call.name.clone(),
                                        content: format!("Browser session start failed: {}", e),
                                    },
                                );
                            }
                            if is_limit_error {
                                results.push(
                                    r#"{"error": "session_limit", "detail": "Maximum 2 concurrent browser sessions"}"#
                                        .to_string(),
                                );
                            } else {
                                results.push(format!(
                                    r#"{{"error": "session_start_failed", "detail": "{}"}}"#,
                                    e
                                ));
                            }
                        }
                    }
                } else {
                    let formatted = serde_json::json!({
                        "error": "unsupported_browser_auto_action",
                        "requested_action": sub_action,
                        "detail": "browser_auto only supports start_session. It runs a managed background browser session that handles navigation and live handoff automatically. For explicit manual browser control, use the browser integration tool with create_session/navigate/click/type_text."
                    })
                    .to_string();
                    if let Some(ref tx) = stream_tx {
                        queue_stream_event(
                            tx,
                            StreamEvent::ToolResult {
                                name: call.name.clone(),
                                content: sanitize_stream(&formatted),
                            },
                        );
                    }
                    results.push(formatted);
                }
                continue;
            }

            if call.name == "self_evolve" {
                match self
                    .handle_self_evolve_tool_call(&call, trace_ref, stream_tx.as_ref())
                    .await
                {
                    Ok(result) => {
                        if let Some(ref tx) = stream_tx {
                            queue_stream_event(
                                tx,
                                StreamEvent::ToolResult {
                                    name: call.name.clone(),
                                    content: sanitize_stream(&result),
                                },
                            );
                        }
                        results.push(result);
                    }
                    Err(error) => {
                        tracing::error!("Self-evolve execution error: {}", error);
                        let formatted = format!("Error executing '{}': {}", call.name, error);
                        if let Some(ref tx) = stream_tx {
                            queue_stream_event(
                                tx,
                                StreamEvent::ToolResult {
                                    name: call.name.clone(),
                                    content: formatted.clone(),
                                },
                            );
                        }
                        results.push(formatted);
                    }
                }
                continue;
            }

            // Handle integration-backed tools via dynamic routing aliases + direct integration IDs.
            if let Some(integration_id) =
                self.resolve_tool_integration_id(&call.name, &integration_aliases)
            {
                let formatted = self
                    .execute_integration_tool_call(
                        &call,
                        trace_ref,
                        stream_tx.as_ref(),
                        request_channel,
                        &integration_id,
                    )
                    .await;
                results.push(formatted);
                continue;
            }

            if matches!(
                call.name.as_str(),
                "app_restart" | "app_stop" | "app_delete"
            ) {
                let result = match call.name.as_str() {
                    "app_restart" => {
                        self.handle_app_restart_tool_call(
                            &call,
                            stream_tx.as_ref(),
                            request_channel,
                            conversation_id,
                        )
                        .await
                    }
                    "app_stop" => {
                        self.handle_app_stop_tool_call(&call, stream_tx.as_ref(), request_channel)
                            .await
                    }
                    "app_delete" => {
                        self.handle_app_delete_tool_call(&call, stream_tx.as_ref(), request_channel)
                            .await
                    }
                    _ => unreachable!(),
                };
                match result {
                    Ok(formatted) => results.push(formatted),
                    Err(error) => {
                        tracing::error!("App management tool '{}' failed: {}", call.name, error);
                        let formatted = format!("Error executing '{}': {}", call.name, error);
                        if let Some(ref tx) = stream_tx {
                            queue_stream_event(
                                tx,
                                StreamEvent::ToolResult {
                                    name: call.name.clone(),
                                    content: formatted.clone(),
                                },
                            );
                        }
                        results.push(formatted);
                    }
                }
                continue;
            }

            // Handle app deployment - needs AppRegistry from agent
            if call.name == "app_deploy" {
                let normalized_args = Self::normalize_app_deploy_arguments(&call.arguments);
                // Resolve secret placeholders for deployment-time env injection, without mutating
                // the original tool arguments (so traces stay safe).
                let mut resolved_args = self
                    .runtime
                    .resolve_secret_placeholders(&call.name, &normalized_args)
                    .unwrap_or(normalized_args);
                if resolved_args
                    .get("access_password")
                    .or_else(|| resolved_args.get("access_key"))
                    .and_then(|value| value.as_str())
                    .map(str::trim)
                    .is_some_and(|value| !value.is_empty())
                {
                    if let Some(obj) = resolved_args.as_object_mut() {
                        obj.insert("access_guard".to_string(), serde_json::json!(true));
                    }
                }
                if resolved_args
                    .get("expose_public")
                    .and_then(|v| v.as_bool())
                    .is_none()
                {
                    if let Some(obj) = resolved_args.as_object_mut() {
                        obj.insert("expose_public".to_string(), serde_json::json!(false));
                    }
                }
                let expose_public_requested = resolved_args
                    .get("expose_public")
                    .and_then(|v| v.as_bool())
                    .unwrap_or(false);
                if expose_public_requested {
                    if let Some(obj) = resolved_args.as_object_mut() {
                        obj.insert("access_guard".to_string(), serde_json::json!(true));
                    }
                } else if resolved_args
                    .get("access_guard")
                    .and_then(|v| v.as_bool())
                    .is_none()
                {
                    if let Some(obj) = resolved_args.as_object_mut() {
                        obj.insert("access_guard".to_string(), serde_json::json!(false));
                    }
                }
                if let Some(cid) = conversation_id
                    .map(str::trim)
                    .filter(|value| !value.is_empty())
                {
                    if let Some(obj) = resolved_args.as_object_mut() {
                        obj.entry("_conversation_id".to_string())
                            .or_insert_with(|| serde_json::json!(cid));
                    }
                }
                let patch_update_without_target = resolved_args
                    .get("app_id")
                    .and_then(|v| v.as_str())
                    .map(str::trim)
                    .map(|value| value.is_empty())
                    .unwrap_or(true)
                    && resolved_args
                        .get("mode")
                        .and_then(|v| v.as_str())
                        .map(str::trim)
                        .is_some_and(|value| value.eq_ignore_ascii_case("patch"))
                    && resolved_args
                        .get("file_patches")
                        .and_then(|v| v.as_array())
                        .is_some_and(|items| !items.is_empty());
                if patch_update_without_target {
                    if let Some(cid) = conversation_id
                        .map(str::trim)
                        .filter(|value| !value.is_empty())
                    {
                        if let Some(recent_app) = self
                            .load_recent_artifact_context(cid)
                            .await
                            .filter(|artifact| artifact.artifact_type.eq_ignore_ascii_case("app"))
                        {
                            let app_id = recent_app.artifact_id.trim();
                            if !app_id.is_empty() {
                                if let Some(obj) = resolved_args.as_object_mut() {
                                    obj.insert("app_id".to_string(), serde_json::json!(app_id));
                                }
                            }
                        }
                    }
                }
                let allow_duplicate_requested = resolved_args
                    .get("allow_duplicate")
                    .and_then(|v| v.as_bool())
                    .unwrap_or(false);
                let targeted_existing_app_id = resolved_args
                    .get("app_id")
                    .and_then(|v| v.as_str())
                    .map(str::trim)
                    .filter(|value| !value.is_empty())
                    .map(ToString::to_string);
                let expose_public_requested = resolved_args
                    .get("expose_public")
                    .and_then(|v| v.as_bool())
                    .unwrap_or(false);
                let replace_existing_requested = resolved_args
                    .get("replace_existing")
                    .and_then(|v| v.as_bool())
                    .unwrap_or(false);
                if targeted_existing_app_id.is_none()
                    && !replace_existing_requested
                    && !allow_duplicate_requested
                {
                    if let Some(duplicate_match) =
                        self.find_existing_duplicate_app(&resolved_args).await
                    {
                        let existing = &duplicate_match.app;
                        let existing_id = existing
                            .get("id")
                            .and_then(|v| v.as_str())
                            .map(str::trim)
                            .filter(|id| !id.is_empty())
                            .unwrap_or("app");
                        let existing_id_for_cleanup = existing
                            .get("id")
                            .and_then(|v| v.as_str())
                            .map(str::trim)
                            .filter(|id| !id.is_empty());
                        let existing_title = existing
                            .get("title")
                            .and_then(|v| v.as_str())
                            .unwrap_or("Existing app");
                        let existing_running = existing
                            .get("running")
                            .and_then(|v| v.as_bool())
                            .unwrap_or(false);
                        let existing_url = existing
                            .get("access_url")
                            .and_then(|v| v.as_str())
                            .or_else(|| existing.get("url").and_then(|v| v.as_str()))
                            .unwrap_or("/apps/");
                        let local_base = Self::user_facing_local_base_url();
                        let local_url =
                            Self::absolutize_public_url(Some(local_base.as_str()), existing_url);
                        let public_link_note = if expose_public_requested {
                            Some(
                                "- Public link requested: I will only share it after validation."
                                    .to_string(),
                            )
                        } else {
                            None
                        };
                        let mut duplicate_msg_lines = vec![
                            format!(
                                "Found an existing deployed app for this request: **{}** (`{}`).",
                                existing_title, existing_id
                            ),
                            format!(
                                "- Similarity: {} ({:.0}% confidence, {})",
                                duplicate_match.match_kind,
                                duplicate_match.score * 100.0,
                                duplicate_match.reason
                            ),
                            format!("- Local: {}", local_url),
                        ];
                        if let Some(note) = public_link_note {
                            duplicate_msg_lines.push(note);
                        }

                        match Self::resolve_duplicate_app(
                            duplicate_match.match_kind,
                            existing_running,
                        ) {
                            DuplicateAppResolution::ReuseExisting => {
                                duplicate_msg_lines.push(
                                    "Auto-resolution: reusing the existing deployment (exact file match + healthy runtime)."
                                        .to_string(),
                                );
                                let duplicate_msg = duplicate_msg_lines.join("\n");
                                if let Some(ref tx) = stream_tx {
                                    queue_stream_event(
                                        tx,
                                        StreamEvent::ToolResult {
                                            name: call.name.clone(),
                                            content: duplicate_msg.clone(),
                                        },
                                    );
                                }
                                results.push(duplicate_msg);
                                continue;
                            }
                            DuplicateAppResolution::ReplaceExisting => {
                                let cleanup_note = if let Some(app_id) = existing_id_for_cleanup {
                                    if let Some(obj) = resolved_args.as_object_mut() {
                                        obj.insert("app_id".to_string(), serde_json::json!(app_id));
                                        obj.insert(
                                            "replace_existing".to_string(),
                                            serde_json::json!(true),
                                        );
                                    }
                                    "Auto-resolution: updating the existing deployment in place."
                                        .to_string()
                                } else {
                                    "Auto-resolution: existing app id missing, continuing with redeploy."
                                        .to_string()
                                };
                                if let Some(obj) = resolved_args.as_object_mut() {
                                    obj.insert(
                                        "replace_existing".to_string(),
                                        serde_json::json!(true),
                                    );
                                }
                                duplicate_msg_lines.push(cleanup_note);
                                let duplicate_msg = duplicate_msg_lines.join("\n");
                                if let Some(ref tx) = stream_tx {
                                    queue_stream_event(
                                        tx,
                                        StreamEvent::ToolResult {
                                            name: call.name.clone(),
                                            content: duplicate_msg,
                                        },
                                    );
                                }
                            }
                            DuplicateAppResolution::NeedsClarification => {
                                duplicate_msg_lines.push(
                                    "I found a similar deployed app, but the request did not include a stable app_id. Confirm the app ID to update, or ask for a new separate deployment."
                                        .to_string(),
                                );
                                let duplicate_msg = duplicate_msg_lines.join("\n");
                                if let Some(ref tx) = stream_tx {
                                    queue_stream_event(
                                        tx,
                                        StreamEvent::ToolResult {
                                            name: call.name.clone(),
                                            content: duplicate_msg.clone(),
                                        },
                                    );
                                }
                                results.push(duplicate_msg);
                                continue;
                            }
                        }
                    }
                }
                let hook_event_id = uuid::Uuid::new_v4().to_string();
                let hook_hint = action_message_hint(&resolved_args);
                self.fire_action_hook(
                    crate::hooks::HookTrigger::PreAction,
                    request_channel,
                    &call.name,
                    hook_hint.as_deref(),
                    None,
                    &hook_event_id,
                )
                .await;
                let llm_env = self.app_model_env_vars();
                let mut deploy_args_for_result = resolved_args.clone();
                let mut app_deploy_repair_signatures = std::collections::HashSet::new();
                for _ in 0..3 {
                    let preflight_started = std::time::Instant::now();
                    let preflight_result = crate::actions::app::app_deploy_preflight(
                        &self.data_dir,
                        &deploy_args_for_result,
                        &self.app_registry,
                    )
                    .await;
                    tracing::debug!(
                        target: "agentark.turn_timing",
                        stage = "app_deploy_preflight",
                        duration_ms = preflight_started.elapsed().as_millis() as u64,
                        success = preflight_result.is_ok(),
                        "app deploy preflight timing"
                    );
                    let Err(error) = preflight_result else {
                        break;
                    };
                    let error_text = error.to_string();
                    let signature = serde_json::json!({
                        "error": error_text,
                        "arguments": deploy_args_for_result.clone(),
                    })
                    .to_string();
                    if !app_deploy_repair_signatures.insert(signature) {
                        break;
                    }
                    let Some(repaired_args) = self
                        .repair_app_deploy_arguments_after_validation_error(
                            &deploy_args_for_result,
                            &error_text,
                            repair_context,
                            stream_tx.as_ref(),
                        )
                        .await
                    else {
                        break;
                    };
                    if serde_json::to_string(&repaired_args).ok()
                        == serde_json::to_string(&deploy_args_for_result).ok()
                    {
                        break;
                    }
                    deploy_args_for_result = repaired_args;
                }
                let preflight_started = std::time::Instant::now();
                if let Err(error) = crate::actions::app::app_deploy_preflight(
                    &self.data_dir,
                    &deploy_args_for_result,
                    &self.app_registry,
                )
                .await
                {
                    tracing::debug!(
                        target: "agentark.turn_timing",
                        stage = "app_deploy_preflight_final",
                        duration_ms = preflight_started.elapsed().as_millis() as u64,
                        success = false,
                        error = %safe_truncate(&error.to_string(), 240),
                        "app deploy final preflight failed"
                    );
                    let error_text = error.to_string();
                    let file_inventory = deploy_args_for_result
                        .get("files")
                        .and_then(|value| value.as_object())
                        .map(|files| {
                            let mut names = files.keys().cloned().collect::<Vec<_>>();
                            names.sort();
                            names
                        })
                        .unwrap_or_default();
                    let detail = format!("Error preparing app deployment: {}", error_text);
                    let completion_payload = serde_json::json!({
                        "tool": "app_deploy",
                        "status": "failed",
                        "success": false,
                        "detail": detail.clone(),
                        "error": error_text.clone(),
                        "deploy_attempted": false,
                        "retryable": true,
                        "data": {
                            "retryable": true,
                            "deploy_attempted": false,
                            "file_inventory": file_inventory,
                            "validation_error": error_text,
                        },
                    });
                    let formatted = format!(
                        "{}{}",
                        crate::runtime::TOOL_COMPLETION_MARKER,
                        completion_payload
                    );
                    if let Some(ref tx) = stream_tx {
                        queue_stream_event(
                            tx,
                            StreamEvent::ToolResult {
                                name: call.name.clone(),
                                content: detail,
                            },
                        );
                    }
                    results.push(formatted);
                    continue;
                }
                tracing::debug!(
                    target: "agentark.turn_timing",
                    stage = "app_deploy_preflight_final",
                    duration_ms = preflight_started.elapsed().as_millis() as u64,
                    success = true,
                    "app deploy final preflight timing"
                );
                let deploy_started = std::time::Instant::now();
                let deploy_result = crate::actions::app::app_deploy(
                    &self.config_dir,
                    &self.data_dir,
                    &deploy_args_for_result,
                    &self.app_registry,
                    &llm_env,
                    stream_tx.clone(),
                )
                .await;
                tracing::debug!(
                    target: "agentark.turn_timing",
                    stage = "app_deploy_action_call",
                    duration_ms = deploy_started.elapsed().as_millis() as u64,
                    success = deploy_result.is_ok(),
                    "app deploy action call timing"
                );
                match deploy_result {
                    Ok(result) => {
                        self.trigger_arkpulse_refresh("app_deploy");
                        self.fire_action_hook(
                            crate::hooks::HookTrigger::PostAction,
                            request_channel,
                            &call.name,
                            hook_hint.as_deref(),
                            Some(&result),
                            &hook_event_id,
                        )
                        .await;
                        // Parse result to extract URL for a nice response
                        if let Ok(mut parsed) = serde_json::from_str::<serde_json::Value>(&result) {
                            if parsed
                                .get("deployment_kind")
                                .and_then(|v| v.as_str())
                                .is_some_and(|kind| kind == "repo_bundle")
                            {
                                let title = parsed
                                    .get("title")
                                    .and_then(|v| v.as_str())
                                    .unwrap_or("Repo deployment");
                                let bundle_id = parsed
                                    .get("bundle_id")
                                    .and_then(|v| v.as_str())
                                    .unwrap_or("bundle");
                                let repo_url = parsed
                                    .get("repo_url")
                                    .and_then(|v| v.as_str())
                                    .unwrap_or("");
                                let repo_ref = parsed
                                    .get("repo_ref")
                                    .and_then(|v| v.as_str())
                                    .unwrap_or("");
                                let status = parsed
                                    .get("status")
                                    .and_then(|v| v.as_str())
                                    .unwrap_or("deployed");
                                let readme_file = parsed
                                    .get("readme_file")
                                    .and_then(|v| v.as_str())
                                    .unwrap_or("");
                                let local_base = Self::user_facing_local_base_url();
                                let mut lines =
                                    vec![format!("Repository deployment ready: **{}**.", title)];
                                if !repo_url.is_empty() {
                                    lines.push(format!("- Source: {}", repo_url));
                                }
                                if !repo_ref.is_empty() {
                                    lines.push(format!("- Ref: `{}`", repo_ref));
                                }
                                lines.push(format!("- Bundle ID: `{}`", bundle_id));
                                lines.push(format!("- Status: {}", status.replace('_', " ")));
                                if !readme_file.is_empty() {
                                    lines.push(format!("- README inspected: `{}`", readme_file));
                                }
                                let services = parsed
                                    .get("services")
                                    .and_then(|v| v.as_array())
                                    .cloned()
                                    .unwrap_or_default();
                                if services.is_empty() {
                                    lines.push("- No runnable services were returned.".to_string());
                                } else {
                                    lines.push("- Services:".to_string());
                                    for mut service in services {
                                        let title = service
                                            .get("title")
                                            .and_then(|v| v.as_str())
                                            .unwrap_or("Service");
                                        let title = title.to_string();
                                        let kind = service
                                            .get("kind")
                                            .and_then(|v| v.as_str())
                                            .unwrap_or("service");
                                        let kind = kind.to_string();
                                        let mut service_status = service
                                            .get("status")
                                            .and_then(|v| v.as_str())
                                            .unwrap_or("unknown")
                                            .to_string();
                                        let relative_dir = service
                                            .get("relative_dir")
                                            .and_then(|v| v.as_str())
                                            .unwrap_or("");
                                        let relative_dir = relative_dir.to_string();
                                        let detection_reason = service
                                            .get("detection_reason")
                                            .and_then(|v| v.as_str())
                                            .unwrap_or("");
                                        let detection_reason = detection_reason.to_string();
                                        if service
                                            .get("result")
                                            .and_then(|result| result.get("runtime_delegated"))
                                            .and_then(|v| v.as_bool())
                                            .unwrap_or(false)
                                        {
                                            let delegated_app_id = service
                                                .get("result")
                                                .and_then(|result| result.get("app_id"))
                                                .and_then(|v| v.as_str())
                                                .map(str::trim)
                                                .filter(|value| !value.is_empty());
                                            match delegated_app_id {
                                                Some(app_id) => match self
                                                    .restart_deployed_app_from_metadata(
                                                        app_id,
                                                        Some(&title),
                                                    )
                                                    .await
                                                {
                                                    Ok(restarted) => {
                                                        service_status = restarted
                                                            .get("status")
                                                            .and_then(|v| v.as_str())
                                                            .unwrap_or("deployed")
                                                            .to_string();
                                                        service["status"] =
                                                            serde_json::Value::String(
                                                                service_status.clone(),
                                                            );
                                                        service["result"] = restarted;
                                                    }
                                                    Err(error) => {
                                                        service_status = "failed".to_string();
                                                        service["status"] =
                                                            serde_json::Value::String(
                                                                service_status.clone(),
                                                            );
                                                        service["error"] =
                                                            serde_json::Value::String(
                                                                error.to_string(),
                                                            );
                                                    }
                                                },
                                                None => {
                                                    service_status = "failed".to_string();
                                                    service["status"] = serde_json::Value::String(
                                                        service_status.clone(),
                                                    );
                                                    service["error"] = serde_json::Value::String(
                                                        format!(
                                                            "Delegated repo service '{}' did not include an app_id.",
                                                            title
                                                        ),
                                                    );
                                                }
                                            }
                                        }
                                        let result_obj = service.get("result");
                                        let local_url = result_obj
                                            .and_then(|result| result.get("url"))
                                            .and_then(|v| v.as_str())
                                            .map(|url| {
                                                Self::absolutize_public_url(
                                                    Some(local_base.as_str()),
                                                    url,
                                                )
                                            });
                                        let public_link_note = if expose_public_requested {
                                            Some(
                                                "    Public link requested; public URL health is not part of the blocking local deploy check."
                                                    .to_string(),
                                            )
                                        } else {
                                            None
                                        };
                                        lines.push(format!(
                                            "  - {} ({}) - {}{}",
                                            title,
                                            kind,
                                            service_status.replace('_', " "),
                                            if relative_dir.is_empty() {
                                                String::new()
                                            } else {
                                                format!(" from `{}`", relative_dir)
                                            }
                                        ));
                                        if let Some(local_url) = local_url {
                                            lines.push(format!(
                                                "    Local: [Open app]({})",
                                                local_url
                                            ));
                                        }
                                        if let Some(public_link_note) = public_link_note {
                                            lines.push(public_link_note);
                                        }
                                        if let Some(port) = result_obj
                                            .and_then(|result| result.get("port"))
                                            .and_then(|v| v.as_u64())
                                        {
                                            lines.push(format!("    Port: `{}`", port));
                                        }
                                        if !detection_reason.is_empty() {
                                            lines.push(format!(
                                                "    Detected from: {}",
                                                detection_reason
                                            ));
                                        }
                                        if service_status == "failed" {
                                            if let Some(error) =
                                                service.get("error").and_then(|v| v.as_str())
                                            {
                                                lines.push(format!("    Error: {}", error));
                                            }
                                        } else if service_status == "needs_secrets" {
                                            if let Some(message) = result_obj
                                                .and_then(|result| result.get("message"))
                                                .and_then(|v| v.as_str())
                                            {
                                                lines.push(format!("    Needs input: {}", message));
                                            }
                                        }
                                    }
                                }
                                let msg = lines.join("\n");
                                if let Some(ref tx) = stream_tx {
                                    queue_stream_event(
                                        tx,
                                        StreamEvent::ToolResult {
                                            name: call.name.clone(),
                                            content: msg.clone(),
                                        },
                                    );
                                }
                                results.push(format!(
                                    "{}{}",
                                    crate::runtime::TOOL_COMPLETION_MARKER,
                                    serde_json::json!({
                                        "tool": "app_deploy",
                                        "status": "completed",
                                        "success": true,
                                        "detail": msg,
                                        "deploy_attempted": true,
                                        "retryable": false,
                                        "data": {
                                            "deploy_attempted": true,
                                            "retryable": false,
                                            "deployment_kind": "repo_bundle",
                                            "bundle_id": bundle_id,
                                        },
                                    })
                                ));
                                continue;
                            }
                            if parsed
                                .get("runtime_delegated")
                                .and_then(|v| v.as_bool())
                                .unwrap_or(false)
                            {
                                let app_id = parsed
                                    .get("app_id")
                                    .and_then(|v| v.as_str())
                                    .map(str::trim)
                                    .unwrap_or("");
                                let title = parsed
                                    .get("title")
                                    .and_then(|v| v.as_str())
                                    .unwrap_or("App");
                                if app_id.is_empty() {
                                    let msg = format!(
                                        "App '{}' files were written, but runtime delegation failed because the deployment response did not include an app_id.",
                                        title
                                    );
                                    if let Some(ref tx) = stream_tx {
                                        queue_stream_event(
                                            tx,
                                            StreamEvent::ToolResult {
                                                name: call.name.clone(),
                                                content: msg.clone(),
                                            },
                                        );
                                    }
                                    results.push(format!(
                                        "{}{}",
                                        crate::runtime::TOOL_COMPLETION_MARKER,
                                        serde_json::json!({
                                            "tool": "app_deploy",
                                            "status": "failed",
                                            "success": false,
                                            "detail": msg,
                                            "deploy_attempted": true,
                                            "retryable": false,
                                            "data": {
                                                "deploy_attempted": true,
                                                "retryable": false,
                                            },
                                        })
                                    ));
                                    continue;
                                }
                                match self
                                    .restart_deployed_app_from_metadata(app_id, Some(title))
                                    .await
                                {
                                    Ok(restarted) => {
                                        parsed = restarted;
                                    }
                                    Err(error) => {
                                        let msg = format!(
                                            "App '{}' files were deployed and registered, but executor startup failed: {}",
                                            title, error
                                        );
                                        if let Some(ref tx) = stream_tx {
                                            queue_stream_event(
                                                tx,
                                                StreamEvent::ToolResult {
                                                    name: call.name.clone(),
                                                    content: msg.clone(),
                                                },
                                            );
                                        }
                                        results.push(format!(
                                            "{}{}",
                                            crate::runtime::TOOL_COMPLETION_MARKER,
                                            serde_json::json!({
                                                "tool": "app_deploy",
                                                "status": "failed",
                                                "success": false,
                                                "detail": msg,
                                                "deploy_attempted": true,
                                                "retryable": false,
                                                "data": {
                                                    "deploy_attempted": true,
                                                    "retryable": false,
                                                    "app_id": app_id,
                                                },
                                            })
                                        ));
                                        continue;
                                    }
                                }
                            }
                            if parsed
                                .get("status")
                                .and_then(|v| v.as_str())
                                .is_some_and(|s| s == "needs_secrets")
                            {
                                let title = parsed
                                    .get("title")
                                    .and_then(|v| v.as_str())
                                    .unwrap_or("App");
                                let app_id = parsed
                                    .get("app_id")
                                    .and_then(|v| v.as_str())
                                    .unwrap_or("app");
                                let missing = parsed
                                    .get("missing_env")
                                    .and_then(|v| v.as_array())
                                    .map(|arr| {
                                        arr.iter()
                                            .filter_map(|v| v.as_str())
                                            .collect::<Vec<_>>()
                                            .join(", ")
                                    })
                                    .unwrap_or_else(|| "unknown".to_string());
                                let missing_config = parsed
                                    .get("missing_config")
                                    .and_then(|v| v.as_array())
                                    .map(|arr| {
                                        arr.iter()
                                            .filter_map(|v| v.as_str())
                                            .collect::<Vec<_>>()
                                            .join(", ")
                                    })
                                    .unwrap_or_default();
                                let public_access_note = if expose_public_requested {
                                    match self
                                        .ensure_public_tunnel_base_url(Some(app_id), stream_tx.as_ref())
                                        .await
                                    {
                                        Some(_) => "\nPublic link requested. I will share it after the app finishes validation.".to_string(),
                                        None => "\nPublic link requested. Tunnel setup is pending, and I will share the link only after validation passes.".to_string(),
                                    }
                                } else {
                                    String::new()
                                };
                                let msg = format!(
                                    "App '{}' is ready, but I need your approval/input for credentials before I continue.\n\
                                      Missing sensitive keys: {}{}\n\n\
                                      Use the secure credential form in chat or Settings for sensitive values.\n\
                                      AgentArk's own model/provider credentials are not inherited by generated apps.\n\
                                      Why I'm asking: credentials are stored encrypted and handled outside model generation to reduce leak risk.\n\
                                      For non-sensitive config values, redeploy/restart with config.{{KEY}}=value.\n\
                                      Then restart app '{}'.{}",
                                    title,
                                    if missing.is_empty() { "none" } else { &missing },
                                    if missing_config.is_empty() {
                                        "".to_string()
                                    } else {
                                        format!("\nMissing config values: {}", missing_config)
                                    },
                                    app_id,
                                    public_access_note
                                );
                                if let Some(cid) =
                                    conversation_id.filter(|value| !value.trim().is_empty())
                                {
                                    self.persist_last_deployed_app_context(
                                        cid,
                                        app_id,
                                        title,
                                        &format!("/apps/{}/", app_id),
                                    )
                                    .await;
                                    self.remember_pending_secret_followup(
                                        cid,
                                        PendingSecretFollowupKind::RestartApp {
                                            app_id: app_id.to_string(),
                                            title: title.to_string(),
                                            missing_env: parsed
                                                .get("missing_env")
                                                .and_then(|v| v.as_array())
                                                .map(|arr| {
                                                    arr.iter()
                                                        .filter_map(|v| v.as_str())
                                                        .map(|value| value.to_string())
                                                        .collect::<Vec<_>>()
                                                })
                                                .unwrap_or_default(),
                                        },
                                    )
                                    .await;
                                }
                                if let Some(ref tx) = stream_tx {
                                    queue_stream_event(
                                        tx,
                                        StreamEvent::ToolResult {
                                            name: call.name.clone(),
                                            content: msg.clone(),
                                        },
                                    );
                                }
                                results.push(format!(
                                    "{}{}",
                                    crate::runtime::TOOL_COMPLETION_MARKER,
                                    serde_json::json!({
                                        "tool": "app_deploy",
                                        "status": "needs_secrets",
                                        "success": true,
                                        "detail": msg,
                                        "app_id": app_id,
                                        "title": title,
                                        "deploy_attempted": true,
                                        "retryable": false,
                                        "data": {
                                            "deploy_attempted": true,
                                            "retryable": false,
                                            "missing_env": parsed.get("missing_env").cloned(),
                                            "missing_config": parsed.get("missing_config").cloned(),
                                        },
                                    })
                                ));
                                continue;
                            }
                            if parsed.get("url").is_some() || parsed.get("app_id").is_some() {
                                let updated_existing = parsed
                                    .get("updated_existing")
                                    .and_then(|v| v.as_bool())
                                    .unwrap_or(false);
                                let action_verb = if updated_existing {
                                    "updated"
                                } else {
                                    "deployed"
                                };
                                let title = parsed
                                    .get("title")
                                    .and_then(|v| v.as_str())
                                    .unwrap_or("App")
                                    .to_string();
                                let app_type = parsed
                                    .get("type")
                                    .and_then(|v| v.as_str())
                                    .unwrap_or("static")
                                    .to_string();
                                let app_id_raw = parsed
                                    .get("app_id")
                                    .and_then(|v| v.as_str())
                                    .map(|v| v.trim())
                                    .unwrap_or("");
                                let app_id = if app_id_raw.is_empty() {
                                    "app".to_string()
                                } else {
                                    app_id_raw.to_string()
                                };
                                let access_key = parsed
                                    .get("access_password")
                                    .or_else(|| parsed.get("access_key"))
                                    .and_then(|v| v.as_str())
                                    .unwrap_or("")
                                    .to_string();
                                let access_guard_enabled = parsed
                                    .get("access_guard_enabled")
                                    .and_then(|v| v.as_bool())
                                    .unwrap_or(false);
                                let expose_public = parsed
                                    .get("expose_public")
                                    .and_then(|v| v.as_bool())
                                    .unwrap_or(expose_public_requested);
                                let public_access_guard_enabled = parsed
                                    .get("public_access_guard_enabled")
                                    .and_then(|v| v.as_bool())
                                    .unwrap_or(access_guard_enabled || expose_public);
                                let canonical_relative_url = parsed
                                    .get("url")
                                    .and_then(|v| v.as_str())
                                    .map(|s| s.to_string())
                                    .unwrap_or_else(|| {
                                        if !app_id_raw.is_empty() {
                                            format!("/apps/{}/", app_id_raw)
                                        } else {
                                            "/apps/".to_string()
                                        }
                                    });
                                let url_with_key = parsed
                                    .get("access_url")
                                    .and_then(|v| v.as_str())
                                    .map(|s| s.to_string())
                                    .unwrap_or_else(|| canonical_relative_url.clone());
                                let mut public_base_for_app = if expose_public_requested {
                                    self.ensure_public_tunnel_base_url(
                                        Some(app_id.as_str()),
                                        stream_tx.as_ref(),
                                    )
                                    .await
                                    .or_else(|| public_base_url.clone())
                                } else {
                                    None
                                };

                                let (
                                    preview_url,
                                    verified,
                                    verify_attempts,
                                    verify_detail,
                                    rendered_content,
                                ) = self
                                    .validate_and_capture_app_preview(
                                        &url_with_key,
                                        &app_id,
                                        &app_type,
                                        Some(access_key.as_str()),
                                        stream_tx.as_ref(),
                                    )
                                    .await
                                    .unwrap_or_else(|e| {
                                        (
                                            None,
                                            false,
                                            0,
                                            format!("Validation helper error: {}", e),
                                            None,
                                        )
                                    });

                                if expose_public_requested && public_base_for_app.is_none() {
                                    // Tunnel URL can appear shortly after initial startup polling.
                                    // Re-run discovery here so the final chat reply includes the
                                    // public link whenever it is already available.
                                    public_base_for_app = self
                                        .ensure_public_tunnel_base_url(Some(&app_id), None)
                                        .await
                                        .or_else(|| public_base_url.clone());
                                }
                                let local_base_url = Self::user_facing_local_base_url();
                                let local_open_url = Self::absolutize_public_url(
                                    Some(local_base_url.as_str()),
                                    &canonical_relative_url,
                                );
                                let mut public_open_url: Option<String> = None;
                                let mut public_verify_detail: Option<String> = None;
                                if expose_public_requested && verified {
                                    if let Some(public_base) = public_base_for_app.as_deref() {
                                        let public_display_url = Self::absolutize_public_url(
                                            Some(public_base),
                                            &canonical_relative_url,
                                        );
                                        if public_display_url != local_open_url {
                                            public_open_url = Some(public_display_url);
                                            public_verify_detail = Some(
                                                "available; local validation passed and public URL was not separately validated."
                                                    .to_string(),
                                            );
                                        }
                                    } else {
                                        public_verify_detail = Some(
                                            "tunnel setup is still pending for this app."
                                                .to_string(),
                                        );
                                    }
                                } else if expose_public_requested && !verified {
                                    public_verify_detail = Some(
                                        "skipped because local app validation did not pass yet."
                                            .to_string(),
                                    );
                                }
                                let conversation_scope_id = call
                                    .arguments
                                    .get("conversation_id")
                                    .and_then(|v| v.as_str())
                                    .map(str::trim)
                                    .filter(|s| !s.is_empty())
                                    .or(conversation_id.filter(|value| !value.trim().is_empty()));
                                if let Some(cid) = conversation_scope_id {
                                    let canonical_url = if app_id.is_empty() {
                                        "/apps/".to_string()
                                    } else {
                                        format!("/apps/{}/", app_id)
                                    };
                                    self.persist_last_deployed_app_context(
                                        cid,
                                        &app_id,
                                        &title,
                                        &canonical_url,
                                    )
                                    .await;
                                }
                                let quality_check_queued = if verified {
                                    let request_context = repair_context.build_request_text();
                                    self.queue_app_quality_check(
                                        &app_id,
                                        &url_with_key,
                                        &app_type,
                                        &request_context,
                                        &repair_context.goal_summaries,
                                    )
                                    .await;
                                    true
                                } else {
                                    false
                                };
                                if let Some(ref tx) = stream_tx {
                                    queue_stream_event(
                                        tx,
                                        StreamEvent::ToolResult {
                                            name: call.name.clone(),
                                            content: if verified {
                                                format!(
                                                    "App {} + local structural validation passed: {} ({}) [{} probe{}]",
                                                    action_verb,
                                                    title,
                                                    app_type,
                                                    verify_attempts,
                                                    if verify_attempts == 1 { "" } else { "s" }
                                                )
                                            } else {
                                                format!(
                                                    "App {}, validation incomplete: {} ({}) - {}",
                                                    action_verb, title, app_type, verify_detail
                                                )
                                            },
                                        },
                                    );
                                }

                                let mut app_message_lines: Vec<String> = Vec::new();
                                if verified {
                                    app_message_lines.push(format!(
                                        "I have {} **{}** ({} app), and the local structural validation passed.",
                                        action_verb, title, app_type
                                    ));
                                } else {
                                    app_message_lines.push(format!(
                                        "I have {} **{}** ({} app), but validation has not passed yet.",
                                        action_verb, title, app_type
                                    ));
                                }
                                app_message_lines.push(format!("- App ID: `{}`", app_id));

                                if verified {
                                    app_message_lines.push(format!(
                                        "- Local: [Open local app]({})",
                                        local_open_url
                                    ));
                                } else {
                                    app_message_lines.push(format!(
                                        "- Local (unverified): [Open local app]({})",
                                        local_open_url
                                    ));
                                }
                                if let Some(public_open_url) = public_open_url.as_ref() {
                                    app_message_lines.push(format!(
                                        "- Public: [Open public app]({})",
                                        public_open_url
                                    ));
                                    if let Some(detail) = public_verify_detail.as_ref() {
                                        app_message_lines
                                            .push(format!("- Public link status: {}", detail));
                                    }
                                } else if expose_public_requested {
                                    app_message_lines.push(format!(
                                        "- Public link status: {}",
                                        public_verify_detail.unwrap_or_else(|| {
                                            "pending tunnel readiness for this app.".to_string()
                                        })
                                    ));
                                }

                                if access_guard_enabled {
                                    app_message_lines
                                        .push("- Local App Guard: enabled.".to_string());
                                    if !access_key.trim().is_empty() {
                                        app_message_lines.push(format!(
                                            "- Access password: `{}`",
                                            access_key.trim()
                                        ));
                                        app_message_lines.push(
                                            "- Open the link above and enter the access password if prompted."
                                                .to_string(),
                                        );
                                    }
                                } else {
                                    app_message_lines
                                        .push("- Local App Guard: not enabled.".to_string());
                                }
                                if expose_public {
                                    app_message_lines.push(format!(
                                        "- Public App Guard: {}.",
                                        if public_access_guard_enabled {
                                            "enabled"
                                        } else {
                                            "not enabled"
                                        }
                                    ));
                                }

                                app_message_lines.push(format!(
                                    "- Webpage status: {}",
                                    if verified {
                                        "reachable by local structural probe."
                                    } else {
                                        "deployed, but validation has not passed yet."
                                    }
                                ));
                                app_message_lines.push(format!(
                                    "- Local structural validation: {} (probes: {}).",
                                    if verified { "passed" } else { "failed" },
                                    verify_attempts
                                ));
                                if quality_check_queued {
                                    app_message_lines.push(
                                        "- Background quality report: queued; it is advisory and cannot trigger another deployment."
                                            .to_string(),
                                    );
                                }
                                if !verified && !verify_detail.trim().is_empty() {
                                    app_message_lines.push(format!(
                                        "- Validation issue: {}",
                                        verify_detail.trim()
                                    ));
                                }
                                app_message_lines.push(format!(
                                    "- Apps page: {}",
                                    crate::actions::app::APP_DEPLOY_CONTROL_HINT
                                ));

                                if let Some(preview) = preview_url {
                                    app_message_lines.push(format!("![App Preview]({})", preview));
                                }
                                let app_message = app_message_lines.join("\n");
                                let rendered_evidence = rendered_content.as_ref().map(|content| {
                                    let interactive_elements = content
                                        .elements
                                        .iter()
                                        .take(40)
                                        .map(|element| {
                                            serde_json::json!({
                                                "tag": &element.tag,
                                                "type": &element.r#type,
                                                "text": safe_truncate(&element.text, 180),
                                                "name": safe_truncate(&element.name, 120),
                                                "id": safe_truncate(&element.id, 80),
                                                "href": safe_truncate(&element.href, 180),
                                            })
                                        })
                                        .collect::<Vec<_>>();
                                    let diagnostics = content
                                        .diagnostics
                                        .iter()
                                        .take(40)
                                        .map(|entry| {
                                            serde_json::json!({
                                                "kind": &entry.kind,
                                                "severity": &entry.severity,
                                                "message": safe_truncate(&entry.message, 260),
                                                "url": safe_truncate(&entry.url, 220),
                                                "resource_type": &entry.resource_type,
                                            })
                                        })
                                        .collect::<Vec<_>>();
                                    serde_json::json!({
                                        "title": &content.title,
                                        "url": &content.url,
                                        "body_text": safe_truncate(&content.body_text, 3000),
                                        "interactive_elements": interactive_elements,
                                        "browser_diagnostics": diagnostics,
                                    })
                                });
                                let completion_payload = serde_json::json!({
                                    "tool": "app_deploy",
                                    "status": if verified { "completed" } else { "validation_incomplete" },
                                    "success": verified,
                                    "detail": app_message,
                                    "app_id": app_id,
                                    "title": title,
                                    "url": canonical_relative_url,
                                    "access_url": url_with_key,
                                    "local_url": local_open_url,
                                    "access_guard_enabled": access_guard_enabled,
                                    "public_access_guard_enabled": public_access_guard_enabled,
                                    "expose_public": expose_public,
                                    "apps_page_hint": crate::actions::app::APP_DEPLOY_CONTROL_HINT,
                                    "verified": verified,
                                    "quality_report_status": if quality_check_queued { "pending" } else { "skipped" },
                                    "deploy_attempted": true,
                                    "retryable": false,
                                    "validation_attempts": verify_attempts,
                                    "validation_detail": verify_detail.clone(),
                                    "rendered_evidence": rendered_evidence.clone(),
                                    "data": {
                                        "retryable": false,
                                        "deploy_attempted": true,
                                        "quality_report_status": if quality_check_queued { "pending" } else { "skipped" },
                                        "validation_attempts": verify_attempts,
                                        "validation_detail": verify_detail,
                                        "rendered_evidence": rendered_evidence,
                                    },
                                });
                                results.push(format!(
                                    "{}{}",
                                    crate::runtime::TOOL_COMPLETION_MARKER,
                                    completion_payload
                                ));
                                continue;
                            }
                        }
                        if let Some(ref tx) = stream_tx {
                            queue_stream_event(
                                tx,
                                StreamEvent::ToolResult {
                                    name: call.name.clone(),
                                    content: sanitize_stream(&result),
                                },
                            );
                        }
                        results.push(result);
                    }
                    Err(e) => {
                        tracing::error!("App deployment error: {}", e);
                        self.fire_action_hook(
                            crate::hooks::HookTrigger::OnError,
                            request_channel,
                            &call.name,
                            hook_hint.as_deref(),
                            Some(&e.to_string()),
                            &hook_event_id,
                        )
                        .await;
                        let error_text = e.to_string();
                        let file_inventory = deploy_args_for_result
                            .get("files")
                            .and_then(|value| value.as_object())
                            .map(|files| {
                                let mut names = files.keys().cloned().collect::<Vec<_>>();
                                names.sort();
                                names
                            })
                            .unwrap_or_default();
                        let detail = format!("Error deploying app: {}", error_text);
                        let completion_payload = serde_json::json!({
                            "tool": "app_deploy",
                            "status": "failed",
                            "success": false,
                            "detail": detail.clone(),
                            "error": error_text.clone(),
                            "deploy_attempted": true,
                            "retryable": false,
                            "data": {
                                "retryable": false,
                                "deploy_attempted": true,
                                "file_inventory": file_inventory,
                                "validation_error": error_text,
                            },
                        });
                        let formatted = format!(
                            "{}{}",
                            crate::runtime::TOOL_COMPLETION_MARKER,
                            completion_payload
                        );
                        if let Some(ref tx) = stream_tx {
                            queue_stream_event(
                                tx,
                                StreamEvent::ToolResult {
                                    name: call.name.clone(),
                                    content: detail,
                                },
                            );
                        }
                        results.push(formatted);
                    }
                }
                continue;
            }

            // Execute in sandbox (runtime will resolve secret placeholders at execution time)
            let call_message_hint = action_message_hint(&call.arguments);
            match self
                .execute_action_with_hooks(
                    &call.name,
                    &call.arguments,
                    request_channel,
                    call_message_hint.as_deref(),
                    authorization,
                )
                .await
            {
                Ok(result) => {
                    let mut result = result;
                    if call.name.starts_with("mcp_") {
                        result = self.sanitize_mcp_output(&result);
                    }
                    // Special handling for schedule_task - actually create the task
                    if call.name == "schedule_task"
                        && crate::runtime::parse_schedule_task_completion(&result).is_some()
                    {
                        if let Some(schedule_result) = self
                            .handle_schedule_task(
                                &call.arguments,
                                request_channel,
                                conversation_id,
                                project_id,
                                authorization,
                            )
                            .await
                        {
                            if let Some(ref tx) = stream_tx {
                                queue_stream_event(
                                    tx,
                                    StreamEvent::ToolResult {
                                        name: call.name.clone(),
                                        content: sanitize_stream(&schedule_result),
                                    },
                                );
                            }
                            results.push(schedule_result);
                            continue;
                        }
                    }

                    // Special handling for watch - spawn background watcher
                    if call.name == "watch"
                        && crate::runtime::parse_watch_completion(&result).is_some()
                    {
                        if let Some(watch_result) = self
                            .handle_watch(
                                &call.arguments,
                                request_channel,
                                conversation_id,
                                project_id,
                                authorization,
                            )
                            .await
                        {
                            if let Some(ref tx) = stream_tx {
                                queue_stream_event(
                                    tx,
                                    StreamEvent::ToolResult {
                                        name: call.name.clone(),
                                        content: sanitize_stream(&watch_result),
                                    },
                                );
                            }
                            results.push(watch_result);
                            continue;
                        }
                    }

                    if call.name == "background_session_manage" {
                        if let Some(session_result) = self
                            .handle_background_session_manage(&call.arguments, conversation_id)
                            .await
                        {
                            if let Some(ref tx) = stream_tx {
                                queue_stream_event(
                                    tx,
                                    StreamEvent::ToolResult {
                                        name: call.name.clone(),
                                        content: sanitize_stream(&session_result),
                                    },
                                );
                            }
                            results.push(session_result);
                            continue;
                        }
                    }

                    if call.name == "delegate"
                        && crate::runtime::parse_delegate_completion(&result).is_some()
                    {
                        let delegate_result = self
                            .handle_delegate_tool_call(
                                &call.arguments,
                                request_channel,
                                trace_ref,
                                stream_tx.as_ref(),
                            )
                            .await;
                        if let Some(ref tx) = stream_tx {
                            queue_stream_event(
                                tx,
                                StreamEvent::ToolResult {
                                    name: call.name.clone(),
                                    content: sanitize_stream(&delegate_result),
                                },
                            );
                        }
                        results.push(delegate_result);
                        continue;
                    }

                    // Format code_execute results with self-heal retry on errors
                    if call.name == "code_execute" {
                        let language = call
                            .arguments
                            .get("language")
                            .and_then(|v| v.as_str())
                            .unwrap_or("code")
                            .to_string();
                        let mut current_code = call
                            .arguments
                            .get("code")
                            .and_then(|v| v.as_str())
                            .unwrap_or("")
                            .to_string();
                        let mut current_result = result.clone();
                        let mut current_args = call.arguments.clone();

                        // Self-heal loop: retry on execution errors
                        const MAX_SAME_ERROR_RETRIES: usize = 2;
                        const MAX_TOTAL_RETRIES: usize = 3;
                        let mut total_retries = 0usize;
                        let mut last_error_sig = String::new();
                        let mut same_error_count = 0usize;
                        let mut self_heal_stop_reason: Option<String> = None;
                        let mut self_heal_error_signatures: Vec<String> = Vec::new();
                        let code_signature = |code: &str| -> String {
                            let mut normalized = code
                                .lines()
                                .map(|line| line.trim())
                                .filter(|line| !line.is_empty())
                                .collect::<Vec<_>>()
                                .join("\n");
                            if normalized.len() > 4096 {
                                normalized.truncate(4096);
                            }
                            normalized
                        };
                        let mut seen_code_signatures: HashSet<String> = HashSet::new();
                        let initial_sig = code_signature(&current_code);
                        if !initial_sig.is_empty() {
                            seen_code_signatures.insert(initial_sig);
                        }

                        loop {
                            let parsed = match serde_json::from_str::<serde_json::Value>(
                                &current_result,
                            ) {
                                Ok(parsed) => parsed,
                                Err(_) => {
                                    if total_retries > 0 {
                                        self_heal_stop_reason = Some(
                                            "runtime response was not structured JSON; stopped auto-fix"
                                                .to_string(),
                                        );
                                    }
                                    break;
                                }
                            };
                            let exit_code = parsed
                                .get("exit_code")
                                .and_then(|v| v.as_i64())
                                .unwrap_or(0);
                            let should_retry = exit_code != 0 && !current_code.trim().is_empty();

                            if !should_retry {
                                break;
                            }

                            if total_retries >= MAX_TOTAL_RETRIES {
                                self_heal_stop_reason =
                                    Some(format!("maximum attempts reached ({MAX_TOTAL_RETRIES})"));
                                break;
                            }

                            let error_text =
                                parsed.get("error").and_then(|v| v.as_str()).unwrap_or("");
                            let output_text =
                                parsed.get("output").and_then(|v| v.as_str()).unwrap_or("");

                            if code_execute_uses_data_path_without_inputs(
                                &current_args,
                                &current_code,
                            ) {
                                self_heal_stop_reason = Some(
                                    "code sandbox `/data` is reserved for injected input files; write generated outputs under `/workspace`, or use app_deploy for web apps".to_string(),
                                );
                                break;
                            }

                            // Bail immediately on sandbox-environment errors that retries cannot fix
                            let combined_for_check = format!("{}\n{}", error_text, output_text);
                            let is_sandbox_unreachable = combined_for_check
                                .contains("No such file or directory")
                                && (combined_for_check.contains("/app/data/")
                                    || combined_for_check.contains("os.chdir"));
                            if is_sandbox_unreachable {
                                self_heal_stop_reason = Some(
                                    "sandbox cannot access app data paths; use file_write/file_read tools instead".to_string(),
                                );
                                break;
                            }

                            // Build error signature for same-error detection
                            let error_combined = format!("{}\n{}", error_text, output_text);
                            let error_sig = error_combined
                                .lines()
                                .take(5)
                                .collect::<Vec<_>>()
                                .join("\n");
                            if !error_sig.is_empty()
                                && !self_heal_error_signatures.iter().any(|s| s == &error_sig)
                                && self_heal_error_signatures.len() < 4
                            {
                                self_heal_error_signatures.push(error_sig.clone());
                            }

                            if error_sig == last_error_sig {
                                same_error_count += 1;
                                if same_error_count >= MAX_SAME_ERROR_RETRIES {
                                    tracing::warn!(
                                        "Self-heal: same error repeated {} times, giving up",
                                        same_error_count
                                    );
                                    self_heal_stop_reason = Some(format!(
                                        "same failure repeated {} times",
                                        same_error_count
                                    ));
                                    break;
                                }
                            } else {
                                same_error_count = 1;
                                last_error_sig = error_sig;
                            }

                            total_retries += 1;
                            tracing::info!(
                                "Self-heal: code execution failed (attempt {}/{}), asking LLM to fix",
                                total_retries,
                                MAX_TOTAL_RETRIES
                            );

                            // Emit trace step
                            {
                                let mut trace = trace_ref.write().await;
                                trace.steps.push(ExecutionStep {
                                    icon: "[fix]".to_string(),
                                    title: format!(
                                        "Self-Heal: Fixing Code (attempt {})",
                                        total_retries
                                    ),
                                    detail: format!(
                                        "Error: {}",
                                        error_text.chars().take(100).collect::<String>()
                                    ),
                                    step_type: "thinking".to_string(),
                                    data: None,
                                    timestamp: chrono::Utc::now(),
                                    duration_ms: None,
                                });
                            }

                            // Ask LLM to fix the code
                            let fix_prompt = format!(
                                "The following {} code failed to execute. Fix the code and return ONLY the corrected code, no explanation.\n\n\
                                Code:\n```{}\n{}\n```\n\n\
                                Error output:\n```\n{}\n{}\n```\n\n\
                                Return only the fixed code, nothing else.",
                                language,
                                language,
                                current_code.trim(),
                                error_text,
                                output_text
                            );

                            let empty_actions: Vec<crate::actions::ActionDef> = Vec::new();
                            match self.llm.chat(
                                "You are a code fixer. Return ONLY the corrected code. No markdown fences, no explanations.",
                                &fix_prompt,
                                &[],
                                &empty_actions,
                            ).await {
                                Ok(fix_response) => {
                                    self.record_llm_usage(request_channel, "self_heal", &fix_response).await;
                                    // Extract code from response (strip markdown fences if present)
                                    let fixed = fix_response.content.trim().to_string();
                                    let fixed = if fixed.starts_with("```") {
                                        // Strip opening ```lang and closing ```
                                        let lines: Vec<&str> = fixed.lines().collect();
                                        let start = if lines.first().is_some_and(|l| l.starts_with("```")) { 1 } else { 0 };
                                        let end = if lines.last().is_some_and(|l| l.trim() == "```") { lines.len() - 1 } else { lines.len() };
                                        lines[start..end].join("\n")
                                    } else {
                                        fixed
                                    };
                                    let fixed_sig = code_signature(&fixed);
                                    let current_sig = code_signature(&current_code);
                                    if fixed_sig.is_empty() {
                                        tracing::warn!(
                                            "Self-heal: LLM returned empty code, giving up"
                                        );
                                        self_heal_stop_reason =
                                            Some("LLM returned empty patch".to_string());
                                        break;
                                    }
                                    if fixed_sig == current_sig {
                                        tracing::warn!("Self-heal: LLM returned identical code, giving up");
                                        self_heal_stop_reason =
                                            Some("LLM returned no meaningful code change".to_string());
                                        break;
                                    }
                                    if seen_code_signatures.contains(&fixed_sig) {
                                        tracing::warn!(
                                            "Self-heal: repeated patch detected, giving up"
                                        );
                                        self_heal_stop_reason = Some(
                                            "repeated patch detected (loop prevention)".to_string(),
                                        );
                                        break;
                                    }
                                    seen_code_signatures.insert(fixed_sig);

                                    current_code = fixed.clone();
                                    current_args["code"] = serde_json::Value::String(fixed);

                                    // Re-execute with fixed code
                                    let retry_hint = action_message_hint(&current_args);
                                    match self
                                        .execute_action_with_hooks(
                                            "code_execute",
                                            &current_args,
                                            request_channel,
                                            retry_hint.as_deref(),
                                            authorization,
                                        )
                                        .await
                                    {
                                        Ok(new_result) => {
                                            current_result = new_result;
                                        }
                                        Err(e) => {
                                            tracing::error!("Self-heal re-execution error: {}", e);
                                            self_heal_stop_reason = Some(format!(
                                                "re-execution failed: {}",
                                                safe_truncate(&e.to_string(), 180)
                                            ));
                                            break;
                                        }
                                    }
                                }
                                Err(e) => {
                                    tracing::error!("Self-heal LLM call failed: {}", e);
                                    self_heal_stop_reason = Some(format!(
                                        "LLM fixer failed: {}",
                                        safe_truncate(&e.to_string(), 180)
                                    ));
                                    break;
                                }
                            }
                        }

                        // Format the final result (after retries or on first success)
                        let parsed_final_result =
                            serde_json::from_str::<serde_json::Value>(&current_result).ok();
                        let formatted = if let Some(parsed) = parsed_final_result.as_ref() {
                            let output =
                                parsed.get("output").and_then(|v| v.as_str()).unwrap_or("");
                            let error = parsed.get("error").and_then(|v| v.as_str());
                            let exit_code = parsed
                                .get("exit_code")
                                .and_then(|v| v.as_i64())
                                .unwrap_or(-1);
                            let files = parsed.get("files").and_then(|v| v.as_array());

                            let mut parts = Vec::new();

                            if total_retries > 0 {
                                let status = if exit_code == 0 {
                                    "fixed"
                                } else {
                                    "still failing"
                                };
                                parts.push(format!(
                                    "*Self-healed after {} attempt{} ({})*",
                                    total_retries,
                                    if total_retries == 1 { "" } else { "s" },
                                    status
                                ));
                                if exit_code != 0 {
                                    if let Some(reason) = &self_heal_stop_reason {
                                        parts.push(format!("**Self-heal stopped:** {}", reason));
                                    }
                                    if !self_heal_error_signatures.is_empty() {
                                        let signatures = self_heal_error_signatures
                                            .iter()
                                            .map(|s| format!("- `{}`", safe_truncate(s, 220)))
                                            .collect::<Vec<_>>()
                                            .join("\n");
                                        parts.push(format!(
                                            "**Observed failure signatures:**\n{}",
                                            signatures
                                        ));
                                    }
                                }
                            }

                            let code_chars = current_code.trim().chars().count();
                            let code_display = if code_chars <= 2_000 {
                                Some(format!("```{}\n{}\n```", language, current_code.trim()))
                            } else {
                                None
                            };

                            // Show compact code evidence with download link if available.
                            if let Some(file_list) = &files {
                                let code_file = file_list
                                    .iter()
                                    .filter_map(|f| f.as_str())
                                    .find(|f| f.contains("code."));
                                if let Some(cf) = code_file {
                                    if let Some(display) = code_display {
                                        parts.push(format!("{}\n[Download code]({})", display, cf));
                                    } else {
                                        parts.push(format!(
                                            "Corrected/generated code omitted from chat output ({} chars).\n[Download code]({})",
                                            code_chars, cf
                                        ));
                                    }
                                } else if let Some(display) = code_display {
                                    parts.push(display);
                                } else {
                                    parts.push(format!(
                                        "Corrected/generated code omitted from chat output ({} chars).",
                                        code_chars
                                    ));
                                }
                            } else if let Some(display) = code_display {
                                parts.push(display);
                            } else {
                                parts.push(format!(
                                    "Corrected/generated code omitted from chat output ({} chars).",
                                    code_chars
                                ));
                            }

                            if !output.is_empty() {
                                parts.push(format!("**Output:**\n```\n{}\n```", output.trim()));
                            }

                            if let Some(err) = error {
                                if !err.is_empty() {
                                    parts.push(format!("**Errors:**\n```\n{}\n```", err.trim()));
                                }
                            }

                            if exit_code != 0 {
                                parts.push(format!("Exit code: {}", exit_code));
                            }

                            if let Some(file_list) = files {
                                let output_files: Vec<&str> = file_list
                                    .iter()
                                    .filter_map(|f| f.as_str())
                                    .filter(|f| !f.contains("code."))
                                    .collect();
                                if !output_files.is_empty() {
                                    let mut file_parts = Vec::new();
                                    for file_path in &output_files {
                                        let filename =
                                            file_path.rsplit('/').next().unwrap_or(file_path);
                                        let ext = filename
                                            .rsplit('.')
                                            .next()
                                            .unwrap_or("")
                                            .to_lowercase();
                                        let image_exts =
                                            ["png", "jpg", "jpeg", "gif", "svg", "webp", "bmp"];
                                        if image_exts.contains(&ext.as_str()) {
                                            file_parts
                                                .push(format!("![{}]({})", filename, file_path));
                                        } else {
                                            file_parts.push(format!(
                                                "[Download {}]({})",
                                                filename, file_path
                                            ));
                                        }
                                    }
                                    parts.push(format!(
                                        "**Generated Files:**\n{}",
                                        file_parts.join("\n")
                                    ));
                                }
                            }

                            parts.join("\n\n")
                        } else {
                            let mut prefix = String::new();
                            if total_retries > 0 {
                                let mut line = format!(
                                    "*Self-healed after {} attempt{} (still failing)*",
                                    total_retries,
                                    if total_retries == 1 { "" } else { "s" }
                                );
                                if let Some(reason) = &self_heal_stop_reason {
                                    line.push_str(&format!("\n**Self-heal stopped:** {}", reason));
                                }
                                prefix.push_str(&line);
                                prefix.push_str("\n\n");
                            }
                            format!(
                                "{}{}{}",
                                prefix,
                                if current_code.trim().chars().count() <= 2_000 {
                                    format!("```{}\n{}\n```\n\n", language, current_code.trim())
                                } else {
                                    format!(
                                        "Generated code omitted from chat output ({} chars).\n\n",
                                        current_code.trim().chars().count()
                                    )
                                },
                                current_result
                            )
                        };

                        let final_exit_code = parsed_final_result
                            .as_ref()
                            .and_then(|parsed| parsed.get("exit_code"))
                            .and_then(|value| value.as_i64());
                        let completed_successfully = final_exit_code
                            .map(|exit_code| exit_code == 0)
                            .unwrap_or_else(|| {
                                parsed_final_result.is_none()
                                    && total_retries == 0
                                    && self_heal_stop_reason.is_none()
                            });
                        let completion_status = if completed_successfully {
                            "completed"
                        } else {
                            "failed"
                        };
                        let completion_data = serde_json::json!({
                            "success": completed_successfully,
                            "retryable": false,
                            "exit_code": final_exit_code,
                            "self_heal_attempts": total_retries,
                            "self_heal_stop_reason": self_heal_stop_reason,
                            "observed_failure_signatures": self_heal_error_signatures,
                        });
                        results.push(render_tool_completion_marker_with_data(
                            "code_execute",
                            completion_status,
                            &formatted,
                            completion_data,
                        ));
                        continue;
                    }

                    // Format gmail_scan results with LLM classification + summary
                    if call.name == "gmail_scan" {
                        let scan_args = serde_json::from_value::<
                            crate::actions::gmail::GmailScanArgs,
                        >(call.arguments.clone())
                        .ok();
                        let scan_mode = scan_args
                            .as_ref()
                            .map(crate::actions::gmail::effective_scan_mode)
                            .unwrap_or(crate::actions::gmail::GmailScanMode::Triage);
                        let parsed_messages = parse_gmail_scan_messages(&result);

                        if !parsed_messages.is_empty()
                            && matches!(
                                scan_mode,
                                crate::actions::gmail::GmailScanMode::Recent
                                    | crate::actions::gmail::GmailScanMode::Search
                            )
                        {
                            if let Some(ref tx) = stream_tx {
                                queue_stream_event(
                                    tx,
                                    StreamEvent::ToolResult {
                                        name: call.name.clone(),
                                        content: format!(
                                            "Gmail scan returned {} exact result{}",
                                            parsed_messages.len(),
                                            if parsed_messages.len() == 1 { "" } else { "s" }
                                        ),
                                    },
                                );
                            }
                            results.push(format_gmail_scan_exact_results(
                                scan_mode,
                                scan_args.as_ref(),
                                &parsed_messages,
                            ));
                            continue;
                        }

                        let email_format_hint = {
                            let profile = self.user_profile.read().await;
                            profile.email_format.clone().unwrap_or_default()
                        };
                        let format_extra = if email_format_hint.is_empty() {
                            String::new()
                        } else {
                            format!("\nUser preference: {}", email_format_hint)
                        };

                        let format_prompt = format!(
                            "Here are raw email results from Gmail. Classify, summarize, and format them.\n\
                            Rules:\n\
                            - Group into categories with **bold** headers: Action Needed, Security Alerts, Receipts & Orders, Newsletters & Promotions, Other\n\
                            - Skip empty categories\n\
                            - For each email: show sender name (not full email address), subject, and a brief one-line summary/gist\n\
                            - Flag anything time-sensitive or requiring action\n\
                            - Use markdown: **bold** for headers, bullet points for items\n\
                            - Be concise - no raw headers, no IDs, no label dumps\n\
                            {}\n\n\
                            Raw email data:\n{}",
                            format_extra, result
                        );

                        let empty_actions: Vec<crate::actions::ActionDef> = Vec::new();
                        let raw_email_requires_approval = self
                            .tool_output_requires_sensitive_context_approval(&result, false)
                            .is_some();
                        if raw_email_requires_approval {
                            if let Some(ref tx) = stream_tx {
                                queue_stream_event(
                                    tx,
                                    StreamEvent::ToolResult {
                                        name: call.name.clone(),
                                        content: "Gmail scan returned sensitive results awaiting approval".to_string(),
                                    },
                                );
                            }
                            results.push(result);
                            continue;
                        }
                        match self
                            .llm
                            .chat_for_helper_request(
                                "You are a concise email assistant. Format email summaries with clear categorization. Use markdown.",
                                &format_prompt,
                                &[],
                                &empty_actions,
                                &self.config.model_privacy,
                                false,
                            )
                            .await
                        {
                            Ok(formatted) => {
                                self.record_llm_usage(request_channel, "gmail_format", &formatted).await;
                                if let Some(ref tx) = stream_tx {
                                    queue_stream_event(tx, StreamEvent::ToolResult {
                                        name: call.name.clone(),
                                        content: "Gmail scan summarized".to_string(),
                                    });
                                }
                                results.push(formatted.content);
                            }
                            Err(e) => {
                                tracing::warn!("Gmail format LLM pass failed, using raw: {}", e);
                                if let Some(ref tx) = stream_tx {
                                    queue_stream_event(tx, StreamEvent::ToolResult {
                                        name: call.name.clone(),
                                        content: "Gmail scan returned raw results".to_string(),
                                    });
                                }
                                results.push(result);
                            }
                        }
                        continue;
                    }

                    if let Ok(parsed) = serde_json::from_str::<serde_json::Value>(&result) {
                        let status = parsed
                            .get("status")
                            .and_then(|value| value.as_str())
                            .unwrap_or("");
                        if status == "needs_credentials" {
                            if let Some(integration_id) = parsed
                                .get("integration_id")
                                .and_then(|value| value.as_str())
                                .map(str::trim)
                                .filter(|value| !value.is_empty())
                            {
                                if let Some(cid) =
                                    conversation_id.filter(|value| !value.trim().is_empty())
                                {
                                    self.remember_integration_auth_chat_prompt(
                                        cid,
                                        integration_id,
                                        Some(&call.name),
                                        None,
                                    )
                                    .await;
                                }
                                let display_name = self
                                    .lookup_integration_auth_manifest(integration_id)
                                    .await
                                    .map(|manifest| manifest.display_name)
                                    .or_else(|| {
                                        parsed
                                            .get("custom_messaging_channel")
                                            .and_then(|value| value.get("name"))
                                            .and_then(|value| value.as_str())
                                            .map(str::to_string)
                                    })
                                    .unwrap_or_else(|| "This connection".to_string());
                                let prompt = format!(
                                    "{} needs credentials before I can use it.\n\nUse the secure credential form that just appeared in this chat. The values are stored encrypted and are not sent to the assistant.",
                                    display_name
                                );
                                if let Some(ref tx) = stream_tx {
                                    queue_stream_event(
                                        tx,
                                        StreamEvent::ToolResult {
                                            name: call.name.clone(),
                                            content: prompt.clone(),
                                        },
                                    );
                                }
                                results.push(prompt);
                                continue;
                            }
                        }
                    }

                    if call.name == "extension_pack_connect" {
                        if let Ok(parsed) = serde_json::from_str::<serde_json::Value>(&result) {
                            let status = parsed
                                .get("status")
                                .and_then(|value| value.as_str())
                                .unwrap_or("");
                            if status == "needs_credentials" {
                                let pack_id = parsed
                                    .get("pack_id")
                                    .and_then(|value| value.as_str())
                                    .or_else(|| {
                                        call.arguments
                                            .get("pack_id")
                                            .and_then(|value| value.as_str())
                                    })
                                    .unwrap_or("integration");
                                let pack_name = parsed
                                    .get("pack_name")
                                    .and_then(|value| value.as_str())
                                    .filter(|value| !value.trim().is_empty())
                                    .unwrap_or(pack_id);
                                let connection_id = parsed
                                    .get("connection")
                                    .and_then(|value| value.get("connection"))
                                    .and_then(|value| value.get("id"))
                                    .and_then(|value| value.as_str())
                                    .unwrap_or_default();
                                let required_secrets = parsed
                                    .get("required_secrets")
                                    .and_then(|value| value.as_array())
                                    .map(|items| {
                                        items
                                            .iter()
                                            .filter_map(|value| value.as_str())
                                            .map(|value| value.to_string())
                                            .collect::<Vec<_>>()
                                    })
                                    .unwrap_or_default();
                                if let Some(cid) =
                                    conversation_id.filter(|value| !value.trim().is_empty())
                                {
                                    self.remember_extension_pack_chat_credential_prompt(
                                        cid,
                                        pack_id,
                                        pack_name,
                                        connection_id,
                                        &required_secrets,
                                    )
                                    .await;
                                }
                                let mut prompt = format!(
                                    "{} is installed, but it still needs credentials before I can use it.\n\nUse the secure credential form that just appeared in this chat. Never paste secrets/API Keys/Password/Sensitive Data into normal chat.",
                                    pack_name
                                );
                                if !required_secrets.is_empty() {
                                    prompt.push_str("\n\nRequired values:");
                                    for key in &required_secrets {
                                        prompt.push_str(&format!("\n- `{}`", key));
                                    }
                                }
                                if let Some(ref tx) = stream_tx {
                                    queue_stream_event(
                                        tx,
                                        StreamEvent::ToolResult {
                                            name: call.name.clone(),
                                            content: prompt.clone(),
                                        },
                                    );
                                }
                                results.push(prompt);
                                continue;
                            }
                            if let Some(cid) =
                                conversation_id.filter(|value| !value.trim().is_empty())
                            {
                                self.clear_extension_pack_chat_credential_prompt(cid).await;
                            }
                        }
                    }

                    if let Some(payload) = parse_workflow_missing_inputs_marker(&result) {
                        if !payload.sensitive_missing.is_empty() {
                            if let Some(cid) =
                                conversation_id.filter(|value| !value.trim().is_empty())
                            {
                                self.remember_pending_secret_followup(
                                    cid,
                                    PendingSecretFollowupKind::RetryWorkflow {
                                        payload: payload.clone(),
                                    },
                                )
                                .await;
                            }
                        }
                        let prompt = Self::format_missing_inputs_prompt(&payload);
                        if let Some(ref tx) = stream_tx {
                            queue_stream_event(
                                tx,
                                StreamEvent::ToolResult {
                                    name: call.name.clone(),
                                    content: prompt.clone(),
                                },
                            );
                        }
                        results.push(prompt);
                        continue;
                    }

                    // Check if this is a workflow action that needs LLM orchestration
                    if let Some((action_name, user_query)) = parse_workflow_action_marker(&result) {
                        match self
                            .execute_workflow_marker_action(&action_name, &user_query)
                            .await
                        {
                            Ok(llm_result) => {
                                if let Some(ref tx) = stream_tx {
                                    queue_stream_event(
                                        tx,
                                        StreamEvent::ToolResult {
                                            name: call.name.clone(),
                                            content: format!(
                                                "Workflow '{}' completed",
                                                action_name
                                            ),
                                        },
                                    );
                                }
                                results.push(llm_result);
                            }
                            Err(e) => {
                                tracing::error!("Workflow action execution error: {}", e);
                                let formatted =
                                    format!("Error executing workflow '{}': {}", action_name, e);
                                if let Some(ref tx) = stream_tx {
                                    queue_stream_event(
                                        tx,
                                        StreamEvent::ToolResult {
                                            name: call.name.clone(),
                                            content: formatted.clone(),
                                        },
                                    );
                                }
                                results.push(formatted);
                            }
                        }
                        continue;
                    }

                    if let Some(ref tx) = stream_tx {
                        queue_stream_event(
                            tx,
                            StreamEvent::ToolResult {
                                name: call.name.clone(),
                                content: sanitize_stream(&result),
                            },
                        );
                    }
                    results.push(result);
                }
                Err(e) => {
                    tracing::error!("Action execution error: {}", e);
                    if let Some(missing) =
                        e.downcast_ref::<crate::runtime::MissingSecretPlaceholder>()
                    {
                        if let Some((prompt, _)) = self
                            .raise_missing_secret_chat_prompt(
                                missing,
                                conversation_id,
                                &call.name,
                                None,
                                stream_tx.as_ref(),
                            )
                            .await
                        {
                            results.push(prompt);
                            continue;
                        }
                    }
                    if call.name == "browse" {
                        let target = call
                            .arguments
                            .get("url")
                            .and_then(|v| v.as_str())
                            .unwrap_or("")
                            .trim();
                        let query_hint = if target.is_empty() {
                            call.arguments
                                .get("query")
                                .and_then(|v| v.as_str())
                                .unwrap_or("")
                                .trim()
                        } else {
                            target
                        };

                        if !query_hint.is_empty() {
                            let fallback_args = serde_json::json!({
                                "query": query_hint,
                                "num_results": 5
                            });
                            match self
                                .runtime
                                .execute_action("web_search", &fallback_args)
                                .await
                            {
                                Ok(search_out) => {
                                    let browse_error = present_user_safe_tool_failure(&e)
                                        .map(|failure| failure.message)
                                        .unwrap_or_else(|| e.to_string());
                                    let healed = format!(
                                        "Browse failed ({})\n\nSelf-heal fallback: searched the web instead.\n{}",
                                        browse_error, search_out
                                    );
                                    if let Some(ref tx) = stream_tx {
                                        queue_stream_event(
                                            tx,
                                            StreamEvent::ToolResult {
                                                name: call.name.clone(),
                                                content: "Browse failed; used search fallback"
                                                    .to_string(),
                                            },
                                        );
                                    }
                                    results.push(healed);
                                    continue;
                                }
                                Err(search_err) => {
                                    tracing::warn!(
                                        "Browse self-heal fallback failed for '{}': {}",
                                        query_hint,
                                        search_err
                                    );
                                }
                            }
                        }
                    }
                    let detail = present_user_safe_tool_failure(&e)
                        .map(|failure| failure.message)
                        .unwrap_or_else(|| format!("Error executing '{}': {}", call.name, e));
                    let formatted = render_tool_completion_marker_with_data(
                        &call.name,
                        "failed",
                        &detail,
                        serde_json::json!({
                            "error": e.to_string(),
                        }),
                    );
                    if let Some(ref tx) = stream_tx {
                        queue_stream_event(
                            tx,
                            StreamEvent::ToolResult {
                                name: call.name.clone(),
                                content: detail,
                            },
                        );
                    }
                    results.push(formatted);
                }
            }
        }

        // If there's content plus tool results, combine them
        if response.content.is_empty() {
            Ok(results.join("\n"))
        } else {
            Ok(format!("{}\n\n{}", response.content, results.join("\n")))
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;
    use std::path::PathBuf;

    fn call(id: &str, name: &str, arguments: serde_json::Value) -> crate::core::llm::ToolCall {
        crate::core::llm::ToolCall {
            id: id.to_string(),
            name: name.to_string(),
            arguments,
        }
    }

    fn safety_engine_with_require_approval_rule(
        action_name: &str,
    ) -> (tempfile::TempDir, crate::safety::SafetyEngine) {
        let temp = tempfile::tempdir().expect("tempdir");
        let safety = crate::safety::SafetyEngine::new(temp.path()).expect("safety engine");
        safety.add_rule(crate::safety::SafetyRule {
            name: format!("permission_gate_{}", action_name),
            description: format!("Require approval for {}", action_name),
            trigger: crate::safety::RuleTrigger::Action {
                name: action_name.to_string(),
            },
            condition: None,
            action: crate::safety::RuleAction::RequireApproval,
            verified: true,
        });
        (temp, safety)
    }

    #[test]
    fn user_safe_tool_failure_hides_internal_paths_for_workspace_scope_errors() {
        let error = anyhow::Error::new(crate::runtime::ToolPathAccessError::OutsideAllowedRoots {
            attempted_path: PathBuf::from("/tmp"),
            allowed_roots: vec![
                PathBuf::from("/app/data"),
                PathBuf::from("/workspace/agentark"),
            ],
        });

        let presented = present_user_safe_tool_failure(&error).expect("user-safe failure");
        assert_eq!(
            presented.failure_class,
            crate::core::FailureClass::Validation
        );
        assert!(presented.retryable);
        assert!(presented.message.contains("workspace"));
        assert!(!presented.message.contains("/tmp"));
        assert!(!presented.message.contains("/app/data"));
        assert!(!presented.message.contains("/workspace/agentark"));
    }

    #[test]
    fn code_execute_data_path_without_uploads_is_not_retryable_scratch() {
        assert!(code_execute_uses_data_path_without_inputs(
            &json!({}),
            "with open('/data/index.html', 'w') as f:\n    f.write('<html></html>')"
        ));
        assert!(!code_execute_uses_data_path_without_inputs(
            &json!({}),
            "with open('/workspace/index.html', 'w') as f:\n    f.write('<html></html>')"
        ));
        assert!(!code_execute_uses_data_path_without_inputs(
            &json!({"file_payloads": [{"filename": "input.txt", "bytes_b64": "aGk="}]}),
            "print(open('/data/input.txt').read())"
        ));
    }

    #[test]
    fn tool_call_signature_ignores_object_key_order() {
        let a = call(
            "1",
            "app_deploy",
            json!({
                "files": {"index.html": "<h1>ok</h1>"},
                "title": "demo",
                "config": {"a": 1, "b": 2}
            }),
        );
        let b = call(
            "2",
            "app_deploy",
            json!({
                "config": {"b": 2, "a": 1},
                "title": "demo",
                "files": {"index.html": "<h1>ok</h1>"}
            }),
        );

        assert_eq!(
            Agent::tool_call_signature(&a),
            Agent::tool_call_signature(&b)
        );
    }

    #[test]
    fn tool_call_signature_preserves_array_order() {
        let a = call("1", "code_execute", json!({ "args": [1, 2, 3] }));
        let b = call("2", "code_execute", json!({ "args": [3, 2, 1] }));

        assert_ne!(
            Agent::tool_call_signature(&a),
            Agent::tool_call_signature(&b)
        );
    }

    #[test]
    fn select_best_ranked_app_requires_confirmation_for_close_deployed_app_matches() {
        let apps = vec![
            json!({
                "id": "becf46bb",
                "title": "arXiv Live Feed"
            }),
            json!({
                "id": "cad20c5e",
                "title": "arXiv Live Papers"
            }),
            json!({
                "id": "other1234",
                "title": "Weather Dashboard"
            }),
        ];

        let ambiguous_ranked = Agent::rank_deployed_apps("arxiv", &apps);
        assert!(
            Agent::select_best_ranked_app("arxiv", &ambiguous_ranked).is_none(),
            "generic references should require confirmation when multiple deployed apps match closely"
        );

        let exact_ranked = Agent::rank_deployed_apps("becf46bb", &apps);
        let exact_match = Agent::select_best_ranked_app("becf46bb", &exact_ranked)
            .expect("exact app id should still resolve without clarification");
        assert_eq!(exact_match.1, "becf46bb");
    }

    #[test]
    fn normalize_app_deploy_arguments_unwraps_double_encoded_payload() {
        let payload = "\"{\\\"title\\\":\\\"Demo\\\",\\\"files\\\":{\\\"index.html\\\":\\\"<h1>ok</h1>\\\"}}\"";
        let input = json!({
            "name": "app_deploy",
            "payload": payload,
            "runtime_preference": "local"
        });

        let normalized = Agent::normalize_app_deploy_arguments(&input);
        let files = normalized
            .get("files")
            .and_then(|v| v.as_object())
            .expect("files object should be recovered");
        assert_eq!(
            files.get("index.html").and_then(|v| v.as_str()),
            Some("<h1>ok</h1>")
        );
        assert_eq!(
            normalized
                .get("runtime_preference")
                .and_then(|v| v.as_str()),
            Some("local")
        );
    }

    #[test]
    fn normalize_app_deploy_arguments_converts_file_array_to_files_map() {
        let input = json!({
            "payload": {
                "title": "Demo",
                "project_files": [
                    { "name": "index.html", "content": "<h1>x</h1>" },
                    { "name": "app.js", "content": "console.log('ok')" }
                ]
            }
        });

        let normalized = Agent::normalize_app_deploy_arguments(&input);
        let files = normalized
            .get("files")
            .and_then(|v| v.as_object())
            .expect("files map should be built from project_files");
        assert_eq!(files.len(), 2);
        assert_eq!(
            files.get("index.html").and_then(|v| v.as_str()),
            Some("<h1>x</h1>")
        );
        assert_eq!(
            files.get("app.js").and_then(|v| v.as_str()),
            Some("console.log('ok')")
        );
    }

    #[test]
    fn normalize_app_deploy_arguments_merges_file_siblings_into_files_map() {
        let input = json!({
            "title": "Enterprise app",
            "files": {
                "index.html": "<!doctype html><html><head><link rel=\"stylesheet\" href=\"style.css\"></head><body><script type=\"module\" src=\"src/App.tsx\"></script></body></html>"
            },
            "style.css": "body { margin: 0; background: #050816; }",
            "src/App.tsx": "export default function App() { return <main />; }",
            "backend/main.py": "from fastapi import FastAPI\napp = FastAPI()\n"
        });

        let normalized = Agent::normalize_app_deploy_arguments(&input);
        let files = normalized
            .get("files")
            .and_then(|v| v.as_object())
            .expect("files object should be preserved and extended");

        assert_eq!(
            files.get("index.html").and_then(|v| v.as_str()),
            Some(
                "<!doctype html><html><head><link rel=\"stylesheet\" href=\"style.css\"></head><body><script type=\"module\" src=\"src/App.tsx\"></script></body></html>"
            )
        );
        assert_eq!(
            files.get("style.css").and_then(|v| v.as_str()),
            Some("body { margin: 0; background: #050816; }")
        );
        assert_eq!(
            files.get("src/App.tsx").and_then(|v| v.as_str()),
            Some("export default function App() { return <main />; }")
        );
        assert_eq!(
            files.get("backend/main.py").and_then(|v| v.as_str()),
            Some("from fastapi import FastAPI\napp = FastAPI()\n")
        );
    }

    #[test]
    fn normalize_app_deploy_arguments_merges_root_file_siblings_into_nested_payload() {
        let input = json!({
            "payload": {
                "title": "Nested full-stack app",
                "files": {
                    "frontend/index.html": "<!doctype html><html><body><script type=\"module\" src=\"src/main.ts\"></script></body></html>"
                },
                "frontend/src/main.ts": "console.log('nested')"
            },
            "frontend/package.json": "{\"scripts\":{\"dev\":\"vite\"}}",
            "backend/app.py": "print('server')"
        });

        let normalized = Agent::normalize_app_deploy_arguments(&input);
        let files = normalized
            .get("files")
            .and_then(|v| v.as_object())
            .expect("nested files object should be recovered");

        assert_eq!(
            files.get("frontend/index.html").and_then(|v| v.as_str()),
            Some(
                "<!doctype html><html><body><script type=\"module\" src=\"src/main.ts\"></script></body></html>"
            )
        );
        assert_eq!(
            files.get("frontend/src/main.ts").and_then(|v| v.as_str()),
            Some("console.log('nested')")
        );
        assert_eq!(
            files.get("frontend/package.json").and_then(|v| v.as_str()),
            Some("{\"scripts\":{\"dev\":\"vite\"}}")
        );
        assert_eq!(
            files.get("backend/app.py").and_then(|v| v.as_str()),
            Some("print('server')")
        );
    }

    #[test]
    fn normalize_app_deploy_arguments_does_not_create_missing_asset_fallbacks() {
        let input = json!({
            "files": {
                "index.html": "<!doctype html><html><head><link rel=\"stylesheet\" href=\"style.css\"></head><body>demo</body></html>"
            }
        });

        let normalized = Agent::normalize_app_deploy_arguments(&input);
        let files = normalized
            .get("files")
            .and_then(|v| v.as_object())
            .expect("files object should remain present");

        assert!(files.get("style.css").is_none());
        assert_eq!(files.len(), 1);
    }

    #[test]
    fn normalize_app_deploy_arguments_preserves_repo_source_metadata() {
        let input = json!({
            "payload": {
                "repo_url": "https://github.com/example/demo",
                "repo_ref": "main",
                "service_mode": "fullstack"
            },
            "runtime_preference": "container"
        });

        let normalized = Agent::normalize_app_deploy_arguments(&input);
        assert_eq!(
            normalized.get("repo_url").and_then(|v| v.as_str()),
            Some("https://github.com/example/demo")
        );
        assert_eq!(
            normalized.get("repo_ref").and_then(|v| v.as_str()),
            Some("main")
        );
        assert_eq!(
            normalized.get("service_mode").and_then(|v| v.as_str()),
            Some("fullstack")
        );
        assert_eq!(
            normalized
                .get("runtime_preference")
                .and_then(|v| v.as_str()),
            Some("container")
        );
    }

    #[test]
    fn normalize_app_deploy_arguments_rejects_generic_prose_recovery() {
        let input = json!({
            "description": "Build a dashboard that shows live crypto prices",
            "title": "Demo"
        });

        let normalized = Agent::normalize_app_deploy_arguments(&input);
        assert!(normalized.get("files").is_none());
        assert_eq!(
            normalized.get("description").and_then(|v| v.as_str()),
            Some("Build a dashboard that shows live crypto prices")
        );
    }

    #[test]
    fn browser_auto_start_session_suppresses_manual_browser_calls_in_same_batch() {
        let browser_auto = call(
            "1",
            "browser_auto",
            json!({
                "action": "start_session",
                "task": "Open Hacker News login page and wait for handoff"
            }),
        );
        let browser = call(
            "2",
            "browser",
            json!({
                "action": "create_session",
                "url": "https://news.ycombinator.com/login"
            }),
        );
        let browser_with_session = call(
            "3",
            "browser",
            json!({
                "action": "navigate",
                "session_id": "existing-session",
                "url": "https://news.ycombinator.com/login"
            }),
        );

        assert_eq!(browser.name, "browser");
        assert_eq!(browser_auto.name, "browser_auto");
        assert_eq!(
            browser_with_session
                .arguments
                .get("session_id")
                .and_then(|value| value.as_str()),
            Some("existing-session")
        );
    }

    #[tokio::test]
    async fn legacy_safety_allows_direct_trusted_chat_bypass() {
        let (_temp, safety) = safety_engine_with_require_approval_rule("app_deploy");
        let deploy_call = call(
            "1",
            "app_deploy",
            json!({ "repo_url": "https://github.com/example/repo-template" }),
        );
        let direct_chat = crate::actions::ActionAuthorizationContext {
            principal: Some(crate::actions::ActionCallerPrincipal::local_admin("web")),
            surface: crate::actions::ActionExecutionSurface::Chat,
            direct_user_intent: true,
            current_turn_is_explicit_approval: false,
            agent_name: None,
            agent_access_scope: None,
            capability_context_id: None,
        };

        assert!(Agent::legacy_tool_call_allowed_by_safety(
            &safety,
            &deploy_call,
            Some(&direct_chat)
        )
        .await
        .expect("legacy safety check should succeed"));
        assert!(
            !Agent::legacy_tool_call_allowed_by_safety(&safety, &deploy_call, None)
                .await
                .expect("legacy safety check should succeed")
        );
    }

    #[tokio::test]
    async fn legacy_safety_honors_auto_approved_actions() {
        let (_temp, safety) = safety_engine_with_require_approval_rule("app_deploy");
        safety.set_auto_approved(&[String::from("app_deploy")]);
        let deploy_call = call(
            "1",
            "app_deploy",
            json!({ "repo_url": "https://github.com/example/repo-template" }),
        );

        assert!(
            Agent::legacy_tool_call_allowed_by_safety(&safety, &deploy_call, None)
                .await
                .expect("legacy safety check should succeed")
        );
    }

    #[tokio::test]
    async fn legacy_safety_allows_direct_trusted_chat_for_generated_app_file_write() {
        let (_temp, safety) = safety_engine_with_require_approval_rule("file_write");
        let file_write_call = call(
            "1",
            "file_write",
            json!({
                "path": "/app/data/apps/new/arxiv-board/index.html",
                "content": "<!doctype html><title>Latest ArXiv</title>"
            }),
        );
        let direct_chat = crate::actions::ActionAuthorizationContext {
            principal: Some(crate::actions::ActionCallerPrincipal::local_admin("web")),
            surface: crate::actions::ActionExecutionSurface::Chat,
            direct_user_intent: true,
            current_turn_is_explicit_approval: false,
            agent_name: None,
            agent_access_scope: None,
            capability_context_id: None,
        };

        assert!(Agent::legacy_tool_call_allowed_by_safety(
            &safety,
            &file_write_call,
            Some(&direct_chat),
        )
        .await
        .expect("legacy safety check should succeed"));
        assert!(
            !Agent::legacy_tool_call_allowed_by_safety(&safety, &file_write_call, None)
                .await
                .expect("legacy safety check should succeed")
        );
    }

    #[test]
    fn app_runtime_error_marker_ignores_generic_retry_copy() {
        assert!(
            Agent::detect_app_runtime_error_marker("please try again to refresh data").is_none()
        );
        assert!(
            Agent::detect_app_runtime_error_marker("this guide explains cors headers").is_none()
        );
        assert_eq!(
            Agent::detect_app_runtime_error_marker("application error: failed to load"),
            Some("failed to load")
        );
    }

    #[test]
    fn http_probe_error_marker_ignores_static_html_error_templates() {
        let html = r#"<!doctype html>
<html>
  <body>
    <div id="error" hidden>Failed to fetch papers. Retrying in 10s...</div>
  </body>
</html>"#;

        assert!(Agent::detect_http_probe_runtime_error_marker("text/html", html).is_none());
        assert_eq!(
            Agent::detect_http_probe_runtime_error_marker(
                "application/json",
                r#"{"error":"failed to fetch"}"#,
            ),
            Some("failed to fetch")
        );
    }

    #[test]
    fn structural_app_probe_accepts_valid_static_html() {
        let result = Agent::validate_structural_app_probe_body(
            reqwest::StatusCode::OK,
            "static",
            "text/html; charset=utf-8",
            "<!doctype html><html><body><main>Ready</main></body></html>",
        );

        assert!(result.is_ok());
    }

    #[test]
    fn structural_app_probe_rejects_empty_body() {
        let result = Agent::validate_structural_app_probe_body(
            reqwest::StatusCode::OK,
            "static",
            "text/html",
            "   ",
        );

        assert!(result
            .expect_err("empty body should fail")
            .contains("empty body"));
    }

    #[test]
    fn structural_app_probe_rejects_non_html_static_response() {
        let result = Agent::validate_structural_app_probe_body(
            reqwest::StatusCode::OK,
            "static",
            "application/json",
            r#"{"ok":true}"#,
        );

        assert!(result
            .expect_err("static app should return html")
            .contains("did not receive HTML"));
    }

    #[test]
    fn structural_app_probe_rejects_unclosed_raw_text_html() {
        let result = Agent::validate_structural_app_probe_body(
            reqwest::StatusCode::OK,
            "static",
            "text/html",
            "<html><body><script>const x = 1;</body></html>",
        );

        assert!(result
            .expect_err("unclosed script should fail")
            .contains("unclosed <script>"));
    }

    #[test]
    fn resolve_duplicate_app_reuses_only_exact_files_with_live_runtime() {
        assert_eq!(
            Agent::resolve_duplicate_app("exact_files", true),
            DuplicateAppResolution::ReuseExisting
        );
        assert_eq!(
            Agent::resolve_duplicate_app("exact_files", false),
            DuplicateAppResolution::ReplaceExisting
        );
    }

    #[test]
    fn resolve_duplicate_app_asks_before_non_exact_matches() {
        assert_eq!(
            Agent::resolve_duplicate_app("exact_title", true),
            DuplicateAppResolution::NeedsClarification
        );
        assert_eq!(
            Agent::resolve_duplicate_app("fuzzy", true),
            DuplicateAppResolution::NeedsClarification
        );
    }

    #[test]
    fn gmail_exact_formatter_preserves_order_and_count() {
        let raw = "- From: Alice Example <alice@example.com>\n  Subject: Quarterly update\n  Date: Fri, 28 Mar 2026 12:00:00 +0530\n  Labels: INBOX\n  Id: 1\n  ThreadId: t1\n  Snippet: Revenue is up.\n\n- From: Bob Example <bob@example.com>\n  Subject: Meeting invite\n  Date: Fri, 28 Mar 2026 11:00:00 +0530\n  Labels: INBOX\n  Id: 2\n  ThreadId: t2\n  Snippet: Please join at 2 PM.";
        let parsed = parse_gmail_scan_messages(raw);
        assert_eq!(parsed.len(), 2);

        let args = crate::actions::gmail::GmailScanArgs {
            mode: crate::actions::gmail::GmailScanMode::Recent,
            query: None,
            labels: Vec::new(),
            max_results: Some(2),
        };
        let formatted = format_gmail_scan_exact_results(
            crate::actions::gmail::GmailScanMode::Recent,
            Some(&args),
            &parsed,
        );

        assert!(formatted.contains("Here are your latest 2 emails:"));
        assert!(formatted.contains("1. **Alice Example** - Quarterly update"));
        assert!(formatted.contains("2. **Bob Example** - Meeting invite"));
        assert!(!formatted.contains("Action Needed"));
        assert!(!formatted.contains("Security Alerts"));
    }
}
