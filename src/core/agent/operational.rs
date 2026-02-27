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
    pub model_slot: Option<&'a str>,
}

impl Agent {
    fn detect_user_correction_signal(message: &str) -> bool {
        let lowered = message.trim().to_ascii_lowercase();
        if lowered.is_empty() {
            return false;
        }
        let patterns = [
            "that's wrong",
            "that is wrong",
            "not what i asked",
            "not what i wanted",
            "you missed",
            "try again",
            "redo",
            "incorrect",
            "no,",
            "no ",
            "instead",
            "don't do that",
            "do not do that",
            "i said",
            "fix this",
        ];
        patterns.iter().any(|p| lowered.contains(p))
    }

    fn sanitize_operational_text(raw: &str, max_chars: usize) -> String {
        let redacted = crate::security::redact_pii(raw);
        safe_truncate(&redacted, max_chars)
    }

    fn normalize_optional_text(raw: Option<&str>, max_chars: usize) -> Option<String> {
        raw.map(|s| s.trim())
            .filter(|s| !s.is_empty())
            .map(|s| Self::sanitize_operational_text(s, max_chars))
    }

    pub(crate) async fn log_operational_event(&self, event: OperationalEvent<'_>) {
        let args_text = event.arguments.and_then(|v| {
            serde_json::to_string(v)
                .ok()
                .map(|s| Self::sanitize_operational_text(&s, 1200))
        });
        let payload_text = event.payload.and_then(|v| {
            serde_json::to_string(v)
                .ok()
                .map(|s| Self::sanitize_operational_text(&s, 2000))
        });
        let model = crate::storage::entities::operational_log::Model {
            id: uuid::Uuid::new_v4().to_string(),
            created_at: chrono::Utc::now().to_rfc3339(),
            trace_id: Self::normalize_optional_text(event.trace_id, 128),
            conversation_id: Self::normalize_optional_text(event.conversation_id, 128),
            channel: Self::sanitize_operational_text(event.channel, 64),
            event_type: Self::sanitize_operational_text(event.event_type, 64),
            success: event.success,
            outcome: Self::sanitize_operational_text(event.outcome, 256),
            tool_name: Self::normalize_optional_text(event.tool_name, 128),
            latency_ms: event.latency_ms.map(|v| v as i64),
            arguments: args_text,
            payload: payload_text,
            strategy_version: Self::normalize_optional_text(event.strategy_version, 128),
            policy_version: Self::normalize_optional_text(event.policy_version, 128),
            prompt_version: Self::normalize_optional_text(event.prompt_version, 128),
            model_slot: Self::normalize_optional_text(event.model_slot, 128),
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

    fn strategy_seed_for_message(message: &str) -> String {
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
    ) -> Option<(String, String, String)> {
        let task_type = crate::core::self_evolve::strategy_runtime::infer_task_type(message);
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
}

pub(super) fn message_looks_like_correction(message: &str) -> bool {
    Agent::detect_user_correction_signal(message)
}
