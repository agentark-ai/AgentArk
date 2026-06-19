//! Core Agent implementation
use crate::{
    actions::{
        ActionAuthorizationContext, ActionCallerPrincipal, ActionCostTier, ActionDef,
        ActionExecutionSurface, ActionIntegrationClass, ActionRole, ActionSideEffectLevel,
    },
    identity::IdentityManager,
    proofs::ProofEngine,
    runtime::{
        parse_workflow_action_marker, parse_workflow_missing_inputs_marker, ActionRuntime,
        InstalledCliSkillManifest, WorkflowMissingInputsPayload,
    },
    safety::SafetyEngine,
    security::SecurityGuard,
    storage::{DatabaseConfig, Storage},
};
use anyhow::Result;
use regex::Regex;
use sea_orm::{entity::prelude::PgVector, TransactionTrait};
use serde::{Deserialize, Serialize};
use sha2::{Digest, Sha256};
use std::collections::{BTreeMap, HashMap, HashSet};
use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicBool, AtomicUsize, Ordering};
use std::sync::{Arc, Mutex};
use tokio::sync::{broadcast, RwLock};

use super::{
    arkorbit,
    automation::{
        self, append_run as append_automation_run,
        autonomy::{self, ConversationScope},
        background_session, compute_retry_at, critique_result as critique_automation_result,
        current_attempt as automation_current_attempt,
        delete_supervisor_state as delete_automation_supervisor_state,
        increment_attempt as automation_increment_attempt,
        inject_authorization_context as inject_automation_authorization_context,
        inject_context as inject_automation_context,
        load_supervisor_state as load_automation_supervisor_state,
        origin_from_arguments as automation_origin_from_arguments,
        policy_from_arguments as automation_policy_from_arguments,
        policy_from_request_argument as automation_policy_from_request_argument,
        primary_result_text as automation_primary_result_text,
        runtime_authorization_context_from_arguments as automation_runtime_authorization_context,
        task::{self, TaskQueue},
        task_router, truncate_text as automation_truncate_text,
        upsert_supervisor_state as upsert_automation_supervisor_state,
        validation_from_request_argument as automation_validation_from_request_argument, watcher,
        AutomationExecutionPolicy, AutomationOriginContext, AutomationRunRecord,
        AutomationRunStatus, AutomationSupervisorState, AutomationValidation,
        AutomationValidationMode,
    },
    connectivity::browser_session,
    knowledge::{
        document_search::{self, DocumentSearchHit},
        embeddings::EmbeddingClient,
    },
    model::{
        llm::{self, LlmClient, LlmProvider},
        model_failover::{
            ModelFailoverControlPlane, ModelFailoverSelectionRequest, ProviderHealthEvent,
        },
        prompt_memory::PromptMemory,
    },
    orchestration::{
        action_catalog::{
            action_catalog_embedding_has_default_dim, action_catalog_entry_needs_embedding,
            build_action_catalog_descriptor, ActionCatalogSyncStats,
        },
        execution::{
            execute_supervised_transport_chat, ExecutionCandidateDescriptor, ExecutionRequest,
            ExecutionRunStatus, ExecutionSupervisor, RecoveryAction, UserFacingOutcome,
            UserFacingOutcomeStatus,
        },
        orchestra::{Orchestra, OrchestraConfig},
        planner::{ExecutionPlan, PlanPromptMode, PlanStep, PlanStepStatus, PlanSubstep},
    },
    runtime::config::{
        self, AgentConfig, ModelCapabilityTier, ModelCostTier, ModelRole, ModelSlot,
    },
    swarm::{AgentId, SwarmManager},
    RequestState,
};

#[path = "operations/ark_distill.rs"]
pub(crate) mod ark_distill;
#[path = "operations/automation_helpers.rs"]
mod automation_helpers;
#[path = "operations/background_sessions.rs"]
mod background_sessions;
#[path = "operations/capability_readiness.rs"]
pub mod capability_readiness;
#[path = "conversation/chat_approvals.rs"]
mod chat_approvals;
#[path = "conversation/conversation_context.rs"]
mod conversation_context;
#[path = "operations/curator.rs"]
mod curator;
#[path = "operations/daily_brief.rs"]
mod daily_brief;
#[path = "memory/memory.rs"]
mod memory;
#[path = "conversation/message_processing.rs"]
mod message_processing;
#[path = "runtime/model_runtime.rs"]
mod model_runtime;
#[path = "operations/notifications.rs"]
mod notifications;
#[path = "operations/operational.rs"]
mod operational;
#[path = "conversation/outcome_judge.rs"]
mod outcome_judge;
#[path = "conversation/pending_flows.rs"]
mod pending_flows;
#[path = "spine/prompt_builder.rs"]
mod prompt_builder;
#[path = "conversation/request_context.rs"]
mod request_context;
#[path = "memory/resilience_followups.rs"]
mod resilience_followups;
#[path = "skills/skill_import.rs"]
mod skill_import;
#[path = "spine/spine.rs"]
mod spine;
#[path = "spine/spine_prompt_bundle.rs"]
mod spine_prompt_bundle;
#[path = "spine/spine_request.rs"]
mod spine_request;
#[path = "operations/startup.rs"]
mod startup;
#[path = "runtime/streaming.rs"]
mod streaming;
#[path = "runtime/task_runtime.rs"]
mod task_runtime;
#[path = "runtime/tool_execution.rs"]
mod tool_execution;
#[path = "runtime/tool_responses.rs"]
mod tool_responses;
#[path = "memory/watcher_followup.rs"]
mod watcher_followup;

use automation_helpers::*;
use background_sessions::*;
pub(crate) use chat_approvals::{
    parse_direct_chat_approval_submit_text, DirectChatApprovalSubmitDecision,
};
pub use conversation_context::ConversationMessage;
use memory::*;
use notifications::{
    direct_notification_external_message, inbound_security_source_label,
    is_external_notification_channel, notification_channel_display_name,
    notification_channel_not_connected_outcome, notification_push_signature,
    telegram_notification_target_is_configured, whatsapp_notification_target_is_configured,
};
pub use notifications::{NotificationDispatchOutcome, NotificationEvent, NotificationStore};
use request_context::*;
use skill_import::*;
pub(crate) use streaming::queue_stream_event;
pub use streaming::StreamEvent;
use tool_responses::*;
pub(crate) use watcher_followup::{WatcherFollowupPreparation, WatcherFollowupWorker};

pub(crate) fn spine_prompt_section_is_evolvable(section: &str) -> bool {
    let section = section.trim();
    spine_prompt_bundle::ALLOWED_EVOLVABLE_SPINE_FRAGMENT_IDS.contains(&section)
}

const TOOL_INTEGRATION_ALIASES_KEY: &str = "tool_integration_aliases_v1";
const HOOKS_STORAGE_KEY: &str = "hooks_v1";
const DEFAULT_CHAT_HISTORY_CONTEXT_WINDOW_TOKENS: usize = 32_000;
const DEFAULT_CHAT_HISTORY_BUDGET_RATIO_PERCENT: usize = 18;
const MIN_CHAT_HISTORY_TOKEN_BUDGET: usize = 1_024;
const MAX_CHAT_HISTORY_SUMMARY_TOKENS: usize = 8_000;
const DEFAULT_DIRECT_CHAT_FIXED_PROMPT_TOKENS: usize = 2_000;
const MIN_CHAT_MESSAGE_TOKEN_BUDGET: usize = 128;
const MAX_CHAT_MESSAGE_TOKEN_BUDGET: usize = 1_200;
const CONVERSATION_RECENT_ARTIFACT_KEY_PREFIX: &str = "conversation_recent_artifact_v1:";
const CONVERSATION_RECENT_ARTIFACT_LIMIT: usize = 8;
const BACKGROUND_SESSION_IDLE_CONSOLIDATION_AFTER_MINS: i64 = 10;
const BACKGROUND_SESSION_CONSOLIDATION_COOLDOWN_MINS: i64 = 30;
const CONVERSATION_LAST_DEPLOYED_APP_KEY_PREFIX: &str = "conversation_last_deployed_app_v1:";
pub(crate) const USER_SELECTED_MODEL_SLOT_KEY: &str = "user_selected_model_slot_v1";
const AMBIENT_INTENT_KIND: &str = "ambient_intent";
const AMBIENT_INTENT_REVISIT_SOURCE: &str = "ambient_intent_revisit";
const AMBIENT_INTENT_REVISIT_LIMIT: u64 = 32;
const AMBIENT_INTENT_MAX_REVISITS_PER_TICK: usize = 8;
const AMBIENT_INTENT_FALLBACK_RECHECK_HOURS: i64 = 24;
const USER_LEARNED_MEMORY_CAPTURE_SOURCE: &str = "user_lifecycle_memory_capture";
const USER_LEARNED_MEMORY_RETRACTION_SOURCE: &str = "user_lifecycle_memory_retraction";
const USER_FACT_MEMORY_CAPTURE_LOCAL_TIMEOUT_MS: u64 = 6_000;
const USER_FACT_MEMORY_CAPTURE_REMOTE_TIMEOUT_MS: u64 = 45_000;
const USER_FACT_MEMORY_CAPTURE_MAX_CANDIDATES: usize = 2;
const USER_FACT_MEMORY_CAPTURE_EMPTY_ESCALATION_MAX_CANDIDATES: usize = 2;
const USER_FACT_MEMORY_CAPTURE_EMPTY_VERDICT_MIN_CONFIDENCE: f32 = 0.70;
const USER_FACT_MEMORY_CAPTURE_ALLOW_SENSITIVE_CONTEXT: bool = true;
const USER_MEMORY_OPERATION_AUTO_APPLY_CONFIDENCE: f64 = 0.80;
const SAVED_USER_FACT_PROMPT_KINDS: &[&str] = &[
    "identity",
    "assistant_preference",
    "work_preference",
    "project_domain_memory",
    "ephemeral_context",
    "knowledge",
    "preference",
    "location",
    "workflow",
    "constraint",
    "personal_fact",
    "other",
];
const MEMORY_OPERATION_CANDIDATE_TYPES: &[&str] =
    &["memory_add", "memory_update", "memory_retract"];
