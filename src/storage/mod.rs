//! Database storage using SeaORM backed by PostgreSQL.
pub mod encrypted;
pub mod entities;
mod migrations;
mod repositories;

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
    crate::core::knowledge::memory_schema::normalize_memory_topics(metadata.get("topics"), 8)
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
    crate::core::knowledge::memory_schema::memory_category_from_metadata(
        metadata,
        semantic_kind.as_deref(),
    )
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
    let memory_category = crate::core::knowledge::memory_schema::memory_category_from_metadata(
        &item.metadata,
        memory_kind.as_deref(),
    )
    .to_string();
    let topics = learned_fact_topics_from_metadata(&item.metadata);
    let key = learned_fact_key_from_metadata(&item.metadata);
    let value = learned_fact_value_from_content(key.as_deref(), &item.content);
    let (key, value) = match key {
        Some(raw_key) => {
            let allow_value_suffix_repair = memory_category
                == crate::core::knowledge::memory_schema::MEMORY_CATEGORY_PROFILE_FACT;
            match crate::core::knowledge::memory_schema::repair_memory_slot_key_and_value(
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
