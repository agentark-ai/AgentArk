use super::*;
use anyhow::{anyhow, Context, Result};
use serde::{de::DeserializeOwned, Deserialize, Serialize};
use sha2::{Digest, Sha256};
use std::collections::HashSet;

const WEBHOOK_SOURCES_KEY: &str = "webhooks:sources:v1";
const WEBHOOK_EVENTS_KEY: &str = "webhooks:events:v1";
const WEBHOOK_SECRET_PREFIX: &str = "webhook_source_secret:";
const WEBHOOK_EVENT_HISTORY_LIMIT: usize = 200;
const WEBHOOK_DEFAULT_DEDUPE_WINDOW_SECS: u64 = 15 * 60;
const WEBHOOK_EXCERPT_MAX_CHARS: usize = 1200;
const WEBHOOK_SUMMARY_MAX_CHARS: usize = 320;
const WEBHOOK_PROMPT_EXCERPT_MAX_CHARS: usize = 2000;

#[derive(Debug, Clone, Copy, Serialize, Deserialize, PartialEq, Eq, Default)]
#[serde(rename_all = "snake_case")]
pub(super) enum WebhookAuthMode {
    #[default]
    HeaderToken,
    None,
    BearerToken,
    HmacSha256,
}

#[derive(Debug, Clone, Copy, Serialize, Deserialize, PartialEq, Eq, Default)]
#[serde(rename_all = "snake_case")]
pub(super) enum WebhookMatchMode {
    #[default]
    All,
    FailuresOnly,
    ChangesOnly,
}

#[derive(Debug, Clone, Copy, Serialize, Deserialize, PartialEq, Eq, Default)]
#[serde(rename_all = "snake_case")]
pub(super) enum WebhookOutputTarget {
    #[default]
    None,
    Preferred,
    Channel,
}

fn default_notify_on_queued() -> bool {
    false
}

fn default_notify_on_success() -> bool {
    true
}

fn default_notify_on_failure() -> bool {
    true
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub(super) struct WebhookSource {
    pub id: String,
    pub name: String,
    pub provider: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub description: Option<String>,
    pub enabled: bool,
    pub auth_mode: WebhookAuthMode,
    pub match_mode: WebhookMatchMode,
    pub instruction: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub event_header: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub secret_header: Option<String>,
    #[serde(default)]
    pub allow_duplicate: bool,
    #[serde(default)]
    pub require_approval: bool,
    #[serde(default)]
    pub dedupe_window_secs: u64,
    #[serde(default = "default_notify_on_queued")]
    pub notify_on_queued: bool,
    #[serde(default = "default_notify_on_success")]
    pub notify_on_success: bool,
    #[serde(default = "default_notify_on_failure")]
    pub notify_on_failure: bool,
    #[serde(default)]
    pub output_target: WebhookOutputTarget,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub output_channel: Option<String>,
    pub conversation_id: String,
    pub created_at: String,
    pub updated_at: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub last_received_at: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub last_outcome: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub last_task_id: Option<String>,
}

#[derive(Debug, Clone, Serialize)]
struct WebhookSourceResponse {
    #[serde(flatten)]
    source: WebhookSource,
    ingest_path: String,
    secret_configured: bool,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub(super) struct WebhookEventRecord {
    pub id: String,
    pub source_id: String,
    pub source_name: String,
    pub provider: String,
    pub received_at: String,
    pub event_type: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub status: Option<String>,
    pub subject: String,
    pub outcome: String,
    pub matched: bool,
    #[serde(default)]
    pub queued: bool,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub message: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub event_id: Option<String>,
    pub dedupe_key: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub event_url: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub payload_excerpt: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub task_id: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub conversation_id: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub severity: Option<String>,
    #[serde(default)]
    pub test_event: bool,
}

#[derive(Debug, Deserialize)]
pub(super) struct WebhookSourceUpsertRequest {
    #[serde(default)]
    pub id: Option<String>,
    pub name: String,
    #[serde(default)]
    pub provider: Option<String>,
    #[serde(default)]
    pub description: Option<String>,
    #[serde(default)]
    pub enabled: Option<bool>,
    #[serde(default)]
    pub auth_mode: Option<WebhookAuthMode>,
    #[serde(default)]
    pub match_mode: Option<WebhookMatchMode>,
    #[serde(default)]
    pub instruction: Option<String>,
    #[serde(default)]
    pub event_header: Option<String>,
    #[serde(default)]
    pub secret_header: Option<String>,
    #[serde(default)]
    pub secret: Option<String>,
    #[serde(default)]
    pub clear_secret: Option<bool>,
    #[serde(default)]
    pub allow_duplicate: Option<bool>,
    #[serde(default)]
    pub require_approval: Option<bool>,
    #[serde(default)]
    pub dedupe_window_secs: Option<u64>,
    #[serde(default)]
    pub notify_on_queued: Option<bool>,
    #[serde(default)]
    pub notify_on_success: Option<bool>,
    #[serde(default)]
    pub notify_on_failure: Option<bool>,
    #[serde(default)]
    pub output_target: Option<WebhookOutputTarget>,
    #[serde(default)]
    pub output_channel: Option<String>,
}

#[derive(Debug, Deserialize, Default)]
pub(super) struct WebhookEventsQuery {
    #[serde(default)]
    pub source_id: Option<String>,
    #[serde(default)]
    pub limit: Option<usize>,
}

#[derive(Debug, Deserialize, Default)]
pub(super) struct WebhookTestRequest {
    #[serde(default)]
    pub event_type: Option<String>,
    #[serde(default)]
    pub status: Option<String>,
    #[serde(default)]
    pub subject: Option<String>,
    #[serde(default)]
    pub payload: Option<serde_json::Value>,
}

#[derive(Debug, Clone)]
struct NormalizedWebhookEvent {
    event_type: String,
    status: Option<String>,
    subject: String,
    event_id: Option<String>,
    event_url: Option<String>,
    summary: String,
    payload_excerpt: String,
    dedupe_key: String,
    severity: Option<String>,
    is_failure: bool,
    is_change: bool,
}

struct PluginWebhookDispatch<'a> {
    source: &'a WebhookSource,
    event: &'a NormalizedWebhookEvent,
    outcome: &'a str,
    matched: bool,
    queued: bool,
    task_id: Option<&'a str>,
    message: Option<&'a str>,
    test_event: bool,
}

struct RouteEventInput<'a> {
    source_index: usize,
    source: &'a WebhookSource,
    headers: &'a HeaderMap,
    raw_body: &'a str,
    payload: Option<&'a serde_json::Value>,
    test_event: bool,
}

fn now_rfc3339() -> String {
    chrono::Utc::now().to_rfc3339()
}

fn webhook_secret_key(source_id: &str) -> String {
    format!("{}{}", WEBHOOK_SECRET_PREFIX, source_id.trim())
}

async fn load_json<T>(storage: &crate::storage::Storage, key: &str) -> Result<T>
where
    T: DeserializeOwned + Default,
{
    let Some(bytes) = storage.get_encrypted(key).await? else {
        return Ok(T::default());
    };
    serde_json::from_slice::<T>(&bytes)
        .with_context(|| format!("failed to decode webhook payload for {}", key))
}

async fn save_json<T>(storage: &crate::storage::Storage, key: &str, value: &T) -> Result<()>
where
    T: Serialize + ?Sized,
{
    let bytes = serde_json::to_vec(value).with_context(|| format!("failed to encode {}", key))?;
    storage.set_encrypted(key, &bytes).await
}

async fn load_sources(storage: &crate::storage::Storage) -> Result<Vec<WebhookSource>> {
    load_json(storage, WEBHOOK_SOURCES_KEY).await
}

async fn save_sources(storage: &crate::storage::Storage, sources: &[WebhookSource]) -> Result<()> {
    save_json(storage, WEBHOOK_SOURCES_KEY, sources).await
}

async fn load_events(storage: &crate::storage::Storage) -> Result<Vec<WebhookEventRecord>> {
    load_json(storage, WEBHOOK_EVENTS_KEY).await
}

async fn save_events(
    storage: &crate::storage::Storage,
    events: &[WebhookEventRecord],
) -> Result<()> {
    save_json(storage, WEBHOOK_EVENTS_KEY, events).await
}

fn present_source(source: &WebhookSource, secret_configured: bool) -> WebhookSourceResponse {
    WebhookSourceResponse {
        source: source.clone(),
        ingest_path: format!("/webhook/inbound/{}", source.id),
        secret_configured,
    }
}

fn sanitize_source_id(value: &str) -> String {
    value
        .trim()
        .to_ascii_lowercase()
        .chars()
        .map(|ch| {
            if ch.is_ascii_alphanumeric() || matches!(ch, '-' | '_' | '.') {
                ch
            } else if ch.is_ascii_whitespace() || matches!(ch, '/' | '\\') {
                '-'
            } else {
                '_'
            }
        })
        .collect::<String>()
        .trim_matches(|c| c == '-' || c == '_')
        .to_string()
}

fn sanitize_provider(value: Option<&str>) -> String {
    let raw = value.unwrap_or("generic").trim();
    let normalized = sanitize_source_id(raw);
    if normalized.is_empty() {
        "generic".to_string()
    } else {
        normalized
    }
}

fn sanitize_header_name(value: Option<&str>) -> Option<String> {
    value
        .map(|s| s.trim().to_string())
        .filter(|s| !s.is_empty())
        .and_then(|candidate| {
            let valid = candidate
                .chars()
                .all(|ch| ch.is_ascii_alphanumeric() || ch == '-');
            if valid {
                Some(candidate)
            } else {
                None
            }
        })
}