static ACTION_CATALOG_SYNC_ACTIVE: AtomicBool = AtomicBool::new(false);
pub(crate) const AUTONOMY_SETTINGS_STORAGE_KEY: &str = "autonomy_settings_v1";

#[derive(Debug, Clone, Default, serde::Serialize, serde::Deserialize)]
struct ApprovalRequestMetadata {
    #[serde(default)]
    title: String,
    #[serde(default)]
    summary: String,
    #[serde(default)]
    reason: String,
    #[serde(default)]
    rule_name: String,
    #[serde(default)]
    risk_level: String,
    #[serde(default)]
    risk_score: Option<u8>,
    #[serde(default)]
    source: String,
}

fn approval_metadata_from_arguments(
    arguments: &serde_json::Value,
) -> Option<ApprovalRequestMetadata> {
    serde_json::from_value(arguments.get("_approval")?.clone()).ok()
}

fn approval_rule_name_for_task(_task: &task::Task, metadata: &ApprovalRequestMetadata) -> String {
    if !metadata.rule_name.trim().is_empty() {
        return metadata.rule_name.trim().to_string();
    }
    if !metadata.reason.trim().is_empty() {
        return metadata.reason.trim().to_string();
    }
    "explicit_user_approval_required".to_string()
}

fn approval_notification_text(task: &task::Task, metadata: &ApprovalRequestMetadata) -> String {
    let title = if metadata.title.trim().is_empty() {
        task.description.trim()
    } else {
        metadata.title.trim()
    };
    let summary = if metadata.summary.trim().is_empty() {
        task.description.trim()
    } else {
        metadata.summary.trim()
    };
    let reason = if metadata.reason.trim().is_empty() {
        "This action affects external state or carries elevated execution risk.".to_string()
    } else {
        metadata.reason.trim().to_string()
    };
    let risk = match (metadata.risk_level.trim(), metadata.risk_score) {
        ("", None) => String::new(),
        ("", Some(score)) => format!("\nRisk score: {}", score),
        (level, None) => format!("\nRisk: {}", level),
        (level, Some(score)) => format!("\nRisk: {} ({})", level, score),
    };
    format!(
        "Approval needed: {}\n{}\nWhy: {}{}\nTask ID: {}\nApprove: /approve-task {}\nReject: /reject-task {}\nYou can also review it in Tasks.",
        title, summary, reason, risk, task.id, task.id, task.id
    )
}

fn approval_metadata_has_display_content(metadata: &ApprovalRequestMetadata) -> bool {
    !metadata.title.trim().is_empty()
        || !metadata.summary.trim().is_empty()
        || !metadata.reason.trim().is_empty()
        || !metadata.risk_level.trim().is_empty()
        || metadata.risk_score.is_some()
        || !metadata.source.trim().is_empty()
}

fn task_has_actionable_approval_context(task: &task::Task) -> bool {
    if !matches!(
        task.status,
        task::TaskStatus::AwaitingApproval | task::TaskStatus::ExpiredNeedsReapproval
    ) {
        return false;
    }

    let description = task.description.trim();
    if !description.is_empty() && description != crate::storage::ENCRYPTED_STORAGE_UNAVAILABLE {
        return true;
    }

    approval_metadata_from_arguments(&task.arguments)
        .map(|metadata| approval_metadata_has_display_content(&metadata))
        .unwrap_or(false)
}

/// Safe string truncation that respects UTF-8 character boundaries
/// Best-effort per-request cost estimate in USD for OpenRouter-backed requests.
/// Returns 0.0 when pricing is unavailable rather than guessing with hardcoded rates.
fn estimate_cost_usd(provider: &str, model: &str, input_tokens: u64, output_tokens: u64) -> f64 {
    if !provider.trim().eq_ignore_ascii_case("openrouter") {
        return 0.0;
    }
    crate::channels::http::estimate_cost_from_pricing_cache(model, input_tokens, output_tokens)
        .unwrap_or(0.0)
}

fn safe_truncate(s: &str, max_chars: usize) -> String {
    if s.chars().count() <= max_chars {
        s.to_string()
    } else {
        format!("{}...", s.chars().take(max_chars).collect::<String>())
    }
}

fn provider_error_status_codes(raw: &str) -> Vec<u16> {
    let mut codes = Vec::new();
    let mut digits = String::new();
    let flush = |digits: &mut String, codes: &mut Vec<u16>| {
        if digits.len() == 3 {
            if let Ok(code) = digits.parse::<u16>() {
                if (400..=599).contains(&code) && !codes.contains(&code) {
                    codes.push(code);
                }
            }
        }
        digits.clear();
    };
    for ch in raw.chars() {
        if ch.is_ascii_digit() {
            digits.push(ch);
        } else {
            flush(&mut digits, &mut codes);
        }
    }
    flush(&mut digits, &mut codes);
    codes
}

fn user_visible_platform_failure_message(raw_error: &str) -> String {
    let codes = provider_error_status_codes(raw_error);
    let reason = if codes.contains(&402) {
        "The provider reported an account quota, credits, or billing limit."
    } else if codes.iter().any(|code| *code == 401 || *code == 403) {
        "The provider rejected the configured credentials or model access."
    } else if codes.contains(&429) {
        "The provider reported a rate limit or temporary capacity limit."
    } else if codes.iter().any(|code| matches!(*code, 408 | 504 | 524)) {
        "The provider timed out before returning a usable response."
    } else if codes.iter().any(|code| (500..=599).contains(code)) {
        "The provider reported a temporary service failure."
    } else if codes.iter().any(|code| matches!(*code, 400 | 413 | 422)) {
        "The provider rejected the request as invalid or outside the model's limits."
    } else {
        "The provider returned an error before a usable response was available."
    };
    format!(
        "I couldn't complete the request because the configured model provider failed before returning a usable response. {} Check the provider settings or switch to another configured model, then retry.",
        reason
    )
}

fn internal_llm_timeout_ms(env_key: &str, default_ms: u64) -> u64 {
    const MIN_INTERNAL_LLM_TIMEOUT_MS: u64 = 5_000;
    const MAX_INTERNAL_LLM_TIMEOUT_MS: u64 = 300_000;

    let configured = std::env::var(env_key)
        .ok()
        .and_then(|value| value.trim().parse::<u64>().ok())
        .unwrap_or(default_ms);
    if configured == 0 {
        return 0;
    }
    configured.clamp(MIN_INTERNAL_LLM_TIMEOUT_MS, MAX_INTERNAL_LLM_TIMEOUT_MS)
}

fn extract_http_urls(text: &str) -> Vec<String> {
    let mut urls = Vec::new();
    let mut seen = HashSet::new();
    for token in text.split_whitespace() {
        let candidate = token
            .trim_matches(|c: char| {
                matches!(
                    c,
                    '"' | '\'' | '`' | '<' | '>' | '(' | ')' | '[' | ']' | '{' | '}'
                )
            })
            .trim_end_matches(['.', ',', ';', ':', '!', '?', '`'])
            .trim();
        if candidate.starts_with("http://") || candidate.starts_with("https://") {
            let normalized = candidate.to_string();
            if seen.insert(normalized.clone()) {
                urls.push(normalized);
            }
        }
    }
    urls
}

fn user_data_autosave_url_allowed(raw: &str) -> bool {
    let Ok(parsed) = reqwest::Url::parse(raw.trim()) else {
        return false;
    };
    matches!(parsed.scheme(), "http" | "https")
        && parsed
            .host_str()
            .map(|host| !crate::clients::host_looks_local_or_internal(host))
            .unwrap_or(false)
}

fn extract_user_supplied_link_user_data_urls(text: &str) -> Vec<String> {
    extract_http_urls(text)
        .into_iter()
        .filter(|url| user_data_autosave_url_allowed(url))
        .collect()
}

fn is_sensitive_tool_call_argument_key(key: &str) -> bool {
    matches!(
        key.trim().to_ascii_lowercase().as_str(),
        "token"
            | "api_key"
            | "apikey"
            | "secret"
            | "password"
            | "authorization"
            | "auth"
            | "cookie"
            | "cookies"
            | "headers"
    )
}

fn default_automation_validation_for_action(action_name: &str) -> AutomationValidation {
    match action_name {
        "daily_brief" | "goal_progress_report" | "goal_reminder" => AutomationValidation {
            mode: AutomationValidationMode::NonEmptyResult,
            ..AutomationValidation::default()
        },
        "moltbook" | "gmail" | "google_calendar" | "notion" | "github" | "twitter" => {
            AutomationValidation {
                mode: AutomationValidationMode::StructuredSuccess,
                ..AutomationValidation::default()
            }
        }
        _ => AutomationValidation {
            mode: AutomationValidationMode::NonEmptyResult,
            ..AutomationValidation::default()
        },
    }
}

