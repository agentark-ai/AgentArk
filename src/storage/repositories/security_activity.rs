use super::super::*;

impl Storage {
    // ==================== Security Logs ====================

    /// Insert a security log entry
    pub async fn insert_security_log(&self, log: &security_log::Model) -> Result<()> {
        security_log::ActiveModel {
            id: Set(log.id.clone()),
            event_type: Set(log.event_type.clone()),
            severity: Set(log.severity.clone()),
            message: Set(encrypt_storage_string(&log.message)?),
            source: Set(encrypt_optional_storage_string(log.source.as_deref())?),
            count: Set(log.count),
            created_at: Set(log.created_at.clone()),
        }
        .insert(&self.db)
        .await?;
        Ok(())
    }

    /// Insert multiple security log entries atomically.
    pub async fn insert_security_logs(&self, logs: &[security_log::Model]) -> Result<()> {
        if logs.is_empty() {
            return Ok(());
        }

        let txn = self.db.begin().await?;
        for log in logs {
            security_log::ActiveModel {
                id: Set(log.id.clone()),
                event_type: Set(log.event_type.clone()),
                severity: Set(log.severity.clone()),
                message: Set(encrypt_storage_string(&log.message)?),
                source: Set(encrypt_optional_storage_string(log.source.as_deref())?),
                count: Set(log.count),
                created_at: Set(log.created_at.clone()),
            }
            .insert(&txn)
            .await?;
        }
        txn.commit().await?;
        Ok(())
    }

    /// List recent security logs (newest first)
    pub async fn list_security_logs(&self, limit: u64) -> Result<Vec<security_log::Model>> {
        let mut logs = security_log::Entity::find()
            .order_by_desc(security_log::Column::CreatedAt)
            .limit(Self::db_limit(limit))
            .all(&self.db)
            .await?;
        for log in &mut logs {
            log.message = decrypt_storage_string(&log.message);
            log.source = decrypt_optional_storage_string(log.source.clone());
        }
        Ok(logs)
    }

    /// List security logs with pagination and optional event-type filter.
    pub async fn list_security_logs_paginated(
        &self,
        limit: u64,
        offset: u64,
        event_type: Option<&str>,
    ) -> Result<Vec<security_log::Model>> {
        let mut query = security_log::Entity::find().order_by_desc(security_log::Column::CreatedAt);

        if let Some(et) = event_type.filter(|s| !s.trim().is_empty()) {
            query = query.filter(security_log::Column::EventType.eq(et.trim().to_string()));
        }

        let mut logs = query
            .limit(Self::db_limit(limit))
            .offset(Self::db_offset(offset))
            .all(&self.db)
            .await?;
        for log in &mut logs {
            log.message = decrypt_storage_string(&log.message);
            log.source = decrypt_optional_storage_string(log.source.clone());
        }
        Ok(logs)
    }

    /// Count security logs for pagination (optional event-type filter).
    pub async fn count_security_logs(&self, event_type: Option<&str>) -> Result<u64> {
        let mut query = security_log::Entity::find();
        if let Some(et) = event_type.filter(|s| !s.trim().is_empty()) {
            query = query.filter(security_log::Column::EventType.eq(et.trim().to_string()));
        }
        Ok(query.count(&self.db).await?)
    }

    /// Delete security logs older than the given number of days
    pub async fn cleanup_old_security_logs(&self, max_age_days: i64) -> Result<u64> {
        let cutoff = (chrono::Utc::now() - chrono::Duration::days(max_age_days)).to_rfc3339();
        let result = security_log::Entity::delete_many()
            .filter(security_log::Column::CreatedAt.lt(cutoff))
            .exec(&self.db)
            .await?;
        Ok(result.rows_affected)
    }

    // ==================== Pulse History ====================

    /// Insert an Pulse history event row.
    pub async fn insert_arkpulse_event(&self, event: &arkpulse_event::Model) -> Result<()> {
        arkpulse_event::Entity::insert(arkpulse_event::ActiveModel {
            id: Set(event.id.clone()),
            timestamp: Set(event.timestamp.clone()),
            status: Set(event.status.clone()),
            message: Set(encrypt_storage_string(&event.message)?),
            summary: Set(encrypt_storage_string(&event.summary)?),
            flags_json: Set(encrypt_storage_string(&event.flags_json)?),
            overdue_tasks: Set(event.overdue_tasks),
            failed_tasks: Set(event.failed_tasks),
            details_json: Set(encrypt_storage_string(&event.details_json)?),
        })
        .on_conflict(
            OnConflict::column(arkpulse_event::Column::Id)
                .do_nothing()
                .to_owned(),
        )
        .exec(&self.db)
        .await?;
        Ok(())
    }

