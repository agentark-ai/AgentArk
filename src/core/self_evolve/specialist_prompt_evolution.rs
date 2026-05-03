//! Specialist-role prompt-bundle self-evolution engine.
//!
//! Optimizes the built-in delegated-agent role prompts while preserving custom
//! specialist overrides and custom-role instructions.

use anyhow::{Context, Result};
use serde::{Deserialize, Serialize};
use std::collections::{HashMap, HashSet};
use std::hash::{Hash, Hasher};
use std::path::PathBuf;

use crate::actions::ActionDef;
use crate::core::llm::{LlmClient, LlmResponse};
use crate::core::orchestra::SubAgentType;
use crate::core::prompt_policy::{
    delegated_policy_v2_block, specialist_analyst_system_prompt_v1,
    specialist_coder_system_prompt_v1, specialist_planner_system_prompt_v1,
    specialist_researcher_system_prompt_v1, specialist_validator_system_prompt_v1,
    specialist_writer_system_prompt_v1,
};

use super::prompt_evolution::PromptSurfaceProfile;

pub const SPECIALIST_PROMPT_BUNDLE_PROFILE_KEY: &str = "specialist_prompt_bundle_profile_v1";
pub const SPECIALIST_PROMPT_BUNDLE_PROFILE_CANARY_KEY: &str =
    "specialist_prompt_bundle_profile_canary_v1";
pub const SPECIALIST_PROMPT_BUNDLE_CANARY_STATE_KEY: &str =
    "specialist_prompt_bundle_canary_state_v1";
pub const SPECIALIST_PROMPT_BUNDLE_BASELINE_SNAPSHOT_KEY: &str =
    "specialist_prompt_bundle_baseline_snapshot_v1";
pub const SPECIALIST_PROMPT_BUNDLE_LAST_RESULT_KEY: &str =
    "specialist_prompt_bundle_last_result_v1";
pub const BASE_SPECIALIST_PROMPT_VERSION: &str = "specialist_prompt_v1";

const DEFAULT_VERSION: &str = "specialist-prompt-bundle-default-v1";
const LINEAGE_ARCHIVE_REL_PATH: &str =
    ".agentark/self_evolve/specialist_prompt_bundle_lineage.jsonl";
const BENCHMARK_PROFILE_JSON: &str = include_str!("benchmarks/specialist_prompt_benchmark_v1.json");
const DEFAULT_RECENT_LINEAGE_LIMIT: usize = 12;
const MAX_LINEAGE_ARCHIVE_ENTRIES: usize = 400;
const MAX_SURFACE_CHARS: usize = 12_000;

const EVIDENCE_MUTATION: &str = r#"
- State the strongest evidence first and name uncertainty explicitly.
- Do not overclaim conclusions that are not supported by the delegated context.
- Keep the output dense and role-appropriate."#;

const BOUNDED_SCOPE_MUTATION: &str = r#"
- Stay inside the assigned task packet.
- Use dependency outputs instead of redoing completed work.
- Do not wander into unrelated recommendations or retries."#;

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct SpecialistPromptBundleProfile {
    pub version: String,
    #[serde(default)]
    pub updated_at: Option<String>,
    pub researcher: PromptSurfaceProfile,
    pub coder: PromptSurfaceProfile,
    pub analyst: PromptSurfaceProfile,
    pub writer: PromptSurfaceProfile,
    pub validator: PromptSurfaceProfile,
    pub planner: PromptSurfaceProfile,
}

#[derive(Debug, Clone, Serialize, Deserialize, Default)]
pub struct SpecialistPromptBundleDiffSummary {
    #[serde(default)]
    pub changed_roles: Vec<String>,
    #[serde(default)]
    pub change_preview: Vec<String>,
}

#[derive(Debug, Clone)]
pub struct SpecialistPromptEvolutionConfig {
    pub project_root: PathBuf,
    pub max_candidates: usize,
    pub min_score_gain: f64,
    pub max_sign_test_p_value: f64,
}

impl Default for SpecialistPromptEvolutionConfig {
    fn default() -> Self {
        Self {
            project_root: PathBuf::from("."),
            max_candidates: 6,
            min_score_gain: 0.03,
            max_sign_test_p_value: 0.10,
        }
    }
}

#[derive(Debug, Clone, Serialize)]
pub struct SpecialistPromptEvolutionResult {
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
    pub wins: usize,
    pub losses: usize,
    pub p_value: f64,
    pub candidate_source: Option<String>,
    pub optimized_surfaces: Vec<String>,
    pub promotion_gate: String,
    pub promoted_specialist_bundle: Option<SpecialistPromptBundleProfile>,
    pub lineage_entry_id: String,
    pub lineage_archive_path: String,
    pub notes: Vec<String>,
    pub diff_summary: SpecialistPromptBundleDiffSummary,
    pub error: Option<String>,
}

