use super::*;

pub(super) async fn list_available_channels(State(state): State<AppState>) -> Response {
    use crate::channels::messaging_registry::{
        BundledConfiguredCheck, ChannelQueryContext, MessagingChannelRegistry,
    };

    let agent = state.agent.read().await;
    let config_manager = match crate::core::config::SecureConfigManager::new_with_data_dir(
        &agent.config_dir,
        Some(&agent.data_dir),
    ) {
        Ok(manager) => Some(manager),
        Err(error) => {
            tracing::debug!(
                "list_available_channels: config manager unavailable: {}",
                error
            );
            None
        }
    };
    let packs_guard = agent.extension_packs.read().await;

    struct AgentBundledCheck<'a>(&'a crate::core::Agent);
    impl<'a> BundledConfiguredCheck for AgentBundledCheck<'a> {
        fn is_configured(&self, channel_id: &str) -> bool {
            self.0.notification_channel_is_configured(channel_id)
        }
    }
    let bundled_check = AgentBundledCheck(&agent);
    let ctx = ChannelQueryContext {
        bundled_configured: &bundled_check,
        extension_packs: &packs_guard,
        storage: &agent.storage,
        config_dir: &agent.config_dir,
        data_dir: &agent.data_dir,
        config_manager: config_manager.as_ref(),
    };
    let registry = MessagingChannelRegistry::new();
    match registry.list(&ctx).await {
        Ok(descriptors) => (
            StatusCode::OK,
            Json(serde_json::json!({
                "channels": descriptors,
            })),
        )
            .into_response(),
        Err(error) => (
            StatusCode::INTERNAL_SERVER_ERROR,
            Json(ErrorResponse {
                error: error.to_string(),
            }),
        )
            .into_response(),
    }
}

// ==================== Memory Endpoints ====================

const ARKMEMORY_CLEANUP_MEMORY_SCAN_LIMIT: u64 = 2_000;

pub(super) async fn memory_stats(
    State(state): State<AppState>,
    axum::extract::Query(_params): axum::extract::Query<std::collections::HashMap<String, String>>,
) -> Response {
    let project_id: Option<&str> = None;
    let agent = state.agent.read().await;
    let profile_fact_count = agent
        .storage
        .count_facts_by_category(
            project_id,
            crate::core::memory_schema::MEMORY_CATEGORY_PROFILE_FACT,
        )
        .await
        .unwrap_or(0);
    let assistant_preference_count = agent
        .storage
        .count_facts_by_category(
            project_id,
            crate::core::memory_schema::MEMORY_CATEGORY_ASSISTANT_PREFERENCE,
        )
        .await
        .unwrap_or(0);
    let work_preference_count = agent
        .storage
        .count_facts_by_category(
            project_id,
            crate::core::memory_schema::MEMORY_CATEGORY_WORK_PREFERENCE,
        )
        .await
        .unwrap_or(0);
    let project_domain_count = agent
        .storage
        .count_facts_by_category(
            project_id,
            crate::core::memory_schema::MEMORY_CATEGORY_PROJECT_DOMAIN,
        )
        .await
        .unwrap_or(0);
    let ephemeral_count = agent
        .storage
        .count_facts_by_category(
            project_id,
            crate::core::memory_schema::MEMORY_CATEGORY_EPHEMERAL_CONTEXT,
        )
        .await
        .unwrap_or(0);
    let other_memory_count = agent
        .storage
        .count_facts_by_category(
            project_id,
            crate::core::memory_schema::MEMORY_CATEGORY_OTHER,
        )
        .await
        .unwrap_or(0);
    let fact_count = profile_fact_count;
    let doc_count = agent.storage.count_documents(project_id).await.unwrap_or(0);
    let preference_count = agent
        .storage
        .count_user_preferences(project_id)
        .await
        .unwrap_or(0);
    let user_data_count = agent
        .storage
        .count_user_data_items(project_id, None)
        .await
        .unwrap_or(0);
    let knowledge_count = agent
        .storage
        .count_visible_knowledge_items(project_id)
        .await
        .unwrap_or(0);
    (
        StatusCode::OK,
        Json(serde_json::json!({
            "facts": fact_count,
            "profile_facts": profile_fact_count,
            "assistant_preferences": assistant_preference_count,
            "work_preferences": work_preference_count,
            "project_domain_memory": project_domain_count,
            "ephemeral_context": ephemeral_count,
            "other_memory": other_memory_count,
            "documents": doc_count,
            "preferences": preference_count,
            "user_data": user_data_count,
            "knowledge": knowledge_count,
        })),
    )
        .into_response()
}

/// List learned facts from the current memory store.
pub(super) async fn list_facts(
    State(state): State<AppState>,
    axum::extract::Query(params): axum::extract::Query<std::collections::HashMap<String, String>>,
) -> Response {
    let project_id: Option<&str> = None;
    let limit = params
        .get("limit")
        .and_then(|s| s.parse().ok())
        .unwrap_or(20u64);
    let offset = params
        .get("offset")
        .and_then(|s| s.parse().ok())
        .unwrap_or(0u64);
    let category = params
        .get("category")
        .map(|value| value.trim())
        .filter(|value| !value.is_empty() && *value != "all");
    let agent = state.agent.read().await;
    let total = match category {
        Some(category) => agent
            .storage
            .count_facts_by_category(project_id, category)
            .await
            .unwrap_or(0),
        None => agent.storage.count_facts(project_id).await.unwrap_or(0),
    };
    let facts_result = match category {
        Some(category) => {
            agent
                .encrypted_storage
                .get_facts_by_project_and_category_decrypted(limit, offset, project_id, category)
                .await
        }
        None => {
            agent
                .encrypted_storage
                .get_facts_by_project_decrypted(limit, offset, project_id)
                .await
        }
    };
    match facts_result {
        Ok(facts) => {
            let mut items: Vec<serde_json::Value> = Vec::with_capacity(facts.len());
            for f in &facts {
                let sources =
                    memory_fact_evidence_sources(&agent.storage, f, project_id, 100).await;
                let evidence_count = sources.len();
                items.push(serde_json::json!({
                    "id": f.id,
                    "fact": f.fact,
                    "key": f.key,
                    "value": f.value,
                    "confidence": f.confidence,
                    "memory_kind": f.memory_kind.clone(),
                    "memory_category": f.memory_category.clone(),
                    "topics": f.topics.clone(),
                    "scope": f.scope.clone(),
                    "sources": sources,
                    "evidence_count": evidence_count,
                    "created_at": f.created_at,
                    "updated_at": f.updated_at,
                }));
            }
            (
                StatusCode::OK,
                Json(serde_json::json!({
                    "facts": items,
                    "total": total,
                    "limit": limit,
                    "offset": offset,
                })),
            )
                .into_response()
        }
        Err(e) => (
            StatusCode::INTERNAL_SERVER_ERROR,
            Json(ErrorResponse {
                error: e.to_string(),
            }),
        )
            .into_response(),
    }
}

pub(super) async fn delete_memory_fact(
    State(state): State<AppState>,
    Path(id): Path<String>,
) -> Response {
    let memory_id = id.trim();
    if memory_id.is_empty() {
        return (
            StatusCode::BAD_REQUEST,
            Json(ErrorResponse {
                error: "memory id is required".to_string(),
            }),
        )
            .into_response();
    }
    let project_id: Option<&str> = None;
    let agent = state.agent.read().await;
    let item = match agent.storage.get_experience_item(memory_id).await {
        Ok(Some(item)) => item,
        Ok(None) => {
            return (
                StatusCode::NOT_FOUND,
                Json(ErrorResponse {
                    error: "memory not found".to_string(),
                }),
            )
                .into_response();
        }
        Err(error) => {
            return (
                StatusCode::INTERNAL_SERVER_ERROR,
                Json(ErrorResponse {
                    error: error.to_string(),
                }),
            )
                .into_response();
        }
    };
    if !arkmemory_item_is_memory(&item) || !arkmemory_item_visible_for_project(&item, project_id) {
        return (
            StatusCode::BAD_REQUEST,
            Json(ErrorResponse {
                error: "item is not a deletable Memory learned memory".to_string(),
            }),
        )
            .into_response();
    }
    match agent
        .storage
        .hard_delete_experience_item_memory(memory_id)
        .await
    {
        Ok(deleted) => (
            StatusCode::OK,
            Json(serde_json::json!({
                "deleted": deleted,
                "id": memory_id,
            })),
        )
            .into_response(),
        Err(error) => (
            StatusCode::INTERNAL_SERVER_ERROR,
            Json(ErrorResponse {
                error: error.to_string(),
            }),
        )
            .into_response(),
    }
}

fn memory_fact_key_for_value_edit(item: &crate::storage::experience_item::Model) -> Option<String> {
    item.metadata
        .get("key")
        .and_then(|value| value.as_str())
        .map(str::trim)
        .filter(|value| !value.is_empty())
        .map(str::to_string)
        .or_else(|| {
            item.content
                .split_once(':')
                .map(|(key, _)| key.trim())
                .filter(|key| !key.is_empty() && !key.chars().any(char::is_whitespace))
                .map(str::to_string)
        })
}

fn memory_fact_content_from_value_edit(
    item: &crate::storage::experience_item::Model,
    value: &str,
) -> String {
    let (key, _) = memory_fact_repaired_key_value(item);
    match key {
        Some(key) => format!("{key}: {}", value.trim()),
        None => value.trim().to_string(),
    }
}

fn memory_fact_value_from_content(key: Option<&str>, content: &str) -> String {
    let trimmed = content.trim();
    if let Some(key) = key.map(str::trim).filter(|value| !value.is_empty()) {
        let prefix = format!("{key}:");
        if let Some(value) = trimmed.strip_prefix(&prefix) {
            return value.trim().to_string();
        }
        return trimmed.to_string();
    }
    if let Some((candidate_key, value)) = trimmed.split_once(':') {
        let candidate_key = candidate_key.trim();
        if !candidate_key.is_empty() && !candidate_key.chars().any(char::is_whitespace) {
            return value.trim().to_string();
        }
    }
    trimmed.to_string()
}

fn memory_fact_repair_allows_value_suffix(item: &crate::storage::experience_item::Model) -> bool {
    let memory_kind = item
        .metadata
        .get("memory_kind")
        .and_then(|value| value.as_str())
        .map(str::trim)
        .filter(|value| !value.is_empty());
    crate::core::memory_schema::memory_category_from_metadata(&item.metadata, memory_kind)
        == crate::core::memory_schema::MEMORY_CATEGORY_PROFILE_FACT
}

fn memory_fact_repaired_key_value(
    item: &crate::storage::experience_item::Model,
) -> (Option<String>, String) {
    let key = memory_fact_key_for_value_edit(item);
    let value = memory_fact_value_from_content(key.as_deref(), &item.content);
    match key {
        Some(raw_key) => {
            match crate::core::memory_schema::repair_memory_slot_key_and_value(
                &raw_key,
                &value,
                memory_fact_repair_allows_value_suffix(item),
            ) {
                Some((key, repaired_value)) => (Some(key), repaired_value.unwrap_or(value)),
                None => (Some(raw_key), value),
            }
        }
        None => (None, value),
    }
}

fn memory_fact_response_payload(
    item: &crate::storage::experience_item::Model,
) -> serde_json::Value {
    let memory_kind = item
        .metadata
        .get("memory_kind")
        .and_then(|value| value.as_str())
        .map(str::trim)
        .filter(|value| !value.is_empty());
    let (key, value) = memory_fact_repaired_key_value(item);
    serde_json::json!({
        "id": item.id,
        "fact": item.content,
        "key": key,
        "value": value,
        "confidence": item.confidence,
        "memory_kind": memory_kind,
        "memory_category": crate::core::memory_schema::memory_category_from_metadata(
            &item.metadata,
            memory_kind,
        ),
        "topics": crate::core::memory_schema::normalize_memory_topics(
            item.metadata.get("topics"),
            8,
        ),
        "scope": item.scope,
        "created_at": item.created_at,
        "updated_at": item.updated_at,
    })
}

pub(super) async fn update_memory_fact_value(
    State(state): State<AppState>,
    Path(id): Path<String>,
    Json(request): Json<UpdateMemoryFactValueRequest>,
) -> Response {
    let memory_id = id.trim();
    if memory_id.is_empty() {
        return (
            StatusCode::BAD_REQUEST,
            Json(ErrorResponse {
                error: "memory id is required".to_string(),
            }),
        )
            .into_response();
    }
    let edited_value = request
        .value
        .or(request.content)
        .map(|value| value.trim().to_string())
        .filter(|value| !value.is_empty());
    let Some(edited_value) = edited_value else {
        return (
            StatusCode::BAD_REQUEST,
            Json(ErrorResponse {
                error: "memory value is required".to_string(),
            }),
        )
            .into_response();
    };

    let project_id: Option<&str> = None;
    let agent = state.agent.read().await;
    let item = match agent.storage.get_experience_item(memory_id).await {
        Ok(Some(item)) => item,
        Ok(None) => {
            return (
                StatusCode::NOT_FOUND,
                Json(ErrorResponse {
                    error: "memory not found".to_string(),
                }),
            )
                .into_response();
        }
        Err(error) => {
            return (
                StatusCode::INTERNAL_SERVER_ERROR,
                Json(ErrorResponse {
                    error: error.to_string(),
                }),
            )
                .into_response();
        }
    };
    if item.status != "active"
        || !arkmemory_item_is_memory(&item)
        || !arkmemory_item_visible_for_project(&item, project_id)
    {
        return (
            StatusCode::BAD_REQUEST,
            Json(ErrorResponse {
                error: "item is not an editable Memory learned memory".to_string(),
            }),
        )
            .into_response();
    }
    let next_content = memory_fact_content_from_value_edit(&item, &edited_value);
    match agent
        .storage
        .update_experience_item_content(memory_id, &next_content)
        .await
    {
        Ok(Some(updated)) => (
            StatusCode::OK,
            Json(serde_json::json!({
                "status": "ok",
                "id": memory_id,
                "memory": memory_fact_response_payload(&updated),
            })),
        )
            .into_response(),
        Ok(None) => (
            StatusCode::NOT_FOUND,
            Json(ErrorResponse {
                error: "memory not found".to_string(),
            }),
        )
            .into_response(),
        Err(error) => (
            StatusCode::INTERNAL_SERVER_ERROR,
            Json(ErrorResponse {
                error: error.to_string(),
            }),
        )
            .into_response(),
    }
}

fn parse_fact_source_refs(value: &str) -> Vec<String> {
    fn push_json_source(value: &serde_json::Value, out: &mut Vec<String>) {
        match value {
            serde_json::Value::String(raw) => {
                let trimmed = raw.trim();
                if !trimmed.is_empty() {
                    out.push(trimmed.to_string());
                }
            }
            serde_json::Value::Array(items) => {
                for item in items {
                    push_json_source(item, out);
                }
            }
            serde_json::Value::Object(object) => {
                for key in [
                    "source",
                    "sources",
                    "source_ref",
                    "source_refs",
                    "evidence_refs",
                ] {
                    if let Some(inner) = object.get(key) {
                        push_json_source(inner, out);
                    }
                }
            }
            _ => {}
        }
    }

    let trimmed = value.trim();
    if trimmed.is_empty() {
        return Vec::new();
    }
    let mut sources = Vec::new();
    match serde_json::from_str::<serde_json::Value>(trimmed) {
        Ok(parsed) => push_json_source(&parsed, &mut sources),
        Err(_) => sources.push(trimmed.to_string()),
    }
    sources
        .into_iter()
        .map(|source| source.trim().to_string())
        .filter(|source| !source.is_empty())
        .collect()
}

fn memory_operation_evidence_source_refs(
    operation: &crate::storage::memory_operation::Model,
) -> Vec<String> {
    operation
        .evidence_refs
        .as_array()
        .into_iter()
        .flatten()
        .filter_map(|value| value.as_str())
        .map(str::trim)
        .filter(|value| !value.is_empty())
        .filter(|value| {
            value.starts_with("message:")
                || value.starts_with("capture_event:")
                || value.starts_with("source:")
                || value.starts_with("source_ref:")
        })
        .map(str::to_string)
        .collect()
}

#[cfg(test)]
mod memory_control_tests {
    use super::*;

    fn memory_fact_test_item(
        content: &str,
        metadata: serde_json::Value,
    ) -> crate::storage::experience_item::Model {
        let now = "2026-05-22T00:00:00Z".to_string();
        crate::storage::experience_item::Model {
            id: "memory-1".to_string(),
            kind: "personal_fact".to_string(),
            scope: "global".to_string(),
            project_id: None,
            conversation_id: Some("conversation-1".to_string()),
            title: "Learned user memory".to_string(),
            content: content.to_string(),
            normalized_key: "user_memory::user_first_name::permanent".to_string(),
            confidence: 0.95,
            support_count: 1,
            contradiction_count: 0,
            status: "active".to_string(),
            metadata,
            last_supported_at: Some(now.clone()),
            last_contradicted_at: None,
            created_at: now.clone(),
            updated_at: now,
            embedding: None,
        }
    }

    #[test]
    fn memory_fact_payload_exposes_structured_key_and_plain_value() {
        let item = memory_fact_test_item(
            "user_first_name: Alex",
            serde_json::json!({
                "key": "user_first_name",
                "memory_kind": "identity",
                "memory_category": "profile_fact",
                "topics": ["identity"],
            }),
        );

        let payload = memory_fact_response_payload(&item);

        assert_eq!(
            payload.get("fact").and_then(|value| value.as_str()),
            Some("user_first_name: Alex")
        );
        assert_eq!(
            payload.get("key").and_then(|value| value.as_str()),
            Some("user_first_name")
        );
        assert_eq!(
            payload.get("value").and_then(|value| value.as_str()),
            Some("Alex")
        );
        assert_eq!(
            memory_fact_content_from_value_edit(&item, "Alexandra"),
            "user_first_name: Alexandra"
        );
    }

    #[test]
    fn memory_fact_payload_repairs_existing_key_that_contains_value() {
        let item = memory_fact_test_item(
            "user_name_alex: The user's name is Alex.",
            serde_json::json!({
                "key": "user_name_alex",
                "memory_kind": "identity",
                "memory_category": "profile_fact",
                "topics": ["identity"],
            }),
        );

        let payload = memory_fact_response_payload(&item);

        assert_eq!(
            payload.get("key").and_then(|value| value.as_str()),
            Some("user_name")
        );
        assert_eq!(
            payload.get("value").and_then(|value| value.as_str()),
            Some("Alex")
        );
        assert_eq!(
            memory_fact_content_from_value_edit(&item, "Alexandra"),
            "user_name: Alexandra"
        );
    }

    #[test]
    fn memory_fact_value_does_not_strip_unrelated_prefix_when_key_is_known() {
        assert_eq!(
            memory_fact_value_from_content(Some("user_first_name"), "display_name: Alex"),
            "display_name: Alex"
        );
    }

