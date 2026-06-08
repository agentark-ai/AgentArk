use super::super::*;

impl Storage {
    // ==================== Learned Facts ====================

    /// Insert a learned fact into the current experience-item memory store.
    #[cfg(test)]
    pub async fn insert_fact(
        &self,
        id: &str,
        fact: &str,
        confidence: f32,
        sources: &str,
        embedding: Option<PgVector>,
        project_id: Option<&str>,
    ) -> Result<()> {
        let now = chrono::Utc::now().to_rfc3339();
        let scope = if project_id.is_some() {
            "project"
        } else {
            "global"
        };

        self.upsert_experience_item(&experience_item::Model {
            id: id.to_string(),
            kind: "personal_fact".to_string(),
            scope: scope.to_string(),
            project_id: project_id.map(str::to_string),
            conversation_id: None,
            title: "Learned fact".to_string(),
            content: fact.to_string(),
            normalized_key: format!("fact::{}", id),
            confidence: confidence.clamp(0.0, 1.0) as f64,
            support_count: 1,
            contradiction_count: 0,
            status: "active".to_string(),
            metadata: serde_json::json!({ "sources": sources }),
            last_supported_at: Some(now.clone()),
            last_contradicted_at: None,
            created_at: now.clone(),
            updated_at: now,
            embedding,
        })
        .await?;

        Ok(())
    }

    /// Get learned facts from the current experience-item memory store.
    pub async fn get_facts(&self) -> Result<Vec<LearnedFactRecord>> {
        let facts = experience_item::Entity::find()
            .filter(experience_item::Column::Status.eq("active"))
            .filter(experience_item::Column::Kind.is_in(["personal_fact", "constraint"]))
            .order_by_desc(experience_item::Column::UpdatedAt)
            .limit(Self::MAX_FACT_ROWS_PER_QUERY)
            .all(&self.db)
            .await?;
        Ok(facts
            .into_iter()
            .map(learned_fact_from_experience_item)
            .collect())
    }

    /// Get learned facts filtered by project (paginated).
    pub async fn get_facts_by_project(
        &self,
        limit: u64,
        offset: u64,
        project_id: Option<&str>,
    ) -> Result<Vec<LearnedFactRecord>> {
        let mut query = experience_item::Entity::find()
            .filter(experience_item::Column::Status.eq("active"))
            .filter(experience_item::Column::Kind.is_in(["personal_fact", "constraint"]))
            .order_by_desc(experience_item::Column::UpdatedAt);
        if let Some(pid) = project_id {
            query = query.filter(experience_item::Column::ProjectId.eq(pid));
        }
        let facts = query
            .limit(Self::db_limit(limit))
            .offset(Self::db_offset(offset))
            .all(&self.db)
            .await?;
        Ok(facts
            .into_iter()
            .map(learned_fact_from_experience_item)
            .collect())
    }

    pub(super) async fn get_fact_items_by_project_unpaged(
        &self,
        project_id: Option<&str>,
    ) -> Result<Vec<experience_item::Model>> {
        let mut query = experience_item::Entity::find()
            .filter(experience_item::Column::Status.eq("active"))
            .filter(experience_item::Column::Kind.is_in(["personal_fact", "constraint"]))
            .order_by_desc(experience_item::Column::UpdatedAt);
        if let Some(pid) = project_id {
            query = query.filter(experience_item::Column::ProjectId.eq(pid));
        }
        Ok(query
            .limit(Self::MAX_FACT_ROWS_PER_QUERY)
            .all(&self.db)
            .await?)
    }

    /// Get learned memory rows filtered by semantic memory category.
    pub async fn get_facts_by_project_and_category(
        &self,
        limit: u64,
        offset: u64,
        project_id: Option<&str>,
        category: &str,
    ) -> Result<Vec<LearnedFactRecord>> {
        let category = category.trim();
        if category.is_empty() || category == "all" {
            return self.get_facts_by_project(limit, offset, project_id).await;
        }
        let offset = Self::db_offset(offset) as usize;
        let limit = Self::db_limit(limit) as usize;
        let facts = self
            .get_fact_items_by_project_unpaged(project_id)
            .await?
            .into_iter()
            .filter(|item| learned_fact_category_from_metadata(&item.metadata) == category)
            .skip(offset)
            .take(limit)
            .map(learned_fact_from_experience_item)
            .collect();
        Ok(facts)
    }

