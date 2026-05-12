use super::*;

/// Streaming events for real-time UI updates
#[derive(Debug, Clone)]
pub enum StreamEvent {
    RunStarted {
        run_id: String,
        flow_kind: String,
        origin: String,
        conversation_id: Option<String>,
        trace_id: Option<String>,
        resumed: bool,
    },
    ChatTaskStarted {
        task_id: String,
        description: String,
        work_type: String,
        conversation_id: Option<String>,
    },
    Token(String),
    /// Periodic heartbeat during long waits (e.g., non-streaming fallback)
    Thinking(String),
    /// Live model/planner reasoning preview. This is distinct from assistant
    /// response tokens so clients can render it as transient progress text.
    ReasoningDelta {
        phase: String,
        content_delta: String,
        done: bool,
    },
    ToolStart {
        name: String,
        payload: Option<serde_json::Value>,
    },
    ToolProgress {
        name: String,
        content: String,
        payload: Option<serde_json::Value>,
    },
    ToolResult {
        name: String,
        content: String,
    },
    /// Structured plan for the current execution branch
    #[allow(dead_code)]
    PlanGenerated {
        plan: crate::core::ExecutionPlan,
    },
    /// Status update for a single plan step
    PlanStepUpdate {
        plan_id: String,
        revision: u32,
        step_id: usize,
        step_title: Option<String>,
        status: PlanStepStatus,
        detail: Option<String>,
        substeps: Option<Vec<PlanSubstep>>,
    },
}

pub(crate) fn queue_stream_event(
    token_tx: &tokio::sync::mpsc::Sender<StreamEvent>,
    event: StreamEvent,
) {
    match token_tx.try_send(event) {
        Ok(_) => {}
        Err(tokio::sync::mpsc::error::TrySendError::Closed(_)) => {}
        Err(tokio::sync::mpsc::error::TrySendError::Full(event)) => {
            let fallback_tx = token_tx.clone();
            crate::spawn_logged!("src/core/agent.rs:10544", async move {
                let _ = fallback_tx.send(event).await;
            });
        }
    }
}
