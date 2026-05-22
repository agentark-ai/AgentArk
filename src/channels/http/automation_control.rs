use super::*;

fn task_list_status_rank(status: &TaskStatus) -> u8 {
    match status {
        TaskStatus::AwaitingApproval | TaskStatus::ExpiredNeedsReapproval => 0,
        TaskStatus::Failed { .. } => 1,
        TaskStatus::Paused => 2,
        TaskStatus::InProgress => 3,
        TaskStatus::Pending => 4,
        TaskStatus::Cancelled => 5,
        TaskStatus::Completed => 6,
    }
}

pub(super) async fn list_tasks(
    State(state): State<AppState>,
    axum::extract::Query(params): axum::extract::Query<std::collections::HashMap<String, String>>,
) -> Json<serde_json::Value> {
    let limit = params
        .get("limit")
        .and_then(|s| s.parse().ok())
        .unwrap_or(20usize);
    let offset = params
        .get("offset")
        .and_then(|s| s.parse().ok())
        .unwrap_or(0usize);
    let agent = state.agent.read().await;
    if let Err(error) = agent.repair_settings_authorized_approval_tasks().await {
        tracing::warn!(
            error = %error,
            "Failed to repair settings-authorized approval tasks before listing tasks"
        );
    }
    let tasks = agent.tasks.read().await;
    let all = tasks.all();
    let total = all.len();
    let sort = params
        .get("sort")
        .map(|value| value.trim().to_ascii_lowercase())
        .unwrap_or_default();
    let mut task_refs: Vec<&Task> = all.iter().collect();
    match sort.as_str() {
        "ops" | "operator" | "operator_queue" => {
            task_refs.sort_by(|left, right| {
                task_list_status_rank(&left.status)
                    .cmp(&task_list_status_rank(&right.status))
                    .then_with(|| right.created_at.cmp(&left.created_at))
                    .then_with(|| left.id.as_bytes().cmp(right.id.as_bytes()))
            });
        }
        "created_desc" | "newest" => {
            task_refs.sort_by(|left, right| {
                right
                    .created_at
                    .cmp(&left.created_at)
                    .then_with(|| left.id.as_bytes().cmp(right.id.as_bytes()))
            });
        }
        _ => {}
    }

    let task_infos: Vec<TaskInfo> = task_refs
        .iter()
        .skip(offset)
        .take(limit)
        .map(|t| TaskInfo {
            id: t.id.to_string(),
            description: if t.description == crate::storage::ENCRYPTED_STORAGE_UNAVAILABLE {
                "Older task details unavailable".to_string()
            } else {
                t.description.clone()
            },
            action: t.action.clone(),
            arguments: t.arguments.clone(),
            status: format!("{:?}", t.status),
            task_kind: task_kind(t).to_string(),
            task_kind_label: task_kind_label(t).to_string(),
            scheduled_for: t.scheduled_for.map(|value| value.to_rfc3339()),
            cron: t.cron.clone(),
            result: t.result.clone(),
            created_at: t.created_at.to_rfc3339(),
        })
        .collect();

    Json(
        serde_json::json!({ "tasks": task_infos, "total": total, "limit": limit, "offset": offset }),
    )
}

pub(super) fn automation_task_status_label(status: &TaskStatus) -> String {
    match status {
        TaskStatus::Pending => "pending".to_string(),
        TaskStatus::AwaitingApproval => "awaiting_approval".to_string(),
        TaskStatus::ExpiredNeedsReapproval => "expired_needs_reapproval".to_string(),
        TaskStatus::Paused => "paused".to_string(),
        TaskStatus::InProgress => "in_progress".to_string(),
        TaskStatus::Completed => "completed".to_string(),
        TaskStatus::Failed { .. } => "failed".to_string(),
        TaskStatus::Cancelled => "cancelled".to_string(),
    }
}

pub(super) fn automation_task_next_run_at(
    task: &Task,
    now: chrono::DateTime<chrono::Utc>,
) -> Option<String> {
    if let Some(scheduled_for) = task.scheduled_for {
        if task.cron.is_some() || scheduled_for >= now - chrono::Duration::seconds(5) {
            return Some(scheduled_for.to_rfc3339());
        }
    }
    let cron = task.cron.as_deref()?.trim();
    if cron.is_empty() {
        return None;
    }
    cron.parse::<cron::Schedule>()
        .ok()?
        .upcoming(chrono::Utc)
        .next()
        .map(|dt| dt.to_rfc3339())
}

pub(super) fn automation_watcher_status_label(
    status: &crate::core::watcher::WatcherStatus,
) -> String {
    match status {
        crate::core::watcher::WatcherStatus::Active => "active".to_string(),
        crate::core::watcher::WatcherStatus::Paused => "paused".to_string(),
        crate::core::watcher::WatcherStatus::Triggered => "triggered".to_string(),
        crate::core::watcher::WatcherStatus::TimedOut => "timed_out".to_string(),
        crate::core::watcher::WatcherStatus::Cancelled => "cancelled".to_string(),
        crate::core::watcher::WatcherStatus::Failed { .. } => "failed".to_string(),
    }
}

pub(super) fn automation_run_status_label(status: &crate::core::AutomationRunStatus) -> String {
    match status {
        crate::core::AutomationRunStatus::Running => "running".to_string(),
        crate::core::AutomationRunStatus::Succeeded => "succeeded".to_string(),
        crate::core::AutomationRunStatus::Failed => "failed".to_string(),
        crate::core::AutomationRunStatus::Retrying => "retrying".to_string(),
        crate::core::AutomationRunStatus::TimedOut => "timed_out".to_string(),
        crate::core::AutomationRunStatus::Triggered => "triggered".to_string(),
    }
}

pub(super) fn automation_watcher_condition_label(
    condition: &crate::core::watcher::WatchCondition,
) -> String {
    condition.summary()
}

pub(super) fn automation_watcher_next_run_at(
    watcher: &crate::core::watcher::Watcher,
) -> Option<String> {
    if !matches!(
        watcher.status,
        crate::core::watcher::WatcherStatus::Active | crate::core::watcher::WatcherStatus::Paused
    ) {
        return None;
    }
    let base = watcher.last_poll_at.unwrap_or(watcher.created_at);
    Some((base + chrono::Duration::seconds(watcher.interval_secs as i64)).to_rfc3339())
}

#[derive(Debug, Default)]
pub(super) struct BackgroundSessionCounts {
    tasks_total: usize,
    tasks_queued: usize,
    tasks_running: usize,
    tasks_waiting: usize,
    tasks_paused: usize,
    tasks_done: usize,
    tasks_failed: usize,
    tasks_cancelled: usize,
    watchers_total: usize,
    watchers_active: usize,
    watchers_paused: usize,
    watchers_triggered: usize,
    watchers_stopped: usize,
}

pub(super) fn parse_background_session_status(
    value: Option<&str>,
) -> Result<Option<crate::core::BackgroundSessionStatus>, String> {
    let Some(raw) = value else {
        return Ok(None);
    };
    let normalized = raw.trim().to_ascii_lowercase();
    if normalized.is_empty() {
        return Ok(None);
    }
    let status = match normalized.as_str() {
        "draft" => crate::core::BackgroundSessionStatus::Draft,
        "active" | "working" => crate::core::BackgroundSessionStatus::Active,
        "waiting" => crate::core::BackgroundSessionStatus::Waiting,
        "needs_input" | "needs-input" | "needsinput" => {
            crate::core::BackgroundSessionStatus::NeedsInput
        }
        "paused" => crate::core::BackgroundSessionStatus::Paused,
        "completed" | "done" => crate::core::BackgroundSessionStatus::Completed,
        "failed" | "error" => crate::core::BackgroundSessionStatus::Failed,
        "cancelled" | "canceled" | "stopped" => crate::core::BackgroundSessionStatus::Cancelled,
        _ => {
            return Err(format!(
                "Unsupported background session status: {}",
                raw.trim()
            ));
        }
    };
    Ok(Some(status))
}

pub(super) fn background_session_status_label(
    status: &crate::core::BackgroundSessionStatus,
) -> String {
    status.label().to_string()
}

pub(super) fn task_background_session_id(task: &Task) -> Option<String> {
    crate::core::background_session_id_from_automation(&task.arguments)
}

pub(super) fn watcher_background_session_id(
    watcher: &crate::core::watcher::Watcher,
) -> Option<String> {
    crate::core::background_session_id_from_automation(&watcher.poll_arguments)
}

pub(super) fn task_is_reminder(task: &Task) -> bool {
    crate::core::task_is_scheduled_reminder(task)
}

pub(super) fn task_kind(task: &Task) -> &'static str {
    if task_is_reminder(task) {
        "reminder"
    } else if task.action.eq_ignore_ascii_case("chat_request") {
        "chat_request"
    } else if task.action.eq_ignore_ascii_case("goal") {
        "goal"
    } else {
        "task"
    }
}

pub(super) fn task_kind_label(task: &Task) -> &'static str {
    match task_kind(task) {
        "reminder" => "Reminder",
        "chat_request" => "Chat Task",
        "goal" => "Goal",
        _ => "Task",
    }
}

pub(super) fn task_is_trivial_one_shot_reminder(task: &Task) -> bool {
    crate::core::task_is_one_shot_scheduled_reminder(task)
}

pub(super) fn background_session_has_linked_work(
    session: &crate::core::BackgroundSession,
    counts: &BackgroundSessionCounts,
) -> bool {
    counts.tasks_total > 0
        || counts.watchers_total > 0
        || !session.linked_task_ids.is_empty()
        || !session.linked_watcher_ids.is_empty()
}

fn background_session_task_is_linked(
    session_id: &str,
    session: &crate::core::BackgroundSession,
    task: &Task,
) -> bool {
    let task_id = task.id.to_string();
    session.linked_task_ids.iter().any(|id| id == &task_id)
        || task_background_session_id(task).as_deref() == Some(session_id)
}

fn background_session_has_visible_background_work(
    session_id: &str,
    session: &crate::core::BackgroundSession,
    counts: &BackgroundSessionCounts,
    tasks: &[Task],
) -> bool {
    if counts.watchers_total > 0 || !session.linked_watcher_ids.is_empty() {
        return true;
    }

    tasks
        .iter()
        .filter(|task| background_session_task_is_linked(session_id, session, task))
        .any(|task| {
            let kind = task_kind(task);
            kind != "chat_request" && kind != "goal" && !task_is_trivial_one_shot_reminder(task)
        })
}

pub(super) fn collect_background_session_counts(
    session_id: &str,
    session: &crate::core::BackgroundSession,
    tasks: &[Task],
    watchers: &[crate::core::watcher::Watcher],
) -> BackgroundSessionCounts {
    let mut counts = BackgroundSessionCounts::default();

    for task in tasks {
        let task_id = task.id.to_string();
        let is_linked = session.linked_task_ids.iter().any(|id| id == &task_id)
            || task_background_session_id(task).as_deref() == Some(session_id);
        if !is_linked {
            continue;
        }
        counts.tasks_total += 1;
        match &task.status {
            TaskStatus::Pending => counts.tasks_queued += 1,
            TaskStatus::AwaitingApproval | TaskStatus::ExpiredNeedsReapproval => {
                counts.tasks_waiting += 1
            }
            TaskStatus::Paused => counts.tasks_paused += 1,
            TaskStatus::InProgress => counts.tasks_running += 1,
            TaskStatus::Completed => counts.tasks_done += 1,
            TaskStatus::Failed { .. } => counts.tasks_failed += 1,
            TaskStatus::Cancelled => counts.tasks_cancelled += 1,
        }
    }

    for watcher in watchers {
        let watcher_id = watcher.id.to_string();
        let is_linked = session
            .linked_watcher_ids
            .iter()
            .any(|id| id == &watcher_id)
            || watcher_background_session_id(watcher).as_deref() == Some(session_id);
        if !is_linked {
            continue;
        }
        counts.watchers_total += 1;
        match &watcher.status {
            crate::core::watcher::WatcherStatus::Active => counts.watchers_active += 1,
            crate::core::watcher::WatcherStatus::Paused => counts.watchers_paused += 1,
            crate::core::watcher::WatcherStatus::Triggered => counts.watchers_triggered += 1,
            crate::core::watcher::WatcherStatus::TimedOut
            | crate::core::watcher::WatcherStatus::Cancelled
            | crate::core::watcher::WatcherStatus::Failed { .. } => counts.watchers_stopped += 1,
        }
    }

    counts
}

pub(super) fn background_session_ui_kind(
    session_id: &str,
    session: &crate::core::BackgroundSession,
    counts: &BackgroundSessionCounts,
    tasks: &[Task],
) -> &'static str {
    if !background_session_has_linked_work(session, counts) {
        return "unlinked_context";
    }
    if counts.tasks_total == 1
        && counts.watchers_total == 0
        && tasks.iter().any(|task| {
            background_session_task_is_linked(session_id, session, task)
                && task_is_trivial_one_shot_reminder(task)
        })
    {
        return "one_shot_reminder";
    }
    if !background_session_has_visible_background_work(session_id, session, counts, tasks) {
        return "chat_context";
    }
    "default"
}

pub(super) fn background_session_live_summary(
    session: &crate::core::BackgroundSession,
    counts: &BackgroundSessionCounts,
) -> String {
    if let Some(waiting_on) = session.waiting_on.as_deref() {
        if !waiting_on.trim().is_empty() {
            return waiting_on.trim().to_string();
        }
    }
    if let Some(current_focus) = session.current_focus.as_deref() {
        if !current_focus.trim().is_empty() {
            return current_focus.trim().to_string();
        }
    }
    match session.status {
        crate::core::BackgroundSessionStatus::Draft => {
            "Ready to attach work and begin in the background.".to_string()
        }
        crate::core::BackgroundSessionStatus::Active => {
            if counts.tasks_running > 0 {
                "Working through linked tasks right now.".to_string()
            } else if counts.watchers_active > 0 {
                "Watching for changes in the background.".to_string()
            } else if counts.tasks_queued > 0 {
                "Queued work is ready to run.".to_string()
            } else {
                "Standing by for the next step.".to_string()
            }
        }
        crate::core::BackgroundSessionStatus::Waiting => {
            "Waiting for an external signal or follow-up.".to_string()
        }
        crate::core::BackgroundSessionStatus::NeedsInput => {
            "Needs an operator decision before it can continue.".to_string()
        }
        crate::core::BackgroundSessionStatus::Paused => {
            "Paused. Linked work will stay idle until resumed.".to_string()
        }
        crate::core::BackgroundSessionStatus::Completed => {
            "Completed. No further work is scheduled.".to_string()
        }
        crate::core::BackgroundSessionStatus::Failed => {
            "Blocked by a failure. Inspect the latest error before resuming.".to_string()
        }
        crate::core::BackgroundSessionStatus::Cancelled => "Stopped by the operator.".to_string(),
    }
}

pub(super) fn background_session_counts_json(
    counts: &BackgroundSessionCounts,
) -> serde_json::Value {
    serde_json::json!({
        "tasks_total": counts.tasks_total,
        "tasks_queued": counts.tasks_queued,
        "tasks_running": counts.tasks_running,
        "tasks_waiting": counts.tasks_waiting,
        "tasks_paused": counts.tasks_paused,
        "tasks_done": counts.tasks_done,
        "tasks_failed": counts.tasks_failed,
        "tasks_cancelled": counts.tasks_cancelled,
        "watchers_total": counts.watchers_total,
        "watchers_active": counts.watchers_active,
        "watchers_paused": counts.watchers_paused,
        "watchers_triggered": counts.watchers_triggered,
        "watchers_stopped": counts.watchers_stopped,
    })
}

pub(super) fn background_session_list_item_json(
    session: &crate::core::BackgroundSession,
    counts: &BackgroundSessionCounts,
    tasks: &[Task],
) -> serde_json::Value {
    let ui_kind = background_session_ui_kind(&session.id, session, counts, tasks);
    serde_json::json!({
        "id": session.id.clone(),
        "title": session.title.clone(),
        "objective": session.objective.clone(),
        "status": background_session_status_label(&session.status),
        "summary": session.summary.clone(),
        "current_focus": session.current_focus.clone(),
        "waiting_on": session.waiting_on.clone(),
        "next_expected_action": session.next_expected_action.clone(),
        "last_error": session.last_error.clone(),
        "preferred_delivery_channel": session.preferred_delivery_channel.clone(),
        "channel": session.channel.clone(),
        "conversation_id": session.conversation_id.clone(),
        "linked_task_ids": session.linked_task_ids.clone(),
        "linked_watcher_ids": session.linked_watcher_ids.clone(),
        "created_at": session.created_at.to_rfc3339(),
        "updated_at": session.updated_at.to_rfc3339(),
        "last_activity_at": session.last_activity_at.to_rfc3339(),
        "live_summary": background_session_live_summary(session, counts),
        "counts": background_session_counts_json(counts),
        "ui_kind": ui_kind,
        "default_visible": ui_kind == "default",
    })
}

pub(super) fn background_session_task_json(task: &Task) -> serde_json::Value {
    serde_json::json!({
        "id": task.id.to_string(),
        "description": task.description.clone(),
        "action": task.action.clone(),
        "task_kind": task_kind(task),
        "task_kind_label": task_kind_label(task),
        "status": automation_task_status_label(&task.status),
        "created_at": task.created_at.to_rfc3339(),
        "scheduled_for": task.scheduled_for.map(|value| value.to_rfc3339()),
        "cron": task.cron.clone(),
        "result": task.result.clone(),
    })
}

