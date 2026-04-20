use super::*;

pub(crate) struct OperationalEvent<'a> {
    pub event_type: &'a str,
    pub channel: &'a str,
    pub success: bool,
    pub outcome: &'a str,
    pub trace_id: Option<&'a str>,
    pub conversation_id: Option<&'a str>,
    pub tool_name: Option<&'a str>,
    pub latency_ms: Option<u64>,
    pub arguments: Option<&'a serde_json::Value>,
    pub payload: Option<&'a serde_json::Value>,
    pub strategy_version: Option<&'a str>,
    pub policy_version: Option<&'a str>,
    pub prompt_version: Option<&'a str>,
    pub classifier_prompt_version: Option<&'a str>,
    pub specialist_prompt_version: Option<&'a str>,
    pub model_slot: Option<&'a str>,
}

#[derive(Debug, Clone, Default)]
pub(crate) struct HeuristicReflectionPassStats {
    pub runs_examined: usize,
    pub heuristics_created: usize,
    pub heuristics_merged: usize,
    pub skipped: usize,
    pub failed: usize,
}

impl HeuristicReflectionPassStats {
    pub(crate) fn changed(&self) -> bool {
        self.heuristics_created > 0 || self.heuristics_merged > 0
    }

    pub(crate) fn summary(&self) -> String {
        if self.runs_examined == 0 {
            "No consolidated runs were ready for heuristic reflection.".to_string()
        } else if self.changed() {
            format!(
                "Learned {} new heuristics and refreshed {} existing heuristics from {} consolidated run(s).",
                self.heuristics_created, self.heuristics_merged, self.runs_examined
            )
        } else if self.skipped > 0 && self.failed == 0 {
            format!(
                "Reviewed {} consolidated run(s) and skipped {} with no transferable heuristic.",
                self.runs_examined, self.skipped
            )
        } else {
            format!(
                "Reviewed {} consolidated run(s); skipped {}, failed {}.",
                self.runs_examined, self.skipped, self.failed
            )
        }
    }
}

impl Agent {
    fn heuristic_prompt_dedupe_key(raw: &str) -> String {
        raw.chars()
            .map(|ch| {
                if ch.is_ascii_alphanumeric() {
                    ch.to_ascii_lowercase()
                } else {
                    ' '
                }
            })
            .collect::<String>()
            .split_whitespace()
            .collect::<Vec<_>>()
            .join(" ")
    }

    fn heuristic_scope_rank(
        item: &crate::storage::experience_item::Model,
        project_id: Option<&str>,
        conversation_id: Option<&str>,
    ) -> usize {
        if conversation_id.is_some() && item.conversation_id.as_deref() == conversation_id {
            3
        } else if project_id.is_some() && item.project_id.as_deref() == project_id {
            2
        } else if item.project_id.is_none() && item.conversation_id.is_none() {
            1
        } else {
            0
        }
    }

    fn tool_names_from_experience_run(run: &crate::storage::experience_run::Model) -> Vec<String> {
        run.tool_sequence_json
            .as_array()
            .map(|items| {
                items
                    .iter()
                    .filter_map(|item| item.get("tool_name").and_then(|value| value.as_str()))
                    .map(|value| value.to_string())
                    .collect::<Vec<_>>()
            })
            .unwrap_or_default()
    }

