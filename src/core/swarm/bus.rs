//! In-process message bus using tokio channels

use super::agent_trait::*;
use std::collections::HashMap;
use std::sync::Arc;
use tokio::sync::{broadcast, mpsc, RwLock};

/// System-wide events broadcast to all agents
#[derive(Debug, Clone, serde::Serialize, serde::Deserialize)]
pub enum SwarmEvent {
    AgentRegistered(AgentInfo),
    AgentUnregistered(AgentId),
    TaskStarted {
        task_id: uuid::Uuid,
        agent_id: AgentId,
    },
    TaskCompleted {
        task_id: uuid::Uuid,
        agent_id: AgentId,
        success: bool,
    },
}

/// Central message bus for inter-agent communication
#[derive(Clone)]
pub struct MessageBus {
    mailboxes: Arc<RwLock<HashMap<AgentId, mpsc::Sender<SwarmMessage>>>>,
    event_tx: broadcast::Sender<SwarmEvent>,
}

impl MessageBus {
    pub fn new() -> Self {
        let (event_tx, _) = broadcast::channel(256);
        Self {
            mailboxes: Arc::new(RwLock::new(HashMap::new())),
            event_tx,
        }
    }

    /// Create a mailbox for an agent. Returns the receiver.
    pub async fn create_mailbox(&self, agent_id: AgentId) -> mpsc::Receiver<SwarmMessage> {
        let (tx, rx) = mpsc::channel(64);
        self.mailboxes.write().await.insert(agent_id, tx);
        rx
    }

    /// Send a directed message to a specific agent
    #[cfg(test)]
    pub async fn send(&self, message: SwarmMessage) -> anyhow::Result<()> {
        let mailboxes = self.mailboxes.read().await;
        if let Some(tx) = mailboxes.get(&message.to) {
            tx.send(message)
                .await
                .map_err(|e| anyhow::anyhow!("Send failed: {}", e))
        } else {
            Err(anyhow::anyhow!("Agent not found: {}", message.to))
        }
    }

    /// Broadcast a system event
    pub fn broadcast(&self, event: SwarmEvent) {
        let _ = self.event_tx.send(event);
    }

    /// Subscribe to system events
    #[cfg(test)]
    pub fn subscribe_events(&self) -> broadcast::Receiver<SwarmEvent> {
        self.event_tx.subscribe()
    }

    /// Remove a mailbox
    pub async fn remove_mailbox(&self, agent_id: &AgentId) {
        self.mailboxes.write().await.remove(agent_id);
    }
}
