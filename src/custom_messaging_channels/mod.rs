use anyhow::{anyhow, bail, Context, Result};
use once_cell::sync::Lazy;
use serde::{Deserialize, Serialize};
use std::collections::{BTreeMap, BTreeSet};
use tokio::sync::Mutex;

use crate::channels::messaging_dispatch::{
    dispatch_pack_channel_with_overlay, extract_secret_references, rewrite_send_spec_secret_refs,
    DispatchInputs,
};
use crate::core::config::SecureConfigManager;
use crate::core::integration_auth::{
    AuthField, AuthMode, FieldInputType, IntegrationAuthManifest, OAuth2CodeFlow, OAuth2DeviceFlow,
    PostSubmitAction, PostSubmitAfter, SecretSlot,
};
use crate::extension_packs::{AuthTransportBinding, MessagingHeaderSpec, MessagingSendSpec};
use crate::storage::Storage;

const CUSTOM_MESSAGING_CHANNEL_CONFIGS_KEY: &str = "custom_messaging_channel:configs:v1";
pub const CUSTOM_CHANNEL_ID_PREFIX: &str = "custom.";
const CUSTOM_CHANNEL_AUTH_PREFIX: &str = "custom_messaging_channel:";
const CHANNEL_TEST_COOLDOWN_SECS: i64 = 30;
static CUSTOM_CHANNEL_CONFIG_LOCK: Lazy<Mutex<()>> = Lazy::new(|| Mutex::new(()));

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct CustomMessagingChannelConfig {
    pub id: String,
    pub name: String,
    #[serde(default)]
    pub description: String,
    #[serde(default)]
    pub enabled: bool,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub docs_url: Option<String>,
    pub send: MessagingSendSpec,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub auth_manifest: Option<IntegrationAuthManifest>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub auth_profile_id: Option<String>,
    pub created_at: String,
    pub updated_at: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub last_tested_at: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub last_test_outcome: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub last_test_message: Option<String>,
}

#[derive(Debug, Clone, Serialize)]
pub struct CustomMessagingChannelView {
    #[serde(skip)]
    pub config: CustomMessagingChannelConfig,
    pub id: String,
    pub name: String,
    pub description: String,
    pub enabled: bool,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub docs_url: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub auth_manifest: Option<IntegrationAuthManifest>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub auth_profile_id: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub last_tested_at: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub last_test_outcome: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub last_test_message: Option<String>,
    pub runtime_channel_id: String,
    pub configured: bool,
    pub requires_auth: bool,
    pub required_secret_count: usize,
}

#[derive(Debug, Clone, Deserialize)]
pub struct CustomMessagingCredentialFieldDraft {
    pub key: String,
    #[serde(default)]
    pub label: Option<String>,
    #[serde(default)]
    pub placeholder: Option<String>,
    #[serde(default)]
    pub help: Option<String>,
    #[serde(default)]
    pub input_type: Option<String>,
    #[serde(default)]
    pub required: Option<bool>,
}

#[derive(Debug, Clone, Deserialize)]
pub struct CustomMessagingChannelUpsertRequest {
    #[serde(default)]
    pub id: Option<String>,
    pub name: String,
    #[serde(default)]
    pub description: Option<String>,
    #[serde(default)]
    pub enabled: Option<bool>,
    #[serde(default)]
    pub docs_url: Option<String>,
    pub send: MessagingSendSpec,
    #[serde(default)]
    pub auth_manifest: Option<IntegrationAuthManifest>,
    #[serde(default)]
    pub auth_profile_id: Option<String>,
    #[serde(default)]
    pub credential_fields: Vec<CustomMessagingCredentialFieldDraft>,
    #[serde(default)]
    pub clear_secrets: Option<bool>,
}

#[derive(Debug, Deserialize)]
pub struct CustomMessagingChannelCredentialsRequest {
    #[serde(default)]
    pub values: BTreeMap<String, String>,
}

#[derive(Debug, Serialize)]
pub struct CustomMessagingChannelTestResult {
    pub ok: bool,
    pub channel_id: String,
    pub detail: String,
}

pub fn runtime_channel_id(id: &str) -> String {
    format!("{}{}", CUSTOM_CHANNEL_ID_PREFIX, sanitize_id(id))
}

pub fn config_id_for_request(request: &CustomMessagingChannelUpsertRequest) -> String {
    let requested_id = request
        .id
        .clone()
        .unwrap_or_else(|| sanitize_id(request.name.trim()));
    sanitize_id(&requested_id)
}

pub fn auth_integration_id(id: &str) -> String {
    format!("{}{}", CUSTOM_CHANNEL_AUTH_PREFIX, sanitize_id(id))
}

pub fn config_id_from_auth_integration_id(integration_id: &str) -> Option<String> {
    integration_id
        .trim()
        .strip_prefix(CUSTOM_CHANNEL_AUTH_PREFIX)
        .map(sanitize_id)
        .filter(|value| !value.is_empty())
}

