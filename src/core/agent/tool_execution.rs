#![allow(dead_code)]

use super::*;
use anyhow::Context as _;
use std::str::FromStr as _;
use std::sync::atomic::{AtomicBool, Ordering};

static GEPA_IDLE_WORKER_ACTIVE: AtomicBool = AtomicBool::new(false);

struct GepaIdleWorkerActiveGuard;

impl Drop for GepaIdleWorkerActiveGuard {
    fn drop(&mut self) {
        GEPA_IDLE_WORKER_ACTIVE.store(false, Ordering::Release);
    }
}

fn background_session_blocks_gepa_idle(
    session: &crate::core::background_session::BackgroundSession,
    now: chrono::DateTime<chrono::Utc>,
    quiet_window_seconds: i64,
) -> bool {
    if !matches!(
        session.status,
        crate::core::background_session::BackgroundSessionStatus::Active
    ) {
        return false;
    }
    let quiet_window_seconds = quiet_window_seconds.max(60);
    now.signed_duration_since(session.last_activity_at)
        .num_seconds()
        < quiet_window_seconds
}

async fn stable_deployment_cadence_allows_auto_promotion(
    storage: &crate::storage::Storage,
    surface: &str,
    version: &str,
) -> bool {
    match crate::core::self_evolve::deployment_cadence::stable_deployment_cadence_block_for_storage(
        storage,
        chrono::Utc::now(),
    )
    .await
    {
        Ok(Some(block)) => {
            tracing::info!(
                surface = %surface,
                version = %version,
                pause_until = %block.next_allowed_at.to_rfc3339(),
                "Skipping direct stable deployment during Evolve observation window"
            );
            false
        }
        Ok(None) => true,
        Err(error) => {
            tracing::warn!(
                surface = %surface,
                version = %version,
                error = %error,
                "Skipping direct stable deployment because cadence check failed"
            );
            false
        }
    }
}

async fn record_auto_stable_deployment(
    storage: &crate::storage::Storage,
    surface: &str,
    version: &str,
) {
    if let Err(error) = crate::core::self_evolve::deployment_cadence::record_stable_deployment(
        storage,
        crate::core::self_evolve::deployment_cadence::StableDeploymentRecord {
            deployed_at: chrono::Utc::now().to_rfc3339(),
            surface: surface.to_string(),
            action: "auto_promote_from_canary_replay".to_string(),
            proposal_id: None,
            version: Some(version.to_string()),
        },
    )
    .await
    {
        tracing::warn!(
            surface = %surface,
            version = %version,
            error = %error,
            "Failed to record automatic Evolve stable deployment cadence"
        );
    }
}

/// Convert any tool-execution `anyhow::Error` into a structured JSON envelope
/// the LLM can parse without surface-form pattern matching. If the underlying
/// error is a typed `ActionError`, its domain/reason/message and a remediation
/// hint are serialized; otherwise a fallback `Action/Failed` envelope is used.
///
/// Output shape (stable, structural — never depends on user phrasing):
///   {"status":"error","tool":"<name>","domain":"<key>","reason":"<key>",
///    "message":"<text>","remediation":{"type":"...","target":"...?"}}
pub(crate) fn tool_result_envelope_error_string(tool_name: &str, error: &anyhow::Error) -> String {
    if let Some(action_err) = error.downcast_ref::<crate::actions::ActionError>() {
        return action_err.to_envelope(tool_name).to_string();
    }
    if let Some(presented) = present_user_safe_tool_failure(error) {
        return serde_json::json!({
            "status": "error",
            "tool": tool_name,
            "domain": "validation",
            "reason": presented.reason,
            "message": presented.message,
            "failure_class": presented.failure_class,
            "retryable": presented.retryable,
            "operational_outcome": presented.operational_outcome,
            "remediation": {
                "type": if presented.retryable { "use_alternative" } else { "none" }
            }
        })
        .to_string();
    }
    // Try to recover an envelope from a pre-formatted "ERR/domain/reason: msg"
    // string that may have come back from a sub-tool call.
    let message = error.to_string();
    if let Some(parsed) = parse_structured_action_error_text(tool_name, &message) {
        return parsed.to_string();
    }
    serde_json::json!({
        "status": "error",
        "tool": tool_name,
        "domain": "action",
        "reason": "failed",
        "message": message,
        "remediation": {"type": "none"},
    })
    .to_string()
}

/// Convert a successful tool result string into a structured JSON envelope.
/// Used by the new turn loop to give the LLM a uniformly-shaped tool result.
pub(crate) fn tool_result_envelope_ok_string(tool_name: &str, result_text: &str) -> String {
    // If the tool already returned a structured "ERR/..." string, surface it
    // as an error envelope rather than wrapping it as success.
    if crate::actions::is_structured_action_error_text(result_text) {
        if let Some(parsed) = parse_structured_action_error_text(tool_name, result_text) {
            return parsed.to_string();
        }
    }
    if let Some(outcome) = super::tool_responses::structured_tool_result_outcome(result_text) {
        if outcome.state != super::tool_responses::StructuredToolOutcomeState::Success {
            let status = match outcome.state {
                super::tool_responses::StructuredToolOutcomeState::Success => "ok",
                super::tool_responses::StructuredToolOutcomeState::Failure => "error",
                super::tool_responses::StructuredToolOutcomeState::NeedsInput => "needs_input",
            };
            let reason = outcome.reason.unwrap_or_else(|| match outcome.state {
                super::tool_responses::StructuredToolOutcomeState::Success => "success".to_string(),
                super::tool_responses::StructuredToolOutcomeState::Failure => {
                    "reported_failure".to_string()
                }
                super::tool_responses::StructuredToolOutcomeState::NeedsInput => {
                    "needs_input".to_string()
                }
            });
            let message = outcome.message.unwrap_or_else(|| {
                if let Some(exit_code) = outcome.exit_code {
                    format!("The tool exited with code {exit_code}.")
                } else {
                    "The tool reported that the requested action did not complete.".to_string()
                }
            });
            let mut envelope = serde_json::json!({
                "status": status,
                "tool": tool_name,
                "reason": reason,
                "message": message,
                "result": result_text,
            });
            if let Some(exit_code) = outcome.exit_code {
                envelope["exit_code"] = serde_json::json!(exit_code);
            }
            return envelope.to_string();
        }
    }
    serde_json::json!({
        "status": "ok",
        "tool": tool_name,
        "result": result_text,
    })
    .to_string()
}

fn parse_structured_action_error_text(tool_name: &str, message: &str) -> Option<serde_json::Value> {
    // Format: "ERR/<domain>/<reason>: <text>"
    let trimmed = message.trim_start();
    let rest = trimmed.strip_prefix("ERR/")?;
    let (domain_str, after_domain) = rest.split_once('/')?;
    let (reason_str, message_part) = after_domain.split_once(": ")?;
    let domain = crate::actions::ActionErrorDomain::from_str(domain_str.trim()).ok()?;
    let reason = crate::actions::ActionErrorReason::from_str(reason_str.trim()).ok()?;
    let action_err = crate::actions::ActionError::new(domain, reason, message_part);
    Some(action_err.to_envelope(tool_name))
}

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
        "repeat_on_match": watcher.repeat_on_match,
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
        "repeat_on_match": serde_json::Value::Null,
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
    reason: &'static str,
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
        .map(|status| matches!(status, "failed" | "timed_out" | "error" | "blocked"))
        .unwrap_or(false)
}

