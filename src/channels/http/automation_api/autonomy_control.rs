use super::*;

// ==================== Autonomy Endpoints ====================
pub(super) async fn get_autonomy_settings(State(state): State<AppState>) -> Response {
    let agent = state.agent.read().await;
    let settings = load_autonomy_settings(&agent).await;
    (
        StatusCode::OK,
        Json(serde_json::json!({
            "settings": settings,
        })),
    )
        .into_response()
}

pub(super) async fn update_autonomy_settings(
    State(state): State<AppState>,
    Json(request): Json<serde_json::Value>,
) -> Response {
    let agent = state.agent.read().await;
    let mut settings = load_autonomy_settings(&agent).await;

    if let Some(scope) = request.get("context_scope").and_then(|v| v.as_str()) {
        settings.context_scope = ConversationScope::from_storage(Some(scope));
        let _ = agent
            .storage
            .set(
                "conversation_scope_mode",
                settings.context_scope.as_storage_str().as_bytes(),
            )
            .await;
    }
    if let Some(enabled) = request
        .get("voice_briefing_enabled")
        .and_then(|v| v.as_bool())
    {
        settings.voice_briefing_enabled = enabled;
    }
    if let Some(mode) = request.get("autonomy_mode").and_then(|v| v.as_str()) {
        let normalized = mode.trim().to_ascii_lowercase();
        if matches!(normalized.as_str(), "off" | "assist" | "auto") {
            settings.autonomy_mode = normalized;
        }
    }
    if let Some(always_ask) = request
        .get("always_ask_high_risk")
        .and_then(|v| v.as_bool())
    {
        settings.always_ask_high_risk = always_ask;
    }
    if let Some(only_approved) = request
        .get("only_approved_skills")
        .and_then(|v| v.as_bool())
    {
        settings.only_approved_skills = only_approved;
    }
    if request.get("quiet_hours_start").is_some() {
        settings.quiet_hours_start = request
            .get("quiet_hours_start")
            .and_then(|v| v.as_str())
            .map(|s| s.trim().to_string())
            .filter(|s| !s.is_empty());
    }
    if request.get("quiet_hours_end").is_some() {
        settings.quiet_hours_end = request
            .get("quiet_hours_end")
            .and_then(|v| v.as_str())
            .map(|s| s.trim().to_string())
            .filter(|s| !s.is_empty());
    }
    if request.get("daily_run_limit").is_some() {
        settings.daily_run_limit = match request.get("daily_run_limit") {
            Some(value) if value.is_null() => None,
            Some(value) => value.as_u64().map(|v| v.clamp(1, 1000) as u32),
            None => settings.daily_run_limit,
        };
    }
    if let Some(paused) = request.get("agent_paused").and_then(|v| v.as_bool()) {
        settings.agent_paused = paused;
    }
    if let Some(mode) = request.get("pause_mode").and_then(|v| v.as_str()) {
        let normalized = mode.trim().to_ascii_lowercase();
        if matches!(normalized.as_str(), "autonomous_only" | "all_execution") {
            settings.pause_mode = normalized;
        }
    }
    if request.get("arkpulse_auth_failures_threshold").is_some() {
        if let Some(v) = request
            .get("arkpulse_auth_failures_threshold")
            .and_then(|v| v.as_u64())
        {
            settings.arkpulse_auth_failures_threshold = v.clamp(1, 100_000) as u32;
        }
    }
    if request.get("arkpulse_rate_limit_hits_threshold").is_some() {
        if let Some(v) = request
            .get("arkpulse_rate_limit_hits_threshold")
            .and_then(|v| v.as_u64())
        {
            settings.arkpulse_rate_limit_hits_threshold = v.clamp(1, 100_000) as u32;
        }
    }
    if request
        .get("arkpulse_unauthorized_channel_threshold")
        .is_some()
    {
        if let Some(v) = request
            .get("arkpulse_unauthorized_channel_threshold")
            .and_then(|v| v.as_u64())
        {
            settings.arkpulse_unauthorized_channel_threshold = v.clamp(1, 100_000) as u32;
        }
    }
    if request
        .get("arkpulse_combined_security_threshold")
        .is_some()
    {
        if let Some(v) = request
            .get("arkpulse_combined_security_threshold")
            .and_then(|v| v.as_u64())
        {
            settings.arkpulse_combined_security_threshold = v.clamp(1, 100_000) as u32;
        }
    }
    if let Some(active_mode_id) = request.get("active_mode_id").and_then(|v| v.as_str()) {
        settings.active_mode_id = if active_mode_id.trim().is_empty() {
            None
        } else {
            Some(active_mode_id.to_string())
        };
    }
    if let Some(trust_policy) = request.get("trust_policy") {
        if let Ok(parsed) = serde_json::from_value::<TrustPolicy>(trust_policy.clone()) {
            settings.trust_policy = parsed;
        }
    }
    if let Some(modes) = request.get("modes") {
        if let Ok(parsed) = serde_json::from_value::<Vec<AutopilotMode>>(modes.clone()) {
            settings.modes = parsed;
        }
    }
    if let Some(sentinel) = request.get("sentinel") {
        if let Ok(parsed) = serde_json::from_value::<
            crate::core::automation::autonomy::SentinelSettings,
        >(sentinel.clone())
        {
            settings.sentinel = parsed;
        }
    }
    settings.enforce_dependencies();

    match save_autonomy_settings(&agent, &settings).await {
        Ok(_) => (
            StatusCode::OK,
            Json(serde_json::json!({"status":"ok","settings":settings})),
        )
            .into_response(),
        Err(e) => (
            StatusCode::INTERNAL_SERVER_ERROR,
            Json(ErrorResponse { error: e }),
        )
            .into_response(),
    }
}

pub(super) async fn list_autonomy_modes(State(state): State<AppState>) -> Response {
    let agent = state.agent.read().await;
    let settings = load_autonomy_settings(&agent).await;
    (
        StatusCode::OK,
        Json(serde_json::json!({
            "modes": settings.modes,
            "active_mode_id": settings.active_mode_id,
        })),
    )
        .into_response()
}

pub(super) async fn save_autonomy_modes(
    State(state): State<AppState>,
    Json(request): Json<serde_json::Value>,
) -> Response {
    let Some(modes) = request.get("modes") else {
        return (
            StatusCode::BAD_REQUEST,
            Json(ErrorResponse {
                error: "modes is required".to_string(),
            }),
        )
            .into_response();
    };
    let parsed = match serde_json::from_value::<Vec<AutopilotMode>>(modes.clone()) {
        Ok(v) => v,
        Err(e) => {
            return (
                StatusCode::BAD_REQUEST,
                Json(ErrorResponse {
                    error: format!("Invalid modes payload: {}", e),
                }),
            )
                .into_response();
        }
    };
    let agent = state.agent.read().await;
    let mut settings = load_autonomy_settings(&agent).await;
    settings.modes = parsed;
    match save_autonomy_settings(&agent, &settings).await {
        Ok(_) => (
            StatusCode::OK,
            Json(serde_json::json!({"status":"ok","modes":settings.modes})),
        )
            .into_response(),
        Err(e) => (
            StatusCode::INTERNAL_SERVER_ERROR,
            Json(ErrorResponse { error: e }),
        )
            .into_response(),
    }
}

