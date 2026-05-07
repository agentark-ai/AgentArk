//! Action system with self-improvement capabilities
//! Based on arXiv:2512.17102 "SAGE: Self-Improving Agent with Action Library"

pub mod app;
pub mod arkorbit;
pub mod calendar;
pub mod gmail;
pub mod google_workspace;
pub mod lan;
pub mod research;
pub mod search;
#[cfg(feature = "ssh")]
pub mod ssh;
pub mod vercel;

use serde::{Deserialize, Serialize};
use std::{collections::BTreeMap, fmt, str::FromStr};

use crate::runtime::SandboxMode;

pub const ACTION_CATALOG_EMBEDDING_DIM: usize = 384;
#[allow(unused_imports)]
pub use gmail::{gmail_reply, gmail_scan};
#[allow(unused_imports)]
pub use research::{execute_research, ResearchArgs, ResearchClient, ResearchDepth, ResearchResult};
#[allow(unused_imports)]
pub use search::{SearchBackend, SearchClient, SearchConfig, SearchResponse, SearchResult};

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "snake_case")]
pub enum PlannerActionRole {
    Trigger,
    Delivery,
    DataSource,
    Mutation,
    Inspection,
    Orchestration,
    Unknown,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "snake_case")]
pub enum PlannerIntegrationClass {
    Internal,
    Messaging,
    Workspace,
    Search,
    Browser,
    Filesystem,
    App,
    Code,
    Network,
    Commerce,
    Analytics,
    Media,
    Unknown,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "snake_case")]
pub enum PlannerCostTier {
    Low,
    Medium,
    High,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "snake_case")]
pub enum PlannerSideEffectLevel {
    None,
    Notify,
    Write,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "snake_case")]
pub enum PlannerDeliveryMode {
    /// Returns a usable result in the current turn.
    Immediate,
    /// Queues work for later execution.
    Async,
    /// Creates a trigger/monitor that fires when a condition is met.
    Conditional,
    /// Delivery timing depends on arguments or external configuration.
    Either,
}

impl Default for PlannerDeliveryMode {
    fn default() -> Self {
        Self::Immediate
    }
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq, Default)]
#[serde(rename_all = "snake_case")]
pub enum ActionRiskLevel {
    #[default]
    None,
    Low,
    Medium,
    High,
    Critical,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq, Default)]
pub struct ActionRateLimit {
    pub max_calls: u32,
    pub window_seconds: u64,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq, Default)]
pub struct ActionHumanApproval {
    #[serde(default)]
    pub required: bool,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq, Default)]
pub struct ActionEgressPolicy {
    #[serde(default)]
    pub read_only: bool,
    #[serde(default)]
    pub outbound_write: bool,
    #[serde(default)]
    pub public_publish: bool,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq, Default)]
pub struct ActionChannelTarget {
    pub argument_key: String,
    pub default_target: String,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq, Default)]
pub struct ActionAccessMetadata {
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub permission_ids: Vec<String>,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub integration_ids: Vec<String>,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub extension_pack_ids: Vec<String>,
    #[serde(default, skip_serializing_if = "BTreeMap::is_empty")]
    pub integration_features: BTreeMap<String, Vec<String>>,
    #[serde(default)]
    pub requires_ssh_connection: bool,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub channel_targets: Vec<ActionChannelTarget>,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq, Default)]
pub struct ActionAuthorization {
    #[serde(default)]
    pub risk_level: ActionRiskLevel,
    /// Machine-readable access requirements used by agent access planning and enforcement.
    #[serde(default)]
    pub access: ActionAccessMetadata,
    #[serde(default)]
    pub requires_auth: bool,
    #[serde(default)]
    pub allowed_roles: Vec<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub rate_limit: Option<ActionRateLimit>,
    #[serde(default)]
    pub human_approval: ActionHumanApproval,
    #[serde(default)]
    pub outbound: ActionEgressPolicy,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct ActionCallerPrincipal {
    pub user_id: String,
    pub role: String,
    pub auth_source: String,
    #[serde(default)]
    pub trusted: bool,
}

impl ActionCallerPrincipal {
    pub fn local_admin(auth_source: &str) -> Self {
        Self {
            user_id: "local_user".to_string(),
            role: "admin".to_string(),
            auth_source: auth_source.trim().to_string(),
            trusted: true,
        }
    }
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq, Default)]
#[serde(rename_all = "snake_case")]
pub enum ActionExecutionSurface {
    Chat,
    Api,
    Automation,
    Background,
    Test,
    #[default]
    Internal,
}

