//! Task-driven auto-spawn agent system
//! Replaces the old pre-configured swarm model with intelligent, on-demand
//! agent spawning. The LLM decides IF sub-agents are needed, WHAT kind,
//! and they are auto-spawned from the model pool. User-configured specialists
//! act as priority boosters — preferred when they match, but never required.

use anyhow::{anyhow, Context, Result};
use serde::{Deserialize, Serialize};
use std::collections::hash_map::DefaultHasher;
use std::collections::{HashMap, HashSet};
use std::hash::{Hash, Hasher};
use std::sync::Arc;
use tokio::sync::RwLock;

use super::agent::QueryComplexity;
use super::config::{ModelRole, ModelSlot};
use super::llm::LlmClient;
use super::orchestra::SubAgentType;
use super::planner::{PlanStepStatus, PlanSubstep};
use super::prompt_policy::delegated_policy_v2_block;
use super::swarm::agent_trait::SwarmAgent;
use super::swarm::specialist::SpecialistAgent;
use super::swarm::{AgentAccessScope, SwarmActivityAgent, SwarmActivityTracker};
use super::{DegradationNote, DelegationStatus, FailureKind, StreamEvent};
use crate::actions::ActionDef;
use crate::core::queue_stream_event;
use crate::core::PromptMemory;

fn compact_text(text: &str, max_chars: usize) -> String {
    if text.chars().count() <= max_chars {
        text.to_string()
    } else {
        format!("{}...", text.chars().take(max_chars).collect::<String>())
    }
}

fn execution_plan_step_status(step: &crate::core::planner::PlanStep) -> PlanStepStatus {
    step.status.unwrap_or(PlanStepStatus::Pending)
}

fn sync_execution_plan_trace_step(trace: &mut super::agent::ExecutionTrace) {
    let Some(plan) = trace.plan.as_ref() else {
        return;
    };
    let Ok(serialized) = serde_json::to_string(plan) else {
        return;
    };
    let detail = format!("{} steps planned", plan.steps.len());
    if let Some(step) =
        trace.steps.iter_mut().rev().find(|step| {
            step.step_type == "plan" || step.title.eq_ignore_ascii_case("Execution Plan")
        })
    {
        step.detail = detail;
        step.data = Some(serialized);
    }
}

fn execution_plan_step_snapshot(
    trace: &super::agent::ExecutionTrace,
    step_id: usize,
) -> Option<(
    String,
    u32,
    String,
    PlanStepStatus,
    Option<Vec<PlanSubstep>>,
)> {
    let plan = trace.plan.as_ref()?;
    let step = plan.steps.iter().find(|step| step.id == step_id)?;
    Some((
        plan.plan_id.clone(),
        plan.revision,
        step.title.clone(),
        execution_plan_step_status(step),
        if step.substeps.is_empty() {
            None
        } else {
            Some(step.substeps.clone())
        },
    ))
}

async fn emit_execution_plan_step_update(
    token_tx: Option<&tokio::sync::mpsc::Sender<StreamEvent>>,
    plan_id: &str,
    revision: u32,
    step_id: usize,
    step_title: Option<&str>,
    status: PlanStepStatus,
    detail: Option<String>,
    substeps: Option<Vec<PlanSubstep>>,
) {
    if let Some(tx) = token_tx {
        queue_stream_event(
            tx,
            StreamEvent::PlanStepUpdate {
                plan_id: plan_id.to_string(),
                revision,
                step_id,
                step_title: step_title.map(str::to_string),
                status,
                detail,
                substeps,
            },
        );
    }
}

async fn update_delegated_plan_step_status(
    trace_ref: &Arc<RwLock<super::agent::ExecutionTrace>>,
    token_tx: Option<&tokio::sync::mpsc::Sender<StreamEvent>>,
    step_id: usize,
    status: PlanStepStatus,
    detail: Option<String>,
) {
    let snapshot = {
        let mut trace = trace_ref.write().await;
        let mut changed = false;
        if let Some(plan) = trace.plan.as_mut() {
            if let Some(step) = plan.steps.iter_mut().find(|step| step.id == step_id) {
                if step.status != Some(status) {
                    step.status = Some(status);
                    changed = true;
                }
            }
        }
        if changed {
            sync_execution_plan_trace_step(&mut trace);
        }
        execution_plan_step_snapshot(&trace, step_id)
    };
    let Some((plan_id, revision, step_title, snapshot_status, substeps)) = snapshot else {
        return;
    };
    emit_execution_plan_step_update(
        token_tx,
        &plan_id,
        revision,
        step_id,
        Some(step_title.as_str()),
        snapshot_status,
        detail,
        substeps,
    )
    .await;
}

#[derive(Debug, Clone, Serialize)]
struct DelegatedDependencyPacket {
    sequence: usize,
    agent_name: String,
    agent_role: String,
    task: String,
    status: String,
    output_summary: String,
    failure_kind: Option<String>,
    next_action_hint: Option<String>,
}

#[derive(Debug, Clone, Serialize)]
struct DelegatedMemoryPacket {
    memory_type: String,
    content: String,
    timestamp: String,
    relevance_score: f32,
    importance: f32,
    final_score: f32,
}

#[derive(Debug, Clone, Serialize)]
struct DelegatedActionPacket {
    name: String,
    description: String,
    role: String,
    integration_class: String,
    side_effect_level: String,
    requires_auth: bool,
}

#[derive(Debug, Clone, Serialize)]
struct DelegatedTaskPacket {
    delegation_id: String,
    agent_id: String,
    agent_name: String,
    agent_role: String,
    assignment_index: usize,
    total_assignments: usize,
    original_request: String,
    assigned_task: String,
    coordinator_notes: String,
    dependency_outputs: Vec<DelegatedDependencyPacket>,
    relevant_memories: Vec<DelegatedMemoryPacket>,
    action_scope: Vec<DelegatedActionPacket>,
    execution_contract: Vec<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize, Default)]
struct PersistedDelegationPayload {
    #[serde(default, skip_serializing_if = "Option::is_none")]
    request: Option<String>,
    #[serde(default)]
    agent_name: String,
    #[serde(default)]
    agent_role: String,
    #[serde(default)]
    model_name: String,
    #[serde(default)]
    status: String,
    #[serde(default)]
    content: String,
    #[serde(default)]
    summary: String,
    #[serde(default)]
    latest_update: String,
    #[serde(default)]
    is_specialist: bool,
    #[serde(default)]
    depends_on: Vec<usize>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    elapsed_ms: Option<u64>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    conversation_id: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    channel: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    failure_kind: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    next_action_hint: Option<String>,
    #[serde(default)]
    artifacts: Vec<String>,
    #[serde(default)]
    sequence: usize,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    updated_at: Option<String>,
}

impl DelegatedTaskPacket {
    fn render_markdown(&self) -> String {
        let dependency_section = if self.dependency_outputs.is_empty() {
            "No completed dependencies were provided.".to_string()
        } else {
            self.dependency_outputs
                .iter()
                .map(|dep| {
                    let failure_suffix = dep
                        .failure_kind
                        .as_ref()
                        .map(|kind| format!(" | failure={kind}"))
                        .unwrap_or_default();
                    let next_step = dep
                        .next_action_hint
                        .as_ref()
                        .map(|hint| format!("\n  Next-step hint: {}", compact_text(hint, 180)))
                        .unwrap_or_default();
                    format!(
                        "- Step {}: {} · {} [{}]\n  Task: {}\n  Output: {}{}{}",
                        dep.sequence,
                        dep.agent_name,
                        dep.agent_role,
                        dep.status,
                        compact_text(&dep.task, 220),
                        compact_text(&dep.output_summary, 320),
                        failure_suffix,
                        next_step
                    )
                })
                .collect::<Vec<_>>()
                .join("\n")
        };

        let memory_section = if self.relevant_memories.is_empty() {
            "No relevant memory snippets were attached.".to_string()
        } else {
            self.relevant_memories
                .iter()
                .map(|memory| {
                    format!(
                        "- {} | score {:.2} | importance {:.2}\n  {}",
                        memory.memory_type,
                        memory.final_score,
                        memory.importance,
                        compact_text(&memory.content, 260)
                    )
                })
                .collect::<Vec<_>>()
                .join("\n")
        };

        let action_section = if self.action_scope.is_empty() {
            "No action context was attached.".to_string()
        } else {
            self.action_scope
                .iter()
                .map(|action| {
                    let auth = if action.requires_auth {
                        "auth required"
                    } else {
                        "no auth"
                    };
                    format!(
                        "- `{}` [{} / {} / {} / {}] {}\n  {}",
                        action.name,
                        action.role,
                        action.integration_class,
                        action.side_effect_level,
                        auth,
                        "",
                        compact_text(&action.description, 220)
                    )
                })
                .collect::<Vec<_>>()
                .join("\n")
        };

        let contract_section = self
            .execution_contract
            .iter()
            .map(|item| format!("- {item}"))
            .collect::<Vec<_>>()
            .join("\n");

        format!(
            "## Delegated Task Packet\n\
- Delegation id: `{}`\n\
- Agent: {} · {}\n\
- Assignment: {}/{}\n\n\
## Original Request\n{}\n\n\
## Assigned Task\n{}\n\n\
## Coordinator Notes\n{}\n\n\
## Dependency Outputs\n{}\n\n\
## Relevant Memory\n{}\n\n\
## Action Context\n{}\n\n\
## Execution Contract\n{}",
            self.delegation_id,
            self.agent_name,
            self.agent_role,
            self.assignment_index,
            self.total_assignments,
            compact_text(&self.original_request, 900),
            compact_text(&self.assigned_task, 700),
            compact_text(&self.coordinator_notes, 700),
            dependency_section,
            memory_section,
            action_section,
            contract_section
        )
    }
}

fn summarize_memory_type(memory: &PromptMemory) -> &str {
    memory.memory_type.as_str()
}

fn delegation_row_id(delegation_id: &str, agent_id: &str) -> String {
    format!("{delegation_id}::{agent_id}")
}

fn parse_persisted_delegation_payload(raw: Option<&str>) -> PersistedDelegationPayload {
    serde_json::from_str::<PersistedDelegationPayload>(raw.unwrap_or_default()).unwrap_or_default()
}

fn parse_persisted_failure_kind(raw: Option<&str>) -> Option<FailureKind> {
    match raw.unwrap_or_default().trim().to_ascii_lowercase().as_str() {
        "transient_transport" => Some(FailureKind::TransientTransport),
        "rate_limited" => Some(FailureKind::RateLimited),
        "authentication" => Some(FailureKind::Authentication),
        "configuration" => Some(FailureKind::Configuration),
        "context_window_exceeded" => Some(FailureKind::ContextWindowExceeded),
        "schema_mismatch" => Some(FailureKind::SchemaMismatch),
        "tool_contract_failure" => Some(FailureKind::ToolContractFailure),
        "capability_bound" => Some(FailureKind::CapabilityBound),
        "upstream_provider" => Some(FailureKind::UpstreamProvider),
        "timeout" => Some(FailureKind::Timeout),
        "missing_input" => Some(FailureKind::MissingInput),
        "internal_post_process" => Some(FailureKind::InternalPostProcess),
        "delegation_failed" => Some(FailureKind::DelegationFailed),
        "panic" => Some(FailureKind::Panic),
        "unknown" => Some(FailureKind::Unknown),
        _ => None,
    }
}

fn parse_persisted_delegation_status(raw: &str) -> Option<DelegationStatus> {
    match raw.trim().to_ascii_lowercase().as_str() {
        "completed" => Some(DelegationStatus::Completed),
        "partial" => Some(DelegationStatus::Partial),
        "failed" => Some(DelegationStatus::Failed),
        "timed_out" => Some(DelegationStatus::TimedOut),
        "panicked" => Some(DelegationStatus::Panicked),
        _ => None,
    }
}

fn is_resume_reusable_status(status: &str) -> bool {
    matches!(
        status.trim().to_ascii_lowercase().as_str(),
        "completed" | "partial"
    )
}

fn memory_overlap_bonus(task_lower: &str, content_lower: &str) -> f32 {
    if task_lower.is_empty() || content_lower.is_empty() {
        return 0.0;
    }

    // Word-set coverage: how much of the task's meaningful vocabulary appears
    // in the memory body. Structural and order-independent, so paraphrased
    // memory entries score the same as their differently-worded twins.
    // Tokenizing on non-alphanumeric boundaries also avoids the "log → login"
    // false-positive that the previous substring scan produced.
    let split_meaningful = |text: &str, min_len: usize| -> std::collections::HashSet<String> {
        text.split(|c: char| !c.is_alphanumeric())
            .filter(|word| word.len() >= min_len)
            .map(|word| word.to_string())
            .collect()
    };
    let char_ngrams = |text: &str, width: usize| -> std::collections::HashSet<String> {
        let chars = text.chars().collect::<Vec<_>>();
        if chars.is_empty() {
            return std::collections::HashSet::new();
        }
        if chars.len() <= width {
            return [text.to_string()].into_iter().collect();
        }
        (0..=chars.len().saturating_sub(width))
            .map(|index| chars[index..index + width].iter().collect::<String>())
            .collect()
    };
    let token_similarity = |left: &str, right: &str| -> f32 {
        if left == right {
            return 1.0;
        }
        let left_len = left.chars().count();
        let right_len = right.chars().count();
        let min_len = left_len.min(right_len) as f32;
        let max_len = left_len.max(right_len) as f32;
        if max_len <= 0.0 {
            return 0.0;
        }
        let left_ngrams = char_ngrams(left, 3);
        let right_ngrams = char_ngrams(right, 3);
        if left_ngrams.is_empty() || right_ngrams.is_empty() {
            return 0.0;
        }
        let overlap = left_ngrams.intersection(&right_ngrams).count() as f32;
        let union = left_ngrams.union(&right_ngrams).count() as f32;
        let ngram_similarity = if union <= 0.0 { 0.0 } else { overlap / union };
        if ngram_similarity <= 0.0 {
            return 0.0;
        }
        let length_similarity = min_len / max_len;
        (ngram_similarity * 0.8 + length_similarity * 0.2).clamp(0.0, 1.0)
    };

    let task_terms = split_meaningful(task_lower, 4);
    if task_terms.is_empty() {
        return 0.0;
    }
    let content_terms = split_meaningful(content_lower, 1);
    if content_terms.is_empty() {
        return 0.0;
    }

    let coverage = task_terms
        .iter()
        .map(|task_term| {
            content_terms
                .iter()
                .map(|content_term| token_similarity(task_term, content_term))
                .fold(0.0f32, f32::max)
        })
        .sum::<f32>()
        / task_terms.len() as f32;
    if coverage <= 0.0 {
        return 0.0;
    }

    // Coverage ∈ (0.0, 1.0] — capped at the same 0.45 ceiling as the legacy
    // heuristic so the relevance bonus stays in scale with the rest of the
    // scoring pipeline.
    (coverage * 0.45).min(0.45)
}

