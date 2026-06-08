use super::super::*;

impl Storage {
    pub async fn list_webhook_sources(&self) -> Result<Vec<webhook_source::Model>> {
        Ok(webhook_source::Entity::find()
            .order_by_desc(webhook_source::Column::UpdatedAt)
            .all(&self.db)
            .await?)
    }

    pub async fn get_webhook_source(&self, id: &str) -> Result<Option<webhook_source::Model>> {
        Ok(webhook_source::Entity::find_by_id(id.to_string())
            .one(&self.db)
            .await?)
    }

    pub async fn upsert_webhook_source(&self, source: webhook_source::Model) -> Result<()> {
        webhook_source::Entity::insert(Self::webhook_source_active_model(source))
            .on_conflict(
                OnConflict::column(webhook_source::Column::Id)
                    .update_columns([
                        webhook_source::Column::Name,
                        webhook_source::Column::Provider,
                        webhook_source::Column::Description,
                        webhook_source::Column::Enabled,
                        webhook_source::Column::AuthMode,
                        webhook_source::Column::MatchMode,
                        webhook_source::Column::Instruction,
                        webhook_source::Column::EventHeader,
                        webhook_source::Column::SecretHeader,
                        webhook_source::Column::SignatureTimestampHeader,
                        webhook_source::Column::SignatureTimestampToleranceSecs,
                        webhook_source::Column::SignaturePayloadMode,
                        webhook_source::Column::AllowDuplicate,
                        webhook_source::Column::RequireApproval,
                        webhook_source::Column::DedupeWindowSecs,
                        webhook_source::Column::NotifyOnQueued,
                        webhook_source::Column::NotifyOnSuccess,
                        webhook_source::Column::NotifyOnFailure,
                        webhook_source::Column::OutputTarget,
                        webhook_source::Column::OutputChannel,
                        webhook_source::Column::ConversationId,
                        webhook_source::Column::UpdatedAt,
                        webhook_source::Column::LastReceivedAt,
                        webhook_source::Column::LastOutcome,
                        webhook_source::Column::LastTaskId,
                    ])
                    .to_owned(),
            )
            .exec(&self.db)
            .await?;
        Ok(())
    }

    pub async fn delete_webhook_source(&self, id: &str) -> Result<bool> {
        let result = webhook_source::Entity::delete_by_id(id.to_string())
            .exec(&self.db)
            .await?;
        Ok(result.rows_affected > 0)
    }

    pub async fn update_webhook_source_runtime_state(
        &self,
        id: &str,
        received_at: &str,
        outcome: &str,
        task_id: Option<&str>,
    ) -> Result<bool> {
        let result = webhook_source::Entity::update_many()
            .col_expr(
                webhook_source::Column::UpdatedAt,
                Expr::value(received_at.to_string()),
            )
            .col_expr(
                webhook_source::Column::LastReceivedAt,
                Expr::value(Some(received_at.to_string())),
            )
            .col_expr(
                webhook_source::Column::LastOutcome,
                Expr::value(Some(outcome.to_string())),
            )
            .col_expr(
                webhook_source::Column::LastTaskId,
                Expr::value(task_id.map(|value| value.to_string())),
            )
            .filter(webhook_source::Column::Id.eq(id.to_string()))
            .exec(&self.db)
            .await?;
        Ok(result.rows_affected > 0)
    }

    #[cfg(test)]
    pub async fn insert_webhook_event_once(&self, event: webhook_event::Model) -> Result<bool> {
        let result = webhook_event::Entity::insert(Self::webhook_event_active_model(event))
            .on_conflict(
                OnConflict::columns([
                    webhook_event::Column::SourceId,
                    webhook_event::Column::IdempotencyKey,
                ])
                .do_nothing()
                .to_owned(),
            )
            .exec(&self.db)
            .await;
        match result {
            Ok(_) => Ok(true),
            Err(sea_orm::DbErr::RecordNotInserted) => Ok(false),
            Err(error) => Err(error.into()),
        }
    }

    pub async fn reserve_webhook_event_once(
        &self,
        event: webhook_event::Model,
        dedupe_window_secs: u64,
    ) -> Result<bool> {
        let txn = self.db.begin().await?;
        txn.execute(Statement::from_sql_and_values(
            DbBackend::Postgres,
            "SELECT pg_advisory_xact_lock($1)",
            vec![webhook_event_lock_key(&event.source_id, &event.dedupe_key).into()],
        ))
        .await?;
        let cutoff = (chrono::Utc::now()
            - chrono::Duration::seconds(dedupe_window_secs.max(1) as i64))
        .to_rfc3339();
        let duplicate = webhook_event::Entity::find()
            .filter(webhook_event::Column::SourceId.eq(event.source_id.clone()))
            .filter(webhook_event::Column::DedupeKey.eq(event.dedupe_key.clone()))
            .filter(webhook_event::Column::ReceivedAt.gte(cutoff))
            .one(&txn)
            .await?
            .is_some();
        if duplicate {
            txn.commit().await?;
            return Ok(false);
        }
        let insert_result = webhook_event::Entity::insert(Self::webhook_event_active_model(event))
            .on_conflict(
                OnConflict::columns([
                    webhook_event::Column::SourceId,
                    webhook_event::Column::IdempotencyKey,
                ])
                .do_nothing()
                .to_owned(),
            )
            .exec(&txn)
            .await;
        match insert_result {
            Ok(_) => {
                txn.commit().await?;
                Ok(true)
            }
            Err(sea_orm::DbErr::RecordNotInserted) => {
                txn.commit().await?;
                Ok(false)
            }
            Err(error) => {
                let _ = txn.rollback().await;
                Err(error.into())
            }
        }
    }