    /// Count learned facts in the current memory store.
    pub async fn count_facts(&self, project_id: Option<&str>) -> Result<u64> {
        let mut query = experience_item::Entity::find()
            .filter(experience_item::Column::Status.eq("active"))
            .filter(experience_item::Column::Kind.is_in(["personal_fact", "constraint"]));
        if let Some(pid) = project_id {
            query = query.filter(experience_item::Column::ProjectId.eq(pid));
        }
        Ok(query.count(&self.db).await?)
    }

    /// Count learned memory rows filtered by semantic memory category.
    pub async fn count_facts_by_category(
        &self,
        project_id: Option<&str>,
        category: &str,
    ) -> Result<u64> {
        let category = category.trim();
        if category.is_empty() || category == "all" {
            return self.count_facts(project_id).await;
        }
        let count = self
            .get_fact_items_by_project_unpaged(project_id)
            .await?
            .into_iter()
            .filter(|item| learned_fact_category_from_metadata(&item.metadata) == category)
            .count();
        Ok(count as u64)
    }

    // ==================== Tasks ====================

    // ==================== User Preferences ====================

    /// Upsert a user preference in a project scope (or global scope when project_id is None).
    pub async fn upsert_user_preference(
        &self,
        key: &str,
        value: &str,
        confidence: f32,
        source: Option<&str>,
        project_id: Option<&str>,
        sensitivity: Option<&str>,
    ) -> Result<user_preference::Model> {
        let key = key.trim();
        if key.is_empty() {
            anyhow::bail!("Preference key cannot be empty");
        }
        let id = Self::preference_row_id(key, project_id);
        let now = chrono::Utc::now().to_rfc3339();
        let bounded_confidence = confidence.clamp(0.0, 1.0);
        let encrypted_value = encrypt_storage_string(value)?;
        let normalized_project = project_id
            .map(|p| p.trim())
            .filter(|p| !p.is_empty())
            .map(|p| p.to_string());
        let normalized_sensitivity = user_preference::normalize_memory_sensitivity(sensitivity)
            .unwrap_or_else(|| user_preference::classify_user_preference_sensitivity(key, value))
            .as_str()
            .to_string();

        if let Some(existing) = user_preference::Entity::find_by_id(id.clone())
            .one(&self.db)
            .await?
        {
            let mut model: user_preference::ActiveModel = existing.into();
            model.key = Set(key.to_ascii_lowercase());
            model.value = Set(encrypted_value.clone());
            model.sensitivity = Set(normalized_sensitivity);
            model.confidence = Set(bounded_confidence);
            model.source = Set(source.map(|s| s.to_string()));
            model.project_id = Set(normalized_project);
            model.updated_at = Set(now);
            let mut updated = model.update(&self.db).await?;
            updated.value = decrypt_storage_string(&updated.value);
            Ok(updated)
        } else {
            let model = user_preference::ActiveModel {
                id: Set(id),
                key: Set(key.to_ascii_lowercase()),
                value: Set(encrypted_value),
                sensitivity: Set(normalized_sensitivity),
                confidence: Set(bounded_confidence),
                source: Set(source.map(|s| s.to_string())),
                project_id: Set(normalized_project),
                created_at: Set(now.clone()),
                updated_at: Set(now),
            }
            .insert(&self.db)
            .await?;
            let mut model = model;
            model.value = decrypt_storage_string(&model.value);
            Ok(model)
        }
    }

    /// Get a single user preference by key + scope.
    pub async fn get_user_preference(
        &self,
        key: &str,
        project_id: Option<&str>,
    ) -> Result<Option<user_preference::Model>> {
        let key = key.trim();
        if key.is_empty() {
            anyhow::bail!("Preference key cannot be empty");
        }
        let id = Self::preference_row_id(key, project_id);
        let Some(mut model) = user_preference::Entity::find_by_id(id)
            .one(&self.db)
            .await?
        else {
            return Ok(None);
        };
        model.value = decrypt_storage_string(&model.value);
        Ok(Some(model))
    }

