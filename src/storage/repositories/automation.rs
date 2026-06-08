use super::super::*;

impl Storage {
    /// Insert a task
    pub async fn insert_task(&self, task: &crate::core::Task) -> Result<()> {
        let now = chrono::Utc::now().to_rfc3339();
        let description = encrypt_storage_string(&task.description)?;
        let arguments = encrypt_storage_string(&serde_json::to_string(&task.arguments)?)?;
        let approval = encrypt_storage_string(&serde_json::to_string(&task.approval)?)?;
        let result = encrypt_optional_storage_string(task.result.as_deref())?;
        task::ActiveModel {
            id: Set(task.id.to_string()),
            description: Set(description),
            action: Set(task.action.clone()),
            arguments: Set(arguments),
            approval: Set(approval),
            status: Set(serde_json::to_string(&task.status)?),
            created_at: Set(task.created_at.to_rfc3339()),
            updated_at: Set(now),
            scheduled_for: Set(task.scheduled_for.map(|t| t.to_rfc3339())),
            cron: Set(task.cron.clone()),
            result: Set(result),
            proof_id: Set(task.proof_id.map(|id| id.to_string())),
            priority: Set(task.priority.map(|v| v as f64)),
            urgency: Set(task.urgency.map(|v| v as f64)),
            importance: Set(task.importance.map(|v| v as f64)),
            eisenhower_quadrant: Set(task.eisenhower_quadrant.map(|v| v as i32)),
            lease_owner: Set(None),
            lease_expires_at: Set(None),
            lease_version: Set(0),
            next_retry_at: Set(None),
            last_run_id: Set(None),
            consecutive_failures: Set(0),
        }
        .insert(&self.db)
        .await?;

        Ok(())
    }

    /// Update task status
    pub async fn update_task_status(&self, id: &str, status: &str) -> Result<()> {
        task::ActiveModel {
            id: Set(id.to_string()),
            status: Set(status.to_string()),
            updated_at: Set(chrono::Utc::now().to_rfc3339()),
            lease_owner: Set(None),
            lease_expires_at: Set(None),
            ..Default::default()
        }
        .update(&self.db)
        .await?;

        Ok(())
    }

    /// Update task fields
    pub async fn update_task(
        &self,
        id: &str,
        description: Option<String>,
        arguments: Option<String>,
        cron: Option<String>,
        scheduled_for: Option<String>,
    ) -> Result<()> {
        let mut model = task::ActiveModel {
            id: Set(id.to_string()),
            ..Default::default()
        };

        if let Some(desc) = description {
            model.description = Set(encrypt_storage_string(&desc)?);
        }
        if let Some(args) = arguments {
            model.arguments = Set(encrypt_storage_string(&args)?);
        }
        if cron.is_some() {
            model.cron = Set(cron);
        }
        if scheduled_for.is_some() {
            model.scheduled_for = Set(scheduled_for);
        }
        model.updated_at = Set(chrono::Utc::now().to_rfc3339());

        model.update(&self.db).await?;
        Ok(())
    }

    pub async fn replace_task_schedule(
        &self,
        id: &str,
        cron: Option<String>,
        scheduled_for: Option<String>,
    ) -> Result<()> {
        task::ActiveModel {
            id: Set(id.to_string()),
            cron: Set(cron),
            scheduled_for: Set(scheduled_for),
            updated_at: Set(chrono::Utc::now().to_rfc3339()),
            ..Default::default()
        }
        .update(&self.db)
        .await?;
        Ok(())
    }

    pub async fn update_task_status_and_result(
        &self,
        id: &str,
        status: &str,
        result: Option<&str>,
    ) -> Result<()> {
        let mut model = task::ActiveModel {
            id: Set(id.to_string()),
            status: Set(status.to_string()),
            updated_at: Set(chrono::Utc::now().to_rfc3339()),
            lease_owner: Set(None),
            lease_expires_at: Set(None),
            ..Default::default()
        };
        if let Some(res) = result {
            model.result = Set(Some(encrypt_storage_string(res)?));
        }
        model.update(&self.db).await?;
        Ok(())
    }

