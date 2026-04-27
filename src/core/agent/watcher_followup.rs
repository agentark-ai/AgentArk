use super::*;

#[derive(Debug, Clone)]
pub(crate) struct WatcherFollowupPreparation {
    pub(super) origin: AutomationOriginContext,
    pub(super) policy: AutomationExecutionPolicy,
    pub(super) attempt: u32,
    pub(super) started_at: chrono::DateTime<chrono::Utc>,
    pub(super) finished_at: chrono::DateTime<chrono::Utc>,
    pub(super) notification_image: Option<WatcherNotificationImage>,
    pub(super) output: String,
    pub(super) suppress_external_reason: Option<String>,
}

#[derive(Clone)]
pub(crate) struct WatcherFollowupWorker {
    storage: Storage,
    llm: LlmClient,
    model_pool: HashMap<String, (ModelSlot, LlmClient)>,
    execution_supervisor: super::ExecutionSupervisor,
    config: AgentConfig,
    primary_model_id: String,
    user_selected_model_slot_id: Arc<std::sync::RwLock<Option<String>>>,
    data_dir: PathBuf,
}

impl WatcherFollowupWorker {
    pub(crate) fn from_agent(agent: &Agent) -> Self {
        Self {
            storage: agent.storage.clone(),
            llm: agent.llm.clone(),
            model_pool: agent.model_pool.clone(),
            execution_supervisor: agent.execution_supervisor.clone(),
            config: agent.config.clone(),
            primary_model_id: agent.primary_model_id.clone(),
            user_selected_model_slot_id: agent.user_selected_model_slot_id.clone(),
            data_dir: agent.data_dir.clone(),
        }
    }

    fn user_selected_model_slot_id(&self) -> Option<String> {
        self.user_selected_model_slot_id
            .read()
            .ok()
            .and_then(|guard| guard.clone())
    }

    fn llm_candidates_for_role(&self, preferred_role: &ModelRole) -> Vec<LlmAttemptCandidate> {
        llm_attempt_candidates_for_role(
            &self.config,
            &self.model_pool,
            &self.primary_model_id,
            self.user_selected_model_slot_id().as_deref(),
            &self.llm,
            preferred_role,
        )
    }

    fn primary_llm_candidate(&self) -> LlmAttemptCandidate {
        LlmAttemptCandidate {
            slot_id: self.primary_model_id.clone(),
            slot_label: "Primary".to_string(),
            role: ModelRole::Primary,
            client: self.llm.clone(),
        }
    }