fn should_retry_background_action(action_name: &str) -> bool {
    !matches!(action_name, "goal" | "goal_progress_report")
}

fn automation_trigger_label(channel: &str, action_name: &str) -> String {
    if channel.eq_ignore_ascii_case("watcher") {
        "watcher".to_string()
    } else if action_name == "daily_brief" {
        "scheduler_daily_brief".to_string()
    } else {
        "scheduler".to_string()
    }
}

fn normalize_model_match_token(raw: &str) -> String {
    raw.trim()
        .trim_matches(|c| matches!(c, '"' | '\'' | '`'))
        .to_ascii_lowercase()
}

fn compact_model_match_token(raw: &str) -> String {
    normalize_model_match_token(raw)
        .chars()
        .filter(|c| c.is_ascii_alphanumeric())
        .collect()
}

fn merge_app_llm_env_from_providers(
    provider_refs: &[&crate::core::LlmProvider],
) -> std::collections::HashMap<String, String> {
    let mut merged: std::collections::HashMap<String, String> = std::collections::HashMap::new();
    for provider in provider_refs {
        for (k, v) in provider.app_env_vars() {
            if v.trim().is_empty() || v == "[ENCRYPTED]" {
                continue;
            }
            merged.entry(k).or_insert(v);
        }
    }

    if !merged.contains_key("OPENROUTER_API_KEY")
        && provider_refs.iter().any(|provider| {
            matches!(
                provider,
                crate::core::LlmProvider::OpenAI { api_key, base_url, .. }
                    if !api_key.trim().is_empty()
                        && base_url
                            .as_deref()
                            .unwrap_or("")
                            .to_ascii_lowercase()
                            .contains("openrouter")
            )
        })
    {
        if let Some(v) = merged.get("OPENAI_API_KEY").cloned() {
            merged.insert("OPENROUTER_API_KEY".to_string(), v);
        }
    }
    if !merged.contains_key("OPENAI_KEY") {
        if let Some(v) = merged.get("OPENAI_API_KEY").cloned() {
            merged.insert("OPENAI_KEY".to_string(), v);
        }
    }
    if !merged.contains_key("OPENAI_TOKEN") {
        if let Some(v) = merged.get("OPENAI_API_KEY").cloned() {
            merged.insert("OPENAI_TOKEN".to_string(), v);
        }
    }

    merged
}

#[derive(Debug, Clone, serde::Serialize, serde::Deserialize)]
enum PendingSecretFollowupKind {
    EnableSkill {
        action_name: String,
    },
    RetryWorkflow {
        payload: WorkflowMissingInputsPayload,
    },
    RestartApp {
        app_id: String,
        title: String,
        missing_env: Vec<String>,
    },
}

#[derive(Debug, Clone, serde::Serialize, serde::Deserialize)]
struct PendingSecretFollowup {
    kind: PendingSecretFollowupKind,
    requested_at: chrono::DateTime<chrono::Utc>,
}

#[derive(Debug, Clone, serde::Serialize, serde::Deserialize)]
enum PendingChatCredentialPromptKind {
    ExtensionPackConnection {
        pack_id: String,
        pack_name: String,
        connection_id: String,
        required_keys: Vec<String>,
    },
    /// Generic integration auth prompt driven by an
    /// [`crate::core::connectivity::integration_auth::IntegrationAuthManifest`]. Same pause/
    /// resume plumbing as the extension-pack variant - this just lets the
    /// manifest system share the existing inline-prompt surface.
    IntegrationAuth {
        integration_id: String,
        origin: IntegrationAuthPromptOrigin,
    },
    RawSecret {
        key: String,
        origin: IntegrationAuthPromptOrigin,
    },
    McpServerAuth {
        server_id: String,
        server_name: String,
        auth_type: String,
        auth_name: Option<String>,
        settings_path: Option<String>,
    },
    CustomApiAuth {
        api_id: String,
        api_name: String,
        auth_mode: String,
        auth_name: Option<String>,
        settings_path: Option<String>,
    },
}

#[derive(Debug, Clone, serde::Serialize, serde::Deserialize)]
enum IntegrationAuthPromptOrigin {
    /// User asked to install/connect an integration.
    InstallIntent,
    /// A tool run requested a `{{secret:KEY}}` that wasn't set yet.
    ToolRuntime {
        #[serde(default, skip_serializing_if = "Option::is_none")]
        tool_name: Option<String>,
        #[serde(default, skip_serializing_if = "Option::is_none")]
        trace_id: Option<String>,
    },
}

#[derive(Debug, Clone, serde::Serialize, serde::Deserialize)]
struct PendingChatCredentialPrompt {
    kind: PendingChatCredentialPromptKind,
    requested_at: chrono::DateTime<chrono::Utc>,
}

#[derive(Debug, Clone, Serialize)]
pub struct ChatCredentialPromptField {
    pub key: String,
    pub label: String,
    pub required: bool,
    /// Input-type hint for the frontend: `"password"`, `"text"`, `"textarea"`,
    /// or `"select"`. Absent for legacy extension-pack prompts (frontend
    /// defaults to password).
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub input_type: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub placeholder: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub help: Option<String>,
    /// Options for `select` inputs.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub options: Option<Vec<String>>,
}

#[derive(Debug, Clone, Serialize)]
pub struct ChatCredentialPrompt {
    pub kind: String,
    pub title: String,
    pub description: String,
    pub warning: String,
    pub submit_label: String,
    pub fallback_command: String,
    pub fields: Vec<ChatCredentialPromptField>,
    /// Stable integration id for manifest-driven prompts. Absent for legacy
    /// extension-pack variants.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub integration_id: Option<String>,
    /// Auth-mode hint for the frontend. One of `"secrets"`,
    /// `"oauth2_authorization_code"`, `"oauth2_device_code"`, `"hybrid"`, or
    /// `"raw_key"`. Absent for legacy variants (frontend renders plain fields).
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub mode_kind: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub docs_url: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub settings_path: Option<String>,
}

const PENDING_RESILIENCE_FOLLOWUP_TTL_HOURS: i64 = 24;

const SCHEDULED_INPUT_NEEDED_MARKER: &str = "__INPUT_NEEDED__:";

#[derive(Debug, Clone, Serialize, Deserialize)]
struct ScheduledInputNeededResult {
    kind: String,
    action: String,
    query: String,
    missing: Vec<String>,
    required: Vec<String>,
    provided: Vec<String>,
    summary: String,
    fix_hint: String,
    notification_title: String,
    notification_body: String,
}

#[derive(Debug, Clone, serde::Serialize, serde::Deserialize)]
struct PendingResilienceFollowup {
    request_state: super::RequestState,
    original_message: String,
    assistant_message: String,
    channel: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    project_id: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    reason_code: Option<String>,
    requested_at: String,
}

#[derive(Debug, Clone, PartialEq, serde::Serialize, serde::Deserialize)]
pub struct ClarificationChoice {
    pub label: String,
    pub submit_text: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub kind: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub approval: Option<DirectChatApprovalChoice>,
}

#[derive(Debug, Clone, PartialEq, serde::Serialize, serde::Deserialize)]
pub struct DirectChatApprovalChoice {
    pub id: String,
    pub decision: String,
    pub action_name: String,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub steps: Vec<DirectChatApprovalStepView>,
}

#[derive(Debug, Clone, serde::Serialize, serde::Deserialize)]
pub struct DirectChatApprovalView {
    pub id: String,
    pub action_name: String,
    pub reason: String,
    pub requested_at: String,
    pub expires_at: String,
    #[serde(default)]
    pub arguments_preview: serde_json::Value,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub steps: Vec<DirectChatApprovalStepView>,
}

#[derive(Debug, Clone, PartialEq, serde::Serialize, serde::Deserialize)]
pub struct DirectChatApprovalStepView {
    pub action_name: String,
    #[serde(default)]
    pub arguments_preview: serde_json::Value,
}

#[derive(Debug, Clone, serde::Serialize, serde::Deserialize)]
pub(crate) struct DirectChatChainApprovalCall {
    pub action_name: String,
    pub arguments: serde_json::Value,
}

fn tokenize_lower(text: &str) -> Vec<String> {
    text.to_lowercase()
        .split(|c: char| !c.is_alphanumeric())
        .filter(|w| w.len() >= 3)
        .map(|w| w.to_string())
        .collect()
}

fn extract_model_failure_tool_name(error: &str) -> Option<String> {
    for marker in ["function '", "function \"", "tool '", "tool \""] {
        let Some(start) = error.find(marker) else {
            continue;
        };
        let rest = &error[start + marker.len()..];
        let terminator = if marker.ends_with('\'') { '\'' } else { '"' };
        if let Some(end) = rest.find(terminator) {
            let name = rest[..end].trim();
            if !name.is_empty() {
                return Some(name.to_string());
            }
        }
    }
    None
}