    #[test]
    fn fact_source_refs_parse_legacy_and_structured_sources() {
        assert_eq!(
            parse_fact_source_refs(r#"["message:m1","capture_event:c1"]"#),
            vec!["message:m1".to_string(), "capture_event:c1".to_string()]
        );
        assert_eq!(
            parse_fact_source_refs(
                r#"{"source":"manual","evidence_refs":["message:m2","capture_event:c2"]}"#
            ),
            vec![
                "manual".to_string(),
                "message:m2".to_string(),
                "capture_event:c2".to_string()
            ]
        );
        assert_eq!(
            parse_fact_source_refs("message:plain"),
            vec!["message:plain".to_string()]
        );
    }

    #[test]
    fn memory_operation_evidence_refs_keep_source_refs_not_channel_metadata() {
        let now = "2026-05-03T00:00:00Z".to_string();
        let operation = crate::storage::memory_operation::Model {
            id: "operation-1".to_string(),
            capture_event_id: Some("capture-1".to_string()),
            operation_type: "add".to_string(),
            status: "applied".to_string(),
            target_memory_id: None,
            applied_memory_id: Some("memory-1".to_string()),
            key: Some("memory_key".to_string()),
            value: Some("Memory value".to_string()),
            memory_kind: "identity".to_string(),
            durability: "permanent".to_string(),
            scope: "global".to_string(),
            project_id: None,
            conversation_id: Some("conversation-1".to_string()),
            confidence: 0.95,
            looks_sensitive: false,
            sensitive_reason: None,
            valid_from: None,
            expires_at: None,
            review_at: None,
            rationale: None,
            evidence_refs: serde_json::json!(["message:m1", "capture_event:c1", "channel:chat"]),
            model_metadata: serde_json::json!({}),
            apply_metadata: serde_json::json!({}),
            applied_at: Some(now.clone()),
            reviewed_at: Some(now.clone()),
            review_notes: None,
            created_at: now.clone(),
            updated_at: now,
        };

        assert_eq!(
            memory_operation_evidence_source_refs(&operation),
            vec!["message:m1".to_string(), "capture_event:c1".to_string()]
        );
    }

    #[test]
    fn expired_memory_cleanup_candidate_requires_temporary_metadata_expiry() {
        let now = chrono::DateTime::parse_from_rfc3339("2026-06-24T00:00:00Z")
            .expect("valid timestamp")
            .with_timezone(&chrono::Utc);
        let mut expired = memory_fact_test_item(
            "travel_context: Visiting family next week.",
            serde_json::json!({
                "key": "travel_context",
                "memory_kind": "personal_fact",
                "memory_category": "ephemeral_context",
                "durability": "temporary",
                "expires_at": "2026-06-23T00:00:00Z",
            }),
        );
        expired.id = "memory-expired".to_string();

        let candidate = arkmemory_expired_memory_cleanup_item(&expired, now)
            .expect("expired active memory should be a cleanup candidate");
        assert_eq!(
            candidate.get("kind").and_then(|value| value.as_str()),
            Some("expired_memory")
        );
        assert_eq!(
            candidate.get("action").and_then(|value| value.as_str()),
            Some("expire_memory")
        );
        assert_eq!(
            candidate.get("memory_id").and_then(|value| value.as_str()),
            Some("memory-expired")
        );

        let mut future = expired.clone();
        future.metadata["expires_at"] = serde_json::json!("2026-06-25T00:00:00Z");
        assert!(arkmemory_expired_memory_cleanup_item(&future, now).is_none());

        let mut inactive = expired;
        inactive.status = "deprecated".to_string();
        assert!(arkmemory_expired_memory_cleanup_item(&inactive, now).is_none());

        let durable_dated_memory = memory_fact_test_item(
            "important_life_event: A significant personal event happened last week.",
            serde_json::json!({
                "key": "important_life_event",
                "memory_kind": "personal_fact",
                "memory_category": "profile_fact",
                "durability": "permanent",
            }),
        );
        assert!(arkmemory_expired_memory_cleanup_item(&durable_dated_memory, now).is_none());

        let mut permanent_with_expiry_metadata = durable_dated_memory;
        permanent_with_expiry_metadata.metadata["expires_at"] =
            serde_json::json!("2026-06-23T00:00:00Z");
        assert!(
            arkmemory_expired_memory_cleanup_item(&permanent_with_expiry_metadata, now).is_none()
        );
    }

    #[test]
    fn pending_capture_events_group_by_semantic_capture_key() {
        let now = "2026-05-03T00:00:00Z".to_string();
        let event = crate::storage::memory_capture_event::Model {
            id: "memory-capture-pending-1".to_string(),
            source_message_id: Some("message-1".to_string()),
            conversation_id: Some("conversation-1".to_string()),
            project_id: None,
            channel: "chat".to_string(),
            status: "pending_consolidation".to_string(),
            capture_kind: "user_fact_memory_capture".to_string(),
            source_hash: "pending:semantic-source-key".to_string(),
            attempt_metadata: serde_json::json!({
                "semantic_capture_key": "semantic-source-key",
            }),
            error_history: serde_json::json!([]),
            replay_count: 0,
            next_retry_at: None,
            completed_at: None,
            created_at: now.clone(),
            updated_at: now,
        };

        assert_eq!(
            arkmemory_capture_event_semantic_key(&event),
            "semantic-source-key"
        );
    }

    #[test]
    fn stale_processing_recovery_is_not_a_failed_capture_health_status() {
        assert!(!ARKMEMORY_FAILED_CAPTURE_STATUSES.contains(&"failed_stale_processing"));
        assert!(!ARKMEMORY_FAILED_CAPTURE_STATUSES.contains(&"retired_stale_processing"));
    }

    fn memory_capture_event_with_status(
        status: &str,
    ) -> crate::storage::memory_capture_event::Model {
        let now = "2026-05-03T00:00:00Z".to_string();
        crate::storage::memory_capture_event::Model {
            id: format!("memory-capture-{}", status),
            source_message_id: Some("message-1".to_string()),
            conversation_id: Some("conversation-1".to_string()),
            project_id: None,
            channel: "chat".to_string(),
            status: status.to_string(),
            capture_kind: "user_fact_memory_capture".to_string(),
            source_hash: "semantic-source-key".to_string(),
            attempt_metadata: serde_json::json!({}),
            error_history: serde_json::json!([]),
            replay_count: 0,
            next_retry_at: None,
            completed_at: None,
            created_at: now.clone(),
            updated_at: now,
        }
    }

    #[test]
    fn learned_review_context_redacts_source_text() {
        let event = memory_capture_event_with_status("rejected_sensitive_input");
        let context = arkmemory_capture_review_context_from_source_text(
            &event,
            "my OpenAI key is sk-abcdefghijklmnopqrstuvwxyz123456",
        );

        assert!(context
            .source_redactions
            .iter()
            .any(|value| value.contains("openai_key")));
        assert!(!context
            .source_semantic_text
            .contains("sk-abcdefghijklmnopqrstuvwxyz123456"));
    }

    #[test]
    fn learned_review_decision_parser_accepts_structured_json() {
        let decision = arkmemory_parse_learned_review_decision(
            r#"{"apply":true,"outcome":"expected_sensitive_skip","confidence":0.91,"matched_example_id":"capture-1","reason":"same reviewed credential-disclosure case"}"#,
        )
        .expect("decision should parse");

        assert!(decision.apply);
        assert_eq!(decision.outcome.as_deref(), Some("expected_sensitive_skip"));
        assert_eq!(decision.matched_example_id.as_deref(), Some("capture-1"));
        assert!(decision.confidence > 0.9);
    }

    #[test]
    fn memory_graph_semantic_edges_skip_explicit_links_and_cap_per_node() {
        use sea_orm::entity::prelude::PgVector;

        let mut a = memory_fact_test_item(
            "memory_a: A",
            serde_json::json!({"memory_category": "profile_fact"}),
        );
        a.id = "memory-a".to_string();
        a.embedding = Some(PgVector::from(vec![1.0_f32, 0.0, 0.0]));

        let mut b = memory_fact_test_item(
            "memory_b: B",
            serde_json::json!({"memory_category": "profile_fact"}),
        );
        b.id = "memory-b".to_string();
        b.embedding = Some(PgVector::from(vec![0.96_f32, 0.04, 0.0]));

        let mut c = memory_fact_test_item(
            "memory_c: C",
            serde_json::json!({"memory_category": "profile_fact"}),
        );
        c.id = "memory-c".to_string();
        c.embedding = Some(PgVector::from(vec![0.95_f32, 0.05, 0.0]));

        let mut explicit_pairs = std::collections::HashSet::new();
        explicit_pairs.insert(arkmemory_graph_pair_key("memory-a", "memory-b"));

        let edges = arkmemory_graph_semantic_edges(&[a, b, c], &explicit_pairs, 0.90, 1, 16);

        assert_eq!(edges.len(), 1);
        assert_eq!(edges[0].edge_type, "semantic_nearby");
        assert!(edges[0].semantic);
        assert_ne!(
            arkmemory_graph_pair_key(&edges[0].source, &edges[0].target),
            arkmemory_graph_pair_key("memory-a", "memory-b")
        );
        assert!(edges[0].weight >= 0.90);
    }

    #[test]
    fn memory_graph_csv_filter_ignores_empty_values_and_dedupes() {
        assert_eq!(
            arkmemory_graph_csv_filter(" active,deprecated,active, ,stale "),
            vec![
                "active".to_string(),
                "deprecated".to_string(),
                "stale".to_string()
            ]
        );
    }

    #[test]
    fn document_memory_sensitivity_gate_blocks_non_prompt_safe_candidates() {
        assert!(arkmemory_document_memory_sensitivity_safe("prompt_safe"));
        assert!(arkmemory_document_memory_sensitivity_safe(""));
        assert!(!arkmemory_document_memory_sensitivity_safe(
            "personal_identifier"
        ));
        assert!(!arkmemory_document_memory_sensitivity_safe("sensitive"));
    }
}

async fn memory_fact_evidence_sources(
    storage: &crate::storage::Storage,
    fact: &crate::storage::LearnedFactRecord,
    project_id: Option<&str>,
    limit: u64,
) -> Vec<String> {
    let mut sources = parse_fact_source_refs(&fact.sources);
    if let Ok(links) = storage
        .list_memory_evidence_links_for_memory(&fact.id, project_id, limit)
        .await
    {
        sources.extend(
            links
                .into_iter()
                .map(|link| format!("{}:{}", link.evidence_kind, link.evidence_ref)),
        );
    }
    if let Ok(operations) = storage
        .list_memory_operations_for_memory(&fact.id, project_id, limit)
        .await
    {
        for operation in &operations {
            sources.extend(memory_operation_evidence_source_refs(operation));
        }
    }
    sources.sort();
    sources.dedup();
    sources
}

#[derive(Debug, Deserialize)]
pub(super) struct UpdateMemoryFactValueRequest {
    #[serde(default)]
    value: Option<String>,
    #[serde(default)]
    content: Option<String>,
}

#[derive(Debug, Deserialize)]
pub(super) struct UpsertUserPreferenceRequest {
    key: String,
    value: String,
    sensitivity: Option<String>,
    confidence: Option<f32>,
    source: Option<String>,
}

#[derive(Debug, Deserialize)]
pub(super) struct CreateUserDataItemRequest {
    kind: String,
    title: String,
    content: String,
    url: Option<String>,
    source_channel: Option<String>,
    conversation_id: Option<String>,
    pinned: Option<bool>,
}

#[derive(Debug, Deserialize)]
pub(super) struct CreateKnowledgeItemRequest {
    title: String,
    content: String,
    source: Option<String>,
    url: Option<String>,
    tags: Option<String>,
}

pub(super) async fn list_user_preferences(
    State(state): State<AppState>,
    axum::extract::Query(params): axum::extract::Query<std::collections::HashMap<String, String>>,
) -> Response {
    let limit = params
        .get("limit")
        .and_then(|v| v.parse::<u64>().ok())
        .unwrap_or(30);
    let offset = params
        .get("offset")
        .and_then(|v| v.parse::<u64>().ok())
        .unwrap_or(0);
    let project_id: Option<&str> = None;

    let agent = state.agent.read().await;
    match agent
        .storage
        .list_user_preferences(limit, offset, project_id)
        .await
    {
        Ok(items) => {
            let total = agent
                .storage
                .count_user_preferences(project_id)
                .await
                .unwrap_or(0);
            (
                StatusCode::OK,
                Json(serde_json::json!({
                    "preferences": items,
                    "total": total,
                    "limit": limit,
                    "offset": offset,
                })),
            )
                .into_response()
        }
        Err(e) => (
            StatusCode::INTERNAL_SERVER_ERROR,
            Json(ErrorResponse {
                error: e.to_string(),
            }),
        )
            .into_response(),
    }
}

pub(super) async fn upsert_user_preference(
    State(state): State<AppState>,
    Json(payload): Json<UpsertUserPreferenceRequest>,
) -> Response {
    if payload.key.trim().is_empty() || payload.value.trim().is_empty() {
        return (
            StatusCode::BAD_REQUEST,
            Json(ErrorResponse {
                error: "key and value are required".to_string(),
            }),
        )
            .into_response();
    }

    let agent = state.agent.read().await;
    match agent
        .storage
        .upsert_user_preference(
            payload.key.trim(),
            payload.value.trim(),
            payload.confidence.unwrap_or(0.85),
            payload.source.as_deref(),
            None,
            payload.sensitivity.as_deref(),
        )
        .await
    {
        Ok(item) => {
            if item.project_id.is_none() {
                let source = item.source.as_deref().unwrap_or("memory_api");
                if let Err(error) = crate::core::learning::sync_user_preference_to_experience_item(
                    &agent.storage,
                    &item.key,
                    &item.value,
                    item.confidence as f64,
                    source,
                    Some(&item.sensitivity),
                )
                .await
                {
                    tracing::warn!(
                        "Failed to sync user preference '{}' into experience memory: {}",
                        item.key,
                        error
                    );
                }
            }
            (
                StatusCode::OK,
                Json(serde_json::json!({"preference": item})),
            )
                .into_response()
        }
        Err(e) => (
            StatusCode::INTERNAL_SERVER_ERROR,
            Json(ErrorResponse {
                error: e.to_string(),
            }),
        )
            .into_response(),
    }
}

pub(super) async fn delete_user_preference(
    State(state): State<AppState>,
    Path(key): Path<String>,
    axum::extract::Query(_params): axum::extract::Query<std::collections::HashMap<String, String>>,
) -> Response {
    let project_id: Option<&str> = None;
    let agent = state.agent.read().await;
    match agent.storage.delete_user_preference(&key, project_id).await {
        Ok(deleted) => (
            StatusCode::OK,
            Json(serde_json::json!({ "deleted": deleted })),
        )
            .into_response(),
        Err(e) => (
            StatusCode::INTERNAL_SERVER_ERROR,
            Json(ErrorResponse {
                error: e.to_string(),
            }),
        )
            .into_response(),
    }
}

pub(super) async fn list_user_data_items(
    State(state): State<AppState>,
    axum::extract::Query(params): axum::extract::Query<std::collections::HashMap<String, String>>,
) -> Response {
    let limit = params
        .get("limit")
        .and_then(|v| v.parse::<u64>().ok())
        .unwrap_or(30);
    let offset = params
        .get("offset")
        .and_then(|v| v.parse::<u64>().ok())
        .unwrap_or(0);
    let project_id: Option<&str> = None;
    let kind = params
        .get("kind")
        .map(String::as_str)
        .map(str::trim)
        .filter(|v| !v.is_empty());
    let agent = state.agent.read().await;
    match agent
        .storage
        .list_user_data_items(limit, offset, project_id, kind)
        .await
    {
        Ok(items) => {
            let total = agent
                .storage
                .count_user_data_items(project_id, kind)
                .await
                .unwrap_or(0);
            (
                StatusCode::OK,
                Json(serde_json::json!({
                    "items": items,
                    "total": total,
                    "limit": limit,
                    "offset": offset,
                })),
            )
                .into_response()
        }
        Err(e) => (
            StatusCode::INTERNAL_SERVER_ERROR,
            Json(ErrorResponse {
                error: e.to_string(),
            }),
        )
            .into_response(),
    }
}

pub(super) async fn create_user_data_item(
    State(state): State<AppState>,
    Json(payload): Json<CreateUserDataItemRequest>,
) -> Response {
    if payload.kind.trim().is_empty() || payload.title.trim().is_empty() {
        return (
            StatusCode::BAD_REQUEST,
            Json(ErrorResponse {
                error: "kind and title are required".to_string(),
            }),
        )
            .into_response();
    }

    let agent = state.agent.read().await;
    match agent
        .storage
        .create_user_data_item(crate::storage::NewUserDataItem {
            kind: payload.kind.trim(),
            title: payload.title.trim(),
            content: payload.content.trim(),
            url: payload.url.as_deref(),
            source_channel: payload.source_channel.as_deref(),
            conversation_id: payload.conversation_id.as_deref(),
            project_id: None,
            pinned: payload.pinned.unwrap_or(false),
        })
        .await
    {
        Ok(item) => (StatusCode::OK, Json(serde_json::json!({"item": item}))).into_response(),
        Err(e) => (
            StatusCode::INTERNAL_SERVER_ERROR,
            Json(ErrorResponse {
                error: e.to_string(),
            }),
        )
            .into_response(),
    }
}

pub(super) async fn delete_user_data_item(
    State(state): State<AppState>,
    Path(id): Path<String>,
) -> Response {
    let agent = state.agent.read().await;
    match agent.storage.delete_user_data_item(&id).await {
        Ok(deleted) => (
            StatusCode::OK,
            Json(serde_json::json!({ "deleted": deleted })),
        )
            .into_response(),
        Err(e) => (
            StatusCode::INTERNAL_SERVER_ERROR,
            Json(ErrorResponse {
                error: e.to_string(),
            }),
        )
            .into_response(),
    }
}

pub(super) async fn list_knowledge_items(
    State(state): State<AppState>,
    axum::extract::Query(params): axum::extract::Query<std::collections::HashMap<String, String>>,
) -> Response {
    let limit = params
        .get("limit")
        .and_then(|v| v.parse::<u64>().ok())
        .unwrap_or(30);
    let offset = params
        .get("offset")
        .and_then(|v| v.parse::<u64>().ok())
        .unwrap_or(0);
    let project_id: Option<&str> = None;

    let agent = state.agent.read().await;
    match agent
        .storage
        .list_visible_knowledge_items(limit, offset, project_id)
        .await
    {
        Ok(items) => {
            let total = agent
                .storage
                .count_visible_knowledge_items(project_id)
                .await
                .unwrap_or(0);
            (
                StatusCode::OK,
                Json(serde_json::json!({
                    "items": items,
                    "total": total,
                    "limit": limit,
                    "offset": offset,
                })),
            )
                .into_response()
        }
        Err(e) => (
            StatusCode::INTERNAL_SERVER_ERROR,
            Json(ErrorResponse {
                error: e.to_string(),
            }),
        )
            .into_response(),
    }
}

pub(super) async fn create_knowledge_item(
    State(state): State<AppState>,
    Json(payload): Json<CreateKnowledgeItemRequest>,
) -> Response {
    if payload.title.trim().is_empty() || payload.content.trim().is_empty() {
        return (
            StatusCode::BAD_REQUEST,
            Json(ErrorResponse {
                error: "title and content are required".to_string(),
            }),
        )
            .into_response();
    }

    let agent = state.agent.read().await;
    match agent
        .storage
        .create_knowledge_item(
            payload.title.trim(),
            payload.content.trim(),
            payload.source.as_deref(),
            payload.url.as_deref(),
            payload.tags.as_deref(),
            None,
        )
        .await
    {
        Ok(item) => (StatusCode::OK, Json(serde_json::json!({"item": item}))).into_response(),
        Err(e) => (
            StatusCode::INTERNAL_SERVER_ERROR,
            Json(ErrorResponse {
                error: e.to_string(),
            }),
        )
            .into_response(),
    }
}

pub(super) async fn delete_knowledge_item(
    State(state): State<AppState>,
    Path(id): Path<String>,
) -> Response {
    let agent = state.agent.read().await;
    match agent.storage.delete_knowledge_item(&id).await {
        Ok(deleted) => (
            StatusCode::OK,
            Json(serde_json::json!({ "deleted": deleted })),
        )
            .into_response(),
        Err(e) => (
            StatusCode::INTERNAL_SERVER_ERROR,
            Json(ErrorResponse {
                error: e.to_string(),
            }),
        )
            .into_response(),
    }
}

// ==================== Memory Endpoints ====================

pub(super) const ARKMEMORY_MEMORY_CANDIDATE_TYPES: &[&str] = &[
    "memory_deprecate",
    "memory_merge",
    "memory_add",
    "memory_update",
    "memory_retract",
];
pub(super) const ARKMEMORY_PENDING_CAPTURE_STATUSES: &[&str] =
    &["pending_consolidation", "processing_deferred", "processing"];
pub(super) const ARKMEMORY_FAILED_CAPTURE_STATUSES: &[&str] =
    &["failed", "failed_deferred", "rejected_sensitive_input"];
pub(super) const ARKMEMORY_REVIEWED_CAPTURE_STATUSES: &[&str] = &[
    "reviewed_failed_capture",
    "reviewed_false_positive_capture",
    "reviewed_sensitive_input",
];
pub(super) const ARKMEMORY_APPLYING_LEASE_TIMEOUT_SECS: i64 = 10 * 60;
const ARKMEMORY_LEARNED_REVIEW_FAILED_LIMIT: u64 = 50;
const ARKMEMORY_LEARNED_REVIEW_EXAMPLE_LIMIT: u64 = 200;
const ARKMEMORY_LEARNED_REVIEW_MAX_EXAMPLES: usize = 6;
const ARKMEMORY_LEARNED_REVIEW_MIN_CONFIDENCE: f64 = 0.82;
const ARKMEMORY_LEARNED_REVIEW_TIMEOUT_SECS: u64 = 8;
const ARKMEMORY_LEARNED_REVIEW_MAX_OUTPUT_TOKENS: u32 = 360;

#[derive(Debug, Deserialize)]
pub(super) struct MemoryHealthReviewRequest {
    outcome: Option<String>,
}

#[derive(Clone, Debug, Default)]
struct MemoryCaptureReviewPattern {
    similar_review_count: usize,
    expected_sensitive_skip_count: usize,
    false_positive_safe_memory_count: usize,
    acknowledged_count: usize,
}

#[derive(Clone, Debug)]
struct MemoryCaptureReviewContext {
    source_semantic_text: String,
    source_redactions: Vec<String>,
    status: String,
    capture_kind: String,
    channel: String,
    last_error_code: Option<String>,
}

#[derive(Clone, Debug)]
struct MemoryLearnedReviewExample {
    event_id: String,
    outcome: &'static str,
    reviewed_at: Option<String>,
    failure_key: String,
    context: MemoryCaptureReviewContext,
}

#[derive(Clone, Debug)]
struct MemoryLearnedReviewDecision {
    apply: bool,
    outcome: Option<String>,
    confidence: f64,
    matched_example_id: Option<String>,
    reason: String,
}

pub(super) fn arkmemory_project_param(params: &HashMap<String, String>) -> Option<&str> {
    params
        .get("project_id")
        .or_else(|| params.get("project"))
        .map(|value| value.trim())
        .filter(|value| !value.is_empty())
}

fn arkmemory_capture_event_visible_for_project(
    event: &crate::storage::memory_capture_event::Model,
    project_id: Option<&str>,
) -> bool {
    let event_project = event.project_id.as_deref().map(str::trim).unwrap_or("");
    match project_id.map(str::trim).filter(|value| !value.is_empty()) {
        Some(active_project) => event_project.is_empty() || event_project == active_project,
        None => event_project.is_empty(),
    }
}

fn arkmemory_capture_event_reviewed_status(
    event: &crate::storage::memory_capture_event::Model,
    outcome: &str,
) -> &'static str {
    if outcome == "false_positive_safe_memory" {
        return "reviewed_false_positive_capture";
    }
    let status = event
        .attempt_metadata
        .get("user_review")
        .and_then(|value| value.get("failure_signature"))
        .and_then(|value| value.get("status"))
        .and_then(|value| value.as_str())
        .or_else(|| {
            event
                .attempt_metadata
                .get("previous_status")
                .and_then(|value| value.as_str())
        })
        .unwrap_or(event.status.as_str())
        .trim();
    if status == "rejected_sensitive_input" {
        "reviewed_sensitive_input"
    } else {
        "reviewed_failed_capture"
    }
}

fn arkmemory_truncate_chars(raw: &str, max_chars: usize) -> String {
    let mut chars = raw.trim().chars();
    let mut value = chars.by_ref().take(max_chars).collect::<String>();
    if chars.next().is_some() {
        value.push_str("...");
    }
    value
}

fn arkmemory_capture_event_error_summary(
    event: &crate::storage::memory_capture_event::Model,
) -> (Option<String>, Option<String>) {
    let Some(last_error) = event
        .error_history
        .as_array()
        .and_then(|items| items.last())
        .and_then(|value| value.as_object())
    else {
        return (None, None);
    };
    let code = last_error
        .get("code")
        .and_then(|value| value.as_str())
        .map(str::trim)
        .filter(|value| !value.is_empty())
        .map(str::to_string);
    let detail = last_error
        .get("detail")
        .and_then(|value| value.as_str())
        .map(|value| crate::security::redact_secret_input(value).text)
        .map(|value| arkmemory_truncate_chars(&value, 240))
        .filter(|value| !value.is_empty());
    (code, detail)
}

fn arkmemory_known_capture_review_outcome(raw: &str) -> Option<&'static str> {
    match raw.trim() {
        "expected_sensitive_skip" => Some("expected_sensitive_skip"),
        "false_positive_safe_memory" => Some("false_positive_safe_memory"),
        "acknowledged" => Some("acknowledged"),
        _ => None,
    }
}

fn arkmemory_capture_review_context_from_source_text(
    event: &crate::storage::memory_capture_event::Model,
    source_text: &str,
) -> MemoryCaptureReviewContext {
    let redacted = crate::security::redact_secret_input(source_text);
    let (last_error_code, _) = arkmemory_capture_event_error_summary(event);
    MemoryCaptureReviewContext {
        source_semantic_text: arkmemory_truncate_chars(&redacted.text, 700),
        source_redactions: redacted.redactions,
        status: event.status.trim().to_string(),
        capture_kind: event.capture_kind.trim().to_string(),
        channel: event.channel.trim().to_string(),
        last_error_code,
    }
}

fn arkmemory_capture_review_context_from_metadata(
    event: &crate::storage::memory_capture_event::Model,
) -> Option<MemoryCaptureReviewContext> {
    let review_context = event
        .attempt_metadata
        .get("user_review")
        .and_then(|value| value.get("review_context"))?;
    let source_semantic_text = review_context
        .get("source_semantic_text")
        .and_then(|value| value.as_str())
        .map(str::trim)
        .filter(|value| !value.is_empty())?
        .to_string();
    let source_redactions = review_context
        .get("source_redactions")
        .and_then(|value| value.as_array())
        .map(|items| {
            items
                .iter()
                .filter_map(|item| item.as_str())
                .map(str::trim)
                .filter(|value| !value.is_empty())
                .map(str::to_string)
                .collect::<Vec<_>>()
        })
        .unwrap_or_default();
    Some(MemoryCaptureReviewContext {
        source_semantic_text,
        source_redactions,
        status: review_context
            .get("status")
            .and_then(|value| value.as_str())
            .unwrap_or(event.status.as_str())
            .trim()
            .to_string(),
        capture_kind: review_context
            .get("capture_kind")
            .and_then(|value| value.as_str())
            .unwrap_or(event.capture_kind.as_str())
            .trim()
            .to_string(),
        channel: review_context
            .get("channel")
            .and_then(|value| value.as_str())
            .unwrap_or(event.channel.as_str())
            .trim()
            .to_string(),
        last_error_code: review_context
            .get("last_error_code")
            .and_then(|value| value.as_str())
            .map(str::trim)
            .filter(|value| !value.is_empty())
            .map(str::to_string),
    })
}

async fn arkmemory_capture_review_context(
    storage: &crate::storage::Storage,
    event: &crate::storage::memory_capture_event::Model,
) -> Option<MemoryCaptureReviewContext> {
    if let Some(context) = arkmemory_capture_review_context_from_metadata(event) {
        return Some(context);
    }
    let source_text = match event.source_message_id.as_deref() {
        Some(message_id) if !message_id.trim().is_empty() => storage
            .get_message(message_id.trim())
            .await
            .ok()
            .flatten()
            .map(|message| message.content),
        _ => None,
    }
    .or_else(|| {
        event
            .attempt_metadata
            .get("user_message_for_link_capture")
            .and_then(|value| value.as_str())
            .map(str::to_string)
    })?;

    Some(arkmemory_capture_review_context_from_source_text(
        event,
        &source_text,
    ))
}

fn arkmemory_capture_review_context_json(
    context: &MemoryCaptureReviewContext,
) -> serde_json::Value {
    serde_json::json!({
        "source_semantic_text": context.source_semantic_text.clone(),
        "source_redactions": context.source_redactions.clone(),
        "status": context.status.clone(),
        "capture_kind": context.capture_kind.clone(),
        "channel": context.channel.clone(),
        "last_error_code": context.last_error_code.clone(),
    })
}

fn arkmemory_capture_review_context_embedding_text(context: &MemoryCaptureReviewContext) -> String {
    format!(
        "source: {}\nredactions: {}\nstatus: {}\ncapture_kind: {}\nchannel: {}\nerror: {}",
        context.source_semantic_text,
        context.source_redactions.join(", "),
        context.status,
        context.capture_kind,
        context.channel,
        context.last_error_code.as_deref().unwrap_or("")
    )
}

fn arkmemory_extract_json_object(text: &str) -> Option<serde_json::Value> {
    let trimmed = text.trim();
    if let Ok(value) = serde_json::from_str::<serde_json::Value>(trimmed) {
        return Some(value);
    }
    let start = trimmed.find('{')?;
    let end = trimmed.rfind('}')?;
    if end <= start {
        return None;
    }
    serde_json::from_str::<serde_json::Value>(&trimmed[start..=end]).ok()
}

fn arkmemory_parse_learned_review_decision(text: &str) -> Option<MemoryLearnedReviewDecision> {
    let json = arkmemory_extract_json_object(text)?;
    Some(MemoryLearnedReviewDecision {
        apply: json
            .get("apply")
            .and_then(|value| value.as_bool())
            .unwrap_or(false),
        outcome: json
            .get("outcome")
            .and_then(|value| value.as_str())
            .and_then(arkmemory_known_capture_review_outcome)
            .map(str::to_string),
        confidence: json
            .get("confidence")
            .and_then(|value| {
                value.as_f64().or_else(|| {
                    value
                        .as_str()
                        .and_then(|raw| raw.trim().parse::<f64>().ok())
                })
            })
            .unwrap_or(0.0)
            .clamp(0.0, 1.0),
        matched_example_id: json
            .get("matched_example_id")
            .and_then(|value| value.as_str())
            .map(str::trim)
            .filter(|value| !value.is_empty() && *value != "null")
            .map(str::to_string),
        reason: json
            .get("reason")
            .and_then(|value| value.as_str())
            .map(|value| {
                arkmemory_truncate_chars(&crate::security::redact_secret_input(value).text, 300)
            })
            .unwrap_or_default(),
    })
}

fn arkmemory_capture_event_semantic_key(
    event: &crate::storage::memory_capture_event::Model,
) -> String {
    event
        .attempt_metadata
        .get("semantic_capture_key")
        .and_then(|value| value.as_str())
        .map(str::trim)
        .filter(|value| !value.is_empty())
        .map(str::to_string)
        .unwrap_or_else(|| {
            event
                .source_hash
                .trim()
                .strip_prefix("pending:")
                .unwrap_or_else(|| event.source_hash.trim())
                .to_string()
        })
}

fn arkmemory_capture_event_timestamp(raw: &str) -> Option<chrono::DateTime<chrono::Utc>> {
    chrono::DateTime::parse_from_rfc3339(raw.trim())
        .ok()
        .map(|value| value.with_timezone(&chrono::Utc))
}

fn arkmemory_pending_capture_group_time_bounds(
    events: &[crate::storage::memory_capture_event::Model],
) -> (String, String) {
    let created_at = events
        .iter()
        .min_by_key(|event| event.created_at.as_str())
        .map(|event| event.created_at.clone())
        .unwrap_or_default();
    let updated_at = events
        .iter()
        .max_by_key(|event| event.updated_at.as_str())
        .map(|event| event.updated_at.clone())
        .unwrap_or_default();
    (created_at, updated_at)
}

async fn arkmemory_pending_capture_group_payload(
    storage: &crate::storage::Storage,
    semantic_key: String,
    mut events: Vec<crate::storage::memory_capture_event::Model>,
) -> serde_json::Value {
    events.sort_by(|left, right| right.updated_at.cmp(&left.updated_at));
    let primary = events
        .iter()
        .find(|event| {
            event
                .source_message_id
                .as_deref()
                .map(str::trim)
                .is_some_and(|value| !value.is_empty())
        })
        .unwrap_or_else(|| {
            events
                .first()
                .expect("pending capture group must contain at least one event")
        });
    let source_context = arkmemory_capture_source_context(storage, primary).await;
    let (created_at, updated_at) = arkmemory_pending_capture_group_time_bounds(&events);
    let now = chrono::Utc::now();
    let age_seconds = arkmemory_capture_event_timestamp(&created_at)
        .map(|created| (now - created).num_seconds().max(0));
    let mut statuses = Vec::<String>::new();
    let event_rows = events
        .iter()
        .map(|event| {
            let status = event.status.trim().to_string();
            if !status.is_empty() && !statuses.iter().any(|known| known == &status) {
                statuses.push(status.clone());
            }
            let message_chars = event
                .attempt_metadata
                .get("message_chars")
                .and_then(|value| value.as_u64());
            serde_json::json!({
                "id": event.id.clone(),
                "status": status,
                "capture_kind": event.capture_kind.clone(),
                "channel": event.channel.clone(),
                "source_message_id": event.source_message_id.clone(),
                "created_at": event.created_at.clone(),
                "updated_at": event.updated_at.clone(),
                "message_chars": message_chars,
                "replay_count": event.replay_count,
            })
        })
        .collect::<Vec<_>>();
    serde_json::json!({
        "id": semantic_key,
        "semantic_capture_key": semantic_key,
        "status": statuses.first().cloned().unwrap_or_default(),
        "statuses": statuses,
        "event_count": event_rows.len(),
        "events": event_rows,
        "source_context": source_context,
        "created_at": created_at,
        "updated_at": updated_at,
        "age_seconds": age_seconds,
    })
}

async fn arkmemory_pending_capture_signal_payloads(
    storage: &crate::storage::Storage,
    project_id: Option<&str>,
    limit: u64,
) -> Result<Vec<serde_json::Value>> {
    let events = storage
        .list_memory_capture_events_by_statuses(
            ARKMEMORY_PENDING_CAPTURE_STATUSES,
            project_id,
            limit.saturating_mul(2).clamp(50, 200),
        )
        .await?;
    let mut grouped =
        std::collections::HashMap::<String, Vec<crate::storage::memory_capture_event::Model>>::new(
        );
    for event in events {
        grouped
            .entry(arkmemory_capture_event_semantic_key(&event))
            .or_default()
            .push(event);
    }
    let mut payloads = Vec::with_capacity(grouped.len());
    for (semantic_key, events) in grouped {
        payloads.push(arkmemory_pending_capture_group_payload(storage, semantic_key, events).await);
    }
    payloads.sort_by(|left, right| {
        let left_updated = left
            .get("updated_at")
            .and_then(|value| value.as_str())
            .unwrap_or_default();
        let right_updated = right
            .get("updated_at")
            .and_then(|value| value.as_str())
            .unwrap_or_default();
        right_updated.cmp(left_updated)
    });
    payloads.truncate(limit as usize);
    Ok(payloads)
}

fn arkmemory_capture_review_outcome_label(outcome: &str) -> &'static str {
    match outcome {
        "expected_sensitive_skip" => "Correct secret-like skip",
        "false_positive_safe_memory" => "False positive",
        _ => "Reviewed",
    }
}

