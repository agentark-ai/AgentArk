use super::super::*;

impl Storage {
    // ==================== Documents ====================

    pub(super) fn user_visible_document_filter() -> Condition {
        // Internal AgentArk manuals/capability docs are indexed for retrieval,
        // but they are not user-uploaded library documents.
        Condition::all()
            .add(
                document::Column::Id
                    .starts_with(crate::core::knowledge::agentark_knowledge::DOCUMENT_ID_PREFIX.to_string())
                    .not(),
            )
            .add(
                document::Column::ContentType
                    .starts_with(
                        crate::core::knowledge::agentark_knowledge::INTERNAL_DOCUMENT_CONTENT_TYPE_PREFIX
                            .to_string(),
                    )
                    .not(),
            )
    }

    pub(super) fn document_is_user_visible_after_load(doc: &document::Model) -> bool {
        !crate::core::knowledge::agentark_knowledge::is_agentark_knowledge_document_id(&doc.id)
            && !crate::core::knowledge::agentark_knowledge::is_internal_agentark_document_content_type(
                &doc.content_type,
            )
    }

    /// Insert a document and all chunks atomically so partial uploads do not leak
    /// into the searchable document library.
    pub async fn insert_document_with_chunks(
        &self,
        doc: &document::Model,
        chunks: &[document_chunk::Model],
    ) -> Result<()> {
        let txn = self.db.begin().await?;
        let filename = encrypt_storage_string(&doc.filename)?;
        document::ActiveModel {
            id: Set(doc.id.clone()),
            filename: Set(filename),
            content_type: Set(doc.content_type.clone()),
            project_id: Set(doc.project_id.clone()),
            chunk_count: Set(doc.chunk_count),
            file_size: Set(doc.file_size),
            created_at: Set(doc.created_at.clone()),
        }
        .insert(&txn)
        .await?;

        for chunk in chunks {
            let content = encrypt_storage_string(&chunk.content)?;
            document_chunk::ActiveModel {
                id: Set(chunk.id.clone()),
                document_id: Set(chunk.document_id.clone()),
                chunk_index: Set(chunk.chunk_index),
                content: Set(content),
                embedding: Set(chunk.embedding.clone()),
            }
            .insert(&txn)
            .await?;
        }

        txn.commit().await?;
        Ok(())
    }

    /// Replace a deterministic internal document set and its chunks atomically.
    pub async fn replace_documents_by_id_prefix(
        &self,
        id_prefix: &str,
        documents: &[(document::Model, Vec<document_chunk::Model>)],
    ) -> Result<()> {
        let id_prefix = id_prefix.trim();
        if id_prefix.is_empty() {
            anyhow::bail!("document id prefix cannot be empty");
        }

        let txn = self.db.begin().await?;
        let pattern = format!("{id_prefix}%");
        let delete_chunks_sql = format!(
            "DELETE FROM document_chunks WHERE document_id LIKE {}",
            sql_string_literal(&pattern)
        );
        txn.execute(Statement::from_string(
            DbBackend::Postgres,
            delete_chunks_sql,
        ))
        .await?;
        let delete_docs_sql = format!(
            "DELETE FROM documents WHERE id LIKE {}",
            sql_string_literal(&pattern)
        );
        txn.execute(Statement::from_string(DbBackend::Postgres, delete_docs_sql))
            .await?;

        for (doc, chunks) in documents {
            let filename = encrypt_storage_string(&doc.filename)?;
            document::ActiveModel {
                id: Set(doc.id.clone()),
                filename: Set(filename),
                content_type: Set(doc.content_type.clone()),
                project_id: Set(doc.project_id.clone()),
                chunk_count: Set(doc.chunk_count),
                file_size: Set(doc.file_size),
                created_at: Set(doc.created_at.clone()),
            }
            .insert(&txn)
            .await?;

            for chunk in chunks {
                let content = encrypt_storage_string(&chunk.content)?;
                document_chunk::ActiveModel {
                    id: Set(chunk.id.clone()),
                    document_id: Set(chunk.document_id.clone()),
                    chunk_index: Set(chunk.chunk_index),
                    content: Set(content),
                    embedding: Set(chunk.embedding.clone()),
                }
                .insert(&txn)
                .await?;
            }
        }

        txn.commit().await?;
        Ok(())
    }

    /// List documents (paginated)
    pub async fn list_documents(
        &self,
        limit: u64,
        offset: u64,
        project_id: Option<&str>,
    ) -> Result<Vec<document::Model>> {
        let mut query = document::Entity::find().order_by_desc(document::Column::CreatedAt);
        query = query.filter(Self::user_visible_document_filter());
        if let Some(pid) = project_id {
            query = query.filter(document::Column::ProjectId.eq(pid));
        }
        let mut docs = query
            .limit(Self::db_limit(limit))
            .offset(Self::db_offset(offset))
            .all(&self.db)
            .await?;
        for doc in &mut docs {
            doc.filename = decrypt_storage_string(&doc.filename);
        }
        docs.retain(Self::document_is_user_visible_after_load);
        Ok(docs)
    }

