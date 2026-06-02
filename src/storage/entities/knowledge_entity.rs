//! Canonical entity node for the provenance-backed personal knowledge graph.

use sea_orm::entity::prelude::*;
use serde::{Deserialize, Serialize};

#[derive(Clone, Debug, PartialEq, DeriveEntityModel, Serialize, Deserialize)]
#[sea_orm(table_name = "knowledge_entities")]
pub struct Model {
    #[sea_orm(primary_key, auto_increment = false)]
    pub id: String,
    pub entity_type: String,
    pub canonical_name: String,
    pub normalized_name: String,
    #[sea_orm(nullable)]
    pub project_id: Option<String>,
    pub status: String,
    pub confidence: f64,
    #[sea_orm(column_type = "JsonBinary")]
    pub aliases: Json,
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
