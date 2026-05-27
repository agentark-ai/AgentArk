use anyhow::{Context, Result, anyhow};
use ed25519_dalek::{Signature, Verifier, VerifyingKey};
use regex::Regex;
use serde::{Deserialize, Serialize, de::DeserializeOwned};
use serde_json::{Map, Value};
use sha2::{Digest, Sha256};
use std::collections::{BTreeMap, HashMap, HashSet};
use std::io::{Cursor, Read};
use std::path::{Path, PathBuf};
use zip::ZipArchive;

use crate::storage::Storage;

pub const EXTENSION_PACK_SDK_VERSION: &str = "agentark-extension-pack/v1";
const INSTALLED_PACKS_KEY: &str = "extension_packs:installed:v1";
const CONNECTIONS_KEY: &str = "extension_packs:connections:v1";
const EVENTS_KEY: &str = "extension_packs:events:v1";
const CONNECTION_SECRET_PREFIX: &str = "extension_pack_secret:";
const PACK_KIND_INTEGRATION: &str = "integration";
const PACK_KIND_MESSAGING_CHANNEL: &str = "messaging_channel";
const FEATURE_KIND_CAPABILITY: &str = "capability";
const FEATURE_KIND_RESOURCE: &str = "resource";
const FEATURE_KIND_EVENT: &str = "event";
const BINDING_KIND_HTTP: &str = "http";
const BINDING_KIND_LOCAL_CLI: &str = "local_cli";
const BINDING_KIND_MCP_TOOL: &str = "mcp_tool";
const BINDING_KIND_MCP_RESOURCE: &str = "mcp_resource";
const BINDING_KIND_PLUGIN: &str = "plugin";
const BINDING_KIND_LEGACY_ACTION: &str = "legacy_action";
const BINDING_KIND_LEGACY_CHANNEL: &str = "legacy_channel";
const BINDING_KIND_UNSUPPORTED: &str = "unsupported";
const MAX_STORED_PACK_EVENTS: usize = 256;
const MAX_EVENT_PAYLOAD_CHARS: usize = 16_000;
const MAX_BUNDLE_ENTRIES: usize = 256;
const MAX_BUNDLE_UNCOMPRESSED_BYTES: u64 = 10 * 1024 * 1024;
const MAX_BUNDLE_MANIFEST_BYTES: u64 = 256 * 1024;

#[derive(Debug, Clone, Copy, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "snake_case")]
pub enum ExtensionPackTrustLevel {
    Trusted,
    Unverified,
}

#[derive(Debug, Clone, Copy, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "snake_case")]
pub enum ExtensionPackSourceKind {
    BundledRegistry,
    LocalManifest,
    DirectUrl,
    Scaffolded,
    LocalPath,
    UploadedBundle,
}

#[derive(Debug, Clone, Copy, Serialize, Deserialize, PartialEq, Eq, Default)]
#[serde(rename_all = "snake_case")]
pub enum ExtensionPackAuthMode {
    #[default]
    None,
    ApiKey,
    Basic,
    #[serde(rename = "oauth2_external", alias = "o_auth2_external")]
    OAuth2External,
}

#[derive(Debug, Clone, Copy, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "snake_case")]
pub enum ExtensionConnectionState {
    Disabled,
    NeedsAuth,
    Ready,
    Error,
}

#[derive(Debug, Clone, Copy, Serialize, Deserialize, PartialEq, Eq, Default)]
#[serde(rename_all = "snake_case")]
pub enum ExtensionPackRuntimeStatus {
    #[default]
    NotRequired,
    Missing,
    Ready,
    Error,
}

#[derive(Debug, Clone, Serialize, Deserialize, Default)]
pub struct ExtensionPackDiscoverySpec {
    #[serde(default)]
    pub aliases: Vec<String>,
    #[serde(default)]
    pub package_names: Vec<String>,
    #[serde(default)]
    pub binaries: Vec<String>,
    #[serde(default)]
    pub domains: Vec<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize, Default)]
pub struct ExtensionPackAuthExportSpec {
    #[serde(default)]
    pub env: BTreeMap<String, String>,
    #[serde(default)]
    pub headers: BTreeMap<String, String>,
    #[serde(default)]
    pub query: BTreeMap<String, String>,
}

#[derive(Debug, Clone, Serialize, Deserialize, Default)]
pub struct ExtensionPackOAuth2Spec {
    pub client_id: String,
    pub client_secret: String,
    pub auth_url: String,
    pub token_url: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub redirect_uri: Option<String>,
    #[serde(default)]
    pub scopes: Vec<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub auth_header: Option<String>,
    #[serde(default)]
    pub use_pkce: bool,
    #[serde(default)]
    pub extra_auth_params: BTreeMap<String, String>,
    #[serde(default)]
    pub extra_token_params: BTreeMap<String, String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub prompt: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub access_type: Option<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize, Default)]
pub struct ExtensionPackCommandSpec {
    pub program: String,
    #[serde(default)]
    pub args: Vec<String>,
    #[serde(default)]
    pub env: BTreeMap<String, String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub stdin: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub timeout_secs: Option<u64>,
    #[serde(default)]
    pub os: Vec<String>,
    #[serde(default)]
    pub arch: Vec<String>,
    #[serde(default)]
    pub allow_failure: bool,
}

#[derive(Debug, Clone, Serialize, Deserialize, Default)]
pub struct ExtensionPackInstallerSpec {
    pub executable: String,
    #[serde(default)]
    pub install: Vec<ExtensionPackCommandSpec>,
    #[serde(default)]
    pub verify: Vec<ExtensionPackCommandSpec>,
    #[serde(default)]
    pub update: Vec<ExtensionPackCommandSpec>,
    #[serde(default)]
    pub uninstall: Vec<ExtensionPackCommandSpec>,
    #[serde(default)]
    pub restart_required: bool,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub restart_instructions: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub post_install_note: Option<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize, Default)]
pub struct ExtensionPackRuntimeSpec {
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub installer: Option<ExtensionPackInstallerSpec>,
}

#[derive(Debug, Clone, Serialize, Deserialize, Default)]
pub struct ExtensionPackRuntimeStateRecord {
    #[serde(default)]
    pub status: ExtensionPackRuntimeStatus,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub executable_path: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub last_checked_at: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub last_error: Option<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize, Default)]
pub struct CanonicalFeatureDef {
    pub id: String,
    pub title: String,
    pub description: String,
    #[serde(default)]
    pub kinds: Vec<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize, Default)]
pub struct ExtensionPackAuthSpec {
    #[serde(default)]
    pub mode: ExtensionPackAuthMode,
    #[serde(default)]
    pub required_secrets: Vec<String>,
    #[serde(default)]
    pub required_scopes: Vec<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub oauth2: Option<ExtensionPackOAuth2Spec>,
    #[serde(default)]
    pub exports: ExtensionPackAuthExportSpec,
    #[serde(default)]
    pub metadata: Value,
}

#[derive(Debug, Clone, Serialize, Deserialize, Default)]
pub struct ExtensionPackBinding {
    pub kind: String,
    #[serde(default)]
    pub config: Value,
}

#[derive(Debug, Clone, Serialize, Deserialize, Default)]
pub struct PackFeatureManifest {
    pub id: String,
    #[serde(default = "default_feature_kind")]
    pub kind: String,
    #[serde(default)]
    pub title: String,
    #[serde(default)]
    pub description: String,
    #[serde(default)]
    pub read_only: bool,
    #[serde(default)]
    pub experimental: bool,
    #[serde(default)]
    pub input_schema: Value,
    #[serde(default)]
    pub output_schema: Value,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub binding: Option<ExtensionPackBinding>,
}

#[derive(Debug, Clone, Serialize, Deserialize, Default)]
pub struct ExtensionPackManifest {
    #[serde(default = "default_sdk_version")]
    pub sdk_version: String,
    pub id: String,
    pub name: String,
    pub version: String,
    #[serde(default = "default_pack_kind")]
    pub kind: String,
    #[serde(default)]
    pub publisher: String,
    #[serde(default)]
    pub publisher_did: Option<String>,
    #[serde(default)]
    pub description: String,
    #[serde(default)]
    pub docs_url: Option<String>,
    #[serde(default)]
    pub signature: Option<String>,
    #[serde(default)]
    pub draft: bool,
    #[serde(default)]
    pub tags: Vec<String>,
    #[serde(default)]
    pub auth: ExtensionPackAuthSpec,
    #[serde(default)]
    pub features: Vec<PackFeatureManifest>,
    #[serde(default)]
    pub discovery: ExtensionPackDiscoverySpec,
    #[serde(default)]
    pub runtime: ExtensionPackRuntimeSpec,
    /// Declares the pack as a messaging/notification channel.
    ///
    /// Presence of this field is the authoritative source for "this pack can
    /// be chosen as a `notify_channel`." The `tags` array stays decorative and
    /// is not consulted by the routing / registry layers.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub channel: Option<MessagingChannelSpec>,
    #[serde(default)]
    pub metadata: Value,
}

/// Channel spec on an extension pack. When present, the pack is surfaced by
/// [`crate::channels::messaging_registry`] as a notification target and is
/// dispatched via [`crate::channels::messaging_dispatch`]. The auth required
/// for this channel is sourced from the pack's existing
/// [`ExtensionPackAuthSpec`] and its declared secret slots; this keeps a
/// single auth declaration across tool-invocation and channel-send surfaces.
#[derive(Debug, Clone, Serialize, Deserialize, Default)]
pub struct MessagingChannelSpec {
    /// User-facing display name. Shown in chooser dropdowns. Falls back to
    /// the pack's display name when absent.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub display_name: Option<String>,
    /// Optional per-channel help URL (typically "how to get a webhook URL").
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub docs_url: Option<String>,
    /// HTTP send spec. V1 supports HTTP(S) only — non-HTTP transports (SMTP,
    /// WebSocket, native plugin) are explicitly out of scope.
    pub send: MessagingSendSpec,
}

/// Declarative HTTP "send notification" primitive. The dispatcher substitutes
/// placeholders (`{{text}}`, `{{to}}`, `{{conversation_id}}`, `{{secret:KEY}}`)
/// just before the request is built, so the raw secret values never appear in
/// LLM-bound text.
#[derive(Debug, Clone, Serialize, Deserialize, Default)]
pub struct MessagingSendSpec {
    #[serde(default)]
    pub method: HttpSendMethod,
    /// Target URL template. May contain placeholders.
    pub url_template: String,
    /// Static + templated headers applied to every request.
    #[serde(default)]
    pub headers: Vec<MessagingHeaderSpec>,
    /// Request body template. `None` means an empty body (usual for webhooks
    /// that accept an empty POST). The body is sent as-is after substitution;
    /// placeholder values are JSON-escaped when `content_type` is JSON so
    /// user-provided text can never break JSON structure.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub body_template: Option<String>,
    /// Defaults to `application/json`.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub content_type: Option<String>,
    /// How auth is attached to the request.
    #[serde(default)]
    pub auth: AuthTransportBinding,
    /// HTTP status codes that mean "success." Default is any 2xx.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub expect_status: Option<Vec<u16>>,
}

#[derive(Debug, Clone, Copy, Serialize, Deserialize, PartialEq, Eq, Default)]
#[serde(rename_all = "lowercase")]
pub enum HttpSendMethod {
    #[default]
    Post,
    Put,
    Patch,
    Get,
    Delete,
}

