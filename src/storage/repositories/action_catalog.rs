use super::super::*;

impl Storage {
    // ==================== Action Catalog Semantic Index ====================

    pub async fn action_catalog_index_entries(
        &self,
        action_names: &[String],
    ) -> Result<HashMap<String, action_catalog_index::Model>> {
        if action_names.is_empty() {
            return Ok(HashMap::new());
        }

        let rows = action_catalog_index::Entity::find()
            .filter(action_catalog_index::Column::ActionName.is_in(action_names.to_vec()))
            .all(&self.db)
            .await?;

        Ok(rows
            .into_iter()
            .map(|row| (row.action_name.clone(), row))
            .collect())
    }

    #[allow(dead_code)]
    pub async fn nearest_action_catalog_index_entries(
        &self,
        embedding: &PgVector,
        limit: u64,
    ) -> Result<Vec<(action_catalog_index::Model, f64)>> {
        if limit == 0 {
            return Ok(Vec::new());
        }
        if self.db.get_database_backend() != DbBackend::Postgres {
            return Ok(Vec::new());
        }

        let embedding_sql = pgvector_sql_literal(embedding);
        let sql = format!(
            "SELECT action_name, embedding <=> {embedding_sql} AS cosine_distance \
             FROM action_catalog_index \
             WHERE enabled = true \
               AND embedding IS NOT NULL \
             ORDER BY embedding <=> {embedding_sql} ASC \
             LIMIT {}",
            Self::db_limit(limit),
        );
        let rows = self
            .db
            .query_all(Statement::from_string(DbBackend::Postgres, sql))
            .await?;
        let mut scored = Vec::with_capacity(rows.len());
        for row in rows {
            let action_name: String = row.try_get("", "action_name")?;
            let distance: f64 = row.try_get("", "cosine_distance")?;
            scored.push((action_name, distance));
        }
        if scored.is_empty() {
            return Ok(Vec::new());
        }

        let action_names = scored
            .iter()
            .map(|(action_name, _)| action_name.clone())
            .collect::<Vec<_>>();
        let models = action_catalog_index::Entity::find()
            .filter(action_catalog_index::Column::ActionName.is_in(action_names))
            .all(&self.db)
            .await?;
        let mut by_name: HashMap<String, action_catalog_index::Model> = models
            .into_iter()
            .map(|model| (model.action_name.clone(), model))
            .collect();
        Ok(scored
            .into_iter()
            .filter_map(|(action_name, distance)| {
                by_name.remove(&action_name).map(|model| (model, distance))
            })
            .collect())
    }

    pub async fn upsert_action_catalog_index_entry(
        &self,
        entry: &ActionCatalogIndexEntry,
    ) -> Result<()> {
        let now = chrono::Utc::now().to_rfc3339();
        action_catalog_index::Entity::insert(action_catalog_index::ActiveModel {
            action_name: Set(entry.action_name.clone()),
            source: Set(entry.source.clone()),
            version: Set(entry.version.clone()),
            descriptor_hash: Set(entry.descriptor_hash.clone()),
            descriptor_text: Set(entry.descriptor_text.clone()),
            enabled: Set(entry.enabled),
            metadata_json: Set(entry.metadata_json.clone()),
            embedding: Set(entry.embedding.clone()),
            updated_at: Set(now),
        })
        .on_conflict(
            OnConflict::column(action_catalog_index::Column::ActionName)
                .update_columns([
                    action_catalog_index::Column::Source,
                    action_catalog_index::Column::Version,
                    action_catalog_index::Column::DescriptorHash,
                    action_catalog_index::Column::DescriptorText,
                    action_catalog_index::Column::Enabled,
                    action_catalog_index::Column::MetadataJson,
                    action_catalog_index::Column::Embedding,
                    action_catalog_index::Column::UpdatedAt,
                ])
                .to_owned(),
        )
        .exec(&self.db)
        .await?;
        Ok(())
    }

    pub async fn mark_unavailable_action_catalog_entries_disabled(
        &self,
        available_action_names: &[String],
    ) -> Result<u64> {
        let now = chrono::Utc::now().to_rfc3339();
        let filter = if available_action_names.is_empty() {
            "enabled = true".to_string()
        } else {
            format!(
                "enabled = true AND action_name NOT IN ({})",
                sql_string_list(available_action_names)
            )
        };
        let sql = format!(
            "UPDATE action_catalog_index \
             SET enabled = false, updated_at = {} \
             WHERE {}",
            sql_string_literal(&now),
            filter
        );
        let result = self
            .db
            .execute(Statement::from_string(DbBackend::Postgres, sql))
            .await?;
        Ok(result.rows_affected())
    }
}
