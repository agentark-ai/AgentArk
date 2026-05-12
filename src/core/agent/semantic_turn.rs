use super::*;
use std::collections::{BTreeSet, HashMap, HashSet};
use std::hash::{Hash, Hasher};
use std::sync::Arc;

pub(super) const SEMANTIC_TURN_PLAN_VERSION: &str = "semantic_turn_plan_v1";

#[derive(Debug, Clone, Copy, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "snake_case")]
pub(super) enum StepSideEffect {
    Read,
    Notify,
    CreateObject,
    ModifyObject,
    DeleteObject,
    None,
    #[serde(other)]
    Unknown,
}

impl Default for StepSideEffect {
    fn default() -> Self {
        Self::Unknown
    }
}

#[derive(Debug, Clone, Copy, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "snake_case")]
pub(super) enum GoalDelivery {
    Chat,
    App,
    File,
    Background,
    External,
    Mixed,
    #[serde(other)]
    Unknown,
}

impl Default for GoalDelivery {
    fn default() -> Self {
        Self::Unknown
    }
}

#[derive(Debug, Clone, Copy, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "snake_case")]
pub(super) enum GoalFreshness {
    None,
    CurrentLookup,
    ArtifactRuntimeRefresh,
    DurableMonitor,
    Scheduled,
    #[serde(other)]
    Unknown,
}

impl Default for GoalFreshness {
    fn default() -> Self {
        Self::Unknown
    }
}

#[derive(Debug, Clone, Copy, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "snake_case")]
pub(super) enum GoalAuthorization {
    None,
    LocalState,
    IntegrationGrant,
    UserApproval,
    SecretSidecar,
    #[serde(other)]
    Unknown,
}

impl Default for GoalAuthorization {
    fn default() -> Self {
        Self::Unknown
    }
}

#[derive(Debug, Clone, Serialize, Deserialize, Default)]
pub(super) struct TurnRequirement {
    pub id: String,
    pub text: String,
}

#[derive(Debug, Clone, Serialize, Deserialize, Default)]
pub(super) struct SemanticObjectRef {
    pub kind: String,
    #[serde(default)]
    pub resolution_family: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub label: Option<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize, Default)]
pub(super) struct GoalRisk {
    #[serde(default)]
    pub level: String,
    #[serde(default)]
    pub reasons: Vec<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize, Default)]
pub(super) struct GoalHints {
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub workspace_modification_request: Option<bool>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub public_freshness_required: Option<bool>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub read_only: Option<bool>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub preserve_existing_object: Option<bool>,
}

#[derive(Debug, Clone, Serialize, Deserialize, Default)]
pub(super) struct SemanticGoal {
    pub goal_id: String,
    pub outcome: String,
    pub capability_need: String,
    #[serde(default)]
    pub side_effect: StepSideEffect,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub object_ref: Option<SemanticObjectRef>,
    #[serde(default)]
    pub freshness: GoalFreshness,
    #[serde(default)]
    pub delivery: GoalDelivery,
    #[serde(default)]
    pub authorization: GoalAuthorization,
    #[serde(default)]
    pub risk: GoalRisk,
    #[serde(default)]
    pub dependencies: Vec<String>,
    #[serde(default)]
    pub success_criteria: Vec<String>,
    #[serde(default)]
    pub covered_requirement_ids: Vec<String>,
    #[serde(default)]
    pub hints: GoalHints,
    #[serde(default)]
    pub confidence: f32,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub(super) struct SemanticTurnPlan {
    pub plan_id: String,
    pub version: String,
    #[serde(default)]
    pub turn_summary: String,
    #[serde(default)]
    pub clarification_needed: bool,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub clarification_question: Option<String>,
    #[serde(default)]
    pub requires_secret_sidecar: bool,
    #[serde(default)]
    pub explicit_requirements: Vec<TurnRequirement>,
    #[serde(default)]
    pub goals: Vec<SemanticGoal>,
    #[serde(default)]
    pub confidence: f32,
}

impl Default for SemanticTurnPlan {
    fn default() -> Self {
        Self {
            plan_id: format!("turn-plan-{}", uuid::Uuid::new_v4()),
            version: SEMANTIC_TURN_PLAN_VERSION.to_string(),
            turn_summary: String::new(),
            clarification_needed: false,
            clarification_question: None,
            requires_secret_sidecar: false,
            explicit_requirements: Vec::new(),
            goals: Vec::new(),
            confidence: 0.0,
        }
    }
}

#[derive(Debug, Clone, Serialize, Deserialize, Default)]
pub(super) struct PlanVerification {
    pub accepted: bool,
    #[serde(default)]
    pub missing_requirement_ids: Vec<String>,
    #[serde(default)]
    pub extra_goal_ids: Vec<String>,
    #[serde(default)]
    pub contradictions: Vec<String>,
    #[serde(default)]
    pub must_clarify: bool,
    #[serde(default)]
    pub risk_notes: Vec<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub(super) struct CapabilityCandidate {
    pub action_name: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub descriptor_hash: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub cosine_distance: Option<f64>,
    pub side_effect_level: crate::actions::ActionSideEffectLevel,
    pub role: crate::actions::ActionRole,
    pub tool_role: crate::actions::ActionToolRole,
    pub integration_class: crate::actions::ActionIntegrationClass,
    pub requires_auth: bool,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub readiness: Option<super::capability_health::CapabilityReadiness>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub contract_fit: Option<super::tool_contracts::ToolContractFit>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub(super) struct CapabilityGraphNode {
    pub action_name: String,
    pub descriptor_hash: String,
    pub descriptor_text_preview: String,
    pub metadata: crate::actions::ActionMetadata,
    #[serde(default)]
    pub required_shapes: Vec<String>,
    #[serde(default)]
    pub capabilities: Vec<String>,
}

#[derive(Debug, Clone, Default, Serialize, Deserialize)]
pub(super) struct CapabilityGraph {
    pub nodes: Vec<CapabilityGraphNode>,
}

impl CapabilityGraph {
    fn from_actions(actions: &[crate::actions::ActionDef]) -> Self {
        let nodes = actions
            .iter()
            .map(|action| {
                let descriptor = crate::core::action_catalog::build_action_catalog_descriptor(action);
                CapabilityGraphNode {
                    action_name: action.name.clone(),
                    descriptor_hash: descriptor.descriptor_hash,
                    descriptor_text_preview: safe_truncate(&descriptor.descriptor_text, 900),
                    metadata: action.action_metadata(),
                    required_shapes:
                        crate::core::action_catalog::action_schema_required_shape_descriptions(
                            &action.input_schema,
                            6,
                        ),
                    capabilities: action.capabilities.clone(),
                }
            })
            .collect();
        Self { nodes }
    }

    fn by_name(&self) -> HashMap<&str, &CapabilityGraphNode> {
        self.nodes
            .iter()
            .map(|node| (node.action_name.as_str(), node))
            .collect()
    }
}

#[derive(Debug, Clone)]
pub(super) struct CapabilitySnapshot {
    pub actions: Arc<Vec<crate::actions::ActionDef>>,
    pub graph: Arc<CapabilityGraph>,
    pub generated_at: chrono::DateTime<chrono::Utc>,
    pub generation: usize,
    pub fingerprint: String,
    pub load_ms: u64,
    pub descriptor_ms: u64,
    pub cache_hit: bool,
}

impl CapabilitySnapshot {
    fn fresh_clone(&self, cache_hit: bool) -> Self {
        Self {
            actions: Arc::clone(&self.actions),
            graph: Arc::clone(&self.graph),
            generated_at: self.generated_at,
            generation: self.generation,
            fingerprint: self.fingerprint.clone(),
            load_ms: self.load_ms,
            descriptor_ms: self.descriptor_ms,
            cache_hit,
        }
    }

    pub(super) fn trace_payload(&self) -> serde_json::Value {
        serde_json::json!({
            "actions": self.actions.len(),
            "graph_nodes": self.graph.nodes.len(),
            "generated_at": self.generated_at.to_rfc3339(),
            "generation": self.generation,
            "fingerprint": self.fingerprint.clone(),
            "cache_hit": self.cache_hit,
            "load_ms": self.load_ms,
            "descriptor_ms": self.descriptor_ms,
            "ttl_ms": capability_snapshot_ttl_ms(),
        })
    }
}

#[derive(Debug, Clone, Serialize, Deserialize, Default)]
pub(super) struct ResolvedStep {
    pub goal_id: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub action_name: Option<String>,
    #[serde(default)]
    pub candidates: Vec<CapabilityCandidate>,
    #[serde(default)]
    pub respond_without_tool: bool,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub fallback_reason: Option<String>,
    #[serde(default)]
    pub side_effect_compatible: bool,
    #[serde(default)]
    pub input_gaps: Vec<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize, Default)]
pub(super) struct ResultVerification {
    pub all_goals_accounted: bool,
    #[serde(default)]
    pub completed_goal_ids: Vec<String>,
    #[serde(default)]
    pub pending_goal_ids: Vec<String>,
    #[serde(default)]
    pub unsupported_claims: Vec<String>,
    #[serde(default)]
    pub evidence_notes: Vec<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize, Default)]
pub(super) struct SemanticTurnBundle {
    pub plan: SemanticTurnPlan,
    pub verification: PlanVerification,
    pub resolved_steps: Vec<ResolvedStep>,
    #[serde(default)]
    pub planner_degraded: bool,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub planner_error: Option<String>,
}

impl SemanticTurnBundle {
    pub(super) fn resolved_action_names(&self) -> Vec<String> {
        let mut seen = HashSet::new();
        let mut out = Vec::new();
        for step in &self.resolved_steps {
            if let Some(action_name) = step.action_name.as_deref() {
                if seen.insert(action_name.to_string()) {
                    out.push(action_name.to_string());
                }
            }
            for candidate in &step.candidates {
                if seen.insert(candidate.action_name.clone()) {
                    out.push(candidate.action_name.clone());
                }
            }
        }
        out
    }

