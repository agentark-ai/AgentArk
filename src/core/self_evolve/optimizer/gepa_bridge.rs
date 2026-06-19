//! File-based DSPy/GEPA bridge for offline Evolve seeding.
//!
//! This module deliberately keeps GEPA out of the production Rust runtime. It
//! exports redacted evidence, imports typed candidates, and leaves evaluation,
//! canarying, and promotion to the existing Evolve engines.

use anyhow::{Context, Result};
use serde::{Deserialize, Serialize};
use serde_json::Value;
use std::collections::HashMap;
use std::collections::HashSet;
use std::path::{Path, PathBuf};
use std::time::{Duration, Instant, SystemTime, UNIX_EPOCH};

use crate::storage::Storage;

use super::prompt_evolution::{
    embedded_prompt_benchmark_profile_json, parse_prompt_bundle_profile, ExternalPromptCandidate,
    PromptBundleProfile, PROMPT_BUNDLE_PROFILE_KEY,
};
use super::prompt_fragment_evolution::{
    prompt_fragment_candidate_benchmark_profile, ExternalPromptFragmentCandidate,
    PROMPT_FRAGMENT_LINEAGE_ARCHIVE_REL_PATH,
};
use super::router_learning::{
    router_learning_benchmark_profile, trace_evidence_from_semantic_steps,
    validate_router_learning_candidate, RouterLearningCandidatePayload,
};
use super::specialist_prompt_evolution::{
    embedded_specialist_prompt_benchmark_profile_json, parse_specialist_prompt_bundle_profile,
    ExternalSpecialistPromptCandidate, SpecialistPromptBundleProfile,
    SPECIALIST_PROMPT_BUNDLE_PROFILE_KEY,
};
use crate::core::model::prompt_fragments::{
    default_prompt_fragment_bundle, parse_prompt_fragment_bundle_profile,
    sanitize_prompt_fragment_bundle, PromptFragmentBundleProfile,
    PROMPT_FRAGMENT_BUNDLE_PROFILE_KEY,
};
use crate::core::{
    arkdistill_contract, sanitize_arkdistill_profile, validate_arkdistill_candidate,
    ArkDistillProfile, ExternalArkDistillCandidate, ARKDISTILL_PROFILE_KEY,
};

const GEPA_ROOT_REL: &str = ".agentark/self_evolve/gepa";
const MAX_EXPORTED_TEXT_CHARS: usize = 1600;
const MAX_JSONL_CANDIDATE_BYTES: usize = 512 * 1024;
const MAX_JSONL_RECORD_BYTES: usize = 768 * 1024;
const MAX_CANDIDATES_FILE_BYTES: u64 = 8 * 1024 * 1024;
const MAX_EXPORT_FILE_BYTES: u64 = 12 * 1024 * 1024;
const MAX_CANDIDATE_RECORDS: usize = 64;
const MAX_EXPORTED_EXPERIENCE_RUNS: u64 = 500;
const DEFAULT_GEPA_QUIET_WINDOW_SECONDS: i64 = 60;
const DEFAULT_GEPA_OPTIMIZER_TIMEOUT_SECONDS: u64 = 30 * 60; // 1800s; background, idle-gated, single-flight
const DEFAULT_GEPA_MAX_ATTEMPTS: u32 = 3;
const DEFAULT_GEPA_RETENTION_DAYS: u64 = 30;
const DEFAULT_GEPA_MAX_RUN_DIRS: usize = 80;
const MAX_GEPA_METRIC_CALLS: u32 = 8;
const GEPA_ESTIMATE_BASE_CALLS: u32 = 3;
const GEPA_ESTIMATE_INPUT_TOKENS_PER_CALL: u64 = 8_000;
const GEPA_ESTIMATE_OUTPUT_TOKENS_PER_CALL: u64 = 8_192;
const GEPA_ESTIMATE_SECONDS_PER_CALL: u64 = 150;
pub const GEPA_OPTIMIZER_CONFIG_KEY: &str = "gepa_optimizer_config_v1";
pub const GEPA_OPTIMIZER_BUDGET_LEDGER_KEY: &str = "gepa_optimizer_budget_ledger_v1";
pub const GEPA_OPTIMIZER_AUTO_STATE_KEY: &str = "gepa_optimizer_auto_state_v1";
pub const GEPA_OPTIMIZER_LAST_RESULT_KEY: &str = "gepa_optimizer_last_result_v1";

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct GepaOptimizerConfig {
    #[serde(default = "default_gepa_enabled")]
    pub enabled: bool,
    #[serde(default = "default_gepa_auto_mode")]
    pub auto_mode: String,
    #[serde(default = "default_gepa_max_metric_calls")]
    pub max_metric_calls: u32,
    #[serde(default = "default_gepa_daily_budget_usd")]
    pub daily_budget_usd: f64,
    #[serde(default = "default_gepa_per_run_budget_usd")]
    pub per_run_budget_usd: f64,
    #[serde(default = "default_gepa_max_runs_per_day")]
    pub max_runs_per_day: u32,
    #[serde(default = "default_true")]
    pub auto_setup: bool,
    #[serde(default = "default_gepa_optimizer_timeout_seconds")]
    pub timeout_seconds: u64,
    #[serde(default = "default_gepa_num_threads")]
    pub num_threads: u32,
}

