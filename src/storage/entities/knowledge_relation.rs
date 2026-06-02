//! Typed relation edge for the provenance-backed personal knowledge graph.

use sea_orm::entity::prelude::*;
use serde::{Deserialize, Serialize};

#[derive(Clone, Debug, PartialEq, DeriveEntityModel, Serialize, Deserialize)]
#[sea_orm(table_name = "knowledge_relations")]
pub struct Model {
    #[sea_orm(primary_key, auto_increment = false)]
    pub id: String,
    pub source_entity_id: String,
    pub target_entity_id: String,
    pub relation_type: String,
    pub status: String,
    pub confidence: f64,
    #[sea_orm(nullable)]
    pub project_id: Option<String>,
    #[sea_orm(nullable)]
    pub valid_from: Option<String>,
    #[sea_orm(nullable)]
    pub valid_until: Option<String>,
    pub support_count: i32,
    pub contradiction_count: i32,
    #[sea_orm(column_type = "JsonBinary")]
    pub metadata: Json,
    pub first_seen_at: String,
    pub last_seen_at: String,
    pub created_at: String,
    pub updated_at: String,
}

#[allow(dead_code)]
#[derive(Copy, Clone, Debug, EnumIter, DeriveRelation)]
pub enum Relation {}

impl ActiveModelBehavior for ActiveModel {}
