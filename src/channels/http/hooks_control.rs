use super::*;

const HOOKS_STORAGE_KEY: &str = "hooks_v1";

// - Hook endpoints -

/// Request to create a new hook
#[derive(Debug, Deserialize)]
pub(super) struct AddHookRequest {
    pub name: String,
    pub trigger: String,
    pub hook_type: String,
    pub url: Option<String>,
    #[serde(default)]
    pub action_name: Option<String>,
}

pub(super) async fn persist_hooks(agent: &Agent) -> std::result::Result<(), String> {
    let bytes = serde_json::to_vec(&agent.hooks.snapshot()).map_err(|e| e.to_string())?;
    agent
        .storage
        .set(HOOKS_STORAGE_KEY, &bytes)
        .await
        .map_err(|e| e.to_string())
}

/// List all registered hooks
pub(super) async fn list_hooks(State(state): State<AppState>) -> Response {
    let agent = state.agent.read().await;
    let hooks_list: Vec<hooks::Hook> = agent.hooks.list_hooks().to_vec();
    (StatusCode::OK, Json(hooks_list)).into_response()
}

/// List recent hook run reports
pub(super) async fn list_hook_runs(
    State(state): State<AppState>,
    Query(params): Query<HashMap<String, String>>,
) -> Response {
    let limit = params
        .get("limit")
        .and_then(|v| v.parse::<usize>().ok())
        .unwrap_or(100)
        .clamp(1, 500);
    let agent = state.agent.read().await;
    let runs = agent.hooks.list_runs(limit).await;
    (StatusCode::OK, Json(runs)).into_response()
}

/// Add a new hook
pub(super) async fn add_hook(
    State(state): State<AppState>,
    Json(request): Json<AddHookRequest>,
) -> Response {
    let trigger: hooks::HookTrigger = match serde_json::from_value(serde_json::Value::String(
        request.trigger.clone(),
    )) {
        Ok(t) => t,
        Err(_) => {
            return (
                StatusCode::BAD_REQUEST,
                Json(ErrorResponse {
                    error: format!(
                        "Invalid trigger '{}'. Valid values: pre_message, post_message, pre_action, post_action, on_consolidate, on_error",
                        request.trigger
                    ),
                }),
            )
                .into_response();
        }
    };

    let hook = hooks::Hook {
        id: uuid::Uuid::new_v4().to_string(),
        name: request.name,
        action_name: request
            .action_name
            .as_ref()
            .map(|v| v.trim().to_string())
            .filter(|v| !v.is_empty()),
        trigger,
        hook_type: request.hook_type,
        url: request.url,
        enabled: true,
    };

    let id = hook.id.clone();
    let mut agent = state.agent.write().await;
    agent.hooks.add_hook(hook);
    if let Err(e) = persist_hooks(&agent).await {
        agent.hooks.remove_hook(&id);
        return (
            StatusCode::INTERNAL_SERVER_ERROR,
            Json(ErrorResponse {
                error: format!("Failed to persist hook: {}", e),
            }),
        )
            .into_response();
    }

    (StatusCode::CREATED, Json(serde_json::json!({ "id": id }))).into_response()
}

/// Remove a hook by ID
pub(super) async fn remove_hook(State(state): State<AppState>, Path(id): Path<String>) -> Response {
    let mut agent = state.agent.write().await;
    let before = agent.hooks.snapshot();
    agent.hooks.remove_hook(&id);
    if let Err(e) = persist_hooks(&agent).await {
        agent.hooks = hooks::HookManager::from_hooks(before);
        return (
            StatusCode::INTERNAL_SERVER_ERROR,
            Json(ErrorResponse {
                error: format!("Failed to persist hook removal: {}", e),
            }),
        )
            .into_response();
    }
    (
        StatusCode::OK,
        Json(serde_json::json!({ "status": "removed" })),
    )
        .into_response()
}
