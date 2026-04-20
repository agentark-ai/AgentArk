//! Swarm coordinator - manages specialist agents and task delegation

use super::agent_trait::*;
use super::bus::{MessageBus, SwarmEvent};
use super::messages::*;
use super::registry::AgentRegistry;
use super::specialist::{SpecialistAgent, SpecialistConfig};
use crate::actions::ActionDef;
use crate::core::llm::LlmClient;
use anyhow::{anyhow, Result};
use serde::{Deserialize, Serialize};
use std::collections::HashMap;
use std::sync::Arc;
use tokio::sync::RwLock;
use uuid::Uuid;

/// Swarm configuration
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct SwarmConfig {
    #[serde(default = "default_max_specialists")]
    pub max_specialists: usize,
    #[serde(default = "default_timeout")]
    pub default_timeout_secs: u64,
    #[serde(default)]
    pub specialists: Vec<SpecialistConfig>,
}

fn default_max_specialists() -> usize {
    5
}
fn default_timeout() -> u64 {
    60
}

impl Default for SwarmConfig {
    fn default() -> Self {
        Self {
            max_specialists: 5,
            default_timeout_secs: 60,
            specialists: vec![],
        }
    }
}

/// Result of a swarm delegation
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct SwarmDelegationResult {
    pub final_result: String,
    pub sub_results: Vec<DelegationResult>,
    pub total_time_ms: u64,
    pub agents_used: Vec<String>,
}

/// Status response for API
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct SwarmStatusResponse {
    pub enabled: bool,
    pub total_agents: usize,
    pub active_agents: usize,
    pub agents: Vec<AgentInfo>,
}

/// The swarm manager -- owns registry, bus, and specialist instances
#[derive(Clone)]
pub struct SwarmManager {
    pub registry: AgentRegistry,
    pub bus: MessageBus,
    pub(crate) specialists: Arc<RwLock<HashMap<AgentId, Arc<SpecialistAgent>>>>,
    pub config: SwarmConfig,
}

impl SwarmManager {
    pub async fn new(config: SwarmConfig) -> Result<Self> {
        let manager = Self {
            registry: AgentRegistry::new(),
            bus: MessageBus::new(),
            specialists: Arc::new(RwLock::new(HashMap::new())),
            config,
        };

        // Initialize configured specialists
        manager.initialize_specialists().await?;

        Ok(manager)
    }

    /// Initialize all configured specialist agents
    async fn initialize_specialists(&self) -> Result<()> {
        for spec_config in &self.config.specialists {
            if !spec_config.enabled {
                continue;
            }
            match self.add_specialist(spec_config.clone(), vec![]).await {
                Ok(_) => {
                    tracing::info!("Initialized swarm specialist: {}", spec_config.name);
                }
                Err(e) => {
                    tracing::warn!(
                        "Failed to initialize specialist '{}': {}",
                        spec_config.name,
                        e
                    );
                }
            }
        }
        Ok(())
    }

    /// Add a specialist agent to the swarm
    pub async fn add_specialist(
        &self,
        config: SpecialistConfig,
        actions: Vec<ActionDef>,
    ) -> Result<AgentId> {
        let specialist = SpecialistAgent::new(config, actions)?;
        let id = specialist.id().clone();
        let info = specialist.info();

        // Register in registry
        self.registry.register(info.clone()).await;

        // Create mailbox
        let _rx = self.bus.create_mailbox(id.clone()).await;

        // Store specialist
        self.specialists
            .write()
            .await
            .insert(id.clone(), Arc::new(specialist));

        // Broadcast event
        self.bus.broadcast(SwarmEvent::AgentRegistered(info));

        tracing::info!("Added swarm specialist: {}", id);
        Ok(id)
    }

