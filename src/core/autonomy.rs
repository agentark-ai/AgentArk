//! Autonomy primitives for policy-driven agent behavior.

use serde::{Deserialize, Serialize};

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
#[derive(Default)]
pub enum ConversationScope {
    #[default]
    PerChannel,
    Global,
}

impl ConversationScope {
    pub fn from_storage(value: Option<&str>) -> Self {
        match value.unwrap_or("").trim().to_ascii_lowercase().as_str() {
            "global" => Self::Global,
            _ => Self::PerChannel,
        }
    }

    pub fn as_storage_str(&self) -> &'static str {
        match self {
            Self::PerChannel => "per_channel",
            Self::Global => "global",
        }
    }

    pub fn conversation_key(&self, channel: &str, project_id: Option<&str>) -> String {
        match (self, project_id) {
            (Self::Global, Some(pid)) => format!("active_conversation_global_{}", pid),
            (Self::Global, None) => "active_conversation_global".to_string(),
            (Self::PerChannel, Some(pid)) => format!("active_conversation_{}_{}", channel, pid),
            (Self::PerChannel, None) => format!("active_conversation_{}", channel),
        }
    }
}

#[derive(Debug, Clone, Serialize, Deserialize, Default)]
#[serde(rename_all = "snake_case")]
pub enum RiskLevel {
    #[default]
    Low,
    Medium,
    High,
    Critical,
}

