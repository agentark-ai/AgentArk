use super::super::*;

impl Storage {
    // ==================== Conversations ====================

    /// Create a new conversation
    pub async fn create_conversation(&self, conv: &conversation::Model) -> Result<()> {
        conversation::ActiveModel {
            id: Set(conv.id.clone()),
            title: Set(conv.title.clone()),
            channel: Set(conv.channel.clone()),
            project_id: Set(conv.project_id.clone()),
            created_at: Set(conv.created_at.clone()),
            updated_at: Set(conv.updated_at.clone()),
            message_count: Set(conv.message_count),
            archived: Set(conv.archived),
            starred: Set(conv.starred),
        }
        .insert(&self.db)
        .await?;
        Ok(())
    }

    /// Create a conversation only if another path has not already created it.
    pub async fn create_conversation_if_absent(&self, conv: &conversation::Model) -> Result<bool> {
        if self.get_conversation(&conv.id).await?.is_some() {
            return Ok(false);
        }

        match self.create_conversation(conv).await {
            Ok(()) => Ok(true),
            Err(error) => {
                let text = error.to_string().to_ascii_lowercase();
                if text.contains("duplicate key") || text.contains("unique constraint failed") {
                    Ok(false)
                } else {
                    Err(error)
                }
            }
        }
    }

    /// List conversations (newest first, paginated)
    pub async fn list_conversations(
        &self,
        limit: u64,
        offset: u64,
        project_id: Option<&str>,
        excluded_channels: &[&str],
        starred: Option<bool>,
    ) -> Result<Vec<conversation::Model>> {
        let mut query = conversation::Entity::find().order_by_desc(conversation::Column::UpdatedAt);

        if let Some(pid) = project_id {
            query = query.filter(conversation::Column::ProjectId.eq(pid));
        }
        if !excluded_channels.is_empty() {
            query = query
                .filter(conversation::Column::Channel.is_not_in(excluded_channels.iter().copied()));
        }
        if let Some(is_starred) = starred {
            query = query.filter(conversation::Column::Starred.eq(is_starred));
        }

        let convs = query
            .limit(Self::db_limit(limit))
            .offset(Self::db_offset(offset))
            .all(&self.db)
            .await?;
        Ok(convs)
    }

    /// List conversations in ascending update order, optionally continuing after a cursor.
    pub async fn list_conversations_after_cursor(
        &self,
        updated_after: Option<&str>,
        conversation_id_after: Option<&str>,
        limit: u64,
        project_id: Option<&str>,
    ) -> Result<Vec<conversation::Model>> {
        let mut query = conversation::Entity::find()
            .order_by_asc(conversation::Column::UpdatedAt)
            .order_by_asc(conversation::Column::Id);

        if let Some(pid) = project_id {
            query = query.filter(conversation::Column::ProjectId.eq(pid));
        }

        if let Some(updated_at) = updated_after {
            let cursor_filter = if let Some(conversation_id) = conversation_id_after {
                Condition::any()
                    .add(conversation::Column::UpdatedAt.gt(updated_at))
                    .add(
                        Condition::all()
                            .add(conversation::Column::UpdatedAt.eq(updated_at))
                            .add(conversation::Column::Id.gt(conversation_id)),
                    )
            } else {
                Condition::all().add(conversation::Column::UpdatedAt.gte(updated_at))
            };
            query = query.filter(cursor_filter);
        }

        let convs = query.limit(Self::db_limit(limit)).all(&self.db).await?;
        Ok(convs)
    }

    /// Count conversations
    pub async fn count_conversations(
        &self,
        project_id: Option<&str>,
        excluded_channels: &[&str],
        starred: Option<bool>,
    ) -> Result<u64> {
        let mut query = conversation::Entity::find();
        if let Some(pid) = project_id {
            query = query.filter(conversation::Column::ProjectId.eq(pid));
        }
        if !excluded_channels.is_empty() {
            query = query
                .filter(conversation::Column::Channel.is_not_in(excluded_channels.iter().copied()));
        }
        if let Some(is_starred) = starred {
            query = query.filter(conversation::Column::Starred.eq(is_starred));
        }
        Ok(query.count(&self.db).await?)
    }

