use anyhow::Result;
use sea_orm::sea_query::{
    extension::postgres::Extension, Index, IndexCreateStatement, PostgresQueryBuilder,
};
use sea_orm::{ConnectionTrait, DatabaseConnection, DbBackend, EntityTrait, Schema, Statement};

use super::entities::*;

const CURRENT_SCHEMA_VERSION: i64 = 1;

pub fn latest_version() -> i64 {
    CURRENT_SCHEMA_VERSION
}

async fn ensure_table<E: EntityTrait>(
    db: &DatabaseConnection,
    backend: DbBackend,
    schema: &Schema,
    entity: E,
) -> Result<()> {
    let statement = schema
        .create_table_from_entity(entity)
        .if_not_exists()
        .to_owned();
    db.execute(backend.build(&statement)).await?;
    Ok(())
}

async fn ensure_index(
    db: &DatabaseConnection,
    backend: DbBackend,
    statement: IndexCreateStatement,
) -> Result<()> {
    db.execute(backend.build(&statement)).await?;
    Ok(())
}

async fn ensure_optional_sql(
    db: &DatabaseConnection,
    backend: DbBackend,
    sql: impl Into<String>,
    description: &str,
) -> Result<()> {
    if let Err(error) = db
        .execute(Statement::from_string(backend, sql.into()))
        .await
    {
        tracing::warn!("Skipping optional {}: {}", description, error);
    }
    Ok(())
}

const ACTION_CATALOG_INDEX_HNSW_SQL: &str =
    "CREATE INDEX IF NOT EXISTS idx_action_catalog_index_embedding_hnsw \
     ON action_catalog_index USING hnsw (embedding vector_cosine_ops) \
     WHERE enabled = true AND embedding IS NOT NULL";

fn sql_string_literal(value: &str) -> String {
    format!("'{}'", value.replace('\'', "''"))
}

fn sql_identifier(value: &str) -> String {
    format!("\"{}\"", value.replace('"', "\"\""))
}

fn vector_type_has_dimensions(formatted_type: &str) -> bool {
    let normalized = formatted_type.trim().to_ascii_lowercase();
    normalized.starts_with("vector(") || normalized.contains(".vector(")
}

async fn pgvector_column_has_dimensions(
    db: &DatabaseConnection,
    table: &str,
    column: &str,
) -> Result<bool> {
    let sql = format!(
        "SELECT format_type(a.atttypid, a.atttypmod) AS formatted_type \
         FROM pg_attribute a \
         JOIN pg_class c ON c.oid = a.attrelid \
         JOIN pg_namespace n ON n.oid = c.relnamespace \
         WHERE n.nspname = current_schema() \
           AND c.relname = {} \
           AND a.attname = {} \
           AND a.attnum > 0 \
           AND NOT a.attisdropped",
        sql_string_literal(table),
        sql_string_literal(column),
    );
    let row = db
        .query_one(Statement::from_string(DbBackend::Postgres, sql))
        .await?;
    let Some(row) = row else {
        return Ok(false);
    };
    let formatted_type: String = row.try_get("", "formatted_type")?;
    Ok(vector_type_has_dimensions(&formatted_type))
}

async fn ensure_pgvector_hnsw_index(
    db: &DatabaseConnection,
    backend: DbBackend,
    table: &str,
    column: &str,
    sql: impl Into<String>,
    description: &str,
) -> Result<()> {
    if !pgvector_column_has_dimensions(db, table, column).await? {
        tracing::info!(
            "Skipping optional {} because {}.{} uses an unconstrained vector type; HNSW indexes require vector(n)",
            description,
            table,
            column
        );
        return Ok(());
    }
    ensure_optional_sql(db, backend, sql, description).await
}

async fn ensure_foreign_key(
    db: &DatabaseConnection,
    backend: DbBackend,
    constraint_name: &str,
    table: &str,
    column: &str,
    referenced_table: &str,
    referenced_column: &str,
    on_delete: &str,
) -> Result<()> {
    let sql = format!(
        "DO $$ BEGIN \
         IF NOT EXISTS (SELECT 1 FROM pg_constraint WHERE conname = {constraint_name}) THEN \
             ALTER TABLE {table} \
             ADD CONSTRAINT {constraint} \
             FOREIGN KEY ({column}) REFERENCES {referenced_table} ({referenced_column}) \
             ON DELETE {on_delete}; \
         END IF; \
         END $$;",
        constraint_name = sql_string_literal(constraint_name),
        table = sql_identifier(table),
        constraint = sql_identifier(constraint_name),
        column = sql_identifier(column),
        referenced_table = sql_identifier(referenced_table),
        referenced_column = sql_identifier(referenced_column),
        on_delete = on_delete,
    );
    db.execute(Statement::from_string(backend, sql)).await?;
    Ok(())
}

async fn ensure_action_catalog_index_table(
    db: &DatabaseConnection,
    backend: DbBackend,
) -> Result<()> {
    let sql = format!(
        "CREATE TABLE IF NOT EXISTS action_catalog_index (\
         action_name TEXT PRIMARY KEY,\
         source TEXT NOT NULL,\
         version TEXT NOT NULL,\
         descriptor_hash TEXT NOT NULL,\
         descriptor_text TEXT NOT NULL,\
         enabled BOOLEAN NOT NULL DEFAULT true,\
         metadata_json JSONB NOT NULL DEFAULT '{{}}'::jsonb,\
         embedding vector({}),\
         updated_at TEXT NOT NULL\
         )",
        crate::actions::ACTION_CATALOG_EMBEDDING_DIM
    );
    db.execute(Statement::from_string(backend, sql)).await?;
    Ok(())
}

macro_rules! ensure_table_list {
    ($db:expr, $backend:expr, $schema:expr, [$($entity:path),+ $(,)?]) => {
        $(
            ensure_table($db, $backend, $schema, $entity).await?;
        )+
    };
}