    /// Count persisted Pulse history rows.
    pub async fn count_arkpulse_events(&self) -> Result<u64> {
        arkpulse_event::Entity::find()
            .count(&self.db)
            .await
            .map_err(Into::into)
    }

    /// List Pulse history rows (newest first).
    pub async fn list_arkpulse_events(&self, limit: u64) -> Result<Vec<arkpulse_event::Model>> {
        let mut rows = arkpulse_event::Entity::find()
            .order_by_desc(arkpulse_event::Column::Timestamp)
            .limit(Self::db_limit(limit))
            .all(&self.db)
            .await?;
        for row in &mut rows {
            row.message = decrypt_storage_string(&row.message);
            row.summary = decrypt_storage_string(&row.summary);
            row.flags_json = decrypt_storage_string(&row.flags_json);
            row.details_json = decrypt_storage_string(&row.details_json);
        }
        Ok(rows)
    }

    /// List Pulse history rows inside a bounded time window.
    #[allow(dead_code)]
    pub async fn list_arkpulse_events_between(
        &self,
        from_rfc3339: &str,
        to_rfc3339: &str,
        limit: u64,
    ) -> Result<Vec<arkpulse_event::Model>> {
        let mut rows = arkpulse_event::Entity::find()
            .filter(arkpulse_event::Column::Timestamp.gte(from_rfc3339.to_string()))
            .filter(arkpulse_event::Column::Timestamp.lt(to_rfc3339.to_string()))
            .order_by_desc(arkpulse_event::Column::Timestamp)
            .limit(Self::db_limit(limit))
            .all(&self.db)
            .await?;
        for row in &mut rows {
            row.message = decrypt_storage_string(&row.message);
            row.summary = decrypt_storage_string(&row.summary);
            row.flags_json = decrypt_storage_string(&row.flags_json);
            row.details_json = decrypt_storage_string(&row.details_json);
        }
        Ok(rows)
    }

    /// Delete Pulse history rows older than the provided cutoff.
    pub async fn delete_arkpulse_events_before(&self, cutoff_rfc3339: &str) -> Result<u64> {
        let result = arkpulse_event::Entity::delete_many()
            .filter(arkpulse_event::Column::Timestamp.lt(cutoff_rfc3339.to_string()))
            .exec(&self.db)
            .await?;
        Ok(result.rows_affected)
    }

    /// Delete Pulse history rows by explicit IDs.
    pub async fn delete_arkpulse_events_by_ids(&self, ids: &[String]) -> Result<u64> {
        if ids.is_empty() {
            return Ok(0);
        }
        let result = arkpulse_event::Entity::delete_many()
            .filter(arkpulse_event::Column::Id.is_in(ids.to_vec()))
            .exec(&self.db)
            .await?;
        Ok(result.rows_affected)
    }

    /// Return Pulse history IDs that exceed the latest retained window.
    pub async fn list_arkpulse_event_ids_beyond_latest(
        &self,
        keep_latest: u64,
    ) -> Result<Vec<String>> {
        let rows = arkpulse_event::Entity::find()
            .order_by_desc(arkpulse_event::Column::Timestamp)
            .offset(Self::db_offset(keep_latest))
            .all(&self.db)
            .await?;
        Ok(rows.into_iter().map(|row| row.id).collect())
    }

    // ==================== Operational Logs ====================

