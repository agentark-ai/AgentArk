use anyhow::{Context, Result, anyhow};
use futures::StreamExt;
use reqwest::header::{AUTHORIZATION, HeaderMap, HeaderName, HeaderValue};
use serde::{Deserialize, Serialize, de::DeserializeOwned};
use serde_json::Value;
use sha2::Digest;
use std::collections::{HashMap, HashSet};
use std::path::PathBuf;
use std::time::{Duration, Instant};

use crate::actions::{ActionDef, ActionSource};
use crate::runtime::{ActionRuntime, PluginBinding, SandboxMode};
use crate::storage::Storage;

const PLUGIN_CONFIGS_KEY: &str = "plugins:sdk:configs:v1";
const PLUGIN_LOGS_KEY: &str = "plugins:sdk:logs:v1";
const PLUGIN_SECRET_PREFIX: &str = "plugin_sdk_secret:";
const PLUGIN_SDK_VERSION: &str = "agentark-plugin/v1";
const PLUGIN_LOG_HISTORY_LIMIT: usize = 200;
const PLUGIN_MESSAGE_MAX_CHARS: usize = 600;
const PLUGIN_REQUEST_TIMEOUT_SECS: u64 = 10;
const PLUGIN_MANIFEST_MAX_BYTES: usize = 256 * 1024;
const PLUGIN_MIN_CALL_INTERVAL: Duration = Duration::from_millis(500);

#[derive(Debug, Clone, Copy, Serialize, Deserialize, PartialEq, Eq, Default)]
#[serde(rename_all = "snake_case")]
pub enum PluginAuthMode {
    #[default]
    None,
    Bearer,
    Header,
}