    /// Reset a failed/cancelled task so it can be retried.
    pub async fn retry_task(
        &self,
        id: &str,
        status: &str,
        scheduled_for: Option<String>,
    ) -> Result<()> {
        task::ActiveModel {
            id: Set(id.to_string()),
            status: Set(status.to_string()),
            scheduled_for: Set(scheduled_for),
            result: Set(None),
            proof_id: Set(None),
            updated_at: Set(chrono::Utc::now().to_rfc3339()),
            lease_owner: Set(None),
            lease_expires_at: Set(None),
            ..Default::default()
        }
        .update(&self.db)
        .await?;

        Ok(())
    }

    pub async fn list_background_sessions(&self) -> Result<Vec<crate::core::BackgroundSession>> {
        let rows = background_session::Entity::find()
            .order_by_desc(background_session::Column::UpdatedAt)
            .all(&self.db)
            .await?;
        let mut sessions = Vec::with_capacity(rows.len());
        for row in rows {
            let payload = decrypt_storage_string(&row.payload);
            match serde_json::from_str::<crate::core::BackgroundSession>(&payload) {
                Ok(mut session) => {
                    session.policy = session.policy.normalized();
                    sessions.push(session);
                }
                Err(error) => {
                    tracing::warn!(
                        "Failed to parse persisted background session {}; skipping row: {}",
                        row.id,
                        error
                    );
                }
            }
        }
        Ok(sessions)
    }

    pub async fn upsert_background_session(
        &self,
        session: &crate::core::BackgroundSession,
    ) -> Result<()> {
        let payload = encrypt_storage_string(&serde_json::to_string(session)?)?;
        background_session::Entity::insert(background_session::ActiveModel {
            id: Set(session.id.clone()),
            status: Set(session.status.label().to_string()),
            conversation_id: Set(session.conversation_id.clone()),
            project_id: Set(session.project_id.clone()),
            created_at: Set(session.created_at.to_rfc3339()),
            updated_at: Set(session.updated_at.to_rfc3339()),
            last_activity_at: Set(session.last_activity_at.to_rfc3339()),
            payload: Set(payload),
        })
        .on_conflict(
            OnConflict::column(background_session::Column::Id)
                .update_columns([
                    background_session::Column::Status,
                    background_session::Column::ConversationId,
                    background_session::Column::ProjectId,
                    background_session::Column::UpdatedAt,
                    background_session::Column::LastActivityAt,
                    background_session::Column::Payload,
                ])
                .to_owned(),
        )
        .exec(&self.db)
        .await?;
        Ok(())
    }

    pub async fn delete_background_session(&self, id: &str) -> Result<()> {
        background_session::Entity::delete_by_id(id.to_string())
            .exec(&self.db)
            .await?;
        Ok(())
    }

    pub async fn try_claim_task(
        &self,
        id: &str,
        expected_status: &str,
        in_progress_status: &str,
        lease_owner: &str,
        lease_expires_at: &str,
    ) -> Result<bool> {
        let now = chrono::Utc::now().to_rfc3339();
        let result = task::Entity::update_many()
            .col_expr(
                task::Column::Status,
                Expr::value(in_progress_status.to_string()),
            )
            .col_expr(task::Column::UpdatedAt, Expr::value(now))
            .col_expr(
                task::Column::LeaseOwner,
                Expr::value(lease_owner.to_string()),
            )
            .col_expr(
                task::Column::LeaseExpiresAt,
                Expr::value(lease_expires_at.to_string()),
            )
            .col_expr(
                task::Column::LeaseVersion,
                Expr::col(task::Column::LeaseVersion).add(1),
            )
            .filter(task::Column::Id.eq(id))
            .filter(task::Column::Status.eq(expected_status))
            .filter(
                Condition::any()
                    .add(task::Column::LeaseExpiresAt.is_null())
                    .add(task::Column::LeaseExpiresAt.lte(chrono::Utc::now().to_rfc3339())),
            )
            .exec(&self.db)
            .await?;
        Ok(result.rows_affected > 0)
    }

    pub async fn record_task_run_metadata(
        &self,
        id: &str,
        last_run_id: Option<&str>,
        next_retry_at: Option<&str>,
        consecutive_failures: Option<i32>,
    ) -> Result<()> {
        task::ActiveModel {
            id: Set(id.to_string()),
            updated_at: Set(chrono::Utc::now().to_rfc3339()),
            last_run_id: Set(last_run_id.map(|value| value.to_string())),
            next_retry_at: Set(next_retry_at.map(|value| value.to_string())),
            consecutive_failures: Set(consecutive_failures.unwrap_or(0)),
            ..Default::default()
        }
        .update(&self.db)
        .await?;
        Ok(())
    }

