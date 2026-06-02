//! Evidence row supporting or contradicting a typed knowledge relation.

use sea_orm::entity::prelude::*;
use serde::{Deserialize, Serialize};

#[derive(Clone, Debug, PartialEq, DeriveEntityModel, Serialize, Deserialize)]
#[sea_orm(table_name = "knowledge_relation_evidence")]
pub struct Model {
    #[sea_orm(primary_key, auto_increment = false)]
    pub id: String,
    pub relation_id: String,
    pub evidence_kind: String,
    pub evidence_ref: String,
    #[sea_orm(nullable)]
    pub memory_id: Option<String>,
    #[sea_orm(nullable)]
    pub message_id: Option<String>,
    #[sea_orm(nullable)]
    pub document_id: Option<String>,
    #[sea_orm(nullable)]
    pub project_id: Option<String>,
    #[sea_orm(nullable)]
    pub conversation_id: Option<String>,
    pub polarity: String,
    pub confidence: f64,
    #[sea_orm(nullable)]
    pub excerpt: Option<String>,
    #[sea_orm(column_type = "JsonBinary")]
    pub metadata: Json,
    pub created_at: String,
}

#[allow(dead_code)]
#[derive(Copy, Clone, Debug, EnumIter, DeriveRelation)]
pub enum Relation {}

impl ActiveModelBehavior for ActiveModel {}
