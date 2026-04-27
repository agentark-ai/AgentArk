use super::*;

// ==================== Conversation Endpoints ====================
pub(super) async fn list_conversations(
    State(state): State<AppState>,
    axum::extract::Query(params): axum::extract::Query<std::collections::HashMap<String, String>>,
) -> Response {
    const SIDEBAR_CONVERSATION_PAGE_SIZE: u64 = 20;
    const STARRED_CONVERSATION_LIMIT: u64 = 3;
    let storage = { state.agent.read().await.storage.clone() };
    let project_id = params.get("project_id").map(|s| s.as_str());
    let sidebar_mode = params
        .get("sidebar")
        .map(|value| {
            matches!(
                value.trim().to_ascii_lowercase().as_str(),
                "1" | "true" | "yes"
            )
        })
        .unwrap_or(false);
    let requested_limit = params
        .get("limit")
        .and_then(|s| s.parse().ok())
        .unwrap_or(20u64);
    let limit = if sidebar_mode {
        requested_limit.min(SIDEBAR_CONVERSATION_PAGE_SIZE)
    } else {
        requested_limit
    };
    let offset = params
        .get("offset")
        .and_then(|s| s.parse().ok())
        .unwrap_or(0u64);
    let internal_channels = ["arkpulse", "sentinel", "system"];
    let total = storage
        .count_conversations(
            project_id,
            &internal_channels,
            if sidebar_mode { Some(false) } else { None },
        )
        .await
        .unwrap_or(0);
    match storage
        .list_conversations(
            limit,
            offset,
            project_id,
            &internal_channels,
            if sidebar_mode { Some(false) } else { None },
        )
        .await
    {
        Ok(convs) => {
            let list: Vec<serde_json::Value> = convs
                .iter()
                .map(|c| {
                    serde_json::json!({
                        "id": c.id, "title": c.title, "channel": c.channel,
                        "project_id": c.project_id, "created_at": c.created_at,
                        "updated_at": c.updated_at, "message_count": c.message_count,
                        "archived": c.archived, "starred": c.starred,
                    })
                })
                .collect();
            let starred_conversations = if sidebar_mode {
                match storage
                    .list_conversations(
                        STARRED_CONVERSATION_LIMIT,
                        0,
                        project_id,
                        &internal_channels,
                        Some(true),
                    )
                    .await
                {
                    Ok(rows) => rows
                        .into_iter()
                        .map(|c| {
                            serde_json::json!({
                                "id": c.id, "title": c.title, "channel": c.channel,
                                "project_id": c.project_id, "created_at": c.created_at,
                                "updated_at": c.updated_at, "message_count": c.message_count,
                                "archived": c.archived, "starred": c.starred,
                            })
                        })
                        .collect::<Vec<_>>(),
                    Err(_) => Vec::new(),
                }
            } else {
                Vec::new()
            };
            (
                StatusCode::OK,
                Json(serde_json::json!({
                    "conversations": list,
                    "starred_conversations": starred_conversations,
                    "total": total,
                    "limit": limit,
                    "offset": offset
                })),
            )
                .into_response()
        }
        Err(e) => (
            StatusCode::INTERNAL_SERVER_ERROR,
            Json(ErrorResponse {
                error: e.to_string(),
            }),
        )
            .into_response(),
    }
}

pub(super) async fn create_conversation_endpoint(
    State(state): State<AppState>,
    Json(request): Json<serde_json::Value>,
) -> Response {
    let title = request
        .get("title")
        .and_then(|v| v.as_str())
        .unwrap_or("New Chat")
        .to_string();
    let channel = request
        .get("channel")
        .and_then(|v| v.as_str())
        .unwrap_or("web")
        .to_string();
    let project_id = request
        .get("project_id")
        .and_then(|v| v.as_str())
        .map(|s| s.to_string());
    let now = chrono::Utc::now().to_rfc3339();
    let id = uuid::Uuid::new_v4().to_string();

    let conv = crate::storage::entities::conversation::Model {
        id: id.clone(),
        title,
        channel,
        project_id,
        created_at: now.clone(),
        updated_at: now,
        message_count: 0,
        archived: false,
        starred: false,
    };

    let storage = { state.agent.read().await.storage.clone() };
    match storage.create_conversation(&conv).await {
        Ok(_) => (
            StatusCode::OK,
            Json(serde_json::json!({"id": id, "status": "ok"})),
        )
            .into_response(),
        Err(e) => (
            StatusCode::INTERNAL_SERVER_ERROR,
            Json(ErrorResponse {
                error: e.to_string(),
            }),
        )
            .into_response(),
    }
}

