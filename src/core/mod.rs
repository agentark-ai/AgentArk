//! Core agent module - the brain of AgentArk

mod agent;
pub mod autonomy;
pub mod browser_session;
pub mod config;
pub mod connect_flow;
pub mod connector;
pub mod intent;
mod llm;
pub mod orchestra;
pub mod parallel;
pub mod pipeline;
pub mod prompt_policy;
pub mod secrets;
pub mod self_evolve;
pub mod swarm;
mod task;
pub mod task_router;
mod tool_handlers;
pub mod watcher;

pub use agent::{
    Agent, ExecutionTrace, SecurityEvents, SecuritySnapshot, StreamEvent, UserProfile,
};
pub use autonomy::{
    score_action_risk, AutonomySettings, AutopilotMode, ConversationScope, RecommendedAction,
    RiskEnvelope, RiskLevel, TrustPolicy,
};
pub use config::{AgentConfig, ModelRole, ModelSlot};
pub use llm::{LlmClient, LlmProvider, ToolCall};
pub use task::{Task, TaskApproval, TaskQueue, TaskStatus};
