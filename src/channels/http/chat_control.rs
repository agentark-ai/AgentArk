use super::*;
use futures::FutureExt as _;

static CHAT_WORKER_RUNTIME: OnceLock<std::result::Result<Arc<tokio::runtime::Runtime>, String>> =
    OnceLock::new();
static CHAT_WORKER_PERMITS: OnceLock<Arc<tokio::sync::Semaphore>> = OnceLock::new();
const CHAT_STREAM_TRACE_LINK_WAIT_ATTEMPTS: usize = 40;
const CHAT_STREAM_TRACE_LINK_WAIT_MS: u64 = 100;

// Keep long model/tool turns off the control-plane runtime so health and UI APIs stay responsive.
fn chat_worker_thread_count() -> usize {
    std::env::var("AGENTARK_CHAT_WORKER_THREADS")
        .ok()
        .and_then(|value| value.trim().parse::<usize>().ok())
        .filter(|value| *value > 0)
        .unwrap_or_else(|| {
            std::thread::available_parallelism()
                .map(|value| value.get())
                .unwrap_or(4)
                .max(2)
        })
        .clamp(1, 64)
}

fn chat_worker_max_concurrency() -> usize {
    std::env::var("AGENTARK_CHAT_WORKER_MAX_CONCURRENCY")
        .ok()
        .and_then(|value| value.trim().parse::<usize>().ok())
        .filter(|value| *value > 0)
        .unwrap_or_else(chat_worker_thread_count)
        .clamp(1, 64)
}

fn chat_turn_timeout_secs() -> u64 {
    std::env::var("AGENTARK_CHAT_TURN_TIMEOUT_SECS")
        .ok()
        .and_then(|value| value.trim().parse::<u64>().ok())
        .filter(|value| *value >= 30)
        .unwrap_or(900)
        .min(7_200)
}

fn chat_stream_idle_timeout_secs() -> u64 {
    std::env::var("AGENTARK_CHAT_STREAM_IDLE_TIMEOUT_SECS")
        .ok()
        .and_then(|value| value.trim().parse::<u64>().ok())
        .filter(|value| *value >= 30)
        .unwrap_or(900)
        .min(7_200)
}

async fn wait_for_chat_stream_idle_timeout(
    last_activity_ms: Arc<AtomicU64>,
    started_at: Instant,
    idle_timeout_secs: u64,
) {
    let idle_timeout_ms = idle_timeout_secs.saturating_mul(1_000);
    loop {
        tokio::time::sleep(Duration::from_secs(idle_timeout_secs.min(30).max(1))).await;
        let elapsed_ms = started_at.elapsed().as_millis().min(u128::from(u64::MAX)) as u64;
        let last_activity = last_activity_ms.load(Ordering::Relaxed);
        if elapsed_ms.saturating_sub(last_activity) >= idle_timeout_ms {
            return;
        }
    }
}

fn chat_worker_queue_timeout_secs() -> u64 {
    std::env::var("AGENTARK_CHAT_WORKER_QUEUE_TIMEOUT_SECS")
        .ok()
        .and_then(|value| value.trim().parse::<u64>().ok())
        .filter(|value| *value > 0)
        .unwrap_or(15)
        .min(300)
}

fn chat_worker_permits() -> Arc<tokio::sync::Semaphore> {
    CHAT_WORKER_PERMITS
        .get_or_init(|| Arc::new(tokio::sync::Semaphore::new(chat_worker_max_concurrency())))
        .clone()
}

fn chat_worker_runtime() -> std::result::Result<Arc<tokio::runtime::Runtime>, String> {
    CHAT_WORKER_RUNTIME
        .get_or_init(|| {
            let worker_threads = chat_worker_thread_count();
            tokio::runtime::Builder::new_multi_thread()
                .worker_threads(worker_threads)
                .thread_name("agentark-chat-worker")
                .enable_all()
                .build()
                .map(|runtime| {
                    tracing::info!(worker_threads, "Started dedicated chat worker runtime");
                    Arc::new(runtime)
                })
                .map_err(|error| format!("Failed to start chat worker runtime: {}", error))
        })
        .clone()
}

fn chat_worker_panic_payload_to_string(payload: Box<dyn std::any::Any + Send>) -> String {
    if let Some(message) = payload.downcast_ref::<&str>() {
        (*message).to_string()
    } else if let Some(message) = payload.downcast_ref::<String>() {
        message.clone()
    } else {
        "unknown panic payload".to_string()
    }
}

async fn run_chat_worker_task<F, T>(
    task_name: &'static str,
    future: F,
) -> std::result::Result<T, String>
where
    F: std::future::Future<Output = T> + Send + 'static,
    T: Send + 'static,
{
    let runtime = chat_worker_runtime()?;
    let (tx, rx) = tokio::sync::oneshot::channel();
    runtime.spawn(async move {
        let permits = chat_worker_permits();
        let queue_timeout_secs = chat_worker_queue_timeout_secs();
        let _permit = match tokio::time::timeout(
            Duration::from_secs(queue_timeout_secs),
            permits.acquire_owned(),
        )
        .await
        {
            Ok(Ok(permit)) => permit,
            Ok(Err(_)) => {
                let _ = tx.send(Err("Chat worker concurrency limiter closed".to_string()));
                return;
            }
            Err(_) => {
                let _ = tx.send(Err(format!(
                    "Chat worker queue timed out after {} seconds",
                    queue_timeout_secs
                )));
                return;
            }
        };
        let timeout_secs = chat_turn_timeout_secs();
        let outcome = tokio::time::timeout(
            Duration::from_secs(timeout_secs),
            std::panic::AssertUnwindSafe(future).catch_unwind(),
        )
        .await;
        let result = match outcome {
            Ok(Ok(output)) => Ok(output),
            Ok(Err(payload)) => {
                let panic = chat_worker_panic_payload_to_string(payload);
                tracing::error!(
                    task = task_name,
                    panic = %panic,
                    "Chat worker task panicked"
                );
                Err(format!("Chat worker task panicked: {}", panic))
            }
            Err(_) => Err(format!(
                "Chat worker task '{}' timed out after {} seconds",
                task_name, timeout_secs
            )),
        };
        let _ = tx.send(result);
    });
    rx.await
        .map_err(|_| format!("Chat worker task '{}' ended without a result", task_name))?
}

fn spawn_chat_worker_logged<F>(
    task_name: &'static str,
    future: F,
) -> std::result::Result<(), String>
where
    F: std::future::Future + Send + 'static,
    F::Output: crate::core::spawn::SpawnLoggedOutcome + Send + 'static,
{
    let runtime = chat_worker_runtime()?;
    runtime.spawn(async move {
        let permits = chat_worker_permits();
        let _permit = match permits.acquire_owned().await {
            Ok(permit) => permit,
            Err(_) => {
                tracing::error!(task = task_name, "Chat worker concurrency limiter closed");
                return;
            }
        };
        match std::panic::AssertUnwindSafe(future).catch_unwind().await {
            Ok(output) => {
                crate::core::spawn::SpawnLoggedOutcome::log_if_error(output, task_name);
            }
            Err(payload) => {
                tracing::error!(
                    task = task_name,
                    panic = %chat_worker_panic_payload_to_string(payload),
                    "Chat worker task panicked"
                );
            }
        }
    });
    Ok(())
}

pub(super) fn conversation_not_found_response() -> Response {
    (
        StatusCode::NOT_FOUND,
        Json(ErrorResponse {
            error: "Conversation not found".to_string(),
        }),
    )
        .into_response()
}

pub(super) fn build_request_execution_hints(
    caller: Option<&crate::actions::ActionCallerPrincipal>,
    surface: crate::actions::ActionExecutionSurface,
    direct_user_intent: bool,
    attachments: Vec<crate::core::ChatAttachmentHint>,
) -> crate::core::RequestExecutionHints {
    build_request_execution_hints_with_arkorbit(
        caller,
        surface,
        direct_user_intent,
        attachments,
        None,
    )
}

/// Variant that also threads ArkOrbit per-call context through to the agent
/// loop. The context is structural (orbit id, widget summary, optional
/// `agent_instructions`); routing/inference are unchanged.
pub(super) fn build_request_execution_hints_with_arkorbit(
    caller: Option<&crate::actions::ActionCallerPrincipal>,
    surface: crate::actions::ActionExecutionSurface,
    direct_user_intent: bool,
    attachments: Vec<crate::core::ChatAttachmentHint>,
    arkorbit_context: Option<serde_json::Value>,
) -> crate::core::RequestExecutionHints {
    build_request_execution_hints_with_context(
        caller,
        surface,
        direct_user_intent,
        attachments,
        arkorbit_context,
        None,
    )
}

pub(super) fn build_request_execution_hints_with_context(
    caller: Option<&crate::actions::ActionCallerPrincipal>,
    surface: crate::actions::ActionExecutionSurface,
    direct_user_intent: bool,
    attachments: Vec<crate::core::ChatAttachmentHint>,
    arkorbit_context: Option<serde_json::Value>,
    accepted_suggestion_context: Option<serde_json::Value>,
) -> crate::core::RequestExecutionHints {
    crate::core::RequestExecutionHints {
        turn_timing_id: None,
        caller_principal: caller.cloned(),
        execution_surface: surface,
        direct_user_intent,
        secret_offered: None,
        attachments,
        saved_user_facts_context: None,
        recorded_user_message_id: None,
        arkorbit_context,
        accepted_suggestion_context,
    }
}

pub(super) fn build_direct_action_auth_context(
    caller: Option<&crate::actions::ActionCallerPrincipal>,
    surface: crate::actions::ActionExecutionSurface,
    direct_user_intent: bool,
) -> crate::actions::ActionAuthorizationContext {
    crate::actions::ActionAuthorizationContext {
        principal: caller.cloned(),
        surface,
        direct_user_intent,
        current_turn_is_explicit_approval: false,
        agent_name: None,
        agent_access_scope: None,
        capability_context_id: None,
    }
}

#[derive(Debug, serde::Deserialize)]
#[serde(rename_all = "snake_case")]
pub(super) enum ChatToolApprovalDecision {
    Approve,
    Reject,
}

#[derive(Debug, serde::Deserialize)]
pub(super) struct ChatToolApprovalDecisionRequest {
    pub decision: ChatToolApprovalDecision,
}

pub(super) async fn decide_chat_tool_approval(
    State(state): State<AppState>,
    Path(id): Path<String>,
    Json(request): Json<ChatToolApprovalDecisionRequest>,
) -> Response {
    let agent = Agent::snapshot(&state.agent).await;
    match request.decision {
        ChatToolApprovalDecision::Approve => {
            match agent.approve_direct_chat_any_approval(&id).await {
                Ok((approval, response)) => (
                    StatusCode::OK,
                    Json(serde_json::json!({
                        "status": "approved",
                        "approval": approval,
                        "response": response,
                    })),
                )
                    .into_response(),
                Err(error) => (
                    StatusCode::BAD_REQUEST,
                    Json(ErrorResponse {
                        error: error.to_string(),
                    }),
                )
                    .into_response(),
            }
        }
        ChatToolApprovalDecision::Reject => {
            match agent.reject_direct_chat_any_approval(&id).await {
                Ok((approval, response)) => (
                    StatusCode::OK,
                    Json(serde_json::json!({
                        "status": "rejected",
                        "approval": approval,
                        "response": response,
                    })),
                )
                    .into_response(),
                Err(error) => (
                    StatusCode::BAD_REQUEST,
                    Json(ErrorResponse {
                        error: error.to_string(),
                    }),
                )
                    .into_response(),
            }
        }
    }
}

async fn handle_direct_chat_approval_submit_text(
    state: &AppState,
    message: &str,
) -> Option<std::result::Result<serde_json::Value, String>> {
    let (approval_id, decision) = crate::core::parse_direct_chat_approval_submit_text(message)?;
    let agent = Agent::snapshot(&state.agent).await;
    let result = match decision {
        crate::core::DirectChatApprovalSubmitDecision::Approve => agent
            .approve_direct_chat_any_approval(&approval_id)
            .await
            .map(|(approval, response)| {
                serde_json::json!({
                    "status": "approved",
                    "decision": decision.as_str(),
                    "approval": approval,
                    "response": response.clone(),
                    "content": response,
                })
            }),
        crate::core::DirectChatApprovalSubmitDecision::Reject => agent
            .reject_direct_chat_any_approval(&approval_id)
            .await
            .map(|(approval, response)| {
                serde_json::json!({
                    "status": "rejected",
                    "decision": decision.as_str(),
                    "approval": approval,
                    "response": response.clone(),
                    "content": response,
                })
            }),
    };
    Some(result.map_err(|error| error.to_string()))
}

pub(super) async fn resolve_chat_request_conversation_id(
    state: &AppState,
    channel: &str,
    conversation_id: Option<&str>,
    _project_id: Option<&str>,
    message: &str,
) -> std::result::Result<String, Response> {
    let agent = state.agent.read().await;
    match agent
        .ensure_conversation_id_for_request(channel, conversation_id, None, message)
        .await
    {
        Ok(conversation_id) => Ok(conversation_id),
        Err(error) if error.to_string() == "Conversation not found" => {
            Err(conversation_not_found_response())
        }
        Err(error) => Err((
            StatusCode::INTERNAL_SERVER_ERROR,
            Json(ErrorResponse {
                error: format!("Failed to resolve conversation: {}", error),
            }),
        )
            .into_response()),
    }
}

/// Chat with the agent
pub(super) async fn chat(
    State(state): State<AppState>,
    maybe_caller: Option<Extension<crate::actions::ActionCallerPrincipal>>,
    ConnectInfo(_addr): ConnectInfo<SocketAddr>,
    Json(request): Json<ChatRequest>,
) -> Response {
    let mut request = request;
    if let Some(response) = validate_chat_message_size(&request.message) {
        return response;
    }

    if let Some(existing_conversation_id) = request
        .conversation_id
        .as_deref()
        .map(str::trim)
        .filter(|value| !value.is_empty())
        .map(str::to_string)
    {
        let resolved_conversation_id = match resolve_chat_request_conversation_id(
            &state,
            &request.channel,
            Some(existing_conversation_id.as_str()),
            None,
            &request.message,
        )
        .await
        {
            Ok(conversation_id) => conversation_id,
            Err(response) => return response,
        };
        request.conversation_id = Some(resolved_conversation_id);
    } else {
        request.conversation_id = None;
    }

    tracing::info!(
        "HTTP /chat request: channel={}, msg={}chars, conv_id={:?}",
        request.channel,
        request.message.len(),
        request.conversation_id.as_deref().unwrap_or("-"),
    );

    if let Some(result) = handle_direct_chat_approval_submit_text(&state, &request.message).await {
        return match result {
            Ok(payload) => (
                StatusCode::OK,
                Json(ChatResponse {
                    response: payload
                        .get("response")
                        .and_then(|value| value.as_str())
                        .unwrap_or("Approval decision recorded.")
                        .to_string(),
                    proof_id: None,
                    conversation_id: request.conversation_id.clone(),
                    conversation_title: None,
                    run_id: None,
                    run_status: None,
                    trace_id: None,
                    total_tokens: 0,
                    choices: Vec::new(),
                    degradation: Vec::new(),
                    attempted_models: Vec::new(),
                    user_outcome: None,
                }),
            )
                .into_response(),
            Err(error) => (StatusCode::BAD_REQUEST, Json(ErrorResponse { error })).into_response(),
        };
    }

    // Internal escape hatch only. The product UX is the secure credential form.
    if let Some((key, value)) = parse_set_secret_command(&request.message) {
        if !crate::core::secrets::setsecret_command_escape_hatch_enabled() {
            return (
                StatusCode::OK,
                Json(ChatResponse {
                    response: crate::core::secrets::setsecret_command_disabled_response()
                        .to_string(),
                    proof_id: None,
                    conversation_id: request.conversation_id.clone(),
                    conversation_title: None,
                    run_id: None,
                    run_status: None,
                    trace_id: None,
                    total_tokens: 0,
                    choices: Vec::new(),
                    degradation: Vec::new(),
                    attempted_models: Vec::new(),
                    user_outcome: None,
                }),
            )
                .into_response();
        }
        let cid = request.conversation_id.clone();
        let mut values = BTreeMap::new();
        values.insert(key.clone(), value);
        let response = {
            let agent = state.agent.read().await;
            match agent
                .submit_chat_credential_values(cid.as_deref(), &values)
                .await
            {
                Ok(response) => response,
                Err(error) => {
                    return (
                        StatusCode::BAD_REQUEST,
                        Json(ErrorResponse {
                            error: error.to_string(),
                        }),
                    )
                        .into_response();
                }
            }
        };

        return (
            StatusCode::OK,
            Json(ChatResponse {
                response,
                proof_id: None,
                conversation_id: cid,
                conversation_title: None,
                run_id: None,
                run_status: None,
                trace_id: None,
                total_tokens: 0,
                choices: Vec::new(),
                degradation: Vec::new(),
                attempted_models: Vec::new(),
                user_outcome: None,
            }),
        )
            .into_response();
    }

    if let Some(response) = chat_secret_prompt_block_message(
        &state,
        request.conversation_id.as_deref(),
        &request.message,
    )
    .await
    {
        return (
            StatusCode::OK,
            Json(ChatResponse {
                response,
                proof_id: None,
                conversation_id: request.conversation_id.clone(),
                conversation_title: None,
                run_id: None,
                run_status: None,
                trace_id: None,
                total_tokens: 0,
                choices: Vec::new(),
                degradation: Vec::new(),
                attempted_models: Vec::new(),
                user_outcome: None,
            }),
        )
            .into_response();
    }

    // Human-in-the-loop shortcut: reuse currently configured model key without sending to the LLM.
    if let Some(key) = crate::core::secrets::parse_use_current_llm_key_command(&request.message) {
        if !crate::core::secrets::secret_command_escape_hatch_enabled() {
            return (
                StatusCode::OK,
                Json(ChatResponse {
                    response: crate::core::secrets::setsecret_command_disabled_response()
                        .to_string(),
                    proof_id: None,
                    conversation_id: request.conversation_id.clone(),
                    conversation_title: None,
                    run_id: None,
                    run_status: None,
                    trace_id: None,
                    total_tokens: 0,
                    choices: Vec::new(),
                    degradation: Vec::new(),
                    attempted_models: Vec::new(),
                    user_outcome: None,
                }),
            )
                .into_response();
        }
        let cid = request.conversation_id.clone();
        let (config_dir, data_dir, llm_env) = {
            let agent = state.agent.read().await;
            (
                agent.config_dir.clone(),
                agent.data_dir.clone(),
                agent.app_model_env_vars(),
            )
        };
        let Some(value) = llm_env.get(&key).cloned().filter(|v| !v.trim().is_empty()) else {
            let mut available: Vec<String> = llm_env
                .iter()
                .filter_map(|(k, v)| {
                    if v.trim().is_empty() {
                        None
                    } else if k.ends_with("_API_KEY")
                        || k.ends_with("_BASE_URL")
                        || k == "LLM_MODEL"
                        || k == "LLM_PROVIDER"
                    {
                        Some(k.clone())
                    } else {
                        None
                    }
                })
                .collect();
            available.sort();
            let available_text = if available.is_empty() {
                "none".to_string()
            } else {
                available.join(", ")
            };
            return (
                StatusCode::BAD_REQUEST,
                Json(ErrorResponse {
                    error: format!(
                        "Can't map '{}' from current model settings. Available model-backed keys: {}",
                        key, available_text
                    ),
                }),
            )
                .into_response();
        };

        if let Err(e) =
            crate::core::secrets::store_user_secret(&config_dir, Some(&data_dir), &key, &value)
        {
            return (
                StatusCode::INTERNAL_SERVER_ERROR,
                Json(ErrorResponse {
                    error: format!("Failed to store secret: {}", e),
                }),
            )
                .into_response();
        }

        let followup = if let Some(ref cid_str) = cid {
            let agent = state.agent.read().await;
            agent.on_secret_saved_followup(cid_str).await
        } else {
            None
        };

        let mut response = format!(
            "Linked '{}' to the currently configured model credential (stored encrypted). This was not sent to the LLM.",
            key
        );
        if let Some(f) = followup {
            response.push_str("\n\n");
            response.push_str(&f);
        }

        return (
            StatusCode::OK,
            Json(ChatResponse {
                response,
                proof_id: None,
                conversation_id: cid,
                conversation_title: None,
                run_id: None,
                run_status: None,
                trace_id: None,
                total_tokens: 0,
                choices: Vec::new(),
                degradation: Vec::new(),
                attempted_models: Vec::new(),
                user_outcome: None,
            }),
        )
            .into_response();
    }

    // Fast command path: push-notification controls without LLM roundtrip.
    if let Some(cmd) = parse_notification_control_command(&request.message) {
        match handle_notification_control_command(&state, cmd).await {
            Ok(response) => {
                return (
                    StatusCode::OK,
                    Json(ChatResponse {
                        response,
                        proof_id: None,
                        conversation_id: request.conversation_id.clone(),
                        conversation_title: None,
                        run_id: None,
                        run_status: None,
                        trace_id: None,
                        total_tokens: 0,
                        choices: Vec::new(),
                        degradation: Vec::new(),
                        attempted_models: Vec::new(),
                        user_outcome: None,
                    }),
                )
                    .into_response();
            }
            Err(error) => {
                return (StatusCode::BAD_REQUEST, Json(ErrorResponse { error })).into_response();
            }
        }
    }

    // Fast command path: tunnel control without LLM roundtrip.
    if let Some(cmd) = tunnel::parse_tunnel_command(&request.message) {
        match tunnel::handle_tunnel_control_command(&state, cmd).await {
            Ok(response) => {
                return (
                    StatusCode::OK,
                    Json(ChatResponse {
                        response,
                        proof_id: None,
                        conversation_id: request.conversation_id.clone(),
                        conversation_title: None,
                        run_id: None,
                        run_status: None,
                        trace_id: None,
                        total_tokens: 0,
                        choices: Vec::new(),
                        degradation: Vec::new(),
                        attempted_models: Vec::new(),
                        user_outcome: None,
                    }),
                )
                    .into_response();
            }
            Err(error) => {
                return (
                    StatusCode::INTERNAL_SERVER_ERROR,
                    Json(ErrorResponse { error }),
                )
                    .into_response();
            }
        }
    }

    // Fast command path: explicit autonomy helpers without LLM roundtrip.
    if let Some(cmd) = parse_autonomy_quick_command(&request.message) {
        match handle_autonomy_quick_command(&state, cmd).await {
            Ok(response) => {
                return (
                    StatusCode::OK,
                    Json(ChatResponse {
                        response,
                        proof_id: None,
                        conversation_id: request.conversation_id.clone(),
                        conversation_title: None,
                        run_id: None,
                        run_status: None,
                        trace_id: None,
                        total_tokens: 0,
                        choices: Vec::new(),
                        degradation: Vec::new(),
                        attempted_models: Vec::new(),
                        user_outcome: None,
                    }),
                )
                    .into_response();
            }
            Err(error) => {
                return (StatusCode::BAD_REQUEST, Json(ErrorResponse { error })).into_response();
            }
        }
    }

    let original_conversation_id = request.conversation_id.clone();
    let message = request.message.clone();
    let channel = request.channel.clone();
    let conversation_id = request.conversation_id.clone();
    let project_id: Option<String> = None;
    let execution_mode = request
        .execution_mode
        .as_deref()
        .map(str::trim)
        .filter(|value| !value.is_empty())
        .map(|value| value.chars().take(64).collect::<String>());
    if let Some(execution_mode) = execution_mode.as_deref() {
        tracing::debug!(
            execution_mode,
            "Chat request included client execution mode hint"
        );
    }
    if request.attachments_present && request.attachments.is_empty() {
        tracing::debug!(
            "Chat request indicated attachments were present, but no structured attachment hints were supplied"
        );
    }
    if request.arkorbit_context.is_some() {
        tracing::debug!("Chat request included ArkOrbit structural context");
    }
    let attachments = request.attachments.clone();
    let arkorbit_context = request.arkorbit_context.clone();
    let caller_principal = maybe_caller.as_ref().map(|Extension(value)| value.clone());
    let agent_for_chat = state.agent.clone();

    let result = {
        let worker = run_chat_worker_task("http_chat_turn", async move {
            let agent_snapshot = Agent::snapshot(&agent_for_chat).await;
            agent_snapshot
                .process_message_with_meta_and_hints(
                    &message,
                    &channel,
                    conversation_id.as_deref(),
                    project_id.as_deref(),
                    build_request_execution_hints_with_arkorbit(
                        caller_principal.as_ref(),
                        crate::actions::ActionExecutionSurface::Chat,
                        true,
                        attachments,
                        arkorbit_context,
                    ),
                )
                .await
        })
        .await;
        match worker {
            Ok(result) => result,
            Err(error) => Err(anyhow::anyhow!("Chat worker failed: {}", error)),
        }
    };

    match result {
        Ok(processed) => (
            StatusCode::OK,
            Json(ChatResponse {
                response: processed.response,
                proof_id: None,
                conversation_id: processed.conversation_id.or(original_conversation_id),
                conversation_title: processed.conversation_title,
                run_id: processed.run_id,
                run_status: processed.run_status,
                trace_id: processed.trace_id,
                total_tokens: processed.total_tokens,
                choices: processed.choices,
                degradation: processed.degradation,
                attempted_models: processed.attempted_models,
                user_outcome: processed.user_outcome,
            }),
        )
            .into_response(),
        Err(e) => {
            if e.to_string() == "Conversation not found" {
                return conversation_not_found_response();
            }
            tracing::error!("Framework-level web chat failure: {}", e);
            let response = "I hit a framework-level problem before supervised execution could finish. Please retry. If it keeps happening, restart the runtime or check the server logs.".to_string();
            let degradation = vec![crate::core::DegradationNote {
                kind: "platform".to_string(),
                summary: "framework error".to_string(),
                detail: Some(
                    "The request left the supervised execution path before completion.".to_string(),
                ),
            }];
            let user_outcome = crate::core::ExecutionSupervisor::default()
                .build_service_outage_outcome(&response, "framework_error", &degradation, &[]);
            (
                StatusCode::OK,
                Json(ChatResponse {
                    response,
                    proof_id: None,
                    conversation_id: original_conversation_id,
                    conversation_title: None,
                    run_id: None,
                    run_status: Some("platform_failed".to_string()),
                    trace_id: None,
                    total_tokens: 0,
                    choices: Vec::new(),
                    degradation,
                    attempted_models: Vec::new(),
                    user_outcome: Some(user_outcome),
                }),
            )
                .into_response()
        }
    }
}

