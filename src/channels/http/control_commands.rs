use super::*;

pub(super) fn parse_autonomy_quick_command(message: &str) -> Option<AutonomyQuickCommand> {
    let trimmed = message.trim();
    let command = trimmed.strip_prefix('/')?;
    let normalized = command.to_ascii_lowercase().replace(['_', '-'], " ");
    let normalized = normalized.split_whitespace().collect::<Vec<_>>().join(" ");
    if normalized == "triage" {
        return Some(AutonomyQuickCommand::TriageInbox);
    }
    if let Some(task) = command.strip_prefix("delegate ") {
        let task = task.trim();
        if task.is_empty() {
            return None;
        }
        return Some(AutonomyQuickCommand::Delegate {
            task: task.to_string(),
            require_approval: false,
        });
    }
    if let Some(rest) = command.strip_prefix("rollback ") {
        let rest = rest.trim();
        if rest.is_empty() {
            return None;
        }
        let mut parts = rest.split_whitespace();
        let event_id = parts.next().unwrap_or("").trim();
        if event_id.is_empty() {
            return None;
        }
        let operation = if let Some(raw) = parts.next() {
            let op = raw.trim().to_ascii_lowercase();
            if op == "mark" {
                parts.next().map(|next| {
                    let next = next.trim().to_ascii_lowercase();
                    match next.as_str() {
                        "read" => "mark_read".to_string(),
                        "unread" => "mark_unread".to_string(),
                        other => format!("mark {}", other),
                    }
                })
            } else {
                Some(match op.as_str() {
                    "read" => "mark_read".to_string(),
                    "unread" => "mark_unread".to_string(),
                    _ => op,
                })
            }
        } else {
            None
        };
        return Some(AutonomyQuickCommand::Rollback {
            event_id: event_id.to_string(),
            operation,
        });
    }
    None
}

pub(super) fn parse_notification_control_command(
    message: &str,
) -> Option<NotificationControlCommand> {
    let command = message.trim().strip_prefix('/')?;
    match command
        .to_ascii_lowercase()
        .replace(['_', '-'], " ")
        .split_whitespace()
        .collect::<Vec<_>>()
        .join(" ")
        .as_str()
    {
        "notifications pause" => Some(NotificationControlCommand::Pause24h),
        "notifications resume" => Some(NotificationControlCommand::Resume),
        "notifications status" => Some(NotificationControlCommand::Status),
        _ => None,
    }
}

pub(super) async fn handle_notification_control_command(
    state: &AppState,
    cmd: NotificationControlCommand,
) -> std::result::Result<String, String> {
    let agent = state.agent.read().await;
    match cmd {
        NotificationControlCommand::Pause24h => {
            let until_ts = agent
                .pause_push_notifications_for_hours(24)
                .await
                .map_err(|e| format!("Failed to pause notifications: {}", e))?;
            let until = chrono::DateTime::<chrono::Utc>::from_timestamp(until_ts, 0)
                .unwrap_or_else(chrono::Utc::now);
            Ok(format!(
                "Push notifications paused until {}. Use `/notifications resume` anytime to re-enable.",
                until.format("%Y-%m-%d %H:%M:%S UTC")
            ))
        }
        NotificationControlCommand::Resume => {
            agent
                .resume_push_notifications()
                .await
                .map_err(|e| format!("Failed to resume notifications: {}", e))?;
            Ok("Push notifications resumed.".to_string())
        }
        NotificationControlCommand::Status => {
            if let Some(until_ts) = agent.push_notifications_muted_until_ts().await {
                let until = chrono::DateTime::<chrono::Utc>::from_timestamp(until_ts, 0)
                    .unwrap_or_else(chrono::Utc::now);
                Ok(format!(
                    "Push notifications are currently paused until {}.",
                    until.format("%Y-%m-%d %H:%M:%S UTC")
                ))
            } else {
                Ok("Push notifications are active.".to_string())
            }
        }
    }
}

