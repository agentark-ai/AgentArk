//! HTTP control surface for filesystem-backed ArkOrbit.

use super::*;

use crate::core::arkorbit::{
    OrbitAgentEvent, OrbitChatUsage, OrbitUpdate, content_type_for_name, stream_orbit_chat_turn,
};
use std::collections::BTreeSet;
use std::sync::Arc;
use tokio::sync::RwLock;

fn current_user_id(agent: &crate::core::Agent) -> String {
    agent.identity.did().to_string()
}

fn json_error(status: StatusCode, message: impl Into<String>) -> Response {
    (
        status,
        Json(ErrorResponse {
            error: message.into(),
        }),
    )
        .into_response()
}

fn internal_error(err: impl std::fmt::Display) -> Response {
    json_error(StatusCode::INTERNAL_SERVER_ERROR, err.to_string())
}

fn optional_string(body: &serde_json::Value, key: &str) -> Option<String> {
    body.get(key)
        .and_then(|v| v.as_str())
        .map(|s| s.trim().to_string())
        .filter(|s| !s.is_empty())
}

const ORBIT_INDEX_CSP: &str = concat!(
    "default-src 'none'; ",
    "base-uri 'none'; ",
    "object-src 'none'; ",
    "frame-ancestors 'self'; ",
    "script-src 'self' 'unsafe-inline' blob:; ",
    "style-src 'self' 'unsafe-inline' https:; ",
    "img-src 'self' data: blob: https:; ",
    "font-src 'self' data: https:; ",
    "connect-src 'self' https: wss:; ",
    "media-src 'self' data: blob: https:; ",
    "frame-src https:; ",
    "worker-src blob:; ",
    "form-action 'self' https:; ",
    "sandbox allow-scripts allow-forms allow-modals"
);

fn set_static_header(response: &mut Response, name: &'static str, value: &'static str) {
    response.headers_mut().insert(
        HeaderName::from_static(name),
        HeaderValue::from_static(value),
    );
}

fn set_orbit_security_headers(mut response: Response, content_type: &str) -> Response {
    if let Ok(value) = HeaderValue::from_str(content_type) {
        response.headers_mut().insert(header::CONTENT_TYPE, value);
    }
    set_static_header(&mut response, "cache-control", "no-store");
    set_static_header(&mut response, "x-content-type-options", "nosniff");
    set_static_header(&mut response, "referrer-policy", "no-referrer");
    set_static_header(
        &mut response,
        "permissions-policy",
        "camera=(), microphone=(), geolocation=(), payment=(), usb=(), serial=(), bluetooth=(), accelerometer=(), gyroscope=(), magnetometer=()",
    );
    response
}

fn set_orbit_index_headers(response: Response) -> Response {
    let mut response = set_orbit_security_headers(response, "text/html; charset=utf-8");
    set_static_header(&mut response, "content-security-policy", ORBIT_INDEX_CSP);
    response
}

fn set_event_stream_headers(mut response: Response) -> Response {
    set_static_header(&mut response, "cache-control", "no-store");
    set_static_header(&mut response, "x-content-type-options", "nosniff");
    response
}

const ORBIT_TRACE_CHANNEL: &str = "arkorbit";
const ORBIT_TRACE_SOURCE_LABEL: &str = "Orbit";
const ORBIT_TRACE_MAX_STATUS_STEPS: usize = 8;
const ORBIT_PUBLIC_FETCH_MAX_BYTES: usize = 2 * 1024 * 1024;

#[derive(Default)]
struct OrbitTraceAccumulator {
    response: String,
    steps: Vec<crate::core::ExecutionStep>,
    status_steps: usize,
    file_write_count: usize,
    read_count: usize,
    error: Option<String>,
    usage: Option<OrbitChatUsage>,
}

fn truncate_trace_value(value: &str, max_chars: usize) -> String {
    if max_chars == 0 {
        return String::new();
    }
    if value.chars().count() <= max_chars {
        return value.to_string();
    }
    let mut truncated = value.chars().take(max_chars).collect::<String>();
    truncated.push_str("...");
    truncated
}

fn orbit_trace_duration_ms(
    started_at: chrono::DateTime<chrono::Utc>,
    timestamp: chrono::DateTime<chrono::Utc>,
) -> u64 {
    timestamp
        .signed_duration_since(started_at)
        .num_milliseconds()
        .max(0) as u64
}

fn orbit_trace_step(
    icon: &str,
    title: &str,
    detail: String,
    step_type: &str,
    data: Option<String>,
    started_at: chrono::DateTime<chrono::Utc>,
) -> crate::core::ExecutionStep {
    let timestamp = chrono::Utc::now();
    crate::core::ExecutionStep {
        icon: icon.to_string(),
        title: title.to_string(),
        detail,
        step_type: step_type.to_string(),
        data,
        timestamp,
        duration_ms: Some(orbit_trace_duration_ms(started_at, timestamp)),
    }
}

