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
        extension_packs: &*packs_guard,
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
                error: "item is not a deletable ArkMemory learned memory".to_string(),
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

// ==================== ArkMemory Endpoints ====================

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

#[derive(Debug, Deserialize)]
pub(super) struct ArkMemoryHealthReviewRequest {
    outcome: Option<String>,
}

#[derive(Clone, Debug, Default)]
struct ArkMemoryCaptureReviewPattern {
    similar_review_count: usize,
    expected_sensitive_skip_count: usize,
    false_positive_safe_memory_count: usize,
    acknowledged_count: usize,
}

pub(super) fn arkmemory_project_param(params: &HashMap<String, String>) -> Option<&str> {
    let _ = params;
    None
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
    let status = event.status.trim();
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

fn arkmemory_capture_event_timestamp(
    raw: &str,
) -> Option<chrono::DateTime<chrono::Utc>> {
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
        payloads
            .push(arkmemory_pending_capture_group_payload(storage, semantic_key, events).await);
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
) -> std::collections::HashMap<String, ArkMemoryCaptureReviewPattern> {
    let mut patterns = std::collections::HashMap::<String, ArkMemoryCaptureReviewPattern>::new();
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
    pattern: Option<&ArkMemoryCaptureReviewPattern>,
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

fn arkmemory_capture_event_finding(
    event: crate::storage::memory_capture_event::Model,
    review_pattern: Option<&ArkMemoryCaptureReviewPattern>,
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

#[derive(Default)]
pub(super) struct ArkMemoryEventContext {
    scope: Option<String>,
    project_id: Option<String>,
    conversation_id: Option<String>,
    source_ref: Option<String>,
}

impl ArkMemoryEventContext {
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
                    Some("Reset stale ArkMemory apply claim."),
                    None,
                )
                .await?;
            if reset {
                candidate.approval_status = "draft".to_string();
                candidate.review_notes = Some("Reset stale ArkMemory apply claim.".to_string());
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
                    Some("Reset stale ArkMemory apply claim."),
                    None,
                )
                .await?;
            if reset {
                row.approval_status = "draft".to_string();
                row.review_notes = Some("Reset stale ArkMemory apply claim.".to_string());
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
        let source_context = arkmemory_capture_source_context(storage, &event).await;
        findings.push(arkmemory_capture_event_finding(
            event,
            reviewed_capture_patterns.get(&pattern_key),
            source_context,
        ));
        if findings.len() >= limit as usize {
            return Ok(findings);
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
            .list_memory_operations_by_statuses(
                &["queued_review", "apply_failed"],
                project_id,
                limit,
            )
            .await?
        {
            let operation_id = operation.id.clone();
            let operation_status = operation.status.clone();
            let operation_type = operation.operation_type.clone();
            let memory_id = operation
                .applied_memory_id
                .clone()
                .or_else(|| operation.target_memory_id.clone());
            findings.push(serde_json::json!({
                "id": format!("operation:{}", operation_id),
                "kind": operation_status.clone(),
                "severity": if operation_status == "apply_failed" { "warning" } else { "review" },
                "memory_id": memory_id,
                "operation_id": operation_id,
                "title": format!("Memory operation {}", operation_type),
                "detail": operation
                    .review_notes
                    .clone()
                    .unwrap_or_else(|| "This staged memory operation still needs ArkMemory review.".to_string()),
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
    context: ArkMemoryEventContext,
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
    context: ArkMemoryEventContext,
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
    context: ArkMemoryEventContext,
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
    context: ArkMemoryEventContext,
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
            Some("Applying from ArkMemory."),
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
                    Some("Approved from ArkMemory."),
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
                ArkMemoryEventContext::from_candidate(candidate),
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
                ArkMemoryEventContext::from_memory(&item),
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
                ArkMemoryEventContext::from_memory(&target_item),
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
    let pending_capture_signals = arkmemory_pending_capture_signal_payloads(storage, project_id, 200)
        .await
        .unwrap_or_default();
    let pending_capture = pending_capture_signals.len();
    let failed_capture = storage
        .list_memory_capture_events_by_statuses(ARKMEMORY_FAILED_CAPTURE_STATUSES, project_id, 200)
        .await
        .map(|items| items.len())
        .unwrap_or(0);
    let ledger = storage.count_recall_events(project_id).await.unwrap_or(0);
    let tests = storage.count_recall_tests(project_id).await.unwrap_or(0);
    let health = arkmemory_build_health_findings(storage, project_id, 200)
        .await
        .map(|items| items.len())
        .unwrap_or(0);
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
                                "Failed to evaluate ArkMemory replay gate for candidate '{}': {}",
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
                Some("Rejected from ArkMemory."),
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
                operation.review_notes = Some("Rejected from ArkMemory.".to_string());
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
            ArkMemoryEventContext::from_candidate(&candidate),
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
            let total = agent
                .storage
                .count_recall_events(project_id)
                .await
                .unwrap_or(0);
            (
                StatusCode::OK,
                Json(serde_json::json!({
                    "events": events,
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
            ArkMemoryEventContext::from_memory(&previous),
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
    payload: Option<Json<ArkMemoryHealthReviewRequest>>,
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
        let context = if let Some(memory_id) = memory_id.as_deref() {
            match agent.storage.get_experience_item(memory_id).await? {
                Some(item) => {
                    if !arkmemory_item_is_memory(&item)
                        || !arkmemory_item_visible_for_project(&item, project_id)
                    {
                        anyhow::bail!("Memory health finding is outside the active memory scope.");
                    }
                    ArkMemoryEventContext::from_memory(&item)
                }
                None => ArkMemoryEventContext::default(),
            }
        } else {
            ArkMemoryEventContext::default()
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
            if ARKMEMORY_FAILED_CAPTURE_STATUSES.contains(&previous_status.as_str()) {
                let now = chrono::Utc::now().to_rfc3339();
                let failure_signature = arkmemory_capture_failure_signature(&event);
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
                    serde_json::Value::String("arkmemory_health".to_string()),
                );
                metadata.insert(
                    "previous_status".to_string(),
                    serde_json::Value::String(previous_status.clone()),
                );
                metadata.insert(
                    "user_review".to_string(),
                    serde_json::json!({
                        "outcome": review_outcome,
                        "outcome_label": arkmemory_capture_review_outcome_label(review_outcome),
                        "reviewed_at": now.clone(),
                        "failure_signature": failure_signature,
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
            ArkMemoryEventContext {
                project_id: None,
                ..ArkMemoryEventContext::default()
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

pub(super) async fn arkmemory_cleanup(
    State(state): State<AppState>,
    axum::extract::Query(params): axum::extract::Query<HashMap<String, String>>,
) -> Response {
    let project_id = arkmemory_project_param(&params);
    let limit = arkmemory_limit(&params, 80);
    let agent = state.agent.read().await;
    let result = async {
        let events = agent
            .storage
            .list_reverted_recall_events(limit, project_id)
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
        Ok::<Vec<serde_json::Value>, anyhow::Error>(events)
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
    let result = arkmemory_record_event_once(
        &agent.storage,
        event_id,
        "cleanup_review_acknowledged",
        None,
        None,
        "Acknowledged ArkMemory cleanup review",
        serde_json::json!({ "cleanup": "retention_managed" }),
        ArkMemoryEventContext {
            project_id: None,
            ..ArkMemoryEventContext::default()
        },
    )
    .await;
    match result {
        Ok(()) => (
            StatusCode::OK,
            Json(serde_json::json!({ "applied": true, "retention_managed": true })),
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
