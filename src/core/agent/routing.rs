use super::*;

const ROUTER_CALL_TIMEOUT_MS: u64 = 12_000;

fn router_call_timeout_ms() -> u64 {
    std::env::var("AGENTARK_ROUTER_TIMEOUT_MS")
        .ok()
        .and_then(|v| v.parse::<u64>().ok())
        .filter(|ms| *ms >= 500 && *ms <= 60_000)
        .unwrap_or(ROUTER_CALL_TIMEOUT_MS)
}

fn extract_first_json_object(raw: &str) -> Option<String> {
    let mut start_idx: Option<usize> = None;
    let mut depth = 0usize;
    let mut in_string = false;
    let mut escape = false;

    for (idx, ch) in raw.char_indices() {
        if start_idx.is_none() {
            if ch == '{' {
                start_idx = Some(idx);
                depth = 1;
                in_string = false;
                escape = false;
            }
            continue;
        }

        if escape {
            escape = false;
            continue;
        }

        match ch {
            '\\' if in_string => {
                escape = true;
            }
            '"' => {
                in_string = !in_string;
            }
            '{' if !in_string => {
                depth += 1;
            }
            '}' if !in_string => {
                depth = depth.saturating_sub(1);
                if depth == 0 {
                    if let Some(start) = start_idx {
                        return raw.get(start..=idx).map(|s| s.to_string());
                    }
                    return None;
                }
            }
            _ => {}
        }
    }

    None
}

fn parse_routing_decision_from_text(
    raw: &str,
) -> Option<crate::core::task_router::RoutingDecision> {
    let trimmed = raw.trim();
    if trimmed.is_empty() {
        return None;
    }

    if let Ok(decision) = serde_json::from_str::<crate::core::task_router::RoutingDecision>(trimmed)
    {
        return Some(decision);
    }

    extract_first_json_object(trimmed).and_then(|json| {
        serde_json::from_str::<crate::core::task_router::RoutingDecision>(&json).ok()
    })
}

fn structural_complexity_score(message: &str) -> f32 {
    let trimmed = message.trim();
    if trimmed.is_empty() {
        return 0.0;
    }

    let word_count = trimmed.split_whitespace().count() as f32;
    let line_count = trimmed.lines().count() as f32;
    let question_count = trimmed.matches('?').count() as f32;
    let has_code_block = trimmed.contains("```");
    let has_list_shape = trimmed
        .lines()
        .map(str::trim_start)
        .any(|line| line.starts_with("- ") || line.starts_with("* ") || line.starts_with("1."));

    let mut score = 0.0_f32;
    if word_count >= 30.0 {
        score += ((word_count - 30.0) / 60.0).clamp(0.1, 0.4);
    }
    if line_count >= 4.0 {
        score += (line_count / 10.0).clamp(0.08, 0.2);
    }
    if question_count >= 2.0 {
        score += 0.08;
    }
    if has_code_block {
        score += 0.12;
    }
    if has_list_shape {
        score += 0.12;
    }
    score.clamp(0.0, 1.0)
}

impl Agent {
    async fn classify_complexity_fallback(
        &self,
        message: &str,
    ) -> crate::core::task_router::RoutingDecision {
        let score = structural_complexity_score(message);
        let complexity = if score >= 0.72 {
            QueryComplexity::Complex
        } else if score >= 0.38 {
            QueryComplexity::Medium
        } else {
            QueryComplexity::Simple
        };
        crate::core::task_router::RoutingDecision {
            needs_delegation: false,
            complexity,
            sub_agents: vec![],
            reasoning: "Structural fallback classification".to_string(),
            confidence: score,
            should_clarify: false,
            clarification_question: None,
        }
    }

