use super::*;

// ==================== Swarm API ====================
pub(super) fn parse_swarm_delegation_result(
    raw: Option<&str>,
) -> std::collections::HashMap<String, serde_json::Value> {
    serde_json::from_str::<serde_json::Value>(raw.unwrap_or_default())
        .ok()
        .and_then(|value| value.as_object().cloned())
        .map(|map| map.into_iter().collect())
        .unwrap_or_default()
}

pub(super) fn swarm_result_string(
    payload: &std::collections::HashMap<String, serde_json::Value>,
    key: &str,
) -> Option<String> {
    payload
        .get(key)
        .and_then(|value| value.as_str())
        .map(|value| value.trim().to_string())
        .filter(|value| !value.is_empty())
}

pub(super) fn build_swarm_agent_from_delegation(
    row: &crate::storage::entities::swarm_delegation::Model,
) -> crate::core::swarm::SwarmActivityAgent {
    let payload = parse_swarm_delegation_result(row.result.as_deref());
    let status = swarm_result_string(&payload, "status").unwrap_or_else(|| {
        if row.completed_at.is_none() {
            "running".to_string()
        } else if row.success == 1 {
            "completed".to_string()
        } else {
            "failed".to_string()
        }
    });
    let summary = swarm_result_string(&payload, "content")
        .or_else(|| swarm_result_string(&payload, "latest_update"))
        .or_else(|| swarm_result_string(&payload, "summary"))
        .unwrap_or_else(|| truncate_stream_task_text(&row.task_description, 180));
    let latest_update = swarm_result_string(&payload, "latest_update")
        .or_else(|| swarm_result_string(&payload, "content"))
        .or_else(|| swarm_result_string(&payload, "summary"))
        .unwrap_or_else(|| summary.clone());
    crate::core::swarm::SwarmActivityAgent {
        id: row.agent_id.clone(),
        agent_name: swarm_result_string(&payload, "agent_name")
            .unwrap_or_else(|| row.agent_id.clone()),
        agent_role: swarm_result_string(&payload, "agent_role")
            .unwrap_or_else(|| "Agent".to_string()),
        model_name: swarm_result_string(&payload, "model_name").unwrap_or_else(|| "-".to_string()),
        task: row.task_description.clone(),
        status,
        summary,
        latest_update,
        is_specialist: payload
            .get("is_specialist")
            .and_then(|value| value.as_bool())
            .unwrap_or(false),
        depends_on: payload
            .get("depends_on")
            .and_then(|value| value.as_array())
            .map(|items| {
                items
                    .iter()
                    .filter_map(|value| value.as_u64().map(|idx| idx as usize))
                    .collect::<Vec<_>>()
            })
            .unwrap_or_default(),
        started_at: Some(row.created_at.clone()),
        completed_at: row.completed_at.clone(),
        updated_at: swarm_result_string(&payload, "updated_at").unwrap_or_else(|| {
            row.completed_at
                .clone()
                .unwrap_or_else(|| row.created_at.clone())
        }),
        elapsed_ms: payload
            .get("elapsed_ms")
            .and_then(|value| value.as_u64())
            .or_else(|| row.execution_time_ms.map(|value| value.max(0) as u64)),
    }
}

pub(super) fn is_system_swarm_agent(agent: &crate::storage::entities::swarm_agent::Model) -> bool {
    agent.id.starts_with("default-")
        || agent.id.contains("::")
        || agent.id.contains(":agent:")
        || agent.id.contains(":plan-step:")
}

fn swarm_delegation_run_id(row: &crate::storage::entities::swarm_delegation::Model) -> String {
    row.parent_task_id
        .clone()
        .or_else(|| {
            row.id
                .split_once("::")
                .map(|(run_id, _)| run_id.to_string())
        })
        .unwrap_or_else(|| row.id.clone())
}

pub(super) fn summarize_swarm_run_status(
    agents: &[crate::core::swarm::SwarmActivityAgent],
) -> String {
    if agents.iter().any(|agent| {
        matches!(
            agent.status.as_str(),
            "assigned" | "running" | "synthesizing"
        )
    }) {
        return "running".to_string();
    }
    if agents.iter().all(|agent| agent.status == "completed") {
        return "completed".to_string();
    }
    if agents
        .iter()
        .any(|agent| matches!(agent.status.as_str(), "interrupted" | "cancelled"))
    {
        return "interrupted".to_string();
    }
    if agents.iter().any(|agent| agent.status == "completed") {
        return "partial".to_string();
    }
    if agents.iter().any(|agent| agent.status == "timed_out") {
        return "timed_out".to_string();
    }
    if agents.iter().any(|agent| agent.status == "panicked") {
        return "panicked".to_string();
    }
    "failed".to_string()
}

pub(super) fn summarize_swarm_run_text(
    status: &str,
    completed_count: usize,
    total_count: usize,
) -> String {
    match status {
        "running" => format!(
            "{} of {} delegated agents active.",
            completed_count, total_count
        ),
        "completed" => format!("All {} delegated agents completed.", total_count),
        "partial" => format!(
            "{} of {} delegated agents completed; follow-up is still needed.",
            completed_count, total_count
        ),
        "interrupted" => "Delegated run was stopped before completion.".to_string(),
        "timed_out" => "Delegated run timed out.".to_string(),
        "panicked" => "Delegated run hit an internal failure.".to_string(),
        _ => "Delegated run failed.".to_string(),
    }
}

pub(super) fn build_swarm_run_from_rows(
    run_id: &str,
    rows: &[crate::storage::entities::swarm_delegation::Model],
) -> crate::core::swarm::SwarmActivityRun {
    let mut agents = rows
        .iter()
        .map(build_swarm_agent_from_delegation)
        .collect::<Vec<_>>();
    agents.sort_by(|left, right| right.updated_at.cmp(&left.updated_at));
    let first_payload = rows
        .first()
        .map(|row| parse_swarm_delegation_result(row.result.as_deref()))
        .unwrap_or_default();
    let started_at = rows
        .iter()
        .map(|row| row.created_at.clone())
        .min()
        .unwrap_or_else(|| chrono::Utc::now().to_rfc3339());
    let updated_at = rows
        .iter()
        .map(|row| {
            row.completed_at
                .clone()
                .unwrap_or_else(|| row.created_at.clone())
        })
        .max()
        .unwrap_or_else(|| started_at.clone());
    let completed_at = if rows.iter().all(|row| row.completed_at.is_some()) {
        rows.iter().filter_map(|row| row.completed_at.clone()).max()
    } else {
        None
    };
    let status = summarize_swarm_run_status(&agents);
    let completed_count = agents
        .iter()
        .filter(|agent| agent.status == "completed")
        .count();
    crate::core::swarm::SwarmActivityRun {
        id: run_id.to_string(),
        conversation_id: swarm_result_string(&first_payload, "conversation_id"),
        channel: swarm_result_string(&first_payload, "channel"),
        request: swarm_result_string(&first_payload, "request").unwrap_or_else(|| {
            rows.first()
                .map(|row| truncate_stream_task_text(&row.task_description, 220))
                .unwrap_or_else(|| "Delegated run".to_string())
        }),
        status: status.clone(),
        summary: summarize_swarm_run_text(&status, completed_count, agents.len()),
        started_at,
        updated_at,
        completed_at,
        agent_count: agents.len(),
        agents,
    }
}