pub(super) fn background_session_watcher_json(
    watcher: &crate::core::watcher::Watcher,
) -> serde_json::Value {
    serde_json::json!({
        "id": watcher.id.to_string(),
        "description": watcher.description.clone(),
        "poll_action": watcher.poll_action.clone(),
        "status": automation_watcher_status_label(&watcher.status),
        "created_at": watcher.created_at.to_rfc3339(),
        "last_poll_at": watcher.last_poll_at.map(|value| value.to_rfc3339()),
        "poll_count": watcher.poll_count,
        "notify_channel": watcher.notify_channel.clone(),
        "repeat_on_match": watcher.repeat_on_match,
        "last_error": watcher.last_error.clone(),
        "last_poll_outcome": watcher.last_poll_outcome.clone(),
        "notification_attempts": watcher.notification_attempts.clone(),
        "trigger_result": watcher.trigger_result.clone(),
    })
}

pub(super) async fn rebind_task_background_session(
    state: &AppState,
    task_id: &str,
    session_id: Option<&str>,
) -> Result<bool, String> {
    let uuid = uuid::Uuid::parse_str(task_id.trim())
        .map_err(|_| format!("Invalid task id: {}", task_id.trim()))?;

    let serialized_arguments = {
        let mut tasks = state.tasks.write().await;
        let Some(task) = tasks.get_mut(uuid) else {
            return Ok(false);
        };
        task.arguments =
            crate::core::set_background_session_id_in_automation(&task.arguments, session_id);
        serde_json::to_string(&task.arguments)
            .map_err(|error| format!("Failed to serialize task arguments: {}", error))?
    };

    let agent = state.agent.read().await;
    agent
        .storage
        .update_task(task_id.trim(), None, Some(serialized_arguments), None, None)
        .await
        .map_err(|error| format!("Failed to update task linkage: {}", error))?;
    Ok(true)
}

pub(super) async fn rebind_watcher_background_session(
    state: &AppState,
    watcher_id: &str,
    session_id: Option<&str>,
) -> Result<bool, String> {
    let uuid = uuid::Uuid::parse_str(watcher_id.trim())
        .map_err(|_| format!("Invalid watcher id: {}", watcher_id.trim()))?;
    let agent = state.agent.read().await;
    Ok(agent
        .watcher_manager
        .set_background_session_id(uuid, session_id)
        .await)
}

async fn delete_watcher_entity(state: &AppState, watcher_id: &str) -> Result<bool, String> {
    let trimmed = watcher_id.trim();
    let uuid =
        uuid::Uuid::parse_str(trimmed).map_err(|_| format!("Invalid watcher id: {}", trimmed))?;
    let agent = state.agent.read().await;
    let deleted_live = agent.watcher_manager.delete(uuid).await;
    let deleted_history = agent.clear_watcher_supervisor_state(trimmed).await;
    let task_ids: Vec<String> = Vec::new();
    let watcher_ids = vec![trimmed.to_string()];
    agent
        .background_sessions
        .remove_child_references(&task_ids, &watcher_ids, Some("api"))
        .await;
    Ok(deleted_live || deleted_history)
}

pub(super) async fn pause_linked_background_session_work(
    state: &AppState,
    session: &crate::core::BackgroundSession,
) -> anyhow::Result<(usize, usize)> {
    let linked_task_ids: HashSet<String> = session.linked_task_ids.iter().cloned().collect();
    let mut paused_tasks = Vec::new();
    {
        let snapshot = state.tasks.read().await.all().to_vec();
        let mut tasks = state.tasks.write().await;
        for task in snapshot {
            let task_id = task.id.to_string();
            if !(linked_task_ids.contains(&task_id)
                || task_background_session_id(&task).as_deref() == Some(session.id.as_str()))
            {
                continue;
            }
            if let Some(inner) = tasks.get_mut(task.id) {
                if matches!(
                    inner.status,
                    TaskStatus::Pending | TaskStatus::AwaitingApproval
                ) {
                    inner.status = TaskStatus::Paused;
                    paused_tasks.push(task_id);
                }
            }
        }
    }

    if !paused_tasks.is_empty() {
        let agent = state.agent.read().await;
        for task_id in &paused_tasks {
            agent
                .storage
                .update_task_status(task_id, "\"Paused\"")
                .await?;
        }
    }

    let mut paused_watchers = 0usize;
    {
        let agent = state.agent.read().await;
        for watcher_id in &session.linked_watcher_ids {
            if let Ok(uuid) = uuid::Uuid::parse_str(watcher_id) {
                if agent.watcher_manager.pause(uuid).await {
                    paused_watchers += 1;
                    if let Some(watcher) = agent.watcher_manager.get(uuid).await {
                        agent
                            .sync_watcher_supervisor_state(&watcher, Some("paused"), None)
                            .await;
                    }
                }
            }
        }
    }

    Ok((paused_tasks.len(), paused_watchers))
}

pub(super) async fn resume_linked_background_session_work(
    state: &AppState,
    session: &crate::core::BackgroundSession,
) -> anyhow::Result<(usize, usize)> {
    let linked_task_ids: HashSet<String> = session.linked_task_ids.iter().cloned().collect();
    let mut resumed_tasks = Vec::new();
    let mut rescheduled_tasks: Vec<(String, String)> = Vec::new();
    {
        let snapshot = state.tasks.read().await.all().to_vec();
        let mut tasks = state.tasks.write().await;
        for task in snapshot {
            let task_id = task.id.to_string();
            if !(linked_task_ids.contains(&task_id)
                || task_background_session_id(&task).as_deref() == Some(session.id.as_str()))
            {
                continue;
            }
            if let Some(inner) = tasks.get_mut(task.id) {
                if inner.action == "chat_request" {
                    continue;
                }
                if matches!(inner.status, TaskStatus::Paused) {
                    inner.status = TaskStatus::Pending;
                    let now = chrono::Utc::now();
                    if inner.cron.is_some()
                        || inner
                            .scheduled_for
                            .as_ref()
                            .map(|dt| *dt <= now)
                            .unwrap_or(false)
                    {
                        inner.scheduled_for = Some(now);
                        rescheduled_tasks.push((task_id.clone(), now.to_rfc3339()));
                    }
                    resumed_tasks.push(task_id);
                }
            }
        }
    }

    if !resumed_tasks.is_empty() {
        let agent = state.agent.read().await;
        for task_id in &resumed_tasks {
            agent
                .storage
                .update_task_status(task_id, "\"Pending\"")
                .await?;
        }
        for (task_id, scheduled_for) in &rescheduled_tasks {
            agent
                .storage
                .update_task(task_id, None, None, None, Some(scheduled_for.clone()))
                .await?;
        }
    }

    let mut resumed_watchers = 0usize;
    {
        let agent = state.agent.read().await;
        for watcher_id in &session.linked_watcher_ids {
            if let Ok(uuid) = uuid::Uuid::parse_str(watcher_id) {
                if agent.watcher_manager.resume(uuid).await {
                    resumed_watchers += 1;
                    if let Some(watcher) = agent.watcher_manager.get(uuid).await {
                        agent
                            .sync_watcher_supervisor_state(&watcher, Some("active"), None)
                            .await;
                    }
                }
            }
        }
    }

    Ok((resumed_tasks.len(), resumed_watchers))
}

pub(super) async fn cancel_linked_background_session_work(
    state: &AppState,
    session: &crate::core::BackgroundSession,
) -> anyhow::Result<(usize, usize)> {
    let linked_task_ids: HashSet<String> = session.linked_task_ids.iter().cloned().collect();
    let mut cancelled_tasks = Vec::new();
    {
        let snapshot = state.tasks.read().await.all().to_vec();
        let mut tasks = state.tasks.write().await;
        for task in snapshot {
            let task_id = task.id.to_string();
            if !(linked_task_ids.contains(&task_id)
                || task_background_session_id(&task).as_deref() == Some(session.id.as_str()))
            {
                continue;
            }
            if let Some(inner) = tasks.get_mut(task.id) {
                if matches!(
                    inner.status,
                    TaskStatus::Pending
                        | TaskStatus::AwaitingApproval
                        | TaskStatus::Paused
                        | TaskStatus::InProgress
                ) {
                    inner.status = TaskStatus::Cancelled;
                    cancelled_tasks.push(task_id);
                }
            }
        }
    }

    if !cancelled_tasks.is_empty() {
        let agent = state.agent.read().await;
        for task_id in &cancelled_tasks {
            agent
                .storage
                .update_task_status(task_id, "\"Cancelled\"")
                .await?;
            signal_chat_task_cancellation(state, task_id).await;
        }
    }

    let mut cancelled_watchers = 0usize;
    {
        let agent = state.agent.read().await;
        for watcher_id in &session.linked_watcher_ids {
            if let Ok(uuid) = uuid::Uuid::parse_str(watcher_id) {
                if agent.watcher_manager.cancel(uuid).await {
                    cancelled_watchers += 1;
                    if let Some(watcher) = agent.watcher_manager.get(uuid).await {
                        agent
                            .sync_watcher_supervisor_state(&watcher, Some("cancelled"), None)
                            .await;
                    }
                }
            }
        }
    }

    Ok((cancelled_tasks.len(), cancelled_watchers))
}

pub(super) async fn update_linked_background_session_delivery(
    state: &AppState,
    session: &crate::core::BackgroundSession,
    channel: &str,
) -> anyhow::Result<(usize, usize)> {
    let channel = channel.trim();
    if channel.is_empty() {
        return Ok((0, 0));
    }

    let linked_task_ids: HashSet<String> = session.linked_task_ids.iter().cloned().collect();
    let mut task_updates = Vec::new();
    {
        let snapshot = state.tasks.read().await.all().to_vec();
        let mut tasks = state.tasks.write().await;
        for task in snapshot {
            let task_id = task.id.to_string();
            if !(linked_task_ids.contains(&task_id)
                || task_background_session_id(&task).as_deref() == Some(session.id.as_str()))
            {
                continue;
            }
            let Some(inner) = tasks.get_mut(task.id) else {
                continue;
            };
            let Some(arguments) = inner.arguments.as_object_mut() else {
                continue;
            };
            if arguments
                .get("report_to")
                .and_then(|value| value.as_str())
                .map(|value| value == channel)
                .unwrap_or(false)
            {
                continue;
            }
            arguments.insert(
                "report_to".to_string(),
                serde_json::Value::String(channel.to_string()),
            );
            let serialized = serde_json::to_string(&inner.arguments)?;
            task_updates.push((task_id, serialized));
        }
    }

    if !task_updates.is_empty() {
        let agent = state.agent.read().await;
        for (task_id, serialized_arguments) in &task_updates {
            agent
                .storage
                .update_task(
                    task_id,
                    None,
                    Some(serialized_arguments.clone()),
                    None,
                    None,
                )
                .await?;
        }
    }

    let mut watcher_ids: HashSet<String> = session.linked_watcher_ids.iter().cloned().collect();
    {
        let agent = state.agent.read().await;
        for watcher in agent.watcher_manager.list().await {
            if watcher_background_session_id(&watcher).as_deref() == Some(session.id.as_str()) {
                watcher_ids.insert(watcher.id.to_string());
            }
        }
    }

    let mut watcher_updates = 0usize;
    {
        let agent = state.agent.read().await;
        for watcher_id in watcher_ids {
            if let Ok(uuid) = uuid::Uuid::parse_str(&watcher_id) {
                if agent
                    .watcher_manager
                    .set_notify_channel(uuid, channel)
                    .await
                {
                    watcher_updates += 1;
                    if let Some(watcher) = agent.watcher_manager.get(uuid).await {
                        agent
                            .sync_watcher_supervisor_state(&watcher, None, None)
                            .await;
                    }
                }
            }
        }
    }

    Ok((task_updates.len(), watcher_updates))
}

pub(super) async fn list_background_sessions(
    State(state): State<AppState>,
) -> Json<serde_json::Value> {
    let tasks = { state.tasks.read().await.all().to_vec() };
    let (sessions, watchers) = {
        let agent = state.agent.read().await;
        (
            agent.background_sessions.list().await,
            agent.watcher_manager.list().await,
        )
    };

    let session_items: Vec<_> = sessions
        .iter()
        .map(|session| {
            let counts = collect_background_session_counts(&session.id, session, &tasks, &watchers);
            background_session_list_item_json(session, &counts, &tasks)
        })
        .collect();

    Json(serde_json::json!({
        "sessions": session_items,
        "total": session_items.len(),
    }))
}

pub(super) async fn create_background_session(
    State(state): State<AppState>,
    Json(request): Json<CreateBackgroundSessionRequest>,
) -> Response {
    let objective = request.objective.trim();
    if objective.is_empty() {
        return (
            StatusCode::BAD_REQUEST,
            Json(ErrorResponse {
                error: "Objective is required.".to_string(),
            }),
        )
            .into_response();
    }

    let session = {
        let agent = state.agent.read().await;
        agent
            .background_sessions
            .create(
                crate::core::BackgroundSessionCreate {
                    title: request.title.clone(),
                    objective: request.objective.clone(),
                    summary: request.summary.clone(),
                    current_focus: request.current_focus.clone(),
                    waiting_on: request.waiting_on.clone(),
                    next_expected_action: request.next_expected_action.clone(),
                    working_memory: request.working_memory.clone(),
                    preferred_delivery_channel: request.preferred_delivery_channel.clone(),
                    channel: request.channel.clone(),
                    conversation_id: request.conversation_id.clone(),
                    project_id: None,
                    task_ids: Vec::new(),
                    watcher_ids: Vec::new(),
                    policy: Default::default(),
                },
                Some("api"),
            )
            .await
    };

    if !request.task_ids.is_empty() || !request.watcher_ids.is_empty() {
        let mut linked_task_ids = Vec::new();
        let mut linked_watcher_ids = Vec::new();

        for task_id in &request.task_ids {
            match rebind_task_background_session(&state, task_id, Some(&session.id)).await {
                Ok(true) => linked_task_ids.push(task_id.trim().to_string()),
                Ok(false) => {}
                Err(error) => {
                    return (StatusCode::BAD_REQUEST, Json(ErrorResponse { error }))
                        .into_response();
                }
            }
        }
        for watcher_id in &request.watcher_ids {
            match rebind_watcher_background_session(&state, watcher_id, Some(&session.id)).await {
                Ok(true) => linked_watcher_ids.push(watcher_id.trim().to_string()),
                Ok(false) => {}
                Err(error) => {
                    return (StatusCode::BAD_REQUEST, Json(ErrorResponse { error }))
                        .into_response();
                }
            }
        }

        let agent = state.agent.read().await;
        agent
            .background_sessions
            .remove_child_references(&linked_task_ids, &linked_watcher_ids, Some("api"))
            .await;
        let _ = agent
            .background_sessions
            .attach_items(
                &session.id,
                &linked_task_ids,
                &linked_watcher_ids,
                Some("api"),
            )
            .await;
    }

    spawn_autonomy_analysis_tick(state.agent.clone(), "background_session_created");
    (
        StatusCode::OK,
        Json(serde_json::json!({
            "status": "ok",
            "id": session.id,
        })),
    )
        .into_response()
}

