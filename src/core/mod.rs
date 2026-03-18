//! Core agent module - the brain of AgentArk

mod agent;
pub mod automation;
pub mod autonomy;
pub mod browser_session;
pub mod config;
pub mod connect_flow;
pub mod connector;
pub mod intent;
mod llm;
pub mod net;
pub mod observability;
pub mod orchestra;
pub mod parallel;
pub mod pipeline;
pub mod prompt_policy;
pub mod secrets;
pub mod self_evolve;
pub mod self_tune;
pub mod swarm;
mod task;
pub mod task_router;
mod tool_handlers;
pub mod watcher;

pub use agent::{
    Agent, ConversationMessage, ExecutionStep, ExecutionTrace, RequestExecutionHints,
    SecurityEvents, SecuritySnapshot, StreamEvent, UserProfile,
};
pub use automation::{
    list_runs as list_automation_runs, list_supervisor_states as list_automation_supervisor_states,
    AutomationRunStatus, AutomationSupervisorState,
};
pub use autonomy::{
    score_action_risk, AutonomySettings, AutopilotMode, ConversationScope, RecommendedAction,
    RiskEnvelope, RiskLevel, TrustPolicy,
};
pub use config::{AgentConfig, ModelRole, ModelSlot};
pub use llm::{LlmClient, LlmProvider, ToolCall};
pub use task::{status_for_task_approval, Task, TaskApproval, TaskQueue, TaskStatus};