fn format_model_timeout_duration(ms: u64) -> String {
    let seconds = (ms / 1_000).max(1);
    if seconds >= 60 {
        let minutes = seconds / 60;
        let remainder = seconds % 60;
        if remainder == 0 {
            format!("{} minutes", minutes)
        } else {
            format!("{} minutes {} seconds", minutes, remainder)
        }
    } else {
        format!("{} seconds", seconds)
    }
}

fn summarize_model_failure_for_user(error: &str) -> String {
    let trimmed = error.trim();
    if trimmed.is_empty() {
        return "A configured model failed unexpectedly.".to_string();
    }

    let (label, detail) = match trimmed.split_once(" failed: ") {
        Some((head, tail)) if !head.trim().is_empty() => (Some(head.trim()), tail.trim()),
        _ => (None, trimmed),
    };
    let lower = detail.to_ascii_lowercase();
    let prefix = label
        .map(|value| format!("{}: ", value))
        .unwrap_or_default();

    if lower.contains("model_not_found")
        || lower.contains("does not exist or you do not have access")
    {
        return format!(
            "{}could not use the requested model because the provider says it does not exist or this key has no access.",
            prefix
        );
    }
    if lower.contains("invalid schema for function")
        || lower.contains("invalid_function_parameters")
    {
        let tool_name = extract_model_failure_tool_name(detail)
            .unwrap_or_else(|| "a framework tool".to_string());
        return format!(
            "{}rejected the current tool schema for `{}`.",
            prefix, tool_name
        );
    }
    if lower.contains("timed out") {
        return format!("{}timed out before responding.", prefix);
    }
    if (lower.contains("localhost:11434")
        || lower.contains("127.0.0.1:11434")
        || lower.contains("/api/chat"))
        && (lower.contains("error sending request for url")
            || lower.contains("connection refused")
            || lower.contains("connection reset")
            || lower.contains("transport"))
    {
        return format!(
            "{}could not reach the configured local model service.",
            prefix
        );
    }
    if lower.contains("dns error")
        || lower.contains("no such host")
        || lower.contains("name or service not known")
    {
        return format!(
            "{}could not reach the configured provider endpoint.",
            prefix
        );
    }
    if lower.contains("rate limit") || lower.contains("rate-limit") || lower.contains("429") {
        return format!("{}was rate-limited by the provider.", prefix);
    }
    if lower.contains("context length")
        || lower.contains("maximum context")
        || lower.contains("too many tokens")
    {
        return format!(
            "{}rejected the request because the context was too large.",
            prefix
        );
    }
    if lower.contains("provider returned error")
        || lower.contains("api error")
        || lower.contains("bad request")
    {
        return format!("{}returned an upstream provider error.", prefix);
    }

    format!("{}failed: {}.", prefix, safe_truncate(detail, 160))
}

fn summarize_model_attempt_failure_for_user(attempt: &crate::core::ModelAttemptRecord) -> String {
    let label = attempt.slot_label.trim();
    let prefix = if label.is_empty() {
        String::new()
    } else {
        format!("{}: ", label)
    };
    if let Some(error) = attempt.error.as_deref() {
        let lower = error.to_ascii_lowercase();
        if lower.contains("model_not_found")
            || lower.contains("does not exist or you do not have access")
        {
            let model = attempt.model_name.trim();
            if model.is_empty() {
                return format!(
                    "{}could not use the requested model because the provider says it does not exist or this key has no access.",
                    prefix
                );
            }
            return format!(
                "{}could not use model `{}` because the provider says it does not exist or this key has no access.",
                prefix, model
            );
        }
    }
    match attempt.failure_kind.as_ref() {
        Some(crate::core::FailureKind::Timeout) => {
            if let Some(elapsed_ms) = attempt.elapsed_ms {
                format!(
                    "{}timed out before responding after {}.",
                    prefix,
                    format_model_timeout_duration(elapsed_ms)
                )
            } else {
                format!("{}timed out before responding.", prefix)
            }
        }
        Some(crate::core::FailureKind::TransientTransport) => {
            format!("{}lost the provider transport connection.", prefix)
        }
        Some(crate::core::FailureKind::UpstreamProvider) => {
            format!("{}returned an upstream provider error.", prefix)
        }
        Some(crate::core::FailureKind::RateLimited) => {
            format!("{}was rate-limited by the provider.", prefix)
        }
        Some(crate::core::FailureKind::Authentication) => {
            format!("{}was rejected by provider authentication.", prefix)
        }
        Some(crate::core::FailureKind::Configuration) => {
            format!(
                "{}has invalid or incomplete provider configuration.",
                prefix
            )
        }
        Some(crate::core::FailureKind::ContextWindowExceeded) => {
            format!(
                "{}rejected the request because the context was too large.",
                prefix
            )
        }
        Some(crate::core::FailureKind::SchemaMismatch) => {
            format!("{}returned a response schema mismatch.", prefix)
        }
        Some(crate::core::FailureKind::ToolContractFailure) => {
            format!("{}could not satisfy the tool-call contract.", prefix)
        }
        Some(crate::core::FailureKind::CapabilityBound) => {
            format!("{}hit a model capability limit.", prefix)
        }
        Some(crate::core::FailureKind::MissingInput) => {
            format!("{}needed missing input.", prefix)
        }
        Some(crate::core::FailureKind::InternalPostProcess) => {
            format!("{}failed during internal post-processing.", prefix)
        }
        Some(crate::core::FailureKind::DelegationFailed) => {
            format!("{}failed during delegated execution.", prefix)
        }
        Some(crate::core::FailureKind::Panic) => {
            format!("{}failed unexpectedly.", prefix)
        }
        Some(crate::core::FailureKind::Unknown) | None => attempt
            .error
            .as_deref()
            .map(summarize_model_failure_for_user)
            .unwrap_or_else(|| format!("{}failed unexpectedly.", prefix)),
    }
}

fn summarize_model_failures_for_user(attempts: &[crate::core::ModelAttemptRecord]) -> String {
    let mut summaries = Vec::new();
    let mut seen = HashSet::new();
    for attempt in attempts {
        let summary = summarize_model_attempt_failure_for_user(attempt);
        if seen.insert(summary.clone()) {
            summaries.push(summary);
        }
        if summaries.len() >= 3 {
            break;
        }
    }
    summaries.join(" ")
}

fn legacy_llm_is_unconfigured_placeholder(config: &AgentConfig) -> bool {
    config.model_pool.slots.is_empty()
        && config.llm_fallback.is_none()
        && matches!(
            &config.llm,
            LlmProvider::Ollama { base_url, model }
                if model.trim().is_empty() && base_url.trim().is_empty()
        )
}

fn legacy_llm_is_explicitly_configured(config: &AgentConfig) -> bool {
    if config.model_pool.slots.is_empty() && config.llm_fallback.is_none() {
        match &config.llm {
            LlmProvider::Ollama { base_url, model } => {
                !base_url.trim().is_empty() && !model.trim().is_empty()
            }
            LlmProvider::Anthropic { api_key, model } => {
                !api_key.trim().is_empty() && !model.trim().is_empty()
            }
            LlmProvider::OpenAI {
                api_key,
                model,
                base_url,
            } => {
                !api_key.trim().is_empty()
                    && !model.trim().is_empty()
                    && (base_url.is_none()
                        || base_url.as_ref().is_some_and(|url| !url.trim().is_empty()))
            }
        }
    } else {
        false
    }
}

fn llm_provider_is_structurally_configured(provider: &LlmProvider) -> bool {
    match provider {
        LlmProvider::Ollama { base_url, model } => {
            !base_url.trim().is_empty() && !model.trim().is_empty()
        }
        LlmProvider::Anthropic { api_key, model } => {
            !api_key.trim().is_empty() && !model.trim().is_empty()
        }
        LlmProvider::OpenAI {
            api_key,
            model,
            base_url,
        } => {
            if model.trim().is_empty() {
                return false;
            }
            match crate::core::model::llm_provider::openai_provider_label(base_url.as_deref()) {
                "openai" => !api_key.trim().is_empty(),
                "openrouter" | "openai-subscription" | "huggingface" => {
                    !api_key.trim().is_empty()
                        && base_url.as_ref().is_some_and(|url| !url.trim().is_empty())
                }
                "openai-compatible" => base_url.as_ref().is_some_and(|url| !url.trim().is_empty()),
                _ => false,
            }
        }
    }
}

fn model_pool_slot_is_configured(slot: &ModelSlot) -> bool {
    slot.enabled && llm_provider_is_structurally_configured(&slot.provider)
}

pub(crate) fn chat_model_is_configured(config: &AgentConfig) -> bool {
    if !config.model_pool.slots.is_empty() {
        return config
            .model_pool
            .slots
            .iter()
            .any(model_pool_slot_is_configured);
    }

    !legacy_llm_is_unconfigured_placeholder(config) && legacy_llm_is_explicitly_configured(config)
}

fn enrich_supervisor_outcome_with_model_failures(outcome: &mut crate::core::UserFacingOutcome) {
    let failures: Vec<crate::core::ModelAttemptRecord> = outcome
        .attempted_models
        .iter()
        .filter(|attempt| !attempt.success)
        .cloned()
        .collect();
    let summary = summarize_model_failures_for_user(&failures);
    if summary.is_empty() || outcome.message.contains(&summary) {
        return;
    }
    outcome.message = format!(
        "{}\n\nRecent model failures: {}",
        outcome.message.trim(),
        summary
    );
}

