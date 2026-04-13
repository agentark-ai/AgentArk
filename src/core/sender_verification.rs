use anyhow::{anyhow, Context, Result};
use once_cell::sync::Lazy;
use serde::{de::DeserializeOwned, Deserialize, Serialize};
use tokio::sync::Mutex;

use crate::storage::Storage;

const SETTINGS_KEY: &str = "sender_verification:settings:v1";
const PENDING_KEY: &str = "sender_verification:pending:v1";
const APPROVED_KEY: &str = "sender_verification:approved:v1";

static SENDER_VERIFICATION_LOCK: Lazy<Mutex<()>> = Lazy::new(|| Mutex::new(()));

#[derive(Debug, Clone, Copy, Serialize, Deserialize, PartialEq, Eq, Default)]
#[serde(rename_all = "snake_case")]
pub enum SenderChannel {
    GoogleChat,
    Signal,
    IMessage,
    Line,
    Slack,
    Teams,
    WeChat,
    Qq,
    #[default]
    Whatsapp,
}

impl SenderChannel {
    pub fn as_str(self) -> &'static str {
        match self {
            Self::GoogleChat => "google_chat",
            Self::Signal => "signal",
            Self::IMessage => "imessage",
            Self::Line => "line",
            Self::Slack => "slack",
            Self::Teams => "teams",
            Self::WeChat => "wechat",
            Self::Qq => "qq",
            Self::Whatsapp => "whatsapp",
        }
    }
}

