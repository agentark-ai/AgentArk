use super::super::*;

impl Storage {
    // ==================== Experience Graph ====================

    pub async fn upsert_experience_run(&self, run: &experience_run::Model) -> Result<()> {
        let txn = self.db.begin().await?;
        experience_run::Entity::insert(experience_run::ActiveModel {
            id: Set(run.id.clone()),
            execution_run_id: Set(run.execution_run_id.clone()),
            trace_id: Set(run.trace_id.clone()),
            conversation_id: Set(run.conversation_id.clone()),
            project_id: Set(run.project_id.clone()),
            channel: Set(run.channel.clone()),
            scope: Set(run.scope.clone()),
            intent_key: Set(run.intent_key.clone()),
            task_type: Set(run.task_type.clone()),
            request_text: Set(run.request_text.clone()),
            tool_sequence_digest: Set(run.tool_sequence_digest.clone()),
            tool_sequence_json: Set(run.tool_sequence_json.clone()),
            strategy_version: Set(run.strategy_version.clone()),
            policy_version: Set(run.policy_version.clone()),
            prompt_version: Set(run.prompt_version.clone()),
            model_slot: Set(run.model_slot.clone()),
            success_state: Set(run.success_state.clone()),
            correction_state: Set(run.correction_state.clone()),
            outcome_summary: Set(run.outcome_summary.clone()),
            failure_reason: Set(run.failure_reason.clone()),
            metadata: Set(run.metadata.clone()),
            consolidated: Set(run.consolidated),
            accepted_at: Set(run.accepted_at.clone()),
            corrected_at: Set(run.corrected_at.clone()),
            heuristic_reflected: Set(run.heuristic_reflected),
            heuristic_reflection_status: Set(run.heuristic_reflection_status.clone()),
            heuristic_reflection_attempted_at: Set(run.heuristic_reflection_attempted_at.clone()),
            heuristic_reflection_completed_at: Set(run.heuristic_reflection_completed_at.clone()),
            heuristic_lesson_id: Set(run.heuristic_lesson_id.clone()),
            heuristic_reflection_error: Set(run.heuristic_reflection_error.clone()),
            created_at: Set(run.created_at.clone()),
            updated_at: Set(run.updated_at.clone()),
        })
        .on_conflict(
            OnConflict::column(experience_run::Column::Id)
                .update_columns(EXPERIENCE_RUN_LIGHT_UPSERT_COLUMNS.iter().copied())
                .to_owned(),
        )
        .exec(&txn)
        .await?;
        let current = experience_run::Entity::find_by_id(run.id.clone())
            .lock_exclusive()
            .one(&txn)
            .await?
            .ok_or_else(|| anyhow!("Experience run '{}' missing after upsert", run.id))?;
        if let Some(model) = Self::experience_run_heavy_update_active_model(&current, run) {
            model.update(&txn).await?;
        }
        txn.commit().await?;
        Ok(())
    }

    pub async fn list_tool_attempts_for_run(
        &self,
        run_id: &str,
    ) -> Result<Vec<crate::core::ToolAttempt>> {
        Ok(tool_attempt::Entity::find()
            .filter(tool_attempt::Column::RunId.eq(run_id.to_string()))
            .order_by_asc(tool_attempt::Column::SequenceNo)
            .all(&self.db)
            .await?
            .into_iter()
            .map(model_to_tool_attempt)
            .collect())
    }

    pub async fn append_readiness_evaluation(
        &self,
        evaluation: &readiness_evaluation::Model,
    ) -> Result<()> {
        readiness_evaluation::Entity::insert(readiness_evaluation::ActiveModel {
            id: Set(evaluation.id.clone()),
            target_type: Set(evaluation.target_type.clone()),
            target_id: Set(evaluation.target_id.clone()),
            score: Set(evaluation.score),
            stage: Set(evaluation.stage.clone()),
            allows_review: Set(evaluation.allows_review),
            allows_auto: Set(evaluation.allows_auto),
            reasons_json: Set(evaluation.reasons_json.clone()),
            blockers_json: Set(evaluation.blockers_json.clone()),
            signals_json: Set(evaluation.signals_json.clone()),
            policy_version: Set(evaluation.policy_version.clone()),
            created_at: Set(evaluation.created_at.clone()),
        })
        .exec(&self.db)
        .await?;
        Ok(())
    }

