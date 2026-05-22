//! Security log entity — tracks security events for Pulse monitoring

use sea_orm::entity::prelude::*;
use serde::{Deserialize, Serialize};

#[derive(Clone, Debug, PartialEq, DeriveEntityModel, Serialize, Deserialize)]
#[sea_orm(table_name = "security_logs")]
pub struct Model {
    #[sea_orm(primary_key, auto_increment = false)]
    pub id: String,
    /// Event type: "injection", "auth_failure", "rate_limit", "unauthorized_channel"
    pub event_type: String,
    /// Severity: "low", "medium", "high", "critical"
    pub severity: String,
    /// Human-readable description
    pub message: String,
    /// Source IP or channel identifier
    #[sea_orm(nullable)]
    pub source: Option<String>,
    /// Count of events in this batch (from atomic counters)
    pub count: i32,
    pub created_at: String,
}

#[allow(dead_code)]
#[derive(Copy, Clone, Debug, EnumIter, DeriveRelation)]
pub enum Relation {}

impl ActiveModelBehavior for ActiveModel {}
