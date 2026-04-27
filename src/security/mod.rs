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

pub mod abuse_tracker;
pub mod action_guard;
pub mod capabilities;
pub mod intent_classifier;
pub mod model_hardening;
pub mod model_input;
pub mod normalize;
pub mod outbound;
pub mod pii;
pub mod skill_review;
pub mod tool_args_guard;
pub mod trust_boundary;

pub use action_guard::ActionGuard;
pub use model_hardening::protect_system_prompt;
#[allow(unused_imports)]
pub use model_input::{
    CurrentChatPiiPolicy, ModelInputContext, ModelInputPrivacyDecision,
    ModelInputPrivacyJsonResult, ModelInputPrivacyMode, ModelInputPrivacyTextResult,
    ModelPrivacyConfig, render_model_input_fallback, sanitize_model_input_json,
    sanitize_model_input_text,
};
pub use normalize::normalize_for_analysis;
pub use outbound::{
    OutboundPrivacyDecision, OutboundPrivacyPolicy, check_outbound_text,
    format_outbound_privacy_block, sanitize_outbound_json,
};
pub use pii::redact_pii;
pub use trust_boundary::{
    canonical_capabilities, redact_json_secrets, sanitize_input_schema, sanitize_untrusted_html,
    sanitize_untrusted_output, scan_untrusted_text,
};

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
        let text = if redacted.primary_kind() == Some(SecretInputType::ApiKeyOrToken) {
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
static ENV_STYLE_SECRET_ASSIGNMENT_PATTERN: Lazy<Regex> = Lazy::new(|| {
    Regex::new(r"(\b[A-Z0-9_]*(?:KEY|TOKEN|SECRET|PASSWORD)[A-Z0-9_]*\s*=\s*)[^\s]{16,}").unwrap()
});
static URL_SECRET_QUERY_PATTERN: Lazy<Regex> = Lazy::new(|| {
    Regex::new(
        r"(?i)([?&](?:access_token|token|sig|signature|api[_-]?key|key|auth|password|client_secret)=)[^&\s]+",
    )
    .unwrap()
});
const OPAQUE_TOKEN_MIN_CHARS: usize = 20;
const OPAQUE_TOKEN_ENTROPY_BITS_PER_CHAR: f64 = 3.0;

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
        .any(|ch| ch.is_ascii_digit() || matches!(ch, '_' | '-' | '=' | '+'))
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
        && !trimmed.chars().any(char::is_whitespace)
        && trimmed.chars().all(opaque_token_shape_char)
        && !is_likely_identifier_slug(trimmed)
        && !is_likely_code_expression_token(trimmed)
        && opaque_token_has_secret_signal(trimmed)
        && shannon_entropy_bits_per_char(trimmed) >= OPAQUE_TOKEN_ENTROPY_BITS_PER_CHAR
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
            if is_opaque_token_shape(candidate) {
                redacted.push_str(&source[last..start]);
                redacted.push_str("[REDACTED_SECRET]");
                last = idx;
                count += 1;
            }
        }
    }

    if let Some(start) = run_start {
        let candidate = &source[start..];
        if is_opaque_token_shape(candidate) {
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
        &URL_SECRET_QUERY_PATTERN,
        "${1}[REDACTED_SECRET]",
        "secret_query_param",
        SecretInputType::ApiKeyOrToken,
    );
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
        ] {
            let result = redact_secret_input(value);
            assert!(!result.had_secret(), "unexpected redaction for {value}");
            assert_eq!(result.text, value);
        }
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
}