    /// List conversations touched inside a time window.
    pub async fn list_conversations_updated_between(
        &self,
        from: &str,
        to: &str,
        limit: u64,
    ) -> Result<Vec<conversation::Model>> {
        Ok(conversation::Entity::find()
            .filter(conversation::Column::UpdatedAt.gte(from.to_string()))
            .filter(conversation::Column::UpdatedAt.lt(to.to_string()))
            .filter(conversation::Column::Archived.eq(false))
            .order_by_desc(conversation::Column::UpdatedAt)
            .limit(Self::db_limit(limit))
            .all(&self.db)
            .await?)
    }

    /// Get a single conversation by ID
    pub async fn get_conversation(&self, id: &str) -> Result<Option<conversation::Model>> {
        let conv = conversation::Entity::find_by_id(id.to_string())
            .one(&self.db)
            .await?;
        Ok(conv)
    }

    /// Update conversation title and updated_at
    pub async fn update_conversation(
        &self,
        id: &str,
        title: Option<&str>,
        message_count: Option<i32>,
        starred: Option<bool>,
    ) -> Result<conversation::Model> {
        let Some(existing) = self.get_conversation(id).await? else {
            anyhow::bail!("Conversation not found");
        };
        if title.is_none() && message_count.is_none() && starred.is_none() {
            return Ok(existing);
        }
        if matches!(starred, Some(true)) && !existing.starred {
            let starred_count = conversation::Entity::find()
                .filter(conversation::Column::Starred.eq(true))
                .count(&self.db)
                .await?;
            if starred_count >= 3 {
                anyhow::bail!("Unstar any other chat. Max 3 starred chats allowed.");
            }
        }
        let now = chrono::Utc::now().to_rfc3339();
        let mut model: conversation::ActiveModel = existing.into();
        let mut touch_updated_at = false;
        if let Some(t) = title {
            model.title = Set(t.to_string());
            touch_updated_at = true;
        }
        if let Some(mc) = message_count {
            model.message_count = Set(mc);
            touch_updated_at = true;
        }
        if let Some(is_starred) = starred {
            model.starred = Set(is_starred);
        }
        if touch_updated_at {
            model.updated_at = Set(now);
        }
        let updated = model.update(&self.db).await?;
        Ok(updated)
    }

