use once_cell::sync::Lazy;
use regex::Regex;
use serde::{Deserialize, Serialize};
use serde_json::Value;

use super::{redact_pii, redact_secret_input};

pub(crate) const EXECUTION_TARGET_BLOCK_START: &str = "<agentark_current_turn_execution_targets>";
pub(crate) const EXECUTION_TARGET_BLOCK_END: &str = "</agentark_current_turn_execution_targets>";

static EXECUTION_ENDPOINT_RE: Lazy<Regex> = Lazy::new(|| {
    Regex::new(
        r#"(?ix)
        \b(?:
            [a-z][a-z0-9+.-]{1,31}://[^\s<>"'`]+
            |
            (?:
                localhost
                |
                (?:(?:25[0-5]|2[0-4][0-9]|[01]?[0-9][0-9]?)\.){3}
                (?:25[0-5]|2[0-4][0-9]|[01]?[0-9][0-9]?)
            )
            (?:
                :\d{1,5}
                |
                /[^\s<>"'`]+
            )
        )
        "#,
    )
    .unwrap()
});

static ADDRESS_RE: Lazy<Regex> = Lazy::new(|| {
    Regex::new(
        r"(?i)\b\d{1,6}\s+[A-Za-z0-9.'-]+\s+(?:street|st|road|rd|avenue|ave|boulevard|blvd|lane|ln|drive|dr|way|court|ct|place|pl)\b",
    )
    .unwrap()
});
static SPII_RE: Lazy<Regex> = Lazy::new(|| {
    Regex::new(
        r"(?i)\b(?:social security|ssn|passport(?:\s+number)?|driver'?s license|date of birth|dob)\b",
    )
    .unwrap()
});

#[derive(Debug, Clone, Copy, Serialize, Deserialize, PartialEq, Eq, Default)]
#[serde(rename_all = "snake_case")]
pub enum ModelInputPrivacyMode {
    #[default]
    DefaultRedact,
    ZeroExposure,
    SecretsOnly,
}

#[derive(Debug, Clone, Copy, Serialize, Deserialize, PartialEq, Eq, Default)]
#[serde(rename_all = "snake_case")]
pub enum CurrentChatPiiPolicy {
    RawCurrentTurn,
    #[default]
    MaskChatPii,
    BlockSensitiveChat,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct ModelPrivacyConfig {
    #[serde(default)]
    pub default_model_input_mode: ModelInputPrivacyMode,
    #[serde(default)]
    pub current_chat_pii_policy: CurrentChatPiiPolicy,
    #[serde(default = "default_true")]
    pub request_scoped_sensitive_approval_enabled: bool,
}

fn default_true() -> bool {
    true
}

impl Default for ModelPrivacyConfig {
    fn default() -> Self {
        Self {
            default_model_input_mode: ModelInputPrivacyMode::DefaultRedact,
            current_chat_pii_policy: CurrentChatPiiPolicy::MaskChatPii,
            request_scoped_sensitive_approval_enabled: true,
        }
    }
}

#[derive(Debug, Clone, Copy, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "snake_case")]
pub enum ModelInputContext {
    CurrentUserMessage,
    HistoryMessage,
    ToolOutput,
    Memory,
    SavedFact,
    Knowledge,
    Document,
    SystemPrompt,
    InternalHelperPrompt,
    Diagnostic,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "snake_case")]
pub enum ModelInputPrivacyDecision {
    Allow,
    RedactedAllow,
    RequiresApproval,
    Block,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ModelInputPrivacyTextResult {
    pub decision: ModelInputPrivacyDecision,
    pub sanitized_text: String,
    #[serde(default)]
    pub reasons: Vec<String>,
    #[serde(default)]
    pub redactions: Vec<String>,
    #[serde(default)]
    pub secret_detected: bool,
    #[serde(default)]
    pub pii_detected: bool,
    #[serde(default)]
    pub strong_identity_detected: bool,
}

impl ModelInputPrivacyTextResult {
    #[allow(dead_code)]
    pub fn is_model_usable(&self) -> bool {
        matches!(
            self.decision,
            ModelInputPrivacyDecision::Allow | ModelInputPrivacyDecision::RedactedAllow
        )
    }
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[allow(dead_code)]
pub struct ModelInputPrivacyJsonResult {
    pub decision: ModelInputPrivacyDecision,
    pub sanitized_value: Value,
    #[serde(default)]
    pub reasons: Vec<String>,
    #[serde(default)]
    pub redactions: Vec<String>,
}

fn push_unique(target: &mut Vec<String>, value: impl Into<String>) {
    let value = value.into();
    if value.trim().is_empty() || target.iter().any(|existing| existing == &value) {
        return;
    }
    target.push(value);
}

fn has_strong_identity_material(text: &str, reasons: &mut Vec<String>) -> bool {
    let mut found = false;
    if ADDRESS_RE.is_match(text) {
        push_unique(
            reasons,
            "street-address-like content detected in model input",
        );
        found = true;
    }
    if SPII_RE.is_match(text) {
        push_unique(
            reasons,
            "sensitive personal identity material detected in model input",
        );
        found = true;
    }
    found
}

fn render_secret_sanitized_text(result: &super::SecretRedactionResult) -> String {
    if result.uses_specific_api_key_placeholder() {
        result
            .text
            .replace("[REDACTED_SECRET]", "[REDACTED_API_KEY]")
    } else {
        result.text.clone()
    }
}

fn protect_execution_target_blocks(text: &str) -> (String, Vec<String>) {
    let mut protected = String::with_capacity(text.len());
    let mut blocks = Vec::new();
    let mut cursor = 0;

    while let Some(relative_start) = text[cursor..].find(EXECUTION_TARGET_BLOCK_START) {
        let start = cursor + relative_start;
        let content_start = start + EXECUTION_TARGET_BLOCK_START.len();
        let Some(relative_end) = text[content_start..].find(EXECUTION_TARGET_BLOCK_END) else {
            break;
        };
        let end = content_start + relative_end + EXECUTION_TARGET_BLOCK_END.len();
        let block = text[start..end].to_string();
        let placeholder = format!("__AGENTARK_EXECUTION_TARGET_BLOCK_{}__", blocks.len());

        protected.push_str(&text[cursor..start]);
        protected.push_str(&placeholder);
        blocks.push(block);
        cursor = end;
    }

    protected.push_str(&text[cursor..]);
    (protected, blocks)
}

fn restore_execution_target_blocks(mut text: String, blocks: &[String]) -> String {
    for (idx, block) in blocks.iter().enumerate() {
        let placeholder = format!("__AGENTARK_EXECUTION_TARGET_BLOCK_{}__", idx);
        text = text.replace(&placeholder, block);
    }
    text
}

fn protect_execution_endpoints(text: &str) -> (String, Vec<String>) {
    let mut protected = String::with_capacity(text.len());
    let mut endpoints = Vec::new();
    let mut cursor = 0;

    for capture in EXECUTION_ENDPOINT_RE.find_iter(text) {
        let start = capture.start();
        let end = capture.end();
        if start < cursor {
            continue;
        }
        let endpoint = capture.as_str().to_string();
        let placeholder = format!("__AGENTARK_EXECUTION_ENDPOINT_{}__", endpoints.len());

        protected.push_str(&text[cursor..start]);
        protected.push_str(&placeholder);
        endpoints.push(endpoint);
        cursor = end;
    }

    protected.push_str(&text[cursor..]);
    (protected, endpoints)
}

fn restore_execution_endpoints(mut text: String, endpoints: &[String]) -> String {
    for (idx, endpoint) in endpoints.iter().enumerate() {
        let placeholder = format!("__AGENTARK_EXECUTION_ENDPOINT_{}__", idx);
        text = text.replace(&placeholder, endpoint);
    }
    text
}

fn redact_pii_for_model_context(text: &str, context: ModelInputContext) -> String {
    let protect_endpoints = matches!(
        context,
        ModelInputContext::CurrentUserMessage
            | ModelInputContext::HistoryMessage
            | ModelInputContext::SystemPrompt
            | ModelInputContext::InternalHelperPrompt
    );

    if matches!(
        context,
        ModelInputContext::SystemPrompt | ModelInputContext::InternalHelperPrompt
    ) {
        let (protected, blocks) = protect_execution_target_blocks(text);
        let (protected, endpoints) = if protect_endpoints {
            protect_execution_endpoints(&protected)
        } else {
            (protected, Vec::new())
        };
        let redacted = redact_pii(&protected);
        let restored = restore_execution_endpoints(redacted, &endpoints);
        restore_execution_target_blocks(restored, &blocks)
    } else if protect_endpoints {
        let (protected, endpoints) = protect_execution_endpoints(text);
        restore_execution_endpoints(redact_pii(&protected), &endpoints)
    } else {
        redact_pii(text)
    }
}

pub fn render_model_input_fallback(
    result: &ModelInputPrivacyTextResult,
    context: ModelInputContext,
) -> String {
    match result.decision {
        ModelInputPrivacyDecision::Allow | ModelInputPrivacyDecision::RedactedAllow => {
            result.sanitized_text.clone()
        }
        ModelInputPrivacyDecision::RequiresApproval => format!(
            "[SENSITIVE_CONTEXT_WITHHELD:{}] {}",
            context.as_str(),
            if result.reasons.is_empty() {
                "approval required before this content can be inspected".to_string()
            } else {
                result.reasons.join("; ")
            }
        ),
        ModelInputPrivacyDecision::Block => format!(
            "[SENSITIVE_CONTEXT_BLOCKED:{}] {}",
            context.as_str(),
            if result.reasons.is_empty() {
                "blocked by privacy policy".to_string()
            } else {
                result.reasons.join("; ")
            }
        ),
    }
}

impl ModelInputContext {
    pub fn as_str(self) -> &'static str {
        match self {
            Self::CurrentUserMessage => "current_user_message",
            Self::HistoryMessage => "history_message",
            Self::ToolOutput => "tool_output",
            Self::Memory => "memory",
            Self::SavedFact => "saved_fact",
            Self::Knowledge => "knowledge",
            Self::Document => "document",
            Self::SystemPrompt => "system_prompt",
            Self::InternalHelperPrompt => "internal_helper_prompt",
            Self::Diagnostic => "diagnostic",
        }
    }
}

fn should_redact_pii(policy: &ModelPrivacyConfig, context: ModelInputContext) -> bool {
    match context {
        ModelInputContext::CurrentUserMessage => {
            matches!(
                policy.current_chat_pii_policy,
                CurrentChatPiiPolicy::MaskChatPii
            )
        }
        _ => matches!(
            policy.default_model_input_mode,
            ModelInputPrivacyMode::DefaultRedact
        ),
    }
}

pub fn sanitize_model_input_text(
    text: &str,
    policy: &ModelPrivacyConfig,
    context: ModelInputContext,
    sensitive_context_approved: bool,
) -> ModelInputPrivacyTextResult {
    if text.is_empty() {
        return ModelInputPrivacyTextResult {
            decision: ModelInputPrivacyDecision::Allow,
            sanitized_text: String::new(),
            reasons: Vec::new(),
            redactions: Vec::new(),
            secret_detected: false,
            pii_detected: false,
            strong_identity_detected: false,
        };
    }

    let raw = text.to_string();
    let mut reasons = Vec::new();
    let mut redactions = Vec::new();
    let mut sanitized = raw.clone();

    let current_turn_raw = matches!(context, ModelInputContext::CurrentUserMessage)
        && matches!(
            policy.current_chat_pii_policy,
            CurrentChatPiiPolicy::RawCurrentTurn
        );
    let approved_sensitive_passthrough =
        sensitive_context_approved && !matches!(context, ModelInputContext::CurrentUserMessage);

    let secret_result = redact_secret_input(&sanitized);
    let secret_detected = secret_result.had_secret();
    if secret_detected {
        push_unique(
            &mut reasons,
            "secret-like material detected and redacted from model input",
        );
        redactions.extend(secret_result.redactions.clone());
        sanitized = render_secret_sanitized_text(&secret_result);
    }

    let pii_detected;
    if current_turn_raw {
        pii_detected = false;
    } else if approved_sensitive_passthrough && !secret_detected {
        pii_detected = redact_pii_for_model_context(&sanitized, context) != sanitized;
    } else if should_redact_pii(policy, context) {
        let pii_redacted = redact_pii_for_model_context(&sanitized, context);
        pii_detected = pii_redacted != sanitized;
        if pii_detected {
            push_unique(
                &mut reasons,
                "PII-like material detected and redacted from model input",
            );
            push_unique(&mut redactions, "pii_redaction");
            sanitized = pii_redacted;
        }
    } else {
        pii_detected = redact_pii_for_model_context(&sanitized, context) != sanitized;
    }

    let strong_identity_detected = if current_turn_raw {
        false
    } else {
        let mut strong_reasons = Vec::new();
        let detected = has_strong_identity_material(&raw, &mut strong_reasons)
            || has_strong_identity_material(&sanitized, &mut strong_reasons);
        for reason in strong_reasons {
            push_unique(&mut reasons, reason);
        }
        detected
    };

    if matches!(context, ModelInputContext::CurrentUserMessage)
        && matches!(
            policy.current_chat_pii_policy,
            CurrentChatPiiPolicy::BlockSensitiveChat
        )
        && (pii_detected || strong_identity_detected)
    {
        return ModelInputPrivacyTextResult {
            decision: ModelInputPrivacyDecision::Block,
            sanitized_text: sanitized,
            reasons,
            redactions,
            secret_detected,
            pii_detected,
            strong_identity_detected,
        };
    }

    if approved_sensitive_passthrough {
        return ModelInputPrivacyTextResult {
            decision: if raw == sanitized {
                ModelInputPrivacyDecision::Allow
            } else {
                ModelInputPrivacyDecision::RedactedAllow
            },
            sanitized_text: sanitized,
            reasons,
            redactions,
            secret_detected,
            pii_detected,
            strong_identity_detected,
        };
    }

    let strong_identity_requires_gate =
        strong_identity_detected && !matches!(context, ModelInputContext::SystemPrompt);

    let decision = if strong_identity_requires_gate {
        if matches!(context, ModelInputContext::CurrentUserMessage) {
            ModelInputPrivacyDecision::Block
        } else if policy.request_scoped_sensitive_approval_enabled && !secret_detected {
            ModelInputPrivacyDecision::RequiresApproval
        } else {
            ModelInputPrivacyDecision::Block
        }
    } else {
        match context {
            ModelInputContext::CurrentUserMessage if current_turn_raw => {
                if secret_detected {
                    ModelInputPrivacyDecision::RedactedAllow
                } else {
                    ModelInputPrivacyDecision::Allow
                }
            }
            _ => match policy.default_model_input_mode {
                ModelInputPrivacyMode::DefaultRedact => {
                    if sanitized != raw {
                        ModelInputPrivacyDecision::RedactedAllow
                    } else {
                        ModelInputPrivacyDecision::Allow
                    }
                }
                ModelInputPrivacyMode::ZeroExposure => {
                    if sanitized != raw || pii_detected {
                        ModelInputPrivacyDecision::Block
                    } else {
                        ModelInputPrivacyDecision::Allow
                    }
                }
                ModelInputPrivacyMode::SecretsOnly => {
                    if secret_detected {
                        ModelInputPrivacyDecision::RedactedAllow
                    } else {
                        ModelInputPrivacyDecision::Allow
                    }
                }
            },
        }
    };

    ModelInputPrivacyTextResult {
        decision,
        sanitized_text: sanitized,
        reasons,
        redactions,
        secret_detected,
        pii_detected,
        strong_identity_detected,
    }
}

#[allow(dead_code)]
fn sanitize_json_value(
    value: &Value,
    policy: &ModelPrivacyConfig,
    context: ModelInputContext,
    sensitive_context_approved: bool,
    reasons: &mut Vec<String>,
    redactions: &mut Vec<String>,
    changed: &mut bool,
    needs_approval: &mut bool,
    blocked: &mut bool,
) -> Value {
    match value {
        Value::String(text) => {
            let result =
                sanitize_model_input_text(text, policy, context, sensitive_context_approved);
            for reason in &result.reasons {
                push_unique(reasons, reason.clone());
            }
            for redaction in &result.redactions {
                push_unique(redactions, redaction.clone());
            }
            match result.decision {
                ModelInputPrivacyDecision::RequiresApproval => *needs_approval = true,
                ModelInputPrivacyDecision::Block => *blocked = true,
                _ => {}
            }
            if result.sanitized_text != *text {
                *changed = true;
            }
            Value::String(render_model_input_fallback(&result, context))
        }
        Value::Array(items) => Value::Array(
            items
                .iter()
                .map(|item| {
                    sanitize_json_value(
                        item,
                        policy,
                        context,
                        sensitive_context_approved,
                        reasons,
                        redactions,
                        changed,
                        needs_approval,
                        blocked,
                    )
                })
                .collect(),
        ),
        Value::Object(map) => {
            let mut sanitized = serde_json::Map::with_capacity(map.len());
            for (key, value) in map {
                sanitized.insert(
                    key.clone(),
                    sanitize_json_value(
                        value,
                        policy,
                        context,
                        sensitive_context_approved,
                        reasons,
                        redactions,
                        changed,
                        needs_approval,
                        blocked,
                    ),
                );
            }
            Value::Object(sanitized)
        }
        other => other.clone(),
    }
}

#[allow(dead_code)]
pub fn sanitize_model_input_json(
    value: &Value,
    policy: &ModelPrivacyConfig,
    context: ModelInputContext,
    sensitive_context_approved: bool,
) -> ModelInputPrivacyJsonResult {
    let mut reasons = Vec::new();
    let mut redactions = Vec::new();
    let mut changed = false;
    let mut needs_approval = false;
    let mut blocked = false;
    let sanitized_value = sanitize_json_value(
        value,
        policy,
        context,
        sensitive_context_approved,
        &mut reasons,
        &mut redactions,
        &mut changed,
        &mut needs_approval,
        &mut blocked,
    );
    let decision = if blocked {
        ModelInputPrivacyDecision::Block
    } else if needs_approval {
        ModelInputPrivacyDecision::RequiresApproval
    } else if changed {
        ModelInputPrivacyDecision::RedactedAllow
    } else {
        ModelInputPrivacyDecision::Allow
    };
    ModelInputPrivacyJsonResult {
        decision,
        sanitized_value,
        reasons,
        redactions,
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn fake_openai_key() -> String {
        ["sk", "-1234567890", "abcdefghijklmnop"].concat()
    }

    #[test]
    fn default_redact_masks_secrets_and_pii() {
        let result = sanitize_model_input_text(
            &format!("Email jane@example.com token {}", fake_openai_key()),
            &ModelPrivacyConfig::default(),
            ModelInputContext::ToolOutput,
            false,
        );
        assert_eq!(result.decision, ModelInputPrivacyDecision::RedactedAllow);
        assert!(result.sanitized_text.contains("[EMAIL]"));
        assert!(result.sanitized_text.contains("[REDACTED_API_KEY]"));
    }

    #[test]
    fn default_current_turn_masks_pii_and_redacts_secrets() {
        let result = sanitize_model_input_text(
            &format!(
                "My email is jane@example.com and key is {}",
                fake_openai_key()
            ),
            &ModelPrivacyConfig::default(),
            ModelInputContext::CurrentUserMessage,
            false,
        );
        assert_eq!(result.decision, ModelInputPrivacyDecision::RedactedAllow);
        assert!(result.sanitized_text.contains("[EMAIL]"));
        assert!(result.sanitized_text.contains("[REDACTED_API_KEY]"));
    }

    #[test]
    fn explicit_raw_current_turn_keeps_pii_but_still_redacts_secrets() {
        let result = sanitize_model_input_text(
            &format!(
                "My email is jane@example.com and key is {}",
                fake_openai_key()
            ),
            &ModelPrivacyConfig {
                current_chat_pii_policy: CurrentChatPiiPolicy::RawCurrentTurn,
                ..ModelPrivacyConfig::default()
            },
            ModelInputContext::CurrentUserMessage,
            false,
        );
        assert_eq!(result.decision, ModelInputPrivacyDecision::RedactedAllow);
        assert!(result.sanitized_text.contains("jane@example.com"));
        assert!(result.sanitized_text.contains("[REDACTED_API_KEY]"));
    }

    #[test]
    fn system_prompt_preserves_trusted_execution_targets_after_secret_redaction() {
        let result = sanitize_model_input_text(
            "Probe server 10.0.0.4.\n<agentark_current_turn_execution_targets>\ntarget: rtsp://10.0.0.5:554/live?password=supersecretvalue123456&channel=1\n</agentark_current_turn_execution_targets>",
            &ModelPrivacyConfig::default(),
            ModelInputContext::SystemPrompt,
            false,
        );

        assert_eq!(result.decision, ModelInputPrivacyDecision::RedactedAllow);
        assert!(result.sanitized_text.contains("Probe server [IP]."));
        assert!(result.sanitized_text.contains("rtsp://10.0.0.5:554/live"));
        assert!(result.sanitized_text.contains("password=[REDACTED_SECRET]"));
        assert!(!result.sanitized_text.contains("supersecretvalue123456"));
    }

    #[test]
    fn system_prompt_identity_policy_is_not_withheld_by_user_name_wording() {
        let result = sanitize_model_input_text(
            "You are AgentArk.\nWhen the user asks what your name is, answer as AgentArk.",
            &ModelPrivacyConfig::default(),
            ModelInputContext::SystemPrompt,
            false,
        );

        assert_eq!(result.decision, ModelInputPrivacyDecision::Allow);
        assert!(result.sanitized_text.contains("You are AgentArk."));
        assert!(!result
            .sanitized_text
            .contains("[SENSITIVE_CONTEXT_WITHHELD"));
    }

    #[test]
    fn history_redaction_preserves_execution_endpoints() {
        let result = sanitize_model_input_text(
            "Poll rtsp://192.168.29.61:554/live.sdp and compare with bare host 10.0.0.4.",
            &ModelPrivacyConfig::default(),
            ModelInputContext::HistoryMessage,
            false,
        );

        assert_eq!(result.decision, ModelInputPrivacyDecision::RedactedAllow);
        assert!(result
            .sanitized_text
            .contains("rtsp://192.168.29.61:554/live.sdp"));
        assert!(result.sanitized_text.contains("bare host [IP]"));
    }

    #[test]
    fn strong_identity_requires_approval_for_tool_output() {
        let result = sanitize_model_input_text(
            "The user address is 123 Main Street and SSN 123-45-6789",
            &ModelPrivacyConfig::default(),
            ModelInputContext::ToolOutput,
            false,
        );
        assert_eq!(result.decision, ModelInputPrivacyDecision::RequiresApproval);
        assert!(result.strong_identity_detected);
    }

    #[test]
    fn strong_identity_can_flow_after_approval() {
        let result = sanitize_model_input_text(
            "The user address is 123 Main Street and SSN 123-45-6789",
            &ModelPrivacyConfig::default(),
            ModelInputContext::ToolOutput,
            true,
        );
        assert_eq!(result.decision, ModelInputPrivacyDecision::Allow);
        assert!(result.sanitized_text.contains("123 Main Street"));
        assert!(result.sanitized_text.contains("123-45-6789"));
    }

    #[test]
    fn internal_helper_prompt_can_flow_identity_after_scoped_approval() {
        let result = sanitize_model_input_text(
            "User message:\nmy name is Example User",
            &ModelPrivacyConfig::default(),
            ModelInputContext::InternalHelperPrompt,
            true,
        );
        assert_eq!(result.decision, ModelInputPrivacyDecision::Allow);
        assert!(result.sanitized_text.contains("Example User"));
        assert!(!result
            .sanitized_text
            .contains("[SENSITIVE_CONTEXT_WITHHELD"));
    }

    #[test]
    fn internal_helper_prompt_allows_ordinary_self_introduction() {
        let result = sanitize_model_input_text(
            "User message:\nmy name is Debanka and i work for OpenAI",
            &ModelPrivacyConfig::default(),
            ModelInputContext::InternalHelperPrompt,
            false,
        );

        assert_eq!(result.decision, ModelInputPrivacyDecision::Allow);
        assert!(!result.strong_identity_detected);
        assert!(result.sanitized_text.contains("Debanka"));
        assert!(result.sanitized_text.contains("OpenAI"));
    }

    #[test]
    fn internal_helper_prompt_does_not_withhold_router_text_about_user_name() {
        let result = sanitize_model_input_text(
            "Candidate capability: save the user's name as a durable memory.\nUser message:\nmy name is Debanka",
            &ModelPrivacyConfig::default(),
            ModelInputContext::InternalHelperPrompt,
            false,
        );

        assert_eq!(result.decision, ModelInputPrivacyDecision::Allow);
        assert!(!result
            .sanitized_text
            .contains("[SENSITIVE_CONTEXT_WITHHELD"));
    }

    #[test]
    fn internal_helper_prompt_keeps_identity_when_secret_is_redacted_after_approval() {
        let result = sanitize_model_input_text(
            &format!(
                "Current user fact:\nmy name is Example User\n\nSecret assignment:\napi_key={}",
                fake_openai_key()
            ),
            &ModelPrivacyConfig::default(),
            ModelInputContext::InternalHelperPrompt,
            true,
        );
        assert_eq!(result.decision, ModelInputPrivacyDecision::RedactedAllow);
        assert!(result.sanitized_text.contains("Example User"));
        assert!(result.sanitized_text.contains("[REDACTED_SECRET]"));
        assert!(!result
            .sanitized_text
            .contains("[SENSITIVE_CONTEXT_WITHHELD"));
    }

    #[test]
    fn zero_exposure_blocks_redacted_helper_input() {
        let result = sanitize_model_input_text(
            "Contact jane@example.com",
            &ModelPrivacyConfig {
                default_model_input_mode: ModelInputPrivacyMode::ZeroExposure,
                ..ModelPrivacyConfig::default()
            },
            ModelInputContext::InternalHelperPrompt,
            false,
        );
        assert_eq!(result.decision, ModelInputPrivacyDecision::Block);
    }
}
