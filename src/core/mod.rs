//! Core agent module - the brain of AgentArk
#![allow(unused_imports)]

pub mod agent;
pub mod arkorbit;
pub mod automation;
pub mod connectivity;
pub mod knowledge;
pub mod model;
pub mod orchestration;
pub mod platform;
pub mod request_contract;
pub mod runtime;
pub mod self_evolve;
pub mod swarm;

#[cfg(test)]
mod voice_tests {
    use super::runtime::voice::{
        voice_runtime_config_from_env, VoiceSessionPhase, VoiceSessionRegistry,
    };
    use std::collections::BTreeMap;

    #[test]
    fn voice_runtime_has_no_default_bridge_url() {
        let config = voice_runtime_config_from_env(&BTreeMap::new());

        assert!(!config.enabled);
        assert_eq!(config.disabled_reason.as_deref(), Some("voice_not_enabled"));
        assert_eq!(config.bridge_url, None);
    }

    #[test]
    fn voice_runtime_uses_structured_bridge_url() {
        let mut env = BTreeMap::new();
        env.insert(
            "AGENTARK_VOICE_BRIDGE_URL".to_string(),
            "http://voice.example.test:3105".to_string(),
        );

        let config = voice_runtime_config_from_env(&env);

        assert!(config.enabled);
        assert_eq!(
            config.bridge_url.as_deref(),
            Some("http://voice.example.test:3105")
        );
        assert_eq!(config.disabled_reason, None);
    }

    #[test]
    fn voice_session_registry_tracks_active_chat_thread_without_phrasing() {
        let mut registry = VoiceSessionRegistry::default();

        let session = registry.start_browser_session(Some("conversation-1".to_string()));
        assert_eq!(session.conversation_id.as_deref(), Some("conversation-1"));
        assert_eq!(session.phase, VoiceSessionPhase::Listening);

        let active = registry.active_for_conversation("conversation-1");
        assert_eq!(
            active.as_ref().map(|session| session.id.as_str()),
            Some(session.id.as_str())
        );

        let stopped = registry.stop(&session.id).expect("session should stop");
        assert_eq!(stopped.phase, VoiceSessionPhase::Stopped);
        assert!(registry.active_for_conversation("conversation-1").is_none());
    }

    #[test]
    fn voice_session_registry_can_bind_conversation_after_first_turn() {
        let mut registry = VoiceSessionRegistry::default();
        let session = registry.start_browser_session(None);

        let updated = registry
            .set_conversation_id(&session.id, Some("conversation-2".to_string()))
            .expect("session should update");

        assert_eq!(updated.conversation_id.as_deref(), Some("conversation-2"));
        assert_eq!(
            registry
                .active_for_conversation("conversation-2")
                .as_ref()
                .map(|session| session.id.as_str()),
            Some(session.id.as_str()),
        );
    }

    #[test]
    fn voice_stream_sessions_use_ephemeral_tokens_not_ui_request_headers() {
        let mut registry = VoiceSessionRegistry::default();
        let session = registry.start_browser_stream_session(Some("conversation-3".to_string()));

        let token = registry
            .stream_token_for_session(&session.id)
            .expect("stream sessions should have a token");

        assert!(registry.stream_token_matches(&session.id, &token));
        assert!(!registry.stream_token_matches(&session.id, "wrong-token"));

        let serialized = serde_json::to_value(&session).expect("session should serialize");
        assert!(serialized.get("stream_token").is_none());
    }

    #[test]
    fn voice_registry_reuses_only_stream_capable_sessions_for_streaming() {
        let mut registry = VoiceSessionRegistry::default();
        let non_stream = registry.start_browser_session(Some("conversation-4".to_string()));
        let stream = registry.start_browser_stream_session(Some("conversation-4".to_string()));

        let active = registry.active_stream_for_conversation("conversation-4");

        assert_eq!(
            active.as_ref().map(|session| session.id.as_str()),
            Some(stream.id.as_str())
        );
        assert_ne!(non_stream.id, stream.id);
    }

    #[test]
    fn voice_bridge_stream_url_uses_websocket_scheme_and_encoded_session() {
        let url = super::runtime::voice::voice_bridge_stream_url(
            "http://voice.example.test:3105",
            "session 1",
        )
        .expect("valid bridge url");

        assert_eq!(
            url.as_str(),
            "ws://voice.example.test:3105/sessions/session%201/stream"
        );
    }
}