fn response_indicates_pending_execution(text: &str) -> bool {
    let Some(payload) = parse_response_status_payload(text) else {
        return false;
    };
    structured_status_from_response_payload(&payload).is_some_and(|status| {
        matches!(
            status.as_str(),
            "accepted" | "routing" | "model_selection" | "planning" | "tool_dispatch" | "synthesis"
        )
    })
}

fn response_indicates_permission_requirement(text: &str) -> bool {
    let Some(payload) = parse_response_status_payload(text) else {
        return false;
    };
    structured_status_from_response_payload(&payload)
        .is_some_and(|status| matches!(status.as_str(), "needs_permission" | "approval_required"))
}

fn response_contains_browser_handoff_reference(text: &str) -> bool {
    let trimmed = text.trim();
    !trimmed.is_empty() && trimmed.contains("/ui/browser-handoff/")
}

fn content_reports_browser_auto_session_started(content: &str) -> bool {
    let Ok(payload) = serde_json::from_str::<serde_json::Value>(content.trim()) else {
        return false;
    };
    payload.get("status").and_then(|value| value.as_str()) == Some("session_started")
        && payload
            .get("session_id")
            .and_then(|value| value.as_str())
            .is_some_and(|value| !value.trim().is_empty())
}

fn tool_result_is_browser_handoff(name: &str, content: &str) -> bool {
    response_contains_browser_handoff_reference(content)
        || (name.eq_ignore_ascii_case("browser_auto")
            && content_reports_browser_auto_session_started(content))
}

fn response_indicates_credentials_requirement(text: &str) -> bool {
    let trimmed = text.trim();
    if trimmed.is_empty() {
        return false;
    }
    if response_contains_browser_handoff_reference(text) {
        return false;
    }
    if let Some(payload) = parse_workflow_missing_inputs_marker(trimmed) {
        return !payload.sensitive_missing.is_empty();
    }
    let Some(payload) = parse_response_status_payload(trimmed) else {
        return false;
    };
    if structured_status_from_response_payload(&payload)
        .is_some_and(|status| status == "needs_credentials")
    {
        return true;
    }
    if payload
        .get("required_secrets")
        .and_then(|value| value.as_array())
        .is_some_and(|items| {
            items
                .iter()
                .any(|item| item.as_str().is_some_and(|value| !value.trim().is_empty()))
        })
    {
        return true;
    }
    false
}

fn response_indicates_integration_requirement(text: &str) -> bool {
    let Some(payload) = parse_response_status_payload(text) else {
        return false;
    };
    structured_status_from_response_payload(&payload)
        .is_some_and(|status| matches!(status.as_str(), "needs_integration"))
        || payload
            .get("required_integrations")
            .and_then(|value| value.as_array())
            .is_some_and(|items| items.iter().any(structured_value_has_visible_content))
}

fn parse_response_status_payload(text: &str) -> Option<serde_json::Value> {
    let trimmed = text.trim();
    if trimmed.is_empty() || response_contains_browser_handoff_reference(trimmed) {
        return None;
    }
    serde_json::from_str::<serde_json::Value>(trimmed).ok()
}

fn structured_status_from_response_payload(payload: &serde_json::Value) -> Option<String> {
    payload
        .get("status")
        .and_then(|value| value.as_str())
        .or_else(|| {
            payload
                .pointer("/user_outcome/request_state")
                .and_then(|value| value.as_str())
        })
        .map(normalize_status_token)
        .filter(|value| !value.is_empty())
}

fn normalize_status_token(value: &str) -> String {
    let mut out = String::new();
    let mut last_separator = false;
    for ch in value.trim().chars() {
        if ch.is_ascii_alphanumeric() {
            out.push(ch.to_ascii_lowercase());
            last_separator = false;
        } else if !last_separator && !out.is_empty() {
            out.push('_');
            last_separator = true;
        }
    }
    while out.ends_with('_') {
        out.pop();
    }
    out
}

fn structured_value_has_visible_content(value: &serde_json::Value) -> bool {
    match value {
        serde_json::Value::Null => false,
        serde_json::Value::Bool(_) | serde_json::Value::Number(_) => true,
        serde_json::Value::String(value) => !value.trim().is_empty(),
        serde_json::Value::Array(items) => items.iter().any(structured_value_has_visible_content),
        serde_json::Value::Object(object) => {
            object.values().any(structured_value_has_visible_content)
        }
    }
}

fn tool_batch_indicates_permission_requirement(batch: &tool_execution::ToolExecutionBatch) -> bool {
    batch.outcomes.iter().any(|outcome| {
        matches!(
            outcome.status,
            crate::core::ToolOutcomeStatus::Blocked | crate::core::ToolOutcomeStatus::NeedsInput
        ) && response_indicates_permission_requirement(&outcome.content)
    }) || batch
        .outputs
        .iter()
        .any(|output| response_indicates_permission_requirement(&output.content))
}

fn tool_batch_indicates_credentials_requirement(
    batch: &tool_execution::ToolExecutionBatch,
) -> bool {
    batch.outcomes.iter().any(|outcome| {
        if tool_result_is_browser_handoff(&outcome.name, &outcome.content) {
            return false;
        }
        matches!(outcome.status, crate::core::ToolOutcomeStatus::NeedsInput)
            && response_indicates_credentials_requirement(&outcome.content)
    }) || batch.outputs.iter().any(|output| {
        !tool_result_is_browser_handoff(&output.name, &output.content)
            && response_indicates_credentials_requirement(&output.content)
    })
}

fn tool_batch_indicates_integration_requirement(
    batch: &tool_execution::ToolExecutionBatch,
) -> bool {
    batch.outcomes.iter().any(|outcome| {
        matches!(
            outcome.status,
            crate::core::ToolOutcomeStatus::NeedsInput
                | crate::core::ToolOutcomeStatus::RecoverableError
                | crate::core::ToolOutcomeStatus::Blocked
        ) && response_indicates_integration_requirement(&outcome.content)
    }) || batch
        .outputs
        .iter()
        .any(|output| response_indicates_integration_requirement(&output.content))
}

fn tool_batch_has_successful_persistent_artifact(
    batch: &tool_execution::ToolExecutionBatch,
) -> bool {
    batch.outputs.iter().enumerate().any(|(index, output)| {
        if !tool_batch_output_succeeded(batch, index) {
            return false;
        }
        let inferred_action = crate::actions::ActionDef {
            name: output.name.clone(),
            description: String::new(),
            version: "0".to_string(),
            input_schema: serde_json::json!({}),
            capabilities: Vec::new(),
            sandbox_mode: None,
            source: crate::actions::ActionSource::System,
            file_path: None,
            authorization: Default::default(),
        };
        let metadata = inferred_action.action_metadata();
        matches!(metadata.role, ActionRole::Orchestration)
            || matches!(metadata.side_effect_level, ActionSideEffectLevel::Write)
    })
}

/// Query complexity classification
#[derive(Debug, Clone, Copy, PartialEq, serde::Serialize, serde::Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum QueryComplexity {
    /// Simple query - direct response
    Simple,
    /// Medium complexity - direct response with normal routing/failover
    Medium,
    /// Complex multi-step task - use orchestra
    Complex,
}

/// Final response payload for a single processed message.
#[derive(Debug, Clone)]
pub struct ProcessedMessage {
    pub response: String,
    pub conversation_id: Option<String>,
    pub conversation_title: Option<String>,
    pub run_id: Option<String>,
    pub run_status: Option<String>,
    pub trace_id: Option<String>,
    pub input_tokens: i64,
    pub output_tokens: i64,
    pub total_tokens: i64,
    pub cached_prompt_tokens: i64,
    pub cache_creation_prompt_tokens: i64,
    pub choices: Vec<ClarificationChoice>,
    pub degradation: Vec<crate::core::DegradationNote>,
    pub attempted_models: Vec<crate::core::ModelAttemptRecord>,
    pub user_outcome: Option<crate::core::UserFacingOutcome>,
    trace_steps: Vec<crate::core::ExecutionStep>,
    turn_records: Vec<AgentTurnRecord>,
    turn_plan: Option<crate::core::ExecutionPlan>,
}

impl ProcessedMessage {
    /// Total provider/LLM latency for this run in milliseconds: the sum of each
    /// model turn's call latency. Model-call steps carry their latency in
    /// `duration_ms`; every other step contributes 0, so this is pure model wait
    /// time (excludes tool execution and framework overhead).
    pub fn model_latency_ms(&self) -> u64 {
        self.trace_steps
            .iter()
            .filter_map(|step| step.duration_ms)
            .filter(|ms| *ms > 0)
            .sum()
    }
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub(crate) struct AgentTurnRecord {
    pub goal_id: String,
    pub outcome: AgentTurnOutcomeKind,
    pub action_name: Option<String>,
    pub side_effect: Option<String>,
    pub resolved_object_ref: Option<AgentResolvedRefSummary>,
    pub tool_output: Option<serde_json::Value>,
    pub reason: Option<String>,
    pub clarification_question: Option<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub(crate) struct AgentResolvedRefSummary {
    pub kind: String,
    pub id: String,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "snake_case")]
pub(crate) enum AgentTurnOutcomeKind {
    Succeeded,
    RespondedWithoutTool,
    NeedsClarification,
    Abandoned,
    Skipped,
}

#[derive(Clone)]
struct ImmediateExchangeContext<'a> {
    channel: &'a str,
    conversation_key: &'a str,
    is_new_conversation: bool,
    project_id: Option<&'a str>,
    model_used: &'a str,
    user_message_already_recorded: bool,
    recorded_user_message_id: Option<String>,
    memory_capture_allowed: bool,
    memory_capture_triage_allowed: bool,
    memory_capture_signal:
        Option<&'a crate::security::intent_classifier::InboundMemoryCaptureSignal>,
    memory_capture_source: Option<&'a str>,
    user_message_for_link_capture: Option<&'a str>,
}