    /// Remove a specialist agent from the swarm
    pub async fn remove_specialist(&self, id: &AgentId) -> Result<()> {
        self.specialists.write().await.remove(id);
        self.registry.unregister(id).await;
        self.bus.remove_mailbox(id).await;
        self.bus
            .broadcast(SwarmEvent::AgentUnregistered(id.clone()));
        Ok(())
    }

    /// Get swarm status for API
    pub async fn status(&self) -> SwarmStatusResponse {
        let agents = self.registry.list().await;
        let active = agents
            .iter()
            .filter(|a| a.status == AgentStatus::Busy)
            .count();
        SwarmStatusResponse {
            enabled: true, // always active — agents auto-spawn on demand
            total_agents: agents.len(),
            active_agents: active,
            agents,
        }
    }

    /// Delegate a complex task to the swarm
    ///
    /// This is the main entry point for swarm delegation:
    /// 1. Use coordinator LLM to decompose the task into sub-tasks
    /// 2. Find the best specialist for each sub-task
    /// 3. Execute sub-tasks (concurrently where possible)
    /// 4. Aggregate results using coordinator LLM
    pub async fn delegate(
        &self,
        task: &str,
        context: &str,
        coordinator_llm: &LlmClient,
        _memories: &[crate::core::PromptMemory],
        _actions: &[ActionDef],
        specialist_prompt_bundle: Option<&crate::core::self_evolve::SpecialistPromptBundleProfile>,
    ) -> Result<SwarmDelegationResult> {
        let start = std::time::Instant::now();
        let specialists = self.specialists.read().await.clone();

        if specialists.is_empty() {
            return Err(anyhow!("No specialist agents available in swarm"));
        }

        // Step 1: Use coordinator LLM to decompose the task
        let agent_descriptions: Vec<String> = specialists
            .values()
            .map(|s| {
                format!(
                    "- {} ({:?}): {}",
                    s.config().name,
                    s.config().agent_type,
                    s.config()
                        .capabilities
                        .iter()
                        .map(|c| c.description.clone())
                        .collect::<Vec<_>>()
                        .join(", ")
                )
            })
            .collect();

        let decompose_prompt = format!(
            "You are a task coordinator managing a team of specialist agents.\n\
             Available agents:\n{}\n\n\
             Decompose the following task into sub-tasks, one per agent. \
             Respond with a JSON array of objects, each with:\n\
             - \"agent_name\": the name of the agent to assign\n\
             - \"task\": the specific sub-task description\n\n\
             Only use agents that are relevant. If only one agent is needed, return a single-element array.\n\
             Respond ONLY with the JSON array, no other text.\n\n\
             Task: {}",
            agent_descriptions.join("\n"),
            task
        );

        let supervisor = crate::core::ExecutionSupervisor::default();
        let decomposition_request = crate::core::ExecutionRequest {
            kind: "swarm_decomposition".to_string(),
            channel: Some("swarm".to_string()),
            message_preview: Some(task.chars().take(200).collect()),
            ..Default::default()
        };
        let decomposition = crate::core::execution::execute_supervised_transport_chat(
            &supervisor,
            coordinator_llm,
            &decomposition_request,
            &decompose_prompt,
            task,
            &[],
            &[],
            Some(self.config.default_timeout_secs.saturating_mul(1000)),
        )
        .await?;

        // Parse sub-tasks from LLM response
        let sub_tasks = parse_sub_tasks(&decomposition.content, &specialists);

        if sub_tasks.is_empty() {
            // Fallback: send entire task to the most capable specialist
            let best = find_best_specialist(&specialists, task);
            if let Some((id, specialist)) = best {
                self.registry.update_status(&id, AgentStatus::Busy).await;
                self.bus.broadcast(SwarmEvent::TaskStarted {
                    task_id: Uuid::new_v4(),
                    agent_id: id.clone(),
                });

                let system_prompt_override = specialist_prompt_bundle.map(|bundle| {
                    crate::core::self_evolve::specialist_prompt_evolution::render_specialist_role_prompt(
                        bundle,
                        &specialist.config().agent_type,
                    )
                });
                let result = specialist
                    .execute_task_with_prompt(task, context, system_prompt_override)
                    .await;

                self.registry.update_status(&id, AgentStatus::Idle).await;

                match result {
                    Ok(content) => {
                        let delegation_result = DelegationResult {
                            task_id: Uuid::new_v4(),
                            agent_id: id.clone(),
                            agent_name: specialist.config().name.clone(),
                            success: true,
                            content: content.clone(),
                            confidence: 0.8,
                            execution_time_ms: start.elapsed().as_millis() as u64,
                            error: None,
                        };
                        return Ok(SwarmDelegationResult {
                            final_result: content,
                            sub_results: vec![delegation_result],
                            total_time_ms: start.elapsed().as_millis() as u64,
                            agents_used: vec![specialist.config().name.clone()],
                        });
                    }
                    Err(e) => return Err(e),
                }
            }
            return Err(anyhow!("No suitable specialist found for task"));
        }

        // Step 2: Execute sub-tasks concurrently
        let mut handles = vec![];

        for (agent_id, sub_task) in &sub_tasks {
            if let Some(specialist) = specialists.get(agent_id) {
                let specialist = specialist.clone();
                let sub_task = sub_task.clone();
                let context = context.to_string();
                let agent_id = agent_id.clone();
                let system_prompt_override = specialist_prompt_bundle.map(|bundle| {
                    crate::core::self_evolve::specialist_prompt_evolution::render_specialist_role_prompt(
                        bundle,
                        &specialist.config().agent_type,
                    )
                });
                let registry = &self.registry;
                let bus = &self.bus;
                let task_id = Uuid::new_v4();

                registry.update_status(&agent_id, AgentStatus::Busy).await;
                bus.broadcast(SwarmEvent::TaskStarted {
                    task_id,
                    agent_id: agent_id.clone(),
                });

                let handle = tokio::spawn(async move {
                    let task_start = std::time::Instant::now();
                    let result = tokio::time::timeout(
                        std::time::Duration::from_secs(60),
                        specialist.execute_task_with_prompt(
                            &sub_task,
                            &context,
                            system_prompt_override,
                        ),
                    )
                    .await;

                    let elapsed = task_start.elapsed().as_millis() as u64;

                    match result {
                        Ok(Ok(content)) => DelegationResult {
                            task_id,
                            agent_id: agent_id.clone(),
                            agent_name: specialist.config().name.clone(),
                            success: true,
                            content,
                            confidence: 0.8,
                            execution_time_ms: elapsed,
                            error: None,
                        },
                        Ok(Err(e)) => DelegationResult {
                            task_id,
                            agent_id: agent_id.clone(),
                            agent_name: specialist.config().name.clone(),
                            success: false,
                            content: String::new(),
                            confidence: 0.0,
                            execution_time_ms: elapsed,
                            error: Some(e.to_string()),
                        },
                        Err(_) => DelegationResult {
                            task_id,
                            agent_id: agent_id.clone(),
                            agent_name: specialist.config().name.clone(),
                            success: false,
                            content: String::new(),
                            confidence: 0.0,
                            execution_time_ms: elapsed,
                            error: Some("Timeout".to_string()),
                        },
                    }
                });
                handles.push(handle);
            }
        }

        // Collect results
        let mut sub_results = vec![];
        let mut agents_used = vec![];
        for handle in handles {
            if let Ok(result) = handle.await {
                self.registry
                    .update_status(&result.agent_id, AgentStatus::Idle)
                    .await;
                self.bus.broadcast(SwarmEvent::TaskCompleted {
                    task_id: result.task_id,
                    agent_id: result.agent_id.clone(),
                    success: result.success,
                });
                agents_used.push(result.agent_name.clone());
                sub_results.push(result);
            }
        }

        // Step 3: Aggregate results using coordinator LLM
        let successful_results: Vec<&DelegationResult> =
            sub_results.iter().filter(|r| r.success).collect();

        let final_result = if successful_results.len() == 1 {
            successful_results[0].content.clone()
        } else if successful_results.is_empty() {
            let errors: Vec<String> = sub_results
                .iter()
                .filter_map(|r| r.error.as_ref().map(|e| format!("{}: {}", r.agent_name, e)))
                .collect();
            return Err(anyhow!("All sub-tasks failed: {}", errors.join("; ")));
        } else {
            // Aggregate multiple results
            let results_text: String = successful_results
                .iter()
                .map(|r| format!("[{}]: {}", r.agent_name, r.content))
                .collect::<Vec<_>>()
                .join("\n\n");

            let aggregate_prompt = format!(
                "You are aggregating results from multiple specialist agents.\n\
                 Original task: {}\n\n\
                 Agent results:\n{}\n\n\
                 Synthesize these into a single coherent response. \
                 Be comprehensive but avoid repetition.",
                task, results_text
            );

            let aggregate_request = crate::core::ExecutionRequest {
                kind: "swarm_aggregation".to_string(),
                channel: Some("swarm".to_string()),
                message_preview: Some(task.chars().take(200).collect()),
                ..Default::default()
            };
            let aggregated = crate::core::execution::execute_supervised_transport_chat(
                &supervisor,
                coordinator_llm,
                &aggregate_request,
                &aggregate_prompt,
                "Synthesize the results",
                &[],
                &[],
                Some(self.config.default_timeout_secs.saturating_mul(1000)),
            )
            .await?;
            aggregated.content
        };

        Ok(SwarmDelegationResult {
            final_result,
            sub_results,
            total_time_ms: start.elapsed().as_millis() as u64,
            agents_used,
        })
    }
}

