//! Core abstractions for the agent swarm

use anyhow::Result;
use async_trait::async_trait;
use serde::{Deserialize, Serialize};
use uuid::Uuid;

/// Unique identifier for an agent within the swarm
#[derive(Debug, Clone, Hash, Eq, PartialEq, Serialize, Deserialize)]
pub struct AgentId(pub String);

impl AgentId {
    pub fn new() -> Self {
        Self(Uuid::new_v4().to_string())
    }
}

impl Default for AgentId {
    fn default() -> Self {
        Self::new()
    }
}

impl std::fmt::Display for AgentId {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(f, "{}", self.0)
    }
}

/// Capability descriptor -- what an agent can do
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct AgentCapability {
    pub name: String,
    pub description: String,
    pub keywords: Vec<String>,
}

/// Metadata about an agent for registry/discovery
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct AgentInfo {
    pub id: AgentId,
    pub name: String,
    pub agent_type: String,
    pub capabilities: Vec<AgentCapability>,
    pub status: AgentStatus,
    pub llm_model: String,
}

/// Agent status within the swarm
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub enum AgentStatus {
    Idle,
    Busy,
    Offline,
}

/// The trait that all agents in the swarm implement
#[async_trait]
pub trait SwarmAgent: Send + Sync {
    fn info(&self) -> AgentInfo;
    fn id(&self) -> &AgentId;
    /// Score how well this agent can handle a task (0.0 to 1.0)
    fn can_handle(&self, task_description: &str) -> f32;
}

/// A message sent between agents in the swarm
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct SwarmMessage {
    pub id: Uuid,
    pub from: AgentId,
    pub to: AgentId,
    pub content: String,
    pub context: Option<String>,
    pub parent_task_id: Option<Uuid>,
    pub timestamp: chrono::DateTime<chrono::Utc>,
}

impl SwarmMessage {
    #[cfg(test)]
    pub fn new(from: AgentId, to: AgentId, content: String) -> Self {
        Self {
            id: Uuid::new_v4(),
            from,
            to,
            content,
            context: None,
            parent_task_id: None,
            timestamp: chrono::Utc::now(),
        }
    }
}
