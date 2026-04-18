use super::agent::ExecutionTrace;
use crate::core::AgentConfig;
use crate::storage::Storage;
use anyhow::anyhow;
use serde::{Deserialize, Serialize};
use serde_json::{json, Value};
use std::collections::HashSet;
use std::path::Path;

pub const OBSERVABILITY_AUTH_TOKEN_SECRET_KEY: &str = "observability_auth_token";
pub const OBSERVABILITY_LOG_KEY: &str = "observability_delivery_log_v1";
const OBSERVABILITY_LOG_LIMIT: usize = 120;
const LANGSMITH_PROJECT_HEADER: &str = "Langsmith-Project";
const LEGACY_LANGSMITH_PROJECT_HEADER: &str = "X-LangSmith-Project";

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ObservabilityDeliveryLog {
    pub id: String,
    pub timestamp: String,
    pub level: String,
    pub event: String,
    pub message: String,
    pub provider: String,
    pub endpoint: String,
    #[serde(default)]
    pub trace_id: Option<String>,
    #[serde(default)]
    pub status_code: Option<u16>,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ObservabilityPrivacyMode {
    MetadataOnly,
    RedactedContent,
    FullContent,
}

impl ObservabilityPrivacyMode {
    pub fn as_str(self) -> &'static str {
        match self {
            Self::MetadataOnly => "metadata_only",
            Self::RedactedContent => "redacted_content",
            Self::FullContent => "full_content",
        }
    }
}

pub fn normalize_observability_provider(value: &str) -> String {
    match value.trim().to_ascii_lowercase().as_str() {
        "langtrace" => "langtrace".to_string(),
        "langsmith" | "langchain" => "langsmith".to_string(),
        "generic_otlp" | "otlp" | "otlp_http" => "generic_otlp".to_string(),
        _ => "langtrace".to_string(),
    }
}

pub fn parse_observability_privacy_mode(value: &str) -> ObservabilityPrivacyMode {
    match value.trim().to_ascii_lowercase().as_str() {
        "full_content" | "full" => ObservabilityPrivacyMode::FullContent,
        "redacted_content" | "redacted" => ObservabilityPrivacyMode::RedactedContent,
        _ => ObservabilityPrivacyMode::MetadataOnly,
    }
}

pub fn normalize_observability_privacy_mode(value: &str) -> String {
    parse_observability_privacy_mode(value).as_str().to_string()
}

pub fn normalize_observability_header_name(value: &str) -> String {
    let trimmed = value.trim();
    if trimmed.is_empty() {
        "x-api-key".to_string()
    } else {
        trimmed.to_string()
    }
}

pub fn normalize_observability_endpoint(provider: &str, endpoint: &str) -> String {
    let trimmed = endpoint.trim().trim_end_matches('/').to_string();
    if trimmed.is_empty() {
        return String::new();
    }
    let normalized_provider = normalize_observability_provider(provider);
    if normalized_provider == "langtrace" {
        if trimmed.ends_with("/api/trace") {
            trimmed
        } else {
            format!("{}/api/trace", trimmed)
        }
    } else if normalized_provider == "langsmith" {
        // LangSmith supports OTLP at /otel/v1/traces
        if trimmed.ends_with("/otel/v1/traces") || trimmed.ends_with("/v1/traces") {
            trimmed
        } else {
            format!("{}/otel/v1/traces", trimmed)
        }
    } else if trimmed.ends_with("/v1/traces") {
        trimmed
    } else {
        format!("{}/v1/traces", trimmed)
    }
}

pub fn has_observability_auth_token(
    config_dir: &Path,
    data_dir: Option<&Path>,
) -> anyhow::Result<bool> {
    let manager =
        crate::core::config::SecureConfigManager::new_with_data_dir(config_dir, data_dir)?;
    Ok(manager
        .get_custom_secret(OBSERVABILITY_AUTH_TOKEN_SECRET_KEY)?
        .map(|value| !value.trim().is_empty())
        .unwrap_or(false))
}

