//! Policy-first self-evolution engine.
//!
//! This module evolves runtime policy (starting with routing complexity policy)
//! using a benchmarked promotion loop, lineage archive, and statistical gating.

use anyhow::{Context, Result};
use serde::{Deserialize, Serialize};
use std::collections::{HashMap, HashSet};
use std::hash::{Hash, Hasher};
use std::path::PathBuf;

use crate::core::llm::LlmClient;

pub const ROUTING_COMPLEXITY_POLICY_KEY: &str = "routing_complexity_policy_v1";
const LINEAGE_ARCHIVE_REL_PATH: &str = ".agentark/self_evolve/routing_policy_lineage.jsonl";
const BENCHMARK_PROFILE_REL_PATH: &str = "assets/self_evolve/routing_benchmark_v1.json";
const DEFAULT_RECENT_LINEAGE_LIMIT: usize = 12;
const MAX_LINEAGE_ARCHIVE_ENTRIES: usize = 400;
const MAX_INDICATORS: usize = 96;
const MAX_INDICATOR_LEN: usize = 80;

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct RoutingComplexityPolicy {
    pub complex_indicators: Vec<String>,
    pub medium_indicators: Vec<String>,
    pub long_question_word_threshold: usize,
    pub long_message_word_threshold: usize,
    pub multi_sentence_threshold: usize,
}

impl Default for RoutingComplexityPolicy {
    fn default() -> Self {
        Self {
            complex_indicators: vec![
                "research".to_string(),
                "investigate".to_string(),
                "analyze and".to_string(),
                "compare and".to_string(),
                "write a report".to_string(),
                "write an article".to_string(),
                "comprehensive".to_string(),
                "step by step".to_string(),
                "multiple".to_string(),
                "all of".to_string(),
                "each of".to_string(),
            ],
            medium_indicators: vec![
                "explain".to_string(),
                "why".to_string(),
                "how does".to_string(),
                "what is the difference".to_string(),
                "should i".to_string(),
                "which is better".to_string(),
                "pros and cons".to_string(),
                "analyze".to_string(),
                "evaluate".to_string(),
                "recommend".to_string(),
                "suggest".to_string(),
                "help me understand".to_string(),
                "clarify".to_string(),
                "create a".to_string(),
                "build a".to_string(),
                "develop".to_string(),
                "implement".to_string(),
                "design".to_string(),
                "make a".to_string(),
                "deploy".to_string(),
                "generate".to_string(),
                "send".to_string(),
                "check".to_string(),
                "fix".to_string(),
            ],
            long_question_word_threshold: 50,
            long_message_word_threshold: 30,
            multi_sentence_threshold: 3,
        }
    }
}

#[derive(Debug, Clone)]
pub struct PolicyEvolutionConfig {
    pub project_root: PathBuf,
    pub max_candidates: usize,
    pub min_accuracy_gain: f64,
    pub min_benchmark_accuracy: f64,
    pub max_sign_test_p_value: f64,
}

impl Default for PolicyEvolutionConfig {
    fn default() -> Self {
        Self {
            project_root: PathBuf::from("."),
            max_candidates: 6,
            min_accuracy_gain: 0.03,
            min_benchmark_accuracy: 0.70,
            max_sign_test_p_value: 0.10,
        }
    }
}

#[derive(Debug, Clone, Serialize)]
pub struct PolicyEvolutionResult {
    pub success: bool,
    pub mode: String,
    pub target_key: String,
    pub promoted: bool,
    pub evaluated_candidates: usize,
    pub baseline_accuracy: f64,
    pub best_candidate_accuracy: f64,
    pub accuracy_gain: f64,
    pub wins: usize,
    pub losses: usize,
    pub p_value: f64,
    pub candidate_source: Option<String>,
    pub promotion_gate: String,
    pub promoted_policy: Option<serde_json::Value>,
    pub lineage_entry_id: String,
    pub lineage_archive_path: String,
    pub notes: Vec<String>,
    pub error: Option<String>,
}

pub struct PolicyEvolutionEngine {
    config: PolicyEvolutionConfig,
    llm: LlmClient,
}

impl PolicyEvolutionEngine {
    pub fn new(config: PolicyEvolutionConfig, llm: LlmClient) -> Self {
        Self { config, llm }
    }