/// Classify a delegation failure structurally. Detects timeouts by walking
/// the anyhow error chain and looking for `tokio::time::error::Elapsed` —
/// which is the actual typed error returned by `tokio::time::timeout`. This
/// replaces the previous phrase-containment scan ("timed out"/"timeout") so
/// callers no longer have to coordinate on exact wording with the error
/// emission sites.
fn classify_agent_failure(error: &anyhow::Error) -> (DelegationStatus, FailureKind, String) {
    let is_timeout = error.chain().any(|cause| {
        cause
            .downcast_ref::<tokio::time::error::Elapsed>()
            .is_some()
    });
    if is_timeout {
        return (
            DelegationStatus::TimedOut,
            FailureKind::Timeout,
            "Retry the delegated step or continue with the completed work.".to_string(),
        );
    }
    (
        DelegationStatus::Failed,
        FailureKind::DelegationFailed,
        "Retry the delegated step or continue with the partial results.".to_string(),
    )
}

fn summarize_delegation_status(results: &[AgentExecResult]) -> DelegationStatus {
    if results
        .iter()
        .all(|result| result.status == DelegationStatus::Completed)
    {
        return DelegationStatus::Completed;
    }

    let successful = results
        .iter()
        .filter(|result| result.status == DelegationStatus::Completed)
        .count();
    if successful > 0 {
        return DelegationStatus::Partial;
    }

    if results
        .iter()
        .any(|result| result.status == DelegationStatus::Panicked)
    {
        DelegationStatus::Panicked
    } else if results
        .iter()
        .any(|result| result.status == DelegationStatus::TimedOut)
    {
        DelegationStatus::TimedOut
    } else {
        DelegationStatus::Failed
    }
}

fn render_agent_result_metadata(result: &AgentExecResult) -> Option<String> {
    let mut parts = Vec::new();
    if let Some(confidence) = result.confidence {
        parts.push(format!(
            "confidence {:.0}%",
            confidence.clamp(0.0, 1.0) * 100.0
        ));
    }
    if !result.artifacts.is_empty() {
        let preview = result
            .artifacts
            .iter()
            .take(3)
            .map(|artifact| compact_text(artifact, 80))
            .collect::<Vec<_>>()
            .join(", ");
        let suffix = if result.artifacts.len() > 3 {
            ", ..."
        } else {
            ""
        };
        parts.push(format!("artifacts: {}{}", preview, suffix));
    }
    if parts.is_empty() {
        None
    } else {
        Some(parts.join(" | "))
    }
}

fn build_delegation_degradation(results: &[AgentExecResult]) -> Vec<DegradationNote> {
    let degraded: Vec<&AgentExecResult> = results
        .iter()
        .filter(|result| result.status != DelegationStatus::Completed)
        .collect();
    if degraded.is_empty() {
        return Vec::new();
    }

    let detail = degraded
        .iter()
        .map(|result| {
            let name = result
                .agent_name
                .as_deref()
                .unwrap_or(result.agent_type.as_str());
            let failure_kind = result
                .failure_kind
                .as_ref()
                .map(|kind| format!("{:?}", kind))
                .unwrap_or_else(|| "unknown".to_string());
            let metadata = render_agent_result_metadata(result)
                .map(|value| format!(" | {}", value))
                .unwrap_or_default();
            format!(
                "{} [{} / {}{}]: {}",
                name,
                result.status.as_str(),
                failure_kind,
                metadata,
                compact_text(&result.content, 180)
            )
        })
        .collect::<Vec<_>>()
        .join(" | ");

    vec![DegradationNote {
        kind: "delegation".to_string(),
        summary: format!("{} delegated execution path(s) degraded", degraded.len()),
        detail: Some(compact_text(&detail, 500)),
    }]
}

fn build_fallback_delegation_response(
    original_task: &str,
    results: &[AgentExecResult],
) -> super::llm::LlmResponse {
    let completed: Vec<String> = results
        .iter()
        .filter(|result| {
            result.status == DelegationStatus::Completed && !result.content.trim().is_empty()
        })
        .map(|result| {
            let metadata = render_agent_result_metadata(result)
                .map(|value| format!(" ({})", value))
                .unwrap_or_default();
            let label = result
                .agent_name
                .as_deref()
                .unwrap_or(result.agent_type.as_str());
            format!(
                "- {} / {}{}: {}\n  {}",
                label,
                result.agent_type,
                metadata,
                compact_text(&result.task, 160),
                compact_text(&result.content, 900)
            )
        })
        .collect();
    let follow_up: Vec<String> = results
        .iter()
        .filter(|result| result.status != DelegationStatus::Completed)
        .map(|result| {
            let label = result
                .agent_name
                .as_deref()
                .unwrap_or(result.agent_type.as_str());
            let reason = result
                .failure_kind
                .as_ref()
                .map(|kind| format!("{:?}", kind))
                .unwrap_or_else(|| "unknown".to_string());
            let hint = result
                .next_action_hint
                .as_deref()
                .map(|value| format!(" {}", value))
                .unwrap_or_default();
            format!(
                "- {}: {} ({}){}",
                label,
                compact_text(&result.task, 100),
                reason,
                hint
            )
        })
        .collect();

    let mut sections = Vec::new();
    if !completed.is_empty() {
        sections.push(format!("Completed so far:\n{}", completed.join("\n")));
    }
    if !follow_up.is_empty() {
        sections.push(format!("Still needs follow-up:\n{}", follow_up.join("\n")));
    }
    if sections.is_empty() {
        sections.push("No delegated paths completed cleanly.".to_string());
    }

    let intro = if completed.is_empty() {
        format!(
            "I couldn't complete the delegated execution cleanly for this request: {}.",
            compact_text(original_task, 160)
        )
    } else if follow_up.is_empty() {
        "The delegated work completed, but the final synthesis pass did not return separate user-facing text. Here are the usable results."
            .to_string()
    } else {
        "I completed part of this request, but some delegated work degraded and needs follow-up."
            .to_string()
    };

    super::llm::LlmResponse {
        content: format!("{}\n\n{}", intro, sections.join("\n\n")),
        tool_calls: vec![],
        reasoning: Some("delegation_fallback_synthesis".to_string()),
        usage: None,
        provider: "internal".to_string(),
        model: "delegation-fallback".to_string(),
    }
}

fn classify_ephemeral_worker_response(
    content: &str,
    requested_tool_count: usize,
) -> (
    DelegationStatus,
    Option<FailureKind>,
    Option<String>,
    String,
) {
    let content = content.trim();
    if requested_tool_count > 0 {
        return (
            DelegationStatus::Failed,
            Some(FailureKind::ToolContractFailure),
            Some(
                "Run required tools in the parent loop or retry with a worker path that can execute those tools."
                    .to_string(),
            ),
            if content.is_empty() {
                format!(
                    "Delegated path requested {} tool call(s) but returned no final result.",
                    requested_tool_count
                )
            } else {
                format!(
                    "Delegated path requested {} tool call(s) and returned text before those tools could run: {}",
                    requested_tool_count,
                    compact_text(content, 500)
                )
            },
        );
    }
    if content.is_empty() {
        return (
            DelegationStatus::Failed,
            Some(FailureKind::InternalPostProcess),
            Some("Retry with a clearer scoped task.".to_string()),
            "Delegated path returned no user-facing result.".to_string(),
        );
    }
    (DelegationStatus::Completed, None, None, content.to_string())
}

const RESEARCHER_CALLSIGNS: &[&str] = &[
    "Orbit", "Beacon", "Drift", "Survey", "Scout", "Mosaic", "Index", "Archive",
];
const CODER_CALLSIGNS: &[&str] = &[
    "Forge", "Vector", "Patch", "Kernel", "Cipher", "Module", "Socket", "Turing",
];
const ANALYST_CALLSIGNS: &[&str] = &[
    "Atlas", "Prism", "Ledger", "Signal", "Metric", "Summit", "Delta", "Axiom",
];
const WRITER_CALLSIGNS: &[&str] = &[
    "Quill", "Echo", "Verse", "Draft", "Lumen", "Script", "Fable", "Brief",
];
const VALIDATOR_CALLSIGNS: &[&str] = &[
    "Aegis", "Sentinel", "Vanta", "Keystone", "Anchor", "Audit", "Proof", "Verity",
];
const PLANNER_CALLSIGNS: &[&str] = &[
    "Helix",
    "Orion",
    "Northstar",
    "Meridian",
    "Compass",
    "Nexus",
    "Pioneer",
    "Route",
];
const CUSTOM_CALLSIGNS: &[&str] = &[
    "Nova", "Relay", "Flux", "Quartz", "Vertex", "Arc", "Pulse", "Beacon",
];

fn cool_name_pool(agent_type: &SubAgentType) -> &'static [&'static str] {
    match agent_type {
        SubAgentType::Researcher => RESEARCHER_CALLSIGNS,
        SubAgentType::Coder => CODER_CALLSIGNS,
        SubAgentType::Analyst => ANALYST_CALLSIGNS,
        SubAgentType::Writer => WRITER_CALLSIGNS,
        SubAgentType::Validator => VALIDATOR_CALLSIGNS,
        SubAgentType::Planner => PLANNER_CALLSIGNS,
        SubAgentType::Custom { .. } => CUSTOM_CALLSIGNS,
    }
}

pub fn cool_name_for_auto_agent(index: usize, agent_type: &SubAgentType) -> String {
    let pool = cool_name_pool(agent_type);
    pool[index % pool.len()].to_string()
}

fn cool_name_for_auto_agent_in_run(
    index: usize,
    agent_type: &SubAgentType,
    run_seed: &str,
) -> String {
    let pool = cool_name_pool(agent_type);
    if pool.is_empty() {
        return "Agent".to_string();
    }
    let mut hasher = DefaultHasher::new();
    run_seed.hash(&mut hasher);
    index.hash(&mut hasher);
    agent_type.name().hash(&mut hasher);
    pool[(hasher.finish() as usize) % pool.len()].to_string()
}

fn has_generic_agent_name(name: &str, agent_type: &SubAgentType) -> bool {
    let trimmed = name.trim();
    trimmed.is_empty()
        || trimmed.eq_ignore_ascii_case(&agent_type.name())
        || matches!(
            trimmed.to_ascii_lowercase().as_str(),
            "agent" | "specialist" | "worker" | "sub-agent" | "subagent"
        )
}

pub fn display_name_for_specialist(name: &str, agent_type: &SubAgentType) -> String {
    let trimmed = name.trim();
    if has_generic_agent_name(trimmed, agent_type) {
        cool_name_for_auto_agent(0, agent_type)
    } else {
        trimmed.to_string()
    }
}

fn display_name_for_specialist_in_run(
    name: &str,
    agent_type: &SubAgentType,
    index: usize,
    run_seed: &str,
) -> String {
    let trimmed = name.trim();
    if has_generic_agent_name(trimmed, agent_type) {
        cool_name_for_auto_agent_in_run(index, agent_type, run_seed)
    } else {
        trimmed.to_string()
    }
}

fn delegation_payload(
    kind: &str,
    delegation_id: &str,
    summary: &str,
    extra: serde_json::Value,
) -> serde_json::Value {
    let mut payload = match extra {
        serde_json::Value::Object(obj) => obj,
        other => {
            let mut obj = serde_json::Map::new();
            obj.insert("payload".to_string(), other);
            obj
        }
    };
    payload.insert("kind".to_string(), serde_json::json!(kind));
    payload.insert(
        "delegation_id".to_string(),
        serde_json::json!(delegation_id.to_string()),
    );
    payload.insert("chat_visible".to_string(), serde_json::json!(true));
    payload.insert("summary".to_string(), serde_json::json!(summary));
    serde_json::Value::Object(payload)
}

fn emit_delegation_event(
    token_tx: Option<&tokio::sync::mpsc::Sender<StreamEvent>>,
    kind: &str,
    delegation_id: &str,
    content: impl Into<String>,
    extra: serde_json::Value,
) {
    let Some(tx) = token_tx else {
        return;
    };
    let content = content.into();
    queue_stream_event(
        tx,
        StreamEvent::ToolProgress {
            name: "delegation".to_string(),
            content: content.clone(),
            payload: Some(delegation_payload(kind, delegation_id, &content, extra)),
        },
    );
}

fn agent_status_summary(result: &AgentExecResult) -> String {
    let base = format!(
        "{} [{}] {}",
        result
            .agent_name
            .as_deref()
            .unwrap_or(result.agent_type.as_str()),
        result.agent_type,
        result.status.as_str()
    );
    let detail = compact_text(&result.content, 180);
    if detail.is_empty() {
        base
    } else {
        format!("{} - {}", base, detail)
    }
}

/// LLM-determined routing decision
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct RoutingDecision {
    /// Whether the task needs sub-agents
    pub needs_delegation: bool,
    /// Complexity tier (used for model selection and execution strategy)
    pub complexity: QueryComplexity,
    /// Sub-agents to spawn (empty if no delegation)
    pub sub_agents: Vec<SubAgentSpec>,
    /// Brief reasoning for the decision (shown in trace)
    pub reasoning: String,
    /// Router confidence [0.0, 1.0]
    #[serde(default)]
    pub confidence: f32,
    /// Whether to ask a clarification before execution
    #[serde(default)]
    pub should_clarify: bool,
    /// Clarification question to ask when `should_clarify` is true
    #[serde(default)]
    pub clarification_question: Option<String>,
}

