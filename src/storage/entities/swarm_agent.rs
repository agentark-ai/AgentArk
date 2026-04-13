//! Swarm agent entity for persistent agent storage

use sea_orm::entity::prelude::*;
use serde::{Deserialize, Serialize};

#[derive(Clone, Debug, PartialEq, DeriveEntityModel, Serialize, Deserialize)]
#[sea_orm(table_name = "swarm_agents")]
pub struct Model {
    #[sea_orm(primary_key, auto_increment = false)]
    pub id: String,
    pub name: String,
    pub agent_type: String,
    pub llm_provider: String,
    pub capabilities: String,
    #[sea_orm(nullable)]
    pub system_prompt: Option<String>,
    pub access_scope: String,
    #[sea_orm(column_type = "Integer")]
    pub enabled: i32,
    pub created_at: String,
}

#[allow(dead_code)]
#[derive(Copy, Clone, Debug, EnumIter, DeriveRelation)]
pub enum Relation {}

impl ActiveModelBehavior for ActiveModel {}
