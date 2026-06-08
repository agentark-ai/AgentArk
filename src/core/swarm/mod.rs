//! Agent Swarm - Multi-agent coordination framework
//!
//! Enables multiple specialized agents to work together on complex tasks.
//! Uses tokio channels for inter-agent communication (no external deps).

#[path = "state/activity.rs"]
pub mod activity;
#[path = "agents/agent_trait.rs"]
pub mod agent_trait;
#[path = "coordination/bus.rs"]
pub mod bus;
#[path = "coordination/coordinator.rs"]
pub mod coordinator;
#[path = "coordination/messages.rs"]
pub mod messages;
#[path = "state/persistence.rs"]
pub mod persistence;
#[path = "agents/registry.rs"]
pub mod registry;
#[path = "agents/specialist.rs"]
pub mod specialist;

pub use activity::{SwarmActivityAgent, SwarmActivityRun, SwarmActivityTracker};
pub use agent_trait::{AgentCapability, AgentId};
pub use coordinator::{SwarmConfig, SwarmManager};
pub use persistence::AgentAccessScope;
pub use specialist::SpecialistConfig;

#[cfg(test)]
#[path = "tests/tests.rs"]
mod tests;