pub async fn run(db: &DatabaseConnection) -> Result<()> {
    if db.get_database_backend() != DbBackend::Postgres {
        anyhow::bail!("storage bootstrap requires a postgres database backend");
    }

    let backend = db.get_database_backend();
    let schema = Schema::new(backend);

    let vector_extension = Extension::create()
        .name("vector")
        .if_not_exists()
        .to_owned();
    db.execute(Statement::from_string(
        backend,
        vector_extension.to_string(PostgresQueryBuilder),
    ))
    .await?;

    ensure_action_catalog_index_table(db, backend).await?;

    ensure_table_list!(
        db,
        backend,
        &schema,
        [
            kv_store::Entity,
            arkpulse_event::Entity,
            background_session::Entity,
            browser_session::Entity,
            action::Entity,
            execution_proof::Entity,
            execution_trace::Entity,
            project::Entity,
            task::Entity,
            swarm_agent::Entity,
            swarm_delegation::Entity,
            conversation::Entity,
            message::Entity,
            document::Entity,
            document_chunk::Entity,
            notification::Entity,
            approval_log::Entity,
            automation_run::Entity,
            automation_supervisor_state::Entity,
            watcher::Entity,
            expense::Entity,
            security_log::Entity,
            operational_log::Entity,
            llm_usage::Entity,
            user_preference::Entity,
            user_data_item::Entity,
            knowledge_item::Entity,
            execution_run::Entity,
            run_checkpoint::Entity,
            tool_attempt::Entity,
            experience_run::Entity,
            experience_item::Entity,
            experience_edge::Entity,
            procedural_pattern::Entity,
            learning_candidate::Entity,
            readiness_evaluation::Entity,
            memory_capture_event::Entity,
            memory_operation::Entity,
            memory_evidence_link::Entity,
            recall_event::Entity,
            recall_test::Entity,
            abuse_tracker_state::Entity
        ]
    );

    ensure_optional_sql(
        db,
        backend,
        "ALTER TABLE swarm_agents ADD COLUMN IF NOT EXISTS access_scope TEXT NOT NULL DEFAULT '{}'",
        "swarm_agents.access_scope column",
    )
    .await?;
    for (constraint_name, table, column, referenced_table, referenced_column, on_delete) in [
        (
            "fk_conversations_project_id",
            "conversations",
            "project_id",
            "projects",
            "id",
            "CASCADE",
        ),
        (
            "fk_messages_conversation_id",
            "messages",
            "conversation_id",
            "conversations",
            "id",
            "CASCADE",
        ),
        (
            "fk_messages_trace_id",
            "messages",
            "trace_id",
            "execution_traces",
            "id",
            "SET NULL",
        ),
        (
            "fk_documents_project_id",
            "documents",
            "project_id",
            "projects",
            "id",
            "CASCADE",
        ),
        (
            "fk_document_chunks_document_id",
            "document_chunks",
            "document_id",
            "documents",
            "id",
            "CASCADE",
        ),
        (
            "fk_background_sessions_conversation_id",
            "background_sessions",
            "conversation_id",
            "conversations",
            "id",
            "CASCADE",
        ),
        (
            "fk_background_sessions_project_id",
            "background_sessions",
            "project_id",
            "projects",
            "id",
            "CASCADE",
        ),
        (
            "fk_execution_traces_proof_id",
            "execution_traces",
            "proof_id",
            "execution_proofs",
            "id",
            "SET NULL",
        ),
        (
            "fk_execution_runs_trace_id",
            "execution_runs",
            "trace_id",
            "execution_traces",
            "id",
            "SET NULL",
        ),
        (
            "fk_execution_runs_conversation_id",
            "execution_runs",
            "conversation_id",
            "conversations",
            "id",
            "CASCADE",
        ),
        (
            "fk_run_checkpoints_run_id",
            "run_checkpoints",
            "run_id",
            "execution_runs",
            "id",
            "CASCADE",
        ),
        (
            "fk_tool_attempts_run_id",
            "tool_attempts",
            "run_id",
            "execution_runs",
            "id",
            "CASCADE",
        ),
        (
            "fk_operational_logs_trace_id",
            "operational_logs",
            "trace_id",
            "execution_traces",
            "id",
            "SET NULL",
        ),
        (
            "fk_operational_logs_conversation_id",
            "operational_logs",
            "conversation_id",
            "conversations",
            "id",
            "CASCADE",
        ),
        (
            "fk_swarm_delegations_parent_task_id",
            "swarm_delegations",
            "parent_task_id",
            "tasks",
            "id",
            "SET NULL",
        ),
        (
            "fk_swarm_delegations_agent_id",
            "swarm_delegations",
            "agent_id",
            "swarm_agents",
            "id",
            "CASCADE",
        ),
        (
            "fk_tasks_proof_id",
            "tasks",
            "proof_id",
            "execution_proofs",
            "id",
            "SET NULL",
        ),
        (
            "fk_tasks_last_run_id",
            "tasks",
            "last_run_id",
            "automation_runs",
            "id",
            "SET NULL",
        ),
        (
            "fk_watchers_last_run_id",
            "watchers",
            "last_run_id",
            "automation_runs",
            "id",
            "SET NULL",
        ),
        (
            "fk_automation_supervisor_states_last_run_id",
            "automation_supervisor_states",
            "last_run_id",
            "automation_runs",
            "id",
            "SET NULL",
        ),
        (
            "fk_experience_runs_execution_run_id",
            "experience_runs",
            "execution_run_id",
            "execution_runs",
            "id",
            "SET NULL",
        ),
        (
            "fk_experience_runs_trace_id",
            "experience_runs",
            "trace_id",
            "execution_traces",
            "id",
            "SET NULL",
        ),
        (
            "fk_experience_runs_conversation_id",
            "experience_runs",
            "conversation_id",
            "conversations",
            "id",
            "CASCADE",
        ),
        (
            "fk_experience_runs_project_id",
            "experience_runs",
            "project_id",
            "projects",
            "id",
            "CASCADE",
        ),
        (
            "fk_experience_items_conversation_id",
            "experience_items",
            "conversation_id",
            "conversations",
            "id",
            "CASCADE",
        ),
        (
            "fk_experience_items_project_id",
            "experience_items",
            "project_id",
            "projects",
            "id",
            "CASCADE",
        ),
        (
            "fk_experience_edges_source_run_id",
            "experience_edges",
            "source_run_id",
            "experience_runs",
            "id",
            "SET NULL",
        ),
        (
            "fk_procedural_patterns_conversation_id",
            "procedural_patterns",
            "conversation_id",
            "conversations",
            "id",
            "CASCADE",
        ),
        (
            "fk_procedural_patterns_project_id",
            "procedural_patterns",
            "project_id",
            "projects",
            "id",
            "CASCADE",
        ),
        (
            "fk_learning_candidates_conversation_id",
            "learning_candidates",
            "conversation_id",
            "conversations",
            "id",
            "CASCADE",
        ),
        (
            "fk_learning_candidates_project_id",
            "learning_candidates",
            "project_id",
            "projects",
            "id",
            "CASCADE",
        ),
        (
            "fk_learning_candidates_pattern_id",
            "learning_candidates",
            "pattern_id",
            "procedural_patterns",
            "id",
            "SET NULL",
        ),
        (
            "fk_user_data_items_conversation_id",
            "user_data_items",
            "conversation_id",
            "conversations",
            "id",
            "SET NULL",
        ),
        (
            "fk_user_data_items_project_id",
            "user_data_items",
            "project_id",
            "projects",
            "id",
            "CASCADE",
        ),
        (
            "fk_knowledge_items_project_id",
            "knowledge_items",
            "project_id",
            "projects",
            "id",
            "CASCADE",
        ),
        (
            "fk_user_preferences_project_id",
            "user_preferences",
            "project_id",
            "projects",
            "id",
            "CASCADE",
        ),
    ] {
        ensure_foreign_key(
            db,
            backend,
            constraint_name,
            table,
            column,
            referenced_table,
            referenced_column,
            on_delete,
        )
        .await?;
    }

    ensure_index(
        db,
        backend,
        Index::create()
            .name("idx_arkpulse_events_timestamp")
            .table(arkpulse_event::Entity)
            .col(arkpulse_event::Column::Timestamp)
            .if_not_exists()
            .to_owned(),
    )
    .await?;
    ensure_index(
        db,
        backend,
        Index::create()
            .name("idx_background_sessions_updated")
            .table(background_session::Entity)
            .col(background_session::Column::UpdatedAt)
            .if_not_exists()
            .to_owned(),
    )
    .await?;
    ensure_index(
        db,
        backend,
        Index::create()
            .name("idx_background_sessions_status_updated")
            .table(background_session::Entity)
            .col(background_session::Column::Status)
            .col(background_session::Column::UpdatedAt)
            .if_not_exists()
            .to_owned(),
    )
    .await?;
    ensure_index(
        db,
        backend,
        Index::create()
            .name("idx_background_sessions_conversation_updated")
            .table(background_session::Entity)
            .col(background_session::Column::ConversationId)
            .col(background_session::Column::UpdatedAt)
            .if_not_exists()
            .to_owned(),
    )
    .await?;
    ensure_index(
        db,
        backend,
        Index::create()
            .name("idx_browser_sessions_updated")
            .table(browser_session::Entity)
            .col(browser_session::Column::UpdatedAt)
            .if_not_exists()
            .to_owned(),
    )
    .await?;
    ensure_index(
        db,
        backend,
        Index::create()
            .name("idx_browser_sessions_status_updated")
            .table(browser_session::Entity)
            .col(browser_session::Column::Status)
            .col(browser_session::Column::UpdatedAt)
            .if_not_exists()
            .to_owned(),
    )
    .await?;
    ensure_index(
        db,
        backend,
        Index::create()
            .name("idx_proofs_timestamp")
            .table(execution_proof::Entity)
            .col(execution_proof::Column::Timestamp)
            .if_not_exists()
            .to_owned(),
    )
    .await?;
    ensure_index(
        db,
        backend,
        Index::create()
            .name("idx_execution_traces_created")
            .table(execution_trace::Entity)
            .col(execution_trace::Column::CreatedAt)
            .if_not_exists()
            .to_owned(),
    )
    .await?;
    ensure_index(
        db,
        backend,
        Index::create()
            .name("idx_execution_traces_started")
            .table(execution_trace::Entity)
            .col(execution_trace::Column::StartedAt)
            .if_not_exists()
            .to_owned(),
    )
    .await?;
    ensure_index(
        db,
        backend,
        Index::create()
            .name("idx_tasks_status")
            .table(task::Entity)
            .col(task::Column::Status)
            .if_not_exists()
            .to_owned(),
    )
    .await?;
    ensure_index(
        db,
        backend,
        Index::create()
            .name("idx_tasks_scheduled_for")
            .table(task::Entity)
            .col(task::Column::ScheduledFor)
            .if_not_exists()
            .to_owned(),
    )
    .await?;
    ensure_index(
        db,
        backend,
        Index::create()
            .name("idx_tasks_status_scheduled")
            .table(task::Entity)
            .col(task::Column::Status)
            .col(task::Column::ScheduledFor)
            .if_not_exists()
            .to_owned(),
    )
    .await?;
    ensure_index(
        db,
        backend,
        Index::create()
            .name("idx_tasks_created_at")
            .table(task::Entity)
            .col(task::Column::CreatedAt)
            .if_not_exists()
            .to_owned(),
    )
    .await?;
    ensure_index(
        db,
        backend,
        Index::create()
            .name("idx_tasks_lease_expires_at")
            .table(task::Entity)
            .col(task::Column::LeaseExpiresAt)
            .if_not_exists()
            .to_owned(),
    )
    .await?;
    ensure_index(
        db,
        backend,
        Index::create()
            .name("idx_tasks_next_retry_at")
            .table(task::Entity)
            .col(task::Column::NextRetryAt)
            .if_not_exists()
            .to_owned(),
    )
    .await?;
    ensure_index(
        db,
        backend,
        Index::create()
            .name("idx_swarm_delegations_agent")
            .table(swarm_delegation::Entity)
            .col(swarm_delegation::Column::AgentId)
            .if_not_exists()
            .to_owned(),
    )
    .await?;
    ensure_index(
        db,
        backend,
        Index::create()
            .name("idx_messages_conversation")
            .table(message::Entity)
            .col(message::Column::ConversationId)
            .if_not_exists()
            .to_owned(),
    )
    .await?;
    ensure_index(
        db,
        backend,
        Index::create()
            .name("idx_messages_timestamp")
            .table(message::Entity)
            .col(message::Column::Timestamp)
            .if_not_exists()
            .to_owned(),
    )
    .await?;
    ensure_index(
        db,
        backend,
        Index::create()
            .name("idx_messages_role_timestamp")
            .table(message::Entity)
            .col(message::Column::Role)
            .col(message::Column::Timestamp)
            .if_not_exists()
            .to_owned(),
    )
    .await?;
    ensure_index(
        db,
        backend,
        Index::create()
            .name("idx_messages_conversation_timestamp")
            .table(message::Entity)
            .col(message::Column::ConversationId)
            .col(message::Column::Timestamp)
            .if_not_exists()
            .to_owned(),
    )
    .await?;
    ensure_index(
        db,
        backend,
        Index::create()
            .name("idx_conversations_updated")
            .table(conversation::Entity)
            .col(conversation::Column::UpdatedAt)
            .if_not_exists()
            .to_owned(),
    )
    .await?;
    ensure_index(
        db,
        backend,
        Index::create()
            .name("idx_conversations_project")
            .table(conversation::Entity)
            .col(conversation::Column::ProjectId)
            .if_not_exists()
            .to_owned(),
    )
    .await?;
    ensure_index(
        db,
        backend,
        Index::create()
            .name("idx_conversations_starred_updated")
            .table(conversation::Entity)
            .col(conversation::Column::Starred)
            .col(conversation::Column::UpdatedAt)
            .if_not_exists()
            .to_owned(),
    )
    .await?;
    ensure_index(
        db,
        backend,
        Index::create()
            .name("idx_documents_project")
            .table(document::Entity)
            .col(document::Column::ProjectId)
            .if_not_exists()
            .to_owned(),
    )
    .await?;
    ensure_index(
        db,
        backend,
        Index::create()
            .name("idx_document_chunks_doc")
            .table(document_chunk::Entity)
            .col(document_chunk::Column::DocumentId)
            .if_not_exists()
            .to_owned(),
    )
    .await?;
    ensure_index(
        db,
        backend,
        Index::create()
            .name("idx_document_chunks_doc_chunk")
            .table(document_chunk::Entity)
            .col(document_chunk::Column::DocumentId)
            .col(document_chunk::Column::ChunkIndex)
            .unique()
            .if_not_exists()
            .to_owned(),
    )
    .await?;
    ensure_pgvector_hnsw_index(
        db,
        backend,
        "action_catalog_index",
        "embedding",
        ACTION_CATALOG_INDEX_HNSW_SQL,
        "action catalog pgvector HNSW partial index",
    )
    .await?;
    ensure_pgvector_hnsw_index(
        db,
        backend,
        "document_chunks",
        "embedding",
        "CREATE INDEX IF NOT EXISTS idx_document_chunks_embedding_hnsw \
         ON document_chunks USING hnsw (embedding vector_cosine_ops) \
         WHERE embedding IS NOT NULL",
        "document chunk pgvector HNSW index",
    )
    .await?;
    ensure_pgvector_hnsw_index(
        db,
        backend,
        "experience_items",
        "embedding",
        "CREATE INDEX IF NOT EXISTS idx_experience_items_embedding_hnsw \
         ON experience_items USING hnsw (embedding vector_cosine_ops) \
         WHERE embedding IS NOT NULL \
           AND status = 'active' \
           AND kind IN ('personal_fact', 'constraint')",
        "experience_items personal-fact pgvector HNSW partial index",
    )
    .await?;
    ensure_index(
        db,
        backend,
        Index::create()
            .name("idx_notifications_created")
            .table(notification::Entity)
            .col(notification::Column::CreatedAt)
            .if_not_exists()
            .to_owned(),
    )
    .await?;
    ensure_index(
        db,
        backend,
        Index::create()
            .name("idx_approval_log_status")
            .table(approval_log::Entity)
            .col(approval_log::Column::Status)
            .if_not_exists()
            .to_owned(),
    )
    .await?;
    ensure_index(
        db,
        backend,
        Index::create()
            .name("idx_approval_log_requested")
            .table(approval_log::Entity)
            .col(approval_log::Column::RequestedAt)
            .if_not_exists()
            .to_owned(),
    )
    .await?;
    ensure_index(
        db,
        backend,
        Index::create()
            .name("idx_automation_runs_started")
            .table(automation_run::Entity)
            .col(automation_run::Column::StartedAt)
            .if_not_exists()
            .to_owned(),
    )
    .await?;
    ensure_index(
        db,
        backend,
        Index::create()
            .name("idx_automation_runs_automation_id")
            .table(automation_run::Entity)
            .col(automation_run::Column::AutomationId)
            .if_not_exists()
            .to_owned(),
    )
    .await?;
    ensure_index(
        db,
        backend,
        Index::create()
            .name("idx_watchers_status")
            .table(watcher::Entity)
            .col(watcher::Column::Status)
            .if_not_exists()
            .to_owned(),
    )
    .await?;
    ensure_index(
        db,
        backend,
        Index::create()
            .name("idx_watchers_created")
            .table(watcher::Entity)
            .col(watcher::Column::CreatedAt)
            .if_not_exists()
            .to_owned(),
    )
    .await?;
    ensure_index(
        db,
        backend,
        Index::create()
            .name("idx_watchers_next_retry_at")
            .table(watcher::Entity)
            .col(watcher::Column::NextRetryAt)
            .if_not_exists()
            .to_owned(),
    )
    .await?;
    ensure_index(
        db,
        backend,
        Index::create()
            .name("idx_watchers_lease_expires_at")
            .table(watcher::Entity)
            .col(watcher::Column::LeaseExpiresAt)
            .if_not_exists()
            .to_owned(),
    )
    .await?;
    ensure_index(
        db,
        backend,
        Index::create()
            .name("idx_security_logs_created")
            .table(security_log::Entity)
            .col(security_log::Column::CreatedAt)
            .if_not_exists()
            .to_owned(),
    )
    .await?;
    ensure_index(
        db,
        backend,
        Index::create()
            .name("idx_security_logs_type")
            .table(security_log::Entity)
            .col(security_log::Column::EventType)
            .if_not_exists()
            .to_owned(),
    )
    .await?;
    ensure_index(
        db,
        backend,
        Index::create()
            .name("idx_operational_logs_created")
            .table(operational_log::Entity)
            .col(operational_log::Column::CreatedAt)
            .if_not_exists()
            .to_owned(),
    )
    .await?;
    ensure_index(
        db,
        backend,
        Index::create()
            .name("idx_operational_logs_event_type")
            .table(operational_log::Entity)
            .col(operational_log::Column::EventType)
            .if_not_exists()
            .to_owned(),
    )
    .await?;
    ensure_index(
        db,
        backend,
        Index::create()
            .name("idx_operational_logs_event_type_created")
            .table(operational_log::Entity)
            .col(operational_log::Column::EventType)
            .col(operational_log::Column::CreatedAt)
            .if_not_exists()
            .to_owned(),
    )
    .await?;
    ensure_index(
        db,
        backend,
        Index::create()
            .name("idx_operational_logs_tool_name")
            .table(operational_log::Entity)
            .col(operational_log::Column::ToolName)
            .if_not_exists()
            .to_owned(),
    )
    .await?;
    ensure_index(
        db,
        backend,
        Index::create()
            .name("idx_operational_logs_success")
            .table(operational_log::Entity)
            .col(operational_log::Column::Success)
            .if_not_exists()
            .to_owned(),
    )
    .await?;
    ensure_index(
        db,
        backend,
        Index::create()
            .name("idx_operational_logs_policy_version")
            .table(operational_log::Entity)
            .col(operational_log::Column::PolicyVersion)
            .if_not_exists()
            .to_owned(),
    )
    .await?;
    ensure_index(
        db,
        backend,
        Index::create()
            .name("idx_operational_logs_strategy_version")
            .table(operational_log::Entity)
            .col(operational_log::Column::StrategyVersion)
            .if_not_exists()
            .to_owned(),
    )
    .await?;
    ensure_index(
        db,
        backend,
        Index::create()
            .name("idx_operational_logs_conversation_id")
            .table(operational_log::Entity)
            .col(operational_log::Column::ConversationId)
            .if_not_exists()
            .to_owned(),
    )
    .await?;
    ensure_index(
        db,
        backend,
        Index::create()
            .name("idx_operational_logs_trace_id")
            .table(operational_log::Entity)
            .col(operational_log::Column::TraceId)
            .if_not_exists()
            .to_owned(),
    )
    .await?;
    ensure_index(
        db,
        backend,
        Index::create()
            .name("idx_llm_usage_created")
            .table(llm_usage::Entity)
            .col(llm_usage::Column::CreatedAt)
            .if_not_exists()
            .to_owned(),
    )
    .await?;
    ensure_index(
        db,
        backend,
        Index::create()
            .name("idx_llm_usage_model")
            .table(llm_usage::Entity)
            .col(llm_usage::Column::Model)
            .if_not_exists()
            .to_owned(),
    )
    .await?;
    ensure_index(
        db,
        backend,
        Index::create()
            .name("idx_llm_usage_provider")
            .table(llm_usage::Entity)
            .col(llm_usage::Column::Provider)
            .if_not_exists()
            .to_owned(),
    )
    .await?;
    ensure_index(
        db,
        backend,
        Index::create()
            .name("idx_llm_usage_channel")
            .table(llm_usage::Entity)
            .col(llm_usage::Column::Channel)
            .if_not_exists()
            .to_owned(),
    )
    .await?;
    ensure_index(
        db,
        backend,
        Index::create()
            .name("idx_user_preferences_key")
            .table(user_preference::Entity)
            .col(user_preference::Column::Key)
            .if_not_exists()
            .to_owned(),
    )
    .await?;
    ensure_index(
        db,
        backend,
        Index::create()
            .name("idx_user_preferences_project")
            .table(user_preference::Entity)
            .col(user_preference::Column::ProjectId)
            .if_not_exists()
            .to_owned(),
    )
    .await?;
    ensure_index(
        db,
        backend,
        Index::create()
            .name("idx_user_data_kind")
            .table(user_data_item::Entity)
            .col(user_data_item::Column::Kind)
            .if_not_exists()
            .to_owned(),
    )
    .await?;
    ensure_index(
        db,
        backend,
        Index::create()
            .name("idx_user_data_conversation")
            .table(user_data_item::Entity)
            .col(user_data_item::Column::ConversationId)
            .if_not_exists()
            .to_owned(),
    )
    .await?;
    ensure_index(
        db,
        backend,
        Index::create()
            .name("idx_user_data_url")
            .table(user_data_item::Entity)
            .col(user_data_item::Column::Url)
            .if_not_exists()
            .to_owned(),
    )
    .await?;
    ensure_index(
        db,
        backend,
        Index::create()
            .name("idx_user_data_project")
            .table(user_data_item::Entity)
            .col(user_data_item::Column::ProjectId)
            .if_not_exists()
            .to_owned(),
    )
    .await?;
    ensure_index(
        db,
        backend,
        Index::create()
            .name("idx_user_data_updated")
            .table(user_data_item::Entity)
            .col(user_data_item::Column::UpdatedAt)
            .if_not_exists()
            .to_owned(),
    )
    .await?;
    ensure_index(
        db,
        backend,
        Index::create()
            .name("idx_knowledge_project")
            .table(knowledge_item::Entity)
            .col(knowledge_item::Column::ProjectId)
            .if_not_exists()
            .to_owned(),
    )
    .await?;
    ensure_index(
        db,
        backend,
        Index::create()
            .name("idx_knowledge_updated")
            .table(knowledge_item::Entity)
            .col(knowledge_item::Column::UpdatedAt)
            .if_not_exists()
            .to_owned(),
    )
    .await?;
    ensure_index(
        db,
        backend,
        Index::create()
            .name("idx_execution_runs_status")
            .table(execution_run::Entity)
            .col(execution_run::Column::Status)
            .if_not_exists()
            .to_owned(),
    )
    .await?;
    ensure_index(
        db,
        backend,
        Index::create()
            .name("idx_execution_runs_stage")
            .table(execution_run::Entity)
            .col(execution_run::Column::CurrentStage)
            .if_not_exists()
            .to_owned(),
    )
    .await?;
    ensure_index(
        db,
        backend,
        Index::create()
            .name("idx_execution_runs_updated_at")
            .table(execution_run::Entity)
            .col(execution_run::Column::UpdatedAt)
            .if_not_exists()
            .to_owned(),
    )
    .await?;
    ensure_index(
        db,
        backend,
        Index::create()
            .name("idx_execution_runs_request_id")
            .table(execution_run::Entity)
            .col(execution_run::Column::RequestId)
            .unique()
            .if_not_exists()
            .to_owned(),
    )
    .await?;
    ensure_index(
        db,
        backend,
        Index::create()
            .name("idx_execution_runs_conversation_id")
            .table(execution_run::Entity)
            .col(execution_run::Column::ConversationId)
            .if_not_exists()
            .to_owned(),
    )
    .await?;
    ensure_index(
        db,
        backend,
        Index::create()
            .name("idx_run_checkpoints_run_id")
            .table(run_checkpoint::Entity)
            .col(run_checkpoint::Column::RunId)
            .if_not_exists()
            .to_owned(),
    )
    .await?;
    ensure_index(
        db,
        backend,
        Index::create()
            .name("idx_run_checkpoints_run_sequence")
            .table(run_checkpoint::Entity)
            .col(run_checkpoint::Column::RunId)
            .col(run_checkpoint::Column::SequenceNo)
            .unique()
            .if_not_exists()
            .to_owned(),
    )
    .await?;
    ensure_index(
        db,
        backend,
        Index::create()
            .name("idx_tool_attempts_run_id")
            .table(tool_attempt::Entity)
            .col(tool_attempt::Column::RunId)
            .if_not_exists()
            .to_owned(),
    )
    .await?;
    ensure_index(
        db,
        backend,
        Index::create()
            .name("idx_tool_attempts_run_sequence")
            .table(tool_attempt::Entity)
            .col(tool_attempt::Column::RunId)
            .col(tool_attempt::Column::SequenceNo)
            .unique()
            .if_not_exists()
            .to_owned(),
    )
    .await?;
    ensure_index(
        db,
        backend,
        Index::create()
            .name("idx_experience_runs_execution_run")
            .table(experience_run::Entity)
            .col(experience_run::Column::ExecutionRunId)
            .unique()
            .if_not_exists()
            .to_owned(),
    )
    .await?;
    ensure_index(
        db,
        backend,
        Index::create()
            .name("idx_experience_runs_conversation_created")
            .table(experience_run::Entity)
            .col(experience_run::Column::ConversationId)
            .col(experience_run::Column::CreatedAt)
            .if_not_exists()
            .to_owned(),
    )
    .await?;
    ensure_index(
        db,
        backend,
        Index::create()
            .name("idx_experience_runs_project_created")
            .table(experience_run::Entity)
            .col(experience_run::Column::ProjectId)
            .col(experience_run::Column::CreatedAt)
            .if_not_exists()
            .to_owned(),
    )
    .await?;
    ensure_index(
        db,
        backend,
        Index::create()
            .name("idx_experience_runs_scope_state")
            .table(experience_run::Entity)
            .col(experience_run::Column::Scope)
            .col(experience_run::Column::SuccessState)
            .col(experience_run::Column::CorrectionState)
            .col(experience_run::Column::Consolidated)
            .if_not_exists()
            .to_owned(),
    )
    .await?;
    ensure_index(
        db,
        backend,
        Index::create()
            .name("idx_experience_runs_intent")
            .table(experience_run::Entity)
            .col(experience_run::Column::IntentKey)
            .col(experience_run::Column::TaskType)
            .col(experience_run::Column::ToolSequenceDigest)
            .if_not_exists()
            .to_owned(),
    )
    .await?;
    ensure_index(
        db,
        backend,
        Index::create()
            .name("idx_experience_runs_updated")
            .table(experience_run::Entity)
            .col(experience_run::Column::UpdatedAt)
            .if_not_exists()
            .to_owned(),
    )
    .await?;
    ensure_index(
        db,
        backend,
        Index::create()
            .name("idx_experience_runs_heuristic_reflection")
            .table(experience_run::Entity)
            .col(experience_run::Column::Consolidated)
            .col(experience_run::Column::HeuristicReflected)
            .col(experience_run::Column::HeuristicReflectionStatus)
            .col(experience_run::Column::UpdatedAt)
            .if_not_exists()
            .to_owned(),
    )
    .await?;
    ensure_index(
        db,
        backend,
        Index::create()
            .name("idx_experience_items_scope_key")
            .table(experience_item::Entity)
            .col(experience_item::Column::Kind)
            .col(experience_item::Column::Scope)
            .col(experience_item::Column::ProjectId)
            .col(experience_item::Column::ConversationId)
            .col(experience_item::Column::NormalizedKey)
            .unique()
            .if_not_exists()
            .to_owned(),
    )
    .await?;
    ensure_index(
        db,
        backend,
        Index::create()
            .name("idx_experience_items_scope_status")
            .table(experience_item::Entity)
            .col(experience_item::Column::Scope)
            .col(experience_item::Column::Status)
            .col(experience_item::Column::Kind)
            .col(experience_item::Column::UpdatedAt)
            .if_not_exists()
            .to_owned(),
    )
    .await?;
    ensure_index(
        db,
        backend,
        Index::create()
            .name("idx_experience_items_project_status")
            .table(experience_item::Entity)
            .col(experience_item::Column::ProjectId)
            .col(experience_item::Column::Status)
            .col(experience_item::Column::UpdatedAt)
            .if_not_exists()
            .to_owned(),
    )
    .await?;
    ensure_index(
        db,
        backend,
        Index::create()
            .name("idx_experience_items_conversation_status")
            .table(experience_item::Entity)
            .col(experience_item::Column::ConversationId)
            .col(experience_item::Column::Status)
            .col(experience_item::Column::UpdatedAt)
            .if_not_exists()
            .to_owned(),
    )
    .await?;
    ensure_index(
        db,
        backend,
        Index::create()
            .name("idx_experience_edges_source")
            .table(experience_edge::Entity)
            .col(experience_edge::Column::SourceRef)
            .col(experience_edge::Column::SourceKind)
            .col(experience_edge::Column::EdgeType)
            .if_not_exists()
            .to_owned(),
    )
    .await?;
    ensure_index(
        db,
        backend,
        Index::create()
            .name("idx_experience_edges_target")
            .table(experience_edge::Entity)
            .col(experience_edge::Column::TargetRef)
            .col(experience_edge::Column::TargetKind)
            .col(experience_edge::Column::EdgeType)
            .if_not_exists()
            .to_owned(),
    )
    .await?;
    ensure_index(
        db,
        backend,
        Index::create()
            .name("idx_experience_edges_source_run")
            .table(experience_edge::Entity)
            .col(experience_edge::Column::SourceRunId)
            .if_not_exists()
            .to_owned(),
    )
    .await?;
    ensure_index(
        db,
        backend,
        Index::create()
            .name("idx_procedural_patterns_scope_key")
            .table(procedural_pattern::Entity)
            .col(procedural_pattern::Column::Scope)
            .col(procedural_pattern::Column::ProjectId)
            .col(procedural_pattern::Column::ConversationId)
            .col(procedural_pattern::Column::IntentKey)
            .col(procedural_pattern::Column::ToolSequenceDigest)
            .unique()
            .if_not_exists()
            .to_owned(),
    )
    .await?;
    ensure_index(
        db,
        backend,
        Index::create()
            .name("idx_procedural_patterns_scope_status")
            .table(procedural_pattern::Entity)
            .col(procedural_pattern::Column::Scope)
            .col(procedural_pattern::Column::Status)
            .col(procedural_pattern::Column::UpdatedAt)
            .if_not_exists()
            .to_owned(),
    )
    .await?;
    ensure_index(
        db,
        backend,
        Index::create()
            .name("idx_procedural_patterns_project_status")
            .table(procedural_pattern::Entity)
            .col(procedural_pattern::Column::ProjectId)
            .col(procedural_pattern::Column::Status)
            .col(procedural_pattern::Column::UpdatedAt)
            .if_not_exists()
            .to_owned(),
    )
    .await?;
    ensure_index(
        db,
        backend,
        Index::create()
            .name("idx_learning_candidates_status")
            .table(learning_candidate::Entity)
            .col(learning_candidate::Column::ApprovalStatus)
            .col(learning_candidate::Column::CandidateType)
            .col(learning_candidate::Column::UpdatedAt)
            .if_not_exists()
            .to_owned(),
    )
    .await?;
    ensure_index(
        db,
        backend,
        Index::create()
            .name("idx_learning_candidates_subject")
            .table(learning_candidate::Entity)
            .col(learning_candidate::Column::SubjectKey)
            .col(learning_candidate::Column::CandidateType)
            .if_not_exists()
            .to_owned(),
    )
    .await?;
    ensure_index(
        db,
        backend,
        Index::create()
            .name("idx_learning_candidates_pattern")
            .table(learning_candidate::Entity)
            .col(learning_candidate::Column::PatternId)
            .if_not_exists()
            .to_owned(),
    )
    .await?;
    ensure_index(
        db,
        backend,
        Index::create()
            .name("idx_learning_candidates_project")
            .table(learning_candidate::Entity)
            .col(learning_candidate::Column::ProjectId)
            .col(learning_candidate::Column::ApprovalStatus)
            .col(learning_candidate::Column::UpdatedAt)
            .if_not_exists()
            .to_owned(),
    )
    .await?;
    ensure_index(
        db,
        backend,
        Index::create()
            .name("idx_memory_capture_events_status")
            .table(memory_capture_event::Entity)
            .col(memory_capture_event::Column::Status)
            .col(memory_capture_event::Column::UpdatedAt)
            .if_not_exists()
            .to_owned(),
    )
    .await?;
    ensure_index(
        db,
        backend,
        Index::create()
            .name("idx_memory_capture_events_message")
            .table(memory_capture_event::Entity)
            .col(memory_capture_event::Column::SourceMessageId)
            .if_not_exists()
            .to_owned(),
    )
    .await?;
    ensure_index(
        db,
        backend,
        Index::create()
            .name("idx_memory_capture_events_scope")
            .table(memory_capture_event::Entity)
            .col(memory_capture_event::Column::ProjectId)
            .col(memory_capture_event::Column::ConversationId)
            .col(memory_capture_event::Column::CreatedAt)
            .if_not_exists()
            .to_owned(),
    )
    .await?;
    ensure_index(
        db,
        backend,
        Index::create()
            .name("idx_memory_operations_status")
            .table(memory_operation::Entity)
            .col(memory_operation::Column::Status)
            .col(memory_operation::Column::OperationType)
            .col(memory_operation::Column::UpdatedAt)
            .if_not_exists()
            .to_owned(),
    )
    .await?;
    ensure_index(
        db,
        backend,
        Index::create()
            .name("idx_memory_operations_capture")
            .table(memory_operation::Entity)
            .col(memory_operation::Column::CaptureEventId)
            .if_not_exists()
            .to_owned(),
    )
    .await?;
    ensure_index(
        db,
        backend,
        Index::create()
            .name("idx_memory_operations_target")
            .table(memory_operation::Entity)
            .col(memory_operation::Column::TargetMemoryId)
            .col(memory_operation::Column::UpdatedAt)
            .if_not_exists()
            .to_owned(),
    )
    .await?;
    ensure_index(
        db,
        backend,
        Index::create()
            .name("idx_memory_operations_scope")
            .table(memory_operation::Entity)
            .col(memory_operation::Column::ProjectId)
            .col(memory_operation::Column::ConversationId)
            .col(memory_operation::Column::UpdatedAt)
            .if_not_exists()
            .to_owned(),
    )
    .await?;
    ensure_index(
        db,
        backend,
        Index::create()
            .name("idx_memory_evidence_links_operation")
            .table(memory_evidence_link::Entity)
            .col(memory_evidence_link::Column::OperationId)
            .if_not_exists()
            .to_owned(),
    )
    .await?;
    ensure_index(
        db,
        backend,
        Index::create()
            .name("idx_memory_evidence_links_memory")
            .table(memory_evidence_link::Entity)
            .col(memory_evidence_link::Column::MemoryId)
            .if_not_exists()
            .to_owned(),
    )
    .await?;
    ensure_index(
        db,
        backend,
        Index::create()
            .name("idx_memory_evidence_links_evidence")
            .table(memory_evidence_link::Entity)
            .col(memory_evidence_link::Column::EvidenceKind)
            .col(memory_evidence_link::Column::EvidenceRef)
            .if_not_exists()
            .to_owned(),
    )
    .await?;
    ensure_index(
        db,
        backend,
        Index::create()
            .name("idx_recall_events_memory_created")
            .table(recall_event::Entity)
            .col(recall_event::Column::MemoryId)
            .col(recall_event::Column::CreatedAt)
            .if_not_exists()
            .to_owned(),
    )
    .await?;
    ensure_index(
        db,
        backend,
        Index::create()
            .name("idx_recall_events_related_created")
            .table(recall_event::Entity)
            .col(recall_event::Column::RelatedMemoryId)
            .col(recall_event::Column::CreatedAt)
            .if_not_exists()
            .to_owned(),
    )
    .await?;
    ensure_index(
        db,
        backend,
        Index::create()
            .name("idx_recall_events_project_created")
            .table(recall_event::Entity)
            .col(recall_event::Column::ProjectId)
            .col(recall_event::Column::CreatedAt)
            .if_not_exists()
            .to_owned(),
    )
    .await?;
    ensure_index(
        db,
        backend,
        Index::create()
            .name("idx_recall_events_type_created")
            .table(recall_event::Entity)
            .col(recall_event::Column::EventType)
            .col(recall_event::Column::CreatedAt)
            .if_not_exists()
            .to_owned(),
    )
    .await?;
    ensure_index(
        db,
        backend,
        Index::create()
            .name("idx_recall_events_reverted_created")
            .table(recall_event::Entity)
            .col(recall_event::Column::RevertedAt)
            .col(recall_event::Column::CreatedAt)
            .if_not_exists()
            .to_owned(),
    )
    .await?;
    ensure_index(
        db,
        backend,
        Index::create()
            .name("idx_recall_tests_memory")
            .table(recall_test::Entity)
            .col(recall_test::Column::MemoryId)
            .col(recall_test::Column::UpdatedAt)
            .if_not_exists()
            .to_owned(),
    )
    .await?;
    ensure_index(
        db,
        backend,
        Index::create()
            .name("idx_recall_tests_scope")
            .table(recall_test::Entity)
            .col(recall_test::Column::Scope)
            .col(recall_test::Column::ProjectId)
            .col(recall_test::Column::UpdatedAt)
            .if_not_exists()
            .to_owned(),
    )
    .await?;

    Ok(())
}

#[cfg(test)]
mod tests {
    use super::{vector_type_has_dimensions, ACTION_CATALOG_INDEX_HNSW_SQL};

    #[test]
    fn vector_type_dimension_detection_requires_explicit_dimensions() {
        assert!(vector_type_has_dimensions("vector(384)"));
        assert!(vector_type_has_dimensions("public.vector(1536)"));
        assert!(!vector_type_has_dimensions("vector"));
        assert!(!vector_type_has_dimensions("text"));
    }

    #[test]
    fn action_catalog_hnsw_index_is_partial_and_cosine() {
        assert!(ACTION_CATALOG_INDEX_HNSW_SQL.contains("USING hnsw"));
        assert!(ACTION_CATALOG_INDEX_HNSW_SQL.contains("vector_cosine_ops"));
        assert!(ACTION_CATALOG_INDEX_HNSW_SQL.contains("enabled = true"));
        assert!(ACTION_CATALOG_INDEX_HNSW_SQL.contains("embedding IS NOT NULL"));
    }
}