impl HttpSendMethod {
    /// Used by the HTTP template dispatcher in
    /// `crate::channels::messaging_dispatch`. That module is currently
    /// behind a `#![allow(dead_code)]` until a caller inside the notify_user
    /// path plugs into it — mirror the tag here so rustc doesn't also flag
    /// this helper transitively.
    #[allow(dead_code)]
    pub fn as_reqwest(self) -> reqwest::Method {
        match self {
            HttpSendMethod::Post => reqwest::Method::POST,
            HttpSendMethod::Put => reqwest::Method::PUT,
            HttpSendMethod::Patch => reqwest::Method::PATCH,
            HttpSendMethod::Get => reqwest::Method::GET,
            HttpSendMethod::Delete => reqwest::Method::DELETE,
        }
    }
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct MessagingHeaderSpec {
    pub name: String,
    /// Value template; supports the same `{{…}}` placeholders as body/url.
    pub value_template: String,
}

/// How the pack's auth material is applied to the outgoing HTTP request.
/// Covers the common industry patterns; expanded cases (SigV4, JWT sign-on-
/// demand) can be added as new variants without changing the registry or
/// dispatch surfaces.
#[derive(Debug, Clone, Serialize, Deserialize, Default)]
#[serde(tag = "kind", rename_all = "snake_case")]
pub enum AuthTransportBinding {
    /// No explicit auth header; auth is embedded in the url/body via
    /// `{{secret:KEY}}` substitution (e.g. Slack incoming webhook URLs).
    #[default]
    None,
    /// Attaches `Authorization: Bearer <value>` where `<value>` is the
    /// current value stored at the given secret key.
    Bearer { secret_key: String },
    /// Custom header whose value is templated (supports `{{secret:KEY}}`).
    CustomHeader {
        name: String,
        value_template: String,
    },
    /// HTTP Basic auth using two secret slots.
    Basic {
        username_key: String,
        password_key: String,
    },
    /// Adds `?<name>=<value>` to the request URL; value supports placeholders.
    QueryParam {
        name: String,
        value_template: String,
    },
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct InstalledExtensionPack {
    pub manifest: ExtensionPackManifest,
    pub trust_level: ExtensionPackTrustLevel,
    #[serde(default = "default_verification_status")]
    pub verification_status: String,
    #[serde(default)]
    pub verification_detail: Option<String>,
    pub source_kind: ExtensionPackSourceKind,
    #[serde(default)]
    pub source_url: Option<String>,
    pub enabled: bool,
    #[serde(default)]
    pub runtime_state: ExtensionPackRuntimeStateRecord,
    pub installed_at: String,
    pub updated_at: String,
}

#[derive(Debug, Clone, Serialize)]
pub struct PackFeatureSummary {
    pub id: String,
    pub kind: String,
    pub title: String,
    pub description: String,
    pub read_only: bool,
    pub experimental: bool,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub binding_kind: Option<String>,
}

#[derive(Debug, Clone, Serialize)]
pub struct ExtensionPackView {
    pub manifest: ExtensionPackManifest,
    pub installed: bool,
    pub enabled: bool,
    pub trust_level: ExtensionPackTrustLevel,
    pub verification_status: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub verification_detail: Option<String>,
    pub source_kind: ExtensionPackSourceKind,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub source_url: Option<String>,
    pub needs_auth: bool,
    pub status: String,
    pub status_detail: Option<String>,
    pub runtime_required: bool,
    pub runtime_status: ExtensionPackRuntimeStatus,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub runtime_detail: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub runtime_executable: Option<String>,
    pub supports_connect_url: bool,
    pub supports_webhook: bool,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub webhook_path: Option<String>,
    pub feature_summaries: Vec<PackFeatureSummary>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ExtensionPackConnection {
    pub id: String,
    pub pack_id: String,
    pub name: String,
    pub enabled: bool,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub auth_profile_id: Option<String>,
    #[serde(default)]
    pub metadata: Value,
    #[serde(default)]
    pub last_error: Option<String>,
    #[serde(default)]
    pub last_tested_at: Option<String>,
    pub created_at: String,
    pub updated_at: String,
}

#[derive(Debug, Clone, Serialize)]
pub struct ExtensionPackConnectionView {
    pub connection: ExtensionPackConnection,
    pub state: ExtensionConnectionState,
    pub auth_mode: ExtensionPackAuthMode,
    pub has_secret: bool,
}

#[derive(Debug, Clone, Serialize)]
pub struct ExtensionPackRuntimeResult {
    pub pack_id: String,
    pub status: ExtensionPackRuntimeStatus,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub executable_path: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub detail: Option<String>,
    #[serde(default)]
    pub restart_required: bool,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub restart_instructions: Option<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ExtensionPackEventRecord {
    pub id: String,
    pub pack_id: String,
    pub feature_id: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub connection_id: Option<String>,
    pub event_type: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub provider_event_id: Option<String>,
    pub transport: String,
    pub status: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub outcome: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub response_preview: Option<String>,
    #[serde(default)]
    pub metadata: Value,
    #[serde(default)]
    pub payload: Value,
    pub received_at: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub processed_at: Option<String>,
}

#[derive(Debug, Clone, Serialize)]
pub struct ExtensionPackEventsResponse {
    pub pack_id: String,
    pub count: usize,
    pub items: Vec<ExtensionPackEventRecord>,
}

#[derive(Debug, Clone)]
pub struct ResolvedExtensionPackWebhook {
    pub manifest: ExtensionPackManifest,
    pub feature: PackFeatureManifest,
    pub connection_id: Option<String>,
    pub secret: Option<Value>,
}

#[derive(Debug, Clone, Deserialize, Default)]
pub struct ExtensionPackInstallRequest {
    #[serde(default)]
    pub pack_id: Option<String>,
    #[serde(default)]
    pub source_url: Option<String>,
    #[serde(default)]
    pub source_path: Option<String>,
    #[serde(default)]
    pub manifest: Option<ExtensionPackManifest>,
    #[serde(default)]
    pub manifest_text: Option<String>,
    #[serde(default)]
    pub trust_unverified: bool,
}

#[derive(Debug, Clone, Deserialize, Default)]
pub struct ExtensionPackScaffoldRequest {
    pub name: String,
    #[serde(default = "default_pack_kind")]
    pub kind: String,
    #[serde(default)]
    pub description: String,
    #[serde(default)]
    pub docs_url: Option<String>,
    #[serde(default)]
    pub openapi_url: Option<String>,
    #[serde(default)]
    pub openapi_text: Option<String>,
    #[serde(default)]
    pub curl_text: Option<String>,
    #[serde(default)]
    pub auth_mode: ExtensionPackAuthMode,
    #[serde(default)]
    pub desired_features: Vec<String>,
    #[serde(default)]
    pub read_only: bool,
    #[serde(default)]
    pub binding_kind: Option<String>,
    #[serde(default)]
    pub publisher: Option<String>,
    #[serde(default)]
    pub tags: Vec<String>,
}

#[derive(Debug, Clone, Deserialize, Default)]
pub struct ExtensionPackConnectionUpsertRequest {
    #[serde(default)]
    pub connection_id: Option<String>,
    #[serde(default)]
    pub name: Option<String>,
    #[serde(default)]
    pub enabled: Option<bool>,
    #[serde(default)]
    pub metadata: Option<Value>,
    #[serde(default)]
    pub secret: Option<Value>,
    #[serde(default)]
    pub clear_secret: bool,
}

#[derive(Debug, Clone, Deserialize, Default)]
pub struct ExtensionPackInvokeRequest {
    #[serde(default)]
    pub pack_id: Option<String>,
    #[serde(default)]
    pub connection_id: Option<String>,
    pub feature_id: String,
    #[serde(default)]
    pub arguments: Value,
}

#[derive(Debug, Clone, Serialize)]
pub struct ExtensionPackInvokeResult {
    pub ok: bool,
    pub status: String,
    pub pack_id: String,
    pub feature_id: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub connection_id: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub message: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub data: Option<Value>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub error: Option<String>,
}

#[derive(Debug, Clone, Serialize)]
pub struct ExtensionPackSearchResponse {
    pub query: String,
    pub installed: Vec<ExtensionPackView>,
    pub catalog: Vec<ExtensionPackView>,
    pub not_found: bool,
    pub next_steps: Vec<String>,
}

pub struct ExtensionPackRegistry {
    storage: Storage,
    config_dir: PathBuf,
    data_dir: PathBuf,
    http_client: reqwest::Client,
    installed: HashMap<String, InstalledExtensionPack>,
    connections: HashMap<String, ExtensionPackConnection>,
    events: Vec<ExtensionPackEventRecord>,
}

#[derive(Debug, Clone)]
struct GenericHealthProbe {
    feature_id: String,
    arguments: Value,
    source: String,
}

#[derive(Debug, Clone)]
struct RuntimePackActionSpec {
    info: crate::actions::ActionDef,
    binding: crate::runtime::ExtensionPackActionBinding,
}

fn default_sdk_version() -> String {
    EXTENSION_PACK_SDK_VERSION.to_string()
}

fn default_pack_kind() -> String {
    PACK_KIND_INTEGRATION.to_string()
}

fn default_feature_kind() -> String {
    FEATURE_KIND_CAPABILITY.to_string()
}

fn default_verification_status() -> String {
    "unverified".to_string()
}

fn now_rfc3339() -> String {
    chrono::Utc::now().to_rfc3339()
}

fn sanitize_pack_id(raw: &str) -> String {
    raw.trim()
        .to_ascii_lowercase()
        .chars()
        .map(|ch| {
            if ch.is_ascii_alphanumeric() || matches!(ch, '-' | '_') {
                ch
            } else if ch.is_ascii_whitespace() || matches!(ch, '/' | '\\' | '.') {
                '-'
            } else {
                '_'
            }
        })
        .collect::<String>()
        .trim_matches(|ch| ch == '-' || ch == '_')
        .to_string()
}

fn default_runtime_state_for_manifest(
    manifest: &ExtensionPackManifest,
) -> ExtensionPackRuntimeStateRecord {
    if manifest_requires_runtime(manifest) {
        ExtensionPackRuntimeStateRecord {
            status: ExtensionPackRuntimeStatus::Missing,
            executable_path: None,
            last_checked_at: None,
            last_error: None,
        }
    } else {
        ExtensionPackRuntimeStateRecord {
            status: ExtensionPackRuntimeStatus::NotRequired,
            executable_path: None,
            last_checked_at: None,
            last_error: None,
        }
    }
}

fn connection_auth_profile_id(pack_id: &str, connection_id: &str) -> String {
    format!(
        "extension_pack_{}_{}",
        sanitize_pack_id(pack_id),
        sanitize_pack_id(connection_id)
    )
}

fn binding_kind(feature: &PackFeatureManifest) -> Option<String> {
    feature
        .binding
        .as_ref()
        .map(|binding| binding.kind.trim().to_ascii_lowercase())
        .filter(|kind| !kind.is_empty())
}

fn feature_supports_generic_probe(feature: &PackFeatureManifest) -> bool {
    matches!(
        binding_kind(feature).as_deref(),
        Some(
            BINDING_KIND_HTTP
                | BINDING_KIND_MCP_TOOL
                | BINDING_KIND_MCP_RESOURCE
                | BINDING_KIND_PLUGIN
                | BINDING_KIND_LEGACY_ACTION
        )
    )
}

fn required_feature_inputs(feature: &PackFeatureManifest) -> Vec<String> {
    feature
        .input_schema
        .get("required")
        .and_then(|value| value.as_array())
        .map(|items| {
            items
                .iter()
                .filter_map(|value| value.as_str())
                .map(str::trim)
                .filter(|value| !value.is_empty())
                .map(|value| value.to_string())
                .collect::<Vec<_>>()
        })
        .unwrap_or_default()
}

fn is_exact_argument_template(value: &Value, argument_name: &str) -> bool {
    matches!(
        value,
        Value::String(text)
            if text.trim()
                == format!("{{{{arg.{}}}}}", argument_name)
    )
}

fn feature_accepts_empty_probe_arguments(feature: &PackFeatureManifest) -> bool {
    let required = required_feature_inputs(feature);
    if required.is_empty() {
        return true;
    }
    let Some(binding) = feature.binding.as_ref() else {
        return false;
    };
    if !binding.kind.eq_ignore_ascii_case(BINDING_KIND_HTTP) {
        return false;
    }
    required.iter().all(|required_name| {
        if let Some(value) = binding
            .config
            .get("query")
            .and_then(|item| item.as_object())
            .and_then(|map| map.get(required_name))
        {
            return !is_exact_argument_template(value, required_name);
        }
        if let Some(value) = binding
            .config
            .get("headers")
            .and_then(|item| item.as_object())
            .and_then(|map| map.get(required_name))
        {
            return !is_exact_argument_template(value, required_name);
        }
        if required_name == "body" {
            if let Some(value) = binding.config.get("body") {
                return !is_exact_argument_template(value, "body");
            }
        }
        false
    })
}

fn resolve_configured_health_probe(manifest: &ExtensionPackManifest) -> Option<GenericHealthProbe> {
    let configured = manifest.metadata.get("health_probe")?.as_object()?;
    let feature_id = configured
        .get("feature_id")
        .and_then(|value| value.as_str())
        .map(str::trim)
        .filter(|value| !value.is_empty())?
        .to_string();
    let arguments = configured
        .get("arguments")
        .cloned()
        .unwrap_or_else(|| serde_json::json!({}));
    Some(GenericHealthProbe {
        feature_id,
        arguments,
        source: "manifest".to_string(),
    })
}

fn infer_generic_health_probe(manifest: &ExtensionPackManifest) -> Option<GenericHealthProbe> {
    let feature = manifest.features.iter().find(|feature| {
        feature.read_only
            && feature_supports_generic_probe(feature)
            && feature_accepts_empty_probe_arguments(feature)
    })?;
    Some(GenericHealthProbe {
        feature_id: feature.id.clone(),
        arguments: serde_json::json!({}),
        source: "inferred".to_string(),
    })
}

fn resolve_generic_health_probe(manifest: &ExtensionPackManifest) -> Option<GenericHealthProbe> {
    resolve_configured_health_probe(manifest).or_else(|| infer_generic_health_probe(manifest))
}

fn pack_matches_kind(manifest: &ExtensionPackManifest, kind: Option<&str>) -> bool {
    let Some(kind) = kind.map(str::trim).filter(|value| !value.is_empty()) else {
        return true;
    };
    manifest.kind.eq_ignore_ascii_case(kind)
}

fn normalize_search_text(value: &str) -> String {
    value
        .to_ascii_lowercase()
        .chars()
        .map(|ch| match ch {
            '-' | '_' | '.' | '/' | '\\' => ' ',
            _ => ch,
        })
        .collect()
}

fn normalized_search_match(candidate: &str, normalized_query: &str, compact_query: &str) -> bool {
    candidate.contains(normalized_query)
        || (!compact_query.is_empty()
            && candidate
                .split_whitespace()
                .collect::<String>()
                .contains(compact_query))
}

fn pack_matches_query(manifest: &ExtensionPackManifest, query: Option<&str>) -> bool {
    let Some(query) = query.map(str::trim).filter(|value| !value.is_empty()) else {
        return true;
    };
    let normalized_query = normalize_search_text(query);
    if normalized_query.is_empty() {
        return true;
    }
    let compact_query = normalized_query.split_whitespace().collect::<String>();
    let query_terms = normalized_query
        .split_whitespace()
        .filter(|term| term.len() >= 3)
        .collect::<Vec<_>>();
    let mut identity_haystack = vec![
        normalize_search_text(&manifest.id),
        normalize_search_text(&manifest.name),
    ];
    identity_haystack.extend(
        manifest
            .discovery
            .aliases
            .iter()
            .map(|value| normalize_search_text(value)),
    );
    let mut discovery_haystack = Vec::new();
    discovery_haystack.extend(
        manifest
            .discovery
            .package_names
            .iter()
            .map(|value| normalize_search_text(value)),
    );
    discovery_haystack.extend(
        manifest
            .discovery
            .binaries
            .iter()
            .map(|value| normalize_search_text(value)),
    );
    discovery_haystack.extend(
        manifest
            .discovery
            .domains
            .iter()
            .map(|value| normalize_search_text(value)),
    );
    let mut context_haystack = vec![
        normalize_search_text(&manifest.description),
        normalize_search_text(&manifest.publisher),
    ];
    context_haystack.extend(manifest.tags.iter().map(|tag| normalize_search_text(tag)));
    let mut full_haystack = identity_haystack.clone();
    full_haystack.extend(discovery_haystack.iter().cloned());
    full_haystack.extend(context_haystack);
    if full_haystack
        .iter()
        .any(|candidate| normalized_search_match(candidate, &normalized_query, &compact_query))
    {
        return true;
    }
    if query_terms.is_empty() {
        return false;
    }
    if identity_haystack
        .iter()
        .chain(discovery_haystack.iter())
        .any(|candidate| query_terms.iter().any(|term| candidate.contains(term)))
    {
        return true;
    }
    query_terms.iter().all(|term| {
        full_haystack
            .iter()
            .any(|candidate| candidate.contains(term))
    })
}

fn connection_state_priority(state: ExtensionConnectionState) -> usize {
    match state {
        ExtensionConnectionState::Ready => 0,
        ExtensionConnectionState::NeedsAuth => 1,
        ExtensionConnectionState::Error => 2,
        ExtensionConnectionState::Disabled => 3,
    }
}

fn connection_secret_key(pack_id: &str, connection_id: &str) -> String {
    format!(
        "{}{}:{}",
        CONNECTION_SECRET_PREFIX,
        sanitize_pack_id(pack_id),
        sanitize_pack_id(connection_id)
    )
}

fn connection_secret_prefix(pack_id: &str) -> String {
    format!("{}{}:", CONNECTION_SECRET_PREFIX, sanitize_pack_id(pack_id))
}

fn pack_supports_connect_url_manifest(manifest: &ExtensionPackManifest) -> bool {
    matches!(manifest.auth.mode, ExtensionPackAuthMode::OAuth2External)
        && (manifest.id.eq_ignore_ascii_case("google_workspace") || manifest.auth.oauth2.is_some())
}

fn manifest_uses_connection_secret(manifest: &ExtensionPackManifest) -> bool {
    if !manifest.auth.required_secrets.is_empty() {
        return true;
    }
    if manifest
        .auth
        .metadata
        .get("secret_field")
        .and_then(Value::as_str)
        .is_some_and(|value| !value.trim().is_empty())
    {
        return true;
    }
    if value_contains_secret_template(
        &serde_json::to_value(&manifest.auth.exports).unwrap_or_default(),
    ) {
        return true;
    }
    manifest.features.iter().any(|feature| {
        let Some(binding) = feature.binding.as_ref() else {
            return false;
        };
        if value_contains_secret_template(&binding.config) {
            return true;
        }
        let Some(auth) = binding.config.get("auth").and_then(Value::as_object) else {
            return false;
        };
        let auth_type = auth
            .get("type")
            .and_then(Value::as_str)
            .map(str::trim)
            .unwrap_or_default();
        if auth_type.eq_ignore_ascii_case("basic") {
            return true;
        }
        auth.get("secret_path")
            .and_then(Value::as_str)
            .is_some_and(|value| !value.trim().is_empty())
    })
}

fn pack_supports_webhook_manifest(manifest: &ExtensionPackManifest) -> bool {
    manifest.features.iter().any(|feature| {
        feature.kind.eq_ignore_ascii_case(FEATURE_KIND_EVENT)
            && feature.id.eq_ignore_ascii_case("message.receive")
            && feature.binding.as_ref().is_some_and(|binding| {
                let kind = binding.kind.trim().to_ascii_lowercase();
                kind == BINDING_KIND_LEGACY_CHANNEL || kind == BINDING_KIND_HTTP
            })
    })
}

fn canonical_feature_ids() -> HashSet<String> {
    ExtensionPackRegistry::canonical_features()
        .into_iter()
        .map(|item| item.id)
        .collect()
}

fn supported_binding_kinds() -> HashSet<&'static str> {
    HashSet::from([
        BINDING_KIND_HTTP,
        BINDING_KIND_LOCAL_CLI,
        BINDING_KIND_MCP_TOOL,
        BINDING_KIND_MCP_RESOURCE,
        BINDING_KIND_PLUGIN,
        BINDING_KIND_LEGACY_ACTION,
        BINDING_KIND_LEGACY_CHANNEL,
        BINDING_KIND_UNSUPPORTED,
    ])
}

fn runtime_installer(manifest: &ExtensionPackManifest) -> Option<&ExtensionPackInstallerSpec> {
    manifest.runtime.installer.as_ref()
}

fn manifest_requires_runtime(manifest: &ExtensionPackManifest) -> bool {
    runtime_installer(manifest).is_some()
}

fn current_platform_os() -> &'static str {
    std::env::consts::OS
}

fn current_platform_arch() -> &'static str {
    std::env::consts::ARCH
}

fn selector_matches(values: &[String], actual: &str) -> bool {
    values.is_empty()
        || values
            .iter()
            .map(|value| value.trim())
            .filter(|value| !value.is_empty())
            .any(|value| value.eq_ignore_ascii_case(actual))
}

fn command_matches_platform(command: &ExtensionPackCommandSpec) -> bool {
    selector_matches(&command.os, current_platform_os())
        && selector_matches(&command.arch, current_platform_arch())
}

fn runtime_state_for_manifest(
    manifest: &ExtensionPackManifest,
    state: &ExtensionPackRuntimeStateRecord,
) -> (ExtensionPackRuntimeStatus, Option<String>, Option<String>) {
    let Some(installer) = runtime_installer(manifest) else {
        return (ExtensionPackRuntimeStatus::NotRequired, None, None);
    };
    let detail = state.last_error.clone().or_else(|| match state.status {
        ExtensionPackRuntimeStatus::Missing => Some(format!(
            "Runtime executable '{}' has not been installed yet.",
            installer.executable
        )),
        _ => None,
    });
    (
        state.status,
        detail,
        Some(installer.executable.clone()).filter(|value| !value.trim().is_empty()),
    )
}

fn runtime_operation_detail(
    installer: &ExtensionPackInstallerSpec,
    detail: Option<String>,
) -> Option<String> {
    let mut parts = Vec::new();
    if let Some(text) = detail
        .map(|value| value.trim().to_string())
        .filter(|value| !value.is_empty())
    {
        parts.push(text);
    }
    if let Some(text) = installer
        .post_install_note
        .as_deref()
        .map(str::trim)
        .filter(|value| !value.is_empty())
    {
        parts.push(text.to_string());
    }
    if installer.restart_required {
        parts.push(
            installer
                .restart_instructions
                .as_deref()
                .map(str::trim)
                .filter(|value| !value.is_empty())
                .unwrap_or("Restart the target app or shell session before using this integration.")
                .to_string(),
        );
    }
    if parts.is_empty() {
        None
    } else {
        Some(parts.join(" "))
    }
}

fn verifying_key_from_did(did: &str) -> Result<VerifyingKey> {
    let multibase = did
        .trim()
        .strip_prefix("did:key:z")
        .ok_or_else(|| anyhow!("Unsupported publisher DID '{}'", did))?;
    let decoded = bs58::decode(multibase)
        .into_vec()
        .map_err(|error| anyhow!("Invalid publisher DID '{}': {}", did, error))?;
    if decoded.len() != 34 || decoded[0] != 0xed || decoded[1] != 0x01 {
        anyhow::bail!("Unsupported publisher DID multicodec for '{}'", did);
    }
    let key_bytes: [u8; 32] = decoded[2..]
        .try_into()
        .map_err(|_| anyhow!("Invalid publisher DID key length for '{}'", did))?;
    VerifyingKey::from_bytes(&key_bytes)
        .map_err(|error| anyhow!("Invalid publisher DID verifying key '{}': {}", did, error))
}

fn manifest_signing_bytes(manifest: &ExtensionPackManifest) -> Result<Vec<u8>> {
    let mut canonical = manifest.clone();
    canonical.signature = None;
    serde_json::to_vec(&canonical).context("failed to canonicalize extension-pack manifest")
}

fn manifest_bundle_hash(manifest: &ExtensionPackManifest) -> Result<String> {
    let bytes = manifest_signing_bytes(manifest)?;
    let mut hasher = Sha256::new();
    hasher.update(&bytes);
    Ok(hex::encode(hasher.finalize()))
}

fn verify_manifest_signature(manifest: &ExtensionPackManifest) -> Result<Option<String>> {
    let Some(signature_hex) = manifest.signature.as_deref().map(str::trim) else {
        anyhow::bail!("Pack '{}' is missing publisher signature", manifest.id);
    };
    if signature_hex.is_empty() || signature_hex.eq_ignore_ascii_case("bundled") {
        anyhow::bail!("Pack '{}' is missing publisher signature", manifest.id);
    }
    let publisher_did = manifest
        .publisher_did
        .as_deref()
        .map(str::trim)
        .filter(|value| !value.is_empty())
        .ok_or_else(|| anyhow!("Signed pack '{}' is missing publisher_did", manifest.id))?;
    let verifying_key = verifying_key_from_did(publisher_did)?;
    let sig_bytes = hex::decode(signature_hex).map_err(|_| {
        anyhow!(
            "Pack '{}' has an invalid signature hex payload",
            manifest.id
        )
    })?;
    let signature = Signature::from_bytes(
        sig_bytes
            .as_slice()
            .try_into()
            .map_err(|_| anyhow!("Pack '{}' has an invalid signature length", manifest.id))?,
    );
    let bundle_hash = manifest_bundle_hash(manifest)?;
    verifying_key
        .verify(bundle_hash.as_bytes(), &signature)
        .map_err(|_| anyhow!("Pack '{}' failed signature verification", manifest.id))?;
    Ok(Some(format!("Verified publisher DID {}.", publisher_did)))
}

fn channel_pack(
    id: &str,
    name: &str,
    description: &str,
    required_secrets: &[&str],
) -> ExtensionPackManifest {
    ExtensionPackManifest {
        sdk_version: default_sdk_version(),
        id: id.to_string(),
        name: name.to_string(),
        version: "1.0.0".to_string(),
        kind: PACK_KIND_MESSAGING_CHANNEL.to_string(),
        publisher: crate::branding::PRODUCT_NAME.to_string(),
        publisher_did: None,
        description: description.to_string(),
        docs_url: None,
        signature: Some("bundled".to_string()),
        draft: false,
        tags: vec!["messaging".to_string(), "channel".to_string()],
        auth: ExtensionPackAuthSpec {
            mode: ExtensionPackAuthMode::ApiKey,
            required_secrets: required_secrets
                .iter()
                .map(|value| value.to_string())
                .collect(),
            required_scopes: Vec::new(),
            oauth2: None,
            exports: ExtensionPackAuthExportSpec::default(),
            metadata: serde_json::json!({
                "secret_shape": required_secrets
            }),
        },
        features: vec![
            PackFeatureManifest {
                id: "message.receive".to_string(),
                kind: FEATURE_KIND_EVENT.to_string(),
                title: "Receive inbound messages".to_string(),
                description: format!("Receive inbound {} messages.", name),
                read_only: true,
                experimental: false,
                input_schema: Value::Null,
                output_schema: Value::Null,
                binding: Some(ExtensionPackBinding {
                    kind: BINDING_KIND_LEGACY_CHANNEL.to_string(),
                    config: serde_json::json!({
                        "channel_id": id,
                        "operation": "receive"
                    }),
                }),
            },
            PackFeatureManifest {
                id: "message.send".to_string(),
                kind: FEATURE_KIND_CAPABILITY.to_string(),
                title: "Send messages".to_string(),
                description: format!("Send proactive {} messages.", name),
                read_only: false,
                experimental: false,
                input_schema: Value::Null,
                output_schema: Value::Null,
                binding: Some(ExtensionPackBinding {
                    kind: BINDING_KIND_LEGACY_CHANNEL.to_string(),
                    config: serde_json::json!({
                        "channel_id": id,
                        "operation": "send"
                    }),
                }),
            },
            PackFeatureManifest {
                id: "message.list_threads".to_string(),
                kind: FEATURE_KIND_RESOURCE.to_string(),
                title: "Inspect default thread routing".to_string(),
                description: format!(
                    "Inspect the default {} delivery target and thread context.",
                    name
                ),
                read_only: true,
                experimental: false,
                input_schema: Value::Null,
                output_schema: Value::Null,
                binding: Some(ExtensionPackBinding {
                    kind: BINDING_KIND_LEGACY_CHANNEL.to_string(),
                    config: serde_json::json!({
                        "channel_id": id,
                        "operation": "list_threads"
                    }),
                }),
            },
        ],
        discovery: ExtensionPackDiscoverySpec {
            aliases: vec![name.to_ascii_lowercase(), id.to_ascii_lowercase()],
            ..ExtensionPackDiscoverySpec::default()
        },
        runtime: ExtensionPackRuntimeSpec::default(),
        channel: None,
        metadata: serde_json::json!({
            "builtin_channel": true,
            "legacy_runtime": true
        }),
    }
}

fn bundled_catalog() -> Vec<ExtensionPackManifest> {
    vec![
        ExtensionPackManifest {
            sdk_version: default_sdk_version(),
            id: "google_workspace".to_string(),
            name: "Google Workspace".to_string(),
            version: "1.0.0".to_string(),
            kind: PACK_KIND_INTEGRATION.to_string(),
            publisher: crate::branding::PRODUCT_NAME.to_string(),
            publisher_did: None,
            description: format!(
                "Connect Google Workspace once, then let {} use Gmail, Calendar, Drive/Docs, Chat, and Admin APIs through one reusable pack.",
                crate::branding::PRODUCT_NAME
            ),
            docs_url: None,
            signature: Some("bundled".to_string()),
            draft: false,
            tags: vec![
                "google".to_string(),
                "workspace".to_string(),
                "integration".to_string(),
            ],
            auth: ExtensionPackAuthSpec {
                mode: ExtensionPackAuthMode::OAuth2External,
                required_secrets: Vec::new(),
                required_scopes: vec![
                    "gmail".to_string(),
                    "calendar".to_string(),
                    "drive".to_string(),
                    "docs".to_string(),
                    "chat".to_string(),
                ],
                oauth2: None,
                exports: ExtensionPackAuthExportSpec {
                    env: BTreeMap::from([
                        (
                            "GWS_AUTH_HEADER".to_string(),
                            "authorization_header".to_string(),
                        ),
                        ("GWS_ACCESS_TOKEN".to_string(), "access_token".to_string()),
                        ("GWS_REFRESH_TOKEN".to_string(), "refresh_token".to_string()),
                    ]),
                    ..ExtensionPackAuthExportSpec::default()
                },
                metadata: serde_json::json!({
                    "builtin_integration_id": "google_workspace"
                }),
            },
            features: vec![
                ("mail.list", "List Gmail messages", true, "gmail_scan"),
                ("mail.send", "Send or reply in Gmail", false, "gmail_reply"),
                (
                    "calendar.list_events",
                    "List Calendar events",
                    true,
                    "calendar_list",
                ),
                (
                    "calendar.create_event",
                    "Create Calendar events",
                    false,
                    "calendar_create",
                ),
                (
                    "files.search",
                    "Search Google Drive",
                    true,
                    "google_drive_search",
                ),
                ("files.read", "Read Google Docs", true, "google_docs_read"),
                (
                    "chat.list_spaces",
                    "List Google Chat spaces",
                    true,
                    "google_chat_list_spaces",
                ),
            ]
            .into_iter()
            .map(|(id, title, read_only, action_name)| PackFeatureManifest {
                id: id.to_string(),
                kind: FEATURE_KIND_CAPABILITY.to_string(),
                title: title.to_string(),
                description: title.to_string(),
                read_only,
                experimental: false,
                input_schema: Value::Null,
                output_schema: Value::Null,
                binding: Some(ExtensionPackBinding {
                    kind: BINDING_KIND_LEGACY_ACTION.to_string(),
                    config: serde_json::json!({
                        "action_name": action_name
                    }),
                }),
            })
            .collect(),
            discovery: ExtensionPackDiscoverySpec {
                aliases: vec![
                    "google workspace".to_string(),
                    "gws".to_string(),
                    "gmail".to_string(),
                    "google calendar".to_string(),
                    "google drive".to_string(),
                ],
                binaries: vec!["gws".to_string()],
                domains: vec![
                    "googleapis.com".to_string(),
                    "workspace.google.com".to_string(),
                ],
                ..ExtensionPackDiscoverySpec::default()
            },
            runtime: ExtensionPackRuntimeSpec {
                installer: Some(ExtensionPackInstallerSpec {
                    executable: "gws".to_string(),
                    install: Vec::new(),
                    verify: Vec::new(),
                    update: Vec::new(),
                    uninstall: Vec::new(),
                    restart_required: false,
                    restart_instructions: None,
                    post_install_note: None,
                }),
            },
            channel: None,
            metadata: serde_json::json!({
                "builtin_integration_id": "google_workspace"
            }),
        },
        channel_pack(
            "slack_channel",
            "Slack Channel",
            "Route Slack through the generic pack framework while still reusing the built-in Slack transport runtime for delivery and inbound handling.",
            &["bot_token", "default_channel_id"],
        ),
        channel_pack(
            "teams_channel",
            "Microsoft Teams Channel",
            "Route Teams through the generic pack framework while still reusing the built-in Teams transport runtime for delivery and inbound handling.",
            &["service_url", "access_token", "bot_app_id"],
        ),
        channel_pack(
            "whatsapp_channel",
            "WhatsApp Channel",
            "Route WhatsApp through the generic pack framework with support for Cloud API or bridge-backed delivery from the same pack runtime.",
            &["mode"],
        ),
    ]
}

fn validate_manifest(
    manifest: &ExtensionPackManifest,
    source_kind: ExtensionPackSourceKind,
) -> Result<()> {
    if manifest.sdk_version.trim() != EXTENSION_PACK_SDK_VERSION {
        anyhow::bail!(
            "Unsupported extension-pack SDK version '{}'. Expected '{}'.",
            manifest.sdk_version.trim(),
            EXTENSION_PACK_SDK_VERSION
        );
    }
    if sanitize_pack_id(&manifest.id).is_empty() {
        anyhow::bail!("Pack manifest id is required");
    }
    if manifest.name.trim().is_empty() {
        anyhow::bail!("Pack manifest name is required");
    }
    let canonical = canonical_feature_ids();
    let supported_bindings = supported_binding_kinds();
    let mut feature_ids = HashSet::new();
    for feature in &manifest.features {
        let normalized = feature.id.trim().to_ascii_lowercase();
        if normalized.is_empty() {
            anyhow::bail!("Pack feature ids must be non-empty");
        }
        if !feature_ids.insert(normalized.clone()) {
            anyhow::bail!("Duplicate pack feature '{}'", feature.id);
        }
        if !canonical.contains(&normalized) && !feature.experimental {
            anyhow::bail!(
                "Unknown feature '{}' must be marked experimental for now",
                feature.id
            );
        }
        let has_binding = feature
            .binding
            .as_ref()
            .map(|binding| !binding.kind.trim().is_empty())
            .unwrap_or(false);
        if let Some(binding) = feature.binding.as_ref() {
            let normalized_binding = binding.kind.trim().to_ascii_lowercase();
            if !normalized_binding.is_empty()
                && !supported_bindings.contains(normalized_binding.as_str())
            {
                anyhow::bail!(
                    "Feature '{}' declares unsupported binding kind '{}'",
                    feature.id,
                    binding.kind
                );
            }
        }
        if !manifest.draft && !has_binding {
            anyhow::bail!(
                "Feature '{}' needs a binding unless the whole pack is draft-only",
                feature.id
            );
        }
    }
    if matches!(source_kind, ExtensionPackSourceKind::BundledRegistry)
        && manifest
            .signature
            .as_deref()
            .unwrap_or("")
            .trim()
            .is_empty()
    {
        anyhow::bail!("Bundled packs must carry a signature marker");
    }
    if !matches!(source_kind, ExtensionPackSourceKind::BundledRegistry)
        && manifest
            .signature
            .as_deref()
            .is_some_and(|value| value.trim().eq_ignore_ascii_case("bundled"))
    {
        anyhow::bail!(
            "The reserved 'bundled' signature marker may only be used by bundled catalog packs"
        );
    }
    if manifest.signature.as_deref().is_some_and(|value| {
        let trimmed = value.trim();
        !trimmed.is_empty() && !trimmed.eq_ignore_ascii_case("bundled")
    }) {
        verify_manifest_signature(manifest)?;
    } else if manifest
        .publisher_did
        .as_deref()
        .is_some_and(|value| !value.trim().is_empty())
    {
        anyhow::bail!(
            "Pack '{}' includes publisher_did but does not include a verifiable signature",
            manifest.id
        );
    }
    Ok(())
}

async fn load_json<T>(storage: &Storage, key: &str) -> Result<T>
where
    T: DeserializeOwned + Default,
{
    let Some(bytes) = storage.get_encrypted(key).await? else {
        return Ok(T::default());
    };
    serde_json::from_slice::<T>(&bytes)
        .with_context(|| format!("failed to decode extension-pack payload for {}", key))
}

async fn save_json<T>(storage: &Storage, key: &str, value: &T) -> Result<()>
where
    T: Serialize + ?Sized,
{
    let bytes = serde_json::to_vec(value)
        .with_context(|| format!("failed to encode extension-pack payload for {}", key))?;
    storage.set_encrypted(key, &bytes).await
}

impl ExtensionPackRegistry {
    pub fn new(storage: Storage, config_dir: PathBuf, data_dir: PathBuf) -> Self {
        let http_client = reqwest::Client::builder()
            .timeout(std::time::Duration::from_secs(20))
            .redirect(reqwest::redirect::Policy::none())
            .build()
            .unwrap_or_else(|_| reqwest::Client::new());
        Self {
            storage,
            config_dir,
            data_dir,
            http_client,
            installed: HashMap::new(),
            connections: HashMap::new(),
            events: Vec::new(),
        }
    }

    pub fn canonical_features() -> Vec<CanonicalFeatureDef> {
        vec![
            (
                "mail.list",
                "List mail",
                "List messages from a mail provider",
            ),
            (
                "mail.get",
                "Read mail",
                "Read one message from a mail provider",
            ),
            ("mail.send", "Send mail", "Send or reply to mail"),
            (
                "calendar.list_events",
                "List calendar events",
                "List events from a calendar provider",
            ),
            (
                "calendar.create_event",
                "Create calendar event",
                "Create an event on a connected calendar provider",
            ),
            ("files.list", "List files", "List files from a provider"),
            (
                "files.search",
                "Search files",
                "Search files from a provider",
            ),
            ("files.read", "Read file", "Read file or document content"),
            (
                "chat.list_spaces",
                "List chat spaces",
                "List spaces/rooms/channels",
            ),
            ("chat.send", "Send chat", "Send a chat message"),
            (
                "contacts.search",
                "Search contacts",
                "Search address book data",
            ),
            (
                "message.receive",
                "Receive message",
                "Receive inbound channel messages",
            ),
            (
                "message.send",
                "Send message",
                "Send outbound channel messages",
            ),
            (
                "message.list_threads",
                "List threads",
                "List message threads or conversations",
            ),
        ]
        .into_iter()
        .map(|(id, title, description)| CanonicalFeatureDef {
            id: id.to_string(),
            title: title.to_string(),
            description: description.to_string(),
            kinds: vec![
                FEATURE_KIND_CAPABILITY.to_string(),
                FEATURE_KIND_RESOURCE.to_string(),
                FEATURE_KIND_EVENT.to_string(),
            ],
        })
        .collect()
    }