#[derive(Debug, Clone)]
pub struct ExternalSpecialistPromptCandidate {
    pub source: String,
    pub bundle: SpecialistPromptBundleProfile,
}

impl Default for SpecialistPromptBundleProfile {
    fn default() -> Self {
        let mut bundle = Self {
            version: DEFAULT_VERSION.to_string(),
            updated_at: None,
            researcher: default_researcher_surface(),
            coder: default_coder_surface(),
            analyst: default_analyst_surface(),
            writer: default_writer_surface(),
            validator: default_validator_surface(),
            planner: default_planner_surface(),
        };
        sanitize_specialist_prompt_bundle(&mut bundle);
        bundle
    }
}

fn default_surface(system_prompt: String) -> PromptSurfaceProfile {
    PromptSurfaceProfile {
        system_prompt,
        policy_block: String::new(),
        instruction_template: String::new(),
    }
}

pub fn default_researcher_surface() -> PromptSurfaceProfile {
    default_surface(specialist_researcher_system_prompt_v1())
}

pub fn default_coder_surface() -> PromptSurfaceProfile {
    default_surface(specialist_coder_system_prompt_v1())
}

pub fn default_analyst_surface() -> PromptSurfaceProfile {
    default_surface(specialist_analyst_system_prompt_v1())
}

pub fn default_writer_surface() -> PromptSurfaceProfile {
    default_surface(specialist_writer_system_prompt_v1())
}

pub fn default_validator_surface() -> PromptSurfaceProfile {
    default_surface(specialist_validator_system_prompt_v1())
}

pub fn default_planner_surface() -> PromptSurfaceProfile {
    default_surface(specialist_planner_system_prompt_v1())
}

pub fn parse_specialist_prompt_bundle_profile(raw: &[u8]) -> Option<SpecialistPromptBundleProfile> {
    let mut bundle = serde_json::from_slice::<SpecialistPromptBundleProfile>(raw).ok()?;
    sanitize_specialist_prompt_bundle(&mut bundle);
    Some(bundle)
}

pub fn embedded_specialist_prompt_benchmark_profile_json() -> &'static str {
    BENCHMARK_PROFILE_JSON
}

pub fn compose_specialist_prompt_version(bundle_version: &str) -> String {
    format!(
        "{}+{}",
        BASE_SPECIALIST_PROMPT_VERSION,
        bundle_version.trim()
    )
}

pub fn sanitize_specialist_prompt_bundle(bundle: &mut SpecialistPromptBundleProfile) {
    if bundle.version.trim().is_empty() {
        bundle.version = DEFAULT_VERSION.to_string();
    } else {
        bundle.version = truncate_chars(bundle.version.trim(), 128);
    }
    sanitize_surface(&mut bundle.researcher, &default_researcher_surface());
    sanitize_surface(&mut bundle.coder, &default_coder_surface());
    sanitize_surface(&mut bundle.analyst, &default_analyst_surface());
    sanitize_surface(&mut bundle.writer, &default_writer_surface());
    sanitize_surface(&mut bundle.validator, &default_validator_surface());
    sanitize_surface(&mut bundle.planner, &default_planner_surface());
}

pub fn render_specialist_role_prompt(
    bundle: &SpecialistPromptBundleProfile,
    agent_type: &SubAgentType,
) -> String {
    match agent_type {
        SubAgentType::Researcher => render_surface(&bundle.researcher),
        SubAgentType::Coder => render_surface(&bundle.coder),
        SubAgentType::Analyst => render_surface(&bundle.analyst),
        SubAgentType::Writer => render_surface(&bundle.writer),
        SubAgentType::Validator => render_surface(&bundle.validator),
        SubAgentType::Planner => render_surface(&bundle.planner),
        SubAgentType::Custom { instructions, .. } => instructions.clone(),
    }
}

pub struct SpecialistPromptEvolutionEngine {
    config: SpecialistPromptEvolutionConfig,
    llm: LlmClient,
}

impl SpecialistPromptEvolutionEngine {
    pub fn new(config: SpecialistPromptEvolutionConfig, llm: LlmClient) -> Self {
        Self { config, llm }
    }

    pub async fn evolve_specialist_prompt_bundle(
        &self,
        user_request: &str,
        current_bundle_raw: Option<&[u8]>,
    ) -> Result<SpecialistPromptEvolutionResult> {
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

        self.evaluate_candidate_set(
            user_request,
            baseline_bundle,
            baseline_eval,
            &benchmark,
            candidates,
        )
        .await
    }