/// Specification for an auto-spawned sub-agent
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct SubAgentSpec {
    /// The role/type of this sub-agent
    pub agent_type: String,
    /// Specific task description for this sub-agent
    pub task: String,
    /// Preferred model role (Code, Research, etc.)
    pub preferred_model_role: Option<String>,
    /// Dependencies on other sub-agents (by index in the array)
    #[serde(default)]
    pub depends_on: Vec<usize>,
    /// Optional execution-plan step id that this delegated assignment owns.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub plan_step_id: Option<usize>,
}

impl SubAgentSpec {
    fn canonical_role_label(value: &str) -> String {
        let mut out = String::new();
        let mut last_separator = false;
        for ch in value.trim().to_ascii_lowercase().chars() {
            if ch.is_ascii_alphanumeric() {
                out.push(ch);
                last_separator = false;
            } else if !out.is_empty() && !last_separator {
                out.push('_');
                last_separator = true;
            }
        }
        while out.ends_with('_') {
            out.pop();
        }
        out
    }

    /// Parse agent_type string into SubAgentType
    pub fn resolve_agent_type(&self) -> SubAgentType {
        let label = Self::canonical_role_label(&self.agent_type);
        match label.as_str() {
            "researcher" | "research" | "research_agent" | "explorer" | "investigator"
            | "scout" => SubAgentType::Researcher,
            "coder" | "code" | "coding" | "developer" | "engineer" | "software_engineer"
            | "programmer" | "builder" | "implementer" => SubAgentType::Coder,
            "analyst" | "analysis" | "data_analyst" | "data" | "diagnostic" => {
                SubAgentType::Analyst
            }
            "writer" | "writing" | "editor" | "docs" | "documentation" | "copywriter" => {
                SubAgentType::Writer
            }
            "validator" | "validate" | "reviewer" | "review" | "qa" | "tester" | "verifier" => {
                SubAgentType::Validator
            }
            "planner" | "plan" | "coordinator" | "orchestrator" | "manager" => {
                SubAgentType::Planner
            }
            _ if self.agent_type.trim().is_empty() => SubAgentType::Planner,
            _ => SubAgentType::Custom {
                name: self.agent_type.trim().to_string(),
                instructions: format!(
                    "You are a specialist agent for '{}'. Work only on the assigned task, surface uncertainty explicitly, and return concrete findings or changes.",
                    self.agent_type.trim()
                ),
            },
        }
    }

    /// Parse preferred_model_role string into ModelRole
    pub fn resolve_model_role(&self) -> Option<ModelRole> {
        self.preferred_model_role.as_ref().and_then(|r| {
            match Self::canonical_role_label(r).as_str() {
                "code" | "coder" | "coding" | "developer" | "engineer" => Some(ModelRole::Code),
                "research" | "researcher" | "explorer" | "investigator" => {
                    Some(ModelRole::Research)
                }
                "fast" | "quick" | "small" | "mini" => Some(ModelRole::Fast),
                "primary" | "main" | "default" | "frontier" => Some(ModelRole::Primary),
                _ => None,
            }
        })
    }
}

/// Result of task routing
#[allow(clippy::large_enum_variant)]
pub enum TaskRouterResult {
    /// Simple query — caller should do a direct LLM call
    Direct,
    /// Delegated to auto-spawned agents — here are the results
    Delegated(Box<DelegatedResult>),
}

/// Result from delegated multi-agent execution
#[derive(Debug, Clone)]
pub struct DelegatedResult {
    /// Final synthesized response (includes tool calls when returned by the LLM)
    pub final_response: super::llm::LlmResponse,
    /// Per-agent results for trace visibility
    pub agent_results: Vec<AgentExecResult>,
    /// Whether delegation completed fully or only partially succeeded.
    pub delegation_status: DelegationStatus,
    /// Degradation notes that should be surfaced to the caller.
    pub degradation: Vec<DegradationNote>,
}

/// Result from a single agent execution (for trace)
#[derive(Debug, Clone)]
pub struct AgentExecResult {
    /// Stable identifier for this delegated agent execution.
    pub agent_id: String,
    /// Agent type name
    pub agent_type: String,
    /// Task that was assigned
    pub task: String,
    /// Whether this was a user-configured specialist or auto-spawned
    pub is_specialist: bool,
    /// Display name shown for the delegated agent.
    pub agent_name: Option<String>,
    /// Model used
    pub model_name: String,
    /// Response content
    pub content: String,
    /// Full LLM response (only present for ephemeral auto-agents)
    pub llm_response: Option<super::llm::LlmResponse>,
    /// Execution time in ms
    pub execution_time_ms: u64,
    /// Typed status for this delegated execution path.
    pub status: DelegationStatus,
    /// Structured failure classification when the path did not complete cleanly.
    pub failure_kind: Option<FailureKind>,
    /// Optional next step hint for degraded sub-agent results.
    pub next_action_hint: Option<String>,
    /// Confidence reported by the delegation layer when available.
    pub confidence: Option<f32>,
    /// Artifact identifiers or summaries produced by the delegated path.
    pub artifacts: Vec<String>,
}

/// Configuration for the task router
#[derive(Clone)]
pub struct TaskRouterConfig {
    /// Max concurrent agents
    pub _max_concurrent: usize,
    /// Legacy field retained for config/API compatibility.
    ///
    /// Delegated agents are not hard-capped by this router; they run until
    /// they finish or the parent run is interrupted.
    pub _agent_timeout_secs: u64,
    /// Minimum confidence for a specialist to be used over ephemeral
    pub specialist_threshold: f32,
}

impl Default for TaskRouterConfig {
    fn default() -> Self {
        Self {
            _max_concurrent: 5,
            _agent_timeout_secs: 0,
            specialist_threshold: 0.3,
        }
    }
}

/// The unified task router — auto-spawns agents based on LLM routing decisions
#[derive(Clone)]
pub struct TaskRouter {
    config: TaskRouterConfig,
}

type SpecialistRegistry = Arc<RwLock<HashMap<super::swarm::AgentId, Arc<SpecialistAgent>>>>;

pub struct TaskRouterExecuteContext<'a> {
    pub delegation_id: &'a str,
    pub conversation_id: Option<&'a str>,
    pub channel: Option<&'a str>,
    pub message: &'a str,
    pub system_prompt: &'a str,
    pub prompt_bundle: &'a crate::core::self_evolve::PromptBundleProfile,
    pub specialist_prompt_bundle: &'a crate::core::self_evolve::SpecialistPromptBundleProfile,
    pub configured_model_slots: &'a [ModelSlot],
    pub model_pool: &'a HashMap<String, (ModelSlot, LlmClient)>,
    pub primary_model_id: &'a str,
    pub user_selected_model_slot_id: Option<&'a str>,
    pub smart_routing: bool,
    pub primary_llm: &'a LlmClient,
    pub specialists: &'a Option<SpecialistRegistry>,
    pub memories: &'a [PromptMemory],
    pub actions: &'a [ActionDef],
    pub action_scope_hints: &'a HashMap<String, crate::runtime::ActionScopeHint>,
    pub trace: &'a Arc<RwLock<super::agent::ExecutionTrace>>,
    pub token_tx: Option<&'a tokio::sync::mpsc::Sender<StreamEvent>>,
    pub swarm_activity: Option<&'a Arc<SwarmActivityTracker>>,
    pub storage: Option<&'a crate::storage::Storage>,
}

impl TaskRouter {
    pub fn new(config: TaskRouterConfig) -> Self {
        Self { config }
    }