fn arkmemory_default_capture_review_outcome(
    event: &crate::storage::memory_capture_event::Model,
) -> &'static str {
    if event.status.trim() == "rejected_sensitive_input" {
        "expected_sensitive_skip"
    } else {
        "acknowledged"
    }
}

fn arkmemory_normalize_capture_review_outcome(
    raw: Option<&str>,
    event: &crate::storage::memory_capture_event::Model,
) -> &'static str {
    match raw.map(str::trim) {
        Some("expected_sensitive_skip") => "expected_sensitive_skip",
        Some("false_positive_safe_memory") => "false_positive_safe_memory",
        Some("acknowledged") => "acknowledged",
        _ => arkmemory_default_capture_review_outcome(event),
    }
}

fn arkmemory_capture_failure_signature(
    event: &crate::storage::memory_capture_event::Model,
) -> serde_json::Value {
    let (last_error_code, _) = arkmemory_capture_event_error_summary(event);
    serde_json::json!({
        "status": event.status.trim(),
        "capture_kind": event.capture_kind.trim(),
        "channel": event.channel.trim(),
        "last_error_code": last_error_code,
    })
}

fn arkmemory_capture_review_failure_signature(
    event: &crate::storage::memory_capture_event::Model,
) -> serde_json::Value {
    event
        .attempt_metadata
        .get("user_review")
        .and_then(|value| value.get("failure_signature"))
        .cloned()
        .filter(|value| value.is_object())
        .unwrap_or_else(|| arkmemory_capture_failure_signature(event))
}

fn arkmemory_capture_failure_signature_key(
    status: &str,
    capture_kind: &str,
    channel: &str,
    last_error_code: Option<&str>,
) -> String {
    arkmemory_stable_event_id(&[
        "capture_review_pattern",
        status.trim(),
        capture_kind.trim(),
        channel.trim(),
        last_error_code.unwrap_or("").trim(),
    ])
}

fn arkmemory_capture_event_failure_signature_key(
    event: &crate::storage::memory_capture_event::Model,
) -> String {
    let (last_error_code, _) = arkmemory_capture_event_error_summary(event);
    arkmemory_capture_failure_signature_key(
        event.status.as_str(),
        event.capture_kind.as_str(),
        event.channel.as_str(),
        last_error_code.as_deref(),
    )
}

fn arkmemory_reviewed_capture_failure_signature_key(
    event: &crate::storage::memory_capture_event::Model,
) -> Option<String> {
    let metadata = event.attempt_metadata.as_object()?;
    let review = metadata.get("user_review")?.as_object()?;
    let signature = review.get("failure_signature")?.as_object()?;
    let status = signature.get("status")?.as_str()?;
    let capture_kind = signature.get("capture_kind")?.as_str()?;
    let channel = signature.get("channel")?.as_str().unwrap_or("");
    let last_error_code = signature
        .get("last_error_code")
        .and_then(|value| value.as_str());
    Some(arkmemory_capture_failure_signature_key(
        status,
        capture_kind,
        channel,
        last_error_code,
    ))
}

fn arkmemory_capture_review_pattern_summary(
    reviewed_events: &[crate::storage::memory_capture_event::Model],
) -> std::collections::HashMap<String, MemoryCaptureReviewPattern> {
    let mut patterns = std::collections::HashMap::<String, MemoryCaptureReviewPattern>::new();
    for event in reviewed_events {
        let Some(key) = arkmemory_reviewed_capture_failure_signature_key(event) else {
            continue;
        };
        let outcome = event
            .attempt_metadata
            .get("user_review")
            .and_then(|value| value.get("outcome"))
            .and_then(|value| value.as_str())
            .unwrap_or("acknowledged");
        let pattern = patterns.entry(key).or_default();
        pattern.similar_review_count += 1;
        match outcome {
            "expected_sensitive_skip" => pattern.expected_sensitive_skip_count += 1,
            "false_positive_safe_memory" => pattern.false_positive_safe_memory_count += 1,
            _ => pattern.acknowledged_count += 1,
        }
    }
    patterns
}

fn arkmemory_capture_review_pattern_payload(
    pattern: Option<&MemoryCaptureReviewPattern>,
) -> serde_json::Value {
    let Some(pattern) = pattern else {
        return serde_json::Value::Null;
    };
    let suggested_outcome = [
        (
            "expected_sensitive_skip",
            pattern.expected_sensitive_skip_count,
        ),
        (
            "false_positive_safe_memory",
            pattern.false_positive_safe_memory_count,
        ),
        ("acknowledged", pattern.acknowledged_count),
    ]
    .into_iter()
    .max_by_key(|(_, count)| *count)
    .filter(|(_, count)| *count > 0)
    .map(|(outcome, _)| outcome);
    serde_json::json!({
        "similar_review_count": pattern.similar_review_count,
        "expected_sensitive_skip_count": pattern.expected_sensitive_skip_count,
        "false_positive_safe_memory_count": pattern.false_positive_safe_memory_count,
        "acknowledged_count": pattern.acknowledged_count,
        "suggested_outcome": suggested_outcome,
    })
}

fn arkmemory_safe_source_message_preview(message: &crate::storage::message::Model) -> String {
    let redacted = crate::security::redact_secret_input(&message.content);
    if redacted.is_mostly_secret_payload() {
        return "Message preview hidden because the source appears to be mostly credential-like material.".to_string();
    }
    let preview = arkmemory_truncate_chars(&redacted.text, 180);
    if preview.trim().is_empty() {
        "Message preview unavailable.".to_string()
    } else {
        preview
    }
}

async fn arkmemory_capture_source_context(
    storage: &crate::storage::Storage,
    event: &crate::storage::memory_capture_event::Model,
) -> serde_json::Value {
    let source_message = match event.source_message_id.as_deref() {
        Some(message_id) if !message_id.trim().is_empty() => {
            storage.get_message(message_id.trim()).await.ok().flatten()
        }
        _ => None,
    };
    let conversation = match event.conversation_id.as_deref() {
        Some(conversation_id) if !conversation_id.trim().is_empty() => storage
            .get_conversation(conversation_id.trim())
            .await
            .ok()
            .flatten(),
        _ => None,
    };
    let message_chars = source_message
        .as_ref()
        .map(|message| message.content.chars().count())
        .or_else(|| {
            event
                .attempt_metadata
                .get("message_chars")
                .and_then(|value| value.as_u64())
                .and_then(|value| usize::try_from(value).ok())
        });
    serde_json::json!({
        "conversation_id": event.conversation_id.clone(),
        "conversation_title": conversation
            .as_ref()
            .map(|conversation| conversation.title.trim())
            .filter(|title| !title.is_empty()),
        "conversation_channel": conversation
            .as_ref()
            .map(|conversation| conversation.channel.trim())
            .filter(|channel| !channel.is_empty())
            .or_else(|| Some(event.channel.trim()).filter(|channel| !channel.is_empty())),
        "source_message_id": event.source_message_id.clone(),
        "source_message_at": source_message.as_ref().map(|message| message.timestamp.as_str()),
        "source_message_role": source_message.as_ref().map(|message| message.role.as_str()),
        "source_message_chars": message_chars,
        "source_message_preview": source_message
            .as_ref()
            .map(arkmemory_safe_source_message_preview),
    })
}

fn arkmemory_json_evidence_ref_value(
    evidence_refs: &serde_json::Value,
    prefix: &str,
) -> Option<String> {
    evidence_refs.as_array().and_then(|items| {
        items.iter().find_map(|item| {
            item.as_str()
                .and_then(|value| value.trim().strip_prefix(prefix))
                .map(str::trim)
                .filter(|value| !value.is_empty())
                .map(str::to_string)
        })
    })
}

fn arkmemory_memory_operation_source_context(
    operation: &crate::storage::memory_operation::Model,
) -> serde_json::Value {
    let capture_event_id = operation
        .capture_event_id
        .as_deref()
        .map(str::trim)
        .filter(|value| !value.is_empty())
        .map(str::to_string)
        .or_else(|| arkmemory_json_evidence_ref_value(&operation.evidence_refs, "capture_event:"));
    let source_message_id = arkmemory_json_evidence_ref_value(&operation.evidence_refs, "message:");
    let channel = arkmemory_json_evidence_ref_value(&operation.evidence_refs, "channel:");
    serde_json::json!({
        "conversation_id": operation.conversation_id.clone(),
        "conversation_channel": channel.as_deref().map(str::trim).filter(|value| !value.is_empty()),
        "capture_event_id": capture_event_id,
        "source_message_id": source_message_id,
    })
}

fn arkmemory_capture_event_finding(
    event: crate::storage::memory_capture_event::Model,
    review_pattern: Option<&MemoryCaptureReviewPattern>,
    source_context: serde_json::Value,
) -> serde_json::Value {
    let capture_event_id = event.id.clone();
    let (last_error_code, last_error_detail) = arkmemory_capture_event_error_summary(&event);
    let is_sensitive_rejection = event.status.trim() == "rejected_sensitive_input";
    let title = if is_sensitive_rejection {
        "Memory capture skipped for secret-like input"
    } else {
        "Memory capture failed"
    };
    let detail = if is_sensitive_rejection {
        "AgentArk did not turn this chat message into memory because the source looked like credential or token material. No memory was stored from that message."
    } else {
        "A user-memory capture event ended before it could produce an auditable operation. Review model/provider health and the recorded error before marking it reviewed."
    };
    serde_json::json!({
        "id": format!("capture_failed:{}", capture_event_id),
        "kind": "capture_failed",
        "severity": "warning",
        "capture_event_id": capture_event_id,
        "status": event.status,
        "capture_kind": event.capture_kind,
        "conversation_id": event.conversation_id,
        "source_message_id": event.source_message_id,
        "replay_count": event.replay_count,
        "last_error_code": last_error_code,
        "last_error_detail": last_error_detail,
        "review_pattern": arkmemory_capture_review_pattern_payload(review_pattern),
        "source_context": source_context,
        "title": title,
        "detail": detail,
        "action": "review_capture_pipeline",
        "created_at": event.updated_at,
    })
}

fn arkmemory_capture_event_learned_review_finding(
    event: crate::storage::memory_capture_event::Model,
    source_context: serde_json::Value,
) -> serde_json::Value {
    let capture_event_id = event.id.clone();
    let learned_review = event
        .attempt_metadata
        .get("learned_review")
        .cloned()
        .unwrap_or(serde_json::Value::Null);
    let review = event
        .attempt_metadata
        .get("user_review")
        .cloned()
        .unwrap_or(serde_json::Value::Null);
    let outcome = review
        .get("outcome")
        .and_then(|value| value.as_str())
        .unwrap_or("acknowledged")
        .to_string();
    let title = match outcome.as_str() {
        "expected_sensitive_skip" => "Memory capture auto-marked as correct secret-like skip",
        "false_positive_safe_memory" => "Memory capture auto-marked as false positive",
        _ => "Memory capture auto-marked reviewed",
    };
    let confidence = learned_review
        .get("confidence")
        .and_then(|value| value.as_f64());
    let original_status = review
        .get("failure_signature")
        .and_then(|value| value.get("status"))
        .and_then(|value| value.as_str())
        .unwrap_or_default()
        .trim();
    let can_correct_sensitive_skip = original_status == "rejected_sensitive_input"
        || matches!(
            outcome.as_str(),
            "expected_sensitive_skip" | "false_positive_safe_memory"
        );
    let reason = learned_review
        .get("reason")
        .and_then(|value| value.as_str())
        .map(str::trim)
        .filter(|value| !value.is_empty())
        .map(str::to_string)
        .unwrap_or_else(|| {
            "The learned reviewer matched this to prior human feedback.".to_string()
        });
    let detail = match confidence {
        Some(value) => format!(
            "Memory auto-applied learned review feedback with {:.0}% confidence. You can confirm or correct this outcome.",
            (value * 100.0).clamp(0.0, 100.0)
        ),
        None => {
            "Memory auto-applied learned review feedback. You can confirm or correct this outcome."
                .to_string()
        }
    };
    serde_json::json!({
        "id": format!("auto_reviewed_capture:{}", capture_event_id),
        "kind": "auto_reviewed_capture",
        "severity": "info",
        "capture_event_id": capture_event_id,
        "status": event.status,
        "capture_kind": event.capture_kind,
        "conversation_id": event.conversation_id,
        "source_message_id": event.source_message_id,
        "replay_count": event.replay_count,
        "review": review,
        "review_outcome": outcome,
        "learned_review": learned_review,
        "can_correct_sensitive_skip": can_correct_sensitive_skip,
        "source_context": source_context,
        "title": title,
        "detail": detail,
        "last_error_detail": reason,
        "action": "review_learned_capture_review",
        "created_at": event.updated_at,
    })
}

pub(super) fn arkmemory_limit(params: &HashMap<String, String>, default_limit: u64) -> u64 {
    params
        .get("limit")
        .and_then(|value| value.parse::<u64>().ok())
        .unwrap_or(default_limit)
        .clamp(1, 200)
}

pub(super) fn arkmemory_offset(params: &HashMap<String, String>) -> u64 {
    params
        .get("offset")
        .and_then(|value| value.parse::<u64>().ok())
        .unwrap_or(0)
}

pub(super) fn arkmemory_candidate_is_memory(candidate_type: &str) -> bool {
    ARKMEMORY_MEMORY_CANDIDATE_TYPES.contains(&candidate_type)
}

pub(super) fn arkmemory_item_is_memory(item: &crate::storage::experience_item::Model) -> bool {
    matches!(item.kind.as_str(), "personal_fact" | "constraint")
}

pub(super) fn arkmemory_item_visible_for_project(
    item: &crate::storage::experience_item::Model,
    _project_id: Option<&str>,
) -> bool {
    let _ = item;
    true
}

#[derive(Debug, Clone, Serialize)]
pub(super) struct ArkmemoryGraphNode {
    pub id: String,
    pub node_type: String,
    pub label: String,
    pub detail: String,
    pub category: Option<String>,
    pub status: Option<String>,
    pub memory_kind: Option<String>,
    pub confidence: Option<f64>,
    pub support_count: Option<i32>,
    pub stale: bool,
    pub pinned: bool,
    pub updated_at: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub ref_kind: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub metadata: Option<serde_json::Value>,
}

#[derive(Debug, Clone, Serialize)]
pub(super) struct ArkmemoryGraphEdge {
    pub id: String,
    pub source: String,
    pub target: String,
    pub edge_type: String,
    pub label: String,
    pub detail: String,
    pub weight: f64,
    pub semantic: bool,
    pub explicit: bool,
    pub updated_at: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub metadata: Option<serde_json::Value>,
}

fn arkmemory_graph_csv_filter(raw: &str) -> Vec<String> {
    let mut seen = HashSet::new();
    let mut values = Vec::new();
    for value in raw.split(',') {
        let normalized = value.trim();
        if normalized.is_empty() || !seen.insert(normalized.to_string()) {
            continue;
        }
        values.push(normalized.to_string());
    }
    values
}

fn arkmemory_graph_params_filter(params: &HashMap<String, String>, key: &str) -> Vec<String> {
    params
        .get(key)
        .map(|value| arkmemory_graph_csv_filter(value))
        .unwrap_or_default()
}

fn arkmemory_graph_pair_key(left: &str, right: &str) -> String {
    if left <= right {
        format!("{left}\n{right}")
    } else {
        format!("{right}\n{left}")
    }
}

fn arkmemory_graph_memory_category(item: &crate::storage::experience_item::Model) -> String {
    let semantic_kind = item
        .metadata
        .get("memory_kind")
        .and_then(|value| value.as_str());
    crate::core::memory_schema::memory_category_from_metadata(&item.metadata, semantic_kind)
        .to_string()
}

fn arkmemory_graph_memory_kind(item: &crate::storage::experience_item::Model) -> Option<String> {
    item.metadata
        .get("memory_kind")
        .and_then(|value| value.as_str())
        .map(str::trim)
        .filter(|value| !value.is_empty())
        .map(str::to_string)
}

fn arkmemory_graph_memory_node(
    item: &crate::storage::experience_item::Model,
    pinned: bool,
) -> ArkmemoryGraphNode {
    let key = item
        .metadata
        .get("key")
        .and_then(|value| value.as_str())
        .map(str::trim)
        .filter(|value| !value.is_empty());
    let label = key
        .or_else(|| {
            item.title
                .trim()
                .split_once(':')
                .map(|(left, _)| left.trim())
                .filter(|value| !value.is_empty())
        })
        .unwrap_or_else(|| item.title.trim())
        .trim();
    let label = if label.is_empty() {
        item.id.as_str()
    } else {
        label
    };
    ArkmemoryGraphNode {
        id: item.id.clone(),
        node_type: "memory".to_string(),
        label: label.to_string(),
        detail: item.content.clone(),
        category: Some(arkmemory_graph_memory_category(item)),
        status: Some(item.status.clone()),
        memory_kind: arkmemory_graph_memory_kind(item),
        confidence: Some(item.confidence),
        support_count: Some(item.support_count),
        stale: item.status != "active",
        pinned,
        updated_at: Some(item.updated_at.clone()),
        ref_kind: None,
        metadata: Some(item.metadata.clone()),
    }
}

fn arkmemory_graph_ref_node(id: String, ref_kind: &str, label: String) -> ArkmemoryGraphNode {
    ArkmemoryGraphNode {
        id,
        node_type: "source".to_string(),
        label,
        detail: ref_kind.to_string(),
        category: None,
        status: None,
        memory_kind: None,
        confidence: None,
        support_count: None,
        stale: false,
        pinned: false,
        updated_at: None,
        ref_kind: Some(ref_kind.to_string()),
        metadata: None,
    }
}

fn arkmemory_graph_edge_label(edge_type: &str) -> String {
    if edge_type == "semantic_nearby" {
        return "Semantic".to_string();
    }
    edge_type
        .split(['_', '-'])
        .filter(|part| !part.trim().is_empty())
        .map(|part| {
            let mut chars = part.chars();
            match chars.next() {
                Some(first) => first.to_uppercase().collect::<String>() + chars.as_str(),
                None => String::new(),
            }
        })
        .collect::<Vec<_>>()
        .join(" ")
}

fn arkmemory_graph_explicit_edge(
    edge: &crate::storage::experience_edge::Model,
) -> ArkmemoryGraphEdge {
    ArkmemoryGraphEdge {
        id: edge.id.clone(),
        source: edge.source_ref.clone(),
        target: edge.target_ref.clone(),
        edge_type: edge.edge_type.clone(),
        label: arkmemory_graph_edge_label(&edge.edge_type),
        detail: edge.edge_type.clone(),
        weight: edge.weight,
        semantic: false,
        explicit: true,
        updated_at: Some(edge.updated_at.clone()),
        metadata: Some(edge.metadata.clone()),
    }
}

fn arkmemory_graph_ref_edge(
    memory_id: &str,
    target_id: &str,
    edge_type: &str,
    detail: String,
) -> ArkmemoryGraphEdge {
    ArkmemoryGraphEdge {
        id: arkmemory_stable_event_id(&["graph_edge", memory_id, target_id, edge_type]),
        source: memory_id.to_string(),
        target: target_id.to_string(),
        edge_type: edge_type.to_string(),
        label: arkmemory_graph_edge_label(edge_type),
        detail,
        weight: 0.45,
        semantic: false,
        explicit: true,
        updated_at: None,
        metadata: None,
    }
}

fn arkmemory_graph_semantic_edges(
    items: &[crate::storage::experience_item::Model],
    explicit_pairs: &HashSet<String>,
    threshold: f64,
    per_node_limit: usize,
    global_limit: usize,
) -> Vec<ArkmemoryGraphEdge> {
    let mut scored: Vec<(String, String, f64)> = Vec::new();
    for (left_index, left) in items.iter().enumerate() {
        let Some(left_embedding) = left.embedding.as_ref() else {
            continue;
        };
        for right in items.iter().skip(left_index + 1) {
            let Some(right_embedding) = right.embedding.as_ref() else {
                continue;
            };
            let pair_key = arkmemory_graph_pair_key(&left.id, &right.id);
            if explicit_pairs.contains(&pair_key) {
                continue;
            }
            let Some(score) = crate::core::document_search::normalized_embedding_similarity(
                left_embedding.as_slice(),
                right_embedding.as_slice(),
            ) else {
                continue;
            };
            let score = f64::from(score);
            if score >= threshold {
                scored.push((left.id.clone(), right.id.clone(), score.clamp(0.0, 1.0)));
            }
        }
    }
    scored.sort_by(|left, right| right.2.total_cmp(&left.2));
    let mut per_node_counts: HashMap<String, usize> = HashMap::new();
    let mut edges = Vec::new();
    for (source, target, score) in scored {
        if edges.len() >= global_limit {
            break;
        }
        let source_count = per_node_counts.get(&source).copied().unwrap_or(0);
        let target_count = per_node_counts.get(&target).copied().unwrap_or(0);
        if source_count >= per_node_limit || target_count >= per_node_limit {
            continue;
        }
        per_node_counts.insert(source.clone(), source_count + 1);
        per_node_counts.insert(target.clone(), target_count + 1);
        let score_label = format!("{:.0}% embedding similarity", score * 100.0);
        edges.push(ArkmemoryGraphEdge {
            id: arkmemory_stable_event_id(&["semantic_nearby", &source, &target]),
            source,
            target,
            edge_type: "semantic_nearby".to_string(),
            label: "Semantic".to_string(),
            detail: score_label,
            weight: score,
            semantic: true,
            explicit: false,
            updated_at: None,
            metadata: Some(serde_json::json!({ "score": score })),
        });
    }
    edges
}

#[derive(Default)]
pub(super) struct MemoryEventContext {
    scope: Option<String>,
    project_id: Option<String>,
    conversation_id: Option<String>,
    source_ref: Option<String>,
}

impl MemoryEventContext {
    fn from_memory(item: &crate::storage::experience_item::Model) -> Self {
        Self {
            scope: Some(item.scope.clone()),
            project_id: None,
            conversation_id: item.conversation_id.clone(),
            source_ref: Some(item.id.clone()),
        }
    }

    fn from_candidate(candidate: &crate::storage::learning_candidate::Model) -> Self {
        Self {
            scope: None,
            project_id: None,
            conversation_id: candidate.conversation_id.clone(),
            source_ref: Some(candidate.id.clone()),
        }
    }

    fn from_operation(operation: &crate::storage::memory_operation::Model) -> Self {
        Self {
            scope: Some(operation.scope.clone()),
            project_id: operation.project_id.clone(),
            conversation_id: operation.conversation_id.clone(),
            source_ref: Some(operation.id.clone()),
        }
    }
}

pub(super) fn arkmemory_candidate_is_stale_applying(
    candidate: &crate::storage::learning_candidate::Model,
) -> bool {
    if candidate.approval_status != "applying" {
        return false;
    }
    chrono::DateTime::parse_from_rfc3339(&candidate.updated_at)
        .map(|updated_at| {
            (chrono::Utc::now() - updated_at.with_timezone(&chrono::Utc)).num_seconds()
                >= ARKMEMORY_APPLYING_LEASE_TIMEOUT_SECS
        })
        .unwrap_or(false)
}

pub(super) async fn arkmemory_visible_open_memory_candidates(
    storage: &crate::storage::Storage,
    project_id: Option<&str>,
    limit: u64,
) -> Result<Vec<crate::storage::learning_candidate::Model>> {
    let fetch_limit = limit.saturating_mul(2).clamp(50, 200);
    let rows = storage
        .list_learning_candidates_for_review(
            &["draft", "applying"],
            ARKMEMORY_MEMORY_CANDIDATE_TYPES,
            project_id,
            fetch_limit,
        )
        .await?;
    let mut visible = Vec::new();
    for mut candidate in rows {
        if candidate.approval_status == "draft" {
            visible.push(candidate);
        } else if arkmemory_candidate_is_stale_applying(&candidate) {
            let reset = storage
                .update_learning_candidate_review_if_status(
                    &candidate.id,
                    "applying",
                    "draft",
                    Some("Reset stale Memory apply claim."),
                    None,
                )
                .await?;
            if reset {
                candidate.approval_status = "draft".to_string();
                candidate.review_notes = Some("Reset stale Memory apply claim.".to_string());
                candidate.updated_at = chrono::Utc::now().to_rfc3339();
                visible.push(candidate);
            }
        } else {
            visible.push(candidate);
        }
    }
    visible.sort_by(|left, right| right.updated_at.cmp(&left.updated_at));
    Ok(visible)
}

pub(super) async fn arkmemory_latest_open_candidate_for_subject(
    storage: &crate::storage::Storage,
    candidate: &crate::storage::learning_candidate::Model,
) -> Result<Option<crate::storage::learning_candidate::Model>> {
    let rows = storage
        .list_learning_candidates_for_subject_key(
            &candidate.subject_key,
            ARKMEMORY_MEMORY_CANDIDATE_TYPES,
            None,
            32,
        )
        .await?;
    let mut open = Vec::new();
    for mut row in rows {
        if row.approval_status == "draft" {
            open.push(row);
        } else if arkmemory_candidate_is_stale_applying(&row) {
            let reset = storage
                .update_learning_candidate_review_if_status(
                    &row.id,
                    "applying",
                    "draft",
                    Some("Reset stale Memory apply claim."),
                    None,
                )
                .await?;
            if reset {
                row.approval_status = "draft".to_string();
                row.review_notes = Some("Reset stale Memory apply claim.".to_string());
                row.updated_at = chrono::Utc::now().to_rfc3339();
                open.push(row);
            }
        } else if row.approval_status == "applying" {
            open.push(row);
        }
    }
    open.sort_by(|left, right| right.updated_at.cmp(&left.updated_at));
    Ok(open.into_iter().next())
}

pub(super) async fn arkmemory_ensure_latest_open_candidate(
    storage: &crate::storage::Storage,
    candidate: &crate::storage::learning_candidate::Model,
) -> Result<crate::storage::learning_candidate::Model> {
    let latest = arkmemory_latest_open_candidate_for_subject(storage, candidate)
        .await?
        .ok_or_else(|| anyhow::anyhow!("Memory queue item is no longer pending review."))?;
    if latest.id != candidate.id {
        anyhow::bail!("A newer memory queue item exists for this subject.");
    }
    Ok(latest)
}

pub(super) fn arkmemory_memory_sources(
    item: &crate::storage::experience_item::Model,
) -> Vec<String> {
    let mut sources = Vec::new();
    let metadata = item.metadata.as_object();
    if let Some(object) = metadata {
        for key in ["source", "sources", "source_refs", "evidence_refs"] {
            if let Some(value) = object.get(key) {
                match value {
                    serde_json::Value::String(raw) if !raw.trim().is_empty() => {
                        sources.push(raw.trim().to_string());
                    }
                    serde_json::Value::Array(values) => {
                        for entry in values {
                            if let Some(raw) =
                                entry.as_str().map(str::trim).filter(|v| !v.is_empty())
                            {
                                sources.push(raw.to_string());
                            }
                        }
                    }
                    _ => {}
                }
            }
        }
    }
    sources.sort();
    sources.dedup();
    sources
}

pub(super) fn arkmemory_stable_event_id(parts: &[&str]) -> String {
    let mut hasher = Sha256::new();
    for part in parts {
        hasher.update((part.len() as u64).to_be_bytes());
        hasher.update(part.as_bytes());
    }
    format!("arkmemory-event-{}", hex::encode(hasher.finalize()))
}