/// Chat with the agent via SSE - streams thinking steps in real-time
pub(super) fn stream_detail_looks_like_html_payload(text: &str) -> bool {
    let trimmed = text.trim();
    if trimmed.is_empty() {
        return false;
    }
    let lower = trimmed.to_ascii_lowercase();
    lower.starts_with("<!doctype html")
        || lower.starts_with("<html")
        || (lower.contains("<html") && (lower.contains("</html>") || lower.contains("</body>")))
}

pub(super) fn stream_detail_looks_like_source_payload(text: &str) -> bool {
    let sample = text.trim().lines().take(12).collect::<Vec<_>>().join("\n");
    if sample.is_empty() {
        return false;
    }
    let lower = sample.to_ascii_lowercase();
    lower.contains("from fastapi import")
        || lower.contains("import asyncio")
        || lower.contains("import httpx")
        || lower.contains("function ")
        || lower.contains("const ")
        || lower.contains("let ")
        || lower.contains("class ")
        || lower.contains("def ")
        || lower.contains("async def ")
        || lower.contains("#include ")
}

fn compact_stream_summary_text(value: &str, max_chars: usize) -> String {
    let normalized = value.split_whitespace().collect::<Vec<_>>().join(" ");
    let mut out = String::new();
    for (idx, ch) in normalized.trim().chars().enumerate() {
        if idx >= max_chars {
            out.push_str("...");
            break;
        }
        out.push(ch);
    }
    out
}

fn humanize_stream_result_key(key: &str) -> String {
    let mut out = String::new();
    let mut previous_was_space = true;
    for ch in key.chars() {
        if ch.is_ascii_alphanumeric() {
            if previous_was_space {
                out.extend(ch.to_lowercase());
            } else {
                out.push(ch.to_ascii_lowercase());
            }
            previous_was_space = false;
        } else if !previous_was_space {
            out.push(' ');
            previous_was_space = true;
        }
    }
    out.trim().to_string()
}

fn stream_result_buckets(value: &serde_json::Value) -> Vec<(String, usize)> {
    if let Some(items) = value.as_array() {
        return vec![("results".to_string(), items.len())];
    }
    let Some(obj) = value.as_object() else {
        return Vec::new();
    };
    obj.iter()
        .filter_map(|(key, value)| {
            value
                .as_array()
                .map(|items| (humanize_stream_result_key(key), items.len()))
        })
        .collect()
}

fn summarize_stream_result_payload(
    results: &serde_json::Value,
    query: Option<&str>,
) -> Option<String> {
    let buckets = stream_result_buckets(results);
    if buckets.is_empty() {
        return None;
    }
    let total = buckets.iter().map(|(_, count)| *count).sum::<usize>();
    let query_part = query
        .map(str::trim)
        .filter(|value| !value.is_empty())
        .map(|value| format!(" for \"{}\"", compact_stream_summary_text(value, 80)))
        .unwrap_or_default();
    if total == 0 {
        return Some(if query_part.is_empty() {
            "No items returned.".to_string()
        } else {
            format!("No results found{}.", query_part)
        });
    }
    let bucket_part = buckets
        .iter()
        .filter(|(_, count)| *count > 0)
        .take(3)
        .map(|(label, count)| {
            let mut label = if label.is_empty() {
                "results".to_string()
            } else {
                label.clone()
            };
            if *count == 1 && label.ends_with('s') {
                label.pop();
            }
            format!("{} {}", count, label)
        })
        .collect::<Vec<_>>()
        .join(", ");
    let bucket_part = if bucket_part.is_empty() {
        String::new()
    } else {
        format!(" in {}", bucket_part)
    };
    let noun = if query_part.is_empty() {
        "item"
    } else {
        "result"
    };
    let verb = if query_part.is_empty() {
        "Collected"
    } else {
        "Found"
    };
    Some(format!(
        "{} {} {}{}{}{}.",
        verb,
        total,
        noun,
        if total == 1 { "" } else { "s" },
        bucket_part,
        query_part
    ))
}

fn summarize_stream_json_tool_output(value: &serde_json::Value) -> Option<String> {
    if let Some(obj) = value.as_object() {
        if let Some(raw_nested) = obj
            .get("raw_content")
            .and_then(|value| value.as_str())
            .and_then(|raw| serde_json::from_str::<serde_json::Value>(raw.trim()).ok())
        {
            if let Some(summary) = summarize_stream_json_tool_output(&raw_nested) {
                return Some(summary);
            }
        }

        if let Some(title) = obj
            .get("matched_app")
            .and_then(|v| v.get("title"))
            .and_then(|v| v.as_str())
            .map(str::trim)
            .filter(|v| !v.is_empty())
        {
            return Some(format!("Matched app and loaded metadata for {}.", title));
        }

        if let Some(results) = obj.get("results") {
            if let Some(summary) = summarize_stream_result_payload(
                results,
                obj.get("query").and_then(|value| value.as_str()),
            ) {
                return Some(summary);
            }
        }

        let buckets = stream_result_buckets(value);
        if !buckets.is_empty() {
            return summarize_stream_result_payload(value, None);
        }

        let keys = obj.keys().take(4).cloned().collect::<Vec<_>>().join(", ");
        if !keys.is_empty() {
            return Some(format!("Returned structured data: {}.", keys));
        }
    } else if let Some(items) = value.as_array() {
        return Some(format!(
            "Returned list with {} item{}.",
            items.len(),
            if items.len() == 1 { "" } else { "s" }
        ));
    }
    None
}

pub(super) fn summarize_stream_tool_activity_content(content: &str) -> String {
    let trimmed = content.trim();
    if trimmed.is_empty() {
        return String::new();
    }

    if stream_detail_looks_like_html_payload(trimmed) {
        if let Some(start) = trimmed.to_ascii_lowercase().find("<title>") {
            let rest = &trimmed[start + "<title>".len()..];
            if let Some(end) = rest.to_ascii_lowercase().find("</title>") {
                let title = rest[..end].trim();
                if !title.is_empty() {
                    return format!("Read HTML document: {}.", title);
                }
            }
        }
        return "Read HTML document.".to_string();
    }

    let json_like = (trimmed.starts_with('{') && trimmed.ends_with('}'))
        || (trimmed.starts_with('[') && trimmed.ends_with(']'));
    if json_like {
        if let Ok(value) = serde_json::from_str::<serde_json::Value>(trimmed) {
            if let Some(summary) = summarize_stream_json_tool_output(&value) {
                return summary;
            }
        }
    }

    if stream_detail_looks_like_source_payload(trimmed) {
        let line_count = trimmed.lines().count();
        return format!(
            "Read source file contents ({} line{}).",
            line_count,
            if line_count == 1 { "" } else { "s" }
        );
    }

    if trimmed.len() > 240
        && trimmed
            .chars()
            .any(|ch| matches!(ch, '{' | '}' | '<' | '>' | ';'))
    {
        return "Received detailed tool output.".to_string();
    }

    trimmed.chars().take(240).collect::<String>()
}

pub(super) fn truncate_stream_tool_raw_content(content: &str, max_chars: usize) -> String {
    let trimmed = content.trim();
    if trimmed.is_empty() {
        return String::new();
    }
    let mut out = String::new();
    for (idx, ch) in trimmed.chars().enumerate() {
        if idx >= max_chars {
            out.push_str("\n...[truncated]");
            break;
        }
        out.push(ch);
    }
    out
}

pub(super) fn normalize_stream_heartbeat_status(status: &str) -> String {
    let mut text = status
        .split_whitespace()
        .collect::<Vec<_>>()
        .join(" ")
        .trim()
        .to_string();
    if text.is_empty() {
        return "Thinking.".to_string();
    }
    let lower = text.to_ascii_lowercase();
    if lower.contains("preparing research plan") {
        return "Preparing research plan. No new output yet.".to_string();
    }
    if lower.contains("context packing") {
        return "Preparing conversation context. No new output yet.".to_string();
    }
    if lower.contains("vector memory active") {
        return "Memory/context setup in progress. No new output yet.".to_string();
    }
    if lower.contains("memory available on demand") {
        return "Still processing. No new output yet.".to_string();
    }
    if (lower.contains("waiting for") && lower.contains("respond"))
        || (lower.contains("model") && lower.contains("generating"))
    {
        return "Waiting on model response. No new output yet.".to_string();
    }
    if !text.ends_with(['.', '!', '?']) {
        text.push('.');
    }
    text
}

fn stream_surface_display_name(name: &str) -> String {
    if name.trim().eq_ignore_ascii_case("agent_turn_loop") {
        return "Working".to_string();
    }

    let cleaned = name
        .split(['_', '-', '.'])
        .filter(|part| !part.trim().is_empty())
        .map(|part| {
            let mut chars = part.chars();
            match chars.next() {
                Some(first) => {
                    let mut out = first.to_uppercase().to_string();
                    out.push_str(chars.as_str());
                    out
                }
                None => String::new(),
            }
        })
        .collect::<Vec<_>>()
        .join(" ");
    if cleaned.trim().is_empty() {
        "Tool".to_string()
    } else {
        cleaned
    }
}

fn stream_surface_renderer(name: &str, payload: &serde_json::Value) -> (String, String) {
    if let Some(renderer_id) = payload
        .get("surface")
        .and_then(|surface| surface.get("renderer"))
        .and_then(|renderer| renderer.get("id"))
        .and_then(|value| value.as_str())
        .filter(|value| !value.trim().is_empty())
    {
        let fallback = payload
            .get("surface")
            .and_then(|surface| surface.get("renderer"))
            .and_then(|renderer| renderer.get("fallback"))
            .and_then(|value| value.as_str())
            .unwrap_or("generic-artifact");
        return (renderer_id.to_string(), fallback.to_string());
    }

    let kind = payload
        .get("kind")
        .and_then(|value| value.as_str())
        .unwrap_or_default();
    if kind == "console_chunk" {
        return (
            "agentark.terminal.transcript.v1".to_string(),
            "generic-artifact".to_string(),
        );
    }
    if kind == "draft_file" || kind == "file_write" {
        return (
            "agentark.file.editor.v1".to_string(),
            "generic-artifact".to_string(),
        );
    }

    let action = crate::actions::ActionDef {
        name: name.to_string(),
        ..crate::actions::ActionDef::default()
    };
    let metadata = crate::actions::action_metadata_for_action(&action);
    match metadata.integration_class {
        crate::actions::ActionIntegrationClass::Code => (
            "agentark.terminal.transcript.v1".to_string(),
            "generic-artifact".to_string(),
        ),
        crate::actions::ActionIntegrationClass::Filesystem => (
            "agentark.file.editor.v1".to_string(),
            "generic-artifact".to_string(),
        ),
        crate::actions::ActionIntegrationClass::Search => (
            "agentark.search.results.v1".to_string(),
            "generic-artifact".to_string(),
        ),
        crate::actions::ActionIntegrationClass::Browser => (
            "agentark.browser.reader.v1".to_string(),
            "generic-artifact".to_string(),
        ),
        crate::actions::ActionIntegrationClass::App => (
            "agentark.app.deploy.v1".to_string(),
            "generic-artifact".to_string(),
        ),
        crate::actions::ActionIntegrationClass::Media => (
            "agentark.artifact.image.v1".to_string(),
            "generic-artifact".to_string(),
        ),
        _ => (
            "agentark.artifact.generic.v1".to_string(),
            "generic-artifact".to_string(),
        ),
    }
}

fn stream_surface_status_from_result(content: &str) -> &'static str {
    let trimmed = content.trim();
    if let Some(payload) = trimmed
        .strip_prefix(crate::runtime::TOOL_COMPLETION_MARKER)
        .map(str::trim)
    {
        if let Ok(value) = serde_json::from_str::<serde_json::Value>(payload) {
            let status = value
                .get("status")
                .and_then(|value| value.as_str())
                .unwrap_or_default();
            return match status {
                "failed" | "error" | "blocked" | "invalid" => "error",
                _ => "done",
            };
        }
    }
    if let Ok(value) = serde_json::from_str::<serde_json::Value>(trimmed) {
        if value.get("error").is_some() {
            return "error";
        }
        if let Some(status) = value.get("status").and_then(|value| value.as_str()) {
            return match status {
                "failed" | "error" | "blocked" | "invalid" => "error",
                _ => "done",
            };
        }
    }
    "done"
}

fn stream_surface_call_id(name: &str, payload: &serde_json::Value) -> String {
    for key in ["call_id", "stream_key", "__streamKey", "id"] {
        if let Some(value) = payload
            .get(key)
            .and_then(|value| value.as_str())
            .map(str::trim)
            .filter(|value| !value.is_empty())
        {
            return value.to_string();
        }
    }
    let run_id = payload
        .get("run_id")
        .and_then(|value| value.as_str())
        .map(str::trim)
        .filter(|value| !value.is_empty())
        .unwrap_or("live");
    format!("{}:{}", run_id, name)
}

fn push_surface_artifact_from_field(
    artifacts: &mut Vec<serde_json::Value>,
    role: &str,
    content_type: &str,
    label: &str,
    path: Option<&str>,
    uri: Option<&str>,
    text: Option<&str>,
) {
    let id_source = path.or(uri).unwrap_or(label);
    let mut artifact = serde_json::Map::new();
    artifact.insert(
        "id".to_string(),
        serde_json::json!(format!("{}:{}", role, id_source)),
    );
    artifact.insert("role".to_string(), serde_json::json!(role));
    artifact.insert("contentType".to_string(), serde_json::json!(content_type));
    artifact.insert("label".to_string(), serde_json::json!(label));
    if let Some(path) = path.filter(|value| !value.trim().is_empty()) {
        artifact.insert("path".to_string(), serde_json::json!(path));
    }
    if let Some(uri) = uri.filter(|value| !value.trim().is_empty()) {
        artifact.insert("uri".to_string(), serde_json::json!(uri));
    }
    if let Some(text) = text.filter(|value| !value.trim().is_empty()) {
        artifact.insert("text".to_string(), serde_json::json!(text));
    }
    artifacts.push(serde_json::Value::Object(artifact));
}

fn stream_surface_artifacts(payload: &serde_json::Value) -> Vec<serde_json::Value> {
    let mut artifacts = Vec::new();
    if let Some(path) = payload
        .get("path")
        .or_else(|| payload.get("file"))
        .or_else(|| payload.get("name"))
        .and_then(|value| value.as_str())
    {
        push_surface_artifact_from_field(
            &mut artifacts,
            "file",
            "text/plain",
            path,
            Some(path),
            None,
            payload
                .get("raw_content")
                .or_else(|| payload.get("file_content"))
                .or_else(|| payload.get("content_snapshot"))
                .or_else(|| payload.get("content"))
                .and_then(|value| value.as_str()),
        );
    }
    if let Some(url) = payload
        .get("url")
        .or_else(|| payload.get("uri"))
        .or_else(|| payload.get("href"))
        .or_else(|| payload.get("app_url"))
        .or_else(|| payload.get("access_url"))
        .and_then(|value| value.as_str())
    {
        push_surface_artifact_from_field(
            &mut artifacts,
            "url",
            "text/uri-list",
            url,
            None,
            Some(url),
            None,
        );
    }
    if let Some(files) = payload.get("files").and_then(|value| value.as_object()) {
        for (path, content) in files {
            push_surface_artifact_from_field(
                &mut artifacts,
                "file",
                "text/plain",
                path,
                Some(path),
                None,
                content.as_str(),
            );
        }
    }
    if let Some(names) = payload.get("file_names").and_then(|value| value.as_array()) {
        for value in names {
            if let Some(path) = value.as_str() {
                push_surface_artifact_from_field(
                    &mut artifacts,
                    "file",
                    "text/plain",
                    path,
                    Some(path),
                    None,
                    None,
                );
            }
        }
    }
    artifacts
}