pub(super) async fn get_conversation_endpoint(
    State(state): State<AppState>,
    Path(id): Path<String>,
) -> Response {
    let agent = { state.agent.read().await.clone() };
    match agent.storage.get_conversation(&id).await {
        Ok(Some(conv)) => {
            let workspace = agent.load_conversation_workspace_snapshot(&id).await;
            (
                StatusCode::OK,
                Json(serde_json::json!({
                    "id": conv.id, "title": conv.title, "channel": conv.channel,
                    "project_id": conv.project_id, "created_at": conv.created_at,
                    "updated_at": conv.updated_at, "message_count": conv.message_count,
                    "archived": conv.archived, "starred": conv.starred,
                    "workspace": workspace,
                })),
            )
                .into_response()
        }
        Ok(None) => (
            StatusCode::NOT_FOUND,
            Json(ErrorResponse {
                error: "Conversation not found".to_string(),
            }),
        )
            .into_response(),
        Err(e) => (
            StatusCode::INTERNAL_SERVER_ERROR,
            Json(ErrorResponse {
                error: e.to_string(),
            }),
        )
            .into_response(),
    }
}

pub(super) async fn update_conversation_endpoint(
    State(state): State<AppState>,
    Path(id): Path<String>,
    Json(body): Json<serde_json::Value>,
) -> Response {
    let title = body.get("title").and_then(|v| v.as_str());
    let starred = body.get("starred").and_then(|v| v.as_bool());
    if title.is_none() && starred.is_none() {
        return (
            StatusCode::BAD_REQUEST,
            Json(ErrorResponse {
                error: "Missing title or starred".to_string(),
            }),
        )
            .into_response();
    }
    let storage = { state.agent.read().await.storage.clone() };
    match storage
        .update_conversation(&id, title, None, starred)
        .await
    {
        Ok(updated) => (
            StatusCode::OK,
            Json(serde_json::json!({
                "status": "ok",
                "title": updated.title,
                "starred": updated.starred
            })),
        )
            .into_response(),
        Err(e) => {
            let message = e.to_string();
            let status = if message.eq_ignore_ascii_case("Conversation not found") {
                StatusCode::NOT_FOUND
            } else if message
                .to_ascii_lowercase()
                .contains("max 3 starred chats allowed")
            {
                StatusCode::BAD_REQUEST
            } else {
                StatusCode::INTERNAL_SERVER_ERROR
            };
            (status, Json(ErrorResponse { error: message })).into_response()
        }
    }
}

pub(super) async fn delete_conversation_endpoint(
    State(state): State<AppState>,
    Path(id): Path<String>,
) -> Response {
    let storage = { state.agent.read().await.storage.clone() };
    match storage.delete_conversation(&id).await {
        Ok(_) => (StatusCode::OK, Json(serde_json::json!({"status": "ok"}))).into_response(),
        Err(e) => (
            StatusCode::INTERNAL_SERVER_ERROR,
            Json(ErrorResponse {
                error: e.to_string(),
            }),
        )
            .into_response(),
    }
}

pub(super) fn clarification_choices_from_operational_payload(
    payload: Option<&str>,
) -> Vec<crate::core::ClarificationChoice> {
    let Some(raw_payload) = payload else {
        return Vec::new();
    };
    let Ok(parsed) = serde_json::from_str::<serde_json::Value>(raw_payload) else {
        return Vec::new();
    };
    let should_clarify = parsed
        .get("should_clarify")
        .or_else(|| parsed.get("clarification_needed"))
        .or_else(|| parsed.get("ambiguous"))
        .and_then(|value| value.as_bool())
        .unwrap_or(false);
    if !should_clarify {
        return Vec::new();
    }
    serde_json::from_value::<Vec<crate::core::ClarificationChoice>>(
        parsed
            .get("choices")
            .cloned()
            .unwrap_or(serde_json::Value::Null),
    )
    .unwrap_or_default()
}

