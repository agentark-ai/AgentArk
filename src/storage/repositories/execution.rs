use super::super::*;

impl Storage {
    pub(super) async fn existing_execution_trace_id(
        &self,
        trace_id: Option<&str>,
    ) -> Result<Option<String>> {
        let Some(trace_id) = trace_id.map(str::trim).filter(|value| !value.is_empty()) else {
            return Ok(None);
        };
        let exists = execution_trace::Entity::find_by_id(trace_id.to_string())
            .count(&self.db)
            .await?
            > 0;
        if exists {
            Ok(Some(trace_id.to_string()))
        } else {
            tracing::debug!(
                trace_id,
                "Skipping execution run trace_id link because the execution trace is not persisted yet"
            );
            Ok(None)
        }
    }

    #[allow(dead_code)]
    pub async fn insert_execution_run(&self, run: &crate::core::ExecutionRun) -> Result<()> {
        let degradation = encrypt_storage_string(&serde_json::to_string(&run.degradation)?)?;
        let last_error = encrypt_optional_storage_string(run.last_error.as_deref())?;
        let result_summary = encrypt_optional_storage_string(run.result_summary.as_deref())?;
        let request_message = encrypt_optional_storage_string(run.request_message.as_deref())?;
        let attempted_models =
            encrypt_storage_string(&serde_json::to_string(&run.attempted_models)?)?;
        let trace_id = self
            .existing_execution_trace_id(run.trace_id.as_deref())
            .await?;

        let insert_result = execution_run::Entity::insert(execution_run::ActiveModel {
            id: Set(run.id.clone()),
            kind: Set(run.kind.clone()),
            request_id: Set(run.request_id.clone()),
            status: Set(run.status.as_str().to_string()),
            current_stage: Set(run.current_stage.clone()),
            lease_owner: Set(run.lease_owner.clone()),
            lease_expires_at: Set(run.lease_expires_at.clone()),
            attempt: Set(run.attempt as i32),
            deadline_at: Set(run.deadline_at.clone()),
            cancellation_requested: Set(run.cancellation_requested),
            degradation: Set(degradation.clone()),
            last_error: Set(last_error.clone()),
            result_summary: Set(result_summary.clone()),
            trace_id: Set(trace_id.clone()),
            conversation_id: Set(run.conversation_id.clone()),
            channel: Set(run.channel.clone()),
            request_message: Set(request_message.clone()),
            attempted_models: Set(attempted_models.clone()),
            created_at: Set(run.created_at.clone()),
            updated_at: Set(run.updated_at.clone()),
        })
        .on_conflict(
            OnConflict::column(execution_run::Column::Id)
                .update_columns([
                    execution_run::Column::RequestId,
                    execution_run::Column::Status,
                    execution_run::Column::CurrentStage,
                    execution_run::Column::LeaseOwner,
                    execution_run::Column::LeaseExpiresAt,
                    execution_run::Column::Attempt,
                    execution_run::Column::DeadlineAt,
                    execution_run::Column::CancellationRequested,
                    execution_run::Column::Degradation,
                    execution_run::Column::LastError,
                    execution_run::Column::ResultSummary,
                    execution_run::Column::TraceId,
                    execution_run::Column::ConversationId,
                    execution_run::Column::Channel,
                    execution_run::Column::RequestMessage,
                    execution_run::Column::AttemptedModels,
                    execution_run::Column::UpdatedAt,
                ])
                .to_owned(),
        )
        .exec(&self.db)
        .await;
        if let Err(error) = insert_result {
            if trace_id.is_some() && is_foreign_key_constraint_error(&error) {
                tracing::warn!(
                    "Retrying execution run upsert '{}' without trace_id after FK failure: {}",
                    run.id,
                    error
                );
                execution_run::Entity::insert(execution_run::ActiveModel {
                    id: Set(run.id.clone()),
                    kind: Set(run.kind.clone()),
                    request_id: Set(run.request_id.clone()),
                    status: Set(run.status.as_str().to_string()),
                    current_stage: Set(run.current_stage.clone()),
                    lease_owner: Set(run.lease_owner.clone()),
                    lease_expires_at: Set(run.lease_expires_at.clone()),
                    attempt: Set(run.attempt as i32),
                    deadline_at: Set(run.deadline_at.clone()),
                    cancellation_requested: Set(run.cancellation_requested),
                    degradation: Set(degradation),
                    last_error: Set(last_error),
                    result_summary: Set(result_summary),
                    trace_id: Set(None),
                    conversation_id: Set(run.conversation_id.clone()),
                    channel: Set(run.channel.clone()),
                    request_message: Set(request_message),
                    attempted_models: Set(attempted_models),
                    created_at: Set(run.created_at.clone()),
                    updated_at: Set(run.updated_at.clone()),
                })
                .on_conflict(
                    OnConflict::column(execution_run::Column::Id)
                        .update_columns([
                            execution_run::Column::RequestId,
                            execution_run::Column::Status,
                            execution_run::Column::CurrentStage,
                            execution_run::Column::LeaseOwner,
                            execution_run::Column::LeaseExpiresAt,
                            execution_run::Column::Attempt,
                            execution_run::Column::DeadlineAt,
                            execution_run::Column::CancellationRequested,
                            execution_run::Column::Degradation,
                            execution_run::Column::LastError,
                            execution_run::Column::ResultSummary,
                            execution_run::Column::TraceId,
                            execution_run::Column::ConversationId,
                            execution_run::Column::Channel,
                            execution_run::Column::RequestMessage,
                            execution_run::Column::AttemptedModels,
                            execution_run::Column::UpdatedAt,
                        ])
                        .to_owned(),
                )
                .exec(&self.db)
                .await?;
            } else {
                return Err(error.into());
            }
        }
        Ok(())
    }

