//! Dynamic Sub-Agent Orchestration Framework (AOrchestra inspired)
//!
//! Enables the main agent to dynamically create specialized sub-agents
//! for complex task decomposition and parallel execution.

use serde::{Deserialize, Serialize};

/// Configuration for the orchestration framework
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct OrchestraConfig {
    /// Maximum number of sub-agents that can run concurrently
    pub max_concurrent_agents: usize,
    /// Maximum depth of sub-agent delegation
    pub max_delegation_depth: u32,
    /// Timeout for sub-agent execution in seconds
    pub agent_timeout_secs: u64,
    /// Whether sub-agents can create their own sub-agents
    pub allow_nested_delegation: bool,
    /// Default capabilities for all sub-agents
    pub default_capabilities: Vec<String>,
}

impl Default for OrchestraConfig {
    fn default() -> Self {
        Self {
            max_concurrent_agents: 5,
            max_delegation_depth: 3,
            agent_timeout_secs: 60,
            allow_nested_delegation: true,
            default_capabilities: vec!["reasoning".to_string(), "analysis".to_string()],
        }
    }
}

/// A specialized sub-agent type
#[derive(Debug, Clone, Serialize, Deserialize)]
pub enum SubAgentType {
    /// Research and information gathering
    Researcher,
    /// Code analysis and generation
    Coder,
    /// Data analysis and processing
    Analyst,
    /// Content writing and editing
    Writer,
    /// Fact-checking and validation
    Validator,
    /// Planning and decomposition
    Planner,
    /// Custom agent with specific instructions
    Custom { name: String, instructions: String },
}

impl SubAgentType {
    /// Get the system prompt for this agent type
    pub fn system_prompt(&self) -> String {
        match self {
            Self::Researcher => {
                crate::core::prompt_policy::specialist_researcher_system_prompt_v1()
            }
            Self::Coder => crate::core::prompt_policy::specialist_coder_system_prompt_v1(),
            Self::Analyst => crate::core::prompt_policy::specialist_analyst_system_prompt_v1(),
            Self::Writer => crate::core::prompt_policy::specialist_writer_system_prompt_v1(),
            Self::Validator => crate::core::prompt_policy::specialist_validator_system_prompt_v1(),
            Self::Planner => crate::core::prompt_policy::specialist_planner_system_prompt_v1(),
            Self::Custom { instructions, .. } => instructions.clone(),
        }
    }

    /// Get the name of this agent type
    pub fn name(&self) -> String {
        match self {
            Self::Researcher => "Researcher".to_string(),
            Self::Coder => "Coder".to_string(),
            Self::Analyst => "Analyst".to_string(),
            Self::Writer => "Writer".to_string(),
            Self::Validator => "Validator".to_string(),
            Self::Planner => "Planner".to_string(),
            Self::Custom { name, .. } => name.clone(),
        }
    }
}

/// The main orchestration controller
#[derive(Clone)]
pub struct Orchestra {
    _config: OrchestraConfig,
}

impl Orchestra {
    pub fn new(config: OrchestraConfig) -> Self {
        Self { _config: config }
    }
}
