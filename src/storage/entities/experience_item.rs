//! Learned memory item entity for the Postgres-native experience graph.

use sea_orm::entity::prelude::*;
use serde::{Deserialize, Serialize};

#[derive(Clone, Debug, PartialEq, DeriveEntityModel, Serialize, Deserialize)]
#[sea_orm(table_name = "experience_items")]
pub struct Model {
    #[sea_orm(primary_key, auto_increment = false)]
    pub id: String,
    pub kind: String,
    pub scope: String,
    #[sea_orm(nullable)]
    pub project_id: Option<String>,
    #[sea_orm(nullable)]
    pub conversation_id: Option<String>,
    pub title: String,
    pub content: String,
    pub normalized_key: String,
    pub confidence: f64,
    pub support_count: i32,
    pub contradiction_count: i32,
    pub status: String,
    #[sea_orm(column_type = "JsonBinary")]
    pub metadata: Json,
    #[sea_orm(nullable)]
    pub last_supported_at: Option<String>,
    #[sea_orm(nullable)]
    pub last_contradicted_at: Option<String>,
    pub created_at: String,
    pub updated_at: String,
    #[sea_orm(nullable)]
    pub embedding: Option<PgVector>,
}

#[allow(dead_code)]
#[derive(Copy, Clone, Debug, EnumIter, DeriveRelation)]
pub enum Relation {}

impl ActiveModelBehavior for ActiveModel {}