pub(crate) use agent::ark_distill::{
    arkdistill_contract, parse_arkdistill_profile, sanitize_arkdistill_profile,
    validate_arkdistill_candidate, ArkDistillProfile, ExternalArkDistillCandidate,
    ARKDISTILL_EVENT_TYPE, ARKDISTILL_PROFILE_KEY,
};
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
pub use automation::autonomy::{
    score_action_risk, AutonomySettings, AutopilotMode, ConversationScope, RecommendedAction,
    RiskEnvelope, RiskLevel, TrustPolicy,
};
pub use automation::background_session::{
    background_session_id_from_automation, set_background_session_id_in_automation,
    BackgroundSession, BackgroundSessionCreate, BackgroundSessionEvent, BackgroundSessionManager,
    BackgroundSessionPolicy, BackgroundSessionStatus, BackgroundSessionUpdate,
};
pub use automation::live_run::{LiveRunRegistry, RunEvent, RunEventPriority};
pub(crate) use automation::task::{
    one_shot_reminder_is_expired, one_shot_reminder_needs_delay_notice,
};
pub use automation::task::{
    status_for_task_approval, task_requires_explicit_approval, Task, TaskApproval, TaskQueue,
    TaskStatus,
};
pub(crate) use automation::task::{
    task_is_one_shot_scheduled_reminder, task_is_scheduled_reminder,
};
pub use automation::{
    list_runs as list_automation_runs, list_supervisor_states as list_automation_supervisor_states,
    AutomationRunStatus, AutomationSupervisorState,
};
pub use connectivity::browser_profiles::{
    BrowserLoginState, BrowserProfileControlPlane, BrowserProfileListResponse,
    BrowserProfileLockRequest, BrowserProfileRecord, BrowserProfileResolveCandidate,
    BrowserProfileResolveOutcome, BrowserProfileSessionRecord, BrowserProfileTargetKind,
    BrowserProfileUpsert,
};
pub use connectivity::companion::{
    companion_presets, presets_response as companion_presets_response,
    protocol_document as companion_protocol_document, CompanionAttestationClaim,
    CompanionAuditEvent, CompanionCommand, CompanionCommandCreate, CompanionCommandDescriptor,
    CompanionCommandStatus, CompanionControlPlane, CompanionDevice, CompanionDeviceAttestation,
    CompanionDeviceState, CompanionGrant, CompanionPairingClaim, CompanionPairingClaimResult,
    CompanionPairingSession, CompanionPairingSessionCreate, CompanionPairingStatus,
    CompanionPresetsResponse, CompanionProtocolDocument, CompanionRiskLevel,
    CompanionTokenRotationRequest, CompanionTokenRotationResult,
};
pub use connectivity::gateway::{
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
pub use connectivity::gateway_ops::{
    GatewayOpsControlPlane, GatewayOpsHighlight, GatewayOpsOperatorCheck, GatewayOpsOverview,
    GatewayOpsServiceSummary,
};
pub use knowledge::embeddings::EmbeddingClient;
pub use model::llm::{LlmClient, LlmProvider, LlmResponse, ToolCall};
pub use model::model_failover::{
    AuthProfileRecord, AuthProfileUpsert, CooldownClearResult, FallbackCandidate,
    FallbackChainRecord, FallbackChainUpsert, ModelFailoverControlPlane, ModelFailoverListResponse,
    ModelFailoverSelectionRequest, ModelFailoverSelectionResult, ModelSessionPin,
    ProviderHealthEvent, ProviderHealthRecord, ProviderHealthUpsert,
};
pub use model::prompt_memory::PromptMemory;
pub use orchestration::execution::{
    execute_supervised_transport_chat, AttemptPolicy, AttemptRecord, DegradationNote,
    DelegationStatus, ExecutionCandidateDescriptor, ExecutionCheckpoint, ExecutionOutcome,
    ExecutionRequest, ExecutionRun, ExecutionRunStatus, ExecutionSupervisor, FailureClass,
    FailureKind, ModelAttemptRecord, RecoveryAction, RequestState, ToolAttempt, ToolOutcome,
    ToolOutcomeStatus, UserFacingOutcome, UserFacingOutcomeStatus,
};
pub use orchestration::nodes::{
    NodeCapability, NodeCommandLogEntry, NodeCommandLogRequest, NodeControlPlane,
    NodeControlPlaneStatus, NodeHeartbeat, NodeHeartbeatRequest, NodeState, NodeTransportKind,
    NodeUpsertRequest, PairedNode,
};
pub use orchestration::planner::{
    ExecutionPlan, PlanPromptMode, PlanStep, PlanStepStatus, PlanSubstep,
};
pub use runtime::config::{
    AgentConfig, ModelCapabilityTier, ModelCostTier, ModelHealthScope, ModelRole, ModelSlot,
};
pub use runtime::readiness::{DevelopmentalReadiness, ReadinessPolicy};
pub use runtime::voice::{
    voice_runtime_config_from_current_env, voice_runtime_config_from_env, VoiceRuntimeConfig,
    VoiceSession, VoiceSessionPhase, VoiceSessionRegistry,
};
