//! Derived semantic work units for reflection and clustering.

use sea_orm::entity::prelude::*;
use serde::{Deserialize, Serialize};

#[derive(Clone, Debug, PartialEq, DeriveEntityModel, Serialize, Deserialize)]
#[sea_orm(table_name = "semantic_work_units")]
pub struct Model {
    #[sea_orm(primary_key, auto_increment = false)]
    pub id: String,
    pub source_kind: String,
    pub source_id: String,
    #[sea_orm(nullable)]
    pub conversation_id: Option<String>,
    #[sea_orm(nullable)]
    pub project_id: Option<String>,
    pub channel: String,
    pub title: String,
    pub summary: String,
    pub content_preview: String,
    pub text_hash: String,
    pub occurred_at: String,
    #[sea_orm(nullable)]
    pub period_start: Option<String>,
    #[sea_orm(nullable)]
    pub period_end: Option<String>,
    pub message_count: i32,
    #[sea_orm(column_type = "JsonBinary")]
    pub metadata: Json,
    pub created_at: String,
    pub updated_at: String,
    #[sea_orm(nullable, column_type = "Vector(Some(super::PGVECTOR_EMBEDDING_DIM))")]
    pub embedding: Option<PgVector>,
}

#[allow(dead_code)]
#[derive(Copy, Clone, Debug, EnumIter, DeriveRelation)]
pub enum Relation {}

impl ActiveModelBehavior for ActiveModel {}