fn stream_surface_payload(
    name: &str,
    status: &'static str,
    payload: &serde_json::Value,
    content: Option<&str>,
) -> serde_json::Value {
    let (renderer_id, fallback) = stream_surface_renderer(name, payload);
    let display_name = stream_surface_display_name(name);
    let surface_title = payload
        .get("label")
        .or_else(|| payload.get("title"))
        .and_then(|value| value.as_str())
        .filter(|value| !value.trim().is_empty())
        .unwrap_or(&display_name);
    let call_id = stream_surface_call_id(name, payload);
    let run_id = payload.get("run_id").and_then(|value| value.as_str());
    let sequence = payload.get("seq").and_then(|value| value.as_u64());
    let mut input = Vec::new();
    if status == "running" || status == "pending" {
        input.push(serde_json::json!({
            "role": "arguments",
            "contentType": "application/json",
            "json": payload,
            "preview": payload.get("content").and_then(|value| value.as_str()).unwrap_or_default(),
        }));
        if let Some(command) = payload
            .get("command")
            .or_else(|| payload.get("cmd"))
            .and_then(|value| value.as_str())
        {
            input.push(serde_json::json!({
                "role": "command",
                "contentType": "text/x-shell-command",
                "text": command,
            }));
        }
    }
    let mut output = Vec::new();
    if let Some(content) = content.filter(|value| !value.trim().is_empty()) {
        output.push(serde_json::json!({
            "role": if renderer_id == "agentark.terminal.transcript.v1" { "transcript" } else { "output" },
            "contentType": if renderer_id == "agentark.terminal.transcript.v1" { "text/x-agentark-terminal-transcript" } else { "text/plain" },
            "text": content,
        }));
    }
    serde_json::json!({
        "protocolVersion": 1,
        "renderer": {
            "id": renderer_id,
            "version": 1,
            "fallback": fallback,
        },
        "call": {
            "runId": run_id,
            "callId": call_id,
            "sequence": sequence,
        },
        "tool": {
            "id": name,
            "displayName": display_name,
        },
        "status": status,
        "title": surface_title,
        "input": input,
        "output": output,
        "artifacts": stream_surface_artifacts(payload),
    })
}

fn attach_stream_surface(
    payload: &mut serde_json::Map<String, serde_json::Value>,
    name: &str,
    status: &'static str,
    content: Option<&str>,
) {
    let payload_value = serde_json::Value::Object(payload.clone());
    payload.insert(
        "surface".to_string(),
        stream_surface_payload(name, status, &payload_value, content),
    );
}

pub(super) fn normalize_stream_event_for_sse(
    ev: crate::core::StreamEvent,
    last_thinking_detail: &str,
) -> (Option<(&'static str, serde_json::Value)>, String) {
    match ev {
        crate::core::StreamEvent::RunStarted {
            run_id,
            flow_kind,
            origin,
            conversation_id,
            trace_id,
            resumed,
        } => (
            Some((
                "run_started",
                serde_json::json!({
                    "run_id": run_id,
                    "flow_kind": flow_kind,
                    "origin": origin,
                    "conversation_id": conversation_id,
                    "trace_id": trace_id,
                    "resumed": resumed,
                }),
            )),
            String::new(),
        ),
        crate::core::StreamEvent::ChatTaskStarted {
            task_id,
            description,
            work_type,
            conversation_id,
        } => (
            Some((
                "task_started",
                serde_json::json!({
                    "task_id": task_id,
                    "description": description,
                    "status": "in_progress",
                    "work_type": work_type,
                    "conversation_id": conversation_id,
                }),
            )),
            String::new(),
        ),
        crate::core::StreamEvent::Token(content) => (
            Some(("token", serde_json::json!({ "content": content }))),
            String::new(),
        ),
        crate::core::StreamEvent::Thinking(status) => {
            let detail = normalize_stream_heartbeat_status(&status);
            if detail == last_thinking_detail {
                (None, detail)
            } else {
                (
                    Some((
                        "thinking",
                        serde_json::json!({
                            "__streamKey": "public-thinking",
                            "step_type": "thinking",
                            "title": "Thinking",
                            "detail": detail
                        }),
                    )),
                    detail,
                )
            }
        }
        crate::core::StreamEvent::ReasoningDelta {
            phase,
            content_delta,
            done,
        } => {
            let normalized_phase = phase.trim();
            let phase_visible = {
                let normalized = normalized_phase.to_ascii_lowercase();
                normalized.ends_with("_summary") || normalized.contains("summary")
            };
            if !phase_visible {
                return (None, String::new());
            }
            let stream_key = if normalized_phase.is_empty() {
                "reasoning:active".to_string()
            } else {
                format!("reasoning:{}", normalized_phase)
            };
            let detail = if !content_delta.trim().is_empty() {
                content_delta.trim().to_string()
            } else if done {
                "Reasoning summary completed.".to_string()
            } else {
                "Reasoning summary in progress.".to_string()
            };
            let content = content_delta.clone();
            (
                Some((
                    "reasoning_delta",
                    serde_json::json!({
                        "kind": "reasoning_delta",
                        "phase": normalized_phase,
                        "title": "Reasoning summary",
                        "detail": detail,
                        "content": content,
                        "content_delta": content_delta,
                        "done": done,
                        "stream_key": stream_key,
                    }),
                )),
                String::new(),
            )
        }
        crate::core::StreamEvent::ToolStart { name, payload } => {
            let mut payload_json = if let Some(payload) = payload {
                if let Some(obj) = payload.as_object() {
                    let mut merged = serde_json::Map::new();
                    merged.insert("name".to_string(), serde_json::json!(name.clone()));
                    for (k, v) in obj {
                        merged.insert(k.clone(), v.clone());
                    }
                    serde_json::Value::Object(merged)
                } else {
                    serde_json::json!({ "name": name.clone(), "payload": payload })
                }
            } else {
                serde_json::json!({ "name": name.clone() })
            };
            if let Some(obj) = payload_json.as_object_mut() {
                attach_stream_surface(obj, &name, "running", None);
            }
            (Some(("tool_start", payload_json)), String::new())
        }
        crate::core::StreamEvent::ToolResult { name, content } => {
            let summarized = summarize_stream_tool_activity_content(&content);
            let trimmed = content.trim();
            let surface_status = stream_surface_status_from_result(&content);
            let mut payload = serde_json::Map::new();
            payload.insert("name".to_string(), serde_json::json!(name.clone()));
            payload.insert("content".to_string(), serde_json::json!(summarized));
            if !trimmed.is_empty() {
                if let Ok(value) = serde_json::from_str::<serde_json::Value>(trimmed) {
                    if let Some(obj) = value.as_object() {
                        for (k, v) in obj {
                            if matches!(k.as_str(), "name" | "content" | "raw_content" | "result") {
                                continue;
                            }
                            payload.insert(k.clone(), v.clone());
                        }
                    } else {
                        payload.insert("result".to_string(), value);
                    }
                } else {
                    payload.insert(
                        "raw_content".to_string(),
                        serde_json::json!(truncate_stream_tool_raw_content(trimmed, 65536)),
                    );
                }
            }
            attach_stream_surface(
                &mut payload,
                &name,
                surface_status,
                (!trimmed.is_empty()).then_some(trimmed),
            );
            (
                Some(("tool_result", serde_json::Value::Object(payload))),
                String::new(),
            )
        }
        crate::core::StreamEvent::ToolProgress {
            name,
            content,
            payload,
        } => {
            let content = summarize_stream_tool_activity_content(&content);
            let mut payload_json = if let Some(payload) = payload {
                if let Some(obj) = payload.as_object() {
                    let mut merged = serde_json::Map::new();
                    merged.insert("name".to_string(), serde_json::json!(name.clone()));
                    merged.insert("content".to_string(), serde_json::json!(content.clone()));
                    for (k, v) in obj {
                        merged.insert(k.clone(), v.clone());
                    }
                    serde_json::Value::Object(merged)
                } else {
                    serde_json::json!({ "name": name.clone(), "content": content.clone(), "payload": payload })
                }
            } else {
                serde_json::json!({ "name": name.clone(), "content": content.clone() })
            };
            if let Some(obj) = payload_json.as_object_mut() {
                attach_stream_surface(obj, &name, "running", Some(&content));
            }
            (Some(("tool_progress", payload_json)), String::new())
        }
        crate::core::StreamEvent::PlanGenerated { plan } => (
            Some((
                "plan_generated",
                serde_json::json!({
                    "step_type": "plan_generated",
                    "title": "Execution Plan",
                    "detail": format!("{} steps planned", plan.steps.len()),
                    "plan": plan,
                }),
            )),
            String::new(),
        ),
        crate::core::StreamEvent::PlanStepUpdate {
            plan_id,
            revision,
            step_id,
            step_title,
            status,
            detail,
            substeps,
        } => {
            let title = match status {
                crate::core::PlanStepStatus::Pending => "Plan Step Queued",
                crate::core::PlanStepStatus::Running => "Plan Step Started",
                crate::core::PlanStepStatus::Completed => "Plan Step Completed",
                crate::core::PlanStepStatus::Failed => "Plan Step Failed",
                crate::core::PlanStepStatus::Skipped => "Plan Step Skipped",
            };
            let step_title_text = step_title
                .clone()
                .filter(|value| !value.trim().is_empty())
                .unwrap_or_else(|| format!("Step {}", step_id));
            let detail_text = detail.unwrap_or_else(|| match status {
                crate::core::PlanStepStatus::Pending => format!("Queued {}.", step_title_text),
                crate::core::PlanStepStatus::Running => format!("Started {}.", step_title_text),
                crate::core::PlanStepStatus::Completed => {
                    format!("Completed {}.", step_title_text)
                }
                crate::core::PlanStepStatus::Failed => format!("Failed {}.", step_title_text),
                crate::core::PlanStepStatus::Skipped => format!("Skipped {}.", step_title_text),
            });
            (
                Some((
                    "plan_step_update",
                    serde_json::json!({
                        "step_type": "plan_step_update",
                        "title": title,
                        "plan_id": plan_id,
                        "revision": revision,
                        "step_id": step_id,
                        "step_title": step_title,
                        "status": status,
                        "detail": detail_text,
                        "substeps": substeps,
                    }),
                )),
                String::new(),
            )
        }
    }
}

fn stream_payload_has_surface(payload: &serde_json::Map<String, serde_json::Value>) -> bool {
    payload
        .get("surface")
        .and_then(|surface| surface.get("renderer"))
        .and_then(|renderer| renderer.get("id"))
        .and_then(|value| value.as_str())
        .is_some_and(|value| !value.trim().is_empty())
}

pub(super) fn run_event_to_sse_payload(run_event: &crate::core::RunEvent) -> serde_json::Value {
    let event_kind = run_event.kind.as_str();
    let incoming_payload = run_event.payload.as_object().cloned().unwrap_or_default();
    let mut payload = match event_kind {
        "thinking" => {
            let mut merged = run_event.payload.as_object().cloned().unwrap_or_default();
            if !merged.contains_key("step_type") {
                merged.insert("step_type".to_string(), serde_json::json!("thinking"));
            }
            if !merged.contains_key("title") {
                merged.insert("title".to_string(), serde_json::json!("Thinking"));
            }
            if !merged.contains_key("__streamKey") {
                merged.insert(
                    "__streamKey".to_string(),
                    serde_json::json!("public-thinking"),
                );
            }
            merged
        }
        "tool_start" => {
            if stream_payload_has_surface(&incoming_payload) {
                incoming_payload
            } else {
                let name = run_event
                    .payload
                    .get("name")
                    .and_then(|value| value.as_str())
                    .unwrap_or_default()
                    .to_string();
                if let Some(inner) = run_event
                    .payload
                    .get("payload")
                    .and_then(|value| value.as_object())
                {
                    let mut merged = serde_json::Map::new();
                    merged.insert("name".to_string(), serde_json::json!(name));
                    for (key, value) in inner {
                        merged.insert(key.clone(), value.clone());
                    }
                    merged
                } else {
                    serde_json::json!({ "name": name })
                        .as_object()
                        .cloned()
                        .unwrap_or_default()
                }
            }
        }
        "tool_progress" => {
            if stream_payload_has_surface(&incoming_payload) {
                incoming_payload
            } else {
                let name = run_event
                    .payload
                    .get("name")
                    .and_then(|value| value.as_str())
                    .unwrap_or_default()
                    .to_string();
                let content = summarize_stream_tool_activity_content(
                    run_event
                        .payload
                        .get("content")
                        .and_then(|value| value.as_str())
                        .unwrap_or_default(),
                );
                let mut merged = serde_json::Map::new();
                merged.insert("name".to_string(), serde_json::json!(name));
                merged.insert("content".to_string(), serde_json::json!(content));
                if let Some(obj) = run_event.payload.as_object() {
                    for (key, value) in obj {
                        if matches!(key.as_str(), "name" | "content" | "payload") {
                            continue;
                        }
                        merged.insert(key.clone(), value.clone());
                    }
                }
                if let Some(inner) = run_event
                    .payload
                    .get("payload")
                    .and_then(|value| value.as_object())
                {
                    for (key, value) in inner {
                        if matches!(key.as_str(), "name" | "content") {
                            continue;
                        }
                        merged.insert(key.clone(), value.clone());
                    }
                    merged
                } else {
                    merged
                }
            }
        }
        "tool_result" => {
            if stream_payload_has_surface(&incoming_payload) {
                incoming_payload
            } else {
                let name = run_event
                    .payload
                    .get("name")
                    .and_then(|value| value.as_str())
                    .unwrap_or_default()
                    .to_string();
                let raw_content = run_event
                    .payload
                    .get("content")
                    .and_then(|value| value.as_str())
                    .unwrap_or_default()
                    .to_string();
                let summarized = summarize_stream_tool_activity_content(&raw_content);
                let trimmed = raw_content.trim();
                let mut merged = serde_json::Map::new();
                merged.insert("name".to_string(), serde_json::json!(name));
                merged.insert("content".to_string(), serde_json::json!(summarized));
                if !trimmed.is_empty() {
                    if let Ok(value) = serde_json::from_str::<serde_json::Value>(trimmed) {
                        if let Some(obj) = value.as_object() {
                            for (key, value) in obj {
                                if matches!(
                                    key.as_str(),
                                    "name" | "content" | "raw_content" | "result"
                                ) {
                                    continue;
                                }
                                merged.insert(key.clone(), value.clone());
                            }
                        } else {
                            merged
                                .insert("raw_content".to_string(), serde_json::json!(raw_content));
                        }
                    } else {
                        merged.insert("raw_content".to_string(), serde_json::json!(raw_content));
                    }
                }
                merged
            }
        }
        "plan_generated" => {
            let plan = run_event.payload.get("plan").cloned().unwrap_or_default();
            let step_count = plan
                .get("steps")
                .and_then(|value| value.as_array())
                .map(|steps| steps.len())
                .unwrap_or(0);
            serde_json::json!({
                "step_type": "plan_generated",
                "title": "Execution Plan",
                "detail": format!("{} steps planned", step_count),
                "plan": plan,
            })
            .as_object()
            .cloned()
            .unwrap_or_default()
        }
        "plan_revised" => {
            let plan = run_event.payload.get("plan").cloned().unwrap_or_default();
            let step_count = plan
                .get("steps")
                .and_then(|value| value.as_array())
                .map(|steps| steps.len())
                .unwrap_or(0);
            let detail = run_event
                .payload
                .get("reason")
                .and_then(|value| value.as_str())
                .map(str::to_string)
                .unwrap_or_else(|| format!("Plan revised to {} steps.", step_count));
            serde_json::json!({
                "step_type": "plan_revised",
                "title": "Execution Plan Revised",
                "detail": detail,
                "plan": plan,
            })
            .as_object()
            .cloned()
            .unwrap_or_default()
        }
        "plan_unavailable" => {
            let detail = run_event
                .payload
                .get("reason")
                .and_then(|value| value.as_str())
                .unwrap_or("Structured planning was unavailable.")
                .to_string();
            serde_json::json!({
                "step_type": "plan_unavailable",
                "title": "Execution Plan Unavailable",
                "detail": detail,
            })
            .as_object()
            .cloned()
            .unwrap_or_default()
        }
        "plan_step_update" => {
            let detail = run_event
                .payload
                .get("detail")
                .and_then(|value| value.as_str())
                .map(str::to_string)
                .unwrap_or_else(|| {
                    format!(
                        "Updated step {}",
                        run_event
                            .payload
                            .get("step_id")
                            .and_then(|value| value.as_u64())
                            .unwrap_or(0)
                    )
                });
            let mut merged = run_event.payload.as_object().cloned().unwrap_or_default();
            merged.insert(
                "step_type".to_string(),
                serde_json::json!("plan_step_update"),
            );
            merged.insert("title".to_string(), serde_json::json!("Plan Step Update"));
            merged.insert("detail".to_string(), serde_json::json!(detail));
            merged
        }
        _ => run_event.payload.as_object().cloned().unwrap_or_default(),
    };
    payload.insert("run_id".to_string(), serde_json::json!(run_event.run_id));
    payload.insert("seq".to_string(), serde_json::json!(run_event.seq));
    payload.insert("ts".to_string(), serde_json::json!(run_event.ts));
    payload.insert(
        "flow_kind".to_string(),
        serde_json::json!(run_event.flow_kind),
    );
    payload.insert("origin".to_string(), serde_json::json!(run_event.origin));
    payload.insert(
        "priority".to_string(),
        serde_json::json!(run_event.priority),
    );
    if let Some(stage) = &run_event.stage {
        payload.insert("stage".to_string(), serde_json::json!(stage));
    }
    if matches!(event_kind, "tool_start" | "tool_progress" | "tool_result")
        && !stream_payload_has_surface(&payload)
    {
        if let Some(name) = payload
            .get("name")
            .and_then(|value| value.as_str())
            .map(str::to_string)
            .filter(|value| !value.trim().is_empty())
        {
            let status = if event_kind == "tool_result" {
                stream_surface_status_from_result(
                    payload
                        .get("raw_content")
                        .or_else(|| payload.get("content"))
                        .and_then(|value| value.as_str())
                        .unwrap_or_default(),
                )
            } else {
                "running"
            };
            let content = payload
                .get("raw_content")
                .or_else(|| payload.get("content"))
                .and_then(|value| value.as_str())
                .map(str::to_string);
            attach_stream_surface(&mut payload, &name, status, content.as_deref());
        }
    }
    serde_json::Value::Object(payload)
}

pub(super) fn run_event_to_sse_event(run_event: crate::core::RunEvent) -> Event {
    let event_kind = run_event.kind.clone();
    let payload = run_event_to_sse_payload(&run_event);
    Event::default()
        .event(event_kind)
        .data(serde_json::to_string(&payload).unwrap_or_default())
}

pub(super) fn truncate_stream_task_text(text: &str, max_chars: usize) -> String {
    let trimmed = text.trim();
    if trimmed.is_empty() {
        return String::new();
    }
    let mut out = String::new();
    for (idx, ch) in trimmed.chars().enumerate() {
        if idx >= max_chars {
            out.push_str("...");
            break;
        }
        out.push(ch);
    }
    out
}

pub(super) fn chat_task_status_key(status: &crate::core::TaskStatus) -> &'static str {
    match status {
        crate::core::TaskStatus::Pending => "pending",
        crate::core::TaskStatus::AwaitingApproval => "awaiting_approval",
        crate::core::TaskStatus::ExpiredNeedsReapproval => "expired_needs_reapproval",
        crate::core::TaskStatus::Paused => "paused",
        crate::core::TaskStatus::InProgress => "in_progress",
        crate::core::TaskStatus::Completed => "completed",
        crate::core::TaskStatus::Failed { .. } => "failed",
        crate::core::TaskStatus::Cancelled => "cancelled",
    }
}

pub(super) fn chat_task_terminal_status(response: &str) -> crate::core::TaskStatus {
    let lower = response.trim().to_ascii_lowercase();
    if lower.contains("waiting for your approval")
        || lower.contains("waiting for your input")
        || lower.contains("reply with approval")
        || lower.contains("needs your approval")
        || lower.contains("requires approval")
        || lower.contains("api key")
    {
        crate::core::TaskStatus::Paused
    } else {
        crate::core::TaskStatus::Completed
    }
}

pub(super) fn deep_research_failure_reason(
    response: &str,
    run_status: Option<&str>,
    user_outcome: Option<&crate::core::UserFacingOutcome>,
) -> String {
    let outcome_message = user_outcome
        .map(|outcome| truncate_stream_task_text(&outcome.message, 240))
        .filter(|value| !value.is_empty());
    if let Some(message) = outcome_message {
        return message;
    }

    let response_preview = truncate_stream_task_text(response, 240);
    if !response_preview.is_empty() {
        return response_preview;
    }

    let normalized = run_status
        .map(|value| value.trim().to_ascii_lowercase())
        .filter(|value| !value.is_empty())
        .unwrap_or_else(|| "failed".to_string())
        .replace('_', " ");
    format!("Deep research {}.", normalized)
}

pub(super) fn deep_research_terminal_status(
    response: &str,
    run_status: Option<&str>,
    user_outcome: Option<&crate::core::UserFacingOutcome>,
) -> crate::core::TaskStatus {
    let normalized_run_status = run_status
        .map(|value| value.trim().to_ascii_lowercase())
        .filter(|value| !value.is_empty());

    match normalized_run_status.as_deref() {
        Some("degraded") | Some("platform_failed") => crate::core::TaskStatus::Failed {
            error: deep_research_failure_reason(response, run_status, user_outcome),
        },
        Some("needs_input") | Some("blocked") | Some("needs_stronger_model") => {
            crate::core::TaskStatus::Paused
        }
        Some("cancelled") => crate::core::TaskStatus::Cancelled,
        Some("completed") => chat_task_terminal_status(response),
        _ => match user_outcome.map(|outcome| &outcome.status) {
            Some(crate::core::UserFacingOutcomeStatus::Degraded)
            | Some(crate::core::UserFacingOutcomeStatus::ServiceUnavailable) => {
                crate::core::TaskStatus::Failed {
                    error: deep_research_failure_reason(response, run_status, user_outcome),
                }
            }
            Some(crate::core::UserFacingOutcomeStatus::NeedsClarification)
            | Some(crate::core::UserFacingOutcomeStatus::NeedsPermission)
            | Some(crate::core::UserFacingOutcomeStatus::NeedsIntegration)
            | Some(crate::core::UserFacingOutcomeStatus::NeedsCredentials)
            | Some(crate::core::UserFacingOutcomeStatus::NeedsStrongerModel) => {
                crate::core::TaskStatus::Paused
            }
            _ => chat_task_terminal_status(response),
        },
    }
}