    fn build_heuristic_reflection_prompt(run: &crate::storage::experience_run::Model) -> String {
        let request_text = run
            .request_text
            .as_deref()
            .map(|value| safe_truncate(value.trim(), 1200))
            .unwrap_or_else(|| "(missing)".to_string());
        let outcome_summary = run
            .outcome_summary
            .as_deref()
            .map(|value| safe_truncate(value.trim(), 400))
            .unwrap_or_else(|| "(none)".to_string());
        let failure_reason = run
            .failure_reason
            .as_deref()
            .map(|value| safe_truncate(value.trim(), 400))
            .unwrap_or_else(|| "(none)".to_string());
        let decision_episode = run
            .metadata
            .get("decision_episode")
            .and_then(|value| serde_json::to_string_pretty(value).ok())
            .map(|value| safe_truncate(&value, 1800))
            .unwrap_or_else(|| "(none)".to_string());
        let tool_names = Self::tool_names_from_experience_run(run);
        let tool_summary = if tool_names.is_empty() {
            "(none)".to_string()
        } else {
            safe_truncate(&tool_names.join(" -> "), 400)
        };

        format!(
            "Experience run:\n\
id={id}\n\
scope={scope}\n\
intent_key={intent_key}\n\
task_type={task_type}\n\
success_state={success_state}\n\
correction_state={correction_state}\n\
tools={tools}\n\
request_text={request_text}\n\
outcome_summary={outcome_summary}\n\
failure_reason={failure_reason}\n\
\n\
Decision episode:\n{decision_episode}\n\
\n\
Return JSON only with this shape:\n\
{{\"heuristic\":\"short actionable sentence or empty\",\"polarity\":\"positive|negative\",\"confidence\":0.0,\"applicability\":\"short applicability note or empty\",\"skip_reason\":\"short reason or empty\"}}\n\
\n\
Rules:\n\
- Extract one transferable lesson about how to handle semantically similar requests in the future.\n\
- Focus on underlying decision quality, verification order, tool choice, safety posture, or failure avoidance.\n\
- Do not quote the user request or restate the exact trace.\n\
- Do not depend on exact wording from the request.\n\
- Keep the heuristic self-contained and actionable.\n\
- Use polarity=positive for a reusable preferred approach, negative for a caution or verification rule.\n\
- If there is no clear transferable lesson, leave heuristic empty and fill skip_reason.",
            id = run.id,
            scope = run.scope,
            intent_key = run.intent_key,
            task_type = run.task_type.as_deref().unwrap_or("general"),
            success_state = run.success_state,
            correction_state = run.correction_state,
            tools = tool_summary,
            request_text = request_text,
            outcome_summary = outcome_summary,
            failure_reason = failure_reason,
            decision_episode = decision_episode,
        )
    }

    fn parse_reflected_heuristic_payload(
        payload: &serde_json::Value,
    ) -> Result<Option<crate::core::learning::ReflectedHeuristic>, String> {
        let heuristic = payload
            .get("heuristic")
            .and_then(|value| value.as_str())
            .map(str::trim)
            .unwrap_or("");
        let skip_reason = payload
            .get("skip_reason")
            .and_then(|value| value.as_str())
            .map(str::trim)
            .unwrap_or("");
        if heuristic.is_empty() {
            return Ok(None);
        }

        let polarity = payload
            .get("polarity")
            .and_then(|value| value.as_str())
            .map(|value| value.trim().to_ascii_lowercase())
            .unwrap_or_else(|| "positive".to_string());
        if polarity != "positive" && polarity != "negative" {
            return Err(format!("invalid polarity '{}'", polarity));
        }

        let confidence = payload
            .get("confidence")
            .and_then(|value| value.as_f64())
            .unwrap_or(0.68)
            .clamp(0.0, 1.0);
        let applicability = payload
            .get("applicability")
            .and_then(|value| value.as_str())
            .map(str::trim)
            .filter(|value| !value.is_empty())
            .map(|value| safe_truncate(value, 220));
        let heuristic = safe_truncate(heuristic, 260);
        if heuristic.is_empty() {
            if skip_reason.is_empty() {
                return Ok(None);
            }
            return Ok(None);
        }

        Ok(Some(crate::core::learning::ReflectedHeuristic {
            heuristic,
            polarity,
            confidence,
            applicability,
        }))
    }

