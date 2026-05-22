//! Scoped companion-device control plane.
//!
//! This module stores companion pairing sessions, device grants, scoped device
//! tokens, typed commands, and audit events in the existing KV store. Device
//! tokens are never equivalent to UI/API sessions.

use anyhow::{Context, Result, anyhow};
use base64::{Engine as _, engine::general_purpose::URL_SAFE_NO_PAD};
use chrono::{Duration, Utc};
use ring::rand::{SecureRandom, SystemRandom};
use serde::{Deserialize, Serialize};
use sha2::{Digest, Sha256};
use std::collections::{BTreeMap, BTreeSet};

use crate::storage::Storage;

const DEVICE_INDEX_KEY: &str = "companion:devices:index";
const PAIRING_INDEX_KEY: &str = "companion:pairing:index";
const AUDIT_INDEX_KEY: &str = "companion:audit:index";
const DEVICE_PREFIX: &str = "companion:device:";
const GRANT_PREFIX: &str = "companion:grant:";
const PAIRING_PREFIX: &str = "companion:pairing:";
const COMMAND_PREFIX: &str = "companion:command:";
const COMMAND_INDEX_PREFIX: &str = "companion:commands:";
const AUDIT_PREFIX: &str = "companion:audit:";
const PAIRING_TTL_SECS: i64 = 10 * 60;
const MAX_AUDIT_EVENTS: usize = 5_000;
const MAX_PAIRING_FAILED_CLAIMS: u32 = 12;
const PAIRING_CLAIM_LOCKOUT_SECS: i64 = 60;

fn device_key(id: &str) -> String {
    format!("{}{}", DEVICE_PREFIX, id.trim())
}

fn grant_key(id: &str) -> String {
    format!("{}{}", GRANT_PREFIX, id.trim())
}

fn pairing_key(id: &str) -> String {
    format!("{}{}", PAIRING_PREFIX, id.trim())
}

fn command_key(id: &str) -> String {
    format!("{}{}", COMMAND_PREFIX, id.trim())
}

fn command_index_key(device_id: &str) -> String {
    format!("{}{}", COMMAND_INDEX_PREFIX, device_id.trim())
}

fn audit_key(id: &str) -> String {
    format!("{}{}", AUDIT_PREFIX, id.trim())
}

fn now_rfc3339() -> String {
    Utc::now().to_rfc3339()
}

fn generate_secret(prefix: &str, bytes: usize) -> Result<String> {
    let rng = SystemRandom::new();
    let mut raw = vec![0u8; bytes.max(16)];
    rng.fill(&mut raw)
        .map_err(|_| anyhow!("failed to generate secure random token"))?;
    Ok(format!("{}{}", prefix, URL_SAFE_NO_PAD.encode(raw)))
}

fn generate_pairing_code() -> Result<String> {
    let rng = SystemRandom::new();
    let mut raw = [0u8; 16];
    rng.fill(&mut raw)
        .map_err(|_| anyhow!("failed to generate pairing code"))?;
    Ok(URL_SAFE_NO_PAD.encode(raw))
}

fn token_fingerprint(token: &str) -> String {
    let mut hasher = Sha256::new();
    hasher.update(b"agentark-companion-device-token-v1");
    hasher.update([0]);
    hasher.update(token.as_bytes());
    hex::encode(hasher.finalize())
}

fn public_key_fingerprint(public_key: &str) -> String {
    let mut hasher = Sha256::new();
    hasher.update(b"agentark-companion-device-public-key-v1");
    hasher.update([0]);
    hasher.update(public_key.as_bytes());
    hex::encode(hasher.finalize())
}

fn constant_time_eq(a: &str, b: &str) -> bool {
    crate::security::constant_time_eq(a.as_bytes(), b.as_bytes())
}

fn normalize_id(value: &str) -> String {
    value
        .trim()
        .to_ascii_lowercase()
        .chars()
        .filter(|ch| ch.is_ascii_alphanumeric() || matches!(ch, '_' | '-' | '.' | ':'))
        .collect()
}

fn is_custom_capability(id: &str) -> bool {
    id.starts_with("custom.") || id.starts_with("custom:")
}

fn is_valid_capability_id(id: &str) -> bool {
    !id.is_empty()
        && id.len() <= 96
        && id
            .chars()
            .all(|ch| ch.is_ascii_alphanumeric() || matches!(ch, '_' | '-' | '.' | ':'))
}

fn normalize_capabilities(values: &[String]) -> Vec<String> {
    let mut out = values
        .iter()
        .map(|value| normalize_id(value))
        .filter(|value| is_valid_capability_id(value))
        .collect::<Vec<_>>();
    out.sort();
    out.dedup();
    out
}

fn normalize_resources(values: BTreeMap<String, Vec<String>>) -> BTreeMap<String, Vec<String>> {
    values
        .into_iter()
        .filter_map(|(key, values)| {
            let key = normalize_id(&key);
            if key.is_empty() {
                return None;
            }
            let mut values = values
                .into_iter()
                .map(|value| value.trim().to_string())
                .filter(|value| !value.is_empty())
                .collect::<Vec<_>>();
            values.sort();
            values.dedup();
            Some((key, values))
        })
        .collect()
}

fn scope_subset(requested: &[String], allowed: &[String]) -> bool {
    let allowed = allowed
        .iter()
        .map(|value| value.as_str())
        .collect::<BTreeSet<_>>();
    requested
        .iter()
        .all(|scope| allowed.contains(scope.as_str()))
}

fn resource_subset(
    requested: &BTreeMap<String, Vec<String>>,
    allowed: &BTreeMap<String, Vec<String>>,
) -> bool {
    requested.iter().all(|(kind, requested_values)| {
        let Some(allowed_values) = allowed.get(kind) else {
            return false;
        };
        if allowed_values.iter().any(|value| value == "*") {
            return true;
        }
        let allowed = allowed_values
            .iter()
            .map(|value| value.as_str())
            .collect::<BTreeSet<_>>();
        requested_values
            .iter()
            .all(|value| allowed.contains(value.as_str()))
    })
}

fn pairing_expired(expires_at: &str) -> bool {
    chrono::DateTime::parse_from_rfc3339(expires_at)
        .map(|ts| ts.with_timezone(&Utc) < Utc::now())
        .unwrap_or(true)
}

fn lockout_active(until: &Option<String>) -> bool {
    until
        .as_deref()
        .and_then(|value| chrono::DateTime::parse_from_rfc3339(value).ok())
        .map(|ts| ts.with_timezone(&Utc) > Utc::now())
        .unwrap_or(false)
}

fn short_fingerprint(value: &str) -> String {
    value.chars().take(16).collect()
}

fn attestation_evidence_fingerprint(evidence: &str) -> String {
    let mut hasher = Sha256::new();
    hasher.update(b"agentark-companion-attestation-evidence-v1");
    hasher.update([0]);
    hasher.update(evidence.as_bytes());
    hex::encode(hasher.finalize())
}

fn evaluate_attestation_claim(
    platform: &str,
    claim: Option<CompanionAttestationClaim>,
) -> CompanionDeviceAttestation {
    let Some(claim) = claim else {
        return CompanionDeviceAttestation {
            platform: Some(platform.to_string()),
            reason: Some("No device attestation evidence was supplied.".to_string()),
            ..Default::default()
        };
    };
    let evidence_fingerprint = claim
        .evidence
        .as_deref()
        .map(str::trim)
        .filter(|value| !value.is_empty())
        .map(attestation_evidence_fingerprint);
    CompanionDeviceAttestation {
        provider: claim
            .provider
            .as_deref()
            .map(str::trim)
            .filter(|value| !value.is_empty())
            .map(str::to_string),
        platform: claim
            .platform
            .as_deref()
            .map(str::trim)
            .filter(|value| !value.is_empty())
            .map(str::to_string)
            .or_else(|| Some(platform.to_string())),
        verified: false,
        evidence_fingerprint,
        verified_at: None,
        reason: Some(
            "Attestation evidence was recorded, but server-side platform verification is not configured."
                .to_string(),
        ),
    }
}

