//! Database storage using SeaORM

pub mod encrypted;
pub mod entities;

use anyhow::Result;
#[allow(unused_imports)]
use sea_orm::{
    ActiveModelTrait, ColumnTrait, Condition, ConnectionTrait, Database, DatabaseConnection,
    DbBackend, EntityTrait, PaginatorTrait, QueryFilter, QueryOrder, QuerySelect, Schema, Set,
    Statement, TransactionTrait, TryGetable,
};
use std::path::Path;

pub use entities::*;

/// Database storage using SeaORM
#[derive(Clone)]
pub struct Storage {
    db: DatabaseConnection,
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

        // Create tables if they don't exist
        Self::create_tables(&db).await?;

        Ok(Self { db })
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
                proof_id TEXT,
                priority REAL,
                urgency REAL,
                importance REAL,
                eisenhower_quadrant INTEGER
            );

            CREATE TABLE IF NOT EXISTS swarm_agents (
                id TEXT PRIMARY KEY,
                name TEXT NOT NULL,
                agent_type TEXT NOT NULL,
                llm_provider TEXT NOT NULL,
                capabilities TEXT NOT NULL,
                system_prompt TEXT,
                enabled INTEGER DEFAULT 1,
                created_at TEXT NOT NULL
            );

            CREATE TABLE IF NOT EXISTS swarm_delegations (
                id TEXT PRIMARY KEY,
                parent_task_id TEXT,
                agent_id TEXT NOT NULL,
                task_description TEXT NOT NULL,
                result TEXT,
                success INTEGER DEFAULT 0,
                confidence REAL,
                execution_time_ms INTEGER,
                created_at TEXT NOT NULL,
                completed_at TEXT
            );

            CREATE TABLE IF NOT EXISTS conversations (
                id TEXT PRIMARY KEY,
                title TEXT NOT NULL,
                channel TEXT NOT NULL,
                project_id TEXT,
                created_at TEXT NOT NULL,
                updated_at TEXT NOT NULL,
                message_count INTEGER DEFAULT 0,
                archived INTEGER DEFAULT 0
            );

            CREATE TABLE IF NOT EXISTS messages (
                id TEXT PRIMARY KEY,
                conversation_id TEXT NOT NULL,
                role TEXT NOT NULL,
                content TEXT NOT NULL,
                timestamp TEXT NOT NULL,
                model_used TEXT,
                trace_id TEXT
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
                updated_at TEXT NOT NULL
            );

            CREATE TABLE IF NOT EXISTS documents (
                id TEXT PRIMARY KEY,
                filename TEXT NOT NULL,
                content_type TEXT NOT NULL,
                project_id TEXT,
                chunk_count INTEGER DEFAULT 0,
                file_size INTEGER DEFAULT 0,
                created_at TEXT NOT NULL
            );

            CREATE TABLE IF NOT EXISTS document_chunks (
                id TEXT PRIMARY KEY,
                document_id TEXT NOT NULL,
                chunk_index INTEGER NOT NULL,
                content TEXT NOT NULL,
                embedding BLOB
            );

            CREATE TABLE IF NOT EXISTS notifications (
                id TEXT PRIMARY KEY,
                title TEXT NOT NULL,
                body TEXT NOT NULL,
                level TEXT NOT NULL DEFAULT 'info',
                source TEXT NOT NULL DEFAULT '',
                read INTEGER DEFAULT 0,
                created_at TEXT NOT NULL
            );

            CREATE TABLE IF NOT EXISTS approval_log (
                id TEXT PRIMARY KEY,
                action_name TEXT NOT NULL,
                arguments TEXT NOT NULL,
                rule_name TEXT NOT NULL,
                status TEXT NOT NULL DEFAULT 'pending',
                requested_at TEXT NOT NULL,
                resolved_at TEXT,
                resolved_by TEXT
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
                trace_id TEXT,
                conversation_id TEXT,
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
                model_slot TEXT
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
                estimated INTEGER NOT NULL DEFAULT 1
            );

            CREATE TABLE IF NOT EXISTS user_preferences (
                id TEXT PRIMARY KEY,
                key TEXT NOT NULL,
                value TEXT NOT NULL,
                confidence REAL NOT NULL DEFAULT 0.8,
                source TEXT,
                project_id TEXT,
                created_at TEXT NOT NULL,
                updated_at TEXT NOT NULL
            );

            CREATE TABLE IF NOT EXISTS user_data_items (
                id TEXT PRIMARY KEY,
                kind TEXT NOT NULL,
                title TEXT NOT NULL,
                content TEXT NOT NULL,
                url TEXT,
                source_channel TEXT,
                conversation_id TEXT,
                project_id TEXT,
                pinned INTEGER NOT NULL DEFAULT 0,
                created_at TEXT NOT NULL,
                updated_at TEXT NOT NULL
            );

            CREATE TABLE IF NOT EXISTS knowledge_items (
                id TEXT PRIMARY KEY,
                title TEXT NOT NULL,
                content TEXT NOT NULL,
                source TEXT,
                url TEXT,
                tags TEXT,
                project_id TEXT,
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
            CREATE INDEX IF NOT EXISTS idx_episodes_project ON episodes(context);
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
            CREATE INDEX IF NOT EXISTS idx_user_data_url ON user_data_items(url);
            CREATE INDEX IF NOT EXISTS idx_user_data_project ON user_data_items(project_id);
            CREATE INDEX IF NOT EXISTS idx_user_data_updated ON user_data_items(updated_at);
            CREATE INDEX IF NOT EXISTS idx_knowledge_project ON knowledge_items(project_id);
            CREATE INDEX IF NOT EXISTS idx_knowledge_updated ON knowledge_items(updated_at);
            "#,
        )
        .await?;

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

        // Use raw SQL to increment access_count atomically
        self.db.execute_unprepared(&format!(
            "UPDATE episodes SET last_accessed = '{}', access_count = access_count + 1 WHERE id = '{}'",
            now, id
        )).await?;

        Ok(())
    }

    /// Get all episodes with their metadata for scoring
    pub async fn get_all_episodes_for_scoring(&self) -> Result<Vec<episode::Model>> {
        let episodes = episode::Entity::find()
            .order_by_desc(episode::Column::Timestamp)
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
        let episodes = query.all(&self.db).await?;
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
            model.value = Set(value.to_string());
            model.confidence = Set(bounded_confidence);
            model.source = Set(source.map(|s| s.to_string()));
            model.project_id = Set(normalized_project);
            model.updated_at = Set(now);
            Ok(model.update(&self.db).await?)
        } else {
            let model = user_preference::ActiveModel {
                id: Set(id),
                key: Set(key.to_ascii_lowercase()),
                value: Set(value.to_string()),
                confidence: Set(bounded_confidence),
                source: Set(source.map(|s| s.to_string())),
                project_id: Set(normalized_project),
                created_at: Set(now.clone()),
                updated_at: Set(now),
            }
            .insert(&self.db)
            .await?;
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
        let rows = query.limit(limit).offset(offset).all(&self.db).await?;
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
        let model = user_data_item::ActiveModel {
            id: Set(uuid::Uuid::new_v4().to_string()),
            kind: Set(item.kind.trim().to_string()),
            title: Set(item.title.trim().to_string()),
            content: Set(item.content.to_string()),
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
            Ok(model.update(&self.db).await?)
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
        let rows = query.limit(limit).offset(offset).all(&self.db).await?;
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
        let model = knowledge_item::ActiveModel {
            id: Set(uuid::Uuid::new_v4().to_string()),
            title: Set(title.trim().to_string()),
            content: Set(content.to_string()),
            source: Set(source.map(|v| v.to_string())),
            url: Set(url.map(|v| v.to_string())),
            tags: Set(tags.map(|v| v.to_string())),
            project_id: Set(project_id.map(|v| v.to_string())),
            created_at: Set(now.clone()),
            updated_at: Set(now),
        }
        .insert(&self.db)
        .await?;
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
        let rows = query.limit(limit).offset(offset).all(&self.db).await?;
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
        task::ActiveModel {
            id: Set(task.id.to_string()),
            description: Set(task.description.clone()),
            action: Set(task.action.clone()),
            arguments: Set(serde_json::to_string(&task.arguments)?),
            approval: Set(serde_json::to_string(&task.approval)?),
            status: Set(serde_json::to_string(&task.status)?),
            created_at: Set(task.created_at.to_rfc3339()),
            scheduled_for: Set(task.scheduled_for.map(|t| t.to_rfc3339())),
            cron: Set(task.cron.clone()),
            result: Set(task.result.clone()),
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
            model.description = Set(desc);
        }
        if let Some(args) = arguments {
            model.arguments = Set(args);
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
            model.result = Set(Some(res.to_string()));
        }
        model.update(&self.db).await?;
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
        let tasks = task::Entity::find().all(&self.db).await?;
        Ok(tasks)
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
        let delegations = swarm_delegation::Entity::find()
            .order_by_desc(swarm_delegation::Column::CreatedAt)
            .limit(limit)
            .all(&self.db)
            .await?;
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
            task_description: Set(delegation.task_description.clone()),
            result: Set(delegation.result.clone()),
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
        // Delete messages first
        self.db
            .execute_unprepared(&format!(
                "DELETE FROM messages WHERE conversation_id = '{}'",
                id
            ))
            .await?;
        conversation::Entity::delete_by_id(id.to_string())
            .exec(&self.db)
            .await?;
        Ok(())
    }

    // ==================== Messages ====================

    /// Insert a message
    pub async fn insert_message(&self, msg: &message::Model) -> Result<()> {
        message::ActiveModel {
            id: Set(msg.id.clone()),
            conversation_id: Set(msg.conversation_id.clone()),
            role: Set(msg.role.clone()),
            content: Set(msg.content.clone()),
            timestamp: Set(msg.timestamp.clone()),
            model_used: Set(msg.model_used.clone()),
            trace_id: Set(msg.trace_id.clone()),
        }
        .insert(&self.db)
        .await?;

        // Update conversation message count and updated_at
        let now = chrono::Utc::now().to_rfc3339();
        self.db.execute_unprepared(&format!(
            "UPDATE conversations SET message_count = message_count + 1, updated_at = '{}' WHERE id = '{}'",
            now, msg.conversation_id
        )).await?;

        Ok(())
    }

    /// Get messages for a conversation
    pub async fn get_messages(
        &self,
        conversation_id: &str,
        limit: u64,
        offset: u64,
    ) -> Result<Vec<message::Model>> {
        let msgs = message::Entity::find()
            .filter(message::Column::ConversationId.eq(conversation_id))
            .order_by_asc(message::Column::Timestamp)
            .limit(limit)
            .offset(offset)
            .all(&self.db)
            .await?;
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
        document::ActiveModel {
            id: Set(doc.id.clone()),
            filename: Set(doc.filename.clone()),
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
        document_chunk::ActiveModel {
            id: Set(chunk.id.clone()),
            document_id: Set(chunk.document_id.clone()),
            chunk_index: Set(chunk.chunk_index),
            content: Set(chunk.content.clone()),
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
        let docs = query.limit(limit).offset(offset).all(&self.db).await?;
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
        let chunks = document_chunk::Entity::find()
            .filter(document_chunk::Column::DocumentId.eq(document_id))
            .order_by_asc(document_chunk::Column::ChunkIndex)
            .all(&self.db)
            .await?;
        Ok(chunks)
    }

    /// Get all chunks (across all documents, for search)
    pub async fn get_all_document_chunks(&self) -> Result<Vec<document_chunk::Model>> {
        let chunks = document_chunk::Entity::find().all(&self.db).await?;
        Ok(chunks)
    }

    /// Delete a document and its chunks
    pub async fn delete_document(&self, id: &str) -> Result<()> {
        self.db
            .execute_unprepared(&format!(
                "DELETE FROM document_chunks WHERE document_id = '{}'",
                id
            ))
            .await?;
        document::Entity::delete_by_id(id.to_string())
            .exec(&self.db)
            .await?;
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
        let cutoff = chrono::Utc::now() - chrono::Duration::hours(Self::ARKPULSE_NOTIFICATION_WINDOW_HOURS);
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
                .filter(notification::Column::Title.eq(title_clean.clone()))
                .order_by_desc(notification::Column::CreatedAt)
                .limit(50)
                .all(&self.db)
                .await
            {
                Ok(recent) => {
                    for existing in recent {
                        if Self::notification_body_signature(&existing.body) == sig {
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
            title: Set(title_clean),
            body: Set(body_clean),
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
        let notifs = query.limit(limit).offset(offset).all(&self.db).await?;
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
        let text_filter = Condition::any()
            .add(notification::Column::Title.contains(trimmed))
            .add(notification::Column::Body.contains(trimmed));

        let result = notification::Entity::delete_many()
            .filter(source_filter)
            .filter(text_filter)
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

        let mut text_filter = Condition::any()
            .add(notification::Column::Title.contains(id_trimmed))
            .add(notification::Column::Body.contains(id_trimmed));
        if let Some(title) = app_title.map(str::trim).filter(|s| !s.is_empty()) {
            text_filter = text_filter
                .add(notification::Column::Title.contains(title))
                .add(notification::Column::Body.contains(title));
        }

        let result = notification::Entity::delete_many()
            .filter(source_filter)
            .filter(text_filter)
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
        let arkpulse_recent_unread = self.count_recent_arkpulse_notifications(true).await.unwrap_or(0);
        if arkpulse_recent_unread > 1 {
            Ok(count.saturating_sub(arkpulse_recent_unread - 1))
        } else {
            Ok(count)
        }
    }

    /// Mark episodes as consolidated
    pub async fn mark_episodes_consolidated(&self, ids: &[String]) -> Result<()> {
        for id in ids {
            self.db
                .execute_unprepared(&format!(
                    "UPDATE episodes SET consolidated = 1 WHERE id = '{}'",
                    id
                ))
                .await?;
        }
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
        let log = approval_log::Entity::find()
            .order_by_desc(approval_log::Column::RequestedAt)
            .limit(limit)
            .offset(offset)
            .all(&self.db)
            .await?;
        Ok(log)
    }

    // ==================== Security Logs ====================

    /// Insert a security log entry
    pub async fn insert_security_log(&self, log: &security_log::Model) -> Result<()> {
        security_log::ActiveModel {
            id: Set(log.id.clone()),
            event_type: Set(log.event_type.clone()),
            severity: Set(log.severity.clone()),
            message: Set(log.message.clone()),
            source: Set(log.source.clone()),
            count: Set(log.count),
            created_at: Set(log.created_at.clone()),
        }
        .insert(&self.db)
        .await?;
        Ok(())
    }

    /// List recent security logs (newest first)
    pub async fn list_security_logs(&self, limit: u64) -> Result<Vec<security_log::Model>> {
        let logs = security_log::Entity::find()
            .order_by_desc(security_log::Column::CreatedAt)
            .limit(limit)
            .all(&self.db)
            .await?;
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

        let logs = query.limit(limit).offset(offset).all(&self.db).await?;
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
            outcome: Set(log.outcome.clone()),
            tool_name: Set(log.tool_name.clone()),
            latency_ms: Set(log.latency_ms),
            arguments: Set(log.arguments.clone()),
            payload: Set(log.payload.clone()),
            strategy_version: Set(log.strategy_version.clone()),
            policy_version: Set(log.policy_version.clone()),
            prompt_version: Set(log.prompt_version.clone()),
            model_slot: Set(log.model_slot.clone()),
        }
        .insert(&self.db)
        .await?;
        Ok(())
    }

    /// List operational logs by event type (newest first).
    pub async fn list_operational_logs_by_event(
        &self,
        event_type: &str,
        limit: u64,
    ) -> Result<Vec<operational_log::Model>> {
        let rows = operational_log::Entity::find()
            .filter(operational_log::Column::EventType.eq(event_type.to_string()))
            .order_by_desc(operational_log::Column::CreatedAt)
            .limit(limit)
            .all(&self.db)
            .await?;
        Ok(rows)
    }

    /// Expire old pending approvals (older than max_age_secs)
    pub async fn expire_old_approvals(&self, max_age_secs: i64) -> Result<u64> {
        let cutoff = (chrono::Utc::now() - chrono::Duration::seconds(max_age_secs)).to_rfc3339();
        let result = self.db.execute_unprepared(&format!(
            "UPDATE approval_log SET status = 'expired', resolved_at = '{}', resolved_by = 'auto_timeout' WHERE status = 'pending' AND requested_at < '{}'",
            chrono::Utc::now().to_rfc3339(), cutoff
        )).await?;
        Ok(result.rows_affected())
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
