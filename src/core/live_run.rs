use crate::storage::Storage;
use serde::{Deserialize, Serialize};
use std::collections::{HashMap, VecDeque};
use std::sync::atomic::{AtomicU64, Ordering};
use std::sync::Arc;
use tokio::sync::{broadcast, RwLock};

use super::agent::StreamEvent;
use super::{ExecutionCheckpoint, ExecutionRun, PlanStepStatus};

const LIVE_RUN_EVENT_BUFFER_LIMIT: usize = 4096;

fn now_rfc3339() -> String {
    chrono::Utc::now().to_rfc3339()
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "snake_case")]
pub enum RunEventPriority {
    Critical,
    High,
    Normal,
    Low,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct RunEvent {
    pub run_id: String,
    pub seq: u64,
    pub ts: String,
    pub flow_kind: String,
    pub origin: String,
    pub kind: String,
    pub priority: RunEventPriority,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub stage: Option<String>,
    pub payload: serde_json::Value,
}

pub struct LiveRunJournal {
    run_id: String,
    flow_kind: RwLock<String>,
    origin: RwLock<String>,
    stage: RwLock<Option<String>>,
    next_seq: AtomicU64,
    events: RwLock<VecDeque<RunEvent>>,
    subscribers: broadcast::Sender<RunEvent>,
    storage: Option<Storage>,
}

impl LiveRunJournal {
    pub fn new(
        run_id: String,
        flow_kind: String,
        origin: String,
        stage: Option<String>,
        storage: Option<Storage>,
    ) -> Self {
        let (subscribers, _) = broadcast::channel(512);
        Self {
            run_id,
            flow_kind: RwLock::new(flow_kind),
            origin: RwLock::new(origin),
            stage: RwLock::new(stage),
            next_seq: AtomicU64::new(1),
            events: RwLock::new(VecDeque::with_capacity(LIVE_RUN_EVENT_BUFFER_LIMIT.min(64))),
            subscribers,
            storage,
        }
    }

    async fn update_metadata(
        &self,
        flow_kind: Option<&str>,
        origin: Option<&str>,
        stage: Option<&str>,
    ) {
        if let Some(flow_kind) = flow_kind.filter(|value| !value.trim().is_empty()) {
            *self.flow_kind.write().await = flow_kind.to_string();
        }
        if let Some(origin) = origin.filter(|value| !value.trim().is_empty()) {
            *self.origin.write().await = origin.to_string();
        }
        if let Some(stage) = stage.filter(|value| !value.trim().is_empty()) {
            *self.stage.write().await = Some(stage.to_string());
        }
    }

    pub async fn replay_since(&self, since_seq: Option<u64>) -> Vec<RunEvent> {
        let floor = since_seq.unwrap_or(0);
        self.events
            .read()
            .await
            .iter()
            .filter(|event| event.seq > floor)
            .cloned()
            .collect()
    }

    pub fn subscribe(&self) -> broadcast::Receiver<RunEvent> {
        self.subscribers.subscribe()
    }

    pub async fn publish(
        &self,
        kind: impl Into<String>,
        priority: RunEventPriority,
        payload: serde_json::Value,
        stage_override: Option<String>,
    ) -> RunEvent {
        let seq = self.next_seq.fetch_add(1, Ordering::Relaxed);
        let flow_kind = self.flow_kind.read().await.clone();
        let origin = self.origin.read().await.clone();
        let stage = match stage_override {
            Some(stage) if !stage.trim().is_empty() => Some(stage),
            _ => self.stage.read().await.clone(),
        };
        let event = RunEvent {
            run_id: self.run_id.clone(),
            seq,
            ts: now_rfc3339(),
            flow_kind,
            origin,
            kind: kind.into(),
            priority,
            stage: stage.clone(),
            payload,
        };

        {
            let mut events = self.events.write().await;
            events.push_back(event.clone());
            while events.len() > LIVE_RUN_EVENT_BUFFER_LIMIT {
                events.pop_front();
            }
        }

        let _ = self.subscribers.send(event.clone());

        if let Some(storage) = self.storage.as_ref() {
            let checkpoint = ExecutionCheckpoint {
                run_id: self.run_id.clone(),
                sequence_no: seq.min(i32::MAX as u64) as u32,
                stage: stage.unwrap_or_default(),
                payload: serde_json::to_string(&event).unwrap_or_else(|_| "{}".to_string()),
                created_at: event.ts.clone(),
            };
            if let Err(error) = storage.append_execution_checkpoint(&checkpoint).await {
                tracing::warn!(
                    "Failed to persist live run event {} for run '{}': {}",
                    checkpoint.sequence_no,
                    self.run_id,
                    error
                );
            }
        }

        event
    }
}

pub struct LiveRunRegistry {
    storage: Option<Storage>,
    journals: RwLock<HashMap<String, Arc<LiveRunJournal>>>,
}

impl LiveRunRegistry {
    pub fn new(storage: Option<Storage>) -> Self {
        Self {
            storage,
            journals: RwLock::new(HashMap::new()),
        }
    }