pub(super) async fn get_conversation_messages(
    State(state): State<AppState>,
    Path(id): Path<String>,
    axum::extract::Query(params): axum::extract::Query<std::collections::HashMap<String, String>>,
) -> Response {
    let limit = params
        .get("limit")
        .and_then(|s| s.parse().ok())
        .unwrap_or(100u64);
    let offset = params
        .get("offset")
        .and_then(|s| s.parse().ok())
        .unwrap_or(0u64);
    let (encrypted_storage, storage) = {
        let agent = state.agent.read().await;
        (agent.encrypted_storage.clone(), agent.storage.clone())
    };
    match encrypted_storage
        .get_messages_decrypted(&id, limit, offset)
        .await
    {
        Ok(msgs) => {
            let unavailable_message =
                "Older message unavailable after a past password/key change.".to_string();
            let trace_ids = msgs
                .iter()
                .filter_map(|m| m.trace_id.as_deref())
                .map(str::trim)
                .filter(|trace_id| !trace_id.is_empty())
                .map(str::to_string)
                .collect::<Vec<_>>();
            let trace_metrics = match storage
                .get_execution_trace_message_metrics_by_ids(&trace_ids)
                .await
            {
                Ok(metrics) => metrics,
                Err(error) => {
                    tracing::warn!(
                        "Failed to load execution trace message metrics for conversation '{}': {}",
                        id,
                        error
                    );
                    std::collections::HashMap::new()
                }
            };
            let clarification_choices_by_trace_id = match storage
                .list_operational_logs_for_trace_ids_by_event(
                    &trace_ids,
                    "action_selection",
                    (trace_ids.len().saturating_mul(4).max(32)) as u64,
                )
                .await
            {
                Ok(rows) => {
                    let mut by_trace_id = std::collections::HashMap::new();
                    for row in rows {
                        let Some(trace_id) = row
                            .trace_id
                            .as_deref()
                            .map(str::trim)
                            .filter(|trace_id| !trace_id.is_empty())
                        else {
                            continue;
                        };
                        if by_trace_id.contains_key(trace_id) {
                            continue;
                        }
                        let choices =
                            clarification_choices_from_operational_payload(row.payload.as_deref());
                        if choices.is_empty() {
                            continue;
                        }
                        by_trace_id.insert(trace_id.to_string(), choices);
                    }
                    by_trace_id
                }
                Err(error) => {
                    tracing::warn!(
                        "Failed to load clarification choices for conversation '{}': {}",
                        id,
                        error
                    );
                    std::collections::HashMap::new()
                }
            };
            let list: Vec<serde_json::Value> = msgs
                .iter()
                .map(|m| {
                    let metrics = if m.role == "assistant" {
                        m.trace_id
                            .as_ref()
                            .and_then(|trace_id| trace_metrics.get(trace_id.trim()))
                    } else {
                        None
                    };
                    let input_tokens = metrics.map(|row| row.input_tokens).unwrap_or(0);
                    let output_tokens = metrics.map(|row| row.output_tokens).unwrap_or(0);
                    let total_tokens = metrics.map(|row| row.total_tokens).unwrap_or(0);
                    let duration_ms = metrics.and_then(|row| row.duration_ms);
                    let time_to_first_token_ms =
                        metrics.and_then(|row| row.time_to_first_token_ms);
                    let mut payload = serde_json::json!({
                        "id": m.id, "role": m.role, "content": if m.content == crate::storage::ENCRYPTED_STORAGE_UNAVAILABLE { unavailable_message.clone() } else { m.content.clone() },
                        "timestamp": m.timestamp, "model_used": m.model_used, "trace_id": m.trace_id,
                        "input_tokens": input_tokens,
                        "output_tokens": output_tokens,
                        "total_tokens": total_tokens,
                        "duration_ms": duration_ms,
                        "time_to_first_token_ms": time_to_first_token_ms,
                    });
                    if let Some(choices) = m
                        .trace_id
                        .as_deref()
                        .map(str::trim)
                        .and_then(|trace_id| clarification_choices_by_trace_id.get(trace_id))
                    {
                        payload["choices"] =
                            serde_json::to_value(choices).unwrap_or(serde_json::Value::Null);
                    }
                    payload
                })
                .collect();
            (StatusCode::OK, Json(serde_json::json!({"messages": list}))).into_response()
        }
        Err(e) => (
            StatusCode::INTERNAL_SERVER_ERROR,
            Json(ErrorResponse {
                error: e.to_string(),
            }),
        )
            .into_response(),
    }
}

// ==================== Project Endpoints ====================

pub(super) async fn list_projects_endpoint(State(state): State<AppState>) -> Response {
    let agent = state.agent.read().await;
    match agent.storage.list_projects().await {
        Ok(projects) => {
            let list: Vec<serde_json::Value> = projects
                .iter()
                .map(|p| {
                    serde_json::json!({
                        "id": p.id, "name": p.name, "description": p.description,
                        "system_prompt": p.system_prompt, "personality": p.personality,
                        "tools_filter": p.tools_filter, "active": p.active,
                        "created_at": p.created_at, "updated_at": p.updated_at,
                    })
                })
                .collect();
            (StatusCode::OK, Json(serde_json::json!({"projects": list}))).into_response()
        }
        Err(e) => (
            StatusCode::INTERNAL_SERVER_ERROR,
            Json(ErrorResponse {
                error: e.to_string(),
            }),
        )
            .into_response(),
    }
}