pub(super) fn arkmemory_candidate_payload(
    candidate: &crate::storage::learning_candidate::Model,
    replay_gate: Option<&crate::core::self_evolve::replay_gate::CandidateReplayGateResult>,
) -> serde_json::Value {
    serde_json::json!({
        "id": candidate.id,
        "candidate_type": candidate.candidate_type,
        "subject_key": candidate.subject_key,
        "title": candidate.title,
        "summary": candidate.summary,
        "conversation_id": candidate.conversation_id,
        "evidence_refs": candidate.evidence_refs,
        "proposed_content": candidate.proposed_content,
        "confidence": candidate.confidence,
        "approval_status": candidate.approval_status,
        "review_notes": candidate.review_notes,
        "reviewed_at": candidate.reviewed_at,
        "approved_ref": candidate.approved_ref,
        "replay_gate": replay_gate,
        "created_at": candidate.created_at,
        "updated_at": candidate.updated_at,
    })
}

pub(super) async fn arkmemory_list_memory_candidates(
    storage: &crate::storage::Storage,
    project_id: Option<&str>,
    limit: u64,
) -> Result<Vec<crate::storage::learning_candidate::Model>> {
    let rows = arkmemory_visible_open_memory_candidates(storage, project_id, limit).await?;
    let mut visible = Vec::new();
    let mut seen_subjects = std::collections::HashSet::new();
    for candidate in rows {
        if !seen_subjects.insert(candidate.subject_key.clone()) {
            continue;
        }
        visible.push(candidate);
        if visible.len() >= limit as usize {
            break;
        }
    }
    Ok(visible)
}

fn arkmemory_capture_review_outcome_from_event(
    event: &crate::storage::memory_capture_event::Model,
) -> Option<&'static str> {
    let review = event.attempt_metadata.get("user_review")?;
    if review
        .get("learned")
        .and_then(|value| value.as_bool())
        .unwrap_or(false)
    {
        return None;
    }
    review
        .get("outcome")
        .and_then(|value| value.as_str())
        .and_then(arkmemory_known_capture_review_outcome)
}

fn arkmemory_capture_event_has_learned_review(
    event: &crate::storage::memory_capture_event::Model,
) -> bool {
    event
        .attempt_metadata
        .get("user_review")
        .and_then(|value| value.get("learned"))
        .and_then(|value| value.as_bool())
        .unwrap_or(false)
        || event
            .attempt_metadata
            .get("reviewed_from")
            .and_then(|value| value.as_str())
            .map(str::trim)
            == Some("arkmemory_learned_review")
}

fn arkmemory_learned_review_example_compatible(
    event: &crate::storage::memory_capture_event::Model,
    example: &MemoryLearnedReviewExample,
) -> bool {
    example.context.status == event.status.trim()
        && example.context.capture_kind == event.capture_kind.trim()
}

async fn arkmemory_learned_review_examples(
    storage: &crate::storage::Storage,
) -> Result<Vec<MemoryLearnedReviewExample>> {
    let reviewed_events = storage
        .list_memory_capture_events_by_statuses_all_scopes(
            ARKMEMORY_REVIEWED_CAPTURE_STATUSES,
            ARKMEMORY_LEARNED_REVIEW_EXAMPLE_LIMIT,
        )
        .await?;
    let mut examples = Vec::new();
    for event in reviewed_events {
        let Some(outcome) = arkmemory_capture_review_outcome_from_event(&event) else {
            continue;
        };
        let Some(failure_key) = arkmemory_reviewed_capture_failure_signature_key(&event) else {
            continue;
        };
        let Some(context) = arkmemory_capture_review_context(storage, &event).await else {
            continue;
        };
        examples.push(MemoryLearnedReviewExample {
            event_id: event.id.clone(),
            outcome,
            reviewed_at: event
                .attempt_metadata
                .get("user_review")
                .and_then(|value| value.get("reviewed_at"))
                .and_then(|value| value.as_str())
                .map(str::to_string),
            failure_key,
            context,
        });
    }
    Ok(examples)
}

async fn arkmemory_rank_learned_review_examples(
    embedder: Option<&crate::core::embeddings::EmbeddingClient>,
    current: &MemoryCaptureReviewContext,
    examples: &[MemoryLearnedReviewExample],
) -> Vec<MemoryLearnedReviewExample> {
    if examples.is_empty() {
        return Vec::new();
    }
    if let Some(embedder) = embedder {
        let mut texts = Vec::with_capacity(examples.len() + 1);
        texts.push(arkmemory_capture_review_context_embedding_text(current));
        texts.extend(
            examples
                .iter()
                .map(|example| arkmemory_capture_review_context_embedding_text(&example.context)),
        );
        match tokio::time::timeout(
            std::time::Duration::from_secs(ARKMEMORY_LEARNED_REVIEW_TIMEOUT_SECS),
            embedder.embed_texts(&texts),
        )
        .await
        {
            Ok(Ok(embeddings)) if embeddings.len() == texts.len() => {
                let current_embedding = &embeddings[0];
                let mut scored = examples
                    .iter()
                    .cloned()
                    .enumerate()
                    .map(|(index, example)| {
                        let score = crate::core::document_search::normalized_embedding_similarity(
                            current_embedding.as_slice(),
                            embeddings[index + 1].as_slice(),
                        )
                        .map(f64::from)
                        .unwrap_or(-1.0);
                        (score, example)
                    })
                    .collect::<Vec<_>>();
                scored.sort_by(|left, right| {
                    right
                        .0
                        .partial_cmp(&left.0)
                        .unwrap_or(std::cmp::Ordering::Equal)
                        .then_with(|| right.1.reviewed_at.cmp(&left.1.reviewed_at))
                });
                return scored
                    .into_iter()
                    .take(ARKMEMORY_LEARNED_REVIEW_MAX_EXAMPLES)
                    .map(|(_, example)| example)
                    .collect();
            }
            Ok(Ok(_)) => {
                tracing::debug!(
                    "Memory learned review embedding ranker returned unexpected vector count"
                );
            }
            Ok(Err(error)) => {
                tracing::debug!(
                    "Memory learned review embedding ranker unavailable: {}",
                    error
                );
            }
            Err(_) => {
                tracing::debug!(
                    "Memory learned review embedding ranker timed out after {}s",
                    ARKMEMORY_LEARNED_REVIEW_TIMEOUT_SECS
                );
            }
        }
    }

    let mut ranked = examples.to_vec();
    ranked.sort_by(|left, right| right.reviewed_at.cmp(&left.reviewed_at));
    ranked.truncate(ARKMEMORY_LEARNED_REVIEW_MAX_EXAMPLES);
    ranked
}

fn arkmemory_learned_review_prompt(
    current: &crate::storage::memory_capture_event::Model,
    current_context: &MemoryCaptureReviewContext,
    examples: &[MemoryLearnedReviewExample],
) -> String {
    let examples_payload = examples
        .iter()
        .map(|example| {
            serde_json::json!({
                "event_id": example.event_id.clone(),
                "outcome": example.outcome,
                "reviewed_at": example.reviewed_at.clone(),
                "review_context": arkmemory_capture_review_context_json(&example.context),
            })
        })
        .collect::<Vec<_>>();
    serde_json::to_string_pretty(&serde_json::json!({
        "task": "Decide whether prior human Memory health-review feedback should apply to the current capture finding.",
        "decision_rules": [
            "Compare semantic meaning, intent, subject, polarity, and source context after secret redaction.",
            "Do not rely on exact wording, punctuation, casing, spacing, token text, or one shared word.",
            "If the examples conflict, the current source has a different meaning, or the match is uncertain, return apply=false.",
            "Use a prior outcome only when a human-reviewed example is clearly the same kind of review case."
        ],
        "allowed_outcomes": {
            "expected_sensitive_skip": "The human said this was correctly skipped because it is credential/secret-like input rather than useful memory.",
            "false_positive_safe_memory": "The human said this skip/failure was wrong and the source should have been treated as safe memory material.",
            "acknowledged": "The human dismissed a non-actionable capture failure."
        },
        "current_event": {
            "event_id": current.id.clone(),
            "failure_signature": arkmemory_capture_failure_signature(current),
            "review_context": arkmemory_capture_review_context_json(current_context),
        },
        "human_reviewed_examples": examples_payload,
        "response_schema": {
            "apply": "boolean",
            "outcome": "expected_sensitive_skip | false_positive_safe_memory | acknowledged | null",
            "confidence": "number from 0 to 1",
            "matched_example_id": "event_id string or null",
            "reason": "brief explanation"
        }
    }))
    .unwrap_or_else(|_| "{}".to_string())
}

async fn arkmemory_judge_learned_review(
    llm: &crate::core::LlmClient,
    current: &crate::storage::memory_capture_event::Model,
    current_context: &MemoryCaptureReviewContext,
    examples: &[MemoryLearnedReviewExample],
) -> Option<MemoryLearnedReviewDecision> {
    let system = "You are a strict semantic reviewer for Memory health feedback. \
        You apply previous human review outcomes only when the current case has the same meaning and review intent. \
        Return only the required JSON object.";
    let user = arkmemory_learned_review_prompt(current, current_context, examples);
    match tokio::time::timeout(
        std::time::Duration::from_secs(ARKMEMORY_LEARNED_REVIEW_TIMEOUT_SECS),
        llm.chat_classifier_bounded(system, &user, ARKMEMORY_LEARNED_REVIEW_MAX_OUTPUT_TOKENS),
    )
    .await
    {
        Ok(Ok(response)) => arkmemory_parse_learned_review_decision(&response.content),
        Ok(Err(error)) => {
            tracing::debug!("Memory learned reviewer model call failed: {}", error);
            None
        }
        Err(_) => {
            tracing::debug!(
                "Memory learned reviewer timed out after {}s",
                ARKMEMORY_LEARNED_REVIEW_TIMEOUT_SECS
            );
            None
        }
    }
}

fn arkmemory_learned_review_can_apply(
    decision: &MemoryLearnedReviewDecision,
    examples: &[MemoryLearnedReviewExample],
) -> bool {
    if !decision.apply || decision.confidence < ARKMEMORY_LEARNED_REVIEW_MIN_CONFIDENCE {
        return false;
    }
    let Some(outcome) = decision.outcome.as_deref() else {
        return false;
    };
    examples.iter().any(|example| example.outcome == outcome)
}

pub(crate) async fn run_arkmemory_learned_review_pass(
    storage: &crate::storage::Storage,
    llm: &crate::core::LlmClient,
    embedder: Option<&crate::core::embeddings::EmbeddingClient>,
) -> Result<serde_json::Value> {
    let examples = arkmemory_learned_review_examples(storage).await?;
    if examples.is_empty() {
        return Ok(serde_json::json!({
            "reviewed_examples": 0,
            "failed_examined": 0,
            "semantic_judgments": 0,
            "auto_reviewed": 0,
            "skipped_no_examples": 0,
            "skipped_no_context": 0,
            "skipped_uncertain": 0,
        }));
    }

    let failed_events = storage
        .list_memory_capture_events_by_statuses_all_scopes(
            ARKMEMORY_FAILED_CAPTURE_STATUSES,
            ARKMEMORY_LEARNED_REVIEW_FAILED_LIMIT,
        )
        .await?;
    let mut failed_examined = 0usize;
    let mut semantic_judgments = 0usize;
    let mut auto_reviewed = 0usize;
    let mut skipped_no_examples = 0usize;
    let mut skipped_no_context = 0usize;
    let mut skipped_uncertain = 0usize;

    for mut event in failed_events {
        failed_examined += 1;
        let failure_key = arkmemory_capture_event_failure_signature_key(&event);
        let mut matching_examples = examples
            .iter()
            .filter(|example| example.failure_key == failure_key)
            .cloned()
            .collect::<Vec<_>>();
        if matching_examples.is_empty() {
            matching_examples = examples
                .iter()
                .filter(|example| arkmemory_learned_review_example_compatible(&event, example))
                .cloned()
                .collect::<Vec<_>>();
        }
        if matching_examples.is_empty() {
            skipped_no_examples += 1;
            continue;
        }
        let Some(current_context) = arkmemory_capture_review_context(storage, &event).await else {
            skipped_no_context += 1;
            continue;
        };
        let ranked_examples =
            arkmemory_rank_learned_review_examples(embedder, &current_context, &matching_examples)
                .await;
        if ranked_examples.is_empty() {
            skipped_no_examples += 1;
            continue;
        }
        semantic_judgments += 1;
        let Some(decision) =
            arkmemory_judge_learned_review(llm, &event, &current_context, &ranked_examples).await
        else {
            skipped_uncertain += 1;
            continue;
        };
        if !arkmemory_learned_review_can_apply(&decision, &ranked_examples) {
            skipped_uncertain += 1;
            continue;
        }
        let outcome = decision
            .outcome
            .clone()
            .unwrap_or_else(|| "acknowledged".to_string());
        let source_review_event_id = decision
            .matched_example_id
            .as_deref()
            .and_then(|matched| {
                ranked_examples
                    .iter()
                    .find(|example| example.event_id == matched)
                    .map(|example| example.event_id.clone())
            })
            .or_else(|| {
                ranked_examples
                    .iter()
                    .find(|example| example.outcome == outcome.as_str())
                    .map(|example| example.event_id.clone())
            });
        let previous_status = event.status.trim().to_string();
        let reviewed_status =
            arkmemory_capture_event_reviewed_status(&event, outcome.as_str()).to_string();
        let now = chrono::Utc::now().to_rfc3339();
        let failure_signature = arkmemory_capture_failure_signature(&event);
        let review_context_json = arkmemory_capture_review_context_json(&current_context);
        let previous_metadata = std::mem::take(&mut event.attempt_metadata);
        let mut metadata = match previous_metadata {
            serde_json::Value::Object(map) => map,
            serde_json::Value::Null => serde_json::Map::new(),
            value => {
                let mut map = serde_json::Map::new();
                map.insert("previous_metadata".to_string(), value);
                map
            }
        };
        metadata.insert(
            "reviewed_at".to_string(),
            serde_json::Value::String(now.clone()),
        );
        metadata.insert(
            "reviewed_from".to_string(),
            serde_json::Value::String("arkmemory_learned_review".to_string()),
        );
        metadata.insert(
            "previous_status".to_string(),
            serde_json::Value::String(previous_status.clone()),
        );
        metadata.insert(
            "user_review".to_string(),
            serde_json::json!({
                "outcome": outcome.clone(),
                "outcome_label": arkmemory_capture_review_outcome_label(outcome.as_str()),
                "reviewed_at": now.clone(),
                "failure_signature": failure_signature,
                "review_context": review_context_json,
                "learned": true,
                "reviewed_from": "arkmemory_learned_review",
            }),
        );
        metadata.insert(
            "learned_review".to_string(),
            serde_json::json!({
                "applied_at": now.clone(),
                "outcome": outcome,
                "confidence": decision.confidence,
                "reason": decision.reason,
                "source_review_event_id": source_review_event_id,
                "ranked_example_count": ranked_examples.len(),
            }),
        );
        event.status = reviewed_status;
        event.attempt_metadata = serde_json::Value::Object(metadata);
        if event.completed_at.is_none() {
            event.completed_at = Some(now.clone());
        }
        event.updated_at = now;
        storage.upsert_memory_capture_event(&event).await?;
        auto_reviewed += 1;
    }

    Ok(serde_json::json!({
        "reviewed_examples": examples.len(),
        "failed_examined": failed_examined,
        "semantic_judgments": semantic_judgments,
        "auto_reviewed": auto_reviewed,
        "skipped_no_examples": skipped_no_examples,
        "skipped_no_context": skipped_no_context,
        "skipped_uncertain": skipped_uncertain,
    }))
}

pub(super) async fn arkmemory_build_health_findings(
    storage: &crate::storage::Storage,
    project_id: Option<&str>,
    limit: u64,
) -> Result<Vec<serde_json::Value>> {
    let mut findings = Vec::new();
    let reviewed_capture_patterns = storage
        .list_memory_capture_events_by_statuses(
            ARKMEMORY_REVIEWED_CAPTURE_STATUSES,
            project_id,
            200,
        )
        .await
        .map(|events| arkmemory_capture_review_pattern_summary(&events))
        .unwrap_or_default();
    for event in storage
        .list_memory_capture_events_by_statuses(
            ARKMEMORY_FAILED_CAPTURE_STATUSES,
            project_id,
            limit,
        )
        .await?
    {
        let pattern_key = arkmemory_capture_event_failure_signature_key(&event);
        let review_pattern = reviewed_capture_patterns.get(&pattern_key);
        let source_context = arkmemory_capture_source_context(storage, &event).await;
        findings.push(arkmemory_capture_event_finding(
            event,
            review_pattern,
            source_context,
        ));
        if findings.len() >= limit as usize {
            return Ok(findings);
        }
    }

    if findings.len() < limit as usize {
        for event in storage
            .list_memory_capture_events_by_statuses(
                ARKMEMORY_REVIEWED_CAPTURE_STATUSES,
                project_id,
                limit,
            )
            .await?
        {
            if !arkmemory_capture_event_has_learned_review(&event) {
                continue;
            }
            let source_context = arkmemory_capture_source_context(storage, &event).await;
            findings.push(arkmemory_capture_event_learned_review_finding(
                event,
                source_context,
            ));
            if findings.len() >= limit as usize {
                return Ok(findings);
            }
        }
    }

    let memory_items = storage
        .list_active_experience_items(&["personal_fact", "constraint"], project_id, None, limit)
        .await?;
    for item in memory_items {
        if item.embedding.is_none() {
            findings.push(serde_json::json!({
                "id": format!("embedding:{}", item.id),
                "kind": "missing_embedding",
                "severity": "warning",
                "memory_id": item.id,
                "title": item.title,
                "detail": "This memory has no semantic vector yet, so retrieval and dedup quality can be lower until it is refreshed.",
                "action": "refresh_on_next_write",
                "created_at": item.updated_at,
            }));
        }
        if item.confidence < 0.55 {
            findings.push(serde_json::json!({
                "id": format!("confidence:{}", item.id),
                "kind": "low_confidence",
                "severity": "review",
                "memory_id": item.id,
                "title": item.title,
                "detail": "This memory is below the normal confidence floor and should be reviewed before it shapes future answers.",
                "action": "review_memory",
                "created_at": item.updated_at,
            }));
        }
        let evidence_links = storage
            .list_memory_evidence_links_for_memory(&item.id, project_id, 16)
            .await
            .unwrap_or_default();
        let operation_evidence_refs = storage
            .list_memory_operations_for_memory(&item.id, project_id, 16)
            .await
            .unwrap_or_default()
            .into_iter()
            .flat_map(|operation| memory_operation_evidence_source_refs(&operation))
            .collect::<Vec<_>>();
        if arkmemory_memory_sources(&item).is_empty()
            && evidence_links.is_empty()
            && operation_evidence_refs.is_empty()
        {
            findings.push(serde_json::json!({
                "id": format!("source:{}", item.id),
                "kind": "missing_source",
                "severity": "review",
                "memory_id": item.id,
                "title": item.title,
                "detail": "This memory has no structured source references attached.",
                "action": "review_provenance",
                "created_at": item.updated_at,
            }));
        }
        if findings.len() >= limit as usize {
            break;
        }
    }
    if findings.len() < limit as usize {
        for operation in storage
            .list_memory_operations_by_statuses(&["apply_failed"], project_id, limit)
            .await?
        {
            let operation_id = operation.id.clone();
            let operation_capture_event_id = operation.capture_event_id.clone();
            let operation_status = operation.status.clone();
            let operation_type = operation.operation_type.clone();
            let operation_key = operation.key.clone();
            let operation_value = if operation.looks_sensitive {
                None
            } else {
                operation.value.clone()
            };
            let operation_title = operation_key
                .as_deref()
                .map(str::trim)
                .filter(|value| !value.is_empty())
                .map(|key| format!("Memory operation {}: {}", operation_type, key))
                .unwrap_or_else(|| format!("Memory operation {}", operation_type));
            let memory_id = operation
                .applied_memory_id
                .clone()
                .or_else(|| operation.target_memory_id.clone());
            let source_context = arkmemory_memory_operation_source_context(&operation);
            findings.push(serde_json::json!({
                "id": format!("operation:{}", operation_id),
                "kind": operation_status.clone(),
                "severity": if operation_status == "apply_failed" { "warning" } else { "review" },
                "memory_id": memory_id,
                "operation_id": operation_id.clone(),
                "capture_event_id": operation_capture_event_id.clone(),
                "operation_type": operation_type.clone(),
                "status": operation_status.clone(),
                "title": operation_title,
                "detail": operation
                    .review_notes
                    .clone()
                    .unwrap_or_else(|| "This staged memory operation still needs Memory review.".to_string()),
                "source_context": source_context,
                "operation": {
                    "id": operation_id,
                    "capture_event_id": operation_capture_event_id,
                    "operation_type": operation_type,
                    "status": operation_status,
                    "target_memory_id": operation.target_memory_id,
                    "applied_memory_id": operation.applied_memory_id,
                    "key": operation_key,
                    "value": operation_value,
                    "memory_kind": operation.memory_kind,
                    "durability": operation.durability,
                    "scope": operation.scope,
                    "project_id": operation.project_id,
                    "conversation_id": operation.conversation_id,
                    "confidence": operation.confidence,
                    "looks_sensitive": operation.looks_sensitive,
                    "sensitive_reason": operation.sensitive_reason,
                    "valid_from": operation.valid_from,
                    "expires_at": operation.expires_at,
                    "review_at": operation.review_at,
                    "rationale": operation.rationale,
                    "evidence_refs": operation.evidence_refs,
                    "model_metadata": operation.model_metadata,
                    "apply_metadata": operation.apply_metadata,
                    "applied_at": operation.applied_at,
                    "reviewed_at": operation.reviewed_at,
                    "review_notes": operation.review_notes,
                    "created_at": operation.created_at,
                    "updated_at": operation.updated_at,
                },
                "action": "review_memory_operation",
                "created_at": operation.updated_at,
            }));
            if findings.len() >= limit as usize {
                break;
            }
        }
    }
    Ok(findings)
}

pub(super) fn arkmemory_event_model_with_id(
    id: String,
    event_type: &str,
    memory_id: Option<String>,
    related_memory_id: Option<String>,
    summary: impl Into<String>,
    metadata: serde_json::Value,
    context: MemoryEventContext,
) -> crate::storage::recall_event::Model {
    let now = chrono::Utc::now().to_rfc3339();
    crate::storage::recall_event::Model {
        id,
        event_type: event_type.to_string(),
        memory_id,
        related_memory_id,
        scope: context.scope,
        project_id: context.project_id,
        conversation_id: context.conversation_id,
        source_kind: Some("arkmemory".to_string()),
        source_ref: context.source_ref,
        actor: "arkmemory".to_string(),
        summary: Some(summary.into()),
        old_snapshot: serde_json::Value::Null,
        new_snapshot: serde_json::Value::Null,
        metadata,
        risk_level: None,
        confidence: None,
        reversible: false,
        reverted_at: None,
        created_at: now.clone(),
        updated_at: now,
    }
}

pub(super) fn arkmemory_event_model(
    event_type: &str,
    memory_id: Option<String>,
    related_memory_id: Option<String>,
    summary: impl Into<String>,
    metadata: serde_json::Value,
    context: MemoryEventContext,
) -> crate::storage::recall_event::Model {
    arkmemory_event_model_with_id(
        uuid::Uuid::new_v4().to_string(),
        event_type,
        memory_id,
        related_memory_id,
        summary,
        metadata,
        context,
    )
}

pub(super) async fn arkmemory_record_event(
    storage: &crate::storage::Storage,
    event_type: &str,
    memory_id: Option<String>,
    related_memory_id: Option<String>,
    summary: impl Into<String>,
    metadata: serde_json::Value,
    context: MemoryEventContext,
) -> Result<()> {
    let event = arkmemory_event_model(
        event_type,
        memory_id,
        related_memory_id,
        summary,
        metadata,
        context,
    );
    storage.insert_recall_event(&event).await
}

pub(super) async fn arkmemory_record_event_once(
    storage: &crate::storage::Storage,
    event_id: String,
    event_type: &str,
    memory_id: Option<String>,
    related_memory_id: Option<String>,
    summary: impl Into<String>,
    metadata: serde_json::Value,
    context: MemoryEventContext,
) -> Result<()> {
    let event = arkmemory_event_model_with_id(
        event_id,
        event_type,
        memory_id,
        related_memory_id,
        summary,
        metadata,
        context,
    );
    storage.insert_recall_event(&event).await
}

pub(super) async fn arkmemory_apply_memory_candidate(
    agent: &crate::core::Agent,
    candidate_id: &str,
    project_id: Option<&str>,
) -> Result<String> {
    let storage = &agent.storage;
    let mut candidate = storage
        .get_learning_candidate(candidate_id)
        .await?
        .ok_or_else(|| anyhow::anyhow!("Memory queue item not found."))?;
    if !arkmemory_candidate_is_memory(&candidate.candidate_type) {
        anyhow::bail!("Memory queue item is not a memory operation.");
    }
    let _ = project_id;
    candidate = arkmemory_ensure_latest_open_candidate(storage, &candidate).await?;
    if candidate.approval_status == "applying" {
        anyhow::bail!("Memory queue item is already being applied.");
    }
    if candidate.approval_status != "draft" {
        anyhow::bail!("Memory queue item is no longer pending review.");
    }
    let replay_gate =
        crate::core::self_evolve::replay_gate::evaluate_candidate_replay_gate(storage, &candidate)
            .await?;
    if !replay_gate.allow_approval {
        anyhow::bail!(
            "Replay gate blocked approval for '{}': {}",
            candidate.title,
            replay_gate.reason
        );
    }
    let claimed = storage
        .update_learning_candidate_review_if_status(
            candidate_id,
            "draft",
            "applying",
            Some("Applying from Memory."),
            None,
        )
        .await?;
    if !claimed {
        anyhow::bail!("Memory queue item was already claimed by another review.");
    }
    let result = arkmemory_apply_claimed_memory_candidate(agent, &candidate).await;
    match result {
        Ok(approved_ref) => {
            let finalized = storage
                .update_learning_candidate_review_if_status(
                    candidate_id,
                    "applying",
                    "approved",
                    Some("Approved from Memory."),
                    Some(&approved_ref),
                )
                .await?;
            if !finalized {
                anyhow::bail!("Memory queue item changed while it was being applied.");
            }
            Ok(approved_ref)
        }
        Err(error) => {
            let note = format!("Apply failed: {error:#}");
            let _ = storage
                .update_learning_candidate_review_if_status(
                    candidate_id,
                    "applying",
                    "draft",
                    Some(&note),
                    None,
                )
                .await;
            Err(error)
        }
    }
}

