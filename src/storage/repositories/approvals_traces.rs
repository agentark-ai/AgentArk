use super::super::*;

impl Storage {
    // ==================== Approval Log ====================

    /// Get approval log (paginated, newest first)
    pub async fn get_approval_log(
        &self,
        limit: u64,
        offset: u64,
    ) -> Result<Vec<approval_log::Model>> {
        let mut log = approval_log::Entity::find()
            .order_by_desc(approval_log::Column::RequestedAt)
            .limit(Self::db_limit(limit))
            .offset(Self::db_offset(offset))
            .all(&self.db)
            .await?;
        for row in &mut log {
            row.arguments = decrypt_storage_string(&row.arguments);
        }
        Ok(log)
    }

    /// Get a single approval request by id with decrypted arguments.
    pub async fn get_approval_request(&self, id: &str) -> Result<Option<approval_log::Model>> {
        let mut row = approval_log::Entity::find_by_id(id.to_string())
            .one(&self.db)
            .await?;
        if let Some(row) = &mut row {
            row.arguments = decrypt_storage_string(&row.arguments);
        }
        Ok(row)
    }

    /// Create or refresh a pending approval request entry.
    pub async fn upsert_approval_request(
        &self,
        id: &str,
        action_name: &str,
        arguments: &str,
        rule_name: &str,
        requested_at: &str,
    ) -> Result<()> {
        let arguments = encrypt_storage_string(arguments)?;
        approval_log::Entity::insert(approval_log::ActiveModel {
            id: Set(id.to_string()),
            action_name: Set(action_name.to_string()),
            arguments: Set(arguments),
            rule_name: Set(rule_name.to_string()),
            status: Set("pending".to_string()),
            requested_at: Set(requested_at.to_string()),
            resolved_at: Set(None),
            resolved_by: Set(None),
        })
        .on_conflict(
            OnConflict::column(approval_log::Column::Id)
                .update_columns([
                    approval_log::Column::ActionName,
                    approval_log::Column::Arguments,
                    approval_log::Column::RuleName,
                    approval_log::Column::Status,
                    approval_log::Column::RequestedAt,
                    approval_log::Column::ResolvedAt,
                    approval_log::Column::ResolvedBy,
                ])
                .to_owned(),
        )
        .exec(&self.db)
        .await?;
        Ok(())
    }

    /// Resolve an approval request entry.
    pub async fn resolve_approval_request(
        &self,
        id: &str,
        status: &str,
        resolved_by: &str,
    ) -> Result<()> {
        let existing = approval_log::Entity::find_by_id(id.to_string())
            .one(&self.db)
            .await?;
        if existing.is_none() {
            return Ok(());
        }
        approval_log::ActiveModel {
            id: Set(id.to_string()),
            status: Set(status.to_string()),
            resolved_at: Set(Some(chrono::Utc::now().to_rfc3339())),
            resolved_by: Set(Some(resolved_by.to_string())),
            ..Default::default()
        }
        .update(&self.db)
        .await?;
        Ok(())
    }

    // ==================== Execution Traces ====================

    #[allow(dead_code)]
    pub async fn insert_execution_proof(
        &self,
        proof: &crate::proofs::ExecutionProof,
    ) -> Result<()> {
        crate::storage::entities::execution_proof::ActiveModel {
            id: Set(proof.id.to_string()),
            action_hash: Set(proof.action_hash.clone()),
            input_hash: Set(proof.input_hash.clone()),
            output_hash: Set(proof.output_hash.clone()),
            prev_hash: Set(proof.prev_hash.clone()),
            timestamp: Set(proof.timestamp.to_rfc3339()),
            signature: Set(proof.signature.clone()),
        }
        .insert(&self.db)
        .await?;
        Ok(())
    }

