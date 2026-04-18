use std::collections::{HashMap, HashSet};
use std::path::Path;

use anyhow::Result;
use serde_json::{Value, json};
use sha2::{Digest, Sha256};

use crate::core::self_evolve::skill_evolution::{self, SkillMetricsSnapshot, SkillWindowDirection};
use crate::core::{ExecutionRun, ExecutionRunStatus, ToolAttempt};
use crate::storage::{
    KvLeaseGuard, Storage, experience_edge, experience_item, experience_run, learning_candidate,
    procedural_pattern,
};

pub const LEARNING_ENABLED_KEY: &str = "learning_enabled_v1";
pub const LEARNING_MODEL_SLOT_KEY: &str = "learning_model_slot_v1";
pub const LEARNING_QUEUE_CAP_KEY: &str = "learning_queue_cap_v1";
const LEARNING_CANDIDATE_GENERATION_LEASE_KEY: &str = "learning_candidate_generation_lease_v1";
const LEARNING_CANDIDATE_GENERATION_LEASE_TTL_SECS: i64 = 10 * 60;
const LEARNING_CANDIDATE_GENERATION_LEASE_HEARTBEAT_SECS: u64 = 60;
const CORRECTION_WINDOW_MINUTES: i64 = 30;
const DEFAULT_QUEUE_CAP: usize = 64;
pub(crate) const HEURISTIC_REFLECTION_ORIGIN: &str = "heuristic_reflection";
pub(crate) const HEURISTIC_REFLECTION_VERSION: &str = "heuristic-reflection-v1";

#[derive(Debug, Clone)]
pub(crate) struct ReflectedHeuristic {
    pub heuristic: String,
    pub polarity: String,
    pub confidence: f64,
    pub applicability: Option<String>,
}

#[derive(Debug, Clone)]
pub(crate) struct ReflectedHeuristicPersistOutcome {
    pub lesson_id: String,
    pub merged: bool,
}

fn safe_truncate(value: &str, max_chars: usize) -> String {
    if value.chars().count() <= max_chars {
        return value.to_string();
    }
    value.chars().take(max_chars).collect::<String>()
}

fn stable_id(prefix: &str, parts: &[&str]) -> String {
    let mut hasher = Sha256::new();
    hasher.update(prefix.as_bytes());
    for part in parts {
        hasher.update([0u8]);
        hasher.update(part.as_bytes());
    }
    let digest = hex::encode(hasher.finalize());
    format!("{}-{}", prefix, &digest[..24])
}

fn short_hash(parts: &[&str]) -> String {
    stable_id("h", parts)
        .rsplit('-')
        .next()
        .unwrap_or("candidate")
        .to_string()
}

fn scope_from_ids(project_id: Option<&str>, conversation_id: Option<&str>) -> &'static str {
    if conversation_id.is_some() {
        "conversation"
    } else if project_id.is_some() {
        "project"
    } else {
        "global"
    }
}

fn normalize_token(token: &str) -> Option<String> {
    let trimmed = token
        .trim_matches(|ch: char| !ch.is_ascii_alphanumeric())
        .to_ascii_lowercase();
    if trimmed.len() < 3 || trimmed.chars().all(|ch| ch.is_ascii_digit()) {
        return None;
    }
    Some(trimmed)
}

fn derive_intent_key(message: &str, task_type: &str) -> String {
    let mut seen = std::collections::HashSet::new();
    let mut tokens = message
        .split_whitespace()
        .filter_map(normalize_token)
        .filter(|token| seen.insert(token.clone()))
        .take(6)
        .collect::<Vec<_>>();
    if tokens.is_empty() {
        tokens.push("general".to_string());
    }
    format!("{}::{}", task_type, tokens.join("-"))
}

fn tool_sequence_digest(tool_attempts: &[ToolAttempt]) -> Option<String> {
    if tool_attempts.is_empty() {
        return None;
    }
    let sequence = tool_attempts
        .iter()
        .map(|attempt| attempt.tool_name.as_str())
        .collect::<Vec<_>>();
    Some(short_hash(&sequence))
}

fn tool_sequence_json(tool_attempts: &[ToolAttempt]) -> Value {
    Value::Array(
        tool_attempts
            .iter()
            .map(|attempt| {
                json!({
                    "tool_name": attempt.tool_name,
                    "status": attempt.status.as_str(),
                    "sequence_no": attempt.sequence_no,
                    "retryable": attempt.retryable,
                    "side_effect_level": attempt.side_effect_level,
                })
            })
            .collect(),
    )
}

fn tool_names_from_value(value: &Value) -> Vec<String> {
    value
        .as_array()
        .map(|items| {
            items
                .iter()
                .filter_map(|item| item.get("tool_name").and_then(|value| value.as_str()))
                .map(|value| value.to_string())
                .collect::<Vec<_>>()
        })
        .unwrap_or_default()
}

fn compact_json_value(value: &Value, depth: usize) -> Value {
    if depth == 0 {
        return Value::String("[truncated]".to_string());
    }

    match value {
        Value::String(text) => Value::String(safe_truncate(text, 240)),
        Value::Array(items) => Value::Array(
            items
                .iter()
                .take(8)
                .map(|item| compact_json_value(item, depth.saturating_sub(1)))
                .collect(),
        ),
        Value::Object(map) => {
            let mut compact = serde_json::Map::new();
            for (key, item) in map.iter().take(16) {
                compact.insert(
                    safe_truncate(key, 64),
                    compact_json_value(item, depth.saturating_sub(1)),
                );
            }
            Value::Object(compact)
        }
        other => other.clone(),
    }
}

fn parse_operational_json(raw: Option<&str>) -> Option<Value> {
    raw.and_then(|text| serde_json::from_str::<Value>(text).ok())
        .map(|value| compact_json_value(&value, 4))
}

fn summarize_operational_event(row: &crate::storage::entities::operational_log::Model) -> Value {
    let mut summary = serde_json::Map::new();
    summary.insert(
        "created_at".to_string(),
        Value::String(row.created_at.clone()),
    );
    summary.insert("success".to_string(), Value::Bool(row.success));
    summary.insert("outcome".to_string(), Value::String(row.outcome.clone()));
    if let Some(latency_ms) = row.latency_ms {
        summary.insert("latency_ms".to_string(), json!(latency_ms));
    }
    if let Some(tool_name) = row
        .tool_name
        .as_ref()
        .filter(|value| !value.trim().is_empty())
    {
        summary.insert("tool_name".to_string(), Value::String(tool_name.clone()));
    }
    if let Some(payload) = parse_operational_json(row.payload.as_deref()) {
        summary.insert("payload".to_string(), payload);
    }
    if let Some(arguments) = parse_operational_json(row.arguments.as_deref()) {
        summary.insert("arguments".to_string(), arguments);
    }
    Value::Object(summary)
}

fn build_decision_episode(
    logs: &[crate::storage::entities::operational_log::Model],
) -> Option<Value> {
    if logs.is_empty() {
        return None;
    }

    let mut decision = serde_json::Map::new();
    let mut tool_calls = Vec::new();
    for row in logs {
        match row.event_type.as_str() {
            "request_shape_assessment" if !decision.contains_key("request_shape") => {
                decision.insert(
                    "request_shape".to_string(),
                    summarize_operational_event(row),
                );
            }
            "action_selection" if !decision.contains_key("action_selection") => {
                decision.insert(
                    "action_selection".to_string(),
                    summarize_operational_event(row),
                );
            }
            "routing_decision" if !decision.contains_key("routing") => {
                decision.insert("routing".to_string(), summarize_operational_event(row));
            }
            "tool_plan_validation" if !decision.contains_key("tool_plan_validation") => {
                decision.insert(
                    "tool_plan_validation".to_string(),
                    summarize_operational_event(row),
                );
            }
            "llm_decision" if !decision.contains_key("llm_decision") => {
                decision.insert("llm_decision".to_string(), summarize_operational_event(row));
            }
            "tool_batch_summary" if !decision.contains_key("tool_batch") => {
                decision.insert("tool_batch".to_string(), summarize_operational_event(row));
            }
            "tool_call" if tool_calls.len() < 6 => {
                tool_calls.push(summarize_operational_event(row));
            }
            "response_complete" | "request_failed" if !decision.contains_key("outcome") => {
                decision.insert("outcome".to_string(), summarize_operational_event(row));
            }
            _ => {}
        }
    }
    if !tool_calls.is_empty() {
        tool_calls.reverse();
        decision.insert("tool_calls".to_string(), Value::Array(tool_calls));
    }
    if decision.is_empty() {
        None
    } else {
        Some(Value::Object(decision))
    }
}

fn prompt_telemetry_from_logs(
    logs: &[crate::storage::entities::operational_log::Model],
) -> Option<Value> {
    logs.iter()
        .filter(|row| row.event_type == "prompt_telemetry")
        .max_by_key(|row| row.created_at.as_str())
        .and_then(|row| {
            row.payload
                .as_deref()
                .and_then(|text| serde_json::from_str::<Value>(text).ok())
        })
}

fn suggested_steps_from_tools(tool_names: &[String]) -> Vec<String> {
    if tool_names.is_empty() {
        return vec!["Review the latest run context and complete the task directly.".to_string()];
    }
    tool_names
        .iter()
        .enumerate()
        .map(|(index, tool)| {
            if index == tool_names.len().saturating_sub(1) {
                format!("Finish with `{}` and return the concrete result.", tool)
            } else {
                format!(
                    "Use `{}` as step {} in the learned sequence.",
                    tool,
                    index + 1
                )
            }
        })
        .collect()
}

fn bool_like(value: &str) -> Option<bool> {
    match value.trim().to_ascii_lowercase().as_str() {
        "true" | "yes" | "on" | "1" => Some(true),
        "false" | "no" | "off" | "0" => Some(false),
        _ => None,
    }
}

fn humanize_fact_key(key: &str) -> String {
    key.split('_')
        .filter(|part| !part.trim().is_empty())
        .map(|part| {
            let mut chars = part.chars();
            match chars.next() {
                Some(first) => first.to_ascii_uppercase().to_string() + chars.as_str(),
                None => String::new(),
            }
        })
        .collect::<Vec<_>>()
        .join(" ")
}

fn describe_user_preference_memory(
    key: &str,
    value: &str,
) -> Option<(&'static str, String, String)> {
    let key = key.trim();
    let value = value.trim();
    if key.is_empty() || value.is_empty() {
        return None;
    }

    let normalized_value = safe_truncate(value, 220);
    let (kind, title, content) = match key {
        "user_name" => (
            "personal_fact",
            "Learned personal fact".to_string(),
            format!("The user's preferred name is {}.", normalized_value),
        ),
        "rule_require_explicit_approval_before_side_effects" => (
            "constraint",
            "Learned operating constraint".to_string(),
            if bool_like(value).unwrap_or(true) {
                "Require explicit approval before side-effecting actions.".to_string()
            } else {
                "Explicit approval before side-effecting actions is not required.".to_string()
            },
        ),
        "rule_show_plan_before_side_effects" => (
            "constraint",
            "Learned operating constraint".to_string(),
            if bool_like(value).unwrap_or(true) {
                "Show the plan before side-effecting actions.".to_string()
            } else {
                "Showing the plan before side-effecting actions is optional.".to_string()
            },
        ),
        _ if key.starts_with("likes_") => (
            "personal_fact",
            "Learned personal fact".to_string(),
            format!("The user likes {}.", normalized_value),
        ),
        _ if key.starts_with("dislikes_") => (
            "personal_fact",
            "Learned personal fact".to_string(),
            format!("The user dislikes {}.", normalized_value),
        ),
        _ if key.starts_with("rule_") => (
            "constraint",
            "Learned operating constraint".to_string(),
            format!("{}: {}.", humanize_fact_key(key), normalized_value),
        ),
        _ => (
            "personal_fact",
            "Learned personal fact".to_string(),
            format!("{}: {}.", humanize_fact_key(key), normalized_value),
        ),
    };

    Some((kind, title, content))
}

