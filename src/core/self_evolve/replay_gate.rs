//! Automatic replay/evidence gate for reviewable Evolve candidates.
//!
//! This gate deliberately relies on structured runtime evidence instead of user
//! phrasing: candidate type, evidence references, run outcome state, correction
//! state, procedural-pattern counters, memory-item counters, and privacy
//! redaction results.

use anyhow::Result;
use serde::Serialize;
use serde_json::Value;
use std::collections::HashSet;

use crate::storage::{
    experience_item, experience_run, learning_candidate, procedural_pattern, Storage,
};

use super::router_learning::{
    validate_router_learning_candidate, RouterLearningCandidatePayload,
    ROUTER_LEARNING_CANDIDATE_TYPE,
};
use super::routing_canonical_evolution::{
    parse_routing_canonical_candidate, ROUTING_CANONICAL_CANDIDATE_TYPE,
};

const DEFAULT_MIN_EVIDENCE_SAMPLES: usize = 2;
const DEFAULT_MIN_CONFIDENCE: f64 = 0.35;
const DEFAULT_MAX_CORRECTION_RATE: f64 = 0.45;

#[derive(Debug, Clone, Serialize)]
pub struct CandidateReplayGateResult {
    pub candidate_id: String,
    pub candidate_type: String,
    pub status: String,
    pub allow_approval: bool,
    pub reason: String,
    pub evidence_refs_checked: usize,
    pub samples: usize,
    pub safe_run_samples: usize,
    pub excluded_sensitive_runs: usize,
    pub pii_redacted_runs: usize,
    pub success_rate: f64,
    pub correction_rate: f64,
    pub confidence: f64,
    pub support_score: f64,
    pub min_samples: usize,
    pub min_confidence: f64,
    pub min_support_score: f64,
    pub max_correction_rate: f64,
    pub usage_samples: usize,
    pub avg_input_tokens: f64,
    pub avg_output_tokens: f64,
    pub avg_total_tokens: f64,
    pub max_total_tokens: i64,
    pub avg_cost_usd: f64,
    pub signals: Vec<String>,
}

#[derive(Debug, Default)]
struct EvidenceBundle {
    refs_checked: usize,
    samples: usize,
    successes: usize,
    corrections: usize,
    safe_run_samples: usize,
    excluded_sensitive_runs: usize,
    pii_redacted_runs: usize,
    item_count: usize,
    pattern_count: usize,
    run_count: usize,
    turn_decision_run_count: usize,
    usage: TurnUsageEvidence,
    signals: Vec<String>,
    items: Vec<experience_item::Model>,
    patterns: Vec<procedural_pattern::Model>,
    runs: Vec<experience_run::Model>,
}

#[derive(Debug, Default)]
struct TurnUsageEvidence {
    samples: usize,
    input_tokens: i64,
    output_tokens: i64,
    total_tokens: i64,
    max_total_tokens: i64,
    cost_usd: f64,
}

impl TurnUsageEvidence {
    fn add(&mut self, value: &Value) {
        let input_tokens = json_i64_field(value, "input_tokens")
            .unwrap_or_default()
            .max(0);
        let output_tokens = json_i64_field(value, "output_tokens")
            .unwrap_or_default()
            .max(0);
        let total_tokens = json_i64_field(value, "total_tokens")
            .unwrap_or_else(|| input_tokens.saturating_add(output_tokens))
            .max(0);
        let cost_usd = value
            .get("cost_usd")
            .and_then(json_f64)
            .unwrap_or_default()
            .max(0.0);
        self.samples = self.samples.saturating_add(1);
        self.input_tokens = self.input_tokens.saturating_add(input_tokens);
        self.output_tokens = self.output_tokens.saturating_add(output_tokens);
        self.total_tokens = self.total_tokens.saturating_add(total_tokens);
        self.max_total_tokens = self.max_total_tokens.max(total_tokens);
        self.cost_usd += cost_usd;
    }

    fn avg_input_tokens(&self) -> f64 {
        ratio_i64(self.input_tokens, self.samples)
    }