fn sanitize_output_channel(value: Option<&str>) -> Option<String> {
    value
        .map(|s| s.trim().to_ascii_lowercase())
        .filter(|s| !s.is_empty())
}

async fn available_completion_channels(state: &AppState) -> Result<HashSet<String>> {
    let (config_dir, data_dir, infos) = {
        let agent = state.agent.read().await;
        (
            agent.config_dir.clone(),
            agent.data_dir.clone(),
            agent.integrations.list().await,
        )
    };

    let mut channels = infos
        .into_iter()
        .filter(|info| {
            info.capabilities
                .contains(&crate::integrations::Capability::Notify)
                && matches!(
                    info.status,
                    crate::integrations::IntegrationStatus::Connected
                )
        })
        .map(|info| info.id.trim().to_ascii_lowercase())
        .filter(|id| !id.is_empty())
        .collect::<HashSet<_>>();

    let manager =
        crate::core::config::SecureConfigManager::new_with_data_dir(&config_dir, Some(&data_dir))?;
    let has_legacy_gmail = manager
        .get_custom_secret("gmail_tokens")?
        .is_some_and(|value| !value.trim().is_empty());
    let has_workspace_gmail = crate::actions::google_workspace::granted_bundles(&config_dir)
        .map(|bundles| bundles.iter().any(|bundle| bundle == "gmail"))
        .unwrap_or(false);
    let config = manager
        .load()
        .unwrap_or_else(|_| crate::core::config::AgentConfig::default());
    let mut email_backends = Vec::new();
    if has_legacy_gmail {
        email_backends.push(crate::core::email_delivery::EMAIL_PROVIDER_GMAIL.to_string());
    }
    if has_workspace_gmail {
        email_backends
            .push(crate::core::email_delivery::EMAIL_PROVIDER_GOOGLE_WORKSPACE.to_string());
    }
    if crate::core::email_delivery::external_email_delivery_is_ready(&config.email) {
        if let Some(provider_id) =
            crate::core::email_delivery::external_email_provider_id(&config.email)
        {
            if !email_backends
                .iter()
                .any(|existing| existing == &provider_id)
            {
                email_backends.push(provider_id);
            }
        }
    }
    if crate::core::email_delivery::email_channel_is_ready(&config.email.provider, &email_backends)
    {
        channels.insert("email".to_string());
    }

    Ok(channels)
}

fn describe_output_target(source: &WebhookSource) -> String {
    match source.output_target {
        WebhookOutputTarget::None => "No completion push.".to_string(),
        WebhookOutputTarget::Preferred => "Completion pushes to the preferred channel.".to_string(),
        WebhookOutputTarget::Channel => format!(
            "Completion pushes to '{}'.",
            source
                .output_channel
                .as_deref()
                .unwrap_or("configured channel")
        ),
    }
}

fn source_report_target(source: &WebhookSource) -> Option<String> {
    match source.output_target {
        WebhookOutputTarget::None => None,
        WebhookOutputTarget::Preferred => Some("preferred".to_string()),
        WebhookOutputTarget::Channel => source.output_channel.clone(),
    }
}

fn default_auth_mode_for_provider(provider: &str) -> WebhookAuthMode {
    match provider {
        "github" => WebhookAuthMode::HmacSha256,
        "gitlab" => WebhookAuthMode::HeaderToken,
        _ => WebhookAuthMode::HeaderToken,
    }
}

fn default_event_header_for_provider(provider: &str) -> String {
    match provider {
        "github" => "X-GitHub-Event".to_string(),
        "gitlab" => "X-Gitlab-Event".to_string(),
        "sentry" => "Sentry-Hook-Resource".to_string(),
        _ => "X-Event-Type".to_string(),
    }
}

fn default_secret_header_for_provider(
    provider: &str,
    auth_mode: WebhookAuthMode,
) -> Option<String> {
    match auth_mode {
        WebhookAuthMode::None => None,
        WebhookAuthMode::BearerToken => Some("Authorization".to_string()),
        WebhookAuthMode::HmacSha256 => Some(match provider {
            "github" => "X-Hub-Signature-256".to_string(),
            _ => crate::branding::WEBHOOK_SIGNATURE_HEADER.to_string(),
        }),
        WebhookAuthMode::HeaderToken => Some(match provider {
            "gitlab" => "X-Gitlab-Token".to_string(),
            _ => crate::branding::WEBHOOK_SECRET_HEADER.to_string(),
        }),
    }
}

fn public_webhook_auth_required_message() -> &'static str {
    "Public webhook sources require a secret. Choose header token, bearer token, or HMAC."
}

fn public_webhook_ingress_requires_auth_for_posture(
    deployment_mode: DeploymentMode,
    tunnel_active: bool,
    tunnel_control_plane_enabled: bool,
) -> bool {
    deployment_mode == DeploymentMode::InternetFacing
        || (tunnel_active && tunnel_control_plane_enabled)
}

fn validate_public_webhook_auth_mode(
    auth_mode: WebhookAuthMode,
    public_ingress_requires_auth: bool,
) -> Result<()> {
    if matches!(auth_mode, WebhookAuthMode::None) && public_ingress_requires_auth {
        anyhow::bail!(public_webhook_auth_required_message());
    }
    Ok(())
}

async fn public_webhook_ingress_requires_auth(state: &AppState) -> bool {
    let (active, control_plane_enabled) = {
        let tunnel = state.tunnel.read().await;
        (tunnel.active, tunnel.control_plane_enabled)
    };
    public_webhook_ingress_requires_auth_for_posture(
        state.deployment_mode,
        active,
        control_plane_enabled,
    )
}

fn clip_chars(value: &str, max_chars: usize) -> String {
    let trimmed = value.trim();
    if trimmed.chars().count() <= max_chars {
        trimmed.to_string()
    } else {
        format!("{}...", trimmed.chars().take(max_chars).collect::<String>())
    }
}

fn sanitize_excerpt(value: &str, max_chars: usize) -> String {
    let redacted = crate::security::redact_secret_input(value).text;
    let pii_redacted = crate::security::redact_pii(&redacted);
    clip_chars(&pii_redacted, max_chars)
}

fn constant_time_eq(left: &[u8], right: &[u8]) -> bool {
    crate::security::constant_time_eq(left, right)
}

fn hmac_sha256_hex(secret: &str, data: &[u8]) -> String {
    const BLOCK_SIZE: usize = 64;
    let mut key = secret.as_bytes().to_vec();
    if key.len() > BLOCK_SIZE {
        let mut hasher = Sha256::new();
        hasher.update(&key);
        key = hasher.finalize().to_vec();
    }
    if key.len() < BLOCK_SIZE {
        key.resize(BLOCK_SIZE, 0);
    }

    let mut ipad = [0x36u8; BLOCK_SIZE];
    let mut opad = [0x5cu8; BLOCK_SIZE];
    for (idx, byte) in key.iter().enumerate() {
        ipad[idx] ^= byte;
        opad[idx] ^= byte;
    }

    let mut inner = Sha256::new();
    inner.update(ipad);
    inner.update(data);
    let inner_hash = inner.finalize();

    let mut outer = Sha256::new();
    outer.update(opad);
    outer.update(inner_hash);
    hex::encode(outer.finalize())
}

fn json_path_value<'a>(value: &'a serde_json::Value, path: &str) -> Option<&'a serde_json::Value> {
    let mut current = value;
    for segment in path.split('.') {
        match current {
            serde_json::Value::Object(map) => {
                current = map.get(segment)?;
            }
            _ => return None,
        }
    }
    Some(current)
}

fn json_value_as_text(value: &serde_json::Value) -> Option<String> {
    match value {
        serde_json::Value::String(s) => Some(s.trim().to_string()).filter(|s| !s.is_empty()),
        serde_json::Value::Number(n) => Some(n.to_string()),
        serde_json::Value::Bool(b) => Some(b.to_string()),
        _ => None,
    }
}

fn first_json_text(value: &serde_json::Value, paths: &[&str]) -> Option<String> {
    paths.iter().find_map(|path| {
        json_path_value(value, path)
            .and_then(json_value_as_text)
            .filter(|candidate| !candidate.trim().is_empty())
    })
}

fn header_text(headers: &HeaderMap, name: &str) -> Option<String> {
    headers
        .get(name)
        .and_then(|value| value.to_str().ok())
        .map(|value| value.trim().to_string())
        .filter(|value| !value.is_empty())
}

fn classify_failure(event_type: &str, status: Option<&str>, subject: &str, summary: &str) -> bool {
    let joined = format!(
        "{} {} {} {}",
        event_type,
        status.unwrap_or_default(),
        subject,
        summary
    )
    .to_ascii_lowercase();
    let failure_terms = [
        "fail",
        "failed",
        "failure",
        "error",
        "errored",
        "critical",
        "incident",
        "degraded",
        "timeout",
        "timed out",
        "cancelled",
        "canceled",
        "down",
    ];
    failure_terms.iter().any(|term| joined.contains(term))
}

fn classify_change(event_type: &str, status: Option<&str>, subject: &str, summary: &str) -> bool {
    let joined = format!(
        "{} {} {} {}",
        event_type,
        status.unwrap_or_default(),
        subject,
        summary
    )
    .to_ascii_lowercase();
    let change_terms = [
        "push",
        "merge",
        "deploy",
        "release",
        "opened",
        "closed",
        "created",
        "updated",
        "deleted",
        "removed",
        "added",
        "change",
        "changed",
        "edited",
        "synchronize",
        "installation",
    ];
    change_terms.iter().any(|term| joined.contains(term))
}

