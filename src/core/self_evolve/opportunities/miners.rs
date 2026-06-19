//! Dimension-generic opportunity detectors. Every threshold is RELATIVE to
//! the install's own distribution (window medians/rates) — never an absolute
//! domain rule, keyword, or use-case template. Each miner is deterministic,
//! synchronous, and bounded by the window size.

use super::window::{SegmentStats, UsageWindow};
use super::{
    stable_content_hash, ExpectedBenefit, OpportunityDraft, OpportunityEvidence, OpportunityMiner,
};

/// Minimum samples before a segment is worth judging at all.
const MIN_SEGMENT_SAMPLES: usize = 5;
/// "Hotspot" = at least this multiple of the install's own median.
const HOTSPOT_RATIO: f64 = 2.0;
/// Failure cluster needs both an absolute floor and a relative excess.
const MIN_CORRECTED_COUNT: usize = 3;
const CORRECTED_RATE_FLOOR: f64 = 0.2;
/// Repeat pattern: segment is a material share of all activity.
const REPEAT_SHARE_FLOOR: f64 = 0.15;
const REPEAT_MIN_SAMPLES: usize = 8;
/// Reserve this fraction of example runs as gate holdouts.
const HOLDOUT_FRACTION: f64 = 0.5;
const MIN_OPERATION_SUPPORT: usize = 2;
const SEGMENT_EXAMPLE_CAP: usize = 8;

pub(super) fn default_miners() -> Vec<Box<dyn OpportunityMiner>> {
    vec![
        Box::new(TokenHotspotMiner),
        Box::new(LatencyHotspotMiner),
        Box::new(FailureClusterMiner),
        Box::new(RepeatPatternMiner),
        Box::new(RouterMissMiner),
        Box::new(OperationContractRepairMiner),
    ]
}

fn confidence_from_samples(samples: usize, window_runs: usize) -> f64 {
    if window_runs == 0 {
        return 0.0;
    }
    // Saturating: 5 samples ≈ 0.4, 20 ≈ 0.75, 60+ ≈ 0.92+. Pure sample-size
    // heuristic, no domain content.
    let samples = samples as f64;
    (samples / (samples + 7.0)).min(0.95)
}

fn evidence_for(segment: &SegmentStats, window: &UsageWindow) -> OpportunityEvidence {
    OpportunityEvidence {
        sample_runs: segment.sample_count,
        corrected_runs: segment.corrected_count,
        corrected_rate: segment.corrected_rate(),
        avg_tokens_per_turn: segment.avg_tokens,
        p95_wall_ms: segment.p95_wall_ms,
        avg_cost_microusd: segment.avg_cost_microusd,
        example_run_ids: segment.example_run_ids.clone(),
        window_runs: window.runs.len(),
    }
}

fn holdout_split(segment: &SegmentStats) -> Vec<String> {
    let holdout_count =
        ((segment.example_run_ids.len() as f64) * HOLDOUT_FRACTION).floor() as usize;
    segment
        .example_run_ids
        .iter()
        .take(holdout_count)
        .cloned()
        .collect()
}

fn topic_text_for(segment: &SegmentStats) -> String {
    let mut parts = vec![segment.label.clone()];
    parts.extend(segment.example_requests.iter().take(3).cloned());
    parts.join("\n")
}

struct TokenHotspotMiner;

