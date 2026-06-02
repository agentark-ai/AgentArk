//! Durable webhook source configuration rows.

use sea_orm::entity::prelude::*;
use serde::{Deserialize, Serialize};

#[derive(Clone, Debug, PartialEq, DeriveEntityModel, Serialize, Deserialize)]
#[sea_orm(table_name = "webhook_sources")]
pub struct Model {
    #[sea_orm(primary_key, auto_increment = false)]
    pub id: String,
    pub name: String,
    pub provider: String,
    #[sea_orm(nullable)]
    pub description: Option<String>,
    pub enabled: bool,
    pub auth_mode: String,
    pub match_mode: String,
    pub instruction: String,
    #[sea_orm(nullable)]
    pub event_header: Option<String>,
    #[sea_orm(nullable)]
    pub secret_header: Option<String>,
    #[sea_orm(nullable)]
    pub signature_timestamp_header: Option<String>,
    #[sea_orm(nullable)]
    pub signature_timestamp_tolerance_secs: Option<i64>,
    #[sea_orm(nullable)]
    pub signature_payload_mode: Option<String>,
    pub allow_duplicate: bool,
    pub require_approval: bool,
    pub dedupe_window_secs: i64,
    pub notify_on_queued: bool,
    pub notify_on_success: bool,
    pub notify_on_failure: bool,
    pub output_target: String,
    #[sea_orm(nullable)]
    pub output_channel: Option<String>,
    pub conversation_id: String,
    pub created_at: String,
    pub updated_at: String,
    #[sea_orm(nullable)]
    pub last_received_at: Option<String>,
    #[sea_orm(nullable)]
    pub last_outcome: Option<String>,
    #[sea_orm(nullable)]
    pub last_task_id: Option<String>,
}

#[allow(dead_code)]
#[derive(Copy, Clone, Debug, EnumIter, DeriveRelation)]
pub enum Relation {}

impl ActiveModelBehavior for ActiveModel {}