    fn avg_output_tokens(&self) -> f64 {
        ratio_i64(self.output_tokens, self.samples)
    }

    fn avg_total_tokens(&self) -> f64 {
        ratio_i64(self.total_tokens, self.samples)
    }

    fn avg_cost_usd(&self) -> f64 {
        if self.samples == 0 {
            0.0
        } else {
            self.cost_usd / self.samples as f64
        }
    }
}

impl EvidenceBundle {
    fn add_item(&mut self, item: experience_item::Model) {
        let support = item.support_count.max(0) as usize;
        let contradictions = item.contradiction_count.max(0) as usize;
        let samples = support.saturating_add(contradictions);
        self.samples = self.samples.saturating_add(samples);
        self.successes = self.successes.saturating_add(support);
        self.corrections = self.corrections.saturating_add(contradictions);
        self.item_count += 1;
        self.signals.push(format!(
            "experience item evidence: kind={}, status={}, support={}, contradictions={}",
            item.kind, item.status, support, contradictions
        ));
        self.items.push(item);
    }

    fn add_pattern(&mut self, pattern: procedural_pattern::Model) {
        let samples = pattern.sample_count.max(0) as usize;
        let successes = pattern.success_count.max(0) as usize;
        let corrections = pattern.correction_count.max(0) as usize;
        self.samples = self.samples.saturating_add(samples);
        self.successes = self.successes.saturating_add(successes);
        self.corrections = self.corrections.saturating_add(corrections);
        self.pattern_count += 1;
        self.signals.push(format!(
            "procedural pattern evidence: status={}, samples={}, success_rate={:.2}",
            pattern.status, samples, pattern.success_rate
        ));
        self.patterns.push(pattern);
    }

    fn add_run(&mut self, run: experience_run::Model) {
        self.run_count += 1;
        if !experience_run_privacy_safe(&run, self) {
            self.excluded_sensitive_runs += 1;
            return;
        }
        if !experience_run_resolved(&run) {
            self.signals
                .push("experience run evidence skipped: unresolved outcome".to_string());
            return;
        }
        self.samples += 1;
        self.safe_run_samples += 1;
        if experience_run_success(&run) {
            self.successes += 1;
        }
        if experience_run_corrected(&run) {
            self.corrections += 1;
        }
        if let Some(turn_decision) = run.metadata.get("turn_decision") {
            self.turn_decision_run_count += 1;
            let path = turn_decision
                .get("path")
                .and_then(|value| value.as_str())
                .unwrap_or("unknown");
            let task_type = turn_decision
                .get("task_type")
                .and_then(|value| value.as_str())
                .unwrap_or("unknown");
            let usage_delta = turn_decision.get("usage_delta");
            if let Some(usage_delta) = usage_delta {
                self.usage.add(usage_delta);
            }
            let total_tokens = usage_delta
                .and_then(|value| json_i64_field(value, "total_tokens"))
                .unwrap_or_default();
            let cost_usd = usage_delta
                .and_then(|value| value.get("cost_usd"))
                .and_then(json_f64)
                .unwrap_or_default();
            self.signals.push(format!(
                "turn decision evidence: path={}, task_type={}, total_tokens={}, cost_usd={:.6}",
                path, task_type, total_tokens, cost_usd
            ));
        }
        self.signals.push(format!(
            "experience run evidence: success_state={}, correction_state={}",
            run.success_state, run.correction_state
        ));
        self.runs.push(run);
    }

    fn success_rate(&self) -> f64 {
        ratio(self.successes, self.samples)
    }

    fn correction_rate(&self) -> f64 {
        ratio(self.corrections, self.samples)
    }
}

pub async fn evaluate_candidate_replay_gate(
    storage: &Storage,
    candidate: &learning_candidate::Model,
) -> Result<CandidateReplayGateResult> {
    let evidence = collect_candidate_evidence(storage, candidate).await?;
    Ok(evaluate_candidate_with_evidence(candidate, evidence))
}