    pub async fn load_execution_run(&self, id: &str) -> Result<Option<crate::core::ExecutionRun>> {
        Ok(execution_run::Entity::find_by_id(id.to_string())
            .one(&self.db)
            .await?
            .map(model_to_execution_run))
    }

    #[allow(dead_code)]
    pub async fn load_execution_run_by_request_id(
        &self,
        request_id: &str,
    ) -> Result<Option<crate::core::ExecutionRun>> {
        Ok(execution_run::Entity::find()
            .filter(execution_run::Column::RequestId.eq(request_id.to_string()))
            .order_by_desc(execution_run::Column::UpdatedAt)
            .one(&self.db)
            .await?
            .map(model_to_execution_run))
    }

    pub async fn load_execution_run_by_trace_id(
        &self,
        trace_id: &str,
    ) -> Result<Option<crate::core::ExecutionRun>> {
        Ok(execution_run::Entity::find()
            .filter(execution_run::Column::TraceId.eq(trace_id.to_string()))
            .order_by_desc(execution_run::Column::UpdatedAt)
            .one(&self.db)
            .await?
            .map(model_to_execution_run))
    }

    pub async fn list_execution_runs_for_conversation(
        &self,
        conversation_id: &str,
        limit: u64,
    ) -> Result<Vec<crate::core::ExecutionRun>> {
        let capped_limit = limit.clamp(1, 50);
        Ok(execution_run::Entity::find()
            .filter(execution_run::Column::ConversationId.eq(conversation_id.to_string()))
            .order_by_desc(execution_run::Column::UpdatedAt)
            .limit(capped_limit)
            .all(&self.db)
            .await?
            .into_iter()
            .map(model_to_execution_run)
            .collect())
    }

    pub async fn list_recent_execution_runs(
        &self,
        limit: u64,
    ) -> Result<Vec<crate::core::ExecutionRun>> {
        let capped_limit = limit.clamp(1, 100);
        Ok(execution_run::Entity::find()
            .order_by_desc(execution_run::Column::UpdatedAt)
            .limit(capped_limit)
            .all(&self.db)
            .await?
            .into_iter()
            .map(model_to_execution_run)
            .collect())
    }