pub(super) async fn get_background_session(
    State(state): State<AppState>,
    Path(id): Path<String>,
) -> Response {
    let tasks = { state.tasks.read().await.all().to_vec() };
    let (session, watchers, runs) = {
        let agent = state.agent.read().await;
        (
            agent.background_sessions.get(&id).await,
            agent.watcher_manager.list().await,
            crate::core::list_automation_runs(&agent.storage, 80)
                .await
                .unwrap_or_default(),
        )
    };
    let Some(session) = session else {
        return (
            StatusCode::NOT_FOUND,
            Json(ErrorResponse {
                error: "Background session not found.".to_string(),
            }),
        )
            .into_response();
    };

    let counts = collect_background_session_counts(&session.id, &session, &tasks, &watchers);
    let linked_task_id_set: HashSet<String> = session.linked_task_ids.iter().cloned().collect();
    let linked_watcher_id_set: HashSet<String> =
        session.linked_watcher_ids.iter().cloned().collect();

    let linked_tasks: Vec<_> = tasks
        .iter()
        .filter(|task| {
            linked_task_id_set.contains(&task.id.to_string())
                || task_background_session_id(task).as_deref() == Some(session.id.as_str())
        })
        .map(background_session_task_json)
        .collect();

    let linked_watchers: Vec<_> = watchers
        .iter()
        .filter(|watcher| {
            linked_watcher_id_set.contains(&watcher.id.to_string())
                || watcher_background_session_id(watcher).as_deref() == Some(session.id.as_str())
        })
        .map(background_session_watcher_json)
        .collect();

    let run_target_ids: HashSet<String> = session
        .linked_task_ids
        .iter()
        .chain(session.linked_watcher_ids.iter())
        .cloned()
        .collect();
    let recent_runs: Vec<_> = runs
        .into_iter()
        .filter(|run| run_target_ids.contains(&run.automation_id))
        .take(24)
        .map(|run| {
            serde_json::json!({
                "id": run.id.clone(),
                "automation_id": run.automation_id.clone(),
                "kind": run.automation_kind.clone(),
                "title": run.title.clone(),
                "action": run.action.clone(),
                "trigger": run.trigger.clone(),
                "status": automation_run_status_label(&run.status),
                "attempt": run.attempt,
                "started_at": run.started_at.clone(),
                "completed_at": run.completed_at.clone(),
                "duration_ms": run.duration_ms,
                "summary": run.critique.summary.clone(),
                "output_preview": run.output_preview.clone(),
                "error": run.error.clone(),
                "next_retry_at": run.next_retry_at.clone(),
            })
        })
        .collect();

    let live_task_ids: HashSet<String> = linked_tasks
        .iter()
        .filter_map(|item| {
            item.get("id")
                .and_then(|value| value.as_str())
                .map(|value| value.to_string())
        })
        .collect();
    let live_watcher_ids: HashSet<String> = linked_watchers
        .iter()
        .filter_map(|item| {
            item.get("id")
                .and_then(|value| value.as_str())
                .map(|value| value.to_string())
        })
        .collect();
    let missing_task_ids: Vec<_> = session
        .linked_task_ids
        .iter()
        .filter(|task_id| !live_task_ids.contains(*task_id))
        .cloned()
        .collect();
    let missing_watcher_ids: Vec<_> = session
        .linked_watcher_ids
        .iter()
        .filter(|watcher_id| !live_watcher_ids.contains(*watcher_id))
        .cloned()
        .collect();

    (
        StatusCode::OK,
        Json(serde_json::json!({
            "session": background_session_list_item_json(&session, &counts, &tasks),
            "session_detail": {
                "working_memory": session.working_memory.clone(),
                "channel": session.channel.clone(),
                "conversation_id": session.conversation_id.clone(),
                "events": session.events.iter().map(|event| serde_json::json!({
                    "id": event.id.clone(),
                    "at": event.at.to_rfc3339(),
                    "kind": event.kind.clone(),
                    "summary": event.summary.clone(),
                    "detail": event.detail.clone(),
                    "actor": event.actor.clone(),
                })).collect::<Vec<_>>(),
            },
            "linked_tasks": linked_tasks,
            "linked_watchers": linked_watchers,
            "recent_runs": recent_runs,
            "missing_links": {
                "task_ids": missing_task_ids,
                "watcher_ids": missing_watcher_ids,
            }
        })),
    )
        .into_response()
}

pub(super) async fn update_background_session(
    State(state): State<AppState>,
    Path(id): Path<String>,
    Json(request): Json<UpdateBackgroundSessionRequest>,
) -> Response {
    let requested_delivery_channel = request
        .preferred_delivery_channel
        .as_deref()
        .map(str::trim)
        .filter(|value| !value.is_empty())
        .map(ToOwned::to_owned);
    let status = match parse_background_session_status(request.status.as_deref()) {
        Ok(status) => status,
        Err(error) => {
            return (StatusCode::BAD_REQUEST, Json(ErrorResponse { error })).into_response();
        }
    };

    let updated = {
        let agent = state.agent.read().await;
        agent
            .background_sessions
            .update(
                &id,
                crate::core::BackgroundSessionUpdate {
                    title: request.title,
                    objective: request.objective,
                    status,
                    summary: request.summary,
                    current_focus: request.current_focus,
                    waiting_on: request.waiting_on,
                    next_expected_action: request.next_expected_action,
                    working_memory: request.working_memory,
                    last_error: request.last_error,
                    preferred_delivery_channel: request.preferred_delivery_channel,
                    policy: request.policy,
                },
                Some("api"),
            )
            .await
    };

    let Some(updated_session) = updated else {
        return (
            StatusCode::NOT_FOUND,
            Json(ErrorResponse {
                error: "Background session not found.".to_string(),
            }),
        )
            .into_response();
    };

    let mut delivery_task_updates = 0usize;
    let mut delivery_watcher_updates = 0usize;
    if let Some(channel) = requested_delivery_channel.as_deref() {
        match update_linked_background_session_delivery(&state, &updated_session, channel).await {
            Ok((task_count, watcher_count)) => {
                delivery_task_updates = task_count;
                delivery_watcher_updates = watcher_count;
            }
            Err(error) => {
                return (
                    StatusCode::INTERNAL_SERVER_ERROR,
                    Json(ErrorResponse {
                        error: format!("Failed to update linked notification delivery: {error}"),
                    }),
                )
                    .into_response();
            }
        }
    }

    spawn_autonomy_analysis_tick(state.agent.clone(), "background_session_updated");
    (
        StatusCode::OK,
        Json(serde_json::json!({
            "status": "ok",
            "delivery_task_updates": delivery_task_updates,
            "delivery_watcher_updates": delivery_watcher_updates,
        })),
    )
        .into_response()
}

pub(super) async fn attach_background_session_work(
    State(state): State<AppState>,
    Path(id): Path<String>,
    Json(request): Json<BackgroundSessionLinkRequest>,
) -> Response {
    let session_exists = {
        let agent = state.agent.read().await;
        agent.background_sessions.get(&id).await.is_some()
    };
    if !session_exists {
        return (
            StatusCode::NOT_FOUND,
            Json(ErrorResponse {
                error: "Background session not found.".to_string(),
            }),
        )
            .into_response();
    }

    let mut linked_task_ids = Vec::new();
    let mut linked_watcher_ids = Vec::new();

    for task_id in &request.task_ids {
        match rebind_task_background_session(&state, task_id, Some(&id)).await {
            Ok(true) => linked_task_ids.push(task_id.trim().to_string()),
            Ok(false) => {}
            Err(error) => {
                return (StatusCode::BAD_REQUEST, Json(ErrorResponse { error })).into_response();
            }
        }
    }
    for watcher_id in &request.watcher_ids {
        match rebind_watcher_background_session(&state, watcher_id, Some(&id)).await {
            Ok(true) => linked_watcher_ids.push(watcher_id.trim().to_string()),
            Ok(false) => {}
            Err(error) => {
                return (StatusCode::BAD_REQUEST, Json(ErrorResponse { error })).into_response();
            }
        }
    }

    let attached = {
        let agent = state.agent.read().await;
        agent
            .background_sessions
            .remove_child_references(&linked_task_ids, &linked_watcher_ids, Some("api"))
            .await;
        agent
            .background_sessions
            .attach_items(&id, &linked_task_ids, &linked_watcher_ids, Some("api"))
            .await
    };

    if attached.is_none() {
        return (
            StatusCode::NOT_FOUND,
            Json(ErrorResponse {
                error: "Background session not found.".to_string(),
            }),
        )
            .into_response();
    }

    spawn_autonomy_analysis_tick(state.agent.clone(), "background_session_attached");
    (
        StatusCode::OK,
        Json(serde_json::json!({
            "status": "ok",
            "linked_task_ids": linked_task_ids,
            "linked_watcher_ids": linked_watcher_ids,
        })),
    )
        .into_response()
}

pub(super) async fn detach_background_session_work(
    State(state): State<AppState>,
    Path(id): Path<String>,
    Json(request): Json<BackgroundSessionLinkRequest>,
) -> Response {
    let session_exists = {
        let agent = state.agent.read().await;
        agent.background_sessions.get(&id).await.is_some()
    };
    if !session_exists {
        return (
            StatusCode::NOT_FOUND,
            Json(ErrorResponse {
                error: "Background session not found.".to_string(),
            }),
        )
            .into_response();
    }

    let mut detached_task_ids = Vec::new();
    let mut detached_watcher_ids = Vec::new();

    for task_id in &request.task_ids {
        match rebind_task_background_session(&state, task_id, None).await {
            Ok(true) => detached_task_ids.push(task_id.trim().to_string()),
            Ok(false) => {}
            Err(error) => {
                return (StatusCode::BAD_REQUEST, Json(ErrorResponse { error })).into_response();
            }
        }
    }
    for watcher_id in &request.watcher_ids {
        match delete_watcher_entity(&state, watcher_id).await {
            Ok(_) => detached_watcher_ids.push(watcher_id.trim().to_string()),
            Err(error) => {
                return (StatusCode::BAD_REQUEST, Json(ErrorResponse { error })).into_response();
            }
        }
    }

    let detached = {
        let agent = state.agent.read().await;
        agent
            .background_sessions
            .detach_items(&id, &detached_task_ids, &detached_watcher_ids, Some("api"))
            .await
    };

    if detached.is_none() {
        return (
            StatusCode::NOT_FOUND,
            Json(ErrorResponse {
                error: "Background session not found.".to_string(),
            }),
        )
            .into_response();
    }

    spawn_autonomy_analysis_tick(state.agent.clone(), "background_session_detached");
    (
        StatusCode::OK,
        Json(serde_json::json!({
            "status": "ok",
            "detached_task_ids": detached_task_ids,
            "detached_watcher_ids": detached_watcher_ids,
            "deleted_watcher_ids": detached_watcher_ids,
        })),
    )
        .into_response()
}

pub(super) async fn pause_background_session(
    State(state): State<AppState>,
    Path(id): Path<String>,
) -> Response {
    let session = {
        let agent = state.agent.read().await;
        agent.background_sessions.get(&id).await
    };
    let Some(session) = session else {
        return (
            StatusCode::NOT_FOUND,
            Json(ErrorResponse {
                error: "Background session not found.".to_string(),
            }),
        )
            .into_response();
    };

    let (paused_tasks, paused_watchers) =
        match pause_linked_background_session_work(&state, &session).await {
            Ok(result) => result,
            Err(error) => {
                return (
                    StatusCode::INTERNAL_SERVER_ERROR,
                    Json(ErrorResponse {
                        error: format!("Failed to pause background session work: {}", error),
                    }),
                )
                    .into_response();
            }
        };

    {
        let agent = state.agent.read().await;
        let _ = agent
            .background_sessions
            .set_status(
                &id,
                crate::core::BackgroundSessionStatus::Paused,
                "Background session paused.",
                Some("api"),
            )
            .await;
    }

    spawn_autonomy_analysis_tick(state.agent.clone(), "background_session_paused");
    (
        StatusCode::OK,
        Json(serde_json::json!({
            "status": "ok",
            "paused_tasks": paused_tasks,
            "paused_watchers": paused_watchers,
        })),
    )
        .into_response()
}

pub(super) async fn resume_background_session(
    State(state): State<AppState>,
    Path(id): Path<String>,
) -> Response {
    let session = {
        let agent = state.agent.read().await;
        agent.background_sessions.get(&id).await
    };
    let Some(session) = session else {
        return (
            StatusCode::NOT_FOUND,
            Json(ErrorResponse {
                error: "Background session not found.".to_string(),
            }),
        )
            .into_response();
    };

    let (resumed_tasks, resumed_watchers) =
        match resume_linked_background_session_work(&state, &session).await {
            Ok(result) => result,
            Err(error) => {
                return (
                    StatusCode::INTERNAL_SERVER_ERROR,
                    Json(ErrorResponse {
                        error: format!("Failed to resume background session work: {}", error),
                    }),
                )
                    .into_response();
            }
        };

    {
        let agent = state.agent.read().await;
        let _ = agent
            .background_sessions
            .set_status(
                &id,
                crate::core::BackgroundSessionStatus::Active,
                "Background session resumed.",
                Some("api"),
            )
            .await;
    }

    spawn_autonomy_analysis_tick(state.agent.clone(), "background_session_resumed");
    (
        StatusCode::OK,
        Json(serde_json::json!({
            "status": "ok",
            "resumed_tasks": resumed_tasks,
            "resumed_watchers": resumed_watchers,
        })),
    )
        .into_response()
}

pub(super) async fn cancel_background_session(
    State(state): State<AppState>,
    Path(id): Path<String>,
) -> Response {
    let session = {
        let agent = state.agent.read().await;
        agent.background_sessions.get(&id).await
    };
    let Some(session) = session else {
        return (
            StatusCode::NOT_FOUND,
            Json(ErrorResponse {
                error: "Background session not found.".to_string(),
            }),
        )
            .into_response();
    };

    let (cancelled_tasks, cancelled_watchers) =
        match cancel_linked_background_session_work(&state, &session).await {
            Ok(result) => result,
            Err(error) => {
                return (
                    StatusCode::INTERNAL_SERVER_ERROR,
                    Json(ErrorResponse {
                        error: format!("Failed to stop background session work: {}", error),
                    }),
                )
                    .into_response();
            }
        };

    {
        let agent = state.agent.read().await;
        let _ = agent
            .background_sessions
            .set_status(
                &id,
                crate::core::BackgroundSessionStatus::Cancelled,
                "Background session stopped.",
                Some("api"),
            )
            .await;
    }

    spawn_autonomy_analysis_tick(state.agent.clone(), "background_session_cancelled");
    (
        StatusCode::OK,
        Json(serde_json::json!({
            "status": "ok",
            "cancelled_tasks": cancelled_tasks,
            "cancelled_watchers": cancelled_watchers,
        })),
    )
        .into_response()
}

pub(super) async fn delete_background_session(
    State(state): State<AppState>,
    Path(id): Path<String>,
) -> Response {
    let removed = {
        let agent = state.agent.read().await;
        agent.background_sessions.delete(&id).await
    };
    let Some(session) = removed else {
        return (
            StatusCode::NOT_FOUND,
            Json(ErrorResponse {
                error: "Background session not found.".to_string(),
            }),
        )
            .into_response();
    };

    for task_id in &session.linked_task_ids {
        if let Err(error) = rebind_task_background_session(&state, task_id, None).await {
            tracing::warn!(
                "Failed to unlink task {} from deleted background session {}: {}",
                task_id,
                id,
                error
            );
        }
    }
    for watcher_id in &session.linked_watcher_ids {
        match delete_watcher_entity(&state, watcher_id).await {
            Ok(false) => {
                tracing::debug!(
                    "Deleted background session {} referenced missing watcher {}",
                    id,
                    watcher_id
                );
            }
            Ok(true) => {}
            Err(error) => {
                tracing::warn!(
                    "Failed to delete watcher {} from deleted background session {}: {}",
                    watcher_id,
                    id,
                    error
                );
            }
        }
    }

    spawn_autonomy_analysis_tick(state.agent.clone(), "background_session_deleted");
    (StatusCode::OK, Json(serde_json::json!({ "status": "ok" }))).into_response()
}