fn candidate_action_name(pattern: &procedural_pattern::Model) -> String {
    let mut slug = pattern
        .title
        .to_ascii_lowercase()
        .chars()
        .map(|ch| if ch.is_ascii_alphanumeric() { ch } else { '-' })
        .collect::<String>();
    while slug.contains("--") {
        slug = slug.replace("--", "-");
    }
    slug = slug.trim_matches('-').to_string();
    let base = if slug.is_empty() {
        "learned-workflow".to_string()
    } else {
        format!("learned-{}", safe_truncate(&slug, 28))
    };
    format!(
        "{}-{}",
        base.trim_matches('-'),
        short_hash(&[pattern.id.as_str()])
    )
}

fn workflow_candidate_markdown(
    pattern: &procedural_pattern::Model,
    action_name: &str,
    steps: &[String],
) -> String {
    let description = pattern.summary.replace('"', "'").replace(['\n', '\r'], " ");
    let workflow_steps = if steps.is_empty() {
        "- Review the request context and follow the learned sequence.".to_string()
    } else {
        steps
            .iter()
            .enumerate()
            .map(|(index, step)| format!("### Step {}\n{}\n", index + 1, step))
            .collect::<Vec<_>>()
            .join("\n")
    };
    format!(
        "---\nname: {action_name}\ndescription: \"{description}\"\nversion: \"1.0.0\"\npermissions: [memory, research]\n---\n\n# {title}\n\n{summary}\n\n## Trigger\n{trigger}\n\n## Workflow\n\n{workflow_steps}\n",
        title = pattern.title,
        summary = pattern.summary,
        trigger = if pattern.trigger_summary.trim().is_empty() {
            "Use this workflow when the request matches the learned pattern."
        } else {
            pattern.trigger_summary.as_str()
        },
    )
}

fn build_skill_patch_candidate_content(
    action: &str,
    skill_name: &str,
    target_source: &str,
    before_content: &str,
    after_content: &str,
    diff_summary: &str,
    evidence: Value,
    impact_baseline: SkillMetricsSnapshot,
    history_versions_read: usize,
) -> Value {
    let canonical_skill_name = skill_evolution::canonicalize_skill_name(skill_name);
    json!({
        "action": action,
        "skill_name": canonical_skill_name,
        "target_source": target_source,
        "before_content": before_content,
        "after_content": after_content,
        "content": after_content,
        "diff_summary": diff_summary,
        "diff_preview": skill_evolution::build_skill_diff_preview(before_content, after_content),
        "evidence": evidence,
        "impact_baseline": impact_baseline,
        "impact_status": "pending",
        "history_versions_read": history_versions_read,
    })
}

fn run_request_preview(run: &experience_run::Model) -> String {
    run.request_text
        .as_deref()
        .map(str::trim)
        .filter(|value| !value.is_empty())
        .map(|value| safe_truncate(value, 120))
        .or_else(|| {
            run.failure_reason
                .as_deref()
                .map(str::trim)
                .filter(|value| !value.is_empty())
                .map(|value| safe_truncate(value, 120))
        })
        .unwrap_or_else(|| run.intent_key.clone())
}

fn run_failure_summary(run: &experience_run::Model) -> Option<String> {
    run.failure_reason
        .as_deref()
        .or(run.outcome_summary.as_deref())
        .map(str::trim)
        .filter(|value| !value.is_empty())
        .map(|value| safe_truncate(value, 140))
}

fn run_failure_label(run: &experience_run::Model) -> String {
    run.task_type
        .as_deref()
        .map(str::trim)
        .filter(|value| !value.is_empty())
        .map(|value| value.to_string())
        .unwrap_or_else(|| run_request_preview(run))
}

fn candidate_exclusion_labels(runs: &[&experience_run::Model]) -> Vec<String> {
    let mut labels = runs
        .iter()
        .map(|run| run_failure_label(run))
        .filter(|value| !value.is_empty())
        .collect::<Vec<_>>();
    labels.sort();
    labels.dedup();
    labels.truncate(4);
    labels
}

fn candidate_failure_checks_section(
    executed_failures: &[&experience_run::Model],
    selected_only_failures: &[&experience_run::Model],
) -> Option<String> {
    let mut bullets = Vec::new();

    let mut tool_names = executed_failures
        .iter()
        .flat_map(|run| {
            run.tool_sequence_json
                .as_array()
                .into_iter()
                .flatten()
                .filter_map(|item| item.get("tool_name").and_then(|value| value.as_str()))
                .map(|value| value.to_string())
                .collect::<Vec<_>>()
        })
        .collect::<Vec<_>>();
    tool_names.sort();
    tool_names.dedup();
    if !tool_names.is_empty() {
        bullets.push(format!(
            "- Recheck the workflow before committing when it depends on {}.",
            tool_names
                .iter()
                .take(4)
                .cloned()
                .collect::<Vec<_>>()
                .join(", ")
        ));
    }

    let mut failure_notes = executed_failures
        .iter()
        .filter_map(|run| run_failure_summary(run))
        .collect::<Vec<_>>();
    failure_notes.sort();
    failure_notes.dedup();
    if !failure_notes.is_empty() {
        bullets.push(format!(
            "- Stop and reassess when you see the same failure mode: {}.",
            failure_notes
                .iter()
                .take(3)
                .cloned()
                .collect::<Vec<_>>()
                .join(" | ")
        ));
    }

    let mismatch_labels = candidate_exclusion_labels(selected_only_failures);
    if !mismatch_labels.is_empty() {
        bullets.push(format!(
            "- Treat {} as out-of-scope unless the request explicitly matches the trigger.",
            mismatch_labels.join(", ")
        ));
    }

    if bullets.is_empty() {
        return None;
    }
    Some(bullets.join("\n"))
}

pub async fn load_learning_enabled(storage: &Storage) -> bool {
    storage
        .get(LEARNING_ENABLED_KEY)
        .await
        .ok()
        .flatten()
        .and_then(|bytes| String::from_utf8(bytes).ok())
        .map(|value| !value.trim().eq_ignore_ascii_case("false"))
        .unwrap_or(true)
}

pub async fn load_learning_model_slot(storage: &Storage) -> Option<String> {
    storage
        .get(LEARNING_MODEL_SLOT_KEY)
        .await
        .ok()
        .flatten()
        .and_then(|bytes| String::from_utf8(bytes).ok())
        .map(|value| value.trim().to_string())
        .filter(|value| !value.is_empty())
}

pub async fn load_learning_queue_cap(storage: &Storage) -> usize {
    storage
        .get(LEARNING_QUEUE_CAP_KEY)
        .await
        .ok()
        .flatten()
        .and_then(|bytes| String::from_utf8(bytes).ok())
        .and_then(|value| value.trim().parse::<usize>().ok())
        .filter(|value| *value > 0)
        .unwrap_or(DEFAULT_QUEUE_CAP)
}

fn build_experience_run_id(execution_run_id: &str) -> String {
    stable_id("exprun", &[execution_run_id])
}

fn build_item_id(
    kind: &str,
    scope: &str,
    project_id: Option<&str>,
    conversation_id: Option<&str>,
    normalized_key: &str,
) -> String {
    stable_id(
        "expitem",
        &[
            kind,
            scope,
            project_id.unwrap_or(""),
            conversation_id.unwrap_or(""),
            normalized_key,
        ],
    )
}

fn build_pattern_id(
    scope: &str,
    project_id: Option<&str>,
    conversation_id: Option<&str>,
    intent_key: &str,
    tool_sequence_digest: Option<&str>,
) -> String {
    stable_id(
        "pattern",
        &[
            scope,
            project_id.unwrap_or(""),
            conversation_id.unwrap_or(""),
            intent_key,
            tool_sequence_digest.unwrap_or(""),
        ],
    )
}

#[allow(clippy::too_many_arguments)]
pub async fn record_execution_experience(
    storage: &Storage,
    execution_run: &ExecutionRun,
    message: &str,
    channel: &str,
    conversation_id: Option<&str>,
    project_id: Option<&str>,
    prompt_version: Option<&str>,
    classifier_prompt_version: Option<&str>,
    specialist_prompt_version: Option<&str>,
    strategy_version: Option<&str>,
    policy_version: Option<&str>,
    model_slot: Option<&str>,
) -> Result<()> {
    if !load_learning_enabled(storage).await {
        return Ok(());
    }
    let tool_attempts = storage
        .list_tool_attempts_for_run(&execution_run.id)
        .await
        .unwrap_or_default();
    let task_type = crate::core::self_evolve::strategy_runtime::infer_task_type_from_action_names(
        tool_attempts
            .iter()
            .map(|attempt| attempt.tool_name.as_str()),
    );
    let intent_key = derive_intent_key(message, &task_type);
    let scope = scope_from_ids(project_id, conversation_id).to_string();
    let sequence_json = tool_sequence_json(&tool_attempts);
    let sequence_digest = tool_sequence_digest(&tool_attempts);
    let success_state = if matches!(
        execution_run.status,
        ExecutionRunStatus::Completed | ExecutionRunStatus::Degraded
    ) {
        "provisional"
    } else {
        "failed"
    };
    let mut metadata = json!({
        "execution_status": execution_run.status.as_str(),
        "degradation": execution_run.degradation,
        "attempted_models": execution_run.attempted_models,
        "last_error": execution_run.last_error,
        "tool_count": tool_attempts.len(),
        "degraded": matches!(execution_run.status, ExecutionRunStatus::Degraded),
    });
    let (decision_episode, prompt_telemetry) = if let Some(trace_id) = execution_run
        .trace_id
        .as_deref()
        .filter(|value| !value.trim().is_empty())
    {
        let logs = storage
            .list_operational_logs_for_trace_ids(&[trace_id.to_string()], 64)
            .await
            .unwrap_or_default();
        (
            build_decision_episode(&logs),
            prompt_telemetry_from_logs(&logs),
        )
    } else {
        (None, None)
    };
    if let Some(obj) = metadata.as_object_mut() {
        if let Some(version) = classifier_prompt_version.filter(|value| !value.trim().is_empty()) {
            obj.insert(
                "classifier_prompt_version".to_string(),
                Value::String(version.to_string()),
            );
        }
        if let Some(version) = specialist_prompt_version.filter(|value| !value.trim().is_empty()) {
            obj.insert(
                "specialist_prompt_version".to_string(),
                Value::String(version.to_string()),
            );
        }
        if let Some(decision_episode) = decision_episode {
            obj.insert("decision_episode".to_string(), decision_episode);
        }
        if let Some(prompt_telemetry) = prompt_telemetry {
            obj.insert("prompt_telemetry".to_string(), prompt_telemetry);
        }
    }
    let experience_id = build_experience_run_id(&execution_run.id);
    let now = chrono::Utc::now().to_rfc3339();
    storage
        .upsert_experience_run(&experience_run::Model {
            id: experience_id.clone(),
            execution_run_id: Some(execution_run.id.clone()),
            trace_id: execution_run.trace_id.clone(),
            conversation_id: conversation_id.map(|value| value.to_string()),
            project_id: project_id.map(|value| value.to_string()),
            channel: channel.to_string(),
            scope,
            intent_key,
            task_type: Some(task_type),
            request_text: Some(safe_truncate(message.trim(), 2000)),
            tool_sequence_digest: sequence_digest.clone(),
            tool_sequence_json: sequence_json,
            strategy_version: strategy_version.map(|value| value.to_string()),
            policy_version: policy_version.map(|value| value.to_string()),
            prompt_version: prompt_version.map(|value| value.to_string()),
            model_slot: model_slot.map(|value| value.to_string()),
            success_state: success_state.to_string(),
            correction_state: "none".to_string(),
            outcome_summary: execution_run.result_summary.clone(),
            failure_reason: execution_run.last_error.clone(),
            metadata,
            consolidated: false,
            accepted_at: None,
            corrected_at: None,
            heuristic_reflected: false,
            heuristic_reflection_status: Some("pending".to_string()),
            heuristic_reflection_attempted_at: None,
            heuristic_reflection_completed_at: None,
            heuristic_lesson_id: None,
            heuristic_reflection_error: None,
            created_at: execution_run.created_at.clone(),
            updated_at: execution_run.updated_at.clone(),
        })
        .await?;

    for attempt in tool_attempts {
        let edge_type = if attempt.status.as_str() == "success" {
            "succeeded_with"
        } else {
            "failed_with"
        };
        storage
            .upsert_experience_edge(&experience_edge::Model {
                id: stable_id(
                    "edge",
                    &[
                        experience_id.as_str(),
                        edge_type,
                        attempt.tool_name.as_str(),
                        &attempt.sequence_no.to_string(),
                    ],
                ),
                source_ref: experience_id.clone(),
                source_kind: "experience_run".to_string(),
                target_ref: format!("tool:{}", attempt.tool_name),
                target_kind: "tool".to_string(),
                edge_type: edge_type.to_string(),
                weight: if edge_type == "succeeded_with" {
                    1.0
                } else {
                    0.35
                },
                source_run_id: Some(experience_id.clone()),
                metadata: json!({
                    "tool_name": attempt.tool_name,
                    "tool_status": attempt.status.as_str(),
                    "sequence_no": attempt.sequence_no,
                }),
                created_at: now.clone(),
                updated_at: now.clone(),
            })
            .await?;
    }

    Ok(())
}