pub(super) fn sort_swarm_runs(runs: &mut [crate::core::swarm::SwarmActivityRun]) {
    runs.sort_by(|left, right| right.updated_at.cmp(&left.updated_at));
}

/// Swarm status overview
pub(super) async fn swarm_status(State(state): State<AppState>) -> Response {
    let agent = state.agent.read().await;
    let mut active_runs = agent.swarm_activity.active_runs().await;
    let mut persisted_grouped: std::collections::HashMap<
        String,
        Vec<crate::storage::entities::swarm_delegation::Model>,
    > = std::collections::HashMap::new();
    if let Ok(rows) = agent.storage.get_active_swarm_delegations(250).await {
        for row in rows {
            let run_id = swarm_delegation_run_id(&row);
            persisted_grouped.entry(run_id).or_default().push(row);
        }
    }
    for (run_id, rows) in persisted_grouped {
        if active_runs.iter().any(|existing| existing.id == run_id) {
            continue;
        }
        active_runs.push(build_swarm_run_from_rows(&run_id, &rows));
    }
    sort_swarm_runs(&mut active_runs);
    let tracked_active_agents = active_runs
        .iter()
        .map(|run| run.active_agent_count())
        .sum::<usize>();

    if let Some(ref swarm) = agent.swarm {
        let status = swarm.status().await;
        (
            StatusCode::OK,
            Json(serde_json::json!({
                "enabled": status.enabled,
                "total_agents": status.total_agents,
                "active_agents": status.active_agents.max(tracked_active_agents),
                "agents": status.agents,
                "active_runs": active_runs,
            })),
        )
            .into_response()
    } else {
        (
            StatusCode::OK,
            Json(serde_json::json!({
                "enabled": true,
                "total_agents": 0,
                "active_agents": tracked_active_agents,
                "agents": [],
                "active_runs": active_runs,
            })),
        )
            .into_response()
    }
}