impl OrbitTraceAccumulator {
    fn record_event(&mut self, event: &OrbitAgentEvent, started_at: chrono::DateTime<chrono::Utc>) {
        match event {
            OrbitAgentEvent::Status { message } => {
                if self.status_steps < ORBIT_TRACE_MAX_STATUS_STEPS {
                    self.status_steps += 1;
                    self.steps.push(orbit_trace_step(
                        "[orbit]",
                        "Orbit Progress",
                        truncate_trace_value(message, 240),
                        "info",
                        None,
                        started_at,
                    ));
                }
            }
            OrbitAgentEvent::Token(content) => {
                self.response.push_str(content);
            }
            OrbitAgentEvent::FileWritten { path, operation } => {
                self.file_write_count += 1;
                self.steps.push(orbit_trace_step(
                    "[file]",
                    "Orbit File Updated",
                    format!("{} {}", operation.as_str(), path),
                    "success",
                    Some(
                        serde_json::json!({
                            "source": ORBIT_TRACE_SOURCE_LABEL,
                            "path": path,
                            "operation": operation.as_str(),
                        })
                        .to_string(),
                    ),
                    started_at,
                ));
            }
            OrbitAgentEvent::ReadRequested { path } => {
                self.read_count += 1;
                self.steps.push(orbit_trace_step(
                    "[read]",
                    "Orbit File Read",
                    format!("Read {}", path),
                    "info",
                    Some(
                        serde_json::json!({
                            "source": ORBIT_TRACE_SOURCE_LABEL,
                            "path": path,
                        })
                        .to_string(),
                    ),
                    started_at,
                ));
            }
            OrbitAgentEvent::Usage(usage) => {
                self.usage = Some(usage.clone());
            }
            OrbitAgentEvent::Error(message) => {
                self.error = Some(message.clone());
                self.steps.push(orbit_trace_step(
                    "[error]",
                    "Orbit Error",
                    truncate_trace_value(message, 500),
                    "warning",
                    None,
                    started_at,
                ));
            }
            OrbitAgentEvent::Done => {}
        }
    }
}

async fn mirror_orbit_trace_snapshot(state: &AppState, trace_ref: &Arc<RwLock<ExecutionTrace>>) {
    let snapshot = trace_ref.read().await.clone();
    if snapshot.id.trim().is_empty() {
        return;
    }
    *state.last_trace.write().await = snapshot.clone();
    let mut history = state.trace_history.write().await;
    history.retain(|item| item.id != snapshot.id);
    history.insert(0, snapshot);
    if history.len() > 100 {
        history.truncate(100);
    }
}

async fn persist_orbit_trace(
    agent_ref: Arc<RwLock<crate::core::Agent>>,
    trace_ref: Arc<RwLock<ExecutionTrace>>,
    accumulator: OrbitTraceAccumulator,
    completed: bool,
    started_at: chrono::DateTime<chrono::Utc>,
) {
    let completed_at = chrono::Utc::now();
    let duration_ms = orbit_trace_duration_ms(started_at, completed_at);
    let failed = accumulator.error.is_some() || !completed;
    let fallback_response = if let Some(error) = accumulator.error.as_deref() {
        error.to_string()
    } else if completed {
        "Orbit chat completed.".to_string()
    } else {
        "Orbit chat ended before a completion event was received.".to_string()
    };
    let response = accumulator
        .response
        .trim()
        .to_string()
        .chars()
        .take(16_000)
        .collect::<String>();
    let response = if response.trim().is_empty() {
        fallback_response
    } else {
        response
    };
    let usage = accumulator.usage;
    let file_write_count = accumulator.file_write_count;
    let read_count = accumulator.read_count;
    let steps = accumulator.steps;

    {
        let mut trace = trace_ref.write().await;
        trace.completed_at = Some(completed_at);
        trace.response = Some(response.clone());
        if let Some(usage) = usage {
            trace.model = usage.model;
            trace.input_tokens = usage.input_tokens.min(i64::MAX as u64) as i64;
            trace.output_tokens = usage.output_tokens.min(i64::MAX as u64) as i64;
            trace.total_tokens = usage.total_tokens.min(i64::MAX as u64) as i64;
            trace.cost_usd = usage.cost_usd.unwrap_or(0.0);
        }
        trace.steps.extend(steps);
        trace.steps.push(crate::core::ExecutionStep {
            icon: if failed { "[error]" } else { "[reply]" }.to_string(),
            title: if failed {
                "Orbit Turn Failed".to_string()
            } else {
                "Orbit Response".to_string()
            },
            detail: format!(
                "{} turn {} after {}ms ({} file update{}, {} read{}).",
                ORBIT_TRACE_SOURCE_LABEL,
                if failed { "failed" } else { "completed" },
                duration_ms,
                file_write_count,
                if file_write_count == 1 { "" } else { "s" },
                read_count,
                if read_count == 1 { "" } else { "s" }
            ),
            step_type: if failed { "warning" } else { "success" }.to_string(),
            data: Some(truncate_trace_value(&response, 8000)),
            timestamp: completed_at,
            duration_ms: Some(duration_ms),
        });
        trace.complexity = Some("orbit_chat".to_string());
    }

    let agent = crate::core::Agent::snapshot(&agent_ref).await;
    agent.persist_completed_trace(&trace_ref).await;
}

