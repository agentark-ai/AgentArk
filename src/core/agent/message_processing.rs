use super::*;

#[derive(Clone)]
struct ArkEvolveTurnRecordInput {
    run_id: String,
    trace_id: String,
    message: String,
    response: String,
    channel: String,
    conversation_id: Option<String>,
    project_id: Option<String>,
    model_used: String,
    run_status: String,
    user_outcome: crate::core::UserFacingOutcome,
    degradation: Vec<crate::core::DegradationNote>,
    attempted_models: Vec<crate::core::ModelAttemptRecord>,
    turn_records: Vec<AgentTurnRecord>,
    turn_plan: Option<ExecutionPlan>,
}

#[derive(Clone, Copy, Debug, Default)]
struct TurnPipelineUsageSnapshot {
    input_tokens: i64,
    output_tokens: i64,
    total_tokens: i64,
    cost_usd: f64,
}

impl TurnPipelineUsageSnapshot {
    fn delta_since(self, previous: Self) -> Self {
        Self {
            input_tokens: self.input_tokens.saturating_sub(previous.input_tokens),
            output_tokens: self.output_tokens.saturating_sub(previous.output_tokens),
            total_tokens: self.total_tokens.saturating_sub(previous.total_tokens),
            cost_usd: (self.cost_usd - previous.cost_usd).max(0.0),
        }
    }
}

fn arkevolve_hash(parts: &[&str]) -> String {
    let mut hasher = Sha256::new();
    hasher.update(b"arkevolve");
    for part in parts {
        hasher.update([0u8]);
        hasher.update(part.as_bytes());
    }
    hex::encode(hasher.finalize())
}

fn arkevolve_tool_sequence(records: &[AgentTurnRecord]) -> Vec<serde_json::Value> {
    records
        .iter()
        .filter_map(|record| {
            let tool_name = record.action_name.as_deref()?.trim();
            if tool_name.is_empty() {
                return None;
            }
            Some(serde_json::json!({
                "goal_id": record.goal_id,
                "tool_name": tool_name,
                "status": match record.outcome {
                    AgentTurnOutcomeKind::Succeeded => "success",
                    AgentTurnOutcomeKind::NeedsClarification => "needs_input",
                    AgentTurnOutcomeKind::RespondedWithoutTool => "no_handler",
                    AgentTurnOutcomeKind::Abandoned => "blocked",
                    AgentTurnOutcomeKind::Skipped => "cancelled",
                },
                "side_effect": record.side_effect.clone(),
                "object_kind": record.resolved_object_ref.as_ref().map(|value| &value.kind),
            }))
        })
        .collect()
}

fn arkevolve_task_type(records: &[AgentTurnRecord]) -> String {
    records
        .iter()
        .filter_map(|record| record.action_name.as_deref())
        .map(str::trim)
        .find(|value| !value.is_empty())
        .map(|value| value.to_ascii_lowercase())
        .unwrap_or_else(|| "conversation".to_string())
}

fn arkevolve_intent_key(message: &str, task_type: &str, records: &[AgentTurnRecord]) -> String {
    let semantic_parts = records
        .iter()
        .filter_map(|record| {
            let action = record.action_name.as_deref()?.trim();
            if action.is_empty() {
                return None;
            }
            let object_kind = record
                .resolved_object_ref
                .as_ref()
                .map(|value| format!("{:?}", value.kind))
                .unwrap_or_default();
            Some(format!("{}:{}", action.to_ascii_lowercase(), object_kind))
        })
        .collect::<Vec<_>>();
    if semantic_parts.is_empty() {
        return crate::core::learning::derive_intent_key(message, task_type);
    }
    let joined = semantic_parts.join("|");
    format!(
        "{}::{}",
        task_type,
        &arkevolve_hash(&[joined.as_str()])[..16]
    )
}

fn arkevolve_execution_status(run_status: &str) -> crate::core::ExecutionRunStatus {
    match run_status.trim().to_ascii_lowercase().as_str() {
        "completed" => crate::core::ExecutionRunStatus::Completed,
        "completed_degraded" | "degraded" => crate::core::ExecutionRunStatus::Degraded,
        "needs_input" => crate::core::ExecutionRunStatus::NeedsInput,
        "blocked" => crate::core::ExecutionRunStatus::Blocked,
        "needs_stronger_model" => crate::core::ExecutionRunStatus::NeedsStrongerModel,
        "platform_failed" => crate::core::ExecutionRunStatus::PlatformFailed,
        _ => crate::core::ExecutionRunStatus::Degraded,
    }
}

fn arkevolve_success_state(
    status: &crate::core::ExecutionRunStatus,
    records: &[AgentTurnRecord],
) -> &'static str {
    let has_success = records
        .iter()
        .any(|record| record.outcome == AgentTurnOutcomeKind::Succeeded);
    let has_terminal_failure = records.iter().any(|record| {
        matches!(
            record.outcome,
            AgentTurnOutcomeKind::Abandoned | AgentTurnOutcomeKind::Skipped
        )
    });
    let has_non_terminal_direct_response = records.iter().any(|record| {
        matches!(
            record.outcome,
            AgentTurnOutcomeKind::RespondedWithoutTool | AgentTurnOutcomeKind::NeedsClarification
        )
    });
    let degraded_without_terminal_failure = !has_terminal_failure
        && (has_success || has_non_terminal_direct_response || records.is_empty());
    match status {
        crate::core::ExecutionRunStatus::Completed => "accepted",
        crate::core::ExecutionRunStatus::Degraded if degraded_without_terminal_failure => {
            "accepted"
        }
        _ => "failed",
    }
}

fn arkevolve_record_status(outcome: &AgentTurnOutcomeKind) -> crate::core::ToolOutcomeStatus {
    match outcome {
        AgentTurnOutcomeKind::Succeeded => crate::core::ToolOutcomeStatus::Success,
        AgentTurnOutcomeKind::NeedsClarification => crate::core::ToolOutcomeStatus::NeedsInput,
        AgentTurnOutcomeKind::RespondedWithoutTool => crate::core::ToolOutcomeStatus::NoHandler,
        AgentTurnOutcomeKind::Abandoned => crate::core::ToolOutcomeStatus::Blocked,
        AgentTurnOutcomeKind::Skipped => crate::core::ToolOutcomeStatus::Cancelled,
    }
}

fn arkevolve_learning_signal(
    input: &ArkEvolveTurnRecordInput,
    tool_sequence: &[serde_json::Value],
    success_state: &str,
) -> serde_json::Value {
    let successful_goals = input
        .turn_records
        .iter()
        .filter(|record| record.outcome == AgentTurnOutcomeKind::Succeeded)
        .count();
    let failed_goals = input
        .turn_records
        .iter()
        .filter(|record| {
            matches!(
                record.outcome,
                AgentTurnOutcomeKind::Abandoned | AgentTurnOutcomeKind::Skipped
            )
        })
        .count();
    let has_tool_evidence = !tool_sequence.is_empty();
    let has_failure_evidence = failed_goals > 0 || success_state == "failed";
    serde_json::json!({
        "procedure_eligible": has_tool_evidence || successful_goals > 0 || has_failure_evidence,
        "global_learning": true,
        "scope_policy": "global",
        "tool_evidence_count": tool_sequence.len(),
        "successful_goal_count": successful_goals,
        "failed_goal_count": failed_goals,
        "degradation_count": input.degradation.len(),
        "recorded_without_blocking_chat": true,
    })
}

