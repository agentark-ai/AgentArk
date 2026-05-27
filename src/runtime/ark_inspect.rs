use anyhow::{Context, Result};
use serde::Deserialize;

use super::ActionRuntime;

pub(super) fn action_def() -> crate::actions::ActionDef {
    crate::actions::ActionDef {
        name: "ark_inspect".to_string(),
        description: "Generic read-only Ark inspection for live/local AgentArk state. Use when the user asks for evidence that may require the control API, deployed app registry, stored database telemetry, schema discovery, structured DB reads, traces, logs, tasks, integrations, files, workspace state, analytics, current model/provider selection, model access/readiness, failover/provider health, model/provider usage, recent conversations, work history, personal activity patterns, recent attention, avoidance, recurring themes, inferred mindset, or other internal runtime data. The runtime injects AgentArk API auth server-side; never include credentials in arguments.".to_string(),
        version: "1.0.0".to_string(),
        input_schema: serde_json::json!({
            "type": "object",
            "properties": {
                "operation": {
                    "type": "string",
                    "enum": ["surface", "api_catalog", "api_get", "db_schema", "db_query", "file_read"],
                    "description": "Structured read-only inspection mode. Omit for overview/surface inspection. Use api_catalog to discover read-only AgentArk API data surfaces before guessing database tables."
                },
                "surface": {
                    "type": "string",
                    "enum": ["overview", "apps", "activity", "analytics", "models", "gateway_ops", "arkpulse", "sentinel", "evolution", "trace", "moltbook"],
                    "description": "Internal AgentArk surface to inspect when operation=surface. Use models for current model/provider selection, configured slots, readiness, access, and failover/provider health. Use analytics for usage, cost, token, model, channel, and purpose reports; use activity for recent user chats, work objects, Reflect-derived work units, Sentinel background signals, and local signals that support reflective pattern summaries about behavior, attention, avoidance, recurring themes, and follow-through. For specific AgentArk-owned data/reporting needs, use api_catalog then api_get. Default: overview."
                },
                "limit": {
                    "type": "integer",
                    "description": "Maximum recent records to include per section (default: 6)."
                },
                "trace_id": {
                    "type": "string",
                    "description": "Optional execution trace id when surface=trace."
                },
                "query": {
                    "type": "string",
                    "description": "Optional read-only target qualifier for surfaces that support matching existing runtime objects, such as deployed apps."
                },
                "inspect_target": {
                    "type": "string",
                    "description": "Optional planner qualifier equivalent to surface. The model may pass this when following an intent plan."
                },
                "api": {
                    "type": "object",
                    "description": "Read-only internal AgentArk API request. Use path=/openapi.json to discover available endpoints, then call a relevant GET endpoint. Bearer auth is injected by runtime and must not be included here.",
                    "properties": {
                        "path": { "type": "string", "description": "AgentArk-relative API path, such as /openapi.json or /api/apps. External URLs are not accepted." },
                        "method": { "type": "string", "enum": ["get"], "description": "Read-only method. Only GET is supported." },
                        "query": { "type": "object", "description": "Optional query parameters. Values may be scalars or arrays." },
                        "timeout_secs": { "type": "integer", "description": "Request timeout seconds (default: 10)." }
                    },
                    "required": ["path"]
                },
                "database": {
                    "type": "object",
                    "description": "Structured read-only public Postgres table query for operation=db_query. Discover valid tables and columns with operation=db_schema first. Raw SQL is not accepted.",
                    "properties": {
                        "table": { "type": "string", "description": "Public AgentArk table name." },
                        "columns": { "type": "array", "items": { "type": "string" }, "description": "Optional list of columns. Default: all readable columns." },
                        "filters": {
                            "type": "array",
                            "items": {
                                "type": "object",
                                "properties": {
                                    "column": { "type": "string" },
                                    "op": { "type": "string", "enum": ["eq", "neq", "gt", "gte", "lt", "lte", "contains", "starts_with", "ends_with", "in", "is_null", "not_null"] },
                                    "value": {}
                                },
                                "required": ["column", "op"]
                            }
                        },
                        "order_by": {
                            "type": "array",
                            "items": {
                                "type": "object",
                                "properties": {
                                    "column": { "type": "string" },
                                    "direction": { "type": "string", "enum": ["asc", "desc"] }
                                },
                                "required": ["column"]
                            }
                        },
                        "limit": { "type": "integer", "description": "Maximum rows to return (default: 50, max: 200)." }
                    },
                    "required": ["table"]
                },
                "table_filter": {
                    "type": "string",
                    "description": "Optional schema table substring filter for operation=db_schema."
                },
                "file": {
                    "type": "object",
                    "description": "Scoped file read under AgentArk allowed roots for operation=file_read. Sensitive credential-like files are refused and output is redacted.",
                    "properties": {
                        "path": { "type": "string", "description": "Path under the workspace, AgentArk data dir, config dir, or skills dir." },
                        "max_chars": { "type": "integer", "description": "Maximum characters to return after redaction (default: 20000)." }
                    },
                    "required": ["path"]
                }
            }
        }),
        capabilities: vec![
            "platform_observability".to_string(),
            "app_registry".to_string(),
            "app_inventory".to_string(),
            "personal_activity".to_string(),
            "activity_insights".to_string(),
            "conversation_history".to_string(),
            "session_history".to_string(),
            "memory".to_string(),
            "database_readonly".to_string(),
            "file_read".to_string(),
            "analytics".to_string(),
            "model_runtime".to_string(),
            "model_status".to_string(),
            "provider_status".to_string(),
        ],
        sandbox_mode: Some(crate::runtime::SandboxMode::Native),
        source: crate::actions::ActionSource::System,
        file_path: None,
        authorization: Default::default(),
    }
}

#[derive(Debug, Deserialize, Default)]
struct InternalApiInspectRequest {
    #[serde(default)]
    path: String,
    #[serde(default)]
    method: Option<String>,
    #[serde(default)]
    query: serde_json::Value,
    #[serde(default)]
    timeout_secs: Option<u64>,
}