pub(super) async fn arkmemory_apply_claimed_memory_candidate(
    agent: &crate::core::Agent,
    candidate: &crate::storage::learning_candidate::Model,
) -> Result<String> {
    let storage = &agent.storage;
    let approved_ref = match candidate.candidate_type.as_str() {
        "memory_add" | "memory_update" | "memory_retract" => {
            let operation_id = candidate
                .proposed_content
                .get("operation_id")
                .and_then(|value| value.as_str())
                .map(str::trim)
                .filter(|value| !value.is_empty())
                .ok_or_else(|| anyhow::anyhow!("Memory queue item is missing operation_id."))?;
            let approved_ref = agent
                .apply_memory_operation_by_id_with_source(operation_id, "arkmemory_review")
                .await?;
            let memory_id =
                storage
                    .get_memory_operation(operation_id)
                    .await?
                    .and_then(|operation| {
                        operation
                            .applied_memory_id
                            .clone()
                            .or(operation.target_memory_id.clone())
                    });
            arkmemory_record_event(
                storage,
                "queue_memory_operation_applied",
                memory_id,
                None,
                format!("Approved memory operation {}", operation_id),
                serde_json::json!({
                    "candidate_id": candidate.id.clone(),
                    "operation_id": operation_id,
                    "operation_type": candidate.candidate_type.clone(),
                }),
                MemoryEventContext::from_candidate(candidate),
            )
            .await?;
            approved_ref
        }
        "memory_deprecate" => {
            let item_id = candidate
                .proposed_content
                .get("item_id")
                .and_then(|value| value.as_str())
                .map(str::trim)
                .filter(|value| !value.is_empty())
                .ok_or_else(|| anyhow::anyhow!("Memory deprecation item is missing item_id."))?;
            let item = storage
                .get_experience_item(item_id)
                .await?
                .ok_or_else(|| anyhow::anyhow!("Memory item not found."))?;
            if !arkmemory_item_is_memory(&item) {
                anyhow::bail!("Memory queue item points at a non-memory experience item.");
            }
            let next_status = candidate
                .proposed_content
                .get("next_status")
                .and_then(|value| value.as_str())
                .map(str::trim)
                .filter(|value| *value == "deprecated")
                .unwrap_or("deprecated");
            storage
                .update_experience_item_status(item_id, next_status)
                .await?;
            arkmemory_record_event(
                storage,
                "queue_memory_deprecated",
                Some(item_id.to_string()),
                None,
                format!("Approved memory deprecation for {}", item_id),
                serde_json::json!({ "candidate_id": candidate.id.clone(), "next_status": next_status }),
                MemoryEventContext::from_memory(&item),
            )
            .await?;
            item_id.to_string()
        }
        "memory_merge" => {
            let target_item_id = candidate
                .proposed_content
                .get("target_item_id")
                .and_then(|value| value.as_str())
                .map(str::trim)
                .filter(|value| !value.is_empty())
                .ok_or_else(|| anyhow::anyhow!("Memory merge item is missing target_item_id."))?;
            let source_item_id = candidate
                .proposed_content
                .get("source_item_id")
                .and_then(|value| value.as_str())
                .map(str::trim)
                .filter(|value| !value.is_empty())
                .ok_or_else(|| anyhow::anyhow!("Memory merge item is missing source_item_id."))?;
            if target_item_id == source_item_id {
                anyhow::bail!("Memory merge source and target must be different items.");
            }
            let target_item = storage
                .get_experience_item(target_item_id)
                .await?
                .ok_or_else(|| anyhow::anyhow!("Memory merge target item not found."))?;
            let source_item = storage
                .get_experience_item(source_item_id)
                .await?
                .ok_or_else(|| anyhow::anyhow!("Memory merge source item not found."))?;
            if !arkmemory_item_is_memory(&target_item) || !arkmemory_item_is_memory(&source_item) {
                anyhow::bail!("Memory merge can only apply to memory experience items.");
            }
            storage
                .update_experience_item_status(source_item_id, "deprecated")
                .await?;
            let now = chrono::Utc::now().to_rfc3339();
            storage
                .upsert_experience_edge(&crate::storage::experience_edge::Model {
                    id: format!("arkmemory-edge-{}", candidate.id),
                    source_ref: target_item_id.to_string(),
                    source_kind: "experience_item".to_string(),
                    target_ref: source_item_id.to_string(),
                    target_kind: "experience_item".to_string(),
                    edge_type: "supersedes".to_string(),
                    weight: 1.0,
                    source_run_id: None,
                    metadata: serde_json::json!({ "approved_via": "arkmemory", "candidate_id": candidate.id.clone() }),
                    created_at: now.clone(),
                    updated_at: now,
                })
                .await?;
            arkmemory_record_event(
                storage,
                "queue_memory_merged",
                Some(target_item_id.to_string()),
                Some(source_item_id.to_string()),
                format!("Approved memory merge into {}", target_item_id),
                serde_json::json!({ "candidate_id": candidate.id.clone() }),
                MemoryEventContext::from_memory(&target_item),
            )
            .await?;
            target_item_id.to_string()
        }
        _ => unreachable!(),
    };
    Ok(approved_ref)
}

pub(super) async fn arkmemory_summary(
    State(state): State<AppState>,
    axum::extract::Query(params): axum::extract::Query<HashMap<String, String>>,
) -> Response {
    let project_id = arkmemory_project_param(&params);
    let agent = state.agent.read().await;
    let storage = &agent.storage;
    let facts = storage
        .count_facts_by_category(
            project_id,
            crate::core::memory_schema::MEMORY_CATEGORY_PROFILE_FACT,
        )
        .await
        .unwrap_or(0);
    let assistant_preferences = storage
        .count_facts_by_category(
            project_id,
            crate::core::memory_schema::MEMORY_CATEGORY_ASSISTANT_PREFERENCE,
        )
        .await
        .unwrap_or(0);
    let work_preferences = storage
        .count_facts_by_category(
            project_id,
            crate::core::memory_schema::MEMORY_CATEGORY_WORK_PREFERENCE,
        )
        .await
        .unwrap_or(0);
    let project_domain_memory = storage
        .count_facts_by_category(
            project_id,
            crate::core::memory_schema::MEMORY_CATEGORY_PROJECT_DOMAIN,
        )
        .await
        .unwrap_or(0);
    let ephemeral_context = storage
        .count_facts_by_category(
            project_id,
            crate::core::memory_schema::MEMORY_CATEGORY_EPHEMERAL_CONTEXT,
        )
        .await
        .unwrap_or(0);
    let other_memory = storage
        .count_facts_by_category(
            project_id,
            crate::core::memory_schema::MEMORY_CATEGORY_OTHER,
        )
        .await
        .unwrap_or(0);
    let preferences = storage
        .count_user_preferences(project_id)
        .await
        .unwrap_or(0);
    let user_data = storage
        .count_user_data_items(project_id, None)
        .await
        .unwrap_or(0);
    let knowledge = storage
        .count_visible_knowledge_items(project_id)
        .await
        .unwrap_or(0);
    let queue = arkmemory_list_memory_candidates(storage, project_id, 200)
        .await
        .map(|items| items.len())
        .unwrap_or(0);
    let pending_capture_signals =
        arkmemory_pending_capture_signal_payloads(storage, project_id, 200)
            .await
            .unwrap_or_default();
    let pending_capture = pending_capture_signals.len();
    let health_findings = arkmemory_build_health_findings(storage, project_id, 200)
        .await
        .unwrap_or_default();
    let failed_capture = health_findings
        .iter()
        .filter(|finding| {
            finding.get("kind").and_then(|value| value.as_str()) == Some("capture_failed")
        })
        .count();
    let ledger = storage.count_recall_events(project_id).await.unwrap_or(0);
    let tests = storage.count_recall_tests(project_id).await.unwrap_or(0);
    let health = health_findings.len();
    (
        StatusCode::OK,
        Json(serde_json::json!({
            "current_memory": {
                "facts": facts,
                "profile_facts": facts,
                "assistant_preferences": assistant_preferences,
                "work_preferences": work_preferences,
                "project_domain_memory": project_domain_memory,
                "ephemeral_context": ephemeral_context,
                "other_memory": other_memory,
                "preferences": preferences,
                "user_data": user_data,
                "knowledge": knowledge,
            },
            "queue": queue,
            "capture_pipeline": {
                "pending": pending_capture,
                "failed": failed_capture,
                "pending_events": pending_capture_signals,
            },
            "ledger": ledger,
            "health": health,
            "tests": tests,
        })),
    )
        .into_response()
}

fn arkmemory_graph_limit(params: &HashMap<String, String>) -> u64 {
    params
        .get("limit")
        .and_then(|value| value.parse::<u64>().ok())
        .unwrap_or(120)
        .clamp(1, 220)
}

fn arkmemory_graph_bool_param(
    params: &HashMap<String, String>,
    key: &str,
    default_value: bool,
) -> bool {
    params
        .get(key)
        .map(|value| value.trim().eq_ignore_ascii_case("true") || value.trim() == "1")
        .unwrap_or(default_value)
}

fn arkmemory_graph_semantic_threshold(params: &HashMap<String, String>) -> f64 {
    params
        .get("semantic_threshold")
        .and_then(|value| value.parse::<f64>().ok())
        .unwrap_or(0.78)
        .clamp(0.50, 0.98)
}

fn arkmemory_graph_per_node_semantic_limit(params: &HashMap<String, String>) -> usize {
    params
        .get("semantic_per_node")
        .and_then(|value| value.parse::<usize>().ok())
        .unwrap_or(3)
        .clamp(1, 8)
}

fn arkmemory_graph_min_confidence(params: &HashMap<String, String>) -> f64 {
    params
        .get("min_confidence")
        .and_then(|value| value.parse::<f64>().ok())
        .unwrap_or(0.0)
        .clamp(0.0, 1.0)
}

fn arkmemory_graph_updated_after(params: &HashMap<String, String>) -> Option<String> {
    params
        .get("updated_after")
        .map(|value| value.trim())
        .filter(|value| !value.is_empty())
        .map(str::to_string)
}

fn arkmemory_graph_updated_before(params: &HashMap<String, String>) -> Option<String> {
    params
        .get("updated_before")
        .map(|value| value.trim())
        .filter(|value| !value.is_empty())
        .map(str::to_string)
}

fn arkmemory_graph_updated_in_range(
    updated_at: &str,
    updated_after: Option<&str>,
    updated_before: Option<&str>,
) -> bool {
    updated_after
        .map(|after| updated_at >= after)
        .unwrap_or(true)
        && updated_before
            .map(|before| updated_at <= before)
            .unwrap_or(true)
}

fn arkmemory_graph_filter_item(
    item: &crate::storage::experience_item::Model,
    categories: &HashSet<String>,
    statuses: &HashSet<String>,
    min_confidence: f64,
    updated_after: Option<&str>,
    updated_before: Option<&str>,
) -> bool {
    (categories.is_empty() || categories.contains(&arkmemory_graph_memory_category(item)))
        && (statuses.is_empty() || statuses.contains(&item.status))
        && item.confidence >= min_confidence
        && arkmemory_graph_updated_in_range(&item.updated_at, updated_after, updated_before)
}

fn arkmemory_graph_filter_edge(edge_type: &str, edge_types: &HashSet<String>) -> bool {
    edge_types.is_empty() || edge_types.contains(edge_type)
}

fn arkmemory_graph_ref_kind(source_ref: &str) -> String {
    source_ref
        .split_once(':')
        .map(|(kind, _)| kind.trim())
        .filter(|value| !value.is_empty())
        .unwrap_or("source")
        .to_string()
}

fn arkmemory_graph_ref_label(source_ref: &str) -> String {
    let trimmed = source_ref.trim();
    let label = trimmed
        .split_once(':')
        .map(|(_, value)| value.trim())
        .filter(|value| !value.is_empty())
        .unwrap_or(trimmed);
    if label.chars().count() > 34 {
        let prefix = label.chars().take(31).collect::<String>();
        format!("{prefix}...")
    } else {
        label.to_string()
    }
}

fn arkmemory_graph_add_ref_node(
    nodes: &mut Vec<ArkmemoryGraphNode>,
    edges: &mut Vec<ArkmemoryGraphEdge>,
    seen_nodes: &mut HashSet<String>,
    anchor_id: &str,
    source_ref: &str,
    edge_type: &str,
) {
    let source_ref = source_ref.trim();
    if source_ref.is_empty() {
        return;
    }
    let node_id = arkmemory_stable_event_id(&["graph_source", source_ref]);
    if seen_nodes.insert(node_id.clone()) {
        nodes.push(arkmemory_graph_ref_node(
            node_id.clone(),
            &arkmemory_graph_ref_kind(source_ref),
            arkmemory_graph_ref_label(source_ref),
        ));
    }
    edges.push(arkmemory_graph_ref_edge(
        anchor_id,
        &node_id,
        edge_type,
        source_ref.to_string(),
    ));
}

fn arkmemory_graph_relation_statuses(params: &HashMap<String, String>) -> Vec<String> {
    let requested = arkmemory_graph_params_filter(params, "relation_status");
    if requested.is_empty() {
        vec!["candidate".to_string(), "confirmed".to_string()]
    } else {
        requested
    }
}

fn arkmemory_graph_entity_statuses(params: &HashMap<String, String>) -> Vec<String> {
    let requested = arkmemory_graph_params_filter(params, "entity_status");
    if requested.is_empty() {
        vec!["active".to_string()]
    } else {
        requested
    }
}

fn arkmemory_graph_knowledge_entity_node(
    entity: &crate::storage::knowledge_entity::Model,
    pinned: bool,
) -> ArkmemoryGraphNode {
    ArkmemoryGraphNode {
        id: entity.id.clone(),
        node_type: "entity".to_string(),
        label: entity.canonical_name.clone(),
        detail: entity.entity_type.clone(),
        category: Some(entity.entity_type.clone()),
        status: Some(entity.status.clone()),
        memory_kind: None,
        confidence: Some(entity.confidence),
        support_count: None,
        stale: entity.status != "active",
        pinned,
        updated_at: Some(entity.updated_at.clone()),
        ref_kind: None,
        metadata: Some(serde_json::json!({
            "entity_type": entity.entity_type,
            "normalized_name": entity.normalized_name,
            "aliases": entity.aliases,
            "metadata": entity.metadata,
        })),
    }
}

fn arkmemory_graph_add_knowledge_entity_node(
    nodes: &mut Vec<ArkmemoryGraphNode>,
    seen_nodes: &mut HashSet<String>,
    entity: &crate::storage::knowledge_entity::Model,
    pinned: bool,
) {
    if seen_nodes.insert(entity.id.clone()) {
        nodes.push(arkmemory_graph_knowledge_entity_node(entity, pinned));
    }
}

fn arkmemory_graph_relation_evidence_payload(
    evidence: &[crate::storage::knowledge_relation_evidence::Model],
) -> Vec<serde_json::Value> {
    evidence
        .iter()
        .take(12)
        .map(|item| {
            serde_json::json!({
                "id": item.id,
                "evidence_kind": item.evidence_kind,
                "evidence_ref": item.evidence_ref,
                "memory_id": item.memory_id,
                "message_id": item.message_id,
                "document_id": item.document_id,
                "project_id": item.project_id,
                "conversation_id": item.conversation_id,
                "polarity": item.polarity,
                "confidence": item.confidence,
                "excerpt": item.excerpt,
                "created_at": item.created_at,
            })
        })
        .collect()
}

fn arkmemory_graph_knowledge_relation_edge(
    relation: &crate::storage::knowledge_relation::Model,
    evidence: &[crate::storage::knowledge_relation_evidence::Model],
) -> ArkmemoryGraphEdge {
    let relation_type_label = arkmemory_graph_edge_label(&relation.relation_type);
    ArkmemoryGraphEdge {
        id: relation.id.clone(),
        source: relation.source_entity_id.clone(),
        target: relation.target_entity_id.clone(),
        edge_type: "knowledge_relation".to_string(),
        label: relation_type_label.clone(),
        detail: format!(
            "{} relation, status {}, {} supporting evidence, {} contradicting evidence",
            relation_type_label,
            relation.status,
            relation.support_count,
            relation.contradiction_count
        ),
        weight: relation.confidence.clamp(0.05, 1.0),
        semantic: false,
        explicit: true,
        updated_at: Some(relation.updated_at.clone()),
        metadata: Some(serde_json::json!({
            "relation_id": relation.id,
            "relation_type": relation.relation_type,
            "status": relation.status,
            "confidence": relation.confidence,
            "support_count": relation.support_count,
            "contradiction_count": relation.contradiction_count,
            "evidence_count": evidence.len(),
            "evidence": arkmemory_graph_relation_evidence_payload(evidence),
        })),
    }
}

fn arkmemory_graph_relation_evidence_edge(
    anchor_id: &str,
    entity_id: &str,
    relation_id: &str,
) -> ArkmemoryGraphEdge {
    ArkmemoryGraphEdge {
        id: arkmemory_stable_event_id(&["relation_evidence", anchor_id, entity_id, relation_id]),
        source: anchor_id.to_string(),
        target: entity_id.to_string(),
        edge_type: "relation_evidence".to_string(),
        label: "Relation Evidence".to_string(),
        detail: "This source supports or contradicts a stored relation candidate.".to_string(),
        weight: 0.55,
        semantic: false,
        explicit: true,
        updated_at: None,
        metadata: Some(serde_json::json!({ "relation_id": relation_id })),
    }
}

fn arkmemory_graph_relation_evidence_ref(
    evidence: &crate::storage::knowledge_relation_evidence::Model,
) -> Option<String> {
    let kind = evidence.evidence_kind.trim();
    let reference = evidence.evidence_ref.trim();
    if kind.is_empty() || reference.is_empty() {
        return None;
    }
    Some(format!("{kind}:{reference}"))
}

fn arkmemory_graph_evidence_matches_sources(
    evidence: &crate::storage::knowledge_relation_evidence::Model,
    source_filters: &HashSet<String>,
) -> bool {
    source_filters.is_empty()
        || source_filters.contains(evidence.evidence_kind.as_str())
        || source_filters.contains(evidence.evidence_ref.as_str())
}

fn arkmemory_graph_relation_matches_filters(
    relation: &crate::storage::knowledge_relation::Model,
    min_confidence: f64,
    updated_after: Option<&str>,
    updated_before: Option<&str>,
) -> bool {
    relation.confidence >= min_confidence
        && arkmemory_graph_updated_in_range(&relation.updated_at, updated_after, updated_before)
}

const ARKMEMORY_KNOWLEDGE_EXTRACT_MEMORY_LIMIT: u64 = 80;
const ARKMEMORY_KNOWLEDGE_EXTRACT_DOCUMENT_LIMIT: u64 = 12;
const ARKMEMORY_DOCUMENT_EXTRACT_CHUNK_BATCH_LIMIT: u64 = 6;
const ARKMEMORY_KNOWLEDGE_RELATION_MIN_CONFIDENCE: f64 = 0.72;
const ARKMEMORY_DOCUMENT_MEMORY_MIN_CONFIDENCE: f64 = 0.86;
const ARKMEMORY_DOCUMENT_MEMORY_CAPTURE_KIND: &str = "document_memory_extract";
const ARKMEMORY_DOCUMENT_MEMORY_PENDING_STATUS: &str = "pending_document_memory_extract";
const ARKMEMORY_DOCUMENT_MEMORY_PROCESSING_STATUS: &str = "processing_document_memory_extract";
const ARKMEMORY_DOCUMENT_MEMORY_COMPLETED_STATUS: &str = "completed_document_memory_extract";
const ARKMEMORY_DOCUMENT_MEMORY_FAILED_STATUS: &str = "failed_document_memory_extract";
const ARKMEMORY_DOCUMENT_MEMORY_IDLE_INTERVAL: Duration = Duration::from_secs(90);
static ARKMEMORY_DOCUMENT_MEMORY_IDLE_LOOP_STARTED: AtomicBool = AtomicBool::new(false);
static ARKMEMORY_DOCUMENT_MEMORY_EXTRACT_ACTIVE: AtomicBool = AtomicBool::new(false);

#[derive(Debug, Clone)]
struct ArkmemoryKnowledgeExtractionSource {
    source_kind: String,
    source_id: String,
    title: String,
    text: String,
    project_id: Option<String>,
    conversation_id: Option<String>,
    memory_id: Option<String>,
    document_id: Option<String>,
    document_chunk_id: Option<String>,
}

#[derive(Debug, Clone)]
struct ArkmemoryExtractedRelation {
    source_entity_name: String,
    source_entity_type: String,
    target_entity_name: String,
    target_entity_type: String,
    relation_type: String,
    confidence: f64,
    polarity: String,
    evidence: String,
    reason: String,
}

#[derive(Debug, Clone)]
struct ArkmemoryExtractedMemoryCandidate {
    key: String,
    value: String,
    category: String,
    kind: String,
    durability: String,
    scope: String,
    sensitivity: String,
    topics: Vec<String>,
    confidence: f64,
    evidence: String,
    reason: String,
    looks_sensitive: bool,
    sensitive_reason: Option<String>,
}

#[derive(Debug, Clone, Default)]
struct ArkmemoryKnowledgeExtractionResult {
    relations: Vec<ArkmemoryExtractedRelation>,
    memory_candidates: Vec<ArkmemoryExtractedMemoryCandidate>,
}

#[derive(Debug, Clone, Default, Serialize)]
struct ArkmemoryKnowledgeExtractionStats {
    sources_seen: usize,
    sources_processed: usize,
    relations_written: usize,
    memory_candidates_queued: usize,
    skipped_low_confidence: usize,
    failed_sources: usize,
}

struct ArkmemoryDocumentExtractGuard;

impl Drop for ArkmemoryDocumentExtractGuard {
    fn drop(&mut self) {
        ARKMEMORY_DOCUMENT_MEMORY_EXTRACT_ACTIVE.store(false, Ordering::Release);
    }
}

fn arkmemory_document_extract_try_guard() -> Option<ArkmemoryDocumentExtractGuard> {
    ARKMEMORY_DOCUMENT_MEMORY_EXTRACT_ACTIVE
        .compare_exchange(false, true, Ordering::AcqRel, Ordering::Acquire)
        .ok()
        .map(|_| ArkmemoryDocumentExtractGuard)
}

fn arkmemory_kg_json_string(value: &serde_json::Value, key: &str) -> String {
    value
        .get(key)
        .and_then(|value| value.as_str())
        .map(str::trim)
        .filter(|value| !value.is_empty())
        .unwrap_or_default()
        .to_string()
}

fn arkmemory_kg_json_bool(value: &serde_json::Value, key: &str) -> bool {
    value
        .get(key)
        .and_then(|value| value.as_bool())
        .unwrap_or(false)
}

fn arkmemory_kg_json_confidence(value: &serde_json::Value) -> f64 {
    value
        .get("confidence")
        .and_then(|value| value.as_f64())
        .unwrap_or(0.0)
        .clamp(0.0, 1.0)
}

fn arkmemory_kg_string_tokens(value: Option<&serde_json::Value>, limit: usize) -> Vec<String> {
    let mut seen = HashSet::new();
    let mut out = Vec::new();
    if let Some(values) = value.and_then(|value| value.as_array()) {
        for item in values {
            let token = item
                .as_str()
                .map(str::trim)
                .filter(|token| !token.is_empty());
            let Some(token) = token else {
                continue;
            };
            let token = arkmemory_truncate_chars(token, 80);
            if seen.insert(token.clone()) {
                out.push(token);
            }
            if out.len() >= limit {
                break;
            }
        }
    }
    out
}

fn arkmemory_kg_normalize_token(raw: &str, fallback: &str) -> String {
    let mut out = String::new();
    let mut last_separator = false;
    for ch in raw.trim().chars().flat_map(|ch| ch.to_lowercase()) {
        if ch.is_ascii_alphanumeric() {
            out.push(ch);
            last_separator = false;
        } else if !last_separator && !out.is_empty() {
            out.push('_');
            last_separator = true;
        }
    }
    let normalized = out.trim_matches('_').to_string();
    if normalized.is_empty() {
        fallback.to_string()
    } else {
        normalized
    }
}

fn arkmemory_kg_normalize_name(raw: &str) -> String {
    let mut out = String::new();
    let mut last_space = false;
    for ch in raw.trim().chars().flat_map(|ch| ch.to_lowercase()) {
        if ch.is_alphanumeric() {
            out.push(ch);
            last_space = false;
        } else if !last_space && !out.is_empty() {
            out.push(' ');
            last_space = true;
        }
    }
    out.trim().to_string()
}

fn arkmemory_kg_stable_id(prefix: &str, parts: &[&str]) -> String {
    let mut owned_parts = vec![prefix];
    owned_parts.extend_from_slice(parts);
    arkmemory_stable_event_id(&owned_parts).replace("arkmemory-event-", &format!("{prefix}-"))
}

fn arkmemory_kg_entity_id(
    project_id: Option<&str>,
    entity_type: &str,
    normalized_name: &str,
) -> String {
    arkmemory_kg_stable_id(
        "kg-entity",
        &[project_id.unwrap_or_default(), entity_type, normalized_name],
    )
}

fn arkmemory_kg_relation_id(
    project_id: Option<&str>,
    source_entity_id: &str,
    relation_type: &str,
    target_entity_id: &str,
) -> String {
    arkmemory_kg_stable_id(
        "kg-relation",
        &[
            project_id.unwrap_or_default(),
            source_entity_id,
            relation_type,
            target_entity_id,
        ],
    )
}

fn arkmemory_kg_evidence_id(
    relation_id: &str,
    source_kind: &str,
    source_id: &str,
    polarity: &str,
    evidence: &str,
) -> String {
    arkmemory_kg_stable_id(
        "kg-evidence",
        &[relation_id, source_kind, source_id, polarity, evidence],
    )
}

fn arkmemory_document_memory_sensitivity_safe(sensitivity: &str) -> bool {
    let normalized = arkmemory_kg_normalize_token(sensitivity, "prompt_safe");
    normalized.is_empty() || normalized == "prompt_safe"
}

fn arkmemory_parse_knowledge_extraction(text: &str) -> ArkmemoryKnowledgeExtractionResult {
    let Some(json) = arkmemory_extract_json_object(text) else {
        return ArkmemoryKnowledgeExtractionResult::default();
    };
    let mut result = ArkmemoryKnowledgeExtractionResult::default();
    if let Some(relations) = json.get("relations").and_then(|value| value.as_array()) {
        for item in relations.iter().take(8) {
            let source_entity_name = arkmemory_kg_json_string(item, "source_entity_name");
            let target_entity_name = arkmemory_kg_json_string(item, "target_entity_name");
            let relation_type = arkmemory_kg_json_string(item, "relation_type");
            let evidence = arkmemory_kg_json_string(item, "evidence");
            if source_entity_name.is_empty()
                || target_entity_name.is_empty()
                || relation_type.is_empty()
                || evidence.is_empty()
            {
                continue;
            }
            result.relations.push(ArkmemoryExtractedRelation {
                source_entity_name: arkmemory_truncate_chars(&source_entity_name, 160),
                source_entity_type: arkmemory_kg_json_string(item, "source_entity_type"),
                target_entity_name: arkmemory_truncate_chars(&target_entity_name, 160),
                target_entity_type: arkmemory_kg_json_string(item, "target_entity_type"),
                relation_type,
                confidence: arkmemory_kg_json_confidence(item),
                polarity: arkmemory_kg_json_string(item, "polarity"),
                evidence: arkmemory_truncate_chars(&evidence, 500),
                reason: arkmemory_truncate_chars(&arkmemory_kg_json_string(item, "reason"), 300),
            });
        }
    }
    if let Some(candidates) = json
        .get("memory_candidates")
        .and_then(|value| value.as_array())
    {
        for item in candidates.iter().take(4) {
            let key = arkmemory_kg_json_string(item, "key");
            let value = arkmemory_kg_json_string(item, "value");
            let evidence = arkmemory_kg_json_string(item, "evidence");
            if key.is_empty() || value.is_empty() || evidence.is_empty() {
                continue;
            }
            result
                .memory_candidates
                .push(ArkmemoryExtractedMemoryCandidate {
                    key: arkmemory_kg_normalize_token(&key, "document_memory"),
                    value: arkmemory_truncate_chars(&value, 900),
                    category: arkmemory_kg_json_string(item, "category"),
                    kind: arkmemory_kg_json_string(item, "kind"),
                    durability: arkmemory_kg_json_string(item, "durability"),
                    scope: arkmemory_kg_json_string(item, "scope"),
                    sensitivity: arkmemory_kg_json_string(item, "sensitivity"),
                    topics: arkmemory_kg_string_tokens(item.get("topics"), 8),
                    confidence: arkmemory_kg_json_confidence(item),
                    evidence: arkmemory_truncate_chars(&evidence, 500),
                    reason: arkmemory_truncate_chars(
                        &arkmemory_kg_json_string(item, "reason"),
                        300,
                    ),
                    looks_sensitive: arkmemory_kg_json_bool(item, "looks_sensitive"),
                    sensitive_reason: Some(arkmemory_kg_json_string(item, "sensitive_reason"))
                        .filter(|value| !value.is_empty()),
                });
        }
    }
    result
}