fn gepa_job_failure_retryable(value: &serde_json::Value) -> bool {
    value
        .get("status")
        .and_then(|status| status.as_str())
        .map(|status| !matches!(status, "failed" | "timed_out" | "error" | "blocked"))
        .unwrap_or(true)
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

fn gepa_prompt_optimization_metadata(
    proposal_id: &str,
    opportunity_context: Option<serde_json::Value>,
) -> serde_json::Value {
    let mut metadata = serde_json::Map::new();
    metadata.insert(
        "source".to_string(),
        serde_json::Value::String("prompt_optimization_approval".to_string()),
    );
    metadata.insert(
        "proposal_id".to_string(),
        serde_json::Value::String(proposal_id.trim().to_string()),
    );
    if let Some(context) = opportunity_context {
        if !context.is_null() {
            metadata.insert("opportunity_context".to_string(), context);
        }
    }
    serde_json::Value::Object(metadata)
}

fn gepa_prompt_optimization_confidence_sample_target(
    opportunity_context: Option<&serde_json::Value>,
) -> Option<usize> {
    opportunity_context
        .and_then(|context| context.get("confidence"))
        .and_then(|confidence| confidence.get("confidence_sample_target"))
        .and_then(|value| value.as_u64())
        .map(|value| value.min(20_000) as usize)
        .filter(|value| *value > 0)
}

fn gepa_prompt_optimization_promotion_settings(
    opportunity_context: Option<&serde_json::Value>,
) -> crate::core::self_evolve::gepa_bridge::GepaPromotionSettings {
    crate::core::self_evolve::gepa_bridge::GepaPromotionSettings {
        apply_promotion: true,
        canary_rollout_percent: 20,
        canary_min_samples_per_version: gepa_prompt_optimization_confidence_sample_target(
            opportunity_context,
        )
        .unwrap_or(25)
        .max(25),
        canary_min_success_gain: 0.03,
        canary_max_sign_test_p_value: 0.10,
        replay_log_limit: 160,
    }
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
        .or_else(|| {
            error
                .downcast_ref::<crate::runtime::ToolPathAccessError>()
                .map(|error| TypedToolErrorFields {
                    code: match error {
                        crate::runtime::ToolPathAccessError::OutsideAllowedRoots { .. } => {
                            "runtime/outside_allowed_roots".to_string()
                        }
                    },
                    scope: Some("filesystem".to_string()),
                    detail: Some(error.to_string()),
                })
        })
        .or_else(|| {
            error
                .downcast_ref::<std::io::Error>()
                .map(|error| TypedToolErrorFields {
                    code: match error.kind() {
                        std::io::ErrorKind::NotFound => "io/not_found".to_string(),
                        std::io::ErrorKind::PermissionDenied => "io/permission_denied".to_string(),
                        _ => "io/failed".to_string(),
                    },
                    scope: Some("filesystem".to_string()),
                    detail: Some(error.to_string()),
                })
        })
}

fn present_user_safe_tool_failure(error: &anyhow::Error) -> Option<UserSafeToolFailure> {
    if let Some(error) = error.downcast_ref::<crate::runtime::ToolPathAccessError>() {
        return match error {
            crate::runtime::ToolPathAccessError::OutsideAllowedRoots { .. } => {
                Some(UserSafeToolFailure {
                    message: "The requested file path is not available in this runtime. It can only access files inside the workspace and configured data directories.".to_string(),
                    reason: "outside_allowed_roots",
                    failure_class: crate::core::FailureClass::Validation,
                    retryable: true,
                    operational_outcome: "validation_error",
                })
            }
        };
    }
    error
        .downcast_ref::<std::io::Error>()
        .and_then(|error| match error.kind() {
            std::io::ErrorKind::NotFound => Some(UserSafeToolFailure {
                message: "The requested file does not exist in the current runtime.".to_string(),
                reason: "not_found",
                failure_class: crate::core::FailureClass::Validation,
                retryable: true,
                operational_outcome: "validation_error",
            }),
            std::io::ErrorKind::PermissionDenied => Some(UserSafeToolFailure {
                message: "The requested file path is not readable in the current runtime."
                    .to_string(),
                reason: "permission_denied",
                failure_class: crate::core::FailureClass::Validation,
                retryable: true,
                operational_outcome: "validation_error",
            }),
            _ => None,
        })
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

#[derive(Debug, Clone, PartialEq, Eq)]
struct AppDeployQualityContract {
    request_context: String,
    sub_goals: Vec<String>,
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

fn work_manage_operation(arguments: &serde_json::Value) -> String {
    arguments
        .get("operation")
        .or_else(|| arguments.get("op"))
        .and_then(|value| value.as_str())
        .map(str::trim)
        .filter(|value| !value.is_empty())
        .map(|value| match value.to_ascii_lowercase().as_str() {
            "read" => "status".to_string(),
            other => other.to_string(),
        })
        .unwrap_or_else(|| "status".to_string())
}

fn work_manage_id(arguments: &serde_json::Value, primary_field: &str) -> Option<String> {
    arguments
        .get(primary_field)
        .or_else(|| arguments.get("id"))
        .or_else(|| arguments.get("work_id"))
        .and_then(|value| value.as_str())
        .map(str::trim)
        .filter(|value| !value.is_empty())
        .map(ToString::to_string)
}

fn work_manage_reference_text(arguments: &serde_json::Value) -> Option<String> {
    arguments
        .get("reference_text")
        .or_else(|| arguments.get("query"))
        .and_then(|value| value.as_str())
        .map(str::trim)
        .filter(|value| !value.is_empty())
        .map(ToString::to_string)
}

fn normalized_work_text(value: &str) -> String {
    let mut out = String::new();
    let mut last_was_space = true;
    for ch in value.chars() {
        if ch.is_alphanumeric() {
            for lower in ch.to_lowercase() {
                out.push(lower);
            }
            last_was_space = false;
        } else if !last_was_space {
            out.push(' ');
            last_was_space = true;
        }
    }
    out.trim().to_string()
}

fn work_text_matches_reference(haystack: &str, reference: &str) -> bool {
    let haystack = normalized_work_text(haystack);
    let reference = normalized_work_text(reference);
    reference.len() >= 3 && (haystack.contains(&reference) || reference.contains(&haystack))
}

fn scheduled_task_reference_blob(task: &crate::core::Task) -> String {
    let args = serde_json::to_string(&task.arguments).unwrap_or_default();
    format!(
        "{}\n{}\n{}\n{}",
        task.id, task.description, task.action, args
    )
}

fn watcher_reference_blob(watcher: &crate::core::watcher::Watcher) -> String {
    let poll_args = serde_json::to_string(&watcher.poll_arguments).unwrap_or_default();
    format!(
        "{}\n{}\n{}\n{}\n{}",
        watcher.id, watcher.description, watcher.poll_action, watcher.on_trigger, poll_args
    )
}

fn scheduled_task_status_label(status: &crate::core::TaskStatus) -> &'static str {
    match status {
        crate::core::TaskStatus::Pending => "pending",
        crate::core::TaskStatus::AwaitingApproval => "awaiting_approval",
        crate::core::TaskStatus::ExpiredNeedsReapproval => "expired_needs_reapproval",
        crate::core::TaskStatus::Paused => "paused",
        crate::core::TaskStatus::InProgress => "in_progress",
        crate::core::TaskStatus::Completed => "completed",
        crate::core::TaskStatus::Failed { .. } => "failed",
        crate::core::TaskStatus::Cancelled => "cancelled",
    }
}

fn scheduled_task_work_row(task: &crate::core::Task) -> serde_json::Value {
    let status_error = match &task.status {
        crate::core::TaskStatus::Failed { error } => Some(error.clone()),
        _ => None,
    };
    serde_json::json!({
        "id": task.id.to_string(),
        "description": task.description,
        "action": task.action,
        "arguments": task.arguments,
        "status": scheduled_task_status_label(&task.status),
        "status_error": status_error,
        "created_at": task.created_at.to_rfc3339(),
        "scheduled_for": task.scheduled_for.map(|value| value.to_rfc3339()),
        "cron": task.cron,
        "result": task.result,
    })
}

fn durable_work_not_found(kind: &str, operation: &str, id: &str) -> String {
    render_tool_completion_marker_with_data(
        "background_work_manage",
        "failed",
        &format!("{} `{}` was not found.", kind, id),
        serde_json::json!({
            "success": false,
            "kind": kind,
            "operation": operation,
            "id": id,
            "reason": "not_found",
        }),
    )
}
impl Agent {
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

    fn delegated_result_text_for_tool_output(
        result: &crate::core::task_router::DelegatedResult,
    ) -> String {
        let direct = result.final_response.content.trim();
        if !direct.is_empty() {
            return direct.to_string();
        }

        let completed = result
            .agent_results
            .iter()
            .filter(|item| {
                item.status == crate::core::DelegationStatus::Completed
                    && !item.content.trim().is_empty()
            })
            .take(8)
            .map(|item| {
                let name = item
                    .agent_name
                    .as_deref()
                    .unwrap_or(item.agent_type.as_str());
                format!(
                    "### {} / {}\n{}",
                    name,
                    item.agent_type,
                    safe_truncate(item.content.trim(), 2_000)
                )
            })
            .collect::<Vec<_>>();

        if !completed.is_empty() {
            return format!(
                "Delegated work completed, but no separate synthesis text was returned. Usable agent outputs:\n\n{}",
                completed.join("\n\n")
            );
        }

        let degraded = result
            .agent_results
            .iter()
            .take(8)
            .map(|item| {
                let name = item
                    .agent_name
                    .as_deref()
                    .unwrap_or(item.agent_type.as_str());
                format!(
                    "- {} / {} [{}]: {}",
                    name,
                    item.agent_type,
                    item.status.as_str(),
                    safe_truncate(item.content.trim(), 240)
                )
            })
            .collect::<Vec<_>>();

        if degraded.is_empty() {
            "Delegated work finished without a usable agent result.".to_string()
        } else {
            format!(
                "Delegated work finished without a usable synthesis. Agent statuses:\n{}",
                degraded.join("\n")
            )
        }
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
            .map(ToString::to_string)
            .or_else(|| {
                arguments
                    .get("tasks")
                    .and_then(|value| value.as_array())
                    .map(|items| {
                        items
                            .iter()
                            .filter_map(|item| item.as_str())
                            .map(str::trim)
                            .filter(|item| !item.is_empty())
                            .enumerate()
                            .map(|(index, item)| format!("{}. {item}", index + 1))
                            .collect::<Vec<_>>()
                            .join("\n")
                    })
                    .filter(|value| !value.trim().is_empty())
            })
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
        let mut delegated_message = task;
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
        let orchestration_required = arguments
            .get("orchestration_required")
            .and_then(|value| value.as_bool())
            .unwrap_or(false);

        let decision = crate::core::task_router::RoutingDecision {
            needs_delegation: true,
            complexity: QueryComplexity::Complex,
            sub_agents: self.forced_swarm_specs(&delegated_message, &actions),
            reasoning: if orchestration_required {
                "Delegation was explicitly requested by the tool arguments.".to_string()
            } else {
                "The spine selected the delegate primitive for this outcome.".to_string()
            },
            confidence: 0.96,
            should_clarify: false,
            clarification_question: None,
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
                let final_result = Self::delegated_result_text_for_tool_output(&result);
                serde_json::json!({
                    "ok": true,
                    "status": "completed",
                    "kind": "delegate",
                    "delegation_id": delegation_id,
                    "delegation_status": result.delegation_status.as_str(),
                    "agents_used": agents,
                    "degradation": degradation,
                    "final_result": crate::security::redact_pii(&final_result),
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
        "request_context",
        "acceptance_criteria",
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

        if let Ok(parsed) = reqwest::Url::parse(source_url) {
            if parsed.scheme() != "http" && parsed.scheme() != "https" {
                anyhow::bail!("Unsupported video URL format for delivery");
            }
            let client = crate::core::net::SafeHttpClient::new(90)?;
            let resp = client.public_get_follow_redirects(source_url, 5).await?;
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

        if let Err(error) =
            crate::actions::app::remove_app_scoped_env_secrets(&self.config_dir, &self.data_dir, app_id)
        {
            tracing::warn!(
                "failed to delete app-scoped env secrets during replacement for {}: {}",
                app_id,
                error
            );
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
            "suggested_actions": ["http_get", "app_restart", "file_read", "file_patch", "app_deploy(mode=patch)", "app_stop", "app_delete"],
            "repair_guidance": {
                "inspect_first": true,
                "prefer_existing_app_id": app_id,
                "minimal_operations": [
                    "Use app_restart when files and metadata are already correct but the runtime is stopped.",
                    "Use file_read/file_patch plus app_deploy mode=patch with app_id for localized source or metadata fixes.",
                    "Use full replacement only when the requested change is broad enough that patching would be less accurate."
                ],
                "avoid": [
                    "Do not create a duplicate app unless the user intentionally wants a separate deployment.",
                    "Do not regenerate unrelated files or reinstall dependencies when a targeted patch or restart is sufficient."
                ]
            },
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
        source_evidence: Option<&serde_json::Value>,
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
            .filter(|entry| Self::rendered_app_diagnostic_can_block(rendered, entry))
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
        let source_evidence = source_evidence.cloned().unwrap_or(serde_json::Value::Null);
        let prompt = serde_json::json!({
            "request_context": request_context,
            "rendered_page": {
                "title": &rendered.title,
                "url": &rendered.url,
                "body_text": safe_truncate(&rendered.body_text, 6000),
                "interactive_elements": elements,
                "browser_diagnostics": diagnostics,
            },
            "deployed_source_and_runtime": source_evidence,
            "judgement_rules": [
                "Judge semantically from the request and rendered-page evidence, not by exact wording alone.",
                "Pass only when the visible page and interactive elements provide concrete evidence that the requested product, content, workflow, controls, and fallback states are present.",
                "Use deployed source and runtime evidence for requested implementation requirements that cannot be proven from the DOM alone, such as persistence model, backend/runtime ownership, lifecycle, integrations, background work, and data flow.",
                "Fail when source/runtime evidence contradicts a requested capability or substitutes a weaker implementation for an explicit user preference.",
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
                "You validate a deployed app against the user's requested outcome using rendered DOM evidence, interactive element labels, browser diagnostics, and deployed source/runtime metadata. Be strict: successful deployment requires evidence that the requested app is actually present, usable, and implemented with any requested runtime or storage properties, not just that a nonblank page loaded. Return strict JSON only.",
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

    fn first_blocking_rendered_app_diagnostic(
        content: &crate::integrations::browser::PageContent,
    ) -> Option<String> {
        content.diagnostics.iter().find_map(|entry| {
            if !Self::rendered_app_diagnostic_can_block(content, entry) {
                return None;
            }
            let severity = entry.severity.trim();
            let kind = entry.kind.trim();
            let message = entry.message.trim();
            let is_blocking = severity.eq_ignore_ascii_case("error")
                || kind.eq_ignore_ascii_case("pageerror")
                || kind.eq_ignore_ascii_case("requestfailed");
            if !is_blocking {
                return None;
            }
            Some(safe_truncate(
                &format!(
                    "{}{}{}{}",
                    if kind.is_empty() { "browser" } else { kind },
                    if message.is_empty() { "" } else { ": " },
                    message,
                    if entry.url.trim().is_empty() {
                        String::new()
                    } else {
                        format!(" ({})", entry.url.trim())
                    }
                ),
                500,
            ))
        })
    }

    fn rendered_app_diagnostic_can_block(
        content: &crate::integrations::browser::PageContent,
        entry: &crate::integrations::browser::PageDiagnostic,
    ) -> bool {
        let severity = entry.severity.trim();
        let kind = entry.kind.trim();
        let is_error = severity.eq_ignore_ascii_case("error")
            || kind.eq_ignore_ascii_case("pageerror")
            || kind.eq_ignore_ascii_case("requestfailed");
        if !is_error {
            return false;
        }
        if kind.eq_ignore_ascii_case("pageerror") || entry.url.trim().is_empty() {
            return true;
        }

        let resource_type = entry.resource_type.trim().to_ascii_lowercase();
        let passive_resource = matches!(
            resource_type.as_str(),
            "font" | "image" | "media" | "prefetch" | "preflight" | "favicon"
        );
        if passive_resource && Self::diagnostic_url_is_outside_rendered_app_scope(content, entry) {
            return false;
        }
        true
    }

    fn diagnostic_url_is_outside_rendered_app_scope(
        content: &crate::integrations::browser::PageContent,
        entry: &crate::integrations::browser::PageDiagnostic,
    ) -> bool {
        let page_url = content.url.trim();
        let diagnostic_url = entry.url.trim();
        if page_url.is_empty() || diagnostic_url.is_empty() {
            return false;
        }
        let Ok(page) = reqwest::Url::parse(page_url) else {
            return false;
        };
        let diagnostic = match reqwest::Url::parse(diagnostic_url) {
            Ok(url) => url,
            Err(_) => match page.join(diagnostic_url) {
                Ok(url) => url,
                Err(_) => return false,
            },
        };
        if page.scheme() != diagnostic.scheme()
            || page.host_str() != diagnostic.host_str()
            || page.port_or_known_default() != diagnostic.port_or_known_default()
        {
            return false;
        }
        let page_path = page.path();
        let Some(app_scope) = Self::rendered_app_path_scope(page_path) else {
            return false;
        };
        !diagnostic.path().starts_with(&app_scope)
    }

    fn rendered_app_path_scope(page_path: &str) -> Option<String> {
        let mut parts = page_path.split('/').filter(|part| !part.is_empty());
        let first = parts.next()?;
        if !first.eq_ignore_ascii_case("apps") {
            return None;
        }
        let app_id = parts.next()?.trim();
        if app_id.is_empty() {
            return None;
        }
        Some(format!("/apps/{}/", app_id))
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

    fn app_deploy_quality_contract_from_inputs(
        arguments: &serde_json::Value,
        recent_messages: &[(&str, &str)],
    ) -> AppDeployQualityContract {
        let request_context = Self::app_deploy_request_context_from_arguments(arguments)
            .or_else(|| Self::latest_user_message_context(recent_messages))
            .unwrap_or_default();
        let mut sub_goals = Self::app_deploy_acceptance_criteria_from_arguments(arguments);
        if sub_goals.is_empty() && !request_context.is_empty() {
            sub_goals.push(request_context.clone());
        }
        AppDeployQualityContract {
            request_context,
            sub_goals,
        }
    }

    fn app_deploy_request_context_from_arguments(arguments: &serde_json::Value) -> Option<String> {
        arguments
            .get("request_context")
            .and_then(|value| value.as_str())
            .map(str::trim)
            .filter(|value| !value.is_empty())
            .map(|value| safe_truncate(value, 4000))
    }

    fn app_deploy_acceptance_criteria_from_arguments(arguments: &serde_json::Value) -> Vec<String> {
        arguments
            .get("acceptance_criteria")
            .and_then(|value| value.as_array())
            .map(|items| {
                items
                    .iter()
                    .filter_map(|item| item.as_str())
                    .map(str::trim)
                    .filter(|item| !item.is_empty())
                    .take(20)
                    .map(|item| safe_truncate(item, 300))
                    .collect::<Vec<_>>()
            })
            .unwrap_or_default()
    }

    fn latest_user_message_context(recent_messages: &[(&str, &str)]) -> Option<String> {
        recent_messages.iter().rev().find_map(|(role, content)| {
            let content = content.trim();
            if role.trim().eq_ignore_ascii_case("user") && !content.is_empty() {
                Some(safe_truncate(content, 4000))
            } else {
                None
            }
        })
    }

    fn app_quality_file_is_app_relative(path: &str) -> bool {
        let path = path.trim();
        !path.is_empty()
            && !path.starts_with('/')
            && !path.starts_with('\\')
            && !path
                .replace('\\', "/")
                .split('/')
                .any(|part| part.is_empty() || part == "." || part == "..")
    }

    async fn build_app_quality_source_evidence(
        &self,
        app_id: &str,
        app_type: &str,
    ) -> serde_json::Value {
        let Some(app_dir) = self.app_registry.get_dir(app_id).await else {
            return serde_json::json!({
                "available": false,
                "reason": "app directory was not available",
            });
        };
        let meta = tokio::fs::read(app_dir.join(".app_meta.json"))
            .await
            .ok()
            .and_then(|bytes| serde_json::from_slice::<serde_json::Value>(&bytes).ok())
            .filter(|value| value.is_object())
            .unwrap_or_else(|| serde_json::json!({}));
        let managed_files = meta
            .get("managed_files")
            .and_then(|value| value.as_array())
            .map(|items| {
                items
                    .iter()
                    .filter_map(|item| item.as_str())
                    .map(str::trim)
                    .filter(|path| Self::app_quality_file_is_app_relative(path))
                    .take(30)
                    .map(ToString::to_string)
                    .collect::<Vec<_>>()
            })
            .unwrap_or_default();
        let mut file_evidence = Vec::new();
        let mut remaining_excerpt_bytes = 20_000usize;
        for path in &managed_files {
            let file_path = app_dir.join(path);
            let Ok(bytes) = tokio::fs::read(&file_path).await else {
                file_evidence.push(serde_json::json!({
                    "path": path,
                    "readable": false,
                }));
                continue;
            };
            let size_bytes = bytes.len();
            let mut entry = serde_json::json!({
                "path": path,
                "readable": true,
                "size_bytes": size_bytes,
            });
            if remaining_excerpt_bytes > 0 {
                if let Ok(text) = String::from_utf8(bytes) {
                    let excerpt_limit = remaining_excerpt_bytes.min(2500);
                    let excerpt = safe_truncate(&text, excerpt_limit);
                    remaining_excerpt_bytes = remaining_excerpt_bytes.saturating_sub(excerpt.len());
                    if let Some(obj) = entry.as_object_mut() {
                        obj.insert("excerpt".to_string(), serde_json::json!(excerpt));
                    }
                }
            }
            file_evidence.push(entry);
        }

        serde_json::json!({
            "available": true,
            "app_id": app_id,
            "app_type": app_type,
            "metadata": {
                "runtime_required": meta.get("runtime_required").cloned().unwrap_or(serde_json::Value::Null),
                "runtime_preference": meta.get("runtime_preference").cloned().unwrap_or(serde_json::Value::Null),
                "runtime_reason": meta.get("runtime_reason").cloned().unwrap_or(serde_json::Value::Null),
                "entry_command": meta.get("entry_command").cloned().unwrap_or(serde_json::Value::Null),
                "start_command": meta.get("start_command").cloned().unwrap_or(serde_json::Value::Null),
                "install_command": meta.get("install_command").cloned().unwrap_or(serde_json::Value::Null),
                "repo_url": meta.get("repo_url").cloned().unwrap_or(serde_json::Value::Null),
                "repo_subdir": meta.get("repo_subdir").cloned().unwrap_or(serde_json::Value::Null),
                "required_inputs": meta.get("required_inputs").cloned().unwrap_or(serde_json::Value::Null),
                "runtime_actions": meta.get("runtime_actions").cloned().unwrap_or(serde_json::Value::Null),
            },
            "managed_file_count": managed_files.len(),
            "files": file_evidence,
        })
    }

    async fn app_deploy_quality_contract(
        &self,
        arguments: &serde_json::Value,
        conversation_id: Option<&str>,
    ) -> AppDeployQualityContract {
        if Self::app_deploy_request_context_from_arguments(arguments).is_some() {
            return Self::app_deploy_quality_contract_from_inputs(arguments, &[]);
        }
        let Some(conversation_id) = conversation_id else {
            return Self::app_deploy_quality_contract_from_inputs(arguments, &[]);
        };
        match self.storage.get_recent_messages(conversation_id, 24).await {
            Ok(messages) => {
                let pairs = messages
                    .iter()
                    .map(|message| (message.role.as_str(), message.content.as_str()))
                    .collect::<Vec<_>>();
                Self::app_deploy_quality_contract_from_inputs(arguments, &pairs)
            }
            Err(error) => {
                tracing::debug!(
                    conversation_id = %conversation_id,
                    error = %error,
                    "app_deploy_quality_context_recent_messages_failed"
                );
                Self::app_deploy_quality_contract_from_inputs(arguments, &[])
            }
        }
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

    async fn run_app_quality_check_for_deploy(
        &self,
        app_id: &str,
        app_url_with_key: &str,
        app_type: &str,
        quality_contract: &AppDeployQualityContract,
    ) -> Option<serde_json::Value> {
        let app_dir = self.app_registry.get_dir(app_id).await?;
        let app_updated_at = Self::app_meta_updated_at(&app_dir).await;
        if let Err(error) = self
            .persist_app_sub_goals(app_id, &quality_contract.sub_goals)
            .await
        {
            tracing::debug!(app_id = %app_id, error = %error, "app_sub_goals_persist_failed");
        }
        let report = self
            .build_app_quality_report(
                app_id,
                app_url_with_key,
                app_type,
                &quality_contract.request_context,
                &quality_contract.sub_goals,
                app_updated_at.as_deref(),
            )
            .await;
        if Self::app_meta_updated_at(&app_dir).await == app_updated_at {
            if let Err(error) = Self::write_app_json_file_atomic(
                &app_dir,
                crate::actions::app::APP_QUALITY_REPORT_FILE,
                &report,
            )
            .await
            {
                tracing::debug!(app_id = %app_id, error = %error, "app_quality_report_write_failed");
            }
        }
        Some(report)
    }

    fn app_quality_tool_summary(
        report: Option<&serde_json::Value>,
        quality_contract: &AppDeployQualityContract,
    ) -> serde_json::Value {
        let request_context_attached = !quality_contract.request_context.trim().is_empty();
        let acceptance_criteria_count = quality_contract.sub_goals.len();
        let Some(report) = report else {
            return serde_json::json!({
                "status": "unavailable",
                "request_context_attached": request_context_attached,
                "acceptance_criteria_count": acceptance_criteria_count,
                "needs_repair": false,
            });
        };
        let status = report
            .get("status")
            .and_then(|value| value.as_str())
            .unwrap_or("unknown");
        let browser_ok = report
            .get("browser_ok")
            .and_then(|value| value.as_bool())
            .unwrap_or(false);
        let judge_passed = report
            .pointer("/judge/passed")
            .and_then(|value| value.as_bool());
        let judge_available = report
            .pointer("/judge/available")
            .and_then(|value| value.as_bool())
            .unwrap_or(true);
        let effective_judge_passed = judge_available.then_some(judge_passed).flatten();
        let judge_summary = report
            .pointer("/judge/summary")
            .and_then(|value| value.as_str())
            .map(str::trim)
            .filter(|value| !value.is_empty())
            .map(|value| safe_truncate(value, 900));
        let concerns = report
            .get("judge_concerns")
            .and_then(|value| value.as_array())
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
        let needs_repair =
            status == "error" || (request_context_attached && effective_judge_passed == Some(false));
        serde_json::json!({
            "status": status,
            "request_context_attached": request_context_attached,
            "acceptance_criteria_count": acceptance_criteria_count,
            "browser_ok": browser_ok,
            "judge_available": judge_available,
            "judge_passed": effective_judge_passed,
            "judge_summary": judge_summary,
            "concerns": concerns,
            "needs_repair": needs_repair,
        })
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
        let source_evidence = self
            .build_app_quality_source_evidence(app_id, app_type)
            .await;

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
                    "title": title.clone(),
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
        let mut blocking_diagnostic: Option<String> = None;
        if let Some(content) = rendered_content.as_ref() {
            let combined = format!("{}\n{}", content.title, content.body_text).to_lowercase();
            runtime_marker = Self::detect_app_runtime_error_marker(&combined).map(str::to_string);
            blocking_diagnostic = Self::first_blocking_rendered_app_diagnostic(content);
            browser_ok = Self::rendered_app_content_has_dom_signal(content)
                && runtime_marker.is_none()
                && blocking_diagnostic.is_none();
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

        let (judge_available, judge_passed, judge_detail) = match rendered_content.as_ref() {
            Some(content) => match self
                .validate_rendered_app_against_request(
                    request_context,
                    content,
                    Some(&source_evidence),
                    None,
                )
                .await
            {
                Ok((passed, detail)) => (true, Some(passed), detail),
                Err(error) => (
                    false,
                    None,
                    format!("Advisory model quality check unavailable: {}", error),
                ),
            },
            None => (
                false,
                None,
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
        if let Some(issue) = blocking_diagnostic.as_deref() {
            concerns.push(format!("Browser diagnostic blocked validation: {}", issue));
        }
        if judge_passed == Some(false) {
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
            "source_summary": {
                "available": source_evidence.get("available").and_then(|value| value.as_bool()).unwrap_or(false),
                "managed_file_count": source_evidence.get("managed_file_count").and_then(|value| value.as_u64()).unwrap_or(0),
                "runtime_required": source_evidence.pointer("/metadata/runtime_required").cloned().unwrap_or(serde_json::Value::Null),
                "runtime_preference": source_evidence.pointer("/metadata/runtime_preference").cloned().unwrap_or(serde_json::Value::Null),
                "entry_command": source_evidence.pointer("/metadata/entry_command").cloned().unwrap_or(serde_json::Value::Null),
                "install_command": source_evidence.pointer("/metadata/install_command").cloned().unwrap_or(serde_json::Value::Null),
            },
            "judge": {
                "available": judge_available,
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

    async fn execute_direct_watcher_delete_tool(
        &self,
        arguments: &serde_json::Value,
    ) -> Result<String> {
        let watcher_id = arguments
            .get("watcher_id")
            .or_else(|| arguments.get("id"))
            .and_then(|value| value.as_str())
            .map(str::trim)
            .filter(|value| !value.is_empty())
            .ok_or_else(|| anyhow::anyhow!("Missing watcher_id"))?;
        let uuid = uuid::Uuid::parse_str(watcher_id)
            .with_context(|| format!("Invalid watcher_id `{watcher_id}`"))?;
        let deleted_live = self.watcher_manager.delete(uuid).await;
        let deleted_history = self.clear_watcher_supervisor_state(watcher_id).await;
        self.background_sessions
            .remove_child_references(&[], &[watcher_id.to_string()], Some("agent"))
            .await;
        let deleted_reflect_units = self
            .storage
            .delete_semantic_work_units_for_source("watcher", watcher_id)
            .await
            .unwrap_or(0);
        let deleted = deleted_live || deleted_history || deleted_reflect_units > 0;
        Ok(serde_json::to_string_pretty(&serde_json::json!({
            "watcher_id": watcher_id,
            "deleted": deleted,
            "deleted_live": deleted_live,
            "deleted_history": deleted_history,
            "deleted_reflect_units": deleted_reflect_units,
        }))?)
    }

    /// Execute a tool call from the turn loop with cross-layer approval preflight.
    ///
    /// Before running the action, this performs an authorization preflight via
    /// the runtime's `authorize_action_invocation`. If the cross-layer capability
    /// policy requires explicit user approval (and the auth context isn't
    /// already approved), this persists a pending approval row and returns a
    /// structured `approval_required` envelope as the tool result. The LLM
    /// reads the envelope and relays it to the user with the inline approval
    /// choices the frontend renders. When the user clicks Approve, the
    /// existing `approve_direct_chat_any_approval` flow re-runs the calls with
    /// `current_turn_is_explicit_approval=true`, which short-circuits this
    /// preflight path.
    ///
    /// If preflight approval is not required, this delegates to
    /// `execute_action_with_hooks` so the regular execution + error handling
    /// path applies. The result is always a tool-result envelope string the
    /// LLM can semantically reason about (no surface phrasing).
    pub(crate) async fn execute_tool_call_with_approval_preflight(
        &self,
        action_name: &str,
        arguments: &serde_json::Value,
        channel: &str,
        message_hint: Option<&str>,
        authorization: &crate::actions::ActionAuthorizationContext,
        conversation_id: Option<&str>,
        stream_tx: Option<&tokio::sync::mpsc::Sender<StreamEvent>>,
    ) -> String {
        // If this turn is already explicit-approval (re-run after Approve
        // click), skip the preflight and go straight to execution. The
        // runtime's own authorization layer applies as usual.
        if !authorization.current_turn_is_explicit_approval {
            let action_def = self.runtime.action_definition(action_name).await;
            let preflight = self
                .runtime
                .authorize_action_invocation(
                    action_name,
                    action_def.as_ref(),
                    arguments,
                    authorization,
                )
                .await;
            if let Ok(decision) = preflight {
                if !decision.allowed && decision.requires_explicit_approval {
                    let calls = vec![DirectChatChainApprovalCall {
                        action_name: action_name.to_string(),
                        arguments: arguments.clone(),
                    }];
                    match self
                        .build_approval_required_envelope(
                            conversation_id,
                            channel,
                            &calls,
                            authorization,
                            "cross_layer_capability_policy",
                            decision.reason.as_str(),
                        )
                        .await
                    {
                        Ok(envelope) => return envelope.to_string(),
                        Err(error) => {
                            tracing::warn!(
                                action = action_name,
                                channel,
                                error = %error,
                                "failed to persist approval request; surfacing typed error"
                            );
                            let action_err = crate::actions::ActionError::new(
                                crate::actions::ActionErrorDomain::Auth,
                                crate::actions::ActionErrorReason::ApprovalRequired,
                                decision.reason.as_str(),
                            );
                            return action_err.to_envelope(action_name).to_string();
                        }
                    }
                }
            }
        }
        match self
            .execute_action_with_hooks(
                action_name,
                arguments,
                channel,
                message_hint,
                Some(authorization),
                conversation_id,
                None,
                stream_tx,
            )
            .await
        {
            Ok(result_text) => tool_result_envelope_ok_string(action_name, &result_text),
            Err(error) => tool_result_envelope_error_string(action_name, &error),
        }
    }

    async fn handle_manage_actions_tool_call(
        &self,
        arguments: &serde_json::Value,
        authorization: Option<&crate::actions::ActionAuthorizationContext>,
    ) -> Result<String> {
        let resource = arguments
            .get("resource")
            .and_then(|value| value.as_str())
            .map(str::trim)
            .filter(|value| !value.is_empty())
            .map(str::to_ascii_lowercase)
            .unwrap_or_else(|| "skill".to_string());
        let result = if resource == "skill_marketplace" {
            self.handle_skill_marketplace_manage_action(arguments)
                .await?
        } else {
            self.handle_skill_manage_action(arguments, authorization)
                .await?
        };
        let status = result
            .get("status")
            .and_then(|value| value.as_str())
            .unwrap_or("ok")
            .to_string();
        let detail = result
            .get("message")
            .and_then(|value| value.as_str())
            .unwrap_or("Skills updated.")
            .to_string();
        Ok(render_tool_completion_marker_with_data(
            "manage_actions",
            &status,
            &detail,
            result,
        ))
    }

    async fn handle_skill_manage_action(
        &self,
        arguments: &serde_json::Value,
        authorization: Option<&crate::actions::ActionAuthorizationContext>,
    ) -> Result<serde_json::Value> {
        let operation = action_text_arg(arguments, "operation")
            .unwrap_or_else(|| "list".to_string())
            .to_ascii_lowercase();
        let name = action_text_arg(arguments, "name").or_else(|| action_text_arg(arguments, "id"));
        let surface = skills_surface_json();
        let result = match operation.as_str() {
            "create" | "update" | "import" | "install" => {
                let url = action_text_arg(arguments, "url")
                    .or_else(|| action_text_arg(arguments, "source_url"))
                    .or_else(|| action_text_arg(arguments, "install_url"));
                let content = action_text_arg(arguments, "content")
                    .or_else(|| action_text_arg(arguments, "markdown"));
                let enabled = arguments
                    .get("enabled")
                    .and_then(|value| value.as_bool())
                    .unwrap_or(true);
                let security_confirmed = arguments
                    .get("security_confirmed")
                    .and_then(|value| value.as_bool())
                    .unwrap_or(false);
                let allow_duplicate = arguments
                    .get("allow_duplicate")
                    .and_then(|value| value.as_bool())
                    .unwrap_or(false);
                let model = action_text_arg(arguments, "model");
                let preview = if let Some(url) = url.clone().filter(|_| content.is_none()) {
                    crate::channels::http::actions::import_action_from_url_shared(
                        self,
                        &url,
                        name.as_deref(),
                        false,
                        model.as_deref(),
                        true,
                        false,
                    )
                    .await
                    .map_err(|error| anyhow::anyhow!("{}", error))?
                } else {
                    let preview_content = content.clone().ok_or_else(|| {
                        anyhow::anyhow!(
                            "Skill {} requires complete SKILL.md content or a raw SKILL.md URL.",
                            operation
                        )
                    })?;
                    crate::channels::http::actions::import_action_from_content_with_agent(
                        self,
                        "chat://skills",
                        preview_content,
                        name.as_deref(),
                        false,
                        model.as_deref(),
                        true,
                        false,
                    )
                    .await
                    .map_err(|error| anyhow::anyhow!("{}", error))?
                };
                if let Some(gate) =
                    skill_chat_review_gate_result(&preview, &operation, security_confirmed)
                {
                    return Ok(gate);
                }
                let imported = if let Some(url) = url.filter(|_| content.is_none()) {
                    crate::channels::http::actions::import_action_from_url_shared(
                        self,
                        &url,
                        name.as_deref(),
                        allow_duplicate,
                        model.as_deref(),
                        false,
                        enabled,
                    )
                    .await
                    .map_err(|error| anyhow::anyhow!("{}", error))?
                } else {
                    let content = content.ok_or_else(|| {
                        anyhow::anyhow!(
                            "Skill {} requires complete SKILL.md content or a raw SKILL.md URL.",
                            operation
                        )
                    })?;
                    crate::channels::http::actions::import_action_from_content_with_agent(
                        self,
                        "chat://skills",
                        content,
                        name.as_deref(),
                        allow_duplicate,
                        model.as_deref(),
                        false,
                        enabled,
                    )
                    .await
                    .map_err(|error| anyhow::anyhow!("{}", error))?
                };
                self.refresh_action_catalog_index("skill_chat_manage").await;
                merge_objects_json(
                    imported,
                    serde_json::json!({
                        "resource": "skill",
                        "operation": operation,
                        "surface": surface,
                    }),
                )
            }
            "delete" => {
                let name =
                    name.ok_or_else(|| anyhow::anyhow!("Skill delete requires a skill name."))?;
                let deleted = self.runtime.delete_action(&name).await?;
                if deleted {
                    self.refresh_action_catalog_index("skill_chat_delete").await;
                    serde_json::json!({
                        "status": "ok",
                        "resource": "skill",
                        "operation": "delete",
                        "name": name,
                        "message": "Skill deleted. You can manage remaining skills in Skills.",
                        "surface": surface,
                    })
                } else {
                    anyhow::bail!("Skill '{}' cannot be deleted or was not found.", name);
                }
            }
            "enable" | "disable" => {
                let name = name.ok_or_else(|| {
                    anyhow::anyhow!("Skill enable/disable requires a skill name.")
                })?;
                let enabled = operation == "enable";
                let changed = self.runtime.set_action_enabled(&name, enabled).await?;
                if changed {
                    self.refresh_action_catalog_index("skill_chat_enabled_changed")
                        .await;
                    serde_json::json!({
                        "status": "ok",
                        "resource": "skill",
                        "operation": operation,
                        "name": name,
                        "enabled": enabled,
                        "message": if enabled { "Skill enabled." } else { "Skill disabled." },
                        "surface": surface,
                    })
                } else {
                    anyhow::bail!(
                        "Skill '{}' was not found or cannot be enabled/disabled.",
                        name
                    );
                }
            }
            "read" | "status" => {
                let name = name
                    .ok_or_else(|| anyhow::anyhow!("Skill read/status requires a skill name."))?;
                let Some((info, content)) = self.runtime.get_action_content(&name).await? else {
                    anyhow::bail!("Skill '{}' was not found.", name);
                };
                serde_json::json!({
                    "status": "ok",
                    "resource": "skill",
                    "operation": operation,
                    "name": info.name,
                    "description": info.description,
                    "enabled": self.runtime.is_action_enabled(&name).await,
                    "source": format!("{:?}", info.source).to_ascii_lowercase(),
                    "content": if operation == "read" { serde_json::Value::String(content) } else { serde_json::Value::Null },
                    "message": "Skill found. You can manage it in Skills.",
                    "surface": surface,
                })
            }
            "test" => {
                let name =
                    name.ok_or_else(|| anyhow::anyhow!("Skill test requires a skill name."))?;
                let test_args = arguments
                    .get("arguments")
                    .cloned()
                    .unwrap_or_else(|| serde_json::json!({}));
                let default_auth;
                let auth_context = if let Some(auth) = authorization {
                    auth
                } else {
                    default_auth = crate::actions::ActionAuthorizationContext::default();
                    &default_auth
                };
                let output = self
                    .runtime
                    .execute_action_with_context(&name, &test_args, auth_context)
                    .await?;
                serde_json::json!({
                    "status": "ok",
                    "resource": "skill",
                    "operation": "test",
                    "name": name,
                    "output": output,
                    "message": "Skill test finished. You can manage it in Skills.",
                    "surface": surface,
                })
            }
            "list" => {
                let actions = self.runtime.list_actions().await?;
                let mut skills = Vec::new();
                for action in actions {
                    let enabled = self.runtime.is_action_enabled(&action.name).await;
                    skills.push(serde_json::json!({
                        "name": action.name,
                        "description": action.description,
                        "version": action.version,
                        "source": format!("{:?}", action.source).to_ascii_lowercase(),
                        "enabled": enabled,
                    }));
                }
                serde_json::json!({
                    "status": "ok",
                    "resource": "skill",
                    "operation": "list",
                    "skills": skills,
                    "message": "Skills listed.",
                    "surface": surface,
                })
            }
            other => anyhow::bail!("Unsupported skill operation '{}'.", other),
        };
        Ok(result)
    }

    async fn browser_profile_from_manage_arguments(
        &self,
        arguments: &serde_json::Value,
    ) -> Result<Option<crate::core::BrowserProfileRecord>> {
        let selector = action_text_arg(arguments, "profile_id")
            .or_else(|| action_text_arg(arguments, "id"))
            .or_else(|| action_text_arg(arguments, "profile"))
            .or_else(|| action_text_arg(arguments, "query"))
            .or_else(|| action_text_arg(arguments, "selector"));
        let Some(selector) = selector else {
            return Ok(None);
        };
        let listed = crate::core::BrowserProfileControlPlane::list(&self.storage).await?;
        if let Some(profile) = listed
            .profiles
            .iter()
            .find(|profile| profile.id == selector)
        {
            return Ok(Some(profile.clone()));
        }
        match crate::core::BrowserProfileControlPlane::resolve(&self.storage, &selector).await? {
            crate::core::BrowserProfileResolveOutcome::Resolved { profile, .. } => {
                Ok(Some(profile))
            }
            _ => Ok(None),
        }
    }

    async fn handle_browser_profile_manage_tool_call(
        &self,
        arguments: &serde_json::Value,
    ) -> Result<serde_json::Value> {
        let operation = action_text_arg(arguments, "operation")
            .or_else(|| action_text_arg(arguments, "action"))
            .or_else(|| action_text_arg(arguments, "op"))
            .map(|value| value.to_ascii_lowercase())
            .unwrap_or_else(|| "list".to_string());

        match operation.as_str() {
            "list" | "status" => {
                let profiles = crate::core::BrowserProfileControlPlane::list(&self.storage).await?;
                let sessions = self.browser_sessions.list_session_views().await;
                Ok(serde_json::json!({
                    "status": "ok",
                    "resource": "browser_profile",
                    "operation": operation,
                    "profiles": profiles.profiles,
                    "summary": profiles.summary,
                    "sessions": sessions,
                }))
            }
            "read" | "resolve" => {
                let Some(profile) = self
                    .browser_profile_from_manage_arguments(arguments)
                    .await?
                else {
                    return Ok(serde_json::json!({
                        "status": "needs_input",
                        "resource": "browser_profile",
                        "operation": operation,
                        "reason": "browser_profile_selector_required",
                        "message": "Choose a saved browser profile id or selector.",
                    }));
                };
                let sessions = self
                    .browser_sessions
                    .list_session_views()
                    .await
                    .into_iter()
                    .filter(|session| session.profile_id.as_deref() == Some(profile.id.as_str()))
                    .collect::<Vec<_>>();
                Ok(serde_json::json!({
                    "status": "ok",
                    "resource": "browser_profile",
                    "operation": operation,
                    "profile": profile,
                    "sessions": sessions,
                }))
            }
            "create" | "update" => {
                let target_kind = browser_profile_target_kind_from_arguments(arguments)?;
                let managed = arguments
                    .get("managed")
                    .and_then(|value| value.as_bool())
                    .or_else(|| {
                        target_kind.map(|kind| {
                            matches!(kind, crate::core::BrowserProfileTargetKind::Sandbox)
                        })
                    });
                let browser = action_text_arg(arguments, "browser");
                let metadata = arguments.get("metadata").cloned().or_else(|| {
                    if browser.is_none() && managed.is_none() {
                        None
                    } else {
                        Some(serde_json::json!({
                            "browser": browser.clone().unwrap_or_else(|| "chrome".to_string()),
                            "managed": managed.unwrap_or(true),
                        }))
                    }
                });
                let profile = crate::core::BrowserProfileControlPlane::upsert(
                    &self.storage,
                    crate::core::BrowserProfileUpsert {
                        id: action_text_arg(arguments, "profile_id")
                            .or_else(|| action_text_arg(arguments, "id")),
                        name: action_text_arg(arguments, "name")
                            .or_else(|| action_text_arg(arguments, "profile_name")),
                        description: action_text_arg(arguments, "description"),
                        target_kind,
                        target_endpoint: action_text_arg(arguments, "target_endpoint"),
                        target_profile_path: action_text_arg(arguments, "target_profile_path"),
                        target_workspace: action_text_arg(arguments, "target_workspace"),
                        login_state: browser_profile_login_state_from_arguments(arguments)?,
                        login_checked_at: Some(chrono::Utc::now().to_rfc3339()),
                        tags: browser_profile_tags_from_arguments(arguments),
                        enabled: arguments.get("enabled").and_then(|value| value.as_bool()),
                        last_error: action_text_arg(arguments, "last_error"),
                        metadata,
                        ..Default::default()
                    },
                )
                .await?;
                Ok(serde_json::json!({
                    "status": "ok",
                    "resource": "browser_profile",
                    "operation": operation,
                    "profile": profile,
                }))
            }
            "delete" => {
                let Some(profile) = self
                    .browser_profile_from_manage_arguments(arguments)
                    .await?
                else {
                    return Ok(serde_json::json!({
                        "status": "needs_input",
                        "resource": "browser_profile",
                        "operation": operation,
                        "reason": "browser_profile_selector_required",
                        "message": "Choose a saved browser profile to delete.",
                    }));
                };
                let stopped = self
                    .browser_sessions
                    .stop_profile_sessions(&profile.id)
                    .await
                    .unwrap_or_default();
                let storage_deleted = self
                    .browser_sessions
                    .delete_profile_storage(&profile.id)
                    .await
                    .is_ok();
                let deleted =
                    crate::core::BrowserProfileControlPlane::delete(&self.storage, &profile.id)
                        .await?;
                Ok(serde_json::json!({
                    "status": "ok",
                    "resource": "browser_profile",
                    "operation": operation,
                    "profile_id": profile.id,
                    "deleted": deleted,
                    "closed_sessions": stopped.len(),
                    "storage_deleted": storage_deleted,
                }))
            }
            "launch" => {
                let Some(profile) = self
                    .browser_profile_from_manage_arguments(arguments)
                    .await?
                else {
                    return Ok(serde_json::json!({
                        "status": "needs_input",
                        "resource": "browser_profile",
                        "operation": operation,
                        "reason": "browser_profile_selector_required",
                        "message": "Choose a saved browser profile to launch.",
                    }));
                };
                let view = self
                    .browser_sessions
                    .start_profile_login_session(
                        crate::core::browser_session::BrowserSessionProfileContext::from_browser_profile(
                            &profile,
                        ),
                    )
                    .await?;
                Ok(serde_json::json!({
                    "status": "needs_input",
                    "resource": "browser_profile",
                    "operation": operation,
                    "profile": {
                        "id": profile.id,
                        "name": profile.name,
                    },
                    "session": view,
                    "message": "Profile browser launched. Complete login or profile repair in the live browser, then close and save it.",
                }))
            }
            "close" => {
                let Some(profile) = self
                    .browser_profile_from_manage_arguments(arguments)
                    .await?
                else {
                    return Ok(serde_json::json!({
                        "status": "needs_input",
                        "resource": "browser_profile",
                        "operation": operation,
                        "reason": "browser_profile_selector_required",
                        "message": "Choose a saved browser profile to close.",
                    }));
                };
                let stopped = self
                    .browser_sessions
                    .stop_profile_sessions(&profile.id)
                    .await?;
                let ended_at = chrono::Utc::now().to_rfc3339();
                for view in &stopped {
                    let _ = crate::core::BrowserProfileControlPlane::record_session(
                        &self.storage,
                        crate::core::BrowserProfileSessionRecord {
                            profile_id: profile.id.clone(),
                            session_id: Some(view.id.clone()),
                            started_at: view.created_at.clone(),
                            ended_at: Some(ended_at.clone()),
                            title: view.page_title.clone(),
                            url: view.page_url.clone(),
                            outcome: "saved".to_string(),
                            channel: Some("web".to_string()),
                            note: Some("Browser profile session closed and saved.".to_string()),
                            ..Default::default()
                        },
                    )
                    .await;
                }
                let profile = if stopped.is_empty() {
                    profile
                } else {
                    crate::core::BrowserProfileControlPlane::upsert(
                        &self.storage,
                        crate::core::BrowserProfileUpsert {
                            id: Some(profile.id.clone()),
                            login_state: Some(crate::core::BrowserLoginState::Unknown),
                            login_checked_at: Some(ended_at),
                            login_note: Some(
                                "Browser state was saved. Login success was not verified by AgentArk."
                                    .to_string(),
                            ),
                            last_error: Some(String::new()),
                            ..Default::default()
                        },
                    )
                    .await?
                };
                Ok(serde_json::json!({
                    "status": "ok",
                    "resource": "browser_profile",
                    "operation": operation,
                    "profile": profile,
                    "closed_sessions": stopped.len(),
                }))
            }
            _ => Ok(serde_json::json!({
                "status": "failed",
                "resource": "browser_profile",
                "operation": operation,
                "reason": "unsupported_browser_profile_operation",
                "supported_operations": [
                    "create", "read", "update", "delete", "list", "status", "launch", "close",
                    "resolve",
                ],
            })),
        }
    }

    async fn handle_skill_marketplace_manage_action(
        &self,
        arguments: &serde_json::Value,
    ) -> Result<serde_json::Value> {
        let operation = action_text_arg(arguments, "operation")
            .unwrap_or_else(|| "list".to_string())
            .to_ascii_lowercase();
        let storage = self.storage.clone();
        let surface = skills_surface_json();
        match operation.as_str() {
            "create" => {
                let url = action_text_arg(arguments, "url").ok_or_else(|| {
                    anyhow::anyhow!("Skill marketplace create requires a marketplace URL.")
                })?;
                let request = crate::core::skill_marketplaces::SkillMarketplaceUpsertRequest {
                    id: action_text_arg(arguments, "id"),
                    name: action_text_arg(arguments, "name"),
                    url,
                    enabled: arguments.get("enabled").and_then(|value| value.as_bool()),
                };
                let (status, marketplace) =
                    crate::core::skill_marketplaces::upsert_marketplace(&storage, None, request)
                        .await?;
                Ok(serde_json::json!({
                    "status": status,
                    "resource": "skill_marketplace",
                    "operation": "create",
                    "marketplace": marketplace,
                    "message": "Skill marketplace saved. You can manage it in Skills.",
                    "surface": surface,
                }))
            }
            "update" => {
                let target = marketplace_target(arguments)?;
                let marketplaces =
                    crate::core::skill_marketplaces::load_marketplaces(&storage).await?;
                let existing = resolve_marketplace(&marketplaces, &target)?;
                let existing_id = existing.id.clone();
                let request = crate::core::skill_marketplaces::SkillMarketplaceUpsertRequest {
                    id: Some(existing_id.clone()),
                    name: action_text_arg(arguments, "name")
                        .or_else(|| Some(existing.name.clone())),
                    url: action_text_arg(arguments, "url").unwrap_or_else(|| existing.url.clone()),
                    enabled: arguments
                        .get("enabled")
                        .and_then(|value| value.as_bool())
                        .or(Some(existing.enabled)),
                };
                let (status, marketplace) = crate::core::skill_marketplaces::upsert_marketplace(
                    &storage,
                    Some(&existing_id),
                    request,
                )
                .await?;
                Ok(serde_json::json!({
                    "status": status,
                    "resource": "skill_marketplace",
                    "operation": "update",
                    "marketplace": marketplace,
                    "message": "Skill marketplace updated. You can manage it in Skills.",
                    "surface": surface,
                }))
            }
            "enable" | "disable" => {
                let target = marketplace_target(arguments)?;
                let marketplaces =
                    crate::core::skill_marketplaces::load_marketplaces(&storage).await?;
                let existing = resolve_marketplace(&marketplaces, &target)?;
                let existing_id = existing.id.clone();
                let enabled = operation == "enable";
                let request = crate::core::skill_marketplaces::SkillMarketplaceUpsertRequest {
                    id: Some(existing_id.clone()),
                    name: Some(existing.name.clone()),
                    url: existing.url.clone(),
                    enabled: Some(enabled),
                };
                let (status, marketplace) = crate::core::skill_marketplaces::upsert_marketplace(
                    &storage,
                    Some(&existing_id),
                    request,
                )
                .await?;
                Ok(serde_json::json!({
                    "status": status,
                    "resource": "skill_marketplace",
                    "operation": operation,
                    "marketplace": marketplace,
                    "message": if enabled { "Skill marketplace enabled." } else { "Skill marketplace disabled." },
                    "surface": surface,
                }))
            }
            "delete" => {
                let target = marketplace_target(arguments)?;
                let marketplaces =
                    crate::core::skill_marketplaces::load_marketplaces(&storage).await?;
                let existing = resolve_marketplace(&marketplaces, &target)?;
                let existing_id = existing.id.clone();
                crate::core::skill_marketplaces::remove_marketplace(&storage, &existing_id).await?;
                Ok(serde_json::json!({
                    "status": "ok",
                    "resource": "skill_marketplace",
                    "operation": "delete",
                    "id": existing_id,
                    "message": "Skill marketplace deleted. You can manage remaining marketplaces in Skills.",
                    "surface": surface,
                }))
            }
            "refresh" => {
                let target = marketplace_target(arguments)?;
                let marketplaces =
                    crate::core::skill_marketplaces::load_marketplaces(&storage).await?;
                let existing = resolve_marketplace(&marketplaces, &target)?;
                let existing_id = existing.id.clone();
                let marketplace =
                    crate::core::skill_marketplaces::refresh_marketplace(&storage, &existing_id)
                        .await?;
                Ok(serde_json::json!({
                    "status": if marketplace.last_error.is_some() { "warning" } else { "ok" },
                    "resource": "skill_marketplace",
                    "operation": "refresh",
                    "marketplace": marketplace,
                    "message": "Skill marketplace refreshed. You can manage it in Skills.",
                    "surface": surface,
                }))
            }
            "read" | "status" => {
                let target = marketplace_target(arguments)?;
                let marketplaces =
                    crate::core::skill_marketplaces::load_marketplaces(&storage).await?;
                let marketplace = resolve_marketplace(&marketplaces, &target)?;
                Ok(serde_json::json!({
                    "status": "ok",
                    "resource": "skill_marketplace",
                    "operation": operation,
                    "marketplace": marketplace,
                    "message": "Skill marketplace found. You can manage it in Skills.",
                    "surface": surface,
                }))
            }
            "list" => {
                let marketplaces =
                    crate::core::skill_marketplaces::load_marketplaces(&storage).await?;
                let installers_count: usize = marketplaces
                    .iter()
                    .map(|marketplace| marketplace.installers.len())
                    .sum();
                Ok(serde_json::json!({
                    "status": "ok",
                    "resource": "skill_marketplace",
                    "operation": "list",
                    "marketplaces": marketplaces,
                    "installers_count": installers_count,
                    "message": "Skill marketplaces listed.",
                    "surface": surface,
                }))
            }
            other => anyhow::bail!("Unsupported skill marketplace operation '{}'.", other),
        }
    }

    fn browser_profile_selector_from_tool(arguments: &serde_json::Value) -> Option<String> {
        action_text_arg(arguments, "profile_id").or_else(|| action_text_arg(arguments, "profile"))
    }

    fn browser_profile_candidates_payload(
        candidates: &[crate::core::BrowserProfileResolveCandidate],
    ) -> serde_json::Value {
        serde_json::Value::Array(
            candidates
                .iter()
                .map(|candidate| {
                    serde_json::json!({
                        "id": candidate.profile.id.clone(),
                        "name": candidate.profile.name.clone(),
                        "score": candidate.score,
                        "login_state": candidate.profile.login_state,
                        "target_kind": candidate.profile.target_kind,
                        "tags": candidate.profile.tags.clone(),
                    })
                })
                .collect(),
        )
    }

    async fn resolve_browser_profile_for_tool(
        &self,
        selector: Option<String>,
        allow_single_enabled_default: bool,
    ) -> anyhow::Result<
        std::result::Result<Option<crate::core::BrowserProfileRecord>, serde_json::Value>,
    > {
        let Some(selector) = selector else {
            if !allow_single_enabled_default {
                return Ok(Ok(None));
            }
            let response = crate::core::BrowserProfileControlPlane::list(&self.storage).await?;
            let enabled = response
                .profiles
                .into_iter()
                .filter(|profile| profile.enabled)
                .collect::<Vec<_>>();
            return if enabled.len() == 1 {
                Ok(Ok(enabled.into_iter().next()))
            } else {
                Ok(Ok(None))
            };
        };
        match crate::core::BrowserProfileControlPlane::resolve(&self.storage, &selector).await? {
            crate::core::BrowserProfileResolveOutcome::Resolved { profile, .. } => {
                Ok(Ok(Some(profile)))
            }
            crate::core::BrowserProfileResolveOutcome::Ambiguous { candidates } => {
                Ok(Err(serde_json::json!({
                    "success": false,
                    "reason": "browser_profile_ambiguous",
                    "profile_selector": selector,
                    "candidates": Self::browser_profile_candidates_payload(&candidates),
                })))
            }
            crate::core::BrowserProfileResolveOutcome::NotFound { candidates } => {
                Ok(Err(serde_json::json!({
                    "success": false,
                    "reason": "browser_profile_not_found",
                    "profile_selector": selector,
                    "candidates": Self::browser_profile_candidates_payload(&candidates),
                })))
            }
        }
    }

    async fn wait_for_browser_auto_checkpoint(
        &self,
        session_id: &str,
    ) -> Option<crate::core::browser_session::BrowserSessionView> {
        let deadline = tokio::time::Instant::now() + tokio::time::Duration::from_secs(540);
        loop {
            let view = self.browser_sessions.describe_session(session_id).await?;
            if !matches!(
                browser_auto_checkpoint_kind(&view),
                BrowserAutoCheckpointKind::Running
            ) {
                return Some(view);
            }
            if tokio::time::Instant::now() >= deadline {
                return Some(view);
            }
            tokio::time::sleep(tokio::time::Duration::from_millis(750)).await;
        }
    }

    fn browser_auto_notification_fn(
        stream_tx: Option<&tokio::sync::mpsc::Sender<StreamEvent>>,
    ) -> std::sync::Arc<
        dyn Fn(crate::core::browser_session::BrowserSessionNotification) + Send + Sync,
    > {
        let stream_target = stream_tx.cloned();
        std::sync::Arc::new(move |notification| {
            let Some(tx) = stream_target.as_ref() else {
                return;
            };
            let kind = match notification.kind {
                crate::core::browser_session::BrowserSessionNotificationKind::Progress => {
                    "progress"
                }
                crate::core::browser_session::BrowserSessionNotificationKind::NeedsInput => {
                    "needs_input"
                }
                crate::core::browser_session::BrowserSessionNotificationKind::Notice => "notice",
                crate::core::browser_session::BrowserSessionNotificationKind::Completed => {
                    "completed"
                }
                crate::core::browser_session::BrowserSessionNotificationKind::Failed => "failed",
                crate::core::browser_session::BrowserSessionNotificationKind::Closed => "closed",
            };
            queue_stream_event(
                tx,
                StreamEvent::ToolProgress {
                    name: "browser_auto".to_string(),
                    content: notification.message.clone(),
                    payload: Some(serde_json::json!({
                        "session_id": notification.session_id,
                        "kind": kind,
                        "has_screenshot": notification.screenshot.is_some(),
                    })),
                },
            );
        })
    }

    async fn resolve_browser_auto_session_id(
        &self,
        arguments: &serde_json::Value,
        conversation_id: Option<&str>,
    ) -> Option<String> {
        if let Some(session_id) = action_text_arg(arguments, "session_id") {
            return Some(session_id);
        }
        self.browser_sessions
            .latest_live_session_view_for_conversation(conversation_id)
            .await
            .map(|view| view.id)
    }

    async fn handle_browser_auto_snapshot_tool_call(
        &self,
        arguments: &serde_json::Value,
        conversation_id: Option<&str>,
    ) -> Result<String> {
        let Some(session_id) = self
            .resolve_browser_auto_session_id(arguments, conversation_id)
            .await
        else {
            return Ok(render_tool_completion_marker_with_data(
                "browser_auto",
                "needs_input",
                "No live browser session is available for this conversation.",
                serde_json::json!({
                    "success": false,
                    "reason": "browser_session_required",
                }),
            ));
        };
        let element_limit = arguments
            .get("element_limit")
            .and_then(|value| value.as_u64())
            .unwrap_or(50)
            .min(50) as usize;
        match self
            .browser_sessions
            .read_session_content(&session_id)
            .await
        {
            Ok((view, content)) => {
                let elements = content
                    .elements
                    .iter()
                    .take(element_limit)
                    .cloned()
                    .collect::<Vec<_>>();
                Ok(render_tool_completion_marker_with_data(
                    "browser_auto",
                    "completed",
                    &format!("Read current browser page: {}", content.title.trim()),
                    serde_json::json!({
                        "success": true,
                        "session": view,
                        "page": {
                            "title": content.title,
                            "url": content.url,
                            "body_text": safe_truncate(&content.body_text, 6_000),
                            "elements": elements,
                            "diagnostics": content.diagnostics,
                        }
                    }),
                ))
            }
            Err(error) => Ok(render_tool_completion_marker_with_data(
                "browser_auto",
                "failed",
                &format!("Could not read the live browser session: {}", error),
                serde_json::json!({
                    "success": false,
                    "session_id": session_id,
                    "reason": "browser_snapshot_failed",
                    "error": error.to_string(),
                }),
            )),
        }
    }

    async fn handle_browser_auto_resume_handoff_tool_call(
        &self,
        arguments: &serde_json::Value,
        channel: &str,
        conversation_id: Option<&str>,
        stream_tx: Option<&tokio::sync::mpsc::Sender<StreamEvent>>,
    ) -> Result<String> {
        let Some(session_id) = self
            .resolve_browser_auto_session_id(arguments, conversation_id)
            .await
        else {
            return Ok(render_tool_completion_marker_with_data(
                "browser_auto",
                "needs_input",
                "No live browser session is available for this conversation.",
                serde_json::json!({
                    "success": false,
                    "reason": "browser_session_required",
                }),
            ));
        };
        let note = action_text_arg(arguments, "note")
            .or_else(|| action_text_arg(arguments, "response"))
            .or_else(|| action_text_arg(arguments, "task"))
            .unwrap_or_default();
        let resume_in_chat = arguments
            .get("resume_in_chat")
            .and_then(|value| value.as_bool())
            .unwrap_or(false);
        let notify_fn = Self::browser_auto_notification_fn(stream_tx);
        match self
            .browser_sessions
            .resume_chat_input_session(
                &session_id,
                &note,
                channel,
                self.llm.clone(),
                notify_fn.clone(),
            )
            .await
        {
            Ok(Some(started)) => {
                if let Some(checkpoint) = self
                    .wait_for_browser_auto_checkpoint(&started.session_id)
                    .await
                {
                    return Ok(browser_auto_checkpoint_marker(checkpoint));
                }
                return Ok(render_tool_completion_marker_with_data(
                    "browser_auto",
                    "running",
                    "Browser chat input was applied. The browser task is still running.",
                    serde_json::json!({
                        "success": true,
                        "session_id": started.session_id,
                    }),
                ));
            }
            Ok(None) => {}
            Err(error) => {
                return Ok(render_tool_completion_marker_with_data(
                    "browser_auto",
                    "failed",
                    &format!("Could not resume the browser chat input: {}", error),
                    serde_json::json!({
                        "success": false,
                        "session_id": session_id,
                        "reason": "browser_chat_input_resume_failed",
                        "error": error.to_string(),
                    }),
                ));
            }
        }
        let view = match self
            .browser_sessions
            .complete_operator_handoff(&session_id, &note, resume_in_chat)
            .await
        {
            Ok(view) => view,
            Err(error) => {
                return Ok(render_tool_completion_marker_with_data(
                    "browser_auto",
                    "failed",
                    &format!("Could not resume the browser handoff: {}", error),
                    serde_json::json!({
                        "success": false,
                        "session_id": session_id,
                        "reason": "browser_handoff_resume_failed",
                        "error": error.to_string(),
                    }),
                ));
            }
        };
        if resume_in_chat {
            return Ok(render_tool_completion_marker_with_data(
                "browser_auto",
                "completed",
                view.summary
                    .as_deref()
                    .unwrap_or("Browser handoff returned control to chat."),
                serde_json::json!({
                    "success": true,
                    "session": view,
                }),
            ));
        }

        if let Some(checkpoint) = self.wait_for_browser_auto_checkpoint(&session_id).await {
            return Ok(browser_auto_checkpoint_marker(checkpoint));
        }
        Ok(render_tool_completion_marker_with_data(
            "browser_auto",
            "running",
            "Browser handoff was resumed. The browser task is still running.",
            serde_json::json!({
                "success": true,
                "session_id": session_id,
            }),
        ))
    }

    async fn handle_browser_auto_tool_call(
        &self,
        arguments: &serde_json::Value,
        channel: &str,
        conversation_id: Option<&str>,
        _message_hint: Option<&str>,
        stream_tx: Option<&tokio::sync::mpsc::Sender<StreamEvent>>,
    ) -> Result<String> {
        let action =
            action_text_arg(arguments, "action").unwrap_or_else(|| "start_session".to_string());
        match action.as_str() {
            "snapshot" => {
                return self
                    .handle_browser_auto_snapshot_tool_call(arguments, conversation_id)
                    .await;
            }
            "resume_handoff" => {
                return self
                    .handle_browser_auto_resume_handoff_tool_call(
                        arguments,
                        channel,
                        conversation_id,
                        stream_tx,
                    )
                    .await;
            }
            "start_session" => {}
            _ => {
                return Ok(render_tool_completion_marker_with_data(
                    "browser_auto",
                    "failed",
                    "browser_auto supports start_session, snapshot, and resume_handoff.",
                    serde_json::json!({
                        "success": false,
                        "reason": "unsupported_browser_auto_action",
                        "action": action,
                    }),
                ));
            }
        }

        let explicit_url = action_text_arg(arguments, "url");
        let recent_artifact = if explicit_url.is_none() {
            match conversation_id {
                Some(conversation_id) => self
                    .load_recent_artifact_contexts(conversation_id)
                    .await
                    .into_iter()
                    .find(|artifact| !artifact.url.trim().is_empty()),
                None => None,
            }
        } else {
            None
        };
        let task = browser_auto_task_description(
            explicit_url.as_deref(),
            recent_artifact.as_ref(),
            action_text_arg(arguments, "task"),
            action_text_arg(arguments, "expectation"),
        );
        if task.is_empty() {
            return Ok(render_tool_completion_marker_with_data(
                "browser_auto",
                "needs_input",
                "A browser task description or URL is required.",
                serde_json::json!({
                    "success": false,
                    "reason": "browser_task_required",
                }),
            ));
        }

        let profile = match self
            .resolve_browser_profile_for_tool(
                Self::browser_profile_selector_from_tool(arguments),
                true,
            )
            .await?
        {
            Ok(profile) => profile,
            Err(payload) => {
                let reason = payload
                    .get("reason")
                    .and_then(|value| value.as_str())
                    .unwrap_or("browser_profile_unresolved");
                let message = if reason == "browser_profile_ambiguous" {
                    "More than one saved browser profile matches the requested profile. Choose one profile id or name."
                } else {
                    "No saved browser profile matched the requested profile."
                };
                return Ok(render_tool_completion_marker_with_data(
                    "browser_auto",
                    "needs_input",
                    message,
                    payload,
                ));
            }
        };
        let profile_json = profile.as_ref().map(|profile| {
            serde_json::json!({
                "id": profile.id.clone(),
                "name": profile.name.clone(),
            })
        });
        let profile_context = profile
            .as_ref()
            .map(crate::core::browser_session::BrowserSessionProfileContext::from_browser_profile);
        if let (Some(profile), Some(profile_context)) = (profile.as_ref(), profile_context.clone())
        {
            if matches!(
                profile.login_state,
                crate::core::BrowserLoginState::LoggedOut
                    | crate::core::BrowserLoginState::NeedsMfa
                    | crate::core::BrowserLoginState::Expired
                    | crate::core::BrowserLoginState::Error
            ) {
                match self
                    .browser_sessions
                    .start_profile_login_session(profile_context)
                    .await
                {
                    Ok(view) => {
                        return Ok(render_tool_completion_marker_with_data(
                            "browser_auto",
                            "needs_input",
                            "The saved browser profile needs attention before AgentArk can reuse it. Sign in or repair it in the live browser, then close and save the profile before retrying the task.",
                            serde_json::json!({
                                "success": false,
                                "reason": "browser_profile_login_required",
                                "profile": profile_json,
                                "session": view,
                            }),
                        ));
                    }
                    Err(error) => {
                        let _ = crate::core::BrowserProfileControlPlane::upsert(
                            &self.storage,
                            crate::core::BrowserProfileUpsert {
                                id: Some(profile.id.clone()),
                                login_state: Some(crate::core::BrowserLoginState::Error),
                                login_checked_at: Some(chrono::Utc::now().to_rfc3339()),
                                last_error: Some(error.to_string()),
                                ..Default::default()
                            },
                        )
                        .await;
                        return Ok(render_tool_completion_marker_with_data(
                            "browser_auto",
                            "needs_input",
                            &format!(
                                "The saved browser profile needs attention, but AgentArk could not launch the profile repair browser: {}",
                                error
                            ),
                            serde_json::json!({
                                "success": false,
                                "reason": "browser_profile_repair_launch_failed",
                                "profile": profile_json,
                                "error": error.to_string(),
                            }),
                        ));
                    }
                }
            }
        }
        let notify_fn = Self::browser_auto_notification_fn(stream_tx);
        let started = match self
            .browser_sessions
            .start_session_with_profile(
                &task,
                channel,
                conversation_id,
                profile_context,
                self.llm.clone(),
                notify_fn,
            )
            .await
        {
            Ok(started) => started,
            Err(error) => {
                if let Some(profile) = profile.as_ref() {
                    let _ = crate::core::BrowserProfileControlPlane::upsert(
                        &self.storage,
                        crate::core::BrowserProfileUpsert {
                            id: Some(profile.id.clone()),
                            login_state: Some(crate::core::BrowserLoginState::Error),
                            login_checked_at: Some(chrono::Utc::now().to_rfc3339()),
                            last_error: Some(error.to_string()),
                            ..Default::default()
                        },
                    )
                    .await;
                    return Ok(render_tool_completion_marker_with_data(
                        "browser_auto",
                        "needs_input",
                        &format!(
                            "The saved browser profile could not be used for this browser task: {}",
                            error
                        ),
                        serde_json::json!({
                            "success": false,
                            "reason": "browser_profile_launch_failed",
                            "profile": profile_json,
                            "error": error.to_string(),
                        }),
                    ));
                }
                return Ok(render_tool_completion_marker_with_data(
                    "browser_auto",
                    "needs_input",
                    &format!("The browser task could not be started: {}", error),
                    serde_json::json!({
                        "success": false,
                        "reason": "browser_session_launch_failed",
                        "error": error.to_string(),
                    }),
                ));
            }
        };
        if let Some(profile) = profile.as_ref() {
            let _ = crate::core::BrowserProfileControlPlane::record_session(
                &self.storage,
                crate::core::BrowserProfileSessionRecord {
                    profile_id: profile.id.clone(),
                    session_id: Some(started.session_id.clone()),
                    started_at: chrono::Utc::now().to_rfc3339(),
                    outcome: "started".to_string(),
                    channel: Some(channel.to_string()),
                    note: Some(task.clone()),
                    url: explicit_url
                        .clone()
                        .or_else(|| recent_artifact.as_ref().map(|artifact| artifact.url.clone())),
                    ..Default::default()
                },
            )
            .await;
        }
        let checkpoint = self
            .wait_for_browser_auto_checkpoint(&started.session_id)
            .await;
        if let Some(view) = checkpoint {
            let marker_kind = browser_auto_checkpoint_kind(&view);
            let detail = browser_auto_checkpoint_detail(&view, marker_kind);
            let current_page = match self
                .browser_sessions
                .read_session_content(&started.session_id)
                .await
            {
                Ok((_, content)) => Some(browser_auto_page_snapshot_payload(&content)),
                Err(_) => view
                    .last_page
                    .as_ref()
                    .map(browser_auto_page_snapshot_payload),
            };
            let data = serde_json::json!({
                "success": marker_kind.success(),
                "session_id": started.session_id,
                "reused_existing": started.reused_existing,
                "task": task,
                "profile": profile_json,
                "session": view,
                "current_page": current_page,
            });
            return Ok(render_tool_completion_marker_with_data(
                "browser_auto",
                marker_kind.marker_status(),
                &detail,
                data,
            ));
        }
        Ok(render_tool_completion_marker_with_data(
            "browser_auto",
            "running",
            &format!(
                "Started managed browser session {}. The requested browser outcome is not complete yet.",
                started.session_id
            ),
            serde_json::json!({
                "success": true,
                "session_id": started.session_id,
                "reused_existing": started.reused_existing,
                "task": task,
                "profile": profile_json,
            }),
        ))
    }

    pub(crate) async fn execute_action_with_hooks(
        &self,
        action_name: &str,
        arguments: &serde_json::Value,
        channel: &str,
        message_hint: Option<&str>,
        authorization: Option<&crate::actions::ActionAuthorizationContext>,
        conversation_id: Option<&str>,
        project_id: Option<&str>,
        stream_tx: Option<&tokio::sync::mpsc::Sender<StreamEvent>>,
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

        let action_catalog_before = self.runtime_action_catalog_fingerprint().await;
        let execution = if action_name.eq_ignore_ascii_case("notify_user")
            && notification_tool_should_dispatch_for_surface(channel, authorization)
        {
            self.execute_direct_notify_user_tool(arguments).await
        } else if action_name.eq_ignore_ascii_case("list_watchers") {
            self.execute_direct_list_watchers_tool(arguments).await
        } else if action_name.eq_ignore_ascii_case("watcher_delete") {
            self.execute_direct_watcher_delete_tool(arguments).await
        } else if action_name.eq_ignore_ascii_case("schedule_task") {
            Ok(durable_orchestration_action_result(
                "schedule_task",
                "Scheduled task",
                self.handle_schedule_task(
                    arguments,
                    channel,
                    conversation_id,
                    project_id,
                    authorization,
                )
                .await,
                |value| crate::runtime::parse_schedule_task_completion(value).is_some(),
            ))
        } else if action_name.eq_ignore_ascii_case("watch") {
            Ok(durable_orchestration_action_result(
                "watch",
                "Watcher",
                self.handle_watch(
                    arguments,
                    channel,
                    conversation_id,
                    project_id,
                    authorization,
                )
                .await,
                |value| crate::runtime::parse_watch_completion(value).is_some(),
            ))
        } else if action_name.eq_ignore_ascii_case("background_session_manage") {
            Ok(self
                .handle_background_session_manage(arguments, conversation_id)
                .await
                .unwrap_or_else(|| {
                    render_tool_completion_marker_with_data(
                        "background_session_manage",
                        "failed",
                        "AgentArk could not resolve that background session operation.",
                        serde_json::json!({
                            "success": false,
                            "reason": "missing_background_session_manage_result",
                        }),
                    )
                }))
        } else if action_name.eq_ignore_ascii_case("work_manage")
            || action_name.eq_ignore_ascii_case("background_work_manage")
        {
            Ok(self
                .handle_work_manage_tool_call(arguments, conversation_id)
                .await)
        } else if action_name.eq_ignore_ascii_case("app_deploy") {
            self.handle_app_deploy_tool_call(arguments, stream_tx, channel, conversation_id)
                .await
        } else if action_name.eq_ignore_ascii_case("service_manage") {
            self.handle_service_manage_tool_call(arguments, stream_tx, channel, conversation_id)
                .await
        } else if action_name.eq_ignore_ascii_case("browser_profile_manage") {
            match self
                .handle_browser_profile_manage_tool_call(arguments)
                .await
            {
                Ok(value) => Ok(serde_json::to_string_pretty(&value)?),
                Err(error) => Err(error),
            }
        } else if action_name.eq_ignore_ascii_case("browser_auto") {
            self.handle_browser_auto_tool_call(
                arguments,
                channel,
                conversation_id,
                message_hint,
                stream_tx,
            )
            .await
        } else if action_name.eq_ignore_ascii_case("delegate") {
            let trace_ref = Arc::new(RwLock::new(ExecutionTrace {
                id: uuid::Uuid::new_v4().to_string(),
                message: message_hint.unwrap_or_default().to_string(),
                channel: channel.to_string(),
                started_at: Some(chrono::Utc::now()),
                ..ExecutionTrace::default()
            }));
            let result = self
                .handle_delegate_tool_call(arguments, channel, &trace_ref, stream_tx)
                .await;
            if let Ok(payload) = serde_json::from_str::<serde_json::Value>(&result) {
                if payload
                    .get("ok")
                    .and_then(|value| value.as_bool())
                    .is_some_and(|ok| !ok)
                {
                    let detail = payload
                        .get("error")
                        .and_then(|value| value.as_str())
                        .unwrap_or("Delegation failed.");
                    return Err(crate::actions::structured_action_error_for_action(
                        action_name,
                        crate::actions::ActionErrorReason::Failed,
                        detail,
                    ));
                }
            }
            Ok(result)
        } else if action_name.eq_ignore_ascii_case("manage_actions") {
            self.handle_manage_actions_tool_call(arguments, authorization)
                .await
        } else if let Some(auth_context) = authorization {
            self.runtime
                .execute_action_payload_with_context(action_name, arguments, auth_context)
                .await
                .map(|payload| {
                    crate::runtime::ActionRuntime::render_tool_payload_for_legacy(
                        action_name,
                        payload,
                    )
                })
        } else {
            self.runtime
                .execute_action_payload(action_name, arguments)
                .await
                .map(|payload| {
                    crate::runtime::ActionRuntime::render_tool_payload_for_legacy(
                        action_name,
                        payload,
                    )
                })
        };

        match execution {
            Ok(result) => {
                let result = crate::runtime::ActionRuntime::render_tool_payload_for_legacy(
                    action_name,
                    crate::runtime::ActionRuntime::tool_payload_from_legacy_output(
                        action_name,
                        result,
                    ),
                );
                self.fire_action_hook(
                    crate::hooks::HookTrigger::PostAction,
                    channel,
                    action_name,
                    message_hint,
                    Some(&result),
                    &event_id,
                )
                .await;
                self.refresh_action_catalog_after_runtime_change(action_catalog_before)
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

    async fn runtime_action_catalog_fingerprint(&self) -> Option<String> {
        let mut actions = self.runtime.list_actions().await.ok()?;
        actions.sort_by(|left, right| left.name.cmp(&right.name));
        Some(
            actions
                .into_iter()
                .map(|action| {
                    let mut capabilities = action.capabilities;
                    capabilities.sort();
                    format!(
                        "{}\u{1f}{}\u{1f}{:?}\u{1f}{}",
                        action.name,
                        action.version,
                        action.source,
                        capabilities.join("\u{1e}")
                    )
                })
                .collect::<Vec<_>>()
                .join("\u{1d}"),
        )
    }

    async fn refresh_action_catalog_after_runtime_change(&self, before: Option<String>) {
        let Some(before) = before else {
            return;
        };
        let Some(after) = self.runtime_action_catalog_fingerprint().await else {
            return;
        };
        if before != after {
            self.refresh_action_catalog_index("runtime_action_catalog_changed")
                .await;
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
                    tracing::debug!("Pulse refresh client init failed: {}", e);
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
                        "Pulse refresh trigger after {} returned {}",
                        reason,
                        resp.status()
                    );
                }
                Err(e) => {
                    tracing::debug!("Pulse refresh trigger after {} failed: {}", reason, e);
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
        let build_command = crate::actions::app::app_meta_lifecycle_command(&meta, "build_command");
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
                    Some(app_id),
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
                    "title": title.clone(),
                    "url": relative_url,
                    "local_url": local_url,
                    "missing_env": missing_sensitive,
                    "missing_config": missing_config,
                    "missing_inputs": missing_all,
                    "required_inputs": required_inputs,
                    "required_secrets": required_secret_keys.clone(),
                    "required_env": required_secret_keys.clone(),
                    "required_config": required_config_keys,
                    "apps_page_hint": crate::actions::app::APP_DEPLOY_CONTROL_HINT,
                    "credential_request": {
                        "kind": "app_runtime_secret",
                        "app_id": app_id,
                        "display_name": title,
                        "required_keys": required_secret_keys,
                        "settings_path": format!("/apps/{}/environment", app_id),
                        "secure_input_required": true
                    },
                    "message": "Missing required inputs. Use the secure credential form in chat or Settings for sensitive values; provide config for non-sensitive values."
                }));
            }

            let runtime_handle = crate::actions::app::launch_dynamic_runtime(
                crate::actions::app::DynamicRuntimeLaunch {
                    app_id,
                    app_dir: &app_dir,
                    entry_command: &entry_command,
                    install_command: install_command.as_deref(),
                    build_command: build_command.as_deref(),
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

    fn normalize_service_manage_deploy_arguments(
        arguments: &serde_json::Value,
    ) -> serde_json::Value {
        let mut deploy = arguments.as_object().cloned().unwrap_or_default();
        let operation = deploy
            .get("operation")
            .and_then(|value| value.as_str())
            .unwrap_or("create")
            .trim()
            .to_ascii_lowercase();
        let kind = deploy
            .get("kind")
            .and_then(|value| value.as_str())
            .unwrap_or("auto")
            .trim()
            .to_ascii_lowercase();

        deploy.remove("operation");
        deploy.remove("kind");
        deploy.remove("query");

        if let Some(service_id) = deploy.remove("service_id") {
            if !deploy.contains_key("app_id") {
                deploy.insert("app_id".to_string(), service_id);
            }
        }
        if let Some(name) = deploy.remove("name") {
            if !deploy.contains_key("title") {
                deploy.insert("title".to_string(), name);
            }
        }
        if let Some(public) = deploy.remove("public") {
            if !deploy.contains_key("expose_public") {
                deploy.insert("expose_public".to_string(), public);
            }
        }
        if let Some(environment) = deploy.remove("environment") {
            if !deploy.contains_key("config") {
                deploy.insert("config".to_string(), environment);
            }
        }
        if let Some(start_command) = deploy.get("start_command").cloned() {
            if !deploy.contains_key("entry_command") {
                deploy.insert("entry_command".to_string(), start_command);
            }
        }

        if operation == "patch" && !deploy.contains_key("mode") {
            deploy.insert("mode".to_string(), serde_json::json!("patch"));
        }

        match kind.as_str() {
            "static" => {
                if !deploy.contains_key("entry_command")
                    && !deploy.contains_key("start_command")
                    && !deploy.contains_key("repo_url")
                    && !deploy.contains_key("runtime_required")
                {
                    deploy.insert("runtime_required".to_string(), serde_json::json!(false));
                }
            }
            "process" => {
                if !deploy.contains_key("runtime_required") {
                    deploy.insert("runtime_required".to_string(), serde_json::json!(true));
                }
            }
            "repo" => {
                deploy
                    .entry("service_mode".to_string())
                    .or_insert_with(|| serde_json::json!("auto"));
            }
            _ => {}
        }

        serde_json::Value::Object(deploy)
    }

    fn service_manage_allows_duplicate(arguments: &serde_json::Value) -> bool {
        let duplicate_policy = arguments
            .get("duplicate_policy")
            .and_then(|value| value.as_str())
            .map(str::trim)
            .unwrap_or("");
        arguments
            .get("allow_duplicate")
            .and_then(|value| value.as_bool())
            .unwrap_or(false)
            || duplicate_policy.eq_ignore_ascii_case("create_new")
    }

    fn deploy_create_should_reuse_recent_app(
        operation: &str,
        arguments: &serde_json::Value,
        has_recent_app: bool,
    ) -> bool {
        let has_explicit_target = arguments
            .get("service_id")
            .or_else(|| arguments.get("app_id"))
            .and_then(|value| value.as_str())
            .map(str::trim)
            .is_some_and(|value| !value.is_empty());
        operation.trim().eq_ignore_ascii_case("create")
            && !has_explicit_target
            && has_recent_app
            && !Self::service_manage_allows_duplicate(arguments)
    }

    fn app_exists_in_registry(app_id: &str, apps: &[serde_json::Value]) -> bool {
        let app_id = app_id.trim();
        !app_id.is_empty()
            && apps.iter().any(|app| {
                app.get("id")
                    .and_then(|value| value.as_str())
                    .map(str::trim)
                    == Some(app_id)
            })
    }

    async fn recent_conversation_app_id(
        &self,
        conversation_id: Option<&str>,
        apps: &[serde_json::Value],
    ) -> Option<String> {
        let conversation_id = conversation_id?.trim();
        if conversation_id.is_empty() {
            return None;
        }
        self.load_recent_artifact_contexts(conversation_id)
            .await
            .into_iter()
            .find_map(|artifact| {
                if !artifact.artifact_type.eq_ignore_ascii_case("app") {
                    return None;
                }
                let app_id = artifact.artifact_id.trim();
                if Self::app_exists_in_registry(app_id, apps) {
                    Some(app_id.to_string())
                } else {
                    None
                }
            })
    }

    async fn resolve_service_manage_deploy_target(
        &self,
        arguments: &serde_json::Value,
        apps: &[serde_json::Value],
        conversation_id: Option<&str>,
    ) -> Option<String> {
        if let Some(service_id) = arguments
            .get("service_id")
            .or_else(|| arguments.get("app_id"))
            .and_then(|value| value.as_str())
            .map(str::trim)
            .filter(|value| !value.is_empty())
        {
            return Some(service_id.to_string());
        }

        let query = arguments
            .get("query")
            .and_then(|value| value.as_str())
            .unwrap_or("")
            .trim();
        if !query.is_empty() {
            let ranked = Self::rank_deployed_apps(query, apps);
            if let Some((_, app_id, _, _)) = Self::select_best_ranked_app(query, &ranked) {
                return Some(app_id.clone());
            }
        }

        self.recent_conversation_app_id(conversation_id, apps).await
    }

    fn service_manage_control_call(
        operation: &str,
        arguments: &serde_json::Value,
    ) -> crate::core::llm::ToolCall {
        let mut control = serde_json::Map::new();
        if let Some(service_id) = arguments
            .get("service_id")
            .or_else(|| arguments.get("app_id"))
            .and_then(|value| value.as_str())
            .map(str::trim)
            .filter(|value| !value.is_empty())
        {
            control.insert("app_id".to_string(), serde_json::json!(service_id));
        }
        if let Some(query) = arguments
            .get("query")
            .and_then(|value| value.as_str())
            .map(str::trim)
            .filter(|value| !value.is_empty())
        {
            control.insert("query".to_string(), serde_json::json!(query));
        }
        if let Some(name) = arguments
            .get("name")
            .or_else(|| arguments.get("title"))
            .and_then(|value| value.as_str())
            .map(str::trim)
            .filter(|value| !value.is_empty())
        {
            control.insert("title".to_string(), serde_json::json!(name));
        }
        if let Some(bundle_id) = arguments
            .get("bundle_id")
            .and_then(|value| value.as_str())
            .map(str::trim)
            .filter(|value| !value.is_empty())
        {
            control.insert("bundle_id".to_string(), serde_json::json!(bundle_id));
        }

        crate::core::llm::ToolCall {
            id: uuid::Uuid::new_v4().to_string(),
            name: format!("service_manage.{}", operation),
            arguments: serde_json::Value::Object(control),
            activity_label: None,
        }
    }

    async fn resolve_service_manage_target(
        &self,
        arguments: &serde_json::Value,
        apps: &[serde_json::Value],
    ) -> Option<String> {
        if let Some(service_id) = arguments
            .get("service_id")
            .or_else(|| arguments.get("app_id"))
            .and_then(|value| value.as_str())
            .map(str::trim)
            .filter(|value| !value.is_empty())
        {
            return Some(service_id.to_string());
        }

        let query = arguments
            .get("query")
            .and_then(|value| value.as_str())
            .unwrap_or("")
            .trim();
        let ranked = Self::rank_deployed_apps(query, apps);
        Self::select_best_ranked_app(query, &ranked).map(|(_, app_id, _, _)| app_id.clone())
    }

    async fn service_manage_target_not_found_payload(
        &self,
        arguments: &serde_json::Value,
        apps: &[serde_json::Value],
    ) -> serde_json::Value {
        let query = arguments
            .get("query")
            .and_then(|value| value.as_str())
            .unwrap_or("")
            .trim();
        let ranked = Self::rank_deployed_apps(query, apps);
        serde_json::json!({
            "status": "not_found",
            "service_id": serde_json::Value::Null,
            "query": if query.is_empty() { serde_json::Value::Null } else { serde_json::json!(query) },
            "services": Self::summarize_ranked_apps_for_user(&ranked, 10),
        })
    }

    pub(crate) async fn handle_service_manage_tool_call(
        &self,
        arguments: &serde_json::Value,
        stream_tx: Option<&tokio::sync::mpsc::Sender<StreamEvent>>,
        request_channel: &str,
        conversation_id: Option<&str>,
    ) -> Result<String> {
        let operation = arguments
            .get("operation")
            .and_then(|value| value.as_str())
            .unwrap_or("status")
            .trim()
            .to_ascii_lowercase();

        let output = match operation.as_str() {
            "create" | "update" | "patch" => {
                let apps = self.app_registry.list().await;
                if operation == "create" {
                    let mut deploy_args =
                        Self::normalize_service_manage_deploy_arguments(arguments);
                    let recent_app_id = self
                        .recent_conversation_app_id(conversation_id, &apps)
                        .await;
                    if Self::deploy_create_should_reuse_recent_app(
                        &operation,
                        arguments,
                        recent_app_id.is_some(),
                    ) {
                        if let Some(app_id) = recent_app_id {
                            if let Some(obj) = deploy_args.as_object_mut() {
                                obj.insert("app_id".to_string(), serde_json::json!(app_id));
                            }
                        }
                    }
                    let quality_contract = self
                        .app_deploy_quality_contract(&deploy_args, conversation_id)
                        .await;
                    let llm_env = self.app_model_env_vars();
                    let raw = crate::actions::app::app_deploy(
                        &self.config_dir,
                        &self.data_dir,
                        &deploy_args,
                        &self.app_registry,
                        &llm_env,
                        stream_tx.cloned(),
                    )
                    .await?;
                    self.service_manage_deploy_response_payload(
                        raw,
                        conversation_id,
                        request_channel,
                        &quality_contract,
                    )
                    .await
                } else if let Some(app_id) = self
                    .resolve_service_manage_deploy_target(arguments, &apps, conversation_id)
                    .await
                {
                    let mut deploy_args =
                        Self::normalize_service_manage_deploy_arguments(arguments);
                    if let Some(obj) = deploy_args.as_object_mut() {
                        obj.insert("app_id".to_string(), serde_json::json!(app_id));
                    }
                    let quality_contract = self
                        .app_deploy_quality_contract(&deploy_args, conversation_id)
                        .await;
                    let llm_env = self.app_model_env_vars();
                    let raw = crate::actions::app::app_deploy(
                        &self.config_dir,
                        &self.data_dir,
                        &deploy_args,
                        &self.app_registry,
                        &llm_env,
                        stream_tx.cloned(),
                    )
                    .await?;
                    self.service_manage_deploy_response_payload(
                        raw,
                        conversation_id,
                        request_channel,
                        &quality_contract,
                    )
                    .await
                } else {
                    self.service_manage_target_not_found_payload(arguments, &apps)
                        .await
                }
            }
            "start" | "restart" => {
                let call = Self::service_manage_control_call("restart", arguments);
                let raw = self
                    .handle_app_restart_tool_call(
                        &call,
                        stream_tx,
                        request_channel,
                        conversation_id,
                    )
                    .await?;
                let mut parsed = serde_json::from_str::<serde_json::Value>(&raw)
                    .unwrap_or_else(|_| serde_json::json!({ "status": "ok", "raw": raw }));
                if let Some(obj) = parsed.as_object_mut() {
                    obj.insert("tool".to_string(), serde_json::json!("service_manage"));
                    if let Some(app_id) = obj.get("app_id").cloned() {
                        obj.insert("service_id".to_string(), app_id);
                    }
                }
                parsed
            }
            "stop" => {
                let call = Self::service_manage_control_call("stop", arguments);
                let raw = self
                    .handle_app_stop_tool_call(&call, stream_tx, request_channel)
                    .await?;
                let mut parsed = serde_json::from_str::<serde_json::Value>(&raw)
                    .unwrap_or_else(|_| serde_json::json!({ "status": "ok", "raw": raw }));
                if let Some(obj) = parsed.as_object_mut() {
                    obj.insert("tool".to_string(), serde_json::json!("service_manage"));
                    if let Some(app_id) = obj.get("app_id").cloned() {
                        obj.insert("service_id".to_string(), app_id);
                    }
                }
                parsed
            }
            "delete" => {
                let call = Self::service_manage_control_call("delete", arguments);
                let raw = self
                    .handle_app_delete_tool_call(&call, stream_tx, request_channel)
                    .await?;
                let mut parsed = serde_json::from_str::<serde_json::Value>(&raw)
                    .unwrap_or_else(|_| serde_json::json!({ "status": "ok", "raw": raw }));
                if let Some(obj) = parsed.as_object_mut() {
                    obj.insert("tool".to_string(), serde_json::json!("service_manage"));
                    if let Some(app_id) = obj.get("app_id").cloned() {
                        obj.insert("service_id".to_string(), app_id);
                    }
                }
                parsed
            }
            "list" => {
                let services = self.app_registry.list().await;
                serde_json::json!({
                    "status": "ok",
                    "tool": "service_manage",
                    "services": services,
                })
            }
            "status" | "logs" => {
                let apps = self.app_registry.list().await;
                if operation == "status"
                    && arguments.get("service_id").is_none()
                    && arguments.get("app_id").is_none()
                    && arguments.get("query").is_none()
                {
                    serde_json::json!({
                        "status": "ok",
                        "tool": "service_manage",
                        "services": apps,
                    })
                } else if let Some(service_id) =
                    self.resolve_service_manage_target(arguments, &apps).await
                {
                    let app = apps.iter().find(|row| {
                        row.get("id").and_then(|value| value.as_str()) == Some(service_id.as_str())
                    });
                    if operation == "logs" {
                        if let Some(app_dir) = self.app_registry.get_dir(&service_id).await {
                            let max_bytes = arguments
                                .get("max_bytes")
                                .and_then(|value| value.as_u64())
                                .unwrap_or(8192)
                                .clamp(1024, 65536)
                                as usize;
                            let log_tail = crate::actions::app::read_local_runtime_log_tail(
                                &app_dir, max_bytes,
                            )
                            .await;
                            serde_json::json!({
                                "status": "ok",
                                "tool": "service_manage",
                                "service_id": service_id,
                                "logs": log_tail,
                            })
                        } else {
                            self.service_manage_target_not_found_payload(arguments, &apps)
                                .await
                        }
                    } else if let Some(app) = app {
                        serde_json::json!({
                            "status": "ok",
                            "tool": "service_manage",
                            "service_id": service_id,
                            "service": app,
                        })
                    } else {
                        self.service_manage_target_not_found_payload(arguments, &apps)
                            .await
                    }
                } else {
                    self.service_manage_target_not_found_payload(arguments, &apps)
                        .await
                }
            }
            _ => {
                anyhow::bail!(
                    "service_manage operation must be one of create, update, patch, start, restart, stop, status, logs, list, delete"
                );
            }
        };

        let formatted = serde_json::to_string_pretty(&output)?;
        if let Some(tx) = stream_tx {
            queue_stream_event(
                tx,
                StreamEvent::ToolResult {
                    name: "service_manage".to_string(),
                    content: formatted.clone(),
                },
            );
        }
        Ok(formatted)
    }

    async fn service_manage_deploy_response_payload(
        &self,
        raw: String,
        conversation_id: Option<&str>,
        _request_channel: &str,
        quality_contract: &AppDeployQualityContract,
    ) -> serde_json::Value {
        let mut parsed = serde_json::from_str::<serde_json::Value>(&raw)
            .unwrap_or_else(|_| serde_json::json!({ "status": "ok", "raw": raw }));
        if let Some(obj) = parsed.as_object_mut() {
            obj.insert("tool".to_string(), serde_json::json!("service_manage"));
            if let Some(app_id) = obj
                .get("app_id")
                .and_then(|value| value.as_str())
                .map(str::trim)
                .filter(|value| !value.is_empty())
                .map(ToString::to_string)
            {
                obj.insert("service_id".to_string(), serde_json::json!(app_id.clone()));
                let title = obj
                    .get("title")
                    .and_then(|value| value.as_str())
                    .unwrap_or("Service")
                    .to_string();
                let url = obj
                    .get("url")
                    .and_then(|value| value.as_str())
                    .map(str::trim)
                    .filter(|value| !value.is_empty())
                    .map(ToString::to_string)
                    .unwrap_or_else(|| format!("/apps/{}/", app_id));
                let app_type = obj
                    .get("type")
                    .and_then(|value| value.as_str())
                    .unwrap_or("static")
                    .to_string();
                if let Some(cid) = conversation_id {
                    self.persist_last_deployed_app_context(cid, &app_id, &title, &url)
                        .await;
                }
                let quality_report = self
                    .run_app_quality_check_for_deploy(&app_id, &url, &app_type, quality_contract)
                    .await;
                obj.insert(
                    "quality_check".to_string(),
                    Self::app_quality_tool_summary(quality_report.as_ref(), quality_contract),
                );
            }
        }
        parsed
    }

    async fn app_deploy_response_payload(
        &self,
        raw: String,
        conversation_id: Option<&str>,
        quality_contract: &AppDeployQualityContract,
    ) -> serde_json::Value {
        let mut parsed = serde_json::from_str::<serde_json::Value>(&raw)
            .unwrap_or_else(|_| serde_json::json!({ "status": "ok", "raw": raw }));
        if let Some(obj) = parsed.as_object_mut() {
            obj.insert("tool".to_string(), serde_json::json!("app_deploy"));
            if let Some(app_id) = obj
                .get("app_id")
                .and_then(|value| value.as_str())
                .map(str::trim)
                .filter(|value| !value.is_empty())
                .map(ToString::to_string)
            {
                let title = obj
                    .get("title")
                    .and_then(|value| value.as_str())
                    .unwrap_or("App")
                    .to_string();
                let url = obj
                    .get("url")
                    .and_then(|value| value.as_str())
                    .map(str::trim)
                    .filter(|value| !value.is_empty())
                    .map(ToString::to_string)
                    .unwrap_or_else(|| format!("/apps/{}/", app_id));
                let app_type = obj
                    .get("type")
                    .and_then(|value| value.as_str())
                    .unwrap_or("static")
                    .to_string();
                if let Some(cid) = conversation_id {
                    self.persist_last_deployed_app_context(cid, &app_id, &title, &url)
                        .await;
                }
                let quality_report = self
                    .run_app_quality_check_for_deploy(&app_id, &url, &app_type, quality_contract)
                    .await;
                obj.insert(
                    "quality_check".to_string(),
                    Self::app_quality_tool_summary(quality_report.as_ref(), quality_contract),
                );
            }
        }
        parsed
    }

    pub(crate) async fn handle_app_deploy_tool_call(
        &self,
        arguments: &serde_json::Value,
        stream_tx: Option<&tokio::sync::mpsc::Sender<StreamEvent>>,
        _request_channel: &str,
        conversation_id: Option<&str>,
    ) -> Result<String> {
        let apps = self.app_registry.list().await;
        let mut deploy_args = Self::normalize_app_deploy_arguments(arguments);
        let recent_app_id = self
            .recent_conversation_app_id(conversation_id, &apps)
            .await;
        if Self::deploy_create_should_reuse_recent_app(
            "create",
            &deploy_args,
            recent_app_id.is_some(),
        ) {
            if let Some(app_id) = recent_app_id {
                if let Some(obj) = deploy_args.as_object_mut() {
                    obj.insert("app_id".to_string(), serde_json::json!(app_id));
                }
            }
        }
        let quality_contract = self
            .app_deploy_quality_contract(&deploy_args, conversation_id)
            .await;
        let llm_env = self.app_model_env_vars();
        let raw = crate::actions::app::app_deploy(
            &self.config_dir,
            &self.data_dir,
            &deploy_args,
            &self.app_registry,
            &llm_env,
            stream_tx.cloned(),
        )
        .await?;
        Ok(self
            .app_deploy_response_payload(raw, conversation_id, &quality_contract)
            .await
            .to_string())
    }

    pub(crate) async fn handle_work_manage_tool_call(
        &self,
        arguments: &serde_json::Value,
        conversation_id: Option<&str>,
    ) -> String {
        let operation = work_manage_operation(arguments);
        let kind = arguments
            .get("kind")
            .or_else(|| arguments.get("resource_kind"))
            .and_then(|value| value.as_str())
            .map(str::trim)
            .filter(|value| !value.is_empty())
            .map(|value| value.to_ascii_lowercase());

        if matches!(kind.as_deref(), Some("scheduled_task" | "task"))
            || arguments.get("task_id").is_some()
        {
            return self
                .handle_scheduled_task_work_manage(arguments, &operation)
                .await;
        }
        if matches!(kind.as_deref(), Some("watcher")) || arguments.get("watcher_id").is_some() {
            return self.handle_watcher_work_manage(arguments, &operation).await;
        }

        let mut mapped = serde_json::Map::new();
        mapped.insert("operation".to_string(), serde_json::json!(operation));
        if let Some(work_id) = arguments
            .get("work_id")
            .or_else(|| arguments.get("background_session_id"))
            .and_then(|value| value.as_str())
            .map(str::trim)
            .filter(|value| !value.is_empty())
        {
            mapped.insert(
                "background_session_id".to_string(),
                serde_json::json!(work_id),
            );
        }
        if let Some(reference_text) = arguments
            .get("reference_text")
            .or_else(|| arguments.get("query"))
            .and_then(|value| value.as_str())
            .map(str::trim)
            .filter(|value| !value.is_empty())
        {
            mapped.insert(
                "reference_text".to_string(),
                serde_json::json!(reference_text),
            );
        }
        if let Some(delivery_channel) = arguments
            .get("delivery_channel")
            .or_else(|| arguments.get("report_to"))
            .and_then(|value| value.as_str())
            .map(str::trim)
            .filter(|value| !value.is_empty())
        {
            mapped.insert(
                "delivery_channel".to_string(),
                serde_json::json!(delivery_channel),
            );
        }
        if let Some(include_closed) = arguments.get("include_closed").cloned() {
            mapped.insert("include_closed".to_string(), include_closed);
        }

        let raw = self
            .handle_background_session_manage(&serde_json::Value::Object(mapped), conversation_id)
            .await
            .unwrap_or_else(|| {
                render_tool_completion_marker_with_data(
                    "background_work_manage",
                    "failed",
                    "AgentArk could not resolve that work-management operation.",
                    serde_json::json!({
                        "success": false,
                        "reason": "missing_work_manage_result",
                    }),
                )
            });

        raw.replace("background_session_manage", "background_work_manage")
    }

    async fn handle_scheduled_task_work_manage(
        &self,
        arguments: &serde_json::Value,
        operation: &str,
    ) -> String {
        if operation == "list" {
            let tasks = self.tasks.read().await.all().to_vec();
            let rows = tasks
                .iter()
                .map(scheduled_task_work_row)
                .collect::<Vec<_>>();
            let detail = format!("Found {} scheduled task(s).", rows.len());
            return render_tool_completion_marker_with_data(
                "background_work_manage",
                "completed",
                &detail,
                serde_json::json!({
                    "kind": "scheduled_task",
                    "operation": operation,
                    "task_count": rows.len(),
                    "tasks": rows,
                }),
            );
        }

        let task_id = match work_manage_id(arguments, "task_id") {
            Some(task_id) => task_id,
            None => {
                let Some(reference) = work_manage_reference_text(arguments) else {
                    return render_tool_completion_marker_with_data(
                        "background_work_manage",
                        "failed",
                        "Scheduled task operation requires task_id or reference_text.",
                        serde_json::json!({
                            "success": false,
                            "kind": "scheduled_task",
                            "operation": operation,
                            "reason": "missing_task_reference",
                        }),
                    );
                };
                let matches = {
                    let tasks = self.tasks.read().await;
                    tasks
                        .all()
                        .iter()
                        .filter(|task| {
                            work_text_matches_reference(
                                &scheduled_task_reference_blob(task),
                                &reference,
                            )
                        })
                        .map(scheduled_task_work_row)
                        .collect::<Vec<_>>()
                };
                match matches.as_slice() {
                    [only] => only
                        .get("id")
                        .and_then(|value| value.as_str())
                        .unwrap_or_default()
                        .to_string(),
                    [] => {
                        return render_tool_completion_marker_with_data(
                            "background_work_manage",
                            "completed",
                            "No scheduled task matched that reference.",
                            serde_json::json!({
                                "success": true,
                                "kind": "scheduled_task",
                                "operation": operation,
                                "status": "not_found",
                                "reference_text": reference,
                                "terminal_observation": true,
                            }),
                        );
                    }
                    _ => {
                        return render_tool_completion_marker_with_data(
                            "background_work_manage",
                            "failed",
                            "More than one scheduled task matched that reference.",
                            serde_json::json!({
                                "success": false,
                                "kind": "scheduled_task",
                                "operation": operation,
                                "reason": "ambiguous_reference",
                                "reference_text": reference,
                                "matches": matches,
                            }),
                        );
                    }
                }
            }
        };
        let task_uuid = match uuid::Uuid::parse_str(&task_id) {
            Ok(id) => id,
            Err(error) => {
                return render_tool_completion_marker_with_data(
                    "background_work_manage",
                    "failed",
                    &format!("Invalid task_id `{}`: {}.", task_id, error),
                    serde_json::json!({
                        "success": false,
                        "kind": "scheduled_task",
                        "operation": operation,
                        "task_id": task_id,
                        "reason": "invalid_task_id",
                    }),
                );
            }
        };

        if matches!(operation, "status" | "read") {
            let task = self
                .tasks
                .read()
                .await
                .all()
                .iter()
                .find(|task| task.id == task_uuid)
                .cloned();
            let Some(task) = task else {
                return durable_work_not_found("scheduled_task", operation, &task_id);
            };
            let detail = "Scheduled task status loaded.".to_string();
            return render_tool_completion_marker_with_data(
                "background_work_manage",
                "completed",
                &detail,
                serde_json::json!({
                    "kind": "scheduled_task",
                    "operation": operation,
                    "task_id": task_id,
                    "task": scheduled_task_work_row(&task),
                    "object_refs": [{"kind": "task", "id": task_id}],
                }),
            );
        }

        if operation == "delete" {
            let removed_live = self.tasks.write().await.remove(task_uuid);
            let storage_result = self.storage.delete_task(&task_id).await;
            self.background_sessions
                .remove_child_references(std::slice::from_ref(&task_id), &[], Some("agent"))
                .await;
            if let Err(error) = storage_result {
                return render_tool_completion_marker_with_data(
                    "background_work_manage",
                    "failed",
                    &format!("Failed to delete scheduled task: {}", error),
                    serde_json::json!({
                        "success": false,
                        "kind": "scheduled_task",
                        "operation": operation,
                        "task_id": task_id,
                        "reason": "storage_delete_failed",
                    }),
                );
            }
            let detail = "Deleted scheduled task.".to_string();
            return render_tool_completion_marker_with_data(
                "background_work_manage",
                "completed",
                &detail,
                serde_json::json!({
                    "kind": "scheduled_task",
                    "operation": operation,
                    "task_id": task_id,
                    "deleted_live": removed_live,
                    "object_refs": [{"kind": "task", "id": task_id}],
                }),
            );
        }

        let changed;
        let updated_task = {
            let mut tasks = self.tasks.write().await;
            let Some(task) = tasks.get_mut(task_uuid) else {
                return durable_work_not_found("scheduled_task", operation, &task_id);
            };
            let before = task.status.clone();
            match operation {
                "pause" => {
                    if matches!(
                        task.status,
                        crate::core::TaskStatus::Pending
                            | crate::core::TaskStatus::AwaitingApproval
                            | crate::core::TaskStatus::ExpiredNeedsReapproval
                            | crate::core::TaskStatus::InProgress
                    ) {
                        task.status = crate::core::TaskStatus::Paused;
                    }
                }
                "resume" => {
                    if matches!(task.status, crate::core::TaskStatus::Paused) {
                        task.status = crate::core::TaskStatus::Pending;
                    }
                }
                "stop" | "cancel" => {
                    if !matches!(
                        task.status,
                        crate::core::TaskStatus::Completed | crate::core::TaskStatus::Cancelled
                    ) {
                        task.status = crate::core::TaskStatus::Cancelled;
                        task.result = Some("Cancelled by user.".to_string());
                    }
                }
                "update_delivery" => {
                    let delivery_channel = normalize_automation_notification_channel(
                        arguments
                            .get("delivery_channel")
                            .or_else(|| arguments.get("report_to"))
                            .and_then(|value| value.as_str()),
                    );
                    let mut next_args = task.arguments.clone();
                    if let Some(object) = next_args.as_object_mut() {
                        object.insert(
                            "report_to".to_string(),
                            serde_json::Value::String(delivery_channel),
                        );
                    } else {
                        next_args = serde_json::json!({"report_to": delivery_channel});
                    }
                    task.arguments = next_args;
                }
                _ => {
                    return render_tool_completion_marker_with_data(
                        "background_work_manage",
                        "failed",
                        &format!("Unsupported scheduled task operation `{}`.", operation),
                        serde_json::json!({
                            "success": false,
                            "kind": "scheduled_task",
                            "operation": operation,
                            "task_id": task_id,
                            "reason": "unsupported_operation",
                        }),
                    );
                }
            }
            changed = before != task.status || operation == "update_delivery";
            Some(task.clone())
        };

        if let Some(task) = updated_task.as_ref() {
            let persist_result = match operation {
                "pause" | "resume" => match serde_json::to_string(&task.status) {
                    Ok(status) => self.storage.update_task_status(&task_id, &status).await,
                    Err(error) => Err(error.into()),
                },
                "stop" | "cancel" => match serde_json::to_string(&task.status) {
                    Ok(status) => {
                        self.storage
                            .update_task_status_and_result(
                                &task_id,
                                &status,
                                task.result.as_deref(),
                            )
                            .await
                    }
                    Err(error) => Err(error.into()),
                },
                "update_delivery" => match serde_json::to_string(&task.arguments) {
                    Ok(arguments) => {
                        self.storage
                            .update_task(&task_id, None, Some(arguments), None, None)
                            .await
                    }
                    Err(error) => Err(error.into()),
                },
                _ => Ok(()),
            };
            if let Err(error) = persist_result {
                return render_tool_completion_marker_with_data(
                    "background_work_manage",
                    "failed",
                    &format!("Failed to persist scheduled task operation: {}", error),
                    serde_json::json!({
                        "success": false,
                        "kind": "scheduled_task",
                        "operation": operation,
                        "task_id": task_id,
                        "reason": "storage_update_failed",
                    }),
                );
            }
        }

        let detail = format!("Scheduled task {} completed.", operation);
        render_tool_completion_marker_with_data(
            "background_work_manage",
            "completed",
            &detail,
            serde_json::json!({
                "kind": "scheduled_task",
                "operation": operation,
                "task_id": task_id,
                "changed": changed,
                "task": updated_task.as_ref().map(scheduled_task_work_row),
                "object_refs": [{"kind": "task", "id": task_id}],
            }),
        )
    }

    async fn handle_watcher_work_manage(
        &self,
        arguments: &serde_json::Value,
        operation: &str,
    ) -> String {
        if operation == "list" {
            return self
                .execute_direct_list_watchers_tool(arguments)
                .await
                .unwrap_or_else(|error| error.to_string());
        }
        let watcher_id = match work_manage_id(arguments, "watcher_id") {
            Some(watcher_id) => watcher_id,
            None => {
                let Some(reference) = work_manage_reference_text(arguments) else {
                    return render_tool_completion_marker_with_data(
                        "background_work_manage",
                        "failed",
                        "Watcher operation requires watcher_id or reference_text.",
                        serde_json::json!({
                            "success": false,
                            "kind": "watcher",
                            "operation": operation,
                            "reason": "missing_watcher_reference",
                        }),
                    );
                };
                let matches = self
                    .watcher_manager
                    .list()
                    .await
                    .into_iter()
                    .filter(|watcher| {
                        work_text_matches_reference(&watcher_reference_blob(watcher), &reference)
                    })
                    .map(|watcher| list_watchers_live_row(&watcher))
                    .collect::<Vec<_>>();
                match matches.as_slice() {
                    [only] => only
                        .get("id")
                        .and_then(|value| value.as_str())
                        .unwrap_or_default()
                        .to_string(),
                    [] => {
                        return render_tool_completion_marker_with_data(
                            "background_work_manage",
                            "completed",
                            "No watcher matched that reference.",
                            serde_json::json!({
                                "success": true,
                                "kind": "watcher",
                                "operation": operation,
                                "status": "not_found",
                                "reference_text": reference,
                                "terminal_observation": true,
                            }),
                        );
                    }
                    _ => {
                        return render_tool_completion_marker_with_data(
                            "background_work_manage",
                            "failed",
                            "More than one watcher matched that reference.",
                            serde_json::json!({
                                "success": false,
                                "kind": "watcher",
                                "operation": operation,
                                "reason": "ambiguous_reference",
                                "reference_text": reference,
                                "matches": matches,
                            }),
                        );
                    }
                }
            }
        };
        let watcher_uuid = match uuid::Uuid::parse_str(&watcher_id) {
            Ok(id) => id,
            Err(error) => {
                return render_tool_completion_marker_with_data(
                    "background_work_manage",
                    "failed",
                    &format!("Invalid watcher_id `{}`: {}.", watcher_id, error),
                    serde_json::json!({
                        "success": false,
                        "kind": "watcher",
                        "operation": operation,
                        "watcher_id": watcher_id,
                        "reason": "invalid_watcher_id",
                    }),
                );
            }
        };
        if matches!(operation, "status" | "read") {
            let Some(watcher) = self.watcher_manager.get(watcher_uuid).await else {
                return durable_work_not_found("watcher", operation, &watcher_id);
            };
            let detail = "Watcher status loaded.".to_string();
            return render_tool_completion_marker_with_data(
                "background_work_manage",
                "completed",
                &detail,
                serde_json::json!({
                    "kind": "watcher",
                    "operation": operation,
                    "watcher_id": watcher_id,
                    "watcher": list_watchers_live_row(&watcher),
                    "object_refs": [{"kind": "watcher", "id": watcher_id}],
                }),
            );
        }
        if operation == "delete" {
            return self
                .execute_direct_watcher_delete_tool(&serde_json::json!({"watcher_id": watcher_id}))
                .await
                .unwrap_or_else(|error| error.to_string())
                .replace("watcher_delete", "background_work_manage");
        }
        let changed = match operation {
            "pause" => self.watcher_manager.pause(watcher_uuid).await,
            "resume" => self.watcher_manager.resume(watcher_uuid).await,
            "stop" | "cancel" => self.watcher_manager.cancel(watcher_uuid).await,
            "update_delivery" => {
                let delivery_channel = normalize_automation_notification_channel(
                    arguments
                        .get("delivery_channel")
                        .or_else(|| arguments.get("report_to"))
                        .and_then(|value| value.as_str()),
                );
                self.watcher_manager
                    .set_notify_channel(watcher_uuid, &delivery_channel)
                    .await
            }
            _ => {
                return render_tool_completion_marker_with_data(
                    "background_work_manage",
                    "failed",
                    &format!("Unsupported watcher operation `{}`.", operation),
                    serde_json::json!({
                        "success": false,
                        "kind": "watcher",
                        "operation": operation,
                        "watcher_id": watcher_id,
                        "reason": "unsupported_operation",
                    }),
                );
            }
        };
        let watcher = self.watcher_manager.get(watcher_uuid).await;
        if let Some(watcher) = watcher.as_ref() {
            let status_label = Self::watcher_supervisor_status_label(&watcher.status);
            self.sync_watcher_supervisor_state(watcher, Some(status_label.as_str()), None)
                .await;
        }
        let detail = format!("Watcher {} completed.", operation);
        render_tool_completion_marker_with_data(
            "background_work_manage",
            "completed",
            &detail,
            serde_json::json!({
                "kind": "watcher",
                "operation": operation,
                "watcher_id": watcher_id,
                "changed": changed,
                "watcher": watcher.as_ref().map(list_watchers_live_row),
                "object_refs": [{"kind": "watcher", "id": watcher_id}],
            }),
        )
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

        let now = chrono::Utc::now();
        let background_active = self
            .background_sessions
            .list()
            .await
            .into_iter()
            .filter(|session| {
                background_session_blocks_gepa_idle(session, now, quiet_window_seconds)
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
        metadata: serde_json::Value,
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
            metadata,
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
            let _guard = GepaIdleWorkerActiveGuard;
            let result = agent.run_gepa_idle_worker().await;
            if let Err(error) = result {
                tracing::warn!("GEPA pending job worker failed: {}", error);
            }
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
            serde_json::Value::Null,
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

    pub(crate) async fn queue_gepa_prompt_optimization_run(
        &self,
        proposal_id: &str,
        request: &str,
        quiet_window_seconds: i64,
        opportunity_context: Option<serde_json::Value>,
    ) -> Result<String> {
        let promotion_settings =
            gepa_prompt_optimization_promotion_settings(opportunity_context.as_ref());
        self.queue_gepa_job(
            crate::core::self_evolve::gepa_bridge::GepaJobKind::Run,
            request,
            gepa_prompt_optimization_metadata(proposal_id, opportunity_context),
            None,
            None,
            None,
            quiet_window_seconds,
            promotion_settings,
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
                        let (status, status_path) = if gepa_job_failure_retryable(&value) {
                            crate::core::self_evolve::gepa_bridge::fail_claimed_job(
                                &project_root,
                                &running_path,
                                job_for_failure,
                                message,
                            )
                            .await?
                        } else {
                            let status_path =
                                crate::core::self_evolve::gepa_bridge::fail_claimed_job_terminal(
                                    &project_root,
                                    &running_path,
                                    job_for_failure,
                                    message,
                                )
                                .await?;
                            ("failed".to_string(), status_path)
                        };
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
                let result = crate::core::self_evolve::gepa_bridge::export_optimization_bundle_with_metadata(
                    &self.storage,
                    &project_root,
                    &job.request,
                    job.promotion.replay_log_limit,
                    Some(&job.metadata),
                )
                .await?;
                Ok(serde_json::to_value(result)?)
            }
            crate::core::self_evolve::gepa_bridge::GepaJobKind::Run => {
                let export_path = if let Some(path) = job.export_path.as_ref() {
                    resolve_gepa_workspace_path(&project_root, path)?
                } else {
                    let export = crate::core::self_evolve::gepa_bridge::export_optimization_bundle_with_metadata(
                        &self.storage,
                        &project_root,
                        &job.request,
                        job.promotion.replay_log_limit,
                        Some(&job.metadata),
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
                if let Err(error) =
                    crate::core::self_evolve::gepa_bridge::validate_python_optimizer_runtime(
                        &runtime,
                    )
                    .await
                {
                    return Ok(serde_json::json!({
                        "status": "blocked",
                        "mode": "gepa_run",
                        "error": error.to_string(),
                    }));
                }
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

        if !imported.arkdistill_candidates.is_empty() {
            let current_arkdistill_raw = self
                .storage
                .get(crate::core::agent::ark_distill::ARKDISTILL_PROFILE_KEY)
                .await
                .ok()
                .flatten();
            let result = crate::core::agent::ark_distill::evaluate_external_arkdistill_candidates(
                request,
                current_arkdistill_raw.as_deref(),
                imported.arkdistill_candidates,
            )?;
            let mut promotion_applied = false;
            if result.promoted && apply_promotion {
                if let Some(profile) = result.promoted_profile.as_ref() {
                    if let Some(existing_baseline) = current_arkdistill_raw.as_ref() {
                        let _ = self
                            .storage
                            .set(
                                crate::core::agent::ark_distill::ARKDISTILL_PROFILE_BASELINE_SNAPSHOT_KEY,
                                existing_baseline,
                            )
                            .await;
                    }
                    self.storage
                        .set(
                            crate::core::agent::ark_distill::ARKDISTILL_PROFILE_KEY,
                            &serde_json::to_vec(profile)?,
                        )
                        .await?;
                    promotion_applied = true;
                }
            }
            let mut value = serde_json::to_value(&result)?;
            if let serde_json::Value::Object(obj) = &mut value {
                obj.insert(
                    "mode".to_string(),
                    serde_json::json!("gepa_import_arkdistill_profile"),
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
                    "candidate_source_path".to_string(),
                    serde_json::json!(candidates_path.display().to_string()),
                );
                obj.insert(
                    "rollback_available".to_string(),
                    serde_json::json!(promotion_applied),
                );
            }
            if let Ok(bytes) = serde_json::to_vec(&value) {
                let _ = self
                    .storage
                    .set(
                        crate::core::agent::ark_distill::ARKDISTILL_PROFILE_LAST_RESULT_KEY,
                        &bytes,
                    )
                    .await;
            }
            results.push(value);
        }

        if !imported.router_learning_candidates.is_empty() {
            let router_learning_candidates = imported.router_learning_candidates;
            results.push(serde_json::json!({
                "success": true,
                "mode": "gepa_import_router_learning",
                "promoted": false,
                "promotion_applied": false,
                "runtime_promotion_applied": false,
                "promotion_requires_user_acceptance": true,
                "candidate_source_path": candidates_path.display().to_string(),
                "candidate_count": router_learning_candidates.len(),
                "promotion_gate": "router learning candidates imported as review-only layer-specific evidence; runtime router code is not auto-promoted",
                "router_learning_candidates": router_learning_candidates,
            }));
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
                "message": "Self-evolution is currently disabled. Enable it in Settings > Advanced > Evolve."
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
        let apply_promotion_requested = call
            .arguments
            .get("apply_promotion")
            .and_then(|v| v.as_bool())
            .unwrap_or(false);
        let apply_promotion = apply_promotion_requested;
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
                if mode == "prompt" {
                    "prompt"
                } else if mode == "specialist_prompt" {
                    "specialist prompt"
                } else if mode.starts_with("gepa_") {
                    "GEPA"
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
                            serde_json::Value::Null,
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
                                    crate::core::self_evolve::gepa_bridge::validate_python_optimizer_runtime(
                                        &runtime,
                                    )
                                    .await
                                {
                                    serde_json::json!({
                                        "status": "blocked",
                                        "mode": "gepa_run",
                                        "error": error.to_string(),
                                    })
                                } else if let Err(error) =
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
                            if replay_eval.promote
                                && stable_deployment_cadence_allows_auto_promotion(
                                    &self.storage,
                                    "routing_policy",
                                    &candidate_version,
                                )
                                .await
                            {
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
                                record_auto_stable_deployment(
                                    &self.storage,
                                    "routing_policy",
                                    &candidate_version,
                                )
                                .await;
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
                            if replay_eval.promote
                                && stable_deployment_cadence_allows_auto_promotion(
                                    &self.storage,
                                    "prompt",
                                    &candidate_version,
                                )
                                .await
                            {
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
                                record_auto_stable_deployment(
                                    &self.storage,
                                    "prompt",
                                    &candidate_version,
                                )
                                .await;
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
                        "Unsupported self_evolve mode '{}'. Use mode='policy' (default), mode='prompt', mode='specialist_prompt', mode='gepa_export', mode='gepa_run', mode='gepa_import', or mode='gepa_status'.",
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
                    if replay_eval.promote
                        && stable_deployment_cadence_allows_auto_promotion(
                            &self.storage,
                            "specialist_prompt",
                            &candidate_version,
                        )
                        .await
                    {
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
                        record_auto_stable_deployment(
                            &self.storage,
                            "specialist_prompt",
                            &candidate_version,
                        )
                        .await;
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

    /// Determine the root used for GEPA artifacts and workspace-relative paths.
    fn find_project_root(&self) -> std::path::PathBuf {
        crate::core::self_evolve::gepa_bridge::resolve_gepa_project_root()
    }
}

fn tool_call_results_response(results: Vec<String>) -> String {
    results.join("\n")
}

fn action_text_arg(arguments: &serde_json::Value, key: &str) -> Option<String> {
    arguments
        .get(key)
        .and_then(|value| value.as_str())
        .map(str::trim)
        .filter(|value| !value.is_empty())
        .map(ToString::to_string)
}

fn normalized_enum_token(value: &str) -> String {
    value
        .trim()
        .chars()
        .map(|ch| {
            if ch.is_ascii_alphanumeric() {
                ch.to_ascii_lowercase()
            } else {
                '_'
            }
        })
        .collect::<String>()
        .split('_')
        .filter(|part| !part.is_empty())
        .collect::<Vec<_>>()
        .join("_")
}

fn browser_profile_target_kind_from_arguments(
    arguments: &serde_json::Value,
) -> Result<Option<crate::core::BrowserProfileTargetKind>> {
    let Some(raw) = action_text_arg(arguments, "target_kind") else {
        return Ok(None);
    };
    serde_json::from_value::<crate::core::BrowserProfileTargetKind>(serde_json::Value::String(
        normalized_enum_token(&raw),
    ))
    .map(Some)
    .map_err(|error| anyhow::anyhow!("Invalid browser profile target kind: {}", error))
}

fn browser_profile_login_state_from_arguments(
    arguments: &serde_json::Value,
) -> Result<Option<crate::core::BrowserLoginState>> {
    let Some(raw) = action_text_arg(arguments, "login_state") else {
        return Ok(None);
    };
    serde_json::from_value::<crate::core::BrowserLoginState>(serde_json::Value::String(
        normalized_enum_token(&raw),
    ))
    .map(Some)
    .map_err(|error| anyhow::anyhow!("Invalid browser profile login state: {}", error))
}

fn browser_profile_tags_from_arguments(arguments: &serde_json::Value) -> Vec<String> {
    let Some(value) = arguments.get("tags") else {
        return Vec::new();
    };
    let values = if let Some(items) = value.as_array() {
        items
            .iter()
            .filter_map(|item| item.as_str())
            .map(str::to_string)
            .collect::<Vec<_>>()
    } else if let Some(text) = value.as_str() {
        text.split(',').map(str::to_string).collect::<Vec<_>>()
    } else {
        Vec::new()
    };
    values
        .into_iter()
        .map(|value| value.trim().to_string())
        .filter(|value| !value.is_empty())
        .collect()
}

fn browser_auto_task_description(
    url: Option<&str>,
    recent_artifact: Option<&ConversationArtifactContext>,
    task: Option<String>,
    expectation: Option<String>,
) -> String {
    let explicit_url = url
        .map(str::trim)
        .filter(|value| !value.is_empty());
    let recent_artifact = if explicit_url.is_none() {
        recent_artifact.and_then(|artifact| {
            let artifact_url = artifact.url.trim();
            (!artifact_url.is_empty()).then_some((artifact, artifact_url))
        })
    } else {
        None
    };
    let task_text = task
        .as_deref()
        .map(str::trim)
        .filter(|value| !value.is_empty())
        .map(ToString::to_string);
    let base = if let Some(task_text) = task_text {
        if let Some((artifact, artifact_url)) = recent_artifact {
            let title = artifact.title.trim();
            let target = if title.is_empty() {
                artifact_url.to_string()
            } else {
                format!("{} ({})", title, artifact_url)
            };
            format!(
                "Recent actionable URL: {target}\nUse this URL when the browser task refers to the current conversation artifact and no other URL is specified.\n\nTask: {task_text}"
            )
        } else {
            task_text
        }
    } else if let Some(url) = explicit_url {
        format!("Open {}", url)
    } else if let Some((artifact, artifact_url)) = recent_artifact {
        let title = artifact.title.trim();
        if title.is_empty() {
            format!("Open {}", artifact_url)
        } else {
            format!("Open {} ({})", title, artifact_url)
        }
    } else {
        String::new()
    };
    let Some(expectation) = expectation
        .as_deref()
        .map(str::trim)
        .filter(|value| !value.is_empty())
    else {
        return base;
    };
    if base.is_empty() {
        return format!("Expected checkpoint: {}", expectation);
    }
    format!("{base}\n\nExpected checkpoint: {expectation}")
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum BrowserAutoCheckpointKind {
    NeedsInput,
    Failed,
    Completed,
    Running,
}

impl BrowserAutoCheckpointKind {
    fn marker_status(self) -> &'static str {
        match self {
            Self::NeedsInput => "needs_input",
            Self::Failed => "failed",
            Self::Completed => "completed",
            Self::Running => "running",
        }
    }

    fn success(self) -> bool {
        !matches!(self, Self::Failed)
    }
}

fn checkpoint_text(value: Option<&str>) -> Option<&str> {
    value.map(str::trim).filter(|value| !value.is_empty())
}

fn browser_auto_checkpoint_kind(
    view: &crate::core::browser_session::BrowserSessionView,
) -> BrowserAutoCheckpointKind {
    if view.question.is_some() {
        BrowserAutoCheckpointKind::NeedsInput
    } else if view.reason.is_some() {
        BrowserAutoCheckpointKind::Failed
    } else if view.summary.is_some() {
        BrowserAutoCheckpointKind::Completed
    } else {
        BrowserAutoCheckpointKind::Running
    }
}

fn browser_auto_checkpoint_detail(
    view: &crate::core::browser_session::BrowserSessionView,
    kind: BrowserAutoCheckpointKind,
) -> String {
    match kind {
        BrowserAutoCheckpointKind::NeedsInput => checkpoint_text(view.question.as_deref())
            .unwrap_or("The browser task needs user input to continue.")
            .to_string(),
        BrowserAutoCheckpointKind::Failed => checkpoint_text(view.reason.as_deref())
            .unwrap_or("The browser task failed.")
            .to_string(),
        BrowserAutoCheckpointKind::Completed => checkpoint_text(view.summary.as_deref())
            .unwrap_or("The browser task reached a stable checkpoint.")
            .to_string(),
        BrowserAutoCheckpointKind::Running => "The managed browser task is still running. Do not state the requested browser outcome as complete until a later browser status confirms it.".to_string(),
    }
}

fn browser_auto_page_snapshot_payload(
    content: &crate::integrations::browser::PageContent,
) -> serde_json::Value {
    let downloads = content
        .downloads
        .iter()
        .map(browser_auto_download_payload)
        .collect::<Vec<_>>();
    serde_json::json!({
        "title": content.title,
        "url": content.url,
        "body_text": safe_truncate(&content.body_text, 6_000),
        "elements": content.elements.iter().take(50).cloned().collect::<Vec<_>>(),
        "diagnostics": content.diagnostics,
        "downloads": downloads,
        "download_dir": content.download_dir,
    })
}

fn browser_auto_download_payload(
    download: &crate::integrations::browser::BrowserDownloadArtifact,
) -> serde_json::Value {
    let created_at = if download.downloaded_at.trim().is_empty() {
        chrono::Utc::now().to_rfc3339()
    } else {
        download.downloaded_at.clone()
    };
    let mime = mime_guess::from_path(if download.path.trim().is_empty() {
        download.filename.as_str()
    } else {
        download.path.as_str()
    })
    .first_raw()
    .map(str::to_string);
    let mut payload = serde_json::json!({
        "id": download.id,
        "filename": download.filename,
        "path": download.path,
        "bytes": download.bytes,
        "url": download.url,
        "downloaded_at": download.downloaded_at,
        "status": download.status,
    });
    if !download.path.trim().is_empty() {
        payload["source_resource"] = serde_json::json!({
            "id": format!("browser-download:{}", download.id),
            "path": download.path,
            "mime": mime,
            "bytes": download.bytes,
            "created_at": created_at,
            "source_action": "browser_auto",
        });
    }
    payload
}

fn browser_auto_checkpoint_marker(
    view: crate::core::browser_session::BrowserSessionView,
) -> String {
    let marker_kind = browser_auto_checkpoint_kind(&view);
    let session_id = view.id.clone();
    let detail = browser_auto_checkpoint_detail(&view, marker_kind);
    let current_page = view
        .last_page
        .as_ref()
        .map(browser_auto_page_snapshot_payload);
    render_tool_completion_marker_with_data(
        "browser_auto",
        marker_kind.marker_status(),
        &detail,
        serde_json::json!({
            "success": marker_kind.success(),
            "session_id": session_id,
            "current_page": current_page,
            "session": view,
        }),
    )
}

fn skills_surface_json() -> serde_json::Value {
    serde_json::json!({
        "label": "Skills",
        "path": "/skills",
        "manage_hint": "Open Skills to review, edit, enable, disable, test, import, delete, or manage marketplaces."
    })
}

fn skill_chat_review_gate_result(
    preview: &serde_json::Value,
    operation: &str,
    confirmed_non_blocking_risk: bool,
) -> Option<serde_json::Value> {
    let security = preview.get("security").unwrap_or(&serde_json::Value::Null);
    let blocked = security
        .get("blocked")
        .and_then(|value| value.as_bool())
        .unwrap_or(false)
        || preview
            .get("status")
            .and_then(|value| value.as_str())
            .is_some_and(|status| status == "blocked");
    let risk_band = security
        .get("risk_band")
        .and_then(|value| value.as_str())
        .unwrap_or("unknown");
    let risk_score = security
        .get("risk_score_10")
        .and_then(|value| value.as_f64())
        .unwrap_or(0.0);
    let warnings = security
        .get("warnings")
        .and_then(|value| value.as_array())
        .cloned()
        .unwrap_or_default();
    let matched_rules = security
        .get("matched_rules")
        .cloned()
        .unwrap_or_else(|| serde_json::json!([]));
    let findings = security
        .get("findings")
        .cloned()
        .unwrap_or_else(|| serde_json::json!([]));
    let review_summary = security
        .get("review_summary")
        .and_then(|value| value.as_str())
        .unwrap_or("");
    let non_blocking_risk =
        !warnings.is_empty() || matches!(risk_band, "review" | "risky" | "unknown");

    if blocked {
        return Some(serde_json::json!({
            "status": "blocked",
            "resource": "skill",
            "operation": operation,
            "name": preview.get("name").cloned().unwrap_or(serde_json::Value::Null),
            "message": format!("Skill {} blocked by security review. Risk band: {}; score: {:.1}/10.", operation, risk_band, risk_score),
            "review": {
                "risk_band": risk_band,
                "risk_score_10": risk_score,
                "warnings": warnings,
                "findings": findings,
                "matched_rules": matched_rules,
                "summary": review_summary,
            },
            "surface": skills_surface_json(),
            "confirmation": {
                "required": false,
                "allowed": false,
                "reason": "blocked_by_skill_security_policy"
            }
        }));
    }

    if non_blocking_risk && !confirmed_non_blocking_risk {
        return Some(serde_json::json!({
            "status": "needs_input",
            "resource": "skill",
            "operation": operation,
            "name": preview.get("name").cloned().unwrap_or(serde_json::Value::Null),
            "message": format!("Skill {} needs confirmation before saving. Risk band: {}; score: {:.1}/10.", operation, risk_band, risk_score),
            "review": {
                "risk_band": risk_band,
                "risk_score_10": risk_score,
                "warnings": warnings,
                "findings": findings,
                "matched_rules": matched_rules,
                "summary": review_summary,
            },
            "surface": skills_surface_json(),
            "confirmation": {
                "required": true,
                "allowed": true,
                "reason": "non_blocking_skill_security_risk",
                "retry_with": {
                    "resource": "skill",
                    "operation": operation,
                    "security_confirmed": true
                }
            }
        }));
    }

    None
}

fn merge_objects_json(
    mut base: serde_json::Value,
    overlay: serde_json::Value,
) -> serde_json::Value {
    if let (Some(base_obj), Some(overlay_obj)) = (base.as_object_mut(), overlay.as_object()) {
        for (key, value) in overlay_obj {
            base_obj.insert(key.clone(), value.clone());
        }
        base
    } else {
        overlay
    }
}

fn marketplace_target(arguments: &serde_json::Value) -> Result<String> {
    action_text_arg(arguments, "id")
        .or_else(|| action_text_arg(arguments, "name"))
        .or_else(|| action_text_arg(arguments, "query"))
        .ok_or_else(|| {
            anyhow::anyhow!("Skill marketplace operation requires a marketplace id or name.")
        })
}

fn resolve_marketplace<'a>(
    marketplaces: &'a [crate::core::skill_marketplaces::SkillMarketplace],
    target: &str,
) -> Result<&'a crate::core::skill_marketplaces::SkillMarketplace> {
    let target = target.trim();
    let target_lower = target.to_ascii_lowercase();
    marketplaces
        .iter()
        .find(|marketplace| {
            marketplace.id == target
                || marketplace.id.to_ascii_lowercase() == target_lower
                || marketplace.name.to_ascii_lowercase() == target_lower
        })
        .ok_or_else(|| anyhow::anyhow!("Skill marketplace '{}' was not found.", target))
}

fn durable_orchestration_action_result(
    action_name: &str,
    durable_object_label: &str,
    handler_result: Option<String>,
    is_structured_completion: impl Fn(&str) -> bool,
) -> String {
    match handler_result {
        Some(result) if is_structured_completion(result.trim_start()) => result,
        Some(result) => {
            let detail = result.trim();
            render_tool_completion_marker_with_data(
                action_name,
                "needs_input",
                if detail.is_empty() {
                    "The durable action handler did not return a committed durable result. No durable object was created."
                } else {
                    detail
                },
                serde_json::json!({
                    "success": false,
                    "durable_commit": false,
                    "durable_object": durable_object_label,
                    "reason": "durable_handler_returned_non_commit_result",
                }),
            )
        }
        None => render_tool_completion_marker_with_data(
            action_name,
            "failed",
            &format!(
                "{} runtime validation passed, but AgentArk did not return a durable commit result. No durable object was created.",
                durable_object_label
            ),
            serde_json::json!({
                "success": false,
                "durable_commit": false,
                "durable_object": durable_object_label,
                "reason": "missing_durable_handler_commit",
            }),
        ),
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
            activity_label: None,
        }
    }

    fn background_session_with_activity(
        status: crate::core::background_session::BackgroundSessionStatus,
        last_activity_at: chrono::DateTime<chrono::Utc>,
    ) -> crate::core::background_session::BackgroundSession {
        crate::core::background_session::BackgroundSession {
            id: "background-session-1".to_string(),
            title: "Background work".to_string(),
            objective: "Do background work".to_string(),
            status,
            summary: None,
            current_focus: None,
            waiting_on: None,
            next_expected_action: None,
            working_memory: None,
            last_error: None,
            preferred_delivery_channel: None,
            channel: None,
            conversation_id: None,
            project_id: None,
            linked_task_ids: Vec::new(),
            linked_watcher_ids: Vec::new(),
            policy: crate::core::background_session::BackgroundSessionPolicy::default(),
            created_at: last_activity_at,
            updated_at: last_activity_at,
            last_activity_at,
            last_consolidated_at: None,
            events: Vec::new(),
        }
    }

    #[test]
    fn stale_active_background_sessions_do_not_block_gepa_idle() {
        let now = chrono::Utc::now();
        let quiet_window_seconds = 300;

        assert!(background_session_blocks_gepa_idle(
            &background_session_with_activity(
                crate::core::background_session::BackgroundSessionStatus::Active,
                now - chrono::Duration::seconds(30),
            ),
            now,
            quiet_window_seconds,
        ));
        assert!(!background_session_blocks_gepa_idle(
            &background_session_with_activity(
                crate::core::background_session::BackgroundSessionStatus::Active,
                now - chrono::Duration::seconds(900),
            ),
            now,
            quiet_window_seconds,
        ));
        assert!(!background_session_blocks_gepa_idle(
            &background_session_with_activity(
                crate::core::background_session::BackgroundSessionStatus::Waiting,
                now,
            ),
            now,
            quiet_window_seconds,
        ));
    }

    #[test]
    fn gepa_idle_worker_guard_clears_active_flag_on_drop() {
        GEPA_IDLE_WORKER_ACTIVE.store(true, Ordering::Release);
        {
            let _guard = GepaIdleWorkerActiveGuard;
        }
        assert!(!GEPA_IDLE_WORKER_ACTIVE.load(Ordering::Acquire));
    }

    #[test]
    fn gepa_job_value_failed_treats_blocked_as_unsuccessful() {
        let blocked = json!({
            "status": "blocked",
            "mode": "gepa_run",
            "error": "budget gate prevented optimizer execution"
        });

        assert!(gepa_job_value_failed(&blocked));
    }

    #[test]
    fn gepa_job_failure_retry_policy_does_not_repeat_bounded_terminal_failures() {
        let timeout = json!({
            "status": "timed_out",
            "timeout_seconds": 600
        });
        let blocked = json!({
            "status": "blocked",
            "error": "budget gate prevented optimizer execution"
        });
        let failed = json!({
            "status": "failed",
            "error": "optimizer process exited unsuccessfully"
        });
        let format_failure = json!({
            "status": "failed",
            "error": "Expected to find output fields in the LM response"
        });

        assert!(!gepa_job_failure_retryable(&timeout));
        assert!(!gepa_job_failure_retryable(&blocked));
        assert!(!gepa_job_failure_retryable(&failed));
        assert!(!gepa_job_failure_retryable(&format_failure));
    }

    #[test]
    fn gepa_prompt_optimization_metadata_attaches_opportunity_context() {
        let metadata = gepa_prompt_optimization_metadata(
            " prompt-opt-section-custom ",
            Some(json!({
                "section": "custom",
                "holdout_cases": [
                    { "trace_id": "trace-1", "outcome": "slow" }
                ],
                "opportunity_score": 7.25
            })),
        );

        assert_eq!(
            metadata.get("proposal_id").and_then(|value| value.as_str()),
            Some("prompt-opt-section-custom")
        );
        assert_eq!(
            metadata
                .get("opportunity_context")
                .and_then(|value| value.get("section"))
                .and_then(|value| value.as_str()),
            Some("custom")
        );
        assert!(metadata
            .get("opportunity_context")
            .and_then(|value| value.get("holdout_cases"))
            .and_then(|value| value.as_array())
            .is_some_and(|items| items.len() == 1));
    }

    #[test]
    fn gepa_prompt_optimization_promotion_uses_opportunity_confidence_target() {
        let default_settings = gepa_prompt_optimization_promotion_settings(None);
        assert_eq!(default_settings.canary_min_samples_per_version, 25);

        let profiled_settings = gepa_prompt_optimization_promotion_settings(Some(&json!({
            "confidence": {
                "confidence_sample_target": 64
            }
        })));

        assert_eq!(profiled_settings.canary_min_samples_per_version, 64);
    }

    #[test]
    fn tool_call_results_response_omits_pre_call_model_content() {
        let response = tool_call_results_response(vec!["structured tool result".to_string()]);

        assert_eq!(response, "structured tool result");
        assert!(!response.contains("Let me"));
    }

    #[test]
    fn ok_envelope_reclassifies_structured_failure_result() {
        let raw_result = json!({
            "output": "tool stderr explains the failure",
            "error": null,
            "exit_code": 1
        })
        .to_string();
        let envelope = tool_result_envelope_ok_string("runner", &raw_result);
        let parsed: serde_json::Value =
            serde_json::from_str(&envelope).expect("envelope should be json");

        assert_eq!(
            parsed.get("status").and_then(|value| value.as_str()),
            Some("error")
        );
        assert_eq!(
            parsed.get("exit_code").and_then(|value| value.as_i64()),
            Some(1)
        );
        assert!(parsed
            .get("message")
            .and_then(|value| value.as_str())
            .is_some_and(|message| message.contains("tool stderr explains the failure")));
        assert_eq!(
            super::super::tool_responses::tool_result_completion_success(&envelope),
            Some(false)
        );
    }

    #[test]
    fn browser_auto_checkpoint_kind_uses_structured_session_fields() {
        fn view(
            question: Option<&str>,
            reason: Option<&str>,
            summary: Option<&str>,
        ) -> crate::core::browser_session::BrowserSessionView {
            crate::core::browser_session::BrowserSessionView {
                id: "session-1".to_string(),
                conversation_id: None,
                task_description: "demo".to_string(),
                profile_id: None,
                profile_name: None,
                status: "internal-status-name-does-not-drive-kind".to_string(),
                question: question.map(str::to_string),
                summary: summary.map(str::to_string),
                reason: reason.map(str::to_string),
                created_at: "2026-01-01T00:00:00Z".to_string(),
                updated_at: "2026-01-01T00:00:00Z".to_string(),
                page_url: None,
                page_title: None,
                download_dir: None,
                downloads: Vec::new(),
                last_page: None,
                live_view_enabled: false,
                live_view_port: None,
                live_view_path: None,
                can_claim: false,
                can_release: false,
                can_complete: false,
            }
        }

        assert_eq!(
            browser_auto_checkpoint_kind(&view(Some("Continue?"), None, None)),
            BrowserAutoCheckpointKind::NeedsInput
        );
        assert_eq!(
            browser_auto_checkpoint_kind(&view(None, Some("Failed"), None)),
            BrowserAutoCheckpointKind::Failed
        );
        assert_eq!(
            browser_auto_checkpoint_kind(&view(None, None, Some("Done"))),
            BrowserAutoCheckpointKind::Completed
        );
        assert_eq!(
            browser_auto_checkpoint_kind(&view(None, None, None)),
            BrowserAutoCheckpointKind::Running
        );
    }

    #[test]
    fn work_reference_matching_is_normalized_for_resource_followups() {
        let stored = "8f0ed937-0063-44d7-adf4-68ac854b4033\nReminder: scheduled for today at 5:45 PM IST\nnotify telegram";

        assert!(work_text_matches_reference(
            stored,
            "reminder scheduled for today at 5:45 pm ist"
        ));
        assert!(work_text_matches_reference(
            stored,
            "8f0ed937-0063-44d7-adf4-68ac854b4033"
        ));
        assert!(!work_text_matches_reference(stored, "zz"));
        assert!(!work_text_matches_reference(
            stored,
            "unrelated background work"
        ));
    }

    #[test]
    fn skill_chat_review_gate_blocks_policy_blocked_skills_with_score() {
        let preview = json!({
            "status": "blocked",
            "name": "risky-skill",
            "security": {
                "blocked": true,
                "risk_band": "blocked",
                "risk_score_10": 9.2,
                "warnings": ["exfiltration risk"],
                "findings": [{"id": "network-write"}],
                "matched_rules": [{"id": "skill.network.write"}],
                "review_summary": "Attempts external writes."
            }
        });

        let gate = skill_chat_review_gate_result(&preview, "import", false)
            .expect("blocked review should stop the save");

        assert_eq!(
            gate.get("status").and_then(|value| value.as_str()),
            Some("blocked")
        );
        assert_eq!(
            gate.pointer("/confirmation/required")
                .and_then(|value| value.as_bool()),
            Some(false)
        );
        assert!(gate
            .get("message")
            .and_then(|value| value.as_str())
            .is_some_and(|message| message.contains("9.2/10")));
    }

    #[test]
    fn skill_chat_review_gate_requires_confirmation_for_non_blocking_risk() {
        let preview = json!({
            "status": "preview",
            "name": "network-helper",
            "security": {
                "blocked": false,
                "risk_band": "review",
                "risk_score_10": 6.4,
                "warnings": ["Uses network access"],
                "findings": [{"id": "network"}],
                "matched_rules": [{"id": "skill.network"}],
                "review_summary": "Network-capable skill."
            }
        });

        let gate = skill_chat_review_gate_result(&preview, "update", false)
            .expect("warning review should ask for confirmation");

        assert_eq!(
            gate.get("status").and_then(|value| value.as_str()),
            Some("needs_input")
        );
        assert_eq!(
            gate.pointer("/confirmation/retry_with/security_confirmed")
                .and_then(|value| value.as_bool()),
            Some(true)
        );
        assert_eq!(
            gate.pointer("/confirmation/retry_with/operation")
                .and_then(|value| value.as_str()),
            Some("update")
        );
        assert!(gate
            .get("message")
            .and_then(|value| value.as_str())
            .is_some_and(|message| message.contains("6.4/10")));
    }

    #[test]
    fn skill_chat_review_gate_allows_confirmed_non_blocking_risk() {
        let preview = json!({
            "status": "preview",
            "name": "network-helper",
            "security": {
                "blocked": false,
                "risk_band": "review",
                "risk_score_10": 6.4,
                "warnings": ["Uses network access"]
            }
        });

        assert!(skill_chat_review_gate_result(&preview, "import", true).is_none());
    }

    #[test]
    fn durable_orchestration_result_fails_when_handler_does_not_commit() {
        let result = durable_orchestration_action_result("watch", "Watcher", None, |value| {
            crate::runtime::parse_watch_completion(value).is_some()
        });
        let completion =
            crate::runtime::parse_watch_completion(&result).expect("watch marker should parse");

        assert_eq!(completion.status, "failed");
        assert!(completion
            .detail
            .as_deref()
            .unwrap_or_default()
            .contains("No durable object was created"));
        assert_eq!(
            super::super::tool_responses::tool_result_completion_success(&result),
            Some(false)
        );
    }

    #[test]
    fn durable_orchestration_plain_handler_message_is_not_success() {
        let result = durable_orchestration_action_result(
            "schedule_task",
            "Scheduled task",
            Some("Need a schedule before I can create this task.".to_string()),
            |value| crate::runtime::parse_schedule_task_completion(value).is_some(),
        );
        let completion = crate::runtime::parse_schedule_task_completion(&result)
            .expect("schedule marker should parse");

        assert_eq!(completion.status, "needs_input");
        assert_eq!(
            super::super::tool_responses::tool_result_completion_success(&result),
            Some(false)
        );
    }

    #[test]
    fn durable_orchestration_structured_commit_result_is_preserved() {
        let committed = render_tool_completion_marker_with_data(
            "watch",
            "completed",
            "Watcher created.",
            json!({"watcher_id": "w1"}),
        );
        let result = durable_orchestration_action_result(
            "watch",
            "Watcher",
            Some(committed.clone()),
            |value| crate::runtime::parse_watch_completion(value).is_some(),
        );

        assert_eq!(result, committed);
        assert_eq!(
            super::super::tool_responses::tool_result_completion_success(&result),
            Some(true)
        );
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
        assert_eq!(presented.reason, "outside_allowed_roots");
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
    fn service_manage_normalization_maps_typed_schema_to_existing_app_runtime() {
        let normalized = Agent::normalize_service_manage_deploy_arguments(&json!({
            "operation": "create",
            "service_id": "demo",
            "name": "Demo Service",
            "kind": "process",
            "source_dir": "workspace/demo",
            "source_paths": ["server.js"],
            "start_command": "node server.js",
            "environment": {"BASE_URL": "https://example.com"},
            "public": true
        }));

        assert_eq!(
            normalized.get("app_id").and_then(|v| v.as_str()),
            Some("demo")
        );
        assert_eq!(
            normalized.get("title").and_then(|v| v.as_str()),
            Some("Demo Service")
        );
        assert_eq!(
            normalized.get("entry_command").and_then(|v| v.as_str()),
            Some("node server.js")
        );
        assert_eq!(
            normalized.get("config").and_then(|v| v.get("BASE_URL")),
            Some(&json!("https://example.com"))
        );
        assert_eq!(
            normalized.get("expose_public").and_then(|v| v.as_bool()),
            Some(true)
        );
        assert_eq!(
            normalized.get("runtime_required").and_then(|v| v.as_bool()),
            Some(true)
        );
        assert!(normalized.get("operation").is_none());
        assert!(normalized.get("service_id").is_none());
    }

    #[test]
    fn service_manage_patch_uses_patch_mode_without_user_phrase_checks() {
        let normalized = Agent::normalize_service_manage_deploy_arguments(&json!({
            "operation": "patch",
            "service_id": "demo",
            "file_patches": [{"path": "index.html", "patch": "@@\n-old\n+new\n"}]
        }));

        assert_eq!(
            normalized.get("mode").and_then(|v| v.as_str()),
            Some("patch")
        );
        assert_eq!(
            normalized.get("app_id").and_then(|v| v.as_str()),
            Some("demo")
        );
    }

    #[test]
    fn deploy_create_reuses_recent_app_without_explicit_duplicate_signal() {
        assert!(
            Agent::deploy_create_should_reuse_recent_app(
                "create",
                &json!({ "files": { "index.html": "<html></html>" } }),
                true
            ),
            "create without an explicit duplicate signal should target the recent app"
        );
        assert!(
            !Agent::deploy_create_should_reuse_recent_app(
                "create",
                &json!({
                    "allow_duplicate": true,
                    "files": { "index.html": "<html></html>" }
                }),
                true
            ),
            "explicit duplicate permission should allow a separate new app"
        );
        assert!(
            !Agent::deploy_create_should_reuse_recent_app(
                "update",
                &json!({ "files": { "index.html": "<html></html>" } }),
                true
            ),
            "update operations should resolve an existing target instead of being treated as new app creation"
        );
    }

    #[test]
    fn app_deploy_quality_contract_prefers_structured_deploy_acceptance() {
        let contract = Agent::app_deploy_quality_contract_from_inputs(
            &json!({
                "request_context": "Build an inventory manager with durable server storage and complete item editing.",
                "acceptance_criteria": [
                    "Items can be created.",
                    "Items can be edited.",
                    "Items can be deleted."
                ]
            }),
            &[
                ("user", "older app request"),
                ("assistant", "working on it"),
                ("user", "latest unrelated follow-up"),
            ],
        );

        assert_eq!(
            contract.request_context,
            "Build an inventory manager with durable server storage and complete item editing."
        );
        assert_eq!(
            contract.sub_goals,
            vec![
                "Items can be created.".to_string(),
                "Items can be edited.".to_string(),
                "Items can be deleted.".to_string()
            ]
        );
    }

    #[test]
    fn app_deploy_quality_contract_falls_back_to_recent_user_intent() {
        let contract = Agent::app_deploy_quality_contract_from_inputs(
            &json!({
                "title": "Generated tool",
                "files": { "index.html": "<!doctype html><html><body>Tool</body></html>" }
            }),
            &[
                (
                    "user",
                    "Build a planning tool with create, update, delete, and persistent records.",
                ),
                ("assistant", "I will build it."),
            ],
        );

        assert_eq!(
            contract.request_context,
            "Build a planning tool with create, update, delete, and persistent records."
        );
        assert_eq!(
            contract.sub_goals,
            vec![
                "Build a planning tool with create, update, delete, and persistent records."
                    .to_string()
            ]
        );
    }

    #[test]
    fn blocking_runtime_diagnostics_are_generic_validation_failures() {
        let content = crate::integrations::browser::PageContent {
            title: "Demo".to_string(),
            url: "http://localhost/apps/demo/".to_string(),
            body_text: "Loaded".to_string(),
            elements: vec![],
            diagnostics: vec![crate::integrations::browser::PageDiagnostic {
                time: "2026-05-28T00:00:00Z".to_string(),
                kind: "response".to_string(),
                severity: "error".to_string(),
                message: "HTTP 502".to_string(),
                url: "http://localhost/apps/demo/__agentark/http/fetch".to_string(),
                resource_type: "fetch".to_string(),
            }],
            downloads: vec![],
            download_dir: None,
        };

        let issue = Agent::first_blocking_rendered_app_diagnostic(&content)
            .expect("error diagnostics should block app validation");
        assert!(
            issue.contains("response"),
            "diagnostic summary should name the structured failing diagnostic kind"
        );

        let warning_only = crate::integrations::browser::PageContent {
            diagnostics: vec![crate::integrations::browser::PageDiagnostic {
                time: "2026-05-28T00:00:00Z".to_string(),
                severity: "warning".to_string(),
                kind: "response".to_string(),
                message: "HTTP 404".to_string(),
                url: "http://localhost/apps/demo/missing.png".to_string(),
                resource_type: "image".to_string(),
            }],
            ..content
        };
        assert!(
            Agent::first_blocking_rendered_app_diagnostic(&warning_only).is_none(),
            "ordinary non-error response warnings should remain advisory"
        );
    }

    #[test]
    fn passive_host_resource_errors_do_not_block_rendered_app_validation() {
        let content = crate::integrations::browser::PageContent {
            title: "Demo".to_string(),
            url: "http://localhost:8990/apps/demo/".to_string(),
            body_text: "Loaded dashboard".to_string(),
            elements: vec![],
            diagnostics: vec![
                crate::integrations::browser::PageDiagnostic {
                    time: "2026-06-03T00:00:00Z".to_string(),
                    kind: "response".to_string(),
                    severity: "error".to_string(),
                    message: "HTTP 401".to_string(),
                    url: "http://localhost:8990/favicon.ico".to_string(),
                    resource_type: "image".to_string(),
                },
                crate::integrations::browser::PageDiagnostic {
                    time: "2026-06-03T00:00:00Z".to_string(),
                    kind: "response".to_string(),
                    severity: "error".to_string(),
                    message: "HTTP 401".to_string(),
                    url: "http://localhost:8990/fonts/ui.woff2".to_string(),
                    resource_type: "font".to_string(),
                },
            ],
            downloads: vec![],
            download_dir: None,
        };

        assert!(
            Agent::first_blocking_rendered_app_diagnostic(&content).is_none(),
            "host chrome/passive asset failures outside the rendered app scope should not make a loaded app look broken"
        );
    }

    #[test]
    fn app_scoped_fetch_errors_still_block_rendered_app_validation() {
        let content = crate::integrations::browser::PageContent {
            title: "Demo".to_string(),
            url: "http://localhost:8990/apps/demo/".to_string(),
            body_text: "Loaded dashboard".to_string(),
            elements: vec![],
            diagnostics: vec![crate::integrations::browser::PageDiagnostic {
                time: "2026-06-03T00:00:00Z".to_string(),
                kind: "response".to_string(),
                severity: "error".to_string(),
                message: "HTTP 500".to_string(),
                url: "http://localhost:8990/apps/demo/api/transactions".to_string(),
                resource_type: "fetch".to_string(),
            }],
            downloads: vec![],
            download_dir: None,
        };

        assert!(
            Agent::first_blocking_rendered_app_diagnostic(&content).is_some(),
            "actual app-owned fetch failures should still surface as validation blockers"
        );
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

    #[test]
    fn browser_auto_task_description_uses_explicit_task_without_url_prefix() {
        let task = browser_auto_task_description(
            Some("https://www.wikipedia.org"),
            None,
            Some(
                "Open https://www.wikipedia.org, search for OpenAI, then stop on the article"
                    .to_string(),
            ),
            None,
        );

        assert_eq!(
            task,
            "Open https://www.wikipedia.org, search for OpenAI, then stop on the article"
        );
    }

    #[test]
    fn browser_auto_task_description_falls_back_to_url_when_task_missing() {
        let task = browser_auto_task_description(Some("https://example.com"), None, None, None);

        assert_eq!(task, "Open https://example.com");
    }

    #[test]
    fn browser_auto_task_description_preserves_explicit_expectation() {
        let task = browser_auto_task_description(
            Some("https://www.wikipedia.org"),
            None,
            Some("Open Wikipedia and search for OpenAI.".to_string()),
            Some(
                "When the OpenAI article is visible, stop and ask: \"Should I inspect the History section or the Products section?\""
                    .to_string(),
            ),
        );

        assert_eq!(
            task,
            "Open Wikipedia and search for OpenAI.\n\nExpected checkpoint: When the OpenAI article is visible, stop and ask: \"Should I inspect the History section or the Products section?\""
        );
    }

    #[test]
    fn browser_auto_task_description_uses_recent_artifact_url_when_url_missing() {
        let artifact = ConversationArtifactContext {
            artifact_type: "app".to_string(),
            artifact_id: "demo-app".to_string(),
            title: "Demo App".to_string(),
            summary: "Recently deployed app".to_string(),
            url: "/apps/demo-app/".to_string(),
            related_actions: vec!["service_manage".to_string()],
            updated_at: "2026-06-02T00:00:00Z".to_string(),
        };

        let task = browser_auto_task_description(
            None,
            Some(&artifact),
            Some("Open the page and take a full-page snapshot.".to_string()),
            None,
        );

        assert!(
            task.contains("/apps/demo-app/"),
            "the browser task should include the structured artifact URL"
        );
        assert!(
            task.contains("Open the page and take a full-page snapshot."),
            "the browser task should preserve the model's requested browser work"
        );
    }

    #[test]
    fn app_quality_summary_does_not_repair_for_unavailable_advisory_judge() {
        let contract = AppDeployQualityContract {
            request_context: "Build a usable dashboard.".to_string(),
            sub_goals: vec!["Build a usable dashboard.".to_string()],
        };
        let report = json!({
            "status": "concerns",
            "browser_ok": true,
            "judge": {
                "available": false,
                "passed": false,
                "summary": "The advisory quality judge was unavailable."
            },
            "judge_concerns": ["The advisory quality judge was unavailable."]
        });

        let summary = Agent::app_quality_tool_summary(Some(&report), &contract);

        assert_eq!(
            summary["needs_repair"], false,
            "an unavailable advisory judge must not convert a served app into a repair loop"
        );
        assert_eq!(summary["judge_passed"], serde_json::Value::Null);
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