    /// Insert a structured operational telemetry entry.
    pub async fn insert_operational_log(&self, log: &operational_log::Model) -> Result<()> {
        let trace_id = match log
            .trace_id
            .as_deref()
            .map(str::trim)
            .filter(|id| !id.is_empty())
        {
            Some(id) => match execution_trace::Entity::find_by_id(id.to_string())
                .one(&self.db)
                .await
            {
                Ok(Some(_)) => Some(id.to_string()),
                Ok(None) => {
                    tracing::debug!(
                        "Dropping operational log trace_id before insert because it does not resolve to an execution trace"
                    );
                    None
                }
                Err(error) => {
                    tracing::warn!(
                        "Dropping operational log trace_id before insert because validation failed: {}",
                        error
                    );
                    None
                }
            },
            None => None,
        };
        let conversation_id = match log
            .conversation_id
            .as_deref()
            .map(str::trim)
            .filter(|id| !id.is_empty())
        {
            Some(id) => match conversation::Entity::find_by_id(id.to_string())
                .one(&self.db)
                .await
            {
                Ok(Some(_)) => Some(id.to_string()),
                Ok(None) => {
                    tracing::debug!(
                        "Dropping operational log conversation_id before insert because it does not resolve to a conversation"
                    );
                    None
                }
                Err(error) => {
                    tracing::warn!(
                        "Dropping operational log conversation_id before insert because validation failed: {}",
                        error
                    );
                    None
                }
            },
            None => None,
        };
        let insert_result = operational_log::ActiveModel {
            id: Set(log.id.clone()),
            created_at: Set(log.created_at.clone()),
            trace_id: Set(trace_id.clone()),
            conversation_id: Set(conversation_id.clone()),
            channel: Set(log.channel.clone()),
            event_type: Set(log.event_type.clone()),
            success: Set(log.success),
            outcome: Set(encrypt_storage_string(&log.outcome)?),
            tool_name: Set(log.tool_name.clone()),
            latency_ms: Set(log.latency_ms),
            arguments: Set(encrypt_optional_storage_string(log.arguments.as_deref())?),
            payload: Set(encrypt_optional_storage_string(log.payload.as_deref())?),
            strategy_version: Set(log.strategy_version.clone()),
            policy_version: Set(log.policy_version.clone()),
            prompt_version: Set(log.prompt_version.clone()),
            model_slot: Set(log.model_slot.clone()),
        }
        .insert(&self.db)
        .await;
        if let Err(error) = insert_result {
            if (trace_id.is_some() || conversation_id.is_some())
                && is_foreign_key_constraint_error(&error)
            {
                tracing::warn!(
                    "Retrying operational log insert '{}' without trace_id/conversation_id after FK failure: {}",
                    log.id,
                    error
                );
                operational_log::ActiveModel {
                    id: Set(log.id.clone()),
                    created_at: Set(log.created_at.clone()),
                    trace_id: Set(None),
                    conversation_id: Set(None),
                    channel: Set(log.channel.clone()),
                    event_type: Set(log.event_type.clone()),
                    success: Set(log.success),
                    outcome: Set(encrypt_storage_string(&log.outcome)?),
                    tool_name: Set(log.tool_name.clone()),
                    latency_ms: Set(log.latency_ms),
                    arguments: Set(encrypt_optional_storage_string(log.arguments.as_deref())?),
                    payload: Set(encrypt_optional_storage_string(log.payload.as_deref())?),
                    strategy_version: Set(log.strategy_version.clone()),
                    policy_version: Set(log.policy_version.clone()),
                    prompt_version: Set(log.prompt_version.clone()),
                    model_slot: Set(log.model_slot.clone()),
                }
                .insert(&self.db)
                .await?;
            } else {
                return Err(error.into());
            }
        }
        Ok(())
    }

    /// List operational logs by event type (newest first).
    pub async fn list_operational_logs_by_event(
        &self,
        event_type: &str,
        limit: u64,
    ) -> Result<Vec<operational_log::Model>> {
        let mut rows = operational_log::Entity::find()
            .filter(operational_log::Column::EventType.eq(event_type.to_string()))
            .order_by_desc(operational_log::Column::CreatedAt)
            .limit(Self::db_limit(limit))
            .all(&self.db)
            .await?;
        for row in &mut rows {
            row.outcome = decrypt_storage_string(&row.outcome);
            row.arguments = decrypt_optional_storage_string(row.arguments.clone());
            row.payload = decrypt_optional_storage_string(row.payload.clone());
        }
        Ok(rows)
    }