    /// Delete a task
    pub async fn delete_task(&self, id: &str) -> Result<()> {
        let txn = self.db.begin().await?;
        self.cleanup_automation_records_for_ids(&txn, &[id.to_string()])
            .await?;

        let delegations = swarm_delegation::Entity::find()
            .filter(swarm_delegation::Column::ParentTaskId.eq(id.to_string()))
            .all(&txn)
            .await?;
        for row in delegations {
            swarm_delegation::ActiveModel {
                id: Unchanged(row.id),
                parent_task_id: Set(None),
                ..Default::default()
            }
            .update(&txn)
            .await?;
        }

        task::Entity::delete_by_id(id.to_string())
            .exec(&txn)
            .await?;
        txn.commit().await?;
        Ok(())
    }

    /// Get all tasks
    pub async fn get_tasks(&self) -> Result<Vec<task::Model>> {
        let mut tasks = task::Entity::find()
            .order_by_desc(task::Column::CreatedAt)
            .limit(Self::MAX_TASK_ROWS_PER_QUERY)
            .all(&self.db)
            .await?;
        for task in &mut tasks {
            task.description = decrypt_storage_string(&task.description);
            task.arguments = decrypt_storage_string(&task.arguments);
            task.approval = decrypt_storage_string(&task.approval);
            task.result = decrypt_optional_storage_string(task.result.take());
        }
        Ok(tasks)
    }

    /// List tasks updated inside a bounded time window.
    #[allow(dead_code)]
    pub async fn list_tasks_updated_between(
        &self,
        from: &str,
        to: &str,
        limit: u64,
    ) -> Result<Vec<task::Model>> {
        let mut tasks = task::Entity::find()
            .filter(task::Column::UpdatedAt.gte(from.to_string()))
            .filter(task::Column::UpdatedAt.lt(to.to_string()))
            .order_by_desc(task::Column::UpdatedAt)
            .limit(Self::db_limit(limit.min(Self::MAX_TASK_ROWS_PER_QUERY)))
            .all(&self.db)
            .await?;
        for task in &mut tasks {
            task.description = decrypt_storage_string(&task.description);
            task.arguments = decrypt_storage_string(&task.arguments);
            task.approval = decrypt_storage_string(&task.approval);
            task.result = decrypt_optional_storage_string(task.result.take());
        }
        Ok(tasks)
    }

    pub async fn list_automation_runs(
        &self,
        limit: usize,
    ) -> Result<Vec<crate::core::automation::AutomationRunRecord>> {
        let mut runs = Vec::new();
        for row in automation_run::Entity::find()
            .order_by_desc(automation_run::Column::StartedAt)
            .limit(limit.max(1) as u64)
            .all(&self.db)
            .await?
        {
            let payload = decrypt_storage_string(&row.payload);
            if let Ok(run) =
                serde_json::from_str::<crate::core::automation::AutomationRunRecord>(&payload)
            {
                runs.push(run);
            }
        }
        Ok(runs)
    }

    pub async fn list_automation_runs_since(
        &self,
        since: Option<&str>,
        limit: usize,
    ) -> Result<Vec<crate::core::automation::AutomationRunRecord>> {
        let mut query = automation_run::Entity::find()
            .order_by_desc(automation_run::Column::StartedAt)
            .limit(limit.max(1) as u64);
        if let Some(since) = since.map(str::trim).filter(|value| !value.is_empty()) {
            query = query.filter(automation_run::Column::StartedAt.gte(since.to_string()));
        }

        let mut runs = Vec::new();
        for row in query.all(&self.db).await? {
            let payload = decrypt_storage_string(&row.payload);
            if let Ok(run) =
                serde_json::from_str::<crate::core::automation::AutomationRunRecord>(&payload)
            {
                runs.push(run);
            }
        }
        Ok(runs)
    }

    pub async fn count_automation_runs(&self, since: Option<&str>) -> Result<u64> {
        let mut query = automation_run::Entity::find();
        if let Some(since) = since.map(str::trim).filter(|value| !value.is_empty()) {
            query = query.filter(automation_run::Column::StartedAt.gte(since.to_string()));
        }
        Ok(query.count(&self.db).await?)
    }