fn is_orbit_reload_event(kind: &notify::EventKind) -> bool {
    match kind {
        notify::EventKind::Create(_) | notify::EventKind::Remove(_) => true,
        notify::EventKind::Modify(modify) => matches!(
            modify,
            notify::event::ModifyKind::Data(_)
                | notify::event::ModifyKind::Name(_)
                | notify::event::ModifyKind::Any
                | notify::event::ModifyKind::Other
        ),
        notify::EventKind::Access(_) | notify::EventKind::Any | notify::EventKind::Other => false,
    }
}

fn normalize_widget_registry_key(value: &str) -> String {
    value
        .trim()
        .trim_start_matches("mod/")
        .trim_end_matches("/index.js")
        .to_string()
}

fn valid_widget_registry_key(value: &str) -> bool {
    !value.is_empty()
        && value.len() <= 128
        && value
            .bytes()
            .all(|byte| byte.is_ascii_alphanumeric() || matches!(byte, b'-' | b'_'))
}

fn widget_registry_key(value: &str) -> Option<String> {
    let normalized = normalize_widget_registry_key(value);
    valid_widget_registry_key(&normalized).then_some(normalized)
}

fn widget_entry_key(widget: &serde_json::Value, field: &str) -> Option<String> {
    widget
        .get(field)
        .and_then(|value| value.as_str())
        .and_then(widget_registry_key)
}

fn widget_entry_module_key(widget: &serde_json::Value) -> Option<String> {
    widget_entry_key(widget, "module").or_else(|| widget_entry_key(widget, "id"))
}

fn widget_module_still_registered(widgets: &[serde_json::Value], module: &str) -> bool {
    widgets.iter().any(|widget| {
        widget_entry_key(widget, "module").as_deref() == Some(module)
            || widget_entry_key(widget, "id").as_deref() == Some(module)
    })
}

pub(super) async fn list_orbits_endpoint(State(state): State<AppState>) -> Response {
    let agent = state.agent.read().await;
    let user_id = current_user_id(&agent);

    match agent.arkorbit.list_orbits(&user_id).await {
        Ok(orbits) => (
            StatusCode::OK,
            Json(serde_json::json!({ "orbits": orbits })),
        )
            .into_response(),
        Err(err) => internal_error(err),
    }
}

pub(super) async fn create_orbit_endpoint(
    State(state): State<AppState>,
    Json(body): Json<serde_json::Value>,
) -> Response {
    let name = body
        .get("name")
        .and_then(|v| v.as_str())
        .map(|s| s.trim())
        .filter(|s| !s.is_empty());
    let Some(name) = name else {
        return json_error(StatusCode::BAD_REQUEST, "'name' is required");
    };

    let icon = optional_string(&body, "icon");
    let color = optional_string(&body, "color");
    let agent_instructions = optional_string(&body, "agent_instructions");

    let agent = state.agent.read().await;
    let user_id = current_user_id(&agent);
    match agent
        .arkorbit
        .create_orbit(&user_id, name, icon, color, agent_instructions)
        .await
    {
        Ok(orbit) => (StatusCode::OK, Json(serde_json::json!({ "orbit": orbit }))).into_response(),
        Err(err) => internal_error(err),
    }
}

pub(super) async fn get_orbit_endpoint(
    State(state): State<AppState>,
    Path(id): Path<String>,
) -> Response {
    let agent = state.agent.read().await;
    match agent.arkorbit.get_orbit(&id).await {
        Ok(Some(orbit)) => {
            (StatusCode::OK, Json(serde_json::json!({ "orbit": orbit }))).into_response()
        }
        Ok(None) => json_error(StatusCode::NOT_FOUND, format!("orbit '{}' not found", id)),
        Err(err) => internal_error(err),
    }
}

pub(super) async fn update_orbit_endpoint(
    State(state): State<AppState>,
    Path(id): Path<String>,
    Json(body): Json<serde_json::Value>,
) -> Response {
    let mut patch = OrbitUpdate::default();
    let obj = match body.as_object() {
        Some(o) => o,
        None => {
            return json_error(
                StatusCode::BAD_REQUEST,
                "request body must be a JSON object",
            );
        }
    };
    if obj.contains_key("name") {
        match obj.get("name").and_then(|v| v.as_str()) {
            Some(s) if !s.trim().is_empty() => patch.name = Some(s.trim().to_string()),
            _ => return json_error(StatusCode::BAD_REQUEST, "'name' must be a non-empty string"),
        }
    }
    if obj.contains_key("icon") {
        patch.icon = Some(
            obj.get("icon")
                .and_then(|v| v.as_str())
                .map(|s| s.to_string()),
        );
    }
    if obj.contains_key("color") {
        patch.color = Some(
            obj.get("color")
                .and_then(|v| v.as_str())
                .map(|s| s.to_string()),
        );
    }
    if obj.contains_key("agent_instructions") {
        patch.agent_instructions = Some(
            obj.get("agent_instructions")
                .and_then(|v| v.as_str())
                .map(|s| s.to_string()),
        );
    }

    let agent = state.agent.read().await;
    match agent.arkorbit.update_orbit(&id, patch).await {
        Ok(orbit) => (StatusCode::OK, Json(serde_json::json!({ "orbit": orbit }))).into_response(),
        Err(err) => json_error(StatusCode::BAD_REQUEST, err.to_string()),
    }
}