pub fn load_observability_auth_token(
    config_dir: &Path,
    data_dir: Option<&Path>,
) -> anyhow::Result<Option<String>> {
    let manager =
        crate::core::config::SecureConfigManager::new_with_data_dir(config_dir, data_dir)?;
    Ok(manager
        .get_custom_secret(OBSERVABILITY_AUTH_TOKEN_SECRET_KEY)?
        .map(|value| value.trim().to_string())
        .filter(|value| !value.is_empty()))
}

pub async fn load_delivery_logs(
    storage: &Storage,
    _config_dir: &Path,
) -> Vec<ObservabilityDeliveryLog> {
    storage
        .get_encrypted(OBSERVABILITY_LOG_KEY)
        .await
        .ok()
        .flatten()
        .and_then(|bytes| serde_json::from_slice::<Vec<ObservabilityDeliveryLog>>(&bytes).ok())
        .unwrap_or_default()
}

pub async fn append_delivery_log(
    storage: &Storage,
    config_dir: &Path,
    entry: ObservabilityDeliveryLog,
) {
    let mut logs = load_delivery_logs(storage, config_dir).await;
    logs.insert(0, entry);
    if logs.len() > OBSERVABILITY_LOG_LIMIT {
        logs.truncate(OBSERVABILITY_LOG_LIMIT);
    }
    if let Ok(bytes) = serde_json::to_vec(&logs) {
        let _ = storage.set_encrypted(OBSERVABILITY_LOG_KEY, &bytes).await;
    }
}

fn truncate_text(value: &str, max_chars: usize) -> String {
    let trimmed = value.trim();
    if trimmed.chars().count() <= max_chars {
        return trimmed.to_string();
    }
    trimmed.chars().take(max_chars).collect::<String>()
}

fn collapse_whitespace(value: &str) -> String {
    value.split_whitespace().collect::<Vec<_>>().join(" ")
}

fn hash_hex(input: &str, len: usize) -> String {
    let mut hash = blake3::hash(input.as_bytes()).to_hex().to_string();
    hash.truncate(len);
    hash
}

fn otel_attribute_string(key: &str, value: String) -> serde_json::Value {
    json!({ "key": key, "value": { "stringValue": value } })
}

fn otel_attribute_int(key: &str, value: i64) -> serde_json::Value {
    json!({ "key": key, "value": { "intValue": value.to_string() } })
}

fn parse_pipe_kv_metadata(value: &str) -> Vec<(String, String)> {
    value
        .split('|')
        .filter_map(|segment| {
            let (raw_key, raw_value) = segment.split_once('=')?;
            let key = raw_key.trim();
            let rendered_key = key
                .chars()
                .map(|ch| {
                    if ch.is_ascii_alphanumeric() {
                        ch.to_ascii_lowercase()
                    } else {
                        '_'
                    }
                })
                .collect::<String>()
                .trim_matches('_')
                .to_string();
            if rendered_key.is_empty() {
                return None;
            }
            let rendered_value = raw_value.trim().to_string();
            if rendered_value.is_empty()
                || rendered_value.contains('\n')
                || rendered_value.contains('{')
                || rendered_value.contains('[')
            {
                return None;
            }
            Some((rendered_key, rendered_value))
        })
        .collect()
}

fn datetime_to_unix_nanos(dt: chrono::DateTime<chrono::Utc>) -> String {
    dt.timestamp_nanos_opt()
        .unwrap_or_else(|| dt.timestamp_millis() * 1_000_000)
        .to_string()
}

fn redact_by_mode(mode: ObservabilityPrivacyMode, value: &str, max_chars: usize) -> Option<String> {
    let trimmed = value.trim();
    if trimmed.is_empty() {
        return None;
    }
    match mode {
        ObservabilityPrivacyMode::MetadataOnly => None,
        ObservabilityPrivacyMode::RedactedContent => Some(truncate_text(
            &crate::security::redact_pii(trimmed),
            max_chars,
        )),
        ObservabilityPrivacyMode::FullContent => Some(truncate_text(trimmed, max_chars)),
    }
}