pub(super) async fn list_automation_objects(
    State(state): State<AppState>,
) -> Json<serde_json::Value> {
    let now = chrono::Utc::now();
    let mut objects: Vec<AutomationObjectInfo> = Vec::new();
    let mut totals = AutomationInventoryTotals::default();

    {
        let tasks = state.tasks.read().await;
        for task in tasks.all() {
            if task.action == "goal" {
                continue;
            }
            totals.tasks += 1;
            let task_kind = task_kind(task);
            let detail = task
                .result
                .as_ref()
                .and_then(|result| {
                    let trimmed = result.trim();
                    if trimmed.is_empty() {
                        None
                    } else {
                        Some(trimmed.chars().take(140).collect::<String>())
                    }
                })
                .or_else(|| {
                    task.scheduled_for.map(|scheduled_for| {
                        format!(
                            "Scheduled for {}",
                            scheduled_for.format("%Y-%m-%d %H:%M UTC")
                        )
                    })
                })
                .or_else(|| {
                    task.cron
                        .as_ref()
                        .map(|cron| format!("Recurring schedule: {}", cron))
                });
            objects.push(AutomationObjectInfo {
                id: task.id.to_string(),
                kind: "task".to_string(),
                title: task.description.clone(),
                subtitle: Some(if task_kind == "task" {
                    task.action.clone()
                } else {
                    task_kind_label(task).to_string()
                }),
                status: automation_task_status_label(&task.status),
                detail,
                created_at: Some(task.created_at.to_rfc3339()),
                next_run_at: automation_task_next_run_at(task, now),
                view: "tasks".to_string(),
                url: None,
                enabled: None,
                connected: None,
            });
        }
    }

    let (config_dir, data_dir, integrations_info, watchers) = {
        let agent = state.agent.read().await;
        (
            agent.config_dir.clone(),
            agent.data_dir.clone(),
            agent.integrations.list().await,
            agent.watcher_manager.list().await,
        )
    };

    for watcher in &watchers {
        totals.watchers += 1;
        let detail = if let Some(trigger_result) = watcher.trigger_result.as_ref() {
            let preview = trigger_result.chars().take(140).collect::<String>();
            Some(format!(
                "{} | Trigger result: {}",
                automation_watcher_condition_label(&watcher.condition),
                preview
            ))
        } else {
            Some(automation_watcher_condition_label(&watcher.condition))
        };
        objects.push(AutomationObjectInfo {
            id: watcher.id.to_string(),
            kind: "watcher".to_string(),
            title: watcher.description.clone(),
            subtitle: Some(watcher.poll_action.clone()),
            status: automation_watcher_status_label(&watcher.status),
            detail,
            created_at: Some(watcher.created_at.to_rfc3339()),
            next_run_at: automation_watcher_next_run_at(watcher),
            view: "watchers".to_string(),
            url: None,
            enabled: None,
            connected: None,
        });
    }

    let manager =
        crate::core::config::SecureConfigManager::new_with_data_dir(&config_dir, Some(&data_dir))
            .ok();
    for info in integrations_info {
        if integrations::external_integration_config(&info.id).is_none() {
            continue;
        }
        let (status, detail) = if info.id == "google_calendar" {
            let configured = integrations::calendar_oauth_pair(manager.as_ref()).is_some();
            let has_refresh_token = integrations::oauth_has_refresh_token(
                integrations::stored_secret(manager.as_ref(), "calendar_tokens"),
            );
            if has_refresh_token {
                match integrations::validate_calendar_oauth_connection(&config_dir).await {
                    Ok(()) => ("connected".to_string(), None),
                    Err(error) => ("error".to_string(), Some(error)),
                }
            } else if configured {
                (
                    "needs_auth".to_string(),
                    Some("Google sign-in required to finish connecting Calendar.".to_string()),
                )
            } else {
                ("not_configured".to_string(), None)
            }
        } else {
            match info.status {
                crate::integrations::IntegrationStatus::NotConfigured => {
                    ("not_configured".to_string(), None)
                }
                crate::integrations::IntegrationStatus::NeedsAuth => {
                    ("needs_auth".to_string(), None)
                }
                crate::integrations::IntegrationStatus::Connected => {
                    ("connected".to_string(), None)
                }
                crate::integrations::IntegrationStatus::Error(error) => {
                    ("error".to_string(), Some(error))
                }
            }
        };
        let enabled = manager
            .as_ref()
            .and_then(|m| {
                m.get_custom_secret(&integrations::integration_enabled_key(&info.id))
                    .ok()
                    .flatten()
            })
            .and_then(|v| integrations::parse_boolish(&v))
            .unwrap_or(status == "connected");
        totals.integrations += 1;
        objects.push(AutomationObjectInfo {
            id: info.id.clone(),
            kind: "integration".to_string(),
            title: info.name.clone(),
            subtitle: Some(info.description.clone()),
            status: status.clone(),
            detail: detail.or_else(|| {
                Some(if enabled {
                    "Enabled".to_string()
                } else {
                    "Disabled".to_string()
                })
            }),
            created_at: None,
            next_run_at: None,
            view: "settings".to_string(),
            url: None,
            enabled: Some(enabled),
            connected: Some(status == "connected"),
        });
    }

    if integrations::external_integration_config("gmail").is_some() {
        let has_refresh_token = integrations::oauth_has_refresh_token(integrations::stored_secret(
            manager.as_ref(),
            "gmail_tokens",
        ));
        let configured = integrations::gmail_oauth_pair(manager.as_ref()).is_some();
        let (status, detail) = if has_refresh_token {
            match integrations::validate_gmail_oauth_connection(&config_dir).await {
                Ok(()) => ("connected".to_string(), None),
                Err(error) => ("error".to_string(), Some(error)),
            }
        } else if configured {
            (
                "needs_auth".to_string(),
                Some("Google sign-in required to finish connecting Gmail.".to_string()),
            )
        } else {
            ("not_configured".to_string(), None)
        };
        let enabled = manager
            .as_ref()
            .and_then(|m| {
                m.get_custom_secret(&integrations::integration_enabled_key("gmail"))
                    .ok()
                    .flatten()
            })
            .and_then(|v| integrations::parse_boolish(&v))
            .unwrap_or(status == "connected");
        totals.integrations += 1;
        objects.push(AutomationObjectInfo {
            id: "gmail".to_string(),
            kind: "integration".to_string(),
            title: "Gmail".to_string(),
            subtitle: Some("Connect Gmail to read, triage, and reply to email".to_string()),
            status: status.clone(),
            detail: detail.or_else(|| {
                Some(if enabled {
                    "Enabled".to_string()
                } else {
                    "Disabled".to_string()
                })
            }),
            created_at: None,
            next_run_at: None,
            view: "settings".to_string(),
            url: None,
            enabled: Some(enabled),
            connected: Some(status == "connected"),
        });
    }

    for app in state.app_registry.list().await {
        let row = app.as_object().cloned().unwrap_or_default();
        totals.apps += 1;
        let enabled = row.get("enabled").and_then(|v| v.as_bool()).unwrap_or(true);
        let running = row
            .get("running")
            .and_then(|v| v.as_bool())
            .unwrap_or(false);
        objects.push(AutomationObjectInfo {
            id: row
                .get("id")
                .and_then(|v| v.as_str())
                .unwrap_or_default()
                .to_string(),
            kind: "app".to_string(),
            title: row
                .get("title")
                .and_then(|v| v.as_str())
                .unwrap_or("App")
                .to_string(),
            subtitle: Some(
                row.get("runtime_mode")
                    .and_then(|v| v.as_str())
                    .unwrap_or("unknown")
                    .to_string(),
            ),
            status: if !enabled {
                "disabled".to_string()
            } else if running {
                "running".to_string()
            } else {
                "stopped".to_string()
            },
            detail: Some(
                if row
                    .get("access_guard_enabled")
                    .and_then(|v| v.as_bool())
                    .unwrap_or(false)
                {
                    "Access guard enabled".to_string()
                } else {
                    "Public in local workspace".to_string()
                },
            ),
            created_at: row
                .get("created_at")
                .and_then(|v| v.as_str())
                .map(|s| s.to_string()),
            next_run_at: None,
            view: "apps".to_string(),
            url: row
                .get("access_url")
                .and_then(|v| v.as_str())
                .map(|s| s.to_string()),
            enabled: Some(enabled),
            connected: None,
        });
    }

    totals.total = objects.len();
    Json(serde_json::json!({
        "objects": objects,
        "totals": totals
    }))
}

pub(super) async fn list_automation_runs_endpoint(
    State(state): State<AppState>,
) -> Json<serde_json::Value> {
    let (runs, supervisor_states) = {
        let agent = state.agent.read().await;
        (
            crate::core::list_automation_runs(&agent.storage, 30)
                .await
                .unwrap_or_default(),
            crate::core::list_automation_supervisor_states(&agent.storage)
                .await
                .unwrap_or_default(),
        )
    };
    let state_map: HashMap<String, crate::core::AutomationSupervisorState> = supervisor_states
        .into_iter()
        .map(|state| (state.automation_id.clone(), state))
        .collect();

    let items: Vec<AutomationRunInfo> = runs
        .into_iter()
        .map(|run| {
            let current_status = state_map
                .get(&run.automation_id)
                .map(|state| state.status.clone());
            AutomationRunInfo {
                id: run.id,
                automation_id: run.automation_id,
                kind: run.automation_kind.clone(),
                title: run.title,
                action: run.action,
                trigger: run.trigger,
                status: automation_run_status_label(&run.status),
                current_status,
                attempt: run.attempt,
                started_at: run.started_at,
                completed_at: run.completed_at,
                duration_ms: run.duration_ms,
                summary: run.critique.summary,
                output_preview: run.output_preview,
                error: run.error,
                next_retry_at: run.next_retry_at,
                conversation_id: run.origin.conversation_id,
                view: match run.automation_kind.as_str() {
                    "task" => "tasks".to_string(),
                    "watcher" => "watchers".to_string(),
                    "app" => "apps".to_string(),
                    "integration" => "settings".to_string(),
                    _ => "trace".to_string(),
                },
            }
        })
        .collect();

    Json(serde_json::json!({
        "runs": items
    }))
}

// =============================================================================
// Goals API (goals are stored as tasks with action="goal")
// =============================================================================

/// List goals (paginated)
pub(super) async fn list_goals(
    State(state): State<AppState>,
    axum::extract::Query(params): axum::extract::Query<std::collections::HashMap<String, String>>,
) -> Response {
    let limit = params
        .get("limit")
        .and_then(|s| s.parse().ok())
        .unwrap_or(20usize);
    let offset = params
        .get("offset")
        .and_then(|s| s.parse().ok())
        .unwrap_or(0usize);
    let tasks = state.tasks.read().await;
    let all_goals: Vec<_> = tasks.all().iter().filter(|t| t.action == "goal").collect();
    let total = all_goals.len();
    let goals: Vec<serde_json::Value> = all_goals
        .into_iter()
        .skip(offset)
        .take(limit)
        .map(|t| {
            let mut g = serde_json::json!({
                "id": t.id.to_string(),
                "description": t.description,
                "status": format!("{:?}", t.status),
                "created_at": t.created_at.to_rfc3339(),
            });
            if let Some(due) = t.scheduled_for {
                g["due_date"] = serde_json::json!(due.format("%Y-%m-%d").to_string());
            }
            if let Some(goal_id) = t.arguments.get("goal_id").and_then(|v| v.as_str()) {
                g["goal_id"] = serde_json::json!(goal_id);
                g["autopilot"] = serde_json::json!(true);
            } else {
                g["autopilot"] = serde_json::json!(false);
            }
            if let Some(goal_text) = t.arguments.get("goal").and_then(|v| v.as_str()) {
                g["goal"] = serde_json::json!(goal_text);
            }
            g
        })
        .collect();
    (
        StatusCode::OK,
        Json(
            serde_json::json!({ "goals": goals, "total": total, "limit": limit, "offset": offset }),
        ),
    )
        .into_response()
}

/// Create a goal
pub(super) async fn create_goal(
    State(state): State<AppState>,
    Json(request): Json<serde_json::Value>,
) -> Response {
    let description = match request.get("description").and_then(|v| v.as_str()) {
        Some(d) if !d.trim().is_empty() => d.trim().to_string(),
        _ => {
            return (
                StatusCode::BAD_REQUEST,
                Json(ErrorResponse {
                    error: "Missing or empty description".to_string(),
                }),
            )
                .into_response();
        }
    };

    // Parse optional due date (YYYY-MM-DD)
    let due_date = request
        .get("due_date")
        .and_then(|v| v.as_str())
        .and_then(|s| chrono::NaiveDate::parse_from_str(s, "%Y-%m-%d").ok())
        .map(|d| d.and_hms_opt(23, 59, 59).unwrap())
        .map(|dt| chrono::DateTime::<chrono::Utc>::from_naive_utc_and_offset(dt, chrono::Utc));

    let mut task = crate::core::Task::new(
        description.clone(),
        "goal".to_string(),
        serde_json::json!({}),
    );
    task.scheduled_for = due_date;

    // Persist to database
    {
        let agent = state.agent.read().await;
        if let Err(e) = agent.storage.insert_task(&task).await {
            return (
                StatusCode::INTERNAL_SERVER_ERROR,
                Json(ErrorResponse {
                    error: format!("Failed to save goal: {}", e),
                }),
            )
                .into_response();
        }
    }

    // Add to in-memory queue
    {
        let mut queue = state.tasks.write().await;
        queue.add(task);
    }

    // Auto-schedule reminder tasks if due date is set and > 1 day away
    if let Some(due) = due_date {
        let now = chrono::Utc::now();
        let days_until = (due - now).num_days();

        let mut reminders = Vec::new();
        // Reminder 1 day before
        if days_until > 1 {
            let remind_at = due - chrono::Duration::days(1);
            let mut r = crate::core::Task::new(
                format!("Reminder: \"{}\" is due tomorrow", description),
                "goal_reminder".to_string(),
                serde_json::json!({"goal": description, "days_left": 1}),
            );
            r.scheduled_for = Some(remind_at);
            reminders.push(r);
        }
        // Reminder 3 days before (if goal is > 3 days out)
        if days_until > 3 {
            let remind_at = due - chrono::Duration::days(3);
            let mut r = crate::core::Task::new(
                format!("Reminder: \"{}\" is due in 3 days", description),
                "goal_reminder".to_string(),
                serde_json::json!({"goal": description, "days_left": 3}),
            );
            r.scheduled_for = Some(remind_at);
            reminders.push(r);
        }

        if !reminders.is_empty() {
            let agent = state.agent.read().await;
            let mut queue = state.tasks.write().await;
            for r in reminders {
                let _ = agent.storage.insert_task(&r).await;
                queue.add(r);
            }
        }
    }

    (StatusCode::OK, Json(serde_json::json!({"status": "ok"}))).into_response()
}

/// Delete a goal
pub(super) async fn delete_goal_endpoint(
    State(state): State<AppState>,
    Path(id): Path<String>,
) -> Response {
    // Best-effort cascade delete:
    // - the goal task itself (by task id OR by goal_id)
    // - any goal-loop tasks keyed by arguments.goal_id (plan + scheduled reports)
    // - reminder tasks that match the goal description (legacy reminders without goal_id)
    //
    // Why: goal-loop "plan" tasks use action="plan" (not "goal_loop_plan"), but they still
    // carry arguments.goal_id. Without this cascade, deleting the goal leaves orphan tasks
    // visible in "Next Up".
    let all_tasks = {
        let agent = state.agent.read().await;
        agent.storage.get_tasks().await.unwrap_or_default()
    };

    // Identify the goal task and canonical goal_id.
    let mut goal_task_id: Option<String> = None;
    let mut goal_id: Option<String> = None;
    let mut goal_desc: Option<String> = None;

    // 1) Treat `id` as a goal task id.
    if let Some(t) = all_tasks.iter().find(|t| t.id == id && t.action == "goal") {
        goal_task_id = Some(t.id.clone());
        if let Ok(args) = serde_json::from_str::<serde_json::Value>(&t.arguments) {
            goal_id = args
                .get("goal_id")
                .and_then(|v| v.as_str())
                .map(|s| s.to_string());
            goal_desc = args
                .get("goal")
                .and_then(|v| v.as_str())
                .map(|s| s.to_string());
        }
        if goal_desc.is_none() {
            goal_desc = Some(t.description.clone());
        }
    }

    // 2) Treat `id` as a goal_id (common UI identifier).
    if goal_task_id.is_none() {
        let mut found: Option<(&crate::storage::entities::task::Model, serde_json::Value)> = None;
        for t in &all_tasks {
            if t.action != "goal" {
                continue;
            }
            let Ok(args) = serde_json::from_str::<serde_json::Value>(&t.arguments) else {
                continue;
            };
            if args.get("goal_id").and_then(|v| v.as_str()) == Some(id.as_str()) {
                found = Some((t, args));
                break;
            }
        }
        if let Some((t, args)) = found {
            goal_task_id = Some(t.id.clone());
            goal_id = Some(id.clone());
            goal_desc = args
                .get("goal")
                .and_then(|v| v.as_str())
                .map(|s| s.to_string())
                .or_else(|| Some(t.description.clone()));
        }
    }

    // If we still didn't find a goal task, we can still cascade-delete tasks by goal_id.
    if goal_id.is_none() {
        let mut any_ref = false;
        for t in &all_tasks {
            let Ok(args) = serde_json::from_str::<serde_json::Value>(&t.arguments) else {
                continue;
            };
            if args.get("goal_id").and_then(|v| v.as_str()) == Some(id.as_str()) {
                any_ref = true;
                break;
            }
        }
        if any_ref {
            goal_id = Some(id.clone());
        }
    }

    if goal_task_id.is_none() && goal_id.is_none() {
        return (
            StatusCode::NOT_FOUND,
            Json(ErrorResponse {
                error: "Goal not found".to_string(),
            }),
        )
            .into_response();
    }

    let mut ids_to_delete: Vec<String> = Vec::new();
    if let Some(gid) = goal_task_id.clone() {
        ids_to_delete.push(gid);
    }

    for t in &all_tasks {
        let tid = &t.id;
        if ids_to_delete.iter().any(|x| x == tid) {
            continue;
        }

        let args = serde_json::from_str::<serde_json::Value>(&t.arguments).ok();
        let arg_goal_id = args
            .as_ref()
            .and_then(|a| a.get("goal_id"))
            .and_then(|v| v.as_str());
        let arg_goal_desc = args
            .as_ref()
            .and_then(|a| a.get("goal"))
            .and_then(|v| v.as_str());

        let matches_goal_id = goal_id
            .as_deref()
            .and_then(|gid| arg_goal_id.map(|x| x == gid))
            .unwrap_or(false);
        let matches_goal_desc = goal_desc
            .as_deref()
            .and_then(|gd| arg_goal_desc.map(|x| x == gd))
            .unwrap_or(false);

        // Goal loop tasks we want to remove:
        // - Scheduled progress report task
        // - Goal reminders (legacy match by goal text)
        // - The "Goal Loop Plan: ..." task (action="plan", matches by description prefix + goal_id)
        let is_progress_report = t.action == "goal_progress_report";
        let is_goal_reminder = t.action == "goal_reminder";
        let is_goal_loop_plan = t.action == "plan" && t.description.starts_with("Goal Loop Plan:");
        let is_legacy_goal_loop_plan = t.action == "goal_loop_plan";

        if matches_goal_id && (is_progress_report || is_goal_loop_plan || is_legacy_goal_loop_plan)
        {
            ids_to_delete.push(t.id.clone());
            continue;
        }

        if is_goal_reminder && matches_goal_desc {
            ids_to_delete.push(t.id.clone());
            continue;
        }
    }

    // Delete from database
    {
        let agent = state.agent.read().await;
        for tid in &ids_to_delete {
            let _ = agent.storage.delete_task(tid).await;
        }
    }

    // Remove from in-memory queue
    {
        let mut queue = state.tasks.write().await;
        for tid in &ids_to_delete {
            if let Ok(uuid) = uuid::Uuid::parse_str(tid) {
                queue.remove(uuid);
            }
        }
    }

    let deleted_notifications = if let Some(goal_text) = goal_desc.as_deref() {
        let agent = state.agent.read().await;
        agent
            .storage
            .delete_goal_notifications(goal_text)
            .await
            .unwrap_or(0)
    } else {
        0
    };

    let deleted_reflect_units = {
        let agent = state.agent.read().await;
        let mut total = 0u64;
        if let Some(gid) = goal_id.as_deref() {
            total = total.saturating_add(
                agent
                    .storage
                    .delete_semantic_work_units_for_source("goal", gid)
                    .await
                    .unwrap_or(0),
            );
        }
        if let Some(task_id) = goal_task_id.as_deref() {
            total = total.saturating_add(
                agent
                    .storage
                    .delete_semantic_work_units_for_source("goal", task_id)
                    .await
                    .unwrap_or(0),
            );
        }
        total
    };

    (
        StatusCode::OK,
        Json(serde_json::json!({
            "status": "ok",
            "deleted_task_ids": ids_to_delete,
            "deleted_notifications": deleted_notifications,
            "deleted_reflect_units": deleted_reflect_units,
        })),
    )
        .into_response()
}

