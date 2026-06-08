//! Node and device foundation for future companion-device support.
//!
//! This module is intentionally low-risk: it stores JSON payloads in the
//! existing KV store and exposes simple async CRUD helpers. It does not add
//! transport, pairing, or command execution logic.
use anyhow::{Context, Result};
use chrono::Utc;
use serde::{Deserialize, Serialize};
use std::collections::BTreeMap;

use crate::storage::Storage;

const INDEX_KEY: &str = "nodes:index";
const NODE_PREFIX: &str = "nodes:node:";
const HEARTBEAT_PREFIX: &str = "nodes:heartbeat:";
const COMMAND_PREFIX: &str = "nodes:command:";

fn node_key(node_id: &str) -> String {
    format!("{}{}", NODE_PREFIX, node_id.trim())
}

fn heartbeat_key(node_id: &str) -> String {
    format!("{}{}", HEARTBEAT_PREFIX, node_id.trim())
}

fn command_key(node_id: &str) -> String {
    format!("{}{}", COMMAND_PREFIX, node_id.trim())
}

fn normalize_tag_list(values: &[String]) -> Vec<String> {
    let mut out: Vec<String> = values
        .iter()
        .map(|value| value.trim().to_string())
        .filter(|value| !value.is_empty())
        .collect();
    out.sort();
    out.dedup();
    out
}

fn stable_json_map(map: &BTreeMap<String, String>) -> BTreeMap<String, String> {
    map.iter()
        .map(|(key, value)| (key.trim().to_string(), value.trim().to_string()))
        .filter(|(key, _)| !key.is_empty())
        .collect()
}

#[derive(Debug, Clone, Default, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "snake_case")]
pub enum NodeTransportKind {
    #[default]
    Local,
    Node,
    Bridge,
    Cloud,
    Plugin,
}

#[derive(Debug, Clone, Default, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "snake_case")]
pub enum NodeState {
    Planned,
    #[default]
    Paired,
    Online,
    Idle,
    Busy,
    Degraded,
    Offline,
    Revoked,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq, PartialOrd, Ord, Hash)]
