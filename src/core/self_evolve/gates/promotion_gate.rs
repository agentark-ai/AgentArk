use serde::{Deserialize, Serialize};

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "snake_case")]
pub enum PromotionGateOutcome {
    Passed,
    Rejected,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct PromotionGateReason {
    pub code: String,
    pub label: String,
}

impl PromotionGateReason {
    pub fn new(code: impl Into<String>, label: impl Into<String>) -> Self {
        Self {
            code: code.into(),
            label: label.into(),
        }
    }
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct PromotionGateReport {
    pub outcome: PromotionGateOutcome,
    pub summary: String,
    #[serde(default)]
    pub reasons: Vec<PromotionGateReason>,
}

impl PromotionGateReport {
    pub fn passed() -> Self {
        Self {
            outcome: PromotionGateOutcome::Passed,
            summary: "Passed all promotion checks.".to_string(),
            reasons: Vec::new(),
        }
    }

    pub fn rejected(reasons: Vec<PromotionGateReason>) -> Self {
        let summary = if reasons.is_empty() {
            "Not promoted: the candidate did not pass the promotion checks.".to_string()
        } else {
            let joined = reasons
                .iter()
                .map(|reason| reason.label.as_str())
                .collect::<Vec<_>>()
                .join("; ");
            format!("Not promoted: {}.", joined)
        };
        Self {
            outcome: PromotionGateOutcome::Rejected,
            summary,
            reasons,
        }
    }
}

pub(crate) trait PromotionGateCheck {
    fn code(self) -> &'static str;
    fn label(self) -> &'static str;
}

#[derive(Debug, Clone, Copy)]
pub(crate) struct PromotionGateCheckResult<T> {
    pub check: T,
    pub passed: bool,
}

pub(crate) fn render_legacy_promotion_gate<T>(checks: &[PromotionGateCheckResult<T>]) -> String
where
    T: PromotionGateCheck + Copy,
{
    let failed = checks
        .iter()
        .filter(|result| !result.passed)
        .map(|result| result.check.code())
        .collect::<Vec<_>>();
    if failed.is_empty() {
        "passed".to_string()
    } else {
        format!("rejected: {}", failed.join(", "))
    }
}

pub(crate) fn promotion_gate_report<T>(
    checks: &[PromotionGateCheckResult<T>],
) -> PromotionGateReport
where
    T: PromotionGateCheck + Copy,
{
    let reasons = checks
        .iter()
        .filter(|result| !result.passed)
        .map(|result| PromotionGateReason::new(result.check.code(), result.check.label()))
        .collect::<Vec<_>>();
    if reasons.is_empty() {
        PromotionGateReport::passed()
    } else {
        PromotionGateReport::rejected(reasons)
    }
}