    /// Execute a routing decision — spawn agents, collect results, synthesize
    pub async fn execute(
        &self,
        decision: &RoutingDecision,
        ctx: TaskRouterExecuteContext<'_>,
    ) -> Result<TaskRouterResult> {
        let delegation_id = ctx.delegation_id;
        let conversation_id = ctx.conversation_id;
        let channel = ctx.channel;
        let message = ctx.message;
        let system_prompt = ctx.system_prompt;
        let prompt_bundle = ctx.prompt_bundle;
        let specialist_prompt_bundle = ctx.specialist_prompt_bundle;
        let configured_model_slots = ctx.configured_model_slots;
        let model_pool = ctx.model_pool;
        let primary_model_id = ctx.primary_model_id;
        let user_selected_model_slot_id = ctx.user_selected_model_slot_id;
        let smart_routing = ctx.smart_routing;
        let primary_llm = ctx.primary_llm;
        let specialists = ctx.specialists;
        let memories = ctx.memories;
        let actions = ctx.actions;
        let action_scope_hints = ctx.action_scope_hints;
        let trace = ctx.trace;
        let token_tx = ctx.token_tx;
        let swarm_activity = ctx.swarm_activity;
        let storage = ctx.storage;
        // Simple queries — no delegation
        if !decision.needs_delegation {
            return match decision.complexity {
                QueryComplexity::Simple => Ok(TaskRouterResult::Direct),
                QueryComplexity::Medium => Ok(TaskRouterResult::Direct),
                QueryComplexity::Complex => Ok(TaskRouterResult::Direct), // complex but LLM said no delegation
            };
        }

        if decision.sub_agents.is_empty() {
            return Ok(TaskRouterResult::Direct);
        }

        let start = std::time::Instant::now();
        if let Some(tracker) = swarm_activity {
            tracker
                .start_run(
                    delegation_id,
                    message,
                    conversation_id,
                    channel,
                    decision.sub_agents.len(),
                )
                .await;
        }
        emit_delegation_event(
            token_tx,
            "delegation_started",
            delegation_id,
            format!("Starting {} delegated agents.", decision.sub_agents.len()),
            serde_json::json!({
                "status": "running",
                "agent_count": decision.sub_agents.len(),
                "request": compact_text(message, 200),
            }),
        );

        // Build assignments: for each spec, find a specialist or pick model from pool
        let mut assignments: Vec<AgentAssignment> = Vec::new();

        for (index, spec) in decision.sub_agents.iter().enumerate() {
            let agent_type = spec.resolve_agent_type();

            // Try to find a matching user-configured specialist
            let specialist_match = if let Some(ref specs) = specialists {
                self.find_matching_specialist(specs, &spec.task, &agent_type)
                    .await
            } else {
                None
            };

            if let Some((name, specialist)) = specialist_match {
                let display_name =
                    display_name_for_specialist_in_run(&name, &agent_type, index, delegation_id);
                let agent_id = specialist.id().to_string();
                // Trace: specialist matched
                {
                    let mut t = trace.write().await;
                    t.steps.push(super::agent::ExecutionStep {
                        icon: "\u{2B50}".to_string(), // star
                        title: format!("Specialist Matched: {}", name),
                        detail: format!("Task: {}", spec.task),
                        step_type: "info".to_string(),
                        data: None,
                        timestamp: chrono::Utc::now(),
                        duration_ms: None,
                    });
                }
                if let Some(tracker) = swarm_activity {
                    tracker
                        .upsert_agent(
                            delegation_id,
                            SwarmActivityAgent {
                                id: agent_id.clone(),
                                agent_name: display_name.clone(),
                                agent_role: agent_type.name(),
                                model_name: specialist.model_name(),
                                task: spec.task.clone(),
                                status: "assigned".to_string(),
                                summary: "Matched to a configured specialist.".to_string(),
                                latest_update: "Waiting to start.".to_string(),
                                is_specialist: true,
                                depends_on: spec.depends_on.clone(),
                                started_at: None,
                                completed_at: None,
                                updated_at: chrono::Utc::now().to_rfc3339(),
                                elapsed_ms: None,
                            },
                        )
                        .await;
                }
                emit_delegation_event(
                    token_tx,
                    "delegation_assignment",
                    delegation_id,
                    format!("Assigned {}.", display_name),
                    serde_json::json!({
                        "agent_id": agent_id.clone(),
                        "agent_name": display_name.clone(),
                        "agent_role": agent_type.name(),
                        "model_name": specialist.model_name(),
                        "task": spec.task.clone(),
                        "is_specialist": true,
                        "depends_on": spec.depends_on.clone(),
                        "plan_step_id": spec.plan_step_id,
                        "sequence": index + 1,
                        "status": "assigned",
                    }),
                );
                assignments.push(AgentAssignment {
                    agent_id,
                    spec: spec.clone(),
                    agent_type: agent_type.clone(),
                    display_name,
                    model_name: specialist.model_name(),
                    kind: AssignmentKind::Specialist(specialist),
                });
            } else {
                // Auto-spawn: select LLM from model pool
                let llm = self.select_llm_for_spec(
                    spec,
                    &agent_type,
                    configured_model_slots,
                    model_pool,
                    primary_model_id,
                    user_selected_model_slot_id,
                    smart_routing,
                    primary_llm,
                );
                let model_name = llm.model_name().to_string();
                let auto_agent_name =
                    cool_name_for_auto_agent_in_run(assignments.len(), &agent_type, delegation_id);
                let agent_id = spec
                    .plan_step_id
                    .map(|step_id| format!("{}:plan-step:{}", delegation_id, step_id))
                    .unwrap_or_else(|| format!("{}:agent:{}", delegation_id, index + 1));
                // Trace: auto-spawning
                {
                    let mut t = trace.write().await;
                    t.steps.push(super::agent::ExecutionStep {
                        icon: "\u{1F916}".to_string(), // robot
                        title: format!("Auto-Agent: {}", auto_agent_name),
                        detail: format!(
                            "{} | Model: {} | Task: {}",
                            agent_type.name(),
                            model_name,
                            spec.task
                        ),
                        step_type: "thinking".to_string(),
                        data: None,
                        timestamp: chrono::Utc::now(),
                        duration_ms: None,
                    });
                }
                if let Some(tracker) = swarm_activity {
                    tracker
                        .upsert_agent(
                            delegation_id,
                            SwarmActivityAgent {
                                id: agent_id.clone(),
                                agent_name: auto_agent_name.clone(),
                                agent_role: agent_type.name(),
                                model_name: model_name.clone(),
                                task: spec.task.clone(),
                                status: "assigned".to_string(),
                                summary: "Prepared as an on-demand helper agent.".to_string(),
                                latest_update: "Waiting to start.".to_string(),
                                is_specialist: false,
                                depends_on: spec.depends_on.clone(),
                                started_at: None,
                                completed_at: None,
                                updated_at: chrono::Utc::now().to_rfc3339(),
                                elapsed_ms: None,
                            },
                        )
                        .await;
                }
                emit_delegation_event(
                    token_tx,
                    "delegation_assignment",
                    delegation_id,
                    format!("Prepared {}.", auto_agent_name),
                    serde_json::json!({
                        "agent_id": agent_id.clone(),
                        "agent_name": auto_agent_name.clone(),
                        "agent_role": agent_type.name(),
                        "model_name": model_name,
                        "task": spec.task.clone(),
                        "is_specialist": false,
                        "depends_on": spec.depends_on.clone(),
                        "plan_step_id": spec.plan_step_id,
                        "sequence": index + 1,
                        "status": "assigned",
                    }),
                );
                assignments.push(AgentAssignment {
                    agent_id,
                    spec: spec.clone(),
                    agent_type: agent_type.clone(),
                    display_name: auto_agent_name,
                    model_name: llm.model_name().to_string(),
                    kind: AssignmentKind::Ephemeral(llm),
                });
            }
        }

        // Execute assignments respecting dependencies
        let results = self
            .execute_assignments(
                delegation_id,
                &assignments,
                message,
                system_prompt,
                specialist_prompt_bundle,
                memories,
                actions,
                action_scope_hints,
                trace,
                token_tx,
                swarm_activity,
                conversation_id,
                channel,
                storage,
            )
            .await?;

        let delegation_status = summarize_delegation_status(&results);
        let mut degradation = build_delegation_degradation(&results);
        let completed_paths = results
            .iter()
            .filter(|result| result.status == DelegationStatus::Completed)
            .count();

        // Aggregate
        let final_response = if completed_paths == 0 {
            degradation.push(DegradationNote {
                kind: "delegation_synthesis".to_string(),
                summary: "delegated synthesis skipped".to_string(),
                detail: Some(
                    "No delegated execution path completed cleanly, so the router returned a best-effort internal summary without another model hop."
                        .to_string(),
                ),
            });
            {
                let mut t = trace.write().await;
                t.steps.push(super::agent::ExecutionStep {
                    icon: "[fallback]".to_string(),
                    title: "Delegation Fallback Summary".to_string(),
                    detail: format!(
                        "All delegated paths degraded, so {} returned a best-effort summary.",
                        crate::branding::PRODUCT_NAME
                    ),
                    step_type: "warning".to_string(),
                    data: None,
                    timestamp: chrono::Utc::now(),
                    duration_ms: None,
                });
            }
            if let Some(tracker) = swarm_activity {
                tracker
                    .update_run_status(
                        delegation_id,
                        "degraded",
                        "No delegated paths completed cleanly; returning a best-effort summary.",
                    )
                    .await;
            }
            emit_delegation_event(
                token_tx,
                "delegation_synthesis_started",
                delegation_id,
                "No delegated paths completed cleanly; using the fallback summary.".to_string(),
                serde_json::json!({
                    "status": "degraded",
                    "completed_paths": completed_paths,
                }),
            );
            build_fallback_delegation_response(message, &results)
        } else {
            if let Some(tracker) = swarm_activity {
                tracker
                    .update_run_status(
                        delegation_id,
                        "synthesizing",
                        "Combining delegated outputs into one answer.",
                    )
                    .await;
            }
            emit_delegation_event(
                token_tx,
                "delegation_synthesis_started",
                delegation_id,
                format!("Synthesizing {} delegated result(s).", completed_paths),
                serde_json::json!({
                    "status": "synthesizing",
                    "completed_paths": completed_paths,
                    "agent_count": results.len(),
                }),
            );
            let aggregate_result = if results.len() == 1 {
                if let Some(resp) = results[0].llm_response.clone() {
                    Ok(resp)
                } else {
                    // Specialist-only single result: run a final synthesis pass so tool calls
                    // can still be emitted by the primary model.
                    self.aggregate(
                        primary_llm,
                        message,
                        system_prompt,
                        prompt_bundle,
                        &results,
                        memories,
                        actions,
                    )
                    .await
                }
            } else {
                // Trace: aggregating
                {
                    let mut t = trace.write().await;
                    t.steps.push(super::agent::ExecutionStep {
                        icon: "\u{1F504}".to_string(), // arrows
                        title: format!("Synthesizing {} agent results", results.len()),
                        detail: results
                            .iter()
                            .map(|r| r.agent_type.clone())
                            .collect::<Vec<_>>()
                            .join(", "),
                        step_type: "thinking".to_string(),
                        data: None,
                        timestamp: chrono::Utc::now(),
                        duration_ms: None,
                    });
                }
                self.aggregate(
                    primary_llm,
                    message,
                    system_prompt,
                    prompt_bundle,
                    &results,
                    memories,
                    actions,
                )
                .await
            };

            match aggregate_result {
                Ok(response) => {
                    if response.content.trim().is_empty() {
                        let detail = if response.tool_calls.is_empty() {
                            "The synthesis pass returned no user-facing text.".to_string()
                        } else {
                            format!(
                                "The synthesis pass returned {} tool call(s) but no user-facing text.",
                                response.tool_calls.len()
                            )
                        };
                        degradation.push(DegradationNote {
                            kind: "delegation_synthesis".to_string(),
                            summary: "delegated synthesis fallback".to_string(),
                            detail: Some(detail.clone()),
                        });
                        {
                            let mut t = trace.write().await;
                            t.steps.push(super::agent::ExecutionStep {
                                icon: "[fallback]".to_string(),
                                title: "Delegation Synthesis Fallback".to_string(),
                                detail: format!(
                                    "The synthesis pass did not return user-facing text, so {} returned a best-effort summary.",
                                    crate::branding::PRODUCT_NAME
                                ),
                                step_type: "warning".to_string(),
                                data: Some(detail),
                                timestamp: chrono::Utc::now(),
                                duration_ms: None,
                            });
                        }
                        build_fallback_delegation_response(message, &results)
                    } else {
                        response
                    }
                }
                Err(error) => {
                    let error_text = compact_text(&error.to_string(), 240);
                    degradation.push(DegradationNote {
                        kind: "delegation_synthesis".to_string(),
                        summary: "delegated synthesis fallback".to_string(),
                        detail: Some(error_text.clone()),
                    });
                    {
                        let mut t = trace.write().await;
                        t.steps.push(super::agent::ExecutionStep {
                            icon: "[fallback]".to_string(),
                            title: "Delegation Synthesis Fallback".to_string(),
                            detail: format!(
                                "The primary synthesis pass failed, so {} returned a best-effort summary.",
                                crate::branding::PRODUCT_NAME
                            ),
                            step_type: "warning".to_string(),
                            data: Some(error_text),
                            timestamp: chrono::Utc::now(),
                            duration_ms: None,
                        });
                    }
                    build_fallback_delegation_response(message, &results)
                }
            }
        };

        // Do not recover omitted tool calls from message/action scores here.
        // The model tool loop owns executable intent; delegated synthesis may
        // summarize completed sub-agent results but cannot revive actions.

        let total_time_ms = start.elapsed().as_millis() as u64;
        let completion_status = if degradation.is_empty() {
            "completed"
        } else if completed_paths > 0 {
            "partial"
        } else {
            "failed"
        };
        let completion_summary = if degradation.is_empty() {
            format!("Completed {} delegated agents successfully.", results.len())
        } else {
            format!(
                "Delegated execution finished with status {}.",
                delegation_status.as_str()
            )
        };

        // Trace: complete
        {
            let mut t = trace.write().await;
            t.steps.push(super::agent::ExecutionStep {
                icon: "\u{2705}".to_string(), // checkmark
                title: "Agent Delegation Complete".to_string(),
                detail: format!(
                    "{} agents | {}ms | status={}",
                    results.len(),
                    total_time_ms,
                    delegation_status.as_str()
                ),
                step_type: if degradation.is_empty() {
                    "success".to_string()
                } else {
                    "warning".to_string()
                },
                data: Some(
                    results
                        .iter()
                        .map(|r| {
                            let tag = if r.is_specialist {
                                "specialist"
                            } else {
                                "auto"
                            };
                            format!(
                                "{} [{} / {}] ({}ms)",
                                r.agent_type,
                                tag,
                                r.status.as_str(),
                                r.execution_time_ms
                            )
                        })
                        .collect::<Vec<_>>()
                        .join(", "),
                ),
                timestamp: chrono::Utc::now(),
                duration_ms: Some(total_time_ms),
            });
        }
        emit_delegation_event(
            token_tx,
            "delegation_completed",
            delegation_id,
            completion_summary.clone(),
            serde_json::json!({
                "status": completion_status,
                "delegation_status": delegation_status.as_str(),
                "agent_count": results.len(),
                "elapsed_ms": total_time_ms,
            }),
        );
        if let Some(tracker) = swarm_activity {
            tracker
                .complete_run(delegation_id, completion_status, &completion_summary)
                .await;
        }

        Ok(TaskRouterResult::Delegated(Box::new(DelegatedResult {
            final_response,
            agent_results: results,
            delegation_status,
            degradation,
        })))
    }

    /// Find a user-configured specialist that matches the task
    async fn find_matching_specialist(
        &self,
        specialists: &Arc<RwLock<HashMap<super::swarm::AgentId, Arc<SpecialistAgent>>>>,
        task: &str,
        expected_type: &SubAgentType,
    ) -> Option<(String, Arc<SpecialistAgent>)> {
        let specs = specialists.read().await;
        let mut best: Option<(f32, String, Arc<SpecialistAgent>)> = None;

        for (_, specialist) in specs.iter() {
            if !specialist.config().enabled {
                continue;
            }

            let score = specialist.can_handle(task);

            // Bonus for matching agent type
            let type_bonus = if specialist.config().agent_type.name() == expected_type.name() {
                0.2
            } else {
                0.0
            };

            let total = score + type_bonus;
            if total > self.config.specialist_threshold
                && best.as_ref().is_none_or(|(s, _, _)| total > *s)
            {
                best = Some((total, specialist.config().name.clone(), specialist.clone()));
            }
        }

        best.map(|(_, name, spec)| (name, spec))
    }

    /// Select the best LLM from the model pool for a sub-agent spec
    fn select_llm_for_spec(
        &self,
        spec: &SubAgentSpec,
        agent_type: &SubAgentType,
        configured_model_slots: &[ModelSlot],
        model_pool: &HashMap<String, (ModelSlot, LlmClient)>,
        primary_model_id: &str,
        user_selected_model_slot_id: Option<&str>,
        smart_routing: bool,
        primary_llm: &LlmClient,
    ) -> LlmClient {
        let requested_role = if !smart_routing {
            ModelRole::Primary
        } else {
            spec.resolve_model_role().unwrap_or(match agent_type {
                SubAgentType::Coder => ModelRole::Code,
                SubAgentType::Researcher => ModelRole::Research,
                _ => ModelRole::Primary,
            })
        };

        let mut ordered_slot_ids = Vec::new();
        let mut seen = HashSet::new();
        let mut push_slot_id = |slot_id: &str| {
            let normalized = slot_id.trim();
            if normalized.is_empty() {
                return;
            }
            if seen.insert(normalized.to_string()) {
                ordered_slot_ids.push(normalized.to_string());
            }
        };

        if let Some(slot_id) = user_selected_model_slot_id {
            if configured_model_slots.iter().any(|slot| slot.id == slot_id) {
                push_slot_id(slot_id);
            }
        }

        if requested_role == ModelRole::Primary {
            if configured_model_slots
                .iter()
                .any(|slot| slot.id == primary_model_id)
            {
                push_slot_id(primary_model_id);
            }
            for slot in configured_model_slots {
                if slot.role == ModelRole::Primary {
                    push_slot_id(&slot.id);
                }
            }
        } else {
            for slot in configured_model_slots {
                if slot.role == requested_role {
                    push_slot_id(&slot.id);
                }
            }
            if configured_model_slots
                .iter()
                .any(|slot| slot.id == primary_model_id)
            {
                push_slot_id(primary_model_id);
            }
        }

        for slot in configured_model_slots {
            if slot.role == ModelRole::Fallback {
                push_slot_id(&slot.id);
            }
        }

        for slot in configured_model_slots {
            push_slot_id(&slot.id);
        }

        for slot_id in ordered_slot_ids {
            let Some((slot, client)) = model_pool.get(&slot_id) else {
                continue;
            };
            let has_runtime_credentials = match &slot.provider {
                crate::core::LlmProvider::Ollama { .. } => true,
                crate::core::LlmProvider::Anthropic { api_key, .. }
                | crate::core::LlmProvider::OpenAI { api_key, .. } => {
                    !api_key.trim().is_empty() && api_key != "[ENCRYPTED]"
                }
            };
            if slot.enabled && has_runtime_credentials {
                return client.clone();
            }
        }

        primary_llm.clone()
    }

    /// Keep sub-agent tool context bounded without making a second routing
    /// decision from text/action scores.
    fn select_actions_for_task(&self, _task: &str, actions: &[ActionDef]) -> Vec<ActionDef> {
        actions.iter().take(8).cloned().collect()
    }

