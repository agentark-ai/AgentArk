//! Task-driven auto-spawn agent system
//!
//! Replaces the old pre-configured swarm model with intelligent, on-demand
//! agent spawning. The LLM decides IF sub-agents are needed, WHAT kind,
//! and they are auto-spawned from the model pool. User-configured specialists
//! act as priority boosters — preferred when they match, but never required.

use anyhow::{anyhow, Result};
use serde::{Deserialize, Serialize};
use std::collections::HashMap;
use std::sync::Arc;
use tokio::sync::RwLock;

use super::agent::QueryComplexity;
use super::config::{ModelRole, ModelSlot};
use super::intent::{action_intent_score, preferred_direct_action_name};
use super::llm::LlmClient;
use super::orchestra::SubAgentType;
use super::prompt_policy::{delegated_policy_v2_block, synthesis_policy_v2_block};
use super::swarm::agent_trait::SwarmAgent;
use super::swarm::specialist::SpecialistAgent;
use crate::actions::ActionDef;
use crate::memory::MemoryEntry;

fn compact_text(text: &str, max_chars: usize) -> String {
    if text.chars().count() <= max_chars {
        text.to_string()
    } else {
        format!("{}...", text.chars().take(max_chars).collect::<String>())
    }
}

const AUTO_AGENT_SCIENTIST_NAMES: &[&str] = &[
    "Curie",
    "Turing",
    "Hopper",
    "Einstein",
    "Tesla",
    "Faraday",
    "Noether",
    "Sagan",
    "Kepler",
    "Galileo",
    "Darwin",
    "Feynman",
    "Hubble",
    "Maxwell",
    "Lovelace",
    "Bohr",
    "Franklin",
    "Planck",
    "Copernicus",
    "Mendel",
    "Raman",
    "Hawking",
    "Meitner",
    "Pasteur",
    "Newton",
    "Shannon",
    "Babbage",
    "Euler",
    "Leavitt",
    "Goodall",
    "Carson",
    "Chandrasekhar",
    "Wu",
    "Boyle",
    "Archimedes",
    "Kapitsa",
];

fn scientist_name_for_index(index: usize) -> String {
    AUTO_AGENT_SCIENTIST_NAMES[index % AUTO_AGENT_SCIENTIST_NAMES.len()].to_string()
}

fn fallback_scientist_name(agent_type: &SubAgentType) -> &'static str {
    match agent_type {
        SubAgentType::Researcher => "Curie",
        SubAgentType::Coder => "Turing",
        SubAgentType::Analyst => "Noether",
        SubAgentType::Writer => "Sagan",
        SubAgentType::Validator => "Franklin",
        SubAgentType::Planner => "Kepler",
        SubAgentType::Custom { .. } => "Faraday",
    }
}

fn display_name_for_specialist(name: &str, agent_type: &SubAgentType) -> String {
    let trimmed = name.trim();
    if trimmed.is_empty() || trimmed.eq_ignore_ascii_case(&agent_type.name()) {
        fallback_scientist_name(agent_type).to_string()
    } else {
        trimmed.to_string()
    }
}

/// LLM-determined routing decision
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct RoutingDecision {
    /// Whether the task needs sub-agents
    pub needs_delegation: bool,
    /// Complexity tier (used for model selection + parallel thinking)
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
}

impl SubAgentSpec {
    /// Parse agent_type string into SubAgentType
    pub fn resolve_agent_type(&self) -> SubAgentType {
        match self.agent_type.to_lowercase().as_str() {
            "researcher" => SubAgentType::Researcher,
            "coder" => SubAgentType::Coder,
            "analyst" => SubAgentType::Analyst,
            "writer" => SubAgentType::Writer,
            "validator" => SubAgentType::Validator,
            "planner" => SubAgentType::Planner,
            _ => SubAgentType::Planner, // safe default
        }
    }

    /// Parse preferred_model_role string into ModelRole
    pub fn resolve_model_role(&self) -> Option<ModelRole> {
        self.preferred_model_role
            .as_ref()
            .map(|r| match r.to_lowercase().as_str() {
                "code" => ModelRole::Code,
                "research" => ModelRole::Research,
                "fast" => ModelRole::Fast,
                _ => ModelRole::Primary,
            })
    }
}

