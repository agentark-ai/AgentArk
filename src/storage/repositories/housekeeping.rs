use super::super::*;

impl Storage {
    /// Expire old pending approvals (older than max_age_secs)
    pub async fn expire_old_approvals(&self, max_age_secs: i64) -> Result<u64> {
        let cutoff = (chrono::Utc::now() - chrono::Duration::seconds(max_age_secs)).to_rfc3339();
        let resolved_at = chrono::Utc::now().to_rfc3339();
        let result = approval_log::Entity::update_many()
            .col_expr(approval_log::Column::Status, Expr::value("expired"))
            .col_expr(approval_log::Column::ResolvedAt, Expr::value(resolved_at))
            .col_expr(
                approval_log::Column::ResolvedBy,
                Expr::value("auto_timeout"),
            )
            .filter(approval_log::Column::Status.eq("pending"))
            .filter(approval_log::Column::RequestedAt.lt(cutoff))
            .exec(&self.db)
            .await?;
        Ok(result.rows_affected)
    }

    pub(super) const HOUSEKEEPING_PURGE_BATCH_SIZE: i64 = 1_000;

    pub(super) async fn delete_by_cutoff_in_batches(
        &self,
        table_name: &str,
        id_column: &str,
        cutoff_column: &str,
        cutoff: &str,
        extra_predicate_sql: &str,
    ) -> Result<u64> {
        let sql = format!(
            "DELETE FROM {table_name} \
             WHERE {id_column} IN ( \
                SELECT {id_column} \
                FROM {table_name} \
                WHERE {cutoff_column} < $1 {extra_predicate_sql} \
                ORDER BY {cutoff_column} ASC \
                LIMIT $2 \
             )"
        );
        let mut total_deleted = 0u64;
        loop {
            let result = self
                .db
                .execute(Statement::from_sql_and_values(
                    DbBackend::Postgres,
                    sql.clone(),
                    vec![
                        cutoff.to_string().into(),
                        Self::HOUSEKEEPING_PURGE_BATCH_SIZE.into(),
                    ],
                ))
                .await?;
            let deleted = result.rows_affected();
            total_deleted = total_deleted.saturating_add(deleted);
            if deleted == 0 {
                break;
            }
        }
        Ok(total_deleted)
    }

    pub(super) async fn delete_rows_by_ids<C>(
        conn: &C,
        table_name: &str,
        id_column: &str,
        ids: &[String],
    ) -> Result<()>
    where
        C: ConnectionTrait,
    {
        if ids.is_empty() {
            return Ok(());
        }
        let placeholders = ids
            .iter()
            .enumerate()
            .map(|(idx, _)| format!("${}", idx + 1))
            .collect::<Vec<_>>()
            .join(", ");
        let sql = format!("DELETE FROM {table_name} WHERE {id_column} IN ({placeholders})");
        let values = ids
            .iter()
            .cloned()
            .map(Into::into)
            .collect::<Vec<sea_orm::Value>>();
        conn.execute(Statement::from_sql_and_values(
            DbBackend::Postgres,
            sql,
            values,
        ))
        .await?;
        Ok(())
    }

    pub(super) async fn recount_conversations_after_message_batch<C>(
        conn: &C,
        conversation_ids: &[String],
        message_cutoff: &str,
    ) -> Result<()>
    where
        C: ConnectionTrait,
    {
        if conversation_ids.is_empty() {
            return Ok(());
        }

        let value_rows = conversation_ids
            .iter()
            .enumerate()
            .map(|(idx, _)| format!("(${})", idx + 1))
            .collect::<Vec<_>>()
            .join(", ");
        let values = conversation_ids
            .iter()
            .cloned()
            .map(Into::into)
            .collect::<Vec<sea_orm::Value>>();
        let update_sql = format!(
            "UPDATE conversations AS c \
             SET message_count = counts.message_count \
             FROM ( \
                SELECT ids.conversation_id, COUNT(m.id)::integer AS message_count \
                FROM (VALUES {value_rows}) AS ids(conversation_id) \
                LEFT JOIN messages AS m ON m.conversation_id = ids.conversation_id \
                GROUP BY ids.conversation_id \
             ) AS counts \
             WHERE c.id = counts.conversation_id"
        );
        conn.execute(Statement::from_sql_and_values(
            DbBackend::Postgres,
            update_sql,
            values.clone(),
        ))
        .await?;

        let placeholders = conversation_ids
            .iter()
            .enumerate()
            .map(|(idx, _)| format!("${}", idx + 1))
            .collect::<Vec<_>>()
            .join(", ");
        let mut delete_values = values;
        delete_values.push(message_cutoff.to_string().into());
        let delete_sql = format!(
            "DELETE FROM conversations \
             WHERE id IN ({placeholders}) \
               AND updated_at < ${} \
               AND message_count = 0",
            conversation_ids.len() + 1
        );
        conn.execute(Statement::from_sql_and_values(
            DbBackend::Postgres,
            delete_sql,
            delete_values,
        ))
        .await?;
        Ok(())
    }

