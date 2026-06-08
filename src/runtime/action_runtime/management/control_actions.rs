use super::super::*;

impl ActionRuntime {
    pub(in crate::runtime) async fn execute_curator_control(
        &self,
        arguments: &serde_json::Value,
    ) -> Result<String> {
        let operation = arguments
            .get("operation")
            .and_then(|value| value.as_str())
            .map(str::trim)
            .filter(|value| !value.is_empty())
            .unwrap_or("status");
        let skills_dir = self.data_dir().join("skills");
        let usage_path = skills_dir.join(".usage.json");
        let review_dir = skills_dir.join(".review_queue");
        let pause_path = skills_dir.join(".curator_paused");
        tokio::fs::create_dir_all(&skills_dir).await?;
        match operation {
            "pause" => {
                tokio::fs::write(&pause_path, chrono::Utc::now().to_rfc3339()).await?;
            }
            "resume" => {
                if tokio::fs::metadata(&pause_path).await.is_ok() {
                    let _ = tokio::fs::remove_file(&pause_path).await;
                }
            }
            "status" => {}
            other => anyhow::bail!("Unsupported curator operation `{}`", other),
        }
        let usage_exists = tokio::fs::metadata(&usage_path).await.is_ok();
        let paused = tokio::fs::metadata(&pause_path).await.is_ok();
        let mut draft_count = 0usize;
        if tokio::fs::metadata(&review_dir).await.is_ok() {
            let mut entries = tokio::fs::read_dir(&review_dir).await?;
            while let Some(entry) = entries.next_entry().await? {
                if entry.file_type().await?.is_file() {
                    draft_count += 1;
                }
            }
        }
        Ok(serde_json::json!({
            "status": "ok",
            "operation": operation,
            "paused": paused,
            "usage_path": usage_path.display().to_string(),
            "usage_exists": usage_exists,
            "review_queue_path": review_dir.display().to_string(),
            "draft_count": draft_count,
        })
        .to_string())
    }

    pub(in crate::runtime) async fn execute_skill_view(
        &self,
        arguments: &serde_json::Value,
    ) -> Result<String> {
        let operation = arguments
            .get("operation")
            .and_then(|value| value.as_str())
            .unwrap_or("list");
        let skills_dir = self.data_dir().join("skills");
        tokio::fs::create_dir_all(&skills_dir).await?;
        match operation {
            "list" => {
                let mut rows = Vec::new();
                let mut entries = tokio::fs::read_dir(&skills_dir).await?;
                while let Some(entry) = entries.next_entry().await? {
                    if !entry.file_type().await?.is_dir() {
                        continue;
                    }
                    let name = entry.file_name().to_string_lossy().to_string();
                    if name.starts_with('.') {
                        continue;
                    }
                    let skill_path = entry.path().join("SKILL.md");
                    if tokio::fs::metadata(&skill_path).await.is_ok() {
                        rows.push(serde_json::json!({
                            "name": name,
                            "path": skill_path.display().to_string(),
                        }));
                    }
                }
                rows.sort_by_key(|row| {
                    row.get("name")
                        .and_then(|value| value.as_str())
                        .unwrap_or_default()
                        .to_string()
                });
                Ok(serde_json::json!({
                    "status": "ok",
                    "skills": rows,
                })
                .to_string())
            }
            "read" => {
                let name = required_skill_name(arguments)?;
                let skill_path = skills_dir.join(&name).join("SKILL.md");
                let markdown = tokio::fs::read_to_string(&skill_path)
                    .await
                    .with_context(|| {
                        format!("Skill '{}' was not found at {}", name, skill_path.display())
                    })?;
                self.increment_skill_usage_counter(&name, "view_count")
                    .await?;
                Ok(serde_json::json!({
                    "status": "ok",
                    "name": name,
                    "path": skill_path.display().to_string(),
                    "markdown": markdown,
                })
                .to_string())
            }
            other => anyhow::bail!("Unsupported skill_view operation `{}`", other),
        }
    }