fn display_channel_name(channel: &str) -> &'static str {
    match channel.trim().to_ascii_lowercase().as_str() {
        "web" | "chat" | "console" | "ui" => "Chat",
        "task" | "tasks" => "Task",
        "telegram" => "Telegram",
        "whatsapp" => "WhatsApp",
        "gmail" | "email" => "Email",
        "moltbook" => "Moltbook",
        "watcher" | "watchers" => "Watcher",
        _ => "Run",
    }
}

fn build_root_span_name(trace: &ExecutionTrace, mode: ObservabilityPrivacyMode) -> String {
    let channel = display_channel_name(&trace.channel);
    let message_preview = redact_by_mode(mode, &collapse_whitespace(&trace.message), 72);
    match message_preview {
        Some(preview) if !preview.is_empty() => {
            format!("{} {}: {}", crate::branding::PRODUCT_NAME, channel, preview)
        }
        _ => format!("{} {}", crate::branding::PRODUCT_NAME, channel),
    }
}

fn should_export_step(step: &super::agent::ExecutionStep) -> bool {
    !matches!(
        step.title.trim(),
        "Message Received" | "Execution Record Saved" | "Response Complete"
    )
}

fn export_step_name(step: &super::agent::ExecutionStep) -> String {
    match step.title.trim() {
        "LLM Routing Decision" => "Route Request".to_string(),
        "Model Selection" => "Select Model".to_string(),
        "Context Packing" => "Assemble Context".to_string(),
        "Memory Layer" => "Retrieve Memory".to_string(),
        "LLM Response Received" => "Receive LLM Output".to_string(),
        "Clarification Needed" => "Need Clarification".to_string(),
        other => truncate_text(other, 120),
    }
}

fn build_trace_attributes(
    trace: &ExecutionTrace,
    mode: ObservabilityPrivacyMode,
) -> Vec<serde_json::Value> {
    let mut attributes = vec![
        otel_attribute_string("agentark.trace_id", trace.id.clone()),
        otel_attribute_string("agentark.channel", trace.channel.clone()),
        otel_attribute_int("agentark.step_count", trace.steps.len() as i64),
        otel_attribute_int("agentark.total_tokens", trace.total_tokens),
        otel_attribute_string(
            "agentark.status",
            if trace.completed_at.is_some() {
                "completed".to_string()
            } else {
                "running".to_string()
            },
        ),
    ];
    if let Some(model) = trace
        .model
        .as_ref()
        .filter(|value| !value.trim().is_empty())
    {
        attributes.push(otel_attribute_string("agentark.model", model.clone()));
    }
    if let Some(complexity) = trace
        .complexity
        .as_ref()
        .filter(|value| !value.trim().is_empty())
    {
        attributes.push(otel_attribute_string(
            "agentark.complexity",
            complexity.clone(),
        ));
    }
    if let Some(message) = redact_by_mode(mode, &trace.message, 1200) {
        attributes.push(otel_attribute_string("agentark.message", message.clone()));
        // LangSmith reads these for the Input/Output panels
        attributes.push(otel_attribute_string("gen_ai.prompt", message.clone()));
        attributes.push(otel_attribute_string("input", message));
    } else {
        attributes.push(otel_attribute_int(
            "agentark.message_chars",
            trace.message.chars().count() as i64,
        ));
    }
    if let Some(response) = trace.response.as_ref() {
        if let Some(rendered) = redact_by_mode(mode, response, 2400) {
            attributes.push(otel_attribute_string("agentark.response", rendered.clone()));
            attributes.push(otel_attribute_string("gen_ai.completion", rendered.clone()));
            attributes.push(otel_attribute_string("output", rendered));
        } else {
            attributes.push(otel_attribute_int(
                "agentark.response_chars",
                response.chars().count() as i64,
            ));
        }
    }
    attributes
}

