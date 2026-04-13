use super::*;

const ROUTING_COMPLEXITY_POLICY_KEY: &str = "routing_complexity_policy_v1";
const ROUTING_COMPLEXITY_POLICY_DEFAULT_VERSION: &str = "routing-policy-default-v2";
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

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum SwarmDirective {
    Auto,
    Force,
}

#[derive(Debug, Clone, serde::Serialize, serde::Deserialize)]
struct RoutingComplexityPolicy {
    long_question_word_threshold: usize,
    long_message_word_threshold: usize,
    multi_sentence_threshold: usize,
    structured_line_threshold: usize,
    medium_score_threshold: f32,
    complex_score_threshold: f32,
}

impl Default for RoutingComplexityPolicy {
    fn default() -> Self {
        Self {
            long_question_word_threshold: 50,
            long_message_word_threshold: 30,
            multi_sentence_threshold: 3,
            structured_line_threshold: 4,
            medium_score_threshold: 0.38,
            complex_score_threshold: 0.72,
        }
    }
}

impl Agent {
    fn apply_routing_complexity_policy_override(
        policy: &mut RoutingComplexityPolicy,
        raw: &serde_json::Value,
    ) {
        let Some(obj) = raw.as_object() else {
            return;
        };

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
        if let Some(v) = obj
            .get("structured_line_threshold")
            .and_then(|v| v.as_u64())
        {
            policy.structured_line_threshold = v.clamp(2, 50) as usize;
        }
        if let Some(v) = obj
            .get("medium_score_threshold")
            .and_then(|v| v.as_f64())
            .map(|v| v as f32)
        {
            policy.medium_score_threshold = v.clamp(0.05, 0.95);
        }
        if let Some(v) = obj
            .get("complex_score_threshold")
            .and_then(|v| v.as_f64())
            .map(|v| v as f32)
        {
            policy.complex_score_threshold = v.clamp(0.10, 0.99);
        }
        if policy.complex_score_threshold <= policy.medium_score_threshold {
            policy.complex_score_threshold = (policy.medium_score_threshold + 0.05).min(0.99);
        }
    }

    fn routing_seed_for_message(message: &str) -> String {
        let normalized = message.trim().to_ascii_lowercase();
        if normalized.is_empty() {
            "_empty".to_string()
        } else {
            normalized
        }
    }

    async fn load_routing_complexity_policy_for_message(
        &self,
        message: &str,
    ) -> (RoutingComplexityPolicy, String) {
        let mut policy = RoutingComplexityPolicy::default();
        let mut selected_version = ROUTING_COMPLEXITY_POLICY_DEFAULT_VERSION.to_string();

        if let Ok(raw_env) = std::env::var("AGENTARK_ROUTING_COMPLEXITY_POLICY_JSON") {
            match serde_json::from_str::<serde_json::Value>(&raw_env) {
                Ok(value) => Self::apply_routing_complexity_policy_override(&mut policy, &value),
                Err(e) => tracing::warn!(
                    "Invalid AGENTARK_ROUTING_COMPLEXITY_POLICY_JSON ignored: {}",
                    e
                ),
            }
        }

        if let Ok(Some(raw)) = self.storage.get(ROUTING_COMPLEXITY_POLICY_KEY).await {
            match serde_json::from_slice::<serde_json::Value>(&raw) {
                Ok(value) => Self::apply_routing_complexity_policy_override(&mut policy, &value),
                Err(e) => tracing::warn!(
                    "Invalid routing complexity policy in storage ignored: {}",
                    e
                ),
            }
        }

        let baseline_policy = policy.clone();

        if let Ok(Some(raw_state)) = self
            .storage
            .get(crate::core::self_evolve::strategy_runtime::ROUTING_COMPLEXITY_CANARY_STATE_KEY)
            .await
        {
            match serde_json::from_slice::<
                crate::core::self_evolve::strategy_runtime::CanaryRolloutState,
            >(&raw_state)
            {
                Ok(state) if state.enabled => {
                    selected_version = state.baseline_version;
                    if crate::core::self_evolve::strategy_runtime::should_use_canary(
                        &Self::routing_seed_for_message(message),
                        state.rollout_percent,
                    ) {
                        if let Ok(Some(raw_canary)) = self
                            .storage
                            .get(
                                crate::core::self_evolve::strategy_runtime::ROUTING_COMPLEXITY_POLICY_CANARY_KEY,
                            )
                            .await
                        {
                            match serde_json::from_slice::<serde_json::Value>(&raw_canary) {
                                Ok(value) => {
                                    let mut canary_policy = baseline_policy.clone();
                                    Self::apply_routing_complexity_policy_override(
                                        &mut canary_policy,
                                        &value,
                                    );
                                    policy = canary_policy;
                                    selected_version = state.candidate_version;
                                }
                                Err(e) => {
                                    tracing::warn!(
                                        "Invalid routing complexity canary policy ignored: {}",
                                        e
                                    );
                                }
                            }
                        }
                    }
                }
                Ok(_) => {}
                Err(e) => tracing::warn!(
                    "Invalid routing complexity canary state in storage ignored: {}",
                    e
                ),
            }
        }

        (policy, selected_version)
    }

