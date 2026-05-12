//! Self-Evolve: policy-first and code-evolution engine
//!
//! Default path evolves runtime strategy/policy with benchmark + lineage + gates.
//! Codebase mutation remains available behind explicit opt-in.

use anyhow::Result;
use std::path::Path;

pub mod agent;
pub mod coding_guidelines;
pub mod gepa_bridge;
pub mod policy_evolution;
pub mod promotion_gate;
pub mod prompt_evolution;
pub(crate) mod prompt_fragment_evolution;
pub mod replay_gate;
pub mod router_learning;
pub mod routing_canonical_evolution;
pub mod security_review;
pub mod skill_evolution;
pub mod specialist_prompt_evolution;
pub mod strategy_runtime;
pub mod tools;

pub use agent::{SelfEvolveAgent, SelfEvolveConfig};
pub use policy_evolution::{
    PolicyEvolutionConfig, PolicyEvolutionEngine, ROUTING_COMPLEXITY_POLICY_KEY,
};
pub use promotion_gate::{PromotionGateReason, PromotionGateReport};
pub use prompt_evolution::{
    PromptBundleDiffSummary, PromptBundleProfile, PromptEvolutionConfig, PromptEvolutionEngine,
    PromptEvolutionResult, PROMPT_BUNDLE_BASELINE_SNAPSHOT_KEY, PROMPT_BUNDLE_CANARY_STATE_KEY,
    PROMPT_BUNDLE_LAST_RESULT_KEY, PROMPT_BUNDLE_PROFILE_CANARY_KEY, PROMPT_BUNDLE_PROFILE_KEY,
};
pub use routing_canonical_evolution::{
    RoutingCanonicalCandidatePayload, RoutingCanonicalOverlayEntry,
    ROUTING_CANONICAL_CANDIDATE_TYPE, ROUTING_CANONICAL_SUBJECT_KEY,
};
pub(crate) use router_learning::maybe_upsert_router_replay_candidate_from_trace;
pub use router_learning::{
    router_learning_benchmark_profile, trace_evidence_from_semantic_steps,
    validate_router_learning_candidate, RouterLearningCandidatePayload, RouterLearningLayer,
    RouterLearningMetric, RouterLearningMetricDelta, RouterLearningTraceEvidence,
    ROUTER_LEARNING_CANDIDATE_TYPE, ROUTER_LEARNING_SUBJECT_KEY,
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