pub(super) async fn delete_orbit_endpoint(
    State(state): State<AppState>,
    Path(id): Path<String>,
) -> Response {
    let agent = state.agent.read().await;
    match agent.arkorbit.delete_orbit(&id).await {
        Ok(()) => {
            let deleted_reflect_units = agent
                .storage
                .delete_semantic_work_units_for_source_prefix("orbit_chat", &id)
                .await
                .unwrap_or(0);
            (StatusCode::OK, Json(serde_json::json!({ "status": "ok", "deleted_reflect_units": deleted_reflect_units }))).into_response()
        }
        Err(err) => internal_error(err),
    }
}

pub(super) async fn orbit_index_endpoint(
    State(state): State<AppState>,
    Path(id): Path<String>,
) -> Response {
    let agent = state.agent.read().await;
    match agent.arkorbit.read_orbit_index(&id) {
        Ok(bytes) => set_orbit_index_headers(bytes.into_response()),
        Err(err) => json_error(StatusCode::NOT_FOUND, err.to_string()),
    }
}

pub(super) async fn orbit_messages_endpoint(
    State(state): State<AppState>,
    Path(id): Path<String>,
) -> Response {
    let agent = state.agent.read().await;
    match agent.arkorbit.read_orbit_chat_messages(&id, 200) {
        Ok(messages) => (
            StatusCode::OK,
            Json(serde_json::json!({ "messages": messages })),
        )
            .into_response(),
        Err(err) => json_error(StatusCode::BAD_REQUEST, err.to_string()),
    }
}

pub(super) async fn orbit_files_endpoint(
    State(state): State<AppState>,
    Path(id): Path<String>,
) -> Response {
    let agent = state.agent.read().await;
    match agent.arkorbit.list_orbit_files(&id) {
        Ok(files) => (StatusCode::OK, Json(serde_json::json!({ "files": files }))).into_response(),
        Err(err) => json_error(StatusCode::BAD_REQUEST, err.to_string()),
    }
}

pub(super) async fn orbit_file_endpoint(
    State(state): State<AppState>,
    Path((id, path)): Path<(String, String)>,
) -> Response {
    let agent = state.agent.read().await;
    match agent.arkorbit.read_orbit_file_text(&id, &path) {
        Ok(content) => {
            set_orbit_security_headers(content.into_response(), content_type_for_name(&path))
        }
        Err(err) => json_error(StatusCode::BAD_REQUEST, err.to_string()),
    }
}

fn orbit_widget_layout_number(
    body: &serde_json::Value,
    key: &str,
) -> std::result::Result<Option<f64>, String> {
    if !body.as_object().is_some_and(|object| object.contains_key(key)) {
        return Ok(None);
    }
    let Some(value) = body.get(key).and_then(|value| value.as_f64()) else {
        return Err(format!("'{}' must be a finite number", key));
    };
    if !value.is_finite() || value < 0.0 || value > 1_000_000.0 {
        return Err(format!("'{}' must be between 0 and 1000000", key));
    }
    Ok(Some(value))
}