    fn execution_candidate_descriptor(
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

    async fn record_llm_usage(
        &self,
        channel: &str,
        purpose: &str,
        resp: &crate::core::llm::LlmResponse,
    ) {
        let Some(usage) = resp.usage.as_ref() else {
            return;
        };
        let model = crate::storage::entities::llm_usage::Model {
            id: uuid::Uuid::new_v4().to_string(),
            created_at: chrono::Utc::now().to_rfc3339(),
            provider: resp.provider.clone(),
            model: resp.model.clone(),
            channel: channel.to_string(),
            purpose: purpose.to_string(),
            prompt_tokens: usage.prompt_tokens.min(i32::MAX as u64) as i32,
            completion_tokens: usage.completion_tokens.min(i32::MAX as u64) as i32,
            total_tokens: usage.total_tokens.min(i32::MAX as u64) as i32,
            estimated: usage.estimated,
            cost_usd: usage.cost_usd,
        };
        if let Err(error) = self.storage.insert_llm_usage(&model).await {
            tracing::debug!("Failed to record llm_usage: {}", error);
        }
    }

    #[allow(clippy::too_many_arguments)]
    async fn record_model_attempt(
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

    async fn reorder_candidates_with_failover(
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
            _ => return candidates,
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

        if ordered.is_empty() { vec![] } else { ordered }
    }

    #[allow(clippy::too_many_arguments)]
    async fn supervised_internal_chat(
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
    ) -> Option<super::llm::LlmResponse> {
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
            preferred_model_role: Some(Agent::model_role_label(&effective_role).to_string()),
            message_preview: Some(safe_truncate(user_message, 200)),
            ..Default::default()
        };
        let mut attempted_models = Vec::new();
        let mut attempt_records = Vec::new();

        for (idx, candidate) in candidates.iter().take(max_candidates.max(1)).enumerate() {
            let started = std::time::Instant::now();
            let result = super::execute_supervised_transport_chat(
                &self.execution_supervisor,
                &candidate.client,
                &request,
                system_prompt,
                user_message,
                memories,
                actions,
                Some(timeout_ms.max(1)),
            )
            .await;

            match result {
                Ok(resp) => {
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
                    return Some(resp);
                }
                Err(error) => {
                    let error_text = error.to_string();
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
        tracing::debug!(
            "Internal supervised request '{}' exhausted its eligible model chain: {}",
            request_kind,
            failure_outcome.user_outcome.message
        );
        None
    }

    async fn compose_watcher_notification_text(
        &self,
        watcher: &super::watcher::Watcher,
        result: &str,
    ) -> Result<String> {
        let primary_result = automation_primary_result_text(result);
        let prompt = serde_json::json!({
            "watcher_goal": safe_truncate(watcher.description.trim(), 240),
            "trigger_instruction": safe_truncate(watcher.on_trigger.trim(), 320),
            "poll_result": safe_truncate(primary_result.trim(), 4000),
            "output_files": watcher_result_output_files(result),
        });

        let response = match self
            .supervised_internal_chat(
                "watcher",
                "notification",
                "watcher_notification",
                &ModelRole::Primary,
                vec![],
                "You write the final notification text for a watcher match.\n\
Write only the message body the user should receive.\n\
Do not repeat the watcher request or title.\n\
Do not mention tools, channels, environment limitations, or that you cannot send messages.\n\
Summarize only the matched update, why it mattered, and include at most 1-3 relevant links if present.\n\
If the poll result includes visual observation fields, summarize what is visible or what subjects appear to be doing only from those fields; otherwise say the activity was not provided instead of guessing.\n\
If output files include a snapshot/image, mention that a snapshot is attached.\n\
Keep it concise and useful.",
                &prompt.to_string(),
                &[],
                &[],
                internal_llm_timeout_ms("AGENTARK_WATCHER_NOTIFICATION_TIMEOUT_MS", 20_000),
                2,
            )
            .await
        {
            Some(response) => response,
            None => return Ok(fallback_watcher_notification_text(watcher, result)),
        };

        let cleaned = normalize_watcher_notification_text(&response.content, &watcher.description);
        if cleaned.is_empty() {
            return Ok(fallback_watcher_notification_text(watcher, result));
        }
        Ok(cleaned)
    }

    pub(crate) async fn prepare_watcher_followup(
        &self,
        watcher: &super::watcher::Watcher,
        result: &str,
    ) -> WatcherFollowupPreparation {
        let origin = automation_origin_from_arguments(&watcher.poll_arguments);
        let policy = automation_policy_from_arguments(
            &watcher.poll_arguments,
            AutomationValidation {
                mode: AutomationValidationMode::NonEmptyResult,
                text: None,
                ..AutomationValidation::default()
            },
        );
        let attempt = automation_current_attempt(&watcher.poll_arguments);
        let started_at = chrono::Utc::now();
        let notification_image =
            Agent::first_watcher_notification_image_from_data_dir(&self.data_dir, result).await;
        let execution = tokio::time::timeout(
            std::time::Duration::from_secs(policy.stall_timeout_secs),
            self.compose_watcher_notification_text(watcher, result),
        )
        .await;
        let finished_at = chrono::Utc::now();
        tracing::info!(
            "Automation supervisor: watcher '{}' follow-up attempt {} finished",
            watcher.description,
            attempt
        );
        let (output, suppress_external_reason) = match execution {
            Ok(Ok(output)) => (output, None),
            Ok(Err(error)) => {
                tracing::warn!(
                    "Watcher '{}' notification summary failed; external notification will be suppressed: {}",
                    watcher.id,
                    error
                );
                (
                    fallback_watcher_notification_text(watcher, result),
                    Some("summary generation failed".to_string()),
                )
            }
            Err(_) => {
                tracing::warn!(
                    "Watcher '{}' notification summary timed out after {} seconds; external notification will be suppressed",
                    watcher.id,
                    policy.stall_timeout_secs
                );
                (
                    fallback_watcher_notification_text(watcher, result),
                    Some("summary generation timed out".to_string()),
                )
            }
        };

        WatcherFollowupPreparation {
            origin,
            policy,
            attempt,
            started_at,
            finished_at,
            notification_image,
            output,
            suppress_external_reason,
        }
    }
}