    /// List user preferences by scope.
    pub async fn list_user_preferences(
        &self,
        limit: u64,
        offset: u64,
        project_id: Option<&str>,
    ) -> Result<Vec<user_preference::Model>> {
        let mut query = user_preference::Entity::find()
            .filter(
                user_preference::Column::Sensitivity
                    .ne(user_preference::SENSITIVITY_PERSONAL_IDENTIFIER),
            )
            .order_by_desc(user_preference::Column::UpdatedAt);
        if let Some(pid) = project_id {
            query = query.filter(user_preference::Column::ProjectId.eq(pid));
        }
        let mut rows = query
            .limit(Self::db_limit(limit))
            .offset(Self::db_offset(offset))
            .all(&self.db)
            .await?;
        for row in &mut rows {
            row.value = decrypt_storage_string(&row.value);
        }
        Ok(rows)
    }

    /// Count user preferences by scope.
    pub async fn count_user_preferences(&self, project_id: Option<&str>) -> Result<u64> {
        let mut query = user_preference::Entity::find().filter(
            user_preference::Column::Sensitivity
                .ne(user_preference::SENSITIVITY_PERSONAL_IDENTIFIER),
        );
        if let Some(pid) = project_id {
            query = query.filter(user_preference::Column::ProjectId.eq(pid));
        }
        Ok(query.count(&self.db).await?)
    }

    /// Delete a user preference by key + scope.
    pub async fn delete_user_preference(
        &self,
        key: &str,
        project_id: Option<&str>,
    ) -> Result<bool> {
        let id = Self::preference_row_id(key, project_id);
        let result = user_preference::Entity::delete_by_id(id)
            .exec(&self.db)
            .await?;
        Ok(result.rows_affected > 0)
    }

    // ==================== User Data Items ====================

    /// Insert a user data item.
    pub async fn create_user_data_item(
        &self,
        item: NewUserDataItem<'_>,
    ) -> Result<user_data_item::Model> {
        let now = chrono::Utc::now().to_rfc3339();
        let title = encrypt_storage_string(item.title.trim())?;
        let content = encrypt_storage_string(item.content)?;
        let model = user_data_item::ActiveModel {
            id: Set(uuid::Uuid::new_v4().to_string()),
            kind: Set(item.kind.trim().to_string()),
            title: Set(title),
            content: Set(content),
            url: Set(item
                .url
                .map(|v| v.trim().to_string())
                .filter(|v| !v.is_empty())),
            source_channel: Set(item.source_channel.map(|v| v.to_string())),
            conversation_id: Set(item.conversation_id.map(|v| v.to_string())),
            project_id: Set(item.project_id.map(|v| v.to_string())),
            pinned: Set(item.pinned),
            created_at: Set(now.clone()),
            updated_at: Set(now),
        }
        .insert(&self.db)
        .await?;
        let mut model = model;
        model.title = decrypt_storage_string(&model.title);
        model.content = decrypt_storage_string(&model.content);
        Ok(model)
    }

