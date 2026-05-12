//! Router-layer learning contracts for ArkEvolve.
//!
//! This module intentionally models router learning as data-level candidates.
//! Runtime router code changes remain outside automatic promotion.

use serde::{Deserialize, Serialize};
use std::hash::{Hash, Hasher};

pub const ROUTER_LEARNING_CANDIDATE_TYPE: &str = "router_learning";
pub const ROUTER_LEARNING_SUBJECT_KEY: &str = "router_learning_v1";

#[derive(Debug, Clone, Copy, Serialize, Deserialize, PartialEq, Eq, PartialOrd, Ord)]
#[serde(rename_all = "snake_case")]
pub enum RouterLearningLayer {
    Canonical,
    ActionDescriptor,
    Benchmark,
    PromptFragment,
    Policy,
    CapabilityGraph,
}

#[derive(Debug, Clone, Copy, Serialize, Deserialize, PartialEq, Eq, PartialOrd, Ord)]
#[serde(rename_all = "snake_case")]
pub enum RouterLearningMetric {
    GoalRecall,
    RequirementCoverage,
    WrongToolRate,
    OverMutationRate,
    UnderActionRate,
    CapabilityGapHonesty,
    ResultHonesty,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct RouterLearningMetricDelta {
    pub metric: RouterLearningMetric,
    pub delta: f64,
}

#[derive(Debug, Clone, Serialize, Deserialize, Default)]
pub struct RouterLearningTraceEvidence {
    pub trace_id: String,
    #[serde(default)]
    pub user_message_preview: String,
    #[serde(default)]
    pub semantic_plan: Option<serde_json::Value>,
    #[serde(default)]
    pub plan_verification: Option<serde_json::Value>,
    #[serde(default)]
    pub capability_resolution: Option<serde_json::Value>,
    #[serde(default)]
    pub result_verification: Option<serde_json::Value>,
    #[serde(default)]
    pub router_budget: Option<serde_json::Value>,
    #[serde(default)]
    pub execution_policy: Option<serde_json::Value>,
    #[serde(default)]
    pub capability_snapshot: Option<serde_json::Value>,
    #[serde(default)]
    pub capability_health: Option<serde_json::Value>,
    #[serde(default)]
    pub model_schedule: Option<serde_json::Value>,
    #[serde(default)]
    pub selected_tool_names: Vec<String>,
    #[serde(default)]
    pub native_schema_count: Option<u64>,
    #[serde(default)]
    pub last_prompt_chars: Option<u64>,
    #[serde(default)]
    pub direct_response_without_tool: bool,
    #[serde(default)]
    pub trace_issue_count: usize,
    #[serde(default)]
    pub trace_issue_summaries: Vec<String>,
    #[serde(default)]
    pub failed_layer: Option<String>,
    #[serde(default)]
    pub failure_summary: Option<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize, Default)]
pub struct RouterLearningCandidatePayload {
    pub candidate_type: String,
    pub router_layer: String,
    pub objective: String,
    #[serde(default)]
    pub evidence: Vec<RouterLearningTraceEvidence>,
    #[serde(default)]
    pub metric_deltas: Vec<RouterLearningMetricDelta>,
    #[serde(default)]
    pub proposed_canonical_payload: Option<serde_json::Value>,
    #[serde(default)]
    pub proposed_action_descriptor_patch: Option<serde_json::Value>,
    #[serde(default)]
    pub proposed_benchmark_entries: Vec<serde_json::Value>,
    #[serde(default)]
    pub proposed_prompt_fragment_bundle: Option<serde_json::Value>,
    #[serde(default)]
    pub proposed_policy_patch: Option<serde_json::Value>,
    #[serde(default)]
    pub proposed_capability_graph_patch: Option<serde_json::Value>,
}