    pub(super) async fn purge_message_batches(&self, message_cutoff: &str) -> Result<()> {
        loop {
            let txn = self.db.begin().await?;
            let deleted_rows = txn
                .query_all(Statement::from_sql_and_values(
                    DbBackend::Postgres,
                    "DELETE FROM messages \
                     WHERE id IN ( \
                        SELECT id \
                        FROM messages \
                        WHERE timestamp < $1 \
                        ORDER BY timestamp ASC \
                        LIMIT $2 \
                     ) \
                     RETURNING conversation_id",
                    vec![
                        message_cutoff.to_string().into(),
                        Self::HOUSEKEEPING_PURGE_BATCH_SIZE.into(),
                    ],
                ))
                .await?;
            if deleted_rows.is_empty() {
                txn.commit().await?;
                break;
            }
            let conversation_ids = deleted_rows
                .into_iter()
                .filter_map(|row| row.try_get::<String>("", "conversation_id").ok())
                .filter(|value| !value.trim().is_empty())
                .collect::<std::collections::BTreeSet<_>>()
                .into_iter()
                .collect::<Vec<_>>();
            Self::recount_conversations_after_message_batch(
                &txn,
                &conversation_ids,
                message_cutoff,
            )
            .await?;
            txn.commit().await?;
        }
        Ok(())
    }

    pub(super) async fn purge_execution_run_batches(
        &self,
        execution_run_cutoff: &str,
    ) -> Result<()> {
        loop {
            let txn = self.db.begin().await?;
            let rows = txn
                .query_all(Statement::from_sql_and_values(
                    DbBackend::Postgres,
                    "SELECT id \
                     FROM execution_runs \
                     WHERE created_at < $1 \
                     ORDER BY created_at ASC \
                     LIMIT $2",
                    vec![
                        execution_run_cutoff.to_string().into(),
                        Self::HOUSEKEEPING_PURGE_BATCH_SIZE.into(),
                    ],
                ))
                .await?;
            let run_ids = rows
                .into_iter()
                .filter_map(|row| row.try_get::<String>("", "id").ok())
                .filter(|value| !value.trim().is_empty())
                .collect::<Vec<_>>();
            if run_ids.is_empty() {
                txn.commit().await?;
                break;
            }
            Self::delete_rows_by_ids(&txn, "run_checkpoints", "run_id", &run_ids).await?;
            Self::delete_rows_by_ids(&txn, "tool_attempts", "run_id", &run_ids).await?;
            Self::delete_rows_by_ids(&txn, "execution_runs", "id", &run_ids).await?;
            txn.commit().await?;
        }
        Ok(())
    }

    pub(super) async fn purge_memory_operation_batches(
        &self,
        memory_operation_cutoff: &str,
    ) -> Result<()> {
        loop {
            let txn = self.db.begin().await?;
            let rows = txn
                .query_all(Statement::from_sql_and_values(
                    DbBackend::Postgres,
                    "SELECT id \
                     FROM memory_operations \
                     WHERE updated_at < $1 \
                       AND (applied_at IS NOT NULL OR reviewed_at IS NOT NULL) \
                     ORDER BY updated_at ASC \
                     LIMIT $2",
                    vec![
                        memory_operation_cutoff.to_string().into(),
                        Self::HOUSEKEEPING_PURGE_BATCH_SIZE.into(),
                    ],
                ))
                .await?;
            let operation_ids = rows
                .into_iter()
                .filter_map(|row| row.try_get::<String>("", "id").ok())
                .filter(|value| !value.trim().is_empty())
                .collect::<Vec<_>>();
            if operation_ids.is_empty() {
                txn.commit().await?;
                break;
            }
            Self::delete_rows_by_ids(
                &txn,
                "memory_evidence_links",
                "operation_id",
                &operation_ids,
            )
            .await?;
            Self::delete_rows_by_ids(&txn, "memory_operations", "id", &operation_ids).await?;
            txn.commit().await?;
        }
        Ok(())
    }