impl Default for GepaOptimizerConfig {
    fn default() -> Self {
        Self {
            enabled: true,
            auto_mode: default_gepa_auto_mode(),
            max_metric_calls: default_gepa_max_metric_calls(),
            daily_budget_usd: default_gepa_daily_budget_usd(),
            per_run_budget_usd: default_gepa_per_run_budget_usd(),
            max_runs_per_day: default_gepa_max_runs_per_day(),
            auto_setup: true,
            timeout_seconds: default_gepa_optimizer_timeout_seconds(),
            num_threads: default_gepa_num_threads(),
        }
    }
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct GepaBudgetLedgerEntry {
    pub run_id: String,
    pub reserved_usd: f64,
    pub status: String,
    pub recorded_at: String,
}

#[derive(Debug, Clone, Serialize, Deserialize, Default)]
pub struct GepaBudgetLedger {
    #[serde(default)]
    pub entries: Vec<GepaBudgetLedgerEntry>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct GepaBudgetStatus {
    pub daily_budget_usd: f64,
    pub per_run_budget_usd: f64,
    pub max_runs_per_day: u32,
    pub used_today_usd: f64,
    pub runs_today: u32,
    pub remaining_today_usd: f64,
    pub allowed: bool,
    pub reason: Option<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct GepaRunEstimate {
    pub estimated_calls: u32,
    pub input_tokens_per_call: u64,
    pub output_tokens_per_call: u64,
    pub estimated_total_tokens: u64,
    pub estimated_cost_usd: Option<f64>,
    pub estimated_minutes: u64,
    pub num_threads: u32,
    pub max_metric_calls: u32,
    pub timeout_seconds: u64,
    pub approval_required: bool,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct GepaReadiness {
    pub ready: bool,
    pub enabled: bool,
    pub python_ready: bool,
    pub dspy_ready: bool,
    pub model_ready: bool,
    pub provider_key_ready: bool,
    pub venv_path: String,
    pub python_path: String,
    pub model: Option<String>,
    pub model_slot: Option<String>,
    pub provider: Option<String>,
    pub auto_setup: bool,
    pub budget: GepaBudgetStatus,
    pub issues: Vec<String>,
    pub bundled: bool,
}

#[derive(Debug, Clone, Serialize, Deserialize, Default)]
pub struct GepaAutoRunState {
    #[serde(default)]
    pub last_checked_at: Option<String>,
    #[serde(default)]
    pub last_queued_at: Option<String>,
    #[serde(default)]
    pub last_completed_at: Option<String>,
    #[serde(default)]
    pub last_status: Option<String>,
    #[serde(default)]
    pub last_reason: Option<String>,
    #[serde(default)]
    pub last_evidence_samples: usize,
    #[serde(default)]
    pub next_check_after: Option<String>,
}

#[derive(Debug, Clone)]
pub struct GepaOptimizerRuntime {
    pub python_path: PathBuf,
    pub model: String,
    pub env: HashMap<String, String>,
    pub auto_mode: String,
    pub max_metric_calls: u32,
    pub per_run_budget_usd: f64,
    pub num_threads: u32,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum GepaJobKind {
    Export,
    Run,
    Import,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct PendingGepaJob {
    pub job_id: String,
    pub kind: GepaJobKind,
    pub request: String,
    #[serde(default)]
    pub metadata: Value,
    #[serde(default)]
    pub run_id: Option<String>,
    #[serde(default)]
    pub export_path: Option<String>,
    #[serde(default)]
    pub candidates_path: Option<String>,
    #[serde(default = "default_gepa_quiet_window_seconds")]
    pub quiet_window_seconds: i64,
    #[serde(default)]
    pub promotion: GepaPromotionSettings,
    #[serde(default = "default_gepa_optimizer_timeout_seconds")]
    pub optimizer_timeout_seconds: u64,
    #[serde(default = "default_gepa_max_attempts")]
    pub max_attempts: u32,
    #[serde(default)]
    pub attempt_count: u32,
    #[serde(default)]
    pub last_error: Option<String>,
    #[serde(default)]
    pub import_after_run: bool,
    #[serde(default)]
    pub started_at: Option<String>,
    #[serde(default)]
    pub finished_at: Option<String>,
    pub created_at: String,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct GepaPromotionSettings {
    #[serde(default = "default_apply_promotion")]
    pub apply_promotion: bool,
    #[serde(default = "default_canary_rollout_percent")]
    pub canary_rollout_percent: u8,
    #[serde(default = "default_canary_min_samples_per_version")]
    pub canary_min_samples_per_version: usize,
    #[serde(default = "default_canary_min_success_gain")]
    pub canary_min_success_gain: f64,
    #[serde(default = "default_canary_max_sign_test_p_value")]
    pub canary_max_sign_test_p_value: f64,
    #[serde(default = "default_replay_log_limit")]
    pub replay_log_limit: u64,
}

impl Default for GepaPromotionSettings {
    fn default() -> Self {
        Self {
            apply_promotion: default_apply_promotion(),
            canary_rollout_percent: default_canary_rollout_percent(),
            canary_min_samples_per_version: default_canary_min_samples_per_version(),
            canary_min_success_gain: default_canary_min_success_gain(),
            canary_max_sign_test_p_value: default_canary_max_sign_test_p_value(),
            replay_log_limit: default_replay_log_limit(),
        }
    }
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct GepaExportResult {
    pub run_id: String,
    pub export_path: String,
    pub candidates_path: String,
    pub experience_samples: usize,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct GepaRunResult {
    pub status: String,
    pub export_path: String,
    pub candidates_path: String,
    pub timeout_seconds: u64,
    pub exit_code: Option<i32>,
    pub actual_cost_usd: Option<f64>,
    pub stdout_tail: String,
    pub stderr_tail: String,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct GepaRetentionSummary {
    pub run_dirs_removed: usize,
    pub status_files_removed: usize,
    pub stale_running_jobs_requeued: usize,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct GepaImportSummary {
    pub candidates_path: String,
    pub prompt_candidates: usize,
    pub specialist_prompt_candidates: usize,
    pub prompt_fragment_candidates: usize,
    #[serde(default)]
    pub arkdistill_candidates: usize,
    #[serde(default)]
    pub router_learning_candidates: usize,
    pub rejected_candidates: Vec<String>,
}

#[derive(Debug, Clone)]
pub(crate) struct GepaImportedCandidates {
    pub(crate) summary: GepaImportSummary,
    pub(crate) prompt_candidates: Vec<ExternalPromptCandidate>,
    pub(crate) specialist_prompt_candidates: Vec<ExternalSpecialistPromptCandidate>,
    pub(crate) prompt_fragment_candidates: Vec<ExternalPromptFragmentCandidate>,
    pub(crate) arkdistill_candidates: Vec<ExternalArkDistillCandidate>,
    pub(crate) router_learning_candidates: Vec<RouterLearningCandidatePayload>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct GepaCandidateRecord {
    pub run_id: String,
    pub surface: String,
    #[serde(default)]
    pub source: String,
    pub candidate: Value,
    #[serde(default)]
    pub objective_scores: Value,
    #[serde(default)]
    pub feedback_summary: String,
    #[serde(default)]
    pub trace_refs: Vec<String>,
    #[serde(default)]
    pub created_at: Option<String>,
}

pub fn gepa_root(project_root: &Path) -> PathBuf {
    project_root.join(GEPA_ROOT_REL)
}

fn env_path(value: Option<&str>) -> Option<PathBuf> {
    value
        .map(str::trim)
        .filter(|value| !value.is_empty())
        .map(PathBuf::from)
}

fn path_looks_like_source_checkout(path: &Path) -> bool {
    path.join("Cargo.toml").is_file() && path.join("src").is_dir()
}

fn gepa_state_root_candidate(path: PathBuf) -> Option<PathBuf> {
    if path_looks_like_source_checkout(&path) {
        None
    } else {
        Some(path)
    }
}

pub(crate) fn resolve_gepa_project_root_from(
    app_path: &Path,
    _cwd: Option<&Path>,
    _workspace_root: Option<&str>,
    data_root: Option<&str>,
) -> PathBuf {
    if let Some(path) = env_path(data_root).and_then(gepa_state_root_candidate) {
        return path;
    }
    if let Some(path) = gepa_state_root_candidate(app_path.join("data")) {
        return path;
    }
    std::env::temp_dir().join("agentark").join("gepa")
}

pub fn resolve_gepa_project_root() -> PathBuf {
    let data_root = std::env::var("AGENTARK_DATA").ok();
    resolve_gepa_project_root_from(Path::new("/app"), None, None, data_root.as_deref())
}

pub fn gepa_runs_dir(project_root: &Path) -> PathBuf {
    gepa_root(project_root).join("runs")
}

pub fn gepa_pending_dir(project_root: &Path) -> PathBuf {
    gepa_root(project_root).join("pending")
}

pub fn gepa_running_dir(project_root: &Path) -> PathBuf {
    gepa_root(project_root).join("running")
}

pub fn gepa_completed_dir(project_root: &Path) -> PathBuf {
    gepa_root(project_root).join("completed")
}

pub fn gepa_failed_dir(project_root: &Path) -> PathBuf {
    gepa_root(project_root).join("failed")
}

pub fn default_candidates_path(project_root: &Path, run_id: &str) -> PathBuf {
    gepa_runs_dir(project_root)
        .join(run_id)
        .join("candidates.jsonl")
}

pub fn gepa_venv_dir(project_root: &Path) -> PathBuf {
    gepa_root(project_root).join("venv")
}

pub fn gepa_venv_python(project_root: &Path) -> PathBuf {
    if cfg!(windows) {
        gepa_venv_dir(project_root)
            .join("Scripts")
            .join("python.exe")
    } else {
        gepa_venv_dir(project_root).join("bin").join("python")
    }
}

fn bundled_gepa_python() -> Option<PathBuf> {
    let path = PathBuf::from("/opt/agentark-gepa/bin/python");
    if path.exists() {
        Some(path)
    } else {
        None
    }
}

pub async fn load_gepa_optimizer_config(storage: &Storage) -> GepaOptimizerConfig {
    storage
        .get(GEPA_OPTIMIZER_CONFIG_KEY)
        .await
        .ok()
        .flatten()
        .and_then(|raw| serde_json::from_slice::<GepaOptimizerConfig>(&raw).ok())
        .map(normalize_gepa_optimizer_config)
        .unwrap_or_default()
}

pub async fn load_gepa_auto_run_state(storage: &Storage) -> GepaAutoRunState {
    storage
        .get(GEPA_OPTIMIZER_AUTO_STATE_KEY)
        .await
        .ok()
        .flatten()
        .and_then(|raw| serde_json::from_slice::<GepaAutoRunState>(&raw).ok())
        .unwrap_or_default()
}

pub async fn save_gepa_auto_run_state(storage: &Storage, state: &GepaAutoRunState) -> Result<()> {
    storage
        .set(GEPA_OPTIMIZER_AUTO_STATE_KEY, &serde_json::to_vec(state)?)
        .await?;
    Ok(())
}

pub async fn load_gepa_last_result(storage: &Storage) -> Option<Value> {
    storage
        .get(GEPA_OPTIMIZER_LAST_RESULT_KEY)
        .await
        .ok()
        .flatten()
        .and_then(|raw| serde_json::from_slice::<Value>(&raw).ok())
}

pub async fn save_gepa_optimizer_config(
    storage: &Storage,
    config: &GepaOptimizerConfig,
) -> Result<()> {
    let normalized = normalize_gepa_optimizer_config(config.clone());
    storage
        .set(GEPA_OPTIMIZER_CONFIG_KEY, &serde_json::to_vec(&normalized)?)
        .await?;
    Ok(())
}

/// Cached result of probing the GEPA Python environment. The two subprocess
/// spawns behind it (`python --version`, `python -c "import dspy"`) cost
/// seconds per call (up to the 8s timeout each) and were re-run on every
/// /settings/evolution request, which is what made the Settings Advanced tab
/// take 2-5s to open. The interpreter environment changes rarely, so
/// readiness serves the cached probe and refreshes stale entries in the
/// background; only the very first probe per interpreter blocks.
#[derive(Clone, Copy)]
struct GepaEnvProbe {
    python_ready: bool,
    dspy_ready: bool,
    checked_at: Instant,
    refreshing: bool,
}

const GEPA_ENV_PROBE_TTL: Duration = Duration::from_secs(10 * 60);

fn gepa_env_probe_cache() -> &'static std::sync::Mutex<HashMap<PathBuf, GepaEnvProbe>> {
    static CACHE: std::sync::OnceLock<std::sync::Mutex<HashMap<PathBuf, GepaEnvProbe>>> =
        std::sync::OnceLock::new();
    CACHE.get_or_init(|| std::sync::Mutex::new(HashMap::new()))
}

async fn run_gepa_env_probe(python_path: &Path) -> (bool, bool) {
    let python_ready = command_runs(python_path, &["--version"]).await;
    let dspy_ready = python_ready && command_runs(python_path, &["-c", "import dspy"]).await;
    (python_ready, dspy_ready)
}

fn store_gepa_env_probe(python_path: PathBuf, python_ready: bool, dspy_ready: bool) {
    if let Ok(mut cache) = gepa_env_probe_cache().lock() {
        cache.insert(
            python_path,
            GepaEnvProbe {
                python_ready,
                dspy_ready,
                checked_at: Instant::now(),
                refreshing: false,
            },
        );
    }
}

/// Returns `(python_ready, dspy_ready)`, serving the cached probe when one
/// exists. Stale entries are returned immediately while a spawned task
/// re-probes, so request latency never depends on subprocess spawn time
/// after the first call.
async fn probe_gepa_python_env(python_path: &Path) -> (bool, bool) {
    let cached = {
        let Ok(mut cache) = gepa_env_probe_cache().lock() else {
            return run_gepa_env_probe(python_path).await;
        };
        cache.get_mut(python_path).map(|entry| {
            let stale = entry.checked_at.elapsed() >= GEPA_ENV_PROBE_TTL;
            let needs_refresh = stale && !entry.refreshing;
            if needs_refresh {
                entry.refreshing = true;
            }
            (entry.python_ready, entry.dspy_ready, needs_refresh)
        })
    };
    if let Some((python_ready, dspy_ready, needs_refresh)) = cached {
        if needs_refresh {
            let path = python_path.to_path_buf();
            tokio::spawn(async move {
                let (python_ready, dspy_ready) = run_gepa_env_probe(&path).await;
                store_gepa_env_probe(path, python_ready, dspy_ready);
            });
        }
        return (python_ready, dspy_ready);
    }
    let (python_ready, dspy_ready) = run_gepa_env_probe(python_path).await;
    store_gepa_env_probe(python_path.to_path_buf(), python_ready, dspy_ready);
    (python_ready, dspy_ready)
}

/// Drops cached environment probes so the next readiness check re-inspects
/// the interpreter (call after installing or upgrading the GEPA venv).
fn invalidate_gepa_env_probe() {
    if let Ok(mut cache) = gepa_env_probe_cache().lock() {
        cache.clear();
    }
}

pub async fn check_gepa_readiness(
    storage: &Storage,
    project_root: &Path,
    agent_config: &crate::core::runtime::config::AgentConfig,
    primary_model_id: &str,
) -> GepaReadiness {
    let config = load_gepa_optimizer_config(storage).await;
    let budget = gepa_budget_status(storage, &config).await;
    let mut issues = Vec::new();
    if !config.enabled {
        issues.push("GEPA background optimizer is disabled.".to_string());
    }

    let selected_slot = select_gepa_model_slot(agent_config, primary_model_id);
    let selected_runtime = selected_slot.and_then(gepa_model_runtime_from_slot);
    let model = selected_runtime
        .as_ref()
        .map(|runtime| runtime.model.clone());
    if selected_slot.is_none() {
        issues.push("Configure AgentArk's primary model before running GEPA.".to_string());
    }
    let provider_key_ready = selected_runtime
        .as_ref()
        .map(|runtime| runtime.provider_key_ready)
        .unwrap_or(false);
    if selected_slot.is_some() && !provider_key_ready {
        issues.push("The selected AgentArk model does not have usable credentials.".to_string());
    }

    let bundled_python = bundled_gepa_python();
    let python_path = configured_gepa_python(project_root);
    let (python_ready, dspy_ready) = probe_gepa_python_env(&python_path).await;
    if !python_ready {
        issues.push("Python for the GEPA optimizer is not ready.".to_string());
    }

    let auto_setup_ready = config.auto_setup
        && python_ready
        && project_root
            .join("bridges/gepa_optimizer/requirements.txt")
            .exists();
    if python_ready && !dspy_ready && !auto_setup_ready {
        issues.push("DSPy is not installed in the GEPA Python environment.".to_string());
    }
    if !budget.allowed {
        if let Some(reason) = budget.reason.as_ref() {
            issues.push(reason.clone());
        }
    }

    let runtime_ready = dspy_ready || auto_setup_ready;
    let ready = config.enabled
        && python_ready
        && runtime_ready
        && model.is_some()
        && provider_key_ready
        && budget.allowed;
    GepaReadiness {
        ready,
        enabled: config.enabled,
        python_ready,
        dspy_ready,
        model_ready: model.is_some(),
        provider_key_ready,
        venv_path: gepa_venv_dir(project_root).display().to_string(),
        python_path: python_path.display().to_string(),
        model,
        model_slot: selected_slot.map(|slot| slot.id.clone()),
        provider: selected_runtime.map(|runtime| runtime.provider_label),
        auto_setup: config.auto_setup,
        budget,
        issues,
        bundled: bundled_python.is_some(),
    }
}

pub async fn ensure_gepa_optimizer_environment(project_root: &Path) -> Result<PathBuf> {
    if let Some(path) = bundled_gepa_python() {
        if command_runs(&path, &["-c", "import dspy"]).await {
            return Ok(path);
        }
    }

    let python_path = gepa_venv_python(project_root);
    if command_runs(&python_path, &["-c", "import dspy"]).await {
        return Ok(python_path);
    }
    let requirements = project_root.join("bridges/gepa_optimizer/requirements.txt");
    if !requirements.exists() {
        anyhow::bail!(
            "GEPA requirements file is missing at {}",
            requirements.display()
        );
    }
    if !python_path.exists() {
        tokio::fs::create_dir_all(gepa_root(project_root)).await?;
        let python = if cfg!(windows) { "python" } else { "python3" };
        let status = tokio::process::Command::new(python)
            .arg("-m")
            .arg("venv")
            .arg(gepa_venv_dir(project_root))
            .status()
            .await
            .context("failed to create GEPA Python venv")?;
        if !status.success() {
            anyhow::bail!("failed to create GEPA Python venv");
        }
    }
    let install_status = tokio::process::Command::new(&python_path)
        .arg("-m")
        .arg("pip")
        .arg("install")
        .arg("-r")
        .arg(requirements)
        .status()
        .await
        .context("failed to install GEPA Python dependencies")?;
    if !install_status.success() {
        anyhow::bail!("failed to install GEPA Python dependencies");
    }
    // The environment just changed; cached readiness probes are stale.
    invalidate_gepa_env_probe();
    Ok(python_path)
}

pub async fn gepa_optimizer_runtime(
    storage: &Storage,
    project_root: &Path,
    agent_config: &crate::core::runtime::config::AgentConfig,
    primary_model_id: &str,
) -> Result<GepaOptimizerRuntime> {
    let config = load_gepa_optimizer_config(storage).await;
    if !config.enabled {
        anyhow::bail!("GEPA background optimizer is disabled");
    }
    let slot = select_gepa_model_slot(agent_config, primary_model_id)
        .ok_or_else(|| anyhow::anyhow!("AgentArk's primary model is not configured"))?;
    let selected = gepa_model_runtime_from_slot(slot)
        .ok_or_else(|| anyhow::anyhow!("AgentArk's selected model cannot be used by GEPA"))?;
    if !selected.provider_key_ready {
        anyhow::bail!("AgentArk's selected model does not have usable credentials");
    }
    let python_path = if config.auto_setup {
        ensure_gepa_optimizer_environment(project_root).await?
    } else {
        configured_gepa_python(project_root)
    };
    if !command_runs(&python_path, &["-c", "import dspy"]).await {
        anyhow::bail!("DSPy is not installed in the configured GEPA Python environment");
    }
    Ok(GepaOptimizerRuntime {
        python_path,
        model: selected.model,
        env: selected.env,
        auto_mode: config.auto_mode,
        max_metric_calls: config.max_metric_calls,
        per_run_budget_usd: config.per_run_budget_usd,
        num_threads: config.num_threads,
    })
}

pub async fn reserve_gepa_budget(
    storage: &Storage,
    run_id: &str,
    status: &str,
) -> Result<GepaBudgetStatus> {
    let config = load_gepa_optimizer_config(storage).await;
    let mut ledger = load_gepa_budget_ledger(storage).await;
    prune_gepa_budget_ledger(&mut ledger);
    let now = chrono::Utc::now();
    let timezone = gepa_budget_timezone();
    if gepa_budget_has_today_reservation(&ledger, run_id, now, timezone) {
        return Ok(gepa_budget_status_for_reservation_at(
            &config, &ledger, run_id, now, timezone,
        ));
    }
    let status_snapshot = gepa_budget_status_from_ledger_at(&config, &ledger, now, timezone);
    if !status_snapshot.allowed {
        anyhow::bail!(
            "{}",
            status_snapshot
                .reason
                .clone()
                .unwrap_or_else(|| "GEPA budget gate blocked this run".to_string())
        );
    }
    ledger.entries.push(GepaBudgetLedgerEntry {
        run_id: run_id.to_string(),
        reserved_usd: 0.0,
        status: status.to_string(),
        recorded_at: now.to_rfc3339(),
    });
    storage
        .set(
            GEPA_OPTIMIZER_BUDGET_LEDGER_KEY,
            &serde_json::to_vec(&ledger)?,
        )
        .await?;
    Ok(gepa_budget_status_from_ledger_at(
        &config, &ledger, now, timezone,
    ))
}

pub async fn finalize_gepa_budget(
    storage: &Storage,
    run_id: &str,
    spent_usd: f64,
    status: &str,
) -> Result<GepaBudgetStatus> {
    let config = load_gepa_optimizer_config(storage).await;
    let mut ledger = load_gepa_budget_ledger(storage).await;
    prune_gepa_budget_ledger(&mut ledger);
    let now = chrono::Utc::now();
    finalize_gepa_budget_ledger_entry(&mut ledger, run_id, spent_usd, status, now);
    storage
        .set(
            GEPA_OPTIMIZER_BUDGET_LEDGER_KEY,
            &serde_json::to_vec(&ledger)?,
        )
        .await?;
    Ok(gepa_budget_status_from_ledger_at(
        &config,
        &ledger,
        now,
        gepa_budget_timezone(),
    ))
}

pub async fn write_pending_job(project_root: &Path, job: &PendingGepaJob) -> Result<String> {
    let pending_dir = gepa_pending_dir(project_root);
    tokio::fs::create_dir_all(&pending_dir).await?;
    let path = pending_dir.join(format!("{}.json", job.job_id));
    write_json_file_atomic(&path, job).await?;
    Ok(path.display().to_string())
}

pub async fn list_pending_jobs(project_root: &Path) -> Result<Vec<(PathBuf, PendingGepaJob)>> {
    let pending_dir = gepa_pending_dir(project_root);
    let mut out = Vec::new();
    let mut entries = match tokio::fs::read_dir(&pending_dir).await {
        Ok(entries) => entries,
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => return Ok(out),
        Err(error) => return Err(error.into()),
    };
    while let Some(entry) = entries.next_entry().await? {
        let path = entry.path();
        if path.extension().and_then(|value| value.to_str()) != Some("json") {
            continue;
        }
        let raw = tokio::fs::read(&path).await?;
        if let Ok(job) = serde_json::from_slice::<PendingGepaJob>(&raw) {
            out.push((path, job));
        }
    }
    out.sort_by(|left, right| left.1.created_at.cmp(&right.1.created_at));
    Ok(out)
}

pub async fn claim_next_pending_job(
    project_root: &Path,
) -> Result<Option<(PathBuf, PendingGepaJob)>> {
    let pending_jobs = list_pending_jobs(project_root).await?;
    if pending_jobs.is_empty() {
        return Ok(None);
    }
    let running_dir = gepa_running_dir(project_root);
    tokio::fs::create_dir_all(&running_dir).await?;

    for (pending_path, mut job) in pending_jobs {
        let Some(file_name) = pending_path.file_name() else {
            continue;
        };
        let running_path = running_dir.join(file_name);
        match tokio::fs::rename(&pending_path, &running_path).await {
            Ok(_) => {
                job.attempt_count = job.attempt_count.saturating_add(1);
                job.started_at = Some(chrono::Utc::now().to_rfc3339());
                job.finished_at = None;
                write_json_file_atomic(&running_path, &job).await?;
                return Ok(Some((running_path, job)));
            }
            Err(error)
                if matches!(
                    error.kind(),
                    std::io::ErrorKind::NotFound | std::io::ErrorKind::AlreadyExists
                ) =>
            {
                continue;
            }
            Err(error) => return Err(error.into()),
        }
    }

    Ok(None)
}

pub async fn complete_claimed_job(
    project_root: &Path,
    running_path: &Path,
    mut job: PendingGepaJob,
    result: &Value,
) -> Result<PathBuf> {
    let completed_dir = gepa_completed_dir(project_root);
    tokio::fs::create_dir_all(&completed_dir).await?;
    job.finished_at = Some(chrono::Utc::now().to_rfc3339());
    let completed_path = completed_dir.join(format!("{}.json", job.job_id));
    let record = serde_json::json!({
        "status": "completed",
        "job": job,
        "result": result,
        "recorded_at": chrono::Utc::now().to_rfc3339(),
    });
    write_json_file_atomic(&completed_path, &record).await?;
    remove_file_if_exists(running_path).await?;
    Ok(completed_path)
}

pub async fn requeue_claimed_job(
    project_root: &Path,
    running_path: &Path,
    mut job: PendingGepaJob,
    reason: &str,
) -> Result<PathBuf> {
    job.attempt_count = job.attempt_count.saturating_sub(1);
    job.started_at = None;
    job.last_error = Some(reason.to_string());
    let pending_dir = gepa_pending_dir(project_root);
    tokio::fs::create_dir_all(&pending_dir).await?;
    let pending_path = pending_dir.join(format!("{}.json", job.job_id));
    write_json_file_atomic(&pending_path, &job).await?;
    remove_file_if_exists(running_path).await?;
    Ok(pending_path)
}

pub async fn fail_claimed_job(
    project_root: &Path,
    running_path: &Path,
    mut job: PendingGepaJob,
    error: &str,
) -> Result<(String, PathBuf)> {
    job.last_error = Some(error.to_string());
    job.started_at = None;
    if job.attempt_count < job.max_attempts.max(1) {
        let pending_dir = gepa_pending_dir(project_root);
        tokio::fs::create_dir_all(&pending_dir).await?;
        let pending_path = pending_dir.join(format!("{}.json", job.job_id));
        write_json_file_atomic(&pending_path, &job).await?;
        remove_file_if_exists(running_path).await?;
        return Ok(("retry_pending".to_string(), pending_path));
    }

    let failed_dir = gepa_failed_dir(project_root);
    tokio::fs::create_dir_all(&failed_dir).await?;
    job.finished_at = Some(chrono::Utc::now().to_rfc3339());
    let failed_path = failed_dir.join(format!("{}.json", job.job_id));
    let record = serde_json::json!({
        "status": "failed",
        "job": job,
        "error": error,
        "recorded_at": chrono::Utc::now().to_rfc3339(),
    });
    write_json_file_atomic(&failed_path, &record).await?;
    remove_file_if_exists(running_path).await?;
    Ok(("failed".to_string(), failed_path))
}

pub async fn fail_claimed_job_terminal(
    project_root: &Path,
    running_path: &Path,
    mut job: PendingGepaJob,
    error: &str,
) -> Result<PathBuf> {
    job.last_error = Some(error.to_string());
    job.started_at = None;
    let failed_dir = gepa_failed_dir(project_root);
    tokio::fs::create_dir_all(&failed_dir).await?;
    job.finished_at = Some(chrono::Utc::now().to_rfc3339());
    let failed_path = failed_dir.join(format!("{}.json", job.job_id));
    let record = serde_json::json!({
        "status": "failed",
        "job": job,
        "error": error,
        "recorded_at": chrono::Utc::now().to_rfc3339(),
    });
    write_json_file_atomic(&failed_path, &record).await?;
    remove_file_if_exists(running_path).await?;
    Ok(failed_path)
}

pub async fn has_pending_jobs(project_root: &Path) -> Result<bool> {
    Ok(!list_pending_jobs(project_root).await?.is_empty())
}

pub async fn active_job_counts(project_root: &Path) -> Result<(usize, usize)> {
    let pending = list_pending_jobs(project_root).await?.len();
    let running = read_json_files(gepa_running_dir(project_root), usize::MAX)
        .await?
        .len();
    Ok((pending, running))
}

pub async fn recover_stale_running_jobs(
    project_root: &Path,
    stale_after_seconds: u64,
) -> Result<usize> {
    let running_dir = gepa_running_dir(project_root);
    let mut entries = match tokio::fs::read_dir(&running_dir).await {
        Ok(entries) => entries,
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => return Ok(0),
        Err(error) => return Err(error.into()),
    };
    let mut recovered = 0usize;
    while let Some(entry) = entries.next_entry().await? {
        let path = entry.path();
        if path.extension().and_then(|value| value.to_str()) != Some("json") {
            continue;
        }
        let metadata = match entry.metadata().await {
            Ok(metadata) => metadata,
            Err(_) => continue,
        };
        let raw = match tokio::fs::read(&path).await {
            Ok(raw) => raw,
            Err(_) => continue,
        };
        let mut job = match serde_json::from_slice::<PendingGepaJob>(&raw) {
            Ok(job) => job,
            Err(_) => continue,
        };
        let stale_after_seconds = job
            .optimizer_timeout_seconds
            .clamp(30, 6 * 60 * 60)
            .saturating_add(300)
            .max(stale_after_seconds);
        let stale = metadata
            .modified()
            .ok()
            .and_then(|modified| modified.elapsed().ok())
            .map(|elapsed| elapsed.as_secs() >= stale_after_seconds)
            .unwrap_or(false);
        if !stale {
            continue;
        }
        job.started_at = None;
        job.last_error = Some("Recovered after an interrupted GEPA worker".to_string());
        let pending_dir = gepa_pending_dir(project_root);
        tokio::fs::create_dir_all(&pending_dir).await?;
        let pending_path = pending_dir.join(format!("{}.json", job.job_id));
        write_json_file_atomic(&pending_path, &job).await?;
        remove_file_if_exists(&path).await?;
        recovered = recovered.saturating_add(1);
    }
    Ok(recovered)
}

pub async fn queue_status_snapshot(project_root: &Path, limit: usize) -> Result<Value> {
    Ok(serde_json::json!({
        "pending": read_job_files(gepa_pending_dir(project_root), "pending", limit).await?,
        "running": read_job_files(gepa_running_dir(project_root), "running", limit).await?,
        "completed": read_json_files(gepa_completed_dir(project_root), limit).await?,
        "failed": read_json_files(gepa_failed_dir(project_root), limit).await?,
    }))
}

pub async fn prune_gepa_artifacts(project_root: &Path) -> Result<GepaRetentionSummary> {
    let stale_running_jobs_requeued =
        recover_stale_running_jobs(project_root, DEFAULT_GEPA_OPTIMIZER_TIMEOUT_SECONDS + 300)
            .await?;
    let run_dirs_removed = prune_run_dirs(
        gepa_runs_dir(project_root),
        DEFAULT_GEPA_RETENTION_DAYS,
        DEFAULT_GEPA_MAX_RUN_DIRS,
    )
    .await?;
    let completed_removed = prune_status_files(
        gepa_completed_dir(project_root),
        DEFAULT_GEPA_RETENTION_DAYS,
    )
    .await?;
    let failed_removed =
        prune_status_files(gepa_failed_dir(project_root), DEFAULT_GEPA_RETENTION_DAYS).await?;
    Ok(GepaRetentionSummary {
        run_dirs_removed,
        status_files_removed: completed_removed.saturating_add(failed_removed),
        stale_running_jobs_requeued,
    })
}

pub async fn export_optimization_bundle(
    storage: &Storage,
    project_root: &Path,
    request: &str,
    recent_limit: u64,
) -> Result<GepaExportResult> {
    export_optimization_bundle_with_metadata(storage, project_root, request, recent_limit, None)
        .await
}

fn gepa_export_target_surfaces(job_metadata: Option<&Value>) -> Vec<&'static str> {
    const ALL: &[&str] = &[
        "prompt_bundle",
        "specialist_prompt_bundle",
        "prompt_fragment_bundle",
        "arkdistill_profile",
        "router_learning",
    ];
    let Some(items) = job_metadata
        .and_then(|metadata| metadata.get("target_gepa_surfaces"))
        .and_then(|value| value.as_array())
    else {
        return ALL.to_vec();
    };
    let mut out = Vec::new();
    for item in items {
        let Some(surface) = item.as_str().map(str::trim) else {
            continue;
        };
        if let Some(allowed) = ALL.iter().copied().find(|allowed| *allowed == surface) {
            if !out.contains(&allowed) {
                out.push(allowed);
            }
        }
    }
    if out.is_empty() {
        ALL.to_vec()
    } else {
        out
    }
}

pub async fn export_optimization_bundle_with_metadata(
    storage: &Storage,
    project_root: &Path,
    request: &str,
    recent_limit: u64,
    job_metadata: Option<&Value>,
) -> Result<GepaExportResult> {
    let run_id = format!("gepa-{}", uuid::Uuid::new_v4().simple());
    let run_dir = gepa_runs_dir(project_root).join(&run_id);
    tokio::fs::create_dir_all(&run_dir).await?;

    let prompt_bundle = storage
        .get(PROMPT_BUNDLE_PROFILE_KEY)
        .await?
        .and_then(|raw| parse_prompt_bundle_profile(&raw))
        .unwrap_or_default();
    let specialist_bundle = storage
        .get(SPECIALIST_PROMPT_BUNDLE_PROFILE_KEY)
        .await?
        .and_then(|raw| parse_specialist_prompt_bundle_profile(&raw))
        .unwrap_or_default();
    let prompt_fragment_bundle = storage
        .get(PROMPT_FRAGMENT_BUNDLE_PROFILE_KEY)
        .await?
        .and_then(|raw| parse_prompt_fragment_bundle_profile(&raw))
        .unwrap_or_else(default_prompt_fragment_bundle);
    let arkdistill_profile = storage
        .get(ARKDISTILL_PROFILE_KEY)
        .await?
        .and_then(|raw| crate::core::parse_arkdistill_profile(&raw))
        .unwrap_or_default();
    let recent_runs = storage
        .list_recent_experience_runs_any_scope(recent_limit.clamp(1, MAX_EXPORTED_EXPERIENCE_RUNS))
        .await
        .unwrap_or_default()
        .into_iter()
        .filter_map(redacted_experience_run)
        .collect::<Vec<_>>();
    let router_trace_evidence = export_recent_router_trace_evidence(storage, recent_limit).await;
    let target_surfaces = gepa_export_target_surfaces(job_metadata);
    let surface_enabled = |surface: &str| target_surfaces.contains(&surface);
    let mut surfaces = serde_json::Map::new();
    if surface_enabled("prompt_bundle") {
        surfaces.insert(
            "prompt_bundle".to_string(),
            serde_json::to_value(prompt_bundle)?,
        );
    }
    if surface_enabled("specialist_prompt_bundle") {
        surfaces.insert(
            "specialist_prompt_bundle".to_string(),
            serde_json::to_value(specialist_bundle)?,
        );
    }
    if surface_enabled("prompt_fragment_bundle") {
        surfaces.insert(
            "prompt_fragment_bundle".to_string(),
            serde_json::to_value(prompt_fragment_bundle)?,
        );
    }
    if surface_enabled("arkdistill_profile") {
        surfaces.insert(
            "arkdistill_profile".to_string(),
            serde_json::to_value(arkdistill_profile)?,
        );
    }
    let mut benchmarks = serde_json::Map::new();
    if surface_enabled("prompt_bundle") {
        benchmarks.insert(
            "prompt_bundle".to_string(),
            serde_json::from_str::<Value>(embedded_prompt_benchmark_profile_json())
                .unwrap_or(Value::Null),
        );
    }
    if surface_enabled("specialist_prompt_bundle") {
        benchmarks.insert(
            "specialist_prompt_bundle".to_string(),
            serde_json::from_str::<Value>(embedded_specialist_prompt_benchmark_profile_json())
                .unwrap_or(Value::Null),
        );
    }
    if surface_enabled("prompt_fragment_bundle") {
        benchmarks.insert(
            "prompt_fragment_bundle".to_string(),
            prompt_fragment_candidate_benchmark_profile(),
        );
    }
    if surface_enabled("arkdistill_profile") {
        benchmarks.insert("arkdistill_profile".to_string(), arkdistill_contract());
    }
    if surface_enabled("router_learning") {
        benchmarks.insert(
            "router_learning".to_string(),
            router_learning_benchmark_profile(),
        );
    }
    let mut recent_lineage = serde_json::Map::new();
    if surface_enabled("prompt_bundle") {
        recent_lineage.insert(
            "prompt_bundle".to_string(),
            Value::Array(
                read_recent_jsonl_values(
                    project_root.join(".agentark/self_evolve/prompt_bundle_lineage.jsonl"),
                    12,
                )
                .await,
            ),
        );
    }
    if surface_enabled("specialist_prompt_bundle") {
        recent_lineage.insert(
            "specialist_prompt_bundle".to_string(),
            Value::Array(
                read_recent_jsonl_values(
                    project_root
                        .join(".agentark/self_evolve/specialist_prompt_bundle_lineage.jsonl"),
                    12,
                )
                .await,
            ),
        );
    }
    if surface_enabled("prompt_fragment_bundle") {
        recent_lineage.insert(
            "prompt_fragment_bundle".to_string(),
            Value::Array(
                read_recent_jsonl_values(
                    project_root.join(PROMPT_FRAGMENT_LINEAGE_ARCHIVE_REL_PATH),
                    12,
                )
                .await,
            ),
        );
    }

    let bundle = serde_json::json!({
        "schema_version": 1,
        "run_id": run_id,
        "created_at": chrono::Utc::now().to_rfc3339(),
        "request": redact_and_truncate(request, MAX_EXPORTED_TEXT_CHARS),
        "job_metadata": job_metadata.cloned().unwrap_or(Value::Null),
        "opportunity_context": job_metadata
            .and_then(|metadata| metadata.get("opportunity_context"))
            .cloned()
            .unwrap_or(Value::Null),
        "surfaces": Value::Object(surfaces),
        "benchmarks": Value::Object(benchmarks),
        "recent_lineage": Value::Object(recent_lineage),
        "experience_runs": recent_runs,
        "router_trace_evidence": router_trace_evidence,
        "candidate_contract": {
            "format": "jsonl",
            "surfaces": target_surfaces,
            "required_fields": ["run_id", "surface", "source", "candidate", "objective_scores", "feedback_summary", "trace_refs", "created_at"]
        }
    });

    let export_path = run_dir.join("export.json");
    tokio::fs::write(&export_path, serde_json::to_vec_pretty(&bundle)?).await?;
    Ok(GepaExportResult {
        run_id: bundle
            .get("run_id")
            .and_then(|value| value.as_str())
            .unwrap_or_default()
            .to_string(),
        export_path: export_path.display().to_string(),
        candidates_path: run_dir.join("candidates.jsonl").display().to_string(),
        experience_samples: bundle
            .get("experience_runs")
            .and_then(|value| value.as_array())
            .map(|items| items.len())
            .unwrap_or_default(),
    })
}

pub async fn run_python_optimizer(
    export_path: &Path,
    candidates_path: &Path,
    timeout_seconds: u64,
    runtime: &GepaOptimizerRuntime,
) -> Result<GepaRunResult> {
    let export_metadata = tokio::fs::metadata(export_path)
        .await
        .with_context(|| format!("failed to inspect GEPA export at {:?}", export_path))?;
    if export_metadata.len() > MAX_EXPORT_FILE_BYTES {
        anyhow::bail!(
            "GEPA export file is too large: {} bytes exceeds {} bytes",
            export_metadata.len(),
            MAX_EXPORT_FILE_BYTES
        );
    }
    if let Some(parent) = candidates_path.parent() {
        tokio::fs::create_dir_all(parent).await?;
    }
    let timeout_seconds = timeout_seconds.clamp(30, 6 * 60 * 60);
    let mut command = tokio::process::Command::new(&runtime.python_path);
    command
        .arg("-m")
        .arg("bridges.gepa_optimizer")
        .arg("run")
        .arg("--export")
        .arg(export_path)
        .arg("--out")
        .arg(candidates_path)
        .stdout(std::process::Stdio::piped())
        .stderr(std::process::Stdio::piped())
        .kill_on_drop(true);
    apply_gepa_optimizer_env(&mut command, runtime);
    let child = command
        .spawn()
        .context("failed to start GEPA optimizer process")?;
    let output = match tokio::time::timeout(
        Duration::from_secs(timeout_seconds),
        child.wait_with_output(),
    )
    .await
    {
        Ok(output) => output.context("failed while waiting for GEPA optimizer process")?,
        Err(_) => {
            return Ok(GepaRunResult {
                status: "timed_out".to_string(),
                export_path: export_path.display().to_string(),
                candidates_path: candidates_path.display().to_string(),
                timeout_seconds,
                exit_code: None,
                actual_cost_usd: None,
                stdout_tail: String::new(),
                stderr_tail: format!("GEPA optimizer timed out after {} seconds", timeout_seconds),
            });
        }
    };
    // Exit code 3 = the optimizer fell back to schema-preserving baseline
    // copies (no real search happened). That is a failure for accounting and
    // UI purposes, never a completed optimization.
    let status = if output.status.success() {
        "completed"
    } else if output.status.code() == Some(3) {
        "failed_fallback"
    } else {
        "failed"
    };
    let stdout_tail = tail_string(&String::from_utf8_lossy(&output.stdout), 4000);
    Ok(GepaRunResult {
        status: status.to_string(),
        export_path: export_path.display().to_string(),
        candidates_path: candidates_path.display().to_string(),
        timeout_seconds,
        exit_code: output.status.code(),
        actual_cost_usd: gepa_stdout_actual_cost_usd(&stdout_tail),
        stdout_tail,
        stderr_tail: tail_string(&String::from_utf8_lossy(&output.stderr), 4000),
    })
}

fn gepa_stdout_actual_cost_usd(stdout: &str) -> Option<f64> {
    stdout.lines().rev().find_map(|line| {
        let value = serde_json::from_str::<Value>(line.trim()).ok()?;
        value
            .get("actual_cost_usd")
            .or_else(|| value.get("cost_usd"))
            .and_then(|value| value.as_f64())
            .filter(|value| value.is_finite() && *value >= 0.0)
    })
}

fn apply_gepa_optimizer_env(command: &mut tokio::process::Command, runtime: &GepaOptimizerRuntime) {
    command.env("MALLOC_ARENA_MAX", "2");
    command.env("OMP_NUM_THREADS", "1");
    command.env("OPENBLAS_NUM_THREADS", "1");
    command.env("MKL_NUM_THREADS", "1");
    command.env("NUMEXPR_NUM_THREADS", "1");
    command.env("TOKENIZERS_PARALLELISM", "false");
    command.env("AGENTARK_GEPA_MODEL", &runtime.model);
    command.env("AGENTARK_GEPA_AUTO", &runtime.auto_mode);
    command.env(
        "AGENTARK_GEPA_MAX_METRIC_CALLS",
        runtime.max_metric_calls.to_string(),
    );
    command.env("AGENTARK_GEPA_NUM_THREADS", runtime.num_threads.to_string());
    command.env("AGENTARK_GEPA_THREADS", runtime.num_threads.to_string());
    command.env(
        "AGENTARK_GEPA_COST_BUDGET_USD",
        format!("{:.4}", runtime.per_run_budget_usd.max(0.0)),
    );
    for (key, value) in &runtime.env {
        command.env(key, value);
    }
}

pub async fn validate_python_optimizer_runtime(runtime: &GepaOptimizerRuntime) -> Result<()> {
    let mut command = tokio::process::Command::new(&runtime.python_path);
    command
        .arg("-m")
        .arg("bridges.gepa_optimizer")
        .arg("validate")
        .stdout(std::process::Stdio::piped())
        .stderr(std::process::Stdio::piped())
        .kill_on_drop(true);
    apply_gepa_optimizer_env(&mut command, runtime);
    let child = command
        .spawn()
        .context("failed to start GEPA optimizer preflight process")?;
    let output = match tokio::time::timeout(Duration::from_secs(45), child.wait_with_output()).await
    {
        Ok(output) => output.context("failed while waiting for GEPA optimizer preflight")?,
        Err(_) => anyhow::bail!("GEPA optimizer preflight timed out after 45 seconds"),
    };
    if output.status.success() {
        return Ok(());
    }
    let stderr = tail_string(&String::from_utf8_lossy(&output.stderr), 1600);
    let stdout = tail_string(&String::from_utf8_lossy(&output.stdout), 1600);
    let detail = [stderr.trim(), stdout.trim()]
        .into_iter()
        .filter(|value| !value.is_empty())
        .collect::<Vec<_>>()
        .join("\n");
    anyhow::bail!(
        "GEPA optimizer preflight failed{}",
        if detail.is_empty() {
            String::new()
        } else {
            format!(": {}", detail)
        }
    );
}

pub(crate) async fn import_candidates(candidates_path: &Path) -> Result<GepaImportedCandidates> {
    let metadata = tokio::fs::metadata(candidates_path)
        .await
        .with_context(|| format!("failed to inspect GEPA candidates at {:?}", candidates_path))?;
    if metadata.len() > MAX_CANDIDATES_FILE_BYTES {
        anyhow::bail!(
            "GEPA candidates file is too large: {} bytes exceeds {} bytes",
            metadata.len(),
            MAX_CANDIDATES_FILE_BYTES
        );
    }
    let raw = tokio::fs::read_to_string(candidates_path)
        .await
        .with_context(|| format!("failed to read GEPA candidates at {:?}", candidates_path))?;
    let records = parse_candidate_records(&raw)?;
    let mut prompt_candidates = Vec::new();
    let mut specialist_prompt_candidates = Vec::new();
    let mut prompt_fragment_candidates = Vec::new();
    let mut arkdistill_candidates = Vec::new();
    let mut router_learning_candidates = Vec::new();
    let mut rejected_candidates = Vec::new();

    for record in records {
        if serde_json::to_vec(&record.candidate)
            .map(|bytes| bytes.len())
            .unwrap_or(usize::MAX)
            > MAX_JSONL_CANDIDATE_BYTES
        {
            rejected_candidates.push(format!(
                "{}:{} rejected because candidate JSON is too large",
                record.run_id, record.surface
            ));
            continue;
        }
        let source = format!(
            "gepa:{}:{}",
            record.run_id.trim(),
            record.source.trim().if_empty("candidate")
        );
        match record.surface.trim() {
            "prompt_bundle" => {
                match serde_json::from_value::<PromptBundleProfile>(record.candidate.clone()) {
                    Ok(mut bundle) => {
                        super::prompt_evolution::sanitize_prompt_bundle(&mut bundle);
                        prompt_candidates.push(ExternalPromptCandidate { source, bundle });
                    }
                    Err(error) => rejected_candidates.push(format!(
                        "{}:prompt_bundle rejected because profile JSON was invalid: {}",
                        record.run_id, error
                    )),
                }
            }
            "specialist_prompt_bundle" => {
                match serde_json::from_value::<SpecialistPromptBundleProfile>(
                    record.candidate.clone(),
                ) {
                    Ok(mut bundle) => {
                        super::specialist_prompt_evolution::sanitize_specialist_prompt_bundle(
                            &mut bundle,
                        );
                        specialist_prompt_candidates
                            .push(ExternalSpecialistPromptCandidate { source, bundle });
                    }
                    Err(error) => rejected_candidates.push(format!(
                        "{}:specialist_prompt_bundle rejected because profile JSON was invalid: {}",
                        record.run_id, error
                    )),
                }
            }
            "prompt_fragment_bundle" => {
                match serde_json::from_value::<PromptFragmentBundleProfile>(
                    record.candidate.clone(),
                ) {
                    Ok(mut bundle) => {
                        sanitize_prompt_fragment_bundle(&mut bundle);
                        prompt_fragment_candidates
                            .push(ExternalPromptFragmentCandidate { source, bundle });
                    }
                    Err(error) => rejected_candidates.push(format!(
                        "{}:prompt_fragment_bundle rejected because profile JSON was invalid: {}",
                        record.run_id, error
                    )),
                }
            }
            "arkdistill_profile" => {
                match serde_json::from_value::<ArkDistillProfile>(record.candidate.clone()) {
                    Ok(profile) => {
                        let profile = sanitize_arkdistill_profile(profile);
                        match validate_arkdistill_candidate(&profile) {
                            Ok(()) => arkdistill_candidates
                                .push(ExternalArkDistillCandidate { source, profile }),
                            Err(error) => rejected_candidates.push(format!(
                                "{}:arkdistill_profile rejected because candidate failed validation: {}",
                                record.run_id, error
                            )),
                        }
                    }
                    Err(error) => rejected_candidates.push(format!(
                        "{}:arkdistill_profile rejected because profile JSON was invalid: {}",
                        record.run_id, error
                    )),
                }
            }
            "router_learning" => {
                match serde_json::from_value::<RouterLearningCandidatePayload>(
                    record.candidate.clone(),
                ) {
                    Ok(payload) => match validate_router_learning_candidate(&payload) {
                        Ok(()) => router_learning_candidates.push(payload),
                        Err(error) => rejected_candidates.push(format!(
                            "{}:router_learning rejected because candidate failed validation: {}",
                            record.run_id, error
                        )),
                    },
                    Err(error) => rejected_candidates.push(format!(
                        "{}:router_learning rejected because candidate JSON was invalid: {}",
                        record.run_id, error
                    )),
                }
            }
            other => rejected_candidates.push(format!(
                "{}:{} rejected because surface is not supported",
                record.run_id, other
            )),
        }
    }

    Ok(GepaImportedCandidates {
        summary: GepaImportSummary {
            candidates_path: candidates_path.display().to_string(),
            prompt_candidates: prompt_candidates.len(),
            specialist_prompt_candidates: specialist_prompt_candidates.len(),
            prompt_fragment_candidates: prompt_fragment_candidates.len(),
            arkdistill_candidates: arkdistill_candidates.len(),
            router_learning_candidates: router_learning_candidates.len(),
            rejected_candidates,
        },
        prompt_candidates,
        specialist_prompt_candidates,
        prompt_fragment_candidates,
        arkdistill_candidates,
        router_learning_candidates,
    })
}

fn parse_candidate_records(raw: &str) -> Result<Vec<GepaCandidateRecord>> {
    let trimmed = raw.trim();
    if trimmed.is_empty() {
        return Ok(Vec::new());
    }
    if trimmed.starts_with('[') {
        let records = serde_json::from_str::<Vec<GepaCandidateRecord>>(trimmed)
            .context("failed to parse GEPA candidate array")?;
        if records.len() > MAX_CANDIDATE_RECORDS {
            anyhow::bail!(
                "GEPA candidate array contains {} records; maximum is {}",
                records.len(),
                MAX_CANDIDATE_RECORDS
            );
        }
        return Ok(records);
    }
    let mut records = Vec::new();
    for (idx, line) in raw.lines().enumerate() {
        let line = line.trim();
        if line.is_empty() {
            continue;
        }
        if line.len() > MAX_JSONL_RECORD_BYTES {
            anyhow::bail!(
                "GEPA candidate JSONL line {} is too large; maximum is {} bytes",
                idx + 1,
                MAX_JSONL_RECORD_BYTES
            );
        }
        if records.len() >= MAX_CANDIDATE_RECORDS {
            anyhow::bail!(
                "GEPA candidates contain more than {} records",
                MAX_CANDIDATE_RECORDS
            );
        }
        let record = serde_json::from_str::<GepaCandidateRecord>(line)
            .with_context(|| format!("failed to parse GEPA candidate JSONL line {}", idx + 1))?;
        records.push(record);
    }
    Ok(records)
}

async fn read_recent_jsonl_values(path: PathBuf, limit: usize) -> Vec<Value> {
    let raw = match tokio::fs::read_to_string(path).await {
        Ok(raw) => raw,
        Err(_) => return Vec::new(),
    };
    let mut values = raw
        .lines()
        .filter_map(|line| serde_json::from_str::<Value>(line).ok())
        .collect::<Vec<_>>();
    if values.len() > limit {
        values = values.split_off(values.len().saturating_sub(limit));
    }
    values
}

fn redacted_experience_run(run: crate::storage::entities::experience_run::Model) -> Option<Value> {
    if !experience_run_export_safe(&run) {
        return None;
    }
    let metadata_shape = json_shape_summary(&run.metadata);
    let request_text = run
        .request_text
        .as_deref()
        .map(|value| redact_and_truncate(value, MAX_EXPORTED_TEXT_CHARS));
    let outcome_summary = run
        .outcome_summary
        .as_deref()
        .map(|value| redact_and_truncate(value, MAX_EXPORTED_TEXT_CHARS));
    let failure_reason = run
        .failure_reason
        .as_deref()
        .map(|value| redact_and_truncate(value, MAX_EXPORTED_TEXT_CHARS));
    Some(serde_json::json!({
        "id": run.id,
        "trace_id": run.trace_id,
        "channel": run.channel,
        "scope": run.scope,
        "intent_key": run.intent_key,
        "task_type": run.task_type,
        "request_text": request_text,
        "tool_sequence_digest": run.tool_sequence_digest,
        "tool_sequence_shape": json_shape_summary(&run.tool_sequence_json),
        "strategy_version": run.strategy_version,
        "policy_version": run.policy_version,
        "prompt_version": run.prompt_version,
        "model_slot": run.model_slot,
        "success_state": run.success_state,
        "correction_state": run.correction_state,
        "outcome_summary": outcome_summary,
        "failure_reason": failure_reason,
        "metadata_shape": metadata_shape,
        "created_at": run.created_at,
        "updated_at": run.updated_at,
    }))
}

fn experience_run_export_safe(run: &crate::storage::entities::experience_run::Model) -> bool {
    let metadata = &run.metadata;
    let sensitive = metadata
        .get("sensitive")
        .and_then(|value| value.as_bool())
        .unwrap_or(false)
        || metadata
            .get("contains_sensitive_data")
            .and_then(|value| value.as_bool())
            .unwrap_or(false)
        || metadata
            .get("privacy_safe")
            .and_then(|value| value.as_bool())
            .is_some_and(|value| !value);
    !sensitive
}

async fn export_recent_router_trace_evidence(storage: &Storage, recent_limit: u64) -> Vec<Value> {
    let rows = storage
        .list_execution_trace_summaries(None, recent_limit.clamp(1, 120), 0)
        .await
        .unwrap_or_default();
    rows.into_iter()
        .filter_map(|row| {
            let steps =
                serde_json::from_str::<Vec<crate::core::ExecutionStep>>(&row.steps_json).ok()?;
            let evidence = trace_evidence_from_semantic_steps(
                row.id.clone(),
                redact_and_truncate(&row.message, MAX_EXPORTED_TEXT_CHARS),
                &steps,
            );
            if evidence.semantic_plan.is_none()
                && evidence.plan_verification.is_none()
                && evidence.capability_resolution.is_none()
                && evidence.result_verification.is_none()
                && evidence.router_budget.is_none()
            {
                return None;
            }
            Some(serde_json::json!({
                "trace_id": evidence.trace_id,
                "user_message_preview": evidence.user_message_preview,
                "semantic_plan": evidence.semantic_plan,
                "plan_verification": evidence.plan_verification,
                "capability_resolution": evidence.capability_resolution,
                "result_verification": evidence.result_verification,
                "router_budget": evidence.router_budget,
                "execution_policy": evidence.execution_policy,
                "capability_snapshot": evidence.capability_snapshot,
                "selected_tool_names": evidence.selected_tool_names,
                "native_schema_count": evidence.native_schema_count,
                "last_prompt_chars": evidence.last_prompt_chars,
                "direct_response_without_tool": evidence.direct_response_without_tool,
                "trace_summary": {
                    "channel": row.channel,
                    "duration_ms": row.duration_ms,
                    "step_count": row.step_count,
                    "total_tokens": row.total_tokens,
                    "cost_usd": row.cost_usd,
                    "complexity": row.complexity,
                    "created_at": row.created_at,
                }
            }))
        })
        .collect()
}

fn redact_and_truncate(raw: &str, max_chars: usize) -> String {
    crate::security::redact_pii(&truncate_chars(raw, max_chars))
}

fn truncate_chars(raw: &str, max_chars: usize) -> String {
    if raw.chars().count() <= max_chars {
        raw.to_string()
    } else {
        raw.chars().take(max_chars).collect()
    }
}

fn tail_string(raw: &str, max_chars: usize) -> String {
    let len = raw.chars().count();
    if len <= max_chars {
        raw.to_string()
    } else {
        raw.chars().skip(len.saturating_sub(max_chars)).collect()
    }
}

fn json_shape_summary(value: &Value) -> Value {
    match value {
        Value::Null => serde_json::json!({ "kind": "null" }),
        Value::Bool(_) => serde_json::json!({ "kind": "bool" }),
        Value::Number(_) => serde_json::json!({ "kind": "number" }),
        Value::String(value) => serde_json::json!({
            "kind": "string",
            "chars": value.chars().count(),
        }),
        Value::Array(items) => serde_json::json!({
            "kind": "array",
            "len": items.len(),
        }),
        Value::Object(map) => serde_json::json!({
            "kind": "object",
            "len": map.len(),
        }),
    }
}

async fn write_json_file_atomic<T: Serialize + ?Sized>(path: &Path, value: &T) -> Result<()> {
    if let Some(parent) = path.parent() {
        tokio::fs::create_dir_all(parent).await?;
    }
    let bytes = serde_json::to_vec_pretty(value)?;
    if tokio::fs::metadata(path).await.is_ok() {
        tokio::fs::write(path, bytes).await?;
        return Ok(());
    }
    let tmp_path = path.with_extension(format!("tmp-{}", uuid::Uuid::new_v4().simple()));
    tokio::fs::write(&tmp_path, bytes).await?;
    match tokio::fs::rename(&tmp_path, path).await {
        Ok(_) => Ok(()),
        Err(error) => {
            let _ = remove_file_if_exists(&tmp_path).await;
            Err(error.into())
        }
    }
}

async fn remove_file_if_exists(path: &Path) -> Result<()> {
    match tokio::fs::remove_file(path).await {
        Ok(_) => Ok(()),
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => Ok(()),
        Err(error) => Err(error.into()),
    }
}

async fn read_job_files(dir: PathBuf, status: &str, limit: usize) -> Result<Vec<Value>> {
    let values = read_json_files(dir, limit).await?;
    Ok(values
        .into_iter()
        .map(|job| {
            serde_json::json!({
                "status": status,
                "job": job,
            })
        })
        .collect())
}

async fn read_json_files(dir: PathBuf, limit: usize) -> Result<Vec<Value>> {
    let mut entries = match tokio::fs::read_dir(&dir).await {
        Ok(entries) => entries,
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => return Ok(Vec::new()),
        Err(error) => return Err(error.into()),
    };
    let mut files = Vec::new();
    while let Some(entry) = entries.next_entry().await? {
        let path = entry.path();
        if path.extension().and_then(|value| value.to_str()) != Some("json") {
            continue;
        }
        let modified = entry
            .metadata()
            .await
            .ok()
            .and_then(|metadata| metadata.modified().ok())
            .unwrap_or(UNIX_EPOCH);
        files.push((modified, path));
    }
    files.sort_by_key(|item| std::cmp::Reverse(item.0));
    let mut values = Vec::new();
    for (_, path) in files.into_iter().take(limit.max(1)) {
        let raw = match tokio::fs::read(&path).await {
            Ok(raw) => raw,
            Err(_) => continue,
        };
        if let Ok(value) = serde_json::from_slice::<Value>(&raw) {
            values.push(value);
        }
    }
    Ok(values)
}

async fn prune_status_files(dir: PathBuf, retention_days: u64) -> Result<usize> {
    let cutoff = retention_cutoff(retention_days);
    let mut entries = match tokio::fs::read_dir(&dir).await {
        Ok(entries) => entries,
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => return Ok(0),
        Err(error) => return Err(error.into()),
    };
    let mut removed = 0usize;
    while let Some(entry) = entries.next_entry().await? {
        let path = entry.path();
        if path.extension().and_then(|value| value.to_str()) != Some("json") {
            continue;
        }
        let modified = entry
            .metadata()
            .await
            .ok()
            .and_then(|metadata| metadata.modified().ok())
            .unwrap_or(UNIX_EPOCH);
        if modified < cutoff {
            remove_file_if_exists(&path).await?;
            removed = removed.saturating_add(1);
        }
    }
    Ok(removed)
}

async fn prune_run_dirs(dir: PathBuf, retention_days: u64, max_run_dirs: usize) -> Result<usize> {
    let mut entries = match tokio::fs::read_dir(&dir).await {
        Ok(entries) => entries,
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => return Ok(0),
        Err(error) => return Err(error.into()),
    };
    let cutoff = retention_cutoff(retention_days);
    let mut run_dirs = Vec::new();
    while let Some(entry) = entries.next_entry().await? {
        let path = entry.path();
        let metadata = match entry.metadata().await {
            Ok(metadata) => metadata,
            Err(_) => continue,
        };
        if !metadata.is_dir() {
            continue;
        }
        let modified = metadata.modified().unwrap_or(UNIX_EPOCH);
        run_dirs.push((modified, path));
    }

    let mut remove_targets: HashSet<PathBuf> = run_dirs
        .iter()
        .filter(|(modified, _)| *modified < cutoff)
        .map(|(_, path)| path.clone())
        .collect();
    run_dirs.sort_by_key(|item| std::cmp::Reverse(item.0));
    for (_, path) in run_dirs.into_iter().skip(max_run_dirs.max(1)) {
        remove_targets.insert(path);
    }

    let mut removed = 0usize;
    for path in remove_targets {
        match tokio::fs::remove_dir_all(&path).await {
            Ok(_) => removed = removed.saturating_add(1),
            Err(error) if error.kind() == std::io::ErrorKind::NotFound => {}
            Err(error) => return Err(error.into()),
        }
    }
    Ok(removed)
}

fn retention_cutoff(retention_days: u64) -> SystemTime {
    SystemTime::now()
        .checked_sub(Duration::from_secs(
            retention_days.saturating_mul(24 * 60 * 60),
        ))
        .unwrap_or(UNIX_EPOCH)
}

#[derive(Debug, Clone)]
struct SelectedGepaModelRuntime {
    model: String,
    provider_label: String,
    provider_key_ready: bool,
    env: HashMap<String, String>,
}

fn normalize_gepa_optimizer_config(mut config: GepaOptimizerConfig) -> GepaOptimizerConfig {
    let auto = config.auto_mode.trim().to_ascii_lowercase();
    config.auto_mode = if matches!(auto.as_str(), "light" | "medium" | "heavy") {
        auto
    } else {
        default_gepa_auto_mode()
    };
    config.max_metric_calls = config.max_metric_calls.clamp(1, MAX_GEPA_METRIC_CALLS);
    config.daily_budget_usd = config.daily_budget_usd.clamp(0.0, 500.0);
    config.per_run_budget_usd = config.per_run_budget_usd.clamp(0.0, 100.0);
    config.max_runs_per_day = config.max_runs_per_day.clamp(0, 100);
    config.timeout_seconds = config.timeout_seconds.clamp(60, 6 * 60 * 60);
    config.num_threads = config.num_threads.clamp(1, 8);
    config
}

pub fn estimate_gepa_run(
    config: &GepaOptimizerConfig,
    estimated_cost_usd: Option<f64>,
) -> GepaRunEstimate {
    let config = normalize_gepa_optimizer_config(config.clone());
    let estimated_calls = GEPA_ESTIMATE_BASE_CALLS.saturating_add(config.max_metric_calls);
    let estimated_total_tokens = u64::from(estimated_calls).saturating_mul(
        GEPA_ESTIMATE_INPUT_TOKENS_PER_CALL.saturating_add(GEPA_ESTIMATE_OUTPUT_TOKENS_PER_CALL),
    );
    let threads = config.num_threads.max(1);
    let estimated_seconds = u64::from(estimated_calls)
        .saturating_mul(GEPA_ESTIMATE_SECONDS_PER_CALL)
        .div_ceil(u64::from(threads));
    GepaRunEstimate {
        estimated_calls,
        input_tokens_per_call: GEPA_ESTIMATE_INPUT_TOKENS_PER_CALL,
        output_tokens_per_call: GEPA_ESTIMATE_OUTPUT_TOKENS_PER_CALL,
        estimated_total_tokens,
        estimated_cost_usd: estimated_cost_usd.filter(|value| value.is_finite() && *value >= 0.0),
        estimated_minutes: estimated_seconds.div_ceil(60).max(1),
        num_threads: threads,
        max_metric_calls: config.max_metric_calls,
        timeout_seconds: config.timeout_seconds,
        approval_required: true,
    }
}

fn select_gepa_model_slot<'a>(
    config: &'a crate::core::runtime::config::AgentConfig,
    primary_model_id: &str,
) -> Option<&'a crate::core::runtime::config::ModelSlot> {
    config
        .model_pool
        .slots
        .iter()
        .find(|slot| slot.enabled && slot.id == primary_model_id)
        .or_else(|| {
            config.model_pool.slots.iter().find(|slot| {
                slot.enabled
                    && matches!(slot.role, crate::core::runtime::config::ModelRole::Primary)
            })
        })
        .or_else(|| config.model_pool.slots.iter().find(|slot| slot.enabled))
}

fn gepa_model_runtime_from_slot(
    slot: &crate::core::runtime::config::ModelSlot,
) -> Option<SelectedGepaModelRuntime> {
    let mut env = slot.provider.app_env_vars();
    match &slot.provider {
        crate::core::LlmProvider::Anthropic { api_key, model } => {
            let model = format_litellm_model("anthropic", model);
            env.insert("ANTHROPIC_API_KEY".to_string(), api_key.clone());
            Some(SelectedGepaModelRuntime {
                model,
                provider_label: "anthropic".to_string(),
                provider_key_ready: !api_key.trim().is_empty() && api_key != "[ENCRYPTED]",
                env,
            })
        }
        crate::core::LlmProvider::OpenAI {
            api_key,
            model,
            base_url,
        } => {
            let model = format_litellm_model("openai", model);
            env.insert("OPENAI_API_KEY".to_string(), api_key.clone());
            if let Some(base_url) = base_url
                .as_deref()
                .map(str::trim)
                .filter(|value| !value.is_empty())
            {
                env.insert("OPENAI_BASE_URL".to_string(), base_url.to_string());
            }
            Some(SelectedGepaModelRuntime {
                model,
                provider_label: "openai-compatible".to_string(),
                provider_key_ready: !api_key.trim().is_empty() && api_key != "[ENCRYPTED]",
                env,
            })
        }
        crate::core::LlmProvider::Ollama { base_url, model } => {
            let model = format_litellm_model("openai", model);
            env.insert(
                "OPENAI_BASE_URL".to_string(),
                format!("{}/v1", base_url.trim_end_matches('/')),
            );
            env.insert("OPENAI_API_KEY".to_string(), "ollama".to_string());
            Some(SelectedGepaModelRuntime {
                model,
                provider_label: "ollama".to_string(),
                provider_key_ready: !base_url.trim().is_empty(),
                env,
            })
        }
    }
}

fn format_litellm_model(provider: &str, model: &str) -> String {
    let trimmed = model.trim();
    if trimmed.contains('/') {
        trimmed.to_string()
    } else {
        format!("{}/{}", provider, trimmed)
    }
}

fn configured_gepa_python(project_root: &Path) -> PathBuf {
    if let Some(path) = bundled_gepa_python() {
        return path;
    }
    let venv_python = gepa_venv_python(project_root);
    if venv_python.exists() {
        return venv_python;
    }
    if cfg!(windows) {
        PathBuf::from("python")
    } else {
        PathBuf::from("python3")
    }
}

async fn command_runs(command: &Path, args: &[&str]) -> bool {
    let mut child = tokio::process::Command::new(command);
    child
        .args(args)
        .stdout(std::process::Stdio::null())
        .stderr(std::process::Stdio::null());
    let Ok(status) = tokio::time::timeout(Duration::from_secs(8), child.status()).await else {
        return false;
    };
    status.map(|status| status.success()).unwrap_or(false)
}

async fn load_gepa_budget_ledger(storage: &Storage) -> GepaBudgetLedger {
    storage
        .get(GEPA_OPTIMIZER_BUDGET_LEDGER_KEY)
        .await
        .ok()
        .flatten()
        .and_then(|raw| serde_json::from_slice::<GepaBudgetLedger>(&raw).ok())
        .unwrap_or_default()
}

async fn gepa_budget_status(storage: &Storage, config: &GepaOptimizerConfig) -> GepaBudgetStatus {
    let mut ledger = load_gepa_budget_ledger(storage).await;
    prune_gepa_budget_ledger(&mut ledger);
    gepa_budget_status_from_ledger(config, &ledger)
}

fn gepa_budget_status_from_ledger(
    config: &GepaOptimizerConfig,
    ledger: &GepaBudgetLedger,
) -> GepaBudgetStatus {
    gepa_budget_status_from_ledger_at(config, ledger, chrono::Utc::now(), gepa_budget_timezone())
}

fn gepa_budget_timezone() -> Option<chrono_tz::Tz> {
    ["AGENTARK_BUDGET_TIMEZONE", "AGENTARK_LOG_TIMEZONE", "TZ"]
        .into_iter()
        .filter_map(|key| std::env::var(key).ok())
        .map(|value| value.trim().to_string())
        .filter(|value| !value.is_empty())
        .find_map(|value| value.parse::<chrono_tz::Tz>().ok())
}

fn gepa_budget_local_date(
    timestamp: chrono::DateTime<chrono::Utc>,
    timezone: Option<chrono_tz::Tz>,
) -> chrono::NaiveDate {
    match timezone {
        Some(timezone) => timestamp.with_timezone(&timezone).date_naive(),
        None => timestamp.date_naive(),
    }
}

fn gepa_budget_status_from_ledger_at(
    config: &GepaOptimizerConfig,
    ledger: &GepaBudgetLedger,
    now: chrono::DateTime<chrono::Utc>,
    timezone: Option<chrono_tz::Tz>,
) -> GepaBudgetStatus {
    let today = gepa_budget_local_date(now, timezone);
    let todays_entries = ledger.entries.iter().filter(|entry| {
        chrono::DateTime::parse_from_rfc3339(&entry.recorded_at)
            .map(|timestamp| {
                gepa_budget_local_date(timestamp.with_timezone(&chrono::Utc), timezone) == today
            })
            .unwrap_or(false)
    });
    let mut runs_today = 0u32;
    let mut used_today_usd = 0.0f64;
    for entry in todays_entries {
        runs_today = runs_today.saturating_add(1);
        if gepa_budget_entry_counts_as_spend(entry) {
            used_today_usd += entry.reserved_usd.max(0.0);
        }
    }
    let daily_budget_usd = config.daily_budget_usd.max(0.0);
    let per_run_budget_usd = config.per_run_budget_usd.max(0.0);
    let max_runs_per_day = config.max_runs_per_day;
    let remaining_today_usd = if daily_budget_usd > 0.0 {
        (daily_budget_usd - used_today_usd).max(0.0)
    } else {
        0.0
    };
    let reason = if max_runs_per_day > 0 && runs_today >= max_runs_per_day {
        Some("GEPA daily run count limit has been reached.".to_string())
    } else if daily_budget_usd > 0.0 && per_run_budget_usd > daily_budget_usd {
        Some("GEPA per-run budget is larger than the daily budget.".to_string())
    } else if daily_budget_usd > 0.0 && per_run_budget_usd > remaining_today_usd {
        Some("GEPA daily spend budget has been reached.".to_string())
    } else {
        None
    };
    GepaBudgetStatus {
        daily_budget_usd,
        per_run_budget_usd,
        max_runs_per_day,
        used_today_usd,
        runs_today,
        remaining_today_usd,
        allowed: reason.is_none(),
        reason,
    }
}

fn gepa_budget_entry_counts_as_spend(entry: &GepaBudgetLedgerEntry) -> bool {
    matches!(
        entry.status.trim(),
        "spent" | "charged" | "actual" | "completed_spend"
    )
}

fn finalize_gepa_budget_ledger_entry(
    ledger: &mut GepaBudgetLedger,
    run_id: &str,
    spent_usd: f64,
    status: &str,
    now: chrono::DateTime<chrono::Utc>,
) {
    let spent_usd = spent_usd.max(0.0);
    let status = status.trim().if_empty("spent").to_string();
    if let Some(entry) = ledger
        .entries
        .iter_mut()
        .find(|entry| entry.run_id == run_id)
    {
        entry.reserved_usd = spent_usd;
        entry.status = status;
        entry.recorded_at = now.to_rfc3339();
        return;
    }
    ledger.entries.push(GepaBudgetLedgerEntry {
        run_id: run_id.to_string(),
        reserved_usd: spent_usd,
        status,
        recorded_at: now.to_rfc3339(),
    });
}

fn gepa_budget_has_today_reservation(
    ledger: &GepaBudgetLedger,
    run_id: &str,
    now: chrono::DateTime<chrono::Utc>,
    timezone: Option<chrono_tz::Tz>,
) -> bool {
    let today = gepa_budget_local_date(now, timezone);
    ledger.entries.iter().any(|entry| {
        entry.run_id == run_id
            && chrono::DateTime::parse_from_rfc3339(&entry.recorded_at)
                .map(|timestamp| {
                    gepa_budget_local_date(timestamp.with_timezone(&chrono::Utc), timezone) == today
                })
                .unwrap_or(false)
    })
}

fn gepa_budget_status_for_reservation_at(
    config: &GepaOptimizerConfig,
    ledger: &GepaBudgetLedger,
    run_id: &str,
    now: chrono::DateTime<chrono::Utc>,
    timezone: Option<chrono_tz::Tz>,
) -> GepaBudgetStatus {
    let ledger_without_existing_run = GepaBudgetLedger {
        entries: ledger
            .entries
            .iter()
            .filter(|entry| entry.run_id != run_id)
            .cloned()
            .collect(),
    };
    gepa_budget_status_from_ledger_at(config, &ledger_without_existing_run, now, timezone)
}

fn prune_gepa_budget_ledger(ledger: &mut GepaBudgetLedger) {
    let cutoff = chrono::Utc::now().date_naive() - chrono::Duration::days(7);
    ledger.entries.retain(|entry| {
        chrono::DateTime::parse_from_rfc3339(&entry.recorded_at)
            .map(|timestamp| timestamp.with_timezone(&chrono::Utc).date_naive() >= cutoff)
            .unwrap_or(false)
    });
    if ledger.entries.len() > 512 {
        ledger.entries = ledger
            .entries
            .split_off(ledger.entries.len().saturating_sub(512));
    }
}

fn default_gepa_quiet_window_seconds() -> i64 {
    DEFAULT_GEPA_QUIET_WINDOW_SECONDS
}

pub fn default_gepa_optimizer_timeout_seconds() -> u64 {
    DEFAULT_GEPA_OPTIMIZER_TIMEOUT_SECONDS
}

fn default_gepa_max_attempts() -> u32 {
    DEFAULT_GEPA_MAX_ATTEMPTS
}

fn default_apply_promotion() -> bool {
    true
}

fn default_canary_rollout_percent() -> u8 {
    20
}

fn default_canary_min_samples_per_version() -> usize {
    25
}

fn default_canary_min_success_gain() -> f64 {
    0.03
}

fn default_canary_max_sign_test_p_value() -> f64 {
    0.10
}

fn default_replay_log_limit() -> u64 {
    4_000
}

fn default_true() -> bool {
    true
}

fn default_gepa_auto_mode() -> String {
    "light".to_string()
}

fn default_gepa_enabled() -> bool {
    true
}

fn default_gepa_max_metric_calls() -> u32 {
    // Keep background optimization bounded enough to finish inside the
    // process timeout; promotion gates still decide whether output is safe.
    8
}

fn default_gepa_num_threads() -> u32 {
    // GEPA evaluates metrics concurrently; 4 is the spec default and
    // remains bounded by the bridge's background-thread cap.
    4
}

fn default_gepa_daily_budget_usd() -> f64 {
    1.0
}

fn default_gepa_per_run_budget_usd() -> f64 {
    0.50
}

fn default_gepa_max_runs_per_day() -> u32 {
    0
}

trait IfEmpty {
    fn if_empty<'a>(&'a self, fallback: &'a str) -> &'a str;
}

impl IfEmpty for str {
    fn if_empty<'a>(&'a self, fallback: &'a str) -> &'a str {
        if self.is_empty() {
            fallback
        } else {
            self
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn parse_candidate_records_accepts_jsonl() {
        let raw = r#"{"run_id":"r1","surface":"unknown","source":"s","candidate":{}}
{"run_id":"r1","surface":"unknown","source":"s2","candidate":{}}"#;
        let records = parse_candidate_records(raw).expect("records parse");
        assert_eq!(records.len(), 2);
    }

    #[test]
    fn normalize_gepa_optimizer_config_preserves_disabled_flag() {
        let config = normalize_gepa_optimizer_config(GepaOptimizerConfig {
            enabled: false,
            auto_mode: "invalid".to_string(),
            max_metric_calls: 999,
            daily_budget_usd: 999.0,
            per_run_budget_usd: 999.0,
            max_runs_per_day: 999,
            auto_setup: true,
            timeout_seconds: 999_999,
            num_threads: 999,
        });

        assert!(!config.enabled);
        assert_eq!(config.auto_mode, "light");
        assert_eq!(config.max_metric_calls, 8);
        assert_eq!(config.daily_budget_usd, 500.0);
        assert_eq!(config.per_run_budget_usd, 100.0);
        assert_eq!(config.max_runs_per_day, 100);
        assert_eq!(config.timeout_seconds, 6 * 60 * 60);
        assert_eq!(config.num_threads, 8);
    }

    #[test]
    fn gepa_promotion_settings_default_allows_gate_to_decide() {
        let settings = GepaPromotionSettings::default();
        assert!(settings.apply_promotion);
    }

    #[test]
    fn gepa_budget_day_uses_configured_timezone() {
        let config = GepaOptimizerConfig {
            max_runs_per_day: 1,
            ..GepaOptimizerConfig::default()
        };
        let ledger = GepaBudgetLedger {
            entries: vec![GepaBudgetLedgerEntry {
                run_id: "run-1".to_string(),
                reserved_usd: 0.50,
                status: "reserved".to_string(),
                recorded_at: "2026-05-27T14:39:54Z".to_string(),
            }],
        };
        let now = chrono::DateTime::parse_from_rfc3339("2026-05-27T18:35:00Z")
            .unwrap()
            .with_timezone(&chrono::Utc);

        let utc_status = gepa_budget_status_from_ledger_at(&config, &ledger, now, None);
        let local_status = gepa_budget_status_from_ledger_at(
            &config,
            &ledger,
            now,
            Some(chrono_tz::Asia::Kolkata),
        );

        assert_eq!(utc_status.runs_today, 1);
        assert!(!utc_status.allowed);
        assert_eq!(local_status.runs_today, 0);
        assert!(local_status.allowed);
    }

    #[test]
    fn gepa_budget_reservation_reuses_same_day_run_id() {
        let config = GepaOptimizerConfig {
            max_runs_per_day: 1,
            daily_budget_usd: 1.0,
            per_run_budget_usd: 0.5,
            ..GepaOptimizerConfig::default()
        };
        let ledger = GepaBudgetLedger {
            entries: vec![GepaBudgetLedgerEntry {
                run_id: "run-1".to_string(),
                reserved_usd: 0.50,
                status: "reserved".to_string(),
                recorded_at: "2026-05-29T10:00:00Z".to_string(),
            }],
        };
        let now = chrono::DateTime::parse_from_rfc3339("2026-05-29T11:00:00Z")
            .unwrap()
            .with_timezone(&chrono::Utc);

        let same_run = gepa_budget_status_for_reservation_at(&config, &ledger, "run-1", now, None);
        let different_run =
            gepa_budget_status_for_reservation_at(&config, &ledger, "run-2", now, None);

        assert!(
            same_run.allowed,
            "a retry of the same GEPA job should reuse its existing reservation"
        );
        assert!(
            !different_run.allowed,
            "a different GEPA job should still respect the daily run cap"
        );
    }

    #[test]
    fn gepa_budget_reservations_do_not_consume_daily_spend() {
        let config = GepaOptimizerConfig {
            max_runs_per_day: 0,
            daily_budget_usd: 2.0,
            per_run_budget_usd: 0.5,
            ..GepaOptimizerConfig::default()
        };
        let ledger = GepaBudgetLedger {
            entries: (0..4)
                .map(|idx| GepaBudgetLedgerEntry {
                    run_id: format!("run-{idx}"),
                    reserved_usd: 0.50,
                    status: "reserved".to_string(),
                    recorded_at: "2026-05-29T10:00:00Z".to_string(),
                })
                .collect(),
        };
        let now = chrono::DateTime::parse_from_rfc3339("2026-05-29T11:00:00Z")
            .unwrap()
            .with_timezone(&chrono::Utc);

        let status = gepa_budget_status_from_ledger_at(&config, &ledger, now, None);

        assert_eq!(status.runs_today, 4);
        assert_eq!(status.used_today_usd, 0.0);
        assert_eq!(status.remaining_today_usd, 2.0);
        assert!(status.allowed);
    }

    #[test]
    fn gepa_run_estimate_is_nonzero_and_parallel_aware() {
        let config = GepaOptimizerConfig {
            max_metric_calls: 8,
            timeout_seconds: 1800,
            num_threads: 4,
            daily_budget_usd: 1.0,
            per_run_budget_usd: 0.50,
            ..GepaOptimizerConfig::default()
        };

        let estimate = estimate_gepa_run(&config, Some(0.0123));

        assert_eq!(estimate.estimated_calls, 11);
        assert_eq!(estimate.num_threads, 4);
        assert!(estimate.estimated_total_tokens > 0);
        assert!(estimate.estimated_minutes > 0);
        assert_eq!(estimate.estimated_cost_usd, Some(0.0123));
        assert!(estimate.approval_required);
    }

    #[test]
    fn gepa_export_target_surfaces_filters_to_requested_surface() {
        let metadata = serde_json::json!({
            "target_gepa_surfaces": [
                "prompt_fragment_bundle",
                "prompt_fragment_bundle",
                "unknown"
            ]
        });

        assert_eq!(
            gepa_export_target_surfaces(Some(&metadata)),
            vec!["prompt_fragment_bundle"]
        );
        assert!(gepa_export_target_surfaces(None).contains(&"prompt_bundle"));
    }

    #[test]
    fn finalized_gepa_budget_entry_counts_as_spend() {
        let config = GepaOptimizerConfig {
            daily_budget_usd: 1.0,
            per_run_budget_usd: 0.5,
            max_runs_per_day: 0,
            ..GepaOptimizerConfig::default()
        };
        let now = chrono::Utc::now();
        let mut ledger = GepaBudgetLedger {
            entries: vec![GepaBudgetLedgerEntry {
                run_id: "run-1".to_string(),
                reserved_usd: 0.0,
                status: "reserved".to_string(),
                recorded_at: now.to_rfc3339(),
            }],
        };

        finalize_gepa_budget_ledger_entry(&mut ledger, "run-1", 0.25, "spent", now);
        let status = gepa_budget_status_from_ledger_at(&config, &ledger, now, None);

        assert_eq!(status.used_today_usd, 0.25);
        assert_eq!(ledger.entries[0].status, "spent");
    }

    #[test]
    fn gepa_budget_run_count_cap_only_applies_when_configured() {
        let ledger = GepaBudgetLedger {
            entries: vec![GepaBudgetLedgerEntry {
                run_id: "run-1".to_string(),
                reserved_usd: 0.0,
                status: "reserved".to_string(),
                recorded_at: "2026-05-29T10:00:00Z".to_string(),
            }],
        };
        let now = chrono::DateTime::parse_from_rfc3339("2026-05-29T11:00:00Z")
            .unwrap()
            .with_timezone(&chrono::Utc);
        let uncapped = gepa_budget_status_from_ledger_at(
            &GepaOptimizerConfig {
                max_runs_per_day: 0,
                ..GepaOptimizerConfig::default()
            },
            &ledger,
            now,
            None,
        );
        let capped = gepa_budget_status_from_ledger_at(
            &GepaOptimizerConfig {
                max_runs_per_day: 1,
                ..GepaOptimizerConfig::default()
            },
            &ledger,
            now,
            None,
        );

        assert!(uncapped.allowed);
        assert!(!capped.allowed);
        assert_eq!(
            capped.reason.as_deref(),
            Some("GEPA daily run count limit has been reached.")
        );
    }

    #[test]
    fn gepa_default_metric_call_budget_keeps_background_runs_bounded() {
        // Non-degenerate search floor; budget guards still bound spend.
        assert_eq!(GepaOptimizerConfig::default().max_metric_calls, 8);
    }

    #[test]
    fn experience_run_export_safe_blocks_sensitive_metadata() {
        let mut run = crate::storage::entities::experience_run::Model {
            id: "run".to_string(),
            execution_run_id: None,
            trace_id: None,
            conversation_id: None,
            project_id: None,
            channel: "web".to_string(),
            scope: "global".to_string(),
            intent_key: "task".to_string(),
            task_type: Some("task".to_string()),
            request_text: Some("hello".to_string()),
            tool_sequence_digest: None,
            tool_sequence_json: Value::Null,
            strategy_version: None,
            policy_version: None,
            prompt_version: None,
            model_slot: None,
            tokens_in: None,
            tokens_out: None,
            wall_ms: None,
            est_cost_microusd: None,
            success_state: "accepted".to_string(),
            correction_state: "none".to_string(),
            outcome_summary: None,
            failure_reason: None,
            metadata: Value::Null,
            consolidated: false,
            accepted_at: None,
            corrected_at: None,
            heuristic_reflected: false,
            heuristic_reflection_status: None,
            heuristic_reflection_attempted_at: None,
            heuristic_reflection_completed_at: None,
            heuristic_lesson_id: None,
            heuristic_reflection_error: None,
            created_at: "2026-01-01T00:00:00Z".to_string(),
            updated_at: "2026-01-01T00:00:00Z".to_string(),
        };
        assert!(experience_run_export_safe(&run));
        run.metadata = serde_json::json!({ "contains_sensitive_data": true });
        assert!(!experience_run_export_safe(&run));
    }

    #[test]
    fn gepa_timeout_and_threads_defaults() {
        assert_eq!(default_gepa_optimizer_timeout_seconds(), 1800);
        assert_eq!(default_gepa_num_threads(), 4);
    }

    #[test]
    fn gepa_config_deserializes_new_fields_with_defaults() {
        let c: GepaOptimizerConfig = serde_json::from_str("{}").unwrap();
        assert_eq!(c.timeout_seconds, 1800);
        assert_eq!(c.num_threads, 4);
        let d = GepaOptimizerConfig::default();
        assert_eq!(d.timeout_seconds, 1800);
        assert_eq!(d.num_threads, 4);
    }

    #[test]
    fn apply_env_sets_thread_count() {
        let runtime = GepaOptimizerRuntime {
            python_path: std::path::PathBuf::from("python3"),
            model: "anthropic/x".to_string(),
            env: std::collections::HashMap::new(),
            auto_mode: "light".to_string(),
            max_metric_calls: 8,
            per_run_budget_usd: 0.5,
            num_threads: 4,
        };
        let mut cmd = tokio::process::Command::new("python3");
        apply_gepa_optimizer_env(&mut cmd, &runtime);
        let found = cmd
            .as_std()
            .get_envs()
            .any(|(k, v)| k == "AGENTARK_GEPA_NUM_THREADS" && v == Some("4".as_ref()));
        assert!(found, "AGENTARK_GEPA_NUM_THREADS should be set to 4");
    }
}
