//! Operational telemetry log entity.
//!
//! Stores structured runtime events for self-evolution and analysis.

use sea_orm::entity::prelude::*;
use serde::{Deserialize, Serialize};

#[derive(Clone, Debug, PartialEq, DeriveEntityModel, Serialize, Deserialize)]
#[sea_orm(table_name = "operational_logs")]
pub struct Model {
    #[sea_orm(primary_key, auto_increment = false)]
    pub id: String,
    pub created_at: String,
    #[sea_orm(nullable)]
    pub trace_id: Option<String>,
    #[sea_orm(nullable)]
    pub conversation_id: Option<String>,
    pub channel: String,
    pub event_type: String,
    pub success: bool,
    pub outcome: String,
    #[sea_orm(nullable)]
    pub tool_name: Option<String>,
    #[sea_orm(nullable)]
    pub latency_ms: Option<i64>,
    #[sea_orm(nullable)]
    pub arguments: Option<String>,
    #[sea_orm(nullable)]
    pub payload: Option<String>,
    #[sea_orm(nullable)]
    pub strategy_version: Option<String>,
    #[sea_orm(nullable)]
    pub policy_version: Option<String>,
    #[sea_orm(nullable)]
    pub prompt_version: Option<String>,
    #[sea_orm(nullable)]
    pub model_slot: Option<String>,
}

#[allow(dead_code)]
#[derive(Copy, Clone, Debug, EnumIter, DeriveRelation)]
pub enum Relation {}

impl ActiveModelBehavior for ActiveModel {}