fn sanitize_prompt_section_key(section: &str) -> String {
    section
        .chars()
        .map(|ch| {
            if ch.is_ascii_alphanumeric() {
                ch.to_ascii_lowercase()
            } else {
                '_'
            }
        })
        .collect::<String>()
        .trim_matches('_')
        .to_string()
}

fn telemetry_i64(value: Option<&Value>) -> Option<i64> {
    value.and_then(|entry| entry.as_i64().or_else(|| entry.as_u64().map(|n| n as i64)))
}

fn telemetry_string(value: Option<&Value>) -> Option<String> {
    value
        .and_then(|entry| entry.as_str())
        .map(str::trim)
        .filter(|entry| !entry.is_empty())
        .map(str::to_string)
}

fn append_prompt_telemetry_attributes(attributes: &mut Vec<serde_json::Value>, data: &str) {
    let Ok(payload) = serde_json::from_str::<Value>(data) else {
        return;
    };
    if payload
        .get("trace_kind")
        .and_then(|value| value.as_str())
        .map(str::trim)
        != Some("prompt_telemetry")
    {
        return;
    }

    for (key, attr_key) in [
        (
            "assembled_system_prompt_chars",
            "agentark.prompt.assembled_system_prompt_chars",
        ),
        (
            "final_system_prompt_chars",
            "agentark.prompt.final_system_prompt_chars",
        ),
        ("user_message_chars", "agentark.prompt.user_message_chars"),
        ("history_chars", "agentark.prompt.history_chars"),
        ("prompt_chars", "agentark.prompt.prompt_chars"),
        ("tool_count", "agentark.prompt.tool_count"),
        ("tool_schema_chars", "agentark.prompt.tool_schema_chars"),
        (
            "estimated_total_request_chars",
            "agentark.prompt.estimated_total_request_chars",
        ),
        (
            "estimated_total_request_tokens",
            "agentark.prompt.estimated_total_request_tokens",
        ),
        ("section_sum_chars", "agentark.prompt.section_sum_chars"),
        ("untracked_chars", "agentark.prompt.untracked_chars"),
        ("attempt", "agentark.prompt.attempt"),
    ] {
        if let Some(value) = telemetry_i64(payload.get(key)) {
            attributes.push(otel_attribute_int(attr_key, value));
        }
    }

    for (key, attr_key) in [
        ("provider", "agentark.prompt.provider"),
        ("model", "agentark.prompt.model"),
        ("model_slot", "agentark.prompt.model_slot"),
        ("tool_schema_format", "agentark.prompt.tool_schema_format"),
        ("request_mode", "agentark.prompt.request_mode"),
    ] {
        if let Some(value) = telemetry_string(payload.get(key)) {
            attributes.push(otel_attribute_string(attr_key, value));
        }
    }

    if let Some(sections) = payload.get("sections").and_then(|value| value.as_object()) {
        for (section, value) in sections {
            let sanitized = sanitize_prompt_section_key(section);
            if sanitized.is_empty() {
                continue;
            }
            if let Some(chars) = telemetry_i64(Some(value)) {
                attributes.push(otel_attribute_int(
                    &format!("agentark.prompt.section.{}.chars", sanitized),
                    chars,
                ));
            }
        }
    }
}