pub fn storage_target(id: &str, field_key: &str) -> String {
    format!(
        "{}{}:{}",
        CUSTOM_CHANNEL_AUTH_PREFIX,
        sanitize_id(id),
        sanitize_field_key(field_key)
    )
}

pub async fn list_custom_messaging_channels(
    storage: &Storage,
    config_dir: &std::path::Path,
    data_dir: &std::path::Path,
) -> Result<Vec<CustomMessagingChannelView>> {
    let manager = SecureConfigManager::new_with_data_dir(config_dir, Some(data_dir)).ok();
    let mut rows = load_configs(storage).await?;
    rows.sort_by(|left, right| right.updated_at.cmp(&left.updated_at));
    let mut views = Vec::with_capacity(rows.len());
    for config in rows {
        views.push(view_for_config(storage, manager.as_ref(), config).await?);
    }
    Ok(views)
}

pub async fn get_custom_messaging_channel_config(
    storage: &Storage,
    id: &str,
) -> Result<Option<CustomMessagingChannelConfig>> {
    let id = sanitize_id(id);
    Ok(load_configs(storage)
        .await?
        .into_iter()
        .find(|item| item.id == id))
}

pub async fn upsert_custom_messaging_channel(
    storage: &Storage,
    config_dir: &std::path::Path,
    data_dir: &std::path::Path,
    request: CustomMessagingChannelUpsertRequest,
    path_id: Option<&str>,
) -> Result<CustomMessagingChannelView> {
    let _guard = CUSTOM_CHANNEL_CONFIG_LOCK.lock().await;
    let name = request.name.trim();
    if name.is_empty() {
        bail!("Name is required");
    }
    let id = path_id
        .map(sanitize_id)
        .unwrap_or_else(|| config_id_for_request(&request));
    if id.is_empty() {
        bail!("Channel id is required");
    }

    let mut configs = load_configs(storage).await?;
    let existing_index = configs.iter().position(|item| item.id == id);
    if path_id.is_some() && existing_index.is_none() {
        bail!("Custom messaging channel not found");
    }
    if path_id.is_none() && existing_index.is_some() {
        bail!("A custom messaging channel with that id already exists");
    }
    let existing = existing_index.and_then(|index| configs.get(index).cloned());
    let now = chrono::Utc::now().to_rfc3339();
    let created_at = existing
        .as_ref()
        .map(|item| item.created_at.clone())
        .unwrap_or_else(|| now.clone());
    let enabled = request
        .enabled
        .or_else(|| existing.as_ref().map(|item| item.enabled))
        .unwrap_or(true);

    validate_send_spec(&request.send)?;
    let mut auth_manifest = normalize_auth_manifest(
        &id,
        name,
        request.docs_url.clone(),
        request.auth_manifest,
        request.credential_fields,
        &request.send,
    )?;
    let aliases = auth_manifest
        .as_ref()
        .map(secret_aliases_for_manifest)
        .unwrap_or_default();
    let send = rewrite_send_spec_secret_refs(&request.send, &aliases);
    validate_send_spec(&send)?;

    let auth_profile_id = clean_optional_string(request.auth_profile_id.as_deref()).or_else(|| {
        existing
            .as_ref()
            .and_then(|item| item.auth_profile_id.clone())
    });
    if let Some(profile_id) = auth_profile_id.as_deref() {
        let profile =
            crate::core::auth_profiles::AuthProfileControlPlane::get(storage, profile_id).await?;
        if profile.is_none() {
            bail!("Auth profile '{}' was not found.", profile_id);
        }
    }

    if let Some(manifest) = auth_manifest.as_mut() {
        manifest.integration_id = auth_integration_id(&id);
        if manifest.display_name.trim().is_empty() {
            manifest.display_name = name.to_string();
        }
        if manifest.docs_url.is_none() {
            manifest.docs_url = request.docs_url.clone();
        }
    }

    let manager = SecureConfigManager::new_with_data_dir(config_dir, Some(data_dir))?;
    if request.clear_secrets.unwrap_or(false) {
        let clear_manifest = auth_manifest
            .as_ref()
            .or_else(|| existing.as_ref().and_then(|c| c.auth_manifest.as_ref()));
        if let Some(manifest) = clear_manifest {
            clear_manifest_secrets(&manager, manifest)?;
        }
    }

    let config = CustomMessagingChannelConfig {
        id: id.clone(),
        name: name.to_string(),
        description: request.description.unwrap_or_default().trim().to_string(),
        enabled,
        docs_url: request.docs_url,
        send,
        auth_manifest,
        auth_profile_id,
        created_at,
        updated_at: now,
        last_tested_at: existing
            .as_ref()
            .and_then(|item| item.last_tested_at.clone()),
        last_test_outcome: existing
            .as_ref()
            .and_then(|item| item.last_test_outcome.clone()),
        last_test_message: existing
            .as_ref()
            .and_then(|item| item.last_test_message.clone()),
    };

    if let Some(index) = existing_index {
        configs[index] = config.clone();
    } else {
        configs.push(config.clone());
    }
    save_configs(storage, &configs).await?;
    view_for_config(storage, Some(&manager), config).await
}