impl OpportunityMiner for TokenHotspotMiner {
    fn key(&self) -> &'static str {
        "token_hotspot"
    }

    fn mine(&self, window: &UsageWindow) -> Vec<OpportunityDraft> {
        let Some(median_tokens) = window.median_tokens.filter(|median| *median > 0.0) else {
            return Vec::new();
        };
        window
            .segments()
            .iter()
            .filter(|segment| segment.sample_count >= MIN_SEGMENT_SAMPLES)
            .filter_map(|segment| {
                let avg = segment.avg_tokens?;
                if avg < median_tokens * HOTSPOT_RATIO {
                    return None;
                }
                let excess = avg - median_tokens;
                Some(OpportunityDraft {
                    miner_key: self.key(),
                    segment_key: segment.key.clone(),
                    segment_label: segment.label.clone(),
                    target_surface: "prompt_bundle:primary_response".to_string(),
                    evidence: evidence_for(segment, window),
                    expected_benefit: ExpectedBenefit {
                        tokens_per_turn: Some(excess * 0.5),
                        ms_per_turn: None,
                        corrected_rate_delta: None,
                        confidence: confidence_from_samples(
                            segment.sample_count,
                            window.runs.len(),
                        ),
                    },
                    risk: "Prompt-weight change for one activity segment; canary-gated".to_string(),
                    holdout_run_ids: holdout_split(segment),
                    topic_text: topic_text_for(segment),
                })
            })
            .collect()
    }
}

struct LatencyHotspotMiner;

impl OpportunityMiner for LatencyHotspotMiner {
    fn key(&self) -> &'static str {
        "latency_hotspot"
    }

    fn mine(&self, window: &UsageWindow) -> Vec<OpportunityDraft> {
        let Some(median_wall) = window.median_wall_ms.filter(|median| *median > 0) else {
            return Vec::new();
        };
        window
            .segments()
            .iter()
            .filter(|segment| segment.sample_count >= MIN_SEGMENT_SAMPLES)
            .filter_map(|segment| {
                let p95 = segment.p95_wall_ms?;
                if (p95 as f64) < (median_wall as f64) * HOTSPOT_RATIO {
                    return None;
                }
                let excess_ms = p95.saturating_sub(median_wall);
                Some(OpportunityDraft {
                    miner_key: self.key(),
                    segment_key: segment.key.clone(),
                    segment_label: segment.label.clone(),
                    target_surface: "prompt_bundle:primary_response".to_string(),
                    evidence: evidence_for(segment, window),
                    expected_benefit: ExpectedBenefit {
                        tokens_per_turn: None,
                        ms_per_turn: Some(excess_ms as f64 * 0.4),
                        corrected_rate_delta: None,
                        confidence: confidence_from_samples(
                            segment.sample_count,
                            window.runs.len(),
                        ),
                    },
                    risk: "Latency-focused prompt/strategy change; canary-gated".to_string(),
                    holdout_run_ids: holdout_split(segment),
                    topic_text: topic_text_for(segment),
                })
            })
            .collect()
    }
}

struct FailureClusterMiner;

impl OpportunityMiner for FailureClusterMiner {
    fn key(&self) -> &'static str {
        "failure_cluster"
    }

    fn mine(&self, window: &UsageWindow) -> Vec<OpportunityDraft> {
        let baseline = window.overall_corrected_rate;
        window
            .segments()
            .iter()
            .filter(|segment| {
                segment.corrected_count >= MIN_CORRECTED_COUNT
                    && segment.corrected_rate() >= CORRECTED_RATE_FLOOR
                    && segment.corrected_rate() >= baseline * HOTSPOT_RATIO
            })
            .map(|segment| OpportunityDraft {
                miner_key: self.key(),
                segment_key: segment.key.clone(),
                segment_label: segment.label.clone(),
                target_surface: "prompt_bundle:primary_response".to_string(),
                evidence: evidence_for(segment, window),
                expected_benefit: ExpectedBenefit {
                    tokens_per_turn: None,
                    ms_per_turn: None,
                    corrected_rate_delta: Some(-(segment.corrected_rate() - baseline) * 0.5),
                    confidence: confidence_from_samples(segment.sample_count, window.runs.len()),
                },
                risk: "Behavioral prompt change where answers keep getting corrected; \
                       canary-gated"
                    .to_string(),
                holdout_run_ids: holdout_split(segment),
                topic_text: topic_text_for(segment),
            })
            .collect()
    }
}

struct RepeatPatternMiner;

