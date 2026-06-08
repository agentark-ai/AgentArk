use super::super::*;

impl Storage {
    pub(super) fn experience_item_active_model(
        item: &experience_item::Model,
    ) -> experience_item::ActiveModel {
        experience_item::ActiveModel {
            id: Set(item.id.clone()),
            kind: Set(item.kind.clone()),
            scope: Set(item.scope.clone()),
            project_id: Set(item.project_id.clone()),
            conversation_id: Set(item.conversation_id.clone()),
            title: Set(item.title.clone()),
            content: Set(item.content.clone()),
            normalized_key: Set(item.normalized_key.clone()),
            confidence: Set(item.confidence),
            support_count: Set(item.support_count),
            contradiction_count: Set(item.contradiction_count),
            status: Set(item.status.clone()),
            metadata: Set(item.metadata.clone()),
            last_supported_at: Set(item.last_supported_at.clone()),
            last_contradicted_at: Set(item.last_contradicted_at.clone()),
            created_at: Set(item.created_at.clone()),
            updated_at: Set(item.updated_at.clone()),
            embedding: Set(item.embedding.clone()),
        }
    }

    pub(super) fn semantic_work_unit_active_model(
        unit: &semantic_work_unit::Model,
    ) -> Result<semantic_work_unit::ActiveModel> {
        Ok(semantic_work_unit::ActiveModel {
            id: Set(unit.id.clone()),
            source_kind: Set(unit.source_kind.clone()),
            source_id: Set(unit.source_id.clone()),
            conversation_id: Set(unit.conversation_id.clone()),
            project_id: Set(unit.project_id.clone()),
            channel: Set(unit.channel.clone()),
            title: Set(encrypt_storage_string(&unit.title)?),
            summary: Set(encrypt_storage_string(&unit.summary)?),
            content_preview: Set(encrypt_storage_string(&unit.content_preview)?),
            text_hash: Set(unit.text_hash.clone()),
            occurred_at: Set(unit.occurred_at.clone()),
            period_start: Set(unit.period_start.clone()),
            period_end: Set(unit.period_end.clone()),
            message_count: Set(unit.message_count),
            metadata: Set(unit.metadata.clone()),
            created_at: Set(unit.created_at.clone()),
            updated_at: Set(unit.updated_at.clone()),
            embedding: Set(unit.embedding.clone()),
        })
    }

    pub(super) fn decrypt_semantic_work_unit(
        mut unit: semantic_work_unit::Model,
    ) -> semantic_work_unit::Model {
        unit.title = decrypt_storage_string(&unit.title);
        unit.summary = decrypt_storage_string(&unit.summary);
        unit.content_preview = decrypt_storage_string(&unit.content_preview);
        unit
    }

    pub(super) fn recall_event_active_model(
        event: &recall_event::Model,
    ) -> recall_event::ActiveModel {
        recall_event::ActiveModel {
            id: Set(event.id.clone()),
            event_type: Set(event.event_type.clone()),
            memory_id: Set(event.memory_id.clone()),
            related_memory_id: Set(event.related_memory_id.clone()),
            scope: Set(event.scope.clone()),
            project_id: Set(event.project_id.clone()),
            conversation_id: Set(event.conversation_id.clone()),
            source_kind: Set(event.source_kind.clone()),
            source_ref: Set(event.source_ref.clone()),
            actor: Set(event.actor.clone()),
            summary: Set(event.summary.clone()),
            old_snapshot: Set(event.old_snapshot.clone()),
            new_snapshot: Set(event.new_snapshot.clone()),
            metadata: Set(event.metadata.clone()),
            risk_level: Set(event.risk_level.clone()),
            confidence: Set(event.confidence),
            reversible: Set(event.reversible),
            reverted_at: Set(event.reverted_at.clone()),
            created_at: Set(event.created_at.clone()),
            updated_at: Set(event.updated_at.clone()),
        }
    }

    pub(super) fn recall_test_active_model(test: &recall_test::Model) -> recall_test::ActiveModel {
        recall_test::ActiveModel {
            id: Set(test.id.clone()),
            memory_id: Set(test.memory_id.clone()),
            scope: Set(test.scope.clone()),
            project_id: Set(test.project_id.clone()),
            conversation_id: Set(test.conversation_id.clone()),
            prompt: Set(test.prompt.clone()),
            expected_answer: Set(test.expected_answer.clone()),
            status: Set(test.status.clone()),
            last_answer: Set(test.last_answer.clone()),
            last_run_at: Set(test.last_run_at.clone()),
            metadata: Set(test.metadata.clone()),
            created_at: Set(test.created_at.clone()),
            updated_at: Set(test.updated_at.clone()),
        }
    }

    pub(super) fn memory_capture_event_active_model(
        event: &memory_capture_event::Model,
    ) -> memory_capture_event::ActiveModel {
        memory_capture_event::ActiveModel {
            id: Set(event.id.clone()),
            source_message_id: Set(event.source_message_id.clone()),
            conversation_id: Set(event.conversation_id.clone()),
            project_id: Set(event.project_id.clone()),
            channel: Set(event.channel.clone()),
            status: Set(event.status.clone()),
            capture_kind: Set(event.capture_kind.clone()),
            source_hash: Set(event.source_hash.clone()),
            attempt_metadata: Set(event.attempt_metadata.clone()),
            error_history: Set(event.error_history.clone()),
            replay_count: Set(event.replay_count),
            next_retry_at: Set(event.next_retry_at.clone()),
            completed_at: Set(event.completed_at.clone()),
            created_at: Set(event.created_at.clone()),
            updated_at: Set(event.updated_at.clone()),
        }
    }

    pub(super) fn memory_operation_active_model(
        operation: &memory_operation::Model,
    ) -> Result<memory_operation::ActiveModel> {
        Ok(memory_operation::ActiveModel {
            id: Set(operation.id.clone()),
            capture_event_id: Set(operation.capture_event_id.clone()),
            operation_type: Set(operation.operation_type.clone()),
            status: Set(operation.status.clone()),
            target_memory_id: Set(operation.target_memory_id.clone()),
            applied_memory_id: Set(operation.applied_memory_id.clone()),
            key: Set(operation.key.clone()),
            value: Set(encrypt_optional_storage_string(operation.value.as_deref())?),
            memory_kind: Set(operation.memory_kind.clone()),
            durability: Set(operation.durability.clone()),
            scope: Set(operation.scope.clone()),
            project_id: Set(operation.project_id.clone()),
            conversation_id: Set(operation.conversation_id.clone()),
            confidence: Set(operation.confidence),
            looks_sensitive: Set(operation.looks_sensitive),
            sensitive_reason: Set(encrypt_optional_storage_string(
                operation.sensitive_reason.as_deref(),
            )?),
            valid_from: Set(operation.valid_from.clone()),
            expires_at: Set(operation.expires_at.clone()),
            review_at: Set(operation.review_at.clone()),
            rationale: Set(encrypt_optional_storage_string(
                operation.rationale.as_deref(),
            )?),
            evidence_refs: Set(operation.evidence_refs.clone()),
            model_metadata: Set(operation.model_metadata.clone()),
            apply_metadata: Set(operation.apply_metadata.clone()),
            applied_at: Set(operation.applied_at.clone()),
            reviewed_at: Set(operation.reviewed_at.clone()),
            review_notes: Set(encrypt_optional_storage_string(
                operation.review_notes.as_deref(),
            )?),
            created_at: Set(operation.created_at.clone()),
            updated_at: Set(operation.updated_at.clone()),
        })
    }

    pub(crate) fn experience_run_heavy_update_active_model(
        current: &experience_run::Model,
        next: &experience_run::Model,
    ) -> Option<experience_run::ActiveModel> {
        let mut model = experience_run::ActiveModel {
            id: Unchanged(next.id.clone()),
            ..Default::default()
        };
        let mut changed = false;

        set_if_changed(
            &mut model.request_text,
            &current.request_text,
            &next.request_text,
            &mut changed,
        );
        set_if_changed(
            &mut model.tool_sequence_json,
            &current.tool_sequence_json,
            &next.tool_sequence_json,
            &mut changed,
        );
        set_if_changed(
            &mut model.outcome_summary,
            &current.outcome_summary,
            &next.outcome_summary,
            &mut changed,
        );
        set_if_changed(
            &mut model.failure_reason,
            &current.failure_reason,
            &next.failure_reason,
            &mut changed,
        );
        set_if_changed(
            &mut model.metadata,
            &current.metadata,
            &next.metadata,
            &mut changed,
        );
        set_if_changed(
            &mut model.heuristic_reflection_error,
            &current.heuristic_reflection_error,
            &next.heuristic_reflection_error,
            &mut changed,
        );

        changed.then_some(model)
    }

    pub(crate) fn experience_item_heavy_update_active_model(
        current: &experience_item::Model,
        next: &experience_item::Model,
    ) -> Option<experience_item::ActiveModel> {
        let mut model = experience_item::ActiveModel {
            id: Unchanged(next.id.clone()),
            ..Default::default()
        };
        let mut changed = false;

        set_if_changed(
            &mut model.content,
            &current.content,
            &next.content,
            &mut changed,
        );
        set_if_changed(
            &mut model.metadata,
            &current.metadata,
            &next.metadata,
            &mut changed,
        );
        set_if_changed(
            &mut model.embedding,
            &current.embedding,
            &next.embedding,
            &mut changed,
        );

        changed.then_some(model)
    }

    pub(crate) fn memory_capture_event_heavy_update_active_model(
        current: &memory_capture_event::Model,
        next: &memory_capture_event::Model,
    ) -> Option<memory_capture_event::ActiveModel> {
        let mut model = memory_capture_event::ActiveModel {
            id: Unchanged(next.id.clone()),
            ..Default::default()
        };
        let mut changed = false;

        set_if_changed(
            &mut model.attempt_metadata,
            &current.attempt_metadata,
            &next.attempt_metadata,
            &mut changed,
        );
        set_if_changed(
            &mut model.error_history,
            &current.error_history,
            &next.error_history,
            &mut changed,
        );

        changed.then_some(model)
    }

    pub(crate) fn memory_operation_heavy_update_active_model(
        current: &memory_operation::Model,
        next: &memory_operation::Model,
    ) -> Result<Option<memory_operation::ActiveModel>> {
        let mut model = memory_operation::ActiveModel {
            id: Unchanged(next.id.clone()),
            ..Default::default()
        };
        let mut changed = false;

        set_encrypted_optional_string_if_changed(
            &mut model.value,
            &current.value,
            &next.value,
            &mut changed,
        )?;
        set_encrypted_optional_string_if_changed(
            &mut model.sensitive_reason,
            &current.sensitive_reason,
            &next.sensitive_reason,
            &mut changed,
        )?;
        set_encrypted_optional_string_if_changed(
            &mut model.rationale,
            &current.rationale,
            &next.rationale,
            &mut changed,
        )?;
        set_if_changed(
            &mut model.evidence_refs,
            &current.evidence_refs,
            &next.evidence_refs,
            &mut changed,
        );
        set_if_changed(
            &mut model.model_metadata,
            &current.model_metadata,
            &next.model_metadata,
            &mut changed,
        );
        set_if_changed(
            &mut model.apply_metadata,
            &current.apply_metadata,
            &next.apply_metadata,
            &mut changed,
        );
        set_encrypted_optional_string_if_changed(
            &mut model.review_notes,
            &current.review_notes,
            &next.review_notes,
            &mut changed,
        )?;

        Ok(changed.then_some(model))
    }