    pub async fn append_automation_run(
        &self,
        run: &crate::core::automation::AutomationRunRecord,
        max_records: usize,
    ) -> Result<()> {
        automation_run::Entity::insert(automation_run::ActiveModel {
            id: Set(run.id.clone()),
            automation_id: Set(run.automation_id.clone()),
            started_at: Set(run.started_at.clone()),
            payload: Set(encrypt_storage_string(&serde_json::to_string(run)?)?),
        })
        .on_conflict(
            OnConflict::column(automation_run::Column::Id)
                .update_columns([
                    automation_run::Column::AutomationId,
                    automation_run::Column::StartedAt,
                    automation_run::Column::Payload,
                ])
                .to_owned(),
        )
        .exec(&self.db)
        .await?;

        let overflow_ids = automation_run::Entity::find()
            .order_by_desc(automation_run::Column::StartedAt)
            .offset(max_records.max(1) as u64)
            .all(&self.db)
            .await?
            .into_iter()
            .map(|row| row.id)
            .collect::<Vec<_>>();
        if !overflow_ids.is_empty() {
            automation_run::Entity::delete_many()
                .filter(automation_run::Column::Id.is_in(overflow_ids))
                .exec(&self.db)
                .await?;
        }
        Ok(())
    }

    pub async fn list_automation_supervisor_states(
        &self,
    ) -> Result<Vec<crate::core::automation::AutomationSupervisorState>> {
        let mut states = Vec::new();
        for row in automation_supervisor_state::Entity::find()
            .order_by_desc(automation_supervisor_state::Column::UpdatedAt)
            .all(&self.db)
            .await?
        {
            let payload = decrypt_storage_string(&row.payload);
            if let Ok(state) =
                serde_json::from_str::<crate::core::automation::AutomationSupervisorState>(&payload)
            {
                states.push(state);
            }
        }
        Ok(states)
    }

    pub async fn load_automation_supervisor_state(
        &self,
        automation_id: &str,
    ) -> Result<Option<crate::core::automation::AutomationSupervisorState>> {
        Ok(
            automation_supervisor_state::Entity::find_by_id(automation_id.to_string())
                .one(&self.db)
                .await?
                .map(|row| decrypt_storage_string(&row.payload))
                .and_then(|payload| {
                    serde_json::from_str::<crate::core::automation::AutomationSupervisorState>(
                        &payload,
                    )
                    .ok()
                }),
        )
    }

    pub async fn upsert_automation_supervisor_state(
        &self,
        state: &crate::core::automation::AutomationSupervisorState,
    ) -> Result<()> {
        let updated_at = state
            .last_run_at
            .clone()
            .or_else(|| state.created_at.clone())
            .unwrap_or_else(|| chrono::Utc::now().to_rfc3339());
        automation_supervisor_state::Entity::insert(automation_supervisor_state::ActiveModel {
            automation_id: Set(state.automation_id.clone()),
            updated_at: Set(updated_at),
            payload: Set(encrypt_storage_string(&serde_json::to_string(state)?)?),
            next_retry_at: Set(state.next_retry_at.clone()),
            last_run_id: Set(state.last_run_id.clone()),
            consecutive_failures: Set(state.consecutive_failures as i32),
        })
        .on_conflict(
            OnConflict::column(automation_supervisor_state::Column::AutomationId)
                .update_columns([
                    automation_supervisor_state::Column::UpdatedAt,
                    automation_supervisor_state::Column::Payload,
                    automation_supervisor_state::Column::NextRetryAt,
                    automation_supervisor_state::Column::LastRunId,
                    automation_supervisor_state::Column::ConsecutiveFailures,
                ])
                .to_owned(),
        )
        .exec(&self.db)
        .await?;
        Ok(())
    }

    pub async fn delete_automation_supervisor_state(&self, automation_id: &str) -> Result<bool> {
        let result = automation_supervisor_state::Entity::delete_by_id(automation_id.to_string())
            .exec(&self.db)
            .await?;
        Ok(result.rows_affected > 0)
    }

