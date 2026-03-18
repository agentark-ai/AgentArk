use super::{agent::ExecutionTrace, config::AgentConfig};
use crate::storage::Storage;
use anyhow::anyhow;
use serde::{Deserialize, Serialize};
use serde_json::json;
use std::collections::HashSet;
use std::path::Path;

pub const OBSERVABILITY_AUTH_TOKEN_SECRET_KEY: &str = "observability_auth_token";
pub const OBSERVABILITY_LOG_KEY: &str = "observability_delivery_log_v1";
const OBSERVABILITY_LOG_LIMIT: usize = 120;

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

pub async fn load_delivery_logs(storage: &Storage) -> Vec<ObservabilityDeliveryLog> {
    let raw = match storage.get_encrypted(OBSERVABILITY_LOG_KEY).await {
        Ok(Some(bytes)) => bytes,
        _ => return Vec::new(),
    };
    serde_json::from_slice::<Vec<ObservabilityDeliveryLog>>(&raw).unwrap_or_default()
}

pub async fn append_delivery_log(storage: &Storage, entry: ObservabilityDeliveryLog) {
    let mut logs = load_delivery_logs(storage).await;
    logs.insert(0, entry);
    if logs.len() > OBSERVABILITY_LOG_LIMIT {
        logs.truncate(OBSERVABILITY_LOG_LIMIT);
    }
    if let Ok(bytes) = serde_json::to_vec(&logs) {
        let _ = storage.set_encrypted(OBSERVABILITY_LOG_KEY, &bytes).await;
    }
}

pub fn observability_is_ready(config: &AgentConfig, auth_token: Option<&str>) -> bool {
    config.observability.enabled
        && !normalize_observability_endpoint(
            &config.observability.provider,
            &config.observability.endpoint,
        )
        .is_empty()
        && auth_token
            .map(|value| !value.trim().is_empty())
            .unwrap_or(false)
}

fn truncate_text(value: &str, max_chars: usize) -> String {
    let trimmed = value.trim();
    if trimmed.chars().count() <= max_chars {
        return trimmed.to_string();
    }
    trimmed.chars().take(max_chars).collect::<String>()
}

fn hash_hex(input: &str, len: usize) -> String {
    let mut hash = blake3::hash(input.as_bytes()).to_hex().to_string();
    hash.truncate(len);
    hash
}

fn otel_attribute_string(key: &str, value: String) -> serde_json::Value {
    json!({ "key": key, "value": { "stringValue": value } })
}

fn otel_attribute_bool(key: &str, value: bool) -> serde_json::Value {
    json!({ "key": key, "value": { "boolValue": value } })
}

fn otel_attribute_int(key: &str, value: i64) -> serde_json::Value {
    json!({ "key": key, "value": { "intValue": value.to_string() } })
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

fn build_step_attributes(
    trace: &ExecutionTrace,
    step: &super::agent::ExecutionStep,
    mode: ObservabilityPrivacyMode,
) -> Vec<serde_json::Value> {
    let mut attributes = vec![
        otel_attribute_string("agentark.trace_id", trace.id.clone()),
        otel_attribute_string("agentark.step_type", step.step_type.clone()),
        otel_attribute_string("agentark.icon", step.icon.clone()),
    ];
    if let Some(duration_ms) = step.duration_ms {
        attributes.push(otel_attribute_int(
            "agentark.duration_ms",
            duration_ms as i64,
        ));
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
        "name": "AgentArk Run",
        "startTimeUnixNano": datetime_to_unix_nanos(start_time),
        "endTimeUnixNano": datetime_to_unix_nanos(end_time),
        "attributes": build_trace_attributes(trace, mode),
    })];

    for (index, step) in trace.steps.iter().enumerate() {
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
            "name": truncate_text(&step.title, 120),
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
    // LangSmith requires project name header for OTLP ingestion
    if endpoint.contains("smith.langchain") || endpoint.contains("langsmith") {
        let project = if service_name.trim().is_empty() { "default" } else { service_name.trim() };
        tracing::info!("LangSmith export: project='{}', endpoint='{}'", project, endpoint);
        request = request.header("X-LangSmith-Project", project);
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
