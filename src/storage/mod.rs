//! Database storage using SeaORM

pub mod encrypted;
pub mod entities;

use crate::crypto::KeyManager;
use anyhow::Result;
use sea_orm::sea_query::Expr;
#[allow(unused_imports)]
use sea_orm::{
    ActiveModelTrait, ColumnTrait, Condition, ConnectionTrait, Database, DatabaseConnection,
    DbBackend, EntityTrait, PaginatorTrait, QueryFilter, QueryOrder, QuerySelect, Schema, Set,
    Statement, TransactionTrait, TryGetable, Unchanged,
};
use std::path::Path;
use std::sync::{Arc, OnceLock, RwLock};

pub use entities::*;

/// Database storage using SeaORM
#[derive(Clone)]
pub struct Storage {
    db: DatabaseConnection,
}

static STORAGE_KEY_MANAGER: OnceLock<RwLock<Option<Arc<KeyManager>>>> = OnceLock::new();

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

fn decrypt_storage_string(value: &str) -> String {
    if value.is_empty() {
        return String::new();
    }
    if let Some(key_manager) = current_storage_key_manager() {
        key_manager
            .decrypt_string(value)
            .unwrap_or_else(|_| value.to_string())
    } else {
        value.to_string()
    }
}

fn encrypt_optional_storage_string(value: Option<&str>) -> Result<Option<String>> {
    value.map(encrypt_storage_string).transpose()
}