async fn collect_candidate_evidence(
    storage: &Storage,
    candidate: &learning_candidate::Model,
) -> Result<EvidenceBundle> {
    let mut evidence = EvidenceBundle::default();
    let mut refs = evidence_ref_strings(&candidate.evidence_refs);
    if let Some(pattern_id) = candidate
        .pattern_id
        .as_deref()
        .map(str::trim)
        .filter(|value| !value.is_empty())
    {
        refs.push(pattern_id.to_string());
    }
    collect_candidate_content_refs(candidate, &mut refs);
    refs.sort();
    refs.dedup();

    let mut seen_items = HashSet::new();
    let mut seen_patterns = HashSet::new();
    let mut seen_runs = HashSet::new();

    for reference in refs {
        evidence.refs_checked += 1;
        if seen_items.insert(reference.clone()) {
            if let Some(item) = storage.get_experience_item(&reference).await? {
                collect_source_run_from_item(storage, &item, &mut evidence, &mut seen_runs).await?;
                evidence.add_item(item);
                continue;
            }
        }

        if seen_patterns.insert(reference.clone()) {
            if let Some(pattern) = storage.get_procedural_pattern(&reference).await? {
                collect_source_item_from_pattern(
                    storage,
                    &pattern,
                    &mut evidence,
                    &mut seen_items,
                    &mut seen_runs,
                )
                .await?;
                evidence.add_pattern(pattern);
                continue;
            }
        }

        if seen_runs.insert(reference.clone()) {
            if let Some(run) = storage.get_experience_run(&reference).await? {
                evidence.add_run(run);
            }
        }
    }

    Ok(evidence)
}

async fn collect_source_item_from_pattern(
    storage: &Storage,
    pattern: &procedural_pattern::Model,
    evidence: &mut EvidenceBundle,
    seen_items: &mut HashSet<String>,
    seen_runs: &mut HashSet<String>,
) -> Result<()> {
    let Some(item_id) = pattern
        .metadata
        .get("source_item_id")
        .and_then(|value| value.as_str())
        .map(str::trim)
        .filter(|value| !value.is_empty())
    else {
        return Ok(());
    };
    if !seen_items.insert(item_id.to_string()) {
        return Ok(());
    }
    if let Some(item) = storage.get_experience_item(item_id).await? {
        collect_source_run_from_item(storage, &item, evidence, seen_runs).await?;
        evidence.add_item(item);
    }
    Ok(())
}

async fn collect_source_run_from_item(
    storage: &Storage,
    item: &experience_item::Model,
    evidence: &mut EvidenceBundle,
    seen_runs: &mut HashSet<String>,
) -> Result<()> {
    let Some(run_id) = item
        .metadata
        .get("source_run_id")
        .and_then(|value| value.as_str())
        .map(str::trim)
        .filter(|value| !value.is_empty())
    else {
        return Ok(());
    };
    if !seen_runs.insert(run_id.to_string()) {
        return Ok(());
    }
    if let Some(run) = storage.get_experience_run(run_id).await? {
        evidence.add_run(run);
    }
    Ok(())
}

fn evaluate_candidate_with_evidence(
    candidate: &learning_candidate::Model,
    evidence: EvidenceBundle,
) -> CandidateReplayGateResult {
    let confidence = candidate.confidence.clamp(0.0, 1.0);
    match candidate.candidate_type.as_str() {
        "memory_deprecate" => evaluate_memory_deprecate(candidate, evidence, confidence),
        "memory_merge" => evaluate_memory_merge(candidate, evidence, confidence),
        "memory_add" | "memory_update" | "memory_retract" => {
            evaluate_memory_operation(candidate, evidence, confidence)
        }
        "skill_patch" => gate_result(
            candidate,
            evidence,
            confidence,
            "blocked",
            false,
            "Evolve learning does not create or modify skills".to_string(),
        ),
        "strategy" | "workflow" => evaluate_structured_candidate(candidate, evidence, confidence),
        "turn_decision" | "turn_routing_policy" | "routing_policy" => {
            evaluate_turn_decision_candidate(candidate, evidence, confidence)
        }
        ROUTING_CANONICAL_CANDIDATE_TYPE => {
            evaluate_routing_canonical_candidate(candidate, evidence, confidence)
        }
        ROUTER_LEARNING_CANDIDATE_TYPE => {
            evaluate_router_learning_candidate(candidate, evidence, confidence)
        }
        other => gate_result(
            candidate,
            evidence,
            confidence,
            "blocked",
            false,
            format!(
                "candidate type '{}' is not covered by the replay gate",
                other
            ),
        ),
    }
}