    /// Delete a conversation and its messages
    pub async fn delete_conversation(&self, id: &str) -> Result<()> {
        let txn = self.db.begin().await?;
        let message_rows = message::Entity::find()
            .filter(message::Column::ConversationId.eq(id))
            .all(&txn)
            .await?;
        let execution_runs = execution_run::Entity::find()
            .filter(execution_run::Column::ConversationId.eq(id.to_string()))
            .all(&txn)
            .await?;
        let experience_run_rows = experience_run::Entity::find()
            .filter(experience_run::Column::ConversationId.eq(id.to_string()))
            .all(&txn)
            .await?;
        let experience_run_ids = experience_run_rows
            .iter()
            .map(|row| row.id.clone())
            .collect::<Vec<_>>();
        let experience_item_rows = experience_item::Entity::find()
            .filter(experience_item::Column::ConversationId.eq(id.to_string()))
            .all(&txn)
            .await?;
        let experience_item_ids = experience_item_rows
            .iter()
            .map(|row| row.id.clone())
            .collect::<Vec<_>>();
        let memory_operation_rows = if experience_item_ids.is_empty() {
            Vec::new()
        } else {
            memory_operation::Entity::find()
                .filter(
                    Condition::any()
                        .add(
                            memory_operation::Column::TargetMemoryId
                                .is_in(experience_item_ids.clone()),
                        )
                        .add(
                            memory_operation::Column::AppliedMemoryId
                                .is_in(experience_item_ids.clone()),
                        ),
                )
                .all(&txn)
                .await?
        };
        let memory_operation_ids = memory_operation_rows
            .iter()
            .map(|row| row.id.clone())
            .collect::<Vec<_>>();
        let mut trace_ids = std::collections::BTreeSet::new();
        for row in &message_rows {
            if let Some(trace_id) = row
                .trace_id
                .as_deref()
                .map(str::trim)
                .filter(|value| !value.is_empty())
            {
                trace_ids.insert(trace_id.to_string());
            }
        }
        for run in &execution_runs {
            if let Some(trace_id) = run
                .trace_id
                .as_deref()
                .map(str::trim)
                .filter(|value| !value.is_empty())
            {
                trace_ids.insert(trace_id.to_string());
            }
        }
        let trace_ids_vec = trace_ids.iter().cloned().collect::<Vec<_>>();
        let proof_ids = if trace_ids_vec.is_empty() {
            Vec::new()
        } else {
            execution_trace::Entity::find()
                .filter(execution_trace::Column::Id.is_in(trace_ids_vec.clone()))
                .all(&txn)
                .await?
                .into_iter()
                .filter_map(|row| row.proof_id)
                .collect::<std::collections::BTreeSet<_>>()
                .into_iter()
                .collect::<Vec<_>>()
        };
        message::Entity::delete_many()
            .filter(message::Column::ConversationId.eq(id))
            .exec(&txn)
            .await?;
        if !experience_item_ids.is_empty() {
            memory_evidence_link::Entity::delete_many()
                .filter(memory_evidence_link::Column::MemoryId.is_in(experience_item_ids.clone()))
                .exec(&txn)
                .await?;
            learning_candidate::Entity::delete_many()
                .filter(learning_candidate::Column::ApprovedRef.is_in(experience_item_ids.clone()))
                .exec(&txn)
                .await?;
            experience_edge::Entity::delete_many()
                .filter(
                    Condition::any()
                        .add(
                            Condition::all()
                                .add(experience_edge::Column::SourceKind.eq("experience_item"))
                                .add(
                                    experience_edge::Column::SourceRef
                                        .is_in(experience_item_ids.clone()),
                                ),
                        )
                        .add(
                            Condition::all()
                                .add(experience_edge::Column::TargetKind.eq("experience_item"))
                                .add(
                                    experience_edge::Column::TargetRef
                                        .is_in(experience_item_ids.clone()),
                                ),
                        ),
                )
                .exec(&txn)
                .await?;
            recall_event::Entity::delete_many()
                .filter(
                    Condition::any()
                        .add(recall_event::Column::MemoryId.is_in(experience_item_ids.clone()))
                        .add(
                            recall_event::Column::RelatedMemoryId
                                .is_in(experience_item_ids.clone()),
                        )
                        .add(
                            Condition::all()
                                .add(recall_event::Column::SourceKind.eq("experience_item"))
                                .add(
                                    recall_event::Column::SourceRef
                                        .is_in(experience_item_ids.clone()),
                                ),
                        ),
                )
                .exec(&txn)
                .await?;
        }
        if !memory_operation_ids.is_empty() {
            memory_evidence_link::Entity::delete_many()
                .filter(
                    memory_evidence_link::Column::OperationId.is_in(memory_operation_ids.clone()),
                )
                .exec(&txn)
                .await?;
            let candidate_ids = memory_operation_ids
                .iter()
                .map(|operation_id| format!("memory-candidate-{operation_id}"))
                .collect::<Vec<_>>();
            learning_candidate::Entity::delete_many()
                .filter(learning_candidate::Column::Id.is_in(candidate_ids))
                .exec(&txn)
                .await?;
            memory_operation::Entity::delete_many()
                .filter(memory_operation::Column::Id.is_in(memory_operation_ids))
                .exec(&txn)
                .await?;
        }
        if !experience_run_ids.is_empty() {
            experience_edge::Entity::delete_many()
                .filter(experience_edge::Column::SourceRunId.is_in(experience_run_ids.clone()))
                .exec(&txn)
                .await?;
            experience_run::Entity::delete_many()
                .filter(experience_run::Column::Id.is_in(experience_run_ids))
                .exec(&txn)
                .await?;
        }
        if !experience_item_ids.is_empty() {
            experience_item::Entity::delete_many()
                .filter(experience_item::Column::Id.is_in(experience_item_ids))
                .exec(&txn)
                .await?;
        }
        operational_log::Entity::delete_many()
            .filter(operational_log::Column::ConversationId.eq(id.to_string()))
            .exec(&txn)
            .await?;
        semantic_work_unit::Entity::delete_many()
            .filter(
                Condition::any()
                    .add(semantic_work_unit::Column::ConversationId.eq(id.to_string()))
                    .add(
                        Condition::all()
                            .add(semantic_work_unit::Column::SourceKind.eq("conversation"))
                            .add(semantic_work_unit::Column::SourceId.eq(id.to_string())),
                    ),
            )
            .exec(&txn)
            .await?;
        if !trace_ids_vec.is_empty() {
            operational_log::Entity::delete_many()
                .filter(operational_log::Column::TraceId.is_in(trace_ids_vec.clone()))
                .exec(&txn)
                .await?;
            execution_trace::Entity::delete_many()
                .filter(execution_trace::Column::Id.is_in(trace_ids_vec))
                .exec(&txn)
                .await?;
        }
        if !proof_ids.is_empty() {
            execution_proof::Entity::delete_many()
                .filter(execution_proof::Column::Id.is_in(proof_ids))
                .exec(&txn)
                .await?;
        }
        execution_run::Entity::delete_many()
            .filter(execution_run::Column::ConversationId.eq(id.to_string()))
            .exec(&txn)
            .await?;
        conversation::Entity::delete_by_id(id.to_string())
            .exec(&txn)
            .await?;
        txn.commit().await?;
        Ok(())
    }

