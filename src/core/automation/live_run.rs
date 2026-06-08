use crate::storage::Storage;
use serde::{Deserialize, Serialize};
use std::{
    collections::{HashMap, VecDeque},
    sync::Arc,
    time::{Duration, Instant},
};
use tokio::sync::{broadcast, Mutex};

const LIVE_RUN_REPLAY_LIMIT: usize = 12_000;
const LIVE_RUN_CHANNEL_CAPACITY: usize = 1_024;
const LIVE_RUN_COMPLETED_TTL: Duration = Duration::from_secs(45 * 60);
const LIVE_RUN_ACTIVE_IDLE_TTL: Duration = Duration::from_secs(2 * 60 * 60);

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

pub type LiveRunSubscription = (Vec<RunEvent>, Option<broadcast::Receiver<RunEvent>>);

struct LiveRunEntry {
    tx: broadcast::Sender<RunEvent>,
    replay: VecDeque<RunEvent>,
    next_seq: u64,
    completed: bool,
    updated_at: Instant,
}

pub struct LiveRunRegistry {
    storage: Option<Storage>,
    entries: Arc<Mutex<HashMap<String, LiveRunEntry>>>,
}

fn should_persist_run_event(event: &RunEvent) -> bool {
    if event.kind == "token" {
        return false;
    }
    if event.kind == "thinking" {
        let stream_key = event
            .payload
            .get("__streamKey")
            .and_then(|value| value.as_str())
            .unwrap_or_default();
        let step_type = event
            .payload
            .get("step_type")
            .and_then(|value| value.as_str())
            .unwrap_or_default();
        if stream_key == "public-thinking" || step_type == "heartbeat" {
            return false;
        }
    }
    true
}

fn run_event_checkpoint(event: &RunEvent) -> Option<crate::core::ExecutionCheckpoint> {
    if !should_persist_run_event(event) {
        return None;
    }
    let payload = serde_json::to_string(event).ok()?;
    Some(crate::core::ExecutionCheckpoint {
        run_id: event.run_id.clone(),
        sequence_no: event.seq.min(u32::MAX as u64) as u32,
        stage: event.kind.clone(),
        payload,
        created_at: event.ts.clone(),
    })
}

impl LiveRunRegistry {
    pub fn new(storage: Option<Storage>) -> Self {
        Self {
            storage,
            entries: Arc::new(Mutex::new(HashMap::new())),
        }
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

    pub async fn publish_event(
        &self,
        run_id: &str,
        flow_kind: &str,
        origin: &str,
        kind: &str,
        priority: RunEventPriority,
        stage: Option<String>,
        payload: serde_json::Value,
    ) -> Option<RunEvent> {
        let run_id = run_id.trim();
        if run_id.is_empty() || kind.trim().is_empty() {
            return None;
        }

        let event = {
            let mut entries = self.entries.lock().await;
            Self::cleanup_locked(&mut entries);
            let entry = entries.entry(run_id.to_string()).or_insert_with(|| {
                let (tx, _) = broadcast::channel(LIVE_RUN_CHANNEL_CAPACITY);
                LiveRunEntry {
                    tx,
                    replay: VecDeque::new(),
                    next_seq: 1,
                    completed: false,
                    updated_at: Instant::now(),
                }
            });

            let event = RunEvent {
                run_id: run_id.to_string(),
                seq: entry.next_seq,
                ts: chrono::Utc::now().to_rfc3339(),
                flow_kind: flow_kind.to_string(),
                origin: origin.to_string(),
                kind: kind.to_string(),
                priority,
                stage,
                payload,
            };
            entry.next_seq = entry.next_seq.saturating_add(1);
            if entry.replay.len() >= LIVE_RUN_REPLAY_LIMIT {
                entry.replay.pop_front();
            }
            entry.replay.push_back(event.clone());
            entry.updated_at = Instant::now();
            if event.kind == "done" {
                entry.completed = true;
            }
            let _ = entry.tx.send(event.clone());
            event
        };

        if let (Some(storage), Some(checkpoint)) =
            (self.storage.clone(), run_event_checkpoint(&event))
        {
            crate::spawn_logged!("src/core/live_run.rs:persist-run-event", async move {
                if let Err(error) = storage.append_execution_checkpoint(&checkpoint).await {
                    tracing::warn!(
                        "Failed to persist live run event {}#{}: {}",
                        checkpoint.run_id,
                        checkpoint.sequence_no,
                        error
                    );
                }
            });
        }
        Some(event)
    }

    pub async fn subscribe(
        &self,
        run_id: &str,
        since_seq: Option<u64>,
    ) -> anyhow::Result<Option<LiveRunSubscription>> {
        let persisted = self.load_persisted_events(run_id).await.unwrap_or_default();
        let min_seq = since_seq.unwrap_or(0);

        let mut entries = self.entries.lock().await;
        Self::cleanup_locked(&mut entries);
        if let Some(entry) = entries.get(run_id) {
            let rx = if entry.completed {
                None
            } else {
                Some(entry.tx.subscribe())
            };
            let mut replay = persisted
                .into_iter()
                .chain(entry.replay.iter().cloned())
                .filter(|event| event.seq > min_seq)
                .collect::<Vec<_>>();
            replay.sort_by_key(|event| event.seq);
            replay.dedup_by_key(|event| event.seq);
            return Ok(Some((replay, rx)));
        }
        drop(entries);

        let replay = persisted
            .into_iter()
            .filter(|event| event.seq > min_seq)
            .collect::<Vec<_>>();
        if replay.is_empty() {
            Ok(None)
        } else {
            Ok(Some((replay, None)))
        }
    }

    fn cleanup_locked(entries: &mut HashMap<String, LiveRunEntry>) {
        let now = Instant::now();
        entries.retain(|_, entry| {
            let ttl = if entry.completed {
                LIVE_RUN_COMPLETED_TTL
            } else {
                LIVE_RUN_ACTIVE_IDLE_TTL
            };
            now.duration_since(entry.updated_at) <= ttl
        });
    }
}
