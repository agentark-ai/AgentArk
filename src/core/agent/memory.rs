use super::action_selection::format_recent_dialogue_for_memory_context;
use super::*;
use crate::storage::entities::user_preference::{
    MemorySensitivity, classify_saved_memory_sensitivity, normalize_memory_sensitivity,
};
use anyhow::Context;

const USER_MEMORY_CAPTURE_PENDING_STATUS: &str = "pending_consolidation";
const USER_MEMORY_CAPTURE_PROCESSING_DEFERRED_STATUS: &str = "processing_deferred";
const USER_MEMORY_CAPTURE_COMPLETED_DEFERRED_STATUS: &str = "completed_deferred";
const USER_MEMORY_CAPTURE_FAILED_DEFERRED_STATUS: &str = "failed_deferred";
const USER_MEMORY_CAPTURE_RETIRED_STALE_PROCESSING_STATUS: &str = "retired_stale_processing";
const USER_MEMORY_CAPTURE_FAILED_STALE_PROCESSING_STATUS: &str = "failed_stale_processing";
const USER_MEMORY_CAPTURE_DEFERRED_BATCH_LIMIT: u64 = 16;
const USER_MEMORY_CAPTURE_DRAIN_MAX_BATCHES: usize = 8;
const USER_MEMORY_CAPTURE_STARTUP_BACKFILL_LIMIT: u64 = 12;
const USER_MEMORY_CAPTURE_STALE_PROCESSING_LEASE_DEFAULT_SECS: i64 = 60 * 60;
const USER_MEMORY_CAPTURE_STALE_PROCESSING_LEASE_MIN_SECS: i64 = 5 * 60;
const USER_MEMORY_CAPTURE_STALE_RECOVERY_BATCH_LIMIT: u64 = 64;
const USER_MEMORY_CAPTURE_SOURCE_HISTORY_LIMIT: u64 = 32;

static USER_MEMORY_CAPTURE_DRAIN_SEMAPHORE: once_cell::sync::Lazy<
    std::sync::Arc<tokio::sync::Semaphore>,
> = once_cell::sync::Lazy::new(|| std::sync::Arc::new(tokio::sync::Semaphore::new(1)));
static USER_MEMORY_CAPTURE_DRAIN_WAKE_REQUESTED: std::sync::atomic::AtomicBool =
    std::sync::atomic::AtomicBool::new(false);

pub(super) fn saved_memory_sensitivity_from_parts(
    key: Option<&str>,
    value: &str,
    kind: Option<&str>,
    stored_sensitivity: Option<&str>,
) -> MemorySensitivity {
    let inferred = classify_saved_memory_sensitivity(key, value, kind);
    match normalize_memory_sensitivity(stored_sensitivity) {
        Some(MemorySensitivity::Sensitive)
            if matches!(
                inferred,
                MemorySensitivity::PromptSafe | MemorySensitivity::PersonalIdentifier
            ) =>
        {
            let key_only_inferred = classify_saved_memory_sensitivity(key, value, None);
            if key.is_some()
                && matches!(
                    key_only_inferred,
                    MemorySensitivity::PromptSafe | MemorySensitivity::PersonalIdentifier
                )
            {
                inferred
            } else {
                MemorySensitivity::Sensitive
            }
        }
        Some(stored) => stored,
        None => inferred,
    }
}

pub(super) fn saved_memory_is_prompt_safe(sensitivity: MemorySensitivity) -> bool {
    matches!(
        sensitivity,
        MemorySensitivity::PromptSafe | MemorySensitivity::PersonalIdentifier
    )
}

#[cfg(test)]
mod saved_memory_sensitivity_tests {
    use super::*;

    #[test]
    fn structured_identity_overrides_legacy_sensitive_default() {
        assert_eq!(
            saved_memory_sensitivity_from_parts(
                Some("user_first_name"),
                "Debanka",
                None,
                Some("sensitive"),
            ),
            MemorySensitivity::PersonalIdentifier
        );
    }

    #[test]
    fn crisis_sensitive_stored_classification_is_not_downgraded() {
        assert_eq!(
            saved_memory_sensitivity_from_parts(
                Some("user_first_name"),
                "Debanka",
                None,
                Some("crisis_sensitive"),
            ),
            MemorySensitivity::CrisisSensitive
        );
    }

    #[test]
    fn legacy_sensitive_is_not_downgraded_by_kind_alone() {
        assert_eq!(
            saved_memory_sensitivity_from_parts(
                Some("private_detail"),
                "sensitive value",
                Some("identity"),
                Some("sensitive"),
            ),
            MemorySensitivity::Sensitive
        );
    }

    #[test]
    fn learned_memory_keys_do_not_use_string_aliases() {
        assert_ne!(
            learned_user_memory_keys("user_first_name", "permanent", None, None),
            learned_user_memory_keys("user_name", "permanent", None, None)
        );

        let item = crate::storage::experience_item::Model {
            id: "memory-1".to_string(),
            kind: "personal_fact".to_string(),
            scope: "global".to_string(),
            project_id: None,
            conversation_id: None,
            title: "Learned user memory".to_string(),
            content: "user_first_name: Debanka".to_string(),
            normalized_key: "user_memory::user_first_name::permanent".to_string(),
            confidence: 0.95,
            support_count: 1,
            contradiction_count: 0,
            status: "active".to_string(),
            metadata: serde_json::json!({
                "key": "user_first_name",
                "memory_kind": "identity",
                "durability": "permanent",
                "sensitivity": "personal_identifier",
            }),
            last_supported_at: None,
            last_contradicted_at: None,
            created_at: "2026-01-01T00:00:00Z".to_string(),
            updated_at: "2026-01-01T00:00:00Z".to_string(),
            embedding: None,
        };

        assert_eq!(
            learned_user_memory_key(&item).as_deref(),
            Some("user_first_name")
        );
        assert!(learned_user_memory_key_matches(&item, "User First-Name"));
        assert!(!learned_user_memory_key_matches(&item, "user_name"));
        assert_eq!(learned_user_memory_lookup_kind(&item), "identity");
    }

    #[test]
    fn memory_capture_payload_accepts_mixed_batch_operations() {
        let parsed = parse_user_memory_capture_payload(
            r#"{
                "memories": [
                    {
                        "key": "preferred_name",
                        "value": "User goes by Ronny.",
                        "kind": "identity",
                        "durability": "permanent",
                        "scope": "global",
                        "sensitivity": "personal_identifier",
                        "confidence": 0.95,
                        "reason": "durable user identity",
                        "looks_sensitive": false,
                        "sensitive_reason": ""
                    },
                    {
                        "key": "legal_name",
                        "value": "User's legal name is Debanka.",
                        "kind": "identity",
                        "durability": "permanent",
                        "scope": "global",
                        "sensitivity": "personal_identifier",
                        "confidence": 0.95,
                        "reason": "durable user identity",
                        "looks_sensitive": false,
                        "sensitive_reason": ""
                    }
                ],
                "retractions": [
                    {
                        "key": "old_display_name",
                        "kind": "identity",
                        "scope": "global",
                        "confidence": 0.9,
                        "reason": "superseded by current self-identification"
                    }
                ]
            }"#,
        )
        .expect("valid mixed memory batch should parse");

        assert_eq!(
            parsed.disposition,
            UserMemoryCapturePayloadDisposition::Exact
        );
        assert_eq!(
            parsed
                .payload
                .get("memories")
                .and_then(|value| value.as_array())
                .map(Vec::len),
            Some(2)
        );
        assert_eq!(
            parsed
                .payload
                .get("retractions")
                .and_then(|value| value.as_array())
                .map(Vec::len),
            Some(1)
        );
    }

    #[test]
    fn memory_capture_payload_recovers_single_memory_object_as_batch() {
        let parsed = parse_user_memory_capture_payload(
            r#"{
                "memories": {
                    "key": "preferred_name",
                    "value": "User goes by Ronny.",
                    "kind": "identity",
                    "durability": "permanent",
                    "scope": "global",
                    "sensitivity": "personal_identifier",
                    "confidence": 0.95,
                    "reason": "durable user identity",
                    "looks_sensitive": false,
                    "sensitive_reason": ""
                },
                "retractions": null
            }"#,
        )
        .expect("single memory object should recover to array shape");

        assert_eq!(
            parsed.disposition,
            UserMemoryCapturePayloadDisposition::ShapeRecovered
        );
        assert_eq!(
            parsed
                .payload
                .get("memories")
                .and_then(|value| value.as_array())
                .map(Vec::len),
            Some(1)
        );
        assert_eq!(
            parsed
                .payload
                .get("retractions")
                .and_then(|value| value.as_array())
                .map(Vec::len),
            Some(0)
        );
    }

    fn memory_operation_with_type(operation_type: &str) -> crate::storage::memory_operation::Model {
        crate::storage::memory_operation::Model {
            id: format!("memory-operation-{operation_type}"),
            capture_event_id: None,
            operation_type: operation_type.to_string(),
            status: "pending_apply".to_string(),
            target_memory_id: Some("target-row".to_string()),
            applied_memory_id: None,
            key: Some("interface_theme".to_string()),
            value: Some("dark".to_string()),
            memory_kind: "preference".to_string(),
            durability: "permanent".to_string(),
            scope: "global".to_string(),
            project_id: None,
            conversation_id: None,
            confidence: 0.95,
            looks_sensitive: false,
            sensitive_reason: None,
            valid_from: None,
            expires_at: None,
            review_at: None,
            rationale: None,
            evidence_refs: serde_json::json!([]),
            model_metadata: serde_json::json!({}),
            apply_metadata: serde_json::json!({}),
            applied_at: None,
            reviewed_at: None,
            review_notes: None,
            created_at: "2026-01-01T00:00:00Z".to_string(),
            updated_at: "2026-01-01T00:00:00Z".to_string(),
        }
    }

    #[test]
    fn add_operations_do_not_force_default_targets_past_dedup() {
        let add = memory_operation_with_type("add");
        assert_eq!(memory_operation_explicit_upsert_target_id(&add), None);

        let update = memory_operation_with_type("update");
        assert_eq!(
            memory_operation_explicit_upsert_target_id(&update),
            Some("target-row")
        );
    }

    #[test]
    fn learned_memory_sanitizer_keeps_semantic_key_out_of_secret_scan() {
        let sanitized = sanitize_learned_user_memory_content_for_storage(
            "friend_best_friend_cs_teammate_9_years",
            "Alex had a best friend of nine years and they played Counter-Strike together.",
        )
        .expect("semantic memory keys must not make prose values look secret");

        assert_eq!(
            sanitized.value,
            "Alex had a best friend of nine years and they played Counter-Strike together."
        );
        assert_eq!(
            sanitized.content,
            "friend_best_friend_cs_teammate_9_years: Alex had a best friend of nine years and they played Counter-Strike together."
        );
        assert!(!sanitized.redacted_secret);
    }

    #[test]
    fn learned_memory_sanitizer_rejects_secret_only_values() {
        assert!(
            sanitize_learned_user_memory_content_for_storage(
                "friend_best_friend_cs_teammate_9_years",
                "2skdjfkj2wlfrj23kr2rlm"
            )
            .is_none()
        );
    }

    #[test]
    fn memory_operation_candidates_do_not_use_capture_event_as_pattern_fk() {
        let mut operation = memory_operation_with_type("add");
        operation.capture_event_id = Some("memory-capture-123".to_string());

        assert_eq!(
            memory_operation_learning_candidate_pattern_id(&operation),
            None
        );
    }

    fn memory_capture_event_with_status(
        status: &str,
        updated_at: String,
    ) -> crate::storage::memory_capture_event::Model {
        crate::storage::memory_capture_event::Model {
            id: format!("memory-capture-{}", status),
            source_message_id: Some("message-1".to_string()),
            conversation_id: Some("conversation-1".to_string()),
            project_id: None,
            channel: "chat".to_string(),
            status: status.to_string(),
            capture_kind: "user_fact_memory_capture".to_string(),
            source_hash: "source-key".to_string(),
            attempt_metadata: serde_json::json!({}),
            error_history: serde_json::json!([]),
            replay_count: 0,
            next_retry_at: None,
            completed_at: None,
            created_at: updated_at.clone(),
            updated_at,
        }
    }

    #[test]
    fn stale_processing_capture_no_longer_blocks_source_retry() {
        let now = chrono::DateTime::parse_from_rfc3339("2026-05-09T12:00:00Z")
            .expect("valid timestamp")
            .with_timezone(&chrono::Utc);
        let stale_updated_at = (now
            - chrono::Duration::seconds(user_memory_capture_stale_processing_lease_secs() + 1))
        .to_rfc3339();
        let stale_processing = memory_capture_event_with_status("processing", stale_updated_at);
        assert!(user_memory_capture_event_is_stale_processing(
            &stale_processing,
            now
        ));
        assert!(!user_memory_capture_event_blocks_source_retry(
            &stale_processing,
            now
        ));

        let fresh_processing = memory_capture_event_with_status("processing", now.to_rfc3339());
        assert!(!user_memory_capture_event_is_stale_processing(
            &fresh_processing,
            now
        ));
        assert!(user_memory_capture_event_blocks_source_retry(
            &fresh_processing,
            now
        ));

        let retired_stale = memory_capture_event_with_status(
            USER_MEMORY_CAPTURE_RETIRED_STALE_PROCESSING_STATUS,
            now.to_rfc3339(),
        );
        assert!(!user_memory_capture_event_blocks_source_retry(
            &retired_stale,
            now
        ));

        let legacy_failed_stale = memory_capture_event_with_status(
            USER_MEMORY_CAPTURE_FAILED_STALE_PROCESSING_STATUS,
            now.to_rfc3339(),
        );
        assert!(!user_memory_capture_event_blocks_source_retry(
            &legacy_failed_stale,
            now
        ));

        let completed = memory_capture_event_with_status("noop", now.to_rfc3339());
        assert!(user_memory_capture_event_blocks_source_retry(
            &completed, now
        ));
    }

    #[test]
    fn memory_capture_rejects_integration_setup_status_as_operational_artifact() {
        assert!(user_memory_candidate_is_operational_artifact(
            "linear_integration_installed",
            "Linear integration has been scaffolded as a custom API integration using Linear's GraphQL API at https://api.linear.app with Bearer token auth via LINEAR_API_KEY. The integration was set up but is not yet fully authenticated.",
            "knowledge",
            "knowledge",
        ));

        assert!(!user_memory_candidate_is_operational_artifact(
            "preferred_issue_tracker",
            "The user prefers Linear for issue tracking.",
            "work_preference",
            "work_preference",
        ));
    }

    #[test]
    fn semantic_memory_review_parser_blocks_operational_artifact_verdict() {
        let review = parse_user_memory_candidate_semantic_review(
            r#"{"store":false,"confidence":0.93,"reason":"The candidate is a watcher notification configuration, not reusable memory."}"#,
        )
        .expect("semantic review should parse");
        assert!(!review.store);
        assert!(user_memory_candidate_review_should_skip(&review));

        let durable = parse_user_memory_candidate_semantic_review(
            r#"{"store":true,"confidence":0.91,"reason":"The candidate is a general durable notification preference."}"#,
        )
        .expect("semantic review should parse");
        assert!(durable.store);
        assert!(!user_memory_candidate_review_should_skip(&durable));
    }

    #[test]
    fn merged_phrasing_history_is_bounded_and_normalized() {
        let mut metadata = serde_json::Map::new();
        for index in 0..(crate::core::memory_dedup::MAX_MERGED_PHRASINGS + 2) {
            append_learned_user_memory_merged_phrasing(
                &mut metadata,
                Some("Interface Theme"),
                Some(&format!("theme value {index}")),
                Some("test"),
                "2026-01-01T00:00:00Z",
            );
        }

        let merged = metadata
            .get("merged_phrasings")
            .and_then(|value| value.as_array())
            .expect("merged phrasing history exists");
        assert_eq!(
            merged.len(),
            crate::core::memory_dedup::MAX_MERGED_PHRASINGS
        );
        assert_eq!(
            merged
                .first()
                .and_then(|value| value.get("key"))
                .and_then(|value| value.as_str()),
            Some("interface_theme")
        );
        assert_eq!(
            merged
                .first()
                .and_then(|value| value.get("value"))
                .and_then(|value| value.as_str()),
            Some("theme value 2")
        );
    }
}

pub(super) fn normalize_user_fact_key(raw: &str) -> Option<String> {
    let key = raw
        .trim()
        .to_ascii_lowercase()
        .chars()
        .map(|ch| {
            if ch.is_ascii_alphanumeric() || ch == '_' {
                ch
            } else {
                '_'
            }
        })
        .collect::<String>()
        .trim_matches('_')
        .to_string();
    if key.is_empty() || key.len() > 80 {
        return None;
    }
    Some(key)
}

pub(super) fn learned_memory_key_to_user_preference_key(raw_key: &str) -> Option<String> {
    let normalized = normalize_user_fact_key(raw_key)?;
    if matches!(
        normalized.as_str(),
        "user_name"
            | "user_timezone"
            | "preferred_tone"
            | "assistant_priority_focus"
            | "user_email"
            | "user_phone"
            | "user_address"
    ) || normalized.starts_with("likes_")
        || normalized.starts_with("dislikes_")
        || normalized.starts_with("rule_")
    {
        return Some(normalized);
    }
    None
}

pub(super) fn normalize_user_memory_text(raw: &str, max_chars: usize) -> Option<String> {
    let value = raw
        .trim()
        .trim_matches(|c: char| matches!(c, '"' | '\'' | '`'));
    if value.is_empty() || value.eq_ignore_ascii_case("null") {
        return None;
    }
    Some(safe_truncate(value, max_chars))
}

pub(super) const MEMORY_OPAQUE_TOKEN_MIN_CHARS: usize = 20;
pub(super) const MEMORY_OPAQUE_TOKEN_ENTROPY_BITS_PER_CHAR: f64 = 3.5;
pub(super) const USER_MEMORY_REDACTION_MARKERS: &[&str] = &[
    "[REDACTED_SECRET]",
    "[REDACTED_API_KEY]",
    "[REDACTED_PRIVATE_KEY]",
    "[REDACTED_CERTIFICATE]",
];

#[derive(Debug, Clone, PartialEq, Eq)]
pub(super) struct SanitizedUserMemoryText {
    pub(super) text: String,
    pub(super) redacted_secret: bool,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub(super) struct SanitizedUserMemoryContent {
    pub(super) value: String,
    pub(super) content: String,
    pub(super) redacted_secret: bool,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub(super) struct StructuralOpaqueTokenRedaction {
    pub(super) text: String,
    pub(super) redacted_secret: bool,
    pub(super) mostly_secret: bool,
}

pub(super) fn user_memory_token_shape_char(ch: char) -> bool {
    ch.is_ascii_alphanumeric() || matches!(ch, '_' | '-' | '/' | '=' | '+' | '.')
}

pub(super) fn user_memory_shannon_entropy_bits_per_char(value: &str) -> f64 {
    let mut counts: HashMap<char, usize> = HashMap::new();
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

pub(super) fn user_memory_is_opaque_token_shape(value: &str) -> bool {
    let trimmed = value.trim();
    let char_count = trimmed.chars().count();
    char_count >= MEMORY_OPAQUE_TOKEN_MIN_CHARS
        && !trimmed.chars().any(char::is_whitespace)
        && trimmed.chars().all(user_memory_token_shape_char)
        && user_memory_shannon_entropy_bits_per_char(trimmed)
            >= MEMORY_OPAQUE_TOKEN_ENTROPY_BITS_PER_CHAR
}

pub(super) fn user_memory_contains_redaction_marker(value: &str) -> bool {
    USER_MEMORY_REDACTION_MARKERS
        .iter()
        .any(|marker| value.contains(marker))
}

pub(super) fn user_memory_is_mostly_redaction_marker_payload(value: &str) -> bool {
    if !user_memory_contains_redaction_marker(value) {
        return false;
    }
    let mut stripped = value.to_string();
    for marker in USER_MEMORY_REDACTION_MARKERS {
        stripped = stripped.replace(marker, " ");
    }
    let meaningful: String = stripped
        .chars()
        .filter(|ch| ch.is_alphanumeric() || ch.is_whitespace())
        .collect();
    meaningful.trim().chars().count() < 24
}

pub(super) fn redact_structural_opaque_tokens(raw: &str) -> StructuralOpaqueTokenRedaction {
    if user_memory_is_opaque_token_shape(raw) {
        return StructuralOpaqueTokenRedaction {
            text: "[REDACTED_SECRET]".to_string(),
            redacted_secret: true,
            mostly_secret: true,
        };
    }

    let mut redacted = String::with_capacity(raw.len());
    let mut last = 0usize;
    let mut run_start: Option<usize> = None;
    let mut redacted_secret = false;

    for (idx, ch) in raw.char_indices() {
        if user_memory_token_shape_char(ch) {
            if run_start.is_none() {
                run_start = Some(idx);
            }
            continue;
        }

        if let Some(start) = run_start.take() {
            let candidate = &raw[start..idx];
            if user_memory_is_opaque_token_shape(candidate) {
                redacted.push_str(&raw[last..start]);
                redacted.push_str("[REDACTED_SECRET]");
                last = idx;
                redacted_secret = true;
            }
        }
    }

    if let Some(start) = run_start {
        let candidate = &raw[start..];
        if user_memory_is_opaque_token_shape(candidate) {
            redacted.push_str(&raw[last..start]);
            redacted.push_str("[REDACTED_SECRET]");
            last = raw.len();
            redacted_secret = true;
        }
    }

    if redacted_secret {
        redacted.push_str(&raw[last..]);
    } else {
        redacted = raw.to_string();
    }

    StructuralOpaqueTokenRedaction {
        text: redacted,
        redacted_secret,
        mostly_secret: false,
    }
}

pub(super) fn sanitize_user_memory_metadata_text_for_storage(
    raw: &str,
    max_chars: usize,
) -> Option<SanitizedUserMemoryText> {
    let mut value = normalize_user_memory_text(raw, max_chars)?;
    let structural = redact_structural_opaque_tokens(&value);
    if structural.mostly_secret {
        return None;
    }
    let mut redacted_secret = structural.redacted_secret;
    value = structural.text;
    if user_memory_is_mostly_redaction_marker_payload(&value) {
        return None;
    }
    if user_memory_contains_redaction_marker(&value) {
        redacted_secret = true;
    }

    let redaction = crate::security::redact_secret_input(&value);
    if !redaction.had_secret() {
        return Some(SanitizedUserMemoryText {
            text: value,
            redacted_secret,
        });
    }
    if redaction.is_mostly_secret_payload() {
        return None;
    }
    let text = normalize_user_memory_text(&redaction.text, max_chars)?;
    if user_memory_is_mostly_redaction_marker_payload(&text) {
        return None;
    }
    Some(SanitizedUserMemoryText {
        text,
        redacted_secret: true,
    })
}

pub(super) fn sanitize_learned_user_memory_content_for_storage(
    key: &str,
    raw_value: &str,
) -> Option<SanitizedUserMemoryContent> {
    let sanitized_value = sanitize_user_memory_metadata_text_for_storage(raw_value, 320)?;
    let value = sanitized_value.text;
    let content = format!("{}: {}", key, value);
    Some(SanitizedUserMemoryContent {
        value,
        content,
        redacted_secret: sanitized_value.redacted_secret,
    })
}

pub(super) fn sanitize_user_memory_prompt_text(
    raw: &str,
    max_chars: usize,
) -> Option<SanitizedUserMemoryText> {
    sanitize_user_memory_metadata_text_for_storage(raw, max_chars)
}

pub(super) fn sanitize_user_memory_metadata_string_field_for_storage(
    metadata: &mut serde_json::Map<String, serde_json::Value>,
    field: &str,
    max_chars: usize,
) -> bool {
    let Some(raw) = metadata
        .get(field)
        .and_then(|value| value.as_str())
        .map(str::to_string)
    else {
        return false;
    };
    match sanitize_user_memory_metadata_text_for_storage(&raw, max_chars) {
        Some(value) => {
            metadata.insert(field.to_string(), serde_json::Value::String(value.text));
            value.redacted_secret
        }
        None => {
            metadata.remove(field);
            true
        }
    }
}

pub(super) fn sanitize_user_memory_merged_phrasings_for_storage(
    metadata: &mut serde_json::Map<String, serde_json::Value>,
) -> bool {
    let Some(items) = metadata
        .get_mut("merged_phrasings")
        .and_then(|value| value.as_array_mut())
    else {
        return false;
    };
    let mut redacted_secret = false;
    for entry in items {
        let Some(object) = entry.as_object_mut() else {
            continue;
        };
        let Some(raw_value) = object
            .get("value")
            .and_then(|value| value.as_str())
            .map(str::to_string)
        else {
            continue;
        };
        match sanitize_user_memory_metadata_text_for_storage(&raw_value, 320) {
            Some(value) => {
                if value.redacted_secret {
                    redacted_secret = true;
                }
                object.insert("value".to_string(), serde_json::Value::String(value.text));
            }
            None => {
                object.remove("value");
                redacted_secret = true;
            }
        }
    }
    redacted_secret
}

pub(super) fn sanitize_user_memory_metadata_for_storage(
    metadata: &mut serde_json::Map<String, serde_json::Value>,
) -> bool {
    let reason_redacted =
        sanitize_user_memory_metadata_string_field_for_storage(metadata, "reason", 180);
    let merged_phrasings_redacted = sanitize_user_memory_merged_phrasings_for_storage(metadata);
    reason_redacted || merged_phrasings_redacted
}

pub(super) fn append_learned_user_memory_merged_phrasing(
    metadata: &mut serde_json::Map<String, serde_json::Value>,
    key: Option<&str>,
    value: Option<&str>,
    source: Option<&str>,
    now_iso: &str,
) -> bool {
    let mut redacted_secret = false;
    let mut entry = serde_json::Map::new();
    if let Some(key) = key.and_then(normalize_user_fact_key) {
        entry.insert("key".to_string(), serde_json::Value::String(key));
    }
    if let Some(raw_value) = value {
        match sanitize_user_memory_metadata_text_for_storage(raw_value, 320) {
            Some(value) => {
                if value.redacted_secret {
                    redacted_secret = true;
                }
                entry.insert("value".to_string(), serde_json::Value::String(value.text));
            }
            None => {
                redacted_secret = true;
            }
        }
    }
    if let Some(source) = source
        .and_then(|raw| normalize_user_memory_text(raw, 80))
        .filter(|value| !value.is_empty())
    {
        entry.insert("source".to_string(), serde_json::Value::String(source));
    }
    entry.insert(
        "at".to_string(),
        serde_json::Value::String(now_iso.to_string()),
    );
    let duplicate = metadata
        .get("merged_phrasings")
        .and_then(|value| value.as_array())
        .map(|items| items.iter().any(|item| item.as_object() == Some(&entry)))
        .unwrap_or(false);
    if duplicate {
        return redacted_secret;
    }
    let list = metadata
        .entry("merged_phrasings".to_string())
        .or_insert_with(|| serde_json::Value::Array(Vec::new()));
    let serde_json::Value::Array(items) = list else {
        *list = serde_json::Value::Array(vec![serde_json::Value::Object(entry)]);
        return redacted_secret;
    };
    items.push(serde_json::Value::Object(entry));
    while items.len() > crate::core::memory_dedup::MAX_MERGED_PHRASINGS {
        items.remove(0);
    }
    redacted_secret
}

pub(super) fn user_memory_json_text_field(
    item: &serde_json::Value,
    field: &str,
    max_chars: usize,
) -> Option<String> {
    let value = item.get(field)?;
    match value {
        serde_json::Value::String(raw) => normalize_user_memory_text(raw, max_chars),
        serde_json::Value::Null => None,
        other => normalize_user_memory_text(&other.to_string(), max_chars),
    }
}

pub(super) fn user_memory_capture_item_looks_sensitive(item: &serde_json::Value) -> bool {
    item.get("looks_sensitive")
        .and_then(|value| value.as_bool())
        .unwrap_or(false)
}

pub(super) fn user_memory_capture_item_sensitivity(
    item: &serde_json::Value,
    key: &str,
    value: &str,
    kind: Option<&str>,
) -> MemorySensitivity {
    let _ = (key, value, kind);
    let model_sensitivity = item
        .get("sensitivity")
        .and_then(|value| value.as_str())
        .and_then(|value| normalize_memory_sensitivity(Some(value)));
    model_sensitivity.unwrap_or(MemorySensitivity::Sensitive)
}

fn memory_candidate_contains_any(text: &str, needles: &[&str]) -> bool {
    needles.iter().any(|needle| text.contains(needle))
}

pub(super) fn user_memory_candidate_is_operational_artifact(
    key: &str,
    value: &str,
    kind: &str,
    category: &str,
) -> bool {
    let text = format!(
        "{} {} {} {}",
        key.trim(),
        value.trim(),
        kind.trim(),
        category.trim()
    )
    .to_ascii_lowercase();
    let integration_or_runtime = memory_candidate_contains_any(
        &text,
        &[
            "integration",
            "oauth",
            "api key",
            "apikey",
            "bearer token",
            "access token",
            "authentication",
            "authorization",
            "credential",
            "endpoint",
            "graphql api",
            "webhook",
            "environment variable",
            "env var",
        ],
    );
    let lifecycle_state = memory_candidate_contains_any(
        &text,
        &[
            "installed",
            "scaffolded",
            "set up",
            "setup",
            "configured",
            "authenticated",
            "not authenticated",
            "not yet authenticated",
            "not yet fully authenticated",
            "pending auth",
            "pending authentication",
            "requires auth",
            "requires authentication",
            "using bearer",
            "with bearer",
            "via bearer",
            "created",
            "updated",
            "deployed",
            "registered",
        ],
    );
    integration_or_runtime && lifecycle_state
}

pub(super) fn user_memory_json_datetime_field(
    item: &serde_json::Value,
    field: &str,
) -> Option<chrono::DateTime<chrono::Utc>> {
    item.get(field)
        .and_then(|value| value.as_str())
        .and_then(parse_ambient_rfc3339)
}

pub(super) fn normalize_user_memory_kind(raw: Option<&str>) -> &'static str {
    match raw.unwrap_or_default().trim().to_ascii_lowercase().as_str() {
        "constraint" | "rule" | "operating_rule" | "workflow_constraint" => "constraint",
        _ => "personal_fact",
    }
}

pub(super) fn normalize_self_memory_lookup_kind(raw: Option<&str>) -> &'static str {
    match raw.unwrap_or_default().trim().to_ascii_lowercase().as_str() {
        "identity" | "name" | "identity_or_name" => "identity",
        "location" | "address" | "home" | "residence" => "location",
        "timezone" | "time_zone" | "tz" => "timezone",
        "relationship" => "relationship",
        "preference" | "preferences" | "taste" | "language" | "tone" => "preference",
        "assistant_preference" => "assistant_preference",
        "work_preference" => "work_preference",
        "project_domain_memory" | "domain_memory" => "project_domain_memory",
        "ephemeral_context" => "ephemeral_context",
        "knowledge" => "knowledge",
        "constraint" | "rule" | "workflow_constraint" => "constraint",
        "contact" | "email" | "phone" => "contact",
        "" | "any" | "all" | "profile" | "memory" => "any",
        _ => "other",
    }
}

pub(super) fn infer_self_memory_kind_from_internal_key(key: &str) -> &'static str {
    let normalized = normalize_user_fact_key(key)
        .unwrap_or_else(|| key.trim().to_ascii_lowercase().replace('-', "_"));
    if normalized.starts_with("rule_") {
        "constraint"
    } else if normalized == "user_name" {
        "identity"
    } else if normalized == "user_address"
        || normalized.contains("location")
        || normalized.contains("address")
    {
        "location"
    } else if normalized.contains("timezone") {
        "timezone"
    } else if normalized.contains("email") || normalized.contains("phone") {
        "contact"
    } else if normalized.starts_with("likes_")
        || normalized.starts_with("dislikes_")
        || normalized.contains("preference")
        || normalized.contains("language")
        || normalized.contains("tone")
    {
        "preference"
    } else {
        "other"
    }
}