    // ==================== Messages ====================

    /// Insert a message
    pub async fn insert_message(&self, msg: &message::Model) -> Result<()> {
        let content = encrypt_storage_string(&msg.content)?;
        let tool_calls_json = encrypt_optional_storage_string(msg.tool_calls_json.as_deref())?;
        let tool_call_id = encrypt_optional_storage_string(msg.tool_call_id.as_deref())?;
        let provider_message_json =
            encrypt_optional_storage_string(msg.provider_message_json.as_deref())?;
        let insert_result = message::ActiveModel {
            id: Set(msg.id.clone()),
            conversation_id: Set(msg.conversation_id.clone()),
            role: Set(msg.role.clone()),
            content: Set(content.clone()),
            tool_calls_json: Set(tool_calls_json.clone()),
            tool_call_id: Set(tool_call_id.clone()),
            provider_message_json: Set(provider_message_json.clone()),
            timestamp: Set(msg.timestamp.clone()),
            model_used: Set(msg.model_used.clone()),
            trace_id: Set(msg.trace_id.clone()),
        }
        .insert(&self.db)
        .await;
        if let Err(error) = insert_result {
            if msg.trace_id.is_some() && is_foreign_key_constraint_error(&error) {
                tracing::warn!(
                    "Retrying message insert '{}' without trace_id after FK failure: {}",
                    msg.id,
                    error
                );
                message::ActiveModel {
                    id: Set(msg.id.clone()),
                    conversation_id: Set(msg.conversation_id.clone()),
                    role: Set(msg.role.clone()),
                    content: Set(content),
                    tool_calls_json: Set(tool_calls_json),
                    tool_call_id: Set(tool_call_id),
                    provider_message_json: Set(provider_message_json),
                    timestamp: Set(msg.timestamp.clone()),
                    model_used: Set(msg.model_used.clone()),
                    trace_id: Set(None),
                }
                .insert(&self.db)
                .await?;
            } else {
                return Err(error.into());
            }
        }

        // Update conversation message count and updated_at
        let now = chrono::Utc::now().to_rfc3339();
        conversation::Entity::update_many()
            .col_expr(conversation::Column::UpdatedAt, Expr::value(now))
            .col_expr(
                conversation::Column::MessageCount,
                Expr::col(conversation::Column::MessageCount).add(1),
            )
            .filter(conversation::Column::Id.eq(msg.conversation_id.clone()))
            .exec(&self.db)
            .await?;

        Ok(())
    }

    /// Insert a message only when its stable id has not already been persisted.
    ///
    /// This is used by response-first chat persistence retries. A partially
    /// completed retry must not insert the same chat message twice or increment
    /// the parent conversation count twice.
    pub async fn insert_message_if_absent(&self, msg: &message::Model) -> Result<bool> {
        if message::Entity::find_by_id(msg.id.clone())
            .one(&self.db)
            .await?
            .is_some()
        {
            return Ok(false);
        }

        match self.insert_message(msg).await {
            Ok(()) => Ok(true),
            Err(error) => {
                let text = error.to_string().to_ascii_lowercase();
                if text.contains("duplicate key") || text.contains("unique constraint failed") {
                    Ok(false)
                } else {
                    Err(error)
                }
            }
        }
    }

