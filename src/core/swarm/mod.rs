//! Agent Swarm - Multi-agent coordination framework
//!
//! Enables multiple specialized agents to work together on complex tasks.
//! Uses tokio channels for inter-agent communication (no external deps).

pub mod activity;
pub mod agent_trait;
pub mod bus;
pub mod coordinator;
pub mod messages;
pub mod persistence;
pub mod registry;
pub mod specialist;

pub use activity::{SwarmActivityAgent, SwarmActivityRun, SwarmActivityTracker};
pub use agent_trait::{AgentCapability, AgentId};
pub use coordinator::{SwarmConfig, SwarmManager};
pub use persistence::AgentAccessScope;
pub use specialist::SpecialistConfig;

#[cfg(test)]
mod tests;