pub(super) async fn activate_autonomy_mode(
    State(state): State<AppState>,
    Path(id): Path<String>,
) -> Response {
    let agent = state.agent.read().await;
    let mut settings = load_autonomy_settings(&agent).await;
    match apply_autopilot_mode(&agent, &mut settings, &id).await {
        Ok(result) => (
            StatusCode::OK,
            Json(serde_json::json!({"status":"ok","result":result})),
        )
            .into_response(),
        Err(e) => (StatusCode::BAD_REQUEST, Json(ErrorResponse { error: e })).into_response(),
    }
}

pub(super) async fn get_context_policy(State(state): State<AppState>) -> Response {
    let agent = state.agent.read().await;
    let settings = load_autonomy_settings(&agent).await;
    (
        StatusCode::OK,
        Json(serde_json::json!({
            "context_scope": settings.context_scope.as_storage_str(),
        })),
    )
        .into_response()
}

pub(super) async fn set_context_policy(
    State(state): State<AppState>,
    Json(request): Json<serde_json::Value>,
) -> Response {
    let scope_raw = request
        .get("context_scope")
        .and_then(|v| v.as_str())
        .unwrap_or("per_channel");
    let scope = ConversationScope::from_storage(Some(scope_raw));
    let agent = state.agent.read().await;
    let mut settings = load_autonomy_settings(&agent).await;
    settings.context_scope = scope;
    let _ = agent
        .storage
        .set("conversation_scope_mode", scope.as_storage_str().as_bytes())
        .await;
    match save_autonomy_settings(&agent, &settings).await {
        Ok(_) => (
            StatusCode::OK,
            Json(serde_json::json!({"status":"ok","context_scope":scope.as_storage_str()})),
        )
            .into_response(),
        Err(e) => (
            StatusCode::INTERNAL_SERVER_ERROR,
            Json(ErrorResponse { error: e }),
        )
            .into_response(),
    }
}

pub(super) async fn get_autonomy_briefing(State(state): State<AppState>) -> Response {
    let agent = state.agent.read().await;
    let settings = load_autonomy_settings(&agent).await;
    let briefing = build_autonomy_briefing(&agent, &settings).await;
    if let Ok(bytes) = serde_json::to_vec(&briefing) {
        let _ = agent.storage.set(AUTONOMY_LAST_BRIEF_KEY, &bytes).await;
    }
    (StatusCode::OK, Json(briefing)).into_response()
}

pub(super) async fn accept_autonomy_suggestion(
    State(state): State<AppState>,
    Path(id): Path<String>,
) -> Response {
    match accept_chat_suggestion(&state, &id).await {
        Ok(result) => (StatusCode::OK, Json(result)).into_response(),
        Err(error) if error.contains("not found") => {
            (StatusCode::NOT_FOUND, Json(ErrorResponse { error })).into_response()
        }
        Err(error) => (StatusCode::BAD_REQUEST, Json(ErrorResponse { error })).into_response(),
    }
}

pub(super) async fn dismiss_autonomy_suggestion(
    State(state): State<AppState>,
    Path(id): Path<String>,
) -> Response {
    match dismiss_chat_suggestion(&state, &id).await {
        Ok(result) => (StatusCode::OK, Json(result)).into_response(),
        Err(error) if error.contains("not found") => {
            (StatusCode::NOT_FOUND, Json(ErrorResponse { error })).into_response()
        }
        Err(error) => (StatusCode::BAD_REQUEST, Json(ErrorResponse { error })).into_response(),
    }
}

pub(super) async fn execute_autonomy_action(
    State(state): State<AppState>,
    Json(request): Json<AutonomyExecuteActionRequest>,
) -> Response {
    let agent = state.agent.read().await;
    let mut settings = load_autonomy_settings(&agent).await;
    match run_recommended_action(&agent, &mut settings, &request.action, request.dry_run).await {
        Ok(result) => {
            let _ = save_autonomy_settings(&agent, &settings).await;
            if !request.dry_run {
                spawn_autonomy_analysis_tick(state.agent.clone(), "autonomy_action");
            }
            let summary = summarize_autonomy_action_result(&request.action, &result);
            let trace_id_from_result = result
                .get("trace_id")
                .and_then(|value| value.as_str())
                .map(str::trim)
                .filter(|value| !value.is_empty())
                .map(|value| value.to_string());
            drop(agent);
            let trace_id = if request.dry_run {
                None
            } else if trace_id_from_result.is_some() {
                trace_id_from_result
            } else {
                persist_autonomy_action_trace(
                    &state,
                    &request.action,
                    result
                        .get("status")
                        .and_then(|value| value.as_str())
                        .unwrap_or("executed"),
                    &summary,
                    serde_json::json!({
                        "action": request.action.clone(),
                        "result": result.clone(),
                    }),
                )
                .await
            };
            (
                StatusCode::OK,
                Json(serde_json::json!({
                    "status":"ok",
                    "message": summary,
                    "trace_id": trace_id,
                    "result": result
                })),
            )
                .into_response()
        }
        Err(error) => {
            drop(agent);
            let trace_id = if request.dry_run {
                None
            } else {
                persist_autonomy_action_trace(
                    &state,
                    &request.action,
                    "error",
                    &error,
                    serde_json::json!({
                        "action": request.action.clone(),
                        "error": error.clone(),
                    }),
                )
                .await
            };
            (
                StatusCode::BAD_REQUEST,
                Json(serde_json::json!({
                    "error": error,
                    "trace_id": trace_id,
                })),
            )
                .into_response()
        }
    }
}

