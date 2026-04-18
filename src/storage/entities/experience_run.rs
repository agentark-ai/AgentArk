//! Learned experience run entity for the Postgres-native experience graph.

use sea_orm::entity::prelude::*;
use serde::{Deserialize, Serialize};

#[derive(Clone, Debug, PartialEq, DeriveEntityModel, Serialize, Deserialize)]
#[sea_orm(table_name = "experience_runs")]
pub struct Model {
    #[sea_orm(primary_key, auto_increment = false)]
    pub id: String,
    #[sea_orm(nullable)]
    pub execution_run_id: Option<String>,
    #[sea_orm(nullable)]
    pub trace_id: Option<String>,
    #[sea_orm(nullable)]
    pub conversation_id: Option<String>,
    #[sea_orm(nullable)]
    pub project_id: Option<String>,
    pub channel: String,
    pub scope: String,
    pub intent_key: String,
    #[sea_orm(nullable)]
    pub task_type: Option<String>,
    #[sea_orm(nullable)]
    pub request_text: Option<String>,
    #[sea_orm(nullable)]
    pub tool_sequence_digest: Option<String>,
    #[sea_orm(column_type = "JsonBinary")]
    pub tool_sequence_json: Json,
    #[sea_orm(nullable)]
    pub strategy_version: Option<String>,
    #[sea_orm(nullable)]
    pub policy_version: Option<String>,
    #[sea_orm(nullable)]
    pub prompt_version: Option<String>,
    #[sea_orm(nullable)]
    pub model_slot: Option<String>,
    pub success_state: String,
    pub correction_state: String,
    #[sea_orm(nullable)]
    pub outcome_summary: Option<String>,
    #[sea_orm(nullable)]
    pub failure_reason: Option<String>,
    #[sea_orm(column_type = "JsonBinary")]
    pub metadata: Json,
    pub consolidated: bool,
    #[sea_orm(nullable)]
    pub accepted_at: Option<String>,
    #[sea_orm(nullable)]
    pub corrected_at: Option<String>,
    pub heuristic_reflected: bool,
    #[sea_orm(nullable)]
    pub heuristic_reflection_status: Option<String>,
    #[sea_orm(nullable)]
    pub heuristic_reflection_attempted_at: Option<String>,
    #[sea_orm(nullable)]
    pub heuristic_reflection_completed_at: Option<String>,
    #[sea_orm(nullable)]
    pub heuristic_lesson_id: Option<String>,
    #[sea_orm(nullable)]
    pub heuristic_reflection_error: Option<String>,
    pub created_at: String,
    pub updated_at: String,
}

#[allow(dead_code)]
#[derive(Copy, Clone, Debug, EnumIter, DeriveRelation)]
pub enum Relation {}

impl ActiveModelBehavior for ActiveModel {}