fn build_step_attributes(
    trace: &ExecutionTrace,
    step: &super::agent::ExecutionStep,
    mode: ObservabilityPrivacyMode,
) -> Vec<serde_json::Value> {
    let mut attributes = vec![
        otel_attribute_string("agentark.trace_id", trace.id.clone()),
        otel_attribute_string("agentark.step_type", step.step_type.clone()),
        otel_attribute_string("agentark.step_title", step.title.clone()),
        otel_attribute_string("agentark.icon", step.icon.clone()),
    ];
    if let Some(duration_ms) = step.duration_ms {
        attributes.push(otel_attribute_int(
            "agentark.duration_ms",
            duration_ms as i64,
        ));
    }
    if let Some(data) = step.data.as_ref() {
        for (key, value) in parse_pipe_kv_metadata(data) {
            let attr_key = format!("agentark.{}", key);
            if let Ok(int_value) = value.parse::<i64>() {
                attributes.push(otel_attribute_int(&attr_key, int_value));
            } else {
                attributes.push(otel_attribute_string(&attr_key, value));
            }
        }
        append_prompt_telemetry_attributes(&mut attributes, data);
    }
    // LangSmith: populate input/output on step spans so tool calls show data
    if let Some(detail) = redact_by_mode(mode, &step.detail, 2000) {
        attributes.push(otel_attribute_string("agentark.detail", detail.clone()));
        attributes.push(otel_attribute_string("input", detail));
    }
    if let Some(data) = step.data.as_ref() {
        if let Some(rendered) = redact_by_mode(mode, data, 3000) {
            attributes.push(otel_attribute_string("agentark.data", rendered.clone()));
            attributes.push(otel_attribute_string("output", rendered));
        } else {
            attributes.push(otel_attribute_int(
                "agentark.data_chars",
                data.chars().count() as i64,
            ));
        }
    }
    attributes
}

fn build_otlp_trace_payload(config: &AgentConfig, trace: &ExecutionTrace) -> serde_json::Value {
    let requested_mode = parse_observability_privacy_mode(&config.observability.privacy_mode);
    let mode = if config.deployment_mode == crate::core::config::DeploymentMode::InternetFacing
        && requested_mode == ObservabilityPrivacyMode::FullContent
    {
        ObservabilityPrivacyMode::RedactedContent
    } else {
        requested_mode
    };
    let trace_id = hash_hex(&trace.id, 32);
    let root_span_id = hash_hex(&format!("{}:root", trace.id), 16);
    let start_time = trace.started_at.unwrap_or_else(chrono::Utc::now);
    let end_time = trace.completed_at.unwrap_or_else(chrono::Utc::now);
    let mut spans = vec![json!({
        "traceId": trace_id,
        "spanId": root_span_id,
        "name": build_root_span_name(trace, mode),
        "startTimeUnixNano": datetime_to_unix_nanos(start_time),
        "endTimeUnixNano": datetime_to_unix_nanos(end_time),
        "attributes": build_trace_attributes(trace, mode),
    })];

    for (index, step) in trace.steps.iter().enumerate() {
        if !should_export_step(step) {
            continue;
        }
        let step_end = if index + 1 < trace.steps.len() {
            trace.steps[index + 1].timestamp
        } else {
            end_time
        };
        let step_start = step.timestamp;
        // Use explicit duration if available, otherwise infer from next step timestamp
        let (effective_start, effective_end) = if let Some(ms) = step.duration_ms {
            let dur_start = step.timestamp - chrono::Duration::milliseconds(ms as i64);
            (dur_start, step.timestamp)
        } else {
            (step_start, step_end)
        };
        let step_span_id = hash_hex(&format!("{}:step:{}", trace.id, index), 16);
        spans.push(json!({
            "traceId": trace_id,
            "spanId": step_span_id,
            "parentSpanId": root_span_id,
            "name": export_step_name(step),
            "startTimeUnixNano": datetime_to_unix_nanos(effective_start),
            "endTimeUnixNano": datetime_to_unix_nanos(effective_end),
            "attributes": build_step_attributes(trace, step, mode),
        }));
    }

    json!({
        "resourceSpans": [
            {
                "resource": {
                    "attributes": [
                        otel_attribute_string(
                            "service.name",
                            if config.observability.service_name.trim().is_empty() {
                                "agentark".to_string()
                            } else {
                                config.observability.service_name.trim().to_string()
                            }
                        ),
                        otel_attribute_string("service.version", env!("CARGO_PKG_VERSION").to_string()),
                        otel_attribute_string("telemetry.sdk.name", "agentark".to_string()),
                    ]
                },
                "scopeSpans": [
                    {
                        "scope": {
                            "name": "agentark.observability",
                            "version": env!("CARGO_PKG_VERSION"),
                        },
                        "spans": spans
                    }
                ]
            }
        ]
    })
}