fn matches_rule(source: &WebhookSource, event: &NormalizedWebhookEvent) -> (bool, String) {
    match source.match_mode {
        WebhookMatchMode::All => (true, "Matched all incoming events.".to_string()),
        WebhookMatchMode::FailuresOnly => {
            if event.is_failure {
                (true, "Matched failure/error event.".to_string())
            } else {
                (
                    false,
                    "Ignored because this source only reacts to failures.".to_string(),
                )
            }
        }
        WebhookMatchMode::ChangesOnly => {
            if event.is_change {
                (
                    true,
                    "Matched create/update/delete style event.".to_string(),
                )
            } else {
                (
                    false,
                    "Ignored because this source only reacts to change events.".to_string(),
                )
            }
        }
    }
}

fn webhook_event_delivery_id(
    source: &WebhookSource,
    headers: &HeaderMap,
    payload: Option<&serde_json::Value>,
) -> Option<String> {
    let header_candidates = [
        match source.provider.as_str() {
            "github" => Some("X-GitHub-Delivery"),
            "gitlab" => Some("X-Gitlab-Event-UUID"),
            _ => None,
        },
        Some("X-Request-Id"),
        Some("X-Correlation-Id"),
    ];
    for header_name in header_candidates.into_iter().flatten() {
        if let Some(value) = header_text(headers, header_name) {
            return Some(value);
        }
    }
    payload.and_then(|json| {
        first_json_text(
            json,
            &[
                "delivery_id",
                "event_id",
                "id",
                "uuid",
                "alert.id",
                "workflow_run.id",
                "check_suite.id",
                "check_run.id",
                "pipeline.id",
            ],
        )
    })
}

fn normalize_event(
    source: &WebhookSource,
    headers: &HeaderMap,
    raw_body: &str,
    payload: Option<&serde_json::Value>,
) -> NormalizedWebhookEvent {
    let event_header = source
        .event_header
        .clone()
        .unwrap_or_else(|| default_event_header_for_provider(&source.provider));
    let event_type = header_text(headers, &event_header)
        .or_else(|| {
            payload.and_then(|json| {
                first_json_text(
                    json,
                    &[
                        "event",
                        "event_name",
                        "type",
                        "kind",
                        "action",
                        "object_kind",
                        "resource",
                    ],
                )
            })
        })
        .unwrap_or_else(|| "webhook".to_string());
    let status = payload.and_then(|json| {
        first_json_text(
            json,
            &[
                "conclusion",
                "status",
                "state",
                "result",
                "severity",
                "level",
                "alert.status",
                "pipeline.status",
                "workflow_run.conclusion",
            ],
        )
    });
    let subject = payload
        .and_then(|json| {
            first_json_text(
                json,
                &[
                    "repository.full_name",
                    "repository.name",
                    "project.path_with_namespace",
                    "project.name",
                    "workflow_run.name",
                    "check_suite.head_branch",
                    "check_run.name",
                    "pipeline.name",
                    "alert.title",
                    "incident.title",
                    "title",
                    "subject",
                    "name",
                ],
            )
        })
        .unwrap_or_else(|| source.name.clone());
    let event_url = payload.and_then(|json| {
        first_json_text(
            json,
            &[
                "html_url",
                "web_url",
                "target_url",
                "url",
                "alert_url",
                "workflow_run.html_url",
                "check_run.html_url",
                "pipeline.web_url",
            ],
        )
    });
    let event_id = webhook_event_delivery_id(source, headers, payload);

    let excerpt_raw = if let Some(json) = payload {
        serde_json::to_string_pretty(json).unwrap_or_else(|_| raw_body.to_string())
    } else {
        raw_body.to_string()
    };
    let payload_excerpt = sanitize_excerpt(&excerpt_raw, WEBHOOK_EXCERPT_MAX_CHARS);
    // Summary for display — includes source name for human context.
    let summary = sanitize_excerpt(
        &format!(
            "{} {}{} for {}",
            source.name,
            event_type,
            status
                .as_deref()
                .map(|value| format!(" ({})", value))
                .unwrap_or_default(),
            subject
        ),
        WEBHOOK_SUMMARY_MAX_CHARS,
    );
    // Classification summary — excludes source name to avoid false positives
    // (e.g. source named "Deploy Failures" shouldn't make every event a failure).
    let classify_summary = sanitize_excerpt(
        &format!(
            "{}{} for {}",
            event_type,
            status
                .as_deref()
                .map(|value| format!(" ({})", value))
                .unwrap_or_default(),
            subject
        ),
        WEBHOOK_SUMMARY_MAX_CHARS,
    );

    let mut hasher = Sha256::new();
    hasher.update(source.id.as_bytes());
    if let Some(event_id) = &event_id {
        hasher.update(event_id.as_bytes());
    } else {
        hasher.update(raw_body.as_bytes());
    }
    let dedupe_key = hex::encode(hasher.finalize());

    let severity = status
        .as_ref()
        .map(|value| value.trim().to_ascii_lowercase())
        .filter(|value| !value.is_empty());
    let is_failure = classify_failure(&event_type, status.as_deref(), &subject, &classify_summary);
    let is_change = classify_change(&event_type, status.as_deref(), &subject, &classify_summary);

    NormalizedWebhookEvent {
        event_type,
        status,
        subject,
        event_id,
        event_url,
        summary,
        payload_excerpt,
        dedupe_key,
        severity,
        is_failure,
        is_change,
    }
}

fn build_webhook_prompt(source: &WebhookSource, event: &NormalizedWebhookEvent) -> String {
    let mut prompt = String::from(
        "This run was triggered automatically by a webhook. No user invoked chat.\n\
Handle it as an autonomous operator task: take the next safe step when it is clear, \
and only ask for approval or missing credentials if required.\n\n",
    );
    prompt.push_str(&format!("Source: {} ({})\n", source.name, source.provider));
    prompt.push_str(&format!("Trigger: {}\n", event.event_type));
    if let Some(status) = &event.status {
        prompt.push_str(&format!("Status: {}\n", status));
    }
    prompt.push_str(&format!("Subject: {}\n", event.subject));
    if let Some(url) = &event.event_url {
        prompt.push_str(&format!("Reference URL: {}\n", url));
    }
    prompt.push_str("\nOperator instruction:\n");
    prompt.push_str(source.instruction.trim());
    prompt.push_str("\n\nNormalized event summary:\n");
    prompt.push_str(event.summary.trim());
    prompt.push_str("\n\nRedacted payload excerpt:\n");
    // Webhook bodies are attacker-controllable; wrap them in the untrusted
    // envelope so the model treats their contents as data, not instructions.
    let clipped_excerpt = clip_chars(
        event.payload_excerpt.trim(),
        WEBHOOK_PROMPT_EXCERPT_MAX_CHARS,
    );
    prompt.push_str(&crate::security::sanitize_untrusted_output(
        "webhook_payload",
        &clipped_excerpt,
    ));
    prompt
}

fn source_secret_present(config_dir: &FsPath, data_dir: &FsPath, source_id: &str) -> Result<bool> {
    let manager =
        crate::core::config::SecureConfigManager::new_with_data_dir(config_dir, Some(data_dir))?;
    Ok(manager
        .get_custom_secret(&webhook_secret_key(source_id))?
        .is_some_and(|value| !value.trim().is_empty()))
}

pub(crate) async fn list_webhook_source_inventory(
    storage: &crate::storage::Storage,
    config_dir: &FsPath,
    data_dir: &FsPath,
    only_connected: bool,
) -> Result<serde_json::Value> {
    let mut sources = load_sources(storage).await?;
    sources.sort_by(|left, right| right.updated_at.cmp(&left.updated_at));
    let total = sources.len();
    let mut connected_total = 0usize;
    let mut rows = Vec::new();

    for source in sources {
        let secret_configured =
            source_secret_present(config_dir, data_dir, &source.id).unwrap_or(false);
        let connected = source.enabled
            && (matches!(source.auth_mode, WebhookAuthMode::None) || secret_configured);
        if connected {
            connected_total += 1;
        }
        if only_connected && !connected {
            continue;
        }
        let mut value = serde_json::to_value(present_source(&source, secret_configured))?;
        if let Some(object) = value.as_object_mut() {
            object.insert("connected".to_string(), serde_json::json!(connected));
        }
        rows.push(value);
    }

    Ok(serde_json::json!({
        "available": true,
        "surface": "webhook_sources",
        "total": total,
        "connected_total": connected_total,
        "filtered_to_connected": only_connected,
        "sources": rows,
    }))
}