/// Parse sub-tasks from LLM decomposition response
fn parse_sub_tasks(
    response: &str,
    specialists: &HashMap<AgentId, Arc<SpecialistAgent>>,
) -> Vec<(AgentId, String)> {
    // Try to parse JSON array from response
    let json_str = extract_json_array(response);
    let parsed: Result<Vec<serde_json::Value>, _> = serde_json::from_str(&json_str);

    match parsed {
        Ok(tasks) => {
            let mut result = vec![];
            for task_obj in tasks {
                let agent_name = task_obj
                    .get("agent_name")
                    .and_then(|v| v.as_str())
                    .unwrap_or("");
                let task_desc = task_obj.get("task").and_then(|v| v.as_str()).unwrap_or("");

                if agent_name.is_empty() || task_desc.is_empty() {
                    continue;
                }

                // Find specialist by name
                let name_lower = agent_name.to_lowercase();
                if let Some((id, _)) = specialists
                    .iter()
                    .find(|(_, s)| s.config().name.to_lowercase() == name_lower)
                {
                    result.push((id.clone(), task_desc.to_string()));
                }
            }
            result
        }
        Err(_) => vec![],
    }
}

/// Extract a JSON array from LLM response (may have surrounding text)
fn extract_json_array(text: &str) -> String {
    // Find the first [ and last ]
    if let Some(start) = text.find('[') {
        if let Some(end) = text.rfind(']') {
            if end > start {
                return text[start..=end].to_string();
            }
        }
    }
    "[]".to_string()
}

/// Find the best specialist for a given task
fn find_best_specialist<'a>(
    specialists: &'a HashMap<AgentId, Arc<SpecialistAgent>>,
    task: &str,
) -> Option<(AgentId, &'a Arc<SpecialistAgent>)> {
    specialists
        .iter()
        .max_by(|(_, a), (_, b)| {
            a.can_handle(task)
                .partial_cmp(&b.can_handle(task))
                .unwrap_or(std::cmp::Ordering::Equal)
        })
        .map(|(id, s)| (id.clone(), s))
}