async fn post_export_payload(
    provider: &str,
    endpoint: &str,
    header_name: &str,
    auth_token: &str,
    service_name: &str,
    payload: &serde_json::Value,
) -> anyhow::Result<reqwest::Response> {
    let client = reqwest::Client::builder()
        .timeout(std::time::Duration::from_secs(8))
        .build()?;
    let mut request = client
        .post(endpoint)
        .header(reqwest::header::CONTENT_TYPE, "application/json")
        .json(payload);
    if !header_name.trim().is_empty() && !auth_token.trim().is_empty() {
        request = request.header(header_name.trim(), auth_token.trim());
    }
    // LangSmith OTLP ingestion uses the Langsmith-Project header. Keep the legacy
    // X-LangSmith-Project header alongside it for compatibility with older setups.
    if normalize_observability_provider(provider) == "langsmith" {
        let project = if service_name.trim().is_empty() {
            "default"
        } else {
            service_name.trim()
        };
        tracing::info!(
            "LangSmith export: project='{}', endpoint='{}'",
            project,
            endpoint
        );
        request = request
            .header(LANGSMITH_PROJECT_HEADER, project)
            .header(LEGACY_LANGSMITH_PROJECT_HEADER, project);
    }
    Ok(request.send().await?)
}

pub async fn export_execution_trace(
    config: &AgentConfig,
    config_dir: &Path,
    data_dir: &Path,
    storage: &Storage,
    trace: &ExecutionTrace,
    event: &str,
) -> anyhow::Result<()> {
    if !config.observability.enabled {
        return Ok(());
    }

    let endpoint = normalize_observability_endpoint(
        &config.observability.provider,
        &config.observability.endpoint,
    );
    if endpoint.is_empty() {
        return Ok(());
    }

    let auth_token = load_observability_auth_token(config_dir, Some(data_dir))?;
    let Some(auth_token) = auth_token.filter(|value| !value.trim().is_empty()) else {
        return Err(anyhow!(
            "Observability export is enabled but no auth token is configured"
        ));
    };

    let payload = build_otlp_trace_payload(config, trace);
    let provider = normalize_observability_provider(&config.observability.provider);
    let service_name = if config.observability.service_name.trim().is_empty() {
        "agentark"
    } else {
        config.observability.service_name.trim()
    };
    match post_export_payload(
        &provider,
        &endpoint,
        &normalize_observability_header_name(&config.observability.header_name),
        &auth_token,
        service_name,
        &payload,
    )
    .await
    {
        Ok(response) => {
            let status = response.status();
            if status.is_success() {
                append_delivery_log(
                    storage,
                    config_dir,
                    ObservabilityDeliveryLog {
                        id: uuid::Uuid::new_v4().to_string(),
                        timestamp: chrono::Utc::now().to_rfc3339(),
                        level: "success".to_string(),
                        event: event.to_string(),
                        message: format!(
                            "Exported trace with {} step(s) to {}.",
                            trace.steps.len(),
                            provider
                        ),
                        provider,
                        endpoint,
                        trace_id: Some(trace.id.clone()),
                        status_code: Some(status.as_u16()),
                    },
                )
                .await;
                Ok(())
            } else {
                let body = response.text().await.unwrap_or_default();
                let message = truncate_text(&body, 280);
                append_delivery_log(
                    storage,
                    config_dir,
                    ObservabilityDeliveryLog {
                        id: uuid::Uuid::new_v4().to_string(),
                        timestamp: chrono::Utc::now().to_rfc3339(),
                        level: "error".to_string(),
                        event: event.to_string(),
                        message: if message.is_empty() {
                            format!("Export failed with HTTP {}.", status.as_u16())
                        } else {
                            format!("Export failed with HTTP {}: {}", status.as_u16(), message)
                        },
                        provider,
                        endpoint,
                        trace_id: Some(trace.id.clone()),
                        status_code: Some(status.as_u16()),
                    },
                )
                .await;
                Err(anyhow!(
                    "Observability export failed with HTTP {}",
                    status.as_u16()
                ))
            }
        }
        Err(error) => {
            append_delivery_log(
                storage,
                config_dir,
                ObservabilityDeliveryLog {
                    id: uuid::Uuid::new_v4().to_string(),
                    timestamp: chrono::Utc::now().to_rfc3339(),
                    level: "error".to_string(),
                    event: event.to_string(),
                    message: truncate_text(&error.to_string(), 280),
                    provider,
                    endpoint,
                    trace_id: Some(trace.id.clone()),
                    status_code: None,
                },
            )
            .await;
            Err(error)
        }
    }
}