    pub(in crate::runtime) async fn execute_skill_manage(
        &self,
        arguments: &serde_json::Value,
    ) -> Result<String> {
        let operation = arguments
            .get("operation")
            .and_then(|value| value.as_str())
            .map(str::trim)
            .filter(|value| !value.is_empty())
            .unwrap_or("create");
        let name = required_skill_name(arguments)?;
        let skills_dir = self.data_dir().join("skills");
        let archive_dir = skills_dir.join(".archive");
        tokio::fs::create_dir_all(&skills_dir).await?;
        tokio::fs::create_dir_all(&archive_dir).await?;
        let skill_dir = skills_dir.join(&name);
        let skill_path = skill_dir.join("SKILL.md");
        match operation {
            "create" | "update" => {
                let markdown = arguments
                    .get("markdown")
                    .and_then(|value| value.as_str())
                    .map(str::trim)
                    .filter(|value| !value.is_empty())
                    .ok_or_else(|| {
                        anyhow::anyhow!("skill_manage {} requires markdown", operation)
                    })?;
                tokio::fs::create_dir_all(&skill_dir).await?;
                tokio::fs::write(&skill_path, markdown).await?;
                self.update_skill_usage_record(&name, Some("agent"), None)
                    .await?;
                self.increment_skill_usage_counter(&name, "patch_count")
                    .await?;
            }
            "pin" => {
                self.update_skill_usage_record(&name, None, Some(true))
                    .await?;
            }
            "unpin" => {
                self.update_skill_usage_record(&name, None, Some(false))
                    .await?;
            }
            "archive" => {
                if tokio::fs::metadata(&skill_path).await.is_err() {
                    anyhow::bail!("Skill '{}' does not exist", name);
                }
                let target = archive_dir.join(format!(
                    "{}-{}",
                    name,
                    chrono::Utc::now().format("%Y%m%d%H%M%S")
                ));
                tokio::fs::rename(&skill_dir, &target).await?;
                self.update_skill_usage_state(&name, "archived").await?;
            }
            "restore" => {
                let mut entries = tokio::fs::read_dir(&archive_dir).await?;
                let mut restored = false;
                while let Some(entry) = entries.next_entry().await? {
                    let archived_name = entry.file_name().to_string_lossy().to_string();
                    if archived_name == name || archived_name.starts_with(&format!("{}-", name)) {
                        if tokio::fs::metadata(&skill_dir).await.is_err() {
                            tokio::fs::rename(entry.path(), &skill_dir).await?;
                            self.update_skill_usage_state(&name, "active").await?;
                            restored = true;
                        }
                        break;
                    }
                }
                if !restored {
                    anyhow::bail!("No archived copy found for skill '{}'", name);
                }
            }
            other => anyhow::bail!("Unsupported skill_manage operation `{}`", other),
        }
        Ok(serde_json::json!({
            "status": "ok",
            "operation": operation,
            "name": name,
            "path": skill_path.display().to_string(),
        })
        .to_string())
    }

