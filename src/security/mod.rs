//! Security module — defenses against prompt injection, credential leakage,
//! and data exposure.
//!
//! Layered architecture (see `docs/plans/implement-all-phases-temporal-naur.md`):
//! - `normalize` — Unicode canonicalization used by every detection surface so
//!   attackers can't evade checks via homoglyphs or invisible characters.
//! - `intent_classifier` (Phase 2) — LLM emits a fixed vocabulary describing
//!   user-message intent; a deterministic policy engine turns that into a
//!   verdict. No hardcoded attacker phrases.
//! - `trust_boundary` — envelope wrapper for untrusted external content.
//! - `output_guard` (Phase 5) — second-pass LLM check for responses that
//!   touched external content.
//! - `tool_args_guard` (Phase 4) — structural SSRF/path-escape guard on
//!   outward-facing tool arguments.
//! - `abuse_tracker` (Phase 6) — per-source sliding window requiring admin
//!   approval after repeated trips.
//! - `capabilities`, `skill_review` — capability vocabulary and semantic
//!   review of imported skills/extension packs.
//! - Secret redaction (this file) — structural patterns matching real wire
//!   formats (not attacker phrasing) plus a high-entropy opaque-token shape
//!   detector. Retained because it encodes token formats, not anticipated
//!   wording.

#[path = "abuse/abuse_tracker.rs"]
pub mod abuse_tracker;
#[path = "guards/action_guard.rs"]
pub mod action_guard;
#[path = "review/capabilities.rs"]
pub mod capabilities;
#[path = "classification/embedding_classifier.rs"]
pub mod embedding_classifier;
#[path = "classification/intent_classifier.rs"]
pub mod intent_classifier;
#[path = "model/model_hardening.rs"]
pub mod model_hardening;
#[path = "privacy/model_input.rs"]
pub mod model_input;
#[path = "privacy/normalize.rs"]
pub mod normalize;
#[path = "guards/outbound.rs"]
pub mod outbound;
#[path = "privacy/pii.rs"]
pub mod pii;
#[path = "review/skill_review.rs"]
pub mod skill_review;
#[path = "guards/tool_args_guard.rs"]
pub mod tool_args_guard;
#[path = "boundary/trust_boundary.rs"]
pub mod trust_boundary;

pub use action_guard::ActionGuard;
pub use model_hardening::protect_system_prompt;
#[allow(unused_imports)]
pub use model_input::{
    render_model_input_fallback, sanitize_model_input_json, sanitize_model_input_text,
    CurrentChatPiiPolicy, ModelInputContext, ModelInputPrivacyDecision,
    ModelInputPrivacyJsonResult, ModelInputPrivacyMode, ModelInputPrivacyTextResult,
    ModelPrivacyConfig,
};
pub use normalize::normalize_for_analysis;
pub use outbound::{
    check_outbound_text, format_outbound_privacy_block, sanitize_outbound_json,
    OutboundPrivacyDecision, OutboundPrivacyPolicy,
};
pub use pii::redact_pii;
pub use trust_boundary::{
    canonical_capabilities, redact_json_secrets, sanitize_input_schema, sanitize_untrusted_html,
    sanitize_untrusted_output, scan_untrusted_text,
};

#[derive(Debug, Clone, serde::Serialize, serde::Deserialize, PartialEq, Eq, thiserror::Error)]
#[serde(tag = "type", rename_all = "snake_case")]
pub enum SecurityError {
    #[error("ERR/security/invalid_input: {message}")]
    InvalidInput { message: String },
    #[error("ERR/security/permission_denied: {message}")]
    PermissionDenied { code: String, message: String },
    #[error("ERR/security/tool_argument_denied: tool argument denied ({reason_code})")]
    ToolArgumentDenied { reason_code: String },
    #[error("ERR/security/secret_detected: {message}")]
    SecretDetected { message: String },
    #[error("ERR/security/failed: {message}")]
    Failed { message: String },
}

impl SecurityError {
    pub fn tool_argument_denied(reason_code: impl Into<String>) -> Self {
        Self::ToolArgumentDenied {
            reason_code: reason_code.into(),
        }
    }

    pub fn code(&self) -> String {
        match self {
            Self::InvalidInput { .. } => "security_invalid_input".to_string(),
            Self::PermissionDenied { code, .. } if !code.trim().is_empty() => {
                format!("security_{}", code.trim())
            }
            Self::PermissionDenied { .. } => "security_permission_denied".to_string(),
            Self::ToolArgumentDenied { reason_code } => {
                format!("security_tool_argument_denied_{}", reason_code)
            }
            Self::SecretDetected { .. } => "security_secret_detected".to_string(),
            Self::Failed { .. } => "security_failed".to_string(),
        }
    }

    pub fn into_anyhow(self) -> anyhow::Error {
        anyhow::Error::new(self)
    }
}

use once_cell::sync::Lazy;
use regex::Regex;

pub fn constant_time_eq(left: &[u8], right: &[u8]) -> bool {
    let max_len = left.len().max(right.len());
    let mut diff = left.len() ^ right.len();
    for idx in 0..max_len {
        let a = left.get(idx).copied().unwrap_or(0);
        let b = right.get(idx).copied().unwrap_or(0);
        diff |= (a ^ b) as usize;
    }
    diff == 0
}

/// Marker layer used by the agent for secret scrubbing on outbound text.
///
/// Inbound intent classification now lives in `intent_classifier` and is
/// invoked directly by the agent; output-side risk classification lives in
/// `output_guard`. This struct retains `filter_output` because it encodes
/// the secret-format scrubber which catches real token wire formats
/// regardless of classifier availability.
#[derive(Default, Clone, Copy)]
pub struct SecurityGuard;

impl SecurityGuard {
    pub fn new(_strict_mode: bool) -> Self {
        Self
    }