    pub async fn evaluate_external_specialist_prompt_candidates(
        &self,
        user_request: &str,
        current_bundle_raw: Option<&[u8]>,
        candidates: Vec<ExternalSpecialistPromptCandidate>,
    ) -> Result<SpecialistPromptEvolutionResult> {
        let baseline_bundle = self.load_baseline_bundle(current_bundle_raw)?;
        let benchmark = self.load_benchmark_suite().await?;
        let baseline_eval = self.evaluate_bundle(&baseline_bundle, &benchmark).await;
        let candidates = candidates
            .into_iter()
            .map(|candidate| CandidateSpecialistBundle {
                source: candidate.source,
                bundle: candidate.bundle,
            })
            .collect::<Vec<_>>();
        self.evaluate_candidate_set(
            user_request,
            baseline_bundle,
            baseline_eval,
            &benchmark,
            candidates,
        )
        .await
    }

    async fn evaluate_candidate_set(
        &self,
        user_request: &str,
        baseline_bundle: SpecialistPromptBundleProfile,
        baseline_eval: BundleEvaluation,
        benchmark: &SpecialistPromptBenchmarkProfile,
        mut candidates: Vec<CandidateSpecialistBundle>,
    ) -> Result<SpecialistPromptEvolutionResult> {
        if candidates.len() > self.config.max_candidates.max(1) {
            candidates.truncate(self.config.max_candidates.max(1));
        }

        let baseline_hash = specialist_bundle_hash(&baseline_bundle);
        let mut seen = HashSet::new();
        candidates.retain(|candidate| {
            let hash = specialist_bundle_hash(&candidate.bundle);
            hash != baseline_hash && seen.insert(hash)
        });

        if candidates.is_empty() {
            return self
                .build_noop_result(user_request, &baseline_bundle, &baseline_eval)
                .await;
        }

        let evaluated_candidates = candidates.len();
        let mut evaluated = Vec::new();
        for candidate in candidates {
            let eval = self.evaluate_bundle(&candidate.bundle, &benchmark).await;
            let paired = paired_stats(&baseline_eval.case_scores, &eval.case_scores);
            evaluated.push((candidate, eval, paired));
        }

        let (mut best_candidate, best_eval, best_stats) =
            select_best_specialist_candidate(evaluated)
                .context("missing best specialist prompt candidate")?;
        best_candidate.bundle.version = format!(
            "specialist-prompt-{}",
            short_hash(&[
                best_candidate.source.as_str(),
                specialist_bundle_hash(&best_candidate.bundle).as_str(),
            ])
        );
        best_candidate.bundle.updated_at = Some(chrono::Utc::now().to_rfc3339());

        let promoted = best_eval.combined_score >= baseline_eval.combined_score
            && best_stats.score_gain >= self.config.min_score_gain
            && best_stats.wins > best_stats.losses
            && best_stats.p_value <= self.config.max_sign_test_p_value;
        let promotion_gate = if promoted {
            "passed".to_string()
        } else if best_eval.combined_score < baseline_eval.combined_score {
            "candidate score below baseline".to_string()
        } else if best_stats.score_gain < self.config.min_score_gain {
            format!(
                "score gain {:.4} below threshold {:.4}",
                best_stats.score_gain, self.config.min_score_gain
            )
        } else if best_stats.wins <= best_stats.losses {
            format!(
                "wins={} not greater than losses={}",
                best_stats.wins, best_stats.losses
            )
        } else {
            format!(
                "p-value {:.4} above threshold {:.4}",
                best_stats.p_value, self.config.max_sign_test_p_value
            )
        };

        let diff_summary = diff_summary(&baseline_bundle, &best_candidate.bundle);
        let optimized_surfaces = diff_summary.changed_roles.clone();
        let candidate_version = best_candidate.bundle.version.clone();
        let notes = build_result_notes(&baseline_eval, &best_eval, &optimized_surfaces);

        let lineage_entry_id = self
            .append_lineage_entry(&SpecialistPromptLineageEntry {
                entry_id: format!("spcp-{}", uuid::Uuid::new_v4()),
                timestamp_utc: chrono::Utc::now().to_rfc3339(),
                target_key: SPECIALIST_PROMPT_BUNDLE_PROFILE_KEY.to_string(),
                request: user_request.to_string(),
                baseline_version: baseline_bundle.version.clone(),
                candidate_version: candidate_version.clone(),
                baseline_bundle_hash: specialist_bundle_hash(&baseline_bundle),
                candidate_bundle_hash: specialist_bundle_hash(&best_candidate.bundle),
                baseline_score: round4(baseline_eval.combined_score),
                candidate_score: round4(best_eval.combined_score),
                score_gain: round4(best_stats.score_gain),
                wins: best_stats.wins,
                losses: best_stats.losses,
                p_value: round4(best_stats.p_value),
                promoted,
                candidate_source: best_candidate.source.clone(),
                optimized_surfaces: optimized_surfaces.clone(),
                notes: notes.clone(),
                diff_summary: diff_summary.clone(),
            })
            .await
            .unwrap_or_else(|_| "lineage-write-failed".to_string());

        Ok(SpecialistPromptEvolutionResult {
            success: true,
            mode: "specialist_prompt".to_string(),
            target_key: SPECIALIST_PROMPT_BUNDLE_PROFILE_KEY.to_string(),
            baseline_version: baseline_bundle.version,
            candidate_version,
            promoted,
            evaluated_candidates,
            baseline_score: round4(baseline_eval.combined_score),
            best_candidate_score: round4(best_eval.combined_score),
            score_gain: round4(best_stats.score_gain),
            wins: best_stats.wins,
            losses: best_stats.losses,
            p_value: round4(best_stats.p_value),
            candidate_source: Some(best_candidate.source),
            optimized_surfaces,
            promotion_gate,
            promoted_specialist_bundle: if promoted {
                Some(best_candidate.bundle)
            } else {
                None
            },
            lineage_entry_id,
            lineage_archive_path: self.archive_path().to_string_lossy().to_string(),
            notes,
            diff_summary,
            error: None,
        })
    }