impl OpportunityMiner for RepeatPatternMiner {
    fn key(&self) -> &'static str {
        "repeat_pattern"
    }

    fn mine(&self, window: &UsageWindow) -> Vec<OpportunityDraft> {
        let total = window.runs.len();
        if total == 0 {
            return Vec::new();
        }
        window
            .segments()
            .iter()
            .filter(|segment| {
                segment.sample_count >= REPEAT_MIN_SAMPLES
                    && (segment.sample_count as f64 / total as f64) >= REPEAT_SHARE_FLOOR
            })
            .map(|segment| OpportunityDraft {
                miner_key: self.key(),
                segment_key: segment.key.clone(),
                segment_label: segment.label.clone(),
                target_surface: "prompt_fragment".to_string(),
                evidence: evidence_for(segment, window),
                expected_benefit: ExpectedBenefit {
                    tokens_per_turn: segment
                        .avg_tokens
                        .zip(window.median_tokens)
                        .map(|(avg, median)| ((avg - median) * 0.25).max(0.0)),
                    ms_per_turn: None,
                    corrected_rate_delta: Some(-segment.corrected_rate() * 0.25),
                    confidence: confidence_from_samples(segment.sample_count, window.runs.len()),
                },
                risk: "Tailored prompt fragment for the user's dominant recurring activity; \
                       canary-gated"
                    .to_string(),
                holdout_run_ids: holdout_split(segment),
                topic_text: topic_text_for(segment),
            })
            .collect()
    }
}

struct RouterMissMiner;

impl OpportunityMiner for RouterMissMiner {
    fn key(&self) -> &'static str {
        "router_miss"
    }

    fn mine(&self, window: &UsageWindow) -> Vec<OpportunityDraft> {
        let mut groups = std::collections::BTreeMap::<String, RouterMissGroup>::new();
        for candidate in &window.router_learning_candidates {
            if candidate.candidate_type != crate::core::self_evolve::ROUTER_LEARNING_CANDIDATE_TYPE
            {
                continue;
            }
            let Ok(payload) = serde_json::from_value::<
                crate::core::self_evolve::RouterLearningCandidatePayload,
            >(candidate.proposed_content.clone()) else {
                continue;
            };
            let layer = payload.router_layer.trim();
            let objective_hash = stable_content_hash(payload.objective.trim());
            let key = format!(
                "router:{}:{}",
                if layer.is_empty() { "unknown" } else { layer },
                objective_hash
            );
            let group = groups.entry(key).or_insert_with(|| RouterMissGroup {
                layer: if layer.is_empty() {
                    "router layer".to_string()
                } else {
                    layer.to_string()
                },
                objective: payload.objective.clone(),
                evidence_count: 0,
                example_run_ids: Vec::new(),
                example_requests: Vec::new(),
            });
            group.evidence_count += payload.evidence.len().max(1);
            for evidence in payload.evidence.iter().take(SEGMENT_EXAMPLE_CAP) {
                if !evidence.trace_id.trim().is_empty()
                    && group.example_run_ids.len() < SEGMENT_EXAMPLE_CAP
                {
                    group.example_run_ids.push(evidence.trace_id.clone());
                }
                if !evidence.user_message_preview.trim().is_empty()
                    && group.example_requests.len() < 3
                {
                    group
                        .example_requests
                        .push(evidence.user_message_preview.trim().to_string());
                }
            }
        }

        groups
            .into_iter()
            .filter(|(_, group)| group.evidence_count > 0)
            .map(|(segment_key, group)| {
                let sample_runs = group.evidence_count;
                OpportunityDraft {
                    miner_key: self.key(),
                    segment_key,
                    segment_label: format!("router misses in {}", group.layer),
                    target_surface: "prompt_fragment:router_guidance".to_string(),
                    evidence: OpportunityEvidence {
                        sample_runs,
                        corrected_runs: sample_runs,
                        corrected_rate: 1.0,
                        avg_tokens_per_turn: None,
                        p95_wall_ms: None,
                        avg_cost_microusd: None,
                        example_run_ids: group.example_run_ids.clone(),
                        window_runs: window.runs.len(),
                    },
                    expected_benefit: ExpectedBenefit {
                        tokens_per_turn: None,
                        ms_per_turn: None,
                        corrected_rate_delta: Some(-0.25),
                        confidence: confidence_from_samples(sample_runs, window.runs.len().max(sample_runs)),
                    },
                    risk: "Router guidance can change tool/surface choice; canary-gated with holdout evals"
                        .to_string(),
                    holdout_run_ids: group.example_run_ids.iter().take(2).cloned().collect(),
                    topic_text: {
                        let mut parts = vec![group.objective];
                        parts.extend(group.example_requests);
                        parts.join("\n")
                    },
                }
            })
            .collect()
    }
}