async fn arkmemory_extract_knowledge_from_source(
    llm: &crate::core::LlmClient,
    source: &ArkmemoryKnowledgeExtractionSource,
    include_memory_candidates: bool,
) -> Result<ArkmemoryKnowledgeExtractionResult> {
    let text = arkmemory_truncate_chars(&source.text, 6_000);
    if text.trim().chars().count() < 80 {
        return Ok(ArkmemoryKnowledgeExtractionResult::default());
    }
    let memory_candidate_rule = if include_memory_candidates {
        "Also emit memory_candidates only for durable or explicitly bounded facts, preferences, constraints, or project/domain memory that are directly stated in this document text and would help future assistance. These are review candidates only; be selective."
    } else {
        "Return an empty memory_candidates array."
    };
    let system_prompt = format!(
        "You are AgentArk's background memory graph extractor. Extract only what the source text itself directly supports. Do not rely on model world knowledge, co-occurrence, headings alone, keyword rules, exact phrase matching, or guessed implications.\n\nReturn JSON only with this shape:\n{{\"relations\":[{{\"source_entity_name\":\"canonical entity name\",\"source_entity_type\":\"person|organization|project|product|document|concept|other\",\"relation_type\":\"open_snake_case_relation\",\"target_entity_name\":\"canonical entity name\",\"target_entity_type\":\"person|organization|project|product|document|concept|other\",\"polarity\":\"supports|contradicts\",\"confidence\":0.0,\"evidence\":\"short source excerpt\",\"reason\":\"brief rationale\"}}],\"memory_candidates\":[{{\"key\":\"stable_snake_case_slot\",\"value\":\"concise memory value\",\"category\":\"profile_fact|assistant_preference|work_preference|project_domain_memory|ephemeral_context|knowledge|other\",\"kind\":\"identity|preference|workflow|constraint|project_domain_memory|knowledge|other\",\"durability\":\"permanent|temporary|situational\",\"scope\":\"global|project|conversation\",\"sensitivity\":\"prompt_safe|personal_identifier|sensitive|crisis_sensitive\",\"topics\":[\"semantic topic\"],\"confidence\":0.0,\"evidence\":\"short source excerpt\",\"reason\":\"brief rationale\",\"looks_sensitive\":false,\"sensitive_reason\":\"\"}}]}}\n\nRules:\n- Relations are candidates, not confirmed real-world facts. Emit a relation only when the source text explicitly states or strongly entails a useful relationship between two named/identifiable entities.\n- Do not emit generic topical associations, repeated words, document structure, vague similarity, or relations that merely say a document mentions a concept.\n- Prefer no output over low-value or weakly supported output.\n- Keep at most five relations and at most two memory candidates for this source.\n- Do not include credentials, secrets, tokens, private keys, passwords, or auth material.\n- {memory_candidate_rule}"
    );
    let user_message = format!(
        "Source kind: {}\nSource id: {}\nTitle: {}\n\nSource text:\n{}",
        source.source_kind, source.source_id, source.title, text
    );
    let response = llm
        .chat_classifier_bounded(&system_prompt, &user_message, 1_400)
        .await?;
    Ok(arkmemory_parse_knowledge_extraction(&response.content))
}

async fn arkmemory_upsert_relation_candidate(
    storage: &crate::storage::Storage,
    source: &ArkmemoryKnowledgeExtractionSource,
    relation: &ArkmemoryExtractedRelation,
) -> Result<bool> {
    if relation.confidence < ARKMEMORY_KNOWLEDGE_RELATION_MIN_CONFIDENCE {
        return Ok(false);
    }
    let project_id = source.project_id.as_deref();
    let source_entity_type = arkmemory_kg_normalize_token(&relation.source_entity_type, "entity");
    let target_entity_type = arkmemory_kg_normalize_token(&relation.target_entity_type, "entity");
    let source_normalized_name = arkmemory_kg_normalize_name(&relation.source_entity_name);
    let target_normalized_name = arkmemory_kg_normalize_name(&relation.target_entity_name);
    if source_normalized_name.is_empty() || target_normalized_name.is_empty() {
        return Ok(false);
    }
    let now = chrono::Utc::now().to_rfc3339();
    let source_entity_id =
        arkmemory_kg_entity_id(project_id, &source_entity_type, &source_normalized_name);
    let target_entity_id =
        arkmemory_kg_entity_id(project_id, &target_entity_type, &target_normalized_name);
    let source_entity = crate::storage::knowledge_entity::Model {
        id: source_entity_id.clone(),
        entity_type: source_entity_type.clone(),
        canonical_name: relation.source_entity_name.clone(),
        normalized_name: source_normalized_name,
        project_id: source.project_id.clone(),
        status: "active".to_string(),
        confidence: relation.confidence,
        aliases: serde_json::json!([]),
        metadata: serde_json::json!({ "last_source_kind": source.source_kind }),
        first_seen_at: now.clone(),
        last_seen_at: now.clone(),
        created_at: now.clone(),
        updated_at: now.clone(),
    };
    let target_entity = crate::storage::knowledge_entity::Model {
        id: target_entity_id.clone(),
        entity_type: target_entity_type.clone(),
        canonical_name: relation.target_entity_name.clone(),
        normalized_name: arkmemory_kg_normalize_name(&relation.target_entity_name),
        project_id: source.project_id.clone(),
        status: "active".to_string(),
        confidence: relation.confidence,
        aliases: serde_json::json!([]),
        metadata: serde_json::json!({ "last_source_kind": source.source_kind }),
        first_seen_at: now.clone(),
        last_seen_at: now.clone(),
        created_at: now.clone(),
        updated_at: now.clone(),
    };
    storage.upsert_knowledge_entity(&source_entity).await?;
    storage.upsert_knowledge_entity(&target_entity).await?;

    let relation_type = arkmemory_kg_normalize_token(&relation.relation_type, "related_to");
    let relation_id = arkmemory_kg_relation_id(
        project_id,
        &source_entity_id,
        &relation_type,
        &target_entity_id,
    );
    let polarity = match relation.polarity.trim() {
        "contradicts" => "contradicts",
        _ => "supports",
    };
    let evidence_id = arkmemory_kg_evidence_id(
        &relation_id,
        &source.source_kind,
        &source.source_id,
        polarity,
        &relation.evidence,
    );
    let prior_evidence = storage
        .list_knowledge_relation_evidence_for_relations(&[relation_id.clone()], 1_000)
        .await?;
    let evidence_is_new = prior_evidence.iter().all(|item| item.id != evidence_id);
    let prior_relation = storage.get_knowledge_relation(&relation_id).await?;
    let mut support_count = prior_relation
        .as_ref()
        .map(|item| item.support_count)
        .unwrap_or_else(|| {
            prior_evidence
                .iter()
                .filter(|item| item.polarity == "supports")
                .count() as i32
        });
    let mut contradiction_count = prior_relation
        .as_ref()
        .map(|item| item.contradiction_count)
        .unwrap_or_else(|| {
            prior_evidence
                .iter()
                .filter(|item| item.polarity == "contradicts")
                .count() as i32
        });
    if evidence_is_new {
        if polarity == "contradicts" {
            contradiction_count += 1;
        } else {
            support_count += 1;
        }
    }
    let stored_relation = crate::storage::knowledge_relation::Model {
        id: relation_id.clone(),
        source_entity_id: source_entity_id.clone(),
        target_entity_id: target_entity_id.clone(),
        relation_type: relation_type.clone(),
        status: prior_relation
            .as_ref()
            .map(|item| item.status.clone())
            .unwrap_or_else(|| "candidate".to_string()),
        confidence: prior_relation
            .as_ref()
            .map(|item| item.confidence.max(relation.confidence))
            .unwrap_or(relation.confidence),
        project_id: source.project_id.clone(),
        valid_from: None,
        valid_until: None,
        support_count,
        contradiction_count,
        metadata: serde_json::json!({
            "last_reason": relation.reason,
            "last_source_kind": source.source_kind,
            "last_source_id": source.source_id,
        }),
        first_seen_at: prior_relation
            .as_ref()
            .map(|item| item.first_seen_at.clone())
            .unwrap_or_else(|| now.clone()),
        last_seen_at: now.clone(),
        created_at: prior_relation
            .as_ref()
            .map(|item| item.created_at.clone())
            .unwrap_or_else(|| now.clone()),
        updated_at: now.clone(),
    };
    storage.upsert_knowledge_relation(&stored_relation).await?;
    let evidence = crate::storage::knowledge_relation_evidence::Model {
        id: evidence_id,
        relation_id,
        evidence_kind: source.source_kind.clone(),
        evidence_ref: source.source_id.clone(),
        memory_id: source.memory_id.clone(),
        message_id: None,
        document_id: source.document_id.clone(),
        project_id: source.project_id.clone(),
        conversation_id: source.conversation_id.clone(),
        polarity: polarity.to_string(),
        confidence: relation.confidence,
        excerpt: Some(relation.evidence.clone()),
        metadata: serde_json::json!({
            "document_chunk_id": source.document_chunk_id,
            "reason": relation.reason,
        }),
        created_at: now,
    };
    storage
        .upsert_knowledge_relation_evidence(&evidence)
        .await?;
    Ok(true)
}

async fn arkmemory_queue_document_memory_candidate(
    storage: &crate::storage::Storage,
    source: &ArkmemoryKnowledgeExtractionSource,
    capture_event_id: Option<&str>,
    candidate: &ArkmemoryExtractedMemoryCandidate,
) -> Result<bool> {
    if candidate.confidence < ARKMEMORY_DOCUMENT_MEMORY_MIN_CONFIDENCE
        || candidate.looks_sensitive
        || !arkmemory_document_memory_sensitivity_safe(&candidate.sensitivity)
        || candidate.value.trim().is_empty()
        || candidate.evidence.trim().is_empty()
    {
        return Ok(false);
    }
    let now = chrono::Utc::now().to_rfc3339();
    let category = arkmemory_kg_normalize_token(&candidate.category, "knowledge");
    let kind = arkmemory_kg_normalize_token(&candidate.kind, &category);
    let durability = match candidate.durability.trim() {
        "temporary" | "situational" | "permanent" => candidate.durability.trim(),
        _ => "permanent",
    };
    let scope = match candidate.scope.trim() {
        "project" | "conversation" | "global" => candidate.scope.trim(),
        _ => "project",
    };
    let semantic_key = arkmemory_kg_stable_id(
        "document-memory-subject",
        &[
            source.project_id.as_deref().unwrap_or_default(),
            scope,
            &candidate.key,
        ],
    );
    let operation_id = arkmemory_kg_stable_id(
        "document-memory-operation",
        &[
            source.source_id.as_str(),
            candidate.key.as_str(),
            candidate.value.as_str(),
        ],
    );
    let evidence_refs = serde_json::json!([
        format!(
            "document:{}",
            source.document_id.as_deref().unwrap_or_default()
        ),
        format!(
            "document_chunk:{}",
            source
                .document_chunk_id
                .as_deref()
                .unwrap_or(source.source_id.as_str())
        ),
    ]);
    let model_metadata = serde_json::json!({
        "memory_category": category,
        "topics": candidate.topics,
        "sensitivity": if candidate.sensitivity.trim().is_empty() { "prompt_safe" } else { candidate.sensitivity.trim() },
        "source_support": "explicit_document_text",
        "source_evidence": candidate.evidence,
        "scope_explicit": true,
        "semantic_key": semantic_key,
        "document_id": source.document_id,
        "document_chunk_id": source.document_chunk_id,
    });
    let operation = crate::storage::memory_operation::Model {
        id: operation_id.clone(),
        capture_event_id: capture_event_id.map(str::to_string),
        operation_type: "add".to_string(),
        status: "queued_review".to_string(),
        target_memory_id: None,
        applied_memory_id: None,
        key: Some(candidate.key.clone()),
        value: Some(candidate.value.clone()),
        memory_kind: kind,
        durability: durability.to_string(),
        scope: scope.to_string(),
        project_id: source.project_id.clone(),
        conversation_id: source.conversation_id.clone(),
        confidence: candidate.confidence,
        looks_sensitive: false,
        sensitive_reason: candidate.sensitive_reason.clone(),
        valid_from: None,
        expires_at: None,
        review_at: None,
        rationale: Some(candidate.reason.clone()),
        evidence_refs: evidence_refs.clone(),
        model_metadata: model_metadata.clone(),
        apply_metadata: serde_json::json!({ "source": "document_memory_extractor" }),
        applied_at: None,
        reviewed_at: None,
        review_notes: Some(
            "Queued from document extraction; requires review before Memory stores it.".to_string(),
        ),
        created_at: now.clone(),
        updated_at: now.clone(),
    };
    storage.upsert_memory_operation(&operation).await?;
    let candidate_row = crate::storage::learning_candidate::Model {
        id: format!("memory-candidate-{}", operation_id),
        candidate_type: "memory_add".to_string(),
        subject_key: semantic_key,
        title: format!("Review document memory: {}", candidate.key),
        summary: Some(candidate.reason.clone()),
        project_id: source.project_id.clone(),
        conversation_id: source.conversation_id.clone(),
        pattern_id: None,
        evidence_refs,
        proposed_content: serde_json::json!({
            "operation_id": operation_id,
            "capture_event_id": capture_event_id,
            "operation_type": "add",
            "target_memory_id": null,
            "applied_memory_id": null,
            "key": candidate.key,
            "value": candidate.value,
            "memory_kind": operation.memory_kind,
            "memory_category": model_metadata.get("memory_category").cloned(),
            "topics": model_metadata.get("topics").cloned(),
            "durability": operation.durability,
            "scope": operation.scope,
            "confidence": operation.confidence,
            "looks_sensitive": false,
            "sensitive_reason": operation.sensitive_reason,
            "sensitivity": model_metadata.get("sensitivity").cloned(),
            "valid_from": null,
            "expires_at": null,
            "review_at": null,
            "rationale": operation.rationale,
            "status": operation.status,
            "scope_explicit": true,
            "semantic_key": operation.model_metadata.get("semantic_key").cloned(),
            "source": "document_memory_extractor",
        }),
        confidence: candidate.confidence,
        approval_status: "draft".to_string(),
        review_notes: Some(
            "Document-derived memory candidate. Review before applying.".to_string(),
        ),
        reviewed_at: None,
        approved_ref: None,
        created_at: now.clone(),
        updated_at: now,
    };
    storage.upsert_learning_candidate(&candidate_row).await?;
    Ok(true)
}

async fn arkmemory_persist_knowledge_extraction(
    storage: &crate::storage::Storage,
    source: &ArkmemoryKnowledgeExtractionSource,
    capture_event_id: Option<&str>,
    extraction: ArkmemoryKnowledgeExtractionResult,
    include_memory_candidates: bool,
) -> Result<ArkmemoryKnowledgeExtractionStats> {
    let mut stats = ArkmemoryKnowledgeExtractionStats {
        sources_processed: 1,
        ..Default::default()
    };
    for relation in extraction.relations {
        if relation.confidence < ARKMEMORY_KNOWLEDGE_RELATION_MIN_CONFIDENCE {
            stats.skipped_low_confidence += 1;
            continue;
        }
        if arkmemory_upsert_relation_candidate(storage, source, &relation).await? {
            stats.relations_written += 1;
        }
    }
    if include_memory_candidates {
        for candidate in extraction.memory_candidates {
            if candidate.confidence < ARKMEMORY_DOCUMENT_MEMORY_MIN_CONFIDENCE
                || candidate.looks_sensitive
                || !arkmemory_document_memory_sensitivity_safe(&candidate.sensitivity)
            {
                stats.skipped_low_confidence += 1;
                continue;
            }
            if arkmemory_queue_document_memory_candidate(
                storage,
                source,
                capture_event_id,
                &candidate,
            )
            .await?
            {
                stats.memory_candidates_queued += 1;
            }
        }
    }
    Ok(stats)
}

fn arkmemory_knowledge_source_from_memory(
    item: &crate::storage::experience_item::Model,
) -> ArkmemoryKnowledgeExtractionSource {
    ArkmemoryKnowledgeExtractionSource {
        source_kind: "memory".to_string(),
        source_id: item.id.clone(),
        title: item.title.clone(),
        text: item.content.clone(),
        project_id: item.project_id.clone(),
        conversation_id: item.conversation_id.clone(),
        memory_id: Some(item.id.clone()),
        document_id: None,
        document_chunk_id: None,
    }
}

fn arkmemory_knowledge_source_from_document_chunk(
    document: &crate::storage::document::Model,
    chunk: &crate::storage::document_chunk::Model,
) -> ArkmemoryKnowledgeExtractionSource {
    ArkmemoryKnowledgeExtractionSource {
        source_kind: "document_chunk".to_string(),
        source_id: chunk.id.clone(),
        title: format!("{} #{}", document.filename, chunk.chunk_index),
        text: chunk.content.clone(),
        project_id: document.project_id.clone(),
        conversation_id: None,
        memory_id: None,
        document_id: Some(document.id.clone()),
        document_chunk_id: Some(chunk.id.clone()),
    }
}

fn arkmemory_merge_extraction_stats(
    total: &mut ArkmemoryKnowledgeExtractionStats,
    next: ArkmemoryKnowledgeExtractionStats,
) {
    total.sources_seen += next.sources_seen;
    total.sources_processed += next.sources_processed;
    total.relations_written += next.relations_written;
    total.memory_candidates_queued += next.memory_candidates_queued;
    total.skipped_low_confidence += next.skipped_low_confidence;
    total.failed_sources += next.failed_sources;
}

async fn arkmemory_run_knowledge_extraction_sources(
    storage: crate::storage::Storage,
    llm: crate::core::LlmClient,
    sources: Vec<ArkmemoryKnowledgeExtractionSource>,
    include_memory_candidates_for_documents: bool,
) -> ArkmemoryKnowledgeExtractionStats {
    let mut stats = ArkmemoryKnowledgeExtractionStats::default();
    for source in sources {
        stats.sources_seen += 1;
        let include_memory_candidates =
            include_memory_candidates_for_documents && source.source_kind == "document_chunk";
        let extraction =
            match arkmemory_extract_knowledge_from_source(&llm, &source, include_memory_candidates)
                .await
            {
                Ok(extraction) => extraction,
                Err(error) => {
                    stats.failed_sources += 1;
                    tracing::debug!(
                        source_kind = %source.source_kind,
                        source_id = %source.source_id,
                        "Memory graph extraction source failed: {}",
                        error
                    );
                    continue;
                }
            };
        match arkmemory_persist_knowledge_extraction(
            &storage,
            &source,
            None,
            extraction,
            include_memory_candidates,
        )
        .await
        {
            Ok(next) => arkmemory_merge_extraction_stats(&mut stats, next),
            Err(error) => {
                stats.failed_sources += 1;
                tracing::warn!(
                    source_kind = %source.source_kind,
                    source_id = %source.source_id,
                    "Failed to persist Memory graph extraction: {}",
                    error
                );
            }
        }
    }
    stats
}

pub(super) async fn arkmemory_mark_document_memory_extract_candidate(
    storage: &crate::storage::Storage,
    document_id: &str,
    project_id: Option<&str>,
) -> bool {
    let document_id = document_id.trim();
    if document_id.is_empty() {
        return false;
    }
    let source_hash = format!(
        "document_memory:{}",
        arkmemory_stable_event_id(&[document_id])
    );
    if storage
        .count_memory_capture_events_by_source_hash(&source_hash)
        .await
        .unwrap_or(0)
        > 0
    {
        return false;
    }
    let now = chrono::Utc::now().to_rfc3339();
    let event = crate::storage::memory_capture_event::Model {
        id: arkmemory_kg_stable_id("document-memory-capture", &[document_id]),
        source_message_id: None,
        conversation_id: None,
        project_id: project_id.map(str::to_string),
        channel: "documents".to_string(),
        status: ARKMEMORY_DOCUMENT_MEMORY_PENDING_STATUS.to_string(),
        capture_kind: ARKMEMORY_DOCUMENT_MEMORY_CAPTURE_KIND.to_string(),
        source_hash,
        attempt_metadata: serde_json::json!({
            "schema_version": 1,
            "document_id": document_id,
            "next_chunk_index": 0,
            "queued_at": now,
            "source": "document_upload",
        }),
        error_history: serde_json::json!([]),
        replay_count: 0,
        next_retry_at: None,
        completed_at: None,
        created_at: now.clone(),
        updated_at: now,
    };
    match storage.upsert_memory_capture_event(&event).await {
        Ok(_) => true,
        Err(error) => {
            tracing::warn!(
                document_id,
                "Failed to queue document Memory extraction: {}",
                error
            );
            false
        }
    }
}

async fn arkmemory_process_document_memory_capture_event(
    storage: &crate::storage::Storage,
    llm: &crate::core::LlmClient,
    mut event: crate::storage::memory_capture_event::Model,
) -> Result<ArkmemoryKnowledgeExtractionStats> {
    let document_id = event
        .attempt_metadata
        .get("document_id")
        .and_then(|value| value.as_str())
        .map(str::trim)
        .filter(|value| !value.is_empty())
        .ok_or_else(|| {
            anyhow::anyhow!("document Memory extraction event is missing document_id")
        })?;
    let next_chunk_index = event
        .attempt_metadata
        .get("next_chunk_index")
        .and_then(|value| value.as_i64())
        .unwrap_or(1)
        .clamp(0, i32::MAX as i64) as i32;
    let document = storage
        .get_document(document_id)
        .await?
        .ok_or_else(|| anyhow::anyhow!("document not found or not visible"))?;
    let chunks = storage
        .list_document_chunks_for_document_window(
            document_id,
            next_chunk_index,
            ARKMEMORY_DOCUMENT_EXTRACT_CHUNK_BATCH_LIMIT,
        )
        .await?;
    if chunks.is_empty() {
        event.status = ARKMEMORY_DOCUMENT_MEMORY_COMPLETED_STATUS.to_string();
        event.completed_at = Some(chrono::Utc::now().to_rfc3339());
        event.updated_at = chrono::Utc::now().to_rfc3339();
        event.attempt_metadata["completed_at"] = serde_json::json!(event.completed_at.clone());
        storage.upsert_memory_capture_event(&event).await?;
        return Ok(ArkmemoryKnowledgeExtractionStats::default());
    }
    let mut total = ArkmemoryKnowledgeExtractionStats::default();
    let mut last_chunk_index = next_chunk_index;
    for chunk in &chunks {
        last_chunk_index = chunk.chunk_index;
        let source = arkmemory_knowledge_source_from_document_chunk(&document, chunk);
        let extraction = match arkmemory_extract_knowledge_from_source(llm, &source, true).await {
            Ok(extraction) => extraction,
            Err(error) => {
                total.failed_sources += 1;
                tracing::debug!(
                    document_id = %document.id,
                    chunk_id = %chunk.id,
                    "Document Memory extraction skipped chunk after model failure: {}",
                    error
                );
                continue;
            }
        };
        match arkmemory_persist_knowledge_extraction(
            storage,
            &source,
            Some(&event.id),
            extraction,
            true,
        )
        .await
        {
            Ok(stats) => arkmemory_merge_extraction_stats(&mut total, stats),
            Err(error) => {
                total.failed_sources += 1;
                tracing::warn!(
                    document_id = %document.id,
                    chunk_id = %chunk.id,
                    "Document Memory extraction failed to persist chunk: {}",
                    error
                );
            }
        }
    }
    let next_index = last_chunk_index.saturating_add(1);
    let has_more = next_index < document.chunk_count;
    event.status = if has_more {
        ARKMEMORY_DOCUMENT_MEMORY_PENDING_STATUS.to_string()
    } else {
        ARKMEMORY_DOCUMENT_MEMORY_COMPLETED_STATUS.to_string()
    };
    event.completed_at = if has_more {
        None
    } else {
        Some(chrono::Utc::now().to_rfc3339())
    };
    event.updated_at = chrono::Utc::now().to_rfc3339();
    event.replay_count = event.replay_count.saturating_add(1);
    event.attempt_metadata["next_chunk_index"] = serde_json::json!(next_index);
    event.attempt_metadata["last_batch"] = serde_json::json!({
        "processed_chunks": chunks.len(),
        "relations_written": total.relations_written,
        "memory_candidates_queued": total.memory_candidates_queued,
        "finished_document": !has_more,
        "updated_at": event.updated_at,
    });
    storage.upsert_memory_capture_event(&event).await?;
    Ok(total)
}

async fn arkmemory_process_document_memory_capture_batch(
    state: AppState,
    limit: u64,
) -> ArkmemoryKnowledgeExtractionStats {
    let Some(_guard) = arkmemory_document_extract_try_guard() else {
        return ArkmemoryKnowledgeExtractionStats::default();
    };
    let (storage, llm) = {
        let agent = state.agent.read().await;
        (agent.storage.clone(), agent.llm.clone())
    };
    let events = match storage
        .list_memory_capture_events_by_statuses_all_scopes(
            &[ARKMEMORY_DOCUMENT_MEMORY_PENDING_STATUS],
            limit,
        )
        .await
    {
        Ok(events) => events
            .into_iter()
            .filter(|event| event.capture_kind == ARKMEMORY_DOCUMENT_MEMORY_CAPTURE_KIND)
            .collect::<Vec<_>>(),
        Err(error) => {
            tracing::debug!("Failed to list document Memory extraction jobs: {}", error);
            return ArkmemoryKnowledgeExtractionStats::default();
        }
    };
    let mut total = ArkmemoryKnowledgeExtractionStats::default();
    for mut event in events {
        let now = chrono::Utc::now().to_rfc3339();
        match storage
            .try_claim_memory_capture_event_status(
                &event.id,
                ARKMEMORY_DOCUMENT_MEMORY_PENDING_STATUS,
                ARKMEMORY_DOCUMENT_MEMORY_PROCESSING_STATUS,
                &now,
            )
            .await
        {
            Ok(true) => {
                event.status = ARKMEMORY_DOCUMENT_MEMORY_PROCESSING_STATUS.to_string();
                event.updated_at = now;
            }
            Ok(false) => continue,
            Err(error) => {
                tracing::warn!(
                    capture_event_id = %event.id,
                    "Failed to claim document Memory extraction event: {}",
                    error
                );
                continue;
            }
        }
        match arkmemory_process_document_memory_capture_event(&storage, &llm, event.clone()).await {
            Ok(stats) => arkmemory_merge_extraction_stats(&mut total, stats),
            Err(error) => {
                total.failed_sources += 1;
                event.status = ARKMEMORY_DOCUMENT_MEMORY_FAILED_STATUS.to_string();
                event.completed_at = Some(chrono::Utc::now().to_rfc3339());
                event.updated_at = chrono::Utc::now().to_rfc3339();
                event.error_history = serde_json::json!([{
                    "code": "document_memory_extract_failed",
                    "message": arkmemory_truncate_chars(&error.to_string(), 500),
                    "at": event.updated_at,
                }]);
                let _ = storage.upsert_memory_capture_event(&event).await;
            }
        }
    }
    total
}