    fn specialist_action_is_allowed(
        &self,
        action: &ActionDef,
        access_scope: &AgentAccessScope,
        action_scope_hints: &HashMap<String, crate::runtime::ActionScopeHint>,
    ) -> bool {
        let required_permission_ids =
            crate::runtime::ActionRuntime::action_required_agent_permission_ids(action);
        if required_permission_ids.iter().any(|permission_id| {
            !access_scope
                .approved_permission_ids
                .iter()
                .any(|approved| approved.eq_ignore_ascii_case(permission_id))
        }) {
            return false;
        }

        let Some(hint) = action_scope_hints.get(&action.name) else {
            return true;
        };

        if let Some(server_id) = hint.mcp_server_id.as_deref() {
            return access_scope
                .mcp_server_ids
                .iter()
                .any(|value| value == server_id);
        }

        if let Some(api_id) = hint.custom_api_id.as_deref() {
            return access_scope
                .custom_api_ids
                .iter()
                .any(|value| value == api_id);
        }

        if hint.requires_ssh_connection {
            return !access_scope.ssh_connection_names.is_empty();
        }

        if !hint.integration_ids.is_empty()
            && !hint.integration_ids.iter().any(|integration_id| {
                access_scope
                    .integration_ids
                    .iter()
                    .any(|value| value == integration_id)
            })
        {
            return false;
        }

        if !hint.extension_pack_ids.is_empty()
            && !hint.extension_pack_ids.iter().any(|pack_id| {
                access_scope
                    .extension_pack_ids
                    .iter()
                    .any(|value| value == pack_id)
            })
        {
            return false;
        }

        if !hint.channel_targets.is_empty() && access_scope.channel_ids.is_empty() {
            return false;
        }

        true
    }

    fn select_actions_for_specialist(
        &self,
        task: &str,
        actions: &[ActionDef],
        access_scope: &AgentAccessScope,
        action_scope_hints: &HashMap<String, crate::runtime::ActionScopeHint>,
    ) -> Vec<ActionDef> {
        let allowed_actions: Vec<ActionDef> = actions
            .iter()
            .filter(|action| {
                self.specialist_action_is_allowed(action, access_scope, action_scope_hints)
            })
            .cloned()
            .collect();
        self.select_actions_for_task(task, &allowed_actions)
    }

    /// Keep delegated memory context compact and task-relevant.
    fn select_memories_for_task(&self, task: &str, memories: &[PromptMemory]) -> Vec<PromptMemory> {
        let task_lower = task.to_ascii_lowercase();
        let mut scored: Vec<(f32, PromptMemory)> = memories
            .iter()
            .map(|memory| {
                let content_lower = memory.content.to_ascii_lowercase();
                let score = memory.final_score.max(memory.relevance_score)
                    + (memory.importance * 0.20)
                    + memory_overlap_bonus(&task_lower, &content_lower);
                (score, memory.clone())
            })
            .collect();

        scored.sort_by(|a, b| b.0.partial_cmp(&a.0).unwrap_or(std::cmp::Ordering::Equal));
        scored
            .into_iter()
            .take(4)
            .map(|(_, memory)| memory)
            .collect()
    }

    fn summarize_action_scope(&self, actions: &[ActionDef]) -> Vec<DelegatedActionPacket> {
        actions
            .iter()
            .take(6)
            .map(|action| {
                let meta = action.action_metadata();
                DelegatedActionPacket {
                    name: action.name.clone(),
                    description: action.description.clone(),
                    role: format!("{:?}", meta.role).to_ascii_lowercase(),
                    integration_class: format!("{:?}", meta.integration_class).to_ascii_lowercase(),
                    side_effect_level: format!("{:?}", meta.side_effect_level).to_ascii_lowercase(),
                    requires_auth: meta.requires_auth,
                }
            })
            .collect()
    }

    fn summarize_memory_scope(&self, memories: &[PromptMemory]) -> Vec<DelegatedMemoryPacket> {
        memories
            .iter()
            .take(4)
            .map(|memory| DelegatedMemoryPacket {
                memory_type: summarize_memory_type(memory).to_string(),
                content: compact_text(&memory.content, 240),
                timestamp: memory.timestamp.to_rfc3339(),
                relevance_score: memory.relevance_score,
                importance: memory.importance,
                final_score: memory.final_score,
            })
            .collect()
    }

    fn build_dependency_scope(
        &self,
        assignment: &AgentAssignment,
        assignments: &[AgentAssignment],
        results: &[Option<AgentExecResult>],
    ) -> Vec<DelegatedDependencyPacket> {
        assignment
            .spec
            .depends_on
            .iter()
            .filter_map(|&dep| {
                let result = results.get(dep)?.as_ref()?;
                let prior_assignment = assignments.get(dep)?;
                Some(DelegatedDependencyPacket {
                    sequence: dep + 1,
                    agent_name: result
                        .agent_name
                        .clone()
                        .unwrap_or_else(|| prior_assignment.display_name.clone()),
                    agent_role: result.agent_type.clone(),
                    task: result.task.clone(),
                    status: result.status.as_str().to_string(),
                    output_summary: compact_text(&result.content, 320),
                    failure_kind: result
                        .failure_kind
                        .as_ref()
                        .map(|kind| format!("{:?}", kind)),
                    next_action_hint: result.next_action_hint.clone(),
                })
            })
            .collect()
    }

    #[allow(clippy::too_many_arguments)]
    fn build_task_packet(
        &self,
        delegation_id: &str,
        assignment_index: usize,
        total_assignments: usize,
        original_request: &str,
        coordinator_notes: &str,
        assignment: &AgentAssignment,
        dependency_scope: Vec<DelegatedDependencyPacket>,
        memory_scope: &[PromptMemory],
        action_scope: &[ActionDef],
    ) -> DelegatedTaskPacket {
        DelegatedTaskPacket {
            delegation_id: delegation_id.to_string(),
            agent_id: assignment.agent_id.clone(),
            agent_name: assignment.display_name.clone(),
            agent_role: assignment.agent_type.name(),
            assignment_index: assignment_index + 1,
            total_assignments,
            original_request: compact_text(original_request, 1000),
            assigned_task: assignment.spec.task.clone(),
            coordinator_notes: compact_text(coordinator_notes, 900),
            dependency_outputs: dependency_scope,
            relevant_memories: self.summarize_memory_scope(memory_scope),
            action_scope: self.summarize_action_scope(action_scope),
            execution_contract: vec![
                "Stay within the assigned task and do not expand scope on your own."
                    .to_string(),
                "Use dependency outputs as upstream truth unless they conflict with the user request."
                    .to_string(),
                "Treat action context as capability context, not as proof that an action ran."
                    .to_string(),
                "Do not claim an action was executed unless an actual action result is present in the packet."
                    .to_string(),
                "If the task needs evidence that is not present, state the missing evidence or verification needed instead of fabricating it."
                    .to_string(),
                "Return the highest-signal result for your task, including risks or missing follow-up if relevant."
                    .to_string(),
            ],
        }
    }

    #[allow(clippy::too_many_arguments)]
    fn build_checkpoint_payload(
        &self,
        request: &str,
        assignment: &AgentAssignment,
        status: &str,
        summary: &str,
        latest_update: &str,
        content: Option<&str>,
        elapsed_ms: Option<u64>,
        conversation_id: Option<&str>,
        channel: Option<&str>,
        failure_kind: Option<&FailureKind>,
        next_action_hint: Option<&str>,
        artifacts: &[String],
        sequence: usize,
    ) -> PersistedDelegationPayload {
        PersistedDelegationPayload {
            request: Some(compact_text(request, 220)),
            agent_name: assignment.display_name.clone(),
            agent_role: assignment.agent_type.name(),
            model_name: assignment.model_name.clone(),
            status: status.to_string(),
            content: content
                .map(|value| compact_text(value, 3_200))
                .unwrap_or_default(),
            summary: compact_text(summary, 320),
            latest_update: compact_text(latest_update, 320),
            is_specialist: matches!(&assignment.kind, AssignmentKind::Specialist(_)),
            depends_on: assignment.spec.depends_on.clone(),
            elapsed_ms,
            conversation_id: conversation_id.map(str::to_string),
            channel: channel.map(str::to_string),
            failure_kind: failure_kind.map(|kind| kind.as_str().to_string()),
            next_action_hint: next_action_hint.map(|value| compact_text(value, 240)),
            artifacts: artifacts
                .iter()
                .map(|artifact| compact_text(artifact, 140))
                .collect(),
            sequence: sequence + 1,
            updated_at: Some(chrono::Utc::now().to_rfc3339()),
        }
    }

    #[allow(clippy::too_many_arguments)]
    async fn persist_assignment_checkpoint(
        &self,
        storage: &crate::storage::Storage,
        delegation_id: &str,
        conversation_id: Option<&str>,
        channel: Option<&str>,
        request: &str,
        sequence: usize,
        assignment: &AgentAssignment,
        status: &str,
        summary: &str,
        latest_update: &str,
        content: Option<&str>,
        elapsed_ms: Option<u64>,
        failure_kind: Option<&FailureKind>,
        next_action_hint: Option<&str>,
        confidence: Option<f32>,
        artifacts: &[String],
        completed_at: Option<String>,
    ) -> Result<()> {
        if matches!(&assignment.kind, AssignmentKind::Ephemeral(_)) {
            let agent_row = crate::storage::entities::swarm_agent::Model {
                id: assignment.agent_id.clone(),
                name: assignment.display_name.clone(),
                agent_type: assignment.agent_type.name(),
                llm_provider: assignment.model_name.clone(),
                capabilities: "[]".to_string(),
                system_prompt: None,
                access_scope: "{}".to_string(),
                enabled: 0,
                created_at: chrono::Utc::now().to_rfc3339(),
            };
            storage.upsert_swarm_agent(&agent_row).await?;
        }
        let row = crate::storage::entities::swarm_delegation::Model {
            id: delegation_row_id(delegation_id, &assignment.agent_id),
            parent_task_id: None,
            agent_id: assignment.agent_id.clone(),
            task_description: assignment.spec.task.clone(),
            result: Some(serde_json::to_string(&self.build_checkpoint_payload(
                request,
                assignment,
                status,
                summary,
                latest_update,
                content,
                elapsed_ms,
                conversation_id,
                channel,
                failure_kind,
                next_action_hint,
                artifacts,
                sequence,
            ))?),
            success: if matches!(status, "completed" | "partial") {
                1
            } else {
                0
            },
            confidence,
            execution_time_ms: elapsed_ms.map(|value| value.min(i32::MAX as u64) as i32),
            created_at: chrono::Utc::now().to_rfc3339(),
            completed_at,
        };
        storage.upsert_swarm_delegation(&row).await
    }

    fn checkpoint_result_from_row(
        &self,
        row: &crate::storage::entities::swarm_delegation::Model,
    ) -> Option<AgentExecResult> {
        let payload = parse_persisted_delegation_payload(row.result.as_deref());
        if row.completed_at.is_none() || !is_resume_reusable_status(&payload.status) {
            return None;
        }
        let status = parse_persisted_delegation_status(&payload.status)?;
        let content = if payload.content.trim().is_empty() {
            payload.summary.trim().to_string()
        } else {
            payload.content.trim().to_string()
        };
        Some(AgentExecResult {
            agent_id: row.agent_id.clone(),
            agent_type: if payload.agent_role.trim().is_empty() {
                "Agent".to_string()
            } else {
                payload.agent_role.trim().to_string()
            },
            task: row.task_description.clone(),
            is_specialist: payload.is_specialist,
            agent_name: if payload.agent_name.trim().is_empty() {
                Some(row.agent_id.clone())
            } else {
                Some(payload.agent_name.trim().to_string())
            },
            model_name: if payload.model_name.trim().is_empty() {
                "-".to_string()
            } else {
                payload.model_name.trim().to_string()
            },
            content,
            llm_response: None,
            execution_time_ms: payload
                .elapsed_ms
                .or_else(|| row.execution_time_ms.map(|value| value.max(0) as u64))
                .unwrap_or_default(),
            status,
            failure_kind: parse_persisted_failure_kind(payload.failure_kind.as_deref()),
            next_action_hint: payload.next_action_hint,
            confidence: row.confidence,
            artifacts: payload.artifacts,
        })
    }

