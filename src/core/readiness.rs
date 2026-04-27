//! Developmental readiness scoring for ArkEvolve and autonomous action gates.
//!
//! The scorer only uses structured evidence: replay-gate metrics, pattern
//! counters, candidate confidence, and trust envelopes. It deliberately avoids
//! routing or gating on user wording.

use anyhow::Result;
use serde::{Deserialize, Serialize};
use serde_json::{json, Value};

use crate::core::autonomy::RiskEnvelope;
use crate::storage::{learning_candidate, procedural_pattern, readiness_evaluation, Storage};

pub const READINESS_POLICY_SETTINGS_KEY: &str = "readiness_policy_settings_v1";
pub const READINESS_POLICY_VERSION: &str = "readiness-policy-v1";

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ReadinessPolicy {
    #[serde(default = "default_readiness_policy_version")]
    pub version: String,
    #[serde(default = "default_min_review_samples")]
    pub min_review_samples: i32,
    #[serde(default = "default_min_auto_samples")]
    pub min_auto_samples: i32,
    #[serde(default = "default_min_review_success_rate")]
    pub min_review_success_rate: f64,
    #[serde(default = "default_min_auto_success_rate")]
    pub min_auto_success_rate: f64,
    #[serde(default = "default_max_review_correction_rate")]
    pub max_review_correction_rate: f64,
    #[serde(default = "default_max_auto_correction_rate")]
    pub max_auto_correction_rate: f64,
    #[serde(default = "default_min_candidate_review_confidence")]
    pub min_candidate_review_confidence: f64,
    #[serde(default = "default_max_review_trust_score")]
    pub max_review_trust_score: u8,
    #[serde(default = "default_max_auto_trust_score")]
    pub max_auto_trust_score: u8,
}

impl Default for ReadinessPolicy {
    fn default() -> Self {
        Self {
            version: default_readiness_policy_version(),
            min_review_samples: default_min_review_samples(),
            min_auto_samples: default_min_auto_samples(),
            min_review_success_rate: default_min_review_success_rate(),
            min_auto_success_rate: default_min_auto_success_rate(),
            max_review_correction_rate: default_max_review_correction_rate(),
            max_auto_correction_rate: default_max_auto_correction_rate(),
            min_candidate_review_confidence: default_min_candidate_review_confidence(),
            max_review_trust_score: default_max_review_trust_score(),
            max_auto_trust_score: default_max_auto_trust_score(),
        }
        .normalized()
    }
}