pub async fn record_user_correction(
    storage: &Storage,
    conversation_id: &str,
    message: &str,
) -> Result<()> {
    if !load_learning_enabled(storage).await {
        return Ok(());
    }
    let signal = safe_truncate(message.trim(), 180);
    let mut corrected = false;
    if let Some(trace_id) = storage
        .latest_assistant_trace_id_for_conversation(conversation_id)
        .await?
    {
        corrected = storage
            .mark_provisional_experience_run_corrected_by_trace_id(&trace_id, &signal)
            .await?
            .is_some();
    }
    if !corrected {
        let _ = storage
            .mark_latest_provisional_experience_run_corrected(
                conversation_id,
                &signal,
                CORRECTION_WINDOW_MINUTES,
            )
            .await?;
    }
    Ok(())
}

fn positive_procedure_summary(run: &experience_run::Model, tool_names: &[String]) -> String {
    if tool_names.is_empty() {
        format!(
            "For `{}`, the successful pattern was to solve the task directly and return the result.",
            run.intent_key
        )
    } else {
        format!(
            "For `{}`, the successful pattern was: {}.",
            run.intent_key,
            tool_names.join(" -> ")
        )
    }
}

fn negative_lesson_summary(run: &experience_run::Model, tool_names: &[String]) -> String {
    let sequence = if tool_names.is_empty() {
        "the recent approach".to_string()
    } else {
        tool_names.join(" -> ")
    };
    format!(
        "Avoid repeating `{}` with {} when the user is correcting or the run failed.",
        run.intent_key, sequence
    )
}

pub async fn sync_user_preference_to_experience_item(
    storage: &Storage,
    key: &str,
    value: &str,
    confidence: f64,
    source: &str,
) -> Result<()> {
    let Some((kind, title, content)) = describe_user_preference_memory(key, value) else {
        return Ok(());
    };
    let normalized_key = format!("user_pref::{}", key.trim());
    let id = build_item_id(kind, "global", None, None, &normalized_key);
    let existing = storage.get_experience_item(&id).await?;
    let now = chrono::Utc::now().to_rfc3339();
    let support_count = existing
        .as_ref()
        .map(|item| item.support_count.saturating_add(1))
        .unwrap_or(1);
    let merged_confidence = existing
        .as_ref()
        .map(|item| item.confidence.max(confidence).min(0.99))
        .unwrap_or(confidence);

    storage
        .upsert_experience_item(&experience_item::Model {
            id,
            kind: kind.to_string(),
            scope: "global".to_string(),
            project_id: None,
            conversation_id: None,
            title,
            content,
            normalized_key,
            confidence: merged_confidence,
            support_count,
            contradiction_count: existing
                .as_ref()
                .map(|item| item.contradiction_count)
                .unwrap_or_default(),
            status: "active".to_string(),
            metadata: json!({
                "source": source,
                "user_preference_key": key.trim(),
                "user_preference_value": safe_truncate(value.trim(), 220),
            }),
            last_supported_at: Some(now.clone()),
            last_contradicted_at: existing
                .as_ref()
                .and_then(|item| item.last_contradicted_at.clone()),
            created_at: existing
                .as_ref()
                .map(|item| item.created_at.clone())
                .unwrap_or_else(|| now.clone()),
            updated_at: now,
            embedding: existing.as_ref().and_then(|item| item.embedding.clone()),
        })
        .await?;

    Ok(())
}

async fn consolidate_run(storage: &Storage, run: &experience_run::Model) -> Result<()> {
    let tool_names = tool_names_from_value(&run.tool_sequence_json);
    let scope = run.scope.as_str();
    let project_id = run.project_id.as_deref();
    let conversation_id = run.conversation_id.as_deref();
    let now = chrono::Utc::now().to_rfc3339();

    let is_negative = run.correction_state == "corrected" || run.success_state == "failed";
    if is_negative {
        let related_procedure_key = format!(
            "procedure::{}::{}",
            run.intent_key,
            run.tool_sequence_digest.as_deref().unwrap_or("direct")
        );
        let related_procedure_id = build_item_id(
            "procedure",
            scope,
            project_id,
            conversation_id,
            &related_procedure_key,
        );
        if let Some(mut procedure) = storage.get_experience_item(&related_procedure_id).await? {
            procedure.contradiction_count = procedure.contradiction_count.saturating_add(1);
            procedure.confidence = (procedure.confidence - 0.12).max(0.20);
            procedure.last_contradicted_at = Some(now.clone());
            procedure.updated_at = now.clone();
            storage.upsert_experience_item(&procedure).await?;
        }
    }
    let kind = if is_negative { "lesson" } else { "procedure" };
    let normalized_key = format!(
        "{}::{}::{}",
        kind,
        run.intent_key,
        run.tool_sequence_digest.as_deref().unwrap_or("direct")
    );
    let id = build_item_id(kind, scope, project_id, conversation_id, &normalized_key);
    let existing = storage.get_experience_item(&id).await?;
    let support_count = existing
        .as_ref()
        .map(|item| item.support_count.saturating_add(1))
        .unwrap_or(1);
    let contradiction_count = if is_negative {
        existing
            .as_ref()
            .map(|item| item.contradiction_count)
            .unwrap_or_default()
    } else {
        existing
            .as_ref()
            .map(|item| item.contradiction_count)
            .unwrap_or_default()
    };
    let confidence = if is_negative {
        existing
            .as_ref()
            .map(|item| (item.confidence + 0.05).min(0.94))
            .unwrap_or(0.64)
    } else {
        existing
            .as_ref()
            .map(|item| (item.confidence + 0.08).min(0.98))
            .unwrap_or(0.7)
    };
    let content = if is_negative {
        negative_lesson_summary(run, &tool_names)
    } else {
        positive_procedure_summary(run, &tool_names)
    };
    let steps = suggested_steps_from_tools(&tool_names);
    storage
        .upsert_experience_item(&experience_item::Model {
            id: id.clone(),
            kind: kind.to_string(),
            scope: scope.to_string(),
            project_id: run.project_id.clone(),
            conversation_id: run.conversation_id.clone(),
            title: if is_negative {
                format!("Lesson for {}", run.intent_key)
            } else {
                format!("Procedure for {}", run.intent_key)
            },
            content,
            normalized_key,
            confidence,
            support_count,
            contradiction_count,
            status: "active".to_string(),
            metadata: json!({
                "intent_key": run.intent_key,
                "task_type": run.task_type,
                "tool_sequence_digest": run.tool_sequence_digest,
                "tool_sequence": tool_names,
                "suggested_steps": steps,
                "source_run_id": run.id,
                "polarity": if is_negative { "negative" } else { "positive" },
            }),
            last_supported_at: Some(now.clone()),
            last_contradicted_at: existing
                .as_ref()
                .and_then(|item| item.last_contradicted_at.clone()),
            created_at: existing
                .as_ref()
                .map(|item| item.created_at.clone())
                .unwrap_or_else(|| now.clone()),
            updated_at: now.clone(),
            embedding: existing.as_ref().and_then(|item| item.embedding.clone()),
        })
        .await?;

    storage
        .upsert_experience_edge(&experience_edge::Model {
            id: stable_id("edge", &[run.id.as_str(), "derived_from", id.as_str()]),
            source_ref: id.clone(),
            source_kind: "experience_item".to_string(),
            target_ref: run.id.clone(),
            target_kind: "experience_run".to_string(),
            edge_type: "derived_from".to_string(),
            weight: 1.0,
            source_run_id: Some(run.id.clone()),
            metadata: json!({
                "intent_key": run.intent_key,
                "success_state": run.success_state,
                "correction_state": run.correction_state,
            }),
            created_at: now.clone(),
            updated_at: now.clone(),
        })
        .await?;

    storage.mark_experience_run_consolidated(&run.id).await?;
    Ok(())
}

pub async fn run_experience_consolidation(storage: &Storage) -> Result<usize> {
    if !load_learning_enabled(storage).await {
        return Ok(0);
    }
    let cap = load_learning_queue_cap(storage).await as u64;
    let _ = storage
        .finalize_stale_provisional_experience_runs(CORRECTION_WINDOW_MINUTES, cap)
        .await?;
    let runs = storage.list_experience_runs_for_consolidation(cap).await?;
    let mut processed = 0usize;
    for run in runs {
        consolidate_run(storage, &run).await?;
        processed += 1;
    }
    Ok(processed)
}