    fn load_baseline_bundle(
        &self,
        current_bundle_raw: Option<&[u8]>,
    ) -> Result<SpecialistPromptBundleProfile> {
        let mut bundle = current_bundle_raw
            .and_then(parse_specialist_prompt_bundle_profile)
            .unwrap_or_default();
        sanitize_specialist_prompt_bundle(&mut bundle);
        Ok(bundle)
    }

    async fn load_benchmark_suite(&self) -> Result<SpecialistPromptBenchmarkProfile> {
        serde_json::from_str::<SpecialistPromptBenchmarkProfile>(BENCHMARK_PROFILE_JSON)
            .context("failed to parse embedded specialist benchmark")
    }

    async fn load_recent_lineage(&self, limit: usize) -> Vec<SpecialistPromptLineageEntry> {
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
            if let Ok(entry) = serde_json::from_str::<SpecialistPromptLineageEntry>(line) {
                parsed.push(entry);
            }
        }
        if parsed.len() <= limit {
            return parsed;
        }
        parsed.split_off(parsed.len().saturating_sub(limit))
    }

    async fn append_lineage_entry(&self, entry: &SpecialistPromptLineageEntry) -> Result<String> {
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
        baseline: &SpecialistPromptBundleProfile,
    ) -> Vec<CandidateSpecialistBundle> {
        let mut evidence = baseline.clone();
        for surface in [
            &mut evidence.researcher,
            &mut evidence.coder,
            &mut evidence.analyst,
            &mut evidence.writer,
            &mut evidence.validator,
            &mut evidence.planner,
        ] {
            surface.system_prompt = append_unique_lines(&surface.system_prompt, EVIDENCE_MUTATION);
        }
        sanitize_specialist_prompt_bundle(&mut evidence);

        let mut bounded_scope = baseline.clone();
        for surface in [
            &mut bounded_scope.researcher,
            &mut bounded_scope.coder,
            &mut bounded_scope.analyst,
            &mut bounded_scope.writer,
            &mut bounded_scope.validator,
            &mut bounded_scope.planner,
        ] {
            surface.system_prompt =
                append_unique_lines(&surface.system_prompt, BOUNDED_SCOPE_MUTATION);
        }
        sanitize_specialist_prompt_bundle(&mut bounded_scope);

        vec![
            CandidateSpecialistBundle {
                bundle: evidence,
                source: "deterministic_evidence".to_string(),
            },
            CandidateSpecialistBundle {
                bundle: bounded_scope,
                source: "deterministic_bounded_scope".to_string(),
            },
        ]
    }

    async fn generate_llm_candidates(
        &self,
        user_request: &str,
        baseline: &SpecialistPromptBundleProfile,
        baseline_eval: &BundleEvaluation,
        recent_lineage: &[SpecialistPromptLineageEntry],
    ) -> Vec<CandidateSpecialistBundle> {
        let misses = baseline_eval
            .cases
            .iter()
            .filter(|case| case.score < 0.999)
            .take(12)
            .map(|case| {
                format!(
                    "- role={} score={:.2} task={}",
                    case.role.as_str(),
                    case.score,
                    truncate_chars(case.task.as_str(), 160)
                )
            })
            .collect::<Vec<_>>();
        let lineage_summary = if recent_lineage.is_empty() {
            "No prior lineage entries.".to_string()
        } else {
            recent_lineage
                .iter()
                .rev()
                .take(6)
                .map(|entry| {
                    format!(
                        "- {} promoted={} gain={:.4} source={} roles={}",
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
        let baseline_json = serde_json::to_string_pretty(baseline).unwrap_or_default();
        let prompt = format!(
            "You are improving AgentArk specialist role prompts.\n\
Return ONLY valid JSON for a full SpecialistPromptBundleProfile.\n\n\
User request that triggered optimization:\n{}\n\n\
Baseline score: {:.4}\n\n\
Benchmark misses:\n{}\n\n\
Recent lineage:\n{}\n\n\
Constraints:\n\
- Preserve the role identity of each specialist.\n\
- Keep prompts concise and practical.\n\
- Encourage evidence, bounded scope, and use of dependency outputs.\n\n\
Baseline bundle:\n{}",
            user_request.trim(),
            baseline_eval.combined_score,
            if misses.is_empty() {
                "No major misses recorded.".to_string()
            } else {
                misses.join("\n")
            },
            lineage_summary,
            baseline_json
        );

        let mut out = Vec::new();
        for idx in 0..4 {
            let response = self.llm.chat_with_system(
                "You mutate specialist role prompt bundles for benchmark improvement. Output JSON only.",
                &prompt,
            );
            let Ok(resp) = response.await else {
                continue;
            };
            let Some(mut bundle) = extract_json_object_from_text(&resp.content).and_then(|value| {
                serde_json::from_value::<SpecialistPromptBundleProfile>(serde_json::Value::Object(
                    value,
                ))
                .ok()
            }) else {
                continue;
            };
            sanitize_specialist_prompt_bundle(&mut bundle);
            bundle.version = format!("specialist-llm-candidate-{}", idx + 1);
            out.push(CandidateSpecialistBundle {
                bundle,
                source: format!("llm_mutation_{}", idx + 1),
            });
        }
        out
    }

    async fn evaluate_bundle(
        &self,
        bundle: &SpecialistPromptBundleProfile,
        benchmark: &SpecialistPromptBenchmarkProfile,
    ) -> BundleEvaluation {
        let mut cases = Vec::with_capacity(benchmark.cases.len());
        for case in &benchmark.cases {
            cases.push(self.evaluate_case(bundle, case).await);
        }
        let combined_score = if cases.is_empty() {
            0.0
        } else {
            cases.iter().map(|case| case.score).sum::<f64>() / cases.len() as f64
        };
        let case_scores = cases.iter().map(|case| case.score).collect::<Vec<_>>();
        BundleEvaluation {
            combined_score: round4(combined_score),
            cases,
            case_scores,
        }
    }

    async fn evaluate_case(
        &self,
        bundle: &SpecialistPromptBundleProfile,
        case: &SpecialistPromptBenchmarkCase,
    ) -> SpecialistCaseEvaluation {
        let agent_type = case.role.to_agent_type();
        let role_prompt = render_specialist_role_prompt(bundle, &agent_type);
        let system_prompt = format!(
            "{}\n\n## Delegated Policy\n{}\n\nDelegated task packet:\n{}",
            role_prompt,
            delegated_policy_v2_block(),
            case.context.as_str()
        );
        let actions = case
            .allowed_actions
            .iter()
            .map(|name| synthetic_action(name))
            .collect::<Vec<_>>();
        let response = self
            .llm
            .chat(&system_prompt, case.task.as_str(), &[], &actions)
            .await;
        score_case(case, response.ok().as_ref())
    }

    async fn build_noop_result(
        &self,
        user_request: &str,
        baseline_bundle: &SpecialistPromptBundleProfile,
        baseline_eval: &BundleEvaluation,
    ) -> Result<SpecialistPromptEvolutionResult> {
        let diff_summary = SpecialistPromptBundleDiffSummary::default();
        let entry_id = self
            .append_lineage_entry(&SpecialistPromptLineageEntry {
                entry_id: format!("spcp-{}", uuid::Uuid::new_v4()),
                timestamp_utc: chrono::Utc::now().to_rfc3339(),
                target_key: SPECIALIST_PROMPT_BUNDLE_PROFILE_KEY.to_string(),
                request: user_request.to_string(),
                baseline_version: baseline_bundle.version.clone(),
                candidate_version: baseline_bundle.version.clone(),
                baseline_bundle_hash: specialist_bundle_hash(baseline_bundle),
                candidate_bundle_hash: specialist_bundle_hash(baseline_bundle),
                baseline_score: round4(baseline_eval.combined_score),
                candidate_score: round4(baseline_eval.combined_score),
                score_gain: 0.0,
                wins: 0,
                losses: 0,
                p_value: 1.0,
                promoted: false,
                candidate_source: "none".to_string(),
                optimized_surfaces: Vec::new(),
                notes: vec!["No distinct specialist prompt candidates were generated".to_string()],
                diff_summary: diff_summary.clone(),
            })
            .await
            .unwrap_or_else(|_| "lineage-write-failed".to_string());

        Ok(SpecialistPromptEvolutionResult {
            success: true,
            mode: "specialist_prompt".to_string(),
            target_key: SPECIALIST_PROMPT_BUNDLE_PROFILE_KEY.to_string(),
            baseline_version: baseline_bundle.version.clone(),
            candidate_version: baseline_bundle.version.clone(),
            promoted: false,
            evaluated_candidates: 0,
            baseline_score: round4(baseline_eval.combined_score),
            best_candidate_score: round4(baseline_eval.combined_score),
            score_gain: 0.0,
            wins: 0,
            losses: 0,
            p_value: 1.0,
            candidate_source: Some("none".to_string()),
            optimized_surfaces: Vec::new(),
            promotion_gate: "no_distinct_candidates".to_string(),
            promoted_specialist_bundle: None,
            lineage_entry_id: entry_id,
            lineage_archive_path: self.archive_path().to_string_lossy().to_string(),
            notes: vec!["No distinct specialist prompt candidates were generated".to_string()],
            diff_summary,
            error: None,
        })
    }
}

#[derive(Debug, Clone, Serialize, Deserialize)]
struct SpecialistPromptLineageEntry {
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
    wins: usize,
    losses: usize,
    p_value: f64,
    promoted: bool,
    candidate_source: String,
    optimized_surfaces: Vec<String>,
    notes: Vec<String>,
    diff_summary: SpecialistPromptBundleDiffSummary,
}

#[derive(Debug, Clone, Deserialize)]
struct SpecialistPromptBenchmarkProfile {
    #[serde(rename = "target_key")]
    _target_key: String,
    #[serde(rename = "version")]
    _version: u32,
    cases: Vec<SpecialistPromptBenchmarkCase>,
}

#[derive(Debug, Clone, Deserialize)]
#[serde(rename_all = "snake_case")]
enum SpecialistRoleKind {
    Researcher,
    Coder,
    Analyst,
    Writer,
    Validator,
    Planner,
}

impl SpecialistRoleKind {
    fn as_str(&self) -> &'static str {
        match self {
            Self::Researcher => "researcher",
            Self::Coder => "coder",
            Self::Analyst => "analyst",
            Self::Writer => "writer",
            Self::Validator => "validator",
            Self::Planner => "planner",
        }
    }

    fn to_agent_type(&self) -> SubAgentType {
        match self {
            Self::Researcher => SubAgentType::Researcher,
            Self::Coder => SubAgentType::Coder,
            Self::Analyst => SubAgentType::Analyst,
            Self::Writer => SubAgentType::Writer,
            Self::Validator => SubAgentType::Validator,
            Self::Planner => SubAgentType::Planner,
        }
    }
}

#[derive(Debug, Clone, Deserialize)]
struct SpecialistPromptBenchmarkCase {
    role: SpecialistRoleKind,
    task: String,
    context: String,
    #[serde(default)]
    allowed_actions: Vec<String>,
    #[serde(default)]
    expected_tool_names: Vec<String>,
    #[serde(default)]
    required_phrases: Vec<String>,
    #[serde(default)]
    forbidden_phrases: Vec<String>,
}

#[derive(Debug, Clone)]
struct BundleEvaluation {
    combined_score: f64,
    cases: Vec<SpecialistCaseEvaluation>,
    case_scores: Vec<f64>,
}

#[derive(Debug, Clone)]
struct CandidateSpecialistBundle {
    bundle: SpecialistPromptBundleProfile,
    source: String,
}

#[derive(Debug, Clone)]
struct SpecialistCaseEvaluation {
    role: SpecialistRoleKind,
    task: String,
    score: f64,
}

#[derive(Debug, Clone)]
struct PairedStats {
    wins: usize,
    losses: usize,
    p_value: f64,
    score_gain: f64,
}

fn sanitize_surface(surface: &mut PromptSurfaceProfile, defaults: &PromptSurfaceProfile) {
    if surface.system_prompt.trim().is_empty() {
        surface.system_prompt = defaults.system_prompt.clone();
    } else {
        surface.system_prompt = truncate_chars(surface.system_prompt.trim(), MAX_SURFACE_CHARS);
    }
    surface.policy_block = truncate_chars(surface.policy_block.trim(), MAX_SURFACE_CHARS / 2);
    surface.instruction_template =
        truncate_chars(surface.instruction_template.trim(), MAX_SURFACE_CHARS / 2);
}

fn render_surface(surface: &PromptSurfaceProfile) -> String {
    let mut combined = surface.system_prompt.trim().to_string();
    if !surface.policy_block.trim().is_empty() {
        if !combined.is_empty() {
            combined.push_str("\n\n");
        }
        combined.push_str(surface.policy_block.trim());
    }
    if !surface.instruction_template.trim().is_empty() {
        if !combined.is_empty() {
            combined.push_str("\n\n");
        }
        combined.push_str(surface.instruction_template.trim());
    }
    combined
}

fn truncate_chars(value: &str, max_chars: usize) -> String {
    if value.chars().count() <= max_chars {
        value.to_string()
    } else {
        value.chars().take(max_chars).collect::<String>()
    }
}

fn append_unique_lines(base: &str, addition: &str) -> String {
    let mut lines = base
        .lines()
        .map(str::trim_end)
        .filter(|line| !line.trim().is_empty())
        .map(|line| line.to_string())
        .collect::<Vec<_>>();
    let existing = lines
        .iter()
        .map(|line| line.trim().to_ascii_lowercase())
        .collect::<HashSet<_>>();
    for line in addition
        .lines()
        .map(str::trim)
        .filter(|line| !line.is_empty())
    {
        if !existing.contains(&line.to_ascii_lowercase()) {
            lines.push(line.to_string());
        }
    }
    lines.join("\n")
}

fn extract_json_object_from_text(text: &str) -> Option<serde_json::Map<String, serde_json::Value>> {
    serde_json::from_str::<serde_json::Value>(text)
        .ok()
        .and_then(|value| value.as_object().cloned())
        .or_else(|| {
            let start = text.find('{')?;
            let end = text.rfind('}')?;
            serde_json::from_str::<serde_json::Value>(&text[start..=end])
                .ok()
                .and_then(|value| value.as_object().cloned())
        })
}

fn synthetic_action(name: &str) -> ActionDef {
    ActionDef {
        name: name.to_string(),
        description: format!("Synthetic benchmark action {}", name),
        ..ActionDef::default()
    }
}

fn score_case(
    case: &SpecialistPromptBenchmarkCase,
    response: Option<&LlmResponse>,
) -> SpecialistCaseEvaluation {
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

    let required_hits = case
        .required_phrases
        .iter()
        .filter(|phrase| content.contains(&phrase.to_ascii_lowercase()))
        .count();
    let forbidden_hits = case
        .forbidden_phrases
        .iter()
        .filter(|phrase| content.contains(&phrase.to_ascii_lowercase()))
        .count();
    let tool_hits = case
        .expected_tool_names
        .iter()
        .filter(|name| response_tool_names.contains(&name.to_ascii_lowercase()))
        .count();

    let mut score = 0.0_f64;
    if case.required_phrases.is_empty() {
        score += 0.35;
    } else {
        score += 0.35 * (required_hits as f64 / case.required_phrases.len() as f64);
    }
    if case.expected_tool_names.is_empty() {
        if response_tool_names.is_empty() {
            score += 0.35;
        }
    } else {
        score += 0.35 * (tool_hits as f64 / case.expected_tool_names.len() as f64);
    }
    score += if case.forbidden_phrases.is_empty() {
        0.30
    } else {
        0.30 * (1.0 - (forbidden_hits as f64 / case.forbidden_phrases.len() as f64))
    };

    SpecialistCaseEvaluation {
        role: case.role.clone(),
        task: case.task.clone(),
        score: round4(score.clamp(0.0, 1.0)),
    }
}

fn specialist_bundle_hash(bundle: &SpecialistPromptBundleProfile) -> String {
    let serialized = serde_json::to_string(bundle).unwrap_or_default();
    short_hash(&[serialized.as_str()])
}

fn short_hash(parts: &[&str]) -> String {
    let mut hasher = std::collections::hash_map::DefaultHasher::new();
    for part in parts {
        part.hash(&mut hasher);
    }
    format!("{:016x}", hasher.finish())
}

fn diff_summary(
    baseline: &SpecialistPromptBundleProfile,
    candidate: &SpecialistPromptBundleProfile,
) -> SpecialistPromptBundleDiffSummary {
    let mut changed_roles = Vec::new();
    let mut change_preview = Vec::new();
    for (name, before, after) in [
        ("researcher", &baseline.researcher, &candidate.researcher),
        ("coder", &baseline.coder, &candidate.coder),
        ("analyst", &baseline.analyst, &candidate.analyst),
        ("writer", &baseline.writer, &candidate.writer),
        ("validator", &baseline.validator, &candidate.validator),
        ("planner", &baseline.planner, &candidate.planner),
    ] {
        if before != after {
            changed_roles.push(name.to_string());
            if let Some(preview) = first_changed_line(before, after) {
                change_preview.push(format!("{}: {}", name, preview));
            }
        }
    }
    SpecialistPromptBundleDiffSummary {
        changed_roles,
        change_preview,
    }
}

fn first_changed_line(
    before: &PromptSurfaceProfile,
    after: &PromptSurfaceProfile,
) -> Option<String> {
    for (label, left, right) in [
        (
            "system",
            before.system_prompt.as_str(),
            after.system_prompt.as_str(),
        ),
        (
            "policy",
            before.policy_block.as_str(),
            after.policy_block.as_str(),
        ),
        (
            "instruction",
            before.instruction_template.as_str(),
            after.instruction_template.as_str(),
        ),
    ] {
        if left != right {
            let right_line = right
                .lines()
                .map(str::trim)
                .find(|line| !line.is_empty())
                .unwrap_or("(empty)");
            return Some(format!("{} -> {}", label, truncate_chars(right_line, 120)));
        }
    }
    None
}

fn build_result_notes(
    baseline: &BundleEvaluation,
    candidate: &BundleEvaluation,
    optimized_surfaces: &[String],
) -> Vec<String> {
    let mut notes = Vec::new();
    if !optimized_surfaces.is_empty() {
        notes.push(format!(
            "Changed specialist roles: {}",
            optimized_surfaces.join(", ")
        ));
    }
    let regressions = candidate
        .cases
        .iter()
        .zip(&baseline.cases)
        .filter(|(cand, base)| cand.score + 0.0001 < base.score)
        .take(4)
        .map(|(cand, base)| {
            format!(
                "{} regressed from {:.2} to {:.2}",
                cand.role.as_str(),
                base.score,
                cand.score
            )
        })
        .collect::<Vec<_>>();
    if !regressions.is_empty() {
        notes.push(format!("Observed regressions: {}", regressions.join("; ")));
    }
    notes
}

fn paired_stats(baseline_scores: &[f64], candidate_scores: &[f64]) -> PairedStats {
    let mut wins = 0usize;
    let mut losses = 0usize;
    let mut score_gain = 0.0_f64;
    for (base, cand) in baseline_scores.iter().zip(candidate_scores.iter()) {
        score_gain += cand - base;
        if cand > &(base + 0.0001) {
            wins += 1;
        } else if cand + 0.0001 < *base {
            losses += 1;
        }
    }
    PairedStats {
        wins,
        losses,
        p_value: one_sided_sign_test_p_value(wins, losses),
        score_gain,
    }
}

fn select_best_specialist_candidate(
    evaluated: Vec<(CandidateSpecialistBundle, BundleEvaluation, PairedStats)>,
) -> Option<(CandidateSpecialistBundle, BundleEvaluation, PairedStats)> {
    if evaluated.is_empty() {
        return None;
    }
    let mut nondominated = Vec::new();
    'candidate: for idx in 0..evaluated.len() {
        for other_idx in 0..evaluated.len() {
            if idx == other_idx {
                continue;
            }
            if specialist_candidate_dominates(&evaluated[other_idx], &evaluated[idx]) {
                continue 'candidate;
            }
        }
        nondominated.push(idx);
    }
    let pool = if nondominated.is_empty() {
        (0..evaluated.len()).collect::<Vec<_>>()
    } else {
        nondominated
    };
    let mut best_idx = pool[0];
    for idx in pool.into_iter().skip(1) {
        if specialist_candidate_preferred(&evaluated[idx], &evaluated[best_idx]) {
            best_idx = idx;
        }
    }
    evaluated.into_iter().nth(best_idx)
}

fn specialist_candidate_dominates(
    left: &(CandidateSpecialistBundle, BundleEvaluation, PairedStats),
    right: &(CandidateSpecialistBundle, BundleEvaluation, PairedStats),
) -> bool {
    let (_, left_eval, left_stats) = left;
    let (_, right_eval, right_stats) = right;
    let better_or_equal = left_eval.combined_score + 0.0001 >= right_eval.combined_score
        && left_stats.wins >= right_stats.wins
        && left_stats.losses <= right_stats.losses
        && left_stats.p_value <= right_stats.p_value + 0.0001;
    let strictly_better = left_eval.combined_score > right_eval.combined_score + 0.0001
        || left_stats.wins > right_stats.wins
        || left_stats.losses < right_stats.losses
        || left_stats.p_value + 0.0001 < right_stats.p_value;
    better_or_equal && strictly_better
}

fn specialist_candidate_preferred(
    left: &(CandidateSpecialistBundle, BundleEvaluation, PairedStats),
    right: &(CandidateSpecialistBundle, BundleEvaluation, PairedStats),
) -> bool {
    let (_, left_eval, left_stats) = left;
    let (_, right_eval, right_stats) = right;
    left_eval.combined_score > right_eval.combined_score
        || (f64_eq(left_eval.combined_score, right_eval.combined_score)
            && left_stats.wins > right_stats.wins)
        || (f64_eq(left_eval.combined_score, right_eval.combined_score)
            && left_stats.wins == right_stats.wins
            && left_stats.p_value < right_stats.p_value)
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

fn round4(value: f64) -> f64 {
    (value * 10_000.0).round() / 10_000.0
}

fn f64_eq(left: f64, right: f64) -> bool {
    (left - right).abs() < 0.0001
}