fn evaluate_router_learning_candidate(
    candidate: &learning_candidate::Model,
    evidence: EvidenceBundle,
    confidence: f64,
) -> CandidateReplayGateResult {
    let payload = serde_json::from_value::<RouterLearningCandidatePayload>(
        candidate.proposed_content.clone(),
    );
    let payload = match payload {
        Ok(payload) => payload,
        Err(error) => {
            return gate_result(
                candidate,
                evidence,
                confidence,
                "blocked",
                false,
                format!("router learning candidate payload is invalid: {error}"),
            );
        }
    };
    if let Err(error) = validate_router_learning_candidate(&payload) {
        return gate_result(
            candidate,
            evidence,
            confidence,
            "blocked",
            false,
            format!("router learning candidate failed validation: {error}"),
        );
    }
    evaluate_turn_decision_candidate(candidate, evidence, confidence)
}

fn evaluate_routing_canonical_candidate(
    candidate: &learning_candidate::Model,
    evidence: EvidenceBundle,
    confidence: f64,
) -> CandidateReplayGateResult {
    if let Err(error) = parse_routing_canonical_candidate(candidate) {
        return gate_result(
            candidate,
            evidence,
            confidence,
            "blocked",
            false,
            format!("routing canonical candidate payload is invalid: {error}"),
        );
    }
    evaluate_turn_decision_candidate(candidate, evidence, confidence)
}

fn evaluate_turn_decision_candidate(
    candidate: &learning_candidate::Model,
    evidence: EvidenceBundle,
    confidence: f64,
) -> CandidateReplayGateResult {
    let turn_decision_samples = evidence.turn_decision_run_count;
    if turn_decision_samples < DEFAULT_MIN_EVIDENCE_SAMPLES {
        return gate_result(
            candidate,
            evidence,
            confidence,
            "needs_more_data",
            false,
            "not enough typed turn-decision evidence for approval".to_string(),
        );
    }
    if confidence < DEFAULT_MIN_CONFIDENCE {
        return gate_result(
            candidate,
            evidence,
            confidence,
            "blocked",
            false,
            "turn-decision candidate confidence is below replay-gate threshold".to_string(),
        );
    }
    if evidence.correction_rate() > DEFAULT_MAX_CORRECTION_RATE {
        return gate_result(
            candidate,
            evidence,
            confidence,
            "blocked",
            false,
            "correction rate in turn-decision evidence is too high".to_string(),
        );
    }
    if evidence.successes == 0 {
        return gate_result(
            candidate,
            evidence,
            confidence,
            "blocked",
            false,
            "turn-decision evidence has no accepted successful run".to_string(),
        );
    }
    gate_result(
        candidate,
        evidence,
        confidence,
        "passed",
        true,
        "typed turn-decision evidence supports approval".to_string(),
    )
}

fn evaluate_structured_candidate(
    candidate: &learning_candidate::Model,
    evidence: EvidenceBundle,
    confidence: f64,
) -> CandidateReplayGateResult {
    if evidence.samples < DEFAULT_MIN_EVIDENCE_SAMPLES {
        return gate_result(
            candidate,
            evidence,
            confidence,
            "needs_more_data",
            false,
            "not enough structured replay evidence for approval".to_string(),
        );
    }
    if confidence < DEFAULT_MIN_CONFIDENCE {
        return gate_result(
            candidate,
            evidence,
            confidence,
            "blocked",
            false,
            "candidate confidence is below replay-gate threshold".to_string(),
        );
    }
    if evidence.correction_rate() > DEFAULT_MAX_CORRECTION_RATE {
        return gate_result(
            candidate,
            evidence,
            confidence,
            "blocked",
            false,
            "correction rate in supporting evidence is too high".to_string(),
        );
    }
    if evidence.successes == 0 {
        return gate_result(
            candidate,
            evidence,
            confidence,
            "blocked",
            false,
            "structured evidence has no accepted successful run".to_string(),
        );
    }
    gate_result(
        candidate,
        evidence,
        confidence,
        "passed",
        true,
        "structured replay evidence supports approval".to_string(),
    )
}

