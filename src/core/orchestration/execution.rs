use crate::actions::ActionDef;
use crate::core::model::llm::{LlmClient, LlmResponse, LlmStreamFailure, LlmStreamFailureKind};
use crate::core::runtime::config::{ModelCapabilityTier, ModelCostTier};
use crate::core::PromptMemory;
use anyhow::{anyhow, Result};
use serde::{Deserialize, Serialize};

#[cfg(test)]
fn now_rfc3339() -> String {
    chrono::Utc::now().to_rfc3339()
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "snake_case")]
pub enum ExecutionRunStatus {
    Accepted,
    Routing,
    ModelSelection,
    Planning,
    ToolDispatch,
    Synthesis,
    Completed,
    Degraded,
    NeedsInput,
    NeedsStrongerModel,
    Blocked,
    PlatformFailed,
    Cancelled,
}

impl ExecutionRunStatus {
    pub fn as_str(&self) -> &'static str {
        match self {
            Self::Accepted => "accepted",
            Self::Routing => "routing",
            Self::ModelSelection => "model_selection",
            Self::Planning => "planning",
            Self::ToolDispatch => "tool_dispatch",
            Self::Synthesis => "synthesis",
            Self::Completed => "completed",
            Self::Degraded => "degraded",
            Self::NeedsInput => "needs_input",
            Self::NeedsStrongerModel => "needs_stronger_model",
            Self::Blocked => "blocked",
            Self::PlatformFailed => "platform_failed",
            Self::Cancelled => "cancelled",
        }
    }
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "snake_case")]
pub enum FailureClass {
    Validation,
    SafetyBlocked,
    HandlerError,
    Timeout,
    Cancelled,
    ModelError,
    ToolError,
    PlatformError,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "snake_case")]
pub enum ToolOutcomeStatus {
    Success,
    RecoverableError,
    FatalError,
    NeedsInput,
    Blocked,
    Cancelled,
    TimedOut,
    NoHandler,
}

impl ToolOutcomeStatus {
    pub fn as_str(&self) -> &'static str {
        match self {
            Self::Success => "success",
            Self::RecoverableError => "recoverable_error",
            Self::FatalError => "fatal_error",
            Self::NeedsInput => "needs_input",
            Self::Blocked => "blocked",
            Self::Cancelled => "cancelled",
            Self::TimedOut => "timed_out",
            Self::NoHandler => "no_handler",
        }
    }
}

#[derive(Debug, Clone, Serialize, Deserialize, Default)]
pub struct DegradationNote {
    pub kind: String,
    pub summary: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub detail: Option<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize, Default)]
pub struct ModelAttemptRecord {
    pub slot_id: String,
    pub slot_label: String,
    pub model_name: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub provider_id: Option<String>,
    pub success: bool,
    pub attempted_at: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub failure_kind: Option<FailureKind>,
    #[serde(default)]
    pub recovery_action: RecoveryAction,
    #[serde(default)]
    pub auto_escalated: bool,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub elapsed_ms: Option<u64>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub error: Option<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq, Default)]
#[serde(rename_all = "snake_case")]
pub enum RequestState {
    NeedsClarification,
    NeedsPermission,
    NeedsIntegration,
    NeedsCredentials,
    NeedsStrongerModel,
    #[default]
    Executing,
    Completed,
    CompletedDegraded,
    HardServiceOutage,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq, Default)]
#[serde(rename_all = "snake_case")]
pub enum FailureKind {
    TransientTransport,
    RateLimited,
    Authentication,
    Configuration,
    ContextWindowExceeded,
    SchemaMismatch,
    ToolContractFailure,
    CapabilityBound,
    UpstreamProvider,
    Timeout,
    MissingInput,
    InternalPostProcess,
    DelegationFailed,
    Panic,
    #[default]
    Unknown,
}

