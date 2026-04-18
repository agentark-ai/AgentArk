use anyhow::{anyhow, Result};
use chrono::Utc;
use serde::{Deserialize, Serialize};
use std::collections::{BTreeSet, HashMap};
use std::path::Path;
use std::process::Stdio;
use tempfile::TempDir;

pub const GOOGLE_WORKSPACE_OAUTH_CONFIG_KEY: &str = "google_workspace_oauth_config";
pub const GOOGLE_WORKSPACE_TOKENS_KEY: &str = "google_workspace_tokens";
pub const GOOGLE_WORKSPACE_BUNDLES_KEY: &str = "google_workspace_bundles";
pub const GOOGLE_WORKSPACE_PENDING_BUNDLES_KEY: &str = "google_workspace_pending_bundles";
const TOKEN_URL: &str = "https://oauth2.googleapis.com/token";
const OAUTH_AUTH_URL: &str = "https://accounts.google.com/o/oauth2/v2/auth";
const DRIVE_API_BASE: &str = "https://www.googleapis.com/drive/v3";
const DOCS_API_BASE: &str = "https://docs.googleapis.com/v1";
const SHEETS_API_BASE: &str = "https://sheets.googleapis.com/v4";
const CHAT_API_BASE: &str = "https://chat.googleapis.com/v1";
const ADMIN_API_BASE: &str = "https://admin.googleapis.com/admin/directory/v1";
const GWS_BINARY_ENV: &str = "AGENTARK_GWS_BINARY";
const GWS_SKILLS_CACHE_DIR: &str = "gws_skills_cache";
const GWS_BUNDLED_SKILLS_DIR: &str = "/app/gws-skills/skills";

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct GwsHelpArgs {
    #[serde(default)]
    pub argv: Vec<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct GwsSchemaArgs {
    pub target: String,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct GwsCommandArgs {
    pub argv: Vec<String>,
    #[serde(default)]
    pub required_bundles: Vec<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct GwsSkillsArgs {
    #[serde(default)]
    pub name: Option<String>,
    #[serde(default)]
    pub filter: Option<String>,
    #[serde(default)]
    pub limit: Option<usize>,
}

#[derive(Debug, Clone)]
struct GwsSkillMetadata {
    name: String,
    description: String,
    cli_help: Option<String>,
    path: std::path::PathBuf,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct GoogleWorkspaceClientConfig {
    pub client_id: String,
    pub client_secret: String,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct GoogleWorkspaceTokens {
    pub access_token: String,
    pub refresh_token: String,
    pub expires_at: i64,
    #[serde(default)]
    pub granted_scopes: Vec<String>,
    #[serde(default)]
    pub granted_bundles: Vec<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct DriveSearchArgs {
    pub query: Option<String>,
    #[serde(default)]
    pub page_size: Option<u32>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct DocsReadArgs {
    pub document_id: String,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct SheetsReadArgs {
    pub spreadsheet_id: String,
    pub range: String,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ChatListSpacesArgs {
    #[serde(default)]
    pub page_size: Option<u32>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct AdminListUsersArgs {
    pub customer: Option<String>,
    pub domain: Option<String>,
    #[serde(default)]
    pub max_results: Option<u32>,
}

#[derive(Debug, Clone, Deserialize)]
struct TokenResponse {
    access_token: String,
    expires_in: i64,
    #[serde(default)]
    refresh_token: Option<String>,
    #[serde(default)]
    scope: Option<String>,
}

#[derive(Debug, Clone, Deserialize)]
struct GoogleApiErrorEnvelope {
    error: GoogleApiErrorPayload,
}

#[derive(Debug, Clone, Deserialize)]
struct GoogleApiErrorPayload {
    #[serde(default)]
    code: Option<i64>,
    #[serde(default)]
    message: Option<String>,
    #[serde(default)]
    reason: Option<String>,
    #[serde(default)]
    enable_url: Option<String>,
}

pub fn supported_bundles() -> &'static [&'static str] {
    &[
        "gmail", "calendar", "drive", "docs", "sheets", "chat", "admin",
    ]
}

pub fn default_bundles() -> Vec<String> {
    vec!["gmail".to_string(), "calendar".to_string()]
}

pub fn bundle_label(bundle: &str) -> &'static str {
    match bundle {
        "gmail" => "Gmail",
        "calendar" => "Calendar",
        "drive" => "Drive",
        "docs" => "Docs",
        "sheets" => "Sheets",
        "chat" => "Chat",
        "admin" => "Admin",
        _ => "Unknown",
    }
}

pub fn normalize_bundle_id(value: &str) -> Option<String> {
    let normalized = value.trim().to_ascii_lowercase().replace([' ', '-'], "_");
    let mapped = match normalized.as_str() {
        "google_calendar" | "calendar" => "calendar",
        "gmail" | "mail" => "gmail",
        "drive" | "google_drive" => "drive",
        "docs" | "google_docs" | "documents" => "docs",
        "sheets" | "google_sheets" | "spreadsheets" => "sheets",
        "chat" | "google_chat" => "chat",
        "admin" | "directory" | "google_admin" => "admin",
        _ => return None,
    };
    Some(mapped.to_string())
}

pub fn infer_required_bundles_from_gws_argv(argv: &[String]) -> Vec<String> {
    let mut inferred = BTreeSet::new();
    for arg in argv {
        let trimmed = arg.trim();
        if trimmed.is_empty() || trimmed.starts_with('-') {
            continue;
        }
        let normalized = trimmed
            .trim_start_matches('+')
            .split(':')
            .next()
            .unwrap_or(trimmed)
            .to_ascii_lowercase()
            .replace([' ', '-'], "_");
        let mapped = match normalized.as_str() {
            "gmail" => Some("gmail"),
            "calendar" => Some("calendar"),
            "drive" => Some("drive"),
            "docs" => Some("docs"),
            "sheets" => Some("sheets"),
            "chat" => Some("chat"),
            "admin" | "admin_reports" | "reports" => Some("admin"),
            _ => None,
        };
        if let Some(bundle) = mapped {
            inferred.insert(bundle.to_string());
        }
    }
    inferred.into_iter().collect()
}

pub fn parse_bundle_list_from_str(value: &str) -> Vec<String> {
    let mut seen = BTreeSet::new();
    for raw in value
        .split([',', '\n', '\r', ';'])
        .map(str::trim)
        .filter(|item| !item.is_empty())
    {
        if let Some(bundle) = normalize_bundle_id(raw) {
            seen.insert(bundle);
        }
    }
    if seen.is_empty() {
        default_bundles()
    } else {
        seen.into_iter().collect()
    }
}

pub fn parse_bundle_list(value: &serde_json::Value) -> Vec<String> {
    if let Some(list) = value.as_array() {
        let mut seen = BTreeSet::new();
        for item in list {
            if let Some(bundle) = item.as_str().and_then(normalize_bundle_id) {
                seen.insert(bundle);
            }
        }
        if !seen.is_empty() {
            return seen.into_iter().collect();
        }
    }
    if let Some(raw) = value.as_str() {
        return parse_bundle_list_from_str(raw);
    }
    default_bundles()
}

pub fn bundle_scopes(bundle: &str) -> &'static [&'static str] {
    match bundle {
        "gmail" => &[
            "https://www.googleapis.com/auth/gmail.readonly",
            "https://www.googleapis.com/auth/gmail.send",
        ],
        "calendar" => &["https://www.googleapis.com/auth/calendar"],
        "drive" => &["https://www.googleapis.com/auth/drive.readonly"],
        "docs" => &["https://www.googleapis.com/auth/documents.readonly"],
        "sheets" => &["https://www.googleapis.com/auth/spreadsheets.readonly"],
        "chat" => &["https://www.googleapis.com/auth/chat.spaces.readonly"],
        "admin" => &["https://www.googleapis.com/auth/admin.directory.user.readonly"],
        _ => &[],
    }
}

pub fn scopes_for_bundles(bundles: &[String]) -> Vec<String> {
    let mut scopes = BTreeSet::new();
    for bundle in bundles {
        for scope in bundle_scopes(bundle) {
            scopes.insert((*scope).to_string());
        }
    }
    scopes.into_iter().collect()
}

pub fn parse_credentials_json(raw: &str) -> Result<GoogleWorkspaceClientConfig> {
    let parsed: serde_json::Value =
        serde_json::from_str(raw).map_err(|e| anyhow!("Invalid credentials JSON: {}", e))?;
    parse_credentials_value(&parsed)
}

pub fn parse_credentials_value(value: &serde_json::Value) -> Result<GoogleWorkspaceClientConfig> {
    fn from_record(record: &serde_json::Value) -> Option<GoogleWorkspaceClientConfig> {
        let client_id = record.get("client_id")?.as_str()?.trim();
        let client_secret = record.get("client_secret")?.as_str()?.trim();
        if client_id.is_empty() || client_secret.is_empty() {
            return None;
        }
        Some(GoogleWorkspaceClientConfig {
            client_id: client_id.to_string(),
            client_secret: client_secret.to_string(),
        })
    }

    if let Some(config) = from_record(value) {
        return Ok(config);
    }
    if let Some(record) = value.get("installed").and_then(from_record) {
        return Ok(record);
    }
    if let Some(record) = value.get("web").and_then(from_record) {
        return Ok(record);
    }
    Err(anyhow!(
        "Credentials JSON must contain client_id and client_secret in either the root, 'installed', or 'web' object."
    ))
}

fn manager(config_dir: &Path) -> Result<crate::core::config::SecureConfigManager> {
    crate::core::config::SecureConfigManager::new(config_dir)
}

fn env_primary_workspace_client_config() -> Option<GoogleWorkspaceClientConfig> {
    let (Ok(client_id), Ok(client_secret)) = (
        std::env::var("GOOGLE_WORKSPACE_CLIENT_ID"),
        std::env::var("GOOGLE_WORKSPACE_CLIENT_SECRET"),
    ) else {
        return None;
    };
    if client_id.trim().is_empty() || client_secret.trim().is_empty() {
        return None;
    }
    Some(GoogleWorkspaceClientConfig {
        client_id,
        client_secret,
    })
}

fn env_legacy_workspace_client_config() -> Option<GoogleWorkspaceClientConfig> {
    for (id_key, secret_key) in [
        ("GMAIL_CLIENT_ID", "GMAIL_CLIENT_SECRET"),
        ("CALENDAR_CLIENT_ID", "CALENDAR_CLIENT_SECRET"),
    ] {
        if let (Ok(client_id), Ok(client_secret)) =
            (std::env::var(id_key), std::env::var(secret_key))
        {
            if !client_id.trim().is_empty() && !client_secret.trim().is_empty() {
                return Some(GoogleWorkspaceClientConfig {
                    client_id,
                    client_secret,
                });
            }
        }
    }
    None
}

fn read_secret_json<T>(config_dir: &Path, key: &str) -> Result<Option<T>>
where
    T: for<'de> Deserialize<'de>,
{
    let manager = manager(config_dir)?;
    let Some(raw) = manager.get_custom_secret(key)? else {
        return Ok(None);
    };
    Ok(Some(serde_json::from_str(&raw)?))
}

fn write_secret_json<T>(config_dir: &Path, key: &str, value: &T) -> Result<()>
where
    T: Serialize,
{
    let manager = manager(config_dir)?;
    manager.set_custom_secret(key, Some(serde_json::to_string(value)?))?;
    Ok(())
}

fn remove_secret(config_dir: &Path, key: &str) -> Result<()> {
    let manager = manager(config_dir)?;
    manager.set_custom_secret(key, None)?;
    Ok(())
}

pub fn load_saved_bundles(config_dir: &Path) -> Result<Vec<String>> {
    match read_secret_json::<Vec<String>>(config_dir, GOOGLE_WORKSPACE_BUNDLES_KEY)? {
        Some(bundles) if !bundles.is_empty() => Ok(bundles
            .into_iter()
            .filter_map(|bundle| normalize_bundle_id(&bundle))
            .collect()),
        _ => {
            if let Some(tokens) = load_workspace_tokens(config_dir)? {
                if !tokens.granted_bundles.is_empty() {
                    return Ok(tokens.granted_bundles);
                }
            }
            let manager = manager(config_dir)?;
            let mut inferred = BTreeSet::new();
            for (bundle, key) in [("gmail", "gmail_tokens"), ("calendar", "calendar_tokens")] {
                if let Some(raw) = manager.get_custom_secret(key)? {
                    let parsed: serde_json::Value = serde_json::from_str(&raw)?;
                    if parsed
                        .get("refresh_token")
                        .and_then(|value| value.as_str())
                        .is_some_and(|value| !value.trim().is_empty())
                    {
                        inferred.insert(bundle.to_string());
                    }
                }
            }
            if inferred.is_empty() {
                Ok(default_bundles())
            } else {
                Ok(inferred.into_iter().collect())
            }
        }
    }
}

pub fn save_selected_bundles(config_dir: &Path, bundles: &[String]) -> Result<()> {
    let normalized = if bundles.is_empty() {
        default_bundles()
    } else {
        bundles.to_vec()
    };
    write_secret_json(config_dir, GOOGLE_WORKSPACE_BUNDLES_KEY, &normalized)
}

pub fn load_pending_bundles(config_dir: &Path) -> Result<Vec<String>> {
    match read_secret_json::<Vec<String>>(config_dir, GOOGLE_WORKSPACE_PENDING_BUNDLES_KEY)? {
        Some(bundles) => Ok(bundles
            .into_iter()
            .filter_map(|bundle| normalize_bundle_id(&bundle))
            .collect()),
        None => Ok(Vec::new()),
    }
}

pub fn save_pending_bundles(config_dir: &Path, bundles: &[String]) -> Result<()> {
    if bundles.is_empty() {
        remove_secret(config_dir, GOOGLE_WORKSPACE_PENDING_BUNDLES_KEY)
    } else {
        write_secret_json(config_dir, GOOGLE_WORKSPACE_PENDING_BUNDLES_KEY, &bundles)
    }
}

pub fn request_additional_bundles(config_dir: &Path, bundles: &[String]) -> Result<Vec<String>> {
    let mut pending = BTreeSet::new();
    for bundle in load_pending_bundles(config_dir)? {
        pending.insert(bundle);
    }
    for bundle in bundles {
        if let Some(normalized) = normalize_bundle_id(bundle) {
            pending.insert(normalized);
        }
    }
    let merged = pending.into_iter().collect::<Vec<_>>();
    save_pending_bundles(config_dir, &merged)?;
    Ok(merged)
}

pub fn oauth_redirect_uri() -> &'static str {
    "http://localhost:8990/oauth/callback"
}

pub fn load_saved_workspace_client_config(
    config_dir: &Path,
) -> Result<Option<GoogleWorkspaceClientConfig>> {
    read_secret_json::<GoogleWorkspaceClientConfig>(config_dir, GOOGLE_WORKSPACE_OAUTH_CONFIG_KEY)
}

pub fn clear_saved_workspace_client_config(config_dir: &Path) -> Result<()> {
    remove_secret(config_dir, GOOGLE_WORKSPACE_OAUTH_CONFIG_KEY)
}

pub fn workspace_client_config_source(config_dir: &Path) -> Result<Option<&'static str>> {
    if env_primary_workspace_client_config().is_some() {
        return Ok(Some("environment_google_workspace"));
    }
    if load_saved_workspace_client_config(config_dir)?.is_some() {
        return Ok(Some("settings"));
    }
    if env_legacy_workspace_client_config().is_some() {
        return Ok(Some("environment_legacy_google"));
    }

    let manager = manager(config_dir)?;
    for key in ["gmail_oauth_config", "calendar_oauth_config"] {
        if let Some(raw) = manager.get_custom_secret(key)? {
            let parsed: serde_json::Value = serde_json::from_str(&raw)?;
            if parse_credentials_value(&parsed).is_ok() {
                return Ok(Some("legacy_integration"));
            }
        }
    }
    Ok(None)
}

pub fn load_workspace_client_config(
    config_dir: &Path,
) -> Result<Option<GoogleWorkspaceClientConfig>> {
    if let Some(config) = env_primary_workspace_client_config() {
        return Ok(Some(config));
    }

    if let Some(config) = load_saved_workspace_client_config(config_dir)? {
        return Ok(Some(config));
    }

    if let Some(config) = env_legacy_workspace_client_config() {
        return Ok(Some(config));
    }

    let manager = manager(config_dir)?;
    for key in ["gmail_oauth_config", "calendar_oauth_config"] {
        if let Some(raw) = manager.get_custom_secret(key)? {
            let parsed: serde_json::Value = serde_json::from_str(&raw)?;
            if let Ok(config) = parse_credentials_value(&parsed) {
                return Ok(Some(config));
            }
        }
    }
    Ok(None)
}

pub fn save_workspace_client_config(
    config_dir: &Path,
    config: &GoogleWorkspaceClientConfig,
) -> Result<()> {
    write_secret_json(config_dir, GOOGLE_WORKSPACE_OAUTH_CONFIG_KEY, config)
}

fn gws_binary() -> String {
    std::env::var(GWS_BINARY_ENV)
        .ok()
        .map(|value| value.trim().to_string())
        .filter(|value| !value.is_empty())
        .unwrap_or_else(|| "gws".to_string())
}

fn format_gws_output(stdout: &[u8], stderr: &[u8]) -> String {
    let stdout_text = String::from_utf8_lossy(stdout).trim().to_string();
    if !stdout_text.is_empty() {
        if let Ok(parsed) = serde_json::from_str::<serde_json::Value>(&stdout_text) {
            return serde_json::to_string_pretty(&parsed).unwrap_or(stdout_text);
        }
        return stdout_text;
    }
    String::from_utf8_lossy(stderr).trim().to_string()
}

fn extract_google_api_error(raw: &str) -> Option<GoogleApiErrorPayload> {
    let trimmed = raw.trim();
    if trimmed.is_empty() {
        return None;
    }

    serde_json::from_str::<GoogleApiErrorEnvelope>(trimmed)
        .or_else(|_| {
            let json_start = trimmed.find('{').ok_or_else(|| {
                serde_json::Error::io(std::io::Error::new(
                    std::io::ErrorKind::InvalidData,
                    "missing JSON body",
                ))
            })?;
            serde_json::from_str::<GoogleApiErrorEnvelope>(&trimmed[json_start..])
        })
        .ok()
        .map(|parsed| parsed.error)
}

fn format_google_api_failure(
    operation: &str,
    status: Option<reqwest::StatusCode>,
    raw_body: &str,
) -> String {
    if let Some(error) = extract_google_api_error(raw_body) {
        let code = error
            .code
            .map(|value| value.to_string())
            .or_else(|| status.map(|value| value.as_u16().to_string()))
            .unwrap_or_else(|| "unknown".to_string());
        let reason = error.reason.unwrap_or_else(|| "unknown_error".to_string());
        let mut message = format!(
            "{} failed: Google reported {} (code {}).",
            operation, reason, code
        );
        if let Some(detail) = error.message.map(|value| value.trim().to_string()) {
            if !detail.is_empty() {
                message.push(' ');
                message.push_str(&detail);
            }
        }
        if let Some(enable_url) = error.enable_url.map(|value| value.trim().to_string()) {
            if !enable_url.is_empty() {
                message.push(' ');
                message.push_str("Enable or inspect the API here: ");
                message.push_str(&enable_url);
            }
        }
        return message;
    }

    match status {
        Some(status) if !raw_body.trim().is_empty() => {
            format!("{} failed ({}): {}", operation, status, raw_body.trim())
        }
        Some(status) => format!("{} failed: {}", operation, status),
        None if !raw_body.trim().is_empty() => format!("{} failed: {}", operation, raw_body.trim()),
        None => format!("{} failed.", operation),
    }
}

async fn ensure_google_response_success(
    response: reqwest::Response,
    operation: &str,
) -> Result<reqwest::Response> {
    let status = response.status();
    if status.is_success() {
        return Ok(response);
    }
    let body = response.text().await.unwrap_or_default();
    Err(anyhow!(
        "{}",
        format_google_api_failure(operation, Some(status), &body)
    ))
}

fn probe_status_counts_as_connected(status: reqwest::StatusCode) -> bool {
    matches!(
        status,
        reqwest::StatusCode::BAD_REQUEST | reqwest::StatusCode::NOT_FOUND
    )
}

#[cfg(test)]
fn google_test_api_override(config_dir: &Path, key: &str, default: &'static str) -> String {
    let path = config_dir.join(key);
    match std::fs::read_to_string(path) {
        Ok(raw) => {
            let trimmed = raw.trim();
            if trimmed.is_empty() {
                default.to_string()
            } else {
                trimmed.to_string()
            }
        }
        Err(_) => default.to_string(),
    }
}

#[cfg(test)]
fn docs_api_base(config_dir: &Path) -> String {
    google_test_api_override(config_dir, ".agentark_test_docs_api_base", DOCS_API_BASE)
}

#[cfg(not(test))]
fn docs_api_base(_config_dir: &Path) -> &'static str {
    DOCS_API_BASE
}

#[cfg(test)]
fn sheets_api_base(config_dir: &Path) -> String {
    google_test_api_override(
        config_dir,
        ".agentark_test_sheets_api_base",
        SHEETS_API_BASE,
    )
}

#[cfg(not(test))]
fn sheets_api_base(_config_dir: &Path) -> &'static str {
    SHEETS_API_BASE
}

async fn current_workspace_access_token(config_dir: &Path) -> Result<String> {
    let mut tokens = load_workspace_tokens(config_dir)?.ok_or_else(|| {
        anyhow!("Google Workspace is not connected yet. Complete the sign-in flow first.")
    })?;
    if tokens.expires_at <= Utc::now().timestamp() + 60 {
        refresh_workspace_token(config_dir, &mut tokens).await?;
    }
    Ok(tokens.access_token)
}

struct GwsRuntimeContext {
    _temp_dir: TempDir,
    config_dir: std::path::PathBuf,
    access_token: Option<String>,
    client: Option<GoogleWorkspaceClientConfig>,
}

async fn prepare_gws_runtime(
    config_dir: &Path,
    required_bundles: &[String],
    require_auth: bool,
) -> Result<GwsRuntimeContext> {
    let temp_dir = tempfile::tempdir()?;
    let gws_config_dir = temp_dir.path().join("gws");
    std::fs::create_dir_all(&gws_config_dir)?;
    if !require_auth {
        return Ok(GwsRuntimeContext {
            _temp_dir: temp_dir,
            config_dir: gws_config_dir,
            access_token: None,
            client: None,
        });
    }

    let normalized_bundles = required_bundles
        .iter()
        .filter_map(|bundle| normalize_bundle_id(bundle))
        .collect::<Vec<_>>();
    let access_token = if normalized_bundles.is_empty() {
        current_workspace_access_token(config_dir).await?
    } else {
        let refs = normalized_bundles
            .iter()
            .map(String::as_str)
            .collect::<Vec<_>>();
        ensure_access_token_for_bundles(config_dir, &refs).await?
    };
    let client = load_workspace_client_config(config_dir)?.ok_or_else(|| {
        anyhow!(
            "Google OAuth client is not configured. Open Integrations > Google Workspace, enter the client ID and client secret, then continue with Google."
        )
    })?;
    Ok(GwsRuntimeContext {
        _temp_dir: temp_dir,
        config_dir: gws_config_dir,
        access_token: Some(access_token),
        client: Some(client),
    })
}

async fn run_gws_command_with_options(
    config_dir: Option<&Path>,
    argv: &[String],
    required_bundles: &[String],
    require_auth: bool,
    current_dir: Option<&Path>,
) -> Result<String> {
    let binary = gws_binary();
    let mut command = tokio::process::Command::new(&binary);
    command.kill_on_drop(true);
    command.args(argv);
    command.stdin(Stdio::null());
    command.stdout(Stdio::piped());
    command.stderr(Stdio::piped());
    command.env("NO_COLOR", "1");
    command.env_remove("GOOGLE_WORKSPACE_CLI_CREDENTIALS_FILE");
    command.env_remove("GOOGLE_WORKSPACE_CLI_TOKEN");
    if let Some(current_dir) = current_dir {
        command.current_dir(current_dir);
    }

    let runtime = if let Some(config_dir) = config_dir {
        Some(prepare_gws_runtime(config_dir, required_bundles, require_auth).await?)
    } else {
        None
    };
    if let Some(runtime) = runtime.as_ref() {
        command.env("GOOGLE_WORKSPACE_CLI_CONFIG_DIR", &runtime.config_dir);
        if let Some(access_token) = runtime.access_token.as_deref() {
            command.env("GOOGLE_WORKSPACE_CLI_TOKEN", access_token);
        }
        if let Some(client) = runtime.client.as_ref() {
            command.env("GOOGLE_WORKSPACE_CLI_CLIENT_ID", &client.client_id);
            command.env("GOOGLE_WORKSPACE_CLI_CLIENT_SECRET", &client.client_secret);
        }
    }

    let output = command.output().await.map_err(|error| {
        if error.kind() == std::io::ErrorKind::NotFound {
            anyhow!(
                "gws CLI is not installed in this {} runtime. Rebuild the runtime image with Google Workspace CLI support.",
                crate::branding::PRODUCT_NAME
            )
        } else {
            anyhow!("Failed to launch gws CLI: {}", error)
        }
    })?;
    let rendered = format_gws_output(&output.stdout, &output.stderr);
    if output.status.success() {
        if rendered.is_empty() {
            Ok("gws command completed.".to_string())
        } else {
            Ok(rendered)
        }
    } else {
        Err(anyhow!(
            "gws command failed (exit {}): {}",
            output.status.code().unwrap_or(-1),
            if rendered.is_empty() {
                "no output returned".to_string()
            } else {
                rendered
            }
        ))
    }
}

async fn run_gws_command(
    config_dir: Option<&Path>,
    argv: &[String],
    required_bundles: &[String],
    require_auth: bool,
) -> Result<String> {
    run_gws_command_with_options(config_dir, argv, required_bundles, require_auth, None).await
}

pub async fn gws_version() -> Result<String> {
    run_gws_command(None, &["--version".to_string()], &[], false).await
}

pub async fn gws_backend_available() -> bool {
    gws_version().await.is_ok()
}

pub async fn gws_text_command(
    config_dir: &Path,
    argv: &[String],
    required_bundles: &[&str],
) -> Result<String> {
    run_gws_command(
        Some(config_dir),
        argv,
        &required_bundles
            .iter()
            .map(|bundle| (*bundle).to_string())
            .collect::<Vec<_>>(),
        true,
    )
    .await
}

pub async fn gws_json_command(
    config_dir: &Path,
    argv: &[String],
    required_bundles: &[&str],
) -> Result<serde_json::Value> {
    let raw = gws_text_command(config_dir, argv, required_bundles).await?;
    serde_json::from_str(&raw)
        .map_err(|error| anyhow!("gws returned non-JSON output: {} | output: {}", error, raw))
}

fn gws_skills_cache_root(config_dir: &Path) -> std::path::PathBuf {
    config_dir.join(GWS_SKILLS_CACHE_DIR)
}

fn trim_frontmatter_value(value: &str) -> String {
    value
        .trim()
        .trim_matches('"')
        .trim_matches('\'')
        .to_string()
}

fn parse_gws_skill_metadata(path: &Path, raw: &str) -> Option<GwsSkillMetadata> {
    let frontmatter = if let Some(stripped) = raw.strip_prefix("---") {
        stripped.split("---").next().unwrap_or("")
    } else {
        ""
    };
    let mut name = String::new();
    let mut description = String::new();
    let mut cli_help = None;
    for line in frontmatter.lines() {
        let trimmed = line.trim();
        if let Some(value) = trimmed.strip_prefix("name:") {
            name = trim_frontmatter_value(value);
        } else if let Some(value) = trimmed.strip_prefix("description:") {
            description = trim_frontmatter_value(value);
        } else if let Some(value) = trimmed.strip_prefix("cliHelp:") {
            let cleaned = trim_frontmatter_value(value);
            if !cleaned.is_empty() {
                cli_help = Some(cleaned);
            }
        }
    }
    if name.is_empty() {
        name = path
            .parent()
            .and_then(|dir| dir.file_name())
            .and_then(|name| name.to_str())
            .unwrap_or("unknown")
            .to_string();
    }
    if description.is_empty() {
        description = format!("Generated Google Workspace CLI skill '{}'.", name);
    }
    Some(GwsSkillMetadata {
        name,
        description,
        cli_help,
        path: path.to_path_buf(),
    })
}

async fn ensure_gws_skills_generated(config_dir: &Path) -> Result<std::path::PathBuf> {
    let bundled_skills_dir = std::path::PathBuf::from(GWS_BUNDLED_SKILLS_DIR);
    let bundled_marker = bundled_skills_dir.join("gws-shared").join("SKILL.md");
    if bundled_marker.exists() {
        return Ok(bundled_skills_dir);
    }

    let cache_root = gws_skills_cache_root(config_dir);
    let skills_dir = cache_root.join("skills");
    let marker = skills_dir.join("gws-shared").join("SKILL.md");
    if marker.exists() {
        return Ok(skills_dir);
    }
    std::fs::create_dir_all(&cache_root)?;
    run_gws_command_with_options(
        None,
        &["generate-skills".to_string()],
        &[],
        false,
        Some(&cache_root),
    )
    .await?;
    if !marker.exists() {
        return Err(anyhow!(
            "gws generate-skills completed but the expected skill catalog was not created."
        ));
    }
    Ok(skills_dir)
}

async fn load_gws_skills(config_dir: &Path) -> Result<Vec<GwsSkillMetadata>> {
    let skills_dir = ensure_gws_skills_generated(config_dir).await?;
    let mut skills = Vec::new();
    for entry in std::fs::read_dir(&skills_dir)? {
        let entry = entry?;
        let path = entry.path();
        if !path.is_dir() {
            continue;
        }
        let skill_path = path.join("SKILL.md");
        if !skill_path.exists() {
            continue;
        }
        let raw = std::fs::read_to_string(&skill_path)?;
        if let Some(parsed) = parse_gws_skill_metadata(&skill_path, &raw) {
            skills.push(parsed);
        }
    }
    skills.sort_by(|a, b| a.name.cmp(&b.name));
    Ok(skills)
}

fn gws_skill_related_bundles(skill: &GwsSkillMetadata) -> Vec<String> {
    let mut bundles = BTreeSet::new();
    for text in [
        skill.name.as_str(),
        skill.description.as_str(),
        skill.cli_help.as_deref().unwrap_or_default(),
    ] {
        for token in text
            .split(|ch: char| !(ch.is_ascii_alphanumeric() || ch == '_' || ch == '-'))
            .filter(|token| !token.is_empty())
        {
            if let Some(bundle) = normalize_bundle_id(token) {
                bundles.insert(bundle);
            }
        }
    }
    if let Some(cli_help) = skill.cli_help.as_deref() {
        let argv = cli_help
            .split_whitespace()
            .filter(|token| !token.eq_ignore_ascii_case("gws"))
            .map(|token| token.to_string())
            .collect::<Vec<_>>();
        for bundle in infer_required_bundles_from_gws_argv(&argv) {
            bundles.insert(bundle);
        }
    }
    bundles.into_iter().collect()
}

fn gws_skill_visible_for_granted_bundles(
    skill: &GwsSkillMetadata,
    granted_bundles: &[String],
) -> bool {
    let related_bundles = gws_skill_related_bundles(skill);
    related_bundles.is_empty()
        || related_bundles.iter().any(|bundle| {
            granted_bundles
                .iter()
                .any(|granted_bundle| granted_bundle == bundle)
        })
}

pub fn load_workspace_tokens(config_dir: &Path) -> Result<Option<GoogleWorkspaceTokens>> {
    read_secret_json::<GoogleWorkspaceTokens>(config_dir, GOOGLE_WORKSPACE_TOKENS_KEY)
}

pub fn save_workspace_tokens(config_dir: &Path, tokens: &GoogleWorkspaceTokens) -> Result<()> {
    write_secret_json(config_dir, GOOGLE_WORKSPACE_TOKENS_KEY, tokens)
}

pub fn selected_and_pending_bundles(config_dir: &Path) -> Result<Vec<String>> {
    let mut bundles = BTreeSet::new();
    for bundle in load_saved_bundles(config_dir)? {
        bundles.insert(bundle);
    }
    for bundle in load_pending_bundles(config_dir)? {
        bundles.insert(bundle);
    }
    if bundles.is_empty() {
        Ok(default_bundles())
    } else {
        Ok(bundles.into_iter().collect())
    }
}

pub fn build_auth_url(
    config_dir: &Path,
    state_token: &str,
    code_challenge: &str,
    redirect_uri: &str,
) -> Result<String> {
    let client = load_workspace_client_config(config_dir)?.ok_or_else(|| {
        anyhow!("Google OAuth client is not configured yet. Open Integrations > Google Workspace, enter the client ID and client secret, then continue with Google.")
    })?;
    let bundles = selected_and_pending_bundles(config_dir)?;
    let scopes = scopes_for_bundles(&bundles).join(" ");
    Ok(format!(
        "{base}?client_id={client_id}&redirect_uri={redirect}&response_type=code&scope={scope}&state={state}&access_type=offline&prompt=consent&include_granted_scopes=true&code_challenge={challenge}&code_challenge_method=S256",
        base = OAUTH_AUTH_URL,
        client_id = urlencoding::encode(&client.client_id),
        redirect = urlencoding::encode(redirect_uri),
        scope = urlencoding::encode(&scopes),
        state = urlencoding::encode(state_token),
        challenge = urlencoding::encode(code_challenge),
    ))
}

pub async fn exchange_code(
    config_dir: &Path,
    redirect_uri: &str,
    code: &str,
    pkce_verifier: Option<&str>,
) -> Result<GoogleWorkspaceTokens> {
    let client = load_workspace_client_config(config_dir)?.ok_or_else(|| {
        anyhow!(
            "Google OAuth client is not configured. Open Integrations > Google Workspace, enter the client ID and client secret, then continue with Google."
        )
    })?;
    let http_client = reqwest::Client::builder()
        .timeout(std::time::Duration::from_secs(15))
        .build()?;
    let mut params = vec![
        ("client_id", client.client_id.clone()),
        ("client_secret", client.client_secret.clone()),
        ("code", code.to_string()),
        ("redirect_uri", redirect_uri.to_string()),
        ("grant_type", "authorization_code".to_string()),
    ];
    if let Some(verifier) = pkce_verifier {
        params.push(("code_verifier", verifier.to_string()));
    }
    let response = http_client.post(TOKEN_URL).form(&params).send().await?;
    if !response.status().is_success() {
        let status = response.status();
        return Err(anyhow!("Token exchange failed ({})", status));
    }

    let token: TokenResponse = response.json().await?;
    let existing = load_workspace_tokens(config_dir)?.unwrap_or(GoogleWorkspaceTokens {
        access_token: String::new(),
        refresh_token: String::new(),
        expires_at: 0,
        granted_scopes: Vec::new(),
        granted_bundles: Vec::new(),
    });
    let granted_scopes = if let Some(scope) = token.scope.as_deref() {
        scope
            .split_whitespace()
            .map(|value| value.trim().to_string())
            .filter(|value| !value.is_empty())
            .collect::<BTreeSet<_>>()
            .into_iter()
            .collect::<Vec<_>>()
    } else {
        scopes_for_bundles(&selected_and_pending_bundles(config_dir)?)
    };
    let granted_bundles = supported_bundles()
        .iter()
        .filter_map(|bundle| {
            let needed = bundle_scopes(bundle);
            if !needed.is_empty()
                && needed
                    .iter()
                    .all(|scope| granted_scopes.iter().any(|value| value == scope))
            {
                Some((*bundle).to_string())
            } else {
                None
            }
        })
        .collect::<Vec<_>>();

    let tokens = GoogleWorkspaceTokens {
        access_token: token.access_token,
        refresh_token: token
            .refresh_token
            .unwrap_or_else(|| existing.refresh_token.clone()),
        expires_at: Utc::now().timestamp() + token.expires_in,
        granted_scopes,
        granted_bundles: granted_bundles.clone(),
    };
    save_workspace_tokens(config_dir, &tokens)?;

    let pending = load_pending_bundles(config_dir)?
        .into_iter()
        .filter(|bundle| !granted_bundles.iter().any(|granted| granted == bundle))
        .collect::<Vec<_>>();
    save_pending_bundles(config_dir, &pending)?;
    Ok(tokens)
}

async fn refresh_workspace_token(
    config_dir: &Path,
    tokens: &mut GoogleWorkspaceTokens,
) -> Result<()> {
    let client = load_workspace_client_config(config_dir)?.ok_or_else(|| {
        anyhow!(
            "Google OAuth client is not configured. Open Integrations > Google Workspace, enter the client ID and client secret, then continue with Google."
        )
    })?;
    if tokens.refresh_token.trim().is_empty() {
        return Err(anyhow!(
            "Google Workspace refresh token is missing. Reconnect the integration."
        ));
    }

    let http_client = reqwest::Client::builder()
        .timeout(std::time::Duration::from_secs(15))
        .build()?;
    let params = [
        ("client_id", client.client_id.as_str()),
        ("client_secret", client.client_secret.as_str()),
        ("refresh_token", tokens.refresh_token.as_str()),
        ("grant_type", "refresh_token"),
    ];
    let response = http_client.post(TOKEN_URL).form(&params).send().await?;
    if !response.status().is_success() {
        let status = response.status();
        return Err(anyhow!(
            "Failed to refresh Google Workspace token ({})",
            status
        ));
    }
    let token: TokenResponse = response.json().await?;
    tokens.access_token = token.access_token;
    tokens.expires_at = Utc::now().timestamp() + token.expires_in;
    if let Some(refresh_token) = token.refresh_token {
        if !refresh_token.trim().is_empty() {
            tokens.refresh_token = refresh_token;
        }
    }
    save_workspace_tokens(config_dir, tokens)
}

pub fn granted_bundles(config_dir: &Path) -> Result<Vec<String>> {
    Ok(load_workspace_tokens(config_dir)?
        .map(|tokens| tokens.granted_bundles)
        .unwrap_or_default())
}

pub fn missing_selected_bundles(config_dir: &Path) -> Result<Vec<String>> {
    let requested = load_saved_bundles(config_dir)?;
    let granted = granted_bundles(config_dir)?;
    Ok(requested
        .into_iter()
        .filter(|bundle| {
            !granted
                .iter()
                .any(|granted_bundle| granted_bundle == bundle)
        })
        .collect())
}

pub async fn ensure_access_token_for_bundles(
    config_dir: &Path,
    bundles: &[&str],
) -> Result<String> {
    let normalized = bundles
        .iter()
        .filter_map(|bundle| normalize_bundle_id(bundle))
        .collect::<Vec<_>>();
    let granted = granted_bundles(config_dir)?;
    let missing = normalized
        .iter()
        .filter(|bundle| {
            !granted
                .iter()
                .any(|granted_bundle| granted_bundle == *bundle)
        })
        .cloned()
        .collect::<Vec<_>>();
    if !missing.is_empty() {
        let _ = request_additional_bundles(config_dir, &missing);
        let requested = missing
            .iter()
            .map(|bundle| bundle_label(bundle))
            .collect::<Vec<_>>()
            .join(", ");
        return Err(anyhow!(
            "Google Workspace needs additional access for {}. Reconnect the integration to grant those bundles.",
            requested
        ));
    }

    current_workspace_access_token(config_dir).await
}

pub fn summarize_connection_status(config_dir: &Path) -> Result<(bool, Vec<String>, Vec<String>)> {
    let granted = granted_bundles(config_dir)?;
    let missing = missing_selected_bundles(config_dir)?;
    Ok((!granted.is_empty(), granted, missing))
}

fn ensure_granted_bundle_visibility(config_dir: &Path, required_bundles: &[String]) -> Result<()> {
    if required_bundles.is_empty() {
        return Ok(());
    }
    let granted = granted_bundles(config_dir)?;
    let missing = required_bundles
        .iter()
        .filter(|bundle| {
            !granted
                .iter()
                .any(|granted_bundle| granted_bundle == *bundle)
        })
        .cloned()
        .collect::<Vec<_>>();
    if missing.is_empty() {
        return Ok(());
    }
    Err(anyhow!(
        "This Google Workspace surface is only available for currently granted bundles. Missing: {}.",
        missing
            .iter()
            .map(|bundle| bundle_label(bundle))
            .collect::<Vec<_>>()
            .join(", ")
    ))
}

pub async fn gws_help(config_dir: &Path, arguments: &serde_json::Value) -> Result<String> {
    let args: GwsHelpArgs = serde_json::from_value(arguments.clone())
        .map_err(|e| anyhow!("Invalid gws help args: {}", e))?;
    let argv = if args.argv.is_empty() {
        vec!["--help".to_string()]
    } else {
        args.argv
    };
    if argv
        .first()
        .is_some_and(|value| value.trim().eq_ignore_ascii_case("auth"))
    {
        return Err(anyhow!(
            "gws auth commands are not exposed through {}. Use the Google Workspace integration popup for sign-in.",
            crate::branding::PRODUCT_NAME
        ));
    }
    let required_bundles = infer_required_bundles_from_gws_argv(&argv);
    ensure_granted_bundle_visibility(config_dir, &required_bundles)?;
    run_gws_command(None, &argv, &[], false).await
}

pub async fn gws_schema(config_dir: &Path, arguments: &serde_json::Value) -> Result<String> {
    let args: GwsSchemaArgs = serde_json::from_value(arguments.clone())
        .map_err(|e| anyhow!("Invalid gws schema args: {}", e))?;
    let target = args.target.trim();
    if target.is_empty() {
        return Err(anyhow!("Missing gws schema target."));
    }
    let required_bundles = infer_required_bundles_from_gws_argv(
        &target
            .split('.')
            .map(|token| token.to_string())
            .collect::<Vec<_>>(),
    );
    ensure_granted_bundle_visibility(config_dir, &required_bundles)?;
    run_gws_command(
        None,
        &["schema".to_string(), target.to_string()],
        &[],
        false,
    )
    .await
}

pub async fn gws_command(config_dir: &Path, arguments: &serde_json::Value) -> Result<String> {
    let args: GwsCommandArgs = serde_json::from_value(arguments.clone())
        .map_err(|e| anyhow!("Invalid gws command args: {}", e))?;
    if args.argv.is_empty() {
        return Err(anyhow!(
            "{}",
            "Missing gws argv. Provide the arguments after `gws`, for example [\"drive\",\"files\",\"list\",\"--params\",\"{\\\"pageSize\\\":5}\"]"
        ));
    }
    if args
        .argv
        .first()
        .is_some_and(|value| value.trim().eq_ignore_ascii_case("auth"))
    {
        return Err(anyhow!(
            "gws auth commands are not exposed through {}. Use the Google Workspace integration popup for sign-in.",
            crate::branding::PRODUCT_NAME
        ));
    }
    let mut required_bundles = BTreeSet::new();
    for bundle in args.required_bundles {
        if let Some(normalized) = normalize_bundle_id(&bundle) {
            required_bundles.insert(normalized);
        }
    }
    for bundle in infer_required_bundles_from_gws_argv(&args.argv) {
        required_bundles.insert(bundle);
    }
    run_gws_command(
        Some(config_dir),
        &args.argv,
        &required_bundles.into_iter().collect::<Vec<_>>(),
        true,
    )
    .await
}

pub async fn gws_skills(config_dir: &Path, arguments: &serde_json::Value) -> Result<String> {
    let args: GwsSkillsArgs = serde_json::from_value(arguments.clone())
        .map_err(|e| anyhow!("Invalid gws skills args: {}", e))?;
    let mut skills = load_gws_skills(config_dir).await?;
    let granted_bundles = granted_bundles(config_dir)?;
    skills.retain(|skill| gws_skill_visible_for_granted_bundles(skill, &granted_bundles));
    if let Some(filter) = args
        .filter
        .as_deref()
        .map(str::trim)
        .filter(|value| !value.is_empty())
    {
        let needle = filter.to_ascii_lowercase();
        skills.retain(|skill| {
            skill.name.to_ascii_lowercase().contains(&needle)
                || skill.description.to_ascii_lowercase().contains(&needle)
                || skill
                    .cli_help
                    .as_deref()
                    .unwrap_or_default()
                    .to_ascii_lowercase()
                    .contains(&needle)
        });
    }
    if let Some(name) = args
        .name
        .as_deref()
        .map(str::trim)
        .filter(|value| !value.is_empty())
    {
        let needle = name.to_ascii_lowercase();
        let skill = skills
            .into_iter()
            .find(|skill| {
                skill.name.eq_ignore_ascii_case(name)
                    || skill.name.to_ascii_lowercase().contains(&needle)
            })
            .ok_or_else(|| anyhow!("No generated gws skill matched '{}'.", name))?;
        return Ok(std::fs::read_to_string(&skill.path)?);
    }
    let limit = args.limit.unwrap_or(80).clamp(1, 200);
    if skills.is_empty() {
        return Ok("No generated gws skills are available for the currently granted Google Workspace bundles.".to_string());
    }
    let granted_labels = if granted_bundles.is_empty() {
        "none".to_string()
    } else {
        granted_bundles
            .iter()
            .map(|bundle| bundle_label(bundle))
            .collect::<Vec<_>>()
            .join(", ")
    };
    let mut lines = vec![format!(
        "Generated Google Workspace CLI skills available for granted bundles ({}): {}",
        granted_labels,
        skills.len()
    )];
    for skill in skills.into_iter().take(limit) {
        let cli_help = skill.cli_help.unwrap_or_else(|| "gws --help".to_string());
        lines.push(format!(
            "- {} — {} | {}",
            skill.name, skill.description, cli_help
        ));
    }
    Ok(lines.join("\n"))
}

pub async fn drive_search(config_dir: &Path, arguments: &serde_json::Value) -> Result<String> {
    let args: DriveSearchArgs = serde_json::from_value(arguments.clone())
        .map_err(|e| anyhow!("Invalid Drive search args: {}", e))?;
    let page_size = args.page_size.unwrap_or(10).clamp(1, 50);
    let data: serde_json::Value = if gws_backend_available().await {
        let mut params = serde_json::json!({
            "pageSize": page_size,
            "fields": "files(id,name,mimeType,modifiedTime,webViewLink,owners(displayName,emailAddress))",
            "orderBy": "modifiedTime desc"
        });
        if let Some(q) = args
            .query
            .as_deref()
            .map(str::trim)
            .filter(|value| !value.is_empty())
        {
            params["q"] = serde_json::Value::String(q.to_string());
        }
        let argv = vec![
            "drive".to_string(),
            "files".to_string(),
            "list".to_string(),
            "--params".to_string(),
            params.to_string(),
        ];
        match gws_json_command(config_dir, &argv, &["drive"]).await {
            Ok(data) => data,
            Err(error) => {
                tracing::warn!(
                    "Drive search gws path failed, falling back to REST: {}",
                    error
                );
                let access_token = ensure_access_token_for_bundles(config_dir, &["drive"]).await?;
                let client = reqwest::Client::builder()
                    .timeout(std::time::Duration::from_secs(15))
                    .build()?;
                let mut url = reqwest::Url::parse(&format!("{}/files", DRIVE_API_BASE))?;
                {
                    let mut query = url.query_pairs_mut();
                    query.append_pair("pageSize", &page_size.to_string());
                    query.append_pair(
                        "fields",
                        "files(id,name,mimeType,modifiedTime,webViewLink,owners(displayName,emailAddress))",
                    );
                    query.append_pair("orderBy", "modifiedTime desc");
                    if let Some(q) = args
                        .query
                        .as_deref()
                        .map(str::trim)
                        .filter(|value| !value.is_empty())
                    {
                        query.append_pair("q", q);
                    }
                }
                let response = client.get(url).bearer_auth(access_token).send().await?;
                ensure_google_response_success(response, "Google Drive search")
                    .await?
                    .json()
                    .await?
            }
        }
    } else {
        let access_token = ensure_access_token_for_bundles(config_dir, &["drive"]).await?;
        let client = reqwest::Client::builder()
            .timeout(std::time::Duration::from_secs(15))
            .build()?;
        let mut url = reqwest::Url::parse(&format!("{}/files", DRIVE_API_BASE))?;
        {
            let mut query = url.query_pairs_mut();
            query.append_pair("pageSize", &page_size.to_string());
            query.append_pair(
                "fields",
                "files(id,name,mimeType,modifiedTime,webViewLink,owners(displayName,emailAddress))",
            );
            query.append_pair("orderBy", "modifiedTime desc");
            if let Some(q) = args
                .query
                .as_deref()
                .map(str::trim)
                .filter(|value| !value.is_empty())
            {
                query.append_pair("q", q);
            }
        }
        let response = client.get(url).bearer_auth(access_token).send().await?;
        ensure_google_response_success(response, "Google Drive search")
            .await?
            .json()
            .await?
    };
    let files = data
        .get("files")
        .and_then(|value| value.as_array())
        .cloned()
        .unwrap_or_default();
    if files.is_empty() {
        return Ok("No matching Google Drive files found.".to_string());
    }

    let mut lines = Vec::new();
    for file in files {
        let name = file
            .get("name")
            .and_then(|value| value.as_str())
            .unwrap_or("Untitled");
        let mime = file
            .get("mimeType")
            .and_then(|value| value.as_str())
            .unwrap_or("unknown");
        let modified = file
            .get("modifiedTime")
            .and_then(|value| value.as_str())
            .unwrap_or("-");
        let owner = file
            .get("owners")
            .and_then(|value| value.as_array())
            .and_then(|owners| owners.first())
            .and_then(|value| {
                value
                    .get("displayName")
                    .and_then(|name| name.as_str())
                    .or_else(|| value.get("emailAddress").and_then(|email| email.as_str()))
            })
            .unwrap_or("-");
        let link = file
            .get("webViewLink")
            .and_then(|value| value.as_str())
            .unwrap_or("-");
        let file_id = file
            .get("id")
            .and_then(|value| value.as_str())
            .unwrap_or("-");
        lines.push(format!(
            "- {} | {} | modified {} | owner {} | {} | id {}",
            name, mime, modified, owner, link, file_id
        ));
    }
    Ok(lines.join("\n"))
}

pub async fn docs_read(config_dir: &Path, arguments: &serde_json::Value) -> Result<String> {
    let args: DocsReadArgs = serde_json::from_value(arguments.clone())
        .map_err(|e| anyhow!("Invalid Docs args: {}", e))?;
    let data: serde_json::Value = if gws_backend_available().await {
        let argv = vec![
            "docs".to_string(),
            "documents".to_string(),
            "get".to_string(),
            "--params".to_string(),
            serde_json::json!({ "documentId": args.document_id }).to_string(),
        ];
        match gws_json_command(config_dir, &argv, &["docs"]).await {
            Ok(data) => data,
            Err(error) => {
                tracing::warn!("Docs read gws path failed, falling back to REST: {}", error);
                let access_token = ensure_access_token_for_bundles(config_dir, &["docs"]).await?;
                let client = reqwest::Client::builder()
                    .timeout(std::time::Duration::from_secs(15))
                    .build()?;
                let response = client
                    .get(format!(
                        "{}/documents/{}",
                        docs_api_base(config_dir),
                        args.document_id
                    ))
                    .bearer_auth(access_token)
                    .send()
                    .await?;
                ensure_google_response_success(response, "Google Docs read")
                    .await?
                    .json()
                    .await?
            }
        }
    } else {
        let access_token = ensure_access_token_for_bundles(config_dir, &["docs"]).await?;
        let client = reqwest::Client::builder()
            .timeout(std::time::Duration::from_secs(15))
            .build()?;
        let response = client
            .get(format!(
                "{}/documents/{}",
                docs_api_base(config_dir),
                args.document_id
            ))
            .bearer_auth(access_token)
            .send()
            .await?;
        ensure_google_response_success(response, "Google Docs read")
            .await?
            .json()
            .await?
    };
    let title = data
        .get("title")
        .and_then(|value| value.as_str())
        .unwrap_or("Untitled doc");
    let mut text_parts = Vec::new();
    if let Some(content) = data
        .get("body")
        .and_then(|value| value.get("content"))
        .and_then(|value| value.as_array())
    {
        for block in content {
            let Some(elements) = block
                .get("paragraph")
                .and_then(|value| value.get("elements"))
                .and_then(|value| value.as_array())
            else {
                continue;
            };
            for element in elements {
                if let Some(text) = element
                    .get("textRun")
                    .and_then(|value| value.get("content"))
                    .and_then(|value| value.as_str())
                {
                    let trimmed = text.trim();
                    if !trimmed.is_empty() {
                        text_parts.push(trimmed.to_string());
                    }
                }
            }
        }
    }
    let joined = text_parts.join(" ");
    if joined.is_empty() {
        Ok(format!("{} has no readable text content.", title))
    } else {
        Ok(format!("{}\n\n{}", title, joined))
    }
}

pub async fn sheets_read(config_dir: &Path, arguments: &serde_json::Value) -> Result<String> {
    let args: SheetsReadArgs = serde_json::from_value(arguments.clone())
        .map_err(|e| anyhow!("Invalid Sheets args: {}", e))?;
    let data: serde_json::Value = if gws_backend_available().await {
        let argv = vec![
            "sheets".to_string(),
            "spreadsheets".to_string(),
            "values".to_string(),
            "get".to_string(),
            "--params".to_string(),
            serde_json::json!({
                "spreadsheetId": args.spreadsheet_id,
                "range": args.range
            })
            .to_string(),
        ];
        match gws_json_command(config_dir, &argv, &["sheets"]).await {
            Ok(data) => data,
            Err(error) => {
                tracing::warn!(
                    "Sheets read gws path failed, falling back to REST: {}",
                    error
                );
                let access_token = ensure_access_token_for_bundles(config_dir, &["sheets"]).await?;
                let client = reqwest::Client::builder()
                    .timeout(std::time::Duration::from_secs(15))
                    .build()?;
                let response = client
                    .get(format!(
                        "{}/spreadsheets/{}/values/{}",
                        sheets_api_base(config_dir),
                        args.spreadsheet_id,
                        urlencoding::encode(&args.range)
                    ))
                    .bearer_auth(access_token)
                    .send()
                    .await?;
                ensure_google_response_success(response, "Google Sheets read")
                    .await?
                    .json()
                    .await?
            }
        }
    } else {
        let access_token = ensure_access_token_for_bundles(config_dir, &["sheets"]).await?;
        let client = reqwest::Client::builder()
            .timeout(std::time::Duration::from_secs(15))
            .build()?;
        let response = client
            .get(format!(
                "{}/spreadsheets/{}/values/{}",
                sheets_api_base(config_dir),
                args.spreadsheet_id,
                urlencoding::encode(&args.range)
            ))
            .bearer_auth(access_token)
            .send()
            .await?;
        ensure_google_response_success(response, "Google Sheets read")
            .await?
            .json()
            .await?
    };
    let values = data
        .get("values")
        .and_then(|value| value.as_array())
        .cloned()
        .unwrap_or_default();
    if values.is_empty() {
        return Ok(format!("No values found for range {}.", args.range));
    }
    let lines = values
        .iter()
        .map(|row| {
            row.as_array()
                .map(|cells| {
                    cells
                        .iter()
                        .map(|cell| {
                            cell.as_str()
                                .map(str::to_string)
                                .unwrap_or_else(|| cell.to_string())
                        })
                        .collect::<Vec<_>>()
                        .join(" | ")
                })
                .unwrap_or_else(|| row.to_string())
        })
        .collect::<Vec<_>>();
    Ok(lines.join("\n"))
}

pub async fn chat_list_spaces(config_dir: &Path, arguments: &serde_json::Value) -> Result<String> {
    let args: ChatListSpacesArgs = serde_json::from_value(arguments.clone())
        .map_err(|e| anyhow!("Invalid Chat args: {}", e))?;
    let page_size = args.page_size.unwrap_or(20).clamp(1, 50);
    let data: serde_json::Value = if gws_backend_available().await {
        let argv = vec![
            "chat".to_string(),
            "spaces".to_string(),
            "list".to_string(),
            "--params".to_string(),
            serde_json::json!({ "pageSize": page_size }).to_string(),
        ];
        match gws_json_command(config_dir, &argv, &["chat"]).await {
            Ok(data) => data,
            Err(error) => {
                tracing::warn!(
                    "Google Chat spaces gws path failed, falling back to REST: {}",
                    error
                );
                let access_token = ensure_access_token_for_bundles(config_dir, &["chat"]).await?;
                let client = reqwest::Client::builder()
                    .timeout(std::time::Duration::from_secs(15))
                    .build()?;
                let mut url = reqwest::Url::parse(&format!("{}/spaces", CHAT_API_BASE))?;
                url.query_pairs_mut()
                    .append_pair("pageSize", &page_size.to_string());
                let response = client.get(url).bearer_auth(access_token).send().await?;
                ensure_google_response_success(response, "Google Chat spaces request")
                    .await?
                    .json()
                    .await?
            }
        }
    } else {
        let access_token = ensure_access_token_for_bundles(config_dir, &["chat"]).await?;
        let client = reqwest::Client::builder()
            .timeout(std::time::Duration::from_secs(15))
            .build()?;
        let mut url = reqwest::Url::parse(&format!("{}/spaces", CHAT_API_BASE))?;
        url.query_pairs_mut()
            .append_pair("pageSize", &page_size.to_string());
        let response = client.get(url).bearer_auth(access_token).send().await?;
        ensure_google_response_success(response, "Google Chat spaces request")
            .await?
            .json()
            .await?
    };
    let spaces = data
        .get("spaces")
        .and_then(|value| value.as_array())
        .cloned()
        .unwrap_or_default();
    if spaces.is_empty() {
        return Ok("No Google Chat spaces found.".to_string());
    }
    let mut lines = Vec::new();
    for space in spaces {
        let name = space
            .get("displayName")
            .and_then(|value| value.as_str())
            .unwrap_or("(unnamed)");
        let kind = space
            .get("spaceType")
            .and_then(|value| value.as_str())
            .unwrap_or("-");
        let id = space
            .get("name")
            .and_then(|value| value.as_str())
            .unwrap_or("-");
        lines.push(format!("- {} | {} | {}", name, kind, id));
    }
    Ok(lines.join("\n"))
}

pub async fn admin_list_users(config_dir: &Path, arguments: &serde_json::Value) -> Result<String> {
    let args: AdminListUsersArgs = serde_json::from_value(arguments.clone())
        .map_err(|e| anyhow!("Invalid Admin args: {}", e))?;
    let max_results = args.max_results.unwrap_or(20).clamp(1, 50);
    let data: serde_json::Value = if gws_backend_available().await {
        let mut params = serde_json::json!({
            "maxResults": max_results,
            "orderBy": "email"
        });
        if let Some(customer) = args
            .customer
            .as_deref()
            .map(str::trim)
            .filter(|value| !value.is_empty())
        {
            params["customer"] = serde_json::Value::String(customer.to_string());
        } else if let Some(domain) = args
            .domain
            .as_deref()
            .map(str::trim)
            .filter(|value| !value.is_empty())
        {
            params["domain"] = serde_json::Value::String(domain.to_string());
        } else {
            params["customer"] = serde_json::Value::String("my_customer".to_string());
        }
        let argv = vec![
            "admin".to_string(),
            "users".to_string(),
            "list".to_string(),
            "--params".to_string(),
            params.to_string(),
        ];
        match gws_json_command(config_dir, &argv, &["admin"]).await {
            Ok(data) => data,
            Err(error) => {
                tracing::warn!(
                    "Google Admin gws path failed, falling back to REST: {}",
                    error
                );
                let access_token = ensure_access_token_for_bundles(config_dir, &["admin"]).await?;
                let client = reqwest::Client::builder()
                    .timeout(std::time::Duration::from_secs(15))
                    .build()?;
                let mut url = reqwest::Url::parse(&format!("{}/users", ADMIN_API_BASE))?;
                {
                    let mut query = url.query_pairs_mut();
                    query.append_pair("maxResults", &max_results.to_string());
                    query.append_pair("orderBy", "email");
                    if let Some(customer) = args
                        .customer
                        .as_deref()
                        .map(str::trim)
                        .filter(|value| !value.is_empty())
                    {
                        query.append_pair("customer", customer);
                    } else if let Some(domain) = args
                        .domain
                        .as_deref()
                        .map(str::trim)
                        .filter(|value| !value.is_empty())
                    {
                        query.append_pair("domain", domain);
                    } else {
                        query.append_pair("customer", "my_customer");
                    }
                }
                let response = client.get(url).bearer_auth(access_token).send().await?;
                ensure_google_response_success(response, "Google Admin users request")
                    .await?
                    .json()
                    .await?
            }
        }
    } else {
        let access_token = ensure_access_token_for_bundles(config_dir, &["admin"]).await?;
        let client = reqwest::Client::builder()
            .timeout(std::time::Duration::from_secs(15))
            .build()?;
        let mut url = reqwest::Url::parse(&format!("{}/users", ADMIN_API_BASE))?;
        {
            let mut query = url.query_pairs_mut();
            query.append_pair("maxResults", &max_results.to_string());
            query.append_pair("orderBy", "email");
            if let Some(customer) = args
                .customer
                .as_deref()
                .map(str::trim)
                .filter(|value| !value.is_empty())
            {
                query.append_pair("customer", customer);
            } else if let Some(domain) = args
                .domain
                .as_deref()
                .map(str::trim)
                .filter(|value| !value.is_empty())
            {
                query.append_pair("domain", domain);
            } else {
                query.append_pair("customer", "my_customer");
            }
        }
        let response = client.get(url).bearer_auth(access_token).send().await?;
        ensure_google_response_success(response, "Google Admin users request")
            .await?
            .json()
            .await?
    };
    let users = data
        .get("users")
        .and_then(|value| value.as_array())
        .cloned()
        .unwrap_or_default();
    if users.is_empty() {
        return Ok("No Google Workspace users found.".to_string());
    }
    let mut lines = Vec::new();
    for user in users {
        let primary_email = user
            .get("primaryEmail")
            .and_then(|value| value.as_str())
            .unwrap_or("-");
        let name = user
            .get("name")
            .and_then(|value| value.get("fullName"))
            .and_then(|value| value.as_str())
            .unwrap_or("-");
        let suspended = user
            .get("suspended")
            .and_then(|value| value.as_bool())
            .unwrap_or(false);
        lines.push(format!(
            "- {} | {}{}",
            primary_email,
            name,
            if suspended { " | suspended" } else { "" }
        ));
    }
    Ok(lines.join("\n"))
}

pub async fn test_bundle_access(config_dir: &Path, bundle: &str) -> Result<String> {
    let normalized = normalize_bundle_id(bundle)
        .ok_or_else(|| anyhow!("Unsupported Google Workspace bundle '{}'.", bundle))?;
    let access_token = ensure_access_token_for_bundles(config_dir, &[normalized.as_str()]).await?;
    let client = reqwest::Client::builder()
        .timeout(std::time::Duration::from_secs(15))
        .build()?;

    match normalized.as_str() {
        "gmail" => {
            let response = client
                .get("https://gmail.googleapis.com/gmail/v1/users/me/profile")
                .bearer_auth(&access_token)
                .send()
                .await?;
            ensure_google_response_success(response, "Gmail profile request").await?;
            Ok("Gmail connected".to_string())
        }
        "calendar" => {
            let response = client
                .get("https://www.googleapis.com/calendar/v3/calendars/primary")
                .bearer_auth(&access_token)
                .send()
                .await?;
            ensure_google_response_success(response, "Calendar primary request").await?;
            Ok("Calendar connected".to_string())
        }
        "drive" => {
            let response = client
                .get(format!(
                    "{}/files?pageSize=1&fields=files(id,name)",
                    DRIVE_API_BASE
                ))
                .bearer_auth(&access_token)
                .send()
                .await?;
            ensure_google_response_success(response, "Drive files request").await?;
            Ok("Drive connected".to_string())
        }
        "docs" => {
            let response = client
                .get(format!(
                    "{}/documents/__agentark_probe__",
                    docs_api_base(config_dir)
                ))
                .bearer_auth(&access_token)
                .send()
                .await?;
            let status = response.status();
            if status.is_success() || probe_status_counts_as_connected(status) {
                Ok("Docs connected".to_string())
            } else {
                let body = response.text().await.unwrap_or_default();
                Err(anyhow!(
                    "{}",
                    format_google_api_failure("Docs probe request", Some(status), &body)
                ))
            }
        }
        "sheets" => {
            let response = client
                .get(format!(
                    "{}/spreadsheets/__agentark_probe__?includeGridData=false&fields=spreadsheetId",
                    sheets_api_base(config_dir)
                ))
                .bearer_auth(&access_token)
                .send()
                .await?;
            let status = response.status();
            if status.is_success() || probe_status_counts_as_connected(status) {
                Ok("Sheets connected".to_string())
            } else {
                let body = response.text().await.unwrap_or_default();
                Err(anyhow!(
                    "{}",
                    format_google_api_failure("Sheets probe request", Some(status), &body)
                ))
            }
        }
        "chat" => {
            let response = client
                .get(format!("{}/spaces?pageSize=1", CHAT_API_BASE))
                .bearer_auth(&access_token)
                .send()
                .await?;
            ensure_google_response_success(response, "Chat spaces request").await?;
            Ok("Chat connected".to_string())
        }
        "admin" => {
            let response = client
                .get(format!(
                    "{}/users?customer=my_customer&maxResults=1",
                    ADMIN_API_BASE
                ))
                .bearer_auth(&access_token)
                .send()
                .await?;
            ensure_google_response_success(response, "Admin users request").await?;
            Ok("Admin connected".to_string())
        }
        _ => Err(anyhow!(
            "Unsupported Google Workspace bundle '{}'.",
            normalized
        )),
    }
}

pub async fn test_selected_bundles(config_dir: &Path) -> Result<HashMap<String, String>> {
    let bundles = load_saved_bundles(config_dir)?;
    let mut results = HashMap::new();
    match gws_version().await {
        Ok(version) => {
            let version = version.trim();
            results.insert(
                "gws_backend".to_string(),
                if version.is_empty() {
                    "gws CLI ready".to_string()
                } else {
                    format!("gws CLI ready ({})", version)
                },
            );
        }
        Err(error) => {
            results.insert(
                "gws_backend".to_string(),
                format!("gws unavailable: {}", error),
            );
        }
    }

    for bundle in bundles {
        match test_bundle_access(config_dir, &bundle).await {
            Ok(message) => {
                results.insert(bundle, message);
            }
            Err(error) => {
                results.insert(bundle, error.to_string());
            }
        }
    }

    Ok(results)
}

#[cfg(test)]
mod tests {
    use super::*;
    use axum::{http::StatusCode, routing::get, Router};
    use tempfile::tempdir;
    use tokio::net::TcpListener;

    #[test]
    fn parses_google_client_json_shapes() {
        let web = r#"{"web":{"client_id":"abc","client_secret":"def"}}"#;
        let installed = r#"{"installed":{"client_id":"ghi","client_secret":"jkl"}}"#;
        assert_eq!(parse_credentials_json(web).unwrap().client_id, "abc");
        assert_eq!(
            parse_credentials_json(installed).unwrap().client_secret,
            "jkl"
        );
    }

    #[test]
    fn bundle_list_normalizes_and_defaults() {
        assert_eq!(
            parse_bundle_list_from_str("Gmail, google_calendar, drive, invalid"),
            vec![
                "calendar".to_string(),
                "drive".to_string(),
                "gmail".to_string()
            ]
        );
        assert_eq!(parse_bundle_list_from_str(""), default_bundles());
    }

    #[test]
    fn scopes_cover_each_selected_bundle_once() {
        let scopes = scopes_for_bundles(&[
            "gmail".to_string(),
            "calendar".to_string(),
            "gmail".to_string(),
        ]);
        assert!(scopes.iter().any(|scope| scope.contains("gmail.readonly")));
        assert!(scopes.iter().any(|scope| scope.contains("calendar")));
    }

    #[test]
    fn infers_required_bundles_from_gws_argv() {
        assert_eq!(
            infer_required_bundles_from_gws_argv(&[
                "drive".to_string(),
                "files".to_string(),
                "list".to_string()
            ]),
            vec!["drive".to_string()]
        );
        assert_eq!(
            infer_required_bundles_from_gws_argv(&[
                "help".to_string(),
                "docs".to_string(),
                "--format".to_string(),
                "json".to_string()
            ]),
            vec!["docs".to_string()]
        );
        assert_eq!(
            infer_required_bundles_from_gws_argv(&[
                "admin-reports:v1".to_string(),
                "activities".to_string(),
                "list".to_string()
            ]),
            vec!["admin".to_string()]
        );
    }

    #[test]
    fn google_api_failure_preserves_reason_and_enable_url() {
        let rendered = format_google_api_failure(
            "Drive files request",
            Some(reqwest::StatusCode::FORBIDDEN),
            r#"{"error":{"code":403,"message":"Drive API disabled","reason":"accessNotConfigured","enable_url":"https://example.com/enable"}}"#,
        );
        assert!(rendered.contains("accessNotConfigured"));
        assert!(rendered.contains("Drive API disabled"));
        assert!(rendered.contains("https://example.com/enable"));
    }

    #[test]
    fn parses_generated_gws_skill_frontmatter() {
        let raw = r#"---
name: gws-docs
description: "Read and write Google Docs."
metadata:
  version: 0.22.3
  agentark:
    cliHelp: "gws docs --help"
---

# docs
"#;
        let parsed =
            parse_gws_skill_metadata(std::path::Path::new("skills/gws-docs/SKILL.md"), raw)
                .unwrap();
        assert_eq!(parsed.name, "gws-docs");
        assert_eq!(parsed.description, "Read and write Google Docs.");
        assert_eq!(parsed.cli_help.as_deref(), Some("gws docs --help"));
    }

    #[test]
    fn infers_related_bundle_from_generated_skill_metadata() {
        let skill = GwsSkillMetadata {
            name: "gws-drive-search".to_string(),
            description: "Find files in Google Drive.".to_string(),
            cli_help: Some("gws drive files list --help".to_string()),
            path: std::path::PathBuf::from("skills/gws-drive-search/SKILL.md"),
        };
        assert_eq!(gws_skill_related_bundles(&skill), vec!["drive".to_string()]);
    }

    #[test]
    fn hides_generated_skill_when_bundle_is_not_granted() {
        let skill = GwsSkillMetadata {
            name: "gws-admin".to_string(),
            description: "Inspect Google Workspace Admin users.".to_string(),
            cli_help: Some("gws admin users list --help".to_string()),
            path: std::path::PathBuf::from("skills/gws-admin/SKILL.md"),
        };
        assert!(!gws_skill_visible_for_granted_bundles(
            &skill,
            &["gmail".to_string(), "calendar".to_string()]
        ));
        assert!(gws_skill_visible_for_granted_bundles(
            &skill,
            &["admin".to_string()]
        ));
    }

    #[test]
    fn granted_bundle_visibility_rejects_ungranted_services() {
        let dir = tempdir().unwrap();
        let manager = crate::core::config::SecureConfigManager::new(dir.path()).unwrap();
        manager
            .set_custom_secret(
                GOOGLE_WORKSPACE_TOKENS_KEY,
                Some(
                    serde_json::json!({
                        "access_token": "access",
                        "refresh_token": "refresh",
                        "expires_at": Utc::now().timestamp() + 3600,
                        "granted_bundles": ["gmail"]
                    })
                    .to_string(),
                ),
            )
            .unwrap();
        let error = ensure_granted_bundle_visibility(dir.path(), &["drive".to_string()])
            .expect_err("drive should be hidden when only gmail is granted");
        assert!(error.to_string().contains("Drive"));
    }

    #[tokio::test]
    async fn falls_back_to_legacy_gmail_tokens_for_workspace_bundle() {
        let dir = tempdir().unwrap();
        let manager = crate::core::config::SecureConfigManager::new(dir.path()).unwrap();
        manager
            .set_custom_secret(
                "gmail_tokens",
                Some(
                    serde_json::json!({
                        "access_token": "legacy-gmail-token",
                        "refresh_token": "legacy-refresh-token",
                        "expires_at": Utc::now().timestamp() + 3600
                    })
                    .to_string(),
                ),
            )
            .unwrap();

        let token = ensure_access_token_for_bundles(dir.path(), &["gmail"])
            .await
            .unwrap();
        assert_eq!(token, "legacy-gmail-token");
    }

    #[tokio::test]
    async fn test_selected_bundles_reports_docs_and_sheets_access() {
        let dir = tempdir().unwrap();
        let app = Router::new()
            .route(
                "/v1/documents/__agentark_probe__",
                get(|| async { StatusCode::NOT_FOUND }),
            )
            .route(
                "/v4/spreadsheets/__agentark_probe__",
                get(|| async { StatusCode::NOT_FOUND }),
            );
        let listener = TcpListener::bind("127.0.0.1:0").await.unwrap();
        let addr = listener.local_addr().unwrap();
        crate::spawn_logged!("src/actions/google_workspace.rs:2263", async move {
            axum::serve(listener, app).await.unwrap();
        });
        std::fs::write(
            dir.path().join(".agentark_test_docs_api_base"),
            format!("http://{addr}/v1"),
        )
        .unwrap();
        std::fs::write(
            dir.path().join(".agentark_test_sheets_api_base"),
            format!("http://{addr}/v4"),
        )
        .unwrap();
        save_workspace_client_config(
            dir.path(),
            &GoogleWorkspaceClientConfig {
                client_id: "client".to_string(),
                client_secret: "secret".to_string(),
            },
        )
        .unwrap();
        save_selected_bundles(dir.path(), &["docs".to_string(), "sheets".to_string()]).unwrap();
        save_workspace_tokens(
            dir.path(),
            &GoogleWorkspaceTokens {
                access_token: "workspace-token".to_string(),
                refresh_token: "workspace-refresh".to_string(),
                expires_at: Utc::now().timestamp() + 3600,
                granted_scopes: scopes_for_bundles(&["docs".to_string(), "sheets".to_string()]),
                granted_bundles: vec!["docs".to_string(), "sheets".to_string()],
            },
        )
        .unwrap();

        let checks = test_selected_bundles(dir.path()).await.unwrap();
        assert_eq!(
            checks.get("docs").map(String::as_str),
            Some("Docs connected")
        );
        assert_eq!(
            checks.get("sheets").map(String::as_str),
            Some("Sheets connected")
        );
    }
}