    pub async fn insert_webhook_event(&self, event: webhook_event::Model) -> Result<()> {
        webhook_event::Entity::insert(Self::webhook_event_active_model(event))
            .exec(&self.db)
            .await?;
        Ok(())
    }

    pub async fn update_webhook_event_outcome(
        &self,
        id: &str,
        outcome: &str,
        matched: bool,
        queued: bool,
        task_id: Option<&str>,
        message: Option<&str>,
    ) -> Result<()> {
        webhook_event::Entity::update_many()
            .col_expr(
                webhook_event::Column::UpdatedAt,
                Expr::value(chrono::Utc::now().to_rfc3339()),
            )
            .col_expr(
                webhook_event::Column::Outcome,
                Expr::value(outcome.to_string()),
            )
            .col_expr(webhook_event::Column::Matched, Expr::value(matched))
            .col_expr(webhook_event::Column::Queued, Expr::value(queued))
            .col_expr(
                webhook_event::Column::TaskId,
                Expr::value(task_id.map(|value| value.to_string())),
            )
            .col_expr(
                webhook_event::Column::Message,
                Expr::value(message.map(|value| value.to_string())),
            )
            .filter(webhook_event::Column::Id.eq(id))
            .exec(&self.db)
            .await?;
        Ok(())
    }

    pub async fn list_webhook_events(
        &self,
        source_id: Option<&str>,
        limit: u64,
    ) -> Result<Vec<webhook_event::Model>> {
        let mut query = webhook_event::Entity::find()
            .order_by_desc(webhook_event::Column::ReceivedAt)
            .limit(Self::db_limit(limit));
        if let Some(source_id) = source_id.map(str::trim).filter(|value| !value.is_empty()) {
            query = query.filter(webhook_event::Column::SourceId.eq(source_id.to_string()));
        }
        Ok(query.all(&self.db).await?)
    }

    pub(super) fn webhook_source_active_model(
        source: webhook_source::Model,
    ) -> webhook_source::ActiveModel {
        webhook_source::ActiveModel {
            id: Set(source.id),
            name: Set(source.name),
            provider: Set(source.provider),
            description: Set(source.description),
            enabled: Set(source.enabled),
            auth_mode: Set(source.auth_mode),
            match_mode: Set(source.match_mode),
            instruction: Set(source.instruction),
            event_header: Set(source.event_header),
            secret_header: Set(source.secret_header),
            signature_timestamp_header: Set(source.signature_timestamp_header),
            signature_timestamp_tolerance_secs: Set(source.signature_timestamp_tolerance_secs),
            signature_payload_mode: Set(source.signature_payload_mode),
            allow_duplicate: Set(source.allow_duplicate),
            require_approval: Set(source.require_approval),
            dedupe_window_secs: Set(source.dedupe_window_secs),
            notify_on_queued: Set(source.notify_on_queued),
            notify_on_success: Set(source.notify_on_success),
            notify_on_failure: Set(source.notify_on_failure),
            output_target: Set(source.output_target),
            output_channel: Set(source.output_channel),
            conversation_id: Set(source.conversation_id),
            created_at: Set(source.created_at),
            updated_at: Set(source.updated_at),
            last_received_at: Set(source.last_received_at),
            last_outcome: Set(source.last_outcome),
            last_task_id: Set(source.last_task_id),
        }
    }

    pub(super) fn webhook_event_active_model(
        event: webhook_event::Model,
    ) -> webhook_event::ActiveModel {
        webhook_event::ActiveModel {
            id: Set(event.id),
            source_id: Set(event.source_id),
            source_name: Set(event.source_name),
            provider: Set(event.provider),
            received_at: Set(event.received_at),
            updated_at: Set(event.updated_at),
            event_type: Set(event.event_type),
            status: Set(event.status),
            subject: Set(event.subject),
            outcome: Set(event.outcome),
            matched: Set(event.matched),
            queued: Set(event.queued),
            message: Set(event.message),
            event_id: Set(event.event_id),
            dedupe_key: Set(event.dedupe_key),
            idempotency_key: Set(event.idempotency_key),
            payload_hash: Set(event.payload_hash),
            event_url: Set(event.event_url),
            payload_excerpt: Set(event.payload_excerpt),
            task_id: Set(event.task_id),
            conversation_id: Set(event.conversation_id),
            severity: Set(event.severity),
            test_event: Set(event.test_event),
        }
    }
}