#[derive(Clone)]
struct LlmAttemptCandidate {
    slot_id: String,
    slot_label: String,
    role: ModelRole,
    client: LlmClient,
}

fn effective_model_role_for_selection(
    config: &AgentConfig,
    preferred_role: &ModelRole,
) -> ModelRole {
    if config.model_pool.smart_routing {
        preferred_role.clone()
    } else {
        ModelRole::Primary
    }
}

fn ordered_model_slot_ids_for_role(
    config: &AgentConfig,
    primary_model_id: &str,
    user_selected_slot_id: Option<&str>,
    preferred_role: &ModelRole,
) -> Vec<String> {
    let effective_role = effective_model_role_for_selection(config, preferred_role);
    let mut ordered = Vec::new();
    let mut seen = HashSet::new();
    let mut push_slot_id = |slot_id: &str| {
        let normalized = slot_id.trim();
        if normalized.is_empty() {
            return;
        }
        if seen.insert(normalized.to_string()) {
            ordered.push(normalized.to_string());
        }
    };

    if let Some(slot_id) = user_selected_slot_id {
        if config
            .model_pool
            .slots
            .iter()
            .any(|slot| slot.id == slot_id)
        {
            push_slot_id(slot_id);
        }
    }

    if effective_role == ModelRole::Primary {
        if config
            .model_pool
            .slots
            .iter()
            .any(|slot| slot.id == primary_model_id)
        {
            push_slot_id(primary_model_id);
        }
        for slot in &config.model_pool.slots {
            if slot.role == ModelRole::Primary {
                push_slot_id(&slot.id);
            }
        }
    } else {
        for slot in &config.model_pool.slots {
            if slot.role == effective_role {
                push_slot_id(&slot.id);
            }
        }
        if config
            .model_pool
            .slots
            .iter()
            .any(|slot| slot.id == primary_model_id)
        {
            push_slot_id(primary_model_id);
        }
    }

    for slot in &config.model_pool.slots {
        if slot.role == ModelRole::Fallback {
            push_slot_id(&slot.id);
        }
    }

    for slot in &config.model_pool.slots {
        push_slot_id(&slot.id);
    }

    ordered
}

fn model_slot_label(slot: &ModelSlot) -> String {
    if slot.label.trim().is_empty() {
        format!("{} slot", Agent::model_role_label(&slot.role))
    } else {
        slot.label.clone()
    }
}

fn llm_attempt_candidates_for_role(
    config: &AgentConfig,
    model_pool: &HashMap<String, (ModelSlot, LlmClient)>,
    primary_model_id: &str,
    user_selected_slot_id: Option<&str>,
    legacy_llm: &LlmClient,
    preferred_role: &ModelRole,
) -> Vec<LlmAttemptCandidate> {
    let mut out = Vec::new();
    for slot_id in ordered_model_slot_ids_for_role(
        config,
        primary_model_id,
        user_selected_slot_id,
        preferred_role,
    ) {
        let Some((slot, client)) = model_pool.get(&slot_id) else {
            continue;
        };
        if !slot.enabled || !Agent::provider_has_runtime_credentials(&slot.provider) {
            continue;
        }
        out.push(LlmAttemptCandidate {
            slot_id: slot.id.clone(),
            slot_label: model_slot_label(slot),
            role: slot.role.clone(),
            client: client.clone(),
        });
    }

    if out.is_empty() {
        out.push(LlmAttemptCandidate {
            slot_id: "legacy".to_string(),
            slot_label: "Legacy Primary".to_string(),
            role: ModelRole::Primary,
            client: legacy_llm.clone(),
        });
    }

    out
}

/// The main Agent struct - orchestrates all subsystems
#[derive(Clone)]
pub struct Agent {
    /// Unique agent ID within the swarm
    pub _agent_id: AgentId,

    /// Persistent storage
    pub storage: Storage,

    /// Encrypted storage for sensitive data (facts, messages, user profile)
    pub encrypted_storage: crate::storage::encrypted::EncryptedStorage,

    /// Decentralized identity manager
    pub identity: IdentityManager,

    /// Safety policy engine
    pub safety: Arc<SafetyEngine>,

    /// Execution proof generator
    #[allow(dead_code)]
    pub proofs: Arc<ProofEngine>,

    /// Action runtime (WASM + Docker sandbox)
    pub runtime: Arc<ActionRuntime>,

    /// MCP registry (external servers/tools)
    pub mcp: Arc<RwLock<crate::mcp::registry::McpRegistry>>,

    /// Plugin registry (third-party HTTP extensions)
    pub plugins: Arc<RwLock<crate::plugins::registry::PluginRegistry>>,

    /// Generic pack registry for integrations, messaging channels, and future user-installed packs
    pub extension_packs: Arc<RwLock<crate::extension_packs::ExtensionPackRegistry>>,

    /// Legacy LLM client (primary model, kept for backward compatibility)
    pub llm: LlmClient,
    /// Optional embedding client used for dense memory and document retrieval.
    pub embedding_client: Option<Arc<EmbeddingClient>>,

    /// Model pool - keyed by slot ID, value is (ModelSlot, LlmClient)
    pub model_pool: std::collections::HashMap<String, (ModelSlot, LlmClient)>,

    /// Centralized resilience/runtime supervisor for model attempts and user outcomes.
    pub execution_supervisor: super::ExecutionSupervisor,

    /// Convenience: ID of the primary model slot
    pub primary_model_id: String,

    /// Task queue for autonomous execution
    pub tasks: Arc<RwLock<TaskQueue>>,

    /// Durable operator-facing container for ongoing work that spans tasks/watchers.
    pub background_sessions: background_session::BackgroundSessionManager,

    /// ArkOrbit per-user canvas service.
    pub arkorbit: super::arkorbit::ArkOrbitService,

    /// Configuration
    pub config: AgentConfig,

    /// Config directory path
    pub config_dir: PathBuf,

    /// Data directory path (persistent storage, outputs, etc.)
    pub data_dir: PathBuf,

    /// Orchestra for sub-agent delegation
    pub _orchestra: Orchestra,

    /// Agent swarm manager for multi-agent coordination
    pub swarm: Option<SwarmManager>,

    /// Task-driven auto-spawn router
    pub task_router: task_router::TaskRouter,

    /// Live swarm/delegation activity shared across chat and the Agents view.
    pub swarm_activity: Arc<crate::core::swarm::SwarmActivityTracker>,

    /// Security guard for prompt injection/leakage protection
    pub security: SecurityGuard,

    /// Conversation history per channel, trimmed by the active model's token budget.
    pub conversation_history:
        Arc<RwLock<std::collections::HashMap<String, Vec<ConversationMessage>>>>,

    /// Canonical readiness state for AgentArk-managed capabilities.
    pub capability_readiness: Arc<RwLock<capability_readiness::CapabilityReadinessRegistry>>,

    /// Readiness invalidation stream for mid-turn and UI consumers.
    capability_readiness_events: broadcast::Sender<capability_readiness::CapabilityReadinessEvent>,

    /// Multi-turn chat flow state for integration onboarding ("connect <integration> ...").
    integration_connect_flows: Arc<
        RwLock<HashMap<String, crate::core::connectivity::connect_flow::PendingIntegrationConnect>>,
    >,

    /// Pending skill import approvals keyed by conversation.
    pending_skill_imports: Arc<RwLock<HashMap<String, PendingSkillImport>>>,

    /// Pending secret-gated follow-ups keyed by conversation.
    pending_secret_followups: Arc<RwLock<HashMap<String, PendingSecretFollowup>>>,

    /// Pending secure credential prompts keyed by conversation.
    pending_chat_credential_prompts: Arc<RwLock<HashMap<String, PendingChatCredentialPrompt>>>,

    /// User profile (name, location, preferences) learned during onboarding
    pub user_profile: Arc<RwLock<UserProfile>>,

    /// Last execution trace - shows what the agent actually did
    pub last_trace: Arc<RwLock<ExecutionTrace>>,

    /// Trace history - stores last 100 execution traces
    pub trace_history: Arc<RwLock<Vec<ExecutionTrace>>>,

    /// External service integrations (Calendar, WhatsApp, etc.)
    pub integrations: Arc<crate::integrations::IntegrationManager>,

    /// Extension hook manager for pre/post processing hooks
    pub hooks: crate::hooks::HookManager,

    /// Last conversation ID used (for exposing to HTTP response)
    pub last_conversation_id: Arc<RwLock<Option<String>>>,

    /// Auto-generated conversation title (set after first message in new conversation)
    pub last_conversation_title: Arc<RwLock<Option<String>>>,

    /// HTTP API key for authentication (loaded from encrypted secrets)
    pub api_key: Option<String>,

    /// Background watcher manager for poll-until-condition workflows
    pub watcher_manager: watcher::WatcherManager,

