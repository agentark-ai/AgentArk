//! Prompt-bundle self-evolution engine.
//!
//! Optimizes three mutable prompt surfaces:
//! - router decision prompt
//! - primary response prompt
//! - delegated-result synthesis prompt
//!
//! The optimizer remains inside AgentArk's Rust control plane so canary rollout,
//! lineage, replay gates, and UI visibility stay consistent with the existing
//! self-evolve workflow.

use anyhow::{Context, Result};
use serde::{Deserialize, Serialize};
use std::collections::{BTreeSet, HashMap, HashSet};
use std::hash::{Hash, Hasher};
use std::path::PathBuf;

use crate::actions::{ActionDef, ActionSource};
use crate::core::agent::QueryComplexity;
use crate::core::llm::{LlmClient, LlmResponse, ToolCall};
use crate::core::prompt_policy::{
    primary_response_instruction_template_v1, primary_response_policy_v1,
    primary_response_system_prompt_v1, router_instruction_template_v1, router_policy_v2_block,
    router_system_prompt_v1, synthesis_instruction_template_v1, synthesis_policy_v2_block,
    synthesis_system_prompt_v1,
};
use crate::core::task_router::{AgentExecResult, RoutingDecision};
use crate::core::{DelegationStatus, FailureKind};

pub const PROMPT_BUNDLE_PROFILE_KEY: &str = "prompt_bundle_profile_v1";
pub const PROMPT_BUNDLE_PROFILE_CANARY_KEY: &str = "prompt_bundle_profile_canary_v1";
pub const PROMPT_BUNDLE_CANARY_STATE_KEY: &str = "prompt_bundle_canary_state_v1";
pub const PROMPT_BUNDLE_BASELINE_SNAPSHOT_KEY: &str = "prompt_bundle_baseline_snapshot_v1";
pub const PROMPT_BUNDLE_LAST_RESULT_KEY: &str = "prompt_bundle_last_result_v1";
pub const BASE_SYSTEM_PROMPT_VERSION: &str = "system_prompt_v2";

const PROMPT_BUNDLE_DEFAULT_VERSION: &str = "prompt-bundle-default-v1";
const LINEAGE_ARCHIVE_REL_PATH: &str = ".agentark/self_evolve/prompt_bundle_lineage.jsonl";
const BENCHMARK_PROFILE_REL_PATH: &str = "assets/self_evolve/prompt_bundle_benchmark_v1.json";
const DEFAULT_RECENT_LINEAGE_LIMIT: usize = 12;
const MAX_LINEAGE_ARCHIVE_ENTRIES: usize = 400;
const MAX_SURFACE_CHARS: usize = 16_000;
const ROUTER_WEIGHT: f64 = 0.35;
const SYNTHESIS_WEIGHT: f64 = 0.25;
const PRIMARY_RESPONSE_WEIGHT: f64 = 0.40;

const ROUTER_DIRECTNESS_MUTATION: &str = r#"- Prefer direct execution when one action clearly matches the request.
- Require at least 2 usable sub-agents before needs_delegation=true.
- Keep should_clarify=false when the request is concrete and executable."#;

const SYNTHESIS_TOOL_PRESERVATION_MUTATION: &str = r#"- Preserve or recover the clearest required tool call from delegated outputs when the task still maps to an available action.
- If any delegated path degraded, explicitly separate completed work from follow-up work.
- Keep the final answer concise, user-facing, and operationally honest."#;

const PRIMARY_RESPONSE_COMPLETION_MUTATION: &str = r#"- Prefer concrete completion status over abstract explanation.
- If work is complete, say what changed and the most important caveat.
- If blocked, name the blocker and the safest next step briefly."#;

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct PromptSurfaceProfile {
    pub system_prompt: String,
    pub policy_block: String,
    pub instruction_template: String,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct PromptBundleProfile {
    pub version: String,
    #[serde(default)]
    pub updated_at: Option<String>,
    pub router: PromptSurfaceProfile,
    #[serde(default = "default_primary_response_prompt_surface")]
    pub primary_response: PromptSurfaceProfile,
    pub delegation_synthesis: PromptSurfaceProfile,
}

#[derive(Debug, Clone, Serialize, Deserialize, Default)]
pub struct PromptBundleDiffSummary {
    #[serde(default)]
    pub router_changed_fields: Vec<String>,
    #[serde(default)]
    pub primary_response_changed_fields: Vec<String>,
    #[serde(default)]
    pub delegation_synthesis_changed_fields: Vec<String>,
    #[serde(default)]
    pub router_change_preview: Vec<String>,
    #[serde(default)]
    pub primary_response_change_preview: Vec<String>,
    #[serde(default)]
    pub delegation_synthesis_change_preview: Vec<String>,
}

#[derive(Debug, Clone)]
pub struct PromptEvolutionConfig {
    pub project_root: PathBuf,
    pub max_candidates: usize,
    pub min_score_gain: f64,
    pub max_sign_test_p_value: f64,
}

impl Default for PromptEvolutionConfig {
    fn default() -> Self {
        Self {
            project_root: PathBuf::from("."),
            max_candidates: 8,
            min_score_gain: 0.03,
            max_sign_test_p_value: 0.10,
        }
    }
}

#[derive(Debug, Clone, Serialize)]
pub struct PromptEvolutionResult {
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
    pub baseline_router_score: f64,
    pub best_candidate_router_score: f64,
    pub baseline_primary_response_score: f64,
    pub best_candidate_primary_response_score: f64,
    pub baseline_synthesis_score: f64,
    pub best_candidate_synthesis_score: f64,
    pub baseline_router_invalid_json_rate: f64,
    pub candidate_router_invalid_json_rate: f64,
    pub wins: usize,
    pub losses: usize,
    pub p_value: f64,
    pub candidate_source: Option<String>,
    pub optimized_surfaces: Vec<String>,
    pub promotion_gate: String,
    pub promoted_prompt_bundle: Option<PromptBundleProfile>,
    pub lineage_entry_id: String,
    pub lineage_archive_path: String,
    pub notes: Vec<String>,
    pub diff_summary: PromptBundleDiffSummary,
    pub error: Option<String>,
}

impl Default for PromptBundleProfile {
    fn default() -> Self {
        let mut bundle = Self {
            version: PROMPT_BUNDLE_DEFAULT_VERSION.to_string(),
            updated_at: None,
            router: default_router_prompt_surface(),
            primary_response: default_primary_response_prompt_surface(),
            delegation_synthesis: default_synthesis_prompt_surface(),
        };
        sanitize_prompt_bundle(&mut bundle);
        bundle
    }
}

pub fn default_router_prompt_surface() -> PromptSurfaceProfile {
    PromptSurfaceProfile {
        system_prompt: router_system_prompt_v1(),
        policy_block: router_policy_v2_block(),
        instruction_template: router_instruction_template_v1(),
    }
}

pub fn default_synthesis_prompt_surface() -> PromptSurfaceProfile {
    PromptSurfaceProfile {
        system_prompt: synthesis_system_prompt_v1(),
        policy_block: synthesis_policy_v2_block(),
        instruction_template: synthesis_instruction_template_v1(),
    }
}

pub fn default_primary_response_prompt_surface() -> PromptSurfaceProfile {
    PromptSurfaceProfile {
        system_prompt: primary_response_system_prompt_v1(),
        policy_block: primary_response_policy_v1(),
        instruction_template: primary_response_instruction_template_v1(),
    }
}

pub fn parse_prompt_bundle_profile(raw: &[u8]) -> Option<PromptBundleProfile> {
    let mut bundle = serde_json::from_slice::<PromptBundleProfile>(raw).ok()?;
    sanitize_prompt_bundle(&mut bundle);
    Some(bundle)
}

pub fn compose_prompt_version(bundle_version: &str) -> String {
    format!("{}+{}", BASE_SYSTEM_PROMPT_VERSION, bundle_version.trim())
}

pub fn sanitize_prompt_bundle(bundle: &mut PromptBundleProfile) {
    if bundle.version.trim().is_empty() {
        bundle.version = PROMPT_BUNDLE_DEFAULT_VERSION.to_string();
    } else {
        bundle.version = truncate_chars(bundle.version.trim(), 128);
    }
    sanitize_surface(
        &mut bundle.router,
        &default_router_prompt_surface(),
        &[
            "{specialists}",
            "{policy_block}",
            "{policy_hint}",
            "{action_hints}",
            "{preferred_action}",
            "{message}",
        ],
    );
    sanitize_surface(
        &mut bundle.primary_response,
        &default_primary_response_prompt_surface(),
        &[],
    );
    sanitize_surface(
        &mut bundle.delegation_synthesis,
        &default_synthesis_prompt_surface(),
        &["{original_task}", "{results_text}"],
    );
}

pub struct RouterPromptRenderInputs<'a> {
    pub specialists: &'a str,
    pub policy_hint: &'a str,
    pub action_hints: &'a str,
    pub preferred_action: &'a str,
    pub message: &'a str,
}

pub struct SynthesisPromptRenderInputs<'a> {
    pub original_task: &'a str,
    pub results_text: &'a str,
}

