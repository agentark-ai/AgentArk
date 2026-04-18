//! Runtime strategy and canary utilities for self-evolution.
//!
//! Provides:
//! - prompt strategy profile structures
//! - deterministic canary selection
//! - offline replay/canary evaluation helpers from operational logs

use std::collections::HashMap;
use std::hash::{Hash, Hasher};

use serde::{Deserialize, Serialize};

use crate::actions::ActionDef;
use crate::core::RequestShapeAssessment;

pub const TOOL_STRATEGY_PROFILE_KEY: &str = "tool_strategy_profile_v1";
pub const TOOL_STRATEGY_PROFILE_CANARY_KEY: &str = "tool_strategy_profile_canary_v1";
pub const TOOL_STRATEGY_CANARY_STATE_KEY: &str = "tool_strategy_canary_state_v1";
pub const TOOL_STRATEGY_PROFILE_BASELINE_SNAPSHOT_KEY: &str =
    "tool_strategy_profile_baseline_snapshot_v1";
pub const ROUTING_COMPLEXITY_POLICY_CANARY_KEY: &str = "routing_complexity_policy_canary_v1";
pub const ROUTING_COMPLEXITY_CANARY_STATE_KEY: &str = "routing_complexity_policy_canary_state_v1";
pub const ROUTING_COMPLEXITY_POLICY_BASELINE_SNAPSHOT_KEY: &str =
    "routing_complexity_policy_baseline_snapshot_v1";
pub const PROMPT_PROFILE_CANARY_SAFETY_EVENTS_KEY: &str = "prompt_profile_canary_safety_events_v1";
pub const SELF_EVOLVE_LAST_RESULT_KEY: &str = "self_evolve_last_result_v1";
pub const APP_DEPLOY_ACCESS_GUARD_DEFAULT_KEY: &str = "app_deploy_access_guard_default_v1";
pub const SELF_EVOLVE_ENABLED_KEY: &str = "self_evolve_enabled_v1";

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

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct PromptProfileCanarySafetyEvent {
    pub id: String,
    pub trace_kind: String,
    pub surface: String,
    pub surface_label: String,
    pub status: String,
    pub review_status: String,
    #[serde(default)]
    pub reviewed_at: Option<String>,
    pub title: String,
    pub summary: String,
    pub baseline_version: String,
    pub candidate_version: String,
    pub baseline_samples: usize,
    pub candidate_samples: usize,
    pub baseline_success_rate: f64,
    pub candidate_success_rate: f64,
    pub success_delta: f64,
    pub wins: usize,
    pub losses: usize,
    pub regression_p_value: f64,
    pub min_success_gain: f64,
    pub max_sign_test_p_value: f64,
    pub created_at: String,
}

fn task_type_for_action_name(action_name: &str) -> Option<&'static str> {
    match action_name.trim().to_ascii_lowercase().as_str() {
        "app_deploy" | "app_restart" | "app_stop" | "app_delete" | "app_inspect" => {
            Some("app_deploy")
        }
        "file_read" | "file_write" | "shell" | "code_execute" | "local_cli"
        | "connector_request" => Some("coding"),
        "research" | "web_search" | "page_fetch" | "rank_signals" => Some("research"),
        "gmail_scan" | "gmail_reply" | "calendar_today" | "calendar_list" | "calendar_free"
        | "calendar_create" | "notify_user" => Some("communication"),
        _ => None,
    }
}

pub fn infer_task_type_from_action_names<I, S>(action_names: I) -> String
where
    I: IntoIterator<Item = S>,
    S: AsRef<str>,
{
    let mut saw_app = false;
    let mut saw_code = false;
    let mut saw_research = false;
    let mut saw_communication = false;

    for action_name in action_names {
        match task_type_for_action_name(action_name.as_ref()) {
            Some("app_deploy") => saw_app = true,
            Some("coding") => saw_code = true,
            Some("research") => saw_research = true,
            Some("communication") => saw_communication = true,
            _ => {}
        }
    }

    if saw_app {
        "app_deploy".to_string()
    } else if saw_code {
        "coding".to_string()
    } else if saw_research {
        "research".to_string()
    } else if saw_communication {
        "communication".to_string()
    } else {
        "general".to_string()
    }
}