    fn sanitize_operational_text(raw: &str, max_chars: usize) -> String {
        let result = crate::security::sanitize_model_input_text(
            raw,
            &crate::security::ModelPrivacyConfig::default(),
            crate::security::ModelInputContext::Diagnostic,
            false,
        );
        let redacted = crate::security::render_model_input_fallback(
            &result,
            crate::security::ModelInputContext::Diagnostic,
        );
        safe_truncate(&redacted, max_chars)
    }

    fn normalize_optional_diagnostic_text(raw: Option<&str>, max_chars: usize) -> Option<String> {
        raw.map(|s| s.trim())
            .filter(|s| !s.is_empty())
            .map(|s| Self::sanitize_operational_text(s, max_chars))
    }

    // Internal version labels are structured identifiers, not user-provided
    // free text. Running them through secret redaction can corrupt the labels
    // and make ArkEvolve telemetry unreadable.
    fn normalize_optional_version_label(raw: Option<&str>, max_chars: usize) -> Option<String> {
        raw.map(str::trim)
            .filter(|value| !value.is_empty())
            .map(|value| safe_truncate(value, max_chars))
            .map(|value| value.trim().to_string())
            .filter(|value| !value.is_empty())
    }

    // Operational reference IDs are not secrets. Redacting them can corrupt FK
    // lookups and persistence because UUID-like values can resemble opaque tokens.
    fn normalize_optional_reference_id(raw: Option<&str>, max_chars: usize) -> Option<String> {
        raw.map(str::trim)
            .filter(|value| !value.is_empty())
            .map(|value| safe_truncate(value, max_chars))
            .map(|value| value.trim().to_string())
            .filter(|value| !value.is_empty())
    }

    fn operational_payload_preserves_identifier(key: &str) -> bool {
        matches!(
            key,
            "strategy_version"
                | "policy_version"
                | "prompt_version"
                | "classifier_prompt_version"
                | "specialist_prompt_version"
        )
    }

    fn sanitize_operational_payload_value(
        key: Option<&str>,
        value: &serde_json::Value,
    ) -> serde_json::Value {
        match value {
            serde_json::Value::String(text) => {
                let sanitized = if key.is_some_and(Self::operational_payload_preserves_identifier) {
                    safe_truncate(text.trim(), 128)
                } else {
                    Self::sanitize_operational_text(text, 320)
                };
                serde_json::Value::String(sanitized)
            }
            serde_json::Value::Array(items) => serde_json::Value::Array(
                items
                    .iter()
                    .map(|item| Self::sanitize_operational_payload_value(None, item))
                    .collect(),
            ),
            serde_json::Value::Object(map) => {
                let mut next = serde_json::Map::with_capacity(map.len());
                for (child_key, child_value) in map {
                    next.insert(
                        child_key.clone(),
                        Self::sanitize_operational_payload_value(Some(child_key), child_value),
                    );
                }
                serde_json::Value::Object(next)
            }
            other => other.clone(),
        }
    }

    fn sanitize_operational_payload_json(value: &serde_json::Value) -> Option<String> {
        let sanitized = Self::sanitize_operational_payload_value(None, value);
        serde_json::to_string(&sanitized).ok()
    }