fn deep_research_plan_id(task_id: &str) -> String {
    let suffix = task_id
        .chars()
        .filter(|ch| ch.is_ascii_hexdigit())
        .take(8)
        .collect::<String>();
    if suffix.is_empty() {
        "deep-research-plan".to_string()
    } else {
        format!("deep-research-{}", suffix)
    }
}

fn deep_research_topic_label(message: &str) -> String {
    let label = truncate_stream_task_text(message.trim(), 120);
    if label.is_empty() {
        "this topic".to_string()
    } else {
        label
    }
}

fn deep_research_plan_text(value: &str, max_chars: usize) -> String {
    let compact = value
        .trim()
        .split_whitespace()
        .collect::<Vec<_>>()
        .join(" ");
    truncate_stream_task_text(&compact, max_chars)
}

fn deep_research_plan_focus_label(message: &str) -> (String, Vec<String>) {
    let (objective, items) =
        crate::core::planner::confirmation_request_objective_and_items(message);
    let objective = if objective.trim().is_empty() {
        deep_research_topic_label(message)
    } else {
        deep_research_plan_text(&objective, 120)
    };
    let mut seen = std::collections::HashSet::new();
    let items = items
        .into_iter()
        .map(|item| deep_research_plan_text(&item, 96))
        .filter(|item| !item.trim().is_empty())
        .filter(|item| seen.insert(item.to_ascii_lowercase()))
        .take(4)
        .collect::<Vec<_>>();
    (objective, items)
}

fn deep_research_plan_step(
    id: usize,
    title: impl Into<String>,
    description: impl Into<String>,
    message: &str,
) -> crate::core::PlanStep {
    crate::core::PlanStep {
        id,
        title: title.into(),
        description: description.into(),
        action: Some("research".to_string()),
        arguments: Some(serde_json::json!({
            "query": message,
            "depth": "deep",
        })),
        tool_hint: Some("research".to_string()),
        status: Some(crate::core::PlanStepStatus::Pending),
        substeps: Vec::new(),
    }
}

fn default_deep_research_plan(message: &str, task_id: &str) -> crate::core::ExecutionPlan {
    let (objective, focus_items) = deep_research_plan_focus_label(message);
    let mut steps = Vec::new();
    steps.push(deep_research_plan_step(
        1,
        format!("Frame {}", objective),
        "Turn the request into source requirements, freshness needs, and verification criteria.",
        message,
    ));
    steps.push(deep_research_plan_step(
        2,
        format!("Search evidence for {}", objective),
        "Gather diverse primary, recent, comparative, and skeptical source candidates.",
        message,
    ));
    for focus in focus_items {
        let id = steps.len() + 1;
        steps.push(deep_research_plan_step(
            id,
            format!("Investigate {}", focus),
            "Read selected sources for this requested dimension and extract cited evidence.",
            message,
        ));
    }
    steps.push(deep_research_plan_step(
        steps.len() + 1,
        format!("Synthesize {}", objective),
        "Compare the evidence, surface disagreements or gaps, and write the cited report.",
        message,
    ));

    crate::core::ExecutionPlan {
        plan_id: deep_research_plan_id(task_id),
        revision: 1,
        summary: format!(
            "Research {} with source discovery, evidence reading, verification, and cited synthesis.",
            objective
        ),
        steps,
    }
}

fn normalize_deep_research_plan(
    mut plan: crate::core::ExecutionPlan,
    message: &str,
    task_id: &str,
) -> crate::core::ExecutionPlan {
    if plan.plan_id.trim().is_empty() {
        plan.plan_id = deep_research_plan_id(task_id);
    }
    if plan.revision == 0 {
        plan.revision = 1;
    }
    if plan.summary.trim().is_empty() {
        plan.summary = default_deep_research_plan(message, task_id).summary;
    }
    if plan.steps.is_empty() {
        plan.steps = default_deep_research_plan(message, task_id).steps;
    }
    for (index, step) in plan.steps.iter_mut().enumerate() {
        if step.id == 0 {
            step.id = index + 1;
        }
        if step.title.trim().is_empty() {
            step.title = format!("Research step {}", index + 1);
        }
        step.status = Some(crate::core::PlanStepStatus::Pending);
        for substep in &mut step.substeps {
            substep.status = Some(crate::core::PlanStepStatus::Pending);
        }
        let maps_to_research = step
            .action
            .as_deref()
            .or(step.tool_hint.as_deref())
            .map(|value| value.trim().eq_ignore_ascii_case("research"))
            .unwrap_or(false);
        if maps_to_research && step.arguments.is_none() {
            step.arguments = Some(serde_json::json!({
                "query": message,
                "depth": "deep",
            }));
        }
    }
    plan
}

fn deep_research_plan_from_value(
    value: Option<&serde_json::Value>,
    message: &str,
    task_id: &str,
) -> crate::core::ExecutionPlan {
    value
        .cloned()
        .and_then(|raw| serde_json::from_value::<crate::core::ExecutionPlan>(raw).ok())
        .map(|plan| normalize_deep_research_plan(plan, message, task_id))
        .unwrap_or_else(|| default_deep_research_plan(message, task_id))
}

fn deep_research_plan_timeout_ms() -> u64 {
    std::env::var("AGENTARK_DEEP_RESEARCH_PLAN_TIMEOUT_MS")
        .ok()
        .and_then(|value| value.trim().parse::<u64>().ok())
        .filter(|value| *value >= 1_000)
        .unwrap_or(45_000)
        .min(180_000)
}

fn deep_research_plan_actions(
    actions: Vec<crate::actions::ActionDef>,
) -> Vec<crate::actions::ActionDef> {
    let scoped = actions
        .iter()
        .filter(|action| action.name.trim().eq_ignore_ascii_case("research"))
        .cloned()
        .collect::<Vec<_>>();
    if scoped.is_empty() {
        actions
    } else {
        scoped
    }
}

async fn generate_deep_research_plan_preview(
    agent: &Agent,
    message: &str,
    task_id: &str,
) -> crate::core::ExecutionPlan {
    let fallback = default_deep_research_plan(message, task_id);
    let actions = agent
        .runtime
        .list_enabled_actions()
        .await
        .map(deep_research_plan_actions)
        .unwrap_or_default();
    let (system, user) =
        crate::core::planner::build_confirmation_plan_prompt(message, None, &actions);
    let planner = agent
        .llm_for_role(&crate::core::ModelRole::Research)
        .clone();
    let response = tokio::time::timeout(
        std::time::Duration::from_millis(deep_research_plan_timeout_ms()),
        planner.chat_classifier_bounded(&system, &user, 1_200),
    )
    .await;
    let Ok(Ok(response)) = response else {
        return fallback;
    };
    let Some(plan) = crate::core::planner::parse_plan_from_llm_content(
        &response.content,
        &actions,
        Some(deep_research_plan_id(task_id)),
        1,
        false,
    ) else {
        return fallback;
    };
    let plan = normalize_deep_research_plan(plan, message, task_id);
    let relevance = crate::core::planner::assess_confirmation_plan_relevance(message, &plan);
    if relevance.accepted {
        plan
    } else {
        fallback
    }
}

fn deep_research_step_for_phase(plan: &crate::core::ExecutionPlan, phase: &str) -> (usize, String) {
    let count = plan.steps.len().max(1);
    let ordinal = match phase.trim().to_ascii_lowercase().as_str() {
        "planning" => 1,
        "searching" | "ranking" => {
            if count >= 4 {
                2
            } else {
                1
            }
        }
        "reading" => {
            if count >= 4 {
                3
            } else if count >= 3 {
                2
            } else {
                count
            }
        }
        "synthesis" => count,
        _ => count.min(2),
    }
    .clamp(1, count);

    plan.steps
        .get(ordinal - 1)
        .map(|step| (step.id, step.title.clone()))
        .unwrap_or((ordinal, format!("Research step {}", ordinal)))
}

async fn send_deep_research_plan_step_update(
    tx: &tokio::sync::mpsc::Sender<std::result::Result<Event, std::convert::Infallible>>,
    live_runs: &std::sync::Arc<crate::core::LiveRunRegistry>,
    run_id: &str,
    channel: &str,
    plan: &crate::core::ExecutionPlan,
    step_id: usize,
    step_title: String,
    status: &str,
    detail: impl Into<String>,
) {
    send_chat_stream_event(
        tx,
        live_runs,
        run_id,
        "chat",
        channel,
        "plan_step_update",
        serde_json::json!({
            "step_type": "plan_step_update",
            "plan_id": plan.plan_id,
            "revision": plan.revision,
            "step_id": step_id,
            "step_title": step_title,
            "status": status,
            "detail": detail.into(),
        }),
        crate::core::RunEventPriority::High,
    )
    .await;
}

async fn persist_deep_research_chat_message(
    agent: &Agent,
    conversation_id: Option<&str>,
    role: &str,
    content: &str,
    model_used: Option<&str>,
    trace_id: Option<&str>,
) {
    let Some(conversation_id) = conversation_id
        .map(str::trim)
        .filter(|value| !value.is_empty())
    else {
        return;
    };
    let should_update_title = role == "user"
        && agent
            .storage
            .get_conversation(conversation_id)
            .await
            .ok()
            .flatten()
            .map(|conversation| {
                conversation.message_count == 0 || conversation.title.trim() == "New Chat"
            })
            .unwrap_or(false);
    let message = crate::storage::entities::message::Model {
        id: uuid::Uuid::new_v4().to_string(),
        conversation_id: conversation_id.to_string(),
        role: role.to_string(),
        content: content.to_string(),
        timestamp: chrono::Utc::now().to_rfc3339(),
        model_used: model_used.map(str::to_string),
        trace_id: trace_id.map(str::to_string),
    };
    if let Err(error) = agent
        .encrypted_storage
        .insert_message_encrypted(&message)
        .await
    {
        tracing::warn!(
            "Failed to persist deep research {} message for conversation '{}': {}",
            role,
            conversation_id,
            error
        );
        return;
    }
    if should_update_title {
        let title = truncate_stream_task_text(content, 48);
        if !title.trim().is_empty() {
            let _ = agent
                .storage
                .update_conversation(conversation_id, Some(&title), None, None)
                .await;
        }
    }
}

fn deep_research_setup_issue(error_text: &str) -> bool {
    let normalized = error_text
        .trim()
        .to_ascii_lowercase()
        .split_whitespace()
        .collect::<Vec<_>>()
        .join(" ");
    normalized.contains(
        crate::actions::search::SEARCH_PROVIDER_SETUP_REQUIRED_MESSAGE
            .to_ascii_lowercase()
            .as_str(),
    ) || normalized.contains("no search backend")
        || normalized.contains("search backend") && normalized.contains("not configured")
        || normalized.contains("no usable sources")
        || normalized.contains("all search angles failed")
        || normalized.contains("available search backends")
        || normalized.contains("not enough evidence for this research depth")
        || normalized.contains("the search produced only")
}

fn deep_research_error_message(error: &anyhow::Error) -> String {
    let error_text = error.to_string();
    if deep_research_setup_issue(&error_text) {
        format!(
            "Deep research stopped because the available search backends failed or returned too little usable source evidence. AgentArk will not generate a cited report from unreliable or unreadable search results. Configure a reliable backend in Search Settings: SearXNG, Serper, Brave Search API, Exa, Tavily, Perplexity, or Firecrawl.\n\nDetails: {}",
            truncate_stream_task_text(&error_text, 900)
        )
    } else {
        format!(
            "Deep research stopped before it could produce a trustworthy cited report. I did not synthesize an answer without usable source evidence.\n\nDetails: {}",
            truncate_stream_task_text(&error_text, 900)
        )
    }
}

fn deep_research_synthesis_timeout_ms() -> u64 {
    std::env::var("AGENTARK_DEEP_RESEARCH_SYNTHESIS_TIMEOUT_MS")
        .ok()
        .and_then(|value| value.trim().parse::<u64>().ok())
        .filter(|value| *value >= 30_000)
        .unwrap_or(300_000)
        .min(900_000)
}

fn strip_leading_markdown_title(markdown: &str) -> String {
    let trimmed = markdown.trim();
    let Some(first_line_end) = trimmed.find('\n') else {
        return if trimmed.starts_with("# ") {
            String::new()
        } else {
            trimmed.to_string()
        };
    };
    let first_line = trimmed[..first_line_end].trim();
    if !first_line.starts_with("# ") {
        return trimmed.to_string();
    }
    trimmed[first_line_end..].trim_start().to_string()
}

fn compose_deep_research_report(query: &str, synthesis: &str, evidence_report: &str) -> String {
    let synthesis = strip_leading_markdown_title(synthesis);
    let evidence = strip_leading_markdown_title(evidence_report);
    let title = truncate_stream_task_text(&deep_research_topic_label(query), 180);
    let mut output = format!("# Deep Research: {}\n\n", title);
    output.push_str(synthesis.trim());
    output.push_str("\n\n---\n\n## Evidence Brief Used For Synthesis\n\n");
    output.push_str(evidence.trim());
    output.push('\n');
    output
}

async fn synthesize_deep_research_report(
    agent: &Agent,
    query: &str,
    evidence_report: &str,
    progress: &crate::actions::research::ResearchProgressReporter,
) -> anyhow::Result<String> {
    progress.emit(
        "synthesis",
        "Analyst synthesis",
        "Asking the research model to write a source-grounded deep report from the gathered evidence.",
        "running",
        "phase-status:research:model-synthesis",
    );

    let system = r#"You are AgentArk's deep research synthesis engine.

Write like a senior research analyst. Use only the supplied evidence brief and its numbered sources. Do not invent facts, citations, dates, statistics, source titles, or URLs. If the evidence is thin, conflicting, or missing for part of the request, say so directly and explain what would need verification.

The report must be rigorous, detailed, and useful: synthesize implications, tradeoffs, uncertainties, counterarguments, and practical options. Format it like a clean analyst report: start with a concise executive summary, use numbered or clearly ordered sections, include compact Markdown tables when they improve comparison or planning, and keep paragraphs readable. Adapt headings to the user's underlying research intent instead of mirroring surface wording. Cite material claims with bracketed source numbers such as [1] or [2, 4]. Do not include a Sources section, chart fences, or raw JSON because AgentArk appends the evidence brief separately."#;

    let evidence = if evidence_report.chars().count() > 45_000 {
        let trimmed = evidence_report.chars().take(45_000).collect::<String>();
        format!(
            "{}\n\n[Evidence brief truncated to fit the synthesis context. Preserve uncertainty where support is incomplete.]",
            trimmed
        )
    } else {
        evidence_report.to_string()
    };
    let user = format!(
        "Original research request:\n{}\n\nEvidence brief:\n{}\n\nWrite the deep research report now. Include an executive summary, detailed analysis organized around the request's real dimensions, compact tables for comparisons or options when useful, the strongest case for and against the likely conclusion, practical recommendations or options, and evidence gaps/open questions. Keep every substantive claim grounded in the supplied numbered sources.",
        query.trim(),
        evidence.trim()
    );
    let llm = agent
        .llm_for_role(&crate::core::ModelRole::Research)
        .clone();
    let response = tokio::time::timeout(
        std::time::Duration::from_millis(deep_research_synthesis_timeout_ms()),
        llm.chat_with_system_bounded(system, &user, 8_000),
    )
    .await
    .map_err(|_| anyhow::anyhow!("research model synthesis timed out"))??;
    let synthesis = response.content.trim();
    if synthesis.is_empty() {
        return Err(anyhow::anyhow!(
            "research model synthesis returned no report content"
        ));
    }

    progress.emit(
        "synthesis",
        "Analyst synthesis",
        format!(
            "Research model completed a {} character synthesis using {}.",
            synthesis.chars().count(),
            response.model
        ),
        "completed",
        "phase-status:research:model-synthesis",
    );
    Ok(compose_deep_research_report(
        query,
        synthesis,
        evidence_report,
    ))
}

async fn prepare_deep_research_plan_confirmation(
    app_state: &AppState,
    tx: &tokio::sync::mpsc::Sender<std::result::Result<Event, std::convert::Infallible>>,
    live_runs: &std::sync::Arc<crate::core::LiveRunRegistry>,
    run_storage: &crate::storage::Storage,
    run_id: &str,
    channel: &str,
    message: &str,
    conversation_id: Option<&str>,
    _project_id: Option<&str>,
    attachments: &[crate::core::ChatAttachmentHint],
    user_message_already_recorded: bool,
) -> anyhow::Result<()> {
    let task_id = uuid::Uuid::new_v4();
    let agent_snapshot = Agent::snapshot(&app_state.agent).await;
    let plan =
        generate_deep_research_plan_preview(&agent_snapshot, message, &task_id.to_string()).await;
    let plan_json = serde_json::to_value(&plan).unwrap_or_else(|_| serde_json::json!({}));
    let description = format!(
        "Deep research: {}",
        truncate_stream_task_text(&deep_research_topic_label(message), 96)
    );
    let mut task = crate::core::Task::new(
        description.clone(),
        "chat_request".to_string(),
        serde_json::json!({
            "_task_kind": "chat_request",
            "_origin": "chat",
            "_work_type": "research",
            "_pause_kind": "plan_confirmation",
            "_plan_preview": {
                "original_plan": plan_json,
                "current_plan": plan_json,
                "source": "deep_research"
            },
            "message": message,
            "channel": channel,
            "conversation_id": conversation_id,
            "deep_research": true,
            "attachments_present": !attachments.is_empty(),
            "attachments": attachments,
        }),
    );
    task.id = task_id;
    task.status = crate::core::TaskStatus::Paused;
    task.capabilities = vec!["network".to_string(), "research".to_string()];

    if !user_message_already_recorded {
        persist_deep_research_chat_message(
            &agent_snapshot,
            conversation_id,
            "user",
            message,
            None,
            None,
        )
        .await;
    }
    agent_snapshot.add_task(task.clone()).await?;

    send_chat_stream_event(
        tx,
        live_runs,
        run_id,
        "chat",
        channel,
        "task_started",
        serde_json::json!({
            "task_id": task.id.to_string(),
            "description": task.description,
            "status": "paused",
            "work_type": "research",
            "conversation_id": conversation_id,
        }),
        crate::core::RunEventPriority::Critical,
    )
    .await;
    send_chat_stream_event(
        tx,
        live_runs,
        run_id,
        "chat",
        channel,
        "plan_generated",
        serde_json::json!({
            "step_type": "plan_generated",
            "task_id": task.id.to_string(),
            "source": "deep_research",
            "plan": plan,
            "conversation_id": conversation_id,
        }),
        crate::core::RunEventPriority::High,
    )
    .await;
    send_chat_stream_event(
        tx,
        live_runs,
        run_id,
        "chat",
        channel,
        "plan_ready_for_confirmation",
        serde_json::json!({
            "step_type": "plan_ready_for_confirmation",
            "task_id": task.id.to_string(),
            "source": "deep_research",
            "plan": plan,
            "conversation_id": conversation_id,
        }),
        crate::core::RunEventPriority::Critical,
    )
    .await;
    send_chat_stream_event(
        tx,
        live_runs,
        run_id,
        "chat",
        channel,
        "task_status",
        serde_json::json!({
            "task_id": task.id.to_string(),
            "description": description,
            "status": "paused",
            "work_type": "research",
            "result_preview": "Deep research plan is awaiting confirmation.",
            "conversation_id": conversation_id,
        }),
        crate::core::RunEventPriority::High,
    )
    .await;
    upsert_chat_stream_execution_run(
        run_storage,
        run_id,
        conversation_id,
        channel,
        message,
        crate::core::ExecutionRunStatus::NeedsInput,
        Some("Deep research plan is awaiting confirmation.".to_string()),
        None,
        None,
        Vec::new(),
        Vec::new(),
    )
    .await;
    send_chat_stream_event(
        tx,
        live_runs,
        run_id,
        "chat",
        channel,
        "run_status",
        serde_json::json!({
            "run_id": run_id,
            "run_status": "needs_input",
            "trace_id": serde_json::Value::Null,
            "summary": "Deep research plan is awaiting confirmation.",
            "conversation_id": conversation_id,
        }),
        crate::core::RunEventPriority::High,
    )
    .await;
    Ok(())
}

