use super::*;

fn diversify_model_attempt_providers(
    candidates: Vec<LlmAttemptCandidate>,
) -> Vec<LlmAttemptCandidate> {
    if candidates.len() <= 2 {
        return candidates;
    }
    let mut first_provider_attempts = Vec::new();
    let mut later_same_provider_attempts = Vec::new();
    let mut seen_providers = HashSet::new();
    for candidate in candidates {
        if seen_providers.insert(candidate.client.provider_name().to_string()) {
            first_provider_attempts.push(candidate);
        } else {
            later_same_provider_attempts.push(candidate);
        }
    }
    first_provider_attempts.extend(later_same_provider_attempts);
    first_provider_attempts
}

impl Agent {
    pub(super) fn model_role_label(role: &ModelRole) -> &'static str {
        match role {
            ModelRole::Primary => "Primary",
            ModelRole::Fast => "Fast",
            ModelRole::Code => "Code",
            ModelRole::Research => "Research",
            ModelRole::Fallback => "Fallback",
        }
    }

    pub(super) fn provider_model_name(provider: &crate::core::LlmProvider) -> &str {
        match provider {
            crate::core::LlmProvider::Anthropic { model, .. }
            | crate::core::LlmProvider::OpenAI { model, .. }
            | crate::core::LlmProvider::Ollama { model, .. } => model.as_str(),
        }
    }

    pub(super) fn model_aliases_for_slot(slot: &ModelSlot, client: &LlmClient) -> Vec<String> {
        let mut aliases = Vec::new();
        let mut push = |value: &str| {
            let trimmed = value.trim();
            if trimmed.is_empty() {
                return;
            }
            if !aliases
                .iter()
                .any(|existing: &String| existing.eq_ignore_ascii_case(trimmed))
            {
                aliases.push(trimmed.to_string());
            }
        };

        push(slot.id.as_str());
        push(slot.label.as_str());
        push(client.model_name());
        push(Self::provider_model_name(&slot.provider));
        aliases
    }

    pub(super) fn model_alias_match_score(alias: &str, hint_norm: &str, hint_compact: &str) -> i32 {
        let alias_norm = normalize_model_match_token(alias);
        if alias_norm.is_empty() || hint_norm.is_empty() {
            return 0;
        }
        if alias_norm == hint_norm {
            return 120;
        }

        let alias_compact = compact_model_match_token(alias);
        if !hint_compact.is_empty() && alias_compact == hint_compact {
            return 110;
        }

        let long_enough = hint_norm.len() >= 4 && alias_norm.len() >= 4;
        if long_enough && (alias_norm.starts_with(hint_norm) || hint_norm.starts_with(&alias_norm))
        {
            return 90;
        }
        if hint_norm.len() >= 5
            && (alias_norm.contains(hint_norm) || hint_norm.contains(&alias_norm))
        {
            return 70;
        }
        0
    }

    pub(super) fn llm_candidate_from_slot_id(&self, slot_id: &str) -> Option<LlmAttemptCandidate> {
        let (slot, client) = self.model_pool.get(slot_id)?;
        if !slot.enabled || !Self::provider_has_runtime_credentials(&slot.provider) {
            return None;
        }
        Some(LlmAttemptCandidate {
            slot_id: slot.id.clone(),
            slot_label: if slot.label.trim().is_empty() {
                format!("{} slot", Self::model_role_label(&slot.role))
            } else {
                slot.label.clone()
            },
            role: slot.role.clone(),
            client: client.clone(),
        })
    }

    pub(super) fn user_selected_model_slot_id(&self) -> Option<String> {
        self.user_selected_model_slot_id
            .read()
            .ok()
            .and_then(|guard| guard.clone())
    }

    pub(super) fn user_selected_llm_candidate(&self) -> Option<LlmAttemptCandidate> {
        let slot_id = self.user_selected_model_slot_id()?;
        self.llm_candidate_from_slot_id(&slot_id)
    }

    pub(super) async fn learning_llm_candidates(&self) -> Vec<LlmAttemptCandidate> {
        let learning_model_hint =
            crate::core::knowledge::learning::load_learning_model_slot(&self.storage).await;
        let mut out: Vec<LlmAttemptCandidate> = Vec::new();
        let mut seen: HashSet<String> = HashSet::new();
        let mut push = |candidate: LlmAttemptCandidate| {
            if seen.insert(candidate.slot_id.clone()) {
                out.push(candidate);
            }
        };

        if let Some(hint) = learning_model_hint.as_deref() {
            if let Some(candidate) = self.resolve_model_hint_candidate(hint) {
                push(candidate);
            }
        }
        if let Some(candidate) = self.user_selected_llm_candidate() {
            push(candidate);
        }
        for candidate in self.llm_candidates_for_role(&ModelRole::Fast) {
            push(candidate);
        }

        out
    }

    pub(super) fn resolve_model_hint_candidate(&self, hint: &str) -> Option<LlmAttemptCandidate> {
        let hint_norm = normalize_model_match_token(hint);
        let hint_compact = compact_model_match_token(hint);
        if hint_norm.is_empty() {
            return None;
        }

        let mut best: Option<(i32, LlmAttemptCandidate)> = None;
        for slot in &self.config.model_pool.slots {
            let Some((runtime_slot, client)) = self.model_pool.get(&slot.id) else {
                continue;
            };
            if !runtime_slot.enabled
                || !Self::provider_has_runtime_credentials(&runtime_slot.provider)
            {
                continue;
            }

            let aliases = Self::model_aliases_for_slot(runtime_slot, client);
            let score = aliases
                .iter()
                .map(|alias| Self::model_alias_match_score(alias, &hint_norm, &hint_compact))
                .max()
                .unwrap_or(0);
            if score <= 0 {
                continue;
            }

            let candidate = LlmAttemptCandidate {
                slot_id: runtime_slot.id.clone(),
                slot_label: if runtime_slot.label.trim().is_empty() {
                    format!("{} slot", Self::model_role_label(&runtime_slot.role))
                } else {
                    runtime_slot.label.clone()
                },
                role: runtime_slot.role.clone(),
                client: client.clone(),
            };

            if let Some((best_score, _)) = best.as_ref() {
                if score <= *best_score {
                    continue;
                }
            }
            best = Some((score, candidate));
        }

        best.map(|(_, candidate)| candidate)
    }

    pub(crate) fn select_llm_for_app_proxy(
        &self,
        requested_model_hint: Option<&str>,
    ) -> (LlmClient, String, Option<String>) {
        let requested_model_hint = requested_model_hint
            .map(str::trim)
            .filter(|s| !s.is_empty());

        let mut warning: Option<String> = None;
        if let Some(hint) = requested_model_hint {
            if let Some(candidate) = self.resolve_model_hint_candidate(hint) {
                return (
                    candidate.client,
                    candidate.slot_label,
                    Some(format!("requested model '{}'", hint)),
                );
            }
            warning = Some(format!(
                "Requested model '{}' is not configured. Using default configured model.",
                hint
            ));
        }

        if let Some(candidate) = self.user_selected_llm_candidate() {
            return (
                candidate.client,
                candidate.slot_label,
                Some("user-selected model override".to_string()),
            );
        }

        (
            self.llm_for_role(&ModelRole::Primary).clone(),
            Self::model_role_label(&ModelRole::Primary).to_string(),
            warning,
        )
    }

    pub(super) fn llm_candidates_for_role(
        &self,
        preferred_role: &ModelRole,
    ) -> Vec<LlmAttemptCandidate> {
        llm_attempt_candidates_for_role(
            &self.config,
            &self.model_pool,
            &self.primary_model_id,
            self.user_selected_model_slot_id().as_deref(),
            &self.llm,
            preferred_role,
        )
    }

    pub(super) fn primary_llm_candidate(&self) -> LlmAttemptCandidate {
        LlmAttemptCandidate {
            slot_id: self.primary_model_id.clone(),
            slot_label: "Primary".to_string(),
            role: ModelRole::Primary,
            client: self.llm.clone(),
        }
    }

    #[allow(clippy::too_many_arguments)]
    pub(super) async fn supervised_internal_chat(
        &self,
        channel: &str,
        usage_label: &str,
        request_kind: &str,
        preferred_role: &ModelRole,
        candidates: Vec<LlmAttemptCandidate>,
        system_prompt: &str,
        user_message: &str,
        memories: &[PromptMemory],
        actions: &[crate::actions::ActionDef],
        timeout_ms: u64,
        max_candidates: usize,
    ) -> Option<super::llm::LlmResponse> {
        self.supervised_internal_chat_detailed(
            channel,
            usage_label,
            request_kind,
            preferred_role,
            candidates,
            system_prompt,
            user_message,
            memories,
            actions,
            timeout_ms,
            max_candidates,
        )
        .await
        .ok()
    }

    #[allow(clippy::too_many_arguments)]
    pub(super) async fn supervised_internal_chat_detailed(
        &self,
        channel: &str,
        usage_label: &str,
        request_kind: &str,
        preferred_role: &ModelRole,
        candidates: Vec<LlmAttemptCandidate>,
        system_prompt: &str,
        user_message: &str,
        memories: &[PromptMemory],
        actions: &[crate::actions::ActionDef],
        timeout_ms: u64,
        max_candidates: usize,
    ) -> Result<super::llm::LlmResponse, crate::core::UserFacingOutcome> {
        self.supervised_internal_chat_detailed_with_stream(
            channel,
            usage_label,
            request_kind,
            preferred_role,
            candidates,
            system_prompt,
            user_message,
            memories,
            actions,
            timeout_ms,
            max_candidates,
            None,
            false,
        )
        .await
    }

    #[allow(clippy::too_many_arguments)]
    pub(super) async fn supervised_internal_chat_detailed_with_stream(
        &self,
        channel: &str,
        usage_label: &str,
        request_kind: &str,
        preferred_role: &ModelRole,
        mut candidates: Vec<LlmAttemptCandidate>,
        system_prompt: &str,
        user_message: &str,
        memories: &[PromptMemory],
        actions: &[crate::actions::ActionDef],
        timeout_ms: u64,
        max_candidates: usize,
        stream_tx: Option<tokio::sync::mpsc::Sender<StreamEvent>>,
        long_running_stream: bool,
    ) -> Result<super::llm::LlmResponse, crate::core::UserFacingOutcome> {
        if candidates.is_empty() {
            candidates = self.llm_candidates_for_role(preferred_role);
        }
        if candidates.is_empty() {
            candidates.push(self.primary_llm_candidate());
        }
        let candidates = self
            .reorder_candidates_with_failover(candidates, None)
            .await;
        let effective_role = effective_model_role_for_selection(&self.config, preferred_role);
        let request = super::ExecutionRequest {
            kind: request_kind.to_string(),
            channel: Some(channel.to_string()),
            preferred_model_role: Some(Self::model_role_label(&effective_role).to_string()),
            message_preview: Some(safe_truncate(user_message, 200)),
            ..Default::default()
        };
        let mut attempted_models = Vec::new();
        let mut attempt_records = Vec::new();
        tracing::debug!(
            target: "agentark.turn_timing",
            channel = %channel,
            usage_label = %usage_label,
            request_kind = %request_kind,
            preferred_role = ?preferred_role,
            candidate_count = candidates.len(),
            max_candidates = max_candidates.max(1),
            timeout_ms,
            stream = stream_tx.is_some(),
            prompt_chars = system_prompt.chars().count().saturating_add(user_message.chars().count()),
            action_count = actions.len(),
            "supervised model request start"
        );

        for (idx, candidate) in candidates.iter().take(max_candidates.max(1)).enumerate() {
            let started = std::time::Instant::now();
            tracing::debug!(
                target: "agentark.turn_timing",
                channel = %channel,
                usage_label = %usage_label,
                request_kind = %request_kind,
                candidate_index = idx,
                slot_id = %candidate.slot_id,
                slot_label = %candidate.slot_label,
                model = %candidate.client.model_name(),
                provider = %candidate.client.provider_name(),
                stream = stream_tx.is_some(),
                timeout_ms,
                "supervised model candidate start"
            );
            let request_timeout_ms = (timeout_ms > 0).then_some(timeout_ms);
            let result = if let Some(token_tx) = stream_tx.clone() {
                crate::core::orchestration::execution::execute_supervised_transport_chat_stream_with_policy(
                    &self.execution_supervisor,
                    &candidate.client,
                    &request,
                    system_prompt,
                    user_message,
                    memories,
                    actions,
                    if long_running_stream {
                        None
                    } else {
                        request_timeout_ms
                    },
                    token_tx,
                    long_running_stream,
                    &self.config.model_privacy,
                    false,
                )
                .await
            } else {
                crate::core::orchestration::execution::execute_supervised_transport_chat_with_policy(
                    &self.execution_supervisor,
                    &candidate.client,
                    &request,
                    system_prompt,
                    user_message,
                    memories,
                    actions,
                    request_timeout_ms,
                    &self.config.model_privacy,
                    false,
                )
                .await
            };

            match result {
                Ok(resp) => {
                    let duration_ms = started.elapsed().as_millis() as u64;
                    tracing::debug!(
                        target: "agentark.turn_timing",
                        channel = %channel,
                        usage_label = %usage_label,
                        request_kind = %request_kind,
                        candidate_index = idx,
                        slot_id = %candidate.slot_id,
                        slot_label = %candidate.slot_label,
                        model = %candidate.client.model_name(),
                        provider = %candidate.client.provider_name(),
                        stream = stream_tx.is_some(),
                        duration_ms,
                        success = true,
                        response_chars = resp.content.chars().count(),
                        tool_calls = resp.tool_calls.len(),
                        "supervised model candidate complete"
                    );
                    self.record_llm_usage(channel, usage_label, &resp).await;
                    self.record_model_attempt(
                        &mut attempted_models,
                        &mut attempt_records,
                        candidate,
                        true,
                        None,
                        idx > 0,
                        started.elapsed().as_millis() as u64,
                        None,
                    )
                    .await;
                    return Ok(resp);
                }
                Err(error) => {
                    let duration_ms = started.elapsed().as_millis() as u64;
                    let error_text = error.to_string();
                    tracing::debug!(
                        target: "agentark.turn_timing",
                        channel = %channel,
                        usage_label = %usage_label,
                        request_kind = %request_kind,
                        candidate_index = idx,
                        slot_id = %candidate.slot_id,
                        slot_label = %candidate.slot_label,
                        model = %candidate.client.model_name(),
                        provider = %candidate.client.provider_name(),
                        stream = stream_tx.is_some(),
                        duration_ms,
                        success = false,
                        error = %safe_truncate(&error_text, 320),
                        "supervised model candidate failed"
                    );
                    self.record_model_attempt(
                        &mut attempted_models,
                        &mut attempt_records,
                        candidate,
                        false,
                        Some(&error_text),
                        idx > 0,
                        started.elapsed().as_millis() as u64,
                        None,
                    )
                    .await;
                }
            }
        }

        let mut failure_outcome =
            self.execution_supervisor
                .build_failure_outcome(&request, &attempt_records, &[]);
        enrich_supervisor_outcome_with_model_failures(&mut failure_outcome.user_outcome);
        tracing::warn!(
            "Internal supervised request '{}' exhausted its eligible model chain: {}",
            request_kind,
            failure_outcome.user_outcome.message
        );
        Err(failure_outcome.user_outcome)
    }

    pub(super) fn execution_candidate_descriptor(
        &self,
        candidate: &LlmAttemptCandidate,
        original_index: usize,
    ) -> super::ExecutionCandidateDescriptor {
        let slot = self
            .config
            .model_pool
            .slots
            .iter()
            .find(|slot| slot.id == candidate.slot_id);
        super::ExecutionCandidateDescriptor {
            slot_id: candidate.slot_id.clone(),
            provider_id: Some(candidate.client.provider_name().to_string()),
            capability_tier: slot
                .map(|slot| slot.capability_tier)
                .unwrap_or(super::ModelCapabilityTier::Balanced),
            cost_tier: slot
                .map(|slot| slot.cost_tier)
                .unwrap_or(super::ModelCostTier::Medium),
            auto_escalate: slot.map(|slot| slot.auto_escalate).unwrap_or(true),
            escalation_rank: slot.map(|slot| slot.escalation_rank).unwrap_or(0),
            is_user_selected: self
                .user_selected_model_slot_id()
                .as_deref()
                .is_some_and(|value| value == candidate.slot_id),
            is_primary: candidate.slot_id == self.primary_model_id,
            original_index,
        }
    }

    pub(super) fn build_response_heuristic_outcome(
        &self,
        response: &str,
        degradation: &[crate::core::DegradationNote],
        attempts: &[crate::core::AttemptRecord],
        tool_batch: Option<&tool_execution::ToolExecutionBatch>,
    ) -> Option<crate::core::UserFacingOutcome> {
        let has_tool_evidence = tool_batch
            .map(|batch| !batch.outputs.is_empty() || !batch.outcomes.is_empty())
            .unwrap_or(false);
        let tool_batch_requires_permission = tool_batch
            .map(tool_batch_indicates_permission_requirement)
            .unwrap_or(false);
        if tool_batch_requires_permission
            || (!has_tool_evidence && response_indicates_permission_requirement(response))
        {
            return Some(self.execution_supervisor.build_permission_outcome(
                response,
                degradation,
                attempts,
            ));
        }

        let tool_batch_requires_credentials = tool_batch
            .map(tool_batch_indicates_credentials_requirement)
            .unwrap_or(false);
        if tool_batch_requires_credentials
            || (!has_tool_evidence && response_indicates_credentials_requirement(response))
        {
            return Some(self.execution_supervisor.build_credentials_outcome(
                response,
                degradation,
                attempts,
            ));
        }

        let tool_batch_requires_integration = tool_batch
            .map(tool_batch_indicates_integration_requirement)
            .unwrap_or(false);
        if tool_batch_requires_integration
            || (!has_tool_evidence && response_indicates_integration_requirement(response))
        {
            return Some(self.execution_supervisor.build_integration_outcome(
                response,
                degradation,
                attempts,
            ));
        }

        if response_indicates_pending_execution(response)
            && !tool_batch
                .map(tool_batch_has_successful_persistent_artifact)
                .unwrap_or(false)
        {
            let mut pending_degradation = degradation.to_vec();
            pending_degradation.push(crate::core::DegradationNote {
                kind: "execution_incomplete".to_string(),
                summary: "assistant stopped before executing promised next step".to_string(),
                detail: Some(
                    "The final response still described future execution instead of reporting a completed result."
                        .to_string(),
                ),
            });
            return Some(self.execution_supervisor.build_success_outcome(
                response,
                &pending_degradation,
                attempts,
            ));
        }

        None
    }

    pub(super) fn execution_run_status_for_outcome(
        outcome: &super::UserFacingOutcome,
    ) -> super::ExecutionRunStatus {
        match outcome.status {
            super::UserFacingOutcomeStatus::Complete => super::ExecutionRunStatus::Completed,
            super::UserFacingOutcomeStatus::Degraded
            | super::UserFacingOutcomeStatus::ServiceUnavailable => {
                super::ExecutionRunStatus::Degraded
            }
            super::UserFacingOutcomeStatus::NeedsClarification => {
                super::ExecutionRunStatus::NeedsInput
            }
            super::UserFacingOutcomeStatus::NeedsPermission
            | super::UserFacingOutcomeStatus::NeedsIntegration
            | super::UserFacingOutcomeStatus::NeedsCredentials => {
                super::ExecutionRunStatus::Blocked
            }
            super::UserFacingOutcomeStatus::NeedsStrongerModel => {
                super::ExecutionRunStatus::NeedsStrongerModel
            }
        }
    }

    #[allow(clippy::too_many_arguments)]
    pub(super) async fn record_model_attempt(
        &self,
        attempted_models: &mut Vec<crate::core::ModelAttemptRecord>,
        attempt_records: &mut Vec<crate::core::AttemptRecord>,
        candidate: &LlmAttemptCandidate,
        success: bool,
        error: Option<&str>,
        auto_escalated: bool,
        elapsed_ms: u64,
        session_id: Option<&str>,
    ) {
        let failure_kind = error.map(|value| self.execution_supervisor.classify_failure(value));
        let recovery_action = if success {
            super::RecoveryAction::None
        } else {
            self.execution_supervisor
                .recovery_action_for_failure(failure_kind.as_ref(), auto_escalated)
        };
        let attempt = crate::core::AttemptRecord {
            slot_id: candidate.slot_id.clone(),
            slot_label: candidate.slot_label.clone(),
            model_name: candidate.client.model_name().to_string(),
            provider_id: Some(candidate.client.provider_name().to_string()),
            success,
            attempted_at: chrono::Utc::now().to_rfc3339(),
            failure_kind: failure_kind.clone(),
            recovery_action: recovery_action.clone(),
            auto_escalated,
            elapsed_ms: Some(elapsed_ms),
            error: error.map(|value| safe_truncate(value, 240)),
        };
        attempted_models.push((&attempt).into());
        attempt_records.push(attempt.clone());

        let provider_event = super::ProviderHealthEvent {
            provider_id: candidate.client.provider_name().to_string(),
            provider_kind: Some(candidate.client.provider_name().to_string()),
            success,
            error: attempt.error.clone(),
            cooldown_secs: if success {
                None
            } else {
                self.execution_supervisor
                    .cooldown_secs_for_failure(failure_kind.as_ref())
            },
            disabled: None,
            auth_profile_id: None,
            model_id: Some(candidate.client.model_name().to_string()),
            session_id: session_id.map(|value| value.to_string()),
            note: if success {
                Some("runtime_attempt_succeeded".to_string())
            } else {
                Some(format!("runtime_attempt_failed:{:?}", failure_kind))
            },
            metadata: Some(serde_json::json!({
                "slot_id": candidate.slot_id,
                "slot_label": candidate.slot_label,
                "auto_escalated": auto_escalated,
                "elapsed_ms": elapsed_ms,
                "recovery_action": recovery_action,
            })),
        };
        if let Err(error) =
            super::ModelFailoverControlPlane::record_health(&self.storage, provider_event).await
        {
            tracing::debug!(
                "Failed to update model health after runtime attempt: {}",
                error
            );
        }
    }

    pub(super) async fn reorder_candidates_with_failover(
        &self,
        candidates: Vec<LlmAttemptCandidate>,
        session_id: Option<&str>,
    ) -> Vec<LlmAttemptCandidate> {
        if candidates.len() <= 1 {
            return candidates;
        }

        let selection = match super::ModelFailoverControlPlane::select_candidate(
            &self.storage,
            super::ModelFailoverSelectionRequest {
                session_id: session_id.map(|value| value.to_string()),
                ..Default::default()
            },
        )
        .await
        {
            Ok(selection) if !selection.blocked => selection,
            _ => return diversify_model_attempt_providers(candidates),
        };

        let preferred_provider = selection.selected_provider_id.as_deref();
        let mut ranked = candidates
            .into_iter()
            .enumerate()
            .map(|(idx, candidate)| {
                let descriptor = self.execution_candidate_descriptor(&candidate, idx);
                (descriptor, candidate)
            })
            .collect::<Vec<_>>();
        ranked.sort_by_key(|(descriptor, _)| {
            self.execution_supervisor
                .candidate_rank(descriptor, preferred_provider)
        });

        let mut ordered = Vec::new();
        for (idx, (descriptor, candidate)) in ranked.into_iter().enumerate() {
            if idx == 0 || descriptor.auto_escalate {
                ordered.push(candidate);
            }
        }

        if ordered.is_empty() {
            vec![]
        } else {
            diversify_model_attempt_providers(ordered)
        }
    }

    /// Get LlmClient for a specific role (falls back to primary)
    pub fn llm_for_role(&self, role: &ModelRole) -> &LlmClient {
        for slot_id in ordered_model_slot_ids_for_role(
            &self.config,
            &self.primary_model_id,
            self.user_selected_model_slot_id().as_deref(),
            role,
        ) {
            if let Some(client) = self.ready_slot_client(&slot_id) {
                return client;
            }
        }

        &self.llm
    }

    /// Merge model-backed app env vars across configured providers.
    /// Prioritizes user-selected slot, then primary, then base llm, then fallback/other enabled slots.
    pub fn app_model_env_vars(&self) -> std::collections::HashMap<String, String> {
        let mut provider_refs: Vec<&crate::core::LlmProvider> = Vec::new();
        let selected_slot_id = self.user_selected_model_slot_id();

        if let Some(selected_slot) =
            self.config.model_pool.slots.iter().find(|slot| {
                selected_slot_id.as_ref().is_some_and(|id| id == &slot.id) && slot.enabled
            })
        {
            provider_refs.push(&selected_slot.provider);
        }
        if let Some(primary_slot) = self
            .config
            .model_pool
            .slots
            .iter()
            .find(|slot| slot.id == self.primary_model_id && slot.enabled)
        {
            provider_refs.push(&primary_slot.provider);
        }
        provider_refs.push(&self.config.llm);
        if let Some(fallback) = self.config.llm_fallback.as_ref() {
            provider_refs.push(fallback);
        }
        for slot in &self.config.model_pool.slots {
            if slot.enabled && slot.id != self.primary_model_id {
                provider_refs.push(&slot.provider);
            }
        }
        merge_app_llm_env_from_providers(&provider_refs)
    }

    pub(super) fn provider_has_runtime_credentials(provider: &crate::core::LlmProvider) -> bool {
        match provider {
            crate::core::LlmProvider::Ollama { .. } => true,
            crate::core::LlmProvider::Anthropic { api_key, .. }
            | crate::core::LlmProvider::OpenAI { api_key, .. } => {
                !api_key.trim().is_empty() && api_key != "[ENCRYPTED]"
            }
        }
    }

    pub(super) fn ready_slot_client(&self, slot_id: &str) -> Option<&LlmClient> {
        self.model_pool.get(slot_id).and_then(|(slot, client)| {
            if slot.enabled && Self::provider_has_runtime_credentials(&slot.provider) {
                Some(client)
            } else {
                None
            }
        })
    }

    #[allow(dead_code)]
    pub(super) fn sanitize_mcp_output(&self, output: &str) -> String {
        // All external content is wrapped unconditionally in a structural envelope
        // and normalized + secret-scrubbed by `trust_boundary::sanitize_untrusted_output`.
        // This treats MCP output as untrusted by default rather than trying to
        // pattern-match whether a particular payload "looks like" an injection.
        crate::security::sanitize_untrusted_output("mcp", output)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn test_candidate(slot_id: &str, provider: crate::core::LlmProvider) -> LlmAttemptCandidate {
        LlmAttemptCandidate {
            slot_id: slot_id.to_string(),
            slot_label: slot_id.to_string(),
            role: ModelRole::Primary,
            client: LlmClient::new(&provider).expect("test provider should create client"),
        }
    }

    #[test]
    fn model_attempt_order_prefers_provider_diversity_before_same_provider_retries() {
        let candidates = vec![
            test_candidate(
                "openrouter-primary",
                crate::core::LlmProvider::OpenAI {
                    api_key: "test".to_string(),
                    model: "provider-a/model-one".to_string(),
                    base_url: Some("https://openrouter.ai/api/v1".to_string()),
                },
            ),
            test_candidate(
                "openrouter-secondary",
                crate::core::LlmProvider::OpenAI {
                    api_key: "test".to_string(),
                    model: "provider-a/model-two".to_string(),
                    base_url: Some("https://openrouter.ai/api/v1".to_string()),
                },
            ),
            test_candidate(
                "openai-primary",
                crate::core::LlmProvider::OpenAI {
                    api_key: "test".to_string(),
                    model: "gpt-test".to_string(),
                    base_url: None,
                },
            ),
            test_candidate(
                "anthropic-primary",
                crate::core::LlmProvider::Anthropic {
                    api_key: "test".to_string(),
                    model: "claude-test".to_string(),
                },
            ),
        ];

        let ordered = diversify_model_attempt_providers(candidates)
            .into_iter()
            .map(|candidate| candidate.slot_id)
            .collect::<Vec<_>>();

        assert_eq!(
            ordered,
            vec![
                "openrouter-primary",
                "openai-primary",
                "anthropic-primary",
                "openrouter-secondary"
            ]
        );
    }
}