    pub(in crate::runtime) async fn execute_goal_manage(
        &self,
        arguments: &serde_json::Value,
    ) -> Result<String> {
        let operation = arguments
            .get("operation")
            .and_then(|value| value.as_str())
            .map(str::trim)
            .filter(|value| !value.is_empty())
            .unwrap_or("list");
        let Some(task_queue) = self.task_queue.as_ref() else {
            anyhow::bail!("goal_manage requires the shared task queue");
        };

        match operation {
            "create" => {
                let goal = arguments
                    .get("goal")
                    .and_then(|value| value.as_str())
                    .map(str::trim)
                    .filter(|value| !value.is_empty())
                    .ok_or_else(|| anyhow::anyhow!("goal_manage create requires goal"))?;
                let allow_duplicate = arguments
                    .get("allow_duplicate")
                    .and_then(|value| value.as_bool())
                    .unwrap_or(false);
                if !allow_duplicate {
                    let existing = {
                        let tasks = task_queue.read().await;
                        tasks
                            .all()
                            .iter()
                            .filter(|task| task.action == "goal")
                            .find(|task| {
                                task.arguments
                                    .get("goal")
                                    .and_then(|value| value.as_str())
                                    .map(str::trim)
                                    == Some(goal)
                            })
                            .cloned()
                    };
                    if let Some(existing) = existing {
                        return Ok(serde_json::json!({
                            "status": "ok",
                            "operation": "create",
                            "reused": true,
                            "goal_id": existing.id.to_string(),
                            "goal": goal,
                        })
                        .to_string());
                    }
                }

                let due_date = arguments
                    .get("due_date")
                    .and_then(|value| value.as_str())
                    .map(str::trim)
                    .filter(|value| !value.is_empty())
                    .map(str::to_string);
                let goal_id = arguments
                    .get("goal_id")
                    .and_then(|value| value.as_str())
                    .map(str::trim)
                    .filter(|value| !value.is_empty())
                    .map(str::to_string)
                    .unwrap_or_else(|| uuid::Uuid::new_v4().to_string());
                let task = crate::core::Task {
                    id: uuid::Uuid::new_v4(),
                    description: goal.to_string(),
                    action: "goal".to_string(),
                    arguments: serde_json::json!({
                        "goal": goal,
                        "goal_id": goal_id,
                        "due_date": due_date,
                        "source": "goal_manage_compat",
                    }),
                    approval: crate::core::TaskApproval::Auto,
                    capabilities: vec!["goal_management".to_string()],
                    status: crate::core::TaskStatus::Pending,
                    created_at: chrono::Utc::now(),
                    scheduled_for: None,
                    cron: None,
                    result: None,
                    proof_id: None,
                    priority: None,
                    urgency: None,
                    importance: None,
                    eisenhower_quadrant: None,
                };
                let task_id = task.id.to_string();
                if let Some(storage) = self.storage.as_ref() {
                    storage.insert_task(&task).await?;
                }
                task_queue.write().await.add(task);
                Ok(serde_json::json!({
                    "status": "ok",
                    "operation": "create",
                    "task_id": task_id,
                    "goal_id": goal_id,
                    "goal": goal,
                    "due_date": due_date,
                })
                .to_string())
            }
            "list" | "report" => {
                let limit = arguments
                    .get("limit")
                    .and_then(|value| value.as_u64())
                    .unwrap_or(10)
                    .clamp(1, 50) as usize;
                let rows = {
                    let tasks = task_queue.read().await;
                    tasks
                        .all()
                        .iter()
                        .filter(|task| task.action == "goal")
                        .take(limit)
                        .map(|task| {
                            serde_json::json!({
                                "task_id": task.id.to_string(),
                                "goal_id": task.arguments.get("goal_id").and_then(|value| value.as_str()),
                                "goal": task.arguments.get("goal").and_then(|value| value.as_str()).unwrap_or(task.description.as_str()),
                                "due_date": task.arguments.get("due_date").cloned().unwrap_or(serde_json::Value::Null),
                                "status": format!("{:?}", task.status),
                                "created_at": task.created_at.to_rfc3339(),
                            })
                        })
                        .collect::<Vec<_>>()
                };
                Ok(serde_json::json!({
                    "status": "ok",
                    "operation": operation,
                    "goals": rows,
                })
                .to_string())
            }
            "update" => {
                let goal_id = arguments
                    .get("goal_id")
                    .and_then(|value| value.as_str())
                    .map(str::trim)
                    .filter(|value| !value.is_empty());
                let goal = arguments
                    .get("goal")
                    .and_then(|value| value.as_str())
                    .map(str::trim)
                    .filter(|value| !value.is_empty());
                if goal_id.is_none() && goal.is_none() {
                    anyhow::bail!("goal_manage update requires goal_id or goal");
                }
                let new_goal = arguments
                    .get("new_goal")
                    .or_else(|| arguments.get("title"))
                    .and_then(|value| value.as_str())
                    .map(str::trim)
                    .filter(|value| !value.is_empty())
                    .map(str::to_string);
                let due_date = arguments
                    .get("due_date")
                    .and_then(|value| value.as_str())
                    .map(str::trim)
                    .filter(|value| !value.is_empty())
                    .map(str::to_string);
                if new_goal.is_none() && due_date.is_none() {
                    anyhow::bail!("goal_manage update requires new_goal/title or due_date");
                }
                let target_id = {
                    let tasks = task_queue.read().await;
                    tasks
                        .all()
                        .iter()
                        .find(|task| {
                            if task.action != "goal" {
                                return false;
                            }
                            let matches_id = goal_id.is_some_and(|id| {
                                task.id.to_string() == id
                                    || task
                                        .arguments
                                        .get("goal_id")
                                        .and_then(|value| value.as_str())
                                        .map(str::trim)
                                        == Some(id)
                            });
                            let matches_goal = goal.is_some_and(|target| {
                                task.arguments
                                    .get("goal")
                                    .and_then(|value| value.as_str())
                                    .map(str::trim)
                                    == Some(target)
                            });
                            matches_id || matches_goal
                        })
                        .map(|task| task.id)
                };
                let Some(target_id) = target_id else {
                    return Ok(serde_json::json!({
                        "status": "not_found",
                        "operation": "update",
                        "updated": false,
                    })
                    .to_string());
                };
                let updated = {
                    let mut tasks = task_queue.write().await;
                    let Some(task) = tasks.get_mut(target_id) else {
                        return Ok(serde_json::json!({
                            "status": "not_found",
                            "operation": "update",
                            "updated": false,
                        })
                        .to_string());
                    };
                    let mut args = task.arguments.clone();
                    if !args.is_object() {
                        args = serde_json::json!({});
                    }
                    if let Some(object) = args.as_object_mut() {
                        if let Some(new_goal) = new_goal.as_deref() {
                            object.insert(
                                "goal".to_string(),
                                serde_json::Value::String(new_goal.to_string()),
                            );
                            task.description = new_goal.to_string();
                        }
                        if let Some(due_date) = due_date.as_deref() {
                            object.insert(
                                "due_date".to_string(),
                                serde_json::Value::String(due_date.to_string()),
                            );
                        }
                    }
                    task.arguments = args;
                    task.clone()
                };
                if let Some(storage) = self.storage.as_ref() {
                    storage
                        .update_task(
                            &updated.id.to_string(),
                            Some(updated.description.clone()),
                            Some(serde_json::to_string(&updated.arguments)?),
                            None,
                            None,
                        )
                        .await?;
                }
                Ok(serde_json::json!({
                    "status": "ok",
                    "operation": "update",
                    "updated": true,
                    "task_id": updated.id.to_string(),
                    "goal_id": updated.arguments.get("goal_id").and_then(|value| value.as_str()),
                    "goal": updated.arguments.get("goal").and_then(|value| value.as_str()).unwrap_or(updated.description.as_str()),
                    "due_date": updated.arguments.get("due_date").cloned().unwrap_or(serde_json::Value::Null),
                })
                .to_string())
            }
            "delete" => {
                let goal_id = arguments
                    .get("goal_id")
                    .and_then(|value| value.as_str())
                    .map(str::trim)
                    .filter(|value| !value.is_empty());
                let goal = arguments
                    .get("goal")
                    .and_then(|value| value.as_str())
                    .map(str::trim)
                    .filter(|value| !value.is_empty());
                if goal_id.is_none() && goal.is_none() {
                    anyhow::bail!("goal_manage delete requires goal_id or goal");
                }
                let matching_ids = {
                    let tasks = task_queue.read().await;
                    tasks
                        .all()
                        .iter()
                        .filter(|task| task.action == "goal")
                        .filter(|task| {
                            let matches_id = goal_id.is_some_and(|id| {
                                task.id.to_string() == id
                                    || task
                                        .arguments
                                        .get("goal_id")
                                        .and_then(|value| value.as_str())
                                        .map(str::trim)
                                        == Some(id)
                            });
                            let matches_goal = goal.is_some_and(|target| {
                                task.arguments
                                    .get("goal")
                                    .and_then(|value| value.as_str())
                                    .map(str::trim)
                                    == Some(target)
                            });
                            matches_id || matches_goal
                        })
                        .map(|task| task.id)
                        .collect::<Vec<_>>()
                };
                if matching_ids.is_empty() {
                    return Ok(serde_json::json!({
                        "status": "ok",
                        "operation": "delete",
                        "deleted": 0,
                    })
                    .to_string());
                }
                {
                    let mut tasks = task_queue.write().await;
                    for id in &matching_ids {
                        tasks.remove(*id);
                    }
                }
                if let Some(storage) = self.storage.as_ref() {
                    for id in &matching_ids {
                        storage.delete_task(&id.to_string()).await?;
                    }
                }
                Ok(serde_json::json!({
                    "status": "ok",
                    "operation": "delete",
                    "deleted": matching_ids.len(),
                    "task_ids": matching_ids.iter().map(|id| id.to_string()).collect::<Vec<_>>(),
                })
                .to_string())
            }
            other => anyhow::bail!("Unsupported goal_manage operation `{}`", other),
        }
    }