    /// Execute all assignments, respecting dependency ordering
    #[allow(clippy::too_many_arguments)]
    async fn execute_assignments(
        &self,
        delegation_id: &str,
        assignments: &[AgentAssignment],
        original_request: &str,
        coordinator_notes: &str,
        specialist_prompt_bundle: &crate::core::self_evolve::SpecialistPromptBundleProfile,
        memories: &[PromptMemory],
        actions: &[ActionDef],
        action_scope_hints: &HashMap<String, crate::runtime::ActionScopeHint>,
        trace: &Arc<RwLock<super::agent::ExecutionTrace>>,
        token_tx: Option<&tokio::sync::mpsc::Sender<StreamEvent>>,
        swarm_activity: Option<&Arc<SwarmActivityTracker>>,
        conversation_id: Option<&str>,
        channel: Option<&str>,
        storage: Option<&crate::storage::Storage>,
    ) -> Result<Vec<AgentExecResult>> {
        let n = assignments.len();
        let mut results: Vec<Option<AgentExecResult>> = vec![None; n];
        let mut completed: Vec<bool> = vec![false; n];

        if let Some(storage) = storage {
            match storage
                .get_swarm_delegations_for_parent(delegation_id)
                .await
            {
                Ok(existing_rows) => {
                    let existing_by_agent_id = existing_rows
                        .into_iter()
                        .map(|row| (row.agent_id.clone(), row))
                        .collect::<HashMap<_, _>>();
                    for (idx, assignment) in assignments.iter().enumerate() {
                        let Some(row) = existing_by_agent_id.get(&assignment.agent_id) else {
                            continue;
                        };
                        let Some(restored) = self.checkpoint_result_from_row(row) else {
                            continue;
                        };
                        if let Some(tracker) = swarm_activity {
                            tracker
                                .update_agent(
                                    delegation_id,
                                    &restored.agent_id,
                                    restored.status.as_str(),
                                    &compact_text(&restored.content, 220),
                                    Some("Restored previously completed delegated work."),
                                    Some(restored.execution_time_ms),
                                )
                                .await;
                        }
                        if let Some(plan_step_id) = assignment.spec.plan_step_id {
                            let step_status = if restored.status == DelegationStatus::Completed {
                                PlanStepStatus::Completed
                            } else {
                                PlanStepStatus::Failed
                            };
                            update_delegated_plan_step_status(
                                trace,
                                token_tx,
                                plan_step_id,
                                step_status,
                                Some(
                                    "Restored delegated progress from the previous run."
                                        .to_string(),
                                ),
                            )
                            .await;
                        }
                        emit_delegation_event(
                            token_tx,
                            "delegation_agent_completed",
                            delegation_id,
                            format!(
                                "{} restored from the previous run state.",
                                restored
                                    .agent_name
                                    .as_deref()
                                    .unwrap_or(restored.agent_type.as_str())
                            ),
                            serde_json::json!({
                                "agent_id": restored.agent_id.clone(),
                                "agent_name": restored.agent_name.clone().unwrap_or_default(),
                                "agent_role": restored.agent_type.clone(),
                                "model_name": restored.model_name.clone(),
                                "task": restored.task.clone(),
                                "status": restored.status.as_str(),
                                "elapsed_ms": restored.execution_time_ms,
                                "is_specialist": restored.is_specialist,
                                "restored": true,
                                "output_preview": compact_text(&restored.content, 1800),
                                "plan_step_id": assignment.spec.plan_step_id,
                            }),
                        );
                        results[idx] = Some(restored);
                        completed[idx] = true;
                    }
                }
                Err(error) => tracing::warn!(
                    "Failed to load swarm delegation checkpoints for '{}': {}",
                    delegation_id,
                    error
                ),
            }
        }

        for (idx, assignment) in assignments.iter().enumerate() {
            if completed[idx] {
                continue;
            }
            let Some(plan_step_id) = assignment.spec.plan_step_id else {
                continue;
            };
            let restored_step = {
                let trace_guard = trace.read().await;
                trace_guard
                    .plan
                    .as_ref()
                    .and_then(|plan| plan.steps.iter().find(|step| step.id == plan_step_id))
                    .map(|step| {
                        (
                            step.title.clone(),
                            step.description.clone(),
                            execution_plan_step_status(step),
                        )
                    })
            };
            let Some((step_title, step_description, step_status)) = restored_step else {
                continue;
            };
            let (status, failure_kind, detail, event_kind) = match step_status {
                PlanStepStatus::Completed => (
                    DelegationStatus::Completed,
                    None,
                    format!("{} was already completed earlier in this run.", step_title),
                    "delegation_agent_completed",
                ),
                PlanStepStatus::Failed | PlanStepStatus::Skipped => (
                    DelegationStatus::Failed,
                    Some(FailureKind::DelegationFailed),
                    format!(
                        "{} was already closed before this delegated pass started.",
                        step_title
                    ),
                    "delegation_agent_failed",
                ),
                PlanStepStatus::Pending | PlanStepStatus::Running => continue,
            };
            let content = if step_description.trim().is_empty() {
                detail.clone()
            } else {
                format!("{}\n\n{}", detail, step_description.trim())
            };
            let restored = AgentExecResult {
                agent_id: assignment.agent_id.clone(),
                agent_type: assignment.agent_type.name(),
                task: assignment.spec.task.clone(),
                is_specialist: matches!(&assignment.kind, AssignmentKind::Specialist(_)),
                agent_name: Some(assignment.display_name.clone()),
                model_name: assignment.model_name.clone(),
                content: content.clone(),
                llm_response: None,
                execution_time_ms: 0,
                status,
                failure_kind,
                next_action_hint: None,
                confidence: None,
                artifacts: Vec::new(),
            };
            if let Some(tracker) = swarm_activity {
                tracker
                    .update_agent(
                        delegation_id,
                        &restored.agent_id,
                        restored.status.as_str(),
                        &compact_text(&restored.content, 220),
                        Some("Restored from the execution plan state."),
                        Some(restored.execution_time_ms),
                    )
                    .await;
            }
            emit_delegation_event(
                token_tx,
                event_kind,
                delegation_id,
                detail,
                serde_json::json!({
                    "agent_id": restored.agent_id.clone(),
                    "agent_name": restored.agent_name.clone().unwrap_or_default(),
                    "agent_role": restored.agent_type.clone(),
                    "model_name": restored.model_name.clone(),
                    "task": restored.task.clone(),
                    "status": restored.status.as_str(),
                    "elapsed_ms": restored.execution_time_ms,
                    "is_specialist": restored.is_specialist,
                    "restored": true,
                    "output_preview": compact_text(&restored.content, 1800),
                    "plan_step_id": plan_step_id,
                }),
            );
            results[idx] = Some(restored);
            completed[idx] = true;
        }

        if let Some(storage) = storage {
            for (idx, assignment) in assignments.iter().enumerate() {
                if completed[idx] {
                    continue;
                }
                if let Err(error) = self
                    .persist_assignment_checkpoint(
                        storage,
                        delegation_id,
                        conversation_id,
                        channel,
                        original_request,
                        idx,
                        assignment,
                        "assigned",
                        "Delegated assignment queued.",
                        "Waiting to start.",
                        None,
                        None,
                        None,
                        None,
                        None,
                        &[],
                        None,
                    )
                    .await
                {
                    tracing::warn!(
                        "Failed to persist queued delegation checkpoint '{}' for '{}': {}",
                        assignment.agent_id,
                        delegation_id,
                        error
                    );
                }
            }
        }

        loop {
            // Find assignments whose dependencies are all satisfied
            let mut ready: Vec<usize> = Vec::new();
            for i in 0..n {
                if completed[i] {
                    continue;
                }
                let deps_ok = assignments[i]
                    .spec
                    .depends_on
                    .iter()
                    .all(|&dep| dep < n && completed[dep]);
                if deps_ok {
                    ready.push(i);
                }
            }

            if ready.is_empty() {
                if completed.iter().all(|&c| c) {
                    break; // all done
                }
                return Err(anyhow!("Circular dependency in sub-agent specs"));
            }

            // Execute ready assignments in parallel
            let mut handles = Vec::new();
            for idx in ready {
                let assignment = &assignments[idx];
                let agent_id = assignment.agent_id.clone();
                let task = assignment.spec.task.clone();
                let agent_type = assignment.agent_type.clone();
                let display_name = assignment.display_name.clone();
                let dependency_scope =
                    self.build_dependency_scope(assignment, assignments, &results);
                let packet_dependency_count = dependency_scope.len();
                let mems: Vec<PromptMemory> = self.select_memories_for_task(&task, memories);
                let acts: Vec<ActionDef> = match &assignment.kind {
                    AssignmentKind::Specialist(specialist) => self.select_actions_for_specialist(
                        &task,
                        actions,
                        &specialist.config().access_scope,
                        action_scope_hints,
                    ),
                    _ => self.select_actions_for_task(&task, actions),
                };
                let packet = self.build_task_packet(
                    delegation_id,
                    idx,
                    assignments.len(),
                    original_request,
                    coordinator_notes,
                    assignment,
                    dependency_scope,
                    &mems,
                    &acts,
                );
                let ctx = packet.render_markdown();
                let dependency_count = assignment.spec.depends_on.len();
                let plan_step_id = assignment.spec.plan_step_id;
                let memory_count = mems.len();
                let action_count = acts.len();
                if let Some(storage) = storage {
                    if let Err(error) = self
                        .persist_assignment_checkpoint(
                            storage,
                            delegation_id,
                            conversation_id,
                            channel,
                            original_request,
                            idx,
                            assignment,
                            "running",
                            "Delegated agent is running.",
                            "Starting delegated work.",
                            None,
                            None,
                            None,
                            None,
                            None,
                            &[],
                            None,
                        )
                        .await
                    {
                        tracing::warn!(
                            "Failed to persist running delegation checkpoint '{}' for '{}': {}",
                            assignment.agent_id,
                            delegation_id,
                            error
                        );
                    }
                }
                if let Some(tracker) = swarm_activity {
                    tracker
                        .update_agent(
                            delegation_id,
                            &agent_id,
                            "running",
                            "Starting delegated work.",
                            Some("Delegated agent is now running."),
                            None,
                        )
                        .await;
                }
                if let Some(plan_step_id) = plan_step_id {
                    update_delegated_plan_step_status(
                        trace,
                        token_tx,
                        plan_step_id,
                        PlanStepStatus::Running,
                        Some(format!("{} is now running in parallel.", display_name)),
                    )
                    .await;
                }
                emit_delegation_event(
                    token_tx,
                    "delegation_agent_started",
                    delegation_id,
                    format!("{} is working.", display_name),
                    serde_json::json!({
                        "agent_id": agent_id.clone(),
                        "agent_name": display_name.clone(),
                        "agent_role": agent_type.name(),
                        "task": task.clone(),
                        "depends_on": assignment.spec.depends_on.clone(),
                        "plan_step_id": plan_step_id,
                        "status": "running",
                        "dependency_count": dependency_count,
                        "resolved_dependency_count": packet_dependency_count,
                        "memory_count": memory_count,
                        "action_count": action_count,
                        "context_mode": "packet_v1",
                    }),
                );
                let (heartbeat_stop_tx, mut heartbeat_stop_rx) = tokio::sync::watch::channel(false);
                let heartbeat_tx = token_tx.cloned();
                let heartbeat_tracker = swarm_activity.cloned();
                let heartbeat_delegation_id = delegation_id.to_string();
                let heartbeat_agent_id = agent_id.clone();
                let heartbeat_agent_name = display_name.clone();
                let heartbeat_agent_role = agent_type.name();
                let heartbeat_task = task.clone();
                let heartbeat_plan_step_id = plan_step_id;
                let heartbeat_handle = tokio::spawn(async move {
                    let started = std::time::Instant::now();
                    loop {
                        tokio::select! {
                            changed = heartbeat_stop_rx.changed() => {
                                if changed.is_err() || *heartbeat_stop_rx.borrow() {
                                    break;
                                }
                            }
                            _ = tokio::time::sleep(std::time::Duration::from_secs(8)) => {
                                let elapsed_ms = started.elapsed().as_millis() as u64;
                                if let Some(tracker) = heartbeat_tracker.as_ref() {
                                    tracker
                                        .update_agent(
                                            &heartbeat_delegation_id,
                                            &heartbeat_agent_id,
                                            "running",
                                            "Still working on delegated task.",
                                            Some("Delegated agent is still working."),
                                            Some(elapsed_ms),
                                        )
                                        .await;
                                }
                                emit_delegation_event(
                                    heartbeat_tx.as_ref(),
                                    "delegation_agent_progress",
                                    &heartbeat_delegation_id,
                                    format!("{} is still working.", heartbeat_agent_name),
                                    serde_json::json!({
                                        "agent_id": heartbeat_agent_id.clone(),
                                        "agent_name": heartbeat_agent_name.clone(),
                                        "agent_role": heartbeat_agent_role.clone(),
                                        "task": heartbeat_task.clone(),
                                        "status": "running",
                                        "plan_step_id": heartbeat_plan_step_id,
                                        "elapsed_ms": elapsed_ms,
                                    }),
                                );
                            }
                        }
                    }
                });

                match &assignment.kind {
                    AssignmentKind::Specialist(specialist) => {
                        let specialist = specialist.clone();
                        let agent_id = agent_id.clone();
                        let specialist_system_prompt =
                            crate::core::self_evolve::specialist_prompt_evolution::render_specialist_role_prompt(
                                specialist_prompt_bundle,
                                &agent_type,
                            );
                        handles.push((
                            idx,
                            true,
                            heartbeat_stop_tx,
                            heartbeat_handle,
                            tokio::spawn(async move {
                                let start = std::time::Instant::now();
                                let result = specialist
                                    .execute_task_with_scope_and_prompt(
                                        &task,
                                        &ctx,
                                        &mems,
                                        &[],
                                        Some(specialist_system_prompt),
                                        None,
                                    )
                                    .await;
                                let elapsed = start.elapsed().as_millis() as u64;
                                let model = specialist.model_name();
                                match result {
                                    Ok(content) => {
                                        let content = content.trim().to_string();
                                        let empty = content.is_empty();
                                        Ok(AgentExecResult {
                                            agent_id,
                                            agent_type: agent_type.name(),
                                            task,
                                            is_specialist: true,
                                            agent_name: Some(display_name),
                                            model_name: model,
                                            content: if empty {
                                                "Delegated path returned no user-facing result."
                                                    .to_string()
                                            } else {
                                                content
                                            },
                                            llm_response: None,
                                            execution_time_ms: elapsed,
                                            status: if empty {
                                                DelegationStatus::Failed
                                            } else {
                                                DelegationStatus::Completed
                                            },
                                            failure_kind: if empty {
                                                Some(FailureKind::InternalPostProcess)
                                            } else {
                                                None
                                            },
                                            next_action_hint: if empty {
                                                Some(
                                                    "Retry with a clearer scoped task or run the required verification in the parent tool loop."
                                                        .to_string(),
                                                )
                                            } else {
                                                None
                                            },
                                            confidence: Some(1.0),
                                            artifacts: Vec::new(),
                                        })
                                    }
                                    Err(e) => Err(anyhow!("Specialist error: {}", e)),
                                }
                            }),
                        ));
                    }
                    AssignmentKind::Ephemeral(llm) => {
                        let llm = llm.clone();
                        let agent_id = agent_id.clone();
                        let model_name = llm.model_name().to_string();
                        let delegated_system_prompt =
                            crate::core::self_evolve::specialist_prompt_evolution::render_specialist_role_prompt(
                                specialist_prompt_bundle,
                                &agent_type,
                            );
                        handles.push((
                            idx,
                            false,
                            heartbeat_stop_tx,
                            heartbeat_handle,
                            tokio::spawn(async move {
                                let start = std::time::Instant::now();
                                let prompt = format!(
                                    "{}\n\n## Delegated Policy\n{}\n\n{}",
                                    delegated_system_prompt,
                                    delegated_policy_v2_block(),
                                    ctx
                                );
                                let result = llm.chat(&prompt, &task, &mems, &[]).await;
                                let elapsed = start.elapsed().as_millis() as u64;
                                match result {
                                    Ok(resp) => {
                                        let content = resp.content.trim().to_string();
                                        let requested_tool_count = resp.tool_calls.len();
                                        let (status, failure_kind, next_action_hint, result_text) =
                                            classify_ephemeral_worker_response(
                                                &content,
                                                requested_tool_count,
                                            );
                                        Ok(AgentExecResult {
                                            agent_id,
                                            agent_type: agent_type.name(),
                                            task,
                                            is_specialist: false,
                                            agent_name: Some(display_name),
                                            model_name,
                                            content: result_text,
                                            llm_response: Some(resp),
                                            execution_time_ms: elapsed,
                                            status,
                                            failure_kind,
                                            next_action_hint,
                                            confidence: Some(1.0),
                                            artifacts: Vec::new(),
                                        })
                                    }
                                    Err(e) => Err(anyhow!("Agent error: {}", e)),
                                }
                            }),
                        ));
                    }
                }
            }

            // Collect results
            for (idx, is_specialist, heartbeat_stop_tx, heartbeat_handle, handle) in handles {
                let agent_outcome = handle.await;
                let _ = heartbeat_stop_tx.send(true);
                let _ = heartbeat_handle.await;
                match agent_outcome {
                    Ok(Ok(result)) => {
                        let result_completed = result.status == DelegationStatus::Completed;
                        // Trace: agent completed
                        {
                            let mut t = trace.write().await;
                            let tag = if is_specialist {
                                format!(
                                    "Specialist: {}",
                                    result.agent_name.as_deref().unwrap_or("?")
                                )
                            } else {
                                format!(
                                    "Auto-Agent: {}",
                                    result
                                        .agent_name
                                        .as_deref()
                                        .unwrap_or(result.agent_type.as_str())
                                )
                            };
                            t.steps.push(super::agent::ExecutionStep {
                                icon: "\u{26A1}".to_string(), // lightning
                                title: if result_completed {
                                    format!("{} completed", tag)
                                } else {
                                    format!("{} degraded", tag)
                                },
                                detail: format!(
                                    "Model: {} | {}ms | {} chars",
                                    result.model_name,
                                    result.execution_time_ms,
                                    result.content.len()
                                ),
                                step_type: if result_completed {
                                    "success".to_string()
                                } else {
                                    "warning".to_string()
                                },
                                data: render_agent_result_metadata(&result),
                                timestamp: chrono::Utc::now(),
                                duration_ms: Some(result.execution_time_ms),
                            });
                        }
                        if let Some(tracker) = swarm_activity {
                            tracker
                                .update_agent(
                                    delegation_id,
                                    &result.agent_id,
                                    result.status.as_str(),
                                    &compact_text(&result.content, 220),
                                    Some(if result_completed {
                                        "Delegated work completed."
                                    } else {
                                        "Delegated work returned no usable final result."
                                    }),
                                    Some(result.execution_time_ms),
                                )
                                .await;
                        }
                        if let Some(plan_step_id) = assignments[idx].spec.plan_step_id {
                            update_delegated_plan_step_status(
                                trace,
                                token_tx,
                                plan_step_id,
                                if result_completed {
                                    PlanStepStatus::Completed
                                } else {
                                    PlanStepStatus::Failed
                                },
                                Some(if result_completed {
                                    format!(
                                        "{} completed delegated work.",
                                        result
                                            .agent_name
                                            .as_deref()
                                            .unwrap_or(result.agent_type.as_str())
                                    )
                                } else {
                                    format!(
                                        "{} returned no usable final result.",
                                        result
                                            .agent_name
                                            .as_deref()
                                            .unwrap_or(result.agent_type.as_str())
                                    )
                                }),
                            )
                            .await;
                        }
                        emit_delegation_event(
                            token_tx,
                            if result_completed {
                                "delegation_agent_completed"
                            } else {
                                "delegation_agent_failed"
                            },
                            delegation_id,
                            agent_status_summary(&result),
                            serde_json::json!({
                                "agent_id": result.agent_id.clone(),
                                "agent_name": result.agent_name.clone().unwrap_or_default(),
                                "agent_role": result.agent_type.clone(),
                                "model_name": result.model_name.clone(),
                                "task": result.task.clone(),
                                "status": result.status.as_str(),
                                "elapsed_ms": result.execution_time_ms,
                                "is_specialist": result.is_specialist,
                                "failure_kind": result.failure_kind.as_ref().map(|kind| kind.as_str()),
                                "next_action_hint": result.next_action_hint.clone(),
                                "output_preview": compact_text(&result.content, 1800),
                                "plan_step_id": assignments[idx].spec.plan_step_id,
                            }),
                        );
                        if let Some(storage) = storage {
                            if let Err(error) = self
                                .persist_assignment_checkpoint(
                                    storage,
                                    delegation_id,
                                    conversation_id,
                                    channel,
                                    original_request,
                                    idx,
                                    &assignments[idx],
                                    result.status.as_str(),
                                    if result_completed {
                                        "Delegated work completed."
                                    } else {
                                        "Delegated work returned no usable final result."
                                    },
                                    &compact_text(&result.content, 220),
                                    Some(&result.content),
                                    Some(result.execution_time_ms),
                                    result.failure_kind.as_ref(),
                                    result.next_action_hint.as_deref(),
                                    result.confidence,
                                    &result.artifacts,
                                    Some(chrono::Utc::now().to_rfc3339()),
                                )
                                .await
                            {
                                tracing::warn!(
                                    "Failed to persist completed delegation checkpoint '{}' for '{}': {}",
                                    assignments[idx].agent_id,
                                    delegation_id,
                                    error
                                );
                            }
                        }
                        results[idx] = Some(result);
                        completed[idx] = true;
                    }
                    Ok(Err(e)) => {
                        tracing::warn!("Agent {} failed: {}", idx, e);
                        let (status, failure_kind, next_action_hint) = classify_agent_failure(&e);
                        // Create a failure result so we can continue
                        results[idx] = Some(AgentExecResult {
                            agent_id: assignments[idx].agent_id.clone(),
                            agent_type: assignments[idx].agent_type.name(),
                            task: assignments[idx].spec.task.clone(),
                            is_specialist,
                            agent_name: Some(assignments[idx].display_name.clone()),
                            model_name: assignments[idx].model_name.clone(),
                            content: format!("Agent failed: {}", e),
                            llm_response: None,
                            execution_time_ms: 0,
                            status,
                            failure_kind: Some(failure_kind),
                            next_action_hint: Some(next_action_hint),
                            confidence: None,
                            artifacts: Vec::new(),
                        });
                        if let Some(result) = results[idx].as_ref() {
                            if let Some(tracker) = swarm_activity {
                                tracker
                                    .update_agent(
                                        delegation_id,
                                        &result.agent_id,
                                        result.status.as_str(),
                                        &compact_text(&result.content, 220),
                                        Some("Delegated work failed."),
                                        None,
                                    )
                                    .await;
                            }
                            if let Some(plan_step_id) = assignments[idx].spec.plan_step_id {
                                update_delegated_plan_step_status(
                                    trace,
                                    token_tx,
                                    plan_step_id,
                                    PlanStepStatus::Failed,
                                    Some(format!(
                                        "{} failed during delegated work.",
                                        result
                                            .agent_name
                                            .as_deref()
                                            .unwrap_or(result.agent_type.as_str())
                                    )),
                                )
                                .await;
                            }
                            emit_delegation_event(
                                token_tx,
                                "delegation_agent_failed",
                                delegation_id,
                                agent_status_summary(result),
                                serde_json::json!({
                                    "agent_id": result.agent_id.clone(),
                                    "agent_name": result.agent_name.clone().unwrap_or_default(),
                                    "agent_role": result.agent_type.clone(),
                                    "task": result.task.clone(),
                                    "status": result.status.as_str(),
                                    "reason": result.failure_kind.as_ref().map(|kind| format!("{:?}", kind)),
                                    "is_specialist": result.is_specialist,
                                    "output_preview": compact_text(&result.content, 1800),
                                    "plan_step_id": assignments[idx].spec.plan_step_id,
                                }),
                            );
                            if let Some(storage) = storage {
                                if let Err(error) = self
                                    .persist_assignment_checkpoint(
                                        storage,
                                        delegation_id,
                                        conversation_id,
                                        channel,
                                        original_request,
                                        idx,
                                        &assignments[idx],
                                        result.status.as_str(),
                                        "Delegated work failed.",
                                        &compact_text(&result.content, 220),
                                        Some(&result.content),
                                        None,
                                        result.failure_kind.as_ref(),
                                        result.next_action_hint.as_deref(),
                                        result.confidence,
                                        &result.artifacts,
                                        Some(chrono::Utc::now().to_rfc3339()),
                                    )
                                    .await
                                {
                                    tracing::warn!(
                                        "Failed to persist failed delegation checkpoint '{}' for '{}': {}",
                                        assignments[idx].agent_id,
                                        delegation_id,
                                        error
                                    );
                                }
                            }
                        }
                        completed[idx] = true;
                    }
                    Err(e) => {
                        tracing::error!("Agent {} panicked: {}", idx, e);
                        results[idx] = Some(AgentExecResult {
                            agent_id: assignments[idx].agent_id.clone(),
                            agent_type: assignments[idx].agent_type.name(),
                            task: assignments[idx].spec.task.clone(),
                            is_specialist,
                            agent_name: Some(assignments[idx].display_name.clone()),
                            model_name: assignments[idx].model_name.clone(),
                            content: format!("Agent panicked: {}", e),
                            llm_response: None,
                            execution_time_ms: 0,
                            status: DelegationStatus::Panicked,
                            failure_kind: Some(FailureKind::Panic),
                            next_action_hint: Some(
                                "Retry the delegated step or continue with the completed results."
                                    .to_string(),
                            ),
                            confidence: None,
                            artifacts: Vec::new(),
                        });
                        if let Some(result) = results[idx].as_ref() {
                            if let Some(tracker) = swarm_activity {
                                tracker
                                    .update_agent(
                                        delegation_id,
                                        &result.agent_id,
                                        result.status.as_str(),
                                        &compact_text(&result.content, 220),
                                        Some("Delegated work panicked."),
                                        None,
                                    )
                                    .await;
                            }
                            if let Some(plan_step_id) = assignments[idx].spec.plan_step_id {
                                update_delegated_plan_step_status(
                                    trace,
                                    token_tx,
                                    plan_step_id,
                                    PlanStepStatus::Failed,
                                    Some(format!(
                                        "{} panicked during delegated work.",
                                        result
                                            .agent_name
                                            .as_deref()
                                            .unwrap_or(result.agent_type.as_str())
                                    )),
                                )
                                .await;
                            }
                            emit_delegation_event(
                                token_tx,
                                "delegation_agent_failed",
                                delegation_id,
                                agent_status_summary(result),
                                serde_json::json!({
                                    "agent_id": result.agent_id.clone(),
                                    "agent_name": result.agent_name.clone().unwrap_or_default(),
                                    "agent_role": result.agent_type.clone(),
                                    "task": result.task.clone(),
                                    "status": result.status.as_str(),
                                    "reason": "panic",
                                    "is_specialist": result.is_specialist,
                                    "output_preview": compact_text(&result.content, 1800),
                                    "plan_step_id": assignments[idx].spec.plan_step_id,
                                }),
                            );
                            if let Some(storage) = storage {
                                if let Err(error) = self
                                    .persist_assignment_checkpoint(
                                        storage,
                                        delegation_id,
                                        conversation_id,
                                        channel,
                                        original_request,
                                        idx,
                                        &assignments[idx],
                                        result.status.as_str(),
                                        "Delegated work panicked.",
                                        &compact_text(&result.content, 220),
                                        Some(&result.content),
                                        None,
                                        result.failure_kind.as_ref(),
                                        result.next_action_hint.as_deref(),
                                        result.confidence,
                                        &result.artifacts,
                                        Some(chrono::Utc::now().to_rfc3339()),
                                    )
                                    .await
                                {
                                    tracing::warn!(
                                        "Failed to persist panicked delegation checkpoint '{}' for '{}': {}",
                                        assignments[idx].agent_id,
                                        delegation_id,
                                        error
                                    );
                                }
                            }
                        }
                        completed[idx] = true;
                    }
                }
            }
        }

        Ok(results.into_iter().flatten().collect())
    }

