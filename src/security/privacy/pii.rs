//! PII detection and redaction for logs and outputs
//!
//! Provides fast regex-based detection of personally identifiable information
//! including emails, phone numbers, SSNs, credit cards, and IP addresses.
//! Redaction is non-reversible — original data is permanently removed.

use once_cell::sync::Lazy;
use regex::Regex;

static EMAIL_RE: Lazy<Regex> =
    Lazy::new(|| Regex::new(r"\b[A-Za-z0-9._%+-]+@[A-Za-z0-9.-]+\.[A-Za-z]{2,}\b").unwrap());

static PHONE_RE: Lazy<Regex> = Lazy::new(|| {
    Regex::new(r"(?:\+?\d{1,3}[-.\s]?)?\(?\d{2,4}\)?[-.\s]?\d{3,4}[-.\s]?\d{4}").unwrap()
});

static SSN_RE: Lazy<Regex> = Lazy::new(|| Regex::new(r"\b\d{3}-\d{2}-\d{4}\b").unwrap());

static CREDIT_CARD_RE: Lazy<Regex> =
    Lazy::new(|| Regex::new(r"\b\d{4}[\s-]?\d{4}[\s-]?\d{4}[\s-]?\d{4}\b").unwrap());

static IPV4_RE: Lazy<Regex> = Lazy::new(|| {
    Regex::new(r"\b(?:(?:25[0-5]|2[0-4][0-9]|[01]?[0-9][0-9]?)\.){3}(?:25[0-5]|2[0-4][0-9]|[01]?[0-9][0-9]?)\b").unwrap()
});

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct AddressablePiiTarget {
    pub kind: String,
    pub value: String,
}

fn char_before(text: &str, idx: usize) -> Option<char> {
    text[..idx].chars().next_back()
}

fn char_after(text: &str, idx: usize) -> Option<char> {
    text[idx..].chars().next()
}

fn is_numeric_token_prefix_char(ch: char) -> bool {
    ch.is_ascii_digit() || matches!(ch, '.' | '_')
}

fn is_numeric_token_suffix_char(ch: char) -> bool {
    ch.is_ascii_digit() || ch == '_'
}

fn is_decimal_literal(value: &str) -> bool {
    let trimmed = value.trim();
    let Some((left, right)) = trimmed.split_once('.') else {
        return false;
    };
    !left.is_empty()
        && !right.is_empty()
        && left.chars().all(|ch| ch.is_ascii_digit())
        && right.chars().all(|ch| ch.is_ascii_digit())
}

fn is_likely_phone_match(source: &str, start: usize, end: usize) -> bool {
    if char_before(source, start).is_some_and(is_numeric_token_prefix_char)
        || char_after(source, end).is_some_and(is_numeric_token_suffix_char)
    {
        return false;
    }

    let value = &source[start..end];
    if is_decimal_literal(value) {
        return false;
    }

    true
}

fn redact_phone_numbers(text: &str) -> String {
    let mut output = String::with_capacity(text.len());
    let mut cursor = 0usize;
    let mut changed = false;

    for matched in PHONE_RE.find_iter(text) {
        if !is_likely_phone_match(text, matched.start(), matched.end()) {
            continue;
        }
        output.push_str(&text[cursor..matched.start()]);
        output.push_str("[PHONE]");
        cursor = matched.end();
        changed = true;
    }

    if !changed {
        return text.to_string();
    }
    output.push_str(&text[cursor..]);
    output
}

#[derive(Clone, Copy)]
struct TextSpan {
    start: usize,
    end: usize,
}

fn spans_overlap(left: TextSpan, right: TextSpan) -> bool {
    left.start < right.end && right.start < left.end
}

fn push_addressable_target(targets: &mut Vec<AddressablePiiTarget>, kind: &str, value: &str) {
    let value = value
        .trim_matches(|ch: char| matches!(ch, '.' | ',' | ';' | ':' | ')' | ']' | '}' | '>'))
        .trim();
    if value.is_empty()
        || targets
            .iter()
            .any(|target| target.kind == kind && target.value == value)
    {
        return;
    }
    targets.push(AddressablePiiTarget {
        kind: kind.to_string(),
        value: value.to_string(),
    });
}

pub fn extract_addressable_pii_targets(text: &str) -> Vec<AddressablePiiTarget> {
    let high_risk_spans = CREDIT_CARD_RE
        .find_iter(text)
        .chain(SSN_RE.find_iter(text))
        .map(|matched| TextSpan {
            start: matched.start(),
            end: matched.end(),
        })
        .collect::<Vec<_>>();
    let overlaps_high_risk = |span: TextSpan| {
        high_risk_spans
            .iter()
            .any(|high_risk| spans_overlap(span, *high_risk))
    };

    let mut targets = Vec::new();
    for matched in EMAIL_RE.find_iter(text) {
        push_addressable_target(&mut targets, "email_address", matched.as_str());
    }
    for matched in PHONE_RE.find_iter(text) {
        let span = TextSpan {
            start: matched.start(),
            end: matched.end(),
        };
        if overlaps_high_risk(span) || !is_likely_phone_match(text, matched.start(), matched.end())
        {
            continue;
        }
        push_addressable_target(&mut targets, "phone_number", matched.as_str());
    }
    for matched in IPV4_RE.find_iter(text) {
        let span = TextSpan {
            start: matched.start(),
            end: matched.end(),
        };
        if overlaps_high_risk(span) {
            continue;
        }
        push_addressable_target(&mut targets, "network_host", matched.as_str());
    }
    targets.truncate(64);
    targets
}