pub(super) fn learned_user_memory_semantic_kind(
    key: Option<&str>,
    raw_kind: Option<&str>,
) -> &'static str {
    let normalized = normalize_self_memory_lookup_kind(raw_kind);
    if normalized != "other"
        || raw_kind
            .unwrap_or_default()
            .trim()
            .eq_ignore_ascii_case("other")
    {
        return normalized;
    }
    key.map(infer_self_memory_kind_from_internal_key)
        .unwrap_or("other")
}

pub(super) fn normalize_learned_user_memory_category(
    raw_category: Option<&str>,
    semantic_kind: Option<&str>,
) -> &'static str {
    crate::core::memory_schema::normalize_memory_category(raw_category, semantic_kind)
}

pub(super) fn learned_user_memory_category(
    item: &crate::storage::experience_item::Model,
) -> &'static str {
    let semantic_kind = learned_user_memory_lookup_kind(item);
    crate::core::memory_schema::memory_category_from_metadata(&item.metadata, Some(semantic_kind))
}

pub(super) fn learned_user_memory_topics(
    item: &crate::storage::experience_item::Model,
) -> Vec<String> {
    crate::core::memory_schema::normalize_memory_topics(item.metadata.get("topics"), 8)
}

pub(super) fn learned_user_memory_matches_exact_scope(
    item: &crate::storage::experience_item::Model,
    scope: &str,
    project_id: Option<&str>,
    conversation_id: Option<&str>,
) -> bool {
    item.scope == scope
        && item.project_id.as_deref() == project_id
        && item.conversation_id.as_deref() == conversation_id
}

pub(super) fn learned_user_memory_merged_history_contains_key(
    item: &crate::storage::experience_item::Model,
    key: &str,
) -> bool {
    let Some(key) = normalize_user_fact_key(key) else {
        return false;
    };
    item.metadata
        .get("merged_phrasings")
        .and_then(|value| value.as_array())
        .map(|entries| {
            entries.iter().any(|entry| {
                entry
                    .get("key")
                    .and_then(|value| value.as_str())
                    .and_then(normalize_user_fact_key)
                    .as_deref()
                    == Some(key.as_str())
            })
        })
        .unwrap_or(false)
}

pub(super) fn learned_user_memory_key(
    item: &crate::storage::experience_item::Model,
) -> Option<String> {
    ambient_metadata_text_field(item, "key", 80).or_else(|| {
        item.content
            .split_once(':')
            .and_then(|(key, _)| normalize_user_memory_text(key, 80))
    })
}

pub(super) fn learned_user_memory_key_matches(
    item: &crate::storage::experience_item::Model,
    key: &str,
) -> bool {
    let Some(key) = normalize_user_fact_key(key) else {
        return false;
    };
    learned_user_memory_key(item)
        .and_then(|existing_key| normalize_user_fact_key(&existing_key))
        .as_deref()
        == Some(key.as_str())
        || learned_user_memory_merged_history_contains_key(item, &key)
}

pub(super) fn learned_user_memory_value(
    item: &crate::storage::experience_item::Model,
) -> Option<String> {
    item.content
        .split_once(':')
        .map(|(_, value)| value)
        .and_then(|value| normalize_user_memory_text(value, 220))
        .or_else(|| normalize_user_memory_text(&item.content, 220))
}

fn normalize_user_memory_equivalence_text(raw: &str) -> Option<String> {
    normalize_user_memory_text(raw, 500).map(|value| {
        value
            .to_ascii_lowercase()
            .split_whitespace()
            .collect::<Vec<_>>()
            .join(" ")
    })
}

pub(super) fn learned_user_memory_lookup_kind(
    item: &crate::storage::experience_item::Model,
) -> &'static str {
    if let Some(kind) = ambient_metadata_text_field(item, "memory_kind", 48) {
        let normalized = normalize_self_memory_lookup_kind(Some(kind.as_str()));
        if normalized != "other" || kind.eq_ignore_ascii_case("other") {
            return normalized;
        }
    }
    learned_user_memory_key(item)
        .as_deref()
        .map(infer_self_memory_kind_from_internal_key)
        .unwrap_or("other")
}

pub(super) fn learned_user_memory_durability(
    item: &crate::storage::experience_item::Model,
) -> String {
    ambient_metadata_text_field(item, "durability", 32).unwrap_or_else(|| "permanent".to_string())
}

pub(super) fn learned_user_memory_sensitivity(
    item: &crate::storage::experience_item::Model,
) -> MemorySensitivity {
    let key = learned_user_memory_key(item);
    let value = learned_user_memory_value(item).unwrap_or_else(|| item.content.clone());
    let kind = learned_user_memory_lookup_kind(item);
    let stored_sensitivity = ambient_metadata_text_field(item, "sensitivity", 48);
    saved_memory_sensitivity_from_parts(
        key.as_deref(),
        &value,
        Some(kind),
        stored_sensitivity.as_deref(),
    )
}

pub(super) fn should_inject_learned_user_memory(
    item: &crate::storage::experience_item::Model,
    now: chrono::DateTime<chrono::Utc>,
) -> bool {
    learned_user_memory_active(item, now)
        && saved_memory_is_prompt_safe(learned_user_memory_sensitivity(item))
}

#[derive(Debug, Clone)]
struct SavedUserMemoryPromptCandidate {
    line: String,
    category: &'static str,
    score: f32,
    scope_rank: u8,
    confidence: f64,
    support_count: i32,
    updated_at: String,
}

fn saved_user_memory_scope_rank(
    item: &crate::storage::experience_item::Model,
    project_id: Option<&str>,
    conversation_id: Option<&str>,
) -> u8 {
    let mut rank = 0u8;
    if project_id.is_some() && item.project_id.as_deref() == project_id {
        rank = rank.saturating_add(1);
    }
    if conversation_id.is_some() && item.conversation_id.as_deref() == conversation_id {
        rank = rank.saturating_add(2);
    }
    rank
}

fn saved_user_memory_is_context_scoped(
    item: &crate::storage::experience_item::Model,
    project_id: Option<&str>,
    conversation_id: Option<&str>,
) -> bool {
    saved_user_memory_scope_rank(item, project_id, conversation_id) > 0
}

fn saved_user_memory_dense_score(
    query_embedding: Option<&PgVector>,
    item: &crate::storage::experience_item::Model,
) -> Option<f32> {
    crate::core::document_search::normalized_embedding_similarity(
        query_embedding?.as_slice(),
        item.embedding.as_ref()?.as_slice(),
    )
    .map(|score| score.clamp(0.0, 1.0))
}

fn saved_user_memory_candidate(
    item: &crate::storage::experience_item::Model,
    project_id: Option<&str>,
    conversation_id: Option<&str>,
    now: chrono::DateTime<chrono::Utc>,
    query_embedding: Option<&PgVector>,
) -> Option<SavedUserMemoryPromptCandidate> {
    if !should_inject_learned_user_memory(item, now) {
        return None;
    }
    let category = learned_user_memory_category(item);
    let scope_rank = saved_user_memory_scope_rank(item, project_id, conversation_id);
    if crate::core::memory_schema::memory_category_is_ephemeral(category)
        && !saved_user_memory_is_context_scoped(item, project_id, conversation_id)
    {
        return None;
    }
    let dense_score = saved_user_memory_dense_score(query_embedding, item);
    if crate::core::memory_schema::memory_category_requires_topical_relevance(category)
        && scope_rank == 0
        && dense_score.unwrap_or(0.0) < 0.42
    {
        return None;
    }
    let line = format_learned_user_memory_for_prompt(item, now)?;
    let score = (0.38 * dense_score.unwrap_or(0.0))
        + (0.18 * (scope_rank as f32 / 3.0))
        + (0.26 * item.confidence.clamp(0.0, 1.0) as f32)
        + (0.10 * ((item.support_count.max(0) as f32) / 6.0).min(1.0));
    Some(SavedUserMemoryPromptCandidate {
        line,
        category,
        score,
        scope_rank,
        confidence: item.confidence,
        support_count: item.support_count,
        updated_at: item.updated_at.clone(),
    })
}

async fn build_saved_user_facts_context_from_storage(
    storage: &crate::storage::Storage,
    embedding_client: Option<&EmbeddingClient>,
    project_id: Option<&str>,
    conversation_id: Option<&str>,
    current_message: &str,
) -> Option<String> {
    let now = chrono::Utc::now();
    let learned_items = storage
        .list_active_experience_items(
            SAVED_USER_FACT_PROMPT_KINDS,
            project_id,
            conversation_id,
            80,
        )
        .await
        .unwrap_or_default();
    let expired_item_ids = learned_items
        .iter()
        .filter(|item| learned_user_memory_expired(item, now))
        .map(|item| item.id.clone())
        .collect::<Vec<_>>();
    for id in expired_item_ids {
        if let Err(error) = storage.update_experience_item_status(&id, "expired").await {
            tracing::warn!("Failed to expire learned user memory '{}': {}", id, error);
        }
    }
    let query_embedding = match embedding_client {
        Some(embedder) => {
            let query = crate::security::redact_secret_input(current_message).text;
            let query = query.trim();
            if query.is_empty() {
                None
            } else {
                embedder
                    .embed_texts(&[safe_truncate(query, 1_200)])
                    .await
                    .ok()
                    .and_then(|mut embeddings| embeddings.pop())
            }
        }
        None => None,
    };
    let mut candidates = learned_items
        .iter()
        .filter_map(|item| {
            saved_user_memory_candidate(
                item,
                project_id,
                conversation_id,
                now,
                query_embedding.as_ref(),
            )
        })
        .collect::<Vec<_>>();
    candidates.sort_by(|left, right| {
        right
            .score
            .total_cmp(&left.score)
            .then_with(|| right.scope_rank.cmp(&left.scope_rank))
            .then_with(|| right.confidence.total_cmp(&left.confidence))
            .then_with(|| right.support_count.cmp(&left.support_count))
            .then_with(|| right.updated_at.cmp(&left.updated_at))
    });
    let mut category_counts: HashMap<&'static str, usize> = HashMap::new();
    let mut lines = Vec::new();
    for candidate in candidates {
        let cap = crate::core::memory_schema::memory_category_prompt_cap(candidate.category);
        let count = category_counts.entry(candidate.category).or_insert(0);
        if *count >= cap {
            continue;
        }
        *count += 1;
        lines.push(candidate.line);
        if lines.len() >= 8 {
            break;
        }
    }
    if lines.is_empty() {
        None
    } else {
        Some(format!(
            "## Saved User Memory\nThese are the top relevant saved memories after category caps. Profile facts and assistant preferences may be generally reusable; work, project/domain, knowledge, and ephemeral memories are included only when scoped or semantically relevant. Use active temporary memories only within their validity window.\n{}",
            lines.join("\n")
        ))
    }
}

pub(super) fn normalize_user_memory_durability(raw: Option<&str>) -> &'static str {
    match raw.unwrap_or_default().trim().to_ascii_lowercase().as_str() {
        "temporary" => "temporary",
        "situational" => "situational",
        _ => "permanent",
    }
}

pub(super) fn normalize_user_memory_scope(raw: Option<&str>) -> &'static str {
    match raw.unwrap_or_default().trim().to_ascii_lowercase().as_str() {
        "project" => "project",
        "conversation" => "conversation",
        _ => "global",
    }
}

pub(super) fn build_user_memory_capture_prompt(
    time_context: &str,
    recent_dialogue: &str,
    message: &str,
    saved_facts: &str,
    response_shape: &str,
) -> String {
    format!(
        "Current time:\n{time_context}\n\nRecent dialogue, context only:\n{recent_dialogue}\n\nSource message, authoritative for new memory:\n{message}\n\nCurrent saved user facts:\n{saved_facts}\n\nReturn JSON only with this shape:\n{response_shape}\n\nRules:\n- Always return the full JSON object with both `memories` and `retractions` keys present as arrays. Use empty arrays when nothing applies. Never omit either key and never return `{{}}` or any abbreviated shape.\n- Extract only memories that are useful beyond this turn and beyond any task/session/work item created by this turn: profile facts, assistant preferences, work preferences, reusable project/domain memory, reusable knowledge, or durable cross-context workflow rules.\n- Treat the source message as the only authoritative source for new memory. Use recent dialogue and saved facts only to resolve references, contradictions, retractions, and scope.\n- Do not create memories from assistant-authored messages, tool outputs, status reports, completed-work summaries, or durable-work records unless the source user message explicitly states the durable fact/preference or asks to remember it.\n- Decide semantically from the source message and context. Do not use fixed phrases, keyword matching, regular expressions, literal wording patterns, or manually predicted variants of what the user might say.\n- Classify every memory into exactly one category: profile_fact for identity, location, contact, job, relationships, and stable user details; assistant_preference for how the assistant should address, format, phrase, or interact with the user; work_preference for the user's durable analysis, source, modeling, coding, review, or workflow preferences; project_domain_memory for durable facts, assumptions, principles, and constraints that should only be reused when the current topic or project is semantically related; ephemeral_context for short-lived context that is useful only in the current conversation; knowledge for reusable non-personal knowledge.\n- Add concise `topics` for topical memories so retrieval can use semantic relevance instead of injecting every memory into every prompt. Topics must describe meaning, domain, project, or task family, not surface wording.\n- Do not store every interesting claim. For work_preference and project_domain_memory, save only explicit user preferences, durable reusable principles, or high-confidence recurring patterns, not one-off analysis details.\n- Treat the source message compositionally. A single user message can contain both durable user information and a live question, request, clarification, follow-up, or correction.\n- Capture durable self-information even when the same source message also asks for help, asks a question, or contains multiple clauses or intents.\n- If recent dialogue shows the assistant was missing a user fact and the source message supplies that fact, capture the supplied fact even when the source message immediately continues with another request.\n- Classify each memory with sensitivity. Use prompt_safe for ordinary preferences and operating style, personal_identifier for identity/contact/location facts, sensitive for private health, finance, legal, relationship, belief, or similarly private facts, and crisis_sensitive for acute distress, self-harm risk, unsafe-place, immediate safety, or coping facts.\n- Sensitive and crisis_sensitive self-memory should still be captured when it is useful beyond this turn; sensitivity controls later prompt injection, not whether the memory may exist.\n- If the user's intent is to stop retaining a previously stored fact, preference, or constraint, emit a retraction for the matching semantic memory instead of a new memory.\n- Do not let interrogative wording, mixed intents, corrections, or extra context suppress a durable memory action the user just expressed.\n- Prefer stable semantic key naming so corrected or updated facts replace stale versions instead of forking into near-duplicate keys.\n- Permanent memories have no expiry unless later contradicted, retracted, or superseded.\n- Temporary memories must include a concrete expires_at when the source message gives or strongly implies a time window.\n- Situational memories are useful now but have an uncertain end; include review_at when a later review is appropriate.\n- Use global scope only for information that is generally reusable; topic-specific domain memory should usually be project or conversation scoped when a project/conversation owns it, and otherwise must include topics.\n- Task-specific configuration, schedule details, watcher conditions, notification channels for a specific object, execution status, retries, pending setup, and tool-operation state belong to the relevant task/session/work item, not ArkMemory.\n- Do not capture integration setup/install/authentication state, API endpoint setup, credential scheme, environment-variable requirements, or pending authorization status as memory; those belong to integration records, tasks, traces, or logs.\n- Set looks_sensitive=true only when the candidate is credential-like, token-like, password/private-key/auth material, or otherwise unsafe to store even as private personal memory; do not use looks_sensitive for ordinary private self-memory, health, distress, identity, or location facts.\n- If looks_sensitive=true, include a concise sensitive_reason and do not rely on redaction markers as useful memory content.\n- Do not capture one-off requests, assistant claims, tool output, transient errors, unsupported guesses, operational setup details, pending/retry status, object-specific task/session/watcher configuration, or sensitive credential material as memories.\n- It is okay to return empty memories and/or retractions arrays, but the keys themselves must be present.\n- Do not invent facts beyond the source message, recent dialogue, current time, or current saved facts.",
        time_context = time_context,
        recent_dialogue = recent_dialogue,
        message = message,
        saved_facts = saved_facts,
        response_shape = response_shape
    )
}

pub(super) fn build_user_memory_candidate_semantic_review_prompt(
    source_message: &str,
    recent_dialogue: &str,
    candidate: &serde_json::Value,
) -> String {
    let candidate_json = serde_json::to_string(candidate).unwrap_or_else(|_| "{}".to_string());
    format!(
        "Review one proposed ArkMemory candidate before it can be saved.\n\nRecent dialogue, context only:\n{recent_dialogue}\n\nSource user message, authoritative:\n{source_message}\n\nProposed memory candidate:\n{candidate_json}\n\nReturn JSON only with this shape:\n{{\"store\":false,\"confidence\":0.0,\"reason\":\"brief semantic rationale\"}}\n\nRules:\n- Decide from the user's underlying intent and the candidate's meaning, not wording, keyword presence, formatting, casing, grammar, or word order.\n- Set store=true only when the candidate is durable user memory: a reusable user fact, durable preference, operating constraint, reusable project/domain fact, or explicit memory retraction signal that should remain useful after the current task/session/work item is complete.\n- Set store=false when the candidate mainly belongs in operational state instead of ArkMemory: task state, watcher or automation configuration, scheduling, trigger/condition details, notification routing for a particular work item, integration setup/auth state, tool-run state, execution status, retry/pending state, or a one-off request.\n- A general durable preference can still be stored even if the source turn also asks for work. A work item configuration should not become memory just because it mentions a preferred tool, channel, source, or destination.\n- If the candidate would become stale when the specific requested task, watcher, automation, or setup record is changed, completed, or deleted, set store=false.\n- When the distinction is genuinely unclear, set store=true and explain the uncertainty rather than suppressing a potentially durable memory.\n- confidence must describe confidence in the store decision and must be between 0.0 and 1.0.",
        recent_dialogue = recent_dialogue,
        source_message = source_message,
        candidate_json = candidate_json
    )
}

pub(super) fn user_memory_capture_payload_has_required_shape(payload: &serde_json::Value) -> bool {
    payload
        .get("memories")
        .is_some_and(|value| value.is_array())
        && payload
            .get("retractions")
            .is_some_and(|value| value.is_array())
}

pub(super) fn user_memory_capture_payload_is_empty_decision(payload: &serde_json::Value) -> bool {
    payload
        .get("memories")
        .and_then(|value| value.as_array())
        .is_some_and(|items| items.is_empty())
        && payload
            .get("retractions")
            .and_then(|value| value.as_array())
            .is_some_and(|items| items.is_empty())
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(super) enum UserMemoryCapturePayloadError {
    Unparseable,
    IncompleteShape,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(super) enum UserMemoryCapturePayloadDisposition {
    Exact,
    ShapeRecovered,
}

#[derive(Debug, Clone, PartialEq)]
pub(super) struct ParsedUserMemoryCapturePayload {
    pub(super) payload: serde_json::Value,
    pub(super) disposition: UserMemoryCapturePayloadDisposition,
}

#[derive(Debug, Clone, Serialize)]
pub(super) struct UserMemoryCaptureAttemptRecord {
    pub(super) slot_id: String,
    pub(super) slot_label: String,
    pub(super) role: String,
    pub(super) provider: Option<String>,
    pub(super) model: Option<String>,
    pub(super) stage: String,
    pub(super) request_kind: String,
    pub(super) outcome: String,
    pub(super) error: Option<String>,
}

#[derive(Debug, Clone)]
pub(super) struct UserMemoryCaptureRunOutcome {
    pub(super) payload: Option<serde_json::Value>,
    pub(super) attempts: Vec<UserMemoryCaptureAttemptRecord>,
    pub(super) selected_slot_id: Option<String>,
    pub(super) selected_slot_label: Option<String>,
    pub(super) selected_provider: Option<String>,
    pub(super) selected_model: Option<String>,
    pub(super) selected_stage: Option<String>,
    pub(super) terminal_error: Option<String>,
}

#[derive(Debug, Clone, PartialEq)]
pub(super) struct UserMemoryCaptureEmptyVerdict {
    pub(super) has_durable_memory: bool,
    pub(super) confidence: f32,
    pub(super) reason: String,
}

#[derive(Debug, Clone)]
pub(super) struct UserMemoryCaptureFocusedRecovery {
    pub(super) payload: serde_json::Value,
    pub(super) selected_slot_id: String,
    pub(super) selected_slot_label: String,
    pub(super) selected_provider: String,
    pub(super) selected_model: String,
    pub(super) selected_stage: String,
}

#[derive(Debug, Clone, PartialEq)]
pub(super) struct UserMemoryCandidateSemanticReview {
    pub(super) store: bool,
    pub(super) confidence: f32,
    pub(super) reason: Option<String>,
}

pub(super) fn parse_user_memory_candidate_semantic_review(
    raw: &str,
) -> Option<UserMemoryCandidateSemanticReview> {
    let payload = extract_json_object_from_text(raw)?;
    let store = payload.get("store").and_then(|value| value.as_bool())?;
    let confidence = payload
        .get("confidence")
        .and_then(|value| value.as_f64())
        .unwrap_or(0.0)
        .clamp(0.0, 1.0) as f32;
    let reason = payload
        .get("reason")
        .and_then(|value| value.as_str())
        .and_then(|value| normalize_user_memory_text(value, 180));
    Some(UserMemoryCandidateSemanticReview {
        store,
        confidence,
        reason,
    })
}

pub(super) fn user_memory_candidate_review_should_skip(
    review: &UserMemoryCandidateSemanticReview,
) -> bool {
    !review.store
}

pub(super) fn coerce_user_memory_capture_array_field(
    value: Option<&serde_json::Value>,
) -> Option<serde_json::Value> {
    match value {
        None | Some(serde_json::Value::Null) => Some(serde_json::Value::Array(Vec::new())),
        Some(serde_json::Value::Array(items)) => Some(serde_json::Value::Array(items.clone())),
        Some(field @ serde_json::Value::Object(_)) => {
            Some(serde_json::Value::Array(vec![field.clone()]))
        }
        _ => None,
    }
}

pub(super) fn user_memory_capture_item_looks_like_retraction(item: &serde_json::Value) -> bool {
    let Some(object) = item.as_object() else {
        return false;
    };
    object.get("key").and_then(|value| value.as_str()).is_some() && object.get("value").is_none()
}

pub(super) fn user_memory_capture_item_looks_like_memory(item: &serde_json::Value) -> bool {
    let Some(object) = item.as_object() else {
        return false;
    };
    object.get("key").and_then(|value| value.as_str()).is_some()
        && object
            .get("value")
            .and_then(|value| value.as_str())
            .is_some()
}

pub(super) fn user_memory_capture_field_matches_kind(
    canonical_field: &str,
    _field_name: &str,
    value: &serde_json::Value,
) -> bool {
    let structural_match = match value {
        serde_json::Value::Array(items) if items.is_empty() => false,
        serde_json::Value::Array(items) => match canonical_field {
            "memories" => items.iter().all(user_memory_capture_item_looks_like_memory),
            "retractions" => items
                .iter()
                .all(user_memory_capture_item_looks_like_retraction),
            _ => false,
        },
        serde_json::Value::Object(_) => match canonical_field {
            "memories" => user_memory_capture_item_looks_like_memory(value),
            "retractions" => user_memory_capture_item_looks_like_retraction(value),
            _ => false,
        },
        _ => false,
    };
    if structural_match {
        return true;
    }
    false
}

pub(super) fn recover_user_memory_capture_arrayish_field<'a>(
    object: &'a serde_json::Map<String, serde_json::Value>,
    canonical_field: &str,
) -> Option<&'a serde_json::Value> {
    if let Some(value) = object.get(canonical_field) {
        return Some(value);
    }

    let mut matched = None;
    for (field_name, value) in object {
        if !matches!(
            value,
            serde_json::Value::Null | serde_json::Value::Array(_) | serde_json::Value::Object(_)
        ) {
            continue;
        }
        if user_memory_capture_field_matches_kind(canonical_field, field_name, value) {
            if matched.is_some() {
                return None;
            }
            matched = Some(value);
        }
    }
    matched
}

