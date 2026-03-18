use super::*;

pub(super) async fn persist_live_trace_snapshot(
    trace_history: &Arc<RwLock<Vec<ExecutionTrace>>>,
    trace_ref: &Arc<RwLock<ExecutionTrace>>,
) -> Option<ExecutionTrace> {
    let snapshot = trace_ref.read().await.clone();
    if snapshot.id.trim().is_empty() {
        return None;
    }
    {
        let mut history = trace_history.write().await;
        history.retain(|item| item.id != snapshot.id);
        history.insert(0, snapshot.clone());
        if history.len() > 100 {
            history.truncate(100);
        }
    }
    Some(snapshot)
}

pub(super) fn spawn_live_trace_mirror(
    trace_history: Arc<RwLock<Vec<ExecutionTrace>>>,
    trace_ref: Arc<RwLock<ExecutionTrace>>,
) {
    tokio::spawn(async move {
        loop {
            let snapshot = persist_live_trace_snapshot(&trace_history, &trace_ref).await;
            if snapshot
                .as_ref()
                .and_then(|trace| trace.completed_at.as_ref())
                .is_some()
            {
                break;
            }
            tokio::time::sleep(std::time::Duration::from_millis(350)).await;
        }
    });
}

#[derive(Debug, Serialize)]
pub(super) struct TraceStep {
    pub icon: String,
    pub title: String,
    pub detail: String,
    #[serde(rename = "type")]
    pub step_type: String,
    pub data: Option<String>,
    pub time: String,
}

#[derive(Debug, Serialize)]
pub(super) struct ProofSummary {
    pub id: String,
    pub message_preview: String,
    pub time: String,
}

#[derive(Debug, Serialize)]
pub(super) struct TraceResponse {
    pub trace: Vec<TraceStep>,
    pub proofs: Vec<ProofSummary>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub history_total: Option<usize>,
    pub history: Vec<TraceSummary>,
}

#[derive(Debug, Serialize)]
pub(super) struct TraceSummary {
    pub id: String,
    pub message_preview: String,
    pub channel: String,
    pub status: String,
    pub step_count: usize,
    pub started_at: String,
    pub duration_ms: Option<u64>,
    pub model: Option<String>,
    pub total_tokens: i64,
    pub cost_usd: f64,
    pub complexity: Option<String>,
}

#[derive(Debug, Serialize)]
pub(super) struct TraceDetailResponse {
    pub id: String,
    pub message: String,
    pub channel: String,
    pub status: String,
    pub started_at: Option<String>,
    pub completed_at: Option<String>,
    pub duration_ms: Option<u64>,
    pub step_count: usize,
    pub steps: Vec<TraceStep>,
    pub response: Option<String>,
    pub proof_id: Option<String>,
    pub model: Option<String>,
    pub input_tokens: i64,
    pub output_tokens: i64,
    pub total_tokens: i64,
    pub cost_usd: f64,
    pub complexity: Option<String>,
}

fn trace_message_preview(message: &str) -> String {
    if message.len() > 120 {
        format!("{}...", &message[..120])
    } else {
        message.to_string()
    }
}

fn trace_status_from_steps(steps: &[crate::core::ExecutionStep], completed: bool) -> String {
    if let Some(last_step) = steps.last() {
        let title = last_step.title.to_ascii_lowercase();
        let step_type = last_step.step_type.to_ascii_lowercase();
        if step_type == "error" || title.contains("failed") {
            return "failed".to_string();
        }
        if step_type == "warning" || title.contains("blocked") {
            return "warning".to_string();
        }
    }
    if completed {
        "completed".to_string()
    } else {
        "running".to_string()
    }
}

fn format_trace_step_time(step: &crate::core::ExecutionStep) -> String {
    // Send full ISO timestamp — frontend converts to local time
    if let Some(ms) = step.duration_ms {
        format!(
            "{} ({}ms)",
            step.timestamp
                .to_rfc3339_opts(chrono::SecondsFormat::Secs, true),
            ms
        )
    } else {
        step.timestamp
            .to_rfc3339_opts(chrono::SecondsFormat::Secs, true)
    }
}

fn format_trace_summary_from_memory(t: &ExecutionTrace) -> TraceSummary {
    let duration_ms = t.started_at.and_then(|start| {
        t.completed_at
            .map(|end| (end - start).num_milliseconds() as u64)
    });
    let status = trace_status_from_steps(&t.steps, t.completed_at.is_some());
    TraceSummary {
        id: t.id.clone(),
        message_preview: trace_message_preview(&t.message),
        channel: t.channel.clone(),
        status,
        step_count: t.steps.len(),
        started_at: t
            .started_at
            .map(|s| s.format("%H:%M:%S").to_string())
            .unwrap_or_default(),
        duration_ms,
        model: t.model.clone(),
        total_tokens: t.total_tokens,
        cost_usd: t.cost_usd,
        complexity: t.complexity.clone(),
    }
}