pub async fn store_custom_messaging_channel_credentials(
    storage: &Storage,
    config_dir: &std::path::Path,
    data_dir: &std::path::Path,
    id: &str,
    values: &BTreeMap<String, String>,
) -> Result<CustomMessagingChannelView> {
    let id = sanitize_id(id);
    let Some(config) = get_custom_messaging_channel_config(storage, &id).await? else {
        bail!("Custom messaging channel not found");
    };
    let Some(manifest) = config.auth_manifest.as_ref() else {
        bail!("This channel does not declare credential fields.");
    };
    let manager = SecureConfigManager::new_with_data_dir(config_dir, Some(data_dir))?;
    store_manifest_values(&manager, manifest, values)?;
    view_for_config(storage, Some(&manager), config).await
}

pub async fn delete_custom_messaging_channel(
    storage: &Storage,
    config_dir: &std::path::Path,
    data_dir: &std::path::Path,
    id: &str,
) -> Result<()> {
    let _guard = CUSTOM_CHANNEL_CONFIG_LOCK.lock().await;
    let id = sanitize_id(id);
    let mut configs = load_configs(storage).await?;
    let Some(index) = configs.iter().position(|item| item.id == id) else {
        bail!("Custom messaging channel not found");
    };
    let removed = configs.remove(index);
    save_configs(storage, &configs).await?;
    if let Some(manifest) = removed.auth_manifest.as_ref() {
        let manager = SecureConfigManager::new_with_data_dir(config_dir, Some(data_dir))?;
        clear_manifest_secrets(&manager, manifest)?;
    }
    Ok(())
}

pub async fn test_custom_messaging_channel(
    storage: &Storage,
    config_dir: &std::path::Path,
    data_dir: &std::path::Path,
    id: &str,
) -> Result<CustomMessagingChannelTestResult> {
    let id = sanitize_id(id);
    let manager = SecureConfigManager::new_with_data_dir(config_dir, Some(data_dir))?;
    let (config, tested_at) = {
        let _guard = CUSTOM_CHANNEL_CONFIG_LOCK.lock().await;
        let mut configs = load_configs(storage).await?;
        let Some(index) = configs.iter().position(|item| item.id == id) else {
            bail!("Custom messaging channel not found");
        };
        let config = configs[index].clone();
        let view = view_for_config(storage, Some(&manager), config.clone()).await?;
        if !view.configured {
            bail!(
                "Custom messaging channel '{}' is not configured.",
                config.name
            );
        }
        let started_at = chrono::Utc::now();
        if let Some(wait_seconds) =
            channel_test_cooldown_remaining(config.last_tested_at.as_deref(), started_at)
        {
            bail!(
                "Custom messaging channel test was run recently. Try again in {} seconds.",
                wait_seconds
            );
        }
        let tested_at = started_at.to_rfc3339();
        configs[index].last_tested_at = Some(tested_at.clone());
        configs[index].last_test_outcome = Some("running".to_string());
        configs[index].last_test_message = Some("Test started.".to_string());
        configs[index].updated_at = chrono::Utc::now().to_rfc3339();
        save_configs(storage, &configs).await?;
        (config, tested_at)
    };
    let overlay = if let Some(profile_id) = config.auth_profile_id.as_deref() {
        Some(
            crate::core::auth_profiles::AuthProfileControlPlane::resolve_http(storage, profile_id)
                .await?
                .overlay,
        )
    } else {
        None
    };
    let client = reqwest::Client::builder()
        .timeout(std::time::Duration::from_secs(20))
        .build()
        .context("Failed to build HTTP client")?;
    let inputs = DispatchInputs {
        text: "AgentArk test notification",
        to: None,
        conversation_id: None,
        subject: Some("AgentArk test notification"),
    };
    let result = dispatch_pack_channel_with_overlay(
        &client,
        &manager,
        &config.send,
        &inputs,
        overlay.as_ref(),
    )
    .await;
    let (ok, detail) = match result {
        Ok(outcome) => (
            true,
            format!("HTTP {} from channel endpoint.", outcome.http_status),
        ),
        Err(error) => (
            false,
            crate::security::redact_secret_input(&error.to_string()).text,
        ),
    };
    {
        let _guard = CUSTOM_CHANNEL_CONFIG_LOCK.lock().await;
        let mut configs = load_configs(storage).await?;
        if let Some(index) = configs.iter().position(|item| item.id == id) {
            configs[index].last_tested_at = Some(tested_at);
            configs[index].last_test_outcome = Some(if ok { "ok" } else { "error" }.to_string());
            configs[index].last_test_message = Some(detail.clone());
            configs[index].updated_at = chrono::Utc::now().to_rfc3339();
            save_configs(storage, &configs).await?;
        }
    }
    Ok(CustomMessagingChannelTestResult {
        ok,
        channel_id: runtime_channel_id(&id),
        detail,
    })
}

