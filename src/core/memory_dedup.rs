//! Semantic dedup for learned user memories (`personal_fact` / `constraint`
//! experience-item rows).
//!
//! This module is intent-based by design. It avoids keyword lists, synonym
//! tables, and surface-phrase matching. Similarity comes from vector
//! embeddings; borderline cases are resolved by asking an LLM whether two
//! memories express the same intent about the same subject — not by matching
//! predicted wording.
//!
//! The one entry point is [`attempt_absorb_into_canonical`] for the write
//! path: given a candidate memory that's about to be inserted, either absorb
//! it into an existing canonical row (no new row written) or return the
//! computed embedding so the caller can insert a fresh row with the vector
//! populated. No periodic backfill is run — on a fresh database all writes
//! flow through this path, and the existing background candidate-generation
//! merger stays in place as a safety net for anything this path misses.

use anyhow::Result;
use async_trait::async_trait;
use sea_orm::entity::prelude::PgVector;
use sea_orm::DatabaseTransaction;
use serde_json::{Map as JsonMap, Value as JsonValue};

use crate::storage::{experience_item, Storage};

use super::embeddings::EmbeddingClient;

const MERGED_MEMORY_OPAQUE_TOKEN_MIN_CHARS: usize = 20;
const MERGED_MEMORY_OPAQUE_TOKEN_ENTROPY_BITS_PER_CHAR: f64 = 3.5;
const MERGED_MEMORY_REDACTION_MARKERS: &[&str] = &[
    "[REDACTED_SECRET]",
    "[REDACTED_API_KEY]",
    "[REDACTED_PRIVATE_KEY]",
    "[REDACTED_CERTIFICATE]",
];
const EQUIVALENCE_JUDGE_TIMEOUT_SECS: u64 = 8;
const EQUIVALENCE_JUDGE_MAX_OUTPUT_TOKENS: u32 = 256;

/// Historical auto-accept threshold kept visible for tuning, but production
/// merge decisions still require an equivalence verdict. Embeddings retrieve
/// candidates; they do not by themselves prove polarity and subject match.
#[allow(dead_code)]
pub const AUTO_ACCEPT_COSINE_SIM: f64 = 0.94;

/// Cosine similarity below this value is treated as definitely not the same
/// intent — the candidate is inserted as a fresh row without consulting the
/// LLM.
pub const NO_MERGE_COSINE_SIM: f64 = 0.82;

/// Historical threshold used by tests and diagnostics. Production merge
/// available. Above this → merge, below → keep distinct. Kept conservative
/// decisions treat an unavailable or uncertain judge as "do not merge".
#[allow(dead_code)]
pub const DEFAULT_MERGE_THRESHOLD: f64 = 0.88;

/// Ring-buffer cap on the per-row `metadata.merged_phrasings` list. The
/// canonical row's metadata must not grow without bound as duplicate
/// phrasings fold into it.
pub const MAX_MERGED_PHRASINGS: usize = 12;

/// Verdict returned by a [`SemanticEquivalenceJudge`].
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum EquivalenceVerdict {
    /// Both memories clearly express the same intent about the same subject.
    Equivalent,
    /// The memories differ in subject, polarity, or specificity enough that
    /// they should remain distinct.
    NotEquivalent,
    /// The judge is unavailable or could not decide. Callers should treat
    /// this as "do not merge" to preserve correctness.
    Uncertain,
}

/// Pluggable LLM equivalence check for candidates at or above
/// [`NO_MERGE_COSINE_SIM`].
///
/// Implementations must be intent-based: they should ask the model whether
/// two stored memories would be interchangeable from the user's perspective,
/// not whether they share literal phrasing or keywords.
#[async_trait]
pub trait SemanticEquivalenceJudge: Send + Sync {
    async fn judge(&self, a: &str, b: &str) -> EquivalenceVerdict;
}

#[async_trait]
pub trait MemoryEmbedder: Send + Sync {
    async fn embed_texts(&self, texts: &[String]) -> Result<Vec<PgVector>>;
}