#[derive(Debug, Deserialize, Default)]
struct FileInspectRequest {
    #[serde(default)]
    path: String,
    #[serde(default)]
    max_chars: Option<usize>,
}

#[derive(Debug, Deserialize, Default)]
struct AgentArkInspectRequest {
    #[serde(default)]
    operation: Option<String>,
    #[serde(default)]
    surface: Option<String>,
    #[serde(default)]
    query: Option<String>,
    #[serde(default)]
    inspect_target: Option<String>,
    #[serde(default)]
    trace_id: Option<String>,
    #[serde(default)]
    limit: Option<u64>,
    #[serde(default)]
    table_filter: Option<String>,
    #[serde(default)]
    api: Option<InternalApiInspectRequest>,
    #[serde(default)]
    database: Option<serde_json::Value>,
    #[serde(default)]
    file: Option<FileInspectRequest>,
}

fn normalized_key(value: &str) -> String {
    value
        .trim()
        .chars()
        .map(|ch| {
            if ch.is_ascii_alphanumeric() {
                ch.to_ascii_lowercase()
            } else {
                '_'
            }
        })
        .collect::<String>()
        .split('_')
        .filter(|part| !part.is_empty())
        .collect::<Vec<_>>()
        .join("_")
}

fn inspect_operation(request: &AgentArkInspectRequest) -> String {
    if let Some(operation) = request
        .operation
        .as_deref()
        .map(normalized_key)
        .filter(|value| !value.is_empty())
    {
        return operation;
    }
    if request
        .api
        .as_ref()
        .is_some_and(|api| !api.path.trim().is_empty())
    {
        return "api_get".to_string();
    }
    if request.database.is_some() {
        return "db_query".to_string();
    }
    if request
        .file
        .as_ref()
        .is_some_and(|file| !file.path.trim().is_empty())
    {
        return "file_read".to_string();
    }
    if request
        .table_filter
        .as_deref()
        .is_some_and(|value| !value.trim().is_empty())
    {
        return "db_schema".to_string();
    }
    "surface".to_string()
}

fn surface_name(request: &AgentArkInspectRequest) -> String {
    request
        .surface
        .as_deref()
        .or(request.inspect_target.as_deref())
        .map(str::trim)
        .filter(|value| !value.is_empty())
        .unwrap_or("overview")
        .to_string()
}

async fn apps_surface(
    runtime: &ActionRuntime,
    request: &AgentArkInspectRequest,
) -> Result<serde_json::Value> {
    let payload = internal_api_get(
        runtime,
        &InternalApiInspectRequest {
            path: "/api/apps".to_string(),
            method: Some("get".to_string()),
            query: serde_json::Value::Null,
            timeout_secs: Some(10),
        },
    )
    .await?;
    let body = payload
        .get("body")
        .cloned()
        .unwrap_or(serde_json::Value::Null);
    let query = request.query.as_deref().map(str::trim).unwrap_or("");
    let mut apps = body
        .get("apps")
        .and_then(|value| value.as_array())
        .cloned()
        .unwrap_or_default();

    for app in &mut apps {
        let Some(obj) = app.as_object_mut() else {
            continue;
        };
        if let Some(app_id) = obj.get("id").and_then(|value| value.as_str()) {
            obj.insert(
                "app_dir".to_string(),
                serde_json::Value::String(
                    runtime
                        .data_dir()
                        .join("apps")
                        .join(app_id)
                        .to_string_lossy()
                        .to_string(),
                ),
            );
        }
    }

    let matched_apps = if query.is_empty() {
        Vec::new()
    } else {
        let query_key = normalized_key(query);
        apps.iter()
            .filter(|app| {
                let id = app
                    .get("id")
                    .and_then(|value| value.as_str())
                    .map(normalized_key)
                    .unwrap_or_default();
                let title = app
                    .get("title")
                    .and_then(|value| value.as_str())
                    .map(normalized_key)
                    .unwrap_or_default();
                !query_key.is_empty()
                    && (id == query_key
                        || title == query_key
                        || id.contains(&query_key)
                        || title.contains(&query_key))
            })
            .cloned()
            .collect::<Vec<_>>()
    };

    Ok(serde_json::json!({
        "surface": "apps",
        "generated_at": chrono::Utc::now().to_rfc3339(),
        "query": if query.is_empty() { serde_json::Value::Null } else { serde_json::json!(query) },
        "total_apps": apps.len(),
        "matched_apps": matched_apps,
        "apps": apps,
        "restore": body.get("restore").cloned().unwrap_or(serde_json::Value::Null),
    }))
}

fn compact_for_activity(value: &str, max_chars: usize) -> String {
    let redacted = redact_text(value);
    if redacted.chars().count() <= max_chars {
        return redacted;
    }
    redacted.chars().take(max_chars).collect::<String>()
}

fn provider_model_name(provider: &crate::core::LlmProvider) -> &str {
    match provider {
        crate::core::LlmProvider::Anthropic { model, .. }
        | crate::core::LlmProvider::OpenAI { model, .. }
        | crate::core::LlmProvider::Ollama { model, .. } => model.as_str(),
    }
}

fn provider_structurally_configured(provider: &crate::core::LlmProvider) -> bool {
    match provider {
        crate::core::LlmProvider::Ollama { base_url, model } => {
            !base_url.trim().is_empty() && !model.trim().is_empty()
        }
        crate::core::LlmProvider::Anthropic { api_key, model } => {
            !api_key.trim().is_empty() && !model.trim().is_empty() && api_key != "[ENCRYPTED]"
        }
        crate::core::LlmProvider::OpenAI {
            api_key,
            model,
            base_url,
        } => {
            if model.trim().is_empty() {
                return false;
            }
            match crate::core::llm_provider::openai_provider_label(base_url.as_deref()) {
                "openai" => !api_key.trim().is_empty() && api_key != "[ENCRYPTED]",
                "openrouter" | "openai-subscription" | "huggingface" => {
                    !api_key.trim().is_empty()
                        && api_key != "[ENCRYPTED]"
                        && base_url.as_ref().is_some_and(|url| !url.trim().is_empty())
                }
                "openai-compatible" => base_url.as_ref().is_some_and(|url| !url.trim().is_empty()),
                _ => false,
            }
        }
    }
}