    pub async fn evolve_routing_policy(
        &self,
        user_request: &str,
        current_policy_raw: Option<&[u8]>,
    ) -> Result<PolicyEvolutionResult> {
        let baseline_policy = self.load_baseline_policy(current_policy_raw)?;
        let benchmark = self.load_benchmark_suite().await?;
        let baseline_eval = evaluate_policy(&baseline_policy, &benchmark);

        let recent_lineage = self.load_recent_lineage(DEFAULT_RECENT_LINEAGE_LIMIT).await;
        let mut candidates =
            self.heuristic_candidates(user_request, &baseline_policy, &baseline_eval, &benchmark);

        if let Some(llm_candidate) = self
            .generate_llm_candidate(
                user_request,
                &baseline_policy,
                &baseline_eval,
                &recent_lineage,
            )
            .await
        {
            candidates.push(llm_candidate);
        }

        let max_candidates = self.config.max_candidates.max(1);
        if candidates.len() > max_candidates {
            candidates.truncate(max_candidates);
        }

        let mut seen_hashes: HashSet<String> = HashSet::new();
        candidates.retain(|candidate| {
            let hash = policy_hash(&candidate.policy);
            if hash == policy_hash(&baseline_policy) {
                return false;
            }
            seen_hashes.insert(hash)
        });

        let evaluated_candidates = candidates.len();
        if evaluated_candidates == 0 {
            let entry_id = self
                .append_lineage_entry(&LineageEntry {
                    entry_id: format!("pol-{}", uuid::Uuid::new_v4()),
                    timestamp_utc: chrono::Utc::now().to_rfc3339(),
                    target_key: ROUTING_COMPLEXITY_POLICY_KEY.to_string(),
                    request: user_request.to_string(),
                    baseline_policy_hash: policy_hash(&baseline_policy),
                    candidate_policy_hash: policy_hash(&baseline_policy),
                    baseline_accuracy: baseline_eval.accuracy,
                    candidate_accuracy: baseline_eval.accuracy,
                    accuracy_gain: 0.0,
                    wins: 0,
                    losses: 0,
                    p_value: 1.0,
                    promoted: false,
                    candidate_source: "none".to_string(),
                    notes: vec!["No distinct candidates were generated".to_string()],
                })
                .await
                .unwrap_or_else(|_| "lineage-write-failed".to_string());

            return Ok(PolicyEvolutionResult {
                success: true,
                mode: "policy".to_string(),
                target_key: ROUTING_COMPLEXITY_POLICY_KEY.to_string(),
                promoted: false,
                evaluated_candidates: 0,
                baseline_accuracy: round4(baseline_eval.accuracy),
                best_candidate_accuracy: round4(baseline_eval.accuracy),
                accuracy_gain: 0.0,
                wins: 0,
                losses: 0,
                p_value: 1.0,
                candidate_source: None,
                promotion_gate: "rejected: no valid policy mutations".to_string(),
                promoted_policy: None,
                lineage_entry_id: entry_id,
                lineage_archive_path: self.archive_path().display().to_string(),
                notes: vec!["No-op evolution cycle; baseline retained.".to_string()],
                error: None,
            });
        }

        let mut best: Option<(CandidatePolicy, PolicyEvaluation, PairedStats)> = None;
        for candidate in candidates {
            let eval = evaluate_policy(&candidate.policy, &benchmark);
            let paired = paired_stats(&baseline_eval, &eval);
            let replace = match &best {
                None => true,
                Some((_, best_eval, best_paired)) => {
                    eval.accuracy > best_eval.accuracy
                        || (f64_eq(eval.accuracy, best_eval.accuracy)
                            && paired.wins > best_paired.wins)
                        || (f64_eq(eval.accuracy, best_eval.accuracy)
                            && paired.wins == best_paired.wins
                            && paired.p_value < best_paired.p_value)
                }
            };
            if replace {
                best = Some((candidate, eval, paired));
            }
        }

        let (best_candidate, best_eval, best_stats) = best.context("no best candidate found")?;
        let promotion_checks = promotion_checks(
            &self.config,
            baseline_eval.accuracy,
            best_eval.accuracy,
            &best_stats,
        );
        let promoted = promotion_checks.values().all(|passed| *passed);
        let promotion_gate = render_promotion_gate(&promotion_checks);
        let notes = build_notes(
            &baseline_eval,
            &best_eval,
            &best_stats,
            &best_candidate.source,
        );

        let lineage_entry = LineageEntry {
            entry_id: format!("pol-{}", uuid::Uuid::new_v4()),
            timestamp_utc: chrono::Utc::now().to_rfc3339(),
            target_key: ROUTING_COMPLEXITY_POLICY_KEY.to_string(),
            request: user_request.to_string(),
            baseline_policy_hash: policy_hash(&baseline_policy),
            candidate_policy_hash: policy_hash(&best_candidate.policy),
            baseline_accuracy: round4(baseline_eval.accuracy),
            candidate_accuracy: round4(best_eval.accuracy),
            accuracy_gain: round4(best_stats.accuracy_gain),
            wins: best_stats.wins,
            losses: best_stats.losses,
            p_value: round4(best_stats.p_value),
            promoted,
            candidate_source: best_candidate.source.clone(),
            notes: notes.clone(),
        };
        let lineage_entry_id = self
            .append_lineage_entry(&lineage_entry)
            .await
            .unwrap_or_else(|_| "lineage-write-failed".to_string());

        Ok(PolicyEvolutionResult {
            success: true,
            mode: "policy".to_string(),
            target_key: ROUTING_COMPLEXITY_POLICY_KEY.to_string(),
            promoted,
            evaluated_candidates,
            baseline_accuracy: round4(baseline_eval.accuracy),
            best_candidate_accuracy: round4(best_eval.accuracy),
            accuracy_gain: round4(best_stats.accuracy_gain),
            wins: best_stats.wins,
            losses: best_stats.losses,
            p_value: round4(best_stats.p_value),
            candidate_source: Some(best_candidate.source),
            promotion_gate,
            promoted_policy: if promoted {
                Some(serde_json::to_value(best_candidate.policy)?)
            } else {
                None
            },
            lineage_entry_id,
            lineage_archive_path: self.archive_path().display().to_string(),
            notes,
            error: None,
        })
    }