#[async_trait]
impl MemoryEmbedder for EmbeddingClient {
    async fn embed_texts(&self, texts: &[String]) -> Result<Vec<PgVector>> {
        EmbeddingClient::embed_texts(self, texts).await
    }
}

/// A judge that always returns [`EquivalenceVerdict::Uncertain`]. Used in
/// tests or when no chat model is configured; forces the merge logic to keep
/// candidates distinct unless a concrete judge returns an equivalent verdict.
#[allow(dead_code)]
pub struct UncertainJudge;

#[async_trait]
impl SemanticEquivalenceJudge for UncertainJudge {
    async fn judge(&self, _a: &str, _b: &str) -> EquivalenceVerdict {
        EquivalenceVerdict::Uncertain
    }
}

/// A judge backed by a shared [`LlmClient`]. Every borderline similarity
/// pair triggers one short LLM call asking whether the two memories are the
/// same intent about the same subject. No keyword or phrase rules are
/// applied on the Rust side — the LLM is the intent adjudicator.
pub struct LlmEquivalenceJudge {
    llm: super::llm::LlmClient,
}

impl LlmEquivalenceJudge {
    pub fn new(llm: super::llm::LlmClient) -> Self {
        Self { llm }
    }
}

#[async_trait]
impl SemanticEquivalenceJudge for LlmEquivalenceJudge {
    async fn judge(&self, a: &str, b: &str) -> EquivalenceVerdict {
        let system = "You are a strict semantic-equivalence checker for stored user memories. \
            You only answer with the required JSON object.";
        let user = build_equivalence_prompt(a, b);
        match tokio::time::timeout(
            std::time::Duration::from_secs(EQUIVALENCE_JUDGE_TIMEOUT_SECS),
            self.llm
                .chat_classifier_bounded(system, &user, EQUIVALENCE_JUDGE_MAX_OUTPUT_TOKENS),
        )
        .await
        {
            Ok(Ok(response)) => parse_equivalence_response(&response.content),
            Ok(Err(error)) => {
                tracing::debug!(
                    "memory_dedup: LLM equivalence judge unavailable; treating as uncertain: {}",
                    error
                );
                EquivalenceVerdict::Uncertain
            }
            Err(_) => {
                tracing::debug!(
                    "memory_dedup: LLM equivalence judge timed out after {}s; treating as uncertain",
                    EQUIVALENCE_JUDGE_TIMEOUT_SECS
                );
                EquivalenceVerdict::Uncertain
            }
        }
    }
}

/// Payload describing a memory the caller is about to insert. Carries enough
/// context to locate existing near-duplicates via the scope tuple and to
/// record the suppressed phrasing if absorbed.
#[derive(Debug, Clone)]
#[allow(dead_code)]
pub struct MergeCandidate {
    pub kind: String,
    pub scope: String,
    pub project_id: Option<String>,
    pub conversation_id: Option<String>,
    /// Content string used for embedding and textual comparison. Callers
    /// should use the same `content` they would otherwise persist in the
    /// experience item.
    pub content: String,
    /// Original LLM-emitted key for the suppressed memory. Stored inside
    /// `canonical.metadata.merged_phrasings[]` for audit when absorbed.
    pub suppressed_key: Option<String>,
    /// Original LLM-emitted value (or equivalent natural-language text)
    /// for the suppressed memory.
    pub suppressed_value: Option<String>,
    /// Confidence the caller would have written on a fresh row. Used to
    /// raise the canonical row's confidence when absorbing.
    pub confidence: f32,
    /// Structured metadata the caller would have persisted on a fresh row.
    /// When the candidate is absorbed into an existing canonical row, these
    /// fields overwrite the canonical metadata so the row reflects the newest
    /// source, durability, scope, validity windows, and reason.
    pub metadata: JsonMap<String, JsonValue>,
    /// Source tag (e.g. `"user_memory_capture"`) stored alongside the
    /// merged phrasing for traceability.
    pub source: Option<String>,
}