    /// Persist a completed execution trace for Trace history/detail views.
    pub async fn insert_execution_trace(&self, trace: &crate::core::ExecutionTrace) -> Result<()> {
        let duration_ms = trace.started_at.and_then(|start| {
            trace
                .completed_at
                .map(|end| (end - start).num_milliseconds())
        });
        let started_at = trace.started_at.map(|value| value.to_rfc3339());
        let completed_at = trace.completed_at.map(|value| value.to_rfc3339());
        let created_at = trace
            .completed_at
            .or(trace.started_at)
            .unwrap_or_else(chrono::Utc::now)
            .to_rfc3339();
        let message = encrypt_storage_string(&trace.message)?;
        let steps_json = encrypt_storage_string(&serde_json::to_string(&trace.steps)?)?;
        let response = encrypt_optional_storage_string(trace.response.as_deref())?;
        let insert_result = crate::storage::entities::execution_trace::Entity::insert(
            crate::storage::entities::execution_trace::ActiveModel {
                id: Set(trace.id.clone()),
                message: Set(message.clone()),
                channel: Set(trace.channel.clone()),
                started_at: Set(started_at.clone()),
                completed_at: Set(completed_at.clone()),
                duration_ms: Set(duration_ms.map(|v| v.min(i32::MAX as i64) as i32)),
                step_count: Set(trace.steps.len().min(i32::MAX as usize) as i32),
                steps_json: Set(steps_json.clone()),
                response: Set(response.clone()),
                proof_id: Set(trace.proof_id.clone()),
                model: Set(trace.model.clone()),
                input_tokens: Set(trace.input_tokens.min(i32::MAX as i64) as i32),
                output_tokens: Set(trace.output_tokens.min(i32::MAX as i64) as i32),
                total_tokens: Set(trace.total_tokens.min(i32::MAX as i64) as i32),
                cost_usd: Set(trace.cost_usd),
                complexity: Set(trace.complexity.clone()),
                created_at: Set(created_at.clone()),
            },
        )
        .on_conflict(
            OnConflict::column(crate::storage::entities::execution_trace::Column::Id)
                .update_columns([
                    crate::storage::entities::execution_trace::Column::Message,
                    crate::storage::entities::execution_trace::Column::Channel,
                    crate::storage::entities::execution_trace::Column::StartedAt,
                    crate::storage::entities::execution_trace::Column::CompletedAt,
                    crate::storage::entities::execution_trace::Column::DurationMs,
                    crate::storage::entities::execution_trace::Column::StepCount,
                    crate::storage::entities::execution_trace::Column::StepsJson,
                    crate::storage::entities::execution_trace::Column::Response,
                    crate::storage::entities::execution_trace::Column::ProofId,
                    crate::storage::entities::execution_trace::Column::Model,
                    crate::storage::entities::execution_trace::Column::InputTokens,
                    crate::storage::entities::execution_trace::Column::OutputTokens,
                    crate::storage::entities::execution_trace::Column::TotalTokens,
                    crate::storage::entities::execution_trace::Column::CostUsd,
                    crate::storage::entities::execution_trace::Column::Complexity,
                ])
                .to_owned(),
        )
        .exec(&self.db)
        .await;
        if let Err(error) = insert_result {
            if trace.proof_id.is_some() && is_foreign_key_constraint_error(&error) {
                tracing::warn!(
                    "Retrying trace insert '{}' without proof_id after FK failure: {}",
                    trace.id,
                    error
                );
                crate::storage::entities::execution_trace::Entity::insert(
                    crate::storage::entities::execution_trace::ActiveModel {
                        id: Set(trace.id.clone()),
                        message: Set(message),
                        channel: Set(trace.channel.clone()),
                        started_at: Set(started_at),
                        completed_at: Set(completed_at),
                        duration_ms: Set(duration_ms.map(|v| v.min(i32::MAX as i64) as i32)),
                        step_count: Set(trace.steps.len().min(i32::MAX as usize) as i32),
                        steps_json: Set(steps_json),
                        response: Set(response),
                        proof_id: Set(None),
                        model: Set(trace.model.clone()),
                        input_tokens: Set(trace.input_tokens.min(i32::MAX as i64) as i32),
                        output_tokens: Set(trace.output_tokens.min(i32::MAX as i64) as i32),
                        total_tokens: Set(trace.total_tokens.min(i32::MAX as i64) as i32),
                        cost_usd: Set(trace.cost_usd),
                        complexity: Set(trace.complexity.clone()),
                        created_at: Set(created_at),
                    },
                )
                .on_conflict(
                    OnConflict::column(crate::storage::entities::execution_trace::Column::Id)
                        .update_columns([
                            crate::storage::entities::execution_trace::Column::Message,
                            crate::storage::entities::execution_trace::Column::Channel,
                            crate::storage::entities::execution_trace::Column::StartedAt,
                            crate::storage::entities::execution_trace::Column::CompletedAt,
                            crate::storage::entities::execution_trace::Column::DurationMs,
                            crate::storage::entities::execution_trace::Column::StepCount,
                            crate::storage::entities::execution_trace::Column::StepsJson,
                            crate::storage::entities::execution_trace::Column::Response,
                            crate::storage::entities::execution_trace::Column::ProofId,
                            crate::storage::entities::execution_trace::Column::Model,
                            crate::storage::entities::execution_trace::Column::InputTokens,
                            crate::storage::entities::execution_trace::Column::OutputTokens,
                            crate::storage::entities::execution_trace::Column::TotalTokens,
                            crate::storage::entities::execution_trace::Column::CostUsd,
                            crate::storage::entities::execution_trace::Column::Complexity,
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

    /// List persisted execution trace summaries (newest first) without loading full responses.
    pub async fn list_execution_trace_summaries(
        &self,
        since: Option<&str>,
        limit: u64,
        offset: u64,
    ) -> Result<Vec<ExecutionTraceSummaryRow>> {
        let mut query = crate::storage::entities::execution_trace::Entity::find()
            .select_only()
            .columns([
                crate::storage::entities::execution_trace::Column::Id,
                crate::storage::entities::execution_trace::Column::Message,
                crate::storage::entities::execution_trace::Column::Channel,
                crate::storage::entities::execution_trace::Column::StartedAt,
                crate::storage::entities::execution_trace::Column::CompletedAt,
                crate::storage::entities::execution_trace::Column::DurationMs,
                crate::storage::entities::execution_trace::Column::StepCount,
                crate::storage::entities::execution_trace::Column::StepsJson,
                crate::storage::entities::execution_trace::Column::Model,
                crate::storage::entities::execution_trace::Column::TotalTokens,
                crate::storage::entities::execution_trace::Column::CostUsd,
                crate::storage::entities::execution_trace::Column::Complexity,
                crate::storage::entities::execution_trace::Column::CreatedAt,
            ])
            .order_by_desc(crate::storage::entities::execution_trace::Column::CreatedAt)
            .limit(Self::db_limit(limit))
            .offset(Self::db_offset(offset));
        if let Some(since) = since.map(str::trim).filter(|value| !value.is_empty()) {
            query = query.filter(
                crate::storage::entities::execution_trace::Column::CreatedAt.gte(since.to_string()),
            );
        }

        let mut traces = query
            .into_model::<ExecutionTraceSummaryRow>()
            .all(&self.db)
            .await?;
        for trace in &mut traces {
            trace.message = decrypt_storage_string(&trace.message);
            trace.steps_json = decrypt_storage_string(&trace.steps_json);
        }
        Ok(traces)
    }

    pub async fn count_execution_traces(&self, since: Option<&str>) -> Result<u64> {
        let mut query = crate::storage::entities::execution_trace::Entity::find();
        if let Some(since) = since.map(str::trim).filter(|value| !value.is_empty()) {
            query = query.filter(
                crate::storage::entities::execution_trace::Column::CreatedAt.gte(since.to_string()),
            );
        }
        Ok(query.count(&self.db).await?)
    }

    pub async fn count_execution_traces_by_ids(
        &self,
        since: Option<&str>,
        ids: &[String],
    ) -> Result<u64> {
        if ids.is_empty() {
            return Ok(0);
        }
        let mut query = crate::storage::entities::execution_trace::Entity::find()
            .filter(crate::storage::entities::execution_trace::Column::Id.is_in(ids.to_vec()));
        if let Some(since) = since.map(str::trim).filter(|value| !value.is_empty()) {
            query = query.filter(
                crate::storage::entities::execution_trace::Column::CreatedAt.gte(since.to_string()),
            );
        }
        Ok(query.count(&self.db).await?)
    }

    pub async fn get_execution_trace_message_metrics_by_ids(
        &self,
        ids: &[String],
    ) -> Result<std::collections::HashMap<String, ExecutionTraceMessageMetrics>> {
        if ids.is_empty() {
            return Ok(std::collections::HashMap::new());
        }
        let mut rows = crate::storage::entities::execution_trace::Entity::find()
            .select_only()
            .columns([
                crate::storage::entities::execution_trace::Column::Id,
                crate::storage::entities::execution_trace::Column::DurationMs,
                crate::storage::entities::execution_trace::Column::InputTokens,
                crate::storage::entities::execution_trace::Column::OutputTokens,
                crate::storage::entities::execution_trace::Column::TotalTokens,
                crate::storage::entities::execution_trace::Column::StepsJson,
            ])
            .filter(crate::storage::entities::execution_trace::Column::Id.is_in(ids.to_vec()))
            .into_model::<ExecutionTraceMessageMetricRow>()
            .all(&self.db)
            .await?;
        for row in &mut rows {
            row.steps_json = decrypt_storage_string(&row.steps_json);
        }
        Ok(rows
            .into_iter()
            .map(|row| {
                let (cached_prompt_tokens, cache_creation_prompt_tokens) =
                    trace_prompt_cache_metrics(&row.steps_json);
                let metrics = ExecutionTraceMessageMetrics {
                    duration_ms: row.duration_ms.map(i64::from),
                    input_tokens: i64::from(row.input_tokens),
                    output_tokens: i64::from(row.output_tokens),
                    total_tokens: i64::from(row.total_tokens),
                    cached_prompt_tokens,
                    cache_creation_prompt_tokens,
                    time_to_first_token_ms: trace_time_to_first_token_ms(&row.steps_json),
                };
                (row.id, metrics)
            })
            .collect())
    }

    /// Get a single persisted execution trace by id.
    pub async fn get_execution_trace(
        &self,
        id: &str,
    ) -> Result<Option<crate::storage::entities::execution_trace::Model>> {
        let mut trace =
            crate::storage::entities::execution_trace::Entity::find_by_id(id.to_string())
                .one(&self.db)
                .await?;
        if let Some(row) = trace.as_mut() {
            row.message = decrypt_storage_string(&row.message);
            row.steps_json = decrypt_storage_string(&row.steps_json);
            row.response = decrypt_optional_storage_string(row.response.take());
        }
        Ok(trace)
    }
}