    /// Get a single user-visible document by id.
    pub async fn get_document(&self, id: &str) -> Result<Option<document::Model>> {
        let id = id.trim();
        if id.is_empty() {
            return Ok(None);
        }
        let Some(mut doc) = document::Entity::find_by_id(id.to_string())
            .filter(Self::user_visible_document_filter())
            .one(&self.db)
            .await?
        else {
            return Ok(None);
        };
        doc.filename = decrypt_storage_string(&doc.filename);
        if !Self::document_is_user_visible_after_load(&doc) {
            return Ok(None);
        }
        Ok(Some(doc))
    }

    /// Count documents
    pub async fn count_documents(&self, project_id: Option<&str>) -> Result<u64> {
        let mut query = document::Entity::find().filter(Self::user_visible_document_filter());
        if let Some(pid) = project_id {
            query = query.filter(document::Column::ProjectId.eq(pid));
        }
        Ok(query.count(&self.db).await?)
    }

    /// Count document chunks across all documents.
    pub async fn count_document_chunks(&self) -> Result<u64> {
        Ok(document_chunk::Entity::find().count(&self.db).await?)
    }

    /// List a bounded set of documents for metadata search.
    pub async fn list_documents_for_search(
        &self,
        project_id: Option<&str>,
    ) -> Result<Vec<document::Model>> {
        let mut query = document::Entity::find().order_by_desc(document::Column::CreatedAt);
        query = query.filter(Self::user_visible_document_filter());
        if let Some(pid) = project_id {
            query = query.filter(
                Condition::any()
                    .add(document::Column::ProjectId.eq(pid))
                    .add(document::Column::ProjectId.is_null()),
            );
        }
        let mut docs = query
            .limit(Self::MAX_DOCUMENTS_FOR_SEARCH)
            .all(&self.db)
            .await?;
        for doc in &mut docs {
            doc.filename = decrypt_storage_string(&doc.filename);
        }
        docs.retain(Self::document_is_user_visible_after_load);
        Ok(docs)
    }

    /// List deterministic internal documents by id prefix for scoped retrieval.
    pub async fn list_documents_by_id_prefix(
        &self,
        id_prefix: &str,
        limit: u64,
    ) -> Result<Vec<document::Model>> {
        let id_prefix = id_prefix.trim();
        if id_prefix.is_empty() || limit == 0 {
            return Ok(Vec::new());
        }
        let mut docs = document::Entity::find()
            .filter(document::Column::Id.starts_with(id_prefix.to_string()))
            .order_by_desc(document::Column::CreatedAt)
            .limit(Self::db_limit(limit))
            .all(&self.db)
            .await?;
        for doc in &mut docs {
            doc.filename = decrypt_storage_string(&doc.filename);
        }
        Ok(docs)
    }

    /// Get document chunks for search
    pub async fn get_document_chunks(
        &self,
        document_id: &str,
    ) -> Result<Vec<document_chunk::Model>> {
        let visible = document::Entity::find_by_id(document_id.to_string())
            .filter(Self::user_visible_document_filter())
            .one(&self.db)
            .await?
            .is_some();
        if !visible {
            return Ok(Vec::new());
        }

        let mut chunks = document_chunk::Entity::find()
            .filter(document_chunk::Column::DocumentId.eq(document_id))
            .order_by_asc(document_chunk::Column::ChunkIndex)
            .all(&self.db)
            .await?;
        for chunk in &mut chunks {
            chunk.content = decrypt_storage_string(&chunk.content);
        }
        Ok(chunks)
    }

    /// Get a bounded document chunk window for background extraction.
    pub async fn list_document_chunks_for_document_window(
        &self,
        document_id: &str,
        min_chunk_index: i32,
        limit: u64,
    ) -> Result<Vec<document_chunk::Model>> {
        let document_id = document_id.trim();
        if document_id.is_empty() || limit == 0 {
            return Ok(Vec::new());
        }
        let visible = document::Entity::find_by_id(document_id.to_string())
            .filter(Self::user_visible_document_filter())
            .one(&self.db)
            .await?
            .is_some();
        if !visible {
            return Ok(Vec::new());
        }

        let mut chunks = document_chunk::Entity::find()
            .filter(document_chunk::Column::DocumentId.eq(document_id.to_string()))
            .filter(document_chunk::Column::ChunkIndex.gte(min_chunk_index.max(0)))
            .order_by_asc(document_chunk::Column::ChunkIndex)
            .limit(Self::db_limit(limit.min(64)))
            .all(&self.db)
            .await?;
        for chunk in &mut chunks {
            chunk.content = decrypt_storage_string(&chunk.content);
        }
        Ok(chunks)
    }