/// Result of a write-time absorb attempt. Fields flagged dead while the
/// semantic-merge chain is not yet wired from `upsert_learned_user_memory`.
#[derive(Debug, Clone)]
#[allow(dead_code)]
pub enum AbsorbOutcome {
    /// The candidate was folded into an existing canonical row. No new row
    /// was written; the caller should stop the insert.
    Absorbed {
        canonical_id: String,
        cosine_similarity: f64,
    },
    /// No near-duplicate exists. The caller should proceed with its insert
    /// and persist the provided embedding on the new row.
    Insert { embedding: PgVector },
}

fn similarity_from_distance(cosine_distance: f64) -> f64 {
    1.0 - cosine_distance
}

/// Strip the legacy `{key}: ` slug prefix from an experience-item `content`
/// string before embedding. The key slug is noise when the extractor emits
/// three different slugs for the same intent; embedding only the natural
/// language part gives the embedder a cleaner semantic signal.
pub(crate) fn embeddable_text_from_content(content: &str) -> &str {
    let trimmed = content.trim_start();
    // Slug prefixes are lowercase ASCII + underscores followed by ": ".
    if let Some(idx) = trimmed.find(": ") {
        let head = &trimmed[..idx];
        if !head.is_empty()
            && head
                .chars()
                .all(|ch| ch.is_ascii_lowercase() || ch.is_ascii_digit() || ch == '_')
        {
            return trimmed[idx + 2..].trim();
        }
    }
    trimmed
}

fn merged_memory_token_shape_char(ch: char) -> bool {
    ch.is_ascii_alphanumeric() || matches!(ch, '_' | '-' | '/' | '=' | '+' | '.')
}