fn provider_safe_summary(provider: &crate::core::LlmProvider) -> serde_json::Value {
    match provider {
        crate::core::LlmProvider::Anthropic { api_key, model } => serde_json::json!({
            "provider_id": "anthropic",
            "provider_kind": "anthropic",
            "model": model,
            "base_url": serde_json::Value::Null,
            "has_api_key": !api_key.trim().is_empty() && api_key != "[ENCRYPTED]",
            "runtime_configured": provider_structurally_configured(provider),
        }),
        crate::core::LlmProvider::OpenAI {
            api_key,
            model,
            base_url,
        } => serde_json::json!({
            "provider_id": crate::core::llm_provider::openai_provider_label(base_url.as_deref()),
            "provider_kind": "openai_compatible",
            "model": model,
            "base_url": crate::core::llm_provider::display_openai_base_url(base_url.as_ref()),
            "has_api_key": !api_key.trim().is_empty() && api_key != "[ENCRYPTED]",
            "runtime_configured": provider_structurally_configured(provider),
        }),
        crate::core::LlmProvider::Ollama { base_url, model } => serde_json::json!({
            "provider_id": "ollama",
            "provider_kind": "ollama",
            "model": model,
            "base_url": base_url,
            "has_api_key": false,
            "runtime_configured": provider_structurally_configured(provider),
        }),
    }
}

fn model_slot_runtime_ready(slot: &crate::core::config::ModelSlot) -> bool {
    slot.enabled && provider_structurally_configured(&slot.provider)
}

fn model_role_value(role: &crate::core::config::ModelRole) -> serde_json::Value {
    serde_json::to_value(role).unwrap_or_else(|_| serde_json::json!("unknown"))
}

fn model_slot_safe_summary(
    slot: &crate::core::config::ModelSlot,
    primary_slot_id: Option<&str>,
    selected_slot_id: Option<&str>,
) -> serde_json::Value {
    serde_json::json!({
        "id": &slot.id,
        "label": &slot.label,
        "role": model_role_value(&slot.role),
        "enabled": slot.enabled,
        "is_primary": primary_slot_id.is_some_and(|id| id == slot.id.as_str()),
        "is_user_selected": selected_slot_id.is_some_and(|id| id == slot.id.as_str()),
        "runtime_ready": model_slot_runtime_ready(slot),
        "capability_tier": serde_json::to_value(slot.capability_tier).unwrap_or_else(|_| serde_json::json!("unknown")),
        "cost_tier": serde_json::to_value(slot.cost_tier).unwrap_or_else(|_| serde_json::json!("unknown")),
        "auto_escalate": slot.auto_escalate,
        "escalation_rank": slot.escalation_rank,
        "health_scope": serde_json::to_value(slot.health_scope).unwrap_or_else(|_| serde_json::json!("unknown")),
        "provider": provider_safe_summary(&slot.provider),
    })
}

fn resolve_primary_model_slot_id_from_config(
    config: &crate::core::config::AgentConfig,
) -> Option<String> {
    let slots = &config.model_pool.slots;
    for role in [
        crate::core::config::ModelRole::Primary,
        crate::core::config::ModelRole::Fast,
        crate::core::config::ModelRole::Code,
        crate::core::config::ModelRole::Research,
        crate::core::config::ModelRole::Fallback,
    ] {
        if let Some(slot) = slots
            .iter()
            .find(|slot| slot.role == role && model_slot_runtime_ready(slot))
        {
            return Some(slot.id.clone());
        }
    }
    slots
        .iter()
        .find(|slot| model_slot_runtime_ready(slot))
        .or_else(|| {
            slots
                .iter()
                .find(|slot| slot.role == crate::core::config::ModelRole::Primary && slot.enabled)
        })
        .or_else(|| slots.iter().find(|slot| slot.enabled))
        .or_else(|| slots.first())
        .map(|slot| slot.id.clone())
}

async fn persisted_selected_model_slot_id(runtime: &ActionRuntime) -> Option<String> {
    let storage = runtime.runtime_storage().ok()?;
    storage
        .get(crate::core::USER_SELECTED_MODEL_SLOT_KEY)
        .await
        .ok()
        .flatten()
        .and_then(|bytes| String::from_utf8(bytes).ok())
        .map(|value| value.trim().to_string())
        .filter(|value| !value.is_empty())
}

async fn model_failover_status(runtime: &ActionRuntime) -> serde_json::Value {
    let Ok(storage) = runtime.runtime_storage() else {
        return serde_json::json!({
            "available": false,
            "reason": "runtime storage unavailable",
        });
    };
    match crate::core::ModelFailoverControlPlane::list(&storage).await {
        Ok(failover) => serde_json::json!({
            "available": true,
            "summary": &failover.summary,
            "provider_health": &failover.provider_health,
            "fallback_chains": failover.fallback_chains.iter().map(|chain| {
                serde_json::json!({
                    "id": &chain.id,
                    "name": &chain.name,
                    "enabled": chain.enabled,
                    "candidate_count": chain.ordered_candidates.len(),
                    "has_session_pin": chain.session_pin.is_some(),
                })
            }).collect::<Vec<_>>(),
        }),
        Err(error) => serde_json::json!({
            "available": false,
            "error": error.to_string(),
        }),
    }
}