pub(super) async fn server_load_reasons(state: &AppState) -> Vec<String> {
    let now = chrono::Utc::now();
    let mut reasons = {
        let tasks = state.tasks.read().await;
        tasks
            .all()
            .iter()
            .filter(|task| matches!(task.status, TaskStatus::InProgress))
            .filter_map(|task| {
                let age_secs = (now - task.created_at).num_seconds().max(0);
                if age_secs > SERVER_BUSY_TASK_WINDOW_SECS {
                    None
                } else {
                    Some(format!(
                        "task '{}' in progress ({}s)",
                        task.action, age_secs
                    ))
                }
            })
            .take(3)
            .collect::<Vec<_>>()
    };

    let active_trace_reason = {
        let last_trace = state.last_trace.read().await;
        last_trace
            .started_at
            .filter(|_| last_trace.completed_at.is_none())
            .and_then(|started_at| {
                let age_secs = (now - started_at).num_seconds().max(0);
                if age_secs > SERVER_BUSY_TRACE_WINDOW_SECS {
                    None
                } else {
                    Some(format!("active trace ({}s)", age_secs))
                }
            })
    };
    if let Some(reason) = active_trace_reason {
        reasons.push(reason);
    }

    reasons
}

pub(super) async fn server_under_load(state: &AppState) -> bool {
    !server_load_reasons(state).await.is_empty()
}