async fn persist_arkevolve_turn_recording(
    storage: crate::storage::Storage,
    input: ArkEvolveTurnRecordInput,
) -> Result<()> {
    let now = chrono::Utc::now().to_rfc3339();
    let execution_status = arkevolve_execution_status(&input.run_status);
    let success_state = arkevolve_success_state(&execution_status, &input.turn_records);
    let task_type = arkevolve_task_type(&input.turn_records);
    let intent_key = arkevolve_intent_key(&input.message, &task_type, &input.turn_records);
    let tool_sequence = arkevolve_tool_sequence(&input.turn_records);
    let tool_sequence_json = serde_json::Value::Array(tool_sequence.clone());
    let tool_sequence_digest = if tool_sequence.is_empty() {
        None
    } else {
        let canonical = serde_json::to_string(&tool_sequence_json).unwrap_or_default();
        Some(arkevolve_hash(&[canonical.as_str()])[..24].to_string())
    };
    let learning_signal = arkevolve_learning_signal(&input, &tool_sequence, success_state);
    let run = crate::core::ExecutionRun {
        id: input.run_id.clone(),
        kind: "chat_turn".to_string(),
        request_id: Some(input.run_id.clone()),
        status: execution_status.clone(),
        current_stage: execution_status.as_str().to_string(),
        lease_owner: None,
        lease_expires_at: None,
        attempt: 0,
        deadline_at: None,
        cancellation_requested: false,
        degradation: input.degradation.clone(),
        last_error: (success_state == "failed")
            .then(|| input.user_outcome.message.clone())
            .filter(|value| !value.trim().is_empty()),
        result_summary: Some(safe_truncate(&input.response, 500)),
        trace_id: Some(input.trace_id.clone()),
        conversation_id: input.conversation_id.clone(),
        channel: Some(input.channel.clone()),
        request_message: Some(input.message.clone()),
        attempted_models: input.attempted_models.clone(),
        created_at: now.clone(),
        updated_at: now.clone(),
    };
    storage.insert_execution_run(&run).await?;

    let experience_run_id = format!("exprun-{}", &arkevolve_hash(&[input.run_id.as_str()])[..24]);
    storage
        .upsert_experience_run(&crate::storage::experience_run::Model {
            id: experience_run_id.clone(),
            execution_run_id: Some(input.run_id.clone()),
            trace_id: Some(input.trace_id.clone()),
            conversation_id: input.conversation_id.clone(),
            project_id: None,
            channel: input.channel.clone(),
            scope: "global".to_string(),
            intent_key,
            task_type: Some(task_type.clone()),
            request_text: Some(safe_truncate(&input.message, 600)),
            tool_sequence_digest: tool_sequence_digest.clone(),
            tool_sequence_json: tool_sequence_json.clone(),
            strategy_version: None,
            policy_version: None,
            prompt_version: None,
            model_slot: Some(input.model_used.clone()),
            success_state: success_state.to_string(),
            correction_state: "none".to_string(),
            outcome_summary: Some(safe_truncate(&input.response, 800)),
            failure_reason: (success_state == "failed").then(|| {
                input
                    .user_outcome
                    .reason_code
                    .clone()
                    .unwrap_or_else(|| safe_truncate(&input.user_outcome.message, 240))
            }),
            metadata: serde_json::json!({
                "source": "agent_turn_loop",
                "learning_signal": learning_signal,
                "source_project_id": input.project_id.clone(),
                "source_conversation_id": input.conversation_id.clone(),
                "turn_plan": input.turn_plan.clone(),
                "turn_records": input.turn_records.clone(),
                "user_outcome": input.user_outcome.clone(),
            }),
            consolidated: false,
            accepted_at: (success_state == "accepted").then(|| now.clone()),
            corrected_at: None,
            heuristic_reflected: false,
            heuristic_reflection_status: Some("pending".to_string()),
            heuristic_reflection_attempted_at: None,
            heuristic_reflection_completed_at: None,
            heuristic_lesson_id: None,
            heuristic_reflection_error: None,
            created_at: now.clone(),
            updated_at: now.clone(),
        })
        .await?;

    for (index, record) in input
        .turn_records
        .iter()
        .filter(|record| {
            record
                .action_name
                .as_deref()
                .map(str::trim)
                .is_some_and(|value| !value.is_empty())
        })
        .enumerate()
    {
        let tool_name = record.action_name.as_deref().unwrap_or("tool").trim();
        let status = arkevolve_record_status(&record.outcome);
        let attempt = crate::core::ToolAttempt {
            id: format!(
                "tool-{}",
                &arkevolve_hash(&[input.run_id.as_str(), tool_name, &index.to_string()])[..24]
            ),
            run_id: input.run_id.clone(),
            sequence_no: index as u32,
            tool_name: tool_name.to_string(),
            status: status.clone(),
            failure_class: None,
            retryable: false,
            side_effect_level: record
                .side_effect
                .as_deref()
                .unwrap_or("unknown")
                .to_string(),
            idempotency_key: None,
            arguments_json: serde_json::to_string(&serde_json::json!({
                "goal_id": record.goal_id,
                "resolved_object_ref": record.resolved_object_ref.as_ref(),
            }))?,
            output_json: serde_json::to_string(
                &record
                    .tool_output
                    .clone()
                    .unwrap_or_else(|| serde_json::Value::Object(Default::default())),
            )?,
            started_at: now.clone(),
            completed_at: Some(now.clone()),
            error_text: record
                .reason
                .clone()
                .or_else(|| record.clarification_question.clone()),
        };
        storage.append_tool_attempt(&attempt).await?;
        let edge_type = if status == crate::core::ToolOutcomeStatus::Success {
            "succeeded_with"
        } else {
            "failed_with"
        };
        storage
            .upsert_experience_edge(&crate::storage::experience_edge::Model {
                id: format!(
                    "edge-{}",
                    &arkevolve_hash(&[
                        input.run_id.as_str(),
                        edge_type,
                        tool_name,
                        &index.to_string()
                    ])[..24]
                ),
                source_ref: experience_run_id.clone(),
                source_kind: "experience_run".to_string(),
                target_ref: tool_name.to_string(),
                target_kind: "tool".to_string(),
                edge_type: edge_type.to_string(),
                weight: if edge_type == "succeeded_with" {
                    1.0
                } else {
                    0.35
                },
                source_run_id: Some(experience_run_id.clone()),
                metadata: serde_json::json!({
                    "goal_id": record.goal_id,
                    "status": status.as_str(),
                    "global_learning": true,
                }),
                created_at: now.clone(),
                updated_at: now.clone(),
            })
            .await?;
    }
    Ok(())
}

