use serde::{Deserialize, Serialize};
use std::collections::{HashMap, VecDeque};
use tokio::sync::RwLock;

#[derive(Debug, Clone, Serialize, Deserialize, Default)]
pub struct SwarmActivityAgent {
    pub id: String,
    pub agent_name: String,
    pub agent_role: String,
    pub model_name: String,
    pub task: String,
    pub status: String,
    pub summary: String,
    pub latest_update: String,
    pub is_specialist: bool,
    #[serde(default)]
    pub depends_on: Vec<usize>,
    #[serde(default)]
    pub started_at: Option<String>,
    #[serde(default)]
    pub completed_at: Option<String>,
    #[serde(default)]
    pub updated_at: String,
    #[serde(default)]
    pub elapsed_ms: Option<u64>,
}

#[derive(Debug, Clone, Serialize, Deserialize, Default)]
pub struct SwarmActivityRun {
    pub id: String,
    #[serde(default)]
    pub conversation_id: Option<String>,
    #[serde(default)]
    pub channel: Option<String>,
    pub request: String,
    pub status: String,
    pub summary: String,
    pub started_at: String,
    pub updated_at: String,
    #[serde(default)]
    pub completed_at: Option<String>,
    pub agent_count: usize,
    #[serde(default)]
    pub agents: Vec<SwarmActivityAgent>,
}

impl SwarmActivityRun {
    pub fn active_agent_count(&self) -> usize {
        self.agents
            .iter()
            .filter(|agent| {
                matches!(
                    agent.status.as_str(),
                    "assigned" | "running" | "synthesizing"
                )
            })
            .count()
    }
}

pub struct SwarmActivityTracker {
    active: RwLock<HashMap<String, SwarmActivityRun>>,
    recent: RwLock<VecDeque<SwarmActivityRun>>,
    max_recent: usize,
}

impl SwarmActivityTracker {
    pub fn new(max_recent: usize) -> Self {
        Self {
            active: RwLock::new(HashMap::new()),
            recent: RwLock::new(VecDeque::new()),
            max_recent: max_recent.max(20),
        }
    }

    pub async fn start_run(
        &self,
        id: &str,
        request: &str,
        conversation_id: Option<&str>,
        channel: Option<&str>,
        agent_count: usize,
    ) {
        let now = chrono::Utc::now().to_rfc3339();
        {
            let mut recent = self.recent.write().await;
            recent.retain(|run| run.id != id);
        }
        let mut active = self.active.write().await;
        active.insert(
            id.to_string(),
            SwarmActivityRun {
                id: id.to_string(),
                conversation_id: conversation_id.map(str::to_string),
                channel: channel.map(str::to_string),
                request: request.to_string(),
                status: "running".to_string(),
                summary: "Preparing delegated agents.".to_string(),
                started_at: now.clone(),
                updated_at: now,
                completed_at: None,
                agent_count,
                agents: Vec::new(),
            },
        );
    }

    pub async fn upsert_agent(&self, run_id: &str, agent: SwarmActivityAgent) {
        let mut active = self.active.write().await;
        let Some(run) = active.get_mut(run_id) else {
            return;
        };
        run.updated_at = chrono::Utc::now().to_rfc3339();
        if run.agent_count < run.agents.len().max(1) {
            run.agent_count = run.agents.len();
        }
        if let Some(existing) = run.agents.iter_mut().find(|item| item.id == agent.id) {
            *existing = agent;
        } else {
            run.agents.push(agent);
            run.agent_count = run.agent_count.max(run.agents.len());
        }
    }

    pub async fn update_agent(
        &self,
        run_id: &str,
        agent_id: &str,
        status: &str,
        latest_update: &str,
        summary: Option<&str>,
        elapsed_ms: Option<u64>,
    ) {
        let now = chrono::Utc::now().to_rfc3339();
        let mut active = self.active.write().await;
        let Some(run) = active.get_mut(run_id) else {
            return;
        };
        run.updated_at = now.clone();
        if let Some(agent) = run.agents.iter_mut().find(|item| item.id == agent_id) {
            agent.status = status.to_string();
            if !latest_update.trim().is_empty() {
                agent.latest_update = latest_update.to_string();
            }
            if let Some(value) = summary.filter(|value| !value.trim().is_empty()) {
                agent.summary = value.to_string();
            }
            if agent.started_at.is_none()
                && matches!(status, "running" | "completed" | "failed" | "interrupted")
            {
                agent.started_at = Some(now.clone());
            }
            if matches!(status, "completed" | "failed" | "interrupted") {
                agent.completed_at = Some(now.clone());
            }
            agent.updated_at = now;
            if elapsed_ms.is_some() {
                agent.elapsed_ms = elapsed_ms;
            }
        }
    }

    pub async fn update_run_status(&self, run_id: &str, status: &str, summary: &str) {
        let mut active = self.active.write().await;
        let Some(run) = active.get_mut(run_id) else {
            return;
        };
        run.status = status.to_string();
        if !summary.trim().is_empty() {
            run.summary = summary.to_string();
        }
        run.updated_at = chrono::Utc::now().to_rfc3339();
    }

    pub async fn complete_run(&self, run_id: &str, status: &str, summary: &str) {
        let run = {
            let mut active = self.active.write().await;
            let Some(mut run) = active.remove(run_id) else {
                return;
            };
            let now = chrono::Utc::now().to_rfc3339();
            run.status = status.to_string();
            if !summary.trim().is_empty() {
                run.summary = summary.to_string();
            }
            run.updated_at = now.clone();
            run.completed_at = Some(now.clone());
            for agent in &mut run.agents {
                if matches!(
                    agent.status.as_str(),
                    "assigned" | "running" | "synthesizing"
                ) {
                    agent.status = if status == "interrupted" {
                        "interrupted".to_string()
                    } else if status == "failed" {
                        "failed".to_string()
                    } else {
                        "completed".to_string()
                    };
                    agent.completed_at = Some(now.clone());
                    agent.updated_at = now.clone();
                }
            }
            run
        };

        let mut recent = self.recent.write().await;
        recent.retain(|item| item.id != run.id);
        recent.push_front(run);
        while recent.len() > self.max_recent {
            recent.pop_back();
        }
    }

    pub async fn interrupt_run(&self, run_id: &str, summary: &str) {
        self.complete_run(run_id, "interrupted", summary).await;
    }

    pub async fn active_runs(&self) -> Vec<SwarmActivityRun> {
        let active = self.active.read().await;
        let mut runs: Vec<SwarmActivityRun> = active.values().cloned().collect();
        runs.sort_by(|left, right| right.updated_at.cmp(&left.updated_at));
        runs
    }

    pub async fn recent_runs(&self, limit: usize) -> Vec<SwarmActivityRun> {
        let recent = self.recent.read().await;
        recent.iter().take(limit).cloned().collect()
    }
}