async fn arkmemory_document_memory_server_is_idle(state: &AppState) -> bool {
    if server_under_load(state).await || crate::sentinel::is_pulse_running() {
        return false;
    }
    let agent = state.agent.read().await;
    agent.active_message_request_count() == 0
}

async fn arkmemory_document_memory_idle_loop(
    state: AppState,
    mut shutdown_rx: tokio::sync::watch::Receiver<bool>,
) {
    let mut interval = tokio::time::interval(ARKMEMORY_DOCUMENT_MEMORY_IDLE_INTERVAL);
    interval.set_missed_tick_behavior(tokio::time::MissedTickBehavior::Delay);
    loop {
        tokio::select! {
            _ = shutdown_rx.changed() => break,
            _ = interval.tick() => {}
        }
        if !arkmemory_document_memory_server_is_idle(&state).await {
            continue;
        }
        let stats = arkmemory_process_document_memory_capture_batch(state.clone(), 2).await;
        if stats.sources_processed > 0
            || stats.relations_written > 0
            || stats.memory_candidates_queued > 0
        {
            tracing::debug!(
                sources_processed = stats.sources_processed,
                relations_written = stats.relations_written,
                memory_candidates_queued = stats.memory_candidates_queued,
                "Document Memory extraction idle pass completed"
            );
        }
    }
}

pub(super) fn spawn_arkmemory_document_memory_idle_loop(
    state: AppState,
    shutdown_rx: tokio::sync::watch::Receiver<bool>,
) -> Option<tokio::task::JoinHandle<()>> {
    if ARKMEMORY_DOCUMENT_MEMORY_IDLE_LOOP_STARTED
        .compare_exchange(false, true, Ordering::AcqRel, Ordering::Acquire)
        .is_err()
    {
        return None;
    }
    Some(crate::spawn_logged!(
        "src/channels/http/memory_control.rs:document_memory_idle_loop",
        async move {
            arkmemory_document_memory_idle_loop(state, shutdown_rx).await;
        }
    ))
}

pub(super) async fn arkmemory_graph(
    State(state): State<AppState>,
    axum::extract::Query(params): axum::extract::Query<HashMap<String, String>>,
) -> Response {
    let mode = params
        .get("mode")
        .map(|value| value.trim())
        .filter(|value| !value.is_empty())
        .unwrap_or("map");
    let project_id = arkmemory_project_param(&params);
    let limit = arkmemory_graph_limit(&params);
    let categories = arkmemory_graph_params_filter(&params, "category")
        .into_iter()
        .collect::<HashSet<_>>();
    let requested_statuses = arkmemory_graph_params_filter(&params, "status");
    let statuses_for_query = if requested_statuses.is_empty() {
        vec!["active".to_string()]
    } else {
        requested_statuses.clone()
    };
    let statuses = statuses_for_query.iter().cloned().collect::<HashSet<_>>();
    let edge_types = arkmemory_graph_params_filter(&params, "edge_type")
        .into_iter()
        .collect::<HashSet<_>>();
    let include_semantic = arkmemory_graph_bool_param(&params, "include_semantic", true)
        && arkmemory_graph_filter_edge("semantic_nearby", &edge_types);
    let include_knowledge_relations = edge_types.is_empty()
        || edge_types.contains("knowledge_relation")
        || edge_types.contains("relation_evidence");
    let relation_statuses = arkmemory_graph_relation_statuses(&params);
    let entity_statuses = arkmemory_graph_entity_statuses(&params);
    let relation_types = arkmemory_graph_params_filter(&params, "relation_type");
    let source_filters = arkmemory_graph_params_filter(&params, "source")
        .into_iter()
        .collect::<HashSet<_>>();
    let min_confidence = arkmemory_graph_min_confidence(&params);
    let updated_after = arkmemory_graph_updated_after(&params);
    let updated_before = arkmemory_graph_updated_before(&params);
    let semantic_threshold = arkmemory_graph_semantic_threshold(&params);
    let semantic_per_node = arkmemory_graph_per_node_semantic_limit(&params);
    let agent = state.agent.read().await;
    let storage = &agent.storage;

    let result = async {
        let mut nodes = Vec::<ArkmemoryGraphNode>::new();
        let mut edges = Vec::<ArkmemoryGraphEdge>::new();
        let mut memory_items = Vec::<crate::storage::experience_item::Model>::new();
        let mut seen_nodes = HashSet::<String>::new();
        let mut knowledge_relation_count = 0usize;
        let mut truncated = false;

        if mode == "focus" {
            let memory_id = params
                .get("memory_id")
                .map(|value| value.trim())
                .filter(|value| !value.is_empty())
                .ok_or_else(|| anyhow::anyhow!("memory_id is required for focused graph mode"))?;
            let memory = storage
                .get_experience_item(memory_id)
                .await?
                .ok_or_else(|| anyhow::anyhow!("Memory item not found."))?;
            if !arkmemory_item_is_memory(&memory)
                || !arkmemory_item_visible_for_project(&memory, project_id)
                || !arkmemory_graph_filter_item(
                    &memory,
                    &categories,
                    &statuses,
                    min_confidence,
                    updated_after.as_deref(),
                    updated_before.as_deref(),
                )
            {
                anyhow::bail!("Memory item is outside the graph scope.");
            }

            seen_nodes.insert(memory.id.clone());
            nodes.push(arkmemory_graph_memory_node(&memory, true));
            memory_items.push(memory.clone());

            let raw_edges = storage
                .list_experience_edges_for_item(memory_id, limit.saturating_mul(4).min(240))
                .await?;
            let mut related_items = HashMap::new();
            for edge in &raw_edges {
                let related_id = if edge.source_kind == "experience_item"
                    && edge.source_ref != memory_id
                {
                    Some(edge.source_ref.as_str())
                } else if edge.target_kind == "experience_item" && edge.target_ref != memory_id {
                    Some(edge.target_ref.as_str())
                } else {
                    None
                };
                let Some(related_id) = related_id else {
                    continue;
                };
                if related_items.contains_key(related_id) {
                    continue;
                }
                if related_items.len() >= limit.saturating_sub(1) as usize {
                    truncated = true;
                    break;
                }
                if let Some(item) = storage.get_experience_item(related_id).await? {
                    if arkmemory_item_is_memory(&item)
                        && arkmemory_item_visible_for_project(&item, project_id)
                        && arkmemory_graph_filter_item(
                            &item,
                            &categories,
                            &statuses,
                            min_confidence,
                            updated_after.as_deref(),
                            updated_before.as_deref(),
                        )
                    {
                        related_items.insert(related_id.to_string(), item);
                    }
                }
            }
            for item in related_items.into_values() {
                if seen_nodes.insert(item.id.clone()) {
                    nodes.push(arkmemory_graph_memory_node(&item, false));
                    memory_items.push(item);
                }
            }

            let memory_node_ids = memory_items
                .iter()
                .map(|item| item.id.clone())
                .collect::<HashSet<_>>();
            for edge in raw_edges {
                if edges.len() >= limit.saturating_mul(3) as usize {
                    truncated = true;
                    break;
                }
                if !arkmemory_graph_filter_edge(&edge.edge_type, &edge_types) {
                    continue;
                }
                if memory_node_ids.contains(&edge.source_ref)
                    && memory_node_ids.contains(&edge.target_ref)
                {
                    edges.push(arkmemory_graph_explicit_edge(&edge));
                }
            }

            let source_cap = (limit as usize / 2).clamp(8, 40);
            for source in arkmemory_memory_sources(&memory)
                .into_iter()
                .take(source_cap)
            {
                if arkmemory_graph_filter_edge("evidence", &edge_types) {
                    arkmemory_graph_add_ref_node(
                        &mut nodes,
                        &mut edges,
                        &mut seen_nodes,
                        memory_id,
                        &source,
                        "evidence",
                    );
                }
            }
            if arkmemory_graph_filter_edge("evidence", &edge_types) {
                let evidence_links = storage
                    .list_memory_evidence_links_for_memory(memory_id, project_id, source_cap as u64)
                    .await?;
                for link in evidence_links {
                    arkmemory_graph_add_ref_node(
                        &mut nodes,
                        &mut edges,
                        &mut seen_nodes,
                        memory_id,
                        &format!("{}:{}", link.evidence_kind, link.evidence_ref),
                        "evidence",
                    );
                }
            }
            if arkmemory_graph_filter_edge("operation", &edge_types) {
                let operations = storage
                    .list_memory_operations_for_memory(memory_id, project_id, 24)
                    .await?;
                for operation in operations {
                    arkmemory_graph_add_ref_node(
                        &mut nodes,
                        &mut edges,
                        &mut seen_nodes,
                        memory_id,
                        &format!("operation:{}", operation.id),
                        "operation",
                    );
                }
            }
            if arkmemory_graph_filter_edge("event", &edge_types) {
                let events = storage
                    .list_recall_events_for_memory(memory_id, 24, project_id)
                    .await?;
                for event in events {
                    arkmemory_graph_add_ref_node(
                        &mut nodes,
                        &mut edges,
                        &mut seen_nodes,
                        memory_id,
                        &format!("event:{}", event.id),
                        "event",
                    );
                }
            }
            if include_knowledge_relations {
                let evidence_rows = storage
                    .list_knowledge_relation_evidence_for_memory(memory_id, limit.min(120))
                    .await?;
                let relation_ids = evidence_rows
                    .iter()
                    .filter(|evidence| {
                        arkmemory_graph_evidence_matches_sources(evidence, &source_filters)
                    })
                    .map(|evidence| evidence.relation_id.clone())
                    .collect::<HashSet<_>>();
                let mut evidence_by_relation: HashMap<
                    String,
                    Vec<crate::storage::knowledge_relation_evidence::Model>,
                > = HashMap::new();
                for evidence in evidence_rows {
                    if !arkmemory_graph_evidence_matches_sources(&evidence, &source_filters) {
                        continue;
                    }
                    if arkmemory_graph_filter_edge("evidence", &edge_types) {
                        if let Some(source_ref) = arkmemory_graph_relation_evidence_ref(&evidence) {
                            arkmemory_graph_add_ref_node(
                                &mut nodes,
                                &mut edges,
                                &mut seen_nodes,
                                memory_id,
                                &source_ref,
                                "evidence",
                            );
                        }
                    }
                    evidence_by_relation
                        .entry(evidence.relation_id.clone())
                        .or_default()
                        .push(evidence);
                }
                for relation_id in relation_ids {
                    let Some(relation) = storage.get_knowledge_relation(&relation_id).await? else {
                        continue;
                    };
                    if !relation_statuses.is_empty()
                        && !relation_statuses.contains(&relation.status)
                    {
                        continue;
                    }
                    if !relation_types.is_empty()
                        && !relation_types.contains(&relation.relation_type)
                    {
                        continue;
                    }
                    if !arkmemory_graph_relation_matches_filters(
                        &relation,
                        min_confidence,
                        updated_after.as_deref(),
                        updated_before.as_deref(),
                    ) {
                        continue;
                    }
                    let Some(source_entity) = storage
                        .get_knowledge_entity(&relation.source_entity_id)
                        .await?
                    else {
                        continue;
                    };
                    let Some(target_entity) = storage
                        .get_knowledge_entity(&relation.target_entity_id)
                        .await?
                    else {
                        continue;
                    };
                    if (!entity_statuses.is_empty()
                        && (!entity_statuses.contains(&source_entity.status)
                            || !entity_statuses.contains(&target_entity.status)))
                        || source_entity.confidence < min_confidence
                        || target_entity.confidence < min_confidence
                        || !arkmemory_graph_updated_in_range(
                            &source_entity.updated_at,
                            updated_after.as_deref(),
                            updated_before.as_deref(),
                        )
                        || !arkmemory_graph_updated_in_range(
                            &target_entity.updated_at,
                            updated_after.as_deref(),
                            updated_before.as_deref(),
                        )
                    {
                        continue;
                    }
                    arkmemory_graph_add_knowledge_entity_node(
                        &mut nodes,
                        &mut seen_nodes,
                        &source_entity,
                        false,
                    );
                    arkmemory_graph_add_knowledge_entity_node(
                        &mut nodes,
                        &mut seen_nodes,
                        &target_entity,
                        false,
                    );
                    let relation_evidence = evidence_by_relation
                        .get(&relation.id)
                        .map(Vec::as_slice)
                        .unwrap_or(&[]);
                    if arkmemory_graph_filter_edge("knowledge_relation", &edge_types) {
                        edges.push(arkmemory_graph_knowledge_relation_edge(
                            &relation,
                            relation_evidence,
                        ));
                    }
                    if arkmemory_graph_filter_edge("relation_evidence", &edge_types) {
                        edges.push(arkmemory_graph_relation_evidence_edge(
                            memory_id,
                            &source_entity.id,
                            &relation.id,
                        ));
                        edges.push(arkmemory_graph_relation_evidence_edge(
                            memory_id,
                            &target_entity.id,
                            &relation.id,
                        ));
                    }
                    knowledge_relation_count += 1;
                }
            }
        } else {
            let scan_limit = if categories.is_empty() {
                limit
            } else {
                limit.saturating_mul(4).clamp(limit, 880)
            };
            let items = storage
                .list_memory_experience_items_for_graph(&statuses_for_query, project_id, scan_limit)
                .await?;
            for item in items {
                if !arkmemory_item_is_memory(&item)
                    || !arkmemory_graph_filter_item(
                        &item,
                        &categories,
                        &statuses,
                        min_confidence,
                        updated_after.as_deref(),
                        updated_before.as_deref(),
                    )
                {
                    continue;
                }
                if memory_items.len() >= limit as usize {
                    truncated = true;
                    break;
                }
                seen_nodes.insert(item.id.clone());
                nodes.push(arkmemory_graph_memory_node(&item, false));
                memory_items.push(item);
            }

            let ids = memory_items
                .iter()
                .map(|item| item.id.clone())
                .collect::<Vec<_>>();
            let id_set = ids.iter().cloned().collect::<HashSet<_>>();
            let raw_edges = storage
                .list_experience_edges_for_refs(&ids, limit.saturating_mul(4).min(500))
                .await?;
            for edge in raw_edges {
                if edges.len() >= limit.saturating_mul(3) as usize {
                    truncated = true;
                    break;
                }
                if !arkmemory_graph_filter_edge(&edge.edge_type, &edge_types) {
                    continue;
                }
                if id_set.contains(&edge.source_ref) && id_set.contains(&edge.target_ref) {
                    edges.push(arkmemory_graph_explicit_edge(&edge));
                }
            }
            if include_knowledge_relations {
                let entity_rows = storage
                    .list_knowledge_entities_for_graph(&entity_statuses, project_id, limit)
                    .await?;
                let entity_rows = entity_rows
                    .into_iter()
                    .filter(|entity| {
                        entity.confidence >= min_confidence
                            && arkmemory_graph_updated_in_range(
                                &entity.updated_at,
                                updated_after.as_deref(),
                                updated_before.as_deref(),
                            )
                    })
                    .collect::<Vec<_>>();
                let entity_ids = entity_rows
                    .iter()
                    .map(|entity| entity.id.clone())
                    .collect::<Vec<_>>();
                for entity in &entity_rows {
                    arkmemory_graph_add_knowledge_entity_node(
                        &mut nodes,
                        &mut seen_nodes,
                        entity,
                        false,
                    );
                }
                let relation_limit = limit.saturating_mul(4).min(500);
                let relations = storage
                    .list_knowledge_relations_for_entities(
                        &entity_ids,
                        &relation_statuses,
                        &relation_types,
                        relation_limit,
                    )
                    .await?;
                let relation_ids = relations
                    .iter()
                    .map(|relation| relation.id.clone())
                    .collect::<Vec<_>>();
                let mut evidence_by_relation: HashMap<
                    String,
                    Vec<crate::storage::knowledge_relation_evidence::Model>,
                > = HashMap::new();
                for evidence in storage
                    .list_knowledge_relation_evidence_for_relations(
                        &relation_ids,
                        limit.saturating_mul(4).min(500),
                    )
                    .await?
                {
                    evidence_by_relation
                        .entry(evidence.relation_id.clone())
                        .or_default()
                        .push(evidence);
                }
                let entity_id_set = entity_ids.into_iter().collect::<HashSet<_>>();
                for relation in relations {
                    if !entity_id_set.contains(&relation.source_entity_id)
                        || !entity_id_set.contains(&relation.target_entity_id)
                    {
                        continue;
                    }
                    if !arkmemory_graph_relation_matches_filters(
                        &relation,
                        min_confidence,
                        updated_after.as_deref(),
                        updated_before.as_deref(),
                    ) {
                        continue;
                    }
                    let relation_evidence = evidence_by_relation
                        .get(&relation.id)
                        .map(Vec::as_slice)
                        .unwrap_or(&[]);
                    if !source_filters.is_empty()
                        && !relation_evidence.iter().any(|evidence| {
                            arkmemory_graph_evidence_matches_sources(evidence, &source_filters)
                        })
                    {
                        continue;
                    }
                    if arkmemory_graph_filter_edge("knowledge_relation", &edge_types) {
                        edges.push(arkmemory_graph_knowledge_relation_edge(
                            &relation,
                            relation_evidence,
                        ));
                    }
                    knowledge_relation_count += 1;
                    if edges.len() >= limit.saturating_mul(4) as usize {
                        truncated = true;
                        break;
                    }
                }
            }
        }

        let explicit_pairs = edges
            .iter()
            .filter(|edge| !edge.semantic)
            .map(|edge| arkmemory_graph_pair_key(&edge.source, &edge.target))
            .collect::<HashSet<_>>();
        if include_semantic {
            let semantic_edges = arkmemory_graph_semantic_edges(
                &memory_items,
                &explicit_pairs,
                semantic_threshold,
                semantic_per_node,
                limit as usize,
            );
            edges.extend(semantic_edges);
        }
        let semantic_edge_count = edges.iter().filter(|edge| edge.semantic).count();
        let edge_count = edges.len();
        let node_count = nodes.len();
        Ok::<serde_json::Value, anyhow::Error>(serde_json::json!({
            "mode": if mode == "focus" { "focus" } else { "map" },
            "nodes": nodes,
            "edges": edges,
            "node_count": node_count,
            "memory_node_count": memory_items.len(),
            "edge_count": edge_count,
            "semantic_edge_count": semantic_edge_count,
            "knowledge_relation_count": knowledge_relation_count,
            "limit": limit,
            "truncated": truncated,
            "filters": {
                "categories": categories,
                "statuses": statuses,
                "edge_types": edge_types,
                "source_filters": source_filters,
                "entity_statuses": entity_statuses,
                "relation_statuses": relation_statuses,
                "relation_types": relation_types,
                "min_confidence": min_confidence,
                "updated_after": updated_after,
                "updated_before": updated_before,
                "include_semantic": include_semantic,
                "semantic_threshold": semantic_threshold,
                "semantic_per_node": semantic_per_node,
            }
        }))
    }
    .await;

    match result {
        Ok(payload) => (StatusCode::OK, Json(payload)).into_response(),
        Err(error) => (
            StatusCode::BAD_REQUEST,
            Json(ErrorResponse {
                error: error.to_string(),
            }),
        )
            .into_response(),
    }
}

pub(super) async fn arkmemory_knowledge_graph_extract(
    State(state): State<AppState>,
    axum::extract::Query(params): axum::extract::Query<HashMap<String, String>>,
    Json(body): Json<serde_json::Value>,
) -> Response {
    let project_id = arkmemory_project_param(&params).map(str::to_string);
    let include_memories = body
        .get("include_memories")
        .and_then(|value| value.as_bool())
        .unwrap_or_else(|| arkmemory_graph_bool_param(&params, "include_memories", true));
    let include_documents = body
        .get("include_documents")
        .and_then(|value| value.as_bool())
        .unwrap_or_else(|| arkmemory_graph_bool_param(&params, "include_documents", true));
    let memory_limit = body
        .get("memory_limit")
        .and_then(|value| value.as_u64())
        .or_else(|| {
            params
                .get("memory_limit")
                .and_then(|value| value.parse().ok())
        })
        .unwrap_or(ARKMEMORY_KNOWLEDGE_EXTRACT_MEMORY_LIMIT)
        .clamp(1, 220);
    let document_limit = body
        .get("document_limit")
        .and_then(|value| value.as_u64())
        .or_else(|| {
            params
                .get("document_limit")
                .and_then(|value| value.parse().ok())
        })
        .unwrap_or(ARKMEMORY_KNOWLEDGE_EXTRACT_DOCUMENT_LIMIT)
        .clamp(1, 50);
    let (storage, llm) = {
        let agent = state.agent.read().await;
        (agent.storage.clone(), agent.llm.clone())
    };
    crate::spawn_logged!(
        "src/channels/http/memory_control.rs:knowledge_graph_extract",
        async move {
            let mut sources = Vec::new();
            if include_memories {
                match storage
                    .list_memory_experience_items_for_graph(
                        &["active".to_string()],
                        project_id.as_deref(),
                        memory_limit,
                    )
                    .await
                {
                    Ok(items) => {
                        sources.extend(
                            items
                                .into_iter()
                                .filter(arkmemory_item_is_memory)
                                .map(|item| arkmemory_knowledge_source_from_memory(&item)),
                        );
                    }
                    Err(error) => {
                        tracing::warn!(
                            "Failed to load memories for knowledge graph extraction: {}",
                            error
                        );
                    }
                }
            }
            if include_documents {
                match storage
                    .list_documents(document_limit, 0, project_id.as_deref())
                    .await
                {
                    Ok(documents) => {
                        for document in documents {
                            match storage
                                .list_document_chunks_for_document_window(
                                    &document.id,
                                    0,
                                    ARKMEMORY_DOCUMENT_EXTRACT_CHUNK_BATCH_LIMIT,
                                )
                                .await
                            {
                                Ok(chunks) => {
                                    sources.extend(chunks.into_iter().map(|chunk| {
                                        arkmemory_knowledge_source_from_document_chunk(
                                            &document, &chunk,
                                        )
                                    }));
                                }
                                Err(error) => {
                                    tracing::debug!(
                                        document_id = %document.id,
                                        "Failed to load document chunks for knowledge graph extraction: {}",
                                        error
                                    );
                                }
                            }
                        }
                    }
                    Err(error) => {
                        tracing::warn!(
                            "Failed to load documents for knowledge graph extraction: {}",
                            error
                        );
                    }
                }
            }
            let stats = arkmemory_run_knowledge_extraction_sources(
                storage,
                llm,
                sources,
                include_documents,
            )
            .await;
            tracing::debug!(
                sources_seen = stats.sources_seen,
                sources_processed = stats.sources_processed,
                relations_written = stats.relations_written,
                memory_candidates_queued = stats.memory_candidates_queued,
                failed_sources = stats.failed_sources,
                "Manual Memory graph extraction completed"
            );
        }
    );
    (
        StatusCode::ACCEPTED,
        Json(serde_json::json!({
            "queued": true,
            "include_memories": include_memories,
            "include_documents": include_documents,
            "memory_limit": memory_limit,
            "document_limit": document_limit,
        })),
    )
        .into_response()
}

async fn arkmemory_set_knowledge_relation_status(
    state: AppState,
    relation_id: String,
    status: &'static str,
) -> Response {
    let relation_id = relation_id.trim();
    if relation_id.is_empty() {
        return (
            StatusCode::BAD_REQUEST,
            Json(ErrorResponse {
                error: "relation id is required".to_string(),
            }),
        )
            .into_response();
    }
    let agent = state.agent.read().await;
    match agent
        .storage
        .update_knowledge_relation_status(relation_id, status)
        .await
    {
        Ok(true) => (
            StatusCode::OK,
            Json(serde_json::json!({ "updated": true, "status": status })),
        )
            .into_response(),
        Ok(false) => (
            StatusCode::NOT_FOUND,
            Json(ErrorResponse {
                error: "relation not found".to_string(),
            }),
        )
            .into_response(),
        Err(error) => (
            StatusCode::INTERNAL_SERVER_ERROR,
            Json(ErrorResponse {
                error: error.to_string(),
            }),
        )
            .into_response(),
    }
}

pub(super) async fn arkmemory_confirm_knowledge_relation(
    State(state): State<AppState>,
    Path(id): Path<String>,
) -> Response {
    arkmemory_set_knowledge_relation_status(state, id, "confirmed").await
}

pub(super) async fn arkmemory_reject_knowledge_relation(
    State(state): State<AppState>,
    Path(id): Path<String>,
) -> Response {
    arkmemory_set_knowledge_relation_status(state, id, "rejected").await
}

pub(super) async fn arkmemory_queue(
    State(state): State<AppState>,
    axum::extract::Query(params): axum::extract::Query<HashMap<String, String>>,
) -> Response {
    let limit = arkmemory_limit(&params, 50);
    let project_id = arkmemory_project_param(&params);
    let agent = state.agent.read().await;
    match arkmemory_list_memory_candidates(&agent.storage, project_id, limit).await {
        Ok(items) => {
            let mut payloads = Vec::with_capacity(items.len());
            for item in &items {
                let replay_gate =
                    match crate::core::self_evolve::replay_gate::evaluate_candidate_replay_gate(
                        &agent.storage,
                        item,
                    )
                    .await
                    {
                        Ok(gate) => Some(gate),
                        Err(error) => {
                            tracing::warn!(
                                "Failed to evaluate Memory replay gate for candidate '{}': {}",
                                item.id,
                                error
                            );
                            None
                        }
                    };
                payloads.push(arkmemory_candidate_payload(item, replay_gate.as_ref()));
            }
            (
                StatusCode::OK,
                Json(serde_json::json!({
                    "items": payloads,
                })),
            )
                .into_response()
        }
        Err(error) => (
            StatusCode::INTERNAL_SERVER_ERROR,
            Json(ErrorResponse {
                error: error.to_string(),
            }),
        )
            .into_response(),
    }
}

pub(super) async fn arkmemory_approve_queue_item(
    State(state): State<AppState>,
    Path(id): Path<String>,
    axum::extract::Query(params): axum::extract::Query<HashMap<String, String>>,
) -> Response {
    let project_id = arkmemory_project_param(&params);
    let agent = state.agent.read().await;
    match arkmemory_apply_memory_candidate(&agent, &id, project_id).await {
        Ok(approved_ref) => (
            StatusCode::OK,
            Json(serde_json::json!({ "approved": true, "approved_ref": approved_ref })),
        )
            .into_response(),
        Err(error) => (
            StatusCode::BAD_REQUEST,
            Json(ErrorResponse {
                error: error.to_string(),
            }),
        )
            .into_response(),
    }
}

