//! Lightweight specialist agent

use super::AgentAccessScope;
use super::agent_trait::*;
use crate::actions::ActionDef;
use crate::core::PromptMemory;
use crate::core::llm::{LlmClient, LlmProvider};
use crate::core::orchestra::SubAgentType;
use crate::core::prompt_policy::delegated_policy_v2_block;
use anyhow::Result;
use async_trait::async_trait;
use std::collections::HashSet;

/// Configuration for a specialist agent
#[derive(Debug, Clone, serde::Serialize, serde::Deserialize)]
pub struct SpecialistConfig {
    #[serde(default)]
    pub id: Option<String>,
    pub name: String,
    pub agent_type: SubAgentType,
    pub llm_provider: LlmProvider,
    #[serde(default)]
    pub system_prompt_override: Option<String>,
    #[serde(default = "default_max_memory")]
    pub max_memory_retrieval: usize,
    #[serde(default)]
    pub capabilities: Vec<AgentCapability>,
    #[serde(default)]
    pub access_scope: AgentAccessScope,
    #[serde(default = "default_enabled")]
    pub enabled: bool,
}

fn default_max_memory() -> usize {
    3
}
fn default_enabled() -> bool {
    true
}

/// A lightweight specialist agent that wraps an LLM client
pub struct SpecialistAgent {
    id: AgentId,
    config: SpecialistConfig,
    llm: LlmClient,
    available_actions: Vec<ActionDef>,
}

impl SpecialistAgent {
    pub fn new(config: SpecialistConfig, available_actions: Vec<ActionDef>) -> Result<Self> {
        let llm = LlmClient::new(&config.llm_provider)?;
        let id = config
            .id
            .clone()
            .filter(|value| !value.trim().is_empty())
            .map(AgentId)
            .unwrap_or_default();
        Ok(Self {
            id,
            config,
            llm,
            available_actions,
        })
    }

    pub fn config(&self) -> &SpecialistConfig {
        &self.config
    }

    /// Build the system prompt for this specialist
    fn system_prompt(&self) -> String {
        if let Some(ref override_prompt) = self.config.system_prompt_override {
            return override_prompt.clone();
        }
        self.config.agent_type.system_prompt()
    }

    /// Execute a task using this specialist's LLM with an optional
    /// caller-supplied per-invocation system prompt.
    pub async fn execute_task_with_prompt(
        &self,
        task: &str,
        context: &str,
        system_prompt_override: Option<String>,
    ) -> Result<String> {
        self.execute_task_with_scope_and_prompt(
            task,
            context,
            &[],
            &self.available_actions,
            system_prompt_override,
            None,
        )
        .await
    }

    /// Execute a task with task-scoped memories/actions and an optional
    /// caller-supplied system prompt override for this invocation.
    pub async fn execute_task_with_scope_and_prompt(
        &self,
        task: &str,
        context: &str,
        memories: &[PromptMemory],
        available_actions: &[ActionDef],
        system_prompt_override: Option<String>,
        timeout_ms: Option<u64>,
    ) -> Result<String> {
        let system_prompt = format!(
            "{}\n\nYou are part of an agent swarm. Your name is '{}'. \
             Respond with your analysis/result for the delegated task. \
             Stay inside the delegated task packet and use dependency outputs instead of redoing completed work.\n\
             {}\n\n\
             Delegated task packet:\n{}",
            system_prompt_override.unwrap_or_else(|| self.system_prompt()),
            self.config.name,
            delegated_policy_v2_block(),
            context
        );

        let supervisor = crate::core::ExecutionSupervisor::default();
        let request = crate::core::ExecutionRequest {
            kind: "swarm_specialist_task".to_string(),
            channel: Some("swarm".to_string()),
            message_preview: Some(task.chars().take(200).collect()),
            ..Default::default()
        };
        let response = crate::core::execution::execute_supervised_transport_chat(
            &supervisor,
            &self.llm,
            &request,
            &system_prompt,
            task,
            memories,
            available_actions,
            timeout_ms.filter(|value| *value > 0),
        )
        .await?;

        Ok(response.content)
    }

    /// Get model name for display
    pub fn model_name(&self) -> String {
        match &self.config.llm_provider {
            LlmProvider::Anthropic { model, .. } => model.clone(),
            LlmProvider::OpenAI { model, .. } => model.clone(),
            LlmProvider::Ollama { model, .. } => model.clone(),
        }
    }

    fn relevance_tokens(text: &str) -> HashSet<String> {
        text.to_ascii_lowercase()
            .split(|ch: char| !ch.is_ascii_alphanumeric())
            .filter_map(|token| {
                let trimmed = token.trim();
                if trimmed.len() < 3 || trimmed.chars().all(|ch| ch.is_ascii_digit()) {
                    None
                } else {
                    Some(trimmed.to_string())
                }
            })
            .collect()
    }

    fn relevance_score(task_tokens: &HashSet<String>, candidate_tokens: &HashSet<String>) -> f32 {
        if task_tokens.is_empty() || candidate_tokens.is_empty() {
            return 0.0;
        }
        let overlap = task_tokens.intersection(candidate_tokens).count() as f32;
        let coverage = overlap / task_tokens.len() as f32;
        let precision = overlap / candidate_tokens.len() as f32;
        (0.8 * coverage + 0.2 * (precision * 6.0).min(1.0)).clamp(0.0, 1.0)
    }

    fn profile_tokens(&self) -> HashSet<String> {
        let mut tokens = Self::relevance_tokens(&self.config.name);
        tokens.extend(Self::relevance_tokens(&self.config.agent_type.name()));
        tokens.extend(Self::relevance_tokens(&self.system_prompt()));
        for capability in &self.config.capabilities {
            tokens.extend(Self::relevance_tokens(&capability.name));
            tokens.extend(Self::relevance_tokens(&capability.description));
            for keyword in &capability.keywords {
                tokens.extend(Self::relevance_tokens(keyword));
            }
        }
        tokens
    }
}

#[async_trait]
impl SwarmAgent for SpecialistAgent {
    fn info(&self) -> AgentInfo {
        AgentInfo {
            id: self.id.clone(),
            name: self.config.name.clone(),
            agent_type: format!("{:?}", self.config.agent_type),
            capabilities: self.config.capabilities.clone(),
            status: AgentStatus::Idle,
            llm_model: self.model_name(),
        }
    }

    fn id(&self) -> &AgentId {
        &self.id
    }

    fn can_handle(&self, task_description: &str) -> f32 {
        let task_tokens = Self::relevance_tokens(task_description);
        if task_tokens.is_empty() {
            return 0.0;
        }

        let profile_score = Self::relevance_score(&task_tokens, &self.profile_tokens());
        let capability_score = self
            .config
            .capabilities
            .iter()
            .map(|capability| {
                let mut tokens = Self::relevance_tokens(&capability.name);
                tokens.extend(Self::relevance_tokens(&capability.description));
                for keyword in &capability.keywords {
                    tokens.extend(Self::relevance_tokens(keyword));
                }
                Self::relevance_score(&task_tokens, &tokens)
            })
            .fold(0.0f32, f32::max);

        (0.35 * profile_score + 0.65 * capability_score.max(profile_score)).clamp(0.0, 1.0)
    }
}