fn evaluate_memory_operation(
    candidate: &learning_candidate::Model,
    evidence: EvidenceBundle,
    confidence: f64,
) -> CandidateReplayGateResult {
    let operation_id = candidate
        .proposed_content
        .get("operation_id")
        .and_then(|value| value.as_str())
        .map(str::trim)
        .filter(|value| !value.is_empty());
    if operation_id.is_none() {
        return gate_result(
            candidate,
            evidence,
            confidence,
            "blocked",
            false,
            "memory operation candidate is missing operation_id".to_string(),
        );
    }
    let looks_sensitive = candidate
        .proposed_content
        .get("looks_sensitive")
        .and_then(|value| value.as_bool())
        .unwrap_or(false);
    if looks_sensitive && candidate.candidate_type != "memory_retract" {
        return gate_result(
            candidate,
            evidence,
            confidence,
            "blocked",
            false,
            "memory operation contains sensitive-looking material and needs separate review"
                .to_string(),
        );
    }
    if confidence < DEFAULT_MIN_CONFIDENCE {
        return gate_result(
            candidate,
            evidence,
            confidence,
            "blocked",
            false,
            "memory operation confidence is below replay-gate threshold".to_string(),
        );
    }
    gate_result(
        candidate,
        evidence,
        confidence,
        "passed",
        true,
        "memory operation has a valid reviewed operation reference".to_string(),
    )
}

fn evaluate_memory_deprecate(
    candidate: &learning_candidate::Model,
    evidence: EvidenceBundle,
    confidence: f64,
) -> CandidateReplayGateResult {
    let item_id = candidate
        .proposed_content
        .get("item_id")
        .and_then(|value| value.as_str())
        .map(str::trim)
        .filter(|value| !value.is_empty());
    let Some(item_id) = item_id else {
        return gate_result(
            candidate,
            evidence,
            confidence,
            "blocked",
            false,
            "memory deprecation candidate is missing item_id".to_string(),
        );
    };
    let Some(item) = evidence.items.iter().find(|item| item.id == item_id) else {
        return gate_result(
            candidate,
            evidence,
            confidence,
            "blocked",
            false,
            "memory deprecation target was not found in evidence".to_string(),
        );
    };
    if item.contradiction_count <= item.support_count {
        return gate_result(
            candidate,
            evidence,
            confidence,
            "blocked",
            false,
            "memory item is not contradicted more often than supported".to_string(),
        );
    }
    gate_result(
        candidate,
        evidence,
        confidence,
        "passed",
        true,
        "memory deprecation is supported by structured contradiction evidence".to_string(),
    )
}