fn verify_source_secret(
    config_dir: &FsPath,
    data_dir: &FsPath,
    source: &WebhookSource,
    headers: &HeaderMap,
    raw_body: &str,
) -> Result<()> {
    if matches!(source.auth_mode, WebhookAuthMode::None) {
        return Ok(());
    }
    let manager =
        crate::core::config::SecureConfigManager::new_with_data_dir(config_dir, Some(data_dir))?;
    let secret = manager
        .get_custom_secret(&webhook_secret_key(&source.id))?
        .filter(|value| !value.trim().is_empty())
        .ok_or_else(|| anyhow!("Webhook secret is not configured"))?;
    match source.auth_mode {
        WebhookAuthMode::None => Ok(()),
        WebhookAuthMode::HeaderToken => {
            let header_name = source
                .secret_header
                .clone()
                .or_else(|| default_secret_header_for_provider(&source.provider, source.auth_mode))
                .ok_or_else(|| anyhow!("Secret header is not configured"))?;
            let provided = header_text(headers, &header_name)
                .ok_or_else(|| anyhow!("Missing {} header", header_name))?;
            if constant_time_eq(provided.trim().as_bytes(), secret.trim().as_bytes()) {
                Ok(())
            } else {
                Err(anyhow!("Invalid webhook secret"))
            }
        }
        WebhookAuthMode::BearerToken => {
            let auth = header_text(headers, "Authorization")
                .ok_or_else(|| anyhow!("Missing Authorization header"))?;
            let token = auth
                .strip_prefix("Bearer ")
                .or_else(|| auth.strip_prefix("bearer "))
                .map(str::trim)
                .ok_or_else(|| anyhow!("Authorization header must use Bearer token"))?;
            if constant_time_eq(token.as_bytes(), secret.trim().as_bytes()) {
                Ok(())
            } else {
                Err(anyhow!("Invalid bearer token"))
            }
        }
        WebhookAuthMode::HmacSha256 => {
            let header_name = source
                .secret_header
                .clone()
                .or_else(|| default_secret_header_for_provider(&source.provider, source.auth_mode))
                .ok_or_else(|| anyhow!("Signature header is not configured"))?;
            let provided = header_text(headers, &header_name)
                .ok_or_else(|| anyhow!("Missing {} header", header_name))?;
            let expected = hmac_sha256_hex(&secret, raw_body.as_bytes());
            let provided_trimmed = provided.trim();
            let provided_without_prefix = provided_trimmed
                .strip_prefix("sha256=")
                .unwrap_or(provided_trimmed);
            if constant_time_eq(expected.as_bytes(), provided_without_prefix.as_bytes()) {
                Ok(())
            } else {
                Err(anyhow!("Invalid HMAC signature"))
            }
        }
    }
}

fn source_status_message(source: &WebhookSource, outcome: &str) -> String {
    match outcome {
        "queued" => "Matched event and queued autonomous work.".to_string(),
        "duplicate" => "Ignored duplicate delivery inside dedupe window.".to_string(),
        "ignored" => match source.match_mode {
            WebhookMatchMode::All => "Ignored event.".to_string(),
            WebhookMatchMode::FailuresOnly => {
                "Ignored event because this source only reacts to failures.".to_string()
            }
            WebhookMatchMode::ChangesOnly => {
                "Ignored event because this source only reacts to changes.".to_string()
            }
        },
        "auth_failed" => {
            "Rejected webhook delivery during signature/token verification.".to_string()
        }
        "error" => "Webhook delivery matched, but task dispatch failed.".to_string(),
        _ => "Webhook delivery processed.".to_string(),
    }
}

fn queued_notification_level(event: &NormalizedWebhookEvent) -> &'static str {
    if event.is_failure {
        "warning"
    } else {
        "info"
    }
}

fn webhook_queue_notification_body(
    source: &WebhookSource,
    event: &NormalizedWebhookEvent,
    task_id: &str,
    reused_existing: bool,
    removed_duplicates: usize,
    dispatch_status: &str,
) -> String {
    let mut lines = vec![
        format!("Source: {}", source.name),
        format!("Provider: {}", source.provider),
        format!("Event: {}", event.event_type),
        format!("Subject: {}", event.subject),
    ];
    if let Some(status) = &event.status {
        lines.push(format!("Status: {}", status));
    }
    lines.push(format!("Dispatch: {}", dispatch_status));
    lines.push(format!("Task ID: {}", task_id));
    lines.push(describe_output_target(source));
    if reused_existing {
        lines.push("Reused an existing matching task.".to_string());
    }
    if removed_duplicates > 0 {
        lines.push(format!(
            "Removed {} duplicate queued task(s).",
            removed_duplicates
        ));
    }
    if let Some(url) = &event.event_url {
        lines.push(format!("Reference: {}", url));
    }
    lines.join("\n")
}

fn webhook_dispatch_failure_body(
    source: &WebhookSource,
    event: &NormalizedWebhookEvent,
    error: &str,
) -> String {
    let mut lines = vec![
        format!("Source: {}", source.name),
        format!("Provider: {}", source.provider),
        format!("Event: {}", event.event_type),
        format!("Subject: {}", event.subject),
    ];
    if let Some(status) = &event.status {
        lines.push(format!("Status: {}", status));
    }
    if let Some(url) = &event.event_url {
        lines.push(format!("Reference: {}", url));
    }
    lines.push(String::new());
    lines.push("Dispatch failed:".to_string());
    lines.push(clip_chars(error, WEBHOOK_SUMMARY_MAX_CHARS));
    lines.join("\n")
}

async fn emit_plugin_webhook_event_best_effort(
    state: &AppState,
    dispatch: PluginWebhookDispatch<'_>,
) {
    let payload = serde_json::json!({
        "event": "webhook.received",
        "source": {
            "id": dispatch.source.id.clone(),
            "name": dispatch.source.name.clone(),
            "provider": dispatch.source.provider.clone(),
            "conversation_id": dispatch.source.conversation_id.clone(),
            "match_mode": dispatch.source.match_mode,
            "require_approval": dispatch.source.require_approval,
        },
        "delivery": {
            "outcome": dispatch.outcome,
            "matched": dispatch.matched,
            "queued": dispatch.queued,
            "task_id": dispatch.task_id,
            "message": dispatch.message,
            "test_event": dispatch.test_event,
            "received_at": now_rfc3339(),
        },
        "webhook": {
            "event_type": dispatch.event.event_type.clone(),
            "status": dispatch.event.status.clone(),
            "subject": dispatch.event.subject.clone(),
            "event_id": dispatch.event.event_id.clone(),
            "event_url": dispatch.event.event_url.clone(),
            "severity": dispatch.event.severity.clone(),
            "is_failure": dispatch.event.is_failure,
            "is_change": dispatch.event.is_change,
            "dedupe_key": dispatch.event.dedupe_key.clone(),
            "summary": clip_chars(&dispatch.event.summary, WEBHOOK_SUMMARY_MAX_CHARS),
            "payload_excerpt": clip_chars(&dispatch.event.payload_excerpt, WEBHOOK_PROMPT_EXCERPT_MAX_CHARS),
        }
    });
    let agent = state.agent.read().await;
    if let Err(error) = agent
        .dispatch_plugin_event("webhook.received", payload)
        .await
    {
        tracing::warn!(
            "Failed to dispatch plugin event webhook.received for source '{}': {}",
            dispatch.source.id,
            error
        );
    }
}

async fn append_event(storage: &crate::storage::Storage, record: WebhookEventRecord) -> Result<()> {
    let mut events = load_events(storage).await?;
    events.insert(0, record);
    if events.len() > WEBHOOK_EVENT_HISTORY_LIMIT {
        events.truncate(WEBHOOK_EVENT_HISTORY_LIMIT);
    }
    save_events(storage, &events).await
}

fn duplicate_seen_recently(
    events: &[WebhookEventRecord],
    source: &WebhookSource,
    event: &NormalizedWebhookEvent,
) -> bool {
    let now = chrono::Utc::now();
    events.iter().any(|entry| {
        if entry.source_id != source.id || entry.dedupe_key != event.dedupe_key {
            return false;
        }
        chrono::DateTime::parse_from_rfc3339(&entry.received_at)
            .ok()
            .map(|ts| {
                now.signed_duration_since(ts.with_timezone(&chrono::Utc))
                    <= chrono::Duration::seconds(source.dedupe_window_secs as i64)
            })
            .unwrap_or(false)
    })
}

async fn persist_source_runtime_state(
    storage: &crate::storage::Storage,
    sources: &mut [WebhookSource],
    index: usize,
    received_at: &str,
    outcome: &str,
    task_id: Option<String>,
) -> Result<()> {
    if let Some(source) = sources.get_mut(index) {
        source.last_received_at = Some(received_at.to_string());
        source.last_outcome = Some(outcome.to_string());
        source.last_task_id = task_id;
        source.updated_at = received_at.to_string();
    }
    save_sources(storage, sources).await
}