pub fn render_router_system_prompt(bundle: &PromptBundleProfile) -> String {
    render_system_prompt(&bundle.router)
}

pub fn render_router_user_prompt(
    bundle: &PromptBundleProfile,
    inputs: &RouterPromptRenderInputs<'_>,
) -> String {
    render_template(
        &bundle.router.instruction_template,
        &[
            ("specialists", inputs.specialists),
            ("policy_block", bundle.router.policy_block.as_str()),
            ("policy_hint", inputs.policy_hint),
            ("action_hints", inputs.action_hints),
            ("preferred_action", inputs.preferred_action),
            ("message", inputs.message),
        ],
    )
}

pub fn render_synthesis_system_prompt(bundle: &PromptBundleProfile) -> String {
    render_system_prompt(&bundle.delegation_synthesis)
}

pub fn render_primary_response_system_prompt(bundle: &PromptBundleProfile) -> String {
    let mut combined = render_system_prompt(&bundle.primary_response);
    let instruction =
        interpolate_runtime_tokens(bundle.primary_response.instruction_template.trim());
    if !instruction.is_empty() {
        combined.push_str("\n\n");
        combined.push_str(&instruction);
    }
    combined
}

pub fn render_synthesis_user_prompt(
    bundle: &PromptBundleProfile,
    inputs: &SynthesisPromptRenderInputs<'_>,
) -> String {
    render_template(
        &bundle.delegation_synthesis.instruction_template,
        &[
            ("original_task", inputs.original_task),
            ("results_text", inputs.results_text),
        ],
    )
}

pub struct PromptEvolutionEngine {
    config: PromptEvolutionConfig,
    llm: LlmClient,
}

impl PromptEvolutionEngine {
    pub fn new(config: PromptEvolutionConfig, llm: LlmClient) -> Self {
        Self { config, llm }
    }

    pub async fn evolve_prompt_bundle(
        &self,
        user_request: &str,
        current_bundle_raw: Option<&[u8]>,
    ) -> Result<PromptEvolutionResult> {
        let baseline_bundle = self.load_baseline_bundle(current_bundle_raw)?;
        let benchmark = self.load_benchmark_suite().await?;
        let baseline_eval = self.evaluate_bundle(&baseline_bundle, &benchmark).await;
        let recent_lineage = self.load_recent_lineage(DEFAULT_RECENT_LINEAGE_LIMIT).await;

        let mut candidates = self.deterministic_candidates(&baseline_bundle);
        candidates.extend(
            self.generate_llm_candidates(
                user_request,
                &baseline_bundle,
                &baseline_eval,
                &recent_lineage,
            )
            .await,
        );

        let max_candidates = self.config.max_candidates.max(1);
        if candidates.len() > max_candidates {
            candidates.truncate(max_candidates);
        }

        let baseline_hash = prompt_bundle_hash(&baseline_bundle);
        let mut seen_hashes: HashSet<String> = HashSet::new();
        candidates.retain(|candidate| {
            let hash = prompt_bundle_hash(&candidate.bundle);
            hash != baseline_hash && seen_hashes.insert(hash)
        });

        let evaluated_candidates = candidates.len();
        if evaluated_candidates == 0 {
            return self
                .build_noop_result(
                    user_request,
                    &baseline_bundle,
                    &baseline_eval,
                    baseline_hash,
                )
                .await;
        }

        let mut best: Option<(CandidatePromptBundle, BundleEvaluation, PairedStats)> = None;
        for candidate in candidates {
            let eval = self.evaluate_bundle(&candidate.bundle, &benchmark).await;
            let paired = paired_stats(&baseline_eval.case_scores, &eval.case_scores);
            let replace = match &best {
                None => true,
                Some((_, best_eval, best_paired)) => {
                    eval.combined_score > best_eval.combined_score
                        || (f64_eq(eval.combined_score, best_eval.combined_score)
                            && paired.wins > best_paired.wins)
                        || (f64_eq(eval.combined_score, best_eval.combined_score)
                            && paired.wins == best_paired.wins
                            && paired.p_value < best_paired.p_value)
                }
            };
            if replace {
                best = Some((candidate, eval, paired));
            }
        }

        let (mut best_candidate, best_eval, best_stats) =
            best.context("no best prompt-bundle candidate found")?;
        let diff_summary =
            build_prompt_bundle_diff_summary(&baseline_bundle, &best_candidate.bundle);
        let optimized_surfaces = collect_optimized_surfaces(&diff_summary);
        let candidate_version = format!("prompt-candidate-{}", uuid::Uuid::new_v4().simple());
        best_candidate.bundle.version = candidate_version.clone();
        best_candidate.bundle.updated_at = Some(chrono::Utc::now().to_rfc3339());

        let promotion_checks =
            prompt_promotion_checks(&self.config, &baseline_eval, &best_eval, &best_stats);
        let promoted = promotion_checks.values().all(|passed| *passed);
        let promotion_gate = render_prompt_promotion_gate(&promotion_checks);
        let notes = build_prompt_notes(
            &baseline_eval,
            &best_eval,
            &best_stats,
            &best_candidate.source,
            &diff_summary,
        );

        let lineage_entry = PromptLineageEntry {
            entry_id: format!("prm-{}", uuid::Uuid::new_v4()),
            timestamp_utc: chrono::Utc::now().to_rfc3339(),
            target_key: PROMPT_BUNDLE_PROFILE_KEY.to_string(),
            request: user_request.to_string(),
            baseline_version: baseline_bundle.version.clone(),
            candidate_version: candidate_version.clone(),
            baseline_bundle_hash: prompt_bundle_hash(&baseline_bundle),
            candidate_bundle_hash: prompt_bundle_hash(&best_candidate.bundle),
            baseline_score: round4(baseline_eval.combined_score),
            candidate_score: round4(best_eval.combined_score),
            score_gain: round4(best_stats.score_gain),
            baseline_router_score: round4(baseline_eval.router_score),
            candidate_router_score: round4(best_eval.router_score),
            baseline_primary_response_score: round4(baseline_eval.primary_response_score),
            candidate_primary_response_score: round4(best_eval.primary_response_score),
            baseline_synthesis_score: round4(baseline_eval.synthesis_score),
            candidate_synthesis_score: round4(best_eval.synthesis_score),
            baseline_router_invalid_json_rate: round4(baseline_eval.router_invalid_json_rate),
            candidate_router_invalid_json_rate: round4(best_eval.router_invalid_json_rate),
            wins: best_stats.wins,
            losses: best_stats.losses,
            p_value: round4(best_stats.p_value),
            promoted,
            candidate_source: best_candidate.source.clone(),
            optimized_surfaces: optimized_surfaces.clone(),
            notes: notes.clone(),
            diff_summary: diff_summary.clone(),
        };
        let lineage_entry_id = self
            .append_lineage_entry(&lineage_entry)
            .await
            .unwrap_or_else(|_| "lineage-write-failed".to_string());

        Ok(PromptEvolutionResult {
            success: true,
            mode: "prompt".to_string(),
            target_key: PROMPT_BUNDLE_PROFILE_KEY.to_string(),
            baseline_version: baseline_bundle.version,
            candidate_version,
            promoted,
            evaluated_candidates,
            baseline_score: round4(baseline_eval.combined_score),
            best_candidate_score: round4(best_eval.combined_score),
            score_gain: round4(best_stats.score_gain),
            baseline_router_score: round4(baseline_eval.router_score),
            best_candidate_router_score: round4(best_eval.router_score),
            baseline_primary_response_score: round4(baseline_eval.primary_response_score),
            best_candidate_primary_response_score: round4(best_eval.primary_response_score),
            baseline_synthesis_score: round4(baseline_eval.synthesis_score),
            best_candidate_synthesis_score: round4(best_eval.synthesis_score),
            baseline_router_invalid_json_rate: round4(baseline_eval.router_invalid_json_rate),
            candidate_router_invalid_json_rate: round4(best_eval.router_invalid_json_rate),
            wins: best_stats.wins,
            losses: best_stats.losses,
            p_value: round4(best_stats.p_value),
            candidate_source: Some(best_candidate.source),
            optimized_surfaces,
            promotion_gate,
            promoted_prompt_bundle: if promoted {
                Some(best_candidate.bundle)
            } else {
                None
            },
            lineage_entry_id,
            lineage_archive_path: self.archive_path().display().to_string(),
            notes,
            diff_summary,
            error: None,
        })
    }