/// List swarm agents (from DB for persistent view)
pub(super) async fn swarm_list_agents(State(state): State<AppState>) -> Response {
    let agent = state.agent.read().await;
    let live_status = if let Some(ref swarm) = agent.swarm {
        Some(swarm.status().await)
    } else {
        None
    };
    let active_runs = agent.swarm_activity.active_runs().await;
    let active_rows = agent
        .storage
        .get_active_swarm_delegations(250)
        .await
        .unwrap_or_default();
    let recent_rows = agent
        .storage
        .get_recent_delegations(250)
        .await
        .unwrap_or_default();
    let mut recent_by_agent_id: std::collections::HashMap<
        String,
        crate::core::swarm::SwarmActivityAgent,
    > = std::collections::HashMap::new();
    for run in &active_runs {
        for delegated in &run.agents {
            recent_by_agent_id.insert(delegated.id.clone(), delegated.clone());
        }
    }
    for row in &active_rows {
        recent_by_agent_id
            .entry(row.agent_id.clone())
            .or_insert_with(|| build_swarm_agent_from_delegation(row));
    }
    for row in &recent_rows {
        recent_by_agent_id
            .entry(row.agent_id.clone())
            .or_insert_with(|| build_swarm_agent_from_delegation(row));
    }

    match agent.storage.get_swarm_agents().await {
        Ok(agents) => {
            let live_by_id: std::collections::HashMap<
                String,
                crate::core::swarm::agent_trait::AgentInfo,
            > = live_status
                .as_ref()
                .map(|status| {
                    status
                        .agents
                        .iter()
                        .cloned()
                        .map(|info| (info.id.to_string(), info))
                        .collect()
                })
                .unwrap_or_default();
            let agent_infos: Vec<serde_json::Value> = agents
                .iter()
                .filter(|a| !(a.enabled == 0 && is_system_swarm_agent(a)))
                .map(|a| {
                    let agent_type = crate::core::swarm::persistence::parse_agent_type(
                        &a.agent_type,
                        a.system_prompt.as_deref(),
                    );
                    let provider = crate::core::swarm::persistence::parse_llm_provider(
                        &a.llm_provider,
                        &agent.config.llm,
                    );
                    let provider_label = match &provider {
                        LlmProvider::Anthropic { .. } => "anthropic",
                        LlmProvider::OpenAI { base_url, .. } => {
                            openai_provider_label(base_url.as_deref())
                        }
                        LlmProvider::Ollama { .. } => "ollama",
                    };
                    let llm_model = match &provider {
                        LlmProvider::Anthropic { model, .. } => model.clone(),
                        LlmProvider::OpenAI { model, .. } => model.clone(),
                        LlmProvider::Ollama { model, .. } => model.clone(),
                    };
                    let llm_base_url = match &provider {
                        LlmProvider::Anthropic { .. } => None,
                        LlmProvider::OpenAI { base_url, .. } => {
                            display_openai_base_url(base_url.as_ref())
                        }
                        LlmProvider::Ollama { base_url, .. } => Some(base_url.clone()),
                    };
                    let access_scope =
                        crate::core::swarm::persistence::parse_access_scope(Some(&a.access_scope));
                    let live = live_by_id.get(&a.id);
                    let recent_activity = recent_by_agent_id.get(&a.id);
                    let display_name =
                        crate::core::task_router::display_name_for_specialist(&a.name, &agent_type);
                    serde_json::json!({
                        "id": a.id,
                        "name": a.name,
                        "display_name": display_name,
                        "is_system": is_system_swarm_agent(a),
                        "agent_type": a.agent_type,
                        "llm_provider": provider_label,
                        "llm_model": live.map(|info| info.llm_model.clone()).unwrap_or(llm_model),
                        "llm_base_url": llm_base_url,
                        "capabilities": crate::core::swarm::persistence::parse_capabilities(&a.capabilities)
                            .into_iter()
                            .map(|cap| cap.description)
                            .collect::<Vec<_>>(),
                        "system_prompt": a.system_prompt,
                        "access_scope": access_scope,
                        "enabled": a.enabled == 1,
                        "status": live
                            .map(|info| format!("{:?}", info.status))
                            .or_else(|| recent_activity.map(|activity| activity.status.clone()))
                            .unwrap_or("Idle".to_string()),
                        "last_task": recent_activity.map(|activity| activity.task.clone()),
                        "last_summary": recent_activity.map(|activity| activity.summary.clone()),
                        "last_update": recent_activity.map(|activity| activity.latest_update.clone()),
                        "last_activity_at": recent_activity
                            .map(|activity| activity.updated_at.clone())
                            .filter(|value| !value.trim().is_empty()),
                        "created_at": a.created_at,
                    })
                })
                .collect();
            (
                StatusCode::OK,
                Json(serde_json::json!({ "agents": agent_infos })),
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

/// Add a new swarm agent request
#[derive(Debug, Deserialize)]
pub struct AddSwarmAgentRequest {
    pub name: String,
    pub agent_type: String,
    pub llm_provider: String,
    pub llm_model: String,
    pub llm_base_url: Option<String>,
    pub llm_api_key: Option<String>,
    #[serde(default)]
    pub system_prompt: Option<String>,
    #[serde(default)]
    pub capabilities: Vec<String>,
    #[serde(default)]
    pub access_scope: crate::core::swarm::AgentAccessScope,
}

#[derive(Debug, Deserialize)]
pub(super) struct SwarmAgentDraftRequest {
    pub description: String,
    #[serde(default)]
    pub model_profile_id: Option<String>,
}

#[derive(Debug, Serialize)]
pub(super) struct SwarmAgentDraftResponse {
    pub name: String,
    pub agent_type: String,
    pub capabilities: Vec<String>,
    pub system_prompt: String,
}

#[derive(Debug, Deserialize)]
pub(super) struct SwarmAgentAccessPlanRequest {
    #[serde(default)]
    pub description: String,
    #[serde(default)]
    pub name: String,
    #[serde(default)]
    pub agent_type: String,
    #[serde(default)]
    pub capabilities: Vec<String>,
    #[serde(default)]
    pub system_prompt: String,
    #[serde(default)]
    pub access_scope: crate::core::swarm::AgentAccessScope,
}

#[derive(Debug, Clone, Serialize)]
pub(super) struct SwarmAgentAccessPlanAction {
    pub name: String,
    pub reason: String,
}

#[derive(Debug, Clone, Serialize)]
pub(super) struct SwarmAgentAccessPlanDetail {
    pub action_name: String,
    pub reason: String,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub permission_ids: Vec<String>,
}

#[derive(Debug, Clone, Serialize)]
pub(super) struct SwarmAgentAccessPlanGroup {
    pub id: String,
    pub scope_field: String,
    pub label: String,
    pub summary: String,
    pub reason: String,
    pub review_band: String,
    pub selection_mode: String,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub suggested_ids: Vec<String>,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub details: Vec<SwarmAgentAccessPlanDetail>,
}

#[derive(Debug, Serialize)]
pub(super) struct SwarmAgentAccessPlanResponse {
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub implicit_access: Vec<SwarmAgentAccessPlanGroup>,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub requested_access: Vec<SwarmAgentAccessPlanGroup>,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub suggested_actions: Vec<SwarmAgentAccessPlanAction>,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub notes: Vec<String>,
}

pub(super) fn resolve_swarm_draft_client(
    agent: &Agent,
    requested_model_profile_id: Option<&str>,
) -> crate::core::LlmClient {
    if let Some(slot_index) = requested_model_profile_id
        .and_then(|value| resolve_model_slot_index(&agent.config.model_pool.slots, value))
    {
        if let Some(slot) = agent.config.model_pool.slots.get(slot_index) {
            if let Some((_, client)) = agent.model_pool.get(&slot.id) {
                return client.clone();
            }
        }
    }
    agent.llm.clone()
}

pub(super) fn build_swarm_agent_spec(
    id: Option<String>,
    request: &AddSwarmAgentRequest,
    existing_provider: Option<&LlmProvider>,
) -> std::result::Result<(LlmProvider, crate::core::swarm::SpecialistConfig), String> {
    let Some(llm_provider_id) = canonical_provider_id(request.llm_provider.as_str()) else {
        return Err(format!("Unknown provider: {}", request.llm_provider));
    };
    let swarm_base_url = normalize_openai_base_url(llm_provider_id, request.llm_base_url.clone())?;
    let requested_api_key = request.llm_api_key.clone().unwrap_or_default();
    let preserved_api_key = existing_provider
        .and_then(|provider| match provider {
            LlmProvider::Anthropic { api_key, .. } => Some(api_key.clone()),
            LlmProvider::OpenAI { api_key, .. } => Some(api_key.clone()),
            LlmProvider::Ollama { .. } => None,
        })
        .unwrap_or_default();
    let effective_api_key = if requested_api_key.trim().is_empty() {
        preserved_api_key
    } else {
        requested_api_key
    };
    let ollama_base_url = request
        .llm_base_url
        .clone()
        .filter(|value| !value.trim().is_empty());

    let llm_provider = match llm_provider_id {
        "anthropic" => LlmProvider::Anthropic {
            api_key: effective_api_key,
            model: request.llm_model.clone(),
        },
        "openai" | "openai-compatible" | "openrouter" | "openai-subscription" | "huggingface" => {
            LlmProvider::OpenAI {
                api_key: effective_api_key,
                model: request.llm_model.clone(),
                base_url: if llm_provider_id == "openai" {
                    None
                } else {
                    swarm_base_url
                },
            }
        }
        "ollama" => LlmProvider::Ollama {
            base_url: ollama_base_url.ok_or_else(|| "Ollama base URL is required".to_string())?,
            model: request.llm_model.clone(),
        },
        _ => return Err(format!("Unknown provider: {}", llm_provider_id)),
    };

    let specialist_config = crate::core::swarm::SpecialistConfig {
        id,
        name: request.name.clone(),
        agent_type: crate::core::swarm::persistence::parse_agent_type(
            &request.agent_type,
            request.system_prompt.as_deref(),
        ),
        llm_provider: llm_provider.clone(),
        system_prompt_override: request.system_prompt.clone(),
        max_memory_retrieval: 3,
        capabilities: crate::core::swarm::persistence::capability_strings_to_models(
            &request.capabilities,
        ),
        access_scope: request.access_scope.clone().normalized(),
        enabled: true,
    };

    Ok((llm_provider, specialist_config))
}

#[derive(Debug, Clone)]
pub(super) struct AccessPlanGroupAccumulator {
    scope_field: String,
    label: String,
    summary: String,
    review_band: String,
    selection_mode: String,
    suggested_ids: Vec<String>,
    reasons: Vec<String>,
    details: Vec<SwarmAgentAccessPlanDetail>,
}

pub(super) fn title_case_access_label(value: &str) -> String {
    value
        .split(['_', '-', ' '])
        .filter(|part| !part.is_empty())
        .map(|part| {
            let lower = part.to_ascii_lowercase();
            match lower.as_str() {
                "ssh" => "SSH".to_string(),
                "mcp" => "MCP".to_string(),
                "api" => "API".to_string(),
                other => {
                    let mut chars = other.chars();
                    match chars.next() {
                        Some(first) => {
                            format!("{}{}", first.to_ascii_uppercase(), chars.as_str())
                        }
                        None => String::new(),
                    }
                }
            }
        })
        .collect::<Vec<_>>()
        .join(" ")
}

pub(super) fn access_permission_label(permission_id: &str) -> String {
    match permission_id.trim().to_ascii_lowercase().as_str() {
        "code_execute" => "Code execution".to_string(),
        "shell" => "Shell commands".to_string(),
        "file_write" => "File writes".to_string(),
        "scheduler" => "Task scheduling".to_string(),
        "local_network_discovery" => "Local network discovery".to_string(),
        "browser_auto" => "Browser automation".to_string(),
        "app_hosting" => "App hosting".to_string(),
        "messaging_send" => "Messaging send".to_string(),
        "broad_network" => "Broad network actions".to_string(),
        "ssh" => "SSH execution".to_string(),
        "gmail" => "Gmail access".to_string(),
        "calendar_write" => "Calendar write".to_string(),
        "google_workspace_command" => "Workspace command execution".to_string(),
        "watcher" => "Background watchers".to_string(),
        "capability_acquire" => "Capability acquisition".to_string(),
        other => title_case_access_label(other),
    }
}

pub(super) fn access_scope_group_meta(
    scope_field: &str,
    selection_mode: &str,
) -> (&'static str, &'static str, &'static str) {
    match scope_field {
        "approved_permission_ids" => (
            "Permission approval",
            "Approve this elevated capability for the agent.",
            "elevated",
        ),
        "integration_ids" => (
            "Integrations",
            "Attach the integration(s) this agent needs.",
            "elevated",
        ),
        "extension_pack_ids" => (
            "Extension packs",
            "Attach the installed extension pack(s) this agent may use.",
            "elevated",
        ),
        "mcp_server_ids" => (
            "MCP servers",
            "Attach only the MCP servers this agent should use.",
            "elevated",
        ),
        "custom_api_ids" => (
            "Custom APIs",
            "Attach only the custom APIs this agent should use.",
            "elevated",
        ),
        "ssh_connection_names" => (
            "SSH connections",
            "Attach the SSH connections this agent may use.",
            "elevated",
        ),
        "channel_ids" => (
            "Messaging channels",
            "Attach the delivery channels this agent may use.",
            "elevated",
        ),
        _ if selection_mode == "toggle" => (
            "Permission approval",
            "Approve this elevated capability for the agent.",
            "elevated",
        ),
        _ => (
            "Access",
            "Review and attach the requested access.",
            "elevated",
        ),
    }
}

pub(super) fn push_access_group(
    groups: &mut HashMap<String, AccessPlanGroupAccumulator>,
    key: String,
    scope_field: &str,
    selection_mode: &str,
    label: String,
    reason: String,
    suggested_ids: Vec<String>,
    detail: Option<SwarmAgentAccessPlanDetail>,
) {
    let (_, summary, review_band) = access_scope_group_meta(scope_field, selection_mode);
    let entry = groups
        .entry(key)
        .or_insert_with(|| AccessPlanGroupAccumulator {
            scope_field: scope_field.to_string(),
            label,
            summary: summary.to_string(),
            review_band: review_band.to_string(),
            selection_mode: selection_mode.to_string(),
            suggested_ids: Vec::new(),
            reasons: Vec::new(),
            details: Vec::new(),
        });
    if !reason.trim().is_empty() && !entry.reasons.iter().any(|value| value == &reason) {
        entry.reasons.push(reason);
    }
    for suggested_id in suggested_ids {
        let normalized = suggested_id.trim().to_string();
        if !normalized.is_empty() && !entry.suggested_ids.iter().any(|value| value == &normalized) {
            entry.suggested_ids.push(normalized);
        }
    }
    if let Some(detail) = detail {
        if !entry.details.iter().any(|existing| {
            existing.action_name == detail.action_name
                && existing.reason == detail.reason
                && existing.permission_ids == detail.permission_ids
        }) {
            entry.details.push(detail);
        }
    }
}

pub(super) fn finalize_access_groups(
    mut groups: Vec<AccessPlanGroupAccumulator>,
) -> Vec<SwarmAgentAccessPlanGroup> {
    groups.sort_by(|left, right| {
        left.scope_field
            .cmp(&right.scope_field)
            .then_with(|| left.label.cmp(&right.label))
    });
    groups
        .into_iter()
        .map(|mut group| {
            group.suggested_ids.sort();
            group.suggested_ids.dedup();
            group.details.sort_by(|left, right| {
                left.action_name
                    .cmp(&right.action_name)
                    .then_with(|| left.reason.cmp(&right.reason))
            });
            let reason = if group.reasons.is_empty() {
                group.summary.clone()
            } else {
                group.reasons.join(" ")
            };
            SwarmAgentAccessPlanGroup {
                id: format!(
                    "{}:{}",
                    group.scope_field,
                    group
                        .suggested_ids
                        .first()
                        .cloned()
                        .unwrap_or_else(|| group.label.to_ascii_lowercase().replace(' ', "_"))
                ),
                scope_field: group.scope_field,
                label: group.label,
                summary: group.summary,
                reason,
                review_band: group.review_band,
                selection_mode: group.selection_mode,
                suggested_ids: group.suggested_ids,
                details: group.details,
            }
        })
        .collect()
}

pub(super) fn fallback_access_plan_actions(
    spec_summary: &str,
    actions: &[crate::actions::ActionDef],
) -> Vec<SwarmAgentAccessPlanAction> {
    let _ = (spec_summary, actions);
    Vec::new()
}

pub(super) async fn swarm_agent_builder_options(State(state): State<AppState>) -> Response {
    fn builder_status_looks_usable(status: &str) -> bool {
        let normalized = status.trim().to_ascii_lowercase();
        !matches!(
            normalized.as_str(),
            "disabled"
                | "not_configured"
                | "missing_config"
                | "missing_token"
                | "offline"
                | "unavailable"
                | "disconnected"
                | "error"
                | "failed"
        )
    }

    let agent = state.agent.read().await;
    let mcp_servers = {
        let registry = agent.mcp.read().await;
        match registry.list_servers(true).await {
            Ok(items) => items,
            Err(error) => {
                return (
                    StatusCode::INTERNAL_SERVER_ERROR,
                    Json(ErrorResponse {
                        error: format!("Failed to load MCP servers: {}", error),
                    }),
                )
                    .into_response();
            }
        }
    };
    #[cfg(feature = "ssh")]
    let ssh_connections = match crate::actions::ssh::list_connections(&agent.config_dir) {
        Ok(items) => items,
        Err(error) => {
            return (
                StatusCode::INTERNAL_SERVER_ERROR,
                Json(ErrorResponse {
                    error: format!("Failed to load SSH connections: {}", error),
                }),
            )
                .into_response();
        }
    };
    #[cfg(not(feature = "ssh"))]
    let ssh_connections: Vec<serde_json::Value> = Vec::new();
    let custom_apis = match crate::custom_apis::list_custom_apis(
        &agent.storage,
        &agent.config_dir,
        &agent.data_dir,
    )
    .await
    {
        Ok(items) => items,
        Err(error) => {
            return (
                StatusCode::INTERNAL_SERVER_ERROR,
                Json(ErrorResponse {
                    error: format!("Failed to load custom APIs: {}", error),
                }),
            )
                .into_response();
        }
    };
    let integrations = integrations::collect_integrations(&agent)
        .await
        .into_iter()
        .filter(|integration| {
            integration.enabled && builder_status_looks_usable(&integration.status)
        })
        .collect::<Vec<_>>();
    let extension_packs = {
        let registry = agent.extension_packs.read().await;
        match registry.search_packs(None, Some("integration")).await {
            Ok(result) => result
                .installed
                .into_iter()
                .filter(|pack| pack.enabled && builder_status_looks_usable(&pack.status))
                .collect::<Vec<_>>(),
            Err(error) => {
                return (
                    StatusCode::INTERNAL_SERVER_ERROR,
                    Json(ErrorResponse {
                        error: format!("Failed to load extension packs: {}", error),
                    }),
                )
                    .into_response();
            }
        }
    };
    let channels: Vec<crate::core::GatewayChannelDescriptor> =
        match crate::core::load_gateway_channels(&agent.storage, &agent.config).await {
            Ok(payload) => payload
                .channels
                .into_iter()
                .filter(|channel| {
                    channel.enabled
                        && channel.configured
                        && builder_status_looks_usable(&channel.status)
                })
                .collect(),
            Err(error) => {
                return (
                    StatusCode::INTERNAL_SERVER_ERROR,
                    Json(ErrorResponse {
                        error: format!("Failed to load messaging channels: {}", error),
                    }),
                )
                    .into_response();
            }
        };
    let custom_api_payload: Vec<serde_json::Value> = custom_apis
        .into_iter()
        .map(|api| {
            serde_json::json!({
                "id": api.config.id,
                "name": api.config.name,
                "base_url": api.config.base_url,
                "enabled": api.config.enabled,
                "action_count": api.action_count,
                "secret_configured": api.secret_configured,
            })
        })
        .collect();
    let channel_payload: Vec<serde_json::Value> = channels
        .into_iter()
        .map(|channel| {
            serde_json::json!({
                "id": channel.id,
                "name": channel.name,
                "kind": channel.kind,
                "status": channel.status,
                "enabled": channel.enabled,
                "configured": channel.configured,
                "account_count": channel.account_count,
                "connected_account_count": channel.connected_account_count,
            })
        })
        .collect();
    (
        StatusCode::OK,
        Json(serde_json::json!({
            "mcp_servers": mcp_servers,
            "ssh_connections": ssh_connections,
            "custom_apis": custom_api_payload,
            "integrations": integrations,
            "extension_packs": extension_packs,
            "channels": channel_payload,
        })),
    )
        .into_response()
}

pub(super) async fn swarm_agent_access_plan(
    State(state): State<AppState>,
    Json(request): Json<SwarmAgentAccessPlanRequest>,
) -> Response {
    let spec_summary = vec![
        format!("name: {}", request.name.trim()),
        format!("role: {}", request.agent_type.trim()),
        format!(
            "description: {}",
            request
                .description
                .trim()
                .replace('\r', " ")
                .replace('\n', " ")
        ),
        format!(
            "capabilities: {}",
            request
                .capabilities
                .iter()
                .map(|value| value.trim())
                .filter(|value| !value.is_empty())
                .collect::<Vec<_>>()
                .join(", ")
        ),
        format!(
            "system_prompt: {}",
            request
                .system_prompt
                .trim()
                .replace('\r', " ")
                .replace('\n', " ")
        ),
    ]
    .join("\n");

    let agent = state.agent.read().await;
    let actions = match agent.runtime.list_enabled_actions().await {
        Ok(actions) => actions,
        Err(error) => {
            return (
                StatusCode::INTERNAL_SERVER_ERROR,
                Json(ErrorResponse {
                    error: format!("Failed to load action catalog: {}", error),
                }),
            )
                .into_response();
        }
    };
    let action_scope_hints = match agent.runtime.list_action_scope_hints().await {
        Ok(hints) => hints,
        Err(error) => {
            return (
                StatusCode::INTERNAL_SERVER_ERROR,
                Json(ErrorResponse {
                    error: format!("Failed to load action scope hints: {}", error),
                }),
            )
                .into_response();
        }
    };

    let mut catalog_by_name: HashMap<String, crate::actions::ActionDef> = HashMap::new();
    for action in &actions {
        catalog_by_name.insert(action.name.clone(), action.clone());
    }

    let suggested_actions = fallback_access_plan_actions(&spec_summary, &actions);
    let notes = Vec::new();

    let mut implicit_groups: HashMap<String, AccessPlanGroupAccumulator> = HashMap::new();
    let mut requested_groups: HashMap<String, AccessPlanGroupAccumulator> = HashMap::new();

    for action_plan in &suggested_actions {
        let Some(action) = catalog_by_name.get(&action_plan.name) else {
            continue;
        };
        let detail = SwarmAgentAccessPlanDetail {
            action_name: action.name.clone(),
            reason: action_plan.reason.clone(),
            permission_ids: crate::runtime::ActionRuntime::action_required_agent_permission_ids(
                action,
            ),
        };

        let safe_permissions =
            crate::security::ActionGuard::permissions_from_capabilities(&action.capabilities)
                .into_iter()
                .filter(|permission| {
                    crate::security::ActionGuard::permission_risk(permission)
                        == crate::security::action_guard::PermissionRisk::Safe
                        && !matches!(
                            permission,
                            crate::security::action_guard::Permission::Custom(_)
                        )
                })
                .map(|permission| permission.to_string())
                .collect::<Vec<_>>();
        for permission_id in safe_permissions {
            push_access_group(
                &mut implicit_groups,
                format!("approved_permission_ids:{}", permission_id),
                "approved_permission_ids",
                "toggle",
                access_permission_label(&permission_id),
                format!(
                    "{} can stay implicit for this agent.",
                    access_permission_label(&permission_id)
                ),
                vec![permission_id],
                Some(detail.clone()),
            );
        }

        for permission_id in
            crate::runtime::ActionRuntime::action_required_agent_permission_ids(action)
        {
            push_access_group(
                &mut requested_groups,
                format!("approved_permission_ids:{}", permission_id),
                "approved_permission_ids",
                "toggle",
                access_permission_label(&permission_id),
                format!("{} requested this elevated capability.", action_plan.name),
                vec![permission_id],
                Some(detail.clone()),
            );
        }

        let hint = action_scope_hints
            .get(&action.name)
            .cloned()
            .unwrap_or_default();
        if let Some(server_id) = hint.mcp_server_id {
            push_access_group(
                &mut requested_groups,
                "mcp_server_ids".to_string(),
                "mcp_server_ids",
                "exact",
                "MCP servers".to_string(),
                format!("{} needs a specific MCP server.", action_plan.name),
                vec![server_id],
                Some(detail.clone()),
            );
        }
        if let Some(api_id) = hint.custom_api_id {
            push_access_group(
                &mut requested_groups,
                "custom_api_ids".to_string(),
                "custom_api_ids",
                "exact",
                "Custom APIs".to_string(),
                format!("{} needs a custom API binding.", action_plan.name),
                vec![api_id],
                Some(detail.clone()),
            );
        }
        if !hint.integration_ids.is_empty() {
            push_access_group(
                &mut requested_groups,
                "integration_ids".to_string(),
                "integration_ids",
                "exact",
                "Integrations".to_string(),
                format!("{} needs external integration access.", action_plan.name),
                hint.integration_ids.clone(),
                Some(detail.clone()),
            );
        }
        if !hint.extension_pack_ids.is_empty() {
            push_access_group(
                &mut requested_groups,
                "extension_pack_ids".to_string(),
                "extension_pack_ids",
                "exact",
                "Extension packs".to_string(),
                format!(
                    "{} needs an installed extension pack binding.",
                    action_plan.name
                ),
                hint.extension_pack_ids.clone(),
                Some(detail.clone()),
            );
        }
        if hint.requires_ssh_connection {
            push_access_group(
                &mut requested_groups,
                "ssh_connection_names".to_string(),
                "ssh_connection_names",
                "choose_any",
                "SSH connections".to_string(),
                format!("{} needs an attached SSH connection.", action_plan.name),
                request.access_scope.ssh_connection_names.clone(),
                Some(detail.clone()),
            );
        }
        if !hint.channel_targets.is_empty() {
            push_access_group(
                &mut requested_groups,
                "channel_ids".to_string(),
                "channel_ids",
                "choose_any",
                "Messaging channels".to_string(),
                format!("{} needs a delivery channel.", action_plan.name),
                request.access_scope.channel_ids.clone(),
                Some(detail),
            );
        }
    }

    for permission_id in &request.access_scope.approved_permission_ids {
        let trimmed = permission_id.trim();
        if trimmed.is_empty() {
            continue;
        }
        push_access_group(
            &mut requested_groups,
            format!("approved_permission_ids:{}", trimmed.to_ascii_lowercase()),
            "approved_permission_ids",
            "toggle",
            access_permission_label(trimmed),
            "Already approved for this agent.".to_string(),
            vec![trimmed.to_string()],
            None,
        );
    }
    for (scope_field, label, selection_mode, values) in [
        (
            "mcp_server_ids",
            "MCP servers",
            "exact",
            request.access_scope.mcp_server_ids.clone(),
        ),
        (
            "custom_api_ids",
            "Custom APIs",
            "exact",
            request.access_scope.custom_api_ids.clone(),
        ),
        (
            "integration_ids",
            "Integrations",
            "exact",
            request.access_scope.integration_ids.clone(),
        ),
        (
            "extension_pack_ids",
            "Extension packs",
            "exact",
            request.access_scope.extension_pack_ids.clone(),
        ),
        (
            "ssh_connection_names",
            "SSH connections",
            "choose_any",
            request.access_scope.ssh_connection_names.clone(),
        ),
        (
            "channel_ids",
            "Messaging channels",
            "choose_any",
            request.access_scope.channel_ids.clone(),
        ),
    ] {
        if values.is_empty() {
            continue;
        }
        push_access_group(
            &mut requested_groups,
            scope_field.to_string(),
            scope_field,
            selection_mode,
            label.to_string(),
            "Already attached for this agent.".to_string(),
            values,
            None,
        );
    }

    let response = SwarmAgentAccessPlanResponse {
        implicit_access: finalize_access_groups(implicit_groups.into_values().collect()),
        requested_access: finalize_access_groups(requested_groups.into_values().collect()),
        suggested_actions,
        notes,
    };
    (StatusCode::OK, Json(response)).into_response()
}

pub(super) async fn swarm_draft_agent(
    State(state): State<AppState>,
    Json(request): Json<SwarmAgentDraftRequest>,
) -> Response {
    let description = request.description.trim();
    if description.is_empty() {
        return (
            StatusCode::BAD_REQUEST,
            Json(ErrorResponse {
                error: "Description is required".to_string(),
            }),
        )
            .into_response();
    }

    let agent = state.agent.read().await;
    let llm = resolve_swarm_draft_client(&agent, request.model_profile_id.as_deref());
    let system_prompt = "You design custom specialist agents. Return strict JSON with keys name, agent_type, capabilities, and system_prompt. The capabilities field must be a short array of strings. The system_prompt must be a direct instruction block for the specialist.";
    let user_prompt = format!(
        "Create a custom specialist agent draft from this user description:\n\n{}\n\nRules:\n- Keep the name short and specific.\n- agent_type can be a built-in role or any custom role label.\n- capabilities should be 3 to 8 concise phrases.\n- system_prompt should be ready to save as the specialist prompt.\n- Return JSON only.",
        description
    );
    let response = match llm.chat_with_system(system_prompt, &user_prompt).await {
        Ok(response) => response,
        Err(error) => {
            return (
                StatusCode::BAD_GATEWAY,
                Json(ErrorResponse {
                    error: format!("Failed to generate draft: {}", error),
                }),
            )
                .into_response();
        }
    };
    let Some(payload) = extract_json(&response.content) else {
        return (
            StatusCode::BAD_GATEWAY,
            Json(ErrorResponse {
                error: "Draft model returned invalid JSON".to_string(),
            }),
        )
            .into_response();
    };
    let name = payload
        .get("name")
        .and_then(|value| value.as_str())
        .map(str::trim)
        .filter(|value| !value.is_empty())
        .map(str::to_string)
        .unwrap_or_else(|| {
            description
                .split_whitespace()
                .take(4)
                .collect::<Vec<_>>()
                .join(" ")
        });
    let agent_type = payload
        .get("agent_type")
        .and_then(|value| value.as_str())
        .map(str::trim)
        .filter(|value| !value.is_empty())
        .map(str::to_string)
        .unwrap_or_else(|| "custom".to_string());
    let capabilities = payload
        .get("capabilities")
        .map(|value| match value {
            serde_json::Value::Array(items) => items
                .iter()
                .filter_map(|item| item.as_str())
                .map(str::trim)
                .filter(|value| !value.is_empty())
                .map(str::to_string)
                .collect::<Vec<_>>(),
            serde_json::Value::String(text) => text
                .split(',')
                .map(str::trim)
                .filter(|value| !value.is_empty())
                .map(str::to_string)
                .collect::<Vec<_>>(),
            _ => Vec::new(),
        })
        .unwrap_or_default();
    let system_prompt = payload
        .get("system_prompt")
        .and_then(|value| value.as_str())
        .map(str::trim)
        .filter(|value| !value.is_empty())
        .map(str::to_string)
        .unwrap_or_else(|| description.to_string());
    (
        StatusCode::OK,
        Json(serde_json::json!(SwarmAgentDraftResponse {
            name: if name.is_empty() {
                "Custom Agent".to_string()
            } else {
                name
            },
            agent_type,
            capabilities,
            system_prompt,
        })),
    )
        .into_response()
}

/// Add a specialist agent to the swarm
pub(super) async fn swarm_add_agent(
    State(state): State<AppState>,
    Json(request): Json<AddSwarmAgentRequest>,
) -> Response {
    let agent_id = uuid::Uuid::new_v4().to_string();
    let (llm_provider, specialist_config) =
        match build_swarm_agent_spec(Some(agent_id.clone()), &request, None) {
            Ok(spec) => spec,
            Err(error) => {
                return (StatusCode::BAD_REQUEST, Json(ErrorResponse { error })).into_response();
            }
        };

    let mut agent = state.agent.write().await;
    let db_agent = crate::storage::entities::swarm_agent::Model {
        id: agent_id.clone(),
        name: request.name.clone(),
        agent_type: request.agent_type.clone(),
        llm_provider: serde_json::to_string(&llm_provider).unwrap_or_default(),
        capabilities: serde_json::to_string(
            &crate::core::swarm::persistence::capability_models_to_strings(
                &specialist_config.capabilities,
            ),
        )
        .unwrap_or("[]".to_string()),
        system_prompt: request.system_prompt.clone(),
        access_scope: crate::core::swarm::persistence::access_scope_to_json(
            &specialist_config.access_scope,
        ),
        enabled: 1,
        created_at: chrono::Utc::now().to_rfc3339(),
    };

    if let Err(e) = agent.storage.insert_swarm_agent(&db_agent).await {
        return (
            StatusCode::INTERNAL_SERVER_ERROR,
            Json(ErrorResponse {
                error: format!("Failed to save agent: {}", e),
            }),
        )
            .into_response();
    }

    let response = if let Some(ref swarm) = agent.swarm {
        match swarm
            .add_specialist(specialist_config.clone(), vec![])
            .await
        {
            Ok(id) => serde_json::json!({
                "status": "ok",
                "agent_id": id.to_string(),
                "message": format!("Agent '{}' added to swarm", request.name),
            }),
            Err(e) => serde_json::json!({
                "status": "ok",
                "agent_id": agent_id,
                "message": format!("Agent '{}' saved but swarm add failed: {}. Will be loaded on restart.", request.name, e),
            }),
        }
    } else {
        serde_json::json!({
            "status": "ok",
            "agent_id": agent_id,
            "message": format!("Agent '{}' saved. Swarm will activate it on next initialization.", request.name),
        })
    };

    if let Some(idx) = agent
        .config
        .swarm
        .specialists
        .iter()
        .position(|item| item.id.as_deref() == Some(agent_id.as_str()))
    {
        agent.config.swarm.specialists[idx] = specialist_config;
    } else {
        agent.config.swarm.specialists.push(specialist_config);
    }
    if let Err(e) = agent.config.save(&agent.config_dir, Some(&agent.data_dir)) {
        return (
            StatusCode::INTERNAL_SERVER_ERROR,
            Json(ErrorResponse {
                error: format!("Failed to save swarm config: {}", e),
            }),
        )
            .into_response();
    }

    (StatusCode::OK, Json(response)).into_response()
}

/// Update a specialist agent
pub(super) async fn swarm_update_agent(
    State(state): State<AppState>,
    Path(id): Path<String>,
    Json(request): Json<AddSwarmAgentRequest>,
) -> Response {
    let mut agent = state.agent.write().await;
    let existing = match agent.storage.get_swarm_agents().await {
        Ok(items) => match items.into_iter().find(|item| item.id == id) {
            Some(model) => model,
            None => {
                return (
                    StatusCode::NOT_FOUND,
                    Json(ErrorResponse {
                        error: "Agent not found".to_string(),
                    }),
                )
                    .into_response();
            }
        },
        Err(e) => {
            return (
                StatusCode::INTERNAL_SERVER_ERROR,
                Json(ErrorResponse {
                    error: format!("Failed to load agent: {}", e),
                }),
            )
                .into_response();
        }
    };
    let existing_provider = crate::core::swarm::persistence::parse_llm_provider(
        &existing.llm_provider,
        &agent.config.llm,
    );
    let (llm_provider, specialist_config) =
        match build_swarm_agent_spec(Some(id.clone()), &request, Some(&existing_provider)) {
            Ok(spec) => spec,
            Err(error) => {
                return (StatusCode::BAD_REQUEST, Json(ErrorResponse { error })).into_response();
            }
        };

    let db_agent = crate::storage::entities::swarm_agent::Model {
        id: existing.id.clone(),
        name: request.name.clone(),
        agent_type: request.agent_type.clone(),
        llm_provider: serde_json::to_string(&llm_provider).unwrap_or_default(),
        capabilities: serde_json::to_string(
            &crate::core::swarm::persistence::capability_models_to_strings(
                &specialist_config.capabilities,
            ),
        )
        .unwrap_or("[]".to_string()),
        system_prompt: request.system_prompt.clone(),
        access_scope: crate::core::swarm::persistence::access_scope_to_json(
            &specialist_config.access_scope,
        ),
        enabled: 1,
        created_at: existing.created_at.clone(),
    };

    if let Err(e) = agent.storage.update_swarm_agent(&db_agent).await {
        return (
            StatusCode::INTERNAL_SERVER_ERROR,
            Json(ErrorResponse {
                error: format!("Failed to update agent: {}", e),
            }),
        )
            .into_response();
    }

    if let Some(ref swarm) = agent.swarm {
        let live_id = crate::core::swarm::AgentId(id.clone());
        let _ = swarm.remove_specialist(&live_id).await;
        if let Err(e) = swarm
            .add_specialist(specialist_config.clone(), vec![])
            .await
        {
            tracing::warn!("Failed to re-register updated swarm agent '{}': {}", id, e);
        }
    }

    if let Some(idx) = agent
        .config
        .swarm
        .specialists
        .iter()
        .position(|item| item.id.as_deref() == Some(id.as_str()))
    {
        agent.config.swarm.specialists[idx] = specialist_config;
    } else {
        agent.config.swarm.specialists.push(specialist_config);
    }

    if let Err(e) = agent.config.save(&agent.config_dir, Some(&agent.data_dir)) {
        return (
            StatusCode::INTERNAL_SERVER_ERROR,
            Json(ErrorResponse {
                error: format!("Failed to save swarm config: {}", e),
            }),
        )
            .into_response();
    }

    (
        StatusCode::OK,
        Json(serde_json::json!({
            "status": "ok",
            "agent_id": id,
            "message": "Agent updated",
        })),
    )
        .into_response()
}

/// Remove a swarm agent
pub(super) async fn swarm_remove_agent(
    State(state): State<AppState>,
    Path(id): Path<String>,
) -> Response {
    let mut agent = state.agent.write().await;

    // Remove from DB
    if let Err(e) = agent.storage.delete_swarm_agent(&id).await {
        return (
            StatusCode::INTERNAL_SERVER_ERROR,
            Json(ErrorResponse {
                error: format!("Failed to delete: {}", e),
            }),
        )
            .into_response();
    }

    // Remove from live swarm
    if let Some(ref swarm) = agent.swarm {
        let agent_id = crate::core::swarm::AgentId(id.clone());
        let _ = swarm.remove_specialist(&agent_id).await;
    }

    agent.config.swarm.specialists.retain(|item| {
        item.id
            .as_deref()
            .map(|value| value != id.as_str())
            .unwrap_or(true)
    });
    if let Err(e) = agent.config.save(&agent.config_dir, Some(&agent.data_dir)) {
        return (
            StatusCode::INTERNAL_SERVER_ERROR,
            Json(ErrorResponse {
                error: format!("Failed to save swarm config: {}", e),
            }),
        )
            .into_response();
    }

    (
        StatusCode::OK,
        Json(serde_json::json!({
            "status": "ok",
            "message": "Agent removed",
        })),
    )
        .into_response()
}

/// Get swarm config
pub(super) async fn swarm_get_config(State(state): State<AppState>) -> Response {
    let agent = state.agent.read().await;
    let config = &agent.config.swarm;
    (
        StatusCode::OK,
        Json(serde_json::json!({
            "enabled": true,
            "max_specialists": config.max_specialists,
            "default_timeout_secs": config.default_timeout_secs,
            "timeout_policy": "wait_until_completion",
        })),
    )
        .into_response()
}

/// Update swarm config request
#[derive(Debug, Deserialize)]
pub struct UpdateSwarmConfigRequest {
    pub max_specialists: Option<usize>,
    pub default_timeout_secs: Option<u64>,
}

/// Update swarm config
pub(super) async fn swarm_update_config(
    State(state): State<AppState>,
    Json(request): Json<UpdateSwarmConfigRequest>,
) -> Response {
    let mut agent = state.agent.write().await;

    if let Some(max) = request.max_specialists {
        agent.config.swarm.max_specialists = max;
    }
    if let Some(timeout) = request.default_timeout_secs {
        agent.config.swarm.default_timeout_secs = timeout;
    }

    // Save config
    if let Err(e) = agent.config.save(&agent.config_dir, Some(&agent.data_dir)) {
        return (
            StatusCode::INTERNAL_SERVER_ERROR,
            Json(ErrorResponse {
                error: format!("Failed to save config: {}", e),
            }),
        )
            .into_response();
    }

    (
        StatusCode::OK,
        Json(serde_json::json!({
            "status": "ok",
            "message": "Swarm config updated. Delegated agents wait until completion.",
            "enabled": true,
            "timeout_policy": "wait_until_completion",
        })),
    )
        .into_response()
}

/// List recent swarm delegations
pub(super) async fn swarm_list_delegations(
    State(state): State<AppState>,
    axum::extract::Query(params): axum::extract::Query<std::collections::HashMap<String, String>>,
) -> Response {
    let agent = state.agent.read().await;
    let limit_param = params.get("limit").map(|value| value.trim().to_string());
    let limit = if matches!(limit_param.as_deref(), Some("all")) {
        usize::MAX
    } else {
        limit_param
            .as_deref()
            .and_then(|value| value.parse::<usize>().ok())
            .unwrap_or(50)
            .clamp(1, 200)
    };
    let delegations = match limit_param.as_deref() {
        Some("all") => agent.storage.get_all_delegations().await,
        _ => {
            let mut merged: std::collections::HashMap<
                String,
                crate::storage::entities::swarm_delegation::Model,
            > = std::collections::HashMap::new();
            for row in agent
                .storage
                .get_active_swarm_delegations(limit as u64)
                .await
                .unwrap_or_default()
            {
                merged.insert(row.id.clone(), row);
            }
            for row in agent
                .storage
                .get_recent_delegations(limit as u64)
                .await
                .unwrap_or_default()
            {
                merged.entry(row.id.clone()).or_insert(row);
            }
            let mut rows = merged.into_values().collect::<Vec<_>>();
            rows.sort_by(|left, right| {
                right
                    .completed_at
                    .clone()
                    .unwrap_or_else(|| right.created_at.clone())
                    .cmp(
                        &left
                            .completed_at
                            .clone()
                            .unwrap_or_else(|| left.created_at.clone()),
                    )
            });
            if limit != usize::MAX && rows.len() > limit {
                rows.truncate(limit);
            }
            Ok(rows)
        }
    };
    match delegations {
        Ok(delegations) => {
            let items: Vec<serde_json::Value> = delegations
                .iter()
                .map(|d| {
                    let payload = parse_swarm_delegation_result(d.result.as_deref());
                    serde_json::json!({
                        "id": d.id,
                        "parent_task_id": d.parent_task_id,
                        "agent_id": d.agent_id,
                        "task": d.task_description,
                        "success": d.success == 1,
                        "confidence": d.confidence,
                        "execution_time_ms": d.execution_time_ms,
                        "result": d.result,
                        "status": swarm_result_string(&payload, "status"),
                        "agent_name": swarm_result_string(&payload, "agent_name"),
                        "agent_role": swarm_result_string(&payload, "agent_role"),
                        "model_name": swarm_result_string(&payload, "model_name"),
                        "conversation_id": swarm_result_string(&payload, "conversation_id"),
                        "channel": swarm_result_string(&payload, "channel"),
                        "created_at": d.created_at,
                        "completed_at": d.completed_at,
                    })
                })
                .collect();
            let mut grouped: std::collections::HashMap<
                String,
                Vec<crate::storage::entities::swarm_delegation::Model>,
            > = std::collections::HashMap::new();
            for row in delegations {
                let run_id = swarm_delegation_run_id(&row);
                grouped.entry(run_id).or_default().push(row);
            }
            let mut runs = agent.swarm_activity.recent_runs(limit).await;
            for run in agent.swarm_activity.active_runs().await {
                if !runs.iter().any(|existing| existing.id == run.id) {
                    runs.push(run);
                }
            }
            for (run_id, rows) in grouped {
                if runs.iter().any(|existing| existing.id == run_id) {
                    continue;
                }
                runs.push(build_swarm_run_from_rows(&run_id, &rows));
            }
            sort_swarm_runs(&mut runs);
            if limit != usize::MAX && runs.len() > limit {
                runs.truncate(limit);
            }
            (
                StatusCode::OK,
                Json(serde_json::json!({
                    "delegations": items,
                    "runs": runs,
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