/// Result of task routing
pub enum TaskRouterResult {
    /// Simple query — caller should do a direct LLM call
    Direct,
    /// Medium query without delegation — use parallel thinking
    UseParallelThinking,
    /// Delegated to auto-spawned agents — here are the results
    Delegated(DelegatedResult),
}

/// Result from delegated multi-agent execution
#[derive(Debug, Clone)]
pub struct DelegatedResult {
    /// Final synthesized response (includes tool calls when returned by the LLM)
    pub final_response: super::llm::LlmResponse,
    /// Per-agent results for trace visibility
    pub _agent_results: Vec<AgentExecResult>,
    /// Total wall-clock time in milliseconds
    pub _total_time_ms: u64,
}

/// Result from a single agent execution (for trace)
#[derive(Debug, Clone)]
pub struct AgentExecResult {
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
}

/// Configuration for the task router
pub struct TaskRouterConfig {
    /// Max concurrent agents
    pub _max_concurrent: usize,
    /// Timeout per agent in seconds
    pub agent_timeout_secs: u64,
    /// Minimum confidence for a specialist to be used over ephemeral
    pub specialist_threshold: f32,
}

impl Default for TaskRouterConfig {
    fn default() -> Self {
        Self {
            _max_concurrent: 5,
            agent_timeout_secs: 60,
            specialist_threshold: 0.3,
        }
    }
}

/// The unified task router — auto-spawns agents based on LLM routing decisions
pub struct TaskRouter {
    config: TaskRouterConfig,
}

type SpecialistRegistry = Arc<RwLock<HashMap<super::swarm::AgentId, Arc<SpecialistAgent>>>>;