    /// Scrub recognizable secret wire formats from outbound text.
    ///
    /// This is a structural detector — it recognizes concrete token shapes
    /// (OpenAI `sk-…`, GitHub PATs, bearer headers, high-entropy opaque
    /// tokens, secret-bearing URL query params). It does not attempt to
    /// detect semantic leakage; that is the job of `output_guard`.
    pub fn filter_output(&self, output: &str) -> FilteredOutput {
        let redacted = redact_secret_input(output);
        let text = if redacted.uses_specific_api_key_placeholder() {
            redacted
                .text
                .replace("[REDACTED_SECRET]", "[REDACTED_API_KEY]")
        } else {
            redacted.text.clone()
        };
        let is_clean = redacted.redactions.is_empty();
        FilteredOutput {
            text,
            redactions: redacted.redactions,
            _is_clean: is_clean,
        }
    }
}

#[derive(Debug, Clone, PartialEq)]
pub enum SecretInputType {
    PrivateKeyMaterial,
    ApiKeyOrToken,
    PaymentCredential,
}

#[derive(Debug, Clone, PartialEq)]
pub struct SecretRedactionResult {
    pub text: String,
    pub redactions: Vec<String>,
    pub kinds: Vec<SecretInputType>,
}

impl SecretRedactionResult {
    pub fn had_secret(&self) -> bool {
        !self.redactions.is_empty()
    }

    pub fn primary_kind(&self) -> Option<SecretInputType> {
        self.kinds.first().cloned()
    }

    pub fn uses_specific_api_key_placeholder(&self) -> bool {
        self.primary_kind() == Some(SecretInputType::ApiKeyOrToken)
            && self.redactions.iter().any(|redaction| {
                redaction.starts_with("openai_key ")
                    || redaction.starts_with("github_token ")
                    || redaction.starts_with("notion_token ")
                    || redaction.starts_with("google_api_key ")
                    || redaction.starts_with("google_oauth_token ")
                    || redaction.starts_with("slack_token ")
                    || redaction.starts_with("moltbook_token ")
            })
            && !self.redactions.iter().any(|redaction| {
                redaction.starts_with("bearer_token ")
                    || redaction.starts_with("secret_assignment ")
                    || redaction.starts_with("env_secret_assignment ")
                    || redaction.starts_with("secret_query_param ")
                    || redaction.starts_with("opaque_token ")
            })
    }

    pub fn is_mostly_secret_payload(&self) -> bool {
        if !self.had_secret() {
            return false;
        }
        let placeholder_pattern = Regex::new(r"\[REDACTED_[A-Z_]+\]").unwrap();
        let stripped = placeholder_pattern.replace_all(&self.text, " ");
        let meaningful: String = stripped
            .chars()
            .filter(|ch| ch.is_alphanumeric() || ch.is_whitespace())
            .collect();
        meaningful.trim().chars().count() < 24
    }
}

static PRIVATE_KEY_BLOCK_PATTERN: Lazy<Regex> = Lazy::new(|| {
    Regex::new(
        r"(?s)-----BEGIN (?:RSA |EC |OPENSSH |PGP )?PRIVATE KEY-----.*?-----END (?:RSA |EC |OPENSSH |PGP )?PRIVATE KEY-----",
    )
    .unwrap()
});
static UUID_PATTERN: Lazy<Regex> = Lazy::new(|| {
    Regex::new(r"(?i)^[0-9a-f]{8}-[0-9a-f]{4}-[0-9a-f]{4}-[0-9a-f]{4}-[0-9a-f]{12}$").unwrap()
});
static CERT_BLOCK_PATTERN: Lazy<Regex> = Lazy::new(|| {
    Regex::new(r"(?s)-----BEGIN CERTIFICATE-----.*?-----END CERTIFICATE-----").unwrap()
});
static OPENAI_KEY_PATTERN: Lazy<Regex> =
    Lazy::new(|| Regex::new(r"\bsk-[A-Za-z0-9]{20,}\b").unwrap());
static GITHUB_PAT_PATTERN: Lazy<Regex> =
    Lazy::new(|| Regex::new(r"\bghp_[A-Za-z0-9]{30,}\b").unwrap());
static NOTION_TOKEN_PATTERN: Lazy<Regex> =
    Lazy::new(|| Regex::new(r"\bsecret_[A-Za-z0-9]{20,}\b").unwrap());
static GOOGLE_API_KEY_PATTERN: Lazy<Regex> =
    Lazy::new(|| Regex::new(r"\bAIza[0-9A-Za-z\-_]{20,}\b").unwrap());
static GOOGLE_OAUTH_TOKEN_PATTERN: Lazy<Regex> =
    Lazy::new(|| Regex::new(r"\bya29\.[0-9A-Za-z\-_]+\b").unwrap());
static SLACK_TOKEN_PATTERN: Lazy<Regex> =
    Lazy::new(|| Regex::new(r"\bxox[baprs]-[A-Za-z0-9-]{10,}\b").unwrap());
static MOLTBOOK_TOKEN_PATTERN: Lazy<Regex> =
    Lazy::new(|| Regex::new(r"\bmoltbook_sk_[A-Za-z0-9_-]{20,}\b").unwrap());
static BEARER_TOKEN_PATTERN: Lazy<Regex> =
    Lazy::new(|| Regex::new(r"(?i)\bbearer\s+[A-Za-z0-9._-]{20,}\b").unwrap());
