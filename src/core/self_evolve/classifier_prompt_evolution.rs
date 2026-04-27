//! Compatibility shell for the retired classifier-prompt evolution surface.
//!
//! Chat semantic classification is owned by the unified semantic router. These
//! types and keys remain so existing dashboard/state readers keep compiling,
//! but this module no longer evolves or promotes legacy classifier prompts.

#![allow(dead_code)]

use anyhow::Result;
use serde::{Deserialize, Serialize};
use std::path::PathBuf;

use crate::core::llm::LlmClient;

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
pub const BASE_CLASSIFIER_PROMPT_VERSION: &str = "agent_turn_loop_v1";

const DEFAULT_VERSION: &str = "retired-classifier-prompt-bundle-v1";
const RETIRED_SURFACE_PROMPT: &str =
    "Legacy classifier prompts are retired. Use the unified semantic router contract.";

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ClassifierPromptBundleProfile {
    pub version: String,
    #[serde(default)]
    pub updated_at: Option<String>,
    #[serde(default = "default_link_intent_surface")]
    pub link_intent: PromptSurfaceProfile,
    #[serde(default = "default_explicit_approval_surface")]
    pub explicit_approval: PromptSurfaceProfile,
    #[serde(default = "default_pending_action_surface")]
    pub pending_action: PromptSurfaceProfile,
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
            max_candidates: 0,
            min_score_gain: 0.0,
            max_sign_test_p_value: 0.0,
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
    pub selection_strategy: String,
    pub focus_cases: Vec<ClassifierPromptFocusCase>,
    pub promotion_gate: String,
    pub promoted_classifier_bundle: Option<ClassifierPromptBundleProfile>,
    pub lineage_entry_id: String,
    pub lineage_archive_path: String,
    pub notes: Vec<String>,
    pub diff_summary: ClassifierPromptBundleDiffSummary,
    pub error: Option<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ClassifierPromptFocusCase {
    pub surface: String,
    pub prompt_preview: String,
    pub baseline_score: f64,
    pub candidate_score: f64,
    pub score_delta: f64,
    pub score_span: f64,
    pub invalid_json_before: bool,
    pub invalid_json_after: bool,
}

impl Default for ClassifierPromptBundleProfile {
    fn default() -> Self {
        Self {
            version: DEFAULT_VERSION.to_string(),
            updated_at: None,
            link_intent: default_link_intent_surface(),
            explicit_approval: default_explicit_approval_surface(),
            pending_action: default_pending_action_surface(),
        }
    }
}

fn retired_surface() -> PromptSurfaceProfile {
    PromptSurfaceProfile {
        system_prompt: RETIRED_SURFACE_PROMPT.to_string(),
        policy_block: String::new(),
        instruction_template: String::new(),
    }
}

pub fn default_link_intent_surface() -> PromptSurfaceProfile {
    retired_surface()
}

pub fn default_explicit_approval_surface() -> PromptSurfaceProfile {
    retired_surface()
}

pub fn default_pending_action_surface() -> PromptSurfaceProfile {
    retired_surface()
}

pub fn parse_classifier_prompt_bundle_profile(raw: &[u8]) -> Option<ClassifierPromptBundleProfile> {
    let mut bundle = serde_json::from_slice::<ClassifierPromptBundleProfile>(raw).ok()?;
    sanitize_classifier_prompt_bundle(&mut bundle);
    Some(bundle)
}

pub fn compose_classifier_prompt_version(bundle_version: &str) -> String {
    let bundle_version = bundle_version.trim();
    if bundle_version.is_empty() {
        BASE_CLASSIFIER_PROMPT_VERSION.to_string()
    } else {
        format!("{}+{}", BASE_CLASSIFIER_PROMPT_VERSION, bundle_version)
    }
}

pub fn sanitize_classifier_prompt_bundle(bundle: &mut ClassifierPromptBundleProfile) {
    if bundle.version.trim().is_empty() {
        bundle.version = DEFAULT_VERSION.to_string();
    } else {
        bundle.version = bundle.version.trim().chars().take(128).collect();
    }
    bundle.link_intent = retired_surface();
    bundle.explicit_approval = retired_surface();
    bundle.pending_action = retired_surface();
}

pub struct ClassifierPromptEvolutionEngine {
    config: ClassifierPromptEvolutionConfig,
    _llm: LlmClient,
}

impl ClassifierPromptEvolutionEngine {
    pub fn new(config: ClassifierPromptEvolutionConfig, llm: LlmClient) -> Self {
        Self { config, _llm: llm }
    }

    pub async fn evolve_classifier_prompt_bundle(
        &self,
        _user_request: &str,
        current_bundle_raw: Option<&[u8]>,
    ) -> Result<ClassifierPromptEvolutionResult> {
        let baseline = current_bundle_raw
            .and_then(parse_classifier_prompt_bundle_profile)
            .unwrap_or_default();
        let version = compose_classifier_prompt_version(&baseline.version);
        Ok(ClassifierPromptEvolutionResult {
            success: true,
            mode: "classifier_prompt".to_string(),
            target_key: CLASSIFIER_PROMPT_BUNDLE_PROFILE_KEY.to_string(),
            baseline_version: version.clone(),
            candidate_version: version,
            promoted: false,
            evaluated_candidates: 0,
            baseline_score: 1.0,
            best_candidate_score: 1.0,
            score_gain: 0.0,
            wins: 0,
            losses: 0,
            p_value: 1.0,
            candidate_source: None,
            optimized_surfaces: Vec::new(),
            selection_strategy: "retired".to_string(),
            focus_cases: Vec::new(),
            promotion_gate: "retired".to_string(),
            promoted_classifier_bundle: None,
            lineage_entry_id: String::new(),
            lineage_archive_path: self
                .config
                .project_root
                .join(".agentark/self_evolve/classifier_prompt_bundle_lineage.jsonl")
                .display()
                .to_string(),
            notes: vec![
                "Classifier prompt evolution is retired; routing is handled by agent_turn_loop_v1."
                    .to_string(),
            ],
            diff_summary: ClassifierPromptBundleDiffSummary::default(),
            error: None,
        })
    }
}