    async fn build_noop_result(
        &self,
        user_request: &str,
        baseline_bundle: &PromptBundleProfile,
        baseline_eval: &BundleEvaluation,
        baseline_hash: String,
    ) -> Result<PromptEvolutionResult> {
        let diff_summary = PromptBundleDiffSummary::default();
        let entry_id = self
            .append_lineage_entry(&PromptLineageEntry {
                entry_id: format!("prm-{}", uuid::Uuid::new_v4()),
                timestamp_utc: chrono::Utc::now().to_rfc3339(),
                target_key: PROMPT_BUNDLE_PROFILE_KEY.to_string(),
                request: user_request.to_string(),
                baseline_version: baseline_bundle.version.clone(),
                candidate_version: baseline_bundle.version.clone(),
                baseline_bundle_hash: baseline_hash.clone(),
                candidate_bundle_hash: baseline_hash,
                baseline_score: round4(baseline_eval.combined_score),
                candidate_score: round4(baseline_eval.combined_score),
                score_gain: 0.0,
                baseline_router_score: round4(baseline_eval.router_score),
                candidate_router_score: round4(baseline_eval.router_score),
                baseline_primary_response_score: round4(baseline_eval.primary_response_score),
                candidate_primary_response_score: round4(baseline_eval.primary_response_score),
                baseline_synthesis_score: round4(baseline_eval.synthesis_score),
                candidate_synthesis_score: round4(baseline_eval.synthesis_score),
                baseline_router_invalid_json_rate: round4(baseline_eval.router_invalid_json_rate),
                candidate_router_invalid_json_rate: round4(baseline_eval.router_invalid_json_rate),
                wins: 0,
                losses: 0,
                p_value: 1.0,
                promoted: false,
                candidate_source: "none".to_string(),
                optimized_surfaces: Vec::new(),
                notes: vec!["No distinct prompt-bundle candidates were generated".to_string()],
                diff_summary: diff_summary.clone(),
            })
            .await
            .unwrap_or_else(|_| "lineage-write-failed".to_string());

        Ok(PromptEvolutionResult {
            success: true,
            mode: "prompt".to_string(),
            target_key: PROMPT_BUNDLE_PROFILE_KEY.to_string(),
            baseline_version: baseline_bundle.version.clone(),
            candidate_version: baseline_bundle.version.clone(),
            promoted: false,
            evaluated_candidates: 0,
            baseline_score: round4(baseline_eval.combined_score),
            best_candidate_score: round4(baseline_eval.combined_score),
            score_gain: 0.0,
            baseline_router_score: round4(baseline_eval.router_score),
            best_candidate_router_score: round4(baseline_eval.router_score),
            baseline_primary_response_score: round4(baseline_eval.primary_response_score),
            best_candidate_primary_response_score: round4(baseline_eval.primary_response_score),
            baseline_synthesis_score: round4(baseline_eval.synthesis_score),
            best_candidate_synthesis_score: round4(baseline_eval.synthesis_score),
            baseline_router_invalid_json_rate: round4(baseline_eval.router_invalid_json_rate),
            candidate_router_invalid_json_rate: round4(baseline_eval.router_invalid_json_rate),
            wins: 0,
            losses: 0,
            p_value: 1.0,
            candidate_source: None,
            optimized_surfaces: Vec::new(),
            promotion_gate: "rejected: no valid prompt bundle mutations".to_string(),
            promoted_prompt_bundle: None,
            lineage_entry_id: entry_id,
            lineage_archive_path: self.archive_path().display().to_string(),
            notes: vec!["No-op evolution cycle; baseline prompt bundle retained.".to_string()],
            diff_summary,
            error: None,
        })
    }

    fn load_baseline_bundle(
        &self,
        current_bundle_raw: Option<&[u8]>,
    ) -> Result<PromptBundleProfile> {
        let mut bundle = if let Some(raw) = current_bundle_raw {
            parse_prompt_bundle_profile(raw).context("failed to parse stored prompt bundle JSON")?
        } else {
            PromptBundleProfile::default()
        };
        sanitize_prompt_bundle(&mut bundle);
        Ok(bundle)
    }

    async fn load_benchmark_suite(&self) -> Result<PromptBenchmarkProfile> {
        let profile_path = self.config.project_root.join(BENCHMARK_PROFILE_REL_PATH);
        let raw = tokio::fs::read_to_string(&profile_path)
            .await
            .with_context(|| {
                format!(
                    "failed to read prompt benchmark profile {}",
                    profile_path.display()
                )
            })?;
        let profile: PromptBenchmarkProfile = serde_json::from_str(&raw).with_context(|| {
            format!(
                "failed to parse prompt benchmark JSON {}",
                profile_path.display()
            )
        })?;
        if profile.target_key != PROMPT_BUNDLE_PROFILE_KEY {
            tracing::warn!(
                "prompt evolution benchmark target_key mismatch: got '{}', expected '{}'",
                profile.target_key,
                PROMPT_BUNDLE_PROFILE_KEY
            );
        }
        if profile.router_cases.is_empty()
            || profile.primary_response_cases.is_empty()
            || profile.synthesis_cases.is_empty()
        {
            anyhow::bail!(
                "prompt benchmark profile must contain router_cases, primary_response_cases, and synthesis_cases"
            );
        }
        Ok(profile)
    }

    async fn load_recent_lineage(&self, limit: usize) -> Vec<PromptLineageEntry> {
        let archive = self.archive_path();
        let raw = match tokio::fs::read_to_string(&archive).await {
            Ok(content) => content,
            Err(_) => return Vec::new(),
        };
        let mut parsed = Vec::new();
        for line in raw.lines() {
            if line.trim().is_empty() {
                continue;
            }
            if let Ok(entry) = serde_json::from_str::<PromptLineageEntry>(line) {
                parsed.push(entry);
            }
        }
        if parsed.len() <= limit {
            return parsed;
        }
        parsed.split_off(parsed.len().saturating_sub(limit))
    }

    async fn append_lineage_entry(&self, entry: &PromptLineageEntry) -> Result<String> {
        let archive = self.archive_path();
        if let Some(parent) = archive.parent() {
            tokio::fs::create_dir_all(parent).await?;
        }
        let mut line = serde_json::to_string(entry)?;
        line.push('\n');
        use tokio::io::AsyncWriteExt;
        let mut file = tokio::fs::OpenOptions::new()
            .create(true)
            .append(true)
            .open(&archive)
            .await?;
        file.write_all(line.as_bytes()).await?;
        super::prune_jsonl_archive(&archive, MAX_LINEAGE_ARCHIVE_ENTRIES).await?;
        Ok(entry.entry_id.clone())
    }

    fn archive_path(&self) -> PathBuf {
        self.config.project_root.join(LINEAGE_ARCHIVE_REL_PATH)
    }

    fn deterministic_candidates(
        &self,
        baseline: &PromptBundleProfile,
    ) -> Vec<CandidatePromptBundle> {
        let mut router_directness = baseline.clone();
        router_directness.router.policy_block = append_unique_policy_lines(
            &router_directness.router.policy_block,
            ROUTER_DIRECTNESS_MUTATION,
        );
        sanitize_prompt_bundle(&mut router_directness);

        let mut synthesis_preservation = baseline.clone();
        synthesis_preservation.delegation_synthesis.policy_block = append_unique_policy_lines(
            &synthesis_preservation.delegation_synthesis.policy_block,
            SYNTHESIS_TOOL_PRESERVATION_MUTATION,
        );
        synthesis_preservation.delegation_synthesis.instruction_template = append_instruction_note(
            &synthesis_preservation.delegation_synthesis.instruction_template,
            "If delegated outputs already contain the right action, preserve the clearest required tool call instead of rewording away the action.",
        );
        sanitize_prompt_bundle(&mut synthesis_preservation);

        let mut primary_completion = baseline.clone();
        primary_completion.primary_response.policy_block = append_unique_policy_lines(
            &primary_completion.primary_response.policy_block,
            PRIMARY_RESPONSE_COMPLETION_MUTATION,
        );
        primary_completion.primary_response.instruction_template = append_instruction_note(
            &primary_completion.primary_response.instruction_template,
            "When the answer reflects completed work, mention the concrete result before the caveat or next step.",
        );
        sanitize_prompt_bundle(&mut primary_completion);

        vec![
            CandidatePromptBundle {
                source: "deterministic-router-directness".to_string(),
                bundle: router_directness,
            },
            CandidatePromptBundle {
                source: "deterministic-primary-response-completion".to_string(),
                bundle: primary_completion,
            },
            CandidatePromptBundle {
                source: "deterministic-synthesis-tool-preservation".to_string(),
                bundle: synthesis_preservation,
            },
        ]
    }

