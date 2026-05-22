//! Self-Evolve: policy-first behavior evolution engine
//!
//! Default path evolves runtime strategy/policy with benchmark + lineage + gates.
//! Source-code mutation is intentionally not available on user machines.

use anyhow::Result;
use std::path::Path;

pub mod gepa_bridge;
pub mod policy_evolution;
pub mod promotion_gate;
pub mod prompt_evolution;
pub(crate) mod prompt_fragment_evolution;
pub mod replay_gate;
pub mod router_learning;
pub mod routing_canonical_evolution;
pub mod specialist_prompt_evolution;
pub mod strategy_runtime;

pub use policy_evolution::{
    PolicyEvolutionConfig, PolicyEvolutionEngine, ROUTING_COMPLEXITY_POLICY_KEY,
};
pub use promotion_gate::{PromotionGateReason, PromotionGateReport};
pub use prompt_evolution::{
    PROMPT_BUNDLE_BASELINE_SNAPSHOT_KEY, PROMPT_BUNDLE_CANARY_STATE_KEY,
    PROMPT_BUNDLE_LAST_RESULT_KEY, PROMPT_BUNDLE_PROFILE_CANARY_KEY, PROMPT_BUNDLE_PROFILE_KEY,
    PromptBundleDiffSummary, PromptBundleProfile, PromptEvolutionConfig, PromptEvolutionEngine,
    PromptEvolutionResult,
};
pub(crate) use router_learning::maybe_upsert_router_replay_candidate_from_trace;
pub use router_learning::{
    ROUTER_LEARNING_CANDIDATE_TYPE, ROUTER_LEARNING_SUBJECT_KEY, RouterLearningCandidatePayload,
    RouterLearningLayer, RouterLearningMetric, RouterLearningMetricDelta,
    RouterLearningTraceEvidence, router_learning_benchmark_profile,
    trace_evidence_from_semantic_steps, validate_router_learning_candidate,
};
pub use routing_canonical_evolution::{
    ROUTING_CANONICAL_CANDIDATE_TYPE, ROUTING_CANONICAL_SUBJECT_KEY,
    RoutingCanonicalCandidatePayload, RoutingCanonicalOverlayEntry,
};
pub use specialist_prompt_evolution::{
    SPECIALIST_PROMPT_BUNDLE_BASELINE_SNAPSHOT_KEY, SPECIALIST_PROMPT_BUNDLE_CANARY_STATE_KEY,
    SPECIALIST_PROMPT_BUNDLE_LAST_RESULT_KEY, SPECIALIST_PROMPT_BUNDLE_PROFILE_CANARY_KEY,
    SPECIALIST_PROMPT_BUNDLE_PROFILE_KEY, SpecialistPromptBundleDiffSummary,
    SpecialistPromptBundleProfile, SpecialistPromptEvolutionConfig,
    SpecialistPromptEvolutionEngine, SpecialistPromptEvolutionResult,
};

pub(crate) async fn prune_jsonl_archive(archive: &Path, max_entries: usize) -> Result<()> {
    if max_entries == 0 {
        return Ok(());
    }
    let raw = match tokio::fs::read_to_string(archive).await {
        Ok(content) => content,
        Err(_) => return Ok(()),
    };
    let mut lines = raw
        .lines()
        .filter(|line| !line.trim().is_empty())
        .map(|line| line.to_string())
        .collect::<Vec<_>>();
    if lines.len() <= max_entries {
        return Ok(());
    }
    let retained = lines.split_off(lines.len().saturating_sub(max_entries));
    let mut rewritten = retained.join("\n");
    rewritten.push('\n');
    tokio::fs::write(archive, rewritten).await?;
    Ok(())
}
