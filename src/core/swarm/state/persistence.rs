use crate::core::model::llm::LlmProvider;
use crate::core::orchestration::orchestra::SubAgentType;
use crate::core::swarm::{AgentCapability, SpecialistConfig};
use crate::storage::entities::swarm_agent;

#[derive(Debug, Clone, serde::Serialize, serde::Deserialize, PartialEq, Eq, Default)]
pub struct AgentAccessScope {
    #[serde(default)]
    pub mcp_server_ids: Vec<String>,
    #[serde(default)]
    pub ssh_connection_names: Vec<String>,
    #[serde(default)]
    pub custom_api_ids: Vec<String>,
    #[serde(default)]
    pub integration_ids: Vec<String>,
    #[serde(default)]
    pub extension_pack_ids: Vec<String>,
    #[serde(default)]
    pub channel_ids: Vec<String>,
    #[serde(default)]
    pub approved_permission_ids: Vec<String>,
}

impl AgentAccessScope {
    pub fn normalized(mut self) -> Self {
        self.mcp_server_ids = normalize_string_list(&self.mcp_server_ids);
        self.ssh_connection_names = normalize_string_list(&self.ssh_connection_names);
        self.custom_api_ids = normalize_string_list(&self.custom_api_ids);
        self.integration_ids = normalize_string_list(&self.integration_ids);
        self.extension_pack_ids = normalize_string_list(&self.extension_pack_ids);
        self.channel_ids = normalize_string_list(&self.channel_ids);
        self.approved_permission_ids = normalize_string_list(&self.approved_permission_ids);
        self
    }
}

fn normalize_string_list(values: &[String]) -> Vec<String> {
    let mut seen = std::collections::BTreeSet::new();
    for value in values {
        let trimmed = value.trim();
        if !trimmed.is_empty() {
            seen.insert(trimmed.to_string());
        }
    }
    seen.into_iter().collect()
}

pub fn parse_access_scope(raw: Option<&str>) -> AgentAccessScope {
    raw.and_then(|value| serde_json::from_str::<AgentAccessScope>(value).ok())
        .unwrap_or_default()
        .normalized()
}

pub fn access_scope_to_json(scope: &AgentAccessScope) -> String {
    serde_json::to_string(&scope.clone().normalized()).unwrap_or_else(|_| "{}".to_string())
}

fn capability_from_text(text: &str) -> Option<AgentCapability> {
    let trimmed = text.trim();
    if trimmed.is_empty() {
        return None;
    }
    Some(AgentCapability {
        name: trimmed.to_string(),
        description: trimmed.to_string(),
        keywords: trimmed
            .split_whitespace()
            .map(|word| word.to_ascii_lowercase())
            .collect(),
    })
}

pub fn capability_strings_to_models(values: &[String]) -> Vec<AgentCapability> {
    values
        .iter()
        .filter_map(|value| capability_from_text(value))
        .collect()
}

pub fn capability_models_to_strings(values: &[AgentCapability]) -> Vec<String> {
    values
        .iter()
        .map(|value| value.description.trim())
        .filter(|value| !value.is_empty())
        .map(str::to_string)
        .collect()
}

pub fn parse_agent_type(agent_type: &str, custom_instructions: Option<&str>) -> SubAgentType {
    match agent_type.trim().to_ascii_lowercase().as_str() {
        "researcher" => SubAgentType::Researcher,
        "coder" => SubAgentType::Coder,
        "analyst" => SubAgentType::Analyst,
        "writer" => SubAgentType::Writer,
        "validator" => SubAgentType::Validator,
        "planner" => SubAgentType::Planner,
        other => SubAgentType::Custom {
            name: if other.is_empty() {
                "Custom".to_string()
            } else {
                agent_type.trim().to_string()
            },
            instructions: custom_instructions.unwrap_or_default().to_string(),
        },
    }
}

pub fn parse_capabilities(raw: &str) -> Vec<AgentCapability> {
    let trimmed = raw.trim();
    if trimmed.is_empty() {
        return Vec::new();
    }

    if let Ok(values) = serde_json::from_str::<Vec<String>>(trimmed) {
        return capability_strings_to_models(&values);
    }

    if let Ok(values) = serde_json::from_str::<Vec<serde_json::Value>>(trimmed) {
        return values
            .iter()
            .filter_map(|value| {
                if let Some(text) = value.as_str() {
                    return capability_from_text(text);
                }
                let name = value
                    .get("name")
                    .and_then(|item| item.as_str())
                    .or_else(|| value.get("description").and_then(|item| item.as_str()))
                    .unwrap_or_default();
                capability_from_text(name)
            })
            .collect();
    }

    trimmed
        .split(',')
        .filter_map(capability_from_text)
        .collect()
}

pub fn parse_llm_provider(raw: &str, fallback: &LlmProvider) -> LlmProvider {
    serde_json::from_str::<LlmProvider>(raw)
        .ok()
        .unwrap_or_else(|| fallback.clone())
}

pub fn specialist_config_from_storage_model(
    agent: &swarm_agent::Model,
    fallback_provider: &LlmProvider,
) -> SpecialistConfig {
    SpecialistConfig {
        id: Some(agent.id.clone()),
        name: agent.name.clone(),
        agent_type: parse_agent_type(&agent.agent_type, agent.system_prompt.as_deref()),
        llm_provider: parse_llm_provider(&agent.llm_provider, fallback_provider),
        system_prompt_override: agent.system_prompt.clone(),
        max_memory_retrieval: 3,
        capabilities: parse_capabilities(&agent.capabilities),
        access_scope: parse_access_scope(Some(&agent.access_scope)),
        enabled: agent.enabled != 0,
    }
}
