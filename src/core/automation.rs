use anyhow::Result;
use chrono::{DateTime, Duration, Utc};
use regex::{Regex, RegexBuilder};
use serde::{Deserialize, Serialize};
use serde_json::Value;

use crate::actions::{ActionAuthorizationContext, ActionCallerPrincipal, ActionExecutionSurface};
const AUTOMATION_RUNS_LIMIT: usize = 600;
const AUTOMATION_MAX_ATTEMPTS_CAP: u32 = 12;
const AUTOMATION_MAX_STALL_TIMEOUT_SECS: u64 = 365 * 24 * 60 * 60;
const AUTOMATION_REGEX_SIZE_LIMIT: usize = 256 * 1024;
const AUTOMATION_REGEX_DFA_SIZE_LIMIT: usize = 256 * 1024;

#[derive(Debug, Clone, Serialize, Deserialize, Default)]
pub struct AutomationOriginContext {
    pub channel: Option<String>,
    pub conversation_id: Option<String>,
    pub project_id: Option<String>,
    pub source: Option<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize, Default)]
pub struct AutomationAuthorizationContext {
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub principal: Option<ActionCallerPrincipal>,
    #[serde(default)]
    pub direct_user_intent: bool,
    #[serde(default)]
    pub current_turn_is_explicit_approval: bool,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub agent_access_scope: Option<crate::core::swarm::AgentAccessScope>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub capability_context_id: Option<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq, Default)]
#[serde(rename_all = "snake_case")]
pub enum AutomationValidationMode {
    #[default]
    None,
    NonEmptyResult,
    StructuredSuccess,
    ContainsText,
    RegexMatch,
    JsonFieldExists,
    JsonFieldEquals,
    JsonArrayNonEmpty,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct AutomationValidation {
    pub mode: AutomationValidationMode,
    #[serde(default)]
    pub text: Option<String>,
    #[serde(default)]
    pub field_path: Option<String>,
    #[serde(default)]
    pub expected: Option<Value>,
    #[serde(default)]
    pub pattern: Option<String>,
}

impl Default for AutomationValidation {
    fn default() -> Self {
        Self {
            mode: AutomationValidationMode::None,
            text: None,
            field_path: None,
            expected: None,
            pattern: None,
        }
    }
}

impl AutomationValidation {
    pub fn is_unset(&self) -> bool {
        self.mode == AutomationValidationMode::None
            && self.text.is_none()
            && self.field_path.is_none()
            && self.expected.is_none()
            && self.pattern.is_none()
    }