pub async fn run_pattern_induction(storage: &Storage) -> Result<usize> {
    if !load_learning_enabled(storage).await {
        return Ok(0);
    }
    let procedures = storage
        .list_active_experience_items(
            &["procedure"],
            None,
            None,
            load_learning_queue_cap(storage).await as u64,
        )
        .await?
        .into_iter()
        .filter(|procedure| !experience_item_is_external_source(procedure))
        .collect::<Vec<_>>();
    let mut updated = 0usize;
    for procedure in procedures {
        let metadata = procedure.metadata.as_object().cloned().unwrap_or_default();
        let intent_key = metadata
            .get("intent_key")
            .and_then(|value| value.as_str())
            .unwrap_or(procedure.normalized_key.as_str());
        let tool_digest = metadata
            .get("tool_sequence_digest")
            .and_then(|value| value.as_str());
        let pattern_id = build_pattern_id(
            &procedure.scope,
            procedure.project_id.as_deref(),
            procedure.conversation_id.as_deref(),
            intent_key,
            tool_digest,
        );
        let tool_sequence = metadata
            .get("tool_sequence")
            .cloned()
            .unwrap_or_else(|| Value::Array(Vec::new()));
        let steps_json = metadata
            .get("suggested_steps")
            .cloned()
            .unwrap_or_else(|| Value::Array(Vec::new()));
        let support_count = procedure.support_count.max(0);
        let correction_count = procedure.contradiction_count.max(0);
        let sample_count = support_count.saturating_add(correction_count).max(1);
        let success_rate = support_count as f64 / sample_count as f64;
        let now = chrono::Utc::now().to_rfc3339();
        storage
            .upsert_procedural_pattern(&procedural_pattern::Model {
                id: pattern_id.clone(),
                intent_key: intent_key.to_string(),
                scope: procedure.scope.clone(),
                project_id: procedure.project_id.clone(),
                conversation_id: procedure.conversation_id.clone(),
                title: procedure.title.clone(),
                trigger_summary: format!(
                    "Use when the request matches `{}` within the current {} scope.",
                    intent_key, procedure.scope
                ),
                summary: procedure.content.clone(),
                tool_sequence_digest: tool_digest.map(|value| value.to_string()),
                steps_json,
                tool_sequence_json: tool_sequence,
                sample_count,
                success_count: support_count,
                correction_count,
                success_rate,
                last_validated_at: Some(now.clone()),
                status: if correction_count > support_count {
                    "deprecated".to_string()
                } else if support_count >= 2 {
                    "active".to_string()
                } else {
                    "draft".to_string()
                },
                metadata: json!({
                    "source_item_id": procedure.id,
                    "task_type": metadata.get("task_type").cloned().unwrap_or(Value::Null),
                }),
                created_at: now.clone(),
                updated_at: now.clone(),
            })
            .await?;

        storage
            .upsert_experience_edge(&experience_edge::Model {
                id: stable_id(
                    "edge",
                    &[procedure.id.as_str(), "supports", pattern_id.as_str()],
                ),
                source_ref: procedure.id.clone(),
                source_kind: "experience_item".to_string(),
                target_ref: pattern_id.clone(),
                target_kind: "procedural_pattern".to_string(),
                edge_type: "supports".to_string(),
                weight: success_rate.max(0.1),
                source_run_id: None,
                metadata: json!({
                    "intent_key": intent_key,
                }),
                created_at: now.clone(),
                updated_at: now.clone(),
            })
            .await?;
        updated += 1;
    }
    Ok(updated)
}

fn build_strategy_candidate_profile(
    pattern: &procedural_pattern::Model,
    task_type: &str,
    tool_names: &[String],
) -> crate::core::self_evolve::strategy_runtime::ToolStrategyProfile {
    let mut task_guidance = std::collections::HashMap::new();
    let mut lines = vec![
        format!(
            "When the request matches `{}`, prefer the learned procedure `{}`.",
            task_type, pattern.title
        ),
        "Use this as guidance, not as a hard rule, and adapt when the context clearly differs."
            .to_string(),
    ];
    if !tool_names.is_empty() {
        lines.push(format!(
            "When the environment matches, start with tools in this order: {}.",
            tool_names.join(" -> ")
        ));
    }
    task_guidance.insert(task_type.to_string(), lines);
    crate::core::self_evolve::strategy_runtime::ToolStrategyProfile {
        version: format!(
            "learned-strategy-{}",
            short_hash(&[pattern.id.as_str(), task_type])
        ),
        updated_at: Some(chrono::Utc::now().to_rfc3339()),
        default_guidance: vec![
            "Prefer proven local procedures before improvising a new tool plan.".to_string(),
        ],
        task_guidance,
    }
}

fn normalize_semantic_text(value: &str) -> String {
    value
        .chars()
        .map(|ch| {
            if ch.is_ascii_alphanumeric() {
                ch.to_ascii_lowercase()
            } else {
                ' '
            }
        })
        .collect::<String>()
        .split_whitespace()
        .collect::<Vec<_>>()
        .join(" ")
}

fn semantic_token_set(value: &str) -> HashSet<String> {
    normalize_semantic_text(value)
        .split_whitespace()
        .filter(|token| token.len() >= 3)
        .map(|token| token.to_string())
        .collect()
}

fn semantic_similarity_score(left: &str, right: &str) -> f64 {
    let left_tokens = semantic_token_set(left);
    let right_tokens = semantic_token_set(right);
    if left_tokens.is_empty() || right_tokens.is_empty() {
        return 0.0;
    }
    let overlap = left_tokens.intersection(&right_tokens).count() as f64;
    let union = left_tokens.union(&right_tokens).count() as f64;
    if union <= f64::EPSILON {
        0.0
    } else {
        overlap / union
    }
}

fn experience_item_metadata_text<'a>(
    item: &'a experience_item::Model,
    field: &str,
) -> Option<&'a str> {
    item.metadata
        .get(field)
        .and_then(|value| value.as_str())
        .map(str::trim)
        .filter(|value| !value.is_empty())
}

pub(crate) fn experience_item_is_reflected_heuristic(item: &experience_item::Model) -> bool {
    item.kind == "lesson"
        && experience_item_metadata_text(item, "origin") == Some(HEURISTIC_REFLECTION_ORIGIN)
}

pub(crate) fn reflected_heuristic_task_type<'a>(
    item: &'a experience_item::Model,
) -> Option<&'a str> {
    experience_item_metadata_text(item, "task_type")
}

pub(crate) fn reflected_heuristic_polarity<'a>(
    item: &'a experience_item::Model,
) -> Option<&'a str> {
    experience_item_metadata_text(item, "polarity")
}

pub(crate) fn reflected_heuristic_applicability<'a>(
    item: &'a experience_item::Model,
) -> Option<&'a str> {
    experience_item_metadata_text(item, "applicability")
}

pub(crate) fn reflected_heuristic_confidence(item: &experience_item::Model) -> f64 {
    item.metadata
        .get("reflection_confidence")
        .and_then(|value| value.as_f64())
        .unwrap_or(item.confidence)
        .clamp(0.0, 1.0)
}

fn reflected_heuristic_merge_score(
    existing: &experience_item::Model,
    run: &experience_run::Model,
    heuristic: &ReflectedHeuristic,
) -> f64 {
    if !experience_item_is_reflected_heuristic(existing) {
        return 0.0;
    }
    if reflected_heuristic_polarity(existing) != Some(heuristic.polarity.as_str()) {
        return 0.0;
    }
    let existing_task_type = reflected_heuristic_task_type(existing)
        .or(run.task_type.as_deref())
        .unwrap_or("general");
    let current_task_type = run.task_type.as_deref().unwrap_or("general");
    if existing_task_type != current_task_type {
        return 0.0;
    }

    let existing_text = format!(
        "{} {}",
        existing.content,
        reflected_heuristic_applicability(existing).unwrap_or("")
    );
    let incoming_text = format!(
        "{} {}",
        heuristic.heuristic,
        heuristic.applicability.as_deref().unwrap_or("")
    );
    let mut score = semantic_similarity_score(&existing_text, &incoming_text);
    if normalize_semantic_text(&existing.content) == normalize_semantic_text(&heuristic.heuristic) {
        score = score.max(1.0);
    }
    score
}

fn reflected_heuristic_title(run: &experience_run::Model, polarity: &str) -> String {
    let task_type = run.task_type.as_deref().unwrap_or("general");
    match polarity {
        "negative" => format!("Reflected caution for {}", task_type),
        _ => format!("Reflected heuristic for {}", task_type),
    }
}

fn build_reflected_heuristic_normalized_key(
    run: &experience_run::Model,
    heuristic: &ReflectedHeuristic,
) -> String {
    let task_type = run.task_type.as_deref().unwrap_or("general");
    let normalized = normalize_semantic_text(&heuristic.heuristic);
    let applicability = normalize_semantic_text(heuristic.applicability.as_deref().unwrap_or(""));
    format!(
        "lesson::heuristic::{}::{}::{}",
        task_type,
        heuristic.polarity,
        short_hash(&[normalized.as_str(), applicability.as_str()])
    )
}

fn build_reflected_heuristic_strategy_profile(
    task_type: &str,
    lessons: &[experience_item::Model],
) -> crate::core::self_evolve::strategy_runtime::ToolStrategyProfile {
    let mut task_guidance = HashMap::new();
    let mut lines = vec![
        format!(
            "Apply these learned heuristics when handling similar {} requests.",
            task_type
        ),
        "Use them as decision guidance, and adapt when the current context clearly differs."
            .to_string(),
    ];
    for lesson in lessons {
        let content = lesson.content.trim();
        if content.is_empty() {
            continue;
        }
        lines.push(content.to_string());
    }
    lines.truncate(6);
    task_guidance.insert(task_type.to_string(), lines);
    crate::core::self_evolve::strategy_runtime::ToolStrategyProfile {
        version: format!(
            "learned-strategy-{}",
            short_hash(&[
                task_type,
                &lessons
                    .iter()
                    .map(|lesson| lesson.id.as_str())
                    .collect::<Vec<_>>()
                    .join("|"),
            ])
        ),
        updated_at: Some(chrono::Utc::now().to_rfc3339()),
        default_guidance: vec![
            "Prefer reflected heuristics from successful and corrected runs before improvising a new tool plan."
                .to_string(),
        ],
        task_guidance,
    }
}

