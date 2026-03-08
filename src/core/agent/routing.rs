use super::*;

const ROUTING_COMPLEXITY_POLICY_KEY: &str = "routing_complexity_policy_v1";
const ROUTING_COMPLEXITY_POLICY_DEFAULT_VERSION: &str = "routing-policy-default-v2";
const ROUTER_CALL_TIMEOUT_MS: u64 = 3500;

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
        let mut scored_actions: Vec<(f32, &crate::actions::ActionDef)> = actions
            .iter()
            .map(|action| {
                (
                    crate::core::intent::action_intent_score(message, action),
                    action,
                )
            })
            .collect();
        scored_actions.sort_by(|a, b| {
            b.0.partial_cmp(&a.0)
                .unwrap_or(std::cmp::Ordering::Equal)
                .then_with(|| a.1.name.cmp(&b.1.name))
        });
        let action_hint_block = if scored_actions.is_empty() {
            "No registered actions available.".to_string()
        } else {
            scored_actions
                .iter()
                .take(8)
                .map(|(score, action)| {
                    format!(
                        "- {} ({:.2}): {}",
                        action.name,
                        score,
                        super::safe_truncate(action.description.trim(), 120)
                    )
                })
                .collect::<Vec<_>>()
                .join("\n")
        };
        let preferred_direct_action =
            crate::core::intent::preferred_direct_action_name(message, actions)
                .unwrap_or_else(|| "none".to_string());
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

        let routing_prompt = format!(
            r#"Analyze this task and decide the execution strategy. Respond with ONLY valid JSON.

Available agent types for sub-agents: Researcher, Coder, Analyst, Writer, Validator, Planner
Available model roles: Primary, Fast, Code, Research
Custom specialists: {specialists}
{router_policy}
{policy_hint}
Top semantic action candidates:
{action_hints}
Preferred direct action candidate: {preferred_action}

Rules:
- "needs_delegation": true ONLY for pure analysis/research tasks that truly need multiple independent agents.
- For executable tasks that map clearly to available actions, prefer direct execution:
  needs_delegation=false unless there is explicit parallel decomposition.
- Set should_clarify=true only when the request is ambiguous or missing critical details.
- Any retry/repair strategy MUST define a hard maximum attempts cap.
- confidence is a number in [0,1]. Use >=0.90 only when intent is very clear.
- depends_on: index of a sub-agent whose result this one needs (use [] if independent/parallel)

JSON format:
{{"needs_delegation": false, "complexity": "simple", "sub_agents": [], "reasoning": "brief why", "confidence": 0.90, "should_clarify": false, "clarification_question": null}}

OR for delegation:
{{"needs_delegation": true, "complexity": "complex", "sub_agents": [{{"agent_type": "Researcher", "task": "specific task", "preferred_model_role": null, "depends_on": []}}], "reasoning": "brief why", "confidence": 0.78, "should_clarify": false, "clarification_question": null}}

If should_clarify=true, provide a short concrete question in clarification_question.

Task: {message}"#,
            specialists = specialist_desc,
            router_policy = crate::core::prompt_policy::router_policy_v2_block(),
            policy_hint = policy_hint_block,
            action_hints = action_hint_block,
            preferred_action = preferred_direct_action,
            message = message
        );

        let empty_actions: Vec<crate::actions::ActionDef> = Vec::new();
        let mut router_response: Option<crate::core::llm::LlmResponse> = None;
        let mut router_errors: Vec<String> = Vec::new();
        let timeout_ms = std::env::var("AGENTARK_ROUTER_TIMEOUT_MS")
            .ok()
            .and_then(|v| v.parse::<u64>().ok())
            .filter(|ms| *ms >= 500 && *ms <= 20000)
            .unwrap_or(ROUTER_CALL_TIMEOUT_MS);
        for (idx, candidate) in router_candidates.iter().enumerate() {
            if idx > 0 {
                tracing::warn!(
                    "Routing self-heal: switching router model to {} ({}) after previous failure",
                    candidate.slot_label,
                    candidate.client.model_name()
                );
            }
            let route_call = candidate.client.chat(
                "You are a task router. Follow Router Policy v2. Output only valid JSON. No markdown, no explanation.",
                &routing_prompt,
                &[],
                &empty_actions,
            );
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
                let json_str = if content.starts_with("```") {
                    content
                        .lines()
                        .skip(1)
                        .take_while(|l| !l.starts_with("```"))
                        .collect::<Vec<_>>()
                        .join("\n")
                } else {
                    content.to_string()
                };

                match serde_json::from_str::<crate::core::task_router::RoutingDecision>(&json_str) {
                    Ok(mut decision) => {
                        if !(0.0..=1.0).contains(&decision.confidence) || decision.confidence <= 0.0
                        {
                            decision.confidence = if decision.needs_delegation {
                                0.75
                            } else {
                                0.65
                            };
                        }

                        let preferred_direct_action =
                            crate::core::intent::preferred_direct_action_name(message, actions);
                        if has_execution_intent(message, actions)
                            && preferred_direct_action.is_some()
                        {
                            decision.needs_delegation = false;
                            decision.complexity = QueryComplexity::Simple;
                            decision.sub_agents.clear();
                            decision.reasoning = format!(
                                "{} | Clear direct action match: {}",
                                decision.reasoning,
                                preferred_direct_action.as_deref().unwrap_or("unknown")
                            );
                            decision.confidence = decision.confidence.max(0.90);
                        }

                        if has_execution_intent(message, actions)
                            && decision.needs_delegation
                            && decision.confidence < 0.90
                        {
                            decision.needs_delegation = false;
                            decision.complexity = QueryComplexity::Simple;
                            decision.sub_agents.clear();
                            decision.reasoning = format!(
                                "{} | Execution task routed to direct tool path (confidence below 0.90)",
                                decision.reasoning
                            );
                        }

                        // When the LLM router succeeds, trust its judgment about clarification.
                        // Only boost confidence when keyword heuristics confirm clear intent —
                        // never override the LLM to FORCE clarification via keyword scoring.
                        if has_execution_intent(message, actions) && decision.confidence < 0.90 {
                            let best_score = best_execution_intent_score(message, actions);
                            let ambiguous = is_ambiguous_user_request(message, actions);
                            let detailed_brief = is_detailed_execution_brief(message, actions);
                            let clear_enough = detailed_brief || (!ambiguous && best_score >= 0.55);
                            if clear_enough || best_score >= 0.80 {
                                decision.confidence = decision.confidence.max(0.90);
                                decision.should_clarify = false;
                                decision.clarification_question = None;
                            }
                            // Otherwise: keep the LLM router's original should_clarify value.
                            // Don't override it with keyword heuristics — the LLM understands
                            // semantic intent better than token overlap scoring.
                        }

                        decision
                    }
                    Err(e) => {
                        tracing::warn!(
                            "Failed to parse routing JSON, falling back to structural classifier: {}",
                            e
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

    /// Fallback: structural complexity classification (used when LLM routing fails)
    pub(crate) async fn classify_complexity_fallback(
        &self,
        message: &str,
        actions: &[crate::actions::ActionDef],
    ) -> crate::core::task_router::RoutingDecision {
        let mut complexity = self.classify_complexity(message).await;
        let execution_intent = has_execution_intent(message, actions);
        if execution_intent && matches!(complexity, QueryComplexity::Complex) {
            complexity = QueryComplexity::Medium;
        }

        // Fallback classifier uses keyword heuristics which can't reliably
        // determine if clarification is needed — never ask for clarification here.
        // The processing LLM will ask for clarification itself if truly confused.
        crate::core::task_router::RoutingDecision {
            needs_delegation: matches!(complexity, QueryComplexity::Complex) && !execution_intent,
            complexity,
            sub_agents: vec![],
            reasoning: "Fallback structural classification".to_string(),
            confidence: 0.60,
            should_clarify: false,
            clarification_question: None,
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