    async fn generate_llm_candidates(
        &self,
        user_request: &str,
        baseline: &PromptBundleProfile,
        baseline_eval: &BundleEvaluation,
        recent_lineage: &[PromptLineageEntry],
    ) -> Vec<CandidatePromptBundle> {
        let lineage_summary = if recent_lineage.is_empty() {
            "No prior lineage entries.".to_string()
        } else {
            recent_lineage
                .iter()
                .rev()
                .take(6)
                .map(|entry| {
                    format!(
                        "- {} promoted={} gain={:.4} source={} surfaces={}",
                        entry.timestamp_utc,
                        entry.promoted,
                        entry.score_gain,
                        entry.candidate_source,
                        if entry.optimized_surfaces.is_empty() {
                            "none".to_string()
                        } else {
                            entry.optimized_surfaces.join(", ")
                        }
                    )
                })
                .collect::<Vec<_>>()
                .join("\n")
        };

        let router_misses = baseline_eval
            .router_cases
            .iter()
            .filter(|case| case.score < 0.9999)
            .take(6)
            .map(|case| {
                format!(
                    "- {} expected delegation={} complexity={:?} clarify={} | parsed={} | score={:.3}",
                    truncate_chars(&case.message, 120),
                    case.expected_needs_delegation,
                    case.expected_complexity,
                    case.expected_should_clarify,
                    case.parsed,
                    case.score
                )
            })
            .collect::<Vec<_>>()
            .join("\n");
        let synthesis_misses = baseline_eval
            .synthesis_cases
            .iter()
            .filter(|case| case.score < 0.9999)
            .take(6)
            .map(|case| {
                format!(
                    "- {} expected tools [{}] | score={:.3} | required-missing={} | forbidden-hit={}",
                    truncate_chars(&case.original_task, 120),
                    case.expected_tool_names.join(", "),
                    case.score,
                    case.required_phrase_hits
                        .iter()
                        .filter(|(_, present)| !*present)
                        .map(|(phrase, _)| phrase.clone())
                        .collect::<Vec<_>>()
                        .join(", "),
                    case.forbidden_phrase_hits
                        .iter()
                        .filter(|(_, present)| *present)
                        .map(|(phrase, _)| phrase.clone())
                        .collect::<Vec<_>>()
                        .join(", "),
                )
            })
            .collect::<Vec<_>>()
            .join("\n");
        let primary_response_misses = baseline_eval
            .primary_response_cases
            .iter()
            .filter(|case| case.score < 0.9999)
            .take(6)
            .map(|case| {
                format!(
                    "- {} expected tools [{}] | score={:.3} | required-missing={} | forbidden-hit={}",
                    truncate_chars(&case.message, 120),
                    case.expected_tool_names.join(", "),
                    case.score,
                    case.required_phrase_hits
                        .iter()
                        .filter(|(_, present)| !*present)
                        .map(|(phrase, _)| phrase.clone())
                        .collect::<Vec<_>>()
                        .join(", "),
                    case.forbidden_phrase_hits
                        .iter()
                        .filter(|(_, present)| *present)
                        .map(|(phrase, _)| phrase.clone())
                        .collect::<Vec<_>>()
                        .join(", "),
                )
            })
            .collect::<Vec<_>>()
            .join("\n");

        let mutation_focuses = [
            "router directness and low-ambiguity execution bias",
            "router JSON consistency and delegation quality",
            "primary response completion quality, tool use, and concise operational honesty",
            "delegated synthesis tool preservation and action recovery",
            "delegated synthesis degraded follow-up clarity and evidence",
        ];
        let mut out = Vec::new();
        for focus in mutation_focuses {
            let user_prompt = format!(
                "Optimize the prompt bundle for this focus: {focus}\n\n\
Target key: {target_key}\n\
User request: {user_request}\n\n\
Baseline prompt bundle JSON:\n{baseline_json}\n\n\
Baseline score: {baseline_score:.4}\n\
Router score: {router_score:.4} | invalid_json_rate={invalid_json_rate:.4}\n\
Primary response score: {primary_response_score:.4}\n\
Synthesis score: {synthesis_score:.4}\n\n\
Router misses:\n{router_misses}\n\n\
Primary response misses:\n{primary_response_misses}\n\n\
Synthesis misses:\n{synthesis_misses}\n\n\
Recent lineage:\n{lineage_summary}\n\n\
Return ONLY a JSON object with optional top-level keys `router`, `primary_response`, and `delegation_synthesis`.\n\
Each nested object may include any of: `system_prompt`, `policy_block`, `instruction_template`.\n\
Keep required placeholders intact:\n\
- router template must still include {{specialists}}, {{policy_block}}, {{policy_hint}}, {{action_hints}}, {{preferred_action}}, {{message}}\n\
- synthesis template must still include {{original_task}} and {{results_text}}\n\
Do not return commentary or markdown.",
                focus = focus,
                target_key = PROMPT_BUNDLE_PROFILE_KEY,
                user_request = user_request,
                baseline_json = serde_json::to_string_pretty(baseline).unwrap_or_default(),
                baseline_score = baseline_eval.combined_score,
                router_score = baseline_eval.router_score,
                invalid_json_rate = baseline_eval.router_invalid_json_rate,
                primary_response_score = baseline_eval.primary_response_score,
                synthesis_score = baseline_eval.synthesis_score,
                router_misses = if router_misses.is_empty() {
                    "none".to_string()
                } else {
                    router_misses.clone()
                },
                primary_response_misses = if primary_response_misses.is_empty() {
                    "none".to_string()
                } else {
                    primary_response_misses.clone()
                },
                synthesis_misses = if synthesis_misses.is_empty() {
                    "none".to_string()
                } else {
                    synthesis_misses.clone()
                },
                lineage_summary = lineage_summary,
            );
            let response = match self
                .llm
                .chat_with_system(
                    "You mutate AgentArk prompt bundles for better benchmark performance. Output strict JSON only.",
                    &user_prompt,
                )
                .await
            {
                Ok(resp) => resp,
                Err(error) => {
                    tracing::warn!("prompt evolution: llm candidate generation failed: {}", error);
                    continue;
                }
            };
            let Some(parsed) = parse_json_object(&response.content) else {
                tracing::warn!("prompt evolution: llm candidate was not valid JSON");
                continue;
            };
            let mut candidate = baseline.clone();
            apply_prompt_bundle_override(&mut candidate, &parsed);
            sanitize_prompt_bundle(&mut candidate);
            out.push(CandidatePromptBundle {
                source: format!("llm-mutation:{}", focus.replace(' ', "-")),
                bundle: candidate,
            });
        }
        out
    }

    async fn evaluate_bundle(
        &self,
        bundle: &PromptBundleProfile,
        benchmark: &PromptBenchmarkProfile,
    ) -> BundleEvaluation {
        let mut router_cases = Vec::with_capacity(benchmark.router_cases.len());
        let mut primary_response_cases = Vec::with_capacity(benchmark.primary_response_cases.len());
        let mut synthesis_cases = Vec::with_capacity(benchmark.synthesis_cases.len());

        for case in &benchmark.router_cases {
            router_cases.push(self.evaluate_router_case(bundle, case).await);
        }
        for case in &benchmark.primary_response_cases {
            primary_response_cases.push(self.evaluate_primary_response_case(bundle, case).await);
        }
        for case in &benchmark.synthesis_cases {
            synthesis_cases.push(self.evaluate_synthesis_case(bundle, case).await);
        }

        let router_score = if router_cases.is_empty() {
            0.0
        } else {
            router_cases.iter().map(|case| case.score).sum::<f64>() / router_cases.len() as f64
        };
        let synthesis_score = if synthesis_cases.is_empty() {
            0.0
        } else {
            synthesis_cases.iter().map(|case| case.score).sum::<f64>()
                / synthesis_cases.len() as f64
        };
        let primary_response_score = if primary_response_cases.is_empty() {
            0.0
        } else {
            primary_response_cases
                .iter()
                .map(|case| case.score)
                .sum::<f64>()
                / primary_response_cases.len() as f64
        };
        let router_invalid_json_rate = if router_cases.is_empty() {
            0.0
        } else {
            router_cases.iter().filter(|case| !case.parsed).count() as f64
                / router_cases.len() as f64
        };

        let mut case_scores = Vec::new();
        if !router_cases.is_empty() {
            let per_case_weight = ROUTER_WEIGHT / router_cases.len() as f64;
            case_scores.extend(
                router_cases
                    .iter()
                    .map(|case| round4(case.score * per_case_weight)),
            );
        }
        if !primary_response_cases.is_empty() {
            let per_case_weight = PRIMARY_RESPONSE_WEIGHT / primary_response_cases.len() as f64;
            case_scores.extend(
                primary_response_cases
                    .iter()
                    .map(|case| round4(case.score * per_case_weight)),
            );
        }
        if !synthesis_cases.is_empty() {
            let per_case_weight = SYNTHESIS_WEIGHT / synthesis_cases.len() as f64;
            case_scores.extend(
                synthesis_cases
                    .iter()
                    .map(|case| round4(case.score * per_case_weight)),
            );
        }

        BundleEvaluation {
            combined_score: round4(
                router_score * ROUTER_WEIGHT
                    + primary_response_score * PRIMARY_RESPONSE_WEIGHT
                    + synthesis_score * SYNTHESIS_WEIGHT,
            ),
            router_score: round4(router_score),
            primary_response_score: round4(primary_response_score),
            synthesis_score: round4(synthesis_score),
            router_invalid_json_rate: round4(router_invalid_json_rate),
            router_cases,
            primary_response_cases,
            synthesis_cases,
            case_scores,
        }
    }

    async fn evaluate_primary_response_case(
        &self,
        bundle: &PromptBundleProfile,
        case: &PrimaryResponseBenchmarkCase,
    ) -> PrimaryResponseCaseEvaluation {
        let system_prompt = render_primary_response_system_prompt(bundle);
        let actions = case
            .allowed_actions
            .iter()
            .map(|name| synthetic_action(name))
            .collect::<Vec<_>>();
        let response = self
            .llm
            .chat(&system_prompt, case.message.as_str(), &[], &actions)
            .await;
        score_primary_response_case(case, response.ok().as_ref())
    }