/// PII redactor with configurable pattern toggles
pub struct PiiRedactor {
    pub redact_emails: bool,
    pub redact_phones: bool,
    pub redact_ssn: bool,
    pub redact_credit_cards: bool,
    pub redact_ips: bool,
}

impl Default for PiiRedactor {
    fn default() -> Self {
        Self::new()
    }
}

impl PiiRedactor {
    /// Create a new PII redactor with all detection enabled
    pub fn new() -> Self {
        Self {
            redact_emails: true,
            redact_phones: true,
            redact_ssn: true,
            redact_credit_cards: true,
            redact_ips: true,
        }
    }

    /// Redact PII from text. Non-reversible.
    pub fn redact(&self, text: &str) -> String {
        let mut result = text.to_string();

        if self.redact_ssn {
            result = SSN_RE.replace_all(&result, "[SSN]").to_string();
        }
        if self.redact_credit_cards {
            result = CREDIT_CARD_RE.replace_all(&result, "[CARD]").to_string();
        }
        if self.redact_emails {
            result = EMAIL_RE.replace_all(&result, "[EMAIL]").to_string();
        }
        if self.redact_phones {
            result = redact_phone_numbers(&result);
        }
        if self.redact_ips {
            result = IPV4_RE.replace_all(&result, "[IP]").to_string();
        }

        result
    }
}

/// Convenience function for quick redaction with all patterns enabled
pub fn redact_pii(text: &str) -> String {
    PiiRedactor::new().redact(text)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_email_redaction() {
        assert_eq!(
            redact_pii("Contact john.doe@example.com for details"),
            "Contact [EMAIL] for details"
        );
    }

    #[test]
    fn test_phone_redaction() {
        assert_eq!(redact_pii("Call 555-123-4567"), "Call [PHONE]");
        assert_eq!(redact_pii("Call 555-123-4567."), "Call [PHONE].");
        assert!(redact_pii("+1 (555) 987-6543").contains("[PHONE]"));
        assert!(redact_pii("Call 555.123.4567").contains("[PHONE]"));
        assert_eq!(redact_pii("Call 5551234567"), "Call [PHONE]");
    }

    #[test]
    fn test_ssn_redaction() {
        assert_eq!(redact_pii("SSN: 123-45-6789"), "SSN: [SSN]");
    }

    #[test]
    fn test_credit_card_redaction() {
        assert_eq!(redact_pii("Card: 4111 1111 1111 1111"), "Card: [CARD]");
        assert!(redact_pii("4111-1111-1111-1111").contains("[CARD]"));
    }

    #[test]
    fn test_ip_redaction() {
        assert_eq!(redact_pii("Server at 192.168.1.100"), "Server at [IP]");
    }

    #[test]
    fn test_multiple_pii() {
        let input = "Email: user@test.com, Phone: 555-123-4567, IP: 10.0.0.1";
        let result = redact_pii(input);
        assert!(result.contains("[EMAIL]"));
        assert!(result.contains("[PHONE]"));
        assert!(result.contains("[IP]"));
        assert!(!result.contains("user@test.com"));
        assert!(!result.contains("555-123"));
        assert!(!result.contains("10.0.0.1"));
    }

    #[test]
    fn test_no_false_positives_on_clean_text() {
        let input = "Hello, how are you today?";
        assert_eq!(redact_pii(input), input);
    }

    #[test]
    fn test_cost_decimal_telemetry_is_not_phone_redacted() {
        for input in [
            r#""cost_usd": 0.012934658880000002"#,
            r#""cost_usd":0.21772148112000006"#,
            r#""cost_usd": 0.29229803162999984"#,
            r#""request_count":97,"cost_usd":0.21772148112000006"#,
        ] {
            assert_eq!(redact_pii(input), input);
        }
    }

    #[test]
    fn test_selective_redaction() {
        let mut redactor = PiiRedactor::new();
        redactor.redact_emails = false;
        let result = redactor.redact("Email: user@test.com, SSN: 123-45-6789");
        assert!(result.contains("user@test.com"));
        assert!(result.contains("[SSN]"));
    }

    #[test]
    fn addressable_pii_targets_include_contact_and_host_values_but_not_high_risk_values() {
        let targets = extract_addressable_pii_targets(
            "Mail jane@example.com, text +1 555 123 4567, inspect 192.168.1.20, keep SSN 123-45-6789 and card 4111 1111 1111 1111 private.",
        );
        let values = targets
            .iter()
            .map(|target| (target.kind.as_str(), target.value.as_str()))
            .collect::<Vec<_>>();

        assert!(values.contains(&("email_address", "jane@example.com")));
        assert!(values.contains(&("phone_number", "+1 555 123 4567")));
        assert!(values.contains(&("network_host", "192.168.1.20")));
        assert!(!values
            .iter()
            .any(|(_, value)| *value == "123-45-6789" || *value == "4111 1111 1111 1111"));
    }
}