    pub(crate) async fn log_operational_event(&self, event: OperationalEvent<'_>) {
        let args_text = event.arguments.and_then(|v| {
            serde_json::to_string(v)
                .ok()
                .map(|s| Self::sanitize_operational_text(&s, 1200))
        });
        let payload_text = event.payload.and_then(|v| {
            let mut payload = v.clone();
            if event.classifier_prompt_version.is_some()
                || event.specialist_prompt_version.is_some()
            {
                let mut object = match payload {
                    serde_json::Value::Object(obj) => obj,
                    other => {
                        let mut obj = serde_json::Map::new();
                        obj.insert("payload".to_string(), other);
                        obj
                    }
                };
                if let Some(version) = event.classifier_prompt_version {
                    object.insert(
                        "classifier_prompt_version".to_string(),
                        serde_json::Value::String(version.to_string()),
                    );
                }
                if let Some(version) = event.specialist_prompt_version {
                    object.insert(
                        "specialist_prompt_version".to_string(),
                        serde_json::Value::String(version.to_string()),
                    );
                }
                payload = serde_json::Value::Object(object);
            }
            Self::sanitize_operational_payload_json(&payload)
        });
        let model = crate::storage::entities::operational_log::Model {
            id: uuid::Uuid::new_v4().to_string(),
            created_at: chrono::Utc::now().to_rfc3339(),
            trace_id: Self::normalize_optional_reference_id(event.trace_id, 128),
            conversation_id: Self::normalize_optional_reference_id(event.conversation_id, 128),
            channel: Self::sanitize_operational_text(event.channel, 64),
            event_type: Self::sanitize_operational_text(event.event_type, 64),
            success: event.success,
            outcome: Self::sanitize_operational_text(event.outcome, 256),
            tool_name: Self::normalize_optional_diagnostic_text(event.tool_name, 128),
            latency_ms: event.latency_ms.map(|v| v as i64),
            arguments: args_text,
            payload: payload_text,
            strategy_version: Self::normalize_optional_version_label(event.strategy_version, 128),
            policy_version: Self::normalize_optional_version_label(event.policy_version, 128),
            prompt_version: Self::normalize_optional_version_label(event.prompt_version, 128),
            model_slot: Self::normalize_optional_diagnostic_text(event.model_slot, 128),
        };
        if let Err(e) = self.storage.insert_operational_log(&model).await {
            tracing::debug!("Failed to insert operational log: {}", e);
        }
    }

    async fn load_tool_strategy_profile_by_key(
        &self,
        key: &str,
    ) -> Option<crate::core::self_evolve::strategy_runtime::ToolStrategyProfile> {
        let raw = self.storage.get(key).await.ok().flatten()?;
        let value = serde_json::from_slice::<
            crate::core::self_evolve::strategy_runtime::ToolStrategyProfile,
        >(&raw)
        .ok()?;
        Some(value)
    }

    async fn load_prompt_bundle_by_key(
        &self,
        key: &str,
    ) -> Option<crate::core::self_evolve::PromptBundleProfile> {
        let raw = self.storage.get(key).await.ok().flatten()?;
        crate::core::self_evolve::prompt_evolution::parse_prompt_bundle_profile(&raw)
    }

    async fn load_classifier_prompt_bundle_by_key(
        &self,
        key: &str,
    ) -> Option<crate::core::self_evolve::ClassifierPromptBundleProfile> {
        let raw = self.storage.get(key).await.ok().flatten()?;
        crate::core::self_evolve::classifier_prompt_evolution::parse_classifier_prompt_bundle_profile(&raw)
    }

    async fn load_specialist_prompt_bundle_by_key(
        &self,
        key: &str,
    ) -> Option<crate::core::self_evolve::SpecialistPromptBundleProfile> {
        let raw = self.storage.get(key).await.ok().flatten()?;
        crate::core::self_evolve::specialist_prompt_evolution::parse_specialist_prompt_bundle_profile(&raw)
    }

    fn strategy_seed_for_message(message: &str) -> String {
        let normalized = message.trim().to_ascii_lowercase();
        if normalized.is_empty() {
            "_empty".to_string()
        } else {
            normalized
        }
    }

    fn prompt_seed_for_message(message: &str) -> String {
        let normalized = message.trim().to_ascii_lowercase();
        if normalized.is_empty() {
            "_empty".to_string()
        } else {
            normalized
        }
    }