pub async fn view_for_config(
    storage: &Storage,
    manager: Option<&SecureConfigManager>,
    config: CustomMessagingChannelConfig,
) -> Result<CustomMessagingChannelView> {
    let auth_profile_ready = if let Some(profile_id) = config.auth_profile_id.as_deref() {
        crate::core::auth_profiles::AuthProfileControlPlane::get(storage, profile_id)
            .await?
            .is_some_and(|profile| profile.ready)
    } else {
        true
    };
    let manifest_ready = manifest_is_configured(config.auth_manifest.as_ref(), manager);
    let configured = config.enabled && auth_profile_ready && manifest_ready;
    let required_secret_count = config
        .auth_manifest
        .as_ref()
        .map(required_secret_targets)
        .unwrap_or_default()
        .len();
    let requires_auth = config.auth_manifest.is_some() || config.auth_profile_id.is_some();
    let id = config.id.clone();
    let name = config.name.clone();
    let description = config.description.clone();
    let enabled = config.enabled;
    let docs_url = config.docs_url.clone();
    let auth_manifest = config.auth_manifest.clone();
    let auth_profile_id = config.auth_profile_id.clone();
    let last_tested_at = config.last_tested_at.clone();
    let last_test_outcome = config.last_test_outcome.clone();
    let last_test_message = config.last_test_message.clone();
    Ok(CustomMessagingChannelView {
        runtime_channel_id: runtime_channel_id(&id),
        configured,
        requires_auth,
        required_secret_count,
        id,
        name,
        description,
        enabled,
        docs_url,
        auth_manifest,
        auth_profile_id,
        last_tested_at,
        last_test_outcome,
        last_test_message,
        config,
    })
}

pub fn manifest_is_configured(
    manifest: Option<&IntegrationAuthManifest>,
    manager: Option<&SecureConfigManager>,
) -> bool {
    let Some(manifest) = manifest else {
        return true;
    };
    let Some(manager) = manager else {
        return false;
    };
    let targets = required_secret_targets(manifest);
    targets.into_iter().all(|target| {
        manager
            .get_custom_secret(&target)
            .ok()
            .flatten()
            .is_some_and(|value| !value.trim().is_empty())
    })
}

pub fn secret_aliases_for_manifest(manifest: &IntegrationAuthManifest) -> BTreeMap<String, String> {
    let mut aliases = BTreeMap::new();
    let prefix = config_id_from_auth_integration_id(&manifest.integration_id)
        .map(|id| storage_target(&id, ""))
        .unwrap_or_default();
    let mut add_target = |logical: &str, target: &str| {
        let logical = sanitize_field_key(logical);
        let target = target.trim();
        if logical.is_empty() || target.is_empty() {
            return;
        }
        aliases.insert(logical, target.to_string());
        aliases.insert(target.to_string(), target.to_string());
        if !prefix.is_empty() && target.starts_with(&prefix) {
            let suffix = target[prefix.len()..].trim();
            if !suffix.is_empty() {
                aliases.insert(suffix.to_string(), target.to_string());
            }
        }
    };

    match &manifest.mode {
        AuthMode::Secrets { fields } | AuthMode::Hybrid { fields, .. } => {
            for field in fields {
                if let Some(target) = field.storage_targets.first() {
                    add_target(&field.key, target);
                }
                for target in &field.storage_targets {
                    add_target(&field.key, target);
                }
            }
        }
        AuthMode::OAuth2AuthorizationCode(_) | AuthMode::OAuth2DeviceCode(_) => {}
    }
    match &manifest.mode {
        AuthMode::OAuth2AuthorizationCode(flow) => add_oauth_aliases(flow, &mut add_target),
        AuthMode::Hybrid { oauth, .. } => add_oauth_aliases(oauth, &mut add_target),
        AuthMode::OAuth2DeviceCode(flow) => add_device_oauth_aliases(flow, &mut add_target),
        AuthMode::Secrets { .. } => {}
    }
    aliases
}

fn add_oauth_aliases(flow: &OAuth2CodeFlow, add: &mut impl FnMut(&str, &str)) {
    add("access_token", &flow.token_storage.access_token_key);
    if let Some(refresh) = flow.token_storage.refresh_token_key.as_deref() {
        add("refresh_token", refresh);
    }
    if let Some(expires) = flow.token_storage.expires_at_key.as_deref() {
        add("expires_at", expires);
    }
    add("client_id", &flow.client_id_source.0);
    if let Some(client_secret) = flow.client_secret_source.as_ref() {
        add("client_secret", &client_secret.0);
    }
}

fn add_device_oauth_aliases(flow: &OAuth2DeviceFlow, add: &mut impl FnMut(&str, &str)) {
    add("access_token", &flow.token_storage.access_token_key);
    if let Some(refresh) = flow.token_storage.refresh_token_key.as_deref() {
        add("refresh_token", refresh);
    }
    if let Some(expires) = flow.token_storage.expires_at_key.as_deref() {
        add("expires_at", expires);
    }
    add("client_id", &flow.client_id_source.0);
}