struct OperationContractRepairMiner;

impl OpportunityMiner for OperationContractRepairMiner {
    fn key(&self) -> &'static str {
        "operation_contract_repair"
    }

    fn mine(&self, window: &UsageWindow) -> Vec<OpportunityDraft> {
        let mut groups = std::collections::BTreeMap::<String, OperationContractGroup>::new();
        for run in &window.runs {
            let events = super::contract_events::contract_events_from_metadata(&run.metadata);
            for event in events {
                let identity = super::contract_events::contract_event_identity(&event);
                let segment_key = format!("operation:{}", stable_content_hash(&identity));
                let group = groups
                    .entry(segment_key)
                    .or_insert_with(|| OperationContractGroup {
                        label: event.operation_descriptor.clone(),
                        contract_kind: event.contract_kind.clone(),
                        run_ids: Vec::new(),
                        item_ids: Vec::new(),
                        corrected_count: 0,
                        token_values: Vec::new(),
                        wall_values: Vec::new(),
                        cost_values: Vec::new(),
                        example_requests: Vec::new(),
                    });
                if group.run_ids.len() < SEGMENT_EXAMPLE_CAP {
                    group.run_ids.push(run.id.clone());
                }
                if run.correction_state == "corrected" || run.success_state == "failed" {
                    group.corrected_count += 1;
                }
                if let (Some(tokens_in), Some(tokens_out)) = (run.tokens_in, run.tokens_out) {
                    group.token_values.push((tokens_in + tokens_out) as f64);
                }
                if let Some(wall_ms) = run.wall_ms {
                    group.wall_values.push(wall_ms);
                }
                if let Some(cost) = run.est_cost_microusd {
                    group.cost_values.push(cost as f64);
                }
                if let Some(request) = run
                    .request_text
                    .as_deref()
                    .map(str::trim)
                    .filter(|value| !value.is_empty())
                {
                    if group.example_requests.len() < 3 {
                        group.example_requests.push(request.to_string());
                    }
                }
            }
        }
        for item in &window.operation_items {
            let Some((segment_key, label, contract_kind)) = operation_item_group_key(item) else {
                continue;
            };
            let group = groups
                .entry(segment_key)
                .or_insert_with(|| OperationContractGroup {
                    label,
                    contract_kind,
                    run_ids: Vec::new(),
                    item_ids: Vec::new(),
                    corrected_count: 0,
                    token_values: Vec::new(),
                    wall_values: Vec::new(),
                    cost_values: Vec::new(),
                    example_requests: Vec::new(),
                });
            if group.item_ids.len() < SEGMENT_EXAMPLE_CAP {
                group.item_ids.push(item.id.clone());
            }
            if group.example_requests.len() < 3 {
                group.example_requests.push(item.content.clone());
            }
        }

        groups
            .into_iter()
            .filter(|(_, group)| group.support_count() >= MIN_OPERATION_SUPPORT)
            .map(|(segment_key, mut group)| {
                let sample_runs = group.support_count();
                let corrected_rate = if group.run_ids.is_empty() {
                    0.0
                } else {
                    group.corrected_count as f64 / group.run_ids.len() as f64
                };
                let mut wall_values = group.wall_values.clone();
                let p95_wall_ms = percentile_i64(&mut wall_values, 0.95);
                let avg_tokens = mean(&group.token_values);
                let avg_cost = mean(&group.cost_values);
                let mut evidence_ids = group.run_ids.clone();
                evidence_ids.extend(group.item_ids.iter().map(|id| format!("item:{id}")));
                evidence_ids.truncate(SEGMENT_EXAMPLE_CAP);
                let holdout_run_ids = group.run_ids.iter().take(2).cloned().collect::<Vec<_>>();
                let mut topic_parts = vec![group.label.clone(), group.contract_kind.clone()];
                topic_parts.append(&mut group.example_requests);
                OpportunityDraft {
                    miner_key: self.key(),
                    segment_key,
                    segment_label: group.label,
                    target_surface: "prompt_fragment:operation_contract_preflight".to_string(),
                    evidence: OpportunityEvidence {
                        sample_runs,
                        corrected_runs: group.corrected_count,
                        corrected_rate,
                        avg_tokens_per_turn: avg_tokens,
                        p95_wall_ms,
                        avg_cost_microusd: avg_cost,
                        example_run_ids: evidence_ids,
                        window_runs: window.runs.len(),
                    },
                    expected_benefit: ExpectedBenefit {
                        tokens_per_turn: avg_tokens.map(|value| value * 0.20),
                        ms_per_turn: p95_wall_ms.map(|value| value as f64 * 0.20),
                        corrected_rate_delta: Some(-corrected_rate.max(0.20) * 0.50),
                        confidence: confidence_from_samples(sample_runs, window.runs.len().max(sample_runs)),
                    },
                    risk: "Scoped operation-contract preflight guidance; canary-gated with secret-handling safety floor"
                        .to_string(),
                    holdout_run_ids,
                    topic_text: topic_parts.join("\n"),
                }
            })
            .collect()
    }
}