    /// List operational logs for analytics without starving the selected time window.
    /// Returns a truncation flag when the server-side safety cap is reached.
    pub async fn list_operational_logs_by_event_window_complete(
        &self,
        event_type: &str,
        from_rfc3339: &str,
        to_rfc3339: &str,
    ) -> Result<(Vec<operational_log::Model>, bool)> {
        let mut rows = Vec::new();
        let mut cursor: Option<(String, String)> = None;
        loop {
            let mut query = operational_log::Entity::find()
                .filter(operational_log::Column::EventType.eq(event_type.to_string()))
                .filter(operational_log::Column::CreatedAt.gte(from_rfc3339.to_string()))
                .filter(operational_log::Column::CreatedAt.lt(to_rfc3339.to_string()));
            if let Some((created_at, id)) = cursor.as_ref() {
                query = query.filter(
                    Condition::any()
                        .add(operational_log::Column::CreatedAt.gt(created_at.clone()))
                        .add(
                            Condition::all()
                                .add(operational_log::Column::CreatedAt.eq(created_at.clone()))
                                .add(operational_log::Column::Id.gt(id.clone())),
                        ),
                );
            }
            let mut page = query
                .order_by_asc(operational_log::Column::CreatedAt)
                .order_by_asc(operational_log::Column::Id)
                .limit(Self::MAX_OPERATIONAL_LOG_ROWS_PER_QUERY)
                .all(&self.db)
                .await?;
            if page.is_empty() {
                return Ok((rows, false));
            }
            let page_len = page.len();
            for row in &mut page {
                row.outcome = decrypt_storage_string(&row.outcome);
                row.arguments = decrypt_optional_storage_string(row.arguments.clone());
                row.payload = decrypt_optional_storage_string(row.payload.clone());
            }
            for row in page {
                cursor = Some((row.created_at.clone(), row.id.clone()));
                if rows.len() >= Self::MAX_OPERATIONAL_LOG_ANALYTICS_ROWS {
                    return Ok((rows, true));
                }
                rows.push(row);
            }
            if page_len < Self::MAX_OPERATIONAL_LOG_ROWS_PER_QUERY as usize {
                return Ok((rows, false));
            }
        }
    }

    /// List recent operational logs across AgentArk modules (newest first).
    pub async fn list_recent_operational_logs(
        &self,
        since: Option<&str>,
        limit: u64,
        offset: u64,
    ) -> Result<Vec<operational_log::Model>> {
        let mut query = operational_log::Entity::find()
            .order_by_desc(operational_log::Column::CreatedAt)
            .limit(Self::db_limit(limit))
            .offset(Self::db_offset(offset));
        if let Some(since) = since.map(str::trim).filter(|value| !value.is_empty()) {
            query = query.filter(operational_log::Column::CreatedAt.gte(since.to_string()));
        }

        let mut rows = query.all(&self.db).await?;
        for row in &mut rows {
            row.outcome = decrypt_storage_string(&row.outcome);
            row.arguments = decrypt_optional_storage_string(row.arguments.clone());
            row.payload = decrypt_optional_storage_string(row.payload.clone());
        }
        Ok(rows)
    }

    pub async fn count_operational_logs(&self, since: Option<&str>) -> Result<u64> {
        let mut query = operational_log::Entity::find();
        if let Some(since) = since.map(str::trim).filter(|value| !value.is_empty()) {
            query = query.filter(operational_log::Column::CreatedAt.gte(since.to_string()));
        }
        Ok(query.count(&self.db).await?)
    }

    /// List recent operational logs for a set of trace ids (newest first).
    pub async fn list_operational_logs_for_trace_ids(
        &self,
        trace_ids: &[String],
        limit: u64,
    ) -> Result<Vec<operational_log::Model>> {
        if trace_ids.is_empty() {
            return Ok(Vec::new());
        }

        let mut rows = operational_log::Entity::find()
            .filter(operational_log::Column::TraceId.is_in(trace_ids.to_vec()))
            .order_by_desc(operational_log::Column::CreatedAt)
            .limit(Self::db_limit(limit))
            .all(&self.db)
            .await?;
        for row in &mut rows {
            row.outcome = decrypt_storage_string(&row.outcome);
            row.arguments = decrypt_optional_storage_string(row.arguments.clone());
            row.payload = decrypt_optional_storage_string(row.payload.clone());
        }
        Ok(rows)
    }