    pub(super) fn capability_probe_texts(&self) -> Vec<String> {
        let mut probes = Vec::new();
        for goal in &self.plan.goals {
            let mut parts = Vec::new();
            parts.push(format!("goal_outcome: {}", safe_truncate(&goal.outcome, 360)));
            parts.push(format!(
                "capability_need: {}",
                safe_truncate(&goal.capability_need, 360)
            ));
            parts.push(format!("side_effect: {:?}", goal.side_effect));
            parts.push(format!("delivery: {:?}", goal.delivery));
            parts.push(format!("freshness: {:?}", goal.freshness));
            if let Some(object_ref) = &goal.object_ref {
                parts.push(format!(
                    "object_ref: {} {}",
                    object_ref.kind, object_ref.resolution_family
                ));
            }
            if !goal.success_criteria.is_empty() {
                parts.push(format!(
                    "success_criteria: {}",
                    goal.success_criteria
                        .iter()
                        .take(6)
                        .map(|value| safe_truncate(value, 160))
                        .collect::<Vec<_>>()
                        .join(" | ")
                ));
            }
            probes.push(parts.join("\n"));
        }
        probes
    }

    pub(super) fn execution_plan(&self) -> crate::core::ExecutionPlan {
        crate::core::ExecutionPlan {
            plan_id: self.plan.plan_id.clone(),
            revision: 1,
            summary: if self.plan.turn_summary.trim().is_empty() {
                "Semantic turn plan".to_string()
            } else {
                self.plan.turn_summary.clone()
            },
            steps: self
                .plan
                .goals
                .iter()
                .enumerate()
                .map(|(index, goal)| {
                    let resolved = self
                        .resolved_steps
                        .iter()
                        .find(|step| step.goal_id == goal.goal_id);
                    crate::core::PlanStep {
                        id: index + 1,
                        title: if goal.outcome.trim().is_empty() {
                            format!("Goal {}", index + 1)
                        } else {
                            safe_truncate(&goal.outcome, 120)
                        },
                        description: safe_truncate(&goal.capability_need, 500),
                        action: resolved.and_then(|step| step.action_name.clone()),
                        arguments: None,
                        tool_hint: Some(format!(
                            "side_effect={:?}; delivery={:?}; freshness={:?}",
                            goal.side_effect, goal.delivery, goal.freshness
                        )),
                        status: Some(crate::core::PlanStepStatus::Pending),
                        substeps: goal
                            .success_criteria
                            .iter()
                            .enumerate()
                            .map(|(sub_index, criterion)| crate::core::PlanSubstep {
                                id: sub_index + 1,
                                title: safe_truncate(criterion, 120),
                                description: String::new(),
                                tool_hint: None,
                                status: Some(crate::core::PlanStepStatus::Pending),
                            })
                            .collect(),
                    }
                })
                .collect(),
        }
    }

