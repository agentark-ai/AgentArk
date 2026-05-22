//! Persisted browser automation session snapshot.

use sea_orm::entity::prelude::*;
use serde::{Deserialize, Serialize};

#[derive(Clone, Debug, PartialEq, DeriveEntityModel, Serialize, Deserialize)]
#[sea_orm(table_name = "browser_sessions")]
pub struct Model {
    #[sea_orm(primary_key, auto_increment = false)]
    pub id: String,
    pub status: String,
    pub task_description: String,
    pub channel: String,
    #[sea_orm(nullable)]
    pub chat_id: Option<String>,
    #[sea_orm(nullable)]
    pub profile_id: Option<String>,
    #[sea_orm(nullable)]
    pub profile_name: Option<String>,
    #[sea_orm(nullable)]
    pub status_detail: Option<String>,
    pub action_history_json: String,
    pub created_at: String,
    pub updated_at: String,
}

#[allow(dead_code)]
#[derive(Copy, Clone, Debug, EnumIter, DeriveRelation)]
pub enum Relation {}

impl ActiveModelBehavior for ActiveModel {}