    /// Get document chunks for a bounded set of documents.
    pub async fn list_document_chunks_for_documents(
        &self,
        document_ids: &[String],
    ) -> Result<Vec<document_chunk::Model>> {
        if document_ids.is_empty() {
            return Ok(Vec::new());
        }

        let mut chunks = document_chunk::Entity::find()
            .filter(document_chunk::Column::DocumentId.is_in(document_ids.iter().cloned()))
            .order_by_asc(document_chunk::Column::DocumentId)
            .order_by_asc(document_chunk::Column::ChunkIndex)
            .limit(Self::MAX_DOCUMENT_CHUNKS_FOR_SEARCH)
            .all(&self.db)
            .await?;
        for chunk in &mut chunks {
            chunk.content = decrypt_storage_string(&chunk.content);
        }
        Ok(chunks)
    }

    pub async fn get_document_chunks_by_ids(
        &self,
        ids: &[String],
    ) -> Result<Vec<document_chunk::Model>> {
        if ids.is_empty() {
            return Ok(Vec::new());
        }

        let mut chunks = document_chunk::Entity::find()
            .filter(document_chunk::Column::Id.is_in(ids.iter().cloned()))
            .all(&self.db)
            .await?;
        for chunk in &mut chunks {
            chunk.content = decrypt_storage_string(&chunk.content);
        }

        let mut by_id = chunks
            .into_iter()
            .map(|chunk| (chunk.id.clone(), chunk))
            .collect::<std::collections::HashMap<_, _>>();

        Ok(ids
            .iter()
            .filter_map(|id| by_id.remove(id))
            .collect::<Vec<_>>())
    }

    pub async fn nearest_document_chunk_ids(
        &self,
        query_embedding: &PgVector,
        document_ids: &[String],
        limit: u64,
    ) -> Result<Vec<String>> {
        if limit == 0 || document_ids.is_empty() {
            return Ok(Vec::new());
        }

        let embedding_sql = pgvector_sql_literal(query_embedding);
        let doc_id_list = sql_string_list(document_ids);
        let sql = format!(
            "SELECT c.id \
             FROM document_chunks c \
             INNER JOIN documents d ON d.id = c.document_id \
             WHERE c.embedding IS NOT NULL AND c.document_id IN ({doc_id_list}) \
             ORDER BY c.embedding <=> {embedding_sql} ASC, d.created_at DESC, c.chunk_index ASC \
             LIMIT {}",
            Self::db_limit(limit)
        );

        let rows = self
            .db
            .query_all(Statement::from_string(DbBackend::Postgres, sql))
            .await?;
        Ok(rows
            .into_iter()
            .filter_map(|row| row.try_get::<String>("", "id").ok())
            .collect())
    }

    pub async fn list_recent_document_chunk_ids(
        &self,
        document_ids: &[String],
        limit: u64,
    ) -> Result<Vec<String>> {
        if limit == 0 || document_ids.is_empty() {
            return Ok(Vec::new());
        }

        let doc_id_list = sql_string_list(document_ids);
        let sql = format!(
            "SELECT c.id \
             FROM document_chunks c \
             INNER JOIN documents d ON d.id = c.document_id \
             WHERE c.document_id IN ({doc_id_list}) \
             ORDER BY d.created_at DESC, c.chunk_index ASC \
             LIMIT {}",
            Self::db_limit(limit)
        );

        let rows = self
            .db
            .query_all(Statement::from_string(DbBackend::Postgres, sql))
            .await?;
        Ok(rows
            .into_iter()
            .filter_map(|row| row.try_get::<String>("", "id").ok())
            .collect())
    }

    pub async fn pgvector_health_check(&self) -> Result<()> {
        if self.db.get_database_backend() != DbBackend::Postgres {
            anyhow::bail!("storage backend is not Postgres");
        }

        let sql = "SELECT '[0,0]'::vector <=> '[0,0]'::vector AS cosine_distance".to_string();
        let row = self
            .db
            .query_one(Statement::from_string(DbBackend::Postgres, sql))
            .await?;

        let row = row.ok_or_else(|| anyhow!("pgvector health check returned no rows"))?;
        let _ = row.try_get::<f64>("", "cosine_distance")?;
        Ok(())
    }

    /// Delete a document and its chunks
    pub async fn delete_document(&self, id: &str) -> Result<()> {
        let txn = self.db.begin().await?;
        let visible = document::Entity::find_by_id(id.to_string())
            .filter(Self::user_visible_document_filter())
            .one(&txn)
            .await?
            .is_some();
        if !visible {
            txn.commit().await?;
            return Ok(());
        }
        document_chunk::Entity::delete_many()
            .filter(document_chunk::Column::DocumentId.eq(id))
            .exec(&txn)
            .await?;
        document::Entity::delete_by_id(id.to_string())
            .exec(&txn)
            .await?;
        txn.commit().await?;
        Ok(())
    }
}