pub(super) async fn start_goal_loop(
    State(state): State<AppState>,
    Json(request): Json<GoalLoopRequest>,
) -> Response {
    let goal = request.goal.trim();
    if goal.is_empty() {
        return (
            StatusCode::BAD_REQUEST,
            Json(ErrorResponse {
                error: "goal is required".to_string(),
            }),
        )
            .into_response();
    }

    // Parse optional due date (YYYY-MM-DD), stored as scheduled_for for reminders and visibility.
    let due_date = request
        .due_date
        .as_deref()
        .and_then(|s| chrono::NaiveDate::parse_from_str(s, "%Y-%m-%d").ok())
        .map(|d| d.and_hms_opt(23, 59, 59).unwrap())
        .map(|dt| chrono::DateTime::<chrono::Utc>::from_naive_utc_and_offset(dt, chrono::Utc));

    let agent = state.agent.read().await;
    let actions = agent
        .runtime
        .list_enabled_actions()
        .await
        .unwrap_or_default();
    let planner_request = format!(
        "Goal: {}\nConstraints: {}",
        goal,
        request
            .constraints
            .clone()
            .unwrap_or_else(|| "none".to_string())
    );
    let override_plan = request.plan_override.as_ref().and_then(|value| {
        crate::core::orchestration::planner::parse_plan_from_value(value, &actions, None, 1, false)
    });
    let planned = if let Some(plan) = override_plan {
        Some(plan)
    } else {
        let (planner_system, planner_prompt) =
            crate::core::orchestration::planner::build_plan_prompt(
                &planner_request,
                None,
                &actions,
                crate::core::PlanPromptMode::GoalLoop,
                None,
            );
        match agent
            .llm
            .chat(&planner_system, &planner_prompt, &[], &actions)
            .await
        {
            Ok(response) => crate::core::orchestration::planner::parse_plan_from_llm_content(
                &response.content,
                &actions,
                None,
                1,
                false,
            ),
            Err(_) => None,
        }
    };
    let Some(parsed) = planned else {
        return (
            StatusCode::BAD_REQUEST,
            Json(ErrorResponse {
                error: "Planner returned invalid JSON".to_string(),
            }),
        )
            .into_response();
    };

    let report_cron = request
        .report_cron
        .clone()
        .unwrap_or("0 0 9 * * *".to_string());

    if request.preview_only {
        return (
            StatusCode::OK,
            Json(serde_json::json!({
                "status":"preview",
                "plan_preview": parsed.clone(),
                "scheduled_report_cron": report_cron.clone(),
            })),
        )
            .into_response();
    }

    let goal_id = uuid::Uuid::new_v4().to_string();
    let mut goal_task = Task::new(
        format!("Goal: {}", goal),
        "goal".to_string(),
        serde_json::json!({
            "goal_id": goal_id,
            "goal": goal,
        }),
    );
    goal_task.scheduled_for = due_date;
    // Goal task is a metadata anchor for grouping/progress, not an executable action.
    goal_task.status = TaskStatus::Completed;
    goal_task.result = Some("Goal registered.".to_string());
    if let Err(e) = agent.add_task(goal_task).await {
        return (
            StatusCode::INTERNAL_SERVER_ERROR,
            Json(ErrorResponse {
                error: e.to_string(),
            }),
        )
            .into_response();
    }

    // Auto-schedule reminder tasks if due date is set and > 1 day away.
    if let Some(due) = due_date {
        let now = chrono::Utc::now();
        let days_until = (due - now).num_days();

        let mut reminders: Vec<Task> = Vec::new();
        if days_until > 1 {
            let remind_at = due - chrono::Duration::days(1);
            let mut r = Task::new(
                format!("Reminder: \"{}\" is due tomorrow", goal),
                "goal_reminder".to_string(),
                serde_json::json!({ "goal": goal, "days_left": 1 }),
            );
            r.scheduled_for = Some(remind_at);
            reminders.push(r);
        }
        if days_until > 3 {
            let remind_at = due - chrono::Duration::days(3);
            let mut r = Task::new(
                format!("Reminder: \"{}\" is due in 3 days", goal),
                "goal_reminder".to_string(),
                serde_json::json!({ "goal": goal, "days_left": 3 }),
            );
            r.scheduled_for = Some(remind_at);
            reminders.push(r);
        }

        for r in reminders {
            let _ = agent.add_task(r).await;
        }
    }

    let action_names: Vec<String> = actions.iter().map(|a| a.name.clone()).collect();
    let normalized_steps: Vec<serde_json::Value> = parsed
        .steps
        .iter()
        .filter_map(|step| {
            let action_name = step
                .action
                .as_deref()
                .or(step.tool_hint.as_deref())
                .unwrap_or("");
            if action_name.is_empty() {
                return None;
            }
            let safe_action = if action_names.iter().any(|name| name == action_name) {
                action_name
            } else {
                return None;
            };
            let mut args = step
                .arguments
                .clone()
                .unwrap_or_else(|| serde_json::json!({}));
            args["goal_id"] = serde_json::json!(goal_id.clone());
            args["goal"] = serde_json::json!(goal);
            Some(serde_json::json!({
                "action": safe_action,
                "arguments": args,
                "rationale": if step.description.trim().is_empty() {
                    "goal-driven step"
                } else {
                    step.description.trim()
                },
            }))
        })
        .collect();
    if normalized_steps.is_empty() {
        return (
            StatusCode::BAD_REQUEST,
            Json(ErrorResponse {
                error: "Planner returned no runnable goal steps".to_string(),
            }),
        )
            .into_response();
    }

    let mut plan_task = Task::new(
        format!("Goal Loop Plan: {}", goal),
        "plan".to_string(),
        serde_json::json!({
            "goal_id": goal_id,
            "steps": normalized_steps,
            "summary": parsed.summary.clone(),
            "plan": parsed.clone(),
        }),
    );
    plan_task.status = TaskStatus::Pending;
    if let Err(e) = agent.add_task(plan_task).await {
        return (
            StatusCode::INTERNAL_SERVER_ERROR,
            Json(ErrorResponse {
                error: e.to_string(),
            }),
        )
            .into_response();
    }

    let mut report_task = Task::new(
        format!("Goal Progress Report: {}", goal),
        "goal_progress_report".to_string(),
        serde_json::json!({ "goal_id": goal_id, "goal": goal }),
    );
    report_task.cron = Some(report_cron.clone());
    report_task.status = TaskStatus::Pending;
    report_task.approval = TaskApproval::Auto;
    let _ = agent.add_task(report_task).await;

    agent
        .emit_notification(
            "Goal loop started",
            &format!(
                "Goal '{}' entered execution loop with {} planned step(s).",
                goal,
                normalized_steps.len()
            ),
            "info",
            "autonomy_goal_loop",
        )
        .await;

    (
        StatusCode::OK,
        Json(serde_json::json!({
            "status":"ok",
            "goal_id": goal_id,
            "plan_preview": parsed,
            "scheduled_report_cron": report_cron,
        })),
    )
        .into_response()
}

pub(super) async fn goal_progress_endpoint(
    State(state): State<AppState>,
    axum::extract::Query(params): axum::extract::Query<HashMap<String, String>>,
) -> Response {
    let goal_id = params.get("goal_id").map(|s| s.as_str());
    let agent = state.agent.read().await;
    let tasks = agent.tasks.read().await;
    let related: Vec<&Task> = tasks
        .all()
        .iter()
        .filter(|t| {
            goal_id
                .map(|g| t.arguments.get("goal_id").and_then(|v| v.as_str()) == Some(g))
                .unwrap_or(t.arguments.get("goal_id").is_some())
        })
        .collect();
    let completed = related
        .iter()
        .filter(|t| matches!(t.status, TaskStatus::Completed))
        .count();
    let pending = related
        .iter()
        .filter(|t| {
            matches!(
                t.status,
                TaskStatus::Pending | TaskStatus::AwaitingApproval | TaskStatus::InProgress
            )
        })
        .count();
    let failed = related
        .iter()
        .filter(|t| matches!(t.status, TaskStatus::Failed { .. }))
        .count();

    let items: Vec<serde_json::Value> = related
        .iter()
        .take(20)
        .map(|t| {
            serde_json::json!({
                "id": t.id.to_string(),
                "description": t.description,
                "action": t.action,
                "status": format!("{:?}", t.status),
                "created_at": t.created_at.to_rfc3339(),
                "result": t.result,
            })
        })
        .collect();

    (
        StatusCode::OK,
        Json(serde_json::json!({
            "goal_id": goal_id,
            "summary": {
                "total": related.len(),
                "completed": completed,
                "pending_or_running": pending,
                "failed": failed,
            },
            "items": items,
        })),
    )
        .into_response()
}

