use serde::{Deserialize, Serialize};
use std::collections::{BTreeMap, HashMap};
use url::Url;

#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
pub struct VoiceRuntimeConfig {
    pub enabled: bool,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub bridge_url: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub disabled_reason: Option<String>,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum VoiceSessionPhase {
    Connecting,
    Listening,
    Thinking,
    Speaking,
    Muted,
    Error,
    Stopped,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
pub struct VoiceSession {
    pub id: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub conversation_id: Option<String>,
    pub transport: String,
    pub phase: VoiceSessionPhase,
    pub created_at: String,
    pub updated_at: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub last_error: Option<String>,
    #[serde(skip_serializing)]
    stream_token: Option<String>,
}

#[derive(Debug, Default, Clone)]
pub struct VoiceSessionRegistry {
    sessions: HashMap<String, VoiceSession>,
}

pub const VOICE_TRANSPORT_BROWSER: &str = "browser";
pub const VOICE_TRANSPORT_BROWSER_WEBSOCKET: &str = "browser_websocket";

impl VoiceSessionRegistry {
    pub fn start_browser_session(&mut self, conversation_id: Option<String>) -> VoiceSession {
        self.start_session(conversation_id, VOICE_TRANSPORT_BROWSER, None)
    }

    pub fn start_browser_stream_session(
        &mut self,
        conversation_id: Option<String>,
    ) -> VoiceSession {
        self.start_session(
            conversation_id,
            VOICE_TRANSPORT_BROWSER_WEBSOCKET,
            Some(new_stream_token()),
        )
    }

    fn start_session(
        &mut self,
        conversation_id: Option<String>,
        transport: impl Into<String>,
        stream_token: Option<String>,
    ) -> VoiceSession {
        let now = chrono::Utc::now().to_rfc3339();
        let session = VoiceSession {
            id: uuid::Uuid::new_v4().to_string(),
            conversation_id: normalize_optional(conversation_id),
            transport: transport.into(),
            phase: VoiceSessionPhase::Listening,
            created_at: now.clone(),
            updated_at: now,
            last_error: None,
            stream_token,
        };
        self.sessions.insert(session.id.clone(), session.clone());
        session
    }

    pub fn get(&self, session_id: &str) -> Option<VoiceSession> {
        self.sessions.get(session_id.trim()).cloned()
    }

    pub fn latest_active(&self) -> Option<VoiceSession> {
        self.sessions
            .values()
            .filter(|session| session.phase != VoiceSessionPhase::Stopped)
            .max_by(|a, b| a.updated_at.cmp(&b.updated_at))
            .cloned()
    }

    pub fn active_for_conversation(&self, conversation_id: &str) -> Option<VoiceSession> {
        let conversation_id = conversation_id.trim();
        if conversation_id.is_empty() {
            return None;
        }
        self.sessions
            .values()
            .filter(|session| {
                session.phase != VoiceSessionPhase::Stopped
                    && session.conversation_id.as_deref() == Some(conversation_id)
            })
            .max_by(|a, b| a.updated_at.cmp(&b.updated_at))
            .cloned()
    }

    pub fn active_stream_for_conversation(&self, conversation_id: &str) -> Option<VoiceSession> {
        let conversation_id = conversation_id.trim();
        if conversation_id.is_empty() {
            return None;
        }
        self.sessions
            .values()
            .filter(|session| {
                session.phase != VoiceSessionPhase::Stopped
                    && session.transport == VOICE_TRANSPORT_BROWSER_WEBSOCKET
                    && session.stream_token.is_some()
                    && session.conversation_id.as_deref() == Some(conversation_id)
            })
            .max_by(|a, b| a.updated_at.cmp(&b.updated_at))
            .cloned()
    }

    pub fn stream_token_for_session(&self, session_id: &str) -> Option<String> {
        self.sessions
            .get(session_id.trim())
            .and_then(|session| session.stream_token.clone())
    }

    pub fn stream_token_matches(&self, session_id: &str, token: &str) -> bool {
        let token = token.trim();
        if token.is_empty() {
            return false;
        }
        let Some(session) = self.sessions.get(session_id.trim()) else {
            return false;
        };
        if session.phase == VoiceSessionPhase::Stopped {
            return false;
        }
        session.stream_token.as_deref().is_some_and(|expected| {
            crate::security::constant_time_eq(expected.as_bytes(), token.as_bytes())
        })
    }

    pub fn set_phase(
        &mut self,
        session_id: &str,
        phase: VoiceSessionPhase,
    ) -> Option<VoiceSession> {
        let session = self.sessions.get_mut(session_id.trim())?;
        session.phase = phase;
        session.updated_at = chrono::Utc::now().to_rfc3339();
        if phase != VoiceSessionPhase::Error {
            session.last_error = None;
        }
        Some(session.clone())
    }

    pub fn set_error(
        &mut self,
        session_id: &str,
        error: impl Into<String>,
    ) -> Option<VoiceSession> {
        let session = self.sessions.get_mut(session_id.trim())?;
        session.phase = VoiceSessionPhase::Error;
        session.last_error = Some(error.into());
        session.updated_at = chrono::Utc::now().to_rfc3339();
        Some(session.clone())
    }

    pub fn set_conversation_id(
        &mut self,
        session_id: &str,
        conversation_id: Option<String>,
    ) -> Option<VoiceSession> {
        let session = self.sessions.get_mut(session_id.trim())?;
        if let Some(conversation_id) = normalize_optional(conversation_id) {
            session.conversation_id = Some(conversation_id);
            session.updated_at = chrono::Utc::now().to_rfc3339();
        }
        Some(session.clone())
    }

    pub fn stop(&mut self, session_id: &str) -> Option<VoiceSession> {
        self.set_phase(session_id, VoiceSessionPhase::Stopped)
    }
}

pub fn voice_runtime_config_from_current_env() -> VoiceRuntimeConfig {
    voice_runtime_config_from_env(&std::env::vars().collect::<BTreeMap<_, _>>())
}

pub fn voice_runtime_config_from_env(env: &BTreeMap<String, String>) -> VoiceRuntimeConfig {
    let bridge_url = env
        .get("AGENTARK_VOICE_BRIDGE_URL")
        .map(|value| value.trim())
        .filter(|value| !value.is_empty())
        .map(str::to_string);
    let enabled = bridge_url.is_some();
    VoiceRuntimeConfig {
        enabled,
        bridge_url,
        disabled_reason: (!enabled).then(|| "voice_not_enabled".to_string()),
    }
}

fn normalize_optional(value: Option<String>) -> Option<String> {
    value
        .map(|value| value.trim().to_string())
        .filter(|value| !value.is_empty())
}

fn new_stream_token() -> String {
    format!(
        "{}{}",
        uuid::Uuid::new_v4().simple(),
        uuid::Uuid::new_v4().simple()
    )
}

pub fn voice_bridge_stream_url(bridge_url: &str, session_id: &str) -> Result<Url, String> {
    let mut url = Url::parse(bridge_url.trim()).map_err(|error| error.to_string())?;
    let scheme = match url.scheme() {
        "http" => "ws",
        "https" => "wss",
        "ws" => "ws",
        "wss" => "wss",
        other => return Err(format!("unsupported bridge scheme: {other}")),
    };
    url.set_scheme(scheme)
        .map_err(|_| "unsupported bridge scheme".to_string())?;
    url.set_query(None);
    url.set_fragment(None);
    {
        let mut segments = url
            .path_segments_mut()
            .map_err(|_| "bridge url cannot be a base".to_string())?;
        segments.clear();
        segments.push("sessions");
        segments.push(session_id.trim());
        segments.push("stream");
    }
    Ok(url)
}