pub(super) fn recover_user_memory_capture_payload_shape(
    payload: &serde_json::Value,
) -> Option<serde_json::Value> {
    let object = payload.as_object()?;
    let recovered_memories = recover_user_memory_capture_arrayish_field(object, "memories");
    let recovered_retractions = recover_user_memory_capture_arrayish_field(object, "retractions");
    let has_known_memory_shape = object.is_empty()
        || object.contains_key("memories")
        || object.contains_key("retractions")
        || recovered_memories.is_some()
        || recovered_retractions.is_some();
    if !has_known_memory_shape {
        return None;
    }
    let memories = coerce_user_memory_capture_array_field(recovered_memories)?;
    let retractions = coerce_user_memory_capture_array_field(recovered_retractions)?;
    let mut recovered = object.clone();
    recovered.insert("memories".to_string(), memories);
    recovered.insert("retractions".to_string(), retractions);
    Some(serde_json::Value::Object(recovered))
}

pub(super) fn parse_user_memory_capture_payload(
    raw: &str,
) -> Result<ParsedUserMemoryCapturePayload, UserMemoryCapturePayloadError> {
    let Some(payload) = extract_json_object_from_text(raw) else {
        return Err(UserMemoryCapturePayloadError::Unparseable);
    };
    if user_memory_capture_payload_has_required_shape(&payload) {
        return Ok(ParsedUserMemoryCapturePayload {
            payload,
            disposition: UserMemoryCapturePayloadDisposition::Exact,
        });
    }
    let Some(payload) = recover_user_memory_capture_payload_shape(&payload) else {
        return Err(UserMemoryCapturePayloadError::IncompleteShape);
    };
    Ok(ParsedUserMemoryCapturePayload {
        payload,
        disposition: UserMemoryCapturePayloadDisposition::ShapeRecovered,
    })
}

pub(super) fn user_memory_capture_response_preview(raw: &str, max_chars: usize) -> String {
    safe_truncate(&crate::security::redact_secret_input(raw).text, max_chars)
}

pub(super) fn user_memory_capture_payload_ok_outcome(
    parsed: &ParsedUserMemoryCapturePayload,
    is_empty_decision: bool,
) -> &'static str {
    match (is_empty_decision, parsed.disposition) {
        (false, UserMemoryCapturePayloadDisposition::Exact) => "ok",
        (false, UserMemoryCapturePayloadDisposition::ShapeRecovered) => "ok_shape_recovered",
        (true, UserMemoryCapturePayloadDisposition::Exact) => "empty_decision",
        (true, UserMemoryCapturePayloadDisposition::ShapeRecovered) => {
            "empty_decision_shape_recovered"
        }
    }
}

pub(super) fn user_memory_capture_attempts_all_transport_failed(
    attempts: &[UserMemoryCaptureAttemptRecord],
) -> bool {
    !attempts.is_empty()
        && attempts
            .iter()
            .all(|attempt| attempt.outcome == "transport_failed")
}

pub(super) fn user_memory_capture_attempts_timed_out(
    attempts: &[UserMemoryCaptureAttemptRecord],
) -> bool {
    attempts.iter().any(|attempt| {
        attempt.outcome == "transport_failed"
            && attempt
                .error
                .as_deref()
                .map(|error| error.contains("kind=timeout"))
                .unwrap_or(false)
    })
}

pub(super) fn user_memory_capture_terminal_error(
    attempts: &[UserMemoryCaptureAttemptRecord],
    attempted_candidates: usize,
    timeout_ms: u64,
) -> String {
    if user_memory_capture_attempts_all_transport_failed(attempts) {
        if user_memory_capture_attempts_timed_out(attempts) {
            return format!(
                "Memory capture timed out after {}ms across {} candidate(s).",
                timeout_ms, attempted_candidates
            );
        }
        return format!(
            "Memory capture transport failed across {} candidate(s).",
            attempted_candidates
        );
    }
    format!(
        "Memory capture failed to produce schema-compliant JSON after {} candidate(s).",
        attempted_candidates
    )
}

pub(super) fn log_user_memory_capture_payload_error(
    stage: &str,
    conversation_id: Option<&str>,
    raw: &str,
    error: UserMemoryCapturePayloadError,
) {
    let preview = user_memory_capture_response_preview(raw, 160);
    match error {
        UserMemoryCapturePayloadError::Unparseable => {
            tracing::warn!(
                "{} returned unparseable content for conversation {:?}. Preview: {}",
                stage,
                conversation_id,
                preview
            );
        }
        UserMemoryCapturePayloadError::IncompleteShape => {
            tracing::warn!(
                "{} returned incomplete shape for conversation {:?}; missing `memories` or `retractions` array. Preview: {}",
                stage,
                conversation_id,
                preview
            );
        }
    }
}

pub(super) fn build_user_memory_capture_repair_prompt(
    original_prompt: &str,
    invalid_response_preview: &str,
    response_shape: &str,
) -> String {
    format!(
        "The previous memory extraction response did not follow the required JSON schema.\n\nRequired JSON shape:\n{response_shape}\n\nPrevious invalid response preview:\n{invalid_response_preview}\n\nRe-run the source extraction task below. Decide semantically from the source user message, using dialogue and saved facts only as context. Do not extract assistant-authored status, tool output, integration setup state, or completed-work records as memory. Do not use phrase lists, keyword rules, or exact wording checks. Return JSON only, and always include both `memories` and `retractions` arrays even when one or both are empty.\n\nSource extraction task:\n{original_prompt}",
        response_shape = response_shape,
        invalid_response_preview = invalid_response_preview,
        original_prompt = original_prompt
    )
}

pub(super) fn build_user_memory_capture_empty_retry_prompt(
    original_prompt: &str,
    previous_response_preview: &str,
    response_shape: &str,
) -> String {
    format!(
        "The previous memory extraction produced no durable memory operations or returned an underspecified payload.\n\nRequired JSON shape:\n{response_shape}\n\nPrevious response preview:\n{previous_response_preview}\n\nRe-evaluate the source semantically. Determine whether the user supplied any durable self-information, preferences, workflow constraints, user-authored current-state facts worth carrying forward, or retractions. Mixed-intent turns, greetings, questions, and extra context do not cancel durable memory content stated in the same turn. Do not recover assistant-authored status, tool output, integration setup state, or completed-work records as memory. Return JSON only, and always include both `memories` and `retractions` arrays even when one or both are empty.\n\nSource extraction task:\n{original_prompt}",
        response_shape = response_shape,
        previous_response_preview = previous_response_preview,
        original_prompt = original_prompt
    )
}

pub(super) fn build_user_memory_capture_empty_verdict_prompt(
    original_prompt: &str,
    previous_response_preview: &str,
) -> String {
    format!(
        "The previous user-memory extraction ended in an empty decision.\n\nPrevious empty extraction preview:\n{previous_response_preview}\n\nReturn JSON only with this shape:\n{{\"has_durable_memory\":false,\"confidence\":0.0,\"reason\":\"brief rationale\"}}\n\nRules:\n- Set has_durable_memory=true only when the source user message contains durable user information worth retaining beyond this turn.\n- Durable user information includes identity, stable preferences, operating rules, workflow constraints, meaningful user-authored current-state facts, relationships, goals, or explicit retractions.\n- Assistant-authored status, tool output, integration setup state, and completed-work records are not durable user memory.\n- Decide from meaning and context. Do not use fixed phrases, literal wording checks, regular expressions, or keyword lists.\n- Mixed-intent turns still count: a self-statement remains durable even if the same message also asks a question, greets the assistant, or continues with another request.\n- Set has_durable_memory=false only when you are confident the source truly contains nothing worth storing.\n- confidence must be between 0.0 and 1.0.\n- reason should briefly describe the durable meaning you found, or why the turn truly contains nothing durable.\n\nSource extraction task:\n{original_prompt}",
        previous_response_preview = previous_response_preview,
        original_prompt = original_prompt
    )
}

pub(super) fn build_user_memory_capture_focused_recovery_prompt(
    original_prompt: &str,
    previous_response_preview: &str,
    review_reason: &str,
    response_shape: &str,
) -> String {
    format!(
        "A semantic review concluded that the previous empty extraction likely missed durable user memory.\n\nReview reason:\n{review_reason}\n\nPrevious empty extraction preview:\n{previous_response_preview}\n\nRequired JSON shape:\n{response_shape}\n\nRecover the missed durable memory operations from meaning. Decide semantically from the source user message, using dialogue and saved facts only as context. Do not use fixed phrases, exact wording checks, regular expressions, or keyword rules. If the source contains durable self-information, preferences, workflow constraints, meaningful user-authored current-state facts, or retractions, emit them. Do not recover assistant-authored status, tool output, integration setup state, or completed-work records as memory. Return JSON only, and keep both `memories` and `retractions` arrays present. Only return empty arrays if you are genuinely confident there is still nothing durable to store.\n\nSource extraction task:\n{original_prompt}",
        review_reason = review_reason,
        previous_response_preview = previous_response_preview,
        response_shape = response_shape,
        original_prompt = original_prompt
    )
}

pub(super) fn parse_user_memory_capture_empty_verdict(
    raw: &str,
) -> Option<UserMemoryCaptureEmptyVerdict> {
    let payload = extract_json_object_from_text(raw)?;
    let has_durable_memory = payload
        .get("has_durable_memory")
        .and_then(|value| value.as_bool())
        .or_else(|| {
            payload
                .get("missed_durable_memory")
                .and_then(|value| value.as_bool())
        })?;
    let confidence = payload
        .get("confidence")
        .or_else(|| payload.get("score"))
        .and_then(|value| value.as_f64())
        .unwrap_or_else(|| {
            if has_durable_memory {
                USER_FACT_MEMORY_CAPTURE_EMPTY_VERDICT_MIN_CONFIDENCE as f64
            } else {
                1.0
            }
        })
        .clamp(0.0, 1.0) as f32;
    let reason = payload
        .get("reason")
        .or_else(|| payload.get("reasoning"))
        .or_else(|| payload.get("rationale"))
        .and_then(|value| value.as_str())
        .map(str::trim)
        .filter(|value| !value.is_empty())
        .map(|value| safe_truncate(value, 180))
        .unwrap_or_default();
    Some(UserMemoryCaptureEmptyVerdict {
        has_durable_memory,
        confidence,
        reason,
    })
}

pub(super) fn learned_user_memory_scope_ids<'a>(
    raw_scope: Option<&str>,
    project_id: Option<&'a str>,
    conversation_id: Option<&'a str>,
) -> (&'static str, Option<&'a str>, Option<&'a str>) {
    match normalize_user_memory_scope(raw_scope) {
        "conversation" if conversation_id.is_some() => {
            ("conversation", project_id, conversation_id)
        }
        "project" if project_id.is_some() => ("project", project_id, None),
        _ => ("global", None, None),
    }
}

pub(super) fn learned_user_memory_keys(
    key: &str,
    durability: &str,
    project_id: Option<&str>,
    conversation_id: Option<&str>,
) -> (String, String) {
    let hash = ambient_stable_hash(&[
        key.trim(),
        durability.trim(),
        project_id.unwrap_or_default(),
        conversation_id.unwrap_or_default(),
    ]);
    (
        format!("user-memory-{}", hash),
        format!("user_memory::{}::{}", key.trim(), durability.trim()),
    )
}

pub(super) fn user_memory_capture_error_entry(
    code: &str,
    detail: impl Into<String>,
) -> serde_json::Value {
    serde_json::json!({
        "code": code,
        "detail": detail.into(),
        "at": chrono::Utc::now().to_rfc3339(),
    })
}

#[derive(Debug, Clone, Default)]
struct UserMemoryCaptureSourceRetryState {
    previous_capture_count: u64,
    blocks_retry: bool,
    recovered_stale_count: usize,
}

fn user_memory_capture_event_timestamp(raw: &str) -> Option<chrono::DateTime<chrono::Utc>> {
    chrono::DateTime::parse_from_rfc3339(raw.trim())
        .ok()
        .map(|value| value.with_timezone(&chrono::Utc))
}

fn user_memory_capture_status_is_in_progress(status: &str) -> bool {
    matches!(
        status.trim(),
        "processing" | USER_MEMORY_CAPTURE_PROCESSING_DEFERRED_STATUS
    )
}

fn user_memory_capture_stale_processing_lease_secs() -> i64 {
    std::env::var("AGENTARK_USER_MEMORY_CAPTURE_STALE_LEASE_SECS")
        .ok()
        .and_then(|value| value.trim().parse::<i64>().ok())
        .filter(|value| *value > 0)
        .map(|value| value.max(USER_MEMORY_CAPTURE_STALE_PROCESSING_LEASE_MIN_SECS))
        .unwrap_or(USER_MEMORY_CAPTURE_STALE_PROCESSING_LEASE_DEFAULT_SECS)
}

fn user_memory_capture_event_is_stale_processing(
    event: &crate::storage::memory_capture_event::Model,
    now: chrono::DateTime<chrono::Utc>,
) -> bool {
    if !user_memory_capture_status_is_in_progress(event.status.as_str()) {
        return false;
    }
    let Some(updated_at) = user_memory_capture_event_timestamp(&event.updated_at)
        .or_else(|| user_memory_capture_event_timestamp(&event.created_at))
    else {
        return true;
    };
    now.signed_duration_since(updated_at)
        >= chrono::Duration::seconds(user_memory_capture_stale_processing_lease_secs())
}

fn user_memory_capture_event_blocks_source_retry(
    event: &crate::storage::memory_capture_event::Model,
    now: chrono::DateTime<chrono::Utc>,
) -> bool {
    let status = event.status.trim();
    if status == USER_MEMORY_CAPTURE_RETIRED_STALE_PROCESSING_STATUS
        || status == USER_MEMORY_CAPTURE_FAILED_STALE_PROCESSING_STATUS
    {
        return false;
    }
    if user_memory_capture_status_is_in_progress(status)
        && user_memory_capture_event_is_stale_processing(event, now)
    {
        return false;
    }
    true
}

fn user_memory_capture_push_error_history(
    history: &serde_json::Value,
    entry: serde_json::Value,
) -> serde_json::Value {
    let mut items = history.as_array().cloned().unwrap_or_default();
    items.push(entry);
    serde_json::Value::Array(items)
}

fn user_memory_capture_stale_recovery_metadata(
    metadata: &serde_json::Value,
    previous_status: &str,
    now: chrono::DateTime<chrono::Utc>,
) -> serde_json::Value {
    let mut object = metadata.as_object().cloned().unwrap_or_default();
    object.insert(
        "previous_status".to_string(),
        serde_json::Value::String(previous_status.to_string()),
    );
    object.insert(
        "stale_recovery".to_string(),
        serde_json::json!({
            "previous_status": previous_status,
            "recovered_at": now.to_rfc3339(),
            "lease_secs": user_memory_capture_stale_processing_lease_secs(),
        }),
    );
    serde_json::Value::Object(object)
}

async fn reclaim_stale_user_memory_capture_event(
    storage: &crate::storage::Storage,
    event: &crate::storage::memory_capture_event::Model,
    now: chrono::DateTime<chrono::Utc>,
) -> bool {
    if !user_memory_capture_event_is_stale_processing(event, now) {
        return false;
    }
    let previous_status = event.status.trim().to_string();
    let mut recovered = event.clone();
    recovered.updated_at = now.to_rfc3339();
    recovered.attempt_metadata = user_memory_capture_stale_recovery_metadata(
        &recovered.attempt_metadata,
        &previous_status,
        now,
    );
    match previous_status.as_str() {
        USER_MEMORY_CAPTURE_PROCESSING_DEFERRED_STATUS => {
            recovered.status = USER_MEMORY_CAPTURE_PENDING_STATUS.to_string();
            recovered.completed_at = None;
            recovered.next_retry_at = None;
            recovered.replay_count = recovered.replay_count.saturating_add(1);
            recovered.error_history = user_memory_capture_push_error_history(
                &recovered.error_history,
                user_memory_capture_error_entry(
                    "stale_processing_deferred_reclaimed",
                    "Deferred memory capture was left in-progress and was reclaimed for retry.",
                ),
            );
        }
        "processing" => {
            recovered.status = USER_MEMORY_CAPTURE_RETIRED_STALE_PROCESSING_STATUS.to_string();
            recovered.completed_at = Some(now.to_rfc3339());
            recovered.error_history = user_memory_capture_push_error_history(
                &recovered.error_history,
                user_memory_capture_error_entry(
                    "stale_processing_reclaimed",
                    "Memory capture was left in-progress and was retired so the source can be retried.",
                ),
            );
        }
        _ => return false,
    }
    match storage
        .try_update_memory_capture_event_from_status(&recovered, &previous_status)
        .await
    {
        Ok(true) => {
            tracing::debug!(
                capture_event_id = %recovered.id,
                previous_status = %previous_status,
                new_status = %recovered.status,
                "Reclaimed stale user memory capture event"
            );
            true
        }
        Ok(false) => {
            tracing::debug!(
                capture_event_id = %event.id,
                previous_status = %previous_status,
                "Skipped stale user memory capture recovery because another worker changed the row first"
            );
            false
        }
        Err(error) => {
            tracing::warn!(
                capture_event_id = %event.id,
                previous_status = %previous_status,
                "Failed to reclaim stale user memory capture event: {}",
                error
            );
            false
        }
    }
}

async fn user_memory_capture_source_retry_state(
    storage: &crate::storage::Storage,
    source_hash: &str,
    now: chrono::DateTime<chrono::Utc>,
) -> Result<UserMemoryCaptureSourceRetryState> {
    let prior_events = storage
        .list_memory_capture_events_by_source_hash(
            source_hash,
            USER_MEMORY_CAPTURE_SOURCE_HISTORY_LIMIT,
        )
        .await?;
    let mut state = UserMemoryCaptureSourceRetryState {
        previous_capture_count: prior_events.len() as u64,
        ..Default::default()
    };
    for event in prior_events {
        if user_memory_capture_event_is_stale_processing(&event, now) {
            if reclaim_stale_user_memory_capture_event(storage, &event, now).await {
                state.recovered_stale_count += 1;
            }
            continue;
        }
        if user_memory_capture_event_blocks_source_retry(&event, now) {
            state.blocks_retry = true;
        }
    }
    Ok(state)
}

async fn recover_stale_user_memory_capture_events(
    storage: &crate::storage::Storage,
    now: chrono::DateTime<chrono::Utc>,
) -> usize {
    let events = match storage
        .list_memory_capture_events_by_statuses_all_scopes(
            &[USER_MEMORY_CAPTURE_PROCESSING_DEFERRED_STATUS, "processing"],
            USER_MEMORY_CAPTURE_STALE_RECOVERY_BATCH_LIMIT,
        )
        .await
    {
        Ok(events) => events,
        Err(error) => {
            tracing::debug!(
                "Failed to load stale user memory capture events for recovery: {}",
                error
            );
            return 0;
        }
    };
    let mut recovered = 0usize;
    for event in events {
        if reclaim_stale_user_memory_capture_event(storage, &event, now).await {
            recovered += 1;
        }
    }
    recovered
}

pub(super) fn user_memory_operation_evidence_refs(
    source_message_id: Option<&str>,
    capture_event_id: Option<&str>,
    channel: &str,
) -> serde_json::Value {
    let mut refs = Vec::new();
    if let Some(source_message_id) = source_message_id
        .map(str::trim)
        .filter(|value| !value.is_empty())
    {
        refs.push(serde_json::Value::String(format!(
            "message:{}",
            source_message_id
        )));
    }
    if let Some(capture_event_id) = capture_event_id
        .map(str::trim)
        .filter(|value| !value.is_empty())
    {
        refs.push(serde_json::Value::String(format!(
            "capture_event:{}",
            capture_event_id
        )));
    }
    let trimmed_channel = channel.trim();
    if !trimmed_channel.is_empty() {
        refs.push(serde_json::Value::String(format!(
            "channel:{}",
            trimmed_channel
        )));
    }
    serde_json::Value::Array(refs)
}

pub(super) fn user_memory_operation_evidence_ref_value(
    evidence_refs: &serde_json::Value,
    prefix: &str,
) -> Option<String> {
    evidence_refs.as_array().and_then(|items| {
        items.iter().find_map(|item| {
            item.as_str()
                .and_then(|value| value.strip_prefix(prefix))
                .map(str::trim)
                .filter(|value| !value.is_empty())
                .map(str::to_string)
        })
    })
}

pub(super) fn user_memory_operation_candidate_type(operation_type: &str) -> &'static str {
    match operation_type {
        "retract" => "memory_retract",
        "update" => "memory_update",
        _ => "memory_add",
    }
}

pub(super) fn memory_operation_learning_candidate_pattern_id(
    _operation: &crate::storage::memory_operation::Model,
) -> Option<String> {
    // learning_candidates.pattern_id is reserved for procedural_patterns.id.
    // Memory operation provenance is stored in evidence_refs/proposed_content.
    None
}

pub(super) fn user_memory_operation_scope_explicit(
    operation: &crate::storage::memory_operation::Model,
) -> bool {
    operation
        .model_metadata
        .get("scope_explicit")
        .and_then(|value| value.as_bool())
        .unwrap_or(operation.operation_type != "retract")
}

pub(super) fn memory_operation_explicit_upsert_target_id(
    operation: &crate::storage::memory_operation::Model,
) -> Option<&str> {
    if operation.operation_type == "update" {
        operation.target_memory_id.as_deref()
    } else {
        None
    }
}

pub(super) fn user_memory_operation_semantic_key(
    operation_type: &str,
    key: Option<&str>,
    memory_kind: &str,
    durability: &str,
    scope: &str,
    scope_explicit: bool,
    project_id: Option<&str>,
    conversation_id: Option<&str>,
    target_memory_id: Option<&str>,
) -> String {
    if operation_type != "retract" {
        if let Some(target_memory_id) = target_memory_id
            .map(str::trim)
            .filter(|value| !value.is_empty())
        {
            return target_memory_id.to_string();
        }
    }
    format!(
        "memory-operation-subject-{}",
        ambient_stable_hash(&[
            operation_type.trim(),
            key.unwrap_or_default().trim(),
            memory_kind.trim(),
            durability.trim(),
            scope.trim(),
            if scope_explicit {
                "explicit"
            } else {
                "implicit"
            },
            project_id.unwrap_or_default().trim(),
            conversation_id.unwrap_or_default().trim(),
        ])
    )
}

pub(super) fn user_memory_operation_subject_key(
    operation: &crate::storage::memory_operation::Model,
) -> String {
    operation
        .model_metadata
        .get("semantic_key")
        .and_then(|value| value.as_str())
        .map(str::trim)
        .filter(|value| !value.is_empty())
        .map(str::to_string)
        .unwrap_or_else(|| {
            user_memory_operation_semantic_key(
                &operation.operation_type,
                operation.key.as_deref(),
                &operation.memory_kind,
                &operation.durability,
                &operation.scope,
                user_memory_operation_scope_explicit(operation),
                operation.project_id.as_deref(),
                operation.conversation_id.as_deref(),
                operation.target_memory_id.as_deref(),
            )
        })
}

pub(super) fn learned_user_memory_expired(
    item: &crate::storage::experience_item::Model,
    now: chrono::DateTime<chrono::Utc>,
) -> bool {
    ambient_metadata_datetime_field(item, "expires_at")
        .map(|dt| dt <= now)
        .unwrap_or(false)
}

pub(super) fn learned_user_memory_active(
    item: &crate::storage::experience_item::Model,
    now: chrono::DateTime<chrono::Utc>,
) -> bool {
    item.status == "active" && !learned_user_memory_expired(item, now)
}

pub(super) fn format_learned_user_memory_for_prompt(
    item: &crate::storage::experience_item::Model,
    now: chrono::DateTime<chrono::Utc>,
) -> Option<String> {
    if !learned_user_memory_active(item, now) {
        return None;
    }
    let label = if item.kind == "constraint" {
        "rule"
    } else {
        "fact"
    };
    let durability = item
        .metadata
        .get("durability")
        .and_then(|value| value.as_str())
        .and_then(|value| normalize_user_memory_text(value, 32))
        .unwrap_or_else(|| "permanent".to_string());
    let mut qualifiers = vec![label.to_string(), durability];
    qualifiers.push(
        crate::core::memory_schema::memory_category_label(learned_user_memory_category(item))
            .to_string(),
    );
    let topics = learned_user_memory_topics(item);
    if !topics.is_empty() {
        qualifiers.push(format!("topics: {}", topics.join(", ")));
    }
    if let Some(expires_at) = ambient_metadata_datetime_field(item, "expires_at") {
        qualifiers.push(format!("valid until {}", expires_at.to_rfc3339()));
    }
    if let Some(review_at) = ambient_metadata_datetime_field(item, "review_at") {
        qualifiers.push(format!("review after {}", review_at.to_rfc3339()));
    }
    Some(format!(
        "- [{}] {}",
        qualifiers.join("; "),
        safe_truncate(&item.content, 180)
    ))
}

pub(super) fn normalize_ambient_text(raw: &str, max_chars: usize) -> Option<String> {
    let value = raw
        .trim()
        .trim_matches(|c: char| matches!(c, '"' | '\'' | '`'));
    if value.is_empty() || value.eq_ignore_ascii_case("null") {
        return None;
    }
    Some(safe_truncate(value, max_chars))
}

pub(super) fn ambient_json_text_field(
    item: &serde_json::Value,
    field: &str,
    max_chars: usize,
) -> Option<String> {
    let value = item.get(field)?;
    match value {
        serde_json::Value::String(raw) => normalize_ambient_text(raw, max_chars),
        serde_json::Value::Null => None,
        other => normalize_ambient_text(&other.to_string(), max_chars),
    }
}

