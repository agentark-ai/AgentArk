use super::*;

const TRACE_RUNTIME_SOURCE: &str = "Runtime";

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
    crate::spawn_logged!("src/channels/http/trace.rs:26", async move {
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
    #[serde(skip_serializing_if = "Option::is_none")]
    pub source: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub status: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub ref_id: Option<String>,
    #[serde(skip_serializing_if = "Vec::is_empty", default)]
    pub artifacts: Vec<TraceArtifact>,
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
    pub recent_events: Vec<TraceOperationalEvent>,
    pub recent_events_total: usize,
    pub recent_events_offset: usize,
    pub recent_events_limit: usize,
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
pub(super) struct TraceOperationalEvent {
    pub id: String,
    pub source: String,
    pub trace_id: Option<String>,
    pub created_at: String,
    pub channel: String,
    pub event_type: String,
    pub success: bool,
    pub outcome: String,
    pub tool_name: Option<String>,
    pub latency_ms: Option<i64>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub details: Option<serde_json::Value>,
}

#[derive(Debug, Serialize)]
pub(super) struct TraceArtifact {
    pub kind: String,
    pub label: String,
    pub summary: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub format: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub data: Option<String>,
}

#[derive(Debug, Serialize)]
pub(super) struct TraceExecutionRunDetail {
    pub id: String,
    pub kind: String,
    pub status: String,
    pub current_stage: String,
    pub attempt: u32,
    pub created_at: String,
    pub updated_at: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub request_message: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub result_summary: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub last_error: Option<String>,
    #[serde(skip_serializing_if = "Vec::is_empty", default)]
    pub degradation: Vec<String>,
    #[serde(skip_serializing_if = "Vec::is_empty", default)]
    pub attempted_models: Vec<String>,
}

#[derive(Debug, Serialize)]
pub(super) struct TraceCheckpointDetail {
    pub run_id: String,
    pub sequence_no: u32,
    pub stage: String,
    pub payload: String,
    pub created_at: String,
}

#[derive(Debug, Serialize)]
pub(super) struct TraceToolAttemptDetail {
    pub id: String,
    pub run_id: String,
    pub sequence_no: u32,
    pub tool_name: String,
    pub status: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub failure_class: Option<String>,
    pub retryable: bool,
    pub side_effect_level: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub idempotency_key: Option<String>,
    pub arguments_json: String,
    pub output_json: String,
    pub started_at: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub completed_at: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub error_text: Option<String>,
}

#[derive(Debug, Serialize)]
pub(super) struct TraceOperationalLogDetail {
    pub id: String,
    pub created_at: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub trace_id: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub conversation_id: Option<String>,
    pub channel: String,
    pub event_type: String,
    pub success: bool,
    pub outcome: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub tool_name: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub latency_ms: Option<i64>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub arguments: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub payload: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub strategy_version: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub policy_version: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub prompt_version: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub model_slot: Option<String>,
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
    #[serde(skip_serializing_if = "Option::is_none")]
    pub execution_run: Option<TraceExecutionRunDetail>,
    #[serde(skip_serializing_if = "Vec::is_empty", default)]
    pub checkpoints: Vec<TraceCheckpointDetail>,
    #[serde(skip_serializing_if = "Vec::is_empty", default)]
    pub tool_attempts: Vec<TraceToolAttemptDetail>,
    #[serde(skip_serializing_if = "Vec::is_empty", default)]
    pub operational_logs: Vec<TraceOperationalLogDetail>,
}

fn trace_message_preview(message: &str) -> String {
    if message == crate::storage::ENCRYPTED_STORAGE_UNAVAILABLE {
        return "Older run details unavailable".to_string();
    }
    if message.len() > 120 {
        format!("{}...", &message[..120])
    } else {
        message.to_string()
    }
}

#[derive(Debug, Default)]
struct TraceEnrichment {
    execution_run: Option<crate::core::ExecutionRun>,
    checkpoints: Vec<crate::core::ExecutionCheckpoint>,
    tool_attempts: Vec<crate::core::ToolAttempt>,
    operational_logs: Vec<crate::storage::entities::operational_log::Model>,
}

fn truncate_trace_text(value: &str, max_chars: usize) -> String {
    if max_chars == 0 {
        return String::new();
    }
    let trimmed = value.trim();
    if trimmed.chars().count() <= max_chars {
        return trimmed.to_string();
    }
    let mut end = trimmed.len();
    for (count, (idx, _)) in trimmed.char_indices().enumerate() {
        if count == max_chars {
            end = idx;
            break;
        }
    }
    let mut out = trimmed[..end].trim_end().to_string();
    out.push_str("...");
    out
}

fn title_case_trace_label(value: &str) -> String {
    value
        .split(['_', '-', ' '])
        .filter(|token| !token.trim().is_empty())
        .map(|token| {
            let mut chars = token.chars();
            match chars.next() {
                Some(first) => {
                    let mut out = first.to_uppercase().to_string();
                    out.push_str(chars.as_str());
                    out
                }
                None => String::new(),
            }
        })
        .collect::<Vec<_>>()
        .join(" ")
}

fn format_trace_time_display(raw: &str, duration_ms: Option<u64>) -> String {
    let normalized = chrono::DateTime::parse_from_rfc3339(raw)
        .ok()
        .map(|dt| {
            dt.with_timezone(&chrono::Utc)
                .to_rfc3339_opts(chrono::SecondsFormat::Secs, true)
        })
        .unwrap_or_else(|| raw.trim().to_string());
    if let Some(duration_ms) = duration_ms {
        format!("{} ({}ms)", normalized, duration_ms)
    } else {
        normalized
    }
}

fn trace_status_for_step(step_type: &str, title: &str) -> String {
    let lower_type = step_type.trim().to_ascii_lowercase();
    let lower_title = title.trim().to_ascii_lowercase();
    if lower_type.contains("error") || lower_title.contains("failed") {
        return "error".to_string();
    }
    if lower_type.contains("warning") || lower_title.contains("blocked") {
        return "warning".to_string();
    }
    if lower_type.contains("success")
        || lower_title.contains("completed")
        || lower_title.contains("done")
    {
        return "success".to_string();
    }
    if lower_type.contains("think") || lower_type.contains("plan") || lower_type.contains("running")
    {
        return "running".to_string();
    }
    "info".to_string()
}

fn summarize_json_value(value: &serde_json::Value) -> String {
    match value {
        serde_json::Value::Object(map) => {
            if let Some(sql) = map.get("sql").and_then(|value| value.as_str()) {
                return truncate_trace_text(&sql.replace('\n', " "), 180);
            }
            if let Some(row_count) = map.get("row_count").and_then(|value| value.as_u64()) {
                let table = map
                    .get("table")
                    .and_then(|value| value.as_str())
                    .unwrap_or("query");
                return format!(
                    "{} row{} from {}",
                    row_count,
                    if row_count == 1 { "" } else { "s" },
                    table
                );
            }
            if let Some(table_count) = map.get("table_count").and_then(|value| value.as_u64()) {
                return format!(
                    "{} table{} discovered",
                    table_count,
                    if table_count == 1 { "" } else { "s" }
                );
            }
            let keys = map.keys().take(4).cloned().collect::<Vec<_>>();
            if keys.is_empty() {
                "Empty object".to_string()
            } else {
                format!("Fields: {}", keys.join(", "))
            }
        }
        serde_json::Value::Array(items) => {
            format!(
                "{} item{}",
                items.len(),
                if items.len() == 1 { "" } else { "s" }
            )
        }
        serde_json::Value::String(value) => truncate_trace_text(value, 180),
        serde_json::Value::Bool(value) => value.to_string(),
        serde_json::Value::Number(value) => value.to_string(),
        serde_json::Value::Null => "null".to_string(),
    }
}

fn parse_jsonish_value(raw: &str) -> Option<serde_json::Value> {
    let mut candidate = raw.trim().to_string();
    if candidate.is_empty() {
        return None;
    }
    for _ in 0..4 {
        match serde_json::from_str::<serde_json::Value>(&candidate) {
            Ok(serde_json::Value::String(inner)) => {
                candidate = inner;
            }
            Ok(value) => return Some(value),
            Err(_) => return None,
        }
    }
    None
}

fn build_text_artifact(
    kind: &str,
    label: &str,
    format: Option<&str>,
    raw: &str,
    summary: Option<String>,
) -> TraceArtifact {
    let trimmed = raw.trim().to_string();
    TraceArtifact {
        kind: kind.to_string(),
        label: label.to_string(),
        summary: summary.unwrap_or_else(|| truncate_trace_text(&trimmed.replace('\n', " "), 180)),
        format: format.map(|value| value.to_string()),
        data: (!trimmed.is_empty()).then_some(trimmed),
    }
}

fn build_json_artifact(
    kind: &str,
    label: &str,
    value: &serde_json::Value,
    summary: Option<String>,
) -> TraceArtifact {
    TraceArtifact {
        kind: kind.to_string(),
        label: label.to_string(),
        summary: summary.unwrap_or_else(|| summarize_json_value(value)),
        format: Some("json".to_string()),
        data: serde_json::to_string_pretty(value).ok(),
    }
}

fn build_artifact_from_raw(kind: &str, label: &str, raw: &str) -> Option<TraceArtifact> {
    let trimmed = raw.trim();
    if trimmed.is_empty() {
        return None;
    }
    Some(match parse_jsonish_value(trimmed) {
        Some(value) => build_json_artifact(kind, label, &value, None),
        None => build_text_artifact(kind, label, None, trimmed, None),
    })
}

fn render_artifact_blocks(artifacts: &[TraceArtifact]) -> Option<String> {
    let blocks = artifacts
        .iter()
        .map(|artifact| {
            let mut lines = vec![artifact.label.clone()];
            if let Some(data) = artifact
                .data
                .as_deref()
                .map(str::trim)
                .filter(|value| !value.is_empty())
            {
                lines.push(data.to_string());
            } else if !artifact.summary.trim().is_empty() {
                lines.push(artifact.summary.trim().to_string());
            }
            lines.join("\n")
        })
        .collect::<Vec<_>>();
    (!blocks.is_empty()).then_some(blocks.join("\n\n"))
}

fn normalize_trace_since_param(raw: Option<&str>) -> Option<String> {
    raw.map(str::trim)
        .filter(|value| !value.is_empty())
        .and_then(|value| chrono::DateTime::parse_from_rfc3339(value).ok())
        .map(|dt| {
            dt.with_timezone(&chrono::Utc)
                .to_rfc3339_opts(chrono::SecondsFormat::Secs, true)
        })
}

fn trace_history_anchor_from_memory(
    trace: &ExecutionTrace,
) -> Option<chrono::DateTime<chrono::Utc>> {
    trace.completed_at.or(trace.started_at)
}

fn trace_matches_since_memory(
    trace: &ExecutionTrace,
    since: Option<&chrono::DateTime<chrono::Utc>>,
) -> bool {
    match since {
        Some(since) => trace_history_anchor_from_memory(trace)
            .map(|value| value >= *since)
            .unwrap_or(false),
        None => true,
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
    // Send full ISO timestamp; frontend converts to local time.
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

fn parse_persisted_trace_steps(
    model: &crate::storage::entities::execution_trace::Model,
) -> Vec<crate::core::ExecutionStep> {
    serde_json::from_str(&model.steps_json).unwrap_or_default()
}

fn format_trace_execution_run_detail(run: &crate::core::ExecutionRun) -> TraceExecutionRunDetail {
    TraceExecutionRunDetail {
        id: run.id.clone(),
        kind: run.kind.clone(),
        status: run.status.as_str().to_string(),
        current_stage: run.current_stage.clone(),
        attempt: run.attempt,
        created_at: format_trace_time_display(&run.created_at, None),
        updated_at: format_trace_time_display(&run.updated_at, None),
        request_message: run.request_message.clone(),
        result_summary: run.result_summary.clone(),
        last_error: run.last_error.clone(),
        degradation: run
            .degradation
            .iter()
            .map(|note| {
                let mut line = note.summary.trim().to_string();
                if let Some(detail) = note
                    .detail
                    .as_deref()
                    .map(str::trim)
                    .filter(|value| !value.is_empty())
                {
                    if !line.is_empty() {
                        line.push_str(" - ");
                    }
                    line.push_str(detail);
                }
                if line.is_empty() {
                    title_case_trace_label(&note.kind)
                } else {
                    line
                }
            })
            .collect(),
        attempted_models: run
            .attempted_models
            .iter()
            .map(|attempt| {
                let mut line = format!("{} via {}", attempt.model_name, attempt.slot_label);
                if let Some(provider) = attempt
                    .provider_id
                    .as_deref()
                    .map(str::trim)
                    .filter(|value| !value.is_empty())
                {
                    line.push_str(&format!(" ({provider})"));
                }
                if let Some(elapsed_ms) = attempt.elapsed_ms {
                    line.push_str(&format!(" | {}ms", elapsed_ms));
                }
                if !attempt.success {
                    line.push_str(" | failed");
                }
                line
            })
            .collect(),
    }
}

fn format_trace_checkpoint_detail(
    checkpoint: &crate::core::ExecutionCheckpoint,
) -> TraceCheckpointDetail {
    TraceCheckpointDetail {
        run_id: checkpoint.run_id.clone(),
        sequence_no: checkpoint.sequence_no,
        stage: checkpoint.stage.clone(),
        payload: checkpoint.payload.clone(),
        created_at: format_trace_time_display(&checkpoint.created_at, None),
    }
}

fn format_trace_tool_attempt_detail(attempt: &crate::core::ToolAttempt) -> TraceToolAttemptDetail {
    TraceToolAttemptDetail {
        id: attempt.id.clone(),
        run_id: attempt.run_id.clone(),
        sequence_no: attempt.sequence_no,
        tool_name: attempt.tool_name.clone(),
        status: attempt.status.as_str().to_string(),
        failure_class: attempt
            .failure_class
            .as_ref()
            .map(|value| format!("{:?}", value).to_ascii_lowercase()),
        retryable: attempt.retryable,
        side_effect_level: attempt.side_effect_level.clone(),
        idempotency_key: attempt.idempotency_key.clone(),
        arguments_json: attempt.arguments_json.clone(),
        output_json: attempt.output_json.clone(),
        started_at: format_trace_time_display(&attempt.started_at, None),
        completed_at: attempt
            .completed_at
            .as_deref()
            .map(|value| format_trace_time_display(value, None)),
        error_text: attempt.error_text.clone(),
    }
}

fn format_trace_operational_log_detail(
    row: &crate::storage::entities::operational_log::Model,
) -> TraceOperationalLogDetail {
    TraceOperationalLogDetail {
        id: row.id.clone(),
        created_at: format_trace_time_display(&row.created_at, None),
        trace_id: row.trace_id.clone(),
        conversation_id: row.conversation_id.clone(),
        channel: row.channel.clone(),
        event_type: row.event_type.clone(),
        success: row.success,
        outcome: row.outcome.clone(),
        tool_name: row.tool_name.clone(),
        latency_ms: row.latency_ms,
        arguments: row.arguments.clone(),
        payload: row.payload.clone(),
        strategy_version: row.strategy_version.clone(),
        policy_version: row.policy_version.clone(),
        prompt_version: row.prompt_version.clone(),
        model_slot: row.model_slot.clone(),
    }
}

fn trace_times_close(left: &str, right: &str) -> bool {
    let Some(left_dt) = chrono::DateTime::parse_from_rfc3339(left).ok() else {
        return false;
    };
    let Some(right_dt) = chrono::DateTime::parse_from_rfc3339(right).ok() else {
        return false;
    };
    let delta = (left_dt - right_dt).num_seconds().abs() as u64;
    delta <= 120
}

fn should_include_operational_log(
    row: &crate::storage::entities::operational_log::Model,
    tool_attempts: &[crate::core::ToolAttempt],
) -> bool {
    if row.event_type != "tool_call" || !row.success {
        return true;
    }
    let Some(tool_name) = row
        .tool_name
        .as_deref()
        .map(str::trim)
        .filter(|value| !value.is_empty())
    else {
        return true;
    };
    !tool_attempts.iter().any(|attempt| {
        attempt.tool_name.eq_ignore_ascii_case(tool_name)
            && (trace_times_close(&attempt.started_at, &row.created_at)
                || attempt
                    .completed_at
                    .as_deref()
                    .map(|value| trace_times_close(value, &row.created_at))
                    .unwrap_or(false))
    })
}

fn build_step_from_execution_trace_step(step: &crate::core::ExecutionStep) -> TraceStep {
    let mut artifacts = Vec::new();
    if let Some(raw_data) = step.data.as_deref() {
        if let Some(artifact) = build_artifact_from_raw("step_data", "Step Data", raw_data) {
            artifacts.push(artifact);
        }
    }
    TraceStep {
        icon: step.icon.clone(),
        title: step.title.clone(),
        detail: step.detail.clone(),
        step_type: step.step_type.clone(),
        data: step.data.clone(),
        time: format_trace_step_time(step),
        source: Some("trace_step".to_string()),
        status: Some(trace_status_for_step(&step.step_type, &step.title)),
        ref_id: None,
        artifacts,
    }
}

fn build_step_from_execution_run(run: &crate::core::ExecutionRun) -> TraceStep {
    let mut artifacts = Vec::new();
    if !run.attempted_models.is_empty() {
        let models = run
            .attempted_models
            .iter()
            .map(|attempt| {
                let status = if attempt.success { "ok" } else { "failed" };
                format!(
                    "{} | {} | {}",
                    attempt.slot_label, attempt.model_name, status
                )
            })
            .collect::<Vec<_>>()
            .join("\n");
        artifacts.push(build_text_artifact(
            "model_attempts",
            "Model Attempts",
            Some("text"),
            &models,
            Some(format!(
                "{} model attempt{}",
                run.attempted_models.len(),
                if run.attempted_models.len() == 1 {
                    ""
                } else {
                    "s"
                }
            )),
        ));
    }
    if !run.degradation.is_empty() {
        let notes = run
            .degradation
            .iter()
            .map(|note| {
                let mut line = title_case_trace_label(&note.kind);
                if !note.summary.trim().is_empty() {
                    line.push_str(": ");
                    line.push_str(note.summary.trim());
                }
                if let Some(detail) = note
                    .detail
                    .as_deref()
                    .map(str::trim)
                    .filter(|value| !value.is_empty())
                {
                    line.push_str(" | ");
                    line.push_str(detail);
                }
                line
            })
            .collect::<Vec<_>>()
            .join("\n");
        artifacts.push(build_text_artifact(
            "degradation",
            "Degradation Notes",
            Some("text"),
            &notes,
            Some(format!(
                "{} degradation note{}",
                run.degradation.len(),
                if run.degradation.len() == 1 { "" } else { "s" }
            )),
        ));
    }
    if let Some(error) = run
        .last_error
        .as_deref()
        .map(str::trim)
        .filter(|value| !value.is_empty())
    {
        artifacts.push(build_text_artifact(
            "run_error",
            "Run Error",
            Some("text"),
            error,
            Some(truncate_trace_text(error, 180)),
        ));
    }
    let detail = run
        .result_summary
        .as_deref()
        .map(str::trim)
        .filter(|value| !value.is_empty())
        .map(|value| truncate_trace_text(value, 220))
        .unwrap_or_else(|| format!("Stage: {}", run.current_stage));
    TraceStep {
        icon: "run".to_string(),
        title: format!(
            "Run Status: {}",
            title_case_trace_label(run.status.as_str())
        ),
        detail,
        step_type: "run_status".to_string(),
        data: render_artifact_blocks(&artifacts),
        time: format_trace_time_display(&run.updated_at, None),
        source: Some("execution_run".to_string()),
        status: Some(match run.status.as_str() {
            "completed" => "success".to_string(),
            "failed" | "panicked" | "platform_failed" => "error".to_string(),
            "blocked" | "needs_permission" | "needs_input" => "warning".to_string(),
            _ => "info".to_string(),
        }),
        ref_id: Some(run.id.clone()),
        artifacts,
    }
}

fn build_step_from_checkpoint(checkpoint: &crate::core::ExecutionCheckpoint) -> TraceStep {
    let artifacts = build_artifact_from_raw(
        "checkpoint_payload",
        "Checkpoint Payload",
        &checkpoint.payload,
    )
    .into_iter()
    .collect::<Vec<_>>();
    TraceStep {
        icon: "checkpoint".to_string(),
        title: format!("Checkpoint: {}", title_case_trace_label(&checkpoint.stage)),
        detail: truncate_trace_text(&checkpoint.payload, 220),
        step_type: "checkpoint".to_string(),
        data: render_artifact_blocks(&artifacts),
        time: format_trace_time_display(&checkpoint.created_at, None),
        source: Some("checkpoint".to_string()),
        status: Some("info".to_string()),
        ref_id: Some(format!("{}:{}", checkpoint.run_id, checkpoint.sequence_no)),
        artifacts,
    }
}

fn append_sql_artifact_if_present(
    artifacts: &mut Vec<TraceArtifact>,
    output_value: &serde_json::Value,
) {
    if let Some(sql) = output_value.get("sql").and_then(|value| value.as_str()) {
        artifacts.push(build_text_artifact(
            "sql_query",
            "SQL Query",
            Some("sql"),
            sql,
            Some(truncate_trace_text(&sql.replace('\n', " "), 180)),
        ));
    }
    if let Some(rows) = output_value.get("rows") {
        let summary = output_value
            .get("row_count")
            .and_then(|value| value.as_u64())
            .map(|count| {
                let table = output_value
                    .get("table")
                    .and_then(|value| value.as_str())
                    .unwrap_or("query");
                format!(
                    "{} row{} returned from {}",
                    count,
                    if count == 1 { "" } else { "s" },
                    table
                )
            });
        artifacts.push(build_json_artifact("sql_result", "Rows", rows, summary));
    }
}

fn build_tool_attempt_artifacts(attempt: &crate::core::ToolAttempt) -> Vec<TraceArtifact> {
    let mut artifacts = Vec::new();
    if let Some(artifact) =
        build_artifact_from_raw("tool_arguments", "Arguments", &attempt.arguments_json)
    {
        artifacts.push(artifact);
    }
    let output_trimmed = attempt.output_json.trim();
    if !output_trimmed.is_empty() {
        if let Some(wrapper) = parse_jsonish_value(output_trimmed) {
            if let Some(content_value) = wrapper.get("content") {
                match content_value {
                    serde_json::Value::String(raw_content) => {
                        if let Some(parsed_content) = parse_jsonish_value(raw_content) {
                            append_sql_artifact_if_present(&mut artifacts, &parsed_content);
                            artifacts.push(build_json_artifact(
                                "tool_output",
                                "Output",
                                &parsed_content,
                                None,
                            ));
                        } else {
                            artifacts.push(build_text_artifact(
                                "tool_output",
                                "Output",
                                Some("text"),
                                raw_content,
                                None,
                            ));
                        }
                    }
                    other => {
                        append_sql_artifact_if_present(&mut artifacts, other);
                        artifacts.push(build_json_artifact("tool_output", "Output", other, None));
                    }
                }
            } else {
                append_sql_artifact_if_present(&mut artifacts, &wrapper);
                artifacts.push(build_json_artifact("tool_output", "Output", &wrapper, None));
            }
            if let Some(error) = wrapper
                .get("error")
                .and_then(|value| value.as_str())
                .map(str::trim)
                .filter(|value| !value.is_empty())
            {
                artifacts.push(build_text_artifact(
                    "tool_error",
                    "Error",
                    Some("text"),
                    error,
                    Some(truncate_trace_text(error, 180)),
                ));
            }
        } else {
            artifacts.push(build_text_artifact(
                "tool_output",
                "Output",
                Some("text"),
                output_trimmed,
                None,
            ));
        }
    }
    if let Some(error_text) = attempt
        .error_text
        .as_deref()
        .map(str::trim)
        .filter(|value| !value.is_empty())
    {
        artifacts.push(build_text_artifact(
            "tool_error",
            "Error",
            Some("text"),
            error_text,
            Some(truncate_trace_text(error_text, 180)),
        ));
    }
    artifacts
}

fn build_step_from_tool_attempt(attempt: &crate::core::ToolAttempt) -> TraceStep {
    let artifacts = build_tool_attempt_artifacts(attempt);
    let detail = format!(
        "Status: {} | side effect: {} | retryable: {}",
        title_case_trace_label(attempt.status.as_str()),
        attempt.side_effect_level,
        if attempt.retryable { "yes" } else { "no" }
    );
    let time_raw = attempt
        .completed_at
        .as_deref()
        .unwrap_or(attempt.started_at.as_str());
    let status = match attempt.status.as_str() {
        "success" => "success",
        "blocked" => "warning",
        _ => "error",
    };
    TraceStep {
        icon: "tool".to_string(),
        title: format!(
            "Tool Result: {}",
            title_case_trace_label(&attempt.tool_name)
        ),
        detail,
        step_type: "tool_result".to_string(),
        data: render_artifact_blocks(&artifacts),
        time: format_trace_time_display(time_raw, None),
        source: Some("tool_attempt".to_string()),
        status: Some(status.to_string()),
        ref_id: Some(attempt.id.clone()),
        artifacts,
    }
}

fn build_step_from_operational_log(
    row: &crate::storage::entities::operational_log::Model,
) -> TraceStep {
    let mut artifacts = Vec::new();
    if let Some(arguments) = row
        .arguments
        .as_deref()
        .map(str::trim)
        .filter(|value| !value.is_empty())
    {
        if let Some(artifact) = build_artifact_from_raw("arguments", "Arguments", arguments) {
            artifacts.push(artifact);
        }
    }
    if let Some(payload) = row
        .payload
        .as_deref()
        .map(str::trim)
        .filter(|value| !value.is_empty())
    {
        if let Some(artifact) = build_artifact_from_raw("payload", "Payload", payload) {
            artifacts.push(artifact);
        }
    }
    let title = if row.event_type == "tool_call" {
        format!(
            "Tool Event: {}",
            row.tool_name
                .as_deref()
                .map(title_case_trace_label)
                .unwrap_or_else(|| "Unknown Tool".to_string())
        )
    } else {
        title_case_trace_label(&row.event_type)
    };
    let mut detail_parts = vec![title_case_trace_label(&row.outcome)];
    if let Some(latency_ms) = row.latency_ms {
        detail_parts.push(format!("{}ms", latency_ms.max(0)));
    }
    if let Some(tool_name) = row
        .tool_name
        .as_deref()
        .map(str::trim)
        .filter(|value| !value.is_empty())
    {
        if row.event_type != "tool_call" {
            detail_parts.push(title_case_trace_label(tool_name));
        }
    }
    TraceStep {
        icon: "log".to_string(),
        title,
        detail: detail_parts.join(" | "),
        step_type: row.event_type.clone(),
        data: render_artifact_blocks(&artifacts),
        time: format_trace_time_display(
            &row.created_at,
            row.latency_ms.map(|value| value.max(0) as u64),
        ),
        source: Some(TRACE_RUNTIME_SOURCE.to_string()),
        status: Some(if row.success {
            "info".to_string()
        } else {
            "warning".to_string()
        }),
        ref_id: Some(row.id.clone()),
        artifacts,
    }
}

fn build_trace_detail_steps(
    parsed_steps: &[crate::core::ExecutionStep],
    enrichment: &TraceEnrichment,
) -> Vec<TraceStep> {
    let mut entries = Vec::<(String, usize, TraceStep)>::new();
    let mut ordinal = 0usize;
    for step in parsed_steps {
        entries.push((
            step.timestamp
                .to_rfc3339_opts(chrono::SecondsFormat::Secs, true),
            ordinal,
            build_step_from_execution_trace_step(step),
        ));
        ordinal += 1;
    }
    if let Some(run) = enrichment.execution_run.as_ref() {
        entries.push((
            run.updated_at.clone(),
            ordinal,
            build_step_from_execution_run(run),
        ));
        ordinal += 1;
    }
    for checkpoint in &enrichment.checkpoints {
        entries.push((
            checkpoint.created_at.clone(),
            ordinal,
            build_step_from_checkpoint(checkpoint),
        ));
        ordinal += 1;
    }
    for row in &enrichment.operational_logs {
        if !should_include_operational_log(row, &enrichment.tool_attempts) {
            continue;
        }
        entries.push((
            row.created_at.clone(),
            ordinal,
            build_step_from_operational_log(row),
        ));
        ordinal += 1;
    }
    for attempt in &enrichment.tool_attempts {
        let sort_key = attempt
            .completed_at
            .clone()
            .unwrap_or_else(|| attempt.started_at.clone());
        entries.push((sort_key, ordinal, build_step_from_tool_attempt(attempt)));
        ordinal += 1;
    }
    entries.sort_by(|left, right| left.0.cmp(&right.0).then(left.1.cmp(&right.1)));
    entries.into_iter().map(|(_, _, step)| step).collect()
}

async fn load_trace_enrichment(
    storage: &crate::storage::Storage,
    trace_id: &str,
) -> TraceEnrichment {
    let execution_run = storage
        .load_execution_run_by_trace_id(trace_id)
        .await
        .ok()
        .flatten();
    let checkpoints = if let Some(run) = execution_run.as_ref() {
        storage
            .load_execution_checkpoints(&run.id)
            .await
            .unwrap_or_default()
    } else {
        Vec::new()
    };
    let tool_attempts = if let Some(run) = execution_run.as_ref() {
        storage
            .list_tool_attempts_for_run(&run.id)
            .await
            .unwrap_or_default()
    } else {
        Vec::new()
    };
    let operational_logs = storage
        .list_operational_logs_for_trace_ids(&[trace_id.to_string()], 96)
        .await
        .unwrap_or_default();
    TraceEnrichment {
        execution_run,
        checkpoints,
        tool_attempts,
        operational_logs,
    }
}

fn parse_rfc3339_to_local_display(value: &Option<String>) -> Option<String> {
    // Send full ISO timestamp; frontend converts to local time.
    value.as_deref().and_then(|raw| {
        chrono::DateTime::parse_from_rfc3339(raw).ok().map(|dt| {
            dt.with_timezone(&chrono::Utc)
                .to_rfc3339_opts(chrono::SecondsFormat::Secs, true)
        })
    })
}

fn parse_rfc3339_to_time_display(value: &Option<String>) -> String {
    // Send full ISO timestamp; frontend converts to local relative time.
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
    t: &crate::storage::ExecutionTraceSummaryRow,
) -> TraceSummary {
    let parsed_steps: Vec<crate::core::ExecutionStep> =
        serde_json::from_str(&t.steps_json).unwrap_or_default();
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
        total_tokens: t.total_tokens as i64,
        cost_usd: t.cost_usd,
        complexity: t.complexity.clone(),
    }
}

fn format_operational_event(
    row: &crate::storage::entities::operational_log::Model,
) -> TraceOperationalEvent {
    TraceOperationalEvent {
        id: row.id.clone(),
        source: TRACE_RUNTIME_SOURCE.to_string(),
        trace_id: row.trace_id.clone(),
        created_at: row.created_at.clone(),
        channel: row.channel.clone(),
        event_type: row.event_type.clone(),
        success: row.success,
        outcome: row.outcome.clone(),
        tool_name: row.tool_name.clone(),
        latency_ms: row.latency_ms,
        details: Some(serde_json::json!({
            "channel": row.channel.clone(),
            "event_type": row.event_type.clone(),
            "success": row.success,
            "outcome": row.outcome.clone(),
            "tool_name": row.tool_name.clone(),
            "latency_ms": row.latency_ms,
            "trace_id": row.trace_id.clone(),
            "conversation_id": row.conversation_id.clone(),
            "arguments": row.arguments.clone(),
            "payload": row.payload.clone(),
            "strategy_version": row.strategy_version.clone(),
            "policy_version": row.policy_version.clone(),
            "prompt_version": row.prompt_version.clone(),
            "model_slot": row.model_slot.clone(),
        })),
    }
}

fn automation_run_status_label(status: &crate::core::AutomationRunStatus) -> &'static str {
    match status {
        crate::core::AutomationRunStatus::Running => "running",
        crate::core::AutomationRunStatus::Succeeded => "succeeded",
        crate::core::AutomationRunStatus::Failed => "failed",
        crate::core::AutomationRunStatus::Retrying => "retrying",
        crate::core::AutomationRunStatus::TimedOut => "timed_out",
        crate::core::AutomationRunStatus::Triggered => "triggered",
    }
}

fn automation_run_success(status: &crate::core::AutomationRunStatus) -> bool {
    matches!(
        status,
        crate::core::AutomationRunStatus::Succeeded
            | crate::core::AutomationRunStatus::Triggered
            | crate::core::AutomationRunStatus::Running
            | crate::core::AutomationRunStatus::Retrying
    )
}

fn trace_automation_channel(run: &crate::core::automation::AutomationRunRecord) -> String {
    run.origin
        .channel
        .as_deref()
        .map(str::trim)
        .filter(|value| !value.is_empty())
        .or_else(|| {
            let trigger = run.trigger.trim();
            (!trigger.is_empty()).then_some(trigger)
        })
        .unwrap_or("agentark")
        .to_string()
}

fn format_automation_event(
    run: &crate::core::automation::AutomationRunRecord,
) -> TraceOperationalEvent {
    let status = automation_run_status_label(&run.status);
    let automation_kind = run.automation_kind.trim();
    let outcome = run
        .error
        .as_deref()
        .map(str::trim)
        .filter(|value| !value.is_empty())
        .or_else(|| {
            run.output_preview
                .as_deref()
                .map(str::trim)
                .filter(|value| !value.is_empty())
        })
        .or_else(|| {
            let summary = run.critique.summary.trim();
            (!summary.is_empty()).then_some(summary)
        })
        .map(|value| truncate_trace_text(value, 220))
        .unwrap_or_else(|| {
            if run.title.trim().is_empty() {
                format!(
                    "{} {}",
                    title_case_trace_label(&run.automation_kind),
                    status
                )
            } else {
                format!(
                    "{}: {}",
                    title_case_trace_label(status),
                    truncate_trace_text(&run.title, 180)
                )
            }
        });
    TraceOperationalEvent {
        id: run.id.clone(),
        source: TRACE_RUNTIME_SOURCE.to_string(),
        trace_id: None,
        created_at: run
            .completed_at
            .clone()
            .unwrap_or_else(|| run.started_at.clone()),
        channel: trace_automation_channel(run),
        event_type: if automation_kind.is_empty() {
            "automation_run".to_string()
        } else {
            format!("{}_run", automation_kind)
        },
        success: automation_run_success(&run.status),
        outcome,
        tool_name: (!run.action.trim().is_empty()).then(|| run.action.clone()),
        latency_ms: run
            .duration_ms
            .map(|value| value.min(i64::MAX as u64) as i64),
        details: Some(serde_json::json!({
            "automation_kind": run.automation_kind.clone(),
            "status": status,
            "trigger": run.trigger.clone(),
            "title": run.title.clone(),
            "action": run.action.clone(),
            "started_at": run.started_at.clone(),
            "completed_at": run.completed_at.clone(),
            "duration_ms": run.duration_ms,
            "error": run.error.clone(),
            "output_preview": run.output_preview.clone(),
            "critique": run.critique.clone(),
        })),
    }
}

fn trace_event_matches_since(
    event: &TraceOperationalEvent,
    since: Option<&chrono::DateTime<chrono::Utc>>,
) -> bool {
    let Some(since) = since else {
        return true;
    };
    chrono::DateTime::parse_from_rfc3339(&event.created_at)
        .map(|value| value.with_timezone(&chrono::Utc) >= *since)
        .unwrap_or(false)
}

fn trace_event_sort_key(event: &TraceOperationalEvent) -> String {
    chrono::DateTime::parse_from_rfc3339(&event.created_at)
        .map(|value| {
            value
                .with_timezone(&chrono::Utc)
                .to_rfc3339_opts(chrono::SecondsFormat::Secs, true)
        })
        .unwrap_or_else(|_| event.created_at.clone())
}

fn insert_trace_activity_event(
    events_by_key: &mut std::collections::BTreeMap<String, TraceOperationalEvent>,
    namespace: &str,
    event: TraceOperationalEvent,
) {
    events_by_key.insert(format!("{}:{}", namespace, event.id), event);
}

fn heartbeat_label_from_key(key: &str) -> &'static str {
    match key {
        crate::sentinel::SENTINEL_SCHEDULER_HEARTBEAT_KEY => "scheduler",
        crate::sentinel::SENTINEL_WATCHER_HEARTBEAT_KEY => "watcher",
        crate::sentinel::SENTINEL_INTEGRATION_SYNC_HEARTBEAT_KEY => "integration_sync",
        crate::sentinel::SENTINEL_APPROVAL_EXPIRY_HEARTBEAT_KEY => "approval_expiry",
        crate::sentinel::SENTINEL_ARKPULSE_HEARTBEAT_KEY => "arkpulse",
        crate::sentinel::SENTINEL_AUTO_ANALYSIS_HEARTBEAT_KEY => "auto_analysis",
        _ => "background",
    }
}

async fn load_trace_background_ping_events(
    storage: &crate::storage::Storage,
    since: Option<&chrono::DateTime<chrono::Utc>>,
) -> Vec<TraceOperationalEvent> {
    let heartbeat_keys = [
        crate::sentinel::SENTINEL_SCHEDULER_HEARTBEAT_KEY,
        crate::sentinel::SENTINEL_WATCHER_HEARTBEAT_KEY,
        crate::sentinel::SENTINEL_INTEGRATION_SYNC_HEARTBEAT_KEY,
        crate::sentinel::SENTINEL_APPROVAL_EXPIRY_HEARTBEAT_KEY,
        crate::sentinel::SENTINEL_ARKPULSE_HEARTBEAT_KEY,
        crate::sentinel::SENTINEL_AUTO_ANALYSIS_HEARTBEAT_KEY,
    ];
    let mut events = Vec::new();
    for key in heartbeat_keys {
        let Some(created_at) = storage
            .get(key)
            .await
            .ok()
            .flatten()
            .and_then(|raw| String::from_utf8(raw).ok())
        else {
            continue;
        };
        let label = heartbeat_label_from_key(key);
        let event = TraceOperationalEvent {
            id: format!("heartbeat:{label}"),
            source: TRACE_RUNTIME_SOURCE.to_string(),
            trace_id: None,
            created_at: created_at.clone(),
            channel: label.to_string(),
            event_type: "background_ping".to_string(),
            success: true,
            outcome: format!("{} heartbeat recorded", title_case_trace_label(label)),
            tool_name: Some(label.to_string()),
            latency_ms: None,
            details: Some(serde_json::json!({
                "heartbeat": label,
                "created_at": created_at,
            })),
        };
        if trace_event_matches_since(&event, since) {
            events.push(event);
        }
    }
    events
}

fn cleanup_event_from_timestamp(
    id: &str,
    created_at: Option<String>,
    channel: &str,
    tool_name: &str,
    outcome: &str,
    since: Option<&chrono::DateTime<chrono::Utc>>,
) -> Option<TraceOperationalEvent> {
    let event = TraceOperationalEvent {
        id: id.to_string(),
        source: TRACE_RUNTIME_SOURCE.to_string(),
        trace_id: None,
        created_at: created_at?,
        channel: channel.to_string(),
        event_type: "cleanup".to_string(),
        success: true,
        outcome: outcome.to_string(),
        tool_name: Some(tool_name.to_string()),
        latency_ms: None,
        details: Some(serde_json::json!({
            "channel": channel,
            "tool_name": tool_name,
            "outcome": outcome,
        })),
    };
    trace_event_matches_since(&event, since).then_some(event)
}

async fn load_trace_cleanup_events(
    storage: &crate::storage::Storage,
    since: Option<&chrono::DateTime<chrono::Utc>>,
) -> Vec<TraceOperationalEvent> {
    let housekeeping = storage.housekeeping_status().await.ok().unwrap_or_default();
    let mut events = Vec::new();
    if let Some(event) = cleanup_event_from_timestamp(
        "cleanup:housekeeping",
        housekeeping.housekeeping_last_run_at,
        "storage",
        "housekeeping",
        "Housekeeping retention cleanup ran",
        since,
    ) {
        events.push(event);
    }
    if let Some(event) = cleanup_event_from_timestamp(
        "cleanup:notifications",
        housekeeping.notification_last_run_at,
        "notifications",
        "notification_retention",
        "Notification retention cleanup ran",
        since,
    ) {
        events.push(event);
    }
    events
}

async fn load_trace_activity_page(
    storage: &crate::storage::Storage,
    since_raw: Option<&str>,
    since: Option<&chrono::DateTime<chrono::Utc>>,
    limit: usize,
    offset: usize,
) -> (usize, Vec<TraceOperationalEvent>) {
    let fetch_limit = limit.saturating_add(offset).max(limit).max(1) as u64;
    let mut events_by_key = std::collections::BTreeMap::<String, TraceOperationalEvent>::new();

    for row in storage
        .list_recent_operational_logs(since_raw, fetch_limit, 0)
        .await
        .unwrap_or_default()
    {
        insert_trace_activity_event(
            &mut events_by_key,
            "operational",
            format_operational_event(&row),
        );
    }

    for run in storage
        .list_automation_runs_since(since_raw, fetch_limit as usize)
        .await
        .unwrap_or_default()
    {
        insert_trace_activity_event(
            &mut events_by_key,
            "automation",
            format_automation_event(&run),
        );
    }

    for event in load_trace_background_ping_events(storage, since).await {
        insert_trace_activity_event(&mut events_by_key, "heartbeat", event);
    }
    for event in load_trace_cleanup_events(storage, since).await {
        insert_trace_activity_event(&mut events_by_key, "cleanup", event);
    }

    let base_total = storage
        .count_operational_logs(since_raw)
        .await
        .unwrap_or(0)
        .saturating_add(storage.count_automation_runs(since_raw).await.unwrap_or(0))
        as usize;
    let synthetic_total = events_by_key
        .iter()
        .filter(|(key, _)| key.starts_with("heartbeat:") || key.starts_with("cleanup:"))
        .count();

    let mut events = events_by_key.into_values().collect::<Vec<_>>();
    events.sort_by(|left, right| {
        trace_event_sort_key(right)
            .cmp(&trace_event_sort_key(left))
            .then(left.id.cmp(&right.id))
    });
    let page = events
        .into_iter()
        .skip(offset)
        .take(limit)
        .collect::<Vec<_>>();

    (base_total.saturating_add(synthetic_total), page)
}

fn format_trace_detail_from_memory(
    t: &ExecutionTrace,
    enrichment: &TraceEnrichment,
) -> TraceDetailResponse {
    let duration_ms = t.started_at.and_then(|start| {
        t.completed_at
            .map(|end| (end - start).num_milliseconds() as u64)
    });
    let steps = build_trace_detail_steps(&t.steps, enrichment);

    TraceDetailResponse {
        id: t.id.clone(),
        message: t.message.clone(),
        channel: t.channel.clone(),
        status: trace_status_from_steps(&t.steps, t.completed_at.is_some()),
        started_at: t
            .started_at
            .map(|s| s.to_rfc3339_opts(chrono::SecondsFormat::Secs, true)),
        completed_at: t
            .completed_at
            .map(|c| c.to_rfc3339_opts(chrono::SecondsFormat::Secs, true)),
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
        execution_run: enrichment
            .execution_run
            .as_ref()
            .map(format_trace_execution_run_detail),
        checkpoints: enrichment
            .checkpoints
            .iter()
            .map(format_trace_checkpoint_detail)
            .collect(),
        tool_attempts: enrichment
            .tool_attempts
            .iter()
            .map(format_trace_tool_attempt_detail)
            .collect(),
        operational_logs: enrichment
            .operational_logs
            .iter()
            .map(format_trace_operational_log_detail)
            .collect(),
    }
}

fn trace_sort_key_from_memory(t: &ExecutionTrace) -> String {
    t.started_at
        .or(t.completed_at)
        .unwrap_or_else(chrono::Utc::now)
        .to_rfc3339()
}

fn trace_sort_key_from_persisted(t: &crate::storage::ExecutionTraceSummaryRow) -> String {
    t.started_at
        .clone()
        .or(t.completed_at.clone())
        .unwrap_or_else(|| t.created_at.clone())
}

fn format_trace_detail_from_persisted(
    t: &crate::storage::entities::execution_trace::Model,
    enrichment: &TraceEnrichment,
) -> TraceDetailResponse {
    let parsed_steps = parse_persisted_trace_steps(t);
    let steps = build_trace_detail_steps(&parsed_steps, enrichment);

    TraceDetailResponse {
        id: t.id.clone(),
        message: if t.message == crate::storage::ENCRYPTED_STORAGE_UNAVAILABLE {
            "Older run details are unavailable after a past password/key change.".to_string()
        } else {
            t.message.clone()
        },
        channel: t.channel.clone(),
        status: trace_status_from_steps(&parsed_steps, t.completed_at.is_some()),
        started_at: parse_rfc3339_to_local_display(&t.started_at),
        completed_at: parse_rfc3339_to_local_display(&t.completed_at),
        duration_ms: t.duration_ms.map(|value| value.max(0) as u64),
        step_count: steps.len(),
        steps,
        response: t.response.clone(),
        proof_id: t.proof_id.clone(),
        model: t.model.clone(),
        input_tokens: t.input_tokens as i64,
        output_tokens: t.output_tokens as i64,
        total_tokens: t.total_tokens as i64,
        cost_usd: t.cost_usd,
        complexity: t.complexity.clone(),
        execution_run: enrichment
            .execution_run
            .as_ref()
            .map(format_trace_execution_run_detail),
        checkpoints: enrichment
            .checkpoints
            .iter()
            .map(format_trace_checkpoint_detail)
            .collect(),
        tool_attempts: enrichment
            .tool_attempts
            .iter()
            .map(format_trace_tool_attempt_detail)
            .collect(),
        operational_logs: enrichment
            .operational_logs
            .iter()
            .map(format_trace_operational_log_detail)
            .collect(),
    }
}

pub(super) async fn get_trace(
    State(state): State<AppState>,
    axum::extract::Query(params): axum::extract::Query<std::collections::HashMap<String, String>>,
) -> Json<TraceResponse> {
    const TRACE_HISTORY_BUFFER: u64 = 100;
    const TRACE_HISTORY_MAX_LIMIT: usize = 200;
    const TRACE_ACTIVITY_DEFAULT_LIMIT: usize = 20;
    const TRACE_ACTIVITY_MAX_LIMIT: usize = 50;
    const SQLITE_MAX_INTEGER: u64 = i64::MAX as u64;
    let history_limit = params
        .get("limit")
        .and_then(|s| s.parse().ok())
        .unwrap_or(20usize)
        .clamp(1, TRACE_HISTORY_MAX_LIMIT);
    let history_offset = params
        .get("offset")
        .and_then(|s| s.parse().ok())
        .unwrap_or(0usize);
    let activity_limit = params
        .get("activity_limit")
        .and_then(|s| s.parse().ok())
        .unwrap_or(TRACE_ACTIVITY_DEFAULT_LIMIT)
        .clamp(1, TRACE_ACTIVITY_MAX_LIMIT);
    let activity_offset = params
        .get("activity_offset")
        .and_then(|s| s.parse().ok())
        .unwrap_or(0usize);
    let normalized_since =
        normalize_trace_since_param(params.get("since").map(|value| value.as_str()));
    let parsed_since = normalized_since
        .as_deref()
        .and_then(|value| chrono::DateTime::parse_from_rfc3339(value).ok())
        .map(|value| value.with_timezone(&chrono::Utc));

    let filtered_memory_history: Vec<ExecutionTrace> = {
        let trace_history = state.trace_history.read().await;
        trace_history
            .iter()
            .filter(|item| trace_matches_since_memory(item, parsed_since.as_ref()))
            .cloned()
            .collect()
    };
    let history_fetch_buffer = (filtered_memory_history.len() as u64).max(TRACE_HISTORY_BUFFER);
    let storage = {
        let agent = state.agent.read().await;
        agent.storage.clone()
    };
    let persisted_fetch_limit = (history_limit as u64)
        .saturating_add(history_offset as u64)
        .saturating_add(history_fetch_buffer)
        .min(SQLITE_MAX_INTEGER);
    let persisted_history = storage
        .list_execution_trace_summaries(normalized_since.as_deref(), persisted_fetch_limit, 0)
        .await
        .unwrap_or_default();
    let persisted_total = storage
        .count_execution_traces(normalized_since.as_deref())
        .await
        .unwrap_or(0) as usize;
    let memory_trace_ids: Vec<String> = filtered_memory_history
        .iter()
        .map(|item| item.id.clone())
        .collect();
    let persisted_memory_overlap = storage
        .count_execution_traces_by_ids(normalized_since.as_deref(), &memory_trace_ids)
        .await
        .unwrap_or(0) as usize;

    let last_trace = state.last_trace.read().await;

    let trace: Vec<TraceStep> = last_trace
        .steps
        .iter()
        .map(build_step_from_execution_trace_step)
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
    for item in &filtered_memory_history {
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
    let history_total = persisted_total
        .saturating_add(memory_trace_ids.len())
        .saturating_sub(persisted_memory_overlap);
    let history: Vec<TraceSummary> = history_all
        .into_iter()
        .skip(history_offset)
        .take(history_limit)
        .map(|(_, summary)| summary)
        .collect();
    let (recent_events_total, recent_events) = load_trace_activity_page(
        &storage,
        normalized_since.as_deref(),
        parsed_since.as_ref(),
        activity_limit,
        activity_offset,
    )
    .await;

    Json(TraceResponse {
        trace,
        proofs,
        history,
        history_total: Some(history_total),
        recent_events,
        recent_events_total,
        recent_events_offset: activity_offset,
        recent_events_limit: activity_limit,
    })
}

pub(super) async fn get_trace_detail(
    State(state): State<AppState>,
    Path(id): Path<String>,
) -> Response {
    let trace_history = state.trace_history.read().await;
    let trace = trace_history.iter().find(|t| t.id == id).cloned();
    drop(trace_history);

    let agent = state.agent.read().await;
    let enrichment = load_trace_enrichment(&agent.storage, &id).await;

    match trace {
        Some(t) => (
            StatusCode::OK,
            Json(format_trace_detail_from_memory(&t, &enrichment)),
        )
            .into_response(),
        None => match agent
            .encrypted_storage
            .get_execution_trace_decrypted(&id)
            .await
        {
            Ok(Some(t)) => (
                StatusCode::OK,
                Json(format_trace_detail_from_persisted(&t, &enrichment)),
            )
                .into_response(),
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
        },
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn sample_trace(
        started_at: Option<chrono::DateTime<chrono::Utc>>,
        completed_at: Option<chrono::DateTime<chrono::Utc>>,
    ) -> ExecutionTrace {
        ExecutionTrace {
            id: "trace-1".to_string(),
            message: "hello".to_string(),
            channel: "chat".to_string(),
            started_at,
            completed_at,
            steps: Vec::new(),
            proof_id: None,
            response: None,
            model: None,
            input_tokens: 0,
            output_tokens: 0,
            total_tokens: 0,
            cost_usd: 0.0,
            complexity: None,
            plan: None,
        }
    }

    #[test]
    fn normalize_trace_since_param_converts_to_utc_rfc3339() {
        let normalized = normalize_trace_since_param(Some("2026-04-03T01:00:00+05:30"));
        assert_eq!(normalized.as_deref(), Some("2026-04-02T19:30:00Z"));
    }

    #[test]
    fn trace_matches_since_memory_prefers_completed_at() {
        let trace = sample_trace(
            Some(
                chrono::DateTime::parse_from_rfc3339("2026-04-02T19:00:00Z")
                    .unwrap()
                    .with_timezone(&chrono::Utc),
            ),
            Some(
                chrono::DateTime::parse_from_rfc3339("2026-04-02T20:00:00Z")
                    .unwrap()
                    .with_timezone(&chrono::Utc),
            ),
        );
        let since = chrono::DateTime::parse_from_rfc3339("2026-04-02T19:30:00Z")
            .unwrap()
            .with_timezone(&chrono::Utc);
        assert!(trace_matches_since_memory(&trace, Some(&since)));
    }

    #[test]
    fn trace_matches_since_memory_rejects_older_started_trace() {
        let trace = sample_trace(
            Some(
                chrono::DateTime::parse_from_rfc3339("2026-04-02T18:00:00Z")
                    .unwrap()
                    .with_timezone(&chrono::Utc),
            ),
            None,
        );
        let since = chrono::DateTime::parse_from_rfc3339("2026-04-02T19:30:00Z")
            .unwrap()
            .with_timezone(&chrono::Utc);
        assert!(!trace_matches_since_memory(&trace, Some(&since)));
    }

    #[test]
    fn operational_events_are_sourced_from_agentark() {
        let event = format_operational_event(&crate::storage::entities::operational_log::Model {
            id: "log-1".to_string(),
            created_at: "2026-04-02T20:00:00Z".to_string(),
            trace_id: None,
            conversation_id: None,
            channel: "scheduler".to_string(),
            event_type: "tool_call".to_string(),
            success: true,
            outcome: "ok".to_string(),
            tool_name: Some("notify_user".to_string()),
            latency_ms: Some(42),
            arguments: None,
            payload: None,
            strategy_version: None,
            policy_version: None,
            prompt_version: None,
            model_slot: None,
        });

        assert_eq!(event.source, "Runtime");
        assert_eq!(event.channel, "scheduler");
    }

    #[test]
    fn automation_events_are_latest_sortable_agentark_activity() {
        let run = crate::core::automation::AutomationRunRecord {
            id: "run-1".to_string(),
            automation_id: "task-1".to_string(),
            automation_kind: "task".to_string(),
            title: "Send reminder".to_string(),
            action: "notify_user".to_string(),
            trigger: "scheduler".to_string(),
            status: crate::core::AutomationRunStatus::Succeeded,
            attempt: 1,
            started_at: "2026-04-02T19:59:00Z".to_string(),
            completed_at: Some("2026-04-02T20:00:00Z".to_string()),
            duration_ms: Some(1000),
            origin: crate::core::automation::AutomationOriginContext {
                channel: Some("scheduler".to_string()),
                ..Default::default()
            },
            policy: crate::core::automation::AutomationExecutionPolicy::default(),
            critique: crate::core::automation::AutomationCritique {
                summary: "completed".to_string(),
                retryable: false,
                validation_passed: true,
            },
            output_preview: None,
            error: None,
            next_retry_at: None,
        };

        let event = format_automation_event(&run);

        assert_eq!(event.source, "Runtime");
        assert_eq!(event.channel, "scheduler");
        assert_eq!(event.created_at, "2026-04-02T20:00:00Z");
        assert_eq!(trace_event_sort_key(&event), "2026-04-02T20:00:00Z");
    }
}
