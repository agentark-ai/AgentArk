//! Public DTOs and value-types for the filesystem-backed ArkOrbit subsystem.

use serde::{Deserialize, Serialize};

/// Public DTO for an orbit canvas. Source of truth is
/// `<DATA_DIR>/arkorbit/L2/orbits/<id>/orbit.json`.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Orbit {
    pub id: String,
    #[serde(default)]
    pub user_id: String,
    pub name: String,
    #[serde(default)]
    pub is_default: bool,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub icon: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub color: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub agent_instructions: Option<String>,
    pub created_at: String,
    pub updated_at: String,
}

/// Manifest persisted in each orbit directory.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct OrbitManifest {
    pub id: String,
    #[serde(default)]
    pub user_id: String,
    pub name: String,
    #[serde(default)]
    pub is_default: bool,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub icon: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub color: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub agent_instructions: Option<String>,
    pub created_at: String,
    pub updated_at: String,
}

impl From<OrbitManifest> for Orbit {
    fn from(value: OrbitManifest) -> Self {
        Self {
            id: value.id,
            user_id: value.user_id,
            name: value.name,
            is_default: value.is_default,
            icon: value.icon,
            color: value.color,
            agent_instructions: value.agent_instructions,
            created_at: value.created_at,
            updated_at: value.updated_at,
        }
    }
}

impl From<&Orbit> for OrbitManifest {
    fn from(value: &Orbit) -> Self {
        Self {
            id: value.id.clone(),
            user_id: value.user_id.clone(),
            name: value.name.clone(),
            is_default: value.is_default,
            icon: value.icon.clone(),
            color: value.color.clone(),
            agent_instructions: value.agent_instructions.clone(),
            created_at: value.created_at.clone(),
            updated_at: value.updated_at.clone(),
        }
    }
}

#[derive(Debug, Clone, Default)]
pub struct OrbitUpdate {
    pub name: Option<String>,
    pub icon: Option<Option<String>>,
    pub color: Option<Option<String>>,
    pub agent_instructions: Option<Option<String>>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct OrbitFileEntry {
    pub path: String,
    pub bytes: u64,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct OrbitChatMessage {
    pub id: String,
    pub role: String,
    pub content: String,
    pub created_at: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub model: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub input_tokens: Option<u64>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub output_tokens: Option<u64>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub total_tokens: Option<u64>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub cost_usd: Option<f64>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub estimated: Option<bool>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub duration_ms: Option<u64>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub time_to_first_token_ms: Option<u64>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct OrbitChatTranscriptSummary {
    pub id: String,
    pub title: String,
    pub created_at: String,
    pub updated_at: String,
    pub message_count: usize,
    #[serde(default)]
    pub current: bool,
}