    pub(in crate::runtime) async fn update_skill_usage_record(
        &self,
        name: &str,
        created_by: Option<&str>,
        pinned: Option<bool>,
    ) -> Result<()> {
        let usage_path = self.data_dir().join("skills").join(".usage.json");
        let mut usage = read_usage_json(&usage_path).await;
        let mut record = usage
            .get(name)
            .and_then(|value| value.as_object().cloned())
            .unwrap_or_default();
        if let Some(created_by) = created_by {
            record.insert(
                "created_by".to_string(),
                serde_json::Value::String(created_by.to_string()),
            );
        }
        if let Some(pinned) = pinned {
            record.insert("pinned".to_string(), serde_json::Value::Bool(pinned));
        }
        let state = record
            .get("state")
            .cloned()
            .unwrap_or_else(|| serde_json::Value::String("active".to_string()));
        record.insert("state".to_string(), state);
        record.insert(
            "last_activity_at".to_string(),
            serde_json::Value::String(chrono::Utc::now().to_rfc3339()),
        );
        usage.insert(name.to_string(), serde_json::Value::Object(record));
        write_usage_json(&usage_path, usage).await
    }

    pub(in crate::runtime) async fn update_skill_usage_state(
        &self,
        name: &str,
        state: &str,
    ) -> Result<()> {
        let usage_path = self.data_dir().join("skills").join(".usage.json");
        let mut usage = read_usage_json(&usage_path).await;
        let mut record = usage
            .get(name)
            .and_then(|value| value.as_object().cloned())
            .unwrap_or_default();
        record.insert(
            "state".to_string(),
            serde_json::Value::String(state.to_string()),
        );
        record.insert(
            "last_activity_at".to_string(),
            serde_json::Value::String(chrono::Utc::now().to_rfc3339()),
        );
        usage.insert(name.to_string(), serde_json::Value::Object(record));
        write_usage_json(&usage_path, usage).await
    }

    pub(in crate::runtime) async fn increment_skill_usage_counter(
        &self,
        name: &str,
        counter: &str,
    ) -> Result<()> {
        let usage_path = self.data_dir().join("skills").join(".usage.json");
        let mut usage = read_usage_json(&usage_path).await;
        let mut record = usage
            .get(name)
            .and_then(|value| value.as_object().cloned())
            .unwrap_or_default();
        let current = record
            .get(counter)
            .and_then(|value| value.as_u64())
            .unwrap_or(0);
        record.insert(
            counter.to_string(),
            serde_json::Value::Number(serde_json::Number::from(current.saturating_add(1))),
        );
        if !record.contains_key("state") {
            record.insert(
                "state".to_string(),
                serde_json::Value::String("active".to_string()),
            );
        }
        record.insert(
            "last_activity_at".to_string(),
            serde_json::Value::String(chrono::Utc::now().to_rfc3339()),
        );
        usage.insert(name.to_string(), serde_json::Value::Object(record));
        write_usage_json(&usage_path, usage).await
    }
}
