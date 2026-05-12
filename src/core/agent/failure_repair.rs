use super::*;

#[derive(Debug, Clone, Serialize, PartialEq, Eq)]
#[serde(rename_all = "snake_case")]
pub(super) enum FailureRepairAction {
    Continue,
    UseAlternative,
    Clarify,
    Stop,
}

#[derive(Debug, Clone, Serialize)]
pub(super) struct FailureRepairDecision {
    pub action: FailureRepairAction,
    pub reason: String,
    pub run_status: &'static str,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub alternative_action: Option<String>,
    pub user_message: String,
}

impl FailureRepairDecision {
    pub(super) fn trace_event(&self, iteration: usize) -> serde_json::Value {
        serde_json::json!({
            "iteration": iteration,
            "status": "tool_failure_repair",
            "action": &self.action,
            "reason": &self.reason,
            "alternative_action": &self.alternative_action,
            "run_status": self.run_status,
            "message": &self.user_message,
        })
    }
}

pub(super) fn decide_failure_repair(
    call: &crate::core::llm::ToolCall,
    failure_kind: &str,
    parsed_result: &serde_json::Value,
    semantic_turn: &super::semantic_turn::SemanticTurnBundle,
    action_by_name: &HashMap<String, crate::actions::ActionDef>,
    capability_health: Option<&super::capability_health::CapabilityHealthSnapshot>,
) -> FailureRepairDecision {
    let remediation_type = parsed_result
        .get("remediation")
        .and_then(|value| value.get("type"))
        .and_then(|value| value.as_str())
        .map(normalize_kind);
    let normalized = normalize_kind(failure_kind);

    if matches!(
        normalized.as_str(),
        "missinginput" | "invalidinput" | "ambiguous"
    ) || remediation_type.as_deref() == Some("clarify")
    {
        return FailureRepairDecision {
            action: FailureRepairAction::Clarify,
            reason: normalized,
            run_status: crate::core::ExecutionRunStatus::NeedsInput.as_str(),
            alternative_action: None,
            user_message: "The selected action needs a missing or more precise input before it can run. I should ask for that detail instead of retrying the same call.".to_string(),
        };
    }

    if matches!(
        normalized.as_str(),
        "notconnected" | "bundlenotgranted" | "permissiondenied" | "approvalrequired"
    ) {
        return FailureRepairDecision {
            action: FailureRepairAction::Clarify,
            reason: normalized,
            run_status: crate::core::ExecutionRunStatus::NeedsInput.as_str(),
            alternative_action: None,
            user_message: "A required authorization, integration, or approval is not ready. I should surface that precondition instead of creating another attempt.".to_string(),
        };
    }

    if matches!(normalized.as_str(), "ratelimited" | "timeout")
        || remediation_type.as_deref() == Some("retry")
    {
        return FailureRepairDecision {
            action: FailureRepairAction::Stop,
            reason: normalized,
            run_status: crate::core::ExecutionRunStatus::PlatformFailed.as_str(),
            alternative_action: None,
            user_message: "The action is blocked by a temporary runtime limit or timeout. I should stop this loop and report the temporary blocker.".to_string(),
        };
    }

    if matches!(normalized.as_str(), "unavailable" | "notfound" | "failed" | "error") {
        if let Some(alternative) =
            best_alternative_action(call, semantic_turn, action_by_name, capability_health)
        {
            return FailureRepairDecision {
                action: FailureRepairAction::UseAlternative,
                reason: normalized,
                run_status: crate::core::ExecutionRunStatus::ToolDispatch.as_str(),
                alternative_action: Some(alternative),
                user_message: "The selected capability failed in a way that may be recoverable with another resolved candidate for the same semantic goal.".to_string(),
            };
        }
    }

    FailureRepairDecision {
        action: FailureRepairAction::Continue,
        reason: normalized,
        run_status: crate::core::ExecutionRunStatus::ToolDispatch.as_str(),
        alternative_action: None,
        user_message: "The failure did not provide a safe deterministic repair. Let the guarded loop decide whether a different tool call is needed.".to_string(),
    }
}

fn best_alternative_action(
    call: &crate::core::llm::ToolCall,
    semantic_turn: &super::semantic_turn::SemanticTurnBundle,
    action_by_name: &HashMap<String, crate::actions::ActionDef>,
    capability_health: Option<&super::capability_health::CapabilityHealthSnapshot>,
) -> Option<String> {
    for step in &semantic_turn.resolved_steps {
        let step_mentions_failed_action = step.action_name.as_deref() == Some(call.name.as_str())
            || step
                .candidates
                .iter()
                .any(|candidate| candidate.action_name == call.name);
        if !step_mentions_failed_action {
            continue;
        }
        for candidate in &step.candidates {
            if candidate.action_name == call.name {
                continue;
            }
            let Some(action) = action_by_name.get(&candidate.action_name) else {
                continue;
            };
            if action.action_metadata().tool_role.is_support() {
                continue;
            }
            let ready = capability_health
                .and_then(|snapshot| snapshot.entry(&candidate.action_name))
                .map(|entry| {
                    matches!(
                        &entry.readiness,
                        super::capability_health::CapabilityReadiness::Ready
                            | super::capability_health::CapabilityReadiness::Degraded
                            | super::capability_health::CapabilityReadiness::Unknown
                    )
                })
                .unwrap_or(true);
            if ready {
                return Some(candidate.action_name.clone());
            }
        }
    }
    None
}

pub(super) fn repair_user_text(decision: &FailureRepairDecision) -> String {
    decision.user_message.clone()
}

fn normalize_kind(value: &str) -> String {
    value
        .trim()
        .chars()
        .filter(|ch| ch.is_ascii_alphanumeric())
        .flat_map(char::to_lowercase)
        .collect()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn missing_input_repair_clarifies_without_retry() {
        let call = crate::core::llm::ToolCall {
            id: "a".to_string(),
            name: "reader".to_string(),
            arguments: serde_json::json!({}),
        };
        let bundle = super::super::semantic_turn::SemanticTurnBundle::default();
        let decision = decide_failure_repair(
            &call,
            "missing_input",
            &serde_json::json!({"status": "error", "reason": "missing_input"}),
            &bundle,
            &HashMap::new(),
            None,
        );

        assert_eq!(decision.action, FailureRepairAction::Clarify);
        assert_eq!(decision.run_status, crate::core::ExecutionRunStatus::NeedsInput.as_str());
    }
}