#[derive(Debug, Clone, Copy, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "snake_case")]
pub enum ActionErrorDomain {
    Action,
    Auth,
    Integration,
    Channel,
    App,
    Search,
    Scheduler,
    Watcher,
}

impl ActionErrorDomain {
    pub fn as_key(self) -> &'static str {
        match self {
            Self::Action => "action",
            Self::Auth => "auth",
            Self::Integration => "integration",
            Self::Channel => "channel",
            Self::App => "app",
            Self::Search => "search",
            Self::Scheduler => "scheduler",
            Self::Watcher => "watcher",
        }
    }
}

impl fmt::Display for ActionErrorDomain {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(self.as_key())
    }
}

impl FromStr for ActionErrorDomain {
    type Err = ();

    fn from_str(value: &str) -> std::result::Result<Self, Self::Err> {
        match value.trim() {
            "action" => Ok(Self::Action),
            "auth" => Ok(Self::Auth),
            "integration" => Ok(Self::Integration),
            "channel" => Ok(Self::Channel),
            "app" => Ok(Self::App),
            "search" => Ok(Self::Search),
            "scheduler" => Ok(Self::Scheduler),
            "watcher" => Ok(Self::Watcher),
            _ => Err(()),
        }
    }
}

#[derive(Debug, Clone, Copy, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "snake_case")]
pub enum ActionErrorReason {
    MissingInput,
    InvalidInput,
    NotFound,
    NotConnected,
    Unavailable,
    PermissionDenied,
    Ambiguous,
    RateLimited,
    Timeout,
    Failed,
}

impl ActionErrorReason {
    pub fn as_key(self) -> &'static str {
        match self {
            Self::MissingInput => "missing_input",
            Self::InvalidInput => "invalid_input",
            Self::NotFound => "not_found",
            Self::NotConnected => "not_connected",
            Self::Unavailable => "unavailable",
            Self::PermissionDenied => "permission_denied",
            Self::Ambiguous => "ambiguous",
            Self::RateLimited => "rate_limited",
            Self::Timeout => "timeout",
            Self::Failed => "failed",
        }
    }
}

impl fmt::Display for ActionErrorReason {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(self.as_key())
    }
}

impl FromStr for ActionErrorReason {
    type Err = ();

    fn from_str(value: &str) -> std::result::Result<Self, Self::Err> {
        match value.trim() {
            "missing_input" => Ok(Self::MissingInput),
            "invalid_input" => Ok(Self::InvalidInput),
            "not_found" => Ok(Self::NotFound),
            "not_connected" => Ok(Self::NotConnected),
            "unavailable" => Ok(Self::Unavailable),
            "permission_denied" => Ok(Self::PermissionDenied),
            "ambiguous" => Ok(Self::Ambiguous),
            "rate_limited" => Ok(Self::RateLimited),
            "timeout" => Ok(Self::Timeout),
            "failed" => Ok(Self::Failed),
            _ => Err(()),
        }
    }
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq, thiserror::Error)]
#[serde(tag = "type", rename_all = "snake_case")]
pub enum ActionError {
    #[error("ERR/{domain}/{reason}: {message}")]
    Structured {
        domain: ActionErrorDomain,
        reason: ActionErrorReason,
        message: String,
    },
}

impl ActionError {
    pub fn new(
        domain: ActionErrorDomain,
        reason: ActionErrorReason,
        message: impl AsRef<str>,
    ) -> Self {
        let message = message.as_ref().trim();
        Self::Structured {
            domain,
            reason,
            message: if message.is_empty() {
                "Action failed".to_string()
            } else {
                message.to_string()
            },
        }
    }