async fn run_approved_deep_research_stream(
    app_state: &AppState,
    tx: &tokio::sync::mpsc::Sender<std::result::Result<Event, std::convert::Infallible>>,
    live_runs: &std::sync::Arc<crate::core::LiveRunRegistry>,
    run_storage: &crate::storage::Storage,
    run_id: &str,
    channel: &str,
    message: &str,
    conversation_id: Option<&str>,
    _project_id: Option<&str>,
    task: &StreamedChatTask,
    started_at: std::time::Instant,
) -> anyhow::Result<()> {
    let task_uuid = uuid::Uuid::parse_str(&task.task_id)
        .map_err(|error| anyhow::anyhow!("Invalid deep research task id: {}", error))?;
    let plan = deep_research_plan_from_value(task.plan_override.as_ref(), message, &task.task_id);

    send_chat_stream_event(
        tx,
        live_runs,
        run_id,
        "chat",
        channel,
        "task_status",
        serde_json::json!({
            "task_id": task.task_id,
            "description": task.description,
            "status": "in_progress",
            "work_type": task.work_type,
            "conversation_id": conversation_id,
        }),
        crate::core::RunEventPriority::High,
    )
    .await;
    send_chat_stream_event(
        tx,
        live_runs,
        run_id,
        "chat",
        channel,
        "tool_start",
        serde_json::json!({
            "name": "research",
            "query": message,
            "depth": "deep",
            "plan_id": plan.plan_id,
            "plan_revision": plan.revision,
        }),
        crate::core::RunEventPriority::High,
    )
    .await;

    let agent_snapshot = Agent::snapshot(&app_state.agent).await;
    let search_config = crate::runtime::build_search_config(
        &agent_snapshot.config_dir,
        Some(&agent_snapshot.storage),
    )
    .await;
    drop(agent_snapshot);

    let (progress_tx, mut progress_rx) =
        tokio::sync::mpsc::unbounded_channel::<crate::actions::research::ResearchProgressUpdate>();
    let progress = crate::actions::research::ResearchProgressReporter::new(progress_tx);
    let progress_forwarder = {
        let tx = tx.clone();
        let live_runs = live_runs.clone();
        let run_id = run_id.to_string();
        let channel = channel.to_string();
        let plan = plan.clone();
        crate::spawn_logged!("src/channels/http.rs:deep_research_progress", async move {
            while let Some(update) = progress_rx.recv().await {
                let (step_id, step_title) = deep_research_step_for_phase(&plan, &update.phase);
                send_deep_research_plan_step_update(
                    &tx,
                    &live_runs,
                    &run_id,
                    &channel,
                    &plan,
                    step_id,
                    step_title.clone(),
                    &update.status,
                    update.detail.clone(),
                )
                .await;
                send_chat_stream_event(
                    &tx,
                    &live_runs,
                    &run_id,
                    "chat",
                    &channel,
                    "tool_progress",
                    serde_json::json!({
                        "name": "research",
                        "content": update.detail,
                        "kind": "phase_status",
                        "phase": update.phase,
                        "label": update.label,
                        "detail": update.detail,
                        "status": update.status,
                        "elapsed_secs": update.elapsed_secs,
                        "stream_key": update.stream_key,
                        "plan_id": plan.plan_id,
                        "plan_revision": plan.revision,
                        "plan_step_id": step_id,
                        "plan_step_title": step_title,
                    }),
                    crate::core::RunEventPriority::Normal,
                )
                .await;
            }
        })
    };

    let research_args = crate::actions::research::ResearchArgs {
        query: message.to_string(),
        max_sources: 12,
        _include_sources: true,
        backend: None,
        depth: crate::actions::research::ResearchDepth::Deep,
        min_primary_sources: 2,
        freshness_window_days: None,
        followup_rounds: 2,
    };
    let research_result = crate::actions::research::execute_research_with_progress(
        &research_args,
        &search_config,
        Some(&progress),
    )
    .await;
    let research_result = match research_result {
        Ok(evidence_output) if evidence_output.trim().is_empty() => Err(anyhow::anyhow!(
            "Deep research completed without report content. No answer was saved."
        )),
        Ok(evidence_output) => {
            let agent_snapshot = Agent::snapshot(&app_state.agent).await;
            synthesize_deep_research_report(&agent_snapshot, message, &evidence_output, &progress)
                .await
                .map_err(|error| {
                    anyhow::anyhow!(
                        "Deep research gathered source evidence, but failed during analyst synthesis: {}",
                        error
                    )
                })
        }
        Err(error) => Err(error),
    };
    drop(progress);
    let _ = progress_forwarder.await;

    match research_result {
        Ok(output) => {
            for step in &plan.steps {
                send_deep_research_plan_step_update(
                    tx,
                    live_runs,
                    run_id,
                    channel,
                    &plan,
                    step.id,
                    step.title.clone(),
                    "completed",
                    "Research step completed.",
                )
                .await;
            }

            let agent_snapshot = Agent::snapshot(&app_state.agent).await;
            persist_deep_research_chat_message(
                &agent_snapshot,
                conversation_id,
                "assistant",
                &output,
                Some("deep_research"),
                None,
            )
            .await;
            agent_snapshot
                .finalize_task(
                    task_uuid,
                    crate::core::TaskStatus::Completed,
                    Some(truncate_stream_task_text(&output, 400)),
                )
                .await?;
            let user_outcome =
                agent_snapshot
                    .execution_supervisor
                    .build_success_outcome(&output, &[], &[]);
            send_chat_stream_event(
                tx,
                live_runs,
                run_id,
                "chat",
                channel,
                "tool_result",
                serde_json::json!({
                    "name": "research",
                    "content": "Deep research completed with a cited report.",
                    "plan_id": plan.plan_id,
                    "plan_revision": plan.revision,
                }),
                crate::core::RunEventPriority::High,
            )
            .await;
            send_chat_stream_event(
                tx,
                live_runs,
                run_id,
                "chat",
                channel,
                "task_status",
                serde_json::json!({
                    "task_id": task.task_id,
                    "description": task.description,
                    "status": "completed",
                    "work_type": task.work_type,
                    "result_preview": truncate_stream_task_text(&output, 400),
                    "conversation_id": conversation_id,
                }),
                crate::core::RunEventPriority::High,
            )
            .await;
            let duration_ms = started_at.elapsed().as_millis().min(u64::MAX as u128) as u64;
            upsert_chat_stream_execution_run(
                run_storage,
                run_id,
                conversation_id,
                channel,
                message,
                crate::core::ExecutionRunStatus::Completed,
                Some(truncate_stream_task_text(&output, 1200)),
                None,
                None,
                Vec::new(),
                Vec::new(),
            )
            .await;
            send_chat_stream_event(
                tx,
                live_runs,
                run_id,
                "chat",
                channel,
                "content",
                serde_json::json!({
                    "content": output,
                    "conversation_id": conversation_id,
                    "run_id": run_id,
                    "run_status": "completed",
                    "trace_id": serde_json::Value::Null,
                    "input_tokens": 0,
                    "output_tokens": 0,
                    "total_tokens": 0,
                    "duration_ms": duration_ms,
                    "time_to_first_token_ms": duration_ms.max(1),
                    "degradation": [],
                    "attempted_models": [],
                    "user_outcome": user_outcome,
                }),
                crate::core::RunEventPriority::Critical,
            )
            .await;
            send_chat_stream_event(
                tx,
                live_runs,
                run_id,
                "chat",
                channel,
                "run_status",
                serde_json::json!({
                    "run_id": run_id,
                    "run_status": "completed",
                    "trace_id": serde_json::Value::Null,
                    "conversation_id": conversation_id,
                    "duration_ms": duration_ms,
                    "time_to_first_token_ms": duration_ms.max(1),
                    "degradation": [],
                    "attempted_models": [],
                    "user_outcome": user_outcome,
                }),
                crate::core::RunEventPriority::Critical,
            )
            .await;
            Ok(())
        }
        Err(error) => {
            let message_text = deep_research_error_message(&error);
            let error_text = error.to_string();
            let setup_issue = deep_research_setup_issue(&error_text);
            let failed_phase = if setup_issue {
                "searching"
            } else {
                "synthesis"
            };
            let (step_id, step_title) = deep_research_step_for_phase(&plan, failed_phase);
            send_deep_research_plan_step_update(
                tx,
                live_runs,
                run_id,
                channel,
                &plan,
                step_id,
                step_title,
                "failed",
                message_text.clone(),
            )
            .await;

            let degradation = vec![crate::core::DegradationNote {
                kind: if setup_issue {
                    "search_provider_setup".to_string()
                } else {
                    "deep_research_failed".to_string()
                },
                summary: if setup_issue {
                    "search backends returned no usable evidence".to_string()
                } else {
                    "deep research failed".to_string()
                },
                detail: Some(error_text.clone()),
            }];
            let agent_snapshot = Agent::snapshot(&app_state.agent).await;
            persist_deep_research_chat_message(
                &agent_snapshot,
                conversation_id,
                "assistant",
                &message_text,
                Some("deep_research"),
                None,
            )
            .await;
            agent_snapshot
                .finalize_task(
                    task_uuid,
                    crate::core::TaskStatus::Failed {
                        error: message_text.clone(),
                    },
                    Some(truncate_stream_task_text(&message_text, 400)),
                )
                .await?;
            let user_outcome = if setup_issue {
                agent_snapshot
                    .execution_supervisor
                    .build_integration_outcome(&message_text, &degradation, &[])
            } else {
                agent_snapshot
                    .execution_supervisor
                    .build_service_outage_outcome(
                        &message_text,
                        "deep_research_failed",
                        &degradation,
                        &[],
                    )
            };
            send_chat_stream_event(
                tx,
                live_runs,
                run_id,
                "chat",
                channel,
                "tool_progress",
                serde_json::json!({
                    "name": "research",
                    "content": message_text,
                    "kind": "phase_status",
                    "phase": failed_phase,
                    "label": if setup_issue { "Search backends failed" } else { "Research stopped" },
                    "detail": message_text,
                    "status": "failed",
                    "elapsed_secs": started_at.elapsed().as_secs(),
                    "stream_key": "phase-status:research:failed",
                    "plan_id": plan.plan_id,
                    "plan_revision": plan.revision,
                    "plan_step_id": step_id,
                }),
                crate::core::RunEventPriority::High,
            )
            .await;
            send_chat_stream_event(
                tx,
                live_runs,
                run_id,
                "chat",
                channel,
                "task_status",
                serde_json::json!({
                    "task_id": task.task_id,
                    "description": task.description,
                    "status": "failed",
                    "work_type": task.work_type,
                    "result_preview": truncate_stream_task_text(&message_text, 400),
                    "conversation_id": conversation_id,
                }),
                crate::core::RunEventPriority::High,
            )
            .await;
            let run_status = if setup_issue {
                crate::core::ExecutionRunStatus::NeedsInput
            } else {
                crate::core::ExecutionRunStatus::Degraded
            };
            upsert_chat_stream_execution_run(
                run_storage,
                run_id,
                conversation_id,
                channel,
                message,
                run_status.clone(),
                Some(message_text.clone()),
                Some(error_text),
                None,
                degradation.clone(),
                Vec::new(),
            )
            .await;
            send_chat_stream_event(
                tx,
                live_runs,
                run_id,
                "chat",
                channel,
                "content",
                serde_json::json!({
                    "content": message_text,
                    "conversation_id": conversation_id,
                    "run_id": run_id,
                    "run_status": run_status.as_str(),
                    "trace_id": serde_json::Value::Null,
                    "input_tokens": 0,
                    "output_tokens": 0,
                    "total_tokens": 0,
                    "duration_ms": started_at.elapsed().as_millis().min(u64::MAX as u128) as u64,
                    "time_to_first_token_ms": started_at.elapsed().as_millis().min(u64::MAX as u128).max(1) as u64,
                    "degradation": degradation,
                    "attempted_models": [],
                    "user_outcome": user_outcome,
                }),
                crate::core::RunEventPriority::Critical,
            )
            .await;
            send_chat_stream_event(
                tx,
                live_runs,
                run_id,
                "chat",
                channel,
                "run_status",
                serde_json::json!({
                    "run_id": run_id,
                    "run_status": run_status.as_str(),
                    "trace_id": serde_json::Value::Null,
                    "conversation_id": conversation_id,
                    "degradation": degradation,
                    "attempted_models": [],
                    "user_outcome": user_outcome,
                }),
                crate::core::RunEventPriority::Critical,
            )
            .await;
            Ok(())
        }
    }
}

#[derive(Clone)]
pub(super) struct StreamedChatTask {
    pub(super) task_id: String,
    pub(super) description: String,
    pub(super) work_type: String,
    pub(super) user_message_already_recorded: bool,
    pub(super) plan_override: Option<serde_json::Value>,
}

#[allow(clippy::large_enum_variant)]
#[derive(Clone)]
pub(super) enum ChatStreamTaskMode {
    CreateIfNeeded,
    Existing(Box<StreamedChatTask>),
}

#[derive(Clone)]
pub(super) struct AcceptedChatSuggestionRun {
    pub(super) suggestion: ChatAutomationSuggestion,
    pub(super) proposal_id: Option<String>,
    pub(super) before: suggestions::SuggestionRunSnapshot,
}

#[derive(Clone)]
pub(super) struct ChatStreamRunRequest {
    pub(super) message: String,
    pub(super) channel: String,
    pub(super) conversation_id: Option<String>,
    pub(super) user_message_already_recorded: bool,
    pub(super) recorded_user_message_id: Option<String>,
    pub(super) deep_research: bool,
    pub(super) plan_confirmation_mode: Option<String>,
    pub(super) attachments: Vec<crate::core::ChatAttachmentHint>,
    pub(super) caller_principal: Option<crate::actions::ActionCallerPrincipal>,
    pub(super) task_mode: ChatStreamTaskMode,
    /// Per-call ArkOrbit structural context. Threaded into request hints so
    /// the agent loop can read it without changing model selection.
    pub(super) arkorbit_context: Option<serde_json::Value>,
    pub(super) accepted_suggestion: Option<AcceptedChatSuggestionRun>,
}

pub(super) struct PersistedChatStreamUserMessage {
    pub(super) conversation_id: String,
    pub(super) message_id: String,
}

pub(super) fn chat_stream_run_request_from_persisted_user_message(
    mut request: ChatRequest,
    persisted_user_message: PersistedChatStreamUserMessage,
    caller_principal: Option<crate::actions::ActionCallerPrincipal>,
    accepted_suggestion: Option<AcceptedChatSuggestionRun>,
) -> ChatStreamRunRequest {
    request.conversation_id = Some(persisted_user_message.conversation_id);
    ChatStreamRunRequest {
        message: request.message,
        channel: request.channel,
        conversation_id: request.conversation_id,
        user_message_already_recorded: true,
        recorded_user_message_id: Some(persisted_user_message.message_id),
        deep_research: request.deep_research,
        plan_confirmation_mode: request.plan_confirmation_mode,
        attachments: request.attachments,
        caller_principal,
        task_mode: ChatStreamTaskMode::CreateIfNeeded,
        arkorbit_context: request.arkorbit_context,
        accepted_suggestion,
    }
}

pub(super) async fn persist_chat_stream_user_message_before_run(
    state: &AppState,
    channel: &str,
    conversation_id: Option<&str>,
    message: &str,
) -> std::result::Result<PersistedChatStreamUserMessage, Response> {
    let agent = Agent::snapshot(&state.agent).await;
    let safe_message = crate::security::redact_secret_input(message).text;
    let conversation_id = match agent
        .ensure_conversation_id_for_request(channel, conversation_id, None, &safe_message)
        .await
    {
        Ok(value) => value,
        Err(error) if error.to_string() == "Conversation not found" => {
            return Err(conversation_not_found_response());
        }
        Err(error) => {
            return Err((
                StatusCode::INTERNAL_SERVER_ERROR,
                Json(ErrorResponse {
                    error: format!("Failed to prepare conversation: {}", error),
                }),
            )
                .into_response());
        }
    };

    let now = chrono::Utc::now().to_rfc3339();
    let conversation = crate::storage::entities::conversation::Model {
        id: conversation_id.clone(),
        title: truncate_stream_task_text(&safe_message, 80),
        channel: channel.to_string(),
        project_id: None,
        created_at: now.clone(),
        updated_at: now.clone(),
        message_count: 0,
        archived: false,
        starred: false,
    };
    if let Err(error) = agent
        .storage
        .create_conversation_if_absent(&conversation)
        .await
    {
        return Err((
            StatusCode::INTERNAL_SERVER_ERROR,
            Json(ErrorResponse {
                error: format!("Failed to prepare conversation: {}", error),
            }),
        )
            .into_response());
    }

    let message_id = uuid::Uuid::new_v4().to_string();
    let user_message = crate::storage::entities::message::Model {
        id: message_id.clone(),
        conversation_id: conversation_id.clone(),
        role: "user".to_string(),
        content: safe_message,
        timestamp: now,
        model_used: None,
        trace_id: None,
    };
    if let Err(error) = agent
        .encrypted_storage
        .insert_message_encrypted_if_absent(&user_message)
        .await
    {
        return Err((
            StatusCode::INTERNAL_SERVER_ERROR,
            Json(ErrorResponse {
                error: format!("Failed to save chat message: {}", error),
            }),
        )
            .into_response());
    }

    Ok(PersistedChatStreamUserMessage {
        conversation_id,
        message_id,
    })
}

fn accepted_chat_suggestion_context_value(
    suggestion: &ChatAutomationSuggestion,
) -> serde_json::Value {
    serde_json::json!({
        "suggestion_id": suggestion.id,
        "accepted_kind": suggestion.kind,
        "title": suggestion.title,
        "detail": suggestion.detail,
        "goal_title": suggestion.goal_title,
        "goal_detail": suggestion.goal_detail,
        "conversation_id": suggestion.conversation_id,
        "source_message_id": suggestion.source_message_id,
    })
}

async fn prepare_accepted_chat_suggestion_launch(
    state: &AppState,
    suggestion_id: Option<&str>,
    proposal_id: Option<&str>,
) -> std::result::Result<Option<AcceptedChatSuggestionRun>, Response> {
    let Some(suggestion_id) = suggestion_id
        .map(str::trim)
        .filter(|value| !value.is_empty())
    else {
        return Ok(None);
    };
    let storage = { state.agent.read().await.storage.clone() };
    let mut suggestions = load_chat_suggestions(&storage).await;
    let Some(idx) = suggestions
        .iter()
        .position(|suggestion| suggestion.id == suggestion_id)
    else {
        return Err((
            StatusCode::NOT_FOUND,
            Json(ErrorResponse {
                error: "Suggestion not found".to_string(),
            }),
        )
            .into_response());
    };
    if suggestions[idx].status != "open" {
        return Err((
            StatusCode::BAD_REQUEST,
            Json(ErrorResponse {
                error: "Suggestion is no longer open".to_string(),
            }),
        )
            .into_response());
    }

    let before = suggestions::capture_run_snapshot(state).await;
    let started_at = chrono::Utc::now().to_rfc3339();
    let suggestion = suggestions[idx].clone();
    suggestions[idx].status = "accepted".to_string();
    suggestions[idx].updated_at = started_at.clone();
    suggestions[idx].accepted_at = Some(started_at.clone());
    suggestions[idx].run_status = Some("running".to_string());
    suggestions[idx].last_run_started_at = Some(started_at);
    suggestions[idx].last_run_completed_at = None;
    suggestions[idx].last_run_error = None;
    suggestions[idx].accepted_trace_id = None;
    suggestions[idx].accepted_goal_id = None;
    suggestions[idx].accepted_outcomes.clear();
    suggestions = prune_chat_suggestion_history(suggestions);
    save_chat_suggestions(&storage, &suggestions).await;

    sentinel_panel::update_chat_suggestion_proposal_run_state(
        &storage,
        proposal_id,
        &suggestion.id,
        "running",
        "running",
        None,
        Some("Chat launch started."),
    )
    .await;

    Ok(Some(AcceptedChatSuggestionRun {
        suggestion,
        proposal_id: proposal_id.map(str::to_string),
        before,
    }))
}

async fn send_chat_stream_event(
    tx: &tokio::sync::mpsc::Sender<std::result::Result<Event, std::convert::Infallible>>,
    live_runs: &std::sync::Arc<crate::core::LiveRunRegistry>,
    run_id: &str,
    flow_kind: &str,
    origin: &str,
    event_name: &str,
    payload: serde_json::Value,
    priority: crate::core::RunEventPriority,
) {
    let event = live_runs
        .publish_event(
            run_id,
            flow_kind,
            origin,
            event_name,
            priority,
            None,
            payload.clone(),
        )
        .await
        .map(run_event_to_sse_event)
        .unwrap_or_else(|| {
            Event::default()
                .event(event_name.to_string())
                .data(serde_json::to_string(&payload).unwrap_or_default())
        });
    let _ = tx.send(Ok(event)).await;
}

fn chat_stream_text_chunks(text: &str) -> Vec<String> {
    let mut chunks = Vec::new();
    let mut current = String::new();
    let mut current_chars = 0usize;

    for ch in text.chars() {
        current.push(ch);
        current_chars += 1;
        if current_chars >= 72 || (current_chars >= 28 && ch.is_whitespace()) {
            chunks.push(std::mem::take(&mut current));
            current_chars = 0;
        }
    }

    if !current.is_empty() {
        chunks.push(current);
    }

    chunks
}

async fn send_chat_stream_synthetic_tokens(
    tx: &tokio::sync::mpsc::Sender<std::result::Result<Event, std::convert::Infallible>>,
    live_runs: &std::sync::Arc<crate::core::LiveRunRegistry>,
    run_id: &str,
    origin: &str,
    response: &str,
) {
    let chunks = chat_stream_text_chunks(response);
    for (idx, chunk) in chunks.into_iter().enumerate() {
        send_chat_stream_event(
            tx,
            live_runs,
            run_id,
            "chat",
            origin,
            "token",
            serde_json::json!({ "content": chunk }),
            crate::core::RunEventPriority::Normal,
        )
        .await;
        if idx < 180 {
            tokio::time::sleep(std::time::Duration::from_millis(8)).await;
        }
    }
}

fn chat_stream_execution_status(raw: Option<&str>) -> crate::core::ExecutionRunStatus {
    match raw.unwrap_or("").trim().to_ascii_lowercase().as_str() {
        "completed" => crate::core::ExecutionRunStatus::Completed,
        "completed_degraded" | "degraded" => crate::core::ExecutionRunStatus::Degraded,
        "needs_input" => crate::core::ExecutionRunStatus::NeedsInput,
        "needs_stronger_model" => crate::core::ExecutionRunStatus::NeedsStrongerModel,
        "blocked" => crate::core::ExecutionRunStatus::Blocked,
        "cancelled" | "canceled" => crate::core::ExecutionRunStatus::Cancelled,
        "platform_failed" | "failed" | "error" => crate::core::ExecutionRunStatus::PlatformFailed,
        _ => crate::core::ExecutionRunStatus::Completed,
    }
}