pub(super) fn parse_ambient_rfc3339(raw: &str) -> Option<chrono::DateTime<chrono::Utc>> {
    let value = raw.trim();
    if value.is_empty() || value.eq_ignore_ascii_case("null") {
        return None;
    }
    chrono::DateTime::parse_from_rfc3339(value)
        .ok()
        .map(|dt| dt.with_timezone(&chrono::Utc))
}

pub(super) fn ambient_json_datetime_field(
    item: &serde_json::Value,
    field: &str,
) -> Option<chrono::DateTime<chrono::Utc>> {
    item.get(field)
        .and_then(|value| value.as_str())
        .and_then(parse_ambient_rfc3339)
}

pub(super) fn ambient_metadata_text_field(
    item: &crate::storage::experience_item::Model,
    field: &str,
    max_chars: usize,
) -> Option<String> {
    item.metadata
        .get(field)
        .and_then(|value| value.as_str())
        .and_then(|raw| normalize_ambient_text(raw, max_chars))
}

pub(super) fn ambient_metadata_datetime_field(
    item: &crate::storage::experience_item::Model,
    field: &str,
) -> Option<chrono::DateTime<chrono::Utc>> {
    item.metadata
        .get(field)
        .and_then(|value| value.as_str())
        .and_then(parse_ambient_rfc3339)
}

pub(super) fn ambient_intent_due(
    item: &crate::storage::experience_item::Model,
    now: chrono::DateTime<chrono::Utc>,
) -> bool {
    if ambient_intent_expired(item, now) {
        return false;
    }
    ambient_metadata_datetime_field(item, "next_revisit_at")
        .map(|dt| dt <= now)
        .unwrap_or(true)
}

pub(super) fn ambient_intent_expired(
    item: &crate::storage::experience_item::Model,
    now: chrono::DateTime<chrono::Utc>,
) -> bool {
    ambient_metadata_datetime_field(item, "expires_at")
        .map(|dt| dt <= now)
        .unwrap_or(false)
}

pub(super) fn ambient_stable_hash(parts: &[&str]) -> String {
    let mut hasher = Sha256::new();
    for part in parts {
        hasher.update([0u8]);
        hasher.update(part.as_bytes());
    }
    let digest = hex::encode(hasher.finalize());
    digest.chars().take(24).collect::<String>()
}

pub(super) fn ambient_intent_metadata_object(
    item: &crate::storage::experience_item::Model,
) -> serde_json::Map<String, serde_json::Value> {
    item.metadata.as_object().cloned().unwrap_or_default()
}

#[derive(Clone)]
pub(super) struct UserMemoryCaptureWorker {
    storage: Storage,
    encrypted_storage: crate::storage::encrypted::EncryptedStorage,
    conversation_history: Arc<RwLock<HashMap<String, Vec<ConversationMessage>>>>,
    user_profile: Arc<RwLock<UserProfile>>,
    llm: LlmClient,
    embedding_client: Option<Arc<EmbeddingClient>>,
    model_pool: HashMap<String, (ModelSlot, LlmClient)>,
    execution_supervisor: super::ExecutionSupervisor,
    config: AgentConfig,
    primary_model_id: String,
    user_selected_model_slot_id: Arc<std::sync::RwLock<Option<String>>>,
}

impl UserMemoryCaptureWorker {
    pub(super) fn from_agent(agent: &Agent) -> Self {
        Self {
            storage: agent.storage.clone(),
            encrypted_storage: agent.encrypted_storage.clone(),
            conversation_history: agent.conversation_history.clone(),
            user_profile: agent.user_profile.clone(),
            llm: agent.llm.clone(),
            embedding_client: agent.embedding_client.clone(),
            model_pool: agent.model_pool.clone(),
            execution_supervisor: agent.execution_supervisor.clone(),
            config: agent.config.clone(),
            primary_model_id: agent.primary_model_id.clone(),
            user_selected_model_slot_id: agent.user_selected_model_slot_id.clone(),
        }
    }

    pub(super) async fn sync_runtime_profile_from_user_preference(
        &self,
        preference_key: &str,
        value: &str,
    ) {
        let trimmed = value.trim();
        if trimmed.is_empty() {
            return;
        }

        let maybe_profile_bytes = {
            let mut profile = self.user_profile.write().await;
            let mut changed = false;
            match preference_key {
                "user_timezone" => {
                    if trimmed.parse::<chrono_tz::Tz>().is_err() {
                        tracing::warn!(
                            preference_key = preference_key,
                            value = trimmed,
                            "Skipping runtime profile sync for invalid timezone value"
                        );
                    } else if profile.timezone.as_deref() != Some(trimmed) {
                        profile.timezone = Some(trimmed.to_string());
                        changed = true;
                    }
                }
                "preferred_tone" => {
                    if profile.tone.as_deref() != Some(trimmed) {
                        profile.tone = Some(trimmed.to_string());
                        changed = true;
                    }
                }
                _ => {}
            }
            if changed {
                match serde_json::to_vec(&*profile) {
                    Ok(bytes) => Some(bytes),
                    Err(error) => {
                        tracing::warn!(
                            error = ?error,
                            preference_key = preference_key,
                            "Failed to serialize user profile after preference sync"
                        );
                        None
                    }
                }
            } else {
                None
            }
        };

        if let Some(profile_bytes) = maybe_profile_bytes {
            if let Err(error) = self
                .encrypted_storage
                .set_encrypted("user_profile", &profile_bytes)
                .await
            {
                tracing::warn!(
                    error = ?error,
                    preference_key = preference_key,
                    "Failed to persist runtime profile after preference sync"
                );
            }
        }
    }

    pub(super) async fn sync_applied_memory_to_user_preferences(
        &self,
        operation: &crate::storage::memory_operation::Model,
        key: &str,
        value: &str,
    ) {
        if operation.scope != "global"
            || operation.project_id.is_some()
            || operation.conversation_id.is_some()
            || operation.looks_sensitive
        {
            return;
        }

        let Some(preference_key) = learned_memory_key_to_user_preference_key(key) else {
            return;
        };

        let confidence = operation.confidence.clamp(0.0, 1.0) as f32;
        if let Err(error) = self
            .storage
            .upsert_user_preference(
                &preference_key,
                value,
                confidence,
                Some(USER_LEARNED_MEMORY_CAPTURE_SOURCE),
                None,
                operation
                    .model_metadata
                    .get("sensitivity")
                    .and_then(|value| value.as_str()),
            )
            .await
        {
            tracing::warn!(
                error = ?error,
                preference_key = preference_key,
                memory_operation_id = %operation.id,
                "Failed to sync applied learned memory into user preferences"
            );
            return;
        }

        self.sync_runtime_profile_from_user_preference(&preference_key, value)
            .await;
    }

    pub(super) async fn capture_user_memory_hints(
        &self,
        message: &str,
        user_message: &str,
        channel: &str,
        conversation_id: Option<&str>,
        project_id: Option<&str>,
        source_message_id: Option<&str>,
    ) {
        self.capture_user_links_as_user_data(user_message, channel, conversation_id, project_id)
            .await;
        self.capture_user_facts_with_llm(
            message,
            channel,
            conversation_id,
            project_id,
            source_message_id,
        )
        .await;
    }

    pub(super) async fn capture_user_links_as_user_data(
        &self,
        message: &str,
        channel: &str,
        conversation_id: Option<&str>,
        project_id: Option<&str>,
    ) {
        let urls = extract_user_supplied_link_user_data_urls(message);
        if urls.is_empty() {
            return;
        }
        for url in urls {
            if let Err(error) = self
                .storage
                .upsert_user_data_link(&url, Some(channel), conversation_id, project_id)
                .await
            {
                tracing::warn!(
                    "Failed to capture user link '{}' into user_data_items: {}",
                    url,
                    error
                );
            }
        }
    }

    async fn existing_equivalent_user_memory_id(
        &self,
        key: &str,
        value: &str,
        semantic_kind: &str,
        project_id: Option<&str>,
        conversation_id: Option<&str>,
        target_memory_id: Option<&str>,
    ) -> Option<String> {
        let active_items = self
            .storage
            .list_active_experience_items(
                SAVED_USER_FACT_PROMPT_KINDS,
                project_id,
                conversation_id,
                48,
            )
            .await
            .ok()?;
        let now = chrono::Utc::now();
        let candidate_value = normalize_user_memory_equivalence_text(value)?;
        let mut candidates = Vec::new();
        for item in active_items {
            if !learned_user_memory_active(&item, now) {
                continue;
            }
            let item_value = learned_user_memory_value(&item)?;
            let item_value_norm = normalize_user_memory_equivalence_text(&item_value)?;
            if target_memory_id == Some(item.id.as_str()) {
                if item_value_norm == candidate_value {
                    return Some(item.id);
                }
                continue;
            }
            let same_key = learned_user_memory_key_matches(&item, key);
            let same_kind = learned_user_memory_lookup_kind(&item) == semantic_kind
                || semantic_kind == "other"
                || learned_user_memory_lookup_kind(&item) == "other";
            if item_value_norm == candidate_value && (same_key || same_kind) {
                return Some(item.id);
            }
            candidates.push(item);
        }

        if candidates.is_empty() {
            return None;
        }
        self.existing_equivalent_user_memory_id_with_llm(value, semantic_kind, &candidates)
            .await
    }

    async fn existing_equivalent_user_memory_id_with_llm(
        &self,
        value: &str,
        semantic_kind: &str,
        candidates: &[crate::storage::experience_item::Model],
    ) -> Option<String> {
        let candidate = self
            .llm_candidates_for_role(&ModelRole::Fast)
            .into_iter()
            .next()?;
        let existing = candidates
            .iter()
            .take(24)
            .filter_map(|item| {
                Some(serde_json::json!({
                    "id": item.id.clone(),
                    "kind": learned_user_memory_lookup_kind(item),
                    "category": learned_user_memory_category(item),
                    "content": learned_user_memory_value(item)?,
                    "scope": item.scope.clone(),
                }))
            })
            .collect::<Vec<_>>();
        if existing.is_empty() {
            return None;
        }
        let payload = serde_json::json!({
            "candidate": {
                "kind": semantic_kind,
                "category": crate::core::memory_schema::normalize_memory_category(None, Some(semantic_kind)),
                "content": safe_truncate(value, 320),
            },
            "existing_memories": existing,
            "output_shape": {
                "duplicate_id": "existing id or null",
                "confidence": 0.0,
                "reason": "brief rationale"
            }
        });
        let system_prompt = concat!(
            "You are a strict semantic duplicate checker for stored user memories. ",
            "Decide whether the candidate memory would add materially new durable information. ",
            "Ignore wording, key names, punctuation, and formatting. Compare subject, polarity, specificity, scope, and meaning. ",
            "Return JSON only. If no existing memory is equivalent or already covers the candidate, use duplicate_id=null."
        );
        let response = tokio::time::timeout(
            std::time::Duration::from_millis(8_000),
            candidate.client.chat_with_system_bounded(
                system_prompt,
                &serde_json::to_string(&payload).ok()?,
                256,
            ),
        )
        .await
        .ok()?
        .ok()?;
        self.record_llm_usage("memory", "user_fact_memory_duplicate_check", &response)
            .await;
        let parsed = extract_json_object_from_text(&response.content)?;
        let confidence = parsed
            .get("confidence")
            .and_then(|value| value.as_f64())
            .unwrap_or(0.0);
        if confidence < 0.78 {
            return None;
        }
        let duplicate_id = parsed
            .get("duplicate_id")
            .and_then(|value| value.as_str())
            .map(str::trim)
            .filter(|value| !value.is_empty() && !value.eq_ignore_ascii_case("null"))?;
        candidates
            .iter()
            .any(|item| item.id == duplicate_id)
            .then(|| duplicate_id.to_string())
    }

    async fn semantically_matching_user_memory_target_id(
        &self,
        key: &str,
        value: &str,
        semantic_kind: &str,
        durability: &str,
        scope: &str,
        project_id: Option<&str>,
        conversation_id: Option<&str>,
        target_memory_id: Option<&str>,
    ) -> Option<String> {
        let active_items = self
            .storage
            .list_active_experience_items(
                SAVED_USER_FACT_PROMPT_KINDS,
                project_id,
                conversation_id,
                64,
            )
            .await
            .ok()?;
        let now = chrono::Utc::now();
        let mut candidates = Vec::new();
        for item in active_items {
            if !learned_user_memory_active(&item, now) {
                continue;
            }
            if target_memory_id == Some(item.id.as_str()) {
                continue;
            }
            if !learned_user_memory_matches_exact_scope(&item, scope, project_id, conversation_id) {
                continue;
            }
            let item_durability = learned_user_memory_durability(&item);
            if normalize_user_memory_durability(Some(item_durability.as_str())) != durability {
                continue;
            }
            if learned_user_memory_key_matches(&item, key) {
                return Some(item.id);
            }
            candidates.push(item);
        }
        if candidates.is_empty() {
            return None;
        }
        candidates.sort_by(|left, right| {
            let left_kind = learned_user_memory_lookup_kind(left);
            let right_kind = learned_user_memory_lookup_kind(right);
            let left_same_kind =
                !matches!(semantic_kind, "" | "any" | "other") && left_kind == semantic_kind;
            let right_same_kind =
                !matches!(semantic_kind, "" | "any" | "other") && right_kind == semantic_kind;
            right_same_kind
                .cmp(&left_same_kind)
                .then_with(|| right.updated_at.cmp(&left.updated_at))
                .then_with(|| right.confidence.total_cmp(&left.confidence))
                .then_with(|| right.support_count.cmp(&left.support_count))
        });

        let candidate = self
            .llm_candidates_for_role(&ModelRole::Fast)
            .into_iter()
            .next()?;
        let candidate_content = format!("{}: {}", key.trim(), value.trim());
        for item in candidates.into_iter().take(8) {
            let existing_key = learned_user_memory_key(&item);
            let existing_value =
                learned_user_memory_value(&item).unwrap_or_else(|| item.content.clone());
            let payload = serde_json::json!({
                "candidate": {
                    "key": key,
                    "kind": semantic_kind,
                    "category": crate::core::memory_schema::normalize_memory_category(None, Some(semantic_kind)),
                    "durability": durability,
                    "scope": scope,
                    "content": safe_truncate(&candidate_content, 320),
                },
                "existing_memory": {
                    "id": item.id.clone(),
                    "key": existing_key,
                    "kind": learned_user_memory_lookup_kind(&item),
                    "category": learned_user_memory_category(&item),
                    "durability": learned_user_memory_durability(&item),
                    "scope": item.scope.clone(),
                    "content": safe_truncate(&existing_value, 320),
                },
                "output_shape": {
                    "same_subject": false,
                    "confidence": 0.0,
                    "reason": "brief rationale"
                }
            });
            let response = match tokio::time::timeout(
                std::time::Duration::from_millis(8_000),
                candidate.client.chat_with_system_bounded(
                    concat!(
                        "You decide whether an existing stored user memory is the same durable ",
                        "user-memory subject that should be updated by a new candidate. Values may ",
                        "contradict; contradictory values can still be the same subject if the new ",
                        "candidate corrects or supersedes the old one. Do not match wording, key ",
                        "names, punctuation, casing, or value equality. Compare the underlying ",
                        "subject, scope, specificity, and whether keeping both active would conflict ",
                        "or duplicate the profile. Return JSON only."
                    ),
                    &serde_json::to_string(&payload).unwrap_or_default(),
                    256,
                ),
            )
            .await
            {
                Ok(Ok(response)) => response,
                Ok(Err(error)) => {
                    tracing::debug!(
                        "Semantic memory update target check failed for '{}': {}",
                        key,
                        error
                    );
                    continue;
                }
                Err(_) => {
                    tracing::debug!(
                        "Semantic memory update target check timed out for '{}'",
                        key
                    );
                    continue;
                }
            };
            self.record_llm_usage("memory", "user_fact_memory_update_target_check", &response)
                .await;
            let Some(parsed) = extract_json_object_from_text(&response.content) else {
                continue;
            };
            let same_subject = parsed
                .get("same_subject")
                .and_then(|value| value.as_bool())
                .unwrap_or(false);
            let confidence = parsed
                .get("confidence")
                .and_then(|value| value.as_f64())
                .unwrap_or(0.0);
            if same_subject && confidence >= 0.72 {
                return Some(item.id);
            }
        }
        None
    }

    async fn review_user_memory_candidate_for_storage(
        &self,
        source_message: &str,
        recent_dialogue: &str,
        candidate_payload: &serde_json::Value,
        channel: &str,
        conversation_id: Option<&str>,
    ) -> Option<UserMemoryCandidateSemanticReview> {
        let candidate = self
            .user_memory_capture_llm_candidates()
            .into_iter()
            .next()?;
        let prompt = build_user_memory_candidate_semantic_review_prompt(
            source_message,
            recent_dialogue,
            candidate_payload,
        );
        let memories: [PromptMemory; 0] = [];
        let actions: [crate::actions::ActionDef; 0] = [];
        let response = match candidate
            .client
            .chat_for_helper_request_limited(
                "You are a semantic ArkMemory admission reviewer. Return strict JSON only.",
                &prompt,
                &memories,
                &actions,
                &crate::security::ModelPrivacyConfig::default(),
                USER_FACT_MEMORY_CAPTURE_ALLOW_SENSITIVE_CONTEXT,
                Some(600),
            )
            .await
        {
            Ok(response) => response,
            Err(error) => {
                tracing::debug!(
                    "User memory semantic candidate review failed for conversation {:?}: {}",
                    conversation_id,
                    error
                );
                return None;
            }
        };
        self.record_llm_usage(
            channel,
            "user_fact_memory_candidate_semantic_review",
            &response,
        )
        .await;
        let review = parse_user_memory_candidate_semantic_review(&response.content);
        if review.is_none() {
            tracing::debug!(
                "User memory semantic candidate review returned invalid JSON for conversation {:?}. Preview: {}",
                conversation_id,
                user_memory_capture_response_preview(&response.content, 160)
            );
        }
        review
    }