    pub fn domain(&self) -> ActionErrorDomain {
        match self {
            Self::Structured { domain, .. } => *domain,
        }
    }

    pub fn reason(&self) -> ActionErrorReason {
        match self {
            Self::Structured { reason, .. } => *reason,
        }
    }

    pub fn message(&self) -> &str {
        match self {
            Self::Structured { message, .. } => message,
        }
    }

    pub fn code(&self) -> String {
        format!("{}_{}", self.domain().as_key(), self.reason().as_key())
    }

    pub fn err_prefix(&self) -> String {
        format!("ERR/{}/{}", self.domain().as_key(), self.reason().as_key())
    }

    pub fn into_anyhow(self) -> anyhow::Error {
        anyhow::Error::new(self)
    }
}

pub fn structured_action_error_text(
    domain: ActionErrorDomain,
    reason: ActionErrorReason,
    message: impl AsRef<str>,
) -> String {
    ActionError::new(domain, reason, message).to_string()
}

pub fn structured_action_error(
    domain: ActionErrorDomain,
    reason: ActionErrorReason,
    message: impl AsRef<str>,
) -> anyhow::Error {
    ActionError::new(domain, reason, message).into_anyhow()
}

pub fn action_error_domain_for_action_name(action_name: &str) -> ActionErrorDomain {
    let name = action_name.trim().to_ascii_lowercase();
    match name.as_str() {
        "notify_user" => ActionErrorDomain::Channel,
        "schedule_task" => ActionErrorDomain::Scheduler,
        "watch" => ActionErrorDomain::Watcher,
        "web_search" | "research" | "page_fetch" => ActionErrorDomain::Search,
        "app_deploy" | "app_restart" | "app_stop" | "app_delete" => ActionErrorDomain::App,
        "gmail_scan"
        | "gmail_reply"
        | "calendar_today"
        | "calendar_list"
        | "calendar_create"
        | "calendar_free"
        | "connector_request"
        | "extension_pack_connect"
        | "extension_pack_invoke" => ActionErrorDomain::Integration,
        _ => ActionErrorDomain::Action,
    }
}

pub fn structured_action_error_text_for_action(
    action_name: &str,
    reason: ActionErrorReason,
    message: impl AsRef<str>,
) -> String {
    structured_action_error_text(
        action_error_domain_for_action_name(action_name),
        reason,
        message,
    )
}

pub fn structured_action_error_for_action(
    action_name: &str,
    reason: ActionErrorReason,
    message: impl AsRef<str>,
) -> anyhow::Error {
    structured_action_error(
        action_error_domain_for_action_name(action_name),
        reason,
        message,
    )
}

pub fn is_structured_action_error_text(message: &str) -> bool {
    message.trim_start().starts_with("ERR/")
}

pub fn parse_structured_action_error_text(message: &str) -> Option<ActionError> {
    let trimmed = message.trim();
    let rest = trimmed.strip_prefix("ERR/")?;
    let (domain, rest) = rest.split_once('/')?;
    let (reason, message) = rest.split_once(':')?;
    Some(ActionError::new(
        ActionErrorDomain::from_str(domain).ok()?,
        ActionErrorReason::from_str(reason).ok()?,
        message,
    ))
}

pub fn ensure_structured_action_error_text(action_name: &str, message: impl AsRef<str>) -> String {
    let message = message.as_ref();
    if is_structured_action_error_text(message) {
        message.trim().to_string()
    } else {
        structured_action_error_text_for_action(action_name, ActionErrorReason::Failed, message)
    }
}

