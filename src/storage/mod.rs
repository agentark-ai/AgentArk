//! Database storage using SeaORM backed by PostgreSQL.
pub mod encrypted;
pub mod entities;
mod migrations;

use crate::crypto::KeyManager;
use anyhow::{anyhow, Context, Result};
use base64::{engine::general_purpose::STANDARD as BASE64, Engine as _};
use sea_orm::entity::prelude::PgVector;
use sea_orm::sea_query::{
    Alias, Expr, Func, OnConflict, Order, PostgresQueryBuilder, Query, SimpleExpr,
};
#[allow(unused_imports)]
use sea_orm::{
    ActiveModelTrait, ActiveValue, ColumnTrait, Condition, ConnectOptions, ConnectionTrait,
    Database, DatabaseConnection, DatabaseTransaction, DbBackend, EntityTrait, FromQueryResult,
    PaginatorTrait, QueryFilter, QueryOrder, QuerySelect, Set, Statement, TransactionTrait,
    TryGetable, Unchanged,
};
use sha2::{Digest, Sha256};
use std::collections::HashMap;
use std::sync::{Arc, OnceLock, RwLock};
use std::time::Duration;

pub use entities::*;

const DB_CONNECT_RETRY_WINDOW_SECS: u64 = 60;
const DB_CONNECT_INITIAL_RETRY_DELAY_MS: u64 = 500;
const DB_CONNECT_MAX_RETRY_DELAY_SECS: u64 = 5;

/// Database storage using SeaORM
#[derive(Clone)]
pub struct Storage {
    db: DatabaseConnection,
}
#[derive(Debug, Clone, Default, serde::Serialize)]
pub struct HousekeepingStatus {
    pub housekeeping_last_run_at: Option<String>,
    pub notification_last_run_at: Option<String>,
}

#[derive(Debug, Clone, serde::Serialize, serde::Deserialize)]
pub struct LearnedFactRecord {
    pub id: String,
    pub fact: String,
    pub key: Option<String>,
    pub value: String,
    pub confidence: f32,
    pub sources: String,
    pub created_at: String,
    pub updated_at: String,
    pub project_id: Option<String>,
    pub scope: String,
    pub memory_kind: Option<String>,
    pub memory_category: String,
    pub topics: Vec<String>,
}

#[derive(Debug, Clone, serde::Serialize, serde::Deserialize)]
struct KvLeaseRecord {
    owner_id: String,
    acquired_at: String,
    expires_at: String,
    #[serde(default)]
    fence_token: u64,
}

#[derive(Debug, Clone)]
pub struct KvLeaseGuard {
    pub owner_id: String,
    pub fence_token: u64,
}

#[derive(Debug, Clone, serde::Serialize, serde::Deserialize)]
pub struct UploadManifest {
    pub id: String,
    pub original_name: String,
    pub stored_name: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub content_type: Option<String>,
    pub size_bytes: u64,
    pub created_at: String,
}

#[derive(Debug, Clone)]
pub struct ActionCatalogIndexEntry {
    pub action_name: String,
    pub source: String,
    pub version: String,
    pub descriptor_hash: String,
    pub descriptor_text: String,
    pub enabled: bool,
    pub metadata_json: serde_json::Value,
    pub embedding: Option<PgVector>,
}

#[derive(Debug, Clone, FromQueryResult, serde::Serialize)]
pub struct ExecutionTraceSummaryRow {
    pub id: String,
    pub message: String,
    pub channel: String,
    pub started_at: Option<String>,
    pub completed_at: Option<String>,
    pub duration_ms: Option<i32>,
    pub step_count: i32,
    pub steps_json: String,
    pub model: Option<String>,
    pub total_tokens: i32,
    pub cost_usd: f64,
    pub complexity: Option<String>,
    pub created_at: String,
}

#[derive(Debug, Clone, FromQueryResult)]
pub struct OperationalLogVersionMetricRow {
    pub success: bool,
    pub latency_ms: Option<i64>,
    pub policy_version: Option<String>,
    pub strategy_version: Option<String>,
}

#[derive(Debug, Clone, serde::Serialize)]
pub struct ExecutionTraceMessageMetrics {
    pub duration_ms: Option<i64>,
    pub input_tokens: i64,
    pub output_tokens: i64,
    pub total_tokens: i64,
    pub cached_prompt_tokens: i64,
    pub cache_creation_prompt_tokens: i64,
    pub time_to_first_token_ms: Option<i64>,
}

#[derive(Debug, Clone, FromQueryResult)]
struct ExecutionTraceMessageMetricRow {
    id: String,
    duration_ms: Option<i32>,
    input_tokens: i32,
    output_tokens: i32,
    total_tokens: i32,
    steps_json: String,
}

#[derive(Debug, Clone, Default, serde::Serialize)]
pub struct DbConnectionActivityCounts {
    pub active: i64,
    pub idle: i64,
    pub total: i64,
}

#[derive(Debug, Clone)]
pub struct DatabaseConfig {
    pub url: String,
    pub max_connections: u32,
    pub min_connections: u32,
    pub connect_timeout_secs: u64,
    pub acquire_timeout_secs: u64,
    pub statement_timeout_ms: u64,
    pub idle_timeout_secs: u64,
    pub schema: Option<String>,
}

impl DatabaseConfig {
    pub fn new(url: impl Into<String>) -> Self {
        Self {
            url: url.into(),
            max_connections: 32,
            min_connections: 2,
            connect_timeout_secs: 5,
            acquire_timeout_secs: 30,
            statement_timeout_ms: 30_000,
            idle_timeout_secs: 300,
            schema: None,
        }
    }

    pub fn apply_optional_env_overrides(&mut self) {
        if let Ok(value) = std::env::var("AGENTARK_DB_MAX_CONNECTIONS") {
            if let Ok(parsed) = value.parse::<u32>() {
                self.max_connections = parsed.max(1);
            }
        }
        if let Ok(value) = std::env::var("AGENTARK_DB_MIN_CONNECTIONS") {
            if let Ok(parsed) = value.parse::<u32>() {
                self.min_connections = parsed.max(1);
            }
        }
        if let Ok(value) = std::env::var("AGENTARK_DB_CONNECT_TIMEOUT_SECS") {
            if let Ok(parsed) = value.parse::<u64>() {
                self.connect_timeout_secs = parsed.max(1);
            }
        }
        if let Ok(value) = std::env::var("AGENTARK_DB_ACQUIRE_TIMEOUT_SECS") {
            if let Ok(parsed) = value.parse::<u64>() {
                self.acquire_timeout_secs = parsed.max(1);
            }
        }
        if let Ok(value) = std::env::var("AGENTARK_DB_STATEMENT_TIMEOUT_MS") {
            if let Ok(parsed) = value.parse::<u64>() {
                self.statement_timeout_ms = parsed.max(1);
            }
        }
        if let Ok(value) = std::env::var("AGENTARK_DB_IDLE_TIMEOUT_SECS") {
            if let Ok(parsed) = value.parse::<u64>() {
                self.idle_timeout_secs = parsed.max(1);
            }
        }
        if let Ok(value) = std::env::var("AGENTARK_DB_SCHEMA") {
            let trimmed = value.trim();
            if !trimmed.is_empty() {
                self.schema = Some(trimmed.to_string());
            }
        }
    }

    pub fn from_env() -> Result<Self> {
        let url = std::env::var("AGENTARK_DATABASE_URL")
            .ok()
            .or_else(database_url_from_postgres_secret_env)
            .context("AGENTARK_DATABASE_URL must be set for Postgres-backed storage")?;
        let mut config = Self::new(url);
        config.apply_optional_env_overrides();
        Ok(config)
    }

    #[cfg(test)]
    pub fn for_tests() -> Result<Self> {
        let base = test_database_url()?;
        let mut config = Self::new(base);
        config.max_connections = 2;
        config.min_connections = 1;
        config.connect_timeout_secs = 15;
        config.acquire_timeout_secs = 15;
        Ok(config)
    }

    fn validate(&self) -> Result<()> {
        let lower = self.url.to_ascii_lowercase();
        if !lower.starts_with("postgres://") && !lower.starts_with("postgresql://") {
            anyhow::bail!("AGENTARK_DATABASE_URL must be a postgres:// or postgresql:// URL");
        }
        if self.schema.is_some() {
            anyhow::bail!(
                "Custom Postgres schemas are not supported by the fresh-install-only bootstrap"
            );
        }
        Ok(())
    }

    fn target_summary(&self) -> String {
        match url::Url::parse(&self.url) {
            Ok(parsed) => {
                let host = parsed.host_str().unwrap_or("unknown-host");
                let port = parsed.port_or_known_default().unwrap_or(5432);
                let database = parsed
                    .path_segments()
                    .and_then(|mut segments| segments.rfind(|segment| !segment.is_empty()))
                    .unwrap_or("unknown-db");
                format!("{host}:{port}/{database}")
            }
            Err(_) => "<invalid-postgres-url>".to_string(),
        }
    }

    fn connect_options(&self) -> ConnectOptions {
        let mut options = ConnectOptions::new(self.url.clone());
        let max_connections = self.max_connections.max(1);
        let min_connections = self.min_connections.max(1).min(max_connections);
        let statement_timeout_ms = self.statement_timeout_ms.max(1).to_string();
        options
            .max_connections(max_connections)
            .min_connections(min_connections)
            .connect_timeout(Duration::from_secs(self.connect_timeout_secs.max(1)))
            .idle_timeout(Duration::from_secs(self.idle_timeout_secs.max(1)))
            .acquire_timeout(Duration::from_secs(self.acquire_timeout_secs.max(1)))
            .sqlx_logging(false)
            .map_sqlx_postgres_opts(move |opts| {
                opts.application_name("agentark").options([
                    ("statement_timeout", statement_timeout_ms.as_str()),
                    ("client_min_messages", "warning"),
                ])
            });
        if let Some(schema) = self.schema.as_deref() {
            options.set_schema_search_path(schema);
        }
        options
    }
}

fn database_url_from_postgres_secret_env() -> Option<String> {
    let password = std::env::var("AGENTARK_POSTGRES_PASSWORD")
        .ok()
        .filter(|value| !value.trim().is_empty())
        .or_else(|| {
            std::env::var("AGENTARK_POSTGRES_PASSWORD_FILE")
                .ok()
                .and_then(|path| std::fs::read_to_string(path).ok())
                .map(|value| value.trim().to_string())
                .filter(|value| !value.is_empty())
        })?;
    let user = std::env::var("AGENTARK_POSTGRES_USER").unwrap_or_else(|_| "agentark".to_string());
    let host = std::env::var("AGENTARK_POSTGRES_HOST").unwrap_or_else(|_| "postgres".to_string());
    let port = std::env::var("AGENTARK_POSTGRES_PORT")
        .ok()
        .and_then(|value| value.parse::<u16>().ok())
        .unwrap_or(5432);
    let database = std::env::var("AGENTARK_POSTGRES_DB").unwrap_or_else(|_| "agentark".to_string());
    postgres_url_from_parts(&user, &password, &host, port, &database).ok()
}

fn postgres_url_from_parts(
    user: &str,
    password: &str,
    host: &str,
    port: u16,
    database: &str,
) -> Result<String> {
    let mut url = url::Url::parse("postgres://localhost/agentark")?;
    url.set_username(user)
        .map_err(|_| anyhow::anyhow!("invalid Postgres user for database URL"))?;
    url.set_password(Some(password))
        .map_err(|_| anyhow::anyhow!("invalid Postgres password for database URL"))?;
    url.set_host(Some(host))
        .map_err(|_| anyhow::anyhow!("invalid Postgres host for database URL"))?;
    url.set_port(Some(port))
        .map_err(|_| anyhow::anyhow!("invalid Postgres port for database URL"))?;
    url.set_path(&format!("/{database}"));
    Ok(url.to_string())
}

#[cfg(test)]
fn test_database_url() -> Result<String> {
    let url = std::env::var("AGENTARK_TEST_DATABASE_URL").context(
        "DB integration tests require AGENTARK_TEST_DATABASE_URL; default tests must not open Postgres",
    )?;
    ensure_database_url_is_test_scoped(&url)?;
    Ok(url)
}

#[cfg(test)]
fn ensure_database_url_is_test_scoped(raw: &str) -> Result<()> {
    let parsed = url::Url::parse(raw)
        .map_err(|error| anyhow::anyhow!("test database URL is invalid: {}", error))?;
    let database = parsed
        .path_segments()
        .and_then(|mut segments| segments.rfind(|segment| !segment.is_empty()))
        .unwrap_or_default();
    if database_is_test_scoped(database) {
        Ok(())
    } else {
        anyhow::bail!(
            "Refusing to run tests against non-test Postgres database '{}'. Set AGENTARK_TEST_DATABASE_URL to an isolated database such as agentark_test_<id>.",
            database
        )
    }
}

#[cfg(test)]
fn database_is_test_scoped(database: &str) -> bool {
    let normalized = database.trim().to_ascii_lowercase();
    normalized.starts_with("agentark_test")
        || normalized.starts_with("test_agentark")
        || normalized.ends_with("_test")
        || normalized.contains("_test_")
}

#[cfg(test)]
mod database_config_tests {
    use super::*;
    use std::sync::Mutex;

    static ENV_LOCK: Mutex<()> = Mutex::new(());

    #[test]
    fn test_database_guard_rejects_live_agentark_database() {
        assert!(ensure_database_url_is_test_scoped(
            "postgres://agentark:secret@127.0.0.1/agentark"
        )
        .is_err());
        assert!(!database_is_test_scoped("agentark"));
    }

    #[test]
    fn test_database_guard_accepts_isolated_test_database() {
        assert!(ensure_database_url_is_test_scoped(
            "postgres://agentark:secret@127.0.0.1/agentark_test_123"
        )
        .is_ok());
        assert!(database_is_test_scoped("agentark_test_123"));
    }

    #[test]
    fn database_config_defaults_use_separate_pool_acquire_timeout() {
        let config = DatabaseConfig::new("postgres://agentark:secret@127.0.0.1/agentark_test");

        assert_eq!(config.max_connections, 32);
        assert_eq!(config.min_connections, 2);
        assert_eq!(config.connect_timeout_secs, 5);
        assert_eq!(config.acquire_timeout_secs, 30);
    }

    #[test]
    fn database_config_env_overrides_include_min_and_acquire_timeouts() {
        let _guard = ENV_LOCK
            .lock()
            .expect("env test lock should not be poisoned");
        let keys = [
            "AGENTARK_DB_MAX_CONNECTIONS",
            "AGENTARK_DB_MIN_CONNECTIONS",
            "AGENTARK_DB_CONNECT_TIMEOUT_SECS",
            "AGENTARK_DB_ACQUIRE_TIMEOUT_SECS",
        ];
        let previous = keys
            .iter()
            .map(|key| (*key, std::env::var(key).ok()))
            .collect::<Vec<_>>();
        for (key, value) in [
            ("AGENTARK_DB_MAX_CONNECTIONS", "24"),
            ("AGENTARK_DB_MIN_CONNECTIONS", "3"),
            ("AGENTARK_DB_CONNECT_TIMEOUT_SECS", "7"),
            ("AGENTARK_DB_ACQUIRE_TIMEOUT_SECS", "45"),
        ] {
            std::env::set_var(key, value);
        }

        let mut config = DatabaseConfig::new("postgres://agentark:secret@127.0.0.1/agentark_test");
        config.apply_optional_env_overrides();

        for (key, value) in previous {
            if let Some(value) = value {
                std::env::set_var(key, value);
            } else {
                std::env::remove_var(key);
            }
        }

        assert_eq!(config.max_connections, 24);
        assert_eq!(config.min_connections, 3);
        assert_eq!(config.connect_timeout_secs, 7);
        assert_eq!(config.acquire_timeout_secs, 45);
    }
}

static STORAGE_KEY_MANAGER: OnceLock<RwLock<Option<Arc<KeyManager>>>> = OnceLock::new();

pub(crate) const ENCRYPTED_STORAGE_UNAVAILABLE: &str = "[Encrypted content unavailable]";

const EXPERIENCE_RUN_LIGHT_UPSERT_COLUMNS: &[experience_run::Column] = &[
    experience_run::Column::ExecutionRunId,
    experience_run::Column::TraceId,
    experience_run::Column::ConversationId,
    experience_run::Column::ProjectId,
    experience_run::Column::Channel,
    experience_run::Column::Scope,
    experience_run::Column::IntentKey,
    experience_run::Column::TaskType,
    experience_run::Column::ToolSequenceDigest,
    experience_run::Column::StrategyVersion,
    experience_run::Column::PolicyVersion,
    experience_run::Column::PromptVersion,
    experience_run::Column::ModelSlot,
    experience_run::Column::SuccessState,
    experience_run::Column::CorrectionState,
    experience_run::Column::Consolidated,
    experience_run::Column::AcceptedAt,
    experience_run::Column::CorrectedAt,
    experience_run::Column::HeuristicReflected,
    experience_run::Column::HeuristicReflectionStatus,
    experience_run::Column::HeuristicReflectionAttemptedAt,
    experience_run::Column::HeuristicReflectionCompletedAt,
    experience_run::Column::HeuristicLessonId,
    experience_run::Column::UpdatedAt,
];

const EXPERIENCE_ITEM_LIGHT_UPSERT_COLUMNS: &[experience_item::Column] = &[
    experience_item::Column::Kind,
    experience_item::Column::Scope,
    experience_item::Column::ProjectId,
    experience_item::Column::ConversationId,
    experience_item::Column::Title,
    experience_item::Column::NormalizedKey,
    experience_item::Column::Confidence,
    experience_item::Column::SupportCount,
    experience_item::Column::ContradictionCount,
    experience_item::Column::Status,
    experience_item::Column::LastSupportedAt,
    experience_item::Column::LastContradictedAt,
    experience_item::Column::UpdatedAt,
];

const MEMORY_CAPTURE_EVENT_LIGHT_UPSERT_COLUMNS: &[memory_capture_event::Column] = &[
    memory_capture_event::Column::SourceMessageId,
    memory_capture_event::Column::ConversationId,
    memory_capture_event::Column::ProjectId,
    memory_capture_event::Column::Channel,
    memory_capture_event::Column::Status,
    memory_capture_event::Column::CaptureKind,
    memory_capture_event::Column::SourceHash,
    memory_capture_event::Column::ReplayCount,
    memory_capture_event::Column::NextRetryAt,
    memory_capture_event::Column::CompletedAt,
    memory_capture_event::Column::UpdatedAt,
];

const MEMORY_OPERATION_LIGHT_UPSERT_COLUMNS: &[memory_operation::Column] = &[
    memory_operation::Column::CaptureEventId,
    memory_operation::Column::OperationType,
    memory_operation::Column::Status,
    memory_operation::Column::TargetMemoryId,
    memory_operation::Column::AppliedMemoryId,
    memory_operation::Column::Key,
    memory_operation::Column::MemoryKind,
    memory_operation::Column::Durability,
    memory_operation::Column::Scope,
    memory_operation::Column::ProjectId,
    memory_operation::Column::ConversationId,
    memory_operation::Column::Confidence,
    memory_operation::Column::LooksSensitive,
    memory_operation::Column::ValidFrom,
    memory_operation::Column::ExpiresAt,
    memory_operation::Column::ReviewAt,
    memory_operation::Column::AppliedAt,
    memory_operation::Column::ReviewedAt,
    memory_operation::Column::UpdatedAt,
];

const SEMANTIC_WORK_UNIT_LIGHT_UPSERT_COLUMNS: &[semantic_work_unit::Column] = &[
    semantic_work_unit::Column::SourceKind,
    semantic_work_unit::Column::SourceId,
    semantic_work_unit::Column::ConversationId,
    semantic_work_unit::Column::ProjectId,
    semantic_work_unit::Column::Channel,
    semantic_work_unit::Column::TextHash,
    semantic_work_unit::Column::OccurredAt,
    semantic_work_unit::Column::PeriodStart,
    semantic_work_unit::Column::PeriodEnd,
    semantic_work_unit::Column::MessageCount,
    semantic_work_unit::Column::UpdatedAt,
];

fn storage_key_manager_slot() -> &'static RwLock<Option<Arc<KeyManager>>> {
    STORAGE_KEY_MANAGER.get_or_init(|| RwLock::new(None))
}

pub fn install_storage_key_manager(key_manager: Arc<KeyManager>) {
    if let Ok(mut guard) = storage_key_manager_slot().write() {
        *guard = Some(key_manager);
    }
}

fn current_storage_key_manager() -> Option<Arc<KeyManager>> {
    storage_key_manager_slot()
        .read()
        .ok()
        .and_then(|guard| guard.clone())
}

fn encrypt_storage_string(value: &str) -> Result<String> {
    if value.is_empty() {
        return Ok(String::new());
    }
    if let Some(key_manager) = current_storage_key_manager() {
        Ok(key_manager.encrypt_string(value)?)
    } else {
        Ok(value.to_string())
    }
}

fn decode_storage_ciphertext_bytes(value: &str) -> Option<Vec<u8>> {
    let trimmed = value.trim();
    if trimmed.len() < 24 || trimmed.chars().any(char::is_whitespace) {
        return None;
    }
    if !trimmed
        .chars()
        .all(|c| c.is_ascii_alphanumeric() || matches!(c, '+' | '/' | '=' | '-' | '_'))
    {
        return None;
    }
    let mut padded = trimmed.to_string();
    while !padded.len().is_multiple_of(4) {
        padded.push('=');
    }
    BASE64.decode(padded.as_bytes()).ok()
}

fn looks_like_encrypted_storage_string(value: &str) -> bool {
    decode_storage_ciphertext_bytes(value)
        .map(|bytes| bytes.len() >= 24)
        .unwrap_or(false)
}

fn decrypt_storage_string(value: &str) -> String {
    if value.is_empty() {
        return String::new();
    }
    if let Some(key_manager) = current_storage_key_manager() {
        if let Ok(decrypted) = key_manager.decrypt_string(value) {
            return decrypted;
        }
    }
    if looks_like_encrypted_storage_string(value) {
        ENCRYPTED_STORAGE_UNAVAILABLE.to_string()
    } else {
        value.to_string()
    }
}

fn encrypt_optional_storage_string(value: Option<&str>) -> Result<Option<String>> {
    value.map(encrypt_storage_string).transpose()
}

fn set_if_changed<T>(field: &mut ActiveValue<T>, current: &T, next: &T, changed: &mut bool)
where
    T: Clone + PartialEq + Into<sea_orm::sea_query::Value>,
{
    if current != next {
        *field = Set(next.clone());
        *changed = true;
    }
}

fn encrypted_storage_string_matches(stored: &str, next_plaintext: &str) -> bool {
    decrypt_storage_string(stored) == next_plaintext
}

fn encrypted_optional_storage_string_matches(
    stored: &Option<String>,
    next_plaintext: &Option<String>,
) -> bool {
    match (stored.as_deref(), next_plaintext.as_deref()) {
        (None, None) => true,
        (Some(stored), Some(next_plaintext)) => {
            encrypted_storage_string_matches(stored, next_plaintext)
        }
        _ => false,
    }
}

fn set_encrypted_string_if_changed(
    field: &mut ActiveValue<String>,
    current: &str,
    next: &str,
    changed: &mut bool,
) -> Result<()> {
    if !encrypted_storage_string_matches(current, next) {
        *field = Set(encrypt_storage_string(next)?);
        *changed = true;
    }
    Ok(())
}

fn set_encrypted_optional_string_if_changed(
    field: &mut ActiveValue<Option<String>>,
    current: &Option<String>,
    next: &Option<String>,
    changed: &mut bool,
) -> Result<()> {
    if !encrypted_optional_storage_string_matches(current, next) {
        *field = Set(encrypt_optional_storage_string(next.as_deref())?);
        *changed = true;
    }
    Ok(())
}

fn decrypt_optional_storage_string(value: Option<String>) -> Option<String> {
    value.map(|inner| decrypt_storage_string(&inner))
}

fn decrypt_message_model(model: &mut message::Model) {
    model.content = decrypt_storage_string(&model.content);
    model.tool_calls_json = decrypt_optional_storage_string(model.tool_calls_json.take());
    model.tool_call_id = decrypt_optional_storage_string(model.tool_call_id.take());
    model.provider_message_json =
        decrypt_optional_storage_string(model.provider_message_json.take());
}

fn trace_time_to_first_token_ms(steps_json: &str) -> Option<i64> {
    let steps: Vec<crate::core::ExecutionStep> = serde_json::from_str(steps_json).ok()?;
    let mut first_stream_activity = None;
    for step in steps {
        let Some(data) = step.data.as_deref() else {
            continue;
        };
        let Ok(payload) = serde_json::from_str::<serde_json::Value>(data) else {
            continue;
        };
        let Some(metric) = payload.get("metric").and_then(|value| value.as_str()) else {
            continue;
        };
        if !matches!(
            metric,
            "time_to_first_token" | "time_to_first_stream_activity"
        ) {
            continue;
        }
        let duration = payload
            .get("duration_ms")
            .and_then(|value| {
                value
                    .as_i64()
                    .or_else(|| value.as_u64().map(|v| v.min(i64::MAX as u64) as i64))
            })
            .or_else(|| {
                step.duration_ms
                    .map(|value| value.min(i64::MAX as u64) as i64)
            });
        if metric == "time_to_first_token" && duration.is_some() {
            return duration;
        }
        if first_stream_activity.is_none() {
            first_stream_activity = duration;
        }
    }
    first_stream_activity
}

fn trace_prompt_cache_metrics(steps_json: &str) -> (i64, i64) {
    let Ok(steps) = serde_json::from_str::<Vec<crate::core::ExecutionStep>>(steps_json) else {
        return (0, 0);
    };
    let mut cached_prompt_tokens = 0i64;
    let mut cache_creation_prompt_tokens = 0i64;
    for step in steps {
        let Some(data) = step.data.as_deref() else {
            continue;
        };
        let Ok(payload) = serde_json::from_str::<serde_json::Value>(data) else {
            continue;
        };
        if payload.get("event").and_then(|value| value.as_str()) != Some("model_completed") {
            continue;
        }
        cached_prompt_tokens = cached_prompt_tokens.saturating_add(
            payload
                .get("cache_read_tokens")
                .and_then(|value| {
                    value
                        .as_i64()
                        .or_else(|| value.as_u64().map(|v| v.min(i64::MAX as u64) as i64))
                })
                .unwrap_or_default(),
        );
        cache_creation_prompt_tokens = cache_creation_prompt_tokens.saturating_add(
            payload
                .get("cache_creation_tokens")
                .and_then(|value| {
                    value
                        .as_i64()
                        .or_else(|| value.as_u64().map(|v| v.min(i64::MAX as u64) as i64))
                })
                .unwrap_or_default(),
        );
    }
    (cached_prompt_tokens, cache_creation_prompt_tokens)
}

fn pgvector_sql_literal(embedding: &PgVector) -> String {
    let values = embedding
        .as_slice()
        .iter()
        .map(|value| format!("{value:.8}"))
        .collect::<Vec<_>>()
        .join(",");
    format!("'[{values}]'::vector")
}

fn sql_string_literal(value: &str) -> String {
    format!("'{}'", value.replace('\'', "''"))
}

fn sql_string_list(values: &[String]) -> String {
    values
        .iter()
        .map(|value| sql_string_literal(value))
        .collect::<Vec<_>>()
        .join(",")
}

fn experience_memory_write_lock_key(
    _kind: &str,
    scope: &str,
    project_id: Option<&str>,
    conversation_id: Option<&str>,
) -> i64 {
    let mut hasher = Sha256::new();
    for part in [
        "experience_memory_write",
        scope.trim(),
        project_id.unwrap_or_default().trim(),
        conversation_id.unwrap_or_default().trim(),
    ] {
        hasher.update([0u8]);
        hasher.update(part.as_bytes());
    }
    let digest = hasher.finalize();
    i64::from_be_bytes(
        digest[..8]
            .try_into()
            .expect("sha256 digest has 8 leading bytes"),
    )
}

fn is_safe_db_identifier_part(value: &str) -> bool {
    !value.trim().is_empty()
        && value
            .chars()
            .all(|ch| ch.is_ascii_alphanumeric() || ch == '_')
}

fn normalize_public_table_name(raw: &str) -> Result<String> {
    let trimmed = raw.trim();
    if trimmed.is_empty() {
        anyhow::bail!("Table name cannot be empty");
    }
    let table = if let Some((schema, table)) = trimmed.split_once('.') {
        if schema.trim() != "public" || !is_safe_db_identifier_part(table.trim()) {
            anyhow::bail!("Only public-schema AgentArk tables are allowed");
        }
        table.trim().to_string()
    } else {
        if !is_safe_db_identifier_part(trimmed) {
            anyhow::bail!("Invalid table name '{}'", trimmed);
        }
        trimmed.to_string()
    };
    Ok(table)
}

fn normalize_db_column_name(raw: &str) -> Result<String> {
    let trimmed = raw.trim();
    if !is_safe_db_identifier_part(trimmed) {
        anyhow::bail!("Invalid column name '{}'", trimmed);
    }
    Ok(trimmed.to_string())
}

fn json_scalar_to_simple_expr(value: &serde_json::Value) -> Result<SimpleExpr> {
    match value {
        serde_json::Value::Bool(inner) => Ok(Expr::value(*inner)),
        serde_json::Value::Number(inner) => {
            if let Some(value) = inner.as_i64() {
                Ok(Expr::value(value))
            } else if let Some(value) = inner.as_u64() {
                Ok(Expr::value(value as i64))
            } else if let Some(value) = inner.as_f64() {
                Ok(Expr::value(value))
            } else {
                anyhow::bail!("Unsupported numeric filter value '{}'", inner);
            }
        }
        serde_json::Value::String(inner) => Ok(Expr::value(inner.clone())),
        serde_json::Value::Null => anyhow::bail!(
            "Null filters must use `is_null` or `not_null` operators instead of a scalar value"
        ),
        _ => anyhow::bail!("Only scalar filter values are supported in structured DB queries"),
    }
}

fn lease_is_active(record: &KvLeaseRecord, now: chrono::DateTime<chrono::Utc>) -> bool {
    chrono::DateTime::parse_from_rfc3339(&record.expires_at)
        .map(|value| value.with_timezone(&chrono::Utc) > now)
        .unwrap_or(false)
}

fn next_lease_fence_token(existing: Option<&KvLeaseRecord>) -> u64 {
    existing
        .map(|record| record.fence_token.saturating_add(1))
        .unwrap_or(1)
}

fn kv_lease_guard_is_current(
    record: &KvLeaseRecord,
    guard: &KvLeaseGuard,
    now: chrono::DateTime<chrono::Utc>,
) -> bool {
    record.owner_id == guard.owner_id
        && record.fence_token == guard.fence_token
        && lease_is_active(record, now)
}

fn webhook_event_lock_key(source_id: &str, dedupe_key: &str) -> i64 {
    let mut hasher = Sha256::new();
    hasher.update(source_id.as_bytes());
    hasher.update([0]);
    hasher.update(dedupe_key.as_bytes());
    let digest = hasher.finalize();
    let mut bytes = [0u8; 8];
    bytes.copy_from_slice(&digest[..8]);
    i64::from_be_bytes(bytes) & i64::MAX
}

fn parse_kv_json_value<T>(key: &str, raw: &[u8]) -> Result<Option<T>>
where
    T: serde::de::DeserializeOwned,
{
    if raw.is_empty() {
        return Ok(None);
    }
    Ok(Some(serde_json::from_slice(raw).with_context(|| {
        format!("Failed to parse kv_store JSON payload for key '{key}'")
    })?))
}

fn parse_strategy_candidate_profile(
    candidate: &learning_candidate::Model,
) -> Result<crate::core::self_evolve::strategy_runtime::ToolStrategyProfile> {
    serde_json::from_value(candidate.proposed_content.clone()).with_context(|| {
        format!(
            "Failed to parse strategy candidate payload for '{}'",
            candidate.id
        )
    })
}

fn is_foreign_key_constraint_error(error: &sea_orm::DbErr) -> bool {
    let text = error.to_string().to_ascii_lowercase();
    text.contains("foreign key constraint failed")
        || text.contains("violates foreign key constraint")
        || text.contains("sqlstate(23503)")
}

fn decrypt_swarm_delegation_model(model: &mut swarm_delegation::Model) {
    model.task_description = decrypt_storage_string(&model.task_description);
    model.result = decrypt_optional_storage_string(model.result.clone());
}

fn decrypt_memory_operation_model(model: &mut memory_operation::Model) {
    model.value = decrypt_optional_storage_string(model.value.clone());
    model.sensitive_reason = decrypt_optional_storage_string(model.sensitive_reason.clone());
    model.rationale = decrypt_optional_storage_string(model.rationale.clone());
    model.review_notes = decrypt_optional_storage_string(model.review_notes.clone());
}

fn encrypt_storage_bytes(value: &[u8]) -> Result<Vec<u8>> {
    if value.is_empty() {
        return Ok(Vec::new());
    }
    if let Some(key_manager) = current_storage_key_manager() {
        key_manager.encrypt(value)
    } else {
        Ok(value.to_vec())
    }
}

fn decrypt_storage_bytes(value: &[u8]) -> Vec<u8> {
    if value.is_empty() {
        return Vec::new();
    }
    if let Some(key_manager) = current_storage_key_manager() {
        key_manager
            .decrypt(value)
            .unwrap_or_else(|_| value.to_vec())
    } else {
        value.to_vec()
    }
}

fn parse_execution_run_status(raw: &str) -> crate::core::ExecutionRunStatus {
    serde_json::from_str(&format!("\"{raw}\""))
        .unwrap_or(crate::core::ExecutionRunStatus::PlatformFailed)
}

fn parse_tool_outcome_status(raw: &str) -> crate::core::ToolOutcomeStatus {
    serde_json::from_str(&format!("\"{raw}\""))
        .unwrap_or(crate::core::ToolOutcomeStatus::FatalError)
}

fn parse_failure_class(raw: Option<String>) -> Option<crate::core::FailureClass> {
    raw.and_then(|value| serde_json::from_str(&format!("\"{value}\"")).ok())
}

fn model_to_execution_run(model: execution_run::Model) -> crate::core::ExecutionRun {
    let attempted_models = decrypt_storage_string(&model.attempted_models);
    let degradation = decrypt_storage_string(&model.degradation);
    crate::core::ExecutionRun {
        id: model.id,
        kind: model.kind,
        request_id: model.request_id,
        status: parse_execution_run_status(&model.status),
        current_stage: model.current_stage,
        lease_owner: model.lease_owner,
        lease_expires_at: model.lease_expires_at,
        attempt: model.attempt.max(0) as u32,
        deadline_at: model.deadline_at,
        cancellation_requested: model.cancellation_requested,
        degradation: serde_json::from_str(&degradation).unwrap_or_default(),
        last_error: decrypt_optional_storage_string(model.last_error),
        result_summary: decrypt_optional_storage_string(model.result_summary),
        trace_id: model.trace_id,
        conversation_id: model.conversation_id,
        channel: model.channel,
        request_message: decrypt_optional_storage_string(model.request_message),
        attempted_models: serde_json::from_str(&attempted_models).unwrap_or_default(),
        created_at: model.created_at,
        updated_at: model.updated_at,
    }
}

fn model_to_tool_attempt(model: tool_attempt::Model) -> crate::core::ToolAttempt {
    crate::core::ToolAttempt {
        id: model.id,
        run_id: model.run_id,
        sequence_no: model.sequence_no.max(0) as u32,
        tool_name: model.tool_name,
        status: parse_tool_outcome_status(&model.status),
        failure_class: parse_failure_class(model.failure_class),
        retryable: model.retryable,
        side_effect_level: model.side_effect_level,
        idempotency_key: model.idempotency_key,
        arguments_json: decrypt_storage_string(&model.arguments_json),
        output_json: decrypt_storage_string(&model.output_json),
        started_at: model.started_at,
        completed_at: model.completed_at,
        error_text: decrypt_optional_storage_string(model.error_text),
    }
}

fn scope_match_rank(
    record_project_id: Option<&str>,
    record_conversation_id: Option<&str>,
    project_id: Option<&str>,
    conversation_id: Option<&str>,
) -> i32 {
    if conversation_id.is_some() && record_conversation_id == conversation_id {
        3
    } else if project_id.is_some() && record_project_id == project_id {
        2
    } else if record_project_id.is_none() && record_conversation_id.is_none() {
        1
    } else {
        0
    }
}

fn experience_item_kind_rank(kind: &str) -> i32 {
    match kind {
        "constraint" => 0,
        "personal_fact" => 1,
        "lesson" => 2,
        "procedure" => 3,
        _ => 4,
    }
}

fn learned_fact_topics_from_metadata(metadata: &serde_json::Value) -> Vec<String> {
    crate::core::memory_schema::normalize_memory_topics(metadata.get("topics"), 8)
}

fn learned_fact_kind_from_metadata(metadata: &serde_json::Value) -> Option<String> {
    metadata
        .get("memory_kind")
        .and_then(|value| value.as_str())
        .map(str::trim)
        .filter(|value| !value.is_empty())
        .map(str::to_string)
}

fn learned_fact_category_from_metadata(metadata: &serde_json::Value) -> String {
    let semantic_kind = learned_fact_kind_from_metadata(metadata);
    crate::core::memory_schema::memory_category_from_metadata(metadata, semantic_kind.as_deref())
        .to_string()
}

fn learned_fact_key_from_metadata(metadata: &serde_json::Value) -> Option<String> {
    metadata
        .get("key")
        .and_then(|value| value.as_str())
        .map(str::trim)
        .filter(|value| !value.is_empty())
        .map(str::to_string)
}

pub(super) fn learned_fact_value_from_content(key: Option<&str>, content: &str) -> String {
    let trimmed = content.trim();
    if let Some(key) = key.map(str::trim).filter(|value| !value.is_empty()) {
        let prefix = format!("{key}:");
        if let Some(value) = trimmed.strip_prefix(&prefix) {
            return value.trim().to_string();
        }
        return trimmed.to_string();
    }
    if let Some((candidate_key, value)) = trimmed.split_once(':') {
        let candidate_key = candidate_key.trim();
        if !candidate_key.is_empty() && !candidate_key.chars().any(char::is_whitespace) {
            return value.trim().to_string();
        }
    }
    trimmed.to_string()
}

fn learned_fact_from_experience_item(item: experience_item::Model) -> LearnedFactRecord {
    let sources = item
        .metadata
        .get("sources")
        .map(|value| {
            value
                .as_str()
                .map(str::to_string)
                .unwrap_or_else(|| value.to_string())
        })
        .unwrap_or_else(|| "[]".to_string());
    let memory_kind = learned_fact_kind_from_metadata(&item.metadata);
    let memory_category = crate::core::memory_schema::memory_category_from_metadata(
        &item.metadata,
        memory_kind.as_deref(),
    )
    .to_string();
    let topics = learned_fact_topics_from_metadata(&item.metadata);
    let key = learned_fact_key_from_metadata(&item.metadata);
    let value = learned_fact_value_from_content(key.as_deref(), &item.content);
    let (key, value) = match key {
        Some(raw_key) => {
            let allow_value_suffix_repair =
                memory_category == crate::core::memory_schema::MEMORY_CATEGORY_PROFILE_FACT;
            match crate::core::memory_schema::repair_memory_slot_key_and_value(
                &raw_key,
                &value,
                allow_value_suffix_repair,
            ) {
                Some((key, repaired_value)) => (Some(key), repaired_value.unwrap_or(value)),
                None => (Some(raw_key), value),
            }
        }
        None => (None, value),
    };
    LearnedFactRecord {
        id: item.id,
        fact: item.content,
        key,
        value,
        confidence: item.confidence.clamp(0.0, 1.0) as f32,
        sources,
        created_at: item.created_at,
        updated_at: item.updated_at,
        project_id: item.project_id,
        scope: item.scope,
        memory_kind,
        memory_category,
        topics,
    }
}

#[cfg(test)]
mod learned_fact_record_tests {
    use super::*;

    fn learned_fact_test_item(
        content: &str,
        metadata: serde_json::Value,
    ) -> experience_item::Model {
        let now = "2026-05-22T00:00:00Z".to_string();
        experience_item::Model {
            id: "memory-1".to_string(),
            kind: "personal_fact".to_string(),
            scope: "global".to_string(),
            project_id: None,
            conversation_id: Some("conversation-1".to_string()),
            title: "Learned user memory".to_string(),
            content: content.to_string(),
            normalized_key: "user_memory::user_name_alex::permanent".to_string(),
            confidence: 0.95,
            support_count: 1,
            contradiction_count: 0,
            status: "active".to_string(),
            metadata,
            last_supported_at: Some(now.clone()),
            last_contradicted_at: None,
            created_at: now.clone(),
            updated_at: now,
            embedding: None,
        }
    }

    #[test]
    fn learned_fact_value_uses_structured_key_as_schema_prefix() {
        assert_eq!(
            learned_fact_value_from_content(Some("user_first_name"), "user_first_name: Alex"),
            "Alex"
        );
    }

    #[test]
    fn learned_fact_value_does_not_strip_unrelated_prefix_when_key_is_known() {
        assert_eq!(
            learned_fact_value_from_content(Some("user_first_name"), "display_name: Alex"),
            "display_name: Alex"
        );
    }

    #[test]
    fn learned_fact_record_repairs_existing_key_that_contains_value() {
        let record = learned_fact_from_experience_item(learned_fact_test_item(
            "user_name_alex: The user's name is Alex.",
            serde_json::json!({
                "key": "user_name_alex",
                "memory_kind": "identity",
                "memory_category": "profile_fact",
                "topics": ["identity"],
            }),
        ));

        assert_eq!(record.key.as_deref(), Some("user_name"));
        assert_eq!(record.value, "Alex");
    }

    #[cfg_attr(
        not(feature = "db-tests"),
        ignore = "requires explicit isolated Postgres test database"
    )]
    #[tokio::test]
    async fn webhook_event_insert_once_rejects_duplicate_idempotency_key() {
        let storage = Storage::connect(
            DatabaseConfig::for_tests().expect("test database config should initialize"),
        )
        .await
        .expect("test database should connect");
        let source_id = format!("source-{}", uuid::Uuid::new_v4());
        let idempotency_key = format!("delivery-{}", uuid::Uuid::new_v4());
        let now = chrono::Utc::now().to_rfc3339();
        let first = webhook_event::Model {
            id: uuid::Uuid::new_v4().to_string(),
            source_id: source_id.clone(),
            source_name: "Build Alerts".to_string(),
            provider: "generic".to_string(),
            received_at: now.clone(),
            updated_at: now.clone(),
            event_type: "workflow".to_string(),
            status: Some("failed".to_string()),
            subject: "core-api".to_string(),
            outcome: "received".to_string(),
            matched: false,
            queued: false,
            message: None,
            event_id: Some("event-1".to_string()),
            dedupe_key: "display-dedupe".to_string(),
            idempotency_key: idempotency_key.clone(),
            payload_hash: "payload-hash".to_string(),
            event_url: None,
            payload_excerpt: Some("{}".to_string()),
            task_id: None,
            conversation_id: Some("conversation-1".to_string()),
            severity: Some("failed".to_string()),
            test_event: false,
        };
        let mut second = first.clone();
        second.id = uuid::Uuid::new_v4().to_string();
        second.received_at = chrono::Utc::now().to_rfc3339();

        assert!(storage.insert_webhook_event_once(first).await.unwrap());
        assert!(!storage.insert_webhook_event_once(second).await.unwrap());

        let events = storage
            .list_webhook_events(Some(&source_id), 10)
            .await
            .unwrap();
        assert_eq!(events.len(), 1);
        assert_eq!(events[0].idempotency_key, idempotency_key);
    }
}

#[cfg(test)]
mod storage_jsonb_churn_tests {
    use super::*;

    fn assert_not_upserted<C>(columns: &[C], column: C)
    where
        C: Copy + std::fmt::Debug,
    {
        let needle = format!("{column:?}");
        assert!(
            !columns
                .iter()
                .any(|candidate| format!("{candidate:?}") == needle),
            "heavy column should not be part of the conflict update list: {column:?}"
        );
    }

    fn sample_experience_run() -> experience_run::Model {
        experience_run::Model {
            id: "run-1".to_string(),
            execution_run_id: Some("execution-1".to_string()),
            trace_id: Some("trace-1".to_string()),
            conversation_id: Some("conversation-1".to_string()),
            project_id: Some("project-1".to_string()),
            channel: "chat".to_string(),
            scope: "project".to_string(),
            intent_key: "intent".to_string(),
            task_type: Some("task".to_string()),
            request_text: Some("large request".to_string()),
            tool_sequence_digest: Some("digest".to_string()),
            tool_sequence_json: serde_json::json!({"tools": ["search"]}),
            strategy_version: Some("strategy-v1".to_string()),
            policy_version: Some("policy-v1".to_string()),
            prompt_version: Some("prompt-v1".to_string()),
            model_slot: Some("primary".to_string()),
            success_state: "succeeded".to_string(),
            correction_state: "none".to_string(),
            outcome_summary: Some("large outcome".to_string()),
            failure_reason: None,
            metadata: serde_json::json!({"k": "v"}),
            consolidated: false,
            accepted_at: None,
            corrected_at: None,
            heuristic_reflected: false,
            heuristic_reflection_status: None,
            heuristic_reflection_attempted_at: None,
            heuristic_reflection_completed_at: None,
            heuristic_lesson_id: None,
            heuristic_reflection_error: None,
            created_at: "2026-05-28T00:00:00Z".to_string(),
            updated_at: "2026-05-28T00:00:01Z".to_string(),
        }
    }

    fn sample_experience_item() -> experience_item::Model {
        experience_item::Model {
            id: "item-1".to_string(),
            kind: "fact".to_string(),
            scope: "project".to_string(),
            project_id: Some("project-1".to_string()),
            conversation_id: Some("conversation-1".to_string()),
            title: "Fact".to_string(),
            content: "large content".to_string(),
            normalized_key: "fact::key".to_string(),
            confidence: 0.9,
            support_count: 1,
            contradiction_count: 0,
            status: "active".to_string(),
            metadata: serde_json::json!({"source": "test"}),
            last_supported_at: Some("2026-05-28T00:00:00Z".to_string()),
            last_contradicted_at: None,
            created_at: "2026-05-28T00:00:00Z".to_string(),
            updated_at: "2026-05-28T00:00:01Z".to_string(),
            embedding: Some(PgVector::from(vec![0.1_f32, 0.2, 0.3])),
        }
    }

    fn sample_memory_capture_event() -> memory_capture_event::Model {
        memory_capture_event::Model {
            id: "capture-1".to_string(),
            source_message_id: Some("message-1".to_string()),
            conversation_id: Some("conversation-1".to_string()),
            project_id: Some("project-1".to_string()),
            channel: "chat".to_string(),
            status: "completed".to_string(),
            capture_kind: "message".to_string(),
            source_hash: "hash".to_string(),
            attempt_metadata: serde_json::json!({"attempts": [{"model": "primary"}]}),
            error_history: serde_json::json!([]),
            replay_count: 0,
            next_retry_at: None,
            completed_at: Some("2026-05-28T00:00:02Z".to_string()),
            created_at: "2026-05-28T00:00:00Z".to_string(),
            updated_at: "2026-05-28T00:00:01Z".to_string(),
        }
    }

    fn sample_memory_operation() -> memory_operation::Model {
        memory_operation::Model {
            id: "operation-1".to_string(),
            capture_event_id: Some("capture-1".to_string()),
            operation_type: "upsert".to_string(),
            status: "applied".to_string(),
            target_memory_id: Some("memory-1".to_string()),
            applied_memory_id: Some("memory-1".to_string()),
            key: Some("favorite_color".to_string()),
            value: Some("large value".to_string()),
            memory_kind: "fact".to_string(),
            durability: "long_term".to_string(),
            scope: "project".to_string(),
            project_id: Some("project-1".to_string()),
            conversation_id: Some("conversation-1".to_string()),
            confidence: 0.95,
            looks_sensitive: false,
            sensitive_reason: None,
            valid_from: None,
            expires_at: None,
            review_at: None,
            rationale: Some("large rationale".to_string()),
            evidence_refs: serde_json::json!([{"kind": "message", "id": "message-1"}]),
            model_metadata: serde_json::json!({"model": "primary"}),
            apply_metadata: serde_json::json!({"applied": true}),
            applied_at: Some("2026-05-28T00:00:02Z".to_string()),
            reviewed_at: None,
            review_notes: None,
            created_at: "2026-05-28T00:00:00Z".to_string(),
            updated_at: "2026-05-28T00:00:01Z".to_string(),
        }
    }

    fn sample_semantic_work_unit() -> semantic_work_unit::Model {
        semantic_work_unit::Model {
            id: "unit-1".to_string(),
            source_kind: "conversation".to_string(),
            source_id: "conversation-1".to_string(),
            conversation_id: Some("conversation-1".to_string()),
            project_id: Some("project-1".to_string()),
            channel: "chat".to_string(),
            title: "Daily recap".to_string(),
            summary: "large summary".to_string(),
            content_preview: "large preview".to_string(),
            text_hash: "hash".to_string(),
            occurred_at: "2026-05-28T00:00:00Z".to_string(),
            period_start: Some("2026-05-28T00:00:00Z".to_string()),
            period_end: Some("2026-05-28T01:00:00Z".to_string()),
            message_count: 5,
            metadata: serde_json::json!({"topics": ["ops"]}),
            created_at: "2026-05-28T00:00:00Z".to_string(),
            updated_at: "2026-05-28T00:00:01Z".to_string(),
            embedding: Some(PgVector::from(vec![0.4_f32, 0.5, 0.6])),
        }
    }

    #[test]
    fn conflict_updates_exclude_heavy_storage_columns() {
        assert_not_upserted(
            EXPERIENCE_RUN_LIGHT_UPSERT_COLUMNS,
            experience_run::Column::ToolSequenceJson,
        );
        assert_not_upserted(
            EXPERIENCE_RUN_LIGHT_UPSERT_COLUMNS,
            experience_run::Column::Metadata,
        );
        assert_not_upserted(
            EXPERIENCE_ITEM_LIGHT_UPSERT_COLUMNS,
            experience_item::Column::Metadata,
        );
        assert_not_upserted(
            EXPERIENCE_ITEM_LIGHT_UPSERT_COLUMNS,
            experience_item::Column::Embedding,
        );
        assert_not_upserted(
            MEMORY_CAPTURE_EVENT_LIGHT_UPSERT_COLUMNS,
            memory_capture_event::Column::AttemptMetadata,
        );
        assert_not_upserted(
            MEMORY_OPERATION_LIGHT_UPSERT_COLUMNS,
            memory_operation::Column::ModelMetadata,
        );
        assert_not_upserted(
            SEMANTIC_WORK_UNIT_LIGHT_UPSERT_COLUMNS,
            semantic_work_unit::Column::Embedding,
        );
    }

    #[test]
    fn unchanged_heavy_columns_are_not_set_for_selective_updates() {
        let run = sample_experience_run();
        assert!(Storage::experience_run_heavy_update_active_model(&run, &run).is_none());

        let item = sample_experience_item();
        assert!(Storage::experience_item_heavy_update_active_model(&item, &item).is_none());

        let capture = sample_memory_capture_event();
        assert!(
            Storage::memory_capture_event_heavy_update_active_model(&capture, &capture).is_none()
        );

        let operation = sample_memory_operation();
        assert!(
            Storage::memory_operation_heavy_update_active_model(&operation, &operation)
                .expect("operation heavy update should build")
                .is_none()
        );

        let unit = sample_semantic_work_unit();
        assert!(
            Storage::semantic_work_unit_heavy_update_active_model(&unit, &unit)
                .expect("semantic work unit heavy update should build")
                .is_none()
        );
    }

    #[test]
    fn selective_updates_set_only_changed_heavy_columns() {
        let existing_item = sample_experience_item();
        let mut next_item = existing_item.clone();
        next_item.embedding = Some(PgVector::from(vec![0.9_f32, 0.8, 0.7]));

        let item_update =
            Storage::experience_item_heavy_update_active_model(&existing_item, &next_item)
                .expect("changed embedding should create update model");

        assert!(item_update.content.is_not_set());
        assert!(item_update.metadata.is_not_set());
        assert!(item_update.embedding.is_set());

        let existing_operation = sample_memory_operation();
        let mut next_operation = existing_operation.clone();
        next_operation.model_metadata = serde_json::json!({"model": "secondary"});

        let operation_update = Storage::memory_operation_heavy_update_active_model(
            &existing_operation,
            &next_operation,
        )
        .expect("operation heavy update should build")
        .expect("changed model metadata should create update model");

        assert!(operation_update.value.is_not_set());
        assert!(operation_update.evidence_refs.is_not_set());
        assert!(operation_update.model_metadata.is_set());
        assert!(operation_update.apply_metadata.is_not_set());
    }
}

fn procedural_pattern_status_rank(status: &str) -> i32 {
    match status {
        "active" => 2,
        "draft" => 1,
        _ => 0,
    }
}

fn normalized_search_terms(query: &str) -> Vec<String> {
    query
        .split_whitespace()
        .map(|term| term.trim().to_ascii_lowercase())
        .filter(|term| !term.is_empty())
        .collect()
}

fn matches_search_terms(terms: &[String], fields: &[&str]) -> bool {
    if terms.is_empty() {
        return false;
    }
    let haystack = fields.join(" ").to_ascii_lowercase();
    terms.iter().all(|term| haystack.contains(term))
}

fn search_score(terms: &[String], weighted_fields: &[(&str, f64)]) -> f64 {
    let mut score = 0.0;
    for (field, weight) in weighted_fields {
        let lower = field.to_ascii_lowercase();
        for term in terms {
            let occurrences = lower.matches(term).count() as f64;
            if occurrences > 0.0 {
                score += occurrences * *weight;
            }
        }
    }
    score
}

#[derive(Debug, Clone)]
pub struct NewUserDataItem<'a> {
    pub kind: &'a str,
    pub title: &'a str,
    pub content: &'a str,
    pub url: Option<&'a str>,
    pub source_channel: Option<&'a str>,
    pub conversation_id: Option<&'a str>,
    pub project_id: Option<&'a str>,
    pub pinned: bool,
}

#[derive(Debug, Clone)]
pub struct ExperienceItemSearchHit {
    pub item: experience_item::Model,
    pub score: f64,
}

#[allow(dead_code)]
#[derive(Debug, Clone)]
pub struct ProceduralPatternSearchHit {
    pub pattern: procedural_pattern::Model,
    pub score: f64,
}

#[derive(Debug, Clone, Default, serde::Serialize)]
pub struct LearningQueueCounts {
    pub provisional_runs: u64,
    pub pending_consolidation: u64,
    pub pending_reflection: u64,
    pub draft_candidates: u64,
    pub active_patterns: u64,
}

#[derive(Debug, Clone, Default, serde::Serialize)]
pub struct LeaseStatusSummary {
    pub pending_task_backlog: u64,
    pub active_task_leases: u64,
    pub tasks_waiting_retry: u64,
    pub watcher_poll_backlog: u64,
    pub active_watcher_leases: u64,
    pub watchers_waiting_retry: u64,
    pub active_run_leases: u64,
    pub runs_pending_cancellation: u64,
}

#[derive(Debug, Clone, serde::Serialize, serde::Deserialize)]
pub struct ReadonlyTableFilter {
    pub column: String,
    pub op: String,
    #[serde(default)]
    pub value: Option<serde_json::Value>,
}

#[derive(Debug, Clone, serde::Serialize, serde::Deserialize)]
pub struct ReadonlyTableSort {
    pub column: String,
    #[serde(default)]
    pub direction: Option<String>,
}

#[derive(Debug, Clone, serde::Serialize, serde::Deserialize)]
pub struct ReadonlyTableQuery {
    pub table: String,
    #[serde(default)]
    pub columns: Vec<String>,
    #[serde(default)]
    pub filters: Vec<ReadonlyTableFilter>,
    #[serde(default)]
    pub order_by: Vec<ReadonlyTableSort>,
    #[serde(default)]
    pub limit: Option<u64>,
}

#[derive(Debug, Clone, FromQueryResult)]
struct DatabaseColumnSchemaRow {
    table_schema: String,
    table_name: String,
    column_name: String,
    data_type: String,
    udt_name: String,
    is_nullable: String,
    column_default: Option<String>,
    ordinal_position: i32,
}

impl Storage {
    const DATABASE_MAX_INTEGER: u64 = i64::MAX as u64;
    const HOUSEKEEPING_PURGE_LAST_RUN_KEY: &'static str = "storage_housekeeping_last_purge_v1";
    const UPLOAD_MANIFEST_KEY_PREFIX: &'static str = "upload_manifest:";
    const MAX_DOCUMENTS_FOR_SEARCH: u64 = 5_000;
    const MAX_DOCUMENT_CHUNKS_FOR_SEARCH: u64 = 20_000;
    const MAX_LLM_USAGE_ROWS_PER_QUERY: u64 = 5_000;
    const MAX_LLM_USAGE_ANALYTICS_ROWS: usize = 250_000;
    const MAX_FACT_ROWS_PER_QUERY: u64 = 5_000;
    const MAX_TASK_ROWS_PER_QUERY: u64 = 5_000;
    const MAX_EXPENSE_ROWS_PER_QUERY: u64 = 5_000;
    const MAX_SWARM_DELEGATION_ROWS_PER_QUERY: u64 = 5_000;
    const MAX_EXPERIENCE_RUN_ROWS_PER_QUERY: u64 = 1_000;
    const MAX_EXPERIENCE_ITEM_ROWS_PER_QUERY: u64 = 2_000;
    const MAX_PROCEDURAL_PATTERN_ROWS_PER_QUERY: u64 = 2_000;
    #[allow(dead_code)]
    const MAX_RELATED_EXPERIENCE_EDGE_ROWS_PER_QUERY: u64 = 5_000;
    const SENSITIVE_PAYLOAD_BACKFILL_MARKER_KEY: &'static str =
        "storage_sensitive_payload_backfill_v4";

    #[inline]
    fn db_limit(limit: u64) -> u64 {
        limit.min(Self::DATABASE_MAX_INTEGER)
    }

    #[inline]
    fn db_offset(offset: u64) -> u64 {
        offset.min(Self::DATABASE_MAX_INTEGER)
    }

    fn upload_manifest_key(id: &str) -> String {
        format!("{}{}", Self::UPLOAD_MANIFEST_KEY_PREFIX, id)
    }

    /// Access the underlying SeaORM connection.
    ///
    /// Exposed so security-layer modules (e.g. `security::abuse_tracker`)
    /// can read and update their own dedicated tables without having to
    /// duplicate CRUD plumbing inside `Storage`.
    pub fn db(&self) -> &DatabaseConnection {
        &self.db
    }

    pub async fn connection_activity_counts(&self) -> Result<DbConnectionActivityCounts> {
        let row = self
            .db
            .query_one(Statement::from_string(
                DbBackend::Postgres,
                "SELECT \
                    COUNT(*) FILTER (WHERE state = 'active') AS active, \
                    COUNT(*) FILTER (WHERE state = 'idle') AS idle, \
                    COUNT(*) AS total \
                 FROM pg_stat_activity \
                 WHERE application_name = 'agentark' \
                   AND datname = current_database()"
                    .to_string(),
            ))
            .await?;
        let Some(row) = row else {
            return Ok(DbConnectionActivityCounts::default());
        };
        Ok(DbConnectionActivityCounts {
            active: row.try_get::<i64>("", "active").unwrap_or_default(),
            idle: row.try_get::<i64>("", "idle").unwrap_or_default(),
            total: row.try_get::<i64>("", "total").unwrap_or_default(),
        })
    }

    fn preference_row_id(key: &str, project_id: Option<&str>) -> String {
        let normalized_key = key.trim().to_ascii_lowercase();
        let scope = project_id
            .map(|p| p.trim())
            .filter(|p| !p.is_empty())
            .unwrap_or("_global");
        format!("{}::{}", scope, normalized_key)
    }

    fn default_link_title(url: &str) -> String {
        if let Ok(parsed) = url::Url::parse(url) {
            if let Some(host) = parsed.host_str() {
                let path = parsed.path().trim_matches('/');
                if path.is_empty() {
                    return host.to_string();
                }
                let compact = path
                    .split('/')
                    .filter(|seg| !seg.is_empty())
                    .take(2)
                    .collect::<Vec<_>>()
                    .join(" / ");
                if compact.is_empty() {
                    host.to_string()
                } else {
                    format!("{} / {}", host, compact)
                }
            } else {
                "Saved link".to_string()
            }
        } else {
            "Saved link".to_string()
        }
    }

    /// Connect to the configured PostgreSQL database and run ordered migrations.
    pub async fn connect(config: DatabaseConfig) -> Result<Self> {
        config.validate()?;
        let target_summary = config.target_summary();
        let connect_timeout_secs = config.connect_timeout_secs.max(1);
        let retry_deadline =
            tokio::time::Instant::now() + Duration::from_secs(DB_CONNECT_RETRY_WINDOW_SECS);
        let mut retry_delay = Duration::from_millis(DB_CONNECT_INITIAL_RETRY_DELAY_MS);
        let db = loop {
            match Database::connect(config.connect_options()).await {
                Ok(db) => break db,
                Err(error) if tokio::time::Instant::now() < retry_deadline => {
                    tracing::warn!(
                        "Failed to connect to Postgres at {}; retrying in {:?}: {}",
                        target_summary,
                        retry_delay,
                        error
                    );
                    tokio::time::sleep(retry_delay).await;
                    retry_delay =
                        (retry_delay * 2).min(Duration::from_secs(DB_CONNECT_MAX_RETRY_DELAY_SECS));
                }
                Err(error) => {
                    return Err(error).with_context(|| {
                        format!(
                            "Failed to connect to Postgres at {} within {}s after {}s retry window",
                            target_summary, connect_timeout_secs, DB_CONNECT_RETRY_WINDOW_SECS
                        )
                    });
                }
            }
        };
        if db.get_database_backend() != DbBackend::Postgres {
            anyhow::bail!("Postgres storage requires the SeaORM Postgres backend");
        }
        migrations::run(&db).await?;
        Ok(Self { db })
    }

    pub fn spawn_housekeeping_purge_worker(
        &self,
        mut shutdown_rx: tokio::sync::watch::Receiver<bool>,
    ) -> tokio::task::JoinHandle<()> {
        let storage = self.clone();
        tokio::spawn(async move {
            loop {
                if *shutdown_rx.borrow() {
                    break;
                }
                let lifecycle =
                    crate::core::data_lifecycle::load_data_lifecycle_settings(&storage).await;
                let sleep = tokio::time::sleep(Duration::from_secs(
                    lifecycle.housekeeping_interval_secs.max(1),
                ));
                tokio::pin!(sleep);
                tokio::select! {
                    _ = &mut sleep => {
                        if let Err(error) = storage.run_housekeeping_purge().await {
                            tracing::warn!("Storage housekeeping purge failed: {}", error);
                        }
                    }
                    changed = shutdown_rx.changed() => {
                        if changed.is_err() || *shutdown_rx.borrow() {
                            break;
                        }
                    }
                }
            }
        })
    }

    // ==================== Key-Value Store ====================

    /// Get a value from the key-value store
    pub async fn get(&self, key: &str) -> Result<Option<Vec<u8>>> {
        let result = kv_store::Entity::find_by_id(key.to_string())
            .one(&self.db)
            .await?;

        Ok(result.map(|m| m.value))
    }

    /// Set a value in the key-value store
    pub async fn set(&self, key: &str, value: &[u8]) -> Result<()> {
        let now = chrono::Utc::now().to_rfc3339();
        kv_store::Entity::insert(kv_store::ActiveModel {
            key: Set(key.to_string()),
            value: Set(value.to_vec()),
            created_at: Set(now.clone()),
            updated_at: Set(now),
        })
        .on_conflict(
            OnConflict::column(kv_store::Column::Key)
                .update_columns([kv_store::Column::Value, kv_store::Column::UpdatedAt])
                .to_owned(),
        )
        .exec(&self.db)
        .await?;
        Ok(())
    }

    /// Delete a key from the store
    pub async fn delete(&self, key: &str) -> Result<()> {
        kv_store::Entity::delete_by_id(key.to_string())
            .exec(&self.db)
            .await?;
        Ok(())
    }

    async fn ensure_kv_row_exists_txn(&self, txn: &DatabaseTransaction, key: &str) -> Result<()> {
        let now = chrono::Utc::now().to_rfc3339();
        match kv_store::Entity::insert(kv_store::ActiveModel {
            key: Set(key.to_string()),
            value: Set(Vec::new()),
            created_at: Set(now.clone()),
            updated_at: Set(now),
        })
        .on_conflict(
            OnConflict::column(kv_store::Column::Key)
                .do_nothing()
                .to_owned(),
        )
        .exec(txn)
        .await
        {
            Ok(_) | Err(sea_orm::DbErr::RecordNotInserted) => {}
            Err(error) => return Err(error.into()),
        }
        Ok(())
    }

    async fn get_kv_for_update_txn(
        &self,
        txn: &DatabaseTransaction,
        key: &str,
    ) -> Result<Option<kv_store::Model>> {
        let sql = format!(
            "SELECT key, value, created_at, updated_at FROM kv_store WHERE key = {} FOR UPDATE",
            sql_string_literal(key)
        );
        let row = txn
            .query_one(Statement::from_string(DbBackend::Postgres, sql))
            .await?;
        let Some(row) = row else {
            return Ok(None);
        };
        Ok(Some(kv_store::Model {
            key: row.try_get("", "key")?,
            value: row.try_get("", "value")?,
            created_at: row.try_get("", "created_at")?,
            updated_at: row.try_get("", "updated_at")?,
        }))
    }

    async fn set_kv_txn(&self, txn: &DatabaseTransaction, key: &str, value: &[u8]) -> Result<()> {
        let now = chrono::Utc::now().to_rfc3339();
        kv_store::Entity::insert(kv_store::ActiveModel {
            key: Set(key.to_string()),
            value: Set(value.to_vec()),
            created_at: Set(now.clone()),
            updated_at: Set(now),
        })
        .on_conflict(
            OnConflict::column(kv_store::Column::Key)
                .update_columns([kv_store::Column::Value, kv_store::Column::UpdatedAt])
                .to_owned(),
        )
        .exec(txn)
        .await?;
        Ok(())
    }

    async fn delete_kv_txn(&self, txn: &DatabaseTransaction, key: &str) -> Result<()> {
        kv_store::Entity::delete_by_id(key.to_string())
            .exec(txn)
            .await?;
        Ok(())
    }

    async fn load_kv_json_txn<T>(&self, txn: &DatabaseTransaction, key: &str) -> Result<Option<T>>
    where
        T: serde::de::DeserializeOwned,
    {
        let row = self.get_kv_for_update_txn(txn, key).await?;
        match row {
            Some(row) => parse_kv_json_value(key, &row.value),
            None => Ok(None),
        }
    }

    async fn set_kv_json_txn<T>(
        &self,
        txn: &DatabaseTransaction,
        key: &str,
        value: &T,
    ) -> Result<()>
    where
        T: serde::Serialize,
    {
        let raw = serde_json::to_vec(value).with_context(|| {
            format!("Failed to serialize kv_store JSON payload for key '{key}'")
        })?;
        self.set_kv_txn(txn, key, &raw).await
    }

    async fn load_learning_candidate_txn(
        &self,
        txn: &DatabaseTransaction,
        id: &str,
    ) -> Result<Option<learning_candidate::Model>> {
        Ok(learning_candidate::Entity::find_by_id(id.to_string())
            .one(txn)
            .await?)
    }

    async fn update_learning_candidate_review_txn(
        &self,
        txn: &DatabaseTransaction,
        id: &str,
        approval_status: &str,
        review_notes: Option<&str>,
        approved_ref: Option<&str>,
    ) -> Result<()> {
        let now = chrono::Utc::now().to_rfc3339();
        learning_candidate::Entity::update_many()
            .col_expr(
                learning_candidate::Column::ApprovalStatus,
                Expr::value(approval_status.to_string()),
            )
            .col_expr(
                learning_candidate::Column::ReviewNotes,
                Expr::value(review_notes.map(|value| value.to_string())),
            )
            .col_expr(
                learning_candidate::Column::ReviewedAt,
                Expr::value(Some(now.clone())),
            )
            .col_expr(
                learning_candidate::Column::ApprovedRef,
                Expr::value(approved_ref.map(|value| value.to_string())),
            )
            .col_expr(learning_candidate::Column::UpdatedAt, Expr::value(now))
            .filter(learning_candidate::Column::Id.eq(id))
            .exec(txn)
            .await?;
        Ok(())
    }

    async fn require_kv_lease_guard_txn(
        &self,
        txn: &DatabaseTransaction,
        key: &str,
        guard: &KvLeaseGuard,
    ) -> Result<bool> {
        self.ensure_kv_row_exists_txn(txn, key).await?;
        let Some(row) = self.get_kv_for_update_txn(txn, key).await? else {
            return Ok(false);
        };
        let Some(record) = serde_json::from_slice::<KvLeaseRecord>(&row.value).ok() else {
            return Ok(false);
        };
        Ok(kv_lease_guard_is_current(
            &record,
            guard,
            chrono::Utc::now(),
        ))
    }

    async fn upsert_learning_candidate_txn(
        &self,
        txn: &DatabaseTransaction,
        candidate: &learning_candidate::Model,
    ) -> Result<()> {
        learning_candidate::Entity::insert(learning_candidate::ActiveModel {
            id: Set(candidate.id.clone()),
            candidate_type: Set(candidate.candidate_type.clone()),
            subject_key: Set(candidate.subject_key.clone()),
            title: Set(candidate.title.clone()),
            summary: Set(candidate.summary.clone()),
            project_id: Set(candidate.project_id.clone()),
            conversation_id: Set(candidate.conversation_id.clone()),
            pattern_id: Set(candidate.pattern_id.clone()),
            evidence_refs: Set(candidate.evidence_refs.clone()),
            proposed_content: Set(candidate.proposed_content.clone()),
            confidence: Set(candidate.confidence),
            approval_status: Set(candidate.approval_status.clone()),
            review_notes: Set(candidate.review_notes.clone()),
            reviewed_at: Set(candidate.reviewed_at.clone()),
            approved_ref: Set(candidate.approved_ref.clone()),
            created_at: Set(candidate.created_at.clone()),
            updated_at: Set(candidate.updated_at.clone()),
        })
        .on_conflict(
            OnConflict::column(learning_candidate::Column::Id)
                .update_columns([
                    learning_candidate::Column::CandidateType,
                    learning_candidate::Column::SubjectKey,
                    learning_candidate::Column::Title,
                    learning_candidate::Column::Summary,
                    learning_candidate::Column::ProjectId,
                    learning_candidate::Column::ConversationId,
                    learning_candidate::Column::PatternId,
                    learning_candidate::Column::EvidenceRefs,
                    learning_candidate::Column::ProposedContent,
                    learning_candidate::Column::Confidence,
                    learning_candidate::Column::UpdatedAt,
                ])
                .to_owned(),
        )
        .exec(txn)
        .await?;
        Ok(())
    }

    pub async fn acquire_kv_lease(&self, key: &str, owner_id: &str, ttl_secs: i64) -> Result<bool> {
        let ttl_secs = ttl_secs.max(1);
        let txn = self.db.begin().await?;
        self.ensure_kv_row_exists_txn(&txn, key).await?;
        let existing = self.get_kv_for_update_txn(&txn, key).await?;
        let now = chrono::Utc::now();
        let lease = existing
            .as_ref()
            .and_then(|row| serde_json::from_slice::<KvLeaseRecord>(&row.value).ok());
        if lease
            .as_ref()
            .is_some_and(|record| lease_is_active(record, now) && record.owner_id != owner_id)
        {
            txn.rollback().await?;
            return Ok(false);
        }

        let next = KvLeaseRecord {
            owner_id: owner_id.to_string(),
            acquired_at: now.to_rfc3339(),
            expires_at: (now + chrono::Duration::seconds(ttl_secs)).to_rfc3339(),
            fence_token: next_lease_fence_token(lease.as_ref()),
        };
        let raw = serde_json::to_vec(&next)?;
        self.set_kv_txn(&txn, key, &raw).await?;
        txn.commit().await?;
        Ok(true)
    }

    pub async fn release_kv_lease(&self, key: &str, owner_id: &str) -> Result<()> {
        let txn = self.db.begin().await?;
        self.ensure_kv_row_exists_txn(&txn, key).await?;
        let existing = self.get_kv_for_update_txn(&txn, key).await?;
        let lease = existing
            .as_ref()
            .and_then(|row| serde_json::from_slice::<KvLeaseRecord>(&row.value).ok());
        if lease
            .as_ref()
            .is_some_and(|record| record.owner_id == owner_id)
        {
            self.delete_kv_txn(&txn, key).await?;
        }
        txn.commit().await?;
        Ok(())
    }

    pub async fn acquire_kv_lease_guard(
        &self,
        key: &str,
        owner_id: &str,
        ttl_secs: i64,
    ) -> Result<Option<KvLeaseGuard>> {
        let ttl_secs = ttl_secs.max(1);
        let txn = self.db.begin().await?;
        self.ensure_kv_row_exists_txn(&txn, key).await?;
        let existing = self.get_kv_for_update_txn(&txn, key).await?;
        let now = chrono::Utc::now();
        let lease = existing
            .as_ref()
            .and_then(|row| serde_json::from_slice::<KvLeaseRecord>(&row.value).ok());
        if lease
            .as_ref()
            .is_some_and(|record| lease_is_active(record, now) && record.owner_id != owner_id)
        {
            txn.rollback().await?;
            return Ok(None);
        }

        let fence_token = next_lease_fence_token(lease.as_ref());
        let next = KvLeaseRecord {
            owner_id: owner_id.to_string(),
            acquired_at: now.to_rfc3339(),
            expires_at: (now + chrono::Duration::seconds(ttl_secs)).to_rfc3339(),
            fence_token,
        };
        let raw = serde_json::to_vec(&next)?;
        self.set_kv_txn(&txn, key, &raw).await?;
        txn.commit().await?;
        Ok(Some(KvLeaseGuard {
            owner_id: owner_id.to_string(),
            fence_token,
        }))
    }

    pub async fn refresh_kv_lease_guard(
        &self,
        key: &str,
        guard: &KvLeaseGuard,
        ttl_secs: i64,
    ) -> Result<bool> {
        let ttl_secs = ttl_secs.max(1);
        let txn = self.db.begin().await?;
        self.ensure_kv_row_exists_txn(&txn, key).await?;
        let existing = self.get_kv_for_update_txn(&txn, key).await?;
        let now = chrono::Utc::now();
        let Some(lease) = existing
            .as_ref()
            .and_then(|row| serde_json::from_slice::<KvLeaseRecord>(&row.value).ok())
        else {
            txn.rollback().await?;
            return Ok(false);
        };
        if !kv_lease_guard_is_current(&lease, guard, now) {
            txn.rollback().await?;
            return Ok(false);
        }
        let refreshed = KvLeaseRecord {
            owner_id: lease.owner_id,
            acquired_at: lease.acquired_at,
            expires_at: (now + chrono::Duration::seconds(ttl_secs)).to_rfc3339(),
            fence_token: lease.fence_token,
        };
        let raw = serde_json::to_vec(&refreshed)?;
        self.set_kv_txn(&txn, key, &raw).await?;
        txn.commit().await?;
        Ok(true)
    }

    pub async fn release_kv_lease_guard(&self, key: &str, guard: &KvLeaseGuard) -> Result<()> {
        let txn = self.db.begin().await?;
        self.ensure_kv_row_exists_txn(&txn, key).await?;
        let existing = self.get_kv_for_update_txn(&txn, key).await?;
        let lease = existing
            .as_ref()
            .and_then(|row| serde_json::from_slice::<KvLeaseRecord>(&row.value).ok());
        if lease.as_ref().is_some_and(|record| {
            record.owner_id == guard.owner_id && record.fence_token == guard.fence_token
        }) {
            self.delete_kv_txn(&txn, key).await?;
        }
        txn.commit().await?;
        Ok(())
    }

    pub async fn list_webhook_sources(&self) -> Result<Vec<webhook_source::Model>> {
        Ok(webhook_source::Entity::find()
            .order_by_desc(webhook_source::Column::UpdatedAt)
            .all(&self.db)
            .await?)
    }

    pub async fn get_webhook_source(&self, id: &str) -> Result<Option<webhook_source::Model>> {
        Ok(webhook_source::Entity::find_by_id(id.to_string())
            .one(&self.db)
            .await?)
    }

    pub async fn upsert_webhook_source(&self, source: webhook_source::Model) -> Result<()> {
        webhook_source::Entity::insert(Self::webhook_source_active_model(source))
            .on_conflict(
                OnConflict::column(webhook_source::Column::Id)
                    .update_columns([
                        webhook_source::Column::Name,
                        webhook_source::Column::Provider,
                        webhook_source::Column::Description,
                        webhook_source::Column::Enabled,
                        webhook_source::Column::AuthMode,
                        webhook_source::Column::MatchMode,
                        webhook_source::Column::Instruction,
                        webhook_source::Column::EventHeader,
                        webhook_source::Column::SecretHeader,
                        webhook_source::Column::SignatureTimestampHeader,
                        webhook_source::Column::SignatureTimestampToleranceSecs,
                        webhook_source::Column::SignaturePayloadMode,
                        webhook_source::Column::AllowDuplicate,
                        webhook_source::Column::RequireApproval,
                        webhook_source::Column::DedupeWindowSecs,
                        webhook_source::Column::NotifyOnQueued,
                        webhook_source::Column::NotifyOnSuccess,
                        webhook_source::Column::NotifyOnFailure,
                        webhook_source::Column::OutputTarget,
                        webhook_source::Column::OutputChannel,
                        webhook_source::Column::ConversationId,
                        webhook_source::Column::UpdatedAt,
                        webhook_source::Column::LastReceivedAt,
                        webhook_source::Column::LastOutcome,
                        webhook_source::Column::LastTaskId,
                    ])
                    .to_owned(),
            )
            .exec(&self.db)
            .await?;
        Ok(())
    }

    pub async fn delete_webhook_source(&self, id: &str) -> Result<bool> {
        let result = webhook_source::Entity::delete_by_id(id.to_string())
            .exec(&self.db)
            .await?;
        Ok(result.rows_affected > 0)
    }

    pub async fn update_webhook_source_runtime_state(
        &self,
        id: &str,
        received_at: &str,
        outcome: &str,
        task_id: Option<&str>,
    ) -> Result<bool> {
        let result = webhook_source::Entity::update_many()
            .col_expr(
                webhook_source::Column::UpdatedAt,
                Expr::value(received_at.to_string()),
            )
            .col_expr(
                webhook_source::Column::LastReceivedAt,
                Expr::value(Some(received_at.to_string())),
            )
            .col_expr(
                webhook_source::Column::LastOutcome,
                Expr::value(Some(outcome.to_string())),
            )
            .col_expr(
                webhook_source::Column::LastTaskId,
                Expr::value(task_id.map(|value| value.to_string())),
            )
            .filter(webhook_source::Column::Id.eq(id.to_string()))
            .exec(&self.db)
            .await?;
        Ok(result.rows_affected > 0)
    }

    #[cfg(test)]
    pub async fn insert_webhook_event_once(&self, event: webhook_event::Model) -> Result<bool> {
        let result = webhook_event::Entity::insert(Self::webhook_event_active_model(event))
            .on_conflict(
                OnConflict::columns([
                    webhook_event::Column::SourceId,
                    webhook_event::Column::IdempotencyKey,
                ])
                .do_nothing()
                .to_owned(),
            )
            .exec(&self.db)
            .await;
        match result {
            Ok(_) => Ok(true),
            Err(sea_orm::DbErr::RecordNotInserted) => Ok(false),
            Err(error) => Err(error.into()),
        }
    }

    pub async fn reserve_webhook_event_once(
        &self,
        event: webhook_event::Model,
        dedupe_window_secs: u64,
    ) -> Result<bool> {
        let txn = self.db.begin().await?;
        txn.execute(Statement::from_sql_and_values(
            DbBackend::Postgres,
            "SELECT pg_advisory_xact_lock($1)",
            vec![webhook_event_lock_key(&event.source_id, &event.dedupe_key).into()],
        ))
        .await?;
        let cutoff = (chrono::Utc::now()
            - chrono::Duration::seconds(dedupe_window_secs.max(1) as i64))
        .to_rfc3339();
        let duplicate = webhook_event::Entity::find()
            .filter(webhook_event::Column::SourceId.eq(event.source_id.clone()))
            .filter(webhook_event::Column::DedupeKey.eq(event.dedupe_key.clone()))
            .filter(webhook_event::Column::ReceivedAt.gte(cutoff))
            .one(&txn)
            .await?
            .is_some();
        if duplicate {
            txn.commit().await?;
            return Ok(false);
        }
        let insert_result = webhook_event::Entity::insert(Self::webhook_event_active_model(event))
            .on_conflict(
                OnConflict::columns([
                    webhook_event::Column::SourceId,
                    webhook_event::Column::IdempotencyKey,
                ])
                .do_nothing()
                .to_owned(),
            )
            .exec(&txn)
            .await;
        match insert_result {
            Ok(_) => {
                txn.commit().await?;
                Ok(true)
            }
            Err(sea_orm::DbErr::RecordNotInserted) => {
                txn.commit().await?;
                Ok(false)
            }
            Err(error) => {
                let _ = txn.rollback().await;
                Err(error.into())
            }
        }
    }

    pub async fn insert_webhook_event(&self, event: webhook_event::Model) -> Result<()> {
        webhook_event::Entity::insert(Self::webhook_event_active_model(event))
            .exec(&self.db)
            .await?;
        Ok(())
    }

    pub async fn update_webhook_event_outcome(
        &self,
        id: &str,
        outcome: &str,
        matched: bool,
        queued: bool,
        task_id: Option<&str>,
        message: Option<&str>,
    ) -> Result<()> {
        webhook_event::Entity::update_many()
            .col_expr(
                webhook_event::Column::UpdatedAt,
                Expr::value(chrono::Utc::now().to_rfc3339()),
            )
            .col_expr(
                webhook_event::Column::Outcome,
                Expr::value(outcome.to_string()),
            )
            .col_expr(webhook_event::Column::Matched, Expr::value(matched))
            .col_expr(webhook_event::Column::Queued, Expr::value(queued))
            .col_expr(
                webhook_event::Column::TaskId,
                Expr::value(task_id.map(|value| value.to_string())),
            )
            .col_expr(
                webhook_event::Column::Message,
                Expr::value(message.map(|value| value.to_string())),
            )
            .filter(webhook_event::Column::Id.eq(id))
            .exec(&self.db)
            .await?;
        Ok(())
    }

    pub async fn list_webhook_events(
        &self,
        source_id: Option<&str>,
        limit: u64,
    ) -> Result<Vec<webhook_event::Model>> {
        let mut query = webhook_event::Entity::find()
            .order_by_desc(webhook_event::Column::ReceivedAt)
            .limit(Self::db_limit(limit));
        if let Some(source_id) = source_id.map(str::trim).filter(|value| !value.is_empty()) {
            query = query.filter(webhook_event::Column::SourceId.eq(source_id.to_string()));
        }
        Ok(query.all(&self.db).await?)
    }

    fn webhook_source_active_model(source: webhook_source::Model) -> webhook_source::ActiveModel {
        webhook_source::ActiveModel {
            id: Set(source.id),
            name: Set(source.name),
            provider: Set(source.provider),
            description: Set(source.description),
            enabled: Set(source.enabled),
            auth_mode: Set(source.auth_mode),
            match_mode: Set(source.match_mode),
            instruction: Set(source.instruction),
            event_header: Set(source.event_header),
            secret_header: Set(source.secret_header),
            signature_timestamp_header: Set(source.signature_timestamp_header),
            signature_timestamp_tolerance_secs: Set(source.signature_timestamp_tolerance_secs),
            signature_payload_mode: Set(source.signature_payload_mode),
            allow_duplicate: Set(source.allow_duplicate),
            require_approval: Set(source.require_approval),
            dedupe_window_secs: Set(source.dedupe_window_secs),
            notify_on_queued: Set(source.notify_on_queued),
            notify_on_success: Set(source.notify_on_success),
            notify_on_failure: Set(source.notify_on_failure),
            output_target: Set(source.output_target),
            output_channel: Set(source.output_channel),
            conversation_id: Set(source.conversation_id),
            created_at: Set(source.created_at),
            updated_at: Set(source.updated_at),
            last_received_at: Set(source.last_received_at),
            last_outcome: Set(source.last_outcome),
            last_task_id: Set(source.last_task_id),
        }
    }

    fn webhook_event_active_model(event: webhook_event::Model) -> webhook_event::ActiveModel {
        webhook_event::ActiveModel {
            id: Set(event.id),
            source_id: Set(event.source_id),
            source_name: Set(event.source_name),
            provider: Set(event.provider),
            received_at: Set(event.received_at),
            updated_at: Set(event.updated_at),
            event_type: Set(event.event_type),
            status: Set(event.status),
            subject: Set(event.subject),
            outcome: Set(event.outcome),
            matched: Set(event.matched),
            queued: Set(event.queued),
            message: Set(event.message),
            event_id: Set(event.event_id),
            dedupe_key: Set(event.dedupe_key),
            idempotency_key: Set(event.idempotency_key),
            payload_hash: Set(event.payload_hash),
            event_url: Set(event.event_url),
            payload_excerpt: Set(event.payload_excerpt),
            task_id: Set(event.task_id),
            conversation_id: Set(event.conversation_id),
            severity: Set(event.severity),
            test_event: Set(event.test_event),
        }
    }

    pub async fn get_encrypted(&self, key: &str) -> Result<Option<Vec<u8>>> {
        Ok(self
            .get(key)
            .await?
            .map(|value| decrypt_storage_bytes(&value)))
    }

    pub async fn set_encrypted(&self, key: &str, value: &[u8]) -> Result<()> {
        let encrypted = encrypt_storage_bytes(value)?;
        self.set(key, &encrypted).await
    }

    pub async fn save_upload_manifest(&self, manifest: &UploadManifest) -> Result<()> {
        let encoded = serde_json::to_vec(manifest)?;
        self.set_encrypted(&Self::upload_manifest_key(&manifest.id), &encoded)
            .await
    }

    pub async fn load_upload_manifest(&self, id: &str) -> Result<Option<UploadManifest>> {
        let Some(raw) = self.get_encrypted(&Self::upload_manifest_key(id)).await? else {
            return Ok(None);
        };
        Ok(Some(serde_json::from_slice::<UploadManifest>(&raw)?))
    }

    pub async fn reencrypt_sensitive_payloads(
        &self,
        old_key: &KeyManager,
        new_key: &KeyManager,
        encrypted_kv_keys: &[&str],
        lineage_record: Option<(String, Vec<u8>)>,
    ) -> Result<()> {
        let txn = self.db.begin().await?;

        let messages = message::Entity::find().all(&txn).await?;
        for row in messages {
            let plaintext = old_key
                .decrypt_string(&row.content)
                .unwrap_or_else(|_| row.content.clone());
            let encrypted = new_key.encrypt_string(&plaintext)?;
            let tool_calls_json = row
                .tool_calls_json
                .as_deref()
                .map(|value| {
                    let plaintext = old_key
                        .decrypt_string(value)
                        .unwrap_or_else(|_| value.to_string());
                    new_key.encrypt_string(&plaintext)
                })
                .transpose()?;
            let tool_call_id = row
                .tool_call_id
                .as_deref()
                .map(|value| {
                    let plaintext = old_key
                        .decrypt_string(value)
                        .unwrap_or_else(|_| value.to_string());
                    new_key.encrypt_string(&plaintext)
                })
                .transpose()?;
            let provider_message_json = row
                .provider_message_json
                .as_deref()
                .map(|value| {
                    let plaintext = old_key
                        .decrypt_string(value)
                        .unwrap_or_else(|_| value.to_string());
                    new_key.encrypt_string(&plaintext)
                })
                .transpose()?;
            message::ActiveModel {
                id: Unchanged(row.id),
                content: Set(encrypted),
                tool_calls_json: Set(tool_calls_json),
                tool_call_id: Set(tool_call_id),
                provider_message_json: Set(provider_message_json),
                ..Default::default()
            }
            .update(&txn)
            .await?;
        }

        let tasks = task::Entity::find().all(&txn).await?;
        for row in tasks {
            let description = new_key.encrypt_string(
                &old_key
                    .decrypt_string(&row.description)
                    .unwrap_or_else(|_| row.description.clone()),
            )?;
            let arguments = new_key.encrypt_string(
                &old_key
                    .decrypt_string(&row.arguments)
                    .unwrap_or_else(|_| row.arguments.clone()),
            )?;
            let approval = new_key.encrypt_string(
                &old_key
                    .decrypt_string(&row.approval)
                    .unwrap_or_else(|_| row.approval.clone()),
            )?;
            let result = row.result.map(|value| {
                let plaintext = old_key
                    .decrypt_string(&value)
                    .unwrap_or_else(|_| value.clone());
                new_key.encrypt_string(&plaintext)
            });
            task::ActiveModel {
                id: Unchanged(row.id),
                description: Set(description),
                arguments: Set(arguments),
                approval: Set(approval),
                result: Set(result.transpose()?),
                ..Default::default()
            }
            .update(&txn)
            .await?;
        }

        let approvals = approval_log::Entity::find().all(&txn).await?;
        for row in approvals {
            let plaintext = old_key
                .decrypt_string(&row.arguments)
                .unwrap_or_else(|_| row.arguments.clone());
            let encrypted = new_key.encrypt_string(&plaintext)?;
            approval_log::ActiveModel {
                id: Unchanged(row.id),
                arguments: Set(encrypted),
                ..Default::default()
            }
            .update(&txn)
            .await?;
        }

        let traces = execution_trace::Entity::find().all(&txn).await?;
        for row in traces {
            let message = new_key.encrypt_string(
                &old_key
                    .decrypt_string(&row.message)
                    .unwrap_or_else(|_| row.message.clone()),
            )?;
            let steps_json = new_key.encrypt_string(
                &old_key
                    .decrypt_string(&row.steps_json)
                    .unwrap_or_else(|_| row.steps_json.clone()),
            )?;
            let response = row.response.map(|value| {
                let plaintext = old_key
                    .decrypt_string(&value)
                    .unwrap_or_else(|_| value.clone());
                new_key.encrypt_string(&plaintext)
            });
            execution_trace::ActiveModel {
                id: Unchanged(row.id),
                message: Set(message),
                steps_json: Set(steps_json),
                response: Set(response.transpose()?),
                ..Default::default()
            }
            .update(&txn)
            .await?;
        }

        let user_data_items = user_data_item::Entity::find().all(&txn).await?;
        for row in user_data_items {
            let title = new_key.encrypt_string(
                &old_key
                    .decrypt_string(&row.title)
                    .unwrap_or_else(|_| row.title.clone()),
            )?;
            let content = new_key.encrypt_string(
                &old_key
                    .decrypt_string(&row.content)
                    .unwrap_or_else(|_| row.content.clone()),
            )?;
            user_data_item::ActiveModel {
                id: Unchanged(row.id),
                title: Set(title),
                content: Set(content),
                ..Default::default()
            }
            .update(&txn)
            .await?;
        }

        let knowledge_items = knowledge_item::Entity::find().all(&txn).await?;
        for row in knowledge_items {
            let title = new_key.encrypt_string(
                &old_key
                    .decrypt_string(&row.title)
                    .unwrap_or_else(|_| row.title.clone()),
            )?;
            let content = new_key.encrypt_string(
                &old_key
                    .decrypt_string(&row.content)
                    .unwrap_or_else(|_| row.content.clone()),
            )?;
            knowledge_item::ActiveModel {
                id: Unchanged(row.id),
                title: Set(title),
                content: Set(content),
                ..Default::default()
            }
            .update(&txn)
            .await?;
        }

        let experience_items = experience_item::Entity::find().all(&txn).await?;
        for row in experience_items {
            let title = new_key.encrypt_string(
                &old_key
                    .decrypt_string(&row.title)
                    .unwrap_or_else(|_| row.title.clone()),
            )?;
            let content = new_key.encrypt_string(
                &old_key
                    .decrypt_string(&row.content)
                    .unwrap_or_else(|_| row.content.clone()),
            )?;
            experience_item::ActiveModel {
                id: Unchanged(row.id),
                title: Set(title),
                content: Set(content),
                ..Default::default()
            }
            .update(&txn)
            .await?;
        }

        let user_preferences = user_preference::Entity::find().all(&txn).await?;
        for row in user_preferences {
            let plaintext = old_key
                .decrypt_string(&row.value)
                .unwrap_or_else(|_| row.value.clone());
            let encrypted = new_key.encrypt_string(&plaintext)?;
            user_preference::ActiveModel {
                id: Unchanged(row.id),
                value: Set(encrypted),
                ..Default::default()
            }
            .update(&txn)
            .await?;
        }

        let document_chunks = document_chunk::Entity::find().all(&txn).await?;
        for row in document_chunks {
            let plaintext = old_key
                .decrypt_string(&row.content)
                .unwrap_or_else(|_| row.content.clone());
            let encrypted = new_key.encrypt_string(&plaintext)?;
            document_chunk::ActiveModel {
                id: Unchanged(row.id),
                content: Set(encrypted),
                ..Default::default()
            }
            .update(&txn)
            .await?;
        }

        let documents = document::Entity::find().all(&txn).await?;
        for row in documents {
            let plaintext = old_key
                .decrypt_string(&row.filename)
                .unwrap_or_else(|_| row.filename.clone());
            let encrypted = new_key.encrypt_string(&plaintext)?;
            document::ActiveModel {
                id: Unchanged(row.id),
                filename: Set(encrypted),
                ..Default::default()
            }
            .update(&txn)
            .await?;
        }

        let notifications = notification::Entity::find().all(&txn).await?;
        for row in notifications {
            let title_plaintext = old_key
                .decrypt_string(&row.title)
                .unwrap_or_else(|_| row.title.clone());
            let body_plaintext = old_key
                .decrypt_string(&row.body)
                .unwrap_or_else(|_| row.body.clone());
            let encrypted_title = new_key.encrypt_string(&title_plaintext)?;
            let encrypted_body = new_key.encrypt_string(&body_plaintext)?;
            notification::ActiveModel {
                id: Unchanged(row.id),
                title: Set(encrypted_title),
                body: Set(encrypted_body),
                ..Default::default()
            }
            .update(&txn)
            .await?;
        }

        let security_logs = security_log::Entity::find().all(&txn).await?;
        for row in security_logs {
            let message = new_key.encrypt_string(
                &old_key
                    .decrypt_string(&row.message)
                    .unwrap_or_else(|_| row.message.clone()),
            )?;
            let source = row.source.map(|value| {
                let plaintext = old_key
                    .decrypt_string(&value)
                    .unwrap_or_else(|_| value.clone());
                new_key.encrypt_string(&plaintext)
            });
            security_log::ActiveModel {
                id: Unchanged(row.id),
                message: Set(message),
                source: Set(source.transpose()?),
                ..Default::default()
            }
            .update(&txn)
            .await?;
        }

        let operational_logs = operational_log::Entity::find().all(&txn).await?;
        for row in operational_logs {
            let outcome = new_key.encrypt_string(
                &old_key
                    .decrypt_string(&row.outcome)
                    .unwrap_or_else(|_| row.outcome.clone()),
            )?;
            let arguments = row.arguments.map(|value| {
                let plaintext = old_key
                    .decrypt_string(&value)
                    .unwrap_or_else(|_| value.clone());
                new_key.encrypt_string(&plaintext)
            });
            let payload = row.payload.map(|value| {
                let plaintext = old_key
                    .decrypt_string(&value)
                    .unwrap_or_else(|_| value.clone());
                new_key.encrypt_string(&plaintext)
            });
            operational_log::ActiveModel {
                id: Unchanged(row.id),
                outcome: Set(outcome),
                arguments: Set(arguments.transpose()?),
                payload: Set(payload.transpose()?),
                ..Default::default()
            }
            .update(&txn)
            .await?;
        }

        let delegations = swarm_delegation::Entity::find().all(&txn).await?;
        for row in delegations {
            let task_description = new_key.encrypt_string(
                &old_key
                    .decrypt_string(&row.task_description)
                    .unwrap_or_else(|_| row.task_description.clone()),
            )?;
            let result = row.result.map(|value| {
                let plaintext = old_key
                    .decrypt_string(&value)
                    .unwrap_or_else(|_| value.clone());
                new_key.encrypt_string(&plaintext)
            });
            swarm_delegation::ActiveModel {
                id: Unchanged(row.id),
                task_description: Set(task_description),
                result: Set(result.transpose()?),
                ..Default::default()
            }
            .update(&txn)
            .await?;
        }

        let automation_runs = automation_run::Entity::find().all(&txn).await?;
        for row in automation_runs {
            let plaintext = old_key
                .decrypt_string(&row.payload)
                .unwrap_or_else(|_| row.payload.clone());
            let encrypted = new_key.encrypt_string(&plaintext)?;
            automation_run::ActiveModel {
                id: Unchanged(row.id),
                payload: Set(encrypted),
                ..Default::default()
            }
            .update(&txn)
            .await?;
        }

        let automation_states = automation_supervisor_state::Entity::find()
            .all(&txn)
            .await?;
        for row in automation_states {
            let plaintext = old_key
                .decrypt_string(&row.payload)
                .unwrap_or_else(|_| row.payload.clone());
            let encrypted = new_key.encrypt_string(&plaintext)?;
            automation_supervisor_state::ActiveModel {
                automation_id: Unchanged(row.automation_id),
                payload: Set(encrypted),
                ..Default::default()
            }
            .update(&txn)
            .await?;
        }

        if !encrypted_kv_keys.is_empty() {
            let keys = encrypted_kv_keys
                .iter()
                .map(|key| (*key).to_string())
                .collect::<Vec<_>>();
            let rows = kv_store::Entity::find()
                .filter(kv_store::Column::Key.is_in(keys))
                .all(&txn)
                .await?;
            let now = chrono::Utc::now().to_rfc3339();
            for row in rows {
                let plaintext = old_key
                    .decrypt(&row.value)
                    .unwrap_or_else(|_| row.value.clone());
                let encrypted = new_key.encrypt(&plaintext)?;
                kv_store::ActiveModel {
                    key: Unchanged(row.key),
                    value: Set(encrypted),
                    updated_at: Set(now.clone()),
                    ..Default::default()
                }
                .update(&txn)
                .await?;
            }
        }

        if let Some((lineage_key, lineage_value)) = lineage_record {
            let now = chrono::Utc::now().to_rfc3339();
            kv_store::Entity::insert(kv_store::ActiveModel {
                key: Set(lineage_key),
                value: Set(lineage_value),
                created_at: Set(now.clone()),
                updated_at: Set(now),
            })
            .on_conflict(
                OnConflict::column(kv_store::Column::Key)
                    .update_columns([kv_store::Column::Value, kv_store::Column::UpdatedAt])
                    .to_owned(),
            )
            .exec(&txn)
            .await?;
        }

        txn.commit().await?;
        Ok(())
    }

    pub async fn ensure_sensitive_payloads_encrypted(
        &self,
        key_manager: &KeyManager,
        encrypted_kv_keys: &[&str],
    ) -> Result<bool> {
        let already_backfilled = self
            .get(Self::SENSITIVE_PAYLOAD_BACKFILL_MARKER_KEY)
            .await?
            .map(|bytes| bytes == b"done")
            .unwrap_or(false);
        if already_backfilled {
            return Ok(false);
        }

        let lineage_record = serde_json::to_vec(&serde_json::json!({
            "version": 1,
            "fingerprint": key_manager.fingerprint(),
            "recorded_at": chrono::Utc::now().to_rfc3339(),
        }))?;
        self.reencrypt_sensitive_payloads(
            key_manager,
            key_manager,
            encrypted_kv_keys,
            Some((
                crate::core::config::SETTINGS_KEY_LINEAGE_KEY.to_string(),
                lineage_record,
            )),
        )
        .await?;
        self.set(Self::SENSITIVE_PAYLOAD_BACKFILL_MARKER_KEY, b"done")
            .await?;
        Ok(true)
    }

    // ==================== Action Catalog Semantic Index ====================

    pub async fn action_catalog_index_entries(
        &self,
        action_names: &[String],
    ) -> Result<HashMap<String, action_catalog_index::Model>> {
        if action_names.is_empty() {
            return Ok(HashMap::new());
        }

        let rows = action_catalog_index::Entity::find()
            .filter(action_catalog_index::Column::ActionName.is_in(action_names.to_vec()))
            .all(&self.db)
            .await?;

        Ok(rows
            .into_iter()
            .map(|row| (row.action_name.clone(), row))
            .collect())
    }

    #[allow(dead_code)]
    pub async fn nearest_action_catalog_index_entries(
        &self,
        embedding: &PgVector,
        limit: u64,
    ) -> Result<Vec<(action_catalog_index::Model, f64)>> {
        if limit == 0 {
            return Ok(Vec::new());
        }
        if self.db.get_database_backend() != DbBackend::Postgres {
            return Ok(Vec::new());
        }

        let embedding_sql = pgvector_sql_literal(embedding);
        let sql = format!(
            "SELECT action_name, embedding <=> {embedding_sql} AS cosine_distance \
             FROM action_catalog_index \
             WHERE enabled = true \
               AND embedding IS NOT NULL \
             ORDER BY embedding <=> {embedding_sql} ASC \
             LIMIT {}",
            Self::db_limit(limit),
        );
        let rows = self
            .db
            .query_all(Statement::from_string(DbBackend::Postgres, sql))
            .await?;
        let mut scored = Vec::with_capacity(rows.len());
        for row in rows {
            let action_name: String = row.try_get("", "action_name")?;
            let distance: f64 = row.try_get("", "cosine_distance")?;
            scored.push((action_name, distance));
        }
        if scored.is_empty() {
            return Ok(Vec::new());
        }

        let action_names = scored
            .iter()
            .map(|(action_name, _)| action_name.clone())
            .collect::<Vec<_>>();
        let models = action_catalog_index::Entity::find()
            .filter(action_catalog_index::Column::ActionName.is_in(action_names))
            .all(&self.db)
            .await?;
        let mut by_name: HashMap<String, action_catalog_index::Model> = models
            .into_iter()
            .map(|model| (model.action_name.clone(), model))
            .collect();
        Ok(scored
            .into_iter()
            .filter_map(|(action_name, distance)| {
                by_name.remove(&action_name).map(|model| (model, distance))
            })
            .collect())
    }

    pub async fn upsert_action_catalog_index_entry(
        &self,
        entry: &ActionCatalogIndexEntry,
    ) -> Result<()> {
        let now = chrono::Utc::now().to_rfc3339();
        action_catalog_index::Entity::insert(action_catalog_index::ActiveModel {
            action_name: Set(entry.action_name.clone()),
            source: Set(entry.source.clone()),
            version: Set(entry.version.clone()),
            descriptor_hash: Set(entry.descriptor_hash.clone()),
            descriptor_text: Set(entry.descriptor_text.clone()),
            enabled: Set(entry.enabled),
            metadata_json: Set(entry.metadata_json.clone()),
            embedding: Set(entry.embedding.clone()),
            updated_at: Set(now),
        })
        .on_conflict(
            OnConflict::column(action_catalog_index::Column::ActionName)
                .update_columns([
                    action_catalog_index::Column::Source,
                    action_catalog_index::Column::Version,
                    action_catalog_index::Column::DescriptorHash,
                    action_catalog_index::Column::DescriptorText,
                    action_catalog_index::Column::Enabled,
                    action_catalog_index::Column::MetadataJson,
                    action_catalog_index::Column::Embedding,
                    action_catalog_index::Column::UpdatedAt,
                ])
                .to_owned(),
        )
        .exec(&self.db)
        .await?;
        Ok(())
    }

    pub async fn mark_unavailable_action_catalog_entries_disabled(
        &self,
        available_action_names: &[String],
    ) -> Result<u64> {
        let now = chrono::Utc::now().to_rfc3339();
        let filter = if available_action_names.is_empty() {
            "enabled = true".to_string()
        } else {
            format!(
                "enabled = true AND action_name NOT IN ({})",
                sql_string_list(available_action_names)
            )
        };
        let sql = format!(
            "UPDATE action_catalog_index \
             SET enabled = false, updated_at = {} \
             WHERE {}",
            sql_string_literal(&now),
            filter
        );
        let result = self
            .db
            .execute(Statement::from_string(DbBackend::Postgres, sql))
            .await?;
        Ok(result.rows_affected())
    }

    // ==================== LLM Usage ====================

    /// Insert an LLM usage record for analytics (tokens/cost estimation).
    pub async fn insert_llm_usage(&self, usage: &llm_usage::Model) -> Result<()> {
        llm_usage::ActiveModel {
            id: Set(usage.id.clone()),
            created_at: Set(usage.created_at.clone()),
            provider: Set(usage.provider.clone()),
            model: Set(usage.model.clone()),
            channel: Set(usage.channel.clone()),
            purpose: Set(usage.purpose.clone()),
            prompt_tokens: Set(usage.prompt_tokens),
            completion_tokens: Set(usage.completion_tokens),
            total_tokens: Set(usage.total_tokens),
            cached_prompt_tokens: Set(usage.cached_prompt_tokens),
            cache_creation_prompt_tokens: Set(usage.cache_creation_prompt_tokens),
            estimated: Set(usage.estimated),
            cost_usd: Set(usage.cost_usd),
        }
        .insert(&self.db)
        .await?;
        Ok(())
    }

    /// List LLM usage rows since a given RFC3339 timestamp (ascending).
    #[allow(dead_code)]
    pub async fn list_llm_usage_since(&self, since_rfc3339: &str) -> Result<Vec<llm_usage::Model>> {
        let rows = llm_usage::Entity::find()
            .filter(llm_usage::Column::CreatedAt.gte(since_rfc3339.to_string()))
            .order_by_asc(llm_usage::Column::CreatedAt)
            .limit(Self::MAX_LLM_USAGE_ROWS_PER_QUERY)
            .all(&self.db)
            .await?;
        Ok(rows)
    }

    /// List LLM usage rows inside a bounded time window.
    #[allow(dead_code)]
    pub async fn list_llm_usage_between(
        &self,
        from_rfc3339: &str,
        to_rfc3339: &str,
        limit: u64,
    ) -> Result<Vec<llm_usage::Model>> {
        let rows = llm_usage::Entity::find()
            .filter(llm_usage::Column::CreatedAt.gte(from_rfc3339.to_string()))
            .filter(llm_usage::Column::CreatedAt.lt(to_rfc3339.to_string()))
            .order_by_asc(llm_usage::Column::CreatedAt)
            .limit(Self::db_limit(
                limit.min(Self::MAX_LLM_USAGE_ROWS_PER_QUERY),
            ))
            .all(&self.db)
            .await?;
        Ok(rows)
    }

    /// List LLM usage rows for analytics without silently stopping at one page.
    /// Returns a truncation flag when the server-side safety cap is reached.
    pub async fn list_llm_usage_window_complete(
        &self,
        from_rfc3339: &str,
        to_rfc3339: &str,
    ) -> Result<(Vec<llm_usage::Model>, bool)> {
        let mut rows = Vec::new();
        let mut cursor: Option<(String, String)> = None;
        loop {
            let mut query = llm_usage::Entity::find()
                .filter(llm_usage::Column::CreatedAt.gte(from_rfc3339.to_string()))
                .filter(llm_usage::Column::CreatedAt.lt(to_rfc3339.to_string()));
            if let Some((created_at, id)) = cursor.as_ref() {
                query = query.filter(
                    Condition::any()
                        .add(llm_usage::Column::CreatedAt.gt(created_at.clone()))
                        .add(
                            Condition::all()
                                .add(llm_usage::Column::CreatedAt.eq(created_at.clone()))
                                .add(llm_usage::Column::Id.gt(id.clone())),
                        ),
                );
            }
            let page = query
                .order_by_asc(llm_usage::Column::CreatedAt)
                .order_by_asc(llm_usage::Column::Id)
                .limit(Self::MAX_LLM_USAGE_ROWS_PER_QUERY)
                .all(&self.db)
                .await?;
            if page.is_empty() {
                return Ok((rows, false));
            }
            let page_len = page.len();
            for row in page {
                cursor = Some((row.created_at.clone(), row.id.clone()));
                if rows.len() >= Self::MAX_LLM_USAGE_ANALYTICS_ROWS {
                    return Ok((rows, true));
                }
                rows.push(row);
            }
            if page_len < Self::MAX_LLM_USAGE_ROWS_PER_QUERY as usize {
                return Ok((rows, false));
            }
        }
    }

    // ==================== Learned Facts ====================

    /// Insert a learned fact into the current experience-item memory store.
    #[cfg(test)]
    pub async fn insert_fact(
        &self,
        id: &str,
        fact: &str,
        confidence: f32,
        sources: &str,
        embedding: Option<PgVector>,
        project_id: Option<&str>,
    ) -> Result<()> {
        let now = chrono::Utc::now().to_rfc3339();
        let scope = if project_id.is_some() {
            "project"
        } else {
            "global"
        };

        self.upsert_experience_item(&experience_item::Model {
            id: id.to_string(),
            kind: "personal_fact".to_string(),
            scope: scope.to_string(),
            project_id: project_id.map(str::to_string),
            conversation_id: None,
            title: "Learned fact".to_string(),
            content: fact.to_string(),
            normalized_key: format!("fact::{}", id),
            confidence: confidence.clamp(0.0, 1.0) as f64,
            support_count: 1,
            contradiction_count: 0,
            status: "active".to_string(),
            metadata: serde_json::json!({ "sources": sources }),
            last_supported_at: Some(now.clone()),
            last_contradicted_at: None,
            created_at: now.clone(),
            updated_at: now,
            embedding,
        })
        .await?;

        Ok(())
    }

    /// Get learned facts from the current experience-item memory store.
    pub async fn get_facts(&self) -> Result<Vec<LearnedFactRecord>> {
        let facts = experience_item::Entity::find()
            .filter(experience_item::Column::Status.eq("active"))
            .filter(experience_item::Column::Kind.is_in(["personal_fact", "constraint"]))
            .order_by_desc(experience_item::Column::UpdatedAt)
            .limit(Self::MAX_FACT_ROWS_PER_QUERY)
            .all(&self.db)
            .await?;
        Ok(facts
            .into_iter()
            .map(learned_fact_from_experience_item)
            .collect())
    }

    /// Get learned facts filtered by project (paginated).
    pub async fn get_facts_by_project(
        &self,
        limit: u64,
        offset: u64,
        project_id: Option<&str>,
    ) -> Result<Vec<LearnedFactRecord>> {
        let mut query = experience_item::Entity::find()
            .filter(experience_item::Column::Status.eq("active"))
            .filter(experience_item::Column::Kind.is_in(["personal_fact", "constraint"]))
            .order_by_desc(experience_item::Column::UpdatedAt);
        if let Some(pid) = project_id {
            query = query.filter(experience_item::Column::ProjectId.eq(pid));
        }
        let facts = query
            .limit(Self::db_limit(limit))
            .offset(Self::db_offset(offset))
            .all(&self.db)
            .await?;
        Ok(facts
            .into_iter()
            .map(learned_fact_from_experience_item)
            .collect())
    }

    async fn get_fact_items_by_project_unpaged(
        &self,
        project_id: Option<&str>,
    ) -> Result<Vec<experience_item::Model>> {
        let mut query = experience_item::Entity::find()
            .filter(experience_item::Column::Status.eq("active"))
            .filter(experience_item::Column::Kind.is_in(["personal_fact", "constraint"]))
            .order_by_desc(experience_item::Column::UpdatedAt);
        if let Some(pid) = project_id {
            query = query.filter(experience_item::Column::ProjectId.eq(pid));
        }
        Ok(query
            .limit(Self::MAX_FACT_ROWS_PER_QUERY)
            .all(&self.db)
            .await?)
    }

    /// Get learned memory rows filtered by semantic memory category.
    pub async fn get_facts_by_project_and_category(
        &self,
        limit: u64,
        offset: u64,
        project_id: Option<&str>,
        category: &str,
    ) -> Result<Vec<LearnedFactRecord>> {
        let category = category.trim();
        if category.is_empty() || category == "all" {
            return self.get_facts_by_project(limit, offset, project_id).await;
        }
        let offset = Self::db_offset(offset) as usize;
        let limit = Self::db_limit(limit) as usize;
        let facts = self
            .get_fact_items_by_project_unpaged(project_id)
            .await?
            .into_iter()
            .filter(|item| learned_fact_category_from_metadata(&item.metadata) == category)
            .skip(offset)
            .take(limit)
            .map(learned_fact_from_experience_item)
            .collect();
        Ok(facts)
    }

    /// Count learned facts in the current memory store.
    pub async fn count_facts(&self, project_id: Option<&str>) -> Result<u64> {
        let mut query = experience_item::Entity::find()
            .filter(experience_item::Column::Status.eq("active"))
            .filter(experience_item::Column::Kind.is_in(["personal_fact", "constraint"]));
        if let Some(pid) = project_id {
            query = query.filter(experience_item::Column::ProjectId.eq(pid));
        }
        Ok(query.count(&self.db).await?)
    }

    /// Count learned memory rows filtered by semantic memory category.
    pub async fn count_facts_by_category(
        &self,
        project_id: Option<&str>,
        category: &str,
    ) -> Result<u64> {
        let category = category.trim();
        if category.is_empty() || category == "all" {
            return self.count_facts(project_id).await;
        }
        let count = self
            .get_fact_items_by_project_unpaged(project_id)
            .await?
            .into_iter()
            .filter(|item| learned_fact_category_from_metadata(&item.metadata) == category)
            .count();
        Ok(count as u64)
    }

    // ==================== Tasks ====================

    // ==================== User Preferences ====================

    /// Upsert a user preference in a project scope (or global scope when project_id is None).
    pub async fn upsert_user_preference(
        &self,
        key: &str,
        value: &str,
        confidence: f32,
        source: Option<&str>,
        project_id: Option<&str>,
        sensitivity: Option<&str>,
    ) -> Result<user_preference::Model> {
        let key = key.trim();
        if key.is_empty() {
            anyhow::bail!("Preference key cannot be empty");
        }
        let id = Self::preference_row_id(key, project_id);
        let now = chrono::Utc::now().to_rfc3339();
        let bounded_confidence = confidence.clamp(0.0, 1.0);
        let encrypted_value = encrypt_storage_string(value)?;
        let normalized_project = project_id
            .map(|p| p.trim())
            .filter(|p| !p.is_empty())
            .map(|p| p.to_string());
        let normalized_sensitivity = user_preference::normalize_memory_sensitivity(sensitivity)
            .unwrap_or_else(|| user_preference::classify_user_preference_sensitivity(key, value))
            .as_str()
            .to_string();

        if let Some(existing) = user_preference::Entity::find_by_id(id.clone())
            .one(&self.db)
            .await?
        {
            let mut model: user_preference::ActiveModel = existing.into();
            model.key = Set(key.to_ascii_lowercase());
            model.value = Set(encrypted_value.clone());
            model.sensitivity = Set(normalized_sensitivity);
            model.confidence = Set(bounded_confidence);
            model.source = Set(source.map(|s| s.to_string()));
            model.project_id = Set(normalized_project);
            model.updated_at = Set(now);
            let mut updated = model.update(&self.db).await?;
            updated.value = decrypt_storage_string(&updated.value);
            Ok(updated)
        } else {
            let model = user_preference::ActiveModel {
                id: Set(id),
                key: Set(key.to_ascii_lowercase()),
                value: Set(encrypted_value),
                sensitivity: Set(normalized_sensitivity),
                confidence: Set(bounded_confidence),
                source: Set(source.map(|s| s.to_string())),
                project_id: Set(normalized_project),
                created_at: Set(now.clone()),
                updated_at: Set(now),
            }
            .insert(&self.db)
            .await?;
            let mut model = model;
            model.value = decrypt_storage_string(&model.value);
            Ok(model)
        }
    }

    /// Get a single user preference by key + scope.
    pub async fn get_user_preference(
        &self,
        key: &str,
        project_id: Option<&str>,
    ) -> Result<Option<user_preference::Model>> {
        let key = key.trim();
        if key.is_empty() {
            anyhow::bail!("Preference key cannot be empty");
        }
        let id = Self::preference_row_id(key, project_id);
        let Some(mut model) = user_preference::Entity::find_by_id(id)
            .one(&self.db)
            .await?
        else {
            return Ok(None);
        };
        model.value = decrypt_storage_string(&model.value);
        Ok(Some(model))
    }

    /// List user preferences by scope.
    pub async fn list_user_preferences(
        &self,
        limit: u64,
        offset: u64,
        project_id: Option<&str>,
    ) -> Result<Vec<user_preference::Model>> {
        let mut query = user_preference::Entity::find()
            .filter(
                user_preference::Column::Sensitivity
                    .ne(user_preference::SENSITIVITY_PERSONAL_IDENTIFIER),
            )
            .order_by_desc(user_preference::Column::UpdatedAt);
        if let Some(pid) = project_id {
            query = query.filter(user_preference::Column::ProjectId.eq(pid));
        }
        let mut rows = query
            .limit(Self::db_limit(limit))
            .offset(Self::db_offset(offset))
            .all(&self.db)
            .await?;
        for row in &mut rows {
            row.value = decrypt_storage_string(&row.value);
        }
        Ok(rows)
    }

    /// Count user preferences by scope.
    pub async fn count_user_preferences(&self, project_id: Option<&str>) -> Result<u64> {
        let mut query = user_preference::Entity::find().filter(
            user_preference::Column::Sensitivity
                .ne(user_preference::SENSITIVITY_PERSONAL_IDENTIFIER),
        );
        if let Some(pid) = project_id {
            query = query.filter(user_preference::Column::ProjectId.eq(pid));
        }
        Ok(query.count(&self.db).await?)
    }

    /// Delete a user preference by key + scope.
    pub async fn delete_user_preference(
        &self,
        key: &str,
        project_id: Option<&str>,
    ) -> Result<bool> {
        let id = Self::preference_row_id(key, project_id);
        let result = user_preference::Entity::delete_by_id(id)
            .exec(&self.db)
            .await?;
        Ok(result.rows_affected > 0)
    }

    // ==================== User Data Items ====================

    /// Insert a user data item.
    pub async fn create_user_data_item(
        &self,
        item: NewUserDataItem<'_>,
    ) -> Result<user_data_item::Model> {
        let now = chrono::Utc::now().to_rfc3339();
        let title = encrypt_storage_string(item.title.trim())?;
        let content = encrypt_storage_string(item.content)?;
        let model = user_data_item::ActiveModel {
            id: Set(uuid::Uuid::new_v4().to_string()),
            kind: Set(item.kind.trim().to_string()),
            title: Set(title),
            content: Set(content),
            url: Set(item
                .url
                .map(|v| v.trim().to_string())
                .filter(|v| !v.is_empty())),
            source_channel: Set(item.source_channel.map(|v| v.to_string())),
            conversation_id: Set(item.conversation_id.map(|v| v.to_string())),
            project_id: Set(item.project_id.map(|v| v.to_string())),
            pinned: Set(item.pinned),
            created_at: Set(now.clone()),
            updated_at: Set(now),
        }
        .insert(&self.db)
        .await?;
        let mut model = model;
        model.title = decrypt_storage_string(&model.title);
        model.content = decrypt_storage_string(&model.content);
        Ok(model)
    }

    /// Upsert an auto-captured link into user data (deduped by URL + project scope).
    pub async fn upsert_user_data_link(
        &self,
        url: &str,
        source_channel: Option<&str>,
        conversation_id: Option<&str>,
        project_id: Option<&str>,
    ) -> Result<user_data_item::Model> {
        let normalized_url = url.trim();
        if normalized_url.is_empty()
            || (!normalized_url.starts_with("http://") && !normalized_url.starts_with("https://"))
        {
            anyhow::bail!("Only http/https URLs can be stored as link user-data");
        }

        let now = chrono::Utc::now().to_rfc3339();
        let mut query = user_data_item::Entity::find()
            .filter(user_data_item::Column::Kind.eq("link"))
            .filter(user_data_item::Column::Url.eq(normalized_url.to_string()))
            .order_by_desc(user_data_item::Column::UpdatedAt);

        if let Some(pid) = project_id {
            query = query.filter(user_data_item::Column::ProjectId.eq(pid));
        } else {
            query = query.filter(user_data_item::Column::ProjectId.is_null());
        }

        if let Some(existing) = query.one(&self.db).await? {
            let mut model: user_data_item::ActiveModel = existing.into();
            model.source_channel = Set(source_channel.map(|v| v.to_string()));
            model.conversation_id = Set(conversation_id.map(|v| v.to_string()));
            model.updated_at = Set(now);
            let mut updated = model.update(&self.db).await?;
            updated.title = decrypt_storage_string(&updated.title);
            updated.content = decrypt_storage_string(&updated.content);
            Ok(updated)
        } else {
            let title = Self::default_link_title(normalized_url);
            self.create_user_data_item(NewUserDataItem {
                kind: "link",
                title: &title,
                content: "Auto-saved link from user chat",
                url: Some(normalized_url),
                source_channel,
                conversation_id,
                project_id,
                pinned: false,
            })
            .await
        }
    }

    /// List user data items by scope and optional kind.
    pub async fn list_user_data_items(
        &self,
        limit: u64,
        offset: u64,
        project_id: Option<&str>,
        kind: Option<&str>,
    ) -> Result<Vec<user_data_item::Model>> {
        let mut query =
            user_data_item::Entity::find().order_by_desc(user_data_item::Column::UpdatedAt);
        if let Some(pid) = project_id {
            query = query.filter(user_data_item::Column::ProjectId.eq(pid));
        }
        if let Some(kind_value) = kind.map(|v| v.trim()).filter(|v| !v.is_empty()) {
            query = query.filter(user_data_item::Column::Kind.eq(kind_value));
        }
        let mut rows = query
            .limit(Self::db_limit(limit))
            .offset(Self::db_offset(offset))
            .all(&self.db)
            .await?;
        for row in &mut rows {
            row.title = decrypt_storage_string(&row.title);
            row.content = decrypt_storage_string(&row.content);
        }
        Ok(rows)
    }

    /// Count user data items by scope and optional kind.
    pub async fn count_user_data_items(
        &self,
        project_id: Option<&str>,
        kind: Option<&str>,
    ) -> Result<u64> {
        let mut query = user_data_item::Entity::find();
        if let Some(pid) = project_id {
            query = query.filter(user_data_item::Column::ProjectId.eq(pid));
        }
        if let Some(kind_value) = kind.map(|v| v.trim()).filter(|v| !v.is_empty()) {
            query = query.filter(user_data_item::Column::Kind.eq(kind_value));
        }
        Ok(query.count(&self.db).await?)
    }

    /// Delete a user data item.
    pub async fn delete_user_data_item(&self, id: &str) -> Result<bool> {
        let result = user_data_item::Entity::delete_by_id(id.to_string())
            .exec(&self.db)
            .await?;
        Ok(result.rows_affected > 0)
    }

    // ==================== Knowledge Items ====================

    /// Insert a knowledge base item.
    pub async fn create_knowledge_item(
        &self,
        title: &str,
        content: &str,
        source: Option<&str>,
        url: Option<&str>,
        tags: Option<&str>,
        project_id: Option<&str>,
    ) -> Result<knowledge_item::Model> {
        let now = chrono::Utc::now().to_rfc3339();
        let title = encrypt_storage_string(title.trim())?;
        let content = encrypt_storage_string(content)?;
        let model = knowledge_item::ActiveModel {
            id: Set(uuid::Uuid::new_v4().to_string()),
            title: Set(title),
            content: Set(content),
            source: Set(source.map(|v| v.to_string())),
            url: Set(url.map(|v| v.to_string())),
            tags: Set(tags.map(|v| v.to_string())),
            project_id: Set(project_id.map(|v| v.to_string())),
            created_at: Set(now.clone()),
            updated_at: Set(now),
        }
        .insert(&self.db)
        .await?;
        let mut model = model;
        model.title = decrypt_storage_string(&model.title);
        model.content = decrypt_storage_string(&model.content);
        Ok(model)
    }

    fn visible_knowledge_source_filter() -> Condition {
        Condition::any()
            .add(knowledge_item::Column::Source.is_null())
            .add(knowledge_item::Column::Source.is_not_in([
                crate::core::agentark_knowledge::CURATED_SOURCE,
                crate::core::agentark_knowledge::RUNTIME_SOURCE,
            ]))
    }

    /// List knowledge base items visible in end-user memory UI.
    pub async fn list_visible_knowledge_items(
        &self,
        limit: u64,
        offset: u64,
        project_id: Option<&str>,
    ) -> Result<Vec<knowledge_item::Model>> {
        let mut query = knowledge_item::Entity::find()
            .filter(Self::visible_knowledge_source_filter())
            .order_by_desc(knowledge_item::Column::UpdatedAt);
        if let Some(pid) = project_id {
            query = query.filter(knowledge_item::Column::ProjectId.eq(pid));
        }
        let mut rows = query
            .limit(Self::db_limit(limit))
            .offset(Self::db_offset(offset))
            .all(&self.db)
            .await?;
        for row in &mut rows {
            row.title = decrypt_storage_string(&row.title);
            row.content = decrypt_storage_string(&row.content);
        }
        Ok(rows)
    }

    /// Count knowledge base items visible in end-user memory UI.
    pub async fn count_visible_knowledge_items(&self, project_id: Option<&str>) -> Result<u64> {
        let mut query =
            knowledge_item::Entity::find().filter(Self::visible_knowledge_source_filter());
        if let Some(pid) = project_id {
            query = query.filter(knowledge_item::Column::ProjectId.eq(pid));
        }
        Ok(query.count(&self.db).await?)
    }

    /// Delete all knowledge base items for a specific source.
    pub async fn delete_knowledge_items_by_source(&self, source: &str) -> Result<u64> {
        let result = knowledge_item::Entity::delete_many()
            .filter(knowledge_item::Column::Source.eq(source.to_string()))
            .exec(&self.db)
            .await?;
        Ok(result.rows_affected)
    }

    /// Delete a knowledge base item.
    pub async fn delete_knowledge_item(&self, id: &str) -> Result<bool> {
        let result = knowledge_item::Entity::delete_by_id(id.to_string())
            .exec(&self.db)
            .await?;
        Ok(result.rows_affected > 0)
    }

    pub async fn upsert_knowledge_entity(&self, entity: &knowledge_entity::Model) -> Result<()> {
        knowledge_entity::Entity::insert(knowledge_entity::ActiveModel {
            id: Set(entity.id.clone()),
            entity_type: Set(entity.entity_type.clone()),
            canonical_name: Set(entity.canonical_name.clone()),
            normalized_name: Set(entity.normalized_name.clone()),
            project_id: Set(entity.project_id.clone()),
            status: Set(entity.status.clone()),
            confidence: Set(entity.confidence),
            aliases: Set(entity.aliases.clone()),
            metadata: Set(entity.metadata.clone()),
            first_seen_at: Set(entity.first_seen_at.clone()),
            last_seen_at: Set(entity.last_seen_at.clone()),
            created_at: Set(entity.created_at.clone()),
            updated_at: Set(entity.updated_at.clone()),
        })
        .on_conflict(
            OnConflict::column(knowledge_entity::Column::Id)
                .update_columns([
                    knowledge_entity::Column::EntityType,
                    knowledge_entity::Column::CanonicalName,
                    knowledge_entity::Column::NormalizedName,
                    knowledge_entity::Column::ProjectId,
                    knowledge_entity::Column::Status,
                    knowledge_entity::Column::Confidence,
                    knowledge_entity::Column::Aliases,
                    knowledge_entity::Column::Metadata,
                    knowledge_entity::Column::LastSeenAt,
                    knowledge_entity::Column::UpdatedAt,
                ])
                .to_owned(),
        )
        .exec(&self.db)
        .await?;
        Ok(())
    }

    pub async fn get_knowledge_entity(&self, id: &str) -> Result<Option<knowledge_entity::Model>> {
        Ok(knowledge_entity::Entity::find_by_id(id.to_string())
            .one(&self.db)
            .await?)
    }

    pub async fn list_knowledge_entities_for_graph(
        &self,
        statuses: &[String],
        project_id: Option<&str>,
        limit: u64,
    ) -> Result<Vec<knowledge_entity::Model>> {
        let mut query = knowledge_entity::Entity::find();
        if !statuses.is_empty() {
            query = query.filter(knowledge_entity::Column::Status.is_in(statuses.to_vec()));
        }
        query = match project_id.map(str::trim).filter(|value| !value.is_empty()) {
            Some(pid) => query.filter(
                Condition::any()
                    .add(knowledge_entity::Column::ProjectId.is_null())
                    .add(knowledge_entity::Column::ProjectId.eq(pid.to_string())),
            ),
            None => query.filter(knowledge_entity::Column::ProjectId.is_null()),
        };
        Ok(query
            .order_by_desc(knowledge_entity::Column::UpdatedAt)
            .limit(Self::db_limit(limit.min(500)))
            .all(&self.db)
            .await?)
    }

    pub async fn upsert_knowledge_relation(
        &self,
        relation: &knowledge_relation::Model,
    ) -> Result<()> {
        knowledge_relation::Entity::insert(knowledge_relation::ActiveModel {
            id: Set(relation.id.clone()),
            source_entity_id: Set(relation.source_entity_id.clone()),
            target_entity_id: Set(relation.target_entity_id.clone()),
            relation_type: Set(relation.relation_type.clone()),
            status: Set(relation.status.clone()),
            confidence: Set(relation.confidence),
            project_id: Set(relation.project_id.clone()),
            valid_from: Set(relation.valid_from.clone()),
            valid_until: Set(relation.valid_until.clone()),
            support_count: Set(relation.support_count),
            contradiction_count: Set(relation.contradiction_count),
            metadata: Set(relation.metadata.clone()),
            first_seen_at: Set(relation.first_seen_at.clone()),
            last_seen_at: Set(relation.last_seen_at.clone()),
            created_at: Set(relation.created_at.clone()),
            updated_at: Set(relation.updated_at.clone()),
        })
        .on_conflict(
            OnConflict::column(knowledge_relation::Column::Id)
                .update_columns([
                    knowledge_relation::Column::SourceEntityId,
                    knowledge_relation::Column::TargetEntityId,
                    knowledge_relation::Column::RelationType,
                    knowledge_relation::Column::Status,
                    knowledge_relation::Column::Confidence,
                    knowledge_relation::Column::ProjectId,
                    knowledge_relation::Column::ValidFrom,
                    knowledge_relation::Column::ValidUntil,
                    knowledge_relation::Column::SupportCount,
                    knowledge_relation::Column::ContradictionCount,
                    knowledge_relation::Column::Metadata,
                    knowledge_relation::Column::LastSeenAt,
                    knowledge_relation::Column::UpdatedAt,
                ])
                .to_owned(),
        )
        .exec(&self.db)
        .await?;
        Ok(())
    }

    pub async fn get_knowledge_relation(
        &self,
        id: &str,
    ) -> Result<Option<knowledge_relation::Model>> {
        Ok(knowledge_relation::Entity::find_by_id(id.to_string())
            .one(&self.db)
            .await?)
    }

    pub async fn update_knowledge_relation_status(&self, id: &str, status: &str) -> Result<bool> {
        let Some(row) = knowledge_relation::Entity::find_by_id(id.to_string())
            .one(&self.db)
            .await?
        else {
            return Ok(false);
        };
        let mut model: knowledge_relation::ActiveModel = row.into();
        model.status = Set(status.to_string());
        model.updated_at = Set(chrono::Utc::now().to_rfc3339());
        model.update(&self.db).await?;
        Ok(true)
    }

    pub async fn list_knowledge_relations_for_entities(
        &self,
        entity_ids: &[String],
        statuses: &[String],
        relation_types: &[String],
        limit: u64,
    ) -> Result<Vec<knowledge_relation::Model>> {
        if entity_ids.is_empty() {
            return Ok(Vec::new());
        }
        let mut query = knowledge_relation::Entity::find().filter(
            Condition::any()
                .add(knowledge_relation::Column::SourceEntityId.is_in(entity_ids.to_vec()))
                .add(knowledge_relation::Column::TargetEntityId.is_in(entity_ids.to_vec())),
        );
        if !statuses.is_empty() {
            query = query.filter(knowledge_relation::Column::Status.is_in(statuses.to_vec()));
        }
        if !relation_types.is_empty() {
            query = query
                .filter(knowledge_relation::Column::RelationType.is_in(relation_types.to_vec()));
        }
        Ok(query
            .order_by_desc(knowledge_relation::Column::UpdatedAt)
            .limit(Self::db_limit(limit.min(1_000)))
            .all(&self.db)
            .await?)
    }

    pub async fn upsert_knowledge_relation_evidence(
        &self,
        evidence: &knowledge_relation_evidence::Model,
    ) -> Result<()> {
        knowledge_relation_evidence::Entity::insert(knowledge_relation_evidence::ActiveModel {
            id: Set(evidence.id.clone()),
            relation_id: Set(evidence.relation_id.clone()),
            evidence_kind: Set(evidence.evidence_kind.clone()),
            evidence_ref: Set(evidence.evidence_ref.clone()),
            memory_id: Set(evidence.memory_id.clone()),
            message_id: Set(evidence.message_id.clone()),
            document_id: Set(evidence.document_id.clone()),
            project_id: Set(evidence.project_id.clone()),
            conversation_id: Set(evidence.conversation_id.clone()),
            polarity: Set(evidence.polarity.clone()),
            confidence: Set(evidence.confidence),
            excerpt: Set(evidence.excerpt.clone()),
            metadata: Set(evidence.metadata.clone()),
            created_at: Set(evidence.created_at.clone()),
        })
        .on_conflict(
            OnConflict::column(knowledge_relation_evidence::Column::Id)
                .update_columns([
                    knowledge_relation_evidence::Column::RelationId,
                    knowledge_relation_evidence::Column::EvidenceKind,
                    knowledge_relation_evidence::Column::EvidenceRef,
                    knowledge_relation_evidence::Column::MemoryId,
                    knowledge_relation_evidence::Column::MessageId,
                    knowledge_relation_evidence::Column::DocumentId,
                    knowledge_relation_evidence::Column::ProjectId,
                    knowledge_relation_evidence::Column::ConversationId,
                    knowledge_relation_evidence::Column::Polarity,
                    knowledge_relation_evidence::Column::Confidence,
                    knowledge_relation_evidence::Column::Excerpt,
                    knowledge_relation_evidence::Column::Metadata,
                ])
                .to_owned(),
        )
        .exec(&self.db)
        .await?;
        Ok(())
    }

    pub async fn list_knowledge_relation_evidence_for_relations(
        &self,
        relation_ids: &[String],
        limit: u64,
    ) -> Result<Vec<knowledge_relation_evidence::Model>> {
        if relation_ids.is_empty() {
            return Ok(Vec::new());
        }
        Ok(knowledge_relation_evidence::Entity::find()
            .filter(knowledge_relation_evidence::Column::RelationId.is_in(relation_ids.to_vec()))
            .order_by_desc(knowledge_relation_evidence::Column::CreatedAt)
            .limit(Self::db_limit(limit.min(1_000)))
            .all(&self.db)
            .await?)
    }

    pub async fn list_knowledge_relation_evidence_for_memory(
        &self,
        memory_id: &str,
        limit: u64,
    ) -> Result<Vec<knowledge_relation_evidence::Model>> {
        let memory_id = memory_id.trim();
        if memory_id.is_empty() {
            return Ok(Vec::new());
        }
        Ok(knowledge_relation_evidence::Entity::find()
            .filter(knowledge_relation_evidence::Column::MemoryId.eq(memory_id.to_string()))
            .order_by_desc(knowledge_relation_evidence::Column::CreatedAt)
            .limit(Self::db_limit(limit.min(1_000)))
            .all(&self.db)
            .await?)
    }

    /// Insert a task
    pub async fn insert_task(&self, task: &crate::core::Task) -> Result<()> {
        let now = chrono::Utc::now().to_rfc3339();
        let description = encrypt_storage_string(&task.description)?;
        let arguments = encrypt_storage_string(&serde_json::to_string(&task.arguments)?)?;
        let approval = encrypt_storage_string(&serde_json::to_string(&task.approval)?)?;
        let result = encrypt_optional_storage_string(task.result.as_deref())?;
        task::ActiveModel {
            id: Set(task.id.to_string()),
            description: Set(description),
            action: Set(task.action.clone()),
            arguments: Set(arguments),
            approval: Set(approval),
            status: Set(serde_json::to_string(&task.status)?),
            created_at: Set(task.created_at.to_rfc3339()),
            updated_at: Set(now),
            scheduled_for: Set(task.scheduled_for.map(|t| t.to_rfc3339())),
            cron: Set(task.cron.clone()),
            result: Set(result),
            proof_id: Set(task.proof_id.map(|id| id.to_string())),
            priority: Set(task.priority.map(|v| v as f64)),
            urgency: Set(task.urgency.map(|v| v as f64)),
            importance: Set(task.importance.map(|v| v as f64)),
            eisenhower_quadrant: Set(task.eisenhower_quadrant.map(|v| v as i32)),
            lease_owner: Set(None),
            lease_expires_at: Set(None),
            lease_version: Set(0),
            next_retry_at: Set(None),
            last_run_id: Set(None),
            consecutive_failures: Set(0),
        }
        .insert(&self.db)
        .await?;

        Ok(())
    }

    /// Update task status
    pub async fn update_task_status(&self, id: &str, status: &str) -> Result<()> {
        task::ActiveModel {
            id: Set(id.to_string()),
            status: Set(status.to_string()),
            updated_at: Set(chrono::Utc::now().to_rfc3339()),
            lease_owner: Set(None),
            lease_expires_at: Set(None),
            ..Default::default()
        }
        .update(&self.db)
        .await?;

        Ok(())
    }

    /// Update task fields
    pub async fn update_task(
        &self,
        id: &str,
        description: Option<String>,
        arguments: Option<String>,
        cron: Option<String>,
        scheduled_for: Option<String>,
    ) -> Result<()> {
        let mut model = task::ActiveModel {
            id: Set(id.to_string()),
            ..Default::default()
        };

        if let Some(desc) = description {
            model.description = Set(encrypt_storage_string(&desc)?);
        }
        if let Some(args) = arguments {
            model.arguments = Set(encrypt_storage_string(&args)?);
        }
        if cron.is_some() {
            model.cron = Set(cron);
        }
        if scheduled_for.is_some() {
            model.scheduled_for = Set(scheduled_for);
        }
        model.updated_at = Set(chrono::Utc::now().to_rfc3339());

        model.update(&self.db).await?;
        Ok(())
    }

    pub async fn replace_task_schedule(
        &self,
        id: &str,
        cron: Option<String>,
        scheduled_for: Option<String>,
    ) -> Result<()> {
        task::ActiveModel {
            id: Set(id.to_string()),
            cron: Set(cron),
            scheduled_for: Set(scheduled_for),
            updated_at: Set(chrono::Utc::now().to_rfc3339()),
            ..Default::default()
        }
        .update(&self.db)
        .await?;
        Ok(())
    }

    pub async fn update_task_status_and_result(
        &self,
        id: &str,
        status: &str,
        result: Option<&str>,
    ) -> Result<()> {
        let mut model = task::ActiveModel {
            id: Set(id.to_string()),
            status: Set(status.to_string()),
            updated_at: Set(chrono::Utc::now().to_rfc3339()),
            lease_owner: Set(None),
            lease_expires_at: Set(None),
            ..Default::default()
        };
        if let Some(res) = result {
            model.result = Set(Some(encrypt_storage_string(res)?));
        }
        model.update(&self.db).await?;
        Ok(())
    }

    /// Reset a failed/cancelled task so it can be retried.
    pub async fn retry_task(
        &self,
        id: &str,
        status: &str,
        scheduled_for: Option<String>,
    ) -> Result<()> {
        task::ActiveModel {
            id: Set(id.to_string()),
            status: Set(status.to_string()),
            scheduled_for: Set(scheduled_for),
            result: Set(None),
            proof_id: Set(None),
            updated_at: Set(chrono::Utc::now().to_rfc3339()),
            lease_owner: Set(None),
            lease_expires_at: Set(None),
            ..Default::default()
        }
        .update(&self.db)
        .await?;

        Ok(())
    }

    pub async fn list_background_sessions(&self) -> Result<Vec<crate::core::BackgroundSession>> {
        let rows = background_session::Entity::find()
            .order_by_desc(background_session::Column::UpdatedAt)
            .all(&self.db)
            .await?;
        let mut sessions = Vec::with_capacity(rows.len());
        for row in rows {
            let payload = decrypt_storage_string(&row.payload);
            match serde_json::from_str::<crate::core::BackgroundSession>(&payload) {
                Ok(mut session) => {
                    session.policy = session.policy.normalized();
                    sessions.push(session);
                }
                Err(error) => {
                    tracing::warn!(
                        "Failed to parse persisted background session {}; skipping row: {}",
                        row.id,
                        error
                    );
                }
            }
        }
        Ok(sessions)
    }

    pub async fn upsert_background_session(
        &self,
        session: &crate::core::BackgroundSession,
    ) -> Result<()> {
        let payload = encrypt_storage_string(&serde_json::to_string(session)?)?;
        background_session::Entity::insert(background_session::ActiveModel {
            id: Set(session.id.clone()),
            status: Set(session.status.label().to_string()),
            conversation_id: Set(session.conversation_id.clone()),
            project_id: Set(session.project_id.clone()),
            created_at: Set(session.created_at.to_rfc3339()),
            updated_at: Set(session.updated_at.to_rfc3339()),
            last_activity_at: Set(session.last_activity_at.to_rfc3339()),
            payload: Set(payload),
        })
        .on_conflict(
            OnConflict::column(background_session::Column::Id)
                .update_columns([
                    background_session::Column::Status,
                    background_session::Column::ConversationId,
                    background_session::Column::ProjectId,
                    background_session::Column::UpdatedAt,
                    background_session::Column::LastActivityAt,
                    background_session::Column::Payload,
                ])
                .to_owned(),
        )
        .exec(&self.db)
        .await?;
        Ok(())
    }

    pub async fn delete_background_session(&self, id: &str) -> Result<()> {
        background_session::Entity::delete_by_id(id.to_string())
            .exec(&self.db)
            .await?;
        Ok(())
    }

    pub async fn try_claim_task(
        &self,
        id: &str,
        expected_status: &str,
        in_progress_status: &str,
        lease_owner: &str,
        lease_expires_at: &str,
    ) -> Result<bool> {
        let now = chrono::Utc::now().to_rfc3339();
        let result = task::Entity::update_many()
            .col_expr(
                task::Column::Status,
                Expr::value(in_progress_status.to_string()),
            )
            .col_expr(task::Column::UpdatedAt, Expr::value(now))
            .col_expr(
                task::Column::LeaseOwner,
                Expr::value(lease_owner.to_string()),
            )
            .col_expr(
                task::Column::LeaseExpiresAt,
                Expr::value(lease_expires_at.to_string()),
            )
            .col_expr(
                task::Column::LeaseVersion,
                Expr::col(task::Column::LeaseVersion).add(1),
            )
            .filter(task::Column::Id.eq(id))
            .filter(task::Column::Status.eq(expected_status))
            .filter(
                Condition::any()
                    .add(task::Column::LeaseExpiresAt.is_null())
                    .add(task::Column::LeaseExpiresAt.lte(chrono::Utc::now().to_rfc3339())),
            )
            .exec(&self.db)
            .await?;
        Ok(result.rows_affected > 0)
    }

    pub async fn record_task_run_metadata(
        &self,
        id: &str,
        last_run_id: Option<&str>,
        next_retry_at: Option<&str>,
        consecutive_failures: Option<i32>,
    ) -> Result<()> {
        task::ActiveModel {
            id: Set(id.to_string()),
            updated_at: Set(chrono::Utc::now().to_rfc3339()),
            last_run_id: Set(last_run_id.map(|value| value.to_string())),
            next_retry_at: Set(next_retry_at.map(|value| value.to_string())),
            consecutive_failures: Set(consecutive_failures.unwrap_or(0)),
            ..Default::default()
        }
        .update(&self.db)
        .await?;
        Ok(())
    }

    /// Delete a task
    pub async fn delete_task(&self, id: &str) -> Result<()> {
        let txn = self.db.begin().await?;
        self.cleanup_automation_records_for_ids(&txn, &[id.to_string()])
            .await?;

        let delegations = swarm_delegation::Entity::find()
            .filter(swarm_delegation::Column::ParentTaskId.eq(id.to_string()))
            .all(&txn)
            .await?;
        for row in delegations {
            swarm_delegation::ActiveModel {
                id: Unchanged(row.id),
                parent_task_id: Set(None),
                ..Default::default()
            }
            .update(&txn)
            .await?;
        }

        task::Entity::delete_by_id(id.to_string())
            .exec(&txn)
            .await?;
        txn.commit().await?;
        Ok(())
    }

    /// Get all tasks
    pub async fn get_tasks(&self) -> Result<Vec<task::Model>> {
        let mut tasks = task::Entity::find()
            .order_by_desc(task::Column::CreatedAt)
            .limit(Self::MAX_TASK_ROWS_PER_QUERY)
            .all(&self.db)
            .await?;
        for task in &mut tasks {
            task.description = decrypt_storage_string(&task.description);
            task.arguments = decrypt_storage_string(&task.arguments);
            task.approval = decrypt_storage_string(&task.approval);
            task.result = decrypt_optional_storage_string(task.result.take());
        }
        Ok(tasks)
    }

    /// List tasks updated inside a bounded time window.
    #[allow(dead_code)]
    pub async fn list_tasks_updated_between(
        &self,
        from: &str,
        to: &str,
        limit: u64,
    ) -> Result<Vec<task::Model>> {
        let mut tasks = task::Entity::find()
            .filter(task::Column::UpdatedAt.gte(from.to_string()))
            .filter(task::Column::UpdatedAt.lt(to.to_string()))
            .order_by_desc(task::Column::UpdatedAt)
            .limit(Self::db_limit(limit.min(Self::MAX_TASK_ROWS_PER_QUERY)))
            .all(&self.db)
            .await?;
        for task in &mut tasks {
            task.description = decrypt_storage_string(&task.description);
            task.arguments = decrypt_storage_string(&task.arguments);
            task.approval = decrypt_storage_string(&task.approval);
            task.result = decrypt_optional_storage_string(task.result.take());
        }
        Ok(tasks)
    }

    pub async fn list_automation_runs(
        &self,
        limit: usize,
    ) -> Result<Vec<crate::core::automation::AutomationRunRecord>> {
        let mut runs = Vec::new();
        for row in automation_run::Entity::find()
            .order_by_desc(automation_run::Column::StartedAt)
            .limit(limit.max(1) as u64)
            .all(&self.db)
            .await?
        {
            let payload = decrypt_storage_string(&row.payload);
            if let Ok(run) =
                serde_json::from_str::<crate::core::automation::AutomationRunRecord>(&payload)
            {
                runs.push(run);
            }
        }
        Ok(runs)
    }

    pub async fn list_automation_runs_since(
        &self,
        since: Option<&str>,
        limit: usize,
    ) -> Result<Vec<crate::core::automation::AutomationRunRecord>> {
        let mut query = automation_run::Entity::find()
            .order_by_desc(automation_run::Column::StartedAt)
            .limit(limit.max(1) as u64);
        if let Some(since) = since.map(str::trim).filter(|value| !value.is_empty()) {
            query = query.filter(automation_run::Column::StartedAt.gte(since.to_string()));
        }

        let mut runs = Vec::new();
        for row in query.all(&self.db).await? {
            let payload = decrypt_storage_string(&row.payload);
            if let Ok(run) =
                serde_json::from_str::<crate::core::automation::AutomationRunRecord>(&payload)
            {
                runs.push(run);
            }
        }
        Ok(runs)
    }

    pub async fn count_automation_runs(&self, since: Option<&str>) -> Result<u64> {
        let mut query = automation_run::Entity::find();
        if let Some(since) = since.map(str::trim).filter(|value| !value.is_empty()) {
            query = query.filter(automation_run::Column::StartedAt.gte(since.to_string()));
        }
        Ok(query.count(&self.db).await?)
    }

    pub async fn append_automation_run(
        &self,
        run: &crate::core::automation::AutomationRunRecord,
        max_records: usize,
    ) -> Result<()> {
        automation_run::Entity::insert(automation_run::ActiveModel {
            id: Set(run.id.clone()),
            automation_id: Set(run.automation_id.clone()),
            started_at: Set(run.started_at.clone()),
            payload: Set(encrypt_storage_string(&serde_json::to_string(run)?)?),
        })
        .on_conflict(
            OnConflict::column(automation_run::Column::Id)
                .update_columns([
                    automation_run::Column::AutomationId,
                    automation_run::Column::StartedAt,
                    automation_run::Column::Payload,
                ])
                .to_owned(),
        )
        .exec(&self.db)
        .await?;

        let overflow_ids = automation_run::Entity::find()
            .order_by_desc(automation_run::Column::StartedAt)
            .offset(max_records.max(1) as u64)
            .all(&self.db)
            .await?
            .into_iter()
            .map(|row| row.id)
            .collect::<Vec<_>>();
        if !overflow_ids.is_empty() {
            automation_run::Entity::delete_many()
                .filter(automation_run::Column::Id.is_in(overflow_ids))
                .exec(&self.db)
                .await?;
        }
        Ok(())
    }

    pub async fn list_automation_supervisor_states(
        &self,
    ) -> Result<Vec<crate::core::automation::AutomationSupervisorState>> {
        let mut states = Vec::new();
        for row in automation_supervisor_state::Entity::find()
            .order_by_desc(automation_supervisor_state::Column::UpdatedAt)
            .all(&self.db)
            .await?
        {
            let payload = decrypt_storage_string(&row.payload);
            if let Ok(state) =
                serde_json::from_str::<crate::core::automation::AutomationSupervisorState>(&payload)
            {
                states.push(state);
            }
        }
        Ok(states)
    }

    pub async fn load_automation_supervisor_state(
        &self,
        automation_id: &str,
    ) -> Result<Option<crate::core::automation::AutomationSupervisorState>> {
        Ok(
            automation_supervisor_state::Entity::find_by_id(automation_id.to_string())
                .one(&self.db)
                .await?
                .map(|row| decrypt_storage_string(&row.payload))
                .and_then(|payload| {
                    serde_json::from_str::<crate::core::automation::AutomationSupervisorState>(
                        &payload,
                    )
                    .ok()
                }),
        )
    }

    pub async fn upsert_automation_supervisor_state(
        &self,
        state: &crate::core::automation::AutomationSupervisorState,
    ) -> Result<()> {
        let updated_at = state
            .last_run_at
            .clone()
            .or_else(|| state.created_at.clone())
            .unwrap_or_else(|| chrono::Utc::now().to_rfc3339());
        automation_supervisor_state::Entity::insert(automation_supervisor_state::ActiveModel {
            automation_id: Set(state.automation_id.clone()),
            updated_at: Set(updated_at),
            payload: Set(encrypt_storage_string(&serde_json::to_string(state)?)?),
            next_retry_at: Set(state.next_retry_at.clone()),
            last_run_id: Set(state.last_run_id.clone()),
            consecutive_failures: Set(state.consecutive_failures as i32),
        })
        .on_conflict(
            OnConflict::column(automation_supervisor_state::Column::AutomationId)
                .update_columns([
                    automation_supervisor_state::Column::UpdatedAt,
                    automation_supervisor_state::Column::Payload,
                    automation_supervisor_state::Column::NextRetryAt,
                    automation_supervisor_state::Column::LastRunId,
                    automation_supervisor_state::Column::ConsecutiveFailures,
                ])
                .to_owned(),
        )
        .exec(&self.db)
        .await?;
        Ok(())
    }

    pub async fn delete_automation_supervisor_state(&self, automation_id: &str) -> Result<bool> {
        let result = automation_supervisor_state::Entity::delete_by_id(automation_id.to_string())
            .exec(&self.db)
            .await?;
        Ok(result.rows_affected > 0)
    }

    async fn cleanup_automation_records_for_ids<C>(
        &self,
        db: &C,
        automation_ids: &[String],
    ) -> Result<()>
    where
        C: ConnectionTrait,
    {
        if automation_ids.is_empty() {
            return Ok(());
        }

        let automation_id_filter = automation_ids.to_vec();
        let run_ids = automation_run::Entity::find()
            .select_only()
            .column(automation_run::Column::Id)
            .filter(automation_run::Column::AutomationId.is_in(automation_id_filter.clone()))
            .into_tuple::<String>()
            .all(db)
            .await?;

        if !run_ids.is_empty() {
            let task_rows = task::Entity::find()
                .filter(task::Column::LastRunId.is_in(run_ids.clone()))
                .all(db)
                .await?;
            for row in task_rows {
                task::ActiveModel {
                    id: Unchanged(row.id),
                    last_run_id: Set(None),
                    ..Default::default()
                }
                .update(db)
                .await?;
            }

            let watcher_rows = watcher::Entity::find()
                .filter(watcher::Column::LastRunId.is_in(run_ids.clone()))
                .all(db)
                .await?;
            for row in watcher_rows {
                watcher::ActiveModel {
                    id: Unchanged(row.id),
                    last_run_id: Set(None),
                    ..Default::default()
                }
                .update(db)
                .await?;
            }

            let supervisor_rows = automation_supervisor_state::Entity::find()
                .filter(automation_supervisor_state::Column::LastRunId.is_in(run_ids.clone()))
                .all(db)
                .await?;
            for row in supervisor_rows {
                automation_supervisor_state::ActiveModel {
                    automation_id: Unchanged(row.automation_id),
                    last_run_id: Set(None),
                    ..Default::default()
                }
                .update(db)
                .await?;
            }
        }

        automation_supervisor_state::Entity::delete_many()
            .filter(
                automation_supervisor_state::Column::AutomationId
                    .is_in(automation_id_filter.clone()),
            )
            .exec(db)
            .await?;
        automation_run::Entity::delete_many()
            .filter(automation_run::Column::AutomationId.is_in(automation_id_filter))
            .exec(db)
            .await?;
        Ok(())
    }

    pub async fn list_watchers(&self) -> Result<Vec<crate::core::watcher::Watcher>> {
        let mut watchers = Vec::new();
        let rows = watcher::Entity::find()
            .order_by_asc(watcher::Column::CreatedAt)
            .all(&self.db)
            .await?;
        for row in rows {
            let payload = row.payload;
            if let Ok(watcher) = serde_json::from_str::<crate::core::watcher::Watcher>(&payload) {
                watchers.push(watcher);
            }
        }
        Ok(watchers)
    }

    pub async fn upsert_watcher(&self, watcher: &crate::core::watcher::Watcher) -> Result<()> {
        let status = match &watcher.status {
            crate::core::watcher::WatcherStatus::Active => "active",
            crate::core::watcher::WatcherStatus::Paused => "paused",
            crate::core::watcher::WatcherStatus::Triggered => "triggered",
            crate::core::watcher::WatcherStatus::TimedOut => "timed_out",
            crate::core::watcher::WatcherStatus::Cancelled => "cancelled",
            crate::core::watcher::WatcherStatus::Failed { .. } => "failed",
        };
        watcher::Entity::insert(watcher::ActiveModel {
            id: Set(watcher.id.to_string()),
            status: Set(status.to_string()),
            created_at: Set(watcher.created_at.to_rfc3339()),
            updated_at: Set(chrono::Utc::now().to_rfc3339()),
            payload: Set(serde_json::to_string(watcher)?),
            lease_owner: Set(None),
            lease_expires_at: sea_orm::NotSet,
            lease_version: Set(0),
            next_retry_at: Set(watcher.next_poll_not_before.map(|value| value.to_rfc3339())),
            last_run_id: Set(None),
            consecutive_failures: Set(watcher.consecutive_failures as i32),
        })
        .on_conflict(
            OnConflict::column(watcher::Column::Id)
                .update_columns([
                    watcher::Column::Status,
                    watcher::Column::UpdatedAt,
                    watcher::Column::Payload,
                    watcher::Column::NextRetryAt,
                    watcher::Column::LastRunId,
                    watcher::Column::ConsecutiveFailures,
                ])
                .to_owned(),
        )
        .exec(&self.db)
        .await?;
        Ok(())
    }

    pub async fn delete_watcher(&self, id: &str) -> Result<()> {
        let txn = self.db.begin().await?;
        self.cleanup_automation_records_for_ids(&txn, &[id.to_string()])
            .await?;
        watcher::Entity::delete_by_id(id.to_string())
            .exec(&txn)
            .await?;
        txn.commit().await?;
        Ok(())
    }

    pub async fn list_browser_sessions(
        &self,
    ) -> Result<Vec<crate::core::browser_session::PersistedBrowserSession>> {
        let rows = browser_session::Entity::find()
            .order_by_desc(browser_session::Column::UpdatedAt)
            .all(&self.db)
            .await?;
        rows.into_iter()
            .map(|row| {
                let task_description = decrypt_storage_string(&row.task_description);
                let chat_id = row.chat_id.map(|value| decrypt_storage_string(&value));
                let profile_id = row.profile_id.map(|value| decrypt_storage_string(&value));
                let profile_name = row.profile_name.map(|value| decrypt_storage_string(&value));
                let status_detail = row
                    .status_detail
                    .map(|value| decrypt_storage_string(&value));
                let action_history_json = decrypt_storage_string(&row.action_history_json);
                Ok(crate::core::browser_session::PersistedBrowserSession {
                    id: row.id,
                    status: row.status,
                    task_description,
                    channel: row.channel,
                    chat_id,
                    profile_id,
                    profile_name,
                    status_detail,
                    action_history: serde_json::from_str(&action_history_json).unwrap_or_default(),
                    created_at: row.created_at,
                    updated_at: row.updated_at,
                })
            })
            .collect()
    }

    pub async fn load_browser_session(
        &self,
        id: &str,
    ) -> Result<Option<crate::core::browser_session::PersistedBrowserSession>> {
        let Some(row) = browser_session::Entity::find_by_id(id.to_string())
            .one(&self.db)
            .await?
        else {
            return Ok(None);
        };
        let task_description = decrypt_storage_string(&row.task_description);
        let chat_id = row.chat_id.map(|value| decrypt_storage_string(&value));
        let profile_id = row.profile_id.map(|value| decrypt_storage_string(&value));
        let profile_name = row.profile_name.map(|value| decrypt_storage_string(&value));
        let status_detail = row
            .status_detail
            .map(|value| decrypt_storage_string(&value));
        let action_history_json = decrypt_storage_string(&row.action_history_json);
        Ok(Some(
            crate::core::browser_session::PersistedBrowserSession {
                id: row.id,
                status: row.status,
                task_description,
                channel: row.channel,
                chat_id,
                profile_id,
                profile_name,
                status_detail,
                action_history: serde_json::from_str(&action_history_json).unwrap_or_default(),
                created_at: row.created_at,
                updated_at: row.updated_at,
            },
        ))
    }

    pub async fn upsert_browser_session(
        &self,
        session: &crate::core::browser_session::PersistedBrowserSession,
    ) -> Result<()> {
        browser_session::Entity::insert(browser_session::ActiveModel {
            id: Set(session.id.clone()),
            status: Set(session.status.clone()),
            task_description: Set(encrypt_storage_string(&session.task_description)?),
            channel: Set(session.channel.clone()),
            chat_id: Set(encrypt_optional_storage_string(session.chat_id.as_deref())?),
            profile_id: Set(encrypt_optional_storage_string(
                session.profile_id.as_deref(),
            )?),
            profile_name: Set(encrypt_optional_storage_string(
                session.profile_name.as_deref(),
            )?),
            status_detail: Set(encrypt_optional_storage_string(
                session.status_detail.as_deref(),
            )?),
            action_history_json: Set(encrypt_storage_string(&serde_json::to_string(
                &session.action_history,
            )?)?),
            created_at: Set(session.created_at.clone()),
            updated_at: Set(session.updated_at.clone()),
        })
        .on_conflict(
            OnConflict::column(browser_session::Column::Id)
                .update_columns([
                    browser_session::Column::Status,
                    browser_session::Column::TaskDescription,
                    browser_session::Column::Channel,
                    browser_session::Column::ChatId,
                    browser_session::Column::ProfileId,
                    browser_session::Column::ProfileName,
                    browser_session::Column::StatusDetail,
                    browser_session::Column::ActionHistoryJson,
                    browser_session::Column::UpdatedAt,
                ])
                .to_owned(),
        )
        .exec(&self.db)
        .await?;
        Ok(())
    }

    pub async fn delete_browser_session(&self, id: &str) -> Result<()> {
        browser_session::Entity::delete_by_id(id.to_string())
            .exec(&self.db)
            .await?;
        Ok(())
    }

    async fn existing_execution_trace_id(&self, trace_id: Option<&str>) -> Result<Option<String>> {
        let Some(trace_id) = trace_id.map(str::trim).filter(|value| !value.is_empty()) else {
            return Ok(None);
        };
        let exists = execution_trace::Entity::find_by_id(trace_id.to_string())
            .count(&self.db)
            .await?
            > 0;
        if exists {
            Ok(Some(trace_id.to_string()))
        } else {
            tracing::debug!(
                trace_id,
                "Skipping execution run trace_id link because the execution trace is not persisted yet"
            );
            Ok(None)
        }
    }

    #[allow(dead_code)]
    pub async fn insert_execution_run(&self, run: &crate::core::ExecutionRun) -> Result<()> {
        let degradation = encrypt_storage_string(&serde_json::to_string(&run.degradation)?)?;
        let last_error = encrypt_optional_storage_string(run.last_error.as_deref())?;
        let result_summary = encrypt_optional_storage_string(run.result_summary.as_deref())?;
        let request_message = encrypt_optional_storage_string(run.request_message.as_deref())?;
        let attempted_models =
            encrypt_storage_string(&serde_json::to_string(&run.attempted_models)?)?;
        let trace_id = self
            .existing_execution_trace_id(run.trace_id.as_deref())
            .await?;

        let insert_result = execution_run::Entity::insert(execution_run::ActiveModel {
            id: Set(run.id.clone()),
            kind: Set(run.kind.clone()),
            request_id: Set(run.request_id.clone()),
            status: Set(run.status.as_str().to_string()),
            current_stage: Set(run.current_stage.clone()),
            lease_owner: Set(run.lease_owner.clone()),
            lease_expires_at: Set(run.lease_expires_at.clone()),
            attempt: Set(run.attempt as i32),
            deadline_at: Set(run.deadline_at.clone()),
            cancellation_requested: Set(run.cancellation_requested),
            degradation: Set(degradation.clone()),
            last_error: Set(last_error.clone()),
            result_summary: Set(result_summary.clone()),
            trace_id: Set(trace_id.clone()),
            conversation_id: Set(run.conversation_id.clone()),
            channel: Set(run.channel.clone()),
            request_message: Set(request_message.clone()),
            attempted_models: Set(attempted_models.clone()),
            created_at: Set(run.created_at.clone()),
            updated_at: Set(run.updated_at.clone()),
        })
        .on_conflict(
            OnConflict::column(execution_run::Column::Id)
                .update_columns([
                    execution_run::Column::RequestId,
                    execution_run::Column::Status,
                    execution_run::Column::CurrentStage,
                    execution_run::Column::LeaseOwner,
                    execution_run::Column::LeaseExpiresAt,
                    execution_run::Column::Attempt,
                    execution_run::Column::DeadlineAt,
                    execution_run::Column::CancellationRequested,
                    execution_run::Column::Degradation,
                    execution_run::Column::LastError,
                    execution_run::Column::ResultSummary,
                    execution_run::Column::TraceId,
                    execution_run::Column::ConversationId,
                    execution_run::Column::Channel,
                    execution_run::Column::RequestMessage,
                    execution_run::Column::AttemptedModels,
                    execution_run::Column::UpdatedAt,
                ])
                .to_owned(),
        )
        .exec(&self.db)
        .await;
        if let Err(error) = insert_result {
            if trace_id.is_some() && is_foreign_key_constraint_error(&error) {
                tracing::warn!(
                    "Retrying execution run upsert '{}' without trace_id after FK failure: {}",
                    run.id,
                    error
                );
                execution_run::Entity::insert(execution_run::ActiveModel {
                    id: Set(run.id.clone()),
                    kind: Set(run.kind.clone()),
                    request_id: Set(run.request_id.clone()),
                    status: Set(run.status.as_str().to_string()),
                    current_stage: Set(run.current_stage.clone()),
                    lease_owner: Set(run.lease_owner.clone()),
                    lease_expires_at: Set(run.lease_expires_at.clone()),
                    attempt: Set(run.attempt as i32),
                    deadline_at: Set(run.deadline_at.clone()),
                    cancellation_requested: Set(run.cancellation_requested),
                    degradation: Set(degradation),
                    last_error: Set(last_error),
                    result_summary: Set(result_summary),
                    trace_id: Set(None),
                    conversation_id: Set(run.conversation_id.clone()),
                    channel: Set(run.channel.clone()),
                    request_message: Set(request_message),
                    attempted_models: Set(attempted_models),
                    created_at: Set(run.created_at.clone()),
                    updated_at: Set(run.updated_at.clone()),
                })
                .on_conflict(
                    OnConflict::column(execution_run::Column::Id)
                        .update_columns([
                            execution_run::Column::RequestId,
                            execution_run::Column::Status,
                            execution_run::Column::CurrentStage,
                            execution_run::Column::LeaseOwner,
                            execution_run::Column::LeaseExpiresAt,
                            execution_run::Column::Attempt,
                            execution_run::Column::DeadlineAt,
                            execution_run::Column::CancellationRequested,
                            execution_run::Column::Degradation,
                            execution_run::Column::LastError,
                            execution_run::Column::ResultSummary,
                            execution_run::Column::TraceId,
                            execution_run::Column::ConversationId,
                            execution_run::Column::Channel,
                            execution_run::Column::RequestMessage,
                            execution_run::Column::AttemptedModels,
                            execution_run::Column::UpdatedAt,
                        ])
                        .to_owned(),
                )
                .exec(&self.db)
                .await?;
            } else {
                return Err(error.into());
            }
        }
        Ok(())
    }

    pub async fn load_execution_run(&self, id: &str) -> Result<Option<crate::core::ExecutionRun>> {
        Ok(execution_run::Entity::find_by_id(id.to_string())
            .one(&self.db)
            .await?
            .map(model_to_execution_run))
    }

    #[allow(dead_code)]
    pub async fn load_execution_run_by_request_id(
        &self,
        request_id: &str,
    ) -> Result<Option<crate::core::ExecutionRun>> {
        Ok(execution_run::Entity::find()
            .filter(execution_run::Column::RequestId.eq(request_id.to_string()))
            .order_by_desc(execution_run::Column::UpdatedAt)
            .one(&self.db)
            .await?
            .map(model_to_execution_run))
    }

    pub async fn load_execution_run_by_trace_id(
        &self,
        trace_id: &str,
    ) -> Result<Option<crate::core::ExecutionRun>> {
        Ok(execution_run::Entity::find()
            .filter(execution_run::Column::TraceId.eq(trace_id.to_string()))
            .order_by_desc(execution_run::Column::UpdatedAt)
            .one(&self.db)
            .await?
            .map(model_to_execution_run))
    }

    pub async fn list_execution_runs_for_conversation(
        &self,
        conversation_id: &str,
        limit: u64,
    ) -> Result<Vec<crate::core::ExecutionRun>> {
        let capped_limit = limit.clamp(1, 50);
        Ok(execution_run::Entity::find()
            .filter(execution_run::Column::ConversationId.eq(conversation_id.to_string()))
            .order_by_desc(execution_run::Column::UpdatedAt)
            .limit(capped_limit)
            .all(&self.db)
            .await?
            .into_iter()
            .map(model_to_execution_run)
            .collect())
    }

    pub async fn list_recent_execution_runs(
        &self,
        limit: u64,
    ) -> Result<Vec<crate::core::ExecutionRun>> {
        let capped_limit = limit.clamp(1, 100);
        Ok(execution_run::Entity::find()
            .order_by_desc(execution_run::Column::UpdatedAt)
            .limit(capped_limit)
            .all(&self.db)
            .await?
            .into_iter()
            .map(model_to_execution_run)
            .collect())
    }

    pub async fn append_execution_checkpoint(
        &self,
        checkpoint: &crate::core::ExecutionCheckpoint,
    ) -> Result<()> {
        run_checkpoint::Entity::insert(run_checkpoint::ActiveModel {
            id: sea_orm::NotSet,
            run_id: Set(checkpoint.run_id.clone()),
            sequence_no: Set(checkpoint.sequence_no as i32),
            stage: Set(checkpoint.stage.clone()),
            payload: Set(encrypt_storage_string(&checkpoint.payload)?),
            created_at: Set(checkpoint.created_at.clone()),
        })
        .on_conflict(
            OnConflict::columns([
                run_checkpoint::Column::RunId,
                run_checkpoint::Column::SequenceNo,
            ])
            .update_columns([
                run_checkpoint::Column::Stage,
                run_checkpoint::Column::Payload,
                run_checkpoint::Column::CreatedAt,
            ])
            .to_owned(),
        )
        .exec(&self.db)
        .await?;
        Ok(())
    }

    pub async fn load_execution_checkpoints(
        &self,
        run_id: &str,
    ) -> Result<Vec<crate::core::ExecutionCheckpoint>> {
        Ok(run_checkpoint::Entity::find()
            .filter(run_checkpoint::Column::RunId.eq(run_id.to_string()))
            .order_by_asc(run_checkpoint::Column::SequenceNo)
            .all(&self.db)
            .await?
            .into_iter()
            .map(|model| crate::core::ExecutionCheckpoint {
                run_id: model.run_id,
                sequence_no: model.sequence_no.max(0) as u32,
                stage: model.stage,
                payload: decrypt_storage_string(&model.payload),
                created_at: model.created_at,
            })
            .collect())
    }

    #[allow(dead_code)]
    pub async fn append_tool_attempt(&self, attempt: &crate::core::ToolAttempt) -> Result<()> {
        tool_attempt::Entity::insert(tool_attempt::ActiveModel {
            id: Set(attempt.id.clone()),
            run_id: Set(attempt.run_id.clone()),
            sequence_no: Set(attempt.sequence_no as i32),
            tool_name: Set(attempt.tool_name.clone()),
            status: Set(attempt.status.as_str().to_string()),
            failure_class: Set(attempt.failure_class.as_ref().map(|value| {
                serde_json::to_string(value)
                    .unwrap_or_else(|_| "\"platform_error\"".to_string())
                    .trim_matches('"')
                    .to_string()
            })),
            retryable: Set(attempt.retryable),
            side_effect_level: Set(attempt.side_effect_level.clone()),
            idempotency_key: Set(attempt.idempotency_key.clone()),
            arguments_json: Set(encrypt_storage_string(&attempt.arguments_json)?),
            output_json: Set(encrypt_storage_string(&attempt.output_json)?),
            started_at: Set(attempt.started_at.clone()),
            completed_at: Set(attempt.completed_at.clone()),
            error_text: Set(encrypt_optional_storage_string(
                attempt.error_text.as_deref(),
            )?),
        })
        .on_conflict(
            OnConflict::column(tool_attempt::Column::Id)
                .update_columns([
                    tool_attempt::Column::Status,
                    tool_attempt::Column::FailureClass,
                    tool_attempt::Column::Retryable,
                    tool_attempt::Column::SideEffectLevel,
                    tool_attempt::Column::IdempotencyKey,
                    tool_attempt::Column::ArgumentsJson,
                    tool_attempt::Column::OutputJson,
                    tool_attempt::Column::StartedAt,
                    tool_attempt::Column::CompletedAt,
                    tool_attempt::Column::ErrorText,
                ])
                .to_owned(),
        )
        .exec(&self.db)
        .await?;
        Ok(())
    }

    // ==================== Experience Graph ====================

    pub async fn upsert_experience_run(&self, run: &experience_run::Model) -> Result<()> {
        let txn = self.db.begin().await?;
        experience_run::Entity::insert(experience_run::ActiveModel {
            id: Set(run.id.clone()),
            execution_run_id: Set(run.execution_run_id.clone()),
            trace_id: Set(run.trace_id.clone()),
            conversation_id: Set(run.conversation_id.clone()),
            project_id: Set(run.project_id.clone()),
            channel: Set(run.channel.clone()),
            scope: Set(run.scope.clone()),
            intent_key: Set(run.intent_key.clone()),
            task_type: Set(run.task_type.clone()),
            request_text: Set(run.request_text.clone()),
            tool_sequence_digest: Set(run.tool_sequence_digest.clone()),
            tool_sequence_json: Set(run.tool_sequence_json.clone()),
            strategy_version: Set(run.strategy_version.clone()),
            policy_version: Set(run.policy_version.clone()),
            prompt_version: Set(run.prompt_version.clone()),
            model_slot: Set(run.model_slot.clone()),
            success_state: Set(run.success_state.clone()),
            correction_state: Set(run.correction_state.clone()),
            outcome_summary: Set(run.outcome_summary.clone()),
            failure_reason: Set(run.failure_reason.clone()),
            metadata: Set(run.metadata.clone()),
            consolidated: Set(run.consolidated),
            accepted_at: Set(run.accepted_at.clone()),
            corrected_at: Set(run.corrected_at.clone()),
            heuristic_reflected: Set(run.heuristic_reflected),
            heuristic_reflection_status: Set(run.heuristic_reflection_status.clone()),
            heuristic_reflection_attempted_at: Set(run.heuristic_reflection_attempted_at.clone()),
            heuristic_reflection_completed_at: Set(run.heuristic_reflection_completed_at.clone()),
            heuristic_lesson_id: Set(run.heuristic_lesson_id.clone()),
            heuristic_reflection_error: Set(run.heuristic_reflection_error.clone()),
            created_at: Set(run.created_at.clone()),
            updated_at: Set(run.updated_at.clone()),
        })
        .on_conflict(
            OnConflict::column(experience_run::Column::Id)
                .update_columns(EXPERIENCE_RUN_LIGHT_UPSERT_COLUMNS.iter().copied())
                .to_owned(),
        )
        .exec(&txn)
        .await?;
        let current = experience_run::Entity::find_by_id(run.id.clone())
            .lock_exclusive()
            .one(&txn)
            .await?
            .ok_or_else(|| anyhow!("Experience run '{}' missing after upsert", run.id))?;
        if let Some(model) = Self::experience_run_heavy_update_active_model(&current, run) {
            model.update(&txn).await?;
        }
        txn.commit().await?;
        Ok(())
    }

    pub async fn list_tool_attempts_for_run(
        &self,
        run_id: &str,
    ) -> Result<Vec<crate::core::ToolAttempt>> {
        Ok(tool_attempt::Entity::find()
            .filter(tool_attempt::Column::RunId.eq(run_id.to_string()))
            .order_by_asc(tool_attempt::Column::SequenceNo)
            .all(&self.db)
            .await?
            .into_iter()
            .map(model_to_tool_attempt)
            .collect())
    }

    pub async fn append_readiness_evaluation(
        &self,
        evaluation: &readiness_evaluation::Model,
    ) -> Result<()> {
        readiness_evaluation::Entity::insert(readiness_evaluation::ActiveModel {
            id: Set(evaluation.id.clone()),
            target_type: Set(evaluation.target_type.clone()),
            target_id: Set(evaluation.target_id.clone()),
            score: Set(evaluation.score),
            stage: Set(evaluation.stage.clone()),
            allows_review: Set(evaluation.allows_review),
            allows_auto: Set(evaluation.allows_auto),
            reasons_json: Set(evaluation.reasons_json.clone()),
            blockers_json: Set(evaluation.blockers_json.clone()),
            signals_json: Set(evaluation.signals_json.clone()),
            policy_version: Set(evaluation.policy_version.clone()),
            created_at: Set(evaluation.created_at.clone()),
        })
        .exec(&self.db)
        .await?;
        Ok(())
    }

    #[allow(dead_code)]
    pub async fn mark_latest_provisional_experience_run_corrected(
        &self,
        conversation_id: &str,
        correction_signal: &str,
        within_minutes: i64,
    ) -> Result<Option<experience_run::Model>> {
        let now = chrono::Utc::now().to_rfc3339();
        let cutoff =
            (chrono::Utc::now() - chrono::Duration::minutes(within_minutes.max(1))).to_rfc3339();
        let payload = serde_json::json!({
            "correction_signal": correction_signal,
            "correction_recorded_at": now,
        });
        let candidates = experience_run::Entity::find()
            .filter(experience_run::Column::ConversationId.eq(conversation_id.to_string()))
            .filter(experience_run::Column::SuccessState.eq("provisional"))
            .filter(experience_run::Column::CorrectionState.eq("none"))
            .filter(experience_run::Column::CreatedAt.gte(cutoff))
            .order_by_desc(experience_run::Column::CreatedAt)
            .limit(2)
            .all(&self.db)
            .await?;
        if candidates.len() != 1 {
            return Ok(None);
        }
        let target = candidates
            .into_iter()
            .next()
            .expect("exactly one correction candidate");

        let mut metadata = target.metadata.clone();
        if let Some(existing) = metadata.as_object_mut() {
            if let Some(payload_map) = payload.as_object() {
                for (key, value) in payload_map {
                    existing.insert(key.clone(), value.clone());
                }
            }
        } else {
            metadata = payload;
        }

        let updated = experience_run::ActiveModel {
            id: Unchanged(target.id),
            success_state: Set(if target.success_state == "provisional" {
                "failed".to_string()
            } else {
                target.success_state
            }),
            correction_state: Set("corrected".to_string()),
            corrected_at: Set(Some(now.clone())),
            updated_at: Set(now),
            metadata: Set(metadata),
            ..Default::default()
        }
        .update(&self.db)
        .await?;
        Ok(Some(updated))
    }

    #[allow(dead_code)]
    pub async fn mark_provisional_experience_run_corrected_by_trace_id(
        &self,
        trace_id: &str,
        correction_signal: &str,
    ) -> Result<Option<experience_run::Model>> {
        let now = chrono::Utc::now().to_rfc3339();
        let payload = serde_json::json!({
            "correction_signal": correction_signal,
            "correction_recorded_at": now,
            "correction_bound_by": "trace_id",
        });
        let candidates = experience_run::Entity::find()
            .filter(experience_run::Column::TraceId.eq(trace_id.to_string()))
            .filter(experience_run::Column::SuccessState.eq("provisional"))
            .filter(experience_run::Column::CorrectionState.eq("none"))
            .limit(2)
            .all(&self.db)
            .await?;
        if candidates.len() != 1 {
            return Ok(None);
        }
        let target = candidates
            .into_iter()
            .next()
            .expect("exactly one trace-bound correction candidate");

        let mut metadata = target.metadata.clone();
        if let Some(existing) = metadata.as_object_mut() {
            if let Some(payload_map) = payload.as_object() {
                for (key, value) in payload_map {
                    existing.insert(key.clone(), value.clone());
                }
            }
        } else {
            metadata = payload;
        }

        let updated = experience_run::ActiveModel {
            id: Unchanged(target.id),
            success_state: Set(if target.success_state == "provisional" {
                "failed".to_string()
            } else {
                target.success_state
            }),
            correction_state: Set("corrected".to_string()),
            corrected_at: Set(Some(now.clone())),
            updated_at: Set(now),
            metadata: Set(metadata),
            ..Default::default()
        }
        .update(&self.db)
        .await?;
        Ok(Some(updated))
    }

    pub async fn finalize_stale_provisional_experience_runs(
        &self,
        older_than_minutes: i64,
        limit: u64,
    ) -> Result<u64> {
        let cutoff = (chrono::Utc::now() - chrono::Duration::minutes(older_than_minutes.max(1)))
            .to_rfc3339();
        let now = chrono::Utc::now().to_rfc3339();
        let target_ids = experience_run::Entity::find()
            .select_only()
            .column(experience_run::Column::Id)
            .filter(experience_run::Column::SuccessState.eq("provisional"))
            .filter(experience_run::Column::CorrectionState.eq("none"))
            .filter(experience_run::Column::CreatedAt.lt(cutoff))
            .order_by_asc(experience_run::Column::CreatedAt)
            .limit(Self::db_limit(
                limit.min(Self::MAX_EXPERIENCE_RUN_ROWS_PER_QUERY),
            ))
            .into_tuple::<String>()
            .all(&self.db)
            .await?;
        if target_ids.is_empty() {
            return Ok(0);
        }
        let result = experience_run::Entity::update_many()
            .col_expr(
                experience_run::Column::SuccessState,
                Expr::value("accepted".to_string()),
            )
            .col_expr(
                experience_run::Column::AcceptedAt,
                Expr::value(Some(now.clone())),
            )
            .col_expr(experience_run::Column::UpdatedAt, Expr::value(now))
            .filter(experience_run::Column::Id.is_in(target_ids))
            .exec(&self.db)
            .await?;
        Ok(result.rows_affected)
    }

    pub async fn list_experience_runs_for_consolidation(
        &self,
        limit: u64,
    ) -> Result<Vec<experience_run::Model>> {
        Ok(experience_run::Entity::find()
            .filter(experience_run::Column::Consolidated.eq(false))
            .filter(
                Condition::any()
                    .add(experience_run::Column::SuccessState.ne("provisional"))
                    .add(experience_run::Column::CorrectionState.eq("corrected")),
            )
            .order_by_asc(experience_run::Column::CreatedAt)
            .limit(Self::db_limit(limit))
            .all(&self.db)
            .await?)
    }

    pub async fn list_recent_experience_runs_any_scope(
        &self,
        limit: u64,
    ) -> Result<Vec<experience_run::Model>> {
        let capped_limit = limit.min(Self::MAX_EXPERIENCE_RUN_ROWS_PER_QUERY);
        experience_run::Entity::find()
            .order_by_desc(experience_run::Column::UpdatedAt)
            .limit(Self::db_limit(capped_limit))
            .all(&self.db)
            .await
            .map_err(Into::into)
    }

    pub async fn get_experience_run(&self, id: &str) -> Result<Option<experience_run::Model>> {
        Ok(experience_run::Entity::find_by_id(id.to_string())
            .one(&self.db)
            .await?)
    }

    pub async fn list_experience_runs_for_heuristic_reflection(
        &self,
        limit: u64,
    ) -> Result<Vec<experience_run::Model>> {
        Ok(experience_run::Entity::find()
            .filter(experience_run::Column::Consolidated.eq(true))
            .filter(experience_run::Column::HeuristicReflected.eq(false))
            .filter(
                Condition::any()
                    .add(experience_run::Column::HeuristicReflectionStatus.is_null())
                    .add(experience_run::Column::HeuristicReflectionStatus.eq("pending")),
            )
            .order_by_asc(experience_run::Column::UpdatedAt)
            .limit(Self::db_limit(
                limit.min(Self::MAX_EXPERIENCE_RUN_ROWS_PER_QUERY),
            ))
            .all(&self.db)
            .await?)
    }

    pub async fn mark_experience_run_consolidated(&self, id: &str) -> Result<()> {
        experience_run::Entity::update_many()
            .col_expr(experience_run::Column::Consolidated, Expr::value(true))
            .col_expr(
                experience_run::Column::UpdatedAt,
                Expr::value(chrono::Utc::now().to_rfc3339()),
            )
            .filter(experience_run::Column::Id.eq(id))
            .exec(&self.db)
            .await?;
        Ok(())
    }

    pub async fn mark_experience_run_heuristic_reflection_started(&self, id: &str) -> Result<()> {
        let now = chrono::Utc::now().to_rfc3339();
        experience_run::Entity::update_many()
            .col_expr(
                experience_run::Column::HeuristicReflectionStatus,
                Expr::value(Option::<String>::Some("pending".to_string())),
            )
            .col_expr(
                experience_run::Column::HeuristicReflectionAttemptedAt,
                Expr::value(Option::<String>::Some(now.clone())),
            )
            .col_expr(experience_run::Column::UpdatedAt, Expr::value(now))
            .filter(experience_run::Column::Id.eq(id))
            .exec(&self.db)
            .await?;
        Ok(())
    }

    pub async fn mark_experience_run_heuristic_reflection_completed(
        &self,
        id: &str,
        lesson_id: &str,
    ) -> Result<()> {
        let now = chrono::Utc::now().to_rfc3339();
        experience_run::Entity::update_many()
            .col_expr(
                experience_run::Column::HeuristicReflected,
                Expr::value(true),
            )
            .col_expr(
                experience_run::Column::HeuristicReflectionStatus,
                Expr::value(Option::<String>::Some("completed".to_string())),
            )
            .col_expr(
                experience_run::Column::HeuristicReflectionCompletedAt,
                Expr::value(Option::<String>::Some(now.clone())),
            )
            .col_expr(
                experience_run::Column::HeuristicLessonId,
                Expr::value(Option::<String>::Some(lesson_id.to_string())),
            )
            .col_expr(
                experience_run::Column::HeuristicReflectionError,
                Expr::value(Option::<String>::None),
            )
            .col_expr(experience_run::Column::UpdatedAt, Expr::value(now))
            .filter(experience_run::Column::Id.eq(id))
            .exec(&self.db)
            .await?;
        Ok(())
    }

    pub async fn mark_experience_run_heuristic_reflection_skipped(
        &self,
        id: &str,
        reason: &str,
    ) -> Result<()> {
        let now = chrono::Utc::now().to_rfc3339();
        experience_run::Entity::update_many()
            .col_expr(
                experience_run::Column::HeuristicReflected,
                Expr::value(true),
            )
            .col_expr(
                experience_run::Column::HeuristicReflectionStatus,
                Expr::value(Option::<String>::Some("skipped".to_string())),
            )
            .col_expr(
                experience_run::Column::HeuristicReflectionCompletedAt,
                Expr::value(Option::<String>::Some(now.clone())),
            )
            .col_expr(
                experience_run::Column::HeuristicReflectionError,
                Expr::value(Option::<String>::Some(reason.to_string())),
            )
            .col_expr(experience_run::Column::UpdatedAt, Expr::value(now))
            .filter(experience_run::Column::Id.eq(id))
            .exec(&self.db)
            .await?;
        Ok(())
    }

    pub async fn mark_experience_run_heuristic_reflection_failed(
        &self,
        id: &str,
        error: &str,
    ) -> Result<()> {
        let now = chrono::Utc::now().to_rfc3339();
        experience_run::Entity::update_many()
            .col_expr(
                experience_run::Column::HeuristicReflected,
                Expr::value(false),
            )
            .col_expr(
                experience_run::Column::HeuristicReflectionStatus,
                Expr::value(Option::<String>::Some("failed".to_string())),
            )
            .col_expr(
                experience_run::Column::HeuristicReflectionError,
                Expr::value(Option::<String>::Some(error.to_string())),
            )
            .col_expr(experience_run::Column::UpdatedAt, Expr::value(now))
            .filter(experience_run::Column::Id.eq(id))
            .exec(&self.db)
            .await?;
        Ok(())
    }

    fn experience_item_active_model(item: &experience_item::Model) -> experience_item::ActiveModel {
        experience_item::ActiveModel {
            id: Set(item.id.clone()),
            kind: Set(item.kind.clone()),
            scope: Set(item.scope.clone()),
            project_id: Set(item.project_id.clone()),
            conversation_id: Set(item.conversation_id.clone()),
            title: Set(item.title.clone()),
            content: Set(item.content.clone()),
            normalized_key: Set(item.normalized_key.clone()),
            confidence: Set(item.confidence),
            support_count: Set(item.support_count),
            contradiction_count: Set(item.contradiction_count),
            status: Set(item.status.clone()),
            metadata: Set(item.metadata.clone()),
            last_supported_at: Set(item.last_supported_at.clone()),
            last_contradicted_at: Set(item.last_contradicted_at.clone()),
            created_at: Set(item.created_at.clone()),
            updated_at: Set(item.updated_at.clone()),
            embedding: Set(item.embedding.clone()),
        }
    }

    fn semantic_work_unit_active_model(
        unit: &semantic_work_unit::Model,
    ) -> Result<semantic_work_unit::ActiveModel> {
        Ok(semantic_work_unit::ActiveModel {
            id: Set(unit.id.clone()),
            source_kind: Set(unit.source_kind.clone()),
            source_id: Set(unit.source_id.clone()),
            conversation_id: Set(unit.conversation_id.clone()),
            project_id: Set(unit.project_id.clone()),
            channel: Set(unit.channel.clone()),
            title: Set(encrypt_storage_string(&unit.title)?),
            summary: Set(encrypt_storage_string(&unit.summary)?),
            content_preview: Set(encrypt_storage_string(&unit.content_preview)?),
            text_hash: Set(unit.text_hash.clone()),
            occurred_at: Set(unit.occurred_at.clone()),
            period_start: Set(unit.period_start.clone()),
            period_end: Set(unit.period_end.clone()),
            message_count: Set(unit.message_count),
            metadata: Set(unit.metadata.clone()),
            created_at: Set(unit.created_at.clone()),
            updated_at: Set(unit.updated_at.clone()),
            embedding: Set(unit.embedding.clone()),
        })
    }

    fn decrypt_semantic_work_unit(
        mut unit: semantic_work_unit::Model,
    ) -> semantic_work_unit::Model {
        unit.title = decrypt_storage_string(&unit.title);
        unit.summary = decrypt_storage_string(&unit.summary);
        unit.content_preview = decrypt_storage_string(&unit.content_preview);
        unit
    }

    fn recall_event_active_model(event: &recall_event::Model) -> recall_event::ActiveModel {
        recall_event::ActiveModel {
            id: Set(event.id.clone()),
            event_type: Set(event.event_type.clone()),
            memory_id: Set(event.memory_id.clone()),
            related_memory_id: Set(event.related_memory_id.clone()),
            scope: Set(event.scope.clone()),
            project_id: Set(event.project_id.clone()),
            conversation_id: Set(event.conversation_id.clone()),
            source_kind: Set(event.source_kind.clone()),
            source_ref: Set(event.source_ref.clone()),
            actor: Set(event.actor.clone()),
            summary: Set(event.summary.clone()),
            old_snapshot: Set(event.old_snapshot.clone()),
            new_snapshot: Set(event.new_snapshot.clone()),
            metadata: Set(event.metadata.clone()),
            risk_level: Set(event.risk_level.clone()),
            confidence: Set(event.confidence),
            reversible: Set(event.reversible),
            reverted_at: Set(event.reverted_at.clone()),
            created_at: Set(event.created_at.clone()),
            updated_at: Set(event.updated_at.clone()),
        }
    }

    fn recall_test_active_model(test: &recall_test::Model) -> recall_test::ActiveModel {
        recall_test::ActiveModel {
            id: Set(test.id.clone()),
            memory_id: Set(test.memory_id.clone()),
            scope: Set(test.scope.clone()),
            project_id: Set(test.project_id.clone()),
            conversation_id: Set(test.conversation_id.clone()),
            prompt: Set(test.prompt.clone()),
            expected_answer: Set(test.expected_answer.clone()),
            status: Set(test.status.clone()),
            last_answer: Set(test.last_answer.clone()),
            last_run_at: Set(test.last_run_at.clone()),
            metadata: Set(test.metadata.clone()),
            created_at: Set(test.created_at.clone()),
            updated_at: Set(test.updated_at.clone()),
        }
    }

    fn memory_capture_event_active_model(
        event: &memory_capture_event::Model,
    ) -> memory_capture_event::ActiveModel {
        memory_capture_event::ActiveModel {
            id: Set(event.id.clone()),
            source_message_id: Set(event.source_message_id.clone()),
            conversation_id: Set(event.conversation_id.clone()),
            project_id: Set(event.project_id.clone()),
            channel: Set(event.channel.clone()),
            status: Set(event.status.clone()),
            capture_kind: Set(event.capture_kind.clone()),
            source_hash: Set(event.source_hash.clone()),
            attempt_metadata: Set(event.attempt_metadata.clone()),
            error_history: Set(event.error_history.clone()),
            replay_count: Set(event.replay_count),
            next_retry_at: Set(event.next_retry_at.clone()),
            completed_at: Set(event.completed_at.clone()),
            created_at: Set(event.created_at.clone()),
            updated_at: Set(event.updated_at.clone()),
        }
    }

    fn memory_operation_active_model(
        operation: &memory_operation::Model,
    ) -> Result<memory_operation::ActiveModel> {
        Ok(memory_operation::ActiveModel {
            id: Set(operation.id.clone()),
            capture_event_id: Set(operation.capture_event_id.clone()),
            operation_type: Set(operation.operation_type.clone()),
            status: Set(operation.status.clone()),
            target_memory_id: Set(operation.target_memory_id.clone()),
            applied_memory_id: Set(operation.applied_memory_id.clone()),
            key: Set(operation.key.clone()),
            value: Set(encrypt_optional_storage_string(operation.value.as_deref())?),
            memory_kind: Set(operation.memory_kind.clone()),
            durability: Set(operation.durability.clone()),
            scope: Set(operation.scope.clone()),
            project_id: Set(operation.project_id.clone()),
            conversation_id: Set(operation.conversation_id.clone()),
            confidence: Set(operation.confidence),
            looks_sensitive: Set(operation.looks_sensitive),
            sensitive_reason: Set(encrypt_optional_storage_string(
                operation.sensitive_reason.as_deref(),
            )?),
            valid_from: Set(operation.valid_from.clone()),
            expires_at: Set(operation.expires_at.clone()),
            review_at: Set(operation.review_at.clone()),
            rationale: Set(encrypt_optional_storage_string(
                operation.rationale.as_deref(),
            )?),
            evidence_refs: Set(operation.evidence_refs.clone()),
            model_metadata: Set(operation.model_metadata.clone()),
            apply_metadata: Set(operation.apply_metadata.clone()),
            applied_at: Set(operation.applied_at.clone()),
            reviewed_at: Set(operation.reviewed_at.clone()),
            review_notes: Set(encrypt_optional_storage_string(
                operation.review_notes.as_deref(),
            )?),
            created_at: Set(operation.created_at.clone()),
            updated_at: Set(operation.updated_at.clone()),
        })
    }

    fn experience_run_heavy_update_active_model(
        current: &experience_run::Model,
        next: &experience_run::Model,
    ) -> Option<experience_run::ActiveModel> {
        let mut model = experience_run::ActiveModel {
            id: Unchanged(next.id.clone()),
            ..Default::default()
        };
        let mut changed = false;

        set_if_changed(
            &mut model.request_text,
            &current.request_text,
            &next.request_text,
            &mut changed,
        );
        set_if_changed(
            &mut model.tool_sequence_json,
            &current.tool_sequence_json,
            &next.tool_sequence_json,
            &mut changed,
        );
        set_if_changed(
            &mut model.outcome_summary,
            &current.outcome_summary,
            &next.outcome_summary,
            &mut changed,
        );
        set_if_changed(
            &mut model.failure_reason,
            &current.failure_reason,
            &next.failure_reason,
            &mut changed,
        );
        set_if_changed(
            &mut model.metadata,
            &current.metadata,
            &next.metadata,
            &mut changed,
        );
        set_if_changed(
            &mut model.heuristic_reflection_error,
            &current.heuristic_reflection_error,
            &next.heuristic_reflection_error,
            &mut changed,
        );

        changed.then_some(model)
    }

    fn experience_item_heavy_update_active_model(
        current: &experience_item::Model,
        next: &experience_item::Model,
    ) -> Option<experience_item::ActiveModel> {
        let mut model = experience_item::ActiveModel {
            id: Unchanged(next.id.clone()),
            ..Default::default()
        };
        let mut changed = false;

        set_if_changed(
            &mut model.content,
            &current.content,
            &next.content,
            &mut changed,
        );
        set_if_changed(
            &mut model.metadata,
            &current.metadata,
            &next.metadata,
            &mut changed,
        );
        set_if_changed(
            &mut model.embedding,
            &current.embedding,
            &next.embedding,
            &mut changed,
        );

        changed.then_some(model)
    }

    fn memory_capture_event_heavy_update_active_model(
        current: &memory_capture_event::Model,
        next: &memory_capture_event::Model,
    ) -> Option<memory_capture_event::ActiveModel> {
        let mut model = memory_capture_event::ActiveModel {
            id: Unchanged(next.id.clone()),
            ..Default::default()
        };
        let mut changed = false;

        set_if_changed(
            &mut model.attempt_metadata,
            &current.attempt_metadata,
            &next.attempt_metadata,
            &mut changed,
        );
        set_if_changed(
            &mut model.error_history,
            &current.error_history,
            &next.error_history,
            &mut changed,
        );

        changed.then_some(model)
    }

    fn memory_operation_heavy_update_active_model(
        current: &memory_operation::Model,
        next: &memory_operation::Model,
    ) -> Result<Option<memory_operation::ActiveModel>> {
        let mut model = memory_operation::ActiveModel {
            id: Unchanged(next.id.clone()),
            ..Default::default()
        };
        let mut changed = false;

        set_encrypted_optional_string_if_changed(
            &mut model.value,
            &current.value,
            &next.value,
            &mut changed,
        )?;
        set_encrypted_optional_string_if_changed(
            &mut model.sensitive_reason,
            &current.sensitive_reason,
            &next.sensitive_reason,
            &mut changed,
        )?;
        set_encrypted_optional_string_if_changed(
            &mut model.rationale,
            &current.rationale,
            &next.rationale,
            &mut changed,
        )?;
        set_if_changed(
            &mut model.evidence_refs,
            &current.evidence_refs,
            &next.evidence_refs,
            &mut changed,
        );
        set_if_changed(
            &mut model.model_metadata,
            &current.model_metadata,
            &next.model_metadata,
            &mut changed,
        );
        set_if_changed(
            &mut model.apply_metadata,
            &current.apply_metadata,
            &next.apply_metadata,
            &mut changed,
        );
        set_encrypted_optional_string_if_changed(
            &mut model.review_notes,
            &current.review_notes,
            &next.review_notes,
            &mut changed,
        )?;

        Ok(changed.then_some(model))
    }

    fn semantic_work_unit_heavy_update_active_model(
        current: &semantic_work_unit::Model,
        next: &semantic_work_unit::Model,
    ) -> Result<Option<semantic_work_unit::ActiveModel>> {
        let mut model = semantic_work_unit::ActiveModel {
            id: Unchanged(next.id.clone()),
            ..Default::default()
        };
        let mut changed = false;

        set_encrypted_string_if_changed(
            &mut model.title,
            &current.title,
            &next.title,
            &mut changed,
        )?;
        set_encrypted_string_if_changed(
            &mut model.summary,
            &current.summary,
            &next.summary,
            &mut changed,
        )?;
        set_encrypted_string_if_changed(
            &mut model.content_preview,
            &current.content_preview,
            &next.content_preview,
            &mut changed,
        )?;
        set_if_changed(
            &mut model.metadata,
            &current.metadata,
            &next.metadata,
            &mut changed,
        );
        set_if_changed(
            &mut model.embedding,
            &current.embedding,
            &next.embedding,
            &mut changed,
        );

        Ok(changed.then_some(model))
    }

    fn memory_evidence_link_active_model(
        link: &memory_evidence_link::Model,
    ) -> memory_evidence_link::ActiveModel {
        memory_evidence_link::ActiveModel {
            id: Set(link.id.clone()),
            operation_id: Set(link.operation_id.clone()),
            memory_id: Set(link.memory_id.clone()),
            evidence_kind: Set(link.evidence_kind.clone()),
            evidence_ref: Set(link.evidence_ref.clone()),
            source_message_id: Set(link.source_message_id.clone()),
            capture_event_id: Set(link.capture_event_id.clone()),
            project_id: Set(link.project_id.clone()),
            conversation_id: Set(link.conversation_id.clone()),
            metadata: Set(link.metadata.clone()),
            created_at: Set(link.created_at.clone()),
        }
    }

    fn experience_item_is_arkmemory_memory(item: &experience_item::Model) -> bool {
        matches!(item.kind.as_str(), "personal_fact" | "constraint")
    }

    fn recall_snapshot_experience_item(item: &experience_item::Model) -> Result<serde_json::Value> {
        let mut value = serde_json::to_value(item)?;
        if let Some(object) = value.as_object_mut() {
            object.insert("embedding".to_string(), serde_json::Value::Null);
        }
        Ok(value)
    }

    fn experience_item_recall_event_type(
        previous: Option<&experience_item::Model>,
        next: &experience_item::Model,
    ) -> Option<&'static str> {
        if !Self::experience_item_is_arkmemory_memory(next) {
            return None;
        }
        let Some(previous) = previous else {
            return Some("memory_created");
        };
        if previous.status != next.status {
            return Some("memory_status_changed");
        }
        if previous.content != next.content
            || previous.title != next.title
            || previous.normalized_key != next.normalized_key
            || previous.scope != next.scope
            || previous.project_id != next.project_id
            || previous.conversation_id != next.conversation_id
        {
            return Some("memory_updated");
        }
        None
    }

    async fn insert_recall_event_conn<C>(conn: &C, event: &recall_event::Model) -> Result<()>
    where
        C: ConnectionTrait,
    {
        recall_event::Entity::insert(Self::recall_event_active_model(event))
            .on_conflict(
                OnConflict::column(recall_event::Column::Id)
                    .update_columns([
                        recall_event::Column::EventType,
                        recall_event::Column::MemoryId,
                        recall_event::Column::RelatedMemoryId,
                        recall_event::Column::Scope,
                        recall_event::Column::ProjectId,
                        recall_event::Column::ConversationId,
                        recall_event::Column::SourceKind,
                        recall_event::Column::SourceRef,
                        recall_event::Column::Actor,
                        recall_event::Column::Summary,
                        recall_event::Column::OldSnapshot,
                        recall_event::Column::NewSnapshot,
                        recall_event::Column::Metadata,
                        recall_event::Column::RiskLevel,
                        recall_event::Column::Confidence,
                        recall_event::Column::Reversible,
                        recall_event::Column::RevertedAt,
                        recall_event::Column::UpdatedAt,
                    ])
                    .to_owned(),
            )
            .exec(conn)
            .await?;
        Ok(())
    }

    async fn record_experience_item_recall_event_conn<C>(
        conn: &C,
        event_type: &str,
        previous: Option<&experience_item::Model>,
        next: &experience_item::Model,
        actor: &str,
        metadata: serde_json::Value,
    ) -> Result<()>
    where
        C: ConnectionTrait,
    {
        let now = chrono::Utc::now().to_rfc3339();
        let summary = match event_type {
            "memory_created" => format!("Created {}", next.title),
            "memory_status_changed" => format!("Changed {} status to {}", next.title, next.status),
            "memory_updated" => format!("Updated {}", next.title),
            _ => format!("Recorded {}", next.title),
        };
        let event = recall_event::Model {
            id: uuid::Uuid::new_v4().to_string(),
            event_type: event_type.to_string(),
            memory_id: Some(next.id.clone()),
            related_memory_id: None,
            scope: Some(next.scope.clone()),
            project_id: next.project_id.clone(),
            conversation_id: next.conversation_id.clone(),
            source_kind: Some("experience_item".to_string()),
            source_ref: Some(next.id.clone()),
            actor: actor.to_string(),
            summary: Some(summary),
            old_snapshot: previous
                .map(Self::recall_snapshot_experience_item)
                .transpose()?
                .unwrap_or(serde_json::Value::Null),
            new_snapshot: Self::recall_snapshot_experience_item(next)?,
            metadata,
            risk_level: None,
            confidence: Some(next.confidence),
            reversible: previous.is_some(),
            reverted_at: None,
            created_at: now.clone(),
            updated_at: now,
        };
        Self::insert_recall_event_conn(conn, &event).await
    }

    async fn upsert_experience_item_conn<C>(conn: &C, item: &experience_item::Model) -> Result<()>
    where
        C: ConnectionTrait,
    {
        let previous = experience_item::Entity::find_by_id(item.id.clone())
            .one(conn)
            .await?;
        experience_item::Entity::insert(Self::experience_item_active_model(item))
            .on_conflict(
                OnConflict::column(experience_item::Column::Id)
                    .update_columns(EXPERIENCE_ITEM_LIGHT_UPSERT_COLUMNS.iter().copied())
                    .to_owned(),
            )
            .exec(conn)
            .await?;
        let current = experience_item::Entity::find_by_id(item.id.clone())
            .lock_exclusive()
            .one(conn)
            .await?
            .ok_or_else(|| anyhow!("Experience item '{}' missing after upsert", item.id))?;
        if let Some(model) = Self::experience_item_heavy_update_active_model(&current, item) {
            model.update(conn).await?;
        }
        if let Some(event_type) = Self::experience_item_recall_event_type(previous.as_ref(), item) {
            Self::record_experience_item_recall_event_conn(
                conn,
                event_type,
                previous.as_ref(),
                item,
                "system",
                serde_json::json!({ "origin": "experience_item_upsert" }),
            )
            .await?;
        }
        Ok(())
    }

    async fn update_experience_item_status_conn<C>(conn: &C, id: &str, status: &str) -> Result<()>
    where
        C: ConnectionTrait,
    {
        let previous = experience_item::Entity::find_by_id(id.to_string())
            .one(conn)
            .await?;
        let now = chrono::Utc::now().to_rfc3339();
        experience_item::Entity::update_many()
            .col_expr(
                experience_item::Column::Status,
                Expr::value(status.to_string()),
            )
            .col_expr(experience_item::Column::UpdatedAt, Expr::value(now.clone()))
            .filter(experience_item::Column::Id.eq(id))
            .exec(conn)
            .await?;
        if let Some(previous_item) = previous.as_ref() {
            let mut next = previous_item.clone();
            next.status = status.to_string();
            next.updated_at = now;
            if let Some(event_type) =
                Self::experience_item_recall_event_type(Some(previous_item), &next)
            {
                Self::record_experience_item_recall_event_conn(
                    conn,
                    event_type,
                    Some(previous_item),
                    &next,
                    "system",
                    serde_json::json!({ "origin": "experience_item_status_update" }),
                )
                .await?;
            }
        }
        Ok(())
    }

    async fn get_experience_item_conn<C>(
        conn: &C,
        id: &str,
    ) -> Result<Option<experience_item::Model>>
    where
        C: ConnectionTrait,
    {
        Ok(experience_item::Entity::find_by_id(id.to_string())
            .one(conn)
            .await?)
    }

    pub async fn upsert_experience_item(&self, item: &experience_item::Model) -> Result<()> {
        let txn = self.db.begin().await?;
        Self::upsert_experience_item_conn(&txn, item).await?;
        txn.commit().await?;
        Ok(())
    }

    pub(crate) async fn upsert_experience_item_txn(
        &self,
        txn: &DatabaseTransaction,
        item: &experience_item::Model,
    ) -> Result<()> {
        Self::upsert_experience_item_conn(txn, item).await
    }

    pub async fn update_experience_item_status(&self, id: &str, status: &str) -> Result<()> {
        let txn = self.db.begin().await?;
        Self::update_experience_item_status_conn(&txn, id, status).await?;
        txn.commit().await?;
        Ok(())
    }

    pub async fn update_experience_item_content(
        &self,
        id: &str,
        content: &str,
    ) -> Result<Option<experience_item::Model>> {
        let id = id.trim();
        if id.is_empty() {
            return Ok(None);
        }
        let txn = self.db.begin().await?;
        let Some(previous) = experience_item::Entity::find_by_id(id.to_string())
            .one(&txn)
            .await?
        else {
            txn.commit().await?;
            return Ok(None);
        };
        let mut next = previous.clone();
        next.content = content.to_string();
        next.updated_at = chrono::Utc::now().to_rfc3339();
        if previous.content != next.content {
            next.embedding = None;
        }
        Self::upsert_experience_item_conn(&txn, &next).await?;
        txn.commit().await?;
        Ok(Some(next))
    }

    pub async fn list_experience_items_between(
        &self,
        from: &str,
        to: &str,
        limit: u64,
    ) -> Result<Vec<experience_item::Model>> {
        Ok(experience_item::Entity::find()
            .filter(experience_item::Column::UpdatedAt.gte(from.to_string()))
            .filter(experience_item::Column::UpdatedAt.lt(to.to_string()))
            .filter(experience_item::Column::Status.eq("active"))
            .order_by_desc(experience_item::Column::UpdatedAt)
            .limit(Self::db_limit(limit))
            .all(&self.db)
            .await?)
    }

    pub(crate) async fn begin_experience_memory_write_txn(
        &self,
        kind: &str,
        scope: &str,
        project_id: Option<&str>,
        conversation_id: Option<&str>,
    ) -> Result<DatabaseTransaction> {
        let txn = self.db.begin().await?;
        self.acquire_experience_memory_write_lock_txn(
            &txn,
            kind,
            scope,
            project_id,
            conversation_id,
        )
        .await?;
        Ok(txn)
    }

    pub(crate) async fn acquire_experience_memory_write_lock_txn(
        &self,
        txn: &DatabaseTransaction,
        kind: &str,
        scope: &str,
        project_id: Option<&str>,
        conversation_id: Option<&str>,
    ) -> Result<()> {
        if txn.get_database_backend() == DbBackend::Postgres {
            let lock_key =
                experience_memory_write_lock_key(kind, scope, project_id, conversation_id);
            txn.execute(Statement::from_sql_and_values(
                DbBackend::Postgres,
                "SELECT pg_advisory_xact_lock($1)",
                vec![lock_key.into()],
            ))
            .await?;
        }
        Ok(())
    }

    /// Cosine-distance nearest-neighbour lookup over active experience items,
    /// scoped to the provided kinds and scope tuple. Returns (model, distance)
    /// pairs in ascending distance order (closest first). Distance is the
    /// pgvector cosine distance: 0.0 is identical, 1.0 is orthogonal, 2.0 is
    /// diametrically opposite. Callers convert to cosine similarity as
    /// `1.0 - distance` when scoring against a threshold.
    async fn nearest_active_experience_items_semantic_conn<C>(
        conn: &C,
        kinds: &[&str],
        scope: &str,
        project_id: Option<&str>,
        conversation_id: Option<&str>,
        embedding: &PgVector,
        limit: u64,
    ) -> Result<Vec<(experience_item::Model, f64)>>
    where
        C: ConnectionTrait,
    {
        if limit == 0 || kinds.is_empty() {
            return Ok(Vec::new());
        }
        if conn.get_database_backend() != DbBackend::Postgres {
            return Ok(Vec::new());
        }
        let embedding_sql = pgvector_sql_literal(embedding);
        let kinds_list = sql_string_list(
            &kinds
                .iter()
                .map(|kind| (*kind).to_string())
                .collect::<Vec<_>>(),
        );
        let scope_filter = format!("scope = {}", sql_string_literal(scope));
        let project_filter = match project_id {
            Some(value) => format!("project_id = {}", sql_string_literal(value)),
            None => "project_id IS NULL".to_string(),
        };
        let conversation_filter = match conversation_id {
            Some(value) => format!("conversation_id = {}", sql_string_literal(value)),
            None => "conversation_id IS NULL".to_string(),
        };
        let sql = format!(
            "SELECT id, embedding <=> {embedding_sql} AS cosine_distance \
             FROM experience_items \
             WHERE status = 'active' \
               AND embedding IS NOT NULL \
               AND kind IN ({kinds_list}) \
               AND {scope_filter} \
               AND {project_filter} \
               AND {conversation_filter} \
             ORDER BY embedding <=> {embedding_sql} ASC \
             LIMIT {}",
            Self::db_limit(limit),
        );
        let rows = conn
            .query_all(Statement::from_string(DbBackend::Postgres, sql))
            .await?;
        let mut scored: Vec<(String, f64)> = Vec::with_capacity(rows.len());
        for row in rows {
            let id: String = row.try_get("", "id")?;
            let distance: f64 = row.try_get("", "cosine_distance")?;
            scored.push((id, distance));
        }
        if scored.is_empty() {
            return Ok(Vec::new());
        }
        let ids = scored.iter().map(|(id, _)| id.clone()).collect::<Vec<_>>();
        let models = experience_item::Entity::find()
            .filter(experience_item::Column::Id.is_in(ids.clone()))
            .all(conn)
            .await?;
        let mut by_id: std::collections::HashMap<String, experience_item::Model> = models
            .into_iter()
            .map(|model| (model.id.clone(), model))
            .collect();
        Ok(scored
            .into_iter()
            .filter_map(|(id, distance)| by_id.remove(&id).map(|model| (model, distance)))
            .collect())
    }

    pub(crate) async fn nearest_active_experience_items_semantic_txn(
        &self,
        txn: &DatabaseTransaction,
        kinds: &[&str],
        scope: &str,
        project_id: Option<&str>,
        conversation_id: Option<&str>,
        embedding: &PgVector,
        limit: u64,
    ) -> Result<Vec<(experience_item::Model, f64)>> {
        Self::nearest_active_experience_items_semantic_conn(
            txn,
            kinds,
            scope,
            project_id,
            conversation_id,
            embedding,
            limit,
        )
        .await
    }

    pub async fn get_experience_item(&self, id: &str) -> Result<Option<experience_item::Model>> {
        Self::get_experience_item_conn(&self.db, id).await
    }

    pub async fn hard_delete_experience_item_memory(&self, id: &str) -> Result<bool> {
        let id = id.trim();
        if id.is_empty() {
            return Ok(false);
        }
        let txn = self.db.begin().await?;
        let Some(_) = experience_item::Entity::find_by_id(id.to_string())
            .one(&txn)
            .await?
        else {
            txn.commit().await?;
            return Ok(false);
        };
        let operation_rows = memory_operation::Entity::find()
            .filter(
                Condition::any()
                    .add(memory_operation::Column::TargetMemoryId.eq(id.to_string()))
                    .add(memory_operation::Column::AppliedMemoryId.eq(id.to_string())),
            )
            .all(&txn)
            .await?;
        let operation_ids = operation_rows
            .iter()
            .map(|operation| operation.id.clone())
            .collect::<Vec<_>>();
        memory_evidence_link::Entity::delete_many()
            .filter(memory_evidence_link::Column::MemoryId.eq(id.to_string()))
            .exec(&txn)
            .await?;
        if !operation_ids.is_empty() {
            memory_evidence_link::Entity::delete_many()
                .filter(memory_evidence_link::Column::OperationId.is_in(operation_ids.clone()))
                .exec(&txn)
                .await?;
        }
        memory_operation::Entity::delete_many()
            .filter(
                Condition::any()
                    .add(memory_operation::Column::TargetMemoryId.eq(id.to_string()))
                    .add(memory_operation::Column::AppliedMemoryId.eq(id.to_string())),
            )
            .exec(&txn)
            .await?;
        learning_candidate::Entity::delete_many()
            .filter(learning_candidate::Column::ApprovedRef.eq(id.to_string()))
            .exec(&txn)
            .await?;
        if !operation_ids.is_empty() {
            let candidate_ids = operation_ids
                .iter()
                .map(|operation_id| format!("memory-candidate-{operation_id}"))
                .collect::<Vec<_>>();
            learning_candidate::Entity::delete_many()
                .filter(learning_candidate::Column::Id.is_in(candidate_ids))
                .exec(&txn)
                .await?;
        }
        experience_edge::Entity::delete_many()
            .filter(
                Condition::any()
                    .add(
                        Condition::all()
                            .add(experience_edge::Column::SourceKind.eq("experience_item"))
                            .add(experience_edge::Column::SourceRef.eq(id.to_string())),
                    )
                    .add(
                        Condition::all()
                            .add(experience_edge::Column::TargetKind.eq("experience_item"))
                            .add(experience_edge::Column::TargetRef.eq(id.to_string())),
                    ),
            )
            .exec(&txn)
            .await?;
        recall_event::Entity::delete_many()
            .filter(
                Condition::any()
                    .add(recall_event::Column::MemoryId.eq(id.to_string()))
                    .add(recall_event::Column::RelatedMemoryId.eq(id.to_string()))
                    .add(
                        Condition::all()
                            .add(recall_event::Column::SourceKind.eq("experience_item"))
                            .add(recall_event::Column::SourceRef.eq(id.to_string())),
                    ),
            )
            .exec(&txn)
            .await?;
        let result = experience_item::Entity::delete_by_id(id.to_string())
            .exec(&txn)
            .await?;
        txn.commit().await?;
        Ok(result.rows_affected > 0)
    }

    pub async fn insert_recall_event(&self, event: &recall_event::Model) -> Result<()> {
        Self::insert_recall_event_conn(&self.db, event).await
    }

    pub async fn get_recall_event(&self, id: &str) -> Result<Option<recall_event::Model>> {
        Ok(recall_event::Entity::find_by_id(id.to_string())
            .one(&self.db)
            .await?)
    }

    pub async fn list_recall_events(
        &self,
        limit: u64,
        offset: u64,
        project_id: Option<&str>,
    ) -> Result<Vec<recall_event::Model>> {
        let mut query = recall_event::Entity::find().order_by_desc(recall_event::Column::CreatedAt);
        query = match project_id.map(str::trim).filter(|value| !value.is_empty()) {
            Some(pid) => query.filter(
                Condition::any()
                    .add(recall_event::Column::ProjectId.is_null())
                    .add(recall_event::Column::ProjectId.eq(pid.to_string())),
            ),
            None => query.filter(recall_event::Column::ProjectId.is_null()),
        };
        Ok(query
            .limit(Self::db_limit(limit))
            .offset(offset)
            .all(&self.db)
            .await?)
    }

    pub async fn list_recall_events_for_memory(
        &self,
        memory_id: &str,
        limit: u64,
        project_id: Option<&str>,
    ) -> Result<Vec<recall_event::Model>> {
        let mut query = recall_event::Entity::find().filter(
            Condition::any()
                .add(recall_event::Column::MemoryId.eq(memory_id.to_string()))
                .add(recall_event::Column::RelatedMemoryId.eq(memory_id.to_string())),
        );
        query = match project_id.map(str::trim).filter(|value| !value.is_empty()) {
            Some(pid) => query.filter(
                Condition::any()
                    .add(recall_event::Column::ProjectId.is_null())
                    .add(recall_event::Column::ProjectId.eq(pid.to_string())),
            ),
            None => query.filter(recall_event::Column::ProjectId.is_null()),
        };
        Ok(query
            .order_by_desc(recall_event::Column::CreatedAt)
            .limit(Self::db_limit(limit))
            .all(&self.db)
            .await?)
    }

    pub async fn list_reverted_recall_events(
        &self,
        limit: u64,
        project_id: Option<&str>,
    ) -> Result<Vec<recall_event::Model>> {
        let mut query = recall_event::Entity::find()
            .filter(recall_event::Column::RevertedAt.is_not_null())
            .order_by_desc(recall_event::Column::CreatedAt);
        query = match project_id.map(str::trim).filter(|value| !value.is_empty()) {
            Some(pid) => query.filter(
                Condition::any()
                    .add(recall_event::Column::ProjectId.is_null())
                    .add(recall_event::Column::ProjectId.eq(pid.to_string())),
            ),
            None => query.filter(recall_event::Column::ProjectId.is_null()),
        };
        Ok(query.limit(Self::db_limit(limit)).all(&self.db).await?)
    }

    pub async fn count_recall_events(&self, project_id: Option<&str>) -> Result<u64> {
        let mut query = recall_event::Entity::find();
        query = match project_id.map(str::trim).filter(|value| !value.is_empty()) {
            Some(pid) => query.filter(
                Condition::any()
                    .add(recall_event::Column::ProjectId.is_null())
                    .add(recall_event::Column::ProjectId.eq(pid.to_string())),
            ),
            None => query.filter(recall_event::Column::ProjectId.is_null()),
        };
        Ok(query.count(&self.db).await?)
    }

    pub async fn rollback_recall_event_with_memory_snapshot(
        &self,
        event_id: &str,
        previous_memory: &experience_item::Model,
        rollback_event: &recall_event::Model,
    ) -> Result<bool> {
        let txn = self.db.begin().await?;
        let now = chrono::Utc::now().to_rfc3339();
        let result = recall_event::Entity::update_many()
            .col_expr(
                recall_event::Column::RevertedAt,
                Expr::value(Some(now.clone())),
            )
            .col_expr(recall_event::Column::UpdatedAt, Expr::value(now))
            .filter(recall_event::Column::Id.eq(event_id.to_string()))
            .filter(recall_event::Column::Reversible.eq(true))
            .filter(recall_event::Column::RevertedAt.is_null())
            .exec(&txn)
            .await?;
        if result.rows_affected == 0 {
            txn.rollback().await?;
            return Ok(false);
        }
        Self::upsert_experience_item_conn(&txn, previous_memory).await?;
        Self::insert_recall_event_conn(&txn, rollback_event).await?;
        txn.commit().await?;
        Ok(true)
    }

    pub async fn upsert_recall_test(&self, test: &recall_test::Model) -> Result<()> {
        recall_test::Entity::insert(Self::recall_test_active_model(test))
            .on_conflict(
                OnConflict::column(recall_test::Column::Id)
                    .update_columns([
                        recall_test::Column::MemoryId,
                        recall_test::Column::Scope,
                        recall_test::Column::ProjectId,
                        recall_test::Column::ConversationId,
                        recall_test::Column::Prompt,
                        recall_test::Column::ExpectedAnswer,
                        recall_test::Column::Status,
                        recall_test::Column::LastAnswer,
                        recall_test::Column::LastRunAt,
                        recall_test::Column::Metadata,
                        recall_test::Column::UpdatedAt,
                    ])
                    .to_owned(),
            )
            .exec(&self.db)
            .await?;
        Ok(())
    }

    pub async fn list_recall_tests(
        &self,
        limit: u64,
        offset: u64,
        project_id: Option<&str>,
    ) -> Result<Vec<recall_test::Model>> {
        let mut query = recall_test::Entity::find().order_by_desc(recall_test::Column::UpdatedAt);
        query = match project_id.map(str::trim).filter(|value| !value.is_empty()) {
            Some(pid) => query.filter(
                Condition::any()
                    .add(recall_test::Column::ProjectId.is_null())
                    .add(recall_test::Column::ProjectId.eq(pid.to_string())),
            ),
            None => query.filter(recall_test::Column::ProjectId.is_null()),
        };
        Ok(query
            .limit(Self::db_limit(limit))
            .offset(offset)
            .all(&self.db)
            .await?)
    }

    pub async fn count_recall_tests(&self, project_id: Option<&str>) -> Result<u64> {
        let mut query = recall_test::Entity::find();
        query = match project_id.map(str::trim).filter(|value| !value.is_empty()) {
            Some(pid) => query.filter(
                Condition::any()
                    .add(recall_test::Column::ProjectId.is_null())
                    .add(recall_test::Column::ProjectId.eq(pid.to_string())),
            ),
            None => query.filter(recall_test::Column::ProjectId.is_null()),
        };
        Ok(query.count(&self.db).await?)
    }

    pub async fn list_experience_edges_for_item(
        &self,
        item_id: &str,
        limit: u64,
    ) -> Result<Vec<experience_edge::Model>> {
        let capped = Self::db_limit(limit);
        Ok(experience_edge::Entity::find()
            .filter(
                Condition::any()
                    .add(
                        Condition::all()
                            .add(experience_edge::Column::SourceKind.eq("experience_item"))
                            .add(experience_edge::Column::SourceRef.eq(item_id.to_string())),
                    )
                    .add(
                        Condition::all()
                            .add(experience_edge::Column::TargetKind.eq("experience_item"))
                            .add(experience_edge::Column::TargetRef.eq(item_id.to_string())),
                    ),
            )
            .order_by_desc(experience_edge::Column::UpdatedAt)
            .limit(capped)
            .all(&self.db)
            .await?)
    }

    pub(crate) async fn get_experience_item_txn(
        &self,
        txn: &DatabaseTransaction,
        id: &str,
    ) -> Result<Option<experience_item::Model>> {
        Self::get_experience_item_conn(txn, id).await
    }

    pub async fn list_active_experience_items(
        &self,
        kinds: &[&str],
        project_id: Option<&str>,
        conversation_id: Option<&str>,
        limit: u64,
    ) -> Result<Vec<experience_item::Model>> {
        let mut query =
            experience_item::Entity::find().filter(experience_item::Column::Status.eq("active"));
        query = match conversation_id {
            Some(value) => query.filter(
                Condition::any()
                    .add(experience_item::Column::ConversationId.is_null())
                    .add(experience_item::Column::ConversationId.eq(value.to_string())),
            ),
            None => query.filter(experience_item::Column::ConversationId.is_null()),
        };
        query = match project_id {
            Some(value) => query.filter(
                Condition::any()
                    .add(experience_item::Column::ProjectId.is_null())
                    .add(experience_item::Column::ProjectId.eq(value.to_string())),
            ),
            None => query.filter(experience_item::Column::ProjectId.is_null()),
        };
        if !kinds.is_empty() {
            query = query.filter(
                experience_item::Column::Kind.is_in(
                    kinds
                        .iter()
                        .map(|kind| (*kind).to_string())
                        .collect::<Vec<_>>(),
                ),
            );
        }
        let capped_limit = limit.min(Self::MAX_EXPERIENCE_ITEM_ROWS_PER_QUERY);
        let mut items = query
            .order_by_desc(experience_item::Column::UpdatedAt)
            .limit(Self::db_limit(capped_limit))
            .all(&self.db)
            .await?;
        items.sort_by(|left, right| {
            scope_match_rank(
                right.project_id.as_deref(),
                right.conversation_id.as_deref(),
                project_id,
                conversation_id,
            )
            .cmp(&scope_match_rank(
                left.project_id.as_deref(),
                left.conversation_id.as_deref(),
                project_id,
                conversation_id,
            ))
            .then_with(|| {
                experience_item_kind_rank(&left.kind).cmp(&experience_item_kind_rank(&right.kind))
            })
            .then_with(|| right.confidence.total_cmp(&left.confidence))
            .then_with(|| right.support_count.cmp(&left.support_count))
            .then_with(|| right.updated_at.cmp(&left.updated_at))
        });
        items.truncate(capped_limit as usize);
        Ok(items)
    }

    pub async fn list_active_experience_items_any_scope(
        &self,
        kinds: &[&str],
        limit: u64,
    ) -> Result<Vec<experience_item::Model>> {
        let mut query =
            experience_item::Entity::find().filter(experience_item::Column::Status.eq("active"));
        if !kinds.is_empty() {
            query = query.filter(
                experience_item::Column::Kind.is_in(
                    kinds
                        .iter()
                        .map(|kind| (*kind).to_string())
                        .collect::<Vec<_>>(),
                ),
            );
        }
        let capped_limit = limit.min(Self::MAX_EXPERIENCE_ITEM_ROWS_PER_QUERY);
        let mut items = query
            .order_by_desc(experience_item::Column::UpdatedAt)
            .limit(Self::db_limit(capped_limit))
            .all(&self.db)
            .await?;
        items.sort_by(|left, right| {
            experience_item_kind_rank(&left.kind)
                .cmp(&experience_item_kind_rank(&right.kind))
                .then_with(|| right.confidence.total_cmp(&left.confidence))
                .then_with(|| right.support_count.cmp(&left.support_count))
                .then_with(|| right.updated_at.cmp(&left.updated_at))
        });
        items.truncate(capped_limit as usize);
        Ok(items)
    }

    pub async fn list_memory_experience_items_for_graph(
        &self,
        statuses: &[String],
        project_id: Option<&str>,
        limit: u64,
    ) -> Result<Vec<experience_item::Model>> {
        let mut query = experience_item::Entity::find().filter(
            experience_item::Column::Kind
                .is_in(["personal_fact".to_string(), "constraint".to_string()]),
        );
        if !statuses.is_empty() {
            query = query.filter(experience_item::Column::Status.is_in(statuses.to_vec()));
        }
        query = match project_id.map(str::trim).filter(|value| !value.is_empty()) {
            Some(pid) => query.filter(
                Condition::any()
                    .add(experience_item::Column::ProjectId.is_null())
                    .add(experience_item::Column::ProjectId.eq(pid.to_string())),
            ),
            None => query.filter(experience_item::Column::ProjectId.is_null()),
        };
        let capped_limit = limit.min(Self::MAX_EXPERIENCE_ITEM_ROWS_PER_QUERY);
        let mut items = query
            .order_by_desc(experience_item::Column::UpdatedAt)
            .limit(Self::db_limit(capped_limit))
            .all(&self.db)
            .await?;
        items.sort_by(|left, right| {
            experience_item_kind_rank(&left.kind)
                .cmp(&experience_item_kind_rank(&right.kind))
                .then_with(|| right.confidence.total_cmp(&left.confidence))
                .then_with(|| right.support_count.cmp(&left.support_count))
                .then_with(|| right.updated_at.cmp(&left.updated_at))
        });
        items.truncate(capped_limit as usize);
        Ok(items)
    }

    pub async fn search_experience_items(
        &self,
        query: &str,
        kinds: &[&str],
        project_id: Option<&str>,
        conversation_id: Option<&str>,
        limit: u64,
    ) -> Result<Vec<ExperienceItemSearchHit>> {
        let terms = normalized_search_terms(query);
        if terms.is_empty() {
            return Ok(Vec::new());
        }
        let mut items = self
            .list_active_experience_items(kinds, project_id, conversation_id, limit)
            .await?;
        let mut hits = Vec::new();
        for item in items.drain(..) {
            if !matches_search_terms(&terms, &[&item.title, &item.content]) {
                continue;
            }
            let score = search_score(&terms, &[(&item.title, 3.0), (&item.content, 1.0)]);
            hits.push(ExperienceItemSearchHit { item, score });
        }
        hits.sort_by(|left, right| {
            scope_match_rank(
                right.item.project_id.as_deref(),
                right.item.conversation_id.as_deref(),
                project_id,
                conversation_id,
            )
            .cmp(&scope_match_rank(
                left.item.project_id.as_deref(),
                left.item.conversation_id.as_deref(),
                project_id,
                conversation_id,
            ))
            .then_with(|| {
                experience_item_kind_rank(&left.item.kind)
                    .cmp(&experience_item_kind_rank(&right.item.kind))
            })
            .then_with(|| right.score.total_cmp(&left.score))
            .then_with(|| right.item.support_count.cmp(&left.item.support_count))
            .then_with(|| right.item.updated_at.cmp(&left.item.updated_at))
        });
        hits.truncate(limit.min(Self::MAX_EXPERIENCE_ITEM_ROWS_PER_QUERY) as usize);
        Ok(hits)
    }

    pub async fn upsert_experience_edge(&self, edge: &experience_edge::Model) -> Result<()> {
        experience_edge::Entity::insert(experience_edge::ActiveModel {
            id: Set(edge.id.clone()),
            source_ref: Set(edge.source_ref.clone()),
            source_kind: Set(edge.source_kind.clone()),
            target_ref: Set(edge.target_ref.clone()),
            target_kind: Set(edge.target_kind.clone()),
            edge_type: Set(edge.edge_type.clone()),
            weight: Set(edge.weight),
            source_run_id: Set(edge.source_run_id.clone()),
            metadata: Set(edge.metadata.clone()),
            created_at: Set(edge.created_at.clone()),
            updated_at: Set(edge.updated_at.clone()),
        })
        .on_conflict(
            OnConflict::column(experience_edge::Column::Id)
                .update_columns([
                    experience_edge::Column::SourceRef,
                    experience_edge::Column::SourceKind,
                    experience_edge::Column::TargetRef,
                    experience_edge::Column::TargetKind,
                    experience_edge::Column::EdgeType,
                    experience_edge::Column::Weight,
                    experience_edge::Column::SourceRunId,
                    experience_edge::Column::Metadata,
                    experience_edge::Column::UpdatedAt,
                ])
                .to_owned(),
        )
        .exec(&self.db)
        .await?;
        Ok(())
    }

    #[allow(dead_code)]
    pub async fn list_related_experience_items(
        &self,
        seed_refs: &[String],
        limit: u64,
    ) -> Result<Vec<experience_item::Model>> {
        if seed_refs.is_empty() {
            return Ok(Vec::new());
        }
        let seed_refs_vec = seed_refs.to_vec();
        let edges = experience_edge::Entity::find()
            .filter(
                Condition::any()
                    .add(
                        Condition::all()
                            .add(experience_edge::Column::SourceRef.is_in(seed_refs_vec.clone()))
                            .add(experience_edge::Column::TargetKind.eq("experience_item")),
                    )
                    .add(
                        Condition::all()
                            .add(experience_edge::Column::TargetRef.is_in(seed_refs_vec.clone()))
                            .add(experience_edge::Column::SourceKind.eq("experience_item")),
                    ),
            )
            .limit(Self::db_limit(
                Self::MAX_RELATED_EXPERIENCE_EDGE_ROWS_PER_QUERY.max(limit),
            ))
            .all(&self.db)
            .await?;
        let seed_set = seed_refs
            .iter()
            .cloned()
            .collect::<std::collections::HashSet<_>>();
        let related_ids = edges
            .into_iter()
            .filter_map(|edge| {
                if seed_set.contains(&edge.source_ref) && edge.target_kind == "experience_item" {
                    Some(edge.target_ref)
                } else if seed_set.contains(&edge.target_ref)
                    && edge.source_kind == "experience_item"
                {
                    Some(edge.source_ref)
                } else {
                    None
                }
            })
            .filter(|id| !seed_set.contains(id))
            .collect::<std::collections::HashSet<_>>()
            .into_iter()
            .collect::<Vec<_>>();
        if related_ids.is_empty() {
            return Ok(Vec::new());
        }
        let mut items = experience_item::Entity::find()
            .filter(experience_item::Column::Id.is_in(related_ids))
            .filter(experience_item::Column::Status.eq("active"))
            .all(&self.db)
            .await?;
        items.sort_by(|left, right| {
            right
                .support_count
                .cmp(&left.support_count)
                .then_with(|| right.confidence.total_cmp(&left.confidence))
                .then_with(|| right.updated_at.cmp(&left.updated_at))
        });
        items.truncate(limit.min(Self::MAX_EXPERIENCE_ITEM_ROWS_PER_QUERY) as usize);
        Ok(items)
    }

    pub async fn upsert_procedural_pattern(
        &self,
        pattern: &procedural_pattern::Model,
    ) -> Result<()> {
        procedural_pattern::Entity::insert(procedural_pattern::ActiveModel {
            id: Set(pattern.id.clone()),
            intent_key: Set(pattern.intent_key.clone()),
            scope: Set(pattern.scope.clone()),
            project_id: Set(pattern.project_id.clone()),
            conversation_id: Set(pattern.conversation_id.clone()),
            title: Set(pattern.title.clone()),
            trigger_summary: Set(pattern.trigger_summary.clone()),
            summary: Set(pattern.summary.clone()),
            tool_sequence_digest: Set(pattern.tool_sequence_digest.clone()),
            steps_json: Set(pattern.steps_json.clone()),
            tool_sequence_json: Set(pattern.tool_sequence_json.clone()),
            sample_count: Set(pattern.sample_count),
            success_count: Set(pattern.success_count),
            correction_count: Set(pattern.correction_count),
            success_rate: Set(pattern.success_rate),
            last_validated_at: Set(pattern.last_validated_at.clone()),
            status: Set(pattern.status.clone()),
            metadata: Set(pattern.metadata.clone()),
            created_at: Set(pattern.created_at.clone()),
            updated_at: Set(pattern.updated_at.clone()),
        })
        .on_conflict(
            OnConflict::column(procedural_pattern::Column::Id)
                .update_columns([
                    procedural_pattern::Column::IntentKey,
                    procedural_pattern::Column::Scope,
                    procedural_pattern::Column::ProjectId,
                    procedural_pattern::Column::ConversationId,
                    procedural_pattern::Column::Title,
                    procedural_pattern::Column::TriggerSummary,
                    procedural_pattern::Column::Summary,
                    procedural_pattern::Column::ToolSequenceDigest,
                    procedural_pattern::Column::StepsJson,
                    procedural_pattern::Column::ToolSequenceJson,
                    procedural_pattern::Column::SampleCount,
                    procedural_pattern::Column::SuccessCount,
                    procedural_pattern::Column::CorrectionCount,
                    procedural_pattern::Column::SuccessRate,
                    procedural_pattern::Column::LastValidatedAt,
                    procedural_pattern::Column::Status,
                    procedural_pattern::Column::Metadata,
                    procedural_pattern::Column::UpdatedAt,
                ])
                .to_owned(),
        )
        .exec(&self.db)
        .await?;
        Ok(())
    }

    pub async fn list_procedural_patterns_between(
        &self,
        from: &str,
        to: &str,
        limit: u64,
    ) -> Result<Vec<procedural_pattern::Model>> {
        Ok(procedural_pattern::Entity::find()
            .filter(procedural_pattern::Column::UpdatedAt.gte(from.to_string()))
            .filter(procedural_pattern::Column::UpdatedAt.lt(to.to_string()))
            .filter(procedural_pattern::Column::Status.eq("active"))
            .order_by_desc(procedural_pattern::Column::UpdatedAt)
            .limit(Self::db_limit(limit))
            .all(&self.db)
            .await?)
    }

    #[allow(dead_code)]
    pub async fn search_procedural_patterns(
        &self,
        query: &str,
        project_id: Option<&str>,
        conversation_id: Option<&str>,
        limit: u64,
    ) -> Result<Vec<ProceduralPatternSearchHit>> {
        let terms = normalized_search_terms(query);
        if terms.is_empty() {
            return Ok(Vec::new());
        }
        let mut patterns = self
            .list_procedural_patterns(project_id, conversation_id, &["active", "draft"], limit)
            .await?;
        let mut hits = Vec::new();
        for pattern in patterns.drain(..) {
            if !matches_search_terms(
                &terms,
                &[&pattern.title, &pattern.trigger_summary, &pattern.summary],
            ) {
                continue;
            }
            let score = search_score(
                &terms,
                &[
                    (&pattern.title, 3.0),
                    (&pattern.trigger_summary, 2.0),
                    (&pattern.summary, 1.0),
                ],
            );
            hits.push(ProceduralPatternSearchHit { pattern, score });
        }
        hits.sort_by(|left, right| {
            scope_match_rank(
                right.pattern.project_id.as_deref(),
                right.pattern.conversation_id.as_deref(),
                project_id,
                conversation_id,
            )
            .cmp(&scope_match_rank(
                left.pattern.project_id.as_deref(),
                left.pattern.conversation_id.as_deref(),
                project_id,
                conversation_id,
            ))
            .then_with(|| right.score.total_cmp(&left.score))
            .then_with(|| right.pattern.sample_count.cmp(&left.pattern.sample_count))
            .then_with(|| {
                right
                    .pattern
                    .success_rate
                    .total_cmp(&left.pattern.success_rate)
            })
            .then_with(|| right.pattern.updated_at.cmp(&left.pattern.updated_at))
        });
        hits.truncate(limit.min(Self::MAX_PROCEDURAL_PATTERN_ROWS_PER_QUERY) as usize);
        Ok(hits)
    }

    pub async fn list_candidate_ready_patterns(
        &self,
        min_samples: i32,
        min_success_rate: f64,
        limit: u64,
    ) -> Result<Vec<procedural_pattern::Model>> {
        Ok(procedural_pattern::Entity::find()
            .filter(procedural_pattern::Column::SampleCount.gte(min_samples))
            .filter(procedural_pattern::Column::SuccessRate.gte(min_success_rate))
            .filter(procedural_pattern::Column::Status.is_in(["active", "draft"]))
            .order_by_desc(procedural_pattern::Column::SuccessRate)
            .order_by_desc(procedural_pattern::Column::SampleCount)
            .order_by_desc(procedural_pattern::Column::UpdatedAt)
            .limit(Self::db_limit(
                limit.min(Self::MAX_PROCEDURAL_PATTERN_ROWS_PER_QUERY),
            ))
            .all(&self.db)
            .await?)
    }

    pub async fn get_procedural_pattern(
        &self,
        id: &str,
    ) -> Result<Option<procedural_pattern::Model>> {
        Ok(procedural_pattern::Entity::find_by_id(id.to_string())
            .one(&self.db)
            .await?)
    }

    pub async fn list_procedural_patterns(
        &self,
        project_id: Option<&str>,
        conversation_id: Option<&str>,
        statuses: &[&str],
        limit: u64,
    ) -> Result<Vec<procedural_pattern::Model>> {
        let mut query = procedural_pattern::Entity::find();
        query = match conversation_id {
            Some(value) => query.filter(
                Condition::any()
                    .add(procedural_pattern::Column::ConversationId.is_null())
                    .add(procedural_pattern::Column::ConversationId.eq(value.to_string())),
            ),
            None => query.filter(procedural_pattern::Column::ConversationId.is_null()),
        };
        query = match project_id {
            Some(value) => query.filter(
                Condition::any()
                    .add(procedural_pattern::Column::ProjectId.is_null())
                    .add(procedural_pattern::Column::ProjectId.eq(value.to_string())),
            ),
            None => query.filter(procedural_pattern::Column::ProjectId.is_null()),
        };
        if !statuses.is_empty() {
            query = query.filter(
                procedural_pattern::Column::Status.is_in(
                    statuses
                        .iter()
                        .map(|status| (*status).to_string())
                        .collect::<Vec<_>>(),
                ),
            );
        }

        let capped_limit = limit.min(Self::MAX_PROCEDURAL_PATTERN_ROWS_PER_QUERY);
        let mut patterns = query
            .limit(Self::db_limit(capped_limit))
            .all(&self.db)
            .await?;
        patterns.sort_by(|left, right| {
            scope_match_rank(
                right.project_id.as_deref(),
                right.conversation_id.as_deref(),
                project_id,
                conversation_id,
            )
            .cmp(&scope_match_rank(
                left.project_id.as_deref(),
                left.conversation_id.as_deref(),
                project_id,
                conversation_id,
            ))
            .then_with(|| {
                procedural_pattern_status_rank(&right.status)
                    .cmp(&procedural_pattern_status_rank(&left.status))
            })
            .then_with(|| right.sample_count.cmp(&left.sample_count))
            .then_with(|| right.success_rate.total_cmp(&left.success_rate))
            .then_with(|| right.updated_at.cmp(&left.updated_at))
        });
        patterns.truncate(capped_limit as usize);
        Ok(patterns)
    }

    pub async fn list_procedural_patterns_any_scope(
        &self,
        statuses: &[&str],
        limit: u64,
    ) -> Result<Vec<procedural_pattern::Model>> {
        let mut query = procedural_pattern::Entity::find();
        if !statuses.is_empty() {
            query = query.filter(
                procedural_pattern::Column::Status.is_in(
                    statuses
                        .iter()
                        .map(|status| (*status).to_string())
                        .collect::<Vec<_>>(),
                ),
            );
        }
        let capped_limit = limit.min(Self::MAX_PROCEDURAL_PATTERN_ROWS_PER_QUERY);
        let mut patterns = query
            .limit(Self::db_limit(capped_limit))
            .all(&self.db)
            .await?;
        patterns.sort_by(|left, right| {
            procedural_pattern_status_rank(&right.status)
                .cmp(&procedural_pattern_status_rank(&left.status))
                .then_with(|| right.sample_count.cmp(&left.sample_count))
                .then_with(|| right.success_rate.total_cmp(&left.success_rate))
                .then_with(|| right.updated_at.cmp(&left.updated_at))
        });
        patterns.truncate(capped_limit as usize);
        Ok(patterns)
    }

    pub async fn list_experience_edges_for_refs(
        &self,
        refs: &[String],
        limit: u64,
    ) -> Result<Vec<experience_edge::Model>> {
        if refs.is_empty() {
            return Ok(Vec::new());
        }
        let capped_limit = limit.min(500);
        Ok(experience_edge::Entity::find()
            .filter(
                Condition::any()
                    .add(experience_edge::Column::SourceRef.is_in(refs.to_vec()))
                    .add(experience_edge::Column::TargetRef.is_in(refs.to_vec())),
            )
            .order_by_desc(experience_edge::Column::UpdatedAt)
            .limit(Self::db_limit(capped_limit))
            .all(&self.db)
            .await?)
    }

    pub async fn upsert_learning_candidate_guarded(
        &self,
        lease_key: &str,
        guard: &KvLeaseGuard,
        candidate: &learning_candidate::Model,
    ) -> Result<bool> {
        let txn = self.db.begin().await?;
        if !self
            .require_kv_lease_guard_txn(&txn, lease_key, guard)
            .await?
        {
            txn.rollback().await?;
            return Ok(false);
        }
        self.upsert_learning_candidate_txn(&txn, candidate).await?;
        txn.commit().await?;
        Ok(true)
    }

    pub async fn upsert_learning_candidate(
        &self,
        candidate: &learning_candidate::Model,
    ) -> Result<()> {
        let txn = self.db.begin().await?;
        self.upsert_learning_candidate_txn(&txn, candidate).await?;
        txn.commit().await?;
        Ok(())
    }

    pub async fn upsert_memory_capture_event(
        &self,
        event: &memory_capture_event::Model,
    ) -> Result<()> {
        let txn = self.db.begin().await?;
        memory_capture_event::Entity::insert(Self::memory_capture_event_active_model(event))
            .on_conflict(
                OnConflict::column(memory_capture_event::Column::Id)
                    .update_columns(MEMORY_CAPTURE_EVENT_LIGHT_UPSERT_COLUMNS.iter().copied())
                    .to_owned(),
            )
            .exec(&txn)
            .await?;
        let current = memory_capture_event::Entity::find_by_id(event.id.clone())
            .lock_exclusive()
            .one(&txn)
            .await?
            .ok_or_else(|| anyhow!("Memory capture event '{}' missing after upsert", event.id))?;
        if let Some(model) = Self::memory_capture_event_heavy_update_active_model(&current, event) {
            model.update(&txn).await?;
        }
        txn.commit().await?;
        Ok(())
    }

    pub async fn get_memory_capture_event(
        &self,
        id: &str,
    ) -> Result<Option<memory_capture_event::Model>> {
        Ok(memory_capture_event::Entity::find_by_id(id.to_string())
            .one(&self.db)
            .await?)
    }

    pub async fn list_memory_capture_events_by_statuses(
        &self,
        statuses: &[&str],
        project_id: Option<&str>,
        limit: u64,
    ) -> Result<Vec<memory_capture_event::Model>> {
        let mut query = memory_capture_event::Entity::find();
        if !statuses.is_empty() {
            query = query.filter(
                memory_capture_event::Column::Status.is_in(
                    statuses
                        .iter()
                        .map(|status| (*status).to_string())
                        .collect::<Vec<_>>(),
                ),
            );
        }
        query = match project_id.map(str::trim).filter(|value| !value.is_empty()) {
            Some(pid) => query.filter(
                Condition::any()
                    .add(memory_capture_event::Column::ProjectId.is_null())
                    .add(memory_capture_event::Column::ProjectId.eq(pid.to_string())),
            ),
            None => query.filter(memory_capture_event::Column::ProjectId.is_null()),
        };
        Ok(query
            .order_by_desc(memory_capture_event::Column::UpdatedAt)
            .limit(Self::db_limit(limit))
            .all(&self.db)
            .await?)
    }

    pub async fn list_memory_capture_events_by_statuses_all_scopes(
        &self,
        statuses: &[&str],
        limit: u64,
    ) -> Result<Vec<memory_capture_event::Model>> {
        let mut query = memory_capture_event::Entity::find();
        if !statuses.is_empty() {
            query = query.filter(
                memory_capture_event::Column::Status.is_in(
                    statuses
                        .iter()
                        .map(|status| (*status).to_string())
                        .collect::<Vec<_>>(),
                ),
            );
        }
        Ok(query
            .order_by_asc(memory_capture_event::Column::UpdatedAt)
            .limit(Self::db_limit(limit))
            .all(&self.db)
            .await?)
    }

    pub async fn count_memory_capture_events_by_statuses_all_scopes(
        &self,
        statuses: &[&str],
    ) -> Result<u64> {
        let mut query = memory_capture_event::Entity::find();
        if !statuses.is_empty() {
            query = query.filter(
                memory_capture_event::Column::Status.is_in(
                    statuses
                        .iter()
                        .map(|status| (*status).to_string())
                        .collect::<Vec<_>>(),
                ),
            );
        }
        Ok(query.count(&self.db).await?)
    }

    pub async fn count_memory_capture_events_by_source_hash(
        &self,
        source_hash: &str,
    ) -> Result<u64> {
        Ok(memory_capture_event::Entity::find()
            .filter(memory_capture_event::Column::SourceHash.eq(source_hash.to_string()))
            .count(&self.db)
            .await?)
    }

    pub async fn list_memory_capture_events_by_source_hash(
        &self,
        source_hash: &str,
        limit: u64,
    ) -> Result<Vec<memory_capture_event::Model>> {
        Ok(memory_capture_event::Entity::find()
            .filter(memory_capture_event::Column::SourceHash.eq(source_hash.to_string()))
            .order_by_desc(memory_capture_event::Column::UpdatedAt)
            .limit(Self::db_limit(limit))
            .all(&self.db)
            .await?)
    }

    pub async fn try_claim_memory_capture_event_status(
        &self,
        id: &str,
        expected_status: &str,
        claimed_status: &str,
        updated_at: &str,
    ) -> Result<bool> {
        let result = memory_capture_event::Entity::update_many()
            .col_expr(
                memory_capture_event::Column::Status,
                Expr::value(claimed_status.to_string()),
            )
            .col_expr(
                memory_capture_event::Column::UpdatedAt,
                Expr::value(updated_at.to_string()),
            )
            .filter(memory_capture_event::Column::Id.eq(id.to_string()))
            .filter(memory_capture_event::Column::Status.eq(expected_status.to_string()))
            .exec(&self.db)
            .await?;
        Ok(result.rows_affected > 0)
    }

    pub async fn try_update_memory_capture_event_from_status(
        &self,
        event: &memory_capture_event::Model,
        expected_status: &str,
    ) -> Result<bool> {
        let result = memory_capture_event::Entity::update_many()
            .col_expr(
                memory_capture_event::Column::SourceMessageId,
                Expr::value(event.source_message_id.clone()),
            )
            .col_expr(
                memory_capture_event::Column::ConversationId,
                Expr::value(event.conversation_id.clone()),
            )
            .col_expr(
                memory_capture_event::Column::ProjectId,
                Expr::value(event.project_id.clone()),
            )
            .col_expr(
                memory_capture_event::Column::Channel,
                Expr::value(event.channel.clone()),
            )
            .col_expr(
                memory_capture_event::Column::Status,
                Expr::value(event.status.clone()),
            )
            .col_expr(
                memory_capture_event::Column::CaptureKind,
                Expr::value(event.capture_kind.clone()),
            )
            .col_expr(
                memory_capture_event::Column::SourceHash,
                Expr::value(event.source_hash.clone()),
            )
            .col_expr(
                memory_capture_event::Column::AttemptMetadata,
                Expr::value(event.attempt_metadata.clone()),
            )
            .col_expr(
                memory_capture_event::Column::ErrorHistory,
                Expr::value(event.error_history.clone()),
            )
            .col_expr(
                memory_capture_event::Column::ReplayCount,
                Expr::value(event.replay_count),
            )
            .col_expr(
                memory_capture_event::Column::NextRetryAt,
                Expr::value(event.next_retry_at.clone()),
            )
            .col_expr(
                memory_capture_event::Column::CompletedAt,
                Expr::value(event.completed_at.clone()),
            )
            .col_expr(
                memory_capture_event::Column::CreatedAt,
                Expr::value(event.created_at.clone()),
            )
            .col_expr(
                memory_capture_event::Column::UpdatedAt,
                Expr::value(event.updated_at.clone()),
            )
            .filter(memory_capture_event::Column::Id.eq(event.id.clone()))
            .filter(memory_capture_event::Column::Status.eq(expected_status.to_string()))
            .exec(&self.db)
            .await?;
        Ok(result.rows_affected > 0)
    }

    pub async fn upsert_memory_operation(&self, operation: &memory_operation::Model) -> Result<()> {
        let txn = self.db.begin().await?;
        memory_operation::Entity::insert(Self::memory_operation_active_model(operation)?)
            .on_conflict(
                OnConflict::column(memory_operation::Column::Id)
                    .update_columns(MEMORY_OPERATION_LIGHT_UPSERT_COLUMNS.iter().copied())
                    .to_owned(),
            )
            .exec(&txn)
            .await?;
        let current = memory_operation::Entity::find_by_id(operation.id.clone())
            .lock_exclusive()
            .one(&txn)
            .await?
            .ok_or_else(|| anyhow!("Memory operation '{}' missing after upsert", operation.id))?;
        if let Some(model) = Self::memory_operation_heavy_update_active_model(&current, operation)?
        {
            model.update(&txn).await?;
        }
        txn.commit().await?;
        Ok(())
    }

    pub async fn get_memory_operation(&self, id: &str) -> Result<Option<memory_operation::Model>> {
        let Some(mut model) = memory_operation::Entity::find_by_id(id.to_string())
            .one(&self.db)
            .await?
        else {
            return Ok(None);
        };
        decrypt_memory_operation_model(&mut model);
        Ok(Some(model))
    }

    pub async fn list_memory_operations_for_memory(
        &self,
        memory_id: &str,
        project_id: Option<&str>,
        limit: u64,
    ) -> Result<Vec<memory_operation::Model>> {
        let mut query = memory_operation::Entity::find().filter(
            Condition::any()
                .add(memory_operation::Column::TargetMemoryId.eq(memory_id.to_string()))
                .add(memory_operation::Column::AppliedMemoryId.eq(memory_id.to_string())),
        );
        query = match project_id.map(str::trim).filter(|value| !value.is_empty()) {
            Some(pid) => query.filter(
                Condition::any()
                    .add(memory_operation::Column::ProjectId.is_null())
                    .add(memory_operation::Column::ProjectId.eq(pid.to_string())),
            ),
            None => query.filter(memory_operation::Column::ProjectId.is_null()),
        };
        let mut rows = query
            .order_by_desc(memory_operation::Column::UpdatedAt)
            .limit(Self::db_limit(limit))
            .all(&self.db)
            .await?;
        for row in &mut rows {
            decrypt_memory_operation_model(row);
        }
        Ok(rows)
    }

    pub async fn list_memory_operations_by_statuses(
        &self,
        statuses: &[&str],
        project_id: Option<&str>,
        limit: u64,
    ) -> Result<Vec<memory_operation::Model>> {
        let mut query = memory_operation::Entity::find();
        if !statuses.is_empty() {
            query = query.filter(
                memory_operation::Column::Status.is_in(
                    statuses
                        .iter()
                        .map(|status| (*status).to_string())
                        .collect::<Vec<_>>(),
                ),
            );
        }
        query = match project_id.map(str::trim).filter(|value| !value.is_empty()) {
            Some(pid) => query.filter(
                Condition::any()
                    .add(memory_operation::Column::ProjectId.is_null())
                    .add(memory_operation::Column::ProjectId.eq(pid.to_string())),
            ),
            None => query.filter(memory_operation::Column::ProjectId.is_null()),
        };
        let mut rows = query
            .order_by_desc(memory_operation::Column::UpdatedAt)
            .limit(Self::db_limit(limit))
            .all(&self.db)
            .await?;
        for row in &mut rows {
            decrypt_memory_operation_model(row);
        }
        Ok(rows)
    }

    pub async fn count_memory_operations_by_statuses_all_scopes(
        &self,
        statuses: &[&str],
    ) -> Result<u64> {
        let mut query = memory_operation::Entity::find();
        if !statuses.is_empty() {
            query = query.filter(
                memory_operation::Column::Status.is_in(
                    statuses
                        .iter()
                        .map(|status| (*status).to_string())
                        .collect::<Vec<_>>(),
                ),
            );
        }
        Ok(query.count(&self.db).await?)
    }

    pub async fn upsert_memory_evidence_link(
        &self,
        link: &memory_evidence_link::Model,
    ) -> Result<()> {
        memory_evidence_link::Entity::insert(Self::memory_evidence_link_active_model(link))
            .on_conflict(
                OnConflict::column(memory_evidence_link::Column::Id)
                    .update_columns([
                        memory_evidence_link::Column::OperationId,
                        memory_evidence_link::Column::MemoryId,
                        memory_evidence_link::Column::EvidenceKind,
                        memory_evidence_link::Column::EvidenceRef,
                        memory_evidence_link::Column::SourceMessageId,
                        memory_evidence_link::Column::CaptureEventId,
                        memory_evidence_link::Column::ProjectId,
                        memory_evidence_link::Column::ConversationId,
                        memory_evidence_link::Column::Metadata,
                    ])
                    .to_owned(),
            )
            .exec(&self.db)
            .await?;
        Ok(())
    }

    pub async fn list_memory_evidence_links_for_memory(
        &self,
        memory_id: &str,
        project_id: Option<&str>,
        limit: u64,
    ) -> Result<Vec<memory_evidence_link::Model>> {
        let mut query = memory_evidence_link::Entity::find()
            .filter(memory_evidence_link::Column::MemoryId.eq(memory_id.to_string()));
        query = match project_id.map(str::trim).filter(|value| !value.is_empty()) {
            Some(pid) => query.filter(
                Condition::any()
                    .add(memory_evidence_link::Column::ProjectId.is_null())
                    .add(memory_evidence_link::Column::ProjectId.eq(pid.to_string())),
            ),
            None => query.filter(memory_evidence_link::Column::ProjectId.is_null()),
        };
        Ok(query
            .order_by_desc(memory_evidence_link::Column::CreatedAt)
            .limit(Self::db_limit(limit))
            .all(&self.db)
            .await?)
    }

    pub async fn get_learning_candidate(
        &self,
        id: &str,
    ) -> Result<Option<learning_candidate::Model>> {
        Ok(learning_candidate::Entity::find_by_id(id.to_string())
            .one(&self.db)
            .await?)
    }

    pub async fn list_learning_candidates_with_options(
        &self,
        approval_status: Option<&str>,
        include_superseded: bool,
        limit: u64,
    ) -> Result<Vec<learning_candidate::Model>> {
        let mut query = learning_candidate::Entity::find();
        if let Some(status) = approval_status.filter(|v| !v.trim().is_empty()) {
            query = query.filter(learning_candidate::Column::ApprovalStatus.eq(status));
        } else if !include_superseded {
            query = query.filter(learning_candidate::Column::ApprovalStatus.ne("superseded"));
        }
        Ok(query
            .order_by_desc(learning_candidate::Column::UpdatedAt)
            .limit(Self::db_limit(limit))
            .all(&self.db)
            .await?)
    }

    pub async fn list_learning_candidates_for_review(
        &self,
        approval_statuses: &[&str],
        candidate_types: &[&str],
        project_id: Option<&str>,
        limit: u64,
    ) -> Result<Vec<learning_candidate::Model>> {
        let mut query = learning_candidate::Entity::find();
        if !approval_statuses.is_empty() {
            query = query.filter(
                learning_candidate::Column::ApprovalStatus.is_in(
                    approval_statuses
                        .iter()
                        .map(|status| (*status).to_string())
                        .collect::<Vec<_>>(),
                ),
            );
        }
        if !candidate_types.is_empty() {
            query = query.filter(
                learning_candidate::Column::CandidateType.is_in(
                    candidate_types
                        .iter()
                        .map(|candidate_type| (*candidate_type).to_string())
                        .collect::<Vec<_>>(),
                ),
            );
        }
        query = match project_id.map(str::trim).filter(|value| !value.is_empty()) {
            Some(pid) => query.filter(
                Condition::any()
                    .add(learning_candidate::Column::ProjectId.is_null())
                    .add(learning_candidate::Column::ProjectId.eq(pid.to_string())),
            ),
            None => query.filter(learning_candidate::Column::ProjectId.is_null()),
        };
        Ok(query
            .order_by_desc(learning_candidate::Column::UpdatedAt)
            .limit(Self::db_limit(limit))
            .all(&self.db)
            .await?)
    }

    #[allow(dead_code)]
    pub async fn list_learning_candidates(
        &self,
        approval_status: Option<&str>,
        limit: u64,
    ) -> Result<Vec<learning_candidate::Model>> {
        self.list_learning_candidates_with_options(approval_status, false, limit)
            .await
    }

    pub async fn list_learning_candidates_for_subject(
        &self,
        candidate_type: &str,
        subject_key: &str,
        limit: u64,
    ) -> Result<Vec<learning_candidate::Model>> {
        Ok(learning_candidate::Entity::find()
            .filter(learning_candidate::Column::CandidateType.eq(candidate_type.to_string()))
            .filter(learning_candidate::Column::SubjectKey.eq(subject_key.to_string()))
            .order_by_desc(learning_candidate::Column::UpdatedAt)
            .limit(Self::db_limit(limit))
            .all(&self.db)
            .await?)
    }

    pub async fn list_learning_candidates_for_subject_key(
        &self,
        subject_key: &str,
        candidate_types: &[&str],
        project_id: Option<&str>,
        limit: u64,
    ) -> Result<Vec<learning_candidate::Model>> {
        let mut query = learning_candidate::Entity::find()
            .filter(learning_candidate::Column::SubjectKey.eq(subject_key.to_string()));
        if !candidate_types.is_empty() {
            query = query.filter(
                learning_candidate::Column::CandidateType.is_in(
                    candidate_types
                        .iter()
                        .map(|candidate_type| (*candidate_type).to_string())
                        .collect::<Vec<_>>(),
                ),
            );
        }
        query = match project_id.map(str::trim).filter(|value| !value.is_empty()) {
            Some(pid) => query.filter(
                Condition::any()
                    .add(learning_candidate::Column::ProjectId.is_null())
                    .add(learning_candidate::Column::ProjectId.eq(pid.to_string())),
            ),
            None => query.filter(learning_candidate::Column::ProjectId.is_null()),
        };
        Ok(query
            .order_by_desc(learning_candidate::Column::UpdatedAt)
            .limit(Self::db_limit(limit))
            .all(&self.db)
            .await?)
    }

    pub async fn update_learning_candidate_review(
        &self,
        id: &str,
        approval_status: &str,
        review_notes: Option<&str>,
        approved_ref: Option<&str>,
    ) -> Result<()> {
        learning_candidate::Entity::update_many()
            .col_expr(
                learning_candidate::Column::ApprovalStatus,
                Expr::value(approval_status.to_string()),
            )
            .col_expr(
                learning_candidate::Column::ReviewNotes,
                Expr::value(review_notes.map(|value| value.to_string())),
            )
            .col_expr(
                learning_candidate::Column::ReviewedAt,
                Expr::value(Some(chrono::Utc::now().to_rfc3339())),
            )
            .col_expr(
                learning_candidate::Column::ApprovedRef,
                Expr::value(approved_ref.map(|value| value.to_string())),
            )
            .col_expr(
                learning_candidate::Column::UpdatedAt,
                Expr::value(chrono::Utc::now().to_rfc3339()),
            )
            .filter(learning_candidate::Column::Id.eq(id))
            .exec(&self.db)
            .await?;
        Ok(())
    }

    pub async fn update_learning_candidate_review_if_status(
        &self,
        id: &str,
        expected_status: &str,
        approval_status: &str,
        review_notes: Option<&str>,
        approved_ref: Option<&str>,
    ) -> Result<bool> {
        let now = chrono::Utc::now().to_rfc3339();
        let result = learning_candidate::Entity::update_many()
            .col_expr(
                learning_candidate::Column::ApprovalStatus,
                Expr::value(approval_status.to_string()),
            )
            .col_expr(
                learning_candidate::Column::ReviewNotes,
                Expr::value(review_notes.map(|value| value.to_string())),
            )
            .col_expr(
                learning_candidate::Column::ReviewedAt,
                Expr::value(Some(now.clone())),
            )
            .col_expr(
                learning_candidate::Column::ApprovedRef,
                Expr::value(approved_ref.map(|value| value.to_string())),
            )
            .col_expr(learning_candidate::Column::UpdatedAt, Expr::value(now))
            .filter(learning_candidate::Column::Id.eq(id))
            .filter(learning_candidate::Column::ApprovalStatus.eq(expected_status))
            .exec(&self.db)
            .await?;
        Ok(result.rows_affected > 0)
    }

    pub async fn update_learning_candidate_review_guarded(
        &self,
        lease_key: &str,
        guard: &KvLeaseGuard,
        id: &str,
        approval_status: &str,
        review_notes: Option<&str>,
        approved_ref: Option<&str>,
    ) -> Result<bool> {
        let txn = self.db.begin().await?;
        if !self
            .require_kv_lease_guard_txn(&txn, lease_key, guard)
            .await?
        {
            txn.rollback().await?;
            return Ok(false);
        }
        self.update_learning_candidate_review_txn(
            &txn,
            id,
            approval_status,
            review_notes,
            approved_ref,
        )
        .await?;
        txn.commit().await?;
        Ok(true)
    }

    pub async fn disable_strategy_canary_for_version(
        &self,
        candidate_version: &str,
    ) -> Result<bool> {
        let txn = self.db.begin().await?;
        self.ensure_kv_row_exists_txn(
            &txn,
            crate::core::self_evolve::strategy_runtime::TOOL_STRATEGY_CANARY_STATE_KEY,
        )
        .await?;
        let Some(mut canary_state) = self
            .load_kv_json_txn::<crate::core::self_evolve::strategy_runtime::CanaryRolloutState>(
                &txn,
                crate::core::self_evolve::strategy_runtime::TOOL_STRATEGY_CANARY_STATE_KEY,
            )
            .await?
        else {
            txn.rollback().await?;
            return Ok(false);
        };
        if canary_state.candidate_version != candidate_version {
            txn.rollback().await?;
            return Ok(false);
        }
        canary_state.enabled = false;
        self.set_kv_json_txn(
            &txn,
            crate::core::self_evolve::strategy_runtime::TOOL_STRATEGY_CANARY_STATE_KEY,
            &canary_state,
        )
        .await?;
        txn.commit().await?;
        Ok(true)
    }

    pub async fn approve_strategy_learning_candidate(
        &self,
        candidate_id: &str,
        review_notes: Option<&str>,
    ) -> Result<String> {
        let txn = self.db.begin().await?;
        let candidate = self
            .load_learning_candidate_txn(&txn, candidate_id)
            .await?
            .ok_or_else(|| anyhow!("Learning candidate '{}' not found", candidate_id))?;
        if candidate.candidate_type != "strategy" {
            anyhow::bail!(
                "Learning candidate '{}' is not a strategy candidate",
                candidate_id
            );
        }
        let profile = parse_strategy_candidate_profile(&candidate)?;
        let baseline_version = self
            .load_kv_json_txn::<crate::core::self_evolve::strategy_runtime::ToolStrategyProfile>(
                &txn,
                crate::core::self_evolve::strategy_runtime::TOOL_STRATEGY_PROFILE_KEY,
            )
            .await?
            .map(|value| value.version)
            .unwrap_or_else(|| "strategy-v1".to_string());
        let canary_state = crate::core::self_evolve::strategy_runtime::CanaryRolloutState {
            enabled: true,
            baseline_version,
            candidate_version: profile.version.clone(),
            rollout_percent: 20,
            activated_at: Some(chrono::Utc::now().to_rfc3339()),
            ..Default::default()
        };
        self.set_kv_json_txn(
            &txn,
            crate::core::self_evolve::strategy_runtime::TOOL_STRATEGY_PROFILE_CANARY_KEY,
            &profile,
        )
        .await?;
        self.set_kv_json_txn(
            &txn,
            crate::core::self_evolve::strategy_runtime::TOOL_STRATEGY_CANARY_STATE_KEY,
            &canary_state,
        )
        .await?;
        self.update_learning_candidate_review_txn(
            &txn,
            candidate_id,
            "approved",
            review_notes,
            Some(&profile.version),
        )
        .await?;
        txn.commit().await?;
        Ok(profile.version)
    }

    pub async fn reject_strategy_learning_candidate(
        &self,
        candidate_id: &str,
        review_notes: Option<&str>,
    ) -> Result<String> {
        let txn = self.db.begin().await?;
        let candidate = self
            .load_learning_candidate_txn(&txn, candidate_id)
            .await?
            .ok_or_else(|| anyhow!("Learning candidate '{}' not found", candidate_id))?;
        if candidate.candidate_type != "strategy" {
            anyhow::bail!(
                "Learning candidate '{}' is not a strategy candidate",
                candidate_id
            );
        }
        let profile = parse_strategy_candidate_profile(&candidate)?;
        let canary_key = crate::core::self_evolve::strategy_runtime::TOOL_STRATEGY_CANARY_STATE_KEY;
        if let Some(mut canary_state) = self
            .load_kv_json_txn::<crate::core::self_evolve::strategy_runtime::CanaryRolloutState>(
                &txn, canary_key,
            )
            .await?
        {
            if canary_state.candidate_version == profile.version {
                canary_state.enabled = false;
                self.set_kv_json_txn(&txn, canary_key, &canary_state)
                    .await?;
            }
        }
        self.update_learning_candidate_review_txn(
            &txn,
            candidate_id,
            "rejected",
            review_notes,
            None,
        )
        .await?;
        txn.commit().await?;
        Ok(profile.version)
    }

    pub async fn promote_strategy_learning_candidate_to_baseline(
        &self,
        candidate_id: &str,
    ) -> Result<String> {
        let txn = self.db.begin().await?;
        let candidate = self
            .load_learning_candidate_txn(&txn, candidate_id)
            .await?
            .ok_or_else(|| anyhow!("Learning candidate '{}' not found", candidate_id))?;
        if candidate.candidate_type != "strategy" {
            anyhow::bail!(
                "Learning candidate '{}' is not a strategy candidate",
                candidate_id
            );
        }
        if !candidate.approval_status.eq_ignore_ascii_case("approved") {
            anyhow::bail!("Strategy candidate must be approved before promotion");
        }
        let profile = parse_strategy_candidate_profile(&candidate)?;
        let profile_key = crate::core::self_evolve::strategy_runtime::TOOL_STRATEGY_PROFILE_KEY;
        let snapshot_key =
            crate::core::self_evolve::strategy_runtime::TOOL_STRATEGY_PROFILE_BASELINE_SNAPSHOT_KEY;
        if let Some(existing_baseline) = self.get_kv_for_update_txn(&txn, profile_key).await? {
            if !existing_baseline.value.is_empty() {
                self.set_kv_txn(&txn, snapshot_key, &existing_baseline.value)
                    .await?;
            }
        }
        self.set_kv_json_txn(&txn, profile_key, &profile).await?;

        let canary_key = crate::core::self_evolve::strategy_runtime::TOOL_STRATEGY_CANARY_STATE_KEY;
        let mut canary_state = self
            .load_kv_json_txn::<crate::core::self_evolve::strategy_runtime::CanaryRolloutState>(
                &txn, canary_key,
            )
            .await?
            .unwrap_or_default();
        canary_state.enabled = false;
        canary_state.baseline_version = profile.version.clone();
        canary_state.candidate_version = profile.version.clone();
        self.set_kv_json_txn(&txn, canary_key, &canary_state)
            .await?;

        txn.commit().await?;
        Ok(profile.version)
    }

    pub async fn rollback_tool_strategy_baseline(&self) -> Result<String> {
        let txn = self.db.begin().await?;
        let snapshot_key =
            crate::core::self_evolve::strategy_runtime::TOOL_STRATEGY_PROFILE_BASELINE_SNAPSHOT_KEY;
        let snapshot_row = self
            .get_kv_for_update_txn(&txn, snapshot_key)
            .await?
            .ok_or_else(|| anyhow!("No tool-strategy baseline snapshot available for rollback"))?;
        let snapshot = snapshot_row.value;
        if snapshot.is_empty() {
            anyhow::bail!("No tool-strategy baseline snapshot available for rollback");
        }
        let restored_profile = parse_kv_json_value::<
            crate::core::self_evolve::strategy_runtime::ToolStrategyProfile,
        >(snapshot_key, &snapshot)?
        .ok_or_else(|| anyhow!("No tool-strategy baseline snapshot available for rollback"))?;
        self.set_kv_txn(
            &txn,
            crate::core::self_evolve::strategy_runtime::TOOL_STRATEGY_PROFILE_KEY,
            &snapshot,
        )
        .await?;

        let canary_key = crate::core::self_evolve::strategy_runtime::TOOL_STRATEGY_CANARY_STATE_KEY;
        let mut canary_state = self
            .load_kv_json_txn::<crate::core::self_evolve::strategy_runtime::CanaryRolloutState>(
                &txn, canary_key,
            )
            .await?
            .unwrap_or_default();
        canary_state.enabled = false;
        canary_state.baseline_version = restored_profile.version.clone();
        canary_state.candidate_version = restored_profile.version.clone();
        self.set_kv_json_txn(&txn, canary_key, &canary_state)
            .await?;

        txn.commit().await?;
        Ok(restored_profile.version)
    }

    pub async fn learning_queue_counts(&self) -> Result<LearningQueueCounts> {
        let provisional_runs = experience_run::Entity::find()
            .filter(experience_run::Column::SuccessState.eq("provisional"))
            .count(&self.db)
            .await?;
        let pending_consolidation = experience_run::Entity::find()
            .filter(experience_run::Column::Consolidated.eq(false))
            .filter(
                Condition::any()
                    .add(experience_run::Column::SuccessState.ne("provisional"))
                    .add(experience_run::Column::CorrectionState.eq("corrected")),
            )
            .count(&self.db)
            .await?;
        let draft_candidates = learning_candidate::Entity::find()
            .filter(learning_candidate::Column::ApprovalStatus.eq("draft"))
            .count(&self.db)
            .await?;
        let pending_reflection = experience_run::Entity::find()
            .filter(experience_run::Column::Consolidated.eq(true))
            .filter(experience_run::Column::HeuristicReflected.eq(false))
            .filter(
                Condition::any()
                    .add(experience_run::Column::HeuristicReflectionStatus.is_null())
                    .add(experience_run::Column::HeuristicReflectionStatus.eq("pending")),
            )
            .count(&self.db)
            .await?;
        let active_patterns = procedural_pattern::Entity::find()
            .filter(procedural_pattern::Column::Status.eq("active"))
            .count(&self.db)
            .await?;
        Ok(LearningQueueCounts {
            provisional_runs,
            pending_consolidation,
            pending_reflection,
            draft_candidates,
            active_patterns,
        })
    }

    pub async fn latest_migration_version(&self) -> Result<Option<i64>> {
        match self
            .db
            .query_one(Statement::from_string(
                DbBackend::Postgres,
                "SELECT MAX(version) AS version FROM schema_migrations".to_string(),
            ))
            .await
        {
            Ok(Some(row)) => Ok(row.try_get::<i64>("", "version").ok()),
            Ok(None) => Ok(None),
            Err(_) => Ok(None),
        }
    }

    pub fn expected_migration_version(&self) -> i64 {
        migrations::latest_version()
    }

    pub async fn database_table_names(&self) -> Result<Vec<String>> {
        let query = Query::select()
            .column((Alias::new("tables"), Alias::new("table_name")))
            .from((Alias::new("information_schema"), Alias::new("tables")))
            .and_where(Expr::col((Alias::new("tables"), Alias::new("table_schema"))).eq("public"))
            .and_where(Expr::col((Alias::new("tables"), Alias::new("table_type"))).eq("BASE TABLE"))
            .order_by((Alias::new("tables"), Alias::new("table_name")), Order::Asc)
            .to_owned();
        let rows = self.db.query_all(DbBackend::Postgres.build(&query)).await?;
        let mut table_names = Vec::with_capacity(rows.len());
        for row in rows {
            if let Ok(name) = row.try_get::<String>("", "table_name") {
                table_names.push(name);
            }
        }
        Ok(table_names)
    }

    pub async fn housekeeping_status(&self) -> Result<HousekeepingStatus> {
        let housekeeping_last_run_at = self
            .get(Self::HOUSEKEEPING_PURGE_LAST_RUN_KEY)
            .await?
            .and_then(|raw| String::from_utf8(raw).ok());
        let notification_last_run_at = self
            .get(Self::NOTIFICATION_PURGE_LAST_RUN_KEY)
            .await?
            .and_then(|raw| String::from_utf8(raw).ok());
        Ok(HousekeepingStatus {
            housekeeping_last_run_at,
            notification_last_run_at,
        })
    }

    pub async fn database_size_bytes(&self) -> Result<Option<i64>> {
        let query = Query::select()
            .expr_as(
                Func::cust(Alias::new("pg_database_size"))
                    .arg(Func::cust(Alias::new("current_database"))),
                Alias::new("size_bytes"),
            )
            .to_owned();
        let row = self.db.query_one(DbBackend::Postgres.build(&query)).await?;
        Ok(row.and_then(|value| value.try_get::<i64>("", "size_bytes").ok()))
    }

    pub async fn lease_status_summary(&self) -> Result<LeaseStatusSummary> {
        let now = chrono::Utc::now().to_rfc3339();
        let pending_status = serde_json::to_string(&crate::core::TaskStatus::Pending)
            .unwrap_or_else(|_| "\"pending\"".to_string());
        Ok(LeaseStatusSummary {
            pending_task_backlog: task::Entity::find()
                .filter(task::Column::Status.eq(pending_status.clone()))
                .filter(
                    Condition::any()
                        .add(task::Column::ScheduledFor.is_null())
                        .add(task::Column::ScheduledFor.lte(now.clone())),
                )
                .filter(
                    Condition::any()
                        .add(task::Column::LeaseExpiresAt.is_null())
                        .add(task::Column::LeaseExpiresAt.lte(now.clone())),
                )
                .count(&self.db)
                .await?,
            active_task_leases: task::Entity::find()
                .filter(task::Column::LeaseExpiresAt.is_not_null())
                .filter(task::Column::LeaseExpiresAt.gt(now.clone()))
                .count(&self.db)
                .await?,
            tasks_waiting_retry: task::Entity::find()
                .filter(task::Column::NextRetryAt.is_not_null())
                .filter(task::Column::NextRetryAt.gt(now.clone()))
                .count(&self.db)
                .await?,
            watcher_poll_backlog: watcher::Entity::find()
                .filter(watcher::Column::Status.eq("active"))
                .filter(
                    Condition::any()
                        .add(watcher::Column::NextRetryAt.is_null())
                        .add(watcher::Column::NextRetryAt.lte(now.clone())),
                )
                .filter(
                    Condition::any()
                        .add(watcher::Column::LeaseExpiresAt.is_null())
                        .add(watcher::Column::LeaseExpiresAt.lte(now.clone())),
                )
                .count(&self.db)
                .await?,
            active_watcher_leases: watcher::Entity::find()
                .filter(watcher::Column::LeaseExpiresAt.is_not_null())
                .filter(watcher::Column::LeaseExpiresAt.gt(now.clone()))
                .count(&self.db)
                .await?,
            watchers_waiting_retry: watcher::Entity::find()
                .filter(watcher::Column::NextRetryAt.is_not_null())
                .filter(watcher::Column::NextRetryAt.gt(now.clone()))
                .count(&self.db)
                .await?,
            active_run_leases: execution_run::Entity::find()
                .filter(execution_run::Column::LeaseExpiresAt.is_not_null())
                .filter(execution_run::Column::LeaseExpiresAt.gt(now.clone()))
                .count(&self.db)
                .await?,
            runs_pending_cancellation: execution_run::Entity::find()
                .filter(execution_run::Column::CancellationRequested.eq(true))
                .filter(execution_run::Column::Status.is_not_in([
                    "completed",
                    "degraded",
                    "needs_input",
                    "blocked",
                    "platform_failed",
                    "cancelled",
                ]))
                .count(&self.db)
                .await?,
        })
    }

    // ==================== Expenses ====================

    /// Insert an expense
    pub async fn insert_expense(&self, model: expense::Model) -> Result<()> {
        expense::ActiveModel {
            id: Set(model.id),
            amount: Set(model.amount),
            currency: Set(model.currency),
            category: Set(model.category),
            description: Set(model.description),
            date: Set(model.date),
            payment_method: Set(model.payment_method),
            vendor: Set(model.vendor),
            tags: Set(model.tags),
            split_with: Set(model.split_with),
            receipt_path: Set(model.receipt_path),
            created_at: Set(model.created_at),
        }
        .insert(&self.db)
        .await?;
        Ok(())
    }

    /// Get expenses with optional date range and category filter
    pub async fn get_expenses(
        &self,
        from_date: Option<&str>,
        to_date: Option<&str>,
        category: Option<&str>,
    ) -> Result<Vec<expense::Model>> {
        let mut query = expense::Entity::find();
        if let Some(from) = from_date {
            query = query.filter(expense::Column::Date.gte(from.to_string()));
        }
        if let Some(to) = to_date {
            query = query.filter(expense::Column::Date.lte(to.to_string()));
        }
        if let Some(cat) = category {
            query = query.filter(expense::Column::Category.eq(cat.to_string()));
        }
        let results = query
            .order_by_desc(expense::Column::Date)
            .limit(Self::MAX_EXPENSE_ROWS_PER_QUERY)
            .all(&self.db)
            .await?;
        Ok(results)
    }

    /// Get expense summary grouped by category
    pub async fn get_expense_summary(
        &self,
        from_date: Option<&str>,
        to_date: Option<&str>,
    ) -> Result<Vec<expense::Model>> {
        // Return all expenses in range; caller aggregates
        self.get_expenses(from_date, to_date, None).await
    }

    /// Delete an expense
    pub async fn delete_expense(&self, id: &str) -> Result<bool> {
        let result = expense::Entity::delete_by_id(id.to_string())
            .exec(&self.db)
            .await?;
        Ok(result.rows_affected > 0)
    }

    // ==================== Swarm Agents ====================

    /// Insert a swarm agent
    pub async fn insert_swarm_agent(&self, agent: &swarm_agent::Model) -> Result<()> {
        swarm_agent::ActiveModel {
            id: Set(agent.id.clone()),
            name: Set(agent.name.clone()),
            agent_type: Set(agent.agent_type.clone()),
            llm_provider: Set(agent.llm_provider.clone()),
            capabilities: Set(agent.capabilities.clone()),
            system_prompt: Set(agent.system_prompt.clone()),
            access_scope: Set(agent.access_scope.clone()),
            enabled: Set(agent.enabled),
            created_at: Set(agent.created_at.clone()),
        }
        .insert(&self.db)
        .await?;
        Ok(())
    }

    pub async fn upsert_swarm_agent(&self, agent: &swarm_agent::Model) -> Result<()> {
        swarm_agent::Entity::insert(swarm_agent::ActiveModel {
            id: Set(agent.id.clone()),
            name: Set(agent.name.clone()),
            agent_type: Set(agent.agent_type.clone()),
            llm_provider: Set(agent.llm_provider.clone()),
            capabilities: Set(agent.capabilities.clone()),
            system_prompt: Set(agent.system_prompt.clone()),
            access_scope: Set(agent.access_scope.clone()),
            enabled: Set(agent.enabled),
            created_at: Set(agent.created_at.clone()),
        })
        .on_conflict(
            OnConflict::column(swarm_agent::Column::Id)
                .update_columns([
                    swarm_agent::Column::Name,
                    swarm_agent::Column::AgentType,
                    swarm_agent::Column::LlmProvider,
                    swarm_agent::Column::Capabilities,
                    swarm_agent::Column::SystemPrompt,
                    swarm_agent::Column::AccessScope,
                    swarm_agent::Column::Enabled,
                ])
                .to_owned(),
        )
        .exec(&self.db)
        .await?;
        Ok(())
    }

    /// Get all swarm agents
    pub async fn get_swarm_agents(&self) -> Result<Vec<swarm_agent::Model>> {
        let agents = swarm_agent::Entity::find().all(&self.db).await?;
        Ok(agents)
    }

    /// Update a persisted swarm agent
    pub async fn update_swarm_agent(&self, agent: &swarm_agent::Model) -> Result<()> {
        swarm_agent::ActiveModel {
            id: Unchanged(agent.id.clone()),
            name: Set(agent.name.clone()),
            agent_type: Set(agent.agent_type.clone()),
            llm_provider: Set(agent.llm_provider.clone()),
            capabilities: Set(agent.capabilities.clone()),
            system_prompt: Set(agent.system_prompt.clone()),
            access_scope: Set(agent.access_scope.clone()),
            enabled: Set(agent.enabled),
            created_at: Set(agent.created_at.clone()),
        }
        .update(&self.db)
        .await?;
        Ok(())
    }

    /// Delete a swarm agent
    pub async fn delete_swarm_agent(&self, id: &str) -> Result<()> {
        swarm_agent::Entity::delete_by_id(id.to_string())
            .exec(&self.db)
            .await?;
        Ok(())
    }

    /// Seed default specialist agents if none exist (first-run only)
    pub async fn seed_default_agents(&self) -> Result<()> {
        let existing = self.get_swarm_agents().await?;
        if !existing.is_empty() {
            return Ok(()); // Already have agents, skip seeding
        }

        tracing::info!("Seeding default specialist agents...");
        let now = chrono::Utc::now().to_rfc3339();

        let defaults = vec![
            swarm_agent::Model {
                id: format!("default-researcher-{}", &uuid::Uuid::new_v4().to_string()[..8]),
                name: "Researcher".to_string(),
                agent_type: "researcher".to_string(),
                llm_provider: "{}".to_string(),
                capabilities: r#"["deep research","web search","data analysis","fact checking","academic research"]"#.to_string(),
                system_prompt: Some("You are a thorough research specialist. When given a topic, search the web, gather multiple sources, cross-reference facts, and present a well-structured summary with key findings, sources, and confidence levels. Be objective and cite your sources.".to_string()),
                access_scope: "{}".to_string(),
                enabled: 1,
                created_at: now.clone(),
            },
            swarm_agent::Model {
                id: format!("default-coder-{}", &uuid::Uuid::new_v4().to_string()[..8]),
                name: "Coder".to_string(),
                agent_type: "coder".to_string(),
                llm_provider: "{}".to_string(),
                capabilities: r#"["code generation","debugging","code review","refactoring","architecture"]"#.to_string(),
                system_prompt: Some("You are an expert software engineer. Write clean, efficient, well-documented code. When debugging, systematically identify root causes. When reviewing code, focus on correctness, performance, security, and maintainability. Support all major programming languages.".to_string()),
                access_scope: "{}".to_string(),
                enabled: 1,
                created_at: now.clone(),
            },
            swarm_agent::Model {
                id: format!("default-writer-{}", &uuid::Uuid::new_v4().to_string()[..8]),
                name: "Writer".to_string(),
                agent_type: "writer".to_string(),
                llm_provider: "{}".to_string(),
                capabilities: r#"["content writing","editing","summarization","translation","creative writing"]"#.to_string(),
                access_scope: "{}".to_string(),
                system_prompt: Some("You are a skilled writer and editor. Adapt your style to the requested format — professional emails, blog posts, reports, creative fiction, marketing copy, etc. Focus on clarity, engagement, and proper structure. When editing, preserve the author's voice while improving quality.".to_string()),
                enabled: 1,
                created_at: now.clone(),
            },
            swarm_agent::Model {
                id: format!("default-analyst-{}", &uuid::Uuid::new_v4().to_string()[..8]),
                name: "Analyst".to_string(),
                agent_type: "analyst".to_string(),
                llm_provider: "{}".to_string(),
                capabilities: r#"["data analysis","market research","financial analysis","trend analysis","reporting"]"#.to_string(),
                system_prompt: Some("You are a sharp data and business analyst. Break down complex data, identify patterns and trends, provide actionable insights, and present findings clearly with charts and tables when appropriate. Always quantify your conclusions and flag uncertainties.".to_string()),
                access_scope: "{}".to_string(),
                enabled: 1,
                created_at: now.clone(),
            },
            swarm_agent::Model {
                id: format!("default-planner-{}", &uuid::Uuid::new_v4().to_string()[..8]),
                name: "Planner".to_string(),
                agent_type: "planner".to_string(),
                llm_provider: "{}".to_string(),
                capabilities: r#"["project planning","task breakdown","scheduling","goal setting","strategy"]"#.to_string(),
                system_prompt: Some("You are a strategic planner and project manager. Break down goals into actionable steps, estimate effort, identify dependencies and risks, and create clear timelines. Prioritize using impact vs effort. Always suggest concrete next actions.".to_string()),
                access_scope: "{}".to_string(),
                enabled: 1,
                created_at: now.clone(),
            },
        ];

        for agent in &defaults {
            if let Err(e) = self.insert_swarm_agent(agent).await {
                tracing::warn!("Failed to seed agent '{}': {}", agent.name, e);
            }
        }

        tracing::info!("Seeded {} default specialist agents", defaults.len());
        Ok(())
    }

    // ==================== Swarm Delegations ====================

    /// Get recent swarm delegations
    pub async fn get_recent_delegations(&self, limit: u64) -> Result<Vec<swarm_delegation::Model>> {
        let mut delegations = swarm_delegation::Entity::find()
            .order_by_desc(swarm_delegation::Column::CreatedAt)
            .limit(Self::db_limit(
                limit.min(Self::MAX_SWARM_DELEGATION_ROWS_PER_QUERY),
            ))
            .all(&self.db)
            .await?;
        for delegation in &mut delegations {
            decrypt_swarm_delegation_model(delegation);
        }
        Ok(delegations)
    }

    /// Get all swarm delegations
    pub async fn get_all_delegations(&self) -> Result<Vec<swarm_delegation::Model>> {
        let mut delegations = swarm_delegation::Entity::find()
            .order_by_desc(swarm_delegation::Column::CreatedAt)
            .limit(Self::MAX_SWARM_DELEGATION_ROWS_PER_QUERY)
            .all(&self.db)
            .await?;
        for delegation in &mut delegations {
            decrypt_swarm_delegation_model(delegation);
        }
        Ok(delegations)
    }

    pub async fn get_swarm_delegations_for_parent(
        &self,
        parent_task_id: &str,
    ) -> Result<Vec<swarm_delegation::Model>> {
        let row_id_prefix = format!("{parent_task_id}::");
        let mut delegations = swarm_delegation::Entity::find()
            .filter(
                Condition::any()
                    .add(swarm_delegation::Column::ParentTaskId.eq(parent_task_id.to_string()))
                    .add(swarm_delegation::Column::Id.starts_with(row_id_prefix)),
            )
            .order_by_asc(swarm_delegation::Column::CreatedAt)
            .limit(Self::MAX_SWARM_DELEGATION_ROWS_PER_QUERY)
            .all(&self.db)
            .await?;
        for delegation in &mut delegations {
            decrypt_swarm_delegation_model(delegation);
        }
        Ok(delegations)
    }

    pub async fn get_active_swarm_delegations(
        &self,
        limit: u64,
    ) -> Result<Vec<swarm_delegation::Model>> {
        let mut delegations = swarm_delegation::Entity::find()
            .filter(swarm_delegation::Column::CompletedAt.is_null())
            .order_by_desc(swarm_delegation::Column::CreatedAt)
            .limit(Self::db_limit(
                limit.min(Self::MAX_SWARM_DELEGATION_ROWS_PER_QUERY),
            ))
            .all(&self.db)
            .await?;
        for delegation in &mut delegations {
            decrypt_swarm_delegation_model(delegation);
        }
        Ok(delegations)
    }

    /// Insert a swarm delegation record
    pub async fn insert_swarm_delegation(
        &self,
        delegation: &swarm_delegation::Model,
    ) -> Result<()> {
        swarm_delegation::ActiveModel {
            id: Set(delegation.id.clone()),
            parent_task_id: Set(delegation.parent_task_id.clone()),
            agent_id: Set(delegation.agent_id.clone()),
            task_description: Set(encrypt_storage_string(&delegation.task_description)?),
            result: Set(encrypt_optional_storage_string(
                delegation.result.as_deref(),
            )?),
            success: Set(delegation.success),
            confidence: Set(delegation.confidence),
            execution_time_ms: Set(delegation.execution_time_ms),
            created_at: Set(delegation.created_at.clone()),
            completed_at: Set(delegation.completed_at.clone()),
        }
        .insert(&self.db)
        .await?;
        Ok(())
    }

    pub async fn upsert_swarm_delegation(
        &self,
        delegation: &swarm_delegation::Model,
    ) -> Result<()> {
        swarm_delegation::Entity::insert(swarm_delegation::ActiveModel {
            id: Set(delegation.id.clone()),
            parent_task_id: Set(delegation.parent_task_id.clone()),
            agent_id: Set(delegation.agent_id.clone()),
            task_description: Set(encrypt_storage_string(&delegation.task_description)?),
            result: Set(encrypt_optional_storage_string(
                delegation.result.as_deref(),
            )?),
            success: Set(delegation.success),
            confidence: Set(delegation.confidence),
            execution_time_ms: Set(delegation.execution_time_ms),
            created_at: Set(delegation.created_at.clone()),
            completed_at: Set(delegation.completed_at.clone()),
        })
        .on_conflict(
            OnConflict::column(swarm_delegation::Column::Id)
                .update_columns([
                    swarm_delegation::Column::ParentTaskId,
                    swarm_delegation::Column::AgentId,
                    swarm_delegation::Column::TaskDescription,
                    swarm_delegation::Column::Result,
                    swarm_delegation::Column::Success,
                    swarm_delegation::Column::Confidence,
                    swarm_delegation::Column::ExecutionTimeMs,
                    swarm_delegation::Column::CompletedAt,
                ])
                .to_owned(),
        )
        .exec(&self.db)
        .await?;
        Ok(())
    }

    pub async fn mark_swarm_run_interrupted(
        &self,
        parent_task_id: &str,
        summary: &str,
    ) -> Result<u64> {
        let now = chrono::Utc::now().to_rfc3339();
        let row_id_prefix = format!("{parent_task_id}::");
        let rows = swarm_delegation::Entity::find()
            .filter(
                Condition::any()
                    .add(swarm_delegation::Column::ParentTaskId.eq(parent_task_id.to_string()))
                    .add(swarm_delegation::Column::Id.starts_with(row_id_prefix)),
            )
            .filter(swarm_delegation::Column::CompletedAt.is_null())
            .all(&self.db)
            .await?;
        let mut updated = 0_u64;
        for row in rows {
            let mut payload = row
                .result
                .clone()
                .and_then(|raw| {
                    serde_json::from_str::<serde_json::Value>(&decrypt_storage_string(&raw)).ok()
                })
                .and_then(|value| value.as_object().cloned())
                .unwrap_or_default();
            payload.insert("status".to_string(), serde_json::json!("interrupted"));
            payload.insert("updated_at".to_string(), serde_json::json!(now.clone()));
            if !summary.trim().is_empty() {
                payload.insert("summary".to_string(), serde_json::json!(summary));
                payload.insert("latest_update".to_string(), serde_json::json!(summary));
            }
            let payload_json = serde_json::Value::Object(payload).to_string();
            swarm_delegation::ActiveModel {
                id: Unchanged(row.id),
                result: Set(encrypt_optional_storage_string(Some(
                    payload_json.as_str(),
                ))?),
                success: Set(0),
                completed_at: Set(Some(now.clone())),
                ..Default::default()
            }
            .update(&self.db)
            .await?;
            updated = updated.saturating_add(1);
        }
        Ok(updated)
    }

    // ==================== Semantic Work Units ====================

    /// Upsert a derived semantic work unit used by reflection and clustering.
    pub async fn upsert_semantic_work_unit(&self, unit: &semantic_work_unit::Model) -> Result<()> {
        let txn = self.db.begin().await?;
        semantic_work_unit::Entity::insert(Self::semantic_work_unit_active_model(unit)?)
            .on_conflict(
                OnConflict::column(semantic_work_unit::Column::Id)
                    .update_columns(SEMANTIC_WORK_UNIT_LIGHT_UPSERT_COLUMNS.iter().copied())
                    .to_owned(),
            )
            .exec(&txn)
            .await?;
        let current = semantic_work_unit::Entity::find_by_id(unit.id.clone())
            .lock_exclusive()
            .one(&txn)
            .await?
            .ok_or_else(|| anyhow!("Semantic work unit '{}' missing after upsert", unit.id))?;
        if let Some(model) = Self::semantic_work_unit_heavy_update_active_model(&current, unit)? {
            model.update(&txn).await?;
        }
        txn.commit().await?;
        Ok(())
    }

    /// Read derived semantic work units for a time window.
    pub async fn list_semantic_work_units_between(
        &self,
        from: &str,
        to: &str,
        limit: u64,
    ) -> Result<Vec<semantic_work_unit::Model>> {
        let rows = semantic_work_unit::Entity::find()
            .filter(semantic_work_unit::Column::OccurredAt.gte(from.to_string()))
            .filter(semantic_work_unit::Column::OccurredAt.lt(to.to_string()))
            .order_by_desc(semantic_work_unit::Column::OccurredAt)
            .limit(Self::db_limit(limit))
            .all(&self.db)
            .await?;
        Ok(rows
            .into_iter()
            .map(Self::decrypt_semantic_work_unit)
            .collect())
    }

    pub async fn get_semantic_work_unit(
        &self,
        id: &str,
    ) -> Result<Option<semantic_work_unit::Model>> {
        Ok(semantic_work_unit::Entity::find_by_id(id.to_string())
            .one(&self.db)
            .await?
            .map(Self::decrypt_semantic_work_unit))
    }

    /// Find semantically similar derived work units outside a selected time window.
    /// This is intentionally bounded and backed by the semantic_work_units HNSW
    /// index so Reflect can compare the current recap to history without
    /// scanning old rows.
    pub async fn nearest_semantic_work_units_outside_window(
        &self,
        embedding: &PgVector,
        from: &str,
        to: &str,
        exclude_ids: &[String],
        limit: u64,
    ) -> Result<Vec<(semantic_work_unit::Model, f64)>> {
        if limit == 0 || self.db.get_database_backend() != DbBackend::Postgres {
            return Ok(Vec::new());
        }
        let embedding_sql = pgvector_sql_literal(embedding);
        let exclude_clause = if exclude_ids.is_empty() {
            String::new()
        } else {
            format!("AND id NOT IN ({})", sql_string_list(exclude_ids))
        };
        let sql = format!(
            "SELECT id, embedding <=> {embedding_sql} AS cosine_distance \
             FROM semantic_work_units \
             WHERE embedding IS NOT NULL \
               AND (occurred_at < {from} OR occurred_at >= {to}) \
               {exclude_clause} \
             ORDER BY embedding <=> {embedding_sql} ASC, occurred_at DESC \
             LIMIT {}",
            Self::db_limit(limit),
            from = sql_string_literal(from),
            to = sql_string_literal(to),
        );
        let rows = self
            .db
            .query_all(Statement::from_string(DbBackend::Postgres, sql))
            .await?;
        let mut scored = Vec::<(String, f64)>::with_capacity(rows.len());
        for row in rows {
            let id: String = row.try_get("", "id")?;
            let distance: f64 = row.try_get("", "cosine_distance")?;
            scored.push((id, distance));
        }
        if scored.is_empty() {
            return Ok(Vec::new());
        }
        let ids = scored.iter().map(|(id, _)| id.clone()).collect::<Vec<_>>();
        let models = semantic_work_unit::Entity::find()
            .filter(semantic_work_unit::Column::Id.is_in(ids))
            .all(&self.db)
            .await?;
        let mut by_id = models
            .into_iter()
            .map(|model| {
                let model = Self::decrypt_semantic_work_unit(model);
                (model.id.clone(), model)
            })
            .collect::<std::collections::HashMap<_, _>>();
        Ok(scored
            .into_iter()
            .filter_map(|(id, distance)| by_id.remove(&id).map(|model| (model, distance)))
            .collect())
    }

    #[allow(dead_code)]
    pub async fn delete_semantic_work_units_before(&self, cutoff: &str) -> Result<u64> {
        let result = semantic_work_unit::Entity::delete_many()
            .filter(semantic_work_unit::Column::OccurredAt.lt(cutoff.to_string()))
            .exec(&self.db)
            .await?;
        Ok(result.rows_affected)
    }

    pub async fn delete_semantic_work_units_for_source(
        &self,
        source_kind: &str,
        source_id: &str,
    ) -> Result<u64> {
        let result = semantic_work_unit::Entity::delete_many()
            .filter(semantic_work_unit::Column::SourceKind.eq(source_kind.to_string()))
            .filter(semantic_work_unit::Column::SourceId.eq(source_id.to_string()))
            .exec(&self.db)
            .await?;
        Ok(result.rows_affected)
    }

    pub async fn delete_semantic_work_units_for_source_prefix(
        &self,
        source_kind: &str,
        source_id_prefix: &str,
    ) -> Result<u64> {
        let prefix = source_id_prefix.trim();
        if prefix.is_empty() {
            return Ok(0);
        }
        let result = semantic_work_unit::Entity::delete_many()
            .filter(semantic_work_unit::Column::SourceKind.eq(source_kind.to_string()))
            .filter(semantic_work_unit::Column::SourceId.contains(format!("{}:", prefix)))
            .exec(&self.db)
            .await?;
        Ok(result.rows_affected)
    }

    #[allow(dead_code)]
    pub async fn delete_semantic_work_units_for_conversation(
        &self,
        conversation_id: &str,
    ) -> Result<u64> {
        let result = semantic_work_unit::Entity::delete_many()
            .filter(
                Condition::any()
                    .add(semantic_work_unit::Column::ConversationId.eq(conversation_id.to_string()))
                    .add(
                        Condition::all()
                            .add(semantic_work_unit::Column::SourceKind.eq("conversation"))
                            .add(
                                semantic_work_unit::Column::SourceId
                                    .eq(conversation_id.to_string()),
                            ),
                    ),
            )
            .exec(&self.db)
            .await?;
        Ok(result.rows_affected)
    }

    // ==================== Conversations ====================

    /// Create a new conversation
    pub async fn create_conversation(&self, conv: &conversation::Model) -> Result<()> {
        conversation::ActiveModel {
            id: Set(conv.id.clone()),
            title: Set(conv.title.clone()),
            channel: Set(conv.channel.clone()),
            project_id: Set(conv.project_id.clone()),
            created_at: Set(conv.created_at.clone()),
            updated_at: Set(conv.updated_at.clone()),
            message_count: Set(conv.message_count),
            archived: Set(conv.archived),
            starred: Set(conv.starred),
        }
        .insert(&self.db)
        .await?;
        Ok(())
    }

    /// Create a conversation only if another path has not already created it.
    pub async fn create_conversation_if_absent(&self, conv: &conversation::Model) -> Result<bool> {
        if self.get_conversation(&conv.id).await?.is_some() {
            return Ok(false);
        }

        match self.create_conversation(conv).await {
            Ok(()) => Ok(true),
            Err(error) => {
                let text = error.to_string().to_ascii_lowercase();
                if text.contains("duplicate key") || text.contains("unique constraint failed") {
                    Ok(false)
                } else {
                    Err(error)
                }
            }
        }
    }

    /// List conversations (newest first, paginated)
    pub async fn list_conversations(
        &self,
        limit: u64,
        offset: u64,
        project_id: Option<&str>,
        excluded_channels: &[&str],
        starred: Option<bool>,
    ) -> Result<Vec<conversation::Model>> {
        let mut query = conversation::Entity::find().order_by_desc(conversation::Column::UpdatedAt);

        if let Some(pid) = project_id {
            query = query.filter(conversation::Column::ProjectId.eq(pid));
        }
        if !excluded_channels.is_empty() {
            query = query
                .filter(conversation::Column::Channel.is_not_in(excluded_channels.iter().copied()));
        }
        if let Some(is_starred) = starred {
            query = query.filter(conversation::Column::Starred.eq(is_starred));
        }

        let convs = query
            .limit(Self::db_limit(limit))
            .offset(Self::db_offset(offset))
            .all(&self.db)
            .await?;
        Ok(convs)
    }

    /// List conversations in ascending update order, optionally continuing after a cursor.
    pub async fn list_conversations_after_cursor(
        &self,
        updated_after: Option<&str>,
        conversation_id_after: Option<&str>,
        limit: u64,
        project_id: Option<&str>,
    ) -> Result<Vec<conversation::Model>> {
        let mut query = conversation::Entity::find()
            .order_by_asc(conversation::Column::UpdatedAt)
            .order_by_asc(conversation::Column::Id);

        if let Some(pid) = project_id {
            query = query.filter(conversation::Column::ProjectId.eq(pid));
        }

        if let Some(updated_at) = updated_after {
            let cursor_filter = if let Some(conversation_id) = conversation_id_after {
                Condition::any()
                    .add(conversation::Column::UpdatedAt.gt(updated_at))
                    .add(
                        Condition::all()
                            .add(conversation::Column::UpdatedAt.eq(updated_at))
                            .add(conversation::Column::Id.gt(conversation_id)),
                    )
            } else {
                Condition::all().add(conversation::Column::UpdatedAt.gte(updated_at))
            };
            query = query.filter(cursor_filter);
        }

        let convs = query.limit(Self::db_limit(limit)).all(&self.db).await?;
        Ok(convs)
    }

    /// Count conversations
    pub async fn count_conversations(
        &self,
        project_id: Option<&str>,
        excluded_channels: &[&str],
        starred: Option<bool>,
    ) -> Result<u64> {
        let mut query = conversation::Entity::find();
        if let Some(pid) = project_id {
            query = query.filter(conversation::Column::ProjectId.eq(pid));
        }
        if !excluded_channels.is_empty() {
            query = query
                .filter(conversation::Column::Channel.is_not_in(excluded_channels.iter().copied()));
        }
        if let Some(is_starred) = starred {
            query = query.filter(conversation::Column::Starred.eq(is_starred));
        }
        Ok(query.count(&self.db).await?)
    }

    /// List conversations touched inside a time window.
    pub async fn list_conversations_updated_between(
        &self,
        from: &str,
        to: &str,
        limit: u64,
    ) -> Result<Vec<conversation::Model>> {
        Ok(conversation::Entity::find()
            .filter(conversation::Column::UpdatedAt.gte(from.to_string()))
            .filter(conversation::Column::UpdatedAt.lt(to.to_string()))
            .filter(conversation::Column::Archived.eq(false))
            .order_by_desc(conversation::Column::UpdatedAt)
            .limit(Self::db_limit(limit))
            .all(&self.db)
            .await?)
    }

    /// Get a single conversation by ID
    pub async fn get_conversation(&self, id: &str) -> Result<Option<conversation::Model>> {
        let conv = conversation::Entity::find_by_id(id.to_string())
            .one(&self.db)
            .await?;
        Ok(conv)
    }

    /// Update conversation title and updated_at
    pub async fn update_conversation(
        &self,
        id: &str,
        title: Option<&str>,
        message_count: Option<i32>,
        starred: Option<bool>,
    ) -> Result<conversation::Model> {
        let Some(existing) = self.get_conversation(id).await? else {
            anyhow::bail!("Conversation not found");
        };
        if title.is_none() && message_count.is_none() && starred.is_none() {
            return Ok(existing);
        }
        if matches!(starred, Some(true)) && !existing.starred {
            let starred_count = conversation::Entity::find()
                .filter(conversation::Column::Starred.eq(true))
                .count(&self.db)
                .await?;
            if starred_count >= 3 {
                anyhow::bail!("Unstar any other chat. Max 3 starred chats allowed.");
            }
        }
        let now = chrono::Utc::now().to_rfc3339();
        let mut model: conversation::ActiveModel = existing.into();
        let mut touch_updated_at = false;
        if let Some(t) = title {
            model.title = Set(t.to_string());
            touch_updated_at = true;
        }
        if let Some(mc) = message_count {
            model.message_count = Set(mc);
            touch_updated_at = true;
        }
        if let Some(is_starred) = starred {
            model.starred = Set(is_starred);
        }
        if touch_updated_at {
            model.updated_at = Set(now);
        }
        let updated = model.update(&self.db).await?;
        Ok(updated)
    }

    /// Delete a conversation and its messages
    pub async fn delete_conversation(&self, id: &str) -> Result<()> {
        let txn = self.db.begin().await?;
        let message_rows = message::Entity::find()
            .filter(message::Column::ConversationId.eq(id))
            .all(&txn)
            .await?;
        let execution_runs = execution_run::Entity::find()
            .filter(execution_run::Column::ConversationId.eq(id.to_string()))
            .all(&txn)
            .await?;
        let experience_run_rows = experience_run::Entity::find()
            .filter(experience_run::Column::ConversationId.eq(id.to_string()))
            .all(&txn)
            .await?;
        let experience_run_ids = experience_run_rows
            .iter()
            .map(|row| row.id.clone())
            .collect::<Vec<_>>();
        let experience_item_rows = experience_item::Entity::find()
            .filter(experience_item::Column::ConversationId.eq(id.to_string()))
            .all(&txn)
            .await?;
        let experience_item_ids = experience_item_rows
            .iter()
            .map(|row| row.id.clone())
            .collect::<Vec<_>>();
        let memory_operation_rows = if experience_item_ids.is_empty() {
            Vec::new()
        } else {
            memory_operation::Entity::find()
                .filter(
                    Condition::any()
                        .add(
                            memory_operation::Column::TargetMemoryId
                                .is_in(experience_item_ids.clone()),
                        )
                        .add(
                            memory_operation::Column::AppliedMemoryId
                                .is_in(experience_item_ids.clone()),
                        ),
                )
                .all(&txn)
                .await?
        };
        let memory_operation_ids = memory_operation_rows
            .iter()
            .map(|row| row.id.clone())
            .collect::<Vec<_>>();
        let mut trace_ids = std::collections::BTreeSet::new();
        for row in &message_rows {
            if let Some(trace_id) = row
                .trace_id
                .as_deref()
                .map(str::trim)
                .filter(|value| !value.is_empty())
            {
                trace_ids.insert(trace_id.to_string());
            }
        }
        for run in &execution_runs {
            if let Some(trace_id) = run
                .trace_id
                .as_deref()
                .map(str::trim)
                .filter(|value| !value.is_empty())
            {
                trace_ids.insert(trace_id.to_string());
            }
        }
        let trace_ids_vec = trace_ids.iter().cloned().collect::<Vec<_>>();
        let proof_ids = if trace_ids_vec.is_empty() {
            Vec::new()
        } else {
            execution_trace::Entity::find()
                .filter(execution_trace::Column::Id.is_in(trace_ids_vec.clone()))
                .all(&txn)
                .await?
                .into_iter()
                .filter_map(|row| row.proof_id)
                .collect::<std::collections::BTreeSet<_>>()
                .into_iter()
                .collect::<Vec<_>>()
        };
        message::Entity::delete_many()
            .filter(message::Column::ConversationId.eq(id))
            .exec(&txn)
            .await?;
        if !experience_item_ids.is_empty() {
            memory_evidence_link::Entity::delete_many()
                .filter(memory_evidence_link::Column::MemoryId.is_in(experience_item_ids.clone()))
                .exec(&txn)
                .await?;
            learning_candidate::Entity::delete_many()
                .filter(learning_candidate::Column::ApprovedRef.is_in(experience_item_ids.clone()))
                .exec(&txn)
                .await?;
            experience_edge::Entity::delete_many()
                .filter(
                    Condition::any()
                        .add(
                            Condition::all()
                                .add(experience_edge::Column::SourceKind.eq("experience_item"))
                                .add(
                                    experience_edge::Column::SourceRef
                                        .is_in(experience_item_ids.clone()),
                                ),
                        )
                        .add(
                            Condition::all()
                                .add(experience_edge::Column::TargetKind.eq("experience_item"))
                                .add(
                                    experience_edge::Column::TargetRef
                                        .is_in(experience_item_ids.clone()),
                                ),
                        ),
                )
                .exec(&txn)
                .await?;
            recall_event::Entity::delete_many()
                .filter(
                    Condition::any()
                        .add(recall_event::Column::MemoryId.is_in(experience_item_ids.clone()))
                        .add(
                            recall_event::Column::RelatedMemoryId
                                .is_in(experience_item_ids.clone()),
                        )
                        .add(
                            Condition::all()
                                .add(recall_event::Column::SourceKind.eq("experience_item"))
                                .add(
                                    recall_event::Column::SourceRef
                                        .is_in(experience_item_ids.clone()),
                                ),
                        ),
                )
                .exec(&txn)
                .await?;
        }
        if !memory_operation_ids.is_empty() {
            memory_evidence_link::Entity::delete_many()
                .filter(
                    memory_evidence_link::Column::OperationId.is_in(memory_operation_ids.clone()),
                )
                .exec(&txn)
                .await?;
            let candidate_ids = memory_operation_ids
                .iter()
                .map(|operation_id| format!("memory-candidate-{operation_id}"))
                .collect::<Vec<_>>();
            learning_candidate::Entity::delete_many()
                .filter(learning_candidate::Column::Id.is_in(candidate_ids))
                .exec(&txn)
                .await?;
            memory_operation::Entity::delete_many()
                .filter(memory_operation::Column::Id.is_in(memory_operation_ids))
                .exec(&txn)
                .await?;
        }
        if !experience_run_ids.is_empty() {
            experience_edge::Entity::delete_many()
                .filter(experience_edge::Column::SourceRunId.is_in(experience_run_ids.clone()))
                .exec(&txn)
                .await?;
            experience_run::Entity::delete_many()
                .filter(experience_run::Column::Id.is_in(experience_run_ids))
                .exec(&txn)
                .await?;
        }
        if !experience_item_ids.is_empty() {
            experience_item::Entity::delete_many()
                .filter(experience_item::Column::Id.is_in(experience_item_ids))
                .exec(&txn)
                .await?;
        }
        operational_log::Entity::delete_many()
            .filter(operational_log::Column::ConversationId.eq(id.to_string()))
            .exec(&txn)
            .await?;
        semantic_work_unit::Entity::delete_many()
            .filter(
                Condition::any()
                    .add(semantic_work_unit::Column::ConversationId.eq(id.to_string()))
                    .add(
                        Condition::all()
                            .add(semantic_work_unit::Column::SourceKind.eq("conversation"))
                            .add(semantic_work_unit::Column::SourceId.eq(id.to_string())),
                    ),
            )
            .exec(&txn)
            .await?;
        if !trace_ids_vec.is_empty() {
            operational_log::Entity::delete_many()
                .filter(operational_log::Column::TraceId.is_in(trace_ids_vec.clone()))
                .exec(&txn)
                .await?;
            execution_trace::Entity::delete_many()
                .filter(execution_trace::Column::Id.is_in(trace_ids_vec))
                .exec(&txn)
                .await?;
        }
        if !proof_ids.is_empty() {
            execution_proof::Entity::delete_many()
                .filter(execution_proof::Column::Id.is_in(proof_ids))
                .exec(&txn)
                .await?;
        }
        execution_run::Entity::delete_many()
            .filter(execution_run::Column::ConversationId.eq(id.to_string()))
            .exec(&txn)
            .await?;
        conversation::Entity::delete_by_id(id.to_string())
            .exec(&txn)
            .await?;
        txn.commit().await?;
        Ok(())
    }

    // ==================== Messages ====================

    /// Insert a message
    pub async fn insert_message(&self, msg: &message::Model) -> Result<()> {
        let content = encrypt_storage_string(&msg.content)?;
        let tool_calls_json = encrypt_optional_storage_string(msg.tool_calls_json.as_deref())?;
        let tool_call_id = encrypt_optional_storage_string(msg.tool_call_id.as_deref())?;
        let provider_message_json =
            encrypt_optional_storage_string(msg.provider_message_json.as_deref())?;
        let insert_result = message::ActiveModel {
            id: Set(msg.id.clone()),
            conversation_id: Set(msg.conversation_id.clone()),
            role: Set(msg.role.clone()),
            content: Set(content.clone()),
            tool_calls_json: Set(tool_calls_json.clone()),
            tool_call_id: Set(tool_call_id.clone()),
            provider_message_json: Set(provider_message_json.clone()),
            timestamp: Set(msg.timestamp.clone()),
            model_used: Set(msg.model_used.clone()),
            trace_id: Set(msg.trace_id.clone()),
        }
        .insert(&self.db)
        .await;
        if let Err(error) = insert_result {
            if msg.trace_id.is_some() && is_foreign_key_constraint_error(&error) {
                tracing::warn!(
                    "Retrying message insert '{}' without trace_id after FK failure: {}",
                    msg.id,
                    error
                );
                message::ActiveModel {
                    id: Set(msg.id.clone()),
                    conversation_id: Set(msg.conversation_id.clone()),
                    role: Set(msg.role.clone()),
                    content: Set(content),
                    tool_calls_json: Set(tool_calls_json),
                    tool_call_id: Set(tool_call_id),
                    provider_message_json: Set(provider_message_json),
                    timestamp: Set(msg.timestamp.clone()),
                    model_used: Set(msg.model_used.clone()),
                    trace_id: Set(None),
                }
                .insert(&self.db)
                .await?;
            } else {
                return Err(error.into());
            }
        }

        // Update conversation message count and updated_at
        let now = chrono::Utc::now().to_rfc3339();
        conversation::Entity::update_many()
            .col_expr(conversation::Column::UpdatedAt, Expr::value(now))
            .col_expr(
                conversation::Column::MessageCount,
                Expr::col(conversation::Column::MessageCount).add(1),
            )
            .filter(conversation::Column::Id.eq(msg.conversation_id.clone()))
            .exec(&self.db)
            .await?;

        Ok(())
    }

    /// Insert a message only when its stable id has not already been persisted.
    ///
    /// This is used by response-first chat persistence retries. A partially
    /// completed retry must not insert the same chat message twice or increment
    /// the parent conversation count twice.
    pub async fn insert_message_if_absent(&self, msg: &message::Model) -> Result<bool> {
        if message::Entity::find_by_id(msg.id.clone())
            .one(&self.db)
            .await?
            .is_some()
        {
            return Ok(false);
        }

        match self.insert_message(msg).await {
            Ok(()) => Ok(true),
            Err(error) => {
                let text = error.to_string().to_ascii_lowercase();
                if text.contains("duplicate key") || text.contains("unique constraint failed") {
                    Ok(false)
                } else {
                    Err(error)
                }
            }
        }
    }

    pub async fn get_message(&self, id: &str) -> Result<Option<message::Model>> {
        let mut message = message::Entity::find_by_id(id.to_string())
            .one(&self.db)
            .await?;
        if let Some(message) = &mut message {
            decrypt_message_model(message);
        }
        Ok(message)
    }

    /// Get messages for a conversation
    pub async fn get_messages(
        &self,
        conversation_id: &str,
        limit: u64,
        offset: u64,
    ) -> Result<Vec<message::Model>> {
        let mut msgs = message::Entity::find()
            .filter(message::Column::ConversationId.eq(conversation_id))
            .order_by_asc(message::Column::Timestamp)
            .limit(Self::db_limit(limit))
            .offset(Self::db_offset(offset))
            .all(&self.db)
            .await?;
        for msg in &mut msgs {
            decrypt_message_model(msg);
        }
        Ok(msgs)
    }

    /// Get messages for a conversation inside a bounded time window.
    #[allow(dead_code)]
    pub async fn get_messages_between(
        &self,
        conversation_id: &str,
        from: &str,
        to: &str,
        limit: u64,
    ) -> Result<Vec<message::Model>> {
        let mut msgs = message::Entity::find()
            .filter(message::Column::ConversationId.eq(conversation_id))
            .filter(message::Column::Timestamp.gte(from.to_string()))
            .filter(message::Column::Timestamp.lt(to.to_string()))
            .order_by_asc(message::Column::Timestamp)
            .limit(Self::db_limit(limit))
            .all(&self.db)
            .await?;
        for msg in &mut msgs {
            decrypt_message_model(msg);
        }
        Ok(msgs)
    }

    /// Get most recent messages for a conversation in chronological order.
    pub async fn get_recent_messages(
        &self,
        conversation_id: &str,
        limit: u64,
    ) -> Result<Vec<message::Model>> {
        let mut msgs = message::Entity::find()
            .filter(message::Column::ConversationId.eq(conversation_id))
            .order_by_desc(message::Column::Timestamp)
            .limit(Self::db_limit(limit))
            .all(&self.db)
            .await?;
        msgs.reverse();
        for msg in &mut msgs {
            decrypt_message_model(msg);
        }
        Ok(msgs)
    }

    #[allow(dead_code)]
    pub async fn latest_assistant_trace_id_for_conversation(
        &self,
        conversation_id: &str,
    ) -> Result<Option<String>> {
        Ok(message::Entity::find()
            .filter(message::Column::ConversationId.eq(conversation_id.to_string()))
            .filter(message::Column::Role.eq("assistant".to_string()))
            .filter(message::Column::TraceId.is_not_null())
            .order_by_desc(message::Column::Timestamp)
            .one(&self.db)
            .await?
            .and_then(|message| message.trace_id)
            .map(|trace_id| trace_id.trim().to_string())
            .filter(|trace_id| !trace_id.is_empty()))
    }

    /// Get most recent user-authored chat messages across conversations.
    pub async fn get_recent_user_messages(&self, limit: u64) -> Result<Vec<message::Model>> {
        let mut msgs = message::Entity::find()
            .filter(message::Column::Role.eq("user"))
            .order_by_desc(message::Column::Timestamp)
            .limit(Self::db_limit(limit))
            .all(&self.db)
            .await?;
        for msg in &mut msgs {
            decrypt_message_model(msg);
        }
        Ok(msgs)
    }

    /// Get most recent persisted messages across conversations.
    pub async fn get_recent_messages_across_conversations(
        &self,
        limit: u64,
    ) -> Result<Vec<message::Model>> {
        let mut msgs = message::Entity::find()
            .order_by_desc(message::Column::Timestamp)
            .limit(Self::db_limit(limit))
            .all(&self.db)
            .await?;
        for msg in &mut msgs {
            decrypt_message_model(msg);
        }
        Ok(msgs)
    }

    /// Returns true when at least one persisted user chat message exists.
    pub async fn has_user_chat_messages(&self) -> Result<bool> {
        let exists = message::Entity::find()
            .filter(message::Column::Role.eq("user"))
            .limit(1)
            .one(&self.db)
            .await?
            .is_some();
        Ok(exists)
    }

    // ==================== Documents ====================

    fn user_visible_document_filter() -> Condition {
        // Internal AgentArk manuals/capability docs are indexed for retrieval,
        // but they are not user-uploaded library documents.
        Condition::all()
            .add(
                document::Column::Id
                    .starts_with(crate::core::agentark_knowledge::DOCUMENT_ID_PREFIX.to_string())
                    .not(),
            )
            .add(
                document::Column::ContentType
                    .starts_with(
                        crate::core::agentark_knowledge::INTERNAL_DOCUMENT_CONTENT_TYPE_PREFIX
                            .to_string(),
                    )
                    .not(),
            )
    }

    fn document_is_user_visible_after_load(doc: &document::Model) -> bool {
        !crate::core::agentark_knowledge::is_agentark_knowledge_document_id(&doc.id)
            && !crate::core::agentark_knowledge::is_internal_agentark_document_content_type(
                &doc.content_type,
            )
    }

    /// Insert a document and all chunks atomically so partial uploads do not leak
    /// into the searchable document library.
    pub async fn insert_document_with_chunks(
        &self,
        doc: &document::Model,
        chunks: &[document_chunk::Model],
    ) -> Result<()> {
        let txn = self.db.begin().await?;
        let filename = encrypt_storage_string(&doc.filename)?;
        document::ActiveModel {
            id: Set(doc.id.clone()),
            filename: Set(filename),
            content_type: Set(doc.content_type.clone()),
            project_id: Set(doc.project_id.clone()),
            chunk_count: Set(doc.chunk_count),
            file_size: Set(doc.file_size),
            created_at: Set(doc.created_at.clone()),
        }
        .insert(&txn)
        .await?;

        for chunk in chunks {
            let content = encrypt_storage_string(&chunk.content)?;
            document_chunk::ActiveModel {
                id: Set(chunk.id.clone()),
                document_id: Set(chunk.document_id.clone()),
                chunk_index: Set(chunk.chunk_index),
                content: Set(content),
                embedding: Set(chunk.embedding.clone()),
            }
            .insert(&txn)
            .await?;
        }

        txn.commit().await?;
        Ok(())
    }

    /// Replace a deterministic internal document set and its chunks atomically.
    pub async fn replace_documents_by_id_prefix(
        &self,
        id_prefix: &str,
        documents: &[(document::Model, Vec<document_chunk::Model>)],
    ) -> Result<()> {
        let id_prefix = id_prefix.trim();
        if id_prefix.is_empty() {
            anyhow::bail!("document id prefix cannot be empty");
        }

        let txn = self.db.begin().await?;
        let pattern = format!("{id_prefix}%");
        let delete_chunks_sql = format!(
            "DELETE FROM document_chunks WHERE document_id LIKE {}",
            sql_string_literal(&pattern)
        );
        txn.execute(Statement::from_string(
            DbBackend::Postgres,
            delete_chunks_sql,
        ))
        .await?;
        let delete_docs_sql = format!(
            "DELETE FROM documents WHERE id LIKE {}",
            sql_string_literal(&pattern)
        );
        txn.execute(Statement::from_string(DbBackend::Postgres, delete_docs_sql))
            .await?;

        for (doc, chunks) in documents {
            let filename = encrypt_storage_string(&doc.filename)?;
            document::ActiveModel {
                id: Set(doc.id.clone()),
                filename: Set(filename),
                content_type: Set(doc.content_type.clone()),
                project_id: Set(doc.project_id.clone()),
                chunk_count: Set(doc.chunk_count),
                file_size: Set(doc.file_size),
                created_at: Set(doc.created_at.clone()),
            }
            .insert(&txn)
            .await?;

            for chunk in chunks {
                let content = encrypt_storage_string(&chunk.content)?;
                document_chunk::ActiveModel {
                    id: Set(chunk.id.clone()),
                    document_id: Set(chunk.document_id.clone()),
                    chunk_index: Set(chunk.chunk_index),
                    content: Set(content),
                    embedding: Set(chunk.embedding.clone()),
                }
                .insert(&txn)
                .await?;
            }
        }

        txn.commit().await?;
        Ok(())
    }

    /// List documents (paginated)
    pub async fn list_documents(
        &self,
        limit: u64,
        offset: u64,
        project_id: Option<&str>,
    ) -> Result<Vec<document::Model>> {
        let mut query = document::Entity::find().order_by_desc(document::Column::CreatedAt);
        query = query.filter(Self::user_visible_document_filter());
        if let Some(pid) = project_id {
            query = query.filter(document::Column::ProjectId.eq(pid));
        }
        let mut docs = query
            .limit(Self::db_limit(limit))
            .offset(Self::db_offset(offset))
            .all(&self.db)
            .await?;
        for doc in &mut docs {
            doc.filename = decrypt_storage_string(&doc.filename);
        }
        docs.retain(Self::document_is_user_visible_after_load);
        Ok(docs)
    }

    /// Get a single user-visible document by id.
    pub async fn get_document(&self, id: &str) -> Result<Option<document::Model>> {
        let id = id.trim();
        if id.is_empty() {
            return Ok(None);
        }
        let Some(mut doc) = document::Entity::find_by_id(id.to_string())
            .filter(Self::user_visible_document_filter())
            .one(&self.db)
            .await?
        else {
            return Ok(None);
        };
        doc.filename = decrypt_storage_string(&doc.filename);
        if !Self::document_is_user_visible_after_load(&doc) {
            return Ok(None);
        }
        Ok(Some(doc))
    }

    /// Count documents
    pub async fn count_documents(&self, project_id: Option<&str>) -> Result<u64> {
        let mut query = document::Entity::find().filter(Self::user_visible_document_filter());
        if let Some(pid) = project_id {
            query = query.filter(document::Column::ProjectId.eq(pid));
        }
        Ok(query.count(&self.db).await?)
    }

    /// Count document chunks across all documents.
    pub async fn count_document_chunks(&self) -> Result<u64> {
        Ok(document_chunk::Entity::find().count(&self.db).await?)
    }

    /// List a bounded set of documents for metadata search.
    pub async fn list_documents_for_search(
        &self,
        project_id: Option<&str>,
    ) -> Result<Vec<document::Model>> {
        let mut query = document::Entity::find().order_by_desc(document::Column::CreatedAt);
        query = query.filter(Self::user_visible_document_filter());
        if let Some(pid) = project_id {
            query = query.filter(
                Condition::any()
                    .add(document::Column::ProjectId.eq(pid))
                    .add(document::Column::ProjectId.is_null()),
            );
        }
        let mut docs = query
            .limit(Self::MAX_DOCUMENTS_FOR_SEARCH)
            .all(&self.db)
            .await?;
        for doc in &mut docs {
            doc.filename = decrypt_storage_string(&doc.filename);
        }
        docs.retain(Self::document_is_user_visible_after_load);
        Ok(docs)
    }

    /// List deterministic internal documents by id prefix for scoped retrieval.
    pub async fn list_documents_by_id_prefix(
        &self,
        id_prefix: &str,
        limit: u64,
    ) -> Result<Vec<document::Model>> {
        let id_prefix = id_prefix.trim();
        if id_prefix.is_empty() || limit == 0 {
            return Ok(Vec::new());
        }
        let mut docs = document::Entity::find()
            .filter(document::Column::Id.starts_with(id_prefix.to_string()))
            .order_by_desc(document::Column::CreatedAt)
            .limit(Self::db_limit(limit))
            .all(&self.db)
            .await?;
        for doc in &mut docs {
            doc.filename = decrypt_storage_string(&doc.filename);
        }
        Ok(docs)
    }

    /// Get document chunks for search
    pub async fn get_document_chunks(
        &self,
        document_id: &str,
    ) -> Result<Vec<document_chunk::Model>> {
        let visible = document::Entity::find_by_id(document_id.to_string())
            .filter(Self::user_visible_document_filter())
            .one(&self.db)
            .await?
            .is_some();
        if !visible {
            return Ok(Vec::new());
        }

        let mut chunks = document_chunk::Entity::find()
            .filter(document_chunk::Column::DocumentId.eq(document_id))
            .order_by_asc(document_chunk::Column::ChunkIndex)
            .all(&self.db)
            .await?;
        for chunk in &mut chunks {
            chunk.content = decrypt_storage_string(&chunk.content);
        }
        Ok(chunks)
    }

    /// Get a bounded document chunk window for background extraction.
    pub async fn list_document_chunks_for_document_window(
        &self,
        document_id: &str,
        min_chunk_index: i32,
        limit: u64,
    ) -> Result<Vec<document_chunk::Model>> {
        let document_id = document_id.trim();
        if document_id.is_empty() || limit == 0 {
            return Ok(Vec::new());
        }
        let visible = document::Entity::find_by_id(document_id.to_string())
            .filter(Self::user_visible_document_filter())
            .one(&self.db)
            .await?
            .is_some();
        if !visible {
            return Ok(Vec::new());
        }

        let mut chunks = document_chunk::Entity::find()
            .filter(document_chunk::Column::DocumentId.eq(document_id.to_string()))
            .filter(document_chunk::Column::ChunkIndex.gte(min_chunk_index.max(0)))
            .order_by_asc(document_chunk::Column::ChunkIndex)
            .limit(Self::db_limit(limit.min(64)))
            .all(&self.db)
            .await?;
        for chunk in &mut chunks {
            chunk.content = decrypt_storage_string(&chunk.content);
        }
        Ok(chunks)
    }

    /// Get document chunks for a bounded set of documents.
    pub async fn list_document_chunks_for_documents(
        &self,
        document_ids: &[String],
    ) -> Result<Vec<document_chunk::Model>> {
        if document_ids.is_empty() {
            return Ok(Vec::new());
        }

        let mut chunks = document_chunk::Entity::find()
            .filter(document_chunk::Column::DocumentId.is_in(document_ids.iter().cloned()))
            .order_by_asc(document_chunk::Column::DocumentId)
            .order_by_asc(document_chunk::Column::ChunkIndex)
            .limit(Self::MAX_DOCUMENT_CHUNKS_FOR_SEARCH)
            .all(&self.db)
            .await?;
        for chunk in &mut chunks {
            chunk.content = decrypt_storage_string(&chunk.content);
        }
        Ok(chunks)
    }

    pub async fn get_document_chunks_by_ids(
        &self,
        ids: &[String],
    ) -> Result<Vec<document_chunk::Model>> {
        if ids.is_empty() {
            return Ok(Vec::new());
        }

        let mut chunks = document_chunk::Entity::find()
            .filter(document_chunk::Column::Id.is_in(ids.iter().cloned()))
            .all(&self.db)
            .await?;
        for chunk in &mut chunks {
            chunk.content = decrypt_storage_string(&chunk.content);
        }

        let mut by_id = chunks
            .into_iter()
            .map(|chunk| (chunk.id.clone(), chunk))
            .collect::<std::collections::HashMap<_, _>>();

        Ok(ids
            .iter()
            .filter_map(|id| by_id.remove(id))
            .collect::<Vec<_>>())
    }

    pub async fn nearest_document_chunk_ids(
        &self,
        query_embedding: &PgVector,
        document_ids: &[String],
        limit: u64,
    ) -> Result<Vec<String>> {
        if limit == 0 || document_ids.is_empty() {
            return Ok(Vec::new());
        }

        let embedding_sql = pgvector_sql_literal(query_embedding);
        let doc_id_list = sql_string_list(document_ids);
        let sql = format!(
            "SELECT c.id \
             FROM document_chunks c \
             INNER JOIN documents d ON d.id = c.document_id \
             WHERE c.embedding IS NOT NULL AND c.document_id IN ({doc_id_list}) \
             ORDER BY c.embedding <=> {embedding_sql} ASC, d.created_at DESC, c.chunk_index ASC \
             LIMIT {}",
            Self::db_limit(limit)
        );

        let rows = self
            .db
            .query_all(Statement::from_string(DbBackend::Postgres, sql))
            .await?;
        Ok(rows
            .into_iter()
            .filter_map(|row| row.try_get::<String>("", "id").ok())
            .collect())
    }

    pub async fn list_recent_document_chunk_ids(
        &self,
        document_ids: &[String],
        limit: u64,
    ) -> Result<Vec<String>> {
        if limit == 0 || document_ids.is_empty() {
            return Ok(Vec::new());
        }

        let doc_id_list = sql_string_list(document_ids);
        let sql = format!(
            "SELECT c.id \
             FROM document_chunks c \
             INNER JOIN documents d ON d.id = c.document_id \
             WHERE c.document_id IN ({doc_id_list}) \
             ORDER BY d.created_at DESC, c.chunk_index ASC \
             LIMIT {}",
            Self::db_limit(limit)
        );

        let rows = self
            .db
            .query_all(Statement::from_string(DbBackend::Postgres, sql))
            .await?;
        Ok(rows
            .into_iter()
            .filter_map(|row| row.try_get::<String>("", "id").ok())
            .collect())
    }

    pub async fn pgvector_health_check(&self) -> Result<()> {
        if self.db.get_database_backend() != DbBackend::Postgres {
            anyhow::bail!("storage backend is not Postgres");
        }

        let sql = "SELECT '[0,0]'::vector <=> '[0,0]'::vector AS cosine_distance".to_string();
        let row = self
            .db
            .query_one(Statement::from_string(DbBackend::Postgres, sql))
            .await?;

        let row = row.ok_or_else(|| anyhow!("pgvector health check returned no rows"))?;
        let _ = row.try_get::<f64>("", "cosine_distance")?;
        Ok(())
    }

    /// Delete a document and its chunks
    pub async fn delete_document(&self, id: &str) -> Result<()> {
        let txn = self.db.begin().await?;
        let visible = document::Entity::find_by_id(id.to_string())
            .filter(Self::user_visible_document_filter())
            .one(&txn)
            .await?
            .is_some();
        if !visible {
            txn.commit().await?;
            return Ok(());
        }
        document_chunk::Entity::delete_many()
            .filter(document_chunk::Column::DocumentId.eq(id))
            .exec(&txn)
            .await?;
        document::Entity::delete_by_id(id.to_string())
            .exec(&txn)
            .await?;
        txn.commit().await?;
        Ok(())
    }

    // ==================== Notifications ====================

    // Deduplicate repetitive notifications (same root message) to avoid spamming users/UI.
    // This is separate from retention, which deletes old rows according to data lifecycle settings.
    const NOTIFICATION_DEDUP_COOLDOWN_DAYS: i64 = 7;
    const ARKPULSE_NOTIFICATION_WINDOW_HOURS: i64 = 24;
    const NOTIFICATION_PURGE_LAST_RUN_KEY: &'static str = "notifications_retention_last_purge_v1";

    fn notification_is_critical(level: &str, source: &str, title: &str) -> bool {
        let lvl = level.trim().to_ascii_lowercase();
        if lvl == "error" || lvl == "critical" {
            return true;
        }
        let src = source.trim().to_ascii_lowercase();
        if src.contains("security") || src.contains("auth") {
            return true;
        }
        let t = title.trim().to_ascii_lowercase();
        t.contains("security") || t.contains("intrusion") || t.contains("breach")
    }

    /// Generate a signature for notification body comparisons.
    /// Collapses whitespace and replaces digit runs with '#', so small counter changes
    /// (e.g. "5 unread") don't produce spammy near-duplicates.
    fn notification_body_signature(body: &str) -> String {
        let mut out = String::with_capacity(body.len().min(240));
        let mut prev_space = false;
        let mut prev_digit = false;
        for ch in body.chars() {
            if ch.is_ascii_digit() {
                if !prev_digit {
                    out.push('#');
                }
                prev_digit = true;
                prev_space = false;
                continue;
            }
            prev_digit = false;
            if ch.is_whitespace() {
                if !prev_space {
                    out.push(' ');
                    prev_space = true;
                }
                continue;
            }
            prev_space = false;
            out.push(ch.to_ascii_lowercase());
            if out.len() >= 220 {
                break;
            }
        }
        out.trim().to_string()
    }

    fn is_arkpulse_notification(source: &str) -> bool {
        source.trim().eq_ignore_ascii_case("arkpulse")
    }

    fn arkpulse_recent_cutoff_rfc3339() -> String {
        (chrono::Utc::now() - chrono::Duration::hours(Self::ARKPULSE_NOTIFICATION_WINDOW_HOURS))
            .to_rfc3339()
    }

    async fn maybe_purge_old_notifications(&self) -> Result<()> {
        let now = chrono::Utc::now().timestamp();
        let lifecycle = crate::core::data_lifecycle::load_data_lifecycle_settings(self).await;
        if !lifecycle.cleanup_enabled || !lifecycle.notifications_cleanup_enabled {
            return Ok(());
        }
        let last_run = self
            .get(Self::NOTIFICATION_PURGE_LAST_RUN_KEY)
            .await?
            .and_then(|bytes| String::from_utf8(bytes).ok())
            .and_then(|s| s.parse::<i64>().ok())
            .unwrap_or(0);

        if last_run > 0 && (now - last_run) < lifecycle.notification_cleanup_interval_secs as i64 {
            return Ok(());
        }

        let _ = self
            .set(
                Self::NOTIFICATION_PURGE_LAST_RUN_KEY,
                now.to_string().as_bytes(),
            )
            .await;

        if lifecycle.notifications_retention_days == 0 {
            return Ok(());
        }

        let cutoff = (chrono::Utc::now()
            - chrono::Duration::days(lifecycle.notifications_retention_days as i64))
        .to_rfc3339();

        let result = notification::Entity::delete_many()
            .filter(notification::Column::CreatedAt.lt(cutoff))
            .exec(&self.db)
            .await?;

        if result.rows_affected > 0 {
            tracing::info!(
                "Purged {} notifications older than {} days",
                result.rows_affected,
                lifecycle.notifications_retention_days
            );
        }

        Ok(())
    }

    /// Insert a notification
    pub async fn insert_notification(&self, notif: &notification::Model) -> Result<()> {
        if let Err(e) = self.maybe_purge_old_notifications().await {
            tracing::warn!("Notification retention purge failed: {}", e);
        }

        // Normalize fields to improve dedup reliability (avoid whitespace/case variants).
        let title_clean = notif.title.trim().to_string();
        let body_clean = notif.body.trim().to_string();
        let encrypted_title = encrypt_storage_string(&title_clean)?;
        let encrypted_body = encrypt_storage_string(&body_clean)?;
        let level_clean = notif.level.trim().to_string();
        let source_clean = notif.source.trim().to_string();
        let body_sig = Self::notification_body_signature(&body_clean);

        if Self::is_arkpulse_notification(&source_clean) {
            let recent = notification::Entity::find()
                .filter(notification::Column::Source.eq(source_clean.clone()))
                .filter(notification::Column::CreatedAt.gte(Self::arkpulse_recent_cutoff_rfc3339()))
                .order_by_desc(notification::Column::CreatedAt)
                .limit(25)
                .all(&self.db)
                .await?;
            for existing in recent {
                let existing_title = decrypt_storage_string(&existing.title);
                let existing_body = decrypt_storage_string(&existing.body);
                if existing_title == title_clean
                    && Self::notification_body_signature(&existing_body) == body_sig
                {
                    return Ok(());
                }
            }
        }

        // Best-effort deduplication to prevent repeated notifications from flooding the DB/UI.
        // Critical/security notifications bypass dedup.
        if !Self::notification_is_critical(&level_clean, &source_clean, &title_clean) {
            let cutoff = (chrono::Utc::now()
                - chrono::Duration::days(Self::NOTIFICATION_DEDUP_COOLDOWN_DAYS))
            .to_rfc3339();
            match notification::Entity::find()
                .filter(notification::Column::CreatedAt.gte(cutoff))
                .filter(notification::Column::Source.eq(source_clean.clone()))
                .order_by_desc(notification::Column::CreatedAt)
                .limit(50)
                .all(&self.db)
                .await
            {
                Ok(recent) => {
                    for existing in recent {
                        let existing_title = decrypt_storage_string(&existing.title);
                        let existing_body = decrypt_storage_string(&existing.body);
                        if existing_title == title_clean
                            && Self::notification_body_signature(&existing_body) == body_sig
                        {
                            // Suppress duplicates within the cooldown window.
                            return Ok(());
                        }
                    }
                }
                Err(e) => {
                    tracing::warn!("Notification dedup lookup failed: {}", e);
                }
            }
        }

        notification::ActiveModel {
            id: Set(notif.id.clone()),
            title: Set(encrypted_title),
            body: Set(encrypted_body),
            level: Set(level_clean),
            source: Set(source_clean),
            read: Set(notif.read),
            created_at: Set(notif.created_at.clone()),
        }
        .insert(&self.db)
        .await?;
        Ok(())
    }

    /// List notifications (newest first, paginated)
    pub async fn list_notifications(
        &self,
        limit: u64,
        offset: u64,
        unread_only: bool,
    ) -> Result<Vec<notification::Model>> {
        if let Err(e) = self.maybe_purge_old_notifications().await {
            tracing::warn!("Notification retention purge failed: {}", e);
        }
        let mut query = notification::Entity::find()
            .filter(notification::Column::Source.ne("arkpulse"))
            .order_by_desc(notification::Column::CreatedAt);
        if unread_only {
            query = query.filter(notification::Column::Read.eq(false));
        }
        let mut notifs = query
            .limit(Self::db_limit(limit))
            .offset(Self::db_offset(offset))
            .all(&self.db)
            .await?;
        for notif in &mut notifs {
            notif.title = decrypt_storage_string(&notif.title);
            notif.body = decrypt_storage_string(&notif.body);
        }
        Ok(notifs)
    }

    /// Count notifications
    pub async fn count_notifications(&self, unread_only: bool) -> Result<u64> {
        if let Err(e) = self.maybe_purge_old_notifications().await {
            tracing::warn!("Notification retention purge failed: {}", e);
        }
        let mut query =
            notification::Entity::find().filter(notification::Column::Source.ne("arkpulse"));
        if unread_only {
            query = query.filter(notification::Column::Read.eq(false));
        }
        query.count(&self.db).await.map_err(Into::into)
    }

    /// Mark notification as read
    pub async fn mark_notification_read(&self, id: &str) -> Result<()> {
        notification::ActiveModel {
            id: Set(id.to_string()),
            read: Set(true),
            ..Default::default()
        }
        .update(&self.db)
        .await?;
        Ok(())
    }

    /// Set notification read flag explicitly
    pub async fn set_notification_read(&self, id: &str, read: bool) -> Result<()> {
        notification::ActiveModel {
            id: Set(id.to_string()),
            read: Set(read),
            ..Default::default()
        }
        .update(&self.db)
        .await?;
        Ok(())
    }

    /// Mark all notifications as read
    pub async fn mark_all_notifications_read(&self) -> Result<()> {
        notification::Entity::update_many()
            .col_expr(notification::Column::Read, Expr::value(true))
            .exec(&self.db)
            .await?;
        Ok(())
    }

    /// Delete goal-related notifications that reference a specific goal text.
    pub async fn delete_goal_notifications(&self, goal_text: &str) -> Result<u64> {
        let trimmed = goal_text.trim();
        if trimmed.is_empty() {
            return Ok(0);
        }

        let source_filter = Condition::any()
            .add(notification::Column::Source.contains("goal"))
            .add(notification::Column::Source.eq("autonomy_goal_loop"));
        let candidates = notification::Entity::find()
            .filter(source_filter)
            .all(&self.db)
            .await?;
        let matching_ids = candidates
            .into_iter()
            .filter(|notif| {
                decrypt_storage_string(&notif.title).contains(trimmed)
                    || decrypt_storage_string(&notif.body).contains(trimmed)
            })
            .map(|notif| notif.id)
            .collect::<Vec<_>>();
        if matching_ids.is_empty() {
            return Ok(0);
        }
        let result = notification::Entity::delete_many()
            .filter(notification::Column::Id.is_in(matching_ids))
            .exec(&self.db)
            .await?;
        Ok(result.rows_affected)
    }

    /// Delete app-related notifications that reference a specific app id/title.
    pub async fn delete_app_notifications(
        &self,
        app_id: &str,
        app_title: Option<&str>,
    ) -> Result<u64> {
        let id_trimmed = app_id.trim();
        if id_trimmed.is_empty() {
            return Ok(0);
        }

        let source_filter = Condition::any()
            .add(notification::Column::Source.contains("app"))
            .add(notification::Column::Title.contains("App"))
            .add(notification::Column::Title.contains("app"));
        let titles = app_title
            .map(str::trim)
            .filter(|s| !s.is_empty())
            .map(|title| vec![id_trimmed.to_string(), title.to_string()])
            .unwrap_or_else(|| vec![id_trimmed.to_string()]);
        let candidates = notification::Entity::find()
            .filter(source_filter)
            .all(&self.db)
            .await?;
        let matching_ids = candidates
            .into_iter()
            .filter(|notif| {
                let title = decrypt_storage_string(&notif.title);
                let body = decrypt_storage_string(&notif.body);
                titles
                    .iter()
                    .any(|needle| title.contains(needle) || body.contains(needle))
            })
            .map(|notif| notif.id)
            .collect::<Vec<_>>();
        if matching_ids.is_empty() {
            return Ok(0);
        }
        let result = notification::Entity::delete_many()
            .filter(notification::Column::Id.is_in(matching_ids))
            .exec(&self.db)
            .await?;
        Ok(result.rows_affected)
    }

    /// Count unread notifications
    pub async fn count_unread_notifications(&self) -> Result<u64> {
        if let Err(e) = self.maybe_purge_old_notifications().await {
            tracing::warn!("Notification retention purge failed: {}", e);
        }
        notification::Entity::find()
            .filter(notification::Column::Source.ne("arkpulse"))
            .filter(notification::Column::Read.eq(false))
            .count(&self.db)
            .await
            .map_err(Into::into)
    }

    // ==================== Approval Log ====================

    /// Get approval log (paginated, newest first)
    pub async fn get_approval_log(
        &self,
        limit: u64,
        offset: u64,
    ) -> Result<Vec<approval_log::Model>> {
        let mut log = approval_log::Entity::find()
            .order_by_desc(approval_log::Column::RequestedAt)
            .limit(Self::db_limit(limit))
            .offset(Self::db_offset(offset))
            .all(&self.db)
            .await?;
        for row in &mut log {
            row.arguments = decrypt_storage_string(&row.arguments);
        }
        Ok(log)
    }

    /// Get a single approval request by id with decrypted arguments.
    pub async fn get_approval_request(&self, id: &str) -> Result<Option<approval_log::Model>> {
        let mut row = approval_log::Entity::find_by_id(id.to_string())
            .one(&self.db)
            .await?;
        if let Some(row) = &mut row {
            row.arguments = decrypt_storage_string(&row.arguments);
        }
        Ok(row)
    }

    /// Create or refresh a pending approval request entry.
    pub async fn upsert_approval_request(
        &self,
        id: &str,
        action_name: &str,
        arguments: &str,
        rule_name: &str,
        requested_at: &str,
    ) -> Result<()> {
        let arguments = encrypt_storage_string(arguments)?;
        approval_log::Entity::insert(approval_log::ActiveModel {
            id: Set(id.to_string()),
            action_name: Set(action_name.to_string()),
            arguments: Set(arguments),
            rule_name: Set(rule_name.to_string()),
            status: Set("pending".to_string()),
            requested_at: Set(requested_at.to_string()),
            resolved_at: Set(None),
            resolved_by: Set(None),
        })
        .on_conflict(
            OnConflict::column(approval_log::Column::Id)
                .update_columns([
                    approval_log::Column::ActionName,
                    approval_log::Column::Arguments,
                    approval_log::Column::RuleName,
                    approval_log::Column::Status,
                    approval_log::Column::RequestedAt,
                    approval_log::Column::ResolvedAt,
                    approval_log::Column::ResolvedBy,
                ])
                .to_owned(),
        )
        .exec(&self.db)
        .await?;
        Ok(())
    }

    /// Resolve an approval request entry.
    pub async fn resolve_approval_request(
        &self,
        id: &str,
        status: &str,
        resolved_by: &str,
    ) -> Result<()> {
        let existing = approval_log::Entity::find_by_id(id.to_string())
            .one(&self.db)
            .await?;
        if existing.is_none() {
            return Ok(());
        }
        approval_log::ActiveModel {
            id: Set(id.to_string()),
            status: Set(status.to_string()),
            resolved_at: Set(Some(chrono::Utc::now().to_rfc3339())),
            resolved_by: Set(Some(resolved_by.to_string())),
            ..Default::default()
        }
        .update(&self.db)
        .await?;
        Ok(())
    }

    // ==================== Execution Traces ====================

    #[allow(dead_code)]
    pub async fn insert_execution_proof(
        &self,
        proof: &crate::proofs::ExecutionProof,
    ) -> Result<()> {
        crate::storage::entities::execution_proof::ActiveModel {
            id: Set(proof.id.to_string()),
            action_hash: Set(proof.action_hash.clone()),
            input_hash: Set(proof.input_hash.clone()),
            output_hash: Set(proof.output_hash.clone()),
            prev_hash: Set(proof.prev_hash.clone()),
            timestamp: Set(proof.timestamp.to_rfc3339()),
            signature: Set(proof.signature.clone()),
        }
        .insert(&self.db)
        .await?;
        Ok(())
    }

    /// Persist a completed execution trace for Trace history/detail views.
    pub async fn insert_execution_trace(&self, trace: &crate::core::ExecutionTrace) -> Result<()> {
        let duration_ms = trace.started_at.and_then(|start| {
            trace
                .completed_at
                .map(|end| (end - start).num_milliseconds())
        });
        let started_at = trace.started_at.map(|value| value.to_rfc3339());
        let completed_at = trace.completed_at.map(|value| value.to_rfc3339());
        let created_at = trace
            .completed_at
            .or(trace.started_at)
            .unwrap_or_else(chrono::Utc::now)
            .to_rfc3339();
        let message = encrypt_storage_string(&trace.message)?;
        let steps_json = encrypt_storage_string(&serde_json::to_string(&trace.steps)?)?;
        let response = encrypt_optional_storage_string(trace.response.as_deref())?;
        let insert_result = crate::storage::entities::execution_trace::Entity::insert(
            crate::storage::entities::execution_trace::ActiveModel {
                id: Set(trace.id.clone()),
                message: Set(message.clone()),
                channel: Set(trace.channel.clone()),
                started_at: Set(started_at.clone()),
                completed_at: Set(completed_at.clone()),
                duration_ms: Set(duration_ms.map(|v| v.min(i32::MAX as i64) as i32)),
                step_count: Set(trace.steps.len().min(i32::MAX as usize) as i32),
                steps_json: Set(steps_json.clone()),
                response: Set(response.clone()),
                proof_id: Set(trace.proof_id.clone()),
                model: Set(trace.model.clone()),
                input_tokens: Set(trace.input_tokens.min(i32::MAX as i64) as i32),
                output_tokens: Set(trace.output_tokens.min(i32::MAX as i64) as i32),
                total_tokens: Set(trace.total_tokens.min(i32::MAX as i64) as i32),
                cost_usd: Set(trace.cost_usd),
                complexity: Set(trace.complexity.clone()),
                created_at: Set(created_at.clone()),
            },
        )
        .on_conflict(
            OnConflict::column(crate::storage::entities::execution_trace::Column::Id)
                .update_columns([
                    crate::storage::entities::execution_trace::Column::Message,
                    crate::storage::entities::execution_trace::Column::Channel,
                    crate::storage::entities::execution_trace::Column::StartedAt,
                    crate::storage::entities::execution_trace::Column::CompletedAt,
                    crate::storage::entities::execution_trace::Column::DurationMs,
                    crate::storage::entities::execution_trace::Column::StepCount,
                    crate::storage::entities::execution_trace::Column::StepsJson,
                    crate::storage::entities::execution_trace::Column::Response,
                    crate::storage::entities::execution_trace::Column::ProofId,
                    crate::storage::entities::execution_trace::Column::Model,
                    crate::storage::entities::execution_trace::Column::InputTokens,
                    crate::storage::entities::execution_trace::Column::OutputTokens,
                    crate::storage::entities::execution_trace::Column::TotalTokens,
                    crate::storage::entities::execution_trace::Column::CostUsd,
                    crate::storage::entities::execution_trace::Column::Complexity,
                ])
                .to_owned(),
        )
        .exec(&self.db)
        .await;
        if let Err(error) = insert_result {
            if trace.proof_id.is_some() && is_foreign_key_constraint_error(&error) {
                tracing::warn!(
                    "Retrying trace insert '{}' without proof_id after FK failure: {}",
                    trace.id,
                    error
                );
                crate::storage::entities::execution_trace::Entity::insert(
                    crate::storage::entities::execution_trace::ActiveModel {
                        id: Set(trace.id.clone()),
                        message: Set(message),
                        channel: Set(trace.channel.clone()),
                        started_at: Set(started_at),
                        completed_at: Set(completed_at),
                        duration_ms: Set(duration_ms.map(|v| v.min(i32::MAX as i64) as i32)),
                        step_count: Set(trace.steps.len().min(i32::MAX as usize) as i32),
                        steps_json: Set(steps_json),
                        response: Set(response),
                        proof_id: Set(None),
                        model: Set(trace.model.clone()),
                        input_tokens: Set(trace.input_tokens.min(i32::MAX as i64) as i32),
                        output_tokens: Set(trace.output_tokens.min(i32::MAX as i64) as i32),
                        total_tokens: Set(trace.total_tokens.min(i32::MAX as i64) as i32),
                        cost_usd: Set(trace.cost_usd),
                        complexity: Set(trace.complexity.clone()),
                        created_at: Set(created_at),
                    },
                )
                .on_conflict(
                    OnConflict::column(crate::storage::entities::execution_trace::Column::Id)
                        .update_columns([
                            crate::storage::entities::execution_trace::Column::Message,
                            crate::storage::entities::execution_trace::Column::Channel,
                            crate::storage::entities::execution_trace::Column::StartedAt,
                            crate::storage::entities::execution_trace::Column::CompletedAt,
                            crate::storage::entities::execution_trace::Column::DurationMs,
                            crate::storage::entities::execution_trace::Column::StepCount,
                            crate::storage::entities::execution_trace::Column::StepsJson,
                            crate::storage::entities::execution_trace::Column::Response,
                            crate::storage::entities::execution_trace::Column::ProofId,
                            crate::storage::entities::execution_trace::Column::Model,
                            crate::storage::entities::execution_trace::Column::InputTokens,
                            crate::storage::entities::execution_trace::Column::OutputTokens,
                            crate::storage::entities::execution_trace::Column::TotalTokens,
                            crate::storage::entities::execution_trace::Column::CostUsd,
                            crate::storage::entities::execution_trace::Column::Complexity,
                        ])
                        .to_owned(),
                )
                .exec(&self.db)
                .await?;
            } else {
                return Err(error.into());
            }
        }
        Ok(())
    }

    /// List persisted execution trace summaries (newest first) without loading full responses.
    pub async fn list_execution_trace_summaries(
        &self,
        since: Option<&str>,
        limit: u64,
        offset: u64,
    ) -> Result<Vec<ExecutionTraceSummaryRow>> {
        let mut query = crate::storage::entities::execution_trace::Entity::find()
            .select_only()
            .columns([
                crate::storage::entities::execution_trace::Column::Id,
                crate::storage::entities::execution_trace::Column::Message,
                crate::storage::entities::execution_trace::Column::Channel,
                crate::storage::entities::execution_trace::Column::StartedAt,
                crate::storage::entities::execution_trace::Column::CompletedAt,
                crate::storage::entities::execution_trace::Column::DurationMs,
                crate::storage::entities::execution_trace::Column::StepCount,
                crate::storage::entities::execution_trace::Column::StepsJson,
                crate::storage::entities::execution_trace::Column::Model,
                crate::storage::entities::execution_trace::Column::TotalTokens,
                crate::storage::entities::execution_trace::Column::CostUsd,
                crate::storage::entities::execution_trace::Column::Complexity,
                crate::storage::entities::execution_trace::Column::CreatedAt,
            ])
            .order_by_desc(crate::storage::entities::execution_trace::Column::CreatedAt)
            .limit(Self::db_limit(limit))
            .offset(Self::db_offset(offset));
        if let Some(since) = since.map(str::trim).filter(|value| !value.is_empty()) {
            query = query.filter(
                crate::storage::entities::execution_trace::Column::CreatedAt.gte(since.to_string()),
            );
        }

        let mut traces = query
            .into_model::<ExecutionTraceSummaryRow>()
            .all(&self.db)
            .await?;
        for trace in &mut traces {
            trace.message = decrypt_storage_string(&trace.message);
            trace.steps_json = decrypt_storage_string(&trace.steps_json);
        }
        Ok(traces)
    }

    pub async fn count_execution_traces(&self, since: Option<&str>) -> Result<u64> {
        let mut query = crate::storage::entities::execution_trace::Entity::find();
        if let Some(since) = since.map(str::trim).filter(|value| !value.is_empty()) {
            query = query.filter(
                crate::storage::entities::execution_trace::Column::CreatedAt.gte(since.to_string()),
            );
        }
        Ok(query.count(&self.db).await?)
    }

    pub async fn count_execution_traces_by_ids(
        &self,
        since: Option<&str>,
        ids: &[String],
    ) -> Result<u64> {
        if ids.is_empty() {
            return Ok(0);
        }
        let mut query = crate::storage::entities::execution_trace::Entity::find()
            .filter(crate::storage::entities::execution_trace::Column::Id.is_in(ids.to_vec()));
        if let Some(since) = since.map(str::trim).filter(|value| !value.is_empty()) {
            query = query.filter(
                crate::storage::entities::execution_trace::Column::CreatedAt.gte(since.to_string()),
            );
        }
        Ok(query.count(&self.db).await?)
    }

    pub async fn get_execution_trace_message_metrics_by_ids(
        &self,
        ids: &[String],
    ) -> Result<std::collections::HashMap<String, ExecutionTraceMessageMetrics>> {
        if ids.is_empty() {
            return Ok(std::collections::HashMap::new());
        }
        let mut rows = crate::storage::entities::execution_trace::Entity::find()
            .select_only()
            .columns([
                crate::storage::entities::execution_trace::Column::Id,
                crate::storage::entities::execution_trace::Column::DurationMs,
                crate::storage::entities::execution_trace::Column::InputTokens,
                crate::storage::entities::execution_trace::Column::OutputTokens,
                crate::storage::entities::execution_trace::Column::TotalTokens,
                crate::storage::entities::execution_trace::Column::StepsJson,
            ])
            .filter(crate::storage::entities::execution_trace::Column::Id.is_in(ids.to_vec()))
            .into_model::<ExecutionTraceMessageMetricRow>()
            .all(&self.db)
            .await?;
        for row in &mut rows {
            row.steps_json = decrypt_storage_string(&row.steps_json);
        }
        Ok(rows
            .into_iter()
            .map(|row| {
                let (cached_prompt_tokens, cache_creation_prompt_tokens) =
                    trace_prompt_cache_metrics(&row.steps_json);
                let metrics = ExecutionTraceMessageMetrics {
                    duration_ms: row.duration_ms.map(i64::from),
                    input_tokens: i64::from(row.input_tokens),
                    output_tokens: i64::from(row.output_tokens),
                    total_tokens: i64::from(row.total_tokens),
                    cached_prompt_tokens,
                    cache_creation_prompt_tokens,
                    time_to_first_token_ms: trace_time_to_first_token_ms(&row.steps_json),
                };
                (row.id, metrics)
            })
            .collect())
    }

    /// Get a single persisted execution trace by id.
    pub async fn get_execution_trace(
        &self,
        id: &str,
    ) -> Result<Option<crate::storage::entities::execution_trace::Model>> {
        let mut trace =
            crate::storage::entities::execution_trace::Entity::find_by_id(id.to_string())
                .one(&self.db)
                .await?;
        if let Some(row) = trace.as_mut() {
            row.message = decrypt_storage_string(&row.message);
            row.steps_json = decrypt_storage_string(&row.steps_json);
            row.response = decrypt_optional_storage_string(row.response.take());
        }
        Ok(trace)
    }

    // ==================== Security Logs ====================

    /// Insert a security log entry
    pub async fn insert_security_log(&self, log: &security_log::Model) -> Result<()> {
        security_log::ActiveModel {
            id: Set(log.id.clone()),
            event_type: Set(log.event_type.clone()),
            severity: Set(log.severity.clone()),
            message: Set(encrypt_storage_string(&log.message)?),
            source: Set(encrypt_optional_storage_string(log.source.as_deref())?),
            count: Set(log.count),
            created_at: Set(log.created_at.clone()),
        }
        .insert(&self.db)
        .await?;
        Ok(())
    }

    /// Insert multiple security log entries atomically.
    pub async fn insert_security_logs(&self, logs: &[security_log::Model]) -> Result<()> {
        if logs.is_empty() {
            return Ok(());
        }

        let txn = self.db.begin().await?;
        for log in logs {
            security_log::ActiveModel {
                id: Set(log.id.clone()),
                event_type: Set(log.event_type.clone()),
                severity: Set(log.severity.clone()),
                message: Set(encrypt_storage_string(&log.message)?),
                source: Set(encrypt_optional_storage_string(log.source.as_deref())?),
                count: Set(log.count),
                created_at: Set(log.created_at.clone()),
            }
            .insert(&txn)
            .await?;
        }
        txn.commit().await?;
        Ok(())
    }

    /// List recent security logs (newest first)
    pub async fn list_security_logs(&self, limit: u64) -> Result<Vec<security_log::Model>> {
        let mut logs = security_log::Entity::find()
            .order_by_desc(security_log::Column::CreatedAt)
            .limit(Self::db_limit(limit))
            .all(&self.db)
            .await?;
        for log in &mut logs {
            log.message = decrypt_storage_string(&log.message);
            log.source = decrypt_optional_storage_string(log.source.clone());
        }
        Ok(logs)
    }

    /// List security logs with pagination and optional event-type filter.
    pub async fn list_security_logs_paginated(
        &self,
        limit: u64,
        offset: u64,
        event_type: Option<&str>,
    ) -> Result<Vec<security_log::Model>> {
        let mut query = security_log::Entity::find().order_by_desc(security_log::Column::CreatedAt);

        if let Some(et) = event_type.filter(|s| !s.trim().is_empty()) {
            query = query.filter(security_log::Column::EventType.eq(et.trim().to_string()));
        }

        let mut logs = query
            .limit(Self::db_limit(limit))
            .offset(Self::db_offset(offset))
            .all(&self.db)
            .await?;
        for log in &mut logs {
            log.message = decrypt_storage_string(&log.message);
            log.source = decrypt_optional_storage_string(log.source.clone());
        }
        Ok(logs)
    }

    /// Count security logs for pagination (optional event-type filter).
    pub async fn count_security_logs(&self, event_type: Option<&str>) -> Result<u64> {
        let mut query = security_log::Entity::find();
        if let Some(et) = event_type.filter(|s| !s.trim().is_empty()) {
            query = query.filter(security_log::Column::EventType.eq(et.trim().to_string()));
        }
        Ok(query.count(&self.db).await?)
    }

    /// Delete security logs older than the given number of days
    pub async fn cleanup_old_security_logs(&self, max_age_days: i64) -> Result<u64> {
        let cutoff = (chrono::Utc::now() - chrono::Duration::days(max_age_days)).to_rfc3339();
        let result = security_log::Entity::delete_many()
            .filter(security_log::Column::CreatedAt.lt(cutoff))
            .exec(&self.db)
            .await?;
        Ok(result.rows_affected)
    }

    // ==================== Pulse History ====================

    /// Insert an Pulse history event row.
    pub async fn insert_arkpulse_event(&self, event: &arkpulse_event::Model) -> Result<()> {
        arkpulse_event::Entity::insert(arkpulse_event::ActiveModel {
            id: Set(event.id.clone()),
            timestamp: Set(event.timestamp.clone()),
            status: Set(event.status.clone()),
            message: Set(encrypt_storage_string(&event.message)?),
            summary: Set(encrypt_storage_string(&event.summary)?),
            flags_json: Set(encrypt_storage_string(&event.flags_json)?),
            overdue_tasks: Set(event.overdue_tasks),
            failed_tasks: Set(event.failed_tasks),
            details_json: Set(encrypt_storage_string(&event.details_json)?),
        })
        .on_conflict(
            OnConflict::column(arkpulse_event::Column::Id)
                .do_nothing()
                .to_owned(),
        )
        .exec(&self.db)
        .await?;
        Ok(())
    }

    /// Count persisted Pulse history rows.
    pub async fn count_arkpulse_events(&self) -> Result<u64> {
        arkpulse_event::Entity::find()
            .count(&self.db)
            .await
            .map_err(Into::into)
    }

    /// List Pulse history rows (newest first).
    pub async fn list_arkpulse_events(&self, limit: u64) -> Result<Vec<arkpulse_event::Model>> {
        let mut rows = arkpulse_event::Entity::find()
            .order_by_desc(arkpulse_event::Column::Timestamp)
            .limit(Self::db_limit(limit))
            .all(&self.db)
            .await?;
        for row in &mut rows {
            row.message = decrypt_storage_string(&row.message);
            row.summary = decrypt_storage_string(&row.summary);
            row.flags_json = decrypt_storage_string(&row.flags_json);
            row.details_json = decrypt_storage_string(&row.details_json);
        }
        Ok(rows)
    }

    /// List Pulse history rows inside a bounded time window.
    #[allow(dead_code)]
    pub async fn list_arkpulse_events_between(
        &self,
        from_rfc3339: &str,
        to_rfc3339: &str,
        limit: u64,
    ) -> Result<Vec<arkpulse_event::Model>> {
        let mut rows = arkpulse_event::Entity::find()
            .filter(arkpulse_event::Column::Timestamp.gte(from_rfc3339.to_string()))
            .filter(arkpulse_event::Column::Timestamp.lt(to_rfc3339.to_string()))
            .order_by_desc(arkpulse_event::Column::Timestamp)
            .limit(Self::db_limit(limit))
            .all(&self.db)
            .await?;
        for row in &mut rows {
            row.message = decrypt_storage_string(&row.message);
            row.summary = decrypt_storage_string(&row.summary);
            row.flags_json = decrypt_storage_string(&row.flags_json);
            row.details_json = decrypt_storage_string(&row.details_json);
        }
        Ok(rows)
    }

    /// Delete Pulse history rows older than the provided cutoff.
    pub async fn delete_arkpulse_events_before(&self, cutoff_rfc3339: &str) -> Result<u64> {
        let result = arkpulse_event::Entity::delete_many()
            .filter(arkpulse_event::Column::Timestamp.lt(cutoff_rfc3339.to_string()))
            .exec(&self.db)
            .await?;
        Ok(result.rows_affected)
    }

    /// Delete Pulse history rows by explicit IDs.
    pub async fn delete_arkpulse_events_by_ids(&self, ids: &[String]) -> Result<u64> {
        if ids.is_empty() {
            return Ok(0);
        }
        let result = arkpulse_event::Entity::delete_many()
            .filter(arkpulse_event::Column::Id.is_in(ids.to_vec()))
            .exec(&self.db)
            .await?;
        Ok(result.rows_affected)
    }

    /// Return Pulse history IDs that exceed the latest retained window.
    pub async fn list_arkpulse_event_ids_beyond_latest(
        &self,
        keep_latest: u64,
    ) -> Result<Vec<String>> {
        let rows = arkpulse_event::Entity::find()
            .order_by_desc(arkpulse_event::Column::Timestamp)
            .offset(Self::db_offset(keep_latest))
            .all(&self.db)
            .await?;
        Ok(rows.into_iter().map(|row| row.id).collect())
    }

    // ==================== Operational Logs ====================

    /// Insert a structured operational telemetry entry.
    pub async fn insert_operational_log(&self, log: &operational_log::Model) -> Result<()> {
        let trace_id = match log
            .trace_id
            .as_deref()
            .map(str::trim)
            .filter(|id| !id.is_empty())
        {
            Some(id) => match execution_trace::Entity::find_by_id(id.to_string())
                .one(&self.db)
                .await
            {
                Ok(Some(_)) => Some(id.to_string()),
                Ok(None) => {
                    tracing::debug!(
                        "Dropping operational log trace_id before insert because it does not resolve to an execution trace"
                    );
                    None
                }
                Err(error) => {
                    tracing::warn!(
                        "Dropping operational log trace_id before insert because validation failed: {}",
                        error
                    );
                    None
                }
            },
            None => None,
        };
        let conversation_id = match log
            .conversation_id
            .as_deref()
            .map(str::trim)
            .filter(|id| !id.is_empty())
        {
            Some(id) => match conversation::Entity::find_by_id(id.to_string())
                .one(&self.db)
                .await
            {
                Ok(Some(_)) => Some(id.to_string()),
                Ok(None) => {
                    tracing::debug!(
                        "Dropping operational log conversation_id before insert because it does not resolve to a conversation"
                    );
                    None
                }
                Err(error) => {
                    tracing::warn!(
                        "Dropping operational log conversation_id before insert because validation failed: {}",
                        error
                    );
                    None
                }
            },
            None => None,
        };
        let insert_result = operational_log::ActiveModel {
            id: Set(log.id.clone()),
            created_at: Set(log.created_at.clone()),
            trace_id: Set(trace_id.clone()),
            conversation_id: Set(conversation_id.clone()),
            channel: Set(log.channel.clone()),
            event_type: Set(log.event_type.clone()),
            success: Set(log.success),
            outcome: Set(encrypt_storage_string(&log.outcome)?),
            tool_name: Set(log.tool_name.clone()),
            latency_ms: Set(log.latency_ms),
            arguments: Set(encrypt_optional_storage_string(log.arguments.as_deref())?),
            payload: Set(encrypt_optional_storage_string(log.payload.as_deref())?),
            strategy_version: Set(log.strategy_version.clone()),
            policy_version: Set(log.policy_version.clone()),
            prompt_version: Set(log.prompt_version.clone()),
            model_slot: Set(log.model_slot.clone()),
        }
        .insert(&self.db)
        .await;
        if let Err(error) = insert_result {
            if (trace_id.is_some() || conversation_id.is_some())
                && is_foreign_key_constraint_error(&error)
            {
                tracing::warn!(
                    "Retrying operational log insert '{}' without trace_id/conversation_id after FK failure: {}",
                    log.id,
                    error
                );
                operational_log::ActiveModel {
                    id: Set(log.id.clone()),
                    created_at: Set(log.created_at.clone()),
                    trace_id: Set(None),
                    conversation_id: Set(None),
                    channel: Set(log.channel.clone()),
                    event_type: Set(log.event_type.clone()),
                    success: Set(log.success),
                    outcome: Set(encrypt_storage_string(&log.outcome)?),
                    tool_name: Set(log.tool_name.clone()),
                    latency_ms: Set(log.latency_ms),
                    arguments: Set(encrypt_optional_storage_string(log.arguments.as_deref())?),
                    payload: Set(encrypt_optional_storage_string(log.payload.as_deref())?),
                    strategy_version: Set(log.strategy_version.clone()),
                    policy_version: Set(log.policy_version.clone()),
                    prompt_version: Set(log.prompt_version.clone()),
                    model_slot: Set(log.model_slot.clone()),
                }
                .insert(&self.db)
                .await?;
            } else {
                return Err(error.into());
            }
        }
        Ok(())
    }

    /// List operational logs by event type (newest first).
    pub async fn list_operational_logs_by_event(
        &self,
        event_type: &str,
        limit: u64,
    ) -> Result<Vec<operational_log::Model>> {
        let mut rows = operational_log::Entity::find()
            .filter(operational_log::Column::EventType.eq(event_type.to_string()))
            .order_by_desc(operational_log::Column::CreatedAt)
            .limit(Self::db_limit(limit))
            .all(&self.db)
            .await?;
        for row in &mut rows {
            row.outcome = decrypt_storage_string(&row.outcome);
            row.arguments = decrypt_optional_storage_string(row.arguments.clone());
            row.payload = decrypt_optional_storage_string(row.payload.clone());
        }
        Ok(rows)
    }

    /// List recent operational logs across AgentArk modules (newest first).
    pub async fn list_recent_operational_logs(
        &self,
        since: Option<&str>,
        limit: u64,
        offset: u64,
    ) -> Result<Vec<operational_log::Model>> {
        let mut query = operational_log::Entity::find()
            .order_by_desc(operational_log::Column::CreatedAt)
            .limit(Self::db_limit(limit))
            .offset(Self::db_offset(offset));
        if let Some(since) = since.map(str::trim).filter(|value| !value.is_empty()) {
            query = query.filter(operational_log::Column::CreatedAt.gte(since.to_string()));
        }

        let mut rows = query.all(&self.db).await?;
        for row in &mut rows {
            row.outcome = decrypt_storage_string(&row.outcome);
            row.arguments = decrypt_optional_storage_string(row.arguments.clone());
            row.payload = decrypt_optional_storage_string(row.payload.clone());
        }
        Ok(rows)
    }

    pub async fn count_operational_logs(&self, since: Option<&str>) -> Result<u64> {
        let mut query = operational_log::Entity::find();
        if let Some(since) = since.map(str::trim).filter(|value| !value.is_empty()) {
            query = query.filter(operational_log::Column::CreatedAt.gte(since.to_string()));
        }
        Ok(query.count(&self.db).await?)
    }

    /// List recent operational logs for a set of trace ids (newest first).
    pub async fn list_operational_logs_for_trace_ids(
        &self,
        trace_ids: &[String],
        limit: u64,
    ) -> Result<Vec<operational_log::Model>> {
        if trace_ids.is_empty() {
            return Ok(Vec::new());
        }

        let mut rows = operational_log::Entity::find()
            .filter(operational_log::Column::TraceId.is_in(trace_ids.to_vec()))
            .order_by_desc(operational_log::Column::CreatedAt)
            .limit(Self::db_limit(limit))
            .all(&self.db)
            .await?;
        for row in &mut rows {
            row.outcome = decrypt_storage_string(&row.outcome);
            row.arguments = decrypt_optional_storage_string(row.arguments.clone());
            row.payload = decrypt_optional_storage_string(row.payload.clone());
        }
        Ok(rows)
    }

    /// List recent operational logs for a set of trace ids and one event type (newest first).
    pub async fn list_operational_logs_for_trace_ids_by_event(
        &self,
        trace_ids: &[String],
        event_type: &str,
        limit: u64,
    ) -> Result<Vec<operational_log::Model>> {
        if trace_ids.is_empty() {
            return Ok(Vec::new());
        }

        let mut rows = operational_log::Entity::find()
            .filter(operational_log::Column::TraceId.is_in(trace_ids.to_vec()))
            .filter(operational_log::Column::EventType.eq(event_type.to_string()))
            .order_by_desc(operational_log::Column::CreatedAt)
            .limit(Self::db_limit(limit))
            .all(&self.db)
            .await?;
        for row in &mut rows {
            row.outcome = decrypt_storage_string(&row.outcome);
            row.arguments = decrypt_optional_storage_string(row.arguments.clone());
            row.payload = decrypt_optional_storage_string(row.payload.clone());
        }
        Ok(rows)
    }

    async fn database_column_schema_rows(&self) -> Result<Vec<DatabaseColumnSchemaRow>> {
        let columns_alias = Alias::new("columns");
        let query = Query::select()
            .columns([
                (columns_alias.clone(), Alias::new("table_schema")),
                (columns_alias.clone(), Alias::new("table_name")),
                (columns_alias.clone(), Alias::new("column_name")),
                (columns_alias.clone(), Alias::new("data_type")),
                (columns_alias.clone(), Alias::new("udt_name")),
                (columns_alias.clone(), Alias::new("is_nullable")),
                (columns_alias.clone(), Alias::new("column_default")),
                (columns_alias.clone(), Alias::new("ordinal_position")),
            ])
            .from((Alias::new("information_schema"), columns_alias.clone()))
            .and_where(Expr::col((columns_alias.clone(), Alias::new("table_schema"))).eq("public"))
            .order_by(
                (columns_alias.clone(), Alias::new("table_name")),
                Order::Asc,
            )
            .order_by(
                (columns_alias.clone(), Alias::new("ordinal_position")),
                Order::Asc,
            )
            .to_owned();
        let rows = self.db.query_all(DbBackend::Postgres.build(&query)).await?;
        rows.into_iter()
            .map(|row| DatabaseColumnSchemaRow::from_query_result(&row, "").map_err(Into::into))
            .collect()
    }

    async fn database_column_names_for_table(&self, table: &str) -> Result<Vec<String>> {
        let table = normalize_public_table_name(table)?;
        Ok(self
            .database_column_schema_rows()
            .await?
            .into_iter()
            .filter(|row| row.table_schema == "public" && row.table_name == table)
            .map(|row| row.column_name)
            .collect())
    }

    fn build_structured_db_filter_expr(
        table_alias: &str,
        filter: &ReadonlyTableFilter,
    ) -> Result<SimpleExpr> {
        let column = normalize_db_column_name(&filter.column)?;
        let op = filter.op.trim().to_ascii_lowercase();
        let expr = Expr::col((Alias::new(table_alias), Alias::new(column.as_str())));
        match op.as_str() {
            "eq" => Ok(expr.eq(json_scalar_to_simple_expr(
                filter
                    .value
                    .as_ref()
                    .ok_or_else(|| anyhow!("Filter '{}' requires a value", filter.column))?,
            )?)),
            "neq" => Ok(expr.ne(json_scalar_to_simple_expr(
                filter
                    .value
                    .as_ref()
                    .ok_or_else(|| anyhow!("Filter '{}' requires a value", filter.column))?,
            )?)),
            "gt" => Ok(expr.gt(json_scalar_to_simple_expr(
                filter
                    .value
                    .as_ref()
                    .ok_or_else(|| anyhow!("Filter '{}' requires a value", filter.column))?,
            )?)),
            "gte" => Ok(expr.gte(json_scalar_to_simple_expr(
                filter
                    .value
                    .as_ref()
                    .ok_or_else(|| anyhow!("Filter '{}' requires a value", filter.column))?,
            )?)),
            "lt" => Ok(expr.lt(json_scalar_to_simple_expr(
                filter
                    .value
                    .as_ref()
                    .ok_or_else(|| anyhow!("Filter '{}' requires a value", filter.column))?,
            )?)),
            "lte" => Ok(expr.lte(json_scalar_to_simple_expr(
                filter
                    .value
                    .as_ref()
                    .ok_or_else(|| anyhow!("Filter '{}' requires a value", filter.column))?,
            )?)),
            "contains" => {
                let value = filter
                    .value
                    .as_ref()
                    .and_then(|value| value.as_str())
                    .ok_or_else(|| anyhow!("Filter '{}' requires a string value", filter.column))?;
                Ok(expr.like(format!("%{}%", value)))
            }
            "starts_with" => {
                let value = filter
                    .value
                    .as_ref()
                    .and_then(|value| value.as_str())
                    .ok_or_else(|| anyhow!("Filter '{}' requires a string value", filter.column))?;
                Ok(expr.like(format!("{}%", value)))
            }
            "ends_with" => {
                let value = filter
                    .value
                    .as_ref()
                    .and_then(|value| value.as_str())
                    .ok_or_else(|| anyhow!("Filter '{}' requires a string value", filter.column))?;
                Ok(expr.like(format!("%{}", value)))
            }
            "in" => {
                let values = filter
                    .value
                    .as_ref()
                    .and_then(|value| value.as_array())
                    .ok_or_else(|| anyhow!("Filter '{}' requires an array value", filter.column))?
                    .iter()
                    .map(json_scalar_to_simple_expr)
                    .collect::<Result<Vec<_>>>()?;
                if values.is_empty() {
                    anyhow::bail!("Filter '{}' requires a non-empty array", filter.column);
                }
                Ok(expr.is_in(values))
            }
            "is_null" => Ok(expr.is_null()),
            "not_null" => Ok(expr.is_not_null()),
            _ => anyhow::bail!(
                "Unsupported filter operator '{}'. Use eq, neq, gt, gte, lt, lte, contains, starts_with, ends_with, in, is_null, or not_null",
                filter.op
            ),
        }
    }

    /// Inspect the live Postgres schema for agent-facing diagnostics.
    pub async fn inspect_postgres_schema_json(
        &self,
        table_filter: Option<&str>,
        limit: u64,
    ) -> Result<serde_json::Value> {
        let filter = table_filter
            .map(str::trim)
            .filter(|value| !value.is_empty())
            .map(|value| value.to_ascii_lowercase());
        let mut tables = Vec::new();
        let mut grouped =
            std::collections::BTreeMap::<(String, String), Vec<DatabaseColumnSchemaRow>>::new();
        for row in self.database_column_schema_rows().await? {
            if let Some(filter) = filter.as_deref() {
                let table_name = row.table_name.to_ascii_lowercase();
                let schema_name = row.table_schema.to_ascii_lowercase();
                if !table_name.contains(filter) && !schema_name.contains(filter) {
                    continue;
                }
            }
            grouped
                .entry((row.table_schema.clone(), row.table_name.clone()))
                .or_default()
                .push(row);
        }
        for ((schema, table), mut columns) in grouped.into_iter().take(limit.clamp(1, 100) as usize)
        {
            columns.sort_by_key(|row| row.ordinal_position);
            tables.push(serde_json::json!({
                "schema": schema,
                "table": table,
                "columns": columns.into_iter().map(|column| serde_json::json!({
                    "name": column.column_name,
                    "type": column.data_type,
                    "udt_name": column.udt_name,
                    "nullable": column.is_nullable.eq_ignore_ascii_case("YES"),
                    "default": column.column_default,
                    "ordinal_position": column.ordinal_position,
                })).collect::<Vec<_>>(),
            }));
        }

        Ok(serde_json::json!({
            "generated_at": chrono::Utc::now().to_rfc3339(),
            "schema": "public",
            "table_filter": table_filter.map(str::trim).filter(|value| !value.is_empty()),
            "table_count": tables.len(),
            "tables": tables,
            "relationships": Vec::<serde_json::Value>::new(),
            "notes": [
                "Only public-schema AgentArk tables are exposed here.",
                "Use the returned table and column names with structured postgres_query_readonly calls."
            ],
        }))
    }

    /// Execute a structured, read-only table query against the live Postgres database.
    pub async fn query_table_json(
        &self,
        request: &ReadonlyTableQuery,
    ) -> Result<serde_json::Value> {
        let table = normalize_public_table_name(&request.table)?;
        let known_tables = self.database_table_names().await?;
        if !known_tables.iter().any(|name| name == &table) {
            anyhow::bail!(
                "Unknown table '{}'. Inspect the live schema with postgres_schema_inspect and retry with a valid public table name",
                table
            );
        }

        let available_columns = self.database_column_names_for_table(&table).await?;
        if available_columns.is_empty() {
            anyhow::bail!("Table '{}' has no readable columns", table);
        }

        let mut selected_columns = Vec::new();
        let mut seen = std::collections::HashSet::new();
        let requested_columns = if request.columns.is_empty() {
            available_columns.clone()
        } else {
            request
                .columns
                .iter()
                .map(|column| normalize_db_column_name(column))
                .collect::<Result<Vec<_>>>()?
        };
        for column in requested_columns {
            if !available_columns.iter().any(|name| name == &column) {
                anyhow::bail!(
                    "Unknown column '{}.{}'. Inspect the live schema with postgres_schema_inspect and retry with a valid column name",
                    table,
                    column
                );
            }
            if seen.insert(column.clone()) {
                selected_columns.push(column);
            }
        }

        for filter in &request.filters {
            let column = normalize_db_column_name(&filter.column)?;
            if !available_columns.iter().any(|name| name == &column) {
                anyhow::bail!(
                    "Unknown filter column '{}.{}'. Inspect the live schema with postgres_schema_inspect and retry with a valid column name",
                    table,
                    column
                );
            }
        }
        for sort in &request.order_by {
            let column = normalize_db_column_name(&sort.column)?;
            if !available_columns.iter().any(|name| name == &column) {
                anyhow::bail!(
                    "Unknown sort column '{}.{}'. Inspect the live schema with postgres_schema_inspect and retry with a valid column name",
                    table,
                    column
                );
            }
        }

        let table_alias = "t";
        let mut json_object = Func::cust(Alias::new("jsonb_build_object"));
        for column in &selected_columns {
            json_object = json_object.arg(column.clone()).arg(Expr::col((
                Alias::new(table_alias),
                Alias::new(column.as_str()),
            )));
        }

        let mut query = Query::select();
        query
            .expr_as(json_object, Alias::new("row_json"))
            .from_as(Alias::new(table.as_str()), Alias::new(table_alias));
        for filter in &request.filters {
            query.and_where(Self::build_structured_db_filter_expr(table_alias, filter)?);
        }
        for sort in &request.order_by {
            let column = normalize_db_column_name(&sort.column)?;
            let direction = sort.direction.as_deref().unwrap_or("asc");
            query.order_by(
                (Alias::new(table_alias), Alias::new(column.as_str())),
                if direction.eq_ignore_ascii_case("desc") {
                    Order::Desc
                } else {
                    Order::Asc
                },
            );
        }
        let applied_limit = request.limit.unwrap_or(50).clamp(1, 200);
        query.limit(applied_limit);

        let rendered_sql = query.to_string(PostgresQueryBuilder);
        let statement = DbBackend::Postgres.build(&query);
        let rows = self.db.query_all(statement).await?;
        let mut json_rows = Vec::with_capacity(rows.len());
        for row in rows {
            if let Ok(value) = row.try_get::<serde_json::Value>("", "row_json") {
                json_rows.push(value);
                continue;
            }
            let fallback = row
                .try_get::<String>("", "row_json")
                .ok()
                .and_then(|value| serde_json::from_str::<serde_json::Value>(&value).ok())
                .ok_or_else(|| anyhow!("Failed to decode structured row JSON"))?;
            json_rows.push(fallback);
        }

        Ok(serde_json::json!({
            "generated_at": chrono::Utc::now().to_rfc3339(),
            "schema": "public",
            "table": table,
            "selected_columns": selected_columns,
            "filters": request.filters,
            "order_by": request.order_by,
            "applied_limit": applied_limit,
            "sql": rendered_sql,
            "row_count": json_rows.len(),
            "rows": json_rows,
        }))
    }

    pub async fn list_operational_log_version_metrics_by_event(
        &self,
        event_type: &str,
        limit: u64,
    ) -> Result<Vec<OperationalLogVersionMetricRow>> {
        operational_log::Entity::find()
            .select_only()
            .columns([
                operational_log::Column::Success,
                operational_log::Column::LatencyMs,
                operational_log::Column::PolicyVersion,
                operational_log::Column::StrategyVersion,
            ])
            .filter(operational_log::Column::EventType.eq(event_type.to_string()))
            .order_by_desc(operational_log::Column::CreatedAt)
            .limit(Self::db_limit(limit))
            .into_model::<OperationalLogVersionMetricRow>()
            .all(&self.db)
            .await
            .map_err(Into::into)
    }

    /// Expire old pending approvals (older than max_age_secs)
    pub async fn expire_old_approvals(&self, max_age_secs: i64) -> Result<u64> {
        let cutoff = (chrono::Utc::now() - chrono::Duration::seconds(max_age_secs)).to_rfc3339();
        let resolved_at = chrono::Utc::now().to_rfc3339();
        let result = approval_log::Entity::update_many()
            .col_expr(approval_log::Column::Status, Expr::value("expired"))
            .col_expr(approval_log::Column::ResolvedAt, Expr::value(resolved_at))
            .col_expr(
                approval_log::Column::ResolvedBy,
                Expr::value("auto_timeout"),
            )
            .filter(approval_log::Column::Status.eq("pending"))
            .filter(approval_log::Column::RequestedAt.lt(cutoff))
            .exec(&self.db)
            .await?;
        Ok(result.rows_affected)
    }

    const HOUSEKEEPING_PURGE_BATCH_SIZE: i64 = 1_000;

    async fn delete_by_cutoff_in_batches(
        &self,
        table_name: &str,
        id_column: &str,
        cutoff_column: &str,
        cutoff: &str,
        extra_predicate_sql: &str,
    ) -> Result<u64> {
        let sql = format!(
            "DELETE FROM {table_name} \
             WHERE {id_column} IN ( \
                SELECT {id_column} \
                FROM {table_name} \
                WHERE {cutoff_column} < $1 {extra_predicate_sql} \
                ORDER BY {cutoff_column} ASC \
                LIMIT $2 \
             )"
        );
        let mut total_deleted = 0u64;
        loop {
            let result = self
                .db
                .execute(Statement::from_sql_and_values(
                    DbBackend::Postgres,
                    sql.clone(),
                    vec![
                        cutoff.to_string().into(),
                        Self::HOUSEKEEPING_PURGE_BATCH_SIZE.into(),
                    ],
                ))
                .await?;
            let deleted = result.rows_affected();
            total_deleted = total_deleted.saturating_add(deleted);
            if deleted == 0 {
                break;
            }
        }
        Ok(total_deleted)
    }

    async fn delete_rows_by_ids<C>(
        conn: &C,
        table_name: &str,
        id_column: &str,
        ids: &[String],
    ) -> Result<()>
    where
        C: ConnectionTrait,
    {
        if ids.is_empty() {
            return Ok(());
        }
        let placeholders = ids
            .iter()
            .enumerate()
            .map(|(idx, _)| format!("${}", idx + 1))
            .collect::<Vec<_>>()
            .join(", ");
        let sql = format!("DELETE FROM {table_name} WHERE {id_column} IN ({placeholders})");
        let values = ids
            .iter()
            .cloned()
            .map(Into::into)
            .collect::<Vec<sea_orm::Value>>();
        conn.execute(Statement::from_sql_and_values(
            DbBackend::Postgres,
            sql,
            values,
        ))
        .await?;
        Ok(())
    }

    async fn recount_conversations_after_message_batch<C>(
        conn: &C,
        conversation_ids: &[String],
        message_cutoff: &str,
    ) -> Result<()>
    where
        C: ConnectionTrait,
    {
        if conversation_ids.is_empty() {
            return Ok(());
        }

        let value_rows = conversation_ids
            .iter()
            .enumerate()
            .map(|(idx, _)| format!("(${})", idx + 1))
            .collect::<Vec<_>>()
            .join(", ");
        let values = conversation_ids
            .iter()
            .cloned()
            .map(Into::into)
            .collect::<Vec<sea_orm::Value>>();
        let update_sql = format!(
            "UPDATE conversations AS c \
             SET message_count = counts.message_count \
             FROM ( \
                SELECT ids.conversation_id, COUNT(m.id)::integer AS message_count \
                FROM (VALUES {value_rows}) AS ids(conversation_id) \
                LEFT JOIN messages AS m ON m.conversation_id = ids.conversation_id \
                GROUP BY ids.conversation_id \
             ) AS counts \
             WHERE c.id = counts.conversation_id"
        );
        conn.execute(Statement::from_sql_and_values(
            DbBackend::Postgres,
            update_sql,
            values.clone(),
        ))
        .await?;

        let placeholders = conversation_ids
            .iter()
            .enumerate()
            .map(|(idx, _)| format!("${}", idx + 1))
            .collect::<Vec<_>>()
            .join(", ");
        let mut delete_values = values;
        delete_values.push(message_cutoff.to_string().into());
        let delete_sql = format!(
            "DELETE FROM conversations \
             WHERE id IN ({placeholders}) \
               AND updated_at < ${} \
               AND message_count = 0",
            conversation_ids.len() + 1
        );
        conn.execute(Statement::from_sql_and_values(
            DbBackend::Postgres,
            delete_sql,
            delete_values,
        ))
        .await?;
        Ok(())
    }

    async fn purge_message_batches(&self, message_cutoff: &str) -> Result<()> {
        loop {
            let txn = self.db.begin().await?;
            let deleted_rows = txn
                .query_all(Statement::from_sql_and_values(
                    DbBackend::Postgres,
                    "DELETE FROM messages \
                     WHERE id IN ( \
                        SELECT id \
                        FROM messages \
                        WHERE timestamp < $1 \
                        ORDER BY timestamp ASC \
                        LIMIT $2 \
                     ) \
                     RETURNING conversation_id",
                    vec![
                        message_cutoff.to_string().into(),
                        Self::HOUSEKEEPING_PURGE_BATCH_SIZE.into(),
                    ],
                ))
                .await?;
            if deleted_rows.is_empty() {
                txn.commit().await?;
                break;
            }
            let conversation_ids = deleted_rows
                .into_iter()
                .filter_map(|row| row.try_get::<String>("", "conversation_id").ok())
                .filter(|value| !value.trim().is_empty())
                .collect::<std::collections::BTreeSet<_>>()
                .into_iter()
                .collect::<Vec<_>>();
            Self::recount_conversations_after_message_batch(
                &txn,
                &conversation_ids,
                message_cutoff,
            )
            .await?;
            txn.commit().await?;
        }
        Ok(())
    }

    async fn purge_execution_run_batches(&self, execution_run_cutoff: &str) -> Result<()> {
        loop {
            let txn = self.db.begin().await?;
            let rows = txn
                .query_all(Statement::from_sql_and_values(
                    DbBackend::Postgres,
                    "SELECT id \
                     FROM execution_runs \
                     WHERE created_at < $1 \
                     ORDER BY created_at ASC \
                     LIMIT $2",
                    vec![
                        execution_run_cutoff.to_string().into(),
                        Self::HOUSEKEEPING_PURGE_BATCH_SIZE.into(),
                    ],
                ))
                .await?;
            let run_ids = rows
                .into_iter()
                .filter_map(|row| row.try_get::<String>("", "id").ok())
                .filter(|value| !value.trim().is_empty())
                .collect::<Vec<_>>();
            if run_ids.is_empty() {
                txn.commit().await?;
                break;
            }
            Self::delete_rows_by_ids(&txn, "run_checkpoints", "run_id", &run_ids).await?;
            Self::delete_rows_by_ids(&txn, "tool_attempts", "run_id", &run_ids).await?;
            Self::delete_rows_by_ids(&txn, "execution_runs", "id", &run_ids).await?;
            txn.commit().await?;
        }
        Ok(())
    }

    async fn purge_memory_operation_batches(&self, memory_operation_cutoff: &str) -> Result<()> {
        loop {
            let txn = self.db.begin().await?;
            let rows = txn
                .query_all(Statement::from_sql_and_values(
                    DbBackend::Postgres,
                    "SELECT id \
                     FROM memory_operations \
                     WHERE updated_at < $1 \
                       AND (applied_at IS NOT NULL OR reviewed_at IS NOT NULL) \
                     ORDER BY updated_at ASC \
                     LIMIT $2",
                    vec![
                        memory_operation_cutoff.to_string().into(),
                        Self::HOUSEKEEPING_PURGE_BATCH_SIZE.into(),
                    ],
                ))
                .await?;
            let operation_ids = rows
                .into_iter()
                .filter_map(|row| row.try_get::<String>("", "id").ok())
                .filter(|value| !value.trim().is_empty())
                .collect::<Vec<_>>();
            if operation_ids.is_empty() {
                txn.commit().await?;
                break;
            }
            Self::delete_rows_by_ids(
                &txn,
                "memory_evidence_links",
                "operation_id",
                &operation_ids,
            )
            .await?;
            Self::delete_rows_by_ids(&txn, "memory_operations", "id", &operation_ids).await?;
            txn.commit().await?;
        }
        Ok(())
    }

    async fn maybe_purge_housekeeping_tables(&self) -> Result<()> {
        let now = chrono::Utc::now();
        let lifecycle = crate::core::data_lifecycle::load_data_lifecycle_settings(self).await;
        if !lifecycle.cleanup_enabled || !lifecycle.logs_cleanup_enabled {
            return Ok(());
        }
        if let Some(bytes) = self.get(Self::HOUSEKEEPING_PURGE_LAST_RUN_KEY).await? {
            if let Ok(raw) = String::from_utf8(bytes) {
                if let Ok(last) = chrono::DateTime::parse_from_rfc3339(&raw) {
                    if (now - last.with_timezone(&chrono::Utc)).num_seconds()
                        < lifecycle.housekeeping_interval_secs as i64
                    {
                        return Ok(());
                    }
                }
            }
        }

        let all_retention_disabled = lifecycle.execution_trace_retention_days == 0
            && lifecycle.execution_proof_retention_days == 0
            && lifecycle.operational_log_retention_days == 0
            && lifecycle.security_log_retention_days == 0
            && lifecycle.approval_log_retention_days == 0
            && lifecycle.swarm_delegation_retention_days == 0
            && lifecycle.llm_usage_retention_days == 0
            && lifecycle.terminal_task_retention_days == 0
            && lifecycle.execution_run_retention_days == 0
            && lifecycle.background_session_retention_days == 0
            && lifecycle.browser_session_retention_days == 0
            && lifecycle.automation_run_retention_days == 0
            && lifecycle.message_retention_days == 0
            && lifecycle.experience_run_retention_days == 0
            && lifecycle.experience_edge_retention_days == 0
            && lifecycle.learning_candidate_retention_days == 0
            && lifecycle.experience_item_retention_days == 0
            && lifecycle.procedural_pattern_retention_days == 0
            && lifecycle.recall_event_retention_days == 0
            && lifecycle.recall_test_retention_days == 0
            && lifecycle.readiness_evaluation_retention_days == 0
            && lifecycle.memory_capture_event_retention_days == 0
            && lifecycle.memory_operation_retention_days == 0
            && lifecycle.memory_evidence_link_retention_days == 0
            && lifecycle.semantic_work_unit_retention_days == 0;

        if all_retention_disabled {
            self.set(
                Self::HOUSEKEEPING_PURGE_LAST_RUN_KEY,
                now.to_rfc3339().as_bytes(),
            )
            .await?;
            return Ok(());
        }

        if lifecycle.message_retention_days > 0 {
            let message_cutoff = (now
                - chrono::Duration::days(lifecycle.message_retention_days as i64))
            .to_rfc3339();
            self.purge_message_batches(&message_cutoff).await?;
        }

        if lifecycle.execution_trace_retention_days > 0 {
            let trace_cutoff = (now
                - chrono::Duration::days(lifecycle.execution_trace_retention_days as i64))
            .to_rfc3339();
            self.delete_by_cutoff_in_batches(
                "execution_traces",
                "id",
                "created_at",
                &trace_cutoff,
                "",
            )
            .await?;
        }
        if lifecycle.execution_proof_retention_days > 0 {
            let proof_cutoff = (now
                - chrono::Duration::days(lifecycle.execution_proof_retention_days as i64))
            .to_rfc3339();
            self.delete_by_cutoff_in_batches(
                "execution_proofs",
                "id",
                "timestamp",
                &proof_cutoff,
                "",
            )
            .await?;
        }
        if lifecycle.operational_log_retention_days > 0 {
            let operational_cutoff = (now
                - chrono::Duration::days(lifecycle.operational_log_retention_days as i64))
            .to_rfc3339();
            self.delete_by_cutoff_in_batches(
                "operational_logs",
                "id",
                "created_at",
                &operational_cutoff,
                "",
            )
            .await?;
        }
        if lifecycle.security_log_retention_days > 0 {
            let security_cutoff = (now
                - chrono::Duration::days(lifecycle.security_log_retention_days as i64))
            .to_rfc3339();
            self.delete_by_cutoff_in_batches(
                "security_logs",
                "id",
                "created_at",
                &security_cutoff,
                "",
            )
            .await?;
        }
        if lifecycle.approval_log_retention_days > 0 {
            let approval_cutoff = (now
                - chrono::Duration::days(lifecycle.approval_log_retention_days as i64))
            .to_rfc3339();
            self.delete_by_cutoff_in_batches(
                "approval_log",
                "id",
                "requested_at",
                &approval_cutoff,
                "AND status <> 'pending'",
            )
            .await?;
        }
        if lifecycle.swarm_delegation_retention_days > 0 {
            let delegation_cutoff = (now
                - chrono::Duration::days(lifecycle.swarm_delegation_retention_days as i64))
            .to_rfc3339();
            self.delete_by_cutoff_in_batches(
                "swarm_delegations",
                "id",
                "created_at",
                &delegation_cutoff,
                "",
            )
            .await?;
        }
        if lifecycle.llm_usage_retention_days > 0 {
            let llm_usage_cutoff = (now
                - chrono::Duration::days(lifecycle.llm_usage_retention_days as i64))
            .to_rfc3339();
            self.delete_by_cutoff_in_batches(
                "llm_usage",
                "id",
                "created_at",
                &llm_usage_cutoff,
                "",
            )
            .await?;
        }
        if lifecycle.execution_run_retention_days > 0 {
            let execution_run_cutoff = (now
                - chrono::Duration::days(lifecycle.execution_run_retention_days as i64))
            .to_rfc3339();
            self.purge_execution_run_batches(&execution_run_cutoff)
                .await?;
        }
        if lifecycle.experience_run_retention_days > 0 {
            let experience_run_cutoff = (now
                - chrono::Duration::days(lifecycle.experience_run_retention_days as i64))
            .to_rfc3339();
            self.delete_by_cutoff_in_batches(
                "experience_runs",
                "id",
                "created_at",
                &experience_run_cutoff,
                "",
            )
            .await?;
        }
        if lifecycle.experience_edge_retention_days > 0 {
            let experience_edge_cutoff = (now
                - chrono::Duration::days(lifecycle.experience_edge_retention_days as i64))
            .to_rfc3339();
            self.delete_by_cutoff_in_batches(
                "experience_edges",
                "id",
                "created_at",
                &experience_edge_cutoff,
                "",
            )
            .await?;
        }
        if lifecycle.learning_candidate_retention_days > 0 {
            let learning_candidate_cutoff = (now
                - chrono::Duration::days(lifecycle.learning_candidate_retention_days as i64))
            .to_rfc3339();
            self.delete_by_cutoff_in_batches(
                "learning_candidates",
                "id",
                "created_at",
                &learning_candidate_cutoff,
                "",
            )
            .await?;
        }
        if lifecycle.experience_item_retention_days > 0 {
            let experience_item_cutoff = (now
                - chrono::Duration::days(lifecycle.experience_item_retention_days as i64))
            .to_rfc3339();
            self.delete_by_cutoff_in_batches(
                "experience_items",
                "id",
                "updated_at",
                &experience_item_cutoff,
                "AND status <> 'active'",
            )
            .await?;
        }
        if lifecycle.procedural_pattern_retention_days > 0 {
            let procedural_pattern_cutoff = (now
                - chrono::Duration::days(lifecycle.procedural_pattern_retention_days as i64))
            .to_rfc3339();
            self.delete_by_cutoff_in_batches(
                "procedural_patterns",
                "id",
                "updated_at",
                &procedural_pattern_cutoff,
                "AND status NOT IN ('active', 'draft')",
            )
            .await?;
        }
        if lifecycle.recall_event_retention_days > 0 {
            let recall_event_cutoff = (now
                - chrono::Duration::days(lifecycle.recall_event_retention_days as i64))
            .to_rfc3339();
            self.delete_by_cutoff_in_batches(
                "recall_events",
                "id",
                "created_at",
                &recall_event_cutoff,
                "",
            )
            .await?;
        }
        if lifecycle.recall_test_retention_days > 0 {
            let recall_test_cutoff = (now
                - chrono::Duration::days(lifecycle.recall_test_retention_days as i64))
            .to_rfc3339();
            self.delete_by_cutoff_in_batches(
                "recall_tests",
                "id",
                "updated_at",
                &recall_test_cutoff,
                "AND status IN ('retired', 'pending', 'passed', 'failed')",
            )
            .await?;
        }
        if lifecycle.readiness_evaluation_retention_days > 0 {
            let readiness_cutoff = (now
                - chrono::Duration::days(lifecycle.readiness_evaluation_retention_days as i64))
            .to_rfc3339();
            self.delete_by_cutoff_in_batches(
                "readiness_evaluations",
                "id",
                "created_at",
                &readiness_cutoff,
                "",
            )
            .await?;
        }
        if lifecycle.memory_capture_event_retention_days > 0 {
            let memory_capture_cutoff = (now
                - chrono::Duration::days(lifecycle.memory_capture_event_retention_days as i64))
            .to_rfc3339();
            self.delete_by_cutoff_in_batches(
                "memory_capture_events",
                "id",
                "completed_at",
                &memory_capture_cutoff,
                "",
            )
            .await?;
        }
        if lifecycle.memory_operation_retention_days > 0 {
            let memory_operation_cutoff = (now
                - chrono::Duration::days(lifecycle.memory_operation_retention_days as i64))
            .to_rfc3339();
            self.purge_memory_operation_batches(&memory_operation_cutoff)
                .await?;
        }
        if lifecycle.memory_evidence_link_retention_days > 0 {
            let memory_evidence_cutoff = (now
                - chrono::Duration::days(lifecycle.memory_evidence_link_retention_days as i64))
            .to_rfc3339();
            self.delete_by_cutoff_in_batches(
                "memory_evidence_links",
                "id",
                "created_at",
                &memory_evidence_cutoff,
                "",
            )
            .await?;
        }
        if lifecycle.semantic_work_unit_retention_days > 0 {
            let semantic_work_unit_cutoff = (now
                - chrono::Duration::days(lifecycle.semantic_work_unit_retention_days as i64))
            .to_rfc3339();
            self.delete_by_cutoff_in_batches(
                "semantic_work_units",
                "id",
                "occurred_at",
                &semantic_work_unit_cutoff,
                "",
            )
            .await?;
        }
        if lifecycle.background_session_retention_days > 0 {
            let background_session_cutoff = (now
                - chrono::Duration::days(lifecycle.background_session_retention_days as i64))
            .to_rfc3339();
            self.delete_by_cutoff_in_batches(
                "background_sessions",
                "id",
                "last_activity_at",
                &background_session_cutoff,
                "AND status IN ('completed', 'failed', 'cancelled')",
            )
            .await?;
        }
        if lifecycle.browser_session_retention_days > 0 {
            let browser_session_cutoff = (now
                - chrono::Duration::days(lifecycle.browser_session_retention_days as i64))
            .to_rfc3339();
            self.delete_by_cutoff_in_batches(
                "browser_sessions",
                "id",
                "updated_at",
                &browser_session_cutoff,
                "AND status IN ('completed', 'failed', 'interrupted')",
            )
            .await?;
        }
        if lifecycle.automation_run_retention_days > 0 {
            let automation_run_cutoff = (now
                - chrono::Duration::days(lifecycle.automation_run_retention_days as i64))
            .to_rfc3339();
            self.delete_by_cutoff_in_batches(
                "automation_runs",
                "id",
                "started_at",
                &automation_run_cutoff,
                "",
            )
            .await?;
        }

        if lifecycle.terminal_task_retention_days > 0 {
            let terminal_task_cutoff = (now
                - chrono::Duration::days(lifecycle.terminal_task_retention_days as i64))
            .to_rfc3339();
            let stale_tasks = task::Entity::find()
                .filter(task::Column::CreatedAt.lt(terminal_task_cutoff))
                .all(&self.db)
                .await?;
            for stale_task in stale_tasks {
                if stale_task.cron.is_some() {
                    continue;
                }
                let status = serde_json::from_str::<crate::core::TaskStatus>(&stale_task.status)
                    .unwrap_or(crate::core::TaskStatus::Pending);
                let terminal = matches!(
                    status,
                    crate::core::TaskStatus::Completed
                        | crate::core::TaskStatus::Cancelled
                        | crate::core::TaskStatus::Failed { .. }
                );
                if !terminal {
                    continue;
                }
                task::Entity::delete_by_id(stale_task.id)
                    .exec(&self.db)
                    .await?;
            }
        }

        self.set(
            Self::HOUSEKEEPING_PURGE_LAST_RUN_KEY,
            now.to_rfc3339().as_bytes(),
        )
        .await?;
        Ok(())
    }

    pub async fn run_housekeeping_purge(&self) -> Result<()> {
        self.maybe_purge_housekeeping_tables().await
    }
}