    pub(super) fn trace_steps(&self, result: Option<&ResultVerification>) -> Vec<ExecutionStep> {
        let now = chrono::Utc::now();
        let mut steps = vec![
            ExecutionStep {
                icon: "[plan]".to_string(),
                title: "Semantic Turn Plan".to_string(),
                detail: format!(
                    "{} goal(s), {} explicit requirement(s).",
                    self.plan.goals.len(),
                    self.plan.explicit_requirements.len()
                ),
                step_type: if self.planner_degraded { "warning" } else { "info" }.to_string(),
                data: serde_json::to_string(&self.plan).ok(),
                timestamp: now,
                duration_ms: Some(0),
            },
            ExecutionStep {
                icon: "[verify]".to_string(),
                title: "Plan Verification".to_string(),
                detail: if self.verification.accepted {
                    "Plan passed structural coverage checks.".to_string()
                } else if self.verification.must_clarify {
                    "Plan requires user clarification before execution.".to_string()
                } else {
                    "Plan had structural gaps; continuing with guarded execution context."
                        .to_string()
                },
                step_type: if self.verification.accepted {
                    "success"
                } else if self.verification.must_clarify {
                    "warning"
                } else {
                    "info"
                }
                .to_string(),
                data: serde_json::to_string(&self.verification).ok(),
                timestamp: now,
                duration_ms: Some(0),
            },
            ExecutionStep {
                icon: "[resolve]".to_string(),
                title: "Capability Resolution".to_string(),
                detail: format!(
                    "{} resolved step(s), {} action candidate(s).",
                    self.resolved_steps.len(),
                    self.resolved_steps
                        .iter()
                        .map(|step| step.candidates.len())
                        .sum::<usize>()
                ),
                step_type: "info".to_string(),
                data: serde_json::to_string(&self.resolved_steps).ok(),
                timestamp: now,
                duration_ms: Some(0),
            },
        ];
        if let Some(result) = result {
            steps.push(ExecutionStep {
                icon: "[result]".to_string(),
                title: "Result Verification".to_string(),
                detail: if result.all_goals_accounted {
                    "All semantic goals have visible result evidence or a user-facing answer."
                        .to_string()
                } else {
                    format!(
                        "{} goal(s) still lack visible result evidence.",
                        result.pending_goal_ids.len()
                    )
                },
                step_type: if result.all_goals_accounted {
                    "success"
                } else {
                    "warning"
                }
                .to_string(),
                data: serde_json::to_string(result).ok(),
                timestamp: chrono::Utc::now(),
                duration_ms: Some(0),
            });
        }
        steps
    }
}

pub(super) async fn build_semantic_turn_bundle(
    agent: &Agent,
    channel: &str,
    message: &str,
    conversation_key: &str,
    packed_context: &super::conversation_context::PackedConversationContext,
    recent_artifacts: &[ConversationArtifactContext],
    pending_actions: &[PendingConversationAction],
    background_sessions: &[crate::core::background_session::BackgroundSession],
    watchers: &[crate::core::watcher::Watcher],
    active_workspace_snapshot: Option<&serde_json::Value>,
    context_ledger: &super::context_ledger::ConversationContextLedger,
    capability_snapshot: Option<&CapabilitySnapshot>,
    request_hints: &RequestExecutionHints,
) -> SemanticTurnBundle {
    let plan_result = build_semantic_turn_plan(
        agent,
        channel,
        message,
        conversation_key,
        packed_context,
        recent_artifacts,
        pending_actions,
        background_sessions,
        watchers,
        active_workspace_snapshot,
        context_ledger,
        request_hints,
    )
    .await;

    let (plan, planner_degraded, planner_error) = match plan_result {
        Ok(plan) => (normalize_turn_plan(plan), false, None),
        Err(error) => (
            fallback_turn_plan(message),
            true,
            Some(safe_truncate(&error.to_string(), 500)),
        ),
    };
    let verification = verify_turn_plan(&plan);
    let resolved_steps = if plan_can_respond_without_tools(&plan) {
        direct_response_resolved_steps(&plan)
    } else if let Some(capability_snapshot) = capability_snapshot {
        resolve_capabilities(agent, &plan, capability_snapshot, None).await
    } else {
        Vec::new()
    };
    SemanticTurnBundle {
        plan,
        verification,
        resolved_steps,
        planner_degraded,
        planner_error,
    }
}

pub(super) async fn resolve_semantic_turn_capabilities(
    agent: &Agent,
    bundle: &mut SemanticTurnBundle,
    capability_snapshot: &CapabilitySnapshot,
    capability_health: Option<&super::capability_health::CapabilityHealthSnapshot>,
) {
    bundle.resolved_steps = if plan_can_respond_without_tools(&bundle.plan) {
        direct_response_resolved_steps(&bundle.plan)
    } else {
        resolve_capabilities(agent, &bundle.plan, capability_snapshot, capability_health).await
    };
}

fn capability_snapshot_ttl_ms() -> i64 {
    std::env::var("AGENTARK_CAPABILITY_SNAPSHOT_TTL_MS")
        .ok()
        .and_then(|value| value.trim().parse::<i64>().ok())
        .unwrap_or(120_000)
        .clamp(1_000, 3_600_000)
}

fn capability_snapshot_is_fresh(snapshot: &CapabilitySnapshot, generation: usize) -> bool {
    let age_ms = chrono::Utc::now()
        .signed_duration_since(snapshot.generated_at)
        .num_milliseconds();
    snapshot.generation == generation && age_ms >= 0 && age_ms <= capability_snapshot_ttl_ms()
}

impl Agent {
    pub(super) async fn invalidate_capability_snapshot(&self, reason: &'static str) {
        self.capability_snapshot_generation
            .fetch_add(1, std::sync::atomic::Ordering::AcqRel);
        *self.capability_snapshot.write().await = None;
        self.invalidate_capability_health_snapshot(reason).await;
        tracing::debug!(reason, "Capability snapshot invalidated");
    }

