use super::ChatAttachmentHint;
use crate::actions::{ActionAuthorizationContext, ActionExecutionSurface};
use serde::{Deserialize, Serialize};
use std::sync::{
    atomic::{AtomicBool, Ordering},
    Arc,
};

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum CallerKind {
    Chat,
    Task,
    Watcher,
    Cron,
    Gateway,
    Companion,
}

impl CallerKind {
    pub fn default_max_turns(self) -> usize {
        match self {
            Self::Chat => 30,
            Self::Task => 100,
            Self::Watcher => 20,
            Self::Cron => 50,
            Self::Gateway => 30,
            Self::Companion => 30,
        }
    }

    pub fn default_streaming(self) -> bool {
        matches!(self, Self::Chat)
    }

    pub fn can_pause_for_approval(self) -> bool {
        matches!(
            self,
            Self::Chat | Self::Task | Self::Gateway | Self::Companion
        )
    }

    pub fn execution_surface(self) -> ActionExecutionSurface {
        match self {
            Self::Chat | Self::Companion | Self::Gateway => ActionExecutionSurface::Chat,
            Self::Task | Self::Watcher | Self::Cron => ActionExecutionSurface::Automation,
        }
    }
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(tag = "role", rename_all = "snake_case")]
pub enum SpineMessage {
    System {
        content: String,
    },
    User {
        content: String,
    },
    Assistant {
        #[serde(default, skip_serializing_if = "Option::is_none")]
        content: Option<String>,
        #[serde(default, skip_serializing_if = "Vec::is_empty")]
        tool_calls: Vec<SpineToolCall>,
    },
    Tool {
        tool_call_id: String,
        content: String,
    },
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub struct SpineToolCall {
    pub id: String,
    pub name: String,
    #[serde(default)]
    pub arguments: serde_json::Value,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub activity_label: Option<String>,
}

#[derive(Debug, Clone, Default)]
pub struct SpineCancelToken {
    cancelled: Arc<AtomicBool>,
}

impl SpineCancelToken {
    pub fn new() -> Self {
        Self::default()
    }

    pub fn is_cancelled(&self) -> bool {
        self.cancelled.load(Ordering::Acquire)
    }
}

#[derive(Debug, Clone)]
pub struct SpineRequest {
    pub messages: Vec<SpineMessage>,
    pub caller_kind: CallerKind,
    pub max_turns: usize,
    pub streaming: bool,
    pub long_running: bool,
    pub cancel_token: SpineCancelToken,
    pub channel: String,
    pub conversation_id: Option<String>,
    pub project_id: Option<String>,
    pub execution_profile: Option<serde_json::Value>,
    pub browser_profile_context: Option<serde_json::Value>,
    pub visual_attachments: Vec<ChatAttachmentHint>,
    pub authorization: ActionAuthorizationContext,
}

impl SpineRequest {
    pub fn new(
        caller_kind: CallerKind,
        messages: Vec<SpineMessage>,
        channel: impl Into<String>,
    ) -> Self {
        let authorization = ActionAuthorizationContext {
            surface: caller_kind.execution_surface(),
            direct_user_intent: matches!(
                caller_kind,
                CallerKind::Chat | CallerKind::Gateway | CallerKind::Companion
            ),
            ..ActionAuthorizationContext::default()
        };
        Self {
            messages,
            caller_kind,
            max_turns: caller_kind.default_max_turns(),
            streaming: caller_kind.default_streaming(),
            long_running: false,
            cancel_token: SpineCancelToken::new(),
            channel: channel.into(),
            conversation_id: None,
            project_id: None,
            execution_profile: None,
            browser_profile_context: None,
            visual_attachments: Vec::new(),
            authorization,
        }
    }
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(tag = "kind", rename_all = "snake_case")]
pub enum SpineResult {
    Completed {
        messages: Vec<SpineMessage>,
        final_text: String,
        turns_used: usize,
    },
    Blocked {
        messages: Vec<SpineMessage>,
        final_text: String,
        turns_used: usize,
    },
    NeedsInput {
        messages: Vec<SpineMessage>,
        final_text: String,
        turns_used: usize,
    },
    MaxTurnsExceeded {
        messages: Vec<SpineMessage>,
        turns_used: usize,
    },
    Cancelled {
        messages: Vec<SpineMessage>,
        turns_used: usize,
        reason: String,
    },
    PausedForApproval {
        messages: Vec<SpineMessage>,
        turns_used: usize,
        pending_call: SpineToolCall,
    },
    PlatformFailed {
        messages: Vec<SpineMessage>,
        turns_used: usize,
        error: SpineError,
    },
}

#[derive(Debug, Clone, Serialize, Deserialize, thiserror::Error)]
#[error("{message}")]
pub struct SpineError {
    pub code: String,
    pub message: String,
}

impl SpineError {
    pub fn new(code: impl Into<String>, message: impl Into<String>) -> Self {
        Self {
            code: code.into(),
            message: message.into(),
        }
    }
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(tag = "event", rename_all = "snake_case")]
pub enum SpineTraceEvent {
    PromptTelemetry {
        data: serde_json::Value,
    },
    ArkDistillTelemetry {
        data: serde_json::Value,
    },
    TurnStarted {
        turn: usize,
        prompt_token_estimate: usize,
        tool_count: usize,
    },
    ModelCompleted {
        turn: usize,
        completion_tokens: usize,
        tool_calls_count: usize,
        cache_read_tokens: usize,
        cache_creation_tokens: usize,
        /// Wall-clock latency of the model call for this turn, in milliseconds —
        /// the real time the user waited for the model to respond.
        latency_ms: u64,
    },
    CompletionVerificationStarted {
        turn: usize,
        proposed_answer_chars: usize,
    },
    CompletionVerificationCompleted {
        turn: usize,
        complete: bool,
    },
    ToolStarted {
        tool_call_id: String,
        name: String,
        #[serde(default, skip_serializing_if = "Option::is_none")]
        arguments: Option<serde_json::Value>,
        #[serde(default, skip_serializing_if = "Option::is_none")]
        activity_label: Option<String>,
        #[serde(default, skip_serializing_if = "Option::is_none")]
        display_label: Option<String>,
        #[serde(default, skip_serializing_if = "Option::is_none")]
        intent_summary: Option<String>,
    },
    ToolCompleted {
        tool_call_id: String,
        name: String,
        ok: bool,
        summary: String,
    },
    TurnCompleted {
        turn: usize,
        terminal_state: SpineTerminalState,
        final_text_present: bool,
    },
}

#[derive(Debug, Clone, Copy, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "snake_case")]
pub enum SpineTerminalState {
    Completed,
    NeedsInput,
    Blocked,
    MaxTurnsExceeded,
    Cancelled,
    PausedForApproval,
    PlatformFailed,
}

#[derive(Debug, Default)]
pub struct SpineTraceRecorder {
    events: tokio::sync::Mutex<Vec<SpineTraceEvent>>,
}

impl SpineTraceRecorder {
    pub async fn emit(&self, event: SpineTraceEvent) {
        self.events.lock().await.push(event);
    }

    pub async fn snapshot(&self) -> Vec<SpineTraceEvent> {
        self.events.lock().await.clone()
    }
}
