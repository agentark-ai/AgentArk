use super::*;

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
    crate::core::RequestExecutionHints {
        caller_principal: caller.cloned(),
        execution_surface: surface,
        direct_user_intent,
        routing: None,
        intent_plan: None,
        secret_offered: None,
        attachments,
        saved_user_facts_context: None,
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

pub(super) async fn resolve_chat_request_conversation_id(
    state: &AppState,
    channel: &str,
    conversation_id: Option<&str>,
    project_id: Option<&str>,
    message: &str,
) -> std::result::Result<String, Response> {
    let agent = state.agent.read().await;
    match agent
        .ensure_conversation_id_for_request(channel, conversation_id, project_id, message)
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

    let resolved_conversation_id = match resolve_chat_request_conversation_id(
        &state,
        &request.channel,
        request.conversation_id.as_deref(),
        request.project_id.as_deref(),
        &request.message,
    )
    .await
    {
        Ok(conversation_id) => conversation_id,
        Err(response) => return response,
    };
    request.conversation_id = Some(resolved_conversation_id);

    tracing::info!(
        "HTTP /chat request: channel={}, msg={}chars, conv_id={:?}, project={:?}",
        request.channel,
        request.message.len(),
        request.conversation_id.as_deref().unwrap_or("-"),
        request.project_id.as_deref().unwrap_or("-"),
    );

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
    let project_id = request.project_id.clone();
    if request.attachments_present && request.attachments.is_empty() {
        tracing::debug!(
            "Chat request indicated attachments were present, but no structured attachment hints were supplied"
        );
    }
    let attachments = request.attachments.clone();
    let caller_principal = maybe_caller.as_ref().map(|Extension(value)| value.clone());
    let agent_for_chat = state.agent.clone();

    let result = {
        let worker = tokio::spawn(async move {
            let agent_snapshot = Agent::snapshot(&agent_for_chat).await;
            agent_snapshot
                .process_message_with_meta_and_hints(
                    &message,
                    &channel,
                    conversation_id.as_deref(),
                    project_id.as_deref(),
                    build_request_execution_hints(
                        caller_principal.as_ref(),
                        crate::actions::ActionExecutionSurface::Chat,
                        true,
                        attachments,
                    ),
                )
                .await
        });
        match worker.await {
            Ok(result) => result,
            Err(error) => Err(anyhow::anyhow!("Chat worker failed: {}", error)),
        }
    };

    match result {
        Ok(processed) => {
            spawn_autonomy_analysis_tick(state.agent.clone(), "chat_event");
            (
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
                .into_response()
        }
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
            if let Some(obj) = value.as_object() {
                if let Some(title) = obj
                    .get("matched_app")
                    .and_then(|v| v.get("title"))
                    .and_then(|v| v.as_str())
                    .map(str::trim)
                    .filter(|v| !v.is_empty())
                {
                    return format!("Matched app and loaded metadata for {}.", title);
                }
                let keys = obj.keys().take(4).cloned().collect::<Vec<_>>().join(", ");
                if !keys.is_empty() {
                    return format!("Returned structured data: {}.", keys);
                }
            } else if let Some(items) = value.as_array() {
                return format!(
                    "Returned list with {} item{}.",
                    items.len(),
                    if items.len() == 1 { "" } else { "s" }
                );
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
        return "Returned verbose tool output.".to_string();
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
            project_id,
        } => (
            Some((
                "task_started",
                serde_json::json!({
                    "task_id": task_id,
                    "description": description,
                    "status": "in_progress",
                    "work_type": work_type,
                    "conversation_id": conversation_id,
                    "project_id": project_id,
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
        crate::core::StreamEvent::ToolStart { name, payload } => {
            let payload_json = if let Some(payload) = payload {
                if let Some(obj) = payload.as_object() {
                    let mut merged = serde_json::Map::new();
                    merged.insert("name".to_string(), serde_json::json!(name));
                    for (k, v) in obj {
                        merged.insert(k.clone(), v.clone());
                    }
                    serde_json::Value::Object(merged)
                } else {
                    serde_json::json!({ "name": name, "payload": payload })
                }
            } else {
                serde_json::json!({ "name": name })
            };
            (Some(("tool_start", payload_json)), String::new())
        }
        crate::core::StreamEvent::ToolResult { name, content } => {
            let summarized = summarize_stream_tool_activity_content(&content);
            let trimmed = content.trim();
            let mut payload = serde_json::Map::new();
            payload.insert("name".to_string(), serde_json::json!(name));
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
            let payload_json = if let Some(payload) = payload {
                if let Some(obj) = payload.as_object() {
                    let mut merged = serde_json::Map::new();
                    merged.insert("name".to_string(), serde_json::json!(name));
                    merged.insert("content".to_string(), serde_json::json!(content));
                    for (k, v) in obj {
                        merged.insert(k.clone(), v.clone());
                    }
                    serde_json::Value::Object(merged)
                } else {
                    serde_json::json!({ "name": name, "content": content, "payload": payload })
                }
            } else {
                serde_json::json!({ "name": name, "content": content })
            };
            (Some(("tool_progress", payload_json)), String::new())
        }
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

pub(super) fn run_event_to_sse_event(run_event: crate::core::RunEvent) -> Event {
    let mut payload = match run_event.kind.as_str() {
        "thinking" => {
            let mut merged = run_event.payload.as_object().cloned().unwrap_or_default();
            if !merged.contains_key("step_type") {
                merged.insert("step_type".to_string(), serde_json::json!("thinking"));
            }
            if !merged.contains_key("title") {
                merged.insert("title".to_string(), serde_json::json!("Thinking"));
            }
            if !merged.contains_key("__streamKey") {
                merged.insert("__streamKey".to_string(), serde_json::json!("public-thinking"));
            }
            merged
        }
        "tool_start" => {
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
        "tool_progress" => {
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
            if let Some(inner) = run_event
                .payload
                .get("payload")
                .and_then(|value| value.as_object())
            {
                let mut merged = serde_json::Map::new();
                merged.insert("name".to_string(), serde_json::json!(name));
                merged.insert("content".to_string(), serde_json::json!(content));
                for (key, value) in inner {
                    merged.insert(key.clone(), value.clone());
                }
                merged
            } else {
                serde_json::json!({
                    "name": name,
                    "content": content,
                })
                .as_object()
                .cloned()
                .unwrap_or_default()
            }
        }
        "tool_result" => {
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
                            if matches!(key.as_str(), "name" | "content" | "raw_content" | "result")
                            {
                                continue;
                            }
                            merged.insert(key.clone(), value.clone());
                        }
                    } else {
                        merged.insert("raw_content".to_string(), serde_json::json!(raw_content));
                    }
                } else {
                    merged.insert("raw_content".to_string(), serde_json::json!(raw_content));
                }
            }
            merged
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
    if let Some(stage) = run_event.stage {
        payload.insert("stage".to_string(), serde_json::json!(stage));
    }
    Event::default()
        .event(run_event.kind)
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

pub(super) fn deep_research_notification_details(
    topic: &str,
    terminal_status: &crate::core::TaskStatus,
    run_status: Option<&str>,
) -> Option<(&'static str, String, &'static str)> {
    match terminal_status {
        crate::core::TaskStatus::Completed => Some((
            "Deep research completed",
            format!(
                "{} is ready. Open Chat to review the completed report.",
                topic
            ),
            "info",
        )),
        crate::core::TaskStatus::Failed { .. } => {
            let normalized_run_status = run_status
                .map(|value| value.trim().to_ascii_lowercase())
                .unwrap_or_default();
            if normalized_run_status == "platform_failed" {
                Some((
                    "Deep research failed",
                    format!(
                        "{} could not finish because AgentArk hit an execution problem. Open Chat to review the error.",
                        topic
                    ),
                    "error",
                ))
            } else {
                Some((
                    "Deep research failed",
                    format!(
                        "{} could not finish cleanly. Open Chat to review the issue.",
                        topic
                    ),
                    "warning",
                ))
            }
        }
        _ => None,
    }
}

#[derive(Clone)]
pub(super) struct StreamedChatTask {
    pub(super) task_id: String,
    pub(super) description: String,
    pub(super) work_type: String,
    pub(super) user_message_already_recorded: bool,
}

#[allow(clippy::large_enum_variant)]
#[derive(Clone)]
pub(super) enum ChatStreamTaskMode {
    CreateIfNeeded,
    Existing(Box<StreamedChatTask>),
}

#[derive(Clone)]
pub(super) struct ChatStreamRunRequest {
    pub(super) message: String,
    pub(super) channel: String,
    pub(super) conversation_id: Option<String>,
    pub(super) project_id: Option<String>,
    pub(super) deep_research: bool,
    pub(super) attachments: Vec<crate::core::ChatAttachmentHint>,
    pub(super) caller_principal: Option<crate::actions::ActionCallerPrincipal>,
    pub(super) task_mode: ChatStreamTaskMode,
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
    let created_at = storage
        .load_execution_run(run_id)
        .await
        .ok()
        .flatten()
        .map(|run| run.created_at)
        .unwrap_or_else(|| now.clone());
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
        conversation_id: conversation_id
            .map(str::trim)
            .filter(|value| !value.is_empty())
            .map(str::to_string),
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
    let project_id = request.project_id.clone();
    let deep_research = request.deep_research;
    let attachments = request.attachments.clone();
    let caller_principal = request.caller_principal.clone();
    let task_mode = request.task_mode.clone();
    let app_state = state.clone();
    let stream_request_id = uuid::Uuid::new_v4().to_string();
    let stream_started_at = Instant::now();
    let time_to_first_token_ms = Arc::new(AtomicU64::new(0));

    crate::spawn_logged!("src/channels/http.rs:14787", async move {
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
        let tracked_task = match task_mode {
            ChatStreamTaskMode::CreateIfNeeded => None,
            ChatStreamTaskMode::Existing(task) => {
                let task_started = crate::core::StreamEvent::ChatTaskStarted {
                    task_id: task.task_id.clone(),
                    description: task.description.clone(),
                    work_type: task.work_type.clone(),
                    conversation_id: conversation_id.clone(),
                    project_id: project_id.clone(),
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
            .unwrap_or("")
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
            let live_runs = live_runs.clone();
            let stream_request_id = stream_request_id.clone();
            let channel = channel.clone();
            crate::spawn_logged!("src/channels/http.rs:14840", async move {
                let mut last_thinking_detail = String::new();
                while let Some(ev) = stream_rx.recv().await {
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
                                user_message_already_recorded: false,
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
            crate::spawn_logged!("src/channels/http.rs:14884", async move {
                let mut last_step_count = 0;
                let start = std::time::Instant::now();
                loop {
                    tokio::time::sleep(std::time::Duration::from_millis(500)).await;
                    if start.elapsed().as_secs() > 1800 {
                        break;
                    }
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

        let tracked_task_snapshot = tracked_task_ref.read().await.clone();
        let user_message_already_recorded = tracked_task_snapshot
            .as_ref()
            .map(|task| task.user_message_already_recorded)
            .unwrap_or(false);
        let mut process_handle = {
            let agent_ref = agent_ref.clone();
            let message = message.clone();
            let channel = channel.clone();
            let conversation_id = conversation_id.clone();
            let project_id = project_id.clone();
            let trace_ref = trace_ref.clone();
            let attachments = attachments.clone();
            let caller_principal = caller_principal.clone();
            tokio::spawn(async move {
                let agent_snapshot = Agent::snapshot(&agent_ref).await;
                if user_message_already_recorded {
                    agent_snapshot
                        .process_message_stream_resume_with_meta_and_hints(
                            &message,
                            &channel,
                            conversation_id.as_deref(),
                            project_id.as_deref(),
                            trace_ref,
                            stream_tx,
                            build_request_execution_hints(
                                caller_principal.as_ref(),
                                crate::actions::ActionExecutionSurface::Chat,
                                true,
                                attachments.clone(),
                            ),
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
                            build_request_execution_hints(
                                caller_principal.as_ref(),
                                crate::actions::ActionExecutionSurface::Chat,
                                true,
                                attachments,
                            ),
                        )
                        .await
                }
            })
        };

        let mut was_cancelled = false;
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
                            "project_id": project_id.clone(),
                        }),
                        crate::core::RunEventPriority::High,
                    )
                    .await;
                    if deep_research {
                        let topic = processed
                            .conversation_title
                            .as_deref()
                            .filter(|value| !value.trim().is_empty())
                            .unwrap_or(task.description.as_str());
                        let notification = deep_research_notification_details(
                            topic,
                            &terminal_status,
                            processed.run_status.as_deref(),
                        );
                        if let Some((title, body, level)) = notification {
                            let agent_snapshot = Agent::snapshot(&agent_ref).await;
                            agent_snapshot
                                .emit_notification(title, &body, level, "deep_research")
                                .await;
                        }
                    }
                }

                if time_to_first_token_ms.load(Ordering::Relaxed) == 0
                    && !processed.response.trim().is_empty()
                {
                    let synthetic_first_token_ms = stream_started_at
                        .elapsed()
                        .as_millis()
                        .min(u64::MAX as u128) as u64;
                    time_to_first_token_ms.store(synthetic_first_token_ms.max(1), Ordering::Relaxed);
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
                let trace_total_tokens =
                    trace_metric_snapshot["total_tokens"].as_i64().unwrap_or(0);
                let effective_total_tokens = if trace_total_tokens > 0 {
                    trace_total_tokens
                } else {
                    processed.total_tokens
                };
                let effective_run_id = stream_request_id.clone();
                upsert_chat_stream_execution_run(
                    &run_storage,
                    &effective_run_id,
                    resolved_conversation_id.as_deref(),
                    &channel,
                    &message,
                    chat_stream_execution_status(processed.run_status.as_deref()),
                    Some(truncate_stream_task_text(&processed.response, 1200)),
                    None,
                    processed.trace_id.clone(),
                    processed.degradation.clone(),
                    processed.attempted_models.clone(),
                )
                .await;
                let mut content = serde_json::json!({
                    "content": processed.response,
                    "conversation_id": resolved_conversation_id,
                    "run_id": effective_run_id,
                    "run_status": processed.run_status,
                    "trace_id": processed.trace_id,
                    "input_tokens": trace_metric_snapshot["input_tokens"],
                    "output_tokens": trace_metric_snapshot["output_tokens"],
                    "total_tokens": effective_total_tokens,
                    "duration_ms": trace_metric_snapshot["duration_ms"],
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
                        "time_to_first_token_ms": content["time_to_first_token_ms"],
                        "degradation": content["degradation"],
                        "attempted_models": content["attempted_models"],
                        "user_outcome": content["user_outcome"],
                    }),
                    crate::core::RunEventPriority::Critical,
                )
                .await;
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
                            "project_id": project_id.clone(),
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
                if let Some(task) = tracked_task.as_ref() {
                    let error_text = error.to_string();
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
                            "project_id": project_id.clone(),
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
    });

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
    pub(super) project_id: Option<String>,
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
    project_id: Option<&str>,
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

    if let Some(pid) = project_id.map(str::trim).filter(|value| !value.is_empty()) {
        let current = arguments
            .get("project_id")
            .and_then(|value| value.as_str())
            .unwrap_or("")
            .trim();
        if current != pid {
            arguments.insert("project_id".to_string(), serde_json::json!(pid.to_string()));
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

    let project_id = arguments
        .get("project_id")
        .and_then(|value| value.as_str())
        .map(str::trim)
        .filter(|value| !value.is_empty())
        .map(ToOwned::to_owned);
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
        project_id,
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
    if let Some(response) = validate_chat_message_size(&request.message) {
        return response;
    }

    let resolved_conversation_id = match resolve_chat_request_conversation_id(
        &state,
        &request.channel,
        request.conversation_id.as_deref(),
        request.project_id.as_deref(),
        &request.message,
    )
    .await
    {
        Ok(conversation_id) => conversation_id,
        Err(response) => return response,
    };
    request.conversation_id = Some(resolved_conversation_id);

    tracing::info!(
        "HTTP /chat/stream request: channel={}, msg={}chars, conv_id={:?}",
        request.channel,
        request.message.len(),
        request.conversation_id.as_deref().unwrap_or("-"),
    );

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

    spawn_chat_stream_response(
        state,
        ChatStreamRunRequest {
            message: request.message,
            channel: request.channel,
            conversation_id: request.conversation_id,
            project_id: request.project_id,
            deep_research: request.deep_research,
            attachments: request.attachments,
            caller_principal: maybe_caller.as_ref().map(|Extension(value)| value.clone()),
            task_mode: ChatStreamTaskMode::CreateIfNeeded,
        },
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
    let project_id = request.get("project_id").and_then(|v| v.as_str());
    let agent = Agent::snapshot(&state.agent).await;
    if let Some(pid) = project_id {
        agent
            .clear_conversation_for_project(channel, Some(pid))
            .await;
    } else {
        agent.clear_conversation_history(channel).await;
    }
    (
        StatusCode::OK,
        Json(serde_json::json!({ "status": "cleared" })),
    )
        .into_response()
}