    /// Browser session manager for LLM-driven browser automation
    pub browser_sessions: browser_session::BrowserSessionManager,

    /// Last user activity timestamp (for idle detection by sentinel cleanup)
    pub last_activity: Arc<RwLock<Option<chrono::DateTime<chrono::Utc>>>>,

    /// Foreground message requests currently being processed.
    active_message_requests: Arc<AtomicUsize>,

    /// Security event counters (reset each pulse cycle)
    pub security_events: Arc<SecurityEvents>,

    /// Optional user-selected model slot override (set via `/usemodel <name>`).
    pub user_selected_model_slot_id: Arc<std::sync::RwLock<Option<String>>>,

    /// Broadcast channel for live notification events consumed by the UI SSE stream.
    notification_events: broadcast::Sender<NotificationEvent>,

    /// Shared live run journal for replayable execution output and persisted checkpoints.
    live_runs: Arc<crate::core::LiveRunRegistry>,

    /// Non-fatal startup degradations that should be surfaced through health/readiness.
    startup_issues: Arc<RwLock<Vec<StartupIssue>>>,

    /// Deployed app registry (static files + dynamic server processes)
    pub app_registry: crate::actions::app::AppRegistry,
}

#[derive(Debug, Clone, serde::Serialize, serde::Deserialize)]
struct AutomationIntentAssessment {
    trigger_kind: String,
    delivery_policy: String,
    source_policy: String,
    fanout: bool,
    #[serde(default)]
    allowed_integration_classes: Vec<String>,
    #[serde(default)]
    avoid_integration_classes: Vec<String>,
    reasoning: String,
}

impl Default for AutomationIntentAssessment {
    fn default() -> Self {
        Self {
            trigger_kind: "unknown".to_string(),
            delivery_policy: "none".to_string(),
            source_policy: "none".to_string(),
            fanout: false,
            allowed_integration_classes: Vec::new(),
            avoid_integration_classes: Vec::new(),
            reasoning: "No automation intent assessment available.".to_string(),
        }
    }
}

#[derive(Debug, Clone, Copy)]
enum AutomationSurface {
    Watch,
    Schedule,
}

impl AutomationSurface {
    fn as_str(self) -> &'static str {
        match self {
            Self::Watch => "watch",
            Self::Schedule => "schedule",
        }
    }
}

#[derive(Debug, Clone)]
struct AutomationPlanValidationResult {
    action_name: String,
    action_arguments: serde_json::Value,
    delivery_channel: String,
    notes: Vec<String>,
    blocked_reason: Option<String>,
}

#[derive(Debug, Clone, Default, Serialize, Deserialize)]
pub struct ChatAttachmentHint {
    pub upload_id: String,
    pub kind: String,
    pub content_type: Option<String>,
    pub document_id: Option<String>,
}

#[derive(Debug, Clone, Default)]
pub struct RequestExecutionHints {
    pub turn_timing_id: Option<String>,
    pub caller_principal: Option<ActionCallerPrincipal>,
    pub execution_surface: ActionExecutionSurface,
    pub direct_user_intent: bool,
    pub recorded_user_message_id: Option<String>,
    pub memory_capture: crate::security::intent_classifier::InboundMemoryCaptureSignal,
    pub attachments_present: bool,
    pub attachments: Vec<ChatAttachmentHint>,
    pub execution_profile: Option<serde_json::Value>,
    pub arkorbit_context: Option<serde_json::Value>,
    pub browser_profile_context: Option<serde_json::Value>,
    pub client_timezone: Option<String>,
    pub client_timezone_offset_minutes: Option<i32>,
    pub recent_actionable_artifacts: Vec<serde_json::Value>,
}

enum InboundSecurityPrecheck {
    Continue {
        memory_capture: crate::security::intent_classifier::InboundMemoryCaptureSignal,
    },
    Respond(ProcessedMessage),
}

struct ActiveMessageRequestGuard {
    counter: Arc<AtomicUsize>,
}

impl Drop for ActiveMessageRequestGuard {
    fn drop(&mut self) {
        self.counter.fetch_sub(1, Ordering::AcqRel);
    }
}

#[derive(Debug, Clone, serde::Serialize, serde::Deserialize)]
pub(crate) struct ConversationLastDeployedApp {
    pub app_id: String,
    pub title: String,
    pub url: String,
    pub updated_at: String,
}

#[derive(Debug, Clone, serde::Serialize, serde::Deserialize)]
pub(crate) struct ConversationArtifactContext {
    pub artifact_type: String,
    pub artifact_id: String,
    pub title: String,
    #[serde(default)]
    pub summary: String,
    #[serde(default)]
    pub url: String,
    #[serde(default)]
    pub related_actions: Vec<String>,
    pub updated_at: String,
}

#[derive(Clone, Copy)]
struct ConversationArtifactSpec<'a> {
    artifact_type: &'a str,
    artifact_id: &'a str,
    title: &'a str,
    summary: &'a str,
    url: Option<&'a str>,
    related_actions: &'a [&'a str],
}

/// Buffered security counters between pulse cycles.
pub struct SecurityEvents {
    counts: std::sync::Mutex<SecuritySnapshot>,
}

impl SecurityEvents {
    pub fn new() -> Self {
        Self {
            counts: std::sync::Mutex::new(SecuritySnapshot::default()),
        }
    }

    fn lock_counts(&self) -> std::sync::MutexGuard<'_, SecuritySnapshot> {
        self.counts
            .lock()
            .unwrap_or_else(|error| error.into_inner())
    }

    pub fn record_injection_attempt(&self) {
        let mut counts = self.lock_counts();
        counts.injection_attempts = counts.injection_attempts.saturating_add(1);
    }

    pub fn record_auth_failure(&self) {
        let mut counts = self.lock_counts();
        counts.auth_failures = counts.auth_failures.saturating_add(1);
    }

    pub fn record_rate_limit_hit(&self) {
        let mut counts = self.lock_counts();
        counts.rate_limit_hits = counts.rate_limit_hits.saturating_add(1);
    }

    pub fn record_unauthorized_channel_attempt(&self) {
        let mut counts = self.lock_counts();
        counts.unauthorized_channel_attempts =
            counts.unauthorized_channel_attempts.saturating_add(1);
    }

    /// Snapshot current counters without consuming them.
    pub fn snapshot(&self) -> SecuritySnapshot {
        self.lock_counts().clone()
    }

    /// Commit a previously persisted snapshot by subtracting it from the live counters.
    pub fn commit_snapshot(&self, persisted: &SecuritySnapshot) {
        let mut counts = self.lock_counts();
        counts.injection_attempts = counts
            .injection_attempts
            .saturating_sub(persisted.injection_attempts);
        counts.auth_failures = counts.auth_failures.saturating_sub(persisted.auth_failures);
        counts.rate_limit_hits = counts
            .rate_limit_hits
            .saturating_sub(persisted.rate_limit_hits);
        counts.unauthorized_channel_attempts = counts
            .unauthorized_channel_attempts
            .saturating_sub(persisted.unauthorized_channel_attempts);
    }
}

#[derive(Debug, Clone, Default, serde::Serialize, serde::Deserialize)]
pub struct SecuritySnapshot {
    pub injection_attempts: u64,
    pub auth_failures: u64,
    pub rate_limit_hits: u64,
    pub unauthorized_channel_attempts: u64,
}

impl SecuritySnapshot {
    pub fn has_events(&self) -> bool {
        self.injection_attempts > 0
            || self.auth_failures > 0
            || self.rate_limit_hits > 0
            || self.unauthorized_channel_attempts > 0
    }
}

#[derive(Debug, Clone, serde::Serialize, serde::Deserialize)]
pub struct StartupIssue {
    pub subsystem: String,
    pub severity: String,
    pub summary: String,
    pub detail: String,
    pub recorded_at: String,
}

impl StartupIssue {
    fn new(
        subsystem: impl Into<String>,
        severity: impl Into<String>,
        summary: impl Into<String>,
        detail: impl Into<String>,
    ) -> Self {
        Self {
            subsystem: subsystem.into(),
            severity: severity.into(),
            summary: summary.into(),
            detail: detail.into(),
            recorded_at: chrono::Utc::now().to_rfc3339(),
        }
    }

    pub fn blocks_readiness(&self) -> bool {
        matches!(
            self.severity.trim().to_ascii_lowercase().as_str(),
            "critical" | "high" | "error"
        )
    }
}

/// User profile collected during onboarding
#[derive(Debug, Clone, Default, serde::Serialize, serde::Deserialize)]
pub struct UserProfile {
    pub name: Option<String>,
    pub location: Option<String>,
    pub timezone: Option<String>,
    pub language: Option<String>,
    pub tone: Option<String>,
    pub email_format: Option<String>,
    pub preferences: Option<String>,
    pub onboarding_complete: bool,
    #[serde(default)]
    pub personalization_dismissed: bool,
}

/// Execution trace step - records what the agent actually did
#[derive(Debug, Clone, serde::Serialize, serde::Deserialize)]
pub struct ExecutionStep {
    pub icon: String,
    pub title: String,
    pub detail: String,
    pub step_type: String, // info, success, thinking, warning
    pub data: Option<String>,
    pub timestamp: chrono::DateTime<chrono::Utc>,
    pub duration_ms: Option<u64>,
}

