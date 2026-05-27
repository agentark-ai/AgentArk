//! Core agent module - the brain of AgentArk
#![allow(unused_imports)]

pub(crate) mod action_catalog;
mod agent;
pub mod agentark_knowledge;
pub mod arkorbit;
pub mod artifact_hygiene;
pub mod auth_profiles;
pub mod automation;
pub mod autonomy;
pub mod background_session;
pub mod browser_profiles;
pub mod browser_session;
pub mod companion;
pub mod config;
pub mod connect_flow;
pub mod connector;
pub(crate) mod context_budget;
pub(crate) mod data_contract;
pub mod data_lifecycle;
pub(crate) mod document_search;
pub mod email_delivery;
pub mod embeddings;
pub mod execution;
pub mod gateway;
pub mod gateway_ops;
pub mod inline_artifacts;
pub mod integration_auth;
pub mod integration_sync;
pub mod learning;
pub mod live_run;
mod llm;
pub(crate) mod llm_context_sanitizer;
pub(crate) mod llm_provider;
pub mod memory_dedup;
pub mod memory_schema;
pub mod model_failover;
pub mod net;
pub mod nodes;
pub mod observability;
pub mod orchestra;
pub mod pipeline;
pub mod planner;
pub(crate) mod prompt_fragments;
pub mod prompt_memory;
pub mod prompt_policy;
pub mod readiness;
pub mod release_updates;
pub mod runtime_image;
pub mod secrets;
pub mod self_evolve;
pub mod self_tune;
pub mod sender_verification;
pub(crate) mod skill_marketplaces;
pub mod spawn;
pub mod swarm;
mod task;
pub mod task_router;
pub mod watcher;

pub(crate) use agent::chat_model_is_configured;
pub(crate) use agent::queue_stream_event;
pub(crate) use agent::AUTONOMY_SETTINGS_STORAGE_KEY;
pub(crate) use agent::USER_SELECTED_MODEL_SLOT_KEY;
pub(crate) use agent::{parse_direct_chat_approval_submit_text, DirectChatApprovalSubmitDecision};
pub use agent::{
    Agent, ChatAttachmentHint, ClarificationChoice, ConversationMessage, ExecutionStep,
    ExecutionTrace, QueryComplexity, RequestExecutionHints, SecurityEvents, SecuritySnapshot,
    StreamEvent, UserProfile,
};
pub use arkorbit::{ArkOrbitService, Orbit, OrbitChatMessage, OrbitFileEntry, OrbitManifest};
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
    BrowserProfileLockRequest, BrowserProfileRecord, BrowserProfileResolveCandidate,
    BrowserProfileResolveOutcome, BrowserProfileSessionRecord, BrowserProfileTargetKind,
    BrowserProfileUpsert,
};
pub use companion::{
    companion_presets, presets_response as companion_presets_response,
    protocol_document as companion_protocol_document, CompanionAttestationClaim,
    CompanionAuditEvent, CompanionCommand, CompanionCommandCreate, CompanionCommandDescriptor,
    CompanionCommandStatus, CompanionControlPlane, CompanionDevice, CompanionDeviceAttestation,
    CompanionDeviceState, CompanionGrant, CompanionPairingClaim, CompanionPairingClaimResult,
    CompanionPairingSession, CompanionPairingSessionCreate, CompanionPairingStatus,
    CompanionPresetsResponse, CompanionProtocolDocument, CompanionRiskLevel,
    CompanionTokenRotationRequest, CompanionTokenRotationResult,
};
pub use config::{
    AgentConfig, ModelCapabilityTier, ModelCostTier, ModelHealthScope, ModelRole, ModelSlot,
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
pub use llm::{LlmClient, LlmProvider, LlmResponse, ToolCall};
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
pub use readiness::{DevelopmentalReadiness, ReadinessPolicy};
pub(crate) use task::{one_shot_reminder_is_expired, one_shot_reminder_needs_delay_notice};
pub use task::{
    status_for_task_approval, task_requires_explicit_approval, Task, TaskApproval, TaskQueue,
    TaskStatus,
};
pub(crate) use task::{task_is_one_shot_scheduled_reminder, task_is_scheduled_reminder};
