//! Core agent module - the brain of AgentArk
#![allow(unused_imports)]

mod agent;
pub mod auth_profiles;
pub mod automation;
pub mod autonomy;
pub mod background_session;
pub mod browser_profiles;
pub mod browser_session;
pub mod capability_router;
pub mod config;
pub mod connect_flow;
pub mod connector;
pub mod companion;
pub(crate) mod data_contract;
pub mod data_lifecycle;
pub(crate) mod document_search;
pub mod email_delivery;
pub mod embeddings;
pub mod execution;
pub mod gateway;
pub mod gateway_ops;
pub mod integration_auth;
pub mod integration_sync;
pub mod intent;
pub mod learning;
pub mod live_run;
pub mod memory_dedup;
mod llm;
pub(crate) mod llm_provider;
pub mod model_failover;
pub mod net;
pub mod nodes;
pub mod observability;
pub mod orchestra;
pub mod pipeline;
pub mod planner;
pub mod product_help;
pub mod prompt_memory;
pub mod prompt_policy;
pub mod release_updates;
pub mod request_shape;
pub mod runtime_image;
pub mod secrets;
pub mod self_evolve;
pub mod self_tune;
pub mod sender_verification;
pub mod spawn;
pub mod swarm;
mod task;
pub mod task_router;
mod tool_handlers;
pub mod watcher;

pub(crate) use agent::chat_model_is_configured;
pub(crate) use agent::queue_stream_event;
pub(crate) use agent::AUTONOMY_SETTINGS_STORAGE_KEY;
pub use agent::{
    Agent, ConversationMessage, ExecutionStep, ExecutionTrace, RequestExecutionHints,
    RequestPlanConfirmationMode, SecurityEvents, SecuritySnapshot, StreamEvent, UserProfile,
};
pub use automation::{
    list_runs as list_automation_runs, list_supervisor_states as list_automation_supervisor_states,
    AutomationRunStatus, AutomationSupervisorState,
};
pub use autonomy::{
    score_action_risk, AutonomySettings, AutopilotMode, ConversationScope, RecommendedAction,
    RiskEnvelope, RiskLevel, TrustPolicy,
};
pub use background_session::{
    background_session_id_from_automation, set_background_session_id_in_automation,
    BackgroundSession, BackgroundSessionCreate, BackgroundSessionEvent, BackgroundSessionManager,
    BackgroundSessionPolicy, BackgroundSessionStatus, BackgroundSessionUpdate,
};
pub use browser_profiles::{
    BrowserLoginState, BrowserProfileControlPlane, BrowserProfileListResponse,
    BrowserProfileLockRequest, BrowserProfileRecord, BrowserProfileSessionRecord,
    BrowserProfileTargetKind, BrowserProfileUpsert,
};
pub use config::{
    AgentConfig, ModelCapabilityTier, ModelCostTier, ModelHealthScope, ModelRole, ModelSlot,
};
pub use companion::{
    companion_presets, presets_response as companion_presets_response,
    protocol_document as companion_protocol_document, CompanionAttestationClaim,
    CompanionAuditEvent, CompanionCommand, CompanionCommandCreate, CompanionCommandStatus,
    CompanionControlPlane, CompanionDevice, CompanionDeviceAttestation, CompanionDeviceState,
    CompanionGrant, CompanionPairingClaim, CompanionPairingClaimResult, CompanionPairingSession,
    CompanionPairingSessionCreate, CompanionPairingStatus, CompanionPresetsResponse,
    CompanionProtocolDocument, CompanionRiskLevel,
    CompanionTokenRotationRequest, CompanionTokenRotationResult,
};
pub use embeddings::EmbeddingClient;
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
pub use live_run::{LiveRunRegistry, RunEvent, RunEventPriority};
pub use llm::{LlmClient, LlmProvider, ToolCall};
pub use model_failover::{
    AuthProfileRecord, AuthProfileUpsert, CooldownClearResult, FallbackCandidate,
    FallbackChainRecord, FallbackChainUpsert, ModelFailoverControlPlane, ModelFailoverListResponse,
    ModelFailoverSelectionRequest, ModelFailoverSelectionResult, ModelSessionPin,
    ProviderHealthEvent, ProviderHealthRecord, ProviderHealthUpsert,
};
pub use nodes::{
    NodeCapability, NodeCommandLogEntry, NodeCommandLogRequest, NodeControlPlane,
    NodeControlPlaneStatus, NodeHeartbeat, NodeHeartbeatRequest, NodeState, NodeTransportKind,
    NodeUpsertRequest, PairedNode,
};
pub use planner::{ExecutionPlan, PlanPromptMode, PlanStep, PlanStepStatus, PlanSubstep};
pub use prompt_memory::PromptMemory;
pub use request_shape::RequestShapeAssessment;
pub(crate) use task::{one_shot_reminder_is_expired, one_shot_reminder_needs_delay_notice};
pub(crate) use task::{task_is_one_shot_scheduled_reminder, task_is_scheduled_reminder};
pub use task::{status_for_task_approval, Task, TaskApproval, TaskQueue, TaskStatus};