async fn models_surface(runtime: &ActionRuntime) -> Result<serde_json::Value> {
    let config = runtime.settings_manager()?.load()?;
    let selected_slot_id = persisted_selected_model_slot_id(runtime).await;
    let primary_slot_id = resolve_primary_model_slot_id_from_config(&config);
    let selected_slot = selected_slot_id.as_deref().and_then(|id| {
        config
            .model_pool
            .slots
            .iter()
            .find(|slot| slot.id == id && model_slot_runtime_ready(slot))
    });
    let primary_slot = primary_slot_id
        .as_deref()
        .and_then(|id| config.model_pool.slots.iter().find(|slot| slot.id == id));
    let active_slot = selected_slot.or(primary_slot);
    let selection_source = if selected_slot.is_some() {
        "user_selected_slot"
    } else if active_slot.is_some() {
        "primary_or_first_ready_slot"
    } else {
        "legacy_llm"
    };
    let active_provider = active_slot
        .map(|slot| &slot.provider)
        .unwrap_or(&config.llm);
    let active_slot_id = active_slot.map(|slot| slot.id.clone());
    let model_pool_slots = config
        .model_pool
        .slots
        .iter()
        .map(|slot| {
            model_slot_safe_summary(
                slot,
                primary_slot_id.as_deref(),
                selected_slot_id.as_deref(),
            )
        })
        .collect::<Vec<_>>();
    let model_pool_ready_count = config
        .model_pool
        .slots
        .iter()
        .filter(|slot| model_slot_runtime_ready(slot))
        .count();

    Ok(serde_json::json!({
        "surface": "models",
        "generated_at": chrono::Utc::now().to_rfc3339(),
        "evidence_policy": "This surface is safe runtime metadata for answering model/provider status questions. It intentionally excludes credentials, raw config payloads, environment variables, hidden prompts, and internal instructions.",
        "product_identity": crate::branding::PRODUCT_NAME,
        "chat_model_configured": if config.model_pool.slots.is_empty() {
            provider_structurally_configured(&config.llm)
        } else {
            model_pool_ready_count > 0
        },
        "smart_routing": config.model_pool.smart_routing,
        "selection": {
            "source": selection_source,
            "active_slot_id": active_slot_id,
            "primary_slot_id": primary_slot_id,
            "user_selected_slot_id": selected_slot_id,
            "active_model": provider_model_name(active_provider),
            "active_provider": provider_safe_summary(active_provider),
        },
        "legacy_primary": provider_safe_summary(&config.llm),
        "legacy_fallback": config.llm_fallback.as_ref().map(provider_safe_summary),
        "model_pool": {
            "configured_slots": config.model_pool.slots.len(),
            "runtime_ready_slots": model_pool_ready_count,
            "slots": model_pool_slots,
        },
        "failover": model_failover_status(runtime).await,
    }))
}

async fn analytics_surface(runtime: &ActionRuntime) -> Result<serde_json::Value> {
    let payload = internal_api_get(
        runtime,
        &InternalApiInspectRequest {
            path: "/analytics/llm".to_string(),
            method: Some("get".to_string()),
            query: serde_json::json!({
                "range": "30d",
                "bucket": "day",
            }),
            timeout_secs: Some(15),
        },
    )
    .await?;

    Ok(serde_json::json!({
        "surface": "analytics",
        "generated_at": chrono::Utc::now().to_rfc3339(),
        "llm_usage": payload,
    }))
}