fn has_high_risk_capability(scopes: &[String]) -> bool {
    scopes
        .iter()
        .any(|scope| capability_risk(scope) == CompanionRiskLevel::High)
}

fn bundled_mobile_preset_requires_attestation(preset_id: &str) -> bool {
    matches!(normalize_id(preset_id).as_str(), "ios" | "android")
}

fn companion_audit_hash(event: &CompanionAuditEvent) -> Result<String> {
    let canonical = serde_json::to_vec(event)?;
    let mut hasher = Sha256::new();
    hasher.update(b"agentark-companion-audit-v1");
    hasher.update([0]);
    hasher.update(canonical);
    Ok(hex::encode(hasher.finalize()))
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "snake_case")]
pub enum CompanionTransportKind {
    WebSocket,
}

impl Default for CompanionTransportKind {
    fn default() -> Self {
        Self::WebSocket
    }
}

#[derive(Debug, Clone, Default, Serialize, Deserialize)]
pub struct CompanionDeviceAttestation {
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub provider: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub platform: Option<String>,
    #[serde(default)]
    pub verified: bool,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub evidence_fingerprint: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub verified_at: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub reason: Option<String>,
}

#[derive(Debug, Clone, Default, Deserialize)]
pub struct CompanionAttestationClaim {
    #[serde(default)]
    pub provider: Option<String>,
    #[serde(default)]
    pub platform: Option<String>,
    #[serde(default)]
    pub evidence: Option<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "snake_case")]
pub enum CompanionDeviceState {
    Pairing,
    Paired,
    Online,
    Idle,
    Busy,
    Offline,
    Revoked,
    Error,
}

impl Default for CompanionDeviceState {
    fn default() -> Self {
        Self::Paired
    }
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "snake_case")]
pub enum CompanionPairingStatus {
    Pending,
    Claimed,
    Approved,
    Completed,
    Expired,
    Denied,
}

impl Default for CompanionPairingStatus {
    fn default() -> Self {
        Self::Pending
    }
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "snake_case")]
pub enum CompanionRiskLevel {
    Low,
    High,
}

impl Default for CompanionRiskLevel {
    fn default() -> Self {
        Self::Low
    }
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "snake_case")]
pub enum CompanionApprovalStatus {
    NotRequired,
    Required,
    Approved,
    Denied,
}

impl Default for CompanionApprovalStatus {
    fn default() -> Self {
        Self::NotRequired
    }
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "snake_case")]
pub enum CompanionCommandStatus {
    ApprovalRequired,
    Queued,
    Running,
    Succeeded,
    Failed,
    Denied,
    Cancelled,
}

impl Default for CompanionCommandStatus {
    fn default() -> Self {
        Self::Queued
    }
}