    pub async fn register_run(
        &self,
        run: &ExecutionRun,
        flow_kind: &str,
        origin: &str,
    ) -> Arc<LiveRunJournal> {
        let mut journals = self.journals.write().await;
        if let Some(existing) = journals.get(&run.id) {
            existing
                .update_metadata(Some(flow_kind), Some(origin), Some(&run.current_stage))
                .await;
            return existing.clone();
        }
        let journal = Arc::new(LiveRunJournal::new(
            run.id.clone(),
            flow_kind.to_string(),
            origin.to_string(),
            Some(run.current_stage.clone()),
            self.storage.clone(),
        ));
        journals.insert(run.id.clone(), journal.clone());
        journal
    }

    pub async fn get(&self, run_id: &str) -> Option<Arc<LiveRunJournal>> {
        self.journals.read().await.get(run_id).cloned()
    }

    pub async fn load_persisted_events(&self, run_id: &str) -> anyhow::Result<Vec<RunEvent>> {
        let Some(storage) = self.storage.as_ref() else {
            return Ok(Vec::new());
        };
        let checkpoints = storage.load_execution_checkpoints(run_id).await?;
        Ok(checkpoints
            .into_iter()
            .filter_map(|checkpoint| {
                serde_json::from_str::<RunEvent>(&checkpoint.payload)
                    .ok()
                    .or_else(|| {
                        serde_json::from_str::<serde_json::Value>(&checkpoint.payload)
                            .ok()
                            .map(|payload| RunEvent {
                                run_id: checkpoint.run_id,
                                seq: checkpoint.sequence_no as u64,
                                ts: checkpoint.created_at,
                                flow_kind: "chat".to_string(),
                                origin: String::new(),
                                kind: "checkpoint".to_string(),
                                priority: RunEventPriority::Normal,
                                stage: Some(checkpoint.stage),
                                payload,
                            })
                    })
            })
            .collect())
    }

    pub async fn publish_run_status(
        &self,
        run: &ExecutionRun,
        flow_kind: &str,
        origin: &str,
        payload: serde_json::Value,
    ) {
        let journal = self.register_run(run, flow_kind, origin).await;
        journal
            .publish(
                "run_status",
                RunEventPriority::Critical,
                serde_json::json!({
                    "run_id": run.id.clone(),
                    "run_status": run.status.as_str(),
                    "stage": run.current_stage.clone(),
                    "trace_id": run.trace_id.clone(),
                    "conversation_id": run.conversation_id.clone(),
                    "payload": payload,
                }),
                Some(run.current_stage.clone()),
            )
            .await;
    }

    pub async fn publish_content(
        &self,
        run: &ExecutionRun,
        flow_kind: &str,
        origin: &str,
        payload: serde_json::Value,
    ) {
        let journal = self.register_run(run, flow_kind, origin).await;
        journal
            .publish(
                "content",
                RunEventPriority::Critical,
                payload,
                Some(run.current_stage.clone()),
            )
            .await;
        journal
            .publish(
                "done",
                RunEventPriority::Critical,
                serde_json::json!({
                    "run_id": run.id.clone(),
                    "run_status": run.status.as_str(),
                }),
                Some(run.current_stage.clone()),
            )
            .await;
    }

