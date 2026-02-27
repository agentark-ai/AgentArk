//! Runtime strategy and canary utilities for self-evolution.
//!
//! Provides:
//! - prompt strategy profile structures
//! - deterministic canary selection
//! - offline replay/canary evaluation helpers from operational logs

use std::collections::HashMap;
use std::hash::{Hash, Hasher};

use serde::{Deserialize, Serialize};

pub const TOOL_STRATEGY_PROFILE_KEY: &str = "tool_strategy_profile_v1";
pub const TOOL_STRATEGY_PROFILE_CANARY_KEY: &str = "tool_strategy_profile_canary_v1";
pub const TOOL_STRATEGY_CANARY_STATE_KEY: &str = "tool_strategy_canary_state_v1";
pub const ROUTING_COMPLEXITY_POLICY_CANARY_KEY: &str = "routing_complexity_policy_canary_v1";
pub const ROUTING_COMPLEXITY_CANARY_STATE_KEY: &str = "routing_complexity_policy_canary_state_v1";
pub const ROUTING_COMPLEXITY_POLICY_BASELINE_SNAPSHOT_KEY: &str =
    "routing_complexity_policy_baseline_snapshot_v1";
pub const SELF_EVOLVE_LAST_RESULT_KEY: &str = "self_evolve_last_result_v1";
pub const APP_DEPLOY_ACCESS_GUARD_DEFAULT_KEY: &str = "app_deploy_access_guard_default_v1";

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ToolStrategyProfile {
    pub version: String,
    #[serde(default)]
    pub updated_at: Option<String>,
    #[serde(default)]
    pub default_guidance: Vec<String>,
    #[serde(default)]
    pub task_guidance: HashMap<String, Vec<String>>,
}