pub(crate) async fn upsert_reflected_heuristic_lesson(
    storage: &Storage,
    run: &experience_run::Model,
    heuristic: &ReflectedHeuristic,
) -> Result<ReflectedHeuristicPersistOutcome> {
    let now = chrono::Utc::now().to_rfc3339();
    let scope = run.scope.as_str();
    let project_id = run.project_id.as_deref();
    let conversation_id = run.conversation_id.as_deref();
    let current_task_type = run.task_type.as_deref().unwrap_or("general");
    let lessons = storage
        .list_active_experience_items(&["lesson"], project_id, conversation_id, 48)
        .await?
        .into_iter()
        .filter(|item| item.scope == scope)
        .filter(|item| item.project_id.as_deref() == project_id)
        .filter(|item| item.conversation_id.as_deref() == conversation_id)
        .filter(experience_item_is_reflected_heuristic)
        .collect::<Vec<_>>();

    let existing_match = lessons
        .into_iter()
        .map(|item| {
            let score = reflected_heuristic_merge_score(&item, run, heuristic);
            (item, score)
        })
        .filter(|(_, score)| *score >= 0.62)
        .max_by(|left, right| left.1.total_cmp(&right.1));

    let lesson_id;
    let merged;
    if let Some((mut existing, _)) = existing_match {
        let merged_confidence =
            ((existing.confidence + heuristic.confidence.clamp(0.0, 1.0)) / 2.0).clamp(0.35, 0.99);
        let existing_content = normalize_semantic_text(&existing.content);
        let incoming_content = normalize_semantic_text(&heuristic.heuristic);
        if incoming_content.len() > existing_content.len()
            && heuristic.confidence >= existing.confidence
        {
            existing.content = safe_truncate(&heuristic.heuristic, 260);
        }

        let mut metadata = existing.metadata.as_object().cloned().unwrap_or_default();
        metadata.insert(
            "origin".to_string(),
            Value::String(HEURISTIC_REFLECTION_ORIGIN.to_string()),
        );
        metadata.insert("source_run_id".to_string(), Value::String(run.id.clone()));
        metadata.insert(
            "intent_key".to_string(),
            Value::String(run.intent_key.clone()),
        );
        metadata.insert(
            "task_type".to_string(),
            Value::String(current_task_type.to_string()),
        );
        metadata.insert(
            "tool_sequence_digest".to_string(),
            run.tool_sequence_digest
                .as_ref()
                .map(|value| Value::String(value.clone()))
                .unwrap_or(Value::Null),
        );
        metadata.insert(
            "polarity".to_string(),
            Value::String(heuristic.polarity.clone()),
        );
        metadata.insert(
            "reflection_version".to_string(),
            Value::String(HEURISTIC_REFLECTION_VERSION.to_string()),
        );
        metadata.insert(
            "reflection_confidence".to_string(),
            json!(merged_confidence),
        );
        metadata.insert("last_reflected_at".to_string(), Value::String(now.clone()));
        if let Some(applicability) = heuristic
            .applicability
            .as_deref()
            .filter(|value| !value.trim().is_empty())
        {
            metadata.insert(
                "applicability".to_string(),
                Value::String(safe_truncate(applicability.trim(), 220)),
            );
        }

        existing.confidence = merged_confidence;
        existing.support_count = existing.support_count.saturating_add(1);
        existing.metadata = Value::Object(metadata);
        existing.last_supported_at = Some(now.clone());
        existing.updated_at = now.clone();
        storage.upsert_experience_item(&existing).await?;
        lesson_id = existing.id;
        merged = true;
    } else {
        let normalized_key = build_reflected_heuristic_normalized_key(run, heuristic);
        let id = build_item_id(
            "lesson",
            scope,
            project_id,
            conversation_id,
            &normalized_key,
        );
        let created_at = chrono::Utc::now().to_rfc3339();
        storage
            .upsert_experience_item(&experience_item::Model {
                id: id.clone(),
                kind: "lesson".to_string(),
                scope: scope.to_string(),
                project_id: run.project_id.clone(),
                conversation_id: run.conversation_id.clone(),
                title: reflected_heuristic_title(run, &heuristic.polarity),
                content: safe_truncate(&heuristic.heuristic, 260),
                normalized_key,
                confidence: heuristic.confidence.clamp(0.35, 0.99),
                support_count: 1,
                contradiction_count: 0,
                status: "active".to_string(),
                metadata: json!({
                    "origin": HEURISTIC_REFLECTION_ORIGIN,
                    "source_run_id": run.id.clone(),
                    "intent_key": run.intent_key.clone(),
                    "task_type": current_task_type,
                    "tool_sequence_digest": run.tool_sequence_digest.clone(),
                    "polarity": heuristic.polarity.clone(),
                    "reflection_version": HEURISTIC_REFLECTION_VERSION,
                    "reflection_confidence": heuristic.confidence.clamp(0.0, 1.0),
                    "applicability": heuristic.applicability.as_deref().map(|value| safe_truncate(value.trim(), 220)),
                    "last_reflected_at": now.clone(),
                }),
                last_supported_at: Some(created_at.clone()),
                last_contradicted_at: None,
                created_at: created_at.clone(),
                updated_at: created_at,
                embedding: None,
            })
            .await?;
        lesson_id = id;
        merged = false;
    }

    let edge_now = chrono::Utc::now().to_rfc3339();
    storage
        .upsert_experience_edge(&experience_edge::Model {
            id: stable_id(
                "edge",
                &[run.id.as_str(), "derived_from", lesson_id.as_str()],
            ),
            source_ref: lesson_id.clone(),
            source_kind: "experience_item".to_string(),
            target_ref: run.id.clone(),
            target_kind: "experience_run".to_string(),
            edge_type: "derived_from".to_string(),
            weight: heuristic.confidence.clamp(0.25, 1.0),
            source_run_id: Some(run.id.clone()),
            metadata: json!({
                "intent_key": run.intent_key.clone(),
                "task_type": current_task_type,
                "heuristic_reflection": true,
                "merged": merged,
            }),
            created_at: edge_now.clone(),
            updated_at: edge_now,
        })
        .await?;

    Ok(ReflectedHeuristicPersistOutcome { lesson_id, merged })
}

fn canonical_memory_merge_signature(item: &experience_item::Model) -> Option<String> {
    if item.status != "active" {
        return None;
    }
    let content = item.content.trim();
    if content.is_empty() {
        return None;
    }
    let normalized = content
        .chars()
        .map(|ch| {
            if ch.is_ascii_alphanumeric() {
                ch.to_ascii_lowercase()
            } else {
                ' '
            }
        })
        .collect::<String>()
        .split_whitespace()
        .collect::<Vec<_>>()
        .join(" ");
    if normalized.len() < 16 {
        return None;
    }
    Some(format!(
        "{}::{}::{}::{}::{}",
        item.kind,
        item.scope,
        item.project_id.as_deref().unwrap_or(""),
        item.conversation_id.as_deref().unwrap_or(""),
        normalized
    ))
}

fn memory_merge_sort_key(item: &experience_item::Model) -> (i32, i32, String, String) {
    (
        item.support_count.max(0),
        (item.confidence * 1000.0) as i32,
        item.updated_at.clone(),
        item.id.clone(),
    )
}

fn metadata_marks_external_source(metadata: &Value) -> bool {
    let Some(object) = metadata.as_object() else {
        return false;
    };

    let contains_external_marker = |value: &str| {
        let lowered = value.trim().to_ascii_lowercase();
        !lowered.is_empty()
            && [
                "external-source",
                "community-learning",
                "learning-fenced",
                "moltbook",
            ]
            .iter()
            .any(|marker| lowered.contains(marker))
    };

    if object
        .get("external_source")
        .and_then(|value| value.as_bool())
        .unwrap_or(false)
        || object
            .get("fenced_external_source")
            .and_then(|value| value.as_bool())
            .unwrap_or(false)
        || object
            .get("learning_fenced")
            .and_then(|value| value.as_bool())
            .unwrap_or(false)
    {
        return true;
    }

    if object
        .get("source_scope")
        .and_then(|value| value.as_str())
        .is_some_and(|value| value.trim().eq_ignore_ascii_case("external"))
        || object
            .get("source_kind")
            .and_then(|value| value.as_str())
            .is_some_and(|value| value.trim().eq_ignore_ascii_case("external_source"))
    {
        return true;
    }

    if object
        .get("source")
        .and_then(|value| value.as_str())
        .is_some_and(contains_external_marker)
        || object
            .get("tags")
            .and_then(|value| value.as_str())
            .is_some_and(contains_external_marker)
    {
        return true;
    }

    object
        .get("tags")
        .and_then(|value| value.as_array())
        .is_some_and(|tags| {
            tags.iter()
                .filter_map(|value| value.as_str())
                .any(contains_external_marker)
        })
}

fn experience_item_is_external_source(item: &experience_item::Model) -> bool {
    metadata_marks_external_source(&item.metadata)
}

fn procedural_pattern_is_external_source(pattern: &procedural_pattern::Model) -> bool {
    metadata_marks_external_source(&pattern.metadata)
}

fn canonical_json_string(value: &Value) -> String {
    match value {
        Value::Null => "null".to_string(),
        Value::Bool(value) => value.to_string(),
        Value::Number(value) => value.to_string(),
        Value::String(value) => serde_json::to_string(value).unwrap_or_else(|_| "\"\"".to_string()),
        Value::Array(items) => format!(
            "[{}]",
            items
                .iter()
                .map(canonical_json_string)
                .collect::<Vec<_>>()
                .join(",")
        ),
        Value::Object(map) => {
            let mut entries = map.iter().collect::<Vec<_>>();
            entries.sort_by(|left, right| left.0.cmp(right.0));
            format!(
                "{{{}}}",
                entries
                    .into_iter()
                    .map(|(key, value)| format!(
                        "{}:{}",
                        serde_json::to_string(key).unwrap_or_else(|_| "\"\"".to_string()),
                        canonical_json_string(value)
                    ))
                    .collect::<Vec<_>>()
                    .join(",")
            )
        }
    }
}

fn semantic_candidate_proposed_content(candidate: &learning_candidate::Model) -> Value {
    match candidate.candidate_type.as_str() {
        "strategy" => {
            let mut payload = candidate.proposed_content.clone();
            if let Some(object) = payload.as_object_mut() {
                object.remove("updated_at");
            }
            payload
        }
        _ => candidate.proposed_content.clone(),
    }
}

fn candidate_confidence_bucket(confidence: f64) -> String {
    let bounded = confidence.clamp(0.0, 1.0);
    let rounded = (bounded * 20.0).round() / 20.0;
    format!("{rounded:.2}")
}

fn learning_candidate_material_signature(candidate: &learning_candidate::Model) -> String {
    let pattern_id = candidate.pattern_id.as_deref().unwrap_or("");
    let evidence = canonical_json_string(&candidate.evidence_refs);
    let proposed = canonical_json_string(&semantic_candidate_proposed_content(candidate));
    let confidence = candidate_confidence_bucket(candidate.confidence);
    short_hash(&[
        candidate.candidate_type.as_str(),
        candidate.subject_key.as_str(),
        pattern_id,
        evidence.as_str(),
        proposed.as_str(),
        confidence.as_str(),
    ])
}

fn learning_candidate_revision_id(
    candidate: &learning_candidate::Model,
    signature: &str,
) -> String {
    stable_id(
        "candidate",
        &[
            candidate.candidate_type.as_str(),
            candidate.subject_key.as_str(),
            signature,
        ],
    )
}

fn learning_candidate_needs_refresh(
    existing: &learning_candidate::Model,
    candidate: &learning_candidate::Model,
) -> bool {
    existing.title != candidate.title
        || existing.summary != candidate.summary
        || existing.project_id != candidate.project_id
        || existing.conversation_id != candidate.conversation_id
        || existing.pattern_id != candidate.pattern_id
        || existing.evidence_refs != candidate.evidence_refs
        || semantic_candidate_proposed_content(existing)
            != semantic_candidate_proposed_content(candidate)
        || (existing.confidence - candidate.confidence).abs() > f64::EPSILON
}

fn learning_candidate_is_draft(status: &str) -> bool {
    status.eq_ignore_ascii_case("draft")
}