    pub(super) async fn maybe_purge_housekeeping_tables(&self) -> Result<()> {
        let now = chrono::Utc::now();
        let lifecycle =
            crate::core::runtime::data_lifecycle::load_data_lifecycle_settings(self).await;
        if !lifecycle.cleanup_enabled || !lifecycle.logs_cleanup_enabled {
            return Ok(());
        }
        if let Some(bytes) = self.get(Self::HOUSEKEEPING_PURGE_LAST_RUN_KEY).await? {
            if let Ok(raw) = String::from_utf8(bytes) {
                if let Ok(last) = chrono::DateTime::parse_from_rfc3339(&raw) {
                    if (now - last.with_timezone(&chrono::Utc)).num_seconds()
                        < lifecycle.housekeeping_interval_secs as i64
                    {
                        return Ok(());
                    }
                }
            }
        }

        let all_retention_disabled = lifecycle.execution_trace_retention_days == 0
            && lifecycle.execution_proof_retention_days == 0
            && lifecycle.operational_log_retention_days == 0
            && lifecycle.security_log_retention_days == 0
            && lifecycle.approval_log_retention_days == 0
            && lifecycle.swarm_delegation_retention_days == 0
            && lifecycle.llm_usage_retention_days == 0
            && lifecycle.terminal_task_retention_days == 0
            && lifecycle.execution_run_retention_days == 0
            && lifecycle.background_session_retention_days == 0
            && lifecycle.browser_session_retention_days == 0
            && lifecycle.automation_run_retention_days == 0
            && lifecycle.message_retention_days == 0
            && lifecycle.experience_run_retention_days == 0
            && lifecycle.experience_edge_retention_days == 0
            && lifecycle.learning_candidate_retention_days == 0
            && lifecycle.experience_item_retention_days == 0
            && lifecycle.procedural_pattern_retention_days == 0
            && lifecycle.recall_event_retention_days == 0
            && lifecycle.recall_test_retention_days == 0
            && lifecycle.readiness_evaluation_retention_days == 0
            && lifecycle.memory_capture_event_retention_days == 0
            && lifecycle.memory_operation_retention_days == 0
            && lifecycle.memory_evidence_link_retention_days == 0
            && lifecycle.semantic_work_unit_retention_days == 0;

        if all_retention_disabled {
            self.set(
                Self::HOUSEKEEPING_PURGE_LAST_RUN_KEY,
                now.to_rfc3339().as_bytes(),
            )
            .await?;
            return Ok(());
        }

        if lifecycle.message_retention_days > 0 {
            let message_cutoff = (now
                - chrono::Duration::days(lifecycle.message_retention_days as i64))
            .to_rfc3339();
            self.purge_message_batches(&message_cutoff).await?;
        }

        if lifecycle.execution_trace_retention_days > 0 {
            let trace_cutoff = (now
                - chrono::Duration::days(lifecycle.execution_trace_retention_days as i64))
            .to_rfc3339();
            self.delete_by_cutoff_in_batches(
                "execution_traces",
                "id",
                "created_at",
                &trace_cutoff,
                "",
            )
            .await?;
        }
        if lifecycle.execution_proof_retention_days > 0 {
            let proof_cutoff = (now
                - chrono::Duration::days(lifecycle.execution_proof_retention_days as i64))
            .to_rfc3339();
            self.delete_by_cutoff_in_batches(
                "execution_proofs",
                "id",
                "timestamp",
                &proof_cutoff,
                "",
            )
            .await?;
        }
        if lifecycle.operational_log_retention_days > 0 {
            let operational_cutoff = (now
                - chrono::Duration::days(lifecycle.operational_log_retention_days as i64))
            .to_rfc3339();
            self.delete_by_cutoff_in_batches(
                "operational_logs",
                "id",
                "created_at",
                &operational_cutoff,
                "",
            )
            .await?;
        }
        if lifecycle.security_log_retention_days > 0 {
            let security_cutoff = (now
                - chrono::Duration::days(lifecycle.security_log_retention_days as i64))
            .to_rfc3339();
            self.delete_by_cutoff_in_batches(
                "security_logs",
                "id",
                "created_at",
                &security_cutoff,
                "",
            )
            .await?;
        }
        if lifecycle.approval_log_retention_days > 0 {
            let approval_cutoff = (now
                - chrono::Duration::days(lifecycle.approval_log_retention_days as i64))
            .to_rfc3339();
            self.delete_by_cutoff_in_batches(
                "approval_log",
                "id",
                "requested_at",
                &approval_cutoff,
                "AND status <> 'pending'",
            )
            .await?;
        }
        if lifecycle.swarm_delegation_retention_days > 0 {
            let delegation_cutoff = (now
                - chrono::Duration::days(lifecycle.swarm_delegation_retention_days as i64))
            .to_rfc3339();
            self.delete_by_cutoff_in_batches(
                "swarm_delegations",
                "id",
                "created_at",
                &delegation_cutoff,
                "",
            )
            .await?;
        }
        if lifecycle.llm_usage_retention_days > 0 {
            let llm_usage_cutoff = (now
                - chrono::Duration::days(lifecycle.llm_usage_retention_days as i64))
            .to_rfc3339();
            self.delete_by_cutoff_in_batches(
                "llm_usage",
                "id",
                "created_at",
                &llm_usage_cutoff,
                "",
            )
            .await?;
        }
        if lifecycle.execution_run_retention_days > 0 {
            let execution_run_cutoff = (now
                - chrono::Duration::days(lifecycle.execution_run_retention_days as i64))
            .to_rfc3339();
            self.purge_execution_run_batches(&execution_run_cutoff)
                .await?;
        }
        if lifecycle.experience_run_retention_days > 0 {
            let experience_run_cutoff = (now
                - chrono::Duration::days(lifecycle.experience_run_retention_days as i64))
            .to_rfc3339();
            self.delete_by_cutoff_in_batches(
                "experience_runs",
                "id",
                "created_at",
                &experience_run_cutoff,
                "",
            )
            .await?;
        }
        if lifecycle.experience_edge_retention_days > 0 {
            let experience_edge_cutoff = (now
                - chrono::Duration::days(lifecycle.experience_edge_retention_days as i64))
            .to_rfc3339();
            self.delete_by_cutoff_in_batches(
                "experience_edges",
                "id",
                "created_at",
                &experience_edge_cutoff,
                "",
            )
            .await?;
        }
        if lifecycle.learning_candidate_retention_days > 0 {
            let learning_candidate_cutoff = (now
                - chrono::Duration::days(lifecycle.learning_candidate_retention_days as i64))
            .to_rfc3339();
            self.delete_by_cutoff_in_batches(
                "learning_candidates",
                "id",
                "created_at",
                &learning_candidate_cutoff,
                "",
            )
            .await?;
        }
        if lifecycle.experience_item_retention_days > 0 {
            let experience_item_cutoff = (now
                - chrono::Duration::days(lifecycle.experience_item_retention_days as i64))
            .to_rfc3339();
            self.delete_by_cutoff_in_batches(
                "experience_items",
                "id",
                "updated_at",
                &experience_item_cutoff,
                "AND status <> 'active'",
            )
            .await?;
        }
        if lifecycle.procedural_pattern_retention_days > 0 {
            let procedural_pattern_cutoff = (now
                - chrono::Duration::days(lifecycle.procedural_pattern_retention_days as i64))
            .to_rfc3339();
            self.delete_by_cutoff_in_batches(
                "procedural_patterns",
                "id",
                "updated_at",
                &procedural_pattern_cutoff,
                "AND status NOT IN ('active', 'draft')",
            )
            .await?;
        }
        if lifecycle.recall_event_retention_days > 0 {
            let recall_event_cutoff = (now
                - chrono::Duration::days(lifecycle.recall_event_retention_days as i64))
            .to_rfc3339();
            self.delete_by_cutoff_in_batches(
                "recall_events",
                "id",
                "created_at",
                &recall_event_cutoff,
                "",
            )
            .await?;
        }
        if lifecycle.recall_test_retention_days > 0 {
            let recall_test_cutoff = (now
                - chrono::Duration::days(lifecycle.recall_test_retention_days as i64))
            .to_rfc3339();
            self.delete_by_cutoff_in_batches(
                "recall_tests",
                "id",
                "updated_at",
                &recall_test_cutoff,
                "AND status IN ('retired', 'pending', 'passed', 'failed')",
            )
            .await?;
        }
        if lifecycle.readiness_evaluation_retention_days > 0 {
            let readiness_cutoff = (now
                - chrono::Duration::days(lifecycle.readiness_evaluation_retention_days as i64))
            .to_rfc3339();
            self.delete_by_cutoff_in_batches(
                "readiness_evaluations",
                "id",
                "created_at",
                &readiness_cutoff,
                "",
            )
            .await?;
        }
        if lifecycle.memory_capture_event_retention_days > 0 {
            let memory_capture_cutoff = (now
                - chrono::Duration::days(lifecycle.memory_capture_event_retention_days as i64))
            .to_rfc3339();
            self.delete_by_cutoff_in_batches(
                "memory_capture_events",
                "id",
                "completed_at",
                &memory_capture_cutoff,
                "",
            )
            .await?;
        }
        if lifecycle.memory_operation_retention_days > 0 {
            let memory_operation_cutoff = (now
                - chrono::Duration::days(lifecycle.memory_operation_retention_days as i64))
            .to_rfc3339();
            self.purge_memory_operation_batches(&memory_operation_cutoff)
                .await?;
        }
        if lifecycle.memory_evidence_link_retention_days > 0 {
            let memory_evidence_cutoff = (now
                - chrono::Duration::days(lifecycle.memory_evidence_link_retention_days as i64))
            .to_rfc3339();
            self.delete_by_cutoff_in_batches(
                "memory_evidence_links",
                "id",
                "created_at",
                &memory_evidence_cutoff,
                "",
            )
            .await?;
        }
        if lifecycle.semantic_work_unit_retention_days > 0 {
            let semantic_work_unit_cutoff = (now
                - chrono::Duration::days(lifecycle.semantic_work_unit_retention_days as i64))
            .to_rfc3339();
            self.delete_by_cutoff_in_batches(
                "semantic_work_units",
                "id",
                "occurred_at",
                &semantic_work_unit_cutoff,
                "",
            )
            .await?;
        }
        if lifecycle.background_session_retention_days > 0 {
            let background_session_cutoff = (now
                - chrono::Duration::days(lifecycle.background_session_retention_days as i64))
            .to_rfc3339();
            self.delete_by_cutoff_in_batches(
                "background_sessions",
                "id",
                "last_activity_at",
                &background_session_cutoff,
                "AND status IN ('completed', 'failed', 'cancelled')",
            )
            .await?;
        }
        if lifecycle.browser_session_retention_days > 0 {
            let browser_session_cutoff = (now
                - chrono::Duration::days(lifecycle.browser_session_retention_days as i64))
            .to_rfc3339();
            self.delete_by_cutoff_in_batches(
                "browser_sessions",
                "id",
                "updated_at",
                &browser_session_cutoff,
                "AND status IN ('completed', 'failed', 'interrupted')",
            )
            .await?;
        }
        if lifecycle.automation_run_retention_days > 0 {
            let automation_run_cutoff = (now
                - chrono::Duration::days(lifecycle.automation_run_retention_days as i64))
            .to_rfc3339();
            self.delete_by_cutoff_in_batches(
                "automation_runs",
                "id",
                "started_at",
                &automation_run_cutoff,
                "",
            )
            .await?;
        }

        if lifecycle.terminal_task_retention_days > 0 {
            let terminal_task_cutoff = (now
                - chrono::Duration::days(lifecycle.terminal_task_retention_days as i64))
            .to_rfc3339();
            let stale_tasks = task::Entity::find()
                .filter(task::Column::CreatedAt.lt(terminal_task_cutoff))
                .all(&self.db)
                .await?;
            for stale_task in stale_tasks {
                if stale_task.cron.is_some() {
                    continue;
                }
                let status = serde_json::from_str::<crate::core::TaskStatus>(&stale_task.status)
                    .unwrap_or(crate::core::TaskStatus::Pending);
                let terminal = matches!(
                    status,
                    crate::core::TaskStatus::Completed
                        | crate::core::TaskStatus::Cancelled
                        | crate::core::TaskStatus::Failed { .. }
                );
                if !terminal {
                    continue;
                }
                task::Entity::delete_by_id(stale_task.id)
                    .exec(&self.db)
                    .await?;
            }
        }

        self.set(
            Self::HOUSEKEEPING_PURGE_LAST_RUN_KEY,
            now.to_rfc3339().as_bytes(),
        )
        .await?;
        Ok(())
    }

    pub async fn run_housekeeping_purge(&self) -> Result<()> {
        self.maybe_purge_housekeeping_tables().await
    }
}