impl Agent {
    fn spawn_arkevolve_turn_recording(&self, input: ArkEvolveTurnRecordInput) {
        let storage = self.storage.clone();
        tokio::spawn(async move {
            if let Err(error) = persist_arkevolve_turn_recording(storage, input).await {
                tracing::warn!("ArkEvolve turn evidence recording failed: {}", error);
            }
        });
    }

    /// Process an incoming message and generate a response
    pub async fn process_message_with_meta(
        &self,
        message: &str,
        channel: &str,
        conversation_id: Option<&str>,
        project_id: Option<&str>,
    ) -> Result<ProcessedMessage> {
        self.process_message_with_meta_and_hints(
            message,
            channel,
            conversation_id,
            project_id,
            RequestExecutionHints::default(),
        )
        .await
    }

    pub async fn process_message_with_meta_and_hints(
        &self,
        message: &str,
        channel: &str,
        conversation_id: Option<&str>,
        project_id: Option<&str>,
        request_hints: RequestExecutionHints,
    ) -> Result<ProcessedMessage> {
        self.process_turn_request(
            message,
            channel,
            conversation_id,
            project_id,
            request_hints,
            false,
            false,
            None,
        )
        .await
    }

    /// Process an incoming message and return only response text.
    pub async fn process_message(
        &self,
        message: &str,
        channel: &str,
        conversation_id: Option<&str>,
        project_id: Option<&str>,
    ) -> Result<String> {
        let processed = self
            .process_message_with_meta(message, channel, conversation_id, project_id)
            .await?;
        Ok(Self::render_plain_channel_response(processed))
    }

    /// Process a message with per-request trace + streaming tokens/tools.
    pub async fn process_message_stream_with_meta(
        &self,
        message: &str,
        channel: &str,
        conversation_id: Option<&str>,
        project_id: Option<&str>,
        trace_override: Arc<RwLock<ExecutionTrace>>,
        token_tx: tokio::sync::mpsc::Sender<StreamEvent>,
    ) -> Result<ProcessedMessage> {
        self.process_message_stream_with_meta_and_hints(
            message,
            channel,
            conversation_id,
            project_id,
            trace_override,
            token_tx,
            RequestExecutionHints::default(),
        )
        .await
    }