#[derive(Debug, Clone, Copy, Serialize, Deserialize, PartialEq, Eq, Default)]
#[serde(rename_all = "snake_case")]
pub enum SenderTrustPolicy {
    #[default]
    Open,
    Pairing,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ChannelSenderVerificationSettings {
    #[serde(default)]
    pub policy: SenderTrustPolicy,
    #[serde(default)]
    pub allowed_senders: Vec<String>,
}

impl Default for ChannelSenderVerificationSettings {
    fn default() -> Self {
        Self {
            policy: SenderTrustPolicy::Open,
            allowed_senders: Vec::new(),
        }
    }
}

#[derive(Debug, Clone, Serialize, Deserialize, Default)]
pub struct SenderVerificationSettings {
    #[serde(default)]
    pub google_chat: ChannelSenderVerificationSettings,
    #[serde(default)]
    pub signal: ChannelSenderVerificationSettings,
    #[serde(default)]
    pub imessage: ChannelSenderVerificationSettings,
    #[serde(default)]
    pub line: ChannelSenderVerificationSettings,
    #[serde(default)]
    pub slack: ChannelSenderVerificationSettings,
    #[serde(default)]
    pub teams: ChannelSenderVerificationSettings,
    #[serde(default)]
    pub wechat: ChannelSenderVerificationSettings,
    #[serde(default)]
    pub qq: ChannelSenderVerificationSettings,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct SenderIdentity {
    pub channel: SenderChannel,
    pub sender_id: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub sender_label: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub scope_id: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub scope_label: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub conversation_id: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub message_preview: Option<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct PendingSenderRequest {
    pub key: String,
    pub channel: SenderChannel,
    pub sender_id: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub sender_label: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub scope_id: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub scope_label: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub conversation_id: Option<String>,
    pub first_seen_at: String,
    pub last_seen_at: String,
    pub occurrences: u32,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub message_preview: Option<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ApprovedSender {
    pub key: String,
    pub channel: SenderChannel,
    pub sender_id: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub sender_label: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub scope_id: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub scope_label: Option<String>,
    pub approved_at: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub approved_by: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub conversation_id: Option<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize, Default)]
pub struct SenderVerificationSnapshot {
    #[serde(default)]
    pub settings: SenderVerificationSettings,
    #[serde(default)]
    pub pending: Vec<PendingSenderRequest>,
    #[serde(default)]
    pub approved: Vec<ApprovedSender>,
}

#[derive(Debug, Clone)]
pub enum SenderTrustDecision {
    Allowed,
    NeedsApproval {
        request: Box<PendingSenderRequest>,
        created_new: bool,
    },
}

fn now_rfc3339() -> String {
    chrono::Utc::now().to_rfc3339()
}

fn truncate_preview(text: Option<&str>) -> Option<String> {
    let raw = text.map(str::trim).filter(|value| !value.is_empty())?;
    Some(raw.chars().take(240).collect())
}

fn normalize_sender_value(channel: SenderChannel, value: &str) -> String {
    let trimmed = value.trim();
    if trimmed.is_empty() {
        return String::new();
    }
    match channel {
        SenderChannel::Whatsapp => {
            let digits = trimmed
                .chars()
                .filter(|ch| ch.is_ascii_digit())
                .collect::<String>();
            if digits.is_empty() {
                trimmed.to_ascii_lowercase()
            } else {
                digits
            }
        }
        SenderChannel::Slack
        | SenderChannel::Teams
        | SenderChannel::GoogleChat
        | SenderChannel::Signal
        | SenderChannel::IMessage
        | SenderChannel::Line
        | SenderChannel::WeChat
        | SenderChannel::Qq => trimmed.to_ascii_lowercase(),
    }
}

fn normalize_optional_scope(channel: SenderChannel, value: Option<&str>) -> Option<String> {
    value
        .map(|inner| normalize_sender_value(channel, inner))
        .filter(|inner| !inner.is_empty())
}

fn approval_key(channel: SenderChannel, sender_id: &str, scope_id: Option<&str>) -> Result<String> {
    let sender = normalize_sender_value(channel, sender_id);
    if sender.is_empty() {
        return Err(anyhow!("sender id is required"));
    }
    let scope =
        normalize_optional_scope(channel, scope_id).unwrap_or_else(|| "_global".to_string());
    Ok(format!("{}::{}::{}", channel.as_str(), scope, sender))
}

async fn load_json<T>(storage: &Storage, key: &str) -> Result<T>
where
    T: DeserializeOwned + Default,
{
    let Some(bytes) = storage.get_encrypted(key).await? else {
        return Ok(T::default());
    };
    serde_json::from_slice::<T>(&bytes)
        .with_context(|| format!("failed to decode sender verification payload for {}", key))
}

async fn save_json<T>(storage: &Storage, key: &str, value: &T) -> Result<()>
where
    T: Serialize + ?Sized,
{
    let bytes = serde_json::to_vec(value)
        .with_context(|| format!("failed to encode sender verification payload for {}", key))?;
    storage.set_encrypted(key, &bytes).await
}

async fn load_pending(storage: &Storage) -> Result<Vec<PendingSenderRequest>> {
    load_json(storage, PENDING_KEY).await
}

async fn save_pending(storage: &Storage, items: &[PendingSenderRequest]) -> Result<()> {
    save_json(storage, PENDING_KEY, items).await
}

async fn load_approved(storage: &Storage) -> Result<Vec<ApprovedSender>> {
    load_json(storage, APPROVED_KEY).await
}

async fn save_approved(storage: &Storage, items: &[ApprovedSender]) -> Result<()> {
    save_json(storage, APPROVED_KEY, items).await
}

pub async fn load_settings(storage: &Storage) -> Result<SenderVerificationSettings> {
    let Some(bytes) = storage.get_encrypted(SETTINGS_KEY).await? else {
        return Ok(SenderVerificationSettings::default());
    };

    match serde_json::from_slice::<SenderVerificationSettings>(&bytes) {
        Ok(settings) => Ok(settings),
        Err(error) => {
            tracing::warn!(
                key = SETTINGS_KEY,
                payload_bytes = bytes.len(),
                "Ignoring unreadable sender verification settings payload: {}",
                error
            );
            Ok(SenderVerificationSettings::default())
        }
    }
}

pub async fn save_settings(storage: &Storage, settings: &SenderVerificationSettings) -> Result<()> {
    save_json(storage, SETTINGS_KEY, settings).await
}

pub async fn load_snapshot(storage: &Storage) -> Result<SenderVerificationSnapshot> {
    Ok(SenderVerificationSnapshot {
        settings: load_settings(storage).await?,
        pending: list_pending(storage).await?,
        approved: list_approved(storage).await?,
    })
}

pub async fn list_pending(storage: &Storage) -> Result<Vec<PendingSenderRequest>> {
    let mut items = load_pending(storage).await?;
    items.sort_by(|left, right| right.last_seen_at.cmp(&left.last_seen_at));
    Ok(items)
}

pub async fn list_approved(storage: &Storage) -> Result<Vec<ApprovedSender>> {
    let mut items = load_approved(storage).await?;
    items.sort_by(|left, right| right.approved_at.cmp(&left.approved_at));
    Ok(items)
}

pub async fn is_sender_approved(storage: &Storage, identity: &SenderIdentity) -> Result<bool> {
    let key = approval_key(
        identity.channel,
        identity.sender_id.as_str(),
        identity.scope_id.as_deref(),
    )?;
    let approved = load_approved(storage).await?;
    Ok(approved.iter().any(|entry| entry.key == key))
}

pub async fn evaluate_sender_with_rules(
    storage: &Storage,
    identity: &SenderIdentity,
    policy: SenderTrustPolicy,
    allowed_senders: &[String],
) -> Result<SenderTrustDecision> {
    if policy == SenderTrustPolicy::Open {
        return Ok(SenderTrustDecision::Allowed);
    }

    let normalized_sender = normalize_sender_value(identity.channel, identity.sender_id.as_str());
    if normalized_sender.is_empty() {
        return Err(anyhow!("sender id is required"));
    }

    if allowed_senders
        .iter()
        .map(|value| normalize_sender_value(identity.channel, value))
        .any(|value| !value.is_empty() && value == normalized_sender)
    {
        return Ok(SenderTrustDecision::Allowed);
    }

    if is_sender_approved(storage, identity).await? {
        return Ok(SenderTrustDecision::Allowed);
    }

    let _guard = SENDER_VERIFICATION_LOCK.lock().await;
    let mut pending = load_pending(storage).await?;
    let key = approval_key(
        identity.channel,
        identity.sender_id.as_str(),
        identity.scope_id.as_deref(),
    )?;
    let now = now_rfc3339();

    if let Some(existing) = pending.iter_mut().find(|entry| entry.key == key) {
        existing.last_seen_at = now;
        existing.occurrences = existing.occurrences.saturating_add(1);
        if existing.sender_label.is_none() {
            existing.sender_label = identity.sender_label.clone();
        }
        if existing.scope_label.is_none() {
            existing.scope_label = identity.scope_label.clone();
        }
        if existing.conversation_id.is_none() {
            existing.conversation_id = identity.conversation_id.clone();
        }
        if existing.message_preview.is_none() {
            existing.message_preview = truncate_preview(identity.message_preview.as_deref());
        }
        let request = existing.clone();
        save_pending(storage, &pending).await?;
        return Ok(SenderTrustDecision::NeedsApproval {
            request: Box::new(request),
            created_new: false,
        });
    }

    let request = PendingSenderRequest {
        key,
        channel: identity.channel,
        sender_id: identity.sender_id.trim().to_string(),
        sender_label: identity.sender_label.clone(),
        scope_id: identity
            .scope_id
            .as_ref()
            .map(|value| value.trim().to_string()),
        scope_label: identity.scope_label.clone(),
        conversation_id: identity.conversation_id.clone(),
        first_seen_at: now.clone(),
        last_seen_at: now,
        occurrences: 1,
        message_preview: truncate_preview(identity.message_preview.as_deref()),
    };
    pending.push(request.clone());
    save_pending(storage, &pending).await?;
    Ok(SenderTrustDecision::NeedsApproval {
        request: Box::new(request),
        created_new: true,
    })
}

pub async fn approve_sender(
    storage: &Storage,
    identity: &SenderIdentity,
    approved_by: Option<&str>,
) -> Result<ApprovedSender> {
    let _guard = SENDER_VERIFICATION_LOCK.lock().await;
    let mut approved = load_approved(storage).await?;
    let mut pending = load_pending(storage).await?;
    let key = approval_key(
        identity.channel,
        identity.sender_id.as_str(),
        identity.scope_id.as_deref(),
    )?;
    let now = now_rfc3339();

    let approved_entry = ApprovedSender {
        key: key.clone(),
        channel: identity.channel,
        sender_id: identity.sender_id.trim().to_string(),
        sender_label: identity.sender_label.clone(),
        scope_id: identity
            .scope_id
            .as_ref()
            .map(|value| value.trim().to_string()),
        scope_label: identity.scope_label.clone(),
        approved_at: now,
        approved_by: approved_by
            .map(str::trim)
            .filter(|value| !value.is_empty())
            .map(ToOwned::to_owned),
        conversation_id: identity.conversation_id.clone(),
    };

    approved.retain(|entry| entry.key != key);
    approved.push(approved_entry.clone());
    pending.retain(|entry| entry.key != key);

    save_approved(storage, &approved).await?;
    save_pending(storage, &pending).await?;
    Ok(approved_entry)
}

pub async fn revoke_sender(
    storage: &Storage,
    channel: SenderChannel,
    sender_id: &str,
    scope_id: Option<&str>,
) -> Result<bool> {
    let _guard = SENDER_VERIFICATION_LOCK.lock().await;
    let mut approved = load_approved(storage).await?;
    let key = approval_key(channel, sender_id, scope_id)?;
    let before = approved.len();
    approved.retain(|entry| entry.key != key);
    if approved.len() == before {
        return Ok(false);
    }
    save_approved(storage, &approved).await?;
    Ok(true)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[tokio::test]
    async fn pairing_policy_requires_approval_then_allows_sender() {
        let _dir = tempfile::tempdir().unwrap();
        let storage = Storage::connect(
            crate::storage::DatabaseConfig::for_tests().expect("test database config"),
        )
        .await
        .unwrap();
        let identity = SenderIdentity {
            channel: SenderChannel::Slack,
            sender_id: "U123".to_string(),
            sender_label: Some("Alice".to_string()),
            scope_id: Some("T999".to_string()),
            scope_label: Some("Workspace".to_string()),
            conversation_id: Some("slack:T999:C111:1.0".to_string()),
            message_preview: Some("deploy failed".to_string()),
        };

        let first =
            evaluate_sender_with_rules(&storage, &identity, SenderTrustPolicy::Pairing, &[])
                .await
                .unwrap();
        match first {
            SenderTrustDecision::NeedsApproval { created_new, .. } => assert!(created_new),
            SenderTrustDecision::Allowed => panic!("sender should have required approval"),
        }

        approve_sender(&storage, &identity, Some("ui"))
            .await
            .unwrap();

        let second =
            evaluate_sender_with_rules(&storage, &identity, SenderTrustPolicy::Pairing, &[])
                .await
                .unwrap();
        assert!(matches!(second, SenderTrustDecision::Allowed));
    }

    #[tokio::test]
    async fn repeated_unknown_sender_updates_pending_without_duplication() {
        let _dir = tempfile::tempdir().unwrap();
        let storage = Storage::connect(
            crate::storage::DatabaseConfig::for_tests().expect("test database config"),
        )
        .await
        .unwrap();
        let identity = SenderIdentity {
            channel: SenderChannel::Teams,
            sender_id: "user@example.com".to_string(),
            sender_label: Some("Taylor".to_string()),
            scope_id: Some("tenant-1".to_string()),
            scope_label: None,
            conversation_id: None,
            message_preview: Some("Need approval".to_string()),
        };

        let _ = evaluate_sender_with_rules(&storage, &identity, SenderTrustPolicy::Pairing, &[])
            .await
            .unwrap();
        let repeat =
            evaluate_sender_with_rules(&storage, &identity, SenderTrustPolicy::Pairing, &[])
                .await
                .unwrap();

        match repeat {
            SenderTrustDecision::NeedsApproval {
                created_new,
                request,
            } => {
                assert!(!created_new);
                assert_eq!(request.occurrences, 2);
            }
            SenderTrustDecision::Allowed => panic!("sender should still be pending"),
        }

        let pending = list_pending(&storage).await.unwrap();
        assert_eq!(pending.len(), 1);
    }

    #[tokio::test]
    async fn unreadable_settings_payload_falls_back_to_defaults() {
        let _dir = tempfile::tempdir().unwrap();
        let storage = Storage::connect(
            crate::storage::DatabaseConfig::for_tests().expect("test database config"),
        )
        .await
        .unwrap();

        storage
            .set_encrypted(SETTINGS_KEY, b"{ definitely-not-json")
            .await
            .unwrap();

        let settings = load_settings(&storage).await.unwrap();
        assert_eq!(settings.google_chat.policy, SenderTrustPolicy::Open);
        assert!(settings.google_chat.allowed_senders.is_empty());
        assert_eq!(settings.slack.policy, SenderTrustPolicy::Open);
        assert!(settings.slack.allowed_senders.is_empty());
    }
}