fn format_trace_detail_from_memory(t: &ExecutionTrace) -> TraceDetailResponse {
    let duration_ms = t.started_at.and_then(|start| {
        t.completed_at
            .map(|end| (end - start).num_milliseconds() as u64)
    });
    let steps: Vec<TraceStep> = t
        .steps
        .iter()
        .map(|step| TraceStep {
            icon: step.icon.clone(),
            title: step.title.clone(),
            detail: step.detail.clone(),
            step_type: step.step_type.clone(),
            data: step.data.clone(),
            time: format_trace_step_time(step),
        })
        .collect();

    TraceDetailResponse {
        id: t.id.clone(),
        message: t.message.clone(),
        channel: t.channel.clone(),
        status: trace_status_from_steps(&t.steps, t.completed_at.is_some()),
        started_at: t
            .started_at
            .map(|s| s.format("%Y-%m-%d %H:%M:%S").to_string()),
        completed_at: t
            .completed_at
            .map(|c| c.format("%Y-%m-%d %H:%M:%S").to_string()),
        duration_ms,
        step_count: steps.len(),
        steps,
        response: t.response.clone(),
        proof_id: t.proof_id.clone(),
        model: t.model.clone(),
        input_tokens: t.input_tokens,
        output_tokens: t.output_tokens,
        total_tokens: t.total_tokens,
        cost_usd: t.cost_usd,
        complexity: t.complexity.clone(),
    }
}

fn parse_persisted_trace_steps(
    model: &crate::storage::entities::execution_trace::Model,
) -> Vec<crate::core::ExecutionStep> {
    serde_json::from_str(&model.steps_json).unwrap_or_default()
}

fn parse_rfc3339_to_local_display(value: &Option<String>) -> Option<String> {
    // Send full ISO timestamp — frontend converts to local time
    value.as_deref().and_then(|raw| {
        chrono::DateTime::parse_from_rfc3339(raw).ok().map(|dt| {
            dt.with_timezone(&chrono::Utc)
                .to_rfc3339_opts(chrono::SecondsFormat::Secs, true)
        })
    })
}

fn parse_rfc3339_to_time_display(value: &Option<String>) -> String {
    // Send full ISO timestamp — frontend converts to local relative time
    value
        .as_deref()
        .and_then(|raw| chrono::DateTime::parse_from_rfc3339(raw).ok())
        .map(|dt| {
            dt.with_timezone(&chrono::Utc)
                .to_rfc3339_opts(chrono::SecondsFormat::Secs, true)
        })
        .unwrap_or_default()
}

fn format_trace_summary_from_persisted(
    t: &crate::storage::entities::execution_trace::Model,
) -> TraceSummary {
    let parsed_steps = parse_persisted_trace_steps(t);
    let status = trace_status_from_steps(&parsed_steps, t.completed_at.is_some());
    TraceSummary {
        id: t.id.clone(),
        message_preview: trace_message_preview(&t.message),
        channel: t.channel.clone(),
        status,
        step_count: t.step_count.max(0) as usize,
        started_at: parse_rfc3339_to_time_display(&t.started_at),
        duration_ms: t.duration_ms.map(|value| value.max(0) as u64),
        model: t.model.clone(),
        total_tokens: t.total_tokens,
        cost_usd: t.cost_usd,
        complexity: t.complexity.clone(),
    }
}

fn trace_sort_key_from_memory(t: &ExecutionTrace) -> String {
    t.started_at
        .or(t.completed_at)
        .unwrap_or_else(chrono::Utc::now)
        .to_rfc3339()
}

fn trace_sort_key_from_persisted(t: &crate::storage::entities::execution_trace::Model) -> String {
    t.started_at
        .clone()
        .or(t.completed_at.clone())
        .unwrap_or_else(|| t.created_at.clone())
}

