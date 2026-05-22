use super::*;

pub(super) async fn sync_agentark_knowledge(State(state): State<AppState>) -> Response {
    let agent = state.agent.read().await;
    match agent.sync_agentark_knowledge().await {
        Ok(synced) => (
            StatusCode::OK,
            Json(serde_json::json!({ "synced": synced })),
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

/// Execute code in an isolated sandbox
pub(super) async fn execute_code(
    State(state): State<AppState>,
    maybe_caller: Option<Extension<crate::actions::ActionCallerPrincipal>>,
    Json(request): Json<CodeExecuteRequest>,
) -> Response {
    let arguments = serde_json::json!({
        "language": request.language,
        "code": request.code,
        "env": request.env,
        "files": request.files,
        "network_access": request.network_access,
    });

    let result = {
        let agent_guard = state.agent.read().await;
        let caller = maybe_caller.as_ref().map(|Extension(value)| value);
        agent_guard
            .runtime
            .execute_action_with_context(
                "code_execute",
                &arguments,
                &build_direct_action_auth_context(
                    caller,
                    crate::actions::ActionExecutionSurface::Api,
                    true,
                ),
            )
            .await
    };

    match result {
        Ok(output_json) => {
            // The action returns a JSON string; parse it for a clean response
            match serde_json::from_str::<serde_json::Value>(&output_json) {
                Ok(parsed) => {
                    let files = parsed["files"].as_array().map(|arr| {
                        arr.iter()
                            .filter_map(|v| v.as_str().map(String::from))
                            .collect()
                    });
                    let resp = CodeExecuteResponse {
                        output: parsed["output"].as_str().unwrap_or("").to_string(),
                        exit_code: parsed["exit_code"].as_i64().unwrap_or(-1),
                        error: parsed["error"].as_str().map(|s| s.to_string()),
                        files,
                    };
                    (StatusCode::OK, Json(resp)).into_response()
                }
                Err(_) => {
                    // Fallback: return raw output
                    let resp = CodeExecuteResponse {
                        output: output_json,
                        exit_code: 0,
                        error: None,
                        files: None,
                    };
                    (StatusCode::OK, Json(resp)).into_response()
                }
            }
        }
        Err(e) => {
            let resp = CodeExecuteResponse {
                output: String::new(),
                exit_code: -1,
                error: Some(e.to_string()),
                files: None,
            };
            (StatusCode::INTERNAL_SERVER_ERROR, Json(resp)).into_response()
        }
    }
}

// ==================== MCP (Model Context Protocol) Endpoints ====================

pub(super) async fn mcp_handler(
    State(state): State<AppState>,
    maybe_caller: Option<Extension<crate::actions::ActionCallerPrincipal>>,
    Json(request): Json<crate::mcp::McpRequest>,
) -> Response {
    let mcp = crate::mcp::McpServer::new();

    // Handle tool calls that need agent access
    if request.method == "tools/call" {
        let tool_name = request
            .params
            .get("name")
            .and_then(|v| v.as_str())
            .unwrap_or("");
        let args = request
            .params
            .get("arguments")
            .cloned()
            .unwrap_or(serde_json::json!({}));

        let result = match tool_name {
            "chat" => {
                let message = args.get("message").and_then(|v| v.as_str()).unwrap_or("");
                let channel = args
                    .get("channel")
                    .and_then(|v| v.as_str())
                    .unwrap_or("mcp");
                let conversation_id = args.get("conversation_id").and_then(|v| v.as_str());
                let agent = Agent::snapshot(&state.agent).await;
                let caller = maybe_caller.as_ref().map(|Extension(value)| value);
                match agent
                    .process_message_with_meta_and_hints(
                        message,
                        channel,
                        conversation_id,
                        None,
                        build_request_execution_hints(
                            caller,
                            crate::actions::ActionExecutionSurface::Api,
                            true,
                        ),
                    )
                    .await
                {
                    Ok(processed) => serde_json::json!({
                        "content": [{ "type": "text", "text": processed.response }],
                        "conversation_id": processed.conversation_id,
                        "conversation_title": processed.conversation_title,
                        "run_status": processed.run_status,
                        "degradation": processed.degradation,
                        "attempted_models": processed.attempted_models,
                        "user_outcome": processed.user_outcome,
                    }),
                    Err(e) => {
                        let response = "I hit a framework-level problem before the MCP request could finish cleanly. Please retry.".to_string();
                        let degradation = vec![crate::core::DegradationNote {
                            kind: "platform".to_string(),
                            summary: "framework error".to_string(),
                            detail: Some(e.to_string()),
                        }];
                        let user_outcome = crate::core::ExecutionSupervisor::default()
                            .build_service_outage_outcome(
                                &response,
                                "framework_error",
                                &degradation,
                                &[],
                            );
                        serde_json::json!({
                            "content": [{ "type": "text", "text": response }],
                            "isError": true,
                            "run_status": "platform_failed",
                            "degradation": degradation,
                            "attempted_models": user_outcome.attempted_models,
                            "user_outcome": user_outcome,
                        })
                    }
                }
            }
            "memory_search" => {
                let query = args.get("query").and_then(|v| v.as_str()).unwrap_or("");
                let limit = args.get("limit").and_then(|v| v.as_u64()).unwrap_or(5) as usize;
                let agent = state.agent.read().await;
                match agent
                    .storage
                    .search_experience_items(
                        query,
                        &["constraint", "personal_fact", "lesson", "procedure"],
                        None,
                        None,
                        limit as u64,
                    )
                    .await
                {
                    Ok(hits) => {
                        let results: Vec<serde_json::Value> = hits
                            .iter()
                            .map(|hit| {
                                serde_json::json!({
                                    "kind": hit.item.kind,
                                    "content": hit.item.content,
                                    "score": hit.score,
                                    "updated_at": hit.item.updated_at,
                                })
                            })
                            .collect();
                        serde_json::json!({ "content": [{ "type": "text", "text": serde_json::to_string_pretty(&results).unwrap_or_default() }] })
                    }
                    Err(e) => {
                        serde_json::json!({ "content": [{ "type": "text", "text": format!("Error: {}", e) }], "isError": true })
                    }
                }
            }
            "document_search" => {
                let query = args.get("query").and_then(|v| v.as_str()).unwrap_or("");
                let limit = args.get("limit").and_then(|v| v.as_u64()).unwrap_or(5) as usize;
                let agent = state.agent.read().await;
                match agent.search_documents(query, limit, None).await {
                    Ok(results) => {
                        let items: Vec<serde_json::Value> = results
                            .iter()
                            .map(|hit| {
                                serde_json::json!({
                                    "document_id": &hit.document_id,
                                    "filename": &hit.filename,
                                    "content_type": &hit.content_type,
                                    "chunk_index": hit.chunk_index,
                                    "content": &hit.content,
                                    "score": hit.score,
                                    "lexical_score": hit.lexical_score,
                                    "dense_score": hit.dense_score,
                                    "match_reason": &hit.match_reason
                                })
                            })
                            .collect();
                        serde_json::json!({ "content": [{ "type": "text", "text": serde_json::to_string_pretty(&items).unwrap_or_default() }] })
                    }
                    Err(e) => {
                        serde_json::json!({ "content": [{ "type": "text", "text": format!("Error: {}", e) }], "isError": true })
                    }
                }
            }
            "list_actions" => {
                let agent = state.agent.read().await;
                match agent.runtime.list_enabled_actions().await {
                    Ok(actions) => {
                        let items: Vec<serde_json::Value> = actions.iter().map(|a| {
                            serde_json::json!({ "name": a.name, "description": a.description })
                        }).collect();
                        serde_json::json!({ "content": [{ "type": "text", "text": serde_json::to_string_pretty(&items).unwrap_or_default() }] })
                    }
                    Err(e) => {
                        serde_json::json!({ "content": [{ "type": "text", "text": format!("Error: {}", e) }], "isError": true })
                    }
                }
            }
            "execute_action" => {
                let action = args.get("action").and_then(|v| v.as_str()).unwrap_or("");
                let action_args = args
                    .get("arguments")
                    .cloned()
                    .unwrap_or(serde_json::json!({}));
                let agent = state.agent.read().await;
                let caller = maybe_caller.as_ref().map(|Extension(value)| value);
                match agent
                    .runtime
                    .execute_action_with_context(
                        action,
                        &action_args,
                        &build_direct_action_auth_context(
                            caller,
                            crate::actions::ActionExecutionSurface::Api,
                            true,
                        ),
                    )
                    .await
                {
                    Ok(result) => {
                        serde_json::json!({ "content": [{ "type": "text", "text": result }] })
                    }
                    Err(e) => {
                        serde_json::json!({ "content": [{ "type": "text", "text": format!("Error: {}", e) }], "isError": true })
                    }
                }
            }
            _ => {
                serde_json::json!({ "content": [{ "type": "text", "text": format!("Unknown tool: {}", tool_name) }], "isError": true })
            }
        };

        return (
            StatusCode::OK,
            Json(crate::mcp::McpResponse {
                jsonrpc: "2.0".to_string(),
                id: request.id.clone(),
                result: Some(result),
                error: None,
            }),
        )
            .into_response();
    }

    // Handle non-tool-call methods
    let response = mcp.handle_request(&request);
    (StatusCode::OK, Json(response)).into_response()
}

pub(super) async fn mcp_list_tools() -> Json<serde_json::Value> {
    let mcp = crate::mcp::McpServer::new();
    Json(
        serde_json::json!({ "tools": mcp.handle_request(&crate::mcp::McpRequest {
        jsonrpc: "2.0".to_string(),
        id: Some(serde_json::json!(1)),
        method: "tools/list".to_string(),
        params: serde_json::json!({}),
    }).result }),
    )
}

#[derive(Debug, Deserialize)]
pub(super) struct McpListQuery {
    #[serde(default)]
    include_details: bool,
}

/// List MCP servers (client-side connections)
pub(super) async fn list_mcp_servers(
    State(state): State<AppState>,
    Query(query): Query<McpListQuery>,
) -> Response {
    let agent = state.agent.read().await;
    let registry = agent.mcp.read().await;
    match registry.list_servers(query.include_details).await {
        Ok(servers) => (
            StatusCode::OK,
            Json(serde_json::json!({
                "servers": servers,
            })),
        )
            .into_response(),
        Err(error) => (
            StatusCode::INTERNAL_SERVER_ERROR,
            Json(ErrorResponse {
                error: error.to_string(),
            }),
        )
            .into_response(),
    }
}

/// Get a specific MCP server
pub(super) async fn get_mcp_server(
    State(state): State<AppState>,
    Path(id): Path<String>,
) -> Response {
    let agent = state.agent.read().await;
    let registry = agent.mcp.read().await;
    match registry.get_server(&id, true).await {
        Ok(Some(server)) => (StatusCode::OK, Json(server)).into_response(),
        Ok(None) => (
            StatusCode::NOT_FOUND,
            Json(ErrorResponse {
                error: "MCP server not found".to_string(),
            }),
        )
            .into_response(),
        Err(error) => (
            StatusCode::INTERNAL_SERVER_ERROR,
            Json(ErrorResponse {
                error: error.to_string(),
            }),
        )
            .into_response(),
    }
}

/// Create a new MCP server
pub(super) async fn create_mcp_server(
    State(state): State<AppState>,
    Json(request): Json<McpServerRequest>,
) -> Response {
    let mut agent = state.agent.write().await;
    let server_id = request
        .id
        .clone()
        .unwrap_or_else(|| uuid::Uuid::new_v4().to_string());
    if agent.config.mcp.servers.iter().any(|s| s.id == server_id) {
        return (
            StatusCode::CONFLICT,
            Json(ErrorResponse {
                error: "MCP server ID already exists".to_string(),
            }),
        )
            .into_response();
    }

    let existing = None;
    let (config, auth_update) =
        match build_mcp_config(&agent.storage, &request, &server_id, existing).await {
            Ok(v) => v,
            Err(e) => {
                return (
                    StatusCode::BAD_REQUEST,
                    Json(ErrorResponse {
                        error: e.to_string(),
                    }),
                )
                    .into_response();
            }
        };

    agent.config.mcp.servers.push(config);

    if let Err(e) = save_mcp_secrets(&mut agent, &server_id, auth_update) {
        return (
            StatusCode::INTERNAL_SERVER_ERROR,
            Json(ErrorResponse {
                error: format!("Failed to save MCP secrets: {}", e),
            }),
        )
            .into_response();
    }

    if let Err(e) = agent.config.save(&agent.config_dir, Some(&agent.data_dir)) {
        return (
            StatusCode::INTERNAL_SERVER_ERROR,
            Json(ErrorResponse {
                error: format!("Failed to save config: {}", e),
            }),
        )
            .into_response();
    }

    drop(agent);
    schedule_mcp_registry_sync(state.agent.clone());

    let agent = state.agent.read().await;
    let registry = agent.mcp.read().await;
    let server = registry.get_server(&server_id, true).await.unwrap_or(None);
    (
        StatusCode::CREATED,
        Json(serde_json::json!({
            "server": server,
            "sync_queued": true,
        })),
    )
        .into_response()
}

/// Update an MCP server
pub(super) async fn update_mcp_server(
    State(state): State<AppState>,
    Path(id): Path<String>,
    Json(request): Json<McpServerRequest>,
) -> Response {
    let mut agent = state.agent.write().await;
    let idx = match agent.config.mcp.servers.iter().position(|s| s.id == id) {
        Some(i) => i,
        None => {
            return (
                StatusCode::NOT_FOUND,
                Json(ErrorResponse {
                    error: "MCP server not found".to_string(),
                }),
            )
                .into_response();
        }
    };

    let existing = agent.config.mcp.servers.get(idx).cloned();
    let (config, auth_update) =
        match build_mcp_config(&agent.storage, &request, &id, existing.as_ref()).await {
            Ok(v) => v,
            Err(e) => {
                return (
                    StatusCode::BAD_REQUEST,
                    Json(ErrorResponse {
                        error: e.to_string(),
                    }),
                )
                    .into_response();
            }
        };

    agent.config.mcp.servers[idx] = config;

    if let Err(e) = save_mcp_secrets(&mut agent, &id, auth_update) {
        return (
            StatusCode::INTERNAL_SERVER_ERROR,
            Json(ErrorResponse {
                error: format!("Failed to save MCP secrets: {}", e),
            }),
        )
            .into_response();
    }

    if let Err(e) = agent.config.save(&agent.config_dir, Some(&agent.data_dir)) {
        return (
            StatusCode::INTERNAL_SERVER_ERROR,
            Json(ErrorResponse {
                error: format!("Failed to save config: {}", e),
            }),
        )
            .into_response();
    }

    drop(agent);
    schedule_mcp_registry_sync(state.agent.clone());

    let agent = state.agent.read().await;
    let registry = agent.mcp.read().await;
    let server = registry.get_server(&id, true).await.unwrap_or(None);
    (
        StatusCode::OK,
        Json(serde_json::json!({ "server": server, "sync_queued": true })),
    )
        .into_response()
}

/// Delete an MCP server
pub(super) async fn delete_mcp_server(
    State(state): State<AppState>,
    Path(id): Path<String>,
) -> Response {
    let id = id.trim().to_string();
    if id.is_empty() {
        return (
            StatusCode::BAD_REQUEST,
            Json(ErrorResponse {
                error: "MCP server id is required".to_string(),
            }),
        )
            .into_response();
    }
    let mut agent = state.agent.write().await;
    let before = agent.config.mcp.servers.len();
    agent.config.mcp.servers.retain(|s| s.id != id);
    let existed = agent.config.mcp.servers.len() != before;

    if let Err(e) = clear_mcp_secrets(&mut agent, &id) {
        return (
            StatusCode::INTERNAL_SERVER_ERROR,
            Json(ErrorResponse {
                error: format!("Failed to remove MCP secrets: {}", e),
            }),
        )
            .into_response();
    }

    if existed {
        if let Err(e) = agent.config.save(&agent.config_dir, Some(&agent.data_dir)) {
            return (
                StatusCode::INTERNAL_SERVER_ERROR,
                Json(ErrorResponse {
                    error: format!("Failed to save config: {}", e),
                }),
            )
                .into_response();
        }
    }

    drop(agent);
    schedule_mcp_registry_sync(state.agent.clone());

    (
        StatusCode::OK,
        Json(serde_json::json!({
            "status": if existed { "deleted" } else { "already_absent" },
            "sync_queued": true
        })),
    )
        .into_response()
}

/// Refresh MCP server tools/resources
pub(super) async fn refresh_mcp_server(
    State(state): State<AppState>,
    Path(id): Path<String>,
) -> Response {
    {
        let agent = state.agent.read().await;
        if agent.config.mcp.servers.iter().all(|s| s.id != id) {
            return (
                StatusCode::NOT_FOUND,
                Json(ErrorResponse {
                    error: "MCP server not found".to_string(),
                }),
            )
                .into_response();
        }
    }

    schedule_mcp_server_refresh(state.agent.clone(), id.clone());

    let agent = state.agent.read().await;
    let registry = agent.mcp.read().await;
    let server = registry.get_server(&id, true).await.unwrap_or(None);
    (
        StatusCode::OK,
        Json(serde_json::json!({
            "status": "refresh_queued",
            "server": server,
            "server_id": id,
        })),
    )
        .into_response()
}

pub(super) async fn build_mcp_config(
    storage: &crate::storage::Storage,
    request: &McpServerRequest,
    server_id: &str,
    existing: Option<&crate::core::config::McpServerConfig>,
) -> Result<(
    crate::core::config::McpServerConfig,
    Option<McpSecretUpdate>,
)> {
    if request.name.trim().is_empty() {
        return Err(anyhow::anyhow!("MCP server name is required"));
    }

    let transport = match &request.transport {
        McpTransportRequest::Http { url } => {
            let parsed = url::Url::parse(url).map_err(|_| anyhow::anyhow!("Invalid MCP URL"))?;
            if parsed.scheme() != "http" && parsed.scheme() != "https" {
                return Err(anyhow::anyhow!("MCP URL must be http or https"));
            }
            crate::core::config::McpTransportConfig::Http { url: url.clone() }
        }
        McpTransportRequest::Stdio {
            command,
            args,
            working_dir,
            env,
        } => {
            if command.trim().is_empty() {
                return Err(anyhow::anyhow!("MCP stdio command is required"));
            }
            let env_keys = env
                .as_ref()
                .map(|map| {
                    let mut keys = map
                        .keys()
                        .map(|key| key.trim().to_string())
                        .filter(|key| !key.is_empty())
                        .collect::<Vec<_>>();
                    keys.sort();
                    keys.dedup();
                    keys
                })
                .unwrap_or_else(|| match existing.map(|cfg| &cfg.transport) {
                    Some(crate::core::config::McpTransportConfig::Stdio { env_keys, .. }) => {
                        env_keys.clone()
                    }
                    _ => Vec::new(),
                });
            crate::core::config::McpTransportConfig::Stdio {
                command: command.clone(),
                args: args.clone(),
                working_dir: working_dir.clone(),
                env_keys,
            }
        }
    };

    let auth_profile_id = request
        .auth_profile_id
        .as_deref()
        .map(str::trim)
        .filter(|value| !value.is_empty())
        .map(|value| value.to_string())
        .or_else(|| existing.and_then(|cfg| cfg.auth_profile_id.clone()));
    if let Some(profile_id) = auth_profile_id.as_deref() {
        if !matches!(
            &transport,
            crate::core::config::McpTransportConfig::Http { .. }
        ) {
            return Err(anyhow::anyhow!(
                "HTTP auth profiles can only be attached to HTTP MCP transports."
            ));
        }
        if crate::core::auth_profiles::AuthProfileControlPlane::get(storage, profile_id)
            .await?
            .is_none()
        {
            return Err(anyhow::anyhow!(
                "Auth profile '{}' was not found.",
                profile_id
            ));
        }
    }

    let (auth_config, auth_update) = parse_mcp_auth(
        request.auth.as_ref(),
        existing.and_then(|e| e.auth.as_ref()),
    );
    let timeout_secs = request
        .timeout_secs
        .or(existing.map(|e| e.timeout_secs))
        .unwrap_or(15);
    let max_response_bytes = request
        .max_response_bytes
        .or(existing.map(|e| e.max_response_bytes))
        .unwrap_or(1024 * 1024);

    let config = crate::core::config::McpServerConfig {
        id: server_id.to_string(),
        name: request.name.trim().to_string(),
        description: request.description.clone(),
        transport,
        enabled: request.enabled,
        resources_enabled: request.resources_enabled,
        auth: auth_config,
        auth_profile_id,
        tool_allowlist: clean_allowlist(&request.tool_allowlist),
        tool_blocklist: request
            .tool_blocklist
            .iter()
            .map(|s| s.trim().to_string())
            .filter(|s| !s.is_empty())
            .collect(),
        resource_allowlist: clean_allowlist(&request.resource_allowlist),
        timeout_secs,
        max_response_bytes,
    };

    Ok((
        config,
        Some(McpSecretUpdate {
            auth: auth_update,
            env: match &request.transport {
                McpTransportRequest::Stdio { env, .. } => env.clone(),
                McpTransportRequest::Http { .. } => Some(std::collections::HashMap::new()),
            },
        }),
    ))
}

pub(super) fn clean_allowlist(list: &[String]) -> Vec<String> {
    list.iter()
        .map(|s| s.trim().to_string())
        .filter(|s| !s.is_empty())
        .collect()
}

#[derive(Debug)]
pub(super) struct McpAuthUpdate {
    clear: bool,
    token: Option<String>,
    username: Option<String>,
    password: Option<String>,
}

#[derive(Debug)]
pub(super) struct McpSecretUpdate {
    auth: Option<McpAuthUpdate>,
    env: Option<std::collections::HashMap<String, String>>,
}

pub(super) fn parse_mcp_auth(
    request: Option<&McpAuthRequest>,
    existing: Option<&crate::core::config::McpAuthConfig>,
) -> (
    Option<crate::core::config::McpAuthConfig>,
    Option<McpAuthUpdate>,
) {
    let Some(req) = request else {
        return (existing.cloned(), None);
    };

    match req {
        McpAuthRequest::None { .. } => (
            None,
            Some(McpAuthUpdate {
                clear: true,
                token: None,
                username: None,
                password: None,
            }),
        ),
        McpAuthRequest::Bearer {
            header,
            token,
            clear,
        } => {
            let header = header.clone().unwrap_or("Authorization".to_string());
            (
                Some(crate::core::config::McpAuthConfig::Bearer { header }),
                Some(McpAuthUpdate {
                    clear: *clear,
                    token: token.clone(),
                    username: None,
                    password: None,
                }),
            )
        }
        McpAuthRequest::Basic {
            username,
            password,
            clear,
        } => (
            Some(crate::core::config::McpAuthConfig::Basic),
            Some(McpAuthUpdate {
                clear: *clear,
                token: None,
                username: username.clone(),
                password: password.clone(),
            }),
        ),
        McpAuthRequest::Header { name, value, clear } => (
            Some(crate::core::config::McpAuthConfig::Header { name: name.clone() }),
            Some(McpAuthUpdate {
                clear: *clear,
                token: value.clone(),
                username: None,
                password: None,
            }),
        ),
        McpAuthRequest::Query { name, value, clear } => (
            Some(crate::core::config::McpAuthConfig::Query { name: name.clone() }),
            Some(McpAuthUpdate {
                clear: *clear,
                token: value.clone(),
                username: None,
                password: None,
            }),
        ),
    }
}

pub(super) fn should_update_secret(value: &Option<String>) -> bool {
    value
        .as_ref()
        .is_some_and(|v| !v.is_empty() && v != "[ENCRYPTED]")
}

pub(super) fn apply_auth_update(
    secrets: &mut crate::core::config::Secrets,
    server_id: &str,
    update: &McpAuthUpdate,
) {
    if update.clear {
        secrets.mcp_auth.remove(server_id);
        return;
    }

    let mut entry = secrets.mcp_auth.get(server_id).cloned().unwrap_or_default();
    let mut changed = false;

    if should_update_secret(&update.token) {
        entry.token = update.token.clone();
        changed = true;
    }
    if should_update_secret(&update.username) {
        entry.username = update.username.clone();
        changed = true;
    }
    if should_update_secret(&update.password) {
        entry.password = update.password.clone();
        changed = true;
    }

    if changed {
        secrets.mcp_auth.insert(server_id.to_string(), entry);
    }
}

pub(super) fn save_mcp_secrets(
    agent: &mut Agent,
    server_id: &str,
    update: Option<McpSecretUpdate>,
) -> Result<()> {
    if update.is_none() {
        return Ok(());
    }
    let manager = crate::core::config::SecureConfigManager::new_with_data_dir(
        &agent.config_dir,
        Some(&agent.data_dir),
    )?;
    if let Some(update) = update.as_ref() {
        manager.update_secrets(|secrets| {
            if let Some(auth) = update.auth.as_ref() {
                apply_auth_update(secrets, server_id, auth);
            }
            if let Some(env) = update.env.as_ref() {
                let cleaned = env
                    .iter()
                    .map(|(key, value)| (key.trim().to_string(), value.trim().to_string()))
                    .filter(|(key, value)| !key.is_empty() && !value.is_empty())
                    .collect::<std::collections::HashMap<_, _>>();
                if cleaned.is_empty() {
                    secrets.mcp_env.remove(server_id);
                } else {
                    secrets.mcp_env.insert(server_id.to_string(), cleaned);
                }
            }
            Ok(())
        })?;
    }
    Ok(())
}

pub(super) fn load_mcp_secrets(agent: &Agent) -> Result<crate::core::config::Secrets> {
    let manager = crate::core::config::SecureConfigManager::new_with_data_dir(
        &agent.config_dir,
        Some(&agent.data_dir),
    )?;
    manager.load_secrets()
}

pub(super) fn clear_mcp_secrets(agent: &mut Agent, server_id: &str) -> Result<()> {
    let manager = crate::core::config::SecureConfigManager::new_with_data_dir(
        &agent.config_dir,
        Some(&agent.data_dir),
    )?;
    manager.update_secrets(|secrets| {
        secrets.mcp_auth.remove(server_id);
        secrets.mcp_env.remove(server_id);
        Ok(())
    })?;
    Ok(())
}

pub(super) async fn sync_mcp_registry(agent: &Agent, secrets: &crate::core::config::Secrets) {
    let Agent {
        mcp,
        safety,
        runtime,
        config,
        ..
    } = agent;
    let mut registry = mcp.write().await;
    match registry
        .sync_from_config(config, secrets, runtime, safety)
        .await
    {
        Ok(()) => {
            drop(registry);
            agent
                .refresh_action_catalog_index("mcp_registry_sync")
                .await;
        }
        Err(error) => {
            tracing::warn!("MCP registry sync failed: {}", error);
        }
    }
}

pub(super) fn mcp_sync_timeout_for_config(
    config: &crate::core::config::AgentConfig,
) -> std::time::Duration {
    let timeout_secs = config
        .mcp
        .servers
        .iter()
        .filter(|server| server.enabled)
        .map(|server| server.timeout_secs.max(5))
        .max()
        .unwrap_or(20)
        .saturating_add(15)
        .max(20);
    std::time::Duration::from_secs(timeout_secs)
}

pub(super) fn schedule_mcp_registry_sync(agent_ref: SharedAgent) {
    crate::spawn_logged!("src/channels/http.rs:36420", async move {
        let agent = agent_ref.read().await;
        let secrets = match load_mcp_secrets(&agent) {
            Ok(s) => s,
            Err(e) => {
                tracing::warn!("MCP registry sync skipped: failed to load secrets: {}", e);
                return;
            }
        };
        let timeout = mcp_sync_timeout_for_config(&agent.config);
        match tokio::time::timeout(timeout, sync_mcp_registry(&agent, &secrets)).await {
            Ok(_) => {}
            Err(_) => tracing::warn!("MCP registry sync timed out after {:?}", timeout),
        }
    });
}

pub(super) fn schedule_mcp_server_refresh(agent_ref: SharedAgent, id: String) {
    crate::spawn_logged!("src/channels/http.rs:36438", async move {
        let agent = agent_ref.read().await;
        if agent.config.mcp.servers.iter().all(|s| s.id != id) {
            return;
        }
        let secrets = match load_mcp_secrets(&agent) {
            Ok(s) => s,
            Err(e) => {
                tracing::warn!(
                    "MCP server refresh skipped for {}: failed to load secrets: {}",
                    id,
                    e
                );
                return;
            }
        };
        let Agent {
            mcp,
            runtime,
            safety,
            config,
            ..
        } = &*agent;
        let refresh_future = async {
            let mut registry = mcp.write().await;
            registry
                .refresh_server(&id, config, &secrets, runtime, safety)
                .await
        };
        let timeout = mcp_sync_timeout_for_config(config);
        match tokio::time::timeout(timeout, refresh_future).await {
            Ok(Ok(())) => {
                tracing::info!("MCP server refresh succeeded for {}", id);
                agent
                    .refresh_action_catalog_index("mcp_server_refresh")
                    .await;
            }
            Ok(Err(e)) => tracing::warn!("MCP server refresh failed for {}: {}", id, e),
            Err(_) => tracing::warn!(
                "MCP server refresh timed out after {:?} for {}",
                timeout,
                id
            ),
        }
    });
}

pub(super) fn schedule_enabled_mcp_server_resumes(agent_ref: SharedAgent) {
    crate::spawn_logged!("src/channels/http.rs:36481", async move {
        tokio::time::sleep(std::time::Duration::from_secs(5)).await;
        let enabled_ids = {
            let agent = agent_ref.read().await;
            agent
                .config
                .mcp
                .servers
                .iter()
                .filter(|server| server.enabled)
                .map(|server| server.id.clone())
                .collect::<Vec<_>>()
        };
        if enabled_ids.is_empty() {
            return;
        }
        tracing::info!(
            "Resuming {} enabled MCP server(s) in the background",
            enabled_ids.len()
        );
        let total = enabled_ids.len();
        for (index, id) in enabled_ids.into_iter().enumerate() {
            tracing::info!("Scheduling background MCP resume for {}", id);
            schedule_mcp_server_refresh(agent_ref.clone(), id);
            if index + 1 < total {
                tokio::time::sleep(std::time::Duration::from_millis(750)).await;
            }
        }
    });
}
