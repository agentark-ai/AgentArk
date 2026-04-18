//! Abuse-tracker state entity — per-source sliding-window trip tracking
//! with pending-approval / paused states.
//!
//! The inbound intent classifier records a "trip" into this table whenever
//! a message is blocked. After enough trips inside the configured window,
//! the source moves to `pending_approval` and a request is written into
//! `approval_log` for operator review. There is no automatic unblock —
//! operator approval or rejection is required, by design (Q10).

use sea_orm::entity::prelude::*;
use serde::{Deserialize, Serialize};

#[derive(Clone, Debug, PartialEq, DeriveEntityModel, Serialize, Deserialize)]
#[sea_orm(table_name = "abuse_tracker_state")]
pub struct Model {
    /// SHA-256 hex digest of (channel_id, user_identity). Stable per source.
    #[sea_orm(primary_key, auto_increment = false)]
    pub source_key_hash: String,
    pub channel_id: String,
    #[sea_orm(nullable)]
    pub user_identity: Option<String>,
    /// JSON array of RFC3339 timestamps recording the rolling trip window.
    pub trip_timestamps_json: String,
    /// "normal" | "pending_approval" | "paused"
    pub status: String,
    pub last_updated: String,
}

#[allow(dead_code)]
#[derive(Copy, Clone, Debug, EnumIter, DeriveRelation)]
pub enum Relation {}

impl ActiveModelBehavior for ActiveModel {}