    pub(crate) fn semantic_work_unit_heavy_update_active_model(
        current: &semantic_work_unit::Model,
        next: &semantic_work_unit::Model,
    ) -> Result<Option<semantic_work_unit::ActiveModel>> {
        let mut model = semantic_work_unit::ActiveModel {
            id: Unchanged(next.id.clone()),
            ..Default::default()
        };
        let mut changed = false;

        set_encrypted_string_if_changed(
            &mut model.title,
            &current.title,
            &next.title,
            &mut changed,
        )?;
        set_encrypted_string_if_changed(
            &mut model.summary,
            &current.summary,
            &next.summary,
            &mut changed,
        )?;
        set_encrypted_string_if_changed(
            &mut model.content_preview,
            &current.content_preview,
            &next.content_preview,
            &mut changed,
        )?;
        set_if_changed(
            &mut model.metadata,
            &current.metadata,
            &next.metadata,
            &mut changed,
        );
        set_if_changed(
            &mut model.embedding,
            &current.embedding,
            &next.embedding,
            &mut changed,
        );

        Ok(changed.then_some(model))
    }

    pub(super) fn memory_evidence_link_active_model(
        link: &memory_evidence_link::Model,
    ) -> memory_evidence_link::ActiveModel {
        memory_evidence_link::ActiveModel {
            id: Set(link.id.clone()),
            operation_id: Set(link.operation_id.clone()),
            memory_id: Set(link.memory_id.clone()),
            evidence_kind: Set(link.evidence_kind.clone()),
            evidence_ref: Set(link.evidence_ref.clone()),
            source_message_id: Set(link.source_message_id.clone()),
            capture_event_id: Set(link.capture_event_id.clone()),
            project_id: Set(link.project_id.clone()),
            conversation_id: Set(link.conversation_id.clone()),
            metadata: Set(link.metadata.clone()),
            created_at: Set(link.created_at.clone()),
        }
    }

    pub(super) fn experience_item_is_arkmemory_memory(item: &experience_item::Model) -> bool {
        matches!(item.kind.as_str(), "personal_fact" | "constraint")
    }

    pub(super) fn recall_snapshot_experience_item(
        item: &experience_item::Model,
    ) -> Result<serde_json::Value> {
        let mut value = serde_json::to_value(item)?;
        if let Some(object) = value.as_object_mut() {
            object.insert("embedding".to_string(), serde_json::Value::Null);
        }
        Ok(value)
    }