    pub(crate) async fn active_strategy_version_for_message(
        &self,
        message: &str,
    ) -> Option<String> {
        let baseline = self
            .load_tool_strategy_profile_by_key(
                crate::core::self_evolve::strategy_runtime::TOOL_STRATEGY_PROFILE_KEY,
            )
            .await;
        let mut selected = baseline;

        let canary_state_raw = self
            .storage
            .get(crate::core::self_evolve::strategy_runtime::TOOL_STRATEGY_CANARY_STATE_KEY)
            .await
            .ok()
            .flatten();
        if let Some(raw) = canary_state_raw {
            if let Ok(state) = serde_json::from_slice::<
                crate::core::self_evolve::strategy_runtime::CanaryRolloutState,
            >(&raw)
            {
                if state.enabled
                    && crate::core::self_evolve::strategy_runtime::should_use_canary(
                        &Self::strategy_seed_for_message(message),
                        state.rollout_percent,
                    )
                {
                    if let Some(canary) = self
                        .load_tool_strategy_profile_by_key(
                            crate::core::self_evolve::strategy_runtime::TOOL_STRATEGY_PROFILE_CANARY_KEY,
                        )
                        .await
                    {
                        selected = Some(canary);
                    }
                }
            }
        }

        selected.map(|p| p.version)
    }

    pub(crate) async fn build_strategy_prompt_block_for_message(
        &self,
        message: &str,
        request_shape: Option<&crate::core::RequestShapeAssessment>,
        actions: &[crate::actions::ActionDef],
    ) -> Option<(String, String, String)> {
        let task_type =
            crate::core::self_evolve::strategy_runtime::infer_task_type_from_request_context(
                request_shape,
                actions,
            );
        let baseline = self
            .load_tool_strategy_profile_by_key(
                crate::core::self_evolve::strategy_runtime::TOOL_STRATEGY_PROFILE_KEY,
            )
            .await;
        let mut selected = baseline;

        let canary_state_raw = self
            .storage
            .get(crate::core::self_evolve::strategy_runtime::TOOL_STRATEGY_CANARY_STATE_KEY)
            .await
            .ok()
            .flatten();
        if let Some(raw) = canary_state_raw {
            if let Ok(state) = serde_json::from_slice::<
                crate::core::self_evolve::strategy_runtime::CanaryRolloutState,
            >(&raw)
            {
                if state.enabled
                    && crate::core::self_evolve::strategy_runtime::should_use_canary(
                        &Self::strategy_seed_for_message(message),
                        state.rollout_percent,
                    )
                {
                    if let Some(canary) = self
                        .load_tool_strategy_profile_by_key(
                            crate::core::self_evolve::strategy_runtime::TOOL_STRATEGY_PROFILE_CANARY_KEY,
                        )
                        .await
                    {
                        selected = Some(canary);
                    }
                }
            }
        }

        let profile = selected?;
        let strategy_version = profile.version.clone();
        let block = crate::core::self_evolve::strategy_runtime::render_prompt_strategy_block(
            &profile, &task_type,
        )?;
        Some((block, strategy_version, task_type))
    }

