//! User preference entity (separate from episodic/semantic memory)

use sea_orm::entity::prelude::*;
use serde::{Deserialize, Serialize};

#[derive(Clone, Debug, PartialEq, DeriveEntityModel, Serialize, Deserialize)]
#[sea_orm(table_name = "user_preferences")]
pub struct Model {
    #[sea_orm(primary_key, auto_increment = false)]
    pub id: String,
    pub key: String,
    pub value: String,
    pub sensitivity: String,
    pub confidence: f32,
    pub source: Option<String>,
    pub project_id: Option<String>,
    pub created_at: String,
    pub updated_at: String,
}

#[derive(Copy, Clone, Debug, EnumIter, DeriveRelation)]
pub enum Relation {}

impl ActiveModelBehavior for ActiveModel {}

pub const SENSITIVITY_PROMPT_SAFE: &str = "prompt_safe";
pub const SENSITIVITY_PERSONAL_IDENTIFIER: &str = "personal_identifier";
pub const SENSITIVITY_SENSITIVE: &str = "sensitive";
pub const SENSITIVITY_CRISIS_SENSITIVE: &str = "crisis_sensitive";

#[derive(Copy, Clone, Debug, PartialEq, Eq, PartialOrd, Ord)]
pub enum MemorySensitivity {
    PromptSafe,
    PersonalIdentifier,
    Sensitive,
    CrisisSensitive,
}

impl MemorySensitivity {
    pub fn as_str(self) -> &'static str {
        match self {
            Self::PromptSafe => SENSITIVITY_PROMPT_SAFE,
            Self::PersonalIdentifier => SENSITIVITY_PERSONAL_IDENTIFIER,
            Self::Sensitive => SENSITIVITY_SENSITIVE,
            Self::CrisisSensitive => SENSITIVITY_CRISIS_SENSITIVE,
        }
    }
}

pub fn normalize_memory_sensitivity(raw: Option<&str>) -> Option<MemorySensitivity> {
    match raw?
        .trim()
        .to_ascii_lowercase()
        .replace(['-', ' '], "_")
        .as_str()
    {
        "prompt_safe" | "safe" | "normal" => Some(MemorySensitivity::PromptSafe),
        "personal_identifier" | "identifier" | "direct_identifier" | "pii" => {
            Some(MemorySensitivity::PersonalIdentifier)
        }
        "sensitive" | "private" => Some(MemorySensitivity::Sensitive),
        "crisis_sensitive" | "crisis" | "safety_sensitive" => {
            Some(MemorySensitivity::CrisisSensitive)
        }
        _ => None,
    }
}

pub fn classify_user_preference_sensitivity(key: &str, value: &str) -> MemorySensitivity {
    let _ = (key, value);
    MemorySensitivity::Sensitive
}

pub fn classify_saved_memory_sensitivity(
    key: Option<&str>,
    value: &str,
    kind: Option<&str>,
) -> MemorySensitivity {
    let key = key.unwrap_or_default().trim().to_ascii_lowercase();
    let kind = kind.unwrap_or_default().trim().to_ascii_lowercase();
    let value = value.trim();
    if value.is_empty() {
        return MemorySensitivity::Sensitive;
    }
    if matches!(
        kind.as_str(),
        "identity" | "contact" | "location" | "personal_identifier"
    ) || key.starts_with("user_")
    {
        return MemorySensitivity::PersonalIdentifier;
    }
    if matches!(
        kind.as_str(),
        "preference" | "workflow" | "constraint" | "personal_fact" | "rule"
    ) || key.starts_with("likes_")
        || key.starts_with("dislikes_")
        || key.starts_with("rule_")
    {
        return MemorySensitivity::PromptSafe;
    }
    MemorySensitivity::Sensitive
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn saved_memory_sensitivity_maps_structured_identity_to_prompt_usable_identifier() {
        assert_eq!(
            classify_saved_memory_sensitivity(Some("user_first_name"), "Debanka", None),
            MemorySensitivity::PersonalIdentifier
        );
        assert_eq!(
            classify_saved_memory_sensitivity(None, "Debanka", Some("identity")),
            MemorySensitivity::PersonalIdentifier
        );
    }
}