    #[allow(dead_code)]
    pub async fn mark_latest_provisional_experience_run_corrected(
        &self,
        conversation_id: &str,
        correction_signal: &str,
        within_minutes: i64,
    ) -> Result<Option<experience_run::Model>> {
        let now = chrono::Utc::now().to_rfc3339();
        let cutoff =
            (chrono::Utc::now() - chrono::Duration::minutes(within_minutes.max(1))).to_rfc3339();
        let payload = serde_json::json!({
            "correction_signal": correction_signal,
            "correction_recorded_at": now,
        });
        let candidates = experience_run::Entity::find()
            .filter(experience_run::Column::ConversationId.eq(conversation_id.to_string()))
            .filter(experience_run::Column::SuccessState.eq("provisional"))
            .filter(experience_run::Column::CorrectionState.eq("none"))
            .filter(experience_run::Column::CreatedAt.gte(cutoff))
            .order_by_desc(experience_run::Column::CreatedAt)
            .limit(2)
            .all(&self.db)
            .await?;
        if candidates.len() != 1 {
            return Ok(None);
        }
        let target = candidates
            .into_iter()
            .next()
            .expect("exactly one correction candidate");

        let mut metadata = target.metadata.clone();
        if let Some(existing) = metadata.as_object_mut() {
            if let Some(payload_map) = payload.as_object() {
                for (key, value) in payload_map {
                    existing.insert(key.clone(), value.clone());
                }
            }
        } else {
            metadata = payload;
        }

        let updated = experience_run::ActiveModel {
            id: Unchanged(target.id),
            success_state: Set(if target.success_state == "provisional" {
                "failed".to_string()
            } else {
                target.success_state
            }),
            correction_state: Set("corrected".to_string()),
            corrected_at: Set(Some(now.clone())),
            updated_at: Set(now),
            metadata: Set(metadata),
            ..Default::default()
        }
        .update(&self.db)
        .await?;
        Ok(Some(updated))
    }

    #[allow(dead_code)]
    pub async fn mark_provisional_experience_run_corrected_by_trace_id(
        &self,
        trace_id: &str,
        correction_signal: &str,
    ) -> Result<Option<experience_run::Model>> {
        let now = chrono::Utc::now().to_rfc3339();
        let payload = serde_json::json!({
            "correction_signal": correction_signal,
            "correction_recorded_at": now,
            "correction_bound_by": "trace_id",
        });
        let candidates = experience_run::Entity::find()
            .filter(experience_run::Column::TraceId.eq(trace_id.to_string()))
            .filter(experience_run::Column::SuccessState.eq("provisional"))
            .filter(experience_run::Column::CorrectionState.eq("none"))
            .limit(2)
            .all(&self.db)
            .await?;
        if candidates.len() != 1 {
            return Ok(None);
        }
        let target = candidates
            .into_iter()
            .next()
            .expect("exactly one trace-bound correction candidate");

        let mut metadata = target.metadata.clone();
        if let Some(existing) = metadata.as_object_mut() {
            if let Some(payload_map) = payload.as_object() {
                for (key, value) in payload_map {
                    existing.insert(key.clone(), value.clone());
                }
            }
        } else {
            metadata = payload;
        }

        let updated = experience_run::ActiveModel {
            id: Unchanged(target.id),
            success_state: Set(if target.success_state == "provisional" {
                "failed".to_string()
            } else {
                target.success_state
            }),
            correction_state: Set("corrected".to_string()),
            corrected_at: Set(Some(now.clone())),
            updated_at: Set(now),
            metadata: Set(metadata),
            ..Default::default()
        }
        .update(&self.db)
        .await?;
        Ok(Some(updated))
    }