struct RouterMissGroup {
    layer: String,
    objective: String,
    evidence_count: usize,
    example_run_ids: Vec<String>,
    example_requests: Vec<String>,
}

struct OperationContractGroup {
    label: String,
    contract_kind: String,
    run_ids: Vec<String>,
    item_ids: Vec<String>,
    corrected_count: usize,
    token_values: Vec<f64>,
    wall_values: Vec<i64>,
    cost_values: Vec<f64>,
    example_requests: Vec<String>,
}

impl OperationContractGroup {
    fn support_count(&self) -> usize {
        self.run_ids.len() + self.item_ids.len()
    }
}

fn operation_item_group_key(
    item: &crate::storage::entities::experience_item::Model,
) -> Option<(String, String, String)> {
    if item.status != "active" || !matches!(item.kind.as_str(), "lesson" | "procedure") {
        return None;
    }
    if item
        .metadata
        .get("operation_contract_learning")
        .and_then(|value| value.as_bool())
        .unwrap_or(false)
    {
        let contract_kind = item
            .metadata
            .get("contract_kind")
            .and_then(|value| value.as_str())
            .map(str::trim)
            .filter(|value| !value.is_empty())
            .unwrap_or("operation_contract")
            .to_string();
        let shape = item
            .metadata
            .get("operation_shape")
            .and_then(|value| value.as_str())
            .map(str::trim)
            .filter(|value| !value.is_empty())
            .unwrap_or(contract_kind.as_str());
        return Some((
            format!(
                "operation:{}",
                stable_content_hash(&format!("{}::{}", contract_kind, shape))
            ),
            shape.to_string(),
            contract_kind,
        ));
    }
    let has_tool_evidence = item.metadata.get("tool_sequence_digest").is_some()
        || item.metadata.get("tool_sequence").is_some();
    let polarity = item
        .metadata
        .get("polarity")
        .and_then(|value| value.as_str())
        .unwrap_or_default();
    if has_tool_evidence && polarity == "negative" {
        let digest = item
            .metadata
            .get("tool_sequence_digest")
            .and_then(|value| value.as_str())
            .unwrap_or(item.normalized_key.as_str());
        return Some((
            format!("operation:{}", stable_content_hash(digest)),
            "repeated operation repair lesson".to_string(),
            "operation_contract".to_string(),
        ));
    }
    None
}

