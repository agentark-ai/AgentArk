use anyhow::{Context, Result};
use reqwest::StatusCode;
use serde::{Deserialize, Serialize};
use serde_json::{json, Map, Value};
use std::path::{Component, Path, PathBuf};
use std::time::Duration;

pub const VERCEL_TOKEN_SECRET_KEY: &str = "vercel_token";
pub const VERCEL_TEAM_ID_SECRET_KEY: &str = "vercel_team_id";
pub const VERCEL_PROJECT_ID_SECRET_KEY: &str = "vercel_project_id";

const VERCEL_API_BASE: &str = "https://api.vercel.com";
const VERCEL_REQUEST_TIMEOUT_SECS: u64 = 45;
const VERCEL_DEPLOY_READY_TIMEOUT_SECS: u64 = 120;
const VERCEL_DEPLOY_READY_POLL_SECS: u64 = 3;
const MAX_VERCEL_FILE_COUNT: usize = 1_000;
const MAX_VERCEL_FILE_BYTES: u64 = 4 * 1024 * 1024;
const MAX_VERCEL_TOTAL_BYTES: u64 = 25 * 1024 * 1024;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ExternalDeployTarget {
    Local,
    VercelDirect,
    VercelGit,
}

impl ExternalDeployTarget {
    pub fn from_value(value: Option<&Value>) -> Self {
        let Some(raw) = value.and_then(|value| value.as_str()) else {
            return Self::Local;
        };
        match raw.trim().to_ascii_lowercase().replace('-', "_").as_str() {
            "vercel_direct" => Self::VercelDirect,
            "vercel_git" => Self::VercelGit,
            _ => Self::Local,
        }
    }