    /// List recent operational logs for a set of trace ids and one event type (newest first).
    pub async fn list_operational_logs_for_trace_ids_by_event(
        &self,
        trace_ids: &[String],
        event_type: &str,
        limit: u64,
    ) -> Result<Vec<operational_log::Model>> {
        if trace_ids.is_empty() {
            return Ok(Vec::new());
        }

        let mut rows = operational_log::Entity::find()
            .filter(operational_log::Column::TraceId.is_in(trace_ids.to_vec()))
            .filter(operational_log::Column::EventType.eq(event_type.to_string()))
            .order_by_desc(operational_log::Column::CreatedAt)
            .limit(Self::db_limit(limit))
            .all(&self.db)
            .await?;
        for row in &mut rows {
            row.outcome = decrypt_storage_string(&row.outcome);
            row.arguments = decrypt_optional_storage_string(row.arguments.clone());
            row.payload = decrypt_optional_storage_string(row.payload.clone());
        }
        Ok(rows)
    }

    pub(super) async fn database_column_schema_rows(&self) -> Result<Vec<DatabaseColumnSchemaRow>> {
        let columns_alias = Alias::new("columns");
        let query = Query::select()
            .columns([
                (columns_alias.clone(), Alias::new("table_schema")),
                (columns_alias.clone(), Alias::new("table_name")),
                (columns_alias.clone(), Alias::new("column_name")),
                (columns_alias.clone(), Alias::new("data_type")),
                (columns_alias.clone(), Alias::new("udt_name")),
                (columns_alias.clone(), Alias::new("is_nullable")),
                (columns_alias.clone(), Alias::new("column_default")),
                (columns_alias.clone(), Alias::new("ordinal_position")),
            ])
            .from((Alias::new("information_schema"), columns_alias.clone()))
            .and_where(Expr::col((columns_alias.clone(), Alias::new("table_schema"))).eq("public"))
            .order_by(
                (columns_alias.clone(), Alias::new("table_name")),
                Order::Asc,
            )
            .order_by(
                (columns_alias.clone(), Alias::new("ordinal_position")),
                Order::Asc,
            )
            .to_owned();
        let rows = self.db.query_all(DbBackend::Postgres.build(&query)).await?;
        rows.into_iter()
            .map(|row| DatabaseColumnSchemaRow::from_query_result(&row, "").map_err(Into::into))
            .collect()
    }

    pub(super) async fn database_column_names_for_table(&self, table: &str) -> Result<Vec<String>> {
        let table = normalize_public_table_name(table)?;
        Ok(self
            .database_column_schema_rows()
            .await?
            .into_iter()
            .filter(|row| row.table_schema == "public" && row.table_name == table)
            .map(|row| row.column_name)
            .collect())
    }

    pub(super) fn build_structured_db_filter_expr(
        table_alias: &str,
        filter: &ReadonlyTableFilter,
    ) -> Result<SimpleExpr> {
        let column = normalize_db_column_name(&filter.column)?;
        let op = filter.op.trim().to_ascii_lowercase();
        let expr = Expr::col((Alias::new(table_alias), Alias::new(column.as_str())));
        match op.as_str() {
            "eq" => Ok(expr.eq(json_scalar_to_simple_expr(
                filter
                    .value
                    .as_ref()
                    .ok_or_else(|| anyhow!("Filter '{}' requires a value", filter.column))?,
            )?)),
            "neq" => Ok(expr.ne(json_scalar_to_simple_expr(
                filter
                    .value
                    .as_ref()
                    .ok_or_else(|| anyhow!("Filter '{}' requires a value", filter.column))?,
            )?)),
            "gt" => Ok(expr.gt(json_scalar_to_simple_expr(
                filter
                    .value
                    .as_ref()
                    .ok_or_else(|| anyhow!("Filter '{}' requires a value", filter.column))?,
            )?)),
            "gte" => Ok(expr.gte(json_scalar_to_simple_expr(
                filter
                    .value
                    .as_ref()
                    .ok_or_else(|| anyhow!("Filter '{}' requires a value", filter.column))?,
            )?)),
            "lt" => Ok(expr.lt(json_scalar_to_simple_expr(
                filter
                    .value
                    .as_ref()
                    .ok_or_else(|| anyhow!("Filter '{}' requires a value", filter.column))?,
            )?)),
            "lte" => Ok(expr.lte(json_scalar_to_simple_expr(
                filter
                    .value
                    .as_ref()
                    .ok_or_else(|| anyhow!("Filter '{}' requires a value", filter.column))?,
            )?)),
            "contains" => {
                let value = filter
                    .value
                    .as_ref()
                    .and_then(|value| value.as_str())
                    .ok_or_else(|| anyhow!("Filter '{}' requires a string value", filter.column))?;
                Ok(expr.like(format!("%{}%", value)))
            }
            "starts_with" => {
                let value = filter
                    .value
                    .as_ref()
                    .and_then(|value| value.as_str())
                    .ok_or_else(|| anyhow!("Filter '{}' requires a string value", filter.column))?;
                Ok(expr.like(format!("{}%", value)))
            }
            "ends_with" => {
                let value = filter
                    .value
                    .as_ref()
                    .and_then(|value| value.as_str())
                    .ok_or_else(|| anyhow!("Filter '{}' requires a string value", filter.column))?;
                Ok(expr.like(format!("%{}", value)))
            }
            "in" => {
                let values = filter
                    .value
                    .as_ref()
                    .and_then(|value| value.as_array())
                    .ok_or_else(|| anyhow!("Filter '{}' requires an array value", filter.column))?
                    .iter()
                    .map(json_scalar_to_simple_expr)
                    .collect::<Result<Vec<_>>>()?;
                if values.is_empty() {
                    anyhow::bail!("Filter '{}' requires a non-empty array", filter.column);
                }
                Ok(expr.is_in(values))
            }
            "is_null" => Ok(expr.is_null()),
            "not_null" => Ok(expr.is_not_null()),
            _ => anyhow::bail!(
                "Unsupported filter operator '{}'. Use eq, neq, gt, gte, lt, lte, contains, starts_with, ends_with, in, is_null, or not_null",
                filter.op
            ),
        }
    }