fn normalize_auth_manifest(
    id: &str,
    name: &str,
    docs_url: Option<String>,
    auth_manifest: Option<IntegrationAuthManifest>,
    credential_fields: Vec<CustomMessagingCredentialFieldDraft>,
    send_spec: &MessagingSendSpec,
) -> Result<Option<IntegrationAuthManifest>> {
    let integration_id = auth_integration_id(id);
    let mut manifest = match auth_manifest {
        Some(mut manifest) => {
            manifest.integration_id = integration_id;
            if manifest.display_name.trim().is_empty() {
                manifest.display_name = name.to_string();
            }
            if manifest.docs_url.is_none() {
                manifest.docs_url = docs_url.clone();
            }
            manifest
        }
        None => {
            let mut keys = BTreeSet::new();
            for field in credential_fields.iter().map(|field| field.key.as_str()) {
                let key = sanitize_field_key(field);
                if !key.is_empty() {
                    keys.insert(key);
                }
            }
            for key in extract_secret_references(send_spec) {
                let key = sanitize_field_key(&key);
                if !key.is_empty() {
                    keys.insert(key);
                }
            }
            if keys.is_empty() {
                return Ok(None);
            }
            let fields = keys
                .into_iter()
                .map(|key| {
                    let draft = credential_fields
                        .iter()
                        .find(|field| sanitize_field_key(&field.key) == key);
                    auth_field_from_draft(id, &key, draft)
                })
                .collect::<Vec<_>>();
            IntegrationAuthManifest {
                integration_id,
                display_name: name.to_string(),
                description: Some(format!(
                    "Enter the credentials required for {} delivery.",
                    name
                )),
                docs_url,
                warning: Some(
                    "The assistant does not see these values. They are sent directly to AgentArk and stored encrypted."
                        .to_string(),
                ),
                mode: AuthMode::Secrets { fields },
                post_submit: PostSubmitAction {
                    label: "Save credentials".to_string(),
                    after: PostSubmitAfter::CloseAndResume,
                },
            }
        }
    };
    normalize_manifest_storage_targets(id, &mut manifest)?;
    Ok(Some(manifest))
}

fn normalize_manifest_storage_targets(
    id: &str,
    manifest: &mut IntegrationAuthManifest,
) -> Result<()> {
    match &mut manifest.mode {
        AuthMode::Secrets { fields } | AuthMode::Hybrid { fields, .. } => {
            for field in fields {
                let key = sanitize_field_key(&field.key);
                if key.is_empty() {
                    bail!("Credential field keys must be non-empty.");
                }
                field.key = key.clone();
                field.storage_targets = vec![storage_target(id, &key)];
            }
        }
        AuthMode::OAuth2AuthorizationCode(_) | AuthMode::OAuth2DeviceCode(_) => {}
    }
    match &mut manifest.mode {
        AuthMode::OAuth2AuthorizationCode(flow) => normalize_oauth_storage_targets(id, flow),
        AuthMode::Hybrid { oauth, .. } => normalize_oauth_storage_targets(id, oauth),
        AuthMode::OAuth2DeviceCode(flow) => normalize_device_oauth_storage_targets(id, flow),
        AuthMode::Secrets { .. } => {}
    }
    Ok(())
}

fn normalize_oauth_storage_targets(id: &str, flow: &mut OAuth2CodeFlow) {
    flow.client_id_source = SecretSlot(storage_target(id, "client_id"));
    if flow.client_secret_source.is_some() {
        flow.client_secret_source = Some(SecretSlot(storage_target(id, "client_secret")));
    }
    flow.token_storage.access_token_key = storage_target(id, "access_token");
    flow.token_storage.refresh_token_key = Some(storage_target(id, "refresh_token"));
    flow.token_storage.expires_at_key = Some(storage_target(id, "expires_at"));
}

fn normalize_device_oauth_storage_targets(id: &str, flow: &mut OAuth2DeviceFlow) {
    flow.client_id_source = SecretSlot(storage_target(id, "client_id"));
    flow.token_storage.access_token_key = storage_target(id, "access_token");
    flow.token_storage.refresh_token_key = Some(storage_target(id, "refresh_token"));
    flow.token_storage.expires_at_key = Some(storage_target(id, "expires_at"));
}

fn auth_field_from_draft(
    id: &str,
    key: &str,
    draft: Option<&CustomMessagingCredentialFieldDraft>,
) -> AuthField {
    AuthField {
        key: key.to_string(),
        label: draft
            .and_then(|draft| clean_optional_string(draft.label.as_deref()))
            .unwrap_or_else(|| humanise_key(key)),
        placeholder: draft.and_then(|draft| clean_optional_string(draft.placeholder.as_deref())),
        help: draft.and_then(|draft| clean_optional_string(draft.help.as_deref())),
        input_type: draft
            .and_then(|draft| draft.input_type.as_deref())
            .map(field_input_type_from_str)
            .unwrap_or(FieldInputType::Password),
        required: draft.and_then(|draft| draft.required).unwrap_or(true),
        storage_targets: vec![storage_target(id, key)],
        validation: None,
    }
}