pub(super) async fn run_goal_report_now(
    State(state): State<AppState>,
    Json(request): Json<GoalReportNowRequest>,
) -> Response {
    let goal_id = request.goal_id.trim();
    if goal_id.is_empty() {
        return (
            StatusCode::BAD_REQUEST,
            Json(ErrorResponse {
                error: "goal_id is required".to_string(),
            }),
        )
            .into_response();
    }

    let agent = state.agent.read().await;

    // Find goal text from an existing goal task.
    let goal_text = {
        let tasks = agent.tasks.read().await;
        let goal_task = tasks.all().iter().find(|t| {
            t.action == "goal"
                && t.arguments
                    .get("goal_id")
                    .and_then(|v| v.as_str())
                    .map(|v| v == goal_id)
                    .unwrap_or(false)
        });
        if let Some(t) = goal_task {
            let goal = t
                .arguments
                .get("goal")
                .and_then(|v| v.as_str())
                .map(|s| s.to_string())
                .unwrap_or(t.description.trim_start_matches("Goal: ").to_string());
            goal
        } else {
            return (
                StatusCode::NOT_FOUND,
                Json(ErrorResponse {
                    error: "goal_id not found".to_string(),
                }),
            )
                .into_response();
        }
    };

    let mut report_task = Task::new(
        format!("Goal Progress Report (manual): {}", goal_text),
        "goal_progress_report".to_string(),
        serde_json::json!({
            "goal_id": goal_id,
            "goal": goal_text,
        }),
    );
    report_task.scheduled_for = Some(chrono::Utc::now());
    report_task.status = TaskStatus::Pending;
    report_task.approval = TaskApproval::Auto;

    if let Err(e) = agent.add_task(report_task).await {
        return (
            StatusCode::INTERNAL_SERVER_ERROR,
            Json(ErrorResponse {
                error: e.to_string(),
            }),
        )
            .into_response();
    }

    (StatusCode::OK, Json(serde_json::json!({ "status": "ok" }))).into_response()
}

pub(super) async fn get_live_incidents(State(state): State<AppState>) -> Response {
    let agent = state.agent.read().await;
    let mut incidents: Vec<serde_json::Value> = Vec::new();

    let security_logs = agent
        .storage
        .list_security_logs(80)
        .await
        .unwrap_or_default();
    let critical_security: Vec<_> = security_logs
        .iter()
        .filter(|s| s.severity == "error" || s.severity == "critical")
        .collect();
    if !critical_security.is_empty() {
        incidents.push(serde_json::json!({
            "id": format!("sec:{}", critical_security[0].event_type),
            "severity": "critical",
            "title": "Security anomaly detected",
            "detail": format!("{} high-severity security event(s) recorded.", critical_security.len()),
        }));
    }

    let failed_tasks: Vec<_> = {
        let tasks = agent.tasks.read().await;
        tasks
            .all()
            .iter()
            .filter(|t| matches!(t.status, TaskStatus::Failed { .. }))
            .take(5)
            .cloned()
            .collect::<Vec<_>>()
    };
    for task in &failed_tasks {
        incidents.push(serde_json::json!({
            "id": format!("task_fail:{}", task.id),
            "severity": "high",
            "title": "Task failure requires triage",
            "detail": task.description,
        }));
    }

    let failed_watchers: Vec<_> = agent
        .watcher_manager
        .list()
        .await
        .into_iter()
        .filter(|w| {
            matches!(
                w.status,
                crate::core::automation::watcher::WatcherStatus::Failed { .. }
                    | crate::core::automation::watcher::WatcherStatus::TimedOut
            )
        })
        .collect();
    for watcher in failed_watchers.iter().take(5) {
        incidents.push(serde_json::json!({
            "id": format!("watcher:{}", watcher.id),
            "severity": "medium",
            "title": "Watcher degraded",
            "detail": watcher.description,
        }));
    }

    incidents.sort_by(|a, b| {
        let sa = a.get("severity").and_then(|v| v.as_str()).unwrap_or("low");
        let sb = b.get("severity").and_then(|v| v.as_str()).unwrap_or("low");
        sb.cmp(sa)
    });

    (
        StatusCode::OK,
        Json(serde_json::json!({ "incidents": incidents })),
    )
        .into_response()
}

pub(super) async fn execute_incident_playbook(
    State(state): State<AppState>,
    Path(id): Path<String>,
) -> Response {
    let agent = state.agent.read().await;
    let mut settings = load_autonomy_settings(&agent).await;
    let readiness_policy =
        crate::core::runtime::readiness::load_readiness_policy(&agent.storage).await;
    let action = if id.starts_with("sec:") {
        recommendation(
            "Contain Security Incident",
            "Start a security containment and mitigation workflow.",
            "create_task",
            serde_json::json!({
                "description":"Contain security incident and propose mitigations",
                "action":"research",
                "arguments":{"query":"Contain current security incident, identify source, and propose mitigations."},
                "approval":"require"
            }),
            &settings.trust_policy,
            &readiness_policy,
        )
    } else if id.starts_with("task_fail:") {
        recommendation(
            "Recover Failed Task",
            "Generate a concrete recovery plan for the failed execution.",
            "chat_prompt",
            serde_json::json!({"prompt":"Review failed tasks, identify root causes, and propose immediate recovery actions."}),
            &settings.trust_policy,
            &readiness_policy,
        )
    } else {
        recommendation(
            "Stabilize Incident",
            "Produce a stabilization checklist for this incident.",
            "chat_prompt",
            serde_json::json!({"prompt":"Create a stabilization checklist for the current incident and prioritize actions."}),
            &settings.trust_policy,
            &readiness_policy,
        )
    };

    match run_recommended_action(&agent, &mut settings, &action, false).await {
        Ok(result) => (
            StatusCode::OK,
            Json(serde_json::json!({"status":"ok","result":result})),
        )
            .into_response(),
        Err(e) => (StatusCode::BAD_REQUEST, Json(ErrorResponse { error: e })).into_response(),
    }
}