    pub(super) async fn capture_user_facts_with_llm(
        &self,
        message: &str,
        channel: &str,
        conversation_id: Option<&str>,
        project_id: Option<&str>,
        source_message_id: Option<&str>,
    ) {
        let trimmed = message.trim();
        if trimmed.is_empty() {
            return;
        }
        let source_hash = ambient_stable_hash(&[
            channel.trim(),
            conversation_id.unwrap_or_default().trim(),
            project_id.unwrap_or_default().trim(),
            trimmed,
        ]);
        let capture_now_dt = chrono::Utc::now();
        let capture_now = capture_now_dt.to_rfc3339();
        let retry_state = match user_memory_capture_source_retry_state(
            &self.storage,
            &source_hash,
            capture_now_dt,
        )
        .await
        {
            Ok(state) => state,
            Err(error) => {
                tracing::debug!(
                    "Failed to inspect prior memory capture events for source retry: {}",
                    error
                );
                UserMemoryCaptureSourceRetryState::default()
            }
        };
        let replay_count = retry_state
            .previous_capture_count
            .saturating_add(1)
            .min(i32::MAX as u64) as i32;
        let event_id = format!("memory-capture-{}", uuid::Uuid::new_v4());
        let mut capture_event = crate::storage::memory_capture_event::Model {
            id: event_id.clone(),
            source_message_id: source_message_id.map(str::to_string),
            conversation_id: conversation_id.map(str::to_string),
            project_id: project_id.map(str::to_string),
            channel: channel.to_string(),
            status: "processing".to_string(),
            capture_kind: "user_fact_memory_capture".to_string(),
            source_hash: source_hash.clone(),
            attempt_metadata: serde_json::json!({}),
            error_history: serde_json::json!([]),
            replay_count,
            next_retry_at: None,
            completed_at: None,
            created_at: capture_now.clone(),
            updated_at: capture_now.clone(),
        };
        capture_event.attempt_metadata = serde_json::json!({
            "schema_version": 1,
            "message_chars": trimmed.chars().count(),
            "semantic_capture_key": source_hash.clone(),
            "source": USER_LEARNED_MEMORY_CAPTURE_SOURCE,
        });
        capture_event.error_history = serde_json::json!([]);
        if let Err(error) = self
            .storage
            .upsert_memory_capture_event(&capture_event)
            .await
        {
            tracing::warn!(
                "Failed to initialize memory capture event '{}' for conversation {:?}: {}",
                event_id,
                conversation_id,
                error
            );
        }
        if retry_state.blocks_retry {
            capture_event.status = "skipped_duplicate_source".to_string();
            capture_event.completed_at = Some(chrono::Utc::now().to_rfc3339());
            capture_event.updated_at = chrono::Utc::now().to_rfc3339();
            capture_event.error_history = serde_json::json!([user_memory_capture_error_entry(
                "duplicate_source",
                "Skipped user memory capture because the same source text was already processed in this conversation scope.",
            )]);
            let mut metadata = capture_event
                .attempt_metadata
                .as_object()
                .cloned()
                .unwrap_or_default();
            metadata.insert(
                "recovered_stale_prior_capture_count".to_string(),
                serde_json::json!(retry_state.recovered_stale_count),
            );
            capture_event.attempt_metadata = serde_json::Value::Object(metadata);
            let _ = self
                .storage
                .upsert_memory_capture_event(&capture_event)
                .await;
            return;
        }
        if trimmed.chars().count() > 1_600 {
            capture_event.status = "skipped_oversize".to_string();
            capture_event.completed_at = Some(chrono::Utc::now().to_rfc3339());
            capture_event.updated_at = chrono::Utc::now().to_rfc3339();
            capture_event.error_history = serde_json::json!([user_memory_capture_error_entry(
                "message_too_large",
                "Skipped user memory capture because the message exceeded the 1600 character prompt limit.",
            )]);
            let _ = self
                .storage
                .upsert_memory_capture_event(&capture_event)
                .await;
            return;
        }
        let Some(safe_message) = sanitize_user_memory_prompt_text(trimmed, 1_600) else {
            tracing::warn!(
                "Skipping user fact memory extraction because the candidate message looked like credential material"
            );
            capture_event.status = "rejected_sensitive_input".to_string();
            capture_event.completed_at = Some(chrono::Utc::now().to_rfc3339());
            capture_event.updated_at = chrono::Utc::now().to_rfc3339();
            capture_event.error_history = serde_json::json!([user_memory_capture_error_entry(
                "sensitive_input",
                "Skipped user memory capture because the source message looked like credential or secret material.",
            )]);
            let _ = self
                .storage
                .upsert_memory_capture_event(&capture_event)
                .await;
            return;
        };
        let prompt_message = safe_message.text;

        let recent_dialogue = if let Some(id) = conversation_id.filter(|id| !id.trim().is_empty()) {
            let history = self
                .recent_messages_for_intent_gating(id, &prompt_message)
                .await;
            format_recent_dialogue_for_memory_context(&history)
                .unwrap_or_else(|| "(none)".to_string())
        } else {
            "(none)".to_string()
        };
        let recent_dialogue = sanitize_user_memory_prompt_text(&recent_dialogue, 4_000)
            .map(|value| value.text)
            .unwrap_or_else(|| "[REDACTED_SECRET]".to_string());
        let saved_facts = self
            .build_saved_user_facts_context(project_id, conversation_id, &prompt_message)
            .await
            .unwrap_or_else(|| "## Saved User Facts\n(none)".to_string());
        let saved_facts = sanitize_user_memory_prompt_text(&saved_facts, 4_000)
            .map(|value| value.text)
            .unwrap_or_else(|| "## Saved User Facts\n[REDACTED_SECRET]".to_string());
        let time_context = self.build_ambient_time_context().await;
        let response_shape = r#"{"memories":[{"key":"stable_snake_case_semantic_key","value":"self-contained memory text","category":"profile_fact|assistant_preference|work_preference|project_domain_memory|ephemeral_context|knowledge|other","topics":["semantic_topic_or_domain"],"kind":"identity|assistant_preference|work_preference|project_domain_memory|ephemeral_context|knowledge|preference|location|workflow|constraint|personal_fact|other","durability":"permanent|temporary|situational","scope":"global|project|conversation","sensitivity":"prompt_safe|personal_identifier|sensitive|crisis_sensitive","valid_from":"RFC3339 UTC timestamp or null","expires_at":"RFC3339 UTC timestamp or null","review_at":"RFC3339 UTC timestamp or null","confidence":0.95,"reason":"brief semantic rationale","looks_sensitive":false,"sensitive_reason":"brief reason or empty"}],"retractions":[{"key":"stable_snake_case_semantic_key","kind":"identity|assistant_preference|work_preference|project_domain_memory|ephemeral_context|knowledge|preference|location|workflow|constraint|personal_fact|other or null","scope":"global|project|conversation","confidence":0.95,"reason":"brief semantic rationale"}]}"#;
        let prompt = build_user_memory_capture_prompt(
            &time_context,
            &recent_dialogue,
            &prompt_message,
            &saved_facts,
            response_shape,
        );

        let memory_capture_candidates = self.user_memory_capture_llm_candidates();
        let memory_capture_candidate_count = memory_capture_candidates.len();
        if memory_capture_candidates.is_empty() {
            tracing::debug!(
                "Skipping user fact memory extraction because no chat model is configured"
            );
            capture_event.status = "failed".to_string();
            capture_event.completed_at = Some(chrono::Utc::now().to_rfc3339());
            capture_event.updated_at = chrono::Utc::now().to_rfc3339();
            capture_event.attempt_metadata = serde_json::json!({
                "schema_version": 1,
                "message_chars": trimmed.chars().count(),
                "candidate_count": 0,
                "semantic_capture_key": source_hash.clone(),
                "source": USER_LEARNED_MEMORY_CAPTURE_SOURCE,
            });
            capture_event.error_history = serde_json::json!([user_memory_capture_error_entry(
                "no_model_configured",
                "Skipped user memory capture because no chat model was configured.",
            )]);
            let _ = self
                .storage
                .upsert_memory_capture_event(&capture_event)
                .await;
            return;
        }
        let memory_capture_timeout_ms =
            self.user_memory_capture_timeout_ms(&memory_capture_candidates);
        let capture_outcome = self
            .run_memory_capture_with_schema_recovery(
                channel,
                conversation_id,
                &prompt,
                response_shape,
                memory_capture_candidates,
                memory_capture_timeout_ms,
            )
            .await;
        let capture_attempts_json = serde_json::to_value(&capture_outcome.attempts)
            .unwrap_or_else(|_| serde_json::json!([]));
        let selected_slot_id = capture_outcome.selected_slot_id.clone();
        let selected_slot_label = capture_outcome.selected_slot_label.clone();
        let selected_provider = capture_outcome.selected_provider.clone();
        let selected_model = capture_outcome.selected_model.clone();
        let selected_stage = capture_outcome.selected_stage.clone();
        capture_event.attempt_metadata = serde_json::json!({
            "schema_version": 1,
            "message_chars": trimmed.chars().count(),
            "candidate_count": memory_capture_candidate_count,
            "semantic_capture_key": source_hash.clone(),
            "timeout_ms": memory_capture_timeout_ms,
            "attempts": capture_attempts_json.clone(),
            "selected_slot_id": selected_slot_id.clone(),
            "selected_slot_label": selected_slot_label.clone(),
            "selected_provider": selected_provider.clone(),
            "selected_model": selected_model.clone(),
            "selected_stage": selected_stage.clone(),
            "source": USER_LEARNED_MEMORY_CAPTURE_SOURCE,
        });
        let Some(payload) = capture_outcome.payload.clone() else {
            capture_event.status = "failed".to_string();
            capture_event.completed_at = Some(chrono::Utc::now().to_rfc3339());
            capture_event.updated_at = chrono::Utc::now().to_rfc3339();
            capture_event.error_history = serde_json::json!([user_memory_capture_error_entry(
                "schema_recovery_failed",
                capture_outcome.terminal_error.clone().unwrap_or_else(|| {
                    "Memory capture failed to produce a schema-compliant payload.".to_string()
                }),
            )]);
            let _ = self
                .storage
                .upsert_memory_capture_event(&capture_event)
                .await;
            return;
        };
        let retractions_raw = payload.get("retractions");
        let items_raw = payload.get("memories");
        let retractions = retractions_raw
            .and_then(|value| value.as_array())
            .cloned()
            .unwrap_or_default();
        let items = items_raw
            .and_then(|value| value.as_array())
            .cloned()
            .unwrap_or_default();
        if retractions.is_empty() && items.is_empty() {
            capture_event.status = "noop".to_string();
            capture_event.completed_at = Some(chrono::Utc::now().to_rfc3339());
            capture_event.updated_at = chrono::Utc::now().to_rfc3339();
            capture_event.error_history = serde_json::json!([]);
            let _ = self
                .storage
                .upsert_memory_capture_event(&capture_event)
                .await;
            return;
        }
        let model_metadata = serde_json::json!({
            "capture_event_id": event_id.clone(),
            "provider": selected_provider.clone(),
            "model": selected_model.clone(),
            "slot_id": selected_slot_id.clone(),
            "slot_label": selected_slot_label.clone(),
            "stage": selected_stage.clone(),
        });
        let mut applied_count = 0usize;
        let mut queued_count = 0usize;
        let mut rejected_sensitive_count = 0usize;
        let mut semantic_rejected_count = 0usize;
        let mut seen_retractions = HashSet::new();
        for item in &retractions {
            let Some(raw_key) = item.get("key").and_then(|value| value.as_str()) else {
                continue;
            };
            let Some(key) = normalize_user_fact_key(raw_key) else {
                continue;
            };
            let retraction_kind = item.get("kind").and_then(|value| value.as_str());
            let scope = item
                .get("scope")
                .and_then(|value| value.as_str())
                .map(|value| normalize_user_memory_scope(Some(value)).to_string());
            let confidence = item
                .get("confidence")
                .and_then(|value| value.as_f64())
                .map(|value| value.clamp(0.0, 1.0) as f32)
                .unwrap_or(0.9);
            if confidence < 0.55 {
                continue;
            }
            let dedupe_scope = scope.clone().unwrap_or_else(|| "*".to_string());
            let dedupe_kind = retraction_kind
                .map(|value| normalize_self_memory_lookup_kind(Some(value)).to_string())
                .unwrap_or_else(|| "any".to_string());
            if !seen_retractions.insert((key.clone(), dedupe_kind, dedupe_scope)) {
                continue;
            }
            let reason = user_memory_json_text_field(item, "reason", 180);
            let (resolved_scope, resolved_project_id, resolved_conversation_id) =
                learned_user_memory_scope_ids(scope.as_deref(), project_id, conversation_id);
            let semantic_key = user_memory_operation_semantic_key(
                "retract",
                Some(&key),
                retraction_kind.unwrap_or("any"),
                "permanent",
                resolved_scope,
                scope.is_some(),
                resolved_project_id,
                resolved_conversation_id,
                None,
            );
            let mut operation_model_metadata = model_metadata.clone();
            if let Some(object) = operation_model_metadata.as_object_mut() {
                object.insert(
                    "scope_explicit".to_string(),
                    serde_json::Value::Bool(scope.is_some()),
                );
                object.insert(
                    "requested_scope".to_string(),
                    serde_json::Value::String(scope.clone().unwrap_or_else(|| "any".to_string())),
                );
                object.insert(
                    "semantic_key".to_string(),
                    serde_json::Value::String(semantic_key),
                );
            }
            let now = chrono::Utc::now().to_rfc3339();
            let mut operation = crate::storage::memory_operation::Model {
                id: format!("memory-operation-{}", uuid::Uuid::new_v4()),
                capture_event_id: Some(event_id.clone()),
                operation_type: "retract".to_string(),
                status: "queued_review".to_string(),
                target_memory_id: None,
                applied_memory_id: None,
                key: Some(key.clone()),
                value: None,
                memory_kind: retraction_kind.unwrap_or("any").to_string(),
                durability: "permanent".to_string(),
                scope: resolved_scope.to_string(),
                project_id: resolved_project_id.map(str::to_string),
                conversation_id: resolved_conversation_id.map(str::to_string),
                confidence: confidence as f64,
                looks_sensitive: false,
                sensitive_reason: None,
                valid_from: None,
                expires_at: None,
                review_at: None,
                rationale: reason.clone(),
                evidence_refs: user_memory_operation_evidence_refs(
                    source_message_id,
                    Some(&event_id),
                    channel,
                ),
                model_metadata: operation_model_metadata,
                apply_metadata: serde_json::json!({}),
                applied_at: None,
                reviewed_at: None,
                review_notes: None,
                created_at: now.clone(),
                updated_at: now,
            };
            if confidence as f64 >= USER_MEMORY_OPERATION_AUTO_APPLY_CONFIDENCE {
                operation.status = "pending_apply".to_string();
                if let Err(error) = self.storage.upsert_memory_operation(&operation).await {
                    tracing::warn!(
                        "Failed to stage memory retraction operation '{}' for conversation {:?}: {}",
                        operation.id,
                        conversation_id,
                        error
                    );
                    continue;
                }
                match self
                    .apply_memory_operation(&operation, "capture_auto_apply")
                    .await
                {
                    Ok(_) => {
                        applied_count += 1;
                    }
                    Err(error) => {
                        operation.status = "queued_review".to_string();
                        let auto_apply_note = format!(
                            "Auto-apply failed: {}",
                            safe_truncate(&error.to_string(), 240)
                        );
                        operation.review_notes = Some(auto_apply_note.clone());
                        operation.updated_at = chrono::Utc::now().to_rfc3339();
                        let _ = self.storage.upsert_memory_operation(&operation).await;
                        if let Err(queue_error) =
                            self.queue_memory_operation_candidate(&operation).await
                        {
                            operation.status = "apply_failed".to_string();
                            operation.review_notes = Some(format!(
                                "{} Review queue failed: {}",
                                auto_apply_note,
                                safe_truncate(&queue_error.to_string(), 200)
                            ));
                            operation.apply_metadata = serde_json::json!({
                                "auto_apply_error": safe_truncate(&error.to_string(), 240),
                                "review_queue_error": safe_truncate(&queue_error.to_string(), 240),
                                "failed_at": chrono::Utc::now().to_rfc3339(),
                            });
                            operation.updated_at = chrono::Utc::now().to_rfc3339();
                            let _ = self.storage.upsert_memory_operation(&operation).await;
                            tracing::warn!(
                                "Failed to queue review candidate for memory operation '{}': {}",
                                operation.id,
                                queue_error
                            );
                        } else {
                            queued_count += 1;
                        }
                    }
                }
            } else {
                if let Err(error) = self.storage.upsert_memory_operation(&operation).await {
                    tracing::warn!(
                        "Failed to stage review memory retraction operation '{}' for conversation {:?}: {}",
                        operation.id,
                        conversation_id,
                        error
                    );
                    continue;
                }
                if let Err(error) = self.queue_memory_operation_candidate(&operation).await {
                    tracing::warn!(
                        "Failed to queue review candidate for memory operation '{}': {}",
                        operation.id,
                        error
                    );
                    continue;
                }
                queued_count += 1;
            }
        }
        let mut seen = HashSet::new();
        for item in &items {
            let Some(raw_key) = item.get("key").and_then(|value| value.as_str()) else {
                continue;
            };
            let Some(raw_value) = item.get("value").and_then(|value| value.as_str()) else {
                continue;
            };
            let Some(key) = normalize_user_fact_key(raw_key) else {
                continue;
            };
            let Some(value) = normalize_user_memory_text(raw_value, 320) else {
                continue;
            };
            let model_marked_sensitive = user_memory_capture_item_looks_sensitive(item);
            let mut durability = normalize_user_memory_durability(
                item.get("durability").and_then(|value| value.as_str()),
            )
            .to_string();
            let valid_from = user_memory_json_datetime_field(item, "valid_from");
            let mut expires_at = user_memory_json_datetime_field(item, "expires_at");
            let review_at = user_memory_json_datetime_field(item, "review_at");
            if durability == "permanent" {
                expires_at = None;
            } else if durability == "temporary" && expires_at.is_none() {
                durability = "situational".to_string();
            }
            let capture_now = chrono::Utc::now();
            if expires_at
                .as_ref()
                .map(|dt| dt <= &capture_now)
                .unwrap_or(false)
            {
                continue;
            }
            let mut scope =
                normalize_user_memory_scope(item.get("scope").and_then(|value| value.as_str()))
                    .to_string();
            if !seen.insert((
                key.clone(),
                value.clone(),
                durability.clone(),
                scope.clone(),
            )) {
                continue;
            }
            let confidence = item
                .get("confidence")
                .and_then(|value| value.as_f64())
                .map(|value| value.clamp(0.0, 1.0) as f32)
                .unwrap_or(0.9);
            if confidence < 0.55 {
                continue;
            }
            let reason = user_memory_json_text_field(item, "reason", 180);
            let sensitive_reason = user_memory_json_text_field(item, "sensitive_reason", 180);
            let raw_kind = item
                .get("kind")
                .and_then(|value| value.as_str())
                .and_then(|value| normalize_user_memory_text(value, 64))
                .unwrap_or_else(|| "other".to_string());
            let looks_sensitive = model_marked_sensitive;
            let sensitivity =
                user_memory_capture_item_sensitivity(item, &key, &value, Some(&raw_kind));
            let semantic_kind = learned_user_memory_semantic_kind(Some(&key), Some(&raw_kind));
            let category = normalize_learned_user_memory_category(
                item.get("category").and_then(|value| value.as_str()),
                Some(semantic_kind),
            )
            .to_string();
            let topics = crate::core::memory_schema::normalize_memory_topics(item.get("topics"), 8);
            let review_payload = serde_json::json!({
                "key": key,
                "value": value,
                "kind": raw_kind,
                "category": category,
                "topics": topics,
                "durability": durability,
                "scope": scope,
                "confidence": confidence,
                "reason": reason,
            });
            if let Some(review) = self
                .review_user_memory_candidate_for_storage(
                    &prompt_message,
                    &recent_dialogue,
                    &review_payload,
                    channel,
                    conversation_id,
                )
                .await
            {
                if user_memory_candidate_review_should_skip(&review) {
                    semantic_rejected_count += 1;
                    tracing::debug!(
                        "Skipped learned user memory '{}' after semantic review: {}",
                        key,
                        review.reason.as_deref().unwrap_or("not durable memory")
                    );
                    continue;
                }
            }
            if user_memory_candidate_is_operational_artifact(&key, &value, &raw_kind, &category) {
                tracing::debug!(
                    "Skipped learned user memory '{}' because it describes integration/runtime setup state, not durable user memory",
                    key
                );
                continue;
            }
            if crate::core::memory_schema::memory_category_is_ephemeral(&category) {
                if scope == "global" {
                    scope = "conversation".to_string();
                }
                if durability == "permanent" {
                    durability = "situational".to_string();
                }
                if expires_at.is_none() && review_at.is_none() {
                    expires_at = Some(capture_now + chrono::Duration::days(7));
                }
            }
            let (resolved_scope, resolved_project_id, resolved_conversation_id) =
                learned_user_memory_scope_ids(Some(&scope), project_id, conversation_id);
            let (exact_target_memory_id, _) = learned_user_memory_keys(
                &key,
                &durability,
                resolved_project_id,
                resolved_conversation_id,
            );
            let target_memory_id = if looks_sensitive {
                exact_target_memory_id.clone()
            } else {
                self.semantically_matching_user_memory_target_id(
                    &key,
                    &value,
                    semantic_kind,
                    &durability,
                    resolved_scope,
                    resolved_project_id,
                    resolved_conversation_id,
                    Some(&exact_target_memory_id),
                )
                .await
                .unwrap_or_else(|| exact_target_memory_id.clone())
            };
            if let Some(existing_id) = self
                .existing_equivalent_user_memory_id(
                    &key,
                    &value,
                    semantic_kind,
                    resolved_project_id,
                    resolved_conversation_id,
                    Some(&target_memory_id),
                )
                .await
            {
                tracing::debug!(
                    "Skipped staging duplicate learned user memory '{}' because it is already covered by '{}'",
                    key,
                    existing_id
                );
                continue;
            }
            let operation_type = match self.storage.get_experience_item(&target_memory_id).await {
                Ok(Some(existing)) if existing.status == "active" => "update",
                _ => "add",
            };
            let scope_explicit = item.get("scope").and_then(|value| value.as_str()).is_some();
            let semantic_key = user_memory_operation_semantic_key(
                operation_type,
                Some(&key),
                &raw_kind,
                &durability,
                resolved_scope,
                scope_explicit,
                resolved_project_id,
                resolved_conversation_id,
                Some(&target_memory_id),
            );
            let mut operation_model_metadata = model_metadata.clone();
            if let Some(object) = operation_model_metadata.as_object_mut() {
                object.insert(
                    "scope_explicit".to_string(),
                    serde_json::Value::Bool(scope_explicit),
                );
                object.insert(
                    "semantic_key".to_string(),
                    serde_json::Value::String(semantic_key),
                );
                object.insert(
                    "sensitivity".to_string(),
                    serde_json::Value::String(sensitivity.as_str().to_string()),
                );
                object.insert(
                    "memory_category".to_string(),
                    serde_json::Value::String(category.clone()),
                );
                object.insert(
                    "topics".to_string(),
                    serde_json::Value::Array(
                        topics
                            .iter()
                            .cloned()
                            .map(serde_json::Value::String)
                            .collect(),
                    ),
                );
            }
            let now = chrono::Utc::now().to_rfc3339();
            let mut operation = crate::storage::memory_operation::Model {
                id: format!("memory-operation-{}", uuid::Uuid::new_v4()),
                capture_event_id: Some(event_id.clone()),
                operation_type: operation_type.to_string(),
                status: "queued_review".to_string(),
                target_memory_id: Some(target_memory_id.clone()),
                applied_memory_id: None,
                key: Some(key.clone()),
                value: if looks_sensitive {
                    None
                } else {
                    Some(value.clone())
                },
                memory_kind: raw_kind,
                durability: durability.clone(),
                scope: resolved_scope.to_string(),
                project_id: resolved_project_id.map(str::to_string),
                conversation_id: resolved_conversation_id.map(str::to_string),
                confidence: confidence as f64,
                looks_sensitive,
                sensitive_reason: sensitive_reason.clone(),
                valid_from: valid_from.map(|value| value.to_rfc3339()),
                expires_at: expires_at.map(|value| value.to_rfc3339()),
                review_at: review_at.map(|value| value.to_rfc3339()),
                rationale: reason.clone(),
                evidence_refs: user_memory_operation_evidence_refs(
                    source_message_id,
                    Some(&event_id),
                    channel,
                ),
                model_metadata: operation_model_metadata,
                apply_metadata: serde_json::json!({}),
                applied_at: None,
                reviewed_at: None,
                review_notes: None,
                created_at: now.clone(),
                updated_at: now,
            };
            if looks_sensitive {
                tracing::warn!(
                    "Rejected learned user memory '{}' because the capture model marked it credential-sensitive",
                    key
                );
                operation.status = "rejected_sensitive".to_string();
                if let Err(error) = self.storage.upsert_memory_operation(&operation).await {
                    tracing::warn!(
                        "Failed to persist rejected sensitive memory operation '{}' for conversation {:?}: {}",
                        operation.id,
                        conversation_id,
                        error
                    );
                } else {
                    rejected_sensitive_count += 1;
                }
                continue;
            }
            if confidence as f64 >= USER_MEMORY_OPERATION_AUTO_APPLY_CONFIDENCE {
                operation.status = "pending_apply".to_string();
                if let Err(error) = self.storage.upsert_memory_operation(&operation).await {
                    tracing::warn!(
                        "Failed to stage memory operation '{}' for conversation {:?}: {}",
                        operation.id,
                        conversation_id,
                        error
                    );
                    continue;
                }
                match self
                    .apply_memory_operation(&operation, "capture_auto_apply")
                    .await
                {
                    Ok(_) => {
                        applied_count += 1;
                    }
                    Err(error) => {
                        operation.status = "queued_review".to_string();
                        let auto_apply_note = format!(
                            "Auto-apply failed: {}",
                            safe_truncate(&error.to_string(), 240)
                        );
                        operation.review_notes = Some(auto_apply_note.clone());
                        operation.updated_at = chrono::Utc::now().to_rfc3339();
                        let _ = self.storage.upsert_memory_operation(&operation).await;
                        if let Err(queue_error) =
                            self.queue_memory_operation_candidate(&operation).await
                        {
                            operation.status = "apply_failed".to_string();
                            operation.review_notes = Some(format!(
                                "{} Review queue failed: {}",
                                auto_apply_note,
                                safe_truncate(&queue_error.to_string(), 200)
                            ));
                            operation.apply_metadata = serde_json::json!({
                                "auto_apply_error": safe_truncate(&error.to_string(), 240),
                                "review_queue_error": safe_truncate(&queue_error.to_string(), 240),
                                "failed_at": chrono::Utc::now().to_rfc3339(),
                            });
                            operation.updated_at = chrono::Utc::now().to_rfc3339();
                            let _ = self.storage.upsert_memory_operation(&operation).await;
                            tracing::warn!(
                                "Failed to queue review candidate for memory operation '{}': {}",
                                operation.id,
                                queue_error
                            );
                        } else {
                            queued_count += 1;
                        }
                    }
                }
            } else {
                if let Err(error) = self.storage.upsert_memory_operation(&operation).await {
                    tracing::warn!(
                        "Failed to stage review memory operation '{}' for conversation {:?}: {}",
                        operation.id,
                        conversation_id,
                        error
                    );
                    continue;
                }
                if let Err(error) = self.queue_memory_operation_candidate(&operation).await {
                    tracing::warn!(
                        "Failed to queue review candidate for memory operation '{}': {}",
                        operation.id,
                        error
                    );
                    continue;
                }
                queued_count += 1;
            }
        }
        capture_event.status = if queued_count > 0 {
            "queued_review".to_string()
        } else if applied_count > 0 {
            "applied".to_string()
        } else if rejected_sensitive_count > 0 {
            "rejected_sensitive".to_string()
        } else if semantic_rejected_count > 0 {
            "noop".to_string()
        } else {
            "noop".to_string()
        };
        capture_event.completed_at = Some(chrono::Utc::now().to_rfc3339());
        capture_event.updated_at = chrono::Utc::now().to_rfc3339();
        capture_event.error_history = serde_json::json!([]);
        capture_event.attempt_metadata = serde_json::json!({
            "schema_version": 1,
            "message_chars": trimmed.chars().count(),
            "candidate_count": memory_capture_candidate_count,
            "semantic_capture_key": source_hash,
            "timeout_ms": memory_capture_timeout_ms,
            "attempts": capture_attempts_json,
            "selected_slot_id": selected_slot_id,
            "selected_slot_label": selected_slot_label,
            "selected_provider": selected_provider,
            "selected_model": selected_model,
            "selected_stage": selected_stage,
            "applied_count": applied_count,
            "queued_review_count": queued_count,
            "rejected_sensitive_count": rejected_sensitive_count,
            "semantic_rejected_count": semantic_rejected_count,
            "recovered_stale_prior_capture_count": retry_state.recovered_stale_count,
            "source": USER_LEARNED_MEMORY_CAPTURE_SOURCE,
        });
        let _ = self
            .storage
            .upsert_memory_capture_event(&capture_event)
            .await;
    }

    pub(super) async fn queue_memory_operation_candidate(
        &self,
        operation: &crate::storage::memory_operation::Model,
    ) -> Result<String> {
        let now = chrono::Utc::now().to_rfc3339();
        let candidate_type = user_memory_operation_candidate_type(&operation.operation_type);
        let subject_key = user_memory_operation_subject_key(operation);
        let title = match operation.operation_type.as_str() {
            "retract" => format!(
                "Review memory retraction: {}",
                operation.key.as_deref().unwrap_or(operation.id.as_str())
            ),
            "update" => format!(
                "Review memory update: {}",
                operation.key.as_deref().unwrap_or(operation.id.as_str())
            ),
            _ => format!(
                "Review memory addition: {}",
                operation.key.as_deref().unwrap_or(operation.id.as_str())
            ),
        };
        let candidate = crate::storage::learning_candidate::Model {
            id: format!("memory-candidate-{}", operation.id),
            candidate_type: candidate_type.to_string(),
            subject_key: subject_key.clone(),
            title,
            summary: operation.rationale.clone(),
            project_id: operation.project_id.clone(),
            conversation_id: operation.conversation_id.clone(),
            pattern_id: memory_operation_learning_candidate_pattern_id(operation),
            evidence_refs: operation.evidence_refs.clone(),
            proposed_content: serde_json::json!({
                "operation_id": operation.id.clone(),
                "capture_event_id": operation.capture_event_id.clone(),
                "operation_type": operation.operation_type.clone(),
                "target_memory_id": operation.target_memory_id.clone(),
                "applied_memory_id": operation.applied_memory_id.clone(),
                "key": operation.key.clone(),
                "value": operation.value.clone(),
                "memory_kind": operation.memory_kind.clone(),
                "memory_category": operation.model_metadata.get("memory_category").cloned(),
                "topics": operation.model_metadata.get("topics").cloned(),
                "durability": operation.durability.clone(),
                "scope": operation.scope.clone(),
                "confidence": operation.confidence,
                "looks_sensitive": operation.looks_sensitive,
                "sensitive_reason": operation.sensitive_reason.clone(),
                "sensitivity": operation.model_metadata.get("sensitivity").cloned(),
                "valid_from": operation.valid_from.clone(),
                "expires_at": operation.expires_at.clone(),
                "review_at": operation.review_at.clone(),
                "rationale": operation.rationale.clone(),
                "status": operation.status.clone(),
                "scope_explicit": user_memory_operation_scope_explicit(operation),
                "semantic_key": subject_key.clone(),
            }),
            confidence: operation.confidence,
            approval_status: "draft".to_string(),
            review_notes: operation.review_notes.clone(),
            reviewed_at: None,
            approved_ref: None,
            created_at: now.clone(),
            updated_at: now,
        };
        self.storage.upsert_learning_candidate(&candidate).await?;
        let prior_candidates = self
            .storage
            .list_learning_candidates_for_subject_key(
                &candidate.subject_key,
                MEMORY_OPERATION_CANDIDATE_TYPES,
                candidate.project_id.as_deref(),
                32,
            )
            .await?;
        for prior_candidate in prior_candidates {
            if prior_candidate.id == candidate.id || prior_candidate.approval_status != "draft" {
                continue;
            }
            let superseded = self
                .storage
                .update_learning_candidate_review_if_status(
                    &prior_candidate.id,
                    "draft",
                    "superseded",
                    Some("Superseded by a newer memory operation candidate."),
                    None,
                )
                .await?;
            if !superseded {
                continue;
            }
            if let Some(operation_id) = prior_candidate
                .proposed_content
                .get("operation_id")
                .and_then(|value| value.as_str())
                .map(str::trim)
                .filter(|value| !value.is_empty())
            {
                if let Some(mut prior_operation) =
                    self.storage.get_memory_operation(operation_id).await?
                {
                    if prior_operation.status == "queued_review" {
                        prior_operation.status = "superseded".to_string();
                        prior_operation.reviewed_at = Some(chrono::Utc::now().to_rfc3339());
                        prior_operation.review_notes =
                            Some("Superseded by a newer memory operation candidate.".to_string());
                        prior_operation.updated_at = chrono::Utc::now().to_rfc3339();
                        let _ = self.storage.upsert_memory_operation(&prior_operation).await;
                    }
                }
            }
        }
        Ok(candidate.id)
    }

    pub(super) async fn apply_memory_operation_by_id_with_source(
        &self,
        operation_id: &str,
        apply_source: &str,
    ) -> Result<String> {
        let operation = self
            .storage
            .get_memory_operation(operation_id)
            .await?
            .ok_or_else(|| anyhow::anyhow!("Memory operation not found."))?;
        self.apply_memory_operation(&operation, apply_source).await
    }