pub fn router_learning_benchmark_profile() -> serde_json::Value {
    serde_json::json!({
        "candidate_type": ROUTER_LEARNING_CANDIDATE_TYPE,
        "subject_key": ROUTER_LEARNING_SUBJECT_KEY,
        "objective": "Improve semantic router behavior through layer-specific data changes under replay/canary gates.",
        "allowed_layers": [
            "canonical",
            "action_descriptor",
            "benchmark",
            "prompt_fragment",
            "policy",
            "capability_graph"
        ],
        "metrics": [
            "goal_recall",
            "requirement_coverage",
            "wrong_tool_rate",
            "over_mutation_rate",
            "under_action_rate",
            "capability_gap_honesty",
            "result_honesty"
        ],
        "required_trace_evidence": [
            "semantic_plan",
            "plan_verification",
            "capability_resolution",
            "result_verification",
            "router_budget"
        ],
        "trace_evidence_notes": {
            "router_budget": "Includes execution policy, selected compact tool cards, native schema count, prompt size, and capability snapshot cache/skipped state.",
            "direct_response_without_tool": "True when the current turn was answered by semantic no-tool routing. This is per-turn evidence, never a sticky conversation mode.",
            "capability_snapshot": "May be null when a pure conversational turn intentionally skipped catalog/graph loading."
        },
        "promotion_rules": {
            "prefer_data_before_prompt": true,
            "runtime_code_auto_promotion": false,
            "must_identify_failed_layer": true,
            "must_include_replayable_evidence": true,
            "must_preserve_multi_outcome_turns": true,
            "must_avoid_phrase_specific_routing": true
        }
    })
}

pub fn validate_router_learning_candidate(
    payload: &RouterLearningCandidatePayload,
) -> Result<(), String> {
    if payload.candidate_type.trim() != ROUTER_LEARNING_CANDIDATE_TYPE {
        return Err("router learning candidate has unsupported candidate_type".to_string());
    }
    if parse_router_learning_layer(&payload.router_layer).is_none() {
        return Err("router learning candidate has unsupported router_layer".to_string());
    }
    if payload.objective.trim().is_empty() {
        return Err("router learning candidate requires an objective".to_string());
    }
    if payload.evidence.is_empty() {
        return Err("router learning candidate requires replayable trace evidence".to_string());
    }
    if payload
        .evidence
        .iter()
        .any(|evidence| evidence.failed_layer.as_deref().unwrap_or_default().trim().is_empty())
    {
        return Err("each router learning evidence item must identify the failed layer".to_string());
    }
    if payload.evidence.iter().any(|evidence| {
        evidence
            .failed_layer
            .as_deref()
            .and_then(parse_router_learning_layer)
            .is_none()
    }) {
        return Err(
            "each router learning failed_layer must map to a supported router layer".to_string(),
        );
    }
    if payload
        .evidence
        .iter()
        .any(|evidence| !evidence.has_router_artifact())
    {
        return Err(
            "each router learning evidence item must include structured router trace evidence"
                .to_string(),
        );
    }
    Ok(())
}

pub(crate) async fn maybe_upsert_router_replay_candidate_from_trace(
    storage: &crate::storage::Storage,
    trace: &crate::core::ExecutionTrace,
) -> anyhow::Result<Option<String>> {
    let mut evidence = trace_evidence_from_semantic_steps(
        trace.id.clone(),
        crate::security::redact_pii(&safe_trace_preview(&trace.message, 1_200)),
        &trace.steps,
    );
    if !evidence.has_router_artifact() || !router_trace_needs_replay_review(&evidence) {
        return Ok(None);
    }
    evidence.failed_layer = Some("benchmark".to_string());
    evidence.failure_summary = Some(router_trace_failure_summary(&evidence));
    let Some(entry) = router_replay_benchmark_entry_from_evidence(&evidence) else {
        return Ok(None);
    };

    let payload = RouterLearningCandidatePayload {
        candidate_type: ROUTER_LEARNING_CANDIDATE_TYPE.to_string(),
        router_layer: "benchmark".to_string(),
        objective:
            "Add a reviewed replay benchmark case from a real router trace so the failure is tested semantically before future router changes."
                .to_string(),
        evidence: vec![evidence.clone()],
        proposed_benchmark_entries: vec![entry],
        metric_deltas: vec![RouterLearningMetricDelta {
            metric: RouterLearningMetric::RequirementCoverage,
            delta: 0.0,
        }],
        ..Default::default()
    };
    validate_router_learning_candidate(&payload)
        .map_err(|error| anyhow::anyhow!("router replay candidate validation failed: {error}"))?;

    let now = chrono::Utc::now().to_rfc3339();
    let candidate_id = stable_router_replay_candidate_id(&trace.id);
    let model = crate::storage::entities::learning_candidate::Model {
        id: candidate_id.clone(),
        candidate_type: ROUTER_LEARNING_CANDIDATE_TYPE.to_string(),
        subject_key: ROUTER_LEARNING_SUBJECT_KEY.to_string(),
        title: "Router replay case from real trace".to_string(),
        summary: Some(router_trace_failure_summary(&evidence)),
        project_id: None,
        conversation_id: None,
        pattern_id: Some(trace.id.clone()),
        evidence_refs: serde_json::json!({
            "trace_ids": [&trace.id],
            "source": "execution_trace",
            "review_required": true
        }),
        proposed_content: serde_json::to_value(&payload)?,
        confidence: 0.72,
        approval_status: "pending".to_string(),
        review_notes: None,
        reviewed_at: None,
        approved_ref: None,
        created_at: now.clone(),
        updated_at: now,
    };
    storage.upsert_learning_candidate(&model).await?;
    Ok(Some(candidate_id))
}