pub fn infer_task_type_from_request_context(
    request_shape: Option<&RequestShapeAssessment>,
    actions: &[ActionDef],
) -> String {
    let from_actions =
        infer_task_type_from_action_names(actions.iter().map(|action| action.name.as_str()));
    if from_actions != "general" {
        return from_actions;
    }

    if let Some(shape) = request_shape {
        if shape.shape_is("app") {
            return "app_deploy".to_string();
        }
        if shape.is_integration_request() {
            return "communication".to_string();
        }
        if shape.shape_is("watcher") || shape.shape_is("goal") || shape.is_execution_request() {
            return "general".to_string();
        }
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

#[cfg(test)]
mod task_type_tests {
    use super::*;

    #[test]
    fn infers_task_type_from_action_names_without_message_keywords() {
        let task_type =
            infer_task_type_from_action_names(["app_restart", "file_write", "notify_user"]);
        assert_eq!(task_type, "app_deploy");

        let task_type = infer_task_type_from_action_names(["research", "page_fetch"]);
        assert_eq!(task_type, "research");
    }

    #[test]
    fn infers_task_type_from_request_shape_when_actions_are_empty() {
        let shape = RequestShapeAssessment {
            shape: "integration".to_string(),
            execution_mode: "immediate".to_string(),
            confidence: 0.91,
            should_confirm: false,
            confirmation_question: None,
            reasoning: String::new(),
            preferred_actions: vec![],
            integration_id: None,
            ..Default::default()
        };

        let task_type = infer_task_type_from_request_context(Some(&shape), &[]);
        assert_eq!(task_type, "communication");
    }
}

pub fn evaluate_experience_canary_by_prompt_version(
    runs: &[crate::storage::entities::experience_run::Model],
    baseline_version: &str,
    candidate_version: &str,
    min_samples_per_version: usize,
    min_success_gain: f64,
    max_sign_test_p_value: f64,
) -> ReplayEvaluationResult {
    let baseline_rows = runs
        .iter()
        .filter(|run| {
            run.prompt_version.as_deref() == Some(baseline_version) && experience_run_resolved(run)
        })
        .collect::<Vec<_>>();
    let candidate_rows = runs
        .iter()
        .filter(|run| {
            run.prompt_version.as_deref() == Some(candidate_version) && experience_run_resolved(run)
        })
        .collect::<Vec<_>>();

    let baseline = compute_experience_metrics(&baseline_rows);
    let candidate = compute_experience_metrics(&candidate_rows);

    let mut paired_deltas: HashMap<(String, String), (f64, f64)> = HashMap::new();
    for row in &baseline_rows {
        let key = (row.channel.clone(), row.intent_key.clone());
        let entry = paired_deltas.entry(key).or_insert((0.0, 0.0));
        entry.0 += if experience_run_success(row) {
            1.0
        } else {
            0.0
        };
    }
    for row in &candidate_rows {
        let key = (row.channel.clone(), row.intent_key.clone());
        let entry = paired_deltas.entry(key).or_insert((0.0, 0.0));
        entry.1 += if experience_run_success(row) {
            1.0
        } else {
            0.0
        };
    }

    let mut wins = 0usize;
    let mut losses = 0usize;
    for ((_channel, _intent), (baseline_score, candidate_score)) in paired_deltas {
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
    let promote = eligible
        && success_gain >= min_success_gain
        && wins > losses
        && p_value <= max_sign_test_p_value;

    let reason = if !eligible {
        format!(
            "insufficient experience samples (baseline={}, candidate={}, min={})",
            baseline.samples, candidate.samples, min_samples_per_version
        )
    } else if success_gain < min_success_gain {
        format!(
            "experience success gain {:.4} below threshold {:.4}",
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

pub fn experience_run_metadata_version<'a>(
    row: &'a crate::storage::entities::experience_run::Model,
    key: &str,
) -> Option<&'a str> {
    row.metadata
        .get(key)
        .and_then(|value| value.as_str())
        .map(str::trim)
        .filter(|value| !value.is_empty())
}

pub fn evaluate_experience_canary_by_metadata_version(
    runs: &[crate::storage::entities::experience_run::Model],
    metadata_key: &str,
    baseline_version: &str,
    candidate_version: &str,
    min_samples_per_version: usize,
    min_success_gain: f64,
    max_sign_test_p_value: f64,
) -> ReplayEvaluationResult {
    let baseline_rows = runs
        .iter()
        .filter(|run| {
            experience_run_metadata_version(run, metadata_key) == Some(baseline_version)
                && experience_run_resolved(run)
        })
        .collect::<Vec<_>>();
    let candidate_rows = runs
        .iter()
        .filter(|run| {
            experience_run_metadata_version(run, metadata_key) == Some(candidate_version)
                && experience_run_resolved(run)
        })
        .collect::<Vec<_>>();

    let baseline = compute_experience_metrics(&baseline_rows);
    let candidate = compute_experience_metrics(&candidate_rows);

    let mut paired_deltas: HashMap<(String, String), (f64, f64)> = HashMap::new();
    for row in &baseline_rows {
        let key = (row.channel.clone(), row.intent_key.clone());
        let entry = paired_deltas.entry(key).or_insert((0.0, 0.0));
        entry.0 += if experience_run_success(row) {
            1.0
        } else {
            0.0
        };
    }
    for row in &candidate_rows {
        let key = (row.channel.clone(), row.intent_key.clone());
        let entry = paired_deltas.entry(key).or_insert((0.0, 0.0));
        entry.1 += if experience_run_success(row) {
            1.0
        } else {
            0.0
        };
    }

    let mut wins = 0usize;
    let mut losses = 0usize;
    for ((_channel, _intent), (baseline_score, candidate_score)) in paired_deltas {
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
    let promote = eligible
        && success_gain >= min_success_gain
        && wins > losses
        && p_value <= max_sign_test_p_value;

    let reason = if !eligible {
        format!(
            "insufficient experience samples (baseline={}, candidate={}, min={})",
            baseline.samples, candidate.samples, min_samples_per_version
        )
    } else if success_gain < min_success_gain {
        format!(
            "experience success gain {:.4} below threshold {:.4}",
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

fn compute_experience_metrics(
    rows: &[&crate::storage::entities::experience_run::Model],
) -> ReplayVersionMetrics {
    let samples = rows.len();
    let successes = rows
        .iter()
        .filter(|row| experience_run_success(row))
        .count();
    let success_rate = if samples == 0 {
        0.0
    } else {
        successes as f64 / samples as f64
    };
    ReplayVersionMetrics {
        samples,
        successes,
        success_rate: round4(success_rate),
        p95_latency_ms: None,
    }
}

fn experience_run_resolved(row: &crate::storage::entities::experience_run::Model) -> bool {
    row.correction_state == "corrected"
        || row.success_state == "accepted"
        || row.success_state == "failed"
}

fn experience_run_success(row: &crate::storage::entities::experience_run::Model) -> bool {
    row.correction_state != "corrected" && row.success_state == "accepted"
}

pub fn one_sided_sign_test_p_value(wins: usize, losses: usize) -> f64 {
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

#[cfg(test)]
mod tests {
    use super::*;

    fn test_experience_run(
        id: &str,
        prompt_version: &str,
        intent_key: &str,
        success_state: &str,
        correction_state: &str,
    ) -> crate::storage::entities::experience_run::Model {
        crate::storage::entities::experience_run::Model {
            id: id.to_string(),
            execution_run_id: None,
            trace_id: Some(format!("trace-{id}")),
            conversation_id: None,
            project_id: None,
            channel: "chat".to_string(),
            scope: "global".to_string(),
            intent_key: intent_key.to_string(),
            task_type: Some("task".to_string()),
            request_text: None,
            tool_sequence_digest: None,
            tool_sequence_json: serde_json::json!([]),
            strategy_version: None,
            policy_version: None,
            prompt_version: Some(prompt_version.to_string()),
            model_slot: None,
            success_state: success_state.to_string(),
            correction_state: correction_state.to_string(),
            outcome_summary: None,
            failure_reason: None,
            metadata: serde_json::json!({}),
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
        }
    }

    #[test]
    fn prompt_experience_gate_ignores_provisional_runs() {
        let runs = vec![
            test_experience_run(
                "baseline-accepted",
                "prompt+baseline",
                "fix_bug",
                "accepted",
                "none",
            ),
            test_experience_run(
                "candidate-provisional",
                "prompt+candidate",
                "fix_bug",
                "provisional",
                "none",
            ),
            test_experience_run(
                "candidate-accepted",
                "prompt+candidate",
                "fix_bug",
                "accepted",
                "none",
            ),
        ];

        let evaluation = evaluate_experience_canary_by_prompt_version(
            &runs,
            "prompt+baseline",
            "prompt+candidate",
            1,
            0.01,
            0.10,
        );

        assert_eq!(evaluation.baseline.samples, 1);
        assert_eq!(evaluation.candidate.samples, 1);
        assert_eq!(evaluation.candidate.successes, 1);
    }
}