    pub(super) async fn apply_memory_operation(
        &self,
        operation: &crate::storage::memory_operation::Model,
        apply_source: &str,
    ) -> Result<String> {
        let mut operation_update = operation.clone();
        let applied_at = chrono::Utc::now().to_rfc3339();
        let source_message_id =
            user_memory_operation_evidence_ref_value(&operation.evidence_refs, "message:");
        let capture_event_id =
            user_memory_operation_evidence_ref_value(&operation.evidence_refs, "capture_event:");
        let evidence_channel =
            user_memory_operation_evidence_ref_value(&operation.evidence_refs, "channel:")
                .unwrap_or_else(|| "chat".to_string());
        let result = async {
            match operation.operation_type.as_str() {
                "add" | "update" => {
                    let key = operation
                        .key
                        .as_deref()
                        .ok_or_else(|| anyhow::anyhow!("Memory operation is missing key."))?;
                    let value = operation
                        .value
                        .as_deref()
                        .ok_or_else(|| anyhow::anyhow!("Memory operation is missing value."))?;
                    let explicit_target_memory_id =
                        memory_operation_explicit_upsert_target_id(operation);
                    let operation_topics = crate::core::memory_schema::normalize_memory_topics(
                        operation.model_metadata.get("topics"),
                        8,
                    );
                    let memory_id = self
                        .upsert_learned_user_memory(
                            key,
                            value,
                            Some(&operation.memory_kind),
                            Some(&operation.durability),
                            Some(&operation.scope),
                            operation.confidence.clamp(0.0, 1.0) as f32,
                            &evidence_channel,
                            operation.conversation_id.as_deref(),
                            operation.project_id.as_deref(),
                            USER_LEARNED_MEMORY_CAPTURE_SOURCE,
                            operation
                                .valid_from
                                .as_deref()
                                .and_then(parse_ambient_rfc3339),
                            operation
                                .expires_at
                                .as_deref()
                                .and_then(parse_ambient_rfc3339),
                            operation
                                .review_at
                                .as_deref()
                                .and_then(parse_ambient_rfc3339),
                            operation.rationale.as_deref(),
                            operation
                                .model_metadata
                                .get("sensitivity")
                                .and_then(|value| value.as_str()),
                            operation
                                .model_metadata
                                .get("memory_category")
                                .and_then(|value| value.as_str()),
                            &operation_topics,
                            explicit_target_memory_id,
                        )
                        .await?;
                    self.sync_applied_memory_to_user_preferences(operation, key, value)
                        .await;
                    Ok::<String, anyhow::Error>(memory_id)
                }
                "retract" => {
                    let key = operation
                        .key
                        .as_deref()
                        .ok_or_else(|| anyhow::anyhow!("Memory retraction is missing key."))?;
                    let scope = if user_memory_operation_scope_explicit(operation) {
                        Some(operation.scope.as_str())
                    } else {
                        None
                    };
                    let retracted_ids = self
                        .retract_learned_user_memory(
                            key,
                            Some(&operation.memory_kind),
                            scope,
                            &evidence_channel,
                            operation.conversation_id.as_deref(),
                            operation.project_id.as_deref(),
                            operation.rationale.as_deref(),
                        )
                        .await;
                    Ok::<String, anyhow::Error>(
                        retracted_ids
                            .first()
                            .cloned()
                            .or_else(|| operation.target_memory_id.clone())
                            .unwrap_or_else(|| operation.id.clone()),
                    )
                }
                other => Err(anyhow::anyhow!(
                    "Unsupported memory operation type '{}'.",
                    other
                )),
            }
        }
        .await;

        match result {
            Ok(approved_ref) => {
                let linked_memory_id = operation.target_memory_id.clone().or_else(|| {
                    if operation.operation_type == "retract" && approved_ref == operation.id {
                        None
                    } else {
                        Some(approved_ref.clone())
                    }
                });
                operation_update.status = "applied".to_string();
                operation_update.applied_memory_id = linked_memory_id
                    .clone()
                    .or_else(|| Some(approved_ref.clone()));
                operation_update.applied_at = Some(applied_at.clone());
                if apply_source != "capture_auto_apply" {
                    operation_update.reviewed_at = Some(applied_at.clone());
                }
                operation_update.review_notes = None;
                operation_update.updated_at = applied_at.clone();
                operation_update.apply_metadata = serde_json::json!({
                    "applied_via": apply_source,
                    "applied_at": applied_at,
                });
                self.storage
                    .upsert_memory_operation(&operation_update)
                    .await?;
                if let Some(memory_id) = linked_memory_id.as_deref() {
                    if let Some(source_message_id) = source_message_id.as_deref() {
                        let link = crate::storage::memory_evidence_link::Model {
                            id: format!(
                                "memory-evidence-{}",
                                ambient_stable_hash(&[
                                    operation.id.as_str(),
                                    memory_id,
                                    "message",
                                    source_message_id,
                                ])
                            ),
                            operation_id: Some(operation.id.clone()),
                            memory_id: Some(memory_id.to_string()),
                            evidence_kind: "message".to_string(),
                            evidence_ref: source_message_id.to_string(),
                            source_message_id: Some(source_message_id.to_string()),
                            capture_event_id: capture_event_id.clone(),
                            project_id: operation.project_id.clone(),
                            conversation_id: operation.conversation_id.clone(),
                            metadata: serde_json::json!({ "applied_via": apply_source }),
                            created_at: chrono::Utc::now().to_rfc3339(),
                        };
                        let _ = self.storage.upsert_memory_evidence_link(&link).await;
                    }
                    if let Some(capture_event_id) = capture_event_id.as_deref() {
                        let link = crate::storage::memory_evidence_link::Model {
                            id: format!(
                                "memory-evidence-{}",
                                ambient_stable_hash(&[
                                    operation.id.as_str(),
                                    memory_id,
                                    "capture_event",
                                    capture_event_id,
                                ])
                            ),
                            operation_id: Some(operation.id.clone()),
                            memory_id: Some(memory_id.to_string()),
                            evidence_kind: "capture_event".to_string(),
                            evidence_ref: capture_event_id.to_string(),
                            source_message_id: source_message_id.clone(),
                            capture_event_id: Some(capture_event_id.to_string()),
                            project_id: operation.project_id.clone(),
                            conversation_id: operation.conversation_id.clone(),
                            metadata: serde_json::json!({ "applied_via": apply_source }),
                            created_at: chrono::Utc::now().to_rfc3339(),
                        };
                        let _ = self.storage.upsert_memory_evidence_link(&link).await;
                    }
                }
                Ok(approved_ref)
            }
            Err(error) => {
                operation_update.status = "apply_failed".to_string();
                operation_update.review_notes = Some(safe_truncate(&error.to_string(), 240));
                operation_update.updated_at = chrono::Utc::now().to_rfc3339();
                operation_update.apply_metadata = serde_json::json!({
                    "applied_via": apply_source,
                    "failed_at": chrono::Utc::now().to_rfc3339(),
                    "error": safe_truncate(&error.to_string(), 240),
                });
                let _ = self
                    .storage
                    .upsert_memory_operation(&operation_update)
                    .await;
                Err(error)
            }
        }
    }

    pub(super) async fn recent_messages_for_intent_gating(
        &self,
        conversation_id: &str,
        current_message: &str,
    ) -> Vec<ConversationMessage> {
        let mut history = {
            let guard = self.conversation_history.read().await;
            guard.get(conversation_id).cloned().unwrap_or_default()
        };
        if let Some(last) = history.last() {
            if last.role == "user" && last.content.trim() == current_message.trim() {
                history.pop();
            }
        }
        if !history.is_empty() {
            return history;
        }

        let mut stored = self
            .encrypted_storage
            .get_recent_messages_decrypted(conversation_id, 8)
            .await
            .unwrap_or_default();
        if let Some(last) = stored.last() {
            if last.role == "user" && last.content.trim() == current_message.trim() {
                stored.pop();
            }
        }
        stored
            .into_iter()
            .map(|msg| ConversationMessage {
                role: msg.role,
                content: msg.content,
                _timestamp: Agent::parse_message_timestamp(&msg.timestamp),
            })
            .collect()
    }

    pub(super) async fn build_saved_user_facts_context(
        &self,
        project_id: Option<&str>,
        conversation_id: Option<&str>,
        current_message: &str,
    ) -> Option<String> {
        build_saved_user_facts_context_from_storage(
            &self.storage,
            self.embedding_client.as_deref(),
            project_id,
            conversation_id,
            current_message,
        )
        .await
    }

    pub(super) async fn build_ambient_time_context(&self) -> String {
        let now = chrono::Utc::now();
        let timezone = {
            let profile = self.user_profile.read().await;
            profile
                .timezone
                .as_deref()
                .map(str::trim)
                .filter(|value| !value.is_empty())
                .map(|value| value.to_string())
        };

        if let Some(timezone) = timezone {
            if let Ok(tz) = timezone.parse::<chrono_tz::Tz>() {
                return format!(
                    "UTC now: {}\nUser timezone: {}\nUser local now: {}",
                    now.to_rfc3339(),
                    timezone,
                    now.with_timezone(&tz).to_rfc3339()
                );
            }
            return format!(
                "UTC now: {}\nUser timezone setting could not be parsed: {}",
                now.to_rfc3339(),
                timezone
            );
        }

        format!("UTC now: {}\nUser timezone: not set", now.to_rfc3339())
    }

    pub(super) async fn upsert_learned_user_memory(
        &self,
        key: &str,
        value: &str,
        kind: Option<&str>,
        durability: Option<&str>,
        scope: Option<&str>,
        confidence: f32,
        channel: &str,
        conversation_id: Option<&str>,
        project_id: Option<&str>,
        source: &str,
        valid_from: Option<chrono::DateTime<chrono::Utc>>,
        mut expires_at: Option<chrono::DateTime<chrono::Utc>>,
        review_at: Option<chrono::DateTime<chrono::Utc>>,
        reason: Option<&str>,
        sensitivity: Option<&str>,
        category: Option<&str>,
        topics: &[String],
        target_memory_id: Option<&str>,
    ) -> Result<String> {
        let Some(key) = normalize_user_fact_key(key) else {
            anyhow::bail!("Memory operation has an invalid or empty key.");
        };
        let Some(sanitized_memory) = sanitize_learned_user_memory_content_for_storage(&key, value)
        else {
            tracing::warn!(
                "Skipped learned user memory '{}' because its candidate value looked like credential material",
                key
            );
            anyhow::bail!(
                "Memory operation value looked like credential or secret material after sanitization."
            );
        };
        let value = sanitized_memory.value;
        let content = sanitized_memory.content;
        let mut memory_secret_redacted = sanitized_memory.redacted_secret;
        let mut durability = normalize_user_memory_durability(durability).to_string();
        if durability == "permanent" {
            expires_at = None;
        } else if durability == "temporary" && expires_at.is_none() {
            durability = "situational".to_string();
        }
        let capture_now = chrono::Utc::now();
        if expires_at
            .as_ref()
            .map(|dt| dt <= &capture_now)
            .unwrap_or(false)
        {
            anyhow::bail!("Memory operation expiry is already in the past.");
        }
        let confidence = confidence.clamp(0.0, 1.0);
        if confidence < 0.55 {
            anyhow::bail!(
                "Memory operation confidence {:.2} is below the storage threshold.",
                confidence
            );
        }
        let normalized_kind = normalize_user_memory_kind(kind);
        let semantic_kind = learned_user_memory_semantic_kind(Some(&key), kind);
        let memory_category = normalize_learned_user_memory_category(category, Some(semantic_kind));
        let memory_topics = topics
            .iter()
            .map(|topic| serde_json::Value::String(topic.clone()))
            .collect::<Vec<_>>();
        let sensitivity = saved_memory_sensitivity_from_parts(
            Some(&key),
            &value,
            Some(semantic_kind),
            sensitivity,
        );
        let (scope, scoped_project_id, scoped_conversation_id) =
            learned_user_memory_scope_ids(scope, project_id, conversation_id);
        let (default_id, normalized_key) =
            learned_user_memory_keys(&key, &durability, scoped_project_id, scoped_conversation_id);
        let explicit_target_id = target_memory_id
            .map(str::trim)
            .filter(|value| !value.is_empty())
            .map(str::to_string);
        let semantic_target_id = if explicit_target_id.is_none() {
            self.semantically_matching_user_memory_target_id(
                &key,
                &value,
                semantic_kind,
                &durability,
                scope,
                scoped_project_id,
                scoped_conversation_id,
                Some(&default_id),
            )
            .await
        } else {
            None
        };
        let id = explicit_target_id
            .or(semantic_target_id)
            .unwrap_or(default_id);
        if let Some(existing_id) = self
            .existing_equivalent_user_memory_id(
                &key,
                &value,
                semantic_kind,
                scoped_project_id,
                scoped_conversation_id,
                Some(&id),
            )
            .await
        {
            return Ok(existing_id);
        }
        let now = capture_now.to_rfc3339();
        let txn = match self
            .storage
            .begin_experience_memory_write_txn(
                normalized_kind,
                scope,
                scoped_project_id,
                scoped_conversation_id,
            )
            .await
        {
            Ok(txn) => txn,
            Err(error) => {
                tracing::warn!(
                    "Failed to start learned user memory transaction for '{}' (scope={}): {}",
                    key,
                    scope,
                    error
                );
                return Err(error).with_context(|| {
                    format!(
                        "Failed to start learned user memory transaction for '{}' (scope={}).",
                        key, scope
                    )
                });
            }
        };
        let existing = match self.storage.get_experience_item_txn(&txn, &id).await {
            Ok(existing) => existing,
            Err(error) => {
                tracing::warn!(
                    "Failed to load learned user memory '{}' before upsert: {}",
                    key,
                    error
                );
                let _ = txn.rollback().await;
                return Err(error).with_context(|| {
                    format!(
                        "Failed to load learned user memory '{}' before upsert.",
                        key
                    )
                });
            }
        };
        let mut metadata = existing
            .as_ref()
            .map(ambient_intent_metadata_object)
            .unwrap_or_default();
        if sanitize_user_memory_metadata_for_storage(&mut metadata) {
            memory_secret_redacted = true;
        }
        metadata.insert(
            "source".to_string(),
            serde_json::Value::String(source.to_string()),
        );
        metadata.insert(
            "channel".to_string(),
            serde_json::Value::String(channel.to_string()),
        );
        metadata.insert("key".to_string(), serde_json::Value::String(key.clone()));
        if semantic_kind != "other" {
            metadata.insert(
                "memory_kind".to_string(),
                serde_json::Value::String(semantic_kind.to_string()),
            );
        } else if let Some(memory_kind) = kind.and_then(|raw| normalize_user_memory_text(raw, 64)) {
            metadata.insert(
                "memory_kind".to_string(),
                serde_json::Value::String(memory_kind),
            );
        }
        metadata.insert(
            "durability".to_string(),
            serde_json::Value::String(durability.clone()),
        );
        metadata.insert(
            "scope".to_string(),
            serde_json::Value::String(scope.to_string()),
        );
        metadata.insert(
            "sensitivity".to_string(),
            serde_json::Value::String(sensitivity.as_str().to_string()),
        );
        metadata.insert(
            "memory_category".to_string(),
            serde_json::Value::String(memory_category.to_string()),
        );
        metadata.insert(
            "topics".to_string(),
            serde_json::Value::Array(memory_topics),
        );
        if let Some(dt) = valid_from {
            metadata.insert(
                "valid_from".to_string(),
                serde_json::Value::String(dt.to_rfc3339()),
            );
        }
        if let Some(dt) = expires_at {
            metadata.insert(
                "expires_at".to_string(),
                serde_json::Value::String(dt.to_rfc3339()),
            );
        }
        if let Some(dt) = review_at {
            metadata.insert(
                "review_at".to_string(),
                serde_json::Value::String(dt.to_rfc3339()),
            );
        }
        if let Some(raw_reason) = reason {
            match sanitize_user_memory_metadata_text_for_storage(raw_reason, 180) {
                Some(reason) => {
                    if reason.redacted_secret {
                        memory_secret_redacted = true;
                    }
                    metadata.insert("reason".to_string(), serde_json::Value::String(reason.text));
                }
                None => {
                    metadata.remove("reason");
                    memory_secret_redacted = true;
                }
            }
        }
        if memory_secret_redacted {
            metadata.insert("secret_redacted".to_string(), serde_json::Value::Bool(true));
        }
        let merged_confidence = existing
            .as_ref()
            .map(|item| item.confidence.max(confidence as f64))
            .unwrap_or(confidence as f64);
        let support_count = existing
            .as_ref()
            .map(|item| item.support_count.saturating_add(1))
            .unwrap_or(1);
        let exact_active_row = existing.as_ref().filter(|item| item.status == "active");
        if let Some(current) = exact_active_row {
            let content_changed = current.content != content;
            let previous_key = learned_user_memory_key(current);
            let previous_value = learned_user_memory_value(current);
            let previous_key_changed = previous_key
                .as_deref()
                .and_then(normalize_user_fact_key)
                .as_deref()
                != Some(key.as_str());
            if content_changed || previous_key_changed {
                if append_learned_user_memory_merged_phrasing(
                    &mut metadata,
                    previous_key.as_deref(),
                    previous_value.as_deref(),
                    Some(source),
                    &now,
                ) {
                    metadata.insert("secret_redacted".to_string(), serde_json::Value::Bool(true));
                }
            }
        }
        let build_memory_item =
            |embedding: Option<PgVector>| crate::storage::experience_item::Model {
                id: id.clone(),
                kind: normalized_kind.to_string(),
                scope: scope.to_string(),
                project_id: scoped_project_id.map(str::to_string),
                conversation_id: scoped_conversation_id.map(str::to_string),
                title: if normalized_kind == "constraint" {
                    "Learned operating constraint".to_string()
                } else {
                    "Learned user memory".to_string()
                },
                content: content.clone(),
                normalized_key: normalized_key.clone(),
                confidence: merged_confidence,
                support_count,
                contradiction_count: existing
                    .as_ref()
                    .map(|item| item.contradiction_count)
                    .unwrap_or_default(),
                status: "active".to_string(),
                metadata: serde_json::Value::Object(metadata.clone()),
                last_supported_at: Some(now.clone()),
                last_contradicted_at: existing
                    .as_ref()
                    .and_then(|item| item.last_contradicted_at.clone()),
                created_at: existing
                    .as_ref()
                    .map(|item| item.created_at.clone())
                    .unwrap_or_else(|| now.clone()),
                updated_at: now.clone(),
                embedding,
            };

        if let Some(current) = exact_active_row {
            let content_changed = current.content != content;
            let embedding = match self.embedding_client.as_deref() {
                Some(embedder) if content_changed || current.embedding.is_none() => {
                    let embed_text =
                        crate::core::memory_dedup::embeddable_text_from_content(&content)
                            .to_string();
                    match embedder.embed_texts(&[embed_text]).await {
                        Ok(mut embeddings) => embeddings.pop().or_else(|| {
                            if content_changed {
                                None
                            } else {
                                current.embedding.clone()
                            }
                        }),
                        Err(error) => {
                            tracing::warn!(
                                "Failed to refresh embedding for learned user memory '{}': {}",
                                key,
                                error
                            );
                            if content_changed {
                                None
                            } else {
                                current.embedding.clone()
                            }
                        }
                    }
                }
                _ => {
                    if content_changed {
                        None
                    } else {
                        current.embedding.clone()
                    }
                }
            };
            let memory_item = build_memory_item(embedding);
            if let Err(error) = self
                .storage
                .upsert_experience_item_txn(&txn, &memory_item)
                .await
            {
                tracing::warn!(
                    "Failed to capture lifecycle user memory '{}' into experience graph: {}",
                    key,
                    error
                );
                let _ = txn.rollback().await;
                return Err(error).with_context(|| {
                    format!(
                        "Failed to write learned user memory '{}' into the experience graph.",
                        key
                    )
                });
            }
            if let Err(error) = txn.commit().await {
                tracing::warn!(
                    "Failed to commit learned user memory update for '{}': {}",
                    key,
                    error
                );
                return Err(error).with_context(|| {
                    format!("Failed to commit learned user memory update for '{}'.", key)
                });
            }
            return Ok(id.clone());
        }

        let mut insert_embedding = existing.as_ref().and_then(|item| {
            if item.content == content {
                item.embedding.clone()
            } else {
                None
            }
        });
        if let Some(embedder) = self.embedding_client.as_deref() {
            let judge = crate::core::memory_dedup::LlmEquivalenceJudge::new(self.llm.clone());
            let candidate = crate::core::memory_dedup::MergeCandidate {
                kind: normalized_kind.to_string(),
                scope: scope.to_string(),
                project_id: scoped_project_id.map(str::to_string),
                conversation_id: scoped_conversation_id.map(str::to_string),
                content: content.clone(),
                suppressed_key: Some(key.clone()),
                suppressed_value: Some(value.clone()),
                confidence,
                metadata: metadata.clone(),
                source: Some(source.to_string()),
            };
            match crate::core::memory_dedup::attempt_absorb_into_canonical(
                &self.storage,
                &txn,
                embedder,
                &judge,
                &candidate,
            )
            .await
            {
                Ok(crate::core::memory_dedup::AbsorbOutcome::Absorbed { canonical_id, .. }) => {
                    if let Err(error) = txn.commit().await {
                        tracing::warn!(
                            "Failed to commit learned user memory absorb for '{}': {}",
                            key,
                            error
                        );
                        return Err(error).with_context(|| {
                            format!("Failed to commit learned user memory absorb for '{}'.", key)
                        });
                    }
                    return Ok(canonical_id);
                }
                Ok(crate::core::memory_dedup::AbsorbOutcome::Insert { embedding }) => {
                    insert_embedding = Some(embedding);
                }
                Err(error) => {
                    tracing::warn!(
                        "Semantic dedup failed for learned user memory '{}'; falling back to direct upsert: {}",
                        key,
                        error
                    );
                }
            }
        }

        let memory_item = build_memory_item(insert_embedding);
        if let Err(error) = self
            .storage
            .upsert_experience_item_txn(&txn, &memory_item)
            .await
        {
            tracing::warn!(
                "Failed to capture lifecycle user memory '{}' into experience graph: {}",
                key,
                error
            );
            let _ = txn.rollback().await;
            return Err(error).with_context(|| {
                format!(
                    "Failed to write learned user memory '{}' into the experience graph.",
                    key
                )
            });
        }
        if let Err(error) = txn.commit().await {
            tracing::warn!(
                "Failed to commit learned user memory insert for '{}': {}",
                key,
                error
            );
            return Err(error).with_context(|| {
                format!("Failed to commit learned user memory insert for '{}'.", key)
            });
        }
        Ok(id)
    }

    pub(super) async fn retract_learned_user_memory(
        &self,
        key: &str,
        kind: Option<&str>,
        scope: Option<&str>,
        channel: &str,
        conversation_id: Option<&str>,
        project_id: Option<&str>,
        reason: Option<&str>,
    ) -> Vec<String> {
        let Some(key) = normalize_user_fact_key(key) else {
            return Vec::new();
        };
        let requested_kind = kind
            .map(|value| normalize_self_memory_lookup_kind(Some(value)))
            .filter(|kind| !matches!(*kind, "any" | "other"));
        let explicit_scope = scope.is_some();
        let (requested_scope, requested_project_id, requested_conversation_id) =
            learned_user_memory_scope_ids(scope, project_id, conversation_id);
        let now = chrono::Utc::now();
        let mut lock_targets = if explicit_scope {
            vec![(
                requested_scope,
                requested_project_id,
                requested_conversation_id,
            )]
        } else {
            let mut targets = vec![("global", None, None)];
            if project_id.is_some() {
                targets.push(("project", project_id, None));
            }
            if conversation_id.is_some() {
                targets.push(("conversation", project_id, conversation_id));
            }
            targets
        };
        lock_targets.dedup();
        let Some((lock_scope, lock_project_id, lock_conversation_id)) =
            lock_targets.first().copied()
        else {
            return Vec::new();
        };
        let txn = match self
            .storage
            .begin_experience_memory_write_txn(
                "memory",
                lock_scope,
                lock_project_id,
                lock_conversation_id,
            )
            .await
        {
            Ok(txn) => txn,
            Err(error) => {
                tracing::warn!(
                    "Failed to start learned user memory retraction transaction for '{}' (scope={}): {}",
                    key,
                    requested_scope,
                    error
                );
                return Vec::new();
            }
        };
        for (extra_scope, extra_project_id, extra_conversation_id) in lock_targets.iter().skip(1) {
            if let Err(error) = self
                .storage
                .acquire_experience_memory_write_lock_txn(
                    &txn,
                    "memory",
                    extra_scope,
                    *extra_project_id,
                    *extra_conversation_id,
                )
                .await
            {
                tracing::warn!(
                    "Failed to extend learned user memory retraction lock for '{}' (scope={}): {}",
                    key,
                    extra_scope,
                    error
                );
                let _ = txn.rollback().await;
                return Vec::new();
            }
        }
        let active_items = self
            .storage
            .list_active_experience_items(
                &["constraint", "personal_fact"],
                project_id,
                conversation_id,
                64,
            )
            .await;
        let active_items = match active_items {
            Ok(items) => items,
            Err(error) => {
                tracing::warn!(
                    "Failed to load active learned user memories for retraction '{}': {}",
                    key,
                    error
                );
                let _ = txn.rollback().await;
                return Vec::new();
            }
        };
        let scoped_items = active_items
            .into_iter()
            .filter(|item| {
                if explicit_scope {
                    learned_user_memory_matches_exact_scope(
                        item,
                        requested_scope,
                        requested_project_id,
                        requested_conversation_id,
                    )
                } else {
                    true
                }
            })
            .collect::<Vec<_>>();
        let mut matches = scoped_items
            .iter()
            .filter(|item| {
                learned_user_memory_key(item).as_deref() == Some(key.as_str())
                    || learned_user_memory_merged_history_contains_key(item, &key)
            })
            .cloned()
            .collect::<Vec<_>>();
        if matches.is_empty() {
            if let Some(requested_kind) = requested_kind {
                let same_kind = scoped_items
                    .iter()
                    .filter(|item| learned_user_memory_lookup_kind(item) == requested_kind)
                    .cloned()
                    .collect::<Vec<_>>();
                if same_kind.len() == 1 {
                    matches = same_kind;
                }
            }
        }
        let mut retracted_ids = Vec::new();
        for item in matches {
            let mut updated = item.clone();
            let mut metadata = ambient_intent_metadata_object(&item);
            metadata.insert(
                "retracted_at".to_string(),
                serde_json::Value::String(now.to_rfc3339()),
            );
            metadata.insert(
                "retraction_source".to_string(),
                serde_json::Value::String(USER_LEARNED_MEMORY_RETRACTION_SOURCE.to_string()),
            );
            metadata.insert(
                "retraction_channel".to_string(),
                serde_json::Value::String(channel.to_string()),
            );
            if let Some(reason) = reason.and_then(|raw| normalize_user_memory_text(raw, 180)) {
                metadata.insert(
                    "retraction_reason".to_string(),
                    serde_json::Value::String(reason),
                );
            }
            updated.status = "retracted".to_string();
            updated.contradiction_count = updated.contradiction_count.saturating_add(1);
            updated.last_contradicted_at = Some(now.to_rfc3339());
            updated.updated_at = now.to_rfc3339();
            updated.metadata = serde_json::Value::Object(metadata);
            if let Err(error) = self
                .storage
                .upsert_experience_item_txn(&txn, &updated)
                .await
            {
                tracing::warn!(
                    "Failed to retract learned user memory '{}' from experience graph: {}",
                    key,
                    error
                );
                continue;
            }
            retracted_ids.push(updated.id.clone());
        }
        if let Err(error) = txn.commit().await {
            tracing::warn!(
                "Failed to commit learned user memory retraction for '{}': {}",
                key,
                error
            );
            return Vec::new();
        }
        retracted_ids
    }

    pub(super) fn user_selected_model_slot_id(&self) -> Option<String> {
        self.user_selected_model_slot_id
            .read()
            .ok()
            .and_then(|guard| guard.clone())
    }

    pub(super) fn base_url_looks_local(raw: &str) -> bool {
        let trimmed = raw.trim();
        if trimmed.is_empty() {
            return false;
        }
        if let Ok(parsed) = reqwest::Url::parse(trimmed) {
            return parsed
                .host_str()
                .map(crate::clients::host_looks_local_or_internal)
                .unwrap_or(false);
        }
        crate::clients::host_looks_local_or_internal(trimmed)
    }

    pub(super) fn provider_looks_local(provider: &crate::core::LlmProvider) -> bool {
        match provider {
            crate::core::LlmProvider::Ollama { .. } => true,
            crate::core::LlmProvider::OpenAI { base_url, .. } => base_url
                .as_deref()
                .map(Self::base_url_looks_local)
                .unwrap_or(false),
            crate::core::LlmProvider::Anthropic { .. } => false,
        }
    }

    pub(super) fn llm_candidate_uses_local_provider(
        &self,
        candidate: &LlmAttemptCandidate,
    ) -> bool {
        if candidate.slot_id == "legacy" {
            return Self::provider_looks_local(&self.config.llm);
        }
        self.config
            .model_pool
            .slots
            .iter()
            .find(|slot| slot.id == candidate.slot_id)
            .map(|slot| Self::provider_looks_local(&slot.provider))
            .unwrap_or(false)
    }

    pub(super) fn llm_candidates_for_role(
        &self,
        preferred_role: &ModelRole,
    ) -> Vec<LlmAttemptCandidate> {
        llm_attempt_candidates_for_role(
            &self.config,
            &self.model_pool,
            &self.primary_model_id,
            self.user_selected_model_slot_id().as_deref(),
            &self.llm,
            preferred_role,
        )
    }

    pub(super) fn user_memory_capture_llm_candidates(&self) -> Vec<LlmAttemptCandidate> {
        if !chat_model_is_configured(&self.config) {
            return Vec::new();
        }
        self.llm_candidates_for_role(&ModelRole::Fast)
    }

