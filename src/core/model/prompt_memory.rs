use chrono::{DateTime, Utc};

#[derive(Debug, Clone)]
pub struct PromptMemory {
    pub memory_type: String,
    pub content: String,
    pub timestamp: DateTime<Utc>,
    pub relevance_score: f32,
    pub importance: f32,
    pub final_score: f32,
}
