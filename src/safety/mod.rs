//! Safety Policy Engine - Formally Verified Constraints
//!
//! Inspired by:
//! - arXiv:2510.05156 "VeriGuard"
//! - arXiv:2503.18666 "AgentSpec"

use anyhow::Result;
use evalexpr::ContextWithMutableVariables;
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

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum SafetyPolicyDecision {
    Allow,
    RequireApproval { reason: String },
    Block { reason: String },
}

impl SafetyPolicyDecision {
    pub fn allowed(&self) -> bool {
        matches!(self, Self::Allow)
    }
}

/// The safety policy engine
pub struct SafetyEngine {
    rules: RwLock<Vec<SafetyRule>>,
    rate_limits: RwLock<std::collections::HashMap<String, RateLimitState>>,
    pending_approvals: RwLock<Vec<PendingApproval>>,
    /// Actions the user has explicitly auto-approved in settings.
    /// When an action is in this set, RequireApproval rules are downgraded to LogAndAllow.
    auto_approved: RwLock<std::collections::HashSet<String>>,
}

struct RateLimitState {
    count: u32,
    window_start: std::time::Instant,
    _interval: std::time::Duration,
    max_count: u32,
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
            rate_limits: RwLock::new(std::collections::HashMap::new()),
            pending_approvals: RwLock::new(Vec::new()),
            auto_approved: RwLock::new(std::collections::HashSet::new()),
        })
    }

    /// Update the set of auto-approved actions from user settings.
    /// Actions in AUTO_APPROVE_BLOCKED are silently ignored.
    pub fn set_auto_approved(&self, actions: &[String]) {
        let blocked: std::collections::HashSet<&str> = crate::core::config::AUTO_APPROVE_BLOCKED
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

    /// Check if an action is allowed by safety policies.
    /// Async so delay rules never block a Tokio worker thread.
    pub async fn is_allowed(
        &self,
        action_name: &str,
        arguments: &serde_json::Value,
    ) -> Result<bool> {
        self.is_allowed_with_authorization(action_name, arguments, None)
            .await
    }

    pub async fn is_allowed_with_authorization(
        &self,
        action_name: &str,
        arguments: &serde_json::Value,
        auth_context: Option<&crate::actions::ActionAuthorizationContext>,
    ) -> Result<bool> {
        Ok(self
            .evaluate_with_authorization(action_name, arguments, auth_context)
            .await?
            .allowed())
    }

    pub async fn evaluate_with_authorization(
        &self,
        action_name: &str,
        arguments: &serde_json::Value,
        auth_context: Option<&crate::actions::ActionAuthorizationContext>,
    ) -> Result<SafetyPolicyDecision> {
        // Clone rules to avoid borrow issues with rate limiting
        let rules = self
            .rules
            .read()
            .map_err(|e| anyhow::anyhow!("Lock error: {}", e))?
            .clone();
        for rule in &rules {
            if Self::rule_matches_static(&rule.trigger, action_name) {
                // Check condition if present
                let condition_met = match &rule.condition {
                    Some(cond) => self.evaluate_condition(cond, action_name, arguments)?,
                    None => true,
                };

                if condition_met {
                    match &rule.action {
                        RuleAction::Allow | RuleAction::LogAndAllow => {
                            if matches!(rule.action, RuleAction::LogAndAllow) {
                                tracing::info!(
                                    "Safety rule '{}' logged action: {}",
                                    rule.name,
                                    action_name
                                );
                            }
                            continue; // Check other rules
                        }
                        RuleAction::Block { message } => {
                            tracing::warn!(
                                "Safety rule '{}' blocked action: {} - {}",
                                rule.name,
                                action_name,
                                message
                            );
                            return Ok(SafetyPolicyDecision::Block {
                                reason: message.clone(),
                            });
                        }
                        RuleAction::RequireApproval => {
                            // Check if user has auto-approved this action in settings
                            let is_auto_approved = self
                                .auto_approved
                                .read()
                                .map(|set| set.contains(action_name))
                                .unwrap_or(false);
                            if is_auto_approved {
                                tracing::info!(
                                    "Safety rule '{}' auto-approved (user setting) for action: {}",
                                    rule.name,
                                    action_name
                                );
                                continue; // User explicitly approved — skip this rule
                            }
                            if explicit_approval_satisfies_safety_rule(auth_context) {
                                tracing::info!(
                                    "Safety rule '{}' approved by explicit approval turn for action: {}",
                                    rule.name,
                                    action_name
                                );
                                continue;
                            }
                            tracing::info!(
                                "Safety rule '{}' requires approval for action: {}",
                                rule.name,
                                action_name
                            );
                            // Store pending approval request for later retrieval
                            if let Ok(mut approvals) = self.pending_approvals.write() {
                                approvals.push(PendingApproval {
                                    _id: uuid::Uuid::new_v4().to_string(),
                                    _action_name: action_name.to_string(),
                                    _arguments: arguments.clone(),
                                    _rule_name: rule.name.clone(),
                                    requested_at: std::time::Instant::now(),
                                });
                            }
                            return Ok(SafetyPolicyDecision::RequireApproval {
                                reason: format!(
                                    "Safety rule '{}' requires explicit approval for action '{}'.",
                                    rule.name, action_name
                                ),
                            });
                        }
                        RuleAction::Delay { seconds } => {
                            tracing::info!(
                                "Safety rule '{}' delaying action by {}s: {}",
                                rule.name,
                                seconds,
                                action_name
                            );
                            tokio::time::sleep(std::time::Duration::from_secs(*seconds)).await;
                            tracing::info!("Delay completed for action: {}", action_name);
                        }
                    }
                }
            }
        }

        Ok(SafetyPolicyDecision::Allow)
    }

    fn rule_matches_static(trigger: &RuleTrigger, action_name: &str) -> bool {
        match trigger {
            RuleTrigger::Action { name } => action_name == name,
            RuleTrigger::ActionPattern { pattern } => {
                // Simple glob matching
                if pattern == "*" {
                    return true;
                }
                if pattern.ends_with('*') {
                    return action_name.starts_with(&pattern[..pattern.len() - 1]);
                }
                action_name == pattern
            }
            RuleTrigger::FileOperation => {
                action_name.starts_with("file_") || action_name.contains("file")
            }
            RuleTrigger::NetworkOperation => {
                action_name.starts_with("http_") || action_name.contains("network")
            }
            RuleTrigger::ShellCommand => action_name == "shell" || action_name == "bash",
            RuleTrigger::Always => true,
        }
    }

    fn evaluate_condition(
        &self,
        condition: &RuleCondition,
        action_name: &str,
        arguments: &serde_json::Value,
    ) -> Result<bool> {
        match condition {
            RuleCondition::PathWithin { directories } => {
                if let Some(path) = arguments.get("path").and_then(|p| p.as_str()) {
                    let path = std::path::Path::new(path);
                    for dir in directories {
                        let dir = std::path::Path::new(dir);
                        if path.starts_with(dir) {
                            return Ok(true);
                        }
                    }
                    return Ok(false);
                }
                Ok(true) // No path argument, allow
            }
            RuleCondition::HostAllowed { hosts } => {
                if let Some(url) = arguments.get("url").and_then(|u| u.as_str()) {
                    if let Ok(parsed) = url::Url::parse(url) {
                        if let Some(host) = parsed.host_str() {
                            return Ok(hosts.iter().any(|h| h == host || h == "*"));
                        }
                    }
                    return Ok(false);
                }
                Ok(true)
            }
            RuleCondition::CommandAllowed { commands } => {
                if let Some(cmd) = arguments.get("command").and_then(|c| c.as_str()) {
                    let first_word = cmd.split_whitespace().next().unwrap_or("");
                    return Ok(commands.iter().any(|c| c == first_word));
                }
                Ok(true)
            }
            RuleCondition::RateLimit {
                max_count,
                interval_seconds,
            } => {
                let key = action_name.to_string();
                let now = std::time::Instant::now();
                let interval = std::time::Duration::from_secs(*interval_seconds);

                let mut rate_limits = self
                    .rate_limits
                    .write()
                    .map_err(|e| anyhow::anyhow!("Lock error: {}", e))?;
                let state = rate_limits.entry(key).or_insert(RateLimitState {
                    count: 0,
                    window_start: now,
                    _interval: interval,
                    max_count: *max_count,
                });

                if now.duration_since(state.window_start) > interval {
                    state.count = 1;
                    state.window_start = now;
                    Ok(true)
                } else if state.count < state.max_count {
                    state.count += 1;
                    Ok(true)
                } else {
                    Ok(false)
                }
            }
            RuleCondition::And { conditions } => {
                for cond in conditions {
                    if !self.evaluate_condition(cond, action_name, arguments)? {
                        return Ok(false);
                    }
                }
                Ok(true)
            }
            RuleCondition::Or { conditions } => {
                for cond in conditions {
                    if self.evaluate_condition(cond, action_name, arguments)? {
                        return Ok(true);
                    }
                }
                Ok(false)
            }
            RuleCondition::Not { condition } => {
                Ok(!self.evaluate_condition(condition, action_name, arguments)?)
            }
            RuleCondition::Expression { expr } => {
                // Evaluate expression using evalexpr
                // Build context with available variables from arguments
                let mut context = evalexpr::HashMapContext::new();

                // Add action_name as variable
                context
                    .set_value(
                        "action_name".into(),
                        evalexpr::Value::String(action_name.to_string()),
                    )
                    .map_err(|e| anyhow::anyhow!("Failed to set context: {}", e))?;

                // Add argument values to context
                if let Some(obj) = arguments.as_object() {
                    for (key, value) in obj {
                        let eval_value = match value {
                            serde_json::Value::String(s) => evalexpr::Value::String(s.clone()),
                            serde_json::Value::Number(n) => {
                                if let Some(i) = n.as_i64() {
                                    evalexpr::Value::Int(i)
                                } else if let Some(f) = n.as_f64() {
                                    evalexpr::Value::Float(f)
                                } else {
                                    continue;
                                }
                            }
                            serde_json::Value::Bool(b) => evalexpr::Value::Boolean(*b),
                            _ => continue,
                        };
                        context
                            .set_value(key.clone(), eval_value)
                            .map_err(|e| anyhow::anyhow!("Failed to set context value: {}", e))?;
                    }
                }

                // Evaluate the expression
                match evalexpr::eval_boolean_with_context(expr, &context) {
                    Ok(result) => Ok(result),
                    Err(e) => {
                        tracing::warn!("Expression evaluation error: {} for expr: {}", e, expr);
                        Ok(true) // Default to allow on error
                    }
                }
            }
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

    /// Get all rules
    #[cfg(feature = "gui")]
    pub fn rules(&self) -> Vec<SafetyRule> {
        self.rules
            .read()
            .map(|rules| rules.clone())
            .unwrap_or_default()
    }

    /// Clear expired pending approvals (older than 1 hour)
    pub fn clear_expired_approvals(&self) {
        let hour = std::time::Duration::from_secs(3600);
        if let Ok(mut approvals) = self.pending_approvals.write() {
            approvals.retain(|a| a.requested_at.elapsed() < hour);
        }
    }
}

fn explicit_approval_satisfies_safety_rule(
    auth_context: Option<&crate::actions::ActionAuthorizationContext>,
) -> bool {
    auth_context.is_some_and(|ctx| {
        matches!(
            ctx.surface,
            crate::actions::ActionExecutionSurface::Chat
                | crate::actions::ActionExecutionSurface::Api
        ) && ctx.direct_user_intent
            && ctx.current_turn_is_explicit_approval
            && ctx
                .principal
                .as_ref()
                .is_some_and(|principal| principal.trusted)
    })
}

#[derive(Debug, Serialize, Deserialize)]
struct SafetyConfig {
    #[serde(rename = "rule")]
    rules: Vec<SafetyRule>,
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn normal_trusted_chat_does_not_satisfy_safety_approval() {
        let ctx = crate::actions::ActionAuthorizationContext {
            principal: Some(crate::actions::ActionCallerPrincipal {
                user_id: "local-user".to_string(),
                role: "owner".to_string(),
                trusted: true,
                auth_source: "web".to_string(),
            }),
            surface: crate::actions::ActionExecutionSurface::Chat,
            direct_user_intent: true,
            current_turn_is_explicit_approval: false,
            agent_name: None,
            agent_access_scope: None,
            capability_context_id: None,
        };
        assert!(!explicit_approval_satisfies_safety_rule(Some(&ctx)));
    }

    #[test]
    fn explicit_trusted_approval_satisfies_safety_approval() {
        let ctx = crate::actions::ActionAuthorizationContext {
            principal: Some(crate::actions::ActionCallerPrincipal {
                user_id: "local-user".to_string(),
                role: "owner".to_string(),
                trusted: true,
                auth_source: "web".to_string(),
            }),
            surface: crate::actions::ActionExecutionSurface::Chat,
            direct_user_intent: true,
            current_turn_is_explicit_approval: true,
            agent_name: None,
            agent_access_scope: None,
            capability_context_id: None,
        };
        assert!(explicit_approval_satisfies_safety_rule(Some(&ctx)));
    }

    #[test]
    fn automation_does_not_override_safety_rules() {
        let ctx = crate::actions::ActionAuthorizationContext {
            principal: Some(crate::actions::ActionCallerPrincipal {
                user_id: "local-user".to_string(),
                role: "owner".to_string(),
                trusted: true,
                auth_source: "automation".to_string(),
            }),
            surface: crate::actions::ActionExecutionSurface::Automation,
            direct_user_intent: false,
            current_turn_is_explicit_approval: false,
            agent_name: None,
            agent_access_scope: None,
            capability_context_id: None,
        };
        assert!(!explicit_approval_satisfies_safety_rule(Some(&ctx)));
    }

    #[tokio::test]
    async fn direct_trusted_chat_does_not_bypass_blocked_rule_actions() {
        let temp = tempfile::tempdir().unwrap();
        let safety = SafetyEngine::new(temp.path()).unwrap();
        safety.add_rule(SafetyRule {
            name: "test_block_all".to_string(),
            description: "Test block rule".to_string(),
            trigger: RuleTrigger::Always,
            condition: None,
            action: RuleAction::Block {
                message: "blocked".to_string(),
            },
            verified: true,
        });
        let ctx = crate::actions::ActionAuthorizationContext {
            principal: Some(crate::actions::ActionCallerPrincipal {
                user_id: "local-user".to_string(),
                role: "owner".to_string(),
                trusted: true,
                auth_source: "web".to_string(),
            }),
            surface: crate::actions::ActionExecutionSurface::Chat,
            direct_user_intent: true,
            current_turn_is_explicit_approval: false,
            agent_name: None,
            agent_access_scope: None,
            capability_context_id: None,
        };

        assert!(!safety
            .is_allowed_with_authorization("http_get", &serde_json::json!({}), Some(&ctx))
            .await
            .unwrap());
        assert!(!safety
            .is_allowed_with_authorization("http_get", &serde_json::json!({}), None)
            .await
            .unwrap());
    }
}
