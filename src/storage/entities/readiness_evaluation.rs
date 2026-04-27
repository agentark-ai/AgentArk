//! Developmental readiness evaluation audit entity.

use sea_orm::entity::prelude::*;
use serde::{Deserialize, Serialize};

#[derive(Clone, Debug, PartialEq, DeriveEntityModel, Serialize, Deserialize)]
#[sea_orm(table_name = "readiness_evaluations")]
pub struct Model {
    #[sea_orm(primary_key, auto_increment = false)]
    pub id: String,
    pub target_type: String,
    pub target_id: String,
    pub score: i32,
    pub stage: String,
    pub allows_review: bool,
    pub allows_auto: bool,
    #[sea_orm(column_type = "JsonBinary")]
    pub reasons_json: Json,
    #[sea_orm(column_type = "JsonBinary")]
    pub blockers_json: Json,
    #[sea_orm(column_type = "JsonBinary")]
    pub signals_json: Json,
    pub policy_version: String,
    pub created_at: String,
}

#[allow(dead_code)]
#[derive(Copy, Clone, Debug, EnumIter, DeriveRelation)]
pub enum Relation {}

impl ActiveModelBehavior for ActiveModel {}