fn mean(values: &[f64]) -> Option<f64> {
    if values.is_empty() {
        None
    } else {
        Some(values.iter().sum::<f64>() / values.len() as f64)
    }
}

fn percentile_i64(values: &mut [i64], percentile: f64) -> Option<i64> {
    if values.is_empty() {
        return None;
    }
    values.sort_unstable();
    let rank = ((values.len() as f64 - 1.0) * percentile.clamp(0.0, 1.0)).round() as usize;
    values.get(rank).copied()
}

#[cfg(test)]
mod tests {
    use super::super::window::UsageWindow;
    use super::*;
    use crate::storage::entities::experience_run::Model as ExperienceRun;

    fn run(
        id: &str,
        intent: &str,
        tokens: Option<(i64, i64)>,
        wall_ms: Option<i64>,
        corrected: bool,
    ) -> ExperienceRun {
        ExperienceRun {
            id: id.to_string(),
            execution_run_id: None,
            trace_id: None,
            conversation_id: None,
            project_id: None,
            channel: "web".to_string(),
            scope: "chat".to_string(),
            intent_key: intent.to_string(),
            task_type: Some("chat".to_string()),
            request_text: Some(format!("request about {intent}")),
            tool_sequence_digest: None,
            tool_sequence_json: serde_json::json!([]),
            strategy_version: None,
            policy_version: None,
            prompt_version: None,
            model_slot: None,
            tokens_in: tokens.map(|(input, _)| input),
            tokens_out: tokens.map(|(_, output)| output),
            wall_ms,
            est_cost_microusd: None,
            success_state: if corrected { "failed" } else { "accepted" }.to_string(),
            correction_state: if corrected { "corrected" } else { "none" }.to_string(),
            outcome_summary: None,
            failure_reason: None,
            metadata: serde_json::json!({}),
            consolidated: false,
            accepted_at: None,
            corrected_at: None,
            heuristic_reflected: false,
            heuristic_reflection_status: None,
            heuristic_reflection_attempted_at: None,
            heuristic_reflection_completed_at: None,
            heuristic_lesson_id: None,
            heuristic_reflection_error: None,
            created_at: "2026-06-10T00:00:00Z".to_string(),
            updated_at: "2026-06-10T00:00:00Z".to_string(),
        }
    }

    fn window_with_token_hotspot() -> UsageWindow {
        let mut runs = Vec::new();
        // Baseline activity: cheap turns across two segments.
        for index in 0..10 {
            runs.push(run(
                &format!("cheap-a-{index}"),
                "everyday-questions",
                Some((400, 100)),
                Some(1_500),
                false,
            ));
        }
        // Hotspot: one segment consistently burns far above the median.
        for index in 0..6 {
            runs.push(run(
                &format!("hot-{index}"),
                "deep-data-analysis",
                Some((4_000, 800)),
                Some(2_000),
                false,
            ));
        }
        UsageWindow::from_runs(runs, None)
    }

    fn contract_run(id: &str, corrected: bool) -> ExperienceRun {
        let mut run = run(
            id,
            "operation-contract",
            Some((1_000, 250)),
            Some(1_800),
            corrected,
        );
        run.metadata = serde_json::json!({
            "contract_events": [{
                "source": "custom_api",
                "surface": "custom_api",
                "operation_descriptor": "custom_api operation_envelope",
                "contract_kind": "operation_envelope",
                "schema_hash": "shape-1",
                "missing_fields": ["body"],
                "violations": ["missing_request_body"],
                "recoverable_by_model": true,
                "requires_user_secret": false,
                "result_state": "failed"
            }]
        });
        run
    }

