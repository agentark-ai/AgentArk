use anyhow::{anyhow, Context, Result};
use serde::{Deserialize, Serialize};
use serde_json::{json, Map, Value};
use std::collections::BTreeMap;

use crate::actions::{ActionDef, ActionSource};
use crate::core::config::SecureConfigManager;
use crate::runtime::{ActionRuntime, CustomApiBinding, SandboxMode};
use crate::storage::Storage;

const CUSTOM_API_CONFIGS_KEY: &str = "custom_api:configs:v1";

#[derive(Debug, Clone, Copy, Serialize, Deserialize, PartialEq, Eq, Default)]
#[serde(rename_all = "snake_case")]
pub enum CustomApiAuthMode {
    #[default]
    None,
    Bearer,
    ApiKeyHeader,
    ApiKeyQuery,
    OAuth2,
    Basic,
}

#[derive(Debug, Clone, Copy, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "snake_case")]
pub enum CustomApiParameterLocation {
    Path,
    Query,
    Header,
    Body,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct CustomApiParameter {
    pub name: String,
    pub location: CustomApiParameterLocation,
    #[serde(default)]
    pub required: bool,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub description: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub schema_type: Option<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct CustomApiOperationDraft {
    pub id: String,
    pub name: String,
    pub method: String,
    pub path: String,
    #[serde(default)]
    pub description: String,
    #[serde(default)]
    pub read_only: bool,
    #[serde(default)]
    pub enabled: bool,
    #[serde(default)]
    pub default_headers: BTreeMap<String, String>,
    #[serde(default)]
    pub default_query: BTreeMap<String, String>,
    #[serde(default)]
    pub parameters: Vec<CustomApiParameter>,
    #[serde(default)]
    pub body_required: bool,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct CustomApiOperation {
    #[serde(flatten)]
    pub draft: CustomApiOperationDraft,
    pub action_name: String,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct CustomApiConfig {
    pub id: String,
    pub name: String,
    #[serde(default)]
    pub description: String,
    pub base_url: String,
    pub enabled: bool,
    pub auth_mode: CustomApiAuthMode,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub auth_profile_id: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub auth_header: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub auth_name: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub auth_username: Option<String>,
    pub created_at: String,
    pub updated_at: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub last_tested_at: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub last_test_outcome: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub last_test_message: Option<String>,
    #[serde(default)]
    pub operations: Vec<CustomApiOperation>,
}

#[derive(Debug, Clone, Serialize)]
pub struct CustomApiView {
    #[serde(flatten)]
    pub config: CustomApiConfig,
    pub secret_configured: bool,
    pub action_count: usize,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub test_action_name: Option<String>,
}

#[derive(Debug, Clone, Serialize)]
pub struct CustomApiPreview {
    pub suggested_id: String,
    pub suggested_name: String,
    pub base_url: String,
    pub auth_mode: CustomApiAuthMode,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub auth_header: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub auth_name: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub auth_username: Option<String>,
    pub operations: Vec<CustomApiOperationDraft>,
    #[serde(default)]
    pub notes: Vec<String>,
    pub source_kind: String,
}

#[derive(Debug, Deserialize)]
pub struct CustomApiPreviewRequest {
    #[serde(default)]
    pub name: Option<String>,
    #[serde(default)]
    pub base_url: Option<String>,
    #[serde(default)]
    pub openapi_url: Option<String>,
    #[serde(default)]
    pub openapi_text: Option<String>,
    #[serde(default)]
    pub curl_text: Option<String>,
}

#[derive(Debug, Deserialize)]
pub struct CustomApiUpsertRequest {
    #[serde(default)]
    pub id: Option<String>,
    pub name: String,
    #[serde(default)]
    pub description: Option<String>,
    pub base_url: String,
    #[serde(default)]
    pub enabled: Option<bool>,
    #[serde(default)]
    pub auth_mode: Option<CustomApiAuthMode>,
    #[serde(default)]
    pub auth_profile_id: Option<String>,
    #[serde(default)]
    pub auth_header: Option<String>,
    #[serde(default)]
    pub auth_name: Option<String>,
    #[serde(default)]
    pub auth_username: Option<String>,
    #[serde(default)]
    pub secret: Option<String>,
    #[serde(default)]
    pub clear_secret: Option<bool>,
    #[serde(default)]
    pub allow_missing_secret: Option<bool>,
    #[serde(default)]
    pub operations: Vec<CustomApiOperationDraft>,
}

#[derive(Debug, Serialize)]
pub struct CustomApiTestResult {
    pub ok: bool,
    pub action_name: String,
    pub detail: String,
}

#[derive(Debug, Clone)]
struct ParsedSource {
    suggested_name: String,
    suggested_id: String,
    base_url: String,
    auth_mode: CustomApiAuthMode,
    auth_header: Option<String>,
    auth_name: Option<String>,
    auth_username: Option<String>,
    operations: Vec<CustomApiOperationDraft>,
    notes: Vec<String>,
    source_kind: String,
}

pub fn custom_api_secret_key(api_id: &str) -> String {
    format!("custom_api_secret:{}", api_id.trim())
}

pub async fn list_custom_apis(
    storage: &Storage,
    config_dir: &std::path::Path,
    data_dir: &std::path::Path,
) -> Result<Vec<CustomApiView>> {
    let manager = SecureConfigManager::new_with_data_dir(config_dir, Some(data_dir))?;
    let mut rows = load_configs(storage).await?;
    rows.sort_by(|left, right| right.updated_at.cmp(&left.updated_at));
    let mut views = Vec::with_capacity(rows.len());
    for config in rows {
        let secret_configured = if let Some(auth_profile_id) = config.auth_profile_id.as_deref() {
            crate::core::auth_profiles::AuthProfileControlPlane::get(storage, auth_profile_id)
                .await?
                .is_some_and(|profile| profile.ready)
        } else {
            manager
                .get_custom_secret(&custom_api_secret_key(&config.id))
                .ok()
                .flatten()
                .is_some_and(|value| !value.trim().is_empty())
        };
        let test_action_name = find_testable_action(&config);
        let action_count = config
            .operations
            .iter()
            .filter(|op| op.draft.enabled)
            .count();
        views.push(CustomApiView {
            config,
            secret_configured,
            action_count,
            test_action_name,
        });
    }
    Ok(views)
}

pub async fn preview_custom_api(request: CustomApiPreviewRequest) -> Result<CustomApiPreview> {
    let parsed = parse_source(request).await?;
    Ok(CustomApiPreview {
        suggested_id: parsed.suggested_id,
        suggested_name: parsed.suggested_name,
        base_url: parsed.base_url,
        auth_mode: parsed.auth_mode,
        auth_header: parsed.auth_header,
        auth_name: parsed.auth_name,
        auth_username: parsed.auth_username,
        operations: parsed.operations,
        notes: parsed.notes,
        source_kind: parsed.source_kind,
    })
}

pub async fn upsert_custom_api(
    storage: &Storage,
    config_dir: &std::path::Path,
    data_dir: &std::path::Path,
    runtime: &ActionRuntime,
    request: CustomApiUpsertRequest,
    path_id: Option<&str>,
) -> Result<CustomApiView> {
    let name = request.name.trim();
    if name.is_empty() {
        anyhow::bail!("Name is required");
    }
    let base_url = request.base_url.trim().trim_end_matches('/').to_string();
    if base_url.is_empty() {
        anyhow::bail!("Base URL is required");
    }
    reqwest::Url::parse(&base_url).context("Base URL must be a valid absolute URL")?;
    if request.operations.is_empty() {
        anyhow::bail!("Select at least one endpoint to import.");
    }

    let mut configs = load_configs(storage).await?;
    let requested_id = path_id
        .map(str::to_string)
        .or_else(|| request.id.clone())
        .unwrap_or_else(|| sanitize_id(name));
    let id = if requested_id.trim().is_empty() {
        uuid::Uuid::new_v4().to_string()
    } else {
        sanitize_id(&requested_id)
    };
    let existing_index = configs.iter().position(|item| item.id == id);
    if path_id.is_some() && existing_index.is_none() {
        anyhow::bail!("Custom API not found");
    }
    if path_id.is_none() && existing_index.is_some() {
        anyhow::bail!("A custom API with that id already exists");
    }

    let now = chrono::Utc::now().to_rfc3339();
    let existing = existing_index.and_then(|index| configs.get(index).cloned());
    let created_at = existing
        .as_ref()
        .map(|item| item.created_at.clone())
        .unwrap_or_else(|| now.clone());
    let last_tested_at = existing
        .as_ref()
        .and_then(|item| item.last_tested_at.clone());
    let last_test_outcome = existing
        .as_ref()
        .and_then(|item| item.last_test_outcome.clone());
    let last_test_message = existing
        .as_ref()
        .and_then(|item| item.last_test_message.clone());

    let auth_mode = request.auth_mode.unwrap_or_else(|| {
        existing
            .as_ref()
            .map(|item| item.auth_mode)
            .unwrap_or_default()
    });
    let auth_profile_id = clean_optional_string(request.auth_profile_id.as_deref()).or_else(|| {
        existing
            .as_ref()
            .and_then(|item| item.auth_profile_id.clone())
    });
    let allow_missing_secret = request.allow_missing_secret.unwrap_or(false);
    if matches!(auth_mode, CustomApiAuthMode::OAuth2)
        && auth_profile_id.is_none()
        && !allow_missing_secret
    {
        anyhow::bail!(
            "OAuth2 custom APIs require an auth_profile_id bound to a real OAuth auth profile."
        );
    }
    if let Some(profile_id) = auth_profile_id.as_deref() {
        if crate::core::auth_profiles::AuthProfileControlPlane::get(storage, profile_id)
            .await?
            .is_none()
        {
            anyhow::bail!("Auth profile '{}' was not found.", profile_id);
        }
    }
    let auth_header = clean_optional_string(request.auth_header.as_deref())
        .or_else(|| existing.as_ref().and_then(|item| item.auth_header.clone()));
    let auth_name = clean_optional_string(request.auth_name.as_deref())
        .or_else(|| existing.as_ref().and_then(|item| item.auth_name.clone()));
    let auth_username = clean_optional_string(request.auth_username.as_deref()).or_else(|| {
        existing
            .as_ref()
            .and_then(|item| item.auth_username.clone())
    });

    let operations = request
        .operations
        .into_iter()
        .filter(|item| item.enabled)
        .map(|draft| CustomApiOperation {
            action_name: build_action_name(&id, &draft.id),
            draft: normalize_operation_draft(draft),
        })
        .collect::<Vec<_>>();
    if operations.is_empty() {
        anyhow::bail!("At least one enabled endpoint is required.");
    }

    let manager = SecureConfigManager::new_with_data_dir(config_dir, Some(data_dir))?;
    if request.clear_secret.unwrap_or(false) {
        manager.set_custom_secret(&custom_api_secret_key(&id), None)?;
    }
    if let Some(secret) = clean_optional_string(request.secret.as_deref()) {
        manager.set_custom_secret(&custom_api_secret_key(&id), Some(secret))?;
    }
    let secret_configured = manager
        .get_custom_secret(&custom_api_secret_key(&id))?
        .is_some_and(|value| !value.trim().is_empty());
    if auth_profile_id.is_none()
        && !matches!(auth_mode, CustomApiAuthMode::None)
        && !secret_configured
        && !allow_missing_secret
    {
        anyhow::bail!("This auth mode requires a secret or token.");
    }

    let config = CustomApiConfig {
        id: id.clone(),
        name: name.to_string(),
        description: request.description.unwrap_or_default().trim().to_string(),
        base_url,
        enabled: request.enabled.unwrap_or(true),
        auth_mode,
        auth_profile_id,
        auth_header,
        auth_name,
        auth_username,
        created_at,
        updated_at: now,
        last_tested_at,
        last_test_outcome,
        last_test_message,
        operations,
    };

    if let Some(index) = existing_index {
        configs[index] = config.clone();
    } else {
        configs.push(config.clone());
    }
    save_configs(storage, &configs).await?;
    sync_to_runtime(storage, config_dir, data_dir, runtime).await?;

    Ok(CustomApiView {
        secret_configured,
        action_count: config
            .operations
            .iter()
            .filter(|op| op.draft.enabled)
            .count(),
        test_action_name: find_testable_action(&config),
        config,
    })
}

pub async fn delete_custom_api(
    storage: &Storage,
    config_dir: &std::path::Path,
    data_dir: &std::path::Path,
    runtime: &ActionRuntime,
    id: &str,
) -> Result<()> {
    let mut configs = load_configs(storage).await?;
    let before = configs.len();
    configs.retain(|item| item.id != id);
    if configs.len() == before {
        anyhow::bail!("Custom API not found");
    }
    save_configs(storage, &configs).await?;
    let manager = SecureConfigManager::new_with_data_dir(config_dir, Some(data_dir))?;
    let _ = manager.set_custom_secret(&custom_api_secret_key(id), None);
    sync_to_runtime(storage, config_dir, data_dir, runtime).await
}

pub async fn test_custom_api(
    storage: &Storage,
    _config_dir: &std::path::Path,
    _data_dir: &std::path::Path,
    runtime: &ActionRuntime,
    id: &str,
) -> Result<CustomApiTestResult> {
    let mut configs = load_configs(storage).await?;
    let index = configs
        .iter()
        .position(|item| item.id == id)
        .ok_or_else(|| anyhow!("Custom API not found"))?;
    let config = configs[index].clone();
    let action_name = find_testable_action(&config)
        .ok_or_else(|| anyhow!("No safe test endpoint is available"))?;
    let tested_at = chrono::Utc::now().to_rfc3339();
    let execution = runtime.execute_action(&action_name, &json!({})).await;
    let (ok, detail) = match execution {
        Ok(detail) => (
            true,
            clip_chars(&crate::security::redact_secret_input(&detail).text, 1_500),
        ),
        Err(error) => (
            false,
            clip_chars(
                &crate::security::redact_secret_input(&error.to_string()).text,
                1_500,
            ),
        ),
    };

    configs[index].last_tested_at = Some(tested_at);
    configs[index].last_test_outcome = Some(if ok {
        "success".to_string()
    } else {
        "failure".to_string()
    });
    configs[index].last_test_message = Some(detail.clone());
    save_configs(storage, &configs).await?;

    if !ok {
        anyhow::bail!("{}", detail);
    }

    Ok(CustomApiTestResult {
        ok,
        action_name,
        detail,
    })
}

pub async fn sync_to_runtime(
    storage: &Storage,
    _config_dir: &std::path::Path,
    _data_dir: &std::path::Path,
    runtime: &ActionRuntime,
) -> Result<()> {
    runtime.unregister_custom_api_actions().await;
    let configs = load_configs(storage).await?;
    for config in configs.into_iter().filter(|item| item.enabled) {
        register_config(runtime, &config).await?;
    }
    Ok(())
}

async fn register_config(runtime: &ActionRuntime, config: &CustomApiConfig) -> Result<()> {
    for operation in config.operations.iter().filter(|op| op.draft.enabled) {
        let description = if operation.draft.description.trim().is_empty() {
            format!(
                "{} {} {}",
                config.name,
                operation.draft.method.to_ascii_uppercase(),
                operation.draft.path
            )
        } else {
            operation.draft.description.trim().to_string()
        };
        let capabilities = if operation.draft.read_only {
            vec!["network".to_string()]
        } else {
            vec!["network".to_string(), "external_write".to_string()]
        };
        runtime
            .register_custom_api_action(
                ActionDef {
                    name: operation.action_name.clone(),
                    description,
                    version: "1.0.0".to_string(),
                    input_schema: build_input_schema(operation),
                    capabilities,
                    sandbox_mode: Some(SandboxMode::Native),
                    source: ActionSource::System,
                    file_path: None,
                    authorization: crate::actions::ActionAuthorization {
                        outbound: crate::actions::ActionEgressPolicy {
                            read_only: operation.draft.read_only,
                            outbound_write: !operation.draft.read_only,
                            public_publish: false,
                        },
                        ..Default::default()
                    },
                },
                CustomApiBinding {
                    api_id: config.id.clone(),
                    api_name: config.name.clone(),
                    operation_id: operation.draft.id.clone(),
                    operation_name: operation.draft.name.clone(),
                    method: operation.draft.method.clone(),
                    base_url: config.base_url.clone(),
                    path: operation.draft.path.clone(),
                    read_only: operation.draft.read_only,
                    secret_key: custom_api_secret_key(&config.id),
                    auth_profile_id: config.auth_profile_id.clone(),
                    auth_mode: config.auth_mode,
                    auth_header: config.auth_header.clone(),
                    auth_name: config.auth_name.clone(),
                    auth_username: config.auth_username.clone(),
                    default_headers: operation.draft.default_headers.clone(),
                    default_query: operation.draft.default_query.clone(),
                    parameters: operation.draft.parameters.clone(),
                    body_required: operation.draft.body_required,
                },
            )
            .await;
    }
    Ok(())
}

fn build_input_schema(operation: &CustomApiOperation) -> Value {
    let mut properties = Map::new();
    let mut required = Vec::new();
    for parameter in &operation.draft.parameters {
        if matches!(parameter.location, CustomApiParameterLocation::Body) {
            continue;
        }
        let field_type = parameter
            .schema_type
            .as_deref()
            .unwrap_or("string")
            .to_ascii_lowercase();
        properties.insert(
            parameter.name.clone(),
            json!({
                "type": match field_type.as_str() {
                    "integer" => "integer",
                    "number" => "number",
                    "boolean" => "boolean",
                    _ => "string",
                },
                "description": parameter.description.clone().unwrap_or_else(|| {
                    format!("{:?} parameter", parameter.location).to_ascii_lowercase()
                })
            }),
        );
        if parameter.required {
            required.push(parameter.name.clone());
        }
    }
    if operation.draft.body_required
        || operation
            .draft
            .parameters
            .iter()
            .any(|param| matches!(param.location, CustomApiParameterLocation::Body))
    {
        properties.insert(
            "body".to_string(),
            json!({
                "type": "object",
                "description": "JSON request body for this endpoint"
            }),
        );
        if operation.draft.body_required {
            required.push("body".to_string());
        }
    }
    json!({
        "type": "object",
        "properties": properties,
        "required": required,
    })
}

async fn load_configs(storage: &Storage) -> Result<Vec<CustomApiConfig>> {
    let Some(bytes) = storage.get_encrypted(CUSTOM_API_CONFIGS_KEY).await? else {
        return Ok(Vec::new());
    };
    serde_json::from_slice::<Vec<CustomApiConfig>>(&bytes)
        .context("failed to decode custom API configs")
}

async fn save_configs(storage: &Storage, value: &[CustomApiConfig]) -> Result<()> {
    let bytes = serde_json::to_vec(value).context("failed to encode custom API configs")?;
    storage.set_encrypted(CUSTOM_API_CONFIGS_KEY, &bytes).await
}

async fn parse_source(request: CustomApiPreviewRequest) -> Result<ParsedSource> {
    if let Some(text) = clean_optional_string(request.curl_text.as_deref()) {
        return parse_curl_text(
            request.name.as_deref(),
            request.base_url.as_deref(),
            text.as_str(),
        );
    }
    let raw_spec = if let Some(text) = clean_optional_string(request.openapi_text.as_deref()) {
        text
    } else if let Some(url) = clean_optional_string(request.openapi_url.as_deref()) {
        let response = reqwest::Client::new()
            .get(url.as_str())
            .send()
            .await
            .with_context(|| format!("failed to fetch OpenAPI document from {}", url))?
            .error_for_status()
            .with_context(|| format!("failed to fetch OpenAPI document from {}", url))?;
        response
            .text()
            .await
            .context("failed to read OpenAPI response body")?
    } else {
        String::new()
    };
    if raw_spec.trim().is_empty() {
        anyhow::bail!("Paste an OpenAPI document or a sample curl command.");
    }
    parse_openapi_document(
        request.name.as_deref(),
        request.base_url.as_deref(),
        raw_spec.as_str(),
        request.openapi_url.as_deref(),
    )
}

fn parse_openapi_document(
    requested_name: Option<&str>,
    requested_base_url: Option<&str>,
    raw_spec: &str,
    source_url: Option<&str>,
) -> Result<ParsedSource> {
    let root: Value = serde_json::from_str(raw_spec)
        .or_else(|_| serde_yaml::from_str(raw_spec))
        .context("OpenAPI document must be valid JSON or YAML")?;
    let info = root.get("info").and_then(Value::as_object);
    let suggested_name = clean_optional_string(requested_name)
        .or_else(|| {
            info.and_then(|value| value.get("title"))
                .and_then(Value::as_str)
                .map(str::to_string)
        })
        .unwrap_or_else(|| "Imported API".to_string());
    let suggested_id = sanitize_id(&suggested_name);
    let base_url = clean_optional_string(requested_base_url)
        .or_else(|| {
            root.get("servers")
                .and_then(Value::as_array)
                .and_then(|items| items.first())
                .and_then(|item| item.get("url"))
                .and_then(Value::as_str)
                .map(str::to_string)
        })
        .or_else(|| {
            source_url.and_then(|raw| {
                reqwest::Url::parse(raw)
                    .ok()
                    .map(|url| format!("{}://{}", url.scheme(), url.host_str().unwrap_or_default()))
            })
        })
        .ok_or_else(|| {
            anyhow!("OpenAPI document must define a server URL or you must provide one.")
        })?;

    let (auth_mode, auth_header, auth_name, auth_username) = infer_openapi_auth(&root);
    let paths = root
        .get("paths")
        .and_then(Value::as_object)
        .ok_or_else(|| anyhow!("OpenAPI document is missing a top-level 'paths' object"))?;

    let mut operations = Vec::new();
    for (path, path_item) in paths {
        let path_item = resolve_refs(&root, path_item);
        let path_params = path_item
            .get("parameters")
            .and_then(Value::as_array)
            .cloned()
            .unwrap_or_default();
        for method in ["get", "post", "put", "patch", "delete"] {
            let Some(raw_operation) = path_item.get(method) else {
                continue;
            };
            let operation = resolve_refs(&root, raw_operation);
            let operation_id = operation
                .get("operationId")
                .and_then(Value::as_str)
                .map(str::to_string)
                .unwrap_or_else(|| format!("{}_{}", method, path));
            let name = operation
                .get("summary")
                .and_then(Value::as_str)
                .map(str::to_string)
                .or_else(|| {
                    operation
                        .get("operationId")
                        .and_then(Value::as_str)
                        .map(str::to_string)
                })
                .unwrap_or_else(|| format!("{} {}", method.to_ascii_uppercase(), path));
            let description = operation
                .get("description")
                .and_then(Value::as_str)
                .or_else(|| operation.get("summary").and_then(Value::as_str))
                .unwrap_or("")
                .to_string();
            let mut parameters = Vec::new();
            for raw_param in path_params.iter().chain(
                operation
                    .get("parameters")
                    .and_then(Value::as_array)
                    .into_iter()
                    .flatten(),
            ) {
                let parameter = resolve_refs(&root, raw_param);
                let Some(name) = parameter.get("name").and_then(Value::as_str) else {
                    continue;
                };
                let Some(location) = parameter.get("in").and_then(Value::as_str) else {
                    continue;
                };
                let location = match location {
                    "path" => CustomApiParameterLocation::Path,
                    "query" => CustomApiParameterLocation::Query,
                    "header" => CustomApiParameterLocation::Header,
                    _ => continue,
                };
                let schema = parameter
                    .get("schema")
                    .map(|value| resolve_refs(&root, value))
                    .unwrap_or(Value::Null);
                parameters.push(CustomApiParameter {
                    name: name.to_string(),
                    location,
                    required: parameter
                        .get("required")
                        .and_then(Value::as_bool)
                        .unwrap_or(matches!(location, CustomApiParameterLocation::Path)),
                    description: parameter
                        .get("description")
                        .and_then(Value::as_str)
                        .map(str::to_string),
                    schema_type: schema
                        .get("type")
                        .and_then(Value::as_str)
                        .map(str::to_string),
                });
            }

            let body_required = operation
                .get("requestBody")
                .map(|body| resolve_refs(&root, body))
                .and_then(|body| {
                    body.get("content")
                        .and_then(Value::as_object)
                        .map(|content| {
                            body.get("required")
                                .and_then(Value::as_bool)
                                .unwrap_or(false)
                                || content.contains_key("application/json")
                        })
                })
                .unwrap_or(false);
            if body_required {
                parameters.push(CustomApiParameter {
                    name: "body".to_string(),
                    location: CustomApiParameterLocation::Body,
                    required: true,
                    description: Some("JSON request body".to_string()),
                    schema_type: Some("object".to_string()),
                });
            }

            operations.push(normalize_operation_draft(CustomApiOperationDraft {
                id: sanitize_id(&operation_id),
                name,
                method: method.to_ascii_uppercase(),
                path: path.to_string(),
                description,
                read_only: matches!(method, "get"),
                enabled: true,
                default_headers: BTreeMap::new(),
                default_query: BTreeMap::new(),
                parameters,
                body_required,
            }));
        }
    }

    if operations.is_empty() {
        anyhow::bail!("No supported HTTP operations were discovered in the OpenAPI document.");
    }

    Ok(ParsedSource {
        suggested_name,
        suggested_id,
        base_url: base_url.trim_end_matches('/').to_string(),
        auth_mode,
        auth_header,
        auth_name,
        auth_username,
        operations,
        notes: vec![
            "Imported from OpenAPI. Review the selected endpoints before saving.".to_string(),
            "Imported tools become available in both chat and autonomous webhook/task runs."
                .to_string(),
        ],
        source_kind: "openapi".to_string(),
    })
}

fn parse_curl_text(
    requested_name: Option<&str>,
    requested_base_url: Option<&str>,
    curl_text: &str,
) -> Result<ParsedSource> {
    let tokens = tokenize_curl(curl_text);
    if tokens.is_empty() {
        anyhow::bail!("The sample curl command is empty.");
    }
    let mut method = "GET".to_string();
    let mut url = String::new();
    let mut headers = BTreeMap::new();
    let mut body = None::<String>;
    let mut idx = 0usize;
    while idx < tokens.len() {
        let token = tokens[idx].as_str();
        match token {
            "curl" => {}
            "-X" | "--request" => {
                if let Some(value) = tokens.get(idx + 1) {
                    method = value.to_ascii_uppercase();
                    idx += 1;
                }
            }
            "-H" | "--header" => {
                if let Some(value) = tokens.get(idx + 1) {
                    if let Some((key, raw_value)) = value.split_once(':') {
                        headers.insert(key.trim().to_string(), raw_value.trim().to_string());
                    }
                    idx += 1;
                }
            }
            "-d" | "--data" | "--data-raw" | "--data-binary" => {
                if let Some(value) = tokens.get(idx + 1) {
                    body = Some(value.to_string());
                    idx += 1;
                }
            }
            value if value.starts_with("http://") || value.starts_with("https://") => {
                url = value.to_string();
            }
            _ => {}
        }
        idx += 1;
    }
    if url.trim().is_empty() {
        anyhow::bail!("The sample curl command must contain an absolute URL.");
    }
    let parsed_url = reqwest::Url::parse(&url).context("Invalid curl URL")?;
    let base_url = clean_optional_string(requested_base_url).unwrap_or_else(|| {
        format!(
            "{}://{}",
            parsed_url.scheme(),
            parsed_url.host_str().unwrap_or_default()
        )
    });
    let path = if parsed_url.path().is_empty() {
        "/".to_string()
    } else {
        parsed_url.path().to_string()
    };
    let mut default_query = BTreeMap::new();
    for (key, value) in parsed_url.query_pairs() {
        default_query.insert(key.to_string(), value.to_string());
    }

    let mut auth_mode = CustomApiAuthMode::None;
    let mut auth_header = None;
    let mut auth_name = None;
    if let Some(authorization) = headers.get("Authorization").cloned() {
        if authorization.to_ascii_lowercase().starts_with("bearer ") {
            auth_mode = CustomApiAuthMode::Bearer;
            auth_header = Some("Authorization".to_string());
            headers.remove("Authorization");
        }
    }
    if matches!(auth_mode, CustomApiAuthMode::None) {
        for candidate in ["X-API-Key", "Api-Key", "X-Auth-Token"] {
            if headers.contains_key(candidate) {
                auth_mode = CustomApiAuthMode::ApiKeyHeader;
                auth_name = Some(candidate.to_string());
                headers.remove(candidate);
                break;
            }
        }
    }
    if matches!(auth_mode, CustomApiAuthMode::None) {
        for candidate in ["api_key", "apikey", "token"] {
            if default_query.contains_key(candidate) {
                auth_mode = CustomApiAuthMode::ApiKeyQuery;
                auth_name = Some(candidate.to_string());
                default_query.remove(candidate);
                break;
            }
        }
    }

    let suggested_name = clean_optional_string(requested_name)
        .unwrap_or_else(|| parsed_url.host_str().unwrap_or("Imported API").to_string());
    let suggested_id = sanitize_id(&suggested_name);
    let mut parameters = Vec::new();
    if body.is_some() {
        parameters.push(CustomApiParameter {
            name: "body".to_string(),
            location: CustomApiParameterLocation::Body,
            required: true,
            description: Some("JSON request body".to_string()),
            schema_type: Some("object".to_string()),
        });
    }

    Ok(ParsedSource {
        suggested_name,
        suggested_id,
        base_url: base_url.trim_end_matches('/').to_string(),
        auth_mode,
        auth_header,
        auth_name,
        auth_username: None,
        operations: vec![normalize_operation_draft(CustomApiOperationDraft {
            id: sanitize_id(&format!("{}_{}", method, path)),
            name: format!("{} {}", method, path),
            method,
            path,
            description: "Imported from sample curl command.".to_string(),
            read_only: body.is_none(),
            enabled: true,
            default_headers: headers,
            default_query,
            parameters,
            body_required: body.is_some(),
        })],
        notes: vec![
            "Imported from a curl example. Review the generated endpoint before saving."
                .to_string(),
        ],
        source_kind: "curl".to_string(),
    })
}

fn infer_openapi_auth(
    root: &Value,
) -> (
    CustomApiAuthMode,
    Option<String>,
    Option<String>,
    Option<String>,
) {
    let security_schemes = root
        .get("components")
        .and_then(|value| value.get("securitySchemes"))
        .and_then(Value::as_object);
    let Some(security_schemes) = security_schemes else {
        return (CustomApiAuthMode::None, None, None, None);
    };
    for scheme_value in security_schemes.values() {
        let scheme = resolve_refs(root, scheme_value);
        match scheme.get("type").and_then(Value::as_str).unwrap_or("") {
            "http" => {
                if scheme
                    .get("scheme")
                    .and_then(Value::as_str)
                    .unwrap_or("")
                    .eq_ignore_ascii_case("bearer")
                {
                    return (
                        CustomApiAuthMode::Bearer,
                        Some("Authorization".to_string()),
                        None,
                        None,
                    );
                }
            }
            "apiKey" => {
                let name = scheme
                    .get("name")
                    .and_then(Value::as_str)
                    .map(str::to_string);
                let location = scheme.get("in").and_then(Value::as_str).unwrap_or("");
                if location == "header" {
                    return (CustomApiAuthMode::ApiKeyHeader, None, name, None);
                }
                if location == "query" {
                    return (CustomApiAuthMode::ApiKeyQuery, None, name, None);
                }
            }
            "oauth2" => {
                return (
                    CustomApiAuthMode::OAuth2,
                    Some("Authorization".to_string()),
                    None,
                    None,
                );
            }
            _ => {}
        }
    }
    (CustomApiAuthMode::None, None, None, None)
}

fn resolve_refs(root: &Value, value: &Value) -> Value {
    let Some(reference) = value.get("$ref").and_then(Value::as_str) else {
        return value.clone();
    };
    if !reference.starts_with("#/") {
        return value.clone();
    }
    let mut current = root;
    for part in reference.trim_start_matches("#/").split('/') {
        let Some(next) = current.get(part) else {
            return value.clone();
        };
        current = next;
    }
    current.clone()
}

fn normalize_operation_draft(mut draft: CustomApiOperationDraft) -> CustomApiOperationDraft {
    draft.id = sanitize_id(&draft.id);
    draft.name = clean_optional_string(Some(draft.name.as_str()))
        .unwrap_or_else(|| format!("{} {}", draft.method, draft.path));
    draft.method = draft.method.trim().to_ascii_uppercase();
    if !draft.path.starts_with('/') {
        draft.path = format!("/{}", draft.path);
    }
    draft.description = draft.description.trim().to_string();
    if draft
        .parameters
        .iter()
        .any(|parameter| matches!(parameter.location, CustomApiParameterLocation::Body))
    {
        draft.body_required = true;
    }
    draft
}

fn build_action_name(api_id: &str, operation_id: &str) -> String {
    format!(
        "api__{}__{}",
        sanitize_id(api_id),
        sanitize_id(operation_id)
    )
}

fn sanitize_id(value: &str) -> String {
    value
        .trim()
        .to_ascii_lowercase()
        .chars()
        .map(|ch| {
            if ch.is_ascii_alphanumeric() || matches!(ch, '-' | '_') {
                ch
            } else {
                '-'
            }
        })
        .collect::<String>()
        .trim_matches('-')
        .to_string()
}

fn clean_optional_string(value: Option<&str>) -> Option<String> {
    value
        .map(str::trim)
        .filter(|value| !value.is_empty())
        .map(str::to_string)
}

fn find_testable_action(config: &CustomApiConfig) -> Option<String> {
    config
        .operations
        .iter()
        .find(|operation| {
            operation.draft.enabled
                && operation.draft.read_only
                && !operation.draft.body_required
                && operation.draft.parameters.iter().all(|parameter| {
                    if !parameter.required {
                        return true;
                    }
                    match parameter.location {
                        CustomApiParameterLocation::Query => {
                            operation.draft.default_query.contains_key(&parameter.name)
                        }
                        _ => false,
                    }
                })
        })
        .map(|operation| operation.action_name.clone())
}

fn tokenize_curl(raw: &str) -> Vec<String> {
    let mut tokens = Vec::new();
    let mut current = String::new();
    let mut quote = None::<char>;
    let mut chars = raw.chars().peekable();
    while let Some(ch) = chars.next() {
        if let Some(active_quote) = quote {
            if ch == active_quote {
                quote = None;
            } else if ch == '\\' {
                if let Some(next) = chars.next() {
                    current.push(next);
                }
            } else {
                current.push(ch);
            }
            continue;
        }
        if ch == '"' || ch == '\'' {
            quote = Some(ch);
            continue;
        }
        if ch.is_whitespace() {
            if !current.is_empty() {
                tokens.push(current.clone());
                current.clear();
            }
            continue;
        }
        current.push(ch);
    }
    if !current.is_empty() {
        tokens.push(current);
    }
    tokens
}

fn clip_chars(value: &str, max_chars: usize) -> String {
    let trimmed = value.trim();
    if trimmed.chars().count() <= max_chars {
        trimmed.to_string()
    } else {
        format!("{}...", trimmed.chars().take(max_chars).collect::<String>())
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::runtime::ActionRuntime;
    use crate::storage::Storage;

    #[tokio::test]
    async fn preview_openapi_discovers_operations() {
        let preview = preview_custom_api(CustomApiPreviewRequest {
            name: Some("GitHub Sample".to_string()),
            base_url: None,
            openapi_url: None,
            openapi_text: Some(
                r#"{
                  "openapi":"3.0.0",
                  "info":{"title":"GitHub Sample"},
                  "servers":[{"url":"https://api.example.com"}],
                  "paths":{
                    "/repos":{"get":{"operationId":"listRepos","summary":"List repos"}},
                    "/repos/{repo}/issues":{"post":{"operationId":"createIssue","summary":"Create issue","parameters":[{"name":"repo","in":"path","required":true,"schema":{"type":"string"}}],"requestBody":{"required":true,"content":{"application/json":{"schema":{"type":"object"}}}}}}
                  }
                }"#.to_string(),
            ),
            curl_text: None,
        })
        .await
        .expect("preview should parse");
        assert_eq!(preview.base_url, "https://api.example.com");
        assert_eq!(preview.operations.len(), 2);
        assert!(preview.operations.iter().any(|item| item.read_only));
        assert!(preview.operations.iter().any(|item| !item.read_only));
    }

    #[tokio::test]
    async fn preview_curl_discovers_auth_and_path() {
        let preview = preview_custom_api(CustomApiPreviewRequest {
            name: Some("Ops".to_string()),
            base_url: None,
            openapi_url: None,
            openapi_text: None,
            curl_text: Some(
                r#"curl -X POST https://api.example.com/v1/incidents -H "Authorization: Bearer abc" -H "Content-Type: application/json" -d "{\"title\":\"broken\"}""#.to_string(),
            ),
        })
        .await
        .expect("curl preview should parse");
        assert_eq!(preview.auth_mode, CustomApiAuthMode::Bearer);
        assert_eq!(preview.operations[0].path, "/v1/incidents");
        assert!(preview.operations[0].body_required);
    }

    #[tokio::test]
    async fn failed_custom_api_test_persists_failure_state() {
        let dir = tempfile::tempdir().expect("tempdir");
        let storage = Storage::connect(
            crate::storage::DatabaseConfig::for_tests().expect("test database config"),
        )
        .await
        .expect("storage");
        let runtime = ActionRuntime::new(dir.path(), dir.path())
            .await
            .expect("runtime");

        upsert_custom_api(
            &storage,
            dir.path(),
            dir.path(),
            &runtime,
            CustomApiUpsertRequest {
                id: Some("ops".to_string()),
                name: "Ops".to_string(),
                description: Some("Test API".to_string()),
                base_url: "http://127.0.0.1:9".to_string(),
                enabled: Some(true),
                auth_mode: Some(CustomApiAuthMode::None),
                auth_profile_id: None,
                auth_header: None,
                auth_name: None,
                auth_username: None,
                secret: None,
                clear_secret: None,
                operations: vec![CustomApiOperationDraft {
                    id: "health".to_string(),
                    name: "Health".to_string(),
                    method: "GET".to_string(),
                    path: "/health".to_string(),
                    description: "Health check".to_string(),
                    read_only: true,
                    enabled: true,
                    default_headers: std::collections::BTreeMap::new(),
                    default_query: std::collections::BTreeMap::new(),
                    parameters: Vec::new(),
                    body_required: false,
                }],
            },
            None,
        )
        .await
        .expect("custom API saved");

        let error = test_custom_api(&storage, dir.path(), dir.path(), &runtime, "ops")
            .await
            .expect_err("test should fail against an unreachable endpoint");
        assert!(!error.to_string().trim().is_empty());

        let apis = list_custom_apis(&storage, dir.path(), dir.path())
            .await
            .expect("custom APIs listed");
        let api = apis
            .into_iter()
            .find(|item| item.config.id == "ops")
            .expect("saved API present");
        assert_eq!(api.config.last_test_outcome.as_deref(), Some("failure"));
        assert!(api
            .config
            .last_test_message
            .as_deref()
            .is_some_and(|value| !value.trim().is_empty()));
        assert!(api.config.last_tested_at.is_some());
    }
}