fn learning_candidate_is_reviewed(status: &str) -> bool {
    status.eq_ignore_ascii_case("approved") || status.eq_ignore_ascii_case("rejected")
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum CandidateWriteOutcome {
    Unchanged,
    Changed,
    LeaseLost,
}

async fn supersede_stale_draft_candidates(
    storage: &Storage,
    lease_guard: &KvLeaseGuard,
    existing_candidates: &[learning_candidate::Model],
    keep_id: Option<&str>,
    reviewed_candidate_id: &str,
) -> Result<CandidateWriteOutcome> {
    let mut updated = 0usize;
    for draft in existing_candidates.iter().filter(|candidate| {
        learning_candidate_is_draft(&candidate.approval_status)
            && keep_id
                .map(|keep_id| candidate.id != keep_id)
                .unwrap_or(true)
    }) {
        let note = format!(
            "Auto-superseded because generation returned to reviewed candidate `{}`.",
            reviewed_candidate_id
        );
        if !storage
            .update_learning_candidate_review_guarded(
                LEARNING_CANDIDATE_GENERATION_LEASE_KEY,
                lease_guard,
                &draft.id,
                "superseded",
                Some(&note),
                None,
            )
            .await?
        {
            return Ok(CandidateWriteOutcome::LeaseLost);
        }
        updated += 1;
    }
    Ok(if updated > 0 {
        CandidateWriteOutcome::Changed
    } else {
        CandidateWriteOutcome::Unchanged
    })
}

async fn upsert_generated_learning_candidate(
    storage: &Storage,
    lease_guard: &KvLeaseGuard,
    mut candidate: learning_candidate::Model,
) -> Result<CandidateWriteOutcome> {
    let existing_candidates = storage
        .list_learning_candidates_for_subject(&candidate.candidate_type, &candidate.subject_key, 64)
        .await?;
    let material_signature = learning_candidate_material_signature(&candidate);

    if let Some(existing) = existing_candidates.iter().find(|existing| {
        learning_candidate_is_reviewed(&existing.approval_status)
            && learning_candidate_material_signature(existing) == material_signature
    }) {
        let superseded = supersede_stale_draft_candidates(
            storage,
            lease_guard,
            &existing_candidates,
            None,
            &existing.id,
        )
        .await?;
        return Ok(superseded);
    }

    if let Some(existing) = existing_candidates.iter().find(|existing| {
        learning_candidate_is_draft(&existing.approval_status)
            && learning_candidate_material_signature(existing) == material_signature
    }) {
        candidate.id = existing.id.clone();
        candidate.created_at = existing.created_at.clone();
        candidate.approval_status = existing.approval_status.clone();
        candidate.review_notes = existing.review_notes.clone();
        candidate.reviewed_at = existing.reviewed_at.clone();
        candidate.approved_ref = existing.approved_ref.clone();
        let superseded = if let Some(reviewed_candidate) = existing_candidates
            .iter()
            .find(|candidate| learning_candidate_is_reviewed(&candidate.approval_status))
        {
            match supersede_stale_draft_candidates(
                storage,
                lease_guard,
                &existing_candidates,
                Some(&existing.id),
                &reviewed_candidate.id,
            )
            .await?
            {
                CandidateWriteOutcome::LeaseLost => return Ok(CandidateWriteOutcome::LeaseLost),
                CandidateWriteOutcome::Changed => true,
                CandidateWriteOutcome::Unchanged => false,
            }
        } else {
            false
        };
        if !learning_candidate_needs_refresh(existing, &candidate) {
            return Ok(if superseded {
                CandidateWriteOutcome::Changed
            } else {
                CandidateWriteOutcome::Unchanged
            });
        }
        if !storage
            .upsert_learning_candidate_guarded(
                LEARNING_CANDIDATE_GENERATION_LEASE_KEY,
                lease_guard,
                &candidate,
            )
            .await?
        {
            return Ok(CandidateWriteOutcome::LeaseLost);
        }
        return Ok(CandidateWriteOutcome::Changed);
    }

    if existing_candidates.is_empty() {
        if !storage
            .upsert_learning_candidate_guarded(
                LEARNING_CANDIDATE_GENERATION_LEASE_KEY,
                lease_guard,
                &candidate,
            )
            .await?
        {
            return Ok(CandidateWriteOutcome::LeaseLost);
        }
        return Ok(CandidateWriteOutcome::Changed);
    }

    if let Some(existing_draft) = existing_candidates
        .iter()
        .find(|existing| learning_candidate_is_draft(&existing.approval_status))
    {
        candidate.id = existing_draft.id.clone();
        candidate.created_at = existing_draft.created_at.clone();
        candidate.approval_status = existing_draft.approval_status.clone();
        candidate.review_notes = existing_draft.review_notes.clone();
        candidate.reviewed_at = existing_draft.reviewed_at.clone();
        candidate.approved_ref = existing_draft.approved_ref.clone();
        if !storage
            .upsert_learning_candidate_guarded(
                LEARNING_CANDIDATE_GENERATION_LEASE_KEY,
                lease_guard,
                &candidate,
            )
            .await?
        {
            return Ok(CandidateWriteOutcome::LeaseLost);
        }
        if let Some(reviewed_candidate) = existing_candidates
            .iter()
            .find(|candidate| learning_candidate_is_reviewed(&candidate.approval_status))
        {
            if matches!(
                supersede_stale_draft_candidates(
                    storage,
                    lease_guard,
                    &existing_candidates,
                    Some(&existing_draft.id),
                    &reviewed_candidate.id,
                )
                .await?,
                CandidateWriteOutcome::LeaseLost
            ) {
                return Ok(CandidateWriteOutcome::LeaseLost);
            }
        }
        return Ok(CandidateWriteOutcome::Changed);
    }

    if let Some(reviewed_candidate) = existing_candidates
        .iter()
        .find(|existing| learning_candidate_is_reviewed(&existing.approval_status))
    {
        let note = format!(
            "Auto-reopened for review after material change from reviewed candidate `{}`.",
            reviewed_candidate.id
        );
        candidate.summary = Some(match candidate.summary.take() {
            Some(summary) if !summary.trim().is_empty() => format!("{summary} {note}"),
            _ => note,
        });
    }
    let revision_id = learning_candidate_revision_id(&candidate, &material_signature);
    candidate.id = if existing_candidates
        .iter()
        .any(|existing| existing.id == revision_id)
    {
        let reopen_marker = chrono::Utc::now().to_rfc3339();
        stable_id(
            "candidate",
            &[
                candidate.candidate_type.as_str(),
                candidate.subject_key.as_str(),
                material_signature.as_str(),
                reopen_marker.as_str(),
            ],
        )
    } else {
        revision_id
    };
    if !storage
        .upsert_learning_candidate_guarded(
            LEARNING_CANDIDATE_GENERATION_LEASE_KEY,
            lease_guard,
            &candidate,
        )
        .await?
    {
        return Ok(CandidateWriteOutcome::LeaseLost);
    }
    Ok(CandidateWriteOutcome::Changed)
}

fn apply_candidate_write_outcome(
    outcome: CandidateWriteOutcome,
    generated: &mut usize,
    lease_alive: &std::sync::Arc<std::sync::atomic::AtomicBool>,
) -> bool {
    match outcome {
        CandidateWriteOutcome::Changed => {
            *generated += 1;
            true
        }
        CandidateWriteOutcome::Unchanged => true,
        CandidateWriteOutcome::LeaseLost => {
            lease_alive.store(false, std::sync::atomic::Ordering::Relaxed);
            tracing::warn!("Stopping learning candidate generation after lease ownership was lost");
            false
        }
    }
}

pub async fn run_candidate_generation(storage: &Storage, data_dir: &Path) -> Result<usize> {
    if !load_learning_enabled(storage).await {
        return Ok(0);
    }
    let lease_owner = uuid::Uuid::new_v4().to_string();
    let Some(lease_guard) = storage
        .acquire_kv_lease_guard(
            LEARNING_CANDIDATE_GENERATION_LEASE_KEY,
            &lease_owner,
            LEARNING_CANDIDATE_GENERATION_LEASE_TTL_SECS,
        )
        .await?
    else {
        return Ok(0);
    };
    let lease_alive = std::sync::Arc::new(std::sync::atomic::AtomicBool::new(true));
    let heartbeat_storage = storage.clone();
    let heartbeat_guard = lease_guard.clone();
    let heartbeat_lease_alive = lease_alive.clone();
    let (lease_stop_tx, mut lease_stop_rx) = tokio::sync::watch::channel(false);
    let lease_heartbeat = tokio::spawn(async move {
        loop {
            tokio::select! {
                changed = lease_stop_rx.changed() => {
                    if changed.is_err() || *lease_stop_rx.borrow() {
                        break;
                    }
                }
                _ = tokio::time::sleep(std::time::Duration::from_secs(
                    LEARNING_CANDIDATE_GENERATION_LEASE_HEARTBEAT_SECS,
                )) => {
                    match heartbeat_storage
                        .refresh_kv_lease_guard(
                            LEARNING_CANDIDATE_GENERATION_LEASE_KEY,
                            &heartbeat_guard,
                            LEARNING_CANDIDATE_GENERATION_LEASE_TTL_SECS,
                        )
                        .await
                    {
                        Ok(true) => {}
                        Ok(false) => {
                            heartbeat_lease_alive
                                .store(false, std::sync::atomic::Ordering::Relaxed);
                            tracing::warn!(
                                "Learning candidate generation lease heartbeat lost ownership"
                            );
                            break;
                        }
                        Err(error) => {
                            heartbeat_lease_alive
                                .store(false, std::sync::atomic::Ordering::Relaxed);
                            tracing::warn!(
                                "Learning candidate generation lease heartbeat refresh failed: {}",
                                error
                            );
                            break;
                        }
                    }
                }
            }
        }
    });

    let result = async {
        let cap = load_learning_queue_cap(storage).await as u64;
        let skill_catalog = skill_evolution::load_skill_catalog(data_dir).unwrap_or_default();
        let known_skill_names = skill_catalog
            .iter()
            .map(|entry| entry.name.trim().to_ascii_lowercase())
            .collect::<HashSet<_>>();
        let recent_runs = storage
            .list_recent_experience_runs_any_scope(cap.max(192))
            .await
            .unwrap_or_default();
        let patterns = storage
            .list_candidate_ready_patterns(3, 0.66, cap)
            .await?
            .into_iter()
            .filter(|pattern| !procedural_pattern_is_external_source(pattern))
            .collect::<Vec<_>>();
        let mut generated = 0usize;
        for pattern in patterns {
            if !lease_alive.load(std::sync::atomic::Ordering::Relaxed) {
                tracing::warn!(
                    "Stopping learning candidate generation after lease ownership was lost"
                );
                break;
            }

            let steps = pattern
                .steps_json
                .as_array()
                .map(|items| {
                    items
                        .iter()
                        .filter_map(|item| item.as_str().map(|value| value.to_string()))
                        .collect::<Vec<_>>()
                })
                .unwrap_or_default();
            let action_name = candidate_action_name(&pattern);
            let workflow_content = workflow_candidate_markdown(&pattern, &action_name, &steps);
            let tool_names = pattern
                .tool_sequence_json
                .as_array()
                .map(|items| {
                    items
                        .iter()
                        .filter_map(|item| item.as_str().map(|value| value.to_string()))
                        .collect::<Vec<_>>()
                })
                .unwrap_or_default();
            if known_skill_names.contains(&action_name.to_ascii_lowercase())
                || tool_names
                    .iter()
                    .any(|name| known_skill_names.contains(&name.trim().to_ascii_lowercase()))
            {
                continue;
            }
            let workflow_candidate_id = stable_id(
                "candidate",
                &[
                    "skill_patch_create",
                    pattern.id.as_str(),
                    short_hash(&[workflow_content.as_str()]).as_str(),
                ],
            );
            let now = chrono::Utc::now().to_rfc3339();
            if !apply_candidate_write_outcome(
                upsert_generated_learning_candidate(
                    storage,
                    &lease_guard,
                    learning_candidate::Model {
                        id: workflow_candidate_id,
                        candidate_type: "skill_patch".to_string(),
                        subject_key: action_name.clone(),
                        title: format!("Skill candidate: {}", pattern.title),
                        summary: Some("Generated from repeated successful procedures.".to_string()),
                        project_id: pattern.project_id.clone(),
                        conversation_id: pattern.conversation_id.clone(),
                        pattern_id: Some(pattern.id.clone()),
                        evidence_refs: json!([pattern.id]),
                        proposed_content: build_skill_patch_candidate_content(
                            "create_skill",
                            &action_name,
                            "custom",
                            "",
                            &workflow_content,
                            "Create a new reusable skill from a repeated successful procedure.",
                            json!({
                                "pattern_id": pattern.id,
                                "sample_count": pattern.sample_count,
                                "success_count": pattern.success_count,
                                "success_rate": pattern.success_rate,
                                "task_type": pattern
                                    .metadata
                                    .get("task_type")
                                    .and_then(|value| value.as_str())
                                    .unwrap_or("general"),
                                "tool_sequence": tool_names,
                                "trigger_summary": pattern.trigger_summary,
                            }),
                            SkillMetricsSnapshot::default(),
                            0,
                        ),
                        confidence: pattern.success_rate,
                        approval_status: "draft".to_string(),
                        review_notes: None,
                        reviewed_at: None,
                        approved_ref: None,
                        created_at: now.clone(),
                        updated_at: now.clone(),
                    },
                )
                .await?,
                &mut generated,
                &lease_alive,
            ) {
                break;
            }

            let metadata = pattern.metadata.as_object().cloned().unwrap_or_default();
            let task_type = metadata
                .get("task_type")
                .and_then(|value| value.as_str())
                .unwrap_or("general");
            let strategy_profile =
                build_strategy_candidate_profile(&pattern, task_type, &tool_names);
            let strategy_candidate_id = stable_id("candidate", &["strategy", pattern.id.as_str()]);
            if !apply_candidate_write_outcome(
                upsert_generated_learning_candidate(
                    storage,
                    &lease_guard,
                    learning_candidate::Model {
                        id: strategy_candidate_id,
                        candidate_type: "strategy".to_string(),
                        subject_key: pattern.id.clone(),
                        title: format!("Strategy candidate: {}", pattern.title),
                        summary: Some("Generated from high-confidence procedural patterns.".to_string()),
                        project_id: pattern.project_id.clone(),
                        conversation_id: pattern.conversation_id.clone(),
                        pattern_id: Some(pattern.id.clone()),
                        evidence_refs: json!([pattern.id]),
                        proposed_content: serde_json::to_value(strategy_profile).unwrap_or(Value::Null),
                        confidence: (pattern.success_rate * 0.92).min(0.98),
                        approval_status: "draft".to_string(),
                        review_notes: None,
                        reviewed_at: None,
                        approved_ref: None,
                        created_at: now.clone(),
                        updated_at: now.clone(),
                    },
                )
                .await?,
                &mut generated,
                &lease_alive,
            ) {
                break;
            }
        }

        let reflected_lessons = storage
            .list_active_experience_items(&["lesson"], None, None, cap)
            .await?
            .into_iter()
            .filter(experience_item_is_reflected_heuristic)
            .collect::<Vec<_>>();
        let mut heuristic_groups: HashMap<String, Vec<experience_item::Model>> = HashMap::new();
        for lesson in reflected_lessons {
            if lesson.support_count < 2 || reflected_heuristic_confidence(&lesson) < 0.72 {
                continue;
            }
            let task_type = reflected_heuristic_task_type(&lesson).unwrap_or("general");
            let subject_key = format!(
                "heuristic_strategy::{}::{}::{}::{}",
                lesson.scope,
                lesson.project_id.as_deref().unwrap_or(""),
                lesson.conversation_id.as_deref().unwrap_or(""),
                task_type
            );
            heuristic_groups.entry(subject_key).or_default().push(lesson);
        }
        for (subject_key, mut lessons) in heuristic_groups {
            if !lease_alive.load(std::sync::atomic::Ordering::Relaxed) {
                tracing::warn!(
                    "Stopping learning candidate generation after lease ownership was lost"
                );
                break;
            }
            lessons.sort_by(|left, right| {
                reflected_heuristic_confidence(right)
                    .total_cmp(&reflected_heuristic_confidence(left))
                    .then_with(|| right.support_count.cmp(&left.support_count))
                    .then_with(|| right.updated_at.cmp(&left.updated_at))
            });

            let mut selected = Vec::new();
            let mut seen = HashSet::new();
            for lesson in lessons {
                let dedupe_key = normalize_semantic_text(&lesson.content);
                if !seen.insert(dedupe_key) {
                    continue;
                }
                selected.push(lesson);
                if selected.len() >= 4 {
                    break;
                }
            }
            if selected.is_empty() {
                continue;
            }

            let task_type = reflected_heuristic_task_type(&selected[0]).unwrap_or("general");
            let avg_confidence = selected
                .iter()
                .map(reflected_heuristic_confidence)
                .sum::<f64>()
                / selected.len() as f64;
            let strategy_profile =
                build_reflected_heuristic_strategy_profile(task_type, &selected);
            let evidence_refs = Value::Array(
                selected
                    .iter()
                    .map(|lesson| Value::String(lesson.id.clone()))
                    .collect(),
            );
            let now = chrono::Utc::now().to_rfc3339();
            if !apply_candidate_write_outcome(
                upsert_generated_learning_candidate(
                    storage,
                    &lease_guard,
                    learning_candidate::Model {
                        id: stable_id("candidate", &["strategy", subject_key.as_str()]),
                        candidate_type: "strategy".to_string(),
                        subject_key: subject_key.clone(),
                        title: format!("Strategy candidate from reflected heuristics: {}", task_type),
                        summary: Some(
                            "Generated from repeated reflected heuristics in the background learning loop."
                                .to_string(),
                        ),
                        project_id: selected[0].project_id.clone(),
                        conversation_id: selected[0].conversation_id.clone(),
                        pattern_id: None,
                        evidence_refs,
                        proposed_content: serde_json::to_value(strategy_profile)
                            .unwrap_or(Value::Null),
                        confidence: avg_confidence.clamp(0.45, 0.98),
                        approval_status: "draft".to_string(),
                        review_notes: None,
                        reviewed_at: None,
                        approved_ref: None,
                        created_at: now.clone(),
                        updated_at: now,
                    },
                )
                .await?,
                &mut generated,
                &lease_alive,
            ) {
                break;
            }
        }

        for skill in skill_catalog {
            if !lease_alive.load(std::sync::atomic::Ordering::Relaxed) {
                tracing::warn!(
                    "Stopping learning candidate generation after lease ownership was lost"
                );
                break;
            }

            let matched_runs = recent_runs
                .iter()
                .filter(|run| skill_evolution::skill_matches_run(run, &skill.name))
                .collect::<Vec<_>>();
            if matched_runs.len() < 3 {
                continue;
            }

            let baseline = skill_evolution::compute_skill_metrics(
                &recent_runs,
                &skill.name,
                None,
                SkillWindowDirection::Baseline,
                8,
                64,
            );
            let selected_only_failures = recent_runs
                .iter()
                .filter(|run| {
                    skill_evolution::skill_selected_by_run(run, &skill.name)
                        && !skill_evolution::skill_executed_by_run(run, &skill.name)
                        && (run.correction_state == "corrected" || run.success_state == "failed")
                })
                .collect::<Vec<_>>();
            let executed_failures = recent_runs
                .iter()
                .filter(|run| {
                    skill_evolution::skill_executed_by_run(run, &skill.name)
                        && (run.correction_state == "corrected" || run.success_state == "failed")
                })
                .collect::<Vec<_>>();

            let mut action = None::<&str>;
            let mut diff_summary = None::<String>;
            let mut updated_content = None::<String>;
            let history = skill_evolution::load_skill_history(&skill.history_dir).unwrap_or_default();

            if selected_only_failures.len() >= 2 {
                let description = skill_evolution::extract_frontmatter_value(&skill.content, "description")
                    .unwrap_or_else(|| skill.description.clone());
                if let Some(next_description) = skill_evolution::append_not_for_clause(
                    &description,
                    &candidate_exclusion_labels(&selected_only_failures),
                ) {
                    let content = skill_evolution::replace_frontmatter_value(
                        &skill.content,
                        "description",
                        &next_description,
                    );
                    if content != skill.content {
                        action = Some("optimize_description");
                        diff_summary = Some(
                            "Tighten the trigger description so the skill stops matching known out-of-scope requests."
                                .to_string(),
                        );
                        updated_content = Some(content);
                    }
                }
            }

            if action.is_none()
                && (executed_failures.len() >= 2
                    || baseline.failure_rate >= 0.34
                    || baseline.tool_error_rate >= 0.25)
            {
                if let Some(section_body) =
                    candidate_failure_checks_section(&executed_failures, &selected_only_failures)
                {
                    let content = skill_evolution::upsert_markdown_section(
                        &skill.content,
                        "## Common failure checks",
                        &section_body,
                    );
                    if content != skill.content {
                        action = Some("improve_skill");
                        diff_summary = Some(
                            "Add a focused failure-check section based on recent corrected and failed runs."
                                .to_string(),
                        );
                        updated_content = Some(content);
                    }
                }
            }

            let (Some(action), Some(diff_summary), Some(after_content)) =
                (action, diff_summary, updated_content)
            else {
                continue;
            };

            let evidence_refs = recent_runs
                .iter()
                .filter(|run| {
                    skill_evolution::skill_matches_run(run, &skill.name)
                        && (run.correction_state == "corrected" || run.success_state == "failed")
                })
                .take(6)
                .map(|run| Value::String(run.id.clone()))
                .collect::<Vec<_>>();
            let candidate_id = stable_id(
                "candidate",
                &[
                    "skill_patch",
                    skill.name.as_str(),
                    action,
                    short_hash(&[after_content.as_str()]).as_str(),
                ],
            );
            let now = chrono::Utc::now().to_rfc3339();
            if !apply_candidate_write_outcome(
                upsert_generated_learning_candidate(
                    storage,
                    &lease_guard,
                    learning_candidate::Model {
                        id: candidate_id,
                        candidate_type: "skill_patch".to_string(),
                        subject_key: skill.name.clone(),
                        title: format!("Skill patch: {}", skill.name),
                        summary: Some(diff_summary.clone()),
                        project_id: None,
                        conversation_id: None,
                        pattern_id: None,
                        evidence_refs: Value::Array(evidence_refs),
                        proposed_content: build_skill_patch_candidate_content(
                            action,
                            &skill.name,
                            &skill.source,
                            &skill.content,
                            &after_content,
                            &diff_summary,
                            json!({
                                "selected_only_failures": selected_only_failures.len(),
                                "executed_failures": executed_failures.len(),
                                "matched_runs": matched_runs.len(),
                                "baseline": baseline,
                                "recent_failure_reasons": executed_failures
                                    .iter()
                                    .filter_map(|run| run_failure_summary(run))
                                    .take(4)
                                    .collect::<Vec<_>>(),
                                "recent_tool_errors": executed_failures
                                    .iter()
                                    .flat_map(|run| {
                                        run.tool_sequence_json
                                            .as_array()
                                            .into_iter()
                                            .flatten()
                                            .filter(|item| {
                                                item.get("status")
                                                    .and_then(|value| value.as_str())
                                                    .map(|status| status != "success")
                                                    .unwrap_or(false)
                                            })
                                            .filter_map(|item| item.get("tool_name").and_then(|value| value.as_str()))
                                            .map(|value| value.to_string())
                                            .collect::<Vec<_>>()
                                    })
                                    .take(6)
                                    .collect::<Vec<_>>(),
                                "selected_failure_examples": selected_only_failures
                                    .iter()
                                    .take(3)
                                    .map(|run| run_request_preview(run))
                                    .collect::<Vec<_>>(),
                                "history_versions_read": history.len(),
                            }),
                            baseline,
                            history.len(),
                        ),
                        confidence: ((matched_runs.len().min(8) as f64) / 8.0).clamp(0.35, 0.95),
                        approval_status: "draft".to_string(),
                        review_notes: None,
                        reviewed_at: None,
                        approved_ref: None,
                        created_at: now.clone(),
                        updated_at: now,
                    },
                )
                .await?,
                &mut generated,
                &lease_alive,
            ) {
                break;
            }
        }

        let at_risk_procedures = storage
            .list_active_experience_items(&["procedure"], None, None, cap)
            .await?
            .into_iter()
            .filter(|item| !experience_item_is_external_source(item))
            .collect::<Vec<_>>();
        for item in at_risk_procedures
            .into_iter()
            .filter(|item| item.contradiction_count > item.support_count && item.status == "active")
        {
            if !lease_alive.load(std::sync::atomic::Ordering::Relaxed) {
                tracing::warn!(
                    "Stopping learning candidate generation after lease ownership was lost"
                );
                break;
            }

            let now = chrono::Utc::now().to_rfc3339();
            if !apply_candidate_write_outcome(
                upsert_generated_learning_candidate(
                    storage,
                    &lease_guard,
                    learning_candidate::Model {
                        id: stable_id("candidate", &["memory_deprecate", item.id.as_str()]),
                        candidate_type: "memory_deprecate".to_string(),
                        subject_key: item.id.clone(),
                        title: format!("Deprecate stale procedure: {}", item.title),
                        summary: Some(
                            "The contradiction count has overtaken positive support for this procedure."
                                .to_string(),
                        ),
                        project_id: item.project_id.clone(),
                        conversation_id: item.conversation_id.clone(),
                        pattern_id: None,
                        evidence_refs: json!([item.id]),
                        proposed_content: json!({
                            "item_id": item.id,
                            "next_status": "deprecated",
                        }),
                        confidence: 0.74,
                        approval_status: "draft".to_string(),
                        review_notes: None,
                        reviewed_at: None,
                        approved_ref: None,
                        created_at: now.clone(),
                        updated_at: now.clone(),
                    },
                )
                .await?,
                &mut generated,
                &lease_alive,
            ) {
                break;
            }
        }

        let mergeable_items = storage
            .list_active_experience_items(
                &["constraint", "personal_fact", "lesson", "procedure"],
                None,
                None,
                cap,
            )
            .await?
            .into_iter()
            .filter(|item| !experience_item_is_external_source(item))
            .collect::<Vec<_>>();
        let mut merge_groups: std::collections::HashMap<String, Vec<experience_item::Model>> =
            std::collections::HashMap::new();
        for item in mergeable_items {
            let Some(signature) = canonical_memory_merge_signature(&item) else {
                continue;
            };
            merge_groups.entry(signature).or_default().push(item);
        }
        for group in merge_groups.into_values() {
            if !lease_alive.load(std::sync::atomic::Ordering::Relaxed) {
                tracing::warn!(
                    "Stopping learning candidate generation after lease ownership was lost"
                );
                break;
            }
            if group.len() < 2 {
                continue;
            }
            let mut sorted = group;
            sorted.sort_by_key(|item| std::cmp::Reverse(memory_merge_sort_key(item)));
            let target = sorted[0].clone();
            for source in sorted.into_iter().skip(1) {
                if !lease_alive.load(std::sync::atomic::Ordering::Relaxed) {
                    tracing::warn!(
                        "Stopping learning candidate generation after lease ownership was lost"
                    );
                    break;
                }
                if source.id == target.id {
                    continue;
                }
                let now = chrono::Utc::now().to_rfc3339();
                if !apply_candidate_write_outcome(
                    upsert_generated_learning_candidate(
                        storage,
                        &lease_guard,
                        learning_candidate::Model {
                            id: stable_id(
                                "candidate",
                                &["memory_merge", source.id.as_str(), target.id.as_str()],
                            ),
                            candidate_type: "memory_merge".to_string(),
                            subject_key: target.id.clone(),
                            title: format!("Merge duplicate memory into {}", target.title),
                            summary: Some(
                                "Two active memories carry substantially the same content and can be merged."
                                    .to_string(),
                            ),
                            project_id: target.project_id.clone(),
                            conversation_id: target.conversation_id.clone(),
                            pattern_id: None,
                            evidence_refs: json!([target.id, source.id]),
                            proposed_content: json!({
                                "target_item_id": target.id,
                                "source_item_id": source.id,
                                "reason": "duplicate_content",
                            }),
                            confidence: ((target.confidence + source.confidence) / 2.0).min(0.96),
                            approval_status: "draft".to_string(),
                            review_notes: None,
                            reviewed_at: None,
                            approved_ref: None,
                            created_at: now.clone(),
                            updated_at: now.clone(),
                        },
                    )
                    .await?,
                    &mut generated,
                    &lease_alive,
                ) {
                    break;
                }
            }
        }

        Ok(generated)
    }
    .await;
    let _ = lease_stop_tx.send(true);
    if let Err(error) = lease_heartbeat.await {
        tracing::warn!(
            "Learning candidate generation lease heartbeat join failed: {}",
            error
        );
    }

    if let Err(error) = storage
        .release_kv_lease_guard(LEARNING_CANDIDATE_GENERATION_LEASE_KEY, &lease_guard)
        .await
    {
        tracing::warn!(
            "Failed to release learning candidate generation lease: {}",
            error
        );
    }

    result
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::storage::entities::operational_log;

    #[test]
    fn derive_intent_key_is_task_aware_and_stable() {
        let key = derive_intent_key(
            "Please fix the Rust bug in the tool execution flow",
            "coding",
        );
        assert!(key.starts_with("coding::"));
        assert!(key.contains("fix"));
        assert!(key.contains("rust"));
    }

    #[test]
    fn candidate_action_name_is_slugged() {
        let pattern = procedural_pattern::Model {
            id: "pattern-123".to_string(),
            intent_key: "coding::fix-tool-bug".to_string(),
            scope: "project".to_string(),
            project_id: None,
            conversation_id: None,
            title: "Fix Tool Bug / Flow".to_string(),
            trigger_summary: String::new(),
            summary: String::new(),
            tool_sequence_digest: None,
            steps_json: Value::Array(Vec::new()),
            tool_sequence_json: Value::Array(Vec::new()),
            sample_count: 3,
            success_count: 3,
            correction_count: 0,
            success_rate: 1.0,
            last_validated_at: None,
            status: "active".to_string(),
            metadata: Value::Null,
            created_at: "2026-01-01T00:00:00Z".to_string(),
            updated_at: "2026-01-01T00:00:00Z".to_string(),
        };
        let action_name = candidate_action_name(&pattern);
        assert!(action_name.starts_with("learned-fix-tool-bug-flow"));
        assert!(
            action_name
                .chars()
                .all(|ch| ch.is_ascii_lowercase() || ch.is_ascii_digit() || ch == '-')
        );
    }

    #[test]
    fn describe_user_preference_memory_maps_name_to_personal_fact() {
        let mapped = describe_user_preference_memory("user_name", "Ava")
            .expect("user_name should map to a personal fact");
        assert_eq!(mapped.0, "personal_fact");
        assert!(mapped.2.contains("Ava"));
    }

    #[test]
    fn canonical_memory_merge_signature_normalizes_equivalent_content() {
        let base = experience_item::Model {
            id: "item-1".to_string(),
            kind: "constraint".to_string(),
            scope: "global".to_string(),
            project_id: None,
            conversation_id: None,
            title: "Constraint".to_string(),
            content: "Require explicit approval before side-effecting actions.".to_string(),
            normalized_key: "constraint::one".to_string(),
            confidence: 0.9,
            support_count: 2,
            contradiction_count: 0,
            status: "active".to_string(),
            metadata: Value::Null,
            last_supported_at: None,
            last_contradicted_at: None,
            created_at: "2026-01-01T00:00:00Z".to_string(),
            updated_at: "2026-01-01T00:00:00Z".to_string(),
            embedding: None,
        };
        let mut variant = base.clone();
        variant.id = "item-2".to_string();
        variant.content = "Require explicit approval before side effecting actions".to_string();

        assert_eq!(
            canonical_memory_merge_signature(&base),
            canonical_memory_merge_signature(&variant)
        );
    }

    #[test]
    fn external_source_metadata_is_fenced_from_learning() {
        let item = experience_item::Model {
            id: "item-ext".to_string(),
            kind: "lesson".to_string(),
            scope: "global".to_string(),
            project_id: None,
            conversation_id: None,
            title: "External insight".to_string(),
            content: "Community learning".to_string(),
            normalized_key: "lesson::external".to_string(),
            confidence: 0.6,
            support_count: 1,
            contradiction_count: 0,
            status: "active".to_string(),
            metadata: serde_json::json!({
                "source": "moltbook",
                "tags": ["external-source", "community-learning", "learning-fenced"],
                "fenced_external_source": true
            }),
            last_supported_at: None,
            last_contradicted_at: None,
            created_at: "2026-01-01T00:00:00Z".to_string(),
            updated_at: "2026-01-01T00:00:00Z".to_string(),
            embedding: None,
        };
        assert!(experience_item_is_external_source(&item));
    }

    #[test]
    fn decision_episode_captures_recent_routing_evidence() {
        let logs = vec![
            operational_log::Model {
                id: "log-outcome".to_string(),
                created_at: "2026-04-14T01:03:00Z".to_string(),
                trace_id: Some("trace-1".to_string()),
                conversation_id: Some("conv-1".to_string()),
                channel: "web".to_string(),
                event_type: "response_complete".to_string(),
                success: true,
                outcome: "completed".to_string(),
                tool_name: None,
                latency_ms: Some(1200),
                arguments: None,
                payload: Some(r#"{"status":"completed","tool_calls":1}"#.to_string()),
                strategy_version: None,
                policy_version: None,
                prompt_version: None,
                model_slot: None,
            },
            operational_log::Model {
                id: "log-routing".to_string(),
                created_at: "2026-04-14T01:02:00Z".to_string(),
                trace_id: Some("trace-1".to_string()),
                conversation_id: Some("conv-1".to_string()),
                channel: "web".to_string(),
                event_type: "routing_decision".to_string(),
                success: true,
                outcome: "ok".to_string(),
                tool_name: None,
                latency_ms: Some(45),
                arguments: None,
                payload: Some(
                    r#"{"complexity":"Complex","needs_delegation":false,"mode":"direct"}"#
                        .to_string(),
                ),
                strategy_version: None,
                policy_version: None,
                prompt_version: None,
                model_slot: None,
            },
            operational_log::Model {
                id: "log-shape".to_string(),
                created_at: "2026-04-14T01:01:00Z".to_string(),
                trace_id: Some("trace-1".to_string()),
                conversation_id: Some("conv-1".to_string()),
                channel: "web".to_string(),
                event_type: "request_shape_assessment".to_string(),
                success: true,
                outcome: "classified".to_string(),
                tool_name: None,
                latency_ms: Some(12),
                arguments: None,
                payload: Some(
                    r#"{"shape":"app","execution_mode":"immediate","preferred_actions":["app_deploy"]}"#
                        .to_string(),
                ),
                strategy_version: None,
                policy_version: None,
                prompt_version: None,
                model_slot: None,
            },
        ];

        let episode = build_decision_episode(&logs).expect("decision episode");
        let payload = episode.as_object().expect("object");
        assert_eq!(
            payload
                .get("request_shape")
                .and_then(|value| value.get("payload"))
                .and_then(|value| value.get("shape"))
                .and_then(Value::as_str),
            Some("app")
        );
        assert_eq!(
            payload
                .get("routing")
                .and_then(|value| value.get("payload"))
                .and_then(|value| value.get("complexity"))
                .and_then(Value::as_str),
            Some("Complex")
        );
        assert_eq!(
            payload
                .get("outcome")
                .and_then(|value| value.get("payload"))
                .and_then(|value| value.get("status"))
                .and_then(Value::as_str),
            Some("completed")
        );
    }
}