    pub(super) fn experience_item_recall_event_type(
        previous: Option<&experience_item::Model>,
        next: &experience_item::Model,
    ) -> Option<&'static str> {
        if !Self::experience_item_is_arkmemory_memory(next) {
            return None;
        }
        let Some(previous) = previous else {
            return Some("memory_created");
        };
        if previous.status != next.status {
            return Some("memory_status_changed");
        }
        if previous.content != next.content
            || previous.title != next.title
            || previous.normalized_key != next.normalized_key
            || previous.scope != next.scope
            || previous.project_id != next.project_id
            || previous.conversation_id != next.conversation_id
        {
            return Some("memory_updated");
        }
        None
    }

    pub(super) async fn insert_recall_event_conn<C>(
        conn: &C,
        event: &recall_event::Model,
    ) -> Result<()>
    where
        C: ConnectionTrait,
    {
        recall_event::Entity::insert(Self::recall_event_active_model(event))
            .on_conflict(
                OnConflict::column(recall_event::Column::Id)
                    .update_columns([
                        recall_event::Column::EventType,
                        recall_event::Column::MemoryId,
                        recall_event::Column::RelatedMemoryId,
                        recall_event::Column::Scope,
                        recall_event::Column::ProjectId,
                        recall_event::Column::ConversationId,
                        recall_event::Column::SourceKind,
                        recall_event::Column::SourceRef,
                        recall_event::Column::Actor,
                        recall_event::Column::Summary,
                        recall_event::Column::OldSnapshot,
                        recall_event::Column::NewSnapshot,
                        recall_event::Column::Metadata,
                        recall_event::Column::RiskLevel,
                        recall_event::Column::Confidence,
                        recall_event::Column::Reversible,
                        recall_event::Column::RevertedAt,
                        recall_event::Column::UpdatedAt,
                    ])
                    .to_owned(),
            )
            .exec(conn)
            .await?;
        Ok(())
    }

    pub(super) async fn record_experience_item_recall_event_conn<C>(
        conn: &C,
        event_type: &str,
        previous: Option<&experience_item::Model>,
        next: &experience_item::Model,
        actor: &str,
        metadata: serde_json::Value,
    ) -> Result<()>
    where
        C: ConnectionTrait,
    {
        let now = chrono::Utc::now().to_rfc3339();
        let summary = match event_type {
            "memory_created" => format!("Created {}", next.title),
            "memory_status_changed" => format!("Changed {} status to {}", next.title, next.status),
            "memory_updated" => format!("Updated {}", next.title),
            _ => format!("Recorded {}", next.title),
        };
        let event = recall_event::Model {
            id: uuid::Uuid::new_v4().to_string(),
            event_type: event_type.to_string(),
            memory_id: Some(next.id.clone()),
            related_memory_id: None,
            scope: Some(next.scope.clone()),
            project_id: next.project_id.clone(),
            conversation_id: next.conversation_id.clone(),
            source_kind: Some("experience_item".to_string()),
            source_ref: Some(next.id.clone()),
            actor: actor.to_string(),
            summary: Some(summary),
            old_snapshot: previous
                .map(Self::recall_snapshot_experience_item)
                .transpose()?
                .unwrap_or(serde_json::Value::Null),
            new_snapshot: Self::recall_snapshot_experience_item(next)?,
            metadata,
            risk_level: None,
            confidence: Some(next.confidence),
            reversible: previous.is_some(),
            reverted_at: None,
            created_at: now.clone(),
            updated_at: now,
        };
        Self::insert_recall_event_conn(conn, &event).await
    }

    pub(super) async fn upsert_experience_item_conn<C>(
        conn: &C,
        item: &experience_item::Model,
    ) -> Result<()>
    where
        C: ConnectionTrait,
    {
        let previous = experience_item::Entity::find_by_id(item.id.clone())
            .one(conn)
            .await?;
        experience_item::Entity::insert(Self::experience_item_active_model(item))
            .on_conflict(
                OnConflict::column(experience_item::Column::Id)
                    .update_columns(EXPERIENCE_ITEM_LIGHT_UPSERT_COLUMNS.iter().copied())
                    .to_owned(),
            )
            .exec(conn)
            .await?;
        let current = experience_item::Entity::find_by_id(item.id.clone())
            .lock_exclusive()
            .one(conn)
            .await?
            .ok_or_else(|| anyhow!("Experience item '{}' missing after upsert", item.id))?;
        if let Some(model) = Self::experience_item_heavy_update_active_model(&current, item) {
            model.update(conn).await?;
        }
        if let Some(event_type) = Self::experience_item_recall_event_type(previous.as_ref(), item) {
            Self::record_experience_item_recall_event_conn(
                conn,
                event_type,
                previous.as_ref(),
                item,
                "system",
                serde_json::json!({ "origin": "experience_item_upsert" }),
            )
            .await?;
        }
        Ok(())
    }

    pub(super) async fn update_experience_item_status_conn<C>(
        conn: &C,
        id: &str,
        status: &str,
    ) -> Result<()>
    where
        C: ConnectionTrait,
    {
        let previous = experience_item::Entity::find_by_id(id.to_string())
            .one(conn)
            .await?;
        let now = chrono::Utc::now().to_rfc3339();
        experience_item::Entity::update_many()
            .col_expr(
                experience_item::Column::Status,
                Expr::value(status.to_string()),
            )
            .col_expr(experience_item::Column::UpdatedAt, Expr::value(now.clone()))
            .filter(experience_item::Column::Id.eq(id))
            .exec(conn)
            .await?;
        if let Some(previous_item) = previous.as_ref() {
            let mut next = previous_item.clone();
            next.status = status.to_string();
            next.updated_at = now;
            if let Some(event_type) =
                Self::experience_item_recall_event_type(Some(previous_item), &next)
            {
                Self::record_experience_item_recall_event_conn(
                    conn,
                    event_type,
                    Some(previous_item),
                    &next,
                    "system",
                    serde_json::json!({ "origin": "experience_item_status_update" }),
                )
                .await?;
            }
        }
        Ok(())
    }

    pub(super) async fn get_experience_item_conn<C>(
        conn: &C,
        id: &str,
    ) -> Result<Option<experience_item::Model>>
    where
        C: ConnectionTrait,
    {
        Ok(experience_item::Entity::find_by_id(id.to_string())
            .one(conn)
            .await?)
    }

    pub async fn upsert_experience_item(&self, item: &experience_item::Model) -> Result<()> {
        let txn = self.db.begin().await?;
        Self::upsert_experience_item_conn(&txn, item).await?;
        txn.commit().await?;
        Ok(())
    }

    pub(crate) async fn upsert_experience_item_txn(
        &self,
        txn: &DatabaseTransaction,
        item: &experience_item::Model,
    ) -> Result<()> {
        Self::upsert_experience_item_conn(txn, item).await
    }

    pub async fn update_experience_item_status(&self, id: &str, status: &str) -> Result<()> {
        let txn = self.db.begin().await?;
        Self::update_experience_item_status_conn(&txn, id, status).await?;
        txn.commit().await?;
        Ok(())
    }

    pub async fn update_experience_item_content(
        &self,
        id: &str,
        content: &str,
    ) -> Result<Option<experience_item::Model>> {
        let id = id.trim();
        if id.is_empty() {
            return Ok(None);
        }
        let txn = self.db.begin().await?;
        let Some(previous) = experience_item::Entity::find_by_id(id.to_string())
            .one(&txn)
            .await?
        else {
            txn.commit().await?;
            return Ok(None);
        };
        let mut next = previous.clone();
        next.content = content.to_string();
        next.updated_at = chrono::Utc::now().to_rfc3339();
        if previous.content != next.content {
            next.embedding = None;
        }
        Self::upsert_experience_item_conn(&txn, &next).await?;
        txn.commit().await?;
        Ok(Some(next))
    }

    pub async fn list_experience_items_between(
        &self,
        from: &str,
        to: &str,
        limit: u64,
    ) -> Result<Vec<experience_item::Model>> {
        Ok(experience_item::Entity::find()
            .filter(experience_item::Column::UpdatedAt.gte(from.to_string()))
            .filter(experience_item::Column::UpdatedAt.lt(to.to_string()))
            .filter(experience_item::Column::Status.eq("active"))
            .order_by_desc(experience_item::Column::UpdatedAt)
            .limit(Self::db_limit(limit))
            .all(&self.db)
            .await?)
    }

    pub(crate) async fn begin_experience_memory_write_txn(
        &self,
        kind: &str,
        scope: &str,
        project_id: Option<&str>,
        conversation_id: Option<&str>,
    ) -> Result<DatabaseTransaction> {
        let txn = self.db.begin().await?;
        self.acquire_experience_memory_write_lock_txn(
            &txn,
            kind,
            scope,
            project_id,
            conversation_id,
        )
        .await?;
        Ok(txn)
    }

    pub(crate) async fn acquire_experience_memory_write_lock_txn(
        &self,
        txn: &DatabaseTransaction,
        kind: &str,
        scope: &str,
        project_id: Option<&str>,
        conversation_id: Option<&str>,
    ) -> Result<()> {
        if txn.get_database_backend() == DbBackend::Postgres {
            let lock_key =
                experience_memory_write_lock_key(kind, scope, project_id, conversation_id);
            txn.execute(Statement::from_sql_and_values(
                DbBackend::Postgres,
                "SELECT pg_advisory_xact_lock($1)",
                vec![lock_key.into()],
            ))
            .await?;
        }
        Ok(())
    }

    /// Cosine-distance nearest-neighbour lookup over active experience items,
    /// scoped to the provided kinds and scope tuple. Returns (model, distance)
    /// pairs in ascending distance order (closest first). Distance is the
    /// pgvector cosine distance: 0.0 is identical, 1.0 is orthogonal, 2.0 is
    /// diametrically opposite. Callers convert to cosine similarity as
    /// `1.0 - distance` when scoring against a threshold.
    pub(super) async fn nearest_active_experience_items_semantic_conn<C>(
        conn: &C,
        kinds: &[&str],
        scope: &str,
        project_id: Option<&str>,
        conversation_id: Option<&str>,
        embedding: &PgVector,
        limit: u64,
    ) -> Result<Vec<(experience_item::Model, f64)>>
    where
        C: ConnectionTrait,
    {
        if limit == 0 || kinds.is_empty() {
            return Ok(Vec::new());
        }
        if conn.get_database_backend() != DbBackend::Postgres {
            return Ok(Vec::new());
        }
        let embedding_sql = pgvector_sql_literal(embedding);
        let kinds_list = sql_string_list(
            &kinds
                .iter()
                .map(|kind| (*kind).to_string())
                .collect::<Vec<_>>(),
        );
        let scope_filter = format!("scope = {}", sql_string_literal(scope));
        let project_filter = match project_id {
            Some(value) => format!("project_id = {}", sql_string_literal(value)),
            None => "project_id IS NULL".to_string(),
        };
        let conversation_filter = match conversation_id {
            Some(value) => format!("conversation_id = {}", sql_string_literal(value)),
            None => "conversation_id IS NULL".to_string(),
        };
        let sql = format!(
            "SELECT id, embedding <=> {embedding_sql} AS cosine_distance \
             FROM experience_items \
             WHERE status = 'active' \
               AND embedding IS NOT NULL \
               AND kind IN ({kinds_list}) \
               AND {scope_filter} \
               AND {project_filter} \
               AND {conversation_filter} \
             ORDER BY embedding <=> {embedding_sql} ASC \
             LIMIT {}",
            Self::db_limit(limit),
        );
        let rows = conn
            .query_all(Statement::from_string(DbBackend::Postgres, sql))
            .await?;
        let mut scored: Vec<(String, f64)> = Vec::with_capacity(rows.len());
        for row in rows {
            let id: String = row.try_get("", "id")?;
            let distance: f64 = row.try_get("", "cosine_distance")?;
            scored.push((id, distance));
        }
        if scored.is_empty() {
            return Ok(Vec::new());
        }
        let ids = scored.iter().map(|(id, _)| id.clone()).collect::<Vec<_>>();
        let models = experience_item::Entity::find()
            .filter(experience_item::Column::Id.is_in(ids.clone()))
            .all(conn)
            .await?;
        let mut by_id: std::collections::HashMap<String, experience_item::Model> = models
            .into_iter()
            .map(|model| (model.id.clone(), model))
            .collect();
        Ok(scored
            .into_iter()
            .filter_map(|(id, distance)| by_id.remove(&id).map(|model| (model, distance)))
            .collect())
    }

    pub(crate) async fn nearest_active_experience_items_semantic_txn(
        &self,
        txn: &DatabaseTransaction,
        kinds: &[&str],
        scope: &str,
        project_id: Option<&str>,
        conversation_id: Option<&str>,
        embedding: &PgVector,
        limit: u64,
    ) -> Result<Vec<(experience_item::Model, f64)>> {
        Self::nearest_active_experience_items_semantic_conn(
            txn,
            kinds,
            scope,
            project_id,
            conversation_id,
            embedding,
            limit,
        )
        .await
    }

    pub async fn get_experience_item(&self, id: &str) -> Result<Option<experience_item::Model>> {
        Self::get_experience_item_conn(&self.db, id).await
    }

    pub async fn hard_delete_experience_item_memory(&self, id: &str) -> Result<bool> {
        let id = id.trim();
        if id.is_empty() {
            return Ok(false);
        }
        let txn = self.db.begin().await?;
        let Some(_) = experience_item::Entity::find_by_id(id.to_string())
            .one(&txn)
            .await?
        else {
            txn.commit().await?;
            return Ok(false);
        };
        let operation_rows = memory_operation::Entity::find()
            .filter(
                Condition::any()
                    .add(memory_operation::Column::TargetMemoryId.eq(id.to_string()))
                    .add(memory_operation::Column::AppliedMemoryId.eq(id.to_string())),
            )
            .all(&txn)
            .await?;
        let operation_ids = operation_rows
            .iter()
            .map(|operation| operation.id.clone())
            .collect::<Vec<_>>();
        memory_evidence_link::Entity::delete_many()
            .filter(memory_evidence_link::Column::MemoryId.eq(id.to_string()))
            .exec(&txn)
            .await?;
        if !operation_ids.is_empty() {
            memory_evidence_link::Entity::delete_many()
                .filter(memory_evidence_link::Column::OperationId.is_in(operation_ids.clone()))
                .exec(&txn)
                .await?;
        }
        memory_operation::Entity::delete_many()
            .filter(
                Condition::any()
                    .add(memory_operation::Column::TargetMemoryId.eq(id.to_string()))
                    .add(memory_operation::Column::AppliedMemoryId.eq(id.to_string())),
            )
            .exec(&txn)
            .await?;
        learning_candidate::Entity::delete_many()
            .filter(learning_candidate::Column::ApprovedRef.eq(id.to_string()))
            .exec(&txn)
            .await?;
        if !operation_ids.is_empty() {
            let candidate_ids = operation_ids
                .iter()
                .map(|operation_id| format!("memory-candidate-{operation_id}"))
                .collect::<Vec<_>>();
            learning_candidate::Entity::delete_many()
                .filter(learning_candidate::Column::Id.is_in(candidate_ids))
                .exec(&txn)
                .await?;
        }
        experience_edge::Entity::delete_many()
            .filter(
                Condition::any()
                    .add(
                        Condition::all()
                            .add(experience_edge::Column::SourceKind.eq("experience_item"))
                            .add(experience_edge::Column::SourceRef.eq(id.to_string())),
                    )
                    .add(
                        Condition::all()
                            .add(experience_edge::Column::TargetKind.eq("experience_item"))
                            .add(experience_edge::Column::TargetRef.eq(id.to_string())),
                    ),
            )
            .exec(&txn)
            .await?;
        recall_event::Entity::delete_many()
            .filter(
                Condition::any()
                    .add(recall_event::Column::MemoryId.eq(id.to_string()))
                    .add(recall_event::Column::RelatedMemoryId.eq(id.to_string()))
                    .add(
                        Condition::all()
                            .add(recall_event::Column::SourceKind.eq("experience_item"))
                            .add(recall_event::Column::SourceRef.eq(id.to_string())),
                    ),
            )
            .exec(&txn)
            .await?;
        let result = experience_item::Entity::delete_by_id(id.to_string())
            .exec(&txn)
            .await?;
        txn.commit().await?;
        Ok(result.rows_affected > 0)
    }

    pub async fn insert_recall_event(&self, event: &recall_event::Model) -> Result<()> {
        Self::insert_recall_event_conn(&self.db, event).await
    }

    pub async fn get_recall_event(&self, id: &str) -> Result<Option<recall_event::Model>> {
        Ok(recall_event::Entity::find_by_id(id.to_string())
            .one(&self.db)
            .await?)
    }

    pub async fn list_recall_events(
        &self,
        limit: u64,
        offset: u64,
        project_id: Option<&str>,
    ) -> Result<Vec<recall_event::Model>> {
        let mut query = recall_event::Entity::find().order_by_desc(recall_event::Column::CreatedAt);
        query = match project_id.map(str::trim).filter(|value| !value.is_empty()) {
            Some(pid) => query.filter(
                Condition::any()
                    .add(recall_event::Column::ProjectId.is_null())
                    .add(recall_event::Column::ProjectId.eq(pid.to_string())),
            ),
            None => query.filter(recall_event::Column::ProjectId.is_null()),
        };
        Ok(query
            .limit(Self::db_limit(limit))
            .offset(offset)
            .all(&self.db)
            .await?)
    }

    pub async fn list_recall_events_for_memory(
        &self,
        memory_id: &str,
        limit: u64,
        project_id: Option<&str>,
    ) -> Result<Vec<recall_event::Model>> {
        let mut query = recall_event::Entity::find().filter(
            Condition::any()
                .add(recall_event::Column::MemoryId.eq(memory_id.to_string()))
                .add(recall_event::Column::RelatedMemoryId.eq(memory_id.to_string())),
        );
        query = match project_id.map(str::trim).filter(|value| !value.is_empty()) {
            Some(pid) => query.filter(
                Condition::any()
                    .add(recall_event::Column::ProjectId.is_null())
                    .add(recall_event::Column::ProjectId.eq(pid.to_string())),
            ),
            None => query.filter(recall_event::Column::ProjectId.is_null()),
        };
        Ok(query
            .order_by_desc(recall_event::Column::CreatedAt)
            .limit(Self::db_limit(limit))
            .all(&self.db)
            .await?)
    }

    pub async fn list_reverted_recall_events(
        &self,
        limit: u64,
        project_id: Option<&str>,
    ) -> Result<Vec<recall_event::Model>> {
        let mut query = recall_event::Entity::find()
            .filter(recall_event::Column::RevertedAt.is_not_null())
            .order_by_desc(recall_event::Column::CreatedAt);
        query = match project_id.map(str::trim).filter(|value| !value.is_empty()) {
            Some(pid) => query.filter(
                Condition::any()
                    .add(recall_event::Column::ProjectId.is_null())
                    .add(recall_event::Column::ProjectId.eq(pid.to_string())),
            ),
            None => query.filter(recall_event::Column::ProjectId.is_null()),
        };
        Ok(query.limit(Self::db_limit(limit)).all(&self.db).await?)
    }

    pub async fn count_recall_events(&self, project_id: Option<&str>) -> Result<u64> {
        let mut query = recall_event::Entity::find();
        query = match project_id.map(str::trim).filter(|value| !value.is_empty()) {
            Some(pid) => query.filter(
                Condition::any()
                    .add(recall_event::Column::ProjectId.is_null())
                    .add(recall_event::Column::ProjectId.eq(pid.to_string())),
            ),
            None => query.filter(recall_event::Column::ProjectId.is_null()),
        };
        Ok(query.count(&self.db).await?)
    }

    pub async fn rollback_recall_event_with_memory_snapshot(
        &self,
        event_id: &str,
        previous_memory: &experience_item::Model,
        rollback_event: &recall_event::Model,
    ) -> Result<bool> {
        let txn = self.db.begin().await?;
        let now = chrono::Utc::now().to_rfc3339();
        let result = recall_event::Entity::update_many()
            .col_expr(
                recall_event::Column::RevertedAt,
                Expr::value(Some(now.clone())),
            )
            .col_expr(recall_event::Column::UpdatedAt, Expr::value(now))
            .filter(recall_event::Column::Id.eq(event_id.to_string()))
            .filter(recall_event::Column::Reversible.eq(true))
            .filter(recall_event::Column::RevertedAt.is_null())
            .exec(&txn)
            .await?;
        if result.rows_affected == 0 {
            txn.rollback().await?;
            return Ok(false);
        }
        Self::upsert_experience_item_conn(&txn, previous_memory).await?;
        Self::insert_recall_event_conn(&txn, rollback_event).await?;
        txn.commit().await?;
        Ok(true)
    }

    pub async fn upsert_recall_test(&self, test: &recall_test::Model) -> Result<()> {
        recall_test::Entity::insert(Self::recall_test_active_model(test))
            .on_conflict(
                OnConflict::column(recall_test::Column::Id)
                    .update_columns([
                        recall_test::Column::MemoryId,
                        recall_test::Column::Scope,
                        recall_test::Column::ProjectId,
                        recall_test::Column::ConversationId,
                        recall_test::Column::Prompt,
                        recall_test::Column::ExpectedAnswer,
                        recall_test::Column::Status,
                        recall_test::Column::LastAnswer,
                        recall_test::Column::LastRunAt,
                        recall_test::Column::Metadata,
                        recall_test::Column::UpdatedAt,
                    ])
                    .to_owned(),
            )
            .exec(&self.db)
            .await?;
        Ok(())
    }

    pub async fn list_recall_tests(
        &self,
        limit: u64,
        offset: u64,
        project_id: Option<&str>,
    ) -> Result<Vec<recall_test::Model>> {
        let mut query = recall_test::Entity::find().order_by_desc(recall_test::Column::UpdatedAt);
        query = match project_id.map(str::trim).filter(|value| !value.is_empty()) {
            Some(pid) => query.filter(
                Condition::any()
                    .add(recall_test::Column::ProjectId.is_null())
                    .add(recall_test::Column::ProjectId.eq(pid.to_string())),
            ),
            None => query.filter(recall_test::Column::ProjectId.is_null()),
        };
        Ok(query
            .limit(Self::db_limit(limit))
            .offset(offset)
            .all(&self.db)
            .await?)
    }

    pub async fn count_recall_tests(&self, project_id: Option<&str>) -> Result<u64> {
        let mut query = recall_test::Entity::find();
        query = match project_id.map(str::trim).filter(|value| !value.is_empty()) {
            Some(pid) => query.filter(
                Condition::any()
                    .add(recall_test::Column::ProjectId.is_null())
                    .add(recall_test::Column::ProjectId.eq(pid.to_string())),
            ),
            None => query.filter(recall_test::Column::ProjectId.is_null()),
        };
        Ok(query.count(&self.db).await?)
    }

    pub async fn list_experience_edges_for_item(
        &self,
        item_id: &str,
        limit: u64,
    ) -> Result<Vec<experience_edge::Model>> {
        let capped = Self::db_limit(limit);
        Ok(experience_edge::Entity::find()
            .filter(
                Condition::any()
                    .add(
                        Condition::all()
                            .add(experience_edge::Column::SourceKind.eq("experience_item"))
                            .add(experience_edge::Column::SourceRef.eq(item_id.to_string())),
                    )
                    .add(
                        Condition::all()
                            .add(experience_edge::Column::TargetKind.eq("experience_item"))
                            .add(experience_edge::Column::TargetRef.eq(item_id.to_string())),
                    ),
            )
            .order_by_desc(experience_edge::Column::UpdatedAt)
            .limit(capped)
            .all(&self.db)
            .await?)
    }

    pub(crate) async fn get_experience_item_txn(
        &self,
        txn: &DatabaseTransaction,
        id: &str,
    ) -> Result<Option<experience_item::Model>> {
        Self::get_experience_item_conn(txn, id).await
    }

    pub async fn list_active_experience_items(
        &self,
        kinds: &[&str],
        project_id: Option<&str>,
        conversation_id: Option<&str>,
        limit: u64,
    ) -> Result<Vec<experience_item::Model>> {
        let mut query =
            experience_item::Entity::find().filter(experience_item::Column::Status.eq("active"));
        query = match conversation_id {
            Some(value) => query.filter(
                Condition::any()
                    .add(experience_item::Column::ConversationId.is_null())
                    .add(experience_item::Column::ConversationId.eq(value.to_string())),
            ),
            None => query.filter(experience_item::Column::ConversationId.is_null()),
        };
        query = match project_id {
            Some(value) => query.filter(
                Condition::any()
                    .add(experience_item::Column::ProjectId.is_null())
                    .add(experience_item::Column::ProjectId.eq(value.to_string())),
            ),
            None => query.filter(experience_item::Column::ProjectId.is_null()),
        };
        if !kinds.is_empty() {
            query = query.filter(
                experience_item::Column::Kind.is_in(
                    kinds
                        .iter()
                        .map(|kind| (*kind).to_string())
                        .collect::<Vec<_>>(),
                ),
            );
        }
        let capped_limit = limit.min(Self::MAX_EXPERIENCE_ITEM_ROWS_PER_QUERY);
        let mut items = query
            .order_by_desc(experience_item::Column::UpdatedAt)
            .limit(Self::db_limit(capped_limit))
            .all(&self.db)
            .await?;
        items.sort_by(|left, right| {
            scope_match_rank(
                right.project_id.as_deref(),
                right.conversation_id.as_deref(),
                project_id,
                conversation_id,
            )
            .cmp(&scope_match_rank(
                left.project_id.as_deref(),
                left.conversation_id.as_deref(),
                project_id,
                conversation_id,
            ))
            .then_with(|| {
                experience_item_kind_rank(&left.kind).cmp(&experience_item_kind_rank(&right.kind))
            })
            .then_with(|| right.confidence.total_cmp(&left.confidence))
            .then_with(|| right.support_count.cmp(&left.support_count))
            .then_with(|| right.updated_at.cmp(&left.updated_at))
        });
        items.truncate(capped_limit as usize);
        Ok(items)
    }

    pub async fn list_active_experience_items_any_scope(
        &self,
        kinds: &[&str],
        limit: u64,
    ) -> Result<Vec<experience_item::Model>> {
        let mut query =
            experience_item::Entity::find().filter(experience_item::Column::Status.eq("active"));
        if !kinds.is_empty() {
            query = query.filter(
                experience_item::Column::Kind.is_in(
                    kinds
                        .iter()
                        .map(|kind| (*kind).to_string())
                        .collect::<Vec<_>>(),
                ),
            );
        }
        let capped_limit = limit.min(Self::MAX_EXPERIENCE_ITEM_ROWS_PER_QUERY);
        let mut items = query
            .order_by_desc(experience_item::Column::UpdatedAt)
            .limit(Self::db_limit(capped_limit))
            .all(&self.db)
            .await?;
        items.sort_by(|left, right| {
            experience_item_kind_rank(&left.kind)
                .cmp(&experience_item_kind_rank(&right.kind))
                .then_with(|| right.confidence.total_cmp(&left.confidence))
                .then_with(|| right.support_count.cmp(&left.support_count))
                .then_with(|| right.updated_at.cmp(&left.updated_at))
        });
        items.truncate(capped_limit as usize);
        Ok(items)
    }

    pub async fn list_memory_experience_items_for_graph(
        &self,
        statuses: &[String],
        project_id: Option<&str>,
        limit: u64,
    ) -> Result<Vec<experience_item::Model>> {
        let mut query = experience_item::Entity::find().filter(
            experience_item::Column::Kind
                .is_in(["personal_fact".to_string(), "constraint".to_string()]),
        );
        if !statuses.is_empty() {
            query = query.filter(experience_item::Column::Status.is_in(statuses.to_vec()));
        }
        query = match project_id.map(str::trim).filter(|value| !value.is_empty()) {
            Some(pid) => query.filter(
                Condition::any()
                    .add(experience_item::Column::ProjectId.is_null())
                    .add(experience_item::Column::ProjectId.eq(pid.to_string())),
            ),
            None => query.filter(experience_item::Column::ProjectId.is_null()),
        };
        let capped_limit = limit.min(Self::MAX_EXPERIENCE_ITEM_ROWS_PER_QUERY);
        let mut items = query
            .order_by_desc(experience_item::Column::UpdatedAt)
            .limit(Self::db_limit(capped_limit))
            .all(&self.db)
            .await?;
        items.sort_by(|left, right| {
            experience_item_kind_rank(&left.kind)
                .cmp(&experience_item_kind_rank(&right.kind))
                .then_with(|| right.confidence.total_cmp(&left.confidence))
                .then_with(|| right.support_count.cmp(&left.support_count))
                .then_with(|| right.updated_at.cmp(&left.updated_at))
        });
        items.truncate(capped_limit as usize);
        Ok(items)
    }

    pub async fn search_experience_items(
        &self,
        query: &str,
        kinds: &[&str],
        project_id: Option<&str>,
        conversation_id: Option<&str>,
        limit: u64,
    ) -> Result<Vec<ExperienceItemSearchHit>> {
        let terms = normalized_search_terms(query);
        if terms.is_empty() {
            return Ok(Vec::new());
        }
        let mut items = self
            .list_active_experience_items(kinds, project_id, conversation_id, limit)
            .await?;
        let mut hits = Vec::new();
        for item in items.drain(..) {
            if !matches_search_terms(&terms, &[&item.title, &item.content]) {
                continue;
            }
            let score = search_score(&terms, &[(&item.title, 3.0), (&item.content, 1.0)]);
            hits.push(ExperienceItemSearchHit { item, score });
        }
        hits.sort_by(|left, right| {
            scope_match_rank(
                right.item.project_id.as_deref(),
                right.item.conversation_id.as_deref(),
                project_id,
                conversation_id,
            )
            .cmp(&scope_match_rank(
                left.item.project_id.as_deref(),
                left.item.conversation_id.as_deref(),
                project_id,
                conversation_id,
            ))
            .then_with(|| {
                experience_item_kind_rank(&left.item.kind)
                    .cmp(&experience_item_kind_rank(&right.item.kind))
            })
            .then_with(|| right.score.total_cmp(&left.score))
            .then_with(|| right.item.support_count.cmp(&left.item.support_count))
            .then_with(|| right.item.updated_at.cmp(&left.item.updated_at))
        });
        hits.truncate(limit.min(Self::MAX_EXPERIENCE_ITEM_ROWS_PER_QUERY) as usize);
        Ok(hits)
    }

    pub async fn upsert_experience_edge(&self, edge: &experience_edge::Model) -> Result<()> {
        experience_edge::Entity::insert(experience_edge::ActiveModel {
            id: Set(edge.id.clone()),
            source_ref: Set(edge.source_ref.clone()),
            source_kind: Set(edge.source_kind.clone()),
            target_ref: Set(edge.target_ref.clone()),
            target_kind: Set(edge.target_kind.clone()),
            edge_type: Set(edge.edge_type.clone()),
            weight: Set(edge.weight),
            source_run_id: Set(edge.source_run_id.clone()),
            metadata: Set(edge.metadata.clone()),
            created_at: Set(edge.created_at.clone()),
            updated_at: Set(edge.updated_at.clone()),
        })
        .on_conflict(
            OnConflict::column(experience_edge::Column::Id)
                .update_columns([
                    experience_edge::Column::SourceRef,
                    experience_edge::Column::SourceKind,
                    experience_edge::Column::TargetRef,
                    experience_edge::Column::TargetKind,
                    experience_edge::Column::EdgeType,
                    experience_edge::Column::Weight,
                    experience_edge::Column::SourceRunId,
                    experience_edge::Column::Metadata,
                    experience_edge::Column::UpdatedAt,
                ])
                .to_owned(),
        )
        .exec(&self.db)
        .await?;
        Ok(())
    }

    #[allow(dead_code)]
    pub async fn list_related_experience_items(
        &self,
        seed_refs: &[String],
        limit: u64,
    ) -> Result<Vec<experience_item::Model>> {
        if seed_refs.is_empty() {
            return Ok(Vec::new());
        }
        let seed_refs_vec = seed_refs.to_vec();
        let edges = experience_edge::Entity::find()
            .filter(
                Condition::any()
                    .add(
                        Condition::all()
                            .add(experience_edge::Column::SourceRef.is_in(seed_refs_vec.clone()))
                            .add(experience_edge::Column::TargetKind.eq("experience_item")),
                    )
                    .add(
                        Condition::all()
                            .add(experience_edge::Column::TargetRef.is_in(seed_refs_vec.clone()))
                            .add(experience_edge::Column::SourceKind.eq("experience_item")),
                    ),
            )
            .limit(Self::db_limit(
                Self::MAX_RELATED_EXPERIENCE_EDGE_ROWS_PER_QUERY.max(limit),
            ))
            .all(&self.db)
            .await?;
        let seed_set = seed_refs
            .iter()
            .cloned()
            .collect::<std::collections::HashSet<_>>();
        let related_ids = edges
            .into_iter()
            .filter_map(|edge| {
                if seed_set.contains(&edge.source_ref) && edge.target_kind == "experience_item" {
                    Some(edge.target_ref)
                } else if seed_set.contains(&edge.target_ref)
                    && edge.source_kind == "experience_item"
                {
                    Some(edge.source_ref)
                } else {
                    None
                }
            })
            .filter(|id| !seed_set.contains(id))
            .collect::<std::collections::HashSet<_>>()
            .into_iter()
            .collect::<Vec<_>>();
        if related_ids.is_empty() {
            return Ok(Vec::new());
        }
        let mut items = experience_item::Entity::find()
            .filter(experience_item::Column::Id.is_in(related_ids))
            .filter(experience_item::Column::Status.eq("active"))
            .all(&self.db)
            .await?;
        items.sort_by(|left, right| {
            right
                .support_count
                .cmp(&left.support_count)
                .then_with(|| right.confidence.total_cmp(&left.confidence))
                .then_with(|| right.updated_at.cmp(&left.updated_at))
        });
        items.truncate(limit.min(Self::MAX_EXPERIENCE_ITEM_ROWS_PER_QUERY) as usize);
        Ok(items)
    }

    pub async fn upsert_procedural_pattern(
        &self,
        pattern: &procedural_pattern::Model,
    ) -> Result<()> {
        procedural_pattern::Entity::insert(procedural_pattern::ActiveModel {
            id: Set(pattern.id.clone()),
            intent_key: Set(pattern.intent_key.clone()),
            scope: Set(pattern.scope.clone()),
            project_id: Set(pattern.project_id.clone()),
            conversation_id: Set(pattern.conversation_id.clone()),
            title: Set(pattern.title.clone()),
            trigger_summary: Set(pattern.trigger_summary.clone()),
            summary: Set(pattern.summary.clone()),
            tool_sequence_digest: Set(pattern.tool_sequence_digest.clone()),
            steps_json: Set(pattern.steps_json.clone()),
            tool_sequence_json: Set(pattern.tool_sequence_json.clone()),
            sample_count: Set(pattern.sample_count),
            success_count: Set(pattern.success_count),
            correction_count: Set(pattern.correction_count),
            success_rate: Set(pattern.success_rate),
            last_validated_at: Set(pattern.last_validated_at.clone()),
            status: Set(pattern.status.clone()),
            metadata: Set(pattern.metadata.clone()),
            created_at: Set(pattern.created_at.clone()),
            updated_at: Set(pattern.updated_at.clone()),
        })
        .on_conflict(
            OnConflict::column(procedural_pattern::Column::Id)
                .update_columns([
                    procedural_pattern::Column::IntentKey,
                    procedural_pattern::Column::Scope,
                    procedural_pattern::Column::ProjectId,
                    procedural_pattern::Column::ConversationId,
                    procedural_pattern::Column::Title,
                    procedural_pattern::Column::TriggerSummary,
                    procedural_pattern::Column::Summary,
                    procedural_pattern::Column::ToolSequenceDigest,
                    procedural_pattern::Column::StepsJson,
                    procedural_pattern::Column::ToolSequenceJson,
                    procedural_pattern::Column::SampleCount,
                    procedural_pattern::Column::SuccessCount,
                    procedural_pattern::Column::CorrectionCount,
                    procedural_pattern::Column::SuccessRate,
                    procedural_pattern::Column::LastValidatedAt,
                    procedural_pattern::Column::Status,
                    procedural_pattern::Column::Metadata,
                    procedural_pattern::Column::UpdatedAt,
                ])
                .to_owned(),
        )
        .exec(&self.db)
        .await?;
        Ok(())
    }

    pub async fn list_procedural_patterns_between(
        &self,
        from: &str,
        to: &str,
        limit: u64,
    ) -> Result<Vec<procedural_pattern::Model>> {
        Ok(procedural_pattern::Entity::find()
            .filter(procedural_pattern::Column::UpdatedAt.gte(from.to_string()))
            .filter(procedural_pattern::Column::UpdatedAt.lt(to.to_string()))
            .filter(procedural_pattern::Column::Status.eq("active"))
            .order_by_desc(procedural_pattern::Column::UpdatedAt)
            .limit(Self::db_limit(limit))
            .all(&self.db)
            .await?)
    }

    #[allow(dead_code)]
    pub async fn search_procedural_patterns(
        &self,
        query: &str,
        project_id: Option<&str>,
        conversation_id: Option<&str>,
        limit: u64,
    ) -> Result<Vec<ProceduralPatternSearchHit>> {
        let terms = normalized_search_terms(query);
        if terms.is_empty() {
            return Ok(Vec::new());
        }
        let mut patterns = self
            .list_procedural_patterns(project_id, conversation_id, &["active", "draft"], limit)
            .await?;
        let mut hits = Vec::new();
        for pattern in patterns.drain(..) {
            if !matches_search_terms(
                &terms,
                &[&pattern.title, &pattern.trigger_summary, &pattern.summary],
            ) {
                continue;
            }
            let score = search_score(
                &terms,
                &[
                    (&pattern.title, 3.0),
                    (&pattern.trigger_summary, 2.0),
                    (&pattern.summary, 1.0),
                ],
            );
            hits.push(ProceduralPatternSearchHit { pattern, score });
        }
        hits.sort_by(|left, right| {
            scope_match_rank(
                right.pattern.project_id.as_deref(),
                right.pattern.conversation_id.as_deref(),
                project_id,
                conversation_id,
            )
            .cmp(&scope_match_rank(
                left.pattern.project_id.as_deref(),
                left.pattern.conversation_id.as_deref(),
                project_id,
                conversation_id,
            ))
            .then_with(|| right.score.total_cmp(&left.score))
            .then_with(|| right.pattern.sample_count.cmp(&left.pattern.sample_count))
            .then_with(|| {
                right
                    .pattern
                    .success_rate
                    .total_cmp(&left.pattern.success_rate)
            })
            .then_with(|| right.pattern.updated_at.cmp(&left.pattern.updated_at))
        });
        hits.truncate(limit.min(Self::MAX_PROCEDURAL_PATTERN_ROWS_PER_QUERY) as usize);
        Ok(hits)
    }

    pub async fn list_candidate_ready_patterns(
        &self,
        min_samples: i32,
        min_success_rate: f64,
        limit: u64,
    ) -> Result<Vec<procedural_pattern::Model>> {
        Ok(procedural_pattern::Entity::find()
            .filter(procedural_pattern::Column::SampleCount.gte(min_samples))
            .filter(procedural_pattern::Column::SuccessRate.gte(min_success_rate))
            .filter(procedural_pattern::Column::Status.is_in(["active", "draft"]))
            .order_by_desc(procedural_pattern::Column::SuccessRate)
            .order_by_desc(procedural_pattern::Column::SampleCount)
            .order_by_desc(procedural_pattern::Column::UpdatedAt)
            .limit(Self::db_limit(
                limit.min(Self::MAX_PROCEDURAL_PATTERN_ROWS_PER_QUERY),
            ))
            .all(&self.db)
            .await?)
    }

    pub async fn get_procedural_pattern(
        &self,
        id: &str,
    ) -> Result<Option<procedural_pattern::Model>> {
        Ok(procedural_pattern::Entity::find_by_id(id.to_string())
            .one(&self.db)
            .await?)
    }

    pub async fn list_procedural_patterns(
        &self,
        project_id: Option<&str>,
        conversation_id: Option<&str>,
        statuses: &[&str],
        limit: u64,
    ) -> Result<Vec<procedural_pattern::Model>> {
        let mut query = procedural_pattern::Entity::find();
        query = match conversation_id {
            Some(value) => query.filter(
                Condition::any()
                    .add(procedural_pattern::Column::ConversationId.is_null())
                    .add(procedural_pattern::Column::ConversationId.eq(value.to_string())),
            ),
            None => query.filter(procedural_pattern::Column::ConversationId.is_null()),
        };
        query = match project_id {
            Some(value) => query.filter(
                Condition::any()
                    .add(procedural_pattern::Column::ProjectId.is_null())
                    .add(procedural_pattern::Column::ProjectId.eq(value.to_string())),
            ),
            None => query.filter(procedural_pattern::Column::ProjectId.is_null()),
        };
        if !statuses.is_empty() {
            query = query.filter(
                procedural_pattern::Column::Status.is_in(
                    statuses
                        .iter()
                        .map(|status| (*status).to_string())
                        .collect::<Vec<_>>(),
                ),
            );
        }

        let capped_limit = limit.min(Self::MAX_PROCEDURAL_PATTERN_ROWS_PER_QUERY);
        let mut patterns = query
            .limit(Self::db_limit(capped_limit))
            .all(&self.db)
            .await?;
        patterns.sort_by(|left, right| {
            scope_match_rank(
                right.project_id.as_deref(),
                right.conversation_id.as_deref(),
                project_id,
                conversation_id,
            )
            .cmp(&scope_match_rank(
                left.project_id.as_deref(),
                left.conversation_id.as_deref(),
                project_id,
                conversation_id,
            ))
            .then_with(|| {
                procedural_pattern_status_rank(&right.status)
                    .cmp(&procedural_pattern_status_rank(&left.status))
            })
            .then_with(|| right.sample_count.cmp(&left.sample_count))
            .then_with(|| right.success_rate.total_cmp(&left.success_rate))
            .then_with(|| right.updated_at.cmp(&left.updated_at))
        });
        patterns.truncate(capped_limit as usize);
        Ok(patterns)
    }

    pub async fn list_procedural_patterns_any_scope(
        &self,
        statuses: &[&str],
        limit: u64,
    ) -> Result<Vec<procedural_pattern::Model>> {
        let mut query = procedural_pattern::Entity::find();
        if !statuses.is_empty() {
            query = query.filter(
                procedural_pattern::Column::Status.is_in(
                    statuses
                        .iter()
                        .map(|status| (*status).to_string())
                        .collect::<Vec<_>>(),
                ),
            );
        }
        let capped_limit = limit.min(Self::MAX_PROCEDURAL_PATTERN_ROWS_PER_QUERY);
        let mut patterns = query
            .limit(Self::db_limit(capped_limit))
            .all(&self.db)
            .await?;
        patterns.sort_by(|left, right| {
            procedural_pattern_status_rank(&right.status)
                .cmp(&procedural_pattern_status_rank(&left.status))
                .then_with(|| right.sample_count.cmp(&left.sample_count))
                .then_with(|| right.success_rate.total_cmp(&left.success_rate))
                .then_with(|| right.updated_at.cmp(&left.updated_at))
        });
        patterns.truncate(capped_limit as usize);
        Ok(patterns)
    }

    pub async fn list_experience_edges_for_refs(
        &self,
        refs: &[String],
        limit: u64,
    ) -> Result<Vec<experience_edge::Model>> {
        if refs.is_empty() {
            return Ok(Vec::new());
        }
        let capped_limit = limit.min(500);
        Ok(experience_edge::Entity::find()
            .filter(
                Condition::any()
                    .add(experience_edge::Column::SourceRef.is_in(refs.to_vec()))
                    .add(experience_edge::Column::TargetRef.is_in(refs.to_vec())),
            )
            .order_by_desc(experience_edge::Column::UpdatedAt)
            .limit(Self::db_limit(capped_limit))
            .all(&self.db)
            .await?)
    }

    pub async fn upsert_learning_candidate_guarded(
        &self,
        lease_key: &str,
        guard: &KvLeaseGuard,
        candidate: &learning_candidate::Model,
    ) -> Result<bool> {
        let txn = self.db.begin().await?;
        if !self
            .require_kv_lease_guard_txn(&txn, lease_key, guard)
            .await?
        {
            txn.rollback().await?;
            return Ok(false);
        }
        self.upsert_learning_candidate_txn(&txn, candidate).await?;
        txn.commit().await?;
        Ok(true)
    }

    pub async fn upsert_learning_candidate(
        &self,
        candidate: &learning_candidate::Model,
    ) -> Result<()> {
        let txn = self.db.begin().await?;
        self.upsert_learning_candidate_txn(&txn, candidate).await?;
        txn.commit().await?;
        Ok(())
    }

    pub async fn upsert_memory_capture_event(
        &self,
        event: &memory_capture_event::Model,
    ) -> Result<()> {
        let txn = self.db.begin().await?;
        memory_capture_event::Entity::insert(Self::memory_capture_event_active_model(event))
            .on_conflict(
                OnConflict::column(memory_capture_event::Column::Id)
                    .update_columns(MEMORY_CAPTURE_EVENT_LIGHT_UPSERT_COLUMNS.iter().copied())
                    .to_owned(),
            )
            .exec(&txn)
            .await?;
        let current = memory_capture_event::Entity::find_by_id(event.id.clone())
            .lock_exclusive()
            .one(&txn)
            .await?
            .ok_or_else(|| anyhow!("Memory capture event '{}' missing after upsert", event.id))?;
        if let Some(model) = Self::memory_capture_event_heavy_update_active_model(&current, event) {
            model.update(&txn).await?;
        }
        txn.commit().await?;
        Ok(())
    }

    pub async fn get_memory_capture_event(
        &self,
        id: &str,
    ) -> Result<Option<memory_capture_event::Model>> {
        Ok(memory_capture_event::Entity::find_by_id(id.to_string())
            .one(&self.db)
            .await?)
    }

    pub async fn list_memory_capture_events_by_statuses(
        &self,
        statuses: &[&str],
        project_id: Option<&str>,
        limit: u64,
    ) -> Result<Vec<memory_capture_event::Model>> {
        let mut query = memory_capture_event::Entity::find();
        if !statuses.is_empty() {
            query = query.filter(
                memory_capture_event::Column::Status.is_in(
                    statuses
                        .iter()
                        .map(|status| (*status).to_string())
                        .collect::<Vec<_>>(),
                ),
            );
        }
        query = match project_id.map(str::trim).filter(|value| !value.is_empty()) {
            Some(pid) => query.filter(
                Condition::any()
                    .add(memory_capture_event::Column::ProjectId.is_null())
                    .add(memory_capture_event::Column::ProjectId.eq(pid.to_string())),
            ),
            None => query.filter(memory_capture_event::Column::ProjectId.is_null()),
        };
        Ok(query
            .order_by_desc(memory_capture_event::Column::UpdatedAt)
            .limit(Self::db_limit(limit))
            .all(&self.db)
            .await?)
    }

    pub async fn list_memory_capture_events_by_statuses_all_scopes(
        &self,
        statuses: &[&str],
        limit: u64,
    ) -> Result<Vec<memory_capture_event::Model>> {
        let mut query = memory_capture_event::Entity::find();
        if !statuses.is_empty() {
            query = query.filter(
                memory_capture_event::Column::Status.is_in(
                    statuses
                        .iter()
                        .map(|status| (*status).to_string())
                        .collect::<Vec<_>>(),
                ),
            );
        }
        Ok(query
            .order_by_asc(memory_capture_event::Column::UpdatedAt)
            .limit(Self::db_limit(limit))
            .all(&self.db)
            .await?)
    }

    pub async fn count_memory_capture_events_by_statuses_all_scopes(
        &self,
        statuses: &[&str],
    ) -> Result<u64> {
        let mut query = memory_capture_event::Entity::find();
        if !statuses.is_empty() {
            query = query.filter(
                memory_capture_event::Column::Status.is_in(
                    statuses
                        .iter()
                        .map(|status| (*status).to_string())
                        .collect::<Vec<_>>(),
                ),
            );
        }
        Ok(query.count(&self.db).await?)
    }

    pub async fn count_memory_capture_events_by_source_hash(
        &self,
        source_hash: &str,
    ) -> Result<u64> {
        Ok(memory_capture_event::Entity::find()
            .filter(memory_capture_event::Column::SourceHash.eq(source_hash.to_string()))
            .count(&self.db)
            .await?)
    }

    pub async fn list_memory_capture_events_by_source_hash(
        &self,
        source_hash: &str,
        limit: u64,
    ) -> Result<Vec<memory_capture_event::Model>> {
        Ok(memory_capture_event::Entity::find()
            .filter(memory_capture_event::Column::SourceHash.eq(source_hash.to_string()))
            .order_by_desc(memory_capture_event::Column::UpdatedAt)
            .limit(Self::db_limit(limit))
            .all(&self.db)
            .await?)
    }

    pub async fn try_claim_memory_capture_event_status(
        &self,
        id: &str,
        expected_status: &str,
        claimed_status: &str,
        updated_at: &str,
    ) -> Result<bool> {
        let result = memory_capture_event::Entity::update_many()
            .col_expr(
                memory_capture_event::Column::Status,
                Expr::value(claimed_status.to_string()),
            )
            .col_expr(
                memory_capture_event::Column::UpdatedAt,
                Expr::value(updated_at.to_string()),
            )
            .filter(memory_capture_event::Column::Id.eq(id.to_string()))
            .filter(memory_capture_event::Column::Status.eq(expected_status.to_string()))
            .exec(&self.db)
            .await?;
        Ok(result.rows_affected > 0)
    }

    pub async fn try_update_memory_capture_event_from_status(
        &self,
        event: &memory_capture_event::Model,
        expected_status: &str,
    ) -> Result<bool> {
        let result = memory_capture_event::Entity::update_many()
            .col_expr(
                memory_capture_event::Column::SourceMessageId,
                Expr::value(event.source_message_id.clone()),
            )
            .col_expr(
                memory_capture_event::Column::ConversationId,
                Expr::value(event.conversation_id.clone()),
            )
            .col_expr(
                memory_capture_event::Column::ProjectId,
                Expr::value(event.project_id.clone()),
            )
            .col_expr(
                memory_capture_event::Column::Channel,
                Expr::value(event.channel.clone()),
            )
            .col_expr(
                memory_capture_event::Column::Status,
                Expr::value(event.status.clone()),
            )
            .col_expr(
                memory_capture_event::Column::CaptureKind,
                Expr::value(event.capture_kind.clone()),
            )
            .col_expr(
                memory_capture_event::Column::SourceHash,
                Expr::value(event.source_hash.clone()),
            )
            .col_expr(
                memory_capture_event::Column::AttemptMetadata,
                Expr::value(event.attempt_metadata.clone()),
            )
            .col_expr(
                memory_capture_event::Column::ErrorHistory,
                Expr::value(event.error_history.clone()),
            )
            .col_expr(
                memory_capture_event::Column::ReplayCount,
                Expr::value(event.replay_count),
            )
            .col_expr(
                memory_capture_event::Column::NextRetryAt,
                Expr::value(event.next_retry_at.clone()),
            )
            .col_expr(
                memory_capture_event::Column::CompletedAt,
                Expr::value(event.completed_at.clone()),
            )
            .col_expr(
                memory_capture_event::Column::CreatedAt,
                Expr::value(event.created_at.clone()),
            )
            .col_expr(
                memory_capture_event::Column::UpdatedAt,
                Expr::value(event.updated_at.clone()),
            )
            .filter(memory_capture_event::Column::Id.eq(event.id.clone()))
            .filter(memory_capture_event::Column::Status.eq(expected_status.to_string()))
            .exec(&self.db)
            .await?;
        Ok(result.rows_affected > 0)
    }

    pub async fn upsert_memory_operation(&self, operation: &memory_operation::Model) -> Result<()> {
        let txn = self.db.begin().await?;
        memory_operation::Entity::insert(Self::memory_operation_active_model(operation)?)
            .on_conflict(
                OnConflict::column(memory_operation::Column::Id)
                    .update_columns(MEMORY_OPERATION_LIGHT_UPSERT_COLUMNS.iter().copied())
                    .to_owned(),
            )
            .exec(&txn)
            .await?;
        let current = memory_operation::Entity::find_by_id(operation.id.clone())
            .lock_exclusive()
            .one(&txn)
            .await?
            .ok_or_else(|| anyhow!("Memory operation '{}' missing after upsert", operation.id))?;
        if let Some(model) = Self::memory_operation_heavy_update_active_model(&current, operation)?
        {
            model.update(&txn).await?;
        }
        txn.commit().await?;
        Ok(())
    }

    pub async fn get_memory_operation(&self, id: &str) -> Result<Option<memory_operation::Model>> {
        let Some(mut model) = memory_operation::Entity::find_by_id(id.to_string())
            .one(&self.db)
            .await?
        else {
            return Ok(None);
        };
        decrypt_memory_operation_model(&mut model);
        Ok(Some(model))
    }

    pub async fn list_memory_operations_for_memory(
        &self,
        memory_id: &str,
        project_id: Option<&str>,
        limit: u64,
    ) -> Result<Vec<memory_operation::Model>> {
        let mut query = memory_operation::Entity::find().filter(
            Condition::any()
                .add(memory_operation::Column::TargetMemoryId.eq(memory_id.to_string()))
                .add(memory_operation::Column::AppliedMemoryId.eq(memory_id.to_string())),
        );
        query = match project_id.map(str::trim).filter(|value| !value.is_empty()) {
            Some(pid) => query.filter(
                Condition::any()
                    .add(memory_operation::Column::ProjectId.is_null())
                    .add(memory_operation::Column::ProjectId.eq(pid.to_string())),
            ),
            None => query.filter(memory_operation::Column::ProjectId.is_null()),
        };
        let mut rows = query
            .order_by_desc(memory_operation::Column::UpdatedAt)
            .limit(Self::db_limit(limit))
            .all(&self.db)
            .await?;
        for row in &mut rows {
            decrypt_memory_operation_model(row);
        }
        Ok(rows)
    }

    pub async fn list_memory_operations_by_statuses(
        &self,
        statuses: &[&str],
        project_id: Option<&str>,
        limit: u64,
    ) -> Result<Vec<memory_operation::Model>> {
        let mut query = memory_operation::Entity::find();
        if !statuses.is_empty() {
            query = query.filter(
                memory_operation::Column::Status.is_in(
                    statuses
                        .iter()
                        .map(|status| (*status).to_string())
                        .collect::<Vec<_>>(),
                ),
            );
        }
        query = match project_id.map(str::trim).filter(|value| !value.is_empty()) {
            Some(pid) => query.filter(
                Condition::any()
                    .add(memory_operation::Column::ProjectId.is_null())
                    .add(memory_operation::Column::ProjectId.eq(pid.to_string())),
            ),
            None => query.filter(memory_operation::Column::ProjectId.is_null()),
        };
        let mut rows = query
            .order_by_desc(memory_operation::Column::UpdatedAt)
            .limit(Self::db_limit(limit))
            .all(&self.db)
            .await?;
        for row in &mut rows {
            decrypt_memory_operation_model(row);
        }
        Ok(rows)
    }

    pub async fn count_memory_operations_by_statuses_all_scopes(
        &self,
        statuses: &[&str],
    ) -> Result<u64> {
        let mut query = memory_operation::Entity::find();
        if !statuses.is_empty() {
            query = query.filter(
                memory_operation::Column::Status.is_in(
                    statuses
                        .iter()
                        .map(|status| (*status).to_string())
                        .collect::<Vec<_>>(),
                ),
            );
        }
        Ok(query.count(&self.db).await?)
    }

    pub async fn upsert_memory_evidence_link(
        &self,
        link: &memory_evidence_link::Model,
    ) -> Result<()> {
        memory_evidence_link::Entity::insert(Self::memory_evidence_link_active_model(link))
            .on_conflict(
                OnConflict::column(memory_evidence_link::Column::Id)
                    .update_columns([
                        memory_evidence_link::Column::OperationId,
                        memory_evidence_link::Column::MemoryId,
                        memory_evidence_link::Column::EvidenceKind,
                        memory_evidence_link::Column::EvidenceRef,
                        memory_evidence_link::Column::SourceMessageId,
                        memory_evidence_link::Column::CaptureEventId,
                        memory_evidence_link::Column::ProjectId,
                        memory_evidence_link::Column::ConversationId,
                        memory_evidence_link::Column::Metadata,
                    ])
                    .to_owned(),
            )
            .exec(&self.db)
            .await?;
        Ok(())
    }

    pub async fn list_memory_evidence_links_for_memory(
        &self,
        memory_id: &str,
        project_id: Option<&str>,
        limit: u64,
    ) -> Result<Vec<memory_evidence_link::Model>> {
        let mut query = memory_evidence_link::Entity::find()
            .filter(memory_evidence_link::Column::MemoryId.eq(memory_id.to_string()));
        query = match project_id.map(str::trim).filter(|value| !value.is_empty()) {
            Some(pid) => query.filter(
                Condition::any()
                    .add(memory_evidence_link::Column::ProjectId.is_null())
                    .add(memory_evidence_link::Column::ProjectId.eq(pid.to_string())),
            ),
            None => query.filter(memory_evidence_link::Column::ProjectId.is_null()),
        };
        Ok(query
            .order_by_desc(memory_evidence_link::Column::CreatedAt)
            .limit(Self::db_limit(limit))
            .all(&self.db)
            .await?)
    }

    pub async fn get_learning_candidate(
        &self,
        id: &str,
    ) -> Result<Option<learning_candidate::Model>> {
        Ok(learning_candidate::Entity::find_by_id(id.to_string())
            .one(&self.db)
            .await?)
    }

    pub async fn list_learning_candidates_with_options(
        &self,
        approval_status: Option<&str>,
        include_superseded: bool,
        limit: u64,
    ) -> Result<Vec<learning_candidate::Model>> {
        let mut query = learning_candidate::Entity::find();
        if let Some(status) = approval_status.filter(|v| !v.trim().is_empty()) {
            query = query.filter(learning_candidate::Column::ApprovalStatus.eq(status));
        } else if !include_superseded {
            query = query.filter(learning_candidate::Column::ApprovalStatus.ne("superseded"));
        }
        Ok(query
            .order_by_desc(learning_candidate::Column::UpdatedAt)
            .limit(Self::db_limit(limit))
            .all(&self.db)
            .await?)
    }

    pub async fn list_learning_candidates_for_review(
        &self,
        approval_statuses: &[&str],
        candidate_types: &[&str],
        project_id: Option<&str>,
        limit: u64,
    ) -> Result<Vec<learning_candidate::Model>> {
        let mut query = learning_candidate::Entity::find();
        if !approval_statuses.is_empty() {
            query = query.filter(
                learning_candidate::Column::ApprovalStatus.is_in(
                    approval_statuses
                        .iter()
                        .map(|status| (*status).to_string())
                        .collect::<Vec<_>>(),
                ),
            );
        }
        if !candidate_types.is_empty() {
            query = query.filter(
                learning_candidate::Column::CandidateType.is_in(
                    candidate_types
                        .iter()
                        .map(|candidate_type| (*candidate_type).to_string())
                        .collect::<Vec<_>>(),
                ),
            );
        }
        query = match project_id.map(str::trim).filter(|value| !value.is_empty()) {
            Some(pid) => query.filter(
                Condition::any()
                    .add(learning_candidate::Column::ProjectId.is_null())
                    .add(learning_candidate::Column::ProjectId.eq(pid.to_string())),
            ),
            None => query.filter(learning_candidate::Column::ProjectId.is_null()),
        };
        Ok(query
            .order_by_desc(learning_candidate::Column::UpdatedAt)
            .limit(Self::db_limit(limit))
            .all(&self.db)
            .await?)
    }

    #[allow(dead_code)]
    pub async fn list_learning_candidates(
        &self,
        approval_status: Option<&str>,
        limit: u64,
    ) -> Result<Vec<learning_candidate::Model>> {
        self.list_learning_candidates_with_options(approval_status, false, limit)
            .await
    }

    pub async fn list_learning_candidates_for_subject(
        &self,
        candidate_type: &str,
        subject_key: &str,
        limit: u64,
    ) -> Result<Vec<learning_candidate::Model>> {
        Ok(learning_candidate::Entity::find()
            .filter(learning_candidate::Column::CandidateType.eq(candidate_type.to_string()))
            .filter(learning_candidate::Column::SubjectKey.eq(subject_key.to_string()))
            .order_by_desc(learning_candidate::Column::UpdatedAt)
            .limit(Self::db_limit(limit))
            .all(&self.db)
            .await?)
    }

    pub async fn list_learning_candidates_for_subject_key(
        &self,
        subject_key: &str,
        candidate_types: &[&str],
        project_id: Option<&str>,
        limit: u64,
    ) -> Result<Vec<learning_candidate::Model>> {
        let mut query = learning_candidate::Entity::find()
            .filter(learning_candidate::Column::SubjectKey.eq(subject_key.to_string()));
        if !candidate_types.is_empty() {
            query = query.filter(
                learning_candidate::Column::CandidateType.is_in(
                    candidate_types
                        .iter()
                        .map(|candidate_type| (*candidate_type).to_string())
                        .collect::<Vec<_>>(),
                ),
            );
        }
        query = match project_id.map(str::trim).filter(|value| !value.is_empty()) {
            Some(pid) => query.filter(
                Condition::any()
                    .add(learning_candidate::Column::ProjectId.is_null())
                    .add(learning_candidate::Column::ProjectId.eq(pid.to_string())),
            ),
            None => query.filter(learning_candidate::Column::ProjectId.is_null()),
        };
        Ok(query
            .order_by_desc(learning_candidate::Column::UpdatedAt)
            .limit(Self::db_limit(limit))
            .all(&self.db)
            .await?)
    }

    pub async fn update_learning_candidate_review(
        &self,
        id: &str,
        approval_status: &str,
        review_notes: Option<&str>,
        approved_ref: Option<&str>,
    ) -> Result<()> {
        learning_candidate::Entity::update_many()
            .col_expr(
                learning_candidate::Column::ApprovalStatus,
                Expr::value(approval_status.to_string()),
            )
            .col_expr(
                learning_candidate::Column::ReviewNotes,
                Expr::value(review_notes.map(|value| value.to_string())),
            )
            .col_expr(
                learning_candidate::Column::ReviewedAt,
                Expr::value(Some(chrono::Utc::now().to_rfc3339())),
            )
            .col_expr(
                learning_candidate::Column::ApprovedRef,
                Expr::value(approved_ref.map(|value| value.to_string())),
            )
            .col_expr(
                learning_candidate::Column::UpdatedAt,
                Expr::value(chrono::Utc::now().to_rfc3339()),
            )
            .filter(learning_candidate::Column::Id.eq(id))
            .exec(&self.db)
            .await?;
        Ok(())
    }

    pub async fn update_learning_candidate_review_if_status(
        &self,
        id: &str,
        expected_status: &str,
        approval_status: &str,
        review_notes: Option<&str>,
        approved_ref: Option<&str>,
    ) -> Result<bool> {
        let now = chrono::Utc::now().to_rfc3339();
        let result = learning_candidate::Entity::update_many()
            .col_expr(
                learning_candidate::Column::ApprovalStatus,
                Expr::value(approval_status.to_string()),
            )
            .col_expr(
                learning_candidate::Column::ReviewNotes,
                Expr::value(review_notes.map(|value| value.to_string())),
            )
            .col_expr(
                learning_candidate::Column::ReviewedAt,
                Expr::value(Some(now.clone())),
            )
            .col_expr(
                learning_candidate::Column::ApprovedRef,
                Expr::value(approved_ref.map(|value| value.to_string())),
            )
            .col_expr(learning_candidate::Column::UpdatedAt, Expr::value(now))
            .filter(learning_candidate::Column::Id.eq(id))
            .filter(learning_candidate::Column::ApprovalStatus.eq(expected_status))
            .exec(&self.db)
            .await?;
        Ok(result.rows_affected > 0)
    }

    pub async fn update_learning_candidate_review_guarded(
        &self,
        lease_key: &str,
        guard: &KvLeaseGuard,
        id: &str,
        approval_status: &str,
        review_notes: Option<&str>,
        approved_ref: Option<&str>,
    ) -> Result<bool> {
        let txn = self.db.begin().await?;
        if !self
            .require_kv_lease_guard_txn(&txn, lease_key, guard)
            .await?
        {
            txn.rollback().await?;
            return Ok(false);
        }
        self.update_learning_candidate_review_txn(
            &txn,
            id,
            approval_status,
            review_notes,
            approved_ref,
        )
        .await?;
        txn.commit().await?;
        Ok(true)
    }

    pub async fn disable_strategy_canary_for_version(
        &self,
        candidate_version: &str,
    ) -> Result<bool> {
        let txn = self.db.begin().await?;
        self.ensure_kv_row_exists_txn(
            &txn,
            crate::core::self_evolve::strategy_runtime::TOOL_STRATEGY_CANARY_STATE_KEY,
        )
        .await?;
        let Some(mut canary_state) = self
            .load_kv_json_txn::<crate::core::self_evolve::strategy_runtime::CanaryRolloutState>(
                &txn,
                crate::core::self_evolve::strategy_runtime::TOOL_STRATEGY_CANARY_STATE_KEY,
            )
            .await?
        else {
            txn.rollback().await?;
            return Ok(false);
        };
        if canary_state.candidate_version != candidate_version {
            txn.rollback().await?;
            return Ok(false);
        }
        canary_state.enabled = false;
        self.set_kv_json_txn(
            &txn,
            crate::core::self_evolve::strategy_runtime::TOOL_STRATEGY_CANARY_STATE_KEY,
            &canary_state,
        )
        .await?;
        txn.commit().await?;
        Ok(true)
    }

    pub async fn approve_strategy_learning_candidate(
        &self,
        candidate_id: &str,
        review_notes: Option<&str>,
    ) -> Result<String> {
        let txn = self.db.begin().await?;
        let candidate = self
            .load_learning_candidate_txn(&txn, candidate_id)
            .await?
            .ok_or_else(|| anyhow!("Learning candidate '{}' not found", candidate_id))?;
        if candidate.candidate_type != "strategy" {
            anyhow::bail!(
                "Learning candidate '{}' is not a strategy candidate",
                candidate_id
            );
        }
        let profile = parse_strategy_candidate_profile(&candidate)?;
        let baseline_version = self
            .load_kv_json_txn::<crate::core::self_evolve::strategy_runtime::ToolStrategyProfile>(
                &txn,
                crate::core::self_evolve::strategy_runtime::TOOL_STRATEGY_PROFILE_KEY,
            )
            .await?
            .map(|value| value.version)
            .unwrap_or_else(|| "strategy-v1".to_string());
        let canary_state = crate::core::self_evolve::strategy_runtime::CanaryRolloutState {
            enabled: true,
            baseline_version,
            candidate_version: profile.version.clone(),
            rollout_percent: 20,
            activated_at: Some(chrono::Utc::now().to_rfc3339()),
            ..Default::default()
        };
        self.set_kv_json_txn(
            &txn,
            crate::core::self_evolve::strategy_runtime::TOOL_STRATEGY_PROFILE_CANARY_KEY,
            &profile,
        )
        .await?;
        self.set_kv_json_txn(
            &txn,
            crate::core::self_evolve::strategy_runtime::TOOL_STRATEGY_CANARY_STATE_KEY,
            &canary_state,
        )
        .await?;
        self.update_learning_candidate_review_txn(
            &txn,
            candidate_id,
            "approved",
            review_notes,
            Some(&profile.version),
        )
        .await?;
        txn.commit().await?;
        Ok(profile.version)
    }

    pub async fn reject_strategy_learning_candidate(
        &self,
        candidate_id: &str,
        review_notes: Option<&str>,
    ) -> Result<String> {
        let txn = self.db.begin().await?;
        let candidate = self
            .load_learning_candidate_txn(&txn, candidate_id)
            .await?
            .ok_or_else(|| anyhow!("Learning candidate '{}' not found", candidate_id))?;
        if candidate.candidate_type != "strategy" {
            anyhow::bail!(
                "Learning candidate '{}' is not a strategy candidate",
                candidate_id
            );
        }
        let profile = parse_strategy_candidate_profile(&candidate)?;
        let canary_key = crate::core::self_evolve::strategy_runtime::TOOL_STRATEGY_CANARY_STATE_KEY;
        if let Some(mut canary_state) = self
            .load_kv_json_txn::<crate::core::self_evolve::strategy_runtime::CanaryRolloutState>(
                &txn, canary_key,
            )
            .await?
        {
            if canary_state.candidate_version == profile.version {
                canary_state.enabled = false;
                self.set_kv_json_txn(&txn, canary_key, &canary_state)
                    .await?;
            }
        }
        self.update_learning_candidate_review_txn(
            &txn,
            candidate_id,
            "rejected",
            review_notes,
            None,
        )
        .await?;
        txn.commit().await?;
        Ok(profile.version)
    }

    pub async fn promote_strategy_learning_candidate_to_baseline(
        &self,
        candidate_id: &str,
    ) -> Result<String> {
        let txn = self.db.begin().await?;
        let candidate = self
            .load_learning_candidate_txn(&txn, candidate_id)
            .await?
            .ok_or_else(|| anyhow!("Learning candidate '{}' not found", candidate_id))?;
        if candidate.candidate_type != "strategy" {
            anyhow::bail!(
                "Learning candidate '{}' is not a strategy candidate",
                candidate_id
            );
        }
        if !candidate.approval_status.eq_ignore_ascii_case("approved") {
            anyhow::bail!("Strategy candidate must be approved before promotion");
        }
        let profile = parse_strategy_candidate_profile(&candidate)?;
        let profile_key = crate::core::self_evolve::strategy_runtime::TOOL_STRATEGY_PROFILE_KEY;
        let snapshot_key =
            crate::core::self_evolve::strategy_runtime::TOOL_STRATEGY_PROFILE_BASELINE_SNAPSHOT_KEY;
        if let Some(existing_baseline) = self.get_kv_for_update_txn(&txn, profile_key).await? {
            if !existing_baseline.value.is_empty() {
                self.set_kv_txn(&txn, snapshot_key, &existing_baseline.value)
                    .await?;
            }
        }
        self.set_kv_json_txn(&txn, profile_key, &profile).await?;

        let canary_key = crate::core::self_evolve::strategy_runtime::TOOL_STRATEGY_CANARY_STATE_KEY;
        let mut canary_state = self
            .load_kv_json_txn::<crate::core::self_evolve::strategy_runtime::CanaryRolloutState>(
                &txn, canary_key,
            )
            .await?
            .unwrap_or_default();
        canary_state.enabled = false;
        canary_state.baseline_version = profile.version.clone();
        canary_state.candidate_version = profile.version.clone();
        self.set_kv_json_txn(&txn, canary_key, &canary_state)
            .await?;

        txn.commit().await?;
        Ok(profile.version)
    }

    pub async fn rollback_tool_strategy_baseline(&self) -> Result<String> {
        let txn = self.db.begin().await?;
        let snapshot_key =
            crate::core::self_evolve::strategy_runtime::TOOL_STRATEGY_PROFILE_BASELINE_SNAPSHOT_KEY;
        let snapshot_row = self
            .get_kv_for_update_txn(&txn, snapshot_key)
            .await?
            .ok_or_else(|| anyhow!("No tool-strategy baseline snapshot available for rollback"))?;
        let snapshot = snapshot_row.value;
        if snapshot.is_empty() {
            anyhow::bail!("No tool-strategy baseline snapshot available for rollback");
        }
        let restored_profile = parse_kv_json_value::<
            crate::core::self_evolve::strategy_runtime::ToolStrategyProfile,
        >(snapshot_key, &snapshot)?
        .ok_or_else(|| anyhow!("No tool-strategy baseline snapshot available for rollback"))?;
        self.set_kv_txn(
            &txn,
            crate::core::self_evolve::strategy_runtime::TOOL_STRATEGY_PROFILE_KEY,
            &snapshot,
        )
        .await?;

        let canary_key = crate::core::self_evolve::strategy_runtime::TOOL_STRATEGY_CANARY_STATE_KEY;
        let mut canary_state = self
            .load_kv_json_txn::<crate::core::self_evolve::strategy_runtime::CanaryRolloutState>(
                &txn, canary_key,
            )
            .await?
            .unwrap_or_default();
        canary_state.enabled = false;
        canary_state.baseline_version = restored_profile.version.clone();
        canary_state.candidate_version = restored_profile.version.clone();
        self.set_kv_json_txn(&txn, canary_key, &canary_state)
            .await?;

        txn.commit().await?;
        Ok(restored_profile.version)
    }

    pub async fn learning_queue_counts(&self) -> Result<LearningQueueCounts> {
        let provisional_runs = experience_run::Entity::find()
            .filter(experience_run::Column::SuccessState.eq("provisional"))
            .count(&self.db)
            .await?;
        let pending_consolidation = experience_run::Entity::find()
            .filter(experience_run::Column::Consolidated.eq(false))
            .filter(
                Condition::any()
                    .add(experience_run::Column::SuccessState.ne("provisional"))
                    .add(experience_run::Column::CorrectionState.eq("corrected")),
            )
            .count(&self.db)
            .await?;
        let draft_candidates = learning_candidate::Entity::find()
            .filter(learning_candidate::Column::ApprovalStatus.eq("draft"))
            .count(&self.db)
            .await?;
        let pending_reflection = experience_run::Entity::find()
            .filter(experience_run::Column::Consolidated.eq(true))
            .filter(experience_run::Column::HeuristicReflected.eq(false))
            .filter(
                Condition::any()
                    .add(experience_run::Column::HeuristicReflectionStatus.is_null())
                    .add(experience_run::Column::HeuristicReflectionStatus.eq("pending")),
            )
            .count(&self.db)
            .await?;
        let active_patterns = procedural_pattern::Entity::find()
            .filter(procedural_pattern::Column::Status.eq("active"))
            .count(&self.db)
            .await?;
        Ok(LearningQueueCounts {
            provisional_runs,
            pending_consolidation,
            pending_reflection,
            draft_candidates,
            active_patterns,
        })
    }
}