pub(super) async fn update_orbit_widget_endpoint(
    State(state): State<AppState>,
    Path((id, widget_id)): Path<(String, String)>,
    Json(body): Json<serde_json::Value>,
) -> Response {
    let Some(widget_id) = widget_registry_key(&widget_id) else {
        return json_error(StatusCode::BAD_REQUEST, "invalid widget id");
    };
    let left = match orbit_widget_layout_number(&body, "left") {
        Ok(value) => value,
        Err(message) => return json_error(StatusCode::BAD_REQUEST, message),
    };
    let top = match orbit_widget_layout_number(&body, "top") {
        Ok(value) => value,
        Err(message) => return json_error(StatusCode::BAD_REQUEST, message),
    };
    if left.is_none() && top.is_none() {
        return json_error(
            StatusCode::BAD_REQUEST,
            "at least one layout field is required",
        );
    }

    let agent = state.agent.read().await;
    match agent.arkorbit.get_orbit(&id).await {
        Ok(Some(_)) => {}
        Ok(None) => return json_error(StatusCode::NOT_FOUND, "orbit not found"),
        Err(err) => return json_error(StatusCode::BAD_REQUEST, err.to_string()),
    }

    let raw = match agent
        .arkorbit
        .read_orbit_file_text(&id, "data/widgets.json")
    {
        Ok(raw) => raw,
        Err(err) => return json_error(StatusCode::NOT_FOUND, err.to_string()),
    };

    let parsed = match serde_json::from_str::<serde_json::Value>(&raw) {
        Ok(value) => value,
        Err(err) => return json_error(StatusCode::BAD_REQUEST, err.to_string()),
    };
    let (mut root, mut widgets) = if let Some(list) = parsed.as_array() {
        (None, list.clone())
    } else if let Some(list) = parsed.get("widgets").and_then(|value| value.as_array()) {
        (Some(parsed.clone()), list.clone())
    } else {
        return json_error(
            StatusCode::BAD_REQUEST,
            "data/widgets.json must be an array or object with widgets",
        );
    };

    let mut updated_widget = None;
    for widget in widgets.iter_mut() {
        let id_matches = widget_entry_key(widget, "id").as_deref() == Some(widget_id.as_str());
        let module_matches =
            widget_entry_key(widget, "module").as_deref() == Some(widget_id.as_str());
        if !id_matches && !module_matches {
            continue;
        }
        let Some(object) = widget.as_object_mut() else {
            continue;
        };
        if let Some(left) = left {
            object.insert("left".to_string(), serde_json::json!(left));
        }
        if let Some(top) = top {
            object.insert("top".to_string(), serde_json::json!(top));
        }
        updated_widget = Some(serde_json::Value::Object(object.clone()));
        break;
    }

    let Some(updated_widget) = updated_widget else {
        return json_error(StatusCode::NOT_FOUND, "widget not found");
    };

    let next = if let Some(root) = root.as_mut() {
        if let Some(object) = root.as_object_mut() {
            object.insert(
                "widgets".to_string(),
                serde_json::Value::Array(widgets.clone()),
            );
        }
        serde_json::to_string_pretty(root)
    } else {
        serde_json::to_string_pretty(&widgets)
    };
    match next {
        Ok(next) => {
            if let Err(err) = agent
                .arkorbit
                .write_orbit_file(&id, "data/widgets.json", &next)
            {
                return internal_error(err);
            }
        }
        Err(err) => return internal_error(err),
    }

    (
        StatusCode::OK,
        Json(serde_json::json!({
            "status": "ok",
            "widget": updated_widget,
        })),
    )
        .into_response()
}

pub(super) async fn delete_orbit_widget_endpoint(
    State(state): State<AppState>,
    Path((id, widget_id)): Path<(String, String)>,
) -> Response {
    let Some(widget_id) = widget_registry_key(&widget_id) else {
        return json_error(StatusCode::BAD_REQUEST, "invalid widget id");
    };

    let agent = state.agent.read().await;
    match agent.arkorbit.get_orbit(&id).await {
        Ok(Some(_)) => {}
        Ok(None) => return json_error(StatusCode::NOT_FOUND, "orbit not found"),
        Err(err) => return json_error(StatusCode::BAD_REQUEST, err.to_string()),
    }

    let raw = match agent
        .arkorbit
        .read_orbit_file_text(&id, "data/widgets.json")
    {
        Ok(raw) => raw,
        Err(_) => {
            return (
                StatusCode::OK,
                Json(serde_json::json!({ "status": "ok", "removed": false })),
            )
                .into_response();
        }
    };

    let parsed = match serde_json::from_str::<serde_json::Value>(&raw) {
        Ok(value) => value,
        Err(err) => return json_error(StatusCode::BAD_REQUEST, err.to_string()),
    };
    let (mut root, mut widgets) = if let Some(list) = parsed.as_array() {
        (None, list.clone())
    } else if let Some(list) = parsed.get("widgets").and_then(|value| value.as_array()) {
        (Some(parsed.clone()), list.clone())
    } else {
        return json_error(
            StatusCode::BAD_REQUEST,
            "data/widgets.json must be an array or object with widgets",
        );
    };

    let before = widgets.len();
    let mut modules_to_delete = BTreeSet::new();
    widgets.retain(|widget| {
        let id_matches = widget_entry_key(widget, "id").as_deref() == Some(widget_id.as_str());
        let module_matches =
            widget_entry_key(widget, "module").as_deref() == Some(widget_id.as_str());
        let matched = id_matches || module_matches;
        if matched {
            if let Some(module) = widget_entry_module_key(widget) {
                modules_to_delete.insert(module);
            }
        }
        !matched
    });
    let removed = widgets.len() != before;

    let next = if let Some(root) = root.as_mut() {
        if let Some(object) = root.as_object_mut() {
            object.insert(
                "widgets".to_string(),
                serde_json::Value::Array(widgets.clone()),
            );
        }
        serde_json::to_string_pretty(root)
    } else {
        serde_json::to_string_pretty(&widgets)
    };
    match next {
        Ok(next) => {
            if let Err(err) = agent
                .arkorbit
                .write_orbit_file(&id, "data/widgets.json", &next)
            {
                return internal_error(err);
            }
        }
        Err(err) => return internal_error(err),
    }

    let mut deleted_modules = Vec::new();
    for module in modules_to_delete {
        if widget_module_still_registered(&widgets, &module) {
            continue;
        }
        match agent.arkorbit.remove_orbit_module_dir(&id, &module) {
            Ok(true) => deleted_modules.push(module),
            Ok(false) => {}
            Err(err) => return internal_error(err),
        }
    }

    (
        StatusCode::OK,
        Json(serde_json::json!({
            "status": "ok",
            "removed": removed,
            "deleted_modules": deleted_modules,
        })),
    )
        .into_response()
}