    /// Inspect the live Postgres schema for agent-facing diagnostics.
    pub async fn inspect_postgres_schema_json(
        &self,
        table_filter: Option<&str>,
        limit: u64,
    ) -> Result<serde_json::Value> {
        let filter = table_filter
            .map(str::trim)
            .filter(|value| !value.is_empty())
            .map(|value| value.to_ascii_lowercase());
        let mut tables = Vec::new();
        let mut grouped =
            std::collections::BTreeMap::<(String, String), Vec<DatabaseColumnSchemaRow>>::new();
        for row in self.database_column_schema_rows().await? {
            if let Some(filter) = filter.as_deref() {
                let table_name = row.table_name.to_ascii_lowercase();
                let schema_name = row.table_schema.to_ascii_lowercase();
                if !table_name.contains(filter) && !schema_name.contains(filter) {
                    continue;
                }
            }
            grouped
                .entry((row.table_schema.clone(), row.table_name.clone()))
                .or_default()
                .push(row);
        }
        for ((schema, table), mut columns) in grouped.into_iter().take(limit.clamp(1, 100) as usize)
        {
            columns.sort_by_key(|row| row.ordinal_position);
            tables.push(serde_json::json!({
                "schema": schema,
                "table": table,
                "columns": columns.into_iter().map(|column| serde_json::json!({
                    "name": column.column_name,
                    "type": column.data_type,
                    "udt_name": column.udt_name,
                    "nullable": column.is_nullable.eq_ignore_ascii_case("YES"),
                    "default": column.column_default,
                    "ordinal_position": column.ordinal_position,
                })).collect::<Vec<_>>(),
            }));
        }

        Ok(serde_json::json!({
            "generated_at": chrono::Utc::now().to_rfc3339(),
            "schema": "public",
            "table_filter": table_filter.map(str::trim).filter(|value| !value.is_empty()),
            "table_count": tables.len(),
            "tables": tables,
            "relationships": Vec::<serde_json::Value>::new(),
            "notes": [
                "Only public-schema AgentArk tables are exposed here.",
                "Use the returned table and column names with structured postgres_query_readonly calls."
            ],
        }))
    }