    fn load_baseline_policy(
        &self,
        current_policy_raw: Option<&[u8]>,
    ) -> Result<RoutingComplexityPolicy> {
        let mut policy = RoutingComplexityPolicy::default();
        if let Some(raw) = current_policy_raw {
            let value: serde_json::Value = serde_json::from_slice(raw)
                .context("failed to parse stored routing complexity policy JSON")?;
            apply_override(&mut policy, &value);
        }
        sanitize_policy(&mut policy);
        Ok(policy)
    }

    async fn load_recent_lineage(&self, limit: usize) -> Vec<LineageEntry> {
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
            if let Ok(entry) = serde_json::from_str::<LineageEntry>(line) {
                parsed.push(entry);
            }
        }
        if parsed.len() <= limit {
            return parsed;
        }
        parsed.split_off(parsed.len() - limit)
    }

    async fn append_lineage_entry(&self, entry: &LineageEntry) -> Result<String> {
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

    async fn load_benchmark_suite(&self) -> Result<Vec<BenchmarkCase>> {
        let profile_path = self.config.project_root.join(BENCHMARK_PROFILE_REL_PATH);
        let raw = tokio::fs::read_to_string(&profile_path)
            .await
            .with_context(|| {
                format!(
                    "failed to read benchmark profile {}",
                    profile_path.display()
                )
            })?;
        let profile: BenchmarkProfile = serde_json::from_str(&raw).with_context(|| {
            format!(
                "failed to parse benchmark profile JSON {}",
                profile_path.display()
            )
        })?;
        if profile.target_key != ROUTING_COMPLEXITY_POLICY_KEY {
            tracing::warn!(
                "policy evolution benchmark target_key mismatch: got '{}', expected '{}'",
                profile.target_key,
                ROUTING_COMPLEXITY_POLICY_KEY
            );
        }
        if profile.cases.is_empty() {
            anyhow::bail!("benchmark profile has zero cases");
        }
        let mut cases = Vec::with_capacity(profile.cases.len());
        for case in profile.cases {
            let prompt = case.prompt.trim().to_string();
            if prompt.is_empty() {
                continue;
            }
            cases.push(BenchmarkCase {
                prompt,
                expected: case.expected,
            });
        }
        if cases.is_empty() {
            anyhow::bail!("benchmark profile has no valid prompt cases");
        }
        Ok(cases)
    }

    fn heuristic_candidates(
        &self,
        user_request: &str,
        baseline: &RoutingComplexityPolicy,
        baseline_eval: &PolicyEvaluation,
        benchmark: &[BenchmarkCase],
    ) -> Vec<CandidatePolicy> {
        let mut out = Vec::new();

        let misses = baseline_eval
            .mismatches
            .iter()
            .map(|m| (&benchmark[m.case_idx], m.expected, m.predicted))
            .collect::<Vec<_>>();

        if !misses.is_empty() {
            let mut threshold_candidate = baseline.clone();
            let mut complex_misses = 0usize;
            let mut simple_overcalls = 0usize;
            for (_, expected, predicted) in &misses {
                if *expected == ComplexityLabel::Complex && *predicted != ComplexityLabel::Complex {
                    complex_misses += 1;
                }
                if *expected == ComplexityLabel::Simple && *predicted != ComplexityLabel::Simple {
                    simple_overcalls += 1;
                }
            }
            if complex_misses >= simple_overcalls {
                threshold_candidate.long_question_word_threshold = threshold_candidate
                    .long_question_word_threshold
                    .saturating_sub(6)
                    .max(5);
                threshold_candidate.long_message_word_threshold = threshold_candidate
                    .long_message_word_threshold
                    .saturating_sub(4)
                    .max(5);
                threshold_candidate.multi_sentence_threshold = threshold_candidate
                    .multi_sentence_threshold
                    .saturating_sub(1)
                    .max(1);
            } else {
                threshold_candidate.long_question_word_threshold =
                    (threshold_candidate.long_question_word_threshold + 6).min(1000);
                threshold_candidate.long_message_word_threshold =
                    (threshold_candidate.long_message_word_threshold + 4).min(1000);
                threshold_candidate.multi_sentence_threshold =
                    (threshold_candidate.multi_sentence_threshold + 1).min(50);
            }
            sanitize_policy(&mut threshold_candidate);
            out.push(CandidatePolicy {
                source: "heuristic-threshold-adjustment".to_string(),
                policy: threshold_candidate,
            });
        }

        let mut keyword_candidate = baseline.clone();
        let mut complex_tokens: Vec<String> = Vec::new();
        let mut medium_tokens: Vec<String> = Vec::new();
        for (case, expected, predicted) in misses {
            if expected == ComplexityLabel::Complex && predicted != ComplexityLabel::Complex {
                complex_tokens.extend(extract_keywords(&case.prompt, 5, 4));
            } else if expected == ComplexityLabel::Medium && predicted == ComplexityLabel::Simple {
                medium_tokens.extend(extract_keywords(&case.prompt, 5, 4));
            }
        }
        append_unique_limited(
            &mut keyword_candidate.complex_indicators,
            &complex_tokens,
            MAX_INDICATORS,
        );
        append_unique_limited(
            &mut keyword_candidate.medium_indicators,
            &medium_tokens,
            MAX_INDICATORS,
        );
        sanitize_policy(&mut keyword_candidate);
        out.push(CandidatePolicy {
            source: "heuristic-failure-keywords".to_string(),
            policy: keyword_candidate,
        });

        let mut request_candidate = baseline.clone();
        let request_tokens = extract_keywords(user_request, 5, 8);
        append_unique_limited(
            &mut request_candidate.medium_indicators,
            &request_tokens,
            MAX_INDICATORS,
        );
        sanitize_policy(&mut request_candidate);
        out.push(CandidatePolicy {
            source: "heuristic-request-keywords".to_string(),
            policy: request_candidate,
        });

        out
    }

    async fn generate_llm_candidate(
        &self,
        user_request: &str,
        baseline: &RoutingComplexityPolicy,
        baseline_eval: &PolicyEvaluation,
        recent_lineage: &[LineageEntry],
    ) -> Option<CandidatePolicy> {
        let lineage_summary = if recent_lineage.is_empty() {
            "No prior lineage entries.".to_string()
        } else {
            recent_lineage
                .iter()
                .rev()
                .take(6)
                .map(|entry| {
                    format!(
                        "- {} promoted={} gain={:.4} p={:.4} source={}",
                        entry.timestamp_utc,
                        entry.promoted,
                        entry.accuracy_gain,
                        entry.p_value,
                        entry.candidate_source
                    )
                })
                .collect::<Vec<_>>()
                .join("\n")
        };

        let mismatch_preview = baseline_eval
            .mismatches
            .iter()
            .take(6)
            .map(|m| {
                format!(
                    "idx={} expected={:?} predicted={:?}",
                    m.case_idx, m.expected, m.predicted
                )
            })
            .collect::<Vec<_>>()
            .join("; ");
        let user_prompt = format!(
            "Target key: {}\nUser request: {}\nBaseline policy JSON:\n{}\n\nBaseline accuracy: {:.4}\nBaseline mismatches: {}\n\nRecent lineage:\n{}\n\nReturn ONLY one JSON object with optional keys: complex_indicators (array<string>), medium_indicators (array<string>), long_question_word_threshold (int), long_message_word_threshold (int), multi_sentence_threshold (int).\nConstraints: no empty strings, lowercase indicators, keep thresholds in safe ranges.",
            ROUTING_COMPLEXITY_POLICY_KEY,
            user_request,
            serde_json::to_string_pretty(baseline).ok()?,
            baseline_eval.accuracy,
            if mismatch_preview.is_empty() {
                "none".to_string()
            } else {
                mismatch_preview
            },
            lineage_summary
        );

        let system =
            "You mutate routing policy for better benchmark accuracy. Output strict JSON only.";
        let response = match self.llm.chat_with_system(system, &user_prompt).await {
            Ok(r) => r,
            Err(e) => {
                tracing::warn!("policy evolution: llm candidate generation failed: {}", e);
                return None;
            }
        };
        let parsed = parse_json_object(&response.content)?;
        let mut candidate = baseline.clone();
        apply_override(&mut candidate, &parsed);
        sanitize_policy(&mut candidate);
        Some(CandidatePolicy {
            source: "llm-mutation".to_string(),
            policy: candidate,
        })
    }
}

