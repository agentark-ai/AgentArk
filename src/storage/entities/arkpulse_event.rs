//! Pulse event history stored as individual rows instead of a single KV blob.

use sea_orm::entity::prelude::*;
use serde::{Deserialize, Serialize};

#[derive(Clone, Debug, PartialEq, DeriveEntityModel, Serialize, Deserialize)]
#[sea_orm(table_name = "arkpulse_events")]
pub struct Model {
    #[sea_orm(primary_key, auto_increment = false)]
    pub id: String,
    pub timestamp: String,
    pub status: String,
    pub message: String,
    pub summary: String,
    pub flags_json: String,
    pub overdue_tasks: i32,
    pub failed_tasks: i32,
    pub details_json: String,
}

#[derive(Copy, Clone, Debug, EnumIter, DeriveRelation)]
pub enum Relation {}

impl ActiveModelBehavior for ActiveModel {}