pub(super) async fn triage_inbox(
    State(state): State<AppState>,
    Json(request): Json<InboxTriageRequest>,
) -> Response {
    let labels = request
        .labels
        .clone()
        .filter(|l| !l.is_empty())
        .unwrap_or(vec![
            "Act now".to_string(),
            "Delegate".to_string(),
            "Ignore".to_string(),
        ]);
    let agent = state.agent.read().await;

    let mut messages = request.messages.clone();
    if messages.is_empty() {
        let fallback = agent
            .storage
            .list_notifications(30, 0, true)
            .await
            .unwrap_or_default();
        messages = fallback
            .into_iter()
            .map(|n| {
                serde_json::json!({
                    "id": n.id,
                    "from": n.source,
                    "subject": n.title,
                    "snippet": n.body,
                })
            })
            .collect();
    }

    if messages.is_empty() {
        return (
            StatusCode::OK,
            Json(serde_json::json!({"triage":[],"draft_replies":[]})),
        )
            .into_response();
    }

    let payload = serde_json::json!({ "messages": messages, "labels": labels });
    let llm_response = agent.llm.chat(
        "You are an executive inbox triage assistant. Return strict JSON {\"triage\":[{\"message_id\":\"...\",\"label\":\"...\",\"reason\":\"...\",\"draft_reply\":\"...\"}]}.",
        &payload.to_string(),
        &[],
        &[],
    ).await.ok();
    if let Some(ref r) = llm_response {
        agent.record_llm_usage("web", "inbox_triage", r).await;
    }

    let parsed = llm_response
        .as_ref()
        .and_then(|r| extract_json(&r.content))
        .unwrap_or_else(|| {
            let triage: Vec<serde_json::Value> = payload
                .get("messages")
                .and_then(|v| v.as_array())
                .cloned()
                .unwrap_or_default()
                .iter()
                .map(|m| {
                    let label = "Review";
                    serde_json::json!({
                                "message_id": m.get("id").cloned().unwrap_or(serde_json::json!("")),
                        "label": label,
                        "reason": "LLM triage unavailable; conservative fallback",
                        "draft_reply": "",
                    })
                })
                .collect();
            serde_json::json!({ "triage": triage })
        });

    let triage = parsed
        .get("triage")
        .cloned()
        .unwrap_or(serde_json::json!([]));
    (
        StatusCode::OK,
        Json(serde_json::json!({
            "triage": triage,
            "labels": labels,
        })),
    )
        .into_response()
}

pub(super) async fn get_outcome_timeline(
    State(state): State<AppState>,
    axum::extract::Query(params): axum::extract::Query<HashMap<String, String>>,
) -> Response {
    let limit = params
        .get("limit")
        .and_then(|v| v.parse::<usize>().ok())
        .unwrap_or(120usize)
        .min(500);
    let agent = state.agent.read().await;
    let mut events: Vec<serde_json::Value> = Vec::new();

    {
        let trace = state.trace_history.read().await;
        for t in trace.iter().take(limit) {
            let ts = t
                .completed_at
                .unwrap_or_else(|| t.started_at.unwrap_or_else(chrono::Utc::now))
                .to_rfc3339();
            events.push(serde_json::json!({
                "id": format!("trace:{}", t.id),
                "source": "trace",
                "timestamp": ts,
                "title": t.message,
                "status": if t.completed_at.is_some() { "completed" } else { "in_progress" },
                "detail": t.response.as_deref().map(crate::security::redact_pii),
                "rollback": null
            }));
        }
    }
    {
        let tasks = agent.tasks.read().await;
        for t in tasks.all().iter().take(limit) {
            events.push(serde_json::json!({
                "id": format!("task:{}", t.id),
                "source": "task",
                "timestamp": t.created_at.to_rfc3339(),
                "title": t.description,
                "status": format!("{:?}", t.status),
                "detail": t.result.as_deref().map(crate::security::redact_pii),
                "rollback": {
                    "operation": if matches!(t.status, TaskStatus::Pending | TaskStatus::AwaitingApproval | TaskStatus::Paused | TaskStatus::InProgress) { "cancel_task" } else { "none" }
                }
            }));
        }
    }

    for n in agent
        .storage
        .list_notifications(limit as u64, 0, false)
        .await
        .unwrap_or_default()
    {
        events.push(serde_json::json!({
            "id": format!("notification:{}", n.id),
            "source": "notification",
            "timestamp": n.created_at,
            "title": n.title,
            "status": if n.read { "read" } else { "unread" },
            "detail": crate::security::redact_pii(&n.body),
            "rollback": { "operation": "toggle_notification_read" }
        }));
    }

    for s in agent
        .storage
        .list_security_logs(limit as u64)
        .await
        .unwrap_or_default()
    {
        events.push(serde_json::json!({
            "id": format!("security:{}", s.id),
            "source": "security",
            "timestamp": s.created_at,
            "title": format!("{} [{}]", s.event_type, s.severity),
            "status": "logged",
            "detail": crate::security::redact_pii(&s.message),
            "rollback": null
        }));
    }

    for d in agent
        .storage
        .get_recent_delegations(limit as u64)
        .await
        .unwrap_or_default()
    {
        events.push(serde_json::json!({
            "id": format!("delegation:{}", d.id),
            "source": "delegation",
            "timestamp": d.created_at,
            "title": d.task_description,
            "status": if d.success == 1 { "success" } else { "failed" },
            "detail": d.result.as_deref().map(crate::security::redact_pii),
            "rollback": null
        }));
    }

    events.sort_by(|a, b| {
        b.get("timestamp")
            .and_then(|v| v.as_str())
            .unwrap_or("")
            .cmp(a.get("timestamp").and_then(|v| v.as_str()).unwrap_or(""))
    });
    events.truncate(limit);

    (
        StatusCode::OK,
        Json(serde_json::json!({ "events": events })),
    )
        .into_response()
}

pub(super) async fn rollback_timeline_event(
    State(state): State<AppState>,
    Json(request): Json<TimelineRollbackRequest>,
) -> Response {
    let agent = state.agent.read().await;
    let event_id = request.event_id.trim();
    let operation = request.operation.unwrap_or_default();

    if let Some(task_id) = event_id.strip_prefix("task:") {
        let uuid = match uuid::Uuid::parse_str(task_id) {
            Ok(v) => v,
            Err(_) => {
                return (
                    StatusCode::BAD_REQUEST,
                    Json(ErrorResponse {
                        error: "Invalid task id".to_string(),
                    }),
                )
                    .into_response();
            }
        };
        let mut tasks = agent.tasks.write().await;
        let Some(task) = tasks.get_mut(uuid) else {
            return (
                StatusCode::NOT_FOUND,
                Json(ErrorResponse {
                    error: "Task not found".to_string(),
                }),
            )
                .into_response();
        };
        if !matches!(
            task.status,
            TaskStatus::Pending | TaskStatus::AwaitingApproval | TaskStatus::InProgress
        ) {
            return (
                StatusCode::BAD_REQUEST,
                Json(ErrorResponse {
                    error: "Task cannot be cancelled from current state".to_string(),
                }),
            )
                .into_response();
        }
        task.status = TaskStatus::Cancelled;
        let status_json =
            serde_json::to_string(&task.status).unwrap_or("\"Cancelled\"".to_string());
        let _ = agent
            .storage
            .update_task_status(task_id, &status_json)
            .await;
        return (
            StatusCode::OK,
            Json(serde_json::json!({"status":"ok","operation":"cancel_task"})),
        )
            .into_response();
    }

    if let Some(watcher_id) = event_id.strip_prefix("watcher:") {
        let uuid = match uuid::Uuid::parse_str(watcher_id) {
            Ok(v) => v,
            Err(_) => {
                return (
                    StatusCode::BAD_REQUEST,
                    Json(ErrorResponse {
                        error: "Invalid watcher id".to_string(),
                    }),
                )
                    .into_response();
            }
        };
        if agent.watcher_manager.cancel(uuid).await {
            if let Some(watcher) = agent.watcher_manager.get(uuid).await {
                agent
                    .sync_watcher_supervisor_state(&watcher, Some("cancelled"), None)
                    .await;
            }
            return (
                StatusCode::OK,
                Json(serde_json::json!({"status":"ok","operation":"cancel_watcher"})),
            )
                .into_response();
        }
        return (
            StatusCode::NOT_FOUND,
            Json(ErrorResponse {
                error: "Watcher not found or not cancellable".to_string(),
            }),
        )
            .into_response();
    }

    if let Some(notification_id) = event_id.strip_prefix("notification:") {
        let read = operation != "mark_unread";
        if let Err(e) = agent
            .storage
            .set_notification_read(notification_id, read)
            .await
        {
            return (
                StatusCode::INTERNAL_SERVER_ERROR,
                Json(ErrorResponse {
                    error: e.to_string(),
                }),
            )
                .into_response();
        }
        return (StatusCode::OK, Json(serde_json::json!({"status":"ok","operation":"toggle_notification_read","read":read}))).into_response();
    }

    (
        StatusCode::BAD_REQUEST,
        Json(ErrorResponse {
            error: "Unsupported rollback target".to_string(),
        }),
    )
        .into_response()
}