    pub async fn get_message(&self, id: &str) -> Result<Option<message::Model>> {
        let mut message = message::Entity::find_by_id(id.to_string())
            .one(&self.db)
            .await?;
        if let Some(message) = &mut message {
            decrypt_message_model(message);
        }
        Ok(message)
    }

    /// Get messages for a conversation
    pub async fn get_messages(
        &self,
        conversation_id: &str,
        limit: u64,
        offset: u64,
    ) -> Result<Vec<message::Model>> {
        let mut msgs = message::Entity::find()
            .filter(message::Column::ConversationId.eq(conversation_id))
            .order_by_asc(message::Column::Timestamp)
            .limit(Self::db_limit(limit))
            .offset(Self::db_offset(offset))
            .all(&self.db)
            .await?;
        for msg in &mut msgs {
            decrypt_message_model(msg);
        }
        Ok(msgs)
    }

    /// Get messages for a conversation inside a bounded time window.
    #[allow(dead_code)]
    pub async fn get_messages_between(
        &self,
        conversation_id: &str,
        from: &str,
        to: &str,
        limit: u64,
    ) -> Result<Vec<message::Model>> {
        let mut msgs = message::Entity::find()
            .filter(message::Column::ConversationId.eq(conversation_id))
            .filter(message::Column::Timestamp.gte(from.to_string()))
            .filter(message::Column::Timestamp.lt(to.to_string()))
            .order_by_asc(message::Column::Timestamp)
            .limit(Self::db_limit(limit))
            .all(&self.db)
            .await?;
        for msg in &mut msgs {
            decrypt_message_model(msg);
        }
        Ok(msgs)
    }

    /// Get most recent messages for a conversation in chronological order.
    pub async fn get_recent_messages(
        &self,
        conversation_id: &str,
        limit: u64,
    ) -> Result<Vec<message::Model>> {
        let mut msgs = message::Entity::find()
            .filter(message::Column::ConversationId.eq(conversation_id))
            .order_by_desc(message::Column::Timestamp)
            .limit(Self::db_limit(limit))
            .all(&self.db)
            .await?;
        msgs.reverse();
        for msg in &mut msgs {
            decrypt_message_model(msg);
        }
        Ok(msgs)
    }

    #[allow(dead_code)]
    pub async fn latest_assistant_trace_id_for_conversation(
        &self,
        conversation_id: &str,
    ) -> Result<Option<String>> {
        Ok(message::Entity::find()
            .filter(message::Column::ConversationId.eq(conversation_id.to_string()))
            .filter(message::Column::Role.eq("assistant".to_string()))
            .filter(message::Column::TraceId.is_not_null())
            .order_by_desc(message::Column::Timestamp)
            .one(&self.db)
            .await?
            .and_then(|message| message.trace_id)
            .map(|trace_id| trace_id.trim().to_string())
            .filter(|trace_id| !trace_id.is_empty()))
    }

    /// Get most recent user-authored chat messages across conversations.
    pub async fn get_recent_user_messages(&self, limit: u64) -> Result<Vec<message::Model>> {
        let mut msgs = message::Entity::find()
            .filter(message::Column::Role.eq("user"))
            .order_by_desc(message::Column::Timestamp)
            .limit(Self::db_limit(limit))
            .all(&self.db)
            .await?;
        for msg in &mut msgs {
            decrypt_message_model(msg);
        }
        Ok(msgs)
    }

    /// Get most recent persisted messages across conversations.
    pub async fn get_recent_messages_across_conversations(
        &self,
        limit: u64,
    ) -> Result<Vec<message::Model>> {
        let mut msgs = message::Entity::find()
            .order_by_desc(message::Column::Timestamp)
            .limit(Self::db_limit(limit))
            .all(&self.db)
            .await?;
        for msg in &mut msgs {
            decrypt_message_model(msg);
        }
        Ok(msgs)
    }

    /// Returns true when at least one persisted user chat message exists.
    pub async fn has_user_chat_messages(&self) -> Result<bool> {
        let exists = message::Entity::find()
            .filter(message::Column::Role.eq("user"))
            .limit(1)
            .one(&self.db)
            .await?
            .is_some();
        Ok(exists)
    }
}