    /// Aggregate multiple agent results into a single coherent response
    async fn aggregate(
        &self,
        llm: &LlmClient,
        original_task: &str,
        _base_system_prompt: &str,
        prompt_bundle: &crate::core::self_evolve::PromptBundleProfile,
        results: &[AgentExecResult],
        memories: &[PromptMemory],
        _actions: &[ActionDef],
    ) -> Result<super::llm::LlmResponse> {
        let mut results_text: String = results
            .iter()
            .map(|r| {
                let tag = if r.is_specialist {
                    format!(
                        "{} (Specialist: {})",
                        r.agent_type,
                        r.agent_name.as_deref().unwrap_or("?")
                    )
                } else {
                    format!(
                        "{} (Auto: {})",
                        r.agent_type,
                        r.agent_name.as_deref().unwrap_or("?")
                    )
                };
                let status_line = if r.status == DelegationStatus::Completed {
                    String::new()
                } else {
                    let failure_kind = r
                        .failure_kind
                        .as_ref()
                        .map(|kind| format!("{:?}", kind))
                        .unwrap_or_else(|| "unknown".to_string());
                    let next_step = r
                        .next_action_hint
                        .as_deref()
                        .map(|hint| format!("\nNext step hint: {}", hint))
                        .unwrap_or_default();
                    format!(
                        "Status: {} ({}){}",
                        r.status.as_str(),
                        failure_kind,
                        next_step
                    )
                };
                let metadata_line = render_agent_result_metadata(r)
                    .map(|value| format!("\nMetadata: {}", value))
                    .unwrap_or_default();
                let body = compact_text(&r.content, 1600);
                if status_line.is_empty() {
                    format!(
                        "## {} - {}{}\n{}",
                        tag,
                        compact_text(&r.task, 240),
                        metadata_line,
                        body
                    )
                } else {
                    format!(
                        "## {} - {}\n{}{}\n{}",
                        tag,
                        compact_text(&r.task, 240),
                        status_line,
                        metadata_line,
                        body
                    )
                }
            })
            .collect::<Vec<_>>()
            .join("\n\n---\n\n");
        results_text = compact_text(&results_text, 9000);

        let compact_original_task = compact_text(original_task, 1200);
        let prompt = crate::core::self_evolve::prompt_evolution::render_synthesis_user_prompt(
            prompt_bundle,
            &crate::core::self_evolve::prompt_evolution::SynthesisPromptRenderInputs {
                original_task: &compact_original_task,
                results_text: &results_text,
            },
        );

        let synth_system_prompt =
            crate::core::self_evolve::prompt_evolution::render_synthesis_system_prompt(
                prompt_bundle,
            );

        llm.chat(&synth_system_prompt, &prompt, memories, &[]).await
    }
}