/// Full execution trace for a message
#[derive(Debug, Clone, Default, serde::Serialize, serde::Deserialize)]
pub struct ExecutionTrace {
    /// Unique ID for this trace
    pub id: String,
    pub message: String,
    pub channel: String,
    pub started_at: Option<chrono::DateTime<chrono::Utc>>,
    pub completed_at: Option<chrono::DateTime<chrono::Utc>>,
    pub steps: Vec<ExecutionStep>,
    pub proof_id: Option<String>,
    /// Response/result of the execution
    pub response: Option<String>,
    /// Model used for the primary LLM call
    pub model: Option<String>,
    /// Token usage (accumulated across all LLM calls in this trace)
    pub input_tokens: i64,
    pub output_tokens: i64,
    pub total_tokens: i64,
    #[serde(default)]
    pub cached_prompt_tokens: i64,
    #[serde(default)]
    pub cache_creation_prompt_tokens: i64,
    /// Estimated cost in USD
    pub cost_usd: f64,
    /// Routing complexity tier (simple/medium/complex)
    pub complexity: Option<String>,
    /// Structured execution plan (for complex tasks)
    #[serde(skip_serializing_if = "Option::is_none")]
    pub plan: Option<ExecutionPlan>,
}

impl Agent {
    /// Clear conversation history for a specific channel
    pub async fn clear_conversation_history(&self, channel: &str) {
        self.clear_conversation_for_project(channel, None).await;
    }

    pub async fn clear_conversation_for_project(&self, channel: &str, project_id: Option<&str>) {
        let scope = self.conversation_scope_mode().await;
        let conv_key = scope.conversation_key(channel, project_id);
        let active_id = self
            .storage
            .get(&conv_key)
            .await
            .ok()
            .flatten()
            .and_then(|bytes| String::from_utf8(bytes).ok())
            .filter(|id| !id.is_empty());

        {
            let mut history = self.conversation_history.write().await;
            if let Some(ref id) = active_id {
                history.remove(id);
            }
            history.remove(channel); // Legacy in-memory key
        }
        if let Some(ref id) = active_id {
            self.clear_pending_skill_import(id).await;
            self.clear_pending_secret_followup(id).await;
            self.clear_pending_integration_connect_flow(id).await;
            let _ = self.storage.delete_conversation(id).await;
            let digest_key = Self::conversation_digest_key(id);
            let _ = self.storage.delete(&digest_key).await;
            self.clear_pending_resilience_followup(id).await;
        }
        let _ = self.storage.set(&conv_key, b"").await;
    }

    /// Clear a specific conversation id for a channel/user context.
    pub async fn clear_conversation_by_id(
        &self,
        channel: &str,
        conversation_id: &str,
        project_id: Option<&str>,
    ) {
        let scope = self.conversation_scope_mode().await;
        let conv_key = scope.conversation_key(channel, project_id);

        {
            let mut history = self.conversation_history.write().await;
            history.remove(conversation_id);
        }
        self.clear_pending_skill_import(conversation_id).await;
        self.clear_pending_secret_followup(conversation_id).await;
        self.clear_pending_chat_credential_prompt(conversation_id)
            .await;
        self.clear_pending_integration_connect_flow(conversation_id)
            .await;
        self.clear_pending_resilience_followup(conversation_id)
            .await;
        let _ = self.storage.delete_conversation(conversation_id).await;
        let digest_key = Self::conversation_digest_key(conversation_id);
        let _ = self.storage.delete(&digest_key).await;

        let active_id = self
            .storage
            .get(&conv_key)
            .await
            .ok()
            .flatten()
            .and_then(|bytes| String::from_utf8(bytes).ok())
            .unwrap_or_default();
        if active_id == conversation_id {
            let _ = self.storage.set(&conv_key, b"").await;
        }
    }

    pub async fn subscribe_live_run(
        &self,
        run_id: &str,
        since_seq: Option<u64>,
    ) -> Option<(
        Vec<crate::core::RunEvent>,
        Option<tokio::sync::broadcast::Receiver<crate::core::RunEvent>>,
    )> {
        self.live_runs
            .subscribe(run_id, since_seq)
            .await
            .ok()
            .flatten()
    }

    pub fn live_run_registry(&self) -> Arc<crate::core::LiveRunRegistry> {
        self.live_runs.clone()
    }

    pub async fn load_persisted_run_events(&self, run_id: &str) -> Vec<crate::core::RunEvent> {
        self.live_runs
            .load_persisted_events(run_id)
            .await
            .unwrap_or_default()
    }

    /// Best-effort analytics: record LLM token usage for this response (if available).
    /// Also accumulates token counts on the current execution trace.
    pub(crate) async fn record_llm_usage(
        &self,
        channel: &str,
        purpose: &str,
        resp: &crate::core::model::llm::LlmResponse,
    ) {
        let Some(usage) = resp.usage.as_ref() else {
            return;
        };
        if usage.cached_prompt_tokens > 0 || usage.cache_creation_prompt_tokens > 0 {
            tracing::debug!(
                target: "agentark.llm_usage",
                provider = %resp.provider,
                model = %resp.model,
                channel = %channel,
                purpose = %purpose,
                prompt_tokens = usage.prompt_tokens,
                cached_prompt_tokens = usage.cached_prompt_tokens,
                cache_creation_prompt_tokens = usage.cache_creation_prompt_tokens,
                "LLM prompt cache usage"
            );
        }
        // Accumulate on current trace
        {
            let mut trace = self.last_trace.write().await;
            if trace.model.is_none() {
                trace.model = Some(resp.model.clone());
            }
            trace.input_tokens += usage.prompt_tokens as i64;
            trace.output_tokens += usage.completion_tokens as i64;
            trace.total_tokens += usage.total_tokens as i64;
            trace.cached_prompt_tokens = trace
                .cached_prompt_tokens
                .saturating_add(usage.cached_prompt_tokens.min(i64::MAX as u64) as i64);
            trace.cache_creation_prompt_tokens = trace
                .cache_creation_prompt_tokens
                .saturating_add(usage.cache_creation_prompt_tokens.min(i64::MAX as u64) as i64);
            trace.cost_usd += usage.cost_usd.unwrap_or_else(|| {
                estimate_cost_usd(
                    &resp.provider,
                    &resp.model,
                    usage.prompt_tokens,
                    usage.completion_tokens,
                )
            });
        }
        let model = crate::storage::entities::llm_usage::Model {
            id: uuid::Uuid::new_v4().to_string(),
            created_at: chrono::Utc::now().to_rfc3339(),
            provider: resp.provider.clone(),
            model: resp.model.clone(),
            channel: channel.to_string(),
            purpose: purpose.to_string(),
            prompt_tokens: usage.prompt_tokens.min(i32::MAX as u64) as i32,
            completion_tokens: usage.completion_tokens.min(i32::MAX as u64) as i32,
            total_tokens: usage.total_tokens.min(i32::MAX as u64) as i32,
            cached_prompt_tokens: usage.cached_prompt_tokens.min(i32::MAX as u64) as i32,
            cache_creation_prompt_tokens: usage.cache_creation_prompt_tokens.min(i32::MAX as u64)
                as i32,
            estimated: usage.estimated,
            cost_usd: usage.cost_usd,
        };
        if let Err(e) = self.storage.insert_llm_usage(&model).await {
            tracing::debug!("Failed to record llm_usage: {}", e);
        }
    }

    /// Search document chunks for RAG-style Q&A.
    pub async fn search_documents(
        &self,
        query: &str,
        limit: usize,
        project_id: Option<&str>,
    ) -> Result<Vec<DocumentSearchHit>> {
        document_search::search_documents(
            &self.storage,
            self.embedding_client.as_deref(),
            query,
            limit,
            project_id,
        )
        .await
    }

    /// Get agent status
    pub async fn status(&self) -> AgentStatus {
        let tasks = self.tasks.read().await;
        let pending_count = tasks
            .all()
            .iter()
            .filter(|t| {
                matches!(
                    t.status,
                    task::TaskStatus::Pending
                        | task::TaskStatus::AwaitingApproval
                        | task::TaskStatus::Paused
                )
            })
            .count();

        AgentStatus {
            did: self.identity.did().to_string(),
            memory_entries: self.storage.count_facts(None).await.unwrap_or(0) as usize,
            actions_loaded: self.runtime.action_count().await,
            tasks_pending: pending_count,
        }
    }
}

#[derive(Debug, Clone, serde::Serialize)]
pub struct AgentStatus {
    pub did: String,
    pub memory_entries: usize,
    pub actions_loaded: usize,
    pub tasks_pending: usize,
}

fn compute_next_run(
    cron_expr: &str,
    tz: Option<chrono_tz::Tz>,
) -> Option<chrono::DateTime<chrono::Utc>> {
    let schedule = cron_expr.parse::<cron::Schedule>().ok()?;
    match tz {
        Some(tz) => schedule
            .upcoming(tz)
            .next()
            .map(|dt| dt.with_timezone(&chrono::Utc)),
        None => schedule.upcoming(chrono::Utc).next(),
    }
}

fn should_preserve_cancelled_task_status(
    current: &task::TaskStatus,
    next: &task::TaskStatus,
) -> bool {
    matches!(current, task::TaskStatus::Cancelled) && !matches!(next, task::TaskStatus::Cancelled)
}
