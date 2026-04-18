//! Durable record for lifecycle user-memory capture attempts.

use sea_orm::entity::prelude::*;
use serde::{Deserialize, Serialize};

#[derive(Clone, Debug, PartialEq, DeriveEntityModel, Serialize, Deserialize)]
#[sea_orm(table_name = "memory_capture_events")]
pub struct Model {
    #[sea_orm(primary_key, auto_increment = false)]
    pub id: String,
    #[sea_orm(nullable)]
    pub source_message_id: Option<String>,
    #[sea_orm(nullable)]
    pub conversation_id: Option<String>,
    #[sea_orm(nullable)]
    pub project_id: Option<String>,
    pub channel: String,
    pub status: String,
    pub capture_kind: String,
    pub source_hash: String,
    #[sea_orm(column_type = "JsonBinary")]
    pub attempt_metadata: Json,
    #[sea_orm(column_type = "JsonBinary")]
    pub error_history: Json,
    pub replay_count: i32,
    #[sea_orm(nullable)]
    pub next_retry_at: Option<String>,
    #[sea_orm(nullable)]
    pub completed_at: Option<String>,
    pub created_at: String,
    pub updated_at: String,
}

#[allow(dead_code)]
#[derive(Copy, Clone, Debug, EnumIter, DeriveRelation)]
pub enum Relation {}

impl ActiveModelBehavior for ActiveModel {}