fn decrypt_optional_storage_string(value: Option<String>) -> Option<String> {
    value.map(|inner| decrypt_storage_string(&inner))
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

impl Storage {
    const EXECUTION_TRACE_RETENTION_DAYS: i64 = 30;
    const EXECUTION_PROOF_RETENTION_DAYS: i64 = 30;
    const OPERATIONAL_LOG_RETENTION_DAYS: i64 = 30;
    const SECURITY_LOG_RETENTION_DAYS: i64 = 30;
    const APPROVAL_LOG_RETENTION_DAYS: i64 = 30;
    const SWARM_DELEGATION_RETENTION_DAYS: i64 = 30;
    const LLM_USAGE_RETENTION_DAYS: i64 = 30;
    const TERMINAL_TASK_RETENTION_DAYS: i64 = 90;
    const MESSAGE_RETENTION_DAYS: i64 = 365;
    const HOUSEKEEPING_PURGE_MIN_INTERVAL_SECS: i64 = 3600;
    const HOUSEKEEPING_PURGE_LAST_RUN_KEY: &'static str = "storage_housekeeping_last_purge_v1";
    const MAX_EPISODES_FOR_SCORING: u64 = 10_000;
    const MAX_DOCUMENT_CHUNKS_FOR_SEARCH: u64 = 20_000;
    const SENSITIVE_PAYLOAD_BACKFILL_MARKER_KEY: &'static str =
        "storage_sensitive_payload_backfill_v4";

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

    /// Create a new storage instance
    pub async fn new(data_dir: &Path) -> Result<Self> {
        let db_path = data_dir.join("agentark.db");
        let db_url = format!("sqlite:{}?mode=rwc", db_path.display());

        let db = Database::connect(&db_url).await?;
        Self::configure_sqlite(&db).await?;

        // Create tables if they don't exist
        Self::create_tables(&db).await?;
        Self::validate_sqlite_schema(&db).await?;

        Ok(Self { db })
    }

    async fn configure_sqlite(db: &DatabaseConnection) -> Result<()> {
        if db.get_database_backend() != DbBackend::Sqlite {
            return Ok(());
        }

        db.execute_unprepared(
            "\
PRAGMA journal_mode = WAL;\n\
PRAGMA synchronous = NORMAL;\n\
PRAGMA foreign_keys = ON;\n\
PRAGMA busy_timeout = 5000;\n",
        )
        .await?;

        Ok(())
    }

    /// Create all tables
    async fn create_tables(db: &DatabaseConnection) -> Result<()> {
        let backend = db.get_database_backend();
        let _schema = Schema::new(backend);

        // Create tables using raw SQL for SQLite compatibility
        db.execute_unprepared(
            r#"
            CREATE TABLE IF NOT EXISTS kv_store (
                key TEXT PRIMARY KEY,
                value BLOB NOT NULL,
                created_at TEXT NOT NULL,
                updated_at TEXT NOT NULL
            );

            CREATE TABLE IF NOT EXISTS episodes (
                id TEXT PRIMARY KEY,
                content TEXT NOT NULL,
                context TEXT NOT NULL,
                embedding BLOB,
                timestamp TEXT NOT NULL,
                consolidated INTEGER DEFAULT 0,
                importance REAL DEFAULT 0.5,
                last_accessed TEXT,
                access_count INTEGER DEFAULT 0,
                project_id TEXT
            );

            CREATE TABLE IF NOT EXISTS semantic_facts (
                id TEXT PRIMARY KEY,
                fact TEXT NOT NULL,
                confidence REAL NOT NULL,
                sources TEXT NOT NULL,
                embedding BLOB,
                created_at TEXT NOT NULL,
                project_id TEXT
            );

            CREATE TABLE IF NOT EXISTS actions (
                name TEXT PRIMARY KEY,
                version TEXT NOT NULL,
                wasm_hash TEXT,
                source TEXT NOT NULL,
                success_rate REAL DEFAULT 1.0,
                execution_count INTEGER DEFAULT 0,
                last_used TEXT
            );

            CREATE TABLE IF NOT EXISTS execution_proofs (
                id TEXT PRIMARY KEY,
                action_hash TEXT NOT NULL,
                input_hash TEXT NOT NULL,
                output_hash TEXT NOT NULL,
                prev_hash TEXT,
                timestamp TEXT NOT NULL,
                signature TEXT NOT NULL
            );

            CREATE TABLE IF NOT EXISTS execution_traces (
                id TEXT PRIMARY KEY,
                message TEXT NOT NULL,
                channel TEXT NOT NULL,
                started_at TEXT,
                completed_at TEXT,
                duration_ms INTEGER,
                step_count INTEGER NOT NULL DEFAULT 0,
                steps_json TEXT NOT NULL,
                response TEXT,
                proof_id TEXT REFERENCES execution_proofs(id) ON DELETE SET NULL,
                model TEXT,
                input_tokens INTEGER NOT NULL DEFAULT 0,
                output_tokens INTEGER NOT NULL DEFAULT 0,
                total_tokens INTEGER NOT NULL DEFAULT 0,
                cost_usd REAL NOT NULL DEFAULT 0.0,
                complexity TEXT,
                created_at TEXT NOT NULL,
                CHECK(step_count >= 0),
                CHECK(input_tokens >= 0),
                CHECK(output_tokens >= 0),
                CHECK(total_tokens >= 0),
                CHECK(cost_usd >= 0.0)
            );

            CREATE TABLE IF NOT EXISTS tasks (
                id TEXT PRIMARY KEY,
                description TEXT NOT NULL,
                action TEXT NOT NULL,
                arguments TEXT NOT NULL,
                approval TEXT NOT NULL,
                status TEXT NOT NULL,
                created_at TEXT NOT NULL,
                scheduled_for TEXT,
                cron TEXT,
                result TEXT,
                proof_id TEXT REFERENCES execution_proofs(id) ON DELETE SET NULL,
                priority REAL,
                urgency REAL,
                importance REAL,
                eisenhower_quadrant INTEGER,
                CHECK(length(trim(action)) > 0),
                CHECK(eisenhower_quadrant IS NULL OR eisenhower_quadrant BETWEEN 1 AND 4)
            );

            CREATE TABLE IF NOT EXISTS swarm_agents (
                id TEXT PRIMARY KEY,
                name TEXT NOT NULL,
                agent_type TEXT NOT NULL,
                llm_provider TEXT NOT NULL,
                capabilities TEXT NOT NULL,
                system_prompt TEXT,
                enabled INTEGER DEFAULT 1,
                created_at TEXT NOT NULL,
                CHECK(enabled IN (0, 1))
            );

            CREATE TABLE IF NOT EXISTS swarm_delegations (
                id TEXT PRIMARY KEY,
                parent_task_id TEXT REFERENCES tasks(id) ON DELETE SET NULL,
                agent_id TEXT NOT NULL REFERENCES swarm_agents(id) ON DELETE CASCADE,
                task_description TEXT NOT NULL,
                result TEXT,
                success INTEGER DEFAULT 0,
                confidence REAL,
                execution_time_ms INTEGER,
                created_at TEXT NOT NULL,
                completed_at TEXT,
                CHECK(success IN (0, 1))
            );

            CREATE TABLE IF NOT EXISTS conversations (
                id TEXT PRIMARY KEY,
                title TEXT NOT NULL,
                channel TEXT NOT NULL,
                project_id TEXT REFERENCES projects(id) ON DELETE CASCADE,
                created_at TEXT NOT NULL,
                updated_at TEXT NOT NULL,
                message_count INTEGER DEFAULT 0,
                archived INTEGER DEFAULT 0,
                CHECK(message_count >= 0),
                CHECK(archived IN (0, 1))
            );

            CREATE TABLE IF NOT EXISTS messages (
                id TEXT PRIMARY KEY,
                conversation_id TEXT NOT NULL REFERENCES conversations(id) ON DELETE CASCADE,
                role TEXT NOT NULL,
                content TEXT NOT NULL,
                timestamp TEXT NOT NULL,
                model_used TEXT,
                trace_id TEXT REFERENCES execution_traces(id) ON DELETE SET NULL
            );

            CREATE TABLE IF NOT EXISTS projects (
                id TEXT PRIMARY KEY,
                name TEXT NOT NULL,
                description TEXT NOT NULL DEFAULT '',
                system_prompt TEXT,
                personality TEXT,
                tools_filter TEXT,
                active INTEGER DEFAULT 1,
                created_at TEXT NOT NULL,
                updated_at TEXT NOT NULL,
                CHECK(active IN (0, 1))
            );

            CREATE TABLE IF NOT EXISTS documents (
                id TEXT PRIMARY KEY,
                filename TEXT NOT NULL,
                content_type TEXT NOT NULL,
                project_id TEXT REFERENCES projects(id) ON DELETE CASCADE,
                chunk_count INTEGER DEFAULT 0,
                file_size INTEGER DEFAULT 0,
                created_at TEXT NOT NULL,
                CHECK(chunk_count >= 0),
                CHECK(file_size >= 0)
            );

            CREATE TABLE IF NOT EXISTS document_chunks (
                id TEXT PRIMARY KEY,
                document_id TEXT NOT NULL REFERENCES documents(id) ON DELETE CASCADE,
                chunk_index INTEGER NOT NULL,
                content TEXT NOT NULL,
                embedding BLOB,
                CHECK(chunk_index >= 0)
            );

            CREATE TABLE IF NOT EXISTS notifications (
                id TEXT PRIMARY KEY,
                title TEXT NOT NULL,
                body TEXT NOT NULL,
                level TEXT NOT NULL DEFAULT 'info',
                source TEXT NOT NULL DEFAULT '',
                read INTEGER DEFAULT 0,
                created_at TEXT NOT NULL,
                CHECK(read IN (0, 1))
            );

            CREATE TABLE IF NOT EXISTS approval_log (
                id TEXT PRIMARY KEY,
                action_name TEXT NOT NULL,
                arguments TEXT NOT NULL,
                rule_name TEXT NOT NULL,
                status TEXT NOT NULL DEFAULT 'pending',
                requested_at TEXT NOT NULL,
                resolved_at TEXT,
                resolved_by TEXT,
                CHECK(status IN ('pending', 'approved', 'denied', 'expired'))
            );

            CREATE TABLE IF NOT EXISTS automation_runs (
                id TEXT PRIMARY KEY,
                automation_id TEXT NOT NULL,
                started_at TEXT NOT NULL,
                payload TEXT NOT NULL
            );

            CREATE TABLE IF NOT EXISTS automation_supervisor_states (
                automation_id TEXT PRIMARY KEY,
                updated_at TEXT NOT NULL,
                payload TEXT NOT NULL
            );

            CREATE TABLE IF NOT EXISTS watchers (
                id TEXT PRIMARY KEY,
                status TEXT NOT NULL,
                created_at TEXT NOT NULL,
                payload TEXT NOT NULL
            );

            CREATE INDEX IF NOT EXISTS idx_episodes_timestamp ON episodes(timestamp);
            CREATE INDEX IF NOT EXISTS idx_proofs_timestamp ON execution_proofs(timestamp);
            CREATE INDEX IF NOT EXISTS idx_execution_traces_created ON execution_traces(created_at);
            CREATE INDEX IF NOT EXISTS idx_execution_traces_started ON execution_traces(started_at);
            CREATE INDEX IF NOT EXISTS idx_tasks_status ON tasks(status);
            CREATE INDEX IF NOT EXISTS idx_tasks_scheduled_for ON tasks(scheduled_for);
            CREATE INDEX IF NOT EXISTS idx_tasks_status_scheduled ON tasks(status, scheduled_for);
            CREATE INDEX IF NOT EXISTS idx_tasks_created_at ON tasks(created_at);
            CREATE INDEX IF NOT EXISTS idx_swarm_delegations_agent ON swarm_delegations(agent_id);
            CREATE INDEX IF NOT EXISTS idx_messages_conversation ON messages(conversation_id);
            CREATE INDEX IF NOT EXISTS idx_messages_timestamp ON messages(timestamp);
            CREATE INDEX IF NOT EXISTS idx_messages_role_timestamp ON messages(role, timestamp);
            CREATE INDEX IF NOT EXISTS idx_conversations_updated ON conversations(updated_at);
            CREATE INDEX IF NOT EXISTS idx_conversations_project ON conversations(project_id);
            CREATE INDEX IF NOT EXISTS idx_documents_project ON documents(project_id);
            CREATE INDEX IF NOT EXISTS idx_document_chunks_doc ON document_chunks(document_id);
            CREATE UNIQUE INDEX IF NOT EXISTS idx_document_chunks_doc_chunk ON document_chunks(document_id, chunk_index);
            CREATE INDEX IF NOT EXISTS idx_notifications_created ON notifications(created_at);
            CREATE INDEX IF NOT EXISTS idx_approval_log_status ON approval_log(status);
            CREATE INDEX IF NOT EXISTS idx_approval_log_requested ON approval_log(requested_at);
            CREATE INDEX IF NOT EXISTS idx_automation_runs_started ON automation_runs(started_at);
            CREATE INDEX IF NOT EXISTS idx_automation_runs_automation_id ON automation_runs(automation_id);
            CREATE INDEX IF NOT EXISTS idx_watchers_status ON watchers(status);
            CREATE INDEX IF NOT EXISTS idx_watchers_created ON watchers(created_at);
            CREATE TABLE IF NOT EXISTS expenses (
                id TEXT PRIMARY KEY,
                amount REAL NOT NULL,
                currency TEXT NOT NULL DEFAULT 'USD',
                category TEXT NOT NULL,
                description TEXT NOT NULL,
                date TEXT NOT NULL,
                payment_method TEXT,
                vendor TEXT,
                tags TEXT,
                split_with TEXT,
                receipt_path TEXT,
                created_at TEXT NOT NULL
            );

            CREATE TABLE IF NOT EXISTS security_logs (
                id TEXT PRIMARY KEY,
                event_type TEXT NOT NULL,
                severity TEXT NOT NULL,
                message TEXT NOT NULL,
                source TEXT,
                count INTEGER NOT NULL DEFAULT 1,
                created_at TEXT NOT NULL
            );

            CREATE TABLE IF NOT EXISTS operational_logs (
                id TEXT PRIMARY KEY,
                created_at TEXT NOT NULL,
                trace_id TEXT REFERENCES execution_traces(id) ON DELETE SET NULL,
                conversation_id TEXT REFERENCES conversations(id) ON DELETE SET NULL,
                channel TEXT NOT NULL DEFAULT '',
                event_type TEXT NOT NULL,
                success INTEGER NOT NULL DEFAULT 0,
                outcome TEXT NOT NULL DEFAULT '',
                tool_name TEXT,
                latency_ms INTEGER,
                arguments TEXT,
                payload TEXT,
                strategy_version TEXT,
                policy_version TEXT,
                prompt_version TEXT,
                model_slot TEXT,
                CHECK(success IN (0, 1))
            );

            CREATE TABLE IF NOT EXISTS llm_usage (
                id TEXT PRIMARY KEY,
                created_at TEXT NOT NULL,
                provider TEXT NOT NULL,
                model TEXT NOT NULL,
                channel TEXT NOT NULL,
                purpose TEXT NOT NULL DEFAULT '',
                prompt_tokens INTEGER NOT NULL,
                completion_tokens INTEGER NOT NULL,
                total_tokens INTEGER NOT NULL,
                estimated INTEGER NOT NULL DEFAULT 1,
                CHECK(prompt_tokens >= 0),
                CHECK(completion_tokens >= 0),
                CHECK(total_tokens >= 0),
                CHECK(estimated IN (0, 1))
            );

            CREATE TABLE IF NOT EXISTS user_preferences (
                id TEXT PRIMARY KEY,
                key TEXT NOT NULL,
                value TEXT NOT NULL,
                confidence REAL NOT NULL DEFAULT 0.8,
                source TEXT,
                project_id TEXT REFERENCES projects(id) ON DELETE CASCADE,
                created_at TEXT NOT NULL,
                updated_at TEXT NOT NULL,
                CHECK(confidence >= 0.0 AND confidence <= 1.0)
            );

            CREATE TABLE IF NOT EXISTS user_data_items (
                id TEXT PRIMARY KEY,
                kind TEXT NOT NULL,
                title TEXT NOT NULL,
                content TEXT NOT NULL,
                url TEXT,
                source_channel TEXT,
                conversation_id TEXT REFERENCES conversations(id) ON DELETE SET NULL,
                project_id TEXT REFERENCES projects(id) ON DELETE CASCADE,
                pinned INTEGER NOT NULL DEFAULT 0,
                created_at TEXT NOT NULL,
                updated_at TEXT NOT NULL,
                CHECK(pinned IN (0, 1))
            );

            CREATE TABLE IF NOT EXISTS knowledge_items (
                id TEXT PRIMARY KEY,
                title TEXT NOT NULL,
                content TEXT NOT NULL,
                source TEXT,
                url TEXT,
                tags TEXT,
                project_id TEXT REFERENCES projects(id) ON DELETE CASCADE,
                created_at TEXT NOT NULL,
                updated_at TEXT NOT NULL
            );

            CREATE INDEX IF NOT EXISTS idx_episodes_timestamp ON episodes(timestamp);
            CREATE INDEX IF NOT EXISTS idx_proofs_timestamp ON execution_proofs(timestamp);
            CREATE INDEX IF NOT EXISTS idx_swarm_delegations_agent ON swarm_delegations(agent_id);
            CREATE INDEX IF NOT EXISTS idx_messages_conversation ON messages(conversation_id);
            CREATE INDEX IF NOT EXISTS idx_messages_timestamp ON messages(timestamp);
            CREATE INDEX IF NOT EXISTS idx_conversations_updated ON conversations(updated_at);
            CREATE INDEX IF NOT EXISTS idx_conversations_project ON conversations(project_id);
            CREATE INDEX IF NOT EXISTS idx_documents_project ON documents(project_id);
            CREATE INDEX IF NOT EXISTS idx_document_chunks_doc ON document_chunks(document_id);
            CREATE INDEX IF NOT EXISTS idx_notifications_created ON notifications(created_at);
            CREATE INDEX IF NOT EXISTS idx_approval_log_status ON approval_log(status);
            CREATE INDEX IF NOT EXISTS idx_approval_log_requested ON approval_log(requested_at);
            CREATE INDEX IF NOT EXISTS idx_episodes_project_id ON episodes(project_id);
            CREATE INDEX IF NOT EXISTS idx_facts_project_id ON semantic_facts(project_id);
            CREATE INDEX IF NOT EXISTS idx_security_logs_created ON security_logs(created_at);
            CREATE INDEX IF NOT EXISTS idx_security_logs_type ON security_logs(event_type);
            CREATE INDEX IF NOT EXISTS idx_operational_logs_created ON operational_logs(created_at);
            CREATE INDEX IF NOT EXISTS idx_operational_logs_event_type ON operational_logs(event_type);
            CREATE INDEX IF NOT EXISTS idx_operational_logs_tool_name ON operational_logs(tool_name);
            CREATE INDEX IF NOT EXISTS idx_operational_logs_success ON operational_logs(success);
            CREATE INDEX IF NOT EXISTS idx_operational_logs_policy_version ON operational_logs(policy_version);
            CREATE INDEX IF NOT EXISTS idx_operational_logs_strategy_version ON operational_logs(strategy_version);
            CREATE INDEX IF NOT EXISTS idx_llm_usage_created ON llm_usage(created_at);
            CREATE INDEX IF NOT EXISTS idx_llm_usage_model ON llm_usage(model);
            CREATE INDEX IF NOT EXISTS idx_llm_usage_provider ON llm_usage(provider);
            CREATE INDEX IF NOT EXISTS idx_llm_usage_channel ON llm_usage(channel);
            CREATE INDEX IF NOT EXISTS idx_user_preferences_key ON user_preferences(key);
            CREATE INDEX IF NOT EXISTS idx_user_preferences_project ON user_preferences(project_id);
            CREATE INDEX IF NOT EXISTS idx_user_data_kind ON user_data_items(kind);
            CREATE INDEX IF NOT EXISTS idx_user_data_conversation ON user_data_items(conversation_id);
            CREATE INDEX IF NOT EXISTS idx_user_data_url ON user_data_items(url);
            CREATE INDEX IF NOT EXISTS idx_user_data_project ON user_data_items(project_id);
            CREATE INDEX IF NOT EXISTS idx_user_data_updated ON user_data_items(updated_at);
            CREATE INDEX IF NOT EXISTS idx_knowledge_project ON knowledge_items(project_id);
            CREATE INDEX IF NOT EXISTS idx_knowledge_updated ON knowledge_items(updated_at);
            "#,
        )
        .await?;

        // ── Migrations for existing databases ──────────────────────────────
        // Migrations for existing databases.
        // Only "duplicate column name" is treated as safe/expected.
        // Any other migration error now fails startup.
        db.execute_unprepared("DROP INDEX IF EXISTS idx_episodes_project;")
            .await?;

        let alter_stmts = vec![
            "ALTER TABLE execution_traces ADD COLUMN model TEXT",
            "ALTER TABLE execution_traces ADD COLUMN input_tokens INTEGER NOT NULL DEFAULT 0",
            "ALTER TABLE execution_traces ADD COLUMN output_tokens INTEGER NOT NULL DEFAULT 0",
            "ALTER TABLE execution_traces ADD COLUMN total_tokens INTEGER NOT NULL DEFAULT 0",
            "ALTER TABLE execution_traces ADD COLUMN cost_usd REAL NOT NULL DEFAULT 0.0",
            "ALTER TABLE execution_traces ADD COLUMN complexity TEXT",
        ];
        for stmt in alter_stmts {
            Self::apply_sqlite_add_column_migration(db, stmt).await?;
        }

        Ok(())
    }

    async fn apply_sqlite_add_column_migration(db: &DatabaseConnection, stmt: &str) -> Result<()> {
        match db.execute_unprepared(stmt).await {
            Ok(_) => Ok(()),
            Err(error) => {
                let message = error.to_string().to_ascii_lowercase();
                if message.contains("duplicate column name") {
                    Ok(())
                } else {
                    Err(error.into())
                }
            }
        }
    }

    async fn validate_sqlite_schema(db: &DatabaseConnection) -> Result<()> {
        if db.get_database_backend() != DbBackend::Sqlite {
            return Ok(());
        }

        let quick_check = db
            .query_one(Statement::from_string(
                DbBackend::Sqlite,
                "PRAGMA quick_check".to_string(),
            ))
            .await?
            .and_then(|row| row.try_get::<String>("", "quick_check").ok())
            .unwrap_or_else(|| "unknown".to_string());
        if !quick_check.eq_ignore_ascii_case("ok") {
            anyhow::bail!("SQLite quick_check failed: {}", quick_check);
        }

        let fk_violations = db
            .query_all(Statement::from_string(
                DbBackend::Sqlite,
                "PRAGMA foreign_key_check".to_string(),
            ))
            .await?;
        if let Some(row) = fk_violations.first() {
            let table = row
                .try_get::<String>("", "table")
                .unwrap_or_else(|_| "unknown".to_string());
            let rowid = row
                .try_get::<i64>("", "rowid")
                .map(|value| value.to_string())
                .unwrap_or_else(|_| "?".to_string());
            let parent = row
                .try_get::<String>("", "parent")
                .unwrap_or_else(|_| "unknown".to_string());
            anyhow::bail!(
                "SQLite foreign_key_check failed: table={} rowid={} parent={}",
                table,
                rowid,
                parent
            );
        }

        Ok(())
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

        // Try to find existing
        let existing = kv_store::Entity::find_by_id(key.to_string())
            .one(&self.db)
            .await?;

        if existing.is_some() {
            // Update
            kv_store::ActiveModel {
                key: Set(key.to_string()),
                value: Set(value.to_vec()),
                created_at: sea_orm::NotSet,
                updated_at: Set(now),
            }
            .update(&self.db)
            .await?;
        } else {
            // Insert
            kv_store::ActiveModel {
                key: Set(key.to_string()),
                value: Set(value.to_vec()),
                created_at: Set(now.clone()),
                updated_at: Set(now),
            }
            .insert(&self.db)
            .await?;
        }

        Ok(())
    }

    /// Delete a key from the store
    pub async fn delete(&self, key: &str) -> Result<()> {
        kv_store::Entity::delete_by_id(key.to_string())
            .exec(&self.db)
            .await?;
        Ok(())
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

    pub async fn reencrypt_sensitive_payloads(
        &self,
        old_key: &KeyManager,
        new_key: &KeyManager,
        encrypted_kv_keys: &[&str],
    ) -> Result<()> {
        let txn = self.db.begin().await?;

        let episodes = episode::Entity::find().all(&txn).await?;
        for row in episodes {
            let plaintext = old_key
                .decrypt_string(&row.content)
                .unwrap_or_else(|_| row.content.clone());
            let encrypted = new_key.encrypt_string(&plaintext)?;
            episode::ActiveModel {
                id: Unchanged(row.id),
                content: Set(encrypted),
                ..Default::default()
            }
            .update(&txn)
            .await?;
        }

        let facts = semantic_fact::Entity::find().all(&txn).await?;
        for row in facts {
            let plaintext = old_key
                .decrypt_string(&row.fact)
                .unwrap_or_else(|_| row.fact.clone());
            let encrypted = new_key.encrypt_string(&plaintext)?;
            semantic_fact::ActiveModel {
                id: Unchanged(row.id),
                fact: Set(encrypted),
                ..Default::default()
            }
            .update(&txn)
            .await?;
        }

        let messages = message::Entity::find().all(&txn).await?;
        for row in messages {
            let plaintext = old_key
                .decrypt_string(&row.content)
                .unwrap_or_else(|_| row.content.clone());
            let encrypted = new_key.encrypt_string(&plaintext)?;
            message::ActiveModel {
                id: Unchanged(row.id),
                content: Set(encrypted),
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

        let backend = txn.get_database_backend();
        let automation_runs = txn
            .query_all(Statement::from_string(
                backend,
                "SELECT id, payload FROM automation_runs".to_string(),
            ))
            .await?;
        for row in automation_runs {
            let id: String = row.try_get("", "id")?;
            let payload: String = row.try_get("", "payload")?;
            let plaintext = old_key
                .decrypt_string(&payload)
                .unwrap_or_else(|_| payload.clone());
            let encrypted = new_key.encrypt_string(&plaintext)?;
            txn.execute(Statement::from_sql_and_values(
                backend,
                "UPDATE automation_runs SET payload = ? WHERE id = ?".to_string(),
                vec![encrypted.into(), id.into()],
            ))
            .await?;
        }

        let automation_states = txn
            .query_all(Statement::from_string(
                backend,
                "SELECT automation_id, payload FROM automation_supervisor_states".to_string(),
            ))
            .await?;
        for row in automation_states {
            let automation_id: String = row.try_get("", "automation_id")?;
            let payload: String = row.try_get("", "payload")?;
            let plaintext = old_key
                .decrypt_string(&payload)
                .unwrap_or_else(|_| payload.clone());
            let encrypted = new_key.encrypt_string(&plaintext)?;
            txn.execute(Statement::from_sql_and_values(
                backend,
                "UPDATE automation_supervisor_states SET payload = ? WHERE automation_id = ?"
                    .to_string(),
                vec![encrypted.into(), automation_id.into()],
            ))
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

        self.reencrypt_sensitive_payloads(key_manager, key_manager, encrypted_kv_keys)
            .await?;
        self.set(Self::SENSITIVE_PAYLOAD_BACKFILL_MARKER_KEY, b"done")
            .await?;
        Ok(true)
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
            estimated: Set(usage.estimated),
        }
        .insert(&self.db)
        .await?;
        if let Err(e) = self.maybe_purge_housekeeping_tables().await {
            tracing::warn!(
                "Storage housekeeping purge failed after llm usage insert: {}",
                e
            );
        }
        Ok(())
    }

    /// List LLM usage rows since a given RFC3339 timestamp (ascending).
    pub async fn list_llm_usage_since(&self, since_rfc3339: &str) -> Result<Vec<llm_usage::Model>> {
        let rows = llm_usage::Entity::find()
            .filter(llm_usage::Column::CreatedAt.gte(since_rfc3339.to_string()))
            .order_by_asc(llm_usage::Column::CreatedAt)
            .all(&self.db)
            .await?;
        Ok(rows)
    }

    // ==================== Episodes ====================

    /// Insert an episodic memory entry.
    pub async fn insert_episode(
        &self,
        id: &str,
        content: &str,
        context: &str,
        embedding: Option<Vec<u8>>,
        importance: f32,
        project_id: Option<&str>,
    ) -> Result<()> {
        let now = chrono::Utc::now().to_rfc3339();
        let bounded_importance = importance.clamp(0.0, 1.0);

        episode::ActiveModel {
            id: Set(id.to_string()),
            content: Set(content.to_string()),
            context: Set(context.to_string()),
            embedding: Set(embedding),
            timestamp: Set(now),
            consolidated: Set(false),
            importance: Set(bounded_importance),
            last_accessed: Set(None),
            access_count: Set(0),
            project_id: Set(project_id.map(|s| s.to_string())),
        }
        .insert(&self.db)
        .await?;

        Ok(())
    }

    /// Update episode access time (called when memory is retrieved)
    pub async fn touch_episode(&self, id: &str) -> Result<()> {
        let now = chrono::Utc::now().to_rfc3339();

        episode::Entity::update_many()
            .col_expr(episode::Column::LastAccessed, Expr::value(now))
            .col_expr(
                episode::Column::AccessCount,
                Expr::col(episode::Column::AccessCount).add(1),
            )
            .filter(episode::Column::Id.eq(id))
            .exec(&self.db)
            .await?;

        Ok(())
    }

    /// Get all episodes with their metadata for scoring
    pub async fn get_all_episodes_for_scoring(&self) -> Result<Vec<episode::Model>> {
        let episodes = episode::Entity::find()
            .order_by_desc(episode::Column::Timestamp)
            .limit(Self::MAX_EPISODES_FOR_SCORING)
            .all(&self.db)
            .await?;

        Ok(episodes)
    }

    /// Count episodes
    pub async fn count_episodes(&self) -> Result<u64> {
        let count = episode::Entity::find().count(&self.db).await?;
        Ok(count)
    }

    /// List newest episode ids (by timestamp desc).
    pub async fn list_newest_episode_ids(&self, limit: u64) -> Result<Vec<String>> {
        if limit == 0 {
            return Ok(vec![]);
        }
        let models = episode::Entity::find()
            .select_only()
            .column(episode::Column::Id)
            .order_by_desc(episode::Column::Timestamp)
            .limit(limit)
            .into_tuple::<String>()
            .all(&self.db)
            .await?;
        Ok(models)
    }

    /// List candidate episode ids for pruning based on metadata only (no decryption).
    pub async fn list_episode_prune_candidates(
        &self,
        cutoff_rfc3339: &str,
        require_consolidated: bool,
        max_importance: f32,
        max_access_count: i32,
        limit: u64,
    ) -> Result<Vec<String>> {
        if limit == 0 {
            return Ok(vec![]);
        }
        let mut query = episode::Entity::find()
            .select_only()
            .column(episode::Column::Id)
            .filter(episode::Column::Timestamp.lte(cutoff_rfc3339.to_string()))
            .filter(episode::Column::Importance.lte(max_importance))
            .filter(episode::Column::AccessCount.lte(max_access_count))
            .order_by_asc(episode::Column::Timestamp)
            .limit(limit);
        if require_consolidated {
            query = query.filter(episode::Column::Consolidated.eq(true));
        }
        let ids = query.into_tuple::<String>().all(&self.db).await?;
        Ok(ids)
    }

    /// List all semantic fact source blobs (JSON arrays of episode UUIDs).
    pub async fn list_all_semantic_fact_sources(&self) -> Result<Vec<String>> {
        let rows = semantic_fact::Entity::find()
            .select_only()
            .column(semantic_fact::Column::Sources)
            .into_tuple::<String>()
            .all(&self.db)
            .await?;
        Ok(rows)
    }

    /// Delete episodes by id. Returns rows affected.
    pub async fn delete_episodes_by_ids(&self, ids: &[String]) -> Result<u64> {
        if ids.is_empty() {
            return Ok(0);
        }
        let res = episode::Entity::delete_many()
            .filter(episode::Column::Id.is_in(ids.to_vec()))
            .exec(&self.db)
            .await?;
        Ok(res.rows_affected)
    }

    // ==================== Semantic Facts ====================

    /// Insert a semantic fact
    pub async fn insert_fact(
        &self,
        id: &str,
        fact: &str,
        confidence: f32,
        sources: &str,
        embedding: Option<Vec<u8>>,
        project_id: Option<&str>,
    ) -> Result<()> {
        let now = chrono::Utc::now().to_rfc3339();

        semantic_fact::ActiveModel {
            id: Set(id.to_string()),
            fact: Set(fact.to_string()),
            confidence: Set(confidence),
            sources: Set(sources.to_string()),
            embedding: Set(embedding),
            created_at: Set(now),
            project_id: Set(project_id.map(|s| s.to_string())),
        }
        .insert(&self.db)
        .await?;

        Ok(())
    }

    /// Get all semantic facts
    pub async fn get_facts(&self) -> Result<Vec<semantic_fact::Model>> {
        let facts = semantic_fact::Entity::find().all(&self.db).await?;
        Ok(facts)
    }

    /// Get episodes filtered by project
    pub async fn get_episodes_by_project(
        &self,
        limit: u64,
        offset: u64,
        project_id: Option<&str>,
    ) -> Result<Vec<episode::Model>> {
        let mut query = episode::Entity::find().order_by_desc(episode::Column::Timestamp);
        if let Some(pid) = project_id {
            query = query.filter(episode::Column::ProjectId.eq(pid));
        }
        let episodes = query.limit(limit).offset(offset).all(&self.db).await?;
        Ok(episodes)
    }

    /// Count episodes filtered by project
    pub async fn count_episodes_by_project(&self, project_id: Option<&str>) -> Result<u64> {
        let mut query = episode::Entity::find();
        if let Some(pid) = project_id {
            query = query.filter(episode::Column::ProjectId.eq(pid));
        }
        let count = query.count(&self.db).await?;
        Ok(count)
    }

    /// Get facts filtered by project (paginated)
    pub async fn get_facts_by_project(
        &self,
        limit: u64,
        offset: u64,
        project_id: Option<&str>,
    ) -> Result<Vec<semantic_fact::Model>> {
        let mut query = semantic_fact::Entity::find();
        if let Some(pid) = project_id {
            query = query.filter(semantic_fact::Column::ProjectId.eq(pid));
        }
        let facts = query.limit(limit).offset(offset).all(&self.db).await?;
        Ok(facts)
    }

    /// Count facts
    pub async fn count_facts(&self, project_id: Option<&str>) -> Result<u64> {
        let mut query = semantic_fact::Entity::find();
        if let Some(pid) = project_id {
            query = query.filter(semantic_fact::Column::ProjectId.eq(pid));
        }
        Ok(query.count(&self.db).await?)
    }

    /// Get episodes for scoring, scoped to project (includes global episodes too)
    pub async fn get_all_episodes_for_scoring_by_project(
        &self,
        project_id: Option<&str>,
    ) -> Result<Vec<episode::Model>> {
        let mut query = episode::Entity::find().order_by_desc(episode::Column::Timestamp);
        if let Some(pid) = project_id {
            query = query.filter(
                Condition::any()
                    .add(episode::Column::ProjectId.eq(pid))
                    .add(episode::Column::ProjectId.is_null()),
            );
        }
        let episodes = query
            .limit(Self::MAX_EPISODES_FOR_SCORING)
            .all(&self.db)
            .await?;
        Ok(episodes)
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

        if let Some(existing) = user_preference::Entity::find_by_id(id.clone())
            .one(&self.db)
            .await?
        {
            let mut model: user_preference::ActiveModel = existing.into();
            model.key = Set(key.to_ascii_lowercase());
            model.value = Set(encrypted_value.clone());
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

    /// List user preferences by scope.
    pub async fn list_user_preferences(
        &self,
        limit: u64,
        offset: u64,
        project_id: Option<&str>,
    ) -> Result<Vec<user_preference::Model>> {
        let mut query =
            user_preference::Entity::find().order_by_desc(user_preference::Column::UpdatedAt);
        if let Some(pid) = project_id {
            query = query.filter(user_preference::Column::ProjectId.eq(pid));
        }
        let mut rows = query.limit(limit).offset(offset).all(&self.db).await?;
        for row in &mut rows {
            row.value = decrypt_storage_string(&row.value);
        }
        Ok(rows)
    }

    /// Count user preferences by scope.
    pub async fn count_user_preferences(&self, project_id: Option<&str>) -> Result<u64> {
        let mut query = user_preference::Entity::find();
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
        let mut rows = query.limit(limit).offset(offset).all(&self.db).await?;
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

    /// List knowledge base items by scope.
    pub async fn list_knowledge_items(
        &self,
        limit: u64,
        offset: u64,
        project_id: Option<&str>,
    ) -> Result<Vec<knowledge_item::Model>> {
        let mut query =
            knowledge_item::Entity::find().order_by_desc(knowledge_item::Column::UpdatedAt);
        if let Some(pid) = project_id {
            query = query.filter(knowledge_item::Column::ProjectId.eq(pid));
        }
        let mut rows = query.limit(limit).offset(offset).all(&self.db).await?;
        for row in &mut rows {
            row.title = decrypt_storage_string(&row.title);
            row.content = decrypt_storage_string(&row.content);
        }
        Ok(rows)
    }

    /// Count knowledge base items by scope.
    pub async fn count_knowledge_items(&self, project_id: Option<&str>) -> Result<u64> {
        let mut query = knowledge_item::Entity::find();
        if let Some(pid) = project_id {
            query = query.filter(knowledge_item::Column::ProjectId.eq(pid));
        }
        Ok(query.count(&self.db).await?)
    }

    /// Delete a knowledge base item.
    pub async fn delete_knowledge_item(&self, id: &str) -> Result<bool> {
        let result = knowledge_item::Entity::delete_by_id(id.to_string())
            .exec(&self.db)
            .await?;
        Ok(result.rows_affected > 0)
    }

    /// Insert a task
    pub async fn insert_task(&self, task: &crate::core::Task) -> Result<()> {
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
            scheduled_for: Set(task.scheduled_for.map(|t| t.to_rfc3339())),
            cron: Set(task.cron.clone()),
            result: Set(result),
            proof_id: Set(task.proof_id.map(|id| id.to_string())),
            priority: Set(task.priority.map(|v| v as f64)),
            urgency: Set(task.urgency.map(|v| v as f64)),
            importance: Set(task.importance.map(|v| v as f64)),
            eisenhower_quadrant: Set(task.eisenhower_quadrant.map(|v| v as i32)),
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

        model.update(&self.db).await?;
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
            ..Default::default()
        }
        .update(&self.db)
        .await?;

        Ok(())
    }

    /// Delete a task
    pub async fn delete_task(&self, id: &str) -> Result<()> {
        task::Entity::delete_by_id(id.to_string())
            .exec(&self.db)
            .await?;
        Ok(())
    }

    /// Get all tasks
    pub async fn get_tasks(&self) -> Result<Vec<task::Model>> {
        let mut tasks = task::Entity::find().all(&self.db).await?;
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
        let backend = self.db.get_database_backend();
        let rows = self
            .db
            .query_all(Statement::from_sql_and_values(
                backend,
                "SELECT payload FROM automation_runs ORDER BY started_at DESC LIMIT ?".to_string(),
                vec![(limit.max(1) as i64).into()],
            ))
            .await?;
        let mut runs = Vec::new();
        for row in rows {
            let payload = decrypt_storage_string(&row.try_get::<String>("", "payload")?);
            if let Ok(run) =
                serde_json::from_str::<crate::core::automation::AutomationRunRecord>(&payload)
            {
                runs.push(run);
            }
        }
        Ok(runs)
    }

    pub async fn append_automation_run(
        &self,
        run: &crate::core::automation::AutomationRunRecord,
        max_records: usize,
    ) -> Result<()> {
        let backend = self.db.get_database_backend();
        self.db
            .execute(Statement::from_sql_and_values(
                backend,
                "INSERT INTO automation_runs (id, automation_id, started_at, payload) VALUES (?, ?, ?, ?) \
                 ON CONFLICT(id) DO UPDATE SET automation_id=excluded.automation_id, started_at=excluded.started_at, payload=excluded.payload"
                    .to_string(),
                vec![
                    run.id.clone().into(),
                    run.automation_id.clone().into(),
                    run.started_at.clone().into(),
                    encrypt_storage_string(&serde_json::to_string(run)?)?.into(),
                ],
            ))
            .await?;
        self.db
            .execute(Statement::from_sql_and_values(
                backend,
                "DELETE FROM automation_runs WHERE id NOT IN (SELECT id FROM automation_runs ORDER BY started_at DESC LIMIT ?)"
                    .to_string(),
                vec![(max_records.max(1) as i64).into()],
            ))
            .await?;
        Ok(())
    }

    pub async fn list_automation_supervisor_states(
        &self,
    ) -> Result<Vec<crate::core::automation::AutomationSupervisorState>> {
        let backend = self.db.get_database_backend();
        let rows = self
            .db
            .query_all(Statement::from_string(
                backend,
                "SELECT payload FROM automation_supervisor_states ORDER BY updated_at DESC"
                    .to_string(),
            ))
            .await?;
        let mut states = Vec::new();
        for row in rows {
            let payload = decrypt_storage_string(&row.try_get::<String>("", "payload")?);
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
        let backend = self.db.get_database_backend();
        let row = self
            .db
            .query_one(Statement::from_sql_and_values(
                backend,
                "SELECT payload FROM automation_supervisor_states WHERE automation_id = ?"
                    .to_string(),
                vec![automation_id.to_string().into()],
            ))
            .await?;
        Ok(row
            .and_then(|row| row.try_get::<String>("", "payload").ok())
            .map(|payload| decrypt_storage_string(&payload))
            .and_then(|payload| {
                serde_json::from_str::<crate::core::automation::AutomationSupervisorState>(&payload)
                    .ok()
            }))
    }

    pub async fn upsert_automation_supervisor_state(
        &self,
        state: &crate::core::automation::AutomationSupervisorState,
    ) -> Result<()> {
        let backend = self.db.get_database_backend();
        let updated_at = state
            .last_run_at
            .clone()
            .or_else(|| state.created_at.clone())
            .unwrap_or_else(|| chrono::Utc::now().to_rfc3339());
        self.db
            .execute(Statement::from_sql_and_values(
                backend,
                "INSERT INTO automation_supervisor_states (automation_id, updated_at, payload) VALUES (?, ?, ?) \
                 ON CONFLICT(automation_id) DO UPDATE SET updated_at=excluded.updated_at, payload=excluded.payload"
                    .to_string(),
                vec![
                    state.automation_id.clone().into(),
                    updated_at.into(),
                    encrypt_storage_string(&serde_json::to_string(state)?)?.into(),
                ],
            ))
            .await?;
        Ok(())
    }

    pub async fn delete_automation_supervisor_state(&self, automation_id: &str) -> Result<bool> {
        let result = self
            .db
            .execute(Statement::from_sql_and_values(
                self.db.get_database_backend(),
                "DELETE FROM automation_supervisor_states WHERE automation_id = ?".to_string(),
                vec![automation_id.to_string().into()],
            ))
            .await?;
        Ok(result.rows_affected() > 0)
    }

    pub async fn list_watchers(&self) -> Result<Vec<crate::core::watcher::Watcher>> {
        let rows = self
            .db
            .query_all(Statement::from_string(
                self.db.get_database_backend(),
                "SELECT payload FROM watchers ORDER BY created_at ASC".to_string(),
            ))
            .await?;
        let mut watchers = Vec::new();
        for row in rows {
            let payload: String = row.try_get("", "payload")?;
            if let Ok(watcher) = serde_json::from_str::<crate::core::watcher::Watcher>(&payload) {
                watchers.push(watcher);
            }
        }
        Ok(watchers)
    }

    pub async fn replace_active_watchers(
        &self,
        watchers: &[crate::core::watcher::Watcher],
    ) -> Result<()> {
        let backend = self.db.get_database_backend();
        let txn = self.db.begin().await?;
        txn.execute(Statement::from_string(
            backend,
            "DELETE FROM watchers".to_string(),
        ))
        .await?;
        for watcher in watchers {
            let status = match &watcher.status {
                crate::core::watcher::WatcherStatus::Active => "active",
                crate::core::watcher::WatcherStatus::Paused => "paused",
                crate::core::watcher::WatcherStatus::Triggered => "triggered",
                crate::core::watcher::WatcherStatus::TimedOut => "timed_out",
                crate::core::watcher::WatcherStatus::Cancelled => "cancelled",
                crate::core::watcher::WatcherStatus::Failed { .. } => "failed",
            };
            txn.execute(Statement::from_sql_and_values(
                backend,
                "INSERT INTO watchers (id, status, created_at, payload) VALUES (?, ?, ?, ?)"
                    .to_string(),
                vec![
                    watcher.id.to_string().into(),
                    status.into(),
                    watcher.created_at.to_rfc3339().into(),
                    serde_json::to_string(watcher)?.into(),
                ],
            ))
            .await?;
        }
        txn.commit().await?;
        Ok(())
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
            enabled: Set(agent.enabled),
            created_at: Set(agent.created_at.clone()),
        }
        .insert(&self.db)
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
                enabled: 1,
                created_at: now.clone(),
            },
            swarm_agent::Model {
                id: format!("default-writer-{}", &uuid::Uuid::new_v4().to_string()[..8]),
                name: "Writer".to_string(),
                agent_type: "writer".to_string(),
                llm_provider: "{}".to_string(),
                capabilities: r#"["content writing","editing","summarization","translation","creative writing"]"#.to_string(),
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
            .limit(limit)
            .all(&self.db)
            .await?;
        for delegation in &mut delegations {
            delegation.task_description = decrypt_storage_string(&delegation.task_description);
            delegation.result = decrypt_optional_storage_string(delegation.result.clone());
        }
        Ok(delegations)
    }

    /// Get all swarm delegations
    pub async fn get_all_delegations(&self) -> Result<Vec<swarm_delegation::Model>> {
        let mut delegations = swarm_delegation::Entity::find()
            .order_by_desc(swarm_delegation::Column::CreatedAt)
            .all(&self.db)
            .await?;
        for delegation in &mut delegations {
            delegation.task_description = decrypt_storage_string(&delegation.task_description);
            delegation.result = decrypt_optional_storage_string(delegation.result.clone());
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
        if let Err(e) = self.maybe_purge_housekeeping_tables().await {
            tracing::warn!(
                "Storage housekeeping purge failed after delegation insert: {}",
                e
            );
        }
        Ok(())
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
        }
        .insert(&self.db)
        .await?;
        Ok(())
    }

    /// List conversations (newest first, paginated)
    pub async fn list_conversations(
        &self,
        limit: u64,
        offset: u64,
        project_id: Option<&str>,
    ) -> Result<Vec<conversation::Model>> {
        let mut query = conversation::Entity::find().order_by_desc(conversation::Column::UpdatedAt);

        if let Some(pid) = project_id {
            query = query.filter(conversation::Column::ProjectId.eq(pid));
        }

        let convs = query.limit(limit).offset(offset).all(&self.db).await?;
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

        let convs = query.limit(limit).all(&self.db).await?;
        Ok(convs)
    }

    /// Count conversations
    pub async fn count_conversations(&self, project_id: Option<&str>) -> Result<u64> {
        let mut query = conversation::Entity::find();
        if let Some(pid) = project_id {
            query = query.filter(conversation::Column::ProjectId.eq(pid));
        }
        Ok(query.count(&self.db).await?)
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
    ) -> Result<()> {
        let now = chrono::Utc::now().to_rfc3339();
        let mut model = conversation::ActiveModel {
            id: Set(id.to_string()),
            updated_at: Set(now),
            ..Default::default()
        };
        if let Some(t) = title {
            model.title = Set(t.to_string());
        }
        if let Some(mc) = message_count {
            model.message_count = Set(mc);
        }
        model.update(&self.db).await?;
        Ok(())
    }

    /// Delete a conversation and its messages
    pub async fn delete_conversation(&self, id: &str) -> Result<()> {
        let txn = self.db.begin().await?;
        message::Entity::delete_many()
            .filter(message::Column::ConversationId.eq(id))
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
        message::ActiveModel {
            id: Set(msg.id.clone()),
            conversation_id: Set(msg.conversation_id.clone()),
            role: Set(msg.role.clone()),
            content: Set(content),
            timestamp: Set(msg.timestamp.clone()),
            model_used: Set(msg.model_used.clone()),
            trace_id: Set(msg.trace_id.clone()),
        }
        .insert(&self.db)
        .await?;

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

        if let Err(e) = self.maybe_purge_housekeeping_tables().await {
            tracing::warn!(
                "Storage housekeeping purge failed after message insert: {}",
                e
            );
        }

        Ok(())
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
            .limit(limit)
            .offset(offset)
            .all(&self.db)
            .await?;
        for msg in &mut msgs {
            msg.content = decrypt_storage_string(&msg.content);
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
            .limit(limit)
            .all(&self.db)
            .await?;
        msgs.reverse();
        for msg in &mut msgs {
            msg.content = decrypt_storage_string(&msg.content);
        }
        Ok(msgs)
    }

    /// Get most recent user-authored chat messages across conversations.
    pub async fn get_recent_user_messages(&self, limit: u64) -> Result<Vec<message::Model>> {
        let mut msgs = message::Entity::find()
            .filter(message::Column::Role.eq("user"))
            .order_by_desc(message::Column::Timestamp)
            .limit(limit)
            .all(&self.db)
            .await?;
        for msg in &mut msgs {
            msg.content = decrypt_storage_string(&msg.content);
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

    // ==================== Projects ====================

    /// Create a project
    pub async fn create_project(&self, proj: &project::Model) -> Result<()> {
        project::ActiveModel {
            id: Set(proj.id.clone()),
            name: Set(proj.name.clone()),
            description: Set(proj.description.clone()),
            system_prompt: Set(proj.system_prompt.clone()),
            personality: Set(proj.personality.clone()),
            tools_filter: Set(proj.tools_filter.clone()),
            active: Set(proj.active),
            created_at: Set(proj.created_at.clone()),
            updated_at: Set(proj.updated_at.clone()),
        }
        .insert(&self.db)
        .await?;
        Ok(())
    }

    /// List projects
    pub async fn list_projects(&self) -> Result<Vec<project::Model>> {
        let projects = project::Entity::find()
            .order_by_desc(project::Column::UpdatedAt)
            .all(&self.db)
            .await?;
        Ok(projects)
    }

    /// Get a project by ID
    pub async fn get_project(&self, id: &str) -> Result<Option<project::Model>> {
        let proj = project::Entity::find_by_id(id.to_string())
            .one(&self.db)
            .await?;
        Ok(proj)
    }

    /// Update a project
    pub async fn update_project(&self, proj: &project::Model) -> Result<()> {
        project::ActiveModel {
            id: Set(proj.id.clone()),
            name: Set(proj.name.clone()),
            description: Set(proj.description.clone()),
            system_prompt: Set(proj.system_prompt.clone()),
            personality: Set(proj.personality.clone()),
            tools_filter: Set(proj.tools_filter.clone()),
            active: Set(proj.active),
            updated_at: Set(chrono::Utc::now().to_rfc3339()),
            ..Default::default()
        }
        .update(&self.db)
        .await?;
        Ok(())
    }

    /// Delete a project
    pub async fn delete_project(&self, id: &str) -> Result<()> {
        let txn = self.db.begin().await?;

        // Documents + chunks
        let doc_ids: Vec<String> = document::Entity::find()
            .select_only()
            .column(document::Column::Id)
            .filter(document::Column::ProjectId.eq(id))
            .into_tuple::<String>()
            .all(&txn)
            .await?;
        if !doc_ids.is_empty() {
            document_chunk::Entity::delete_many()
                .filter(document_chunk::Column::DocumentId.is_in(doc_ids))
                .exec(&txn)
                .await?;
        }
        document::Entity::delete_many()
            .filter(document::Column::ProjectId.eq(id))
            .exec(&txn)
            .await?;

        // Conversations + messages
        let conv_ids: Vec<String> = conversation::Entity::find()
            .select_only()
            .column(conversation::Column::Id)
            .filter(conversation::Column::ProjectId.eq(id))
            .into_tuple::<String>()
            .all(&txn)
            .await?;
        if !conv_ids.is_empty() {
            message::Entity::delete_many()
                .filter(message::Column::ConversationId.is_in(conv_ids))
                .exec(&txn)
                .await?;
        }
        conversation::Entity::delete_many()
            .filter(conversation::Column::ProjectId.eq(id))
            .exec(&txn)
            .await?;

        // Memory scoped to project
        episode::Entity::delete_many()
            .filter(episode::Column::ProjectId.eq(id))
            .exec(&txn)
            .await?;
        semantic_fact::Entity::delete_many()
            .filter(semantic_fact::Column::ProjectId.eq(id))
            .exec(&txn)
            .await?;
        user_preference::Entity::delete_many()
            .filter(user_preference::Column::ProjectId.eq(id))
            .exec(&txn)
            .await?;
        user_data_item::Entity::delete_many()
            .filter(user_data_item::Column::ProjectId.eq(id))
            .exec(&txn)
            .await?;
        knowledge_item::Entity::delete_many()
            .filter(knowledge_item::Column::ProjectId.eq(id))
            .exec(&txn)
            .await?;

        // Finally delete the project row
        let res = project::Entity::delete_by_id(id.to_string())
            .exec(&txn)
            .await?;
        if res.rows_affected == 0 {
            txn.rollback().await?;
            anyhow::bail!("Project not found");
        }

        txn.commit().await?;
        Ok(())
    }

    // ==================== Documents ====================

    /// Insert a document record
    pub async fn insert_document(&self, doc: &document::Model) -> Result<()> {
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
        .insert(&self.db)
        .await?;
        Ok(())
    }

    /// Insert a document chunk
    pub async fn insert_document_chunk(&self, chunk: &document_chunk::Model) -> Result<()> {
        let content = encrypt_storage_string(&chunk.content)?;
        document_chunk::ActiveModel {
            id: Set(chunk.id.clone()),
            document_id: Set(chunk.document_id.clone()),
            chunk_index: Set(chunk.chunk_index),
            content: Set(content),
            embedding: Set(chunk.embedding.clone()),
        }
        .insert(&self.db)
        .await?;
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
        if let Some(pid) = project_id {
            query = query.filter(document::Column::ProjectId.eq(pid));
        }
        let mut docs = query.limit(limit).offset(offset).all(&self.db).await?;
        for doc in &mut docs {
            doc.filename = decrypt_storage_string(&doc.filename);
        }
        Ok(docs)
    }

    /// Count documents
    pub async fn count_documents(&self, project_id: Option<&str>) -> Result<u64> {
        let mut query = document::Entity::find();
        if let Some(pid) = project_id {
            query = query.filter(document::Column::ProjectId.eq(pid));
        }
        Ok(query.count(&self.db).await?)
    }

    /// Get document chunks for search
    pub async fn get_document_chunks(
        &self,
        document_id: &str,
    ) -> Result<Vec<document_chunk::Model>> {
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

    /// Get all chunks (across all documents, for search)
    pub async fn get_all_document_chunks(&self) -> Result<Vec<document_chunk::Model>> {
        let mut chunks = document_chunk::Entity::find()
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

    /// Delete a document and its chunks
    pub async fn delete_document(&self, id: &str) -> Result<()> {
        let txn = self.db.begin().await?;
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

    const NOTIFICATION_RETENTION_DAYS: i64 = 7;
    // Deduplicate repetitive notifications (same root message) to avoid spamming users/UI.
    // This is separate from retention, which deletes old rows after NOTIFICATION_RETENTION_DAYS.
    const NOTIFICATION_DEDUP_COOLDOWN_DAYS: i64 = 7;
    const ARKPULSE_NOTIFICATION_WINDOW_HOURS: i64 = 24;
    const NOTIFICATION_PURGE_MIN_INTERVAL_SECS: i64 = 3600;
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

    fn collapse_recent_arkpulse_notifications(
        notifications: Vec<notification::Model>,
    ) -> Vec<notification::Model> {
        let cutoff =
            chrono::Utc::now() - chrono::Duration::hours(Self::ARKPULSE_NOTIFICATION_WINDOW_HOURS);
        let mut kept_recent_arkpulse = false;
        let mut filtered = Vec::with_capacity(notifications.len());

        for notif in notifications {
            if !Self::is_arkpulse_notification(&notif.source) {
                filtered.push(notif);
                continue;
            }

            let is_recent = chrono::DateTime::parse_from_rfc3339(&notif.created_at)
                .map(|dt| dt.with_timezone(&chrono::Utc) >= cutoff)
                .unwrap_or(true);
            if !is_recent {
                filtered.push(notif);
                continue;
            }

            if kept_recent_arkpulse {
                continue;
            }
            kept_recent_arkpulse = true;
            filtered.push(notif);
        }

        filtered
    }

    async fn count_recent_arkpulse_notifications(&self, unread_only: bool) -> Result<u64> {
        let mut query = notification::Entity::find()
            .filter(notification::Column::Source.eq("arkpulse"))
            .filter(notification::Column::CreatedAt.gte(Self::arkpulse_recent_cutoff_rfc3339()));
        if unread_only {
            query = query.filter(notification::Column::Read.eq(false));
        }
        Ok(query.count(&self.db).await?)
    }

    async fn maybe_purge_old_notifications(&self) -> Result<()> {
        let now = chrono::Utc::now().timestamp();
        let last_run = self
            .get(Self::NOTIFICATION_PURGE_LAST_RUN_KEY)
            .await?
            .and_then(|bytes| String::from_utf8(bytes).ok())
            .and_then(|s| s.parse::<i64>().ok())
            .unwrap_or(0);

        if last_run > 0 && (now - last_run) < Self::NOTIFICATION_PURGE_MIN_INTERVAL_SECS {
            return Ok(());
        }

        let cutoff = (chrono::Utc::now()
            - chrono::Duration::days(Self::NOTIFICATION_RETENTION_DAYS))
        .to_rfc3339();

        let result = notification::Entity::delete_many()
            .filter(notification::Column::CreatedAt.lt(cutoff))
            .exec(&self.db)
            .await?;

        let _ = self
            .set(
                Self::NOTIFICATION_PURGE_LAST_RUN_KEY,
                now.to_string().as_bytes(),
            )
            .await;

        if result.rows_affected > 0 {
            tracing::info!(
                "Purged {} notifications older than {} days",
                result.rows_affected,
                Self::NOTIFICATION_RETENTION_DAYS
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

        if Self::is_arkpulse_notification(&source_clean) {
            let existing_recent = notification::Entity::find()
                .filter(notification::Column::Source.eq(source_clean.clone()))
                .filter(notification::Column::CreatedAt.gte(Self::arkpulse_recent_cutoff_rfc3339()))
                .order_by_desc(notification::Column::CreatedAt)
                .limit(1)
                .one(&self.db)
                .await?;
            if existing_recent.is_some() {
                return Ok(());
            }
        }

        // Best-effort deduplication to prevent repeated notifications from flooding the DB/UI.
        // Critical/security notifications bypass dedup.
        if !Self::notification_is_critical(&level_clean, &source_clean, &title_clean) {
            let cutoff = (chrono::Utc::now()
                - chrono::Duration::days(Self::NOTIFICATION_DEDUP_COOLDOWN_DAYS))
            .to_rfc3339();
            let sig = Self::notification_body_signature(&body_clean);
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
                            && Self::notification_body_signature(&existing_body) == sig
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
        let mut query = notification::Entity::find().order_by_desc(notification::Column::CreatedAt);
        if unread_only {
            query = query.filter(notification::Column::Read.eq(false));
        }
        let mut notifs = query.limit(limit).offset(offset).all(&self.db).await?;
        for notif in &mut notifs {
            notif.title = decrypt_storage_string(&notif.title);
            notif.body = decrypt_storage_string(&notif.body);
        }
        Ok(Self::collapse_recent_arkpulse_notifications(notifs))
    }

    /// Count notifications
    pub async fn count_notifications(&self, unread_only: bool) -> Result<u64> {
        if let Err(e) = self.maybe_purge_old_notifications().await {
            tracing::warn!("Notification retention purge failed: {}", e);
        }
        let mut query = notification::Entity::find();
        if unread_only {
            query = query.filter(notification::Column::Read.eq(false));
        }
        let count = query.count(&self.db).await?;
        let arkpulse_recent = self
            .count_recent_arkpulse_notifications(unread_only)
            .await
            .unwrap_or(0);
        if arkpulse_recent > 1 {
            Ok(count.saturating_sub(arkpulse_recent - 1))
        } else {
            Ok(count)
        }
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
        self.db
            .execute_unprepared("UPDATE notifications SET read = 1")
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
        let count = notification::Entity::find()
            .filter(notification::Column::Read.eq(false))
            .count(&self.db)
            .await?;
        let arkpulse_recent_unread = self
            .count_recent_arkpulse_notifications(true)
            .await
            .unwrap_or(0);
        if arkpulse_recent_unread > 1 {
            Ok(count.saturating_sub(arkpulse_recent_unread - 1))
        } else {
            Ok(count)
        }
    }

    /// Mark episodes as consolidated
    pub async fn mark_episodes_consolidated(&self, ids: &[String]) -> Result<()> {
        if ids.is_empty() {
            return Ok(());
        }
        episode::Entity::update_many()
            .col_expr(episode::Column::Consolidated, Expr::value(true))
            .filter(episode::Column::Id.is_in(ids.to_vec()))
            .exec(&self.db)
            .await?;
        Ok(())
    }

    /// Get unconsolidated episodes for LLM consolidation
    pub async fn get_unconsolidated_episodes(&self, limit: u64) -> Result<Vec<episode::Model>> {
        let episodes = episode::Entity::find()
            .filter(episode::Column::Consolidated.eq(false))
            .order_by_asc(episode::Column::Timestamp)
            .limit(limit)
            .all(&self.db)
            .await?;
        Ok(episodes)
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
            .limit(limit)
            .offset(offset)
            .all(&self.db)
            .await?;
        for row in &mut log {
            row.arguments = decrypt_storage_string(&row.arguments);
        }
        Ok(log)
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
        let existing = approval_log::Entity::find_by_id(id.to_string())
            .one(&self.db)
            .await?;
        if existing.is_some() {
            approval_log::ActiveModel {
                id: Set(id.to_string()),
                action_name: Set(action_name.to_string()),
                arguments: Set(arguments.clone()),
                rule_name: Set(rule_name.to_string()),
                status: Set("pending".to_string()),
                requested_at: Set(requested_at.to_string()),
                resolved_at: Set(None),
                resolved_by: Set(None),
            }
            .update(&self.db)
            .await?;
        } else {
            approval_log::ActiveModel {
                id: Set(id.to_string()),
                action_name: Set(action_name.to_string()),
                arguments: Set(arguments),
                rule_name: Set(rule_name.to_string()),
                status: Set("pending".to_string()),
                requested_at: Set(requested_at.to_string()),
                resolved_at: Set(None),
                resolved_by: Set(None),
            }
            .insert(&self.db)
            .await?;
        }
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

        crate::storage::entities::execution_trace::ActiveModel {
            id: Set(trace.id.clone()),
            message: Set(message),
            channel: Set(trace.channel.clone()),
            started_at: Set(started_at),
            completed_at: Set(completed_at),
            duration_ms: Set(duration_ms),
            step_count: Set(trace.steps.len() as i64),
            steps_json: Set(steps_json),
            response: Set(response),
            proof_id: Set(trace.proof_id.clone()),
            model: Set(trace.model.clone()),
            input_tokens: Set(trace.input_tokens),
            output_tokens: Set(trace.output_tokens),
            total_tokens: Set(trace.total_tokens),
            cost_usd: Set(trace.cost_usd),
            complexity: Set(trace.complexity.clone()),
            created_at: Set(created_at),
        }
        .insert(&self.db)
        .await?;
        if let Err(e) = self.maybe_purge_housekeeping_tables().await {
            tracing::warn!(
                "Storage housekeeping purge failed after trace insert: {}",
                e
            );
        }
        Ok(())
    }

    /// List persisted execution traces (newest first).
    pub async fn list_execution_traces(
        &self,
        limit: u64,
        offset: u64,
    ) -> Result<Vec<crate::storage::entities::execution_trace::Model>> {
        let mut traces = crate::storage::entities::execution_trace::Entity::find()
            .order_by_desc(crate::storage::entities::execution_trace::Column::CreatedAt)
            .limit(limit)
            .offset(offset)
            .all(&self.db)
            .await?;
        for trace in &mut traces {
            trace.message = decrypt_storage_string(&trace.message);
            trace.steps_json = decrypt_storage_string(&trace.steps_json);
            trace.response = decrypt_optional_storage_string(trace.response.take());
        }
        Ok(traces)
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
        if let Err(e) = self.maybe_purge_housekeeping_tables().await {
            tracing::warn!(
                "Storage housekeeping purge failed after security log insert: {}",
                e
            );
        }
        Ok(())
    }

    /// List recent security logs (newest first)
    pub async fn list_security_logs(&self, limit: u64) -> Result<Vec<security_log::Model>> {
        let mut logs = security_log::Entity::find()
            .order_by_desc(security_log::Column::CreatedAt)
            .limit(limit)
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

        let mut logs = query.limit(limit).offset(offset).all(&self.db).await?;
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

    // ==================== Operational Logs ====================

    /// Insert a structured operational telemetry entry.
    pub async fn insert_operational_log(&self, log: &operational_log::Model) -> Result<()> {
        operational_log::ActiveModel {
            id: Set(log.id.clone()),
            created_at: Set(log.created_at.clone()),
            trace_id: Set(log.trace_id.clone()),
            conversation_id: Set(log.conversation_id.clone()),
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
        if let Err(e) = self.maybe_purge_housekeeping_tables().await {
            tracing::warn!(
                "Storage housekeeping purge failed after operational log insert: {}",
                e
            );
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
            .limit(limit)
            .all(&self.db)
            .await?;
        for row in &mut rows {
            row.outcome = decrypt_storage_string(&row.outcome);
            row.arguments = decrypt_optional_storage_string(row.arguments.clone());
            row.payload = decrypt_optional_storage_string(row.payload.clone());
        }
        Ok(rows)
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

    async fn maybe_purge_housekeeping_tables(&self) -> Result<()> {
        let now = chrono::Utc::now();
        if let Some(bytes) = self.get(Self::HOUSEKEEPING_PURGE_LAST_RUN_KEY).await? {
            if let Ok(raw) = String::from_utf8(bytes) {
                if let Ok(last) = chrono::DateTime::parse_from_rfc3339(&raw) {
                    if (now - last.with_timezone(&chrono::Utc)).num_seconds()
                        < Self::HOUSEKEEPING_PURGE_MIN_INTERVAL_SECS
                    {
                        return Ok(());
                    }
                }
            }
        }

        let trace_cutoff =
            (now - chrono::Duration::days(Self::EXECUTION_TRACE_RETENTION_DAYS)).to_rfc3339();
        let proof_cutoff =
            (now - chrono::Duration::days(Self::EXECUTION_PROOF_RETENTION_DAYS)).to_rfc3339();
        let operational_cutoff =
            (now - chrono::Duration::days(Self::OPERATIONAL_LOG_RETENTION_DAYS)).to_rfc3339();
        let security_cutoff =
            (now - chrono::Duration::days(Self::SECURITY_LOG_RETENTION_DAYS)).to_rfc3339();
        let approval_cutoff =
            (now - chrono::Duration::days(Self::APPROVAL_LOG_RETENTION_DAYS)).to_rfc3339();
        let delegation_cutoff =
            (now - chrono::Duration::days(Self::SWARM_DELEGATION_RETENTION_DAYS)).to_rfc3339();
        let llm_usage_cutoff =
            (now - chrono::Duration::days(Self::LLM_USAGE_RETENTION_DAYS)).to_rfc3339();
        let terminal_task_cutoff =
            (now - chrono::Duration::days(Self::TERMINAL_TASK_RETENTION_DAYS)).to_rfc3339();
        let message_cutoff =
            (now - chrono::Duration::days(Self::MESSAGE_RETENTION_DAYS)).to_rfc3339();

        let txn = self.db.begin().await?;
        let message_delete = message::Entity::delete_many()
            .filter(message::Column::Timestamp.lt(message_cutoff.clone()))
            .exec(&txn)
            .await?;
        if message_delete.rows_affected > 0 {
            txn.execute(Statement::from_string(
                DbBackend::Sqlite,
                "UPDATE conversations SET message_count = (SELECT COUNT(*) FROM messages WHERE messages.conversation_id = conversations.id);".to_string(),
            ))
            .await?;
            conversation::Entity::delete_many()
                .filter(conversation::Column::MessageCount.eq(0))
                .filter(conversation::Column::UpdatedAt.lt(message_cutoff))
                .exec(&txn)
                .await?;
        }
        crate::storage::entities::execution_trace::Entity::delete_many()
            .filter(crate::storage::entities::execution_trace::Column::CreatedAt.lt(trace_cutoff))
            .exec(&txn)
            .await?;
        crate::storage::entities::execution_proof::Entity::delete_many()
            .filter(crate::storage::entities::execution_proof::Column::Timestamp.lt(proof_cutoff))
            .exec(&txn)
            .await?;
        operational_log::Entity::delete_many()
            .filter(operational_log::Column::CreatedAt.lt(operational_cutoff))
            .exec(&txn)
            .await?;
        security_log::Entity::delete_many()
            .filter(security_log::Column::CreatedAt.lt(security_cutoff))
            .exec(&txn)
            .await?;
        approval_log::Entity::delete_many()
            .filter(approval_log::Column::RequestedAt.lt(approval_cutoff))
            .filter(approval_log::Column::Status.ne("pending"))
            .exec(&txn)
            .await?;
        swarm_delegation::Entity::delete_many()
            .filter(swarm_delegation::Column::CreatedAt.lt(delegation_cutoff))
            .exec(&txn)
            .await?;
        llm_usage::Entity::delete_many()
            .filter(llm_usage::Column::CreatedAt.lt(llm_usage_cutoff))
            .exec(&txn)
            .await?;

        let stale_tasks = task::Entity::find()
            .filter(task::Column::CreatedAt.lt(terminal_task_cutoff))
            .all(&txn)
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
            task::Entity::delete_by_id(stale_task.id).exec(&txn).await?;
        }
        txn.commit().await?;

        self.set(
            Self::HOUSEKEEPING_PURGE_LAST_RUN_KEY,
            now.to_rfc3339().as_bytes(),
        )
        .await?;
        Ok(())
    }

    /// Run SQLite quick integrity check.
    pub async fn sqlite_quick_check(&self) -> Result<String> {
        let row = self
            .db
            .query_one(Statement::from_string(
                DbBackend::Sqlite,
                "PRAGMA quick_check(1);".to_string(),
            ))
            .await?;

        if let Some(row) = row {
            let result: String = row
                .try_get("", "quick_check")
                .unwrap_or_else(|_| "unknown".to_string());
            Ok(result)
        } else {
            Ok("unknown".to_string())
        }
    }

    /// List SQLite table names (excluding internal sqlite_* tables).
    pub async fn sqlite_table_names(&self) -> Result<Vec<String>> {
        let rows = self
            .db
            .query_all(Statement::from_string(
                DbBackend::Sqlite,
                "SELECT name FROM sqlite_master WHERE type = 'table' ORDER BY name;".to_string(),
            ))
            .await?;

        let mut names = Vec::new();
        for row in rows {
            if let Ok(name) = row.try_get::<String>("", "name") {
                if !name.starts_with("sqlite_") {
                    names.push(name);
                }
            }
        }
        Ok(names)
    }
}