async fn activity_surface(
    runtime: &ActionRuntime,
    request: &AgentArkInspectRequest,
) -> Result<serde_json::Value> {
    let storage = runtime.runtime_storage()?;
    let limit = request.limit.unwrap_or(8).clamp(3, 30);
    let message_limit = limit.saturating_mul(2).min(24);
    let now = chrono::Utc::now();
    let reflect_from = (now - chrono::Duration::days(35)).to_rfc3339();
    let reflect_to = now.to_rfc3339();

    let recent_conversations = storage
        .list_conversations(limit, 0, None, &[], None)
        .await?
        .into_iter()
        .map(|conversation| {
            serde_json::json!({
                "id": conversation.id,
                "title": compact_for_activity(&conversation.title, 160),
                "channel": conversation.channel,
                "created_at": conversation.created_at,
                "updated_at": conversation.updated_at,
                "message_count": conversation.message_count,
                "starred": conversation.starred,
            })
        })
        .collect::<Vec<_>>();

    let recent_user_messages = storage
        .get_recent_user_messages(message_limit)
        .await?
        .into_iter()
        .map(|message| {
            serde_json::json!({
                "conversation_id": message.conversation_id,
                "timestamp": message.timestamp,
                "content": compact_for_activity(&message.content, 360),
            })
        })
        .collect::<Vec<_>>();

    let recent_tasks = storage
        .get_tasks()
        .await?
        .into_iter()
        .take(limit as usize)
        .map(|task| {
            serde_json::json!({
                "id": task.id,
                "description": compact_for_activity(&task.description, 260),
                "action": task.action,
                "status": task.status,
                "created_at": task.created_at,
                "scheduled_for": task.scheduled_for,
                "cron": task.cron,
                "priority": task.priority,
                "urgency": task.urgency,
                "importance": task.importance,
                "eisenhower_quadrant": task.eisenhower_quadrant,
            })
        })
        .collect::<Vec<_>>();

    let mut watcher_rows = storage.list_watchers().await.unwrap_or_default();
    watcher_rows.sort_by(|left, right| right.created_at.cmp(&left.created_at));
    let recent_watchers = watcher_rows
        .into_iter()
        .take(limit as usize)
        .map(|watcher| {
            serde_json::json!({
                "id": watcher.id,
                "description": compact_for_activity(&watcher.description, 260),
                "poll_action": watcher.poll_action,
                "condition": compact_for_activity(&watcher.condition.summary(), 260),
                "on_trigger": compact_for_activity(&watcher.on_trigger, 260),
                "interval_secs": watcher.interval_secs,
                "timeout_secs": watcher.timeout_secs,
                "notify_channel": watcher.notify_channel,
                "repeat_on_match": watcher.repeat_on_match,
                "status": watcher.status,
                "created_at": watcher.created_at,
                "last_poll_at": watcher.last_poll_at,
                "poll_count": watcher.poll_count,
                "last_poll_outcome": watcher.last_poll_outcome,
                "last_error": watcher.last_error.map(|value| compact_for_activity(&value, 260)),
            })
        })
        .collect::<Vec<_>>();

    let recent_background_sessions = storage
        .list_background_sessions()
        .await
        .unwrap_or_default()
        .into_iter()
        .take(limit as usize)
        .map(|session| {
            serde_json::json!({
                "id": session.id,
                "title": compact_for_activity(&session.title, 180),
                "objective": compact_for_activity(&session.objective, 320),
                "status": session.status,
                "summary": session.summary.map(|value| compact_for_activity(&value, 320)),
                "current_focus": session.current_focus.map(|value| compact_for_activity(&value, 260)),
                "waiting_on": session.waiting_on.map(|value| compact_for_activity(&value, 220)),
                "last_error": session.last_error.map(|value| compact_for_activity(&value, 260)),
                "preferred_delivery_channel": session.preferred_delivery_channel,
                "created_at": session.created_at,
                "updated_at": session.updated_at,
                "last_activity_at": session.last_activity_at,
                "linked_task_ids": session.linked_task_ids,
                "linked_watcher_ids": session.linked_watcher_ids,
            })
        })
        .collect::<Vec<_>>();

    let experience_items = storage
        .list_active_experience_items_any_scope(
            &["constraint", "personal_fact", "lesson", "procedure"],
            limit,
        )
        .await
        .unwrap_or_default()
        .into_iter()
        .map(|item| {
            serde_json::json!({
                "id": item.id,
                "kind": item.kind,
                "scope": item.scope,
                "title": compact_for_activity(&item.title, 180),
                "content": compact_for_activity(&item.content, 420),
                "confidence": item.confidence,
                "support_count": item.support_count,
                "status": item.status,
                "updated_at": item.updated_at,
            })
        })
        .collect::<Vec<_>>();
    let procedural_patterns = storage
        .list_procedural_patterns_any_scope(&["active", "draft"], limit)
        .await
        .unwrap_or_default()
        .into_iter()
        .map(|pattern| {
            serde_json::json!({
                "id": pattern.id,
                "status": pattern.status,
                "title": compact_for_activity(&pattern.title, 180),
                "trigger_summary": compact_for_activity(&pattern.trigger_summary, 300),
                "summary": compact_for_activity(&pattern.summary, 420),
                "sample_count": pattern.sample_count,
                "success_count": pattern.success_count,
                "correction_count": pattern.correction_count,
                "success_rate": pattern.success_rate,
                "updated_at": pattern.updated_at,
            })
        })
        .collect::<Vec<_>>();
    let arkreflect_units = storage
        .list_semantic_work_units_between(&reflect_from, &reflect_to, limit.saturating_mul(3))
        .await
        .unwrap_or_default()
        .into_iter()
        .take(limit as usize)
        .map(|unit| {
            serde_json::json!({
                "id": unit.id,
                "source_kind": unit.source_kind,
                "source_id": compact_for_activity(&unit.source_id, 160),
                "conversation_id": unit.conversation_id,
                "project_id": unit.project_id,
                "channel": unit.channel,
                "title": compact_for_activity(&unit.title, 180),
                "summary": compact_for_activity(&unit.summary, 420),
                "content_preview": compact_for_activity(&unit.content_preview, 420),
                "occurred_at": unit.occurred_at,
                "message_count": unit.message_count,
                "has_embedding": unit.embedding.is_some(),
                "metadata": unit.metadata,
            })
        })
        .collect::<Vec<_>>();
    let arkreflect_experience_runs = storage
        .list_recent_experience_runs_any_scope(limit)
        .await
        .unwrap_or_default()
        .into_iter()
        .map(|run| {
            serde_json::json!({
                "id": run.id,
                "trace_id": run.trace_id,
                "conversation_id": run.conversation_id,
                "project_id": run.project_id,
                "channel": run.channel,
                "scope": run.scope,
                "intent_key": compact_for_activity(&run.intent_key, 180),
                "task_type": run.task_type.map(|value| compact_for_activity(&value, 120)),
                "request_text": run.request_text.map(|value| compact_for_activity(&value, 360)),
                "success_state": run.success_state,
                "correction_state": run.correction_state,
                "outcome_summary": run.outcome_summary.map(|value| compact_for_activity(&value, 360)),
                "failure_reason": run.failure_reason.map(|value| compact_for_activity(&value, 300)),
                "consolidated": run.consolidated,
                "heuristic_reflected": run.heuristic_reflected,
                "heuristic_reflection_status": run.heuristic_reflection_status,
                "created_at": run.created_at,
                "updated_at": run.updated_at,
            })
        })
        .collect::<Vec<_>>();
    let sentinel = runtime
        .inspect_sentinel_json(&storage, limit)
        .await
        .unwrap_or_else(|error| serde_json::json!({ "error": error.to_string() }));
    let evolution = runtime
        .inspect_evolution_json(&storage, limit)
        .await
        .unwrap_or_else(|error| serde_json::json!({ "error": error.to_string() }));

    Ok(serde_json::json!({
        "surface": "activity",
        "generated_at": chrono::Utc::now().to_rfc3339(),
        "evidence_policy": "Use this read-only local evidence to answer reflective questions about recent activity, work patterns, recurring interests, blockers, habits, focus, avoidance, inferred mindset, or follow-through. Treat it as evidence, not proof; avoid overclaiming when the evidence is thin.",
        "recent_conversations": recent_conversations,
        "recent_user_messages": recent_user_messages,
        "recent_tasks": recent_tasks,
        "recent_watchers": recent_watchers,
        "recent_background_sessions": recent_background_sessions,
        "memory_and_learning": {
            "active_experience_items": experience_items,
            "procedural_patterns": procedural_patterns,
            "evolution": evolution,
        },
        "arkreflect": {
            "window": {
                "from": reflect_from,
                "to": reflect_to,
            },
            "semantic_work_units": arkreflect_units,
            "recent_experience_runs": arkreflect_experience_runs,
        },
        "sentinel": sentinel,
    }))
}