    async fn evaluate_router_case(
        &self,
        bundle: &PromptBundleProfile,
        case: &RouterBenchmarkCase,
    ) -> RouterCaseEvaluation {
        let system_prompt = render_router_system_prompt(bundle);
        let action_hints = case
            .preferred_direct_action
            .as_deref()
            .map(|action| format!("- {} (0.95): preferred action from benchmark", action))
            .unwrap_or_else(|| "No registered actions available.".to_string());
        let user_prompt = render_router_user_prompt(
            bundle,
            &RouterPromptRenderInputs {
                specialists: "None configured.",
                policy_hint: "Benchmark policy hint: preserve valid JSON and prefer direct action when appropriate.",
                action_hints: &action_hints,
                preferred_action: case.preferred_direct_action.as_deref().unwrap_or("none"),
                message: case.message.as_str(),
            },
        );
        let response = self
            .llm
            .chat_with_system(&system_prompt, &user_prompt)
            .await;
        score_router_case(case, response.ok().as_ref())
    }

    async fn evaluate_synthesis_case(
        &self,
        bundle: &PromptBundleProfile,
        case: &SynthesisBenchmarkCase,
    ) -> SynthesisCaseEvaluation {
        let system_prompt = render_synthesis_system_prompt(bundle);
        let results = case
            .agent_results
            .iter()
            .enumerate()
            .map(|(index, result)| benchmark_agent_result_to_runtime(result, index))
            .collect::<Vec<_>>();
        let results_text = format_benchmark_results_text(&results);
        let user_prompt = render_synthesis_user_prompt(
            bundle,
            &SynthesisPromptRenderInputs {
                original_task: case.original_task.as_str(),
                results_text: results_text.as_str(),
            },
        );
        let actions = case
            .allowed_actions
            .iter()
            .map(|name| synthetic_action(name))
            .collect::<Vec<_>>();
        let response = self
            .llm
            .chat(&system_prompt, &user_prompt, &[], &actions)
            .await;
        score_synthesis_case(case, response.ok().as_ref())
    }
}

#[derive(Debug, Clone, Serialize, Deserialize)]
struct PromptLineageEntry {
    entry_id: String,
    timestamp_utc: String,
    target_key: String,
    request: String,
    baseline_version: String,
    candidate_version: String,
    baseline_bundle_hash: String,
    candidate_bundle_hash: String,
    baseline_score: f64,
    candidate_score: f64,
    score_gain: f64,
    baseline_router_score: f64,
    candidate_router_score: f64,
    #[serde(default)]
    baseline_primary_response_score: f64,
    #[serde(default)]
    candidate_primary_response_score: f64,
    baseline_synthesis_score: f64,
    candidate_synthesis_score: f64,
    baseline_router_invalid_json_rate: f64,
    candidate_router_invalid_json_rate: f64,
    wins: usize,
    losses: usize,
    p_value: f64,
    promoted: bool,
    candidate_source: String,
    optimized_surfaces: Vec<String>,
    notes: Vec<String>,
    diff_summary: PromptBundleDiffSummary,
}

#[derive(Debug, Clone)]
struct CandidatePromptBundle {
    source: String,
    bundle: PromptBundleProfile,
}

#[derive(Debug, Clone, Deserialize)]
struct PromptBenchmarkProfile {
    target_key: String,
    #[serde(rename = "version")]
    _version: u32,
    router_cases: Vec<RouterBenchmarkCase>,
    primary_response_cases: Vec<PrimaryResponseBenchmarkCase>,
    synthesis_cases: Vec<SynthesisBenchmarkCase>,
}

#[derive(Debug, Clone, Deserialize)]
struct RouterBenchmarkCase {
    message: String,
    expected_needs_delegation: bool,
    expected_complexity: QueryComplexity,
    expected_should_clarify: bool,
    #[serde(default)]
    min_sub_agents: Option<usize>,
    #[serde(default)]
    preferred_direct_action: Option<String>,
}

#[derive(Debug, Clone, Deserialize)]
struct SynthesisBenchmarkCase {
    original_task: String,
    agent_results: Vec<SynthesisBenchmarkAgentResult>,
    #[serde(default)]
    allowed_actions: Vec<String>,
    #[serde(default)]
    expected_tool_names: Vec<String>,
    #[serde(default)]
    required_phrases: Vec<String>,
    #[serde(default)]
    forbidden_phrases: Vec<String>,
    #[serde(default)]
    expect_followup_note: bool,
}

#[derive(Debug, Clone, Deserialize)]
struct PrimaryResponseBenchmarkCase {
    message: String,
    #[serde(default)]
    allowed_actions: Vec<String>,
    #[serde(default)]
    expected_tool_names: Vec<String>,
    #[serde(default)]
    required_phrases: Vec<String>,
    #[serde(default)]
    forbidden_phrases: Vec<String>,
}

#[derive(Debug, Clone, Deserialize)]
struct SynthesisBenchmarkAgentResult {
    #[serde(default = "default_agent_type")]
    agent_type: String,
    task: String,
    content: String,
    #[serde(default = "default_completed_status")]
    status: String,
    #[serde(default)]
    tool_calls: Vec<String>,
    #[serde(default)]
    next_action_hint: Option<String>,
}

#[derive(Debug, Clone)]
struct RouterCaseEvaluation {
    message: String,
    expected_needs_delegation: bool,
    expected_complexity: QueryComplexity,
    expected_should_clarify: bool,
    parsed: bool,
    score: f64,
}

#[derive(Debug, Clone)]
struct SynthesisCaseEvaluation {
    original_task: String,
    expected_tool_names: Vec<String>,
    required_phrase_hits: Vec<(String, bool)>,
    forbidden_phrase_hits: Vec<(String, bool)>,
    score: f64,
}

#[derive(Debug, Clone)]
struct PrimaryResponseCaseEvaluation {
    message: String,
    expected_tool_names: Vec<String>,
    required_phrase_hits: Vec<(String, bool)>,
    forbidden_phrase_hits: Vec<(String, bool)>,
    score: f64,
}

#[derive(Debug, Clone)]
struct BundleEvaluation {
    combined_score: f64,
    router_score: f64,
    primary_response_score: f64,
    synthesis_score: f64,
    router_invalid_json_rate: f64,
    router_cases: Vec<RouterCaseEvaluation>,
    primary_response_cases: Vec<PrimaryResponseCaseEvaluation>,
    synthesis_cases: Vec<SynthesisCaseEvaluation>,
    case_scores: Vec<f64>,
}

#[derive(Debug, Clone)]
struct PairedStats {
    wins: usize,
    losses: usize,
    p_value: f64,
    score_gain: f64,
}

fn sanitize_surface(
    surface: &mut PromptSurfaceProfile,
    defaults: &PromptSurfaceProfile,
    required_placeholders: &[&str],
) {
    surface.system_prompt = sanitize_text_field(&surface.system_prompt, &defaults.system_prompt);
    surface.policy_block = sanitize_text_field(&surface.policy_block, &defaults.policy_block);
    surface.instruction_template = sanitize_text_field(
        &surface.instruction_template,
        &defaults.instruction_template,
    );
    for placeholder in required_placeholders {
        if !surface.instruction_template.contains(placeholder) {
            surface.instruction_template.push_str("\n\n");
            surface.instruction_template.push_str(placeholder);
        }
    }
}

fn sanitize_text_field(value: &str, fallback: &str) -> String {
    let trimmed = value.trim();
    if trimmed.is_empty() {
        return fallback.to_string();
    }
    truncate_chars(trimmed, MAX_SURFACE_CHARS)
}

fn truncate_chars(value: &str, max_chars: usize) -> String {
    value.chars().take(max_chars).collect::<String>()
}

fn render_system_prompt(surface: &PromptSurfaceProfile) -> String {
    let mut combined = interpolate_runtime_tokens(surface.system_prompt.trim());
    if !surface.policy_block.trim().is_empty() {
        combined.push_str("\n\n");
        combined.push_str(&interpolate_runtime_tokens(surface.policy_block.trim()));
    }
    combined
}

fn render_template(template: &str, replacements: &[(&str, &str)]) -> String {
    let mut rendered = interpolate_runtime_tokens(template.trim());
    for (key, value) in replacements {
        rendered = rendered.replace(&format!("{{{}}}", key), value);
    }
    rendered
}

fn interpolate_runtime_tokens(text: &str) -> String {
    text.replace("{product_name}", crate::branding::PRODUCT_NAME)
}

fn append_unique_policy_lines(base: &str, additions: &str) -> String {
    let mut lines = base
        .lines()
        .map(str::trim)
        .filter(|line| !line.is_empty())
        .map(|line| line.to_string())
        .collect::<Vec<_>>();
    let mut seen = lines.iter().cloned().collect::<HashSet<_>>();
    for line in additions
        .lines()
        .map(str::trim)
        .filter(|line| !line.is_empty())
    {
        if seen.insert(line.to_string()) {
            lines.push(line.to_string());
        }
    }
    lines.join("\n")
}