fn evaluate_memory_merge(
    candidate: &learning_candidate::Model,
    evidence: EvidenceBundle,
    confidence: f64,
) -> CandidateReplayGateResult {
    let target_id = candidate
        .proposed_content
        .get("target_item_id")
        .and_then(|value| value.as_str())
        .map(str::trim)
        .filter(|value| !value.is_empty());
    let source_id = candidate
        .proposed_content
        .get("source_item_id")
        .and_then(|value| value.as_str())
        .map(str::trim)
        .filter(|value| !value.is_empty());
    let (Some(target_id), Some(source_id)) = (target_id, source_id) else {
        return gate_result(
            candidate,
            evidence,
            confidence,
            "blocked",
            false,
            "memory merge candidate is missing source or target item".to_string(),
        );
    };
    if target_id == source_id {
        return gate_result(
            candidate,
            evidence,
            confidence,
            "blocked",
            false,
            "memory merge source and target are the same item".to_string(),
        );
    }
    let target = evidence.items.iter().find(|item| item.id == target_id);
    let source = evidence.items.iter().find(|item| item.id == source_id);
    let (Some(target), Some(source)) = (target, source) else {
        return gate_result(
            candidate,
            evidence,
            confidence,
            "blocked",
            false,
            "memory merge source or target was not found in evidence".to_string(),
        );
    };
    if target.kind != source.kind || target.scope != source.scope {
        return gate_result(
            candidate,
            evidence,
            confidence,
            "blocked",
            false,
            "memory merge source and target do not share kind and scope".to_string(),
        );
    }
    if confidence < DEFAULT_MIN_CONFIDENCE {
        return gate_result(
            candidate,
            evidence,
            confidence,
            "blocked",
            false,
            "memory merge confidence is below replay-gate threshold".to_string(),
        );
    }
    gate_result(
        candidate,
        evidence,
        confidence,
        "passed",
        true,
        "memory merge source and target are structurally compatible".to_string(),
    )
}

fn gate_result(
    candidate: &learning_candidate::Model,
    evidence: EvidenceBundle,
    confidence: f64,
    status: &str,
    allow_approval: bool,
    reason: String,
) -> CandidateReplayGateResult {
    CandidateReplayGateResult {
        candidate_id: candidate.id.clone(),
        candidate_type: candidate.candidate_type.clone(),
        status: status.to_string(),
        allow_approval,
        reason,
        evidence_refs_checked: evidence.refs_checked,
        samples: evidence.samples,
        safe_run_samples: evidence.safe_run_samples,
        excluded_sensitive_runs: evidence.excluded_sensitive_runs,
        pii_redacted_runs: evidence.pii_redacted_runs,
        success_rate: round4(evidence.success_rate()),
        correction_rate: round4(evidence.correction_rate()),
        confidence: round4(confidence),
        support_score: round4(support_score(&evidence, confidence)),
        min_samples: DEFAULT_MIN_EVIDENCE_SAMPLES,
        min_confidence: DEFAULT_MIN_CONFIDENCE,
        min_support_score: 0.0,
        max_correction_rate: DEFAULT_MAX_CORRECTION_RATE,
        usage_samples: evidence.usage.samples,
        avg_input_tokens: round4(evidence.usage.avg_input_tokens()),
        avg_output_tokens: round4(evidence.usage.avg_output_tokens()),
        avg_total_tokens: round4(evidence.usage.avg_total_tokens()),
        max_total_tokens: evidence.usage.max_total_tokens,
        avg_cost_usd: round4(evidence.usage.avg_cost_usd()),
        signals: evidence.signals.into_iter().take(8).collect(),
    }
}

fn support_score(evidence: &EvidenceBundle, confidence: f64) -> f64 {
    let sample_support = if evidence.samples >= DEFAULT_MIN_EVIDENCE_SAMPLES {
        1.0
    } else {
        ratio(evidence.samples, DEFAULT_MIN_EVIDENCE_SAMPLES)
    };
    let correction_support = 1.0 - evidence.correction_rate().clamp(0.0, 1.0);
    confidence
        .min(evidence.success_rate())
        .min(sample_support)
        .min(correction_support)
        .clamp(0.0, 1.0)
}

fn evidence_ref_strings(value: &Value) -> Vec<String> {
    value
        .as_array()
        .map(|items| {
            items
                .iter()
                .filter_map(|item| item.as_str())
                .map(str::trim)
                .filter(|value| !value.is_empty())
                .map(|value| value.to_string())
                .collect::<Vec<_>>()
        })
        .unwrap_or_default()
}

fn collect_candidate_content_refs(candidate: &learning_candidate::Model, refs: &mut Vec<String>) {
    for key in [
        "item_id",
        "target_item_id",
        "source_item_id",
        "operation_id",
        "pattern_id",
    ] {
        if let Some(value) = candidate
            .proposed_content
            .get(key)
            .and_then(|value| value.as_str())
            .map(str::trim)
            .filter(|value| !value.is_empty())
        {
            refs.push(value.to_string());
        }
    }
}