fn json_value_to_http_string(value: &serde_json::Value) -> Option<String> {
    match value {
        serde_json::Value::Null => None,
        serde_json::Value::String(text) => Some(text.clone()),
        serde_json::Value::Bool(value) => Some(value.to_string()),
        serde_json::Value::Number(value) => Some(value.to_string()),
        other => serde_json::to_string(other).ok(),
    }
}

fn add_query_pairs(url: &mut reqwest::Url, query: &serde_json::Value) {
    let Some(query) = query.as_object() else {
        return;
    };
    let mut pairs = url.query_pairs_mut();
    for (key, value) in query {
        if let Some(items) = value.as_array() {
            for item in items {
                if let Some(rendered) = json_value_to_http_string(item) {
                    pairs.append_pair(key, &rendered);
                }
            }
        } else if let Some(rendered) = json_value_to_http_string(value) {
            pairs.append_pair(key, &rendered);
        }
    }
}

fn redact_json(value: serde_json::Value) -> serde_json::Value {
    crate::security::redact_json_secrets(&value)
}

fn redact_text(value: &str) -> String {
    crate::security::sanitize_untrusted_output(
        "agentark_internal_api",
        &crate::security::redact_secret_input(value).text,
    )
}

async fn internal_api_get(
    runtime: &ActionRuntime,
    api: &InternalApiInspectRequest,
) -> Result<serde_json::Value> {
    let method = api
        .method
        .as_deref()
        .map(normalized_key)
        .unwrap_or_default();
    if !method.is_empty() && method != "get" {
        anyhow::bail!("ark_inspect internal API access is read-only and only supports GET");
    }

    let path = api.path.trim();
    if path.is_empty() {
        anyhow::bail!("Missing internal API path");
    }
    if path.contains("://") || path.starts_with("//") {
        anyhow::bail!("Internal API inspection accepts only AgentArk-relative paths");
    }

    let base_url = crate::core::net::internal_api_base_url();
    let mut url = reqwest::Url::parse(&base_url)
        .context("failed to parse internal AgentArk base URL")?
        .join(path.trim_start_matches('/'))
        .context("failed to build internal AgentArk API URL")?;
    add_query_pairs(&mut url, &api.query);

    let manager = crate::core::config::SecureConfigManager::new_with_data_dir(
        &runtime.config_dir,
        Some(runtime.data_dir()),
    )?;
    let key = manager
        .get_api_key()?
        .map(|value| value.trim().to_string())
        .filter(|value| !value.is_empty())
        .ok_or_else(|| anyhow::anyhow!("AgentArk HTTP API key is not configured"))?;

    let client = crate::core::net::build_internal_control_client(api.timeout_secs.unwrap_or(10))?;
    let response = client
        .get(url.clone())
        .bearer_auth(key)
        .header("Accept", "application/json, text/plain;q=0.8, */*;q=0.5")
        .send()
        .await
        .context("internal AgentArk API request failed")?;
    let status = response.status();
    let content_type = response
        .headers()
        .get(reqwest::header::CONTENT_TYPE)
        .and_then(|value| value.to_str().ok())
        .unwrap_or("")
        .to_string();
    let body = response.bytes().await.unwrap_or_default();
    let truncated = body.len() > super::HTTP_GET_MAX_BODY_BYTES;
    let body_slice = if truncated {
        &body[..super::HTTP_GET_MAX_BODY_BYTES]
    } else {
        &body[..]
    };
    let body_text = String::from_utf8_lossy(body_slice).to_string();
    let body_json = if content_type.contains("json") {
        serde_json::from_str::<serde_json::Value>(&body_text)
            .ok()
            .map(redact_json)
    } else {
        None
    };

    Ok(serde_json::json!({
        "operation": "api_get",
        "path": url.path(),
        "query": url.query(),
        "status": status.as_u16(),
        "success": status.is_success(),
        "content_type": content_type,
        "truncated": truncated,
        "body": body_json.unwrap_or_else(|| serde_json::Value::String(redact_text(&body_text))),
    }))
}

fn openapi_string_field(value: &serde_json::Value, key: &str, max_chars: usize) -> Option<String> {
    value
        .get(key)
        .and_then(|value| value.as_str())
        .map(|value| compact_for_activity(value, max_chars))
        .filter(|value| !value.trim().is_empty())
}

fn summarize_openapi_schema(schema: &serde_json::Value) -> serde_json::Value {
    let Some(schema) = schema.as_object() else {
        return serde_json::Value::Null;
    };
    let mut summary = serde_json::Map::new();
    for key in ["type", "format", "default"] {
        if let Some(value) = schema.get(key) {
            summary.insert(key.to_string(), value.clone());
        }
    }
    if let Some(values) = schema.get("enum").and_then(|value| value.as_array()) {
        summary.insert("enum".to_string(), serde_json::Value::Array(values.clone()));
    }
    if let Some(items) = schema.get("items").and_then(|value| value.as_object()) {
        let mut item_summary = serde_json::Map::new();
        for key in ["type", "format"] {
            if let Some(value) = items.get(key) {
                item_summary.insert(key.to_string(), value.clone());
            }
        }
        if !item_summary.is_empty() {
            summary.insert("items".to_string(), serde_json::Value::Object(item_summary));
        }
    }
    for key in ["oneOf", "anyOf", "allOf"] {
        if let Some(values) = schema.get(key).and_then(|value| value.as_array()) {
            summary.insert(
                key.to_string(),
                serde_json::json!({ "variants": values.len() }),
            );
        }
    }
    if summary.is_empty() {
        serde_json::Value::Null
    } else {
        serde_json::Value::Object(summary)
    }
}