/// Create a new task
pub(super) async fn create_task(
    State(state): State<AppState>,
    Json(request): Json<CreateTaskRequest>,
) -> Response {
    use crate::core::{Task, TaskApproval, status_for_task_approval};

    // Convert and validate cron expression if provided
    // Standard 5-field cron is converted to 6-field (with seconds) for Rust cron crate
    let cron_expr = request.cron.as_ref().map(|expr| {
        if expr.split_whitespace().count() == 5 {
            format!("0 {}", expr) // Prepend "0 " for seconds
        } else {
            expr.clone()
        }
    });

    if let Some(ref cron) = cron_expr {
        if cron.parse::<cron::Schedule>().is_err() {
            return (
                StatusCode::BAD_REQUEST,
                Json(ErrorResponse {
                    error: format!("Invalid cron expression: {}", cron),
                }),
            )
                .into_response();
        }
    }

    let approval = match request.approval.as_deref() {
        Some("require") => TaskApproval::RequireApproval,
        Some("notify") => TaskApproval::RequireApproval,
        _ => TaskApproval::Auto,
    };

    let status = status_for_task_approval(&approval);

    let task = Task {
        id: uuid::Uuid::new_v4(),
        description: request.description,
        action: request.action.clone(),
        arguments: request.arguments,
        approval,
        capabilities: vec![request.action],
        status,
        created_at: chrono::Utc::now(),
        scheduled_for: None,
        cron: cron_expr,
        result: None,
        proof_id: None,
        priority: None,
        urgency: None,
        importance: None,
        eisenhower_quadrant: None,
    };

    let is_scheduled = task.cron.is_some();
    let save_result = {
        let agent = state.agent.read().await;
        agent
            .add_or_update_similar_task(task.clone(), request.allow_duplicate, None)
            .await
    };
    let (task_id, reused_existing, removed_duplicates) = match save_result {
        Ok(outcome) => outcome,
        Err(e) => {
            return (
                StatusCode::INTERNAL_SERVER_ERROR,
                Json(ErrorResponse {
                    error: format!("Failed to save task: {}", e),
                }),
            )
                .into_response();
        }
    };

    let message = if reused_existing {
        if is_scheduled {
            "Scheduled task updated"
        } else {
            "Task updated"
        }
    } else if is_scheduled {
        "Scheduled task created"
    } else {
        "Task created"
    };

    spawn_autonomy_analysis_tick(state.agent.clone(), "task_created");
    (
        StatusCode::OK,
        Json(serde_json::json!({
            "status": "ok",
            "id": task_id.to_string(),
            "message": message,
            "reused_existing": reused_existing,
            "removed_duplicates": removed_duplicates,
        })),
    )
        .into_response()
}