    pub async fn sync_from_storage(&mut self) -> Result<()> {
        let installed =
            load_json::<Vec<InstalledExtensionPack>>(&self.storage, INSTALLED_PACKS_KEY).await?;
        let connections =
            load_json::<Vec<ExtensionPackConnection>>(&self.storage, CONNECTIONS_KEY).await?;
        let events = load_json::<Vec<ExtensionPackEventRecord>>(&self.storage, EVENTS_KEY).await?;
        self.installed = installed
            .into_iter()
            .map(|mut item| {
                if item.verification_status.trim().is_empty() {
                    let (status, detail) =
                        if matches!(item.source_kind, ExtensionPackSourceKind::BundledRegistry) {
                            (
                                "bundled".to_string(),
                                Some("Bundled first-party pack.".to_string()),
                            )
                        } else if let Ok(Some(detail)) = verify_manifest_signature(&item.manifest) {
                            ("verified".to_string(), Some(detail))
                        } else {
                            (
                                "unverified".to_string(),
                                Some(
                                    "Installed before pack verification metadata was recorded."
                                        .to_string(),
                                ),
                            )
                        };
                    item.verification_status = status;
                    item.verification_detail = detail;
                }
                if manifest_requires_runtime(&item.manifest)
                    && matches!(
                        item.runtime_state.status,
                        ExtensionPackRuntimeStatus::NotRequired
                    )
                {
                    item.runtime_state = default_runtime_state_for_manifest(&item.manifest);
                }
                (item.manifest.id.clone(), item)
            })
            .collect();
        self.connections = connections
            .into_iter()
            .map(|item| (item.id.clone(), item))
            .collect();
        self.events = events;
        let mut changed = false;
        let pack_ids = self.installed.keys().cloned().collect::<Vec<_>>();
        for pack_id in pack_ids {
            changed |= self.ensure_default_connection(&pack_id);
        }
        if changed {
            self.persist_connections().await?;
        }
        Ok(())
    }

    pub async fn list_installed(&self, kind: Option<&str>) -> Result<Vec<ExtensionPackView>> {
        let mut items = Vec::new();
        for pack in self.installed.values() {
            if !pack_matches_kind(&pack.manifest, kind) {
                continue;
            }
            items.push(self.pack_view(pack).await?);
        }
        items.sort_by(|left, right| left.manifest.name.cmp(&right.manifest.name));
        Ok(items)
    }

    pub async fn list_catalog(
        &self,
        query: Option<&str>,
        kind: Option<&str>,
    ) -> Result<Vec<ExtensionPackView>> {
        let mut items = Vec::new();
        for manifest in bundled_catalog() {
            if self.installed.contains_key(&manifest.id)
                || !pack_matches_kind(&manifest, kind)
                || !pack_matches_query(&manifest, query)
            {
                continue;
            }
            items.push(self.catalog_view(&manifest).await?);
        }
        items.sort_by(|left, right| left.manifest.name.cmp(&right.manifest.name));
        Ok(items)
    }

    pub async fn search_packs(
        &self,
        query: Option<&str>,
        kind: Option<&str>,
    ) -> Result<ExtensionPackSearchResponse> {
        let installed = self
            .list_installed(kind)
            .await?
            .into_iter()
            .filter(|item| pack_matches_query(&item.manifest, query))
            .collect::<Vec<_>>();
        let catalog = self.list_catalog(query, kind).await?;
        let not_found = installed.is_empty() && catalog.is_empty();
        let next_steps = if not_found {
            vec![
                "Scaffold a local draft pack from chat or the settings panel.".to_string(),
                "Import from OpenAPI, curl, a manifest file, or a bundle upload if you already have one.".to_string(),
                "Start with an unverified read-only pack before enabling write actions.".to_string(),
            ]
        } else {
            Vec::new()
        };
        Ok(ExtensionPackSearchResponse {
            query: query.unwrap_or_default().to_string(),
            installed,
            catalog,
            not_found,
            next_steps,
        })
    }

    pub async fn get_pack(&self, pack_id: &str) -> Result<Option<ExtensionPackView>> {
        if let Some(pack) = self.installed.get(pack_id) {
            return Ok(Some(self.pack_view(pack).await?));
        }
        let manifest = bundled_catalog()
            .into_iter()
            .find(|item| item.id.eq_ignore_ascii_case(pack_id));
        match manifest {
            Some(manifest) => Ok(Some(self.catalog_view(&manifest).await?)),
            None => Ok(None),
        }
    }

    pub async fn sync_to_runtime(&self, runtime: &crate::runtime::ActionRuntime) -> Result<()> {
        runtime.unregister_extension_pack_actions().await;
        for spec in self.runtime_action_specs().await? {
            runtime
                .register_extension_pack_action(spec.info, spec.binding)
                .await;
        }
        Ok(())
    }

    fn ordered_connections_for_pack(
        &self,
        manifest: &ExtensionPackManifest,
    ) -> Vec<ExtensionPackConnection> {
        let mut items = self
            .connections
            .values()
            .filter(|item| item.pack_id == manifest.id)
            .cloned()
            .collect::<Vec<_>>();
        items.sort_by(|left, right| {
            right
                .enabled
                .cmp(&left.enabled)
                .then_with(|| left.name.cmp(&right.name))
                .then_with(|| left.id.cmp(&right.id))
        });
        items
    }

    async fn preferred_connection_for_pack(
        &self,
        manifest: &ExtensionPackManifest,
    ) -> Result<Option<ExtensionPackConnection>> {
        let ordered = self.ordered_connections_for_pack(manifest);
        let fallback = ordered
            .iter()
            .find(|item| item.enabled)
            .cloned()
            .or_else(|| ordered.first().cloned());
        for connection in ordered.into_iter().filter(|item| item.enabled) {
            if matches!(
                self.connection_state(manifest, &connection).await?,
                ExtensionConnectionState::Ready
            ) {
                return Ok(Some(connection));
            }
        }
        Ok(fallback)
    }

    async fn runtime_action_specs(&self) -> Result<Vec<RuntimePackActionSpec>> {
        let mut ready_features_by_id: BTreeMap<String, usize> = BTreeMap::new();
        let mut ready_packs = Vec::new();
        for pack in self.installed.values() {
            let view = self.pack_view(pack).await?;
            if !view.enabled || !matches!(view.status.as_str(), "ready" | "connected") {
                continue;
            }
            ready_packs.push((pack.clone(), view));
        }
        for (pack, _) in &ready_packs {
            for feature in &pack.manifest.features {
                if feature.kind.eq_ignore_ascii_case(FEATURE_KIND_EVENT) {
                    continue;
                }
                if !feature.binding.as_ref().is_some_and(|binding| {
                    !binding.kind.trim().is_empty()
                        && !binding.kind.eq_ignore_ascii_case(BINDING_KIND_UNSUPPORTED)
                }) {
                    continue;
                }
                *ready_features_by_id.entry(feature.id.clone()).or_default() += 1;
            }
        }

        let mut specs = Vec::new();
        for (pack, _) in ready_packs {
            let default_connection_id = self
                .preferred_connection_for_pack(&pack.manifest)
                .await?
                .map(|item| item.id);
            for feature in &pack.manifest.features {
                let Some(binding) = feature.binding.as_ref() else {
                    continue;
                };
                if feature.kind.eq_ignore_ascii_case(FEATURE_KIND_EVENT)
                    || binding.kind.eq_ignore_ascii_case(BINDING_KIND_UNSUPPORTED)
                {
                    continue;
                }
                let mut action_names = vec![format!("{}.{}", pack.manifest.id, feature.id)];
                if ready_features_by_id
                    .get(&feature.id)
                    .copied()
                    .unwrap_or_default()
                    == 1
                {
                    action_names.push(feature.id.clone());
                }
                for action_name in action_names {
                    let outbound_write = !feature.read_only;
                    let capabilities = if binding.kind.eq_ignore_ascii_case(BINDING_KIND_LOCAL_CLI)
                    {
                        if outbound_write {
                            vec!["local_cli".to_string(), "external_write".to_string()]
                        } else {
                            vec!["local_cli".to_string(), "read".to_string()]
                        }
                    } else if outbound_write {
                        vec!["network".to_string(), "external_write".to_string()]
                    } else {
                        vec!["network".to_string(), "read".to_string()]
                    };
                    specs.push(RuntimePackActionSpec {
                        info: crate::actions::ActionDef {
                            name: action_name.clone(),
                            description: format!(
                                "{} via {}. {}",
                                feature.title.trim(),
                                pack.manifest.name.trim(),
                                feature.description.trim()
                            )
                            .trim()
                            .to_string(),
                            version: pack.manifest.version.clone(),
                            input_schema: if feature.input_schema.is_null() {
                                serde_json::json!({
                                    "type": "object",
                                    "additionalProperties": true
                                })
                            } else {
                                feature.input_schema.clone()
                            },
                            capabilities,
                            sandbox_mode: Some(crate::runtime::SandboxMode::Native),
                            source: crate::actions::ActionSource::System,
                            file_path: None,
                            authorization: crate::actions::ActionAuthorization {
                                outbound: crate::actions::ActionEgressPolicy {
                                    read_only: feature.read_only,
                                    outbound_write,
                                    public_publish: false,
                                },
                                access: crate::actions::ActionAccessMetadata {
                                    extension_pack_ids: vec![pack.manifest.id.clone()],
                                    ..crate::actions::ActionAccessMetadata::default()
                                },
                                ..crate::actions::ActionAuthorization::default()
                            },
                        },
                        binding: crate::runtime::ExtensionPackActionBinding {
                            pack_id: pack.manifest.id.clone(),
                            feature_id: feature.id.clone(),
                            action_name,
                            connection_id: default_connection_id.clone(),
                            read_only: feature.read_only,
                            binding_kind: binding.kind.clone(),
                        },
                    });
                }
            }
        }
        Ok(specs)
    }

    pub async fn list_connections(
        &self,
        pack_id: &str,
    ) -> Result<Vec<ExtensionPackConnectionView>> {
        let Some(pack) = self.installed.get(pack_id) else {
            return Ok(Vec::new());
        };
        let mut items = Vec::new();
        for connection in self.connections.values() {
            if connection.pack_id != pack.manifest.id {
                continue;
            }
            items.push(self.connection_view(&pack.manifest, connection).await?);
        }
        items.sort_by(|left, right| {
            connection_state_priority(left.state)
                .cmp(&connection_state_priority(right.state))
                .then_with(|| left.connection.name.cmp(&right.connection.name))
                .then_with(|| left.connection.id.cmp(&right.connection.id))
        });
        Ok(items)
    }

    pub fn supports_connect_url(&self, pack_id: &str) -> bool {
        self.installed
            .get(pack_id)
            .map(|pack| pack_supports_connect_url_manifest(&pack.manifest))
            .unwrap_or_else(|| {
                bundled_catalog()
                    .into_iter()
                    .find(|item| item.id.eq_ignore_ascii_case(pack_id))
                    .map(|manifest| pack_supports_connect_url_manifest(&manifest))
                    .unwrap_or(false)
            })
    }

    pub fn connect_redirect_uri(
        &self,
        pack_id: &str,
        fallback_redirect_uri: &str,
    ) -> Result<String> {
        if pack_id.eq_ignore_ascii_case("google_workspace") {
            return Ok(fallback_redirect_uri.to_string());
        }
        let pack = self
            .installed
            .get(pack_id)
            .ok_or_else(|| anyhow!("Pack '{}' is not installed", pack_id))?;
        let oauth = pack
            .manifest
            .auth
            .oauth2
            .as_ref()
            .ok_or_else(|| anyhow!("This pack does not expose a browser connect URL"))?;
        Ok(oauth
            .redirect_uri
            .as_deref()
            .map(str::trim)
            .filter(|value| !value.is_empty())
            .unwrap_or(fallback_redirect_uri)
            .to_string())
    }

    pub fn supports_webhook(&self, pack_id: &str) -> bool {
        self.installed
            .get(pack_id)
            .map(|pack| pack_supports_webhook_manifest(&pack.manifest))
            .unwrap_or_else(|| {
                bundled_catalog()
                    .into_iter()
                    .find(|item| item.id.eq_ignore_ascii_case(pack_id))
                    .map(|manifest| pack_supports_webhook_manifest(&manifest))
                    .unwrap_or(false)
            })
    }

    pub async fn ensure_connect_auth_profile(&mut self, pack_id: &str) -> Result<Option<String>> {
        if pack_id.eq_ignore_ascii_case("google_workspace") {
            return Ok(None);
        }
        let pack = self
            .installed
            .get(pack_id)
            .cloned()
            .ok_or_else(|| anyhow!("Pack '{}' is not installed", pack_id))?;
        pack.manifest
            .auth
            .oauth2
            .as_ref()
            .ok_or_else(|| anyhow!("This pack does not expose a browser connect URL"))?;
        if self.ensure_default_connection(pack_id) {
            self.persist_connections().await?;
        }
        let connection = self
            .preferred_connection_for_pack(&pack.manifest)
            .await?
            .ok_or_else(|| {
                anyhow!(
                    "No connection is configured for pack '{}'",
                    pack.manifest.id
                )
            })?;
        let profile_id = self
            .upsert_connection_auth_profile(
                &pack.manifest,
                &connection.id,
                &connection.name,
                None,
                connection.auth_profile_id.as_deref(),
            )
            .await?
            .ok_or_else(|| anyhow!("OAuth profile could not be prepared for this pack"))?;
        if connection.auth_profile_id.as_deref() != Some(profile_id.as_str()) {
            if let Some(stored) = self.connections.get_mut(&connection.id) {
                stored.auth_profile_id = Some(profile_id.clone());
                stored.updated_at = now_rfc3339();
            }
            self.persist_connections().await?;
        }
        Ok(Some(profile_id))
    }

    pub fn webhook_path(&self, pack_id: &str) -> Option<String> {
        self.supports_webhook(pack_id)
            .then(|| format!("/extension-packs/{}/webhook", urlencoding::encode(pack_id)))
    }

    pub async fn build_connect_url(
        &mut self,
        pack_id: &str,
        redirect_uri: &str,
        state_token: &str,
        code_challenge: &str,
    ) -> Result<String> {
        if !pack_id.eq_ignore_ascii_case("google_workspace") {
            let pack = self
                .installed
                .get(pack_id)
                .cloned()
                .ok_or_else(|| anyhow!("Pack '{}' is not installed", pack_id))?;
            pack.manifest
                .auth
                .oauth2
                .as_ref()
                .ok_or_else(|| anyhow!("This pack does not expose a browser connect URL"))?;
            let profile_id = self
                .ensure_connect_auth_profile(pack_id)
                .await?
                .ok_or_else(|| anyhow!("OAuth profile could not be prepared for this pack"))?;
            return crate::core::auth_profiles::AuthProfileControlPlane::oauth_authorization_url(
                &self.storage,
                &profile_id,
                state_token,
                Some(code_challenge),
                Some(redirect_uri),
            )
            .await;
        }
        crate::actions::google_workspace::build_auth_url(
            &self.config_dir,
            state_token,
            code_challenge,
            redirect_uri,
        )
    }

    pub async fn verify_runtime(&mut self, pack_id: &str) -> Result<ExtensionPackRuntimeResult> {
        let pack = self
            .installed
            .get(pack_id)
            .cloned()
            .ok_or_else(|| anyhow!("Pack '{}' is not installed", pack_id))?;
        let Some(installer) = runtime_installer(&pack.manifest).cloned() else {
            return Ok(ExtensionPackRuntimeResult {
                pack_id: pack.manifest.id,
                status: ExtensionPackRuntimeStatus::NotRequired,
                executable_path: None,
                detail: Some("This pack does not require a local runtime.".to_string()),
                restart_required: false,
                restart_instructions: None,
            });
        };
        let commands = if installer.verify.is_empty() {
            vec![ExtensionPackCommandSpec {
                program: installer.executable.clone(),
                args: vec!["--version".to_string()],
                ..ExtensionPackCommandSpec::default()
            }]
        } else {
            installer.verify.clone()
        };
        let verify_result = self
            .run_runtime_commands(pack.manifest.id.as_str(), &commands)
            .await;
        let detail = match &verify_result {
            Ok(output) => Some(output.clone()),
            Err(error) => Some(error.to_string()),
        };
        if let Some(stored) = self.installed.get_mut(pack.manifest.id.as_str()) {
            stored.runtime_state.status = if verify_result.is_ok() {
                ExtensionPackRuntimeStatus::Ready
            } else {
                ExtensionPackRuntimeStatus::Error
            };
            stored.runtime_state.executable_path = Some(installer.executable.clone());
            stored.runtime_state.last_checked_at = Some(now_rfc3339());
            stored.runtime_state.last_error = verify_result.as_ref().err().map(|e| e.to_string());
            stored.updated_at = now_rfc3339();
        }
        self.persist_installed().await?;
        Ok(ExtensionPackRuntimeResult {
            pack_id: pack.manifest.id,
            status: if verify_result.is_ok() {
                ExtensionPackRuntimeStatus::Ready
            } else {
                ExtensionPackRuntimeStatus::Error
            },
            executable_path: Some(installer.executable),
            detail,
            restart_required: installer.restart_required,
            restart_instructions: installer.restart_instructions.clone(),
        })
    }

    pub async fn install_runtime(&mut self, pack_id: &str) -> Result<ExtensionPackRuntimeResult> {
        let pack = self
            .installed
            .get(pack_id)
            .cloned()
            .ok_or_else(|| anyhow!("Pack '{}' is not installed", pack_id))?;
        let Some(installer) = runtime_installer(&pack.manifest).cloned() else {
            return Ok(ExtensionPackRuntimeResult {
                pack_id: pack.manifest.id,
                status: ExtensionPackRuntimeStatus::NotRequired,
                executable_path: None,
                detail: Some("This pack does not require a local runtime.".to_string()),
                restart_required: false,
                restart_instructions: None,
            });
        };
        if !installer.install.is_empty() {
            self.run_runtime_commands(pack.manifest.id.as_str(), &installer.install)
                .await?;
        }
        let mut result = self.verify_runtime(pack_id).await?;
        result.detail = runtime_operation_detail(&installer, result.detail);
        result.restart_required = installer.restart_required;
        result.restart_instructions = installer.restart_instructions.clone();
        Ok(result)
    }

    pub async fn update_runtime(&mut self, pack_id: &str) -> Result<ExtensionPackRuntimeResult> {
        let pack = self
            .installed
            .get(pack_id)
            .cloned()
            .ok_or_else(|| anyhow!("Pack '{}' is not installed", pack_id))?;
        let Some(installer) = runtime_installer(&pack.manifest).cloned() else {
            return Ok(ExtensionPackRuntimeResult {
                pack_id: pack.manifest.id,
                status: ExtensionPackRuntimeStatus::NotRequired,
                executable_path: None,
                detail: Some("This pack does not require a local runtime.".to_string()),
                restart_required: false,
                restart_instructions: None,
            });
        };
        if installer.update.is_empty() {
            return self.verify_runtime(pack_id).await;
        }
        self.run_runtime_commands(pack.manifest.id.as_str(), &installer.update)
            .await?;
        let mut result = self.verify_runtime(pack_id).await?;
        result.detail = runtime_operation_detail(&installer, result.detail);
        result.restart_required = installer.restart_required;
        result.restart_instructions = installer.restart_instructions.clone();
        Ok(result)
    }

    pub async fn uninstall_runtime(&mut self, pack_id: &str) -> Result<ExtensionPackRuntimeResult> {
        let pack = self
            .installed
            .get(pack_id)
            .cloned()
            .ok_or_else(|| anyhow!("Pack '{}' is not installed", pack_id))?;
        let Some(installer) = runtime_installer(&pack.manifest).cloned() else {
            return Ok(ExtensionPackRuntimeResult {
                pack_id: pack.manifest.id,
                status: ExtensionPackRuntimeStatus::NotRequired,
                executable_path: None,
                detail: Some("This pack does not require a local runtime.".to_string()),
                restart_required: false,
                restart_instructions: None,
            });
        };
        if !installer.uninstall.is_empty() {
            self.run_runtime_commands(pack.manifest.id.as_str(), &installer.uninstall)
                .await?;
        }
        if let Some(stored) = self.installed.get_mut(pack.manifest.id.as_str()) {
            stored.runtime_state.status = ExtensionPackRuntimeStatus::Missing;
            stored.runtime_state.executable_path = None;
            stored.runtime_state.last_checked_at = Some(now_rfc3339());
            stored.runtime_state.last_error = None;
            stored.updated_at = now_rfc3339();
        }
        self.persist_installed().await?;
        let executable_path = installer.executable.clone();
        let detail =
            runtime_operation_detail(&installer, Some("Runtime uninstall completed.".to_string()));
        Ok(ExtensionPackRuntimeResult {
            pack_id: pack.manifest.id,
            status: ExtensionPackRuntimeStatus::Missing,
            executable_path: Some(executable_path),
            detail,
            restart_required: installer.restart_required,
            restart_instructions: installer.restart_instructions.clone(),
        })
    }

    pub async fn install(
        &mut self,
        request: ExtensionPackInstallRequest,
    ) -> Result<ExtensionPackView> {
        let pack_id = request.pack_id.clone();
        let source_url = request.source_url.clone();
        let source_path = request.source_path.clone();
        let inline_manifest = request.manifest.clone();
        let manifest_text = request.manifest_text.clone();
        let trust_unverified = request.trust_unverified;
        let manifest = if let Some(pack_id) = pack_id
            .as_deref()
            .map(str::trim)
            .filter(|value| !value.is_empty())
        {
            bundled_catalog()
                .into_iter()
                .find(|item| item.id.eq_ignore_ascii_case(pack_id))
                .ok_or_else(|| {
                    anyhow::anyhow!("Pack '{}' was not found in the bundled catalog", pack_id)
                })?
        } else if let Some(manifest) = inline_manifest {
            manifest
        } else if let Some(text) = manifest_text
            .as_deref()
            .map(str::trim)
            .filter(|value| !value.is_empty())
        {
            self.parse_manifest_text(text, "inline manifest text")?
        } else if let Some(source_url) = source_url
            .as_deref()
            .map(str::trim)
            .filter(|value| !value.is_empty())
        {
            self.fetch_manifest_from_url(source_url).await?
        } else if let Some(source_path) = source_path
            .as_deref()
            .map(str::trim)
            .filter(|value| !value.is_empty())
        {
            self.fetch_manifest_from_path(source_path)?
        } else {
            anyhow::bail!(
                "pack_id, source_url, source_path, manifest_text, or manifest is required"
            );
        };

        let source_kind = if pack_id.is_some() {
            ExtensionPackSourceKind::BundledRegistry
        } else if source_url.is_some() {
            ExtensionPackSourceKind::DirectUrl
        } else if source_path.is_some() {
            ExtensionPackSourceKind::LocalPath
        } else {
            ExtensionPackSourceKind::LocalManifest
        };
        validate_manifest(&manifest, source_kind)?;
        let (trust_level, verification_status, verification_detail) = if matches!(
            source_kind,
            ExtensionPackSourceKind::BundledRegistry
        ) {
            (
                ExtensionPackTrustLevel::Trusted,
                "bundled".to_string(),
                Some("Bundled first-party pack.".to_string()),
            )
        } else if let Ok(Some(detail)) = verify_manifest_signature(&manifest) {
            (
                ExtensionPackTrustLevel::Trusted,
                "verified".to_string(),
                Some(detail),
            )
        } else if trust_unverified {
            (
                ExtensionPackTrustLevel::Unverified,
                "unverified".to_string(),
                Some("Installed with explicit user trust because publisher signature verification did not pass.".to_string()),
            )
        } else {
            anyhow::bail!("Unverified packs require trust_unverified=true before installation");
        };
        let now = now_rfc3339();
        let existing_runtime_state = self
            .installed
            .get(&manifest.id)
            .map(|item| item.runtime_state.clone())
            .unwrap_or_else(|| default_runtime_state_for_manifest(&manifest));
        let installed = InstalledExtensionPack {
            manifest: manifest.clone(),
            trust_level,
            verification_status,
            verification_detail,
            source_kind,
            source_url: source_url.clone().or(source_path.clone()),
            enabled: true,
            runtime_state: existing_runtime_state,
            installed_at: self
                .installed
                .get(&manifest.id)
                .map(|item| item.installed_at.clone())
                .unwrap_or_else(|| now.clone()),
            updated_at: now,
        };
        self.installed
            .insert(manifest.id.clone(), installed.clone());
        self.persist_installed().await?;
        if self.ensure_default_connection(&manifest.id) {
            self.persist_connections().await?;
        }
        self.pack_view(&installed).await
    }

    pub async fn scaffold(
        &mut self,
        request: ExtensionPackScaffoldRequest,
    ) -> Result<ExtensionPackView> {
        let manifest = if request
            .openapi_url
            .as_deref()
            .is_some_and(|value| !value.trim().is_empty())
            || request
                .openapi_text
                .as_deref()
                .is_some_and(|value| !value.trim().is_empty())
            || request
                .curl_text
                .as_deref()
                .is_some_and(|value| !value.trim().is_empty())
        {
            self.scaffold_manifest_from_api_source(&request).await?
        } else {
            self.scaffold_manifest_from_request(&request)?
        };
        let view = self
            .install(ExtensionPackInstallRequest {
                pack_id: None,
                source_url: None,
                source_path: None,
                manifest: Some(manifest),
                manifest_text: None,
                trust_unverified: true,
            })
            .await?;
        if let Some(installed) = self.installed.get_mut(&view.manifest.id) {
            installed.source_kind = ExtensionPackSourceKind::Scaffolded;
            installed.updated_at = now_rfc3339();
        }
        self.persist_installed().await?;
        self.get_pack(&view.manifest.id)
            .await?
            .ok_or_else(|| anyhow::anyhow!("scaffolded pack disappeared"))
    }