    pub(crate) async fn build_heuristic_prompt_block_for_message(
        &self,
        message: &str,
        project_id: Option<&str>,
        conversation_id: Option<&str>,
        request_shape: Option<&crate::core::RequestShapeAssessment>,
        actions: &[crate::actions::ActionDef],
    ) -> Option<(String, usize, String)> {
        let task_type =
            crate::core::self_evolve::strategy_runtime::infer_task_type_from_request_context(
                request_shape,
                actions,
            );
        let task_type_label = task_type.as_str();
        let query_tokens = tokenize_lower(message);
        let mut lessons = self
            .storage
            .list_active_experience_items(&["lesson"], project_id, conversation_id, 32)
            .await
            .unwrap_or_default()
            .into_iter()
            .filter(crate::core::learning::experience_item_is_reflected_heuristic)
            .collect::<Vec<_>>();

        let mut ranked = Vec::new();
        for lesson in lessons.drain(..) {
            let lesson_task_type =
                crate::core::learning::reflected_heuristic_task_type(&lesson).unwrap_or("general");
            if lesson_task_type != task_type_label
                && lesson_task_type != "general"
                && task_type_label != "general"
            {
                continue;
            }

            let applicability =
                crate::core::learning::reflected_heuristic_applicability(&lesson).unwrap_or("");
            let combined = format!(
                "{} {} {} {}",
                lesson.title, lesson.content, applicability, lesson_task_type
            );
            let scope_rank = Self::heuristic_scope_rank(&lesson, project_id, conversation_id);
            if scope_rank == 0 {
                continue;
            }
            let task_rank = if lesson_task_type == task_type_label {
                3
            } else {
                2
            };
            let overlap = keyword_overlap_score(&combined, &query_tokens);
            let confidence = crate::core::learning::reflected_heuristic_confidence(&lesson);
            let score = (scope_rank * 100 + task_rank * 40 + overlap) as f64
                + confidence * 25.0
                + lesson.support_count.max(0) as f64 * 6.0;
            ranked.push((lesson, score));
        }
        ranked.sort_by(|left, right| {
            right
                .1
                .total_cmp(&left.1)
                .then_with(|| right.0.support_count.cmp(&left.0.support_count))
                .then_with(|| right.0.updated_at.cmp(&left.0.updated_at))
        });

        let mut seen = std::collections::HashSet::new();
        let mut lines = Vec::new();
        let mut used = 0usize;
        for (lesson, _) in ranked {
            let dedupe_key = Self::heuristic_prompt_dedupe_key(&lesson.content);
            if !seen.insert(dedupe_key) {
                continue;
            }
            let applicability =
                crate::core::learning::reflected_heuristic_applicability(&lesson).unwrap_or("");
            let prefix = match crate::core::learning::reflected_heuristic_polarity(&lesson) {
                Some("negative") => "[Avoid]",
                _ => "[Prefer]",
            };
            if applicability.is_empty() {
                lines.push(format!(
                    "- {} {}",
                    prefix,
                    safe_truncate(&lesson.content, 220)
                ));
            } else {
                lines.push(format!(
                    "- {} {} ({})",
                    prefix,
                    safe_truncate(&lesson.content, 180),
                    safe_truncate(applicability, 120)
                ));
            }
            used += 1;
            if used >= 4 {
                break;
            }
        }
        if lines.is_empty() {
            return None;
        }

        Some((
            format!(
                "## Learned Heuristics\n- Task type: {}\n{}",
                task_type_label,
                lines.join("\n")
            ),
            used,
            task_type,
        ))
    }