async fn route_event(
    state: &AppState,
    storage: &crate::storage::Storage,
    sources: &mut [WebhookSource],
    input: RouteEventInput<'_>,
) -> Result<serde_json::Value> {
    let received_at = now_rfc3339();
    let source = input.source;
    let source_index = input.source_index;
    let event = normalize_event(source, input.headers, input.raw_body, input.payload);
    let events = load_events(storage).await?;
    if duplicate_seen_recently(&events, source, &event) {
        let record = WebhookEventRecord {
            id: uuid::Uuid::new_v4().to_string(),
            source_id: source.id.clone(),
            source_name: source.name.clone(),
            provider: source.provider.clone(),
            received_at: received_at.clone(),
            event_type: event.event_type.clone(),
            status: event.status.clone(),
            subject: event.subject.clone(),
            outcome: "duplicate".to_string(),
            matched: false,
            queued: false,
            message: Some(source_status_message(source, "duplicate")),
            event_id: event.event_id.clone(),
            dedupe_key: event.dedupe_key.clone(),
            event_url: event.event_url.clone(),
            payload_excerpt: Some(event.payload_excerpt.clone()),
            task_id: None,
            conversation_id: Some(source.conversation_id.clone()),
            severity: event.severity.clone(),
            test_event: input.test_event,
        };
        append_event(storage, record).await?;
        persist_source_runtime_state(
            storage,
            sources,
            source_index,
            &received_at,
            "duplicate",
            None,
        )
        .await?;
        emit_plugin_webhook_event_best_effort(
            state,
            PluginWebhookDispatch {
                source,
                event: &event,
                outcome: "duplicate",
                matched: false,
                queued: false,
                task_id: None,
                message: Some(&source_status_message(source, "duplicate")),
                test_event: input.test_event,
            },
        )
        .await;
        return Ok(serde_json::json!({
            "status": "ok",
            "queued": false,
            "duplicate": true,
            "matched": false,
            "message": source_status_message(source, "duplicate"),
            "conversation_id": source.conversation_id.clone(),
        }));
    }

    let (matched, match_message) = matches_rule(source, &event);
    if !matched {
        let record = WebhookEventRecord {
            id: uuid::Uuid::new_v4().to_string(),
            source_id: source.id.clone(),
            source_name: source.name.clone(),
            provider: source.provider.clone(),
            received_at: received_at.clone(),
            event_type: event.event_type.clone(),
            status: event.status.clone(),
            subject: event.subject.clone(),
            outcome: "ignored".to_string(),
            matched: false,
            queued: false,
            message: Some(match_message.clone()),
            event_id: event.event_id.clone(),
            dedupe_key: event.dedupe_key.clone(),
            event_url: event.event_url.clone(),
            payload_excerpt: Some(event.payload_excerpt.clone()),
            task_id: None,
            conversation_id: Some(source.conversation_id.clone()),
            severity: event.severity.clone(),
            test_event: input.test_event,
        };
        append_event(storage, record).await?;
        persist_source_runtime_state(
            storage,
            sources,
            source_index,
            &received_at,
            "ignored",
            None,
        )
        .await?;
        emit_plugin_webhook_event_best_effort(
            state,
            PluginWebhookDispatch {
                source,
                event: &event,
                outcome: "ignored",
                matched: false,
                queued: false,
                task_id: None,
                message: Some(&match_message),
                test_event: input.test_event,
            },
        )
        .await;
        return Ok(serde_json::json!({
            "status": "ok",
            "queued": false,
            "duplicate": false,
            "matched": false,
            "message": match_message,
            "conversation_id": source.conversation_id.clone(),
        }));
    }

    let prompt = build_webhook_prompt(source, &event);
    let mut autonomy_payload = serde_json::Map::new();
    autonomy_payload.insert("prompt".to_string(), serde_json::Value::String(prompt));
    autonomy_payload.insert(
        "channel".to_string(),
        serde_json::Value::String("system".to_string()),
    );
    autonomy_payload.insert(
        "conversation_id".to_string(),
        serde_json::Value::String(source.conversation_id.clone()),
    );
    let mut task_arguments = serde_json::json!({
        "autonomy_action_kind": "chat_prompt",
        "autonomy_action_payload": serde_json::Value::Object(autonomy_payload),
        "source_id": source.id.clone(),
        "source_name": source.name.clone(),
        "event_type": event.event_type.clone(),
        "event_status": event.status.clone(),
        "subject": event.subject.clone(),
        "event_url": event.event_url.clone(),
        "delivery_key": event.dedupe_key.clone(),
        "_automation": {
            "kind": "webhook",
            "provider": source.provider.clone(),
            "webhook": {
                "source_id": source.id.clone(),
                "source_name": source.name.clone(),
                "event_type": event.event_type.clone(),
                "event_status": event.status.clone(),
                "subject": event.subject.clone(),
                "event_url": event.event_url.clone(),
                "notify_on_queued": source.notify_on_queued,
                "notify_on_success": source.notify_on_success,
                "notify_on_failure": source.notify_on_failure,
                "output_target": source.output_target,
                "output_channel": source.output_channel.clone(),
                "test_event": input.test_event,
            }
        }
    });
    if let Some(report_to) = source_report_target(source) {
        if let Some(arguments) = task_arguments.as_object_mut() {
            arguments.insert(
                "report_to".to_string(),
                serde_json::Value::String(report_to),
            );
        }
    }

    let mut task = crate::core::Task::new(
        format!(
            "Webhook: {} - {}",
            source.name,
            clip_chars(&event.summary, 120)
        ),
        "autonomy_action".to_string(),
        task_arguments,
    );
    task.capabilities = vec!["autonomy_action".to_string()];
    task.approval = if source.require_approval {
        crate::core::TaskApproval::RequireApproval
    } else {
        crate::core::TaskApproval::Auto
    };
    task.status = crate::core::status_for_task_approval(&task.approval);

    let queued = {
        let agent = state.agent.read().await;
        agent
            .add_or_update_similar_task(task.clone(), source.allow_duplicate, None)
            .await
    };
    match queued {
        Ok((task_id, reused_existing, removed_duplicates)) => {
            spawn_autonomy_analysis_tick(state.agent.clone(), "webhook_event");
            let task_id_str = task_id.to_string();
            let record = WebhookEventRecord {
                id: uuid::Uuid::new_v4().to_string(),
                source_id: source.id.clone(),
                source_name: source.name.clone(),
                provider: source.provider.clone(),
                received_at: received_at.clone(),
                event_type: event.event_type.clone(),
                status: event.status.clone(),
                subject: event.subject.clone(),
                outcome: "queued".to_string(),
                matched: true,
                queued: true,
                message: Some(source_status_message(source, "queued")),
                event_id: event.event_id.clone(),
                dedupe_key: event.dedupe_key.clone(),
                event_url: event.event_url.clone(),
                payload_excerpt: Some(event.payload_excerpt.clone()),
                task_id: Some(task_id_str.clone()),
                conversation_id: Some(source.conversation_id.clone()),
                severity: event.severity.clone(),
                test_event: input.test_event,
            };
            append_event(storage, record).await?;
            persist_source_runtime_state(
                storage,
                sources,
                source_index,
                &received_at,
                "queued",
                Some(task_id_str.clone()),
            )
            .await?;
            if source.notify_on_queued {
                let notify_body = webhook_queue_notification_body(
                    source,
                    &event,
                    &task_id_str,
                    reused_existing,
                    removed_duplicates,
                    "queued",
                );
                let agent = state.agent.read().await;
                agent
                    .emit_notification_forced(
                        &format!("Webhook queued: {}", source.name),
                        &notify_body,
                        queued_notification_level(&event),
                        "webhook",
                    )
                    .await;
            }
            emit_plugin_webhook_event_best_effort(
                state,
                PluginWebhookDispatch {
                    source,
                    event: &event,
                    outcome: "queued",
                    matched: true,
                    queued: true,
                    task_id: Some(&task_id_str),
                    message: Some(&source_status_message(source, "queued")),
                    test_event: input.test_event,
                },
            )
            .await;
            Ok(serde_json::json!({
                "status": "ok",
                "queued": true,
                "duplicate": false,
                "matched": true,
                "message": source_status_message(source, "queued"),
                "task_id": task_id_str,
                "reused_existing": reused_existing,
                "removed_duplicates": removed_duplicates,
                "conversation_id": source.conversation_id.clone(),
            }))
        }
        Err(error) => {
            let record = WebhookEventRecord {
                id: uuid::Uuid::new_v4().to_string(),
                source_id: source.id.clone(),
                source_name: source.name.clone(),
                provider: source.provider.clone(),
                received_at: received_at.clone(),
                event_type: event.event_type.clone(),
                status: event.status.clone(),
                subject: event.subject.clone(),
                outcome: "error".to_string(),
                matched: true,
                queued: false,
                message: Some(error.to_string()),
                event_id: event.event_id.clone(),
                dedupe_key: event.dedupe_key.clone(),
                event_url: event.event_url.clone(),
                payload_excerpt: Some(event.payload_excerpt.clone()),
                task_id: None,
                conversation_id: Some(source.conversation_id.clone()),
                severity: event.severity.clone(),
                test_event: input.test_event,
            };
            append_event(storage, record).await?;
            persist_source_runtime_state(
                storage,
                sources,
                source_index,
                &received_at,
                "error",
                None,
            )
            .await?;
            if source.notify_on_failure {
                let notify_body = webhook_dispatch_failure_body(source, &event, &error.to_string());
                let agent = state.agent.read().await;
                agent
                    .emit_notification_forced(
                        &format!("Webhook failed: {}", source.name),
                        &notify_body,
                        "error",
                        "webhook",
                    )
                    .await;
            }
            emit_plugin_webhook_event_best_effort(
                state,
                PluginWebhookDispatch {
                    source,
                    event: &event,
                    outcome: "error",
                    matched: true,
                    queued: false,
                    task_id: None,
                    message: Some(&error.to_string()),
                    test_event: input.test_event,
                },
            )
            .await;
            Err(error)
        }
    }
}