    /// Upsert an auto-captured link into user data (deduped by URL + project scope).
    pub async fn upsert_user_data_link(
        &self,
        url: &str,
        source_channel: Option<&str>,
        conversation_id: Option<&str>,
        project_id: Option<&str>,
    ) -> Result<user_data_item::Model> {
        let normalized_url = url.trim();
        if normalized_url.is_empty()
            || (!normalized_url.starts_with("http://") && !normalized_url.starts_with("https://"))
        {
            anyhow::bail!("Only http/https URLs can be stored as link user-data");
        }

        let now = chrono::Utc::now().to_rfc3339();
        let mut query = user_data_item::Entity::find()
            .filter(user_data_item::Column::Kind.eq("link"))
            .filter(user_data_item::Column::Url.eq(normalized_url.to_string()))
            .order_by_desc(user_data_item::Column::UpdatedAt);

        if let Some(pid) = project_id {
            query = query.filter(user_data_item::Column::ProjectId.eq(pid));
        } else {
            query = query.filter(user_data_item::Column::ProjectId.is_null());
        }

        if let Some(existing) = query.one(&self.db).await? {
            let mut model: user_data_item::ActiveModel = existing.into();
            model.source_channel = Set(source_channel.map(|v| v.to_string()));
            model.conversation_id = Set(conversation_id.map(|v| v.to_string()));
            model.updated_at = Set(now);
            let mut updated = model.update(&self.db).await?;
            updated.title = decrypt_storage_string(&updated.title);
            updated.content = decrypt_storage_string(&updated.content);
            Ok(updated)
        } else {
            let title = Self::default_link_title(normalized_url);
            self.create_user_data_item(NewUserDataItem {
                kind: "link",
                title: &title,
                content: "Auto-saved link from user chat",
                url: Some(normalized_url),
                source_channel,
                conversation_id,
                project_id,
                pinned: false,
            })
            .await
        }
    }

    /// List user data items by scope and optional kind.
    pub async fn list_user_data_items(
        &self,
        limit: u64,
        offset: u64,
        project_id: Option<&str>,
        kind: Option<&str>,
    ) -> Result<Vec<user_data_item::Model>> {
        let mut query =
            user_data_item::Entity::find().order_by_desc(user_data_item::Column::UpdatedAt);
        if let Some(pid) = project_id {
            query = query.filter(user_data_item::Column::ProjectId.eq(pid));
        }
        if let Some(kind_value) = kind.map(|v| v.trim()).filter(|v| !v.is_empty()) {
            query = query.filter(user_data_item::Column::Kind.eq(kind_value));
        }
        let mut rows = query
            .limit(Self::db_limit(limit))
            .offset(Self::db_offset(offset))
            .all(&self.db)
            .await?;
        for row in &mut rows {
            row.title = decrypt_storage_string(&row.title);
            row.content = decrypt_storage_string(&row.content);
        }
        Ok(rows)
    }

    /// Count user data items by scope and optional kind.
    pub async fn count_user_data_items(
        &self,
        project_id: Option<&str>,
        kind: Option<&str>,
    ) -> Result<u64> {
        let mut query = user_data_item::Entity::find();
        if let Some(pid) = project_id {
            query = query.filter(user_data_item::Column::ProjectId.eq(pid));
        }
        if let Some(kind_value) = kind.map(|v| v.trim()).filter(|v| !v.is_empty()) {
            query = query.filter(user_data_item::Column::Kind.eq(kind_value));
        }
        Ok(query.count(&self.db).await?)
    }

    /// Delete a user data item.
    pub async fn delete_user_data_item(&self, id: &str) -> Result<bool> {
        let result = user_data_item::Entity::delete_by_id(id.to_string())
            .exec(&self.db)
            .await?;
        Ok(result.rows_affected > 0)
    }

    // ==================== Knowledge Items ====================

    /// Insert a knowledge base item.
    pub async fn create_knowledge_item(
        &self,
        title: &str,
        content: &str,
        source: Option<&str>,
        url: Option<&str>,
        tags: Option<&str>,
        project_id: Option<&str>,
    ) -> Result<knowledge_item::Model> {
        let now = chrono::Utc::now().to_rfc3339();
        let title = encrypt_storage_string(title.trim())?;
        let content = encrypt_storage_string(content)?;
        let model = knowledge_item::ActiveModel {
            id: Set(uuid::Uuid::new_v4().to_string()),
            title: Set(title),
            content: Set(content),
            source: Set(source.map(|v| v.to_string())),
            url: Set(url.map(|v| v.to_string())),
            tags: Set(tags.map(|v| v.to_string())),
            project_id: Set(project_id.map(|v| v.to_string())),
            created_at: Set(now.clone()),
            updated_at: Set(now),
        }
        .insert(&self.db)
        .await?;
        let mut model = model;
        model.title = decrypt_storage_string(&model.title);
        model.content = decrypt_storage_string(&model.content);
        Ok(model)
    }

