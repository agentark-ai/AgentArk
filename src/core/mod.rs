//! Core agent module - the brain of AgentArk
#![allow(unused_imports)]

mod agent;
pub mod automation;
pub mod autonomy;
pub mod browser_profiles;
pub mod browser_session;
pub mod config;
pub mod connect_flow;
pub mod connector;
pub mod data_lifecycle;
pub(crate) mod document_search;
pub mod execution;
pub mod gateway;
pub mod gateway_ops;
pub mod integration_sync;
pub mod intent;
pub mod learning;
mod llm;
pub mod model_failover;
pub mod net;
pub mod nodes;
pub mod observability;
pub mod orchestra;
pub mod parallel;
pub mod pipeline;
pub mod prompt_policy;
pub mod product_help;
pub mod secrets;
pub mod self_evolve;
pub mod self_tune;
pub mod sender_verification;
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
pub use browser_profiles::{
    BrowserLoginState, BrowserProfileControlPlane, BrowserProfileListResponse,
    BrowserProfileLockRequest, BrowserProfileRecord, BrowserProfileSessionRecord,
    BrowserProfileTargetKind, BrowserProfileUpsert,
};
pub use config::{
    AgentConfig, ModelCapabilityTier, ModelCostTier, ModelHealthScope, ModelRole, ModelSlot,
};
pub use execution::{
    execute_supervised_transport_chat, AttemptPolicy, AttemptRecord, DegradationNote,
    DelegationStatus, ExecutionCandidateDescriptor, ExecutionCheckpoint, ExecutionOutcome,
    ExecutionRequest, ExecutionRun, ExecutionRunStatus, ExecutionSupervisor, FailureClass,
    FailureKind, ModelAttemptRecord, RecoveryAction, RequestState, ToolAttempt, ToolOutcome,
    ToolOutcomeStatus, UserFacingOutcome, UserFacingOutcomeStatus,
};
pub use gateway::{
    create_broadcast_group as create_gateway_broadcast_group,
    delete_channel_account as delete_gateway_channel_account,
    delete_route_rule as delete_gateway_route_rule, load_channels as load_gateway_channels,
    load_routing as load_gateway_routing, simulate_routing as simulate_gateway_routing,
    upsert_channel_account as upsert_gateway_channel_account,
    upsert_route_rule as upsert_gateway_route_rule, GatewayBroadcastGroup,
    GatewayBroadcastGroupCreate, GatewayChannelAccount, GatewayChannelAccountUpsert,
    GatewayChannelDescriptor, GatewayChannelsResponse, GatewayChannelsSummary, GatewayRouteRule,
    GatewayRouteRuleUpsert, GatewayRoutingResponse, GatewayRoutingSimulation,
    GatewayRoutingSimulationRequest, GatewayRoutingSummary,
};
pub use gateway_ops::{
    GatewayOpsControlPlane, GatewayOpsHighlight, GatewayOpsOperatorCheck, GatewayOpsOverview,
    GatewayOpsServiceSummary,
};
pub use llm::{LlmClient, LlmProvider, ToolCall};
pub use model_failover::{
    AuthProfileRecord, AuthProfileUpsert, CooldownClearResult, FallbackCandidate,
    FallbackChainRecord, FallbackChainUpsert, ModelFailoverControlPlane, ModelFailoverListResponse,
    ModelFailoverSelectionRequest, ModelFailoverSelectionResult, ModelSessionPin,
    ProviderHealthEvent, ProviderHealthRecord, ProviderHealthUpsert,
};
pub use nodes::{
    NodeCapability, NodeCommandLogEntry, NodeCommandLogRequest, NodeControlPlane,
    NodeControlPlaneStatus, NodeGrant, NodeHeartbeat, NodeHeartbeatRequest,
    NodePermissionGrantRequest, NodeState, NodeTransportKind, NodeUpsertRequest, PairedNode,
};
pub use task::{status_for_task_approval, Task, TaskApproval, TaskQueue, TaskStatus};