    pub(crate) async fn active_routing_policy_version_for_message(&self, message: &str) -> String {
        let (_, version) = self
            .load_routing_complexity_policy_for_message(message)
            .await;
        version
    }

    /// Select the best model role based on message content and complexity
    pub(crate) fn select_model_role(
        &self,
        message: &str,
        complexity: &QueryComplexity,
    ) -> ModelRole {
        if !self.config.model_pool.smart_routing {
            return ModelRole::Primary;
        }
        let trimmed = message.trim();
        let word_count = trimmed.split_whitespace().count();
        let question_count = trimmed.matches('?').count();
        let has_role = |role: ModelRole| {
            self.model_pool
                .values()
                .any(|(s, _)| s.role == role && s.enabled)
        };

        let code_syntax_signal = message.contains("```")
            || message.contains("fn ")
            || message.contains("def ")
            || message.contains("SELECT ")
            || message.contains("class ")
            || message.contains("import ")
            || message.contains("=>");
        let symbol_chars = trimmed
            .chars()
            .filter(|c| "{}[]();:=<>/\\#`".contains(*c))
            .count();
        let symbol_ratio = if trimmed.is_empty() {
            0.0
        } else {
            symbol_chars as f32 / trimmed.chars().count().max(1) as f32
        };

        if has_role(ModelRole::Code)
            && (code_syntax_signal || (symbol_ratio >= 0.08 && word_count >= 12))
        {
            return ModelRole::Code;
        }
        if has_role(ModelRole::Research)
            && matches!(complexity, QueryComplexity::Complex)
            && (question_count > 0 || word_count >= 90)
        {
            return ModelRole::Research;
        }

        match complexity {
            QueryComplexity::Simple => {
                if has_role(ModelRole::Fast) {
                    ModelRole::Fast
                } else {
                    ModelRole::Primary
                }
            }
            _ => ModelRole::Primary,
        }
    }

    /// LLM-based routing: decide if we need sub-agents and what kind
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

        let (routing_policy_hint, routing_policy_version) = self
            .load_routing_complexity_policy_for_message(message)
            .await;
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
        let preferred_direct_action = "none".to_string();
        let policy_hint_block = format!(
            "Active routing policy version: {}\n\
Routing fallback signals are structure-first (not keyword lists).\n\
Thresholds: long_question_word_threshold={}, long_message_word_threshold={}, multi_sentence_threshold={}, structured_line_threshold={}, medium_score_threshold={:.2}, complex_score_threshold={:.2}",
            routing_policy_version,
            routing_policy_hint.long_question_word_threshold,
            routing_policy_hint.long_message_word_threshold,
            routing_policy_hint.multi_sentence_threshold,
            routing_policy_hint.structured_line_threshold,
            routing_policy_hint.medium_score_threshold,
            routing_policy_hint.complex_score_threshold,
        );

        let routing_prompt = crate::core::self_evolve::prompt_evolution::render_router_user_prompt(
            prompt_bundle,
            &crate::core::self_evolve::prompt_evolution::RouterPromptRenderInputs {
                specialists: &specialist_desc,
                policy_hint: &policy_hint_block,
                action_hints: &action_hint_block,
                preferred_action: &preferred_direct_action,
                message,
            },
        );
        let router_system_prompt =
            crate::core::self_evolve::prompt_evolution::render_router_system_prompt(prompt_bundle);