    pub(super) fn user_memory_capture_timeout_ms(&self, candidates: &[LlmAttemptCandidate]) -> u64 {
        if candidates.is_empty() {
            return USER_FACT_MEMORY_CAPTURE_LOCAL_TIMEOUT_MS;
        }
        if candidates
            .iter()
            .all(|candidate| self.llm_candidate_uses_local_provider(candidate))
        {
            USER_FACT_MEMORY_CAPTURE_LOCAL_TIMEOUT_MS
        } else {
            USER_FACT_MEMORY_CAPTURE_REMOTE_TIMEOUT_MS
        }
    }

    pub(super) async fn run_memory_capture_llm_candidate(
        &self,
        channel: &str,
        conversation_id: Option<&str>,
        request_kind: &str,
        prompt: &str,
        candidate: &LlmAttemptCandidate,
        timeout_ms: u64,
    ) -> Result<super::llm::LlmResponse> {
        self.run_user_memory_capture_json_candidate(
            channel,
            conversation_id,
            request_kind,
            "You extract lifecycle-aware user memory as strict JSON. Output JSON only.",
            prompt,
            candidate,
            timeout_ms,
        )
        .await
    }

    pub(super) async fn run_user_memory_capture_json_candidate(
        &self,
        channel: &str,
        conversation_id: Option<&str>,
        request_kind: &str,
        system_prompt: &str,
        prompt: &str,
        candidate: &LlmAttemptCandidate,
        timeout_ms: u64,
    ) -> Result<super::llm::LlmResponse> {
        let request = super::ExecutionRequest {
            kind: request_kind.to_string(),
            channel: Some(channel.to_string()),
            conversation_id: conversation_id.map(str::to_string),
            session_id: conversation_id.map(str::to_string),
            preferred_model_role: Some(
                Agent::model_role_label(&effective_model_role_for_selection(
                    &self.config,
                    &candidate.role,
                ))
                .to_string(),
            ),
            message_preview: Some(safe_truncate(prompt, 200)),
            ..Default::default()
        };
        let memories: [PromptMemory; 0] = [];
        let actions: [crate::actions::ActionDef; 0] = [];
        let response = if let Some(timeout_ms) = Some(timeout_ms.max(1)).filter(|value| *value > 0)
        {
            match tokio::time::timeout(
                std::time::Duration::from_millis(timeout_ms),
                candidate.client.chat_for_helper_request_limited(
                    system_prompt,
                    prompt,
                    &memories,
                    &actions,
                    &crate::security::ModelPrivacyConfig::default(),
                    USER_FACT_MEMORY_CAPTURE_ALLOW_SENSITIVE_CONTEXT,
                    Some(900),
                ),
            )
            .await
            {
                Ok(result) => result,
                Err(_) => {
                    return Err(anyhow::anyhow!(
                        "supervised_chat_failed(kind=timeout, request_kind={}, model={}): request timed out after {}ms",
                        request.kind,
                        candidate.client.model_name(),
                        timeout_ms
                    ));
                }
            }
        } else {
            candidate
                .client
                .chat_for_helper_request_limited(
                    system_prompt,
                    prompt,
                    &memories,
                    &actions,
                    &crate::security::ModelPrivacyConfig::default(),
                    USER_FACT_MEMORY_CAPTURE_ALLOW_SENSITIVE_CONTEXT,
                    Some(900),
                )
                .await
        };
        match response {
            Ok(resp) => {
                self.record_llm_usage(channel, request_kind, &resp).await;
                Ok(resp)
            }
            Err(error) => {
                let failure_kind = self
                    .execution_supervisor
                    .classify_failure(&error.to_string());
                tracing::debug!(
                    "User memory capture attempt failed on {} [{}]: {}",
                    candidate.slot_label,
                    candidate.client.model_name(),
                    error
                );
                Err(anyhow::anyhow!(
                    "supervised_chat_failed(kind={}, request_kind={}, model={}): {}",
                    failure_kind.as_str(),
                    request.kind,
                    candidate.client.model_name(),
                    error
                ))
            }
        }
    }

    pub(super) fn user_memory_capture_empty_escalation_candidates(
        &self,
        original_candidate: &LlmAttemptCandidate,
    ) -> Vec<LlmAttemptCandidate> {
        let mut ordered = Vec::new();
        let mut seen = HashSet::new();

        for candidate in self.llm_candidates_for_role(&ModelRole::Primary) {
            if seen.insert(candidate.slot_id.clone()) {
                ordered.push(candidate);
            }
            if ordered.len() >= USER_FACT_MEMORY_CAPTURE_EMPTY_ESCALATION_MAX_CANDIDATES.max(1) {
                return ordered;
            }
        }

        if seen.insert(original_candidate.slot_id.clone()) {
            ordered.push(original_candidate.clone());
        }
        if ordered.is_empty() {
            ordered.push(original_candidate.clone());
        }
        ordered.truncate(USER_FACT_MEMORY_CAPTURE_EMPTY_ESCALATION_MAX_CANDIDATES.max(1));
        ordered
    }

    pub(super) async fn recover_terminal_empty_user_memory_capture_payload(
        &self,
        channel: &str,
        conversation_id: Option<&str>,
        original_prompt: &str,
        response_shape: &str,
        previous_response_preview: &str,
        original_candidate: &LlmAttemptCandidate,
        attempts: &mut Vec<UserMemoryCaptureAttemptRecord>,
        timeout_ms: u64,
    ) -> Option<UserMemoryCaptureFocusedRecovery> {
        let verdict_prompt = build_user_memory_capture_empty_verdict_prompt(
            original_prompt,
            previous_response_preview,
        );

        for candidate in self.user_memory_capture_empty_escalation_candidates(original_candidate) {
            let role = Agent::model_role_label(&candidate.role).to_string();
            let verdict = match self
                .run_user_memory_capture_json_candidate(
                    channel,
                    conversation_id,
                    "user_fact_memory_capture_empty_verdict",
                    "You decide whether an empty user-memory extraction missed durable user memory. Return only the requested JSON.",
                    &verdict_prompt,
                    &candidate,
                    timeout_ms,
                )
                .await
            {
                Ok(resp) => resp,
                Err(error) => {
                    attempts.push(UserMemoryCaptureAttemptRecord {
                        slot_id: candidate.slot_id.clone(),
                        slot_label: candidate.slot_label.clone(),
                        role: role.clone(),
                        provider: None,
                        model: None,
                        stage: "empty_verdict".to_string(),
                        request_kind: "user_fact_memory_capture_empty_verdict".to_string(),
                        outcome: "transport_failed".to_string(),
                        error: Some(safe_truncate(&error.to_string(), 240)),
                    });
                    continue;
                }
            };

            let Some(parsed_verdict) = parse_user_memory_capture_empty_verdict(&verdict.content)
            else {
                attempts.push(UserMemoryCaptureAttemptRecord {
                    slot_id: candidate.slot_id.clone(),
                    slot_label: candidate.slot_label.clone(),
                    role: role.clone(),
                    provider: Some(verdict.provider.clone()),
                    model: Some(verdict.model.clone()),
                    stage: "empty_verdict".to_string(),
                    request_kind: "user_fact_memory_capture_empty_verdict".to_string(),
                    outcome: "invalid_verdict".to_string(),
                    error: Some("invalid_verdict".to_string()),
                });
                continue;
            };

            let durable_detected = parsed_verdict.has_durable_memory
                && parsed_verdict.confidence
                    >= USER_FACT_MEMORY_CAPTURE_EMPTY_VERDICT_MIN_CONFIDENCE;
            attempts.push(UserMemoryCaptureAttemptRecord {
                slot_id: candidate.slot_id.clone(),
                slot_label: candidate.slot_label.clone(),
                role: role.clone(),
                provider: Some(verdict.provider.clone()),
                model: Some(verdict.model.clone()),
                stage: "empty_verdict".to_string(),
                request_kind: "user_fact_memory_capture_empty_verdict".to_string(),
                outcome: if durable_detected {
                    "durable_memory_detected".to_string()
                } else if parsed_verdict.has_durable_memory {
                    "durable_memory_low_confidence".to_string()
                } else {
                    "confirmed_empty".to_string()
                },
                error: None,
            });
            if !durable_detected {
                continue;
            }

            let focused_recovery_prompt = build_user_memory_capture_focused_recovery_prompt(
                original_prompt,
                previous_response_preview,
                &parsed_verdict.reason,
                response_shape,
            );
            let focused_recovery = match self
                .run_memory_capture_llm_candidate(
                    channel,
                    conversation_id,
                    "user_fact_memory_capture_focused_recovery",
                    &focused_recovery_prompt,
                    &candidate,
                    timeout_ms,
                )
                .await
            {
                Ok(resp) => resp,
                Err(error) => {
                    attempts.push(UserMemoryCaptureAttemptRecord {
                        slot_id: candidate.slot_id.clone(),
                        slot_label: candidate.slot_label.clone(),
                        role: role.clone(),
                        provider: None,
                        model: None,
                        stage: "focused_recovery".to_string(),
                        request_kind: "user_fact_memory_capture_focused_recovery".to_string(),
                        outcome: "transport_failed".to_string(),
                        error: Some(safe_truncate(&error.to_string(), 240)),
                    });
                    continue;
                }
            };

            match parse_user_memory_capture_payload(&focused_recovery.content) {
                Ok(parsed) => {
                    let is_empty_decision =
                        user_memory_capture_payload_is_empty_decision(&parsed.payload);
                    attempts.push(UserMemoryCaptureAttemptRecord {
                        slot_id: candidate.slot_id.clone(),
                        slot_label: candidate.slot_label.clone(),
                        role: role.clone(),
                        provider: Some(focused_recovery.provider.clone()),
                        model: Some(focused_recovery.model.clone()),
                        stage: "focused_recovery".to_string(),
                        request_kind: "user_fact_memory_capture_focused_recovery".to_string(),
                        outcome: user_memory_capture_payload_ok_outcome(&parsed, is_empty_decision)
                            .to_string(),
                        error: None,
                    });
                    if !is_empty_decision {
                        return Some(UserMemoryCaptureFocusedRecovery {
                            payload: parsed.payload,
                            selected_slot_id: candidate.slot_id.clone(),
                            selected_slot_label: candidate.slot_label.clone(),
                            selected_provider: focused_recovery.provider.clone(),
                            selected_model: focused_recovery.model.clone(),
                            selected_stage: "focused_recovery".to_string(),
                        });
                    }
                }
                Err(error) => {
                    let outcome = match error {
                        UserMemoryCapturePayloadError::Unparseable => "unparseable",
                        UserMemoryCapturePayloadError::IncompleteShape => "incomplete_shape",
                    };
                    attempts.push(UserMemoryCaptureAttemptRecord {
                        slot_id: candidate.slot_id.clone(),
                        slot_label: candidate.slot_label.clone(),
                        role: role.clone(),
                        provider: Some(focused_recovery.provider.clone()),
                        model: Some(focused_recovery.model.clone()),
                        stage: "focused_recovery".to_string(),
                        request_kind: "user_fact_memory_capture_focused_recovery".to_string(),
                        outcome: outcome.to_string(),
                        error: Some(outcome.to_string()),
                    });
                    log_user_memory_capture_payload_error(
                        "memory capture focused recovery",
                        conversation_id,
                        &focused_recovery.content,
                        error,
                    );
                }
            }
        }

        None
    }

    pub(super) async fn run_memory_capture_with_schema_recovery(
        &self,
        channel: &str,
        conversation_id: Option<&str>,
        original_prompt: &str,
        response_shape: &str,
        candidates: Vec<LlmAttemptCandidate>,
        timeout_ms: u64,
    ) -> UserMemoryCaptureRunOutcome {
        let mut attempts = Vec::new();
        if candidates.is_empty() {
            return UserMemoryCaptureRunOutcome {
                payload: None,
                attempts,
                selected_slot_id: None,
                selected_slot_label: None,
                selected_provider: None,
                selected_model: None,
                selected_stage: None,
                terminal_error: Some("No memory capture candidates were available.".to_string()),
            };
        }

        let limited_candidates = candidates
            .iter()
            .take(USER_FACT_MEMORY_CAPTURE_MAX_CANDIDATES.max(1))
            .collect::<Vec<_>>();
        let mut attempted_candidates = 0usize;
        for (idx, candidate) in limited_candidates.iter().copied().enumerate() {
            attempted_candidates += 1;
            let role = Agent::model_role_label(&candidate.role).to_string();
            let resp = match self
                .run_memory_capture_llm_candidate(
                    channel,
                    conversation_id,
                    "user_fact_memory_capture",
                    original_prompt,
                    candidate,
                    timeout_ms,
                )
                .await
            {
                Ok(resp) => resp,
                Err(error) => {
                    attempts.push(UserMemoryCaptureAttemptRecord {
                        slot_id: candidate.slot_id.clone(),
                        slot_label: candidate.slot_label.clone(),
                        role: role.clone(),
                        provider: None,
                        model: None,
                        stage: "extract".to_string(),
                        request_kind: "user_fact_memory_capture".to_string(),
                        outcome: "transport_failed".to_string(),
                        error: Some(safe_truncate(&error.to_string(), 240)),
                    });
                    continue;
                }
            };

            match parse_user_memory_capture_payload(&resp.content) {
                Ok(parsed) => {
                    let is_empty_decision =
                        user_memory_capture_payload_is_empty_decision(&parsed.payload);
                    attempts.push(UserMemoryCaptureAttemptRecord {
                        slot_id: candidate.slot_id.clone(),
                        slot_label: candidate.slot_label.clone(),
                        role: role.clone(),
                        provider: Some(resp.provider.clone()),
                        model: Some(resp.model.clone()),
                        stage: "extract".to_string(),
                        request_kind: "user_fact_memory_capture".to_string(),
                        outcome: user_memory_capture_payload_ok_outcome(&parsed, is_empty_decision)
                            .to_string(),
                        error: None,
                    });
                    if !is_empty_decision {
                        return UserMemoryCaptureRunOutcome {
                            payload: Some(parsed.payload),
                            attempts,
                            selected_slot_id: Some(candidate.slot_id.clone()),
                            selected_slot_label: Some(candidate.slot_label.clone()),
                            selected_provider: Some(resp.provider.clone()),
                            selected_model: Some(resp.model.clone()),
                            selected_stage: Some("extract".to_string()),
                            terminal_error: None,
                        };
                    }

                    let empty_retry_prompt = build_user_memory_capture_empty_retry_prompt(
                        original_prompt,
                        &user_memory_capture_response_preview(&resp.content, 240),
                        response_shape,
                    );
                    let original_empty_payload = parsed.payload;
                    let empty_retry = match self
                        .run_memory_capture_llm_candidate(
                            channel,
                            conversation_id,
                            "user_fact_memory_capture_empty_retry",
                            &empty_retry_prompt,
                            candidate,
                            timeout_ms,
                        )
                        .await
                    {
                        Ok(resp) => resp,
                        Err(error) => {
                            attempts.push(UserMemoryCaptureAttemptRecord {
                                slot_id: candidate.slot_id.clone(),
                                slot_label: candidate.slot_label.clone(),
                                role: role.clone(),
                                provider: None,
                                model: None,
                                stage: "empty_retry".to_string(),
                                request_kind: "user_fact_memory_capture_empty_retry".to_string(),
                                outcome: "transport_failed".to_string(),
                                error: Some(safe_truncate(&error.to_string(), 240)),
                            });
                            if idx + 1 < limited_candidates.len() {
                                continue;
                            }
                            return UserMemoryCaptureRunOutcome {
                                payload: Some(original_empty_payload),
                                attempts,
                                selected_slot_id: Some(candidate.slot_id.clone()),
                                selected_slot_label: Some(candidate.slot_label.clone()),
                                selected_provider: Some(resp.provider.clone()),
                                selected_model: Some(resp.model.clone()),
                                selected_stage: Some("extract".to_string()),
                                terminal_error: None,
                            };
                        }
                    };

                    match parse_user_memory_capture_payload(&empty_retry.content) {
                        Ok(retried) => {
                            let retried_is_empty =
                                user_memory_capture_payload_is_empty_decision(&retried.payload);
                            let retry_next = retried_is_empty && idx + 1 < limited_candidates.len();
                            attempts.push(UserMemoryCaptureAttemptRecord {
                                slot_id: candidate.slot_id.clone(),
                                slot_label: candidate.slot_label.clone(),
                                role: role.clone(),
                                provider: Some(empty_retry.provider.clone()),
                                model: Some(empty_retry.model.clone()),
                                stage: "empty_retry".to_string(),
                                request_kind: "user_fact_memory_capture_empty_retry".to_string(),
                                outcome: if retry_next {
                                    "empty_decision_retry_next".to_string()
                                } else {
                                    user_memory_capture_payload_ok_outcome(
                                        &retried,
                                        retried_is_empty,
                                    )
                                    .to_string()
                                },
                                error: None,
                            });
                            if retry_next {
                                tracing::debug!(
                                    "memory capture empty retry still returned an empty decision for conversation {:?}; trying the next configured capture model",
                                    conversation_id
                                );
                                continue;
                            }
                            if retried_is_empty {
                                if let Some(recovered) = self
                                    .recover_terminal_empty_user_memory_capture_payload(
                                        channel,
                                        conversation_id,
                                        original_prompt,
                                        response_shape,
                                        &user_memory_capture_response_preview(
                                            &empty_retry.content,
                                            240,
                                        ),
                                        candidate,
                                        &mut attempts,
                                        timeout_ms,
                                    )
                                    .await
                                {
                                    return UserMemoryCaptureRunOutcome {
                                        payload: Some(recovered.payload),
                                        attempts,
                                        selected_slot_id: Some(recovered.selected_slot_id),
                                        selected_slot_label: Some(recovered.selected_slot_label),
                                        selected_provider: Some(recovered.selected_provider),
                                        selected_model: Some(recovered.selected_model),
                                        selected_stage: Some(recovered.selected_stage),
                                        terminal_error: None,
                                    };
                                }
                            }
                            return UserMemoryCaptureRunOutcome {
                                payload: Some(retried.payload),
                                attempts,
                                selected_slot_id: Some(candidate.slot_id.clone()),
                                selected_slot_label: Some(candidate.slot_label.clone()),
                                selected_provider: Some(empty_retry.provider.clone()),
                                selected_model: Some(empty_retry.model.clone()),
                                selected_stage: Some("empty_retry".to_string()),
                                terminal_error: None,
                            };
                        }
                        Err(error) => {
                            let outcome = match error {
                                UserMemoryCapturePayloadError::Unparseable => "unparseable",
                                UserMemoryCapturePayloadError::IncompleteShape => {
                                    "incomplete_shape"
                                }
                            };
                            attempts.push(UserMemoryCaptureAttemptRecord {
                                slot_id: candidate.slot_id.clone(),
                                slot_label: candidate.slot_label.clone(),
                                role: role.clone(),
                                provider: Some(empty_retry.provider.clone()),
                                model: Some(empty_retry.model.clone()),
                                stage: "empty_retry".to_string(),
                                request_kind: "user_fact_memory_capture_empty_retry".to_string(),
                                outcome: outcome.to_string(),
                                error: Some(outcome.to_string()),
                            });
                            log_user_memory_capture_payload_error(
                                "memory capture empty retry",
                                conversation_id,
                                &empty_retry.content,
                                error,
                            );
                            if idx + 1 < limited_candidates.len() {
                                continue;
                            }
                            return UserMemoryCaptureRunOutcome {
                                payload: Some(original_empty_payload),
                                attempts,
                                selected_slot_id: Some(candidate.slot_id.clone()),
                                selected_slot_label: Some(candidate.slot_label.clone()),
                                selected_provider: Some(resp.provider.clone()),
                                selected_model: Some(resp.model.clone()),
                                selected_stage: Some("extract".to_string()),
                                terminal_error: None,
                            };
                        }
                    }
                }
                Err(error) => {
                    let outcome = match error {
                        UserMemoryCapturePayloadError::Unparseable => "unparseable",
                        UserMemoryCapturePayloadError::IncompleteShape => "incomplete_shape",
                    };
                    attempts.push(UserMemoryCaptureAttemptRecord {
                        slot_id: candidate.slot_id.clone(),
                        slot_label: candidate.slot_label.clone(),
                        role: role.clone(),
                        provider: Some(resp.provider.clone()),
                        model: Some(resp.model.clone()),
                        stage: "extract".to_string(),
                        request_kind: "user_fact_memory_capture".to_string(),
                        outcome: outcome.to_string(),
                        error: Some(outcome.to_string()),
                    });
                    log_user_memory_capture_payload_error(
                        "memory capture",
                        conversation_id,
                        &resp.content,
                        error,
                    );
                }
            }

            let invalid_preview = user_memory_capture_response_preview(&resp.content, 240);
            let repair_prompt = build_user_memory_capture_repair_prompt(
                original_prompt,
                &invalid_preview,
                response_shape,
            );
            let repaired = match self
                .run_memory_capture_llm_candidate(
                    channel,
                    conversation_id,
                    "user_fact_memory_capture_schema_repair",
                    &repair_prompt,
                    candidate,
                    timeout_ms,
                )
                .await
            {
                Ok(resp) => resp,
                Err(error) => {
                    attempts.push(UserMemoryCaptureAttemptRecord {
                        slot_id: candidate.slot_id.clone(),
                        slot_label: candidate.slot_label.clone(),
                        role: role.clone(),
                        provider: None,
                        model: None,
                        stage: "schema_repair".to_string(),
                        request_kind: "user_fact_memory_capture_schema_repair".to_string(),
                        outcome: "transport_failed".to_string(),
                        error: Some(safe_truncate(&error.to_string(), 240)),
                    });
                    continue;
                }
            };

            match parse_user_memory_capture_payload(&repaired.content) {
                Ok(parsed) => {
                    let is_empty_decision =
                        user_memory_capture_payload_is_empty_decision(&parsed.payload);
                    attempts.push(UserMemoryCaptureAttemptRecord {
                        slot_id: candidate.slot_id.clone(),
                        slot_label: candidate.slot_label.clone(),
                        role: role.clone(),
                        provider: Some(repaired.provider.clone()),
                        model: Some(repaired.model.clone()),
                        stage: "schema_repair".to_string(),
                        request_kind: "user_fact_memory_capture_schema_repair".to_string(),
                        outcome: user_memory_capture_payload_ok_outcome(&parsed, is_empty_decision)
                            .to_string(),
                        error: None,
                    });
                    if !is_empty_decision {
                        return UserMemoryCaptureRunOutcome {
                            payload: Some(parsed.payload),
                            attempts,
                            selected_slot_id: Some(candidate.slot_id.clone()),
                            selected_slot_label: Some(candidate.slot_label.clone()),
                            selected_provider: Some(repaired.provider.clone()),
                            selected_model: Some(repaired.model.clone()),
                            selected_stage: Some("schema_repair".to_string()),
                            terminal_error: None,
                        };
                    }

                    let empty_retry_prompt = build_user_memory_capture_empty_retry_prompt(
                        original_prompt,
                        &user_memory_capture_response_preview(&repaired.content, 240),
                        response_shape,
                    );
                    let original_empty_payload = parsed.payload;
                    let empty_retry = match self
                        .run_memory_capture_llm_candidate(
                            channel,
                            conversation_id,
                            "user_fact_memory_capture_empty_retry",
                            &empty_retry_prompt,
                            candidate,
                            timeout_ms,
                        )
                        .await
                    {
                        Ok(resp) => resp,
                        Err(error) => {
                            attempts.push(UserMemoryCaptureAttemptRecord {
                                slot_id: candidate.slot_id.clone(),
                                slot_label: candidate.slot_label.clone(),
                                role: role.clone(),
                                provider: None,
                                model: None,
                                stage: "empty_retry".to_string(),
                                request_kind: "user_fact_memory_capture_empty_retry".to_string(),
                                outcome: "transport_failed".to_string(),
                                error: Some(safe_truncate(&error.to_string(), 240)),
                            });
                            if idx + 1 < limited_candidates.len() {
                                continue;
                            }
                            return UserMemoryCaptureRunOutcome {
                                payload: Some(original_empty_payload),
                                attempts,
                                selected_slot_id: Some(candidate.slot_id.clone()),
                                selected_slot_label: Some(candidate.slot_label.clone()),
                                selected_provider: Some(repaired.provider.clone()),
                                selected_model: Some(repaired.model.clone()),
                                selected_stage: Some("schema_repair".to_string()),
                                terminal_error: None,
                            };
                        }
                    };

                    match parse_user_memory_capture_payload(&empty_retry.content) {
                        Ok(retried) => {
                            let retried_is_empty =
                                user_memory_capture_payload_is_empty_decision(&retried.payload);
                            let retry_next = retried_is_empty && idx + 1 < limited_candidates.len();
                            attempts.push(UserMemoryCaptureAttemptRecord {
                                slot_id: candidate.slot_id.clone(),
                                slot_label: candidate.slot_label.clone(),
                                role: role.clone(),
                                provider: Some(empty_retry.provider.clone()),
                                model: Some(empty_retry.model.clone()),
                                stage: "empty_retry".to_string(),
                                request_kind: "user_fact_memory_capture_empty_retry".to_string(),
                                outcome: if retry_next {
                                    "empty_decision_retry_next".to_string()
                                } else {
                                    user_memory_capture_payload_ok_outcome(
                                        &retried,
                                        retried_is_empty,
                                    )
                                    .to_string()
                                },
                                error: None,
                            });
                            if retry_next {
                                tracing::debug!(
                                    "memory capture empty retry still returned an empty decision for conversation {:?}; trying the next configured capture model",
                                    conversation_id
                                );
                                continue;
                            }
                            if retried_is_empty {
                                if let Some(recovered) = self
                                    .recover_terminal_empty_user_memory_capture_payload(
                                        channel,
                                        conversation_id,
                                        original_prompt,
                                        response_shape,
                                        &user_memory_capture_response_preview(
                                            &empty_retry.content,
                                            240,
                                        ),
                                        candidate,
                                        &mut attempts,
                                        timeout_ms,
                                    )
                                    .await
                                {
                                    return UserMemoryCaptureRunOutcome {
                                        payload: Some(recovered.payload),
                                        attempts,
                                        selected_slot_id: Some(recovered.selected_slot_id),
                                        selected_slot_label: Some(recovered.selected_slot_label),
                                        selected_provider: Some(recovered.selected_provider),
                                        selected_model: Some(recovered.selected_model),
                                        selected_stage: Some(recovered.selected_stage),
                                        terminal_error: None,
                                    };
                                }
                            }
                            return UserMemoryCaptureRunOutcome {
                                payload: Some(retried.payload),
                                attempts,
                                selected_slot_id: Some(candidate.slot_id.clone()),
                                selected_slot_label: Some(candidate.slot_label.clone()),
                                selected_provider: Some(empty_retry.provider.clone()),
                                selected_model: Some(empty_retry.model.clone()),
                                selected_stage: Some("empty_retry".to_string()),
                                terminal_error: None,
                            };
                        }
                        Err(error) => {
                            let outcome = match error {
                                UserMemoryCapturePayloadError::Unparseable => "unparseable",
                                UserMemoryCapturePayloadError::IncompleteShape => {
                                    "incomplete_shape"
                                }
                            };
                            attempts.push(UserMemoryCaptureAttemptRecord {
                                slot_id: candidate.slot_id.clone(),
                                slot_label: candidate.slot_label.clone(),
                                role: role.clone(),
                                provider: Some(empty_retry.provider.clone()),
                                model: Some(empty_retry.model.clone()),
                                stage: "empty_retry".to_string(),
                                request_kind: "user_fact_memory_capture_empty_retry".to_string(),
                                outcome: outcome.to_string(),
                                error: Some(outcome.to_string()),
                            });
                            log_user_memory_capture_payload_error(
                                "memory capture empty retry",
                                conversation_id,
                                &empty_retry.content,
                                error,
                            );
                            if idx + 1 < limited_candidates.len() {
                                tracing::debug!(
                                    "memory capture schema repair recovered an empty decision for conversation {:?}; trying the next configured capture model",
                                    conversation_id
                                );
                                continue;
                            }
                            return UserMemoryCaptureRunOutcome {
                                payload: Some(original_empty_payload),
                                attempts,
                                selected_slot_id: Some(candidate.slot_id.clone()),
                                selected_slot_label: Some(candidate.slot_label.clone()),
                                selected_provider: Some(repaired.provider.clone()),
                                selected_model: Some(repaired.model.clone()),
                                selected_stage: Some("schema_repair".to_string()),
                                terminal_error: None,
                            };
                        }
                    }
                }
                Err(error) => {
                    let outcome = match error {
                        UserMemoryCapturePayloadError::Unparseable => "unparseable",
                        UserMemoryCapturePayloadError::IncompleteShape => "incomplete_shape",
                    };
                    attempts.push(UserMemoryCaptureAttemptRecord {
                        slot_id: candidate.slot_id.clone(),
                        slot_label: candidate.slot_label.clone(),
                        role,
                        provider: Some(repaired.provider.clone()),
                        model: Some(repaired.model.clone()),
                        stage: "schema_repair".to_string(),
                        request_kind: "user_fact_memory_capture_schema_repair".to_string(),
                        outcome: outcome.to_string(),
                        error: Some(outcome.to_string()),
                    });
                    log_user_memory_capture_payload_error(
                        "memory capture schema repair",
                        conversation_id,
                        &repaired.content,
                        error,
                    );
                }
            }
        }

        let terminal_error =
            user_memory_capture_terminal_error(&attempts, attempted_candidates, timeout_ms);
        if attempted_candidates > 0 {
            if user_memory_capture_attempts_all_transport_failed(&attempts) {
                tracing::warn!(
                    "memory capture transport failed for conversation {:?} after {} candidate(s): {}",
                    conversation_id,
                    attempted_candidates,
                    terminal_error
                );
            } else {
                tracing::warn!(
                    "memory capture failed to produce schema-compliant JSON for conversation {:?} after {} candidate(s)",
                    conversation_id,
                    attempted_candidates
                );
            }
        }
        UserMemoryCaptureRunOutcome {
            payload: None,
            attempts,
            selected_slot_id: None,
            selected_slot_label: None,
            selected_provider: None,
            selected_model: None,
            selected_stage: None,
            terminal_error: Some(terminal_error),
        }
    }