    pub(crate) async fn run_heuristic_reflection_pass(
        &self,
    ) -> Result<HeuristicReflectionPassStats> {
        if !crate::core::learning::load_learning_enabled(&self.storage).await {
            return Ok(HeuristicReflectionPassStats::default());
        }
        let cap = crate::core::learning::load_learning_queue_cap(&self.storage).await as u64;
        let runs = self
            .storage
            .list_experience_runs_for_heuristic_reflection(cap)
            .await?;
        if runs.is_empty() {
            return Ok(HeuristicReflectionPassStats::default());
        }

        let learning_candidates = self.learning_llm_candidates().await;
        if learning_candidates.is_empty() {
            anyhow::bail!("no_learning_model");
        }

        let mut stats = HeuristicReflectionPassStats {
            runs_examined: runs.len(),
            ..Default::default()
        };
        for run in runs {
            self.storage
                .mark_experience_run_heuristic_reflection_started(&run.id)
                .await?;

            let prompt = Self::build_heuristic_reflection_prompt(&run);
            let Some(resp) = self
                .supervised_internal_chat(
                    "autonomy",
                    "heuristic_reflection",
                    "heuristic_reflection",
                    &ModelRole::Fast,
                    learning_candidates.clone(),
                    "You extract one transferable heuristic from a completed AgentArk experience run. Return strict JSON only.",
                    &prompt,
                    &[],
                    &[],
                    2500,
                    2,
                )
                .await
            else {
                stats.failed = stats.failed.saturating_add(1);
                self.storage
                    .mark_experience_run_heuristic_reflection_failed(
                        &run.id,
                        "learning_model_exhausted",
                    )
                    .await?;
                continue;
            };

            let Some(payload) = extract_json_object_from_text(&resp.content) else {
                stats.failed = stats.failed.saturating_add(1);
                self.storage
                    .mark_experience_run_heuristic_reflection_failed(
                        &run.id,
                        "invalid_reflection_payload",
                    )
                    .await?;
                continue;
            };

            match Self::parse_reflected_heuristic_payload(&payload) {
                Ok(Some(heuristic)) => {
                    let outcome = crate::core::learning::upsert_reflected_heuristic_lesson(
                        &self.storage,
                        &run,
                        &heuristic,
                    )
                    .await?;
                    self.storage
                        .mark_experience_run_heuristic_reflection_completed(
                            &run.id,
                            &outcome.lesson_id,
                        )
                        .await?;
                    if outcome.merged {
                        stats.heuristics_merged = stats.heuristics_merged.saturating_add(1);
                    } else {
                        stats.heuristics_created = stats.heuristics_created.saturating_add(1);
                    }
                }
                Ok(None) => {
                    let reason = payload
                        .get("skip_reason")
                        .and_then(|value| value.as_str())
                        .map(str::trim)
                        .filter(|value| !value.is_empty())
                        .unwrap_or("no_transferable_heuristic");
                    self.storage
                        .mark_experience_run_heuristic_reflection_skipped(&run.id, reason)
                        .await?;
                    stats.skipped = stats.skipped.saturating_add(1);
                }
                Err(error) => {
                    self.storage
                        .mark_experience_run_heuristic_reflection_failed(&run.id, &error)
                        .await?;
                    stats.failed = stats.failed.saturating_add(1);
                }
            }
        }

        Ok(stats)
    }

    pub(crate) async fn active_prompt_bundle_for_message(
        &self,
        message: &str,
    ) -> crate::core::self_evolve::PromptBundleProfile {
        let mut selected = self
            .load_prompt_bundle_by_key(crate::core::self_evolve::PROMPT_BUNDLE_PROFILE_KEY)
            .await
            .unwrap_or_default();

        let canary_state_raw = self
            .storage
            .get(crate::core::self_evolve::PROMPT_BUNDLE_CANARY_STATE_KEY)
            .await
            .ok()
            .flatten();
        if let Some(raw) = canary_state_raw {
            if let Ok(state) = serde_json::from_slice::<
                crate::core::self_evolve::strategy_runtime::CanaryRolloutState,
            >(&raw)
            {
                if state.enabled
                    && crate::core::self_evolve::strategy_runtime::should_use_canary(
                        &Self::prompt_seed_for_message(message),
                        state.rollout_percent,
                    )
                {
                    if let Some(canary) = self
                        .load_prompt_bundle_by_key(
                            crate::core::self_evolve::PROMPT_BUNDLE_PROFILE_CANARY_KEY,
                        )
                        .await
                    {
                        selected = canary;
                    }
                }
            }
        }

        selected
    }

    pub(crate) async fn active_classifier_prompt_bundle_for_message(
        &self,
        message: &str,
    ) -> crate::core::self_evolve::ClassifierPromptBundleProfile {
        let mut selected = self
            .load_classifier_prompt_bundle_by_key(
                crate::core::self_evolve::CLASSIFIER_PROMPT_BUNDLE_PROFILE_KEY,
            )
            .await
            .unwrap_or_default();

        let canary_state_raw = self
            .storage
            .get(crate::core::self_evolve::CLASSIFIER_PROMPT_BUNDLE_CANARY_STATE_KEY)
            .await
            .ok()
            .flatten();
        if let Some(raw) = canary_state_raw {
            if let Ok(state) = serde_json::from_slice::<
                crate::core::self_evolve::strategy_runtime::CanaryRolloutState,
            >(&raw)
            {
                if state.enabled
                    && crate::core::self_evolve::strategy_runtime::should_use_canary(
                        &Self::prompt_seed_for_message(message),
                        state.rollout_percent,
                    )
                {
                    if let Some(canary) = self
                        .load_classifier_prompt_bundle_by_key(
                            crate::core::self_evolve::CLASSIFIER_PROMPT_BUNDLE_PROFILE_CANARY_KEY,
                        )
                        .await
                    {
                        selected = canary;
                    }
                }
            }
        }

        selected
    }