impl FailureKind {
    pub fn as_str(&self) -> &'static str {
        match self {
            Self::TransientTransport => "transient_transport",
            Self::RateLimited => "rate_limited",
            Self::Authentication => "authentication",
            Self::Configuration => "configuration",
            Self::ContextWindowExceeded => "context_window_exceeded",
            Self::SchemaMismatch => "schema_mismatch",
            Self::ToolContractFailure => "tool_contract_failure",
            Self::CapabilityBound => "capability_bound",
            Self::UpstreamProvider => "upstream_provider",
            Self::Timeout => "timeout",
            Self::MissingInput => "missing_input",
            Self::InternalPostProcess => "internal_post_process",
            Self::DelegationFailed => "delegation_failed",
            Self::Panic => "panic",
            Self::Unknown => "unknown",
        }
    }

    pub fn from_str_label(value: &str) -> Option<Self> {
        match value.trim() {
            "transient_transport" => Some(Self::TransientTransport),
            "rate_limited" => Some(Self::RateLimited),
            "authentication" => Some(Self::Authentication),
            "configuration" => Some(Self::Configuration),
            "context_window_exceeded" => Some(Self::ContextWindowExceeded),
            "schema_mismatch" => Some(Self::SchemaMismatch),
            "tool_contract_failure" => Some(Self::ToolContractFailure),
            "capability_bound" => Some(Self::CapabilityBound),
            "upstream_provider" => Some(Self::UpstreamProvider),
            "timeout" => Some(Self::Timeout),
            "missing_input" => Some(Self::MissingInput),
            "internal_post_process" => Some(Self::InternalPostProcess),
            "delegation_failed" => Some(Self::DelegationFailed),
            "panic" => Some(Self::Panic),
            "unknown" => Some(Self::Unknown),
            _ => None,
        }
    }
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq, Default)]
#[serde(rename_all = "snake_case")]
pub enum RecoveryAction {
    #[default]
    None,
    RetrySameModel,
    SwitchModel,
    AskForMissingInput,
    AskForCredentials,
    AskForPermission,
    AskForIntegration,
    AskForStrongerModel,
    ReturnDegraded,
    SurfaceServiceOutage,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq, Default)]
#[serde(rename_all = "snake_case")]
pub enum DelegationStatus {
    #[default]
    Completed,
    Partial,
    Failed,
    TimedOut,
    Panicked,
}

