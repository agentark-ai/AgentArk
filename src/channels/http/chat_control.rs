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
        .map(|value| {
            if value == 0 {
                0
            } else {
                value.clamp(30, 7_200)
            }
        })
        .unwrap_or(0)
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
        tokio::time::sleep(Duration::from_secs(idle_timeout_secs.clamp(1, 30))).await;
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
        let guarded = std::panic::AssertUnwindSafe(future).catch_unwind();
        let result = if timeout_secs > 0 {
            match tokio::time::timeout(Duration::from_secs(timeout_secs), guarded).await {
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
            }
        } else {
            match guarded.await {
                Ok(output) => Ok(output),
                Err(payload) => {
                    let panic = chat_worker_panic_payload_to_string(payload);
                    tracing::error!(
                        task = task_name,
                        panic = %panic,
                        "Chat worker task panicked"
                    );
                    Err(format!("Chat worker task panicked: {}", panic))
                }
            }
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
) -> crate::core::RequestExecutionHints {
    crate::core::RequestExecutionHints {
        turn_timing_id: None,
        caller_principal: caller.cloned(),
        execution_surface: surface,
        direct_user_intent,
        recorded_user_message_id: None,
        attachments_present: false,
        attachments: Vec::new(),
        execution_profile: None,
        arkorbit_context: None,
        browser_profile_context: None,
        recent_actionable_artifacts: Vec::new(),
    }
}