/// Update a task (description, arguments, cron)
pub(super) async fn update_task(
    State(state): State<AppState>,
    Path(id): Path<String>,
    Json(request): Json<UpdateTaskRequest>,
) -> Response {
    let uuid = match uuid::Uuid::parse_str(&id) {
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

    let mut tasks = state.tasks.write().await;
    let Some(task) = tasks.get_mut(uuid) else {
        return (
            StatusCode::NOT_FOUND,
            Json(ErrorResponse {
                error: "Task not found".to_string(),
            }),
        )
            .into_response();
    };

    let mut desc_to_save = None;
    let mut args_to_save = None;
    let mut cron_to_save = None;

    if let Some(description) = request.description {
        if !description.trim().is_empty() {
            task.description = description;
            desc_to_save = Some(task.description.clone());
        }
    }

    if let Some(arguments) = request.arguments {
        task.arguments = arguments;
        args_to_save = Some(serde_json::to_string(&task.arguments).unwrap_or("{}".to_string()));
    }

    if let Some(cron_value) = request.cron {
        let cron_clean = if cron_value.trim().is_empty() {
            None
        } else if cron_value.split_whitespace().count() == 5 {
            Some(format!("0 {}", cron_value))
        } else {
            Some(cron_value)
        };

        if let Some(ref cron) = cron_clean {
            if cron.parse::<cron::Schedule>().is_err() {
                return (
                    StatusCode::BAD_REQUEST,
                    Json(ErrorResponse {
                        error: format!("Invalid cron expression: {}", cron),
                    }),
                )
                    .into_response();
            }
        }

        task.cron = cron_clean;
        cron_to_save = task.cron.clone();
    }

    let save_result = {
        let agent = state.agent.read().await;
        agent
            .storage
            .update_task(&id, desc_to_save, args_to_save, cron_to_save, None)
            .await
    };

    if let Err(e) = save_result {
        return (
            StatusCode::INTERNAL_SERVER_ERROR,
            Json(ErrorResponse {
                error: format!("Failed to update task: {}", e),
            }),
        )
            .into_response();
    }

    spawn_autonomy_analysis_tick(state.agent.clone(), "task_updated");
    (StatusCode::OK, Json(serde_json::json!({"status": "ok"}))).into_response()
}

/// Delete a task
pub(super) async fn delete_task(State(state): State<AppState>, Path(id): Path<String>) -> Response {
    let uuid = match uuid::Uuid::parse_str(&id) {
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

    let mut tasks = state.tasks.write().await;
    let removed = tasks.remove(uuid);

    if removed {
        let delete_result = {
            let agent = state.agent.read().await;
            agent.storage.delete_task(&id).await
        };

        if let Err(e) = delete_result {
            return (
                StatusCode::INTERNAL_SERVER_ERROR,
                Json(ErrorResponse {
                    error: format!("Failed to delete task: {}", e),
                }),
            )
                .into_response();
        }

        (StatusCode::OK, Json(serde_json::json!({"status": "ok"}))).into_response()
    } else {
        (
            StatusCode::NOT_FOUND,
            Json(ErrorResponse {
                error: "Task not found".to_string(),
            }),
        )
            .into_response()
    }
}

/// Approve a task for execution
pub(super) async fn approve_task(
    State(state): State<AppState>,
    Path(id): Path<String>,
    body: Option<Json<ApprovalDecisionRequest>>,
) -> Response {
    let uuid = match uuid::Uuid::parse_str(&id) {
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

    let agent = state.agent.read().await;
    let comment = body
        .as_ref()
        .and_then(|payload| payload.0.comment.as_deref())
        .map(str::trim)
        .filter(|value| !value.is_empty());
    match agent
        .approve_task_request_with_comment(uuid, "api", comment)
        .await
    {
        Ok(Some(_)) => (StatusCode::OK, Json(serde_json::json!({"status": "ok"}))).into_response(),
        Ok(None) => {
            let _ = agent
                .storage
                .resolve_approval_request(&id, "stale", "api")
                .await;
            (
                StatusCode::GONE,
                Json(serde_json::json!({
                    "code": "approval_stale",
                    "error": "This approval is no longer attached to an awaiting task.",
                    "status": "stale"
                })),
            )
                .into_response()
        }
        Err(e) => (
            StatusCode::INTERNAL_SERVER_ERROR,
            Json(ErrorResponse {
                error: format!("Failed to approve task: {}", e),
            }),
        )
            .into_response(),
    }
}

/// Reject a task
pub(super) async fn reject_task(
    State(state): State<AppState>,
    Path(id): Path<String>,
    body: Option<Json<ApprovalDecisionRequest>>,
) -> Response {
    let uuid = match uuid::Uuid::parse_str(&id) {
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

    let agent = state.agent.read().await;
    let comment = body
        .as_ref()
        .and_then(|payload| payload.0.comment.as_deref().or(payload.0.reason.as_deref()))
        .map(str::trim)
        .filter(|value| !value.is_empty())
        .unwrap_or("Task was rejected and will not be executed.");
    match agent.reject_task_request(uuid, "api", comment).await {
        Ok(Some(_)) => (StatusCode::OK, Json(serde_json::json!({"status": "ok"}))).into_response(),
        Ok(None) => {
            let _ = agent
                .storage
                .resolve_approval_request(&id, "stale", "api")
                .await;
            (
                StatusCode::GONE,
                Json(serde_json::json!({
                    "code": "approval_stale",
                    "error": "This approval is no longer attached to an awaiting task.",
                    "status": "stale"
                })),
            )
                .into_response()
        }
        Err(e) => (
            StatusCode::INTERNAL_SERVER_ERROR,
            Json(ErrorResponse {
                error: format!("Failed to reject task: {}", e),
            }),
        )
            .into_response(),
    }
}

/// Cancel a queued or running task.
pub(super) async fn cancel_task(State(state): State<AppState>, Path(id): Path<String>) -> Response {
    let uuid = match uuid::Uuid::parse_str(&id) {
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

    let mut tasks = state.tasks.write().await;
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
        TaskStatus::Pending
            | TaskStatus::AwaitingApproval
            | TaskStatus::Paused
            | TaskStatus::InProgress
    ) {
        return (
            StatusCode::CONFLICT,
            Json(ErrorResponse {
                error: "Only queued, paused, approval-pending, or running tasks can be cancelled."
                    .to_string(),
            }),
        )
            .into_response();
    }

    task.status = TaskStatus::Cancelled;
    let save_result = {
        let agent = state.agent.read().await;
        agent
            .storage
            .update_task_status(
                &id,
                &serde_json::to_string(&task.status).unwrap_or("Cancelled".to_string()),
            )
            .await
    };

    if let Err(e) = save_result {
        return (
            StatusCode::INTERNAL_SERVER_ERROR,
            Json(ErrorResponse {
                error: format!("Failed to cancel task: {}", e),
            }),
        )
            .into_response();
    }

    signal_chat_task_cancellation(&state, &id).await;

    (StatusCode::OK, Json(serde_json::json!({"status": "ok"}))).into_response()
}

/// Pause a queued recurring or deferred task without deleting it.
pub(super) async fn pause_task(State(state): State<AppState>, Path(id): Path<String>) -> Response {
    let uuid = match uuid::Uuid::parse_str(&id) {
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

    let status_json = {
        let mut tasks = state.tasks.write().await;
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
            TaskStatus::Pending | TaskStatus::AwaitingApproval
        ) {
            return (
                StatusCode::CONFLICT,
                Json(ErrorResponse {
                    error: "Only queued or approval-pending tasks can be paused.".to_string(),
                }),
            )
                .into_response();
        }

        task.status = TaskStatus::Paused;
        serde_json::to_string(&task.status).unwrap_or("\"Paused\"".to_string())
    };

    let save_result = {
        let agent = state.agent.read().await;
        agent.storage.update_task_status(&id, &status_json).await
    };

    if let Err(e) = save_result {
        return (
            StatusCode::INTERNAL_SERVER_ERROR,
            Json(ErrorResponse {
                error: format!("Failed to pause task: {}", e),
            }),
        )
            .into_response();
    }

    (
        StatusCode::OK,
        Json(serde_json::json!({"status": "ok", "paused": true})),
    )
        .into_response()
}

/// Resume a paused task so it can run again.
pub(super) async fn resume_task(State(state): State<AppState>, Path(id): Path<String>) -> Response {
    let uuid = match uuid::Uuid::parse_str(&id) {
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

    let (status_json, scheduled_for_rfc3339) = {
        let mut tasks = state.tasks.write().await;
        let Some(task) = tasks.get_mut(uuid) else {
            return (
                StatusCode::NOT_FOUND,
                Json(ErrorResponse {
                    error: "Task not found".to_string(),
                }),
            )
                .into_response();
        };

        if task.action == "chat_request" {
            return (
                StatusCode::CONFLICT,
                Json(ErrorResponse {
                    error:
                        "Chat-request tasks must be resumed from chat via /tasks/{id}/resume-chat/stream."
                            .to_string(),
                }),
            )
                .into_response();
        }

        if !matches!(task.status, TaskStatus::Paused) {
            return (
                StatusCode::CONFLICT,
                Json(ErrorResponse {
                    error: "Only paused tasks can be resumed.".to_string(),
                }),
            )
                .into_response();
        }

        task.status = TaskStatus::Pending;
        let now = chrono::Utc::now();
        if task.cron.is_some()
            || task
                .scheduled_for
                .as_ref()
                .map(|dt| *dt <= now)
                .unwrap_or(false)
        {
            task.scheduled_for = Some(now);
        }

        (
            serde_json::to_string(&task.status).unwrap_or("\"Pending\"".to_string()),
            task.scheduled_for.as_ref().map(|dt| dt.to_rfc3339()),
        )
    };

    let save_result: anyhow::Result<()> = {
        let agent = state.agent.read().await;
        if let Err(err) = agent.storage.update_task_status(&id, &status_json).await {
            Err(err)
        } else if let Some(scheduled_for) = scheduled_for_rfc3339 {
            agent
                .storage
                .update_task(&id, None, None, None, Some(scheduled_for))
                .await
        } else {
            Ok(())
        }
    };

    if let Err(e) = save_result {
        return (
            StatusCode::INTERNAL_SERVER_ERROR,
            Json(ErrorResponse {
                error: format!("Failed to resume task: {}", e),
            }),
        )
            .into_response();
    }

    spawn_autonomy_analysis_tick(state.agent.clone(), "task_resumed");
    (
        StatusCode::OK,
        Json(serde_json::json!({"status": "ok", "resumed": true})),
    )
        .into_response()
}

pub(super) async fn resume_chat_task_stream(
    State(state): State<AppState>,
    maybe_caller: Option<Extension<crate::actions::ActionCallerPrincipal>>,
    Path(id): Path<String>,
    request: Option<Json<ResumeChatTaskStreamRequest>>,
) -> Response {
    let uuid = match uuid::Uuid::parse_str(&id) {
        Ok(value) => value,
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
    let resume_request_body = request.map(|Json(body)| body).unwrap_or_default();

    let (resume_request, resumed_task, previous_task, status_json, arguments_json, plan_override) = {
        let mut tasks = state.tasks.write().await;
        let Some(task) = tasks.get_mut(uuid) else {
            return (
                StatusCode::NOT_FOUND,
                Json(ErrorResponse {
                    error: "Task not found".to_string(),
                }),
            )
                .into_response();
        };

        let resume_request = match extract_resumable_web_chat_task(task) {
            Ok(request) => request,
            Err(error) => {
                return (StatusCode::CONFLICT, Json(ErrorResponse { error })).into_response();
            }
        };

        let previous_task = task.clone();
        let effective_plan_override = resume_request_body
            .plan_override
            .clone()
            .or_else(|| resume_request.stored_plan_override.clone());
        if resume_request.paused_for_plan_confirmation {
            let mut updated_arguments = task.arguments.as_object().cloned().unwrap_or_default();
            updated_arguments.remove("_pause_kind");
            if let Some(raw_plan) = effective_plan_override.clone() {
                let mut preview = updated_arguments
                    .get("_plan_preview")
                    .and_then(|value| value.as_object())
                    .cloned()
                    .unwrap_or_default();
                preview.insert("current_plan".to_string(), raw_plan);
                updated_arguments.insert(
                    "_plan_preview".to_string(),
                    serde_json::Value::Object(preview),
                );
            }
            task.arguments = serde_json::Value::Object(updated_arguments.clone());
        }
        task.status = TaskStatus::InProgress;
        task.result = None;
        task.proof_id = None;
        task.scheduled_for = None;
        let paused_for_plan_confirmation = resume_request.paused_for_plan_confirmation;

        (
            resume_request,
            task.clone(),
            previous_task,
            serde_json::to_string(&task.status).unwrap_or("\"InProgress\"".to_string()),
            if paused_for_plan_confirmation {
                Some(serde_json::to_string(&task.arguments).unwrap_or_else(|_| "{}".to_string()))
            } else {
                None
            },
            effective_plan_override,
        )
    };

    let save_result = {
        let agent = state.agent.read().await;
        if let Err(error) = agent.storage.retry_task(&id, &status_json, None).await {
            Err(error)
        } else if let Some(arguments_json) = arguments_json.clone() {
            agent
                .storage
                .update_task(&id, None, Some(arguments_json), None, None)
                .await
        } else {
            Ok(())
        }
    };

    if let Err(error) = save_result {
        let mut tasks = state.tasks.write().await;
        if let Some(task) = tasks.get_mut(uuid) {
            *task = previous_task;
        }
        return (
            StatusCode::INTERNAL_SERVER_ERROR,
            Json(ErrorResponse {
                error: format!("Failed to resume chat task: {}", error),
            }),
        )
            .into_response();
    }

    spawn_chat_stream_response(
        state,
        ChatStreamRunRequest {
            message: resume_request.message,
            channel: resume_request.channel,
            conversation_id: Some(resume_request.conversation_id),
            user_message_already_recorded: true,
            recorded_user_message_id: None,
            deep_research: resume_request.deep_research,
            plan_confirmation_mode: None,
            attachments_present: false,
            attachments: Vec::new(),
            arkorbit_context: None,
            caller_principal: maybe_caller.as_ref().map(|Extension(value)| value.clone()),
            accepted_suggestion: None,
            task_mode: ChatStreamTaskMode::Existing(Box::new(StreamedChatTask {
                task_id: resumed_task.id.to_string(),
                description: resumed_task.description.clone(),
                work_type: resume_request.work_type,
                user_message_already_recorded: true,
                plan_override,
            })),
        },
    )
}

/// Retry a failed or cancelled task.
pub(super) async fn retry_task(State(state): State<AppState>, Path(id): Path<String>) -> Response {
    let uuid = match uuid::Uuid::parse_str(&id) {
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

    let (status_json, scheduled_for_rfc3339) = {
        let mut tasks = state.tasks.write().await;
        let Some(task) = tasks.get_mut(uuid) else {
            return (
                StatusCode::NOT_FOUND,
                Json(ErrorResponse {
                    error: "Task not found".to_string(),
                }),
            )
                .into_response();
        };

        if task.action == "chat_request" {
            return (
                StatusCode::CONFLICT,
                Json(ErrorResponse {
                    error:
                        "Chat-request tasks must be retried from chat via /tasks/{id}/resume-chat/stream."
                            .to_string(),
                }),
            )
                .into_response();
        }

        if !matches!(
            task.status,
            TaskStatus::Failed { .. } | TaskStatus::Cancelled
        ) {
            return (
                StatusCode::CONFLICT,
                Json(ErrorResponse {
                    error: "Only failed or cancelled tasks can be retried.".to_string(),
                }),
            )
                .into_response();
        }

        task.status = TaskStatus::Pending;
        task.result = None;
        task.proof_id = None;
        task.scheduled_for = if task.cron.is_some() || task.scheduled_for.is_some() {
            Some(chrono::Utc::now())
        } else {
            None
        };

        (
            serde_json::to_string(&task.status).unwrap_or("Pending".to_string()),
            task.scheduled_for.as_ref().map(|dt| dt.to_rfc3339()),
        )
    };

    let save_result = {
        let agent = state.agent.read().await;
        agent
            .storage
            .retry_task(&id, &status_json, scheduled_for_rfc3339)
            .await
    };

    if let Err(e) = save_result {
        return (
            StatusCode::INTERNAL_SERVER_ERROR,
            Json(ErrorResponse {
                error: format!("Failed to retry task: {}", e),
            }),
        )
            .into_response();
    }

    spawn_autonomy_analysis_tick(state.agent.clone(), "task_retried");
    (
        StatusCode::OK,
        Json(serde_json::json!({
            "status": "ok",
            "message": "Task queued for retry"
        })),
    )
        .into_response()
}

/// Plan a task using the LLM (returns a structured plan)
pub(super) async fn plan_task(
    State(state): State<AppState>,
    Json(request): Json<PlanTaskRequest>,
) -> Response {
    let (llm, actions) = {
        let agent = state.agent.read().await;
        let actions = match agent.runtime.list_enabled_actions().await {
            Ok(s) => s,
            Err(e) => {
                return (
                    StatusCode::INTERNAL_SERVER_ERROR,
                    Json(ErrorResponse {
                        error: format!("Failed to list actions: {}", e),
                    }),
                )
                    .into_response();
            }
        };
        (agent.llm.clone(), actions)
    };

    let (selector_prompt, selector_message) = crate::core::planner::build_action_selector_prompt(
        &request.description,
        request.prompt.as_deref(),
        &actions,
    );

    let selector_response = match llm
        .chat(&selector_prompt, &selector_message, &[], &actions)
        .await
    {
        Ok(r) => r,
        Err(e) => {
            return (
                StatusCode::INTERNAL_SERVER_ERROR,
                Json(ErrorResponse {
                    error: format!("LLM planning failed: {}", e),
                }),
            )
                .into_response();
        }
    };

    let needed_action_names = crate::core::planner::parse_action_selection(
        &selector_response.content,
        &actions,
        crate::core::planner::DEFAULT_MAX_ACTIONS_FOR_PLAN,
    );
    let needed_actions = crate::core::planner::shortlist_actions(
        &actions,
        &needed_action_names,
        crate::core::planner::DEFAULT_MAX_ACTIONS_FOR_PLAN,
    );

    let (plan_prompt, plan_message) = crate::core::planner::build_plan_prompt(
        &request.description,
        request.prompt.as_deref(),
        &needed_actions,
        crate::core::PlanPromptMode::TaskAutomation,
        None,
    );

    let plan_response = match llm
        .chat(&plan_prompt, &plan_message, &[], &needed_actions)
        .await
    {
        Ok(r) => r,
        Err(e) => {
            return (
                StatusCode::INTERNAL_SERVER_ERROR,
                Json(ErrorResponse {
                    error: format!("LLM planning failed: {}", e),
                }),
            )
                .into_response();
        }
    };

    match crate::core::planner::parse_plan_from_llm_content(
        &plan_response.content,
        &needed_actions,
        None,
        1,
        false,
    ) {
        Some(plan) => (StatusCode::OK, Json(PlanTaskResponse { plan })).into_response(),
        None => (
            StatusCode::BAD_REQUEST,
            Json(ErrorResponse {
                error: "Planner returned invalid JSON".to_string(),
            }),
        )
            .into_response(),
    }
}

pub(super) fn strip_markdown_code_fence(text: &str) -> Option<&str> {
    let trimmed = text.trim();
    if !trimmed.starts_with("```") {
        return None;
    }
    let after_ticks = &trimmed[3..];
    let body_start = after_ticks.find('\n')?;
    let body = &after_ticks[body_start + 1..];
    let fence_end = body.rfind("```")?;
    Some(body[..fence_end].trim())
}

pub(super) fn find_json_value_bounds(raw: &str) -> Option<(usize, usize)> {
    let mut stack: Vec<char> = Vec::new();
    let mut start: Option<usize> = None;
    let mut in_string = false;
    let mut escaped = false;

    for (idx, ch) in raw.char_indices() {
        if in_string {
            if escaped {
                escaped = false;
                continue;
            }
            if ch == '\\' {
                escaped = true;
                continue;
            }
            if ch == '"' {
                in_string = false;
            }
            continue;
        }

        match ch {
            '"' => in_string = true,
            '{' | '[' => {
                if stack.is_empty() {
                    start = Some(idx);
                }
                stack.push(ch);
            }
            '}' => {
                if stack.last() == Some(&'{') {
                    stack.pop();
                    if stack.is_empty() {
                        if let Some(value_start) = start {
                            return Some((value_start, idx + ch.len_utf8()));
                        }
                    }
                }
            }
            ']' => {
                if stack.last() == Some(&'[') {
                    stack.pop();
                    if stack.is_empty() {
                        if let Some(value_start) = start {
                            return Some((value_start, idx + ch.len_utf8()));
                        }
                    }
                }
            }
            _ => {}
        }
    }

    None
}

pub(super) fn extract_json(text: &str) -> Option<serde_json::Value> {
    for candidate in [Some(text.trim()), strip_markdown_code_fence(text)] {
        let Some(candidate) = candidate.map(str::trim).filter(|value| !value.is_empty()) else {
            continue;
        };
        if let Ok(value) = serde_json::from_str::<serde_json::Value>(candidate) {
            return Some(value);
        }
        if let Some((start, end)) = find_json_value_bounds(candidate) {
            if let Ok(value) = serde_json::from_str::<serde_json::Value>(&candidate[start..end]) {
                return Some(value);
            }
        }
    }
    None
}

pub(super) fn risk_level_label(level: &RiskLevel) -> &'static str {
    match level {
        RiskLevel::Low => "low",
        RiskLevel::Medium => "medium",
        RiskLevel::High => "high",
        RiskLevel::Critical => "critical",
    }
}

pub(super) async fn load_autonomy_settings(agent: &Agent) -> AutonomySettings {
    agent.load_autonomy_settings().await
}

pub(super) async fn save_autonomy_settings(
    agent: &Agent,
    settings: &AutonomySettings,
) -> Result<(), String> {
    agent.save_autonomy_settings(settings).await
}

pub(super) fn recommendation(
    title: &str,
    description: &str,
    action_kind: &str,
    payload: serde_json::Value,
    trust_policy: &TrustPolicy,
    readiness_policy: &crate::core::ReadinessPolicy,
) -> RecommendedAction {
    let id = uuid::Uuid::new_v4().to_string();
    let trust = score_action_risk(action_kind, &payload, trust_policy);
    let readiness =
        crate::core::readiness::evaluate_recommended_action_readiness(&trust, readiness_policy);
    RecommendedAction {
        id,
        title: title.to_string(),
        description: description.to_string(),
        action_kind: action_kind.to_string(),
        payload,
        trust,
        readiness: Some(readiness),
    }
}

#[derive(Debug, Clone)]
pub(super) struct BriefingGoalCandidate {
    task_id: String,
    objective: String,
    due_date: Option<String>,
}

pub(super) fn choose_briefing_goal_candidate(tasks: &[Task]) -> Option<BriefingGoalCandidate> {
    #[derive(Debug)]
    struct RankedGoalCandidate {
        goal: BriefingGoalCandidate,
        status_rank: u8,
        importance: f32,
        urgency: f32,
        priority: f32,
        due_at_unix: i64,
        created_at_unix: i64,
    }

    let mut candidates = tasks
        .iter()
        .filter(|task| task.action.eq_ignore_ascii_case("goal"))
        .filter_map(|task| {
            let status_rank = match task.status {
                TaskStatus::InProgress => 0,
                TaskStatus::Pending => 1,
                TaskStatus::Paused => 2,
                _ => return None,
            };

            let objective = task
                .arguments
                .get("goal")
                .and_then(|value| value.as_str())
                .map(str::trim)
                .filter(|value| !value.is_empty())
                .unwrap_or_else(|| task.description.trim());
            if objective.is_empty() {
                return None;
            }

            Some(RankedGoalCandidate {
                goal: BriefingGoalCandidate {
                    task_id: task.id.to_string(),
                    objective: objective.to_string(),
                    due_date: task
                        .scheduled_for
                        .as_ref()
                        .map(|value| value.format("%Y-%m-%d").to_string()),
                },
                status_rank,
                importance: task.importance.unwrap_or(0.0),
                urgency: task.urgency.unwrap_or(0.0),
                priority: task.priority.unwrap_or(0.0),
                due_at_unix: task
                    .scheduled_for
                    .as_ref()
                    .map(|value| value.timestamp())
                    .unwrap_or(i64::MAX),
                created_at_unix: task.created_at.timestamp(),
            })
        })
        .collect::<Vec<_>>();

    candidates.sort_by(|left, right| {
        left.status_rank
            .cmp(&right.status_rank)
            .then_with(|| right.importance.total_cmp(&left.importance))
            .then_with(|| right.urgency.total_cmp(&left.urgency))
            .then_with(|| right.priority.total_cmp(&left.priority))
            .then_with(|| left.due_at_unix.cmp(&right.due_at_unix))
            .then_with(|| right.created_at_unix.cmp(&left.created_at_unix))
    });

    candidates
        .into_iter()
        .next()
        .map(|candidate| candidate.goal)
}

pub(super) fn autonomy_background_disabled(settings: &AutonomySettings) -> bool {
    crate::core::autonomy::autonomy_background_paused(settings)
}

pub(super) async fn apply_autopilot_mode(
    agent: &Agent,
    settings: &mut AutonomySettings,
    mode_id: &str,
) -> Result<serde_json::Value, String> {
    agent.apply_autopilot_mode(settings, mode_id).await
}

pub(super) async fn run_chat_suggestion_scan(state: &AppState, trigger: &str) -> serde_json::Value {
    let Some(_scan_guard) = try_start_chat_suggestion_scan() else {
        return serde_json::json!({
            "status": "running",
            "message": "Chat suggestion scan already in progress"
        });
    };

    let (storage, encrypted_storage) = {
        let agent = state.agent.read().await;
        (agent.storage.clone(), agent.encrypted_storage.clone())
    };
    let settings = load_autonomy_settings_from_storage(&storage).await;
    let autonomy_disabled = autonomy_background_disabled(&settings);
    let now = chrono::Utc::now();
    let now_rfc3339 = now.to_rfc3339();
    let mut scan_state = load_chat_suggestion_scan_state(&storage).await;

    if autonomy_disabled {
        scan_state.last_completed_at = Some(now_rfc3339.clone());
        scan_state.last_status = Some("disabled".to_string());
        scan_state.last_error = None;
        scan_state.next_due_at = None;
        scan_state.defer_count = 0;
        save_chat_suggestion_scan_state(&storage, &scan_state).await;
        return serde_json::json!({
            "status": "disabled",
            "message": "Autonomy is disabled.",
        });
    }

    if trigger != "manual" && !chat_suggestion_scan_is_due(&scan_state, now) {
        return serde_json::json!({
            "status": "not_due",
            "next_due_at": scan_state.next_due_at,
        });
    }

    scan_state.last_started_at = Some(now_rfc3339.clone());
    scan_state.last_status = Some("running".to_string());
    scan_state.last_error = None;
    save_chat_suggestion_scan_state(&storage, &scan_state).await;

    let has_user_chat = storage.has_user_chat_messages().await.unwrap_or(false);
    if !has_user_chat {
        scan_state.last_completed_at = Some(now_rfc3339.clone());
        scan_state.last_status = Some("no_user_chat".to_string());
        scan_state.next_due_at = Some(chat_suggestion_due_at(now));
        scan_state.defer_count = 0;
        scan_state.last_examined_chats = 0;
        scan_state.last_created_suggestions = 0;
        scan_state.last_low_signal_skips = 0;
        scan_state.last_artifact_skips = 0;
        scan_state.last_backlog_hint = 0;
        save_chat_suggestion_scan_state(&storage, &scan_state).await;
        return serde_json::json!({
            "status": "no_user_chat",
            "next_due_at": scan_state.next_due_at,
        });
    }

    if server_busy_for_chat_suggestions(state).await {
        scan_state.defer_count = scan_state.defer_count.saturating_add(1);
        scan_state.last_status = Some("deferred_busy".to_string());
        scan_state.next_due_at = Some(chat_suggestion_deferred_due_at(now, scan_state.defer_count));
        scan_state.last_error = None;
        save_chat_suggestion_scan_state(&storage, &scan_state).await;
        return serde_json::json!({
            "status": "deferred_busy",
            "next_due_at": scan_state.next_due_at,
            "defer_count": scan_state.defer_count,
        });
    }

    let agent_snapshot = Agent::snapshot(&state.agent).await;
    let mut suggestions = load_chat_suggestions(&storage).await;
    let mut removed_duplicate_suggestions = collapse_semantically_equivalent_chat_suggestions(
        &agent_snapshot.llm,
        &mut suggestions,
        &now_rfc3339,
    )
    .await;
    let mut reconciled_suggestion_updates =
        suggestions::reconcile_open_chat_suggestions_with_durable_work(
            &agent_snapshot,
            &mut suggestions,
        )
        .await;
    if removed_duplicate_suggestions > 0 || reconciled_suggestion_updates > 0 {
        save_chat_suggestions(&storage, &suggestions).await;
    }
    let internal_channels = ["arkpulse", "sentinel", "system", "autonomy"];
    let conversations = match storage
        .list_conversations_after_cursor(
            scan_state.cursor_updated_at.as_deref(),
            scan_state.cursor_conversation_id.as_deref(),
            CHAT_SUGGESTION_SCAN_FETCH_LIMIT,
            None,
        )
        .await
    {
        Ok(rows) => rows,
        Err(error) => {
            scan_state.last_status = Some("error".to_string());
            scan_state.last_error = Some(error.to_string());
            scan_state.next_due_at = Some(chat_suggestion_deferred_due_at(now, 1));
            save_chat_suggestion_scan_state(&storage, &scan_state).await;
            return serde_json::json!({
                "status": "error",
                "message": error.to_string(),
            });
        }
    };

    if conversations.is_empty() {
        scan_state.last_completed_at = Some(now_rfc3339.clone());
        scan_state.last_status = Some("no_candidates".to_string());
        scan_state.next_due_at = Some(chat_suggestion_due_at(now));
        scan_state.defer_count = 0;
        scan_state.cursor_updated_at = None;
        scan_state.cursor_conversation_id = None;
        scan_state.last_examined_chats = 0;
        scan_state.last_created_suggestions = 0;
        scan_state.last_low_signal_skips = 0;
        scan_state.last_artifact_skips = 0;
        scan_state.last_backlog_hint = 0;
        save_chat_suggestion_scan_state(&storage, &scan_state).await;
        return serde_json::json!({
            "status": "no_candidates",
            "next_due_at": scan_state.next_due_at,
        });
    }

    let mut examined_chats = 0usize;
    let mut low_signal_skips = 0usize;
    let mut artifact_skips = 0usize;
    let mut new_suggestion_ids = std::collections::HashSet::new();

    for conversation in &conversations {
        if internal_channels.contains(&conversation.channel.as_str()) || conversation.archived {
            scan_state.cursor_updated_at = Some(conversation.updated_at.clone());
            scan_state.cursor_conversation_id = Some(conversation.id.clone());
            continue;
        }

        let recent_messages = encrypted_storage
            .get_recent_messages_decrypted(
                &conversation.id,
                CHAT_SUGGESTION_RECENT_MESSAGES_PER_CHAT as u64,
            )
            .await
            .unwrap_or_default();
        let latest_user = recent_messages
            .iter()
            .rev()
            .find(|message| message.role.eq_ignore_ascii_case("user"))
            .cloned();

        scan_state.cursor_updated_at = Some(conversation.updated_at.clone());
        scan_state.cursor_conversation_id = Some(conversation.id.clone());

        let Some(latest_user) = latest_user else {
            upsert_chat_suggestion_watermark(
                &mut scan_state,
                &conversation.id,
                &conversation.updated_at,
                None,
                None,
                &now_rfc3339,
            );
            continue;
        };

        let existing_watermark = scan_state
            .conversation_watermarks
            .iter()
            .find(|entry| entry.conversation_id == conversation.id);
        let already_scanned = existing_watermark.is_some_and(|entry| {
            entry.last_user_message_id.as_deref() == Some(latest_user.id.as_str())
                && entry.last_scanned_updated_at >= conversation.updated_at
        });
        if already_scanned {
            continue;
        }

        if examined_chats >= CHAT_SUGGESTION_SCAN_BATCH_LIMIT {
            break;
        }
        examined_chats += 1;

        if conversation_has_recent_app_artifact(&storage, &conversation.id).await {
            artifact_skips += 1;
            upsert_chat_suggestion_watermark(
                &mut scan_state,
                &conversation.id,
                &conversation.updated_at,
                Some(&latest_user.id),
                Some(&latest_user.timestamp),
                &now_rfc3339,
            );
            continue;
        }

        if !conversation_has_signal(&recent_messages) {
            low_signal_skips += 1;
            upsert_chat_suggestion_watermark(
                &mut scan_state,
                &conversation.id,
                &conversation.updated_at,
                Some(&latest_user.id),
                Some(&latest_user.timestamp),
                &now_rfc3339,
            );
            continue;
        }

        let Some(source_message) = extract_latest_signal_user_message(&recent_messages) else {
            low_signal_skips += 1;
            upsert_chat_suggestion_watermark(
                &mut scan_state,
                &conversation.id,
                &conversation.updated_at,
                Some(&latest_user.id),
                Some(&latest_user.timestamp),
                &now_rfc3339,
            );
            continue;
        };

        if let Some(mut suggestion) = infer_chat_automation_suggestion(
            &agent_snapshot,
            conversation,
            &source_message,
            &recent_messages,
        )
        .await
        {
            let duplicate_idx = suggestions
                .iter()
                .position(|existing| existing.fingerprint == suggestion.fingerprint);
            if let Some(idx) = duplicate_idx {
                suggestions[idx].updated_at = now_rfc3339.clone();
                suggestions[idx].source_message_id = suggestion.source_message_id.clone();
                suggestions[idx].source_snippet = suggestion.source_snippet.clone();
                suggestions[idx].conversation_title = suggestion.conversation_title.clone();
                suggestions[idx].conversation_channel = suggestion.conversation_channel.clone();
                suggestions[idx].detail = suggestion.detail.clone();
                suggestions[idx].rationale = suggestion.rationale.clone();
                if suggestions[idx].status == "open" {
                    suggestions[idx].goal_title = suggestion.goal_title.clone();
                    suggestions[idx].goal_detail = suggestion.goal_detail.clone();
                }
            } else if suggestions
                .iter()
                .filter(|item| item.status == "open")
                .count()
                < CHAT_SUGGESTION_OPEN_LIMIT
            {
                suggestion.created_at = now_rfc3339.clone();
                suggestion.updated_at = now_rfc3339.clone();
                new_suggestion_ids.insert(suggestion.id.clone());
                suggestions.push(suggestion);
            }
        }

        upsert_chat_suggestion_watermark(
            &mut scan_state,
            &conversation.id,
            &conversation.updated_at,
            Some(&latest_user.id),
            Some(&latest_user.timestamp),
            &now_rfc3339,
        );
    }

    removed_duplicate_suggestions += collapse_semantically_equivalent_chat_suggestions(
        &agent_snapshot.llm,
        &mut suggestions,
        &now_rfc3339,
    )
    .await;
    reconciled_suggestion_updates +=
        suggestions::reconcile_open_chat_suggestions_with_durable_work(
            &agent_snapshot,
            &mut suggestions,
        )
        .await;
    let created_suggestions = new_suggestion_ids
        .iter()
        .filter(|id| {
            suggestions
                .iter()
                .any(|suggestion| suggestion.id == **id && suggestion.status == "open")
        })
        .count();
    suggestions = prune_chat_suggestion_history(suggestions);
    save_chat_suggestions(&storage, &suggestions).await;

    scan_state.last_completed_at = Some(now_rfc3339.clone());
    scan_state.last_status = Some("completed".to_string());
    scan_state.last_error = None;
    scan_state.next_due_at = Some(chat_suggestion_due_at(now));
    scan_state.defer_count = 0;
    scan_state.last_examined_chats = examined_chats;
    scan_state.last_created_suggestions = created_suggestions;
    scan_state.last_low_signal_skips = low_signal_skips;
    scan_state.last_artifact_skips = artifact_skips;
    scan_state.last_backlog_hint = conversations.len().saturating_sub(examined_chats);
    if conversations.len() < CHAT_SUGGESTION_SCAN_FETCH_LIMIT as usize {
        scan_state.cursor_updated_at = None;
        scan_state.cursor_conversation_id = None;
    }
    save_chat_suggestion_scan_state(&storage, &scan_state).await;

    serde_json::json!({
        "status": "completed",
        "examined_chats": examined_chats,
        "created_suggestions": created_suggestions,
        "reconciled_suggestions": reconciled_suggestion_updates,
        "removed_duplicate_suggestions": removed_duplicate_suggestions,
        "low_signal_skips": low_signal_skips,
        "artifact_skips": artifact_skips,
        "next_due_at": scan_state.next_due_at,
    })
}

fn chat_suggestion_execution_focus(suggestion: &ChatAutomationSuggestion) -> String {
    suggestion
        .goal_detail
        .as_deref()
        .filter(|value| !value.trim().is_empty())
        .or_else(|| (!suggestion.detail.trim().is_empty()).then_some(suggestion.detail.as_str()))
        .or_else(|| {
            (!suggestion.source_snippet.trim().is_empty())
                .then_some(suggestion.source_snippet.as_str())
        })
        .unwrap_or(suggestion.title.as_str())
        .trim()
        .to_string()
}

async fn choose_watch_poll_action_for_suggestion(
    agent: &Agent,
    _focus: &str,
) -> std::result::Result<String, String> {
    let actions = agent
        .runtime
        .list_enabled_actions()
        .await
        .map_err(|error| format!("Failed to load action catalog: {}", error))?;
    let mut data_sources = actions
        .into_iter()
        .filter(|action| {
            matches!(
                crate::actions::action_metadata_for_action(action).role,
                crate::actions::ActionRole::DataSource
            )
        })
        .collect::<Vec<_>>();
    data_sources.sort_by(|left, right| left.name.cmp(&right.name));
    data_sources
        .into_iter()
        .next()
        .map(|action| action.name)
        .ok_or_else(|| "No data-source action is available for watcher polling".to_string())
}

fn watch_payload_from_chat_suggestion(
    suggestion: &ChatAutomationSuggestion,
    poll_action: &str,
    focus: &str,
) -> serde_json::Value {
    let description = if suggestion.goal_title.trim().is_empty() {
        suggestion.title.trim()
    } else {
        suggestion.goal_title.trim()
    };
    let source_context = serde_json::json!({
        "suggestion_id": suggestion.id,
        "suggestion_kind": suggestion.kind,
        "conversation_id": suggestion.conversation_id,
        "source_message_id": suggestion.source_message_id,
    });
    serde_json::json!({
        "description": description,
        "poll_action": poll_action,
        "poll_arguments": {
            "query": focus,
            "_chat_suggestion": source_context,
        },
        "condition": {
            "description": "Trigger when the poll result contains a meaningful new or actionable update for the accepted monitoring goal.",
            "type": "llm",
        },
        "on_trigger": format!("Notify the user with the relevant update and source context for: {}", focus),
        "interval_secs": 3600,
        "timeout_days": 7,
        "notify_channel": "preferred",
        "channel": "autonomy",
        "conversation_id": suggestion.conversation_id,
        "_chat_suggestion": source_context,
    })
}

async fn append_suggestion_trace_step(
    trace_ref: &Arc<RwLock<ExecutionTrace>>,
    icon: &str,
    title: &str,
    detail: &str,
    step_type: &str,
    data: Option<serde_json::Value>,
) {
    trace_ref
        .write()
        .await
        .steps
        .push(crate::core::ExecutionStep {
            icon: icon.to_string(),
            title: title.to_string(),
            detail: detail.to_string(),
            step_type: step_type.to_string(),
            data: data
                .and_then(|value| serde_json::to_string_pretty(&value).ok())
                .map(|text| crate::security::redact_pii(&text)),
            timestamp: chrono::Utc::now(),
            duration_ms: Some(0),
        });
}

pub(super) async fn execute_accepted_watcher_suggestion(
    agent: &Agent,
    suggestion: &ChatAutomationSuggestion,
    trace_ref: &Arc<RwLock<ExecutionTrace>>,
) -> std::result::Result<(), String> {
    let focus = chat_suggestion_execution_focus(suggestion);
    let poll_action = choose_watch_poll_action_for_suggestion(agent, &focus).await?;
    let payload = watch_payload_from_chat_suggestion(suggestion, &poll_action, &focus);
    let action = RecommendedAction {
        id: format!("chat-suggestion-watch:{}", suggestion.id),
        title: suggestion.title.clone(),
        description: suggestion.detail.clone(),
        action_kind: "watch".to_string(),
        payload: payload.clone(),
        trust: RiskEnvelope::default(),
        readiness: None,
    };

    append_suggestion_trace_step(
        trace_ref,
        "[plan]",
        "Prepared watcher launch",
        "Mapped the accepted suggestion to a durable watcher action.",
        "info",
        Some(serde_json::json!({
            "suggestion_id": suggestion.id,
            "poll_action": poll_action,
            "focus": focus,
        })),
    )
    .await;

    let mut settings = load_autonomy_settings(agent).await;
    let result = agent
        .execute_autonomy_action_payload(&mut settings, &action.action_kind, &action.payload)
        .await?;
    let _ = save_autonomy_settings(agent, &settings).await;
    let summary = summarize_autonomy_action_result(&action, &result);
    append_suggestion_trace_step(
        trace_ref,
        "[ok]",
        "Watcher saved",
        &summary,
        "success",
        Some(serde_json::json!({
            "action": action,
            "result": result,
        })),
    )
    .await;
    let mut trace = trace_ref.write().await;
    trace.completed_at = Some(chrono::Utc::now());
    trace.response = Some(summary);
    trace.model = Some("internal:accepted-suggestion".to_string());
    trace.complexity = Some("watcher".to_string());
    Ok(())
}

pub(super) async fn accept_chat_suggestion(
    state: &AppState,
    suggestion_id: &str,
) -> Result<serde_json::Value, String> {
    let storage = { state.agent.read().await.storage.clone() };
    let mut suggestions = load_chat_suggestions(&storage).await;
    let Some(idx) = suggestions
        .iter()
        .position(|suggestion| suggestion.id == suggestion_id)
    else {
        return Err("Suggestion not found".to_string());
    };

    if suggestions[idx].status != "open" {
        return Err("Suggestion is no longer open".to_string());
    }

    let started_at = chrono::Utc::now();
    let started_at_text = started_at.to_rfc3339();
    let suggestion = suggestions[idx].clone();
    let trace_ref = Arc::new(RwLock::new(ExecutionTrace::default()));
    let trace_id = uuid::Uuid::new_v4().to_string();
    let prompt = build_chat_suggestion_execution_prompt(&suggestion);
    let suggestion_record_id = suggestion.id.clone();
    let run_snapshot = suggestions::capture_run_snapshot(state).await;

    {
        let mut trace = trace_ref.write().await;
        trace.id = trace_id.clone();
        trace.message = prompt.clone();
        trace.channel = "autonomy".to_string();
        trace.started_at = Some(started_at);
    }

    suggestions[idx].status = "accepted".to_string();
    suggestions[idx].updated_at = started_at_text.clone();
    suggestions[idx].accepted_at = Some(started_at_text.clone());
    suggestions[idx].run_status = Some("running".to_string());
    suggestions[idx].last_run_started_at = Some(started_at_text.clone());
    suggestions[idx].last_run_completed_at = None;
    suggestions[idx].last_run_error = None;
    suggestions[idx].accepted_trace_id = Some(trace_id.clone());
    suggestions[idx].accepted_goal_id = None;
    suggestions[idx].accepted_outcomes.clear();
    suggestions = prune_chat_suggestion_history(suggestions);
    save_chat_suggestions(&storage, &suggestions).await;
    let _ = trace::persist_live_trace_snapshot(&state.trace_history, &trace_ref).await;

    trace::spawn_live_trace_mirror(state.trace_history.clone(), trace_ref.clone());
    {
        let state_for_run = state.clone();
        let trace_ref_for_run = trace_ref.clone();
        let storage_for_run = storage.clone();
        let prompt_for_run = prompt.clone();
        let suggestion_for_run = suggestion.clone();
        let suggestion_id_for_run = suggestion_record_id.clone();
        let trace_id_for_run = trace_id.clone();
        let suggestion_kind_for_run = suggestion.kind.clone();
        let run_snapshot_for_run = run_snapshot;
        crate::spawn_logged!("src/channels/http.rs:19428", async move {
            let run_result: std::result::Result<(), String> = if suggestion_kind_for_run
                == "watcher"
            {
                let agent = state_for_run.agent.read().await;
                execute_accepted_watcher_suggestion(&agent, &suggestion_for_run, &trace_ref_for_run)
                    .await
            } else {
                let (token_tx, mut token_rx) =
                    tokio::sync::mpsc::channel::<crate::core::StreamEvent>(256);
                let drain = tokio::spawn(async move { while token_rx.recv().await.is_some() {} });
                let agent = state_for_run.agent.read().await;
                let result = agent
                    .process_message_stream_with_meta(
                        &prompt_for_run,
                        "autonomy",
                        None,
                        None,
                        trace_ref_for_run.clone(),
                        token_tx,
                    )
                    .await
                    .map(|_| ())
                    .map_err(|error| error.to_string());
                let _ = drain.await;
                result
            };
            let snapshot = trace::persist_live_trace_snapshot(
                &state_for_run.trace_history,
                &trace_ref_for_run,
            )
            .await
            .unwrap_or_else(ExecutionTrace::default);
            let resolved_trace_id = if snapshot.id.trim().is_empty() {
                trace_id_for_run.clone()
            } else {
                snapshot.id.clone()
            };
            let outcomes = suggestions::collect_run_outcomes(
                &state_for_run,
                &run_snapshot_for_run,
                &suggestion_kind_for_run,
            )
            .await;
            let completed_at = chrono::Utc::now().to_rfc3339();
            match run_result {
                Ok(_) => {
                    suggestions::update_chat_suggestion_after_run(
                        &storage_for_run,
                        &suggestion_id_for_run,
                        &resolved_trace_id,
                        "completed",
                        &completed_at,
                        None,
                        outcomes,
                    )
                    .await;
                }
                Err(error) => {
                    let err_text = error.to_string();
                    {
                        let mut trace = trace_ref_for_run.write().await;
                        trace.completed_at = Some(chrono::Utc::now());
                        trace.response = Some(err_text.clone());
                        trace.steps.push(crate::core::ExecutionStep {
                            icon: "[err]".to_string(),
                            title: "Suggestion run failed".to_string(),
                            detail: err_text.clone(),
                            step_type: "error".to_string(),
                            data: None,
                            timestamp: chrono::Utc::now(),
                            duration_ms: Some(0),
                        });
                    }
                    let _ = trace::persist_live_trace_snapshot(
                        &state_for_run.trace_history,
                        &trace_ref_for_run,
                    )
                    .await;
                    suggestions::update_chat_suggestion_after_run(
                        &storage_for_run,
                        &suggestion_id_for_run,
                        &resolved_trace_id,
                        "failed",
                        &completed_at,
                        Some(err_text),
                        outcomes,
                    )
                    .await;
                }
            }
        });
    }

    Ok(serde_json::json!({
        "status": "started",
        "trace_id": trace_id.clone(),
        "trace_path": format!("/trace/{}", trace_id),
        "run": {
            "kind": "suggestion_execution",
            "title": suggestion.title,
            "status": "running",
            "started_at": started_at_text,
            "summary": format!(
                "Launched a real {} execution run. Open the live trace to watch steps, tool output, and any app build/runtime logs.",
                suggestion_kind_title(&suggestion.kind).to_ascii_lowercase()
            ),
            "trace_id": trace_id
        }
    }))
}

pub(super) async fn dismiss_chat_suggestion(
    state: &AppState,
    suggestion_id: &str,
) -> Result<serde_json::Value, String> {
    let storage = { state.agent.read().await.storage.clone() };
    let mut suggestions = load_chat_suggestions(&storage).await;
    let Some(idx) = suggestions
        .iter()
        .position(|suggestion| suggestion.id == suggestion_id)
    else {
        return Err("Suggestion not found".to_string());
    };

    if suggestions[idx].status != "open" {
        return Err("Suggestion is no longer open".to_string());
    }

    let now = chrono::Utc::now().to_rfc3339();
    suggestions[idx].status = "dismissed".to_string();
    suggestions[idx].updated_at = now.clone();
    suggestions[idx].dismissed_at = Some(now);
    suggestions = prune_chat_suggestion_history(suggestions);
    save_chat_suggestions(&storage, &suggestions).await;

    Ok(serde_json::json!({ "status": "dismissed" }))
}

pub(super) async fn build_autonomy_briefing(
    agent: &Agent,
    settings: &AutonomySettings,
) -> AutonomyBriefingResponse {
    let readiness_policy = crate::core::readiness::load_readiness_policy(&agent.storage).await;
    let mut suggested_automations = load_chat_suggestions(&agent.storage).await;
    suggested_automations.retain(|suggestion| suggestion.status == "open");
    suggested_automations.sort_by(|a, b| {
        parse_rfc3339_utc(&b.updated_at)
            .cmp(&parse_rfc3339_utc(&a.updated_at))
            .then_with(|| a.id.cmp(&b.id))
    });
    suggested_automations.truncate(6);
    let mut suggestion_scan = load_chat_suggestion_scan_state(&agent.storage).await;
    suggestion_scan.tracked_chats = suggestion_scan.conversation_watermarks.len();
    suggestion_scan.conversation_watermarks.clear();
    if autonomy_background_disabled(settings) {
        suggestion_scan.last_status = Some("disabled".to_string());
        suggestion_scan.next_due_at = None;
    }

    let (
        pending_tasks,
        awaiting_approval,
        paused_tasks,
        failed_tasks,
        in_progress_tasks,
        total_tasks,
        strategic_goal,
    ) = {
        let tasks = agent.tasks.read().await;
        let total = tasks.all().len();
        let pending = tasks
            .all()
            .iter()
            .filter(|t| matches!(t.status, TaskStatus::Pending))
            .count();
        let awaiting = tasks
            .all()
            .iter()
            .filter(|t| matches!(t.status, TaskStatus::AwaitingApproval))
            .count();
        let paused = tasks
            .all()
            .iter()
            .filter(|t| matches!(t.status, TaskStatus::Paused))
            .count();
        let failed = tasks
            .all()
            .iter()
            .filter(|t| matches!(t.status, TaskStatus::Failed { .. }))
            .count();
        let in_progress = tasks
            .all()
            .iter()
            .filter(|t| matches!(t.status, TaskStatus::InProgress))
            .count();
        let strategic_goal = choose_briefing_goal_candidate(tasks.all());
        (
            pending,
            awaiting,
            paused,
            failed,
            in_progress,
            total,
            strategic_goal,
        )
    };

    let unread_alerts = agent
        .storage
        .list_notifications(50, 0, true)
        .await
        .unwrap_or_default()
        .into_iter()
        .filter(|n| n.level == "warning" || n.level == "error")
        .count();

    let security_logs = agent
        .storage
        .list_security_logs(80)
        .await
        .unwrap_or_default();
    let auth_failures = security_logs
        .iter()
        .filter(|s| s.event_type.to_ascii_lowercase().contains("auth"))
        .count();
    let security_spikes = security_logs
        .iter()
        .filter(|s| s.severity == "error" || s.severity == "critical")
        .count();

    let active_watchers = agent
        .watcher_manager
        .list()
        .await
        .into_iter()
        .filter(|w| matches!(w.status, crate::core::watcher::WatcherStatus::Active))
        .count();

    let completed_runs = {
        let trace = agent.trace_history.read().await;
        trace.iter().filter(|t| t.completed_at.is_some()).count()
    };

    let mut top_risks = Vec::new();
    if awaiting_approval > 0 {
        top_risks.push(serde_json::json!({
            "type": "approval_queue",
            "severity": "high",
            "title": "Approvals waiting",
            "detail": format!("{} task(s) are blocked pending approval", awaiting_approval),
        }));
    }
    if failed_tasks > 0 {
        top_risks.push(serde_json::json!({
            "type": "execution_failures",
            "severity": "high",
            "title": "Recent task failures",
            "detail": format!("{} failed task(s) need triage", failed_tasks),
        }));
    }
    if paused_tasks > 0 {
        top_risks.push(serde_json::json!({
            "type": "paused_tasks",
            "severity": "medium",
            "title": "Paused automations",
            "detail": format!("{} task(s) are paused and waiting to be resumed", paused_tasks),
        }));
    }
    if unread_alerts > 0 {
        top_risks.push(serde_json::json!({
            "type": "alerts",
            "severity": "medium",
            "title": "Unread alerts",
            "detail": format!("{} warning/error notification(s) are unread", unread_alerts),
        }));
    }
    if auth_failures > 0 {
        top_risks.push(serde_json::json!({
            "type": "auth_failures",
            "severity": "critical",
            "title": "Authentication pressure",
            "detail": format!("{} auth-related security events were logged recently", auth_failures),
        }));
    }
    let mut top_opportunities = Vec::new();
    if completed_runs > 0 {
        top_opportunities.push(serde_json::json!({
            "type": "throughput",
            "title": "Strong execution throughput",
            "detail": format!("{} run(s) completed recently - capture lessons into reusable routines", completed_runs),
        }));
    }
    if active_watchers > 0 {
        top_opportunities.push(serde_json::json!({
            "type": "automation",
            "title": "Automation already active",
            "detail": format!("{} watcher(s) are actively monitoring external conditions", active_watchers),
        }));
    }
    if pending_tasks == 0 && paused_tasks == 0 && in_progress_tasks == 0 {
        top_opportunities.push(serde_json::json!({
            "type": "capacity",
            "title": "High strategic capacity",
            "detail": "No active queue pressure - good window for high-leverage planning",
        }));
    }
    if !suggested_automations.is_empty() {
        top_opportunities.push(serde_json::json!({
            "type": "chat_suggestions",
            "title": "Uncaptured chat opportunities",
            "detail": format!(
                "{} suggestion draft(s) are waiting in Mission Control. Chat scan status: {}.",
                suggested_automations.len(),
                chat_suggestion_display_status(suggestion_scan.last_status.as_deref().unwrap_or("scheduled"))
            ),
        }));
    }
    if top_opportunities.is_empty() {
        top_opportunities.push(serde_json::json!({
            "type": "stability",
            "title": "Stable operating window",
            "detail": "Use this period to improve automation and documentation coverage",
        }));
    }

    let mut recommended_actions = Vec::new();
    if awaiting_approval > 0 {
        recommended_actions.push(recommendation(
            "Resolve Approval Queue",
            "Review blocked tasks and make explicit approve/reject decisions.",
            "chat_prompt",
            serde_json::json!({"prompt":"Show tasks awaiting approval with recommended decisions and expected impact."}),
            &settings.trust_policy,
            &readiness_policy,
        ));
    }
    if unread_alerts > 0 || security_spikes > 0 {
        recommended_actions.push(recommendation(
            "Enable Ops Mode",
            "Apply the Ops preset: create monitoring watchers and incident-focused routines, and make Ops the active autonomy mode.",
            "activate_mode",
            serde_json::json!({"mode_id":"ops"}),
            &settings.trust_policy,
            &readiness_policy,
        ));
    }
    // Only suggest daily brief when the agent has a notification channel configured.
    let has_notification_channel = agent.config.telegram.is_some()
        || agent.config.slack.is_some()
        || agent.config.discord.is_some();
    if has_notification_channel {
        recommended_actions.push(recommendation(
            "Send Daily Command Brief",
            "Generate today's executive brief and push it to your preferred channel.",
            "daily_brief_now",
            serde_json::json!({}),
            &settings.trust_policy,
            &readiness_policy,
        ));
    }
    // Only suggest delegation when swarm is ready and has specialists.
    let swarm_ready = agent.swarm.is_some() && !agent.config.swarm.specialists.is_empty();
    if recommended_actions.len() < 3 && swarm_ready {
        if let Some(goal) = strategic_goal {
            let context = if let Some(due_date) = goal.due_date.as_deref() {
                format!(
                    "Break this tracked goal into execution tracks, risks, dependencies, and first actions. Due date: {}.",
                    due_date
                )
            } else {
                "Break this tracked goal into execution tracks, risks, dependencies, and first actions."
                    .to_string()
            };
            recommended_actions.push(recommendation(
                "Decompose Active Goal",
                "Use swarm delegation to break the current tracked goal into execution tracks, risks, dependencies, and first actions.",
                "delegate",
                serde_json::json!({
                    "task": goal.objective,
                    "context": context,
                    "source_task_id": goal.task_id,
                }),
                &settings.trust_policy,
                &readiness_policy,
            ));
        }
    }
    recommended_actions.truncate(3);
    for action in &recommended_actions {
        if let Some(readiness) = action.readiness.as_ref() {
            if let Err(error) = crate::core::readiness::record_readiness_evaluation(
                &agent.storage,
                "recommended_action",
                &action.id,
                readiness,
            )
            .await
            {
                tracing::warn!(
                    action_id = %action.id,
                    error = %error,
                    "Failed to record recommended action readiness evaluation"
                );
            }
        }
    }

    AutonomyBriefingResponse {
        generated_at: chrono::Utc::now().to_rfc3339(),
        scope: settings.context_scope.as_storage_str().to_string(),
        top_risks,
        top_opportunities,
        trust_summary: serde_json::json!({
            "auto_execute_max_score": settings.trust_policy.auto_execute_max_score,
            "blocked_actions": settings.trust_policy.blocked_actions,
            "approval_actions": settings.trust_policy.always_require_approval_actions,
            "queue": {
                "pending_tasks": pending_tasks,
                "awaiting_approval": awaiting_approval,
                "paused_tasks": paused_tasks,
                "in_progress_tasks": in_progress_tasks,
                "total_tasks": total_tasks,
            }
        }),
        recommended_actions,
        suggested_automations,
        suggestion_scan,
    }
}

pub(super) async fn run_recommended_action(
    agent: &Agent,
    settings: &mut AutonomySettings,
    action: &RecommendedAction,
    dry_run: bool,
) -> Result<serde_json::Value, String> {
    let trust = score_action_risk(&action.action_kind, &action.payload, &settings.trust_policy);
    if trust.blocked {
        return Err("Action blocked by trust policy".to_string());
    }
    let readiness_policy = crate::core::readiness::load_readiness_policy(&agent.storage).await;
    let readiness = action.readiness.clone().unwrap_or_else(|| {
        crate::core::readiness::evaluate_recommended_action_readiness(&trust, &readiness_policy)
    });

    if dry_run {
        return Ok(serde_json::json!({
            "dry_run": true,
            "action_id": action.id,
            "risk": { "level": risk_level_label(&trust.level), "score": trust.score, "requires_approval": trust.requires_approval, "reasons": trust.reasons },
            "readiness": readiness,
        }));
    }

    if crate::core::task_requires_explicit_approval(&TaskApproval::RequireApproval)
        && trust.requires_approval
    {
        let mut approval_task = Task::new(
            format!("Approval required: {}", action.title),
            "autonomy_action".to_string(),
            serde_json::json!({
                "autonomy_action_kind": action.action_kind.clone(),
                "autonomy_action_payload": action.payload.clone(),
                "_approval": {
                    "title": action.title.clone(),
                    "summary": action.description.clone(),
                    "reason": trust.reasons.join("; "),
                    "rule_name": "elevated_action_requires_explicit_approval",
                    "risk_level": risk_level_label(&trust.level),
                    "risk_score": trust.score,
                    "source": "autonomy"
                }
            }),
        );
        approval_task.approval = TaskApproval::RequireApproval;
        approval_task.status = TaskStatus::AwaitingApproval;
        let (task_id, reused_existing, removed_duplicates) = agent
            .add_or_update_similar_task(approval_task, false, None)
            .await
            .map_err(|e| e.to_string())?;
        return Ok(serde_json::json!({
            "status": "queued_for_approval",
            "action_id": action.id,
            "task_id": task_id,
            "reused_existing": reused_existing,
            "removed_duplicates": removed_duplicates,
            "risk": { "level": risk_level_label(&trust.level), "score": trust.score },
            "readiness": readiness,
        }));
    }

    let result = agent
        .execute_autonomy_action_payload(settings, &action.action_kind, &action.payload)
        .await;
    if result.is_ok() {
        agent.record_self_tune_autonomous_success().await;
    }
    result
}

pub(super) async fn start_codex_cli_oauth() -> Response {
    let runtime = codex_oauth_runtime();

    match spawn_codex_oauth_probe().await {
        Ok(()) => {
            let snapshot = runtime.read().await.clone();
            let auth_url = snapshot.auth_url.unwrap_or_default();
            let device_code = snapshot.device_code.unwrap_or_default();
            let message = if !auth_url.is_empty() && !device_code.is_empty() {
                format!(
                    "Open the URL below and enter code {}. After completion, click Check Status.",
                    device_code
                )
            } else {
                "OAuth flow started. Waiting for device code...".to_string()
            };

            let opened_browser = if !auth_url.is_empty() && server_can_launch_local_browser() {
                open_url_in_default_browser(&auth_url).await.is_ok()
            } else {
                false
            };

            (
                StatusCode::OK,
                Json(CodexCliOAuthStartResponse {
                    started: true,
                    running: snapshot.active,
                    opened_browser,
                    auth_url,
                    device_code,
                    message,
                }),
            )
                .into_response()
        }
        Err(e) => (
            StatusCode::OK,
            Json(CodexCliOAuthStartResponse {
                started: false,
                running: false,
                opened_browser: false,
                auth_url: String::new(),
                device_code: String::new(),
                message: format!("OAuth failed: {}", e),
            }),
        )
            .into_response(),
    }
}

pub(super) async fn codex_cli_oauth_status() -> Response {
    let client = reqwest::Client::builder()
        .timeout(std::time::Duration::from_secs(10))
        .build()
        .unwrap_or_default();
    let has_api_key = resolve_codex_cli_api_key(&client, false)
        .await
        .ok()
        .flatten()
        .map(|k| !k.trim().is_empty())
        .unwrap_or(false);
    let runtime = codex_oauth_runtime();
    let snapshot = runtime.read().await.clone();
    let auth_url = snapshot.auth_url.unwrap_or_default();
    let device_code = snapshot.device_code.unwrap_or_default();

    let message = if has_api_key {
        "OpenAI Subscription connected and ready.".to_string()
    } else if snapshot.active {
        if !auth_url.is_empty() && !device_code.is_empty() {
            format!(
                "Waiting for OAuth completion. Open URL and enter code {}, then click Check Status again.",
                device_code
            )
        } else {
            "OAuth flow is running, waiting for authorization...".to_string()
        }
    } else if let Some(err) = &snapshot.last_error {
        format!("OAuth failed: {}", err)
    } else if !snapshot.last_output.is_empty() && snapshot.last_output.contains("successfully") {
        "OpenAI Subscription connected and ready.".to_string()
    } else {
        "OpenAI Subscription is not connected. Click 'Connect via Browser' to start OAuth."
            .to_string()
    };

    (
        StatusCode::OK,
        Json(CodexCliOAuthStatusResponse {
            connected: has_api_key,
            has_api_key,
            running: snapshot.active,
            auth_url,
            device_code,
            message,
        }),
    )
        .into_response()
}

#[cfg(test)]
mod background_session_visibility_tests {
    use super::*;

    fn test_session(task: &Task) -> crate::core::BackgroundSession {
        let now = chrono::Utc::now();
        crate::core::BackgroundSession {
            id: "bg-test".to_string(),
            title: "Test session".to_string(),
            objective: "Test objective".to_string(),
            status: crate::core::BackgroundSessionStatus::Active,
            summary: None,
            current_focus: None,
            waiting_on: None,
            next_expected_action: None,
            working_memory: None,
            last_error: None,
            preferred_delivery_channel: None,
            channel: Some("test".to_string()),
            conversation_id: Some("conv-test".to_string()),
            project_id: None,
            linked_task_ids: vec![task.id.to_string()],
            linked_watcher_ids: Vec::new(),
            policy: crate::core::BackgroundSessionPolicy::default(),
            created_at: now,
            updated_at: now,
            last_activity_at: now,
            last_consolidated_at: None,
            events: Vec::new(),
        }
    }

    #[test]
    fn background_session_ui_kind_hides_chat_only_context() {
        let task = Task::new(
            "hello".to_string(),
            "chat_request".to_string(),
            serde_json::json!({}),
        );
        let session = test_session(&task);
        let tasks = vec![task];
        let counts = collect_background_session_counts(&session.id, &session, &tasks, &[]);

        assert_eq!(
            background_session_ui_kind(&session.id, &session, &counts, &tasks),
            "chat_context"
        );
        assert_eq!(
            background_session_list_item_json(&session, &counts, &tasks)["default_visible"],
            serde_json::Value::Bool(false)
        );
    }

    #[test]
    fn background_session_ui_kind_shows_real_background_task() {
        let task = Task::new(
            "build the page".to_string(),
            "app_deploy".to_string(),
            serde_json::json!({}),
        );
        let session = test_session(&task);
        let tasks = vec![task];
        let counts = collect_background_session_counts(&session.id, &session, &tasks, &[]);

        assert_eq!(
            background_session_ui_kind(&session.id, &session, &counts, &tasks),
            "default"
        );
        assert_eq!(
            background_session_list_item_json(&session, &counts, &tasks)["default_visible"],
            serde_json::Value::Bool(true)
        );
    }
}