    pub fn normalized(&self) -> Self {
        Self {
            mode: self.mode.clone(),
            text: normalize_optional_text(self.text.as_deref()),
            field_path: normalize_optional_text(self.field_path.as_deref()),
            expected: self.expected.clone(),
            pattern: normalize_optional_text(self.pattern.as_deref()),
        }
    }
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct AutomationExecutionPolicy {
    pub max_attempts: u32,
    pub stall_timeout_secs: u64,
    pub retry_backoff_secs: u64,
    pub validation: AutomationValidation,
}

impl Default for AutomationExecutionPolicy {
    fn default() -> Self {
        Self {
            max_attempts: 3,
            stall_timeout_secs: 0,
            retry_backoff_secs: 60,
            validation: AutomationValidation::default(),
        }
    }
}

impl AutomationExecutionPolicy {
    pub fn normalized(&self) -> Self {
        Self {
            max_attempts: self.max_attempts.clamp(1, AUTOMATION_MAX_ATTEMPTS_CAP),
            stall_timeout_secs: if self.stall_timeout_secs == 0 {
                0
            } else {
                self.stall_timeout_secs
                    .clamp(30, AUTOMATION_MAX_STALL_TIMEOUT_SECS)
            },
            retry_backoff_secs: self.retry_backoff_secs.clamp(10, 24 * 60 * 60),
            validation: self.validation.normalized(),
        }
    }
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "snake_case")]
pub enum AutomationRunStatus {
    Running,
    Succeeded,
    Failed,
    Retrying,
    TimedOut,
    Triggered,
}

#[derive(Debug, Clone, Serialize, Deserialize, Default)]
pub struct AutomationCritique {
    pub summary: String,
    pub retryable: bool,
    pub validation_passed: bool,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct AutomationRunRecord {
    pub id: String,
    pub automation_id: String,
    pub automation_kind: String,
    pub title: String,
    pub action: String,
    pub trigger: String,
    pub status: AutomationRunStatus,
    pub attempt: u32,
    pub started_at: String,
    pub completed_at: Option<String>,
    pub duration_ms: Option<u64>,
    pub origin: AutomationOriginContext,
    pub policy: AutomationExecutionPolicy,
    pub critique: AutomationCritique,
    pub output_preview: Option<String>,
    pub error: Option<String>,
    pub next_retry_at: Option<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize, Default)]
pub struct AutomationSupervisorState {
    pub automation_id: String,
    pub automation_kind: String,
    pub title: String,
    pub action: String,
    pub status: String,
    pub attempt_count: u32,
    pub consecutive_failures: u32,
    pub last_run_id: Option<String>,
    pub last_run_at: Option<String>,
    pub last_success_at: Option<String>,
    pub last_error: Option<String>,
    pub next_retry_at: Option<String>,
    pub stalled_count: u32,
    #[serde(default)]
    pub origin: AutomationOriginContext,
    #[serde(default)]
    pub created_at: Option<String>,
}

pub async fn list_runs(
    storage: &crate::storage::Storage,
    limit: usize,
) -> Result<Vec<AutomationRunRecord>> {
    let mut runs = storage.list_automation_runs(limit).await?;
    runs.sort_by(|a, b| b.started_at.cmp(&a.started_at));
    if runs.len() > limit {
        runs.truncate(limit);
    }
    Ok(runs)
}

pub async fn append_run(storage: &crate::storage::Storage, run: AutomationRunRecord) -> Result<()> {
    storage
        .append_automation_run(&run, AUTOMATION_RUNS_LIMIT)
        .await
}

pub async fn list_supervisor_states(
    storage: &crate::storage::Storage,
) -> Result<Vec<AutomationSupervisorState>> {
    storage.list_automation_supervisor_states().await
}

pub async fn load_supervisor_state(
    storage: &crate::storage::Storage,
    automation_id: &str,
) -> Result<Option<AutomationSupervisorState>> {
    storage
        .load_automation_supervisor_state(automation_id)
        .await
}

pub async fn upsert_supervisor_state(
    storage: &crate::storage::Storage,
    state: AutomationSupervisorState,
) -> Result<()> {
    storage.upsert_automation_supervisor_state(&state).await
}

pub async fn delete_supervisor_state(
    storage: &crate::storage::Storage,
    automation_id: &str,
) -> Result<bool> {
    storage
        .delete_automation_supervisor_state(automation_id)
        .await
}

pub fn inject_context(
    arguments: &Value,
    origin: AutomationOriginContext,
    default_policy: AutomationExecutionPolicy,
) -> Value {
    let mut next = arguments.clone();
    if !next.is_object() {
        next = serde_json::json!({});
    }
    let Some(root) = next.as_object_mut() else {
        return next;
    };

    let existing_meta = root
        .remove("_automation")
        .and_then(|value| value.as_object().cloned())
        .unwrap_or_default();
    let mut meta = serde_json::Map::from_iter(existing_meta);
    if !meta.contains_key("origin") {
        meta.insert(
            "origin".to_string(),
            serde_json::to_value(origin).unwrap_or(Value::Null),
        );
    }
    if !meta.contains_key("policy") {
        meta.insert(
            "policy".to_string(),
            serde_json::to_value(default_policy.normalized()).unwrap_or(Value::Null),
        );
    }
    root.insert("_automation".to_string(), Value::Object(meta));
    next
}

fn persistable_authorization_context(
    authorization: Option<&ActionAuthorizationContext>,
) -> Option<AutomationAuthorizationContext> {
    let authorization = authorization?;
    let principal = authorization.principal.as_ref()?.clone();
    if !principal.trusted || !authorization.direct_user_intent {
        return None;
    }
    Some(AutomationAuthorizationContext {
        principal: Some(principal),
        direct_user_intent: true,
        current_turn_is_explicit_approval: authorization.current_turn_is_explicit_approval,
        agent_access_scope: authorization.agent_access_scope.clone(),
        capability_context_id: authorization.capability_context_id.clone(),
    })
}

pub fn inject_authorization_context(
    arguments: &Value,
    authorization: Option<&ActionAuthorizationContext>,
) -> Value {
    let Some(persisted) = persistable_authorization_context(authorization) else {
        return arguments.clone();
    };

    let mut next = arguments.clone();
    if !next.is_object() {
        next = serde_json::json!({});
    }
    let Some(root) = next.as_object_mut() else {
        return next;
    };

    let existing_meta = root
        .remove("_automation")
        .and_then(|value| value.as_object().cloned())
        .unwrap_or_default();
    let mut meta = serde_json::Map::from_iter(existing_meta);
    meta.insert(
        "authorization".to_string(),
        serde_json::to_value(persisted).unwrap_or(Value::Null),
    );
    root.insert("_automation".to_string(), Value::Object(meta));
    next
}

pub fn origin_from_arguments(arguments: &Value) -> AutomationOriginContext {
    arguments
        .get("_automation")
        .and_then(|value| value.get("origin"))
        .cloned()
        .and_then(|value| serde_json::from_value::<AutomationOriginContext>(value).ok())
        .unwrap_or_default()
}

pub fn authorization_from_arguments(arguments: &Value) -> Option<AutomationAuthorizationContext> {
    arguments
        .get("_automation")
        .and_then(|value| value.get("authorization"))
        .cloned()
        .and_then(|value| serde_json::from_value::<AutomationAuthorizationContext>(value).ok())
}

pub fn runtime_authorization_context_from_arguments(
    arguments: &Value,
    surface: ActionExecutionSurface,
) -> ActionAuthorizationContext {
    let persisted = authorization_from_arguments(arguments).unwrap_or_default();
    ActionAuthorizationContext {
        principal: persisted.principal,
        surface,
        direct_user_intent: persisted.direct_user_intent,
        current_turn_is_explicit_approval: persisted.current_turn_is_explicit_approval,
        agent_name: None,
        agent_access_scope: persisted.agent_access_scope,
        capability_context_id: persisted.capability_context_id,
    }
}

pub fn policy_from_arguments(
    arguments: &Value,
    fallback_validation: AutomationValidation,
) -> AutomationExecutionPolicy {
    let mut policy = arguments
        .get("_automation")
        .and_then(|value| value.get("policy"))
        .cloned()
        .and_then(|value| serde_json::from_value::<AutomationExecutionPolicy>(value).ok())
        .unwrap_or_default();
    if policy.validation.is_unset() && !fallback_validation.is_unset() {
        policy.validation = fallback_validation;
    }
    policy.normalized()
}

pub fn validation_from_request_argument(
    arguments: &Value,
    fallback_validation: AutomationValidation,
) -> AutomationValidation {
    arguments
        .get("validation")
        .cloned()
        .and_then(|value| serde_json::from_value::<AutomationValidation>(value).ok())
        .map(|value| value.normalized())
        .filter(|value| !value.is_unset())
        .unwrap_or_else(|| fallback_validation.normalized())
}

pub fn policy_from_request_argument(
    arguments: &Value,
    default_policy: AutomationExecutionPolicy,
) -> AutomationExecutionPolicy {
    let mut policy = arguments
        .get("automation_policy")
        .cloned()
        .and_then(|value| serde_json::from_value::<AutomationExecutionPolicy>(value).ok())
        .unwrap_or_else(|| default_policy.normalized());

    if let Some(max_attempts) = arguments
        .get("max_attempts")
        .and_then(|value| value.as_u64())
    {
        policy.max_attempts = max_attempts.clamp(1, AUTOMATION_MAX_ATTEMPTS_CAP as u64) as u32;
    }
    if let Some(stall_timeout_secs) = arguments
        .get("stall_timeout_secs")
        .and_then(|value| value.as_u64())
    {
        policy.stall_timeout_secs = stall_timeout_secs;
    }
    if let Some(retry_backoff_secs) = arguments
        .get("retry_backoff_secs")
        .and_then(|value| value.as_u64())
    {
        policy.retry_backoff_secs = retry_backoff_secs;
    }

    policy.validation =
        validation_from_request_argument(arguments, default_policy.validation.clone());
    policy.normalized()
}

pub fn increment_attempt(arguments: &mut Value, attempt: u32) {
    let Some(root) = arguments.as_object_mut() else {
        return;
    };
    let meta = root
        .entry("_automation".to_string())
        .or_insert_with(|| Value::Object(serde_json::Map::new()));
    let Some(meta_obj) = meta.as_object_mut() else {
        return;
    };
    meta_obj.insert("attempt".to_string(), serde_json::json!(attempt));
}

pub fn current_attempt(arguments: &Value) -> u32 {
    arguments
        .get("_automation")
        .and_then(|value| value.get("attempt"))
        .and_then(|value| value.as_u64())
        .map(|value| value.clamp(1, AUTOMATION_MAX_ATTEMPTS_CAP as u64) as u32)
        .unwrap_or(1)
}

pub fn compute_retry_at(
    now: DateTime<Utc>,
    policy: &AutomationExecutionPolicy,
    attempt: u32,
) -> DateTime<Utc> {
    let exponent = attempt.saturating_sub(1).min(8);
    let multiplier = 2u64.saturating_pow(exponent);
    let delay_secs = policy
        .retry_backoff_secs
        .saturating_mul(multiplier)
        .min(7 * 24 * 60 * 60);
    now + Duration::seconds(delay_secs as i64)
}

fn structured_success_state(value: &Value) -> Option<bool> {
    for path in [
        "success",
        "ok",
        "result.success",
        "result.ok",
        "_automation.success",
    ] {
        if let Some(flag) = json_value_at_path(value, path).and_then(|item| item.as_bool()) {
            return Some(flag);
        }
    }

    for path in ["status", "state", "result.status", "result.state"] {
        if let Some(status) = json_value_at_path(value, path).and_then(|item| item.as_str()) {
            return Some(matches!(
                status.trim().to_ascii_lowercase().as_str(),
                "ok" | "success" | "completed"
            ));
        }
    }

    None
}

fn parse_automation_output_json(output: &str) -> Option<Value> {
    let trimmed = output.trim();
    if let Some(payload) = trimmed.strip_prefix(crate::runtime::TOOL_COMPLETION_MARKER) {
        let marker_json = payload.lines().next().unwrap_or(payload).trim();
        return serde_json::from_str::<Value>(marker_json).ok();
    }
    serde_json::from_str::<Value>(trimmed).ok()
}

fn structured_retryable_state(value: &Value) -> Option<bool> {
    for path in [
        "retryable",
        "should_retry",
        "transient",
        "error.retryable",
        "_automation.retryable",
    ] {
        if let Some(flag) = json_value_at_path(value, path).and_then(|item| item.as_bool()) {
            return Some(flag);
        }
    }

    for path in ["retry_after_secs", "retry.after_secs"] {
        if json_value_at_path(value, path)
            .and_then(|item| item.as_u64())
            .filter(|value| *value > 0)
            .is_some()
        {
            return Some(true);
        }
    }

    if json_value_at_path(value, "next_retry_at").is_some() {
        return Some(true);
    }

    for path in ["status", "state", "error.code"] {
        if let Some(status) = json_value_at_path(value, path).and_then(|item| item.as_str()) {
            return Some(matches!(
                status.trim().to_ascii_lowercase().as_str(),
                "retrying" | "pending" | "queued" | "throttled" | "transient_error"
            ));
        }
    }

    None
}

pub fn validate_result(validation: &AutomationValidation, output: &str) -> bool {
    let trimmed = output.trim();
    let normalized = validation.normalized();
    let parsed_json = parse_automation_output_json(trimmed);
    match validation.mode {
        AutomationValidationMode::None => true,
        AutomationValidationMode::NonEmptyResult => {
            if parsed_json.is_some() {
                if let Some(value) = parsed_json.as_ref() {
                    if let Some(success) = structured_success_state(value) {
                        return success && !primary_result_text(trimmed).trim().is_empty();
                    }
                }
                !primary_result_text(trimmed).trim().is_empty()
            } else {
                !trimmed.is_empty()
            }
        }
        AutomationValidationMode::StructuredSuccess => {
            if trimmed.is_empty() {
                return false;
            }
            if let Some(value) = parsed_json.as_ref() {
                return structured_success_state(value).unwrap_or(false);
            }
            false
        }
        AutomationValidationMode::ContainsText => normalized
            .text
            .as_ref()
            .map(|text| {
                validation_target_text(&normalized, trimmed, parsed_json.as_ref())
                    .to_ascii_lowercase()
                    .contains(&text.to_ascii_lowercase())
            })
            .unwrap_or(false),
        AutomationValidationMode::RegexMatch => normalized
            .pattern
            .as_ref()
            .or(normalized.text.as_ref())
            .and_then(|pattern| {
                RegexBuilder::new(pattern)
                    .size_limit(AUTOMATION_REGEX_SIZE_LIMIT)
                    .dfa_size_limit(AUTOMATION_REGEX_DFA_SIZE_LIMIT)
                    .build()
                    .ok()
            })
            .map(|regex| {
                regex.is_match(&validation_target_text(
                    &normalized,
                    trimmed,
                    parsed_json.as_ref(),
                ))
            })
            .unwrap_or(false),
        AutomationValidationMode::JsonFieldExists => parsed_json
            .as_ref()
            .and_then(|value| {
                normalized
                    .field_path
                    .as_deref()
                    .and_then(|path| json_value_at_path(value, path))
            })
            .is_some(),
        AutomationValidationMode::JsonFieldEquals => parsed_json
            .as_ref()
            .and_then(|value| {
                normalized
                    .field_path
                    .as_deref()
                    .and_then(|path| json_value_at_path(value, path))
                    .map(|actual| {
                        if let Some(expected) = normalized.expected.as_ref() {
                            actual == expected
                        } else if let Some(expected_text) = normalized.text.as_ref() {
                            json_value_to_text(actual).eq_ignore_ascii_case(expected_text)
                        } else {
                            false
                        }
                    })
            })
            .unwrap_or(false),
        AutomationValidationMode::JsonArrayNonEmpty => parsed_json
            .as_ref()
            .and_then(|value| {
                let target = normalized
                    .field_path
                    .as_deref()
                    .and_then(|path| json_value_at_path(value, path))
                    .unwrap_or(value);
                target.as_array().map(|items| !items.is_empty())
            })
            .unwrap_or(false),
    }
}

pub fn critique_result(
    validation: &AutomationValidation,
    output: Option<&str>,
    error: Option<&str>,
) -> AutomationCritique {
    let validation_passed = output
        .map(|value| validate_result(validation, value))
        .unwrap_or(false);
    let normalized = validation.normalized();
    let error_text = error.unwrap_or_default().trim();
    let output_text = output.unwrap_or_default().trim();
    let error_json = parse_automation_output_json(error_text);
    let output_json = parse_automation_output_json(output_text);
    let retryable = error_json
        .as_ref()
        .and_then(structured_retryable_state)
        .or_else(|| output_json.as_ref().and_then(structured_retryable_state))
        .unwrap_or(false);
    let summary = if !error_text.is_empty() {
        format!("Execution failed: {}", truncate_text(error_text, 180))
    } else if !validation_passed {
        match validation.mode {
            AutomationValidationMode::None => "Execution completed without validation.".to_string(),
            AutomationValidationMode::NonEmptyResult => {
                "Execution finished but did not produce a usable result.".to_string()
            }
            AutomationValidationMode::StructuredSuccess => {
                "Execution finished but did not report a structured success state.".to_string()
            }
            AutomationValidationMode::ContainsText => format!(
                "Execution finished but did not contain the required text{}.",
                normalized
                    .text
                    .as_ref()
                    .map(|text| format!(" `{}`", truncate_text(text, 80)))
                    .unwrap_or_default()
            ),
            AutomationValidationMode::RegexMatch => format!(
                "Execution finished but did not match the required pattern{}.",
                normalized
                    .pattern
                    .as_ref()
                    .or(normalized.text.as_ref())
                    .map(|text| format!(" `{}`", truncate_text(text, 80)))
                    .unwrap_or_default()
            ),
            AutomationValidationMode::JsonFieldExists => format!(
                "Execution finished but the expected JSON field{} was missing.",
                normalized
                    .field_path
                    .as_ref()
                    .map(|path| format!(" `{}`", truncate_text(path, 80)))
                    .unwrap_or_default()
            ),
            AutomationValidationMode::JsonFieldEquals => format!(
                "Execution finished but JSON field{} did not match the expected value.",
                normalized
                    .field_path
                    .as_ref()
                    .map(|path| format!(" `{}`", truncate_text(path, 80)))
                    .unwrap_or_default()
            ),
            AutomationValidationMode::JsonArrayNonEmpty => format!(
                "Execution finished but JSON array{} was empty or missing.",
                normalized
                    .field_path
                    .as_ref()
                    .map(|path| format!(" `{}`", truncate_text(path, 80)))
                    .unwrap_or_default()
            ),
        }
    } else {
        "Execution and validation completed successfully.".to_string()
    };
    AutomationCritique {
        summary,
        retryable,
        validation_passed,
    }
}

fn normalize_optional_text(value: Option<&str>) -> Option<String> {
    value
        .map(str::trim)
        .filter(|value| !value.is_empty())
        .map(|value| value.to_string())
}

fn json_value_at_path<'a>(root: &'a Value, path: &str) -> Option<&'a Value> {
    let mut current = root;
    for segment in path
        .split('.')
        .map(str::trim)
        .filter(|segment| !segment.is_empty())
    {
        if let Ok(index) = segment.parse::<usize>() {
            current = current.as_array()?.get(index)?;
        } else {
            current = current.get(segment)?;
        }
    }
    Some(current)
}