fn format_trace_detail_from_persisted(
    t: &crate::storage::entities::execution_trace::Model,
) -> TraceDetailResponse {
    let parsed_steps = parse_persisted_trace_steps(t);
    let step_count = parsed_steps.len();
    let steps = parsed_steps
        .iter()
        .cloned()
        .map(|step| {
            let time = format_trace_step_time(&step);
            TraceStep {
                icon: step.icon,
                title: step.title,
                detail: step.detail,
                step_type: step.step_type,
                data: step.data,
                time,
            }
        })
        .collect();

    TraceDetailResponse {
        id: t.id.clone(),
        message: t.message.clone(),
        channel: t.channel.clone(),
        status: trace_status_from_steps(&parsed_steps, t.completed_at.is_some()),
        started_at: parse_rfc3339_to_local_display(&t.started_at),
        completed_at: parse_rfc3339_to_local_display(&t.completed_at),
        duration_ms: t.duration_ms.map(|value| value.max(0) as u64),
        step_count,
        steps,
        response: t.response.clone(),
        proof_id: t.proof_id.clone(),
        model: t.model.clone(),
        input_tokens: t.input_tokens,
        output_tokens: t.output_tokens,
        total_tokens: t.total_tokens,
        cost_usd: t.cost_usd,
        complexity: t.complexity.clone(),
    }
}

pub(super) async fn get_trace(
    State(state): State<AppState>,
    axum::extract::Query(params): axum::extract::Query<std::collections::HashMap<String, String>>,
) -> Json<TraceResponse> {
    let history_limit = params
        .get("limit")
        .and_then(|s| s.parse().ok())
        .unwrap_or(20usize);
    let history_offset = params
        .get("offset")
        .and_then(|s| s.parse().ok())
        .unwrap_or(0usize);

    let agent = state.agent.read().await;
    let persisted_history = agent
        .encrypted_storage
        .list_execution_traces_decrypted(history_limit as u64 + history_offset as u64 + 100, 0)
        .await
        .unwrap_or_default();

    let last_trace = state.last_trace.read().await;
    let trace_history = state.trace_history.read().await;

    let trace: Vec<TraceStep> = last_trace
        .steps
        .iter()
        .map(|step| TraceStep {
            icon: step.icon.clone(),
            title: step.title.clone(),
            detail: step.detail.clone(),
            step_type: step.step_type.clone(),
            data: step.data.clone(),
            time: format_trace_step_time(step),
        })
        .collect();

    let proofs = if let Some(ref proof_id) = last_trace.proof_id {
        vec![ProofSummary {
            id: proof_id.clone(),
            message_preview: if last_trace.message.len() > 50 {
                format!("{}...", &last_trace.message[..50])
            } else {
                last_trace.message.clone()
            },
            time: last_trace
                .completed_at
                .map(|t| t.format("%H:%M:%S").to_string())
                .unwrap_or_else(|| "pending".to_string()),
        }]
    } else {
        vec![]
    };

    let mut history_by_id = std::collections::BTreeMap::<String, (String, TraceSummary)>::new();
    for item in persisted_history.iter() {
        history_by_id.insert(
            item.id.clone(),
            (
                trace_sort_key_from_persisted(item),
                format_trace_summary_from_persisted(item),
            ),
        );
    }
    for item in trace_history.iter() {
        history_by_id.insert(
            item.id.clone(),
            (
                trace_sort_key_from_memory(item),
                format_trace_summary_from_memory(item),
            ),
        );
    }
    let mut history_all: Vec<(String, TraceSummary)> = history_by_id.into_values().collect();
    history_all.sort_by(|a, b| b.0.cmp(&a.0));
    let history_total = history_all.len();
    let history: Vec<TraceSummary> = history_all
        .into_iter()
        .skip(history_offset)
        .take(history_limit)
        .map(|(_, summary)| summary)
        .collect();

    Json(TraceResponse {
        trace,
        proofs,
        history,
        history_total: Some(history_total),
    })
}

pub(super) async fn get_trace_detail(
    State(state): State<AppState>,
    Path(id): Path<String>,
) -> Response {
    let trace_history = state.trace_history.read().await;
    let trace = trace_history.iter().find(|t| t.id == id);

    match trace {
        Some(t) => (StatusCode::OK, Json(format_trace_detail_from_memory(t))).into_response(),
        None => {
            drop(trace_history);
            let agent = state.agent.read().await;
            match agent
                .encrypted_storage
                .get_execution_trace_decrypted(&id)
                .await
            {
                Ok(Some(t)) => {
                    (StatusCode::OK, Json(format_trace_detail_from_persisted(&t))).into_response()
                }
                Ok(None) => (
                    StatusCode::NOT_FOUND,
                    Json(ErrorResponse {
                        error: format!("Trace '{}' not found", id),
                    }),
                )
                    .into_response(),
                Err(e) => (
                    StatusCode::INTERNAL_SERVER_ERROR,
                    Json(ErrorResponse {
                        error: format!("Failed to load trace '{}': {}", id, e),
                    }),
                )
                    .into_response(),
            }
        }
    }
}
