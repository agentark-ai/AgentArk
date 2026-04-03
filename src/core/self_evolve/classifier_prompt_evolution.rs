//! Classifier/helper prompt-bundle self-evolution engine.
//!
//! Optimizes the high-impact classifier and planning-helper prompts that shape
//! request interpretation before the main response path runs.

use anyhow::{Context, Result};
use serde::{Deserialize, Serialize};
use serde_json::Value;
use std::collections::{HashMap, HashSet};
use std::hash::{Hash, Hasher};
use std::path::PathBuf;

use crate::core::llm::{LlmClient, LlmResponse};
use crate::core::prompt_policy::{
    action_selector_system_prompt_v1, automation_intent_classifier_system_prompt_v1,
    chat_routing_classifier_system_prompt_v1, explicit_approval_classifier_system_prompt_v1,
    link_intent_classifier_system_prompt_v1, pending_action_classifier_system_prompt_v1,
    request_shape_classifier_system_prompt_v1, smalltalk_classifier_system_prompt_v1,
    user_fact_fast_path_system_prompt_v1,
};

use super::prompt_evolution::PromptSurfaceProfile;

pub const CLASSIFIER_PROMPT_BUNDLE_PROFILE_KEY: &str = "classifier_prompt_bundle_profile_v1";
pub const CLASSIFIER_PROMPT_BUNDLE_PROFILE_CANARY_KEY: &str =
    "classifier_prompt_bundle_profile_canary_v1";
pub const CLASSIFIER_PROMPT_BUNDLE_CANARY_STATE_KEY: &str =
    "classifier_prompt_bundle_canary_state_v1";
pub const CLASSIFIER_PROMPT_BUNDLE_BASELINE_SNAPSHOT_KEY: &str =
    "classifier_prompt_bundle_baseline_snapshot_v1";
pub const CLASSIFIER_PROMPT_BUNDLE_LAST_RESULT_KEY: &str =
    "classifier_prompt_bundle_last_result_v1";
pub const BASE_CLASSIFIER_PROMPT_VERSION: &str = "classifier_prompt_v1";

const DEFAULT_VERSION: &str = "classifier-prompt-bundle-default-v1";
const LINEAGE_ARCHIVE_REL_PATH: &str =
    ".agentark/self_evolve/classifier_prompt_bundle_lineage.jsonl";
const BENCHMARK_PROFILE_REL_PATH: &str = "assets/self_evolve/classifier_prompt_benchmark_v1.json";
const DEFAULT_RECENT_LINEAGE_LIMIT: usize = 12;
const MAX_LINEAGE_ARCHIVE_ENTRIES: usize = 400;
const MAX_SURFACE_CHARS: usize = 12_000;

const JSON_DISCIPLINE_MUTATION: &str = r#"
- Return only schema-compliant JSON for structured classifiers.
- Do not add explanations, markdown, hedging, or extra keys.
- When uncertain, stay conservative and prefer the safer non-side-effecting interpretation."#;

const ACTION_GROUNDING_MUTATION: &str = r#"
- Ground action, routing, and automation choices in the provided catalog and execution contract.
- Prefer the smallest actionable interpretation that still matches the request.
- Avoid background work or heavy execution shapes unless the request clearly calls for them."#;

const FACT_CONSERVATISM_MUTATION: &str = r#"
- Capture only durable, stable user facts.
- Do not infer preferences or facts that are not directly supported by the message.
- Prefer `none` over speculative fact extraction."#;

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ClassifierPromptBundleProfile {
    pub version: String,
    #[serde(default)]
    pub updated_at: Option<String>,
    pub smalltalk: PromptSurfaceProfile,
    pub link_intent: PromptSurfaceProfile,
    pub chat_routing: PromptSurfaceProfile,
    pub request_shape: PromptSurfaceProfile,
    pub action_selector: PromptSurfaceProfile,
    pub automation_intent: PromptSurfaceProfile,
    pub explicit_approval: PromptSurfaceProfile,
    pub pending_action: PromptSurfaceProfile,
    pub user_fact_fast_path: PromptSurfaceProfile,
}

#[derive(Debug, Clone, Serialize, Deserialize, Default)]
pub struct ClassifierPromptBundleDiffSummary {
    #[serde(default)]
    pub changed_surfaces: Vec<String>,
    #[serde(default)]
    pub change_preview: Vec<String>,
}