impl DelegationStatus {
    pub fn as_str(&self) -> &'static str {
        match self {
            Self::Completed => "completed",
            Self::Partial => "partial",
            Self::Failed => "failed",
            Self::TimedOut => "timed_out",
            Self::Panicked => "panicked",
        }
    }
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "snake_case")]
pub enum UserFacingOutcomeStatus {
    Complete,
    Degraded,
    NeedsClarification,
    NeedsPermission,
    NeedsIntegration,
    NeedsCredentials,
    NeedsStrongerModel,
    ServiceUnavailable,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct AttemptPolicy {
    pub auto_step_up: bool,
    pub max_model_attempts: usize,
    pub max_same_model_retries: usize,
    pub allow_degraded_completion: bool,
}

impl Default for AttemptPolicy {
    fn default() -> Self {
        Self {
            auto_step_up: true,
            max_model_attempts: 6,
            max_same_model_retries: 1,
            allow_degraded_completion: true,
        }
    }
}

#[derive(Debug, Clone, Serialize, Deserialize, Default)]
pub struct ExecutionRequest {
    pub kind: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub channel: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub conversation_id: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub session_id: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub preferred_model_role: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub message_preview: Option<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct AttemptRecord {
    pub slot_id: String,
    pub slot_label: String,
    pub model_name: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub provider_id: Option<String>,
    pub success: bool,
    pub attempted_at: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub failure_kind: Option<FailureKind>,
    #[serde(default)]
    pub recovery_action: RecoveryAction,
    #[serde(default)]
    pub auto_escalated: bool,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub elapsed_ms: Option<u64>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub error: Option<String>,
}

impl AttemptRecord {
    pub fn to_model_attempt_record(&self) -> ModelAttemptRecord {
        ModelAttemptRecord {
            slot_id: self.slot_id.clone(),
            slot_label: self.slot_label.clone(),
            model_name: self.model_name.clone(),
            provider_id: self.provider_id.clone(),
            success: self.success,
            attempted_at: self.attempted_at.clone(),
            failure_kind: self.failure_kind.clone(),
            recovery_action: self.recovery_action.clone(),
            auto_escalated: self.auto_escalated,
            elapsed_ms: self.elapsed_ms,
            error: self.error.clone(),
        }
    }
}

impl From<&AttemptRecord> for ModelAttemptRecord {
    fn from(value: &AttemptRecord) -> Self {
        value.to_model_attempt_record()
    }
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct UserFacingOutcome {
    pub status: UserFacingOutcomeStatus,
    pub request_state: RequestState,
    pub message: String,
    #[serde(default)]
    pub retryable: bool,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub reason_code: Option<String>,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub degradation: Vec<DegradationNote>,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub attempted_models: Vec<ModelAttemptRecord>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ExecutionOutcome {
    pub request_state: RequestState,
    pub user_outcome: UserFacingOutcome,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub attempts: Vec<AttemptRecord>,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub degradation: Vec<DegradationNote>,
}

#[derive(Debug, Clone)]
pub struct ExecutionCandidateDescriptor {
    pub slot_id: String,
    pub provider_id: Option<String>,
    pub capability_tier: ModelCapabilityTier,
    pub cost_tier: ModelCostTier,
    pub auto_escalate: bool,
    pub escalation_rank: i32,
    pub is_user_selected: bool,
    pub is_primary: bool,
    pub original_index: usize,
}

#[derive(Debug, Clone, Default)]
pub struct ExecutionSupervisor {
    pub policy: AttemptPolicy,
}

impl ExecutionSupervisor {
    pub fn candidate_rank(
        &self,
        candidate: &ExecutionCandidateDescriptor,
        preferred_provider: Option<&str>,
    ) -> (u8, u8, u8, i32, usize) {
        let preferred_provider_match = preferred_provider
            .map(|provider| {
                candidate
                    .provider_id
                    .as_deref()
                    .is_some_and(|value| value.eq_ignore_ascii_case(provider))
                    || candidate.slot_id.eq_ignore_ascii_case(provider)
            })
            .unwrap_or(false);

        let cost_rank: u8 = match candidate.cost_tier {
            ModelCostTier::Low => 0,
            ModelCostTier::Medium => 1,
            ModelCostTier::High => 2,
        };
        let capability_rank: u8 = match candidate.capability_tier {
            ModelCapabilityTier::Economy => 0,
            ModelCapabilityTier::Balanced => 1,
            ModelCapabilityTier::Premium => 2,
        };

        (
            if candidate.is_user_selected { 0 } else { 1 },
            if preferred_provider_match {
                0
            } else if candidate.is_primary {
                1
            } else {
                2
            },
            cost_rank.saturating_add(capability_rank),
            candidate.escalation_rank,
            candidate.original_index,
        )
    }

    pub fn classify_error(&self, error: &anyhow::Error) -> FailureKind {
        if let Some(stream_failure) = error.downcast_ref::<LlmStreamFailure>() {
            return match stream_failure.kind {
                LlmStreamFailureKind::TotalTimeout | LlmStreamFailureKind::InterChunkStall => {
                    FailureKind::Timeout
                }
                LlmStreamFailureKind::NoFirstDelta
                | LlmStreamFailureKind::NoUsefulProgress
                | LlmStreamFailureKind::ChunkErrors
                | LlmStreamFailureKind::EmptyEnd
                | LlmStreamFailureKind::NoUsableContent => FailureKind::TransientTransport,
            };
        }
        self.classify_failure(&error.to_string())
    }

    pub fn classify_failure(&self, error_text: &str) -> FailureKind {
        let lower = error_text.trim().to_ascii_lowercase();
        if lower.is_empty() {
            return FailureKind::Unknown;
        }
        if let Some(kind_marker) = lower
            .split("kind=")
            .nth(1)
            .map(|tail| {
                tail.chars()
                    .take_while(|ch| ch.is_ascii_lowercase() || *ch == '_')
                    .collect::<String>()
            })
            .and_then(|value| FailureKind::from_str_label(&value))
        {
            return kind_marker;
        }
        if lower.contains("timed out") || lower.contains("timeout") {
            return FailureKind::Timeout;
        }
        if lower.contains("rate limit") || lower.contains("rate-limit") || lower.contains("429") {
            return FailureKind::RateLimited;
        }
        if lower.contains("invalid api key")
            || lower.contains("authentication")
            || lower.contains("unauthorized")
            || lower.contains("forbidden")
            || lower.contains("permission denied")
        {
            return FailureKind::Authentication;
        }
        if lower.contains("model_not_found")
            || lower.contains("does not exist or you do not have access")
        {
            return FailureKind::Configuration;
        }
        if lower.contains("missing api key")
            || lower.contains("not configured")
            || lower.contains("base url is required")
            || lower.contains("unknown provider")
        {
            return FailureKind::Configuration;
        }
        if lower.contains("context length")
            || lower.contains("maximum context")
            || lower.contains("too many tokens")
        {
            return FailureKind::ContextWindowExceeded;
        }
        if lower.contains("invalid schema")
            || lower.contains("schema mismatch")
            || lower.contains("response was not valid json")
        {
            return FailureKind::SchemaMismatch;
        }
        if lower.contains("invalid_function_parameters")
            || lower.contains("tool schema")
            || lower.contains("tool call")
        {
            return FailureKind::ToolContractFailure;
        }
        if lower.contains("model produced no")
            || lower.contains("capability")
            || lower.contains("stronger model")
        {
            return FailureKind::CapabilityBound;
        }
        if lower.contains("connection reset")
            || lower.contains("broken pipe")
            || lower.contains("connection closed")
            || lower.contains("stream ended unexpectedly")
            || lower.contains("connect error")
        {
            return FailureKind::TransientTransport;
        }
        if lower.contains("api error")
            || lower.contains("provider returned error")
            || lower.contains("bad request")
            || lower.contains("upstream")
        {
            return FailureKind::UpstreamProvider;
        }
        if lower.contains("missing input") || lower.contains("required input") {
            return FailureKind::MissingInput;
        }
        if lower.contains("proof generation") || lower.contains("post-process") {
            return FailureKind::InternalPostProcess;
        }
        if lower.contains("panicked") || lower.contains("panic") {
            return FailureKind::Panic;
        }
        FailureKind::Unknown
    }

    pub fn recovery_action_for_failure(
        &self,
        failure_kind: Option<&FailureKind>,
        auto_escalated: bool,
    ) -> RecoveryAction {
        match failure_kind {
            None => RecoveryAction::None,
            Some(FailureKind::Authentication | FailureKind::Configuration) => {
                RecoveryAction::AskForCredentials
            }
            Some(FailureKind::MissingInput) => RecoveryAction::AskForMissingInput,
            Some(
                FailureKind::CapabilityBound
                | FailureKind::ContextWindowExceeded
                | FailureKind::SchemaMismatch
                | FailureKind::ToolContractFailure,
            ) => RecoveryAction::AskForStrongerModel,
            Some(
                FailureKind::RateLimited
                | FailureKind::Timeout
                | FailureKind::TransientTransport
                | FailureKind::UpstreamProvider,
            ) if auto_escalated || self.policy.auto_step_up => RecoveryAction::SwitchModel,
            Some(
                FailureKind::RateLimited
                | FailureKind::Timeout
                | FailureKind::TransientTransport
                | FailureKind::UpstreamProvider,
            ) => RecoveryAction::RetrySameModel,
            Some(FailureKind::InternalPostProcess) => RecoveryAction::ReturnDegraded,
            _ => RecoveryAction::SurfaceServiceOutage,
        }
    }

    pub fn cooldown_secs_for_failure(&self, failure_kind: Option<&FailureKind>) -> Option<i64> {
        match failure_kind {
            Some(FailureKind::RateLimited) => Some(60),
            Some(FailureKind::Timeout | FailureKind::TransientTransport) => Some(30),
            Some(FailureKind::UpstreamProvider) => Some(45),
            Some(FailureKind::Authentication | FailureKind::Configuration) => Some(300),
            _ => None,
        }
    }

    pub fn build_success_outcome(
        &self,
        response: &str,
        degradation: &[DegradationNote],
        attempts: &[AttemptRecord],
    ) -> UserFacingOutcome {
        let degraded = !degradation.is_empty();
        UserFacingOutcome {
            status: if degraded {
                UserFacingOutcomeStatus::Degraded
            } else {
                UserFacingOutcomeStatus::Complete
            },
            request_state: if degraded {
                RequestState::CompletedDegraded
            } else {
                RequestState::Completed
            },
            message: response.to_string(),
            retryable: degraded,
            reason_code: if degraded {
                Some("completed_with_degradation".to_string())
            } else {
                None
            },
            degradation: degradation.to_vec(),
            attempted_models: attempts.iter().map(ModelAttemptRecord::from).collect(),
        }
    }

    #[cfg(test)]
    pub fn build_clarification_outcome(
        &self,
        message: &str,
        attempts: &[AttemptRecord],
    ) -> UserFacingOutcome {
        UserFacingOutcome {
            status: UserFacingOutcomeStatus::NeedsClarification,
            request_state: RequestState::NeedsClarification,
            message: message.to_string(),
            retryable: false,
            reason_code: Some("clarification_required".to_string()),
            degradation: Vec::new(),
            attempted_models: attempts.iter().map(ModelAttemptRecord::from).collect(),
        }
    }

    pub fn build_permission_outcome(
        &self,
        message: &str,
        degradation: &[DegradationNote],
        attempts: &[AttemptRecord],
    ) -> UserFacingOutcome {
        UserFacingOutcome {
            status: UserFacingOutcomeStatus::NeedsPermission,
            request_state: RequestState::NeedsPermission,
            message: message.to_string(),
            retryable: false,
            reason_code: Some("permission_required".to_string()),
            degradation: degradation.to_vec(),
            attempted_models: attempts.iter().map(ModelAttemptRecord::from).collect(),
        }
    }

    pub fn build_integration_outcome(
        &self,
        message: &str,
        degradation: &[DegradationNote],
        attempts: &[AttemptRecord],
    ) -> UserFacingOutcome {
        UserFacingOutcome {
            status: UserFacingOutcomeStatus::NeedsIntegration,
            request_state: RequestState::NeedsIntegration,
            message: message.to_string(),
            retryable: false,
            reason_code: Some("integration_required".to_string()),
            degradation: degradation.to_vec(),
            attempted_models: attempts.iter().map(ModelAttemptRecord::from).collect(),
        }
    }

    pub fn build_credentials_outcome(
        &self,
        message: &str,
        degradation: &[DegradationNote],
        attempts: &[AttemptRecord],
    ) -> UserFacingOutcome {
        UserFacingOutcome {
            status: UserFacingOutcomeStatus::NeedsCredentials,
            request_state: RequestState::NeedsCredentials,
            message: message.to_string(),
            retryable: false,
            reason_code: Some("credentials_required".to_string()),
            degradation: degradation.to_vec(),
            attempted_models: attempts.iter().map(ModelAttemptRecord::from).collect(),
        }
    }

    pub fn build_service_outage_outcome(
        &self,
        message: &str,
        reason_code: &str,
        degradation: &[DegradationNote],
        attempts: &[AttemptRecord],
    ) -> UserFacingOutcome {
        UserFacingOutcome {
            status: UserFacingOutcomeStatus::ServiceUnavailable,
            request_state: RequestState::HardServiceOutage,
            message: message.to_string(),
            retryable: true,
            reason_code: Some(reason_code.to_string()),
            degradation: degradation.to_vec(),
            attempted_models: attempts.iter().map(ModelAttemptRecord::from).collect(),
        }
    }

    pub fn build_failure_outcome(
        &self,
        _request: &ExecutionRequest,
        attempts: &[AttemptRecord],
        degradation: &[DegradationNote],
    ) -> ExecutionOutcome {
        let request_state = if !attempts.is_empty()
            && attempts.iter().all(|attempt| {
                matches!(
                    attempt.failure_kind,
                    Some(FailureKind::Authentication | FailureKind::Configuration)
                )
            }) {
            RequestState::NeedsCredentials
        } else if attempts.iter().any(|attempt| {
            matches!(
                attempt.failure_kind,
                Some(
                    FailureKind::CapabilityBound
                        | FailureKind::ContextWindowExceeded
                        | FailureKind::SchemaMismatch
                        | FailureKind::ToolContractFailure
                )
            )
        }) {
            RequestState::NeedsStrongerModel
        } else {
            RequestState::HardServiceOutage
        };

        let (status, retryable, reason_code, message) = match request_state {
            RequestState::NeedsCredentials => (
                UserFacingOutcomeStatus::NeedsCredentials,
                false,
                Some("credentials_required".to_string()),
                "I couldn't use the configured model chain because the current credentials or provider configuration were rejected. Update the model settings and try again."
                    .to_string(),
            ),
            RequestState::NeedsStrongerModel => (
                UserFacingOutcomeStatus::NeedsStrongerModel,
                false,
                Some("stronger_model_required".to_string()),
                "I exhausted the eligible model chain for this request and hit a capability or context limit. Retry with a stronger model tier and I can continue."
                    .to_string(),
            ),
            _ => (
                UserFacingOutcomeStatus::ServiceUnavailable,
                true,
                Some("provider_chain_unavailable".to_string()),
                "I kept the request inside the resilience layer, but every eligible model attempt failed due to provider instability or transport issues. Please retry in a moment."
                    .to_string(),
            ),
        };

        ExecutionOutcome {
            request_state: request_state.clone(),
            user_outcome: UserFacingOutcome {
                status,
                request_state,
                message,
                retryable,
                reason_code,
                degradation: degradation.to_vec(),
                attempted_models: attempts.iter().map(ModelAttemptRecord::from).collect(),
            },
            attempts: attempts.to_vec(),
            degradation: degradation.to_vec(),
        }
    }
}

fn bounded_helper_output_tokens(env_key: &str, default_tokens: u32) -> u32 {
    std::env::var(env_key)
        .ok()
        .and_then(|value| value.trim().parse::<u32>().ok())
        .unwrap_or(default_tokens)
        .clamp(128, 8_192)
}

fn supervised_request_output_budget(request_kind: &str) -> Option<u32> {
    match request_kind {
        "model_routed_spine_v1" => Some(bounded_helper_output_tokens(
            "AGENTARK_MODEL_ROUTED_SPINE_MAX_OUTPUT_TOKENS",
            2_400,
        )),
        kind if kind.starts_with("user_fact_memory_capture") => Some(bounded_helper_output_tokens(
            "AGENTARK_MEMORY_CAPTURE_MAX_OUTPUT_TOKENS",
            900,
        )),
        _ => Some(bounded_helper_output_tokens(
            "AGENTARK_INTERNAL_HELPER_MAX_OUTPUT_TOKENS",
            1_200,
        )),
    }
}

#[allow(clippy::too_many_arguments)]
pub async fn execute_supervised_transport_chat(
    supervisor: &ExecutionSupervisor,
    llm: &LlmClient,
    request: &ExecutionRequest,
    system_prompt: &str,
    user_message: &str,
    memories: &[PromptMemory],
    actions: &[ActionDef],
    timeout_ms: Option<u64>,
) -> Result<LlmResponse> {
    execute_supervised_transport_chat_with_policy(
        supervisor,
        llm,
        request,
        system_prompt,
        user_message,
        memories,
        actions,
        timeout_ms,
        &crate::security::ModelPrivacyConfig::default(),
        false,
    )
    .await
}

#[allow(clippy::too_many_arguments)]
pub async fn execute_supervised_transport_chat_with_policy(
    supervisor: &ExecutionSupervisor,
    llm: &LlmClient,
    request: &ExecutionRequest,
    system_prompt: &str,
    user_message: &str,
    memories: &[PromptMemory],
    actions: &[ActionDef],
    timeout_ms: Option<u64>,
    policy: &crate::security::ModelPrivacyConfig,
    allow_sensitive_context: bool,
) -> Result<LlmResponse> {
    let max_output_tokens = supervised_request_output_budget(&request.kind);
    let response = if let Some(timeout_ms) = timeout_ms.filter(|value| *value > 0) {
        match tokio::time::timeout(
            std::time::Duration::from_millis(timeout_ms),
            llm.chat_for_helper_request_limited(
                system_prompt,
                user_message,
                memories,
                actions,
                policy,
                allow_sensitive_context,
                max_output_tokens,
            ),
        )
        .await
        {
            Ok(result) => result,
            Err(_) => {
                let failure_kind = FailureKind::Timeout;
                return Err(anyhow!(
                    "supervised_chat_failed(kind={}, request_kind={}, model={}): request timed out after {}ms",
                    failure_kind.as_str(),
                    request.kind,
                    llm.model_name(),
                    timeout_ms
                ));
            }
        }
    } else {
        llm.chat_for_helper_request_limited(
            system_prompt,
            user_message,
            memories,
            actions,
            policy,
            allow_sensitive_context,
            max_output_tokens,
        )
        .await
    };

    response.map_err(|error| {
        let failure_kind = supervisor.classify_error(&error);
        anyhow!(
            "supervised_chat_failed(kind={}, request_kind={}, model={}): {}",
            failure_kind.as_str(),
            request.kind,
            llm.model_name(),
            error
        )
    })
}

#[allow(clippy::too_many_arguments)]
pub async fn execute_supervised_transport_chat_stream_with_policy(
    supervisor: &ExecutionSupervisor,
    llm: &LlmClient,
    request: &ExecutionRequest,
    system_prompt: &str,
    user_message: &str,
    memories: &[PromptMemory],
    actions: &[ActionDef],
    timeout_ms: Option<u64>,
    token_tx: tokio::sync::mpsc::Sender<crate::core::agent::StreamEvent>,
    long_running_stream: bool,
    policy: &crate::security::ModelPrivacyConfig,
    allow_sensitive_context: bool,
) -> Result<LlmResponse> {
    let history: Vec<crate::core::agent::ConversationMessage> = Vec::new();
    let response = if let Some(timeout_ms) = timeout_ms.filter(|value| *value > 0) {
        match tokio::time::timeout(std::time::Duration::from_millis(timeout_ms), async {
            if long_running_stream {
                llm.chat_with_history_stream_for_long_running_tool(
                    system_prompt,
                    user_message,
                    &history,
                    memories,
                    actions,
                    token_tx,
                    policy,
                    allow_sensitive_context,
                )
                .await
            } else {
                llm.chat_with_history_stream_for_helper(
                    system_prompt,
                    user_message,
                    &history,
                    memories,
                    actions,
                    token_tx,
                    policy,
                    allow_sensitive_context,
                )
                .await
            }
        })
        .await
        {
            Ok(result) => result,
            Err(_) => {
                let failure_kind = FailureKind::Timeout;
                return Err(anyhow!(
                    "supervised_chat_failed(kind={}, request_kind={}, model={}): request timed out after {}ms",
                    failure_kind.as_str(),
                    request.kind,
                    llm.model_name(),
                    timeout_ms
                ));
            }
        }
    } else if long_running_stream {
        llm.chat_with_history_stream_for_long_running_tool(
            system_prompt,
            user_message,
            &history,
            memories,
            actions,
            token_tx,
            policy,
            allow_sensitive_context,
        )
        .await
    } else {
        llm.chat_with_history_stream_for_helper(
            system_prompt,
            user_message,
            &history,
            memories,
            actions,
            token_tx,
            policy,
            allow_sensitive_context,
        )
        .await
    };

    response.map_err(|error| {
        let failure_kind = supervisor.classify_error(&error);
        anyhow!(
            "supervised_chat_failed(kind={}, request_kind={}, model={}): {}",
            failure_kind.as_str(),
            request.kind,
            llm.model_name(),
            error
        )
    })
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ExecutionRun {
    pub id: String,
    pub kind: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub request_id: Option<String>,
    pub status: ExecutionRunStatus,
    pub current_stage: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub lease_owner: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub lease_expires_at: Option<String>,
    pub attempt: u32,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub deadline_at: Option<String>,
    #[serde(default)]
    pub cancellation_requested: bool,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub degradation: Vec<DegradationNote>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub last_error: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub result_summary: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub trace_id: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub conversation_id: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub channel: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub request_message: Option<String>,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub attempted_models: Vec<ModelAttemptRecord>,
    pub created_at: String,
    pub updated_at: String,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ExecutionCheckpoint {
    pub run_id: String,
    pub sequence_no: u32,
    pub stage: String,
    pub payload: String,
    pub created_at: String,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ToolAttempt {
    pub id: String,
    pub run_id: String,
    pub sequence_no: u32,
    pub tool_name: String,
    pub status: ToolOutcomeStatus,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub failure_class: Option<FailureClass>,
    pub retryable: bool,
    pub side_effect_level: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub idempotency_key: Option<String>,
    pub arguments_json: String,
    pub output_json: String,
    pub started_at: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub completed_at: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub error_text: Option<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ToolOutcome {
    pub name: String,
    pub content: String,
    pub status: ToolOutcomeStatus,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub failure_class: Option<FailureClass>,
    #[serde(default)]
    pub retryable: bool,
    pub side_effect_level: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub error: Option<String>,
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn classify_failure_detects_capability_limits() {
        let supervisor = ExecutionSupervisor::default();
        assert_eq!(
            supervisor.classify_failure(
                "provider rejected the request because the context length was too large"
            ),
            FailureKind::ContextWindowExceeded
        );
        assert_eq!(
            supervisor.classify_failure("invalid schema for function file_write"),
            FailureKind::SchemaMismatch
        );
    }

    #[test]
    fn classify_failure_uses_internal_kind_marker() {
        let supervisor = ExecutionSupervisor::default();
        assert_eq!(
            supervisor.classify_failure(
                "supervised_chat_failed(kind=transient_transport, request_kind=model_routed_spine_v1, model=m): provider stream stalled"
            ),
            FailureKind::TransientTransport
        );
    }

    #[test]
    fn classify_failure_treats_provider_model_not_found_as_configuration() {
        let supervisor = ExecutionSupervisor::default();
        assert_eq!(
            supervisor.classify_failure(
                r#"OpenAI API error (404 Not Found): {"message":"Model zai-glm-4.7 does not exist or you do not have access to it.","type":"not_found_error","param":"model","code":"model_not_found"}"#
            ),
            FailureKind::Configuration
        );
    }

    #[test]
    fn build_failure_outcome_prefers_stronger_model_path() {
        let supervisor = ExecutionSupervisor::default();
        let outcome = supervisor.build_failure_outcome(
            &ExecutionRequest {
                kind: "chat".to_string(),
                ..Default::default()
            },
            &[AttemptRecord {
                slot_id: "fast".to_string(),
                slot_label: "Fast".to_string(),
                model_name: "cheap".to_string(),
                provider_id: Some("openai".to_string()),
                success: false,
                attempted_at: now_rfc3339(),
                failure_kind: Some(FailureKind::ContextWindowExceeded),
                recovery_action: RecoveryAction::AskForStrongerModel,
                auto_escalated: false,
                elapsed_ms: Some(1000),
                error: Some("context length exceeded".to_string()),
            }],
            &[],
        );

        assert_eq!(outcome.request_state, RequestState::NeedsStrongerModel);
        assert_eq!(
            outcome.user_outcome.status,
            UserFacingOutcomeStatus::NeedsStrongerModel
        );
    }

    #[test]
    fn build_clarification_outcome_marks_request_as_pending_input() {
        let supervisor = ExecutionSupervisor::default();
        let outcome =
            supervisor.build_clarification_outcome("Which repository should I update?", &[]);

        assert_eq!(outcome.status, UserFacingOutcomeStatus::NeedsClarification);
        assert_eq!(outcome.request_state, RequestState::NeedsClarification);
        assert_eq!(
            outcome.reason_code.as_deref(),
            Some("clarification_required")
        );
    }

    #[test]
    fn single_model_failure_marks_credentials_needed() {
        let supervisor = ExecutionSupervisor::default();
        let outcome = supervisor.build_failure_outcome(
            &ExecutionRequest {
                kind: "chat".to_string(),
                preferred_model_role: Some("primary".to_string()),
                ..Default::default()
            },
            &[AttemptRecord {
                slot_id: "only-slot".to_string(),
                slot_label: "Primary".to_string(),
                model_name: "single-model".to_string(),
                provider_id: Some("openai".to_string()),
                success: false,
                attempted_at: now_rfc3339(),
                failure_kind: Some(FailureKind::Authentication),
                recovery_action: RecoveryAction::AskForCredentials,
                auto_escalated: false,
                elapsed_ms: Some(250),
                error: Some("401 unauthorized".to_string()),
            }],
            &[],
        );

        assert_eq!(outcome.request_state, RequestState::NeedsCredentials);
        assert_eq!(
            outcome.user_outcome.status,
            UserFacingOutcomeStatus::NeedsCredentials
        );
    }

    #[test]
    fn single_model_failure_marks_service_unavailable_for_transport_issues() {
        let supervisor = ExecutionSupervisor::default();
        let outcome = supervisor.build_failure_outcome(
            &ExecutionRequest {
                kind: "chat".to_string(),
                preferred_model_role: Some("primary".to_string()),
                ..Default::default()
            },
            &[AttemptRecord {
                slot_id: "only-slot".to_string(),
                slot_label: "Primary".to_string(),
                model_name: "single-model".to_string(),
                provider_id: Some("openai".to_string()),
                success: false,
                attempted_at: now_rfc3339(),
                failure_kind: Some(FailureKind::Timeout),
                recovery_action: RecoveryAction::RetrySameModel,
                auto_escalated: false,
                elapsed_ms: Some(5000),
                error: Some("request timed out".to_string()),
            }],
            &[],
        );

        assert_eq!(outcome.request_state, RequestState::HardServiceOutage);
        assert_eq!(
            outcome.user_outcome.status,
            UserFacingOutcomeStatus::ServiceUnavailable
        );
    }

    #[test]
    fn candidate_rank_prefers_user_selected_then_cheaper() {
        let supervisor = ExecutionSupervisor::default();
        let fast = ExecutionCandidateDescriptor {
            slot_id: "fast".to_string(),
            provider_id: Some("openai".to_string()),
            capability_tier: ModelCapabilityTier::Economy,
            cost_tier: ModelCostTier::Low,
            auto_escalate: true,
            escalation_rank: 10,
            is_user_selected: false,
            is_primary: false,
            original_index: 0,
        };
        let premium = ExecutionCandidateDescriptor {
            slot_id: "premium".to_string(),
            provider_id: Some("anthropic".to_string()),
            capability_tier: ModelCapabilityTier::Premium,
            cost_tier: ModelCostTier::High,
            auto_escalate: true,
            escalation_rank: 50,
            is_user_selected: true,
            is_primary: false,
            original_index: 1,
        };

        assert!(supervisor.candidate_rank(&premium, None) < supervisor.candidate_rank(&fast, None));
    }
}