    pub(super) async fn record_llm_usage(
        &self,
        channel: &str,
        purpose: &str,
        resp: &crate::core::llm::LlmResponse,
    ) {
        let Some(usage) = resp.usage.as_ref() else {
            return;
        };
        let model = crate::storage::entities::llm_usage::Model {
            id: uuid::Uuid::new_v4().to_string(),
            created_at: chrono::Utc::now().to_rfc3339(),
            provider: resp.provider.clone(),
            model: resp.model.clone(),
            channel: channel.to_string(),
            purpose: purpose.to_string(),
            prompt_tokens: usage.prompt_tokens.min(i32::MAX as u64) as i32,
            completion_tokens: usage.completion_tokens.min(i32::MAX as u64) as i32,
            total_tokens: usage.total_tokens.min(i32::MAX as u64) as i32,
            estimated: usage.estimated,
            cost_usd: usage.cost_usd,
        };
        if let Err(error) = self.storage.insert_llm_usage(&model).await {
            tracing::debug!("Failed to record llm_usage: {}", error);
        }
    }
}

impl Agent {
    pub(super) async fn build_saved_user_facts_context(
        &self,
        project_id: Option<&str>,
        conversation_id: Option<&str>,
        current_message: &str,
    ) -> Option<String> {
        build_saved_user_facts_context_from_storage(
            &self.storage,
            self.embedding_client.as_deref(),
            project_id,
            conversation_id,
            current_message,
        )
        .await
    }

    pub(super) async fn mark_user_memory_capture_candidate(
        &self,
        message: &str,
        user_message_for_link_capture: &str,
        channel: &str,
        conversation_id: Option<&str>,
        project_id: Option<&str>,
        source_message_id: Option<&str>,
    ) -> bool {
        let trimmed = message.trim();
        if trimmed.is_empty() || source_message_id.unwrap_or_default().trim().is_empty() {
            return false;
        }
        let semantic_capture_key = ambient_stable_hash(&[
            channel.trim(),
            conversation_id.unwrap_or_default().trim(),
            project_id.unwrap_or_default().trim(),
            trimmed,
        ]);
        let pending_source_hash = format!("pending:{}", semantic_capture_key);
        let now_dt = chrono::Utc::now();
        match user_memory_capture_source_retry_state(&self.storage, &pending_source_hash, now_dt)
            .await
        {
            Ok(state) if state.previous_capture_count > 0 => return false,
            Ok(_) => {}
            Err(error) => {
                tracing::debug!(
                    "Failed to inspect pending memory capture source state: {}",
                    error
                );
                if self
                    .storage
                    .count_memory_capture_events_by_source_hash(&pending_source_hash)
                    .await
                    .unwrap_or(0)
                    > 0
                {
                    return false;
                }
            }
        }
        match user_memory_capture_source_retry_state(&self.storage, &semantic_capture_key, now_dt)
            .await
        {
            Ok(state) if state.blocks_retry => return false,
            Ok(_) => {}
            Err(error) => {
                tracing::debug!(
                    "Failed to inspect memory capture source retry state: {}",
                    error
                );
                if self
                    .storage
                    .count_memory_capture_events_by_source_hash(&semantic_capture_key)
                    .await
                    .unwrap_or(0)
                    > 0
                {
                    return false;
                }
            }
        }
        let now = chrono::Utc::now().to_rfc3339();
        let pending_event_id = format!("memory-capture-pending-{}", semantic_capture_key);
        let event = crate::storage::memory_capture_event::Model {
            id: pending_event_id,
            source_message_id: source_message_id.map(str::to_string),
            conversation_id: conversation_id.map(str::to_string),
            project_id: project_id.map(str::to_string),
            channel: channel.to_string(),
            status: USER_MEMORY_CAPTURE_PENDING_STATUS.to_string(),
            capture_kind: "user_fact_memory_capture".to_string(),
            source_hash: pending_source_hash,
            attempt_metadata: serde_json::json!({
                "schema_version": 1,
                "message_chars": trimmed.chars().count(),
                "semantic_capture_key": semantic_capture_key,
                "source": USER_LEARNED_MEMORY_CAPTURE_SOURCE,
                "deferred": true,
                "user_message_for_link_capture": safe_truncate(user_message_for_link_capture, 4000),
            }),
            error_history: serde_json::json!([]),
            replay_count: 0,
            next_retry_at: None,
            completed_at: None,
            created_at: now.clone(),
            updated_at: now,
        };
        if let Err(error) = self.storage.upsert_memory_capture_event(&event).await {
            tracing::warn!(
                "Failed to mark deferred memory capture candidate for conversation {:?}: {}",
                conversation_id,
                error
            );
            return false;
        }
        true
    }

    pub(super) fn kick_deferred_user_memory_capture_processing(&self) {
        USER_MEMORY_CAPTURE_DRAIN_WAKE_REQUESTED.store(true, std::sync::atomic::Ordering::Release);
        let semaphore = USER_MEMORY_CAPTURE_DRAIN_SEMAPHORE.clone();
        let Ok(permit) = semaphore.try_acquire_owned() else {
            return;
        };
        let agent = self.clone();
        crate::spawn_logged!(
            "src/core/agent/memory.rs:deferred_user_memory_capture_drain",
            async move {
                let handled = agent
                    .drain_deferred_user_memory_capture_candidates_unlocked()
                    .await;
                drop(permit);
                if USER_MEMORY_CAPTURE_DRAIN_WAKE_REQUESTED
                    .load(std::sync::atomic::Ordering::Acquire)
                {
                    agent.kick_deferred_user_memory_capture_processing();
                }
                if handled > 0 {
                    tracing::debug!(
                        handled,
                        "Deferred user memory capture drain handled queued candidate(s)"
                    );
                }
            }
        );
    }

    pub(super) async fn backfill_recent_user_memory_capture_candidates(&self) -> usize {
        let mut messages = match self
            .encrypted_storage
            .get_recent_user_messages_decrypted(USER_MEMORY_CAPTURE_STARTUP_BACKFILL_LIMIT)
            .await
        {
            Ok(messages) => messages,
            Err(error) => {
                tracing::debug!(
                    "Failed to load recent user messages for memory capture backfill: {}",
                    error
                );
                return 0;
            }
        };
        messages.reverse();
        let mut queued = 0usize;
        let mut conversation_channels: HashMap<String, String> = HashMap::new();
        for message in messages {
            let conversation_id = message.conversation_id.trim();
            if conversation_id.is_empty() || message.content.trim().is_empty() {
                continue;
            }
            let channel = match conversation_channels.get(conversation_id) {
                Some(channel) => channel.clone(),
                None => {
                    let channel = self
                        .storage
                        .get_conversation(conversation_id)
                        .await
                        .ok()
                        .flatten()
                        .map(|conversation| conversation.channel)
                        .map(|channel| channel.trim().to_string())
                        .filter(|channel| !channel.is_empty())
                        .unwrap_or_else(|| "chat".to_string());
                    conversation_channels.insert(conversation_id.to_string(), channel.clone());
                    channel
                }
            };
            if self
                .mark_user_memory_capture_candidate(
                    &message.content,
                    &message.content,
                    &channel,
                    Some(conversation_id),
                    None,
                    Some(&message.id),
                )
                .await
            {
                queued += 1;
            }
        }
        if queued > 0 {
            tracing::debug!(
                queued,
                "Backfilled recent user message(s) for deferred memory capture"
            );
        }
        queued
    }

    async fn pending_memory_capture_source_message(
        &self,
        event: &crate::storage::memory_capture_event::Model,
    ) -> Option<crate::storage::entities::message::Model> {
        let conversation_id = event.conversation_id.as_deref()?.trim();
        let source_message_id = event.source_message_id.as_deref()?.trim();
        if conversation_id.is_empty() || source_message_id.is_empty() {
            return None;
        }
        self.encrypted_storage
            .get_recent_messages_decrypted(conversation_id, 64)
            .await
            .ok()?
            .into_iter()
            .find(|message| {
                message.id == source_message_id && message.role.eq_ignore_ascii_case("user")
            })
    }

    pub(crate) async fn process_deferred_user_memory_capture_candidates(&self) -> usize {
        let semaphore = USER_MEMORY_CAPTURE_DRAIN_SEMAPHORE.clone();
        let permit = match semaphore.acquire_owned().await {
            Ok(permit) => permit,
            Err(_) => {
                tracing::debug!("Deferred memory capture drain limiter is closed");
                return 0;
            }
        };
        let handled = self
            .drain_deferred_user_memory_capture_candidates_unlocked()
            .await;
        drop(permit);
        if USER_MEMORY_CAPTURE_DRAIN_WAKE_REQUESTED.load(std::sync::atomic::Ordering::Acquire) {
            self.kick_deferred_user_memory_capture_processing();
        }
        handled
    }

    async fn drain_deferred_user_memory_capture_candidates_unlocked(&self) -> usize {
        let mut total_handled = 0usize;
        for batch_index in 0..USER_MEMORY_CAPTURE_DRAIN_MAX_BATCHES {
            USER_MEMORY_CAPTURE_DRAIN_WAKE_REQUESTED
                .store(false, std::sync::atomic::Ordering::Release);
            let handled = self
                .process_deferred_user_memory_capture_candidate_batch()
                .await;
            total_handled += handled;
            let wake_requested = USER_MEMORY_CAPTURE_DRAIN_WAKE_REQUESTED
                .swap(false, std::sync::atomic::Ordering::AcqRel);
            if handled < USER_MEMORY_CAPTURE_DEFERRED_BATCH_LIMIT as usize && !wake_requested {
                break;
            }
            if batch_index + 1 >= USER_MEMORY_CAPTURE_DRAIN_MAX_BATCHES {
                USER_MEMORY_CAPTURE_DRAIN_WAKE_REQUESTED
                    .store(true, std::sync::atomic::Ordering::Release);
                tracing::debug!(
                    max_batches = USER_MEMORY_CAPTURE_DRAIN_MAX_BATCHES,
                    total_handled,
                    "Deferred user memory capture drain yielded with more work likely pending"
                );
                break;
            }
        }
        total_handled
    }

    async fn process_deferred_user_memory_capture_candidate_batch(&self) -> usize {
        let recovered_stale =
            recover_stale_user_memory_capture_events(&self.storage, chrono::Utc::now()).await;
        if recovered_stale > 0 {
            tracing::debug!(
                recovered_stale,
                "Recovered stale user memory capture event(s) before draining deferred captures"
            );
        }
        let events = match self
            .storage
            .list_memory_capture_events_by_statuses_all_scopes(
                &[USER_MEMORY_CAPTURE_PENDING_STATUS],
                USER_MEMORY_CAPTURE_DEFERRED_BATCH_LIMIT,
            )
            .await
        {
            Ok(events) => events,
            Err(error) => {
                tracing::debug!(
                    "Failed to load deferred memory capture candidates: {}",
                    error
                );
                return 0;
            }
        };
        let worker = UserMemoryCaptureWorker::from_agent(self);
        let mut handled = recovered_stale;
        for mut event in events {
            let claim_now = chrono::Utc::now().to_rfc3339();
            match self
                .storage
                .try_claim_memory_capture_event_status(
                    &event.id,
                    USER_MEMORY_CAPTURE_PENDING_STATUS,
                    USER_MEMORY_CAPTURE_PROCESSING_DEFERRED_STATUS,
                    &claim_now,
                )
                .await
            {
                Ok(true) => {
                    event.status = USER_MEMORY_CAPTURE_PROCESSING_DEFERRED_STATUS.to_string();
                    event.updated_at = claim_now;
                    handled += 1;
                }
                Ok(false) => {
                    tracing::debug!(
                        capture_event_id = %event.id,
                        "Skipped deferred memory capture candidate because another worker claimed it first"
                    );
                    continue;
                }
                Err(error) => {
                    tracing::warn!(
                        capture_event_id = %event.id,
                        "Failed to claim deferred memory capture candidate: {}",
                        error
                    );
                    continue;
                }
            }

            let Some(source_message) = self.pending_memory_capture_source_message(&event).await
            else {
                event.status = USER_MEMORY_CAPTURE_FAILED_DEFERRED_STATUS.to_string();
                event.completed_at = Some(chrono::Utc::now().to_rfc3339());
                event.updated_at = chrono::Utc::now().to_rfc3339();
                event.error_history = serde_json::json!([user_memory_capture_error_entry(
                    "source_message_missing",
                    "Deferred memory capture could not find the source chat message.",
                )]);
                let _ = self.storage.upsert_memory_capture_event(&event).await;
                continue;
            };

            let user_message_for_link_capture = event
                .attempt_metadata
                .get("user_message_for_link_capture")
                .and_then(|value| value.as_str())
                .map(str::trim)
                .filter(|value| !value.is_empty())
                .unwrap_or(source_message.content.as_str());
            worker
                .capture_user_memory_hints(
                    &source_message.content,
                    user_message_for_link_capture,
                    event.channel.as_str(),
                    event.conversation_id.as_deref(),
                    event.project_id.as_deref(),
                    event.source_message_id.as_deref(),
                )
                .await;

            event.status = USER_MEMORY_CAPTURE_COMPLETED_DEFERRED_STATUS.to_string();
            event.completed_at = Some(chrono::Utc::now().to_rfc3339());
            event.updated_at = chrono::Utc::now().to_rfc3339();
            let _ = self.storage.upsert_memory_capture_event(&event).await;
        }
        handled
    }

    pub(crate) async fn apply_memory_operation_by_id_with_source(
        &self,
        operation_id: &str,
        apply_source: &str,
    ) -> Result<String> {
        UserMemoryCaptureWorker::from_agent(self)
            .apply_memory_operation_by_id_with_source(operation_id, apply_source)
            .await
    }

    pub(super) async fn build_ambient_time_context(&self) -> String {
        let now = chrono::Utc::now();
        let timezone = {
            let profile = self.user_profile.read().await;
            profile
                .timezone
                .as_deref()
                .map(str::trim)
                .filter(|value| !value.is_empty())
                .map(|value| value.to_string())
        };

        if let Some(timezone) = timezone {
            if let Ok(tz) = timezone.parse::<chrono_tz::Tz>() {
                return format!(
                    "UTC now: {}\nUser timezone: {}\nUser local now: {}",
                    now.to_rfc3339(),
                    timezone,
                    now.with_timezone(&tz).to_rfc3339()
                );
            }
            return format!(
                "UTC now: {}\nUser timezone setting could not be parsed: {}",
                now.to_rfc3339(),
                timezone
            );
        }

        format!("UTC now: {}\nUser timezone: not set", now.to_rfc3339())
    }

    pub(super) async fn ambient_intent_recent_dialogue(
        &self,
        item: &crate::storage::experience_item::Model,
    ) -> String {
        let Some(conversation_id) =
            ambient_metadata_text_field(item, "source_conversation_id", 120)
        else {
            return "(none)".to_string();
        };
        let messages = self
            .encrypted_storage
            .get_recent_messages_decrypted(&conversation_id, 8)
            .await
            .unwrap_or_default();
        if messages.is_empty() {
            return "(none)".to_string();
        }
        messages
            .iter()
            .filter(|message| !message.content.trim().is_empty())
            .map(|message| {
                format!(
                    "{}: {}",
                    message.role,
                    safe_truncate(message.content.trim(), 220)
                )
            })
            .collect::<Vec<_>>()
            .join("\n")
    }

    pub(super) fn apply_ambient_revisit_metadata(
        item: &crate::storage::experience_item::Model,
        trigger: &str,
        now: chrono::DateTime<chrono::Utc>,
        next_revisit_at: Option<chrono::DateTime<chrono::Utc>>,
        last_notified_at: Option<chrono::DateTime<chrono::Utc>>,
        status: &str,
    ) -> crate::storage::experience_item::Model {
        let mut updated = item.clone();
        let mut metadata = ambient_intent_metadata_object(item);
        let revisit_count = metadata
            .get("revisit_count")
            .and_then(|value| value.as_u64())
            .unwrap_or(0)
            .saturating_add(1);
        metadata.insert(
            "last_revisited_at".to_string(),
            serde_json::Value::String(now.to_rfc3339()),
        );
        metadata.insert(
            "last_revisit_trigger".to_string(),
            serde_json::Value::String(trigger.to_string()),
        );
        metadata.insert(
            "revisit_count".to_string(),
            serde_json::json!(revisit_count),
        );
        if let Some(next_revisit_at) = next_revisit_at {
            metadata.insert(
                "next_revisit_at".to_string(),
                serde_json::Value::String(next_revisit_at.to_rfc3339()),
            );
        }
        if let Some(last_notified_at) = last_notified_at {
            metadata.insert(
                "last_notified_at".to_string(),
                serde_json::Value::String(last_notified_at.to_rfc3339()),
            );
        }
        updated.status = status.to_string();
        updated.metadata = serde_json::Value::Object(metadata);
        updated.updated_at = now.to_rfc3339();
        updated
    }

    pub async fn revisit_ambient_intents(&self, trigger: &str) -> serde_json::Value {
        let now = chrono::Utc::now();
        let mut checked = 0usize;
        let mut deferred = 0usize;
        let mut notified = 0usize;
        let mut resolved = 0usize;
        let mut expired = 0usize;
        let mut errors = 0usize;

        let intents = match self
            .storage
            .list_active_experience_items(
                &[AMBIENT_INTENT_KIND],
                None,
                None,
                AMBIENT_INTENT_REVISIT_LIMIT,
            )
            .await
        {
            Ok(items) => items,
            Err(error) => {
                tracing::warn!("Failed to load ambient intents for revisit: {}", error);
                return serde_json::json!({
                    "status": "error",
                    "error": error.to_string(),
                });
            }
        };
        let loaded = intents.len();

        let learning_candidates = self.learning_llm_candidates().await;
        if learning_candidates.is_empty() {
            return serde_json::json!({
                "status": "skipped",
                "reason": "no_learning_model",
                "active": loaded,
            });
        }
        let time_context = self.build_ambient_time_context().await;
        let response_shape = r#"{"decision":"wait|check_in|resolve","message":"short user-facing note or empty","next_revisit_at":"RFC3339 UTC timestamp or null","reason":"brief internal reason"}"#;

        for item in intents {
            if ambient_intent_expired(&item, now) {
                let updated = Self::apply_ambient_revisit_metadata(
                    &item, trigger, now, None, None, "expired",
                );
                if let Err(error) = self.storage.upsert_experience_item(&updated).await {
                    errors = errors.saturating_add(1);
                    tracing::warn!("Failed to expire ambient intent '{}': {}", item.id, error);
                } else {
                    expired = expired.saturating_add(1);
                }
                continue;
            }
            if !ambient_intent_due(&item, now) {
                continue;
            }
            if checked >= AMBIENT_INTENT_MAX_REVISITS_PER_TICK {
                break;
            }
            checked = checked.saturating_add(1);

            let recent_dialogue = self.ambient_intent_recent_dialogue(&item).await;
            let prompt = format!(
                "Current time:\n{time_context}\n\nAmbient intent:\nid={id}\ntitle={title}\ncontent={content}\nnext_revisit_at={next_revisit_at}\nrevisit_condition={revisit_condition}\nresolution_condition={resolution_condition}\nexpires_at={expires_at}\n\nRecent dialogue from the source conversation:\n{recent_dialogue}\n\nReturn JSON only with this shape:\n{response_shape}\n\nRules:\n- Use semantic judgement, not keyword matching.\n- Choose wait when it is not yet useful to contact the user.\n- Choose check_in only when a short compassionate message would be useful now.\n- Choose resolve when the intent is complete, stale, obsolete, or recent dialogue indicates no further support is needed.\n- If you choose check_in, write the message directly to the user. Do not mention internal memory, Sentinel, proposals, scoring, or automation.\n- If you choose wait, provide the next useful background review time as next_revisit_at.\n- If you choose resolve, next_revisit_at may be null.",
                time_context = time_context,
                id = item.id.as_str(),
                title = safe_truncate(&item.title, 160),
                content = safe_truncate(&item.content, 600),
                next_revisit_at = ambient_metadata_text_field(&item, "next_revisit_at", 80)
                    .unwrap_or_else(|| "not set".to_string()),
                revisit_condition = ambient_metadata_text_field(&item, "revisit_condition", 260)
                    .unwrap_or_else(|| "not set".to_string()),
                resolution_condition =
                    ambient_metadata_text_field(&item, "resolution_condition", 260)
                        .unwrap_or_else(|| "not set".to_string()),
                expires_at = ambient_metadata_text_field(&item, "expires_at", 80)
                    .unwrap_or_else(|| "not set".to_string()),
                recent_dialogue = recent_dialogue,
                response_shape = response_shape
            );

            let Some(resp) = self
                .supervised_internal_chat(
                    "autonomy",
                    AMBIENT_INTENT_REVISIT_SOURCE,
                    AMBIENT_INTENT_REVISIT_SOURCE,
                    &ModelRole::Fast,
                    learning_candidates.clone(),
                    "You silently revisit ambient follow-up memories and return strict JSON only.",
                    &prompt,
                    &[],
                    &[],
                    internal_llm_timeout_ms("AGENTARK_AMBIENT_INTENT_REVISIT_TIMEOUT_MS", 20_000),
                    1,
                )
                .await
            else {
                errors = errors.saturating_add(1);
                continue;
            };
            let Some(payload) = extract_json_object_from_text(&resp.content) else {
                errors = errors.saturating_add(1);
                continue;
            };
            let decision = payload
                .get("decision")
                .and_then(|value| value.as_str())
                .map(|value| value.trim().to_ascii_lowercase())
                .unwrap_or_else(|| "wait".to_string());
            let next_revisit_at = ambient_json_datetime_field(&payload, "next_revisit_at")
                .filter(|dt| *dt > now)
                .unwrap_or_else(|| {
                    now + chrono::Duration::hours(AMBIENT_INTENT_FALLBACK_RECHECK_HOURS)
                });

            match decision.as_str() {
                "resolve" => {
                    let updated = Self::apply_ambient_revisit_metadata(
                        &item, trigger, now, None, None, "resolved",
                    );
                    if let Err(error) = self.storage.upsert_experience_item(&updated).await {
                        errors = errors.saturating_add(1);
                        tracing::warn!("Failed to resolve ambient intent '{}': {}", item.id, error);
                    } else {
                        resolved = resolved.saturating_add(1);
                    }
                }
                "check_in" => {
                    if let Some(message) = ambient_json_text_field(&payload, "message", 500) {
                        self.emit_notification(
                            "Checking in",
                            &message,
                            "info",
                            AMBIENT_INTENT_REVISIT_SOURCE,
                        )
                        .await;
                        self.notify_preferred_channel(&message).await;
                        let updated = Self::apply_ambient_revisit_metadata(
                            &item,
                            trigger,
                            now,
                            Some(next_revisit_at),
                            Some(now),
                            "active",
                        );
                        if let Err(error) = self.storage.upsert_experience_item(&updated).await {
                            errors = errors.saturating_add(1);
                            tracing::warn!(
                                "Failed to update ambient intent '{}' after check-in: {}",
                                item.id,
                                error
                            );
                        } else {
                            notified = notified.saturating_add(1);
                        }
                    } else {
                        let updated = Self::apply_ambient_revisit_metadata(
                            &item,
                            trigger,
                            now,
                            Some(next_revisit_at),
                            None,
                            "active",
                        );
                        if let Err(error) = self.storage.upsert_experience_item(&updated).await {
                            errors = errors.saturating_add(1);
                            tracing::warn!(
                                "Failed to defer ambient intent '{}': {}",
                                item.id,
                                error
                            );
                        } else {
                            deferred = deferred.saturating_add(1);
                        }
                    }
                }
                _ => {
                    let updated = Self::apply_ambient_revisit_metadata(
                        &item,
                        trigger,
                        now,
                        Some(next_revisit_at),
                        None,
                        "active",
                    );
                    if let Err(error) = self.storage.upsert_experience_item(&updated).await {
                        errors = errors.saturating_add(1);
                        tracing::warn!("Failed to defer ambient intent '{}': {}", item.id, error);
                    } else {
                        deferred = deferred.saturating_add(1);
                    }
                }
            }
        }

        serde_json::json!({
            "status": "ok",
            "active": loaded,
            "checked": checked,
            "deferred": deferred,
            "notified": notified,
            "resolved": resolved,
            "expired": expired,
            "errors": errors,
        })
    }
}
