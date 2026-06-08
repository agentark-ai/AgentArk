use super::super::*;

impl Storage {
    /// Get a value from the key-value store
    pub async fn get(&self, key: &str) -> Result<Option<Vec<u8>>> {
        let result = kv_store::Entity::find_by_id(key.to_string())
            .one(&self.db)
            .await?;

        Ok(result.map(|m| m.value))
    }

    /// Set a value in the key-value store
    pub async fn set(&self, key: &str, value: &[u8]) -> Result<()> {
        let now = chrono::Utc::now().to_rfc3339();
        kv_store::Entity::insert(kv_store::ActiveModel {
            key: Set(key.to_string()),
            value: Set(value.to_vec()),
            created_at: Set(now.clone()),
            updated_at: Set(now),
        })
        .on_conflict(
            OnConflict::column(kv_store::Column::Key)
                .update_columns([kv_store::Column::Value, kv_store::Column::UpdatedAt])
                .to_owned(),
        )
        .exec(&self.db)
        .await?;
        Ok(())
    }

    /// Delete a key from the store
    pub async fn delete(&self, key: &str) -> Result<()> {
        kv_store::Entity::delete_by_id(key.to_string())
            .exec(&self.db)
            .await?;
        Ok(())
    }

    pub(super) async fn ensure_kv_row_exists_txn(
        &self,
        txn: &DatabaseTransaction,
        key: &str,
    ) -> Result<()> {
        let now = chrono::Utc::now().to_rfc3339();
        match kv_store::Entity::insert(kv_store::ActiveModel {
            key: Set(key.to_string()),
            value: Set(Vec::new()),
            created_at: Set(now.clone()),
            updated_at: Set(now),
        })
        .on_conflict(
            OnConflict::column(kv_store::Column::Key)
                .do_nothing()
                .to_owned(),
        )
        .exec(txn)
        .await
        {
            Ok(_) | Err(sea_orm::DbErr::RecordNotInserted) => {}
            Err(error) => return Err(error.into()),
        }
        Ok(())
    }

    pub(super) async fn get_kv_for_update_txn(
        &self,
        txn: &DatabaseTransaction,
        key: &str,
    ) -> Result<Option<kv_store::Model>> {
        let sql = format!(
            "SELECT key, value, created_at, updated_at FROM kv_store WHERE key = {} FOR UPDATE",
            sql_string_literal(key)
        );
        let row = txn
            .query_one(Statement::from_string(DbBackend::Postgres, sql))
            .await?;
        let Some(row) = row else {
            return Ok(None);
        };
        Ok(Some(kv_store::Model {
            key: row.try_get("", "key")?,
            value: row.try_get("", "value")?,
            created_at: row.try_get("", "created_at")?,
            updated_at: row.try_get("", "updated_at")?,
        }))
    }

    pub(super) async fn set_kv_txn(
        &self,
        txn: &DatabaseTransaction,
        key: &str,
        value: &[u8],
    ) -> Result<()> {
        let now = chrono::Utc::now().to_rfc3339();
        kv_store::Entity::insert(kv_store::ActiveModel {
            key: Set(key.to_string()),
            value: Set(value.to_vec()),
            created_at: Set(now.clone()),
            updated_at: Set(now),
        })
        .on_conflict(
            OnConflict::column(kv_store::Column::Key)
                .update_columns([kv_store::Column::Value, kv_store::Column::UpdatedAt])
                .to_owned(),
        )
        .exec(txn)
        .await?;
        Ok(())
    }

    pub(super) async fn delete_kv_txn(&self, txn: &DatabaseTransaction, key: &str) -> Result<()> {
        kv_store::Entity::delete_by_id(key.to_string())
            .exec(txn)
            .await?;
        Ok(())
    }

    pub(super) async fn load_kv_json_txn<T>(
        &self,
        txn: &DatabaseTransaction,
        key: &str,
    ) -> Result<Option<T>>
    where
        T: serde::de::DeserializeOwned,
    {
        let row = self.get_kv_for_update_txn(txn, key).await?;
        match row {
            Some(row) => parse_kv_json_value(key, &row.value),
            None => Ok(None),
        }
    }

    pub(super) async fn set_kv_json_txn<T>(
        &self,
        txn: &DatabaseTransaction,
        key: &str,
        value: &T,
    ) -> Result<()>
    where
        T: serde::Serialize,
    {
        let raw = serde_json::to_vec(value).with_context(|| {
            format!("Failed to serialize kv_store JSON payload for key '{key}'")
        })?;
        self.set_kv_txn(txn, key, &raw).await
    }

