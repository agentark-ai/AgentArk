use super::*;

const INBOUND_CLASSIFIER_RECENT_ARTIFACTS: usize = 4;
const DEFERRED_CHAT_PERSISTENCE_MAX_CONCURRENCY: usize = 8;
const DEFERRED_CHAT_PERSISTENCE_ATTEMPTS: usize = 3;
const DEFERRED_CHAT_PERSISTENCE_ATTEMPT_TIMEOUT_SECS: u64 = 45;
const DEFERRED_CHAT_PERSISTENCE_WARN_PENDING: usize = 64;
const TURN_TIMING_SLOW_STAGE_WARN_MS: u64 = 1_000;
const DEFAULT_INBOUND_EMBEDDING_FASTPATH_GRACE_MS: u64 = 900;
const EXPERIENCE_RUN_REQUEST_TEXT_CHARS: usize = 4_000;
const EXPERIENCE_RUN_OUTCOME_SUMMARY_CHARS: usize = 2_000;

static DEFERRED_CHAT_PERSISTENCE_SEMAPHORE: once_cell::sync::Lazy<Arc<tokio::sync::Semaphore>> =
    once_cell::sync::Lazy::new(|| {
        Arc::new(tokio::sync::Semaphore::new(
            DEFERRED_CHAT_PERSISTENCE_MAX_CONCURRENCY,
        ))
    });
static DEFERRED_CHAT_PERSISTENCE_PENDING: std::sync::atomic::AtomicUsize =
    std::sync::atomic::AtomicUsize::new(0);

fn elapsed_ms(started: std::time::Instant) -> u64 {
    started.elapsed().as_millis().min(u128::from(u64::MAX)) as u64
}

fn inbound_embedding_fastpath_grace_ms() -> u64 {
    std::env::var("AGENTARK_INBOUND_EMBEDDING_FASTPATH_GRACE_MS")
        .ok()
        .and_then(|value| value.trim().parse::<u64>().ok())
        .unwrap_or(DEFAULT_INBOUND_EMBEDDING_FASTPATH_GRACE_MS)
        .clamp(100, 5_000)
}

fn emit_inbound_precheck_progress(
    stream_tx: Option<&tokio::sync::mpsc::Sender<StreamEvent>>,
    stage: &str,
    content: impl Into<String>,
    data: serde_json::Value,
) {
    let Some(tx) = stream_tx else {
        return;
    };
    let mut payload = serde_json::Map::new();
    payload.insert(
        "kind".to_string(),
        serde_json::Value::String("inbound_precheck".to_string()),
    );
    payload.insert(
        "stage".to_string(),
        serde_json::Value::String(stage.to_string()),
    );
    payload.insert(
        "status".to_string(),
        serde_json::Value::String("progress".to_string()),
    );
    if !data.is_null() {
        if let Some(object) = data.as_object() {
            for (key, value) in object {
                if !payload.contains_key(key) {
                    payload.insert(key.clone(), value.clone());
                }
            }
        }
        payload.insert("details".to_string(), data);
    }
    queue_stream_event(
        tx,
        StreamEvent::ToolProgress {
            name: "inbound_precheck".to_string(),
            content: content.into(),
            payload: Some(serde_json::Value::Object(payload)),
        },
    );
}

fn log_turn_timing_stage(
    turn_timing_id: &str,
    conversation_id: &str,
    channel: &str,
    stage: &str,
    duration_ms: u64,
    success: bool,
    warn_after_ms: u64,
) {
    tracing::debug!(
        target: "agentark.turn_timing",
        turn_timing_id = %turn_timing_id,
        conversation_id = %conversation_id,
        channel = %channel,
        stage = %stage,
        duration_ms,
        success,
        "turn timing stage"
    );
    if duration_ms >= warn_after_ms {
        tracing::debug!(
            target: "agentark.turn_timing",
            turn_timing_id = %turn_timing_id,
            conversation_id = %conversation_id,
            channel = %channel,
            stage = %stage,
            duration_ms,
            warn_after_ms,
            "slow turn timing stage"
        );
    }
}

fn log_turn_timing_instant(
    turn_timing_id: &str,
    conversation_id: &str,
    channel: &str,
    stage: &str,
    started: std::time::Instant,
    success: bool,
    warn_after_ms: u64,
) {
    log_turn_timing_stage(
        turn_timing_id,
        conversation_id,
        channel,
        stage,
        elapsed_ms(started),
        success,
        warn_after_ms,
    );
}

fn redact_chat_message_for_storage(secret_scrubbed_message: &str) -> String {
    let mut redactor = crate::security::pii::PiiRedactor::new();
    redactor.redact_emails = false;
    redactor.redact_phones = false;
    redactor.redact_ips = false;
    redactor.redact(secret_scrubbed_message)
}

