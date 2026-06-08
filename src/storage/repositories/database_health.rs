use super::super::*;

impl Storage {
    pub async fn latest_migration_version(&self) -> Result<Option<i64>> {
        match self
            .db
            .query_one(Statement::from_string(
                DbBackend::Postgres,
                "SELECT MAX(version) AS version FROM schema_migrations".to_string(),
            ))
            .await
        {
            Ok(Some(row)) => Ok(row.try_get::<i64>("", "version").ok()),
            Ok(None) => Ok(None),
            Err(_) => Ok(None),
        }
    }

    pub fn expected_migration_version(&self) -> i64 {
        migrations::latest_version()
    }

    pub async fn database_table_names(&self) -> Result<Vec<String>> {
        let query = Query::select()
            .column((Alias::new("tables"), Alias::new("table_name")))
            .from((Alias::new("information_schema"), Alias::new("tables")))
            .and_where(Expr::col((Alias::new("tables"), Alias::new("table_schema"))).eq("public"))
            .and_where(Expr::col((Alias::new("tables"), Alias::new("table_type"))).eq("BASE TABLE"))
            .order_by((Alias::new("tables"), Alias::new("table_name")), Order::Asc)
            .to_owned();
        let rows = self.db.query_all(DbBackend::Postgres.build(&query)).await?;
        let mut table_names = Vec::with_capacity(rows.len());
        for row in rows {
            if let Ok(name) = row.try_get::<String>("", "table_name") {
                table_names.push(name);
            }
        }
        Ok(table_names)
    }

    pub async fn housekeeping_status(&self) -> Result<HousekeepingStatus> {
        let housekeeping_last_run_at = self
            .get(Self::HOUSEKEEPING_PURGE_LAST_RUN_KEY)
            .await?
            .and_then(|raw| String::from_utf8(raw).ok());
        let notification_last_run_at = self
            .get(Self::NOTIFICATION_PURGE_LAST_RUN_KEY)
            .await?
            .and_then(|raw| String::from_utf8(raw).ok());
        Ok(HousekeepingStatus {
            housekeeping_last_run_at,
            notification_last_run_at,
        })
    }

    pub async fn database_size_bytes(&self) -> Result<Option<i64>> {
        let query = Query::select()
            .expr_as(
                Func::cust(Alias::new("pg_database_size"))
                    .arg(Func::cust(Alias::new("current_database"))),
                Alias::new("size_bytes"),
            )
            .to_owned();
        let row = self.db.query_one(DbBackend::Postgres.build(&query)).await?;
        Ok(row.and_then(|value| value.try_get::<i64>("", "size_bytes").ok()))
    }

    pub async fn lease_status_summary(&self) -> Result<LeaseStatusSummary> {
        let now = chrono::Utc::now().to_rfc3339();
        let pending_status = serde_json::to_string(&crate::core::TaskStatus::Pending)
            .unwrap_or_else(|_| "\"pending\"".to_string());
        Ok(LeaseStatusSummary {
            pending_task_backlog: task::Entity::find()
                .filter(task::Column::Status.eq(pending_status.clone()))
                .filter(
                    Condition::any()
                        .add(task::Column::ScheduledFor.is_null())
                        .add(task::Column::ScheduledFor.lte(now.clone())),
                )
                .filter(
                    Condition::any()
                        .add(task::Column::LeaseExpiresAt.is_null())
                        .add(task::Column::LeaseExpiresAt.lte(now.clone())),
                )
                .count(&self.db)
                .await?,
            active_task_leases: task::Entity::find()
                .filter(task::Column::LeaseExpiresAt.is_not_null())
                .filter(task::Column::LeaseExpiresAt.gt(now.clone()))
                .count(&self.db)
                .await?,
            tasks_waiting_retry: task::Entity::find()
                .filter(task::Column::NextRetryAt.is_not_null())
                .filter(task::Column::NextRetryAt.gt(now.clone()))
                .count(&self.db)
                .await?,
            watcher_poll_backlog: watcher::Entity::find()
                .filter(watcher::Column::Status.eq("active"))
                .filter(
                    Condition::any()
                        .add(watcher::Column::NextRetryAt.is_null())
                        .add(watcher::Column::NextRetryAt.lte(now.clone())),
                )
                .filter(
                    Condition::any()
                        .add(watcher::Column::LeaseExpiresAt.is_null())
                        .add(watcher::Column::LeaseExpiresAt.lte(now.clone())),
                )
                .count(&self.db)
                .await?,
            active_watcher_leases: watcher::Entity::find()
                .filter(watcher::Column::LeaseExpiresAt.is_not_null())
                .filter(watcher::Column::LeaseExpiresAt.gt(now.clone()))
                .count(&self.db)
                .await?,
            watchers_waiting_retry: watcher::Entity::find()
                .filter(watcher::Column::NextRetryAt.is_not_null())
                .filter(watcher::Column::NextRetryAt.gt(now.clone()))
                .count(&self.db)
                .await?,
            active_run_leases: execution_run::Entity::find()
                .filter(execution_run::Column::LeaseExpiresAt.is_not_null())
                .filter(execution_run::Column::LeaseExpiresAt.gt(now.clone()))
                .count(&self.db)
                .await?,
            runs_pending_cancellation: execution_run::Entity::find()
                .filter(execution_run::Column::CancellationRequested.eq(true))
                .filter(execution_run::Column::Status.is_not_in([
                    "completed",
                    "degraded",
                    "needs_input",
                    "blocked",
                    "platform_failed",
                    "cancelled",
                ]))
                .count(&self.db)
                .await?,
        })
    }

    // ==================== Expenses ====================

    /// Insert an expense
    pub async fn insert_expense(&self, model: expense::Model) -> Result<()> {
        expense::ActiveModel {
            id: Set(model.id),
            amount: Set(model.amount),
            currency: Set(model.currency),
            category: Set(model.category),
            description: Set(model.description),
            date: Set(model.date),
            payment_method: Set(model.payment_method),
            vendor: Set(model.vendor),
            tags: Set(model.tags),
            split_with: Set(model.split_with),
            receipt_path: Set(model.receipt_path),
            created_at: Set(model.created_at),
        }
        .insert(&self.db)
        .await?;
        Ok(())
    }

    /// Get expenses with optional date range and category filter
    pub async fn get_expenses(
        &self,
        from_date: Option<&str>,
        to_date: Option<&str>,
        category: Option<&str>,
    ) -> Result<Vec<expense::Model>> {
        let mut query = expense::Entity::find();
        if let Some(from) = from_date {
            query = query.filter(expense::Column::Date.gte(from.to_string()));
        }
        if let Some(to) = to_date {
            query = query.filter(expense::Column::Date.lte(to.to_string()));
        }
        if let Some(cat) = category {
            query = query.filter(expense::Column::Category.eq(cat.to_string()));
        }
        let results = query
            .order_by_desc(expense::Column::Date)
            .limit(Self::MAX_EXPENSE_ROWS_PER_QUERY)
            .all(&self.db)
            .await?;
        Ok(results)
    }

    /// Get expense summary grouped by category
    pub async fn get_expense_summary(
        &self,
        from_date: Option<&str>,
        to_date: Option<&str>,
    ) -> Result<Vec<expense::Model>> {
        // Return all expenses in range; caller aggregates
        self.get_expenses(from_date, to_date, None).await
    }

    /// Delete an expense
    pub async fn delete_expense(&self, id: &str) -> Result<bool> {
        let result = expense::Entity::delete_by_id(id.to_string())
            .exec(&self.db)
            .await?;
        Ok(result.rows_affected > 0)
    }
}