    pub async fn append_execution_checkpoint(
        &self,
        checkpoint: &crate::core::ExecutionCheckpoint,
    ) -> Result<()> {
        run_checkpoint::Entity::insert(run_checkpoint::ActiveModel {
            id: sea_orm::NotSet,
            run_id: Set(checkpoint.run_id.clone()),
            sequence_no: Set(checkpoint.sequence_no as i32),
            stage: Set(checkpoint.stage.clone()),
            payload: Set(encrypt_storage_string(&checkpoint.payload)?),
            created_at: Set(checkpoint.created_at.clone()),
        })
        .on_conflict(
            OnConflict::columns([
                run_checkpoint::Column::RunId,
                run_checkpoint::Column::SequenceNo,
            ])
            .update_columns([
                run_checkpoint::Column::Stage,
                run_checkpoint::Column::Payload,
                run_checkpoint::Column::CreatedAt,
            ])
            .to_owned(),
        )
        .exec(&self.db)
        .await?;
        Ok(())
    }

    pub async fn load_execution_checkpoints(
        &self,
        run_id: &str,
    ) -> Result<Vec<crate::core::ExecutionCheckpoint>> {
        Ok(run_checkpoint::Entity::find()
            .filter(run_checkpoint::Column::RunId.eq(run_id.to_string()))
            .order_by_asc(run_checkpoint::Column::SequenceNo)
            .all(&self.db)
            .await?
            .into_iter()
            .map(|model| crate::core::ExecutionCheckpoint {
                run_id: model.run_id,
                sequence_no: model.sequence_no.max(0) as u32,
                stage: model.stage,
                payload: decrypt_storage_string(&model.payload),
                created_at: model.created_at,
            })
            .collect())
    }

    #[allow(dead_code)]
    pub async fn append_tool_attempt(&self, attempt: &crate::core::ToolAttempt) -> Result<()> {
        tool_attempt::Entity::insert(tool_attempt::ActiveModel {
            id: Set(attempt.id.clone()),
            run_id: Set(attempt.run_id.clone()),
            sequence_no: Set(attempt.sequence_no as i32),
            tool_name: Set(attempt.tool_name.clone()),
            status: Set(attempt.status.as_str().to_string()),
            failure_class: Set(attempt.failure_class.as_ref().map(|value| {
                serde_json::to_string(value)
                    .unwrap_or_else(|_| "\"platform_error\"".to_string())
                    .trim_matches('"')
                    .to_string()
            })),
            retryable: Set(attempt.retryable),
            side_effect_level: Set(attempt.side_effect_level.clone()),
            idempotency_key: Set(attempt.idempotency_key.clone()),
            arguments_json: Set(encrypt_storage_string(&attempt.arguments_json)?),
            output_json: Set(encrypt_storage_string(&attempt.output_json)?),
            started_at: Set(attempt.started_at.clone()),
            completed_at: Set(attempt.completed_at.clone()),
            error_text: Set(encrypt_optional_storage_string(
                attempt.error_text.as_deref(),
            )?),
        })
        .on_conflict(
            OnConflict::column(tool_attempt::Column::Id)
                .update_columns([
                    tool_attempt::Column::Status,
                    tool_attempt::Column::FailureClass,
                    tool_attempt::Column::Retryable,
                    tool_attempt::Column::SideEffectLevel,
                    tool_attempt::Column::IdempotencyKey,
                    tool_attempt::Column::ArgumentsJson,
                    tool_attempt::Column::OutputJson,
                    tool_attempt::Column::StartedAt,
                    tool_attempt::Column::CompletedAt,
                    tool_attempt::Column::ErrorText,
                ])
                .to_owned(),
        )
        .exec(&self.db)
        .await?;
        Ok(())
    }
}