    pub fn as_str(self) -> &'static str {
        match self {
            Self::Local => "local",
            Self::VercelDirect => "vercel_direct",
            Self::VercelGit => "vercel_git",
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum VercelProjectMode {
    Auto,
    Existing,
    Create,
}

impl VercelProjectMode {
    pub fn from_value(value: Option<&Value>) -> Self {
        let Some(raw) = value.and_then(|value| value.as_str()) else {
            return Self::Auto;
        };
        match raw.trim().to_ascii_lowercase().as_str() {
            "existing" => Self::Existing,
            "create" => Self::Create,
            _ => Self::Auto,
        }
    }

    pub fn as_str(self) -> &'static str {
        match self {
            Self::Auto => "auto",
            Self::Existing => "existing",
            Self::Create => "create",
        }
    }
}

#[derive(Debug, Clone)]
pub struct ExternalDeployOptions {
    pub target: ExternalDeployTarget,
    pub production: bool,
    pub vercel_project_mode: VercelProjectMode,
    pub vercel_project_id: Option<String>,
    pub vercel_team_id: Option<String>,
    pub build_command: Option<String>,
    pub output_dir: Option<String>,
}

impl ExternalDeployOptions {
    pub fn from_arguments(arguments: &Value) -> Self {
        let target = ExternalDeployTarget::from_value(
            arguments
                .get("deploy_target")
                .or_else(|| arguments.get("external_deploy_target")),
        );
        Self {
            target,
            production: arguments
                .get("production")
                .and_then(|value| value.as_bool())
                .unwrap_or(false),
            vercel_project_mode: VercelProjectMode::from_value(
                arguments
                    .get("vercel_project_mode")
                    .or_else(|| arguments.get("project_mode")),
            ),
            vercel_project_id: optional_string(
                arguments
                    .get("vercel_project_id")
                    .or_else(|| arguments.get("project_id")),
            ),
            vercel_team_id: optional_string(
                arguments
                    .get("vercel_team_id")
                    .or_else(|| arguments.get("team_id")),
            ),
            build_command: optional_string(arguments.get("build_command")),
            output_dir: optional_string(arguments.get("output_dir")),
        }
    }
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct VercelConfigStatus {
    pub connected: bool,
    pub token_configured: bool,
    pub source: String,
    pub team_id: Option<String>,
    pub project_id: Option<String>,
    pub username: Option<String>,
    pub email: Option<String>,
    pub error: Option<String>,
}

#[derive(Debug, Clone)]
struct VercelCredentials {
    token: String,
    source: String,
    team_id: Option<String>,
    project_id: Option<String>,
}

#[derive(Debug, Clone)]
struct VercelCollectedFile {
    file: String,
    bytes: Vec<u8>,
}

#[derive(Debug, Clone, Serialize)]
struct VercelUploadedFile {
    file: String,
    sha: String,
    size: u64,
}

#[derive(Debug, Clone)]
struct VercelProjectSelection {
    identifier: String,
    id: Option<String>,
    name: String,
    created: bool,
}

pub fn store_vercel_config(
    config_dir: &Path,
    data_dir: &Path,
    token: Option<String>,
    team_id: Option<String>,
    project_id: Option<String>,
) -> Result<()> {
    let manager = crate::core::runtime::config::SecureConfigManager::new_with_data_dir(
        config_dir,
        Some(data_dir),
    )?;
    if let Some(token) = token.map(|value| value.trim().to_string()) {
        if !token.is_empty() {
            manager.set_custom_secret(VERCEL_TOKEN_SECRET_KEY, Some(token))?;
        }
    }
    set_optional_secret(&manager, VERCEL_TEAM_ID_SECRET_KEY, team_id)?;
    set_optional_secret(&manager, VERCEL_PROJECT_ID_SECRET_KEY, project_id)?;
    Ok(())
}

pub fn clear_vercel_config(config_dir: &Path, data_dir: &Path) -> Result<()> {
    let manager = crate::core::runtime::config::SecureConfigManager::new_with_data_dir(
        config_dir,
        Some(data_dir),
    )?;
    manager.set_custom_secret(VERCEL_TOKEN_SECRET_KEY, None)?;
    manager.set_custom_secret(VERCEL_TEAM_ID_SECRET_KEY, None)?;
    manager.set_custom_secret(VERCEL_PROJECT_ID_SECRET_KEY, None)?;
    Ok(())
}

pub fn load_vercel_config_values(config_dir: &Path, data_dir: &Path) -> Value {
    let creds = load_vercel_credentials(config_dir, data_dir).ok().flatten();
    json!({
        "token_configured": creds.as_ref().is_some_and(|value| !value.token.trim().is_empty()),
        "token_source": creds.as_ref().map(|value| value.source.as_str()).unwrap_or("none"),
        "team_id": creds.as_ref().and_then(|value| value.team_id.clone()).unwrap_or_default(),
        "project_id": creds.as_ref().and_then(|value| value.project_id.clone()).unwrap_or_default(),
    })
}

pub fn vercel_token_is_configured(config_dir: &Path, data_dir: &Path) -> bool {
    load_vercel_credentials(config_dir, data_dir)
        .ok()
        .flatten()
        .is_some_and(|creds| !creds.token.trim().is_empty())
}

pub async fn vercel_connection_status(config_dir: &Path, data_dir: &Path) -> VercelConfigStatus {
    let creds = match load_vercel_credentials(config_dir, data_dir) {
        Ok(Some(creds)) => creds,
        Ok(None) => {
            return VercelConfigStatus {
                connected: false,
                token_configured: false,
                source: "none".to_string(),
                team_id: None,
                project_id: None,
                username: None,
                email: None,
                error: None,
            };
        }
        Err(error) => {
            return VercelConfigStatus {
                connected: false,
                token_configured: false,
                source: "error".to_string(),
                team_id: None,
                project_id: None,
                username: None,
                email: None,
                error: Some(error.to_string()),
            };
        }
    };

    match test_vercel_token(&creds.token).await {
        Ok(payload) => {
            let user = payload.get("user").and_then(|value| value.as_object());
            VercelConfigStatus {
                connected: true,
                token_configured: true,
                source: creds.source,
                team_id: creds.team_id,
                project_id: creds.project_id,
                username: user
                    .and_then(|obj| obj.get("username"))
                    .and_then(|value| value.as_str())
                    .map(|value| value.to_string()),
                email: user
                    .and_then(|obj| obj.get("email"))
                    .and_then(|value| value.as_str())
                    .map(|value| value.to_string()),
                error: None,
            }
        }
        Err(error) => VercelConfigStatus {
            connected: false,
            token_configured: true,
            source: creds.source,
            team_id: creds.team_id,
            project_id: creds.project_id,
            username: None,
            email: None,
            error: Some(error),
        },
    }
}

pub async fn validate_vercel_token_value(token: &str) -> std::result::Result<(), String> {
    let token = token.trim();
    if token.is_empty() {
        return Err("Missing token".to_string());
    }
    test_vercel_token(token).await.map(|_| ())
}

pub async fn publish_app_to_external_target(
    config_dir: &Path,
    data_dir: &Path,
    app_id: &str,
    app_dir: &Path,
    app_meta: &Value,
    title: &str,
    options: &ExternalDeployOptions,
) -> Result<Option<Value>> {
    match options.target {
        ExternalDeployTarget::Local => Ok(None),
        ExternalDeployTarget::VercelDirect => {
            let result = publish_app_to_vercel_direct(
                config_dir, data_dir, app_id, app_dir, app_meta, title, options,
            )
            .await?;
            persist_external_deployment_meta(app_dir, "vercel", &result).await?;
            Ok(Some(result))
        }
        ExternalDeployTarget::VercelGit => {
            let creds = load_vercel_credentials(config_dir, data_dir).ok().flatten();
            let saved_project_id = creds.as_ref().and_then(|creds| creds.project_id.clone());
            let result = vercel_git_nudge(
                app_id,
                app_dir,
                app_meta,
                title,
                options,
                saved_project_id,
                creds.as_ref(),
            )
            .await;
            persist_external_deployment_meta(app_dir, "vercel", &result).await?;
            Ok(Some(result))
        }
    }
}

pub async fn load_app_meta(app_dir: &Path) -> Value {
    let path = app_dir.join(".app_meta.json");
    let mut meta = match tokio::fs::read(path).await {
        Ok(bytes) => serde_json::from_slice::<Value>(&bytes).unwrap_or_else(|_| json!({})),
        Err(_) => json!({}),
    };
    if !meta.is_object() {
        meta = json!({});
    }
    meta
}

async fn publish_app_to_vercel_direct(
    config_dir: &Path,
    data_dir: &Path,
    app_id: &str,
    app_dir: &Path,
    app_meta: &Value,
    title: &str,
    options: &ExternalDeployOptions,
) -> Result<Value> {
    let creds = match load_vercel_credentials(config_dir, data_dir)? {
        Some(creds) => creds,
        None => {
            return Ok(json!({
                "provider": "vercel",
                "deploy_target": ExternalDeployTarget::VercelDirect.as_str(),
                "project_mode": options.vercel_project_mode.as_str(),
                "status": "needs_auth",
                "app_id": app_id,
                "title": title,
                "message": "Connect Vercel with an access token before publishing this app.",
                "updated_at": chrono::Utc::now().to_rfc3339(),
            }));
        }
    };

    let files = collect_vercel_files(app_dir, app_meta).await?;
    let team_id = options.vercel_team_id.clone().or(creds.team_id);
    let project_id = options.vercel_project_id.clone().or(creds.project_id);
    let project_settings = project_settings_from_meta(app_meta, options);
    let client = reqwest::Client::builder()
        .timeout(Duration::from_secs(VERCEL_REQUEST_TIMEOUT_SECS))
        .build()?;
    let project_selection = match options.vercel_project_mode {
        VercelProjectMode::Existing if project_id.is_none() => {
            return Ok(json!({
                "provider": "vercel",
                "deploy_target": ExternalDeployTarget::VercelDirect.as_str(),
                "project_mode": options.vercel_project_mode.as_str(),
                "status": "needs_project",
                "app_id": app_id,
                "title": title,
                "message": "Select an existing Vercel project or switch project handling to auto/create.",
                "updated_at": chrono::Utc::now().to_rfc3339(),
            }));
        }
        VercelProjectMode::Create => {
            let project_name = project_id
                .as_deref()
                .map(|value| value.to_string())
                .unwrap_or_else(|| vercel_project_name(title, app_id));
            match create_vercel_project(
                &client,
                &creds.token,
                team_id.as_deref(),
                &project_name,
                &project_settings,
            )
            .await
            {
                Ok(selection) => Some(selection),
                Err((http_status, message, error_code)) => {
                    return Ok(json!({
                        "provider": "vercel",
                        "deploy_target": ExternalDeployTarget::VercelDirect.as_str(),
                        "project_mode": options.vercel_project_mode.as_str(),
                        "status": "error",
                        "app_id": app_id,
                        "title": title,
                        "http_status": http_status,
                        "message": message,
                        "error_code": error_code,
                        "updated_at": chrono::Utc::now().to_rfc3339(),
                    }));
                }
            }
        }
        _ => project_id.as_ref().map(|project| VercelProjectSelection {
            identifier: project.clone(),
            id: None,
            name: project.clone(),
            created: false,
        }),
    };
    let deployment_name = project_selection
        .as_ref()
        .map(|selection| selection.name.clone())
        .unwrap_or_else(|| vercel_project_name(title, app_id));

    let uploaded_files =
        match upload_vercel_files(&client, &creds.token, team_id.as_deref(), &files).await {
            Ok(uploaded_files) => uploaded_files,
            Err((http_status, message, error_code)) => {
                return Ok(json!({
                    "provider": "vercel",
                    "deploy_target": ExternalDeployTarget::VercelDirect.as_str(),
                    "project_mode": options.vercel_project_mode.as_str(),
                    "status": "error",
                    "app_id": app_id,
                    "title": title,
                    "http_status": http_status,
                    "message": message,
                    "error_code": error_code,
                    "updated_at": chrono::Utc::now().to_rfc3339(),
                }));
            }
        };

    let mut body = Map::new();
    body.insert("name".to_string(), Value::String(deployment_name.clone()));
    body.insert("files".to_string(), serde_json::to_value(&uploaded_files)?);
    body.insert(
        "meta".to_string(),
        json!({
            "agentark_app_id": app_id,
            "agentark_title": title,
            "agentark_deploy_target": ExternalDeployTarget::VercelDirect.as_str(),
        }),
    );
    if let Some(project) = project_selection
        .as_ref()
        .map(|selection| selection.identifier.as_str())
        .filter(|value| !value.trim().is_empty())
    {
        body.insert("project".to_string(), Value::String(project.to_string()));
    }
    if options.production {
        body.insert(
            "target".to_string(),
            Value::String("production".to_string()),
        );
    }

    body.insert(
        "projectSettings".to_string(),
        Value::Object(project_settings.clone()),
    );

    let mut url = url::Url::parse(&format!("{}/v13/deployments", VERCEL_API_BASE))?;
    {
        let mut pairs = url.query_pairs_mut();
        pairs.append_pair("forceNew", "1");
        pairs.append_pair("skipAutoDetectionConfirmation", "1");
        if let Some(team_id) = team_id.as_deref().filter(|value| !value.trim().is_empty()) {
            pairs.append_pair("teamId", team_id);
        }
    }

    let response = client
        .post(url)
        .bearer_auth(&creds.token)
        .json(&Value::Object(body))
        .send()
        .await;
    let response = match response {
        Ok(response) => response,
        Err(error) => {
            return Ok(json!({
                "provider": "vercel",
                "deploy_target": ExternalDeployTarget::VercelDirect.as_str(),
                "project_mode": options.vercel_project_mode.as_str(),
                "status": "error",
                "app_id": app_id,
                "title": title,
                "message": format!("Vercel deployment request failed: {}", error),
                "updated_at": chrono::Utc::now().to_rfc3339(),
            }));
        }
    };

    let status = response.status();
    let payload = response_json_or_text(response).await;
    if !status.is_success() {
        return Ok(json!({
            "provider": "vercel",
            "deploy_target": ExternalDeployTarget::VercelDirect.as_str(),
            "project_mode": options.vercel_project_mode.as_str(),
            "status": "error",
            "app_id": app_id,
            "title": title,
            "http_status": status.as_u16(),
            "message": vercel_error_message(status, &payload),
            "error_code": vercel_error_code(&payload).unwrap_or_default(),
            "updated_at": chrono::Utc::now().to_rfc3339(),
        }));
    }

    let deployment_id = payload
        .get("id")
        .and_then(|value| value.as_str())
        .unwrap_or_default()
        .to_string();
    let raw_url = payload
        .get("url")
        .and_then(|value| value.as_str())
        .unwrap_or_default()
        .trim();
    let deployment_url = absolute_vercel_url(raw_url);
    let readiness = wait_for_vercel_deployment(
        &client,
        &creds.token,
        team_id.as_deref(),
        if deployment_id.trim().is_empty() {
            raw_url
        } else {
            deployment_id.as_str()
        },
    )
    .await;
    let ready_state = readiness
        .as_ref()
        .and_then(|value| deployment_ready_state(value))
        .or_else(|| deployment_ready_state(&payload))
        .unwrap_or("UNKNOWN");
    if matches!(ready_state, "ERROR" | "CANCELED") {
        let diagnostic = readiness.as_ref().unwrap_or(&payload);
        return Ok(json!({
            "provider": "vercel",
            "deploy_target": ExternalDeployTarget::VercelDirect.as_str(),
            "project_mode": options.vercel_project_mode.as_str(),
            "status": "error",
            "app_id": app_id,
            "title": title,
            "deployment_id": deployment_id,
            "url": deployment_url,
            "target": if options.production { "production" } else { "preview" },
            "project_id": payload_deployment_project_id(diagnostic).or_else(|| payload_deployment_project_id(&payload)).or_else(|| project_selection.as_ref().and_then(|selection| selection.id.as_deref())).or(project_id.as_deref()).unwrap_or_default(),
            "project_name": diagnostic.get("name").and_then(|value| value.as_str()).or_else(|| payload.get("name").and_then(|value| value.as_str())).unwrap_or(deployment_name.as_str()),
            "ready_state": ready_state,
            "message": deployment_failure_message(diagnostic).unwrap_or_else(|| format!("Vercel deployment finished with state {}", ready_state)),
            "file_count": files.len(),
            "updated_at": chrono::Utc::now().to_rfc3339(),
        }));
    }
    let status_label = if ready_state == "READY" {
        "deployed"
    } else {
        "building"
    };
    Ok(json!({
        "provider": "vercel",
        "deploy_target": ExternalDeployTarget::VercelDirect.as_str(),
        "project_mode": options.vercel_project_mode.as_str(),
        "status": status_label,
        "app_id": app_id,
        "title": title,
        "deployment_id": deployment_id,
        "url": deployment_url,
        "target": if options.production { "production" } else { "preview" },
        "project_id": readiness.as_ref().and_then(payload_deployment_project_id).or_else(|| payload_deployment_project_id(&payload)).or_else(|| project_selection.as_ref().and_then(|selection| selection.id.as_deref())).or(project_id.as_deref()).unwrap_or_default(),
        "project_name": readiness.as_ref().and_then(|value| value.get("name").and_then(|value| value.as_str())).or_else(|| payload.get("name").and_then(|value| value.as_str())).unwrap_or(deployment_name.as_str()),
        "project_created": project_selection.as_ref().is_some_and(|selection| selection.created),
        "ready_state": ready_state,
        "message": if ready_state == "READY" { "Vercel deployment is ready." } else { "Vercel accepted the deployment; readiness polling timed out before the deployment reached a terminal state." },
        "team_id": team_id.unwrap_or_default(),
        "file_count": files.len(),
        "updated_at": chrono::Utc::now().to_rfc3339(),
    }))
}

async fn vercel_git_nudge(
    app_id: &str,
    app_dir: &Path,
    app_meta: &Value,
    title: &str,
    options: &ExternalDeployOptions,
    saved_project_id: Option<String>,
    credentials: Option<&VercelCredentials>,
) -> Value {
    let git_remote = find_git_remote_url(app_dir)
        .await
        .or_else(|| {
            app_meta
                .get("repo_url")
                .and_then(|value| value.as_str())
                .map(|value| value.to_string())
        })
        .map(|value| sanitize_remote_url(&value))
        .filter(|value| !value.trim().is_empty());
    let project_id = options.vercel_project_id.clone().or(saved_project_id);
    let project_configured = project_id
        .as_deref()
        .is_some_and(|value| !value.trim().is_empty());

    if let Some(project) = project_id
        .as_deref()
        .map(str::trim)
        .filter(|value| !value.is_empty())
    {
        if let Some(creds) = credentials {
            let client = match reqwest::Client::builder()
                .timeout(Duration::from_secs(15))
                .build()
            {
                Ok(client) => client,
                Err(error) => {
                    return json!({
                        "provider": "vercel",
                        "deploy_target": ExternalDeployTarget::VercelGit.as_str(),
                        "status": "error",
                        "app_id": app_id,
                        "title": title,
                        "message": format!("HTTP client error: {}", error),
                        "updated_at": chrono::Utc::now().to_rfc3339(),
                    });
                }
            };
            let team_id = options
                .vercel_team_id
                .as_deref()
                .or(creds.team_id.as_deref());
            match find_vercel_project(&client, &creds.token, team_id, project).await {
                Ok(project_payload) => {
                    if vercel_project_has_git_link(&project_payload) {
                        return json!({
                            "provider": "vercel",
                            "deploy_target": ExternalDeployTarget::VercelGit.as_str(),
                            "status": "vercel_project_linked",
                            "app_id": app_id,
                            "title": title,
                            "git_remote_configured": git_remote.is_some(),
                            "vercel_project_configured": true,
                            "vercel_project_id": payload_project_id(&project_payload).unwrap_or(project),
                            "vercel_project_name": project_payload.get("name").and_then(|value| value.as_str()).unwrap_or(project),
                            "message": "The selected Vercel project is already connected to Git in Vercel. Push changes to that connected repository, or use direct Vercel publish when AgentArk should deploy the current app bundle immediately.",
                            "updated_at": chrono::Utc::now().to_rfc3339(),
                        });
                    }
                }
                Err((http_status, message, error_code)) => {
                    return json!({
                        "provider": "vercel",
                        "deploy_target": ExternalDeployTarget::VercelGit.as_str(),
                        "status": "error",
                        "app_id": app_id,
                        "title": title,
                        "http_status": http_status,
                        "message": message,
                        "error_code": error_code,
                        "updated_at": chrono::Utc::now().to_rfc3339(),
                    });
                }
            }
        } else if git_remote.is_none() {
            return json!({
                "provider": "vercel",
                "deploy_target": ExternalDeployTarget::VercelGit.as_str(),
                "status": "needs_auth",
                "app_id": app_id,
                "title": title,
                "git_remote_configured": false,
                "vercel_project_configured": true,
                "message": "Connect Vercel so AgentArk can inspect whether the selected project is already Git-linked in Vercel, or use direct Vercel publish.",
                "updated_at": chrono::Utc::now().to_rfc3339(),
            });
        }
    }

    if git_remote.is_none() || !project_configured {
        return json!({
            "provider": "vercel",
            "deploy_target": ExternalDeployTarget::VercelGit.as_str(),
            "status": "needs_git",
            "app_id": app_id,
            "title": title,
            "git_remote_configured": git_remote.is_some(),
            "vercel_project_configured": project_configured,
            "message": "Vercel Git deployment needs a Git remote and a connected Vercel project. Configure Git/Vercel or use direct Vercel deployment.",
            "updated_at": chrono::Utc::now().to_rfc3339(),
        });
    }

    json!({
        "provider": "vercel",
        "deploy_target": ExternalDeployTarget::VercelGit.as_str(),
        "status": "needs_git_push",
        "app_id": app_id,
        "title": title,
        "git_remote": git_remote.unwrap_or_default(),
        "vercel_project_id": project_id.unwrap_or_default(),
        "message": "Git-backed Vercel deployment is configured. Commit and push the app changes to let Vercel deploy from the connected repository.",
        "updated_at": chrono::Utc::now().to_rfc3339(),
    })
}

async fn find_vercel_project(
    client: &reqwest::Client,
    token: &str,
    team_id: Option<&str>,
    id_or_name: &str,
) -> std::result::Result<Value, (u16, String, String)> {
    let mut url =
        url::Url::parse(&format!("{}/v9/projects/", VERCEL_API_BASE)).map_err(|error| {
            (
                0,
                format!("Failed to build Vercel project lookup URL: {}", error),
                String::new(),
            )
        })?;
    url.path_segments_mut()
        .map_err(|_| {
            (
                0,
                "Failed to build Vercel project lookup URL".to_string(),
                String::new(),
            )
        })?
        .push(id_or_name);
    if let Some(team_id) = team_id.filter(|value| !value.trim().is_empty()) {
        url.query_pairs_mut().append_pair("teamId", team_id);
    }

    let response = client
        .get(url)
        .bearer_auth(token)
        .send()
        .await
        .map_err(|error| {
            (
                0,
                format!("Vercel project lookup request failed: {}", error),
                String::new(),
            )
        })?;
    let status = response.status();
    let payload = response_json_or_text(response).await;
    if status.is_success() {
        Ok(payload)
    } else {
        Err((
            status.as_u16(),
            vercel_error_message(status, &payload),
            vercel_error_code(&payload).unwrap_or_default(),
        ))
    }
}

async fn upload_vercel_files(
    client: &reqwest::Client,
    token: &str,
    team_id: Option<&str>,
    files: &[VercelCollectedFile],
) -> std::result::Result<Vec<VercelUploadedFile>, (u16, String, String)> {
    let mut uploaded = Vec::with_capacity(files.len());
    for file in files {
        let sha = vercel_sha1_hex(&file.bytes);
        let size = file.bytes.len() as u64;
        let mut url =
            url::Url::parse(&format!("{}/v2/files", VERCEL_API_BASE)).map_err(|error| {
                (
                    0,
                    format!("Failed to build Vercel file upload URL: {}", error),
                    String::new(),
                )
            })?;
        if let Some(team_id) = team_id.filter(|value| !value.trim().is_empty()) {
            url.query_pairs_mut().append_pair("teamId", team_id);
        }

        let response = client
            .post(url)
            .bearer_auth(token)
            .header("Content-Length", size.to_string())
            .header("Content-Type", "application/octet-stream")
            .header("x-vercel-digest", &sha)
            .body(file.bytes.clone())
            .send()
            .await
            .map_err(|error| {
                (
                    0,
                    format!(
                        "Vercel file upload request failed for '{}': {}",
                        file.file, error
                    ),
                    String::new(),
                )
            })?;
        let status = response.status();
        let payload = response_json_or_text(response).await;
        if !status.is_success() {
            return Err((
                status.as_u16(),
                format!(
                    "Vercel file upload failed for '{}': {}",
                    file.file,
                    vercel_error_message(status, &payload)
                ),
                vercel_error_code(&payload).unwrap_or_default(),
            ));
        }

        uploaded.push(VercelUploadedFile {
            file: file.file.clone(),
            sha,
            size,
        });
    }
    Ok(uploaded)
}

fn vercel_sha1_hex(bytes: &[u8]) -> String {
    let digest = ring::digest::digest(&ring::digest::SHA1_FOR_LEGACY_USE_ONLY, bytes);
    hex::encode(digest.as_ref())
}

async fn wait_for_vercel_deployment(
    client: &reqwest::Client,
    token: &str,
    team_id: Option<&str>,
    id_or_url: &str,
) -> Option<Value> {
    let id_or_url = id_or_url.trim();
    if id_or_url.is_empty() {
        return None;
    }
    let deadline =
        tokio::time::Instant::now() + Duration::from_secs(VERCEL_DEPLOY_READY_TIMEOUT_SECS);
    let mut latest = None;
    loop {
        match get_vercel_deployment(client, token, team_id, id_or_url).await {
            Ok(payload) => {
                let state = deployment_ready_state(&payload)
                    .unwrap_or("UNKNOWN")
                    .to_string();
                latest = Some(payload);
                if matches!(state.as_str(), "READY" | "ERROR" | "CANCELED") {
                    return latest;
                }
            }
            Err((_, message, _)) => {
                tracing::warn!("Vercel deployment readiness check failed: {}", message);
            }
        }
        if tokio::time::Instant::now() >= deadline {
            return latest;
        }
        tokio::time::sleep(Duration::from_secs(VERCEL_DEPLOY_READY_POLL_SECS)).await;
    }
}

async fn get_vercel_deployment(
    client: &reqwest::Client,
    token: &str,
    team_id: Option<&str>,
    id_or_url: &str,
) -> std::result::Result<Value, (u16, String, String)> {
    let mut url =
        url::Url::parse(&format!("{}/v13/deployments/", VERCEL_API_BASE)).map_err(|error| {
            (
                0,
                format!("Failed to build Vercel deployment lookup URL: {}", error),
                String::new(),
            )
        })?;
    url.path_segments_mut()
        .map_err(|_| {
            (
                0,
                "Failed to build Vercel deployment lookup URL".to_string(),
                String::new(),
            )
        })?
        .push(id_or_url);
    if let Some(team_id) = team_id.filter(|value| !value.trim().is_empty()) {
        url.query_pairs_mut().append_pair("teamId", team_id);
    }

    let response = client
        .get(url)
        .bearer_auth(token)
        .send()
        .await
        .map_err(|error| {
            (
                0,
                format!("Vercel deployment lookup request failed: {}", error),
                String::new(),
            )
        })?;
    let status = response.status();
    let payload = response_json_or_text(response).await;
    if status.is_success() {
        Ok(payload)
    } else {
        Err((
            status.as_u16(),
            vercel_error_message(status, &payload),
            vercel_error_code(&payload).unwrap_or_default(),
        ))
    }
}

fn deployment_ready_state(payload: &Value) -> Option<&str> {
    payload
        .get("readyState")
        .or_else(|| payload.get("state"))
        .or_else(|| payload.get("status"))
        .and_then(|value| value.as_str())
        .map(str::trim)
        .filter(|value| !value.is_empty())
}

fn deployment_failure_message(payload: &Value) -> Option<String> {
    for key in ["errorMessage", "error", "message"] {
        if let Some(text) = payload.get(key).and_then(|value| value.as_str()) {
            let text = text.trim();
            if !text.is_empty() {
                return Some(text.to_string());
            }
        }
        if let Some(obj) = payload.get(key).and_then(|value| value.as_object()) {
            if let Some(text) = obj.get("message").and_then(|value| value.as_str()) {
                let text = text.trim();
                if !text.is_empty() {
                    return Some(text.to_string());
                }
            }
        }
    }
    None
}

fn vercel_project_has_git_link(payload: &Value) -> bool {
    payload
        .get("link")
        .and_then(|value| value.as_object())
        .is_some_and(|link| {
            link.get("repo")
                .and_then(|value| value.as_str())
                .is_some_and(|value| !value.trim().is_empty())
                || link
                    .get("gitCredentialId")
                    .and_then(|value| value.as_str())
                    .is_some_and(|value| !value.trim().is_empty())
                || link
                    .get("deployHooks")
                    .and_then(|value| value.as_array())
                    .is_some_and(|hooks| !hooks.is_empty())
        })
}

async fn create_vercel_project(
    client: &reqwest::Client,
    token: &str,
    team_id: Option<&str>,
    project_name: &str,
    project_settings: &Map<String, Value>,
) -> std::result::Result<VercelProjectSelection, (u16, String, String)> {
    let mut body = Map::new();
    body.insert("name".to_string(), Value::String(project_name.to_string()));
    for key in ["buildCommand", "installCommand", "outputDirectory"] {
        if let Some(value) = project_settings.get(key) {
            body.insert(key.to_string(), value.clone());
        }
    }

    let mut url =
        url::Url::parse(&format!("{}/v11/projects", VERCEL_API_BASE)).map_err(|error| {
            (
                0,
                format!("Failed to build Vercel project request URL: {}", error),
                String::new(),
            )
        })?;
    if let Some(team_id) = team_id.filter(|value| !value.trim().is_empty()) {
        url.query_pairs_mut().append_pair("teamId", team_id);
    }

    let response = client
        .post(url)
        .bearer_auth(token)
        .json(&Value::Object(body))
        .send()
        .await
        .map_err(|error| {
            (
                0,
                format!("Vercel project creation request failed: {}", error),
                String::new(),
            )
        })?;
    let status = response.status();
    let payload = response_json_or_text(response).await;
    if status.is_success() {
        let id = payload
            .get("id")
            .and_then(|value| value.as_str())
            .map(|value| value.to_string());
        let name = payload
            .get("name")
            .and_then(|value| value.as_str())
            .map(|value| value.to_string())
            .unwrap_or_else(|| project_name.to_string());
        return Ok(VercelProjectSelection {
            identifier: id.clone().unwrap_or_else(|| name.clone()),
            id,
            name,
            created: true,
        });
    }

    if status == StatusCode::CONFLICT {
        let existing = find_vercel_project(client, token, team_id, project_name).await?;
        let id = existing
            .get("id")
            .and_then(|value| value.as_str())
            .map(|value| value.to_string());
        let name = existing
            .get("name")
            .and_then(|value| value.as_str())
            .map(|value| value.to_string())
            .unwrap_or_else(|| project_name.to_string());
        return Ok(VercelProjectSelection {
            identifier: id.clone().unwrap_or_else(|| name.clone()),
            id,
            name,
            created: false,
        });
    }

    Err((
        status.as_u16(),
        vercel_error_message(status, &payload),
        vercel_error_code(&payload).unwrap_or_default(),
    ))
}

async fn collect_vercel_files(
    app_dir: &Path,
    app_meta: &Value,
) -> Result<Vec<VercelCollectedFile>> {
    let paths = managed_paths_from_meta(app_meta).unwrap_or_else(|| scan_deployable_paths(app_dir));
    if paths.is_empty() {
        anyhow::bail!("No deployable app files were found for Vercel publishing");
    }
    if paths.len() > MAX_VERCEL_FILE_COUNT {
        anyhow::bail!(
            "Vercel publish has {} files; limit is {} for direct upload",
            paths.len(),
            MAX_VERCEL_FILE_COUNT
        );
    }

    let mut files = Vec::new();
    let mut total_bytes = 0_u64;
    for path in paths {
        let normalized = normalize_deploy_path(&path)
            .with_context(|| format!("Invalid deploy path '{}'", path.display()))?;
        if excluded_deploy_path(&normalized) {
            continue;
        }
        let full_path = app_dir.join(&normalized);
        let metadata = tokio::fs::metadata(&full_path)
            .await
            .with_context(|| format!("Failed to read metadata for '{}'", normalized))?;
        if !metadata.is_file() {
            continue;
        }
        if metadata.len() > MAX_VERCEL_FILE_BYTES {
            anyhow::bail!(
                "File '{}' is too large for direct Vercel publish ({} bytes)",
                normalized,
                metadata.len()
            );
        }
        total_bytes = total_bytes.saturating_add(metadata.len());
        if total_bytes > MAX_VERCEL_TOTAL_BYTES {
            anyhow::bail!(
                "Vercel publish payload is too large (>{} bytes)",
                MAX_VERCEL_TOTAL_BYTES
            );
        }
        let bytes = tokio::fs::read(&full_path)
            .await
            .with_context(|| format!("Failed to read file '{}'", normalized))?;
        files.push(VercelCollectedFile {
            file: normalized,
            bytes,
        });
    }
    if files.is_empty() {
        anyhow::bail!("No deployable files were found for Vercel publishing");
    }
    Ok(files)
}

fn managed_paths_from_meta(app_meta: &Value) -> Option<Vec<PathBuf>> {
    let values = app_meta.get("managed_files")?.as_array()?;
    let paths = values
        .iter()
        .filter_map(|value| value.as_str())
        .map(str::trim)
        .filter(|value| !value.is_empty())
        .map(PathBuf::from)
        .collect::<Vec<_>>();
    (!paths.is_empty()).then_some(paths)
}

fn scan_deployable_paths(app_dir: &Path) -> Vec<PathBuf> {
    let mut out = Vec::new();
    for entry in walkdir::WalkDir::new(app_dir)
        .follow_links(false)
        .into_iter()
        .filter_map(|entry| entry.ok())
    {
        if !entry.file_type().is_file() {
            continue;
        }
        let Ok(relative) = entry.path().strip_prefix(app_dir) else {
            continue;
        };
        let Ok(normalized) = normalize_deploy_path(relative) else {
            continue;
        };
        if excluded_deploy_path(&normalized) {
            continue;
        }
        out.push(PathBuf::from(normalized));
        if out.len() > MAX_VERCEL_FILE_COUNT {
            break;
        }
    }
    out.sort();
    out
}

fn normalize_deploy_path(path: &Path) -> Result<String> {
    let mut parts = Vec::new();
    for component in path.components() {
        match component {
            Component::Normal(part) => {
                let text = part
                    .to_str()
                    .ok_or_else(|| anyhow::anyhow!("path contains non-UTF-8 segment"))?;
                if text.trim().is_empty() {
                    anyhow::bail!("path contains empty segment");
                }
                parts.push(text.to_string());
            }
            Component::CurDir => {}
            _ => anyhow::bail!("path must be app-relative"),
        }
    }
    if parts.is_empty() {
        anyhow::bail!("path is empty");
    }
    Ok(parts.join("/"))
}

fn excluded_deploy_path(path: &str) -> bool {
    let file_name = path.rsplit('/').next().unwrap_or(path);
    path == ".app_meta.json"
        || path == crate::actions::app::APP_QUALITY_REPORT_FILE
        || path == crate::actions::app::APP_SUB_GOALS_FILE
        || path == ".agentark_runtime_stdout.log"
        || path == ".agentark_runtime_stderr.log"
        || file_name == ".env"
        || file_name.starts_with(".env.")
        || file_name.ends_with(".pem")
        || file_name.ends_with(".key")
        || matches!(file_name, "id_rsa" | "id_ed25519")
        || path
            .split('/')
            .any(|segment| matches!(segment, ".git" | ".agentark" | "node_modules" | "target"))
}

fn project_settings_from_meta(
    app_meta: &Value,
    options: &ExternalDeployOptions,
) -> Map<String, Value> {
    let mut settings = Map::new();
    let build_command = options.build_command.as_deref().or_else(|| {
        app_meta
            .get("build_command")
            .and_then(|value| value.as_str())
    });
    if let Some(value) = build_command
        .map(str::trim)
        .filter(|value| !value.is_empty())
    {
        settings.insert("buildCommand".to_string(), Value::String(value.to_string()));
    }
    let output_dir = options
        .output_dir
        .as_deref()
        .or_else(|| app_meta.get("output_dir").and_then(|value| value.as_str()));
    if let Some(value) = output_dir.map(str::trim).filter(|value| !value.is_empty()) {
        settings.insert(
            "outputDirectory".to_string(),
            Value::String(value.to_string()),
        );
    }
    if let Some(value) = app_meta
        .get("install_command")
        .and_then(|value| value.as_str())
        .map(str::trim)
        .filter(|value| !value.is_empty())
    {
        settings.insert(
            "installCommand".to_string(),
            Value::String(value.to_string()),
        );
    }
    settings
}

async fn persist_external_deployment_meta(
    app_dir: &Path,
    provider: &str,
    deployment: &Value,
) -> Result<()> {
    let mut meta = load_app_meta(app_dir).await;
    let obj = meta
        .as_object_mut()
        .ok_or_else(|| anyhow::anyhow!("app metadata is not an object"))?;
    let deployments = obj
        .entry("external_deployments")
        .or_insert_with(|| Value::Object(Map::new()));
    if !deployments.is_object() {
        *deployments = Value::Object(Map::new());
    }
    if let Some(map) = deployments.as_object_mut() {
        map.insert(provider.to_string(), deployment.clone());
    }
    obj.insert(
        "external_deploy_updated_at".to_string(),
        Value::String(chrono::Utc::now().to_rfc3339()),
    );
    tokio::fs::write(
        app_dir.join(".app_meta.json"),
        serde_json::to_vec_pretty(&meta)?,
    )
    .await?;
    Ok(())
}

fn load_vercel_credentials(
    config_dir: &Path,
    data_dir: &Path,
) -> Result<Option<VercelCredentials>> {
    if let Ok(token) = std::env::var("VERCEL_TOKEN") {
        let token = token.trim().to_string();
        if !token.is_empty() {
            return Ok(Some(VercelCredentials {
                token,
                source: "env".to_string(),
                team_id: std::env::var("VERCEL_TEAM_ID")
                    .ok()
                    .map(|value| value.trim().to_string())
                    .filter(|value| !value.is_empty()),
                project_id: std::env::var("VERCEL_PROJECT_ID")
                    .ok()
                    .map(|value| value.trim().to_string())
                    .filter(|value| !value.is_empty()),
            }));
        }
    }

    let manager = crate::core::runtime::config::SecureConfigManager::new_with_data_dir(
        config_dir,
        Some(data_dir),
    )?;
    let token = manager
        .get_custom_secret(VERCEL_TOKEN_SECRET_KEY)?
        .map(|value| value.trim().to_string())
        .filter(|value| !value.is_empty());
    let Some(token) = token else {
        return Ok(None);
    };
    Ok(Some(VercelCredentials {
        token,
        source: "secure_config".to_string(),
        team_id: manager
            .get_custom_secret(VERCEL_TEAM_ID_SECRET_KEY)?
            .map(|value| value.trim().to_string())
            .filter(|value| !value.is_empty()),
        project_id: manager
            .get_custom_secret(VERCEL_PROJECT_ID_SECRET_KEY)?
            .map(|value| value.trim().to_string())
            .filter(|value| !value.is_empty()),
    }))
}

async fn test_vercel_token(token: &str) -> std::result::Result<Value, String> {
    let client = reqwest::Client::builder()
        .timeout(Duration::from_secs(15))
        .build()
        .map_err(|error| format!("HTTP client error: {}", error))?;
    let response = client
        .get(format!("{}/v2/user", VERCEL_API_BASE))
        .bearer_auth(token)
        .send()
        .await
        .map_err(|error| format!("Vercel status request failed: {}", error))?;
    let status = response.status();
    let payload = response_json_or_text(response).await;
    if status.is_success() {
        Ok(payload)
    } else {
        Err(vercel_error_message(status, &payload))
    }
}

async fn response_json_or_text(response: reqwest::Response) -> Value {
    let status = response.status();
    match response.json::<Value>().await {
        Ok(value) => value,
        Err(error) => json!({
            "status": status.as_u16(),
            "message": format!("Invalid JSON response: {}", error),
        }),
    }
}

fn vercel_error_message(status: StatusCode, payload: &Value) -> String {
    payload
        .get("error")
        .and_then(|value| {
            value
                .get("message")
                .or_else(|| value.get("code"))
                .and_then(|nested| nested.as_str())
        })
        .or_else(|| payload.get("message").and_then(|value| value.as_str()))
        .map(|value| format!("Vercel API returned {}: {}", status.as_u16(), value))
        .unwrap_or_else(|| format!("Vercel API returned {}", status.as_u16()))
}

fn vercel_error_code(payload: &Value) -> Option<String> {
    payload
        .get("error")
        .and_then(|value| value.get("code"))
        .and_then(|value| value.as_str())
        .map(|value| value.to_string())
}

fn payload_project_id(payload: &Value) -> Option<&str> {
    payload
        .get("id")
        .and_then(|value| value.as_str())
        .or_else(|| payload.get("projectId").and_then(|value| value.as_str()))
        .or_else(|| {
            payload
                .get("project")
                .and_then(|value| value.get("id"))
                .and_then(|value| value.as_str())
        })
}

fn payload_deployment_project_id(payload: &Value) -> Option<&str> {
    payload
        .get("projectId")
        .and_then(|value| value.as_str())
        .or_else(|| {
            payload
                .get("project")
                .and_then(|value| value.get("id"))
                .and_then(|value| value.as_str())
        })
}

fn absolute_vercel_url(raw: &str) -> String {
    let value = raw.trim();
    if value.is_empty() {
        return String::new();
    }
    if value.starts_with("http://") || value.starts_with("https://") {
        value.to_string()
    } else {
        format!("https://{}", value.trim_start_matches('/'))
    }
}

fn vercel_project_name(title: &str, app_id: &str) -> String {
    let mut out = String::new();
    for ch in title.chars() {
        if ch.is_ascii_alphanumeric() {
            out.push(ch.to_ascii_lowercase());
        } else if !out.ends_with('-') {
            out.push('-');
        }
        if out.len() >= 48 {
            break;
        }
    }
    let cleaned = out.trim_matches('-').to_string();
    if cleaned.is_empty() {
        format!("agentark-{}", app_id)
    } else {
        cleaned
    }
}

async fn find_git_remote_url(app_dir: &Path) -> Option<String> {
    for candidate in app_dir.ancestors().take(4) {
        let config = candidate.join(".git").join("config");
        let raw = tokio::fs::read_to_string(config).await.ok()?;
        let remote = parse_origin_remote_url(&raw)?;
        if !remote.trim().is_empty() {
            return Some(remote);
        }
    }
    None
}

fn parse_origin_remote_url(config: &str) -> Option<String> {
    let mut in_origin = false;
    for line in config.lines() {
        let trimmed = line.trim();
        if trimmed.starts_with('[') && trimmed.ends_with(']') {
            in_origin = trimmed == r#"[remote "origin"]"#;
            continue;
        }
        if in_origin {
            let Some((key, value)) = trimmed.split_once('=') else {
                continue;
            };
            if key.trim() == "url" {
                let value = value.trim();
                if !value.is_empty() {
                    return Some(value.to_string());
                }
            }
        }
    }
    None
}

fn sanitize_remote_url(raw: &str) -> String {
    let trimmed = raw.trim();
    if let Ok(mut url) = url::Url::parse(trimmed) {
        if !url.username().is_empty() || url.password().is_some() {
            let _ = url.set_username("");
            let _ = url.set_password(None);
            return url.to_string();
        }
        return trimmed.to_string();
    }

    let Some((scheme, rest)) = trimmed.split_once("://") else {
        return trimmed.to_string();
    };
    let Some(at_idx) = rest.find('@') else {
        return trimmed.to_string();
    };
    format!("{}://{}", scheme, &rest[at_idx + 1..])
}

fn set_optional_secret(
    manager: &crate::core::runtime::config::SecureConfigManager,
    key: &str,
    value: Option<String>,
) -> Result<()> {
    if let Some(value) = value.map(|value| value.trim().to_string()) {
        if !value.is_empty() {
            manager.set_custom_secret(key, Some(value))?;
        }
    }
    Ok(())
}

fn optional_string(value: Option<&Value>) -> Option<String> {
    value
        .and_then(|value| value.as_str())
        .map(str::trim)
        .filter(|value| !value.is_empty())
        .map(|value| value.to_string())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn deploy_target_defaults_to_local_for_unknown_values() {
        assert_eq!(
            ExternalDeployTarget::from_value(None),
            ExternalDeployTarget::Local
        );
        assert_eq!(
            ExternalDeployTarget::from_value(Some(&json!("vercel_direct"))),
            ExternalDeployTarget::VercelDirect
        );
        assert_eq!(
            ExternalDeployTarget::from_value(Some(&json!("vercel_git"))),
            ExternalDeployTarget::VercelGit
        );
    }

    #[test]
    fn vercel_project_mode_defaults_to_auto_for_unknown_values() {
        assert_eq!(VercelProjectMode::from_value(None), VercelProjectMode::Auto);
        assert_eq!(
            VercelProjectMode::from_value(Some(&json!("existing"))),
            VercelProjectMode::Existing
        );
        assert_eq!(
            VercelProjectMode::from_value(Some(&json!("create"))),
            VercelProjectMode::Create
        );
        assert_eq!(
            VercelProjectMode::from_value(Some(&json!("anything else"))),
            VercelProjectMode::Auto
        );
    }

    #[test]
    fn parses_origin_remote_from_git_config() {
        let config = r#"
[remote "upstream"]
    url = https://example.com/upstream.git
[remote "origin"]
    url = https://github.com/example/app.git
"#;
        assert_eq!(
            parse_origin_remote_url(config).as_deref(),
            Some("https://github.com/example/app.git")
        );
    }

    #[test]
    fn sanitizes_credential_bearing_git_remotes() {
        assert_eq!(
            sanitize_remote_url("https://user:secret@github.com/example/app.git"),
            "https://github.com/example/app.git"
        );
        assert_eq!(
            sanitize_remote_url("https://ghp_secret@github.com/example/app.git"),
            "https://github.com/example/app.git"
        );
        assert_eq!(
            sanitize_remote_url("git@github.com:example/app.git"),
            "git@github.com:example/app.git"
        );
    }
}