fn append_instruction_note(base: &str, note: &str) -> String {
    if base.contains(note) {
        return base.to_string();
    }
    format!("{}\n- {}", base.trim_end(), note.trim())
}

fn apply_prompt_bundle_override(bundle: &mut PromptBundleProfile, raw: &serde_json::Value) {
    let Some(obj) = raw.as_object() else {
        return;
    };
    if let Some(router) = obj.get("router") {
        apply_surface_override(&mut bundle.router, router);
    }
    if let Some(primary_response) = obj.get("primary_response") {
        apply_surface_override(&mut bundle.primary_response, primary_response);
    }
    if let Some(synthesis) = obj.get("delegation_synthesis") {
        apply_surface_override(&mut bundle.delegation_synthesis, synthesis);
    }
}

fn apply_surface_override(surface: &mut PromptSurfaceProfile, raw: &serde_json::Value) {
    let Some(obj) = raw.as_object() else {
        return;
    };
    if let Some(value) = obj.get("system_prompt").and_then(|value| value.as_str()) {
        surface.system_prompt = value.to_string();
    }
    if let Some(value) = obj.get("policy_block").and_then(|value| value.as_str()) {
        surface.policy_block = value.to_string();
    }
    if let Some(value) = obj
        .get("instruction_template")
        .and_then(|value| value.as_str())
    {
        surface.instruction_template = value.to_string();
    }
}

fn score_router_case(
    case: &RouterBenchmarkCase,
    response: Option<&LlmResponse>,
) -> RouterCaseEvaluation {
    let parsed = response.and_then(|resp| parse_routing_decision_from_text(resp.content.as_str()));
    let mut score = 0.0_f64;

    if let Some(decision) = parsed.as_ref() {
        if decision.needs_delegation == case.expected_needs_delegation {
            score += 0.35;
        }
        if decision.complexity == case.expected_complexity {
            score += 0.25;
        }
        if decision.should_clarify == case.expected_should_clarify {
            score += 0.20;
        }
        let min_sub_agents = case.min_sub_agents.unwrap_or_default();
        if min_sub_agents > 0
            && decision.needs_delegation
            && decision.sub_agents.len() < min_sub_agents
        {
            score = score.min(0.25);
        } else if case.expected_needs_delegation {
            if decision.sub_agents.len() >= min_sub_agents.max(1) {
                score += 0.20;
            }
        } else {
            score += 0.20;
        }
    }

    RouterCaseEvaluation {
        message: case.message.clone(),
        expected_needs_delegation: case.expected_needs_delegation,
        expected_complexity: case.expected_complexity,
        expected_should_clarify: case.expected_should_clarify,
        parsed: parsed.is_some(),
        score: round4(score),
    }
}

fn score_synthesis_case(
    case: &SynthesisBenchmarkCase,
    response: Option<&LlmResponse>,
) -> SynthesisCaseEvaluation {
    let content = response
        .map(|resp| resp.content.to_ascii_lowercase())
        .unwrap_or_default();
    let response_tool_names = response
        .map(|resp| {
            resp.tool_calls
                .iter()
                .map(|call| call.name.to_ascii_lowercase())
                .collect::<HashSet<_>>()
        })
        .unwrap_or_default();

    let required_phrase_hits = case
        .required_phrases
        .iter()
        .map(|phrase| {
            (
                phrase.clone(),
                content.contains(&phrase.to_ascii_lowercase()),
            )
        })
        .collect::<Vec<_>>();
    let forbidden_phrase_hits = case
        .forbidden_phrases
        .iter()
        .map(|phrase| {
            (
                phrase.clone(),
                content.contains(&phrase.to_ascii_lowercase()),
            )
        })
        .collect::<Vec<_>>();

    let mut score = 0.0_f64;
    if case.expected_tool_names.is_empty() {
        if response_tool_names.is_empty() {
            score += 0.30;
        }
    } else {
        let expected = case
            .expected_tool_names
            .iter()
            .map(|name| name.to_ascii_lowercase())
            .collect::<HashSet<_>>();
        let matched = expected
            .iter()
            .filter(|tool| response_tool_names.contains(*tool))
            .count();
        score += 0.30 * (matched as f64 / expected.len().max(1) as f64);
    }
    if !required_phrase_hits.is_empty() {
        let matched = required_phrase_hits
            .iter()
            .filter(|(_, present)| *present)
            .count();
        score += 0.35 * (matched as f64 / required_phrase_hits.len().max(1) as f64);
    } else {
        score += 0.35;
    }
    if forbidden_phrase_hits.iter().all(|(_, present)| !*present) {
        score += 0.20;
    }
    let has_followup_note = content.contains("follow-up")
        || content.contains("follow up")
        || content.contains("still needs")
        || content.contains("needs follow-up");
    if !case.expect_followup_note || has_followup_note {
        score += 0.15;
    }

    SynthesisCaseEvaluation {
        original_task: case.original_task.clone(),
        expected_tool_names: case.expected_tool_names.clone(),
        required_phrase_hits,
        forbidden_phrase_hits,
        score: round4(score),
    }
}

fn score_primary_response_case(
    case: &PrimaryResponseBenchmarkCase,
    response: Option<&LlmResponse>,
) -> PrimaryResponseCaseEvaluation {
    let content = response
        .map(|resp| resp.content.to_ascii_lowercase())
        .unwrap_or_default();
    let response_tool_names = response
        .map(|resp| {
            resp.tool_calls
                .iter()
                .map(|call| call.name.to_ascii_lowercase())
                .collect::<HashSet<_>>()
        })
        .unwrap_or_default();

    let required_phrase_hits = case
        .required_phrases
        .iter()
        .map(|phrase| {
            (
                phrase.clone(),
                content.contains(&phrase.to_ascii_lowercase()),
            )
        })
        .collect::<Vec<_>>();
    let forbidden_phrase_hits = case
        .forbidden_phrases
        .iter()
        .map(|phrase| {
            (
                phrase.clone(),
                content.contains(&phrase.to_ascii_lowercase()),
            )
        })
        .collect::<Vec<_>>();

    let mut score = 0.0_f64;
    if case.expected_tool_names.is_empty() {
        if response_tool_names.is_empty() {
            score += 0.30;
        }
    } else {
        let expected = case
            .expected_tool_names
            .iter()
            .map(|name| name.to_ascii_lowercase())
            .collect::<HashSet<_>>();
        let matched = expected
            .iter()
            .filter(|tool| response_tool_names.contains(*tool))
            .count();
        score += 0.30 * (matched as f64 / expected.len().max(1) as f64);
    }
    if !required_phrase_hits.is_empty() {
        let matched = required_phrase_hits
            .iter()
            .filter(|(_, present)| *present)
            .count();
        score += 0.45 * (matched as f64 / required_phrase_hits.len().max(1) as f64);
    } else {
        score += 0.45;
    }
    if forbidden_phrase_hits.iter().all(|(_, present)| !*present) {
        score += 0.25;
    }

    PrimaryResponseCaseEvaluation {
        message: case.message.clone(),
        expected_tool_names: case.expected_tool_names.clone(),
        required_phrase_hits,
        forbidden_phrase_hits,
        score: round4(score),
    }
}

fn benchmark_agent_result_to_runtime(
    result: &SynthesisBenchmarkAgentResult,
    index: usize,
) -> AgentExecResult {
    AgentExecResult {
        agent_id: format!("benchmark-agent-{}", index),
        agent_type: result.agent_type.clone(),
        task: result.task.clone(),
        is_specialist: false,
        agent_name: Some(format!("Benchmark {}", index + 1)),
        model_name: "benchmark".to_string(),
        content: result.content.clone(),
        llm_response: Some(LlmResponse {
            content: result.content.clone(),
            tool_calls: result
                .tool_calls
                .iter()
                .enumerate()
                .map(|(tool_idx, name)| ToolCall {
                    id: format!("tool-{}-{}", index, tool_idx),
                    name: name.clone(),
                    arguments: serde_json::json!({}),
                })
                .collect(),
            reasoning: None,
            usage: None,
            provider: "benchmark".to_string(),
            model: "benchmark".to_string(),
        }),
        execution_time_ms: 0,
        status: parse_benchmark_status(result.status.as_str()),
        failure_kind: if result.status.trim().eq_ignore_ascii_case("completed") {
            None
        } else {
            Some(FailureKind::DelegationFailed)
        },
        next_action_hint: result.next_action_hint.clone(),
        confidence: Some(1.0),
        artifacts: Vec::new(),
    }
}

fn parse_benchmark_status(value: &str) -> DelegationStatus {
    match value.trim().to_ascii_lowercase().as_str() {
        "partial" => DelegationStatus::Partial,
        "failed" => DelegationStatus::Failed,
        "timed_out" | "timeout" => DelegationStatus::TimedOut,
        "panicked" | "panic" => DelegationStatus::Panicked,
        _ => DelegationStatus::Completed,
    }
}