pub(super) async fn arkmemory_reject_queue_item(
    State(state): State<AppState>,
    Path(id): Path<String>,
    axum::extract::Query(params): axum::extract::Query<HashMap<String, String>>,
) -> Response {
    let _project_id = arkmemory_project_param(&params);
    let agent = state.agent.read().await;
    let storage = &agent.storage;
    let result = async {
        let mut candidate = storage
            .get_learning_candidate(&id)
            .await?
            .ok_or_else(|| anyhow::anyhow!("Memory queue item not found."))?;
        if !arkmemory_candidate_is_memory(&candidate.candidate_type) {
            anyhow::bail!("Memory queue item is not a memory operation.");
        }
        candidate = arkmemory_ensure_latest_open_candidate(storage, &candidate).await?;
        if candidate.approval_status == "applying" {
            anyhow::bail!("Memory queue item is already being applied.");
        }
        if candidate.approval_status != "draft" {
            anyhow::bail!("Memory queue item is no longer pending review.");
        }
        let rejected = storage
            .update_learning_candidate_review_if_status(
                &id,
                "draft",
                "rejected",
                Some("Rejected from Memory."),
                None,
            )
            .await?;
        if !rejected {
            anyhow::bail!("Memory queue item is no longer pending review.");
        }
        if let Some(operation_id) = candidate
            .proposed_content
            .get("operation_id")
            .and_then(|value| value.as_str())
            .map(str::trim)
            .filter(|value| !value.is_empty())
        {
            if let Some(mut operation) = storage.get_memory_operation(operation_id).await? {
                operation.status = "rejected".to_string();
                operation.reviewed_at = Some(chrono::Utc::now().to_rfc3339());
                operation.review_notes = Some("Rejected from Memory.".to_string());
                operation.updated_at = chrono::Utc::now().to_rfc3339();
                storage.upsert_memory_operation(&operation).await?;
            }
        }
        arkmemory_record_event(
            storage,
            "queue_item_rejected",
            None,
            None,
            format!("Rejected memory queue item {}", id),
            serde_json::json!({ "candidate_id": id }),
            MemoryEventContext::from_candidate(&candidate),
        )
        .await?;
        Ok::<(), anyhow::Error>(())
    }
    .await;
    match result {
        Ok(()) => (
            StatusCode::OK,
            Json(serde_json::json!({ "rejected": true })),
        )
            .into_response(),
        Err(error) => (
            StatusCode::BAD_REQUEST,
            Json(ErrorResponse {
                error: error.to_string(),
            }),
        )
            .into_response(),
    }
}

pub(super) async fn arkmemory_ledger(
    State(state): State<AppState>,
    axum::extract::Query(params): axum::extract::Query<HashMap<String, String>>,
) -> Response {
    let project_id = arkmemory_project_param(&params);
    let limit = arkmemory_limit(&params, 50);
    let offset = arkmemory_offset(&params);
    let agent = state.agent.read().await;
    match agent
        .storage
        .list_recall_events(limit, offset, project_id)
        .await
    {
        Ok(events) => {
            let mut event_payloads = Vec::with_capacity(events.len());
            for event in events {
                let mut payload = serde_json::to_value(&event).unwrap_or_else(|_| {
                    serde_json::json!({
                        "id": event.id,
                        "event_type": event.event_type,
                        "created_at": event.created_at,
                    })
                });
                if let Some(memory_id) = event
                    .memory_id
                    .as_deref()
                    .map(str::trim)
                    .filter(|id| !id.is_empty())
                {
                    let current_memory = agent
                        .storage
                        .get_experience_item(memory_id)
                        .await
                        .ok()
                        .flatten();
                    if let Some(object) = payload.as_object_mut() {
                        object.insert(
                            "memory_current_exists".to_string(),
                            serde_json::Value::Bool(current_memory.is_some()),
                        );
                        if let Some(current_memory) = current_memory {
                            object.insert(
                                "memory_current_status".to_string(),
                                serde_json::Value::String(current_memory.status),
                            );
                        }
                    }
                }
                event_payloads.push(payload);
            }
            let total = agent
                .storage
                .count_recall_events(project_id)
                .await
                .unwrap_or(0);
            (
                StatusCode::OK,
                Json(serde_json::json!({
                    "events": event_payloads,
                    "total": total,
                    "limit": limit,
                    "offset": offset,
                })),
            )
                .into_response()
        }
        Err(error) => (
            StatusCode::INTERNAL_SERVER_ERROR,
            Json(ErrorResponse {
                error: error.to_string(),
            }),
        )
            .into_response(),
    }
}

pub(super) async fn arkmemory_rollback_ledger_event(
    State(state): State<AppState>,
    Path(id): Path<String>,
    axum::extract::Query(params): axum::extract::Query<HashMap<String, String>>,
) -> Response {
    let project_id = arkmemory_project_param(&params);
    let agent = state.agent.read().await;
    let storage = &agent.storage;
    let result = async {
        let event = storage
            .get_recall_event(&id)
            .await?
            .ok_or_else(|| anyhow::anyhow!("Memory ledger event not found."))?;
        if !event.reversible || event.reverted_at.is_some() {
            anyhow::bail!("Memory ledger event is not reversible.");
        }
        let previous: crate::storage::experience_item::Model =
            serde_json::from_value(event.old_snapshot.clone()).map_err(|_| {
                anyhow::anyhow!("Memory ledger event has no restorable memory snapshot.")
            })?;
        if !arkmemory_item_is_memory(&previous)
            || !arkmemory_item_visible_for_project(&previous, project_id)
        {
            anyhow::bail!("Memory ledger event cannot restore outside the active memory scope.");
        }
        let rollback_event = arkmemory_event_model(
            "ledger_event_rolled_back",
            Some(previous.id.clone()),
            event.related_memory_id.clone(),
            format!("Rolled back memory ledger event {}", id),
            serde_json::json!({ "rolled_back_event_id": id.clone() }),
            MemoryEventContext::from_memory(&previous),
        );
        let marked = storage
            .rollback_recall_event_with_memory_snapshot(&id, &previous, &rollback_event)
            .await?;
        if !marked {
            anyhow::bail!("Memory ledger event was already rolled back.");
        }
        Ok::<String, anyhow::Error>(previous.id)
    }
    .await;
    match result {
        Ok(memory_id) => (
            StatusCode::OK,
            Json(serde_json::json!({ "rolled_back": true, "memory_id": memory_id })),
        )
            .into_response(),
        Err(error) => (
            StatusCode::BAD_REQUEST,
            Json(ErrorResponse {
                error: error.to_string(),
            }),
        )
            .into_response(),
    }
}

pub(super) async fn arkmemory_health(
    State(state): State<AppState>,
    axum::extract::Query(params): axum::extract::Query<HashMap<String, String>>,
) -> Response {
    let project_id = arkmemory_project_param(&params);
    let limit = arkmemory_limit(&params, 80);
    let agent = state.agent.read().await;
    match arkmemory_build_health_findings(&agent.storage, project_id, limit).await {
        Ok(findings) => (
            StatusCode::OK,
            Json(serde_json::json!({ "findings": findings })),
        )
            .into_response(),
        Err(error) => (
            StatusCode::INTERNAL_SERVER_ERROR,
            Json(ErrorResponse {
                error: error.to_string(),
            }),
        )
            .into_response(),
    }
}

pub(super) async fn arkmemory_apply_health(
    State(state): State<AppState>,
    Path(id): Path<String>,
    axum::extract::Query(params): axum::extract::Query<HashMap<String, String>>,
    payload: Option<Json<MemoryHealthReviewRequest>>,
) -> Response {
    let project_id = arkmemory_project_param(&params);
    let agent = state.agent.read().await;
    let result = async {
        let active_findings =
            arkmemory_build_health_findings(&agent.storage, project_id, 200).await?;
        let finding = active_findings
            .iter()
            .find(|finding| finding.get("id").and_then(|value| value.as_str()) == Some(id.as_str()))
            .ok_or_else(|| anyhow::anyhow!("Memory health finding is no longer active."))?;
        let capture_event_id = finding
            .get("capture_event_id")
            .and_then(|value| value.as_str())
            .map(str::trim)
            .filter(|value| !value.is_empty())
            .map(str::to_string);
        let memory_id = finding
            .get("memory_id")
            .and_then(|value| value.as_str())
            .map(str::trim)
            .filter(|value| !value.is_empty())
            .map(str::to_string);
        let operation_id = finding
            .get("operation_id")
            .and_then(|value| value.as_str())
            .map(str::trim)
            .filter(|value| !value.is_empty())
            .map(str::to_string);
        let mut operation_context = None;
        let operation_resolution = if let Some(operation_id) = operation_id.as_deref() {
            let mut operation = agent
                .storage
                .get_memory_operation(operation_id)
                .await?
                .ok_or_else(|| anyhow::anyhow!("Memory operation not found."))?;
            if let Some(pid) = project_id.map(str::trim).filter(|value| !value.is_empty()) {
                if operation
                    .project_id
                    .as_deref()
                    .map(str::trim)
                    .filter(|value| !value.is_empty())
                    .is_some_and(|operation_project| operation_project != pid)
                {
                    anyhow::bail!("Memory operation is outside the active memory scope.");
                }
            } else if operation.project_id.is_some() {
                anyhow::bail!("Memory operation is outside the active memory scope.");
            }
            operation_context = Some(MemoryEventContext::from_operation(&operation));
            let previous_status = operation.status.trim().to_string();
            let now = chrono::Utc::now().to_rfc3339();
            operation.status = "reviewed_ignored".to_string();
            operation.reviewed_at = Some(now.clone());
            operation.review_notes = Some("Dismissed from Memory health.".to_string());
            operation.updated_at = now.clone();
            agent.storage.upsert_memory_operation(&operation).await?;

            let candidate_id = format!("memory-candidate-{}", operation_id);
            let mut candidate_resolution = None;
            for status in ["draft", "applying"] {
                let updated = agent
                    .storage
                    .update_learning_candidate_review_if_status(
                        &candidate_id,
                        status,
                        "rejected",
                        Some("Dismissed from Memory health."),
                        None,
                    )
                    .await?;
                if updated {
                    candidate_resolution = Some(serde_json::json!({
                        "candidate_id": candidate_id,
                        "previous_status": status,
                        "status": "rejected",
                    }));
                    break;
                }
            }

            Some(serde_json::json!({
                "operation_id": operation_id,
                "previous_status": previous_status,
                "status": "reviewed_ignored",
                "candidate_resolution": candidate_resolution,
            }))
        } else {
            None
        };
        let context = if let Some(memory_id) = memory_id.as_deref() {
            match agent.storage.get_experience_item(memory_id).await? {
                Some(item) => {
                    if !arkmemory_item_is_memory(&item)
                        || !arkmemory_item_visible_for_project(&item, project_id)
                    {
                        anyhow::bail!("Memory health finding is outside the active memory scope.");
                    }
                    MemoryEventContext::from_memory(&item)
                }
                None => MemoryEventContext::default(),
            }
        } else {
            operation_context.unwrap_or_default()
        };
        let capture_resolution = if let Some(capture_event_id) = capture_event_id.as_deref() {
            let mut event = agent
                .storage
                .get_memory_capture_event(capture_event_id)
                .await?
                .ok_or_else(|| anyhow::anyhow!("Memory capture event not found."))?;
            if !arkmemory_capture_event_visible_for_project(&event, project_id) {
                anyhow::bail!("Memory capture event is outside the active memory scope.");
            }
            let review_outcome = arkmemory_normalize_capture_review_outcome(
                payload
                    .as_ref()
                    .and_then(|payload| payload.0.outcome.as_deref()),
                &event,
            );
            let previous_status = event.status.trim().to_string();
            let reviewed_status =
                arkmemory_capture_event_reviewed_status(&event, review_outcome).to_string();
            let mut final_status = previous_status.clone();
            let finding_kind = finding
                .get("kind")
                .and_then(|value| value.as_str())
                .map(str::trim)
                .unwrap_or_default();
            let is_reviewable_capture = ARKMEMORY_FAILED_CAPTURE_STATUSES
                .contains(&previous_status.as_str())
                || (finding_kind == "auto_reviewed_capture"
                    && ARKMEMORY_REVIEWED_CAPTURE_STATUSES.contains(&previous_status.as_str()));
            if is_reviewable_capture {
                let now = chrono::Utc::now().to_rfc3339();
                let failure_signature = arkmemory_capture_review_failure_signature(&event);
                let review_context = arkmemory_capture_review_context(&agent.storage, &event)
                    .await
                    .map(|context| arkmemory_capture_review_context_json(&context));
                let previous_metadata = std::mem::take(&mut event.attempt_metadata);
                let mut metadata = match previous_metadata {
                    serde_json::Value::Object(map) => map,
                    serde_json::Value::Null => serde_json::Map::new(),
                    value => {
                        let mut map = serde_json::Map::new();
                        map.insert("previous_metadata".to_string(), value);
                        map
                    }
                };
                let previous_user_review = metadata.get("user_review").cloned();
                let previous_learned_review = metadata.get("learned_review").cloned();
                metadata.insert(
                    "reviewed_at".to_string(),
                    serde_json::Value::String(now.clone()),
                );
                metadata.insert(
                    "reviewed_from".to_string(),
                    serde_json::Value::String("arkmemory_health".to_string()),
                );
                metadata.insert(
                    "previous_status".to_string(),
                    serde_json::Value::String(previous_status.clone()),
                );
                if let Some(previous_user_review) = previous_user_review {
                    metadata.insert("previous_user_review".to_string(), previous_user_review);
                }
                if let Some(previous_learned_review) = previous_learned_review {
                    metadata.insert(
                        "superseded_learned_review".to_string(),
                        previous_learned_review,
                    );
                }
                metadata.insert(
                    "user_review".to_string(),
                    serde_json::json!({
                        "outcome": review_outcome,
                        "outcome_label": arkmemory_capture_review_outcome_label(review_outcome),
                        "reviewed_at": now.clone(),
                        "failure_signature": failure_signature,
                        "review_context": review_context,
                        "learned": false,
                        "reviewed_from": "arkmemory_health",
                        "corrected_auto_review": finding_kind == "auto_reviewed_capture",
                    }),
                );
                event.status = reviewed_status.clone();
                final_status = reviewed_status;
                event.attempt_metadata = serde_json::Value::Object(metadata);
                if event.completed_at.is_none() {
                    event.completed_at = Some(now.clone());
                }
                event.updated_at = now;
                agent.storage.upsert_memory_capture_event(&event).await?;
            }
            Some(serde_json::json!({
                "capture_event_id": capture_event_id,
                "previous_status": previous_status,
                "status": final_status,
                "outcome": review_outcome,
            }))
        } else {
            None
        };
        let project_part = project_id.unwrap_or("global");
        let event_id =
            arkmemory_stable_event_id(&["health_finding_acknowledged", project_part, id.as_str()]);
        arkmemory_record_event_once(
            &agent.storage,
            event_id,
            "health_finding_acknowledged",
            memory_id,
            None,
            format!("Acknowledged memory health finding {}", id),
            serde_json::json!({
                "finding_id": id,
                "capture_resolution": capture_resolution,
                "operation_resolution": operation_resolution,
            }),
            context,
        )
        .await
    }
    .await;
    match result {
        Ok(()) => (StatusCode::OK, Json(serde_json::json!({ "applied": true }))).into_response(),
        Err(error) => (
            StatusCode::INTERNAL_SERVER_ERROR,
            Json(ErrorResponse {
                error: error.to_string(),
            }),
        )
            .into_response(),
    }
}

pub(super) async fn arkmemory_sources(
    State(state): State<AppState>,
    Path(memory_id): Path<String>,
    axum::extract::Query(params): axum::extract::Query<HashMap<String, String>>,
) -> Response {
    let project_id = arkmemory_project_param(&params);
    let agent = state.agent.read().await;
    let storage = &agent.storage;
    let result = async {
        let memory = storage
            .get_experience_item(&memory_id)
            .await?
            .ok_or_else(|| anyhow::anyhow!("Memory item not found."))?;
        if !arkmemory_item_is_memory(&memory)
            || !arkmemory_item_visible_for_project(&memory, project_id)
        {
            anyhow::bail!("Memory item is outside the active memory scope.");
        }
        let mut edges = Vec::new();
        for edge in storage
            .list_experience_edges_for_item(&memory_id, 100)
            .await?
        {
            let related_item_id =
                if edge.source_kind == "experience_item" && edge.source_ref != memory_id {
                    Some(edge.source_ref.as_str())
                } else if edge.target_kind == "experience_item" && edge.target_ref != memory_id {
                    Some(edge.target_ref.as_str())
                } else {
                    None
                };
            let visible = match related_item_id {
                Some(related_id) => match storage.get_experience_item(related_id).await? {
                    Some(item) => {
                        arkmemory_item_is_memory(&item)
                            && arkmemory_item_visible_for_project(&item, project_id)
                    }
                    None => false,
                },
                None => true,
            };
            if visible {
                edges.push(edge);
            }
        }
        let events = storage
            .list_recall_events_for_memory(&memory_id, 100, project_id)
            .await?;
        let operations = storage
            .list_memory_operations_for_memory(&memory_id, project_id, 100)
            .await?;
        let evidence_links = storage
            .list_memory_evidence_links_for_memory(&memory_id, project_id, 100)
            .await?;
        let mut sources = arkmemory_memory_sources(&memory);
        for link in &evidence_links {
            sources.push(format!("{}:{}", link.evidence_kind, link.evidence_ref));
        }
        for operation in &operations {
            sources.extend(memory_operation_evidence_source_refs(operation));
        }
        sources.sort();
        sources.dedup();
        Ok::<serde_json::Value, anyhow::Error>(serde_json::json!({
            "memory": memory,
            "edges": edges,
            "events": events,
            "operations": operations,
            "evidence_links": evidence_links,
            "sources": sources,
        }))
    }
    .await;
    match result {
        Ok(payload) => (StatusCode::OK, Json(payload)).into_response(),
        Err(error) => (
            StatusCode::BAD_REQUEST,
            Json(ErrorResponse {
                error: error.to_string(),
            }),
        )
            .into_response(),
    }
}

pub(super) async fn arkmemory_tests(
    State(state): State<AppState>,
    axum::extract::Query(params): axum::extract::Query<HashMap<String, String>>,
) -> Response {
    let project_id = arkmemory_project_param(&params);
    let limit = arkmemory_limit(&params, 50);
    let offset = arkmemory_offset(&params);
    let agent = state.agent.read().await;
    match agent
        .storage
        .list_recall_tests(limit, offset, project_id)
        .await
    {
        Ok(tests) => {
            let total = agent
                .storage
                .count_recall_tests(project_id)
                .await
                .unwrap_or(0);
            (
                StatusCode::OK,
                Json(serde_json::json!({
                    "tests": tests,
                    "total": total,
                    "limit": limit,
                    "offset": offset,
                })),
            )
                .into_response()
        }
        Err(error) => (
            StatusCode::INTERNAL_SERVER_ERROR,
            Json(ErrorResponse {
                error: error.to_string(),
            }),
        )
            .into_response(),
    }
}

pub(super) async fn arkmemory_run_tests(
    State(state): State<AppState>,
    axum::extract::Query(params): axum::extract::Query<HashMap<String, String>>,
) -> Response {
    let project_id = arkmemory_project_param(&params);
    let agent = state.agent.read().await;
    let storage = &agent.storage;
    let result = async {
        let memories = storage
            .list_active_experience_items(&["personal_fact", "constraint"], project_id, None, 25)
            .await?;
        let now = chrono::Utc::now().to_rfc3339();
        let mut generated = 0usize;
        for memory in memories {
            let test_id = format!("recall-test-{}", memory.id);
            let test = crate::storage::recall_test::Model {
                id: test_id.clone(),
                memory_id: Some(memory.id.clone()),
                scope: memory.scope.clone(),
                project_id: None,
                conversation_id: memory.conversation_id.clone(),
                prompt: "Return the current value of this stored memory.".to_string(),
                expected_answer: memory.content.clone(),
                status: "pending".to_string(),
                last_answer: None,
                last_run_at: Some(now.clone()),
                metadata: serde_json::json!({ "generated_by": "arkmemory", "memory_kind": memory.kind }),
                created_at: now.clone(),
                updated_at: now.clone(),
            };
            storage.upsert_recall_test(&test).await?;
            generated += 1;
        }
        arkmemory_record_event(
            storage,
            "recall_tests_refreshed",
            None,
            None,
            format!("Refreshed {} memory checks", generated),
            serde_json::json!({ "generated_or_refreshed": generated }),
            MemoryEventContext {
                project_id: None,
                ..MemoryEventContext::default()
            },
        )
        .await?;
        Ok::<usize, anyhow::Error>(generated)
    }
    .await;
    match result {
        Ok(generated) => (
            StatusCode::OK,
            Json(
                serde_json::json!({ "refreshed": generated, "generated_or_refreshed": generated }),
            ),
        )
            .into_response(),
        Err(error) => (
            StatusCode::INTERNAL_SERVER_ERROR,
            Json(ErrorResponse {
                error: error.to_string(),
            }),
        )
            .into_response(),
    }
}

fn arkmemory_memory_metadata_datetime_field(
    item: &crate::storage::experience_item::Model,
    field: &str,
) -> Option<chrono::DateTime<chrono::Utc>> {
    item.metadata
        .get(field)
        .and_then(|value| value.as_str())
        .map(str::trim)
        .filter(|value| !value.is_empty())
        .and_then(|value| chrono::DateTime::parse_from_rfc3339(value).ok())
        .map(|value| value.with_timezone(&chrono::Utc))
}

fn arkmemory_memory_durability(item: &crate::storage::experience_item::Model) -> Option<String> {
    item.metadata
        .get("durability")
        .and_then(|value| value.as_str())
        .map(str::trim)
        .filter(|value| !value.is_empty())
        .map(|value| value.to_ascii_lowercase())
}

fn arkmemory_expired_memory_cleanup_item(
    item: &crate::storage::experience_item::Model,
    now: chrono::DateTime<chrono::Utc>,
) -> Option<serde_json::Value> {
    if item.status != "active" || !arkmemory_item_is_memory(item) {
        return None;
    }
    if arkmemory_memory_durability(item).as_deref() != Some("temporary") {
        return None;
    }
    let expires_at = arkmemory_memory_metadata_datetime_field(item, "expires_at")?;
    if expires_at > now {
        return None;
    }
    let memory_kind = item
        .metadata
        .get("memory_kind")
        .and_then(|value| value.as_str())
        .map(str::trim)
        .filter(|value| !value.is_empty());
    let memory_category =
        crate::core::memory_schema::memory_category_from_metadata(&item.metadata, memory_kind);
    Some(serde_json::json!({
        "id": format!("expired-memory:{}", item.id),
        "kind": "expired_memory",
        "action": "expire_memory",
        "title": format!(
            "Expired {}",
            crate::core::memory_schema::memory_category_label(memory_category)
                .to_ascii_lowercase()
        ),
        "detail": "This memory is past its structured expires_at lifecycle metadata and can be marked expired.",
        "memory_id": item.id,
        "memory_category": memory_category,
        "memory_kind": memory_kind,
        "durability": "temporary",
        "scope": item.scope,
        "conversation_id": item.conversation_id,
        "expires_at": expires_at.to_rfc3339(),
        "created_at": item.updated_at,
        "preview": arkmemory_truncate_chars(&item.content, 180),
    }))
}

async fn arkmemory_expired_memory_cleanup_items(
    storage: &crate::storage::Storage,
    project_id: Option<&str>,
    limit: u64,
) -> Result<Vec<serde_json::Value>> {
    let now = chrono::Utc::now();
    let items = storage
        .list_active_experience_items_any_scope(
            &["personal_fact", "constraint"],
            ARKMEMORY_CLEANUP_MEMORY_SCAN_LIMIT,
        )
        .await?;
    Ok(items
        .iter()
        .filter(|item| arkmemory_item_visible_for_project(item, project_id))
        .filter_map(|item| arkmemory_expired_memory_cleanup_item(item, now))
        .take(limit as usize)
        .collect())
}

async fn arkmemory_expire_stale_memory_items(
    storage: &crate::storage::Storage,
    project_id: Option<&str>,
) -> Result<usize> {
    let now = chrono::Utc::now();
    let items = storage
        .list_active_experience_items_any_scope(
            &["personal_fact", "constraint"],
            ARKMEMORY_CLEANUP_MEMORY_SCAN_LIMIT,
        )
        .await?;
    let mut expired = 0usize;
    for item in items {
        if !arkmemory_item_visible_for_project(&item, project_id) {
            continue;
        }
        let Some(candidate) = arkmemory_expired_memory_cleanup_item(&item, now) else {
            continue;
        };
        storage
            .update_experience_item_status(&item.id, "expired")
            .await?;
        let expires_at = candidate
            .get("expires_at")
            .and_then(|value| value.as_str())
            .unwrap_or_default();
        let category = candidate
            .get("memory_category")
            .and_then(|value| value.as_str())
            .unwrap_or(crate::core::memory_schema::MEMORY_CATEGORY_OTHER);
        arkmemory_record_event_once(
            storage,
            arkmemory_stable_event_id(&["expired_memory_cleanup_applied", &item.id, expires_at]),
            "expired_memory_cleanup_applied",
            Some(item.id.clone()),
            None,
            format!("Expired stale memory {}", item.id),
            serde_json::json!({
                "action": "expire_memory",
                "expires_at": expires_at,
                "memory_category": category,
                "durability": "temporary",
            }),
            MemoryEventContext::from_memory(&item),
        )
        .await?;
        expired += 1;
    }
    Ok(expired)
}

pub(super) async fn arkmemory_cleanup(
    State(state): State<AppState>,
    axum::extract::Query(params): axum::extract::Query<HashMap<String, String>>,
) -> Response {
    let project_id = arkmemory_project_param(&params);
    let limit = arkmemory_limit(&params, 80);
    let agent = state.agent.read().await;
    let result = async {
        let mut items = arkmemory_expired_memory_cleanup_items(&agent.storage, project_id, limit)
            .await?;
        let remaining = limit.saturating_sub(items.len() as u64);
        if remaining == 0 {
            return Ok::<Vec<serde_json::Value>, anyhow::Error>(items);
        }
        let events = agent
            .storage
            .list_reverted_recall_events(remaining, project_id)
            .await?
            .into_iter()
            .map(|event| {
                let title = event
                    .summary
                    .clone()
                    .unwrap_or_else(|| event.event_type.clone());
                serde_json::json!({
                    "id": format!("reverted-event:{}", event.id),
                    "kind": "reverted_ledger_event",
                    "title": title,
                    "detail": "This ledger event has already been rolled back and can age out through retention.",
                    "created_at": event.created_at,
                    "memory_id": event.memory_id,
                })
            })
            .collect::<Vec<_>>();
        items.extend(events);
        Ok::<Vec<serde_json::Value>, anyhow::Error>(items)
    }
    .await;
    match result {
        Ok(items) => (StatusCode::OK, Json(serde_json::json!({ "items": items }))).into_response(),
        Err(error) => (
            StatusCode::INTERNAL_SERVER_ERROR,
            Json(ErrorResponse {
                error: error.to_string(),
            }),
        )
            .into_response(),
    }
}

pub(super) async fn arkmemory_apply_cleanup(
    State(state): State<AppState>,
    axum::extract::Query(params): axum::extract::Query<HashMap<String, String>>,
) -> Response {
    let project_id = arkmemory_project_param(&params);
    let agent = state.agent.read().await;
    let project_part = project_id.unwrap_or("global");
    let event_id = arkmemory_stable_event_id(&["cleanup_review_acknowledged", project_part]);
    let result = async {
        let expired_memories =
            arkmemory_expire_stale_memory_items(&agent.storage, project_id).await?;
        arkmemory_record_event_once(
            &agent.storage,
            event_id,
            "cleanup_review_acknowledged",
            None,
            None,
            "Acknowledged Memory cleanup review",
            serde_json::json!({
                "cleanup": "retention_managed",
                "expired_memories": expired_memories,
            }),
            MemoryEventContext {
                project_id: None,
                ..MemoryEventContext::default()
            },
        )
        .await?;
        Ok::<usize, anyhow::Error>(expired_memories)
    }
    .await;
    match result {
        Ok(expired_memories) => (
            StatusCode::OK,
            Json(serde_json::json!({
                "applied": true,
                "retention_managed": true,
                "expired_memories": expired_memories,
            })),
        )
            .into_response(),
        Err(error) => (
            StatusCode::INTERNAL_SERVER_ERROR,
            Json(ErrorResponse {
                error: error.to_string(),
            }),
        )
            .into_response(),
    }
}