impl Default for ToolStrategyProfile {
    fn default() -> Self {
        Self {
            version: "strategy-v1".to_string(),
            updated_at: None,
            default_guidance: Vec::new(),
            task_guidance: HashMap::new(),
        }
    }
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct CanaryRolloutState {
    pub enabled: bool,
    pub baseline_version: String,
    pub candidate_version: String,
    pub rollout_percent: u8,
    #[serde(default = "default_min_samples")]
    pub min_samples_per_version: usize,
    #[serde(default = "default_min_success_gain")]
    pub min_success_gain: f64,
    #[serde(default = "default_max_sign_test_p")]
    pub max_sign_test_p_value: f64,
    #[serde(default)]
    pub activated_at: Option<String>,
}

fn default_min_samples() -> usize {
    25
}

fn default_min_success_gain() -> f64 {
    0.03
}

fn default_max_sign_test_p() -> f64 {
    0.10
}

impl Default for CanaryRolloutState {
    fn default() -> Self {
        Self {
            enabled: false,
            baseline_version: "baseline".to_string(),
            candidate_version: "candidate".to_string(),
            rollout_percent: 20,
            min_samples_per_version: default_min_samples(),
            min_success_gain: default_min_success_gain(),
            max_sign_test_p_value: default_max_sign_test_p(),
            activated_at: None,
        }
    }
}

#[derive(Debug, Clone, Serialize)]
pub struct ReplayVersionMetrics {
    pub samples: usize,
    pub successes: usize,
    pub success_rate: f64,
    pub p95_latency_ms: Option<i64>,
}

#[derive(Debug, Clone, Serialize)]
pub struct ReplayEvaluationResult {
    pub eligible: bool,
    pub promote: bool,
    pub baseline_version: String,
    pub candidate_version: String,
    pub baseline: ReplayVersionMetrics,
    pub candidate: ReplayVersionMetrics,
    pub success_gain: f64,
    pub wins: usize,
    pub losses: usize,
    pub p_value: f64,
    pub reason: String,
}

pub fn infer_task_type(message: &str) -> String {
    let text = message.to_ascii_lowercase();
    if text.contains("deploy")
        || text.contains("dashboard")
        || text.contains("web app")
        || text.contains("website")
    {
        return "app_deploy".to_string();
    }
    if text.contains("code")
        || text.contains("rust")
        || text.contains("python")
        || text.contains("bug")
        || text.contains("test")
    {
        return "coding".to_string();
    }
    if text.contains("research")
        || text.contains("analyze")
        || text.contains("compare")
        || text.contains("report")
    {
        return "research".to_string();
    }
    if text.contains("email")
        || text.contains("calendar")
        || text.contains("message")
        || text.contains("reply")
    {
        return "communication".to_string();
    }
    "general".to_string()
}

pub fn render_prompt_strategy_block(
    profile: &ToolStrategyProfile,
    task_type: &str,
) -> Option<String> {
    let mut lines: Vec<String> = profile
        .default_guidance
        .iter()
        .map(|s| s.trim())
        .filter(|s| !s.is_empty())
        .take(4)
        .map(|s| s.to_string())
        .collect();

    if let Some(task_lines) = profile.task_guidance.get(task_type) {
        for line in task_lines
            .iter()
            .map(|s| s.trim())
            .filter(|s| !s.is_empty())
            .take(4)
        {
            lines.push(line.to_string());
        }
    }
    if lines.is_empty() {
        return None;
    }
    lines.truncate(6);
    let mut out = String::new();
    out.push_str("## Active Tool Strategy\n");
    out.push_str(&format!(
        "- Strategy version: {}\n- Task type: {}\n",
        profile.version, task_type
    ));
    for line in lines {
        out.push_str("- ");
        out.push_str(&line);
        out.push('\n');
    }
    Some(out)
}

pub fn should_use_canary(seed: &str, rollout_percent: u8) -> bool {
    if rollout_percent == 0 {
        return false;
    }
    if rollout_percent >= 100 {
        return true;
    }
    let mut hasher = std::collections::hash_map::DefaultHasher::new();
    seed.hash(&mut hasher);
    let bucket = (hasher.finish() % 100) as u8;
    bucket < rollout_percent
}

pub fn evaluate_canary_by_policy_version(
    logs: &[crate::storage::entities::operational_log::Model],
    baseline_version: &str,
    candidate_version: &str,
    min_samples_per_version: usize,
    min_success_gain: f64,
    max_sign_test_p_value: f64,
) -> ReplayEvaluationResult {
    let mut base_samples = Vec::new();
    let mut cand_samples = Vec::new();
    for row in logs.iter().filter(|row| row.event_type == "tool_call") {
        match row.policy_version.as_deref() {
            Some(v) if v == baseline_version => base_samples.push(row),
            Some(v) if v == candidate_version => cand_samples.push(row),
            _ => {}
        }
    }
    evaluate_two_sets(
        &base_samples,
        &cand_samples,
        baseline_version,
        candidate_version,
        min_samples_per_version,
        min_success_gain,
        max_sign_test_p_value,
    )
}

fn evaluate_two_sets(
    baseline_rows: &[&crate::storage::entities::operational_log::Model],
    candidate_rows: &[&crate::storage::entities::operational_log::Model],
    baseline_version: &str,
    candidate_version: &str,
    min_samples_per_version: usize,
    min_success_gain: f64,
    max_sign_test_p_value: f64,
) -> ReplayEvaluationResult {
    let baseline = compute_metrics(baseline_rows);
    let candidate = compute_metrics(candidate_rows);

    let mut paired_deltas: HashMap<(String, String), (f64, f64)> = HashMap::new();
    for row in baseline_rows {
        let key = (
            row.channel.clone(),
            row.tool_name.clone().unwrap_or_else(|| "_none".to_string()),
        );
        let entry = paired_deltas.entry(key).or_insert((0.0, 0.0));
        entry.0 += if row.success { 1.0 } else { 0.0 };
    }
    for row in candidate_rows {
        let key = (
            row.channel.clone(),
            row.tool_name.clone().unwrap_or_else(|| "_none".to_string()),
        );
        let entry = paired_deltas.entry(key).or_insert((0.0, 0.0));
        entry.1 += if row.success { 1.0 } else { 0.0 };
    }

    let mut wins = 0usize;
    let mut losses = 0usize;
    for ((_channel, _tool), (baseline_score, candidate_score)) in paired_deltas {
        if baseline_score == 0.0 && candidate_score == 0.0 {
            continue;
        }
        if candidate_score > baseline_score {
            wins += 1;
        } else if candidate_score < baseline_score {
            losses += 1;
        }
    }

    let p_value = one_sided_sign_test_p_value(wins, losses);
    let success_gain = candidate.success_rate - baseline.success_rate;

    let eligible =
        baseline.samples >= min_samples_per_version && candidate.samples >= min_samples_per_version;
    let latency_guard_ok = match (baseline.p95_latency_ms, candidate.p95_latency_ms) {
        (Some(b), Some(c)) if b > 0 => (c as f64) <= (b as f64 * 1.15),
        _ => true,
    };
    let promote = eligible
        && success_gain >= min_success_gain
        && wins > losses
        && p_value <= max_sign_test_p_value
        && latency_guard_ok;

    let reason = if !eligible {
        format!(
            "insufficient samples (baseline={}, candidate={}, min={})",
            baseline.samples, candidate.samples, min_samples_per_version
        )
    } else if !latency_guard_ok {
        "candidate latency regression exceeds 15% p95 threshold".to_string()
    } else if success_gain < min_success_gain {
        format!(
            "success gain {:.4} below threshold {:.4}",
            success_gain, min_success_gain
        )
    } else if wins <= losses {
        format!("wins={} not greater than losses={}", wins, losses)
    } else if p_value > max_sign_test_p_value {
        format!(
            "p-value {:.4} above threshold {:.4}",
            p_value, max_sign_test_p_value
        )
    } else {
        "passed".to_string()
    };

    ReplayEvaluationResult {
        eligible,
        promote,
        baseline_version: baseline_version.to_string(),
        candidate_version: candidate_version.to_string(),
        baseline,
        candidate,
        success_gain,
        wins,
        losses,
        p_value,
        reason,
    }
}

fn compute_metrics(
    rows: &[&crate::storage::entities::operational_log::Model],
) -> ReplayVersionMetrics {
    let samples = rows.len();
    let successes = rows.iter().filter(|row| row.success).count();
    let success_rate = if samples == 0 {
        0.0
    } else {
        successes as f64 / samples as f64
    };
    let mut latencies: Vec<i64> = rows.iter().filter_map(|row| row.latency_ms).collect();
    latencies.sort_unstable();
    let p95_latency_ms = if latencies.is_empty() {
        None
    } else {
        let idx = (((latencies.len() as f64) * 0.95).ceil() as usize)
            .saturating_sub(1)
            .min(latencies.len().saturating_sub(1));
        Some(latencies[idx])
    };
    ReplayVersionMetrics {
        samples,
        successes,
        success_rate: round4(success_rate),
        p95_latency_ms,
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