#[derive(Debug, Clone, Serialize, Deserialize, Default)]
pub struct RiskEnvelope {
    pub level: RiskLevel,
    #[serde(default)]
    pub score: u8,
    #[serde(default)]
    pub requires_approval: bool,
    #[serde(default)]
    pub reasons: Vec<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct TrustPolicy {
    pub auto_execute_max_score: u8,
    pub always_require_approval_actions: Vec<String>,
    pub blocked_actions: Vec<String>,
}

impl Default for TrustPolicy {
    fn default() -> Self {
        Self {
            auto_execute_max_score: 45,
            always_require_approval_actions: vec![
                "shell".to_string(),
                "code_execute".to_string(),
                "file_write".to_string(),
                "ssh".to_string(),
            ],
            blocked_actions: vec![],
        }
    }
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct AutopilotRoutine {
    pub description: String,
    pub action: String,
    #[serde(default)]
    pub arguments: serde_json::Value,
    #[serde(default)]
    pub cron: Option<String>,
    #[serde(default)]
    pub approval: Option<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct AutopilotWatcher {
    pub description: String,
    pub poll_action: String,
    #[serde(default)]
    pub poll_arguments: serde_json::Value,
    pub interval_secs: u64,
    pub timeout_secs: u64,
    #[serde(default)]
    pub condition_contains: Option<String>,
    #[serde(default)]
    pub condition_matches: Option<String>,
    #[serde(default)]
    pub condition_custom: Option<String>,
    pub on_trigger: String,
    pub notify_channel: String,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct AutopilotMode {
    pub id: String,
    pub name: String,
    pub description: String,
    #[serde(default)]
    pub tags: Vec<String>,
    #[serde(default)]
    pub routines: Vec<AutopilotRoutine>,
    #[serde(default)]
    pub watchers: Vec<AutopilotWatcher>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct AutonomySettings {
    pub version: u32,
    #[serde(default)]
    pub context_scope: ConversationScope,
    #[serde(default)]
    pub trust_policy: TrustPolicy,
    #[serde(default)]
    pub modes: Vec<AutopilotMode>,
    #[serde(default)]
    pub active_mode_id: Option<String>,
    #[serde(default)]
    pub voice_briefing_enabled: bool,
    #[serde(default = "default_autonomy_mode")]
    pub autonomy_mode: String,
    #[serde(default)]
    pub always_ask_high_risk: bool,
    #[serde(default)]
    pub only_approved_skills: bool,
    #[serde(default)]
    pub quiet_hours_start: Option<String>,
    #[serde(default)]
    pub quiet_hours_end: Option<String>,
    #[serde(default)]
    pub daily_run_limit: Option<u32>,
    #[serde(default)]
    pub agent_paused: bool,
    #[serde(default = "default_pause_mode")]
    pub pause_mode: String,
    #[serde(default = "default_arkpulse_auth_failures_threshold")]
    pub arkpulse_auth_failures_threshold: u32,
    #[serde(default = "default_arkpulse_rate_limit_hits_threshold")]
    pub arkpulse_rate_limit_hits_threshold: u32,
    #[serde(default = "default_arkpulse_unauthorized_channel_threshold")]
    pub arkpulse_unauthorized_channel_threshold: u32,
    #[serde(default = "default_arkpulse_combined_security_threshold")]
    pub arkpulse_combined_security_threshold: u32,
}

fn default_autonomy_mode() -> String {
    "assist".to_string()
}

fn default_pause_mode() -> String {
    "autonomous_only".to_string()
}

fn default_arkpulse_auth_failures_threshold() -> u32 {
    30
}

fn default_arkpulse_rate_limit_hits_threshold() -> u32 {
    75
}

fn default_arkpulse_unauthorized_channel_threshold() -> u32 {
    24
}

fn default_arkpulse_combined_security_threshold() -> u32 {
    120
}

impl Default for AutonomySettings {
    fn default() -> Self {
        Self {
            version: 1,
            context_scope: ConversationScope::PerChannel,
            trust_policy: TrustPolicy::default(),
            modes: default_modes(),
            active_mode_id: None,
            voice_briefing_enabled: true,
            autonomy_mode: default_autonomy_mode(),
            always_ask_high_risk: true,
            only_approved_skills: true,
            quiet_hours_start: None,
            quiet_hours_end: None,
            daily_run_limit: Some(40),
            agent_paused: false,
            pause_mode: default_pause_mode(),
            arkpulse_auth_failures_threshold: default_arkpulse_auth_failures_threshold(),
            arkpulse_rate_limit_hits_threshold: default_arkpulse_rate_limit_hits_threshold(),
            arkpulse_unauthorized_channel_threshold:
                default_arkpulse_unauthorized_channel_threshold(),
            arkpulse_combined_security_threshold: default_arkpulse_combined_security_threshold(),
        }
    }
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct RecommendedAction {
    pub id: String,
    pub title: String,
    #[serde(default)]
    pub description: String,
    #[serde(alias = "skill_kind")]
    pub action_kind: String,
    #[serde(default)]
    pub payload: serde_json::Value,
    #[serde(default)]
    pub trust: RiskEnvelope,
}

pub fn default_modes() -> Vec<AutopilotMode> {
    vec![
        AutopilotMode {
            id: "focus".to_string(),
            name: "Focus".to_string(),
            description: "Execution-heavy mode that keeps active missions prioritized.".to_string(),
            tags: vec!["execution".to_string(), "deep_work".to_string()],
            routines: vec![AutopilotRoutine {
                description: "Morning focus brief".to_string(),
                action: "daily_brief".to_string(),
                arguments: serde_json::json!({ "mode": "focus" }),
                cron: Some("0 0 9 * * *".to_string()),
                approval: Some("auto".to_string()),
            }],
            watchers: vec![],
        },
        AutopilotMode {
            id: "ops".to_string(),
            name: "Ops".to_string(),
            description: "Operations mode for incident visibility and queue health.".to_string(),
            tags: vec!["operations".to_string(), "incident_response".to_string()],
            routines: vec![AutopilotRoutine {
                description: "Ops pulse summary".to_string(),
                action: "daily_brief".to_string(),
                arguments: serde_json::json!({ "mode": "ops" }),
                cron: Some("0 0 */6 * * *".to_string()),
                approval: Some("auto".to_string()),
            }],
            watchers: vec![],
        },
        AutopilotMode {
            id: "travel".to_string(),
            name: "Travel".to_string(),
            description: "Travel mode for itinerary changes and time-sensitive alerts.".to_string(),
            tags: vec!["travel".to_string(), "logistics".to_string()],
            routines: vec![],
            watchers: vec![],
        },
        AutopilotMode {
            id: "finance".to_string(),
            name: "Finance".to_string(),
            description: "Finance mode for expense hygiene and risk alerts.".to_string(),
            tags: vec!["finance".to_string(), "budget".to_string()],
            routines: vec![],
            watchers: vec![],
        },
    ]
}

pub fn score_action_risk(
    kind: &str,
    payload: &serde_json::Value,
    trust: &TrustPolicy,
) -> RiskEnvelope {
    let mut score: u8 = 10;
    let mut reasons = Vec::new();
    let kind_lc = kind.to_ascii_lowercase();

    let payload_text = payload.to_string().to_ascii_lowercase();
    let risky_tokens = [
        "shell",
        "sudo",
        "delete",
        "rm -rf",
        "credential",
        "token",
        "password",
        "ssh",
    ];
    for token in risky_tokens {
        if payload_text.contains(token) || kind_lc.contains(token) {
            score = score.saturating_add(18);
            reasons.push(format!("contains sensitive operation pattern: {}", token));
        }
    }

    for action in &trust.always_require_approval_actions {
        if kind_lc.contains(&action.to_ascii_lowercase())
            || payload_text.contains(&action.to_ascii_lowercase())
        {
            score = score.max(70);
            reasons.push(format!("action '{}' requires approval by policy", action));
        }
    }

    for action in &trust.blocked_actions {
        if kind_lc.contains(&action.to_ascii_lowercase())
            || payload_text.contains(&action.to_ascii_lowercase())
        {
            score = 100;
            reasons.push(format!("action '{}' is blocked by policy", action));
        }
    }

    let level = match score {
        0..=25 => RiskLevel::Low,
        26..=50 => RiskLevel::Medium,
        51..=79 => RiskLevel::High,
        _ => RiskLevel::Critical,
    };

    let requires_approval = score > trust.auto_execute_max_score
        || matches!(level, RiskLevel::High | RiskLevel::Critical);

    RiskEnvelope {
        level,
        score,
        requires_approval,
        reasons,
    }
}
