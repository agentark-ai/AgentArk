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
    axum::extract::Query(params): axum::extract::Query<std::collections::HashMap<String, String>>,
) -> Response {
    let project_id = params.get("project_id").map(|s| s.as_str());
    let agent = state.agent.read().await;
    let fact_count = agent.storage.count_facts(project_id).await.unwrap_or(0);
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
    let project_id = params.get("project_id").map(|s| s.as_str());
    let limit = params
        .get("limit")
        .and_then(|s| s.parse().ok())
        .unwrap_or(20u64);
    let offset = params
        .get("offset")
        .and_then(|s| s.parse().ok())
        .unwrap_or(0u64);
    let agent = state.agent.read().await;
    let total = agent.storage.count_facts(project_id).await.unwrap_or(0);
    match agent
        .encrypted_storage
        .get_facts_by_project_decrypted(limit, offset, project_id)
        .await
    {
        Ok(facts) => {
            let items: Vec<serde_json::Value> = facts
                .iter()
                .map(|f| {
                    serde_json::json!({
                        "id": f.id,
                        "fact": f.fact,
                        "confidence": f.confidence,
                        "sources": f.sources,
                        "created_at": f.created_at,
                    })
                })
                .collect();
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

#[derive(Debug, Deserialize)]
pub(super) struct UpsertUserPreferenceRequest {
    key: String,
    value: String,
    sensitivity: Option<String>,
    confidence: Option<f32>,
    source: Option<String>,
    project_id: Option<String>,
}

#[derive(Debug, Deserialize)]
pub(super) struct CreateUserDataItemRequest {
    kind: String,
    title: String,
    content: String,
    url: Option<String>,
    source_channel: Option<String>,
    conversation_id: Option<String>,
    project_id: Option<String>,
    pinned: Option<bool>,
}

#[derive(Debug, Deserialize)]
pub(super) struct CreateKnowledgeItemRequest {
    title: String,
    content: String,
    source: Option<String>,
    url: Option<String>,
    tags: Option<String>,
    project_id: Option<String>,
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
    let project_id = params.get("project_id").map(|s| s.as_str());

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
            payload
                .project_id
                .as_deref()
                .map(str::trim)
                .filter(|v| !v.is_empty()),
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
    axum::extract::Query(params): axum::extract::Query<std::collections::HashMap<String, String>>,
) -> Response {
    let project_id = params
        .get("project_id")
        .map(String::as_str)
        .map(str::trim)
        .filter(|v| !v.is_empty());
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
    let project_id = params
        .get("project_id")
        .map(String::as_str)
        .map(str::trim)
        .filter(|v| !v.is_empty());
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
            project_id: payload
                .project_id
                .as_deref()
                .map(str::trim)
                .filter(|v| !v.is_empty()),
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
    let project_id = params
        .get("project_id")
        .map(String::as_str)
        .map(str::trim)
        .filter(|v| !v.is_empty());

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
            payload
                .project_id
                .as_deref()
                .map(str::trim)
                .filter(|v| !v.is_empty()),
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
pub(super) const ARKMEMORY_APPLYING_LEASE_TIMEOUT_SECS: i64 = 10 * 60;

pub(super) fn arkmemory_project_param(params: &HashMap<String, String>) -> Option<&str> {
    params
        .get("project_id")
        .map(String::as_str)
        .map(str::trim)
        .filter(|value| !value.is_empty())
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
    project_id: Option<&str>,
) -> bool {
    match project_id.map(str::trim).filter(|value| !value.is_empty()) {
        Some(pid) => item.project_id.as_deref() == Some(pid) || item.project_id.is_none(),
        None => item.project_id.is_none(),
    }
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
            project_id: item.project_id.clone(),
            conversation_id: item.conversation_id.clone(),
            source_ref: Some(item.id.clone()),
        }
    }

    fn from_candidate(candidate: &crate::storage::learning_candidate::Model) -> Self {
        Self {
            scope: None,
            project_id: candidate.project_id.clone(),
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
            candidate.project_id.as_deref(),
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
        "project_id": candidate.project_id,
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
    let memory_items = storage
        .list_active_experience_items(&["personal_fact", "constraint"], project_id, None, limit)
        .await?;
    let mut findings = Vec::new();
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
        if arkmemory_memory_sources(&item).is_empty() && evidence_links.is_empty() {
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
        for event in storage
            .list_memory_capture_events_by_statuses(&["failed"], project_id, limit)
            .await?
        {
            let capture_event_id = event.id.clone();
            findings.push(serde_json::json!({
                "id": format!("capture_failed:{}", capture_event_id),
                "kind": "capture_failed",
                "severity": "warning",
                "capture_event_id": capture_event_id,
                "title": "Memory capture failed",
                "detail": "A user-memory capture event failed before it could produce an auditable operation.",
                "action": "review_capture_pipeline",
                "created_at": event.updated_at,
            }));
            if findings.len() >= limit as usize {
                break;
            }
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
    match project_id.map(str::trim).filter(|value| !value.is_empty()) {
        Some(pid) => {
            if candidate.project_id.as_deref() != Some(pid) && candidate.project_id.is_some() {
                anyhow::bail!("Memory queue item is outside the active project scope.");
            }
        }
        None => {
            if candidate.project_id.is_some() {
                anyhow::bail!("Memory queue item is outside the global scope.");
            }
        }
    }
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
            if !arkmemory_item_visible_for_project(&item, candidate.project_id.as_deref()) {
                anyhow::bail!("Memory queue item is outside its project scope.");
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
            if !arkmemory_item_visible_for_project(&target_item, candidate.project_id.as_deref())
                || !arkmemory_item_visible_for_project(
                    &source_item,
                    candidate.project_id.as_deref(),
                )
            {
                anyhow::bail!("Memory merge is outside its project scope.");
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
    let facts = storage.count_facts(project_id).await.unwrap_or(0);
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
                "preferences": preferences,
                "user_data": user_data,
                "knowledge": knowledge,
            },
            "queue": queue,
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
    let project_id = arkmemory_project_param(&params);
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
        match project_id.map(str::trim).filter(|value| !value.is_empty()) {
            Some(pid) => {
                if candidate.project_id.as_deref() != Some(pid) && candidate.project_id.is_some() {
                    anyhow::bail!("Memory queue item is outside the active project scope.");
                }
            }
            None => {
                if candidate.project_id.is_some() {
                    anyhow::bail!("Memory queue item is outside the global scope.");
                }
            }
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
        match project_id.map(str::trim).filter(|value| !value.is_empty()) {
            Some(pid) => {
                if event.project_id.as_deref() != Some(pid) && event.project_id.is_some() {
                    anyhow::bail!("Memory ledger event is outside the active project scope.");
                }
            }
            None => {
                if event.project_id.is_some() {
                    anyhow::bail!("Memory ledger event is outside the global scope.");
                }
            }
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
            serde_json::json!({ "finding_id": id }),
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
                project_id: memory.project_id.clone(),
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
                project_id: project_id.map(|value| value.to_string()),
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
            project_id: project_id.map(|value| value.to_string()),
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
