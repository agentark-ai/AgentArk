//! Durable Memory ledger event.

use sea_orm::entity::prelude::*;
use serde::{Deserialize, Serialize};

#[derive(Clone, Debug, PartialEq, DeriveEntityModel, Serialize, Deserialize)]
#[sea_orm(table_name = "recall_events")]
pub struct Model {
    #[sea_orm(primary_key, auto_increment = false)]
    pub id: String,
    pub event_type: String,
    #[sea_orm(nullable)]
    pub memory_id: Option<String>,
    #[sea_orm(nullable)]
    pub related_memory_id: Option<String>,
    #[sea_orm(nullable)]
    pub scope: Option<String>,
    #[sea_orm(nullable)]
    pub project_id: Option<String>,
    #[sea_orm(nullable)]
    pub conversation_id: Option<String>,
    #[sea_orm(nullable)]
    pub source_kind: Option<String>,
    #[sea_orm(nullable)]
    pub source_ref: Option<String>,
    pub actor: String,
    #[sea_orm(nullable)]
    pub summary: Option<String>,
    #[sea_orm(column_type = "JsonBinary")]
    pub old_snapshot: Json,
    #[sea_orm(column_type = "JsonBinary")]
    pub new_snapshot: Json,
    #[sea_orm(column_type = "JsonBinary")]
    pub metadata: Json,
    #[sea_orm(nullable)]
    pub risk_level: Option<String>,
    #[sea_orm(nullable)]
    pub confidence: Option<f64>,
    pub reversible: bool,
    #[sea_orm(nullable)]
    pub reverted_at: Option<String>,
    pub created_at: String,
    pub updated_at: String,
}

#[allow(dead_code)]
#[derive(Copy, Clone, Debug, EnumIter, DeriveRelation)]
pub enum Relation {}

impl ActiveModelBehavior for ActiveModel {}