pub(super) async fn query_knowledge_brain(
    State(state): State<AppState>,
    Json(request): Json<KnowledgeQueryRequest>,
) -> Response {
    if request.query.trim().is_empty() {
        return (
            StatusCode::BAD_REQUEST,
            Json(ErrorResponse {
                error: "query is required".to_string(),
            }),
        )
            .into_response();
    }
    let limit = request.limit.unwrap_or(8).clamp(1, 20);
    let agent = state.agent.read().await;
    let docs = agent
        .search_documents(&request.query, limit, None)
        .await
        .unwrap_or_default();
    let facts = agent
        .encrypted_storage
        .get_facts_decrypted()
        .await
        .unwrap_or_default()
        .into_iter()
        .take(limit)
        .collect::<Vec<_>>();

    let evidence_docs: Vec<serde_json::Value> = docs
        .iter()
        .map(|hit| {
            serde_json::json!({
                "document_id": &hit.document_id,
                "filename": &hit.filename,
                "content_type": &hit.content_type,
                "chunk_index": hit.chunk_index,
                "score": hit.score,
                "match_reason": &hit.match_reason,
                "snippet": hit.content.chars().take(260).collect::<String>(),
            })
        })
        .collect();
    let evidence_facts: Vec<serde_json::Value> = facts
        .iter()
        .take(limit)
        .map(|f| {
            serde_json::json!({
                "fact": f.fact,
                "confidence": f.confidence,
                "sources": f.sources,
            })
        })
        .collect();

    let synthesis_prompt = format!(
        "Answer the user query using the supplied evidence only.\n\
If confidence is low, explicitly say what knowledge should be imported.\n\
User query: {}\n\nEvidence docs: {}\n\nEvidence facts: {}",
        request.query,
        serde_json::to_string(&evidence_docs).unwrap_or_default(),
        serde_json::to_string(&evidence_facts).unwrap_or_default(),
    );
    let answer = match agent
        .llm
        .chat(
            "You are a grounded knowledge assistant. Cite document IDs inline like [doc:<id>].",
            &synthesis_prompt,
            &[],
            &[],
        )
        .await
    {
        Ok(r) => {
            agent
                .record_llm_usage("web", "knowledge_synthesis", &r)
                .await;
            crate::security::redact_pii(&r.content)
        }
        Err(_) => {
            if evidence_docs.is_empty() && evidence_facts.is_empty() {
                "I do not have enough indexed knowledge yet. Import documents, notes, or emails related to this topic.".to_string()
            } else {
                "I found relevant evidence, but synthesis failed. Try again with a narrower question.".to_string()
            }
        }
    };

    let missing_signals = if evidence_docs.len() < 2 {
        vec![
            "Import source documents for this topic".to_string(),
            "Ingest related emails or notes to improve retrieval".to_string(),
        ]
    } else {
        vec![]
    };

    (
        StatusCode::OK,
        Json(serde_json::json!({
            "answer": answer,
            "sources": {
                "documents": evidence_docs,
                "facts": evidence_facts,
            },
            "import_suggestions": missing_signals,
        })),
    )
        .into_response()
}

pub(super) async fn suggest_knowledge_imports(State(state): State<AppState>) -> Response {
    let agent = state.agent.read().await;
    let traces = agent.trace_history.read().await;
    let mut token_counts: HashMap<String, usize> = HashMap::new();
    for t in traces.iter().take(60) {
        for word in t
            .message
            .to_ascii_lowercase()
            .split(|c: char| !c.is_ascii_alphanumeric())
            .filter(|w| w.len() >= 5)
        {
            *token_counts.entry(word.to_string()).or_insert(0) += 1;
        }
    }
    let mut tokens: Vec<(String, usize)> = token_counts.into_iter().collect();
    tokens.sort_by_key(|item| std::cmp::Reverse(item.1));
    let suggestions: Vec<serde_json::Value> = tokens
        .into_iter()
        .take(8)
        .map(|(topic, count)| {
            serde_json::json!({
                "topic": topic,
                "signal_count": count,
                "suggested_import": format!("Add documents/notes related to '{}'", topic),
            })
        })
        .collect();
    (
        StatusCode::OK,
        Json(serde_json::json!({ "suggestions": suggestions })),
    )
        .into_response()
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub(super) enum AutonomyAnalysisTickMode {
    Full,
    Reactive,
}

impl AutonomyAnalysisTickMode {
    fn as_str(self) -> &'static str {
        match self {
            Self::Full => "full",
            Self::Reactive => "reactive",
        }
    }

    fn timeout(self) -> Duration {
        match self {
            Self::Full => AUTONOMY_ANALYSIS_TICK_TIMEOUT,
            Self::Reactive => AUTONOMY_REACTIVE_TICK_TIMEOUT,
        }
    }

    fn runs_full_scan(self) -> bool {
        matches!(self, Self::Full)
    }
}

pub(super) struct AutonomyAnalysisTickGuard;

impl Drop for AutonomyAnalysisTickGuard {
    fn drop(&mut self) {
        AUTONOMY_ANALYSIS_TICK_IN_FLIGHT.store(false, Ordering::Release);
    }
}

pub(super) fn try_start_autonomy_analysis_tick(
    trigger: &str,
    mode: AutonomyAnalysisTickMode,
) -> Option<AutonomyAnalysisTickGuard> {
    if AUTONOMY_ANALYSIS_TICK_IN_FLIGHT
        .compare_exchange(false, true, Ordering::AcqRel, Ordering::Acquire)
        .is_err()
    {
        tracing::info!(
            trigger = trigger,
            mode = mode.as_str(),
            "Autonomy analysis tick skipped because one is already running"
        );
        return None;
    }
    Some(AutonomyAnalysisTickGuard)
}