fn summarize_openapi_parameter(parameter: &serde_json::Value) -> serde_json::Value {
    if let Some(reference) = parameter.get("$ref").and_then(|value| value.as_str()) {
        return serde_json::json!({ "ref": reference });
    }
    let mut summary = serde_json::Map::new();
    for key in ["name", "in"] {
        if let Some(value) = parameter.get(key).and_then(|value| value.as_str()) {
            summary.insert(
                key.to_string(),
                serde_json::Value::String(value.to_string()),
            );
        }
    }
    if let Some(required) = parameter.get("required").and_then(|value| value.as_bool()) {
        summary.insert("required".to_string(), serde_json::Value::Bool(required));
    }
    if let Some(description) = openapi_string_field(parameter, "description", 220) {
        summary.insert(
            "description".to_string(),
            serde_json::Value::String(description),
        );
    }
    let schema =
        summarize_openapi_schema(parameter.get("schema").unwrap_or(&serde_json::Value::Null));
    if !schema.is_null() {
        summary.insert("schema".to_string(), schema);
    }
    serde_json::Value::Object(summary)
}

fn summarize_openapi_tags(body: &serde_json::Value) -> Vec<serde_json::Value> {
    body.get("tags")
        .and_then(|value| value.as_array())
        .map(|tags| {
            tags.iter()
                .filter_map(|tag| {
                    let name = tag.get("name").and_then(|value| value.as_str())?;
                    Some(serde_json::json!({
                        "name": name,
                        "description": openapi_string_field(tag, "description", 180),
                    }))
                })
                .collect::<Vec<_>>()
        })
        .unwrap_or_default()
}

async fn api_catalog(
    runtime: &ActionRuntime,
    request: &AgentArkInspectRequest,
) -> Result<serde_json::Value> {
    let source_api = request.api.as_ref();
    let source_path = source_api
        .map(|api| api.path.trim())
        .filter(|path| !path.is_empty())
        .unwrap_or("/openapi.json");
    let payload = internal_api_get(
        runtime,
        &InternalApiInspectRequest {
            path: source_path.to_string(),
            method: Some("get".to_string()),
            query: source_api
                .map(|api| api.query.clone())
                .unwrap_or(serde_json::Value::Null),
            timeout_secs: source_api.and_then(|api| api.timeout_secs).or(Some(15)),
        },
    )
    .await?;
    let body = payload
        .get("body")
        .ok_or_else(|| anyhow::anyhow!("AgentArk API catalog response did not include a body"))?;
    let paths = body
        .get("paths")
        .and_then(|value| value.as_object())
        .ok_or_else(|| {
            anyhow::anyhow!("AgentArk API catalog response is not an OpenAPI document")
        })?;
    let limit = request.limit.unwrap_or(160).clamp(20, 240) as usize;
    let mut endpoints = Vec::new();
    for (path, path_item) in paths {
        let Some(methods) = path_item.as_object() else {
            continue;
        };
        for (method, operation) in methods {
            if !method.eq_ignore_ascii_case("get") {
                continue;
            }
            let tags = operation
                .get("tags")
                .and_then(|value| value.as_array())
                .map(|values| {
                    values
                        .iter()
                        .filter_map(|value| value.as_str().map(str::to_string))
                        .collect::<Vec<_>>()
                })
                .unwrap_or_default();
            let parameters = operation
                .get("parameters")
                .and_then(|value| value.as_array())
                .map(|values| {
                    values
                        .iter()
                        .map(summarize_openapi_parameter)
                        .collect::<Vec<_>>()
                })
                .unwrap_or_default();
            endpoints.push(serde_json::json!({
                "method": "GET",
                "path": path,
                "operation_id": openapi_string_field(operation, "operationId", 140),
                "summary": openapi_string_field(operation, "summary", 220),
                "description": openapi_string_field(operation, "description", 360),
                "tags": tags,
                "parameters": parameters,
            }));
        }
    }
    endpoints.sort_by(|left, right| {
        let left_tag = left
            .get("tags")
            .and_then(|value| value.as_array())
            .and_then(|values| values.first())
            .and_then(|value| value.as_str())
            .unwrap_or("");
        let right_tag = right
            .get("tags")
            .and_then(|value| value.as_array())
            .and_then(|values| values.first())
            .and_then(|value| value.as_str())
            .unwrap_or("");
        left_tag.cmp(right_tag).then_with(|| {
            left.get("path")
                .and_then(|value| value.as_str())
                .unwrap_or("")
                .cmp(
                    right
                        .get("path")
                        .and_then(|value| value.as_str())
                        .unwrap_or(""),
                )
        })
    });
    let total_readonly_endpoints = endpoints.len();
    let truncated = endpoints.len() > limit;
    endpoints.truncate(limit);
    Ok(serde_json::json!({
        "operation": "api_catalog",
        "generated_at": chrono::Utc::now().to_rfc3339(),
        "source_path": payload.get("path").cloned().unwrap_or_else(|| serde_json::json!(source_path)),
        "total_readonly_endpoints": total_readonly_endpoints,
        "returned_endpoints": endpoints.len(),
        "truncated": truncated,
        "evidence_policy": "This is the live read-only AgentArk API catalog. For AgentArk-owned data, reports, status, analytics, or registry questions, choose the relevant GET endpoint from this catalog and call operation=api_get. Prefer API reads before database reads when an API surface exists.",
        "tags": summarize_openapi_tags(body),
        "endpoints": endpoints,
    }))
}

fn table_query_from_request(
    request: &AgentArkInspectRequest,
) -> Result<crate::storage::ReadonlyTableQuery> {
    let value = request
        .database
        .clone()
        .ok_or_else(|| anyhow::anyhow!("Missing database query object"))?;
    serde_json::from_value(value).context("invalid structured database query")
}