    pub(crate) async fn route_query(
        &self,
        message: &str,
        actions: &[crate::actions::ActionDef],
        prompt_bundle: &crate::core::self_evolve::PromptBundleProfile,
    ) -> crate::core::task_router::RoutingDecision {
        let router_candidates = self.llm_candidates_for_role(&ModelRole::Fast);

        let specialist_desc = if self.config.swarm.specialists.is_empty() {
            "None configured.".to_string()
        } else {
            self.config
                .swarm
                .specialists
                .iter()
                .filter(|s| s.enabled)
                .map(|s| {
                    format!(
                        "- {} ({:?}): {}",
                        s.name,
                        s.agent_type,
                        s.capabilities
                            .iter()
                            .map(|c| c.description.clone())
                            .collect::<Vec<_>>()
                            .join(", ")
                    )
                })
                .collect::<Vec<_>>()
                .join("\n")
        };

        let action_hint_block = if actions.is_empty() {
            "No registered actions available.".to_string()
        } else {
            actions
                .iter()
                .take(8)
                .map(|action| {
                    format!(
                        "- {}: {}",
                        action.name,
                        super::safe_truncate(action.description.trim(), 120)
                    )
                })
                .collect::<Vec<_>>()
                .join("\n")
        };

        let preferred_direct_action =
            crate::core::intent::preferred_direct_action_name(message, actions)
                .unwrap_or_else(|| "none".to_string());
        let policy_hint_block = "Fallback routing is structure-first. Do not infer a delegated plan from keyword matches alone.";

        let routing_prompt = crate::core::self_evolve::prompt_evolution::render_router_user_prompt(
            prompt_bundle,
            &crate::core::self_evolve::prompt_evolution::RouterPromptRenderInputs {
                specialists: &specialist_desc,
                policy_hint: policy_hint_block,
                action_hints: &action_hint_block,
                preferred_action: &preferred_direct_action,
                message,
            },
        );
        let router_system_prompt =
            crate::core::self_evolve::prompt_evolution::render_router_system_prompt(prompt_bundle);

        let empty_actions: Vec<crate::actions::ActionDef> = Vec::new();
        let mut router_response: Option<crate::core::llm::LlmResponse> = None;
        let timeout_ms = router_call_timeout_ms();

        for candidate in router_candidates {
            let route_call =
                candidate
                    .client
                    .chat(&router_system_prompt, &routing_prompt, &[], &empty_actions);
            match tokio::time::timeout(std::time::Duration::from_millis(timeout_ms), route_call)
                .await
            {
                Ok(Ok(resp)) => {
                    router_response = Some(resp);
                    break;
                }
                Ok(Err(error)) => {
                    tracing::warn!(
                        "Routing model attempt failed for {} ({}): {}",
                        candidate.slot_label,
                        candidate.client.model_name(),
                        error
                    );
                }
                Err(_) => {
                    tracing::warn!(
                        "Routing model attempt timed out for {} ({}) after {}ms",
                        candidate.slot_label,
                        candidate.client.model_name(),
                        timeout_ms
                    );
                }
            }
        }

        match router_response {
            Some(response) => match parse_routing_decision_from_text(response.content.trim()) {
                Some(mut decision) => {
                    if !(0.0..=1.0).contains(&decision.confidence) || decision.confidence <= 0.0 {
                        decision.confidence = if decision.needs_delegation {
                            0.75
                        } else {
                            0.65
                        };
                    }
                    decision
                }
                None => {
                    tracing::warn!("Failed to parse routing JSON, using structural fallback");
                    self.classify_complexity_fallback(message).await
                }
            },
            None => self.classify_complexity_fallback(message).await,
        }
    }