pub(super) async fn list_webhook_sources(State(state): State<AppState>) -> Response {
    let (storage, config_dir, data_dir) = {
        let agent = state.agent.read().await;
        (
            agent.storage.clone(),
            agent.config_dir.clone(),
            agent.data_dir.clone(),
        )
    };
    match load_sources(&storage).await {
        Ok(mut sources) => {
            sources.sort_by(|left, right| right.updated_at.cmp(&left.updated_at));
            let rows = sources
                .into_iter()
                .map(|source| {
                    let configured =
                        source_secret_present(&config_dir, &data_dir, &source.id).unwrap_or(false);
                    serde_json::to_value(present_source(&source, configured)).unwrap_or_default()
                })
                .collect::<Vec<_>>();
            Json(serde_json::json!({
                "sources": rows,
                "count": rows.len(),
            }))
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

async fn upsert_source_internal(
    state: &AppState,
    source_id: Option<&str>,
    request: WebhookSourceUpsertRequest,
) -> Result<WebhookSourceResponse> {
    let name = request.name.trim();
    if name.is_empty() {
        anyhow::bail!("Name is required");
    }
    let provider = sanitize_provider(request.provider.as_deref());
    let auth_mode = request
        .auth_mode
        .unwrap_or_else(|| default_auth_mode_for_provider(&provider));
    validate_public_webhook_auth_mode(
        auth_mode,
        public_webhook_ingress_requires_auth(state).await,
    )?;
    let match_mode = request.match_mode.unwrap_or_default();
    let event_header = sanitize_header_name(request.event_header.as_deref())
        .or_else(|| Some(default_event_header_for_provider(&provider)));
    let secret_header = sanitize_header_name(request.secret_header.as_deref())
        .or_else(|| default_secret_header_for_provider(&provider, auth_mode));
    let instruction = request
        .instruction
        .as_deref()
        .map(str::trim)
        .filter(|value| !value.is_empty())
        .unwrap_or(
            "Analyze this event and take the next safe action. If it is only informational, summarize it briefly and stop.",
        )
        .to_string();
    let now = now_rfc3339();
    let candidate_id = source_id
        .map(|value| value.to_string())
        .or_else(|| request.id.clone())
        .unwrap_or_else(|| sanitize_source_id(name));
    let id = if candidate_id.trim().is_empty() {
        uuid::Uuid::new_v4().to_string()
    } else {
        candidate_id
    };

    let (storage, config_dir, data_dir) = {
        let agent = state.agent.read().await;
        (
            agent.storage.clone(),
            agent.config_dir.clone(),
            agent.data_dir.clone(),
        )
    };
    let mut sources = load_sources(&storage).await?;
    let existing_index = sources.iter().position(|source| source.id == id);
    if source_id.is_some() && existing_index.is_none() {
        anyhow::bail!("Webhook source not found");
    }
    if source_id.is_none() && existing_index.is_some() {
        anyhow::bail!("A webhook source with that id already exists");
    }
    let existing_source = existing_index.and_then(|index| sources.get(index).cloned());

    let (conversation_id, created_at, last_received_at, last_outcome, last_task_id) =
        if let Some(index) = existing_index {
            let existing = &sources[index];
            (
                existing.conversation_id.clone(),
                existing.created_at.clone(),
                existing.last_received_at.clone(),
                existing.last_outcome.clone(),
                existing.last_task_id.clone(),
            )
        } else {
            (
                uuid::Uuid::new_v4().to_string(),
                now.clone(),
                None,
                None,
                None,
            )
        };
    let notify_on_queued = request.notify_on_queued.unwrap_or_else(|| {
        existing_source
            .as_ref()
            .map(|source| source.notify_on_queued)
            .unwrap_or_else(default_notify_on_queued)
    });
    let notify_on_success = request.notify_on_success.unwrap_or_else(|| {
        existing_source
            .as_ref()
            .map(|source| source.notify_on_success)
            .unwrap_or_else(default_notify_on_success)
    });
    let notify_on_failure = request.notify_on_failure.unwrap_or_else(|| {
        existing_source
            .as_ref()
            .map(|source| source.notify_on_failure)
            .unwrap_or_else(default_notify_on_failure)
    });
    let requested_output_target = request.output_target;
    let output_target = requested_output_target
        .or(existing_source.as_ref().map(|source| source.output_target))
        .unwrap_or_default();
    let requested_output_channel = sanitize_output_channel(request.output_channel.as_deref());
    let output_channel = match output_target {
        WebhookOutputTarget::Channel => requested_output_channel.or_else(|| {
            if requested_output_target.is_none() && request.output_channel.is_none() {
                existing_source
                    .as_ref()
                    .and_then(|source| source.output_channel.clone())
            } else {
                None
            }
        }),
        WebhookOutputTarget::None | WebhookOutputTarget::Preferred => None,
    };
    if matches!(output_target, WebhookOutputTarget::Channel) && output_channel.is_none() {
        anyhow::bail!(
            "Select a completion channel or switch completion delivery to none/preferred."
        );
    }
    if let Some(channel) = output_channel.as_deref() {
        let available_channels = available_completion_channels(state).await?;
        if !available_channels.contains(channel) {
            anyhow::bail!(
                "Completion channel '{}' is not currently available. Connect that channel first or switch completion delivery.",
                channel
            );
        }
    }

    let source = WebhookSource {
        id: id.clone(),
        name: name.to_string(),
        provider,
        description: request
            .description
            .as_deref()
            .map(str::trim)
            .filter(|value| !value.is_empty())
            .map(|value| value.to_string()),
        enabled: request.enabled.unwrap_or(true),
        auth_mode,
        match_mode,
        instruction,
        event_header,
        secret_header,
        allow_duplicate: request.allow_duplicate.unwrap_or(false),
        require_approval: request.require_approval.unwrap_or(false),
        dedupe_window_secs: request
            .dedupe_window_secs
            .unwrap_or(WEBHOOK_DEFAULT_DEDUPE_WINDOW_SECS)
            .clamp(60, 24 * 60 * 60),
        notify_on_queued,
        notify_on_success,
        notify_on_failure,
        output_target,
        output_channel,
        conversation_id,
        created_at,
        updated_at: now,
        last_received_at,
        last_outcome,
        last_task_id,
    };

    let manager =
        crate::core::config::SecureConfigManager::new_with_data_dir(&config_dir, Some(&data_dir))?;
    if request.clear_secret.unwrap_or(false) {
        manager.set_custom_secret(&webhook_secret_key(&id), None)?;
    }
    if let Some(secret) = request
        .secret
        .as_deref()
        .map(str::trim)
        .filter(|value| !value.is_empty())
    {
        manager.set_custom_secret(&webhook_secret_key(&id), Some(secret.to_string()))?;
    }
    let secret_configured = manager
        .get_custom_secret(&webhook_secret_key(&id))?
        .is_some_and(|value| !value.trim().is_empty());
    if !matches!(source.auth_mode, WebhookAuthMode::None) && !secret_configured {
        anyhow::bail!("This auth mode requires a secret. Save one or switch auth mode to none.");
    }

    if let Some(index) = existing_index {
        sources[index] = source.clone();
    } else {
        sources.push(source.clone());
    }
    save_sources(&storage, &sources).await?;

    Ok(present_source(&source, secret_configured))
}

pub(super) async fn create_webhook_source(
    State(state): State<AppState>,
    Json(request): Json<WebhookSourceUpsertRequest>,
) -> Response {
    match upsert_source_internal(&state, None, request).await {
        Ok(source) => Json(serde_json::json!({ "status": "ok", "source": source })).into_response(),
        Err(error) => (
            StatusCode::BAD_REQUEST,
            Json(ErrorResponse {
                error: error.to_string(),
            }),
        )
            .into_response(),
    }
}

pub(super) async fn update_webhook_source(
    State(state): State<AppState>,
    Path(id): Path<String>,
    Json(request): Json<WebhookSourceUpsertRequest>,
) -> Response {
    match upsert_source_internal(&state, Some(&id), request).await {
        Ok(source) => Json(serde_json::json!({ "status": "ok", "source": source })).into_response(),
        Err(error) if error.to_string().contains("not found") => (
            StatusCode::NOT_FOUND,
            Json(ErrorResponse {
                error: error.to_string(),
            }),
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

pub(super) async fn delete_webhook_source(
    State(state): State<AppState>,
    Path(id): Path<String>,
) -> Response {
    let (storage, config_dir, data_dir) = {
        let agent = state.agent.read().await;
        (
            agent.storage.clone(),
            agent.config_dir.clone(),
            agent.data_dir.clone(),
        )
    };
    let mut sources = match load_sources(&storage).await {
        Ok(sources) => sources,
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
    let before = sources.len();
    sources.retain(|source| source.id != id);
    if before == sources.len() {
        return (
            StatusCode::NOT_FOUND,
            Json(ErrorResponse {
                error: "Webhook source not found".to_string(),
            }),
        )
            .into_response();
    }
    if let Err(error) = save_sources(&storage, &sources).await {
        return (
            StatusCode::INTERNAL_SERVER_ERROR,
            Json(ErrorResponse {
                error: error.to_string(),
            }),
        )
            .into_response();
    }
    if let Ok(manager) =
        crate::core::config::SecureConfigManager::new_with_data_dir(&config_dir, Some(&data_dir))
    {
        let _ = manager.set_custom_secret(&webhook_secret_key(&id), None);
    }
    Json(serde_json::json!({ "status": "ok" })).into_response()
}

pub(super) async fn list_webhook_events(
    State(state): State<AppState>,
    Query(query): Query<WebhookEventsQuery>,
) -> Response {
    let storage = {
        let agent = state.agent.read().await;
        agent.storage.clone()
    };
    match load_events(&storage).await {
        Ok(mut events) => {
            if let Some(source_id) = query
                .source_id
                .as_deref()
                .map(str::trim)
                .filter(|value| !value.is_empty())
            {
                events.retain(|event| event.source_id == source_id);
            }
            let limit = query.limit.unwrap_or(40).clamp(1, 200);
            if events.len() > limit {
                events.truncate(limit);
            }
            Json(serde_json::json!({
                "events": events,
                "count": events.len(),
            }))
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

pub(super) async fn handle_inbound_webhook(
    State(state): State<AppState>,
    Path(source_id): Path<String>,
    headers: HeaderMap,
    raw_body: String,
) -> Response {
    let (storage, config_dir, data_dir) = {
        let agent = state.agent.read().await;
        (
            agent.storage.clone(),
            agent.config_dir.clone(),
            agent.data_dir.clone(),
        )
    };
    let mut sources = match load_sources(&storage).await {
        Ok(sources) => sources,
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
    let Some(source_index) = sources.iter().position(|source| source.id == source_id) else {
        return StatusCode::NOT_FOUND.into_response();
    };
    let source = sources[source_index].clone();
    if !source.enabled {
        return StatusCode::NOT_FOUND.into_response();
    }
    if let Err(error) = validate_public_webhook_auth_mode(
        source.auth_mode,
        public_webhook_ingress_requires_auth(&state).await,
    ) {
        return (
            StatusCode::UNAUTHORIZED,
            Json(ErrorResponse {
                error: error.to_string(),
            }),
        )
            .into_response();
    }

    let payload = serde_json::from_str::<serde_json::Value>(&raw_body).ok();
    let normalized = normalize_event(&source, &headers, &raw_body, payload.as_ref());
    if let Err(error) = verify_source_secret(&config_dir, &data_dir, &source, &headers, &raw_body) {
        let received_at = now_rfc3339();
        let record = WebhookEventRecord {
            id: uuid::Uuid::new_v4().to_string(),
            source_id: source.id.clone(),
            source_name: source.name.clone(),
            provider: source.provider.clone(),
            received_at: received_at.clone(),
            event_type: normalized.event_type,
            status: normalized.status,
            subject: normalized.subject,
            outcome: "auth_failed".to_string(),
            matched: false,
            queued: false,
            message: Some(error.to_string()),
            event_id: normalized.event_id,
            dedupe_key: normalized.dedupe_key,
            event_url: normalized.event_url,
            payload_excerpt: Some(normalized.payload_excerpt),
            task_id: None,
            conversation_id: Some(source.conversation_id.clone()),
            severity: normalized.severity,
            test_event: false,
        };
        let _ = append_event(&storage, record).await;
        let _ = persist_source_runtime_state(
            &storage,
            &mut sources,
            source_index,
            &received_at,
            "auth_failed",
            None,
        )
        .await;
        return (
            StatusCode::FORBIDDEN,
            Json(ErrorResponse {
                error: error.to_string(),
            }),
        )
            .into_response();
    }

    match route_event(
        &state,
        &storage,
        &mut sources,
        RouteEventInput {
            source_index,
            source: &source,
            headers: &headers,
            raw_body: &raw_body,
            payload: payload.as_ref(),
            test_event: false,
        },
    )
    .await
    {
        Ok(result) => (StatusCode::OK, Json(result)).into_response(),
        Err(error) => (
            StatusCode::INTERNAL_SERVER_ERROR,
            Json(ErrorResponse {
                error: error.to_string(),
            }),
        )
            .into_response(),
    }
}

pub(super) async fn test_webhook_source(
    State(state): State<AppState>,
    Path(source_id): Path<String>,
    Json(request): Json<WebhookTestRequest>,
) -> Response {
    let storage = {
        let agent = state.agent.read().await;
        agent.storage.clone()
    };
    let mut sources = match load_sources(&storage).await {
        Ok(sources) => sources,
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
    let Some(source_index) = sources.iter().position(|source| source.id == source_id) else {
        return (
            StatusCode::NOT_FOUND,
            Json(ErrorResponse {
                error: "Webhook source not found".to_string(),
            }),
        )
            .into_response();
    };
    let source = sources[source_index].clone();
    let payload = request.payload.unwrap_or_else(|| {
        serde_json::json!({
            "event": request.event_type.unwrap_or_else(|| "workflow_run".to_string()),
            "status": request.status.unwrap_or_else(|| "failed".to_string()),
            "title": request.subject.unwrap_or_else(|| format!("Test event for {}", source.name)),
            "message": "Synthetic webhook test payload generated from Settings > Webhooks.",
        })
    });
    let raw_body = serde_json::to_string(&payload).unwrap_or_else(|_| "{}".to_string());
    let headers = HeaderMap::new();

    match route_event(
        &state,
        &storage,
        &mut sources,
        RouteEventInput {
            source_index,
            source: &source,
            headers: &headers,
            raw_body: &raw_body,
            payload: Some(&payload),
            test_event: true,
        },
    )
    .await
    {
        Ok(result) => (StatusCode::OK, Json(result)).into_response(),
        Err(error) => (
            StatusCode::BAD_REQUEST,
            Json(ErrorResponse {
                error: error.to_string(),
            }),
        )
            .into_response(),
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::core::Agent;
    use axum::body::{to_bytes, Body};
    use axum::http::{header, Request};
    use axum::routing::{get, post};
    use tower::ServiceExt;

    async fn build_test_state() -> (AppState, tempfile::TempDir, tempfile::TempDir) {
        let config_dir = tempfile::tempdir().unwrap();
        let data_dir = tempfile::tempdir().unwrap();
        let shared = Arc::new(RwLock::new(
            Agent::init(
                config_dir.path(),
                data_dir.path(),
                crate::storage::DatabaseConfig::for_tests()
                    .expect("test database config should initialize"),
                None,
            )
            .await
            .unwrap(),
        ));
        let (trace_history, last_trace, tasks, user_profile, security_events, app_registry) = {
            let guard = shared.read().await;
            (
                guard.trace_history.clone(),
                guard.last_trace.clone(),
                guard.tasks.clone(),
                guard.user_profile.clone(),
                guard.security_events.clone(),
                guard.app_registry.clone(),
            )
        };
        (
            AppState {
                agent: shared,
                trace_history,
                last_trace,
                tasks,
                chat_task_cancellations: Arc::new(RwLock::new(HashMap::new())),
                action_test_cancellations: Arc::new(RwLock::new(HashMap::new())),
                chat_conversation_cancellations: Arc::new(RwLock::new(HashMap::new())),
                user_profile,
                tiered_rate_limiter: TieredRateLimiter::new(),
                api_key: Arc::new(RwLock::new(None)),
                api_key_expires_at: Arc::new(RwLock::new(None)),
                allow_insecure_no_auth: true,
                ui_sessions: Arc::new(RwLock::new(std::collections::HashMap::new())),
                local_ui_bootstrap_enabled: true,
                local_ui_bootstrap_tokens: Arc::new(RwLock::new(HashMap::new())),
                cookie_secure_default: false,
                oauth_states: Arc::new(RwLock::new(HashMap::new())),
                remote_login_attempts: Arc::new(RwLock::new(HashMap::new())),
                tunnel: Arc::new(RwLock::new(tunnel::TunnelState::new())),
                whatsapp_bridge: Arc::new(RwLock::new(WhatsAppBridgeState::new())),
                security_events,
                app_registry,
                app_publish_locks: Arc::new(parking_lot::Mutex::new(
                    std::collections::HashSet::new(),
                )),
                executor_client: None,
                workspace_client: None,
                application_registry: applications::ApplicationLauncherRegistry::default(),
                deployment_mode: DeploymentMode::TrustedLocal,
                server_role: HttpServerRole::ControlPlane,
                runtime_started_at: Instant::now(),
                public_app_bind_addr: None,
                public_app_base_url: None,
                release_update_cache: Arc::new(RwLock::new(ReleaseUpdateCache::default())),
            },
            config_dir,
            data_dir,
        )
    }

    async fn json_response(response: Response) -> serde_json::Value {
        let bytes = to_bytes(response.into_body(), 1024 * 1024).await.unwrap();
        serde_json::from_slice(&bytes).unwrap()
    }

    #[tokio::test]
    async fn webhook_source_secret_is_stored_encrypted_but_not_returned() {
        let (state, config_dir, data_dir) = build_test_state().await;
        let router = Router::new()
            .route(
                "/webhooks/sources",
                post(create_webhook_source).get(list_webhook_sources),
            )
            .with_state(state);

        let request = Request::builder()
            .method("POST")
            .uri("/webhooks/sources")
            .header(header::CONTENT_TYPE, "application/json")
            .body(Body::from(
                serde_json::json!({
                    "name": "GitHub CI",
                    "provider": "github",
                    "auth_mode": "header_token",
                    "secret_header": "X-Test-Secret",
                    "secret": "super-secret-token",
                    "instruction": "Triage failed CI runs and take the next safe step."
                })
                .to_string(),
            ))
            .unwrap();
        let response = router.clone().oneshot(request).await.unwrap();
        assert_eq!(response.status(), StatusCode::OK);
        let body = json_response(response).await;
        assert_eq!(
            body.get("source")
                .and_then(|value| value.get("secret_configured"))
                .and_then(|value| value.as_bool()),
            Some(true)
        );
        assert!(!body.to_string().contains("super-secret-token"));

        let manager = crate::core::config::SecureConfigManager::new_with_data_dir(
            config_dir.path(),
            Some(data_dir.path()),
        )
        .unwrap();
        let source_id = body
            .get("source")
            .and_then(|value| value.get("id"))
            .and_then(|value| value.as_str())
            .unwrap();
        assert_eq!(
            manager
                .get_custom_secret(&webhook_secret_key(source_id))
                .unwrap(),
            Some("super-secret-token".to_string())
        );

        let list_request = Request::builder()
            .method("GET")
            .uri("/webhooks/sources")
            .body(Body::empty())
            .unwrap();
        let list_response = router.oneshot(list_request).await.unwrap();
        let list_body = json_response(list_response).await;
        assert!(!list_body.to_string().contains("super-secret-token"));
    }

    #[tokio::test]
    async fn webhook_source_rejects_unknown_completion_channel() {
        let (state, _config_dir, _data_dir) = build_test_state().await;
        let router = Router::new()
            .route("/webhooks/sources", post(create_webhook_source))
            .with_state(state);

        let request = Request::builder()
            .method("POST")
            .uri("/webhooks/sources")
            .header(header::CONTENT_TYPE, "application/json")
            .body(Body::from(
                serde_json::json!({
                    "name": "Invalid Route",
                    "provider": "generic",
                    "auth_mode": "none",
                    "output_target": "channel",
                    "output_channel": "definitely-not-real",
                    "instruction": "Do nothing"
                })
                .to_string(),
            ))
            .unwrap();
        let response = router.oneshot(request).await.unwrap();
        assert_eq!(response.status(), StatusCode::BAD_REQUEST);
        let body = json_response(response).await;
        assert!(body
            .get("error")
            .and_then(|value| value.as_str())
            .is_some_and(|value| value.contains("not currently available")));
    }

    #[test]
    fn public_webhook_posture_requires_auth_for_internet_and_control_tunnel() {
        assert!(public_webhook_ingress_requires_auth_for_posture(
            DeploymentMode::InternetFacing,
            false,
            false
        ));
        assert!(public_webhook_ingress_requires_auth_for_posture(
            DeploymentMode::TrustedLocal,
            true,
            true
        ));
        assert!(!public_webhook_ingress_requires_auth_for_posture(
            DeploymentMode::TrustedLocal,
            true,
            false
        ));
    }

    #[test]
    fn public_webhook_auth_mode_rejects_no_auth_when_public() {
        let error = validate_public_webhook_auth_mode(WebhookAuthMode::None, true)
            .expect_err("public no-auth webhooks should be rejected");

        assert!(error
            .to_string()
            .contains("Public webhook sources require a secret"));
        validate_public_webhook_auth_mode(WebhookAuthMode::HeaderToken, true)
            .expect("authenticated public webhooks are allowed");
        validate_public_webhook_auth_mode(WebhookAuthMode::None, false)
            .expect("local-only no-auth webhooks are allowed");
    }

    #[tokio::test]
    async fn inbound_webhook_queues_autonomous_task() {
        let (state, _config_dir, _data_dir) = build_test_state().await;
        let router = Router::new()
            .route("/webhooks/sources", post(create_webhook_source))
            .route("/webhooks/events", get(list_webhook_events))
            .route("/webhook/inbound/{source_id}", post(handle_inbound_webhook))
            .with_state(state.clone());

        let create_request = Request::builder()
            .method("POST")
            .uri("/webhooks/sources")
            .header(header::CONTENT_TYPE, "application/json")
            .body(Body::from(
                serde_json::json!({
                    "name": "Build Alerts",
                    "provider": "generic",
                    "auth_mode": "header_token",
                    "secret_header": crate::branding::WEBHOOK_SECRET_HEADER,
                    "secret": "abc123",
                    "notify_on_queued": true,
                    "notify_on_success": true,
                    "notify_on_failure": true,
                    "output_target": "preferred",
                    "instruction": "When CI fails, inspect the event and take the next safe action."
                })
                .to_string(),
            ))
            .unwrap();
        let create_body =
            json_response(router.clone().oneshot(create_request).await.unwrap()).await;
        let source_id = create_body
            .get("source")
            .and_then(|value| value.get("id"))
            .and_then(|value| value.as_str())
            .unwrap();

        let inbound_request = Request::builder()
            .method("POST")
            .uri(format!("/webhook/inbound/{}", source_id))
            .header(crate::branding::WEBHOOK_SECRET_HEADER, "abc123")
            .header(header::CONTENT_TYPE, "application/json")
            .body(Body::from(
                serde_json::json!({
                    "event": "workflow_run",
                    "status": "failed",
                    "title": "core-api / build"
                })
                .to_string(),
            ))
            .unwrap();
        let inbound_response = router.clone().oneshot(inbound_request).await.unwrap();
        assert_eq!(inbound_response.status(), StatusCode::OK);
        let inbound_body = json_response(inbound_response).await;
        assert_eq!(
            inbound_body.get("queued").and_then(|value| value.as_bool()),
            Some(true)
        );

        let tasks = state.tasks.read().await;
        let webhook_task = tasks
            .all()
            .iter()
            .find(|task| task.action == "autonomy_action")
            .cloned();
        drop(tasks);
        let webhook_task = webhook_task.expect("webhook task should be queued");
        assert_eq!(
            webhook_task
                .arguments
                .get("report_to")
                .and_then(|value| value.as_str()),
            Some("preferred")
        );
        let webhook_meta = webhook_task
            .arguments
            .get("_automation")
            .and_then(|value| value.get("webhook"))
            .cloned()
            .unwrap_or_default();
        assert_eq!(
            webhook_meta
                .get("notify_on_success")
                .and_then(|value| value.as_bool()),
            Some(true)
        );
        assert_eq!(
            webhook_meta
                .get("output_target")
                .and_then(|value| value.as_str()),
            Some("preferred")
        );

        let events_request = Request::builder()
            .method("GET")
            .uri("/webhooks/events?limit=5")
            .body(Body::empty())
            .unwrap();
        let events_body = json_response(router.oneshot(events_request).await.unwrap()).await;
        assert_eq!(
            events_body
                .get("events")
                .and_then(|value| value.as_array())
                .and_then(|value| value.first())
                .and_then(|value| value.get("outcome"))
                .and_then(|value| value.as_str()),
            Some("queued")
        );
    }

    #[tokio::test]
    async fn failures_only_source_ignores_success_events() {
        let (state, _config_dir, _data_dir) = build_test_state().await;
        let router = Router::new()
            .route("/webhooks/sources", post(create_webhook_source))
            .route("/webhook/inbound/{source_id}", post(handle_inbound_webhook))
            .with_state(state.clone());

        let create_request = Request::builder()
            .method("POST")
            .uri("/webhooks/sources")
            .header(header::CONTENT_TYPE, "application/json")
            .body(Body::from(
                serde_json::json!({
                    "name": "Deploy Failures",
                    "provider": "generic",
                    "auth_mode": "header_token",
                    "secret_header": "X-Webhook-Secret",
                    "secret": "keepme",
                    "match_mode": "failures_only",
                    "instruction": "Act on failed deploys."
                })
                .to_string(),
            ))
            .unwrap();
        let create_body =
            json_response(router.clone().oneshot(create_request).await.unwrap()).await;
        let source_id = create_body
            .get("source")
            .and_then(|value| value.get("id"))
            .and_then(|value| value.as_str())
            .unwrap();

        let inbound_request = Request::builder()
            .method("POST")
            .uri(format!("/webhook/inbound/{}", source_id))
            .header("X-Webhook-Secret", "keepme")
            .header(header::CONTENT_TYPE, "application/json")
            .body(Body::from(
                serde_json::json!({
                    "event": "deployment_status",
                    "status": "success",
                    "title": "production deploy"
                })
                .to_string(),
            ))
            .unwrap();
        let inbound_body = json_response(router.oneshot(inbound_request).await.unwrap()).await;
        assert_eq!(
            inbound_body
                .get("matched")
                .and_then(|value| value.as_bool()),
            Some(false)
        );
        let tasks = state.tasks.read().await;
        assert!(tasks.all().is_empty());
    }

    #[tokio::test]
    async fn duplicate_delivery_is_ignored() {
        let (state, _config_dir, _data_dir) = build_test_state().await;
        let router = Router::new()
            .route("/webhooks/sources", post(create_webhook_source))
            .route("/webhook/inbound/{source_id}", post(handle_inbound_webhook))
            .with_state(state.clone());

        let create_request = Request::builder()
            .method("POST")
            .uri("/webhooks/sources")
            .header(header::CONTENT_TYPE, "application/json")
            .body(Body::from(
                serde_json::json!({
                    "name": "Pager",
                    "provider": "generic",
                    "auth_mode": "header_token",
                    "secret_header": "X-Webhook-Secret",
                    "secret": "dupe-secret",
                    "instruction": "Handle incidents.",
                    "dedupe_window_secs": 900
                })
                .to_string(),
            ))
            .unwrap();
        let create_body =
            json_response(router.clone().oneshot(create_request).await.unwrap()).await;
        let source_id = create_body
            .get("source")
            .and_then(|value| value.get("id"))
            .and_then(|value| value.as_str())
            .unwrap();

        let payload = serde_json::json!({
            "event": "incident",
            "status": "critical",
            "title": "api latency spike",
            "id": "incident-42"
        })
        .to_string();
        let build_request = || {
            Request::builder()
                .method("POST")
                .uri(format!("/webhook/inbound/{}", source_id))
                .header("X-Webhook-Secret", "dupe-secret")
                .header(header::CONTENT_TYPE, "application/json")
                .body(Body::from(payload.clone()))
                .unwrap()
        };

        let first = json_response(router.clone().oneshot(build_request()).await.unwrap()).await;
        let second = json_response(router.oneshot(build_request()).await.unwrap()).await;
        assert_eq!(
            first.get("queued").and_then(|value| value.as_bool()),
            Some(true)
        );
        assert_eq!(
            second.get("duplicate").and_then(|value| value.as_bool()),
            Some(true)
        );
    }
}