fn attach_chat_request_context_to_hints(
    mut hints: crate::core::RequestExecutionHints,
    attachments_present: bool,
    attachments: Vec<crate::core::ChatAttachmentHint>,
    arkorbit_context: Option<serde_json::Value>,
    browser_profile_context: Option<serde_json::Value>,
) -> crate::core::RequestExecutionHints {
    hints.attachments_present = attachments_present || !attachments.is_empty();
    hints.attachments = attachments;
    hints.arkorbit_context = arkorbit_context;
    hints.browser_profile_context = browser_profile_context;
    hints
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
    let attachments_present = request.attachments_present || !request.attachments.is_empty();
    let attachments = request.attachments.clone();
    let arkorbit_context = request.arkorbit_context.clone();
    let browser_profile_context = request.browser_profile_context.clone();
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
    if request.browser_profile_context.is_some() {
        tracing::debug!("Chat request included browser profile context");
    }
    let caller_principal = maybe_caller.as_ref().map(|Extension(value)| value.clone());
    let agent_for_chat = state.agent.clone();
    let persisted_user_message = match persist_chat_stream_user_message_before_run(
        &state,
        &channel,
        conversation_id.as_deref(),
        &message,
    )
    .await
    {
        Ok(value) => value,
        Err(response) => return response,
    };
    let conversation_id = Some(persisted_user_message.conversation_id.clone());
    let recorded_user_message_id = Some(persisted_user_message.message_id.clone());

    let result = {
        let worker = run_chat_worker_task("http_chat_turn", async move {
            let agent_snapshot = Agent::snapshot(&agent_for_chat).await;
            let mut hints = attach_chat_request_context_to_hints(
                build_request_execution_hints(
                    caller_principal.as_ref(),
                    crate::actions::ActionExecutionSurface::Chat,
                    true,
                ),
                attachments_present,
                attachments,
                arkorbit_context,
                browser_profile_context,
            );
            hints.recorded_user_message_id = recorded_user_message_id;
            agent_snapshot
                .process_message_prerecorded_with_meta_and_hints(
                    &message,
                    &channel,
                    conversation_id.as_deref(),
                    project_id.as_deref(),
                    hints,
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
                response: crate::core::llm_context_sanitizer::strip_internal_tool_transcript(
                    &processed.response,
                ),
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

fn chat_stream_first_token_metric_value(observed_first_token_ms: u64) -> serde_json::Value {
    if observed_first_token_ms > 0 {
        serde_json::json!(observed_first_token_ms)
    } else {
        serde_json::Value::Null
    }
}

fn redact_stream_visible_text(value: &str) -> String {
    let secret_redacted = crate::security::redact_secret_input(value).text;
    crate::security::redact_pii(&secret_redacted)
}

fn compact_stream_visible_json(value: &serde_json::Value, max_chars: usize) -> Option<String> {
    let visible = stream_visible_json_value(value, 0).unwrap_or_else(|| value.clone());
    let redacted_value = redact_stream_visible_json_value(&visible, "");
    let raw = match &redacted_value {
        serde_json::Value::String(text) => text.trim().to_string(),
        _ => serde_json::to_string(&redacted_value).ok()?,
    };
    let compacted = compact_stream_summary_text(&raw, max_chars);
    if compacted.is_empty() {
        None
    } else {
        Some(compacted)
    }
}

fn stream_visible_json_value(value: &serde_json::Value, depth: usize) -> Option<serde_json::Value> {
    if depth > 6 {
        return None;
    }
    match value {
        serde_json::Value::Null => None,
        serde_json::Value::Bool(_) | serde_json::Value::Number(_) => Some(value.clone()),
        serde_json::Value::String(text) => {
            let trimmed = text.trim();
            if trimmed.is_empty() {
                None
            } else if let Ok(parsed) = serde_json::from_str::<serde_json::Value>(trimmed) {
                stream_visible_json_value(&parsed, depth + 1)
                    .or_else(|| Some(serde_json::Value::String(trimmed.to_string())))
            } else {
                Some(serde_json::Value::String(trimmed.to_string()))
            }
        }
        serde_json::Value::Array(items) => {
            if items.is_empty() {
                None
            } else {
                Some(serde_json::Value::Array(items.clone()))
            }
        }
        serde_json::Value::Object(object) => {
            let mut visible_entries = object
                .iter()
                .filter(|(key, value)| {
                    !stream_visible_metadata_key(key)
                        && !stream_sensitive_field_key(key)
                        && stream_json_has_visible_content(value)
                })
                .collect::<Vec<_>>();
            if visible_entries.is_empty() {
                visible_entries = object
                    .iter()
                    .filter(|(key, value)| {
                        !stream_sensitive_field_key(key) && stream_json_has_visible_content(value)
                    })
                    .collect::<Vec<_>>();
            }
            if visible_entries.is_empty() {
                return None;
            }

            let structured_entries = visible_entries
                .iter()
                .filter(|(_, value)| match value {
                    serde_json::Value::Array(items) => !items.is_empty(),
                    serde_json::Value::Object(object) => !object.is_empty(),
                    _ => false,
                })
                .collect::<Vec<_>>();
            if structured_entries.len() == 1 {
                let nested = stream_visible_json_value(structured_entries[0].1, depth + 1);
                return nested.or_else(|| Some(structured_entries[0].1.clone()));
            }
            if !structured_entries.is_empty() {
                let mut out = serde_json::Map::new();
                for (key, value) in structured_entries {
                    out.insert((*key).clone(), (*value).clone());
                }
                return Some(serde_json::Value::Object(out));
            }
            if visible_entries.len() == 1 {
                return Some(visible_entries[0].1.clone());
            }
            let mut out = serde_json::Map::new();
            for (key, value) in visible_entries {
                out.insert(key.clone(), value.clone());
            }
            Some(serde_json::Value::Object(out))
        }
    }
}

fn stream_json_has_visible_content(value: &serde_json::Value) -> bool {
    match value {
        serde_json::Value::Null => false,
        serde_json::Value::String(text) => !text.trim().is_empty(),
        serde_json::Value::Array(items) => !items.is_empty(),
        serde_json::Value::Object(object) => !object.is_empty(),
        serde_json::Value::Bool(_) | serde_json::Value::Number(_) => true,
    }
}

fn stream_normalized_key(value: &str) -> String {
    let mut out = String::new();
    let mut previous_was_separator = true;
    for ch in value.chars() {
        if ch.is_ascii_uppercase() {
            if !previous_was_separator && !out.ends_with('_') {
                out.push('_');
            }
            out.push(ch.to_ascii_lowercase());
            previous_was_separator = false;
        } else if ch.is_ascii_alphanumeric() {
            out.push(ch.to_ascii_lowercase());
            previous_was_separator = false;
        } else if !previous_was_separator {
            out.push('_');
            previous_was_separator = true;
        }
    }
    out.trim_matches('_').to_string()
}

fn stream_visible_metadata_key(key: &str) -> bool {
    let normalized = stream_normalized_key(key);
    if normalized.is_empty() || normalized.starts_with('_') {
        return true;
    }
    normalized.split('_').any(|part| {
        matches!(
            part,
            "activity"
                | "call"
                | "cid"
                | "conversation"
                | "display"
                | "id"
                | "kind"
                | "label"
                | "name"
                | "ok"
                | "renderer"
                | "run"
                | "seq"
                | "sequence"
                | "state"
                | "status"
                | "stream"
                | "surface"
                | "task"
                | "time"
                | "timestamp"
                | "tool"
                | "trace"
                | "type"
                | "version"
        )
    })
}

fn stream_sensitive_field_key(key: &str) -> bool {
    let normalized = stream_normalized_key(key);
    normalized.split('_').any(|part| {
        matches!(
            part,
            "authorization"
                | "auth"
                | "apikey"
                | "cookie"
                | "cookies"
                | "credential"
                | "credentials"
                | "password"
                | "passcode"
                | "secret"
                | "session"
                | "token"
        )
    }) || normalized.contains("api_key")
        || normalized.contains("private_key")
        || normalized.contains("refresh_token")
}

fn redact_stream_visible_json_value(value: &serde_json::Value, key: &str) -> serde_json::Value {
    if stream_sensitive_field_key(key) {
        return serde_json::Value::String("[REDACTED]".to_string());
    }
    match value {
        serde_json::Value::Null | serde_json::Value::Bool(_) | serde_json::Value::Number(_) => {
            value.clone()
        }
        serde_json::Value::String(text) => {
            serde_json::Value::String(redact_stream_visible_text(text.trim()))
        }
        serde_json::Value::Array(items) => serde_json::Value::Array(
            items
                .iter()
                .take(25)
                .map(|item| redact_stream_visible_json_value(item, ""))
                .collect(),
        ),
        serde_json::Value::Object(object) => {
            let mut out = serde_json::Map::new();
            for (entry_key, entry_value) in object {
                if entry_key.starts_with('_') || !stream_json_has_visible_content(entry_value) {
                    continue;
                }
                out.insert(
                    entry_key.clone(),
                    redact_stream_visible_json_value(entry_value, entry_key),
                );
            }
            serde_json::Value::Object(out)
        }
    }
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

        if !obj.is_empty() {
            return compact_stream_visible_json(value, 480);
        }
    } else if let Some(items) = value.as_array() {
        if !items.is_empty() {
            return compact_stream_visible_json(value, 480);
        }
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
        return compact_stream_summary_text(&redact_stream_visible_text(trimmed), 240);
    }

    redact_stream_visible_text(trimmed)
        .chars()
        .take(240)
        .collect::<String>()
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
        return "Agent workflow".to_string();
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
    let is_agent_loop_progress = name.trim().eq_ignore_ascii_case("agent_turn_loop")
        && payload
            .get("kind")
            .and_then(|value| value.as_str())
            .is_some_and(|value| value == "agent_loop_progress");
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
    if (status == "running" || status == "pending") && !is_agent_loop_progress {
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
            "role": if is_agent_loop_progress {
                "progress"
            } else if renderer_id == "agentark.terminal.transcript.v1" {
                "transcript"
            } else {
                "output"
            },
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

fn stream_reasoning_phase_is_visible(phase: &str) -> bool {
    let normalized = phase.trim().to_ascii_lowercase();
    matches!(
        normalized.as_str(),
        "model" | "model_summary" | "reasoning" | "reasoning_summary"
    )
}

fn stream_event_first_activity_source(ev: &crate::core::StreamEvent) -> Option<&'static str> {
    match ev {
        crate::core::StreamEvent::Token(content) if !content.is_empty() => Some("token"),
        crate::core::StreamEvent::ReasoningDelta {
            phase,
            content_delta,
            done,
        } if stream_reasoning_phase_is_visible(phase)
            && (*done || !content_delta.trim().is_empty()) =>
        {
            Some("reasoning_delta")
        }
        crate::core::StreamEvent::ToolStart { .. } => Some("tool_start"),
        crate::core::StreamEvent::ToolProgress {
            content, payload, ..
        } if !content.trim().is_empty() || payload.is_some() => Some("tool_progress"),
        crate::core::StreamEvent::ToolResult { content, .. } if !content.trim().is_empty() => {
            Some("tool_result")
        }
        crate::core::StreamEvent::ChatTaskStarted { .. } => Some("task_started"),
        crate::core::StreamEvent::PlanGenerated { .. } => Some("plan_generated"),
        crate::core::StreamEvent::PlanStepUpdate { .. } => Some("plan_step_update"),
        _ => None,
    }
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
            Some(("token", chat_stream_token_payload(&content, false))),
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
            if !stream_reasoning_phase_is_visible(normalized_phase) {
                return (None, String::new());
            }
            let normalized_phase_lower = normalized_phase.to_ascii_lowercase();
            let title = match normalized_phase_lower.as_str() {
                "model" | "reasoning" => "Thinking",
                "model_summary" | "reasoning_summary" => "Reasoning summary",
                _ => "Reasoning",
            };
            let stream_key = if normalized_phase.is_empty() {
                "reasoning:active".to_string()
            } else {
                format!("reasoning:{}", normalized_phase)
            };
            let sanitized_delta =
                crate::core::llm_context_sanitizer::strip_internal_tool_transcript_preserve_spacing(
                    &redact_stream_visible_text(&content_delta),
                );
            let detail = if !sanitized_delta.trim().is_empty() {
                sanitized_delta.trim().to_string()
            } else {
                format!("{title} completed.")
            };
            let content = sanitized_delta;
            (
                Some((
                    "reasoning_delta",
                    serde_json::json!({
                        "kind": "reasoning_delta",
                        "phase": normalized_phase,
                        "title": title,
                        "detail": detail,
                        "content": content,
                        "content_delta": content,
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
    let parsed = serde_json::from_str::<serde_json::Value>(response.trim()).ok();
    if let Some(status) = parsed
        .as_ref()
        .and_then(|value| value.get("status"))
        .and_then(|value| value.as_str())
        .map(|value| value.trim().to_ascii_lowercase())
    {
        return match status.as_str() {
            "needs_input" | "needs_permission" | "needs_credentials" | "needs_integration"
            | "approval_required" => crate::core::TaskStatus::Paused,
            "failed" | "failure" | "error" | "platform_failed" => crate::core::TaskStatus::Failed {
                error: truncate_stream_task_text(response, 240),
            },
            "cancelled" => crate::core::TaskStatus::Cancelled,
            _ => crate::core::TaskStatus::Completed,
        };
    }
    crate::core::TaskStatus::Completed
}

pub(super) fn chat_task_terminal_status_for_run(
    response: &str,
    run_status: Option<&str>,
    user_outcome: Option<&crate::core::UserFacingOutcome>,
) -> crate::core::TaskStatus {
    let normalized_run_status = run_status
        .map(|value| value.trim().to_ascii_lowercase())
        .filter(|value| !value.is_empty());
    match normalized_run_status.as_deref() {
        Some("degraded") | Some("platform_failed") => crate::core::TaskStatus::Failed {
            error: truncate_stream_task_text(response, 240),
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
                    error: truncate_stream_task_text(response, 240),
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

async fn persist_chat_stream_message(
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
        tool_calls_json: None,
        tool_call_id: None,
        provider_message_json: None,
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
            "Failed to persist chat stream {} message for conversation '{}': {}",
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

#[derive(Clone)]
pub(super) struct StreamedChatTask {
    pub(super) task_id: String,
    pub(super) description: String,
    pub(super) work_type: String,
    pub(super) user_message_already_recorded: bool,
    pub(super) execution_profile: Option<ChatExecutionProfile>,
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

#[derive(Clone, Debug, PartialEq)]
pub(super) struct ChatExecutionProfile {
    pub(super) capability_tags: Vec<String>,
    pub(super) depth_hint: Option<String>,
    pub(super) deliverables: Vec<String>,
    pub(super) long_running: bool,
    pub(super) confidence: Option<f64>,
    pub(super) source: String,
}

impl ChatExecutionProfile {
    fn explicit_deep_research() -> Self {
        Self {
            capability_tags: vec![
                "research".to_string(),
                "source_synthesis".to_string(),
                "decision_grade".to_string(),
            ],
            depth_hint: Some("deep".to_string()),
            deliverables: Vec::new(),
            long_running: true,
            confidence: Some(1.0),
            source: "ui_override".to_string(),
        }
    }

    fn deep_research_override_with(semantic: Option<Self>) -> Self {
        let mut profile = semantic.unwrap_or_else(Self::explicit_deep_research);
        for tag in ["research", "source_synthesis", "decision_grade"] {
            if !profile.capability_tags.iter().any(|value| value == tag) {
                profile.capability_tags.push(tag.to_string());
            }
        }
        profile.depth_hint = Some("deep".to_string());
        profile.long_running = true;
        profile.confidence = Some(1.0);
        profile.source = if profile.source == "semantic_classifier" {
            "ui_override+semantic_classifier".to_string()
        } else {
            "ui_override".to_string()
        };
        profile
    }

    fn is_ui_override(&self) -> bool {
        self.source
            .split('+')
            .any(|part| part.trim() == "ui_override")
    }

    fn from_classifier_value(value: &serde_json::Value, source: &str) -> Option<Self> {
        let confidence = value.get("confidence").and_then(|value| value.as_f64());
        if confidence.is_some_and(|value| value < 0.65) {
            return None;
        }
        let mut capability_tags =
            normalize_chat_profile_list(value.get("capability_tags")).unwrap_or_default();
        if capability_tags.is_empty() {
            if let Some(legacy) = normalize_chat_profile_label(
                value.get("work_type").and_then(|value| value.as_str()),
            ) {
                capability_tags.push(legacy);
            }
        }
        let depth_hint = normalize_chat_profile_label(
            value
                .get("depth_hint")
                .or_else(|| value.get("depth"))
                .and_then(|value| value.as_str()),
        );
        let deliverables =
            normalize_chat_profile_list(value.get("deliverables")).unwrap_or_default();
        let long_running = value
            .get("long_running")
            .and_then(|value| value.as_bool())
            .unwrap_or(false);
        Some(Self {
            capability_tags,
            depth_hint,
            deliverables,
            long_running,
            confidence,
            source: source.to_string(),
        })
    }

    fn from_stored_value(value: &serde_json::Value) -> Option<Self> {
        Self::from_classifier_value(
            value,
            value
                .get("source")
                .and_then(|item| item.as_str())
                .unwrap_or("stored"),
        )
    }

    fn to_value(&self) -> serde_json::Value {
        serde_json::json!({
            "capability_tags": self.capability_tags,
            "depth_hint": self.depth_hint,
            "deliverables": self.deliverables,
            "long_running": self.long_running,
            "confidence": self.confidence,
            "source": self.source,
        })
    }
}

fn normalize_chat_profile_label(raw: Option<&str>) -> Option<String> {
    let sanitized = raw?
        .trim()
        .to_ascii_lowercase()
        .chars()
        .map(|ch| {
            if ch.is_ascii_alphanumeric() || ch == '_' || ch == '-' {
                ch
            } else {
                '_'
            }
        })
        .collect::<String>();
    let token = sanitized
        .split('_')
        .filter(|part| !part.is_empty())
        .collect::<Vec<_>>()
        .join("_");
    (!token.is_empty() && token.chars().count() <= 64).then_some(token)
}

fn normalize_chat_profile_list(value: Option<&serde_json::Value>) -> Option<Vec<String>> {
    let values = match value? {
        serde_json::Value::String(item) => vec![item.as_str()],
        serde_json::Value::Array(items) => items
            .iter()
            .filter_map(|item| item.as_str())
            .collect::<Vec<_>>(),
        _ => Vec::new(),
    };
    let mut out = Vec::new();
    for value in values {
        if let Some(token) = normalize_chat_profile_label(Some(value)) {
            if !out.contains(&token) {
                out.push(token);
            }
        }
    }
    Some(out)
}

fn extract_chat_execution_profile_value(text: &str) -> Option<serde_json::Value> {
    serde_json::from_str::<serde_json::Value>(text.trim())
        .ok()
        .or_else(|| {
            let bytes = text.as_bytes();
            let mut start = None::<usize>;
            let mut depth = 0usize;
            let mut in_string = false;
            let mut escaped = false;

            for (index, byte) in bytes.iter().enumerate() {
                if in_string {
                    if escaped {
                        escaped = false;
                        continue;
                    }
                    match byte {
                        b'\\' => escaped = true,
                        b'"' => in_string = false,
                        _ => {}
                    }
                    continue;
                }
                match byte {
                    b'"' => in_string = true,
                    b'{' => {
                        if depth == 0 {
                            start = Some(index);
                        }
                        depth += 1;
                    }
                    b'}' => {
                        if depth == 0 {
                            continue;
                        }
                        depth -= 1;
                        if depth == 0 {
                            let begin = start?;
                            let candidate = &text[begin..=index];
                            if let Ok(value) = serde_json::from_str::<serde_json::Value>(candidate)
                            {
                                if value.is_object() {
                                    return Some(value);
                                }
                            }
                            start = None;
                        }
                    }
                    _ => {}
                }
            }
            None
        })
}

fn chat_execution_profile_classifier_timeout_ms() -> u64 {
    std::env::var("AGENTARK_CHAT_EXECUTION_PROFILE_TIMEOUT_MS")
        .ok()
        .and_then(|value| value.trim().parse::<u64>().ok())
        .map(|value| value.clamp(500, 15_000))
        .unwrap_or(8_000)
}

async fn infer_chat_execution_profile(
    agent_ref: &Arc<RwLock<Agent>>,
    message: &str,
    attachments_present: bool,
) -> Option<ChatExecutionProfile> {
    let agent_snapshot = Agent::snapshot(agent_ref).await;
    let classifier = agent_snapshot
        .llm_for_role(&crate::core::ModelRole::Fast)
        .clone();
    let system = r#"Describe the user's request as an AgentArk execution profile.

Return only compact JSON with this schema:
{
  "capability_tags": [string],
  "depth_hint": string | null,
  "deliverables": [string],
  "long_running": boolean,
  "confidence": number
}

Infer from the underlying intent and expected work, not from exact words. Use short semantic capability tags for the capabilities likely needed; do not choose from a fixed taxonomy. Use depth_hint and long_running for broad, multi-source, multi-step, expensive, or decision-grade work. Put artifact formats in deliverables when the user expects a saved or rendered output. If the intent is uncertain, keep tags sparse and confidence below 0.65."#;
    let user = format!(
        "User request:\n{}\n\nStructured context:\nattachments_present: {}",
        message.trim(),
        attachments_present
    );
    let call = classifier.chat_classifier_bounded(system, &user, 260);
    let response = match tokio::time::timeout(
        std::time::Duration::from_millis(chat_execution_profile_classifier_timeout_ms()),
        call,
    )
    .await
    {
        Ok(Ok(value)) => value,
        Ok(Err(error)) => {
            tracing::debug!("Chat execution profile classification failed: {}", error);
            return None;
        }
        Err(_) => {
            tracing::debug!("Chat execution profile classification timed out");
            return None;
        }
    };
    let value = extract_chat_execution_profile_value(&response.content)?;
    ChatExecutionProfile::from_classifier_value(&value, "semantic_classifier")
}

#[derive(Clone)]
pub(super) struct ChatStreamRunRequest {
    pub(super) message: String,
    pub(super) channel: String,
    pub(super) conversation_id: Option<String>,
    pub(super) user_message_already_recorded: bool,
    pub(super) recorded_user_message_id: Option<String>,
    pub(super) deep_research: bool,
    pub(super) attachments_present: bool,
    pub(super) attachments: Vec<crate::core::ChatAttachmentHint>,
    pub(super) arkorbit_context: Option<serde_json::Value>,
    pub(super) browser_profile_context: Option<serde_json::Value>,
    pub(super) caller_principal: Option<crate::actions::ActionCallerPrincipal>,
    pub(super) task_mode: ChatStreamTaskMode,
    pub(super) accepted_suggestion: Option<AcceptedChatSuggestionRun>,
    pub(super) execution_profile: Option<ChatExecutionProfile>,
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
    let request_execution_profile = request
        .execution_profile
        .as_ref()
        .and_then(ChatExecutionProfile::from_stored_value)
        .or_else(|| {
            request
                .deep_research
                .then(ChatExecutionProfile::explicit_deep_research)
        });
    ChatStreamRunRequest {
        message: request.message,
        channel: request.channel,
        conversation_id: request.conversation_id,
        user_message_already_recorded: true,
        recorded_user_message_id: Some(persisted_user_message.message_id),
        deep_research: request.deep_research,
        attachments_present: request.attachments_present || !request.attachments.is_empty(),
        attachments: request.attachments,
        arkorbit_context: request.arkorbit_context,
        browser_profile_context: request.browser_profile_context,
        caller_principal,
        task_mode: ChatStreamTaskMode::CreateIfNeeded,
        accepted_suggestion,
        execution_profile: request_execution_profile,
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
        tool_calls_json: None,
        tool_call_id: None,
        provider_message_json: None,
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
                .event(event_name)
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
            chat_stream_token_payload(&chunk, true),
            crate::core::RunEventPriority::Normal,
        )
        .await;
        if idx < 180 {
            tokio::time::sleep(std::time::Duration::from_millis(8)).await;
        }
    }
}

fn chat_stream_token_payload(content: &str, synthetic: bool) -> serde_json::Value {
    if synthetic {
        serde_json::json!({ "content": content, "synthetic": true })
    } else {
        serde_json::json!({ "content": content })
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

fn active_chat_stream_conflict_response(conversation_id: &str, active_run_id: &str) -> Response {
    (
        StatusCode::CONFLICT,
        Json(serde_json::json!({
            "error": "This conversation already has an active run. Stop it or wait for it to finish before sending another message.",
            "conversation_id": conversation_id,
            "active_run_id": active_run_id,
            "retryable": true,
        })),
    )
        .into_response()
}

pub(super) async fn reject_if_chat_conversation_stream_active(
    state: &AppState,
    conversation_id: Option<&str>,
) -> Option<Response> {
    let conversation_id = conversation_id
        .map(str::trim)
        .filter(|value| !value.is_empty())?;
    active_chat_conversation_request_id(state, conversation_id)
        .await
        .map(|active_run_id| active_chat_stream_conflict_response(conversation_id, &active_run_id))
}

pub(super) async fn spawn_chat_stream_response(
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
    let legacy_deep_research = request.deep_research;
    let attachments_present = request.attachments_present;
    let attachments = request.attachments.clone();
    let arkorbit_context = request.arkorbit_context.clone();
    let browser_profile_context = request.browser_profile_context.clone();
    let caller_principal = request.caller_principal.clone();
    let accepted_suggestion = request.accepted_suggestion.clone();
    let task_mode = request.task_mode.clone();
    let requested_execution_profile = request.execution_profile.clone();
    let app_state = state.clone();
    let stream_request_id = uuid::Uuid::new_v4().to_string();
    let stream_started_at = Instant::now();
    let time_to_first_stream_activity_ms = Arc::new(AtomicU64::new(0));
    let time_to_first_token_ms = Arc::new(AtomicU64::new(0));
    let stream_last_activity_ms = Arc::new(AtomicU64::new(0));
    let (cancel_tx, cancel_rx) = tokio::sync::watch::channel(false);
    let registered_conversation_id = conversation_id
        .as_deref()
        .map(str::trim)
        .filter(|value| !value.is_empty())
        .unwrap_or(stream_request_id.as_str())
        .to_string();
    if let Err(active_run_id) = try_register_chat_conversation_cancellation_sender(
        &state,
        &registered_conversation_id,
        &stream_request_id,
        cancel_tx.clone(),
    )
    .await
    {
        return active_chat_stream_conflict_response(&registered_conversation_id, &active_run_id);
    }
    let worker_stream_request_id = stream_request_id.clone();
    let worker_registered_conversation_id = registered_conversation_id.clone();

    if let Err(error) = spawn_chat_worker_logged("http_chat_stream_turn", async move {
        let stream_request_id = worker_stream_request_id;
        let registered_conversation_id = worker_registered_conversation_id;
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
        let mut cancel_rx = cancel_rx;
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
            let time_to_first_stream_activity_ms = time_to_first_stream_activity_ms.clone();
            let time_to_first_token_ms = time_to_first_token_ms.clone();
            let stream_last_activity_ms = stream_last_activity_ms.clone();
            let live_runs = live_runs.clone();
            let stream_request_id = stream_request_id.clone();
            let channel = channel.clone();
            crate::spawn_logged!("src/channels/http.rs:14840", async move {
                let mut last_thinking_detail = String::new();
                let mut internal_token_filter =
                    crate::core::llm_context_sanitizer::InternalToolTranscriptStreamFilter::new();
                while let Some(ev) = stream_rx.recv().await {
                    let ev = match ev {
                        crate::core::StreamEvent::Token(content) => {
                            let content = internal_token_filter.feed(&content);
                            if content.is_empty() {
                                continue;
                            }
                            crate::core::StreamEvent::Token(content)
                        }
                        other => other,
                    };
                    stream_last_activity_ms.store(
                        stream_started_at
                            .elapsed()
                            .as_millis()
                            .min(u128::from(u64::MAX)) as u64,
                        Ordering::Relaxed,
                    );
                    if let Some(activity_source) = stream_event_first_activity_source(&ev) {
                        let elapsed_ms = stream_started_at
                            .elapsed()
                            .as_millis()
                            .min(u64::MAX as u128) as u64;
                        let recorded_ms = elapsed_ms.max(1);
                        if time_to_first_stream_activity_ms
                            .compare_exchange(0, recorded_ms, Ordering::Relaxed, Ordering::Relaxed)
                            .is_ok()
                        {
                            let mut trace = trace_ref.write().await;
                            trace.steps.push(crate::core::ExecutionStep {
                                icon: "[model]".to_string(),
                                title: "First Stream Activity".to_string(),
                                detail: format!(
                                    "Model stream activity began after {}ms via {}.",
                                    recorded_ms, activity_source
                                ),
                                step_type: "info".to_string(),
                                data: Some(
                                    serde_json::json!({
                                        "metric": "time_to_first_stream_activity",
                                        "source": activity_source,
                                        "duration_ms": recorded_ms
                                    })
                                    .to_string(),
                                ),
                                timestamp: chrono::Utc::now(),
                                duration_ms: Some(recorded_ms),
                            });
                        }
                    }
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
                                    title: "First Content".to_string(),
                                    detail: format!(
                                        "AgentArk produced the first user-visible response content after {}ms.",
                                        recorded_ms
                                    ),
                                    step_type: "info".to_string(),
                                    data: Some(
                                        serde_json::json!({
                                            "metric": "time_to_first_token",
                                            "source": "token",
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
                                execution_profile: None,
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
                let tail = internal_token_filter.finish();
                if !tail.is_empty() {
                    send_chat_stream_event(
                        &tx,
                        &live_runs,
                        &stream_request_id,
                        "chat",
                        &channel,
                        "token",
                        chat_stream_token_payload(&tail, false),
                        crate::core::RunEventPriority::Normal,
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
                    persist_chat_stream_message(
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
                        persist_chat_stream_message(
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

        let mut execution_profile = requested_execution_profile.clone();
        if accepted_suggestion.is_none() && task_mode_create_if_needed {
            let profile_is_override = execution_profile
                .as_ref()
                .is_some_and(ChatExecutionProfile::is_ui_override);
            if legacy_deep_research || profile_is_override {
                let semantic_profile =
                    infer_chat_execution_profile(&agent_ref, &message, attachments_present).await;
                let previous_profile = execution_profile.take();
                execution_profile = Some(ChatExecutionProfile::deep_research_override_with(
                    semantic_profile.or(previous_profile),
                ));
            } else if execution_profile.is_none() {
                execution_profile =
                    infer_chat_execution_profile(&agent_ref, &message, attachments_present).await;
            }
        }
        let tracked_task_snapshot = tracked_task_ref.read().await.clone();
        let resume_existing_chat_task = tracked_task_snapshot.is_some();
        let user_message_already_recorded = tracked_task_snapshot
            .as_ref()
            .map(|task| task.user_message_already_recorded)
            .unwrap_or(request_user_message_already_recorded);
        let execution_profile_value = tracked_task_snapshot
            .as_ref()
            .and_then(|task| task.execution_profile.as_ref())
            .or(execution_profile.as_ref())
            .map(ChatExecutionProfile::to_value);
        let mut process_handle = {
            let agent_ref = agent_ref.clone();
            let message = message.clone();
            let channel = channel.clone();
            let conversation_id = conversation_id.clone();
            let project_id = project_id.clone();
            let trace_ref = trace_ref.clone();
            let caller_principal = caller_principal.clone();
            let recorded_user_message_id = request_recorded_user_message_id.clone();
            let attachments_present = attachments_present;
            let attachments = attachments.clone();
            let arkorbit_context = arkorbit_context.clone();
            let browser_profile_context = browser_profile_context.clone();
            let execution_profile_value = execution_profile_value.clone();
            tokio::spawn(async move {
                let agent_snapshot = Agent::snapshot(&agent_ref).await;
                if resume_existing_chat_task {
                    let mut hints = attach_chat_request_context_to_hints(
                        build_request_execution_hints(
                            caller_principal.as_ref(),
                            crate::actions::ActionExecutionSurface::Chat,
                            true,
                        ),
                        attachments_present,
                        attachments.clone(),
                        arkorbit_context.clone(),
                        browser_profile_context.clone(),
                    );
                    hints.execution_profile = execution_profile_value.clone();
                    agent_snapshot
                        .process_message_stream_resume_with_meta_and_hints(
                            &message,
                            &channel,
                            conversation_id.as_deref(),
                            project_id.as_deref(),
                            trace_ref,
                            stream_tx,
                            hints,
                        )
                        .await
                } else if user_message_already_recorded {
                    let mut hints = attach_chat_request_context_to_hints(
                        build_request_execution_hints(
                            caller_principal.as_ref(),
                            crate::actions::ActionExecutionSurface::Chat,
                            true,
                        ),
                        attachments_present,
                        attachments.clone(),
                        arkorbit_context.clone(),
                        browser_profile_context.clone(),
                    );
                    hints.recorded_user_message_id = recorded_user_message_id;
                    hints.execution_profile = execution_profile_value.clone();
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
                            {
                                let mut hints = attach_chat_request_context_to_hints(
                                    build_request_execution_hints(
                                        caller_principal.as_ref(),
                                        crate::actions::ActionExecutionSurface::Chat,
                                        true,
                                    ),
                                    attachments_present,
                                    attachments,
                                    arkorbit_context,
                                    browser_profile_context,
                                );
                                hints.execution_profile = execution_profile_value;
                                hints
                            },
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
            Ok(mut processed) => {
                processed.response =
                    crate::core::llm_context_sanitizer::strip_internal_tool_transcript(
                        &processed.response,
                    );
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
                    let terminal_status = chat_task_terminal_status_for_run(
                        &processed.response,
                        processed.run_status.as_deref(),
                        processed.user_outcome.as_ref(),
                    );
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

                if time_to_first_stream_activity_ms.load(Ordering::Relaxed) == 0
                    && !processed.response.trim().is_empty()
                {
                    let synthetic_first_activity_ms = stream_started_at
                        .elapsed()
                        .as_millis()
                        .min(u64::MAX as u128)
                        as u64;
                    time_to_first_stream_activity_ms
                        .store(synthetic_first_activity_ms.max(1), Ordering::Relaxed);
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
                let first_stream_activity_ms =
                    time_to_first_stream_activity_ms.load(Ordering::Relaxed);
                let first_activity_ms = if first_stream_activity_ms > 0 {
                    first_stream_activity_ms
                } else {
                    stream_started_at
                        .elapsed()
                        .as_millis()
                        .min(u64::MAX as u128) as u64
                }
                .max(1);
                let observed_first_token_ms = time_to_first_token_ms.load(Ordering::Relaxed);
                let wall_duration_ms = stream_started_at
                    .elapsed()
                    .as_millis()
                    .min(u64::MAX as u128) as u64;
                let first_token_value =
                    chat_stream_first_token_metric_value(observed_first_token_ms);
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
                    "cached_prompt_tokens": processed.cached_prompt_tokens,
                    "cache_creation_prompt_tokens": processed.cache_creation_prompt_tokens,
                    "duration_ms": wall_duration_ms,
                    "trace_duration_ms": trace_metric_snapshot["duration_ms"],
                    "time_to_first_stream_activity_ms": first_activity_ms,
                    "time_to_first_token_ms": first_token_value,
                    "model_latency_ms": processed.model_latency_ms(),
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
                        "cached_prompt_tokens": content["cached_prompt_tokens"],
                        "cache_creation_prompt_tokens": content["cache_creation_prompt_tokens"],
                        "duration_ms": content["duration_ms"],
                        "trace_duration_ms": content["trace_duration_ms"],
                        "time_to_first_stream_activity_ms": content["time_to_first_stream_activity_ms"],
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
        unregister_chat_conversation_cancellation(
            &state,
            &registered_conversation_id,
            &stream_request_id,
        )
        .await;
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
    pub(super) execution_profile: Option<ChatExecutionProfile>,
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
    let work_type = arguments
        .get("_work_type")
        .and_then(|value| value.as_str())
        .map(str::trim)
        .filter(|value| !value.is_empty())
        .map(ToOwned::to_owned)
        .unwrap_or_else(|| "task".to_string());
    let stored_plan_override = arguments
        .get("_plan_preview")
        .and_then(|value| value.as_object())
        .and_then(|value| value.get("current_plan"))
        .cloned();
    let execution_profile = arguments
        .get("_execution_profile")
        .and_then(ChatExecutionProfile::from_stored_value);

    Ok(ResumableChatTaskRequest {
        message,
        channel,
        conversation_id,
        deep_research,
        work_type,
        stored_plan_override,
        execution_profile,
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
    if request.browser_profile_context.is_some() {
        tracing::debug!("Chat stream request included browser profile context");
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
        crate::spawn_logged!(
            "src/channels/http.rs:direct_chat_approval_submit",
            async move {
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
            }
        );

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

    if let Some(response) =
        reject_if_chat_conversation_stream_active(&state, request.conversation_id.as_deref()).await
    {
        return response;
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
    .await
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

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn chat_task_status_uses_structured_run_status_before_text_fallback() {
        assert!(matches!(
            chat_task_terminal_status_for_run("I will keep working.", Some("blocked"), None),
            crate::core::TaskStatus::Paused
        ));
        assert!(matches!(
            chat_task_terminal_status_for_run("Partial result", Some("platform_failed"), None),
            crate::core::TaskStatus::Failed { .. }
        ));
    }

    #[test]
    fn execution_profile_uses_semantic_capability_tags_without_routing() {
        let profile = ChatExecutionProfile::from_classifier_value(
            &serde_json::json!({
                "capability_tags": ["research", "source synthesis"],
                "depth_hint": "deep",
                "deliverables": ["pdf", "document"],
                "long_running": true,
                "confidence": 0.91
            }),
            "test",
        )
        .expect("valid profile");

        assert_eq!(
            profile.capability_tags,
            vec!["research", "source_synthesis"]
        );
        assert_eq!(profile.depth_hint.as_deref(), Some("deep"));
        assert!(profile.deliverables.iter().any(|value| value == "pdf"));
        assert!(profile.long_running);
    }

    #[test]
    fn execution_profile_deep_button_override_keeps_semantic_deliverables() {
        let semantic = ChatExecutionProfile::from_classifier_value(
            &serde_json::json!({
                "capability_tags": ["artifact"],
                "depth_hint": "standard",
                "deliverables": ["pdf", "document"],
                "confidence": 0.9
            }),
            "semantic_classifier",
        );
        let profile = ChatExecutionProfile::deep_research_override_with(semantic);

        assert!(profile
            .capability_tags
            .iter()
            .any(|value| value == "research"));
        assert_eq!(profile.depth_hint.as_deref(), Some("deep"));
        assert!(profile.long_running);
        assert!(profile.deliverables.iter().any(|value| value == "pdf"));
        assert_eq!(profile.source, "ui_override+semantic_classifier");
    }

    #[test]
    fn chat_stream_request_uses_execution_profile_over_legacy_boolean() {
        let request = ChatRequest {
            message: "Compare the market and make a report".to_string(),
            channel: "web".to_string(),
            conversation_id: None,
            deep_research: false,
            execution_profile: Some(serde_json::json!({
                "capability_tags": ["research"],
                "depth_hint": "deep",
                "deliverables": ["answer", "document"],
                "long_running": true,
                "confidence": 1.0,
                "source": "ui_override"
            })),
            execution_mode: None,
            attachments_present: false,
            attachments: Vec::new(),
            arkorbit_context: None,
            browser_profile_context: None,
            accepted_suggestion_id: None,
            sentinel_proposal_id: None,
        };
        let run_request = chat_stream_run_request_from_persisted_user_message(
            request,
            PersistedChatStreamUserMessage {
                conversation_id: "conv-1".to_string(),
                message_id: "msg-1".to_string(),
            },
            None,
            None,
        );
        let profile = run_request.execution_profile.expect("profile");

        assert!(!run_request.deep_research);
        assert!(profile
            .capability_tags
            .iter()
            .any(|value| value == "research"));
        assert!(profile.long_running);
        assert!(profile.is_ui_override());
    }

    #[test]
    fn execution_profile_accepts_non_research_capabilities_without_route_logic() {
        let profile = ChatExecutionProfile::from_classifier_value(
            &serde_json::json!({
                "capability_tags": ["app_builder"],
                "depth_hint": "deep",
                "deliverables": ["app"],
                "confidence": 0.94
            }),
            "test",
        )
        .expect("valid profile");

        assert_eq!(profile.capability_tags, vec!["app_builder".to_string()]);
        assert!(!profile.deliverables.iter().any(|value| value == "pdf"));
    }

    #[test]
    fn execution_profile_keeps_answer_only_work_as_chat_profile() {
        let profile = ChatExecutionProfile::from_classifier_value(
            &serde_json::json!({
                "capability_tags": ["conversation"],
                "depth_hint": "standard",
                "deliverables": ["answer"],
                "confidence": 0.94
            }),
            "test",
        )
        .expect("valid profile");

        assert_eq!(profile.capability_tags, vec!["conversation".to_string()]);
        assert_eq!(profile.deliverables, vec!["answer".to_string()]);
    }

    #[test]
    fn execution_profile_rejects_low_confidence_classifier_output() {
        let profile = ChatExecutionProfile::from_classifier_value(
            &serde_json::json!({
                "capability_tags": ["research"],
                "depth_hint": "deep",
                "deliverables": ["pdf"],
                "confidence": 0.42
            }),
            "test",
        );

        assert!(profile.is_none());
    }

    #[test]
    fn execution_profile_json_extractor_handles_fences_nested_braces_and_trailing_text() {
        let value = extract_chat_execution_profile_value(
            "```json\n{\"capability_tags\":[\"research\"],\"meta\":{\"brace\":\"}\"},\"confidence\":0.91}\n```\ntrailing",
        )
        .expect("profile JSON should be extracted");

        assert_eq!(value["capability_tags"][0], "research");
        assert_eq!(value["meta"]["brace"], "}");
    }

    #[test]
    fn first_stream_activity_counts_public_reasoning_completion_and_tools() {
        assert_eq!(
            stream_event_first_activity_source(&crate::core::StreamEvent::ReasoningDelta {
                phase: "model".to_string(),
                content_delta: "Preparing the app files.".to_string(),
                done: false,
            }),
            Some("reasoning_delta")
        );
        assert_eq!(
            stream_event_first_activity_source(&crate::core::StreamEvent::ReasoningDelta {
                phase: "reasoning_summary".to_string(),
                content_delta: "Checking files before deploying.".to_string(),
                done: false,
            }),
            Some("reasoning_delta")
        );
        assert_eq!(
            stream_event_first_activity_source(&crate::core::StreamEvent::ReasoningDelta {
                phase: "model".to_string(),
                content_delta: String::new(),
                done: true,
            }),
            Some("reasoning_delta")
        );
        assert_eq!(
            stream_event_first_activity_source(&crate::core::StreamEvent::ToolStart {
                name: "app_deploy".to_string(),
                payload: None,
            }),
            Some("tool_start")
        );
        assert_eq!(
            stream_event_first_activity_source(&crate::core::StreamEvent::Thinking(
                "Waiting on model response.".to_string(),
            )),
            None
        );
    }

    #[test]
    fn reasoning_delta_sse_streams_live_model_reasoning_text() {
        let (event, _) = normalize_stream_event_for_sse(
            crate::core::StreamEvent::ReasoningDelta {
                phase: "model".to_string(),
                content_delta: "Draft package manifest and server source here.".to_string(),
                done: false,
            },
            "",
        );

        let (event_name, payload) = event.expect("live model reasoning should be visible");

        assert_eq!(event_name, "reasoning_delta");
        assert_eq!(payload["title"], "Thinking");
        assert_eq!(
            payload["content_delta"],
            "Draft package manifest and server source here."
        );
        assert_eq!(payload["done"], false);
    }

    #[test]
    fn reasoning_delta_sse_preserves_token_whitespace() {
        let (event, _) = normalize_stream_event_for_sse(
            crate::core::StreamEvent::ReasoningDelta {
                phase: "model".to_string(),
                content_delta: " user".to_string(),
                done: false,
            },
            "",
        );

        let (event_name, payload) = event.expect("reasoning delta should be emitted");
        assert_eq!(event_name, "reasoning_delta");
        assert_eq!(payload["content_delta"], serde_json::json!(" user"));
        assert_eq!(payload["content"], serde_json::json!(" user"));
        assert_eq!(payload["detail"], serde_json::json!("user"));
    }

    #[test]
    fn reasoning_summary_delta_sse_streams_safe_summary_text() {
        let (event, _) = normalize_stream_event_for_sse(
            crate::core::StreamEvent::ReasoningDelta {
                phase: "reasoning_summary".to_string(),
                content_delta: "Checking staged files and deploy readiness.".to_string(),
                done: false,
            },
            "",
        );

        let (event_name, payload) = event.expect("summary reasoning should be visible");

        assert_eq!(event_name, "reasoning_delta");
        assert_eq!(payload["title"], "Reasoning summary");
        assert_eq!(
            payload["content_delta"],
            "Checking staged files and deploy readiness."
        );
        assert_eq!(payload["done"], false);
    }

    #[test]
    fn first_token_metric_requires_observed_real_token() {
        assert_eq!(
            chat_stream_first_token_metric_value(0),
            serde_json::Value::Null
        );
        assert_eq!(
            chat_stream_first_token_metric_value(428),
            serde_json::json!(428)
        );
    }
}