    pub async fn finalize_stale_provisional_experience_runs(
        &self,
        older_than_minutes: i64,
        limit: u64,
    ) -> Result<u64> {
        let cutoff = (chrono::Utc::now() - chrono::Duration::minutes(older_than_minutes.max(1)))
            .to_rfc3339();
        let now = chrono::Utc::now().to_rfc3339();
        let target_ids = experience_run::Entity::find()
            .select_only()
            .column(experience_run::Column::Id)
            .filter(experience_run::Column::SuccessState.eq("provisional"))
            .filter(experience_run::Column::CorrectionState.eq("none"))
            .filter(experience_run::Column::CreatedAt.lt(cutoff))
            .order_by_asc(experience_run::Column::CreatedAt)
            .limit(Self::db_limit(
                limit.min(Self::MAX_EXPERIENCE_RUN_ROWS_PER_QUERY),
            ))
            .into_tuple::<String>()
            .all(&self.db)
            .await?;
        if target_ids.is_empty() {
            return Ok(0);
        }
        let result = experience_run::Entity::update_many()
            .col_expr(
                experience_run::Column::SuccessState,
                Expr::value("accepted".to_string()),
            )
            .col_expr(
                experience_run::Column::AcceptedAt,
                Expr::value(Some(now.clone())),
            )
            .col_expr(experience_run::Column::UpdatedAt, Expr::value(now))
            .filter(experience_run::Column::Id.is_in(target_ids))
            .exec(&self.db)
            .await?;
        Ok(result.rows_affected)
    }

    pub async fn list_experience_runs_for_consolidation(
        &self,
        limit: u64,
    ) -> Result<Vec<experience_run::Model>> {
        Ok(experience_run::Entity::find()
            .filter(experience_run::Column::Consolidated.eq(false))
            .filter(
                Condition::any()
                    .add(experience_run::Column::SuccessState.ne("provisional"))
                    .add(experience_run::Column::CorrectionState.eq("corrected")),
            )
            .order_by_asc(experience_run::Column::CreatedAt)
            .limit(Self::db_limit(limit))
            .all(&self.db)
            .await?)
    }

    pub async fn list_recent_experience_runs_any_scope(
        &self,
        limit: u64,
    ) -> Result<Vec<experience_run::Model>> {
        let capped_limit = limit.min(Self::MAX_EXPERIENCE_RUN_ROWS_PER_QUERY);
        experience_run::Entity::find()
            .order_by_desc(experience_run::Column::UpdatedAt)
            .limit(Self::db_limit(capped_limit))
            .all(&self.db)
            .await
            .map_err(Into::into)
    }

    pub async fn get_experience_run(&self, id: &str) -> Result<Option<experience_run::Model>> {
        Ok(experience_run::Entity::find_by_id(id.to_string())
            .one(&self.db)
            .await?)
    }

    pub async fn list_experience_runs_for_heuristic_reflection(
        &self,
        limit: u64,
    ) -> Result<Vec<experience_run::Model>> {
        Ok(experience_run::Entity::find()
            .filter(experience_run::Column::Consolidated.eq(true))
            .filter(experience_run::Column::HeuristicReflected.eq(false))
            .filter(
                Condition::any()
                    .add(experience_run::Column::HeuristicReflectionStatus.is_null())
                    .add(experience_run::Column::HeuristicReflectionStatus.eq("pending")),
            )
            .order_by_asc(experience_run::Column::UpdatedAt)
            .limit(Self::db_limit(
                limit.min(Self::MAX_EXPERIENCE_RUN_ROWS_PER_QUERY),
            ))
            .all(&self.db)
            .await?)
    }

