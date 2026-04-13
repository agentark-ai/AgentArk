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

impl Agent {
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
            serde_json::to_string(&payload)
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