#[derive(Debug, Clone, Serialize, Deserialize)]
struct LineageEntry {
    entry_id: String,
    timestamp_utc: String,
    target_key: String,
    request: String,
    baseline_policy_hash: String,
    candidate_policy_hash: String,
    baseline_accuracy: f64,
    candidate_accuracy: f64,
    accuracy_gain: f64,
    wins: usize,
    losses: usize,
    p_value: f64,
    promoted: bool,
    candidate_source: String,
    notes: Vec<String>,
}

#[derive(Debug, Clone)]
struct CandidatePolicy {
    source: String,
    policy: RoutingComplexityPolicy,
}

#[derive(Debug, Clone, Copy, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "lowercase")]
enum ComplexityLabel {
    Simple,
    Medium,
    Complex,
}

#[derive(Debug, Clone)]
struct BenchmarkCase {
    prompt: String,
    expected: ComplexityLabel,
}

#[derive(Debug, Clone, Deserialize)]
struct BenchmarkProfile {
    target_key: String,
    #[serde(rename = "version")]
    _version: u32,
    cases: Vec<BenchmarkProfileCase>,
}

#[derive(Debug, Clone, Deserialize)]
struct BenchmarkProfileCase {
    prompt: String,
    expected: ComplexityLabel,
}

#[derive(Debug, Clone)]
struct PolicyMismatch {
    case_idx: usize,
    expected: ComplexityLabel,
    predicted: ComplexityLabel,
}

