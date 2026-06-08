use super::super::*;

impl Storage {
    pub(super) const DATABASE_MAX_INTEGER: u64 = i64::MAX as u64;
    pub(super) const HOUSEKEEPING_PURGE_LAST_RUN_KEY: &'static str =
        "storage_housekeeping_last_purge_v1";
    pub(super) const UPLOAD_MANIFEST_KEY_PREFIX: &'static str = "upload_manifest:";
    pub(super) const MAX_DOCUMENTS_FOR_SEARCH: u64 = 5_000;
    pub(super) const MAX_DOCUMENT_CHUNKS_FOR_SEARCH: u64 = 20_000;
    pub(super) const MAX_LLM_USAGE_ROWS_PER_QUERY: u64 = 5_000;
    pub(super) const MAX_LLM_USAGE_ANALYTICS_ROWS: usize = 250_000;
    pub(super) const MAX_OPERATIONAL_LOG_ROWS_PER_QUERY: u64 = 5_000;
    pub(super) const MAX_OPERATIONAL_LOG_ANALYTICS_ROWS: usize = 250_000;
    pub(super) const MAX_FACT_ROWS_PER_QUERY: u64 = 5_000;
    pub(super) const MAX_TASK_ROWS_PER_QUERY: u64 = 5_000;
    pub(super) const MAX_EXPENSE_ROWS_PER_QUERY: u64 = 5_000;
    pub(super) const MAX_SWARM_DELEGATION_ROWS_PER_QUERY: u64 = 5_000;
    pub(super) const MAX_EXPERIENCE_RUN_ROWS_PER_QUERY: u64 = 1_000;
    pub(super) const MAX_EXPERIENCE_ITEM_ROWS_PER_QUERY: u64 = 2_000;
    pub(super) const MAX_PROCEDURAL_PATTERN_ROWS_PER_QUERY: u64 = 2_000;
    #[allow(dead_code)]
    pub(super) const MAX_RELATED_EXPERIENCE_EDGE_ROWS_PER_QUERY: u64 = 5_000;
    pub(super) const SENSITIVE_PAYLOAD_BACKFILL_MARKER_KEY: &'static str =
        "storage_sensitive_payload_backfill_v4";

    #[inline]
    pub(super) fn db_limit(limit: u64) -> u64 {
        limit.min(Self::DATABASE_MAX_INTEGER)
    }

    #[inline]
    pub(super) fn db_offset(offset: u64) -> u64 {
        offset.min(Self::DATABASE_MAX_INTEGER)
    }

    pub(super) fn upload_manifest_key(id: &str) -> String {
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

    pub(super) fn preference_row_id(key: &str, project_id: Option<&str>) -> String {
        let normalized_key = key.trim().to_ascii_lowercase();
        let scope = project_id
            .map(|p| p.trim())
            .filter(|p| !p.is_empty())
            .unwrap_or("_global");
        format!("{}::{}", scope, normalized_key)
    }

    pub(super) fn default_link_title(url: &str) -> String {
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
                    crate::core::runtime::data_lifecycle::load_data_lifecycle_settings(&storage)
                        .await;
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
}
