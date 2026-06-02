//! Durable webhook event and idempotency records.

use sea_orm::entity::prelude::*;
use serde::{Deserialize, Serialize};

#[derive(Clone, Debug, PartialEq, DeriveEntityModel, Serialize, Deserialize)]
#[sea_orm(table_name = "webhook_events")]
pub struct Model {
    #[sea_orm(primary_key, auto_increment = false)]
    pub id: String,
    pub source_id: String,
    pub source_name: String,
    pub provider: String,
    pub received_at: String,
    pub updated_at: String,
    pub event_type: String,
    #[sea_orm(nullable)]
    pub status: Option<String>,
    pub subject: String,
    pub outcome: String,
    pub matched: bool,
    pub queued: bool,
    #[sea_orm(nullable)]
    pub message: Option<String>,
    #[sea_orm(nullable)]
    pub event_id: Option<String>,
    pub dedupe_key: String,
    pub idempotency_key: String,
    pub payload_hash: String,
    #[sea_orm(nullable)]
    pub event_url: Option<String>,
    #[sea_orm(nullable)]
    pub payload_excerpt: Option<String>,
    #[sea_orm(nullable)]
    pub task_id: Option<String>,
    #[sea_orm(nullable)]
    pub conversation_id: Option<String>,
    #[sea_orm(nullable)]
    pub severity: Option<String>,
    pub test_event: bool,
}

#[allow(dead_code)]
#[derive(Copy, Clone, Debug, EnumIter, DeriveRelation)]
pub enum Relation {}

impl ActiveModelBehavior for ActiveModel {}