fn field_input_type_from_str(value: &str) -> FieldInputType {
    match value.trim().to_ascii_lowercase().as_str() {
        "text" => FieldInputType::Text,
        "textarea" | "multiline" => FieldInputType::Textarea,
        _ => FieldInputType::Password,
    }
}

fn required_secret_targets(manifest: &IntegrationAuthManifest) -> Vec<String> {
    let mut out = Vec::new();
    match &manifest.mode {
        AuthMode::Secrets { fields } | AuthMode::Hybrid { fields, .. } => {
            for field in fields {
                if field.required {
                    out.extend(field.storage_targets.iter().cloned());
                }
            }
        }
        AuthMode::OAuth2AuthorizationCode(_) | AuthMode::OAuth2DeviceCode(_) => {}
    }
    match &manifest.mode {
        AuthMode::OAuth2AuthorizationCode(flow) => {
            out.push(flow.token_storage.access_token_key.clone());
        }
        AuthMode::Hybrid { oauth, .. } => {
            out.push(oauth.token_storage.access_token_key.clone());
        }
        AuthMode::OAuth2DeviceCode(flow) => {
            out.push(flow.token_storage.access_token_key.clone());
        }
        AuthMode::Secrets { .. } => {}
    }
    out.sort();
    out.dedup();
    out
}

fn store_manifest_values(
    manager: &SecureConfigManager,
    manifest: &IntegrationAuthManifest,
    values: &BTreeMap<String, String>,
) -> Result<()> {
    let fields = match &manifest.mode {
        AuthMode::Secrets { fields } | AuthMode::Hybrid { fields, .. } => fields,
        AuthMode::OAuth2AuthorizationCode(_) | AuthMode::OAuth2DeviceCode(_) => {
            bail!("This channel uses OAuth and does not accept direct secret fields.")
        }
    };
    for field in fields {
        let value = values
            .get(&field.key)
            .map(|value| value.trim())
            .filter(|value| !value.is_empty());
        if field.required && value.is_none() {
            bail!("Field '{}' is required.", field.key);
        }
        let Some(value) = value else {
            continue;
        };
        for target in &field.storage_targets {
            manager.set_custom_secret(target, Some(value.to_string()))?;
        }
    }
    Ok(())
}

fn clear_manifest_secrets(
    manager: &SecureConfigManager,
    manifest: &IntegrationAuthManifest,
) -> Result<()> {
    for target in crate::core::integration_auth::manifest_all_storage_targets(manifest) {
        manager.set_custom_secret(&target, None)?;
    }
    Ok(())
}

fn validate_send_spec(send: &MessagingSendSpec) -> Result<()> {
    reject_secret_literals_in_send_spec(send)?;
    let url = send.url_template.trim();
    if url.is_empty() {
        bail!("Channel send URL template is required.");
    }
    if !url.contains("{{") {
        let parsed = reqwest::Url::parse(url).context("Channel send URL must be absolute")?;
        crate::channels::messaging_dispatch::validate_channel_url_static(&parsed)?;
    } else if !(url.starts_with("http://")
        || url.starts_with("https://")
        || url.starts_with("{{secret:"))
    {
        bail!("Templated channel URL must start with http(s) or a full secret URL placeholder.");
    }
    if let Some(content_type) = send.content_type.as_deref() {
        if content_type.trim().is_empty() {
            bail!("Content type cannot be empty.");
        }
    }
    for header in &send.headers {
        validate_header(header)?;
    }
    validate_auth_transport(&send.auth)?;
    if let Some(codes) = send.expect_status.as_ref() {
        if codes.is_empty() {
            bail!("Expected status list cannot be empty.");
        }
        if codes.iter().any(|code| !(100..=599).contains(code)) {
            bail!("Expected HTTP status codes must be between 100 and 599.");
        }
    }
    Ok(())
}

fn reject_secret_literals_in_send_spec(send: &MessagingSendSpec) -> Result<()> {
    reject_secret_literal_template("URL template", &send.url_template)?;
    if let Some(body) = send.body_template.as_deref() {
        reject_secret_literal_template("body template", body)?;
    }
    for header in &send.headers {
        reject_secret_literal_template("header value template", &header.value_template)?;
    }
    match &send.auth {
        AuthTransportBinding::CustomHeader { value_template, .. }
        | AuthTransportBinding::QueryParam { value_template, .. } => {
            reject_secret_literal_template("auth value template", value_template)?;
        }
        AuthTransportBinding::None
        | AuthTransportBinding::Bearer { .. }
        | AuthTransportBinding::Basic { .. } => {}
    }
    Ok(())
}