fn default_result_trust() -> String {
    "device_reported_unverified".to_string()
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct CompanionCapabilityDescriptor {
    pub id: String,
    pub label: String,
    pub description: String,
    pub risk: CompanionRiskLevel,
    #[serde(default)]
    pub resource_kinds: Vec<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct CompanionPreset {
    pub id: String,
    pub label: String,
    pub description: String,
    pub platform: String,
    #[serde(default)]
    pub capability_ids: Vec<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct CompanionPresetsResponse {
    pub presets: Vec<CompanionPreset>,
    pub capabilities: Vec<CompanionCapabilityDescriptor>,
    pub protocol_version: String,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct CompanionDevice {
    pub id: String,
    pub display_name: String,
    pub preset_id: String,
    pub platform: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub model: Option<String>,
    pub state: CompanionDeviceState,
    pub transport: CompanionTransportKind,
    #[serde(default)]
    pub available_capabilities: Vec<String>,
    #[serde(default)]
    pub granted_capabilities: Vec<String>,
    #[serde(default)]
    pub token_capabilities: Vec<String>,
    pub paired_at: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub last_seen_at: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub owner: Option<String>,
    #[serde(default)]
    pub metadata: BTreeMap<String, String>,
    #[serde(default)]
    pub attestation: CompanionDeviceAttestation,
    #[serde(default)]
    pub trusted_unattested: bool,
    pub token_fingerprint: String,
    pub active_grant_id: String,
    #[serde(default)]
    pub command_count: usize,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct CompanionGrant {
    pub id: String,
    pub device_id: String,
    pub subject: String,
    #[serde(default)]
    pub capabilities: Vec<String>,
    #[serde(default)]
    pub resources: BTreeMap<String, Vec<String>>,
    pub created_at: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub revoked_at: Option<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct CompanionPairingSession {
    pub id: String,
    pub code: String,
    pub preset_id: String,
    pub display_name: String,
    pub platform: String,
    #[serde(default)]
    pub requested_capabilities: Vec<String>,
    #[serde(default)]
    pub requested_resources: BTreeMap<String, Vec<String>>,
    pub status: CompanionPairingStatus,
    pub created_at: String,
    pub expires_at: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub claimed_at: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub approved_at: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub completed_at: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub claimed_device_public_key: Option<String>,
    #[serde(default)]
    pub attestation: CompanionDeviceAttestation,
    #[serde(default)]
    pub trusted_unattested: bool,
    #[serde(default)]
    pub metadata: BTreeMap<String, String>,
    #[serde(default)]
    pub claim_attempts: u32,
    #[serde(default)]
    pub failed_claim_attempts: u32,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub claim_locked_until: Option<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct CompanionCommand {
    pub id: String,
    pub device_id: String,
    pub capability: String,
    pub action: String,
    #[serde(default)]
    pub arguments: serde_json::Value,
    #[serde(default)]
    pub requested_scopes: Vec<String>,
    #[serde(default)]
    pub resource_scope: BTreeMap<String, Vec<String>>,
    pub risk: CompanionRiskLevel,
    pub approval_status: CompanionApprovalStatus,
    pub status: CompanionCommandStatus,
    pub requested_at: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub approved_at: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub dispatched_at: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub completed_at: Option<String>,
    #[serde(default)]
    pub success: bool,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub result_preview: Option<String>,
    #[serde(default = "default_result_trust")]
    pub result_trust: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub actor: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub audit_reason: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub error: Option<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct CompanionAuditEvent {
    pub id: String,
    pub event_type: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub device_id: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub command_id: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub grant_id: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub actor: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub surface: Option<String>,
    pub decision: String,
    pub reason: String,
    pub timestamp: String,
    #[serde(default)]
    pub metadata: BTreeMap<String, String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub previous_hash: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub event_hash: Option<String>,
}

#[derive(Debug, Clone, Deserialize)]
pub struct CompanionPairingSessionCreate {
    pub display_name: String,
    #[serde(default)]
    pub preset_id: Option<String>,
    #[serde(default)]
    pub platform: Option<String>,
    #[serde(default)]
    pub capabilities: Vec<String>,
    #[serde(default)]
    pub resources: BTreeMap<String, Vec<String>>,
    #[serde(default)]
    pub metadata: BTreeMap<String, String>,
    #[serde(default)]
    pub trusted_unattested: bool,
}

#[derive(Debug, Clone, Deserialize)]
pub struct CompanionCommandCreate {
    pub capability: String,
    pub action: String,
    #[serde(default)]
    pub arguments: serde_json::Value,
    #[serde(default)]
    pub requested_scopes: Vec<String>,
    #[serde(default)]
    pub resource_scope: BTreeMap<String, Vec<String>>,
    #[serde(default)]
    pub actor: Option<String>,
    #[serde(default)]
    pub audit_reason: Option<String>,
}

#[derive(Debug, Clone, Deserialize)]
pub struct CompanionPairingClaim {
    pub session_id: String,
    pub code: String,
    #[serde(default)]
    pub device_public_key: Option<String>,
    #[serde(default)]
    pub attestation: Option<CompanionAttestationClaim>,
    #[serde(default)]
    pub metadata: BTreeMap<String, String>,
}

#[derive(Debug, Clone, Serialize)]
pub struct CompanionPairingClaimResult {
    pub status: CompanionPairingStatus,
    pub message: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub device: Option<CompanionDevice>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub device_token: Option<String>,
}

#[derive(Debug, Clone, Deserialize)]
pub struct CompanionTokenRotationRequest {
    #[serde(default)]
    pub requested_scopes: Vec<String>,
}

#[derive(Debug, Clone, Serialize)]
pub struct CompanionTokenRotationResult {
    pub device: CompanionDevice,
    pub device_token: String,
}

#[derive(Debug, Clone, Serialize)]
pub struct CompanionProtocolDocument {
    pub protocol_version: String,
    pub websocket_path: String,
    pub auth: String,
    pub pairing: String,
    pub messages: Vec<String>,
    pub security: Vec<String>,
}

#[derive(Debug, Clone, Serialize)]
pub struct CompanionOverview {
    pub generated_at: String,
    pub total: usize,
    pub online: usize,
    pub pending_pairing: usize,
    pub pending_approvals: usize,
    pub revoked: usize,
}

#[derive(Clone)]
pub struct CompanionControlPlane {
    storage: Storage,
}

impl CompanionControlPlane {
    pub fn new(storage: Storage) -> Self {
        Self { storage }
    }

    async fn read_json<T: for<'de> Deserialize<'de>>(&self, key: &str) -> Result<Option<T>> {
        self.storage
            .get(key)
            .await?
            .map(|raw| serde_json::from_slice(&raw))
            .transpose()
            .with_context(|| format!("failed to decode companion storage key {key}"))
    }

    async fn write_json<T: Serialize>(&self, key: &str, value: &T) -> Result<()> {
        self.storage.set(key, &serde_json::to_vec(value)?).await
    }

    async fn read_index(&self, key: &str) -> Result<Vec<String>> {
        Ok(self
            .read_json::<Vec<String>>(key)
            .await?
            .unwrap_or_default())
    }

    async fn write_index(&self, key: &str, ids: &[String]) -> Result<()> {
        let mut ids = ids
            .iter()
            .map(|id| id.trim().to_string())
            .filter(|id| !id.is_empty())
            .collect::<Vec<_>>();
        ids.sort();
        ids.dedup();
        self.write_json(key, &ids).await
    }

    async fn write_ordered_index(&self, key: &str, ids: &[String]) -> Result<()> {
        let mut seen = BTreeSet::new();
        let ids = ids
            .iter()
            .map(|id| id.trim().to_string())
            .filter(|id| !id.is_empty())
            .filter(|id| seen.insert(id.clone()))
            .collect::<Vec<_>>();
        self.write_json(key, &ids).await
    }

    async fn append_index(&self, key: &str, id: &str) -> Result<()> {
        let mut ids = self.read_index(key).await?;
        if !ids.iter().any(|existing| existing == id) {
            ids.push(id.to_string());
        }
        self.write_index(key, &ids).await
    }

    pub async fn list_devices(&self) -> Result<Vec<CompanionDevice>> {
        let mut out = Vec::new();
        for id in self.read_index(DEVICE_INDEX_KEY).await? {
            if let Some(device) = self.get_device(&id).await? {
                out.push(device);
            }
        }
        Ok(out)
    }

    pub async fn overview(&self) -> Result<CompanionOverview> {
        let devices = self.list_devices().await?;
        let pending_approvals = self.list_pending_approval_commands().await?.len();
        let pending_pairing = self
            .list_pairing_sessions()
            .await?
            .into_iter()
            .filter(|session| {
                matches!(
                    &session.status,
                    CompanionPairingStatus::Pending | CompanionPairingStatus::Claimed
                ) && !pairing_expired(&session.expires_at)
            })
            .count();
        Ok(CompanionOverview {
            generated_at: now_rfc3339(),
            total: devices.len(),
            online: devices
                .iter()
                .filter(|device| device.state == CompanionDeviceState::Online)
                .count(),
            pending_pairing,
            pending_approvals,
            revoked: devices
                .iter()
                .filter(|device| device.state == CompanionDeviceState::Revoked)
                .count(),
        })
    }

    pub async fn get_device(&self, id: &str) -> Result<Option<CompanionDevice>> {
        self.read_json(&device_key(id)).await
    }

    async fn write_device(&self, device: &CompanionDevice) -> Result<()> {
        self.write_json(&device_key(&device.id), device).await?;
        self.append_index(DEVICE_INDEX_KEY, &device.id).await
    }

    pub async fn get_grant(&self, id: &str) -> Result<Option<CompanionGrant>> {
        self.read_json(&grant_key(id)).await
    }

    async fn write_grant(&self, grant: &CompanionGrant) -> Result<()> {
        self.write_json(&grant_key(&grant.id), grant).await
    }

    pub async fn list_pairing_sessions(&self) -> Result<Vec<CompanionPairingSession>> {
        let mut out = Vec::new();
        for id in self.read_index(PAIRING_INDEX_KEY).await? {
            if let Some(session) = self.read_json(&pairing_key(&id)).await? {
                out.push(session);
            }
        }
        out.sort_by(|a: &CompanionPairingSession, b| b.created_at.cmp(&a.created_at));
        Ok(out)
    }

    pub async fn create_pairing_session(
        &self,
        input: CompanionPairingSessionCreate,
        actor: &str,
    ) -> Result<CompanionPairingSession> {
        let display_name = input.display_name.trim();
        anyhow::ensure!(!display_name.is_empty(), "device name is required");

        let preset_id = input
            .preset_id
            .as_deref()
            .map(normalize_id)
            .filter(|value| !value.is_empty())
            .unwrap_or_else(|| "custom".to_string());
        let preset = companion_presets()
            .into_iter()
            .find(|preset| preset.id == preset_id);
        anyhow::ensure!(
            preset.is_some() || preset_id == "custom",
            "unknown companion preset"
        );
        let platform = input
            .platform
            .as_deref()
            .map(|value| value.trim().to_string())
            .filter(|value| !value.is_empty())
            .or_else(|| preset.as_ref().map(|preset| preset.platform.clone()))
            .unwrap_or_else(|| "custom".to_string());
        let requested_capabilities = if input.capabilities.is_empty() {
            preset
                .as_ref()
                .map(|preset| preset.capability_ids.clone())
                .unwrap_or_default()
        } else {
            normalize_capabilities(&input.capabilities)
        };
        validate_capability_set(&requested_capabilities)?;

        let created_at = now_rfc3339();
        let expires_at = (Utc::now() + Duration::seconds(PAIRING_TTL_SECS)).to_rfc3339();
        let session = CompanionPairingSession {
            id: format!("pairing-{}", uuid::Uuid::new_v4()),
            code: generate_pairing_code()?,
            preset_id,
            display_name: display_name.to_string(),
            platform,
            requested_capabilities,
            requested_resources: normalize_resources(input.resources),
            status: CompanionPairingStatus::Pending,
            created_at,
            expires_at,
            claimed_at: None,
            approved_at: None,
            completed_at: None,
            claimed_device_public_key: None,
            attestation: CompanionDeviceAttestation::default(),
            trusted_unattested: input.trusted_unattested,
            metadata: input
                .metadata
                .into_iter()
                .map(|(key, value)| (key.trim().to_string(), value.trim().to_string()))
                .filter(|(key, _)| !key.is_empty())
                .collect(),
            claim_attempts: 0,
            failed_claim_attempts: 0,
            claim_locked_until: None,
        };
        self.write_json(&pairing_key(&session.id), &session).await?;
        self.append_index(PAIRING_INDEX_KEY, &session.id).await?;
        self.audit(
            "pairing_session_created",
            None,
            None,
            None,
            Some(actor),
            "ui",
            "allow",
            "Companion pairing session created.",
            BTreeMap::from([("pairing_session_id".to_string(), session.id.clone())]),
        )
        .await?;
        Ok(session)
    }

    pub async fn approve_pairing_session(
        &self,
        id: &str,
        actor: &str,
    ) -> Result<CompanionPairingSession> {
        let mut session = self
            .read_json::<CompanionPairingSession>(&pairing_key(id))
            .await?
            .ok_or_else(|| anyhow!("pairing session not found"))?;
        anyhow::ensure!(
            !pairing_expired(&session.expires_at),
            "pairing session has expired"
        );
        anyhow::ensure!(
            !matches!(
                &session.status,
                CompanionPairingStatus::Completed
                    | CompanionPairingStatus::Denied
                    | CompanionPairingStatus::Expired
            ),
            "pairing session cannot be approved from its current state"
        );
        anyhow::ensure!(
            session.status == CompanionPairingStatus::Claimed,
            "pairing session must be claimed by a device before approval"
        );
        anyhow::ensure!(
            session
                .claimed_device_public_key
                .as_deref()
                .map(|value| !value.trim().is_empty())
                .unwrap_or(false),
            "pairing claim must include a stable device identity"
        );
        let high_risk = has_high_risk_capability(&session.requested_capabilities);
        if high_risk && !session.attestation.verified {
            let mut denial_metadata =
                BTreeMap::from([("pairing_session_id".to_string(), session.id.clone())]);
            denial_metadata.insert(
                "attestation_verified".to_string(),
                session.attestation.verified.to_string(),
            );
            denial_metadata.insert(
                "trusted_unattested".to_string(),
                session.trusted_unattested.to_string(),
            );
            if bundled_mobile_preset_requires_attestation(&session.preset_id) {
                let reason = "High-risk bundled mobile companions require verified platform attestation before approval.";
                self.audit(
                    "pairing_session_approval_denied",
                    None,
                    None,
                    None,
                    Some(actor),
                    "ui",
                    "deny",
                    reason,
                    denial_metadata,
                )
                .await?;
                anyhow::bail!("{}", reason);
            }
            if !session.trusted_unattested {
                let reason = "High-risk custom or desktop companions require an explicit trusted_unattested override.";
                self.audit(
                    "pairing_session_approval_denied",
                    None,
                    None,
                    None,
                    Some(actor),
                    "ui",
                    "deny",
                    reason,
                    denial_metadata,
                )
                .await?;
                anyhow::bail!("{}", reason);
            }
        }
        session.status = CompanionPairingStatus::Approved;
        session.approved_at = Some(now_rfc3339());
        self.write_json(&pairing_key(&session.id), &session).await?;
        let mut metadata = BTreeMap::from([("pairing_session_id".to_string(), session.id.clone())]);
        metadata.insert(
            "attestation_verified".to_string(),
            session.attestation.verified.to_string(),
        );
        metadata.insert(
            "trusted_unattested".to_string(),
            session.trusted_unattested.to_string(),
        );
        self.audit(
            "pairing_session_approved",
            None,
            None,
            None,
            Some(actor),
            "ui",
            "allow",
            "Companion pairing session approved for one device claim.",
            metadata,
        )
        .await?;
        Ok(session)
    }

    async fn record_pairing_claim_denial(
        &self,
        session: &mut CompanionPairingSession,
        reason: &str,
        mut metadata: BTreeMap<String, String>,
    ) -> Result<()> {
        session.failed_claim_attempts = session.failed_claim_attempts.saturating_add(1);
        if session.failed_claim_attempts >= MAX_PAIRING_FAILED_CLAIMS {
            session.claim_locked_until =
                Some((Utc::now() + Duration::seconds(PAIRING_CLAIM_LOCKOUT_SECS)).to_rfc3339());
        }
        metadata.insert("pairing_session_id".to_string(), session.id.clone());
        metadata.insert(
            "failed_claim_attempts".to_string(),
            session.failed_claim_attempts.to_string(),
        );
        if let Some(until) = &session.claim_locked_until {
            metadata.insert("claim_locked_until".to_string(), until.clone());
        }
        self.write_json(&pairing_key(&session.id), session).await?;
        self.audit(
            "pairing_claim_denied",
            None,
            None,
            None,
            None,
            "websocket",
            "deny",
            reason,
            metadata,
        )
        .await
    }

    pub async fn claim_pairing_session(
        &self,
        claim: CompanionPairingClaim,
    ) -> Result<CompanionPairingClaimResult> {
        let mut session = self
            .read_json::<CompanionPairingSession>(&pairing_key(&claim.session_id))
            .await?
            .ok_or_else(|| anyhow!("pairing session not found"))?;
        if lockout_active(&session.claim_locked_until) {
            self.audit(
                "pairing_claim_denied",
                None,
                None,
                None,
                None,
                "websocket",
                "deny",
                "Pairing claim is temporarily locked after repeated failed attempts.",
                BTreeMap::from([("pairing_session_id".to_string(), session.id.clone())]),
            )
            .await?;
            anyhow::bail!("pairing session is temporarily locked");
        }
        session.claim_attempts = session.claim_attempts.saturating_add(1);
        if !constant_time_eq(&session.code, claim.code.trim()) {
            self.record_pairing_claim_denial(
                &mut session,
                "Pairing claim used an invalid code.",
                BTreeMap::new(),
            )
            .await?;
            anyhow::bail!("invalid pairing code");
        }
        let Some(incoming_public_key) = claim
            .device_public_key
            .as_deref()
            .map(str::trim)
            .filter(|value| !value.is_empty())
            .map(str::to_string)
        else {
            self.record_pairing_claim_denial(
                &mut session,
                "Pairing claim did not include a stable device identity.",
                BTreeMap::new(),
            )
            .await?;
            anyhow::bail!("device identity is required for pairing");
        };
        if pairing_expired(&session.expires_at) {
            session.status = CompanionPairingStatus::Expired;
            self.write_json(&pairing_key(&session.id), &session).await?;
            anyhow::bail!("pairing session has expired");
        }
        match session.status.clone() {
            CompanionPairingStatus::Pending | CompanionPairingStatus::Claimed => {
                if let Some(existing_key) = session.claimed_device_public_key.as_deref() {
                    if existing_key != incoming_public_key {
                        self.record_pairing_claim_denial(
                            &mut session,
                            "Pairing claim used a different device identity than the approved claim.",
                            BTreeMap::from([(
                                "incoming_device_key_fingerprint".to_string(),
                                public_key_fingerprint(&incoming_public_key),
                            )]),
                        )
                        .await?;
                        anyhow::bail!("pairing session is claimed by another device identity");
                    }
                }
                session.status = CompanionPairingStatus::Claimed;
                let first_claim = session.claimed_at.is_none();
                if first_claim {
                    session.claimed_at = Some(now_rfc3339());
                }
                session.claimed_device_public_key = Some(incoming_public_key.clone());
                session.attestation =
                    evaluate_attestation_claim(&session.platform, claim.attestation);
                for (key, value) in claim.metadata {
                    let key = key.trim();
                    if !key.is_empty() {
                        session
                            .metadata
                            .insert(key.to_string(), value.trim().to_string());
                    }
                }
                self.write_json(&pairing_key(&session.id), &session).await?;
                if first_claim {
                    self.audit(
                        "pairing_session_claimed",
                        None,
                        None,
                        None,
                        None,
                        "websocket",
                        "allow",
                        "Companion device claimed pairing session and is waiting for approval.",
                        BTreeMap::from([
                            ("pairing_session_id".to_string(), session.id.clone()),
                            (
                                "device_key_fingerprint".to_string(),
                                public_key_fingerprint(&incoming_public_key),
                            ),
                            (
                                "attestation_verified".to_string(),
                                session.attestation.verified.to_string(),
                            ),
                        ]),
                    )
                    .await?;
                }
                Ok(CompanionPairingClaimResult {
                    status: CompanionPairingStatus::Claimed,
                    message: "Pairing claim received. Approve it in AgentArk.".to_string(),
                    device: None,
                    device_token: None,
                })
            }
            CompanionPairingStatus::Approved => {
                if session.claimed_device_public_key.as_deref()
                    != Some(incoming_public_key.as_str())
                {
                    self.record_pairing_claim_denial(
                        &mut session,
                        "Pairing finalization used a different device identity than the approved claim.",
                        BTreeMap::from([(
                            "incoming_device_key_fingerprint".to_string(),
                            public_key_fingerprint(&incoming_public_key),
                        )]),
                    )
                    .await?;
                    anyhow::bail!("pairing session was approved for another device identity");
                }
                self.finalize_pairing_session(session).await
            }
            CompanionPairingStatus::Completed => Ok(CompanionPairingClaimResult {
                status: CompanionPairingStatus::Completed,
                message: "Pairing session was already completed.".to_string(),
                device: None,
                device_token: None,
            }),
            CompanionPairingStatus::Denied | CompanionPairingStatus::Expired => {
                anyhow::bail!("pairing session is not active")
            }
        }
    }

    async fn finalize_pairing_session(
        &self,
        mut session: CompanionPairingSession,
    ) -> Result<CompanionPairingClaimResult> {
        let token = generate_secret("acd_", 32)?;
        let token_fingerprint = token_fingerprint(&token);
        let device_id = format!("device-{}", uuid::Uuid::new_v4());
        let grant_id = format!("grant-{}", uuid::Uuid::new_v4());
        let now = now_rfc3339();
        let capabilities = normalize_capabilities(&session.requested_capabilities);
        let grant = CompanionGrant {
            id: grant_id.clone(),
            device_id: device_id.clone(),
            subject: "local_user".to_string(),
            capabilities: capabilities.clone(),
            resources: session.requested_resources.clone(),
            created_at: now.clone(),
            revoked_at: None,
        };
        let device = CompanionDevice {
            id: device_id.clone(),
            display_name: session.display_name.clone(),
            preset_id: session.preset_id.clone(),
            platform: session.platform.clone(),
            model: session.metadata.get("model").cloned(),
            state: CompanionDeviceState::Paired,
            transport: CompanionTransportKind::WebSocket,
            available_capabilities: capabilities.clone(),
            granted_capabilities: capabilities.clone(),
            token_capabilities: capabilities,
            paired_at: now.clone(),
            last_seen_at: None,
            owner: Some("local_user".to_string()),
            metadata: session.metadata.clone(),
            attestation: session.attestation.clone(),
            trusted_unattested: session.trusted_unattested,
            token_fingerprint,
            active_grant_id: grant_id.clone(),
            command_count: 0,
        };
        session.status = CompanionPairingStatus::Completed;
        session.completed_at = Some(now);
        session
            .metadata
            .insert("device_id".to_string(), device_id.clone());
        self.write_grant(&grant).await?;
        self.write_device(&device).await?;
        self.write_json(&pairing_key(&session.id), &session).await?;
        self.audit(
            "device_paired",
            Some(&device.id),
            None,
            Some(&grant.id),
            None,
            "websocket",
            "allow",
            "Companion device finalized approved pairing and received a scoped token.",
            BTreeMap::from([("pairing_session_id".to_string(), session.id.clone())]),
        )
        .await?;
        Ok(CompanionPairingClaimResult {
            status: CompanionPairingStatus::Completed,
            message: "Pairing completed. Store the device token in the platform keychain."
                .to_string(),
            device: Some(device),
            device_token: Some(token),
        })
    }

    pub async fn verify_device_token(
        &self,
        device_id: &str,
        token: &str,
    ) -> Result<CompanionDevice> {
        let device = self
            .get_device(device_id)
            .await?
            .ok_or_else(|| anyhow!("device not found"))?;
        anyhow::ensure!(
            device.state != CompanionDeviceState::Revoked,
            "device has been revoked"
        );
        let presented = token_fingerprint(token.trim());
        if !constant_time_eq(&device.token_fingerprint, &presented) {
            self.audit(
                "device_auth_denied",
                Some(&device.id),
                None,
                Some(&device.active_grant_id),
                None,
                "websocket",
                "deny",
                "Device presented an invalid token.",
                BTreeMap::new(),
            )
            .await?;
            anyhow::bail!("invalid device token");
        }
        Ok(device)
    }

    pub async fn pulse_device(
        &self,
        device_id: &str,
        state: Option<CompanionDeviceState>,
        available_capabilities: Vec<String>,
        metadata: BTreeMap<String, String>,
    ) -> Result<CompanionDevice> {
        let mut device = self
            .get_device(device_id)
            .await?
            .ok_or_else(|| anyhow!("device not found"))?;
        anyhow::ensure!(
            device.state != CompanionDeviceState::Revoked,
            "device has been revoked"
        );
        device.state = state.unwrap_or(CompanionDeviceState::Online);
        device.last_seen_at = Some(now_rfc3339());
        if !available_capabilities.is_empty() {
            let available = normalize_capabilities(&available_capabilities);
            validate_capability_set(&available)?;
            device.available_capabilities = available;
        }
        for (key, value) in metadata {
            let key = key.trim();
            if !key.is_empty() {
                device
                    .metadata
                    .insert(key.to_string(), value.trim().to_string());
            }
        }
        self.write_device(&device).await?;
        Ok(device)
    }

    pub async fn create_command(
        &self,
        device_id: &str,
        input: CompanionCommandCreate,
        caller_scopes: &[String],
    ) -> Result<CompanionCommand> {
        let mut device = self
            .get_device(device_id)
            .await?
            .ok_or_else(|| anyhow!("device not found"))?;
        anyhow::ensure!(
            device.state != CompanionDeviceState::Revoked,
            "device has been revoked"
        );
        let grant = self
            .get_grant(&device.active_grant_id)
            .await?
            .ok_or_else(|| anyhow!("device grant not found"))?;
        anyhow::ensure!(grant.revoked_at.is_none(), "device grant has been revoked");
        let capability = normalize_id(&input.capability);
        anyhow::ensure!(
            is_valid_capability_id(&capability),
            "capability id is invalid"
        );
        let action = input.action.trim().to_string();
        anyhow::ensure!(!action.is_empty(), "typed command action is required");
        let mut requested_scopes = normalize_capabilities(&input.requested_scopes);
        if !requested_scopes.iter().any(|scope| scope == &capability) {
            requested_scopes.push(capability.clone());
            requested_scopes.sort();
            requested_scopes.dedup();
        }
        validate_capability_set(&requested_scopes)?;
        let resource_scope = normalize_resources(input.resource_scope);
        self.ensure_scopes_allowed(
            &requested_scopes,
            &resource_scope,
            &grant,
            &device,
            caller_scopes,
        )
        .await?;

        let high_risk = requested_scopes
            .iter()
            .any(|scope| capability_risk(scope) == CompanionRiskLevel::High);
        let now = now_rfc3339();
        let command = CompanionCommand {
            id: format!("cmd-{}", uuid::Uuid::new_v4()),
            device_id: device_id.to_string(),
            capability,
            action,
            arguments: input.arguments,
            requested_scopes,
            resource_scope,
            risk: if high_risk {
                CompanionRiskLevel::High
            } else {
                CompanionRiskLevel::Low
            },
            approval_status: if high_risk {
                CompanionApprovalStatus::Required
            } else {
                CompanionApprovalStatus::NotRequired
            },
            status: if high_risk {
                CompanionCommandStatus::ApprovalRequired
            } else {
                CompanionCommandStatus::Queued
            },
            requested_at: now,
            approved_at: None,
            dispatched_at: None,
            completed_at: None,
            success: false,
            result_preview: None,
            result_trust: "pending_device_report".to_string(),
            actor: input.actor.filter(|value| !value.trim().is_empty()),
            audit_reason: input.audit_reason.filter(|value| !value.trim().is_empty()),
            error: None,
        };
        self.write_command(&command).await?;
        device.command_count = device.command_count.saturating_add(1);
        self.write_device(&device).await?;
        self.audit(
            "command_created",
            Some(device_id),
            Some(&command.id),
            Some(&grant.id),
            command.actor.as_deref(),
            "ui",
            if high_risk {
                "approval_required"
            } else {
                "allow"
            },
            if high_risk {
                "High-risk companion command is waiting for fresh approval."
            } else {
                "Companion command queued after scoped grant validation."
            },
            BTreeMap::new(),
        )
        .await?;
        Ok(command)
    }

    async fn ensure_scopes_allowed(
        &self,
        requested_scopes: &[String],
        requested_resources: &BTreeMap<String, Vec<String>>,
        grant: &CompanionGrant,
        device: &CompanionDevice,
        caller_scopes: &[String],
    ) -> Result<()> {
        if !scope_subset(requested_scopes, &grant.capabilities) {
            self.audit(
                "scope_denied",
                Some(&device.id),
                None,
                Some(&grant.id),
                None,
                "ui",
                "deny",
                "Requested scopes exceeded the paired device grant.",
                BTreeMap::new(),
            )
            .await?;
            anyhow::bail!("requested scopes exceed paired device grant");
        }
        if !scope_subset(requested_scopes, &device.token_capabilities) {
            self.audit(
                "scope_denied",
                Some(&device.id),
                None,
                Some(&grant.id),
                None,
                "ui",
                "deny",
                "Requested scopes exceeded the active device token scope.",
                BTreeMap::new(),
            )
            .await?;
            anyhow::bail!("requested scopes exceed active device token scope");
        }
        if !scope_subset(requested_scopes, caller_scopes) {
            self.audit(
                "scope_denied",
                Some(&device.id),
                None,
                Some(&grant.id),
                None,
                "ui",
                "deny",
                "Requested scopes exceeded the caller grant.",
                BTreeMap::new(),
            )
            .await?;
            anyhow::bail!("requested scopes exceed caller grant");
        }
        if !requested_resources.is_empty()
            && !resource_subset(requested_resources, &grant.resources)
        {
            self.audit(
                "resource_scope_denied",
                Some(&device.id),
                None,
                Some(&grant.id),
                None,
                "ui",
                "deny",
                "Requested resources exceeded the device grant.",
                BTreeMap::new(),
            )
            .await?;
            anyhow::bail!("requested resources exceed device grant");
        }
        Ok(())
    }

    pub async fn approve_command(
        &self,
        command_id: &str,
        actor: &str,
        approved: bool,
        reason: Option<String>,
    ) -> Result<CompanionCommand> {
        let mut command = self
            .get_command(command_id)
            .await?
            .ok_or_else(|| anyhow!("command not found"))?;
        anyhow::ensure!(
            command.status == CompanionCommandStatus::ApprovalRequired,
            "command is not waiting for approval"
        );
        if let Some(reason) = reason.filter(|value| !value.trim().is_empty()) {
            command.audit_reason = Some(reason);
        }
        command.actor = Some(actor.to_string());
        if approved {
            command.approval_status = CompanionApprovalStatus::Approved;
            command.status = CompanionCommandStatus::Queued;
            command.approved_at = Some(now_rfc3339());
        } else {
            command.approval_status = CompanionApprovalStatus::Denied;
            command.status = CompanionCommandStatus::Denied;
            command.completed_at = Some(now_rfc3339());
            command.error = Some("Denied by user approval decision.".to_string());
        }
        self.write_command(&command).await?;
        self.audit(
            if approved {
                "command_approved"
            } else {
                "command_denied"
            },
            Some(&command.device_id),
            Some(&command.id),
            None,
            Some(actor),
            "ui",
            if approved { "allow" } else { "deny" },
            if approved {
                "Fresh approval granted for high-risk companion command."
            } else {
                "Fresh approval denied for high-risk companion command."
            },
            BTreeMap::new(),
        )
        .await?;
        Ok(command)
    }

    pub async fn dispatch_next_command(&self, device_id: &str) -> Result<Option<CompanionCommand>> {
        let device = self
            .get_device(device_id)
            .await?
            .ok_or_else(|| anyhow!("device not found"))?;
        if device.state == CompanionDeviceState::Revoked {
            return Ok(None);
        }
        for mut command in self.list_commands(device_id).await? {
            if command.status != CompanionCommandStatus::Queued {
                continue;
            }
            if !scope_subset(&command.requested_scopes, &device.token_capabilities) {
                command.status = CompanionCommandStatus::Denied;
                command.completed_at = Some(now_rfc3339());
                command.error =
                    Some("Command no longer fits active device token scope.".to_string());
                self.write_command(&command).await?;
                self.audit(
                    "command_scope_stale",
                    Some(device_id),
                    Some(&command.id),
                    Some(&device.active_grant_id),
                    None,
                    "websocket",
                    "deny",
                    "Queued command no longer fits the active device token scope.",
                    BTreeMap::new(),
                )
                .await?;
                continue;
            }
            command.status = CompanionCommandStatus::Running;
            command.dispatched_at = Some(now_rfc3339());
            self.write_command(&command).await?;
            self.audit(
                "command_dispatched",
                Some(device_id),
                Some(&command.id),
                Some(&device.active_grant_id),
                None,
                "websocket",
                "allow",
                "Companion command dispatched over authenticated WebSocket.",
                BTreeMap::new(),
            )
            .await?;
            return Ok(Some(command));
        }
        Ok(None)
    }

    pub async fn complete_command(
        &self,
        device_id: &str,
        command_id: &str,
        success: bool,
        result_preview: Option<String>,
        error: Option<String>,
    ) -> Result<CompanionCommand> {
        let mut command = self
            .get_command(command_id)
            .await?
            .ok_or_else(|| anyhow!("command not found"))?;
        anyhow::ensure!(
            command.device_id == device_id,
            "command belongs to another device"
        );
        command.status = if success {
            CompanionCommandStatus::Succeeded
        } else {
            CompanionCommandStatus::Failed
        };
        command.success = success;
        command.completed_at = Some(now_rfc3339());
        command.result_preview = result_preview
            .map(|value| {
                let clipped = value.trim().chars().take(2000).collect::<String>();
                crate::security::sanitize_untrusted_output("companion", &clipped)
            })
            .filter(|value| !value.is_empty());
        command.result_trust = "device_reported_unverified".to_string();
        command.error = error
            .map(|value| {
                crate::security::redact_secret_input(value.trim())
                    .text
                    .chars()
                    .take(1000)
                    .collect::<String>()
            })
            .filter(|value| !value.is_empty());
        self.write_command(&command).await?;
        self.audit(
            "command_result",
            Some(device_id),
            Some(command_id),
            None,
            None,
            "websocket",
            if success { "succeeded" } else { "failed" },
            "Companion device returned a structured command result. The server records this as device-reported, not independently proven.",
            BTreeMap::from([(
                "result_trust".to_string(),
                command.result_trust.clone(),
            )]),
        )
        .await?;
        Ok(command)
    }

    async fn write_command(&self, command: &CompanionCommand) -> Result<()> {
        self.write_json(&command_key(&command.id), command).await?;
        self.append_index(&command_index_key(&command.device_id), &command.id)
            .await
    }

    pub async fn get_command(&self, id: &str) -> Result<Option<CompanionCommand>> {
        self.read_json(&command_key(id)).await
    }

    pub async fn list_commands(&self, device_id: &str) -> Result<Vec<CompanionCommand>> {
        let mut out = Vec::new();
        for id in self.read_index(&command_index_key(device_id)).await? {
            if let Some(command) = self.get_command(&id).await? {
                out.push(command);
            }
        }
        out.sort_by(|a, b| b.requested_at.cmp(&a.requested_at));
        Ok(out)
    }

    pub async fn list_pending_approval_commands(&self) -> Result<Vec<CompanionCommand>> {
        let mut out = Vec::new();
        for device in self.list_devices().await? {
            out.extend(
                self.list_commands(&device.id)
                    .await?
                    .into_iter()
                    .filter(|command| command.status == CompanionCommandStatus::ApprovalRequired),
            );
        }
        out.sort_by(|a, b| b.requested_at.cmp(&a.requested_at));
        Ok(out)
    }

    pub async fn rotate_token(
        &self,
        device_id: &str,
        requested_scopes: Vec<String>,
        caller_scopes: &[String],
    ) -> Result<CompanionTokenRotationResult> {
        let mut device = self
            .get_device(device_id)
            .await?
            .ok_or_else(|| anyhow!("device not found"))?;
        anyhow::ensure!(
            device.state != CompanionDeviceState::Revoked,
            "device has been revoked"
        );
        let grant = self
            .get_grant(&device.active_grant_id)
            .await?
            .ok_or_else(|| anyhow!("device grant not found"))?;
        let scopes = if requested_scopes.is_empty() {
            device.token_capabilities.clone()
        } else {
            normalize_capabilities(&requested_scopes)
        };
        validate_capability_set(&scopes)?;
        self.ensure_scopes_allowed(&scopes, &BTreeMap::new(), &grant, &device, caller_scopes)
            .await?;
        let old_fingerprint = device.token_fingerprint.clone();
        let token = generate_secret("acd_", 32)?;
        let new_fingerprint = token_fingerprint(&token);
        device.token_fingerprint = new_fingerprint.clone();
        device.token_capabilities = scopes;
        self.write_device(&device).await?;
        self.audit(
            "device_token_rotated",
            Some(&device.id),
            None,
            Some(&grant.id),
            Some("local_user"),
            "ui",
            "allow",
            "Companion token rotated without expanding beyond device and caller grants.",
            BTreeMap::from([
                (
                    "old_token_fingerprint".to_string(),
                    short_fingerprint(&old_fingerprint),
                ),
                (
                    "new_token_fingerprint".to_string(),
                    short_fingerprint(&new_fingerprint),
                ),
            ]),
        )
        .await?;
        Ok(CompanionTokenRotationResult {
            device,
            device_token: token,
        })
    }

    pub async fn revoke_device(&self, device_id: &str, actor: &str) -> Result<CompanionDevice> {
        let mut device = self
            .get_device(device_id)
            .await?
            .ok_or_else(|| anyhow!("device not found"))?;
        device.state = CompanionDeviceState::Revoked;
        device.token_fingerprint = token_fingerprint(&generate_secret("revoked_", 32)?);
        let mut grant = self
            .get_grant(&device.active_grant_id)
            .await?
            .ok_or_else(|| anyhow!("device grant not found"))?;
        grant.revoked_at = Some(now_rfc3339());
        self.write_grant(&grant).await?;
        self.write_device(&device).await?;
        self.audit(
            "device_revoked",
            Some(&device.id),
            None,
            Some(&grant.id),
            Some(actor),
            "ui",
            "allow",
            "Companion device and active grant revoked.",
            BTreeMap::new(),
        )
        .await?;
        Ok(device)
    }

    pub async fn list_audit_events(&self, limit: usize) -> Result<Vec<CompanionAuditEvent>> {
        let ids = self.read_index(AUDIT_INDEX_KEY).await?;
        let mut out = Vec::new();
        for id in ids
            .into_iter()
            .rev()
            .take(limit.max(1).min(MAX_AUDIT_EVENTS))
        {
            if let Some(event) = self.read_json(&audit_key(&id)).await? {
                out.push(event);
            }
        }
        Ok(out)
    }

    #[allow(clippy::too_many_arguments)]
    async fn audit(
        &self,
        event_type: &str,
        device_id: Option<&str>,
        command_id: Option<&str>,
        grant_id: Option<&str>,
        actor: Option<&str>,
        surface: &str,
        decision: &str,
        reason: &str,
        metadata: BTreeMap<String, String>,
    ) -> Result<()> {
        let mut ids = self.read_index(AUDIT_INDEX_KEY).await?;
        let previous_hash = if let Some(id) = ids.last() {
            self.read_json::<CompanionAuditEvent>(&audit_key(id))
                .await?
                .and_then(|event| event.event_hash)
        } else {
            None
        };
        let mut event = CompanionAuditEvent {
            id: format!("audit-{}", uuid::Uuid::new_v4()),
            event_type: event_type.to_string(),
            device_id: device_id.map(str::to_string),
            command_id: command_id.map(str::to_string),
            grant_id: grant_id.map(str::to_string),
            actor: actor.map(str::to_string),
            surface: Some(surface.to_string()),
            decision: decision.to_string(),
            reason: reason.to_string(),
            timestamp: now_rfc3339(),
            metadata,
            previous_hash,
            event_hash: None,
        };
        event.event_hash = Some(companion_audit_hash(&event)?);
        self.write_json(&audit_key(&event.id), &event).await?;
        ids.push(event.id);
        if ids.len() > MAX_AUDIT_EVENTS {
            let drop_count = ids.len() - MAX_AUDIT_EVENTS;
            ids.drain(0..drop_count);
        }
        self.write_ordered_index(AUDIT_INDEX_KEY, &ids).await
    }
}

fn validate_capability_set(values: &[String]) -> Result<()> {
    let known = capability_catalog()
        .into_iter()
        .map(|cap| cap.id)
        .collect::<BTreeSet<_>>();
    for value in values {
        anyhow::ensure!(
            known.contains(value) || is_custom_capability(value),
            "unknown capability '{}'",
            value
        );
    }
    Ok(())
}

fn capability_risk(id: &str) -> CompanionRiskLevel {
    capability_catalog()
        .into_iter()
        .find(|cap| cap.id == id)
        .map(|cap| cap.risk)
        .unwrap_or(CompanionRiskLevel::High)
}

pub fn capability_catalog() -> Vec<CompanionCapabilityDescriptor> {
    vec![
        cap(
            "approval_prompt",
            "Approval prompts",
            "Receive and answer AgentArk approval prompts.",
            CompanionRiskLevel::Low,
            &[],
        ),
        cap(
            "notifications",
            "Notifications",
            "Receive device notifications from AgentArk.",
            CompanionRiskLevel::Low,
            &[],
        ),
        cap(
            "camera",
            "Camera",
            "Capture images after explicit approval.",
            CompanionRiskLevel::High,
            &["media"],
        ),
        cap(
            "microphone",
            "Microphone",
            "Capture audio after explicit approval.",
            CompanionRiskLevel::High,
            &["media"],
        ),
        cap(
            "photos",
            "Photos",
            "Read or contribute selected photo-library assets.",
            CompanionRiskLevel::High,
            &["media"],
        ),
        cap(
            "location",
            "Location",
            "Share current device location after explicit approval.",
            CompanionRiskLevel::High,
            &["location"],
        ),
        cap(
            "sms",
            "SMS",
            "Send SMS through the paired phone after explicit approval.",
            CompanionRiskLevel::High,
            &["recipient"],
        ),
        cap(
            "whatsapp_handoff",
            "WhatsApp handoff",
            "Prepare or send WhatsApp messages through the paired device.",
            CompanionRiskLevel::High,
            &["recipient"],
        ),
        cap(
            "shortcuts_run",
            "Shortcuts actions",
            "Run Shortcuts-style actions after explicit approval.",
            CompanionRiskLevel::High,
            &["shortcut"],
        ),
        cap(
            "screen_capture",
            "Screen capture",
            "Capture a screenshot after explicit approval.",
            CompanionRiskLevel::High,
            &["screen"],
        ),
        cap(
            "screen_recording",
            "Screen recording",
            "Record screen content after explicit approval.",
            CompanionRiskLevel::High,
            &["screen"],
        ),
        cap(
            "browser_control",
            "Browser control",
            "Control browser sessions on the companion device.",
            CompanionRiskLevel::High,
            &["profile", "origin"],
        ),
        cap(
            "file_read",
            "File read",
            "Read scoped files from the companion device.",
            CompanionRiskLevel::High,
            &["path"],
        ),
        cap(
            "file_write",
            "File write",
            "Write scoped files on the companion device.",
            CompanionRiskLevel::High,
            &["path"],
        ),
        cap(
            "system_run",
            "Local commands",
            "Run typed local commands on the companion device.",
            CompanionRiskLevel::High,
            &["command"],
        ),
        cap(
            "lan_access",
            "LAN access",
            "Reach LAN-only services near the companion device.",
            CompanionRiskLevel::High,
            &["host"],
        ),
        cap(
            "sensor_read",
            "Sensor read",
            "Read custom sensors exposed by the companion device.",
            CompanionRiskLevel::Low,
            &["sensor"],
        ),
        cap(
            "smart_home",
            "Smart home",
            "Control smart-home devices through a local companion.",
            CompanionRiskLevel::High,
            &["device"],
        ),
    ]
}

fn cap(
    id: &str,
    label: &str,
    description: &str,
    risk: CompanionRiskLevel,
    resource_kinds: &[&str],
) -> CompanionCapabilityDescriptor {
    CompanionCapabilityDescriptor {
        id: id.to_string(),
        label: label.to_string(),
        description: description.to_string(),
        risk,
        resource_kinds: resource_kinds
            .iter()
            .map(|value| value.to_string())
            .collect(),
    }
}

pub fn companion_presets() -> Vec<CompanionPreset> {
    vec![
        preset(
            "ios",
            "iPhone / iPad",
            "Pair an iOS device for approvals, notifications, camera/photos, location, and Shortcuts actions.",
            "ios",
            &[
                "approval_prompt",
                "notifications",
                "camera",
                "photos",
                "location",
                "shortcuts_run",
            ],
        ),
        preset(
            "android",
            "Android phone",
            "Pair Android for approvals, notifications, SMS/WhatsApp handoff, camera/photos, and location.",
            "android",
            &[
                "approval_prompt",
                "notifications",
                "sms",
                "whatsapp_handoff",
                "camera",
                "photos",
                "location",
            ],
        ),
        preset(
            "desktop",
            "macOS / Windows / Linux",
            "Pair a desktop agent for screenshots, browser control, scoped files, local commands, and notifications.",
            "desktop",
            &[
                "notifications",
                "screen_capture",
                "browser_control",
                "file_read",
                "file_write",
                "system_run",
            ],
        ),
        preset(
            "home_server",
            "Home server / mini PC",
            "Run scripts, local automations, and private LAN integrations near local resources.",
            "headless",
            &[
                "notifications",
                "system_run",
                "lan_access",
                "file_read",
                "file_write",
            ],
        ),
        preset(
            "raspberry_pi",
            "Raspberry Pi / IoT",
            "Read sensors, control local devices, and expose LAN-only services through a small companion.",
            "iot",
            &["sensor_read", "smart_home", "lan_access", "system_run"],
        ),
        preset(
            "custom",
            "Custom device",
            "Connect any device that implements the AgentArk companion WebSocket protocol.",
            "custom",
            &["approval_prompt", "notifications"],
        ),
    ]
}

fn preset(
    id: &str,
    label: &str,
    description: &str,
    platform: &str,
    capabilities: &[&str],
) -> CompanionPreset {
    CompanionPreset {
        id: id.to_string(),
        label: label.to_string(),
        description: description.to_string(),
        platform: platform.to_string(),
        capability_ids: capabilities.iter().map(|value| value.to_string()).collect(),
    }
}

pub fn presets_response() -> CompanionPresetsResponse {
    CompanionPresetsResponse {
        presets: companion_presets(),
        capabilities: capability_catalog(),
        protocol_version: "agentark-companion-v1".to_string(),
    }
}

pub fn protocol_document() -> CompanionProtocolDocument {
    CompanionProtocolDocument {
        protocol_version: "agentark-companion-v1".to_string(),
        websocket_path: "/companion/ws".to_string(),
        auth: "Native companions reconnect with Authorization: Bearer acd_... and X-AgentArk-Companion-Device: device-.... Browser companions that cannot set WebSocket headers send {\"type\":\"browser_auth\",\"device_id\":\"device-...\",\"token\":\"acd_...\"} over the already-open companion WebSocket.".to_string(),
        pairing: "Create a pairing session in Settings > Companion Devices, then the device sends {\"type\":\"pairing_claim\",\"session_id\":\"...\",\"code\":\"...\",\"device_public_key\":\"stable-device-key\",\"attestation\":{\"provider\":\"app_attest|play_integrity|custom\",\"evidence\":\"...\"}}. Approval is bound to that device identity; after UI approval the approved claim returns the one-time scoped device token on the same WebSocket.".to_string(),
        messages: vec![
            "hello".to_string(),
            "pairing_claim".to_string(),
            "pairing_claim_result".to_string(),
            "browser_auth".to_string(),
            "auth".to_string(),
            "auth_ok".to_string(),
            "auth_error".to_string(),
            "pulse".to_string(),
            "pulse_ok".to_string(),
            "capability_report".to_string(),
            "command_dispatch".to_string(),
            "command_result".to_string(),
            "command_result_ok".to_string(),
            "error".to_string(),
        ],
        security: vec![
            "Requested scopes must be a subset of the paired device grant.".to_string(),
            "Requested scopes must be a subset of the caller's current grant.".to_string(),
            "Production companion connections require TLS; local plaintext is only for development.".to_string(),
            "Pairing approval is bound to the claimed device identity before token issue.".to_string(),
            "Bundled iOS/Android high-risk grants require verified platform attestation.".to_string(),
            "Custom or desktop high-risk grants require an audited trusted_unattested override when attestation is unavailable.".to_string(),
            "Command results are stored as device-reported and not independently proven by the server.".to_string(),
            "High-risk actions require fresh UI approval before dispatch.".to_string(),
            "Capability reports cannot expand grants automatically.".to_string(),
            "Commands are typed JSON actions, not raw free-text instructions.".to_string(),
            "Notification and approval_prompt commands may include optional string arguments: title and body.".to_string(),
        ],
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn scope_subset_requires_every_requested_scope() {
        let requested = vec!["camera".to_string(), "location".to_string()];
        let allowed = vec!["camera".to_string()];
        assert!(!scope_subset(&requested, &allowed));
        let allowed = vec!["camera".to_string(), "location".to_string()];
        assert!(scope_subset(&requested, &allowed));
    }

    #[test]
    fn custom_capabilities_are_structured_not_phrase_based() {
        let capabilities = normalize_capabilities(&[
            " custom.greenhouse_sensor ".to_string(),
            "CUSTOM.GREENHOUSE_SENSOR".to_string(),
        ]);
        assert_eq!(capabilities, vec!["custom.greenhouse_sensor".to_string()]);
        assert!(validate_capability_set(&capabilities).is_ok());
    }
}
