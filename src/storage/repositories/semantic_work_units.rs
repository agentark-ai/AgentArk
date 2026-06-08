use super::super::*;

impl Storage {
    // ==================== Semantic Work Units ====================

    /// Upsert a derived semantic work unit used by reflection and clustering.
    pub async fn upsert_semantic_work_unit(&self, unit: &semantic_work_unit::Model) -> Result<()> {
        let txn = self.db.begin().await?;
        semantic_work_unit::Entity::insert(Self::semantic_work_unit_active_model(unit)?)
            .on_conflict(
                OnConflict::column(semantic_work_unit::Column::Id)
                    .update_columns(SEMANTIC_WORK_UNIT_LIGHT_UPSERT_COLUMNS.iter().copied())
                    .to_owned(),
            )
            .exec(&txn)
            .await?;
        let current = semantic_work_unit::Entity::find_by_id(unit.id.clone())
            .lock_exclusive()
            .one(&txn)
            .await?
            .ok_or_else(|| anyhow!("Semantic work unit '{}' missing after upsert", unit.id))?;
        if let Some(model) = Self::semantic_work_unit_heavy_update_active_model(&current, unit)? {
            model.update(&txn).await?;
        }
        txn.commit().await?;
        Ok(())
    }

    /// Read derived semantic work units for a time window.
    pub async fn list_semantic_work_units_between(
        &self,
        from: &str,
        to: &str,
        limit: u64,
    ) -> Result<Vec<semantic_work_unit::Model>> {
        let rows = semantic_work_unit::Entity::find()
            .filter(semantic_work_unit::Column::OccurredAt.gte(from.to_string()))
            .filter(semantic_work_unit::Column::OccurredAt.lt(to.to_string()))
            .order_by_desc(semantic_work_unit::Column::OccurredAt)
            .limit(Self::db_limit(limit))
            .all(&self.db)
            .await?;
        Ok(rows
            .into_iter()
            .map(Self::decrypt_semantic_work_unit)
            .collect())
    }

    pub async fn get_semantic_work_unit(
        &self,
        id: &str,
    ) -> Result<Option<semantic_work_unit::Model>> {
        Ok(semantic_work_unit::Entity::find_by_id(id.to_string())
            .one(&self.db)
            .await?
            .map(Self::decrypt_semantic_work_unit))
    }

    /// Find semantically similar derived work units outside a selected time window.
    /// This is intentionally bounded and backed by the semantic_work_units HNSW
    /// index so Reflect can compare the current recap to history without
    /// scanning old rows.
    pub async fn nearest_semantic_work_units_outside_window(
        &self,
        embedding: &PgVector,
        from: &str,
        to: &str,
        exclude_ids: &[String],
        limit: u64,
    ) -> Result<Vec<(semantic_work_unit::Model, f64)>> {
        if limit == 0 || self.db.get_database_backend() != DbBackend::Postgres {
            return Ok(Vec::new());
        }
        let embedding_sql = pgvector_sql_literal(embedding);
        let exclude_clause = if exclude_ids.is_empty() {
            String::new()
        } else {
            format!("AND id NOT IN ({})", sql_string_list(exclude_ids))
        };
        let sql = format!(
            "SELECT id, embedding <=> {embedding_sql} AS cosine_distance \
             FROM semantic_work_units \
             WHERE embedding IS NOT NULL \
               AND (occurred_at < {from} OR occurred_at >= {to}) \
               {exclude_clause} \
             ORDER BY embedding <=> {embedding_sql} ASC, occurred_at DESC \
             LIMIT {}",
            Self::db_limit(limit),
            from = sql_string_literal(from),
            to = sql_string_literal(to),
        );
        let rows = self
            .db
            .query_all(Statement::from_string(DbBackend::Postgres, sql))
            .await?;
        let mut scored = Vec::<(String, f64)>::with_capacity(rows.len());
        for row in rows {
            let id: String = row.try_get("", "id")?;
            let distance: f64 = row.try_get("", "cosine_distance")?;
            scored.push((id, distance));
        }
        if scored.is_empty() {
            return Ok(Vec::new());
        }
        let ids = scored.iter().map(|(id, _)| id.clone()).collect::<Vec<_>>();
        let models = semantic_work_unit::Entity::find()
            .filter(semantic_work_unit::Column::Id.is_in(ids))
            .all(&self.db)
            .await?;
        let mut by_id = models
            .into_iter()
            .map(|model| {
                let model = Self::decrypt_semantic_work_unit(model);
                (model.id.clone(), model)
            })
            .collect::<std::collections::HashMap<_, _>>();
        Ok(scored
            .into_iter()
            .filter_map(|(id, distance)| by_id.remove(&id).map(|model| (model, distance)))
            .collect())
    }

    #[allow(dead_code)]
    pub async fn delete_semantic_work_units_before(&self, cutoff: &str) -> Result<u64> {
        let result = semantic_work_unit::Entity::delete_many()
            .filter(semantic_work_unit::Column::OccurredAt.lt(cutoff.to_string()))
            .exec(&self.db)
            .await?;
        Ok(result.rows_affected)
    }

    pub async fn delete_semantic_work_units_for_source(
        &self,
        source_kind: &str,
        source_id: &str,
    ) -> Result<u64> {
        let result = semantic_work_unit::Entity::delete_many()
            .filter(semantic_work_unit::Column::SourceKind.eq(source_kind.to_string()))
            .filter(semantic_work_unit::Column::SourceId.eq(source_id.to_string()))
            .exec(&self.db)
            .await?;
        Ok(result.rows_affected)
    }

    pub async fn delete_semantic_work_units_for_source_prefix(
        &self,
        source_kind: &str,
        source_id_prefix: &str,
    ) -> Result<u64> {
        let prefix = source_id_prefix.trim();
        if prefix.is_empty() {
            return Ok(0);
        }
        let result = semantic_work_unit::Entity::delete_many()
            .filter(semantic_work_unit::Column::SourceKind.eq(source_kind.to_string()))
            .filter(semantic_work_unit::Column::SourceId.contains(format!("{}:", prefix)))
            .exec(&self.db)
            .await?;
        Ok(result.rows_affected)
    }

    #[allow(dead_code)]
    pub async fn delete_semantic_work_units_for_conversation(
        &self,
        conversation_id: &str,
    ) -> Result<u64> {
        let result = semantic_work_unit::Entity::delete_many()
            .filter(
                Condition::any()
                    .add(semantic_work_unit::Column::ConversationId.eq(conversation_id.to_string()))
                    .add(
                        Condition::all()
                            .add(semantic_work_unit::Column::SourceKind.eq("conversation"))
                            .add(
                                semantic_work_unit::Column::SourceId
                                    .eq(conversation_id.to_string()),
                            ),
                    ),
            )
            .exec(&self.db)
            .await?;
        Ok(result.rows_affected)
    }
}