fn reject_secret_literal_template(label: &str, raw: &str) -> Result<()> {
    let masked = mask_template_placeholders(raw);
    if crate::security::redact_secret_input(&masked).had_secret()
        || contains_opaque_literal_secret(&masked)
    {
        bail!(
            "{} contains secret-like literal material. Use credential fields and {{{{secret:KEY}}}} placeholders instead.",
            label
        );
    }
    Ok(())
}

fn mask_template_placeholders(raw: &str) -> String {
    let mut out = String::with_capacity(raw.len());
    let mut chars = raw.chars().peekable();
    while let Some(ch) = chars.next() {
        if ch == '{' && chars.peek() == Some(&'{') {
            out.push(' ');
            out.push(' ');
            chars.next();
            let mut previous = '\0';
            while let Some(inner) = chars.next() {
                out.push(' ');
                if previous == '}' && inner == '}' {
                    break;
                }
                previous = inner;
            }
            continue;
        }
        out.push(ch);
    }
    out
}

fn contains_opaque_literal_secret(raw: &str) -> bool {
    for token in raw.split(|ch: char| {
        !(ch.is_ascii_alphanumeric() || ch == '_' || ch == '-' || ch == '.' || ch == '+')
    }) {
        let token = token.trim_matches('.');
        if token.chars().count() >= 20
            && token
                .chars()
                .filter(|ch| ch.is_ascii_alphanumeric())
                .count()
                >= 16
            && shannon_entropy_bits_per_char(token) >= 3.5
        {
            return true;
        }
    }
    false
}

fn shannon_entropy_bits_per_char(value: &str) -> f64 {
    let mut counts = BTreeMap::<char, usize>::new();
    let mut total = 0usize;
    for ch in value.chars() {
        total += 1;
        *counts.entry(ch).or_insert(0) += 1;
    }
    if total == 0 {
        return 0.0;
    }
    counts.values().fold(0.0, |entropy, count| {
        let p = *count as f64 / total as f64;
        entropy - p * p.log2()
    })
}

fn channel_test_cooldown_remaining(
    last_tested_at: Option<&str>,
    now: chrono::DateTime<chrono::Utc>,
) -> Option<i64> {
    let last = last_tested_at
        .and_then(|value| chrono::DateTime::parse_from_rfc3339(value).ok())
        .map(|value| value.with_timezone(&chrono::Utc))?;
    let elapsed = now.signed_duration_since(last).num_seconds();
    if elapsed < CHANNEL_TEST_COOLDOWN_SECS {
        Some((CHANNEL_TEST_COOLDOWN_SECS - elapsed).max(1))
    } else {
        None
    }
}

fn validate_header(header: &MessagingHeaderSpec) -> Result<()> {
    let name = header.name.trim();
    if name.is_empty() {
        bail!("Header names cannot be empty.");
    }
    reqwest::header::HeaderName::from_bytes(name.as_bytes())
        .map(|_| ())
        .map_err(|_| anyhow!("Invalid header name '{}'.", name))
}

fn validate_auth_transport(binding: &AuthTransportBinding) -> Result<()> {
    match binding {
        AuthTransportBinding::None => Ok(()),
        AuthTransportBinding::Bearer { secret_key } => {
            require_non_empty(secret_key, "bearer secret key")
        }
        AuthTransportBinding::CustomHeader {
            name,
            value_template,
        } => {
            require_non_empty(name, "custom auth header name")?;
            validate_header(&MessagingHeaderSpec {
                name: name.clone(),
                value_template: value_template.clone(),
            })?;
            require_non_empty(value_template, "custom auth header value")
        }
        AuthTransportBinding::Basic {
            username_key,
            password_key,
        } => {
            require_non_empty(username_key, "basic auth username key")?;
            require_non_empty(password_key, "basic auth password key")
        }
        AuthTransportBinding::QueryParam {
            name,
            value_template,
        } => {
            require_non_empty(name, "query auth parameter name")?;
            require_non_empty(value_template, "query auth parameter value")
        }
    }
}

fn require_non_empty(value: &str, label: &str) -> Result<()> {
    if value.trim().is_empty() {
        bail!("{} cannot be empty.", label);
    }
    Ok(())
}

async fn load_configs(storage: &Storage) -> Result<Vec<CustomMessagingChannelConfig>> {
    let Some(bytes) = storage
        .get_encrypted(CUSTOM_MESSAGING_CHANNEL_CONFIGS_KEY)
        .await?
    else {
        return Ok(Vec::new());
    };
    serde_json::from_slice::<Vec<CustomMessagingChannelConfig>>(&bytes)
        .context("failed to decode custom messaging channel configs")
}

async fn save_configs(storage: &Storage, value: &[CustomMessagingChannelConfig]) -> Result<()> {
    let bytes =
        serde_json::to_vec(value).context("failed to encode custom messaging channel configs")?;
    storage
        .set_encrypted(CUSTOM_MESSAGING_CHANNEL_CONFIGS_KEY, &bytes)
        .await
}

fn clean_optional_string(value: Option<&str>) -> Option<String> {
    value
        .map(str::trim)
        .filter(|value| !value.is_empty())
        .map(str::to_string)
}