    fn scaffold_manifest_from_request(
        &self,
        request: &ExtensionPackScaffoldRequest,
    ) -> Result<ExtensionPackManifest> {
        let binding_kind = request
            .binding_kind
            .as_deref()
            .map(str::trim)
            .filter(|value| !value.is_empty())
            .unwrap_or(BINDING_KIND_UNSUPPORTED)
            .to_string();
        Ok(ExtensionPackManifest {
            sdk_version: default_sdk_version(),
            id: sanitize_pack_id(&request.name),
            name: request.name.trim().to_string(),
            version: "0.1.0".to_string(),
            kind: request.kind.trim().to_ascii_lowercase(),
            publisher: request
                .publisher
                .as_deref()
                .map(str::trim)
                .filter(|value| !value.is_empty())
                .unwrap_or("local-user")
                .to_string(),
            publisher_did: None,
            description: request.description.trim().to_string(),
            docs_url: request.docs_url.clone(),
            signature: None,
            draft: true,
            tags: {
                let mut tags = vec!["scaffolded".to_string()];
                tags.extend(
                    request
                        .tags
                        .iter()
                        .map(|value| value.trim().to_ascii_lowercase())
                        .filter(|value| !value.is_empty()),
                );
                tags
            },
            auth: ExtensionPackAuthSpec {
                mode: request.auth_mode,
                required_secrets: match request.auth_mode {
                    ExtensionPackAuthMode::ApiKey => vec!["api_key".to_string()],
                    ExtensionPackAuthMode::Basic => {
                        vec!["username".to_string(), "password".to_string()]
                    }
                    _ => Vec::new(),
                },
                required_scopes: Vec::new(),
                oauth2: None,
                exports: ExtensionPackAuthExportSpec::default(),
                metadata: Value::Null,
            },
            features: request
                .desired_features
                .iter()
                .map(|feature_id| PackFeatureManifest {
                    id: feature_id.trim().to_ascii_lowercase(),
                    kind: FEATURE_KIND_CAPABILITY.to_string(),
                    title: feature_id.trim().replace('.', " "),
                    description: format!(
                        "Draft feature generated from chat for {}.",
                        request.name.trim()
                    ),
                    read_only: request.read_only,
                    experimental: !canonical_feature_ids()
                        .contains(&feature_id.trim().to_ascii_lowercase()),
                    input_schema: Value::Null,
                    output_schema: Value::Null,
                    binding: Some(ExtensionPackBinding {
                        kind: binding_kind.clone(),
                        config: if binding_kind == BINDING_KIND_UNSUPPORTED {
                            serde_json::json!({
                                "reason": "Scaffolded draft. Review and replace the binding before production use."
                            })
                        } else {
                            Value::Null
                        },
                    }),
                })
                .collect(),
            discovery: ExtensionPackDiscoverySpec {
                aliases: vec![request.name.trim().to_ascii_lowercase()],
                ..ExtensionPackDiscoverySpec::default()
            },
            runtime: ExtensionPackRuntimeSpec::default(),
            channel: None,
            metadata: serde_json::json!({
                "scaffolded": true
            }),
        })
    }

    async fn scaffold_manifest_from_api_source(
        &self,
        request: &ExtensionPackScaffoldRequest,
    ) -> Result<ExtensionPackManifest> {
        let preview =
            crate::custom_apis::preview_custom_api(crate::custom_apis::CustomApiPreviewRequest {
                name: if request.name.trim().is_empty() {
                    None
                } else {
                    Some(request.name.trim().to_string())
                },
                base_url: None,
                openapi_url: request.openapi_url.clone(),
                openapi_text: request.openapi_text.clone(),
                curl_text: request.curl_text.clone(),
            })
            .await?;
        let name = if request.name.trim().is_empty() {
            preview.suggested_name.clone()
        } else {
            request.name.trim().to_string()
        };
        let pack_id = sanitize_pack_id(&name);
        let pack_kind = request.kind.trim().to_ascii_lowercase();
        let imported_auth = imported_auth_contract(request.auth_mode, &preview);
        let feature_ids = request
            .desired_features
            .iter()
            .map(|value| value.trim().to_ascii_lowercase())
            .filter(|value| !value.is_empty())
            .collect::<Vec<_>>();
        let canonical = canonical_feature_ids();
        let features = preview
            .operations
            .iter()
            .enumerate()
            .map(|(index, operation)| {
                let feature_id = feature_ids.get(index).cloned().unwrap_or_else(|| {
                    infer_feature_id_for_operation(&pack_kind, &pack_id, operation)
                });
                PackFeatureManifest {
                    id: feature_id.clone(),
                    kind: FEATURE_KIND_CAPABILITY.to_string(),
                    title: operation.name.clone(),
                    description: if operation.description.trim().is_empty() {
                        format!("Imported from {} {}.", operation.method, operation.path)
                    } else {
                        operation.description.clone()
                    },
                    read_only: operation.read_only,
                    experimental: !canonical.contains(&feature_id),
                    input_schema: operation_input_schema(operation),
                    output_schema: Value::Null,
                    binding: Some(ExtensionPackBinding {
                        kind: BINDING_KIND_HTTP.to_string(),
                        config: http_binding_from_operation(
                            &preview.base_url,
                            operation,
                            imported_auth.binding.clone(),
                        ),
                    }),
                }
            })
            .collect::<Vec<_>>();
        let health_probe = preview
            .operations
            .iter()
            .enumerate()
            .find(|(_, operation)| imported_operation_supports_health_probe(operation))
            .and_then(|(index, _)| {
                features.get(index).map(|feature| {
                    serde_json::json!({
                        "feature_id": feature.id,
                        "arguments": {}
                    })
                })
            });
        let mut metadata = serde_json::json!({
            "scaffolded": true,
            "imported_from_api": true,
            "base_url": preview.base_url,
        });
        if let Some(health_probe) = health_probe {
            if let Some(map) = metadata.as_object_mut() {
                map.insert("health_probe".to_string(), health_probe);
            }
        }
        let discovery_alias = name.to_ascii_lowercase();
        let discovery_pack_id = pack_id.clone();
        Ok(ExtensionPackManifest {
            sdk_version: default_sdk_version(),
            id: pack_id,
            name,
            version: "0.1.0".to_string(),
            kind: pack_kind,
            publisher: request
                .publisher
                .as_deref()
                .map(str::trim)
                .filter(|value| !value.is_empty())
                .unwrap_or("local-user")
                .to_string(),
            publisher_did: None,
            description: if request.description.trim().is_empty() {
                format!(
                    "Draft pack imported from {} with {} discovered operations.",
                    preview.source_kind,
                    preview.operations.len()
                )
            } else {
                request.description.trim().to_string()
            },
            docs_url: request.docs_url.clone().or(request.openapi_url.clone()),
            signature: None,
            draft: true,
            tags: {
                let mut tags = vec!["scaffolded".to_string(), "imported".to_string()];
                tags.extend(
                    request
                        .tags
                        .iter()
                        .map(|value| value.trim().to_ascii_lowercase())
                        .filter(|value| !value.is_empty()),
                );
                tags
            },
            auth: ExtensionPackAuthSpec {
                mode: imported_auth.mode,
                required_secrets: imported_auth.required_secrets,
                required_scopes: Vec::new(),
                oauth2: None,
                exports: ExtensionPackAuthExportSpec::default(),
                metadata: Value::Object(imported_auth.metadata),
            },
            features,
            discovery: ExtensionPackDiscoverySpec {
                aliases: vec![discovery_alias],
                package_names: vec![discovery_pack_id],
                domains: request
                    .docs_url
                    .as_deref()
                    .and_then(|url| reqwest::Url::parse(url).ok())
                    .and_then(|url| url.host_str().map(str::to_string))
                    .into_iter()
                    .collect(),
                ..ExtensionPackDiscoverySpec::default()
            },
            runtime: ExtensionPackRuntimeSpec::default(),
            channel: None,
            metadata,
        })
    }

    pub async fn upsert_connection(
        &mut self,
        pack_id: &str,
        request: ExtensionPackConnectionUpsertRequest,
    ) -> Result<ExtensionPackConnectionView> {
        let Some(pack) = self.installed.get(pack_id).cloned() else {
            anyhow::bail!("Pack '{}' is not installed", pack_id);
        };
        let now = now_rfc3339();
        let connection_id = request
            .connection_id
            .as_deref()
            .map(sanitize_pack_id)
            .filter(|value| !value.is_empty())
            .or_else(|| {
                self.connections
                    .values()
                    .find(|item| item.pack_id == pack.manifest.id)
                    .map(|item| item.id.clone())
            })
            .unwrap_or_else(|| {
                format!(
                    "{}-{}",
                    sanitize_pack_id(pack_id),
                    uuid::Uuid::new_v4().simple()
                )
            });
        let existing = self.connections.get(&connection_id).cloned();
        if existing
            .as_ref()
            .is_some_and(|item| !item.pack_id.eq_ignore_ascii_case(&pack.manifest.id))
        {
            anyhow::bail!(
                "Connection '{}' belongs to pack '{}' rather than '{}'",
                connection_id,
                existing
                    .as_ref()
                    .map(|item| item.pack_id.as_str())
                    .unwrap_or_default(),
                pack.manifest.id
            );
        }
        let next_secret = if request.clear_secret {
            None
        } else if let Some(secret) = request.secret.clone() {
            Some(secret)
        } else {
            existing.as_ref().and_then(|item| {
                self.load_connection_secret(pack_id, &item.id)
                    .ok()
                    .flatten()
            })
        };
        let auth_profile_id = if request.clear_secret
            && !matches!(
                pack.manifest.auth.mode,
                ExtensionPackAuthMode::OAuth2External
            ) {
            if let Some(profile_id) = existing
                .as_ref()
                .and_then(|item| item.auth_profile_id.as_deref())
            {
                let _ = crate::core::auth_profiles::AuthProfileControlPlane::delete(
                    &self.storage,
                    profile_id,
                )
                .await;
            }
            None
        } else {
            self.upsert_connection_auth_profile(
                &pack.manifest,
                &connection_id,
                request
                    .name
                    .as_deref()
                    .map(str::trim)
                    .filter(|value| !value.is_empty())
                    .or_else(|| existing.as_ref().map(|item| item.name.as_str()))
                    .unwrap_or("Default connection"),
                next_secret.as_ref(),
                existing
                    .as_ref()
                    .and_then(|item| item.auth_profile_id.as_deref()),
            )
            .await?
            .or_else(|| {
                existing
                    .as_ref()
                    .and_then(|item| item.auth_profile_id.clone())
            })
        };
        let connection = ExtensionPackConnection {
            id: connection_id.clone(),
            pack_id: pack.manifest.id.clone(),
            name: request
                .name
                .as_deref()
                .map(str::trim)
                .filter(|value| !value.is_empty())
                .map(|value| value.to_string())
                .or_else(|| existing.as_ref().map(|item| item.name.clone()))
                .unwrap_or_else(|| "Default connection".to_string()),
            enabled: request
                .enabled
                .unwrap_or_else(|| existing.as_ref().map(|item| item.enabled).unwrap_or(true)),
            auth_profile_id,
            metadata: request
                .metadata
                .clone()
                .or_else(|| existing.as_ref().map(|item| item.metadata.clone()))
                .unwrap_or(Value::Null),
            last_error: None,
            last_tested_at: existing
                .as_ref()
                .and_then(|item| item.last_tested_at.clone()),
            created_at: existing
                .as_ref()
                .map(|item| item.created_at.clone())
                .unwrap_or_else(|| now.clone()),
            updated_at: now,
        };
        if request.clear_secret {
            self.store_connection_secret(pack_id, &connection_id, None)?;
        } else if let Some(secret) = request.secret.clone() {
            self.store_connection_secret(pack_id, &connection_id, Some(secret))?;
        }
        self.connections
            .insert(connection_id.clone(), connection.clone());
        self.persist_connections().await?;
        self.connection_view(&pack.manifest, &connection).await
    }

    pub fn get_connection_secret(
        &self,
        pack_id: &str,
        connection_id: &str,
    ) -> Result<Option<Value>> {
        self.load_connection_secret(pack_id, connection_id)
    }

    pub async fn test_connection(
        &mut self,
        pack_id: &str,
        connection_id: &str,
        mcp_registry: Option<
            std::sync::Arc<tokio::sync::RwLock<crate::mcp::registry::McpRegistry>>,
        >,
        plugin_registry: Option<
            std::sync::Arc<tokio::sync::RwLock<crate::plugins::registry::PluginRegistry>>,
        >,
    ) -> Result<ExtensionPackInvokeResult> {
        let Some(pack) = self.installed.get(pack_id).cloned() else {
            anyhow::bail!("Pack '{}' is not installed", pack_id);
        };
        let Some(connection) = self.connections.get(connection_id).cloned() else {
            anyhow::bail!("Connection '{}' was not found", connection_id);
        };
        if !connection.pack_id.eq_ignore_ascii_case(&pack.manifest.id) {
            anyhow::bail!(
                "Connection '{}' belongs to pack '{}' rather than '{}'",
                connection_id,
                connection.pack_id,
                pack.manifest.id
            );
        }
        let state = self.connection_state(&pack.manifest, &connection).await?;
        if matches!(state, ExtensionConnectionState::NeedsAuth) {
            let result = ExtensionPackInvokeResult {
                ok: false,
                status: "auth_required".to_string(),
                pack_id: pack.manifest.id,
                feature_id: "health.test".to_string(),
                connection_id: Some(connection.id),
                message: Some(
                    "Connection exists but still needs credentials or authorization.".to_string(),
                ),
                data: None,
                error: Some("auth_required".to_string()),
            };
            self.persist_connection_test_result(connection_id, &result)
                .await?;
            return Ok(result);
        }
        if let Some(result) = self
            .test_legacy_channel_connection(&pack.manifest, &connection)
            .await?
        {
            self.persist_connection_test_result(connection_id, &result)
                .await?;
            return Ok(result);
        }
        if pack.manifest.id.eq_ignore_ascii_case("google_workspace") {
            let checks =
                crate::actions::google_workspace::test_selected_bundles(&self.config_dir).await?;
            let ok = checks.values().all(|value| {
                let lowered = value.to_ascii_lowercase();
                !lowered.contains("failed")
                    && !lowered.contains("unavailable")
                    && !lowered.contains("needs additional access")
                    && !lowered.contains("reconnect")
            });
            let result = ExtensionPackInvokeResult {
                ok,
                status: if ok { "ok" } else { "warning" }.to_string(),
                pack_id: pack.manifest.id,
                feature_id: "health.test".to_string(),
                connection_id: Some(connection.id),
                message: Some(if ok {
                    "Connection is ready.".to_string()
                } else {
                    "Connection needs attention.".to_string()
                }),
                data: Some(serde_json::json!({ "checks": checks })),
                error: None,
            };
            self.persist_connection_test_result(connection_id, &result)
                .await?;
            return Ok(result);
        }
        if let Some(result) = self
            .test_generic_pack_connection(
                &pack.manifest,
                &connection,
                mcp_registry,
                plugin_registry,
            )
            .await?
        {
            self.persist_connection_test_result(connection_id, &result)
                .await?;
            return Ok(result);
        }
        let result = ExtensionPackInvokeResult {
            ok: true,
            status: "ok".to_string(),
            pack_id: pack.manifest.id,
            feature_id: "health.test".to_string(),
            connection_id: Some(connection.id),
            message: Some(if pack.manifest.draft {
                "Connection saved. This draft pack does not declare a runnable live test yet, so provider health was not verified.".to_string()
            } else {
                "Connection saved. This pack does not declare a runnable live test yet, so provider health was not verified.".to_string()
            }),
            data: None,
            error: None,
        };
        self.persist_connection_test_result(connection_id, &result)
            .await?;
        Ok(result)
    }

    pub async fn set_pack_enabled(
        &mut self,
        pack_id: &str,
        enabled: bool,
    ) -> Result<ExtensionPackView> {
        let Some(pack) = self.installed.get_mut(pack_id) else {
            anyhow::bail!("Pack '{}' is not installed", pack_id);
        };
        pack.enabled = enabled;
        pack.updated_at = now_rfc3339();
        self.persist_installed().await?;
        let pack = self
            .installed
            .get(pack_id)
            .ok_or_else(|| anyhow::anyhow!("pack disappeared after update"))?;
        self.pack_view(pack).await
    }

    pub async fn delete_pack(&mut self, pack_id: &str, remove_connections: bool) -> Result<()> {
        let Some(existing) = self.installed.get(pack_id).cloned() else {
            anyhow::bail!("Pack '{}' is not installed", pack_id);
        };
        if matches!(
            existing.source_kind,
            ExtensionPackSourceKind::BundledRegistry
        ) {
            anyhow::bail!(
                "Bundled AgentArk packs cannot be deleted. Disable the pack or remove its saved connections instead."
            );
        }
        self.clear_pack_runtime_artifacts(&existing.manifest.id)?;
        self.installed.remove(pack_id);
        let mut removed_auth_profile_ids = HashSet::new();
        if remove_connections {
            let connection_ids = self
                .connections
                .values()
                .filter(|item| item.pack_id.eq_ignore_ascii_case(&existing.manifest.id))
                .map(|item| item.id.clone())
                .collect::<Vec<_>>();
            for connection_id in connection_ids {
                if let Some(connection) = self.connections.remove(&connection_id) {
                    if let Some(profile_id) = connection.auth_profile_id {
                        removed_auth_profile_ids.insert(profile_id);
                    }
                }
            }
            self.clear_pack_connection_secrets(&existing.manifest.id)?;
            self.delete_pack_auth_profiles(&existing.manifest.id, &removed_auth_profile_ids)
                .await?;
            self.persist_connections().await?;
        }
        let before_events = self.events.len();
        self.events
            .retain(|event| !event.pack_id.eq_ignore_ascii_case(&existing.manifest.id));
        if self.events.len() != before_events {
            self.persist_events().await?;
        }
        self.persist_installed().await
    }

    pub async fn invoke_feature(
        &mut self,
        request: ExtensionPackInvokeRequest,
        mcp_registry: Option<
            std::sync::Arc<tokio::sync::RwLock<crate::mcp::registry::McpRegistry>>,
        >,
        plugin_registry: Option<
            std::sync::Arc<tokio::sync::RwLock<crate::plugins::registry::PluginRegistry>>,
        >,
    ) -> Result<ExtensionPackInvokeResult> {
        let feature_id = request.feature_id.trim().to_ascii_lowercase();
        if feature_id.is_empty() {
            anyhow::bail!("feature_id is required");
        }
        let pack = self.resolve_pack_for_feature(request.pack_id.as_deref(), &feature_id)?;
        if !pack.enabled {
            return Ok(Self::error_result(
                &pack.manifest.id,
                &feature_id,
                None,
                "pack_disabled",
                "The selected pack is installed but disabled.",
            ));
        }
        let feature = pack
            .manifest
            .features
            .iter()
            .find(|item| item.id.eq_ignore_ascii_case(&feature_id))
            .cloned()
            .ok_or_else(|| {
                anyhow::anyhow!(
                    "Feature '{}' is not declared by pack '{}'",
                    feature_id,
                    pack.manifest.id
                )
            })?;
        let connection = self
            .resolve_connection_for_pack(&pack.manifest, request.connection_id.as_deref())
            .await?;
        let binding = feature.binding.clone().unwrap_or(ExtensionPackBinding {
            kind: BINDING_KIND_UNSUPPORTED.to_string(),
            config: serde_json::json!({
                "reason": "No binding declared for this feature."
            }),
        });
        let connection_secret = match connection.as_ref() {
            Some(connection) => self.load_connection_secret(&pack.manifest.id, &connection.id)?,
            None => None,
        };
        let data = match binding.kind.trim().to_ascii_lowercase().as_str() {
            BINDING_KIND_LEGACY_ACTION => Some(
                invoke_legacy_action_binding(&self.config_dir, &binding, &request.arguments)
                    .await?,
            ),
            BINDING_KIND_LEGACY_CHANNEL => Some(
                self.invoke_legacy_channel_binding(
                    &pack.manifest,
                    &feature,
                    &binding,
                    &request.arguments,
                    connection.as_ref(),
                    connection_secret.as_ref(),
                )
                .await?,
            ),
            BINDING_KIND_HTTP => Some(
                self.invoke_http_binding(
                    &pack.manifest,
                    &binding,
                    &request.arguments,
                    connection.as_ref(),
                    connection_secret.as_ref(),
                )
                .await?,
            ),
            BINDING_KIND_LOCAL_CLI => Some(
                self.invoke_local_cli_binding(
                    &pack.manifest,
                    &binding,
                    &request.arguments,
                    connection.as_ref(),
                    connection_secret.as_ref(),
                )
                .await?,
            ),
            BINDING_KIND_MCP_TOOL => {
                let Some(registry) = mcp_registry else {
                    return Ok(Self::error_result(
                        &pack.manifest.id,
                        &feature_id,
                        connection.as_ref().map(|item| item.id.as_str()),
                        "mcp_unavailable",
                        "The MCP registry is not available in this runtime.",
                    ));
                };
                let server_id = binding
                    .config
                    .get("server_id")
                    .and_then(|value| value.as_str())
                    .map(str::trim)
                    .filter(|value| !value.is_empty())
                    .ok_or_else(|| anyhow::anyhow!("mcp_tool binding requires server_id"))?;
                let tool_name = binding
                    .config
                    .get("tool_name")
                    .and_then(|value| value.as_str())
                    .map(str::trim)
                    .filter(|value| !value.is_empty())
                    .ok_or_else(|| anyhow::anyhow!("mcp_tool binding requires tool_name"))?;
                let mut guard = registry.write().await;
                Some(parse_action_payload(
                    &guard
                        .call_tool(server_id, tool_name, &request.arguments)
                        .await?,
                ))
            }
            BINDING_KIND_MCP_RESOURCE => {
                let Some(registry) = mcp_registry else {
                    return Ok(Self::error_result(
                        &pack.manifest.id,
                        &feature_id,
                        connection.as_ref().map(|item| item.id.as_str()),
                        "mcp_unavailable",
                        "The MCP registry is not available in this runtime.",
                    ));
                };
                let server_id = binding
                    .config
                    .get("server_id")
                    .and_then(|value| value.as_str())
                    .map(str::trim)
                    .filter(|value| !value.is_empty())
                    .ok_or_else(|| anyhow::anyhow!("mcp_resource binding requires server_id"))?;
                let uri = binding
                    .config
                    .get("uri")
                    .and_then(|value| value.as_str())
                    .map(str::trim)
                    .filter(|value| !value.is_empty())
                    .ok_or_else(|| anyhow::anyhow!("mcp_resource binding requires uri"))?;
                let mut guard = registry.write().await;
                Some(parse_action_payload(
                    &guard.read_resource(server_id, uri).await?,
                ))
            }
            BINDING_KIND_PLUGIN => {
                let Some(registry) = plugin_registry else {
                    return Ok(Self::error_result(
                        &pack.manifest.id,
                        &feature_id,
                        connection.as_ref().map(|item| item.id.as_str()),
                        "plugin_unavailable",
                        "The plugin registry is not available in this runtime.",
                    ));
                };
                let plugin_id = binding
                    .config
                    .get("plugin_id")
                    .and_then(|value| value.as_str())
                    .map(str::trim)
                    .filter(|value| !value.is_empty())
                    .ok_or_else(|| anyhow::anyhow!("plugin binding requires plugin_id"))?;
                let action_name = binding
                    .config
                    .get("action_name")
                    .and_then(|value| value.as_str())
                    .map(str::trim)
                    .filter(|value| !value.is_empty())
                    .ok_or_else(|| anyhow::anyhow!("plugin binding requires action_name"))?;
                let mut guard = registry.write().await;
                Some(parse_action_payload(
                    &guard
                        .invoke_action(plugin_id, action_name, &request.arguments)
                        .await?,
                ))
            }
            _ => {
                let reason = binding
                    .config
                    .get("reason")
                    .and_then(|value| value.as_str())
                    .unwrap_or("This feature is installed as a draft placeholder and does not have an executable binding yet.");
                return Ok(Self::error_result(
                    &pack.manifest.id,
                    &feature_id,
                    connection.as_ref().map(|item| item.id.as_str()),
                    "binding_unsupported",
                    reason,
                ));
            }
        };
        Ok(ExtensionPackInvokeResult {
            ok: true,
            status: "ok".to_string(),
            pack_id: pack.manifest.id,
            feature_id,
            connection_id: connection.map(|item| item.id),
            message: Some("Feature invocation completed.".to_string()),
            data,
            error: None,
        })
    }

    fn error_result(
        pack_id: &str,
        feature_id: &str,
        connection_id: Option<&str>,
        status: &str,
        message: &str,
    ) -> ExtensionPackInvokeResult {
        ExtensionPackInvokeResult {
            ok: false,
            status: status.to_string(),
            pack_id: pack_id.to_string(),
            feature_id: feature_id.to_string(),
            connection_id: connection_id.map(|value| value.to_string()),
            message: Some(message.to_string()),
            data: None,
            error: Some(status.to_string()),
        }
    }

