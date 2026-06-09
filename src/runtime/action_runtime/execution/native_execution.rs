use super::super::*;

impl ActionRuntime {
    /// Execute an action natively (no sandbox)
    pub(in crate::runtime) async fn execute_native(
        &self,
        action_name: &str,
        arguments: &serde_json::Value,
    ) -> Result<String> {
        match action_name {
            "app_deploy" => anyhow::bail!(
                "app_deploy requires the agent app-host execution context; call it through the agent tool executor so app registry, streaming, and validation are available"
            ),
            "file_read" => {
                let path = arguments["path"]
                    .as_str()
                    .ok_or_else(|| anyhow::anyhow!("Missing path"))?;
                let path = self.resolve_tool_read_path(path)?;
                let bytes = tokio::fs::read(&path).await?;
                let mime = mime_guess::from_path(&path).first_raw().map(str::to_string);
                if runtime_response_body_is_probably_binary(mime.as_deref().unwrap_or(""), &bytes)
                    || std::str::from_utf8(&bytes).is_err()
                {
                    let path_text = Self::display_tool_path(&path);
                    let resource = RuntimeResourceRef {
                        id: format!("file:{}", Self::fingerprint_text(&[path_text.as_str()])),
                        path: path_text.clone(),
                        mime,
                        bytes: bytes.len() as u64,
                        created_at: chrono::Utc::now().to_rfc3339(),
                        source_action: Some("file_read".to_string()),
                    };
                    return Ok(structured_tool_completion_output(
                        "file_read",
                        "completed",
                        format!("Read binary file resource {}.", path_text),
                        serde_json::json!({
                            "payload": {
                                "kind": "resource",
                                "resource": resource,
                            },
                            "body_quality": {
                                "body_bytes": bytes.len(),
                                "binary": true,
                                "degenerate": false,
                            }
                        }),
                    ));
                }
                Ok(String::from_utf8(bytes)?)
            }
            "file_write" => {
                let path = arguments["path"]
                    .as_str()
                    .ok_or_else(|| anyhow::anyhow!("Missing path"))?;
                let payload = self.file_write_payload_from_arguments(arguments).await?;
                let path = self.resolve_tool_write_path(path)?;
                if let Some(parent) = path.parent() {
                    tokio::fs::create_dir_all(parent).await?;
                }
                tokio::fs::write(&path, &payload.bytes).await?;
                Self::set_private_file_permissions(&path).await?;
                let mime = payload
                    .mime
                    .clone()
                    .or_else(|| mime_guess::from_path(&path).first_raw().map(str::to_string));
                let document = self
                    .index_file_write_document_if_requested(
                        &path,
                        arguments,
                        &payload,
                        mime.as_deref(),
                    )
                    .await;
                Ok(self.file_write_completion_output(&path, &payload, document.as_ref()))
            }
            "file_search" => self.execute_file_search(arguments).await,
            "file_delete" => self.execute_file_delete(arguments).await,
            "file_patch" => self.execute_file_patch(arguments).await,
            "goal_manage" => self.execute_goal_manage(arguments).await,
            "curator" => self.execute_curator_control(arguments).await,
            "skill_view" => self.execute_skill_view(arguments).await,
            "skill_manage" => self.execute_skill_manage(arguments).await,
            "page_fetch" => self.execute_page_fetch(arguments).await,
            "http_request" => self.execute_http_request(arguments).await,
            name if Self::is_browser_wrapper_action(name) => {
                self.execute_browser_wrapper_action(name, arguments).await
            }
            "clipboard_read" => {
                let mut clipboard = arboard::Clipboard::new()
                    .map_err(|e| anyhow::anyhow!("Failed to access clipboard: {}", e))?;
                let content = clipboard
                    .get_text()
                    .map_err(|e| anyhow::anyhow!("Failed to read clipboard: {}", e))?;
                Ok(content)
            }
            "clipboard_write" => {
                let content = arguments["content"]
                    .as_str()
                    .ok_or_else(|| anyhow::anyhow!("Missing content"))?;
                let mut clipboard = arboard::Clipboard::new()
                    .map_err(|e| anyhow::anyhow!("Failed to access clipboard: {}", e))?;
                clipboard
                    .set_text(content)
                    .map_err(|e| anyhow::anyhow!("Failed to write clipboard: {}", e))?;
                Ok("Content copied to clipboard".to_string())
            }
            "list_tasks" => {
                let queue = self
                    .task_queue
                    .as_ref()
                    .ok_or_else(|| anyhow::anyhow!("Task queue not available"))?;
                let tasks = queue.read().await;
                let filter = arguments
                    .get("filter")
                    .and_then(|v| v.as_str())
                    .unwrap_or("pending");

                let filtered: Vec<_> = tasks
                    .all()
                    .iter()
                    .filter(|t| match filter {
                        "pending" => matches!(
                            t.status,
                            crate::core::TaskStatus::Pending
                                | crate::core::TaskStatus::AwaitingApproval
                        ),
                        "paused" => matches!(t.status, crate::core::TaskStatus::Paused),
                        "goals" => t.action == "goal",
                        "routines" => t.cron.is_some(),
                        "completed" => matches!(t.status, crate::core::TaskStatus::Completed),
                        "failed" => matches!(t.status, crate::core::TaskStatus::Failed { .. }),
                        _ => true, // "all"
                    })
                    .collect();

                if filtered.is_empty() {
                    return Ok(format!("No {} items found.", filter));
                }

                let mut output = format!("Found {} {} item(s):\n\n", filtered.len(), filter);
                for t in &filtered {
                    let status_str = match &t.status {
                        crate::core::TaskStatus::Pending => "Pending",
                        crate::core::TaskStatus::AwaitingApproval => "Awaiting Approval",
                        crate::core::TaskStatus::ExpiredNeedsReapproval => {
                            "Expired - Needs Reapproval"
                        }
                        crate::core::TaskStatus::Paused => "Paused",
                        crate::core::TaskStatus::InProgress => "In Progress",
                        crate::core::TaskStatus::Completed => "Completed",
                        crate::core::TaskStatus::Failed { .. } => "Failed",
                        crate::core::TaskStatus::Cancelled => "Cancelled",
                    };
                    output.push_str(&format!(
                        "- {} (id: {}, action: {}, status: {})\n",
                        t.description, t.id, t.action, status_str
                    ));
                    if let Some(ref cron) = t.cron {
                        output.push_str(&format!("  Schedule: {}\n", cron));
                    }
                    if let Some(scheduled_for) = t.scheduled_for {
                        output.push_str(&format!("  Next run: {}\n", scheduled_for.to_rfc3339()));
                    }
                }
                Ok(output)
            }
            "list_watchers" => {
                let storage = self
                    .storage
                    .as_ref()
                    .ok_or_else(|| anyhow::anyhow!("Storage not available"))?;
                let filter = arguments
                    .get("filter")
                    .and_then(|value| value.as_str())
                    .map(str::trim)
                    .filter(|value| !value.is_empty())
                    .unwrap_or("active");
                let limit = arguments
                    .get("limit")
                    .and_then(|value| value.as_u64())
                    .unwrap_or(20)
                    .clamp(1, 100) as usize;
                let mut watchers = storage.list_watchers().await?;
                watchers.sort_by(|left, right| right.created_at.cmp(&left.created_at));
                let status_label =
                    |status: &crate::core::automation::watcher::WatcherStatus| -> &'static str {
                        match status {
                            crate::core::automation::watcher::WatcherStatus::Active => "active",
                            crate::core::automation::watcher::WatcherStatus::Paused => "paused",
                            crate::core::automation::watcher::WatcherStatus::Triggered => {
                                "triggered"
                            }
                            crate::core::automation::watcher::WatcherStatus::TimedOut => {
                                "timed_out"
                            }
                            crate::core::automation::watcher::WatcherStatus::Cancelled => {
                                "cancelled"
                            }
                            crate::core::automation::watcher::WatcherStatus::Failed { .. } => {
                                "failed"
                            }
                        }
                    };
                let rows = watchers
                    .into_iter()
                    .filter(|watcher| filter == "all" || status_label(&watcher.status) == filter)
                    .take(limit)
                    .map(|watcher| {
                        let status = status_label(&watcher.status);
                        let status_error = match &watcher.status {
                            crate::core::automation::watcher::WatcherStatus::Failed { error } => {
                                Some(error.clone())
                            }
                            _ => None,
                        };
                        serde_json::json!({
                            "id": watcher.id.to_string(),
                            "description": watcher.description,
                            "poll_action": watcher.poll_action,
                            "condition": watcher.condition,
                            "status": status,
                            "status_error": status_error,
                            "interval_secs": watcher.interval_secs,
                            "timeout_secs": watcher.timeout_secs,
                            "repeat_on_match": watcher.repeat_on_match,
                            "poll_count": watcher.poll_count,
                            "created_at": watcher.created_at.to_rfc3339(),
                            "last_poll_at": watcher.last_poll_at.map(|value| value.to_rfc3339()),
                            "next_poll_not_before": watcher
                                .next_poll_not_before
                                .map(|value| value.to_rfc3339()),
                            "notify_channel": watcher.notify_channel,
                            "on_trigger": watcher.on_trigger,
                            "last_error": watcher.last_error,
                            "last_poll_outcome": watcher.last_poll_outcome,
                        })
                    })
                    .collect::<Vec<_>>();
                if rows.is_empty() {
                    return Ok(format!("No {} watcher(s) found.", filter));
                }
                Ok(serde_json::to_string_pretty(&serde_json::json!({
                    "filter": filter,
                    "count": rows.len(),
                    "watchers": rows,
                }))?)
            }
            "tunnel_control" => self.execute_tunnel_control(arguments).await,
            "background_session_manage" => {
                let operation = arguments
                    .get("operation")
                    .and_then(|value| value.as_str())
                    .map(str::trim)
                    .filter(|value| !value.is_empty())
                    .ok_or_else(|| anyhow::anyhow!("Missing background session operation"))?;
                let valid = matches!(
                    operation,
                    "status"
                        | "list"
                        | "pause"
                        | "resume"
                        | "stop"
                        | "cancel"
                        | "delete"
                        | "update_delivery"
                );
                if !valid {
                    anyhow::bail!("Unsupported background session operation `{}`", operation);
                }
                if operation == "update_delivery"
                    && arguments
                        .get("delivery_channel")
                        .and_then(|value| value.as_str())
                        .map(str::trim)
                        .filter(|value| !value.is_empty())
                        .is_none()
                {
                    anyhow::bail!("update_delivery requires delivery_channel");
                }
                Ok(format!(
                    "{}{}",
                    TOOL_COMPLETION_MARKER,
                    serde_json::json!({
                        "tool": "background_session_manage",
                        "status": "completed",
                        "detail": format!("Prepared background session operation: {}", operation),
                    })
                ))
            }
            "schedule_task" => {
                let validate_schedule_item = |item: &serde_json::Value| -> Result<String> {
                    let task_desc = item
                        .get("task")
                        .and_then(|value| value.as_str())
                        .or_else(|| {
                            item.get("task_id")
                                .and_then(|value| value.as_str())
                                .map(|_| "existing task")
                        })
                        .ok_or_else(|| anyhow::anyhow!("Missing task description"))?;

                    let schedule_info =
                        if let Some(cron_expr) = item.get("cron").and_then(|v| v.as_str()) {
                            // Auto-convert standard 5-field cron to 6-field (with seconds)
                            // Standard: "minute hour day month weekday" -> "0 9 * * *"
                            // Rust cron: "second minute hour day month weekday" -> "0 0 9 * * *"
                            let cron_6field = if cron_expr.split_whitespace().count() == 5 {
                                format!("0 {}", cron_expr) // Prepend "0 " for seconds
                            } else {
                                cron_expr.to_string()
                            };

                            // Validate cron expression
                            cron_6field.parse::<cron::Schedule>().map_err(|e| {
                                anyhow::anyhow!("Invalid cron expression '{}': {}", cron_6field, e)
                            })?;
                            format!("cron:{}", cron_6field)
                        } else if let Some(at_time) = item.get("at").and_then(|v| v.as_str()) {
                            // Validate ISO timestamp
                            chrono::DateTime::parse_from_rfc3339(at_time)
                                .map_err(|e| anyhow::anyhow!("Invalid timestamp: {}", e))?;
                            format!("at:{}", at_time)
                        } else {
                            return Err(anyhow::anyhow!(
                                "Must specify either 'cron' or 'at' for scheduling"
                            ));
                        };
                    Ok(format!("Task: {}; schedule: {}", task_desc, schedule_info))
                };

                let detail = if let Some(items) = arguments.get("items") {
                    let items = items
                        .as_array()
                        .filter(|items| !items.is_empty())
                        .ok_or_else(|| {
                            anyhow::anyhow!("schedule_task.items must be a non-empty array")
                        })?;
                    let inheritable_keys = [
                        "task",
                        "report_to",
                        "action",
                        "action_arguments",
                        "script",
                        "script_language",
                        "context_from",
                        "workdir",
                        "network_access",
                        "allow_duplicate",
                        "validation",
                        "max_attempts",
                        "stall_timeout_secs",
                        "retry_backoff_secs",
                        "automation_policy",
                    ];
                    for (index, item) in items.iter().enumerate() {
                        let Some(item_obj) = item.as_object() else {
                            return Err(anyhow::anyhow!(
                                "schedule_task.items[{}] must be an object",
                                index
                            ));
                        };
                        let mut merged = serde_json::Map::new();
                        for key in inheritable_keys {
                            if let Some(value) = arguments.get(key) {
                                merged.insert(key.to_string(), value.clone());
                            }
                        }
                        for (key, value) in item_obj {
                            merged.insert(key.clone(), value.clone());
                        }
                        merged.remove("items");
                        validate_schedule_item(&serde_json::Value::Object(merged)).map_err(
                            |error| {
                                anyhow::anyhow!("Invalid schedule item {}: {}", index + 1, error)
                            },
                        )?;
                    }
                    format!("Prepared {} scheduled task item(s)", items.len())
                } else {
                    validate_schedule_item(arguments)?
                };

                // Return structured scheduling info - actual scheduling is handled by the agent's task queue
                Ok(format!(
                    "{}{}",
                    TOOL_COMPLETION_MARKER,
                    serde_json::json!({
                        "tool": "schedule_task",
                        "status": "completed",
                        "detail": detail,
                    })
                ))
            }
            "watch" => {
                // Return structured watcher info - actual watcher creation is handled by Agent::handle_watch
                let validate_watch_item = |item: &serde_json::Value| -> Result<String> {
                    if item
                        .get("watcher_id")
                        .and_then(|value| value.as_str())
                        .map(str::trim)
                        .is_some_and(|value| !value.is_empty())
                    {
                        return Ok("existing watcher".to_string());
                    }
                    let desc = item
                        .get("description")
                        .and_then(|value| value.as_str())
                        .map(str::trim)
                        .filter(|value| !value.is_empty())
                        .ok_or_else(|| anyhow::anyhow!("Missing watcher description"))?;
                    for key in ["condition", "on_trigger"] {
                        if item.get(key).is_none() {
                            return Err(anyhow::anyhow!("Missing watcher `{}`", key));
                        }
                    }
                    Ok(desc.to_string())
                };
                let desc = if let Some(items) = arguments.get("items") {
                    let items = items
                        .as_array()
                        .filter(|items| !items.is_empty())
                        .ok_or_else(|| anyhow::anyhow!("watch.items must be a non-empty array"))?;
                    let inheritable_keys = [
                        "description",
                        "poll_action",
                        "poll_arguments",
                        "script",
                        "script_language",
                        "context_from",
                        "workdir",
                        "network_access",
                        "condition",
                        "on_trigger",
                        "interval_secs",
                        "timeout_secs",
                        "timeout_hours",
                        "timeout_days",
                        "until_stopped",
                        "notify_channel",
                        "repeat_on_match",
                        "allow_duplicate",
                        "validation",
                        "max_attempts",
                        "stall_timeout_secs",
                        "retry_backoff_secs",
                        "automation_policy",
                    ];
                    for (index, item) in items.iter().enumerate() {
                        let Some(item_obj) = item.as_object() else {
                            return Err(anyhow::anyhow!(
                                "watch.items[{}] must be an object",
                                index
                            ));
                        };
                        let mut merged = serde_json::Map::new();
                        for key in inheritable_keys {
                            if let Some(value) = arguments.get(key) {
                                merged.insert(key.to_string(), value.clone());
                            }
                        }
                        for (key, value) in item_obj {
                            merged.insert(key.clone(), value.clone());
                        }
                        merged.remove("items");
                        validate_watch_item(&serde_json::Value::Object(merged)).map_err(
                            |error| anyhow::anyhow!("Invalid watch item {}: {}", index + 1, error),
                        )?;
                    }
                    format!("Prepared {} watcher item(s)", items.len())
                } else {
                    validate_watch_item(arguments)?
                };
                Ok(format!(
                    "{}{}",
                    TOOL_COMPLETION_MARKER,
                    serde_json::json!({
                        "tool": "watch",
                        "status": "completed",
                        "detail": desc,
                    })
                ))
            }
            "delegate" => {
                let task = arguments
                    .get("task")
                    .and_then(|v| v.as_str())
                    .map(str::trim)
                    .filter(|value| !value.is_empty())
                    .map(ToString::to_string)
                    .or_else(|| {
                        arguments
                            .get("tasks")
                            .and_then(|value| value.as_array())
                            .map(|items| {
                                items
                                    .iter()
                                    .filter_map(|item| item.as_str())
                                    .map(str::trim)
                                    .filter(|item| !item.is_empty())
                                    .enumerate()
                                    .map(|(index, item)| format!("{}. {item}", index + 1))
                                    .collect::<Vec<_>>()
                                    .join("\n")
                            })
                            .filter(|value| !value.trim().is_empty())
                    })
                    .ok_or_else(|| anyhow::anyhow!("Missing delegated task"))?;
                Ok(format!(
                    "{}{}",
                    TOOL_COMPLETION_MARKER,
                    serde_json::json!({
                        "tool": "delegate",
                        "status": "completed",
                        "detail": task,
                    })
                ))
            }
            "manage_actions" => self.execute_manage_actions(arguments).await,
            "ark_inspect" => self.execute_ark_inspect(arguments).await,
            "memory_lookup" => self.execute_memory_lookup(arguments).await,
            "agentark_capability_lookup" => {
                self.execute_agentark_capability_lookup(arguments).await
            }
            "list_integrations" => self.execute_list_integrations(arguments).await,
            "integration_catalog_list" => self.execute_integration_catalog_list(arguments).await,
            "integration_catalog_describe" => {
                self.execute_integration_catalog_describe(arguments).await
            }
            "integration_catalog_status" => {
                self.execute_integration_catalog_status(arguments).await
            }
            "inspect_integration" => self.execute_inspect_integration(arguments).await,
            "mcp_server_manage" => self.execute_mcp_server_manage(arguments).await,
            "postgres_schema_inspect" => self.execute_postgres_schema_inspect(arguments).await,
            "postgres_query_readonly" => self.execute_postgres_query_readonly(arguments).await,
            "capability_acquire" => self.execute_capability_acquire(arguments).await,
            "custom_api_request" => self.execute_custom_api_request(arguments).await,
            "custom_api_manage" => self.execute_custom_api_manage(arguments).await,
            "capability_resolve" => self.execute_capability_resolve(arguments).await,
            "connector_request" => self.execute_connector_request(arguments).await,
            "lan_discover" => crate::actions::lan::lan_discover(arguments).await,
            "extension_pack_list" => self.execute_extension_pack_list(arguments).await,
            "extension_pack_search" => self.execute_extension_pack_search(arguments).await,
            "extension_pack_install" => self.execute_extension_pack_install(arguments).await,
            "extension_pack_scaffold" => self.execute_extension_pack_scaffold(arguments).await,
            "custom_messaging_channel_upsert" => {
                self.execute_custom_messaging_channel_upsert(arguments)
                    .await
            }
            "custom_messaging_channel_manage" => {
                self.execute_custom_messaging_channel_manage(arguments)
                    .await
            }
            "extension_pack_connect" => self.execute_extension_pack_connect(arguments).await,
            "extension_pack_set_enabled" => {
                self.execute_extension_pack_set_enabled(arguments).await
            }
            "extension_pack_delete" => self.execute_extension_pack_delete(arguments).await,
            "extension_pack_runtime_install" => {
                self.execute_extension_pack_runtime_install(arguments).await
            }
            "extension_pack_runtime_verify" => {
                self.execute_extension_pack_runtime_verify(arguments).await
            }
            "extension_pack_runtime_update" => {
                self.execute_extension_pack_runtime_update(arguments).await
            }
            "extension_pack_runtime_uninstall" => {
                self.execute_extension_pack_runtime_uninstall(arguments)
                    .await
            }
            "extension_pack_test_connection" => {
                self.execute_extension_pack_test_connection(arguments).await
            }
            "extension_pack_list_events" => {
                self.execute_extension_pack_list_events(arguments).await
            }
            "extension_pack_invoke" => self.execute_extension_pack_invoke(arguments).await,
            "pipeline_compile" => self.execute_pipeline_compile(arguments).await,
            "pipeline_run" => self.execute_pipeline_run(arguments).await,
            "signal_consensus" => self.execute_signal_consensus(arguments).await,
            "gmail_scan" => crate::actions::gmail::gmail_scan(&self.config_dir, arguments).await,
            "gmail_reply" => crate::actions::gmail::gmail_reply(&self.config_dir, arguments).await,
            "google_drive_search" => {
                crate::actions::google_workspace::drive_search(&self.config_dir, arguments).await
            }
            "google_docs_read" => {
                crate::actions::google_workspace::docs_read(&self.config_dir, arguments).await
            }
            "google_sheets_read" => {
                crate::actions::google_workspace::sheets_read(&self.config_dir, arguments).await
            }
            "google_chat_list_spaces" => {
                crate::actions::google_workspace::chat_list_spaces(&self.config_dir, arguments)
                    .await
            }
            "google_admin_list_users" => {
                crate::actions::google_workspace::admin_list_users(&self.config_dir, arguments)
                    .await
            }
            "google_workspace_gws_help" => {
                crate::actions::google_workspace::gws_help(&self.config_dir, arguments).await
            }
            "google_workspace_gws_schema" => {
                crate::actions::google_workspace::gws_schema(&self.config_dir, arguments).await
            }
            "google_workspace_gws_skills" => {
                crate::actions::google_workspace::gws_skills(&self.config_dir, arguments).await
            }
            "google_workspace_gws_command" => {
                crate::actions::google_workspace::gws_command(&self.config_dir, arguments).await
            }
            "web_search" => {
                let args: crate::actions::search::SearchArgs =
                    serde_json::from_value(arguments.clone())
                        .map_err(|e| anyhow::anyhow!("Invalid search arguments: {}", e))?;

                let config = build_search_config(&self.config_dir, self.storage.as_ref()).await;
                let response =
                    crate::actions::search::execute_search_response(&args, &config).await?;
                let detail = crate::actions::search::format_search_results(&response);
                Ok(structured_tool_completion_output(
                    "web_search",
                    "completed",
                    detail,
                    serde_json::json!({
                        "query": response.query,
                        "backend": response.backend,
                        "results": response.results,
                    }),
                ))
            }
            "research" => {
                let args: crate::actions::research::ResearchArgs =
                    serde_json::from_value(arguments.clone())
                        .map_err(|e| anyhow::anyhow!("Invalid research arguments: {}", e))?;

                let config = build_search_config(&self.config_dir, self.storage.as_ref()).await;
                crate::actions::research::execute_research(&args, &config).await
            }
            "moltbook" => {
                let sub_action = arguments
                    .get("action")
                    .and_then(|value| value.as_str())
                    .map(str::trim)
                    .filter(|value| !value.is_empty())
                    .ok_or_else(|| anyhow::anyhow!("Missing Moltbook action"))?;
                let connector =
                    crate::integrations::moltbook::MoltbookConnector::new_with_config_dir(
                        self.config_dir.clone(),
                    );
                let result =
                    crate::integrations::Integration::execute(&connector, sub_action, arguments)
                        .await?;
                Ok(serde_json::to_string_pretty(&result).unwrap_or_else(|_| result.to_string()))
            }
            "session_search" => self.execute_session_search(arguments).await,
            "document_lookup" => self.execute_document_lookup(arguments).await,
            "vision_ocr" => self.execute_vision_ocr(arguments).await,
            "code_execute" => {
                // Native fallback for code execution (when Docker mode falls through)
                self.execute_code_native(arguments).await
            }
            "browse" => {
                let url = arguments["url"]
                    .as_str()
                    .ok_or_else(|| anyhow::anyhow!("Missing url"))?;
                let parsed_url = reqwest::Url::parse(url).ok();
                let expected_mime = parsed_url
                    .as_ref()
                    .and_then(|url| runtime_url_expected_mime(url));
                let expected_non_text_resource =
                    expected_mime.is_some_and(|mime| !runtime_mime_is_textual(mime));
                let extract = arguments
                    .get("extract")
                    .and_then(|v| v.as_str())
                    .unwrap_or("text");

                // Fetch the page
                let client = reqwest::Client::builder()
                    .user_agent(crate::branding::user_agent_with_suffix(
                        "(AI Agent Browser)",
                    ))
                    .timeout(std::time::Duration::from_secs(30))
                    .redirect(reqwest::redirect::Policy::limited(5))
                    .build()
                    .map_err(|e| anyhow::anyhow!("Failed to build HTTP client: {}", e))?;

                let response = client
                    .get(url)
                    .send()
                    .await
                    .map_err(|e| anyhow::anyhow!("Failed to fetch URL: {}", e))?;

                let status = response.status();
                if !status.is_success() {
                    return Err(crate::actions::structured_action_error(
                        ActionErrorDomain::Search,
                        ActionErrorReason::Failed,
                        format!("Browse request returned HTTP status {}", status.as_u16()),
                    ));
                }

                let content_type = response
                    .headers()
                    .get(reqwest::header::CONTENT_TYPE)
                    .and_then(|value| value.to_str().ok())
                    .unwrap_or("")
                    .to_string();
                let body_bytes = response
                    .bytes()
                    .await
                    .map_err(|e| anyhow::anyhow!("Failed to read response body: {}", e))?;
                if !runtime_response_matches_expected_url_mime(
                    expected_mime,
                    &content_type,
                    body_bytes.as_ref(),
                ) {
                    return Err(crate::actions::structured_action_error(
                        ActionErrorDomain::Search,
                        ActionErrorReason::Failed,
                        runtime_expected_mime_mismatch_message(
                            "Browse",
                            expected_mime,
                            &content_type,
                        ),
                    ));
                }
                if expected_non_text_resource
                    || runtime_response_body_is_probably_binary(&content_type, body_bytes.as_ref())
                {
                    let payload = self
                        .persist_tool_payload_if_needed(
                            ToolPayload::Bytes {
                                mime: Some(content_type.clone())
                                    .filter(|value| !value.trim().is_empty()),
                                body: body_bytes.as_ref().to_vec(),
                                suggested_name: parsed_url
                                    .as_ref()
                                    .and_then(runtime_url_suggested_filename),
                            },
                            PersistHints {
                                mime: Some(content_type.clone())
                                    .filter(|value| !value.trim().is_empty()),
                                source_action: Some("browse".to_string()),
                                force_resource: expected_non_text_resource,
                                ..PersistHints::default()
                            },
                        )
                        .await?;
                    return Ok(Self::render_tool_payload_for_legacy("browse", payload));
                }

                let html = String::from_utf8_lossy(body_bytes.as_ref()).to_string();

                // Extract content based on the extract parameter
                let title_re = regex::Regex::new(r"(?is)<title[^>]*>(.*?)</title>").unwrap();
                let title = title_re
                    .captures(&html)
                    .and_then(|c| c.get(1))
                    .map(|m| m.as_str().trim().to_string())
                    .unwrap_or_default();
                let tag_re = regex::Regex::new(r"<[^>]+>").unwrap();

                let content = match extract {
                    "title" => {
                        if title.is_empty() {
                            "(no title found)".to_string()
                        } else {
                            title.clone()
                        }
                    }
                    "links" => {
                        let link_re = regex::Regex::new(
                            r#"(?is)<a[^>]+href\s*=\s*["']([^"']+)["'][^>]*>(.*?)</a>"#,
                        )
                        .unwrap();
                        let mut links = Vec::new();
                        for cap in link_re.captures_iter(&html) {
                            let href = cap.get(1).map(|m| m.as_str()).unwrap_or("");
                            let text = cap.get(2).map(|m| m.as_str()).unwrap_or("");
                            // Strip HTML tags from link text
                            let clean_text = tag_re.replace_all(text, "").trim().to_string();
                            if !href.is_empty()
                                && !href.starts_with('#')
                                && !href.starts_with("javascript:")
                            {
                                links.push(format!(
                                    "[{}]({})",
                                    if clean_text.is_empty() {
                                        href
                                    } else {
                                        &clean_text
                                    },
                                    href
                                ));
                            }
                        }
                        if links.is_empty() {
                            "(no links found)".to_string()
                        } else {
                            // Limit to 50 links to avoid overwhelming output
                            let display_links: Vec<&str> =
                                links.iter().take(50).map(|s| s.as_str()).collect();
                            format!(
                                "Found {} links (showing up to 50):\n{}",
                                links.len(),
                                display_links.join("\n")
                            )
                        }
                    }
                    "all" => {
                        // Extract text
                        let text = Self::html_to_text(&html);
                        // Extract links
                        let link_re = regex::Regex::new(
                            r#"(?is)<a[^>]+href\s*=\s*["']([^"']+)["'][^>]*>(.*?)</a>"#,
                        )
                        .unwrap();
                        let mut links = Vec::new();
                        for cap in link_re.captures_iter(&html) {
                            let href = cap.get(1).map(|m| m.as_str()).unwrap_or("");
                            let link_text = cap.get(2).map(|m| m.as_str()).unwrap_or("");
                            let clean_text = tag_re.replace_all(link_text, "").trim().to_string();
                            if !href.is_empty()
                                && !href.starts_with('#')
                                && !href.starts_with("javascript:")
                            {
                                links.push(format!(
                                    "[{}]({})",
                                    if clean_text.is_empty() {
                                        href
                                    } else {
                                        &clean_text
                                    },
                                    href
                                ));
                            }
                        }
                        let links_section = if links.is_empty() {
                            "(no links found)".to_string()
                        } else {
                            let display_links: Vec<&str> =
                                links.iter().take(30).map(|s| s.as_str()).collect();
                            format!(
                                "{} links (showing up to 30):\n{}",
                                links.len(),
                                display_links.join("\n")
                            )
                        };
                        format!(
                            "## Title\n{}\n\n## Content\n{}\n\n## Links\n{}",
                            if title.is_empty() {
                                "(no title)"
                            } else {
                                &title
                            },
                            text,
                            links_section
                        )
                    }
                    _ => {
                        // Default: extract text content
                        let text = Self::html_to_text(&html);
                        if text.is_empty() {
                            "(no text content extracted)".to_string()
                        } else {
                            text
                        }
                    }
                };
                Ok(structured_tool_completion_output(
                    "browse",
                    "completed",
                    browse_completion_detail(url, &title, extract, &content),
                    serde_json::json!({
                        "url": url,
                        "title": title,
                        "extract": extract,
                        "content": content,
                    }),
                ))
            }
            "pdf_generate" => {
                let content = arguments["content"]
                    .as_str()
                    .ok_or_else(|| anyhow::anyhow!("Missing content"))?;
                let title = arguments
                    .get("title")
                    .and_then(|v| v.as_str())
                    .unwrap_or("Document");
                let filename = arguments
                    .get("filename")
                    .and_then(|v| v.as_str())
                    .unwrap_or("output.pdf");
                let style = arguments
                    .get("style")
                    .and_then(|v| v.as_str())
                    .unwrap_or("plain");

                let filename = Self::pdf_generate_filename(filename);
                let pdf_bytes = Self::generate_simple_pdf_bytes(title, content, style);
                let exec_id = uuid::Uuid::new_v4().to_string();
                let output_path = self
                    .data_dir()
                    .join("outputs")
                    .join(exec_id)
                    .join(&filename);
                if let Some(parent) = output_path.parent() {
                    tokio::fs::create_dir_all(parent).await?;
                }

                tokio::fs::write(&output_path, &pdf_bytes).await?;
                Self::set_private_file_permissions(&output_path).await?;
                let payload = FileWritePayload {
                    bytes: pdf_bytes,
                    mime: Some("application/pdf".to_string()),
                    source_resource: None,
                };
                let mut document_args = arguments.clone();
                if let Some(object) = document_args.as_object_mut() {
                    object.insert(
                        "document_visible".to_string(),
                        serde_json::Value::Bool(true),
                    );
                }
                let document = self
                    .index_file_write_document_if_requested(
                        &output_path,
                        &document_args,
                        &payload,
                        Some("application/pdf"),
                    )
                    .await;
                Ok(self.managed_file_completion_output(
                    "pdf_generate",
                    &output_path,
                    &payload,
                    document.as_ref(),
                ))
            }
            "expense" => {
                let action = arguments["action"]
                    .as_str()
                    .ok_or_else(|| anyhow::anyhow!("Missing action parameter"))?;
                let storage = self
                    .storage
                    .as_ref()
                    .ok_or_else(|| anyhow::anyhow!("Storage not available"))?;

                match action {
                    "add" => {
                        let amount = arguments
                            .get("amount")
                            .and_then(|v| v.as_f64())
                            .ok_or_else(|| anyhow::anyhow!("Missing amount"))?;
                        let description = arguments
                            .get("description")
                            .and_then(|v| v.as_str())
                            .ok_or_else(|| anyhow::anyhow!("Missing description"))?;
                        let currency = arguments
                            .get("currency")
                            .and_then(|v| v.as_str())
                            .unwrap_or("USD");
                        let category = arguments
                            .get("category")
                            .and_then(|v| v.as_str())
                            .unwrap_or("other");
                        let date = arguments
                            .get("date")
                            .and_then(|v| v.as_str())
                            .map(|s| s.to_string())
                            .unwrap_or_else(|| chrono::Utc::now().format("%Y-%m-%d").to_string());

                        let id = format!(
                            "exp-{}",
                            uuid::Uuid::new_v4()
                                .to_string()
                                .split('-')
                                .next()
                                .unwrap_or("0")
                        );
                        let model = crate::storage::entities::expense::Model {
                            id: id.clone(),
                            amount,
                            currency: currency.to_string(),
                            category: category.to_string(),
                            description: description.to_string(),
                            date,
                            payment_method: arguments
                                .get("payment_method")
                                .and_then(|v| v.as_str())
                                .map(|s| s.to_string()),
                            vendor: arguments
                                .get("vendor")
                                .and_then(|v| v.as_str())
                                .map(|s| s.to_string()),
                            tags: arguments
                                .get("tags")
                                .and_then(|v| v.as_str())
                                .map(|s| s.to_string()),
                            split_with: None,
                            receipt_path: None,
                            created_at: chrono::Utc::now().to_rfc3339(),
                        };
                        storage.insert_expense(model).await?;
                        Ok(format!(
                            "Expense recorded: {} {} for '{}' (category: {}, id: {})",
                            currency, amount, description, category, id
                        ))
                    }
                    "list" => {
                        let from = arguments.get("from_date").and_then(|v| v.as_str());
                        let to = arguments.get("to_date").and_then(|v| v.as_str());
                        let cat = arguments.get("filter_category").and_then(|v| v.as_str());
                        let expenses = storage.get_expenses(from, to, cat).await?;
                        if expenses.is_empty() {
                            return Ok("No expenses found.".to_string());
                        }
                        let mut output = format!("Found {} expense(s):\n\n", expenses.len());
                        let mut total = 0.0f64;
                        for e in &expenses {
                            output.push_str(&format!(
                                "- [{}] {} {} - {} ({}){}\n",
                                e.id,
                                e.currency,
                                e.amount,
                                e.description,
                                e.category,
                                e.vendor
                                    .as_ref()
                                    .map(|v| format!(" @ {}", v))
                                    .unwrap_or_default()
                            ));
                            total += e.amount;
                        }
                        output.push_str(&format!("\nTotal: {:.2}", total));
                        Ok(output)
                    }
                    "summary" => {
                        let from = arguments.get("from_date").and_then(|v| v.as_str());
                        let to = arguments.get("to_date").and_then(|v| v.as_str());
                        let expenses = storage.get_expense_summary(from, to).await?;
                        if expenses.is_empty() {
                            return Ok("No expenses found for the period.".to_string());
                        }
                        // Aggregate by category
                        let mut by_category: std::collections::HashMap<String, f64> =
                            std::collections::HashMap::new();
                        for e in &expenses {
                            *by_category.entry(e.category.clone()).or_insert(0.0) += e.amount;
                        }
                        let mut output = "Expense Summary by Category:\n\n".to_string();
                        let mut grand_total = 0.0f64;
                        let mut cats: Vec<_> = by_category.into_iter().collect();
                        cats.sort_by(|a, b| {
                            b.1.partial_cmp(&a.1).unwrap_or(std::cmp::Ordering::Equal)
                        });
                        for (category, total) in &cats {
                            output.push_str(&format!("  {}: {:.2}\n", category, total));
                            grand_total += total;
                        }
                        output.push_str(&format!("\nGrand Total: {:.2}", grand_total));
                        Ok(output)
                    }
                    "delete" => {
                        let id = arguments
                            .get("id")
                            .and_then(|v| v.as_str())
                            .ok_or_else(|| anyhow::anyhow!("Missing expense ID"))?;
                        let deleted = storage.delete_expense(id).await?;
                        if deleted {
                            Ok(format!("Expense {} deleted.", id))
                        } else {
                            Ok(format!("Expense {} not found.", id))
                        }
                    }
                    _ => Err(anyhow::anyhow!("Unknown expense action: {}", action)),
                }
            }
            "security_logs" => {
                let storage = self
                    .storage
                    .as_ref()
                    .ok_or_else(|| anyhow::anyhow!("Storage not available"))?;
                let limit = arguments
                    .get("limit")
                    .and_then(|v| v.as_u64())
                    .unwrap_or(50);
                let logs = storage.list_security_logs(limit).await?;
                if logs.is_empty() {
                    return Ok("No security events recorded. All clear.".to_string());
                }
                let mut output = format!("Security Log ({} entries):\n\n", logs.len());
                for log in &logs {
                    output.push_str(&format!(
                        "- [{}] {} ({}): {} (count: {})\n",
                        log.created_at.split('T').next().unwrap_or(&log.created_at),
                        log.event_type,
                        log.severity,
                        log.message,
                        log.count,
                    ));
                }
                Ok(output)
            }
            "home_assistant" => self.execute_home_assistant(arguments).await,
            "home_assistant_call_service" => {
                self.execute_home_assistant_call_service(arguments).await
            }
            "transcribe_audio" => {
                let file_path = arguments["file_path"]
                    .as_str()
                    .ok_or_else(|| anyhow::anyhow!("Missing file_path"))?;
                let language = arguments
                    .get("language")
                    .and_then(|v| v.as_str())
                    .unwrap_or("auto");
                let model = arguments
                    .get("model")
                    .and_then(|v| v.as_str())
                    .unwrap_or("base");

                let escaped_path = file_path.replace('\\', "/");
                let lang_arg = if language == "auto" {
                    "None".to_string()
                } else {
                    format!("\"{}\"", language)
                };

                let python_code = format!(
                    r#"
import subprocess, sys
subprocess.check_call([sys.executable, "-m", "pip", "install", "-q", "openai-whisper"], stdout=subprocess.DEVNULL, stderr=subprocess.DEVNULL)
import whisper

model = whisper.load_model("{model}")
result = model.transcribe("{escaped_path}", language={lang_arg})
print(result["text"])
"#
                );
                let code_args = serde_json::json!({
                    "language": "python",
                    "code": python_code.trim()
                });
                self.execute_code_native(&code_args).await
            }
            "weekly_review" => {
                let period = arguments
                    .get("period_days")
                    .and_then(|v| v.as_i64())
                    .unwrap_or(7);
                let queue = self
                    .task_queue
                    .as_ref()
                    .ok_or_else(|| anyhow::anyhow!("Task queue not available"))?;
                let tasks = queue.read().await;

                let mut output = format!("Weekly Review (last {} days):\n\n", period);

                // Completed tasks
                let completed: Vec<_> = tasks
                    .all()
                    .iter()
                    .filter(|t| matches!(t.status, crate::core::TaskStatus::Completed))
                    .collect();
                output.push_str(&format!("**Completed Tasks** ({})\n", completed.len()));
                for t in &completed {
                    output.push_str(&format!("  - {}\n", t.description));
                }

                // Pending tasks
                let pending: Vec<_> = tasks
                    .all()
                    .iter()
                    .filter(|t| {
                        matches!(
                            t.status,
                            crate::core::TaskStatus::Pending
                                | crate::core::TaskStatus::AwaitingApproval
                        )
                    })
                    .collect();
                output.push_str(&format!("\n**Pending Tasks** ({})\n", pending.len()));
                for t in &pending {
                    output.push_str(&format!("  - {}\n", t.description));
                }

                let paused: Vec<_> = tasks
                    .all()
                    .iter()
                    .filter(|t| matches!(t.status, crate::core::TaskStatus::Paused))
                    .collect();
                if !paused.is_empty() {
                    output.push_str(&format!("\n**Paused Tasks** ({})\n", paused.len()));
                    for t in &paused {
                        output.push_str(&format!("  - {}\n", t.description));
                    }
                }

                let paused: Vec<_> = tasks
                    .all()
                    .iter()
                    .filter(|t| matches!(t.status, crate::core::TaskStatus::Paused))
                    .collect();
                if !paused.is_empty() {
                    output.push_str(&format!("\n**Paused Tasks** ({})\n", paused.len()));
                    for t in &paused {
                        output.push_str(&format!("  - {}\n", t.description));
                    }
                }

                // Failed tasks
                let failed: Vec<_> = tasks
                    .all()
                    .iter()
                    .filter(|t| matches!(t.status, crate::core::TaskStatus::Failed { .. }))
                    .collect();
                if !failed.is_empty() {
                    output.push_str(&format!("\n**Failed Tasks** ({})\n", failed.len()));
                    for t in &failed {
                        output.push_str(&format!("  - {}\n", t.description));
                    }
                }

                // Expense summary if storage available
                if let Some(ref storage) = self.storage {
                    let from_date = (chrono::Utc::now() - chrono::Duration::days(period))
                        .format("%Y-%m-%d")
                        .to_string();
                    if let Ok(expenses) = storage.get_expense_summary(Some(&from_date), None).await
                    {
                        if !expenses.is_empty() {
                            let mut by_cat: std::collections::HashMap<String, f64> =
                                std::collections::HashMap::new();
                            for e in &expenses {
                                *by_cat.entry(e.category.clone()).or_insert(0.0) += e.amount;
                            }
                            output.push_str("\n**Spending Summary**\n");
                            let mut total = 0.0;
                            for (cat, amt) in &by_cat {
                                output.push_str(&format!("  {}: {:.2}\n", cat, amt));
                                total += amt;
                            }
                            output.push_str(&format!("  Total: {:.2}\n", total));
                        }
                    }
                }

                Ok(output)
            }
            "current_time" => {
                let timezone_name = arguments
                    .get("timezone")
                    .and_then(|value| value.as_str())
                    .map(str::trim)
                    .filter(|value| !value.is_empty());
                let now_utc = chrono::Utc::now();
                if let Some(timezone_name) = timezone_name {
                    let timezone = timezone_name.parse::<chrono_tz::Tz>().map_err(|_| {
                        anyhow::anyhow!(
                            "Invalid timezone '{}'. Expected an IANA name such as Asia/Kolkata.",
                            timezone_name
                        )
                    })?;
                    let local = now_utc.with_timezone(&timezone);
                    Ok(format!(
                        "Timezone: {}\nISO: {}\nDate: {}\nReadable date: {}\nTime: {}\nWeekday: {}\nUnix: {}",
                        timezone_name,
                        local.to_rfc3339(),
                        local.format("%Y-%m-%d"),
                        local.format("%B %d, %Y"),
                        local.format("%H:%M:%S %Z"),
                        local.format("%A"),
                        now_utc.timestamp()
                    ))
                } else {
                    Ok(format!(
                        "Timezone: UTC\nISO: {}\nDate: {}\nReadable date: {}\nTime: {}\nWeekday: {}\nUnix: {}",
                        now_utc.to_rfc3339(),
                        now_utc.format("%Y-%m-%d"),
                        now_utc.format("%B %d, %Y"),
                        now_utc.format("%H:%M:%S UTC"),
                        now_utc.format("%A"),
                        now_utc.timestamp()
                    ))
                }
            }
            "notify_user" => {
                let message = arguments
                    .get("message")
                    .and_then(|value| value.as_str())
                    .map(str::trim)
                    .filter(|value| !value.is_empty())
                    .ok_or_else(|| {
                        crate::actions::structured_action_error(
                            ActionErrorDomain::Channel,
                            ActionErrorReason::MissingInput,
                            "notify_user requires a non-empty `message`",
                        )
                    })?;
                let title = arguments
                    .get("title")
                    .and_then(|value| value.as_str())
                    .map(str::trim)
                    .filter(|value| !value.is_empty());
                if let Some(title) = title {
                    Ok(format!("{}\n\n{}", title, message))
                } else {
                    Ok(message.to_string())
                }
            }
            // ArkOrbit (per-user limitless canvas)
            "arkorbit_create_orbit" => {
                let service = self.arkorbit_service()?;
                let user_id = self.current_user_id()?.to_string();
                crate::actions::arkorbit::create_orbit(&service, &user_id, arguments).await
            }
            "arkorbit_file_write" => {
                let service = self.arkorbit_service()?;
                crate::actions::arkorbit::orbit_file_write(&service, arguments).await
            }
            // Google Calendar actions
            "calendar_today" => {
                crate::actions::calendar::calendar_today(&self.config_dir, arguments).await
            }
            "calendar_list" => {
                crate::actions::calendar::calendar_list(&self.config_dir, arguments).await
            }
            "calendar_create" => {
                crate::actions::calendar::calendar_create(&self.config_dir, arguments).await
            }
            "calendar_free" => {
                crate::actions::calendar::calendar_free(&self.config_dir, arguments).await
            }
            // SSH remote execution
            #[cfg(feature = "ssh")]
            "ssh" => crate::actions::ssh::ssh_execute(&self.config_dir, arguments).await,
            #[cfg(feature = "ssh")]
            "ssh_connections" => crate::actions::ssh::ssh_list_connections(&self.config_dir).await,
            // Handle workflow actions - return marker for agent to process with LLM
            other => {
                let actions = self.actions.read().await;
                if let Some(action) = actions.get(other) {
                    if action.workflow_content.is_some() {
                        // Return a special marker that tells the agent to use LLM-driven execution.
                        let user_query = Self::build_workflow_user_query(arguments);
                        let has_freeform_query = arguments
                            .get("query")
                            .and_then(|v| v.as_str())
                            .is_some_and(|s| !s.trim().is_empty());
                        if !has_freeform_query {
                            let required = Self::collect_required_fields_from_schema(
                                &action.info.input_schema,
                            );
                            let sensitive_required =
                                Self::collect_sensitive_required_fields_from_schema(
                                    &action.info.input_schema,
                                );
                            let missing: Vec<String> = required
                                .iter()
                                .filter(|k| !Self::has_non_empty_argument(arguments, k))
                                .cloned()
                                .collect();
                            if !missing.is_empty() {
                                let sensitive_missing = missing
                                    .iter()
                                    .filter(|key| {
                                        sensitive_required.iter().any(|required| required == *key)
                                    })
                                    .cloned()
                                    .collect();
                                let payload = WorkflowMissingInputsPayload {
                                    action: other.to_string(),
                                    missing,
                                    sensitive_missing,
                                    required,
                                    provided: Self::collect_provided_argument_keys(arguments),
                                    query: user_query,
                                };
                                return Ok(Self::build_workflow_missing_inputs_marker(&payload));
                            }
                        }
                        return Ok(format!(
                            "{}{}:{}",
                            WORKFLOW_ACTION_MARKER, other, user_query
                        ));
                    }
                }
                Err(crate::actions::structured_action_error(
                    ActionErrorDomain::Action,
                    ActionErrorReason::NotFound,
                    format!("Unknown native action: {}", action_name),
                ))
            }
        }
    }
}