pub(super) async fn orbit_public_fetch_endpoint(
    State(state): State<AppState>,
    Path(id): Path<String>,
    uri: Uri,
    Query(params): Query<HashMap<String, String>>,
    headers: HeaderMap,
) -> Response {
    {
        let agent = state.agent.read().await;
        match agent.arkorbit.get_orbit(&id).await {
            Ok(Some(_)) => {}
            Ok(None) => return json_error(StatusCode::NOT_FOUND, "orbit not found"),
            Err(err) => return json_error(StatusCode::BAD_REQUEST, err.to_string()),
        }
    }

    let raw_url = extract_public_proxy_target_from_query(uri.query(), "url")
        .or_else(|| params.get("url").map(|value| value.trim().to_string()));
    let Some(raw_url) = raw_url.filter(|value| !value.trim().is_empty()) else {
        return json_error(StatusCode::BAD_REQUEST, "missing query param: url");
    };

    let parsed = match parse_public_proxy_target_url(&raw_url) {
        Ok(url) => url,
        Err((error, message)) => {
            return (
                StatusCode::BAD_REQUEST,
                Json(serde_json::json!({
                    "error": error,
                    "message": message,
                })),
            )
                .into_response();
        }
    };

    let client = match reqwest::Client::builder()
        .timeout(Duration::from_secs(12))
        .redirect(reqwest::redirect::Policy::limited(3))
        .user_agent(crate::branding::user_agent_with_suffix(
            "arkorbit public fetch",
        ))
        .build()
    {
        Ok(client) => client,
        Err(_) => return StatusCode::INTERNAL_SERVER_ERROR.into_response(),
    };

    let mut request_builder = client.get(parsed.clone());
    for header_name in [
        "accept",
        "accept-language",
        "if-none-match",
        "if-modified-since",
        "range",
    ] {
        if let Some(value) = headers
            .get(header_name)
            .and_then(|value| value.to_str().ok())
        {
            request_builder = request_builder.header(header_name, value);
        }
    }

    let response = match request_builder.send().await {
        Ok(response) => response,
        Err(error) => {
            return (
                StatusCode::BAD_GATEWAY,
                Json(serde_json::json!({
                    "error": "fetch_failed",
                    "message": error.to_string(),
                    "url": parsed.to_string(),
                })),
            )
                .into_response();
        }
    };

    if !response.status().is_success() {
        return (
            StatusCode::BAD_GATEWAY,
            Json(serde_json::json!({
                "error": "upstream_error",
                "status": response.status().as_u16(),
                "url": parsed.to_string(),
            })),
        )
            .into_response();
    }

    if response
        .content_length()
        .is_some_and(|len| len > ORBIT_PUBLIC_FETCH_MAX_BYTES as u64)
    {
        return json_error(
            StatusCode::PAYLOAD_TOO_LARGE,
            "upstream response is too large",
        );
    }

    let content_type = response
        .headers()
        .get(header::CONTENT_TYPE)
        .and_then(|value| value.to_str().ok())
        .unwrap_or("application/octet-stream")
        .to_string();

    match response.bytes().await {
        Ok(bytes) if bytes.len() <= ORBIT_PUBLIC_FETCH_MAX_BYTES => (
            StatusCode::OK,
            [
                (header::CONTENT_TYPE, content_type),
                (header::CACHE_CONTROL, "no-store".to_string()),
            ],
            bytes,
        )
            .into_response(),
        Ok(_) => json_error(
            StatusCode::PAYLOAD_TOO_LARGE,
            "upstream response is too large",
        ),
        Err(_) => StatusCode::BAD_GATEWAY.into_response(),
    }
}

pub(super) async fn orbit_chat_transcripts_endpoint(
    State(state): State<AppState>,
    Path(id): Path<String>,
) -> Response {
    let agent = state.agent.read().await;
    match agent.arkorbit.list_orbit_chat_transcripts(&id) {
        Ok(transcripts) => (
            StatusCode::OK,
            Json(serde_json::json!({ "transcripts": transcripts })),
        )
            .into_response(),
        Err(err) => json_error(StatusCode::BAD_REQUEST, err.to_string()),
    }
}