    async fn pack_view(&self, pack: &InstalledExtensionPack) -> Result<ExtensionPackView> {
        let connection_views = self.list_connections(&pack.manifest.id).await?;
        let has_ready = connection_views
            .iter()
            .any(|item| matches!(item.state, ExtensionConnectionState::Ready));
        let has_error = connection_views
            .iter()
            .any(|item| matches!(item.state, ExtensionConnectionState::Error));
        let (runtime_status, runtime_detail, runtime_executable) =
            runtime_state_for_manifest(&pack.manifest, &pack.runtime_state);
        let runtime_ready = matches!(
            runtime_status,
            ExtensionPackRuntimeStatus::NotRequired | ExtensionPackRuntimeStatus::Ready
        );
        let needs_auth =
            !matches!(pack.manifest.auth.mode, ExtensionPackAuthMode::None) && !has_ready;
        let status = if !pack.enabled {
            "disabled"
        } else if !runtime_ready {
            "runtime_missing"
        } else if has_ready {
            "connected"
        } else if has_error {
            "error"
        } else if needs_auth {
            "needs_auth"
        } else {
            "ready"
        };
        let status_detail = if !runtime_ready {
            runtime_detail.clone()
        } else if needs_auth {
            Some(
                "Install completed, but this pack still needs a connected account or secret."
                    .to_string(),
            )
        } else if pack.manifest.draft {
            Some("Installed as a draft pack. Review bindings before depending on it for production workflows.".to_string())
        } else {
            None
        };
        Ok(ExtensionPackView {
            manifest: pack.manifest.clone(),
            installed: true,
            enabled: pack.enabled,
            trust_level: pack.trust_level,
            verification_status: pack.verification_status.clone(),
            verification_detail: pack.verification_detail.clone(),
            source_kind: pack.source_kind,
            source_url: pack.source_url.clone(),
            needs_auth,
            status: status.to_string(),
            status_detail,
            runtime_required: manifest_requires_runtime(&pack.manifest),
            runtime_status,
            runtime_detail,
            runtime_executable,
            supports_connect_url: pack_supports_connect_url_manifest(&pack.manifest),
            supports_webhook: pack_supports_webhook_manifest(&pack.manifest),
            webhook_path: self.webhook_path(&pack.manifest.id),
            feature_summaries: feature_summaries(&pack.manifest),
        })
    }

    async fn catalog_view(&self, manifest: &ExtensionPackManifest) -> Result<ExtensionPackView> {
        Ok(ExtensionPackView {
            manifest: manifest.clone(),
            installed: false,
            enabled: false,
            trust_level: ExtensionPackTrustLevel::Trusted,
            verification_status: "bundled".to_string(),
            verification_detail: Some("Bundled first-party pack.".to_string()),
            source_kind: ExtensionPackSourceKind::BundledRegistry,
            source_url: None,
            needs_auth: !matches!(manifest.auth.mode, ExtensionPackAuthMode::None),
            status: if manifest.draft {
                "draft".to_string()
            } else {
                "available".to_string()
            },
            status_detail: None,
            runtime_required: manifest_requires_runtime(manifest),
            runtime_status: if manifest_requires_runtime(manifest) {
                ExtensionPackRuntimeStatus::Missing
            } else {
                ExtensionPackRuntimeStatus::NotRequired
            },
            runtime_detail: runtime_installer(manifest).map(|installer| {
                format!(
                    "This pack uses local runtime '{}', which will need an explicit install step.",
                    installer.executable
                )
            }),
            runtime_executable: runtime_installer(manifest)
                .map(|installer| installer.executable.clone()),
            supports_connect_url: pack_supports_connect_url_manifest(manifest),
            supports_webhook: pack_supports_webhook_manifest(manifest),
            webhook_path: self.webhook_path(&manifest.id),
            feature_summaries: feature_summaries(manifest),
        })
    }

    async fn connection_view(
        &self,
        manifest: &ExtensionPackManifest,
        connection: &ExtensionPackConnection,
    ) -> Result<ExtensionPackConnectionView> {
        let has_secret = self
            .load_connection_secret(&manifest.id, &connection.id)?
            .is_some_and(|value| !value.is_null())
            || connection
                .auth_profile_id
                .as_deref()
                .is_some_and(|value| !value.trim().is_empty());
        let state = self.connection_state(manifest, connection).await?;
        Ok(ExtensionPackConnectionView {
            connection: connection.clone(),
            state,
            auth_mode: manifest.auth.mode,
            has_secret,
        })
    }

    async fn connection_state(
        &self,
        manifest: &ExtensionPackManifest,
        connection: &ExtensionPackConnection,
    ) -> Result<ExtensionConnectionState> {
        if !connection.enabled {
            return Ok(ExtensionConnectionState::Disabled);
        }
        if manifest.id.eq_ignore_ascii_case("google_workspace") {
            let (connected, _granted, missing) =
                crate::actions::google_workspace::summarize_connection_status(&self.config_dir)?;
            if connected && missing.is_empty() {
                return Ok(ExtensionConnectionState::Ready);
            }
            return Ok(ExtensionConnectionState::NeedsAuth);
        }
        if let Some(profile_id) = connection.auth_profile_id.as_deref() {
            if let Some(profile) =
                crate::core::auth_profiles::AuthProfileControlPlane::get(&self.storage, profile_id)
                    .await?
            {
                return if profile.ready {
                    Ok(ExtensionConnectionState::Ready)
                } else if matches!(
                    profile.status,
                    crate::core::auth_profiles::AuthProfileStatus::Error
                ) {
                    Ok(ExtensionConnectionState::Error)
                } else {
                    Ok(ExtensionConnectionState::NeedsAuth)
                };
            }
        }
        let has_secret = self
            .load_connection_secret(&manifest.id, &connection.id)?
            .is_some_and(|value| !value.is_null());
        match manifest.auth.mode {
            ExtensionPackAuthMode::None => Ok(ExtensionConnectionState::Ready),
            ExtensionPackAuthMode::ApiKey | ExtensionPackAuthMode::Basic => {
                if has_secret {
                    Ok(ExtensionConnectionState::Ready)
                } else {
                    Ok(ExtensionConnectionState::NeedsAuth)
                }
            }
            ExtensionPackAuthMode::OAuth2External => {
                if manifest.auth.oauth2.is_none()
                    && manifest_uses_connection_secret(manifest)
                    && has_secret
                {
                    Ok(ExtensionConnectionState::Ready)
                } else {
                    Ok(ExtensionConnectionState::NeedsAuth)
                }
            }
        }
    }

    fn resolve_pack_for_feature(
        &self,
        requested_pack_id: Option<&str>,
        feature_id: &str,
    ) -> Result<InstalledExtensionPack> {
        if let Some(pack_id) = requested_pack_id
            .map(str::trim)
            .filter(|value| !value.is_empty())
        {
            return self
                .installed
                .get(pack_id)
                .cloned()
                .ok_or_else(|| anyhow::anyhow!("Pack '{}' is not installed", pack_id));
        }
        self.installed
            .values()
            .find(|pack| {
                pack.manifest
                    .features
                    .iter()
                    .any(|feature| feature.id.eq_ignore_ascii_case(feature_id))
            })
            .cloned()
            .ok_or_else(|| anyhow::anyhow!("No installed pack provides feature '{}'", feature_id))
    }

    async fn resolve_connection_for_pack(
        &self,
        manifest: &ExtensionPackManifest,
        requested_connection_id: Option<&str>,
    ) -> Result<Option<ExtensionPackConnection>> {
        if matches!(manifest.auth.mode, ExtensionPackAuthMode::None) {
            return Ok(None);
        }
        if let Some(connection_id) = requested_connection_id
            .map(str::trim)
            .filter(|value| !value.is_empty())
        {
            let connection = self
                .connections
                .get(connection_id)
                .cloned()
                .ok_or_else(|| anyhow::anyhow!("Connection '{}' was not found", connection_id))?;
            if !connection.pack_id.eq_ignore_ascii_case(&manifest.id) {
                anyhow::bail!(
                    "Connection '{}' belongs to pack '{}' rather than '{}'",
                    connection_id,
                    connection.pack_id,
                    manifest.id
                );
            }
            return Ok(Some(connection));
        }
        self.preferred_connection_for_pack(manifest)
            .await?
            .map(Some)
            .ok_or_else(|| {
                anyhow::anyhow!("No connection is configured for pack '{}'", manifest.id)
            })
    }

    fn ensure_default_connection(&mut self, pack_id: &str) -> bool {
        let Some(pack) = self.installed.get(pack_id) else {
            return false;
        };
        if matches!(pack.manifest.auth.mode, ExtensionPackAuthMode::None) {
            return false;
        }
        let exists = self
            .connections
            .values()
            .any(|item| item.pack_id == pack.manifest.id);
        if exists {
            return false;
        }
        let now = now_rfc3339();
        let connection = ExtensionPackConnection {
            id: format!("{}-default", sanitize_pack_id(&pack.manifest.id)),
            pack_id: pack.manifest.id.clone(),
            name: "Default connection".to_string(),
            enabled: true,
            auth_profile_id: None,
            metadata: Value::Null,
            last_error: None,
            last_tested_at: None,
            created_at: now.clone(),
            updated_at: now,
        };
        self.connections.insert(connection.id.clone(), connection);
        true
    }

    fn load_connection_secret(&self, pack_id: &str, connection_id: &str) -> Result<Option<Value>> {
        let manager = crate::core::config::SecureConfigManager::new_with_data_dir(
            &self.config_dir,
            Some(&self.data_dir),
        )?;
        let Some(raw) =
            manager.get_custom_secret(&connection_secret_key(pack_id, connection_id))?
        else {
            return Ok(None);
        };
        Ok(serde_json::from_str::<Value>(&raw)
            .ok()
            .or(Some(Value::String(raw))))
    }

    fn store_connection_secret(
        &self,
        pack_id: &str,
        connection_id: &str,
        value: Option<Value>,
    ) -> Result<()> {
        let manager = crate::core::config::SecureConfigManager::new_with_data_dir(
            &self.config_dir,
            Some(&self.data_dir),
        )?;
        let encoded = match value {
            Some(Value::String(text)) => Some(text),
            Some(other) => Some(serde_json::to_string(&other)?),
            None => None,
        };
        manager.set_custom_secret(&connection_secret_key(pack_id, connection_id), encoded)
    }

    async fn upsert_connection_auth_profile(
        &self,
        manifest: &ExtensionPackManifest,
        connection_id: &str,
        connection_name: &str,
        secret: Option<&Value>,
        existing_profile_id: Option<&str>,
    ) -> Result<Option<String>> {
        use crate::core::auth_profiles::{
            AuthProfileControlPlane, AuthProfileKind, AuthProfileMaterial, AuthProfileScope,
            AuthProfileUpsert, OAuth2ProfileConfigRecord,
        };

        let profile_id = existing_profile_id
            .map(|value| value.trim().to_string())
            .filter(|value| !value.is_empty())
            .unwrap_or_else(|| connection_auth_profile_id(&manifest.id, connection_id));

        let material = match manifest.auth.mode {
            ExtensionPackAuthMode::None => return Ok(None),
            ExtensionPackAuthMode::ApiKey => {
                let Some(secret) = secret else {
                    return Ok(existing_profile_id.map(|value| value.to_string()));
                };
                Some(auth_profile_material_for_secret_backed_pack(
                    manifest, secret,
                )?)
            }
            ExtensionPackAuthMode::Basic => {
                let Some(secret) = secret else {
                    return Ok(existing_profile_id.map(|value| value.to_string()));
                };
                let username = select_json_path(secret, "username")
                    .and_then(scalar_to_string)
                    .filter(|value| !value.trim().is_empty())
                    .ok_or_else(|| anyhow!("Basic auth connections require secret.username"))?;
                let password = select_json_path(secret, "password")
                    .and_then(scalar_to_string)
                    .filter(|value| !value.trim().is_empty())
                    .ok_or_else(|| anyhow!("Basic auth connections require secret.password"))?;
                Some(AuthProfileMaterial::Basic { username, password })
            }
            ExtensionPackAuthMode::OAuth2External => match manifest.auth.oauth2.as_ref() {
                Some(oauth2) => {
                    let existing = AuthProfileControlPlane::get(&self.storage, &profile_id).await?;
                    if existing.is_some() {
                        return Ok(Some(profile_id));
                    }
                    Some(AuthProfileMaterial::OAuth2 {
                        config: Box::new(OAuth2ProfileConfigRecord {
                            client_id: oauth2.client_id.clone(),
                            client_secret: oauth2.client_secret.clone(),
                            auth_url: oauth2.auth_url.clone(),
                            token_url: oauth2.token_url.clone(),
                            redirect_uri: oauth2.redirect_uri.clone(),
                            scopes: oauth2.scopes.clone(),
                            auth_header: oauth2.auth_header.clone(),
                            use_pkce: oauth2.use_pkce,
                            extra_auth_params: oauth2.extra_auth_params.clone(),
                            extra_token_params: oauth2.extra_token_params.clone(),
                            prompt: oauth2.prompt.clone(),
                            access_type: oauth2.access_type.clone(),
                        }),
                        tokens: None,
                    })
                }
                None => return Ok(existing_profile_id.map(|value| value.to_string())),
            },
        };

        let Some(material) = material else {
            return Ok(None);
        };
        let kind = match &material {
            AuthProfileMaterial::ApiKey { .. } => AuthProfileKind::ApiKey,
            AuthProfileMaterial::Basic { .. } => AuthProfileKind::Basic,
            AuthProfileMaterial::OAuth2 { .. } => AuthProfileKind::OAuth2,
            AuthProfileMaterial::Bearer { .. } => AuthProfileKind::Bearer,
            AuthProfileMaterial::Header { .. } => AuthProfileKind::Header,
            AuthProfileMaterial::Query { .. } => AuthProfileKind::Query,
            AuthProfileMaterial::CookieSession { .. } => AuthProfileKind::CookieSession,
            AuthProfileMaterial::BrowserSession { .. } => AuthProfileKind::BrowserSession,
            AuthProfileMaterial::ServiceAccount { .. } => AuthProfileKind::ServiceAccount,
        };
        AuthProfileControlPlane::upsert(
            &self.storage,
            AuthProfileUpsert {
                id: Some(profile_id.clone()),
                name: Some(format!("{} {}", manifest.name, connection_name)),
                kind: Some(kind),
                enabled: Some(true),
                scope: Some(AuthProfileScope::User),
                provider: Some(manifest.id.clone()),
                description: Some(format!(
                    "Managed by extension pack '{}' for connection '{}'.",
                    manifest.name, connection_name
                )),
                metadata: Some(serde_json::json!({
                    "extension_pack_id": manifest.id,
                    "connection_id": connection_id,
                })),
                material: Some(material),
                ..AuthProfileUpsert::default()
            },
        )
        .await?;
        Ok(Some(profile_id))
    }

    async fn resolve_auth_profile_headers(
        &self,
        profile_id: &str,
        requested: &BTreeMap<String, String>,
    ) -> Result<BTreeMap<String, String>> {
        crate::core::auth_profiles::AuthProfileControlPlane::resolve_env_exports(
            &self.storage,
            profile_id,
            requested,
        )
        .await
    }

    async fn mark_connection_auth_used(&self, connection: Option<&ExtensionPackConnection>) {
        let Some(profile_id) = connection.and_then(|value| value.auth_profile_id.as_deref()) else {
            return;
        };
        let _ = crate::core::auth_profiles::AuthProfileControlPlane::mark_used(
            &self.storage,
            profile_id,
        )
        .await;
    }

    async fn persist_installed(&self) -> Result<()> {
        let mut items = self.installed.values().cloned().collect::<Vec<_>>();
        items.sort_by(|left, right| left.manifest.id.cmp(&right.manifest.id));
        save_json(&self.storage, INSTALLED_PACKS_KEY, &items).await
    }

    async fn persist_connections(&self) -> Result<()> {
        let mut items = self.connections.values().cloned().collect::<Vec<_>>();
        items.sort_by(|left, right| left.id.cmp(&right.id));
        save_json(&self.storage, CONNECTIONS_KEY, &items).await
    }

    async fn persist_events(&self) -> Result<()> {
        save_json(&self.storage, EVENTS_KEY, &self.events).await
    }

    fn clear_pack_connection_secrets(&self, pack_id: &str) -> Result<()> {
        let prefix = connection_secret_prefix(pack_id);
        let manager = crate::core::config::SecureConfigManager::new_with_data_dir(
            &self.config_dir,
            Some(&self.data_dir),
        )?;
        manager.update_custom_secrets(|custom| {
            custom.retain(|key, _| !key.starts_with(&prefix));
            Ok(())
        })
    }

    fn pack_runtime_dir(&self, pack_id: &str) -> PathBuf {
        self.data_dir
            .join("extension-pack-runtime")
            .join(sanitize_pack_id(pack_id))
    }

    fn clear_pack_runtime_artifacts(&self, pack_id: &str) -> Result<()> {
        let runtime_dir = self.pack_runtime_dir(pack_id);
        if !runtime_dir.exists() {
            return Ok(());
        }
        if !runtime_dir.is_dir() {
            anyhow::bail!(
                "Extension pack runtime path '{}' is not a directory",
                runtime_dir.display()
            );
        }
        std::fs::remove_dir_all(&runtime_dir).with_context(|| {
            format!(
                "Failed to remove extension-pack runtime directory '{}'",
                runtime_dir.display()
            )
        })
    }

    async fn delete_pack_auth_profiles(
        &self,
        pack_id: &str,
        explicit_profile_ids: &HashSet<String>,
    ) -> Result<()> {
        let mut profile_ids = explicit_profile_ids
            .iter()
            .map(|value| value.trim().to_string())
            .filter(|value| !value.is_empty())
            .collect::<HashSet<_>>();
        let profiles = crate::core::auth_profiles::AuthProfileControlPlane::list(&self.storage)
            .await?
            .profiles;
        for profile in profiles {
            let provider_matches = profile
                .provider
                .as_deref()
                .is_some_and(|value| value.eq_ignore_ascii_case(pack_id));
            let metadata_matches = profile
                .metadata
                .as_ref()
                .and_then(|value| value.get("extension_pack_id"))
                .and_then(Value::as_str)
                .is_some_and(|value| value.eq_ignore_ascii_case(pack_id));
            if provider_matches || metadata_matches {
                profile_ids.insert(profile.id);
            }
        }
        for profile_id in profile_ids {
            let _ = crate::core::auth_profiles::AuthProfileControlPlane::delete(
                &self.storage,
                &profile_id,
            )
            .await?;
        }
        Ok(())
    }

    async fn run_runtime_commands(
        &self,
        pack_id: &str,
        commands: &[ExtensionPackCommandSpec],
    ) -> Result<String> {
        let pack = self
            .installed
            .get(pack_id)
            .ok_or_else(|| anyhow!("Pack '{}' is not installed", pack_id))?;
        if !matches!(pack.trust_level, ExtensionPackTrustLevel::Trusted) {
            anyhow::bail!(
                "Runtime commands are blocked for unverified extension pack '{}'",
                pack_id
            );
        }
        let runtime_dir = self.pack_runtime_dir(pack_id);
        std::fs::create_dir_all(&runtime_dir).with_context(|| {
            format!(
                "Failed to create extension-pack runtime directory '{}'",
                runtime_dir.display()
            )
        })?;
        let matching = commands
            .iter()
            .filter(|command| command_matches_platform(command))
            .collect::<Vec<_>>();
        if matching.is_empty() {
            anyhow::bail!(
                "Pack '{}' does not define runtime commands for {} / {}.",
                pack_id,
                current_platform_os(),
                current_platform_arch()
            );
        }
        let mut outputs = Vec::new();
        for command in matching {
            let program = command.program.trim();
            if program.is_empty() {
                anyhow::bail!("Runtime command program cannot be empty");
            }
            let mut child = tokio::process::Command::new(program);
            child.env_clear();
            child.current_dir(&runtime_dir);
            child.args(&command.args);
            for (key, value) in &command.env {
                child.env(key, value);
            }
            child.stdin(std::process::Stdio::piped());
            child.stdout(std::process::Stdio::piped());
            child.stderr(std::process::Stdio::piped());
            let mut child = child
                .spawn()
                .with_context(|| format!("Failed to launch runtime command '{}'", program))?;
            if let Some(stdin) = command.stdin.as_deref() {
                if let Some(mut writer) = child.stdin.take() {
                    use tokio::io::AsyncWriteExt;
                    writer.write_all(stdin.as_bytes()).await?;
                }
            }
            let output = tokio::time::timeout(
                std::time::Duration::from_secs(command.timeout_secs.unwrap_or(300)),
                child.wait_with_output(),
            )
            .await
            .map_err(|_| anyhow!("Runtime command '{}' timed out", program))??;
            let stdout = String::from_utf8_lossy(&output.stdout).trim().to_string();
            let stderr = String::from_utf8_lossy(&output.stderr).trim().to_string();
            let combined = [stdout.as_str(), stderr.as_str()]
                .into_iter()
                .filter(|value| !value.is_empty())
                .collect::<Vec<_>>()
                .join("\n");
            if !output.status.success() && !command.allow_failure {
                anyhow::bail!(
                    "Runtime command '{}' failed with status {}. {}",
                    program,
                    output
                        .status
                        .code()
                        .map(|code| code.to_string())
                        .unwrap_or_else(|| "unknown".to_string()),
                    if combined.is_empty() {
                        "(no output)".to_string()
                    } else {
                        crate::security::redact_secret_input(&combined).text
                    }
                );
            }
            if !combined.is_empty() {
                outputs.push(crate::security::redact_secret_input(&combined).text);
            }
        }
        Ok(if outputs.is_empty() {
            "Runtime command completed.".to_string()
        } else {
            outputs.join("\n\n")
        })
    }

    pub async fn resolve_webhook_binding(
        &self,
        pack_id: &str,
    ) -> Result<ResolvedExtensionPackWebhook> {
        let pack = self
            .installed
            .get(pack_id)
            .cloned()
            .ok_or_else(|| anyhow!("Pack '{}' is not installed", pack_id))?;
        if !pack.enabled {
            anyhow::bail!("Pack '{}' is installed but disabled", pack_id);
        }
        let feature = pack
            .manifest
            .features
            .iter()
            .find(|feature| {
                feature.kind.eq_ignore_ascii_case(FEATURE_KIND_EVENT)
                    && feature.id.eq_ignore_ascii_case("message.receive")
                    && feature.binding.as_ref().is_some()
            })
            .cloned()
            .ok_or_else(|| {
                anyhow!(
                    "Pack '{}' does not expose an inbound event binding",
                    pack_id
                )
            })?;
        feature
            .binding
            .clone()
            .ok_or_else(|| anyhow!("Pack '{}' inbound event is missing its binding", pack_id))?;
        let connection = if matches!(pack.manifest.auth.mode, ExtensionPackAuthMode::None) {
            None
        } else {
            self.resolve_connection_for_pack(&pack.manifest, None)
                .await?
        };
        let secret = match connection.as_ref() {
            Some(connection) => self.load_connection_secret(&pack.manifest.id, &connection.id)?,
            None => None,
        };
        Ok(ResolvedExtensionPackWebhook {
            manifest: pack.manifest,
            feature,
            connection_id: connection.map(|item| item.id),
            secret,
        })
    }

    pub async fn list_events(
        &self,
        pack_id: &str,
        limit: usize,
    ) -> Result<ExtensionPackEventsResponse> {
        let mut items = self
            .events
            .iter()
            .filter(|item| item.pack_id.eq_ignore_ascii_case(pack_id))
            .cloned()
            .collect::<Vec<_>>();
        items.sort_by(|left, right| right.received_at.cmp(&left.received_at));
        items.truncate(limit.clamp(1, MAX_STORED_PACK_EVENTS));
        Ok(ExtensionPackEventsResponse {
            pack_id: pack_id.to_string(),
            count: items.len(),
            items,
        })
    }

    pub async fn record_event_received(
        &mut self,
        pack_id: &str,
        feature_id: &str,
        connection_id: Option<&str>,
        transport: &str,
        event_type: &str,
        provider_event_id: Option<&str>,
        metadata: Value,
        payload: Value,
        initial_status: &str,
        outcome: Option<&str>,
        response_preview: Option<&str>,
    ) -> Result<ExtensionPackEventRecord> {
        let event = ExtensionPackEventRecord {
            id: uuid::Uuid::new_v4().to_string(),
            pack_id: pack_id.to_string(),
            feature_id: feature_id.to_string(),
            connection_id: connection_id.map(|value| value.to_string()),
            event_type: event_type.to_string(),
            provider_event_id: provider_event_id.map(|value| value.to_string()),
            transport: transport.to_string(),
            status: initial_status.to_string(),
            outcome: outcome.map(|value| value.to_string()),
            response_preview: response_preview.map(|value| value.to_string()),
            metadata: sanitize_event_value(&metadata),
            payload: sanitize_event_value(&payload),
            received_at: now_rfc3339(),
            processed_at: if matches!(initial_status, "processed" | "ignored" | "error") {
                Some(now_rfc3339())
            } else {
                None
            },
        };
        self.events.push(event.clone());
        if self.events.len() > MAX_STORED_PACK_EVENTS {
            let overflow = self.events.len() - MAX_STORED_PACK_EVENTS;
            self.events.drain(0..overflow);
        }
        self.persist_events().await?;
        Ok(event)
    }

    pub async fn finish_event(
        &mut self,
        event_id: &str,
        status: &str,
        outcome: Option<&str>,
        response_preview: Option<&str>,
    ) -> Result<()> {
        let Some(event) = self.events.iter_mut().find(|item| item.id == event_id) else {
            return Ok(());
        };
        event.status = status.to_string();
        event.outcome = outcome.map(|value| value.to_string());
        event.response_preview = response_preview.map(|value| value.to_string());
        event.processed_at = Some(now_rfc3339());
        self.persist_events().await
    }

    pub async fn install_uploaded_bundle(
        &mut self,
        filename: Option<&str>,
        bytes: &[u8],
        trust_unverified: bool,
    ) -> Result<ExtensionPackView> {
        let label = filename
            .map(str::trim)
            .filter(|value| !value.is_empty())
            .unwrap_or("uploaded pack bundle");
        let manifest = self.parse_manifest_bytes(bytes, label)?;
        let view = self
            .install(ExtensionPackInstallRequest {
                pack_id: None,
                source_url: None,
                source_path: None,
                manifest: Some(manifest),
                manifest_text: None,
                trust_unverified,
            })
            .await?;
        if let Some(installed) = self.installed.get_mut(&view.manifest.id) {
            installed.source_kind = ExtensionPackSourceKind::UploadedBundle;
            installed.source_url = filename.map(|value| value.to_string());
            installed.updated_at = now_rfc3339();
        }
        self.persist_installed().await?;
        self.get_pack(&view.manifest.id)
            .await?
            .ok_or_else(|| anyhow!("uploaded pack disappeared after install"))
    }