    pub(super) fn visible_knowledge_source_filter() -> Condition {
        Condition::any()
            .add(knowledge_item::Column::Source.is_null())
            .add(knowledge_item::Column::Source.is_not_in([
                crate::core::knowledge::agentark_knowledge::CURATED_SOURCE,
                crate::core::knowledge::agentark_knowledge::RUNTIME_SOURCE,
            ]))
    }

    /// List knowledge base items visible in end-user memory UI.
    pub async fn list_visible_knowledge_items(
        &self,
        limit: u64,
        offset: u64,
        project_id: Option<&str>,
    ) -> Result<Vec<knowledge_item::Model>> {
        let mut query = knowledge_item::Entity::find()
            .filter(Self::visible_knowledge_source_filter())
            .order_by_desc(knowledge_item::Column::UpdatedAt);
        if let Some(pid) = project_id {
            query = query.filter(knowledge_item::Column::ProjectId.eq(pid));
        }
        let mut rows = query
            .limit(Self::db_limit(limit))
            .offset(Self::db_offset(offset))
            .all(&self.db)
            .await?;
        for row in &mut rows {
            row.title = decrypt_storage_string(&row.title);
            row.content = decrypt_storage_string(&row.content);
        }
        Ok(rows)
    }

    /// Count knowledge base items visible in end-user memory UI.
    pub async fn count_visible_knowledge_items(&self, project_id: Option<&str>) -> Result<u64> {
        let mut query =
            knowledge_item::Entity::find().filter(Self::visible_knowledge_source_filter());
        if let Some(pid) = project_id {
            query = query.filter(knowledge_item::Column::ProjectId.eq(pid));
        }
        Ok(query.count(&self.db).await?)
    }

    /// Delete all knowledge base items for a specific source.
    pub async fn delete_knowledge_items_by_source(&self, source: &str) -> Result<u64> {
        let result = knowledge_item::Entity::delete_many()
            .filter(knowledge_item::Column::Source.eq(source.to_string()))
            .exec(&self.db)
            .await?;
        Ok(result.rows_affected)
    }

    /// Delete a knowledge base item.
    pub async fn delete_knowledge_item(&self, id: &str) -> Result<bool> {
        let result = knowledge_item::Entity::delete_by_id(id.to_string())
            .exec(&self.db)
            .await?;
        Ok(result.rows_affected > 0)
    }

    pub async fn upsert_knowledge_entity(&self, entity: &knowledge_entity::Model) -> Result<()> {
        knowledge_entity::Entity::insert(knowledge_entity::ActiveModel {
            id: Set(entity.id.clone()),
            entity_type: Set(entity.entity_type.clone()),
            canonical_name: Set(entity.canonical_name.clone()),
            normalized_name: Set(entity.normalized_name.clone()),
            project_id: Set(entity.project_id.clone()),
            status: Set(entity.status.clone()),
            confidence: Set(entity.confidence),
            aliases: Set(entity.aliases.clone()),
            metadata: Set(entity.metadata.clone()),
            first_seen_at: Set(entity.first_seen_at.clone()),
            last_seen_at: Set(entity.last_seen_at.clone()),
            created_at: Set(entity.created_at.clone()),
            updated_at: Set(entity.updated_at.clone()),
        })
        .on_conflict(
            OnConflict::column(knowledge_entity::Column::Id)
                .update_columns([
                    knowledge_entity::Column::EntityType,
                    knowledge_entity::Column::CanonicalName,
                    knowledge_entity::Column::NormalizedName,
                    knowledge_entity::Column::ProjectId,
                    knowledge_entity::Column::Status,
                    knowledge_entity::Column::Confidence,
                    knowledge_entity::Column::Aliases,
                    knowledge_entity::Column::Metadata,
                    knowledge_entity::Column::LastSeenAt,
                    knowledge_entity::Column::UpdatedAt,
                ])
                .to_owned(),
        )
        .exec(&self.db)
        .await?;
        Ok(())
    }

    pub async fn get_knowledge_entity(&self, id: &str) -> Result<Option<knowledge_entity::Model>> {
        Ok(knowledge_entity::Entity::find_by_id(id.to_string())
            .one(&self.db)
            .await?)
    }