impl ActionExecutionSurface {
    pub fn as_key(&self) -> &'static str {
        match self {
            ActionExecutionSurface::Chat => "chat",
            ActionExecutionSurface::Api => "api",
            ActionExecutionSurface::Automation => "automation",
            ActionExecutionSurface::Background => "background",
            ActionExecutionSurface::Test => "test",
            ActionExecutionSurface::Internal => "internal",
        }
    }
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq, Default)]
pub struct ActionAuthorizationContext {
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub principal: Option<ActionCallerPrincipal>,
    #[serde(default)]
    pub surface: ActionExecutionSurface,
    #[serde(default)]
    pub direct_user_intent: bool,
    #[serde(default)]
    pub current_turn_is_explicit_approval: bool,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub agent_name: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub agent_access_scope: Option<crate::core::swarm::AgentAccessScope>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub capability_context_id: Option<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct ActionAuthorizationDecision {
    pub allowed: bool,
    pub requires_explicit_approval: bool,
    pub reason: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub matched_role: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub rate_limit_key: Option<String>,
}

impl ActionAuthorizationDecision {
    pub fn allow(reason: impl Into<String>) -> Self {
        Self {
            allowed: true,
            requires_explicit_approval: false,
            reason: reason.into(),
            matched_role: None,
            rate_limit_key: None,
        }
    }

    pub fn deny(reason: impl Into<String>) -> Self {
        Self {
            allowed: false,
            requires_explicit_approval: false,
            reason: reason.into(),
            matched_role: None,
            rate_limit_key: None,
        }
    }

