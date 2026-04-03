//! Action system with self-improvement capabilities
//!
//! Based on arXiv:2512.17102 "SAGE: Self-Improving Agent with Action Library"

pub mod app;
pub mod calendar;
pub mod gmail;
pub mod google_workspace;
pub mod research;
pub mod search;
#[cfg(feature = "ssh")]
pub mod ssh;

use serde::{Deserialize, Serialize};

use crate::runtime::SandboxMode;

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
pub struct ActionAuthorization {
    #[serde(default)]
    pub risk_level: ActionRiskLevel,
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
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct ActionPlannerMetadata {
    pub role: PlannerActionRole,
    pub requires_auth: bool,
    pub integration_class: PlannerIntegrationClass,
    pub cost: PlannerCostTier,
    pub side_effect_level: PlannerSideEffectLevel,
}

impl Default for ActionPlannerMetadata {
    fn default() -> Self {
        Self {
            role: PlannerActionRole::Unknown,
            requires_auth: false,
            integration_class: PlannerIntegrationClass::Unknown,
            cost: PlannerCostTier::Medium,
            side_effect_level: PlannerSideEffectLevel::None,
        }
    }
}

impl ActionDef {
    pub fn planner_metadata(&self) -> ActionPlannerMetadata {
        planner_metadata_for_action(self)
    }
}

pub fn planner_metadata_for_action(action: &ActionDef) -> ActionPlannerMetadata {
    let name = action.name.trim().to_ascii_lowercase();
    let capabilities = action
        .capabilities
        .iter()
        .map(|value| value.trim().to_ascii_lowercase())
        .collect::<Vec<_>>();

    let mut meta = ActionPlannerMetadata::default();

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
        "watch" | "schedule_task" => {
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
        "file_read" | "list_tasks" | "list_watchers" | "list_integrations" | "app_inspect" => {
            meta.role = PlannerActionRole::Inspection;
            meta.integration_class = if name == "file_read" {
                PlannerIntegrationClass::Filesystem
            } else if name == "app_inspect" {
                PlannerIntegrationClass::App
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

    if capabilities
        .iter()
        .any(|cap| cap == "watcher" || cap == "scheduler")
    {
        meta.role = PlannerActionRole::Orchestration;
        meta.integration_class = PlannerIntegrationClass::Internal;
        meta.cost = PlannerCostTier::Low;
        meta.side_effect_level = PlannerSideEffectLevel::Write;
        return meta;
    }

    if name.starts_with("list_") || capabilities.iter().any(|cap| cap.contains("inventory")) {
        meta.role = PlannerActionRole::Inspection;
        meta.integration_class = PlannerIntegrationClass::Internal;
        meta.cost = PlannerCostTier::Low;
        return meta;
    }

    if name.contains("telegram")
        || name.contains("whatsapp")
        || name.contains("slack")
        || name.contains("discord")
        || name.contains("matrix")
        || name.contains("teams")
    {
        meta.role = PlannerActionRole::Delivery;
        meta.requires_auth = true;
        meta.integration_class = PlannerIntegrationClass::Messaging;
        meta.side_effect_level = PlannerSideEffectLevel::Notify;
        return meta;
    }

    if name.contains("gmail")
        || name.contains("calendar")
        || name.contains("google_workspace")
        || name.contains("google_drive")
        || name.contains("google_docs")
        || name.contains("google_sheets")
        || name.contains("google_chat")
    {
        meta.role = if name.contains("create") || name.contains("reply") {
            PlannerActionRole::Mutation
        } else {
            PlannerActionRole::DataSource
        };
        meta.requires_auth = true;
        meta.integration_class = PlannerIntegrationClass::Workspace;
        meta.side_effect_level = if name.contains("create") || name.contains("reply") {
            PlannerSideEffectLevel::Write
        } else {
            PlannerSideEffectLevel::None
        };
        return meta;
    }

    if capabilities
        .iter()
        .any(|cap| cap == "search" || cap == "research")
    {
        meta.role = PlannerActionRole::DataSource;
        meta.integration_class = PlannerIntegrationClass::Search;
        return meta;
    }

    if capabilities.iter().any(|cap| cap == "browser") {
        meta.role = PlannerActionRole::Mutation;
        meta.integration_class = PlannerIntegrationClass::Browser;
        meta.cost = PlannerCostTier::High;
        meta.side_effect_level = PlannerSideEffectLevel::Write;
        return meta;
    }

    if capabilities.iter().any(|cap| cap.contains("notify")) {
        meta.role = PlannerActionRole::Delivery;
        meta.integration_class = PlannerIntegrationClass::Messaging;
        meta.side_effect_level = PlannerSideEffectLevel::Notify;
        return meta;
    }

    meta
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