    pub(crate) async fn active_specialist_prompt_bundle_for_message(
        &self,
        message: &str,
    ) -> crate::core::self_evolve::SpecialistPromptBundleProfile {
        let mut selected = self
            .load_specialist_prompt_bundle_by_key(
                crate::core::self_evolve::SPECIALIST_PROMPT_BUNDLE_PROFILE_KEY,
            )
            .await
            .unwrap_or_default();

        let canary_state_raw = self
            .storage
            .get(crate::core::self_evolve::SPECIALIST_PROMPT_BUNDLE_CANARY_STATE_KEY)
            .await
            .ok()
            .flatten();
        if let Some(raw) = canary_state_raw {
            if let Ok(state) = serde_json::from_slice::<
                crate::core::self_evolve::strategy_runtime::CanaryRolloutState,
            >(&raw)
            {
                if state.enabled
                    && crate::core::self_evolve::strategy_runtime::should_use_canary(
                        &Self::prompt_seed_for_message(message),
                        state.rollout_percent,
                    )
                {
                    if let Some(canary) = self
                        .load_specialist_prompt_bundle_by_key(
                            crate::core::self_evolve::SPECIALIST_PROMPT_BUNDLE_PROFILE_CANARY_KEY,
                        )
                        .await
                    {
                        selected = canary;
                    }
                }
            }
        }

        selected
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn operational_reference_ids_are_not_secret_redacted() {
        let trace_id = "cf45b01b-799d-40f3-9aa5-e91bc4831dae";
        assert_eq!(
            Agent::normalize_optional_reference_id(Some(trace_id), 128).as_deref(),
            Some(trace_id)
        );
    }

    #[test]
    fn operational_diagnostic_text_still_redacts_secret_like_values() {
        let fake_key = ["sk", "-1234567890", "abcdefghijklmnop"].concat();
        let sanitized = Agent::sanitize_operational_text(&format!("api_key={fake_key}"), 128);
        assert!(sanitized.contains("[REDACTED_API_KEY]"));
        assert!(!sanitized.contains(&fake_key));
    }

    #[test]
    fn operational_version_labels_are_not_secret_redacted() {
        let version = "system_prompt_v2+prompt-bundle-default-v1";
        assert_eq!(
            Agent::normalize_optional_version_label(Some(version), 128).as_deref(),
            Some(version)
        );
    }

    #[test]
    fn operational_payload_preserves_versions_but_redacts_secret_like_text() {
        let fake_key = ["sk", "-1234567890", "abcdefghijklmnop"].concat();
        let payload = serde_json::json!({
            "classifier_prompt_version": "classifier_prompt_v1+classifier-prompt-bundle-default-v1",
            "specialist_prompt_version": "specialist_prompt_v1+specialist-prompt-bundle-default-v1",
            "diagnostic": format!("api_key={fake_key}")
        });
        let sanitized = Agent::sanitize_operational_payload_json(&payload).expect("payload");

        assert!(sanitized.contains("classifier_prompt_v1+classifier-prompt-bundle-default-v1"));
        assert!(sanitized.contains("specialist_prompt_v1+specialist-prompt-bundle-default-v1"));
        assert!(sanitized.contains("[REDACTED_API_KEY]"));
        assert!(!sanitized.contains(&fake_key));
    }
}