fn merged_memory_shannon_entropy_bits_per_char(value: &str) -> f64 {
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

fn merged_memory_is_opaque_token_shape(value: &str) -> bool {
    let trimmed = value.trim();
    trimmed.chars().count() >= MERGED_MEMORY_OPAQUE_TOKEN_MIN_CHARS
        && !trimmed.chars().any(char::is_whitespace)
        && trimmed.chars().all(merged_memory_token_shape_char)
        && merged_memory_shannon_entropy_bits_per_char(trimmed)
            >= MERGED_MEMORY_OPAQUE_TOKEN_ENTROPY_BITS_PER_CHAR
}

fn merged_memory_contains_redaction_marker(value: &str) -> bool {
    MERGED_MEMORY_REDACTION_MARKERS
        .iter()
        .any(|marker| value.contains(marker))
}

fn merged_memory_is_mostly_redaction_marker_payload(value: &str) -> bool {
    if !merged_memory_contains_redaction_marker(value) {
        return false;
    }
    let mut stripped = value.to_string();
    for marker in MERGED_MEMORY_REDACTION_MARKERS {
        stripped = stripped.replace(marker, " ");
    }
    let meaningful: String = stripped
        .chars()
        .filter(|ch| ch.is_alphanumeric() || ch.is_whitespace())
        .collect();
    meaningful.trim().chars().count() < 24
}

fn redact_merged_memory_opaque_tokens(raw: &str) -> Option<String> {
    if merged_memory_is_opaque_token_shape(raw) {
        return None;
    }

    let mut redacted = String::with_capacity(raw.len());
    let mut last = 0usize;
    let mut run_start: Option<usize> = None;
    let mut redacted_any = false;

    for (idx, ch) in raw.char_indices() {
        if merged_memory_token_shape_char(ch) {
            if run_start.is_none() {
                run_start = Some(idx);
            }
            continue;
        }
        if let Some(start) = run_start.take() {
            let candidate = &raw[start..idx];
            if merged_memory_is_opaque_token_shape(candidate) {
                redacted.push_str(&raw[last..start]);
                redacted.push_str("[REDACTED_SECRET]");
                last = idx;
                redacted_any = true;
            }
        }
    }

    if let Some(start) = run_start {
        let candidate = &raw[start..];
        if merged_memory_is_opaque_token_shape(candidate) {
            redacted.push_str(&raw[last..start]);
            redacted.push_str("[REDACTED_SECRET]");
            last = raw.len();
            redacted_any = true;
        }
    }

    if redacted_any {
        redacted.push_str(&raw[last..]);
        Some(redacted)
    } else {
        Some(raw.to_string())
    }
}

fn sanitize_merged_phrasing_value_for_storage(raw: &str) -> Option<String> {
    let trimmed = raw.trim();
    if trimmed.is_empty() {
        return None;
    }
    let structurally_redacted = redact_merged_memory_opaque_tokens(trimmed)?;
    if merged_memory_is_mostly_redaction_marker_payload(&structurally_redacted) {
        return None;
    }
    let redaction = crate::security::redact_secret_input(&structurally_redacted);
    let value = if redaction.had_secret() {
        if redaction.is_mostly_secret_payload() {
            return None;
        }
        redaction.text.trim().to_string()
    } else {
        structurally_redacted
    };
    let value = value.trim();
    if merged_memory_is_mostly_redaction_marker_payload(value) {
        return None;
    }
    if value.is_empty() {
        None
    } else {
        Some(value.chars().take(320).collect())
    }
}

/// Append a suppressed phrasing entry to the canonical row's metadata,
/// keeping the list capped at [`MAX_MERGED_PHRASINGS`] (drop oldest).
fn append_merged_phrasing(
    metadata: &mut JsonMap<String, JsonValue>,
    suppressed_key: Option<&str>,
    suppressed_value: Option<&str>,
    source: Option<&str>,
    now_iso: &str,
) {
    let entry = {
        let mut map = JsonMap::new();
        if let Some(key) = suppressed_key.filter(|value| !value.trim().is_empty()) {
            map.insert("key".to_string(), JsonValue::String(key.to_string()));
        }
        if let Some(value) = suppressed_value.and_then(sanitize_merged_phrasing_value_for_storage) {
            map.insert("value".to_string(), JsonValue::String(value));
        }
        if let Some(source) = source.filter(|value| !value.trim().is_empty()) {
            map.insert("source".to_string(), JsonValue::String(source.to_string()));
        }
        map.insert("at".to_string(), JsonValue::String(now_iso.to_string()));
        JsonValue::Object(map)
    };

    let list = metadata
        .entry("merged_phrasings".to_string())
        .or_insert_with(|| JsonValue::Array(Vec::new()));
    let JsonValue::Array(items) = list else {
        *list = JsonValue::Array(vec![entry]);
        return;
    };
    items.push(entry);
    while items.len() > MAX_MERGED_PHRASINGS {
        items.remove(0);
    }
}

/// Prompt used for the LLM equivalence check. Phrased around the *user
/// attribute or subject* each memory is about — not around whether the two
/// memories convey identical information. This is deliberate: when the user
/// updates or refines a stored fact ("I live in Kolkata" → "I live in
/// Madhyam, Kolkata"), both memories describe the same attribute and the new
/// one should supersede the old. Holding out for strict informational
/// equivalence would force an update to be stored as a separate row, which is
/// the exact bug we are fixing.
///
/// The judge should return true whenever both memories describe the same
/// user attribute / same subject and could reasonably unify into a single
/// canonical row — even if one is a refinement, correction, or newer version
/// of the other. It should return false only when the subjects differ, or
/// when the two memories directly contradict each other in a way that must
/// be preserved as distinct rows (e.g. two mutually-incompatible facts that
/// the user appears to assert simultaneously, rather than an update).
pub fn build_equivalence_prompt(a: &str, b: &str) -> String {
    format!(
        "You are checking whether an existing stored user memory and a newer candidate user memory describe the same user attribute or subject so that the newer candidate should unify with or supersede the existing canonical record.\n\n\
         Existing memory A:\n{a}\n\n\
         Newer candidate memory B:\n{b}\n\n\
         Answer with a single JSON object of the form {{\"equivalent\": true|false, \"rationale\": \"...\"}}.\n\
         Rules:\n\
         - Return true when both memories are about the same user attribute, preference, identity, location, workflow, or relationship — even when one refines, corrects, or updates the other, and even when one is more specific than the other. The point is whether they belong on the same canonical row.\n\
         - Return true for paraphrases, clarifications, and value updates of the same underlying fact, including later polarity changes for the same preference or attribute.\n\
         - Return false when the subjects or attributes differ (e.g. the user's name vs the user's location vs the user's employer).\n\
         - Return false when the two memories directly negate each other in a way the user is asserting at the same time rather than as an update.\n\
         - Decide on meaning, not on shared words. Different wording about the same attribute is still the same attribute; similar wording about different attributes is not.\n\
         - If unsure, return false."
    )
}

/// Parse an LLM response into an [`EquivalenceVerdict`]. The response is
/// expected to contain a JSON object with an `equivalent` boolean. Anything
/// else — including missing JSON, malformed JSON, or a non-boolean field —
/// collapses to [`EquivalenceVerdict::Uncertain`] so the caller stays
/// conservative.
pub fn parse_equivalence_response(text: &str) -> EquivalenceVerdict {
    let Some(json) = extract_json_object(text) else {
        return EquivalenceVerdict::Uncertain;
    };
    match json.get("equivalent").and_then(|value| value.as_bool()) {
        Some(true) => EquivalenceVerdict::Equivalent,
        Some(false) => EquivalenceVerdict::NotEquivalent,
        None => EquivalenceVerdict::Uncertain,
    }
}

fn extract_json_object(text: &str) -> Option<JsonValue> {
    // Accept either a direct JSON object or a JSON object embedded in prose.
    let trimmed = text.trim();
    if let Ok(value) = serde_json::from_str::<JsonValue>(trimmed) {
        return Some(value);
    }
    let start = trimmed.find('{')?;
    let end = trimmed.rfind('}')?;
    if end <= start {
        return None;
    }
    serde_json::from_str::<JsonValue>(&trimmed[start..=end]).ok()
}

/// Fold the incoming candidate into the canonical row. The newer candidate
/// wins on content so that updates like "I live in Kolkata" → "I live in
/// Madhyam, Kolkata" land on the canonical row instead of being silently
/// dropped. The previous content is preserved inside
/// `metadata.merged_phrasings[]` (ring-buffered) so the audit trail still
/// shows what was superseded.
async fn absorb_candidate_into_row(
    storage: &Storage,
    txn: &DatabaseTransaction,
    canonical: &experience_item::Model,
    candidate: &MergeCandidate,
    candidate_embedding: PgVector,
) -> Result<()> {
    let mut updated = canonical.clone();
    let mut metadata = updated.metadata.as_object().cloned().unwrap_or_default();
    let now_iso = chrono::Utc::now().to_rfc3339();

    let previous_value_text = embeddable_text_from_content(&canonical.content).to_string();
    let previous_key = metadata
        .get("key")
        .and_then(|value| value.as_str())
        .map(|value| value.to_string());
    let previous_source = metadata
        .get("source")
        .and_then(|value| value.as_str())
        .map(|value| value.to_string());
    append_merged_phrasing(
        &mut metadata,
        previous_key.as_deref(),
        Some(&previous_value_text),
        previous_source.as_deref(),
        &now_iso,
    );

    for (field, value) in &candidate.metadata {
        metadata.insert(field.clone(), value.clone());
    }

    updated.content = candidate.content.clone();
    updated.metadata = JsonValue::Object(metadata);
    updated.support_count = updated.support_count.saturating_add(1);
    let candidate_confidence = candidate.confidence.clamp(0.0, 1.0) as f64;
    if candidate_confidence > updated.confidence {
        updated.confidence = candidate_confidence.min(0.99);
    }
    updated.last_supported_at = Some(now_iso.clone());
    updated.updated_at = now_iso;
    updated.embedding = Some(candidate_embedding);
    storage.upsert_experience_item_txn(txn, &updated).await?;
    Ok(())
}

/// Write-time dedup entry point. Computes an embedding for `candidate`,
/// looks up near-duplicates scoped to the same kind / scope / project /
/// conversation, and either absorbs the candidate into a canonical row or
/// returns the embedding for the caller to persist on a fresh row.
///
/// This is the function the agent's write path calls **after** the exact
/// `(key, durability, scope)` fast-path has already been checked. Do not use
/// this for structurally-keyed upserts where the same row id already
/// resolves to a live canonical — that path should short-circuit to a direct
/// upsert without paying the embedding cost.
pub async fn attempt_absorb_into_canonical(
    storage: &Storage,
    txn: &DatabaseTransaction,
    embedder: &dyn MemoryEmbedder,
    judge: &dyn SemanticEquivalenceJudge,
    candidate: &MergeCandidate,
) -> Result<AbsorbOutcome> {
    let embed_text = embeddable_text_from_content(&candidate.content);
    let mut embeddings = embedder.embed_texts(&[embed_text.to_string()]).await?;
    let Some(embedding) = embeddings.pop() else {
        anyhow::bail!("Embedding client returned no vectors for candidate content");
    };

    let neighbours = storage
        .nearest_active_experience_items_semantic_txn(
            txn,
            &[candidate.kind.as_str()],
            &candidate.scope,
            candidate.project_id.as_deref(),
            candidate.conversation_id.as_deref(),
            &embedding,
            3,
        )
        .await?;

    for (existing, distance) in neighbours {
        let similarity = similarity_from_distance(distance);
        let decision = decide_merge_with_judge(
            embeddable_text_from_content(&existing.content),
            embed_text,
            similarity,
            judge,
        )
        .await;
        if !decision {
            continue;
        }
        absorb_candidate_into_row(storage, txn, &existing, candidate, embedding.clone()).await?;
        return Ok(AbsorbOutcome::Absorbed {
            canonical_id: existing.id,
            cosine_similarity: similarity,
        });
    }

    Ok(AbsorbOutcome::Insert { embedding })
}

async fn decide_merge_with_judge(
    canonical_text: &str,
    candidate_text: &str,
    similarity: f64,
    judge: &dyn SemanticEquivalenceJudge,
) -> bool {
    if similarity < NO_MERGE_COSINE_SIM {
        return false;
    }
    match judge.judge(canonical_text, candidate_text).await {
        EquivalenceVerdict::Equivalent => true,
        EquivalenceVerdict::NotEquivalent => false,
        EquivalenceVerdict::Uncertain => false,
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn similarity_from_distance_inverts_cosine_distance() {
        assert!((similarity_from_distance(0.0) - 1.0).abs() < 1e-9);
        assert!((similarity_from_distance(1.0) - 0.0).abs() < 1e-9);
        assert!((similarity_from_distance(2.0) - (-1.0)).abs() < 1e-9);
    }

    #[test]
    fn embeddable_text_strips_legacy_slug_prefix() {
        let stripped = embeddable_text_from_content(
            "requested_linear_integration: User wants to install the Linear integration.",
        );
        assert_eq!(stripped, "User wants to install the Linear integration.");
    }

    #[test]
    fn embeddable_text_leaves_plain_content_alone() {
        let content = "The user lives in Kolkata, India.";
        assert_eq!(embeddable_text_from_content(content), content);
    }

    #[test]
    fn merged_phrasings_caps_at_maximum() {
        let mut metadata = JsonMap::new();
        for i in 0..(MAX_MERGED_PHRASINGS + 5) {
            append_merged_phrasing(
                &mut metadata,
                Some(&format!("key_{i}")),
                Some(&format!("value_{i}")),
                Some("test"),
                "2026-04-17T00:00:00Z",
            );
        }
        let items = metadata
            .get("merged_phrasings")
            .and_then(|value| value.as_array())
            .expect("merged_phrasings list present");
        assert_eq!(items.len(), MAX_MERGED_PHRASINGS);
        let first = items
            .first()
            .and_then(|item| item.get("key"))
            .and_then(|value| value.as_str())
            .expect("first entry has key");
        assert_eq!(
            first,
            format!("key_{}", 5).as_str(),
            "oldest entries should have been dropped"
        );
    }

    #[test]
    fn merged_phrasings_drop_whole_opaque_token_values() {
        assert_eq!(
            sanitize_merged_phrasing_value_for_storage("A7fK2mQ9zP4xR8tV1bN6yD3sL0h"),
            None
        );
    }

    #[test]
    fn merged_phrasings_drop_marker_only_values() {
        assert_eq!(
            sanitize_merged_phrasing_value_for_storage("[REDACTED_SECRET]"),
            None
        );
    }

    #[test]
    fn merged_phrasings_redact_opaque_token_substrings() {
        let value = sanitize_merged_phrasing_value_for_storage(
            "Use production workspace with A7fK2mQ9zP4xR8tV1bN6yD3sL0h removed.",
        )
        .expect("mixed phrasing should retain useful prose");

        assert!(value.contains("[REDACTED_SECRET]"));
        assert!(!value.contains("A7fK2mQ9zP4xR8tV1bN6yD3sL0h"));
        assert!(value.contains("production workspace"));
    }

    #[test]
    fn parse_equivalence_response_recognises_bare_json() {
        assert_eq!(
            parse_equivalence_response(r#"{"equivalent": true, "rationale": "same subject"}"#),
            EquivalenceVerdict::Equivalent
        );
        assert_eq!(
            parse_equivalence_response(r#"{"equivalent": false}"#),
            EquivalenceVerdict::NotEquivalent
        );
    }

    #[test]
    fn parse_equivalence_response_extracts_embedded_json() {
        let response = "Sure, here is my verdict:\n{\"equivalent\": true}\n-- end";
        assert_eq!(
            parse_equivalence_response(response),
            EquivalenceVerdict::Equivalent
        );
    }

    #[test]
    fn parse_equivalence_response_defaults_to_uncertain() {
        assert_eq!(
            parse_equivalence_response("definitely the same"),
            EquivalenceVerdict::Uncertain
        );
        assert_eq!(
            parse_equivalence_response("{\"equivalent\": \"maybe\"}"),
            EquivalenceVerdict::Uncertain
        );
    }

    #[test]
    fn equivalence_prompt_treats_later_preference_polarity_as_update() {
        let prompt = build_equivalence_prompt(
            "The user prefers one interface theme.",
            "The user now prefers a different interface theme.",
        );

        assert!(prompt.contains("same user attribute or subject"));
        assert!(prompt.contains("newer candidate"));
        assert!(prompt.contains("later polarity changes"));
        assert!(prompt.contains("same preference or attribute"));
    }

    #[tokio::test]
    async fn decide_merge_does_not_merge_when_judge_is_uncertain() {
        let judge = UncertainJudge;
        let decision = decide_merge_with_judge(
            "The user lives in Kolkata, India.",
            "The user lives in Kolkata, India (same subject rewording).",
            0.95,
            &judge,
        )
        .await;
        assert!(
            !decision,
            "high cosine similarity must not merge without an equivalence verdict"
        );
    }

    #[tokio::test]
    async fn decide_merge_rejects_below_floor() {
        let judge = UncertainJudge;
        let decision = decide_merge_with_judge(
            "The user lives in Kolkata, India.",
            "The user is employed at Ignite Data.",
            0.70,
            &judge,
        )
        .await;
        assert!(!decision, "cosine below no-merge floor must not merge");
    }

    #[tokio::test]
    async fn decide_merge_uses_judge_in_gray_zone() {
        struct AlwaysNo;
        #[async_trait]
        impl SemanticEquivalenceJudge for AlwaysNo {
            async fn judge(&self, _a: &str, _b: &str) -> EquivalenceVerdict {
                EquivalenceVerdict::NotEquivalent
            }
        }
        struct AlwaysYes;
        #[async_trait]
        impl SemanticEquivalenceJudge for AlwaysYes {
            async fn judge(&self, _a: &str, _b: &str) -> EquivalenceVerdict {
                EquivalenceVerdict::Equivalent
            }
        }
        let no = AlwaysNo;
        let yes = AlwaysYes;
        let gray = 0.87;
        assert!(
            !decide_merge_with_judge("left", "right", gray, &no).await,
            "gray-zone merge must respect a NotEquivalent verdict"
        );
        assert!(
            decide_merge_with_judge("left", "right", gray, &yes).await,
            "gray-zone merge must respect an Equivalent verdict"
        );
    }
}