    pub(super) async fn cleanup_automation_records_for_ids<C>(
        &self,
        db: &C,
        automation_ids: &[String],
    ) -> Result<()>
    where
        C: ConnectionTrait,
    {
        if automation_ids.is_empty() {
            return Ok(());
        }

        let automation_id_filter = automation_ids.to_vec();
        let run_ids = automation_run::Entity::find()
            .select_only()
            .column(automation_run::Column::Id)
            .filter(automation_run::Column::AutomationId.is_in(automation_id_filter.clone()))
            .into_tuple::<String>()
            .all(db)
            .await?;

        if !run_ids.is_empty() {
            let task_rows = task::Entity::find()
                .filter(task::Column::LastRunId.is_in(run_ids.clone()))
                .all(db)
                .await?;
            for row in task_rows {
                task::ActiveModel {
                    id: Unchanged(row.id),
                    last_run_id: Set(None),
                    ..Default::default()
                }
                .update(db)
                .await?;
            }

            let watcher_rows = watcher::Entity::find()
                .filter(watcher::Column::LastRunId.is_in(run_ids.clone()))
                .all(db)
                .await?;
            for row in watcher_rows {
                watcher::ActiveModel {
                    id: Unchanged(row.id),
                    last_run_id: Set(None),
                    ..Default::default()
                }
                .update(db)
                .await?;
            }

            let supervisor_rows = automation_supervisor_state::Entity::find()
                .filter(automation_supervisor_state::Column::LastRunId.is_in(run_ids.clone()))
                .all(db)
                .await?;
            for row in supervisor_rows {
                automation_supervisor_state::ActiveModel {
                    automation_id: Unchanged(row.automation_id),
                    last_run_id: Set(None),
                    ..Default::default()
                }
                .update(db)
                .await?;
            }
        }

        automation_supervisor_state::Entity::delete_many()
            .filter(
                automation_supervisor_state::Column::AutomationId
                    .is_in(automation_id_filter.clone()),
            )
            .exec(db)
            .await?;
        automation_run::Entity::delete_many()
            .filter(automation_run::Column::AutomationId.is_in(automation_id_filter))
            .exec(db)
            .await?;
        Ok(())
    }

    pub async fn list_watchers(&self) -> Result<Vec<crate::core::automation::watcher::Watcher>> {
        let mut watchers = Vec::new();
        let rows = watcher::Entity::find()
            .order_by_asc(watcher::Column::CreatedAt)
            .all(&self.db)
            .await?;
        for row in rows {
            let payload = row.payload;
            if let Ok(watcher) =
                serde_json::from_str::<crate::core::automation::watcher::Watcher>(&payload)
            {
                watchers.push(watcher);
            }
        }
        Ok(watchers)
    }

    pub async fn upsert_watcher(
        &self,
        watcher: &crate::core::automation::watcher::Watcher,
    ) -> Result<()> {
        let status = match &watcher.status {
            crate::core::automation::watcher::WatcherStatus::Active => "active",
            crate::core::automation::watcher::WatcherStatus::Paused => "paused",
            crate::core::automation::watcher::WatcherStatus::Triggered => "triggered",
            crate::core::automation::watcher::WatcherStatus::TimedOut => "timed_out",
            crate::core::automation::watcher::WatcherStatus::Cancelled => "cancelled",
            crate::core::automation::watcher::WatcherStatus::Failed { .. } => "failed",
        };
        watcher::Entity::insert(watcher::ActiveModel {
            id: Set(watcher.id.to_string()),
            status: Set(status.to_string()),
            created_at: Set(watcher.created_at.to_rfc3339()),
            updated_at: Set(chrono::Utc::now().to_rfc3339()),
            payload: Set(serde_json::to_string(watcher)?),
            lease_owner: Set(None),
            lease_expires_at: sea_orm::NotSet,
            lease_version: Set(0),
            next_retry_at: Set(watcher.next_poll_not_before.map(|value| value.to_rfc3339())),
            last_run_id: Set(None),
            consecutive_failures: Set(watcher.consecutive_failures as i32),
        })
        .on_conflict(
            OnConflict::column(watcher::Column::Id)
                .update_columns([
                    watcher::Column::Status,
                    watcher::Column::UpdatedAt,
                    watcher::Column::Payload,
                    watcher::Column::NextRetryAt,
                    watcher::Column::LastRunId,
                    watcher::Column::ConsecutiveFailures,
                ])
                .to_owned(),
        )
        .exec(&self.db)
        .await?;
        Ok(())
    }

    pub async fn delete_watcher(&self, id: &str) -> Result<()> {
        let txn = self.db.begin().await?;
        self.cleanup_automation_records_for_ids(&txn, &[id.to_string()])
            .await?;
        watcher::Entity::delete_by_id(id.to_string())
            .exec(&txn)
            .await?;
        txn.commit().await?;
        Ok(())
    }