    pub(super) async fn load_capability_snapshot(&self) -> anyhow::Result<CapabilitySnapshot> {
        let generation = self
            .capability_snapshot_generation
            .load(std::sync::atomic::Ordering::Acquire);
        if let Some(snapshot) = self
            .capability_snapshot
            .read()
            .await
            .as_ref()
            .filter(|snapshot| capability_snapshot_is_fresh(snapshot, generation))
            .map(|snapshot| snapshot.fresh_clone(true))
        {
            return Ok(snapshot);
        }

        let _refresh_guard = self.capability_snapshot_refresh.lock().await;
        if let Some(snapshot) = self
            .capability_snapshot
            .read()
            .await
            .as_ref()
            .filter(|snapshot| capability_snapshot_is_fresh(snapshot, generation))
            .map(|snapshot| snapshot.fresh_clone(true))
        {
            return Ok(snapshot);
        }

        let load_started = std::time::Instant::now();
        let actions = self.load_action_catalog_actions().await?;
        let load_ms = load_started.elapsed().as_millis() as u64;
        let fingerprint = capability_actions_fingerprint(&actions);

        let descriptor_started = std::time::Instant::now();
        let graph = CapabilityGraph::from_actions(&actions);
        let descriptor_ms = descriptor_started.elapsed().as_millis() as u64;

        let snapshot = CapabilitySnapshot {
            actions: Arc::new(actions),
            graph: Arc::new(graph),
            generated_at: chrono::Utc::now(),
            generation,
            fingerprint,
            load_ms,
            descriptor_ms,
            cache_hit: false,
        };
        *self.capability_snapshot.write().await = Some(snapshot.fresh_clone(false));
        Ok(snapshot)
    }
}

fn capability_actions_fingerprint(actions: &[crate::actions::ActionDef]) -> String {
    let mut summaries = actions
        .iter()
        .map(|action| {
            let mut capabilities = action.capabilities.clone();
            capabilities.sort();
            let metadata = action.action_metadata();
            format!(
                "{}\u{1f}{}\u{1f}{:?}\u{1f}{:?}\u{1f}{:?}\u{1f}{:?}\u{1f}{:?}\u{1f}{}\u{1f}{}",
                action.name,
                action.version,
                action.source,
                metadata.role,
                metadata.tool_role,
                metadata.integration_class,
                metadata.side_effect_level,
                metadata.requires_auth,
                capabilities.join("\u{1e}")
            )
        })
        .collect::<Vec<_>>();
    summaries.sort();
    let mut hasher = std::collections::hash_map::DefaultHasher::new();
    summaries.hash(&mut hasher);
    format!("{:016x}", hasher.finish())
}

#[allow(clippy::too_many_arguments)]
async fn build_semantic_turn_plan(
    agent: &Agent,
    channel: &str,
    message: &str,
    conversation_key: &str,
    packed_context: &super::conversation_context::PackedConversationContext,
    recent_artifacts: &[ConversationArtifactContext],
    pending_actions: &[PendingConversationAction],
    background_sessions: &[crate::core::background_session::BackgroundSession],
    watchers: &[crate::core::watcher::Watcher],
    active_workspace_snapshot: Option<&serde_json::Value>,
    context_ledger: &super::context_ledger::ConversationContextLedger,
    request_hints: &RequestExecutionHints,
) -> anyhow::Result<SemanticTurnPlan> {
    let response = agent
        .supervised_internal_chat_detailed(
            channel,
            "semantic_turn_planner",
            SEMANTIC_TURN_PLAN_VERSION,
            &ModelRole::Fast,
            agent.llm_candidates_for_role(&ModelRole::Fast),
            semantic_turn_planner_system_prompt().as_str(),
            &semantic_turn_planner_user_prompt(
                message,
                conversation_key,
                packed_context,
                recent_artifacts,
                pending_actions,
                background_sessions,
                watchers,
                active_workspace_snapshot,
                context_ledger,
                request_hints,
            ),
            &[],
            &[],
            internal_llm_timeout_ms("AGENTARK_SEMANTIC_TURN_PLAN_TIMEOUT_MS", 45_000),
            super::agent_loop::agent_loop_max_candidates(),
        )
        .await
        .map_err(|outcome| anyhow::anyhow!(outcome.message))?;
    let value = extract_json_value(&response.content)
        .ok_or_else(|| anyhow::anyhow!("semantic turn planner did not return JSON"))?;
    serde_json::from_value::<SemanticTurnPlan>(value)
        .map_err(|error| anyhow::anyhow!("semantic turn plan JSON was invalid: {error}"))
}

fn semantic_turn_planner_system_prompt() -> String {
    format!(
        r#"You are the semantic TurnPlanner for {product}.

Return only compact JSON matching the requested schema. Do not include markdown.
Plan by meaning and constraints, not by exact words, keyword bundles, casing, punctuation, or anticipated phrasing.
One user turn may contain zero, one, or many independent or dependent goals. Preserve all requested outcomes.
Use conversation history and runtime state only to resolve references, continuations, corrections, approvals, and dependencies. The current user turn wins when it changes intent.
Do not choose exact tool names. Emit semantic goals with capability_need, side_effect, object_ref, dependencies, success criteria, and covered requirement ids.
For ordinary conversation, explanation, planning, review, or advice that does not require live/private state, durable side effects, or an external lookup, use side_effect=none, freshness=none, delivery=chat, authorization=none.
Separate examples, transcripts, and background text from actual requested outcomes when the current turn makes that distinction.
If a required detail blocks a specific outcome, set clarification_needed=true, goals=[], and ask only for the missing detail.
Secrets must be represented only through requires_secret_sidecar=true and authorization=secret_sidecar; never reproduce secret values."#,
        product = crate::branding::PRODUCT_NAME
    )
}

#[allow(clippy::too_many_arguments)]
fn semantic_turn_planner_user_prompt(
    message: &str,
    conversation_key: &str,
    packed_context: &super::conversation_context::PackedConversationContext,
    recent_artifacts: &[ConversationArtifactContext],
    pending_actions: &[PendingConversationAction],
    background_sessions: &[crate::core::background_session::BackgroundSession],
    watchers: &[crate::core::watcher::Watcher],
    active_workspace_snapshot: Option<&serde_json::Value>,
    context_ledger: &super::context_ledger::ConversationContextLedger,
    request_hints: &RequestExecutionHints,
) -> String {
    let recent_messages = packed_context
        .history
        .iter()
        .rev()
        .take(8)
        .map(|message| {
            serde_json::json!({
                "role": &message.role,
                "content": safe_truncate(&crate::security::redact_secret_input(&message.content).text, 800),
                "timestamp": &message._timestamp,
            })
        })
        .collect::<Vec<_>>();
    let recent_messages = recent_messages.into_iter().rev().collect::<Vec<_>>();
    serde_json::json!({
        "schema": {
            "plan_id": "string",
            "version": SEMANTIC_TURN_PLAN_VERSION,
            "turn_summary": "short semantic summary",
            "clarification_needed": "boolean",
            "clarification_question": "string|null",
            "requires_secret_sidecar": "boolean",
            "explicit_requirements": [{"id": "req_0", "text": "requirement text"}],
            "goals": [{
                "goal_id": "goal_0",
                "outcome": "user-visible outcome",
                "capability_need": "semantic capability needed, not a tool name",
                "side_effect": "read|notify|create_object|modify_object|delete_object|none|unknown",
                "object_ref": {"kind": "object kind", "resolution_family": "by_id|most_recent_in_context|last_agent_created|by_description", "label": "optional label"},
                "freshness": "none|current_lookup|artifact_runtime_refresh|durable_monitor|scheduled|unknown",
                "delivery": "chat|app|file|background|external|mixed|unknown",
                "authorization": "none|local_state|integration_grant|user_approval|secret_sidecar|unknown",
                "risk": {"level": "none|low|medium|high|critical", "reasons": ["short reason"]},
                "dependencies": ["goal_0"],
                "success_criteria": ["observable completion condition"],
                "covered_requirement_ids": ["req_0"],
                "hints": {
                    "workspace_modification_request": true,
                    "public_freshness_required": false,
                    "read_only": false,
                    "preserve_existing_object": true
                },
                "confidence": 0.0
            }],
            "confidence": 0.0
        },
        "turn": {
            "conversation_id": conversation_key,
            "user_message": crate::security::redact_secret_input(message).text,
            "surface": &request_hints.execution_surface,
            "secret_offered": &request_hints.secret_offered,
            "attachments": super::agent_loop::attachment_hints_for_prompt(request_hints),
        },
        "conversation_context": {
            "digest": packed_context.digest.as_ref().map(|value| safe_truncate(&crate::security::redact_secret_input(value).text, 1200)),
            "recent_messages": recent_messages,
            "compacted_messages": packed_context.compacted_messages,
        },
        "runtime_state": {
            "pending_actions": pending_actions.iter().take(8).map(|action| serde_json::json!({
                "kind": action.kind.as_pending_action_kind(),
                "summary": safe_truncate(&crate::security::redact_secret_input(&action.summary).text, 240),
            })).collect::<Vec<_>>(),
            "background_sessions": background_sessions.iter().take(8).map(|session| serde_json::json!({
                "id": &session.id,
                "title": safe_truncate(&crate::security::redact_secret_input(&session.title).text, 180),
                "status": session.status.label(),
                "summary": session.summary.as_ref().map(|value| safe_truncate(&crate::security::redact_secret_input(value).text, 240)),
            })).collect::<Vec<_>>(),
            "watchers": watchers.iter().take(8).map(|watcher| serde_json::json!({
                "id": &watcher.id,
                "description": safe_truncate(&crate::security::redact_secret_input(&watcher.description).text, 240),
                "status": &watcher.status,
                "notify_channel": &watcher.notify_channel,
            })).collect::<Vec<_>>(),
            "conversation_ledger": context_ledger.compact_for_prompt(),
            "recent_artifacts": recent_artifacts.iter().take(8).map(|artifact| serde_json::json!({
                "artifact_type": &artifact.artifact_type,
                "title": safe_truncate(&crate::security::redact_secret_input(&artifact.title).text, 180),
            })).collect::<Vec<_>>(),
            "active_workspace_present": active_workspace_snapshot.is_some(),
        }
    })
    .to_string()
}

fn normalize_turn_plan(mut plan: SemanticTurnPlan) -> SemanticTurnPlan {
    if plan.plan_id.trim().is_empty() {
        plan.plan_id = format!("turn-plan-{}", uuid::Uuid::new_v4());
    }
    if plan.version.trim().is_empty() {
        plan.version = SEMANTIC_TURN_PLAN_VERSION.to_string();
    }
    normalize_requirements(&mut plan.explicit_requirements);
    normalize_goals(&mut plan.goals);
    plan
}

fn normalize_requirements(requirements: &mut [TurnRequirement]) {
    for (index, requirement) in requirements.iter_mut().enumerate() {
        if requirement.id.trim().is_empty() {
            requirement.id = format!("req_{index}");
        }
        requirement.id = requirement.id.trim().to_string();
        requirement.text = safe_truncate(requirement.text.trim(), 500);
    }
}

fn normalize_goals(goals: &mut [SemanticGoal]) {
    for (index, goal) in goals.iter_mut().enumerate() {
        if goal.goal_id.trim().is_empty() {
            goal.goal_id = format!("goal_{index}");
        }
        goal.goal_id = goal.goal_id.trim().to_string();
        goal.outcome = safe_truncate(goal.outcome.trim(), 600);
        goal.capability_need = safe_truncate(goal.capability_need.trim(), 600);
        goal.dependencies = dedup_trimmed(goal.dependencies.drain(..));
        goal.covered_requirement_ids = dedup_trimmed(goal.covered_requirement_ids.drain(..));
        goal.success_criteria = dedup_trimmed(goal.success_criteria.drain(..));
    }
}

fn dedup_trimmed(values: impl Iterator<Item = String>) -> Vec<String> {
    let mut seen = BTreeSet::new();
    values
        .filter_map(|value| {
            let value = safe_truncate(value.trim(), 500);
            if value.is_empty() || !seen.insert(value.clone()) {
                None
            } else {
                Some(value)
            }
        })
        .collect()
}

fn fallback_turn_plan(message: &str) -> SemanticTurnPlan {
    let redacted = crate::security::redact_secret_input(message).text;
    let requirement_text = safe_truncate(redacted.trim(), 500);
    SemanticTurnPlan {
        plan_id: format!("turn-plan-{}", uuid::Uuid::new_v4()),
        version: SEMANTIC_TURN_PLAN_VERSION.to_string(),
        turn_summary: "Handle the current user request.".to_string(),
        clarification_needed: false,
        clarification_question: None,
        requires_secret_sidecar: false,
        explicit_requirements: vec![TurnRequirement {
            id: "req_0".to_string(),
            text: requirement_text.clone(),
        }],
        goals: vec![SemanticGoal {
            goal_id: "goal_0".to_string(),
            outcome: "Handle the current user request.".to_string(),
            capability_need: requirement_text,
            side_effect: StepSideEffect::Unknown,
            covered_requirement_ids: vec!["req_0".to_string()],
            confidence: 0.0,
            ..Default::default()
        }],
        confidence: 0.0,
    }
}

pub(super) fn verify_turn_plan(plan: &SemanticTurnPlan) -> PlanVerification {
    let mut contradictions = Vec::new();
    let mut risk_notes = Vec::new();
    let mut requirement_ids = HashSet::new();
    for requirement in &plan.explicit_requirements {
        if requirement.id.trim().is_empty() {
            contradictions.push("explicit requirement has an empty id".to_string());
        } else if !requirement_ids.insert(requirement.id.clone()) {
            contradictions.push(format!("duplicate explicit requirement id {}", requirement.id));
        }
        if requirement.text.trim().is_empty() {
            contradictions.push(format!("requirement {} has empty text", requirement.id));
        }
    }

    if plan.clarification_needed {
        if plan
            .clarification_question
            .as_deref()
            .map(str::trim)
            .unwrap_or_default()
            .is_empty()
        {
            contradictions.push("clarification_needed=true without a question".to_string());
        }
        if !plan.goals.is_empty() {
            contradictions.push("clarification plan must not include executable goals".to_string());
        }
    }

    let mut goal_ids = HashSet::new();
    let mut covered_requirement_ids = HashSet::new();
    for goal in &plan.goals {
        if goal.goal_id.trim().is_empty() {
            contradictions.push("goal has an empty id".to_string());
        } else if !goal_ids.insert(goal.goal_id.clone()) {
            contradictions.push(format!("duplicate goal id {}", goal.goal_id));
        }
        if goal.outcome.trim().is_empty() {
            contradictions.push(format!("goal {} has an empty outcome", goal.goal_id));
        }
        if goal.capability_need.trim().is_empty() {
            contradictions.push(format!(
                "goal {} has an empty capability_need",
                goal.goal_id
            ));
        }
        if matches!(goal.side_effect, StepSideEffect::Unknown) {
            risk_notes.push(format!("goal {} has unknown side effect", goal.goal_id));
        }
        for covered in &goal.covered_requirement_ids {
            if !requirement_ids.contains(covered) {
                contradictions.push(format!(
                    "goal {} covers unknown requirement {}",
                    goal.goal_id, covered
                ));
            } else {
                covered_requirement_ids.insert(covered.clone());
            }
        }
        for dependency in &goal.dependencies {
            if dependency == &goal.goal_id {
                contradictions.push(format!("goal {} depends on itself", goal.goal_id));
            }
        }
    }
    for goal in &plan.goals {
        for dependency in &goal.dependencies {
            if !goal_ids.contains(dependency) {
                contradictions.push(format!(
                    "goal {} depends on unknown goal {}",
                    goal.goal_id, dependency
                ));
            }
        }
    }

    let missing_requirement_ids = plan
        .explicit_requirements
        .iter()
        .filter(|requirement| !covered_requirement_ids.contains(&requirement.id))
        .map(|requirement| requirement.id.clone())
        .collect::<Vec<_>>();
    let must_clarify = plan.clarification_needed;
    let accepted = contradictions.is_empty() && (must_clarify || missing_requirement_ids.is_empty());
    PlanVerification {
        accepted,
        missing_requirement_ids,
        extra_goal_ids: Vec::new(),
        contradictions,
        must_clarify,
        risk_notes,
    }
}

async fn resolve_capabilities(
    agent: &Agent,
    plan: &SemanticTurnPlan,
    capability_snapshot: &CapabilitySnapshot,
    capability_health: Option<&super::capability_health::CapabilityHealthSnapshot>,
) -> Vec<ResolvedStep> {
    let mut out = Vec::new();
    let graph = capability_snapshot.graph.as_ref();
    let by_name = graph.by_name();

    for goal in &plan.goals {
        if goal_can_respond_without_tool(goal) {
            out.push(ResolvedStep {
                goal_id: goal.goal_id.clone(),
                action_name: None,
                candidates: Vec::new(),
                respond_without_tool: true,
                fallback_reason: Some(
                    "semantic goal can be satisfied with a direct conversational response"
                        .to_string(),
                ),
                side_effect_compatible: true,
                input_gaps: Vec::new(),
            });
            continue;
        }
        let candidates =
            resolve_goal_candidates(
                agent,
                goal,
                graph,
                &by_name,
                capability_snapshot.actions.as_ref(),
                capability_health,
                capability_candidate_limit(),
            )
            .await;
        let selected = candidates
            .iter()
            .find(|candidate| {
                !candidate.tool_role.is_support()
                    && side_effect_matches(goal.side_effect, &candidate.side_effect_level)
            })
            .or_else(|| {
                candidates
                    .iter()
                    .find(|candidate| !candidate.tool_role.is_support())
            })
            .or_else(|| candidates.first());
        let side_effect_compatible = selected
            .map(|candidate| side_effect_matches(goal.side_effect, &candidate.side_effect_level))
            .unwrap_or(false);
        out.push(ResolvedStep {
            goal_id: goal.goal_id.clone(),
            action_name: selected.map(|candidate| candidate.action_name.clone()),
            respond_without_tool: selected.is_none(),
            fallback_reason: selected
                .is_none()
                .then(|| "no action descriptor resolved for semantic capability need".to_string()),
            side_effect_compatible,
            input_gaps: Vec::new(),
            candidates,
        });
    }
    out
}

pub(super) fn goal_can_respond_without_tool(goal: &SemanticGoal) -> bool {
    let no_side_effect = matches!(goal.side_effect, StepSideEffect::None);
    let chat_delivery = matches!(goal.delivery, GoalDelivery::Chat);
    let no_freshness = matches!(goal.freshness, GoalFreshness::None);
    let no_external_authorization = matches!(
        goal.authorization,
        GoalAuthorization::None | GoalAuthorization::LocalState
    );
    let no_object_reference = goal.object_ref.is_none();
    let no_mutation_hint = goal.hints.workspace_modification_request != Some(true)
        && goal.hints.public_freshness_required != Some(true);
    let noncritical_risk = !matches!(
        goal.risk.level.trim().to_ascii_lowercase().as_str(),
        "high" | "critical"
    );

    no_side_effect
        && chat_delivery
        && no_freshness
        && no_external_authorization
        && no_object_reference
        && no_mutation_hint
        && noncritical_risk
}

pub(super) fn plan_can_respond_without_tools(plan: &SemanticTurnPlan) -> bool {
    !plan.clarification_needed
        && !plan.goals.is_empty()
        && plan.goals.iter().all(goal_can_respond_without_tool)
}

fn direct_response_resolved_steps(plan: &SemanticTurnPlan) -> Vec<ResolvedStep> {
    plan.goals
        .iter()
        .map(|goal| ResolvedStep {
            goal_id: goal.goal_id.clone(),
            action_name: None,
            candidates: Vec::new(),
            respond_without_tool: true,
            fallback_reason: Some(
                "semantic goal can be satisfied with a direct conversational response".to_string(),
            ),
            side_effect_compatible: true,
            input_gaps: Vec::new(),
        })
        .collect()
}

async fn resolve_goal_candidates(
    agent: &Agent,
    goal: &SemanticGoal,
    graph: &CapabilityGraph,
    by_name: &HashMap<&str, &CapabilityGraphNode>,
    actions: &[crate::actions::ActionDef],
    capability_health: Option<&super::capability_health::CapabilityHealthSnapshot>,
    limit: usize,
) -> Vec<CapabilityCandidate> {
    let mut ordered: Vec<(String, Option<String>, Option<f64>)> = Vec::new();
    if let Some(embedder) = agent.embedding_client.as_deref() {
        let probe = goal_capability_probe(goal);
        if let Ok(mut embeddings) = embedder.embed_texts(&[probe]).await {
            if let Some(embedding) = embeddings.pop() {
                match agent
                    .storage
                    .nearest_action_catalog_index_entries(&embedding, graph.nodes.len() as u64)
                    .await
                {
                    Ok(nearest) => {
                        ordered.extend(nearest.into_iter().map(|(entry, distance)| {
                            (
                                entry.action_name,
                                Some(entry.descriptor_hash),
                                Some(distance),
                            )
                        }));
                    }
                    Err(error) => {
                        tracing::debug!("semantic capability resolver retrieval failed: {}", error);
                    }
                }
            }
        }
    }
    if ordered.is_empty() {
        let probe = goal_capability_probe(goal);
        ordered.extend(
            crate::core::action_catalog::rank_action_names_lexically(actions, &[probe])
                .into_iter()
                .filter_map(|action_name| {
                    by_name.get(action_name.as_str()).map(|node| {
                        (
                            action_name,
                            Some(node.descriptor_hash.clone()),
                            None,
                        )
                    })
                }),
        );
    }

    let mut seen = HashSet::new();
    let mut candidates = Vec::new();
    for (action_name, descriptor_hash, cosine_distance) in ordered {
        if candidates.len() >= limit || !seen.insert(action_name.clone()) {
            continue;
        }
        let Some(node) = by_name.get(action_name.as_str()).copied() else {
            continue;
        };
        candidates.push(CapabilityCandidate {
            action_name,
            descriptor_hash,
            cosine_distance,
            side_effect_level: node.metadata.side_effect_level.clone(),
            role: node.metadata.role.clone(),
            tool_role: node.metadata.tool_role,
            integration_class: node.metadata.integration_class.clone(),
            requires_auth: node.metadata.requires_auth,
            readiness: capability_health
                .and_then(|health| health.entry(&node.action_name))
                .map(|entry| entry.readiness.clone()),
            contract_fit: None,
        });
    }
    let action_by_name = actions
        .iter()
        .map(|action| (action.name.clone(), action.clone()))
        .collect::<HashMap<_, _>>();
    super::tool_contracts::sort_candidates_by_contract_fit(
        goal,
        &mut candidates,
        &action_by_name,
        capability_health,
    );
    candidates
}

fn capability_candidate_limit() -> usize {
    std::env::var("AGENTARK_SEMANTIC_CAPABILITY_CANDIDATES")
        .ok()
        .and_then(|value| value.trim().parse::<usize>().ok())
        .unwrap_or(12)
        .clamp(4, 64)
}

fn goal_capability_probe(goal: &SemanticGoal) -> String {
    let mut parts = Vec::new();
    parts.push(format!("outcome: {}", goal.outcome));
    parts.push(format!("capability_need: {}", goal.capability_need));
    parts.push(format!("side_effect: {:?}", goal.side_effect));
    parts.push(format!("delivery: {:?}", goal.delivery));
    parts.push(format!("freshness: {:?}", goal.freshness));
    parts.push(format!("authorization: {:?}", goal.authorization));
    if let Some(object_ref) = &goal.object_ref {
        parts.push(format!(
            "object_ref: {} {}",
            object_ref.kind, object_ref.resolution_family
        ));
    }
    if !goal.success_criteria.is_empty() {
        parts.push(format!(
            "success_criteria: {}",
            goal.success_criteria.join(" | ")
        ));
    }
    safe_truncate(&parts.join("\n"), 2_000)
}

pub(super) fn side_effect_matches(
    goal_side_effect: StepSideEffect,
    action_side_effect: &crate::actions::ActionSideEffectLevel,
) -> bool {
    match goal_side_effect {
        StepSideEffect::Read | StepSideEffect::None => {
            matches!(action_side_effect, crate::actions::ActionSideEffectLevel::None)
        }
        StepSideEffect::Notify => matches!(
            action_side_effect,
            crate::actions::ActionSideEffectLevel::Notify
                | crate::actions::ActionSideEffectLevel::Write
        ),
        StepSideEffect::CreateObject
        | StepSideEffect::ModifyObject
        | StepSideEffect::DeleteObject => {
            matches!(action_side_effect, crate::actions::ActionSideEffectLevel::Write)
        }
        StepSideEffect::Unknown => true,
    }
}

pub(super) fn verify_result(
    bundle: &SemanticTurnBundle,
    tool_history: &[serde_json::Value],
    final_text: &str,
    run_status: &str,
) -> ResultVerification {
    let successful_tools = tool_history
        .iter()
        .filter_map(|entry| {
            let tool = entry.get("tool").and_then(|value| value.as_str())?;
            let result = entry.get("result")?;
            let failed = result
                .get("status")
                .and_then(|value| value.as_str())
                .map(|status| matches!(status, "error" | "failed" | "approval_required"))
                .unwrap_or(false);
            (!failed).then(|| tool.to_string())
        })
        .collect::<HashSet<_>>();
    let final_text_present = !final_text.trim().is_empty();
    let mut completed_goal_ids = Vec::new();
    let mut pending_goal_ids = Vec::new();
    let mut evidence_notes = Vec::new();

    if bundle.plan.clarification_needed {
        return ResultVerification {
            all_goals_accounted: final_text_present,
            completed_goal_ids,
            pending_goal_ids: Vec::new(),
            unsupported_claims: Vec::new(),
            evidence_notes: vec!["turn ended with a clarification request".to_string()],
        };
    }

    for goal in &bundle.plan.goals {
        let resolved = bundle
            .resolved_steps
            .iter()
            .find(|step| step.goal_id == goal.goal_id);
        let has_tool_evidence = resolved
            .and_then(|step| step.action_name.as_ref())
            .map(|action| successful_tools.contains(action))
            .unwrap_or(false);
        let has_direct_evidence = resolved
            .map(|step| step.respond_without_tool && final_text_present)
            .unwrap_or(false);
        if has_tool_evidence || has_direct_evidence || (run_status == "needs_input" && final_text_present) {
            completed_goal_ids.push(goal.goal_id.clone());
            if has_tool_evidence {
                evidence_notes.push(format!("goal {} has tool-result evidence", goal.goal_id));
            } else {
                evidence_notes.push(format!("goal {} has user-facing response evidence", goal.goal_id));
            }
        } else {
            pending_goal_ids.push(goal.goal_id.clone());
        }
    }

    ResultVerification {
        all_goals_accounted: pending_goal_ids.is_empty(),
        completed_goal_ids,
        pending_goal_ids,
        unsupported_claims: Vec::new(),
        evidence_notes,
    }
}

fn extract_json_value(text: &str) -> Option<serde_json::Value> {
    serde_json::from_str::<serde_json::Value>(text)
        .ok()
        .or_else(|| {
            let start = text.find('{')?;
            let end = text.rfind('}')?;
            serde_json::from_str::<serde_json::Value>(&text[start..=end]).ok()
        })
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde::Deserialize;

    #[test]
    fn verifier_requires_requirement_coverage_without_prompt_phrase_checks() {
        let mut plan = SemanticTurnPlan::default();
        plan.explicit_requirements = vec![
            TurnRequirement {
                id: "req_0".to_string(),
                text: "first outcome".to_string(),
            },
            TurnRequirement {
                id: "req_1".to_string(),
                text: "second outcome".to_string(),
            },
        ];
        plan.goals = vec![SemanticGoal {
            goal_id: "goal_0".to_string(),
            outcome: "first outcome handled".to_string(),
            capability_need: "handle the first requested outcome".to_string(),
            covered_requirement_ids: vec!["req_0".to_string()],
            side_effect: StepSideEffect::Read,
            ..Default::default()
        }];

        let verification = verify_turn_plan(&plan);

        assert!(!verification.accepted);
        assert_eq!(verification.missing_requirement_ids, vec!["req_1"]);
    }

    #[test]
    fn side_effect_compatibility_is_structural() {
        assert!(side_effect_matches(
            StepSideEffect::Read,
            &crate::actions::ActionSideEffectLevel::None
        ));
        assert!(side_effect_matches(
            StepSideEffect::CreateObject,
            &crate::actions::ActionSideEffectLevel::Write
        ));
        assert!(!side_effect_matches(
            StepSideEffect::DeleteObject,
            &crate::actions::ActionSideEffectLevel::None
        ));
    }

    #[test]
    fn direct_conversation_goal_does_not_make_later_tool_goals_sticky() {
        let conversational = SemanticGoal {
            goal_id: "goal_0".to_string(),
            outcome: "Answer a general question.".to_string(),
            capability_need: "general conversation".to_string(),
            side_effect: StepSideEffect::None,
            freshness: GoalFreshness::None,
            delivery: GoalDelivery::Chat,
            authorization: GoalAuthorization::None,
            confidence: 1.0,
            ..Default::default()
        };
        let app_goal = SemanticGoal {
            goal_id: "goal_1".to_string(),
            outcome: "Build a browser app.".to_string(),
            capability_need: "create and deploy an application artifact".to_string(),
            side_effect: StepSideEffect::CreateObject,
            freshness: GoalFreshness::ArtifactRuntimeRefresh,
            delivery: GoalDelivery::App,
            authorization: GoalAuthorization::LocalState,
            confidence: 1.0,
            ..Default::default()
        };

        assert!(goal_can_respond_without_tool(&conversational));
        assert!(!goal_can_respond_without_tool(&app_goal));
    }

    #[derive(Debug, Deserialize)]
    struct BenchmarkCorpus {
        entries: Vec<BenchmarkEntry>,
    }

    #[derive(Debug, Deserialize)]
    struct BenchmarkEntry {
        id: String,
        expected: BenchmarkExpected,
    }

    #[derive(Debug, Deserialize)]
    struct BenchmarkExpected {
        goal_count: usize,
        goals: Vec<BenchmarkExpectedGoal>,
        #[serde(default)]
        clarification_needed: bool,
        #[serde(default)]
        requires_secret_sidecar: bool,
    }

    #[derive(Debug, Deserialize)]
    struct BenchmarkExpectedGoal {
        #[serde(default)]
        capability_target: Option<String>,
        #[serde(default)]
        capability_target_any_of: Vec<String>,
        #[serde(default)]
        side_effect: Option<String>,
        #[serde(default)]
        depends_on: Vec<String>,
    }

    #[test]
    fn semantic_benchmark_fixture_is_executable_against_turn_plan_contract() {
        let path = std::path::PathBuf::from(env!("CARGO_MANIFEST_DIR"))
            .join("tests")
            .join("fixtures")
            .join("semantic_benchmark.json");
        let raw = std::fs::read_to_string(&path)
            .unwrap_or_else(|error| panic!("read {}: {error}", path.display()));
        let corpus: BenchmarkCorpus = serde_json::from_str(&raw)
            .unwrap_or_else(|error| panic!("parse {}: {error}", path.display()));

        for entry in corpus.entries {
            let mut plan = SemanticTurnPlan {
                plan_id: format!("fixture-{}", entry.id),
                version: SEMANTIC_TURN_PLAN_VERSION.to_string(),
                turn_summary: format!("fixture {}", entry.id),
                clarification_needed: entry.expected.clarification_needed,
                clarification_question: entry
                    .expected
                    .clarification_needed
                    .then(|| "Need one missing detail.".to_string()),
                requires_secret_sidecar: entry.expected.requires_secret_sidecar,
                explicit_requirements: Vec::new(),
                goals: Vec::new(),
                confidence: 1.0,
            };
            if !entry.expected.clarification_needed {
                for index in 0..entry.expected.goal_count {
                    plan.explicit_requirements.push(TurnRequirement {
                        id: format!("req_{index}"),
                        text: format!("expected requirement {index}"),
                    });
                }
                for (index, goal) in entry.expected.goals.iter().enumerate() {
                    let capability = goal
                        .capability_target
                        .as_deref()
                        .or_else(|| goal.capability_target_any_of.first().map(String::as_str))
                        .unwrap_or("respond_without_tool");
                    plan.goals.push(SemanticGoal {
                        goal_id: format!("goal_{index}"),
                        outcome: format!("expected outcome {index}"),
                        capability_need: capability.to_string(),
                        side_effect: parse_fixture_side_effect(goal.side_effect.as_deref()),
                        dependencies: goal.depends_on.clone(),
                        covered_requirement_ids: vec![format!("req_{index}")],
                        confidence: 1.0,
                        ..Default::default()
                    });
                }
            }

            let verification = verify_turn_plan(&plan);
            assert!(
                verification.accepted,
                "fixture entry {} should satisfy the TurnPlan verifier: {:?}",
                entry.id, verification
            );
        }
    }

    fn parse_fixture_side_effect(value: Option<&str>) -> StepSideEffect {
        match value.unwrap_or("unknown") {
            "read" => StepSideEffect::Read,
            "notify" => StepSideEffect::Notify,
            "create_object" => StepSideEffect::CreateObject,
            "modify_object" => StepSideEffect::ModifyObject,
            "delete_object" => StepSideEffect::DeleteObject,
            "none" => StepSideEffect::None,
            _ => StepSideEffect::Unknown,
        }
    }
}