#[allow(dead_code)]
pub fn trace_evidence_from_semantic_steps(
    trace_id: impl Into<String>,
    user_message_preview: impl Into<String>,
    steps: &[crate::core::ExecutionStep],
) -> RouterLearningTraceEvidence {
    let mut evidence = RouterLearningTraceEvidence {
        trace_id: trace_id.into(),
        user_message_preview: user_message_preview.into(),
        ..Default::default()
    };
    for step in steps {
        if step_type_is_execution_issue(&step.step_type) {
            evidence.trace_issue_count = evidence.trace_issue_count.saturating_add(1);
            if evidence.trace_issue_summaries.len() < 6 {
                evidence.trace_issue_summaries.push(safe_trace_preview(
                    &format!("{}: {}", step.title, step.detail),
                    360,
                ));
            }
        }
        let Some(data) = step
            .data
            .as_deref()
            .and_then(|raw| serde_json::from_str::<serde_json::Value>(raw).ok())
        else {
            continue;
        };
        if looks_like_router_budget(&data) {
            evidence.execution_policy = data.get("policy").cloned();
            evidence.capability_snapshot = data.get("capability_snapshot").cloned();
            evidence.capability_health = data.get("capability_health").cloned();
            evidence.model_schedule = data.get("model_schedule").cloned();
            evidence.selected_tool_names = data
                .get("selected_tool_names")
                .and_then(|value| value.as_array())
                .map(|items| {
                    items
                        .iter()
                        .filter_map(|item| item.as_str())
                        .map(str::trim)
                        .filter(|item| !item.is_empty())
                        .map(ToString::to_string)
                        .collect::<Vec<_>>()
                })
                .unwrap_or_default();
            evidence.native_schema_count = data
                .get("native_schema_count")
                .and_then(|value| value.as_u64());
            evidence.last_prompt_chars =
                data.get("last_prompt_chars").and_then(|value| value.as_u64());
            evidence.router_budget = Some(data);
            continue;
        }
        if looks_like_semantic_plan(&data) {
            evidence.semantic_plan = Some(data);
            continue;
        }
        if looks_like_plan_verification(&data) {
            evidence.plan_verification = Some(data);
            continue;
        }
        if looks_like_capability_resolution(&data) {
            evidence.direct_response_without_tool = capability_resolution_is_direct(&data);
            evidence.capability_resolution = Some(data);
            continue;
        }
        if looks_like_result_verification(&data) {
            evidence.result_verification = Some(data);
        }
    }
    evidence
}

impl RouterLearningTraceEvidence {
    fn has_router_artifact(&self) -> bool {
        self.semantic_plan.is_some()
            || self.plan_verification.is_some()
            || self.capability_resolution.is_some()
            || self.result_verification.is_some()
            || self.router_budget.is_some()
    }
}

fn router_trace_needs_replay_review(evidence: &RouterLearningTraceEvidence) -> bool {
    plan_verification_rejected(evidence.plan_verification.as_ref())
        || result_verification_has_gaps(evidence.result_verification.as_ref())
        || capability_resolution_has_gap(evidence.capability_resolution.as_ref())
        || evidence.trace_issue_count > 0
}

fn plan_verification_rejected(value: Option<&serde_json::Value>) -> bool {
    value
        .and_then(|value| value.get("accepted"))
        .and_then(|value| value.as_bool())
        .is_some_and(|accepted| !accepted)
}

fn result_verification_has_gaps(value: Option<&serde_json::Value>) -> bool {
    let Some(value) = value else {
        return false;
    };
    let pending = value
        .get("pending_goal_ids")
        .and_then(|value| value.as_array())
        .map(|items| !items.is_empty())
        .unwrap_or(false);
    let unsupported = value
        .get("unsupported_claims")
        .and_then(|value| value.as_array())
        .map(|items| !items.is_empty())
        .unwrap_or(false);
    let accounted = value
        .get("all_goals_accounted")
        .and_then(|value| value.as_bool())
        .unwrap_or(true);
    pending || unsupported || !accounted
}