async fn wait_for_chat_stream_trace_link(
    storage: &crate::storage::Storage,
    trace_id: Option<String>,
) -> Option<String> {
    let trace_id = trace_id
        .map(|value| value.trim().to_string())
        .filter(|value| !value.is_empty())?;
    for attempt in 0..CHAT_STREAM_TRACE_LINK_WAIT_ATTEMPTS {
        match storage.get_execution_trace(&trace_id).await {
            Ok(Some(_)) => return Some(trace_id),
            Ok(None) => {}
            Err(error) => {
                tracing::warn!(
                    trace_id,
                    "Failed to check execution trace before linking chat stream run: {}",
                    error
                );
                return None;
            }
        }
        if attempt + 1 < CHAT_STREAM_TRACE_LINK_WAIT_ATTEMPTS {
            tokio::time::sleep(Duration::from_millis(CHAT_STREAM_TRACE_LINK_WAIT_MS)).await;
        }
    }
    tracing::debug!(
        trace_id,
        "Execution trace was not persisted before chat stream run linkage window elapsed"
    );
    None
}

async fn upsert_chat_stream_execution_run(
    storage: &crate::storage::Storage,
    run_id: &str,
    conversation_id: Option<&str>,
    channel: &str,
    message: &str,
    status: crate::core::ExecutionRunStatus,
    result_summary: Option<String>,
    last_error: Option<String>,
    trace_id: Option<String>,
    degradation: Vec<crate::core::DegradationNote>,
    attempted_models: Vec<crate::core::ModelAttemptRecord>,
) {
    let now = chrono::Utc::now().to_rfc3339();
    let normalized_conversation_id = conversation_id
        .map(str::trim)
        .filter(|value| !value.is_empty())
        .map(str::to_string);
    let trace_id = wait_for_chat_stream_trace_link(storage, trace_id).await;
    if let Some(conversation_id) = normalized_conversation_id.as_deref() {
        let conversation = crate::storage::entities::conversation::Model {
            id: conversation_id.to_string(),
            title: truncate_stream_task_text(message, 80),
            channel: channel.to_string(),
            project_id: None,
            created_at: now.clone(),
            updated_at: now.clone(),
            message_count: 0,
            archived: false,
            starred: false,
        };
        if let Err(error) = storage.create_conversation_if_absent(&conversation).await {
            tracing::warn!(
                "Failed to ensure chat stream conversation '{}' before run persistence: {}",
                conversation_id,
                error
            );
        }
    }
    let created_at = if status == crate::core::ExecutionRunStatus::Accepted {
        now.clone()
    } else {
        storage
            .load_execution_run(run_id)
            .await
            .ok()
            .flatten()
            .map(|run| run.created_at)
            .unwrap_or_else(|| now.clone())
    };
    let run = crate::core::ExecutionRun {
        id: run_id.to_string(),
        kind: "chat".to_string(),
        request_id: Some(run_id.to_string()),
        status: status.clone(),
        current_stage: status.as_str().to_string(),
        lease_owner: None,
        lease_expires_at: None,
        attempt: 0,
        deadline_at: None,
        cancellation_requested: false,
        degradation,
        last_error,
        result_summary,
        trace_id,
        conversation_id: normalized_conversation_id,
        channel: Some(channel.to_string()),
        request_message: Some(truncate_stream_task_text(message, 2000)),
        attempted_models,
        created_at,
        updated_at: now,
    };
    if let Err(error) = storage.insert_execution_run(&run).await {
        tracing::warn!("Failed to persist chat stream run '{}': {}", run_id, error);
    }
}