    pub async fn mark_experience_run_consolidated(&self, id: &str) -> Result<()> {
        experience_run::Entity::update_many()
            .col_expr(experience_run::Column::Consolidated, Expr::value(true))
            .col_expr(
                experience_run::Column::UpdatedAt,
                Expr::value(chrono::Utc::now().to_rfc3339()),
            )
            .filter(experience_run::Column::Id.eq(id))
            .exec(&self.db)
            .await?;
        Ok(())
    }

    pub async fn mark_experience_run_heuristic_reflection_started(&self, id: &str) -> Result<()> {
        let now = chrono::Utc::now().to_rfc3339();
        experience_run::Entity::update_many()
            .col_expr(
                experience_run::Column::HeuristicReflectionStatus,
                Expr::value(Option::<String>::Some("pending".to_string())),
            )
            .col_expr(
                experience_run::Column::HeuristicReflectionAttemptedAt,
                Expr::value(Option::<String>::Some(now.clone())),
            )
            .col_expr(experience_run::Column::UpdatedAt, Expr::value(now))
            .filter(experience_run::Column::Id.eq(id))
            .exec(&self.db)
            .await?;
        Ok(())
    }

    pub async fn mark_experience_run_heuristic_reflection_completed(
        &self,
        id: &str,
        lesson_id: &str,
    ) -> Result<()> {
        let now = chrono::Utc::now().to_rfc3339();
        experience_run::Entity::update_many()
            .col_expr(
                experience_run::Column::HeuristicReflected,
                Expr::value(true),
            )
            .col_expr(
                experience_run::Column::HeuristicReflectionStatus,
                Expr::value(Option::<String>::Some("completed".to_string())),
            )
            .col_expr(
                experience_run::Column::HeuristicReflectionCompletedAt,
                Expr::value(Option::<String>::Some(now.clone())),
            )
            .col_expr(
                experience_run::Column::HeuristicLessonId,
                Expr::value(Option::<String>::Some(lesson_id.to_string())),
            )
            .col_expr(
                experience_run::Column::HeuristicReflectionError,
                Expr::value(Option::<String>::None),
            )
            .col_expr(experience_run::Column::UpdatedAt, Expr::value(now))
            .filter(experience_run::Column::Id.eq(id))
            .exec(&self.db)
            .await?;
        Ok(())
    }

    pub async fn mark_experience_run_heuristic_reflection_skipped(
        &self,
        id: &str,
        reason: &str,
    ) -> Result<()> {
        let now = chrono::Utc::now().to_rfc3339();
        experience_run::Entity::update_many()
            .col_expr(
                experience_run::Column::HeuristicReflected,
                Expr::value(true),
            )
            .col_expr(
                experience_run::Column::HeuristicReflectionStatus,
                Expr::value(Option::<String>::Some("skipped".to_string())),
            )
            .col_expr(
                experience_run::Column::HeuristicReflectionCompletedAt,
                Expr::value(Option::<String>::Some(now.clone())),
            )
            .col_expr(
                experience_run::Column::HeuristicReflectionError,
                Expr::value(Option::<String>::Some(reason.to_string())),
            )
            .col_expr(experience_run::Column::UpdatedAt, Expr::value(now))
            .filter(experience_run::Column::Id.eq(id))
            .exec(&self.db)
            .await?;
        Ok(())
    }

    pub async fn mark_experience_run_heuristic_reflection_failed(
        &self,
        id: &str,
        error: &str,
    ) -> Result<()> {
        let now = chrono::Utc::now().to_rfc3339();
        experience_run::Entity::update_many()
            .col_expr(
                experience_run::Column::HeuristicReflected,
                Expr::value(false),
            )
            .col_expr(
                experience_run::Column::HeuristicReflectionStatus,
                Expr::value(Option::<String>::Some("failed".to_string())),
            )
            .col_expr(
                experience_run::Column::HeuristicReflectionError,
                Expr::value(Option::<String>::Some(error.to_string())),
            )
            .col_expr(experience_run::Column::UpdatedAt, Expr::value(now))
            .filter(experience_run::Column::Id.eq(id))
            .exec(&self.db)
            .await?;
        Ok(())
    }
}