    fn fetch_manifest_from_path(&self, source_path: &str) -> Result<ExtensionPackManifest> {
        let bytes = std::fs::read(source_path).with_context(|| {
            format!(
                "failed to read pack manifest or bundle from {}",
                source_path
            )
        })?;
        self.parse_manifest_bytes(&bytes, source_path)
    }

    async fn fetch_manifest_from_url(&self, source_url: &str) -> Result<ExtensionPackManifest> {
        let response = self
            .http_client
            .get(source_url)
            .send()
            .await
            .with_context(|| format!("failed to fetch pack manifest from {}", source_url))?;
        let response = response
            .error_for_status()
            .with_context(|| format!("pack manifest request failed for {}", source_url))?;
        let bytes = response
            .bytes()
            .await
            .with_context(|| format!("failed to read pack manifest bytes from {}", source_url))?;
        self.parse_manifest_bytes(bytes.as_ref(), source_url)
    }

    fn parse_manifest_text(&self, raw: &str, source_label: &str) -> Result<ExtensionPackManifest> {
        serde_json::from_str::<ExtensionPackManifest>(raw)
            .or_else(|_| serde_yaml::from_str::<ExtensionPackManifest>(raw))
            .with_context(|| {
                format!(
                    "failed to decode extension-pack manifest from {}",
                    source_label
                )
            })
    }

    fn parse_manifest_bytes(
        &self,
        bytes: &[u8],
        source_label: &str,
    ) -> Result<ExtensionPackManifest> {
        if let Ok(text) = std::str::from_utf8(bytes) {
            if let Ok(manifest) = self.parse_manifest_text(text, source_label) {
                return Ok(manifest);
            }
        }
        let cursor = Cursor::new(bytes.to_vec());
        let mut archive = ZipArchive::new(cursor).with_context(|| {
            format!(
                "failed to decode extension-pack manifest or bundle from {}",
                source_label
            )
        })?;
        if archive.len() > MAX_BUNDLE_ENTRIES {
            anyhow::bail!("Extension-pack bundle has too many entries");
        }
        let mut total_uncompressed = 0u64;
        for index in 0..archive.len() {
            let file = archive
                .by_index(index)
                .with_context(|| format!("failed to inspect bundle entry in {}", source_label))?;
            validate_zip_entry_name(file.name())?;
            total_uncompressed = total_uncompressed.saturating_add(file.size());
            if total_uncompressed > MAX_BUNDLE_UNCOMPRESSED_BYTES {
                anyhow::bail!("Extension-pack bundle is too large after decompression");
            }
        }
        let preferred_names = [
            "extension-pack.json",
            "extension-pack.yaml",
            "extension-pack.yml",
            "pack.json",
            "pack.yaml",
            "pack.yml",
            "manifest.json",
            "manifest.yaml",
            "manifest.yml",
        ];
        for preferred in preferred_names {
            for index in 0..archive.len() {
                let name = {
                    let file = archive.by_index(index).with_context(|| {
                        format!("failed to inspect bundle entry in {}", source_label)
                    })?;
                    file.name().to_string()
                };
                if !name
                    .rsplit('/')
                    .next()
                    .is_some_and(|value| value.eq_ignore_ascii_case(preferred))
                {
                    continue;
                }
                let mut file = archive.by_index(index).with_context(|| {
                    format!("failed to open bundle entry '{}' in {}", name, source_label)
                })?;
                if file.size() > MAX_BUNDLE_MANIFEST_BYTES {
                    anyhow::bail!("Extension-pack manifest '{}' is too large", name);
                }
                let mut raw = String::new();
                file.read_to_string(&mut raw).with_context(|| {
                    format!(
                        "failed to read bundle manifest '{}' in {}",
                        name, source_label
                    )
                })?;
                return self.parse_manifest_text(&raw, &format!("{}:{}", source_label, name));
            }
        }
        anyhow::bail!(
            "No extension-pack manifest file was found in {}. Expected one of: {}",
            source_label,
            preferred_names.join(", ")
        );
    }

    async fn invoke_legacy_channel_binding(
        &self,
        manifest: &ExtensionPackManifest,
        feature: &PackFeatureManifest,
        binding: &ExtensionPackBinding,
        arguments: &Value,
        connection: Option<&ExtensionPackConnection>,
        secret: Option<&Value>,
    ) -> Result<Value> {
        let channel_id = binding
            .config
            .get("channel_id")
            .and_then(|value| value.as_str())
            .map(str::trim)
            .filter(|value| !value.is_empty())
            .unwrap_or(manifest.id.as_str());
        let operation = binding
            .config
            .get("operation")
            .and_then(|value| value.as_str())
            .map(str::trim)
            .filter(|value| !value.is_empty())
            .unwrap_or(match feature.id.as_str() {
                "message.send" => "send",
                "message.list_threads" => "list_threads",
                _ => "receive_status",
            });
        let secret = secret.ok_or_else(|| {
            anyhow!(
                "Pack '{}' needs a saved secret payload before '{}' can run",
                manifest.id,
                feature.id
            )
        })?;
        match channel_id {
            "slack_channel" => {
                invoke_slack_channel_operation(operation, arguments, connection, secret).await
            }
            "teams_channel" => {
                invoke_teams_channel_operation(operation, arguments, connection, secret).await
            }
            "whatsapp_channel" => {
                invoke_whatsapp_channel_operation(operation, arguments, connection, secret).await
            }
            other => Err(anyhow!("Unsupported legacy channel binding '{}'", other)),
        }
    }

    async fn test_legacy_channel_connection(
        &self,
        manifest: &ExtensionPackManifest,
        connection: &ExtensionPackConnection,
    ) -> Result<Option<ExtensionPackInvokeResult>> {
        let Some(feature) = manifest.features.iter().find(|feature| {
            feature.binding.as_ref().is_some_and(|binding| {
                binding
                    .kind
                    .eq_ignore_ascii_case(BINDING_KIND_LEGACY_CHANNEL)
            })
        }) else {
            return Ok(None);
        };
        let Some(binding) = feature.binding.as_ref() else {
            return Ok(None);
        };
        let Some(secret) = self.load_connection_secret(&manifest.id, &connection.id)? else {
            return Ok(Some(Self::error_result(
                &manifest.id,
                "health.test",
                Some(connection.id.as_str()),
                "auth_required",
                "This channel pack still needs secret configuration.",
            )));
        };
        let channel_id = binding
            .config
            .get("channel_id")
            .and_then(|value| value.as_str())
            .unwrap_or(manifest.id.as_str());
        let detail = match channel_id {
            "slack_channel" => inspect_slack_channel_secret(&secret),
            "teams_channel" => inspect_teams_channel_secret(&secret),
            "whatsapp_channel" => inspect_whatsapp_channel_secret(&secret),
            other => Err(anyhow!("Unsupported legacy channel binding '{}'", other)),
        }?;
        Ok(Some(ExtensionPackInvokeResult {
            ok: true,
            status: "ok".to_string(),
            pack_id: manifest.id.clone(),
            feature_id: "health.test".to_string(),
            connection_id: Some(connection.id.clone()),
            message: Some("Connection is ready.".to_string()),
            data: Some(detail),
            error: None,
        }))
    }

    async fn test_generic_pack_connection(
        &mut self,
        manifest: &ExtensionPackManifest,
        connection: &ExtensionPackConnection,
        mcp_registry: Option<
            std::sync::Arc<tokio::sync::RwLock<crate::mcp::registry::McpRegistry>>,
        >,
        plugin_registry: Option<
            std::sync::Arc<tokio::sync::RwLock<crate::plugins::registry::PluginRegistry>>,
        >,
    ) -> Result<Option<ExtensionPackInvokeResult>> {
        let Some(probe) = resolve_generic_health_probe(manifest) else {
            return Ok(None);
        };
        let invocation = self
            .invoke_feature(
                ExtensionPackInvokeRequest {
                    pack_id: Some(manifest.id.clone()),
                    connection_id: Some(connection.id.clone()),
                    feature_id: probe.feature_id.clone(),
                    arguments: probe.arguments.clone(),
                },
                mcp_registry,
                plugin_registry,
            )
            .await;
        let result = match invocation {
            Ok(probe_result) => {
                let probe_status = probe_result.status.clone();
                let probe_message_detail = probe_result.message.clone();
                let probe_error = probe_result.error.clone();
                let probe_data = probe_result.data.clone();
                let probe_message = if probe_result.ok {
                    format!(
                        "Connection is ready. Verified via {} probe '{}'.",
                        probe.source, probe.feature_id
                    )
                } else {
                    probe_message_detail.clone().unwrap_or_else(|| {
                        format!(
                            "Connection probe '{}' returned status '{}'.",
                            probe.feature_id, probe_status
                        )
                    })
                };
                ExtensionPackInvokeResult {
                    ok: probe_result.ok,
                    status: probe_status.clone(),
                    pack_id: manifest.id.clone(),
                    feature_id: "health.test".to_string(),
                    connection_id: Some(connection.id.clone()),
                    message: Some(probe_message),
                    data: Some(serde_json::json!({
                        "probe_feature_id": probe.feature_id,
                        "probe_source": probe.source,
                        "probe_arguments": probe.arguments,
                        "probe_result": {
                            "status": probe_status,
                            "message": probe_message_detail,
                            "error": probe_error,
                            "data": probe_data,
                        }
                    })),
                    error: probe_result.error,
                }
            }
            Err(error) => ExtensionPackInvokeResult {
                ok: false,
                status: "health_probe_failed".to_string(),
                pack_id: manifest.id.clone(),
                feature_id: "health.test".to_string(),
                connection_id: Some(connection.id.clone()),
                message: Some(format!(
                    "Connection probe '{}' failed: {}",
                    probe.feature_id, error
                )),
                data: Some(serde_json::json!({
                    "probe_feature_id": probe.feature_id,
                    "probe_source": probe.source,
                    "probe_arguments": probe.arguments,
                })),
                error: Some("health_probe_failed".to_string()),
            },
        };
        Ok(Some(result))
    }

    async fn persist_connection_test_result(
        &mut self,
        connection_id: &str,
        result: &ExtensionPackInvokeResult,
    ) -> Result<()> {
        if let Some(record) = self.connections.get_mut(connection_id) {
            record.last_tested_at = Some(now_rfc3339());
            record.last_error = if result.ok {
                None
            } else {
                result.message.clone().or_else(|| result.error.clone())
            };
            record.updated_at = now_rfc3339();
        }
        self.persist_connections().await
    }

    async fn invoke_http_binding(
        &self,
        manifest: &ExtensionPackManifest,
        binding: &ExtensionPackBinding,
        arguments: &Value,
        connection: Option<&ExtensionPackConnection>,
        secret: Option<&Value>,
    ) -> Result<Value> {
        let method = binding
            .config
            .get("method")
            .and_then(|value| value.as_str())
            .map(str::trim)
            .filter(|value| !value.is_empty())
            .unwrap_or("POST")
            .parse::<reqwest::Method>()
            .context("invalid http binding method")?;
        let raw_url = binding
            .config
            .get("url")
            .and_then(|value| value.as_str())
            .map(str::trim)
            .filter(|value| !value.is_empty())
            .ok_or_else(|| anyhow::anyhow!("http binding requires url"))?;
        if template_contains_secret(raw_url) {
            anyhow::bail!("http binding URLs cannot contain secret templates");
        }
        let mut url =
            reqwest::Url::parse(&render_template(raw_url, arguments, connection, secret)?)
                .context("invalid http binding url")?;
        let query_value = render_value_templates(
            binding.config.get("query").unwrap_or(&Value::Null),
            arguments,
            connection,
            secret,
        )?;
        if let Some(query) = query_value.as_object() {
            if value_contains_secret_template(binding.config.get("query").unwrap_or(&Value::Null)) {
                anyhow::bail!("http binding query parameters cannot contain secret templates");
            }
            for (key, value) in query {
                if let Some(text) = scalar_to_string(value) {
                    url.query_pairs_mut().append_pair(key, &text);
                }
            }
        }
        if let Some(profile_id) = connection.and_then(|value| value.auth_profile_id.as_deref()) {
            let auth_query = self
                .resolve_auth_profile_headers(profile_id, &manifest.auth.exports.query)
                .await?;
            for (key, value) in auth_query {
                url.query_pairs_mut().append_pair(&key, &value);
            }
        }
        crate::core::net::validate_external_https_url(url.as_str()).await?;
        let mut request = self.http_client.request(method.clone(), url);
        if let Some(headers) = binding
            .config
            .get("headers")
            .and_then(|value| value.as_object())
        {
            for (key, value) in headers {
                if let Some(text) = scalar_to_string(&render_value_templates(
                    value, arguments, connection, secret,
                )?) {
                    request = request.header(key, text);
                }
            }
        }
        if let Some(profile_id) = connection.and_then(|value| value.auth_profile_id.as_deref()) {
            let auth_headers = self
                .resolve_auth_profile_headers(profile_id, &manifest.auth.exports.headers)
                .await?;
            for (key, value) in auth_headers {
                request = request.header(key, value);
            }
        }
        if let Some(auth) = binding
            .config
            .get("auth")
            .and_then(|value| value.as_object())
        {
            let auth_type = auth
                .get("type")
                .and_then(|value| value.as_str())
                .map(str::trim)
                .unwrap_or("");
            match auth_type {
                "bearer" => {
                    let secret_path = auth
                        .get("secret_path")
                        .and_then(|value| value.as_str())
                        .unwrap_or("api_key");
                    validate_secret_path(secret_path)?;
                    if let Some(token) = secret
                        .and_then(|value| select_json_path(value, secret_path))
                        .and_then(scalar_to_string)
                    {
                        request = request.bearer_auth(token);
                    }
                }
                "header" => {
                    if let Some(name) = auth.get("name").and_then(|value| value.as_str()) {
                        let secret_path = auth
                            .get("secret_path")
                            .and_then(|value| value.as_str())
                            .unwrap_or("api_key");
                        validate_secret_path(secret_path)?;
                        if let Some(value) = secret
                            .and_then(|secret_value| select_json_path(secret_value, secret_path))
                            .and_then(scalar_to_string)
                        {
                            request = request.header(name, value);
                        }
                    }
                }
                "query" => {
                    if let Some(name) = auth.get("name").and_then(|value| value.as_str()) {
                        let secret_path = auth
                            .get("secret_path")
                            .and_then(|value| value.as_str())
                            .unwrap_or("api_key");
                        validate_secret_path(secret_path)?;
                        if let Some(value) = secret
                            .and_then(|secret_value| select_json_path(secret_value, secret_path))
                            .and_then(scalar_to_string)
                        {
                            request = request.query(&[(name, value)]);
                        }
                    }
                }
                "basic" => {
                    let username = secret
                        .and_then(|secret_value| select_json_path(secret_value, "username"))
                        .and_then(scalar_to_string)
                        .unwrap_or_default();
                    let password = secret
                        .and_then(|secret_value| select_json_path(secret_value, "password"))
                        .and_then(scalar_to_string)
                        .unwrap_or_default();
                    request = request.basic_auth(username, Some(password));
                }
                _ => {}
            }
        }
        if method != reqwest::Method::GET && method != reqwest::Method::DELETE {
            let body = render_value_templates(
                binding.config.get("body").unwrap_or(arguments),
                arguments,
                connection,
                secret,
            )?;
            if !body.is_null() {
                request = request.json(&body);
            }
        }
        let response = request.send().await?;
        let response = response.error_for_status()?;
        let content_type = response
            .headers()
            .get(reqwest::header::CONTENT_TYPE)
            .and_then(|value| value.to_str().ok())
            .unwrap_or("")
            .to_ascii_lowercase();
        let mut payload = if content_type.contains("json") {
            response.json::<Value>().await.unwrap_or(Value::Null)
        } else {
            Value::String(response.text().await.unwrap_or_default())
        };
        if let Some(path) = binding
            .config
            .get("result_path")
            .and_then(|value| value.as_str())
            .map(str::trim)
            .filter(|value| !value.is_empty())
        {
            payload = select_json_path(&payload, path)
                .cloned()
                .unwrap_or(Value::Null);
        }
        self.mark_connection_auth_used(connection).await;
        Ok(payload)
    }

    async fn invoke_local_cli_binding(
        &self,
        manifest: &ExtensionPackManifest,
        binding: &ExtensionPackBinding,
        arguments: &Value,
        connection: Option<&ExtensionPackConnection>,
        secret: Option<&Value>,
    ) -> Result<Value> {
        let program = binding
            .config
            .get("program")
            .and_then(|value| value.as_str())
            .map(str::trim)
            .filter(|value| !value.is_empty())
            .or_else(|| {
                runtime_installer(manifest)
                    .map(|installer| installer.executable.as_str())
                    .filter(|value| !value.trim().is_empty())
            })
            .ok_or_else(|| {
                anyhow!("local_cli binding requires program or runtime.installer.executable")
            })?;
        let allowed_program = runtime_installer(manifest)
            .map(|installer| installer.executable.trim())
            .filter(|value| !value.is_empty());
        if allowed_program != Some(program) {
            anyhow::bail!(
                "local_cli binding for pack '{}' may only run its declared runtime executable",
                manifest.id
            );
        }
        if !self
            .installed
            .get(&manifest.id)
            .is_some_and(|pack| matches!(pack.trust_level, ExtensionPackTrustLevel::Trusted))
        {
            anyhow::bail!(
                "local_cli bindings are blocked for unverified extension pack '{}'",
                manifest.id
            );
        }
        let args = binding
            .config
            .get("args")
            .map(|value| render_value_templates(value, arguments, connection, secret))
            .transpose()?
            .and_then(|value| value.as_array().cloned())
            .unwrap_or_default()
            .into_iter()
            .filter_map(|item| scalar_to_string(&item))
            .collect::<Vec<_>>();
        let runtime_dir = self.pack_runtime_dir(&manifest.id);
        std::fs::create_dir_all(&runtime_dir).with_context(|| {
            format!(
                "Failed to create extension-pack runtime directory '{}'",
                runtime_dir.display()
            )
        })?;
        let mut child = tokio::process::Command::new(program);
        child.env_clear();
        child.current_dir(runtime_dir);
        child.args(&args);
        if let Some(env) = binding
            .config
            .get("env")
            .and_then(|value| value.as_object())
        {
            for (key, value) in env {
                if let Some(text) = scalar_to_string(&render_value_templates(
                    value, arguments, connection, secret,
                )?) {
                    child.env(key, text);
                }
            }
        }
        if let Some(profile_id) = connection.and_then(|value| value.auth_profile_id.as_deref()) {
            let auth_env = self
                .resolve_auth_profile_headers(profile_id, &manifest.auth.exports.env)
                .await?;
            for (key, value) in auth_env {
                child.env(key, value);
            }
        }
        child.stdin(std::process::Stdio::piped());
        child.stdout(std::process::Stdio::piped());
        child.stderr(std::process::Stdio::piped());
        let mut child = child
            .spawn()
            .with_context(|| format!("Failed to launch local CLI '{}'", program))?;
        if let Some(stdin) = binding
            .config
            .get("stdin")
            .map(|value| render_value_templates(value, arguments, connection, secret))
            .transpose()?
            .and_then(|value| scalar_to_string(&value))
        {
            if let Some(mut writer) = child.stdin.take() {
                use tokio::io::AsyncWriteExt;
                writer.write_all(stdin.as_bytes()).await?;
            }
        }
        let output = tokio::time::timeout(
            std::time::Duration::from_secs(
                binding
                    .config
                    .get("timeout_secs")
                    .and_then(|value| value.as_u64())
                    .unwrap_or(120),
            ),
            child.wait_with_output(),
        )
        .await
        .map_err(|_| anyhow!("Local CLI '{}' timed out", program))??;
        let stdout = String::from_utf8_lossy(&output.stdout).trim().to_string();
        let stderr = String::from_utf8_lossy(&output.stderr).trim().to_string();
        let combined = [stdout.as_str(), stderr.as_str()]
            .into_iter()
            .filter(|value| !value.is_empty())
            .collect::<Vec<_>>()
            .join("\n");
        if !output.status.success() {
            anyhow::bail!(
                "Local CLI '{}' failed with status {}. {}",
                program,
                output
                    .status
                    .code()
                    .map(|code| code.to_string())
                    .unwrap_or_else(|| "unknown".to_string()),
                if combined.is_empty() {
                    "(no output)".to_string()
                } else {
                    combined
                }
            );
        }
        self.mark_connection_auth_used(connection).await;
        Ok(parse_action_payload(if stdout.is_empty() {
            &stderr
        } else {
            &stdout
        }))
    }
}

fn feature_summaries(manifest: &ExtensionPackManifest) -> Vec<PackFeatureSummary> {
    manifest
        .features
        .iter()
        .map(|feature| PackFeatureSummary {
            id: feature.id.clone(),
            kind: feature.kind.clone(),
            title: feature.title.clone(),
            description: feature.description.clone(),
            read_only: feature.read_only,
            experimental: feature.experimental,
            binding_kind: binding_kind(feature),
        })
        .collect()
}

async fn invoke_legacy_action_binding(
    config_dir: &Path,
    binding: &ExtensionPackBinding,
    arguments: &Value,
) -> Result<Value> {
    let action_name = binding
        .config
        .get("action_name")
        .and_then(|value| value.as_str())
        .map(str::trim)
        .filter(|value| !value.is_empty())
        .ok_or_else(|| anyhow::anyhow!("legacy_action binding requires action_name"))?;
    let output = match action_name {
        "gmail_scan" => crate::actions::gmail::gmail_scan(config_dir, arguments).await?,
        "gmail_reply" => crate::actions::gmail::gmail_reply(config_dir, arguments).await?,
        "calendar_list" => crate::actions::calendar::calendar_list(config_dir, arguments).await?,
        "calendar_create" => {
            crate::actions::calendar::calendar_create(config_dir, arguments).await?
        }
        "google_drive_search" => {
            crate::actions::google_workspace::drive_search(config_dir, arguments).await?
        }
        "google_docs_read" => {
            crate::actions::google_workspace::docs_read(config_dir, arguments).await?
        }
        "google_chat_list_spaces" => {
            crate::actions::google_workspace::chat_list_spaces(config_dir, arguments).await?
        }
        other => anyhow::bail!("Unsupported legacy action binding '{}'", other),
    };
    Ok(parse_action_payload(&output))
}

fn parse_action_payload(text: &str) -> Value {
    let trimmed = text.trim();
    if trimmed.is_empty() {
        Value::Null
    } else if let Ok(value) = serde_json::from_str::<Value>(trimmed) {
        value
    } else {
        Value::String(trimmed.to_string())
    }
}

fn sanitize_event_value(value: &Value) -> Value {
    if let Ok(encoded) = serde_json::to_string(value) {
        if encoded.chars().count() > MAX_EVENT_PAYLOAD_CHARS {
            return Value::String(
                encoded
                    .chars()
                    .take(MAX_EVENT_PAYLOAD_CHARS)
                    .collect::<String>(),
            );
        }
    }
    crate::security::redact_json_secrets(value)
}

fn validate_zip_entry_name(name: &str) -> Result<()> {
    let normalized = name.replace('\\', "/");
    if normalized.starts_with('/')
        || normalized.starts_with("~/")
        || normalized.contains("../")
        || normalized.contains("/..")
        || normalized.contains(':')
    {
        anyhow::bail!(
            "Extension-pack bundle contains an unsafe entry name '{}'",
            name
        );
    }
    Ok(())
}

fn required_secrets_for_auth_mode(mode: ExtensionPackAuthMode) -> Vec<String> {
    match mode {
        ExtensionPackAuthMode::ApiKey => vec!["api_key".to_string()],
        ExtensionPackAuthMode::Basic => vec!["username".to_string(), "password".to_string()],
        _ => Vec::new(),
    }
}

#[derive(Debug, Clone)]
struct ImportedAuthContract {
    mode: ExtensionPackAuthMode,
    required_secrets: Vec<String>,
    binding: Option<Value>,
    metadata: Map<String, Value>,
}

fn imported_auth_contract(
    requested_mode: ExtensionPackAuthMode,
    preview: &crate::custom_apis::CustomApiPreview,
) -> ImportedAuthContract {
    let secret_path = imported_secret_path(requested_mode, preview.auth_mode);
    let mut metadata = Map::new();
    metadata.insert(
        "import_source".to_string(),
        Value::String(preview.source_kind.clone()),
    );
    metadata.insert(
        "import_notes".to_string(),
        serde_json::to_value(&preview.notes).unwrap_or(Value::Null),
    );
    if let Some(header) = preview
        .auth_header
        .as_ref()
        .filter(|value| !value.trim().is_empty())
    {
        metadata.insert("auth_header".to_string(), Value::String(header.clone()));
    }
    if let Some(name) = preview
        .auth_name
        .as_ref()
        .filter(|value| !value.trim().is_empty())
    {
        metadata.insert("auth_name".to_string(), Value::String(name.clone()));
    }
    if let Some(secret_path) = secret_path {
        metadata.insert(
            "secret_field".to_string(),
            Value::String(secret_path.to_string()),
        );
    }
    if let Some(auth_binding) = auth_binding_from_preview(preview, secret_path) {
        if let Some(auth_type) = auth_binding.get("type").and_then(|value| value.as_str()) {
            metadata.insert(
                "auth_binding_type".to_string(),
                Value::String(auth_type.to_string()),
            );
        }
    }
    ImportedAuthContract {
        mode: imported_auth_mode(requested_mode, preview.auth_mode),
        required_secrets: required_secrets_for_imported_auth(requested_mode, preview.auth_mode),
        binding: auth_binding_from_preview(preview, secret_path),
        metadata,
    }
}

fn imported_auth_mode(
    requested_mode: ExtensionPackAuthMode,
    preview_mode: crate::custom_apis::CustomApiAuthMode,
) -> ExtensionPackAuthMode {
    use crate::custom_apis::CustomApiAuthMode;
    match preview_mode {
        CustomApiAuthMode::Basic => ExtensionPackAuthMode::Basic,
        CustomApiAuthMode::Bearer
        | CustomApiAuthMode::ApiKeyHeader
        | CustomApiAuthMode::ApiKeyQuery
        | CustomApiAuthMode::OAuth2 => ExtensionPackAuthMode::ApiKey,
        CustomApiAuthMode::None => requested_mode,
    }
}

fn required_secrets_for_imported_auth(
    requested_mode: ExtensionPackAuthMode,
    preview_mode: crate::custom_apis::CustomApiAuthMode,
) -> Vec<String> {
    use crate::custom_apis::CustomApiAuthMode;
    match preview_mode {
        CustomApiAuthMode::None => required_secrets_for_auth_mode(requested_mode),
        CustomApiAuthMode::Basic => vec!["username".to_string(), "password".to_string()],
        CustomApiAuthMode::ApiKeyHeader | CustomApiAuthMode::ApiKeyQuery => {
            vec!["api_key".to_string()]
        }
        CustomApiAuthMode::Bearer | CustomApiAuthMode::OAuth2 => {
            vec![
                imported_secret_path(requested_mode, preview_mode)
                    .unwrap_or("access_token")
                    .to_string(),
            ]
        }
    }
}

fn imported_secret_path(
    requested_mode: ExtensionPackAuthMode,
    preview_mode: crate::custom_apis::CustomApiAuthMode,
) -> Option<&'static str> {
    use crate::custom_apis::CustomApiAuthMode;
    match preview_mode {
        CustomApiAuthMode::None => required_secrets_for_auth_mode(requested_mode)
            .into_iter()
            .next()
            .map(|value| match value.as_str() {
                "username" => "username",
                "password" => "password",
                "access_token" => "access_token",
                _ => "api_key",
            }),
        CustomApiAuthMode::Basic => None,
        CustomApiAuthMode::ApiKeyHeader | CustomApiAuthMode::ApiKeyQuery => Some("api_key"),
        CustomApiAuthMode::Bearer | CustomApiAuthMode::OAuth2 => {
            if matches!(requested_mode, ExtensionPackAuthMode::ApiKey) {
                Some("api_key")
            } else {
                Some("access_token")
            }
        }
    }
}