pub(super) async fn handle_autonomy_quick_command(
    state: &AppState,
    cmd: AutonomyQuickCommand,
) -> std::result::Result<String, String> {
    match cmd {
        AutonomyQuickCommand::TriageInbox => {
            let labels = vec![
                "Act now".to_string(),
                "Delegate".to_string(),
                "Ignore".to_string(),
            ];
            let agent = state.agent.read().await;
            let fallback = agent
                .storage
                .list_notifications(30, 0, true)
                .await
                .unwrap_or_default();
            let messages: Vec<serde_json::Value> = fallback
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
            if messages.is_empty() {
                return Ok(
                    "No inbox items found to triage. Unread notifications are already clear."
                        .to_string(),
                );
            }

            let payload = serde_json::json!({ "messages": messages, "labels": labels });
            let llm_response = agent
                .llm
                .chat(
                    "You are an executive inbox triage assistant. Return strict JSON {\"triage\":[{\"message_id\":\"...\",\"label\":\"...\",\"reason\":\"...\",\"draft_reply\":\"...\"}]}.",
                    &payload.to_string(),
                    &[],
                    &[],
                )
                .await
                .ok();
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

            let rows = parsed
                .get("triage")
                .and_then(|v| v.as_array())
                .cloned()
                .unwrap_or_default();
            if rows.is_empty() {
                return Ok("Inbox triage complete. No items were classified.".to_string());
            }
            let mut out = format!("Inbox triage complete: {} item(s).\n", rows.len());
            for row in rows.iter().take(10) {
                let id = row
                    .get("message_id")
                    .and_then(|v| v.as_str())
                    .map(|s| s.to_string())
                    .unwrap_or_else(|| {
                        row.get("message_id")
                            .map(|v| v.to_string())
                            .unwrap_or("-".to_string())
                    });
                let label = row.get("label").and_then(|v| v.as_str()).unwrap_or("-");
                let reason = row.get("reason").and_then(|v| v.as_str()).unwrap_or("-");
                out.push_str(&format!("- [{}] {}: {}\n", label, id, reason));
            }
            if rows.len() > 10 {
                out.push_str(&format!("... and {} more.\n", rows.len() - 10));
            }
            out.push_str("Tip: use /delegate <task> for anything labeled Delegate.");
            Ok(out)
        }
        AutonomyQuickCommand::Delegate {
            task,
            require_approval,
        } => {
            if task.trim().is_empty() {
                return Err("Task is required. Usage: /delegate <task description>".to_string());
            }
            let agent = state.agent.read().await;
            let settings = load_autonomy_settings(&agent).await;
            let trust = score_action_risk(
                "delegate",
                &serde_json::json!({"task": task}),
                &settings.trust_policy,
            );

            if crate::core::task_requires_explicit_approval(&TaskApproval::RequireApproval)
                && (trust.requires_approval || require_approval)
            {
                let mut approval_task = Task::new(
                    format!("Delegation approval: {}", task),
                    "delegate".to_string(),
                    serde_json::json!({
                        "task": task,
                        "context": "",
                        "_approval": {
                            "title": format!("Delegate: {}", task),
                            "summary": "This delegation will spawn specialist/background work on your behalf.",
                            "reason": trust.reasons.join("; "),
                            "rule_name": "elevated_action_requires_explicit_approval",
                            "risk_level": risk_level_label(&trust.level),
                            "risk_score": trust.score,
                            "source": "autonomy_quick_command"
                        }
                    }),
                );
                approval_task.status = TaskStatus::AwaitingApproval;
                approval_task.approval = TaskApproval::RequireApproval;
                let queued = agent
                    .add_or_update_similar_task(approval_task, false, None)
                    .await;
                if let Err(e) = queued {
                    return Err(format!("Failed to queue delegation approval: {}", e));
                }
                return Ok(format!(
                    "Delegation queued for approval (risk: {} / score {}).",
                    risk_level_label(&trust.level),
                    trust.score
                ));
            }

            let actions = agent
                .runtime
                .list_enabled_actions()
                .await
                .unwrap_or_default();
            let active_prompt_bundle = agent.active_prompt_bundle_for_message(&task).await;
            let active_specialist_prompt_bundle = agent
                .active_specialist_prompt_bundle_for_message(&task)
                .await;
            let decision = crate::core::task_router::RoutingDecision {
                needs_delegation: true,
                complexity: crate::core::QueryComplexity::Complex,
                sub_agents: agent.forced_swarm_specs(&task, &actions),
                reasoning: "Delegation was explicitly requested by the control command."
                    .to_string(),
                confidence: 0.96,
                should_clarify: false,
                clarification_question: None,
            };
            let system_prompt = match agent
                .build_system_prompt(&[], Some(&active_prompt_bundle))
                .await
            {
                Ok(prompt) => prompt,
                Err(error) => {
                    return Err(format!("Failed to build delegation prompt: {}", error));
                }
            };
            let delegation_id = uuid::Uuid::new_v4().to_string();
            let empty_memories: Vec<crate::core::PromptMemory> = Vec::new();
            let specialists = agent
                .swarm
                .as_ref()
                .map(|manager| manager.specialists.clone());
            let action_scope_hints = agent
                .runtime
                .list_action_scope_hints()
                .await
                .unwrap_or_default();
            let delegation_user_selected_model_slot_id = agent
                .user_selected_model_slot_id
                .read()
                .ok()
                .and_then(|guard| guard.clone());
            let trace = Arc::new(RwLock::new(crate::core::ExecutionTrace {
                id: uuid::Uuid::new_v4().to_string(),
                message: task.clone(),
                channel: "http".to_string(),
                started_at: Some(chrono::Utc::now()),
                ..Default::default()
            }));
            match agent
                .task_router
                .execute(
                    &decision,
                    crate::core::task_router::TaskRouterExecuteContext {
                        delegation_id: &delegation_id,
                        conversation_id: None,
                        channel: Some("http"),
                        message: &task,
                        system_prompt: &system_prompt,
                        prompt_bundle: &active_prompt_bundle,
                        specialist_prompt_bundle: &active_specialist_prompt_bundle,
                        configured_model_slots: &agent.config.model_pool.slots,
                        model_pool: &agent.model_pool,
                        primary_model_id: &agent.primary_model_id,
                        user_selected_model_slot_id: delegation_user_selected_model_slot_id
                            .as_deref(),
                        smart_routing: agent.config.model_pool.smart_routing,
                        primary_llm: &agent.llm,
                        specialists: &specialists,
                        memories: &empty_memories,
                        actions: &actions,
                        action_scope_hints: &action_scope_hints,
                        trace: &trace,
                        token_tx: None,
                        swarm_activity: Some(&agent.swarm_activity),
                        storage: Some(&agent.storage),
                    },
                )
                .await
            {
                Ok(crate::core::task_router::TaskRouterResult::Delegated(result)) => {
                    let final_result = crate::security::redact_pii(&result.final_response.content);
                    let degradation = if result.degradation.is_empty() {
                        "-".to_string()
                    } else {
                        result
                            .degradation
                            .iter()
                            .map(|note| match note.detail.as_deref() {
                                Some(detail) if !detail.is_empty() => {
                                    format!("{}: {} ({})", note.kind, note.summary, detail)
                                }
                                _ => format!("{}: {}", note.kind, note.summary),
                            })
                            .collect::<Vec<_>>()
                            .join("\n")
                    };
                    Ok(format!(
                        "Delegation complete.\nAgents used: {}\nStatus: {}\nDegradation: {}\nResult:\n{}",
                        if result.agent_results.is_empty() {
                            "-".to_string()
                        } else {
                            result
                                .agent_results
                                .iter()
                                .map(|item| {
                                    item.agent_name
                                        .clone()
                                        .unwrap_or_else(|| item.agent_type.clone())
                                })
                                .collect::<Vec<_>>()
                                .join(", ")
                        },
                        result.delegation_status.as_str(),
                        degradation,
                        final_result
                    ))
                }
                Ok(crate::core::task_router::TaskRouterResult::Direct) => {
                    Err("Delegation unexpectedly routed to a direct path.".to_string())
                }
                Err(e) => Err(format!("Delegation failed: {}", e)),
            }
        }
        AutonomyQuickCommand::Rollback {
            event_id,
            operation,
        } => {
            let agent = state.agent.read().await;
            let event_id_trimmed = event_id.trim();
            let operation = operation.unwrap_or_default();

            if let Some(task_id) = event_id_trimmed.strip_prefix("task:") {
                let uuid = uuid::Uuid::parse_str(task_id)
                    .map_err(|_| "Invalid task id. Expected format: task:<uuid>".to_string())?;
                let mut tasks = agent.tasks.write().await;
                let Some(task) = tasks.get_mut(uuid) else {
                    return Err("Task not found.".to_string());
                };
                if !matches!(
                    task.status,
                    TaskStatus::Pending | TaskStatus::AwaitingApproval | TaskStatus::InProgress
                ) {
                    return Err("Task cannot be cancelled from its current state.".to_string());
                }
                task.status = TaskStatus::Cancelled;
                let status_json =
                    serde_json::to_string(&task.status).unwrap_or("\"Cancelled\"".to_string());
                let _ = agent
                    .storage
                    .update_task_status(task_id, &status_json)
                    .await;
                return Ok("Rollback applied: task cancelled.".to_string());
            }

            if let Some(watcher_id) = event_id_trimmed.strip_prefix("watcher:") {
                let uuid = uuid::Uuid::parse_str(watcher_id).map_err(|_| {
                    "Invalid watcher id. Expected format: watcher:<uuid>".to_string()
                })?;
                if agent.watcher_manager.cancel(uuid).await {
                    if let Some(watcher) = agent.watcher_manager.get(uuid).await {
                        agent
                            .sync_watcher_supervisor_state(&watcher, Some("cancelled"), None)
                            .await;
                    }
                    return Ok("Rollback applied: watcher cancelled.".to_string());
                }
                return Err("Watcher not found or not cancellable.".to_string());
            }

            if let Some(notification_id) = event_id_trimmed.strip_prefix("notification:") {
                let read = operation != "mark_unread";
                agent
                    .storage
                    .set_notification_read(notification_id, read)
                    .await
                    .map_err(|e| format!("Failed to update notification: {}", e))?;
                return Ok(if read {
                    "Rollback applied: notification marked as read.".to_string()
                } else {
                    "Rollback applied: notification marked as unread.".to_string()
                });
            }

            Err("Unsupported rollback target. Use task:<uuid>, watcher:<uuid>, or notification:<id>.".to_string())
        }
    }
}