fn experience_run_privacy_safe(run: &experience_run::Model, evidence: &mut EvidenceBundle) -> bool {
    let Some(request) = run.request_text.as_deref() else {
        return true;
    };
    let secret_redaction = crate::security::redact_secret_input(request);
    if secret_redaction.had_secret() {
        return false;
    }
    if crate::security::redact_pii(&secret_redaction.text) != secret_redaction.text {
        evidence.pii_redacted_runs += 1;
    }
    true
}

fn experience_run_resolved(run: &experience_run::Model) -> bool {
    run.success_state != "provisional" || run.correction_state == "corrected"
}

fn experience_run_success(run: &experience_run::Model) -> bool {
    run.success_state == "accepted" && run.correction_state != "corrected"
}

fn experience_run_corrected(run: &experience_run::Model) -> bool {
    run.correction_state == "corrected"
}

fn json_i64_field(value: &Value, key: &str) -> Option<i64> {
    value
        .get(key)
        .and_then(|field| field.as_i64().or_else(|| field.as_u64().map(|v| v as i64)))
}

fn json_f64(value: &Value) -> Option<f64> {
    value
        .as_f64()
        .or_else(|| value.as_i64().map(|value| value as f64))
        .or_else(|| value.as_u64().map(|value| value as f64))
        .or_else(|| {
            value
                .as_str()
                .and_then(|value| value.trim().parse::<f64>().ok())
        })
}

fn ratio(numerator: usize, denominator: usize) -> f64 {
    if denominator == 0 {
        0.0
    } else {
        numerator as f64 / denominator as f64
    }
}

fn ratio_i64(numerator: i64, denominator: usize) -> f64 {
    if denominator == 0 {
        0.0
    } else {
        numerator as f64 / denominator as f64
    }
}

fn round4(value: f64) -> f64 {
    (value * 10_000.0).round() / 10_000.0
}

#[cfg(test)]
mod tests {
    use super::*;

    fn candidate(
        candidate_type: &str,
        confidence: f64,
        proposed_content: Value,
    ) -> learning_candidate::Model {
        learning_candidate::Model {
            id: "candidate-test".to_string(),
            candidate_type: candidate_type.to_string(),
            subject_key: "subject".to_string(),
            title: "Test candidate".to_string(),
            summary: None,
            project_id: None,
            conversation_id: None,
            pattern_id: None,
            evidence_refs: Value::Array(Vec::new()),
            proposed_content,
            confidence,
            approval_status: "draft".to_string(),
            review_notes: None,
            reviewed_at: None,
            approved_ref: None,
            created_at: "2026-01-01T00:00:00Z".to_string(),
            updated_at: "2026-01-01T00:00:00Z".to_string(),
        }
    }

    #[test]
    fn structured_candidate_blocks_without_evidence() {
        let result = evaluate_candidate_with_evidence(
            &candidate("strategy", 0.9, serde_json::json!({})),
            EvidenceBundle::default(),
        );
        assert!(!result.allow_approval);
        assert_eq!(result.status, "needs_more_data");
    }

    #[test]
    fn skill_patch_candidates_are_blocked_from_ark_evolve() {
        let result = evaluate_candidate_with_evidence(
            &candidate("skill_patch", 0.99, serde_json::json!({})),
            EvidenceBundle::default(),
        );
        assert!(!result.allow_approval);
        assert_eq!(result.status, "blocked");
        assert!(result.reason.contains("does not create or modify skills"));
    }