fn auth_binding_from_preview(
    preview: &crate::custom_apis::CustomApiPreview,
    secret_path: Option<&str>,
) -> Option<Value> {
    use crate::custom_apis::CustomApiAuthMode;
    match preview.auth_mode {
        CustomApiAuthMode::None => None,
        CustomApiAuthMode::Bearer | CustomApiAuthMode::OAuth2 => Some(serde_json::json!({
            "type": "bearer",
            "secret_path": secret_path.unwrap_or("access_token")
        })),
        CustomApiAuthMode::ApiKeyHeader => Some(serde_json::json!({
            "type": "header",
            "name": preview.auth_header.clone().or(preview.auth_name.clone()).unwrap_or_else(|| "x-api-key".to_string()),
            "secret_path": secret_path.unwrap_or("api_key")
        })),
        CustomApiAuthMode::ApiKeyQuery => Some(serde_json::json!({
            "type": "query",
            "name": preview.auth_name.clone().unwrap_or_else(|| "api_key".to_string()),
            "secret_path": secret_path.unwrap_or("api_key")
        })),
        CustomApiAuthMode::Basic => Some(serde_json::json!({
            "type": "basic"
        })),
    }
}

fn auth_profile_material_for_secret_backed_pack(
    manifest: &ExtensionPackManifest,
    secret: &Value,
) -> Result<crate::core::auth_profiles::AuthProfileMaterial> {
    use crate::core::auth_profiles::AuthProfileMaterial;

    let secret_field = manifest
        .auth
        .metadata
        .get("secret_field")
        .and_then(|value| value.as_str())
        .filter(|value| !value.trim().is_empty())
        .unwrap_or("api_key");
    let auth_binding_type = manifest
        .auth
        .metadata
        .get("auth_binding_type")
        .and_then(|value| value.as_str())
        .filter(|value| !value.trim().is_empty())
        .unwrap_or("");

    let value = select_json_path(secret, secret_field)
        .and_then(scalar_to_string)
        .filter(|value| !value.trim().is_empty())
        .ok_or_else(|| anyhow!("API key connections require secret.{secret_field}"))?;

    if auth_binding_type.eq_ignore_ascii_case("bearer") || secret_field == "access_token" {
        return Ok(AuthProfileMaterial::Bearer {
            token: value,
            header: manifest
                .auth
                .metadata
                .get("auth_header")
                .and_then(|value| value.as_str())
                .map(str::to_string),
        });
    }

    Ok(AuthProfileMaterial::ApiKey {
        value,
        header: manifest
            .auth
            .metadata
            .get("auth_header")
            .and_then(|value| value.as_str())
            .map(str::to_string),
        query_name: manifest
            .auth
            .metadata
            .get("auth_name")
            .and_then(|value| value.as_str())
            .map(str::to_string),
    })
}

fn infer_feature_id_for_operation(
    pack_kind: &str,
    pack_id: &str,
    operation: &crate::custom_apis::CustomApiOperationDraft,
) -> String {
    let summary = format!(
        "{} {} {} {}",
        operation.id, operation.name, operation.method, operation.path
    )
    .to_ascii_lowercase();
    if pack_kind.eq_ignore_ascii_case(PACK_KIND_MESSAGING_CHANNEL) {
        if operation.method.eq_ignore_ascii_case("POST")
            && (summary.contains("message") || summary.contains("send"))
        {
            return "message.send".to_string();
        }
        if operation.method.eq_ignore_ascii_case("GET")
            && (summary.contains("thread")
                || summary.contains("conversation")
                || summary.contains("channel")
                || summary.contains("space"))
        {
            return "message.list_threads".to_string();
        }
        if summary.contains("webhook") || summary.contains("event") || summary.contains("receive") {
            return "message.receive".to_string();
        }
    }
    if summary.contains("mail") || summary.contains("gmail") {
        if operation.method.eq_ignore_ascii_case("POST") && summary.contains("send") {
            return "mail.send".to_string();
        }
        if operation.method.eq_ignore_ascii_case("GET")
            && (summary.contains("list") || summary.contains("search"))
        {
            return "mail.list".to_string();
        }
        if operation.method.eq_ignore_ascii_case("GET") {
            return "mail.get".to_string();
        }
    }
    if summary.contains("calendar") || summary.contains("event") {
        if operation.method.eq_ignore_ascii_case("POST") {
            return "calendar.create_event".to_string();
        }
        if operation.method.eq_ignore_ascii_case("GET") {
            return "calendar.list_events".to_string();
        }
    }
    if summary.contains("file") || summary.contains("document") || summary.contains("drive") {
        if operation.method.eq_ignore_ascii_case("GET")
            && (summary.contains("search") || summary.contains("query"))
        {
            return "files.search".to_string();
        }
        if operation.method.eq_ignore_ascii_case("GET")
            && (summary.contains("read")
                || summary.contains("download")
                || summary.contains("content"))
        {
            return "files.read".to_string();
        }
        if operation.method.eq_ignore_ascii_case("GET") {
            return "files.list".to_string();
        }
    }
    if summary.contains("chat") || summary.contains("space") || summary.contains("channel") {
        if operation.method.eq_ignore_ascii_case("POST") {
            return "chat.send".to_string();
        }
        if operation.method.eq_ignore_ascii_case("GET") {
            return "chat.list_spaces".to_string();
        }
    }
    if summary.contains("contact") && operation.method.eq_ignore_ascii_case("GET") {
        return "contacts.search".to_string();
    }
    format!(
        "{}.{}",
        pack_id,
        sanitize_pack_id(&operation.id).replace('-', "_")
    )
}

fn operation_input_schema(operation: &crate::custom_apis::CustomApiOperationDraft) -> Value {
    let mut properties = serde_json::Map::new();
    let mut required = Vec::new();
    for parameter in &operation.parameters {
        let schema_type = parameter.schema_type.as_deref().unwrap_or(
            if matches!(
                parameter.location,
                crate::custom_apis::CustomApiParameterLocation::Body
            ) {
                "object"
            } else {
                "string"
            },
        );
        properties.insert(
            parameter.name.clone(),
            serde_json::json!({
                "type": schema_type,
                "description": parameter.description.clone().unwrap_or_default(),
            }),
        );
        if parameter.required && !operation_parameter_has_default(operation, parameter) {
            required.push(parameter.name.clone());
        }
    }
    serde_json::json!({
        "type": "object",
        "properties": properties,
        "required": required,
    })
}

fn render_openapi_path_template(path: &str) -> String {
    let regex = Regex::new(r"\{([a-zA-Z0-9_\-]+)\}").expect("openapi path regex");
    regex
        .replace_all(path, |captures: &regex::Captures<'_>| {
            let name = captures.get(1).map(|value| value.as_str()).unwrap_or("");
            format!("{{{{arg.{}}}}}", name)
        })
        .to_string()
}

fn http_binding_from_operation(
    base_url: &str,
    operation: &crate::custom_apis::CustomApiOperationDraft,
    auth: Option<Value>,
) -> Value {
    let mut query = serde_json::Map::new();
    let mut headers = serde_json::Map::new();
    for (key, value) in &operation.default_query {
        query.insert(key.clone(), Value::String(value.clone()));
    }
    for (key, value) in &operation.default_headers {
        headers.insert(key.clone(), Value::String(value.clone()));
    }
    for parameter in &operation.parameters {
        let template = Value::String(format!("{{{{arg.{}}}}}", parameter.name));
        match parameter.location {
            crate::custom_apis::CustomApiParameterLocation::Query => {
                query.entry(parameter.name.clone()).or_insert(template);
            }
            crate::custom_apis::CustomApiParameterLocation::Header => {
                headers.entry(parameter.name.clone()).or_insert(template);
            }
            _ => {}
        }
    }
    let url = format!(
        "{}{}",
        base_url.trim_end_matches('/'),
        render_openapi_path_template(&operation.path)
    );
    let mut config = serde_json::json!({
        "method": operation.method,
        "url": url,
    });
    if let Some(map) = config.as_object_mut() {
        if !query.is_empty() {
            map.insert("query".to_string(), Value::Object(query));
        }
        if !headers.is_empty() {
            map.insert("headers".to_string(), Value::Object(headers));
        }
        if let Some(auth) = auth {
            map.insert("auth".to_string(), auth);
        }
        if operation.body_required {
            map.insert(
                "body".to_string(),
                Value::String("{{arg.body}}".to_string()),
            );
        }
    }
    config
}

fn operation_parameter_has_default(
    operation: &crate::custom_apis::CustomApiOperationDraft,
    parameter: &crate::custom_apis::CustomApiParameter,
) -> bool {
    match parameter.location {
        crate::custom_apis::CustomApiParameterLocation::Query => {
            operation.default_query.contains_key(&parameter.name)
        }
        crate::custom_apis::CustomApiParameterLocation::Header => {
            operation.default_headers.contains_key(&parameter.name)
        }
        _ => false,
    }
}

fn imported_operation_supports_health_probe(
    operation: &crate::custom_apis::CustomApiOperationDraft,
) -> bool {
    operation.enabled
        && operation.read_only
        && !operation.body_required
        && operation.parameters.iter().all(|parameter| {
            !parameter.required || operation_parameter_has_default(operation, parameter)
        })
}

fn value_at_paths<'a>(value: &'a Value, paths: &[&str]) -> Option<&'a Value> {
    paths.iter().find_map(|path| select_json_path(value, path))
}

fn string_at_paths(value: &Value, paths: &[&str]) -> Option<String> {
    value_at_paths(value, paths).and_then(scalar_to_string)
}

fn string_from_arguments_or_secret(
    arguments: &Value,
    secret: &Value,
    argument_paths: &[&str],
    secret_paths: &[&str],
) -> Option<String> {
    string_at_paths(arguments, argument_paths)
        .or_else(|| string_at_paths(secret, secret_paths))
        .map(|value| value.trim().to_string())
        .filter(|value| !value.is_empty())
}

fn string_list_at_paths(value: &Value, paths: &[&str]) -> Option<Vec<String>> {
    let raw = value_at_paths(value, paths)?;
    match raw {
        Value::Array(items) => {
            let values = items
                .iter()
                .filter_map(scalar_to_string)
                .map(|value| value.trim().to_string())
                .filter(|value| !value.is_empty())
                .collect::<Vec<_>>();
            if values.is_empty() {
                None
            } else {
                Some(values)
            }
        }
        Value::String(text) => {
            let values = text
                .split(',')
                .map(str::trim)
                .filter(|value| !value.is_empty())
                .map(|value| value.to_string())
                .collect::<Vec<_>>();
            if values.is_empty() {
                None
            } else {
                Some(values)
            }
        }
        _ => None,
    }
}

fn required_message_text(arguments: &Value) -> Result<String> {
    string_at_paths(arguments, &["text", "message.text", "body.text", "message"])
        .filter(|value| !value.is_empty())
        .ok_or_else(|| anyhow!("message text is required"))
}

fn parse_teams_delivery_mode(value: Option<String>) -> crate::channels::teams::TeamsDeliveryMode {
    match value
        .as_deref()
        .map(str::trim)
        .unwrap_or("")
        .to_ascii_lowercase()
        .as_str()
    {
        "bot_framework" | "bot" => crate::channels::teams::TeamsDeliveryMode::BotFramework,
        "graph" => crate::channels::teams::TeamsDeliveryMode::Graph,
        _ => crate::channels::teams::TeamsDeliveryMode::Auto,
    }
}

fn parse_whatsapp_mode(value: Option<String>) -> crate::channels::whatsapp::WhatsAppMode {
    match value
        .as_deref()
        .map(str::trim)
        .unwrap_or("")
        .to_ascii_lowercase()
        .as_str()
    {
        "cloud_api" | "cloud" | "meta" => crate::channels::whatsapp::WhatsAppMode::CloudApi,
        _ => crate::channels::whatsapp::WhatsAppMode::Baileys,
    }
}

pub(crate) fn slack_config_from_secret(
    arguments: &Value,
    secret: &Value,
) -> Result<crate::channels::slack::SlackChannelConfig> {
    Ok(crate::channels::slack::SlackChannelConfig {
        bot_token: string_from_arguments_or_secret(
            arguments,
            secret,
            &["bot_token"],
            &["bot_token", "access_token", "api_key"],
        )
        .unwrap_or_default(),
        signing_secret: string_from_arguments_or_secret(
            arguments,
            secret,
            &["signing_secret"],
            &["signing_secret"],
        )
        .unwrap_or_default(),
        default_channel_id: string_from_arguments_or_secret(
            arguments,
            secret,
            &["channel_id", "destination.channel_id"],
            &["default_channel_id", "channel_id"],
        )
        .unwrap_or_default(),
        default_thread_ts: string_from_arguments_or_secret(
            arguments,
            secret,
            &["thread_ts", "destination.thread_ts"],
            &["default_thread_ts", "thread_ts"],
        ),
        api_base_url: string_from_arguments_or_secret(
            arguments,
            secret,
            &["api_base_url"],
            &["api_base_url"],
        )
        .unwrap_or_else(|| "https://slack.com/api".to_string()),
        workspace_id: string_from_arguments_or_secret(
            arguments,
            secret,
            &["workspace_id"],
            &["workspace_id"],
        ),
        workspace_name: string_from_arguments_or_secret(
            arguments,
            secret,
            &["workspace_name"],
            &["workspace_name"],
        ),
    })
}

pub(crate) fn teams_config_from_secret(
    arguments: &Value,
    secret: &Value,
) -> Result<crate::channels::teams::TeamsTransportConfig> {
    Ok(crate::channels::teams::TeamsTransportConfig {
        service_url: string_from_arguments_or_secret(
            arguments,
            secret,
            &["service_url"],
            &["service_url"],
        )
        .unwrap_or_default(),
        access_token: string_from_arguments_or_secret(
            arguments,
            secret,
            &["access_token"],
            &["access_token", "api_key"],
        )
        .unwrap_or_default(),
        bot_app_id: string_from_arguments_or_secret(
            arguments,
            secret,
            &["bot_app_id"],
            &["bot_app_id"],
        ),
        bot_name: string_from_arguments_or_secret(arguments, secret, &["bot_name"], &["bot_name"]),
        tenant_id: string_from_arguments_or_secret(
            arguments,
            secret,
            &["tenant_id"],
            &["tenant_id"],
        ),
        team_id: string_from_arguments_or_secret(arguments, secret, &["team_id"], &["team_id"]),
        channel_id: string_from_arguments_or_secret(
            arguments,
            secret,
            &["channel_id"],
            &["channel_id"],
        ),
        chat_id: string_from_arguments_or_secret(arguments, secret, &["chat_id"], &["chat_id"]),
        graph_base_url: string_from_arguments_or_secret(
            arguments,
            secret,
            &["graph_base_url"],
            &["graph_base_url"],
        ),
        delivery_mode: parse_teams_delivery_mode(string_from_arguments_or_secret(
            arguments,
            secret,
            &["delivery_mode"],
            &["delivery_mode"],
        )),
        timeout_secs: string_from_arguments_or_secret(
            arguments,
            secret,
            &["timeout_secs"],
            &["timeout_secs"],
        )
        .and_then(|value| value.parse::<u64>().ok())
        .unwrap_or(15),
        user_agent: string_from_arguments_or_secret(
            arguments,
            secret,
            &["user_agent"],
            &["user_agent"],
        ),
    })
}

pub(crate) fn whatsapp_config_from_secret(
    arguments: &Value,
    secret: &Value,
) -> Result<crate::channels::whatsapp::WhatsAppChannelConfig> {
    Ok(crate::channels::whatsapp::WhatsAppChannelConfig {
        mode: parse_whatsapp_mode(string_from_arguments_or_secret(
            arguments,
            secret,
            &["mode"],
            &["mode"],
        )),
        access_token: string_from_arguments_or_secret(
            arguments,
            secret,
            &["access_token"],
            &["access_token", "api_key"],
        )
        .unwrap_or_default(),
        phone_number_id: string_from_arguments_or_secret(
            arguments,
            secret,
            &["phone_number_id"],
            &["phone_number_id"],
        )
        .unwrap_or_default(),
        app_secret: string_from_arguments_or_secret(
            arguments,
            secret,
            &["app_secret"],
            &["app_secret"],
        )
        .unwrap_or_default(),
        verify_token: string_from_arguments_or_secret(
            arguments,
            secret,
            &["verify_token"],
            &["verify_token"],
        )
        .unwrap_or_default(),
        bridge_runtime: None,
        bridge_url: string_from_arguments_or_secret(
            arguments,
            secret,
            &["bridge_url"],
            &["bridge_url"],
        )
        .unwrap_or_else(|| crate::channels::whatsapp::EMBEDDED_BRIDGE_URL.to_string()),
        bridge_token: string_from_arguments_or_secret(
            arguments,
            secret,
            &["bridge_token"],
            &["bridge_token"],
        )
        .unwrap_or_default(),
        allowed_numbers: string_list_at_paths(arguments, &["allowed_numbers"])
            .or_else(|| string_list_at_paths(secret, &["allowed_numbers"]))
            .unwrap_or_default(),
        dm_policy: string_from_arguments_or_secret(
            arguments,
            secret,
            &["dm_policy"],
            &["dm_policy"],
        )
        .unwrap_or_else(|| "pairing".to_string()),
    })
}

fn inspect_slack_channel_secret(secret: &Value) -> Result<Value> {
    let config = slack_config_from_secret(&Value::Null, secret)?;
    let destination = crate::channels::slack::default_destination(&config)?;
    Ok(serde_json::json!({
        "channel": "slack",
        "ready": true,
        "default_destination": destination,
        "workspace_id": config.workspace_id,
        "workspace_name": config.workspace_name,
    }))
}

fn inspect_teams_channel_secret(secret: &Value) -> Result<Value> {
    let config = teams_config_from_secret(&Value::Null, secret)?;
    crate::channels::teams::validate_transport_config(&config)?;
    let destination = crate::channels::teams::default_destination_for_config(&config)?
        .ok_or_else(|| anyhow!("Teams has no default chat_id or team/channel destination"))?;
    Ok(serde_json::json!({
        "channel": "teams",
        "ready": true,
        "default_destination": destination,
    }))
}

fn inspect_whatsapp_channel_secret(secret: &Value) -> Result<Value> {
    let config = whatsapp_config_from_secret(&Value::Null, secret)?;
    let recipient = crate::channels::whatsapp::configured_notification_recipient(&config);
    match config.mode {
        crate::channels::whatsapp::WhatsAppMode::CloudApi => {
            if config.access_token.trim().is_empty() {
                anyhow::bail!("WhatsApp Cloud API access_token is required");
            }
            if config.phone_number_id.trim().is_empty() {
                anyhow::bail!("WhatsApp Cloud API phone_number_id is required");
            }
        }
        crate::channels::whatsapp::WhatsAppMode::Baileys => {
            let _ = config.effective_bridge_url()?;
        }
    }
    Ok(serde_json::json!({
        "channel": "whatsapp",
        "ready": true,
        "mode": match config.mode {
            crate::channels::whatsapp::WhatsAppMode::Baileys => "baileys",
            crate::channels::whatsapp::WhatsAppMode::CloudApi => "cloud_api",
        },
        "recipient": recipient,
    }))
}

async fn invoke_slack_channel_operation(
    operation: &str,
    arguments: &Value,
    _connection: Option<&ExtensionPackConnection>,
    secret: &Value,
) -> Result<Value> {
    let config = slack_config_from_secret(arguments, secret)?;
    match operation {
        "send" => {
            let text = required_message_text(arguments)?;
            crate::channels::slack::send_message_with_config(&config, &text).await?;
            let destination = crate::channels::slack::default_destination(&config)?;
            Ok(serde_json::json!({
                "channel": "slack",
                "sent": true,
                "destination": destination,
            }))
        }
        "list_threads" => {
            let destination = crate::channels::slack::default_destination(&config)?;
            Ok(serde_json::json!({
                "channel": "slack",
                "items": [destination],
            }))
        }
        "receive_status" => {
            let mut detail = inspect_slack_channel_secret(secret)?;
            if let Value::Object(map) = &mut detail {
                map.insert(
                    "managed_by".to_string(),
                    Value::String("builtin_channel_runtime".to_string()),
                );
            }
            Ok(detail)
        }
        other => Err(anyhow!("Unsupported Slack channel operation '{}'", other)),
    }
}

async fn invoke_teams_channel_operation(
    operation: &str,
    arguments: &Value,
    _connection: Option<&ExtensionPackConnection>,
    secret: &Value,
) -> Result<Value> {
    let config = teams_config_from_secret(arguments, secret)?;
    match operation {
        "send" => {
            crate::channels::teams::validate_transport_config(&config)?;
            let destination = crate::channels::teams::default_destination_for_config(&config)?
                .ok_or_else(|| {
                    anyhow!("Teams has no default chat_id or team/channel destination")
                })?;
            let text = required_message_text(arguments)?;
            let response = crate::channels::teams::send_message_to_destination(
                &config,
                &destination,
                &crate::channels::teams::TeamsOutboundMessage {
                    conversation_id: destination.conversation_id.clone(),
                    text,
                    reply_to_id: destination.last_reply_to_id.clone(),
                    service_url: destination.service_url.clone(),
                    team_id: destination.team_id.clone(),
                    channel_id: destination.channel_id.clone(),
                    chat_id: destination.chat_id.clone(),
                    content_type: None,
                },
            )
            .await?;
            Ok(serde_json::json!({
                "channel": "teams",
                "sent": true,
                "destination": destination,
                "response": response,
            }))
        }
        "list_threads" => {
            crate::channels::teams::validate_transport_config(&config)?;
            let destination = crate::channels::teams::default_destination_for_config(&config)?
                .ok_or_else(|| {
                    anyhow!("Teams has no default chat_id or team/channel destination")
                })?;
            Ok(serde_json::json!({
                "channel": "teams",
                "items": [destination],
            }))
        }
        "receive_status" => {
            let mut detail = inspect_teams_channel_secret(secret)?;
            if let Value::Object(map) = &mut detail {
                map.insert(
                    "managed_by".to_string(),
                    Value::String("builtin_channel_runtime".to_string()),
                );
            }
            Ok(detail)
        }
        other => Err(anyhow!("Unsupported Teams channel operation '{}'", other)),
    }
}

