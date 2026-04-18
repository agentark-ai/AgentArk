//! Provenance edge between memory operations, memories, messages, and events.

use sea_orm::entity::prelude::*;
use serde::{Deserialize, Serialize};

#[derive(Clone, Debug, PartialEq, DeriveEntityModel, Serialize, Deserialize)]
#[sea_orm(table_name = "memory_evidence_links")]
pub struct Model {
    #[sea_orm(primary_key, auto_increment = false)]
    pub id: String,
    #[sea_orm(nullable)]
    pub operation_id: Option<String>,
    #[sea_orm(nullable)]
    pub memory_id: Option<String>,
    pub evidence_kind: String,
    pub evidence_ref: String,
    #[sea_orm(nullable)]
    pub source_message_id: Option<String>,
    #[sea_orm(nullable)]
    pub capture_event_id: Option<String>,
    #[sea_orm(nullable)]
    pub project_id: Option<String>,
    #[sea_orm(nullable)]
    pub conversation_id: Option<String>,
    #[sea_orm(column_type = "JsonBinary")]
    pub metadata: Json,
    pub created_at: String,
}

#[allow(dead_code)]
#[derive(Copy, Clone, Debug, EnumIter, DeriveRelation)]
pub enum Relation {}

impl ActiveModelBehavior for ActiveModel {}