fn capability_resolution_has_gap(value: Option<&serde_json::Value>) -> bool {
    value
        .and_then(|value| value.as_array())
        .is_some_and(|steps| {
            steps.iter().any(|step| {
                step.get("respond_without_tool")
                    .and_then(|value| value.as_bool())
                    .unwrap_or(false)
                    && step
                        .get("fallback_reason")
                        .and_then(|value| value.as_str())
                        .map(str::trim)
                        .is_some_and(|value| !value.is_empty())
            })
        })
}

fn router_trace_failure_summary(evidence: &RouterLearningTraceEvidence) -> String {
    if plan_verification_rejected(evidence.plan_verification.as_ref()) {
        return "Semantic plan verification reported uncovered or contradictory requirements."
            .to_string();
    }
    if result_verification_has_gaps(evidence.result_verification.as_ref()) {
        return "Result verification found pending goals or unsupported claims.".to_string();
    }
    if capability_resolution_has_gap(evidence.capability_resolution.as_ref()) {
        return "Capability resolution did not resolve a usable action for at least one goal."
            .to_string();
    }
    if evidence.trace_issue_count > 0 {
        return "Execution trace recorded a router-visible failure or blocked step.".to_string();
    }
    "Router trace needs review.".to_string()
}

fn router_replay_benchmark_entry_from_evidence(
    evidence: &RouterLearningTraceEvidence,
) -> Option<serde_json::Value> {
    let semantic_plan = evidence.semantic_plan.as_ref()?;
    Some(serde_json::json!({
        "schema_version": 1,
        "source_trace_id": evidence.trace_id.clone(),
        "review_status": "needs_human_review",
        "input": {
            "user_message_preview": evidence.user_message_preview.clone(),
            "conversation_context_required": true
        },
        "expected_semantic_contract": {
            "explicit_requirements": semantic_plan.get("explicit_requirements").cloned().unwrap_or_else(|| serde_json::json!([])),
            "goals": semantic_plan.get("goals").cloned().unwrap_or_else(|| serde_json::json!([])),
            "selected_tool_names": evidence.selected_tool_names.clone(),
            "direct_response_without_tool": evidence.direct_response_without_tool
        },
        "observed_router_artifacts": {
            "plan_verification": evidence.plan_verification.clone(),
            "capability_resolution": evidence.capability_resolution.clone(),
            "result_verification": evidence.result_verification.clone(),
            "execution_policy": evidence.execution_policy.clone(),
            "capability_snapshot": evidence.capability_snapshot.clone(),
            "capability_health": evidence.capability_health.clone(),
            "model_schedule": evidence.model_schedule.clone()
        },
        "regression_signals": {
            "failure_summary": evidence.failure_summary.clone(),
            "native_schema_count": evidence.native_schema_count,
            "last_prompt_chars": evidence.last_prompt_chars,
            "trace_issue_count": evidence.trace_issue_count,
            "trace_issue_summaries": &evidence.trace_issue_summaries
        }
    }))
}

fn stable_router_replay_candidate_id(trace_id: &str) -> String {
    let mut hasher = std::collections::hash_map::DefaultHasher::new();
    ROUTER_LEARNING_CANDIDATE_TYPE.hash(&mut hasher);
    trace_id.hash(&mut hasher);
    format!("router-replay-{:016x}", hasher.finish())
}

fn safe_trace_preview(text: &str, max_chars: usize) -> String {
    if text.chars().count() <= max_chars {
        text.to_string()
    } else {
        text.chars().take(max_chars).collect()
    }
}

fn looks_like_semantic_plan(data: &serde_json::Value) -> bool {
    data.get("goals").is_some_and(|value| value.is_array())
        && data
            .get("explicit_requirements")
            .is_some_and(|value| value.is_array())
}

fn looks_like_plan_verification(data: &serde_json::Value) -> bool {
    data.get("accepted").is_some_and(|value| value.is_boolean())
        && data
            .get("missing_requirement_ids")
            .is_some_and(|value| value.is_array())
        && data
            .get("must_clarify")
            .is_some_and(|value| value.is_boolean())
}

fn looks_like_capability_resolution(data: &serde_json::Value) -> bool {
    data.as_array().is_some_and(|steps| {
        steps.iter().any(|step| {
            step.get("goal_id").and_then(|value| value.as_str()).is_some()
                && (step.get("action_name").is_some()
                    || step
                        .get("respond_without_tool")
                        .is_some_and(|value| value.is_boolean())
                    || step.get("candidates").is_some_and(|value| value.is_array()))
        })
    })
}