    pub(crate) fn forced_swarm_specs(
        &self,
        message: &str,
        actions: &[crate::actions::ActionDef],
    ) -> Vec<crate::core::task_router::SubAgentSpec> {
        let preferred_action = crate::core::intent::preferred_direct_action_name(message, actions)
            .and_then(|name| actions.iter().find(|action| action.name == name));
        let (primary_agent_type, primary_task, primary_role) = preferred_action
            .map(|action| {
                let metadata = action.planner_metadata();
                if matches!(
                    metadata.integration_class,
                    crate::actions::PlannerIntegrationClass::Code
                        | crate::actions::PlannerIntegrationClass::Filesystem
                        | crate::actions::PlannerIntegrationClass::App
                ) || matches!(metadata.role, crate::actions::PlannerActionRole::Mutation)
                {
                    (
                        "Coder".to_string(),
                        format!(
                            "Drive the implementation or technical solution for this request using the best matching action path: {}",
                            message.trim()
                        ),
                        Some("Code".to_string()),
                    )
                } else if matches!(
                    metadata.integration_class,
                    crate::actions::PlannerIntegrationClass::Search
                        | crate::actions::PlannerIntegrationClass::Analytics
                        | crate::actions::PlannerIntegrationClass::Network
                ) || matches!(metadata.role, crate::actions::PlannerActionRole::DataSource)
                {
                    (
                        "Researcher".to_string(),
                        format!(
                            "Gather the key facts, sources, and relevant context for this delegated request: {}",
                            message.trim()
                        ),
                        Some("Research".to_string()),
                    )
                } else if matches!(
                    metadata.integration_class,
                    crate::actions::PlannerIntegrationClass::Messaging
                        | crate::actions::PlannerIntegrationClass::Workspace
                ) || matches!(metadata.role, crate::actions::PlannerActionRole::Delivery)
                {
                    (
                        "Writer".to_string(),
                        format!(
                            "Draft the user-facing communication or delivery artifact for this request: {}",
                            message.trim()
                        ),
                        None,
                    )
                } else {
                    (
                        "Analyst".to_string(),
                        format!(
                            "Evaluate the strongest execution path, tradeoffs, and risks for this delegated request: {}",
                            message.trim()
                        ),
                        None,
                    )
                }
            })
            .unwrap_or_else(|| {
                (
                    "Analyst".to_string(),
                    format!(
                        "Evaluate the strongest execution path, tradeoffs, and risks for this delegated request: {}",
                        message.trim()
                    ),
                    None,
                )
            });

        vec![
            crate::core::task_router::SubAgentSpec {
                agent_type: "Planner".to_string(),
                task: format!(
                    "Decompose this request into clear execution tracks with dependencies, risks, and acceptance criteria: {}",
                    message.trim()
                ),
                preferred_model_role: None,
                depends_on: vec![],
                plan_step_id: None,
            },
            crate::core::task_router::SubAgentSpec {
                agent_type: primary_agent_type,
                task: primary_task,
                preferred_model_role: primary_role,
                depends_on: vec![],
                plan_step_id: None,
            },
            crate::core::task_router::SubAgentSpec {
                agent_type: "Validator".to_string(),
                task: format!(
                    "Review the plan and delegated result for correctness, risks, and missing checks: {}",
                    message.trim()
                ),
                preferred_model_role: None,
                depends_on: vec![0, 1],
                plan_step_id: None,
            },
        ]
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn extracts_routing_json_from_wrapper_prose() {
        let content = "Routing decision: {\"needs_delegation\":false,\"complexity\":\"simple\",\"sub_agents\":[],\"reasoning\":\"brief\",\"confidence\":0.8,\"should_clarify\":false,\"clarification_question\":null}";
        let parsed =
            parse_routing_decision_from_text(content).expect("wrapped JSON should still parse");
        assert!(!parsed.needs_delegation);
        assert!(matches!(parsed.complexity, QueryComplexity::Simple));
    }

    #[test]
    fn extracts_first_json_object_from_mixed_text() {
        let content = "ignore this {\"needs_delegation\":true,\"complexity\":\"complex\",\"sub_agents\":[{\"agent_type\":\"Planner\",\"task\":\"x\",\"preferred_model_role\":null,\"depends_on\":[]}],\"reasoning\":\"brief\",\"confidence\":0.9,\"should_clarify\":false,\"clarification_question\":null} trailing prose";
        let extracted = extract_first_json_object(content).expect("json object should be found");
        assert!(extracted.contains("\"needs_delegation\":true"));
    }
}