    pub async fn list_knowledge_entities_for_graph(
        &self,
        statuses: &[String],
        project_id: Option<&str>,
        limit: u64,
    ) -> Result<Vec<knowledge_entity::Model>> {
        let mut query = knowledge_entity::Entity::find();
        if !statuses.is_empty() {
            query = query.filter(knowledge_entity::Column::Status.is_in(statuses.to_vec()));
        }
        query = match project_id.map(str::trim).filter(|value| !value.is_empty()) {
            Some(pid) => query.filter(
                Condition::any()
                    .add(knowledge_entity::Column::ProjectId.is_null())
                    .add(knowledge_entity::Column::ProjectId.eq(pid.to_string())),
            ),
            None => query.filter(knowledge_entity::Column::ProjectId.is_null()),
        };
        Ok(query
            .order_by_desc(knowledge_entity::Column::UpdatedAt)
            .limit(Self::db_limit(limit.min(500)))
            .all(&self.db)
            .await?)
    }

    pub async fn upsert_knowledge_relation(
        &self,
        relation: &knowledge_relation::Model,
    ) -> Result<()> {
        knowledge_relation::Entity::insert(knowledge_relation::ActiveModel {
            id: Set(relation.id.clone()),
            source_entity_id: Set(relation.source_entity_id.clone()),
            target_entity_id: Set(relation.target_entity_id.clone()),
            relation_type: Set(relation.relation_type.clone()),
            status: Set(relation.status.clone()),
            confidence: Set(relation.confidence),
            project_id: Set(relation.project_id.clone()),
            valid_from: Set(relation.valid_from.clone()),
            valid_until: Set(relation.valid_until.clone()),
            support_count: Set(relation.support_count),
            contradiction_count: Set(relation.contradiction_count),
            metadata: Set(relation.metadata.clone()),
            first_seen_at: Set(relation.first_seen_at.clone()),
            last_seen_at: Set(relation.last_seen_at.clone()),
            created_at: Set(relation.created_at.clone()),
            updated_at: Set(relation.updated_at.clone()),
        })
        .on_conflict(
            OnConflict::column(knowledge_relation::Column::Id)
                .update_columns([
                    knowledge_relation::Column::SourceEntityId,
                    knowledge_relation::Column::TargetEntityId,
                    knowledge_relation::Column::RelationType,
                    knowledge_relation::Column::Status,
                    knowledge_relation::Column::Confidence,
                    knowledge_relation::Column::ProjectId,
                    knowledge_relation::Column::ValidFrom,
                    knowledge_relation::Column::ValidUntil,
                    knowledge_relation::Column::SupportCount,
                    knowledge_relation::Column::ContradictionCount,
                    knowledge_relation::Column::Metadata,
                    knowledge_relation::Column::LastSeenAt,
                    knowledge_relation::Column::UpdatedAt,
                ])
                .to_owned(),
        )
        .exec(&self.db)
        .await?;
        Ok(())
    }

    pub async fn get_knowledge_relation(
        &self,
        id: &str,
    ) -> Result<Option<knowledge_relation::Model>> {
        Ok(knowledge_relation::Entity::find_by_id(id.to_string())
            .one(&self.db)
            .await?)
    }

    pub async fn update_knowledge_relation_status(&self, id: &str, status: &str) -> Result<bool> {
        let Some(row) = knowledge_relation::Entity::find_by_id(id.to_string())
            .one(&self.db)
            .await?
        else {
            return Ok(false);
        };
        let mut model: knowledge_relation::ActiveModel = row.into();
        model.status = Set(status.to_string());
        model.updated_at = Set(chrono::Utc::now().to_rfc3339());
        model.update(&self.db).await?;
        Ok(true)
    }