async fn file_read(
    runtime: &ActionRuntime,
    file: &FileInspectRequest,
) -> Result<serde_json::Value> {
    let path = runtime.resolve_tool_read_path(&file.path)?;
    let content = tokio::fs::read_to_string(&path).await?;
    let max_chars = file.max_chars.unwrap_or(20_000).clamp(1_000, 80_000);
    let redacted = crate::security::redact_secret_input(&content).text;
    let char_count = redacted.chars().count();
    let truncated = char_count > max_chars;
    let content = if truncated {
        redacted.chars().take(max_chars).collect::<String>()
    } else {
        redacted
    };
    Ok(serde_json::json!({
        "operation": "file_read",
        "path": path.display().to_string(),
        "truncated": truncated,
        "content": crate::security::sanitize_untrusted_output("agentark_file", &content),
    }))
}

async fn surface(
    runtime: &ActionRuntime,
    request: &AgentArkInspectRequest,
) -> Result<serde_json::Value> {
    let storage = runtime.runtime_storage()?;
    let limit = request.limit.unwrap_or(6).clamp(1, 24);
    let trace_id = request.trace_id.as_deref();
    let surface = surface_name(request);
    match surface.as_str() {
        "overview" => Ok(serde_json::json!({
            "surface": "overview",
            "generated_at": chrono::Utc::now().to_rfc3339(),
            "analytics": match analytics_surface(runtime).await {
                Ok(value) => value,
                Err(error) => serde_json::json!({
                    "surface": "analytics",
                    "error": error.to_string(),
                }),
            },
            "gateway_ops": runtime.inspect_gateway_ops_json(&storage, limit).await?,
            "arkpulse": runtime.inspect_arkpulse_json(&storage, limit).await?,
            "sentinel": runtime.inspect_sentinel_json(&storage, limit).await?,
            "evolution": runtime.inspect_evolution_json(&storage, limit).await?,
            "trace": runtime.inspect_trace_json(&storage, None, limit).await?,
            "moltbook": runtime.inspect_moltbook_json(&storage, limit).await?,
            "models": models_surface(runtime).await?,
        })),
        "apps" | "app_registry" | "deployed_apps" => apps_surface(runtime, request).await,
        "analytics" => analytics_surface(runtime).await,
        "models" | "model_runtime" | "model_status" | "provider_status" | "llm" | "llms" => {
            models_surface(runtime).await
        }
        "activity" | "personal_activity" | "activity_insights" => {
            activity_surface(runtime, request).await
        }
        "gateway_ops" => runtime.inspect_gateway_ops_json(&storage, limit).await,
        "arkpulse" => runtime.inspect_arkpulse_json(&storage, limit).await,
        "sentinel" => runtime.inspect_sentinel_json(&storage, limit).await,
        "evolution" => runtime.inspect_evolution_json(&storage, limit).await,
        "trace" => runtime.inspect_trace_json(&storage, trace_id, limit).await,
        "moltbook" => runtime.inspect_moltbook_json(&storage, limit).await,
        other => Ok(serde_json::json!({
            "surface": other,
            "generated_at": chrono::Utc::now().to_rfc3339(),
            "notice": "No built-in summary surface matched this requested surface. Use the read-only API catalog to choose the relevant AgentArk endpoint instead of guessing database tables.",
            "api_catalog": api_catalog(runtime, request).await?,
        })),
    }
}

pub(super) async fn execute(
    runtime: &ActionRuntime,
    arguments: &serde_json::Value,
) -> Result<String> {
    let request: AgentArkInspectRequest =
        serde_json::from_value(arguments.clone()).context("invalid ark_inspect arguments")?;
    let payload = match inspect_operation(&request).as_str() {
        "surface" | "overview" => surface(runtime, &request).await?,
        "api_catalog" | "api_discover" | "openapi" | "openapi_catalog" => {
            api_catalog(runtime, &request).await?
        }
        "api" | "api_get" | "internal_api" | "internal_api_get" => {
            let api = request
                .api
                .as_ref()
                .ok_or_else(|| anyhow::anyhow!("Missing api request object"))?;
            internal_api_get(runtime, api).await?
        }
        "db_schema" | "database_schema" | "postgres_schema" => {
            let storage = runtime.runtime_storage()?;
            storage
                .inspect_postgres_schema_json(
                    request.table_filter.as_deref(),
                    request.limit.unwrap_or(25),
                )
                .await?
        }
        "db_query" | "database_query" | "postgres_query" => {
            let storage = runtime.runtime_storage()?;
            let query = table_query_from_request(&request)?;
            storage.query_table_json(&query).await?
        }
        "file" | "file_read" | "filesystem" => {
            let file = request
                .file
                .as_ref()
                .ok_or_else(|| anyhow::anyhow!("Missing file request object"))?;
            file_read(runtime, file).await?
        }
        other => {
            anyhow::bail!(
                "Unsupported ark_inspect operation '{}'. Use surface, api_catalog, api_get, db_schema, db_query, or file_read.",
                other
            )
        }
    };
    Ok(serde_json::to_string_pretty(&redact_json(payload))?)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn action_definition_exposes_safe_model_runtime_surface() {
        let action = action_def();
        let rendered = serde_json::to_string(&action.input_schema).expect("schema should render");

        assert!(
            action
                .description
                .contains("current model/provider selection")
        );
        assert!(action.capabilities.contains(&"model_runtime".to_string()));
        assert!(rendered.contains("\"models\""));
        assert!(rendered.contains("current model/provider selection"));
    }

    #[test]
    fn provider_summary_excludes_secret_values() {
        let provider = crate::core::LlmProvider::OpenAI {
            api_key: "sk-test-secret".to_string(),
            model: "gpt-test".to_string(),
            base_url: None,
        };

        let summary = provider_safe_summary(&provider);
        let rendered = serde_json::to_string(&summary).expect("summary should render");
        assert!(rendered.contains("gpt-test"));
        assert!(rendered.contains("\"has_api_key\":true"));
        assert!(!rendered.contains("sk-test-secret"));
    }
}