pub(super) fn autonomy_analysis_in_progress_response(
    trigger: &str,
    mode: AutonomyAnalysisTickMode,
) -> serde_json::Value {
    serde_json::json!({
        "status": "ok",
        "trigger": trigger,
        "mode": mode.as_str(),
        "skipped": true,
        "reason": "in_progress",
        "generated_at": chrono::Utc::now().to_rfc3339(),
    })
}

pub(crate) async fn run_autonomy_analysis_tick(
    shared: Arc<RwLock<Agent>>,
    trigger: &str,
) -> serde_json::Value {
    let mode = AutonomyAnalysisTickMode::Full;
    let Some(_guard) = try_start_autonomy_analysis_tick(trigger, mode) else {
        return autonomy_analysis_in_progress_response(trigger, mode);
    };
    run_autonomy_analysis_tick_inner(shared, trigger, mode).await
}

pub(super) async fn run_autonomy_analysis_tick_inner(
    shared: Arc<RwLock<Agent>>,
    trigger: &str,
    mode: AutonomyAnalysisTickMode,
) -> serde_json::Value {
    let now = chrono::Utc::now();
    tracing::info!(
        trigger = trigger,
        mode = mode.as_str(),
        "Autonomy analysis loading state"
    );
    let (storage, tasks) = {
        let agent = shared.read().await;
        (agent.storage.clone(), agent.tasks.clone())
    };
    let settings = load_autonomy_settings_from_storage(&storage).await;

    if autonomy_background_disabled(&settings) {
        return serde_json::json!({
            "status":"paused",
            "trigger": trigger,
            "mode": mode.as_str(),
            "generated_at": now.to_rfc3339(),
            "message": "Autonomy is disabled.",
        });
    }

    // Prevent storming on chat-heavy sessions. Manual and scheduled triggers bypass this gate.
    if trigger != "manual" && trigger != "sentinel_periodic" {
        let last_scan = storage
            .get(AUTONOMY_ANALYSIS_LAST_RUN_KEY)
            .await
            .ok()
            .flatten()
            .and_then(|raw| String::from_utf8(raw).ok())
            .and_then(|s| parse_utc_rfc3339(&s));
        if let Some(last) = last_scan {
            if (now - last).num_seconds() < 30 {
                return serde_json::json!({
                    "status":"ok",
                    "trigger": trigger,
                    "mode": mode.as_str(),
                    "generated_at": now.to_rfc3339(),
                    "skipped": true,
                    "reason": "cooldown",
                });
            }
        }
    }

    tracing::info!(
        trigger = trigger,
        mode = mode.as_str(),
        "Autonomy analysis checking task attention"
    );
    let (awaiting_approval, missing_inputs) = {
        let tasks = tasks.read().await;
        let awaiting = tasks
            .all()
            .iter()
            .filter(|t| matches!(t.status, TaskStatus::AwaitingApproval))
            .count();
        drop(tasks);
        let unread = storage
            .list_notifications(120, 0, true)
            .await
            .unwrap_or_default();
        let missing = unread
            .iter()
            .filter(|n| notification_represents_missing_input(n))
            .count();
        (awaiting, missing)
    };
    let attention_signature = if missing_inputs > 0 {
        Some(format!("m:{}", missing_inputs))
    } else {
        None
    };
    let last_attention_signature = storage
        .get(AUTONOMY_ATTENTION_STATE_KEY)
        .await
        .ok()
        .flatten()
        .and_then(|raw| String::from_utf8(raw).ok());
    if let Some(signature) = attention_signature.as_deref() {
        if last_attention_signature.as_deref() != Some(signature) {
            let mode_state = if settings.autonomy_mode.eq_ignore_ascii_case("auto") {
                "ON"
            } else if settings.autonomy_mode.eq_ignore_ascii_case("assist") {
                "ASSIST"
            } else {
                "OFF"
            };
            let body = format!(
                "Auto Mode is {} | Waiting on you: {} missing input{}",
                mode_state,
                missing_inputs,
                if missing_inputs == 1 { "" } else { "s" }
            );
            {
                let agent = Agent::snapshot(&shared).await;
                agent
                    .emit_notification(
                        "Autonomy Needs Attention",
                        &body,
                        "warning",
                        "autonomy_attention",
                    )
                    .await;
                agent.notify_preferred_channel(&body).await;
            }
            let _ = storage
                .set(AUTONOMY_ATTENTION_STATE_KEY, signature.as_bytes())
                .await;
        }
    } else if last_attention_signature.is_some() {
        let _ = storage.delete(AUTONOMY_ATTENTION_STATE_KEY).await;
    }

    let _ = storage
        .set(AUTONOMY_ANALYSIS_LAST_RUN_KEY, now.to_rfc3339().as_bytes())
        .await;
    let sentinel = if mode.runs_full_scan() {
        tracing::info!(
            trigger = trigger,
            mode = mode.as_str(),
            "Autonomy analysis running sentinel scan"
        );
        sentinel_panel::run_sentinel_scan_tick(shared.clone(), trigger).await
    } else {
        tracing::info!(
            trigger = trigger,
            mode = mode.as_str(),
            "Autonomy analysis skipped sentinel scan for reactive tick"
        );
        serde_json::json!({
            "status": "ok",
            "trigger": trigger,
            "skipped": true,
            "reason": "reactive_tick",
            "generated_at": now.to_rfc3339(),
        })
    };
    let ambient_intents = if mode.runs_full_scan() {
        tracing::info!(
            trigger = trigger,
            mode = mode.as_str(),
            "Autonomy analysis revisiting ambient intents"
        );
        let agent = Agent::snapshot(&shared).await;
        agent.revisit_ambient_intents(trigger).await
    } else {
        tracing::info!(
            trigger = trigger,
            mode = mode.as_str(),
            "Autonomy analysis skipped ambient intent revisit for reactive tick"
        );
        serde_json::json!({
            "status": "ok",
            "trigger": trigger,
            "skipped": true,
            "reason": "reactive_tick",
            "generated_at": now.to_rfc3339(),
        })
    };

    serde_json::json!({
        "status":"ok",
        "trigger": trigger,
        "mode": mode.as_str(),
        "generated_at": now.to_rfc3339(),
        "needs_attention": {
            "awaiting_approval": awaiting_approval,
            "missing_inputs": missing_inputs,
        },
        "sentinel": sentinel,
        "ambient_intents": ambient_intents,
    })
}

