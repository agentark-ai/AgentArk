use super::*;
use sha2::{Digest, Sha256};
use std::collections::HashMap;
use std::sync::{Arc, Mutex};

static WATCHER_NOTIFICATION_SUMMARY_PERMITS: once_cell::sync::Lazy<Arc<tokio::sync::Semaphore>> =
    once_cell::sync::Lazy::new(|| {
        Arc::new(tokio::sync::Semaphore::new(
            watcher_notification_summary_concurrency_limit(),
        ))
    });
static WATCHER_NOTIFICATION_SUMMARY_CACHE: once_cell::sync::Lazy<
    Mutex<HashMap<String, WatcherNotificationSummaryCacheEntry>>,
> = once_cell::sync::Lazy::new(|| Mutex::new(HashMap::new()));

#[derive(Debug, Clone)]
struct WatcherNotificationSummaryCacheEntry {
    output: Option<String>,
    started_at: chrono::DateTime<chrono::Utc>,
    completed_at: Option<chrono::DateTime<chrono::Utc>>,
}

enum WatcherNotificationSummaryReservation {
    Cached(String),
    InFlight,
    Reserved,
}

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

    fn watcher_notification_llm_candidates(&self) -> Vec<LlmAttemptCandidate> {
        let candidates = self.llm_candidates_for_role(&ModelRole::Fast);
        let fast_candidates = candidates
            .iter()
            .filter(|candidate| candidate.role == ModelRole::Fast)
            .cloned()
            .collect::<Vec<_>>();
        if fast_candidates.is_empty() {
            candidates.into_iter().take(1).collect()
        } else {
            fast_candidates
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
            cached_prompt_tokens: usage.cached_prompt_tokens.min(i32::MAX as u64) as i32,
            cache_creation_prompt_tokens: usage.cache_creation_prompt_tokens.min(i32::MAX as u64)
                as i32,
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

        if ordered.is_empty() {
            vec![]
        } else {
            ordered
        }
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
        let cache_key = watcher_notification_summary_cache_key(watcher, result);
        match reserve_watcher_notification_summary(&cache_key) {
            WatcherNotificationSummaryReservation::Cached(output) => {
                tracing::debug!(
                    "Watcher '{}' notification summary reused for repeated result",
                    watcher.id
                );
                return Ok(output);
            }
            WatcherNotificationSummaryReservation::InFlight => {
                tracing::debug!(
                    "Watcher '{}' notification summary already in flight; using fallback text",
                    watcher.id
                );
                return Ok(fallback_watcher_notification_text(watcher, result));
            }
            WatcherNotificationSummaryReservation::Reserved => {}
        }

        let acquire_timeout_ms = watcher_notification_summary_acquire_timeout_ms();
        let permit = match tokio::time::timeout(
            std::time::Duration::from_millis(acquire_timeout_ms),
            WATCHER_NOTIFICATION_SUMMARY_PERMITS.clone().acquire_owned(),
        )
        .await
        {
            Ok(Ok(permit)) => permit,
            Ok(Err(_)) => {
                let fallback = fallback_watcher_notification_text(watcher, result);
                complete_watcher_notification_summary(&cache_key, fallback.clone());
                tracing::warn!(
                    "Watcher '{}' notification summary semaphore closed; using fallback text",
                    watcher.id
                );
                return Ok(fallback);
            }
            Err(_) => {
                let fallback = fallback_watcher_notification_text(watcher, result);
                complete_watcher_notification_summary(&cache_key, fallback.clone());
                tracing::debug!(
                    "Watcher '{}' notification summary saturated after {}ms; using fallback text",
                    watcher.id,
                    acquire_timeout_ms
                );
                return Ok(fallback);
            }
        };

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
                &ModelRole::Fast,
                self.watcher_notification_llm_candidates(),
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
                watcher_notification_summary_timeout_ms(),
                1,
            )
            .await
        {
            Some(response) => response,
            None => {
                drop(permit);
                let fallback = fallback_watcher_notification_text(watcher, result);
                complete_watcher_notification_summary(&cache_key, fallback.clone());
                return Ok(fallback);
            }
        };
        drop(permit);

        let cleaned = normalize_watcher_notification_text(&response.content, &watcher.description);
        if cleaned.is_empty() {
            let fallback = fallback_watcher_notification_text(watcher, result);
            complete_watcher_notification_summary(&cache_key, fallback.clone());
            return Ok(fallback);
        }
        complete_watcher_notification_summary(&cache_key, cleaned.clone());
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
        let summary_timeout_ms = watcher_notification_summary_timeout_ms()
            .saturating_add(watcher_notification_summary_acquire_timeout_ms())
            .saturating_add(1_000);
        let execution = tokio::time::timeout(
            std::time::Duration::from_millis(summary_timeout_ms),
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
                    "Watcher '{}' notification summary failed; using fallback notification text: {}",
                    watcher.id,
                    error
                );
                (fallback_watcher_notification_text(watcher, result), None)
            }
            Err(_) => {
                tracing::warn!(
                    "Watcher '{}' notification summary timed out after {}ms; using fallback notification text",
                    watcher.id,
                    summary_timeout_ms
                );
                (fallback_watcher_notification_text(watcher, result), None)
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

fn watcher_notification_summary_concurrency_limit() -> usize {
    watcher_notification_env_usize(
        "AGENTARK_WATCHER_NOTIFICATION_SUMMARY_CONCURRENCY",
        2,
        1,
        16,
    )
}

fn watcher_notification_summary_acquire_timeout_ms() -> u64 {
    watcher_notification_env_u64(
        "AGENTARK_WATCHER_NOTIFICATION_SUMMARY_ACQUIRE_TIMEOUT_MS",
        100,
        0,
        5_000,
    )
}

fn watcher_notification_summary_timeout_ms() -> u64 {
    watcher_notification_env_u64(
        "AGENTARK_WATCHER_NOTIFICATION_TIMEOUT_MS",
        12_000,
        1_000,
        60_000,
    )
}

fn watcher_notification_summary_cooldown_secs() -> i64 {
    watcher_notification_env_u64(
        "AGENTARK_WATCHER_NOTIFICATION_DEDUPE_COOLDOWN_SECS",
        600,
        30,
        86_400,
    ) as i64
}

fn watcher_notification_env_usize(key: &str, default: usize, min: usize, max: usize) -> usize {
    std::env::var(key)
        .ok()
        .and_then(|value| value.trim().parse::<usize>().ok())
        .unwrap_or(default)
        .clamp(min, max)
}

fn watcher_notification_env_u64(key: &str, default: u64, min: u64, max: u64) -> u64 {
    std::env::var(key)
        .ok()
        .and_then(|value| value.trim().parse::<u64>().ok())
        .unwrap_or(default)
        .clamp(min, max)
}

fn reserve_watcher_notification_summary(key: &str) -> WatcherNotificationSummaryReservation {
    let now = chrono::Utc::now();
    let cooldown_secs = watcher_notification_summary_cooldown_secs();
    let pending_ttl_ms = watcher_notification_summary_timeout_ms()
        .saturating_add(watcher_notification_summary_acquire_timeout_ms())
        .saturating_add(1_000);
    let Ok(mut cache) = WATCHER_NOTIFICATION_SUMMARY_CACHE.lock() else {
        return WatcherNotificationSummaryReservation::Reserved;
    };
    prune_watcher_notification_summary_cache(&mut cache, now, cooldown_secs, pending_ttl_ms);
    if let Some(entry) = cache.get(key) {
        if let (Some(output), Some(completed_at)) = (&entry.output, entry.completed_at) {
            if now.signed_duration_since(completed_at).num_seconds().max(0) <= cooldown_secs {
                return WatcherNotificationSummaryReservation::Cached(output.clone());
            }
        }
        let pending_age_ms = now
            .signed_duration_since(entry.started_at)
            .num_milliseconds()
            .max(0) as u64;
        if entry.output.is_none() && pending_age_ms <= pending_ttl_ms {
            return WatcherNotificationSummaryReservation::InFlight;
        }
    }
    cache.insert(
        key.to_string(),
        WatcherNotificationSummaryCacheEntry {
            output: None,
            started_at: now,
            completed_at: None,
        },
    );
    WatcherNotificationSummaryReservation::Reserved
}

fn complete_watcher_notification_summary(key: &str, output: String) {
    let now = chrono::Utc::now();
    let Ok(mut cache) = WATCHER_NOTIFICATION_SUMMARY_CACHE.lock() else {
        return;
    };
    cache.insert(
        key.to_string(),
        WatcherNotificationSummaryCacheEntry {
            output: Some(output),
            started_at: now,
            completed_at: Some(now),
        },
    );
}

fn prune_watcher_notification_summary_cache(
    cache: &mut HashMap<String, WatcherNotificationSummaryCacheEntry>,
    now: chrono::DateTime<chrono::Utc>,
    cooldown_secs: i64,
    pending_ttl_ms: u64,
) {
    cache.retain(|_, entry| {
        if let Some(completed_at) = entry.completed_at {
            return now.signed_duration_since(completed_at).num_seconds().max(0) <= cooldown_secs;
        }
        let pending_age_ms = now
            .signed_duration_since(entry.started_at)
            .num_milliseconds()
            .max(0) as u64;
        pending_age_ms <= pending_ttl_ms
    });
    if cache.len() > 2_048 {
        let overflow = cache.len().saturating_sub(1_024);
        let keys = cache.keys().take(overflow).cloned().collect::<Vec<_>>();
        for key in keys {
            cache.remove(&key);
        }
    }
}

fn watcher_notification_summary_cache_key(
    watcher: &super::watcher::Watcher,
    result: &str,
) -> String {
    format!(
        "{}:{}",
        watcher.id,
        watcher_notification_result_fingerprint(watcher, result)
    )
}

fn watcher_notification_result_fingerprint(
    watcher: &super::watcher::Watcher,
    result: &str,
) -> String {
    let projection = watcher_notification_fingerprint_projection(watcher, result);
    let serialized = serde_json::to_string(&projection).unwrap_or_else(|_| projection.to_string());
    let mut hasher = Sha256::new();
    hasher.update(b"agentark-watcher-notification-summary-v1");
    hasher.update([0u8]);
    hasher.update(serialized.as_bytes());
    hex::encode(hasher.finalize())
}

fn watcher_notification_fingerprint_projection(
    watcher: &super::watcher::Watcher,
    result: &str,
) -> serde_json::Value {
    let primary_result = automation_primary_result_text(result);
    let result_projection =
        if let Some(payload) = Agent::extract_structured_watcher_condition_payload(result) {
            serde_json::json!({
                "kind": "structured",
                "payload": canonical_watcher_notification_value(&payload),
            })
        } else {
            serde_json::json!({
                "kind": "text",
                "text": normalize_watcher_notification_fingerprint_text(&primary_result),
            })
        };
    serde_json::json!({
        "poll_action": &watcher.poll_action,
        "condition": watcher_notification_condition_shape(&watcher.condition),
        "result": result_projection,
        "output_files": watcher_notification_output_file_projection(result),
    })
}

fn watcher_notification_condition_shape(
    condition: &super::watcher::WatchCondition,
) -> serde_json::Value {
    let matcher = match &condition.matcher {
        super::watcher::WatchConditionMatcher::NotEmpty => {
            serde_json::json!({ "type": "not_empty" })
        }
        super::watcher::WatchConditionMatcher::TextContains {
            text,
            case_sensitive,
        } => serde_json::json!({
            "type": "text_contains",
            "text": normalize_watcher_notification_fingerprint_text(text),
            "case_sensitive": case_sensitive,
        }),
        super::watcher::WatchConditionMatcher::Regex { pattern } => {
            serde_json::json!({
                "type": "regex",
                "pattern": pattern,
            })
        }
        super::watcher::WatchConditionMatcher::JsonPredicate {
            path,
            operator,
            value,
        } => serde_json::json!({
            "type": "json_predicate",
            "path": path.trim(),
            "operator": operator,
            "value": value.as_ref().map(canonical_watcher_notification_value),
        }),
        super::watcher::WatchConditionMatcher::JsonLogic { logic, rules } => {
            let rules = rules
                .iter()
                .map(|rule| {
                    serde_json::json!({
                        "path": rule.path.trim(),
                        "operator": &rule.operator,
                        "value": rule.value.as_ref().map(canonical_watcher_notification_value),
                    })
                })
                .collect::<Vec<_>>();
            serde_json::json!({
                "type": "json_logic",
                "logic": logic,
                "rules": rules,
            })
        }
        super::watcher::WatchConditionMatcher::Llm => serde_json::json!({ "type": "llm" }),
    };
    serde_json::json!({
        "evaluation_mode": &condition.evaluation_mode,
        "matcher": matcher,
    })
}

fn watcher_notification_output_file_projection(result: &str) -> Vec<serde_json::Value> {
    watcher_result_output_files(result)
        .iter()
        .map(|path| {
            let lower = path.to_ascii_lowercase();
            let extension = lower.rsplit('.').next().unwrap_or_default();
            serde_json::json!({
                "image": watcher_output_file_is_image(path),
                "extension": extension,
            })
        })
        .collect()
}

fn canonical_watcher_notification_value(value: &serde_json::Value) -> serde_json::Value {
    match value {
        serde_json::Value::Object(map) => {
            let mut keys = map.keys().collect::<Vec<_>>();
            keys.sort();
            let mut out = serde_json::Map::new();
            for key in keys {
                if let Some(value) = map.get(key) {
                    out.insert(key.clone(), canonical_watcher_notification_value(value));
                }
            }
            serde_json::Value::Object(out)
        }
        serde_json::Value::Array(items) => serde_json::Value::Array(
            items
                .iter()
                .map(canonical_watcher_notification_value)
                .collect(),
        ),
        serde_json::Value::String(text) => {
            serde_json::Value::String(normalize_watcher_notification_fingerprint_text(text))
        }
        _ => value.clone(),
    }
}

fn normalize_watcher_notification_fingerprint_text(text: &str) -> String {
    text.split_whitespace().collect::<Vec<_>>().join(" ")
}