fn format_benchmark_results_text(results: &[AgentExecResult]) -> String {
    let mut results_text = results
        .iter()
        .map(|result| {
            let tag = format!(
                "{} ({})",
                result.agent_type,
                result.agent_name.as_deref().unwrap_or("?")
            );
            let status_line = if result.status == DelegationStatus::Completed {
                String::new()
            } else {
                let next_step = result
                    .next_action_hint
                    .as_deref()
                    .map(|hint| format!("\nNext step hint: {}", hint))
                    .unwrap_or_default();
                format!("Status: {}{}", result.status.as_str(), next_step)
            };
            let tool_summary = result
                .llm_response
                .as_ref()
                .map(|response| {
                    if response.tool_calls.is_empty() {
                        String::new()
                    } else {
                        format!(
                            "\nMetadata: tools={}",
                            response
                                .tool_calls
                                .iter()
                                .map(|call| call.name.as_str())
                                .collect::<Vec<_>>()
                                .join(", ")
                        )
                    }
                })
                .unwrap_or_default();
            if status_line.is_empty() {
                format!(
                    "## {} - {}{}\n{}",
                    tag,
                    truncate_chars(&result.task, 240),
                    tool_summary,
                    truncate_chars(&result.content, 1600)
                )
            } else {
                format!(
                    "## {} - {}\n{}{}\n{}",
                    tag,
                    truncate_chars(&result.task, 240),
                    status_line,
                    tool_summary,
                    truncate_chars(&result.content, 1600)
                )
            }
        })
        .collect::<Vec<_>>()
        .join("\n\n---\n\n");
    results_text = truncate_chars(&results_text, 9_000);
    results_text
}

fn synthetic_action(name: &str) -> ActionDef {
    ActionDef {
        name: name.to_string(),
        description: format!("Synthetic benchmark action `{}`.", name),
        version: "1.0.0".to_string(),
        input_schema: serde_json::json!({
            "type": "object",
            "properties": {}
        }),
        capabilities: Vec::new(),
        sandbox_mode: None,
        source: ActionSource::System,
        file_path: None,
        authorization: Default::default(),
    }
}

fn parse_routing_decision_from_text(raw: &str) -> Option<RoutingDecision> {
    let trimmed = raw.trim();
    if trimmed.is_empty() {
        return None;
    }
    if let Ok(decision) = serde_json::from_str::<RoutingDecision>(trimmed) {
        return Some(decision);
    }
    let extracted = extract_first_json_object(trimmed)?;
    serde_json::from_str::<RoutingDecision>(&extracted).ok()
}

fn extract_first_json_object(raw: &str) -> Option<String> {
    let mut start_idx: Option<usize> = None;
    let mut depth = 0usize;
    let mut in_string = false;
    let mut escape = false;

    for (idx, ch) in raw.char_indices() {
        if start_idx.is_none() {
            if ch == '{' {
                start_idx = Some(idx);
                depth = 1;
                in_string = false;
                escape = false;
            }
            continue;
        }

        if escape {
            escape = false;
            continue;
        }

        match ch {
            '\\' if in_string => escape = true,
            '"' => in_string = !in_string,
            '{' if !in_string => depth += 1,
            '}' if !in_string => {
                depth = depth.saturating_sub(1);
                if depth == 0 {
                    if let Some(start) = start_idx {
                        return raw.get(start..=idx).map(|slice| slice.to_string());
                    }
                    return None;
                }
            }
            _ => {}
        }
    }

    None
}

fn build_prompt_bundle_diff_summary(
    baseline: &PromptBundleProfile,
    candidate: &PromptBundleProfile,
) -> PromptBundleDiffSummary {
    PromptBundleDiffSummary {
        router_changed_fields: changed_surface_fields(&baseline.router, &candidate.router),
        primary_response_changed_fields: changed_surface_fields(
            &baseline.primary_response,
            &candidate.primary_response,
        ),
        delegation_synthesis_changed_fields: changed_surface_fields(
            &baseline.delegation_synthesis,
            &candidate.delegation_synthesis,
        ),
        router_change_preview: surface_change_preview(&baseline.router, &candidate.router),
        primary_response_change_preview: surface_change_preview(
            &baseline.primary_response,
            &candidate.primary_response,
        ),
        delegation_synthesis_change_preview: surface_change_preview(
            &baseline.delegation_synthesis,
            &candidate.delegation_synthesis,
        ),
    }
}

fn changed_surface_fields(
    baseline: &PromptSurfaceProfile,
    candidate: &PromptSurfaceProfile,
) -> Vec<String> {
    let mut changed = Vec::new();
    if baseline.system_prompt.trim() != candidate.system_prompt.trim() {
        changed.push("system_prompt".to_string());
    }
    if baseline.policy_block.trim() != candidate.policy_block.trim() {
        changed.push("policy_block".to_string());
    }
    if baseline.instruction_template.trim() != candidate.instruction_template.trim() {
        changed.push("instruction_template".to_string());
    }
    changed
}

fn surface_change_preview(
    baseline: &PromptSurfaceProfile,
    candidate: &PromptSurfaceProfile,
) -> Vec<String> {
    let mut preview = Vec::new();
    if baseline.system_prompt.trim() != candidate.system_prompt.trim() {
        preview.extend(diff_preview_lines(
            "system",
            &baseline.system_prompt,
            &candidate.system_prompt,
        ));
    }
    if baseline.policy_block.trim() != candidate.policy_block.trim() {
        preview.extend(diff_preview_lines(
            "policy",
            &baseline.policy_block,
            &candidate.policy_block,
        ));
    }
    if baseline.instruction_template.trim() != candidate.instruction_template.trim() {
        preview.extend(diff_preview_lines(
            "instruction",
            &baseline.instruction_template,
            &candidate.instruction_template,
        ));
    }
    preview.truncate(6);
    preview
}

fn diff_preview_lines(label: &str, before: &str, after: &str) -> Vec<String> {
    let before_lines = normalize_diff_lines(before);
    let after_lines = normalize_diff_lines(after);
    let before_set = before_lines.iter().cloned().collect::<HashSet<_>>();
    let after_set = after_lines.iter().cloned().collect::<HashSet<_>>();

    let mut out = Vec::new();
    for line in after_lines
        .iter()
        .filter(|line| !before_set.contains(*line))
        .take(2)
    {
        out.push(format!("{}: + {}", label, truncate_chars(line, 120)));
    }
    for line in before_lines
        .iter()
        .filter(|line| !after_set.contains(*line))
        .take(2)
    {
        out.push(format!("{}: - {}", label, truncate_chars(line, 120)));
    }
    if out.is_empty() && before.trim() != after.trim() {
        out.push(format!("{}: text changed", label));
    }
    out
}

fn normalize_diff_lines(text: &str) -> Vec<String> {
    text.lines()
        .map(str::trim)
        .filter(|line| !line.is_empty())
        .map(|line| line.to_string())
        .collect()
}

fn collect_optimized_surfaces(diff_summary: &PromptBundleDiffSummary) -> Vec<String> {
    let mut surfaces = BTreeSet::new();
    if !diff_summary.router_changed_fields.is_empty() {
        surfaces.insert("router".to_string());
    }
    if !diff_summary.primary_response_changed_fields.is_empty() {
        surfaces.insert("primary_response".to_string());
    }
    if !diff_summary.delegation_synthesis_changed_fields.is_empty() {
        surfaces.insert("delegation_synthesis".to_string());
    }
    surfaces.into_iter().collect()
}

fn build_prompt_notes(
    baseline: &BundleEvaluation,
    candidate: &BundleEvaluation,
    stats: &PairedStats,
    source: &str,
    diff_summary: &PromptBundleDiffSummary,
) -> Vec<String> {
    let mut notes = vec![
        format!("baseline_router_score={:.4}", baseline.router_score),
        format!("candidate_router_score={:.4}", candidate.router_score),
        format!(
            "baseline_primary_response_score={:.4}",
            baseline.primary_response_score
        ),
        format!(
            "candidate_primary_response_score={:.4}",
            candidate.primary_response_score
        ),
        format!("baseline_synthesis_score={:.4}", baseline.synthesis_score),
        format!("candidate_synthesis_score={:.4}", candidate.synthesis_score),
        format!("wins={} losses={}", stats.wins, stats.losses),
        format!("candidate_source={}", source),
    ];
    if !diff_summary.router_change_preview.is_empty() {
        notes.push(format!(
            "router_changes={}",
            diff_summary.router_change_preview.join(" | ")
        ));
    }
    if !diff_summary.primary_response_change_preview.is_empty() {
        notes.push(format!(
            "primary_response_changes={}",
            diff_summary.primary_response_change_preview.join(" | ")
        ));
    }
    if !diff_summary.delegation_synthesis_change_preview.is_empty() {
        notes.push(format!(
            "synthesis_changes={}",
            diff_summary.delegation_synthesis_change_preview.join(" | ")
        ));
    }
    notes
}