fn sanitize_id(value: &str) -> String {
    let mut out = String::with_capacity(value.len());
    let mut last_was_sep = false;
    for ch in value.trim().chars() {
        let next = if ch.is_ascii_alphanumeric() {
            Some(ch.to_ascii_lowercase())
        } else if ch == '_' || ch == '-' || ch == '.' {
            Some('_')
        } else if ch.is_whitespace() {
            Some('_')
        } else {
            None
        };
        if let Some(ch) = next {
            if ch == '_' {
                if last_was_sep {
                    continue;
                }
                last_was_sep = true;
            } else {
                last_was_sep = false;
            }
            out.push(ch);
        }
    }
    out.trim_matches('_').to_string()
}

fn sanitize_field_key(value: &str) -> String {
    let mut out = String::with_capacity(value.len());
    let mut last_was_sep = false;
    for ch in value.trim().chars() {
        let next = if ch.is_ascii_alphanumeric() {
            Some(ch.to_ascii_lowercase())
        } else if ch == '_' || ch == '-' || ch == '.' {
            Some('_')
        } else {
            None
        };
        if let Some(ch) = next {
            if ch == '_' {
                if last_was_sep {
                    continue;
                }
                last_was_sep = true;
            } else {
                last_was_sep = false;
            }
            out.push(ch);
        }
    }
    out.trim_matches('_').to_string()
}

fn humanise_key(key: &str) -> String {
    let mut out = String::new();
    for part in key.split('_') {
        if part.is_empty() {
            continue;
        }
        if !out.is_empty() {
            out.push(' ');
        }
        let mut chars = part.chars();
        if let Some(first) = chars.next() {
            out.push(first.to_ascii_uppercase());
            out.push_str(chars.as_str());
        }
    }
    if out.is_empty() {
        "Credential".to_string()
    } else {
        out
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::extension_packs::{HttpSendMethod, MessagingSendSpec};

    #[test]
    fn runtime_id_is_namespaced() {
        assert_eq!(runtime_channel_id("My Discord"), "custom.my_discord");
    }

    #[test]
    fn generated_auth_manifest_uses_namespaced_storage() {
        let send = MessagingSendSpec {
            method: HttpSendMethod::Post,
            url_template: "{{secret:webhook_url}}".to_string(),
            ..MessagingSendSpec::default()
        };
        let manifest = normalize_auth_manifest(
            "discord_hook",
            "Discord Hook",
            None,
            None,
            Vec::new(),
            &send,
        )
        .expect("manifest")
        .expect("auth");
        let targets = crate::core::integration_auth::manifest_form_storage_targets(&manifest);
        assert_eq!(
            targets,
            vec!["custom_messaging_channel:discord_hook:webhook_url".to_string()]
        );
    }

    #[test]
    fn send_spec_secret_refs_are_rewritten_to_storage_targets() {
        let send = MessagingSendSpec {
            method: HttpSendMethod::Post,
            url_template: "{{secret:webhook_url}}".to_string(),
            body_template: Some("{\"content\":\"{{text}}\"}".to_string()),
            ..MessagingSendSpec::default()
        };
        let manifest = normalize_auth_manifest(
            "discord_hook",
            "Discord Hook",
            None,
            None,
            Vec::new(),
            &send,
        )
        .expect("manifest")
        .expect("auth");
        let rewritten =
            rewrite_send_spec_secret_refs(&send, &secret_aliases_for_manifest(&manifest));
        assert_eq!(
            rewritten.url_template,
            "{{secret:custom_messaging_channel:discord_hook:webhook_url}}"
        );
    }

    #[test]
    fn send_spec_validation_rejects_private_literal_urls() {
        let send = MessagingSendSpec {
            method: HttpSendMethod::Post,
            url_template: "http://127.0.0.1/admin".to_string(),
            ..MessagingSendSpec::default()
        };
        let error = validate_send_spec(&send).expect_err("private URL should be rejected");
        assert!(error.to_string().contains("private or local"));
    }

    #[test]
    fn send_spec_validation_rejects_literal_secret_material() {
        let send = MessagingSendSpec {
            method: HttpSendMethod::Post,
            url_template: "https://example.com/notify".to_string(),
            body_template: Some(
                "{\"token\":\"2skdjfkj2wlfrj23kr2rlm\",\"text\":\"{{text}}\"}".to_string(),
            ),
            ..MessagingSendSpec::default()
        };
        let error = validate_send_spec(&send).expect_err("literal token should be rejected");
        assert!(error.to_string().contains("secret-like literal material"));
    }

    #[test]
    fn send_spec_validation_allows_secret_placeholders() {
        let send = MessagingSendSpec {
            method: HttpSendMethod::Post,
            url_template: "{{secret:webhook_url}}".to_string(),
            body_template: Some("{\"text\":\"{{text}}\"}".to_string()),
            ..MessagingSendSpec::default()
        };
        validate_send_spec(&send).expect("placeholder-only secret use should be valid");
    }
}