    pub fn require_explicit_approval(reason: impl Into<String>) -> Self {
        Self {
            allowed: false,
            requires_explicit_approval: true,
            reason: reason.into(),
            matched_role: None,
            rate_limit_key: None,
        }
    }
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct ActionPlannerMetadata {
    pub role: PlannerActionRole,
    pub requires_auth: bool,
    pub integration_class: PlannerIntegrationClass,
    pub cost: PlannerCostTier,
    pub side_effect_level: PlannerSideEffectLevel,
    #[serde(default)]
    pub delivery_mode: PlannerDeliveryMode,
}

impl Default for ActionPlannerMetadata {
    fn default() -> Self {
        Self {
            role: PlannerActionRole::Unknown,
            requires_auth: false,
            integration_class: PlannerIntegrationClass::Unknown,
            cost: PlannerCostTier::Medium,
            side_effect_level: PlannerSideEffectLevel::None,
            delivery_mode: PlannerDeliveryMode::Immediate,
        }
    }
}

impl ActionDef {
    pub fn planner_metadata(&self) -> ActionPlannerMetadata {
        planner_metadata_for_action(self)
    }
}

fn infer_delivery_mode(capabilities: &std::collections::HashSet<String>) -> PlannerDeliveryMode {
    if capabilities.contains("scheduler") {
        return PlannerDeliveryMode::Async;
    }
    if capabilities.contains("watcher") {
        return PlannerDeliveryMode::Conditional;
    }
    PlannerDeliveryMode::Immediate
}

pub fn planner_metadata_for_action(action: &ActionDef) -> ActionPlannerMetadata {
    let name = action.name.trim().to_ascii_lowercase();
    let capabilities = action
        .capabilities
        .iter()
        .map(|value| value.trim().to_ascii_lowercase())
        .collect::<std::collections::HashSet<_>>();

    let mut meta = ActionPlannerMetadata::default();
    meta.requires_auth = action.authorization.requires_auth;
    meta.delivery_mode = infer_delivery_mode(&capabilities);

    match name.as_str() {
        "current_time" => {
            meta.role = PlannerActionRole::Trigger;
            meta.integration_class = PlannerIntegrationClass::Internal;
            meta.cost = PlannerCostTier::Low;
            return meta;
        }
        "notify_user" => {
            meta.role = PlannerActionRole::Delivery;
            meta.integration_class = PlannerIntegrationClass::Internal;
            meta.cost = PlannerCostTier::Low;
            meta.side_effect_level = PlannerSideEffectLevel::Notify;
            return meta;
        }
        "watch" | "schedule_task" | "delegate" => {
            meta.role = PlannerActionRole::Orchestration;
            meta.integration_class = PlannerIntegrationClass::Internal;
            meta.cost = PlannerCostTier::Low;
            meta.side_effect_level = PlannerSideEffectLevel::Write;
            return meta;
        }
        "gmail_scan" => {
            meta.role = PlannerActionRole::DataSource;
            meta.requires_auth = true;
            meta.integration_class = PlannerIntegrationClass::Workspace;
            return meta;
        }
        "gmail_reply" => {
            meta.role = PlannerActionRole::Delivery;
            meta.requires_auth = true;
            meta.integration_class = PlannerIntegrationClass::Workspace;
            meta.side_effect_level = PlannerSideEffectLevel::Notify;
            return meta;
        }
        "calendar_today" | "calendar_list" | "calendar_free" => {
            meta.role = PlannerActionRole::DataSource;
            meta.requires_auth = true;
            meta.integration_class = PlannerIntegrationClass::Workspace;
            return meta;
        }
        "calendar_create" => {
            meta.role = PlannerActionRole::Mutation;
            meta.requires_auth = true;
            meta.integration_class = PlannerIntegrationClass::Workspace;
            meta.side_effect_level = PlannerSideEffectLevel::Write;
            return meta;
        }
        "web_search" | "research" | "page_fetch" => {
            meta.role = PlannerActionRole::DataSource;
            meta.integration_class = PlannerIntegrationClass::Search;
            meta.cost = if name == "research" {
                PlannerCostTier::High
            } else {
                PlannerCostTier::Medium
            };
            return meta;
        }
        "browser_auto" => {
            meta.role = PlannerActionRole::Mutation;
            meta.integration_class = PlannerIntegrationClass::Browser;
            meta.cost = PlannerCostTier::High;
            meta.side_effect_level = PlannerSideEffectLevel::Write;
            return meta;
        }
        "home_assistant" => {
            meta.role = PlannerActionRole::DataSource;
            meta.requires_auth = true;
            meta.integration_class = PlannerIntegrationClass::Network;
            meta.cost = PlannerCostTier::Low;
            return meta;
        }
        "home_assistant_call_service" => {
            meta.role = PlannerActionRole::Mutation;
            meta.requires_auth = true;
            meta.integration_class = PlannerIntegrationClass::Network;
            meta.cost = PlannerCostTier::Medium;
            meta.side_effect_level = PlannerSideEffectLevel::Write;
            return meta;
        }
        "capability_resolve" => {
            meta.role = PlannerActionRole::Inspection;
            meta.integration_class = PlannerIntegrationClass::Internal;
            meta.cost = PlannerCostTier::Low;
            return meta;
        }
        "capability_acquire" | "manage_actions" => {
            meta.role = PlannerActionRole::Mutation;
            meta.integration_class = PlannerIntegrationClass::Internal;
            meta.cost = PlannerCostTier::Medium;
            meta.side_effect_level = PlannerSideEffectLevel::Write;
            return meta;
        }
        "file_read"
        | "list_tasks"
        | "list_watchers"
        | "list_integrations"
        | "ark_inspect"
        | "postgres_schema_inspect"
        | "postgres_query_readonly" => {
            meta.role = PlannerActionRole::Inspection;
            meta.integration_class = if name == "file_read" {
                PlannerIntegrationClass::Filesystem
            } else if name == "postgres_schema_inspect" || name == "postgres_query_readonly" {
                PlannerIntegrationClass::Analytics
            } else {
                PlannerIntegrationClass::Internal
            };
            meta.cost = PlannerCostTier::Low;
            return meta;
        }
        "file_write" => {
            meta.role = PlannerActionRole::Mutation;
            meta.integration_class = PlannerIntegrationClass::Filesystem;
            meta.cost = PlannerCostTier::Low;
            meta.side_effect_level = PlannerSideEffectLevel::Write;
            return meta;
        }
        "app_deploy" | "app_restart" | "app_stop" | "app_delete" => {
            meta.role = PlannerActionRole::Mutation;
            meta.integration_class = PlannerIntegrationClass::App;
            meta.cost = PlannerCostTier::Medium;
            meta.side_effect_level = PlannerSideEffectLevel::Write;
            return meta;
        }
        "shell" | "code_execute" => {
            meta.role = PlannerActionRole::Mutation;
            meta.integration_class = PlannerIntegrationClass::Code;
            meta.cost = PlannerCostTier::High;
            meta.side_effect_level = PlannerSideEffectLevel::Write;
            return meta;
        }
        _ => {}
    }

    if capabilities.contains("watcher")
        || capabilities.contains("scheduler")
        || capabilities.contains("orchestration")
    {
        meta.role = PlannerActionRole::Orchestration;
        meta.integration_class = PlannerIntegrationClass::Internal;
        meta.cost = PlannerCostTier::Low;
        meta.side_effect_level = PlannerSideEffectLevel::Write;
        return meta;
    }

    if capabilities.contains("capability_inventory")
        || capabilities.contains("watcher_inventory")
        || capabilities.contains("integration_inventory")
        || capabilities.contains("app_registry")
        || capabilities.contains("app_inventory")
        || capabilities.contains("platform_observability")
        || capabilities.contains("database_readonly")
        || capabilities.contains("session_history")
        || capabilities.contains("memory")
        || capabilities.contains("documents")
    {
        meta.role = PlannerActionRole::Inspection;
        meta.integration_class = if capabilities.contains("database_readonly") {
            PlannerIntegrationClass::Analytics
        } else {
            PlannerIntegrationClass::Internal
        };
        meta.cost = PlannerCostTier::Low;
        return meta;
    }

    if capabilities.contains("skill_management")
        || capabilities.contains("integration_builder")
        || capabilities.contains("integration_admin")
    {
        meta.role = PlannerActionRole::Mutation;
        meta.integration_class = PlannerIntegrationClass::Internal;
        meta.cost = PlannerCostTier::Medium;
        meta.side_effect_level = PlannerSideEffectLevel::Write;
        return meta;
    }

    if capabilities.contains("notify") {
        meta.role = PlannerActionRole::Delivery;
        meta.integration_class = PlannerIntegrationClass::Internal;
        meta.side_effect_level = PlannerSideEffectLevel::Notify;
        return meta;
    }

    if capabilities.contains("gmail") || capabilities.contains("google_workspace") {
        meta.role = PlannerActionRole::DataSource;
        meta.integration_class = PlannerIntegrationClass::Workspace;
        return meta;
    }

    if capabilities.contains("search") || capabilities.contains("research") {
        meta.role = PlannerActionRole::DataSource;
        meta.integration_class = PlannerIntegrationClass::Search;
        return meta;
    }

    if capabilities.contains("vision_ocr")
        || capabilities.contains("image_generation")
        || capabilities.contains("pdf_generation")
        || capabilities.contains("document_generation")
    {
        meta.role = PlannerActionRole::DataSource;
        meta.integration_class = PlannerIntegrationClass::Media;
        meta.cost = PlannerCostTier::Medium;
        if capabilities.contains("pdf_generation") || capabilities.contains("document_generation") {
            meta.role = PlannerActionRole::Mutation;
            meta.side_effect_level = PlannerSideEffectLevel::Write;
        }
        return meta;
    }

    if capabilities.contains("browser") {
        meta.role = PlannerActionRole::Mutation;
        meta.integration_class = PlannerIntegrationClass::Browser;
        meta.cost = PlannerCostTier::High;
        meta.side_effect_level = PlannerSideEffectLevel::Write;
        return meta;
    }

    if capabilities.contains("app_hosting") {
        meta.role = PlannerActionRole::Mutation;
        meta.integration_class = PlannerIntegrationClass::App;
        meta.cost = PlannerCostTier::Medium;
        meta.side_effect_level = PlannerSideEffectLevel::Write;
        return meta;
    }

    if capabilities.contains("file_read") {
        meta.role = PlannerActionRole::Inspection;
        meta.integration_class = PlannerIntegrationClass::Filesystem;
        meta.cost = PlannerCostTier::Low;
        return meta;
    }

    if capabilities.contains("file_write") {
        meta.role = PlannerActionRole::Mutation;
        meta.integration_class = PlannerIntegrationClass::Filesystem;
        meta.cost = PlannerCostTier::Low;
        meta.side_effect_level = PlannerSideEffectLevel::Write;
        return meta;
    }

    if capabilities.contains("shell")
        || capabilities.contains("code_execute")
        || capabilities.contains("local_cli")
    {
        meta.role = PlannerActionRole::Mutation;
        meta.integration_class = PlannerIntegrationClass::Code;
        meta.cost = PlannerCostTier::High;
        meta.side_effect_level = PlannerSideEffectLevel::Write;
        return meta;
    }

    if capabilities.contains("ssh") {
        meta.role = PlannerActionRole::Mutation;
        meta.integration_class = PlannerIntegrationClass::Network;
        meta.cost = PlannerCostTier::High;
        meta.side_effect_level = PlannerSideEffectLevel::Write;
        return meta;
    }

    if capabilities.contains("local_network_discovery") {
        meta.role = PlannerActionRole::DataSource;
        meta.integration_class = PlannerIntegrationClass::Network;
        meta.cost = PlannerCostTier::Medium;
        return meta;
    }

    if capabilities.contains("network") {
        meta.role = PlannerActionRole::DataSource;
        meta.integration_class = PlannerIntegrationClass::Network;
        meta.cost = PlannerCostTier::Medium;
        return meta;
    }

    if capabilities.contains("analytics") {
        meta.role = PlannerActionRole::DataSource;
        meta.integration_class = PlannerIntegrationClass::Analytics;
        return meta;
    }

    if capabilities.contains("image_generation") || capabilities.contains("video_generation") {
        meta.role = PlannerActionRole::Mutation;
        meta.integration_class = PlannerIntegrationClass::Media;
        meta.cost = PlannerCostTier::High;
        meta.side_effect_level = PlannerSideEffectLevel::Write;
        return meta;
    }

    meta
}

#[cfg(test)]
pub fn action_requires_nontrivial_direct_execution(action: &ActionDef) -> bool {
    let metadata = planner_metadata_for_action(action);
    matches!(metadata.role, PlannerActionRole::Orchestration)
        || matches!(
            metadata.integration_class,
            PlannerIntegrationClass::App
                | PlannerIntegrationClass::Code
                | PlannerIntegrationClass::Browser
                | PlannerIntegrationClass::Network
                | PlannerIntegrationClass::Commerce
                | PlannerIntegrationClass::Media
        )
}

/// Action source type
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub enum ActionSource {
    /// Built-in system action (not editable)
    System,
    /// Bundled workflow action (editable)
    Bundled,
    /// User-created custom action (editable)
    Custom,
}

/// Information about an action
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ActionDef {
    /// Action name (unique identifier)
    pub name: String,

    /// Human-readable description
    pub description: String,

    /// Action version
    pub version: String,

    /// JSON Schema for input parameters
    pub input_schema: serde_json::Value,

    /// Required capabilities
    pub capabilities: Vec<String>,

    /// Preferred sandbox mode
    pub sandbox_mode: Option<SandboxMode>,

    /// Action source (system, bundled, or custom)
    #[serde(default = "default_action_source")]
    pub source: ActionSource,

    /// Path to action file (for editable actions)
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub file_path: Option<String>,

    /// Normalized authorization metadata used by the runtime permission layer.
    #[serde(default)]
    pub authorization: ActionAuthorization,
}

impl Default for ActionDef {
    fn default() -> Self {
        Self {
            name: String::new(),
            description: String::new(),
            version: "1.0.0".to_string(),
            input_schema: serde_json::json!({}),
            capabilities: vec![],
            sandbox_mode: None,
            source: ActionSource::System,
            file_path: None,
            authorization: ActionAuthorization::default(),
        }
    }
}

fn default_action_source() -> ActionSource {
    ActionSource::System
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn planner_metadata_does_not_infer_delivery_from_name_alone() {
        let action = ActionDef {
            name: "telegram_bridge".to_string(),
            description: "Internal helper with no messaging semantics".to_string(),
            ..ActionDef::default()
        };

        let metadata = planner_metadata_for_action(&action);
        assert_eq!(metadata.role, PlannerActionRole::Unknown);
        assert_eq!(metadata.integration_class, PlannerIntegrationClass::Unknown);
    }

    #[test]
    fn planner_metadata_uses_exact_workspace_capability() {
        let action = ActionDef {
            name: "workspace_reader".to_string(),
            capabilities: vec!["google_workspace".to_string()],
            authorization: ActionAuthorization {
                requires_auth: true,
                ..ActionAuthorization::default()
            },
            ..ActionDef::default()
        };

        let metadata = planner_metadata_for_action(&action);
        assert_eq!(metadata.role, PlannerActionRole::DataSource);
        assert_eq!(
            metadata.integration_class,
            PlannerIntegrationClass::Workspace
        );
        assert!(metadata.requires_auth);
    }

    #[test]
    fn orchestration_actions_require_nontrivial_direct_execution() {
        let action = ActionDef {
            name: "watch".to_string(),
            capabilities: vec!["watcher".to_string()],
            ..ActionDef::default()
        };

        assert!(action_requires_nontrivial_direct_execution(&action));
    }

    #[test]
    fn planner_delivery_mode_comes_from_capabilities() {
        let scheduled = ActionDef {
            capabilities: vec!["scheduler".to_string()],
            ..ActionDef::default()
        };
        let watcher = ActionDef {
            capabilities: vec!["watcher".to_string()],
            ..ActionDef::default()
        };
        let immediate = ActionDef {
            capabilities: vec!["file_write".to_string()],
            ..ActionDef::default()
        };

        assert_eq!(
            planner_metadata_for_action(&scheduled).delivery_mode,
            PlannerDeliveryMode::Async
        );
        assert_eq!(
            planner_metadata_for_action(&watcher).delivery_mode,
            PlannerDeliveryMode::Conditional
        );
        assert_eq!(
            planner_metadata_for_action(&immediate).delivery_mode,
            PlannerDeliveryMode::Immediate
        );
    }

    #[test]
    fn pdf_generation_keeps_write_effect_without_filesystem_routing_class() {
        let action = ActionDef {
            name: "pdf_generate".to_string(),
            capabilities: vec![
                "file_write".to_string(),
                "pdf_generation".to_string(),
                "document_generation".to_string(),
            ],
            ..ActionDef::default()
        };

        let metadata = planner_metadata_for_action(&action);
        assert_eq!(metadata.role, PlannerActionRole::Mutation);
        assert_eq!(metadata.integration_class, PlannerIntegrationClass::Media);
        assert_eq!(metadata.side_effect_level, PlannerSideEffectLevel::Write);
    }

    #[test]
    fn search_actions_stay_simple_direct_execution_candidates() {
        let action = ActionDef {
            name: "web_search".to_string(),
            capabilities: vec!["search".to_string()],
            ..ActionDef::default()
        };

        assert!(!action_requires_nontrivial_direct_execution(&action));
    }

    #[test]
    fn structured_action_errors_use_machine_readable_prefixes() {
        let error = structured_action_error_text(
            ActionErrorDomain::Channel,
            ActionErrorReason::NotConnected,
            "Telegram delivery is not connected",
        );

        assert_eq!(
            error,
            "ERR/channel/not_connected: Telegram delivery is not connected"
        );
        assert!(is_structured_action_error_text(&error));
    }

    #[test]
    fn structured_action_errors_are_typed_under_anyhow() {
        let error = structured_action_error(
            ActionErrorDomain::Channel,
            ActionErrorReason::NotConnected,
            "Telegram delivery is not connected",
        );
        let typed = error
            .downcast_ref::<ActionError>()
            .expect("structured action error should downcast");

        assert_eq!(typed.code(), "channel_not_connected");
        assert_eq!(typed.err_prefix(), "ERR/channel/not_connected");
        assert_eq!(
            typed.to_string(),
            "ERR/channel/not_connected: Telegram delivery is not connected"
        );
    }

    #[test]
    fn structured_action_error_text_parses_back_to_type() {
        let parsed =
            parse_structured_action_error_text("ERR/search/timeout: search provider timed out")
                .expect("structured action error should parse");

        assert_eq!(parsed.domain(), ActionErrorDomain::Search);
        assert_eq!(parsed.reason(), ActionErrorReason::Timeout);
        assert_eq!(parsed.code(), "search_timeout");
        assert_eq!(parsed.message(), "search provider timed out");
    }

    #[test]
    fn unstructured_action_errors_are_wrapped_by_action_domain() {
        let error = ensure_structured_action_error_text("web_search", "provider timed out");

        assert_eq!(error, "ERR/search/failed: provider timed out");
    }
}