pub(super) async fn orbit_chat_transcript_messages_endpoint(
    State(state): State<AppState>,
    Path((id, transcript_id)): Path<(String, String)>,
) -> Response {
    let agent = state.agent.read().await;
    match agent
        .arkorbit
        .read_orbit_chat_transcript(&id, &transcript_id, 200)
    {
        Ok(messages) => (
            StatusCode::OK,
            Json(serde_json::json!({ "messages": messages })),
        )
            .into_response(),
        Err(err) => json_error(StatusCode::BAD_REQUEST, err.to_string()),
    }
}

pub(super) async fn reset_orbit_chat_endpoint(
    State(state): State<AppState>,
    Path(id): Path<String>,
) -> Response {
    let agent = state.agent.read().await;
    match agent.arkorbit.reset_orbit_chat(&id) {
        Ok(transcript) => (
            StatusCode::OK,
            Json(serde_json::json!({ "status": "ok", "transcript": transcript })),
        )
            .into_response(),
        Err(err) => json_error(StatusCode::BAD_REQUEST, err.to_string()),
    }
}

pub(super) async fn resolve_module_endpoint(
    State(state): State<AppState>,
    Path((orbit_id, path)): Path<(String, String)>,
) -> Response {
    let agent = state.agent.read().await;
    match agent.arkorbit.resolve_module(&orbit_id, &path) {
        Ok(Some(resolved)) => {
            set_orbit_security_headers(resolved.bytes.into_response(), &resolved.content_type)
        }
        Ok(None) => json_error(StatusCode::NOT_FOUND, "ArkOrbit module not found"),
        Err(err) => json_error(StatusCode::BAD_REQUEST, err.to_string()),
    }
}

pub(super) async fn orbit_events_endpoint(
    State(state): State<AppState>,
    Path(id): Path<String>,
) -> Response {
    let (service, orbit_dir) = {
        let agent = state.agent.read().await;
        let service = agent.arkorbit.clone();
        let orbit_dir = match service.orbit_dir(&id) {
            Ok(dir) => dir,
            Err(err) => return json_error(StatusCode::BAD_REQUEST, err.to_string()),
        };
        (service, orbit_dir)
    };
    drop(service);

    let (notify_tx, mut notify_rx) =
        tokio::sync::mpsc::channel::<notify::Result<notify::Event>>(64);
    let mut watcher = match notify::recommended_watcher(move |event| {
        let _ = notify_tx.blocking_send(event);
    }) {
        Ok(watcher) => watcher,
        Err(err) => return internal_error(err),
    };
    if let Err(err) =
        notify::Watcher::watch(&mut watcher, &orbit_dir, notify::RecursiveMode::Recursive)
    {
        return internal_error(err);
    }

    let (tx, srx) =
        tokio::sync::mpsc::channel::<std::result::Result<Event, std::convert::Infallible>>(32);
    crate::spawn_logged!(
        "src/channels/http/arkorbit_control.rs:orbit_events",
        async move {
            let _watcher = watcher;
            while let Some(event) = notify_rx.recv().await {
                let Ok(event) = event else {
                    continue;
                };
                if !is_orbit_reload_event(&event.kind) {
                    continue;
                }
                for path in event.paths {
                    let Ok(rel) = path.strip_prefix(&orbit_dir) else {
                        continue;
                    };
                    let rel = rel.to_string_lossy().replace('\\', "/");
                    if rel.is_empty() || rel.starts_with(".tmp/") {
                        continue;
                    }
                    let payload = serde_json::json!({
                        "kind": "file_changed",
                        "path": rel,
                    });
                    let Ok(data) = serde_json::to_string(&payload) else {
                        continue;
                    };
                    if tx
                        .send(Ok(Event::default().event("file_changed").data(data)))
                        .await
                        .is_err()
                    {
                        return;
                    }
                }
            }
        }
    );

    set_event_stream_headers(
        Sse::new(cap_sse_lifetime(
            tokio_stream::wrappers::ReceiverStream::new(srx),
        ))
        .keep_alive(KeepAlive::default())
        .into_response(),
    )
}