#[derive(Debug, Clone)]
struct PolicyEvaluation {
    accuracy: f64,
    correct: usize,
    predictions: Vec<ComplexityLabel>,
    mismatches: Vec<PolicyMismatch>,
}

#[derive(Debug, Clone)]
struct PairedStats {
    wins: usize,
    losses: usize,
    p_value: f64,
    accuracy_gain: f64,
}

fn evaluate_policy(policy: &RoutingComplexityPolicy, cases: &[BenchmarkCase]) -> PolicyEvaluation {
    let mut correct = 0usize;
    let mut predictions = Vec::with_capacity(cases.len());
    let mut mismatches = Vec::new();
    for (idx, case) in cases.iter().enumerate() {
        let predicted = classify_complexity(policy, &case.prompt);
        if predicted == case.expected {
            correct += 1;
        } else {
            mismatches.push(PolicyMismatch {
                case_idx: idx,
                expected: case.expected,
                predicted,
            });
        }
        predictions.push(predicted);
    }
    let accuracy = if cases.is_empty() {
        0.0
    } else {
        correct as f64 / cases.len() as f64
    };
    PolicyEvaluation {
        accuracy,
        correct,
        predictions,
        mismatches,
    }
}

fn classify_complexity(policy: &RoutingComplexityPolicy, message: &str) -> ComplexityLabel {
    let msg_lower = message.to_ascii_lowercase();
    let word_count = message.split_whitespace().count();

    for indicator in &policy.complex_indicators {
        if !indicator.is_empty() && msg_lower.contains(indicator) {
            return ComplexityLabel::Complex;
        }
    }
    if word_count > policy.long_question_word_threshold && msg_lower.contains('?') {
        return ComplexityLabel::Complex;
    }
    for indicator in &policy.medium_indicators {
        if !indicator.is_empty() && msg_lower.contains(indicator) {
            return ComplexityLabel::Medium;
        }
    }
    let sentence_count = message.matches('.').count() + message.matches('?').count();
    if sentence_count >= policy.multi_sentence_threshold
        || word_count > policy.long_message_word_threshold
    {
        return ComplexityLabel::Medium;
    }
    ComplexityLabel::Simple
}