pub async fn export_test_trace(
    config: &AgentConfig,
    config_dir: &Path,
    data_dir: &Path,
    storage: &Storage,
) -> anyhow::Result<()> {
    if !config.observability.enabled {
        return Err(anyhow!(
            "Enable observability export before sending a test trace"
        ));
    }
    let endpoint = normalize_observability_endpoint(
        &config.observability.provider,
        &config.observability.endpoint,
    );
    if endpoint.is_empty() {
        return Err(anyhow!(
            "Set an observability endpoint before sending a test trace"
        ));
    }

    let trace = ExecutionTrace {
        id: uuid::Uuid::new_v4().to_string(),
        message: "Observability test export".to_string(),
        channel: "settings".to_string(),
        started_at: Some(chrono::Utc::now() - chrono::Duration::seconds(1)),
        completed_at: Some(chrono::Utc::now()),
        steps: vec![super::agent::ExecutionStep {
            icon: "[test]".to_string(),
            title: "Observability Test".to_string(),
            detail: "Test export triggered from Settings.".to_string(),
            step_type: "info".to_string(),
            data: Some("origin=settings".to_string()),
            timestamp: chrono::Utc::now(),
            duration_ms: Some(1000),
        }],
        proof_id: None,
        response: Some("Observability test complete.".to_string()),
        model: None,
        input_tokens: 0,
        output_tokens: 0,
        total_tokens: 0,
        cost_usd: 0.0,
        complexity: Some("simple".to_string()),
        plan: None,
    };

    export_execution_trace(config, config_dir, data_dir, storage, &trace, "test_export").await
}

pub fn summarize_log_issues(logs: &[ObservabilityDeliveryLog]) -> Vec<String> {
    // Only show errors that are more recent than the last success.
    // Logs are ordered newest-first.
    let mut out = Vec::new();
    let mut seen = HashSet::new();
    for log in logs {
        if log.level == "success" {
            break; // A success clears all older errors
        }
        if log.level == "error" {
            let key = format!("{}|{}", log.event, log.message);
            if seen.insert(key) {
                out.push(log.message.clone());
            }
            if out.len() >= 5 {
                break;
            }
        }
    }
    out
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn normalize_langsmith_provider_accepts_aliases() {
        assert_eq!(normalize_observability_provider("langsmith"), "langsmith");
        assert_eq!(normalize_observability_provider("langchain"), "langsmith");
    }

    #[test]
    fn normalize_langsmith_endpoint_appends_otel_trace_path() {
        assert_eq!(
            normalize_observability_endpoint("langsmith", "https://api.smith.langchain.com"),
            "https://api.smith.langchain.com/otel/v1/traces"
        );
    }
}
