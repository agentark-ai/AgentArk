//! Persisted execution trace entity for Trace history and detail views.

use sea_orm::entity::prelude::*;
use serde::{Deserialize, Serialize};

#[derive(Clone, Debug, PartialEq, DeriveEntityModel, Serialize, Deserialize)]
#[sea_orm(table_name = "execution_traces")]
pub struct Model {
    #[sea_orm(primary_key, auto_increment = false)]
    pub id: String,
    pub message: String,
    pub channel: String,
    #[sea_orm(nullable)]
    pub started_at: Option<String>,
    #[sea_orm(nullable)]
    pub completed_at: Option<String>,
    #[sea_orm(nullable)]
    pub duration_ms: Option<i64>,
    pub step_count: i64,
    pub steps_json: String,
    #[sea_orm(nullable)]
    pub response: Option<String>,
    #[sea_orm(nullable)]
    pub proof_id: Option<String>,
    pub created_at: String,
}

#[allow(dead_code)]
#[derive(Copy, Clone, Debug, EnumIter, DeriveRelation)]
pub enum Relation {}

impl ActiveModelBehavior for ActiveModel {}