fn normalize_request_project_id(project_id: Option<&str>) -> Option<&str> {
    project_id.map(str::trim).filter(|value| !value.is_empty())
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum ConversationControlCommand {
    New,
    Clear,
}

fn parse_conversation_control_command(message: &str) -> Option<ConversationControlCommand> {
    let mut parts = message.split_whitespace();
    let command = parts.next()?;
    if parts.next().is_some() {
        return None;
    }
    match command {
        "/new" | "\\new" => Some(ConversationControlCommand::New),
        "/clear" | "\\clear" => Some(ConversationControlCommand::Clear),
        _ => None,
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn request_project_id_is_trimmed_not_dropped() {
        assert_eq!(
            normalize_request_project_id(Some(" project-1 ")),
            Some("project-1")
        );
        assert_eq!(normalize_request_project_id(Some("   ")), None);
        assert_eq!(normalize_request_project_id(None), None);
    }

    #[test]
    fn turn_pipeline_usage_delta_includes_prompt_cache_usage() {
        let before = TurnPipelineUsageSnapshot {
            input_tokens: 10,
            output_tokens: 4,
            total_tokens: 14,
            cached_prompt_tokens: 3,
            cache_creation_prompt_tokens: 2,
            cost_usd: 0.01,
        };
        let after = TurnPipelineUsageSnapshot {
            input_tokens: 25,
            output_tokens: 9,
            total_tokens: 34,
            cached_prompt_tokens: 11,
            cache_creation_prompt_tokens: 7,
            cost_usd: 0.03,
        };

        let delta = after.delta_since(before);

        assert_eq!(delta.input_tokens, 15);
        assert_eq!(delta.output_tokens, 5);
        assert_eq!(delta.total_tokens, 20);
        assert_eq!(delta.cached_prompt_tokens, 8);
        assert_eq!(delta.cache_creation_prompt_tokens, 5);
        assert!((delta.cost_usd - 0.02).abs() < f64::EPSILON);
    }
}

#[derive(Clone, Copy, Debug, Default, serde::Serialize)]
struct TurnPipelineUsageSnapshot {
    input_tokens: i64,
    output_tokens: i64,
    total_tokens: i64,
    cached_prompt_tokens: i64,
    cache_creation_prompt_tokens: i64,
    cost_usd: f64,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum DeferredExchangePersistenceKind {
    TurnPipeline,
    Immediate,
}

#[derive(Clone)]
struct DeferredExchangePersistence {
    kind: DeferredExchangePersistenceKind,
    trace_snapshot: ExecutionTrace,
    execution_run_id: Option<String>,
    message: String,
    response: String,
    run_status: String,
    channel: String,
    conversation_key: String,
    project_id: Option<String>,
    model_used: String,
    user_message_already_recorded: bool,
    recorded_user_message_id: Option<String>,
    memory_capture_allowed: bool,
    memory_capture_source: Option<String>,
    user_message_for_link_capture: Option<String>,
    user_message_id: String,
    assistant_message_id: String,
    user_timestamp: String,
    assistant_timestamp: String,
    is_new_conversation: bool,
    conversation_title: Option<String>,
    user_outcome: crate::core::UserFacingOutcome,
    choices: Vec<ClarificationChoice>,
}

fn trace_duration_ms(trace: &ExecutionTrace) -> Option<u64> {
    trace.started_at.and_then(|start| {
        trace
            .completed_at
            .map(|end| (end - start).num_milliseconds().max(0) as u64)
    })
}

fn operational_success_for_run_status(status: &str) -> bool {
    matches!(
        status.trim(),
        "completed" | "completed_degraded" | "degraded"
    )
}

fn experience_run_success_state(outcome: &crate::core::UserFacingOutcome) -> &'static str {
    match outcome.status {
        crate::core::UserFacingOutcomeStatus::Complete
        | crate::core::UserFacingOutcomeStatus::Degraded => "provisional",
        crate::core::UserFacingOutcomeStatus::NeedsClarification
        | crate::core::UserFacingOutcomeStatus::NeedsPermission
        | crate::core::UserFacingOutcomeStatus::NeedsIntegration
        | crate::core::UserFacingOutcomeStatus::NeedsCredentials
        | crate::core::UserFacingOutcomeStatus::NeedsStrongerModel
        | crate::core::UserFacingOutcomeStatus::ServiceUnavailable => "failed",
    }
}

fn experience_run_failure_reason(
    run_status: &str,
    outcome: &crate::core::UserFacingOutcome,
) -> Option<String> {
    if experience_run_success_state(outcome) != "failed" {
        return None;
    }
    outcome
        .reason_code
        .as_ref()
        .map(|reason| reason.trim())
        .filter(|reason| !reason.is_empty())
        .map(|reason| safe_truncate(reason, 240))
        .or_else(|| {
            outcome
                .degradation
                .first()
                .map(|note| safe_truncate(&note.summary, 240))
        })
        .or_else(|| Some(safe_truncate(run_status, 120)))
}

fn experience_tool_history_from_trace(trace: &ExecutionTrace) -> Vec<serde_json::Value> {
    let mut history = Vec::new();
    for step in &trace.steps {
        let Some(data) = step.data.as_deref() else {
            continue;
        };
        let Ok(value) = serde_json::from_str::<serde_json::Value>(data) else {
            continue;
        };
        let Some(entries) = value
            .get("tool_history_json")
            .or_else(|| value.get("tool_history"))
            .and_then(|value| value.as_array())
        else {
            continue;
        };
        history.extend(entries.iter().cloned());
    }
    history
}

fn experience_tool_names(tool_history: &[serde_json::Value]) -> Vec<String> {
    tool_history
        .iter()
        .filter_map(|entry| entry.get("tool").and_then(|value| value.as_str()))
        .map(str::trim)
        .filter(|name| !name.is_empty())
        .map(str::to_string)
        .collect()
}

fn experience_tool_batch_events(tool_history: &[serde_json::Value]) -> Vec<serde_json::Value> {
    tool_history
        .iter()
        .filter(|entry| {
            entry
                .get("status")
                .and_then(|value| value.as_str())
                .is_some_and(|status| status == "tool_batch")
        })
        .filter_map(|entry| entry.get("batch").cloned())
        .collect()
}

fn experience_tool_entry_status(entry: &serde_json::Value) -> &'static str {
    let Some(result) = entry.get("result") else {
        return "unknown";
    };
    if super::tool_responses::structured_tool_value_reports_degenerate_output(result) {
        return "degenerate";
    }
    match super::tool_responses::structured_tool_value_outcome(result).map(|report| report.state) {
        Some(super::tool_responses::StructuredToolOutcomeState::Success) => "success",
        Some(super::tool_responses::StructuredToolOutcomeState::NeedsInput) => "needs_input",
        Some(super::tool_responses::StructuredToolOutcomeState::Failure) => "failed",
        None => "unknown",
    }
}

fn experience_tool_sequence_json(tool_history: &[serde_json::Value]) -> serde_json::Value {
    serde_json::Value::Array(
        tool_history
            .iter()
            .filter_map(|entry| {
                let name = entry
                    .get("tool")
                    .and_then(|value| value.as_str())
                    .map(str::trim)
                    .filter(|name| !name.is_empty())?;
                Some(serde_json::json!({
                    "name": name,
                    "status": experience_tool_entry_status(entry),
                    "started_at": serde_json::Value::Null,
                    "completed_at": serde_json::Value::Null,
                }))
            })
            .collect(),
    )
}

fn experience_tool_sequence_digest(tool_names: &[String]) -> Option<String> {
    if tool_names.is_empty() {
        return None;
    }
    let mut hasher = Sha256::new();
    hasher.update(tool_names.join("|").as_bytes());
    Some(hex::encode(hasher.finalize()))
}

fn experience_intent_key(plan: Option<&ExecutionPlan>, tool_names: &[String]) -> String {
    let mut keys = plan
        .map(|plan| {
            plan.steps
                .iter()
                .filter_map(|step| {
                    step.action
                        .as_deref()
                        .or(step.tool_hint.as_deref())
                        .map(str::trim)
                        .filter(|value| !value.is_empty())
                        .map(str::to_string)
                })
                .collect::<Vec<_>>()
        })
        .unwrap_or_default();
    if keys.is_empty() {
        keys = tool_names.to_vec();
    }
    if keys.is_empty() {
        "chat".to_string()
    } else {
        keys.dedup();
        safe_truncate(&keys.join("+"), 240)
    }
}

fn experience_repair_decisions(tool_history: &[serde_json::Value]) -> Vec<serde_json::Value> {
    tool_history
        .iter()
        .filter_map(|entry| {
            let action = entry.get("action")?;
            let reason = entry.get("reason")?;
            Some(serde_json::json!({
                "action": action,
                "reason": reason,
                "alternative_action": entry.get("alternative_action").cloned().unwrap_or(serde_json::Value::Null),
                "run_status": entry.get("run_status").cloned().unwrap_or(serde_json::Value::Null),
            }))
        })
        .collect()
}

fn experience_run_scope(project_id: Option<&str>, conversation_id: Option<&str>) -> String {
    if conversation_id
        .map(str::trim)
        .filter(|value| !value.is_empty())
        .is_some()
    {
        return "conversation".to_string();
    }
    if project_id
        .map(str::trim)
        .filter(|value| !value.is_empty())
        .is_some()
    {
        "project".to_string()
    } else {
        "global".to_string()
    }
}

impl TurnPipelineUsageSnapshot {
    fn delta_since(self, previous: Self) -> Self {
        Self {
            input_tokens: self.input_tokens.saturating_sub(previous.input_tokens),
            output_tokens: self.output_tokens.saturating_sub(previous.output_tokens),
            total_tokens: self.total_tokens.saturating_sub(previous.total_tokens),
            cached_prompt_tokens: self
                .cached_prompt_tokens
                .saturating_sub(previous.cached_prompt_tokens),
            cache_creation_prompt_tokens: self
                .cache_creation_prompt_tokens
                .saturating_sub(previous.cache_creation_prompt_tokens),
            cost_usd: (self.cost_usd - previous.cost_usd).max(0.0),
        }
    }
}

impl Agent {
    pub(super) fn trim_in_memory_conversation_history(
        &self,
        messages: &mut Vec<ConversationMessage>,
    ) {
        let budget = self.conversation_history_budget("");
        let message_token_budget = Self::chat_message_token_budget(budget);
        let mut used_tokens = 0usize;
        let mut keep_start = messages.len();
        for (idx, message) in messages.iter().enumerate().rev() {
            let message_tokens =
                Self::conversation_message_token_estimate(message, message_token_budget);
            if keep_start < messages.len()
                && used_tokens.saturating_add(message_tokens) > budget.history_tokens
            {
                break;
            }
            used_tokens = used_tokens.saturating_add(message_tokens);
            keep_start = idx;
        }
        if keep_start > 0 && keep_start < messages.len() {
            messages.drain(0..keep_start);
        }
    }

    async fn respond_if_abuse_tracker_suppressed(
        &self,
        channel: &str,
        stored_user_message: &str,
        conversation_key: &str,
        is_new_conversation: bool,
        project_id: Option<&str>,
        user_message_already_recorded: bool,
        turn_timing_id: &str,
    ) -> Result<Option<ProcessedMessage>> {
        let abuse_source = crate::security::abuse_tracker::SourceKey {
            channel_id: channel.to_string(),
            user_identity: None,
        };
        let abuse_tracker = crate::security::abuse_tracker::AbuseTracker::new(
            self.storage.db(),
            self.config.security.abuse_tracker.clone(),
        );
        let stage_started = std::time::Instant::now();
        match abuse_tracker.current_status(&abuse_source).await {
            Ok(status) if status.should_suppress_responses() => {
                log_turn_timing_instant(
                    turn_timing_id,
                    conversation_key,
                    channel,
                    "inbound_abuse_status_lookup",
                    stage_started,
                    true,
                    TURN_TIMING_SLOW_STAGE_WARN_MS,
                );
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
                            recorded_user_message_id: None,
                            memory_capture_allowed: false,
                            memory_capture_source: None,
                            user_message_for_link_capture: Some(stored_user_message),
                        },
                    )
                    .await?;
                Ok(Some(processed))
            }
            Err(error) => {
                tracing::warn!(
                    target: "security.abuse",
                    channel = %channel,
                    error = %error,
                    "abuse_tracker status lookup failed; continuing with inbound guard"
                );
                Ok(None)
            }
            _ => Ok(None),
        }
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

    pub async fn process_message_prerecorded_with_meta_and_hints(
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
            true,
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
            Ok(processed) => Ok(processed),
            Err(error) => Err(error),
        }
    }

    #[expect(
        clippy::too_many_arguments,
        reason = "Public streaming API preserves existing call sites"
    )]
    pub async fn process_message_stream_prerecorded_with_meta_and_hints(
        &self,
        message: &str,
        channel: &str,
        conversation_id: Option<&str>,
        project_id: Option<&str>,
        _trace_override: Arc<RwLock<ExecutionTrace>>,
        token_tx: tokio::sync::mpsc::Sender<StreamEvent>,
        request_hints: RequestExecutionHints,
    ) -> Result<ProcessedMessage> {
        match self
            .process_turn_request(
                message,
                channel,
                conversation_id,
                project_id,
                request_hints,
                true,
                false,
                Some(token_tx.clone()),
            )
            .await
        {
            Ok(processed) => Ok(processed),
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
            Ok(processed) => Ok(processed),
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
        let process_started = std::time::Instant::now();
        let project_id = normalize_request_project_id(project_id);
        let _active_request = self.track_active_message_request();
        *self.last_activity.write().await = Some(chrono::Utc::now());

        let secret_redaction = crate::security::redact_secret_input(message);
        if secret_redaction.had_secret() {
            tracing::warn!(
                "Security: redacted likely secret input from channel={} ({} match(es))",
                channel,
                secret_redaction.redactions.len()
            );
        }
        let message_storage = redact_chat_message_for_storage(&secret_redaction.text);
        let early_safe_message = message_storage.clone();
        if !matches!(channel, "http" | "web") {
            if let (Some(request_conversation_id), Some(command)) = (
                conversation_id,
                parse_conversation_control_command(&message_storage),
            ) {
                let (response, new_conversation_id) = match command {
                    ConversationControlCommand::New => {
                        let new_id = self
                            .start_new_channel_conversation(
                                channel,
                                request_conversation_id,
                                project_id,
                                "New Chat",
                            )
                            .await?;
                        (
                            "Started a new conversation. Previous history is kept.".to_string(),
                            new_id,
                        )
                    }
                    ConversationControlCommand::Clear => {
                        let new_id = self
                            .clear_current_channel_conversation(
                                channel,
                                request_conversation_id,
                                project_id,
                            )
                            .await?;
                        ("Conversation cleared. Starting fresh.".to_string(), new_id)
                    }
                };
                return Ok(ProcessedMessage {
                    response,
                    conversation_id: Some(new_conversation_id),
                    conversation_title: None,
                    run_id: None,
                    run_status: Some(
                        crate::core::ExecutionRunStatus::Completed
                            .as_str()
                            .to_string(),
                    ),
                    trace_id: None,
                    input_tokens: 0,
                    output_tokens: 0,
                    total_tokens: 0,
                    cached_prompt_tokens: 0,
                    cache_creation_prompt_tokens: 0,
                    choices: Vec::new(),
                    degradation: Vec::new(),
                    attempted_models: Vec::new(),
                    user_outcome: None,
                    trace_steps: Vec::new(),
                    turn_records: Vec::new(),
                    turn_plan: None,
                });
            }
        }
        let (resolved_conversation_id, is_new_conversation) = self
            .resolve_conversation_id(channel, conversation_id, project_id, &early_safe_message)
            .await?;
        let conversation_key = resolved_conversation_id.clone();
        let turn_timing_id = request_hints
            .turn_timing_id
            .get_or_insert_with(|| uuid::Uuid::new_v4().to_string())
            .clone();
        tracing::debug!(
            target: "agentark.turn_timing",
            turn_timing_id = %turn_timing_id,
            conversation_id = %conversation_key,
            channel = %channel,
            message_chars = message_storage.chars().count(),
            user_message_already_recorded,
            skip_inbound_security_precheck,
            "turn timing start"
        );
        log_turn_timing_instant(
            &turn_timing_id,
            &conversation_key,
            channel,
            "resolve_conversation",
            process_started,
            true,
            TURN_TIMING_SLOW_STAGE_WARN_MS,
        );
        emit_inbound_precheck_progress(
            stream_tx.as_ref(),
            "request_context",
            "Preparing request context...",
            serde_json::json!({
                "conversation_id": conversation_key,
                "skip_inbound_security_precheck": skip_inbound_security_precheck,
            }),
        );

        let turn_started_at = chrono::Utc::now();
        let usage_before_turn = self.turn_pipeline_usage_snapshot().await;
        if !skip_inbound_security_precheck {
            if self
                .pending_chat_credential_prompt(&conversation_key)
                .await
                .is_some()
            {
                let safe_reply = "A secure credential form is open for this chat. I can't accept credential values through normal chat. Use the secure form shown in the conversation or the integration's Settings page.";
                let processed = self
                    .persist_immediate_exchange(
                        &message_storage,
                        safe_reply,
                        ImmediateExchangeContext {
                            channel,
                            conversation_key: &conversation_key,
                            is_new_conversation,
                            project_id,
                            model_used: "secure_credential_prompt_guard",
                            user_message_already_recorded,
                            recorded_user_message_id: request_hints
                                .recorded_user_message_id
                                .clone(),
                            memory_capture_allowed: false,
                            memory_capture_source: None,
                            user_message_for_link_capture: Some(message_storage.as_str()),
                        },
                    )
                    .await?;
                return Ok(processed);
            }

            if secret_redaction.had_secret() {
                if let Some(processed) = self
                    .respond_if_abuse_tracker_suppressed(
                        channel,
                        &message_storage,
                        &conversation_key,
                        is_new_conversation,
                        project_id,
                        user_message_already_recorded,
                        &turn_timing_id,
                    )
                    .await?
                {
                    return Ok(processed);
                }
                let pending_chat_credential_prompt =
                    self.pending_chat_credential_prompt(&conversation_key).await;
                let safe_reply = if pending_chat_credential_prompt.is_some() {
                    "I redacted the secret from this chat message. Submit credentials through the secure credential form instead; I can't use, test, or save secrets pasted into normal chat."
                } else {
                    "I redacted the secret from this chat message. I can't use, test, or save secrets pasted into normal chat. Add credentials through the secure Settings or integration credential flow, then tell me what you want to do."
                };
                let processed = self
                    .persist_immediate_exchange(
                        &message_storage,
                        safe_reply,
                        ImmediateExchangeContext {
                            channel,
                            conversation_key: &conversation_key,
                            is_new_conversation,
                            project_id,
                            model_used: "secret_input_guard",
                            user_message_already_recorded,
                            recorded_user_message_id: request_hints
                                .recorded_user_message_id
                                .clone(),
                            memory_capture_allowed: false,
                            memory_capture_source: None,
                            user_message_for_link_capture: Some(message_storage.as_str()),
                        },
                    )
                    .await?;
                return Ok(processed);
            }

            if let Some(tx) = stream_tx.as_ref() {
                queue_stream_event(
                    tx,
                    StreamEvent::Thinking("Reviewing request intent...".to_string()),
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
                    &turn_timing_id,
                    stream_tx.as_ref(),
                )
                .await?
            {
                InboundSecurityPrecheck::Respond(processed) => return Ok(processed),
                InboundSecurityPrecheck::Continue => {}
            }
        }

        if is_new_conversation && !user_message_already_recorded && !conversation_key.is_empty() {
            self.ensure_conversation_row_for_turn(
                &conversation_key,
                channel,
                project_id,
                message_storage.as_str(),
                None,
            )
            .await?;
        }

        if request_hints.recent_actionable_artifacts.is_empty() && !is_new_conversation {
            request_hints.recent_actionable_artifacts = Self::conversation_artifacts_for_prompt(
                &self.load_recent_artifact_contexts(&conversation_key).await,
                INBOUND_CLASSIFIER_RECENT_ARTIFACTS,
            );
        }

        match self
            .run_model_routed_spine_for_chat(
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
                        model_used: "model_routed_spine",
                        user_message_already_recorded,
                        recorded_user_message_id: request_hints.recorded_user_message_id.clone(),
                        memory_capture_allowed: true,
                        memory_capture_source: None,
                        user_message_for_link_capture: Some(message_storage.as_str()),
                    },
                    processed.run_status.as_deref().unwrap_or("completed"),
                    processed.trace_steps.clone(),
                    processed.turn_records.clone(),
                    processed.turn_plan.clone(),
                    turn_started_at,
                    usage_delta,
                    processed.choices.clone(),
                )
                .await
            }
            Err(error) => {
                if error.to_string() == "Conversation not found" {
                    return Err(error);
                }
                tracing::warn!(
                    "Model-routed spine failed on channel '{}': {}",
                    channel,
                    error
                );
                let response = format!(
                    "The model-routed spine hit a framework-level failure before execution could complete, so I did not run any action. Please retry after checking the server logs. Error: {}",
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
                        model_used: "model_routed_spine_failed",
                        user_message_already_recorded,
                        recorded_user_message_id: request_hints.recorded_user_message_id.clone(),
                        memory_capture_allowed: true,
                        memory_capture_source: None,
                        user_message_for_link_capture: Some(message_storage.as_str()),
                    },
                    crate::core::ExecutionRunStatus::PlatformFailed.as_str(),
                    Vec::new(),
                    Vec::new(),
                    None,
                    turn_started_at,
                    usage_delta,
                    Vec::new(),
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
            cached_prompt_tokens: trace.cached_prompt_tokens,
            cache_creation_prompt_tokens: trace.cache_creation_prompt_tokens,
            cost_usd: trace.cost_usd,
        }
    }

    async fn ensure_conversation_row_for_turn(
        &self,
        conversation_id: &str,
        channel: &str,
        project_id: Option<&str>,
        message_preview: &str,
        conversation_title: Option<&str>,
    ) -> Result<()> {
        let conversation_id = conversation_id.trim();
        if conversation_id.is_empty() {
            return Ok(());
        }

        let now = chrono::Utc::now().to_rfc3339();
        let title = conversation_title
            .map(str::trim)
            .filter(|value| !value.is_empty())
            .map(|value| safe_truncate(value, 80))
            .unwrap_or_else(|| safe_truncate(message_preview, 50));
        let conv = crate::storage::entities::conversation::Model {
            id: conversation_id.to_string(),
            title,
            channel: channel.to_string(),
            project_id: project_id.map(str::to_string),
            created_at: now.clone(),
            updated_at: now,
            message_count: 0,
            archived: false,
            starred: false,
        };
        self.storage.create_conversation_if_absent(&conv).await?;
        Ok(())
    }

    async fn remember_completed_trace_snapshot(
        &self,
        trace_snapshot: ExecutionTrace,
        update_last_trace: bool,
    ) {
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

        if update_last_trace {
            *self.last_trace.write().await = trace_snapshot;
        }
    }

    async fn load_tool_strategy_profile_by_key(
        &self,
        key: &str,
    ) -> Option<crate::core::self_evolve::strategy_runtime::ToolStrategyProfile> {
        let raw = self.storage.get(key).await.ok().flatten()?;
        serde_json::from_slice::<crate::core::self_evolve::strategy_runtime::ToolStrategyProfile>(
            &raw,
        )
        .ok()
    }

    async fn active_tool_strategy_version_for_message(&self, message: &str) -> Option<String> {
        let mut selected = self
            .load_tool_strategy_profile_by_key(
                crate::core::self_evolve::strategy_runtime::TOOL_STRATEGY_PROFILE_KEY,
            )
            .await
            .unwrap_or_default();
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
                        &Self::prompt_seed_for_message(message),
                        state.rollout_percent,
                    )
                {
                    if let Some(canary) = self
                        .load_tool_strategy_profile_by_key(
                            crate::core::self_evolve::strategy_runtime::TOOL_STRATEGY_PROFILE_CANARY_KEY,
                        )
                        .await
                    {
                        selected = canary;
                    }
                }
            }
        }
        let version = selected.version.trim();
        if version.is_empty() {
            None
        } else {
            Some(safe_truncate(version, 128))
        }
    }

    async fn persist_experience_run_for_exchange(
        &self,
        job: &DeferredExchangePersistence,
        policy_version: &str,
    ) -> Result<()> {
        let trace_id = job.trace_snapshot.id.trim().to_string();
        let run_id = if trace_id.is_empty() {
            uuid::Uuid::new_v4().to_string()
        } else {
            trace_id.clone()
        };
        let now = chrono::Utc::now().to_rfc3339();
        let tool_history = experience_tool_history_from_trace(&job.trace_snapshot);
        let tool_names = experience_tool_names(&tool_history);
        let prompt_bundle = self.active_prompt_bundle_for_message(&job.message).await;
        let prompt_version = crate::core::self_evolve::prompt_evolution::compose_prompt_version(
            &prompt_bundle.version,
        );
        let execution_run_id = match job
            .execution_run_id
            .as_deref()
            .map(str::trim)
            .filter(|value| !value.is_empty())
        {
            Some(id) => match self.storage.load_execution_run(id).await {
                Ok(Some(_)) => Some(id.to_string()),
                Ok(None) => {
                    tracing::debug!(
                        trace_id = %trace_id,
                        execution_run_id = %id,
                        "Dropping stale execution_run_id while persisting experience_run"
                    );
                    None
                }
                Err(error) => {
                    tracing::warn!(
                        trace_id = %trace_id,
                        execution_run_id = %id,
                        error = %error,
                        "Could not verify execution_run_id while persisting experience_run"
                    );
                    None
                }
            },
            None => None,
        };
        let strategy_version = self
            .active_tool_strategy_version_for_message(&job.message)
            .await;
        let success_state = experience_run_success_state(&job.user_outcome);
        let repair_decisions = experience_repair_decisions(&tool_history);
        let tool_batches = experience_tool_batch_events(&tool_history);
        let tool_batch_count = tool_batches.len();
        let reports_degenerate = tool_history.iter().any(|entry| {
            entry
                .get("result")
                .is_some_and(super::tool_responses::structured_tool_value_reports_degenerate_output)
        });
        let metadata = serde_json::json!({
            "run_status": job.run_status.as_str(),
            "flow_kind": match job.kind {
                DeferredExchangePersistenceKind::TurnPipeline => "turn_pipeline",
                DeferredExchangePersistenceKind::Immediate => "immediate",
            },
            "degenerate": reports_degenerate,
            "tool_call_count": tool_names.len(),
            "tool_batch_count": tool_batch_count,
            "tool_batches": tool_batches,
            "retries": tool_names.len().saturating_sub(tool_names.iter().collect::<HashSet<_>>().len()),
            "repair_decisions": repair_decisions,
            "trace_complexity": job.trace_snapshot.complexity.as_deref(),
            "user_outcome_status": &job.user_outcome.status,
            "learning_signal": {
                "procedure_eligible": !tool_names.is_empty(),
                "negative_eligible": success_state == "failed",
            },
        });
        let run = crate::storage::entities::experience_run::Model {
            id: run_id,
            execution_run_id,
            trace_id: if trace_id.is_empty() {
                None
            } else {
                Some(trace_id.clone())
            },
            conversation_id: if job.conversation_key.trim().is_empty() {
                None
            } else {
                Some(job.conversation_key.clone())
            },
            project_id: job.project_id.clone(),
            channel: job.channel.clone(),
            scope: experience_run_scope(job.project_id.as_deref(), Some(&job.conversation_key)),
            intent_key: experience_intent_key(job.trace_snapshot.plan.as_ref(), &tool_names),
            task_type: Some("chat".to_string()),
            request_text: Some(safe_truncate(
                &crate::security::redact_secret_input(&job.message).text,
                EXPERIENCE_RUN_REQUEST_TEXT_CHARS,
            )),
            tool_sequence_digest: experience_tool_sequence_digest(&tool_names),
            tool_sequence_json: experience_tool_sequence_json(&tool_history),
            strategy_version,
            policy_version: Some(policy_version.to_string()),
            prompt_version: Some(prompt_version),
            model_slot: Some(job.model_used.clone()),
            success_state: success_state.to_string(),
            correction_state: "none".to_string(),
            outcome_summary: Some(safe_truncate(
                &crate::security::redact_secret_input(&job.response).text,
                EXPERIENCE_RUN_OUTCOME_SUMMARY_CHARS,
            )),
            failure_reason: experience_run_failure_reason(&job.run_status, &job.user_outcome),
            metadata,
            consolidated: false,
            accepted_at: None,
            corrected_at: None,
            heuristic_reflected: false,
            heuristic_reflection_status: None,
            heuristic_reflection_attempted_at: None,
            heuristic_reflection_completed_at: None,
            heuristic_lesson_id: None,
            heuristic_reflection_error: None,
            created_at: now.clone(),
            updated_at: now,
        };
        self.storage.upsert_experience_run(&run).await
    }

    fn spawn_deferred_exchange_persistence(&self, job: DeferredExchangePersistence) {
        let agent = self.clone();
        let trace_id = job.trace_snapshot.id.clone();
        let pending = DEFERRED_CHAT_PERSISTENCE_PENDING
            .fetch_add(1, std::sync::atomic::Ordering::Relaxed)
            + 1;
        if pending >= DEFERRED_CHAT_PERSISTENCE_WARN_PENDING {
            tracing::warn!(
                pending,
                "Deferred chat persistence backlog is high; responses are still unblocked"
            );
        }

        tokio::spawn(async move {
            let semaphore = DEFERRED_CHAT_PERSISTENCE_SEMAPHORE.clone();
            let _permit = match semaphore.acquire_owned().await {
                Ok(permit) => permit,
                Err(_) => {
                    DEFERRED_CHAT_PERSISTENCE_PENDING
                        .fetch_sub(1, std::sync::atomic::Ordering::Relaxed);
                    tracing::error!(
                        trace_id,
                        "Deferred chat persistence limiter closed before persistence could run"
                    );
                    return;
                }
            };

            let mut last_error: Option<String> = None;
            let mut persisted = false;
            for attempt in 1..=DEFERRED_CHAT_PERSISTENCE_ATTEMPTS {
                let result = tokio::time::timeout(
                    std::time::Duration::from_secs(DEFERRED_CHAT_PERSISTENCE_ATTEMPT_TIMEOUT_SECS),
                    agent.persist_deferred_exchange_once(job.clone()),
                )
                .await;

                match result {
                    Ok(Ok(())) => {
                        persisted = true;
                        break;
                    }
                    Ok(Err(error)) => {
                        last_error = Some(error.to_string());
                    }
                    Err(_) => {
                        last_error = Some(format!(
                            "attempt timed out after {} seconds",
                            DEFERRED_CHAT_PERSISTENCE_ATTEMPT_TIMEOUT_SECS
                        ));
                    }
                }

                if attempt < DEFERRED_CHAT_PERSISTENCE_ATTEMPTS {
                    let delay_secs = match attempt {
                        1 => 1,
                        2 => 3,
                        _ => 8,
                    };
                    tokio::time::sleep(std::time::Duration::from_secs(delay_secs)).await;
                }
            }

            if !persisted {
                tracing::error!(
                    trace_id,
                    error = last_error.as_deref().unwrap_or("unknown"),
                    "Deferred chat persistence failed after retries"
                );
            }
            DEFERRED_CHAT_PERSISTENCE_PENDING.fetch_sub(1, std::sync::atomic::Ordering::Relaxed);
        });
    }

    async fn persist_deferred_exchange_once(&self, job: DeferredExchangePersistence) -> Result<()> {
        let trace_id = job.trace_snapshot.id.clone();
        self.persist_completed_trace_snapshot_durable(&job.trace_snapshot)
            .await?;

        let flow_kind = match job.kind {
            DeferredExchangePersistenceKind::TurnPipeline => "turn_pipeline",
            DeferredExchangePersistenceKind::Immediate => "immediate",
        };
        let policy_version = self
            .active_tool_strategy_version_for_message(&job.message)
            .await
            .unwrap_or_else(|| "model_routed_spine_v1".to_string());
        if let Err(error) = self
            .persist_experience_run_for_exchange(&job, &policy_version)
            .await
        {
            tracing::warn!(
                trace_id,
                "Failed to persist experience_run evidence for completed turn: {}",
                error
            );
        }
        let duration_ms = trace_duration_ms(&job.trace_snapshot).unwrap_or(0);
        let started_payload = serde_json::json!({
            "flow_kind": flow_kind,
            "message_chars": job.message.chars().count(),
            "resumed": job.user_message_already_recorded,
        });
        self.log_operational_event(operational::OperationalEvent {
            event_type: "agent_request",
            channel: &job.channel,
            success: true,
            outcome: "started",
            trace_id: Some(&trace_id),
            conversation_id: Some(&job.conversation_key),
            tool_name: None,
            latency_ms: Some(0),
            arguments: None,
            payload: Some(&started_payload),
            strategy_version: None,
            policy_version: Some(&policy_version),
            prompt_version: None,
            specialist_prompt_version: None,
            model_slot: Some(&job.model_used),
        })
        .await;

        let completed_payload = serde_json::json!({
            "flow_kind": flow_kind,
            "response_chars": job.response.chars().count(),
            "tool_calls": 0,
            "degradation_notes": job.user_outcome.degradation.len(),
            "status": job.run_status.as_str(),
        });
        self.log_operational_event(operational::OperationalEvent {
            event_type: "response_complete",
            channel: &job.channel,
            success: operational_success_for_run_status(&job.run_status),
            outcome: &job.run_status,
            trace_id: Some(&trace_id),
            conversation_id: Some(&job.conversation_key),
            tool_name: None,
            latency_ms: Some(duration_ms),
            arguments: None,
            payload: Some(&completed_payload),
            strategy_version: None,
            policy_version: Some(&policy_version),
            prompt_version: None,
            specialist_prompt_version: None,
            model_slot: Some(&job.model_used),
        })
        .await;

        if !job.choices.is_empty() {
            let choice_payload = serde_json::json!({
                "should_clarify": true,
                "choices": job.choices,
            });
            self.log_operational_event(operational::OperationalEvent {
                event_type: "spine_user_input",
                channel: &job.channel,
                success: true,
                outcome: "needs_input",
                trace_id: Some(&trace_id),
                conversation_id: Some(&job.conversation_key),
                tool_name: None,
                latency_ms: Some(duration_ms),
                arguments: None,
                payload: Some(&choice_payload),
                strategy_version: None,
                policy_version: Some(&policy_version),
                prompt_version: None,
                specialist_prompt_version: None,
                model_slot: Some(&job.model_used),
            })
            .await;
        }

        if !job.conversation_key.is_empty() {
            if job.is_new_conversation && !job.user_message_already_recorded {
                self.ensure_conversation_row_for_turn(
                    &job.conversation_key,
                    &job.channel,
                    job.project_id.as_deref(),
                    &job.message,
                    job.conversation_title.as_deref(),
                )
                .await?;
            }
            let mut persisted_user_message_id = job.recorded_user_message_id.clone();
            if !job.user_message_already_recorded {
                let user_msg = crate::storage::entities::message::Model {
                    id: job.user_message_id.clone(),
                    conversation_id: job.conversation_key.clone(),
                    role: "user".to_string(),
                    content: job.message.clone(),
                    tool_calls_json: None,
                    tool_call_id: None,
                    provider_message_json: None,
                    timestamp: job.user_timestamp.clone(),
                    model_used: None,
                    trace_id: Some(trace_id.clone()),
                };
                self.encrypted_storage
                    .insert_message_encrypted_if_absent(&user_msg)
                    .await?;
                persisted_user_message_id = Some(user_msg.id);
            }
            let asst_msg = crate::storage::entities::message::Model {
                id: job.assistant_message_id.clone(),
                conversation_id: job.conversation_key.clone(),
                role: "assistant".to_string(),
                content: job.response.clone(),
                tool_calls_json: None,
                tool_call_id: None,
                provider_message_json: None,
                timestamp: job.assistant_timestamp.clone(),
                model_used: Some(job.model_used.clone()),
                trace_id: Some(trace_id.clone()),
            };
            self.encrypted_storage
                .insert_message_encrypted_if_absent(&asst_msg)
                .await?;

            if job.is_new_conversation {
                if let Some(title) = job.conversation_title.as_deref() {
                    self.storage
                        .update_conversation(&job.conversation_key, Some(title), None, None)
                        .await?;
                }
            }

            if job.memory_capture_allowed {
                let memory_source = job
                    .memory_capture_source
                    .as_deref()
                    .unwrap_or(job.message.as_str());
                let user_message_for_link_capture = job
                    .user_message_for_link_capture
                    .as_deref()
                    .unwrap_or(job.message.as_str());
                let queued_memory_capture = self
                    .mark_user_memory_capture_candidate(
                        memory_source,
                        user_message_for_link_capture,
                        &job.channel,
                        Some(&job.conversation_key),
                        job.project_id.as_deref(),
                        persisted_user_message_id.as_deref(),
                    )
                    .await;
                if queued_memory_capture {
                    self.kick_deferred_user_memory_capture_processing();
                }
            }

            self.sync_background_session_after_response(
                &job.conversation_key,
                &job.message,
                &job.response,
            )
            .await;
            self.sync_pending_resilience_followup(
                &job.conversation_key,
                &job.message,
                &job.channel,
                job.project_id.as_deref(),
                &job.user_outcome,
            )
            .await;
        }

        self.record_completed_interaction_for_self_tune().await;
        Ok(())
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
        choices: Vec<ClarificationChoice>,
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
        let flow_label = "Model-routed spine";
        let complexity = "model_routed_spine";
        let first_content_source = "model_routed_spine_first_content";
        let mut steps = Vec::with_capacity(trace_steps.len() + 3);
        steps.push(ExecutionStep {
            icon: "[turn]".to_string(),
            title: "Turn Request".to_string(),
            detail: format!(
                "{} | Channel: {} | Length: {} chars",
                flow_label,
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
                    "source": first_content_source
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
            cached_prompt_tokens: usage_delta.cached_prompt_tokens,
            cache_creation_prompt_tokens: usage_delta.cache_creation_prompt_tokens,
            cost_usd: usage_delta.cost_usd,
            complexity: Some(complexity.to_string()),
            plan: turn_plan.clone(),
        }));
        let trace_snapshot = trace_ref.read().await.clone();
        self.remember_completed_trace_snapshot(trace_snapshot.clone(), true)
            .await;

        {
            let mut history = self.conversation_history.write().await;
            let conversation_history = history
                .entry(context.conversation_key.to_string())
                .or_insert_with(Vec::new);
            if !context.user_message_already_recorded || context.recorded_user_message_id.is_some()
            {
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
            self.trim_in_memory_conversation_history(conversation_history);
        }

        let mut conversation_title: Option<String> = None;
        if !context.conversation_key.is_empty() {
            if context.is_new_conversation {
                let title = self.generate_conversation_title(message);
                *self.last_conversation_title.write().await = Some(title.clone());
                conversation_title = Some(title);
            } else {
                *self.last_conversation_title.write().await = None;
            }
        }

        *self.last_conversation_id.write().await = Some(context.conversation_key.to_string());

        let user_outcome = self
            .build_response_heuristic_outcome(&safe_response, &[], &[], None)
            .unwrap_or_else(|| {
                self.execution_supervisor
                    .build_success_outcome(&safe_response, &[], &[])
            });
        let final_run_status = if run_status.trim().is_empty() {
            Self::execution_run_status_for_outcome(&user_outcome)
                .as_str()
                .to_string()
        } else {
            run_status.trim().to_string()
        };
        self.spawn_deferred_exchange_persistence(DeferredExchangePersistence {
            kind: DeferredExchangePersistenceKind::TurnPipeline,
            trace_snapshot,
            execution_run_id: Some(run_id.clone()),
            message: message.to_string(),
            response: safe_response.clone(),
            run_status: final_run_status.clone(),
            channel: context.channel.to_string(),
            conversation_key: context.conversation_key.to_string(),
            project_id: context.project_id.map(str::to_string),
            model_used: context.model_used.to_string(),
            user_message_already_recorded: context.user_message_already_recorded,
            recorded_user_message_id: context.recorded_user_message_id.clone(),
            memory_capture_allowed: context.memory_capture_allowed,
            memory_capture_source: context.memory_capture_source.map(str::to_string),
            user_message_for_link_capture: context
                .user_message_for_link_capture
                .map(str::to_string),
            user_message_id: uuid::Uuid::new_v4().to_string(),
            assistant_message_id: uuid::Uuid::new_v4().to_string(),
            user_timestamp: chrono::Utc::now().to_rfc3339(),
            assistant_timestamp: chrono::Utc::now().to_rfc3339(),
            is_new_conversation: context.is_new_conversation,
            conversation_title: conversation_title.clone(),
            user_outcome: user_outcome.clone(),
            choices: choices.clone(),
        });

        Ok(ProcessedMessage {
            response: safe_response,
            conversation_id: Some(context.conversation_key.to_string()),
            conversation_title,
            run_id: Some(run_id),
            run_status: Some(final_run_status),
            trace_id: Some(trace_id),
            input_tokens: usage_delta.input_tokens,
            output_tokens: usage_delta.output_tokens,
            total_tokens: usage_delta.total_tokens,
            cached_prompt_tokens: usage_delta.cached_prompt_tokens,
            cache_creation_prompt_tokens: usage_delta.cache_creation_prompt_tokens,
            choices,
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
        let hints = RequestExecutionHints {
            execution_surface: ActionExecutionSurface::Chat,
            direct_user_intent: true,
            caller_principal: caller.cloned(),
            ..RequestExecutionHints::default()
        };

        self.process_message_with_meta_and_hints(
            &resume_message,
            run.channel.as_deref().unwrap_or("web"),
            run.conversation_id.as_deref(),
            None,
            hints,
        )
        .await
    }

    /// Build a structured surface-context JSON for the inbound classifier.
    ///
    /// We intentionally describe the surface in capability terms — what the
    /// user can do here — not in any user-facing language. The classifier
    /// reasons from the structural shape (which capability clusters are
    /// available) rather than from any phrase. Returns `None` when the
    /// channel does not correspond to a known structural surface, so the
    /// general inbound prompt is unchanged for ordinary chat traffic.
    pub(super) async fn run_inbound_security_precheck(
        &self,
        classification_message: &str,
        stored_user_message: &str,
        channel: &str,
        conversation_key: &str,
        is_new_conversation: bool,
        project_id: Option<&str>,
        user_message_already_recorded: bool,
        turn_timing_id: &str,
        stream_tx: Option<&tokio::sync::mpsc::Sender<StreamEvent>>,
    ) -> Result<InboundSecurityPrecheck> {
        let inbound_total_started = std::time::Instant::now();
        emit_inbound_precheck_progress(
            stream_tx,
            "start",
            "Checking request safety and routing signals...",
            serde_json::json!({
                "channel": channel,
                "conversation_id": conversation_key,
            }),
        );
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
        let stage_started = std::time::Instant::now();
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
                            recorded_user_message_id: None,
                            memory_capture_allowed: false,
                            memory_capture_source: None,
                            user_message_for_link_capture: Some(stored_user_message),
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
        log_turn_timing_instant(
            turn_timing_id,
            conversation_key,
            channel,
            "inbound_abuse_status_lookup",
            stage_started,
            true,
            TURN_TIMING_SLOW_STAGE_WARN_MS,
        );

        // Intent-based inbound guard. The classifier sees the already-redacted
        // storage form, then normalization removes unicode obfuscation controls.
        let normalized_for_guard = crate::security::normalize_for_analysis(classification_message);
        let new_empty_conversation = is_new_conversation && !user_message_already_recorded;
        let stage_started = std::time::Instant::now();
        let recent_artifacts = if new_empty_conversation {
            Vec::new()
        } else {
            Self::conversation_artifacts_for_prompt(
                &self.load_recent_artifact_contexts(conversation_key).await,
                INBOUND_CLASSIFIER_RECENT_ARTIFACTS,
            )
        };
        log_turn_timing_instant(
            turn_timing_id,
            conversation_key,
            channel,
            "inbound_recent_artifacts_load",
            stage_started,
            true,
            TURN_TIMING_SLOW_STAGE_WARN_MS,
        );
        let recent_artifacts_context = (!recent_artifacts.is_empty())
            .then(|| serde_json::Value::Array(recent_artifacts.clone()));
        let stage_started = std::time::Instant::now();
        let mut recent_messages_context = if new_empty_conversation {
            Vec::new()
        } else {
            self.recent_messages_for_intent_gating(conversation_key, stored_user_message)
                .await
                .into_iter()
                .rev()
                .take(4)
                .map(|message| {
                    serde_json::json!({
                        "role": message.role,
                        "content": safe_truncate(
                            &crate::security::redact_secret_input(&message.content).text,
                            360,
                        ),
                        "timestamp": message._timestamp,
                    })
                })
                .collect::<Vec<_>>()
        };
        log_turn_timing_instant(
            turn_timing_id,
            conversation_key,
            channel,
            "inbound_recent_messages_load",
            stage_started,
            true,
            TURN_TIMING_SLOW_STAGE_WARN_MS,
        );
        recent_messages_context.reverse();
        let recent_messages_context_value = (!recent_messages_context.is_empty())
            .then(|| serde_json::Value::Array(recent_messages_context.clone()));
        let embedding_context = (!recent_messages_context.is_empty()
            || recent_artifacts_context.is_some())
        .then(|| {
            serde_json::json!({
                "recent_messages": recent_messages_context_value.clone(),
                "recent_actionable_artifacts": recent_artifacts_context,
            })
            .to_string()
        });
        if let Some(embedder) = self.embedding_client.as_deref() {
            let stage_started = std::time::Instant::now();
            let grace_ms = inbound_embedding_fastpath_grace_ms();
            emit_inbound_precheck_progress(
                stream_tx,
                "embedding_fastpath",
                "Checking fast semantic guard...",
                serde_json::json!({
                    "grace_ms": grace_ms,
                }),
            );
            let embedding_decision = tokio::time::timeout(
                std::time::Duration::from_millis(grace_ms),
                crate::security::embedding_classifier::classify_inbound_embedding_fast(
                    embedder,
                    &normalized_for_guard,
                    embedding_context.as_deref(),
                    Some(self.data_dir.as_path()),
                ),
            )
            .await;
            match embedding_decision {
                Ok(Ok(Some(fast))) => {
                    log_turn_timing_instant(
                        turn_timing_id,
                        conversation_key,
                        channel,
                        "inbound_embedding_classifier",
                        stage_started,
                        true,
                        TURN_TIMING_SLOW_STAGE_WARN_MS,
                    );
                    tracing::info!(
                        target: "security.inbound",
                        category = ?fast.category,
                        concept = %fast.concept,
                        score = fast.score,
                        margin = fast.margin,
                        "inbound embedding classifier accepted high-confidence security decision"
                    );
                    match &fast.decision.verdict {
                        crate::security::intent_classifier::IntentVerdict::Block {
                            message: safe_reply,
                            rule_id,
                            severity,
                        } => {
                            emit_inbound_precheck_progress(
                                stream_tx,
                                "complete",
                                "Request was handled by the fast guard.",
                                serde_json::json!({
                                    "verdict": "block",
                                    "source": "embedding_fastpath",
                                }),
                            );
                            log_turn_timing_instant(
                                turn_timing_id,
                                conversation_key,
                                channel,
                                "inbound_precheck_total",
                                inbound_total_started,
                                true,
                                TURN_TIMING_SLOW_STAGE_WARN_MS,
                            );
                            self.security_events.record_injection_attempt();
                            tracing::warn!(
                                target: "security.inbound",
                                rule_id = %rule_id,
                                severity = severity,
                                channel = %channel,
                                "inbound embedding classifier blocked message"
                            );
                            let source_label = inbound_security_source_label(channel);
                            let alert_msg = format!(
                                "Security guard blocked a message from {} (rule {}).",
                                &source_label, rule_id
                            );
                            tracing::info!(
                                target: "security.inbound",
                                channel = %channel,
                                rule_id = %rule_id,
                                alert = %alert_msg,
                                "inbound guard block kept in security logs without user notification"
                            );
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
                                        "abuse_tracker.record_trip failed after embedding block; block applied but escalation state not updated"
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
                                        model_used: "security_embedding_guard",
                                        user_message_already_recorded,
                                        recorded_user_message_id: None,
                                        memory_capture_allowed: false,
                                        memory_capture_source: None,
                                        user_message_for_link_capture: Some(stored_user_message),
                                    },
                                )
                                .await?;
                            return Ok(InboundSecurityPrecheck::Respond(processed));
                        }
                        crate::security::intent_classifier::IntentVerdict::Allow => {
                            emit_inbound_precheck_progress(
                                stream_tx,
                                "complete",
                                "Fast semantic guard passed.",
                                serde_json::json!({
                                    "verdict": "allow",
                                    "source": "embedding_fastpath",
                                }),
                            );
                            log_turn_timing_instant(
                                turn_timing_id,
                                conversation_key,
                                channel,
                                "inbound_precheck_total",
                                inbound_total_started,
                                true,
                                TURN_TIMING_SLOW_STAGE_WARN_MS,
                            );
                            return Ok(InboundSecurityPrecheck::Continue);
                        }
                        crate::security::intent_classifier::IntentVerdict::AllowWithUncheckedTag {
                            ..
                        } => {
                            emit_inbound_precheck_progress(
                                stream_tx,
                                "complete",
                                "Fast semantic guard passed with limited context.",
                                serde_json::json!({
                                    "verdict": "allow_unchecked",
                                    "source": "embedding_fastpath",
                                }),
                            );
                            log_turn_timing_instant(
                                turn_timing_id,
                                conversation_key,
                                channel,
                                "inbound_precheck_total",
                                inbound_total_started,
                                true,
                                TURN_TIMING_SLOW_STAGE_WARN_MS,
                            );
                            return Ok(InboundSecurityPrecheck::Continue);
                        }
                        crate::security::intent_classifier::IntentVerdict::ClassifierUnavailable {
                            ..
                        } => {}
                    }
                }
                Ok(Ok(None)) => {
                    log_turn_timing_instant(
                        turn_timing_id,
                        conversation_key,
                        channel,
                        "inbound_embedding_classifier",
                        stage_started,
                        true,
                        TURN_TIMING_SLOW_STAGE_WARN_MS,
                    );
                }
                Ok(Err(error)) => {
                    log_turn_timing_instant(
                        turn_timing_id,
                        conversation_key,
                        channel,
                        "inbound_embedding_classifier",
                        stage_started,
                        false,
                        TURN_TIMING_SLOW_STAGE_WARN_MS,
                    );
                    tracing::warn!(
                        target: "security.inbound",
                        error = %error,
                        "inbound embedding classifier unavailable; continuing to spine without model pre-classification"
                    );
                }
                Err(_) => {
                    log_turn_timing_instant(
                        turn_timing_id,
                        conversation_key,
                        channel,
                        "inbound_embedding_classifier_deferred",
                        stage_started,
                        true,
                        TURN_TIMING_SLOW_STAGE_WARN_MS,
                    );
                    let embedder_clone = (*embedder).clone();
                    let normalized_for_warmup = normalized_for_guard.clone();
                    let embedding_context_for_warmup = embedding_context.clone();
                    let data_dir_for_warmup = self.data_dir.clone();
                    crate::spawn_logged!(
                        "src/core/agent/message_processing.rs:inbound_embedding_warmup",
                        async move {
                            let _ =
                                crate::security::embedding_classifier::classify_inbound_embedding_fast(
                                    &embedder_clone,
                                    &normalized_for_warmup,
                                    embedding_context_for_warmup.as_deref(),
                                    Some(data_dir_for_warmup.as_path()),
                                )
                                .await;
                        }
                    );
                    emit_inbound_precheck_progress(
                        stream_tx,
                        "embedding_fastpath",
                        "Fast guard is still warming; continuing with the main model.",
                        serde_json::json!({
                            "embedding_fastpath_deferred": true,
                            "grace_ms": grace_ms,
                        }),
                    );
                }
            }
        }
        emit_inbound_precheck_progress(
            stream_tx,
            "complete",
            "Request guard complete.",
            serde_json::json!({
                "verdict": "continue",
                "source": "embedding_fastpath",
            }),
        );
        log_turn_timing_instant(
            turn_timing_id,
            conversation_key,
            channel,
            "inbound_precheck_total",
            inbound_total_started,
            true,
            TURN_TIMING_SLOW_STAGE_WARN_MS,
        );
        Ok(InboundSecurityPrecheck::Continue)
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
                    icon: "[guard]".to_string(),
                    title: "Message Received".to_string(),
                    detail: format!(
                        "Pre-tool response path | Channel: {} | Length: {} chars",
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
                        "Returned before tool execution using {}.",
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
            cached_prompt_tokens: 0,
            cache_creation_prompt_tokens: 0,
            cost_usd: 0.0,
            complexity: Some("immediate".to_string()),
            plan: None,
        }));
        let trace_snapshot = trace_ref.read().await.clone();
        self.remember_completed_trace_snapshot(trace_snapshot.clone(), true)
            .await;
        tracing::info!(
            "Request started: trace={} channel={} flow=immediate resumed={}",
            trace_id,
            context.channel,
            context.user_message_already_recorded
        );
        tracing::info!(
            "Request completed: trace={} channel={} status=completed duration=0ms tools=0",
            trace_id,
            context.channel
        );

        // Mirror normal chat persistence path for pre-tool responses.
        {
            let mut history = self.conversation_history.write().await;
            let conversation_history = history
                .entry(context.conversation_key.to_string())
                .or_insert_with(Vec::new);
            if !context.user_message_already_recorded || context.recorded_user_message_id.is_some()
            {
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
            self.trim_in_memory_conversation_history(conversation_history);
        }

        let mut conversation_title: Option<String> = None;
        if !context.conversation_key.is_empty() {
            if context.is_new_conversation {
                let title = self.generate_conversation_title(message);
                *self.last_conversation_title.write().await = Some(title.clone());
                conversation_title = Some(title);
            } else {
                *self.last_conversation_title.write().await = None;
            }
        }

        *self.last_conversation_id.write().await = Some(context.conversation_key.to_string());

        let user_outcome = self
            .build_response_heuristic_outcome(&safe_response, &[], &[], None)
            .unwrap_or_else(|| {
                self.execution_supervisor
                    .build_success_outcome(&safe_response, &[], &[])
            });
        let run_status = Self::execution_run_status_for_outcome(&user_outcome);
        self.spawn_deferred_exchange_persistence(DeferredExchangePersistence {
            kind: DeferredExchangePersistenceKind::Immediate,
            trace_snapshot,
            execution_run_id: None,
            message: message.to_string(),
            response: safe_response.clone(),
            run_status: run_status.as_str().to_string(),
            channel: context.channel.to_string(),
            conversation_key: context.conversation_key.to_string(),
            project_id: context.project_id.map(str::to_string),
            model_used: context.model_used.to_string(),
            user_message_already_recorded: context.user_message_already_recorded,
            recorded_user_message_id: context.recorded_user_message_id.clone(),
            memory_capture_allowed: context.memory_capture_allowed,
            memory_capture_source: context.memory_capture_source.map(str::to_string),
            user_message_for_link_capture: context
                .user_message_for_link_capture
                .map(str::to_string),
            user_message_id: uuid::Uuid::new_v4().to_string(),
            assistant_message_id: uuid::Uuid::new_v4().to_string(),
            user_timestamp: chrono::Utc::now().to_rfc3339(),
            assistant_timestamp: chrono::Utc::now().to_rfc3339(),
            is_new_conversation: context.is_new_conversation,
            conversation_title: conversation_title.clone(),
            user_outcome: user_outcome.clone(),
            choices: Vec::new(),
        });

        Ok(ProcessedMessage {
            response: safe_response,
            conversation_id: Some(context.conversation_key.to_string()),
            conversation_title,
            run_id: None,
            run_status: Some(run_status.as_str().to_string()),
            trace_id: None,
            input_tokens: 0,
            output_tokens: 0,
            total_tokens: 0,
            cached_prompt_tokens: 0,
            cache_creation_prompt_tokens: 0,
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

        self.remember_completed_trace_snapshot(
            trace_snapshot.clone(),
            !Arc::ptr_eq(trace_ref, &self.last_trace),
        )
        .await;

        if let Err(e) = self
            .persist_completed_trace_snapshot_durable(&trace_snapshot)
            .await
        {
            tracing::warn!(
                "Failed to persist execution trace '{}': {}",
                trace_snapshot.id,
                e
            );
        } else {
            self.record_completed_interaction_for_self_tune().await;
        }
    }

    async fn persist_completed_trace_snapshot_durable(
        &self,
        trace_snapshot: &ExecutionTrace,
    ) -> Result<()> {
        self.encrypted_storage
            .insert_execution_trace_encrypted(trace_snapshot)
            .await?;
        if let Err(error) =
            crate::core::self_evolve::maybe_upsert_router_replay_candidate_from_trace(
                &self.storage,
                trace_snapshot,
            )
            .await
        {
            tracing::debug!(
                trace_id = %trace_snapshot.id,
                error = %error,
                "router replay candidate capture skipped"
            );
        }
        let observability_endpoint =
            crate::core::platform::observability::normalize_observability_endpoint(
                &self.config.observability.provider,
                &self.config.observability.endpoint,
            );
        let observability_ready = self.config.observability.enabled
            && !observability_endpoint.is_empty()
            && crate::core::platform::observability::has_observability_auth_token(
                &self.config_dir,
                Some(&self.data_dir),
            )
            .unwrap_or(false);
        if observability_ready {
            let provider = crate::core::platform::observability::normalize_observability_provider(
                &self.config.observability.provider,
            );
            match crate::core::platform::observability::export_execution_trace(
                &self.config,
                &self.config_dir,
                &self.data_dir,
                &self.storage,
                trace_snapshot,
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

        Ok(())
    }

    async fn record_completed_interaction_for_self_tune(&self) {
        // Self-tune: track interaction for adaptive learning
        crate::core::self_evolve::self_tune::on_interaction_completed(
            &self.storage,
            &self.encrypted_storage,
            &self.llm,
        )
        .await;
    }
}