        let empty_actions: Vec<crate::actions::ActionDef> = Vec::new();
        let mut router_response: Option<crate::core::llm::LlmResponse> = None;
        let mut router_errors: Vec<String> = Vec::new();
        let timeout_ms = router_call_timeout_ms();
        for (idx, candidate) in router_candidates.iter().enumerate() {
            if idx > 0 {
                tracing::warn!(
                    "Routing self-heal: switching router model to {} ({}) after previous failure",
                    candidate.slot_label,
                    candidate.client.model_name()
                );
            }
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
                Ok(Err(e)) => {
                    let err_msg = format!(
                        "{} ({}) failed: {}",
                        candidate.slot_label,
                        candidate.client.model_name(),
                        e
                    );
                    tracing::warn!("Routing model attempt failed: {}", err_msg);
                    router_errors.push(err_msg);
                }
                Err(_) => {
                    let err_msg = format!(
                        "{} ({}) timed out after {}ms",
                        candidate.slot_label,
                        candidate.client.model_name(),
                        timeout_ms
                    );
                    tracing::warn!("Routing model attempt timed out: {}", err_msg);
                    router_errors.push(err_msg);
                }
            };
        }

        match router_response {
            Some(response) => {
                let content = response.content.trim();
                match parse_routing_decision_from_text(content) {
                    Some(mut decision) => {
                        if !(0.0..=1.0).contains(&decision.confidence) || decision.confidence <= 0.0
                        {
                            decision.confidence = if decision.needs_delegation {
                                0.75
                            } else {
                                0.65
                            };
                        }
                        decision
                    }
                    None => {
                        tracing::warn!(
                            "Failed to parse routing JSON, falling back to structural classifier"
                        );
                        self.classify_complexity_fallback(message, actions).await
                    }
                }
            }
            None => {
                tracing::warn!(
                    "Routing LLM call failed across all candidates, falling back to structural classifier: {}",
                    router_errors.join(" | ")
                );
                self.classify_complexity_fallback(message, actions).await
            }
        }
    }

    /// Conservative fallback used when semantic routing is unavailable.
    pub(crate) async fn classify_complexity_fallback(
        &self,
        message: &str,
        actions: &[crate::actions::ActionDef],
    ) -> crate::core::task_router::RoutingDecision {
        let _ = (self, message, actions);
        crate::core::task_router::RoutingDecision {
            needs_delegation: false,
            complexity: QueryComplexity::Simple,
            sub_agents: vec![],
            reasoning: "Conservative fallback classification".to_string(),
            confidence: 0.40,
            should_clarify: false,
            clarification_question: None,
        }
    }
    pub(crate) fn detect_swarm_directive(&self, message: &str) -> SwarmDirective {
        let trimmed = message.trim();
        if trimmed.eq_ignore_ascii_case("/delegate")
            || trimmed.to_ascii_lowercase().starts_with("/delegate ")
        {
            return SwarmDirective::Force;
        }

        SwarmDirective::Auto
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

    pub(crate) fn apply_swarm_directive(
        &self,
        message: &str,
        actions: &[crate::actions::ActionDef],
        decision: &mut crate::core::task_router::RoutingDecision,
        directive: SwarmDirective,
    ) {
        match directive {
            SwarmDirective::Force => {
                decision.needs_delegation = true;
                decision.complexity = QueryComplexity::Complex;
                if decision.sub_agents.len() < 2 {
                    decision.sub_agents = self.forced_swarm_specs(message, actions);
                }
                decision.confidence = decision.confidence.max(0.96);
                decision.reasoning = format!(
                    "{} | Explicit structured delegation command forced multi-agent execution.",
                    decision.reasoning
                );
            }
            SwarmDirective::Auto => {
                if decision.needs_delegation && decision.sub_agents.len() < 2 {
                    decision.needs_delegation = false;
                    decision.sub_agents.clear();
                    decision.reasoning = format!(
                        "{} | Delegation suppressed because the task did not decompose into 2+ usable agents.",
                        decision.reasoning
                    );
                }
            }
        }
    }

    fn structural_complexity_score(message: &str, policy: &RoutingComplexityPolicy) -> f32 {
        let trimmed = message.trim();
        if trimmed.is_empty() {
            return 0.0;
        }

        let word_count = trimmed.split_whitespace().count();
        let line_count = trimmed.lines().count();
        let sentence_count = trimmed.matches('.').count()
            + trimmed.matches('?').count()
            + trimmed.matches('!').count();
        let question_count = trimmed.matches('?').count();
        let has_code_block = trimmed.contains("```");
        let has_list_shape = trimmed
            .lines()
            .map(str::trim_start)
            .any(|line| line.starts_with("- ") || line.starts_with("* ") || line.starts_with("1."));
        let has_structured_layout =
            line_count >= policy.structured_line_threshold || has_code_block || has_list_shape;

        let mut score = 0.0_f32;

        if word_count >= policy.long_message_word_threshold {
            let span = (policy.long_message_word_threshold.max(1)) as f32;
            let normalized = ((word_count as f32 - span) / span).clamp(0.0, 1.0);
            score += 0.35 * normalized.max(0.25);
        }

        if sentence_count >= policy.multi_sentence_threshold {
            let normalized = (sentence_count as f32 / policy.multi_sentence_threshold as f32)
                .clamp(1.0, 3.0)
                / 3.0;
            score += 0.22 * normalized;
        }

        if question_count > 0 && word_count >= policy.long_question_word_threshold {
            score += 0.14;
        } else if question_count >= 2 {
            score += 0.08;
        }

        if has_structured_layout {
            score += 0.16;
        }
        if line_count >= policy.structured_line_threshold + 2 {
            score += 0.10;
        }
        if has_code_block {
            score += 0.10;
        }

        score.clamp(0.0, 1.0)
    }

    /// Classify query complexity for routing (structure-first fallback)
    pub(crate) async fn classify_complexity(&self, message: &str) -> QueryComplexity {
        let (policy, _) = self
            .load_routing_complexity_policy_for_message(message)
            .await;
        let score = Self::structural_complexity_score(message, &policy);

        if score >= policy.complex_score_threshold {
            return QueryComplexity::Complex;
        }
        if score >= policy.medium_score_threshold {
            return QueryComplexity::Medium;
        }
        QueryComplexity::Simple
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