    pub async fn list_knowledge_relations_for_entities(
        &self,
        entity_ids: &[String],
        statuses: &[String],
        relation_types: &[String],
        limit: u64,
    ) -> Result<Vec<knowledge_relation::Model>> {
        if entity_ids.is_empty() {
            return Ok(Vec::new());
        }
        let mut query = knowledge_relation::Entity::find().filter(
            Condition::any()
                .add(knowledge_relation::Column::SourceEntityId.is_in(entity_ids.to_vec()))
                .add(knowledge_relation::Column::TargetEntityId.is_in(entity_ids.to_vec())),
        );
        if !statuses.is_empty() {
            query = query.filter(knowledge_relation::Column::Status.is_in(statuses.to_vec()));
        }
        if !relation_types.is_empty() {
            query = query
                .filter(knowledge_relation::Column::RelationType.is_in(relation_types.to_vec()));
        }
        Ok(query
            .order_by_desc(knowledge_relation::Column::UpdatedAt)
            .limit(Self::db_limit(limit.min(1_000)))
            .all(&self.db)
            .await?)
    }

    pub async fn upsert_knowledge_relation_evidence(
        &self,
        evidence: &knowledge_relation_evidence::Model,
    ) -> Result<()> {
        knowledge_relation_evidence::Entity::insert(knowledge_relation_evidence::ActiveModel {
            id: Set(evidence.id.clone()),
            relation_id: Set(evidence.relation_id.clone()),
            evidence_kind: Set(evidence.evidence_kind.clone()),
            evidence_ref: Set(evidence.evidence_ref.clone()),
            memory_id: Set(evidence.memory_id.clone()),
            message_id: Set(evidence.message_id.clone()),
            document_id: Set(evidence.document_id.clone()),
            project_id: Set(evidence.project_id.clone()),
            conversation_id: Set(evidence.conversation_id.clone()),
            polarity: Set(evidence.polarity.clone()),
            confidence: Set(evidence.confidence),
            excerpt: Set(evidence.excerpt.clone()),
            metadata: Set(evidence.metadata.clone()),
            created_at: Set(evidence.created_at.clone()),
        })
        .on_conflict(
            OnConflict::column(knowledge_relation_evidence::Column::Id)
                .update_columns([
                    knowledge_relation_evidence::Column::RelationId,
                    knowledge_relation_evidence::Column::EvidenceKind,
                    knowledge_relation_evidence::Column::EvidenceRef,
                    knowledge_relation_evidence::Column::MemoryId,
                    knowledge_relation_evidence::Column::MessageId,
                    knowledge_relation_evidence::Column::DocumentId,
                    knowledge_relation_evidence::Column::ProjectId,
                    knowledge_relation_evidence::Column::ConversationId,
                    knowledge_relation_evidence::Column::Polarity,
                    knowledge_relation_evidence::Column::Confidence,
                    knowledge_relation_evidence::Column::Excerpt,
                    knowledge_relation_evidence::Column::Metadata,
                ])
                .to_owned(),
        )
        .exec(&self.db)
        .await?;
        Ok(())
    }

    pub async fn list_knowledge_relation_evidence_for_relations(
        &self,
        relation_ids: &[String],
        limit: u64,
    ) -> Result<Vec<knowledge_relation_evidence::Model>> {
        if relation_ids.is_empty() {
            return Ok(Vec::new());
        }
        Ok(knowledge_relation_evidence::Entity::find()
            .filter(knowledge_relation_evidence::Column::RelationId.is_in(relation_ids.to_vec()))
            .order_by_desc(knowledge_relation_evidence::Column::CreatedAt)
            .limit(Self::db_limit(limit.min(1_000)))
            .all(&self.db)
            .await?)
    }

    pub async fn list_knowledge_relation_evidence_for_memory(
        &self,
        memory_id: &str,
        limit: u64,
    ) -> Result<Vec<knowledge_relation_evidence::Model>> {
        let memory_id = memory_id.trim();
        if memory_id.is_empty() {
            return Ok(Vec::new());
        }
        Ok(knowledge_relation_evidence::Entity::find()
            .filter(knowledge_relation_evidence::Column::MemoryId.eq(memory_id.to_string()))
            .order_by_desc(knowledge_relation_evidence::Column::CreatedAt)
            .limit(Self::db_limit(limit.min(1_000)))
            .all(&self.db)
            .await?)
    }
}