pub(super) async fn orbit_chat_endpoint(
    State(state): State<AppState>,
    Path(id): Path<String>,
    Json(body): Json<serde_json::Value>,
) -> Response {
    let message = body
        .get("message")
        .and_then(|v| v.as_str())
        .map(|s| s.trim().to_string())
        .filter(|s| !s.is_empty());
    let Some(message) = message else {
        return json_error(StatusCode::BAD_REQUEST, "'message' is required");
    };

    let trace_id = uuid::Uuid::new_v4().to_string();
    let started_at = chrono::Utc::now();
    let trace_ref = Arc::new(RwLock::new(ExecutionTrace {
        id: trace_id,
        message: message.clone(),
        channel: ORBIT_TRACE_CHANNEL.to_string(),
        started_at: Some(started_at),
        completed_at: None,
        steps: vec![crate::core::ExecutionStep {
            icon: "[orbit]".to_string(),
            title: "Orbit Request".to_string(),
            detail: format!(
                "Orbit chat turn | Source: {} | Orbit: {} | Length: {} chars",
                ORBIT_TRACE_SOURCE_LABEL,
                id,
                message.chars().count()
            ),
            step_type: "info".to_string(),
            data: Some(
                serde_json::json!({
                    "source": ORBIT_TRACE_SOURCE_LABEL,
                    "channel": ORBIT_TRACE_CHANNEL,
                    "orbit_id": id.clone(),
                    "message_chars": message.chars().count(),
                })
                .to_string(),
            ),
            timestamp: started_at,
            duration_ms: Some(0),
        }],
        proof_id: None,
        response: None,
        model: None,
        input_tokens: 0,
        output_tokens: 0,
        total_tokens: 0,
        cost_usd: 0.0,
        complexity: Some("orbit_chat".to_string()),
        plan: None,
    }));
    mirror_orbit_trace_snapshot(&state, &trace_ref).await;

    let (service, llm) = {
        let agent = state.agent.read().await;
        (
            agent.arkorbit.clone(),
            agent.llm_for_role(&crate::core::ModelRole::Primary).clone(),
        )
    };

    let (agent_tx, mut agent_rx) = tokio::sync::mpsc::channel::<OrbitAgentEvent>(128);
    let (sse_tx, sse_rx) =
        tokio::sync::mpsc::channel::<std::result::Result<Event, std::convert::Infallible>>(128);
    let worker_orbit_id = id.clone();
    let worker_message = message.clone();
    crate::spawn_logged!(
        "src/channels/http/arkorbit_control.rs:orbit_chat_worker",
        async move {
            let error_tx = agent_tx.clone();
            if let Err(err) =
                stream_orbit_chat_turn(service, llm, worker_orbit_id, worker_message, agent_tx)
                    .await
            {
                tracing::warn!(target: "arkorbit.chat", error = %err, "orbit chat stream failed");
                let _ = error_tx.send(OrbitAgentEvent::Error(err.to_string())).await;
                let _ = error_tx.send(OrbitAgentEvent::Done).await;
            }
        }
    );
    let trace_ref_for_sse = trace_ref.clone();
    let agent_for_trace = state.agent.clone();
    crate::spawn_logged!(
        "src/channels/http/arkorbit_control.rs:orbit_chat_sse",
        async move {
            let mut trace_accumulator = OrbitTraceAccumulator::default();
            let mut client_open = true;
            let mut idle_status = tokio::time::interval(std::time::Duration::from_secs(20));
            idle_status.set_missed_tick_behavior(tokio::time::MissedTickBehavior::Delay);
            idle_status.tick().await;
            loop {
                let event = tokio::select! {
                    event = agent_rx.recv() => event,
                    _ = idle_status.tick() => {
                        if client_open {
                            let payload = serde_json::json!({
                                "message": "Still working on the Orbit request."
                            });
                            if let Ok(data) = serde_json::to_string(&payload) {
                                if sse_tx
                                    .send(Ok(Event::default().event("status").data(data)))
                                    .await
                                    .is_err()
                                {
                                    client_open = false;
                                }
                            }
                        }
                        continue;
                    }
                };
                let Some(event) = event else {
                    break;
                };
                trace_accumulator.record_event(&event, started_at);
                let done = matches!(event, OrbitAgentEvent::Done);
                let (name, payload) = match event {
                    OrbitAgentEvent::Status { message } => {
                        ("status", serde_json::json!({ "message": message }))
                    }
                    OrbitAgentEvent::Token(content) => {
                        ("token", serde_json::json!({ "content": content }))
                    }
                    OrbitAgentEvent::FileWritten { path, operation } => (
                        "file_written",
                        serde_json::json!({
                            "path": path,
                            "operation": operation.as_str()
                        }),
                    ),
                    OrbitAgentEvent::ReadRequested { path } => {
                        ("read", serde_json::json!({ "path": path }))
                    }
                    OrbitAgentEvent::Usage(usage) => (
                        "usage",
                        serde_json::to_value(&usage).unwrap_or_else(|_| serde_json::json!({})),
                    ),
                    OrbitAgentEvent::Done => ("done", serde_json::json!({})),
                    OrbitAgentEvent::Error(message) => {
                        ("error", serde_json::json!({ "message": message }))
                    }
                };
                let Ok(data) = serde_json::to_string(&payload) else {
                    continue;
                };
                if client_open {
                    if sse_tx
                        .send(Ok(Event::default().event(name).data(data)))
                        .await
                        .is_err()
                    {
                        client_open = false;
                    }
                }
                if done {
                    persist_orbit_trace(
                        agent_for_trace.clone(),
                        trace_ref_for_sse.clone(),
                        trace_accumulator,
                        true,
                        started_at,
                    )
                    .await;
                    return;
                }
            }
            persist_orbit_trace(
                agent_for_trace,
                trace_ref_for_sse,
                trace_accumulator,
                false,
                started_at,
            )
            .await;
        }
    );

    set_event_stream_headers(
        Sse::new(cap_sse_lifetime(
            tokio_stream::wrappers::ReceiverStream::new(sse_rx),
        ))
        .keep_alive(KeepAlive::default())
        .into_response(),
    )
}
