use once_cell::sync::Lazy;
use regex::Regex;
use serde::{Deserialize, Serialize};
use serde_json::Value;

use super::{redact_pii, redact_secret_input};

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

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "snake_case")]
pub enum OutboundPrivacyDecision {
    Allow,
    RedactedAllow,
    Block,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct OutboundPrivacyPolicy {
    pub auto_redact_enabled: bool,
    pub public_learning_fenced: bool,
}

impl Default for OutboundPrivacyPolicy {
    fn default() -> Self {
        Self {
            auto_redact_enabled: true,
            public_learning_fenced: true,
        }
    }
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct OutboundPrivacyTextResult {
    pub decision: OutboundPrivacyDecision,
    pub sanitized_text: String,
    #[serde(default)]
    pub reasons: Vec<String>,
    #[serde(default)]
    pub redactions: Vec<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct OutboundPrivacyJsonResult {
    pub decision: OutboundPrivacyDecision,
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

fn has_hard_blocker(text: &str, reasons: &mut Vec<String>) -> bool {
    let mut blocked = false;
    if ADDRESS_RE.is_match(text) {
        push_unique(
            reasons,
            "street-address-like content detected in outbound content",
        );
        blocked = true;
    }
    if SPII_RE.is_match(text) {
        push_unique(
            reasons,
            "sensitive personal identity material detected in outbound content",
        );
        blocked = true;
    }
    blocked
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

pub fn format_outbound_privacy_block(context: &str, reasons: &[String]) -> String {
    let joined = reasons
        .iter()
        .map(|value| value.trim())
        .filter(|value| !value.is_empty())
        .collect::<Vec<_>>()
        .join("; ");
    if joined.is_empty() {
        format!("Outbound privacy gate blocked {}", context)
    } else {
        format!("Outbound privacy gate blocked {}: {}", context, joined)
    }
}

pub fn check_outbound_text(
    text: &str,
    policy: &OutboundPrivacyPolicy,
) -> OutboundPrivacyTextResult {
    let original = text.trim().to_string();
    if original.is_empty() {
        return OutboundPrivacyTextResult {
            decision: OutboundPrivacyDecision::Allow,
            sanitized_text: String::new(),
            reasons: Vec::new(),
            redactions: Vec::new(),
        };
    }

    let mut reasons = Vec::new();
    let mut redactions = Vec::new();
    let mut sanitized = original.clone();

    let secret_result = redact_secret_input(&sanitized);
    if secret_result.had_secret() {
        push_unique(
            &mut reasons,
            "secret-like material detected and redacted from outbound content",
        );
        redactions.extend(secret_result.redactions.clone());
        sanitized = render_secret_sanitized_text(&secret_result);
    }

    let pii_redacted = redact_pii(&sanitized);
    if pii_redacted != sanitized {
        push_unique(
            &mut reasons,
            "PII-like material detected and redacted from outbound content",
        );
        push_unique(&mut redactions, "pii_redaction");
        sanitized = pii_redacted;
    }

    let blocked =
        has_hard_blocker(&original, &mut reasons) || has_hard_blocker(&sanitized, &mut reasons);
    let changed = sanitized != original;

    let decision = if blocked {
        OutboundPrivacyDecision::Block
    } else if changed {
        if policy.auto_redact_enabled {
            OutboundPrivacyDecision::RedactedAllow
        } else {
            push_unique(
                &mut reasons,
                "auto-redaction is disabled, so risky outbound content was blocked",
            );
            OutboundPrivacyDecision::Block
        }
    } else {
        OutboundPrivacyDecision::Allow
    };

    OutboundPrivacyTextResult {
        decision,
        sanitized_text: sanitized,
        reasons,
        redactions,
    }
}

fn sanitize_json_value(
    value: &Value,
    policy: &OutboundPrivacyPolicy,
    reasons: &mut Vec<String>,
    redactions: &mut Vec<String>,
    changed: &mut bool,
    blocked: &mut bool,
) -> Value {
    match value {
        Value::String(text) => {
            let result = check_outbound_text(text, policy);
            for reason in result.reasons {
                push_unique(reasons, reason);
            }
            for redaction in result.redactions {
                push_unique(redactions, redaction);
            }
            if matches!(result.decision, OutboundPrivacyDecision::Block) {
                *blocked = true;
            }
            if result.sanitized_text != *text {
                *changed = true;
            }
            Value::String(result.sanitized_text)
        }
        Value::Array(items) => Value::Array(
            items
                .iter()
                .map(|item| {
                    sanitize_json_value(item, policy, reasons, redactions, changed, blocked)
                })
                .collect(),
        ),
        Value::Object(map) => {
            let mut sanitized = serde_json::Map::with_capacity(map.len());
            for (key, value) in map {
                sanitized.insert(
                    key.clone(),
                    sanitize_json_value(value, policy, reasons, redactions, changed, blocked),
                );
            }
            Value::Object(sanitized)
        }
        other => other.clone(),
    }
}

pub fn sanitize_outbound_json(
    value: &Value,
    policy: &OutboundPrivacyPolicy,
) -> OutboundPrivacyJsonResult {
    let mut reasons = Vec::new();
    let mut redactions = Vec::new();
    let mut changed = false;
    let mut blocked = false;
    let sanitized_value = sanitize_json_value(
        value,
        policy,
        &mut reasons,
        &mut redactions,
        &mut changed,
        &mut blocked,
    );
    let decision = if blocked {
        OutboundPrivacyDecision::Block
    } else if changed {
        if policy.auto_redact_enabled {
            OutboundPrivacyDecision::RedactedAllow
        } else {
            push_unique(
                &mut reasons,
                "auto-redaction is disabled, so risky outbound content was blocked",
            );
            OutboundPrivacyDecision::Block
        }
    } else {
        OutboundPrivacyDecision::Allow
    };
    OutboundPrivacyJsonResult {
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

    fn fake_github_token() -> String {
        ["ghp", "_1234567890", "abcdefghijklmnopqrstuv"].concat()
    }

    #[test]
    fn outbound_text_allows_clean_content() {
        let result = check_outbound_text(
            "Ship a short reflection about autonomous planning quality.",
            &OutboundPrivacyPolicy::default(),
        );
        assert!(matches!(result.decision, OutboundPrivacyDecision::Allow));
        assert_eq!(
            result.sanitized_text,
            "Ship a short reflection about autonomous planning quality."
        );
    }

    #[test]
    fn outbound_text_redacts_secret_like_content() {
        let result = check_outbound_text(
            &format!("Token is {}", fake_openai_key()),
            &OutboundPrivacyPolicy::default(),
        );
        assert!(matches!(
            result.decision,
            OutboundPrivacyDecision::RedactedAllow
        ));
        assert!(result.sanitized_text.contains("[REDACTED_API_KEY]"));
    }

    #[test]
    fn outbound_text_redacts_pii_without_phrase_bound_identity_block() {
        let result = check_outbound_text(
            "The user email is jane@example.com, mention it publicly.",
            &OutboundPrivacyPolicy::default(),
        );
        assert!(matches!(
            result.decision,
            OutboundPrivacyDecision::RedactedAllow
        ));
        assert!(result.sanitized_text.contains("[EMAIL]"));
        assert!(
            result
                .reasons
                .iter()
                .any(|reason| reason.contains("PII-like"))
        );
    }

    #[test]
    fn outbound_json_redacts_nested_values() {
        let result = sanitize_outbound_json(
            &serde_json::json!({
                "message": "Call me at 555-123-4567",
                "body": { "token": fake_github_token() }
            }),
            &OutboundPrivacyPolicy::default(),
        );
        assert!(matches!(
            result.decision,
            OutboundPrivacyDecision::RedactedAllow
        ));
        assert!(
            result
                .sanitized_value
                .to_string()
                .contains("[REDACTED_API_KEY]")
                || result.sanitized_value.to_string().contains("[PHONE]")
        );
    }

    #[test]
    fn outbound_json_blocks_when_auto_redact_is_disabled() {
        let result = sanitize_outbound_json(
            &serde_json::json!({
                "message": "Reach me at jane@example.com"
            }),
            &OutboundPrivacyPolicy {
                auto_redact_enabled: false,
                public_learning_fenced: true,
            },
        );
        assert!(matches!(result.decision, OutboundPrivacyDecision::Block));
        assert!(
            result
                .reasons
                .iter()
                .any(|reason| reason.contains("auto-redaction is disabled"))
        );
    }
}