pub(super) fn spawn_chat_stream_response(
    state: AppState,
    request: ChatStreamRunRequest,
) -> Response {
    let (tx, rx) =
        tokio::sync::mpsc::channel::<std::result::Result<Event, std::convert::Infallible>>(64);
    // Per-request trace so concurrent requests cannot clobber each other.
    let trace_ref = Arc::new(RwLock::new(ExecutionTrace::default()));
    let agent_ref = state.agent.clone();
    let message = request.message.clone();
    let channel = request.channel.clone();
    let conversation_id = request.conversation_id.clone();
    let request_user_message_already_recorded = request.user_message_already_recorded;
    let request_recorded_user_message_id = request.recorded_user_message_id.clone();
    let project_id: Option<String> = None;
    let deep_research = request.deep_research;
    let plan_confirmation_mode = request.plan_confirmation_mode.clone();
    let attachments = request.attachments.clone();
    let caller_principal = request.caller_principal.clone();
    let arkorbit_context = request.arkorbit_context.clone();
    let accepted_suggestion = request.accepted_suggestion.clone();
    let accepted_suggestion_context = accepted_suggestion
        .as_ref()
        .map(|accepted| accepted_chat_suggestion_context_value(&accepted.suggestion));
    let task_mode = request.task_mode.clone();
    let app_state = state.clone();
    let stream_request_id = uuid::Uuid::new_v4().to_string();
    let stream_started_at = Instant::now();
    let time_to_first_token_ms = Arc::new(AtomicU64::new(0));
    let stream_last_activity_ms = Arc::new(AtomicU64::new(0));

    if let Err(error) = spawn_chat_worker_logged("http_chat_stream_turn", async move {
        let (live_runs, run_storage) = {
            let agent = app_state.agent.read().await;
            (agent.live_run_registry(), agent.storage.clone())
        };
        upsert_chat_stream_execution_run(
            &run_storage,
            &stream_request_id,
            conversation_id.as_deref(),
            &channel,
            &message,
            crate::core::ExecutionRunStatus::Accepted,
            None,
            None,
            None,
            Vec::new(),
            Vec::new(),
        )
        .await;
        let task_mode_create_if_needed = matches!(&task_mode, ChatStreamTaskMode::CreateIfNeeded);
        let tracked_task = match task_mode {
            ChatStreamTaskMode::CreateIfNeeded => None,
            ChatStreamTaskMode::Existing(task) => {
                let task_started = crate::core::StreamEvent::ChatTaskStarted {
                    task_id: task.task_id.clone(),
                    description: task.description.clone(),
                    work_type: task.work_type.clone(),
                    conversation_id: conversation_id.clone(),
                };
                let (maybe_event, _) = normalize_stream_event_for_sse(task_started, "");
                if let Some((event_name, payload)) = maybe_event {
                    send_chat_stream_event(
                        &tx,
                        &live_runs,
                        &stream_request_id,
                        "chat",
                        &channel,
                        event_name,
                        payload,
                        crate::core::RunEventPriority::High,
                    )
                    .await;
                }
                Some(*task)
            }
        };
        let tracked_task_ref = Arc::new(RwLock::new(tracked_task));
        let (cancel_tx, mut cancel_rx) = tokio::sync::watch::channel(false);
        let registered_conversation_id = conversation_id
            .as_deref()
            .map(str::trim)
            .filter(|value| !value.is_empty())
            .unwrap_or(stream_request_id.as_str())
            .to_string();
        if let Some(previous) = replace_chat_conversation_cancellation_sender(
            &app_state,
            &registered_conversation_id,
            &stream_request_id,
            cancel_tx.clone(),
        )
        .await
        {
            let _ = previous.send(true);
            tracing::info!(
                "Cancelled prior active chat stream for conversation={} due to replacement request",
                registered_conversation_id
            );
        }
        let run_started = crate::core::StreamEvent::RunStarted {
            run_id: stream_request_id.clone(),
            flow_kind: "chat".to_string(),
            origin: channel.clone(),
            conversation_id: conversation_id.clone(),
            trace_id: None,
            resumed: tracked_task_ref.read().await.is_some(),
        };
        let (maybe_event, _) = normalize_stream_event_for_sse(run_started, "");
        if let Some((event_name, payload)) = maybe_event {
            send_chat_stream_event(
                &tx,
                &live_runs,
                &stream_request_id,
                "chat",
                &channel,
                event_name,
                payload,
                crate::core::RunEventPriority::Critical,
            )
            .await;
        }
        if let Some(task) = tracked_task_ref.read().await.as_ref() {
            bind_chat_task_cancellation_sender(&app_state, &task.task_id, cancel_tx.clone()).await;
        }

        // Stream model tokens + tool progress as dedicated SSE events.
        let (stream_tx, mut stream_rx) =
            tokio::sync::mpsc::channel::<crate::core::StreamEvent>(256);
        let stream_forwarder = {
            let tx = tx.clone();
            let tracked_task_ref = tracked_task_ref.clone();
            let app_state = app_state.clone();
            let cancel_tx = cancel_tx.clone();
            let trace_ref = trace_ref.clone();
            let time_to_first_token_ms = time_to_first_token_ms.clone();
            let stream_last_activity_ms = stream_last_activity_ms.clone();
            let live_runs = live_runs.clone();
            let stream_request_id = stream_request_id.clone();
            let channel = channel.clone();
            crate::spawn_logged!("src/channels/http.rs:14840", async move {
                let mut last_thinking_detail = String::new();
                while let Some(ev) = stream_rx.recv().await {
                    stream_last_activity_ms.store(
                        stream_started_at
                            .elapsed()
                            .as_millis()
                            .min(u128::from(u64::MAX)) as u64,
                        Ordering::Relaxed,
                    );
                    if let crate::core::StreamEvent::Token(content) = &ev {
                        if !content.is_empty() {
                            let elapsed_ms = stream_started_at
                                .elapsed()
                                .as_millis()
                                .min(u64::MAX as u128)
                                as u64;
                            let recorded_ms = elapsed_ms.max(1);
                            if time_to_first_token_ms
                                .compare_exchange(
                                    0,
                                    recorded_ms,
                                    Ordering::Relaxed,
                                    Ordering::Relaxed,
                                )
                                .is_ok()
                            {
                                let mut trace = trace_ref.write().await;
                                trace.steps.push(crate::core::ExecutionStep {
                                    icon: "[model]".to_string(),
                                    title: "First Token".to_string(),
                                    detail: format!(
                                        "Model began streaming after {}ms.",
                                        recorded_ms
                                    ),
                                    step_type: "info".to_string(),
                                    data: Some(
                                        serde_json::json!({
                                            "metric": "time_to_first_token",
                                            "duration_ms": recorded_ms
                                        })
                                        .to_string(),
                                    ),
                                    timestamp: chrono::Utc::now(),
                                    duration_ms: Some(recorded_ms),
                                });
                            }
                        }
                    }
                    if let crate::core::StreamEvent::ChatTaskStarted {
                        task_id,
                        description,
                        work_type,
                        ..
                    } = &ev
                    {
                        {
                            let mut tracked = tracked_task_ref.write().await;
                            *tracked = Some(StreamedChatTask {
                                task_id: task_id.clone(),
                                description: description.clone(),
                                work_type: work_type.clone(),
                                user_message_already_recorded:
                                    request_user_message_already_recorded,
                                plan_override: None,
                            });
                        }
                        bind_chat_task_cancellation_sender(&app_state, task_id, cancel_tx.clone())
                            .await;
                    }
                    let (maybe_event, next_thinking_detail) =
                        normalize_stream_event_for_sse(ev, &last_thinking_detail);
                    last_thinking_detail = next_thinking_detail;
                    let Some((event_name, payload)) = maybe_event else {
                        continue;
                    };
                    let priority = if matches!(event_name, "token" | "thinking") {
                        crate::core::RunEventPriority::Normal
                    } else {
                        crate::core::RunEventPriority::High
                    };
                    send_chat_stream_event(
                        &tx,
                        &live_runs,
                        &stream_request_id,
                        "chat",
                        &channel,
                        event_name,
                        payload,
                        priority,
                    )
                    .await;
                }
            })
        };

        // Poll trace for new steps and emit as SSE events.
        let trace_poller = {
            let tx = tx.clone();
            let trace_ref = trace_ref.clone();
            let live_runs = live_runs.clone();
            let stream_request_id = stream_request_id.clone();
            let channel = channel.clone();
            let stream_last_activity_ms = stream_last_activity_ms.clone();
            crate::spawn_logged!("src/channels/http.rs:14884", async move {
                let mut last_step_count = 0;
                loop {
                    tokio::time::sleep(std::time::Duration::from_millis(500)).await;
                    let trace = trace_ref.read().await;
                    let current_count = trace.steps.len();
                    if current_count > last_step_count {
                        for step in &trace.steps[last_step_count..current_count] {
                            let event_data = serde_json::json!({
                                "icon": step.icon,
                                "title": step.title,
                                "detail": step.detail,
                                "step_type": step.step_type,
                                "data": step.data,
                                "time": step.timestamp.to_rfc3339_opts(chrono::SecondsFormat::Secs, true),
                                "duration_ms": step.duration_ms,
                            });
                            send_chat_stream_event(
                                &tx,
                                &live_runs,
                                &stream_request_id,
                                "chat",
                                &channel,
                                "thinking",
                                event_data,
                                crate::core::RunEventPriority::Normal,
                            )
                            .await;
                            stream_last_activity_ms.store(
                                stream_started_at
                                    .elapsed()
                                    .as_millis()
                                    .min(u128::from(u64::MAX))
                                    as u64,
                                Ordering::Relaxed,
                            );
                        }
                        last_step_count = current_count;
                    }
                    if trace.completed_at.is_some() {
                        break;
                    }
                }
            })
        };

        send_chat_stream_event(
            &tx,
            &live_runs,
            &stream_request_id,
            "chat",
            &channel,
            "thinking",
            serde_json::json!({
                "icon": "[recv]",
                "title": "Request received",
                "detail": "Preparing model call and tool plan...",
                "step_type": "thinking",
                "data": null,
                "conversation_id": conversation_id.clone(),
            }),
            crate::core::RunEventPriority::High,
        )
        .await;

        if let Some(accepted) = accepted_suggestion.clone() {
            if accepted
                .suggestion
                .kind
                .trim()
                .eq_ignore_ascii_case("watcher")
            {
                {
                    let mut trace = trace_ref.write().await;
                    trace.id = stream_request_id.clone();
                    trace.message = message.clone();
                    trace.channel = channel.clone();
                    trace.started_at = Some(chrono::Utc::now());
                }
                let agent_snapshot = Agent::snapshot(&agent_ref).await;
                if !request_user_message_already_recorded {
                    persist_deep_research_chat_message(
                        &agent_snapshot,
                        conversation_id.as_deref(),
                        "user",
                        &message,
                        None,
                        Some(&stream_request_id),
                    )
                    .await;
                }
                let run_result = automation_control::execute_accepted_watcher_suggestion(
                    &agent_snapshot,
                    &accepted.suggestion,
                    &trace_ref,
                )
                .await;
                let _ =
                    trace::persist_live_trace_snapshot(&app_state.trace_history, &trace_ref).await;
                let outcomes = suggestions::collect_run_outcomes(
                    &app_state,
                    &accepted.before,
                    &accepted.suggestion.kind,
                )
                .await;
                let completed_at = chrono::Utc::now().to_rfc3339();
                match run_result {
                    Ok(_) => {
                        let summary =
                            trace_ref.read().await.response.clone().unwrap_or_else(|| {
                                "Sentinel watcher launched from Chat.".to_string()
                            });
                        suggestions::update_chat_suggestion_after_run(
                            &run_storage,
                            &accepted.suggestion.id,
                            &stream_request_id,
                            "completed",
                            &completed_at,
                            None,
                            outcomes,
                        )
                        .await;
                        sentinel_panel::update_chat_suggestion_proposal_run_state(
                            &run_storage,
                            accepted.proposal_id.as_deref(),
                            &accepted.suggestion.id,
                            "completed",
                            "completed",
                            Some(&stream_request_id),
                            Some(&summary),
                        )
                        .await;
                        persist_deep_research_chat_message(
                            &agent_snapshot,
                            conversation_id.as_deref(),
                            "assistant",
                            &summary,
                            Some("internal:accepted-suggestion"),
                            Some(&stream_request_id),
                        )
                        .await;
                        send_chat_stream_synthetic_tokens(
                            &tx,
                            &live_runs,
                            &stream_request_id,
                            &channel,
                            &summary,
                        )
                        .await;
                        upsert_chat_stream_execution_run(
                            &run_storage,
                            &stream_request_id,
                            conversation_id.as_deref(),
                            &channel,
                            &message,
                            crate::core::ExecutionRunStatus::Completed,
                            Some(summary.clone()),
                            None,
                            Some(stream_request_id.clone()),
                            Vec::new(),
                            Vec::new(),
                        )
                        .await;
                        let content = serde_json::json!({
                            "content": summary,
                            "conversation_id": conversation_id.clone(),
                            "run_id": stream_request_id.clone(),
                            "run_status": "completed",
                            "trace_id": stream_request_id.clone(),
                            "input_tokens": 0,
                            "output_tokens": 0,
                            "total_tokens": 0,
                            "duration_ms": stream_started_at.elapsed().as_millis().min(u64::MAX as u128) as u64,
                            "attempted_models": [],
                            "degradation": [],
                            "user_outcome": serde_json::Value::Null,
                        });
                        send_chat_stream_event(
                            &tx,
                            &live_runs,
                            &stream_request_id,
                            "chat",
                            &channel,
                            "content",
                            content.clone(),
                            crate::core::RunEventPriority::Critical,
                        )
                        .await;
                        send_chat_stream_event(
                            &tx,
                            &live_runs,
                            &stream_request_id,
                            "chat",
                            &channel,
                            "run_status",
                            serde_json::json!({
                                "run_id": content["run_id"],
                                "run_status": content["run_status"],
                                "trace_id": content["trace_id"],
                                "input_tokens": content["input_tokens"],
                                "output_tokens": content["output_tokens"],
                                "total_tokens": content["total_tokens"],
                                "duration_ms": content["duration_ms"],
                                "degradation": content["degradation"],
                                "attempted_models": content["attempted_models"],
                                "user_outcome": content["user_outcome"],
                            }),
                            crate::core::RunEventPriority::Critical,
                        )
                        .await;
                    }
                    Err(error) => {
                        let error_text = error.to_string();
                        {
                            let mut trace = trace_ref.write().await;
                            trace.completed_at = Some(chrono::Utc::now());
                            trace.response = Some(error_text.clone());
                        }
                        let _ = trace::persist_live_trace_snapshot(
                            &app_state.trace_history,
                            &trace_ref,
                        )
                        .await;
                        suggestions::update_chat_suggestion_after_run(
                            &run_storage,
                            &accepted.suggestion.id,
                            &stream_request_id,
                            "failed",
                            &completed_at,
                            Some(error_text.clone()),
                            outcomes,
                        )
                        .await;
                        sentinel_panel::update_chat_suggestion_proposal_run_state(
                            &run_storage,
                            accepted.proposal_id.as_deref(),
                            &accepted.suggestion.id,
                            "failed",
                            "failed",
                            Some(&stream_request_id),
                            Some(&error_text),
                        )
                        .await;
                        upsert_chat_stream_execution_run(
                            &run_storage,
                            &stream_request_id,
                            conversation_id.as_deref(),
                            &channel,
                            &message,
                            crate::core::ExecutionRunStatus::PlatformFailed,
                            Some("Accepted Sentinel watcher launch failed.".to_string()),
                            Some(error_text.clone()),
                            Some(stream_request_id.clone()),
                            Vec::new(),
                            Vec::new(),
                        )
                        .await;
                        send_chat_stream_event(
                            &tx,
                            &live_runs,
                            &stream_request_id,
                            "chat",
                            &channel,
                            "error",
                            serde_json::json!({ "error": error_text }),
                            crate::core::RunEventPriority::Critical,
                        )
                        .await;
                    }
                }
                unregister_chat_conversation_cancellation(
                    &app_state,
                    &registered_conversation_id,
                    &stream_request_id,
                )
                .await;
                send_chat_stream_event(
                    &tx,
                    &live_runs,
                    &stream_request_id,
                    "chat",
                    &channel,
                    "done",
                    serde_json::json!({}),
                    crate::core::RunEventPriority::Critical,
                )
                .await;
                drop(stream_tx);
                let _ = trace_poller.await;
                let _ = stream_forwarder.await;
                return;
            }
        }

        let deep_research_plan_first = deep_research
            && plan_confirmation_mode.as_deref() == Some("before_execution")
            && task_mode_create_if_needed;
        if deep_research_plan_first {
            let result = prepare_deep_research_plan_confirmation(
                &app_state,
                &tx,
                &live_runs,
                &run_storage,
                &stream_request_id,
                &channel,
                &message,
                conversation_id.as_deref(),
                project_id.as_deref(),
                &attachments,
                request_user_message_already_recorded,
            )
            .await;
            if let Err(error) = result {
                let error_text = error.to_string();
                send_chat_stream_event(
                    &tx,
                    &live_runs,
                    &stream_request_id,
                    "chat",
                    &channel,
                    "error",
                    serde_json::json!({ "error": error_text }),
                    crate::core::RunEventPriority::Critical,
                )
                .await;
                upsert_chat_stream_execution_run(
                    &run_storage,
                    &stream_request_id,
                    conversation_id.as_deref(),
                    &channel,
                    &message,
                    crate::core::ExecutionRunStatus::PlatformFailed,
                    Some("Deep research could not prepare a reviewable plan.".to_string()),
                    Some(error_text),
                    None,
                    Vec::new(),
                    Vec::new(),
                )
                .await;
            }
            send_chat_stream_event(
                &tx,
                &live_runs,
                &stream_request_id,
                "chat",
                &channel,
                "done",
                serde_json::json!({}),
                crate::core::RunEventPriority::Critical,
            )
            .await;
            {
                let mut trace = trace_ref.write().await;
                trace.completed_at = Some(chrono::Utc::now());
            }
            unregister_chat_conversation_cancellation(
                &app_state,
                &registered_conversation_id,
                &stream_request_id,
            )
            .await;
            drop(stream_tx);
            let _ = trace_poller.await;
            let _ = stream_forwarder.await;
            return;
        }

        if deep_research {
            let existing_task = tracked_task_ref.read().await.clone();
            if let Some(task) = existing_task.as_ref() {
                let result = run_approved_deep_research_stream(
                    &app_state,
                    &tx,
                    &live_runs,
                    &run_storage,
                    &stream_request_id,
                    &channel,
                    &message,
                    conversation_id.as_deref(),
                    project_id.as_deref(),
                    task,
                    stream_started_at,
                )
                .await;
                if let Err(error) = result {
                    let error_text = error.to_string();
                    send_chat_stream_event(
                        &tx,
                        &live_runs,
                        &stream_request_id,
                        "chat",
                        &channel,
                        "error",
                        serde_json::json!({ "error": error_text }),
                        crate::core::RunEventPriority::Critical,
                    )
                    .await;
                    upsert_chat_stream_execution_run(
                        &run_storage,
                        &stream_request_id,
                        conversation_id.as_deref(),
                        &channel,
                        &message,
                        crate::core::ExecutionRunStatus::PlatformFailed,
                        Some("Deep research hit a framework-level failure.".to_string()),
                        Some(error_text),
                        None,
                        Vec::new(),
                        Vec::new(),
                    )
                    .await;
                }
                unregister_chat_task_cancellation(&app_state, &task.task_id).await;
                unregister_chat_conversation_cancellation(
                    &app_state,
                    &registered_conversation_id,
                    &stream_request_id,
                )
                .await;
                send_chat_stream_event(
                    &tx,
                    &live_runs,
                    &stream_request_id,
                    "chat",
                    &channel,
                    "done",
                    serde_json::json!({}),
                    crate::core::RunEventPriority::Critical,
                )
                .await;
                {
                    let mut trace = trace_ref.write().await;
                    trace.completed_at = Some(chrono::Utc::now());
                }
                drop(stream_tx);
                let _ = trace_poller.await;
                let _ = stream_forwarder.await;
                return;
            }
        }

        let tracked_task_snapshot = tracked_task_ref.read().await.clone();
        let resume_existing_chat_task = tracked_task_snapshot.is_some();
        let user_message_already_recorded = tracked_task_snapshot
            .as_ref()
            .map(|task| task.user_message_already_recorded)
            .unwrap_or(request_user_message_already_recorded);
        let mut process_handle = {
            let agent_ref = agent_ref.clone();
            let message = message.clone();
            let channel = channel.clone();
            let conversation_id = conversation_id.clone();
            let project_id = project_id.clone();
            let trace_ref = trace_ref.clone();
            let attachments = attachments.clone();
            let caller_principal = caller_principal.clone();
            let arkorbit_context = arkorbit_context.clone();
            let accepted_suggestion_context = accepted_suggestion_context.clone();
            let recorded_user_message_id = request_recorded_user_message_id.clone();
            tokio::spawn(async move {
                let agent_snapshot = Agent::snapshot(&agent_ref).await;
                if resume_existing_chat_task {
                    agent_snapshot
                        .process_message_stream_resume_with_meta_and_hints(
                            &message,
                            &channel,
                            conversation_id.as_deref(),
                            project_id.as_deref(),
                            trace_ref,
                            stream_tx,
                            build_request_execution_hints_with_arkorbit(
                                caller_principal.as_ref(),
                                crate::actions::ActionExecutionSurface::Chat,
                                true,
                                attachments.clone(),
                                arkorbit_context.clone(),
                            ),
                        )
                        .await
                } else if user_message_already_recorded {
                    let mut hints = build_request_execution_hints_with_context(
                        caller_principal.as_ref(),
                        crate::actions::ActionExecutionSurface::Chat,
                        true,
                        attachments,
                        arkorbit_context,
                        accepted_suggestion_context,
                    );
                    hints.recorded_user_message_id = recorded_user_message_id;
                    agent_snapshot
                        .process_message_stream_prerecorded_with_meta_and_hints(
                            &message,
                            &channel,
                            conversation_id.as_deref(),
                            project_id.as_deref(),
                            trace_ref,
                            stream_tx,
                            hints,
                        )
                        .await
                } else {
                    agent_snapshot
                        .process_message_stream_with_meta_and_hints(
                            &message,
                            &channel,
                            conversation_id.as_deref(),
                            project_id.as_deref(),
                            trace_ref,
                            stream_tx,
                            build_request_execution_hints_with_context(
                                caller_principal.as_ref(),
                                crate::actions::ActionExecutionSurface::Chat,
                                true,
                                attachments,
                                arkorbit_context,
                                accepted_suggestion_context,
                            ),
                        )
                        .await
                }
            })
        };

        let mut was_cancelled = false;
        let idle_timeout_secs = chat_stream_idle_timeout_secs();
        let idle_watchdog = wait_for_chat_stream_idle_timeout(
            stream_last_activity_ms.clone(),
            stream_started_at,
            idle_timeout_secs,
        );
        tokio::pin!(idle_watchdog);
        let result = tokio::select! {
            worker = &mut process_handle => {
                match worker {
                    Ok(result) => result,
                    Err(error) if error.is_cancelled() => {
                        was_cancelled = true;
                        Err(anyhow::anyhow!("Chat run cancelled"))
                    }
                    Err(error) => Err(anyhow::anyhow!("Chat worker failed: {}", error)),
                }
            }
            _ = &mut idle_watchdog => {
                process_handle.abort();
                let _ = process_handle.await;
                Err(anyhow::anyhow!(
                    "Chat run made no progress for {} seconds",
                    idle_timeout_secs
                ))
            }
            changed = cancel_rx.changed() => {
                if changed.is_ok() && *cancel_rx.borrow() {
                    was_cancelled = true;
                    process_handle.abort();
                    let _ = process_handle.await;
                    Err(anyhow::anyhow!("Chat run cancelled"))
                } else {
                    match process_handle.await {
                        Ok(result) => result,
                        Err(error) if error.is_cancelled() => {
                            was_cancelled = true;
                            Err(anyhow::anyhow!("Chat run cancelled"))
                        }
                        Err(error) => Err(anyhow::anyhow!("Chat worker failed: {}", error)),
                    }
                }
            }
        };

        {
            let mut trace = trace_ref.write().await;
            if trace.completed_at.is_none() {
                trace.completed_at = Some(chrono::Utc::now());
            }
        }

        trace_poller.abort();
        let _ = trace_poller.await;
        let _ = stream_forwarder.await;

        let tracked_task = tracked_task_ref.read().await.clone();

        if let Some(task) = tracked_task.as_ref() {
            unregister_chat_task_cancellation(&app_state, &task.task_id).await;
        }
        unregister_chat_conversation_cancellation(
            &app_state,
            &registered_conversation_id,
            &stream_request_id,
        )
        .await;

        match result {
            Ok(processed) => {
                let resolved_conversation_id = processed
                    .conversation_id
                    .clone()
                    .or(conversation_id.clone());
                let updated_arguments_json = if let Some(task) = tracked_task.as_ref() {
                    let mut tasks = app_state.tasks.write().await;
                    task.task_id
                        .parse::<uuid::Uuid>()
                        .ok()
                        .and_then(|task_id| tasks.get_mut(task_id))
                        .and_then(|entry| {
                            backfill_chat_task_origin_metadata(
                                entry,
                                resolved_conversation_id.as_deref(),
                                project_id.as_deref(),
                            )
                        })
                } else {
                    None
                };
                if let (Some(task), Some(arguments_json)) =
                    (tracked_task.as_ref(), updated_arguments_json)
                {
                    let agent_snapshot = Agent::snapshot(&agent_ref).await;
                    if task.task_id.parse::<uuid::Uuid>().is_err() {
                        tracing::warn!(
                            "Failed to parse streamed chat task id '{}' during finalize",
                            task.task_id
                        );
                    }
                    if let Err(error) = agent_snapshot
                        .storage
                        .update_task(&task.task_id, None, Some(arguments_json), None, None)
                        .await
                    {
                        tracing::warn!(
                            "Failed to backfill streamed chat task '{}' conversation metadata: {}",
                            task.task_id,
                            error
                        );
                    }
                }
                if let Some(task) = tracked_task.as_ref() {
                    let terminal_status = if deep_research {
                        deep_research_terminal_status(
                            &processed.response,
                            processed.run_status.as_deref(),
                            processed.user_outcome.as_ref(),
                        )
                    } else {
                        chat_task_terminal_status(&processed.response)
                    };
                    let result_preview = truncate_stream_task_text(
                        if processed.response.trim().is_empty() {
                            "Task completed."
                        } else {
                            &processed.response
                        },
                        400,
                    );
                    {
                        let agent_snapshot = Agent::snapshot(&agent_ref).await;
                        match task.task_id.parse::<uuid::Uuid>() {
                            Ok(task_uuid) => {
                                if let Err(error) = agent_snapshot
                                    .finalize_task(
                                        task_uuid,
                                        terminal_status.clone(),
                                        Some(result_preview.clone()),
                                    )
                                    .await
                                {
                                    tracing::warn!(
                                        "Failed to finalize streamed chat task '{}': {}",
                                        task.task_id,
                                        error
                                    );
                                }
                            }
                            Err(_) => {
                                tracing::warn!(
                                    "Failed to parse streamed chat task id '{}' during success finalize",
                                    task.task_id
                                );
                            }
                        }
                    }
                    send_chat_stream_event(
                        &tx,
                        &live_runs,
                        &stream_request_id,
                        "chat",
                        &channel,
                        "task_status",
                        serde_json::json!({
                            "task_id": task.task_id.clone(),
                            "description": task.description.clone(),
                            "status": chat_task_status_key(&terminal_status),
                            "work_type": task.work_type.clone(),
                            "result_preview": result_preview,
                            "conversation_id": resolved_conversation_id.clone(),
                        }),
                        crate::core::RunEventPriority::High,
                    )
                    .await;
                }

                if time_to_first_token_ms.load(Ordering::Relaxed) == 0
                    && !processed.response.trim().is_empty()
                {
                    let synthetic_first_token_ms = stream_started_at
                        .elapsed()
                        .as_millis()
                        .min(u64::MAX as u128)
                        as u64;
                    time_to_first_token_ms
                        .store(synthetic_first_token_ms.max(1), Ordering::Relaxed);
                    send_chat_stream_synthetic_tokens(
                        &tx,
                        &live_runs,
                        &stream_request_id,
                        &channel,
                        &processed.response,
                    )
                    .await;
                }

                let trace_metric_snapshot = {
                    let trace = trace_ref.read().await;
                    let duration_ms = trace.started_at.and_then(|start| {
                        trace
                            .completed_at
                            .map(|end| (end - start).num_milliseconds().max(0))
                    });
                    serde_json::json!({
                        "input_tokens": trace.input_tokens,
                        "output_tokens": trace.output_tokens,
                        "total_tokens": trace.total_tokens,
                        "duration_ms": duration_ms,
                    })
                };
                let first_token_ms = time_to_first_token_ms.load(Ordering::Relaxed);
                let first_content_ms = if first_token_ms > 0 {
                    first_token_ms
                } else {
                    stream_started_at
                        .elapsed()
                        .as_millis()
                        .min(u64::MAX as u128) as u64
                }
                .max(1);
                let wall_duration_ms = stream_started_at
                    .elapsed()
                    .as_millis()
                    .min(u64::MAX as u128) as u64;
                let trace_total_tokens =
                    trace_metric_snapshot["total_tokens"].as_i64().unwrap_or(0);
                let trace_input_tokens =
                    trace_metric_snapshot["input_tokens"].as_i64().unwrap_or(0);
                let trace_output_tokens =
                    trace_metric_snapshot["output_tokens"].as_i64().unwrap_or(0);
                let effective_input_tokens = if trace_input_tokens > 0 {
                    trace_input_tokens
                } else {
                    processed.input_tokens
                };
                let effective_output_tokens = if trace_output_tokens > 0 {
                    trace_output_tokens
                } else {
                    processed.output_tokens
                };
                let effective_total_tokens = if trace_total_tokens > 0 {
                    trace_total_tokens
                } else if processed.total_tokens > 0 {
                    processed.total_tokens
                } else {
                    effective_input_tokens.saturating_add(effective_output_tokens)
                };
                let effective_run_id = stream_request_id.clone();
                let persist_run_storage = run_storage.clone();
                let persist_run_id = effective_run_id.clone();
                let persist_conversation_id = resolved_conversation_id.clone();
                let persist_channel = channel.clone();
                let persist_message = message.clone();
                let persist_status = chat_stream_execution_status(processed.run_status.as_deref());
                let persist_summary = Some(truncate_stream_task_text(&processed.response, 1200));
                let persist_trace_id = processed.trace_id.clone();
                let persist_degradation = processed.degradation.clone();
                let persist_attempted_models = processed.attempted_models.clone();
                let mut content = serde_json::json!({
                    "content": processed.response,
                    "conversation_id": resolved_conversation_id,
                    "run_id": effective_run_id,
                    "run_status": processed.run_status,
                    "trace_id": processed.trace_id,
                    "input_tokens": effective_input_tokens,
                    "output_tokens": effective_output_tokens,
                    "total_tokens": effective_total_tokens,
                    "duration_ms": wall_duration_ms,
                    "trace_duration_ms": trace_metric_snapshot["duration_ms"],
                    "time_to_first_token_ms": first_content_ms,
                    "degradation": processed.degradation,
                    "attempted_models": processed.attempted_models,
                    "user_outcome": processed.user_outcome,
                });
                if let Some(title) = processed.conversation_title {
                    content["conversation_title"] = serde_json::json!(title);
                }
                if !processed.choices.is_empty() {
                    content["choices"] =
                        serde_json::to_value(&processed.choices).unwrap_or(serde_json::Value::Null);
                }
                send_chat_stream_event(
                    &tx,
                    &live_runs,
                    &stream_request_id,
                    "chat",
                    &channel,
                    "content",
                    content.clone(),
                    crate::core::RunEventPriority::Critical,
                )
                .await;
                send_chat_stream_event(
                    &tx,
                    &live_runs,
                    &stream_request_id,
                    "chat",
                    &channel,
                    "run_status",
                    serde_json::json!({
                        "run_id": content["run_id"],
                        "run_status": content["run_status"],
                        "trace_id": content["trace_id"],
                        "input_tokens": content["input_tokens"],
                        "output_tokens": content["output_tokens"],
                        "total_tokens": content["total_tokens"],
                        "duration_ms": content["duration_ms"],
                        "trace_duration_ms": content["trace_duration_ms"],
                        "time_to_first_token_ms": content["time_to_first_token_ms"],
                        "degradation": content["degradation"],
                        "attempted_models": content["attempted_models"],
                        "user_outcome": content["user_outcome"],
                    }),
                    crate::core::RunEventPriority::Critical,
                )
                .await;
                if let Some(accepted) = accepted_suggestion.as_ref() {
                    let suggestion_trace_id = persist_trace_id
                        .clone()
                        .unwrap_or_else(|| effective_run_id.clone());
                    let outcomes = suggestions::collect_run_outcomes(
                        &app_state,
                        &accepted.before,
                        &accepted.suggestion.kind,
                    )
                    .await;
                    let completed_at = chrono::Utc::now().to_rfc3339();
                    suggestions::update_chat_suggestion_after_run(
                        &run_storage,
                        &accepted.suggestion.id,
                        &suggestion_trace_id,
                        "completed",
                        &completed_at,
                        None,
                        outcomes,
                    )
                    .await;
                    sentinel_panel::update_chat_suggestion_proposal_run_state(
                        &run_storage,
                        accepted.proposal_id.as_deref(),
                        &accepted.suggestion.id,
                        "completed",
                        "completed",
                        Some(&suggestion_trace_id),
                        persist_summary.as_deref(),
                    )
                    .await;
                }
                crate::spawn_logged!(
                    "src/channels/http/chat_control.rs:persist_stream_success_run",
                    async move {
                        upsert_chat_stream_execution_run(
                            &persist_run_storage,
                            &persist_run_id,
                            persist_conversation_id.as_deref(),
                            &persist_channel,
                            &persist_message,
                            persist_status,
                            persist_summary,
                            None,
                            persist_trace_id,
                            persist_degradation,
                            persist_attempted_models,
                        )
                        .await;
                    }
                );
            }
            Err(error) if was_cancelled => {
                if let Some(task) = tracked_task.as_ref() {
                    let result_preview = "Cancelled by user.";
                    {
                        let agent_snapshot = Agent::snapshot(&agent_ref).await;
                        agent_snapshot
                            .swarm_activity
                            .interrupt_run(
                                &task.task_id,
                                "Cancelled by user before the delegated run completed.",
                            )
                            .await;
                        if let Err(storage_error) = agent_snapshot
                            .storage
                            .mark_swarm_run_interrupted(
                                &task.task_id,
                                "Cancelled by user before the delegated run completed.",
                            )
                            .await
                        {
                            tracing::warn!(
                                "Failed to persist interrupted swarm run '{}' after cancellation: {}",
                                task.task_id,
                                storage_error
                            );
                        }
                        match task.task_id.parse::<uuid::Uuid>() {
                            Ok(task_uuid) => {
                                if let Err(finalize_error) = agent_snapshot
                                    .finalize_task(
                                        task_uuid,
                                        crate::core::TaskStatus::Cancelled,
                                        Some(result_preview.to_string()),
                                    )
                                    .await
                                {
                                    tracing::warn!(
                                        "Failed to finalize cancelled streamed chat task '{}': {}",
                                        task.task_id,
                                        finalize_error
                                    );
                                }
                            }
                            Err(_) => {
                                tracing::warn!(
                                    "Failed to parse streamed chat task id '{}' during cancellation finalize",
                                    task.task_id
                                );
                            }
                        }
                    }
                    send_chat_stream_event(
                        &tx,
                        &live_runs,
                        &stream_request_id,
                        "chat",
                        &channel,
                        "task_status",
                        serde_json::json!({
                            "task_id": task.task_id.clone(),
                            "description": task.description.clone(),
                            "status": "cancelled",
                            "work_type": task.work_type.clone(),
                            "result_preview": result_preview,
                            "conversation_id": conversation_id.clone(),
                        }),
                        crate::core::RunEventPriority::High,
                    )
                    .await;
                }
                let cancellation_degradation = vec![crate::core::DegradationNote {
                    kind: "cancellation".to_string(),
                    summary: "run cancelled".to_string(),
                    detail: Some(error.to_string()),
                }];
                upsert_chat_stream_execution_run(
                    &run_storage,
                    &stream_request_id,
                    conversation_id.as_deref(),
                    &channel,
                    &message,
                    crate::core::ExecutionRunStatus::Cancelled,
                    Some("Cancelled by user.".to_string()),
                    Some(error.to_string()),
                    None,
                    cancellation_degradation.clone(),
                    Vec::new(),
                )
                .await;
                send_chat_stream_event(
                    &tx,
                    &live_runs,
                    &stream_request_id,
                    "chat",
                    &channel,
                    "run_status",
                    serde_json::json!({
                        "run_id": stream_request_id.clone(),
                        "run_status": "cancelled",
                        "trace_id": serde_json::Value::Null,
                        "degradation": cancellation_degradation,
                        "attempted_models": [],
                        "user_outcome": serde_json::Value::Null,
                    }),
                    crate::core::RunEventPriority::Critical,
                )
                .await;
            }
            Err(error) => {
                let error_text_for_suggestion = error.to_string();
                tracing::error!(
                    run_id = %stream_request_id,
                    conversation_id = conversation_id.as_deref().unwrap_or(""),
                    channel = %channel,
                    error = %error_text_for_suggestion,
                    "Chat stream worker failed before supervised execution completed"
                );
                if let Some(accepted) = accepted_suggestion.as_ref() {
                    let outcomes = suggestions::collect_run_outcomes(
                        &app_state,
                        &accepted.before,
                        &accepted.suggestion.kind,
                    )
                    .await;
                    let completed_at = chrono::Utc::now().to_rfc3339();
                    suggestions::update_chat_suggestion_after_run(
                        &run_storage,
                        &accepted.suggestion.id,
                        &stream_request_id,
                        "failed",
                        &completed_at,
                        Some(error_text_for_suggestion.clone()),
                        outcomes,
                    )
                    .await;
                    sentinel_panel::update_chat_suggestion_proposal_run_state(
                        &run_storage,
                        accepted.proposal_id.as_deref(),
                        &accepted.suggestion.id,
                        "failed",
                        "failed",
                        Some(&stream_request_id),
                        Some(&error_text_for_suggestion),
                    )
                    .await;
                }
                if let Some(task) = tracked_task.as_ref() {
                    let error_text = error_text_for_suggestion.clone();
                    {
                        let agent_snapshot = Agent::snapshot(&agent_ref).await;
                        match task.task_id.parse::<uuid::Uuid>() {
                            Ok(task_uuid) => {
                                if let Err(finalize_error) = agent_snapshot
                                    .finalize_task(
                                        task_uuid,
                                        crate::core::TaskStatus::Failed {
                                            error: error_text.clone(),
                                        },
                                        Some(truncate_stream_task_text(&error_text, 400)),
                                    )
                                    .await
                                {
                                    tracing::warn!(
                                        "Failed to finalize failed streamed chat task '{}': {}",
                                        task.task_id,
                                        finalize_error
                                    );
                                }
                            }
                            Err(_) => {
                                tracing::warn!(
                                    "Failed to parse streamed chat task id '{}' during failure finalize",
                                    task.task_id
                                );
                            }
                        }
                    }
                    send_chat_stream_event(
                        &tx,
                        &live_runs,
                        &stream_request_id,
                        "chat",
                        &channel,
                        "task_status",
                        serde_json::json!({
                            "task_id": task.task_id.clone(),
                            "description": task.description.clone(),
                            "status": "failed",
                            "work_type": task.work_type.clone(),
                            "result_preview": truncate_stream_task_text(&error_text, 400),
                            "conversation_id": conversation_id.clone(),
                        }),
                        crate::core::RunEventPriority::High,
                    )
                    .await;
                }

                let error_payload = serde_json::json!({ "error": error.to_string() });
                send_chat_stream_event(
                    &tx,
                    &live_runs,
                    &stream_request_id,
                    "chat",
                    &channel,
                    "error",
                    error_payload,
                    crate::core::RunEventPriority::Critical,
                )
                .await;
                let response =
                    "I hit a framework-level problem before supervised execution could finish. Please retry."
                        .to_string();
                let degradation = vec![crate::core::DegradationNote {
                    kind: "platform".to_string(),
                    summary: "framework error".to_string(),
                    detail: Some(error.to_string()),
                }];
                let user_outcome = crate::core::ExecutionSupervisor::default()
                    .build_service_outage_outcome(&response, "framework_error", &degradation, &[]);
                upsert_chat_stream_execution_run(
                    &run_storage,
                    &stream_request_id,
                    conversation_id.as_deref(),
                    &channel,
                    &message,
                    crate::core::ExecutionRunStatus::PlatformFailed,
                    Some(response),
                    Some(error.to_string()),
                    None,
                    degradation.clone(),
                    user_outcome.attempted_models.clone(),
                )
                .await;
                send_chat_stream_event(
                    &tx,
                    &live_runs,
                    &stream_request_id,
                    "chat",
                    &channel,
                    "run_status",
                    serde_json::json!({
                        "run_id": stream_request_id.clone(),
                        "run_status": "platform_failed",
                        "trace_id": serde_json::Value::Null,
                        "degradation": degradation,
                        "attempted_models": user_outcome.attempted_models,
                        "user_outcome": user_outcome,
                    }),
                    crate::core::RunEventPriority::Critical,
                )
                .await;
            }
        }

        send_chat_stream_event(
            &tx,
            &live_runs,
            &stream_request_id,
            "chat",
            &channel,
            "done",
            serde_json::json!({}),
            crate::core::RunEventPriority::Critical,
        )
        .await;
    }) {
        tracing::error!("Failed to dispatch chat stream worker: {}", error);
        return error_response(StatusCode::SERVICE_UNAVAILABLE, error);
    }

    let stream = tokio_stream::wrappers::ReceiverStream::new(rx);
    Sse::new(cap_sse_lifetime(stream))
        .keep_alive(KeepAlive::default())
        .into_response()
}