#[derive(Debug, Clone, Serialize, Deserialize, Default)]
pub struct PluginActionManifest {
    pub name: String,
    #[serde(default)]
    pub title: Option<String>,
    #[serde(default)]
    pub description: String,
    #[serde(default)]
    pub input_schema: Value,
    #[serde(default)]
    pub capabilities: Vec<String>,
    #[serde(default)]
    pub read_only: bool,
    #[serde(default)]
    pub outbound_write: bool,
    #[serde(default)]
    pub public_publish: bool,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct PluginManifest {
    pub sdk_version: String,
    pub id: String,
    pub name: String,
    pub version: String,
    #[serde(default)]
    pub description: String,
    #[serde(default)]
    pub actions: Vec<PluginActionManifest>,
    #[serde(default)]
    pub events: Vec<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct PluginConfig {
    pub id: String,
    pub name: String,
    pub base_url: String,
    pub enabled: bool,
    pub auth_mode: PluginAuthMode,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub auth_profile_id: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub auth_header: Option<String>,
    #[serde(default)]
    pub subscribed_events: Vec<String>,
    pub manifest: PluginManifest,
    #[serde(default)]
    pub manifest_hash: String,
    pub created_at: String,
    pub updated_at: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub last_synced_at: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub last_error: Option<String>,
}

#[derive(Debug, Clone, Serialize)]
pub struct PluginView {
    #[serde(flatten)]
    pub plugin: PluginConfig,
    pub token_configured: bool,
    pub available_events: Vec<String>,
    pub registered_actions: Vec<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct PluginLogRecord {
    pub id: String,
    pub plugin_id: String,
    pub plugin_name: String,
    pub kind: String,
    pub subject: String,
    pub outcome: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub message: Option<String>,
    pub created_at: String,
}

#[derive(Debug, Deserialize)]
pub struct PluginUpsertRequest {
    #[serde(default)]
    pub id: Option<String>,
    #[serde(default)]
    pub name: Option<String>,
    pub base_url: String,
    #[serde(default)]
    pub enabled: Option<bool>,
    #[serde(default)]
    pub auth_mode: Option<PluginAuthMode>,
    #[serde(default)]
    pub auth_profile_id: Option<String>,
    #[serde(default)]
    pub auth_header: Option<String>,
    #[serde(default)]
    pub token: Option<String>,
    #[serde(default)]
    pub clear_token: Option<bool>,
    #[serde(default)]
    pub subscribed_events: Option<Vec<String>>,
    #[serde(default)]
    pub allow_manifest_update: Option<bool>,
}

#[derive(Debug, Deserialize, Default)]
pub struct PluginLogsQuery {
    #[serde(default)]
    pub plugin_id: Option<String>,
    #[serde(default)]
    pub limit: Option<usize>,
}

#[derive(Debug, Serialize)]
pub struct PluginTestResult {
    pub ok: bool,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub detail: Option<String>,
}

pub struct PluginRegistry {
    storage: Storage,
    config_dir: PathBuf,
    data_dir: PathBuf,
    http_client: reqwest::Client,
    plugins: HashMap<String, PluginConfig>,
    last_call_at: HashMap<String, Instant>,
}

impl PluginRegistry {
    pub fn new(storage: Storage, config_dir: PathBuf, data_dir: PathBuf) -> Self {
        let http_client = reqwest::Client::builder()
            .timeout(std::time::Duration::from_secs(PLUGIN_REQUEST_TIMEOUT_SECS))
            .redirect(reqwest::redirect::Policy::none())
            .build()
            .unwrap_or_else(|_| reqwest::Client::new());
        Self {
            storage,
            config_dir,
            data_dir,
            http_client,
            plugins: HashMap::new(),
            last_call_at: HashMap::new(),
        }
    }

    pub fn platform_events() -> Vec<String> {
        [
            "webhook.received",
            "task.completed",
            "task.failed",
            "approval.requested",
        ]
        .iter()
        .map(|value| value.to_string())
        .collect()
    }

    pub async fn list_plugins(&self) -> Result<Vec<PluginView>> {
        let mut rows = Vec::with_capacity(self.plugins.len());
        for plugin in self.plugins.values() {
            rows.push(self.plugin_view(plugin).await?);
        }
        rows.sort_by(|left, right| right.plugin.updated_at.cmp(&left.plugin.updated_at));
        Ok(rows)
    }

    pub async fn get_plugin(&self, id: &str) -> Result<Option<PluginView>> {
        let Some(plugin) = self.plugins.get(id) else {
            return Ok(None);
        };
        Ok(Some(self.plugin_view(plugin).await?))
    }

    pub async fn sync_from_storage(&mut self, runtime: &ActionRuntime) -> Result<()> {
        runtime.unregister_plugin_actions().await;
        self.plugins.clear();
        for plugin in load_json::<Vec<PluginConfig>>(&self.storage, PLUGIN_CONFIGS_KEY).await? {
            if plugin.enabled {
                register_plugin_actions(
                    runtime,
                    &plugin,
                    self.plugin_auth_configured(&plugin).await?,
                )
                .await?;
            }
            self.plugins.insert(plugin.id.clone(), plugin);
        }
        Ok(())
    }

    pub async fn upsert_plugin(
        &mut self,
        runtime: &ActionRuntime,
        plugin_id: Option<&str>,
        request: PluginUpsertRequest,
    ) -> Result<PluginView> {
        let existing_id = plugin_id
            .map(|value| value.trim().to_string())
            .or_else(|| request.id.clone());
        let existing = existing_id
            .as_deref()
            .and_then(|value| self.plugins.get(value))
            .cloned();
        if plugin_id.is_some() && existing.is_none() {
            anyhow::bail!("Plugin not found");
        }

        let auth_mode = request.auth_mode.unwrap_or_else(|| {
            existing
                .as_ref()
                .map(|item| item.auth_mode)
                .unwrap_or_default()
        });
        let auth_profile_id = request
            .auth_profile_id
            .as_deref()
            .map(str::trim)
            .filter(|value| !value.is_empty())
            .map(|value| value.to_string())
            .or_else(|| {
                existing
                    .as_ref()
                    .and_then(|item| item.auth_profile_id.clone())
            });
        let auth_header = normalize_header_name(request.auth_header.as_deref()).or_else(|| {
            existing
                .as_ref()
                .and_then(|item| item.auth_header.clone())
                .filter(|_| request.auth_header.is_none())
        });
        let base_url = normalize_base_url(&request.base_url).await?;
        let stored_token = existing
            .as_ref()
            .and_then(|item| self.load_plugin_secret(&item.id).ok().flatten());
        let token = if request.clear_token.unwrap_or(false) {
            None
        } else {
            request
                .token
                .as_deref()
                .map(str::trim)
                .filter(|value| !value.is_empty())
                .map(|value| value.to_string())
                .or(stored_token)
        };

        if let Some(profile_id) = auth_profile_id.as_deref() {
            if crate::core::auth_profiles::AuthProfileControlPlane::get(&self.storage, profile_id)
                .await?
                .is_none()
            {
                anyhow::bail!("Auth profile '{}' was not found.", profile_id);
            }
        }
        if auth_profile_id.is_none()
            && !matches!(auth_mode, PluginAuthMode::None)
            && token.as_deref().unwrap_or("").is_empty()
        {
            anyhow::bail!("This auth mode requires a token.");
        }

        let manifest = match self
            .fetch_manifest(
                &base_url,
                auth_mode,
                auth_header.as_deref(),
                token.as_deref(),
                auth_profile_id.as_deref(),
            )
            .await
        {
            Ok(manifest) => manifest,
            Err(error) => {
                if let Some(existing_plugin) = existing.as_ref() {
                    let detail = sanitize_plugin_message(&error.to_string());
                    self.set_plugin_last_error(&existing_plugin.id, Some(detail.clone()))
                        .await?;
                    self.append_log(
                        existing_plugin,
                        "manifest",
                        "Manifest sync",
                        "error",
                        Some(detail),
                    )
                    .await?;
                }
                return Err(error);
            }
        };
        let manifest_hash = plugin_manifest_hash(&manifest)?;
        if let Some(existing_plugin) = existing.as_ref() {
            if !existing_plugin.manifest_hash.is_empty()
                && existing_plugin.manifest_hash != manifest_hash
                && !request.allow_manifest_update.unwrap_or(false)
            {
                anyhow::bail!(
                    "Plugin manifest changed. Re-registration requires allow_manifest_update=true after reviewing the new manifest."
                );
            }
        }
        let id = existing_id
            .clone()
            .filter(|value| !value.trim().is_empty())
            .map(|value| sanitize_plugin_id(&value))
            .filter(|value| !value.is_empty())
            .or_else(|| {
                let from_manifest = sanitize_plugin_id(&manifest.id);
                (!from_manifest.is_empty()).then_some(from_manifest)
            })
            .or_else(|| {
                request
                    .name
                    .as_deref()
                    .map(sanitize_plugin_id)
                    .filter(|value| !value.is_empty())
            })
            .unwrap_or_else(|| uuid::Uuid::new_v4().to_string());
        let created_at = existing
            .as_ref()
            .map(|item| item.created_at.clone())
            .unwrap_or_else(now_rfc3339);
        let name = request
            .name
            .as_deref()
            .map(str::trim)
            .filter(|value| !value.is_empty())
            .map(|value| value.to_string())
            .unwrap_or_else(|| manifest.name.clone());
        let subscribed_events = normalize_subscribed_events(
            request.subscribed_events.as_deref(),
            existing
                .as_ref()
                .map(|item| item.subscribed_events.as_slice()),
            &manifest.events,
        );
        let plugin = PluginConfig {
            id: id.clone(),
            name,
            base_url,
            enabled: request
                .enabled
                .unwrap_or_else(|| existing.as_ref().map(|item| item.enabled).unwrap_or(true)),
            auth_mode,
            auth_profile_id,
            auth_header,
            subscribed_events,
            manifest,
            manifest_hash,
            created_at,
            updated_at: now_rfc3339(),
            last_synced_at: Some(now_rfc3339()),
            last_error: None,
        };

        if request.clear_token.unwrap_or(false) {
            self.set_plugin_secret(&id, None)?;
        } else if let Some(token_value) = token {
            self.set_plugin_secret(&id, Some(token_value))?;
        }

        runtime.unregister_plugin_actions_for_plugin(&id).await;
        if plugin.enabled {
            register_plugin_actions(
                runtime,
                &plugin,
                self.plugin_auth_configured(&plugin).await?,
            )
            .await?;
        }
        self.plugins.insert(id.clone(), plugin.clone());
        self.persist_configs().await?;
        self.append_log(
            &plugin,
            "manifest",
            "Manifest synced",
            "success",
            Some(format!(
                "Loaded {} action(s), {} event(s).",
                plugin.manifest.actions.len(),
                plugin.manifest.events.len()
            )),
        )
        .await?;

        self.get_plugin(&id)
            .await?
            .ok_or_else(|| anyhow!("Plugin saved but could not be read back"))
    }

    pub async fn delete_plugin(&mut self, runtime: &ActionRuntime, id: &str) -> Result<()> {
        let removed = self
            .plugins
            .remove(id)
            .ok_or_else(|| anyhow!("Plugin not found"))?;
        runtime.unregister_plugin_actions_for_plugin(id).await;
        self.set_plugin_secret(id, None)?;
        self.persist_configs().await?;
        self.append_log(
            &removed,
            "manifest",
            "Plugin removed",
            "success",
            Some("Removed plugin registration and secrets.".to_string()),
        )
        .await?;
        Ok(())
    }

    pub async fn refresh_plugin(
        &mut self,
        runtime: &ActionRuntime,
        id: &str,
    ) -> Result<PluginView> {
        let existing = self
            .plugins
            .get(id)
            .cloned()
            .ok_or_else(|| anyhow!("Plugin not found"))?;
        let token = self.load_plugin_secret(id)?;
        let manifest = match self
            .fetch_manifest(
                &existing.base_url,
                existing.auth_mode,
                existing.auth_header.as_deref(),
                token.as_deref(),
                existing.auth_profile_id.as_deref(),
            )
            .await
        {
            Ok(manifest) => manifest,
            Err(error) => {
                let detail = sanitize_plugin_message(&error.to_string());
                self.set_plugin_last_error(id, Some(detail.clone())).await?;
                self.append_log(
                    &existing,
                    "manifest",
                    "Manifest refresh",
                    "error",
                    Some(detail),
                )
                .await?;
                return Err(error);
            }
        };
        let manifest_hash = plugin_manifest_hash(&manifest)?;
        if !existing.manifest_hash.is_empty() && existing.manifest_hash != manifest_hash {
            let detail = "Plugin manifest changed during refresh. Review and re-register before accepting new actions, capabilities, or events.".to_string();
            self.set_plugin_last_error(id, Some(detail.clone())).await?;
            self.append_log(
                &existing,
                "manifest",
                "Manifest refresh",
                "error",
                Some(detail.clone()),
            )
            .await?;
            anyhow::bail!("{}", detail);
        }
        let mut next = existing.clone();
        next.manifest = manifest;
        next.manifest_hash = manifest_hash;
        next.updated_at = now_rfc3339();
        next.last_synced_at = Some(now_rfc3339());
        next.last_error = None;
        next.subscribed_events = normalize_subscribed_events(
            Some(next.subscribed_events.as_slice()),
            None,
            &next.manifest.events,
        );
        runtime.unregister_plugin_actions_for_plugin(id).await;
        if next.enabled {
            register_plugin_actions(runtime, &next, self.plugin_auth_configured(&next).await?)
                .await?;
        }
        self.plugins.insert(id.to_string(), next.clone());
        self.persist_configs().await?;
        self.append_log(
            &next,
            "manifest",
            "Manifest refreshed",
            "success",
            Some(format!(
                "Loaded {} action(s), {} event(s).",
                next.manifest.actions.len(),
                next.manifest.events.len()
            )),
        )
        .await?;
        self.get_plugin(id)
            .await?
            .ok_or_else(|| anyhow!("Plugin refreshed but could not be read back"))
    }

    pub async fn invoke_action(
        &mut self,
        plugin_id: &str,
        action_name: &str,
        arguments: &Value,
    ) -> Result<String> {
        let plugin = self
            .plugins
            .get(plugin_id)
            .cloned()
            .ok_or_else(|| anyhow!("Plugin '{}' not found", plugin_id))?;
        if !plugin.enabled {
            anyhow::bail!("Plugin '{}' is disabled", plugin_id);
        }
        let action = plugin
            .manifest
            .actions
            .iter()
            .find(|action| action.name == action_name)
            .cloned()
            .ok_or_else(|| anyhow!("Plugin action '{}' is not registered", action_name))?;
        self.enforce_plugin_rate_limit(plugin_id).await;
        let token = self.load_plugin_secret(plugin_id)?;
        let request = match self
            .build_authorized_request(
                reqwest::Method::POST,
                &plugin_endpoint(
                    &plugin.base_url,
                    &format!("/agentark/actions/{}", action.name),
                ),
                plugin.auth_mode,
                plugin.auth_header.as_deref(),
                token.as_deref(),
                plugin.auth_profile_id.as_deref(),
            )
            .await
        {
            Ok(request) => request,
            Err(error) => {
                let detail = sanitize_plugin_message(&format!(
                    "failed to call plugin action '{}': {}",
                    action_name, error
                ));
                self.set_plugin_last_error(plugin_id, Some(detail.clone()))
                    .await?;
                self.append_log(
                    &plugin,
                    "action",
                    action_name,
                    "error",
                    Some(detail.clone()),
                )
                .await?;
                anyhow::bail!("{}", detail);
            }
        };
        let response = match request
            .json(&serde_json::json!({
                "sdk_version": PLUGIN_SDK_VERSION,
                "plugin_id": plugin.id,
                "plugin_name": plugin.name,
                "action": action.name,
                "arguments": arguments,
                "invoked_at": now_rfc3339(),
            }))
            .send()
            .await
        {
            Ok(response) => response,
            Err(error) => {
                let detail = sanitize_plugin_message(&format!(
                    "failed to call plugin action '{}': {}",
                    action_name, error
                ));
                self.set_plugin_last_error(plugin_id, Some(detail.clone()))
                    .await?;
                self.append_log(
                    &plugin,
                    "action",
                    action_name,
                    "error",
                    Some(detail.clone()),
                )
                .await?;
                anyhow::bail!("{}", detail);
            }
        };
        let status = response.status();
        let body = response.text().await.unwrap_or_default();
        if !status.is_success() {
            let message = sanitize_plugin_message(&body);
            self.set_plugin_last_error(plugin_id, Some(message.clone()))
                .await?;
            self.append_log(
                &plugin,
                "action",
                action_name,
                "error",
                Some(message.clone()),
            )
            .await?;
            anyhow::bail!("Plugin action '{}' failed: {}", action_name, message);
        }
        self.set_plugin_last_error(plugin_id, None).await?;
        self.append_log(
            &plugin,
            "action",
            action_name,
            "success",
            Some("Plugin action executed successfully.".to_string()),
        )
        .await?;
        if let Some(auth_profile_id) = plugin.auth_profile_id.as_deref() {
            let _ = crate::core::auth_profiles::AuthProfileControlPlane::mark_used(
                &self.storage,
                auth_profile_id,
            )
            .await;
        }
        Ok(format_plugin_result(&body))
    }

    pub async fn dispatch_event(&mut self, event_name: &str, payload: &Value) -> Result<()> {
        let eligible = self
            .plugins
            .values()
            .filter(|plugin| plugin.enabled)
            .filter(|plugin| {
                plugin
                    .subscribed_events
                    .iter()
                    .any(|item| item == event_name)
            })
            .cloned()
            .collect::<Vec<_>>();
        for plugin in eligible {
            self.enforce_plugin_rate_limit(&plugin.id).await;
            let mut event_payload = serde_json::Map::new();
            event_payload.insert(
                "sdk_version".to_string(),
                Value::String(PLUGIN_SDK_VERSION.to_string()),
            );
            event_payload.insert("plugin_id".to_string(), Value::String(plugin.id.clone()));
            event_payload.insert(
                "plugin_name".to_string(),
                Value::String(plugin.name.clone()),
            );
            event_payload.insert("event".to_string(), Value::String(event_name.to_string()));
            event_payload.insert("occurred_at".to_string(), Value::String(now_rfc3339()));
            let sanitized_payload = sanitize_plugin_event_payload(payload);
            if let Some(object) = sanitized_payload.as_object() {
                for (key, value) in object {
                    event_payload
                        .entry(key.clone())
                        .or_insert_with(|| value.clone());
                }
            }
            event_payload.insert("payload".to_string(), sanitized_payload);
            let token = self.load_plugin_secret(&plugin.id)?;
            let response: Result<reqwest::Response> = match self
                .build_authorized_request(
                    reqwest::Method::POST,
                    &plugin_endpoint(
                        &plugin.base_url,
                        &format!("/agentark/events/{}", event_name),
                    ),
                    plugin.auth_mode,
                    plugin.auth_header.as_deref(),
                    token.as_deref(),
                    plugin.auth_profile_id.as_deref(),
                )
                .await
            {
                Ok(request) => request
                    .json(&Value::Object(event_payload))
                    .send()
                    .await
                    .map_err(Into::into),
                Err(error) => Err(error),
            };
            match response {
                Ok(resp) if resp.status().is_success() => {
                    self.set_plugin_last_error(&plugin.id, None).await?;
                    if let Some(auth_profile_id) = plugin.auth_profile_id.as_deref() {
                        let _ = crate::core::auth_profiles::AuthProfileControlPlane::mark_used(
                            &self.storage,
                            auth_profile_id,
                        )
                        .await;
                    }
                    self.append_log(
                        &plugin,
                        "event",
                        event_name,
                        "success",
                        Some("Event delivered.".to_string()),
                    )
                    .await?;
                }
                Ok(resp) => {
                    let body = resp.text().await.unwrap_or_default();
                    let detail = sanitize_plugin_message(&body);
                    self.set_plugin_last_error(&plugin.id, Some(detail.clone()))
                        .await?;
                    self.append_log(&plugin, "event", event_name, "error", Some(detail))
                        .await?;
                }
                Err(error) => {
                    let detail = sanitize_plugin_message(&error.to_string());
                    self.set_plugin_last_error(&plugin.id, Some(detail.clone()))
                        .await?;
                    self.append_log(&plugin, "event", event_name, "error", Some(detail))
                        .await?;
                }
            }
        }
        Ok(())
    }

    async fn enforce_plugin_rate_limit(&mut self, plugin_id: &str) {
        if let Some(last_call_at) = self.last_call_at.get(plugin_id).copied() {
            let elapsed = last_call_at.elapsed();
            if elapsed < PLUGIN_MIN_CALL_INTERVAL {
                tokio::time::sleep(PLUGIN_MIN_CALL_INTERVAL - elapsed).await;
            }
        }
        self.last_call_at
            .insert(plugin_id.to_string(), Instant::now());
    }

    pub async fn ping_plugin(&mut self, id: &str) -> Result<PluginTestResult> {
        let plugin = self
            .plugins
            .get(id)
            .cloned()
            .ok_or_else(|| anyhow!("Plugin not found"))?;
        let token = self.load_plugin_secret(id)?;
        let request = match self
            .build_authorized_request(
                reqwest::Method::GET,
                &plugin_endpoint(&plugin.base_url, "/agentark/ping"),
                plugin.auth_mode,
                plugin.auth_header.as_deref(),
                token.as_deref(),
                plugin.auth_profile_id.as_deref(),
            )
            .await
        {
            Ok(request) => request,
            Err(error) => {
                let detail =
                    sanitize_plugin_message(&format!("failed to ping plugin '{}': {}", id, error));
                self.set_plugin_last_error(id, Some(detail.clone())).await?;
                self.append_log(&plugin, "ping", "Ping", "error", Some(detail.clone()))
                    .await?;
                return Err(anyhow!(detail));
            }
        };
        let response = match request.send().await {
            Ok(response) => response,
            Err(error) => {
                let detail =
                    sanitize_plugin_message(&format!("failed to ping plugin '{}': {}", id, error));
                self.set_plugin_last_error(id, Some(detail.clone())).await?;
                self.append_log(&plugin, "ping", "Ping", "error", Some(detail.clone()))
                    .await?;
                return Err(anyhow!(detail));
            }
        };
        let status = response.status();
        let detail = sanitize_plugin_message(&response.text().await.unwrap_or_default());
        if !status.is_success() {
            self.set_plugin_last_error(id, Some(detail.clone())).await?;
            self.append_log(&plugin, "ping", "Ping", "error", Some(detail.clone()))
                .await?;
            anyhow::bail!("Plugin ping failed: {}", detail);
        }
        self.set_plugin_last_error(id, None).await?;
        self.append_log(
            &plugin,
            "ping",
            "Ping",
            "success",
            Some("Plugin responded.".to_string()),
        )
        .await?;
        if let Some(auth_profile_id) = plugin.auth_profile_id.as_deref() {
            let _ = crate::core::auth_profiles::AuthProfileControlPlane::mark_used(
                &self.storage,
                auth_profile_id,
            )
            .await;
        }
        Ok(PluginTestResult {
            ok: true,
            detail: (!detail.is_empty()).then_some(detail),
        })
    }

    pub async fn list_logs(
        &self,
        plugin_id: Option<&str>,
        limit: usize,
    ) -> Result<Vec<PluginLogRecord>> {
        let mut logs = load_json::<Vec<PluginLogRecord>>(&self.storage, PLUGIN_LOGS_KEY).await?;
        if let Some(id) = plugin_id.map(str::trim).filter(|value| !value.is_empty()) {
            logs.retain(|entry| entry.plugin_id == id);
        }
        logs.sort_by(|left, right| right.created_at.cmp(&left.created_at));
        logs.truncate(limit.clamp(1, PLUGIN_LOG_HISTORY_LIMIT));
        Ok(logs)
    }

    async fn auth_profile_overlay(
        &self,
        auth_profile_id: Option<&str>,
    ) -> Result<Option<crate::core::auth_profiles::HttpAuthOverlay>> {
        let Some(auth_profile_id) = auth_profile_id
            .map(str::trim)
            .filter(|value| !value.is_empty())
        else {
            return Ok(None);
        };
        Ok(Some(
            crate::core::auth_profiles::AuthProfileControlPlane::resolve_http(
                &self.storage,
                auth_profile_id,
            )
            .await?
            .overlay,
        ))
    }

    async fn plugin_auth_configured(&self, plugin: &PluginConfig) -> Result<bool> {
        if let Some(auth_profile_id) = plugin
            .auth_profile_id
            .as_deref()
            .map(str::trim)
            .filter(|value| !value.is_empty())
        {
            Ok(crate::core::auth_profiles::AuthProfileControlPlane::get(
                &self.storage,
                auth_profile_id,
            )
            .await?
            .is_some_and(|profile| profile.ready))
        } else {
            self.plugin_secret_present(&plugin.id)
        }
    }

    async fn plugin_view(&self, plugin: &PluginConfig) -> Result<PluginView> {
        Ok(PluginView {
            token_configured: self.plugin_auth_configured(plugin).await?,
            available_events: plugin.manifest.events.clone(),
            registered_actions: plugin
                .manifest
                .actions
                .iter()
                .map(|action| plugin_action_runtime_name(&plugin.id, &action.name))
                .collect(),
            plugin: plugin.clone(),
        })
    }

    async fn build_authorized_request(
        &self,
        method: reqwest::Method,
        url: &str,
        auth_mode: PluginAuthMode,
        auth_header: Option<&str>,
        token: Option<&str>,
        auth_profile_id: Option<&str>,
    ) -> Result<reqwest::RequestBuilder> {
        let mut parsed = reqwest::Url::parse(url).context("invalid plugin endpoint URL")?;
        crate::core::net::validate_external_https_url(parsed.as_str()).await?;
        if let Some(overlay) = self.auth_profile_overlay(auth_profile_id).await? {
            overlay.apply_to_url(&mut parsed);
            crate::core::net::validate_external_https_url(parsed.as_str()).await?;
            let request = self.http_client.request(method, parsed);
            return overlay.apply_to_request_builder(request);
        }

        Ok(self
            .http_client
            .request(method, parsed)
            .headers(self.build_auth_headers(auth_mode, auth_header, token)?))
    }

    fn build_auth_headers(
        &self,
        auth_mode: PluginAuthMode,
        auth_header: Option<&str>,
        token: Option<&str>,
    ) -> Result<HeaderMap> {
        let mut headers = HeaderMap::new();
        match auth_mode {
            PluginAuthMode::None => {}
            PluginAuthMode::Bearer => {
                let token = token
                    .map(str::trim)
                    .filter(|value| !value.is_empty())
                    .ok_or_else(|| anyhow!("Plugin token is not configured"))?;
                let value = HeaderValue::from_str(&format!("Bearer {}", token))
                    .context("invalid bearer token header value")?;
                headers.insert(AUTHORIZATION, value);
            }
            PluginAuthMode::Header => {
                let token = token
                    .map(str::trim)
                    .filter(|value| !value.is_empty())
                    .ok_or_else(|| anyhow!("Plugin token is not configured"))?;
                let name = auth_header
                    .map(str::trim)
                    .filter(|value| !value.is_empty())
                    .ok_or_else(|| anyhow!("Custom header auth requires a header name"))?;
                let header_name =
                    HeaderName::from_bytes(name.as_bytes()).context("invalid auth header name")?;
                let value =
                    HeaderValue::from_str(token).context("invalid custom auth header value")?;
                headers.insert(header_name, value);
            }
        }
        Ok(headers)
    }

    async fn fetch_manifest(
        &self,
        base_url: &str,
        auth_mode: PluginAuthMode,
        auth_header: Option<&str>,
        token: Option<&str>,
        auth_profile_id: Option<&str>,
    ) -> Result<PluginManifest> {
        let request = self
            .build_authorized_request(
                reqwest::Method::GET,
                &plugin_endpoint(base_url, "/agentark/manifest"),
                auth_mode,
                auth_header,
                token,
                auth_profile_id,
            )
            .await
            .context("failed to build plugin manifest request")?;
        let response = request
            .send()
            .await
            .context("failed to fetch plugin manifest")?;
        let status = response.status();
        let body = read_response_text_limited(response, PLUGIN_MANIFEST_MAX_BYTES).await?;
        if !status.is_success() {
            anyhow::bail!(
                "Plugin manifest request failed with {}: {}",
                status,
                sanitize_plugin_message(&body)
            );
        }
        let manifest = serde_json::from_str::<PluginManifest>(&body)
            .context("plugin manifest is not valid JSON")?;
        validate_manifest(&manifest)?;
        Ok(manifest)
    }

    async fn persist_configs(&self) -> Result<()> {
        let configs = self.plugins.values().cloned().collect::<Vec<_>>();
        save_json(&self.storage, PLUGIN_CONFIGS_KEY, &configs).await
    }

    async fn set_plugin_last_error(
        &mut self,
        plugin_id: &str,
        value: Option<String>,
    ) -> Result<()> {
        let mut changed = false;
        if let Some(plugin) = self.plugins.get_mut(plugin_id) {
            plugin.last_error = value.map(|text| sanitize_plugin_message(&text));
            plugin.updated_at = now_rfc3339();
            changed = true;
        }
        if changed {
            self.persist_configs().await?;
        }
        Ok(())
    }

    async fn append_log(
        &self,
        plugin: &PluginConfig,
        kind: &str,
        subject: &str,
        outcome: &str,
        message: Option<String>,
    ) -> Result<()> {
        let mut logs = load_json::<Vec<PluginLogRecord>>(&self.storage, PLUGIN_LOGS_KEY).await?;
        logs.push(PluginLogRecord {
            id: uuid::Uuid::new_v4().to_string(),
            plugin_id: plugin.id.clone(),
            plugin_name: plugin.name.clone(),
            kind: kind.to_string(),
            subject: subject.to_string(),
            outcome: outcome.to_string(),
            message: message.map(|text| sanitize_plugin_message(&text)),
            created_at: now_rfc3339(),
        });
        if logs.len() > PLUGIN_LOG_HISTORY_LIMIT {
            let trim_to = logs.len() - PLUGIN_LOG_HISTORY_LIMIT;
            logs.drain(0..trim_to);
        }
        save_json(&self.storage, PLUGIN_LOGS_KEY, &logs).await
    }

    fn plugin_secret_present(&self, plugin_id: &str) -> Result<bool> {
        Ok(self
            .load_plugin_secret(plugin_id)?
            .is_some_and(|value| !value.trim().is_empty()))
    }

    fn load_plugin_secret(&self, plugin_id: &str) -> Result<Option<String>> {
        let manager = crate::core::config::SecureConfigManager::new_with_data_dir(
            &self.config_dir,
            Some(&self.data_dir),
        )?;
        manager.get_custom_secret(&plugin_secret_key(plugin_id))
    }

    fn set_plugin_secret(&self, plugin_id: &str, value: Option<String>) -> Result<()> {
        let manager = crate::core::config::SecureConfigManager::new_with_data_dir(
            &self.config_dir,
            Some(&self.data_dir),
        )?;
        manager.set_custom_secret(&plugin_secret_key(plugin_id), value)
    }
}

pub fn plugin_action_runtime_name(plugin_id: &str, action_name: &str) -> String {
    format!(
        "plugin__{}__{}",
        sanitize_plugin_id(plugin_id),
        sanitize_plugin_id(action_name)
    )
}

fn plugin_endpoint(base_url: &str, path: &str) -> String {
    format!("{}{}", base_url.trim_end_matches('/'), path)
}

fn plugin_secret_key(plugin_id: &str) -> String {
    format!("{}{}", PLUGIN_SECRET_PREFIX, plugin_id.trim())
}

async fn register_plugin_actions(
    runtime: &ActionRuntime,
    plugin: &PluginConfig,
    token_configured: bool,
) -> Result<()> {
    for action in &plugin.manifest.actions {
        let name = plugin_action_runtime_name(&plugin.id, &action.name);
        let description = if action.description.trim().is_empty() {
            format!("Plugin '{}' action '{}'.", plugin.name, action.name)
        } else {
            format!("Plugin '{}': {}", plugin.name, action.description.trim())
        };
        let info = ActionDef {
            name,
            description: crate::security::sanitize_untrusted_output(
                "plugin_manifest",
                &description,
            ),
            version: plugin.manifest.version.clone(),
            input_schema: if action.input_schema.is_null() {
                serde_json::json!({})
            } else {
                crate::security::sanitize_input_schema(&action.input_schema)
            },
            capabilities: crate::security::canonical_capabilities(&action.capabilities, true)?,
            sandbox_mode: Some(SandboxMode::Native),
            source: ActionSource::System,
            file_path: None,
            authorization: crate::actions::ActionAuthorization {
                outbound: crate::actions::ActionEgressPolicy {
                    read_only: action.read_only,
                    outbound_write: !action.read_only || action.outbound_write,
                    public_publish: action.public_publish,
                },
                ..Default::default()
            },
        };
        runtime
            .register_plugin_action(
                info,
                PluginBinding {
                    plugin_id: plugin.id.clone(),
                    action_name: action.name.clone(),
                    base_url: plugin.base_url.clone(),
                    auth_profile_id: plugin.auth_profile_id.clone(),
                    auth_required: plugin.auth_profile_id.is_some()
                        || !matches!(plugin.auth_mode, PluginAuthMode::None),
                    auth_configured: if plugin.auth_profile_id.is_some()
                        || !matches!(plugin.auth_mode, PluginAuthMode::None)
                    {
                        token_configured
                    } else {
                        true
                    },
                },
            )
            .await;
    }
    Ok(())
}

fn validate_manifest(manifest: &PluginManifest) -> Result<()> {
    if manifest.sdk_version.trim() != PLUGIN_SDK_VERSION {
        anyhow::bail!(
            "Unsupported plugin SDK version '{}'. Expected '{}'.",
            manifest.sdk_version.trim(),
            PLUGIN_SDK_VERSION
        );
    }
    if sanitize_plugin_id(&manifest.id).is_empty() {
        anyhow::bail!("Plugin manifest id is required");
    }
    if manifest.name.trim().is_empty() {
        anyhow::bail!("Plugin manifest name is required");
    }
    let mut names = HashSet::new();
    crate::security::scan_untrusted_text(&manifest.description);
    for action in &manifest.actions {
        let normalized = sanitize_plugin_id(&action.name);
        if normalized.is_empty() {
            anyhow::bail!("Plugin action names must be non-empty");
        }
        if !names.insert(normalized) {
            anyhow::bail!("Duplicate plugin action '{}'", action.name);
        }
        crate::security::canonical_capabilities(&action.capabilities, true).with_context(|| {
            format!(
                "Plugin action '{}' declares unsupported capabilities",
                action.name
            )
        })?;
        crate::security::sanitize_input_schema(&action.input_schema);
    }
    Ok(())
}

fn normalize_subscribed_events(
    requested: Option<&[String]>,
    existing: Option<&[String]>,
    supported: &[String],
) -> Vec<String> {
    let supported_set = supported
        .iter()
        .map(|value| value.trim().to_ascii_lowercase())
        .collect::<HashSet<_>>();
    let platform_set = PluginRegistry::platform_events()
        .into_iter()
        .map(|value| value.trim().to_ascii_lowercase())
        .collect::<HashSet<_>>();
    let source = requested.or(existing).unwrap_or(&[]);
    source
        .iter()
        .map(|value| value.trim().to_ascii_lowercase())
        .filter(|value| supported_set.contains(value) && platform_set.contains(value))
        .collect::<HashSet<_>>()
        .into_iter()
        .collect::<Vec<_>>()
}

async fn normalize_base_url(value: &str) -> Result<String> {
    let trimmed = value.trim().trim_end_matches('/');
    if trimmed.is_empty() {
        anyhow::bail!("Base URL is required");
    }
    let parsed = crate::core::net::validate_external_https_url(trimmed).await?;
    Ok(parsed.as_str().trim_end_matches('/').to_string())
}

fn sanitize_plugin_id(raw: &str) -> String {
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

fn normalize_header_name(value: Option<&str>) -> Option<String> {
    value
        .map(|entry| entry.trim().to_string())
        .filter(|entry| !entry.is_empty())
        .and_then(|entry| {
            let valid = entry
                .chars()
                .all(|ch| ch.is_ascii_alphanumeric() || ch == '-');
            valid.then_some(entry)
        })
}

fn clip_chars(value: &str, max_chars: usize) -> String {
    let trimmed = value.trim();
    if trimmed.chars().count() <= max_chars {
        trimmed.to_string()
    } else {
        format!("{}...", trimmed.chars().take(max_chars).collect::<String>())
    }
}

fn sanitize_plugin_message(value: &str) -> String {
    clip_chars(
        &crate::security::redact_secret_input(value).text,
        PLUGIN_MESSAGE_MAX_CHARS,
    )
}

fn now_rfc3339() -> String {
    chrono::Utc::now().to_rfc3339()
}

fn format_plugin_result(body: &str) -> String {
    let trimmed = body.trim();
    if trimmed.is_empty() {
        return "Plugin completed with no response body.".to_string();
    }
    if let Ok(value) = serde_json::from_str::<Value>(trimmed) {
        if let Some(text) = value
            .get("message")
            .and_then(|entry| entry.as_str())
            .map(str::trim)
            .filter(|text| !text.is_empty())
        {
            return crate::security::sanitize_untrusted_output("plugin", text);
        }
        if let Some(text) = value
            .get("result")
            .and_then(|entry| entry.as_str())
            .map(str::trim)
            .filter(|text| !text.is_empty())
        {
            return crate::security::sanitize_untrusted_output("plugin", text);
        }
        return crate::security::sanitize_untrusted_output(
            "plugin",
            &serde_json::to_string_pretty(&value).unwrap_or_else(|_| trimmed.to_string()),
        );
    }
    crate::security::sanitize_untrusted_output("plugin", trimmed)
}

fn plugin_manifest_hash(manifest: &PluginManifest) -> Result<String> {
    let bytes = serde_json::to_vec(manifest)?;
    Ok(hex::encode(sha2::Sha256::digest(&bytes)))
}

fn sanitize_plugin_event_payload(payload: &Value) -> Value {
    crate::security::redact_json_secrets(payload)
}

async fn read_response_text_limited(
    response: reqwest::Response,
    max_bytes: usize,
) -> Result<String> {
    if response
        .content_length()
        .is_some_and(|length| length as usize > max_bytes)
    {
        anyhow::bail!("Plugin response exceeded the maximum allowed size");
    }
    let mut stream = response.bytes_stream();
    let mut bytes = Vec::new();
    while let Some(chunk) = stream.next().await {
        let chunk = chunk.context("failed to read plugin response")?;
        bytes.extend_from_slice(&chunk);
        if bytes.len() > max_bytes {
            anyhow::bail!("Plugin response exceeded the maximum allowed size");
        }
    }
    String::from_utf8(bytes).context("plugin response is not valid UTF-8")
}

async fn load_json<T>(storage: &Storage, key: &str) -> Result<T>
where
    T: DeserializeOwned + Default,
{
    let Some(bytes) = storage.get_encrypted(key).await? else {
        return Ok(T::default());
    };
    serde_json::from_slice::<T>(&bytes)
        .with_context(|| format!("failed to decode plugin payload for {}", key))
}

async fn save_json<T>(storage: &Storage, key: &str, value: &T) -> Result<()>
where
    T: Serialize + ?Sized,
{
    let bytes = serde_json::to_vec(value).with_context(|| format!("failed to encode {}", key))?;
    storage.set_encrypted(key, &bytes).await
}