fn capability_resolution_is_direct(data: &serde_json::Value) -> bool {
    data.as_array()
        .filter(|steps| !steps.is_empty())
        .is_some_and(|steps| {
            steps.iter().all(|step| {
                step.get("respond_without_tool")
                    .and_then(|value| value.as_bool())
                    .unwrap_or(false)
                    && step.get("action_name").is_none()
            })
        })
}

fn looks_like_result_verification(data: &serde_json::Value) -> bool {
    data.get("all_goals_accounted")
        .is_some_and(|value| value.is_boolean())
        && data
            .get("completed_goal_ids")
            .is_some_and(|value| value.is_array())
        && data
            .get("pending_goal_ids")
            .is_some_and(|value| value.is_array())
}

fn looks_like_router_budget(data: &serde_json::Value) -> bool {
    data.get("policy").is_some_and(|value| value.is_object())
        && data
            .get("selected_tool_names")
            .is_some_and(|value| value.is_array())
        && data.get("native_schema_count").is_some()
}

fn step_type_is_execution_issue(step_type: &str) -> bool {
    matches!(
        step_type.trim().to_ascii_lowercase().as_str(),
        "error" | "failed" | "failure" | "blocked"
    )
}

fn parse_router_learning_layer(value: &str) -> Option<RouterLearningLayer> {
    match value.trim() {
        "canonical" => Some(RouterLearningLayer::Canonical),
        "action_descriptor" => Some(RouterLearningLayer::ActionDescriptor),
        "benchmark" => Some(RouterLearningLayer::Benchmark),
        "prompt_fragment" => Some(RouterLearningLayer::PromptFragment),
        "policy" => Some(RouterLearningLayer::Policy),
        "capability_graph" => Some(RouterLearningLayer::CapabilityGraph),
        _ => None,
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn router_learning_candidate_requires_layer_and_trace_evidence() {
        let payload = RouterLearningCandidatePayload {
            candidate_type: ROUTER_LEARNING_CANDIDATE_TYPE.to_string(),
            router_layer: "benchmark".to_string(),
            objective: "add replay case for missed multi-outcome turn".to_string(),
            evidence: vec![RouterLearningTraceEvidence {
                trace_id: "trace-1".to_string(),
                failed_layer: Some("benchmark".to_string()),
                failure_summary: Some("missing replay case".to_string()),
                semantic_plan: Some(serde_json::json!({
                    "goals": [],
                    "explicit_requirements": [],
                })),
                ..Default::default()
            }],
            ..Default::default()
        };

        assert!(validate_router_learning_candidate(&payload).is_ok());
    }

    #[test]
    fn semantic_trace_steps_can_be_converted_to_router_learning_evidence() {
        let steps = vec![
            crate::core::ExecutionStep {
                icon: "[plan]".to_string(),
                title: "Planner artifact".to_string(),
                detail: String::new(),
                step_type: "info".to_string(),
                data: Some(r#"{"goals":[],"explicit_requirements":[]}"#.to_string()),
                timestamp: chrono::Utc::now(),
                duration_ms: Some(0),
            },
            crate::core::ExecutionStep {
                icon: "[resolve]".to_string(),
                title: "Resolver artifact".to_string(),
                detail: String::new(),
                step_type: "info".to_string(),
                data: Some(
                    r#"[{"goal_id":"goal_0","respond_without_tool":true}]"#.to_string(),
                ),
                timestamp: chrono::Utc::now(),
                duration_ms: Some(0),
            },
            crate::core::ExecutionStep {
                icon: "[budget]".to_string(),
                title: "Budget artifact".to_string(),
                detail: String::new(),
                step_type: "info".to_string(),
                data: Some(
                    r#"{"policy":{"class":"direct"},"capability_snapshot":null,"selected_tool_names":[],"native_schema_count":0,"last_prompt_chars":900}"#
                        .to_string(),
                ),
                timestamp: chrono::Utc::now(),
                duration_ms: Some(0),
            },
        ];

        let evidence = trace_evidence_from_semantic_steps("trace-1", "preview", &steps);

        assert_eq!(evidence.trace_id, "trace-1");
        assert!(evidence.semantic_plan.is_some());
        assert!(evidence.capability_resolution.is_some());
        assert!(evidence.router_budget.is_some());
        assert!(evidence.direct_response_without_tool);
        assert_eq!(evidence.native_schema_count, Some(0));
    }
}