    #[test]
    fn token_hotspot_fires_relative_to_install_median_only() {
        let drafts = TokenHotspotMiner.mine(&window_with_token_hotspot());
        assert_eq!(drafts.len(), 1);
        let draft = &drafts[0];
        assert_eq!(draft.miner_key, "token_hotspot");
        assert_eq!(draft.segment_key, "deep-data-analysis");
        assert!(draft.expected_benefit.tokens_per_turn.unwrap() > 0.0);
        assert!(!draft.holdout_run_ids.is_empty());
    }

    #[test]
    fn failure_cluster_requires_relative_excess_not_just_failures() {
        let mut runs = Vec::new();
        // Everything fails equally often — no cluster stands out, no draft.
        for index in 0..10 {
            runs.push(run(
                &format!("a-{index}"),
                "segment-a",
                None,
                None,
                index % 2 == 0,
            ));
            runs.push(run(
                &format!("b-{index}"),
                "segment-b",
                None,
                None,
                index % 2 == 0,
            ));
        }
        let uniform = UsageWindow::from_runs(runs, None);
        assert!(FailureClusterMiner.mine(&uniform).is_empty());

        // One segment fails far above the install baseline → exactly one draft.
        let mut runs = Vec::new();
        for index in 0..20 {
            runs.push(run(
                &format!("ok-{index}"),
                "healthy-segment",
                None,
                None,
                false,
            ));
        }
        for index in 0..6 {
            runs.push(run(
                &format!("bad-{index}"),
                "struggling-segment",
                None,
                None,
                index < 4,
            ));
        }
        let skewed = UsageWindow::from_runs(runs, None);
        let drafts = FailureClusterMiner.mine(&skewed);
        assert_eq!(drafts.len(), 1);
        assert_eq!(drafts[0].segment_key, "struggling-segment");
        assert!(drafts[0].expected_benefit.corrected_rate_delta.unwrap() < 0.0);
    }

    #[test]
    fn repeat_pattern_finds_dominant_segments() {
        let mut runs = Vec::new();
        for index in 0..12 {
            runs.push(run(
                &format!("dominant-{index}"),
                "recurring-workflow",
                Some((500, 120)),
                None,
                false,
            ));
        }
        for index in 0..4 {
            runs.push(run(
                &format!("misc-{index}"),
                &format!("one-off-{index}"),
                None,
                None,
                false,
            ));
        }
        let window = UsageWindow::from_runs(runs, None);
        let drafts = RepeatPatternMiner.mine(&window);
        assert_eq!(drafts.len(), 1);
        assert_eq!(drafts[0].segment_key, "recurring-workflow");
        assert_eq!(drafts[0].target_surface, "prompt_fragment");
    }

    #[test]
    fn operation_contract_repair_surfaces_from_contract_events() {
        let window = UsageWindow::from_runs(
            vec![
                contract_run("contract-a", true),
                contract_run("contract-b", false),
            ],
            None,
        );

        let drafts = OperationContractRepairMiner.mine(&window);

        assert_eq!(drafts.len(), 1);
        assert_eq!(drafts[0].miner_key, "operation_contract_repair");
        assert_eq!(
            drafts[0].target_surface,
            "prompt_fragment:operation_contract_preflight"
        );
        assert!(drafts[0].topic_text.contains("operation_envelope"));
        assert!(!drafts[0].topic_text.contains("Linear"));
        assert!(!drafts[0].topic_text.contains("GraphQL"));
    }

    #[test]
    fn default_miners_include_router_and_operation_learning() {
        let keys = default_miners()
            .into_iter()
            .map(|miner| miner.key())
            .collect::<Vec<_>>();

        assert!(keys.contains(&"router_miss"));
        assert!(keys.contains(&"operation_contract_repair"));
    }
}