async fn invoke_whatsapp_channel_operation(
    operation: &str,
    arguments: &Value,
    _connection: Option<&ExtensionPackConnection>,
    secret: &Value,
) -> Result<Value> {
    let config = whatsapp_config_from_secret(arguments, secret)?;
    match operation {
        "send" => {
            let text = required_message_text(arguments)?;
            let recipient = string_at_paths(arguments, &["recipient", "to", "phone_number"])
                .filter(|value| !value.is_empty())
                .or_else(|| crate::channels::whatsapp::configured_notification_recipient(&config))
                .ok_or_else(|| {
                    anyhow!(
                        "WhatsApp needs an explicit recipient or exactly one allowed_numbers entry"
                    )
                })?;
            crate::channels::whatsapp::send_message_to_recipient(
                &config,
                &recipient,
                crate::branding::PRODUCT_NAME,
                &text,
            )
            .await?;
            Ok(serde_json::json!({
                "channel": "whatsapp",
                "sent": true,
                "recipient": recipient,
                "mode": match config.mode {
                    crate::channels::whatsapp::WhatsAppMode::Baileys => "baileys",
                    crate::channels::whatsapp::WhatsAppMode::CloudApi => "cloud_api",
                }
            }))
        }
        "list_threads" => Ok(serde_json::json!({
            "channel": "whatsapp",
            "items": crate::channels::whatsapp::configured_notification_recipient(&config)
                .map(|value| vec![serde_json::json!({ "recipient": value })])
                .unwrap_or_default(),
        })),
        "receive_status" => {
            let mut detail = inspect_whatsapp_channel_secret(secret)?;
            if let Value::Object(map) = &mut detail {
                map.insert(
                    "managed_by".to_string(),
                    Value::String("builtin_channel_runtime".to_string()),
                );
            }
            Ok(detail)
        }
        other => Err(anyhow!(
            "Unsupported WhatsApp channel operation '{}'",
            other
        )),
    }
}

fn render_template(
    template: &str,
    arguments: &Value,
    connection: Option<&ExtensionPackConnection>,
    secret: Option<&Value>,
) -> Result<String> {
    let regex = Regex::new(r"\{\{\s*(arg|connection|secret)\.([a-zA-Z0-9_\-\.]+)\s*\}\}")
        .expect("extension pack template regex");
    validate_template_secret_paths(template)?;
    let connection_value = connection
        .map(serde_json::to_value)
        .transpose()?
        .unwrap_or(Value::Null);
    let rendered = regex.replace_all(template, |captures: &regex::Captures<'_>| {
        let scope = captures.get(1).map(|value| value.as_str()).unwrap_or("");
        let path = captures.get(2).map(|value| value.as_str()).unwrap_or("");
        let value = match scope {
            "arg" => select_json_path(arguments, path),
            "connection" => select_json_path(&connection_value, path),
            "secret" => secret.and_then(|value| select_json_path(value, path)),
            _ => None,
        };
        value.and_then(scalar_to_string).unwrap_or_default()
    });
    Ok(rendered.to_string())
}

fn render_value_templates(
    value: &Value,
    arguments: &Value,
    connection: Option<&ExtensionPackConnection>,
    secret: Option<&Value>,
) -> Result<Value> {
    match value {
        Value::Null => Ok(Value::Null),
        Value::Bool(_) | Value::Number(_) => Ok(value.clone()),
        Value::String(text) => {
            validate_template_secret_paths(text)?;
            let exact_regex =
                Regex::new(r"^\{\{\s*(arg|connection|secret)\.([a-zA-Z0-9_\-\.]+)\s*\}\}$")
                    .expect("extension pack exact template regex");
            if let Some(captures) = exact_regex.captures(text) {
                let scope = captures.get(1).map(|value| value.as_str()).unwrap_or("");
                let path = captures.get(2).map(|value| value.as_str()).unwrap_or("");
                let connection_value = connection
                    .map(serde_json::to_value)
                    .transpose()?
                    .unwrap_or(Value::Null);
                if let Some(value) = match scope {
                    "arg" => select_json_path(arguments, path),
                    "connection" => select_json_path(&connection_value, path),
                    "secret" => secret.and_then(|value| select_json_path(value, path)),
                    _ => None,
                } {
                    return Ok(value.clone());
                }
            }
            Ok(Value::String(render_template(
                text, arguments, connection, secret,
            )?))
        }
        Value::Array(items) => Ok(Value::Array(
            items
                .iter()
                .map(|item| render_value_templates(item, arguments, connection, secret))
                .collect::<Result<Vec<_>>>()?,
        )),
        Value::Object(map) => {
            let mut rendered = serde_json::Map::with_capacity(map.len());
            for (key, item) in map {
                rendered.insert(
                    key.clone(),
                    render_value_templates(item, arguments, connection, secret)?,
                );
            }
            Ok(Value::Object(rendered))
        }
    }
}

fn validate_template_secret_paths(template: &str) -> Result<()> {
    let regex = Regex::new(r"\{\{\s*secret\.([a-zA-Z0-9_\-\.]+)\s*\}\}")
        .expect("extension pack secret template regex");
    for captures in regex.captures_iter(template) {
        if let Some(path) = captures.get(1).map(|value| value.as_str()) {
            validate_secret_path(path)?;
        }
    }
    Ok(())
}

fn validate_secret_path(path: &str) -> Result<()> {
    let trimmed = path.trim();
    if trimmed.is_empty()
        || trimmed.starts_with('.')
        || trimmed.ends_with('.')
        || trimmed.contains("..")
        || trimmed.split('.').any(|segment| {
            segment.is_empty()
                || !segment
                    .chars()
                    .all(|ch| ch.is_ascii_alphanumeric() || matches!(ch, '_' | '-'))
        })
    {
        anyhow::bail!("Invalid secret path '{}'", path);
    }
    Ok(())
}

fn template_contains_secret(text: &str) -> bool {
    text.contains("{{") && text.contains("secret.")
}

fn value_contains_secret_template(value: &Value) -> bool {
    match value {
        Value::String(text) => template_contains_secret(text),
        Value::Array(items) => items.iter().any(value_contains_secret_template),
        Value::Object(map) => map.values().any(value_contains_secret_template),
        _ => false,
    }
}

fn select_json_path<'a>(value: &'a Value, path: &str) -> Option<&'a Value> {
    let mut current = value;
    for segment in path
        .split('.')
        .map(str::trim)
        .filter(|value| !value.is_empty())
    {
        current = match current {
            Value::Object(map) => map.get(segment)?,
            _ => return None,
        };
    }
    Some(current)
}

fn scalar_to_string(value: &Value) -> Option<String> {
    match value {
        Value::Null => None,
        Value::Bool(value) => Some(value.to_string()),
        Value::Number(value) => Some(value.to_string()),
        Value::String(value) => Some(value.clone()),
        other => serde_json::to_string(other).ok(),
    }
}

#[cfg(test)]
mod tests {
    use super::{
        ExtensionConnectionState, ExtensionPackAuthMode, ExtensionPackAuthSpec,
        ExtensionPackBinding, ExtensionPackConnection, ExtensionPackConnectionUpsertRequest,
        ExtensionPackEventRecord, ExtensionPackManifest, ExtensionPackRegistry,
        ExtensionPackRuntimeStateRecord, ExtensionPackSourceKind, ExtensionPackTrustLevel,
        InstalledExtensionPack, PackFeatureManifest, auth_profile_material_for_secret_backed_pack,
        connection_secret_key, imported_auth_contract, manifest_uses_connection_secret,
        required_secrets_for_auth_mode,
    };
    use crate::core::auth_profiles::{
        AuthProfileControlPlane, AuthProfileKind, AuthProfileMaterial, AuthProfileScope,
        AuthProfileUpsert,
    };
    use crate::custom_apis::{CustomApiAuthMode, CustomApiOperationDraft, CustomApiPreview};
    use crate::storage::{DatabaseConfig, Storage};
    use serde_json::json;

    #[test]
    fn extension_pack_auth_mode_accepts_canonical_and_legacy_oauth2_names() {
        let canonical: ExtensionPackAuthMode =
            serde_json::from_str("\"oauth2_external\"").expect("canonical auth mode");
        let legacy: ExtensionPackAuthMode =
            serde_json::from_str("\"o_auth2_external\"").expect("legacy auth mode");

        assert_eq!(canonical, ExtensionPackAuthMode::OAuth2External);
        assert_eq!(legacy, ExtensionPackAuthMode::OAuth2External);
        assert_eq!(
            serde_json::to_string(&ExtensionPackAuthMode::OAuth2External)
                .expect("serialize auth mode"),
            "\"oauth2_external\""
        );
    }

    #[test]
    fn generic_api_key_packs_only_require_api_key() {
        assert_eq!(
            required_secrets_for_auth_mode(ExtensionPackAuthMode::ApiKey),
            vec!["api_key".to_string()]
        );
    }

    #[test]
    fn imported_bearer_auth_uses_requested_api_key_shape_when_requested() {
        let preview = CustomApiPreview {
            suggested_id: "linear".to_string(),
            suggested_name: "Linear".to_string(),
            base_url: "https://api.linear.app".to_string(),
            auth_mode: CustomApiAuthMode::Bearer,
            auth_header: Some("Authorization".to_string()),
            auth_name: None,
            auth_username: None,
            operations: vec![CustomApiOperationDraft {
                id: "list_issues".to_string(),
                name: "List Issues".to_string(),
                method: "GET".to_string(),
                path: "/graphql".to_string(),
                description: String::new(),
                read_only: true,
                enabled: true,
                default_headers: Default::default(),
                default_query: Default::default(),
                parameters: Vec::new(),
                body_required: false,
            }],
            notes: Vec::new(),
            source_kind: "curl".to_string(),
        };

        let contract = imported_auth_contract(ExtensionPackAuthMode::ApiKey, &preview);

        assert_eq!(contract.mode, ExtensionPackAuthMode::ApiKey);
        assert_eq!(contract.required_secrets, vec!["api_key".to_string()]);
        assert_eq!(
            contract
                .binding
                .as_ref()
                .and_then(|value| value.get("secret_path"))
                .and_then(|value| value.as_str()),
            Some("api_key")
        );
    }

    #[test]
    fn bearer_secret_backed_pack_uses_bearer_auth_profile_material() {
        let manifest = ExtensionPackManifest {
            id: "linear".to_string(),
            name: "Linear".to_string(),
            version: "0.1.0".to_string(),
            kind: "integration".to_string(),
            auth: ExtensionPackAuthSpec {
                mode: ExtensionPackAuthMode::ApiKey,
                required_secrets: vec!["access_token".to_string()],
                required_scopes: Vec::new(),
                oauth2: None,
                exports: Default::default(),
                metadata: json!({
                    "secret_field": "access_token",
                    "auth_binding_type": "bearer",
                    "auth_header": "Authorization"
                }),
            },
            ..ExtensionPackManifest::default()
        };

        let material = auth_profile_material_for_secret_backed_pack(
            &manifest,
            &json!({ "access_token": "abc123" }),
        )
        .expect("bearer auth profile material");

        match material {
            AuthProfileMaterial::Bearer { token, header } => {
                assert_eq!(token, "abc123");
                assert_eq!(header.as_deref(), Some("Authorization"));
            }
            other => panic!("expected bearer auth material, got {other:?}"),
        }
    }

    #[cfg_attr(
        not(feature = "db-tests"),
        ignore = "requires explicit isolated Postgres test database"
    )]
    #[tokio::test]
    async fn secret_backed_external_oauth_pack_without_oauth_spec_can_be_ready() {
        let dir = tempfile::tempdir().expect("tempdir");
        let storage = Storage::connect(DatabaseConfig::for_tests().expect("database config"))
            .await
            .expect("storage");
        let mut registry =
            ExtensionPackRegistry::new(storage, dir.path().to_path_buf(), dir.path().to_path_buf());
        let manifest = ExtensionPackManifest {
            id: "issue_tracker".to_string(),
            name: "Issue Tracker".to_string(),
            version: "0.1.0".to_string(),
            kind: "integration".to_string(),
            auth: ExtensionPackAuthSpec {
                mode: ExtensionPackAuthMode::OAuth2External,
                required_secrets: vec!["access_token".to_string()],
                required_scopes: Vec::new(),
                oauth2: None,
                exports: Default::default(),
                metadata: json!({
                    "secret_field": "access_token",
                    "auth_binding_type": "bearer"
                }),
            },
            features: vec![PackFeatureManifest {
                id: "items.list".to_string(),
                kind: "capability".to_string(),
                title: "List items".to_string(),
                description: "List items from the connected service.".to_string(),
                read_only: true,
                experimental: false,
                input_schema: json!({ "type": "object" }),
                output_schema: json!({}),
                binding: Some(ExtensionPackBinding {
                    kind: "http".to_string(),
                    config: json!({
                        "method": "GET",
                        "url": "https://api.example.com/items",
                        "auth": {
                            "type": "bearer",
                            "secret_path": "access_token"
                        }
                    }),
                }),
            }],
            ..ExtensionPackManifest::default()
        };
        assert!(manifest_uses_connection_secret(&manifest));
        registry.installed.insert(
            manifest.id.clone(),
            InstalledExtensionPack {
                manifest: manifest.clone(),
                trust_level: ExtensionPackTrustLevel::Unverified,
                verification_status: "unverified".to_string(),
                verification_detail: Some("draft".to_string()),
                source_kind: ExtensionPackSourceKind::Scaffolded,
                source_url: None,
                enabled: true,
                runtime_state: ExtensionPackRuntimeStateRecord::default(),
                installed_at: chrono::Utc::now().to_rfc3339(),
                updated_at: chrono::Utc::now().to_rfc3339(),
            },
        );

        let view = registry
            .upsert_connection(
                &manifest.id,
                ExtensionPackConnectionUpsertRequest {
                    name: Some("Default connection".to_string()),
                    secret: Some(json!({ "access_token": "token-value" })),
                    ..ExtensionPackConnectionUpsertRequest::default()
                },
            )
            .await
            .expect("connection saved");

        assert_eq!(view.state, ExtensionConnectionState::Ready);
    }

    #[cfg_attr(
        not(feature = "db-tests"),
        ignore = "requires explicit isolated Postgres test database"
    )]
    #[tokio::test]
    async fn draft_pack_with_ready_connection_registers_runtime_actions() {
        let dir = tempfile::tempdir().expect("tempdir");
        let storage = Storage::connect(DatabaseConfig::for_tests().expect("database config"))
            .await
            .expect("storage");
        let mut registry =
            ExtensionPackRegistry::new(storage, dir.path().to_path_buf(), dir.path().to_path_buf());
        let manifest = ExtensionPackManifest {
            id: "work_items".to_string(),
            name: "Work Items".to_string(),
            version: "0.1.0".to_string(),
            kind: "integration".to_string(),
            draft: true,
            auth: ExtensionPackAuthSpec {
                mode: ExtensionPackAuthMode::ApiKey,
                required_secrets: vec!["api_key".to_string()],
                required_scopes: Vec::new(),
                oauth2: None,
                exports: Default::default(),
                metadata: json!({
                    "secret_field": "api_key",
                    "auth_binding_type": "header",
                    "auth_header": "X-API-Key"
                }),
            },
            features: vec![PackFeatureManifest {
                id: "items.list".to_string(),
                kind: "capability".to_string(),
                title: "List items".to_string(),
                description: "List work items.".to_string(),
                read_only: true,
                experimental: false,
                input_schema: json!({ "type": "object" }),
                output_schema: json!({}),
                binding: Some(ExtensionPackBinding {
                    kind: "http".to_string(),
                    config: json!({
                        "method": "GET",
                        "url": "https://api.example.com/items",
                        "auth": {
                            "type": "header",
                            "name": "X-API-Key",
                            "secret_path": "api_key"
                        }
                    }),
                }),
            }],
            ..ExtensionPackManifest::default()
        };
        registry.installed.insert(
            manifest.id.clone(),
            InstalledExtensionPack {
                manifest: manifest.clone(),
                trust_level: ExtensionPackTrustLevel::Unverified,
                verification_status: "unverified".to_string(),
                verification_detail: Some("draft".to_string()),
                source_kind: ExtensionPackSourceKind::Scaffolded,
                source_url: None,
                enabled: true,
                runtime_state: ExtensionPackRuntimeStateRecord::default(),
                installed_at: chrono::Utc::now().to_rfc3339(),
                updated_at: chrono::Utc::now().to_rfc3339(),
            },
        );
        registry
            .upsert_connection(
                &manifest.id,
                ExtensionPackConnectionUpsertRequest {
                    name: Some("Default connection".to_string()),
                    secret: Some(json!({ "api_key": "secret-value" })),
                    ..ExtensionPackConnectionUpsertRequest::default()
                },
            )
            .await
            .expect("connection saved");

        let view = registry
            .get_pack(&manifest.id)
            .await
            .expect("pack lookup")
            .expect("installed pack");
        assert_eq!(view.status, "connected");

        let specs = registry
            .runtime_action_specs()
            .await
            .expect("runtime action specs");
        assert!(specs.iter().any(|spec| {
            spec.binding.pack_id == manifest.id && spec.binding.feature_id == "items.list"
        }));
    }

    #[cfg_attr(
        not(feature = "db-tests"),
        ignore = "requires explicit isolated Postgres test database"
    )]
    #[tokio::test]
    async fn delete_pack_removes_owned_state_and_secret_namespace() {
        let dir = tempfile::tempdir().expect("tempdir");
        let storage = Storage::connect(DatabaseConfig::for_tests().expect("database config"))
            .await
            .expect("storage");
        let mut registry = ExtensionPackRegistry::new(
            storage.clone(),
            dir.path().to_path_buf(),
            dir.path().to_path_buf(),
        );
        let manifest = ExtensionPackManifest {
            id: "linear".to_string(),
            name: "Linear".to_string(),
            version: "0.1.0".to_string(),
            kind: "integration".to_string(),
            draft: true,
            auth: ExtensionPackAuthSpec {
                mode: ExtensionPackAuthMode::ApiKey,
                required_secrets: vec!["access_token".to_string()],
                required_scopes: Vec::new(),
                oauth2: None,
                exports: Default::default(),
                metadata: json!({
                    "secret_field": "access_token",
                    "auth_binding_type": "bearer",
                    "auth_header": "Authorization"
                }),
            },
            features: vec![PackFeatureManifest {
                id: "issues.list".to_string(),
                kind: "capability".to_string(),
                title: "List issues".to_string(),
                description: "List issues from Linear.".to_string(),
                read_only: true,
                experimental: false,
                input_schema: json!({}),
                output_schema: json!({}),
                binding: Some(ExtensionPackBinding {
                    kind: "http".to_string(),
                    config: json!({
                        "method": "POST",
                        "url": "https://api.linear.app/graphql"
                    }),
                }),
            }],
            ..ExtensionPackManifest::default()
        };
        let now = chrono::Utc::now().to_rfc3339();
        registry.installed.insert(
            manifest.id.clone(),
            InstalledExtensionPack {
                manifest: manifest.clone(),
                trust_level: ExtensionPackTrustLevel::Unverified,
                verification_status: "unverified".to_string(),
                verification_detail: Some("draft".to_string()),
                source_kind: ExtensionPackSourceKind::Scaffolded,
                source_url: None,
                enabled: true,
                runtime_state: ExtensionPackRuntimeStateRecord::default(),
                installed_at: now.clone(),
                updated_at: now.clone(),
            },
        );
        registry
            .persist_installed()
            .await
            .expect("persist installed");

        let connection_id = "default".to_string();
        let auth_profile_id = registry
            .upsert_connection_auth_profile(
                &manifest,
                &connection_id,
                "Default connection",
                Some(&json!({ "access_token": "linear-secret" })),
                None,
            )
            .await
            .expect("create managed auth profile")
            .expect("managed auth profile id");
        registry.connections.insert(
            connection_id.clone(),
            ExtensionPackConnection {
                id: connection_id.clone(),
                pack_id: manifest.id.clone(),
                name: "Default connection".to_string(),
                enabled: true,
                auth_profile_id: Some(auth_profile_id.clone()),
                metadata: serde_json::Value::Null,
                last_error: None,
                last_tested_at: None,
                created_at: now.clone(),
                updated_at: now.clone(),
            },
        );
        registry
            .persist_connections()
            .await
            .expect("persist connection");
        registry
            .store_connection_secret(
                &manifest.id,
                &connection_id,
                Some(json!({ "access_token": "linear-secret" })),
            )
            .expect("store pack secret");
        let manager = crate::core::config::SecureConfigManager::new_with_data_dir(
            dir.path(),
            Some(dir.path()),
        )
        .expect("secure config manager");
        manager
            .set_custom_secret(
                &connection_secret_key(&manifest.id, "orphan"),
                Some("orphan-secret".to_string()),
            )
            .expect("store orphan secret");
        manager
            .set_custom_secret(
                &connection_secret_key("github", "default"),
                Some("keep-secret".to_string()),
            )
            .expect("store unrelated secret");
        registry.events = vec![
            ExtensionPackEventRecord {
                id: "event-linear".to_string(),
                pack_id: manifest.id.clone(),
                feature_id: "issues.list".to_string(),
                connection_id: Some(connection_id.clone()),
                event_type: "webhook".to_string(),
                provider_event_id: None,
                transport: "http".to_string(),
                status: "processed".to_string(),
                outcome: None,
                response_preview: None,
                metadata: serde_json::Value::Null,
                payload: serde_json::Value::Null,
                received_at: now.clone(),
                processed_at: Some(now.clone()),
            },
            ExtensionPackEventRecord {
                id: "event-github".to_string(),
                pack_id: "github".to_string(),
                feature_id: "issues.list".to_string(),
                connection_id: None,
                event_type: "webhook".to_string(),
                provider_event_id: None,
                transport: "http".to_string(),
                status: "processed".to_string(),
                outcome: None,
                response_preview: None,
                metadata: serde_json::Value::Null,
                payload: serde_json::Value::Null,
                received_at: now.clone(),
                processed_at: Some(now.clone()),
            },
        ];
        registry.persist_events().await.expect("persist events");
        let runtime_dir = registry.pack_runtime_dir(&manifest.id);
        std::fs::create_dir_all(&runtime_dir).expect("create runtime dir");
        std::fs::write(runtime_dir.join("state.json"), "{}").expect("write runtime artifact");

        let unrelated_profile = AuthProfileControlPlane::upsert(
            &storage,
            AuthProfileUpsert {
                id: Some("github-profile".to_string()),
                name: Some("GitHub Default".to_string()),
                kind: Some(AuthProfileKind::ApiKey),
                enabled: Some(true),
                scope: Some(AuthProfileScope::User),
                provider: Some("github".to_string()),
                metadata: Some(json!({
                    "extension_pack_id": "github",
                    "connection_id": "default"
                })),
                material: Some(AuthProfileMaterial::ApiKey {
                    value: "keep-me".to_string(),
                    header: Some("Authorization".to_string()),
                    query_name: None,
                }),
                ..AuthProfileUpsert::default()
            },
        )
        .await
        .expect("create unrelated profile");

        registry
            .delete_pack(&manifest.id, true)
            .await
            .expect("delete pack");

        assert!(!registry.installed.contains_key(&manifest.id));
        assert!(!registry.connections.contains_key(&connection_id));
        assert!(
            registry
                .events
                .iter()
                .all(|event| !event.pack_id.eq_ignore_ascii_case(&manifest.id))
        );
        assert!(
            registry
                .get_connection_secret(&manifest.id, &connection_id)
                .expect("lookup deleted secret")
                .is_none()
        );
        assert!(
            registry
                .get_connection_secret(&manifest.id, "orphan")
                .expect("lookup orphan secret")
                .is_none()
        );
        assert!(
            registry
                .get_connection_secret("github", "default")
                .expect("lookup unrelated secret")
                .is_some()
        );
        assert!(
            AuthProfileControlPlane::get(&storage, &auth_profile_id)
                .await
                .expect("read deleted profile")
                .is_none()
        );
        assert!(!runtime_dir.exists());
        assert!(
            AuthProfileControlPlane::get(&storage, &unrelated_profile.id)
                .await
                .expect("read unrelated profile")
                .is_some()
        );

        let mut reloaded = ExtensionPackRegistry::new(
            storage.clone(),
            dir.path().to_path_buf(),
            dir.path().to_path_buf(),
        );
        reloaded.sync_from_storage().await.expect("reload registry");
        assert!(!reloaded.installed.contains_key(&manifest.id));
        assert!(
            !reloaded
                .connections
                .values()
                .any(|connection| connection.pack_id.eq_ignore_ascii_case(&manifest.id))
        );
        assert!(
            reloaded
                .events
                .iter()
                .all(|event| !event.pack_id.eq_ignore_ascii_case(&manifest.id))
        );
    }

    #[cfg_attr(
        not(feature = "db-tests"),
        ignore = "requires explicit isolated Postgres test database"
    )]
    #[tokio::test]
    async fn delete_pack_rejects_bundled_registry_packs() {
        let dir = tempfile::tempdir().expect("tempdir");
        let storage = Storage::connect(DatabaseConfig::for_tests().expect("database config"))
            .await
            .expect("storage");
        let mut registry =
            ExtensionPackRegistry::new(storage, dir.path().to_path_buf(), dir.path().to_path_buf());
        let manifest = ExtensionPackManifest {
            id: "bundled_issue_tracker".to_string(),
            name: "Bundled Issue Tracker".to_string(),
            version: "1.0.0".to_string(),
            kind: "integration".to_string(),
            signature: Some("bundled".to_string()),
            draft: false,
            features: Vec::new(),
            ..ExtensionPackManifest::default()
        };
        let now = chrono::Utc::now().to_rfc3339();
        registry.installed.insert(
            manifest.id.clone(),
            InstalledExtensionPack {
                manifest: manifest.clone(),
                trust_level: ExtensionPackTrustLevel::Trusted,
                verification_status: "bundled".to_string(),
                verification_detail: Some("Bundled with AgentArk.".to_string()),
                source_kind: ExtensionPackSourceKind::BundledRegistry,
                source_url: None,
                enabled: true,
                runtime_state: ExtensionPackRuntimeStateRecord::default(),
                installed_at: now.clone(),
                updated_at: now,
            },
        );

        let error = registry
            .delete_pack(&manifest.id, true)
            .await
            .expect_err("bundled packs should not be deletable");

        assert!(error.to_string().contains("cannot be deleted"));
        assert!(registry.installed.contains_key(&manifest.id));
    }
}