pub(super) fn spawn_autonomy_analysis_tick(agent: SharedAgent, trigger: &str) {
    let mode = AutonomyAnalysisTickMode::Reactive;
    let Some(guard) = try_start_autonomy_analysis_tick(trigger, mode) else {
        return;
    };
    let trigger = trigger.to_string();
    crate::spawn_logged!("src/channels/http.rs:33346", async move {
        let _guard = guard;
        tracing::info!(
            trigger = trigger.as_str(),
            mode = mode.as_str(),
            "Autonomy analysis tick started"
        );
        match tokio::time::timeout(
            mode.timeout(),
            run_autonomy_analysis_tick_inner(agent, &trigger, mode),
        )
        .await
        {
            Ok(result) => {
                tracing::info!(
                    trigger = trigger.as_str(),
                    mode = mode.as_str(),
                    status = result
                        .get("status")
                        .and_then(|value| value.as_str())
                        .unwrap_or("unknown"),
                    "Autonomy analysis tick completed"
                );
            }
            Err(_) => {
                tracing::warn!(
                    trigger = trigger.as_str(),
                    mode = mode.as_str(),
                    timeout_secs = mode.timeout().as_secs(),
                    "Autonomy analysis tick timed out"
                );
            }
        }
    });
}

pub(super) async fn evaluate_trust_request(
    State(state): State<AppState>,
    Json(request): Json<TrustEvaluateRequest>,
) -> Response {
    let agent = state.agent.read().await;
    let settings = load_autonomy_settings(&agent).await;
    let envelope: RiskEnvelope = score_action_risk(
        &request.action_kind,
        &request.payload,
        &settings.trust_policy,
    );
    (
        StatusCode::OK,
        Json(serde_json::json!({
            "risk": {
                "level": risk_level_label(&envelope.level),
                "score": envelope.score,
                "requires_approval": envelope.requires_approval,
                "reasons": envelope.reasons,
            }
        })),
    )
        .into_response()
}

pub(super) async fn get_voice_briefing(State(state): State<AppState>) -> Response {
    let agent = Agent::snapshot(&state.agent).await;
    let settings = load_autonomy_settings(&agent).await;
    if !settings.voice_briefing_enabled {
        return (
            StatusCode::OK,
            Json(serde_json::json!({"enabled":false,"message":"Voice briefing is disabled"})),
        )
            .into_response();
    }
    let briefing = build_autonomy_briefing(&agent, &settings).await;
    let short_risks = briefing
        .top_risks
        .iter()
        .take(2)
        .map(|r| r.get("title").and_then(|v| v.as_str()).unwrap_or("risk"))
        .collect::<Vec<_>>()
        .join("; ");
    let short_opps = briefing
        .top_opportunities
        .iter()
        .take(2)
        .map(|r| {
            r.get("title")
                .and_then(|v| v.as_str())
                .unwrap_or("opportunity")
        })
        .collect::<Vec<_>>()
        .join("; ");
    let spoken = format!(
        "Good day. Top risks: {}. Top opportunities: {}. I have {} recommended action items.",
        if short_risks.is_empty() {
            "none critical"
        } else {
            &short_risks
        },
        if short_opps.is_empty() {
            "none identified"
        } else {
            &short_opps
        },
        briefing.recommended_actions.len()
    );
    let ssml = format!(
        "<speak><p>{}</p><p>You can say: do it, defer, or summarize.</p></speak>",
        spoken
    );
    if let Ok(bytes) = serde_json::to_vec(&briefing) {
        let _ = agent.storage.set(AUTONOMY_LAST_BRIEF_KEY, &bytes).await;
    }
    (
        StatusCode::OK,
        Json(serde_json::json!({
            "enabled": true,
            "spoken_text": spoken,
            "ssml": ssml,
            "recommended_actions": briefing.recommended_actions,
        })),
    )
        .into_response()
}

pub(super) async fn handle_voice_command(
    State(state): State<AppState>,
    Json(request): Json<VoiceCommandRequest>,
) -> Response {
    let cmd = request.command.trim().to_ascii_lowercase();
    let agent = Agent::snapshot(&state.agent).await;
    let mut settings = load_autonomy_settings(&agent).await;
    let last_brief = agent
        .storage
        .get(AUTONOMY_LAST_BRIEF_KEY)
        .await
        .ok()
        .flatten()
        .and_then(|v| serde_json::from_slice::<AutonomyBriefingResponse>(&v).ok());

    match cmd.as_str() {
        "summarize" => {
            if let Some(brief) = last_brief {
                return (
                    StatusCode::OK,
                    Json(serde_json::json!({
                        "status":"ok",
                        "summary": {
                            "risks": brief.top_risks,
                            "opportunities": brief.top_opportunities,
                            "actions": brief.recommended_actions,
                        }
                    })),
                )
                    .into_response();
            }
            (
                StatusCode::OK,
                Json(serde_json::json!({"status":"ok","summary":"No recent briefing available"})),
            )
                .into_response()
        }
        "defer" => {
            agent
                .emit_notification(
                    "Voice command",
                    "Skill deferred by voice command.",
                    "info",
                    "voice",
                )
                .await;
            (
                StatusCode::OK,
                Json(serde_json::json!({"status":"ok","result":"Deferred current recommendation"})),
            )
                .into_response()
        }
        "do it" => {
            let Some(brief) = last_brief else {
                return (
                    StatusCode::BAD_REQUEST,
                    Json(ErrorResponse {
                        error: "No recent voice briefing to execute from".to_string(),
                    }),
                )
                    .into_response();
            };
            let action = if let Some(action_id) = request.action_id.as_ref() {
                brief
                    .recommended_actions
                    .into_iter()
                    .find(|a| &a.id == action_id)
            } else {
                brief.recommended_actions.into_iter().next()
            };
            let Some(action) = action else {
                return (
                    StatusCode::BAD_REQUEST,
                    Json(ErrorResponse {
                        error: "No matching recommendation found".to_string(),
                    }),
                )
                    .into_response();
            };
            return match run_recommended_action(&agent, &mut settings, &action, false).await {
                Ok(result) => (
                    StatusCode::OK,
                    Json(serde_json::json!({"status":"ok","result":result})),
                )
                    .into_response(),
                Err(e) => {
                    (StatusCode::BAD_REQUEST, Json(ErrorResponse { error: e })).into_response()
                }
            };
        }
        _ => {
            let prompt = format!(
                "Voice command: {}. Respond with a short actionable interpretation.",
                request.command
            );
            let conversation_id = request
                .conversation_id
                .clone()
                .filter(|value| !value.trim().is_empty())
                .unwrap_or_else(|| format!("voice:{}", uuid::Uuid::new_v4()));
            match agent
                .process_message_with_meta(&prompt, "voice", Some(&conversation_id), None)
                .await
            {
                Ok(r) => (
                    StatusCode::OK,
                    Json(serde_json::json!({
                        "status": "ok",
                        "response": crate::security::redact_pii(&r.response),
                        "run_status": r.run_status,
                        "degradation": r.degradation,
                        "attempted_models": r.attempted_models,
                        "user_outcome": r.user_outcome,
                    })),
                )
                    .into_response(),
                Err(e) => {
                    let response =
                        "I hit a framework-level problem while handling the voice command."
                            .to_string();
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
                    (
                        StatusCode::OK,
                        Json(serde_json::json!({
                            "status": "error",
                            "response": response,
                            "run_status": "platform_failed",
                            "degradation": degradation,
                            "attempted_models": user_outcome.attempted_models,
                            "user_outcome": user_outcome,
                        })),
                    )
                        .into_response()
                }
            }
        }
    }
}