    /// Execute a structured, read-only table query against the live Postgres database.
    pub async fn query_table_json(
        &self,
        request: &ReadonlyTableQuery,
    ) -> Result<serde_json::Value> {
        let table = normalize_public_table_name(&request.table)?;
        let known_tables = self.database_table_names().await?;
        if !known_tables.iter().any(|name| name == &table) {
            anyhow::bail!(
                "Unknown table '{}'. Inspect the live schema with postgres_schema_inspect and retry with a valid public table name",
                table
            );
        }

        let available_columns = self.database_column_names_for_table(&table).await?;
        if available_columns.is_empty() {
            anyhow::bail!("Table '{}' has no readable columns", table);
        }

        let mut selected_columns = Vec::new();
        let mut seen = std::collections::HashSet::new();
        let requested_columns = if request.columns.is_empty() {
            available_columns.clone()
        } else {
            request
                .columns
                .iter()
                .map(|column| normalize_db_column_name(column))
                .collect::<Result<Vec<_>>>()?
        };
        for column in requested_columns {
            if !available_columns.iter().any(|name| name == &column) {
                anyhow::bail!(
                    "Unknown column '{}.{}'. Inspect the live schema with postgres_schema_inspect and retry with a valid column name",
                    table,
                    column
                );
            }
            if seen.insert(column.clone()) {
                selected_columns.push(column);
            }
        }

        for filter in &request.filters {
            let column = normalize_db_column_name(&filter.column)?;
            if !available_columns.iter().any(|name| name == &column) {
                anyhow::bail!(
                    "Unknown filter column '{}.{}'. Inspect the live schema with postgres_schema_inspect and retry with a valid column name",
                    table,
                    column
                );
            }
        }
        for sort in &request.order_by {
            let column = normalize_db_column_name(&sort.column)?;
            if !available_columns.iter().any(|name| name == &column) {
                anyhow::bail!(
                    "Unknown sort column '{}.{}'. Inspect the live schema with postgres_schema_inspect and retry with a valid column name",
                    table,
                    column
                );
            }
        }

        let table_alias = "t";
        let mut json_object = Func::cust(Alias::new("jsonb_build_object"));
        for column in &selected_columns {
            json_object = json_object.arg(column.clone()).arg(Expr::col((
                Alias::new(table_alias),
                Alias::new(column.as_str()),
            )));
        }

        let mut query = Query::select();
        query
            .expr_as(json_object, Alias::new("row_json"))
            .from_as(Alias::new(table.as_str()), Alias::new(table_alias));
        for filter in &request.filters {
            query.and_where(Self::build_structured_db_filter_expr(table_alias, filter)?);
        }
        for sort in &request.order_by {
            let column = normalize_db_column_name(&sort.column)?;
            let direction = sort.direction.as_deref().unwrap_or("asc");
            query.order_by(
                (Alias::new(table_alias), Alias::new(column.as_str())),
                if direction.eq_ignore_ascii_case("desc") {
                    Order::Desc
                } else {
                    Order::Asc
                },
            );
        }
        let applied_limit = request.limit.unwrap_or(50).clamp(1, 200);
        query.limit(applied_limit);

        let rendered_sql = query.to_string(PostgresQueryBuilder);
        let statement = DbBackend::Postgres.build(&query);
        let rows = self.db.query_all(statement).await?;
        let mut json_rows = Vec::with_capacity(rows.len());
        for row in rows {
            if let Ok(value) = row.try_get::<serde_json::Value>("", "row_json") {
                json_rows.push(value);
                continue;
            }
            let fallback = row
                .try_get::<String>("", "row_json")
                .ok()
                .and_then(|value| serde_json::from_str::<serde_json::Value>(&value).ok())
                .ok_or_else(|| anyhow!("Failed to decode structured row JSON"))?;
            json_rows.push(fallback);
        }

        Ok(serde_json::json!({
            "generated_at": chrono::Utc::now().to_rfc3339(),
            "schema": "public",
            "table": table,
            "selected_columns": selected_columns,
            "filters": request.filters,
            "order_by": request.order_by,
            "applied_limit": applied_limit,
            "sql": rendered_sql,
            "row_count": json_rows.len(),
            "rows": json_rows,
        }))
    }

    pub async fn list_operational_log_version_metrics_by_event(
        &self,
        event_type: &str,
        limit: u64,
    ) -> Result<Vec<OperationalLogVersionMetricRow>> {
        operational_log::Entity::find()
            .select_only()
            .columns([
                operational_log::Column::Success,
                operational_log::Column::LatencyMs,
                operational_log::Column::PolicyVersion,
                operational_log::Column::StrategyVersion,
            ])
            .filter(operational_log::Column::EventType.eq(event_type.to_string()))
            .order_by_desc(operational_log::Column::CreatedAt)
            .limit(Self::db_limit(limit))
            .into_model::<OperationalLogVersionMetricRow>()
            .all(&self.db)
            .await
            .map_err(Into::into)
    }
}