    #[expect(
        clippy::too_many_arguments,
        reason = "Public streaming API preserves existing call sites"
    )]
    pub async fn process_message_stream_with_meta_and_hints(
        &self,
        message: &str,
        channel: &str,
        conversation_id: Option<&str>,
        project_id: Option<&str>,
        _trace_override: Arc<RwLock<ExecutionTrace>>,
        token_tx: tokio::sync::mpsc::Sender<StreamEvent>,
        request_hints: RequestExecutionHints,
    ) -> Result<ProcessedMessage> {
        let fallback_tx = token_tx.clone();
        match self
            .process_turn_request(
                message,
                channel,
                conversation_id,
                project_id,
                request_hints,
                false,
                false,
                Some(token_tx.clone()),
            )
            .await
        {
            Ok(processed) => {
                queue_stream_event(
                    &fallback_tx,
                    StreamEvent::ToolProgress {
                        name: "turn".to_string(),
                        content: processed.response.clone(),
                        payload: Some(serde_json::json!({
                            "kind": "turn_completed",
                            "run_status": processed.run_status.clone(),
                        })),
                    },
                );
                Ok(processed)
            }
            Err(error) => Err(error),
        }
    }

    #[expect(
        clippy::too_many_arguments,
        reason = "Public streaming API preserves existing call sites"
    )]
    pub async fn process_message_stream_resume_with_meta_and_hints(
        &self,
        message: &str,
        channel: &str,
        conversation_id: Option<&str>,
        project_id: Option<&str>,
        _trace_override: Arc<RwLock<ExecutionTrace>>,
        token_tx: tokio::sync::mpsc::Sender<StreamEvent>,
        request_hints: RequestExecutionHints,
    ) -> Result<ProcessedMessage> {
        let fallback_tx = token_tx.clone();
        match self
            .process_turn_request(
                message,
                channel,
                conversation_id,
                project_id,
                request_hints,
                true,
                true,
                Some(token_tx.clone()),
            )
            .await
        {
            Ok(processed) => {
                queue_stream_event(
                    &fallback_tx,
                    StreamEvent::ToolProgress {
                        name: "turn".to_string(),
                        content: processed.response.clone(),
                        payload: Some(serde_json::json!({
                            "kind": "turn_completed",
                            "run_status": processed.run_status.clone(),
                        })),
                    },
                );
                Ok(processed)
            }
            Err(error) => Err(error),
        }
    }

    #[expect(
        clippy::too_many_arguments,
        reason = "Shared turn request envelope spans chat, streaming resume, and task follow-up entrypoints"
    )]
    pub(super) async fn process_turn_request(
        &self,
        message: &str,
        channel: &str,
        conversation_id: Option<&str>,
        project_id: Option<&str>,
        mut request_hints: RequestExecutionHints,
        user_message_already_recorded: bool,
        skip_inbound_security_precheck: bool,
        stream_tx: Option<tokio::sync::mpsc::Sender<StreamEvent>>,
    ) -> Result<ProcessedMessage> {
        let _active_request = self.track_active_message_request();
        *self.last_activity.write().await = Some(chrono::Utc::now());

        let early_safe_message = crate::security::redact_secret_input(message).text;
        let (resolved_conversation_id, is_new_conversation) = self
            .resolve_conversation_id(channel, conversation_id, project_id, &early_safe_message)
            .await?;
        let conversation_key = resolved_conversation_id.clone();
        self.maybe_consolidate_idle_background_sessions().await;

        let secret_redaction = crate::security::redact_secret_input(message);
        if secret_redaction.had_secret() {
            tracing::warn!(
                "Security: redacted likely secret input from channel={} ({} match(es))",
                channel,
                secret_redaction.redactions.len()
            );
        }
        let message_storage = secret_redaction.text.clone();

        let mut memory_capture_allowed = false;
        if !skip_inbound_security_precheck {
            if let Some(tx) = stream_tx.as_ref() {
                queue_stream_event(
                    tx,
                    StreamEvent::Thinking("Checking request safety...".to_string()),
                );
            }
            match self
                .run_inbound_security_precheck(
                    &message_storage,
                    &message_storage,
                    channel,
                    &conversation_key,
                    is_new_conversation,
                    project_id,
                    user_message_already_recorded,
                )
                .await?
            {
                InboundSecurityPrecheck::Respond(processed) => return Ok(processed),
                InboundSecurityPrecheck::Continue {
                    memory_capture_allowed: should_capture,
                    routing,
                } => {
                    memory_capture_allowed = should_capture;
                    if let Some(routing) = routing {
                        request_hints.routing = Some(routing);
                    }
                }
            }
        }

        if secret_redaction.had_secret() {
            memory_capture_allowed = false;
            let pending_chat_credential_prompt =
                self.pending_chat_credential_prompt(&conversation_key).await;
            let secure_prompt_pending = pending_chat_credential_prompt.is_some();
            let kind = match secret_redaction
                .primary_kind()
                .unwrap_or(crate::security::SecretInputType::ApiKeyOrToken)
            {
                crate::security::SecretInputType::PrivateKeyMaterial => "private_key_material",
                crate::security::SecretInputType::ApiKeyOrToken => "api_key_or_token",
            };
            request_hints.secret_offered = Some(SecretOfferedHint {
                kind: kind.to_string(),
                redactions: secret_redaction.redactions.clone(),
                secure_prompt_pending,
            });
        }

        let turn_started_at = chrono::Utc::now();
        let usage_before_turn = self.turn_pipeline_usage_snapshot().await;
        match self
            .run_agent_turn_loop_for_chat(
                channel,
                message_storage.as_str(),
                Some(&conversation_key),
                project_id,
                &request_hints,
                stream_tx.clone(),
            )
            .await
        {
            Ok(processed) => {
                let usage_delta = self
                    .turn_pipeline_usage_snapshot()
                    .await
                    .delta_since(usage_before_turn);
                self.persist_turn_pipeline_exchange(
                    message_storage.as_str(),
                    &processed.response,
                    ImmediateExchangeContext {
                        channel,
                        conversation_key: &conversation_key,
                        is_new_conversation,
                        project_id,
                        model_used: "agent_turn_loop",
                        user_message_already_recorded,
                        memory_capture_allowed,
                    },
                    processed.run_status.as_deref().unwrap_or("completed"),
                    processed.trace_steps.clone(),
                    processed.turn_records.clone(),
                    processed.turn_plan.clone(),
                    turn_started_at,
                    usage_delta,
                )
                .await
            }
            Err(error) => {
                if error.to_string() == "Conversation not found" {
                    return Err(error);
                }
                tracing::warn!("Agent turn loop failed on channel '{}': {}", channel, error);
                let response = format!(
                    "The agent turn loop hit a framework-level failure before execution could complete, so I did not run any action. Please retry after checking the server logs. Error: {}",
                    error
                );
                let usage_delta = self
                    .turn_pipeline_usage_snapshot()
                    .await
                    .delta_since(usage_before_turn);
                self.persist_turn_pipeline_exchange(
                    message_storage.as_str(),
                    &response,
                    ImmediateExchangeContext {
                        channel,
                        conversation_key: &conversation_key,
                        is_new_conversation,
                        project_id,
                        model_used: "agent_turn_loop_failed",
                        user_message_already_recorded,
                        memory_capture_allowed,
                    },
                    crate::core::ExecutionRunStatus::PlatformFailed.as_str(),
                    Vec::new(),
                    Vec::new(),
                    None,
                    turn_started_at,
                    usage_delta,
                )
                .await
            }
        }
    }

    async fn turn_pipeline_usage_snapshot(&self) -> TurnPipelineUsageSnapshot {
        let trace = self.last_trace.read().await;
        TurnPipelineUsageSnapshot {
            input_tokens: trace.input_tokens,
            output_tokens: trace.output_tokens,
            total_tokens: trace.total_tokens,
            cost_usd: trace.cost_usd,
        }
    }

    async fn persist_turn_pipeline_exchange(
        &self,
        message: &str,
        response: &str,
        context: ImmediateExchangeContext<'_>,
        run_status: &str,
        trace_steps: Vec<ExecutionStep>,
        turn_records: Vec<AgentTurnRecord>,
        turn_plan: Option<ExecutionPlan>,
        started_at: chrono::DateTime<chrono::Utc>,
        usage_delta: TurnPipelineUsageSnapshot,
    ) -> Result<ProcessedMessage> {
        let trace_id = uuid::Uuid::new_v4().to_string();
        let run_id = uuid::Uuid::new_v4().to_string();
        let trace_time = chrono::Utc::now();
        let first_content_ms = (trace_time - started_at).num_milliseconds().max(1) as u64;
        let filtered_response = self.security.filter_output(response);
        if !filtered_response.redactions.is_empty() {
            tracing::warn!(
                "Security: redacted sensitive data from turn output before persistence ({} rule match(es))",
                filtered_response.redactions.len()
            );
        }
        let safe_response = filtered_response.text;
        let mut steps = Vec::with_capacity(trace_steps.len() + 3);
        steps.push(ExecutionStep {
            icon: "[turn]".to_string(),
            title: "Turn Request".to_string(),
            detail: format!(
                "Agent turn loop | Channel: {} | Length: {} chars",
                context.channel,
                message.chars().count()
            ),
            step_type: "info".to_string(),
            data: None,
            timestamp: started_at,
            duration_ms: Some(0),
        });
        steps.extend(trace_steps);
        steps.push(ExecutionStep {
            icon: "[model]".to_string(),
            title: "First Content".to_string(),
            detail: format!(
                "AgentArk produced the first user-visible response content after {}ms.",
                first_content_ms
            ),
            step_type: "info".to_string(),
            data: Some(
                serde_json::json!({
                    "metric": "time_to_first_token",
                    "duration_ms": first_content_ms,
                    "source": "agent_turn_loop_first_content"
                })
                .to_string(),
            ),
            timestamp: trace_time,
            duration_ms: Some(first_content_ms),
        });
        steps.push(ExecutionStep {
            icon: "[reply]".to_string(),
            title: "Turn Response".to_string(),
            detail: format!(
                "Returned via {} with status '{}'.",
                context.model_used, run_status
            ),
            step_type: "success".to_string(),
            data: Some(safe_truncate(&safe_response, 8000)),
            timestamp: chrono::Utc::now(),
            duration_ms: Some(0),
        });

        let trace_ref = Arc::new(RwLock::new(ExecutionTrace {
            id: trace_id.clone(),
            message: message.to_string(),
            channel: context.channel.to_string(),
            started_at: Some(started_at),
            completed_at: Some(trace_time),
            steps,
            proof_id: None,
            response: Some(safe_response.clone()),
            model: Some(context.model_used.to_string()),
            input_tokens: usage_delta.input_tokens,
            output_tokens: usage_delta.output_tokens,
            total_tokens: usage_delta.total_tokens,
            cost_usd: usage_delta.cost_usd,
            complexity: Some("agent_turn_loop".to_string()),
            plan: turn_plan.clone(),
        }));
        self.seed_execution_trace_snapshot(&trace_ref).await;
        self.persist_completed_trace(&trace_ref).await;

        {
            let mut history = self.conversation_history.write().await;
            let conversation_history = history
                .entry(context.conversation_key.to_string())
                .or_insert_with(Vec::new);
            if !context.user_message_already_recorded {
                conversation_history.push(ConversationMessage {
                    role: "user".to_string(),
                    content: message.to_string(),
                    _timestamp: chrono::Utc::now(),
                });
            }
            conversation_history.push(ConversationMessage {
                role: "assistant".to_string(),
                content: safe_response.clone(),
                _timestamp: chrono::Utc::now(),
            });
            if conversation_history.len() > 10 {
                conversation_history.drain(0..conversation_history.len() - 10);
            }
        }

        let mut conversation_title: Option<String> = None;
        if !context.conversation_key.is_empty() {
            if !context.user_message_already_recorded {
                let now = chrono::Utc::now().to_rfc3339();
                let user_msg = crate::storage::entities::message::Model {
                    id: uuid::Uuid::new_v4().to_string(),
                    conversation_id: context.conversation_key.to_string(),
                    role: "user".to_string(),
                    content: message.to_string(),
                    timestamp: now.clone(),
                    model_used: None,
                    trace_id: Some(trace_id.clone()),
                };
                if let Err(error) = self
                    .encrypted_storage
                    .insert_message_encrypted(&user_msg)
                    .await
                {
                    tracing::warn!("Failed to persist turn-path user message: {}", error);
                }
                if context.memory_capture_allowed {
                    self.spawn_user_memory_capture(
                        message,
                        context.channel,
                        Some(context.conversation_key),
                        context.project_id,
                        Some(&user_msg.id),
                    );
                }
            }

            let asst_msg = crate::storage::entities::message::Model {
                id: uuid::Uuid::new_v4().to_string(),
                conversation_id: context.conversation_key.to_string(),
                role: "assistant".to_string(),
                content: safe_response.clone(),
                timestamp: chrono::Utc::now().to_rfc3339(),
                model_used: Some(context.model_used.to_string()),
                trace_id: Some(trace_id.clone()),
            };
            if let Err(error) = self
                .encrypted_storage
                .insert_message_encrypted(&asst_msg)
                .await
            {
                tracing::warn!("Failed to persist turn-path assistant message: {}", error);
            }

            if context.is_new_conversation {
                let title = self.generate_conversation_title(message);
                let _ = self
                    .storage
                    .update_conversation(context.conversation_key, Some(&title), Some(2), None)
                    .await;
                *self.last_conversation_title.write().await = Some(title.clone());
                conversation_title = Some(title);
            } else {
                *self.last_conversation_title.write().await = None;
            }
        }

        *self.last_conversation_id.write().await = Some(context.conversation_key.to_string());
        if !context.conversation_key.is_empty() {
            self.sync_background_session_after_response(
                context.conversation_key,
                message,
                &safe_response,
            )
            .await;
        }

        let user_outcome = self
            .build_response_heuristic_outcome(&safe_response, &[], &[], None)
            .unwrap_or_else(|| {
                self.execution_supervisor
                    .build_success_outcome(&safe_response, &[], &[])
            });
        if !context.conversation_key.is_empty() {
            self.sync_pending_resilience_followup(
                context.conversation_key,
                message,
                context.channel,
                context.project_id,
                &user_outcome,
            )
            .await;
        }
        let final_run_status = if run_status.trim().is_empty() {
            Self::execution_run_status_for_outcome(&user_outcome)
                .as_str()
                .to_string()
        } else {
            run_status.trim().to_string()
        };
        self.spawn_arkevolve_turn_recording(ArkEvolveTurnRecordInput {
            run_id: run_id.clone(),
            trace_id: trace_id.clone(),
            message: message.to_string(),
            response: safe_response.clone(),
            channel: context.channel.to_string(),
            conversation_id: (!context.conversation_key.trim().is_empty())
                .then(|| context.conversation_key.to_string()),
            project_id: context.project_id.map(str::to_string),
            model_used: context.model_used.to_string(),
            run_status: final_run_status.clone(),
            user_outcome: user_outcome.clone(),
            degradation: user_outcome.degradation.clone(),
            attempted_models: user_outcome.attempted_models.clone(),
            turn_records: turn_records.clone(),
            turn_plan: turn_plan.clone(),
        });

        Ok(ProcessedMessage {
            response: safe_response,
            conversation_id: Some(context.conversation_key.to_string()),
            conversation_title,
            run_id: Some(run_id),
            run_status: Some(final_run_status),
            trace_id: Some(trace_id),
            total_tokens: usage_delta.total_tokens,
            choices: Vec::new(),
            degradation: Vec::new(),
            attempted_models: Vec::new(),
            user_outcome: Some(user_outcome),
            trace_steps: Vec::new(),
            turn_records,
            turn_plan,
        })
    }

    pub(crate) fn render_plain_channel_response(processed: ProcessedMessage) -> String {
        let mut response = processed.response;
        if let Some(outcome) = processed.user_outcome.as_ref() {
            let needs_prefix = match outcome.status {
                super::UserFacingOutcomeStatus::NeedsPermission => {
                    !response.to_ascii_lowercase().contains("approval")
                }
                super::UserFacingOutcomeStatus::NeedsIntegration => {
                    !response.to_ascii_lowercase().contains("integration")
                }
                super::UserFacingOutcomeStatus::NeedsCredentials => {
                    !response.to_ascii_lowercase().contains("credential")
                        && !response.to_ascii_lowercase().contains("api key")
                        && !response.to_ascii_lowercase().contains("token")
                }
                super::UserFacingOutcomeStatus::NeedsStrongerModel => {
                    !response.to_ascii_lowercase().contains("stronger model")
                }
                super::UserFacingOutcomeStatus::ServiceUnavailable => {
                    !response
                        .to_ascii_lowercase()
                        .contains("framework-level problem")
                        && !response.to_ascii_lowercase().contains("service")
                }
                _ => false,
            };

            if needs_prefix {
                let prefix = match outcome.status {
                    super::UserFacingOutcomeStatus::NeedsPermission => {
                        "Approval needed before I can continue.\n\n"
                    }
                    super::UserFacingOutcomeStatus::NeedsIntegration => {
                        "Integration setup needed before I can continue.\n\n"
                    }
                    super::UserFacingOutcomeStatus::NeedsCredentials => {
                        "Credentials or configuration are needed before I can continue.\n\n"
                    }
                    super::UserFacingOutcomeStatus::NeedsStrongerModel => {
                        "A stronger model is needed to finish this request.\n\n"
                    }
                    super::UserFacingOutcomeStatus::ServiceUnavailable => {
                        "The request stayed inside the resilience layer, but the service is currently unavailable.\n\n"
                    }
                    _ => "",
                };
                if !prefix.is_empty() {
                    response = format!("{}{}", prefix, response);
                }
            }
        }
        let should_prefix_degraded = processed
            .user_outcome
            .as_ref()
            .is_some_and(|outcome| outcome.status == super::UserFacingOutcomeStatus::Degraded)
            && processed
                .degradation
                .iter()
                .any(|note| matches!(note.kind.as_str(), "delegation" | "tool" | "tool_dispatch"));

        if should_prefix_degraded
            && !response.starts_with("Note: I completed this with partial")
            && !response.starts_with("Note: I completed this with degraded")
        {
            let prefix = if processed
                .degradation
                .iter()
                .any(|note| note.kind == "delegation")
            {
                "Note: I completed this with partial delegated coverage because one or more execution paths degraded.\n\n"
            } else {
                "Note: I completed this with degraded execution, so parts of the result may be partial.\n\n"
            };
            response = format!("{}{}", prefix, response);
        }

        response
    }

    pub(super) fn build_execution_resume_message(
        run: &crate::core::ExecutionRun,
        checkpoints: &[crate::core::ExecutionCheckpoint],
        tool_attempts: &[crate::core::ToolAttempt],
    ) -> String {
        let mut lines = vec![
            "Resume this AgentArk execution from its last completed checkpoint.".to_string(),
            "Do not restart or repeat completed work unless validation proves it is stale or missing.".to_string(),
            "Continue with the next required action/tool step, and finish only when the original goal is complete or a real blocker is reached.".to_string(),
            String::new(),
            format!("Previous run id: {}", run.id),
            format!("Previous status: {}", run.status.as_str()),
            format!("Previous stage: {}", run.current_stage),
        ];

        if let Some(original) = run
            .request_message
            .as_deref()
            .map(str::trim)
            .filter(|v| !v.is_empty())
        {
            lines.push(format!("Original request: {}", original));
        }
        if let Some(summary) = run
            .result_summary
            .as_deref()
            .map(str::trim)
            .filter(|v| !v.is_empty())
        {
            lines.push(format!(
                "Previous result summary: {}",
                safe_truncate(summary, 600)
            ));
        }
        if let Some(error) = run
            .last_error
            .as_deref()
            .map(str::trim)
            .filter(|v| !v.is_empty())
        {
            lines.push(format!("Previous error: {}", safe_truncate(error, 600)));
        }

        lines.push(String::new());
        lines.push("Persisted checkpoints, oldest to newest:".to_string());
        if checkpoints.is_empty() {
            lines.push("- No checkpoint payloads were persisted for this run; use the run status and original request as context.".to_string());
        } else {
            let start = checkpoints.len().saturating_sub(12);
            for checkpoint in checkpoints.iter().skip(start) {
                lines.push(format!(
                    "- #{} stage={} at {} payload={}",
                    checkpoint.sequence_no,
                    checkpoint.stage,
                    checkpoint.created_at,
                    safe_truncate(&checkpoint.payload, 800)
                ));
            }
        }

        lines.push(String::new());
        lines.push("Persisted tool attempts, oldest to newest:".to_string());
        if tool_attempts.is_empty() {
            lines.push("- No persisted tool attempts were found for this run.".to_string());
        } else {
            let start = tool_attempts.len().saturating_sub(12);
            for attempt in tool_attempts.iter().skip(start) {
                let error = attempt
                    .error_text
                    .as_deref()
                    .map(str::trim)
                    .filter(|value| !value.is_empty())
                    .map(|value| format!(" error={}", safe_truncate(value, 300)))
                    .unwrap_or_default();
                lines.push(format!(
                    "- #{} tool={} status={} retryable={} side_effect={} args={} output={}{}",
                    attempt.sequence_no,
                    attempt.tool_name,
                    attempt.status.as_str(),
                    attempt.retryable,
                    attempt.side_effect_level,
                    safe_truncate(&attempt.arguments_json, 500),
                    safe_truncate(&attempt.output_json, 700),
                    error
                ));
            }
        }

        lines.push(String::new());
        lines.push("If the last completed step only installed dependencies, prepared files, cloned a repo, or gathered setup evidence, continue from the validation or handoff step instead of reinstalling/recloning. If a persistent object already exists, inspect/reuse it rather than creating duplicates.".to_string());
        lines.join("\n")
    }

    pub async fn resume_execution_run(
        &self,
        run_id: &str,
        caller: Option<&ActionCallerPrincipal>,
    ) -> Result<ProcessedMessage> {
        let run = self
            .storage
            .load_execution_run(run_id)
            .await?
            .ok_or_else(|| anyhow::anyhow!("Run not found"))?;
        let checkpoints = self.storage.load_execution_checkpoints(run_id).await?;
        let tool_attempts = self.storage.list_tool_attempts_for_run(run_id).await?;
        let resume_message =
            Self::build_execution_resume_message(&run, &checkpoints, &tool_attempts);
        let mut hints = RequestExecutionHints::default();
        hints.execution_surface = ActionExecutionSurface::Chat;
        hints.direct_user_intent = true;
        hints.caller_principal = caller.cloned();

        self.process_message_with_meta_and_hints(
            &resume_message,
            run.channel.as_deref().unwrap_or("web"),
            run.conversation_id.as_deref(),
            None,
            hints,
        )
        .await
    }

    pub(super) async fn run_inbound_security_precheck(
        &self,
        classification_message: &str,
        stored_user_message: &str,
        channel: &str,
        conversation_key: &str,
        is_new_conversation: bool,
        project_id: Option<&str>,
        user_message_already_recorded: bool,
    ) -> Result<InboundSecurityPrecheck> {
        // Abuse-tracker short-circuit: if this source is currently in
        // pending-approval or paused status from prior guard trips, decline
        // before any early command path can mutate state.
        let abuse_source = crate::security::abuse_tracker::SourceKey {
            channel_id: channel.to_string(),
            user_identity: None,
        };
        let abuse_tracker = crate::security::abuse_tracker::AbuseTracker::new(
            self.storage.db(),
            self.config.security.abuse_tracker.clone(),
        );
        match abuse_tracker.current_status(&abuse_source).await {
            Ok(status) if status.should_suppress_responses() => {
                let reply = match status {
                    crate::security::abuse_tracker::TrackerStatus::PendingApproval => {
                        "This channel is paused pending an operator review. Please wait - your administrator will decide whether to resume or pause further messages."
                    }
                    crate::security::abuse_tracker::TrackerStatus::Paused => {
                        "This channel has been paused by an operator. Please contact your administrator."
                    }
                    crate::security::abuse_tracker::TrackerStatus::Normal => unreachable!(),
                };
                let processed = self
                    .persist_immediate_exchange(
                        stored_user_message,
                        reply,
                        ImmediateExchangeContext {
                            channel,
                            conversation_key,
                            is_new_conversation,
                            project_id,
                            model_used: "security_guard",
                            user_message_already_recorded,
                            memory_capture_allowed: false,
                        },
                    )
                    .await?;
                return Ok(InboundSecurityPrecheck::Respond(processed));
            }
            Err(error) => {
                tracing::warn!(
                    target: "security.abuse",
                    channel = %channel,
                    error = %error,
                    "abuse_tracker status lookup failed; continuing with inbound guard"
                );
            }
            _ => {}
        }

        // Intent-based inbound guard. The classifier sees the already-redacted
        // storage form, then normalization removes unicode obfuscation controls.
        let normalized_for_guard = crate::security::normalize_for_analysis(classification_message);
        let pending_actions_for_guard = self.pending_conversation_actions(conversation_key).await;
        let trusted_prior_assistant_message = if pending_actions_for_guard.is_empty() {
            None
        } else {
            self.recent_trusted_assistant_message_for_inbound_guard(
                conversation_key,
                stored_user_message,
            )
            .await
        };
        let inbound_policy = crate::security::intent_classifier::default_policy();
        let mut inbound_candidates = self.llm_candidates_for_role(&ModelRole::Fast);
        if inbound_candidates.is_empty() {
            inbound_candidates.push(self.primary_llm_candidate());
        }
        let mut inbound_candidates = self
            .reorder_candidates_with_failover(inbound_candidates, Some(conversation_key))
            .await;
        if inbound_candidates.is_empty() {
            inbound_candidates.push(self.primary_llm_candidate());
        }
        let mut inbound_decision = None;
        for candidate in inbound_candidates.iter().take(2) {
            let decision = crate::security::intent_classifier::classify_inbound_with_metadata(
                &candidate.client,
                &inbound_policy,
                &normalized_for_guard,
                trusted_prior_assistant_message.as_deref(),
            )
            .await;
            if matches!(
                decision.verdict,
                crate::security::intent_classifier::IntentVerdict::RouterUnavailable { .. }
            ) {
                tracing::warn!(
                    target: "security.inbound",
                    slot_id = %candidate.slot_id,
                    slot_label = %candidate.slot_label,
                    "inbound intent classifier candidate returned no usable routing decision"
                );
                inbound_decision = Some(decision);
                continue;
            }
            inbound_decision = Some(decision);
            break;
        }
        let inbound_decision = inbound_decision.unwrap_or_else(|| {
            crate::security::intent_classifier::InboundClassificationDecision {
                verdict: crate::security::intent_classifier::IntentVerdict::RouterUnavailable {
                    reason: "no inbound classifier model candidates available".to_string(),
                },
                memory_capture: Default::default(),
                routing: Default::default(),
            }
        });
        let memory_capture_allowed = inbound_decision.memory_capture.should_capture;
        let routing = inbound_decision.routing.clone();

        match &inbound_decision.verdict {
            crate::security::intent_classifier::IntentVerdict::Block {
                message: safe_reply,
                rule_id,
                severity,
            } => {
                self.security_events.record_injection_attempt();
                tracing::warn!(
                    target: "security.inbound",
                    rule_id = %rule_id,
                    severity = severity,
                    channel = %channel,
                    "inbound intent classifier blocked message"
                );
                let source_label = inbound_security_source_label(channel);
                let alert_msg = format!(
                    "Security guard blocked a message from {} (rule {}).",
                    &source_label, rule_id
                );
                self.emit_notification("Security Alert", &alert_msg, "error", "security")
                    .await;
                self.notify_preferred_channel(&alert_msg).await;
                match abuse_tracker.record_trip(&abuse_source).await {
                    Ok(outcome) if outcome.newly_pending => {
                        let escalation = format!(
                            "Security escalation: {} reached {} guard trips in the configured window. Operator approval required to resume.",
                            &source_label, outcome.trip_count_in_window
                        );
                        self.emit_notification(
                            "Security approval required",
                            &escalation,
                            "error",
                            "security",
                        )
                        .await;
                        self.notify_preferred_channel(&escalation).await;
                    }
                    Ok(_) => {}
                    Err(error) => {
                        tracing::warn!(
                            target: "security.abuse",
                            channel = %channel,
                            error = %error,
                            "abuse_tracker.record_trip failed; block applied but escalation state not updated"
                        );
                    }
                }
                let processed = self
                    .persist_immediate_exchange(
                        stored_user_message,
                        safe_reply,
                        ImmediateExchangeContext {
                            channel,
                            conversation_key,
                            is_new_conversation,
                            project_id,
                            model_used: "security_guard",
                            user_message_already_recorded,
                            memory_capture_allowed: false,
                        },
                    )
                    .await?;
                Ok(InboundSecurityPrecheck::Respond(processed))
            }
            crate::security::intent_classifier::IntentVerdict::AllowWithUncheckedTag {
                reason,
                intent_kinds,
            } => {
                tracing::warn!(
                    target: "security.inbound",
                    reason = %reason,
                    channel = %channel,
                    "inbound intent classifier degraded; message passed with unchecked tag"
                );
                let _ = (reason, intent_kinds);
                Ok(InboundSecurityPrecheck::Continue {
                    memory_capture_allowed: false,
                    routing: Some(routing),
                })
            }
            crate::security::intent_classifier::IntentVerdict::RouterUnavailable { reason } => {
                tracing::warn!(
                    target: "security.inbound",
                    reason = %reason,
                    channel = %channel,
                    "inbound intent router unavailable; continuing without routing hints"
                );
                Ok(InboundSecurityPrecheck::Continue {
                    memory_capture_allowed: false,
                    routing: None,
                })
            }
            crate::security::intent_classifier::IntentVerdict::Allow => {
                Ok(InboundSecurityPrecheck::Continue {
                    memory_capture_allowed,
                    routing: Some(routing),
                })
            }
        }
    }

    pub(super) async fn persist_immediate_exchange(
        &self,
        message: &str,
        response: &str,
        context: ImmediateExchangeContext<'_>,
    ) -> Result<ProcessedMessage> {
        let trace_id = uuid::Uuid::new_v4().to_string();
        let trace_time = chrono::Utc::now();
        let filtered_response = self.security.filter_output(response);
        if !filtered_response.redactions.is_empty() {
            tracing::warn!(
                "Security: redacted sensitive data from immediate output before persistence ({} rule match(es))",
                filtered_response.redactions.len()
            );
        }
        let safe_response = filtered_response.text;
        let trace_ref = Arc::new(RwLock::new(ExecutionTrace {
            id: trace_id.clone(),
            message: message.to_string(),
            channel: context.channel.to_string(),
            started_at: Some(trace_time),
            completed_at: Some(trace_time),
            steps: vec![
                ExecutionStep {
                    icon: "[fast]".to_string(),
                    title: "Message Received".to_string(),
                    detail: format!(
                        "Immediate reply path | Channel: {} | Length: {} chars",
                        context.channel,
                        message.chars().count()
                    ),
                    step_type: "info".to_string(),
                    data: None,
                    timestamp: trace_time,
                    duration_ms: Some(0),
                },
                ExecutionStep {
                    icon: "[reply]".to_string(),
                    title: "Immediate Response".to_string(),
                    detail: format!(
                        "Returned without the full tool loop using {}.",
                        context.model_used
                    ),
                    step_type: "success".to_string(),
                    data: Some(safe_truncate(&safe_response, 8000)),
                    timestamp: trace_time,
                    duration_ms: Some(0),
                },
            ],
            proof_id: None,
            response: Some(safe_response.clone()),
            model: Some(context.model_used.to_string()),
            input_tokens: 0,
            output_tokens: 0,
            total_tokens: 0,
            cost_usd: 0.0,
            complexity: Some("immediate".to_string()),
            plan: None,
        }));
        self.seed_execution_trace_snapshot(&trace_ref).await;
        self.log_operational_event(operational::OperationalEvent {
            event_type: "agent_request",
            channel: context.channel,
            success: true,
            outcome: "started",
            trace_id: Some(&trace_id),
            conversation_id: Some(context.conversation_key),
            tool_name: None,
            latency_ms: Some(0),
            arguments: None,
            payload: Some(&serde_json::json!({
                "flow_kind": "immediate",
                "message_chars": message.chars().count(),
                "resumed": context.user_message_already_recorded,
            })),
            strategy_version: None,
            policy_version: None,
            prompt_version: None,
            classifier_prompt_version: None,
            specialist_prompt_version: None,
            model_slot: Some(context.model_used),
        })
        .await;
        tracing::info!(
            "Request started: trace={} channel={} flow=immediate resumed={}",
            trace_id,
            context.channel,
            context.user_message_already_recorded
        );
        self.log_operational_event(operational::OperationalEvent {
            event_type: "response_complete",
            channel: context.channel,
            success: true,
            outcome: "completed",
            trace_id: Some(&trace_id),
            conversation_id: Some(context.conversation_key),
            tool_name: None,
            latency_ms: Some(0),
            arguments: None,
            payload: Some(&serde_json::json!({
                "response_chars": safe_response.chars().count(),
                "tool_calls": 0,
                "degradation_notes": 0,
                "status": "completed",
            })),
            strategy_version: None,
            policy_version: None,
            prompt_version: None,
            classifier_prompt_version: None,
            specialist_prompt_version: None,
            model_slot: Some(context.model_used),
        })
        .await;
        tracing::info!(
            "Request completed: trace={} channel={} status=completed duration=0ms tools=0",
            trace_id,
            context.channel
        );
        self.persist_completed_trace(&trace_ref).await;

        // Mirror normal chat persistence path for immediate shortcut responses.
        {
            let mut history = self.conversation_history.write().await;
            let conversation_history = history
                .entry(context.conversation_key.to_string())
                .or_insert_with(Vec::new);
            if !context.user_message_already_recorded {
                conversation_history.push(ConversationMessage {
                    role: "user".to_string(),
                    content: message.to_string(),
                    _timestamp: chrono::Utc::now(),
                });
            }
            conversation_history.push(ConversationMessage {
                role: "assistant".to_string(),
                content: safe_response.clone(),
                _timestamp: chrono::Utc::now(),
            });
            if conversation_history.len() > 10 {
                conversation_history.drain(0..conversation_history.len() - 10);
            }
        }

        let mut conversation_title: Option<String> = None;
        if !context.conversation_key.is_empty() {
            if !context.user_message_already_recorded {
                let now = chrono::Utc::now().to_rfc3339();
                let user_msg = crate::storage::entities::message::Model {
                    id: uuid::Uuid::new_v4().to_string(),
                    conversation_id: context.conversation_key.to_string(),
                    role: "user".to_string(),
                    content: message.to_string(),
                    timestamp: now.clone(),
                    model_used: None,
                    trace_id: Some(trace_id.clone()),
                };
                if let Err(e) = self
                    .encrypted_storage
                    .insert_message_encrypted(&user_msg)
                    .await
                {
                    tracing::warn!("Failed to persist immediate-path user message: {}", e);
                }
                if context.memory_capture_allowed {
                    self.spawn_user_memory_capture(
                        message,
                        context.channel,
                        Some(context.conversation_key),
                        context.project_id,
                        Some(&user_msg.id),
                    );
                }
            }

            let asst_msg = crate::storage::entities::message::Model {
                id: uuid::Uuid::new_v4().to_string(),
                conversation_id: context.conversation_key.to_string(),
                role: "assistant".to_string(),
                content: safe_response.clone(),
                timestamp: chrono::Utc::now().to_rfc3339(),
                model_used: Some(context.model_used.to_string()),
                trace_id: Some(trace_id.clone()),
            };
            if let Err(e) = self
                .encrypted_storage
                .insert_message_encrypted(&asst_msg)
                .await
            {
                tracing::warn!("Failed to persist immediate-path assistant message: {}", e);
            }

            if context.is_new_conversation {
                let title = self.generate_conversation_title(message);
                let _ = self
                    .storage
                    .update_conversation(context.conversation_key, Some(&title), Some(2), None)
                    .await;
                *self.last_conversation_title.write().await = Some(title.clone());
                conversation_title = Some(title);
            } else {
                *self.last_conversation_title.write().await = None;
            }
        }

        *self.last_conversation_id.write().await = Some(context.conversation_key.to_string());
        if !context.conversation_key.is_empty() {
            self.sync_background_session_after_response(
                context.conversation_key,
                message,
                &safe_response,
            )
            .await;
        }

        let user_outcome = self
            .build_response_heuristic_outcome(&safe_response, &[], &[], None)
            .unwrap_or_else(|| {
                self.execution_supervisor
                    .build_success_outcome(&safe_response, &[], &[])
            });
        if !context.conversation_key.is_empty() {
            self.sync_pending_resilience_followup(
                context.conversation_key,
                message,
                context.channel,
                context.project_id,
                &user_outcome,
            )
            .await;
        }
        let run_status = Self::execution_run_status_for_outcome(&user_outcome);

        Ok(ProcessedMessage {
            response: safe_response,
            conversation_id: Some(context.conversation_key.to_string()),
            conversation_title,
            run_id: None,
            run_status: Some(run_status.as_str().to_string()),
            trace_id: None,
            total_tokens: 0,
            choices: Vec::new(),
            degradation: Vec::new(),
            attempted_models: Vec::new(),
            user_outcome: Some(user_outcome),
            trace_steps: Vec::new(),
            turn_records: Vec::new(),
            turn_plan: None,
        })
    }

    pub(crate) async fn persist_completed_trace(&self, trace_ref: &Arc<RwLock<ExecutionTrace>>) {
        let trace_snapshot = trace_ref.read().await.clone();
        if trace_snapshot.id.trim().is_empty() {
            return;
        }

        {
            let mut history = self.trace_history.write().await;
            history.retain(|item| item.id != trace_snapshot.id);
            history.insert(0, trace_snapshot.clone());
            if history.len() > 100 {
                history.truncate(100);
            }
        }

        if let Err(e) = self
            .encrypted_storage
            .insert_execution_trace_encrypted(&trace_snapshot)
            .await
        {
            tracing::warn!(
                "Failed to persist execution trace '{}': {}",
                trace_snapshot.id,
                e
            );
        }

        let observability_endpoint = crate::core::observability::normalize_observability_endpoint(
            &self.config.observability.provider,
            &self.config.observability.endpoint,
        );
        let observability_ready = self.config.observability.enabled
            && !observability_endpoint.is_empty()
            && crate::core::observability::has_observability_auth_token(
                &self.config_dir,
                Some(&self.data_dir),
            )
            .unwrap_or(false);
        if observability_ready {
            let provider = crate::core::observability::normalize_observability_provider(
                &self.config.observability.provider,
            );
            match crate::core::observability::export_execution_trace(
                &self.config,
                &self.config_dir,
                &self.data_dir,
                &self.storage,
                &trace_snapshot,
                "trace_completed",
            )
            .await
            {
                Ok(()) => {
                    tracing::info!(
                        "Observability: exported trace '{}' to {}",
                        trace_snapshot.id,
                        provider
                    );
                }
                Err(e) => {
                    tracing::warn!(
                        "Observability: export failed for trace '{}' to {}: {}",
                        trace_snapshot.id,
                        provider,
                        e
                    );
                }
            }
        }

        if !Arc::ptr_eq(trace_ref, &self.last_trace) {
            *self.last_trace.write().await = trace_snapshot;
        }

        // Self-tune: track interaction for adaptive learning
        crate::core::self_tune::on_interaction_completed(
            &self.storage,
            &self.encrypted_storage,
            &self.llm,
        )
        .await;
    }
}
