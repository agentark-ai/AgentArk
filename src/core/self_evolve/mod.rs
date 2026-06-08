//! Self-Evolve: policy-first behavior evolution engine
//!
//! Default path evolves runtime strategy/policy with benchmark + lineage + gates.
//! Source-code mutation is intentionally not available on user machines.

use anyhow::Result;
use std::path::Path;

#[path = "gates/deployment_cadence.rs"]
pub mod deployment_cadence;
#[path = "optimizer/gepa_bridge.rs"]
pub mod gepa_bridge;
#[path = "policies/policy_evolution.rs"]
pub mod policy_evolution;
#[path = "gates/promotion_gate.rs"]
pub mod promotion_gate;
#[path = "prompts/prompt_evolution.rs"]
pub mod prompt_evolution;
#[path = "prompts/prompt_fragment_evolution.rs"]
pub(crate) mod prompt_fragment_evolution;
#[path = "gates/replay_gate.rs"]
pub mod replay_gate;
#[path = "routing/router_learning.rs"]
pub mod router_learning;
#[path = "routing/routing_canonical_evolution.rs"]
pub mod routing_canonical_evolution;
#[path = "runtime/self_tune.rs"]
pub mod self_tune;
#[path = "prompts/specialist_prompt_evolution.rs"]
pub mod specialist_prompt_evolution;
#[path = "runtime/strategy_runtime.rs"]
pub mod strategy_runtime;

pub use policy_evolution::{
    PolicyEvolutionConfig, PolicyEvolutionEngine, ROUTING_COMPLEXITY_POLICY_KEY,
};
pub use promotion_gate::{PromotionGateReason, PromotionGateReport};
pub use prompt_evolution::{
    PromptBundleDiffSummary, PromptBundleProfile, PromptEvolutionConfig, PromptEvolutionEngine,
    PromptEvolutionResult, PROMPT_BUNDLE_BASELINE_SNAPSHOT_KEY, PROMPT_BUNDLE_CANARY_STATE_KEY,
    PROMPT_BUNDLE_LAST_RESULT_KEY, PROMPT_BUNDLE_PROFILE_CANARY_KEY, PROMPT_BUNDLE_PROFILE_KEY,
};
pub(crate) use router_learning::maybe_upsert_router_replay_candidate_from_trace;
pub use router_learning::{
    router_learning_benchmark_profile, trace_evidence_from_semantic_steps,
    validate_router_learning_candidate, RouterLearningCandidatePayload, RouterLearningLayer,
    RouterLearningMetric, RouterLearningMetricDelta, RouterLearningTraceEvidence,
    ROUTER_LEARNING_CANDIDATE_TYPE, ROUTER_LEARNING_SUBJECT_KEY,
};
pub use routing_canonical_evolution::{
    RoutingCanonicalCandidatePayload, RoutingCanonicalOverlayEntry,
    ROUTING_CANONICAL_CANDIDATE_TYPE, ROUTING_CANONICAL_SUBJECT_KEY,
};
pub use specialist_prompt_evolution::{
    SpecialistPromptBundleDiffSummary, SpecialistPromptBundleProfile,
    SpecialistPromptEvolutionConfig, SpecialistPromptEvolutionEngine,
    SpecialistPromptEvolutionResult, SPECIALIST_PROMPT_BUNDLE_BASELINE_SNAPSHOT_KEY,
    SPECIALIST_PROMPT_BUNDLE_CANARY_STATE_KEY, SPECIALIST_PROMPT_BUNDLE_LAST_RESULT_KEY,
    SPECIALIST_PROMPT_BUNDLE_PROFILE_CANARY_KEY, SPECIALIST_PROMPT_BUNDLE_PROFILE_KEY,
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
