//! Agent registry for capability discovery

use super::agent_trait::*;
use std::collections::HashMap;
use std::sync::Arc;
use tokio::sync::RwLock;

/// Registry of all agents in the swarm
#[derive(Clone)]
pub struct AgentRegistry {
    agents: Arc<RwLock<HashMap<AgentId, AgentInfo>>>,
}

impl AgentRegistry {
    pub fn new() -> Self {
        Self {
            agents: Arc::new(RwLock::new(HashMap::new())),
        }
    }

    pub async fn register(&self, info: AgentInfo) {
        self.agents.write().await.insert(info.id.clone(), info);
    }

    pub async fn unregister(&self, id: &AgentId) {
        self.agents.write().await.remove(id);
    }

    pub async fn list(&self) -> Vec<AgentInfo> {
        self.agents.read().await.values().cloned().collect()
    }

    #[cfg(test)]
    pub async fn get(&self, id: &AgentId) -> Option<AgentInfo> {
        self.agents.read().await.get(id).cloned()
    }

    #[cfg(test)]
    pub async fn find_by_capability(&self, capability: &str) -> Vec<AgentInfo> {
        let cap_lower = capability.to_lowercase();
        self.agents
            .read()
            .await
            .values()
            .filter(|info| {
                info.capabilities.iter().any(|c| {
                    c.name.to_lowercase().contains(&cap_lower)
                        || c.keywords
                            .iter()
                            .any(|k| k.to_lowercase().contains(&cap_lower))
                })
            })
            .cloned()
            .collect()
    }

    pub async fn update_status(&self, id: &AgentId, status: AgentStatus) {
        if let Some(info) = self.agents.write().await.get_mut(id) {
            info.status = status;
        }
    }

    #[cfg(test)]
    pub async fn count(&self) -> usize {
        self.agents.read().await.len()
    }

    #[cfg(test)]
    pub async fn active_count(&self) -> usize {
        self.agents
            .read()
            .await
            .values()
            .filter(|i| i.status == AgentStatus::Busy)
            .count()
    }
}