    fn turn_decision_run(id: &str) -> experience_run::Model {
        experience_run::Model {
            id: id.to_string(),
            execution_run_id: None,
            trace_id: Some(format!("trace-{id}")),
            conversation_id: None,
            project_id: None,
            channel: "chat".to_string(),
            scope: "global".to_string(),
            intent_key: "turn_decision_test".to_string(),
            task_type: Some("conversation".to_string()),
            request_text: None,
            tool_sequence_digest: None,
            tool_sequence_json: serde_json::json!([]),
            strategy_version: None,
            policy_version: None,
            prompt_version: None,
            model_slot: Some("model_tool_loop".to_string()),
            success_state: "accepted".to_string(),
            correction_state: "none".to_string(),
            outcome_summary: None,
            failure_reason: None,
            metadata: serde_json::json!({
                "turn_decision": {
                    "schema_version": 1,
                    "objective_order": ["accuracy", "safety", "tokens_latency_cost"],
                    "path": "model_tool_loop",
                    "task_type": "conversation",
                    "usage_delta": {
                        "input_tokens": 1200,
                        "output_tokens": 80,
                        "total_tokens": 1280,
                        "cost_usd": 0.002
                    }
                }
            }),
            consolidated: false,
            accepted_at: Some("2026-01-01T00:00:00Z".to_string()),
            corrected_at: None,
            heuristic_reflected: false,
            heuristic_reflection_status: None,
            heuristic_reflection_attempted_at: None,
            heuristic_reflection_completed_at: None,
            heuristic_lesson_id: None,
            heuristic_reflection_error: None,
            created_at: "2026-01-01T00:00:00Z".to_string(),
            updated_at: "2026-01-01T00:00:00Z".to_string(),
        }
    }

    #[test]
    fn turn_decision_candidate_requires_typed_turn_evidence() {
        let mut evidence = EvidenceBundle::default();
        evidence.add_run(turn_decision_run("run-1"));
        evidence.add_run(turn_decision_run("run-2"));
        let result = evaluate_candidate_with_evidence(
            &candidate("turn_decision", 0.9, serde_json::json!({})),
            evidence,
        );
        assert!(result.allow_approval);
        assert_eq!(result.status, "passed");
        assert_eq!(result.usage_samples, 2);
        assert_eq!(result.avg_total_tokens, 1280.0);
        assert_eq!(result.max_total_tokens, 1280);
        assert_eq!(result.avg_cost_usd, 0.002);
        assert!(result
            .signals
            .iter()
            .any(|signal| signal.contains("turn decision evidence")));
    }

    #[test]
    fn routing_canonical_candidate_uses_turn_decision_replay_gate() {
        let mut evidence = EvidenceBundle::default();
        evidence.add_run(turn_decision_run("run-1"));
        evidence.add_run(turn_decision_run("run-2"));
        let result = evaluate_candidate_with_evidence(
            &candidate(
                super::ROUTING_CANONICAL_CANDIDATE_TYPE,
                0.9,
                serde_json::json!({
                    "add": [{
                        "category": "durable_work",
                        "concept": "background_monitoring_goal",
                        "text": "The user wants durable background monitoring that persists independently of the current response and reports only when its condition is met."
                    }],
                    "evidence_summary": "Replay evidence showed durable monitoring was misrouted."
                }),
            ),
            evidence,
        );
        assert!(result.allow_approval);
        assert_eq!(result.status, "passed");
    }

    #[test]
    fn memory_deprecate_requires_more_contradictions_than_support() {
        let item = experience_item::Model {
            id: "item-1".to_string(),
            kind: "procedure".to_string(),
            scope: "global".to_string(),
            project_id: None,
            conversation_id: None,
            title: "Procedure".to_string(),
            content: "content".to_string(),
            normalized_key: "procedure".to_string(),
            confidence: 0.7,
            support_count: 1,
            contradiction_count: 3,
            status: "active".to_string(),
            metadata: Value::Null,
            last_supported_at: None,
            last_contradicted_at: None,
            created_at: "2026-01-01T00:00:00Z".to_string(),
            updated_at: "2026-01-01T00:00:00Z".to_string(),
            embedding: None,
        };
        let mut evidence = EvidenceBundle::default();
        evidence.add_item(item);
        let result = evaluate_candidate_with_evidence(
            &candidate(
                "memory_deprecate",
                0.74,
                serde_json::json!({ "item_id": "item-1" }),
            ),
            evidence,
        );
        assert!(result.allow_approval);
        assert_eq!(result.status, "passed");
    }
}