    pub(super) async fn load_learning_candidate_txn(
        &self,
        txn: &DatabaseTransaction,
        id: &str,
    ) -> Result<Option<learning_candidate::Model>> {
        Ok(learning_candidate::Entity::find_by_id(id.to_string())
            .one(txn)
            .await?)
    }

    pub(super) async fn update_learning_candidate_review_txn(
        &self,
        txn: &DatabaseTransaction,
        id: &str,
        approval_status: &str,
        review_notes: Option<&str>,
        approved_ref: Option<&str>,
    ) -> Result<()> {
        let now = chrono::Utc::now().to_rfc3339();
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
                Expr::value(Some(now.clone())),
            )
            .col_expr(
                learning_candidate::Column::ApprovedRef,
                Expr::value(approved_ref.map(|value| value.to_string())),
            )
            .col_expr(learning_candidate::Column::UpdatedAt, Expr::value(now))
            .filter(learning_candidate::Column::Id.eq(id))
            .exec(txn)
            .await?;
        Ok(())
    }

    pub(super) async fn require_kv_lease_guard_txn(
        &self,
        txn: &DatabaseTransaction,
        key: &str,
        guard: &KvLeaseGuard,
    ) -> Result<bool> {
        self.ensure_kv_row_exists_txn(txn, key).await?;
        let Some(row) = self.get_kv_for_update_txn(txn, key).await? else {
            return Ok(false);
        };
        let Some(record) = serde_json::from_slice::<KvLeaseRecord>(&row.value).ok() else {
            return Ok(false);
        };
        Ok(kv_lease_guard_is_current(
            &record,
            guard,
            chrono::Utc::now(),
        ))
    }

    pub(super) async fn upsert_learning_candidate_txn(
        &self,
        txn: &DatabaseTransaction,
        candidate: &learning_candidate::Model,
    ) -> Result<()> {
        learning_candidate::Entity::insert(learning_candidate::ActiveModel {
            id: Set(candidate.id.clone()),
            candidate_type: Set(candidate.candidate_type.clone()),
            subject_key: Set(candidate.subject_key.clone()),
            title: Set(candidate.title.clone()),
            summary: Set(candidate.summary.clone()),
            project_id: Set(candidate.project_id.clone()),
            conversation_id: Set(candidate.conversation_id.clone()),
            pattern_id: Set(candidate.pattern_id.clone()),
            evidence_refs: Set(candidate.evidence_refs.clone()),
            proposed_content: Set(candidate.proposed_content.clone()),
            confidence: Set(candidate.confidence),
            approval_status: Set(candidate.approval_status.clone()),
            review_notes: Set(candidate.review_notes.clone()),
            reviewed_at: Set(candidate.reviewed_at.clone()),
            approved_ref: Set(candidate.approved_ref.clone()),
            created_at: Set(candidate.created_at.clone()),
            updated_at: Set(candidate.updated_at.clone()),
        })
        .on_conflict(
            OnConflict::column(learning_candidate::Column::Id)
                .update_columns([
                    learning_candidate::Column::CandidateType,
                    learning_candidate::Column::SubjectKey,
                    learning_candidate::Column::Title,
                    learning_candidate::Column::Summary,
                    learning_candidate::Column::ProjectId,
                    learning_candidate::Column::ConversationId,
                    learning_candidate::Column::PatternId,
                    learning_candidate::Column::EvidenceRefs,
                    learning_candidate::Column::ProposedContent,
                    learning_candidate::Column::Confidence,
                    learning_candidate::Column::UpdatedAt,
                ])
                .to_owned(),
        )
        .exec(txn)
        .await?;
        Ok(())
    }

    pub async fn acquire_kv_lease(&self, key: &str, owner_id: &str, ttl_secs: i64) -> Result<bool> {
        let ttl_secs = ttl_secs.max(1);
        let txn = self.db.begin().await?;
        self.ensure_kv_row_exists_txn(&txn, key).await?;
        let existing = self.get_kv_for_update_txn(&txn, key).await?;
        let now = chrono::Utc::now();
        let lease = existing
            .as_ref()
            .and_then(|row| serde_json::from_slice::<KvLeaseRecord>(&row.value).ok());
        if lease
            .as_ref()
            .is_some_and(|record| lease_is_active(record, now) && record.owner_id != owner_id)
        {
            txn.rollback().await?;
            return Ok(false);
        }

        let next = KvLeaseRecord {
            owner_id: owner_id.to_string(),
            acquired_at: now.to_rfc3339(),
            expires_at: (now + chrono::Duration::seconds(ttl_secs)).to_rfc3339(),
            fence_token: next_lease_fence_token(lease.as_ref()),
        };
        let raw = serde_json::to_vec(&next)?;
        self.set_kv_txn(&txn, key, &raw).await?;
        txn.commit().await?;
        Ok(true)
    }

    pub async fn release_kv_lease(&self, key: &str, owner_id: &str) -> Result<()> {
        let txn = self.db.begin().await?;
        self.ensure_kv_row_exists_txn(&txn, key).await?;
        let existing = self.get_kv_for_update_txn(&txn, key).await?;
        let lease = existing
            .as_ref()
            .and_then(|row| serde_json::from_slice::<KvLeaseRecord>(&row.value).ok());
        if lease
            .as_ref()
            .is_some_and(|record| record.owner_id == owner_id)
        {
            self.delete_kv_txn(&txn, key).await?;
        }
        txn.commit().await?;
        Ok(())
    }

    pub async fn acquire_kv_lease_guard(
        &self,
        key: &str,
        owner_id: &str,
        ttl_secs: i64,
    ) -> Result<Option<KvLeaseGuard>> {
        let ttl_secs = ttl_secs.max(1);
        let txn = self.db.begin().await?;
        self.ensure_kv_row_exists_txn(&txn, key).await?;
        let existing = self.get_kv_for_update_txn(&txn, key).await?;
        let now = chrono::Utc::now();
        let lease = existing
            .as_ref()
            .and_then(|row| serde_json::from_slice::<KvLeaseRecord>(&row.value).ok());
        if lease
            .as_ref()
            .is_some_and(|record| lease_is_active(record, now) && record.owner_id != owner_id)
        {
            txn.rollback().await?;
            return Ok(None);
        }

        let fence_token = next_lease_fence_token(lease.as_ref());
        let next = KvLeaseRecord {
            owner_id: owner_id.to_string(),
            acquired_at: now.to_rfc3339(),
            expires_at: (now + chrono::Duration::seconds(ttl_secs)).to_rfc3339(),
            fence_token,
        };
        let raw = serde_json::to_vec(&next)?;
        self.set_kv_txn(&txn, key, &raw).await?;
        txn.commit().await?;
        Ok(Some(KvLeaseGuard {
            owner_id: owner_id.to_string(),
            fence_token,
        }))
    }

    pub async fn refresh_kv_lease_guard(
        &self,
        key: &str,
        guard: &KvLeaseGuard,
        ttl_secs: i64,
    ) -> Result<bool> {
        let ttl_secs = ttl_secs.max(1);
        let txn = self.db.begin().await?;
        self.ensure_kv_row_exists_txn(&txn, key).await?;
        let existing = self.get_kv_for_update_txn(&txn, key).await?;
        let now = chrono::Utc::now();
        let Some(lease) = existing
            .as_ref()
            .and_then(|row| serde_json::from_slice::<KvLeaseRecord>(&row.value).ok())
        else {
            txn.rollback().await?;
            return Ok(false);
        };
        if !kv_lease_guard_is_current(&lease, guard, now) {
            txn.rollback().await?;
            return Ok(false);
        }
        let refreshed = KvLeaseRecord {
            owner_id: lease.owner_id,
            acquired_at: lease.acquired_at,
            expires_at: (now + chrono::Duration::seconds(ttl_secs)).to_rfc3339(),
            fence_token: lease.fence_token,
        };
        let raw = serde_json::to_vec(&refreshed)?;
        self.set_kv_txn(&txn, key, &raw).await?;
        txn.commit().await?;
        Ok(true)
    }

    pub async fn release_kv_lease_guard(&self, key: &str, guard: &KvLeaseGuard) -> Result<()> {
        let txn = self.db.begin().await?;
        self.ensure_kv_row_exists_txn(&txn, key).await?;
        let existing = self.get_kv_for_update_txn(&txn, key).await?;
        let lease = existing
            .as_ref()
            .and_then(|row| serde_json::from_slice::<KvLeaseRecord>(&row.value).ok());
        if lease.as_ref().is_some_and(|record| {
            record.owner_id == guard.owner_id && record.fence_token == guard.fence_token
        }) {
            self.delete_kv_txn(&txn, key).await?;
        }
        txn.commit().await?;
        Ok(())
    }
}