static SECRET_ASSIGNMENT_PATTERN: Lazy<Regex> = Lazy::new(|| {
    Regex::new(
        r#"(?i)(\b(?:api[_-]?key|secret|token|password|client_secret)\b\s*[:=]\s*['"]?)[A-Za-z0-9_\-./+=]{16,}(['"]?)"#,
    )
    .unwrap()
});
static LABELED_CREDENTIAL_VALUE_PATTERN: Lazy<Regex> = Lazy::new(|| {
    Regex::new(
        r#"(?ix)
        (?P<prefix>
            \b(?:
                api\s*[-_]?\s*key
                | access\s*[-_]?\s*token
                | auth(?:entication)?\s*[-_]?\s*token
                | bearer\s*[-_]?\s*token
                | client\s*[-_]?\s*secret
                | secret\s*[-_]?\s*key
                | password
                | passwd
                | pwd
                | credential
            )\b
            (?:
                \s*(?:is|as|for|value|=|:|-)\s*
            ){0,4}
            ['"]?
        )
        (?P<value>[A-Za-z0-9][A-Za-z0-9_\-./+=]{7,})
        (?P<suffix>['"]?)
        "#,
    )
    .unwrap()
});
static ENV_STYLE_SECRET_ASSIGNMENT_PATTERN: Lazy<Regex> = Lazy::new(|| {
    Regex::new(r"(\b[A-Z0-9_]*(?:KEY|TOKEN|SECRET|PASSWORD)[A-Z0-9_]*\s*=\s*)[^\s]{16,}").unwrap()
});
static URL_SECRET_QUERY_PATTERN: Lazy<Regex> = Lazy::new(|| {
    Regex::new(
        r"(?i)([?&](?:access_token|token|sig|signature|api[_-]?key|key|auth|password|client_secret)=)[^&\s]+",
    )
    .unwrap()
});
static PAYMENT_NUMBER_PATTERN: Lazy<Regex> =
    Lazy::new(|| Regex::new(r"\b(?:\d{12,19}|\d{3,4}(?:[\s.-]\d{3,6}){2,4})\b").unwrap());
static SHORT_NUMERIC_CODE_PATTERN: Lazy<Regex> = Lazy::new(|| Regex::new(r"\b\d{3,4}\b").unwrap());
const OPAQUE_TOKEN_MIN_CHARS: usize = 20;
const OPAQUE_TOKEN_ENTROPY_BITS_PER_CHAR: f64 = 3.0;
const PAYMENT_CODE_PROXIMITY_BYTES: usize = 96;

fn push_secret_kind(kinds: &mut Vec<SecretInputType>, kind: SecretInputType) {
    if !kinds.contains(&kind) {
        kinds.push(kind);
    }
}

fn apply_secret_redaction(
    text: &mut String,
    redactions: &mut Vec<String>,
    kinds: &mut Vec<SecretInputType>,
    pattern: &Regex,
    replacement: &str,
    label: &str,
    kind: SecretInputType,
) {
    let count = pattern.find_iter(text).count();
    if count == 0 {
        return;
    }
    *text = pattern.replace_all(text, replacement).to_string();
    redactions.push(format!("{} x{}", label, count));
    push_secret_kind(kinds, kind);
}

fn opaque_token_shape_char(ch: char) -> bool {
    ch.is_ascii_alphanumeric() || matches!(ch, '_' | '-' | '.' | '=' | '+')
}

fn opaque_token_has_secret_signal(value: &str) -> bool {
    value
        .chars()
        .any(|ch| ch.is_ascii_digit() || matches!(ch, '_' | '=' | '+'))
}

fn shannon_entropy_bits_per_char(value: &str) -> f64 {
    let mut counts: std::collections::HashMap<char, usize> = std::collections::HashMap::new();
    let mut total = 0usize;
    for ch in value.chars() {
        *counts.entry(ch).or_insert(0) += 1;
        total += 1;
    }
    if total == 0 {
        return 0.0;
    }
    counts
        .values()
        .map(|count| {
            let p = *count as f64 / total as f64;
            -p * p.log2()
        })
        .sum()
}

fn is_lowercase_word_segment(value: &str) -> bool {
    value.len() >= 2 && value.chars().all(|ch| ch.is_ascii_lowercase())
}

fn is_safe_identifier_version_segment(value: &str) -> bool {
    value
        .strip_prefix('v')
        .filter(|rest| !rest.is_empty() && rest.chars().all(|ch| ch.is_ascii_digit()))
        .is_some()
}

fn is_uuid_hex_group(value: &str) -> bool {
    matches!(value.len(), 4 | 8 | 12) && value.chars().all(|ch| ch.is_ascii_hexdigit())
}

fn is_likely_identifier_slug(value: &str) -> bool {
    let trimmed = value.trim();
    if UUID_PATTERN.is_match(trimmed) {
        return true;
    }
    if !trimmed.contains('_') && !trimmed.contains('-') {
        return false;
    }
    if !trimmed
        .chars()
        .all(|ch| ch.is_ascii_lowercase() || ch.is_ascii_digit() || matches!(ch, '_' | '-'))
    {
        return false;
    }

    let segments: Vec<&str> = trimmed
        .split(['_', '-'])
        .filter(|segment| !segment.is_empty())
        .collect();
    if segments.len() < 2 {
        return false;
    }
    let word_segments = segments
        .iter()
        .filter(|segment| is_lowercase_word_segment(segment))
        .count();
    word_segments >= 2
        && segments.iter().all(|segment| {
            is_lowercase_word_segment(segment)
                || is_safe_identifier_version_segment(segment)
                || is_uuid_hex_group(segment)
        })
}

fn is_lowercase_identifier_version_part(value: &str) -> bool {
    if value.is_empty() {
        return false;
    }
    is_lowercase_word_segment(value)
        || value.chars().all(|ch| ch.is_ascii_digit())
        || is_safe_identifier_version_segment(value)
        || is_uuid_hex_group(value)
        || (value.len() <= 12
            && value
                .chars()
                .all(|ch| ch.is_ascii_lowercase() || ch.is_ascii_digit())
            && value.chars().any(|ch| ch.is_ascii_lowercase())
            && value.chars().any(|ch| ch.is_ascii_digit()))
}

fn is_likely_public_identifier_version(value: &str) -> bool {
    let trimmed = value.trim();
    if !trimmed.chars().any(|ch| matches!(ch, '-' | '_' | '.')) {
        return false;
    }
    if !trimmed
        .chars()
        .all(|ch| ch.is_ascii_lowercase() || ch.is_ascii_digit() || matches!(ch, '-' | '_' | '.'))
    {
        return false;
    }
    let segments: Vec<&str> = trimmed
        .split(['-', '_', '.'])
        .filter(|segment| !segment.is_empty())
        .collect();
    if segments.len() < 2 {
        return false;
    }
    let word_segments = segments
        .iter()
        .filter(|segment| is_lowercase_word_segment(segment))
        .count();
    word_segments >= 1
        && segments
            .iter()
            .all(|segment| is_lowercase_identifier_version_part(segment))
}

fn is_likely_public_slug_filename(value: &str) -> bool {
    let trimmed = value.trim();
    let Some((stem, extension)) = trimmed.rsplit_once('.') else {
        return false;
    };
    if stem.is_empty()
        || extension.len() < 2
        || extension.len() > 8
        || !extension
            .chars()
            .all(|ch| ch.is_ascii_lowercase() || ch.is_ascii_digit())
    {
        return false;
    }
    is_likely_identifier_slug(stem)
}

fn is_likely_namespaced_public_identifier(source: &str, span: RedactionSpan) -> bool {
    if char_before(source, span.start) != Some('/') {
        return false;
    }
    let namespace_start = source[..span.start.saturating_sub(1)]
        .rfind(|ch: char| !opaque_token_shape_char(ch))
        .map(|idx| idx + 1)
        .unwrap_or(0);
    let namespace = source[namespace_start..span.start.saturating_sub(1)].trim();
    if namespace.len() < 2
        || namespace.len() > 48
        || !namespace.chars().any(|ch| ch.is_ascii_alphabetic())
        || !namespace
            .chars()
            .all(|ch| ch.is_ascii_alphanumeric() || matches!(ch, '-' | '_' | '.'))
    {
        return false;
    }

    let value = source[span.start..span.end].trim();
    if value.contains('=') || value.contains('+') {
        return false;
    }
    if is_likely_identifier_slug(value)
        || is_likely_public_identifier_version(value)
        || is_likely_public_slug_filename(value)
    {
        return true;
    }
    if !value.chars().any(|ch| matches!(ch, '-' | '_' | '.')) {
        return false;
    }
    let segments = value
        .split(['-', '_', '.'])
        .filter(|segment| !segment.is_empty())
        .collect::<Vec<_>>();
    segments.len() >= 2
        && segments
            .iter()
            .any(|segment| segment.chars().any(|ch| ch.is_ascii_alphabetic()))
        && segments
            .iter()
            .all(|segment| segment.chars().all(|ch| ch.is_ascii_alphanumeric()))
}

fn is_identifier_segment(value: &str) -> bool {
    let mut chars = value.chars();
    let Some(first) = chars.next() else {
        return false;
    };
    (first.is_ascii_alphabetic() || first == '_')
        && chars.all(|ch| ch.is_ascii_alphanumeric() || ch == '_')
}

fn is_likely_code_expression_token(value: &str) -> bool {
    fn is_member_expression(value: &str) -> bool {
        value.split('.').all(is_identifier_segment)
    }

    let trimmed = value.trim();
    if let Some((left, right)) = trimmed.split_once('=') {
        return is_member_expression(left) && is_member_expression(right);
    }
    trimmed.contains('.')
        && trimmed.contains('_')
        && trimmed.split('.').count() >= 2
        && is_member_expression(trimmed)
}

fn is_opaque_token_shape(value: &str) -> bool {
    let trimmed = value.trim();
    trimmed.chars().count() >= OPAQUE_TOKEN_MIN_CHARS
        && !is_redaction_placeholder_token(trimmed)
        && !trimmed.chars().any(char::is_whitespace)
        && trimmed.chars().all(opaque_token_shape_char)
        && !is_likely_identifier_slug(trimmed)
        && !is_likely_public_identifier_version(trimmed)
        && !is_likely_public_slug_filename(trimmed)
        && !is_likely_code_expression_token(trimmed)
        && opaque_token_has_secret_signal(trimmed)
        && shannon_entropy_bits_per_char(trimmed) >= OPAQUE_TOKEN_ENTROPY_BITS_PER_CHAR
}

fn is_redaction_placeholder_token(value: &str) -> bool {
    value.starts_with("REDACTED_") && value.chars().all(|ch| ch.is_ascii_uppercase() || ch == '_')
}

fn is_untrusted_output_envelope_token(value: &str) -> bool {
    value.starts_with("UNTRUSTED_")
        && value.ends_with("_OUTPUT")
        && value
            .chars()
            .all(|ch| ch.is_ascii_uppercase() || ch.is_ascii_digit() || ch == '_')
}

fn is_untrusted_output_envelope_span(source: &str, span: RedactionSpan) -> bool {
    if !is_untrusted_output_envelope_token(&source[span.start..span.end]) {
        return false;
    }
    if source.as_bytes().get(span.end) != Some(&b']') {
        return false;
    }

    let prefix = &source[..span.start];
    let marker_start = if prefix.ends_with("[/") {
        span.start.saturating_sub(2)
    } else if prefix.ends_with('[') {
        span.start.saturating_sub(1)
    } else {
        return false;
    };
    let marker_end = span.end + 1;
    let before_marker = source[..marker_start].chars().next_back();
    let after_marker = source[marker_end..].chars().next();

    before_marker.is_none_or(|ch| matches!(ch, '\n' | '\r'))
        && after_marker.is_none_or(|ch| matches!(ch, '\n' | '\r'))
}

fn apply_opaque_token_redaction(
    text: &mut String,
    redactions: &mut Vec<String>,
    kinds: &mut Vec<SecretInputType>,
) {
    let source = text.clone();
    let mut redacted = String::with_capacity(source.len());
    let mut last = 0usize;
    let mut run_start: Option<usize> = None;
    let mut count = 0usize;

    for (idx, ch) in source.char_indices() {
        if opaque_token_shape_char(ch) {
            if run_start.is_none() {
                run_start = Some(idx);
            }
            continue;
        }

        if let Some(start) = run_start.take() {
            let candidate = &source[start..idx];
            let span = RedactionSpan { start, end: idx };
            if is_opaque_token_shape(candidate)
                && !is_likely_namespaced_public_identifier(&source, span)
                && !is_untrusted_output_envelope_span(&source, span)
            {
                redacted.push_str(&source[last..start]);
                redacted.push_str("[REDACTED_SECRET]");
                last = idx;
                count += 1;
            }
        }
    }

    if let Some(start) = run_start {
        let candidate = &source[start..];
        let span = RedactionSpan {
            start,
            end: source.len(),
        };
        if is_opaque_token_shape(candidate)
            && !is_likely_namespaced_public_identifier(&source, span)
            && !is_untrusted_output_envelope_span(&source, span)
        {
            redacted.push_str(&source[last..start]);
            redacted.push_str("[REDACTED_SECRET]");
            last = source.len();
            count += 1;
        }
    }

    if count == 0 {
        return;
    }
    redacted.push_str(&source[last..]);
    *text = redacted;
    redactions.push(format!("opaque_token x{}", count));
    push_secret_kind(kinds, SecretInputType::ApiKeyOrToken);
}

fn is_contextual_credential_value(value: &str) -> bool {
    let trimmed = value.trim_matches(|ch: char| matches!(ch, '\'' | '"' | '.' | ',' | ';'));
    trimmed.chars().count() >= 8
        && !is_redaction_placeholder_token(trimmed)
        && !trimmed.starts_with("http://")
        && !trimmed.starts_with("https://")
        && !trimmed.chars().any(char::is_whitespace)
        && trimmed.chars().all(opaque_token_shape_char)
        && (trimmed.chars().count() >= 16
            || trimmed.chars().any(|ch| ch.is_ascii_digit())
            || trimmed
                .chars()
                .any(|ch| matches!(ch, '_' | '-' | '.' | '=' | '+' | '/')))
}

fn apply_labeled_credential_value_redaction(
    text: &mut String,
    redactions: &mut Vec<String>,
    kinds: &mut Vec<SecretInputType>,
) {
    let source = text.clone();
    let mut redacted = String::with_capacity(source.len());
    let mut cursor = 0usize;
    let mut count = 0usize;

    for captures in LABELED_CREDENTIAL_VALUE_PATTERN.captures_iter(&source) {
        let Some(full_match) = captures.get(0) else {
            continue;
        };
        let Some(value_match) = captures.name("value") else {
            continue;
        };
        if full_match.start() < cursor || !is_contextual_credential_value(value_match.as_str()) {
            continue;
        }
        redacted.push_str(&source[cursor..value_match.start()]);
        redacted.push_str("[REDACTED_SECRET]");
        cursor = value_match.end();
        count += 1;
    }

    if count == 0 {
        return;
    }
    redacted.push_str(&source[cursor..]);
    *text = redacted;
    redactions.push(format!("labeled_credential x{}", count));
    push_secret_kind(kinds, SecretInputType::ApiKeyOrToken);
}

#[derive(Clone, Copy)]
struct RedactionSpan {
    start: usize,
    end: usize,
}

fn digit_count(value: &str) -> usize {
    value.chars().filter(|ch| ch.is_ascii_digit()).count()
}

fn digits_only(value: &str) -> String {
    value.chars().filter(|ch| ch.is_ascii_digit()).collect()
}

fn char_before(text: &str, idx: usize) -> Option<char> {
    text[..idx].chars().next_back()
}

fn char_after(text: &str, idx: usize) -> Option<char> {
    text[idx..].chars().next()
}

fn numeric_literal_prefix_char(ch: char) -> bool {
    ch.is_ascii_digit() || matches!(ch, '.' | '_')
}

fn numeric_literal_suffix_char(ch: char) -> bool {
    ch.is_ascii_digit() || ch == '_'
}

fn decimal_numeric_literal(value: &str) -> bool {
    let trimmed = value.trim();
    let Some((left, right)) = trimmed.split_once('.') else {
        return false;
    };
    !left.is_empty()
        && !right.is_empty()
        && !right.contains('.')
        && left.chars().all(|ch| ch.is_ascii_digit())
        && right.chars().all(|ch| ch.is_ascii_digit())
}

fn numeric_span_is_decimal_literal(source: &str, span: RedactionSpan) -> bool {
    if char_before(source, span.start).is_some_and(numeric_literal_prefix_char)
        || char_after(source, span.end).is_some_and(numeric_literal_suffix_char)
    {
        return true;
    }
    decimal_numeric_literal(&source[span.start..span.end])
}

fn luhn_valid(digits: &str) -> bool {
    if digits.len() < 8 || !digits.chars().all(|ch| ch.is_ascii_digit()) {
        return false;
    }
    let mut sum = 0u32;
    let mut double = false;
    for ch in digits.chars().rev() {
        let Some(mut value) = ch.to_digit(10) else {
            return false;
        };
        if double {
            value *= 2;
            if value > 9 {
                value -= 9;
            }
        }
        sum += value;
        double = !double;
    }
    sum.is_multiple_of(10)
}

fn spans_overlap(left: RedactionSpan, right: RedactionSpan) -> bool {
    left.start < right.end && right.start < left.end
}

fn spans_are_near(left: RedactionSpan, right: RedactionSpan, max_distance: usize) -> bool {
    if spans_overlap(left, right) {
        return true;
    }
    let distance = if left.end <= right.start {
        right.start.saturating_sub(left.end)
    } else {
        left.start.saturating_sub(right.end)
    };
    distance <= max_distance
}

fn span_contains_index(span: RedactionSpan, index: usize) -> bool {
    span.start <= index && index < span.end
}

fn index_is_in_any_span(spans: &[RedactionSpan], index: usize) -> bool {
    spans.iter().any(|span| span_contains_index(*span, index))
}

fn grouped_numeric_separator(ch: char) -> bool {
    ch.is_ascii_whitespace() || matches!(ch, '.' | '-' | '(' | ')' | '+')
}

fn previous_group_digit_index(source: &str, span: RedactionSpan) -> Option<usize> {
    let mut idx = span.start;
    let mut saw_separator = false;
    while let Some((prev_idx, ch)) = source[..idx].char_indices().next_back() {
        if grouped_numeric_separator(ch) {
            saw_separator = true;
            idx = prev_idx;
            continue;
        }
        return (saw_separator && ch.is_ascii_digit()).then_some(prev_idx);
    }
    None
}

fn next_group_digit_index(source: &str, span: RedactionSpan) -> Option<usize> {
    let mut idx = span.end;
    let mut saw_separator = false;
    while idx < source.len() {
        let ch = source[idx..].chars().next()?;
        if grouped_numeric_separator(ch) {
            saw_separator = true;
            idx += ch.len_utf8();
            continue;
        }
        return (saw_separator && ch.is_ascii_digit()).then_some(idx);
    }
    None
}

fn short_numeric_code_is_grouped_number_part(
    source: &str,
    code_span: RedactionSpan,
    payment_number_spans: &[RedactionSpan],
) -> bool {
    previous_group_digit_index(source, code_span)
        .is_some_and(|idx| !index_is_in_any_span(payment_number_spans, idx))
        || next_group_digit_index(source, code_span)
            .is_some_and(|idx| !index_is_in_any_span(payment_number_spans, idx))
}

fn apply_payment_credential_redaction(
    text: &mut String,
    redactions: &mut Vec<String>,
    kinds: &mut Vec<SecretInputType>,
) {
    let source = text.clone();
    let candidate_spans: Vec<(RedactionSpan, bool)> = PAYMENT_NUMBER_PATTERN
        .find_iter(&source)
        .filter(|matched| {
            let span = RedactionSpan {
                start: matched.start(),
                end: matched.end(),
            };
            if numeric_span_is_decimal_literal(&source, span) {
                return false;
            }
            let digits = digit_count(matched.as_str());
            (12..=19).contains(&digits)
        })
        .map(|matched| {
            (
                RedactionSpan {
                    start: matched.start(),
                    end: matched.end(),
                },
                luhn_valid(&digits_only(matched.as_str())),
            )
        })
        .collect();

    if candidate_spans.is_empty() {
        return;
    }

    let short_code_spans: Vec<RedactionSpan> = SHORT_NUMERIC_CODE_PATTERN
        .find_iter(&source)
        .map(|matched| RedactionSpan {
            start: matched.start(),
            end: matched.end(),
        })
        .collect();

    let mut spans: Vec<RedactionSpan> = candidate_spans
        .iter()
        .filter(|(payment_span, valid_luhn)| {
            let digits = digit_count(&source[payment_span.start..payment_span.end]);
            *valid_luhn && digits >= 13
                || short_code_spans.iter().any(|code_span| {
                    !spans_overlap(*payment_span, *code_span)
                        && spans_are_near(*payment_span, *code_span, PAYMENT_CODE_PROXIMITY_BYTES)
                })
        })
        .map(|(span, _)| *span)
        .collect();

    if spans.is_empty() {
        return;
    }

    let payment_number_spans = spans.clone();
    for matched in SHORT_NUMERIC_CODE_PATTERN.find_iter(&source) {
        let code_span = RedactionSpan {
            start: matched.start(),
            end: matched.end(),
        };
        if payment_number_spans
            .iter()
            .any(|payment_span| spans_overlap(*payment_span, code_span))
        {
            continue;
        }
        if short_numeric_code_is_grouped_number_part(&source, code_span, &payment_number_spans) {
            continue;
        }
        if payment_number_spans.iter().any(|payment_span| {
            spans_are_near(*payment_span, code_span, PAYMENT_CODE_PROXIMITY_BYTES)
        }) {
            spans.push(code_span);
        }
    }

    spans.sort_by_key(|span| (span.start, span.end));
    let mut redacted = String::with_capacity(source.len());
    let mut cursor = 0usize;
    let mut count = 0usize;
    for span in spans {
        if span.start < cursor {
            continue;
        }
        redacted.push_str(&source[cursor..span.start]);
        redacted.push_str("[REDACTED_PAYMENT_DATA]");
        cursor = span.end;
        count += 1;
    }
    redacted.push_str(&source[cursor..]);

    if count == 0 {
        return;
    }
    *text = redacted;
    redactions.push(format!("payment_credential x{}", count));
    push_secret_kind(kinds, SecretInputType::PaymentCredential);
}

pub fn redact_secret_input(text: &str) -> SecretRedactionResult {
    let mut redacted = text.to_string();
    let mut redactions = Vec::new();
    let mut kinds = Vec::new();

    apply_secret_redaction(
        &mut redacted,
        &mut redactions,
        &mut kinds,
        &PRIVATE_KEY_BLOCK_PATTERN,
        "[REDACTED_PRIVATE_KEY]",
        "private_key",
        SecretInputType::PrivateKeyMaterial,
    );
    apply_secret_redaction(
        &mut redacted,
        &mut redactions,
        &mut kinds,
        &CERT_BLOCK_PATTERN,
        "[REDACTED_CERTIFICATE]",
        "certificate",
        SecretInputType::PrivateKeyMaterial,
    );
    apply_secret_redaction(
        &mut redacted,
        &mut redactions,
        &mut kinds,
        &URL_SECRET_QUERY_PATTERN,
        "${1}[REDACTED_SECRET]",
        "secret_query_param",
        SecretInputType::ApiKeyOrToken,
    );
    apply_secret_redaction(
        &mut redacted,
        &mut redactions,
        &mut kinds,
        &SECRET_ASSIGNMENT_PATTERN,
        "${1}[REDACTED_SECRET]${2}",
        "secret_assignment",
        SecretInputType::ApiKeyOrToken,
    );
    apply_secret_redaction(
        &mut redacted,
        &mut redactions,
        &mut kinds,
        &ENV_STYLE_SECRET_ASSIGNMENT_PATTERN,
        "${1}[REDACTED_SECRET]",
        "env_secret_assignment",
        SecretInputType::ApiKeyOrToken,
    );
    apply_secret_redaction(
        &mut redacted,
        &mut redactions,
        &mut kinds,
        &BEARER_TOKEN_PATTERN,
        "Bearer [REDACTED_SECRET]",
        "bearer_token",
        SecretInputType::ApiKeyOrToken,
    );
    apply_secret_redaction(
        &mut redacted,
        &mut redactions,
        &mut kinds,
        &OPENAI_KEY_PATTERN,
        "[REDACTED_SECRET]",
        "openai_key",
        SecretInputType::ApiKeyOrToken,
    );
    apply_secret_redaction(
        &mut redacted,
        &mut redactions,
        &mut kinds,
        &GITHUB_PAT_PATTERN,
        "[REDACTED_SECRET]",
        "github_token",
        SecretInputType::ApiKeyOrToken,
    );
    apply_secret_redaction(
        &mut redacted,
        &mut redactions,
        &mut kinds,
        &NOTION_TOKEN_PATTERN,
        "[REDACTED_SECRET]",
        "notion_token",
        SecretInputType::ApiKeyOrToken,
    );
    apply_secret_redaction(
        &mut redacted,
        &mut redactions,
        &mut kinds,
        &GOOGLE_API_KEY_PATTERN,
        "[REDACTED_SECRET]",
        "google_api_key",
        SecretInputType::ApiKeyOrToken,
    );
    apply_secret_redaction(
        &mut redacted,
        &mut redactions,
        &mut kinds,
        &GOOGLE_OAUTH_TOKEN_PATTERN,
        "[REDACTED_SECRET]",
        "google_oauth_token",
        SecretInputType::ApiKeyOrToken,
    );
    apply_secret_redaction(
        &mut redacted,
        &mut redactions,
        &mut kinds,
        &SLACK_TOKEN_PATTERN,
        "[REDACTED_SECRET]",
        "slack_token",
        SecretInputType::ApiKeyOrToken,
    );
    apply_secret_redaction(
        &mut redacted,
        &mut redactions,
        &mut kinds,
        &MOLTBOOK_TOKEN_PATTERN,
        "[REDACTED_SECRET]",
        "moltbook_token",
        SecretInputType::ApiKeyOrToken,
    );
    apply_labeled_credential_value_redaction(&mut redacted, &mut redactions, &mut kinds);
    apply_payment_credential_redaction(&mut redacted, &mut redactions, &mut kinds);
    apply_opaque_token_redaction(&mut redacted, &mut redactions, &mut kinds);

    SecretRedactionResult {
        text: redacted,
        redactions,
        kinds,
    }
}

/// Result of secret-format scrubbing applied to outbound text.
#[derive(Debug, Clone)]
pub struct FilteredOutput {
    pub text: String,
    pub redactions: Vec<String>,
    pub _is_clean: bool,
}

#[cfg(test)]
mod tests {
    use super::*;

    fn fake_openai_key() -> String {
        ["sk", "-1234567890", "abcdefghijklmnop"].concat()
    }

    fn fake_moltbook_key() -> String {
        ["moltbook", "_sk_", "8ghQ92XoaW4VsHGUrOv00Ox17zc2__Y2"].concat()
    }

    #[test]
    fn filter_output_masks_api_keys() {
        let guard = SecurityGuard::new(true);
        let output = format!("Here's your API key: {}", fake_openai_key());
        let filtered = guard.filter_output(&output);
        assert!(!filtered._is_clean);
        assert!(filtered.text.contains("[REDACTED_API_KEY]"));
    }

    #[test]
    fn filter_output_passes_clean_prose() {
        let guard = SecurityGuard::new(true);
        let output = "Top headlines mention OpenAI and Anthropic funding activity.";
        let filtered = guard.filter_output(output);
        assert!(filtered.redactions.is_empty());
        assert_eq!(filtered.text, output);
    }

    #[test]
    fn test_secret_input_detection() {
        let token = redact_secret_input(&fake_openai_key());
        assert!(token.had_secret());
        assert_eq!(token.primary_kind(), Some(SecretInputType::ApiKeyOrToken));

        let private_key =
            redact_secret_input("-----BEGIN PRIVATE KEY-----\nabc\n-----END PRIVATE KEY-----");
        assert!(private_key.had_secret());
        assert_eq!(
            private_key.primary_kind(),
            Some(SecretInputType::PrivateKeyMaterial)
        );

        let plain_text = redact_secret_input("Hello world");
        assert!(!plain_text.had_secret());
        assert_eq!(plain_text.primary_kind(), None);
    }

    #[test]
    fn test_secret_redaction_masks_bare_and_inline_secrets() {
        let moltbook_key = fake_moltbook_key();
        let openai_key = fake_openai_key();
        let result =
            redact_secret_input(&format!("Use {} and api_key={}", moltbook_key, openai_key,));
        assert!(result.had_secret());
        assert!(!result.text.contains(&moltbook_key));
        assert!(!result.text.contains(&openai_key));
        assert!(result.text.contains("[REDACTED_SECRET]"));
    }

    #[test]
    fn test_secret_redaction_masks_opaque_token_shapes() {
        let result = redact_secret_input("here is my key 2skdjfkj2wlfrj23kr2rlm");

        assert!(result.had_secret());
        assert_eq!(result.primary_kind(), Some(SecretInputType::ApiKeyOrToken));
        assert!(!result.text.contains("2skdjfkj2wlfrj23kr2rlm"));
        assert!(result.text.contains("[REDACTED_SECRET]"));
        assert!(result.is_mostly_secret_payload());
    }

    #[test]
    fn test_secret_redaction_masks_labeled_low_entropy_credentials() {
        let result = redact_secret_input("my provider api key asdas-asdasd-asdasdasd-asdasd");

        assert!(result.had_secret());
        assert_eq!(result.primary_kind(), Some(SecretInputType::ApiKeyOrToken));
        assert!(!result.text.contains("asdas-asdasd-asdasdasd-asdasd"));
        assert!(result.text.contains("[REDACTED_SECRET]"));
    }

    #[test]
    fn test_secret_redaction_keeps_plain_prose() {
        let result = redact_secret_input("I live in Madhyam, Kolkata and prefer concise answers.");

        assert!(!result.had_secret());
    }

    #[test]
    fn test_secret_redaction_keeps_identifier_slugs_and_versions() {
        for value in [
            "memory_capture_events",
            "routing-policy-default-v2",
            "prompt-candidate-550e8400-e29b-41d4-a716-446655440000",
            "baidu/ernie-4.5-vl-424b-a47b",
            "deepseek/deepseek-v4-pro",
            "Recent model failures: baidu/ernie-4.5-turbo-128k-preview hit a capability limit.",
            "Model: provider-a/frontier.reasoning-v4-pro-2026",
        ] {
            let result = redact_secret_input(value);
            assert!(!result.had_secret(), "unexpected redaction for {value}");
            assert_eq!(result.text, value);
        }
    }

    #[test]
    fn test_secret_redaction_keeps_untrusted_output_envelopes() {
        let wrapped = crate::security::sanitize_untrusted_output(
            "web_search",
            "Search result snippet with api_key=sk-1234567890abcdefghijklmnop.",
        );
        let result = redact_secret_input(&wrapped);

        assert!(result.text.contains("[UNTRUSTED_WEB_SEARCH_OUTPUT]"));
        assert!(result.text.contains("[/UNTRUSTED_WEB_SEARCH_OUTPUT]"));
        assert!(!result.text.contains("[[REDACTED_SECRET]]"));
        assert!(!result.text.contains("sk-1234567890abcdefghijklmnop"));
        assert!(result.text.contains("api_key=[REDACTED_SECRET]"));

        let bare = redact_secret_input("UNTRUSTED_SEARCH_RESULT_TOKEN_OUTPUT");
        assert!(bare.had_secret());
        assert_eq!(bare.text, "[REDACTED_SECRET]");
    }

    #[test]
    fn test_secret_redaction_keeps_public_url_slug_filenames() {
        let message = concat!(
            "Trevi https://www.skylinewebcams.com/en/webcam/italia/lazio/roma/fontana-di-trevi.html ",
            "Rialto https://www.skylinewebcams.com/en/webcam/italia/veneto/venezia/rialto-canal-grande.html"
        );
        let result = redact_secret_input(message);

        assert!(!result.had_secret());
        assert_eq!(result.text, message);
        assert!(result.text.contains("fontana-di-trevi.html"));
        assert!(result.text.contains("rialto-canal-grande.html"));
    }

    #[test]
    fn test_secret_redaction_keeps_public_app_urls_and_hyphenated_hosts() {
        let message =
            "Open https://sacred-duration-season-drainage.trycloudflare.com/apps/7b7c4863/";
        let result = redact_secret_input(message);

        assert!(!result.had_secret());
        assert_eq!(result.text, message);
    }

    #[test]
    fn test_secret_redaction_keeps_sentence_with_internal_identifier() {
        let message = "Inspect memory_capture_events and capture model health";
        let result = redact_secret_input(message);

        assert!(!result.had_secret());
        assert_eq!(result.text, message);
    }

    #[test]
    fn test_secret_redaction_keeps_python_traceback_expressions() {
        let message = "subprocess.check_call stdout=subprocess.DEVNULL stderr=subprocess.DEVNULL";
        let result = redact_secret_input(message);

        assert!(!result.had_secret());
        assert_eq!(result.text, message);
    }

    #[test]
    fn test_secret_redaction_masks_sensitive_query_params() {
        let result =
            redact_secret_input("Open https://example.com/callback?token=supersecretvalue123456");
        assert!(result.had_secret());
        assert!(result.text.contains("token=[REDACTED_SECRET]"));
        assert!(!result.text.contains("supersecretvalue123456"));
    }

    #[test]
    fn test_secret_redaction_masks_payment_credentials_by_shape() {
        let result = redact_secret_input("Use 4242 4242 4242 4242 with 899 at checkout");

        assert!(result.had_secret());
        assert_eq!(
            result.primary_kind(),
            Some(SecretInputType::PaymentCredential)
        );
        assert!(!result.text.contains("4242 4242 4242 4242"));
        assert!(!result.text.contains("899"));
        assert!(result.text.contains("[REDACTED_PAYMENT_DATA]"));

        let punctuated = redact_secret_input("Use 4242 4242 4242 4242.");
        assert!(punctuated.had_secret());
        assert!(punctuated.text.contains("[REDACTED_PAYMENT_DATA]."));
    }

    #[test]
    fn test_secret_redaction_keeps_phone_number_groups_near_payment_credentials() {
        let result =
            redact_secret_input("text +1 555 123 4567 and card 4111 1111 1111 1111 stays private");

        assert!(result.had_secret());
        assert!(result.text.contains("+1 555 123 4567"));
        assert!(!result.text.contains("4111 1111 1111 1111"));
        assert!(result.text.contains("[REDACTED_PAYMENT_DATA]"));
    }

    #[test]
    fn test_secret_redaction_masks_compact_payment_like_values() {
        let result = redact_secret_input("123412342344 899");

        assert!(result.had_secret());
        assert!(!result.text.contains("123412342344"));
        assert!(!result.text.contains("899"));
    }

    #[test]
    fn test_secret_redaction_keeps_decimal_telemetry_values() {
        for input in [
            r#""cost_usd": 0.012934658880000002"#,
            r#""cost_usd":"0.21772148112000006""#,
            r#""value":0.29229803162999984"#,
        ] {
            let result = redact_secret_input(input);
            assert!(!result.had_secret(), "unexpected redaction for {input}");
            assert_eq!(result.text, input);
        }
    }

    #[test]
    fn security_errors_have_machine_readable_codes() {
        let error = SecurityError::tool_argument_denied("private_or_local_ip");

        assert_eq!(
            error.code(),
            "security_tool_argument_denied_private_or_local_ip"
        );
        assert_eq!(
            error.to_string(),
            "ERR/security/tool_argument_denied: tool argument denied (private_or_local_ip)"
        );
    }
}