pub(super) async fn create_project_endpoint(
    State(state): State<AppState>,
    Json(request): Json<serde_json::Value>,
) -> Response {
    let name = match request.get("name").and_then(|v| v.as_str()) {
        Some(n) if !n.trim().is_empty() => n.trim().to_string(),
        _ => {
            return (
                StatusCode::BAD_REQUEST,
                Json(ErrorResponse {
                    error: "Name required".to_string(),
                }),
            )
                .into_response();
        }
    };
    let now = chrono::Utc::now().to_rfc3339();
    let id = uuid::Uuid::new_v4().to_string();
    let proj = crate::storage::entities::project::Model {
        id: id.clone(),
        name,
        description: request
            .get("description")
            .and_then(|v| v.as_str())
            .unwrap_or("")
            .to_string(),
        system_prompt: request
            .get("system_prompt")
            .and_then(|v| v.as_str())
            .map(|s| s.to_string()),
        personality: request
            .get("personality")
            .and_then(|v| v.as_str())
            .map(|s| s.to_string()),
        tools_filter: request
            .get("tools_filter")
            .and_then(|v| v.as_str())
            .map(|s| s.to_string()),
        active: true,
        created_at: now.clone(),
        updated_at: now,
    };
    let agent = state.agent.read().await;
    match agent.storage.create_project(&proj).await {
        Ok(_) => (
            StatusCode::OK,
            Json(serde_json::json!({"id": id, "status": "ok"})),
        )
            .into_response(),
        Err(e) => (
            StatusCode::INTERNAL_SERVER_ERROR,
            Json(ErrorResponse {
                error: e.to_string(),
            }),
        )
            .into_response(),
    }
}

pub(super) async fn get_project_endpoint(
    State(state): State<AppState>,
    Path(id): Path<String>,
) -> Response {
    let agent = state.agent.read().await;
    match agent.storage.get_project(&id).await {
        Ok(Some(p)) => (
            StatusCode::OK,
            Json(serde_json::json!({
                "id": p.id, "name": p.name, "description": p.description,
                "system_prompt": p.system_prompt, "personality": p.personality,
                "tools_filter": p.tools_filter, "active": p.active,
            })),
        )
            .into_response(),
        Ok(None) => (
            StatusCode::NOT_FOUND,
            Json(ErrorResponse {
                error: "Project not found".to_string(),
            }),
        )
            .into_response(),
        Err(e) => (
            StatusCode::INTERNAL_SERVER_ERROR,
            Json(ErrorResponse {
                error: e.to_string(),
            }),
        )
            .into_response(),
    }
}

pub(super) async fn update_project_endpoint(
    State(state): State<AppState>,
    Path(id): Path<String>,
    Json(request): Json<serde_json::Value>,
) -> Response {
    let agent = state.agent.read().await;
    let existing = match agent.storage.get_project(&id).await {
        Ok(Some(p)) => p,
        Ok(None) => {
            return (
                StatusCode::NOT_FOUND,
                Json(ErrorResponse {
                    error: "Not found".to_string(),
                }),
            )
                .into_response();
        }
        Err(e) => {
            return (
                StatusCode::INTERNAL_SERVER_ERROR,
                Json(ErrorResponse {
                    error: e.to_string(),
                }),
            )
                .into_response();
        }
    };
    let updated = crate::storage::entities::project::Model {
        id: id.clone(),
        name: request
            .get("name")
            .and_then(|v| v.as_str())
            .unwrap_or(&existing.name)
            .to_string(),
        description: request
            .get("description")
            .and_then(|v| v.as_str())
            .unwrap_or(&existing.description)
            .to_string(),
        system_prompt: request
            .get("system_prompt")
            .and_then(|v| v.as_str())
            .map(|s| s.to_string())
            .or(existing.system_prompt),
        personality: request
            .get("personality")
            .and_then(|v| v.as_str())
            .map(|s| s.to_string())
            .or(existing.personality),
        tools_filter: request
            .get("tools_filter")
            .and_then(|v| v.as_str())
            .map(|s| s.to_string())
            .or(existing.tools_filter),
        active: request
            .get("active")
            .and_then(|v| v.as_bool())
            .unwrap_or(existing.active),
        created_at: existing.created_at,
        updated_at: chrono::Utc::now().to_rfc3339(),
    };
    match agent.storage.update_project(&updated).await {
        Ok(_) => (StatusCode::OK, Json(serde_json::json!({"status": "ok"}))).into_response(),
        Err(e) => (
            StatusCode::INTERNAL_SERVER_ERROR,
            Json(ErrorResponse {
                error: e.to_string(),
            }),
        )
            .into_response(),
    }
}

pub(super) async fn delete_project_endpoint(
    State(state): State<AppState>,
    Path(id): Path<String>,
) -> Response {
    let agent = state.agent.read().await;
    match agent.storage.delete_project(&id).await {
        Ok(_) => (StatusCode::OK, Json(serde_json::json!({"status": "ok"}))).into_response(),
        Err(e) => (
            StatusCode::INTERNAL_SERVER_ERROR,
            Json(ErrorResponse {
                error: e.to_string(),
            }),
        )
            .into_response(),
    }
}