    pub async fn list_browser_sessions(
        &self,
    ) -> Result<Vec<crate::core::connectivity::browser_session::PersistedBrowserSession>> {
        let rows = browser_session::Entity::find()
            .order_by_desc(browser_session::Column::UpdatedAt)
            .all(&self.db)
            .await?;
        rows.into_iter()
            .map(|row| {
                let task_description = decrypt_storage_string(&row.task_description);
                let chat_id = row.chat_id.map(|value| decrypt_storage_string(&value));
                let profile_id = row.profile_id.map(|value| decrypt_storage_string(&value));
                let profile_name = row.profile_name.map(|value| decrypt_storage_string(&value));
                let status_detail = row
                    .status_detail
                    .map(|value| decrypt_storage_string(&value));
                let action_history_json = decrypt_storage_string(&row.action_history_json);
                Ok(
                    crate::core::connectivity::browser_session::PersistedBrowserSession {
                        id: row.id,
                        status: row.status,
                        task_description,
                        channel: row.channel,
                        chat_id,
                        profile_id,
                        profile_name,
                        status_detail,
                        action_history: serde_json::from_str(&action_history_json)
                            .unwrap_or_default(),
                        created_at: row.created_at,
                        updated_at: row.updated_at,
                    },
                )
            })
            .collect()
    }

    pub async fn load_browser_session(
        &self,
        id: &str,
    ) -> Result<Option<crate::core::connectivity::browser_session::PersistedBrowserSession>> {
        let Some(row) = browser_session::Entity::find_by_id(id.to_string())
            .one(&self.db)
            .await?
        else {
            return Ok(None);
        };
        let task_description = decrypt_storage_string(&row.task_description);
        let chat_id = row.chat_id.map(|value| decrypt_storage_string(&value));
        let profile_id = row.profile_id.map(|value| decrypt_storage_string(&value));
        let profile_name = row.profile_name.map(|value| decrypt_storage_string(&value));
        let status_detail = row
            .status_detail
            .map(|value| decrypt_storage_string(&value));
        let action_history_json = decrypt_storage_string(&row.action_history_json);
        Ok(Some(
            crate::core::connectivity::browser_session::PersistedBrowserSession {
                id: row.id,
                status: row.status,
                task_description,
                channel: row.channel,
                chat_id,
                profile_id,
                profile_name,
                status_detail,
                action_history: serde_json::from_str(&action_history_json).unwrap_or_default(),
                created_at: row.created_at,
                updated_at: row.updated_at,
            },
        ))
    }

    pub async fn upsert_browser_session(
        &self,
        session: &crate::core::connectivity::browser_session::PersistedBrowserSession,
    ) -> Result<()> {
        browser_session::Entity::insert(browser_session::ActiveModel {
            id: Set(session.id.clone()),
            status: Set(session.status.clone()),
            task_description: Set(encrypt_storage_string(&session.task_description)?),
            channel: Set(session.channel.clone()),
            chat_id: Set(encrypt_optional_storage_string(session.chat_id.as_deref())?),
            profile_id: Set(encrypt_optional_storage_string(
                session.profile_id.as_deref(),
            )?),
            profile_name: Set(encrypt_optional_storage_string(
                session.profile_name.as_deref(),
            )?),
            status_detail: Set(encrypt_optional_storage_string(
                session.status_detail.as_deref(),
            )?),
            action_history_json: Set(encrypt_storage_string(&serde_json::to_string(
                &session.action_history,
            )?)?),
            created_at: Set(session.created_at.clone()),
            updated_at: Set(session.updated_at.clone()),
        })
        .on_conflict(
            OnConflict::column(browser_session::Column::Id)
                .update_columns([
                    browser_session::Column::Status,
                    browser_session::Column::TaskDescription,
                    browser_session::Column::Channel,
                    browser_session::Column::ChatId,
                    browser_session::Column::ProfileId,
                    browser_session::Column::ProfileName,
                    browser_session::Column::StatusDetail,
                    browser_session::Column::ActionHistoryJson,
                    browser_session::Column::UpdatedAt,
                ])
                .to_owned(),
        )
        .exec(&self.db)
        .await?;
        Ok(())
    }

    pub async fn delete_browser_session(&self, id: &str) -> Result<()> {
        browser_session::Entity::delete_by_id(id.to_string())
            .exec(&self.db)
            .await?;
        Ok(())
    }
}