fn json_value_to_text(value: &Value) -> String {
    match value {
        Value::Null => "null".to_string(),
        Value::Bool(flag) => flag.to_string(),
        Value::Number(num) => num.to_string(),
        Value::String(text) => text.clone(),
        other => serde_json::to_string(other).unwrap_or_default(),
    }
}

pub fn primary_result_text(raw_output: &str) -> String {
    let trimmed = raw_output.trim();
    let Some(value) = parse_automation_output_json(trimmed) else {
        return raw_output.to_string();
    };
    let Some(output) = json_value_at_path(&value, "output") else {
        return raw_output.to_string();
    };
    json_value_to_text(output)
}

fn validation_target_text(
    validation: &AutomationValidation,
    raw_output: &str,
    parsed_json: Option<&Value>,
) -> String {
    if let (Some(value), Some(path)) = (parsed_json, validation.field_path.as_deref()) {
        if let Some(target) = json_value_at_path(value, path) {
            return json_value_to_text(target);
        }
    }
    if parsed_json.is_some() {
        return primary_result_text(raw_output);
    }
    raw_output.to_string()
}

pub fn truncate_text(value: &str, max_chars: usize) -> String {
    if value.chars().count() <= max_chars {
        value.to_string()
    } else {
        format!("{}...", value.chars().take(max_chars).collect::<String>())
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn automation_authorization_context_preserves_runtime_scope_without_secrets() {
        let mut scope = crate::core::swarm::AgentAccessScope::default();
        scope.channel_ids.push("telegram".to_string());
        scope.approved_permission_ids.push("watcher".to_string());
        let authorization = ActionAuthorizationContext {
            principal: Some(ActionCallerPrincipal::local_admin("web")),
            surface: ActionExecutionSurface::Chat,
            direct_user_intent: true,
            current_turn_is_explicit_approval: true,
            agent_name: Some("Ops".to_string()),
            agent_access_scope: Some(scope.clone()),
            capability_context_id: Some("turn-123".to_string()),
        };

        let arguments = inject_authorization_context(&serde_json::json!({}), Some(&authorization));
        let restored = runtime_authorization_context_from_arguments(
            &arguments,
            ActionExecutionSurface::Background,
        );

        assert_eq!(restored.principal, authorization.principal);
        assert_eq!(restored.surface, ActionExecutionSurface::Background);
        assert!(restored.direct_user_intent);
        assert!(restored.current_turn_is_explicit_approval);
        assert_eq!(restored.agent_access_scope, Some(scope));
        assert_eq!(restored.capability_context_id.as_deref(), Some("turn-123"));
        assert_eq!(restored.agent_name, None);
    }

    #[test]
    fn automation_authorization_context_rejects_untrusted_persistence() {
        let authorization = ActionAuthorizationContext {
            principal: Some(ActionCallerPrincipal {
                user_id: "external".to_string(),
                role: "viewer".to_string(),
                auth_source: "webhook".to_string(),
                trusted: false,
            }),
            surface: ActionExecutionSurface::Api,
            direct_user_intent: true,
            current_turn_is_explicit_approval: false,
            agent_name: None,
            agent_access_scope: None,
            capability_context_id: Some("turn-456".to_string()),
        };

        let arguments = inject_authorization_context(&serde_json::json!({}), Some(&authorization));
        let restored = runtime_authorization_context_from_arguments(
            &arguments,
            ActionExecutionSurface::Background,
        );

        assert_eq!(restored.principal, None);
        assert!(!restored.direct_user_intent);
        assert_eq!(restored.capability_context_id, None);
    }

    #[test]
    fn structured_success_requires_structured_signal() {
        let validation = AutomationValidation {
            mode: AutomationValidationMode::StructuredSuccess,
            ..AutomationValidation::default()
        };

        assert!(validate_result(&validation, r#"{"success":true}"#));
        assert!(!validate_result(&validation, "completed successfully"));
    }

    #[test]
    fn critique_result_prefers_structured_retryability() {
        let validation = AutomationValidation {
            mode: AutomationValidationMode::StructuredSuccess,
            ..AutomationValidation::default()
        };

        let critique = critique_result(
            &validation,
            Some(r#"{"success":false,"retryable":true}"#),
            None,
        );
        assert!(critique.retryable);

        let critique = critique_result(&validation, None, Some("temporary network issue"));
        assert!(!critique.retryable);
    }

    #[test]
    fn non_empty_validation_uses_wrapped_output_field() {
        let validation = AutomationValidation {
            mode: AutomationValidationMode::NonEmptyResult,
            ..AutomationValidation::default()
        };

        assert!(!validate_result(
            &validation,
            r#"{"output":"","error":null,"exit_code":0}"#
        ));
        assert!(validate_result(
            &validation,
            r#"{"output":"{\"new_person\": false}","error":null,"exit_code":0}"#
        ));
    }

    #[test]
    fn non_empty_validation_rejects_structured_failure_status() {
        let validation = AutomationValidation {
            mode: AutomationValidationMode::NonEmptyResult,
            ..AutomationValidation::default()
        };

        assert!(!validate_result(
            &validation,
            r#"{"status":"failed","output":"I will retry this later"}"#
        ));
        assert!(!validate_result(
            &validation,
            r#"{"ok":false,"message":"request failed"}"#
        ));
    }

    #[test]
    fn non_empty_validation_rejects_tool_completion_failure_marker() {
        let validation = AutomationValidation {
            mode: AutomationValidationMode::NonEmptyResult,
            ..AutomationValidation::default()
        };
        let marker = format!(
            "{}{}",
            crate::runtime::TOOL_COMPLETION_MARKER,
            serde_json::json!({
                "tool": "notify_user",
                "status": "failed",
                "detail": "Telegram delivery failed"
            })
        );

        assert!(!validate_result(&validation, &marker));
    }

    #[test]
    fn structured_success_accepts_tool_completion_completed_marker() {
        let validation = AutomationValidation {
            mode: AutomationValidationMode::StructuredSuccess,
            ..AutomationValidation::default()
        };
        let marker = format!(
            "{}{}",
            crate::runtime::TOOL_COMPLETION_MARKER,
            serde_json::json!({
                "tool": "notify_user",
                "status": "completed",
                "detail": "Notification delivered"
            })
        );

        assert!(validate_result(&validation, &marker));
    }
}