    pub async fn publish_stream_event(
        &self,
        run_id: &str,
        flow_kind: &str,
        origin: &str,
        stage: Option<&str>,
        event: &StreamEvent,
    ) {
        let Some(journal) = self.get(run_id).await else {
            return;
        };
        journal
            .update_metadata(Some(flow_kind), Some(origin), stage)
            .await;
        let (kind, priority, payload) = stream_event_payload(event);
        journal
            .publish(kind, priority, payload, stage.map(str::to_string))
            .await;
    }
}

fn stream_event_payload(event: &StreamEvent) -> (String, RunEventPriority, serde_json::Value) {
    match event {
        StreamEvent::RunStarted {
            run_id,
            flow_kind,
            origin,
            conversation_id,
            trace_id,
            resumed,
        } => (
            "run_started".to_string(),
            RunEventPriority::Critical,
            serde_json::json!({
                "run_id": run_id,
                "flow_kind": flow_kind,
                "origin": origin,
                "conversation_id": conversation_id,
                "trace_id": trace_id,
                "resumed": resumed,
            }),
        ),
        StreamEvent::Token(content) => (
            "token".to_string(),
            RunEventPriority::Low,
            serde_json::json!({ "content": content }),
        ),
        StreamEvent::Thinking(detail) => (
            "thinking".to_string(),
            RunEventPriority::Low,
            serde_json::json!({ "detail": detail }),
        ),
        StreamEvent::ToolStart { name, payload } => (
            "tool_start".to_string(),
            RunEventPriority::High,
            serde_json::json!({
                "name": name,
                "payload": payload,
            }),
        ),
        StreamEvent::ToolProgress {
            name,
            content,
            payload,
        } => {
            let kind = payload
                .as_ref()
                .and_then(|value| value.get("kind"))
                .and_then(|value| value.as_str())
                .unwrap_or("");
            let priority = if matches!(
                kind,
                "draft_file" | "file_write" | "phase_status" | "console_chunk"
            ) {
                RunEventPriority::High
            } else {
                RunEventPriority::Normal
            };
            (
                "tool_progress".to_string(),
                priority,
                serde_json::json!({
                    "name": name,
                    "content": content,
                    "payload": payload,
                }),
            )
        }
        StreamEvent::ToolResult { name, content } => (
            "tool_result".to_string(),
            RunEventPriority::High,
            serde_json::json!({
                "name": name,
                "content": content,
            }),
        ),
        StreamEvent::PlanGenerated { plan } => (
            "plan_generated".to_string(),
            RunEventPriority::High,
            serde_json::json!({
                "plan": plan,
            }),
        ),
        StreamEvent::PlanRevised { plan, reason } => (
            "plan_revised".to_string(),
            RunEventPriority::High,
            serde_json::json!({
                "plan": plan,
                "reason": reason,
            }),
        ),
        StreamEvent::PlanReadyForConfirmation {
            task_id,
            plan,
            source,
        } => (
            "plan_ready_for_confirmation".to_string(),
            RunEventPriority::Critical,
            serde_json::json!({
                "task_id": task_id,
                "plan": plan,
                "source": source,
            }),
        ),
        StreamEvent::PlanUnavailable { reason } => (
            "plan_unavailable".to_string(),
            RunEventPriority::High,
            serde_json::json!({ "reason": reason }),
        ),
        StreamEvent::PlanStepUpdate {
            plan_id,
            revision,
            step_id,
            step_title,
            status,
            detail,
        } => (
            "plan_step_update".to_string(),
            match status {
                PlanStepStatus::Failed => RunEventPriority::High,
                PlanStepStatus::Running => RunEventPriority::High,
                _ => RunEventPriority::Normal,
            },
            serde_json::json!({
                "plan_id": plan_id,
                "revision": revision,
                "step_id": step_id,
                "step_title": step_title,
                "status": status,
                "detail": detail,
            }),
        ),
    }
}
