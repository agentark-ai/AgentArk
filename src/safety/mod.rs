//! Safety Policy Engine - Formally Verified Constraints
//!
//! Inspired by:
//! - arXiv:2510.05156 "VeriGuard"
//! - arXiv:2503.18666 "AgentSpec"

use anyhow::Result;
use serde::{Deserialize, Serialize};
use std::path::Path;
use std::sync::RwLock;

/// A safety rule that constrains agent behavior
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct SafetyRule {
    /// Rule name
    pub name: String,

    /// Description of what this rule does
    pub description: String,

    /// Trigger condition (which actions this applies to)
    pub trigger: RuleTrigger,

    /// Condition that must be true for action to proceed
    pub condition: Option<RuleCondition>,

    /// What to do when rule matches
    pub action: RuleAction,

    /// Whether this rule has been formally verified
    pub verified: bool,
}

/// What triggers a safety rule
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(tag = "type", rename_all = "snake_case")]
pub enum RuleTrigger {
    /// Match a specific action/tool name
    Action { name: String },
    /// Match actions matching a pattern
    ActionPattern { pattern: String },
    /// Match any file operation
    FileOperation,
    /// Match any network operation
    NetworkOperation,
    /// Match any shell command
    ShellCommand,
    /// Always trigger
    Always,
}

/// Condition for a safety rule
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(tag = "type", rename_all = "snake_case")]
pub enum RuleCondition {
    /// Path must be within allowed directories
    PathWithin { directories: Vec<String> },
    /// Host must be in allowlist
    HostAllowed { hosts: Vec<String> },
    /// Command must be in allowlist
    CommandAllowed { commands: Vec<String> },
    /// Rate limit (max N per interval)
    RateLimit {
        max_count: u32,
        interval_seconds: u64,
    },
    /// Custom expression
    Expression { expr: String },
    /// All conditions must match
    And { conditions: Vec<RuleCondition> },
    /// Any condition must match
    Or { conditions: Vec<RuleCondition> },
    /// Negate a condition
    Not { condition: Box<RuleCondition> },
}

/// Action to take when rule matches
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum RuleAction {
    /// Allow the action
    Allow,
    /// Block the action with a message
    Block { message: String },
    /// Require explicit user approval
    RequireApproval,
    /// Delay execution
    Delay { seconds: u64 },
    /// Log and allow
    LogAndAllow,
}

/// The safety policy engine
pub struct SafetyEngine {
    rules: RwLock<Vec<SafetyRule>>,
    pending_approvals: RwLock<Vec<PendingApproval>>,
    /// Actions the user has explicitly auto-approved in settings.
    /// Retained for settings compatibility; RequireApproval rules now audit and allow.
    auto_approved: RwLock<std::collections::HashSet<String>>,
}

/// A pending approval request
#[derive(Debug, Clone)]
pub struct PendingApproval {
    pub _id: String,
    pub _action_name: String,
    pub _arguments: serde_json::Value,
    pub _rule_name: String,
    pub requested_at: std::time::Instant,
}

impl SafetyEngine {
    /// Create a new safety engine with default rules
    pub fn new(config_dir: &Path) -> Result<Self> {
        let rules_path = config_dir.join("safety.toml");

        let rules = if rules_path.exists() {
            let content = std::fs::read_to_string(&rules_path)?;
            let config: SafetyConfig = toml::from_str(&content)?;
            config.rules
        } else {
            // Create default safety rules
            let default_rules = Self::default_rules();
            let config = SafetyConfig {
                rules: default_rules.clone(),
            };
            let content = toml::to_string_pretty(&config)?;
            std::fs::write(&rules_path, content)?;
            default_rules
        };

        Ok(Self {
            rules: RwLock::new(rules),
            pending_approvals: RwLock::new(Vec::new()),
            auto_approved: RwLock::new(std::collections::HashSet::new()),
        })
    }

    /// Update the set of auto-approved actions from user settings.
    /// Actions in AUTO_APPROVE_BLOCKED are silently ignored.
    pub fn set_auto_approved(&self, actions: &[String]) {
        let blocked: std::collections::HashSet<&str> =
            crate::core::runtime::config::AUTO_APPROVE_BLOCKED
                .iter()
                .copied()
                .collect();
        let approved: std::collections::HashSet<String> = actions
            .iter()
            .map(|s| s.trim().to_string())
            .filter(|s| !s.is_empty() && !blocked.contains(s.as_str()))
            .collect();
        if let Ok(mut set) = self.auto_approved.write() {
            *set = approved;
        }
    }

    /// Get default safety rules
    fn default_rules() -> Vec<SafetyRule> {
        vec![
            SafetyRule {
                name: "block_dangerous_commands".to_string(),
                description: "Block potentially dangerous shell commands".to_string(),
                trigger: RuleTrigger::ShellCommand,
                condition: Some(RuleCondition::Not {
                    condition: Box::new(RuleCondition::CommandAllowed {
                        commands: vec![
                            "ls".to_string(),
                            "cat".to_string(),
                            "echo".to_string(),
                            "pwd".to_string(),
                            "whoami".to_string(),
                            "date".to_string(),
                            "cargo".to_string(),
                            "git".to_string(),
                            "npm".to_string(),
                            "node".to_string(),
                            "python".to_string(),
                            "pip".to_string(),
                        ],
                    }),
                }),
                action: RuleAction::RequireApproval,
                verified: true,
            },
            SafetyRule {
                name: "rate_limit_network".to_string(),
                description: "Rate limit network requests".to_string(),
                trigger: RuleTrigger::NetworkOperation,
                condition: Some(RuleCondition::RateLimit {
                    max_count: 60,
                    interval_seconds: 60,
                }),
                action: RuleAction::Block {
                    message: "Rate limit exceeded".to_string(),
                },
                verified: true,
            },
            SafetyRule {
                name: "log_all_file_operations".to_string(),
                description: "Log all file operations".to_string(),
                trigger: RuleTrigger::FileOperation,
                condition: None,
                action: RuleAction::LogAndAllow,
                verified: true,
            },
            SafetyRule {
                name: "approve_gmail_send".to_string(),
                description: "Sending emails requires explicit user approval unless auto-approved in settings".to_string(),
                trigger: RuleTrigger::Action {
                    name: "gmail_send".to_string(),
                },
                condition: None,
                action: RuleAction::RequireApproval,
                verified: true,
            },
            SafetyRule {
                name: "approve_gmail_reply".to_string(),
                description: "Replying to emails requires explicit user approval unless auto-approved in settings".to_string(),
                trigger: RuleTrigger::Action {
                    name: "gmail_reply".to_string(),
                },
                condition: None,
                action: RuleAction::RequireApproval,
                verified: true,
            },
        ]
    }

    /// Add a new safety rule
    pub fn add_rule(&self, rule: SafetyRule) {
        if let Ok(mut rules) = self.rules.write() {
            rules.push(rule);
        }
    }

    /// Clear expired pending approvals (older than 1 hour)
    pub fn clear_expired_approvals(&self) {
        let hour = std::time::Duration::from_secs(3600);
        if let Ok(mut approvals) = self.pending_approvals.write() {
            approvals.retain(|a| a.requested_at.elapsed() < hour);
        }
    }
}

#[derive(Debug, Serialize, Deserialize)]
struct SafetyConfig {
    #[serde(rename = "rule")]
    rules: Vec<SafetyRule>,
}
