//! Prompt-fragment bundle self-evolution support.
//!
//! This surface is intentionally smaller than the full prompt-bundle optimizer:
//! GEPA proposes complete fragment bundles, then AgentArk validates invariants,
//! records lineage, and activates the candidate through the same canary path
//! used by other prompt surfaces.

use anyhow::{Context, Result};
use serde::{Deserialize, Serialize};
use std::collections::{BTreeMap, BTreeSet, HashSet};
use std::hash::{Hash, Hasher};
use std::path::PathBuf;
use tokio::io::AsyncWriteExt;

use crate::core::prompt_fragments::{
    default_prompt_fragment_bundle, parse_prompt_fragment_bundle_profile,
    required_prompt_fragment_ids, sanitize_prompt_fragment_bundle, PromptFragment,
    PromptFragmentBundleProfile, PROMPT_FRAGMENT_BUNDLE_PROFILE_KEY,
};

pub(crate) const PROMPT_FRAGMENT_LINEAGE_ARCHIVE_REL_PATH: &str =
    ".agentark/self_evolve/prompt_fragment_bundle_lineage.jsonl";

const MAX_LINEAGE_ARCHIVE_ENTRIES: usize = 400;
const MAX_FRAGMENT_TOKEN_REGRESSION_RATIO: f64 = 0.15;

#[derive(Debug, Clone, Serialize, Deserialize, Default)]
pub(crate) struct PromptFragmentBundleDiffSummary {
    #[serde(default)]
    pub added_fragment_ids: Vec<String>,
    #[serde(default)]
    pub removed_fragment_ids: Vec<String>,
    #[serde(default)]
    pub changed_fragment_ids: Vec<String>,
    #[serde(default)]
    pub disabled_required_fragment_ids: Vec<String>,
    #[serde(default)]
    pub changed_surfaces: Vec<String>,
    #[serde(default)]
    pub change_preview: Vec<String>,
}

#[derive(Debug, Clone, Copy, Serialize, Deserialize, Default)]
pub(crate) struct PromptFragmentEfficiencyMetrics {
    pub total_fragments: usize,
    pub enabled_fragments: usize,
    pub always_on_fragments: usize,
    pub enabled_estimated_tokens: usize,
    pub always_on_estimated_tokens: usize,
    pub max_fragment_estimated_tokens: usize,
}

#[derive(Debug, Clone, Serialize)]
pub(crate) struct PromptFragmentEvolutionResult {
    pub success: bool,
    pub mode: String,
    pub target_key: String,
    pub baseline_version: String,
    pub candidate_version: String,
    pub promoted: bool,
    pub evaluated_candidates: usize,
    pub baseline_score: f64,
    pub best_candidate_score: f64,
    pub score_gain: f64,
    pub baseline_prompt_efficiency: PromptFragmentEfficiencyMetrics,
    pub best_candidate_prompt_efficiency: PromptFragmentEfficiencyMetrics,
    pub wins: usize,
    pub losses: usize,
    pub p_value: f64,
    pub candidate_source: Option<String>,
    pub optimized_surfaces: Vec<String>,
    pub promotion_gate: String,
    pub promoted_prompt_fragment_bundle: Option<PromptFragmentBundleProfile>,
    pub lineage_entry_id: String,
    pub lineage_archive_path: String,
    pub notes: Vec<String>,
    pub diff_summary: PromptFragmentBundleDiffSummary,
    pub error: Option<String>,
}

#[derive(Debug, Clone)]
pub(crate) struct ExternalPromptFragmentCandidate {
    pub source: String,
    pub bundle: PromptFragmentBundleProfile,
}

#[derive(Debug, Clone)]
struct CandidatePromptFragmentBundle {
    source: String,
    bundle: PromptFragmentBundleProfile,
}