#[cfg(test)]
#[allow(clippy::items_after_test_module)]
mod tests {
    use super::*;
    use chrono::Utc;

    fn degraded_result(status: DelegationStatus) -> AgentExecResult {
        let failure_kind = match status {
            DelegationStatus::TimedOut => FailureKind::Timeout,
            DelegationStatus::Panicked => FailureKind::Panic,
            _ => FailureKind::DelegationFailed,
        };
        AgentExecResult {
            agent_id: "agent:test".to_string(),
            agent_type: "planner".to_string(),
            task: "do something".to_string(),
            is_specialist: false,
            agent_name: Some("Turing".to_string()),
            model_name: "test-model".to_string(),
            content: "Agent failed: timeout".to_string(),
            llm_response: None,
            execution_time_ms: 0,
            status,
            failure_kind: Some(failure_kind),
            next_action_hint: Some("retry".to_string()),
            confidence: None,
            artifacts: Vec::new(),
        }
    }

    fn test_model_slot(id: &str, role: ModelRole, model: &str) -> ModelSlot {
        ModelSlot {
            id: id.to_string(),
            label: id.to_string(),
            role,
            provider: crate::core::LlmProvider::Ollama {
                base_url: "http://127.0.0.1:11434".to_string(),
                model: model.to_string(),
            },
            enabled: true,
            capability_tier: crate::core::config::ModelCapabilityTier::Balanced,
            cost_tier: crate::core::config::ModelCostTier::Medium,
            auto_escalate: true,
            escalation_rank: 0,
            health_scope: crate::core::config::ModelHealthScope::Provider,
        }
    }

    fn build_model_pool(slots: &[ModelSlot]) -> HashMap<String, (ModelSlot, LlmClient)> {
        let mut pool = HashMap::new();
        for slot in slots.iter().cloned().rev() {
            let client = LlmClient::new(&slot.provider).expect("test llm client");
            pool.insert(slot.id.clone(), (slot, client));
        }
        pool
    }

    #[test]
    fn summarize_delegation_status_returns_partial_when_some_paths_succeed() {
        let results = vec![
            AgentExecResult {
                agent_id: "agent:coder".to_string(),
                agent_type: "coder".to_string(),
                task: "ship".to_string(),
                is_specialist: false,
                agent_name: Some("Curie".to_string()),
                model_name: "test-model".to_string(),
                content: "ok".to_string(),
                llm_response: None,
                execution_time_ms: 12,
                status: DelegationStatus::Completed,
                failure_kind: None,
                next_action_hint: None,
                confidence: Some(1.0),
                artifacts: Vec::new(),
            },
            degraded_result(DelegationStatus::TimedOut),
        ];

        assert_eq!(
            summarize_delegation_status(&results),
            DelegationStatus::Partial
        );
    }

    #[test]
    fn build_delegation_degradation_includes_failed_paths() {
        let degradation =
            build_delegation_degradation(&[degraded_result(DelegationStatus::Panicked)]);

        assert_eq!(degradation.len(), 1);
        assert_eq!(degradation[0].kind, "delegation");
        assert!(degradation[0]
            .detail
            .as_deref()
            .unwrap_or_default()
            .contains("panicked"));
    }

    #[test]
    fn ephemeral_worker_tool_calls_are_not_completed_by_text() {
        let (status, failure_kind, next_action_hint, result_text) =
            classify_ephemeral_worker_response("I can do that after I inspect the file.", 1);

        assert_eq!(status, DelegationStatus::Failed);
        assert_eq!(failure_kind, Some(FailureKind::ToolContractFailure));
        assert!(next_action_hint.is_some());
        assert!(result_text.contains("requested 1 tool call"));
    }

    #[test]
    fn fallback_delegation_response_exposes_partial_completion() {
        let response = build_fallback_delegation_response(
            "Ship the feature",
            &[
                AgentExecResult {
                    agent_id: "agent:coder".to_string(),
                    agent_type: "coder".to_string(),
                    task: "implement".to_string(),
                    is_specialist: false,
                    agent_name: Some("Curie".to_string()),
                    model_name: "test-model".to_string(),
                    content: "Patched the core runtime.".to_string(),
                    llm_response: None,
                    execution_time_ms: 8,
                    status: DelegationStatus::Completed,
                    failure_kind: None,
                    next_action_hint: None,
                    confidence: Some(1.0),
                    artifacts: Vec::new(),
                },
                degraded_result(DelegationStatus::TimedOut),
            ],
        );

        assert!(response.content.contains("Completed so far"));
        assert!(response.content.contains("Still needs follow-up"));
    }

    #[test]
    fn fallback_delegation_response_handles_total_failure() {
        let response = build_fallback_delegation_response(
            "Ship the feature",
            &[degraded_result(DelegationStatus::TimedOut)],
        );

        assert!(response
            .content
            .contains("couldn't complete the delegated execution cleanly"));
        assert!(response.content.contains("Still needs follow-up"));
    }

    #[test]
    fn delegated_task_packet_render_includes_scoped_context_sections() {
        let packet = DelegatedTaskPacket {
            delegation_id: "run-123".to_string(),
            agent_id: "agent-1".to_string(),
            agent_name: "Forge".to_string(),
            agent_role: "Coder".to_string(),
            assignment_index: 2,
            total_assignments: 3,
            original_request: "Implement the pgvector-backed memory lookup path.".to_string(),
            assigned_task: "Patch the retrieval layer and validate the query path.".to_string(),
            coordinator_notes: "Keep the change local to the current workspace.".to_string(),
            dependency_outputs: vec![DelegatedDependencyPacket {
                sequence: 1,
                agent_name: "Helix".to_string(),
                agent_role: "Planner".to_string(),
                task: "Break the work into execution steps.".to_string(),
                status: "completed".to_string(),
                output_summary: "Identified retrieval wiring and validation as the critical path."
                    .to_string(),
                failure_kind: None,
                next_action_hint: None,
            }],
            relevant_memories: vec![DelegatedMemoryPacket {
                memory_type: "semantic".to_string(),
                content: "The memory layer now uses Postgres plus pgvector embeddings.".to_string(),
                timestamp: Utc::now().to_rfc3339(),
                relevance_score: 0.91,
                importance: 0.78,
                final_score: 0.88,
            }],
            action_scope: vec![DelegatedActionPacket {
                name: "file_write".to_string(),
                description: "Write changes to a workspace file.".to_string(),
                role: "mutation".to_string(),
                integration_class: "filesystem".to_string(),
                side_effect_level: "write".to_string(),
                requires_auth: false,
            }],
            execution_contract: vec!["Stay within the assigned task.".to_string()],
        };

        let rendered = packet.render_markdown();
        assert!(rendered.contains("## Delegated Task Packet"));
        assert!(rendered.contains("## Dependency Outputs"));
        assert!(rendered.contains("## Relevant Memory"));
        assert!(rendered.contains("## Action Context"));
        assert!(rendered.contains("Forge"));
        assert!(rendered.contains("file_write"));
    }

    #[test]
    fn select_llm_for_spec_uses_primary_when_smart_routing_is_disabled() {
        let router = TaskRouter::new(TaskRouterConfig::default());
        let configured_slots = vec![
            test_model_slot("primary", ModelRole::Primary, "primary-model"),
            test_model_slot("code", ModelRole::Code, "code-model"),
        ];
        let model_pool = build_model_pool(&configured_slots);
        let spec = SubAgentSpec {
            agent_type: "coder".to_string(),
            task: "Patch the backend".to_string(),
            preferred_model_role: Some("code".to_string()),
            depends_on: Vec::new(),
            plan_step_id: None,
        };

        let selected = router.select_llm_for_spec(
            &spec,
            &SubAgentType::Coder,
            &configured_slots,
            &model_pool,
            "primary",
            None,
            false,
            model_pool
                .get("primary")
                .map(|(_, client)| client)
                .expect("primary llm"),
        );

        assert_eq!(selected.model_name(), "primary-model");
    }

    #[test]
    fn select_llm_for_spec_honors_user_selected_slot() {
        let router = TaskRouter::new(TaskRouterConfig::default());
        let configured_slots = vec![
            test_model_slot("primary", ModelRole::Primary, "primary-model"),
            test_model_slot("research", ModelRole::Research, "research-model"),
            test_model_slot("fast", ModelRole::Fast, "fast-model"),
        ];
        let model_pool = build_model_pool(&configured_slots);
        let spec = SubAgentSpec {
            agent_type: "researcher".to_string(),
            task: "Investigate the production issue".to_string(),
            preferred_model_role: Some("research".to_string()),
            depends_on: Vec::new(),
            plan_step_id: None,
        };

        let selected = router.select_llm_for_spec(
            &spec,
            &SubAgentType::Researcher,
            &configured_slots,
            &model_pool,
            "primary",
            Some("fast"),
            true,
            model_pool
                .get("primary")
                .map(|(_, client)| client)
                .expect("primary llm"),
        );

        assert_eq!(selected.model_name(), "fast-model");
    }

    #[test]
    fn select_memories_for_task_prefers_overlap_with_task_text() {
        let router = TaskRouter::new(TaskRouterConfig::default());
        let memories = vec![
            PromptMemory {
                content: "Unrelated brainstorming note about vacation photos.".to_string(),
                memory_type: "learned_fact".to_string(),
                timestamp: Utc::now(),
                relevance_score: 0.85,
                importance: 0.9,
                final_score: 0.88,
            },
            PromptMemory {
                content: "The pgvector retrieval path uses similarity search in Postgres."
                    .to_string(),
                memory_type: "knowledge".to_string(),
                timestamp: Utc::now(),
                relevance_score: 0.60,
                importance: 0.6,
                final_score: 0.62,
            },
        ];

        let selected = router
            .select_memories_for_task("Fix the pgvector retrieval path in Postgres.", &memories);

        assert_eq!(
            selected.first().map(|memory| memory.content.as_str()),
            Some("The pgvector retrieval path uses similarity search in Postgres.")
        );
    }

    // -- memory_overlap_bonus: word-set coverage replaces substring scan --

    #[test]
    fn memory_overlap_bonus_returns_zero_on_empty_inputs() {
        assert_eq!(memory_overlap_bonus("", "anything goes here"), 0.0);
        assert_eq!(memory_overlap_bonus("anything goes here", ""), 0.0);
    }

    #[test]
    fn memory_overlap_bonus_rewards_paraphrased_recall() {
        // The substring scan returned 0 here (no contiguous substring of the
        // task is present verbatim in the memory). Set-based coverage finds
        // the shared meaningful vocabulary regardless of ordering.
        let task = "fix the pgvector retrieval path in postgres";
        let memory =
            "in postgres, the pgvector retrieval path is wired through the index lookup helpers";
        let bonus = memory_overlap_bonus(task, memory);
        assert!(bonus > 0.0, "expected non-zero bonus, got {bonus}");
        assert!(bonus <= 0.45, "bonus must stay capped, got {bonus}");
    }

    #[test]
    fn memory_overlap_bonus_does_not_reward_inner_substring_fragments() {
        // The previous substring scan would treat "log" as overlapping with
        // "login" — a classic false positive. Tokenising on word boundaries
        // eliminates it: only "application" is a shared meaningful token here.
        let task = "make the application log";
        let memory = "the application uses the login system";
        let bonus = memory_overlap_bonus(task, memory);
        assert!(bonus <= 0.45);
        // And the score reflects only the one real shared token, not two.
        let task_only_real_overlap = "make the application";
        let lower_bound = memory_overlap_bonus(task_only_real_overlap, memory);
        assert!((bonus - lower_bound).abs() < f32::EPSILON.max(0.001));
    }

    // -- classify_agent_failure: structural typed dispatch replaces phrase scan --

    #[tokio::test]
    async fn classify_agent_failure_detects_typed_timeout() {
        let elapsed = tokio::time::timeout(
            std::time::Duration::ZERO,
            tokio::time::sleep(std::time::Duration::from_secs(3600)),
        )
        .await
        .unwrap_err();
        let err = anyhow::Error::new(elapsed).context("Specialist deadline elapsed");
        let (status, kind, _hint) = classify_agent_failure(&err);
        assert_eq!(status, DelegationStatus::TimedOut);
        assert_eq!(kind, FailureKind::Timeout);
    }

    #[test]
    fn classify_agent_failure_does_not_misclassify_other_errors() {
        let err = anyhow::anyhow!("Specialist error: something else broke");
        let (status, kind, _hint) = classify_agent_failure(&err);
        assert_eq!(status, DelegationStatus::Failed);
        assert_eq!(kind, FailureKind::DelegationFailed);
    }

    #[test]
    fn classify_agent_failure_ignores_phrase_resemblance() {
        // The old substring-based classifier would have flagged this as a
        // timeout because the message *contains* "timed out". The structural
        // typed dispatch ignores phrasing entirely and falls back to the
        // generic delegation-failure case, which is the correct outcome.
        let err =
            anyhow::anyhow!("Specialist error: the upstream connection timed out at the proxy");
        let (status, kind, _hint) = classify_agent_failure(&err);
        assert_eq!(status, DelegationStatus::Failed);
        assert_eq!(kind, FailureKind::DelegationFailed);
    }
}

// -- Internal types --

struct AgentAssignment {
    agent_id: String,
    spec: SubAgentSpec,
    agent_type: SubAgentType,
    display_name: String,
    model_name: String,
    kind: AssignmentKind,
}

enum AssignmentKind {
    Specialist(Arc<SpecialistAgent>),
    Ephemeral(LlmClient),
}