fn prompt_promotion_checks(
    config: &PromptEvolutionConfig,
    baseline_eval: &BundleEvaluation,
    candidate_eval: &BundleEvaluation,
    stats: &PairedStats,
) -> HashMap<&'static str, bool> {
    let mut checks = HashMap::new();
    checks.insert(
        "score_not_worse",
        candidate_eval.combined_score >= baseline_eval.combined_score,
    );
    checks.insert("min_score_gain", stats.score_gain >= config.min_score_gain);
    checks.insert("wins_gt_losses", stats.wins > stats.losses);
    checks.insert("sign_test", stats.p_value <= config.max_sign_test_p_value);
    checks.insert(
        "router_invalid_json_not_worse",
        candidate_eval.router_invalid_json_rate
            <= baseline_eval.router_invalid_json_rate + f64::EPSILON,
    );
    checks
}

fn render_prompt_promotion_gate(checks: &HashMap<&str, bool>) -> String {
    let ordered = [
        "score_not_worse",
        "min_score_gain",
        "wins_gt_losses",
        "sign_test",
        "router_invalid_json_not_worse",
    ];
    let failed = ordered
        .iter()
        .copied()
        .filter(|key| !checks.get(key).copied().unwrap_or(false))
        .collect::<Vec<_>>();
    if failed.is_empty() {
        "passed".to_string()
    } else {
        format!("rejected: {}", failed.join(", "))
    }
}

fn paired_stats(baseline_scores: &[f64], candidate_scores: &[f64]) -> PairedStats {
    let total = baseline_scores.len().min(candidate_scores.len());
    let mut wins = 0usize;
    let mut losses = 0usize;
    for idx in 0..total {
        if candidate_scores[idx] > baseline_scores[idx] {
            wins += 1;
        } else if candidate_scores[idx] < baseline_scores[idx] {
            losses += 1;
        }
    }
    let p_value = one_sided_sign_test_p_value(wins, losses);
    let baseline_total = baseline_scores.iter().sum::<f64>();
    let candidate_total = candidate_scores.iter().sum::<f64>();
    PairedStats {
        wins,
        losses,
        p_value,
        score_gain: round4(candidate_total - baseline_total),
    }
}

fn one_sided_sign_test_p_value(wins: usize, losses: usize) -> f64 {
    let n = wins + losses;
    if n == 0 || wins <= losses {
        return 1.0;
    }
    let mut cumulative = 0.0_f64;
    for k in wins..=n {
        cumulative += combination(n, k) * 0.5_f64.powi(n as i32);
    }
    cumulative.min(1.0)
}

fn combination(n: usize, k: usize) -> f64 {
    if k > n {
        return 0.0;
    }
    let k = k.min(n - k);
    if k == 0 {
        return 1.0;
    }
    let mut result = 1.0_f64;
    for i in 1..=k {
        result *= (n - k + i) as f64;
        result /= i as f64;
    }
    result
}

fn parse_json_object(text: &str) -> Option<serde_json::Value> {
    if let Ok(value) = serde_json::from_str::<serde_json::Value>(text.trim()) {
        if value.is_object() {
            return Some(value);
        }
    }
    let start = text.find('{')?;
    let end = text.rfind('}')?;
    if end <= start {
        return None;
    }
    serde_json::from_str::<serde_json::Value>(&text[start..=end])
        .ok()
        .filter(|value| value.is_object())
}

fn prompt_bundle_hash(bundle: &PromptBundleProfile) -> String {
    let serialized = serde_json::to_string(bundle).unwrap_or_default();
    let mut hasher = std::collections::hash_map::DefaultHasher::new();
    serialized.hash(&mut hasher);
    format!("{:016x}", hasher.finish())
}

fn default_agent_type() -> String {
    "Researcher".to_string()
}

fn default_completed_status() -> String {
    "completed".to_string()
}

fn round4(value: f64) -> f64 {
    (value * 10_000.0).round() / 10_000.0
}

fn f64_eq(a: f64, b: f64) -> bool {
    (a - b).abs() < f64::EPSILON
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn sanitize_prompt_bundle_restores_missing_placeholders() {
        let mut bundle = PromptBundleProfile {
            version: "".to_string(),
            updated_at: None,
            router: PromptSurfaceProfile {
                system_prompt: "".to_string(),
                policy_block: "".to_string(),
                instruction_template: "Task only".to_string(),
            },
            primary_response: PromptSurfaceProfile {
                system_prompt: "".to_string(),
                policy_block: "".to_string(),
                instruction_template: "".to_string(),
            },
            delegation_synthesis: PromptSurfaceProfile {
                system_prompt: "".to_string(),
                policy_block: "".to_string(),
                instruction_template: "Answer".to_string(),
            },
        };

        sanitize_prompt_bundle(&mut bundle);

        assert_eq!(bundle.version, PROMPT_BUNDLE_DEFAULT_VERSION);
        assert!(bundle.router.instruction_template.contains("{message}"));
        assert!(bundle
            .router
            .instruction_template
            .contains("{policy_block}"));
        assert!(bundle
            .delegation_synthesis
            .instruction_template
            .contains("{original_task}"));
        assert!(bundle
            .delegation_synthesis
            .instruction_template
            .contains("{results_text}"));
    }

    #[test]
    fn router_scoring_penalizes_invalid_json() {
        let case = RouterBenchmarkCase {
            message: "Implement the fix in this repo.".to_string(),
            expected_needs_delegation: false,
            expected_complexity: QueryComplexity::Simple,
            expected_should_clarify: false,
            min_sub_agents: None,
            preferred_direct_action: Some("file_write".to_string()),
        };

        let score = score_router_case(&case, None);
        assert!(!score.parsed);
        assert_eq!(score.score, 0.0);
    }

    #[test]
    fn router_scoring_rewards_exact_match() {
        let case = RouterBenchmarkCase {
            message: "Implement the fix in this repo.".to_string(),
            expected_needs_delegation: false,
            expected_complexity: QueryComplexity::Simple,
            expected_should_clarify: false,
            min_sub_agents: None,
            preferred_direct_action: Some("file_write".to_string()),
        };
        let response = LlmResponse {
            content: serde_json::json!({
                "needs_delegation": false,
                "complexity": "simple",
                "sub_agents": [],
                "reasoning": "direct fix",
                "confidence": 0.91,
                "should_clarify": false,
                "clarification_question": null
            })
            .to_string(),
            tool_calls: Vec::new(),
            reasoning: None,
            usage: None,
            provider: "test".to_string(),
            model: "test".to_string(),
        };

        let score = score_router_case(&case, Some(&response));
        assert!(score.parsed);
        assert!(score.score >= 0.99);
    }

    #[test]
    fn synthesis_scoring_checks_tools_and_phrases() {
        let case = SynthesisBenchmarkCase {
            original_task: "Ship the fix".to_string(),
            agent_results: Vec::new(),
            allowed_actions: vec!["file_write".to_string()],
            expected_tool_names: vec!["file_write".to_string()],
            required_phrases: vec!["completed".to_string()],
            forbidden_phrases: vec!["agent".to_string()],
            expect_followup_note: false,
        };
        let response = LlmResponse {
            content: "Completed the fix and validated it.".to_string(),
            tool_calls: vec![ToolCall {
                id: "tool-1".to_string(),
                name: "file_write".to_string(),
                arguments: serde_json::json!({}),
            }],
            reasoning: None,
            usage: None,
            provider: "test".to_string(),
            model: "test".to_string(),
        };

        let score = score_synthesis_case(&case, Some(&response));
        assert!(score.score >= 0.80);
    }

    #[test]
    fn primary_response_scoring_checks_tools_and_phrases() {
        let case = PrimaryResponseBenchmarkCase {
            message: "Search the web and summarize the top 2 updates.".to_string(),
            allowed_actions: vec!["research".to_string()],
            expected_tool_names: vec!["research".to_string()],
            required_phrases: vec!["top 2".to_string()],
            forbidden_phrases: vec!["as an ai".to_string()],
        };
        let response = LlmResponse {
            content: "I'll gather the top 2 updates now.".to_string(),
            tool_calls: vec![ToolCall {
                id: "tool-1".to_string(),
                name: "research".to_string(),
                arguments: serde_json::json!({}),
            }],
            reasoning: None,
            usage: None,
            provider: "test".to_string(),
            model: "test".to_string(),
        };

        let score = score_primary_response_case(&case, Some(&response));
        assert!(score.score >= 0.90);
    }

    #[test]
    fn diff_summary_reports_changed_fields() {
        let baseline = PromptBundleProfile::default();
        let mut candidate = baseline.clone();
        candidate.primary_response.policy_block = append_unique_policy_lines(
            &candidate.primary_response.policy_block,
            PRIMARY_RESPONSE_COMPLETION_MUTATION,
        );

        let diff = build_prompt_bundle_diff_summary(&baseline, &candidate);
        assert!(diff
            .primary_response_changed_fields
            .iter()
            .any(|field| field == "policy_block"));
        assert!(!diff.primary_response_change_preview.is_empty());
    }

    #[test]
    fn paired_stats_score_gain_matches_weighted_combined_delta() {
        let baseline_scores = vec![0.1500, 0.1500, 0.1000];
        let candidate_scores = vec![0.2000, 0.1750, 0.1250];

        let stats = paired_stats(&baseline_scores, &candidate_scores);

        assert_eq!(stats.score_gain, 0.1000);
    }
}