#[derive(Debug, Clone)]
struct PromptFragmentBundleEvaluation {
    score: f64,
    required_fragments_enabled: bool,
    has_enabled_fragments: bool,
    efficiency: PromptFragmentEfficiencyMetrics,
    disabled_required_fragment_ids: Vec<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
struct PromptFragmentLineageEntry {
    entry_id: String,
    timestamp_utc: String,
    target_key: String,
    request: String,
    baseline_version: String,
    candidate_version: String,
    baseline_bundle_hash: String,
    candidate_bundle_hash: String,
    promoted: bool,
    baseline_score: f64,
    candidate_score: f64,
    score_gain: f64,
    candidate_source: Option<String>,
    optimized_surfaces: Vec<String>,
    promotion_gate: String,
    diff_summary: PromptFragmentBundleDiffSummary,
    notes: Vec<String>,
}

pub(crate) async fn evaluate_external_prompt_fragment_candidates(
    project_root: PathBuf,
    user_request: &str,
    current_bundle_raw: Option<&[u8]>,
    candidates: Vec<ExternalPromptFragmentCandidate>,
) -> Result<PromptFragmentEvolutionResult> {
    let mut baseline_bundle = current_bundle_raw
        .and_then(parse_prompt_fragment_bundle_profile)
        .unwrap_or_else(default_prompt_fragment_bundle);
    sanitize_prompt_fragment_bundle(&mut baseline_bundle);
    let baseline_eval = evaluate_prompt_fragment_bundle(&baseline_bundle);
    let baseline_hash = prompt_fragment_bundle_hash(&baseline_bundle);

    let mut seen_hashes = HashSet::new();
    let mut candidate_bundles = candidates
        .into_iter()
        .map(|candidate| {
            let mut bundle = candidate.bundle;
            sanitize_prompt_fragment_bundle(&mut bundle);
            CandidatePromptFragmentBundle {
                source: candidate.source,
                bundle,
            }
        })
        .filter(|candidate| {
            let hash = prompt_fragment_bundle_hash(&candidate.bundle);
            hash != baseline_hash && seen_hashes.insert(hash)
        })
        .collect::<Vec<_>>();

    let evaluated_candidates = candidate_bundles.len();
    if evaluated_candidates == 0 {
        return build_prompt_fragment_noop_result(
            project_root,
            user_request,
            &baseline_bundle,
            &baseline_eval,
        )
        .await;
    }

    candidate_bundles.sort_by(|left, right| {
        let left_eval = evaluate_prompt_fragment_bundle(&left.bundle);
        let right_eval = evaluate_prompt_fragment_bundle(&right.bundle);
        right_eval
            .required_fragments_enabled
            .cmp(&left_eval.required_fragments_enabled)
            .then_with(|| right_eval.has_enabled_fragments.cmp(&left_eval.has_enabled_fragments))
            .then_with(|| {
                left_eval
                    .efficiency
                    .enabled_estimated_tokens
                    .cmp(&right_eval.efficiency.enabled_estimated_tokens)
            })
            .then_with(|| left.source.cmp(&right.source))
    });

    let mut best_candidate = candidate_bundles
        .into_iter()
        .next()
        .context("missing prompt-fragment candidate")?;
    let candidate_hash = prompt_fragment_bundle_hash(&best_candidate.bundle);
    best_candidate.bundle.version = format!(
        "prompt-fragments-{}",
        short_hash(&[best_candidate.source.as_str(), candidate_hash.as_str()])
    );
    best_candidate.bundle.updated_at = Some(chrono::Utc::now().to_rfc3339());
    sanitize_prompt_fragment_bundle(&mut best_candidate.bundle);

    let best_eval = evaluate_prompt_fragment_bundle(&best_candidate.bundle);
    let diff_summary =
        build_prompt_fragment_bundle_diff_summary(&baseline_bundle, &best_candidate.bundle);
    let optimized_surfaces = diff_summary.changed_surfaces.clone();
    let score_gain = best_eval.score - baseline_eval.score;
    let token_regression_ratio = token_regression_ratio(
        baseline_eval.efficiency.enabled_estimated_tokens,
        best_eval.efficiency.enabled_estimated_tokens,
    );
    let changed = !diff_summary.added_fragment_ids.is_empty()
        || !diff_summary.removed_fragment_ids.is_empty()
        || !diff_summary.changed_fragment_ids.is_empty();
    let promoted = changed
        && best_eval.has_enabled_fragments
        && best_eval.required_fragments_enabled
        && token_regression_ratio <= MAX_FRAGMENT_TOKEN_REGRESSION_RATIO;
    let promotion_gate = render_prompt_fragment_promotion_gate(
        promoted,
        changed,
        best_eval.required_fragments_enabled,
        best_eval.has_enabled_fragments,
        token_regression_ratio,
    );
    let wins = usize::from(best_eval.required_fragments_enabled)
        + usize::from(best_eval.has_enabled_fragments)
        + usize::from(token_regression_ratio <= MAX_FRAGMENT_TOKEN_REGRESSION_RATIO);
    let losses = usize::from(!best_eval.required_fragments_enabled)
        + usize::from(!best_eval.has_enabled_fragments)
        + usize::from(token_regression_ratio > MAX_FRAGMENT_TOKEN_REGRESSION_RATIO);
    let notes = build_prompt_fragment_notes(
        &baseline_eval,
        &best_eval,
        token_regression_ratio,
        &diff_summary,
    );
    let lineage_entry = PromptFragmentLineageEntry {
        entry_id: format!("pfg-{}", uuid::Uuid::new_v4()),
        timestamp_utc: chrono::Utc::now().to_rfc3339(),
        target_key: PROMPT_FRAGMENT_BUNDLE_PROFILE_KEY.to_string(),
        request: user_request.to_string(),
        baseline_version: baseline_bundle.version.clone(),
        candidate_version: best_candidate.bundle.version.clone(),
        baseline_bundle_hash: baseline_hash,
        candidate_bundle_hash: prompt_fragment_bundle_hash(&best_candidate.bundle),
        promoted,
        baseline_score: round4(baseline_eval.score),
        candidate_score: round4(best_eval.score),
        score_gain: round4(score_gain),
        candidate_source: Some(best_candidate.source.clone()),
        optimized_surfaces: optimized_surfaces.clone(),
        promotion_gate: promotion_gate.clone(),
        diff_summary: diff_summary.clone(),
        notes: notes.clone(),
    };
    let lineage_archive_path = append_prompt_fragment_lineage_entry(&project_root, &lineage_entry)
        .await
        .unwrap_or_else(|_| {
            project_root
                .join(PROMPT_FRAGMENT_LINEAGE_ARCHIVE_REL_PATH)
                .display()
                .to_string()
        });

    Ok(PromptFragmentEvolutionResult {
        success: true,
        mode: "prompt_fragment_bundle".to_string(),
        target_key: PROMPT_FRAGMENT_BUNDLE_PROFILE_KEY.to_string(),
        baseline_version: baseline_bundle.version,
        candidate_version: best_candidate.bundle.version.clone(),
        promoted,
        evaluated_candidates,
        baseline_score: round4(baseline_eval.score),
        best_candidate_score: round4(best_eval.score),
        score_gain: round4(score_gain),
        baseline_prompt_efficiency: baseline_eval.efficiency,
        best_candidate_prompt_efficiency: best_eval.efficiency,
        wins,
        losses,
        p_value: 1.0,
        candidate_source: Some(best_candidate.source),
        optimized_surfaces,
        promotion_gate,
        promoted_prompt_fragment_bundle: if promoted {
            Some(best_candidate.bundle)
        } else {
            None
        },
        lineage_entry_id: lineage_entry.entry_id,
        lineage_archive_path,
        notes,
        diff_summary,
        error: None,
    })
}

async fn build_prompt_fragment_noop_result(
    project_root: PathBuf,
    user_request: &str,
    baseline_bundle: &PromptFragmentBundleProfile,
    baseline_eval: &PromptFragmentBundleEvaluation,
) -> Result<PromptFragmentEvolutionResult> {
    let diff_summary = PromptFragmentBundleDiffSummary::default();
    let lineage_entry = PromptFragmentLineageEntry {
        entry_id: format!("pfg-{}", uuid::Uuid::new_v4()),
        timestamp_utc: chrono::Utc::now().to_rfc3339(),
        target_key: PROMPT_FRAGMENT_BUNDLE_PROFILE_KEY.to_string(),
        request: user_request.to_string(),
        baseline_version: baseline_bundle.version.clone(),
        candidate_version: baseline_bundle.version.clone(),
        baseline_bundle_hash: prompt_fragment_bundle_hash(baseline_bundle),
        candidate_bundle_hash: prompt_fragment_bundle_hash(baseline_bundle),
        promoted: false,
        baseline_score: round4(baseline_eval.score),
        candidate_score: round4(baseline_eval.score),
        score_gain: 0.0,
        candidate_source: None,
        optimized_surfaces: Vec::new(),
        promotion_gate: "no distinct prompt-fragment candidates".to_string(),
        diff_summary: diff_summary.clone(),
        notes: vec!["No imported prompt-fragment candidate differed from baseline.".to_string()],
    };
    let lineage_archive_path = append_prompt_fragment_lineage_entry(&project_root, &lineage_entry)
        .await
        .unwrap_or_else(|_| {
            project_root
                .join(PROMPT_FRAGMENT_LINEAGE_ARCHIVE_REL_PATH)
                .display()
                .to_string()
        });
    Ok(PromptFragmentEvolutionResult {
        success: true,
        mode: "prompt_fragment_bundle".to_string(),
        target_key: PROMPT_FRAGMENT_BUNDLE_PROFILE_KEY.to_string(),
        baseline_version: baseline_bundle.version.clone(),
        candidate_version: baseline_bundle.version.clone(),
        promoted: false,
        evaluated_candidates: 0,
        baseline_score: round4(baseline_eval.score),
        best_candidate_score: round4(baseline_eval.score),
        score_gain: 0.0,
        baseline_prompt_efficiency: baseline_eval.efficiency,
        best_candidate_prompt_efficiency: baseline_eval.efficiency,
        wins: 0,
        losses: 0,
        p_value: 1.0,
        candidate_source: None,
        optimized_surfaces: Vec::new(),
        promotion_gate: lineage_entry.promotion_gate,
        promoted_prompt_fragment_bundle: None,
        lineage_entry_id: lineage_entry.entry_id,
        lineage_archive_path,
        notes: lineage_entry.notes,
        diff_summary,
        error: None,
    })
}

fn evaluate_prompt_fragment_bundle(
    bundle: &PromptFragmentBundleProfile,
) -> PromptFragmentBundleEvaluation {
    let efficiency = prompt_fragment_bundle_efficiency(bundle);
    let enabled = bundle
        .fragments
        .iter()
        .filter(|fragment| fragment.enabled && !fragment.body.trim().is_empty())
        .collect::<Vec<_>>();
    let enabled_ids = enabled
        .iter()
        .map(|fragment| fragment.id.as_str())
        .collect::<BTreeSet<_>>();
    let disabled_required_fragment_ids = required_prompt_fragment_ids()
        .iter()
        .filter(|id| !enabled_ids.contains(**id))
        .map(|id| (*id).to_string())
        .collect::<Vec<_>>();
    let invariant_checks = 2usize;
    let passed_invariant_checks =
        usize::from(disabled_required_fragment_ids.is_empty()) + usize::from(!enabled.is_empty());
    let score = passed_invariant_checks as f64 / invariant_checks as f64;

    PromptFragmentBundleEvaluation {
        score: round4(score),
        required_fragments_enabled: disabled_required_fragment_ids.is_empty(),
        has_enabled_fragments: efficiency.enabled_fragments > 0,
        efficiency,
        disabled_required_fragment_ids,
    }
}

fn prompt_fragment_bundle_efficiency(
    bundle: &PromptFragmentBundleProfile,
) -> PromptFragmentEfficiencyMetrics {
    let enabled = bundle
        .fragments
        .iter()
        .filter(|fragment| fragment.enabled)
        .collect::<Vec<_>>();
    PromptFragmentEfficiencyMetrics {
        total_fragments: bundle.fragments.len(),
        enabled_fragments: enabled.len(),
        always_on_fragments: enabled.iter().filter(|fragment| fragment.always_on).count(),
        enabled_estimated_tokens: enabled
            .iter()
            .map(|fragment| fragment.est_tokens.max(estimate_tokens(&fragment.body)))
            .sum(),
        always_on_estimated_tokens: enabled
            .iter()
            .filter(|fragment| fragment.always_on)
            .map(|fragment| fragment.est_tokens.max(estimate_tokens(&fragment.body)))
            .sum(),
        max_fragment_estimated_tokens: enabled
            .iter()
            .map(|fragment| fragment.est_tokens.max(estimate_tokens(&fragment.body)))
            .max()
            .unwrap_or_default(),
    }
}

fn build_prompt_fragment_bundle_diff_summary(
    baseline: &PromptFragmentBundleProfile,
    candidate: &PromptFragmentBundleProfile,
) -> PromptFragmentBundleDiffSummary {
    let baseline_by_id = baseline
        .fragments
        .iter()
        .map(|fragment| (fragment.id.clone(), fragment))
        .collect::<BTreeMap<_, _>>();
    let candidate_by_id = candidate
        .fragments
        .iter()
        .map(|fragment| (fragment.id.clone(), fragment))
        .collect::<BTreeMap<_, _>>();

    let mut added_fragment_ids = Vec::new();
    let mut removed_fragment_ids = Vec::new();
    let mut changed_fragment_ids = Vec::new();
    let mut changed_surfaces = BTreeSet::new();
    let mut change_preview = Vec::new();

    for (id, fragment) in &candidate_by_id {
        if !baseline_by_id.contains_key(id) {
            added_fragment_ids.push(id.clone());
            changed_surfaces.insert(fragment.surface.clone());
            push_preview(&mut change_preview, format!("Added fragment {}", id));
        }
    }
    for (id, fragment) in &baseline_by_id {
        if !candidate_by_id.contains_key(id) {
            removed_fragment_ids.push(id.clone());
            changed_surfaces.insert(fragment.surface.clone());
            push_preview(&mut change_preview, format!("Removed fragment {}", id));
        }
    }
    for (id, candidate_fragment) in &candidate_by_id {
        let Some(baseline_fragment) = baseline_by_id.get(id) else {
            continue;
        };
        if prompt_fragment_changed(baseline_fragment, candidate_fragment) {
            changed_fragment_ids.push(id.clone());
            changed_surfaces.insert(candidate_fragment.surface.clone());
            push_preview(&mut change_preview, format!("Changed fragment {}", id));
        }
    }

    let disabled_required_fragment_ids = required_prompt_fragment_ids()
        .iter()
        .filter(|id| {
            candidate_by_id
                .get(**id)
                .map(|fragment| !fragment.enabled || fragment.body.trim().is_empty())
                .unwrap_or(true)
        })
        .map(|id| (*id).to_string())
        .collect::<Vec<_>>();

    PromptFragmentBundleDiffSummary {
        added_fragment_ids,
        removed_fragment_ids,
        changed_fragment_ids,
        disabled_required_fragment_ids,
        changed_surfaces: changed_surfaces.into_iter().collect(),
        change_preview,
    }
}

fn prompt_fragment_changed(left: &PromptFragment, right: &PromptFragment) -> bool {
    left.surface != right.surface
        || left.body != right.body
        || left.scope_tags != right.scope_tags
        || left.always_on != right.always_on
        || left.priority != right.priority
        || left.enabled != right.enabled
}

fn render_prompt_fragment_promotion_gate(
    promoted: bool,
    changed: bool,
    required_fragments_enabled: bool,
    has_enabled_fragments: bool,
    token_regression_ratio: f64,
) -> String {
    if promoted {
        return "passed structural prompt-fragment canary gate".to_string();
    }
    if !changed {
        return "candidate did not change the prompt-fragment bundle".to_string();
    }
    if !has_enabled_fragments {
        return "candidate has no enabled prompt fragments".to_string();
    }
    if !required_fragments_enabled {
        return "candidate disables a required baseline prompt fragment".to_string();
    }
    if token_regression_ratio > MAX_FRAGMENT_TOKEN_REGRESSION_RATIO {
        return format!(
            "enabled prompt-fragment tokens grew by {:.1}% above the {:.1}% limit",
            token_regression_ratio * 100.0,
            MAX_FRAGMENT_TOKEN_REGRESSION_RATIO * 100.0
        );
    }
    "candidate did not pass prompt-fragment canary gate".to_string()
}

fn build_prompt_fragment_notes(
    baseline_eval: &PromptFragmentBundleEvaluation,
    candidate_eval: &PromptFragmentBundleEvaluation,
    token_regression_ratio: f64,
    diff_summary: &PromptFragmentBundleDiffSummary,
) -> Vec<String> {
    let mut notes = Vec::new();
    if candidate_eval.required_fragments_enabled {
        notes.push("Required baseline prompt fragments remain enabled.".to_string());
    } else if !candidate_eval.disabled_required_fragment_ids.is_empty() {
        notes.push(format!(
            "Required prompt fragments disabled: {}.",
            candidate_eval.disabled_required_fragment_ids.join(", ")
        ));
    }
    let token_delta = candidate_eval
        .efficiency
        .enabled_estimated_tokens
        .saturating_sub(baseline_eval.efficiency.enabled_estimated_tokens);
    if candidate_eval.efficiency.enabled_estimated_tokens
        <= baseline_eval.efficiency.enabled_estimated_tokens
    {
        notes.push(
            "Candidate does not increase enabled prompt-fragment token estimate.".to_string(),
        );
    } else {
        notes.push(format!(
            "Candidate adds about {} enabled prompt-fragment token(s), {:.1}% over baseline.",
            token_delta,
            token_regression_ratio * 100.0
        ));
    }
    if !diff_summary.changed_surfaces.is_empty() {
        notes.push(format!(
            "Changed surfaces: {}.",
            diff_summary.changed_surfaces.join(", ")
        ));
    }
    notes
}

async fn append_prompt_fragment_lineage_entry(
    project_root: &std::path::Path,
    entry: &PromptFragmentLineageEntry,
) -> Result<String> {
    let path = project_root.join(PROMPT_FRAGMENT_LINEAGE_ARCHIVE_REL_PATH);
    if let Some(parent) = path.parent() {
        tokio::fs::create_dir_all(parent).await?;
    }
    let mut file = tokio::fs::OpenOptions::new()
        .create(true)
        .append(true)
        .open(&path)
        .await?;
    let line = serde_json::to_string(entry)?;
    file.write_all(line.as_bytes()).await?;
    file.write_all(b"\n").await?;
    super::prune_jsonl_archive(&path, MAX_LINEAGE_ARCHIVE_ENTRIES).await?;
    Ok(path.display().to_string())
}

fn token_regression_ratio(baseline_tokens: usize, candidate_tokens: usize) -> f64 {
    if candidate_tokens <= baseline_tokens {
        0.0
    } else if baseline_tokens == 0 {
        1.0
    } else {
        (candidate_tokens - baseline_tokens) as f64 / baseline_tokens as f64
    }
}

fn estimate_tokens(text: &str) -> usize {
    text.chars().count().saturating_add(3) / 4
}

fn push_preview(preview: &mut Vec<String>, value: String) {
    if preview.len() < 8 {
        preview.push(value);
    }
}

fn prompt_fragment_bundle_hash(bundle: &PromptFragmentBundleProfile) -> String {
    let mut hasher = std::collections::hash_map::DefaultHasher::new();
    serde_json::to_string(bundle)
        .unwrap_or_default()
        .hash(&mut hasher);
    format!("{:016x}", hasher.finish())
}

fn short_hash(parts: &[&str]) -> String {
    let mut hasher = std::collections::hash_map::DefaultHasher::new();
    for part in parts {
        part.hash(&mut hasher);
    }
    format!("{:08x}", hasher.finish())
}

fn round4(value: f64) -> f64 {
    (value * 10_000.0).round() / 10_000.0
}

pub(crate) fn prompt_fragment_candidate_benchmark_profile() -> serde_json::Value {
    serde_json::json!({
        "target_key": PROMPT_FRAGMENT_BUNDLE_PROFILE_KEY,
        "surface": "prompt_fragment_bundle",
        "objective": "Improve selectable agent-loop guidance fragments while preserving required safety and turn-contract invariants.",
        "required_invariants": required_prompt_fragment_ids(),
        "semantic_contract": {
            "preserve_multi_outcome_turns": true,
            "conversation_history_resolves_references_without_overriding_current_turn": true,
            "memory_capture_remains_deferred_after_chat_persistence": true,
            "avoid_phrase_specific_or_magic_bonus_routing": true
        },
        "candidate_shape": {
            "version": "string",
            "updated_at": "optional RFC3339 timestamp",
            "fragments": [{
                "id": "stable internal fragment id",
                "surface": "agent_loop or all",
                "body": "fragment guidance text",
                "scope_tags": ["semantic routing tags"],
                "always_on": false,
                "priority": 0,
                "enabled": true
            }]
        },
        "canary_gate": {
            "preserve_required_fragments": true,
            "max_enabled_token_regression_ratio": MAX_FRAGMENT_TOKEN_REGRESSION_RATIO,
            "reject_empty_enabled_bundle": true
        }
    })
}
