//! Structured lifecycle memory operation emitted by capture and review flows.

use sea_orm::entity::prelude::*;
use serde::{Deserialize, Serialize};

#[derive(Clone, Debug, PartialEq, DeriveEntityModel, Serialize, Deserialize)]
#[sea_orm(table_name = "memory_operations")]
pub struct Model {
    #[sea_orm(primary_key, auto_increment = false)]
    pub id: String,
    #[sea_orm(nullable)]
    pub capture_event_id: Option<String>,
    pub operation_type: String,
    pub status: String,
    #[sea_orm(nullable)]
    pub target_memory_id: Option<String>,
    #[sea_orm(nullable)]
    pub applied_memory_id: Option<String>,
    #[sea_orm(nullable)]
    pub key: Option<String>,
    #[sea_orm(nullable)]
    pub value: Option<String>,
    pub memory_kind: String,
    pub durability: String,
    pub scope: String,
    #[sea_orm(nullable)]
    pub project_id: Option<String>,
    #[sea_orm(nullable)]
    pub conversation_id: Option<String>,
    pub confidence: f64,
    pub looks_sensitive: bool,
    #[sea_orm(nullable)]
    pub sensitive_reason: Option<String>,
    #[sea_orm(nullable)]
    pub valid_from: Option<String>,
    #[sea_orm(nullable)]
    pub expires_at: Option<String>,
    #[sea_orm(nullable)]
    pub review_at: Option<String>,
    #[sea_orm(nullable)]
    pub rationale: Option<String>,
    #[sea_orm(column_type = "JsonBinary")]
    pub evidence_refs: Json,
    #[sea_orm(column_type = "JsonBinary")]
    pub model_metadata: Json,
    #[sea_orm(column_type = "JsonBinary")]
    pub apply_metadata: Json,
    #[sea_orm(nullable)]
    pub applied_at: Option<String>,
    #[sea_orm(nullable)]
    pub reviewed_at: Option<String>,
    #[sea_orm(nullable)]
    pub review_notes: Option<String>,
    pub created_at: String,
    pub updated_at: String,
}

#[allow(dead_code)]
#[derive(Copy, Clone, Debug, EnumIter, DeriveRelation)]
pub enum Relation {}

impl ActiveModelBehavior for ActiveModel {}