impl ReadinessPolicy {
    pub fn normalized(mut self) -> Self {
        self.version = if self.version.trim().is_empty() {
            READINESS_POLICY_VERSION.to_string()
        } else {
            self.version.trim().to_string()
        };
        self.min_review_samples = self.min_review_samples.clamp(1, 10_000);
        self.min_auto_samples = self.min_auto_samples.clamp(self.min_review_samples, 50_000);
        self.min_review_success_rate = clamp_rate(self.min_review_success_rate);
        self.min_auto_success_rate =
            clamp_rate(self.min_auto_success_rate).max(self.min_review_success_rate);
        self.max_review_correction_rate = clamp_rate(self.max_review_correction_rate);
        self.max_auto_correction_rate =
            clamp_rate(self.max_auto_correction_rate).min(self.max_review_correction_rate);
        self.min_candidate_review_confidence = clamp_rate(self.min_candidate_review_confidence);
        self.max_review_trust_score = self.max_review_trust_score.min(100);
        self.max_auto_trust_score = self
            .max_auto_trust_score
            .min(100)
            .min(self.max_review_trust_score);
        self
    }
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct DevelopmentalReadiness {
    pub policy_version: String,
    pub score: u8,
    pub stage: String,
    pub label: String,
    pub plain_summary: String,
    pub allows_review: bool,
    pub allows_auto: bool,
    pub reasons: Vec<String>,
    pub blockers: Vec<String>,
    #[serde(default)]
    pub signals: Value,
}

fn default_readiness_policy_version() -> String {
    READINESS_POLICY_VERSION.to_string()
}

fn default_min_review_samples() -> i32 {
    3
}

fn default_min_auto_samples() -> i32 {
    8
}

fn default_min_review_success_rate() -> f64 {
    0.66
}

fn default_min_auto_success_rate() -> f64 {
    0.85
}

fn default_max_review_correction_rate() -> f64 {
    0.34
}

fn default_max_auto_correction_rate() -> f64 {
    0.10
}

fn default_min_candidate_review_confidence() -> f64 {
    0.70
}

fn default_max_review_trust_score() -> u8 {
    50
}

fn default_max_auto_trust_score() -> u8 {
    25
}

fn clamp_rate(value: f64) -> f64 {
    if value.is_finite() {
        value.clamp(0.0, 1.0)
    } else {
        0.0
    }
}

fn stage_label(allows_auto: bool, allows_review: bool) -> (&'static str, &'static str) {
    if allows_auto {
        ("auto_ready", "Auto-ready")
    } else if allows_review {
        ("review_ready", "Ready for review")
    } else {
        ("watching", "Still learning")
    }
}

fn summarize(stage: &str, reasons: &[String], blockers: &[String]) -> String {
    if stage == "auto_ready" {
        "Enough evidence has accumulated for low-risk automatic use.".to_string()
    } else if stage == "review_ready" {
        "The evidence looks strong enough for a human review step.".to_string()
    } else if let Some(blocker) = blockers.first() {
        blocker.clone()
    } else if let Some(reason) = reasons.first() {
        reason.clone()
    } else {
        "ArkEvolve is still collecting evidence.".to_string()
    }
}

fn build_readiness(
    policy: &ReadinessPolicy,
    score: u8,
    allows_review: bool,
    allows_auto: bool,
    reasons: Vec<String>,
    blockers: Vec<String>,
    signals: Value,
) -> DevelopmentalReadiness {
    let (stage, label) = stage_label(allows_auto, allows_review);
    DevelopmentalReadiness {
        policy_version: policy.version.clone(),
        score,
        stage: stage.to_string(),
        label: label.to_string(),
        plain_summary: summarize(stage, &reasons, &blockers),
        allows_review,
        allows_auto,
        reasons,
        blockers,
        signals,
    }
}

fn score_from_evidence(
    sample_count: i32,
    success_rate: f64,
    correction_rate: f64,
    confidence: Option<f64>,
    policy: &ReadinessPolicy,
) -> u8 {
    let sample_progress =
        (sample_count.max(0) as f64 / policy.min_auto_samples.max(1) as f64).clamp(0.0, 1.0);
    let success_progress = clamp_rate(success_rate);
    let correction_progress = (1.0 - clamp_rate(correction_rate)).clamp(0.0, 1.0);
    let confidence_progress = confidence.map(clamp_rate).unwrap_or(1.0);
    ((sample_progress * 35.0)
        + (success_progress * 30.0)
        + (correction_progress * 20.0)
        + (confidence_progress * 15.0))
        .round()
        .clamp(0.0, 100.0) as u8
}

pub async fn load_readiness_policy(storage: &Storage) -> ReadinessPolicy {
    storage
        .get(READINESS_POLICY_SETTINGS_KEY)
        .await
        .ok()
        .flatten()
        .and_then(|raw| serde_json::from_slice::<ReadinessPolicy>(&raw).ok())
        .unwrap_or_default()
        .normalized()
}

pub async fn save_readiness_policy(storage: &Storage, policy: &ReadinessPolicy) -> Result<()> {
    let normalized = policy.clone().normalized();
    let encoded = serde_json::to_vec(&normalized)?;
    storage
        .set(READINESS_POLICY_SETTINGS_KEY, &encoded)
        .await
        .map(|_| ())
}

pub fn evaluate_procedural_pattern_readiness(
    pattern: &procedural_pattern::Model,
    policy: &ReadinessPolicy,
) -> DevelopmentalReadiness {
    let policy = policy.clone().normalized();
    let sample_count = pattern.sample_count.max(0);
    let success_rate = clamp_rate(pattern.success_rate);
    let correction_rate = if sample_count > 0 {
        clamp_rate(pattern.correction_count.max(0) as f64 / sample_count as f64)
    } else {
        0.0
    };

    let mut reasons = Vec::new();
    let mut blockers = Vec::new();
    if sample_count >= policy.min_review_samples {
        reasons.push(format!(
            "{} supporting run(s) have been seen.",
            sample_count
        ));
    } else {
        blockers.push(format!(
            "Needs at least {} supporting run(s) before review.",
            policy.min_review_samples
        ));
    }
    if success_rate >= policy.min_review_success_rate {
        reasons.push(format!("Success rate is {:.0}%.", success_rate * 100.0));
    } else {
        blockers.push(format!(
            "Success rate must reach {:.0}% before review.",
            policy.min_review_success_rate * 100.0
        ));
    }
    if correction_rate <= policy.max_review_correction_rate {
        reasons.push(format!(
            "Correction rate is {:.0}%.",
            correction_rate * 100.0
        ));
    } else {
        blockers.push(format!(
            "Correction rate must stay at or below {:.0}% before review.",
            policy.max_review_correction_rate * 100.0
        ));
    }

    let allows_review = blockers.is_empty();
    let auto_blockers = [
        sample_count < policy.min_auto_samples,
        success_rate < policy.min_auto_success_rate,
        correction_rate > policy.max_auto_correction_rate,
    ];
    let allows_auto = allows_review && auto_blockers.iter().all(|blocked| !*blocked);
    if !allows_auto {
        if sample_count < policy.min_auto_samples {
            blockers.push(format!(
                "Auto-run needs {} supporting run(s).",
                policy.min_auto_samples
            ));
        }
        if success_rate < policy.min_auto_success_rate {
            blockers.push(format!(
                "Auto-run needs at least {:.0}% success.",
                policy.min_auto_success_rate * 100.0
            ));
        }
        if correction_rate > policy.max_auto_correction_rate {
            blockers.push(format!(
                "Auto-run needs correction rate at or below {:.0}%.",
                policy.max_auto_correction_rate * 100.0
            ));
        }
    }

    build_readiness(
        &policy,
        score_from_evidence(sample_count, success_rate, correction_rate, None, &policy),
        allows_review,
        allows_auto,
        reasons,
        blockers,
        json!({
            "sample_count": sample_count,
            "success_count": pattern.success_count.max(0),
            "correction_count": pattern.correction_count.max(0),
            "success_rate": success_rate,
            "correction_rate": correction_rate,
            "source": "procedural_pattern",
        }),
    )
}

pub fn evaluate_learning_candidate_readiness(
    candidate: &learning_candidate::Model,
    replay_gate: Option<&crate::core::self_evolve::replay_gate::CandidateReplayGateResult>,
    policy: &ReadinessPolicy,
) -> DevelopmentalReadiness {
    let policy = policy.clone().normalized();
    let confidence = clamp_rate(candidate.confidence);
    let sample_count = replay_gate.map(|gate| gate.samples as i32).unwrap_or(0);
    let success_rate = replay_gate
        .map(|gate| clamp_rate(gate.success_rate))
        .unwrap_or(0.0);
    let correction_rate = replay_gate
        .map(|gate| clamp_rate(gate.correction_rate))
        .unwrap_or(0.0);
    let replay_allows_review = replay_gate.map(|gate| gate.allow_approval).unwrap_or(false);

    let mut reasons = Vec::new();
    let mut blockers = Vec::new();
    if confidence >= policy.min_candidate_review_confidence {
        reasons.push(format!("Confidence is {:.0}%.", confidence * 100.0));
    } else {
        blockers.push(format!(
            "Confidence must reach {:.0}% before review.",
            policy.min_candidate_review_confidence * 100.0
        ));
    }
    if sample_count >= policy.min_review_samples {
        reasons.push(format!(
            "{} replay/support sample(s) checked.",
            sample_count
        ));
    } else {
        blockers.push(format!(
            "Needs at least {} replay/support sample(s).",
            policy.min_review_samples
        ));
    }
    if replay_allows_review {
        reasons.push("Replay gate allows human review.".to_string());
    } else if let Some(gate) = replay_gate {
        blockers.push(format!("Replay gate is holding review: {}", gate.reason));
    } else {
        blockers.push("Replay gate has not produced enough evidence yet.".to_string());
    }
    if correction_rate > policy.max_review_correction_rate {
        blockers.push(format!(
            "Correction rate must stay at or below {:.0}% before review.",
            policy.max_review_correction_rate * 100.0
        ));
    }

    let allows_review = blockers.is_empty();
    let allows_auto = allows_review
        && sample_count >= policy.min_auto_samples
        && success_rate >= policy.min_auto_success_rate
        && correction_rate <= policy.max_auto_correction_rate;
    if !allows_auto {
        blockers.push("Automatic use waits for stronger repeated evidence.".to_string());
    }

    build_readiness(
        &policy,
        score_from_evidence(
            sample_count,
            success_rate,
            correction_rate,
            Some(confidence),
            &policy,
        ),
        allows_review,
        allows_auto,
        reasons,
        blockers,
        json!({
            "confidence": confidence,
            "sample_count": sample_count,
            "success_rate": success_rate,
            "correction_rate": correction_rate,
            "replay_allows_review": replay_allows_review,
            "candidate_type": candidate.candidate_type,
            "approval_status": candidate.approval_status,
            "source": "learning_candidate",
        }),
    )
}

pub fn evaluate_recommended_action_readiness(
    trust: &RiskEnvelope,
    policy: &ReadinessPolicy,
) -> DevelopmentalReadiness {
    let policy = policy.clone().normalized();
    let blocked_by_trust = trust.blocked;
    let allows_review = !blocked_by_trust && trust.score <= policy.max_review_trust_score;
    let mut reasons = Vec::new();
    let mut blockers = Vec::new();
    if allows_review {
        reasons.push(format!(
            "Trust risk score {} is inside the review threshold.",
            trust.score
        ));
    } else if blocked_by_trust {
        blockers.push("Trust policy blocks this action.".to_string());
    } else {
        blockers.push(format!(
            "Trust risk score must be {} or lower before review.",
            policy.max_review_trust_score
        ));
    }
    blockers.push(
        "Auto-run needs repeated successful history from ArkEvolve, not only a low risk score."
            .to_string(),
    );

    let trust_component = 100u8.saturating_sub(trust.score);
    build_readiness(
        &policy,
        trust_component,
        allows_review,
        false,
        reasons,
        blockers,
        json!({
            "trust_score": trust.score,
            "trust_level": format!("{:?}", trust.level).to_ascii_lowercase(),
            "requires_approval": trust.requires_approval,
            "source": "recommended_action",
            "max_auto_trust_score": policy.max_auto_trust_score,
        }),
    )
}

pub async fn record_readiness_evaluation(
    storage: &Storage,
    target_type: &str,
    target_id: &str,
    readiness: &DevelopmentalReadiness,
) -> Result<()> {
    storage
        .append_readiness_evaluation(&readiness_evaluation::Model {
            id: uuid::Uuid::new_v4().to_string(),
            target_type: target_type.to_string(),
            target_id: target_id.to_string(),
            score: readiness.score as i32,
            stage: readiness.stage.clone(),
            allows_review: readiness.allows_review,
            allows_auto: readiness.allows_auto,
            reasons_json: json!(readiness.reasons),
            blockers_json: json!(readiness.blockers),
            signals_json: readiness.signals.clone(),
            policy_version: readiness.policy_version.clone(),
            created_at: chrono::Utc::now().to_rfc3339(),
        })
        .await
}