pub struct TaskRouterExecuteContext<'a> {
    pub message: &'a str,
    pub system_prompt: &'a str,
    pub model_pool: &'a HashMap<String, (ModelSlot, LlmClient)>,
    pub primary_llm: &'a LlmClient,
    pub specialists: &'a Option<SpecialistRegistry>,
    pub memories: &'a [MemoryEntry],
    pub actions: &'a [ActionDef],
    pub trace: &'a Arc<RwLock<super::agent::ExecutionTrace>>,
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
        let message = ctx.message;
        let system_prompt = ctx.system_prompt;
        let model_pool = ctx.model_pool;
        let primary_llm = ctx.primary_llm;
        let specialists = ctx.specialists;
        let memories = ctx.memories;
        let actions = ctx.actions;
        let trace = ctx.trace;
        // Simple queries — no delegation
        if !decision.needs_delegation {
            return match decision.complexity {
                QueryComplexity::Simple => Ok(TaskRouterResult::Direct),
                QueryComplexity::Medium => Ok(TaskRouterResult::UseParallelThinking),
                QueryComplexity::Complex => Ok(TaskRouterResult::Direct), // complex but LLM said no delegation
            };
        }

        if decision.sub_agents.is_empty() {
            return Ok(TaskRouterResult::Direct);
        }

        let start = std::time::Instant::now();

        // Build assignments: for each spec, find a specialist or pick model from pool
        let mut assignments: Vec<AgentAssignment> = Vec::new();

        for spec in &decision.sub_agents {
            let agent_type = spec.resolve_agent_type();

            // Try to find a matching user-configured specialist
            let specialist_match = if let Some(ref specs) = specialists {
                self.find_matching_specialist(specs, &spec.task, &agent_type)
                    .await
            } else {
                None
            };

            if let Some((name, specialist)) = specialist_match {
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
                assignments.push(AgentAssignment {
                    spec: spec.clone(),
                    agent_type: agent_type.clone(),
                    display_name: display_name_for_specialist(&name, &agent_type),
                    kind: AssignmentKind::Specialist(specialist),
                });
            } else {
                // Auto-spawn: select LLM from model pool
                let llm = self.select_llm_for_spec(spec, &agent_type, model_pool, primary_llm);
                let model_name = llm.model_name().to_string();
                let auto_agent_name = scientist_name_for_index(assignments.len());
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
                assignments.push(AgentAssignment {
                    spec: spec.clone(),
                    agent_type: agent_type.clone(),
                    display_name: auto_agent_name,
                    kind: AssignmentKind::Ephemeral(llm),
                });
            }
        }

        // Execute assignments respecting dependencies
        let results = self
            .execute_assignments(&assignments, system_prompt, memories, actions, trace)
            .await?;

        // Aggregate
        let mut final_response = if results.len() == 1 {
            if let Some(resp) = results[0].llm_response.clone() {
                resp
            } else {
                // Specialist-only single result: run a final synthesis pass so tool calls
                // can still be emitted by the primary model.
                self.aggregate(
                    primary_llm,
                    message,
                    system_prompt,
                    &results,
                    memories,
                    actions,
                )
                .await?
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
                &results,
                memories,
                actions,
            )
            .await?
        };

        // Safety net: delegated synthesis can occasionally omit tool calls even when
        // sub-agents produced them. Recover the clearest direct action when available.
        if let Some(preferred_action) = preferred_direct_action_name(message, actions) {
            if final_response.tool_calls.is_empty() {
                if let Some(recovered_call) = results
                    .iter()
                    .filter_map(|r| r.llm_response.as_ref())
                    .flat_map(|resp| resp.tool_calls.iter())
                    .find(|tc| tc.name == preferred_action)
                    .cloned()
                {
                    final_response.tool_calls.push(recovered_call);
                }
            }
        }

        let total_time_ms = start.elapsed().as_millis() as u64;

        // Trace: complete
        {
            let mut t = trace.write().await;
            t.steps.push(super::agent::ExecutionStep {
                icon: "\u{2705}".to_string(), // checkmark
                title: "Agent Delegation Complete".to_string(),
                detail: format!("{} agents | {}ms", results.len(), total_time_ms),
                step_type: "success".to_string(),
                data: Some(
                    results
                        .iter()
                        .map(|r| {
                            let tag = if r.is_specialist {
                                "specialist"
                            } else {
                                "auto"
                            };
                            format!("{} [{}] ({}ms)", r.agent_type, tag, r.execution_time_ms)
                        })
                        .collect::<Vec<_>>()
                        .join(", "),
                ),
                timestamp: chrono::Utc::now(),
                duration_ms: Some(total_time_ms),
            });
        }

        Ok(TaskRouterResult::Delegated(DelegatedResult {
            final_response,
            _agent_results: results,
            _total_time_ms: total_time_ms,
        }))
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
        model_pool: &HashMap<String, (ModelSlot, LlmClient)>,
        primary_llm: &LlmClient,
    ) -> LlmClient {
        // 1. Try explicit preferred role from routing decision
        if let Some(role) = spec.resolve_model_role() {
            for (slot, client) in model_pool.values() {
                if slot.role == role && slot.enabled {
                    return client.clone();
                }
            }
        }

        // 2. Auto-detect based on agent type
        let auto_role = match agent_type {
            SubAgentType::Coder => ModelRole::Code,
            SubAgentType::Researcher => ModelRole::Research,
            _ => ModelRole::Primary,
        };

        for (slot, client) in model_pool.values() {
            if slot.role == auto_role && slot.enabled {
                return client.clone();
            }
        }

        // 3. Fallback to primary
        primary_llm.clone()
    }

    /// Keep sub-agent tool context small by passing only task-relevant actions.
    fn select_actions_for_task(&self, task: &str, actions: &[ActionDef]) -> Vec<ActionDef> {
        let task_lower = task.to_ascii_lowercase();
        let mut scored: Vec<(f32, ActionDef)> = actions
            .iter()
            .map(|action| {
                let mut score = action_intent_score(task, action);
                if task_lower.contains(&action.name.to_ascii_lowercase()) {
                    score = score.max(0.95);
                }
                (score, action.clone())
            })
            .collect();

        scored.sort_by(|a, b| b.0.partial_cmp(&a.0).unwrap_or(std::cmp::Ordering::Equal));

        let mut selected: Vec<ActionDef> = scored
            .iter()
            .filter(|(score, _)| *score >= 0.10)
            .take(8)
            .map(|(_, action)| action.clone())
            .collect();

        if selected.is_empty() {
            selected = scored
                .into_iter()
                .take(8)
                .map(|(_, action)| action)
                .collect();
        }
        selected
    }

    /// Execute all assignments, respecting dependency ordering
    async fn execute_assignments(
        &self,
        assignments: &[AgentAssignment],
        system_prompt: &str,
        memories: &[MemoryEntry],
        actions: &[ActionDef],
        trace: &Arc<RwLock<super::agent::ExecutionTrace>>,
    ) -> Result<Vec<AgentExecResult>> {
        let n = assignments.len();
        let mut results: Vec<Option<AgentExecResult>> = vec![None; n];
        let mut completed: Vec<bool> = vec![false; n];

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

            // Build context from completed dependencies
            let dep_contexts: Vec<(usize, String)> = ready
                .iter()
                .map(|&idx| {
                    let ctx: String = assignments[idx]
                        .spec
                        .depends_on
                        .iter()
                        .filter_map(|&dep| {
                            results[dep]
                                .as_ref()
                                .map(|r| compact_text(&r.content, 1200))
                        })
                        .collect::<Vec<_>>()
                        .join("\n\n");
                    (idx, ctx)
                })
                .collect();

            // Execute ready assignments in parallel
            let mut handles = Vec::new();
            for (idx, context) in dep_contexts {
                let assignment = &assignments[idx];
                let task = assignment.spec.task.clone();
                let agent_type = assignment.agent_type.clone();
                let display_name = assignment.display_name.clone();
                // Keep delegated context compact to control token costs.
                let sys_prompt = compact_text(system_prompt, 2200);
                let ctx = context;
                let mems: Vec<MemoryEntry> = memories.to_vec();
                let acts: Vec<ActionDef> = self.select_actions_for_task(&task, actions);
                let timeout = self.config.agent_timeout_secs;

                match &assignment.kind {
                    AssignmentKind::Specialist(specialist) => {
                        let specialist = specialist.clone();
                        handles.push((
                            idx,
                            true,
                            tokio::spawn(async move {
                                let start = std::time::Instant::now();
                                let result = tokio::time::timeout(
                                    std::time::Duration::from_secs(timeout),
                                    specialist.execute_task(&task, &ctx),
                                )
                                .await;
                                let elapsed = start.elapsed().as_millis() as u64;
                                let model = specialist.model_name();
                                match result {
                                    Ok(Ok(content)) => Ok(AgentExecResult {
                                        agent_type: agent_type.name(),
                                        task,
                                        is_specialist: true,
                                        agent_name: Some(display_name),
                                        model_name: model,
                                        content,
                                        llm_response: None,
                                        execution_time_ms: elapsed,
                                    }),
                                    Ok(Err(e)) => Err(anyhow!("Specialist error: {}", e)),
                                    Err(_) => {
                                        Err(anyhow!("Specialist timed out after {}s", timeout))
                                    }
                                }
                            }),
                        ));
                    }
                    AssignmentKind::Ephemeral(llm) => {
                        let llm = llm.clone();
                        let model_name = llm.model_name().to_string();
                        handles.push((idx, false, tokio::spawn(async move {
                                let start = std::time::Instant::now();
                                let prompt = format!(
                                    "{}\n\n## Inherited Policy\n{}\n\n## Coordinator Context\n{}\n\n## Context from Previous Steps\n{}",
                                    agent_type.system_prompt(),
                                    delegated_policy_v2_block(),
                                    sys_prompt,
                                    if ctx.is_empty() {
                                        "No previous context.".to_string()
                                    } else {
                                        ctx
                                    }
                                );
                                let result = tokio::time::timeout(
                                    std::time::Duration::from_secs(timeout),
                                    llm.chat(&prompt, &task, &mems, &acts),
                                )
                                .await;
                                let elapsed = start.elapsed().as_millis() as u64;
                                match result {
                                    Ok(Ok(resp)) => Ok(AgentExecResult {
                                        agent_type: agent_type.name(),
                                        task,
                                        is_specialist: false,
                                        agent_name: Some(display_name),
                                        model_name,
                                        content: resp.content.clone(),
                                        llm_response: Some(resp),
                                        execution_time_ms: elapsed,
                                    }),
                                    Ok(Err(e)) => Err(anyhow!("Agent error: {}", e)),
                                    Err(_) => Err(anyhow!("Agent timed out after {}s", timeout)),
                                }
                            })));
                    }
                }
            }

            // Collect results
            for (idx, is_specialist, handle) in handles {
                match handle.await {
                    Ok(Ok(result)) => {
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
                                title: format!("{} completed", tag),
                                detail: format!(
                                    "Model: {} | {}ms | {} chars",
                                    result.model_name,
                                    result.execution_time_ms,
                                    result.content.len()
                                ),
                                step_type: "success".to_string(),
                                data: None,
                                timestamp: chrono::Utc::now(),
                                duration_ms: Some(result.execution_time_ms),
                            });
                        }
                        results[idx] = Some(result);
                        completed[idx] = true;
                    }
                    Ok(Err(e)) => {
                        tracing::warn!("Agent {} failed: {}", idx, e);
                        // Create a failure result so we can continue
                        results[idx] = Some(AgentExecResult {
                            agent_type: assignments[idx].agent_type.name(),
                            task: assignments[idx].spec.task.clone(),
                            is_specialist,
                            agent_name: Some(assignments[idx].display_name.clone()),
                            model_name: "failed".to_string(),
                            content: format!("Agent failed: {}", e),
                            llm_response: None,
                            execution_time_ms: 0,
                        });
                        completed[idx] = true;
                    }
                    Err(e) => {
                        tracing::error!("Agent {} panicked: {}", idx, e);
                        results[idx] = Some(AgentExecResult {
                            agent_type: assignments[idx].agent_type.name(),
                            task: assignments[idx].spec.task.clone(),
                            is_specialist,
                            agent_name: Some(assignments[idx].display_name.clone()),
                            model_name: "panicked".to_string(),
                            content: format!("Agent panicked: {}", e),
                            llm_response: None,
                            execution_time_ms: 0,
                        });
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
        results: &[AgentExecResult],
        memories: &[MemoryEntry],
        actions: &[ActionDef],
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
                format!(
                    "## {} - {}\n{}",
                    tag,
                    compact_text(&r.task, 240),
                    compact_text(&r.content, 1600)
                )
            })
            .collect::<Vec<_>>()
            .join("\n\n---\n\n");
        results_text = compact_text(&results_text, 9000);

        let prompt = format!(
            "Synthesize specialist outputs into one final user answer.\n\n\
            Original task:\n{}\n\n\
            Specialist outputs:\n{}\n\n\
            Requirements:\n\
            - Do not mention agents or synthesis.\n\
            - If the task maps cleanly to an available action, emit that tool call with complete arguments.\n\
            - If the task targets the current workspace or framework itself, prefer local code, file, or shell actions over deploying a separate artifact.\n\
            - Any retry/repair plan must explicitly state a maximum attempts cap.\n\
            - Include a compact evidence summary for actions used.\n\
            - Keep the response concise and practical.",
            compact_text(original_task, 1200),
            results_text
        );

        let mut wanted_tools: std::collections::HashSet<String> = std::collections::HashSet::new();
        for result in results {
            if let Some(resp) = &result.llm_response {
                for tc in &resp.tool_calls {
                    wanted_tools.insert(tc.name.clone());
                }
            }
        }
        let mut scored_actions: Vec<(f32, String)> = actions
            .iter()
            .map(|a| (action_intent_score(original_task, a), a.name.clone()))
            .collect();
        scored_actions.sort_by(|a, b| b.0.partial_cmp(&a.0).unwrap_or(std::cmp::Ordering::Equal));
        for (idx, (score, name)) in scored_actions.into_iter().enumerate() {
            if score >= 0.10 || idx < 6 {
                wanted_tools.insert(name);
            }
        }
        let filtered_actions: Vec<ActionDef> = actions
            .iter()
            .filter(|a| wanted_tools.contains(&a.name))
            .cloned()
            .collect();

        let synth_system_prompt = format!(
            "You are AgentArk. Return only the final user-facing answer. \
Use tool calls when required by the task and prefer the clearest semantic action match from the available actions. \
For requests about the current workspace/framework itself, prefer local code, file, and shell actions over deployment actions. \
Any retry/repair loop must declare an explicit max attempts cap and stop when reached. \
Be concise and action-oriented.\n\n{}",
            synthesis_policy_v2_block()
        );

        llm.chat(&synth_system_prompt, &prompt, memories, &filtered_actions)
            .await
    }
}

// -- Internal types --

struct AgentAssignment {
    spec: SubAgentSpec,
    agent_type: SubAgentType,
    display_name: String,
    kind: AssignmentKind,
}

enum AssignmentKind {
    Specialist(Arc<SpecialistAgent>),
    Ephemeral(LlmClient),
}