#[derive(Debug, Clone)]
pub struct ClassifierPromptEvolutionConfig {
    pub project_root: PathBuf,
    pub max_candidates: usize,
    pub min_score_gain: f64,
    pub max_sign_test_p_value: f64,
}

impl Default for ClassifierPromptEvolutionConfig {
    fn default() -> Self {
        Self {
            project_root: PathBuf::from("."),
            max_candidates: 7,
            min_score_gain: 0.03,
            max_sign_test_p_value: 0.10,
        }
    }
}

#[derive(Debug, Clone, Serialize)]
pub struct ClassifierPromptEvolutionResult {
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
    pub promoted_classifier_bundle: Option<ClassifierPromptBundleProfile>,
    pub lineage_entry_id: String,
    pub lineage_archive_path: String,
    pub notes: Vec<String>,
    pub diff_summary: ClassifierPromptBundleDiffSummary,
    pub error: Option<String>,
}

impl Default for ClassifierPromptBundleProfile {
    fn default() -> Self {
        let mut bundle = Self {
            version: DEFAULT_VERSION.to_string(),
            updated_at: None,
            smalltalk: default_smalltalk_surface(),
            link_intent: default_link_intent_surface(),
            chat_routing: default_chat_routing_surface(),
            request_shape: default_request_shape_surface(),
            action_selector: default_action_selector_surface(),
            automation_intent: default_automation_intent_surface(),
            explicit_approval: default_explicit_approval_surface(),
            pending_action: default_pending_action_surface(),
            user_fact_fast_path: default_user_fact_fast_path_surface(),
        };
        sanitize_classifier_prompt_bundle(&mut bundle);
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

pub fn default_smalltalk_surface() -> PromptSurfaceProfile {
    default_surface(smalltalk_classifier_system_prompt_v1())
}

pub fn default_link_intent_surface() -> PromptSurfaceProfile {
    default_surface(link_intent_classifier_system_prompt_v1())
}

pub fn default_chat_routing_surface() -> PromptSurfaceProfile {
    default_surface(chat_routing_classifier_system_prompt_v1())
}

pub fn default_request_shape_surface() -> PromptSurfaceProfile {
    default_surface(request_shape_classifier_system_prompt_v1())
}

pub fn default_action_selector_surface() -> PromptSurfaceProfile {
    default_surface(action_selector_system_prompt_v1())
}

pub fn default_automation_intent_surface() -> PromptSurfaceProfile {
    default_surface(automation_intent_classifier_system_prompt_v1())
}

pub fn default_explicit_approval_surface() -> PromptSurfaceProfile {
    default_surface(explicit_approval_classifier_system_prompt_v1())
}

pub fn default_pending_action_surface() -> PromptSurfaceProfile {
    default_surface(pending_action_classifier_system_prompt_v1())
}

pub fn default_user_fact_fast_path_surface() -> PromptSurfaceProfile {
    default_surface(user_fact_fast_path_system_prompt_v1())
}

pub fn parse_classifier_prompt_bundle_profile(raw: &[u8]) -> Option<ClassifierPromptBundleProfile> {
    let mut bundle = serde_json::from_slice::<ClassifierPromptBundleProfile>(raw).ok()?;
    sanitize_classifier_prompt_bundle(&mut bundle);
    Some(bundle)
}

pub fn compose_classifier_prompt_version(bundle_version: &str) -> String {
    format!(
        "{}+{}",
        BASE_CLASSIFIER_PROMPT_VERSION,
        bundle_version.trim()
    )
}

pub fn sanitize_classifier_prompt_bundle(bundle: &mut ClassifierPromptBundleProfile) {
    if bundle.version.trim().is_empty() {
        bundle.version = DEFAULT_VERSION.to_string();
    } else {
        bundle.version = truncate_chars(bundle.version.trim(), 128);
    }
    sanitize_surface(&mut bundle.smalltalk, &default_smalltalk_surface());
    sanitize_surface(&mut bundle.link_intent, &default_link_intent_surface());
    sanitize_surface(&mut bundle.chat_routing, &default_chat_routing_surface());
    sanitize_surface(&mut bundle.request_shape, &default_request_shape_surface());
    sanitize_surface(
        &mut bundle.action_selector,
        &default_action_selector_surface(),
    );
    sanitize_surface(
        &mut bundle.automation_intent,
        &default_automation_intent_surface(),
    );
    sanitize_surface(
        &mut bundle.explicit_approval,
        &default_explicit_approval_surface(),
    );
    sanitize_surface(
        &mut bundle.pending_action,
        &default_pending_action_surface(),
    );
    sanitize_surface(
        &mut bundle.user_fact_fast_path,
        &default_user_fact_fast_path_surface(),
    );
}

pub fn render_smalltalk_classifier_system_prompt(bundle: &ClassifierPromptBundleProfile) -> String {
    render_surface(&bundle.smalltalk)
}

pub fn render_link_intent_classifier_system_prompt(
    bundle: &ClassifierPromptBundleProfile,
) -> String {
    render_surface(&bundle.link_intent)
}

pub fn render_chat_routing_classifier_system_prompt(
    bundle: &ClassifierPromptBundleProfile,
) -> String {
    render_surface(&bundle.chat_routing)
}

pub fn render_request_shape_classifier_system_prompt(
    bundle: &ClassifierPromptBundleProfile,
) -> String {
    render_surface(&bundle.request_shape)
}

pub fn render_action_selector_system_prompt(bundle: &ClassifierPromptBundleProfile) -> String {
    render_surface(&bundle.action_selector)
}

pub fn render_automation_intent_classifier_system_prompt(
    bundle: &ClassifierPromptBundleProfile,
) -> String {
    render_surface(&bundle.automation_intent)
}

pub fn render_explicit_approval_classifier_system_prompt(
    bundle: &ClassifierPromptBundleProfile,
) -> String {
    render_surface(&bundle.explicit_approval)
}

pub fn render_pending_action_classifier_system_prompt(
    bundle: &ClassifierPromptBundleProfile,
) -> String {
    render_surface(&bundle.pending_action)
}

pub fn render_user_fact_fast_path_system_prompt(bundle: &ClassifierPromptBundleProfile) -> String {
    render_surface(&bundle.user_fact_fast_path)
}

pub struct ClassifierPromptEvolutionEngine {
    config: ClassifierPromptEvolutionConfig,
    llm: LlmClient,
}

impl ClassifierPromptEvolutionEngine {
    pub fn new(config: ClassifierPromptEvolutionConfig, llm: LlmClient) -> Self {
        Self { config, llm }
    }

    pub async fn evolve_classifier_prompt_bundle(
        &self,
        user_request: &str,
        current_bundle_raw: Option<&[u8]>,
    ) -> Result<ClassifierPromptEvolutionResult> {
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

        if candidates.len() > self.config.max_candidates.max(1) {
            candidates.truncate(self.config.max_candidates.max(1));
        }

        let baseline_hash = classifier_bundle_hash(&baseline_bundle);
        let mut seen = HashSet::new();
        candidates.retain(|candidate| {
            let hash = classifier_bundle_hash(&candidate.bundle);
            hash != baseline_hash && seen.insert(hash)
        });

        if candidates.is_empty() {
            return self
                .build_noop_result(user_request, &baseline_bundle, &baseline_eval)
                .await;
        }

        let evaluated_candidates = candidates.len();
        let mut best: Option<(CandidateClassifierBundle, BundleEvaluation, PairedStats)> = None;
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
            best.context("missing best classifier prompt candidate")?;
        best_candidate.bundle.version = format!(
            "classifier-prompt-{}",
            short_hash(&[
                best_candidate.source.as_str(),
                classifier_bundle_hash(&best_candidate.bundle).as_str(),
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
        let optimized_surfaces = diff_summary.changed_surfaces.clone();
        let candidate_version = best_candidate.bundle.version.clone();
        let notes = build_result_notes(&baseline_eval, &best_eval, &optimized_surfaces);

        let lineage_entry_id = self
            .append_lineage_entry(&ClassifierPromptLineageEntry {
                entry_id: format!("clsp-{}", uuid::Uuid::new_v4()),
                timestamp_utc: chrono::Utc::now().to_rfc3339(),
                target_key: CLASSIFIER_PROMPT_BUNDLE_PROFILE_KEY.to_string(),
                request: user_request.to_string(),
                baseline_version: baseline_bundle.version.clone(),
                candidate_version: candidate_version.clone(),
                baseline_bundle_hash: classifier_bundle_hash(&baseline_bundle),
                candidate_bundle_hash: classifier_bundle_hash(&best_candidate.bundle),
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

        Ok(ClassifierPromptEvolutionResult {
            success: true,
            mode: "classifier_prompt".to_string(),
            target_key: CLASSIFIER_PROMPT_BUNDLE_PROFILE_KEY.to_string(),
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
            promoted_classifier_bundle: if promoted {
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
    ) -> Result<ClassifierPromptBundleProfile> {
        let mut bundle = current_bundle_raw
            .and_then(parse_classifier_prompt_bundle_profile)
            .unwrap_or_default();
        sanitize_classifier_prompt_bundle(&mut bundle);
        Ok(bundle)
    }

    async fn load_benchmark_suite(&self) -> Result<ClassifierBenchmarkProfile> {
        let path = self.config.project_root.join(BENCHMARK_PROFILE_REL_PATH);
        let raw = tokio::fs::read(&path)
            .await
            .with_context(|| format!("failed to read classifier benchmark {}", path.display()))?;
        serde_json::from_slice::<ClassifierBenchmarkProfile>(&raw)
            .with_context(|| format!("failed to parse classifier benchmark {}", path.display()))
    }

    async fn load_recent_lineage(&self, limit: usize) -> Vec<ClassifierPromptLineageEntry> {
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
            if let Ok(entry) = serde_json::from_str::<ClassifierPromptLineageEntry>(line) {
                parsed.push(entry);
            }
        }
        if parsed.len() <= limit {
            return parsed;
        }
        parsed.split_off(parsed.len().saturating_sub(limit))
    }

    async fn append_lineage_entry(&self, entry: &ClassifierPromptLineageEntry) -> Result<String> {
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
        baseline: &ClassifierPromptBundleProfile,
    ) -> Vec<CandidateClassifierBundle> {
        let mut json_discipline = baseline.clone();
        for surface in [
            &mut json_discipline.link_intent,
            &mut json_discipline.chat_routing,
            &mut json_discipline.request_shape,
            &mut json_discipline.action_selector,
            &mut json_discipline.automation_intent,
            &mut json_discipline.explicit_approval,
            &mut json_discipline.pending_action,
            &mut json_discipline.user_fact_fast_path,
        ] {
            surface.system_prompt =
                append_unique_lines(&surface.system_prompt, JSON_DISCIPLINE_MUTATION);
        }
        json_discipline.smalltalk.system_prompt = append_unique_lines(
            &json_discipline.smalltalk.system_prompt,
            "\n- Reply with only SMALLTALK or TASK.",
        );
        sanitize_classifier_prompt_bundle(&mut json_discipline);

        let mut action_grounding = baseline.clone();
        for surface in [
            &mut action_grounding.chat_routing,
            &mut action_grounding.request_shape,
            &mut action_grounding.action_selector,
            &mut action_grounding.automation_intent,
        ] {
            surface.system_prompt =
                append_unique_lines(&surface.system_prompt, ACTION_GROUNDING_MUTATION);
        }
        sanitize_classifier_prompt_bundle(&mut action_grounding);

        let mut fact_conservatism = baseline.clone();
        fact_conservatism.user_fact_fast_path.system_prompt = append_unique_lines(
            &fact_conservatism.user_fact_fast_path.system_prompt,
            FACT_CONSERVATISM_MUTATION,
        );
        fact_conservatism.link_intent.system_prompt = append_unique_lines(
            &fact_conservatism.link_intent.system_prompt,
            "\n- Prefer SHARE_ONLY over IMPORT_SKILL unless import intent is explicit.",
        );
        sanitize_classifier_prompt_bundle(&mut fact_conservatism);

        vec![
            CandidateClassifierBundle {
                bundle: json_discipline,
                source: "deterministic_json_discipline".to_string(),
            },
            CandidateClassifierBundle {
                bundle: action_grounding,
                source: "deterministic_action_grounding".to_string(),
            },
            CandidateClassifierBundle {
                bundle: fact_conservatism,
                source: "deterministic_fact_conservatism".to_string(),
            },
        ]
    }

    async fn generate_llm_candidates(
        &self,
        user_request: &str,
        baseline: &ClassifierPromptBundleProfile,
        baseline_eval: &BundleEvaluation,
        recent_lineage: &[ClassifierPromptLineageEntry],
    ) -> Vec<CandidateClassifierBundle> {
        let misses = baseline_eval
            .cases
            .iter()
            .filter(|case| case.score < 0.999)
            .take(12)
            .map(|case| {
                format!(
                    "- surface={} score={:.2} invalid_json={} prompt={}",
                    case.surface.as_str(),
                    case.score,
                    case.invalid_json,
                    truncate_chars(case.prompt.as_str(), 180)
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
        let baseline_json = serde_json::to_string_pretty(baseline).unwrap_or_default();
        let prompt = format!(
            "You are improving AgentArk classifier/helper prompts.\n\
Return ONLY valid JSON for a full ClassifierPromptBundleProfile.\n\n\
User request that triggered optimization:\n{}\n\n\
Baseline score: {:.4}\n\n\
Benchmark misses:\n{}\n\n\
Recent lineage:\n{}\n\n\
Constraints:\n\
- Keep each surface concise and operational.\n\
- Do not add markdown fences or commentary.\n\
- Preserve strict JSON behavior for structured classifiers.\n\
- Prefer conservative decisions over speculative or high-side-effect routing.\n\n\
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
                "You mutate classifier prompt bundles for benchmark improvement. Output JSON only.",
                &prompt,
            );
            let Ok(resp) = response.await else {
                continue;
            };
            let Some(mut bundle) = extract_json_object_from_text(&resp.content).and_then(|value| {
                serde_json::from_value::<ClassifierPromptBundleProfile>(Value::Object(value)).ok()
            }) else {
                continue;
            };
            sanitize_classifier_prompt_bundle(&mut bundle);
            bundle.version = format!("classifier-llm-candidate-{}", idx + 1);
            out.push(CandidateClassifierBundle {
                bundle,
                source: format!("llm_mutation_{}", idx + 1),
            });
        }
        out
    }

    async fn evaluate_bundle(
        &self,
        bundle: &ClassifierPromptBundleProfile,
        benchmark: &ClassifierBenchmarkProfile,
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
        bundle: &ClassifierPromptBundleProfile,
        case: &ClassifierBenchmarkCase,
    ) -> ClassifierCaseEvaluation {
        let system_prompt = match case.surface {
            ClassifierSurfaceKind::Smalltalk => render_smalltalk_classifier_system_prompt(bundle),
            ClassifierSurfaceKind::LinkIntent => {
                render_link_intent_classifier_system_prompt(bundle)
            }
            ClassifierSurfaceKind::ChatRouting => {
                render_chat_routing_classifier_system_prompt(bundle)
            }
            ClassifierSurfaceKind::RequestShape => {
                render_request_shape_classifier_system_prompt(bundle)
            }
            ClassifierSurfaceKind::ActionSelector => render_action_selector_system_prompt(bundle),
            ClassifierSurfaceKind::AutomationIntent => {
                render_automation_intent_classifier_system_prompt(bundle)
            }
            ClassifierSurfaceKind::ExplicitApproval => {
                render_explicit_approval_classifier_system_prompt(bundle)
            }
            ClassifierSurfaceKind::PendingAction => {
                render_pending_action_classifier_system_prompt(bundle)
            }
            ClassifierSurfaceKind::UserFactFastPath => {
                render_user_fact_fast_path_system_prompt(bundle)
            }
        };
        let response = self
            .llm
            .chat_with_system(&system_prompt, case.prompt.as_str())
            .await;
        score_case(case, response.ok().as_ref())
    }

    async fn build_noop_result(
        &self,
        user_request: &str,
        baseline_bundle: &ClassifierPromptBundleProfile,
        baseline_eval: &BundleEvaluation,
    ) -> Result<ClassifierPromptEvolutionResult> {
        let diff_summary = ClassifierPromptBundleDiffSummary::default();
        let entry_id = self
            .append_lineage_entry(&ClassifierPromptLineageEntry {
                entry_id: format!("clsp-{}", uuid::Uuid::new_v4()),
                timestamp_utc: chrono::Utc::now().to_rfc3339(),
                target_key: CLASSIFIER_PROMPT_BUNDLE_PROFILE_KEY.to_string(),
                request: user_request.to_string(),
                baseline_version: baseline_bundle.version.clone(),
                candidate_version: baseline_bundle.version.clone(),
                baseline_bundle_hash: classifier_bundle_hash(baseline_bundle),
                candidate_bundle_hash: classifier_bundle_hash(baseline_bundle),
                baseline_score: round4(baseline_eval.combined_score),
                candidate_score: round4(baseline_eval.combined_score),
                score_gain: 0.0,
                wins: 0,
                losses: 0,
                p_value: 1.0,
                promoted: false,
                candidate_source: "none".to_string(),
                optimized_surfaces: Vec::new(),
                notes: vec!["No distinct classifier prompt candidates were generated".to_string()],
                diff_summary: diff_summary.clone(),
            })
            .await
            .unwrap_or_else(|_| "lineage-write-failed".to_string());

        Ok(ClassifierPromptEvolutionResult {
            success: true,
            mode: "classifier_prompt".to_string(),
            target_key: CLASSIFIER_PROMPT_BUNDLE_PROFILE_KEY.to_string(),
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
            promoted_classifier_bundle: None,
            lineage_entry_id: entry_id,
            lineage_archive_path: self.archive_path().to_string_lossy().to_string(),
            notes: vec!["No distinct classifier prompt candidates were generated".to_string()],
            diff_summary,
            error: None,
        })
    }
}

#[derive(Debug, Clone, Serialize, Deserialize)]
struct ClassifierPromptLineageEntry {
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
    diff_summary: ClassifierPromptBundleDiffSummary,
}

#[derive(Debug, Clone, Deserialize)]
struct ClassifierBenchmarkProfile {
    #[serde(rename = "target_key")]
    _target_key: String,
    #[serde(rename = "version")]
    _version: u32,
    cases: Vec<ClassifierBenchmarkCase>,
}

#[derive(Debug, Clone, Deserialize)]
#[serde(rename_all = "snake_case")]
enum ClassifierSurfaceKind {
    Smalltalk,
    LinkIntent,
    ChatRouting,
    RequestShape,
    ActionSelector,
    AutomationIntent,
    ExplicitApproval,
    PendingAction,
    UserFactFastPath,
}

impl ClassifierSurfaceKind {
    fn as_str(&self) -> &'static str {
        match self {
            Self::Smalltalk => "smalltalk",
            Self::LinkIntent => "link_intent",
            Self::ChatRouting => "chat_routing",
            Self::RequestShape => "request_shape",
            Self::ActionSelector => "action_selector",
            Self::AutomationIntent => "automation_intent",
            Self::ExplicitApproval => "explicit_approval",
            Self::PendingAction => "pending_action",
            Self::UserFactFastPath => "user_fact_fast_path",
        }
    }
}

#[derive(Debug, Clone, Deserialize)]
struct ClassifierBenchmarkCase {
    surface: ClassifierSurfaceKind,
    prompt: String,
    #[serde(default)]
    expected_text: Option<String>,
    #[serde(default)]
    expected_json: HashMap<String, Value>,
    #[serde(default)]
    expected_array_contains: HashMap<String, Vec<String>>,
    #[serde(default)]
    expected_fact_keys: Vec<String>,
}

#[derive(Debug, Clone)]
struct BundleEvaluation {
    combined_score: f64,
    cases: Vec<ClassifierCaseEvaluation>,
    case_scores: Vec<f64>,
}

#[derive(Debug, Clone)]
struct CandidateClassifierBundle {
    bundle: ClassifierPromptBundleProfile,
    source: String,
}

#[derive(Debug, Clone)]
struct ClassifierCaseEvaluation {
    surface: ClassifierSurfaceKind,
    prompt: String,
    score: f64,
    invalid_json: bool,
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

fn extract_json_object_from_text(text: &str) -> Option<serde_json::Map<String, Value>> {
    serde_json::from_str::<Value>(text)
        .ok()
        .and_then(|value| value.as_object().cloned())
        .or_else(|| {
            let start = text.find('{')?;
            let end = text.rfind('}')?;
            serde_json::from_str::<Value>(&text[start..=end])
                .ok()
                .and_then(|value| value.as_object().cloned())
        })
}

fn score_case(
    case: &ClassifierBenchmarkCase,
    response: Option<&LlmResponse>,
) -> ClassifierCaseEvaluation {
    let content = response
        .map(|resp| resp.content.trim().to_string())
        .unwrap_or_default();
    let needs_json = !case.expected_json.is_empty()
        || !case.expected_array_contains.is_empty()
        || !case.expected_fact_keys.is_empty();
    let parsed = if needs_json {
        extract_json_object_from_text(&content)
    } else {
        None
    };
    let invalid_json = needs_json && parsed.is_none();

    let mut total_checks = 0usize;
    let mut earned = 0.0_f64;

    if let Some(expected_text) = case.expected_text.as_deref().map(str::trim) {
        if !expected_text.is_empty() {
            total_checks += 1;
            if content.trim().eq_ignore_ascii_case(expected_text)
                || content
                    .trim()
                    .to_ascii_uppercase()
                    .contains(&expected_text.to_ascii_uppercase())
            {
                earned += 1.0;
            }
        }
    }

    for (key, expected) in &case.expected_json {
        total_checks += 1;
        if parsed
            .as_ref()
            .and_then(|obj| obj.get(key))
            .is_some_and(|actual| json_field_matches(actual, expected))
        {
            earned += 1.0;
        }
    }

    for (key, expected_values) in &case.expected_array_contains {
        total_checks += 1;
        let matched = parsed
            .as_ref()
            .and_then(|obj| obj.get(key))
            .and_then(|value| value.as_array())
            .map(|items| {
                let actual = items
                    .iter()
                    .filter_map(|item| item.as_str())
                    .map(|value| value.trim().to_ascii_lowercase())
                    .collect::<HashSet<_>>();
                expected_values
                    .iter()
                    .all(|value| actual.contains(&value.trim().to_ascii_lowercase()))
            })
            .unwrap_or(false);
        if matched {
            earned += 1.0;
        }
    }

    if !case.expected_fact_keys.is_empty() {
        total_checks += 1;
        let matched = parsed
            .as_ref()
            .and_then(|obj| obj.get("facts"))
            .and_then(|value| value.as_array())
            .map(|facts| {
                let keys = facts
                    .iter()
                    .filter_map(|item| item.get("key").and_then(|value| value.as_str()))
                    .map(|value| value.trim().to_ascii_lowercase())
                    .collect::<HashSet<_>>();
                case.expected_fact_keys
                    .iter()
                    .all(|key| keys.contains(&key.trim().to_ascii_lowercase()))
            })
            .unwrap_or(false);
        if matched {
            earned += 1.0;
        }
    }

    let score = if total_checks == 0 {
        if response.is_some() {
            1.0
        } else {
            0.0
        }
    } else {
        earned / total_checks as f64
    };

    ClassifierCaseEvaluation {
        surface: case.surface.clone(),
        prompt: case.prompt.clone(),
        score: round4(score),
        invalid_json,
    }
}

fn json_field_matches(actual: &Value, expected: &Value) -> bool {
    match (actual, expected) {
        (Value::String(left), Value::String(right)) => {
            left.trim().eq_ignore_ascii_case(right.trim())
        }
        (Value::Bool(left), Value::Bool(right)) => left == right,
        (Value::Number(left), Value::Number(right)) => left == right,
        (Value::Null, Value::Null) => true,
        _ => actual == expected,
    }
}

fn classifier_bundle_hash(bundle: &ClassifierPromptBundleProfile) -> String {
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
    baseline: &ClassifierPromptBundleProfile,
    candidate: &ClassifierPromptBundleProfile,
) -> ClassifierPromptBundleDiffSummary {
    let mut changed_surfaces = Vec::new();
    let mut change_preview = Vec::new();
    for (name, before, after) in [
        ("smalltalk", &baseline.smalltalk, &candidate.smalltalk),
        ("link_intent", &baseline.link_intent, &candidate.link_intent),
        (
            "chat_routing",
            &baseline.chat_routing,
            &candidate.chat_routing,
        ),
        (
            "request_shape",
            &baseline.request_shape,
            &candidate.request_shape,
        ),
        (
            "action_selector",
            &baseline.action_selector,
            &candidate.action_selector,
        ),
        (
            "automation_intent",
            &baseline.automation_intent,
            &candidate.automation_intent,
        ),
        (
            "explicit_approval",
            &baseline.explicit_approval,
            &candidate.explicit_approval,
        ),
        (
            "pending_action",
            &baseline.pending_action,
            &candidate.pending_action,
        ),
        (
            "user_fact_fast_path",
            &baseline.user_fact_fast_path,
            &candidate.user_fact_fast_path,
        ),
    ] {
        if before != after {
            changed_surfaces.push(name.to_string());
            if let Some(preview) = first_changed_line(before, after) {
                change_preview.push(format!("{}: {}", name, preview));
            }
        }
    }
    ClassifierPromptBundleDiffSummary {
        changed_surfaces,
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
            "Changed classifier surfaces: {}",
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
                cand.surface.as_str(),
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