#[derive(Clone)]
pub(super) struct ResumableChatTaskRequest {
    pub(super) message: String,
    pub(super) channel: String,
    pub(super) conversation_id: String,
    pub(super) deep_research: bool,
    pub(super) work_type: String,
    pub(super) stored_plan_override: Option<serde_json::Value>,
    pub(super) paused_for_plan_confirmation: bool,
}

#[derive(Debug, Deserialize, Default)]
pub(super) struct ResumeChatTaskStreamRequest {
    #[serde(default)]
    pub(super) plan_override: Option<serde_json::Value>,
}

pub(super) fn backfill_chat_task_origin_metadata(
    task: &mut crate::core::Task,
    conversation_id: Option<&str>,
    _project_id: Option<&str>,
) -> Option<String> {
    let mut arguments = task.arguments.as_object().cloned().unwrap_or_default();
    let mut changed = false;

    if let Some(cid) = conversation_id
        .map(str::trim)
        .filter(|value| !value.is_empty())
    {
        let current = arguments
            .get("conversation_id")
            .and_then(|value| value.as_str())
            .unwrap_or("")
            .trim();
        if current != cid {
            arguments.insert(
                "conversation_id".to_string(),
                serde_json::json!(cid.to_string()),
            );
            changed = true;
        }
    }

    if !changed {
        return None;
    }

    task.arguments = serde_json::Value::Object(arguments);
    serde_json::to_string(&task.arguments).ok()
}

pub(super) fn extract_resumable_web_chat_task(
    task: &crate::core::Task,
) -> std::result::Result<ResumableChatTaskRequest, String> {
    if task.action != "chat_request" {
        return Err("Only chat-request tasks can be resumed in chat.".to_string());
    }
    let arguments = task
        .arguments
        .as_object()
        .ok_or_else(|| "This chat task is missing its stored arguments.".to_string())?;

    let pause_kind = arguments
        .get("_pause_kind")
        .and_then(|value| value.as_str())
        .unwrap_or("")
        .trim()
        .to_string();
    let paused_for_plan_confirmation = pause_kind == "plan_confirmation";
    if !matches!(
        task.status,
        crate::core::TaskStatus::Cancelled
            | crate::core::TaskStatus::Failed { .. }
            | crate::core::TaskStatus::Paused
    ) {
        return Err(
            "Only cancelled, failed, or plan-confirmation-paused chat tasks can be resumed in chat."
                .to_string(),
        );
    }
    if matches!(task.status, crate::core::TaskStatus::Paused) && !paused_for_plan_confirmation {
        return Err(
            "Only cancelled, failed, or plan-confirmation-paused chat tasks can be resumed in chat."
                .to_string(),
        );
    }

    let origin = arguments
        .get("_origin")
        .and_then(|value| value.as_str())
        .unwrap_or("")
        .trim();
    if origin != "chat" {
        return Err("Only chat-origin tasks can be resumed in chat.".to_string());
    }

    let channel = arguments
        .get("channel")
        .and_then(|value| value.as_str())
        .unwrap_or("")
        .trim()
        .to_string();
    if channel != "web" {
        return Err("Only web chat tasks can be resumed in chat.".to_string());
    }

    let message = arguments
        .get("message")
        .and_then(|value| value.as_str())
        .unwrap_or("")
        .trim()
        .to_string();
    if message.is_empty() {
        return Err(
            "This chat task no longer has its stored message, so it cannot be resumed.".to_string(),
        );
    }

    let conversation_id = arguments
        .get("conversation_id")
        .and_then(|value| value.as_str())
        .unwrap_or("")
        .trim()
        .to_string();
    if conversation_id.is_empty() {
        return Err(
            "This chat task no longer has a conversation id, so it cannot be resumed.".to_string(),
        );
    }

    let deep_research = arguments
        .get("deep_research")
        .and_then(|value| value.as_bool())
        .unwrap_or(false);
    let attachments_present = arguments
        .get("attachments_present")
        .and_then(|value| value.as_bool())
        .unwrap_or(false);
    let work_type = arguments
        .get("_work_type")
        .and_then(|value| value.as_str())
        .map(str::trim)
        .filter(|value| !value.is_empty())
        .map(ToOwned::to_owned)
        .unwrap_or_else(|| {
            if deep_research {
                "research".to_string()
            } else if attachments_present {
                "workspace".to_string()
            } else {
                "task".to_string()
            }
        });
    let stored_plan_override = arguments
        .get("_plan_preview")
        .and_then(|value| value.as_object())
        .and_then(|value| value.get("current_plan"))
        .cloned();

    Ok(ResumableChatTaskRequest {
        message,
        channel,
        conversation_id,
        deep_research,
        work_type,
        stored_plan_override,
        paused_for_plan_confirmation,
    })
}

pub(super) async fn chat_stream(
    State(state): State<AppState>,
    maybe_caller: Option<Extension<crate::actions::ActionCallerPrincipal>>,
    ConnectInfo(_addr): ConnectInfo<SocketAddr>,
    Json(request): Json<ChatRequest>,
) -> Response {
    let mut request = request;
    if request.attachments_present && request.attachments.is_empty() {
        tracing::debug!(
            "Chat stream request indicated attachments were present, but no structured attachment hints were supplied"
        );
    }
    if request.arkorbit_context.is_some() {
        tracing::debug!("Chat stream request included ArkOrbit structural context");
    }
    if let Some(response) = validate_chat_message_size(&request.message) {
        return response;
    }

    let accepted_suggestion = match prepare_accepted_chat_suggestion_launch(
        &state,
        request.accepted_suggestion_id.as_deref(),
        request.sentinel_proposal_id.as_deref(),
    )
    .await
    {
        Ok(value) => value,
        Err(response) => return response,
    };
    if let Some(accepted) = accepted_suggestion.as_ref() {
        let suggestion_conversation_id = accepted.suggestion.conversation_id.trim();
        if !suggestion_conversation_id.is_empty() {
            request.conversation_id = Some(suggestion_conversation_id.to_string());
        }
    }

    if let Some(existing_conversation_id) = request
        .conversation_id
        .as_deref()
        .map(str::trim)
        .filter(|value| !value.is_empty())
        .map(str::to_string)
    {
        let resolved_conversation_id = match resolve_chat_request_conversation_id(
            &state,
            &request.channel,
            Some(existing_conversation_id.as_str()),
            None,
            &request.message,
        )
        .await
        {
            Ok(conversation_id) => conversation_id,
            Err(response) => return response,
        };
        request.conversation_id = Some(resolved_conversation_id);
    } else {
        request.conversation_id = None;
    }

    tracing::info!(
        "HTTP /chat/stream request: channel={}, msg={}chars, conv_id={:?}",
        request.channel,
        request.message.len(),
        request.conversation_id.as_deref().unwrap_or("-"),
    );

    if let Some(result) = handle_direct_chat_approval_submit_text(&state, &request.message).await {
        let cid = request.conversation_id.clone();
        let payload = match result {
            Ok(mut payload) => {
                if let serde_json::Value::Object(map) = &mut payload {
                    map.insert("conversation_id".to_string(), serde_json::json!(cid));
                }
                payload
            }
            Err(error) => serde_json::json!({ "error": error, "conversation_id": cid }),
        };
        let (tx, rx) =
            tokio::sync::mpsc::channel::<std::result::Result<Event, std::convert::Infallible>>(4);
        crate::spawn_logged!("src/channels/http.rs:direct_chat_approval_submit", async move {
            let event_name = if payload.get("error").is_some() {
                "error"
            } else {
                "content"
            };
            let _ = tx
                .send(Ok(Event::default()
                    .event(event_name)
                    .data(payload.to_string())))
                .await;
            let _ = tx.send(Ok(Event::default().event("done").data("{}"))).await;
        });

        return Sse::new(cap_sse_lifetime(
            tokio_stream::wrappers::ReceiverStream::new(rx),
        ))
        .keep_alive(KeepAlive::default())
        .into_response();
    }

    // Internal escape hatch only. The product UX is the secure credential form.
    if let Some((key, value)) = parse_set_secret_command(&request.message) {
        if !crate::core::secrets::setsecret_command_escape_hatch_enabled() {
            let payload = serde_json::json!({
                "content": crate::core::secrets::setsecret_command_disabled_response(),
                "conversation_id": request.conversation_id,
            });
            let (tx, rx) = tokio::sync::mpsc::channel::<
                std::result::Result<Event, std::convert::Infallible>,
            >(4);
            crate::spawn_logged!("src/channels/http.rs:setsecret_disabled", async move {
                let _ = tx
                    .send(Ok(Event::default()
                        .event("content")
                        .data(payload.to_string())))
                    .await;
                let _ = tx.send(Ok(Event::default().event("done").data("{}"))).await;
            });
            return Sse::new(cap_sse_lifetime(
                tokio_stream::wrappers::ReceiverStream::new(rx),
            ))
            .keep_alive(KeepAlive::default())
            .into_response();
        }
        let cid = request.conversation_id.clone();
        let mut values = BTreeMap::new();
        values.insert(key.clone(), value);
        let payload = {
            let agent = state.agent.read().await;
            match agent
                .submit_chat_credential_values(cid.as_deref(), &values)
                .await
            {
                Ok(content) => serde_json::json!({
                    "content": content,
                    "conversation_id": cid,
                }),
                Err(error) => {
                    serde_json::json!({ "error": error.to_string() })
                }
            }
        };

        let (tx, rx) =
            tokio::sync::mpsc::channel::<std::result::Result<Event, std::convert::Infallible>>(4);
        crate::spawn_logged!("src/channels/http.rs:15726", async move {
            let event_name = if payload.get("error").is_some() {
                "error"
            } else {
                "content"
            };
            let _ = tx
                .send(Ok(Event::default()
                    .event(event_name)
                    .data(payload.to_string())))
                .await;
            let _ = tx.send(Ok(Event::default().event("done").data("{}"))).await;
        });

        return Sse::new(cap_sse_lifetime(
            tokio_stream::wrappers::ReceiverStream::new(rx),
        ))
        .keep_alive(KeepAlive::default())
        .into_response();
    }

    if let Some(content) = chat_secret_prompt_block_message(
        &state,
        request.conversation_id.as_deref(),
        &request.message,
    )
    .await
    {
        let payload = serde_json::json!({
            "content": content,
            "conversation_id": request.conversation_id,
        });
        let (tx, rx) =
            tokio::sync::mpsc::channel::<std::result::Result<Event, std::convert::Infallible>>(4);
        crate::spawn_logged!(
            "src/channels/http.rs:chat_secret_prompt_guard",
            async move {
                let _ = tx
                    .send(Ok(Event::default()
                        .event("content")
                        .data(payload.to_string())))
                    .await;
                let _ = tx.send(Ok(Event::default().event("done").data("{}"))).await;
            }
        );
        return Sse::new(cap_sse_lifetime(
            tokio_stream::wrappers::ReceiverStream::new(rx),
        ))
        .keep_alive(KeepAlive::default())
        .into_response();
    }

    // Human-in-the-loop shortcut: reuse currently configured model key without sending to the LLM.
    if let Some(key) = crate::core::secrets::parse_use_current_llm_key_command(&request.message) {
        if !crate::core::secrets::secret_command_escape_hatch_enabled() {
            let payload = serde_json::json!({
                "content": crate::core::secrets::setsecret_command_disabled_response(),
                "conversation_id": request.conversation_id,
            });
            let (tx, rx) = tokio::sync::mpsc::channel::<
                std::result::Result<Event, std::convert::Infallible>,
            >(4);
            crate::spawn_logged!("src/channels/http.rs:secret_command_disabled", async move {
                let _ = tx
                    .send(Ok(Event::default()
                        .event("content")
                        .data(payload.to_string())))
                    .await;
                let _ = tx.send(Ok(Event::default().event("done").data("{}"))).await;
            });
            return Sse::new(cap_sse_lifetime(
                tokio_stream::wrappers::ReceiverStream::new(rx),
            ))
            .keep_alive(KeepAlive::default())
            .into_response();
        }
        let cid = request.conversation_id.clone();
        let (config_dir, data_dir, llm_env) = {
            let agent = state.agent.read().await;
            (
                agent.config_dir.clone(),
                agent.data_dir.clone(),
                agent.app_model_env_vars(),
            )
        };
        let payload = if let Some(value) =
            llm_env.get(&key).cloned().filter(|v| !v.trim().is_empty())
        {
            match crate::core::secrets::store_user_secret(
                &config_dir,
                Some(&data_dir),
                &key,
                &value,
            ) {
                Ok(_) => {
                    let followup = if let Some(ref cid_str) = cid {
                        let agent = state.agent.read().await;
                        agent.on_secret_saved_followup(cid_str).await
                    } else {
                        None
                    };
                    let mut content = format!(
                        "Linked '{}' to the currently configured model credential (stored encrypted). This was not sent to the LLM.",
                        key
                    );
                    if let Some(f) = followup {
                        content.push_str("\n\n");
                        content.push_str(&f);
                    }
                    serde_json::json!({
                        "content": content,
                        "conversation_id": cid,
                    })
                }
                Err(e) => serde_json::json!({ "error": format!("Failed to store secret: {}", e) }),
            }
        } else {
            let mut available: Vec<String> = llm_env
                .iter()
                .filter_map(|(k, v)| {
                    if v.trim().is_empty() {
                        None
                    } else if k.ends_with("_API_KEY")
                        || k.ends_with("_BASE_URL")
                        || k == "LLM_MODEL"
                        || k == "LLM_PROVIDER"
                    {
                        Some(k.clone())
                    } else {
                        None
                    }
                })
                .collect();
            available.sort();
            let available_text = if available.is_empty() {
                "none".to_string()
            } else {
                available.join(", ")
            };
            serde_json::json!({
                "error": format!(
                    "Can't map '{}' from current model settings. Available model-backed keys: {}",
                    key, available_text
                )
            })
        };

        let (tx, rx) =
            tokio::sync::mpsc::channel::<std::result::Result<Event, std::convert::Infallible>>(4);
        crate::spawn_logged!("src/channels/http.rs:15821", async move {
            let event_name = if payload.get("error").is_some() {
                "error"
            } else {
                "content"
            };
            let _ = tx
                .send(Ok(Event::default()
                    .event(event_name)
                    .data(payload.to_string())))
                .await;
            let _ = tx.send(Ok(Event::default().event("done").data("{}"))).await;
        });

        return Sse::new(cap_sse_lifetime(
            tokio_stream::wrappers::ReceiverStream::new(rx),
        ))
        .keep_alive(KeepAlive::default())
        .into_response();
    }

    // Fast command path: push-notification controls without LLM roundtrip.
    if let Some(cmd) = parse_notification_control_command(&request.message) {
        let cid = request.conversation_id.clone();
        let payload = match handle_notification_control_command(&state, cmd).await {
            Ok(content) => serde_json::json!({
                "content": content,
                "conversation_id": cid,
            }),
            Err(error) => serde_json::json!({ "error": error }),
        };

        let (tx, rx) =
            tokio::sync::mpsc::channel::<std::result::Result<Event, std::convert::Infallible>>(4);
        crate::spawn_logged!("src/channels/http.rs:15854", async move {
            let event_name = if payload.get("error").is_some() {
                "error"
            } else {
                "content"
            };
            let _ = tx
                .send(Ok(Event::default()
                    .event(event_name)
                    .data(payload.to_string())))
                .await;
            let _ = tx.send(Ok(Event::default().event("done").data("{}"))).await;
        });

        return Sse::new(cap_sse_lifetime(
            tokio_stream::wrappers::ReceiverStream::new(rx),
        ))
        .keep_alive(KeepAlive::default())
        .into_response();
    }

    // Fast command path: tunnel control without LLM roundtrip.
    if let Some(cmd) = tunnel::parse_tunnel_command(&request.message) {
        let cid = request.conversation_id.clone();
        let payload = match tunnel::handle_tunnel_control_command(&state, cmd).await {
            Ok(content) => serde_json::json!({
                "content": content,
                "conversation_id": cid,
            }),
            Err(error) => serde_json::json!({ "error": error }),
        };

        let (tx, rx) =
            tokio::sync::mpsc::channel::<std::result::Result<Event, std::convert::Infallible>>(4);
        crate::spawn_logged!("src/channels/http.rs:15887", async move {
            let event_name = if payload.get("error").is_some() {
                "error"
            } else {
                "content"
            };
            let _ = tx
                .send(Ok(Event::default()
                    .event(event_name)
                    .data(payload.to_string())))
                .await;
            let _ = tx.send(Ok(Event::default().event("done").data("{}"))).await;
        });

        return Sse::new(cap_sse_lifetime(
            tokio_stream::wrappers::ReceiverStream::new(rx),
        ))
        .keep_alive(KeepAlive::default())
        .into_response();
    }

    // Fast command path: explicit autonomy helpers without LLM roundtrip.
    if let Some(cmd) = parse_autonomy_quick_command(&request.message) {
        let cid = request.conversation_id.clone();
        let payload = match handle_autonomy_quick_command(&state, cmd).await {
            Ok(content) => serde_json::json!({
                "content": content,
                "conversation_id": cid,
            }),
            Err(error) => serde_json::json!({ "error": error }),
        };

        let (tx, rx) =
            tokio::sync::mpsc::channel::<std::result::Result<Event, std::convert::Infallible>>(4);
        crate::spawn_logged!("src/channels/http.rs:15920", async move {
            let event_name = if payload.get("error").is_some() {
                "error"
            } else {
                "content"
            };
            let _ = tx
                .send(Ok(Event::default()
                    .event(event_name)
                    .data(payload.to_string())))
                .await;
            let _ = tx.send(Ok(Event::default().event("done").data("{}"))).await;
        });

        return Sse::new(cap_sse_lifetime(
            tokio_stream::wrappers::ReceiverStream::new(rx),
        ))
        .keep_alive(KeepAlive::default())
        .into_response();
    }

    let persisted_user_message = match persist_chat_stream_user_message_before_run(
        &state,
        &request.channel,
        request.conversation_id.as_deref(),
        &request.message,
    )
    .await
    {
        Ok(value) => value,
        Err(response) => return response,
    };

    spawn_chat_stream_response(
        state,
        chat_stream_run_request_from_persisted_user_message(
            request,
            persisted_user_message,
            maybe_caller.as_ref().map(|Extension(value)| value.clone()),
            accepted_suggestion,
        ),
    )
}

/// Clear conversation history for a channel
pub(super) async fn clear_chat(
    State(state): State<AppState>,
    Json(request): Json<serde_json::Value>,
) -> Response {
    let channel = request
        .get("channel")
        .and_then(|v| v.as_str())
        .unwrap_or("web");
    let agent = Agent::snapshot(&state.agent).await;
    agent.clear_conversation_history(channel).await;
    (
        StatusCode::OK,
        Json(serde_json::json!({ "status": "cleared" })),
    )
        .into_response()
}