#[serde(rename_all = "snake_case")]
pub enum NodeCapability {
    Canvas,
    Camera,
    ScreenCapture,
    ScreenRecord,
    Location,
    Sms,
    Notifications,
    SystemRun,
    BrowserControl,
    Voice,
    Files,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct NodeHeartbeat {
    pub node_id: String,
    pub observed_at: String,
    pub state: NodeState,
    pub transport: NodeTransportKind,
    #[serde(default)]
    pub capabilities: Vec<NodeCapability>,
    #[serde(default)]
    pub metrics: BTreeMap<String, String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub version: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub message: Option<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct NodeCommandLogEntry {
    pub id: String,
    pub node_id: String,
    pub command: String,
    pub requested_at: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub completed_at: Option<String>,
    #[serde(default)]
    pub success: bool,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub exit_code: Option<i32>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub output_preview: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub actor: Option<String>,
    #[serde(default)]
    pub context: BTreeMap<String, String>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct PairedNode {
    pub id: String,
    pub display_name: String,
    pub transport: NodeTransportKind,
    pub state: NodeState,
    #[serde(default)]
    pub capabilities: Vec<NodeCapability>,
    #[serde(default)]
    pub labels: Vec<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub platform: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub owner: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub last_heartbeat_at: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub last_error: Option<String>,
    #[serde(default)]
    pub permissions_granted: usize,
    #[serde(default)]
    pub command_count: usize,
    #[serde(default)]
    pub metadata: BTreeMap<String, String>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct NodeUpsertRequest {
    pub id: String,
    pub display_name: String,
    #[serde(default)]
    pub transport: NodeTransportKind,
    #[serde(default)]
    pub state: NodeState,
    #[serde(default)]
    pub capabilities: Vec<NodeCapability>,
    #[serde(default)]
    pub labels: Vec<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub platform: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub owner: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub metadata: Option<BTreeMap<String, String>>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct NodeHeartbeatRequest {
    pub node_id: String,
    #[serde(default)]
    pub transport: NodeTransportKind,
    #[serde(default)]
    pub state: NodeState,
    #[serde(default)]
    pub capabilities: Vec<NodeCapability>,
    #[serde(default)]
    pub metrics: BTreeMap<String, String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub version: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub message: Option<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct NodeCommandLogRequest {
    pub node_id: String,
    pub command: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub completed_at: Option<String>,
    #[serde(default)]
    pub success: bool,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub exit_code: Option<i32>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub output_preview: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub actor: Option<String>,
    #[serde(default)]
    pub context: BTreeMap<String, String>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct NodeSummary {
    pub total: usize,
    pub paired: usize,
    pub online: usize,
    pub degraded: usize,
    pub offline: usize,
    pub revoked: usize,
    pub capabilities: BTreeMap<String, usize>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct NodeControlPlaneStatus {
    pub generated_at: String,
    pub summary: NodeSummary,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct NodeControlPlaneConfig {
    #[serde(default = "default_namespace")]
    pub namespace: String,
}

fn default_namespace() -> String {
    "nodes".to_string()
}

impl Default for NodeControlPlaneConfig {
    fn default() -> Self {
        Self {
            namespace: default_namespace(),
        }
    }
}

/// KV-backed control plane for paired nodes and companion devices.
#[derive(Clone)]
pub struct NodeControlPlane {
    storage: Storage,
    namespace: String,
}

impl NodeControlPlane {
    pub fn new(storage: Storage) -> Self {
        Self::with_config(storage, NodeControlPlaneConfig::default())
    }

    pub fn with_config(storage: Storage, config: NodeControlPlaneConfig) -> Self {
        let namespace = config.namespace.trim().to_string();
        Self {
            storage,
            namespace: if namespace.is_empty() {
                default_namespace()
            } else {
                namespace
            },
        }
    }

    fn scoped_key(&self, suffix: &str) -> String {
        format!("{}:{}", self.namespace, suffix.trim())
    }

    fn node_index_key(&self) -> String {
        self.scoped_key(INDEX_KEY)
    }

    fn node_key(&self, node_id: &str) -> String {
        self.scoped_key(&node_key(node_id))
    }

    fn heartbeat_key(&self, node_id: &str) -> String {
        self.scoped_key(&heartbeat_key(node_id))
    }

    fn command_key(&self, node_id: &str) -> String {
        self.scoped_key(&command_key(node_id))
    }

    async fn read_index(&self) -> Result<Vec<String>> {
        let ids = self
            .storage
            .get(&self.node_index_key())
            .await?
            .map(|raw| serde_json::from_slice::<Vec<String>>(&raw))
            .transpose()
            .context("failed to decode node index")?
            .unwrap_or_default();
        Ok(ids)
    }

    async fn write_index(&self, ids: &[String]) -> Result<()> {
        let mut normalized: Vec<String> = ids
            .iter()
            .map(|id| id.trim().to_string())
            .filter(|id| !id.is_empty())
            .collect();
        normalized.sort();
        normalized.dedup();
        self.storage
            .set(&self.node_index_key(), &serde_json::to_vec(&normalized)?)
            .await
    }

    async fn read_node(&self, node_id: &str) -> Result<Option<PairedNode>> {
        let Some(raw) = self.storage.get(&self.node_key(node_id)).await? else {
            return Ok(None);
        };
        let node =
            serde_json::from_slice::<PairedNode>(&raw).context("failed to decode node record")?;
        Ok(Some(node))
    }

    async fn write_node(&self, node: &PairedNode) -> Result<()> {
        self.storage
            .set(&self.node_key(&node.id), &serde_json::to_vec(node)?)
            .await
    }

    async fn read_heartbeat(&self, node_id: &str) -> Result<Option<NodeHeartbeat>> {
        let Some(raw) = self.storage.get(&self.heartbeat_key(node_id)).await? else {
            return Ok(None);
        };
        let heartbeat = serde_json::from_slice::<NodeHeartbeat>(&raw)
            .context("failed to decode node heartbeat")?;
        Ok(Some(heartbeat))
    }

    async fn write_heartbeat(&self, heartbeat: &NodeHeartbeat) -> Result<()> {
        self.storage
            .set(
                &self.heartbeat_key(&heartbeat.node_id),
                &serde_json::to_vec(heartbeat)?,
            )
            .await
    }

    async fn read_command_log(&self, node_id: &str) -> Result<Vec<NodeCommandLogEntry>> {
        let entries = self
            .storage
            .get(&self.command_key(node_id))
            .await?
            .map(|raw| serde_json::from_slice::<Vec<NodeCommandLogEntry>>(&raw))
            .transpose()
            .context("failed to decode node command log")?
            .unwrap_or_default();
        Ok(entries)
    }

    async fn write_command_log(
        &self,
        node_id: &str,
        entries: &[NodeCommandLogEntry],
    ) -> Result<()> {
        self.storage
            .set(&self.command_key(node_id), &serde_json::to_vec(entries)?)
            .await
    }

    pub async fn list(&self) -> Result<Vec<PairedNode>> {
        let mut out = Vec::new();
        for node_id in self.read_index().await? {
            if let Some(node) = self.read_node(&node_id).await? {
                out.push(self.decorate_node(node).await?);
            }
        }
        Ok(out)
    }

    pub async fn upsert(&self, request: NodeUpsertRequest) -> Result<PairedNode> {
        let node_id = request.id.trim();
        anyhow::ensure!(!node_id.is_empty(), "node id cannot be empty");
        let now = Utc::now().to_rfc3339();
        let previous = self.read_node(node_id).await?;
        let previous_heartbeat_at = previous
            .as_ref()
            .and_then(|node| node.last_heartbeat_at.clone());
        let previous_error = previous.as_ref().and_then(|node| node.last_error.clone());
        let previous_commands = previous
            .as_ref()
            .map(|node| node.command_count)
            .unwrap_or(0);

        let node = PairedNode {
            id: node_id.to_string(),
            display_name: request.display_name.trim().to_string(),
            transport: request.transport,
            state: request.state,
            capabilities: normalize_capabilities(request.capabilities),
            labels: normalize_tag_list(&request.labels),
            platform: request
                .platform
                .map(|value| value.trim().to_string())
                .filter(|value| !value.is_empty()),
            owner: request
                .owner
                .map(|value| value.trim().to_string())
                .filter(|value| !value.is_empty()),
            last_heartbeat_at: previous_heartbeat_at,
            last_error: previous_error,
            permissions_granted: 0,
            command_count: previous_commands,
            metadata: stable_json_map(&request.metadata.unwrap_or_default()),
        };

        let mut ids = self.read_index().await?;
        if !ids.iter().any(|existing| existing == &node.id) {
            ids.push(node.id.clone());
        }
        self.write_index(&ids).await?;
        self.write_node(&node).await?;

        let heartbeat = NodeHeartbeat {
            node_id: node.id.clone(),
            observed_at: now,
            state: node.state.clone(),
            transport: node.transport.clone(),
            capabilities: node.capabilities.clone(),
            metrics: BTreeMap::new(),
            version: None,
            message: None,
        };
        self.write_heartbeat(&heartbeat).await?;

        self.decorate_node(node).await
    }

    pub async fn revoke(&self, node_id: &str) -> Result<Option<PairedNode>> {
        let node_id = node_id.trim();
        if node_id.is_empty() {
            return Ok(None);
        }
        let mut node = match self.read_node(node_id).await? {
            Some(node) => node,
            None => return Ok(None),
        };
        node.state = NodeState::Revoked;
        node.last_error = Some("revoked".to_string());
        self.write_node(&node).await?;
        self.storage.delete(&self.heartbeat_key(node_id)).await?;
        let node = self.decorate_node(node).await?;
        Ok(Some(node))
    }

    pub async fn heartbeat(&self, request: NodeHeartbeatRequest) -> Result<NodeHeartbeat> {
        let node_id = request.node_id.trim();
        anyhow::ensure!(!node_id.is_empty(), "node id cannot be empty");

        let heartbeat = NodeHeartbeat {
            node_id: node_id.to_string(),
            observed_at: Utc::now().to_rfc3339(),
            state: request.state,
            transport: request.transport,
            capabilities: request.capabilities,
            metrics: stable_json_map(&request.metrics),
            version: request.version,
            message: request.message,
        };
        self.write_heartbeat(&heartbeat).await?;

        let mut node = self
            .read_node(node_id)
            .await?
            .unwrap_or_else(|| PairedNode {
                id: node_id.to_string(),
                display_name: node_id.to_string(),
                transport: heartbeat.transport.clone(),
                state: heartbeat.state.clone(),
                capabilities: heartbeat.capabilities.clone(),
                labels: Vec::new(),
                platform: None,
                owner: None,
                last_heartbeat_at: None,
                last_error: None,
                permissions_granted: 0,
                command_count: 0,
                metadata: BTreeMap::new(),
            });
        node.state = heartbeat.state.clone();
        node.transport = heartbeat.transport.clone();
        node.capabilities = heartbeat.capabilities.clone();
        node.last_heartbeat_at = Some(heartbeat.observed_at.clone());
        node.last_error = heartbeat.message.clone();
        self.write_node(&node).await?;

        let mut ids = self.read_index().await?;
        if !ids.iter().any(|existing| existing == &node.id) {
            ids.push(node.id.clone());
            self.write_index(&ids).await?;
        }

        Ok(heartbeat)
    }

    pub async fn log_command(&self, request: NodeCommandLogRequest) -> Result<NodeCommandLogEntry> {
        let node_id = request.node_id.trim();
        anyhow::ensure!(!node_id.is_empty(), "node id cannot be empty");
        let now = Utc::now().to_rfc3339();
        let entry = NodeCommandLogEntry {
            id: format!("cmd-{}-{}", node_id, now),
            node_id: node_id.to_string(),
            command: request.command.trim().to_string(),
            requested_at: now,
            completed_at: request.completed_at,
            success: request.success,
            exit_code: request.exit_code,
            output_preview: request.output_preview,
            actor: request.actor,
            context: stable_json_map(&request.context),
        };

        let mut entries = self.read_command_log(node_id).await?;
        entries.push(entry.clone());
        self.write_command_log(node_id, &entries).await?;

        if let Some(mut node) = self.read_node(node_id).await? {
            node.command_count = node.command_count.saturating_add(1);
            self.write_node(&node).await?;
        }

        Ok(entry)
    }

    pub async fn list_commands(&self, node_id: &str) -> Result<Vec<NodeCommandLogEntry>> {
        self.read_command_log(node_id).await
    }

    pub async fn status(&self) -> Result<NodeControlPlaneStatus> {
        let nodes = self.list().await?;
        let mut summary = NodeSummary {
            total: nodes.len(),
            paired: 0,
            online: 0,
            degraded: 0,
            offline: 0,
            revoked: 0,
            capabilities: BTreeMap::new(),
        };

        for node in nodes {
            match node.state {
                NodeState::Paired => summary.paired += 1,
                NodeState::Online | NodeState::Idle | NodeState::Busy => summary.online += 1,
                NodeState::Degraded => summary.degraded += 1,
                NodeState::Offline => summary.offline += 1,
                NodeState::Revoked => summary.revoked += 1,
                NodeState::Planned => {}
            }
            for capability in node.capabilities {
                let key = format!("{:?}", capability);
                *summary.capabilities.entry(key).or_insert(0) += 1;
            }
        }

        Ok(NodeControlPlaneStatus {
            generated_at: Utc::now().to_rfc3339(),
            summary,
        })
    }

    async fn decorate_node(&self, mut node: PairedNode) -> Result<PairedNode> {
        if let Some(heartbeat) = self.read_heartbeat(&node.id).await? {
            node.last_heartbeat_at = Some(heartbeat.observed_at);
            node.state = heartbeat.state;
            node.transport = heartbeat.transport;
            node.capabilities = heartbeat.capabilities;
        }
        node.permissions_granted = 0;
        node.command_count = self.read_command_log(&node.id).await?.len();
        Ok(node)
    }
}

fn normalize_capabilities(values: Vec<NodeCapability>) -> Vec<NodeCapability> {
    let mut out = values;
    out.sort();
    out.dedup();
    out
}

#[cfg(test)]
mod tests {
    use super::*;

    fn request(node_id: &str) -> NodeUpsertRequest {
        NodeUpsertRequest {
            id: node_id.to_string(),
            display_name: "Primary Node".to_string(),
            transport: NodeTransportKind::Node,
            state: NodeState::Paired,
            capabilities: vec![NodeCapability::Camera, NodeCapability::SystemRun],
            labels: vec![
                "mobile".to_string(),
                "mobile".to_string(),
                "paired".to_string(),
            ],
            platform: Some("android".to_string()),
            owner: Some("user".to_string()),
            metadata: Some(BTreeMap::from([("region".to_string(), "in".to_string())])),
        }
    }

    #[test]
    fn normalizes_metadata() {
        let labels = normalize_tag_list(&[
            " b ".to_string(),
            "".to_string(),
            "a".to_string(),
            "a".to_string(),
        ]);
        assert_eq!(labels, vec!["a".to_string(), "b".to_string()]);
        let map = stable_json_map(&BTreeMap::from([(" a ".to_string(), " b ".to_string())]));
        assert_eq!(map.get("a").map(String::as_str), Some("b"));
    }

    #[tokio::test]
    async fn status_model_serializes() {
        let status = NodeControlPlaneStatus {
            generated_at: Utc::now().to_rfc3339(),
            summary: NodeSummary {
                total: 1,
                paired: 1,
                online: 0,
                degraded: 0,
                offline: 0,
                revoked: 0,
                capabilities: BTreeMap::new(),
            },
        };
        let json = serde_json::to_string(&status).expect("serialize");
        assert!(json.contains("generated_at"));
    }

    #[test]
    fn request_is_well_formed() {
        let req = request("node-1");
        assert_eq!(req.id, "node-1");
        assert!(req.capabilities.contains(&NodeCapability::Camera));
    }
}
