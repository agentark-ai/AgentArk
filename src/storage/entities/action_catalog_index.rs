//! Semantic action catalog index used for bounded tool retrieval.

use sea_orm::entity::prelude::*;
use serde::{Deserialize, Serialize};

#[derive(Clone, Debug, PartialEq, DeriveEntityModel, Serialize, Deserialize)]
#[sea_orm(table_name = "action_catalog_index")]
pub struct Model {
    #[sea_orm(primary_key, auto_increment = false)]
    pub action_name: String,
    pub source: String,
    pub version: String,
    pub descriptor_hash: String,
    pub descriptor_text: String,
    pub enabled: bool,
    #[sea_orm(column_type = "JsonBinary")]
    pub metadata_json: Json,
    #[sea_orm(nullable)]
    pub embedding: Option<PgVector>,
    pub updated_at: String,
}

#[allow(dead_code)]
#[derive(Copy, Clone, Debug, EnumIter, DeriveRelation)]
pub enum Relation {}

impl ActiveModelBehavior for ActiveModel {}