fn paired_stats(baseline: &PolicyEvaluation, candidate: &PolicyEvaluation) -> PairedStats {
    let total = baseline.predictions.len().min(candidate.predictions.len());
    let baseline_errors = baseline
        .mismatches
        .iter()
        .map(|m| m.case_idx)
        .collect::<HashSet<_>>();
    let candidate_errors = candidate
        .mismatches
        .iter()
        .map(|m| m.case_idx)
        .collect::<HashSet<_>>();

    let mut wins = 0usize;
    let mut losses = 0usize;
    for idx in 0..total {
        let baseline_ok = !baseline_errors.contains(&idx);
        let candidate_ok = !candidate_errors.contains(&idx);
        match (baseline_ok, candidate_ok) {
            (false, true) => wins += 1,
            (true, false) => losses += 1,
            _ => {}
        }
    }

    let p_value = one_sided_sign_test_p_value(wins, losses);
    PairedStats {
        wins,
        losses,
        p_value,
        accuracy_gain: candidate.accuracy - baseline.accuracy,
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

fn promotion_checks(
    config: &PolicyEvolutionConfig,
    baseline_accuracy: f64,
    candidate_accuracy: f64,
    stats: &PairedStats,
) -> HashMap<&'static str, bool> {
    let mut checks = HashMap::new();
    checks.insert(
        "accuracy_not_worse",
        candidate_accuracy >= baseline_accuracy,
    );
    checks.insert(
        "min_accuracy_gain",
        stats.accuracy_gain >= config.min_accuracy_gain,
    );
    checks.insert("wins_gt_losses", stats.wins > stats.losses);
    checks.insert("sign_test", stats.p_value <= config.max_sign_test_p_value);
    checks.insert(
        "min_absolute_accuracy",
        candidate_accuracy >= config.min_benchmark_accuracy,
    );
    checks
}

fn render_promotion_gate(checks: &HashMap<&str, bool>) -> String {
    let ordered = [
        "accuracy_not_worse",
        "min_accuracy_gain",
        "wins_gt_losses",
        "sign_test",
        "min_absolute_accuracy",
    ];
    let failed: Vec<&str> = ordered
        .iter()
        .copied()
        .filter(|key| !checks.get(key).copied().unwrap_or(false))
        .collect();
    if failed.is_empty() {
        "passed".to_string()
    } else {
        format!("rejected: {}", failed.join(", "))
    }
}

fn apply_override(policy: &mut RoutingComplexityPolicy, raw: &serde_json::Value) {
    let Some(obj) = raw.as_object() else {
        return;
    };

    if let Some(v) = obj.get("complex_indicators").and_then(|v| v.as_array()) {
        policy.complex_indicators = v
            .iter()
            .filter_map(|item| item.as_str())
            .map(normalize_indicator)
            .filter(|s| !s.is_empty())
            .collect();
    }
    if let Some(v) = obj.get("medium_indicators").and_then(|v| v.as_array()) {
        policy.medium_indicators = v
            .iter()
            .filter_map(|item| item.as_str())
            .map(normalize_indicator)
            .filter(|s| !s.is_empty())
            .collect();
    }
    if let Some(v) = obj
        .get("long_question_word_threshold")
        .and_then(|v| v.as_u64())
    {
        policy.long_question_word_threshold = v.clamp(5, 1000) as usize;
    }
    if let Some(v) = obj
        .get("long_message_word_threshold")
        .and_then(|v| v.as_u64())
    {
        policy.long_message_word_threshold = v.clamp(5, 1000) as usize;
    }
    if let Some(v) = obj.get("multi_sentence_threshold").and_then(|v| v.as_u64()) {
        policy.multi_sentence_threshold = v.clamp(1, 50) as usize;
    }
}

fn sanitize_policy(policy: &mut RoutingComplexityPolicy) {
    policy.complex_indicators = policy
        .complex_indicators
        .iter()
        .map(|s| normalize_indicator(s))
        .filter(|s| !s.is_empty())
        .take(MAX_INDICATORS)
        .collect::<Vec<_>>();
    dedup_stable(&mut policy.complex_indicators);

    policy.medium_indicators = policy
        .medium_indicators
        .iter()
        .map(|s| normalize_indicator(s))
        .filter(|s| !s.is_empty())
        .take(MAX_INDICATORS)
        .collect::<Vec<_>>();
    dedup_stable(&mut policy.medium_indicators);

    policy.long_question_word_threshold = policy.long_question_word_threshold.clamp(5, 1000);
    policy.long_message_word_threshold = policy.long_message_word_threshold.clamp(5, 1000);
    policy.multi_sentence_threshold = policy.multi_sentence_threshold.clamp(1, 50);
}

fn normalize_indicator(input: &str) -> String {
    input
        .trim()
        .to_ascii_lowercase()
        .chars()
        .take(MAX_INDICATOR_LEN)
        .collect()
}

fn dedup_stable(items: &mut Vec<String>) {
    let mut seen = HashSet::new();
    items.retain(|item| seen.insert(item.clone()));
}

fn append_unique_limited(target: &mut Vec<String>, additions: &[String], max_len: usize) {
    let mut seen: HashSet<String> = target.iter().cloned().collect();
    for item in additions {
        let normalized = normalize_indicator(item);
        if normalized.is_empty() {
            continue;
        }
        if target.len() >= max_len {
            break;
        }
        if seen.insert(normalized.clone()) {
            target.push(normalized);
        }
    }
}

fn extract_keywords(text: &str, min_len: usize, max_items: usize) -> Vec<String> {
    const STOPWORDS: &[&str] = &[
        "about", "across", "after", "all", "also", "and", "any", "are", "because", "been",
        "before", "between", "build", "could", "each", "from", "have", "into", "need", "next",
        "only", "please", "should", "that", "their", "them", "then", "there", "these", "they",
        "this", "those", "through", "under", "using", "with", "would", "your",
    ];
    let stop = STOPWORDS.iter().copied().collect::<HashSet<_>>();
    let mut counts: HashMap<String, usize> = HashMap::new();
    for token in text
        .split(|c: char| !c.is_ascii_alphanumeric())
        .filter(|t| !t.is_empty())
    {
        let t = token.to_ascii_lowercase();
        if t.len() < min_len || stop.contains(t.as_str()) || t.chars().all(|c| c.is_ascii_digit()) {
            continue;
        }
        *counts.entry(t).or_insert(0) += 1;
    }
    let mut sorted = counts.into_iter().collect::<Vec<_>>();
    sorted.sort_by(|(ak, av), (bk, bv)| bv.cmp(av).then_with(|| ak.cmp(bk)));
    sorted
        .into_iter()
        .take(max_items)
        .map(|(token, _)| token)
        .collect()
}

fn parse_json_object(text: &str) -> Option<serde_json::Value> {
    if let Ok(v) = serde_json::from_str::<serde_json::Value>(text.trim()) {
        if v.is_object() {
            return Some(v);
        }
    }
    let start = text.find('{')?;
    let end = text.rfind('}')?;
    if end <= start {
        return None;
    }
    let slice = &text[start..=end];
    serde_json::from_str::<serde_json::Value>(slice)
        .ok()
        .filter(|v| v.is_object())
}

fn policy_hash(policy: &RoutingComplexityPolicy) -> String {
    let as_json = serde_json::to_string(policy).unwrap_or_default();
    let mut hasher = std::collections::hash_map::DefaultHasher::new();
    as_json.hash(&mut hasher);
    format!("{:016x}", hasher.finish())
}

fn build_notes(
    baseline_eval: &PolicyEvaluation,
    best_eval: &PolicyEvaluation,
    stats: &PairedStats,
    source: &str,
) -> Vec<String> {
    vec![
        format!("baseline_correct={}", baseline_eval.correct),
        format!("candidate_correct={}", best_eval.correct),
        format!("wins={} losses={}", stats.wins, stats.losses),
        format!("candidate_source={}", source),
    ]
}

fn round4(value: f64) -> f64 {
    (value * 10_000.0).round() / 10_000.0
}

fn f64_eq(a: f64, b: f64) -> bool {
    (a - b).abs() < f64::EPSILON
}
