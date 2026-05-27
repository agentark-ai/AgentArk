//! App deployment: write files, optionally start a server, return a live URL.
//!
//! Supports any kind of app:
//! - Static HTML/JS/CSS -> served directly at /apps/{id}/
//! - Python server (FastAPI, Flask, etc.) -> started as subprocess, reverse-proxied
//! - Node.js server (Express, etc.) -> started as subprocess, reverse-proxied
//!
//! Dynamic apps get an auto-assigned port on localhost. The main HTTP server
//! reverse-proxies /apps/{id}/* to that port.

use anyhow::{Context, Result};
use scraper::{Html, Selector};
use sha2::{Digest, Sha256};
use std::collections::{BTreeMap, BTreeSet, HashMap, HashSet};
use std::path::{Path, PathBuf};
use std::process::Stdio;
use std::sync::Arc;
use tokio::io::{AsyncReadExt, AsyncWriteExt};
use tokio::sync::mpsc::Sender;
use tokio::sync::RwLock;

use crate::core::runtime_image;
use crate::core::StreamEvent;

/// Port range for dynamic apps (localhost only)
const PORT_RANGE_START: u16 = 9100;
const PORT_RANGE_END: u16 = 9200;
const DEFAULT_FALLBACK_APP_RUNTIME_IMAGE: &str = runtime_image::DEFAULT_RUNTIME_IMAGE;
const APP_CONTAINER_PREFIX: &str = "agentark-app-";
const MAX_APP_COMMAND_LEN: usize = 1024;
const LOCAL_RUNTIME_STDOUT_LOG_FILE: &str = ".agentark_runtime_stdout.log";
const LOCAL_RUNTIME_STDERR_LOG_FILE: &str = ".agentark_runtime_stderr.log";
pub(crate) const APP_QUALITY_REPORT_FILE: &str = "quality_report.json";
pub(crate) const APP_SUB_GOALS_FILE: &str = "sub_goals.json";
const LOCAL_RUNTIME_LOG_TAIL_BYTES: usize = 4096;
const DEFAULT_DYNAMIC_RUNTIME_READY_TIMEOUT_SECS: u64 = 120;
const DEFAULT_DYNAMIC_RUNTIME_INSTALL_TIMEOUT_SECS: u64 = 1_800;
const DEFAULT_DOCKER_LAUNCH_TIMEOUT_SECS: u64 = 120;
const DYNAMIC_RUNTIME_READY_POLL_MS: u64 = 500;
const DYNAMIC_RUNTIME_PROGRESS_INTERVAL_SECS: u64 = 5;
const APP_ACCESS_BOOTSTRAP_TTL_SECS: i64 = 10 * 60;
const APP_ACCESS_SESSION_TTL_SECS: i64 = 7 * 24 * 60 * 60;
const APP_ACCESS_BOOTSTRAP_MAX_TOKENS: usize = 4096;
const APP_ACCESS_SESSION_MAX_TOKENS: usize = 8192;
const MAX_REPO_CLONE_TIMEOUT_SECS: u64 = 240;
const REPO_DEPLOY_INFLIGHT_STALE_SECS: u64 = MAX_REPO_CLONE_TIMEOUT_SECS + 300;
const MAX_REPO_COMMAND_COUNT: usize = 120;
const MAX_REPO_TEXT_FILE_BYTES: usize = 512 * 1024;
const MAX_REPO_TOTAL_TEXT_BYTES: usize = 8 * 1024 * 1024;
const MAX_REPO_TEXT_FILES: usize = 600;
const MAX_README_BYTES: usize = 256 * 1024;

fn startup_restore_parallelism() -> usize {
    std::thread::available_parallelism()
        .map(|value| value.get().clamp(2, 8))
        .unwrap_or(4)
}

fn env_timeout_secs(name: &str, default_value: u64, min_value: u64) -> u64 {
    std::env::var(name)
        .ok()
        .and_then(|value| value.trim().parse::<u64>().ok())
        .filter(|value| *value >= min_value)
        .unwrap_or(default_value)
}

fn dynamic_runtime_ready_timeout_secs() -> u64 {
    env_timeout_secs(
        "AGENTARK_APP_RUNTIME_READY_TIMEOUT_SECS",
        DEFAULT_DYNAMIC_RUNTIME_READY_TIMEOUT_SECS,
        5,
    )
}

fn dynamic_runtime_install_timeout_secs() -> u64 {
    env_timeout_secs(
        "AGENTARK_APP_INSTALL_TIMEOUT_SECS",
        DEFAULT_DYNAMIC_RUNTIME_INSTALL_TIMEOUT_SECS,
        30,
    )
}

fn docker_launch_timeout_secs() -> u64 {
    env_timeout_secs(
        "AGENTARK_APP_DOCKER_LAUNCH_TIMEOUT_SECS",
        DEFAULT_DOCKER_LAUNCH_TIMEOUT_SECS,
        15,
    )
}

fn app_access_guard_enabled_for_deploy(
    expose_public: bool,
    requested_access_guard_enabled: bool,
    has_access_secret: bool,
) -> bool {
    requested_access_guard_enabled || (!expose_public && has_access_secret)
}

fn configured_runtime_image() -> Option<String> {
    runtime_image::configured_runtime_image_from_env()
}

fn control_plane_catalog_mode() -> bool {
    std::env::var("AGENTARK_STACK_ROLE")
        .ok()
        .is_some_and(|value| {
            let role = value.trim();
            role.eq_ignore_ascii_case("control-plane") || role.eq_ignore_ascii_case("control")
        })
}

fn control_plane_executor_client() -> Option<crate::clients::ExecutorClient> {
    if !control_plane_catalog_mode() {
        return None;
    }
    let client =
        crate::clients::ExecutorClient::new(crate::clients::ExecutorClientConfig::from_env())
            .ok()?;
    client.bearer_token()?;
    Some(client)
}

#[derive(Debug, Clone, PartialEq, Eq)]
enum StaticAssetReference {
    Bundled(String),
    RootAbsolute(String),
}

fn normalize_static_bundle_path(raw: &str) -> Option<String> {
    let normalized = raw.trim().replace('\\', "/");
    if normalized.is_empty() {
        return None;
    }
    let mut parts: Vec<&str> = Vec::new();
    for part in normalized.split('/') {
        if part.is_empty() || part == "." {
            continue;
        }
        if part == ".." {
            parts.pop()?;
            continue;
        }
        parts.push(part);
    }
    if parts.is_empty() {
        None
    } else {
        Some(parts.join("/"))
    }
}

fn has_url_scheme(raw: &str) -> bool {
    let Some(colon_idx) = raw.find(':') else {
        return false;
    };
    let prefix = &raw[..colon_idx];
    if prefix.is_empty() {
        return false;
    }
    prefix
        .chars()
        .all(|ch| ch.is_ascii_alphanumeric() || matches!(ch, '+' | '-' | '.'))
}

fn strip_url_suffixes(raw: &str) -> &str {
    let query_idx = raw.find('?');
    let hash_idx = raw.find('#');
    let end = match (query_idx, hash_idx) {
        (Some(a), Some(b)) => a.min(b),
        (Some(a), None) | (None, Some(a)) => a,
        (None, None) => raw.len(),
    };
    &raw[..end]
}

fn resolve_static_asset_reference(owner_file: &str, raw: &str) -> Option<StaticAssetReference> {
    let trimmed = raw.trim().trim_matches('"').trim_matches('\'').trim();
    if trimmed.is_empty()
        || trimmed.starts_with('#')
        || trimmed.starts_with("//")
        || has_url_scheme(trimmed)
    {
        return None;
    }
    let without_suffix = strip_url_suffixes(trimmed).trim();
    if without_suffix.is_empty() {
        return None;
    }
    if without_suffix.starts_with('/') {
        return Some(StaticAssetReference::RootAbsolute(
            without_suffix.to_string(),
        ));
    }

    let owner_dir = owner_file
        .rsplit_once('/')
        .map(|(dir, _)| dir)
        .filter(|dir| !dir.is_empty());
    let combined = match owner_dir {
        Some(dir) => format!("{}/{}", dir, without_suffix),
        None => without_suffix.to_string(),
    };
    normalize_static_bundle_path(&combined).map(StaticAssetReference::Bundled)
}

fn srcset_candidates(raw: &str) -> impl Iterator<Item = &str> {
    raw.split(',')
        .filter_map(|candidate| candidate.split_whitespace().next())
}

fn css_url_references(raw: &str) -> Vec<String> {
    let mut refs = Vec::new();
    let mut rest = raw;
    loop {
        let lower_rest = rest.to_ascii_lowercase();
        let Some(idx) = lower_rest.find("url(") else {
            break;
        };
        let after = &rest[idx + 4..];
        let Some(end) = after.find(')') else {
            break;
        };
        refs.push(after[..end].trim().to_string());
        rest = &after[end + 1..];
    }
    refs
}

fn html_tag_name_boundary(ch: Option<char>) -> bool {
    ch.is_none_or(|ch| ch.is_ascii_whitespace() || matches!(ch, '>' | '/' | '\t' | '\r' | '\n'))
}

fn html_raw_text_element_needs_close(tag_name: &str) -> bool {
    matches!(tag_name, "script" | "style" | "textarea" | "title")
}

pub(crate) fn detect_unclosed_html_raw_text_element(content: &str) -> Option<&'static str> {
    let lower = content.to_ascii_lowercase();
    let mut cursor = 0usize;

    while let Some(relative_start) = lower[cursor..].find('<') {
        let start = cursor + relative_start;
        let after_lt = start + 1;
        let next = lower[after_lt..].chars().next()?;

        if next == '!' || next == '?' || next == '/' {
            cursor = lower[after_lt..]
                .find('>')
                .map(|relative_end| after_lt + relative_end + 1)
                .unwrap_or(lower.len());
            continue;
        }

        let name_start = after_lt;
        let mut name_end = name_start;
        for (offset, ch) in lower[name_start..].char_indices() {
            if ch.is_ascii_alphanumeric() {
                name_end = name_start + offset + ch.len_utf8();
            } else {
                break;
            }
        }
        if name_end == name_start {
            cursor = after_lt;
            continue;
        }

        let tag_name = &lower[name_start..name_end];
        if !html_tag_name_boundary(lower[name_end..].chars().next()) {
            cursor = name_end;
            continue;
        }
        let Some(start_tag_end_relative) = lower[start..].find('>') else {
            return match tag_name {
                "script" => Some("script"),
                "style" => Some("style"),
                "textarea" => Some("textarea"),
                "title" => Some("title"),
                _ => None,
            };
        };
        let start_tag_end = start + start_tag_end_relative;
        if !html_raw_text_element_needs_close(tag_name) {
            cursor = start_tag_end + 1;
            continue;
        }
        let close_prefix = format!("</{}", tag_name);
        let body_start = start_tag_end + 1;
        let Some(close_relative) = lower[body_start..].find(&close_prefix) else {
            return match tag_name {
                "script" => Some("script"),
                "style" => Some("style"),
                "textarea" => Some("textarea"),
                "title" => Some("title"),
                _ => None,
            };
        };
        let close_start = body_start + close_relative;
        let close_name_end = close_start + close_prefix.len();
        if !html_tag_name_boundary(lower[close_name_end..].chars().next()) {
            cursor = close_name_end;
            continue;
        }
        cursor = lower[close_start..]
            .find('>')
            .map(|relative_end| close_start + relative_end + 1)
            .unwrap_or(lower.len());
    }

    None
}

fn validate_static_app_html_structure(
    files: &serde_json::Map<String, serde_json::Value>,
) -> Result<()> {
    let mut malformed: Vec<String> = Vec::new();
    for (filename, content) in files {
        let Some(owner_file) = normalize_static_bundle_path(filename) else {
            continue;
        };
        let lower_name = owner_file.to_ascii_lowercase();
        if !lower_name.ends_with(".html") && !lower_name.ends_with(".htm") {
            continue;
        }
        let Some(content) = content.as_str() else {
            continue;
        };
        if let Some(tag_name) = detect_unclosed_html_raw_text_element(content) {
            malformed.push(format!(
                "{} has an unclosed <{}> block",
                owner_file, tag_name
            ));
        }
    }

    if malformed.is_empty() {
        return Ok(());
    }

    malformed.sort();
    malformed.dedup();
    anyhow::bail!(
        "Static app bundle contains malformed HTML ({}). Redeploy a complete HTML document with all raw-text blocks closed; for generated pages with substantial styling or scripting, use separate app-relative files such as style.css and app.js and include them in the files object.",
        malformed
            .iter()
            .take(8)
            .cloned()
            .collect::<Vec<_>>()
            .join("; ")
    );
}

fn record_static_asset_reference(
    owner_file: &str,
    raw_ref: &str,
    available_files: &HashSet<String>,
    missing_refs: &mut Vec<String>,
) {
    match resolve_static_asset_reference(owner_file, raw_ref) {
        Some(StaticAssetReference::Bundled(path)) => {
            if !available_files.contains(&path) {
                missing_refs.push(format!(
                    "{} references missing local asset {}",
                    owner_file, path
                ));
            }
        }
        Some(StaticAssetReference::RootAbsolute(path)) => {
            let bundled_path = normalize_static_bundle_path(path.trim_start_matches('/'));
            if let Some(bundled_path) = bundled_path {
                if !available_files.contains(&bundled_path) {
                    missing_refs.push(format!(
                        "{} references missing root-relative local asset {}",
                        owner_file, path
                    ));
                }
            }
        }
        None => {}
    }
}

fn validate_static_app_asset_references(
    files: &serde_json::Map<String, serde_json::Value>,
) -> Result<()> {
    let available_files: HashSet<String> = files
        .keys()
        .filter_map(|name| normalize_static_bundle_path(name))
        .collect();
    validate_static_app_html_structure(files)?;
    let mut missing_refs: Vec<String> = Vec::new();

    let html_selectors = [
        ("link[href]", "href"),
        ("script[src]", "src"),
        ("img[src]", "src"),
        ("source[src]", "src"),
        ("video[src]", "src"),
        ("audio[src]", "src"),
        ("iframe[src]", "src"),
        ("object[data]", "data"),
    ];
    let parsed_selectors: Vec<(Selector, &str)> = html_selectors
        .iter()
        .filter_map(|(selector, attr)| Selector::parse(selector).ok().map(|s| (s, *attr)))
        .collect();
    let srcset_selector = Selector::parse("[srcset]").ok();

    for (filename, content) in files {
        let Some(owner_file) = normalize_static_bundle_path(filename) else {
            continue;
        };
        let Some(content) = content.as_str() else {
            continue;
        };
        let lower_name = owner_file.to_ascii_lowercase();
        if lower_name.ends_with(".html") || lower_name.ends_with(".htm") {
            let document = Html::parse_document(content);
            for (selector, attr) in &parsed_selectors {
                for element in document.select(selector) {
                    if let Some(raw) = element.value().attr(attr) {
                        record_static_asset_reference(
                            &owner_file,
                            raw,
                            &available_files,
                            &mut missing_refs,
                        );
                    }
                }
            }
            if let Some(selector) = &srcset_selector {
                for element in document.select(selector) {
                    if let Some(raw) = element.value().attr("srcset") {
                        for candidate in srcset_candidates(raw) {
                            record_static_asset_reference(
                                &owner_file,
                                candidate,
                                &available_files,
                                &mut missing_refs,
                            );
                        }
                    }
                }
            }
        } else if lower_name.ends_with(".css") {
            for raw_ref in css_url_references(content) {
                record_static_asset_reference(
                    &owner_file,
                    &raw_ref,
                    &available_files,
                    &mut missing_refs,
                );
            }
        }
    }

    if missing_refs.is_empty() {
        return Ok(());
    }

    missing_refs.sort();
    missing_refs.dedup();
    let mut details = Vec::new();
    if !missing_refs.is_empty() {
        details.push(format!(
            "missing bundled files: {}",
            missing_refs
                .iter()
                .take(8)
                .cloned()
                .collect::<Vec<_>>()
                .join("; ")
        ));
    }
    anyhow::bail!(
        "Static app bundle has unresolved local asset references ({})",
        details.join(" | ")
    );
}

async fn restart_delegated_runtime(
    app_id: &str,
    title: &str,
    access_guard_enabled: bool,
    access_key: &str,
    expose_public: bool,
) -> Result<serde_json::Value> {
    let executor = control_plane_executor_client()
        .ok_or_else(|| anyhow::anyhow!("Executor service is not configured"))?;
    let response = executor
        .request(
            reqwest::Method::POST,
            &format!("/internal/v1/apps/{}/restart", app_id),
        )
        .json(&crate::clients::AppLifecycleRequest {
            title: Some(title.to_string()),
            query: None,
        })
        .send()
        .await?;
    if !response.status().is_success() {
        let payload = response
            .json::<serde_json::Value>()
            .await
            .unwrap_or_else(|_| serde_json::json!({}));
        anyhow::bail!(
            "{}",
            payload
                .get("message")
                .and_then(|value| value.as_str())
                .unwrap_or("executor restart failed")
        );
    }

    let payload = response
        .json::<serde_json::Value>()
        .await
        .unwrap_or_else(|_| serde_json::json!({}));
    let raw = payload
        .get("raw")
        .cloned()
        .unwrap_or_else(|| serde_json::json!({}));
    let mode = raw
        .get("mode")
        .and_then(|value| value.as_str())
        .unwrap_or("dynamic");
    let url = format!("/apps/{}/", app_id);
    let access_url = url.clone();
    Ok(serde_json::json!({
        "status": "deployed",
        "type": mode,
        "app_id": app_id,
        "title": raw.get("title").and_then(|value| value.as_str()).unwrap_or(title),
        "url": url,
        "access_url": access_url,
        "access_key": access_key,
        "access_password": access_key,
        "access_guard_enabled": access_guard_enabled,
        "public_access_guard_enabled": access_guard_enabled || expose_public,
        "expose_public": expose_public,
        "apps_page_hint": APP_DEPLOY_CONTROL_HINT,
        "port": raw.get("port").cloned().unwrap_or(serde_json::Value::Null),
        "runtime_preference": raw
            .get("runtime_mode")
            .and_then(|value| value.as_str())
            .unwrap_or("executor"),
        "enabled": true
    }))
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum RepoServiceMode {
    Auto,
    Frontend,
    Backend,
    Fullstack,
}

fn repo_service_mode_from_opt(raw: Option<&str>) -> RepoServiceMode {
    match raw.unwrap_or("").trim().to_ascii_lowercase().as_str() {
        "frontend" | "front-end" | "ui" | "web" => RepoServiceMode::Frontend,
        "backend" | "back-end" | "api" | "server" => RepoServiceMode::Backend,
        "fullstack" | "full-stack" | "all" => RepoServiceMode::Fullstack,
        _ => RepoServiceMode::Auto,
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum RepoServiceKind {
    Frontend,
    Backend,
    Fullstack,
    Static,
}

impl RepoServiceKind {
    fn as_str(self) -> &'static str {
        match self {
            RepoServiceKind::Frontend => "frontend",
            RepoServiceKind::Backend => "backend",
            RepoServiceKind::Fullstack => "fullstack",
            RepoServiceKind::Static => "static",
        }
    }

    fn matches_mode(self, mode: RepoServiceMode) -> bool {
        match mode {
            RepoServiceMode::Auto | RepoServiceMode::Fullstack => true,
            RepoServiceMode::Frontend => {
                matches!(
                    self,
                    RepoServiceKind::Frontend
                        | RepoServiceKind::Fullstack
                        | RepoServiceKind::Static
                )
            }
            RepoServiceMode::Backend => {
                matches!(self, RepoServiceKind::Backend | RepoServiceKind::Fullstack)
            }
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum RepoCopyScope {
    RepositoryRoot,
    ServiceRoot,
}

#[derive(Debug, Clone, Default)]
struct RepoReadmeHints {
    install_command: Option<String>,
    start_command: Option<String>,
    mentions_compose: bool,
}

#[derive(Debug, Clone, Default)]
struct RepoNodeManifest {
    name: Option<String>,
    scripts: HashSet<String>,
    dependencies: HashSet<String>,
    has_workspaces: bool,
}

#[derive(Debug, Clone)]
struct RepoServicePlan {
    title: String,
    relative_dir: String,
    kind: RepoServiceKind,
    copy_scope: RepoCopyScope,
    install_command: Option<String>,
    entry_command: Option<String>,
    required_inputs: Vec<AppRequiredInput>,
    detection_reason: String,
}

fn normalize_repo_relative_path(path: &Path) -> String {
    let raw = path.to_string_lossy().replace('\\', "/");
    let trimmed = raw.trim_matches('/');
    trimmed
        .trim_start_matches("./")
        .trim_end_matches('/')
        .to_string()
}

fn humanize_repo_label(raw: &str) -> String {
    let mut parts = Vec::new();
    for token in raw
        .split(|ch: char| !(ch.is_ascii_alphanumeric()))
        .filter(|token| !token.is_empty())
    {
        let mut chars = token.chars();
        let Some(first) = chars.next() else {
            continue;
        };
        let mut rebuilt = String::new();
        rebuilt.push(first.to_ascii_uppercase());
        rebuilt.push_str(&chars.as_str().to_ascii_lowercase());
        parts.push(rebuilt);
    }
    if parts.is_empty() {
        "Repo".to_string()
    } else {
        parts.join(" ")
    }
}

fn repo_title_from_url(repo_url: &str) -> String {
    let fallback = humanize_repo_label(
        repo_url
            .rsplit('/')
            .next()
            .unwrap_or("repo")
            .trim_end_matches(".git"),
    );
    let Ok(parsed) = reqwest::Url::parse(repo_url) else {
        return fallback;
    };
    parsed
        .path_segments()
        .and_then(|mut segments| segments.rfind(|segment| !segment.is_empty()))
        .map(|segment| humanize_repo_label(segment.trim_end_matches(".git")))
        .unwrap_or(fallback)
}

fn build_repo_service_title(repo_title: &str, relative_dir: &str, kind: RepoServiceKind) -> String {
    if relative_dir.trim().is_empty() {
        return repo_title.to_string();
    }
    let segment = relative_dir
        .rsplit('/')
        .find(|part| !part.trim().is_empty())
        .unwrap_or(relative_dir);
    let label = humanize_repo_label(segment);
    if label.eq_ignore_ascii_case(kind.as_str()) {
        format!("{} {}", repo_title, label)
    } else {
        format!(
            "{} {} {}",
            repo_title,
            label,
            humanize_repo_label(kind.as_str())
        )
    }
}

fn repo_dir_name_hint(relative_dir: &str) -> Option<RepoServiceKind> {
    let lower = relative_dir.to_ascii_lowercase();
    let segment = lower
        .rsplit('/')
        .find(|part| !part.trim().is_empty())
        .unwrap_or(lower.as_str());
    if [
        "frontend",
        "front",
        "client",
        "web",
        "ui",
        "site",
        "app",
        "dashboard",
    ]
    .iter()
    .any(|needle| segment.contains(needle))
    {
        return Some(RepoServiceKind::Frontend);
    }
    if ["backend", "back", "api", "server", "svc", "service"]
        .iter()
        .any(|needle| segment.contains(needle))
    {
        return Some(RepoServiceKind::Backend);
    }
    None
}

fn is_allowed_repo_url(repo_url: &str) -> Result<reqwest::Url> {
    let parsed = reqwest::Url::parse(repo_url)
        .with_context(|| format!("invalid repo_url '{}'", repo_url))?;
    match parsed.scheme() {
        "https" | "http" => {}
        other => anyhow::bail!("unsupported repo_url scheme '{}': use http/https", other),
    }
    let host = parsed
        .host_str()
        .ok_or_else(|| anyhow::anyhow!("repo_url must include a host"))?;
    let lower_host = host.trim().to_ascii_lowercase();
    if lower_host == "localhost" || lower_host.ends_with(".local") {
        anyhow::bail!("repo_url must not target localhost or .local hosts");
    }
    if let Ok(ip) = lower_host.parse::<std::net::IpAddr>() {
        let blocked = match ip {
            std::net::IpAddr::V4(v4) => {
                v4.is_private() || v4.is_loopback() || v4.is_link_local() || v4.is_unspecified()
            }
            std::net::IpAddr::V6(v6) => {
                v6.is_loopback() || v6.is_unspecified() || v6.is_unique_local()
            }
        };
        if blocked {
            anyhow::bail!("repo_url must not target a private or loopback address");
        }
    }
    Ok(parsed)
}

fn should_skip_repo_dir(entry: &walkdir::DirEntry) -> bool {
    if !entry.file_type().is_dir() {
        return true;
    }
    let name = entry.file_name().to_string_lossy().to_ascii_lowercase();
    !matches!(
        name.as_str(),
        ".git"
            | "node_modules"
            | ".next"
            | ".nuxt"
            | ".turbo"
            | "dist"
            | "build"
            | "coverage"
            | ".venv"
            | "venv"
            | ".agentark"
            | "__pycache__"
            | "target"
            | ".idea"
            | ".vscode"
    )
}

fn read_text_file_limited(path: &Path, max_bytes: usize) -> Option<String> {
    let bytes = std::fs::read(path).ok()?;
    if bytes.is_empty() || bytes.len() > max_bytes {
        return None;
    }
    String::from_utf8(bytes).ok()
}

fn discover_readme_path(dir: &Path) -> Option<PathBuf> {
    for candidate in [
        "README.md",
        "README.MD",
        "README.txt",
        "README",
        "readme.md",
        "readme.txt",
        "readme",
    ] {
        let path = dir.join(candidate);
        if path.exists() {
            return Some(path);
        }
    }
    None
}

fn normalize_readme_command_line(line: &str) -> Option<String> {
    let mut trimmed = line.trim();
    if trimmed.is_empty()
        || trimmed.starts_with('#')
        || trimmed.starts_with("```")
        || trimmed.starts_with("<!--")
    {
        return None;
    }
    if let Some(stripped) = trimmed.strip_prefix("$ ") {
        trimmed = stripped.trim();
    }
    if let Some(stripped) = trimmed.strip_prefix("- ") {
        trimmed = stripped.trim();
    }
    if let Some(stripped) = trimmed.strip_prefix("* ") {
        trimmed = stripped.trim();
    }
    let trimmed = trimmed.trim_matches('`').trim();
    if trimmed.is_empty() {
        None
    } else {
        Some(trimmed.to_string())
    }
}

fn extract_readme_hints(readme: &str) -> RepoReadmeHints {
    let mut hints = RepoReadmeHints::default();
    for line in readme.lines() {
        let Some(command) = normalize_readme_command_line(line) else {
            continue;
        };
        let lower = command.to_ascii_lowercase();
        if lower.contains("docker compose") || lower.contains("docker-compose") {
            hints.mentions_compose = true;
        }
        if hints.install_command.is_none()
            && [
                "npm install",
                "npm ci",
                "pnpm install",
                "yarn install",
                "pip install",
                "poetry install",
                "uv sync",
                "cargo build",
            ]
            .iter()
            .any(|needle| lower.starts_with(needle))
        {
            hints.install_command = Some(command.clone());
        }
        if hints.start_command.is_none()
            && [
                "npm run dev",
                "npm run start",
                "pnpm dev",
                "pnpm start",
                "yarn dev",
                "yarn start",
                "uvicorn ",
                "python ",
                "streamlit run",
                "flask run",
                "cargo run",
                "docker compose up",
                "docker-compose up",
            ]
            .iter()
            .any(|needle| lower.starts_with(needle))
        {
            hints.start_command = Some(command);
        }
    }
    hints
}

fn load_readme_hints(dir: &Path) -> Option<(String, RepoReadmeHints)> {
    let path = discover_readme_path(dir)?;
    let content = read_text_file_limited(&path, MAX_README_BYTES)?;
    let relative = path
        .file_name()
        .map(|value| value.to_string_lossy().to_string())
        .unwrap_or_else(|| "README".to_string());
    Some((relative, extract_readme_hints(&content)))
}

fn parse_node_manifest_value(parsed: &serde_json::Value) -> RepoNodeManifest {
    let mut manifest = RepoNodeManifest {
        name: parsed
            .get("name")
            .and_then(|value| value.as_str())
            .map(|value| value.trim().to_string())
            .filter(|value| !value.is_empty()),
        ..RepoNodeManifest::default()
    };
    if parsed.get("workspaces").is_some() {
        manifest.has_workspaces = true;
    }
    for key in [
        "scripts",
        "dependencies",
        "devDependencies",
        "optionalDependencies",
    ] {
        if let Some(obj) = parsed.get(key).and_then(|value| value.as_object()) {
            if key == "scripts" {
                manifest.scripts.extend(obj.keys().cloned());
            } else {
                manifest.dependencies.extend(
                    obj.keys()
                        .map(|value| value.to_ascii_lowercase())
                        .collect::<HashSet<_>>(),
                );
            }
        }
    }
    manifest
}

fn parse_node_manifest_text(raw: &str) -> Option<RepoNodeManifest> {
    let parsed: serde_json::Value = serde_json::from_str(raw).ok()?;
    Some(parse_node_manifest_value(&parsed))
}

fn load_node_manifest(dir: &Path) -> Option<RepoNodeManifest> {
    let raw = read_text_file_limited(&dir.join("package.json"), MAX_REPO_TEXT_FILE_BYTES)?;
    parse_node_manifest_text(&raw)
}

fn load_python_dependency_text(dir: &Path) -> String {
    let mut combined = String::new();
    for candidate in ["requirements.txt", "pyproject.toml"] {
        let path = dir.join(candidate);
        if let Some(text) = read_text_file_limited(&path, MAX_REPO_TEXT_FILE_BYTES) {
            if !combined.is_empty() {
                combined.push('\n');
            }
            combined.push_str(&text);
        }
    }
    combined
}

fn first_existing_file(dir: &Path, names: &[&str]) -> Option<PathBuf> {
    names
        .iter()
        .map(|name| dir.join(name))
        .find(|path| path.exists())
}

fn build_relative_file_arg(relative_dir: &str, filename: &str) -> String {
    if relative_dir.trim().is_empty() {
        filename.to_string()
    } else {
        format!("{}/{}", relative_dir.trim_end_matches('/'), filename)
    }
}

fn detect_fastapi_entry(dir: &Path) -> Option<PathBuf> {
    for candidate in ["main.py", "app.py", "server.py", "api.py"] {
        let path = dir.join(candidate);
        let Some(text) = read_text_file_limited(&path, MAX_REPO_TEXT_FILE_BYTES) else {
            continue;
        };
        if text.contains("FastAPI(") || text.contains("from fastapi import") {
            return Some(path);
        }
    }
    None
}

fn detect_flask_entry(dir: &Path) -> Option<PathBuf> {
    for candidate in ["app.py", "main.py", "server.py", "wsgi.py"] {
        let path = dir.join(candidate);
        let Some(text) = read_text_file_limited(&path, MAX_REPO_TEXT_FILE_BYTES) else {
            continue;
        };
        if text.contains("Flask(") || text.contains("from flask import") {
            return Some(path);
        }
    }
    None
}

fn build_python_commands(
    dir: &Path,
    relative_dir: &str,
) -> Option<(RepoServiceKind, Option<String>, String)> {
    let dependency_text = load_python_dependency_text(dir).to_ascii_lowercase();
    let requirements_path = dir.join("requirements.txt");
    let pyproject_path = dir.join("pyproject.toml");
    let install_command = if requirements_path.exists() {
        Some(format!(
            "pip install -r {} -q",
            shell_quote_arg(&build_relative_file_arg(relative_dir, "requirements.txt"))
        ))
    } else if pyproject_path.exists() {
        Some(if relative_dir.trim().is_empty() {
            "pip install -e .".to_string()
        } else {
            format!("pip install -e {}", shell_quote_arg(relative_dir))
        })
    } else {
        None
    };

    if dir.join("manage.py").exists() {
        return Some((
            RepoServiceKind::Backend,
            install_command,
            format!(
                "python {} runserver 0.0.0.0:{{PORT}}",
                shell_quote_arg(&build_relative_file_arg(relative_dir, "manage.py"))
            ),
        ));
    }

    if dependency_text.contains("streamlit") {
        if let Some(entry) = first_existing_file(dir, &["app.py", "main.py", "streamlit_app.py"]) {
            let rel = normalize_repo_relative_path(entry.strip_prefix(dir).ok().unwrap_or(&entry));
            return Some((
                RepoServiceKind::Fullstack,
                install_command,
                format!(
                    "streamlit run {} --server.address 0.0.0.0 --server.port {{PORT}}",
                    shell_quote_arg(&build_relative_file_arg(relative_dir, &rel))
                ),
            ));
        }
    }

    if let Some(entry) = detect_fastapi_entry(dir) {
        let rel_dir = entry.parent().unwrap_or(dir);
        let app_dir = if rel_dir == dir {
            relative_dir.to_string()
        } else {
            let nested =
                normalize_repo_relative_path(rel_dir.strip_prefix(dir).ok().unwrap_or(rel_dir));
            if relative_dir.trim().is_empty() {
                nested
            } else if nested.is_empty() {
                relative_dir.to_string()
            } else {
                format!("{}/{}", relative_dir.trim_end_matches('/'), nested)
            }
        };
        let module = entry
            .file_stem()
            .and_then(|value| value.to_str())
            .unwrap_or("app");
        let app_dir_arg = if app_dir.trim().is_empty() {
            ".".to_string()
        } else {
            app_dir
        };
        return Some((
            RepoServiceKind::Backend,
            install_command,
            format!(
                "uvicorn --app-dir {} {}:app --host 0.0.0.0 --port {{PORT}}",
                shell_quote_arg(&app_dir_arg),
                module
            ),
        ));
    }

    if let Some(entry) = detect_flask_entry(dir) {
        let rel = normalize_repo_relative_path(entry.strip_prefix(dir).ok().unwrap_or(&entry));
        return Some((
            RepoServiceKind::Backend,
            install_command,
            format!(
                "flask --app {} run --host 0.0.0.0 --port {{PORT}}",
                shell_quote_arg(&build_relative_file_arg(relative_dir, &rel))
            ),
        ));
    }

    if dependency_text.contains("gradio") {
        if let Some(entry) = first_existing_file(dir, &["app.py", "main.py"]) {
            let rel = normalize_repo_relative_path(entry.strip_prefix(dir).ok().unwrap_or(&entry));
            return Some((
                RepoServiceKind::Fullstack,
                install_command,
                format!(
                    "python {}",
                    shell_quote_arg(&build_relative_file_arg(relative_dir, &rel))
                ),
            ));
        }
    }

    if let Some(entry) = first_existing_file(dir, &["server.py", "app.py", "main.py", "run.py"]) {
        let rel = normalize_repo_relative_path(entry.strip_prefix(dir).ok().unwrap_or(&entry));
        return Some((
            repo_dir_name_hint(relative_dir).unwrap_or(RepoServiceKind::Backend),
            install_command,
            format!(
                "python {}",
                shell_quote_arg(&build_relative_file_arg(relative_dir, &rel))
            ),
        ));
    }

    None
}

fn build_rust_commands(
    dir: &Path,
    relative_dir: &str,
) -> Option<(RepoServiceKind, Option<String>, String)> {
    if !dir.join("Cargo.toml").exists() {
        return None;
    }
    let entry_command = if relative_dir.trim().is_empty() {
        "cargo run".to_string()
    } else {
        format!(
            "cargo run --manifest-path {}",
            shell_quote_arg(&build_relative_file_arg(relative_dir, "Cargo.toml"))
        )
    };
    Some((
        repo_dir_name_hint(relative_dir).unwrap_or(RepoServiceKind::Backend),
        None,
        entry_command,
    ))
}

fn classify_node_service_kind(manifest: &RepoNodeManifest, relative_dir: &str) -> RepoServiceKind {
    let deps = &manifest.dependencies;
    let has_frontend_framework = deps.iter().any(|dep| {
        matches!(
            dep.as_str(),
            "react"
                | "react-dom"
                | "vite"
                | "next"
                | "vue"
                | "nuxt"
                | "svelte"
                | "@sveltejs/kit"
                | "astro"
                | "gatsby"
                | "@angular/core"
                | "remix"
        )
    });
    let has_backend_framework = deps.iter().any(|dep| {
        matches!(
            dep.as_str(),
            "express" | "koa" | "fastify" | "hapi" | "@nestjs/core" | "@nestjs/common" | "restify"
        )
    });
    if deps.contains("next")
        || deps.contains("nuxt")
        || deps.contains("@sveltejs/kit")
        || deps.contains("remix")
    {
        return RepoServiceKind::Fullstack;
    }
    if has_frontend_framework && has_backend_framework {
        return RepoServiceKind::Fullstack;
    }
    if has_frontend_framework {
        return RepoServiceKind::Frontend;
    }
    if has_backend_framework {
        return RepoServiceKind::Backend;
    }
    repo_dir_name_hint(relative_dir).unwrap_or(RepoServiceKind::Backend)
}

fn build_node_run_command(
    manifest: &RepoNodeManifest,
    relative_dir: &str,
    script: &str,
    extra_args: &[&str],
    root_has_workspaces: bool,
) -> String {
    let workspace_name = manifest.name.as_deref().filter(|_| root_has_workspaces);
    let mut command = if let Some(name) = workspace_name {
        format!("npm run {} --workspace={}", script, shell_quote_arg(name))
    } else if relative_dir.trim().is_empty() {
        format!("npm run {}", script)
    } else {
        format!(
            "npm --prefix {} run {}",
            shell_quote_arg(relative_dir),
            script
        )
    };
    if !extra_args.is_empty() {
        command.push_str(" -- ");
        command.push_str(&extra_args.join(" "));
    }
    command
}

fn build_node_commands(
    dir: &Path,
    relative_dir: &str,
    manifest: &RepoNodeManifest,
    root_has_workspaces: bool,
) -> Option<(RepoServiceKind, String, String)> {
    build_node_commands_from_manifest(relative_dir, manifest, root_has_workspaces, |entry| {
        dir.join(entry).exists()
    })
}

fn build_node_commands_from_manifest<F>(
    relative_dir: &str,
    manifest: &RepoNodeManifest,
    root_has_workspaces: bool,
    entry_file_exists: F,
) -> Option<(RepoServiceKind, String, String)>
where
    F: Fn(&str) -> bool,
{
    let kind = classify_node_service_kind(manifest, relative_dir);
    let install_command = if relative_dir.trim().is_empty() || root_has_workspaces {
        "npm install --omit=dev".to_string()
    } else {
        format!(
            "npm --prefix {} install --omit=dev",
            shell_quote_arg(relative_dir)
        )
    };

    let frontend_args = if manifest.dependencies.contains("next") {
        vec!["--hostname", "0.0.0.0", "--port", "{PORT}"]
    } else {
        vec!["--host", "0.0.0.0", "--port", "{PORT}"]
    };

    let entry_command = if manifest.scripts.contains("preview")
        && matches!(kind, RepoServiceKind::Frontend | RepoServiceKind::Fullstack)
    {
        build_node_run_command(
            manifest,
            relative_dir,
            "preview",
            &frontend_args,
            root_has_workspaces,
        )
    } else if manifest.scripts.contains("start") {
        if manifest.dependencies.contains("next") {
            build_node_run_command(
                manifest,
                relative_dir,
                "start",
                &["--hostname", "0.0.0.0", "--port", "{PORT}"],
                root_has_workspaces,
            )
        } else {
            build_node_run_command(manifest, relative_dir, "start", &[], root_has_workspaces)
        }
    } else if manifest.scripts.contains("dev") {
        if matches!(kind, RepoServiceKind::Frontend | RepoServiceKind::Fullstack) {
            build_node_run_command(
                manifest,
                relative_dir,
                "dev",
                &frontend_args,
                root_has_workspaces,
            )
        } else {
            build_node_run_command(manifest, relative_dir, "dev", &[], root_has_workspaces)
        }
    } else if entry_file_exists("server.js") {
        let path = build_relative_file_arg(relative_dir, "server.js");
        format!("node {}", shell_quote_arg(&path))
    } else if entry_file_exists("app.js") {
        let path = build_relative_file_arg(relative_dir, "app.js");
        format!("node {}", shell_quote_arg(&path))
    } else if entry_file_exists("index.js") && matches!(kind, RepoServiceKind::Backend) {
        let path = build_relative_file_arg(relative_dir, "index.js");
        format!("node {}", shell_quote_arg(&path))
    } else {
        return None;
    };

    Some((kind, install_command, entry_command))
}

#[derive(Debug, Clone)]
struct GeneratedBundleLifecycleInference {
    install_command: Option<String>,
    entry_command: String,
    runtime_reason: String,
}

fn text_files_from_effective_bundle(
    files: &serde_json::Map<String, serde_json::Value>,
) -> HashMap<String, String> {
    files
        .iter()
        .filter_map(|(path, value)| {
            value.as_str().map(|content| {
                (
                    normalize_repo_relative_path(Path::new(path)),
                    content.to_string(),
                )
            })
        })
        .filter(|(path, _)| !path.is_empty())
        .collect()
}

fn bundle_parent_dir(path: &str, filename: &str) -> Option<String> {
    let normalized = normalize_repo_relative_path(Path::new(path));
    if normalized == filename {
        return Some(String::new());
    }
    normalized
        .strip_suffix(&format!("/{}", filename))
        .map(|parent| parent.trim_matches('/').to_string())
}

fn bundle_candidate_dirs_with_file(files: &HashMap<String, String>, filename: &str) -> Vec<String> {
    let mut dirs = files
        .keys()
        .filter_map(|path| bundle_parent_dir(path, filename))
        .collect::<Vec<_>>();
    dirs.sort_by_key(|dir| (dir.matches('/').count(), dir.len(), dir.clone()));
    dirs.dedup();
    dirs
}

fn bundle_file_path(relative_dir: &str, filename: &str) -> String {
    normalize_repo_relative_path(Path::new(&build_relative_file_arg(relative_dir, filename)))
}

fn bundle_file_exists(files: &HashMap<String, String>, relative_dir: &str, filename: &str) -> bool {
    files.contains_key(&bundle_file_path(relative_dir, filename))
}

fn bundle_file_text<'a>(
    files: &'a HashMap<String, String>,
    relative_dir: &str,
    filename: &str,
) -> Option<&'a str> {
    files
        .get(&bundle_file_path(relative_dir, filename))
        .map(String::as_str)
}

fn infer_generated_node_bundle_lifecycle(
    files: &HashMap<String, String>,
) -> Option<GeneratedBundleLifecycleInference> {
    let root_has_workspaces = files
        .get("package.json")
        .and_then(|raw| parse_node_manifest_text(raw))
        .map(|manifest| manifest.has_workspaces)
        .unwrap_or(false);

    for relative_dir in bundle_candidate_dirs_with_file(files, "package.json") {
        let Some(manifest) = bundle_file_text(files, &relative_dir, "package.json")
            .and_then(parse_node_manifest_text)
        else {
            continue;
        };
        let Some((_kind, install_command, entry_command)) = build_node_commands_from_manifest(
            &relative_dir,
            &manifest,
            root_has_workspaces,
            |entry| bundle_file_exists(files, &relative_dir, entry),
        ) else {
            continue;
        };
        return Some(GeneratedBundleLifecycleInference {
            install_command: Some(install_command),
            entry_command,
            runtime_reason: "generated bundle contains a runnable Node package manifest"
                .to_string(),
        });
    }
    None
}

fn generated_python_dependency_text(files: &HashMap<String, String>, relative_dir: &str) -> String {
    let mut combined = String::new();
    for candidate in ["requirements.txt", "pyproject.toml"] {
        if let Some(text) = bundle_file_text(files, relative_dir, candidate) {
            if !combined.is_empty() {
                combined.push('\n');
            }
            combined.push_str(text);
        }
    }
    combined.to_ascii_lowercase()
}

fn generated_python_install_command(
    files: &HashMap<String, String>,
    relative_dir: &str,
) -> Option<String> {
    if bundle_file_exists(files, relative_dir, "requirements.txt") {
        return Some(format!(
            "pip install -r {} -q",
            shell_quote_arg(&build_relative_file_arg(relative_dir, "requirements.txt"))
        ));
    }
    if bundle_file_exists(files, relative_dir, "pyproject.toml") {
        return Some(if relative_dir.trim().is_empty() {
            "pip install -e .".to_string()
        } else {
            format!("pip install -e {}", shell_quote_arg(relative_dir))
        });
    }
    None
}

fn generated_python_entry_content<'a>(
    files: &'a HashMap<String, String>,
    relative_dir: &str,
    candidates: &[&str],
) -> Option<(String, &'a str)> {
    for candidate in candidates {
        if let Some(text) = bundle_file_text(files, relative_dir, candidate) {
            return Some((candidate.to_string(), text));
        }
    }
    None
}

fn infer_generated_python_bundle_lifecycle(
    files: &HashMap<String, String>,
) -> Option<GeneratedBundleLifecycleInference> {
    let mut dirs = Vec::new();
    for filename in [
        "requirements.txt",
        "pyproject.toml",
        "manage.py",
        "server.py",
        "app.py",
        "main.py",
        "run.py",
        "streamlit_app.py",
    ] {
        dirs.extend(bundle_candidate_dirs_with_file(files, filename));
    }
    dirs.sort_by_key(|dir| (dir.matches('/').count(), dir.len(), dir.clone()));
    dirs.dedup();

    for relative_dir in dirs {
        let dependency_text = generated_python_dependency_text(files, &relative_dir);
        let install_command = generated_python_install_command(files, &relative_dir);

        if bundle_file_exists(files, &relative_dir, "manage.py") {
            return Some(GeneratedBundleLifecycleInference {
                install_command,
                entry_command: format!(
                    "python {} runserver 0.0.0.0:{{PORT}}",
                    shell_quote_arg(&build_relative_file_arg(&relative_dir, "manage.py"))
                ),
                runtime_reason: "generated bundle contains a Python web project entry point"
                    .to_string(),
            });
        }

        if dependency_text.contains("streamlit") {
            if let Some((entry, _)) = generated_python_entry_content(
                files,
                &relative_dir,
                &["app.py", "main.py", "streamlit_app.py"],
            ) {
                return Some(GeneratedBundleLifecycleInference {
                    install_command,
                    entry_command: format!(
                        "streamlit run {} --server.address 0.0.0.0 --server.port {{PORT}}",
                        shell_quote_arg(&build_relative_file_arg(&relative_dir, &entry))
                    ),
                    runtime_reason: "generated bundle contains a Streamlit application".to_string(),
                });
            }
        }

        for entry in ["main.py", "app.py", "server.py", "api.py"] {
            if let Some(text) = bundle_file_text(files, &relative_dir, entry) {
                if text.contains("FastAPI(") || text.contains("from fastapi import") {
                    let module = entry.trim_end_matches(".py");
                    let app_dir_arg = if relative_dir.trim().is_empty() {
                        "."
                    } else {
                        relative_dir.as_str()
                    };
                    return Some(GeneratedBundleLifecycleInference {
                        install_command,
                        entry_command: format!(
                            "uvicorn --app-dir {} {}:app --host 0.0.0.0 --port {{PORT}}",
                            shell_quote_arg(app_dir_arg),
                            module
                        ),
                        runtime_reason: "generated bundle contains a FastAPI application"
                            .to_string(),
                    });
                }
            }
        }

        for entry in ["app.py", "main.py", "server.py", "wsgi.py"] {
            if let Some(text) = bundle_file_text(files, &relative_dir, entry) {
                if text.contains("Flask(") || text.contains("from flask import") {
                    return Some(GeneratedBundleLifecycleInference {
                        install_command,
                        entry_command: format!(
                            "flask --app {} run --host 0.0.0.0 --port {{PORT}}",
                            shell_quote_arg(&build_relative_file_arg(&relative_dir, entry))
                        ),
                        runtime_reason: "generated bundle contains a Flask application".to_string(),
                    });
                }
            }
        }

        if dependency_text.contains("gradio") {
            if let Some((entry, _)) =
                generated_python_entry_content(files, &relative_dir, &["app.py", "main.py"])
            {
                return Some(GeneratedBundleLifecycleInference {
                    install_command,
                    entry_command: format!(
                        "python {}",
                        shell_quote_arg(&build_relative_file_arg(&relative_dir, &entry))
                    ),
                    runtime_reason: "generated bundle contains a Gradio application".to_string(),
                });
            }
        }

        if let Some((entry, _)) = generated_python_entry_content(
            files,
            &relative_dir,
            &["server.py", "app.py", "main.py", "run.py"],
        ) {
            return Some(GeneratedBundleLifecycleInference {
                install_command,
                entry_command: format!(
                    "python {}",
                    shell_quote_arg(&build_relative_file_arg(&relative_dir, &entry))
                ),
                runtime_reason: "generated bundle contains a Python server entry point".to_string(),
            });
        }
    }
    None
}

fn infer_generated_rust_bundle_lifecycle(
    files: &HashMap<String, String>,
) -> Option<GeneratedBundleLifecycleInference> {
    let relative_dir = bundle_candidate_dirs_with_file(files, "Cargo.toml")
        .into_iter()
        .next()?;
    let entry_command = if relative_dir.trim().is_empty() {
        "cargo run".to_string()
    } else {
        format!(
            "cargo run --manifest-path {}",
            shell_quote_arg(&build_relative_file_arg(&relative_dir, "Cargo.toml"))
        )
    };
    Some(GeneratedBundleLifecycleInference {
        install_command: None,
        entry_command,
        runtime_reason: "generated bundle contains a Cargo manifest".to_string(),
    })
}

fn infer_generated_bundle_lifecycle(
    files: &serde_json::Map<String, serde_json::Value>,
) -> Option<GeneratedBundleLifecycleInference> {
    let text_files = text_files_from_effective_bundle(files);
    infer_generated_node_bundle_lifecycle(&text_files)
        .or_else(|| infer_generated_python_bundle_lifecycle(&text_files))
        .or_else(|| infer_generated_rust_bundle_lifecycle(&text_files))
}

fn set_generated_app_lifecycle_meta(
    meta: &mut serde_json::Value,
    inferred: &GeneratedBundleLifecycleInference,
) {
    let install_missing = app_meta_lifecycle_command(meta, "install_command").is_none();
    let Some(obj) = meta.as_object_mut() else {
        return;
    };

    obj.insert(
        "entry_command".to_string(),
        serde_json::Value::String(inferred.entry_command.clone()),
    );
    obj.insert(
        "start_command".to_string(),
        serde_json::Value::String(inferred.entry_command.clone()),
    );
    obj.insert(
        "runtime_required".to_string(),
        serde_json::Value::Bool(true),
    );
    obj.insert(
        "runtime_reason".to_string(),
        serde_json::Value::String(inferred.runtime_reason.clone()),
    );
    obj.insert(
        "updated_at".to_string(),
        serde_json::Value::String(chrono::Utc::now().to_rfc3339()),
    );

    let commands = obj
        .entry("commands".to_string())
        .or_insert_with(|| serde_json::json!({}));
    if !commands.is_object() {
        *commands = serde_json::json!({});
    }
    if let Some(commands) = commands.as_object_mut() {
        commands.insert(
            "start".to_string(),
            serde_json::Value::String(inferred.entry_command.clone()),
        );
        commands.insert(
            "entry".to_string(),
            serde_json::Value::String(inferred.entry_command.clone()),
        );
        if install_missing {
            if let Some(install) = inferred.install_command.as_ref() {
                commands.insert(
                    "install".to_string(),
                    serde_json::Value::String(install.clone()),
                );
            }
        }
    }

    if install_missing {
        if let Some(install) = inferred.install_command.as_ref() {
            obj.insert(
                "install_command".to_string(),
                serde_json::Value::String(install.clone()),
            );
        }
    }
}

fn collect_env_example_inputs(scope_root: &Path) -> Vec<AppRequiredInput> {
    let mut out = Vec::new();
    for candidate in [
        ".env.example",
        ".env.sample",
        ".env.template",
        ".env.local.example",
        ".env.development.example",
    ] {
        let path = scope_root.join(candidate);
        let Some(text) = read_text_file_limited(&path, MAX_REPO_TEXT_FILE_BYTES) else {
            continue;
        };
        for line in text.lines() {
            let trimmed = line.trim();
            if trimmed.is_empty() || trimmed.starts_with('#') {
                continue;
            }
            let Some((key, _)) = trimmed.split_once('=') else {
                continue;
            };
            let normalized = key.trim();
            if normalized.is_empty()
                || !normalized
                    .chars()
                    .all(|ch| ch.is_ascii_uppercase() || ch.is_ascii_digit() || ch == '_')
            {
                continue;
            }
            let sensitive = ["KEY", "TOKEN", "SECRET", "PASSWORD", "PASS", "PRIVATE"]
                .iter()
                .any(|needle| normalized.contains(needle));
            push_required_input(&mut out, normalized, sensitive);
        }
    }
    out
}

fn discover_repo_candidate_dirs(root: &Path) -> Vec<PathBuf> {
    let mut candidates = Vec::new();
    let walker = walkdir::WalkDir::new(root)
        .min_depth(0)
        .max_depth(3)
        .into_iter()
        .filter_entry(should_skip_repo_dir);
    for entry in walker.flatten() {
        if !entry.file_type().is_dir() {
            continue;
        }
        let dir = entry.path();
        let is_candidate = dir.join("package.json").exists()
            || dir.join("requirements.txt").exists()
            || dir.join("pyproject.toml").exists()
            || dir.join("manage.py").exists()
            || dir.join("Cargo.toml").exists()
            || dir.join("index.html").exists();
        if is_candidate {
            candidates.push(dir.to_path_buf());
        }
    }
    candidates.sort();
    candidates.dedup();
    candidates
}

fn plan_repo_services(
    repo_root: &Path,
    repo_title: &str,
    service_mode: RepoServiceMode,
) -> Result<Vec<RepoServicePlan>> {
    let root_manifest = load_node_manifest(repo_root);
    let root_has_workspaces = root_manifest
        .as_ref()
        .map(|manifest| manifest.has_workspaces)
        .unwrap_or(false);
    let root_readme_hints = load_readme_hints(repo_root);
    let candidate_dirs = discover_repo_candidate_dirs(repo_root);
    let has_child_package = candidate_dirs
        .iter()
        .any(|candidate| candidate != repo_root && candidate.join("package.json").exists());
    let mut plans = Vec::new();

    for dir in candidate_dirs {
        let relative_dir =
            normalize_repo_relative_path(dir.strip_prefix(repo_root).unwrap_or(&dir));
        if root_has_workspaces && relative_dir.is_empty() && has_child_package {
            continue;
        }

        let local_readme = load_readme_hints(&dir)
            .map(|(_, hints)| hints)
            .unwrap_or_default();
        let _readme_hints = if local_readme.install_command.is_some()
            || local_readme.start_command.is_some()
            || local_readme.mentions_compose
        {
            local_readme
        } else {
            root_readme_hints
                .as_ref()
                .map(|(_, hints)| hints.clone())
                .unwrap_or_default()
        };

        let required_inputs = {
            let mut inputs = collect_env_example_inputs(repo_root);
            for input in collect_env_example_inputs(&dir) {
                push_required_input(&mut inputs, &input.key, input.sensitive);
            }
            inputs
        };

        if let Some(manifest) = load_node_manifest(&dir) {
            let Some((kind, install_command, entry_command)) =
                build_node_commands(&dir, &relative_dir, &manifest, root_has_workspaces)
            else {
                continue;
            };
            if !kind.matches_mode(service_mode) {
                continue;
            }
            plans.push(RepoServicePlan {
                title: build_repo_service_title(repo_title, &relative_dir, kind),
                relative_dir,
                kind,
                copy_scope: RepoCopyScope::RepositoryRoot,
                install_command: Some(install_command),
                entry_command: Some(entry_command),
                required_inputs,
                detection_reason: "package.json scripts".to_string(),
            });
            continue;
        }

        if let Some((kind, install_command, entry_command)) =
            build_python_commands(&dir, &relative_dir)
        {
            if !kind.matches_mode(service_mode) {
                continue;
            }
            plans.push(RepoServicePlan {
                title: build_repo_service_title(repo_title, &relative_dir, kind),
                relative_dir,
                kind,
                copy_scope: RepoCopyScope::RepositoryRoot,
                install_command,
                entry_command: Some(entry_command),
                required_inputs,
                detection_reason: "python app manifest".to_string(),
            });
            continue;
        }

        if let Some((kind, install_command, entry_command)) =
            build_rust_commands(&dir, &relative_dir)
        {
            if !kind.matches_mode(service_mode) {
                continue;
            }
            plans.push(RepoServicePlan {
                title: build_repo_service_title(repo_title, &relative_dir, kind),
                relative_dir,
                kind,
                copy_scope: RepoCopyScope::RepositoryRoot,
                install_command,
                entry_command: Some(entry_command),
                required_inputs,
                detection_reason: "cargo manifest".to_string(),
            });
            continue;
        }

        if dir.join("index.html").exists() {
            let kind = RepoServiceKind::Static;
            if !kind.matches_mode(service_mode) {
                continue;
            }
            plans.push(RepoServicePlan {
                title: build_repo_service_title(repo_title, &relative_dir, kind),
                relative_dir,
                kind,
                copy_scope: RepoCopyScope::ServiceRoot,
                install_command: None,
                entry_command: None,
                required_inputs,
                detection_reason: "static index.html".to_string(),
            });
        }
    }

    if plans.is_empty() {
        if let Some((_, hints)) = root_readme_hints {
            if hints.mentions_compose {
                anyhow::bail!(
                    "Repo README suggests docker compose, but managed compose lifecycles are not supported yet. Use a repo with a directly runnable app or split the services explicitly."
                );
            }
            if service_mode == RepoServiceMode::Auto {
                if let Some(start_command) = hints.start_command {
                    plans.push(RepoServicePlan {
                        title: repo_title.to_string(),
                        relative_dir: String::new(),
                        kind: RepoServiceKind::Fullstack,
                        copy_scope: RepoCopyScope::RepositoryRoot,
                        install_command: hints.install_command,
                        entry_command: Some(start_command),
                        required_inputs: collect_env_example_inputs(repo_root),
                        detection_reason: "README install/run instructions".to_string(),
                    });
                }
            }
        }
    }

    if plans.len() > MAX_REPO_COMMAND_COUNT {
        anyhow::bail!(
            "Repo analysis detected too many runnable services ({}). Narrow the repo with repo_subdir or service_mode.",
            plans.len()
        );
    }
    Ok(plans)
}

fn collect_repo_files(root: &Path) -> Result<serde_json::Map<String, serde_json::Value>> {
    let mut files = serde_json::Map::new();
    let mut total_bytes = 0usize;
    let mut total_files = 0usize;
    let walker = walkdir::WalkDir::new(root)
        .into_iter()
        .filter_entry(should_skip_repo_dir);
    for entry in walker.flatten() {
        if entry.file_type().is_dir() {
            continue;
        }
        let path = entry.path();
        let metadata = match entry.metadata() {
            Ok(value) => value,
            Err(_) => continue,
        };
        if metadata.len() == 0 || metadata.len() as usize > MAX_REPO_TEXT_FILE_BYTES {
            continue;
        }
        let relative = normalize_repo_relative_path(path.strip_prefix(root).unwrap_or(path));
        if relative.is_empty() {
            continue;
        }
        let Some(content) = read_text_file_limited(path, MAX_REPO_TEXT_FILE_BYTES) else {
            continue;
        };
        total_files += 1;
        total_bytes += content.len();
        if total_files > MAX_REPO_TEXT_FILES {
            anyhow::bail!(
                "Repo is too large to deploy safely (>{} text files). Narrow it with repo_subdir.",
                MAX_REPO_TEXT_FILES
            );
        }
        if total_bytes > MAX_REPO_TOTAL_TEXT_BYTES {
            anyhow::bail!(
                "Repo is too large to deploy safely (>{} bytes of text content). Narrow it with repo_subdir.",
                MAX_REPO_TOTAL_TEXT_BYTES
            );
        }
        files.insert(relative, serde_json::Value::String(content));
    }
    if files.is_empty() {
        anyhow::bail!("Repo did not contain any deployable text files after filtering");
    }
    Ok(files)
}

async fn emit_repo_clone_progress(
    stream_tx: &Option<Sender<StreamEvent>>,
    message: impl Into<String>,
) {
    if let Some(tx) = stream_tx {
        let _ = tx
            .send(StreamEvent::ToolProgress {
                name: "app_deploy".to_string(),
                content: message.into(),
                payload: None,
            })
            .await;
    }
}

async fn clone_repo(
    repo_url: &str,
    repo_ref: Option<&str>,
    target_dir: &Path,
    stream_tx: &Option<Sender<StreamEvent>>,
) -> Result<()> {
    let mut clone_args = vec!["git".to_string(), "clone".to_string()];
    if repo_ref.is_none() {
        clone_args.push("--depth".to_string());
        clone_args.push("1".to_string());
    }
    clone_args.push(repo_url.to_string());
    clone_args.push(target_dir.to_string_lossy().to_string());

    emit_repo_clone_progress(stream_tx, format!("Cloning repository {}", repo_url)).await;
    let output = run_local_command_with_progress(
        &join_shell_command(&clone_args),
        "git clone",
        std::env::current_dir()?.as_path(),
        &HashMap::new(),
        MAX_REPO_CLONE_TIMEOUT_SECS,
        stream_tx,
        "repo_clone",
    )
    .await?;
    if !output.status.success() {
        let stderr = String::from_utf8_lossy(&output.stderr);
        let stdout = String::from_utf8_lossy(&output.stdout);
        let detail = if !stderr.trim().is_empty() {
            stderr.trim().to_string()
        } else {
            stdout.trim().to_string()
        };
        anyhow::bail!("git clone failed: {}", detail);
    }

    if let Some(reference) = repo_ref.filter(|value| !value.trim().is_empty()) {
        emit_repo_clone_progress(stream_tx, format!("Checking out repo ref {}", reference)).await;
        let output = run_local_command_with_progress(
            &format!("git checkout {}", shell_quote_arg(reference)),
            "git checkout",
            target_dir,
            &HashMap::new(),
            120,
            stream_tx,
            "repo_checkout",
        )
        .await?;
        if !output.status.success() {
            let stderr = String::from_utf8_lossy(&output.stderr);
            let stdout = String::from_utf8_lossy(&output.stdout);
            let detail = if !stderr.trim().is_empty() {
                stderr.trim().to_string()
            } else {
                stdout.trim().to_string()
            };
            anyhow::bail!("git checkout failed: {}", detail);
        }
    }

    Ok(())
}

fn repo_service_mode_label(mode: RepoServiceMode) -> &'static str {
    match mode {
        RepoServiceMode::Auto => "auto",
        RepoServiceMode::Frontend => "frontend",
        RepoServiceMode::Backend => "backend",
        RepoServiceMode::Fullstack => "fullstack",
    }
}

fn repo_deploy_fingerprint(
    repo_url: &str,
    repo_ref: Option<&str>,
    repo_subdir: Option<&str>,
    repo_title: &str,
    service_mode: RepoServiceMode,
    runtime_preference: RuntimePreference,
    expose_public: bool,
    access_guard_enabled: bool,
    runtime_image: Option<&serde_json::Value>,
) -> String {
    let payload = serde_json::json!({
        "repo_url": repo_url,
        "repo_ref": repo_ref,
        "repo_subdir": repo_subdir,
        "title": repo_title,
        "service_mode": repo_service_mode_label(service_mode),
        "runtime_preference": runtime_preference.as_str(),
        "expose_public": expose_public,
        "access_guard": access_guard_enabled,
        "runtime_image": runtime_image,
    });
    blake3::hash(payload.to_string().as_bytes())
        .to_hex()
        .to_string()
}

fn build_repo_deploy_lock_metadata(
    bundle_id: &str,
    fingerprint: &str,
    repo_url: &str,
    repo_ref: Option<&str>,
    repo_subdir: Option<&str>,
    repo_title: &str,
    service_mode: RepoServiceMode,
    runtime_preference: RuntimePreference,
    expose_public: bool,
    access_guard_enabled: bool,
    runtime_image: Option<&serde_json::Value>,
) -> serde_json::Value {
    serde_json::json!({
        "bundle_id": bundle_id,
        "fingerprint": fingerprint,
        "repo_url": repo_url,
        "repo_ref": repo_ref,
        "repo_subdir": repo_subdir,
        "title": repo_title,
        "service_mode": repo_service_mode_label(service_mode),
        "runtime_preference": runtime_preference.as_str(),
        "expose_public": expose_public,
        "access_guard": access_guard_enabled,
        "runtime_image": runtime_image,
        "started_at": chrono::Utc::now().to_rfc3339(),
        "started_at_unix": chrono::Utc::now().timestamp(),
    })
}

async fn read_repo_deploy_lock_metadata(lock_path: &Path) -> Option<serde_json::Value> {
    let raw = tokio::fs::read_to_string(lock_path).await.ok()?;
    serde_json::from_str(&raw).ok()
}

async fn repo_deploy_lock_is_stale(lock_path: &Path) -> bool {
    if let Some(metadata) = read_repo_deploy_lock_metadata(lock_path).await {
        if let Some(started_at_unix) = metadata
            .get("started_at_unix")
            .and_then(|value| value.as_i64())
        {
            let age = chrono::Utc::now()
                .timestamp()
                .saturating_sub(started_at_unix);
            return age >= REPO_DEPLOY_INFLIGHT_STALE_SECS as i64;
        }
    }
    if let Ok(metadata) = tokio::fs::metadata(lock_path).await {
        if let Ok(modified) = metadata.modified() {
            if let Ok(age) = std::time::SystemTime::now().duration_since(modified) {
                return age.as_secs() >= REPO_DEPLOY_INFLIGHT_STALE_SECS;
            }
        }
    }
    false
}

fn format_existing_repo_deploy_lock_message(metadata: Option<&serde_json::Value>) -> String {
    let Some(metadata) = metadata else {
        return "A matching repo deployment is already in progress. Wait for it to finish instead of starting another clone.".to_string();
    };

    let bundle_id = metadata
        .get("bundle_id")
        .and_then(|value| value.as_str())
        .map(str::trim)
        .filter(|value| !value.is_empty());
    let started_at = metadata
        .get("started_at")
        .and_then(|value| value.as_str())
        .map(str::trim)
        .filter(|value| !value.is_empty());

    let mut message = "A matching repo deployment is already in progress".to_string();
    if let Some(bundle_id) = bundle_id {
        message.push_str(&format!(" (bundle {})", bundle_id));
    }
    if let Some(started_at) = started_at {
        message.push_str(&format!(", started at {}", started_at));
    }
    message.push_str(". Wait for it to finish instead of starting another clone.");
    message
}

#[derive(Debug)]
struct RepoDeployInFlightGuard {
    lock_path: PathBuf,
}

impl RepoDeployInFlightGuard {
    async fn acquire(
        data_dir: &Path,
        fingerprint: &str,
        metadata: &serde_json::Value,
    ) -> Result<Self> {
        let inflight_dir = data_dir.join("repo-deployments").join(".inflight");
        tokio::fs::create_dir_all(&inflight_dir).await?;
        let lock_path = inflight_dir.join(format!("{fingerprint}.json"));
        let payload = serde_json::to_vec_pretty(metadata)?;
        let mut reclaimed_stale_lock = false;

        loop {
            match tokio::fs::OpenOptions::new()
                .create_new(true)
                .write(true)
                .open(&lock_path)
                .await
            {
                Ok(mut file) => {
                    file.write_all(&payload).await?;
                    file.flush().await?;
                    return Ok(Self { lock_path });
                }
                Err(error) if error.kind() == std::io::ErrorKind::AlreadyExists => {
                    if !reclaimed_stale_lock && repo_deploy_lock_is_stale(&lock_path).await {
                        reclaimed_stale_lock = true;
                        tracing::warn!("Reclaiming stale repo deploy lock {}", lock_path.display());
                        match tokio::fs::remove_file(&lock_path).await {
                            Ok(_) => continue,
                            Err(remove_error)
                                if remove_error.kind() == std::io::ErrorKind::NotFound =>
                            {
                                continue;
                            }
                            Err(remove_error) => {
                                tracing::warn!(
                                    "Failed to remove stale repo deploy lock {}: {}",
                                    lock_path.display(),
                                    remove_error
                                );
                            }
                        }
                    }
                    let existing_metadata = read_repo_deploy_lock_metadata(&lock_path).await;
                    anyhow::bail!(
                        "{}",
                        format_existing_repo_deploy_lock_message(existing_metadata.as_ref())
                    );
                }
                Err(error) => {
                    anyhow::bail!(
                        "Failed to create repo deploy lock '{}': {}",
                        lock_path.display(),
                        error
                    );
                }
            }
        }
    }
}

impl Drop for RepoDeployInFlightGuard {
    fn drop(&mut self) {
        let _ = std::fs::remove_file(&self.lock_path);
    }
}

struct RepoDeployWorkspaceGuard {
    bundle_dir: PathBuf,
    preserve_bundle: bool,
}

impl RepoDeployWorkspaceGuard {
    async fn create(bundle_dir: PathBuf) -> Result<Self> {
        tokio::fs::create_dir_all(&bundle_dir).await?;
        Ok(Self {
            bundle_dir,
            preserve_bundle: false,
        })
    }

    fn preserve_bundle(&mut self) {
        self.preserve_bundle = true;
    }
}

impl Drop for RepoDeployWorkspaceGuard {
    fn drop(&mut self) {
        if self.preserve_bundle {
            return;
        }
        let _ = std::fs::remove_dir_all(&self.bundle_dir);
    }
}

fn should_deploy_repo_bundle(arguments: &serde_json::Value) -> bool {
    let has_repo_url = arguments
        .get("repo_url")
        .and_then(|value| value.as_str())
        .is_some_and(|value| !value.trim().is_empty());
    let has_repo_bundle_id = arguments
        .get("repo_bundle_id")
        .and_then(|value| value.as_str())
        .is_some_and(|value| !value.trim().is_empty());
    has_repo_url && !has_repo_bundle_id
}

async fn deploy_repo_bundle(
    config_dir: &Path,
    data_dir: &Path,
    arguments: &serde_json::Value,
    registry: &AppRegistry,
    llm_env: &HashMap<String, String>,
    stream_tx: Option<Sender<StreamEvent>>,
) -> Result<String> {
    let repo_url = arguments
        .get("repo_url")
        .and_then(|value| value.as_str())
        .map(str::trim)
        .filter(|value| !value.is_empty())
        .ok_or_else(|| anyhow::anyhow!("repo_url cannot be empty"))?;
    let parsed_url = is_allowed_repo_url(repo_url)?;
    let repo_ref = arguments
        .get("repo_ref")
        .and_then(|value| value.as_str())
        .map(str::trim)
        .filter(|value| !value.is_empty());
    let repo_subdir = arguments
        .get("repo_subdir")
        .and_then(|value| value.as_str())
        .map(|value| value.trim_matches('/').trim_matches('\\'))
        .filter(|value| !value.is_empty())
        .map(|value| value.to_string());
    let requested_title = arguments
        .get("title")
        .and_then(|value| value.as_str())
        .map(str::trim)
        .filter(|value| !value.is_empty());
    let repo_title = requested_title
        .map(|value| value.to_string())
        .unwrap_or_else(|| repo_title_from_url(parsed_url.as_str()));
    let service_mode = repo_service_mode_from_opt(
        arguments
            .get("service_mode")
            .and_then(|value| value.as_str()),
    );
    let runtime_preference = if arguments
        .get("runtime_preference")
        .and_then(|value| value.as_str())
        .is_some()
    {
        runtime_preference_from_opt(
            arguments
                .get("runtime_preference")
                .and_then(|value| value.as_str()),
        )
    } else {
        runtime_preference_from_opt(None)
    };
    let expose_public = arguments
        .get("expose_public")
        .and_then(|value| value.as_bool())
        .unwrap_or(false);
    let requested_access_guard_enabled = arguments
        .get("access_guard")
        .and_then(|value| value.as_bool())
        .unwrap_or(false);
    let access_secret = access_secret_from_arguments(arguments)?;
    let access_guard_enabled = app_access_guard_enabled_for_deploy(
        expose_public,
        requested_access_guard_enabled,
        access_secret.is_some(),
    );
    let runtime_image = arguments.get("runtime_image").cloned();

    let fingerprint = repo_deploy_fingerprint(
        repo_url,
        repo_ref,
        repo_subdir.as_deref(),
        &repo_title,
        service_mode,
        runtime_preference,
        expose_public,
        access_guard_enabled,
        runtime_image.as_ref(),
    );
    let bundle_id = uuid::Uuid::new_v4().to_string()[..8].to_string();
    let lock_metadata = build_repo_deploy_lock_metadata(
        &bundle_id,
        &fingerprint,
        repo_url,
        repo_ref,
        repo_subdir.as_deref(),
        &repo_title,
        service_mode,
        runtime_preference,
        expose_public,
        access_guard_enabled,
        runtime_image.as_ref(),
    );
    let _inflight_guard =
        match RepoDeployInFlightGuard::acquire(data_dir, &fingerprint, &lock_metadata).await {
            Ok(guard) => guard,
            Err(error) => {
                emit_phase_progress(
                    &stream_tx,
                    AppDeployProgressPhase::Planning,
                    error.to_string(),
                )
                .await;
                return Err(error);
            }
        };
    tracing::info!(
        "Starting repo bundle deploy: bundle={} repo={} ref={:?} subdir={:?} fingerprint={}",
        bundle_id,
        repo_url,
        repo_ref,
        repo_subdir,
        &fingerprint[..12]
    );
    let bundle_dir = data_dir.join("repo-deployments").join(&bundle_id);
    let source_dir = bundle_dir.join("source");
    let mut workspace_guard = RepoDeployWorkspaceGuard::create(bundle_dir.clone()).await?;
    clone_repo(repo_url, repo_ref, &source_dir, &stream_tx).await?;

    let repo_root = if let Some(subdir) = repo_subdir.as_ref() {
        let candidate = source_dir.join(subdir);
        if !candidate.exists() || !candidate.is_dir() {
            anyhow::bail!(
                "repo_subdir '{}' was not found inside the cloned repo",
                subdir
            );
        }
        candidate
    } else {
        source_dir.clone()
    };

    let (readme_file, readme_mentions_compose) = load_readme_hints(&repo_root)
        .map(|(file, hints)| (Some(file), hints.mentions_compose))
        .unwrap_or((None, false));

    emit_phase_progress(
        &stream_tx,
        AppDeployProgressPhase::Planning,
        "Reading repo README and local manifests",
    )
    .await;
    let service_plans = {
        let repo_root = repo_root.clone();
        let repo_title = repo_title.clone();
        tokio::task::spawn_blocking(move || {
            plan_repo_services(&repo_root, &repo_title, service_mode)
        })
        .await
        .context("repo service planning task failed")??
    };
    if service_plans.is_empty() {
        anyhow::bail!(
            "I cloned the repo, but I could not detect a runnable frontend/backend service from the README or local manifests."
        );
    }
    emit_phase_progress(
        &stream_tx,
        AppDeployProgressPhase::Planning,
        format!(
            "Detected {} repo service(s): {}",
            service_plans.len(),
            service_plans
                .iter()
                .map(|plan| format!(
                    "{} ({})",
                    if plan.relative_dir.is_empty() {
                        ".".to_string()
                    } else {
                        plan.relative_dir.clone()
                    },
                    plan.kind.as_str()
                ))
                .collect::<Vec<_>>()
                .join(", ")
        ),
    )
    .await;

    let mut deployed_services = Vec::new();
    let mut success_like_count = 0usize;
    let mut needs_inputs_count = 0usize;
    let mut failure_count = 0usize;

    for (idx, plan) in service_plans.iter().enumerate() {
        emit_phase_progress(
            &stream_tx,
            AppDeployProgressPhase::Deploying,
            format!(
                "Deploying repo service {}/{}: {}",
                idx + 1,
                service_plans.len(),
                plan.title
            ),
        )
        .await;
        let scope_root = match plan.copy_scope {
            RepoCopyScope::RepositoryRoot => repo_root.clone(),
            RepoCopyScope::ServiceRoot => {
                if plan.relative_dir.is_empty() {
                    repo_root.clone()
                } else {
                    repo_root.join(&plan.relative_dir)
                }
            }
        };
        let files = tokio::task::spawn_blocking(move || collect_repo_files(&scope_root))
            .await
            .context("repo file collection task failed")??;
        let mut service_args = serde_json::Map::new();
        service_args.insert("files".to_string(), serde_json::Value::Object(files));
        service_args.insert("title".to_string(), serde_json::json!(plan.title));
        service_args.insert(
            "runtime_preference".to_string(),
            serde_json::json!(runtime_preference.as_str()),
        );
        service_args.insert(
            "expose_public".to_string(),
            serde_json::json!(expose_public),
        );
        service_args.insert(
            "access_guard".to_string(),
            serde_json::json!(access_guard_enabled),
        );
        if let Some(access_secret) = access_secret.as_ref() {
            service_args.insert(
                "access_password".to_string(),
                serde_json::json!(access_secret),
            );
        }
        service_args.insert("repo_url".to_string(), serde_json::json!(repo_url));
        service_args.insert("repo_bundle_id".to_string(), serde_json::json!(bundle_id));
        service_args.insert(
            "repo_service_kind".to_string(),
            serde_json::json!(plan.kind.as_str()),
        );
        service_args.insert(
            "repo_service_dir".to_string(),
            serde_json::json!(plan.relative_dir),
        );
        if let Some(ref value) = repo_ref {
            service_args.insert("repo_ref".to_string(), serde_json::json!(value));
        }
        if let Some(ref value) = repo_subdir {
            service_args.insert("repo_subdir".to_string(), serde_json::json!(value));
        }
        if let Some(ref value) = runtime_image {
            service_args.insert("runtime_image".to_string(), value.clone());
        }
        if !plan.required_inputs.is_empty() {
            service_args.insert(
                "required_inputs".to_string(),
                serde_json::to_value(&plan.required_inputs)
                    .unwrap_or_else(|_| serde_json::json!([])),
            );
        }
        if let Some(command) = plan.install_command.as_ref() {
            service_args.insert("install_command".to_string(), serde_json::json!(command));
        }
        if let Some(command) = plan.entry_command.as_ref() {
            service_args.insert("entry_command".to_string(), serde_json::json!(command));
            service_args.insert("runtime_required".to_string(), serde_json::json!(true));
            service_args.insert(
                "runtime_reason".to_string(),
                serde_json::json!("Repo service plan detected a runnable server command."),
            );
        }

        match std::pin::Pin::from(Box::new(app_deploy(
            config_dir,
            data_dir,
            &serde_json::Value::Object(service_args),
            registry,
            llm_env,
            stream_tx.clone(),
        )))
        .await
        {
            Ok(result) => {
                let mut parsed = serde_json::from_str::<serde_json::Value>(&result)
                    .unwrap_or_else(|_| serde_json::json!({ "status": "deployed", "raw": result }));
                if parsed
                    .get("runtime_delegated")
                    .and_then(|value| value.as_bool())
                    .unwrap_or(false)
                {
                    let Some(app_id) = parsed
                        .get("app_id")
                        .and_then(|value| value.as_str())
                        .map(str::trim)
                        .filter(|value| !value.is_empty())
                    else {
                        failure_count += 1;
                        deployed_services.push(serde_json::json!({
                            "title": plan.title,
                            "relative_dir": plan.relative_dir,
                            "kind": plan.kind.as_str(),
                            "status": "failed",
                            "detection_reason": plan.detection_reason,
                            "error": format!(
                                "Delegated repo service '{}' did not return an app_id",
                                plan.title
                            ),
                            "result": parsed,
                        }));
                        continue;
                    };
                    let delegated_title = parsed
                        .get("title")
                        .and_then(|value| value.as_str())
                        .unwrap_or(plan.title.as_str());
                    let delegated_access_guard_enabled = parsed
                        .get("access_guard_enabled")
                        .and_then(|value| value.as_bool())
                        .unwrap_or(access_guard_enabled);
                    let delegated_access_key = parsed
                        .get("access_password")
                        .and_then(|value| value.as_str())
                        .or_else(|| parsed.get("access_key").and_then(|value| value.as_str()))
                        .unwrap_or_default();
                    let delegated_expose_public = parsed
                        .get("expose_public")
                        .and_then(|value| value.as_bool())
                        .unwrap_or(expose_public);
                    match restart_delegated_runtime(
                        app_id,
                        delegated_title,
                        delegated_access_guard_enabled,
                        delegated_access_key,
                        delegated_expose_public,
                    )
                    .await
                    .with_context(|| {
                        format!("Executor startup failed for repo service '{}'", plan.title)
                    }) {
                        Ok(restarted) => {
                            parsed = restarted;
                        }
                        Err(error) => {
                            failure_count += 1;
                            deployed_services.push(serde_json::json!({
                                "title": plan.title,
                                "relative_dir": plan.relative_dir,
                                "kind": plan.kind.as_str(),
                                "status": "failed",
                                "detection_reason": plan.detection_reason,
                                "error": error.to_string(),
                                "result": parsed,
                            }));
                            continue;
                        }
                    }
                }
                let status = parsed
                    .get("status")
                    .and_then(|value| value.as_str())
                    .unwrap_or("deployed");
                if matches!(status, "deployed" | "needs_secrets" | "restarted") {
                    success_like_count += 1;
                }
                if status == "needs_secrets" {
                    needs_inputs_count += 1;
                }
                deployed_services.push(serde_json::json!({
                    "title": plan.title,
                    "relative_dir": plan.relative_dir,
                    "kind": plan.kind.as_str(),
                    "status": status,
                    "detection_reason": plan.detection_reason,
                    "result": parsed,
                }));
            }
            Err(error) => {
                failure_count += 1;
                deployed_services.push(serde_json::json!({
                    "title": plan.title,
                    "relative_dir": plan.relative_dir,
                    "kind": plan.kind.as_str(),
                    "status": "failed",
                    "detection_reason": plan.detection_reason,
                    "error": error.to_string(),
                }));
            }
        }
    }

    let summary_status = if failure_count == 0 && needs_inputs_count == 0 {
        "deployed"
    } else if failure_count == 0 {
        "needs_inputs"
    } else if success_like_count > 0 {
        "deployed_partially"
    } else {
        anyhow::bail!(
            "Repo deploy failed for all detected services: {}",
            deployed_services
                .iter()
                .filter_map(|service| {
                    let title = service.get("title").and_then(|value| value.as_str())?;
                    let error = service.get("error").and_then(|value| value.as_str())?;
                    Some(format!("{}: {}", title, error))
                })
                .collect::<Vec<_>>()
                .join(" | ")
        );
    };

    let manifest = serde_json::json!({
        "bundle_id": bundle_id,
        "repo_url": repo_url,
        "repo_ref": repo_ref,
        "repo_subdir": repo_subdir,
        "title": repo_title,
        "service_mode": repo_service_mode_label(service_mode),
        "status": summary_status,
        "readme_file": readme_file,
        "readme_mentions_compose": readme_mentions_compose,
        "created_at": chrono::Utc::now().to_rfc3339(),
        "apps_page_hint": APP_DEPLOY_CONTROL_HINT,
        "services": deployed_services,
    });
    tokio::fs::write(
        bundle_dir.join("bundle.json"),
        serde_json::to_string_pretty(&manifest)?,
    )
    .await?;
    workspace_guard.preserve_bundle();
    let _ = tokio::fs::remove_dir_all(&source_dir).await;
    tracing::info!(
        "Finished repo bundle deploy: bundle={} status={} services={}",
        bundle_id,
        summary_status,
        manifest
            .get("services")
            .and_then(|value| value.as_array())
            .map(|value| value.len())
            .unwrap_or(0)
    );

    Ok(serde_json::json!({
        "status": summary_status,
        "deployment_kind": "repo_bundle",
        "bundle_id": bundle_id,
        "title": repo_title,
        "repo_url": repo_url,
        "repo_ref": repo_ref,
        "repo_subdir": repo_subdir,
        "readme_file": readme_file,
        "runtime_preference": runtime_preference.as_str(),
        "service_count": deployed_services.len(),
        "deployed_count": success_like_count,
        "failed_count": failure_count,
        "apps_page_hint": APP_DEPLOY_CONTROL_HINT,
        "services": deployed_services,
    })
    .to_string())
}
pub fn app_container_name(app_id: &str) -> String {
    format!("{}{}", APP_CONTAINER_PREFIX, app_id)
}

#[derive(Debug, Clone, serde::Serialize, serde::Deserialize)]
pub struct AppRequiredInput {
    pub key: String,
    #[serde(default = "default_required_input_sensitive")]
    pub sensitive: bool,
}

fn default_required_input_sensitive() -> bool {
    true
}

fn push_required_input(out: &mut Vec<AppRequiredInput>, key: &str, sensitive: bool) {
    let k = key.trim();
    if k.is_empty() {
        return;
    }
    if let Some(existing) = out.iter_mut().find(|r| r.key == k) {
        // If any declaration marks it sensitive, keep it sensitive.
        existing.sensitive = existing.sensitive || sensitive;
        return;
    }
    out.push(AppRequiredInput {
        key: k.to_string(),
        sensitive,
    });
}

fn collect_required_string_list(
    out: &mut Vec<AppRequiredInput>,
    arr: Option<&Vec<serde_json::Value>>,
    sensitive: bool,
) {
    let Some(arr) = arr else {
        return;
    };
    for item in arr {
        if let Some(key) = item.as_str() {
            push_required_input(out, key, sensitive);
        }
    }
}

pub fn parse_required_inputs(arguments: &serde_json::Value) -> Vec<AppRequiredInput> {
    let mut out = Vec::new();
    // New generic model.
    if let Some(arr) = arguments.get("required_inputs").and_then(|v| v.as_array()) {
        for item in arr {
            match item {
                serde_json::Value::String(key) => push_required_input(&mut out, key, true),
                serde_json::Value::Object(obj) => {
                    let key = obj
                        .get("key")
                        .and_then(|v| v.as_str())
                        .or_else(|| obj.get("name").and_then(|v| v.as_str()))
                        .or_else(|| obj.get("env").and_then(|v| v.as_str()))
                        .unwrap_or("");
                    let sensitive = obj
                        .get("sensitive")
                        .and_then(|v| v.as_bool())
                        .unwrap_or(true);
                    push_required_input(&mut out, key, sensitive);
                }
                _ => {}
            }
        }
    }

    // Compatibility aliases.
    collect_required_string_list(
        &mut out,
        arguments.get("required_secrets").and_then(|v| v.as_array()),
        true,
    );
    collect_required_string_list(
        &mut out,
        arguments.get("required_env").and_then(|v| v.as_array()),
        true,
    );
    collect_required_string_list(
        &mut out,
        arguments.get("required_config").and_then(|v| v.as_array()),
        false,
    );
    out
}

pub fn parse_config_values(arguments: &serde_json::Value) -> HashMap<String, String> {
    let mut out = HashMap::new();
    let Some(obj) = arguments.get("config").and_then(|v| v.as_object()) else {
        return out;
    };
    for (k, v) in obj {
        let value = match v {
            serde_json::Value::String(s) => s.clone(),
            serde_json::Value::Bool(b) => b.to_string(),
            serde_json::Value::Number(n) => n.to_string(),
            _ => continue,
        };
        if !value.trim().is_empty() {
            out.insert(k.clone(), value);
        }
    }
    out
}

pub fn parse_runtime_actions(arguments: &serde_json::Value) -> Vec<String> {
    let mut out = Vec::new();
    let mut seen = std::collections::BTreeSet::new();
    let Some(items) = arguments.get("runtime_actions").and_then(|v| v.as_array()) else {
        return out;
    };
    for item in items {
        let raw = match item {
            serde_json::Value::String(value) => Some(value.as_str()),
            serde_json::Value::Object(obj) => obj.get("action").and_then(|v| v.as_str()),
            _ => None,
        };
        let Some(raw) = raw else {
            continue;
        };
        let action = raw.trim();
        if action.is_empty()
            || action.len() > 160
            || action.contains('/')
            || action.contains('\\')
            || action.chars().any(char::is_control)
        {
            continue;
        }
        let key = action.to_ascii_lowercase();
        if seen.insert(key) {
            out.push(action.to_string());
        }
    }
    out
}

fn resolve_secret_value(
    custom: &std::collections::HashMap<String, String>,
    _llm_env: &HashMap<String, String>,
    env: &str,
) -> Option<String> {
    if let Some(v) = custom
        .get(&format!("env:{}", env))
        .or_else(|| custom.get(&format!("secret:{}", env)))
        .or_else(|| custom.get(env))
    {
        if !v.trim().is_empty() {
            return Some(v.clone());
        }
    }

    for key in crate::core::secrets::storage_keys_for_user_key(env) {
        if let Some(v) = custom.get(&key) {
            if !v.trim().is_empty() {
                return Some(v.clone());
            }
        }
    }

    None
}

pub async fn resolve_required_env_values(
    config_dir: &Path,
    data_dir: &Path,
    required_inputs: &[AppRequiredInput],
    llm_env: &HashMap<String, String>,
    config_values: &HashMap<String, String>,
) -> Result<(HashMap<String, String>, Vec<String>, Vec<String>)> {
    let mgr =
        crate::core::config::SecureConfigManager::new_with_data_dir(config_dir, Some(data_dir))?;
    let secrets = mgr.load_secrets()?;
    let mut resolved = HashMap::new();
    let mut missing_sensitive = Vec::new();
    let mut missing_config = Vec::new();

    for required in required_inputs {
        let key = required.key.trim();
        if key.is_empty() {
            continue;
        }
        if required.sensitive {
            if let Some(v) = resolve_secret_value(&secrets.custom, llm_env, key) {
                resolved.insert(key.to_string(), v);
            } else if !missing_sensitive.iter().any(|m| m == key) {
                missing_sensitive.push(key.to_string());
            }
            continue;
        }

        if let Some(v) = config_values.get(key).cloned() {
            resolved.insert(key.to_string(), v);
            continue;
        }

        // Fallback: allow resolving non-sensitive values from encrypted store if user chose to save there.
        if let Some(v) = resolve_secret_value(&secrets.custom, llm_env, key) {
            resolved.insert(key.to_string(), v);
        } else if !missing_config.iter().any(|m| m == key) {
            missing_config.push(key.to_string());
        }
    }
    Ok((resolved, missing_sensitive, missing_config))
}

fn normalize_mount_path(path: &Path) -> String {
    path.to_string_lossy().replace('\\', "/")
}

fn command_looks_python_related(command: &str) -> bool {
    let lower = command.to_ascii_lowercase();
    [
        "python",
        "pip",
        "uvicorn",
        "gunicorn",
        "streamlit",
        "flask",
        "django",
        "manage.py",
        "fastapi",
    ]
    .iter()
    .any(|needle| lower.contains(needle))
}

fn build_dynamic_container_run_args(
    app_id: &str,
    app_dir: &Path,
    port: u16,
    image: &str,
    container_name: String,
    env_file_path: Option<&Path>,
    network_container_ref: Option<&str>,
    launch_script: String,
) -> Vec<String> {
    let mount = normalize_mount_path(app_dir);
    let mut args = vec![
        "run".to_string(),
        "-d".to_string(),
        "--memory".to_string(),
        "512m".to_string(),
        "--memory-swap".to_string(),
        "512m".to_string(),
        "--cpus".to_string(),
        "0.5".to_string(),
        "--pids-limit".to_string(),
        "128".to_string(),
        "--security-opt".to_string(),
        "no-new-privileges=true".to_string(),
        "--cap-drop".to_string(),
        "ALL".to_string(),
        "--user".to_string(),
        "65532:65532".to_string(),
        "--tmpfs".to_string(),
        "/tmp:size=64m,noexec,nosuid,nodev".to_string(),
        "--name".to_string(),
        container_name,
        "-v".to_string(),
        format!("{}:/workspace", mount),
        "-w".to_string(),
        "/workspace".to_string(),
        "--label".to_string(),
        "agentark.managed=true".to_string(),
        "--label".to_string(),
        format!("agentark.app_id={}", app_id),
        "--entrypoint".to_string(),
        "/bin/sh".to_string(),
        "-e".to_string(),
        format!("PORT={}", port),
        "-e".to_string(),
        "HOST=0.0.0.0".to_string(),
    ];
    if let Some(container_ref) = network_container_ref
        .map(str::trim)
        .filter(|value| !value.is_empty())
    {
        args.push("--network".to_string());
        args.push(format!("container:{}", container_ref));
    } else {
        args.push("-p".to_string());
        args.push(format!("127.0.0.1:{0}:{0}", port));
    }
    if let Some(path) = env_file_path {
        args.push("--env-file".to_string());
        args.push(path.to_string_lossy().to_string());
    }
    args.push(image.to_string());
    args.push("-lc".to_string());
    args.push(launch_script);
    args
}

fn build_dynamic_container_install_args(
    app_id: &str,
    app_dir: &Path,
    port: u16,
    image: &str,
    container_name: String,
    env_file_path: Option<&Path>,
    install_script: String,
) -> Vec<String> {
    let mount = normalize_mount_path(app_dir);
    let mut args = vec![
        "run".to_string(),
        "--rm".to_string(),
        "--memory".to_string(),
        "512m".to_string(),
        "--memory-swap".to_string(),
        "512m".to_string(),
        "--cpus".to_string(),
        "0.5".to_string(),
        "--pids-limit".to_string(),
        "128".to_string(),
        "--security-opt".to_string(),
        "no-new-privileges=true".to_string(),
        "--cap-drop".to_string(),
        "ALL".to_string(),
        "--user".to_string(),
        "65532:65532".to_string(),
        "--tmpfs".to_string(),
        "/tmp:size=64m,noexec,nosuid,nodev".to_string(),
        "--name".to_string(),
        container_name,
        "--entrypoint".to_string(),
        "/bin/sh".to_string(),
        "-v".to_string(),
        format!("{}:/workspace", mount),
        "-w".to_string(),
        "/workspace".to_string(),
        "--label".to_string(),
        "agentark.managed=true".to_string(),
        "--label".to_string(),
        format!("agentark.app_id={}", app_id),
        "-e".to_string(),
        format!("PORT={}", port),
        "-e".to_string(),
        "HOST=0.0.0.0".to_string(),
    ];
    if let Some(path) = env_file_path {
        args.push("--env-file".to_string());
        args.push(path.to_string_lossy().to_string());
    }
    args.push(image.to_string());
    args.push("-lc".to_string());
    args.push(install_script);
    args
}

fn local_runtime_stdout_log_path(app_dir: &Path) -> PathBuf {
    app_dir.join(LOCAL_RUNTIME_STDOUT_LOG_FILE)
}

fn local_runtime_stderr_log_path(app_dir: &Path) -> PathBuf {
    app_dir.join(LOCAL_RUNTIME_STDERR_LOG_FILE)
}

fn prepare_local_runtime_log_files(app_dir: &Path) -> Result<(PathBuf, PathBuf)> {
    let stdout_path = local_runtime_stdout_log_path(app_dir);
    let stderr_path = local_runtime_stderr_log_path(app_dir);

    std::fs::OpenOptions::new()
        .create(true)
        .write(true)
        .truncate(true)
        .open(&stdout_path)
        .with_context(|| format!("failed to prepare runtime stdout log at {:?}", stdout_path))?;
    std::fs::OpenOptions::new()
        .create(true)
        .write(true)
        .truncate(true)
        .open(&stderr_path)
        .with_context(|| format!("failed to prepare runtime stderr log at {:?}", stderr_path))?;

    Ok((stdout_path, stderr_path))
}

fn open_local_runtime_log_for_append(path: &Path, label: &str) -> Result<std::fs::File> {
    std::fs::OpenOptions::new()
        .create(true)
        .append(true)
        .open(path)
        .with_context(|| format!("failed to open runtime {} log at {:?}", label, path))
}

async fn read_file_tail(path: &Path, max_bytes: usize) -> String {
    let Ok(bytes) = tokio::fs::read(path).await else {
        return String::new();
    };
    if bytes.is_empty() {
        return String::new();
    }
    let start = bytes.len().saturating_sub(max_bytes);
    String::from_utf8_lossy(&bytes[start..]).trim().to_string()
}

pub async fn read_local_runtime_log_tail(app_dir: &Path, max_bytes: usize) -> String {
    let stderr_tail = read_file_tail(&local_runtime_stderr_log_path(app_dir), max_bytes).await;
    let stdout_tail = read_file_tail(&local_runtime_stdout_log_path(app_dir), max_bytes).await;
    let mut parts = Vec::new();
    if !stderr_tail.is_empty() {
        parts.push(format!("stderr:\n{}", stderr_tail));
    }
    if !stdout_tail.is_empty() {
        parts.push(format!("stdout:\n{}", stdout_tail));
    }
    parts.join("\n\n")
}

fn prepend_path_entry(prefix: &Path, existing_path: Option<&str>) -> Option<String> {
    let mut entries: Vec<PathBuf> = vec![prefix.to_path_buf()];
    if let Some(existing) = existing_path {
        entries.extend(std::env::split_paths(existing));
    } else if let Some(system) = std::env::var_os("PATH") {
        entries.extend(std::env::split_paths(&system));
    }
    std::env::join_paths(entries)
        .ok()
        .and_then(|v| v.into_string().ok())
}

async fn ensure_local_python_venv(app_dir: &Path) -> Result<(PathBuf, PathBuf)> {
    // Per-app venv lives under `<app_dir>/.agentark/venv` so it stays
    // isolated from any user-managed `.venv` and we own the path layout.
    let agentark_dir = app_dir.join(".agentark");
    if !agentark_dir.exists() {
        std::fs::create_dir_all(&agentark_dir).with_context(|| {
            format!(
                "failed to create per-app state directory {}",
                agentark_dir.display()
            )
        })?;
    }
    let venv_dir = agentark_dir.join("venv");
    let bin_dir = if cfg!(windows) {
        venv_dir.join("Scripts")
    } else {
        venv_dir.join("bin")
    };
    let python_candidates = if cfg!(windows) {
        vec![bin_dir.join("python.exe"), bin_dir.join("python")]
    } else {
        vec![bin_dir.join("python3"), bin_dir.join("python")]
    };

    if python_candidates.iter().any(|p| p.exists()) {
        return Ok((venv_dir, bin_dir));
    }

    // Pass the venv as an absolute path so this works regardless of cwd
    // resolution quirks on Windows / macOS sandboxes.
    let venv_arg = venv_dir.to_string_lossy().to_string();
    let creators: Vec<(&str, Vec<&str>)> = if cfg!(windows) {
        vec![
            ("python", vec!["-m", "venv", venv_arg.as_str()]),
            ("py", vec!["-3", "-m", "venv", venv_arg.as_str()]),
        ]
    } else {
        vec![
            ("python3", vec!["-m", "venv", venv_arg.as_str()]),
            ("python", vec!["-m", "venv", venv_arg.as_str()]),
        ]
    };

    let mut last_error = String::new();
    for (program, args) in creators {
        let mut cmd = tokio::process::Command::new(program);
        cmd.args(args)
            .stdout(Stdio::piped())
            .stderr(Stdio::piped())
            .kill_on_drop(true)
            .current_dir(app_dir);
        match tokio::time::timeout(std::time::Duration::from_secs(120), cmd.output()).await {
            Ok(Ok(output)) if output.status.success() => {
                if python_candidates.iter().any(|p| p.exists()) {
                    return Ok((venv_dir, bin_dir));
                }
                last_error =
                    "venv command succeeded but Python executable was not found in .agentark/venv"
                        .to_string();
            }
            Ok(Ok(output)) => {
                let stderr = String::from_utf8_lossy(&output.stderr);
                let stdout = String::from_utf8_lossy(&output.stdout);
                last_error = if !stderr.trim().is_empty() {
                    stderr.trim().to_string()
                } else if !stdout.trim().is_empty() {
                    stdout.trim().to_string()
                } else {
                    format!("{} -m venv exited with status {}", program, output.status)
                };
            }
            Ok(Err(e)) => {
                last_error = format!("failed to spawn {}: {}", program, e);
            }
            Err(_) => {
                last_error = format!("{} -m venv timed out", program);
            }
        }
    }

    if last_error.is_empty() {
        last_error = "unknown error creating .agentark/venv".to_string();
    }
    anyhow::bail!(
        "failed to prepare local Python virtual environment: {}",
        last_error
    );
}

/// Validate and normalise an app entry command.
/// Local runtime commands must stay as direct program+args invocations rather
/// than shell snippets so shell metacharacters cannot change execution shape.
fn validate_app_command(command: &str, label: &str) -> Result<String> {
    let trimmed = command.trim();
    if trimmed.is_empty() {
        anyhow::bail!("{} cannot be empty", label);
    }
    if trimmed.len() > MAX_APP_COMMAND_LEN {
        anyhow::bail!(
            "{} is too long ({} chars, max {})",
            label,
            trimmed.len(),
            MAX_APP_COMMAND_LEN
        );
    }
    if trimmed.contains('\n') || trimmed.contains('\r') {
        anyhow::bail!(
            "{} must be a single direct command and cannot contain newlines",
            label
        );
    }
    let collapsed = trimmed.to_string();
    let lowered = collapsed.to_ascii_lowercase();
    let direct_shell_prefixes = [
        "sh -c ",
        "bash -c ",
        "zsh -c ",
        "cmd /c ",
        "cmd.exe /c ",
        "powershell -command ",
        "powershell.exe -command ",
        "pwsh -command ",
        "pwsh.exe -command ",
    ];
    if direct_shell_prefixes
        .iter()
        .any(|prefix| lowered.starts_with(prefix))
    {
        anyhow::bail!(
            "{} must be a direct command and cannot invoke a shell interpreter",
            label
        );
    }

    let shell_tokens = ["&&", "||", ";", "|", "`", "$(", "<", ">"];
    if shell_tokens.iter().any(|tok| collapsed.contains(tok)) {
        anyhow::bail!(
            "{} contains shell operators and must be a direct command only",
            label
        );
    }
    let _ = split_command_args(&collapsed, label)?;
    Ok(collapsed)
}

fn is_valid_env_key(key: &str) -> bool {
    !key.is_empty()
        && key.len() <= 128
        && key
            .chars()
            .all(|c| c.is_ascii_uppercase() || c.is_ascii_digit() || c == '_')
}

async fn write_runtime_env_file(
    app_dir: &Path,
    extra_env: &HashMap<String, String>,
) -> Result<Option<PathBuf>> {
    if extra_env.is_empty() {
        return Ok(None);
    }

    let mut ordered: BTreeMap<String, String> = BTreeMap::new();
    for (k, v) in extra_env {
        if !is_valid_env_key(k) {
            anyhow::bail!("Invalid env key '{}': use [A-Z0-9_]", k);
        }
        if v.contains('\0') || v.contains('\n') || v.contains('\r') {
            anyhow::bail!(
                "Env value for '{}' contains unsupported control characters",
                k
            );
        }
        ordered.insert(k.clone(), v.clone());
    }

    let env_file_path = app_dir.join(".agentark_runtime_env");
    let mut content = String::new();
    for (k, v) in ordered {
        content.push_str(&k);
        content.push('=');
        content.push_str(&v);
        content.push('\n');
    }
    tokio::fs::write(&env_file_path, content)
        .await
        .with_context(|| format!("failed to write runtime env file at {:?}", env_file_path))?;

    Ok(Some(env_file_path))
}

async fn run_docker(
    args: &[String],
    cwd: Option<&Path>,
    timeout_secs: u64,
) -> Result<std::process::Output> {
    let mut cmd = tokio::process::Command::new("docker");
    cmd.args(args)
        .stdout(Stdio::piped())
        .stderr(Stdio::piped())
        .kill_on_drop(true);
    if let Some(dir) = cwd {
        cmd.current_dir(dir);
    }
    let fut = cmd.output();
    tokio::time::timeout(std::time::Duration::from_secs(timeout_secs), fut)
        .await
        .map_err(|_| anyhow::anyhow!("docker command timed out after {}s", timeout_secs))?
        .map_err(|e| anyhow::anyhow!("failed to execute docker: {}", e))
}

async fn discover_current_agent_image() -> Option<String> {
    let container_ref = current_container_ref_from_env()?;
    let args = vec![
        "inspect".to_string(),
        "-f".to_string(),
        "{{.Config.Image}}".to_string(),
        container_ref,
    ];
    let output = run_docker(&args, None, 20).await.ok()?;
    if !output.status.success() {
        return None;
    }
    let image = String::from_utf8_lossy(&output.stdout).trim().to_string();
    if image.is_empty() {
        None
    } else {
        Some(image)
    }
}

fn current_container_ref_from_env() -> Option<String> {
    std::env::var("HOSTNAME")
        .ok()
        .map(|value| value.trim().to_string())
        .filter(|value| !value.is_empty())
}

async fn discover_current_container_ref() -> Option<String> {
    let container_ref = current_container_ref_from_env()?;
    let args = vec![
        "inspect".to_string(),
        "-f".to_string(),
        "{{.Id}}".to_string(),
        container_ref.clone(),
    ];
    let output = run_docker(&args, None, 20).await.ok()?;
    if !output.status.success() {
        return None;
    }
    let id = String::from_utf8_lossy(&output.stdout).trim().to_string();
    if id.is_empty() {
        None
    } else {
        Some(id)
    }
}

async fn resolve_runtime_image(runtime_image: Option<&str>) -> String {
    if let Some(image) = runtime_image
        .map(str::trim)
        .filter(|value| !value.is_empty())
    {
        return image.to_string();
    }
    if let Some(configured) = configured_runtime_image() {
        return configured;
    }
    if let Some(current_image) = discover_current_agent_image().await {
        return current_image;
    }
    DEFAULT_FALLBACK_APP_RUNTIME_IMAGE.to_string()
}

/// Rewrite absolute `/app/` paths in entry commands to be relative to `app_dir`.
/// Container-authored commands use `/app/server.py` etc., but when running as a
/// local process the cwd is already `app_dir`, so `/app/server.py` should become
/// `./server.py` (or the actual file in `app_dir`).
fn localize_app_entry_command(command: &str, app_dir: &Path) -> String {
    let mut parts: Vec<String> = command.split_whitespace().map(|s| s.to_string()).collect();
    for part in &mut parts {
        if part.starts_with("/app/") {
            let relative = &part[5..]; // strip "/app/"
            let candidate = app_dir.join(relative);
            if candidate.exists() {
                *part = format!("./{}", relative);
            }
        }
    }
    parts.join(" ")
}

fn command_program_name(raw: &str) -> String {
    let file = raw
        .trim()
        .replace('\\', "/")
        .rsplit('/')
        .next()
        .unwrap_or(raw)
        .to_ascii_lowercase();
    file.strip_suffix(".cmd")
        .or_else(|| file.strip_suffix(".exe"))
        .unwrap_or(file.as_str())
        .to_string()
}

fn package_manager_script_name(args: &[String]) -> Option<&str> {
    let program = command_program_name(args.first()?.as_str());
    match program.as_str() {
        "npm" => (args.get(1).map(String::as_str) == Some("run"))
            .then(|| args.get(2).map(String::as_str))
            .flatten(),
        "pnpm" | "yarn" | "bun" => {
            if args.get(1).map(String::as_str) == Some("run") {
                args.get(2).map(String::as_str)
            } else {
                args.get(1).map(String::as_str)
            }
        }
        _ => None,
    }
}

fn is_package_manager_run(args: &[String]) -> bool {
    package_manager_script_name(args).is_some()
}

fn app_manifest_uses_vite(app_dir: &Path) -> bool {
    load_node_manifest(app_dir)
        .map(|manifest| manifest.dependencies.contains("vite"))
        .unwrap_or(false)
}

fn command_is_vite_runtime(args: &[String], app_dir: &Path) -> bool {
    let Some(first) = args.first() else {
        return false;
    };
    let program = command_program_name(first);
    if program == "vite" {
        return true;
    }
    if matches!(program.as_str(), "npx" | "pnpm" | "yarn" | "bun")
        && args
            .iter()
            .skip(1)
            .any(|arg| command_program_name(arg) == "vite")
    {
        return true;
    }
    let Some(script) = package_manager_script_name(args) else {
        return false;
    };
    app_manifest_uses_vite(app_dir) && matches!(script, "dev" | "start" | "preview")
}

fn command_vite_base_arg(args: &[String]) -> Option<&str> {
    let mut iter = args.iter();
    while let Some(arg) = iter.next() {
        if arg == "--base" {
            return iter.next().map(String::as_str);
        }
        if let Some(value) = arg.strip_prefix("--base=") {
            return Some(value);
        }
    }
    None
}

fn command_has_vite_base_arg(args: &[String]) -> bool {
    command_vite_base_arg(args).is_some()
}

fn app_mount_base(app_id: &str) -> String {
    format!("/apps/{}/", app_id.trim_matches('/'))
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum AppProxyPathMode {
    StripAppPrefix,
    PreserveAppPrefix,
}

impl AppProxyPathMode {
    pub fn as_str(self) -> &'static str {
        match self {
            Self::StripAppPrefix => "strip_app_prefix",
            Self::PreserveAppPrefix => "preserve_app_prefix",
        }
    }

    fn from_meta(raw: &str) -> Option<Self> {
        match raw.trim() {
            "strip_app_prefix" | "stripped" | "strip" => Some(Self::StripAppPrefix),
            "preserve_app_prefix" | "app_scoped" | "preserve" => Some(Self::PreserveAppPrefix),
            _ => None,
        }
    }
}

pub fn dynamic_app_upstream_path(app_id: &str, path: &str, mode: AppProxyPathMode) -> String {
    let normalized_path = path.trim_start_matches('/');
    match mode {
        AppProxyPathMode::StripAppPrefix => {
            if normalized_path.is_empty() {
                "/".to_string()
            } else {
                format!("/{}", normalized_path)
            }
        }
        AppProxyPathMode::PreserveAppPrefix => {
            if normalized_path.is_empty() {
                app_mount_base(app_id)
            } else {
                format!(
                    "{}{}",
                    app_mount_base(app_id),
                    normalized_path.trim_start_matches('/')
                )
            }
        }
    }
}

pub fn proxy_path_mode_for_entry_command(
    entry_command: Option<&str>,
    app_dir: &Path,
    app_id: &str,
) -> AppProxyPathMode {
    let Some(command) = entry_command
        .map(str::trim)
        .filter(|value| !value.is_empty())
    else {
        return AppProxyPathMode::StripAppPrefix;
    };
    let Ok(args) = split_command_args(command, "entry_command") else {
        return AppProxyPathMode::StripAppPrefix;
    };
    if !command_is_vite_runtime(&args, app_dir) {
        return AppProxyPathMode::StripAppPrefix;
    }
    match command_vite_base_arg(&args) {
        Some(base)
            if base.trim_end_matches('/') == app_mount_base(app_id).trim_end_matches('/') =>
        {
            AppProxyPathMode::PreserveAppPrefix
        }
        Some(_) => AppProxyPathMode::StripAppPrefix,
        None => AppProxyPathMode::PreserveAppPrefix,
    }
}

pub async fn proxy_path_mode_for_app_dir(app_dir: &Path, app_id: &str) -> AppProxyPathMode {
    let meta = load_app_meta_json(app_dir).await;
    if let Some(mode) = meta
        .get("proxy_path_mode")
        .and_then(|value| value.as_str())
        .and_then(AppProxyPathMode::from_meta)
    {
        return mode;
    }
    let entry_command = app_meta_lifecycle_command(&meta, "entry_command");
    proxy_path_mode_for_entry_command(entry_command.as_deref(), app_dir, app_id)
}

async fn persist_app_proxy_path_mode_meta(app_dir: &Path, mode: AppProxyPathMode) -> Result<()> {
    let mut meta = load_app_meta_json(app_dir).await;
    meta["proxy_path_mode"] = serde_json::Value::String(mode.as_str().to_string());
    write_app_meta_json(app_dir, &meta).await?;
    Ok(())
}

fn apply_app_mount_base_to_vite_entry_command(
    command: &str,
    app_dir: &Path,
    app_id: &str,
) -> Result<String> {
    let mut args = split_command_args(command, "entry_command")?;
    if !command_is_vite_runtime(&args, app_dir) || command_has_vite_base_arg(&args) {
        return Ok(command.to_string());
    }
    if is_package_manager_run(&args) && !args.iter().any(|arg| arg == "--") {
        args.push("--".to_string());
    }
    args.push("--base".to_string());
    args.push(app_mount_base(app_id));
    Ok(join_shell_command(&args))
}

fn split_command_args(command: &str, label: &str) -> Result<Vec<String>> {
    let mut out: Vec<String> = Vec::new();
    let mut current = String::new();
    let mut quote: Option<char> = None;
    let mut escape = false;

    for ch in command.chars() {
        if escape {
            current.push(ch);
            escape = false;
            continue;
        }
        match quote {
            Some(q) => {
                if ch == '\\' && q == '"' {
                    escape = true;
                } else if ch == q {
                    quote = None;
                } else {
                    current.push(ch);
                }
            }
            None => {
                if ch == '\'' || ch == '"' {
                    quote = Some(ch);
                } else if ch == '\\' {
                    escape = true;
                } else if ch.is_whitespace() {
                    if !current.is_empty() {
                        out.push(std::mem::take(&mut current));
                    }
                } else {
                    current.push(ch);
                }
            }
        }
    }

    if escape {
        anyhow::bail!("{} has a trailing escape character", label);
    }
    if quote.is_some() {
        anyhow::bail!("{} has an unclosed quote", label);
    }
    if !current.is_empty() {
        out.push(current);
    }
    if out.is_empty() {
        anyhow::bail!("{} cannot be empty", label);
    }
    Ok(out)
}

fn shell_quote_arg(arg: &str) -> String {
    if arg.is_empty() {
        return "''".to_string();
    }
    let safe = arg.chars().all(|c| {
        c.is_ascii_alphanumeric()
            || matches!(
                c,
                '_' | '-' | '.' | '/' | ':' | '@' | '%' | '+' | '=' | ',' | '{' | '}'
            )
    });
    if safe {
        arg.to_string()
    } else {
        format!("'{}'", arg.replace('\'', "'\"'\"'"))
    }
}

pub fn app_meta_lifecycle_command(meta: &serde_json::Value, key: &str) -> Option<String> {
    let (top_level_keys, nested_keys): (&[&str], &[&str]) = match key {
        "entry_command" | "start_command" => {
            (&["entry_command", "start_command"], &["start", "entry"])
        }
        "install_command" => (&["install_command"], &["install", "setup"]),
        "stop_command" => (&["stop_command"], &["stop"]),
        _ => (&[], &[]),
    };

    for candidate in top_level_keys {
        if let Some(value) = meta
            .get(candidate)
            .and_then(|value| value.as_str())
            .map(str::trim)
            .filter(|value| !value.is_empty())
        {
            return Some(value.to_string());
        }
    }

    let commands = meta.get("commands").and_then(|value| value.as_object())?;
    for candidate in nested_keys {
        if let Some(value) = commands
            .get(*candidate)
            .and_then(|value| value.as_str())
            .map(str::trim)
            .filter(|value| !value.is_empty())
        {
            return Some(value.to_string());
        }
    }
    None
}

fn join_shell_command(args: &[String]) -> String {
    args.iter()
        .map(|arg| shell_quote_arg(arg))
        .collect::<Vec<_>>()
        .join(" ")
}

fn normalize_python_runtime_command_for_container(command: &str) -> String {
    let Ok(args) = split_command_args(command, "command") else {
        return command.to_string();
    };
    if args.len() >= 2 {
        let head = args[0].to_ascii_lowercase();
        if (head == "sh" || head == "bash") && args[1] == "-c" {
            return command.to_string();
        }
    }

    let candidates = command_arg_candidates(&args);
    if candidates.is_empty() {
        return command.to_string();
    }

    if let Some(py3) = candidates.iter().find(|candidate| {
        candidate
            .first()
            .is_some_and(|p| p.eq_ignore_ascii_case("python3"))
    }) {
        return join_shell_command(py3);
    }
    join_shell_command(candidates.first().unwrap_or(&args))
}

fn command_arg_candidates(args: &[String]) -> Vec<Vec<String>> {
    if args.is_empty() {
        return Vec::new();
    }
    let program = args[0].trim();
    if program.is_empty() {
        return vec![args.to_vec()];
    }
    let has_path_hint = program.contains('/') || program.contains('\\');
    if has_path_hint {
        return vec![args.to_vec()];
    }

    let rest: Vec<String> = args.iter().skip(1).cloned().collect();
    let mut candidates: Vec<Vec<String>> = vec![args.to_vec()];
    let lowered = program.to_ascii_lowercase();

    let push_program_variant = |list: &mut Vec<Vec<String>>, alt: &str| {
        let mut variant = Vec::with_capacity(1 + rest.len());
        variant.push(alt.to_string());
        variant.extend(rest.iter().cloned());
        list.push(variant);
    };
    let push_module_variant = |list: &mut Vec<Vec<String>>, py: &str, module: &str| {
        let mut variant = Vec::with_capacity(3 + rest.len());
        variant.push(py.to_string());
        variant.push("-m".to_string());
        variant.push(module.to_string());
        variant.extend(rest.iter().cloned());
        list.push(variant);
    };

    match lowered.as_str() {
        "python" => {
            push_program_variant(&mut candidates, "python3");
            if cfg!(windows) {
                push_program_variant(&mut candidates, "py");
            }
        }
        "python3" => {
            push_program_variant(&mut candidates, "python");
        }
        "pip" => {
            push_program_variant(&mut candidates, "pip3");
            push_module_variant(&mut candidates, "python", "pip");
            push_module_variant(&mut candidates, "python3", "pip");
        }
        "pip3" => {
            push_program_variant(&mut candidates, "pip");
            push_module_variant(&mut candidates, "python3", "pip");
            push_module_variant(&mut candidates, "python", "pip");
        }
        "node" => {
            push_program_variant(&mut candidates, "nodejs");
        }
        "nodejs" => {
            push_program_variant(&mut candidates, "node");
        }
        "uvicorn" | "gunicorn" | "streamlit" | "flask" => {
            push_module_variant(&mut candidates, "python", lowered.as_str());
            push_module_variant(&mut candidates, "python3", lowered.as_str());
        }
        _ => {}
    }

    let mut deduped: Vec<Vec<String>> = Vec::new();
    let mut seen: std::collections::HashSet<String> = std::collections::HashSet::new();
    for candidate in candidates {
        if candidate.is_empty() {
            continue;
        }
        let key = candidate.join("\u{1f}");
        if seen.insert(key) {
            deduped.push(candidate);
        }
    }
    deduped
}

async fn spawn_local_process_with_fallback(
    args: &[String],
    label: &str,
    cwd: &Path,
    envs: &HashMap<String, String>,
    stdout_log_path: &Path,
    stderr_log_path: &Path,
) -> Result<(tokio::process::Child, String)> {
    let mut attempted: Vec<String> = Vec::new();
    for candidate in command_arg_candidates(args) {
        if candidate.is_empty() {
            continue;
        }
        let program = candidate[0].clone();
        attempted.push(candidate.join(" "));
        let stdout_log = open_local_runtime_log_for_append(stdout_log_path, "stdout")?;
        let stderr_log = open_local_runtime_log_for_append(stderr_log_path, "stderr")?;
        let mut cmd = tokio::process::Command::new(&program);
        // Strip orchestrator secrets / namespaced env from the inherited
        // environment (LLM API keys, internal auth, integration creds), then
        // layer the curated runtime env on top. See
        // `is_orchestrator_secret_var` for the pattern the scrub uses -
        // intent-based, not enumerative, so legitimate dev tooling vars
        // (JAVA_HOME, NODE_PATH, RUSTUP_HOME, etc.) still flow through.
        scrub_inherited_env_for_local_app(&mut cmd);
        cmd.args(candidate.iter().skip(1))
            .stdout(Stdio::from(stdout_log))
            .stderr(Stdio::from(stderr_log))
            .kill_on_drop(true)
            .current_dir(cwd)
            .envs(envs);
        match cmd.spawn() {
            Ok(child) => return Ok((child, program)),
            Err(e) if e.kind() == std::io::ErrorKind::NotFound => continue,
            Err(e) => anyhow::bail!("failed to execute {} '{}': {}", label, program, e),
        }
    }
    anyhow::bail!(
        "failed to execute {}: no executable found (tried: {})",
        label,
        attempted.join(" | ")
    )
}

/// Decide whether a specific app must use Docker, using intrinsic app signals
/// rather than a user/operator env knob. This stays deliberately narrow:
/// missing Docker must not poison ordinary Node/Python/Rust apps that can run
/// as executor-local processes.
fn docker_required_for_app(app_dir: &Path, runtime_image: Option<&str>) -> bool {
    if runtime_image
        .map(str::trim)
        .filter(|value| !value.is_empty())
        .is_some()
    {
        return true;
    }
    app_has_compose_manifest(app_dir)
}

/// Build a one-line "why" for the Docker-required decision so the user-facing
/// error tells the operator exactly which signal triggered the requirement.
fn describe_docker_signals(app_dir: &Path, runtime_image: Option<&str>) -> String {
    let mut signals: Vec<String> = Vec::new();
    if let Some(image) = runtime_image {
        signals.push(format!("runtime_image={}", image));
    }
    if app_has_compose_manifest(app_dir) {
        signals.push("compose manifest detected".to_string());
    }
    if signals.is_empty() {
        "no specific signal".to_string()
    } else {
        signals.join(", ")
    }
}

fn app_has_compose_manifest(app_dir: &Path) -> bool {
    const MANIFESTS: &[&str] = &[
        "docker-compose.yml",
        "docker-compose.yaml",
        "compose.yml",
        "compose.yaml",
    ];
    MANIFESTS.iter().any(|name| app_dir.join(name).is_file())
}

/// Namespaces of environment variables owned by the orchestrator. Anything
/// starting with one of these prefixes is considered orchestrator-internal
/// and never inherited by user-deployed apps, regardless of name shape.
const LOCAL_RUNTIME_ENV_BLOCKED_PREFIXES: &[&str] = &[
    "AGENTARK_",
    "ANTHROPIC_",
    "OPENAI_",
    "GOOGLE_API",
    "GROQ_",
    "DEEPSEEK_",
    "MISTRAL_",
    "AZURE_OPENAI_",
    "INTERNAL_",
    "POSTGRES_",
    "DATABASE_",
    "DOCKER_HOST_",
    "EMBEDDINGS_",
    "TELEGRAM_",
    "WHATSAPP_",
    "SLACK_",
    "DISCORD_",
    "MATRIX_",
    "TEAMS_",
    "GMAIL_",
    "GOOGLE_CHAT_",
    "RUNNER_",
    "GITHUB_TOKEN",
];

/// Variable names that always pass through (well-known dev tooling roots and
/// system essentials), even if their text would otherwise look secret-shaped
/// to the secret-name heuristic below. Empty for now; left for future surgical
/// exemptions.
const LOCAL_RUNTIME_ENV_FORCE_ALLOW: &[&str] = &[];

fn env_name_tokens(name: &str) -> Vec<String> {
    let mut tokens = Vec::new();
    let mut current = String::new();
    let mut previous: Option<char> = None;

    for ch in name.chars() {
        if !ch.is_ascii_alphanumeric() {
            if !current.is_empty() {
                tokens.push(current.to_ascii_uppercase());
                current.clear();
            }
            previous = None;
            continue;
        }

        let camel_boundary = previous
            .map(|prev| prev.is_ascii_lowercase() && ch.is_ascii_uppercase())
            .unwrap_or(false);
        if camel_boundary && !current.is_empty() {
            tokens.push(current.to_ascii_uppercase());
            current.clear();
        }
        current.push(ch);
        previous = Some(ch);
    }

    if !current.is_empty() {
        tokens.push(current.to_ascii_uppercase());
    }
    tokens
}

fn env_name_has_token_pair(tokens: &[String], first: &str, second: &str) -> bool {
    tokens
        .windows(2)
        .any(|window| window[0] == first && window[1] == second)
}

fn env_name_looks_secret(name: &str) -> bool {
    let tokens = env_name_tokens(name);
    if tokens.is_empty() {
        return false;
    }
    let sensitive_single_tokens = [
        "APIKEY",
        "KEY",
        "SECRET",
        "TOKEN",
        "PASSWORD",
        "PASSWD",
        "CREDENTIAL",
        "CREDENTIALS",
        "PRIVATE",
        "BEARER",
        "SESSION",
        "AUTH",
        "OAUTH",
    ];
    if tokens
        .iter()
        .any(|token| sensitive_single_tokens.contains(&token.as_str()))
    {
        return true;
    }
    if env_name_has_token_pair(&tokens, "API", "KEY")
        || env_name_has_token_pair(&tokens, "ACCESS", "TOKEN")
        || env_name_has_token_pair(&tokens, "REFRESH", "TOKEN")
        || env_name_has_token_pair(&tokens, "DATABASE", "URL")
        || env_name_has_token_pair(&tokens, "POSTGRES", "URL")
        || env_name_has_token_pair(&tokens, "MONGO", "URI")
        || env_name_has_token_pair(&tokens, "MONGODB", "URI")
        || env_name_has_token_pair(&tokens, "MYSQL", "URL")
        || env_name_has_token_pair(&tokens, "REDIS", "URL")
    {
        return true;
    }
    false
}

/// Returns true if a given env var should be scrubbed before launching a
/// user app. The rule is intent-based, not phrasing-list-based:
/// 1. Anything in an orchestrator namespace is dropped.
/// 2. Anything whose tokenized name looks like a secret is dropped.
/// 3. Everything else (PATH, HOME, JAVA_HOME, NODE_PATH, RUSTUP_HOME,
///    GOPATH, locale, proxy, Windows essentials, custom dev vars) passes.
fn is_orchestrator_secret_var(name: &str) -> bool {
    if LOCAL_RUNTIME_ENV_FORCE_ALLOW
        .iter()
        .any(|allowed| allowed.eq_ignore_ascii_case(name))
    {
        return false;
    }
    let upper = name.to_ascii_uppercase();
    for prefix in LOCAL_RUNTIME_ENV_BLOCKED_PREFIXES {
        if upper.starts_with(prefix) {
            return true;
        }
    }
    env_name_looks_secret(name)
}

/// Apply the env-scrub policy to a `tokio::process::Command`. We don't
/// `env_clear()` (that would drop dev tooling roots like JAVA_HOME and
/// NODE_PATH that vary across hosts and aren't easy to enumerate). Instead
/// we walk the parent env, remove anything `is_orchestrator_secret_var`
/// flags, and let the caller layer on the curated `extra_env` afterwards.
fn scrub_inherited_env_for_local_app(cmd: &mut tokio::process::Command) {
    for (key, _) in std::env::vars_os() {
        let key_str = key.to_string_lossy();
        if is_orchestrator_secret_var(&key_str) {
            cmd.env_remove(&key);
        }
    }
}

fn docker_cli_available() -> bool {
    std::env::var_os("PATH")
        .map(|paths| {
            std::env::split_paths(&paths).any(|dir| {
                if cfg!(windows) {
                    let exts = std::env::var_os("PATHEXT")
                        .map(|raw| {
                            raw.to_string_lossy()
                                .split(';')
                                .map(|ext| ext.trim().trim_start_matches('.').to_ascii_lowercase())
                                .filter(|ext| !ext.is_empty())
                                .collect::<Vec<_>>()
                        })
                        .filter(|exts| !exts.is_empty())
                        .unwrap_or_else(|| {
                            vec!["exe".to_string(), "cmd".to_string(), "bat".to_string()]
                        });
                    exts.iter()
                        .any(|ext| dir.join(format!("docker.{ext}")).is_file())
                } else {
                    dir.join("docker").is_file()
                }
            })
        })
        .unwrap_or(false)
}

fn container_runtime_configured() -> bool {
    std::env::var("DOCKER_HOST")
        .map(|value| !value.trim().is_empty())
        .unwrap_or(false)
        || Path::new("/var/run/docker.sock").exists()
}

fn container_runtime_available() -> bool {
    docker_cli_available() && container_runtime_configured()
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum RuntimePreference {
    Local,
    Container,
}

impl RuntimePreference {
    pub fn as_str(self) -> &'static str {
        match self {
            RuntimePreference::Local => "local",
            RuntimePreference::Container => "container",
        }
    }
}

fn default_runtime_preference() -> RuntimePreference {
    match std::env::var("AGENTARK_APP_RUNTIME_DEFAULT")
        .unwrap_or_default()
        .trim()
        .to_ascii_lowercase()
        .as_str()
    {
        "container" | "docker" => RuntimePreference::Container,
        "local" | "native" | "process" => RuntimePreference::Local,
        _ => {
            if container_runtime_available() {
                RuntimePreference::Container
            } else {
                RuntimePreference::Local
            }
        }
    }
}

pub fn runtime_preference_from_opt(raw: Option<&str>) -> RuntimePreference {
    match raw.unwrap_or("").trim().to_ascii_lowercase().as_str() {
        "local" | "native" | "process" => RuntimePreference::Local,
        "container" | "docker" => RuntimePreference::Container,
        _ => default_runtime_preference(),
    }
}

fn access_secret_from_arguments(arguments: &serde_json::Value) -> Result<Option<String>> {
    let provided = arguments
        .get("access_password")
        .and_then(|value| value.as_str())
        .or_else(|| arguments.get("access_key").and_then(|value| value.as_str()));
    let Some(raw) = provided else {
        return Ok(None);
    };
    let trimmed = raw.trim();
    if trimmed.is_empty() {
        anyhow::bail!("Access password cannot be empty");
    }
    if trimmed.chars().count() > 256 {
        anyhow::bail!("Access password is too long (max 256 characters)");
    }
    Ok(Some(trimmed.to_string()))
}

fn with_node_bin_path(app_dir: &Path) -> Option<String> {
    let node_bin = app_dir.join("node_modules").join(".bin");
    if !node_bin.exists() {
        return None;
    }
    let mut entries: Vec<std::path::PathBuf> = vec![node_bin];
    if let Some(existing) = std::env::var_os("PATH") {
        entries.extend(std::env::split_paths(&existing));
    }
    std::env::join_paths(entries)
        .ok()
        .and_then(|os| os.into_string().ok())
}

fn node_modules_populated(app_dir: &Path) -> bool {
    let modules = app_dir.join("node_modules");
    modules.is_dir()
        && std::fs::read_dir(&modules)
            .map(|iter| iter.take(2).count() > 0)
            .unwrap_or(false)
}

/// True if the app declares Python dependencies. Used to decide whether to
/// bootstrap a per-app venv when the entry/install commands don't obviously
/// look like Python (e.g. a Makefile target that wraps `pytest`).
fn app_declares_python_deps(app_dir: &Path) -> bool {
    const MARKERS: &[&str] = &[
        "requirements.txt",
        "pyproject.toml",
        "Pipfile",
        "Pipfile.lock",
        "setup.py",
        "setup.cfg",
    ];
    MARKERS.iter().any(|name| app_dir.join(name).is_file())
}

/// Pick the right Node package manager based on the lockfile present.
/// Returns `None` when the app has no `package.json`, or when `node_modules`
/// already contains installed packages so we don't double-install.
fn detect_node_install_command(app_dir: &Path) -> Option<&'static str> {
    if !app_dir.join("package.json").is_file() {
        return None;
    }
    if node_modules_populated(app_dir) {
        return None;
    }
    if app_dir.join("pnpm-lock.yaml").is_file() {
        Some("pnpm install")
    } else if app_dir.join("yarn.lock").is_file() {
        Some("yarn install")
    } else {
        Some("npm install")
    }
}

fn install_command_is_node_dependency_install(command: &str) -> bool {
    let Ok(args) = split_command_args(command, "install_command") else {
        return false;
    };
    let Some(program) = args.first().map(|value| command_program_name(value)) else {
        return false;
    };
    match program.as_str() {
        "npm" => args
            .iter()
            .skip(1)
            .any(|arg| matches!(arg.as_str(), "install" | "i" | "ci")),
        "pnpm" => args
            .iter()
            .skip(1)
            .any(|arg| matches!(arg.as_str(), "install" | "i")),
        "yarn" => args
            .get(1)
            .map(|arg| matches!(arg.as_str(), "install" | "add"))
            .unwrap_or(true),
        "bun" => args
            .iter()
            .skip(1)
            .any(|arg| matches!(arg.as_str(), "install" | "i")),
        _ => false,
    }
}

fn should_skip_redundant_install_command(app_dir: &Path, command: &str) -> bool {
    app_dir.join("package.json").is_file()
        && node_modules_populated(app_dir)
        && install_command_is_node_dependency_install(command)
}

/// Walk PATH and return true if any candidate program resolves to an existing
/// executable. Honours the explicit `PATH` we hand the child process so the
/// check matches what the spawn will see.
fn program_resolvable_on_path(program: &str, path_value: Option<&str>) -> bool {
    if program.contains('/') || program.contains('\\') {
        return Path::new(program).is_file();
    }
    let exts: Vec<String> = if cfg!(windows) {
        std::env::var("PATHEXT")
            .ok()
            .map(|raw| {
                raw.split(';')
                    .filter(|s| !s.is_empty())
                    .map(|s| s.to_string())
                    .collect()
            })
            .unwrap_or_else(|| vec![".COM".into(), ".EXE".into(), ".BAT".into(), ".CMD".into()])
    } else {
        vec![String::new()]
    };
    let raw_path = match path_value {
        Some(p) if !p.is_empty() => p.to_string(),
        _ => std::env::var("PATH").unwrap_or_default(),
    };
    for entry in std::env::split_paths(&raw_path) {
        if entry.as_os_str().is_empty() {
            continue;
        }
        for ext in exts.iter() {
            let candidate = if ext.is_empty() {
                entry.join(program)
            } else {
                entry.join(format!("{}{}", program, ext))
            };
            if candidate.is_file() {
                return true;
            }
        }
    }
    false
}

/// Names of language runtimes the entry command needs on PATH. Returned as
/// `(canonical_name, accepted_aliases)` so a missing `python3` can also be
/// satisfied by `python` / `py`.
fn required_runtimes_for_command(args: &[String]) -> Vec<(&'static str, Vec<&'static str>)> {
    let Some(program) = args.first().map(|s| s.to_ascii_lowercase()) else {
        return Vec::new();
    };
    let py_aliases: Vec<&'static str> = if cfg!(windows) {
        vec!["python3", "python", "py"]
    } else {
        vec!["python3", "python"]
    };
    match program.as_str() {
        "python" | "python3" | "py" | "pip" | "pip3" | "uvicorn" | "gunicorn" | "streamlit"
        | "flask" | "fastapi" | "celery" => vec![("python3", py_aliases)],
        "node" | "nodejs" | "npm" | "npx" => vec![("node", vec!["node", "nodejs"])],
        "pnpm" => vec![("pnpm", vec!["pnpm"]), ("node", vec!["node", "nodejs"])],
        "yarn" => vec![("yarn", vec!["yarn"]), ("node", vec!["node", "nodejs"])],
        "bun" | "bunx" => vec![("bun", vec!["bun"])],
        "deno" => vec![("deno", vec!["deno"])],
        "cargo" | "rustc" => vec![("cargo", vec!["cargo"])],
        "go" => vec![("go", vec!["go"])],
        "ruby" | "bundle" | "bundler" | "rails" => vec![("ruby", vec!["ruby"])],
        "java" => vec![("java", vec!["java"])],
        _ => Vec::new(),
    }
}

fn runtime_install_hint(canonical: &str) -> &'static str {
    match canonical {
        "python3" => {
            "install Python 3 (e.g. `apt-get install -y python3 python3-venv` on Debian/Ubuntu, `brew install python` on macOS)"
        }
        "node" => "install Node.js 20+ (e.g. `apt-get install -y nodejs npm`, or via nvm/fnm)",
        "bun" => "install Bun (https://bun.sh) - `curl -fsSL https://bun.sh/install | bash`",
        "deno" => {
            "install Deno (https://deno.land) - `curl -fsSL https://deno.land/install.sh | sh`"
        }
        "cargo" => "install Rust + cargo (https://rustup.rs)",
        "go" => "install Go 1.21+ (https://go.dev/dl)",
        "ruby" => "install Ruby 3+ (e.g. `apt-get install -y ruby` or via rbenv)",
        "java" => "install a JDK 17+ (e.g. `apt-get install -y default-jdk`)",
        "pnpm" => "enable pnpm via corepack (`corepack enable pnpm`) or `npm i -g pnpm`",
        "yarn" => "enable yarn via corepack (`corepack enable`) or `npm i -g yarn`",
        _ => "install the missing runtime on this host",
    }
}

fn ensure_required_runtimes_available(
    app_id: &str,
    label: &str,
    args: &[String],
    runtime_path: Option<&str>,
) -> Result<()> {
    for (canonical, aliases) in required_runtimes_for_command(args) {
        if aliases
            .iter()
            .any(|alias| program_resolvable_on_path(alias, runtime_path))
        {
            continue;
        }
        anyhow::bail!(
            "app {} {} needs '{}' which isn't installed on this orchestrator host. {}, or rebuild the agentark image to include it.",
            app_id,
            label,
            canonical,
            runtime_install_hint(canonical)
        );
    }
    Ok(())
}

fn compact_progress_line(line: &str, max_chars: usize) -> String {
    let trimmed = line.trim().replace('\r', "");
    let char_count = trimmed.chars().count();
    if char_count <= max_chars {
        return trimmed;
    }
    let head = max_chars.saturating_sub(3);
    format!("{}...", trimmed.chars().take(head).collect::<String>())
}

async fn read_command_output_chunks<R>(
    reader: Option<R>,
    stream_tx: Option<Sender<StreamEvent>>,
    tool_name: &str,
    stage: &str,
    stream_name: &str,
) -> Vec<u8>
where
    R: tokio::io::AsyncRead + Unpin,
{
    let mut collected = Vec::new();
    let Some(mut reader) = reader else {
        return collected;
    };

    let mut buf = [0u8; 2048];
    let mut pending = String::new();

    loop {
        match reader.read(&mut buf).await {
            Ok(0) => break,
            Ok(read) => {
                collected.extend_from_slice(&buf[..read]);
                pending.push_str(&String::from_utf8_lossy(&buf[..read]));

                if pending.len() >= 512 || pending.contains('\n') || pending.contains('\r') {
                    let normalized = pending.replace('\r', "\n");
                    let chunk = normalized
                        .lines()
                        .map(|line| compact_progress_line(line, 220))
                        .filter(|line| !line.is_empty())
                        .collect::<Vec<_>>()
                        .join("\n");
                    if !chunk.is_empty() {
                        if let Some(tx) = stream_tx.as_ref() {
                            let _ = tx
                                .send(StreamEvent::ToolProgress {
                                    name: tool_name.to_string(),
                                    content: format!("{} {}: {}", stage, stream_name, chunk),
                                    payload: Some(serde_json::json!({
                                        "kind": "console_chunk",
                                        "stage": stage,
                                        "stream": stream_name,
                                        "text": chunk,
                                        "stream_key": format!("console:{}:{}:{}", tool_name, stage, stream_name),
                                    })),
                                })
                                .await;
                        }
                    }
                    pending.clear();
                }
            }
            Err(_) => break,
        }
    }

    let final_chunk = pending.replace('\r', "\n");
    let final_chunk = final_chunk
        .lines()
        .map(|line| compact_progress_line(line, 220))
        .filter(|line| !line.is_empty())
        .collect::<Vec<_>>()
        .join("\n");
    if !final_chunk.is_empty() {
        if let Some(tx) = stream_tx.as_ref() {
            let _ = tx
                .send(StreamEvent::ToolProgress {
                    name: tool_name.to_string(),
                    content: format!("{} {}: {}", stage, stream_name, final_chunk),
                    payload: Some(serde_json::json!({
                        "kind": "console_chunk",
                        "stage": stage,
                        "stream": stream_name,
                        "text": final_chunk,
                        "stream_key": format!("console:{}:{}:{}", tool_name, stage, stream_name),
                    })),
                })
                .await;
        }
    }

    collected
}

async fn run_local_command_with_progress(
    command: &str,
    label: &str,
    cwd: &Path,
    envs: &HashMap<String, String>,
    timeout_secs: u64,
    stream_tx: &Option<Sender<StreamEvent>>,
    stage: &str,
) -> Result<std::process::Output> {
    let args = split_command_args(command, label)?;
    let mut attempted: Vec<String> = Vec::new();
    for candidate in command_arg_candidates(&args) {
        if candidate.is_empty() {
            continue;
        }
        let program = candidate[0].clone();
        attempted.push(candidate.join(" "));
        let mut cmd = tokio::process::Command::new(&program);
        // Same secret-scrub policy as the spawn path. See
        // `is_orchestrator_secret_var` for the rule.
        scrub_inherited_env_for_local_app(&mut cmd);
        cmd.args(candidate.iter().skip(1))
            .stdout(Stdio::piped())
            .stderr(Stdio::piped())
            .kill_on_drop(true)
            .current_dir(cwd)
            .envs(envs);

        let mut child = match cmd.spawn() {
            Ok(child) => child,
            Err(e) if e.kind() == std::io::ErrorKind::NotFound => continue,
            Err(e) => anyhow::bail!("failed to execute {} '{}': {}", label, program, e),
        };

        let stdout = child.stdout.take();
        let stderr = child.stderr.take();
        let stdout_tx = stream_tx.clone();
        let stderr_tx = stream_tx.clone();
        let stage_stdout = stage.to_string();
        let stage_stderr = stage.to_string();

        let stdout_task = tokio::spawn(async move {
            read_command_output_chunks(stdout, stdout_tx, "app_deploy", &stage_stdout, "stdout")
                .await
        });

        let stderr_task = tokio::spawn(async move {
            read_command_output_chunks(stderr, stderr_tx, "app_deploy", &stage_stderr, "stderr")
                .await
        });

        let status =
            match tokio::time::timeout(std::time::Duration::from_secs(timeout_secs), child.wait())
                .await
            {
                Err(_) => {
                    let _ = child.kill().await;
                    let _ = child.wait().await;
                    let _ = stdout_task.await;
                    let _ = stderr_task.await;
                    anyhow::bail!("{} timed out", label);
                }
                Ok(Ok(status)) => status,
                Ok(Err(e)) => {
                    let _ = stdout_task.await;
                    let _ = stderr_task.await;
                    anyhow::bail!("failed waiting for {} '{}': {}", label, program, e);
                }
            };

        let stdout = stdout_task.await.unwrap_or_default();
        let stderr = stderr_task.await.unwrap_or_default();
        return Ok(std::process::Output {
            status,
            stdout,
            stderr,
        });
    }

    anyhow::bail!(
        "failed to execute {}: no executable found (tried: {})",
        label,
        attempted.join(" | ")
    );
}

pub async fn cleanup_existing_container(name: &str) {
    let args = vec!["rm".to_string(), "-f".to_string(), name.to_string()];
    let _ = run_docker(&args, None, 20).await;
}

async fn is_container_running(container_id: &str) -> bool {
    let args = vec![
        "inspect".to_string(),
        "-f".to_string(),
        "{{.State.Running}}".to_string(),
        container_id.to_string(),
    ];
    match run_docker(&args, None, 15).await {
        Ok(output) if output.status.success() => String::from_utf8_lossy(&output.stdout)
            .trim()
            .eq_ignore_ascii_case("true"),
        _ => false,
    }
}

async fn stop_container(container_id: &str) -> Result<()> {
    let stop_args = vec![
        "stop".to_string(),
        "-t".to_string(),
        "10".to_string(),
        container_id.to_string(),
    ];
    let output = run_docker(&stop_args, None, 30).await?;
    if output.status.success() {
        return Ok(());
    }
    let stderr = String::from_utf8_lossy(&output.stderr);
    if stderr.contains("No such container") {
        return Ok(());
    }
    anyhow::bail!(
        "failed to stop container {}: {}",
        container_id,
        stderr.trim()
    );
}

async fn stop_child_process(child: &mut tokio::process::Child, app_id: &str) -> Result<()> {
    let already_exited = matches!(child.try_wait(), Ok(Some(_)));
    if already_exited {
        return Ok(());
    }
    child
        .kill()
        .await
        .with_context(|| format!("failed to kill app process {}", app_id))?;
    tokio::time::timeout(std::time::Duration::from_secs(5), child.wait())
        .await
        .map_err(|_| anyhow::anyhow!("timeout waiting for process {} to exit", app_id))?
        .with_context(|| format!("failed waiting for app process {}", app_id))?;
    Ok(())
}

async fn read_app_lifecycle_command(app_dir: &Path, key: &str) -> Option<String> {
    let bytes = tokio::fs::read(app_dir.join(".app_meta.json")).await.ok()?;
    let meta = serde_json::from_slice::<serde_json::Value>(&bytes).ok()?;
    app_meta_lifecycle_command(&meta, key)
}

fn build_local_lifecycle_env(app_dir: &Path) -> HashMap<String, String> {
    let mut env = HashMap::new();
    if let Some(path) = with_node_bin_path(app_dir) {
        env.insert("PATH".to_string(), path);
    }
    // Check the per-app venv first (.agentark/venv), then fall back to the
    // legacy .venv layout so already-bootstrapped apps keep working.
    let venv_candidates: [PathBuf; 2] = [
        app_dir.join(".agentark").join("venv"),
        app_dir.join(".venv"),
    ];
    let mut resolved_venv: Option<(PathBuf, PathBuf)> = None;
    for venv_dir in venv_candidates.iter() {
        let venv_bin = if cfg!(windows) {
            venv_dir.join("Scripts")
        } else {
            venv_dir.join("bin")
        };
        if venv_bin.exists() {
            resolved_venv = Some((venv_dir.clone(), venv_bin));
            break;
        }
    }
    if let Some((venv_dir, venv_bin)) = resolved_venv {
        if let Some(path) = prepend_path_entry(&venv_bin, env.get("PATH").map(|v| v.as_str())) {
            env.insert("PATH".to_string(), path);
        }
        env.insert(
            "VIRTUAL_ENV".to_string(),
            venv_dir.to_string_lossy().to_string(),
        );
    }
    env
}

async fn run_container_lifecycle_command(
    container_id: &str,
    command: &str,
    label: &str,
) -> Result<()> {
    let command = validate_app_command(command, label)?;
    let args = split_command_args(&command, label)?;
    let mut docker_args = vec![
        "exec".to_string(),
        "-w".to_string(),
        "/workspace".to_string(),
        container_id.to_string(),
    ];
    docker_args.extend(args);
    let output = run_docker(&docker_args, None, 45).await?;
    if output.status.success() {
        return Ok(());
    }
    let stderr = String::from_utf8_lossy(&output.stderr);
    let stdout = String::from_utf8_lossy(&output.stdout);
    let detail = if !stderr.trim().is_empty() {
        stderr.trim().to_string()
    } else {
        stdout.trim().to_string()
    };
    anyhow::bail!("{} failed in app container: {}", label, detail);
}

async fn run_app_stop_command(
    app_id: &str,
    app_dir: &Path,
    container_id: Option<&str>,
    command: &str,
) {
    let localized = localize_app_entry_command(command, app_dir);
    let result = if let Some(container_id) = container_id {
        if is_container_running(container_id).await {
            run_container_lifecycle_command(container_id, &localized, "stop_command").await
        } else {
            Ok(())
        }
    } else {
        let env = build_local_lifecycle_env(app_dir);
        run_local_command_with_progress(
            &localized,
            "stop_command",
            app_dir,
            &env,
            45,
            &None,
            "stop",
        )
        .await
        .map(|_| ())
    };
    if let Err(error) = result {
        tracing::warn!(
            app_id = %app_id,
            command = %localized,
            error = %error,
            "App stop command failed; continuing with managed runtime stop"
        );
    }
}

pub async fn launch_dynamic_container(
    app_id: &str,
    app_dir: &Path,
    entry_command: &str,
    install_command: Option<&str>,
    port: u16,
    extra_env: &HashMap<String, String>,
    runtime_image: Option<&str>,
) -> Result<String> {
    if !docker_cli_available() {
        anyhow::bail!("container runtime unavailable: docker executable was not found on PATH");
    }

    let container_name = app_container_name(app_id);
    cleanup_existing_container(&container_name).await;

    let mut entry_cmd = validate_app_command(entry_command, "entry_command")?;
    entry_cmd = apply_app_mount_base_to_vite_entry_command(&entry_cmd, app_dir, app_id)?;
    let mut install_cmd = if let Some(cmd) = install_command {
        Some(validate_app_command(cmd, "install_command")?)
    } else {
        None
    };
    if install_cmd
        .as_deref()
        .is_some_and(|cmd| should_skip_redundant_install_command(app_dir, cmd))
    {
        install_cmd = None;
    }
    let uses_python_runtime = command_looks_python_related(&entry_cmd)
        || install_cmd
            .as_deref()
            .map(command_looks_python_related)
            .unwrap_or(false);
    if uses_python_runtime {
        entry_cmd = normalize_python_runtime_command_for_container(&entry_cmd);
    }

    let mut setup_parts: Vec<String> = Vec::new();
    setup_parts.push("set -e".to_string());
    setup_parts.push("export PATH=\"/workspace/node_modules/.bin:$PATH\"".to_string());
    setup_parts
        .push("export PYTHONPATH=\"/workspace/_deps${PYTHONPATH:+:$PYTHONPATH}\"".to_string());
    if uses_python_runtime {
        setup_parts.push(
            "if [ ! -x /workspace/.venv/bin/python ]; then python3 -m venv /workspace/.venv || python -m venv /workspace/.venv || true; fi".to_string(),
        );
        setup_parts.push(
            "if [ -x /workspace/.venv/bin/python ]; then . /workspace/.venv/bin/activate; fi"
                .to_string(),
        );
        setup_parts.push("export PIP_DISABLE_PIP_VERSION_CHECK=1".to_string());
        setup_parts.push("export PIP_BREAK_SYSTEM_PACKAGES=1".to_string());
    }
    let image = resolve_runtime_image(runtime_image).await;
    let network_container_ref = discover_current_container_ref().await;
    let env_file_path = write_runtime_env_file(app_dir, extra_env).await?;

    if let Some(ref cmd) = install_cmd {
        let trimmed = cmd.trim();
        if !trimmed.is_empty() {
            let normalized = if uses_python_runtime {
                normalize_python_runtime_command_for_container(trimmed)
            } else {
                trimmed.to_string()
            };
            let mut install_parts = setup_parts.clone();
            install_parts.push(normalized);
            let install_script = install_parts
                .join(" && ")
                .replace("{PORT}", &port.to_string());
            let install_container_name = format!(
                "{}-install-{}",
                app_container_name(app_id),
                &uuid::Uuid::new_v4().to_string()[..8]
            );
            let install_args = build_dynamic_container_install_args(
                app_id,
                app_dir,
                port,
                &image,
                install_container_name,
                env_file_path.as_deref(),
                install_script,
            );
            let install_output =
                run_docker(&install_args, None, dynamic_runtime_install_timeout_secs()).await;
            match install_output {
                Ok(output) if output.status.success() => {}
                Ok(output) => {
                    let _ = env_file_path.as_ref().map(std::fs::remove_file);
                    let stderr = String::from_utf8_lossy(&output.stderr);
                    let stdout = String::from_utf8_lossy(&output.stdout);
                    let detail = if !stderr.trim().is_empty() {
                        stderr.trim().to_string()
                    } else {
                        stdout.trim().to_string()
                    };
                    anyhow::bail!("install_command failed for app {}: {}", app_id, detail);
                }
                Err(error) => {
                    let _ = env_file_path.as_ref().map(std::fs::remove_file);
                    return Err(error)
                        .with_context(|| format!("install_command failed for app {}", app_id));
                }
            }
        }
    }

    let mut launch_parts = setup_parts;
    launch_parts.push(entry_cmd.trim().to_string());
    let launch_script = launch_parts
        .join(" && ")
        .replace("{PORT}", &port.to_string());
    let args = build_dynamic_container_run_args(
        app_id,
        app_dir,
        port,
        &image,
        container_name,
        env_file_path.as_deref(),
        network_container_ref.as_deref(),
        launch_script,
    );

    let output = run_docker(&args, None, docker_launch_timeout_secs()).await;
    if let Some(path) = env_file_path {
        let _ = tokio::fs::remove_file(path).await;
    }
    let output = output?;
    if !output.status.success() {
        let stderr = String::from_utf8_lossy(&output.stderr);
        anyhow::bail!("docker run failed: {}", stderr.trim());
    }
    let container_id = String::from_utf8_lossy(&output.stdout).trim().to_string();
    if container_id.is_empty() {
        anyhow::bail!("docker run did not return a container id");
    }
    Ok(container_id)
}

pub async fn launch_dynamic_process(
    app_id: &str,
    app_dir: &Path,
    entry_command: &str,
    install_command: Option<&str>,
    port: u16,
    extra_env: &HashMap<String, String>,
    stream_tx: Option<Sender<StreamEvent>>,
) -> Result<tokio::process::Child> {
    // Normalize absolute /app/ paths to relative: entry commands are often authored
    // for container context where the app lives at /app/, but for local process runtime
    // the cwd is already app_dir so these should be relative.
    let localized_entry =
        localize_app_entry_command(entry_command, app_dir).replace("{PORT}", &port.to_string());
    let entry_command = validate_app_command(
        &apply_app_mount_base_to_vite_entry_command(&localized_entry, app_dir, app_id)?,
        "entry_command",
    )?;

    let mut install_command = if let Some(cmd) = install_command {
        Some(validate_app_command(
            &cmd.replace("{PORT}", &port.to_string()),
            "install_command",
        )?)
    } else {
        None
    };
    if install_command
        .as_deref()
        .is_some_and(|cmd| should_skip_redundant_install_command(app_dir, cmd))
    {
        install_command = None;
    }

    let mut runtime_env: HashMap<String, String> = HashMap::new();
    runtime_env.insert("PORT".to_string(), port.to_string());
    runtime_env.insert("HOST".to_string(), "0.0.0.0".to_string());
    runtime_env.extend(extra_env.clone());

    // Legacy: support old --target _deps installs for backward compat.
    let deps_dir = app_dir.join("_deps");
    if deps_dir.exists() {
        let deps = deps_dir.to_string_lossy().to_string();
        let merged = std::env::var("PYTHONPATH")
            .map(|existing| {
                if existing.trim().is_empty() {
                    deps.clone()
                } else if cfg!(windows) {
                    format!("{};{}", deps, existing)
                } else {
                    format!("{}:{}", deps, existing)
                }
            })
            .unwrap_or(deps);
        runtime_env.insert("PYTHONPATH".to_string(), merged);
    }

    if let Some(path) = with_node_bin_path(app_dir) {
        runtime_env.insert("PATH".to_string(), path);
    }

    // Bootstrap a venv when the entry/install command obviously calls Python,
    // OR when the app declares Python deps even if the entry is something
    // opaque like `make serve`. Pure Node/Bun/Go apps skip this entirely.
    let uses_python_runtime = command_looks_python_related(&entry_command)
        || install_command
            .as_deref()
            .map(command_looks_python_related)
            .unwrap_or(false)
        || app_declares_python_deps(app_dir);
    if uses_python_runtime {
        match ensure_local_python_venv(app_dir).await {
            Ok((venv_dir, venv_bin_dir)) => {
                if let Some(merged_path) =
                    prepend_path_entry(&venv_bin_dir, runtime_env.get("PATH").map(|v| v.as_str()))
                {
                    runtime_env.insert("PATH".to_string(), merged_path);
                }
                runtime_env.insert(
                    "VIRTUAL_ENV".to_string(),
                    venv_dir.to_string_lossy().to_string(),
                );
            }
            Err(err) => {
                tracing::warn!(
                    "Python venv bootstrap unavailable for app {}. Falling back to system Python: {}",
                    app_id,
                    err
                );
            }
        }
        runtime_env.insert("PIP_DISABLE_PIP_VERSION_CHECK".to_string(), "1".to_string());
        runtime_env.insert("PIP_BREAK_SYSTEM_PACKAGES".to_string(), "1".to_string());
    }

    if let Some(ref cmd) = install_command {
        let install_args = split_command_args(cmd, "install_command")?;
        ensure_required_runtimes_available(
            app_id,
            "install_command",
            &install_args,
            runtime_env.get("PATH").map(|v| v.as_str()),
        )?;
        let output = run_local_command_with_progress(
            cmd,
            "install_command",
            app_dir,
            &runtime_env,
            dynamic_runtime_install_timeout_secs(),
            &stream_tx,
            "install",
        )
        .await?;
        if !output.status.success() {
            let stderr = String::from_utf8_lossy(&output.stderr);
            let stdout = String::from_utf8_lossy(&output.stdout);
            let detail = if !stderr.trim().is_empty() {
                stderr.trim().to_string()
            } else {
                stdout.trim().to_string()
            };
            anyhow::bail!("install_command failed for app {}: {}", app_id, detail);
        }
    }

    // If the app ships a package.json but no install_command (or one that
    // didn't populate node_modules), auto-install with the right manager.
    // Per-app `node_modules` keeps installs isolated; we never touch a global
    // prefix.
    if let Some(node_install) = detect_node_install_command(app_dir) {
        let node_install_args = split_command_args(node_install, "node_install")?;
        ensure_required_runtimes_available(
            app_id,
            "node_install",
            &node_install_args,
            runtime_env.get("PATH").map(|v| v.as_str()),
        )?;
        tracing::info!(
            "auto-installing node deps for app {} via `{}`",
            app_id,
            node_install
        );
        let output = run_local_command_with_progress(
            node_install,
            "node_install",
            app_dir,
            &runtime_env,
            dynamic_runtime_install_timeout_secs(),
            &stream_tx,
            "install",
        )
        .await?;
        if !output.status.success() {
            let stderr = String::from_utf8_lossy(&output.stderr);
            let stdout = String::from_utf8_lossy(&output.stdout);
            let detail = if !stderr.trim().is_empty() {
                stderr.trim().to_string()
            } else {
                stdout.trim().to_string()
            };
            anyhow::bail!(
                "auto `{}` failed for app {}: {}",
                node_install,
                app_id,
                detail
            );
        }
        // Refresh PATH so the freshly populated `node_modules/.bin` is on it
        // for the entry-command spawn that follows.
        if let Some(path) = with_node_bin_path(app_dir) {
            runtime_env.insert("PATH".to_string(), path);
        }
    }

    let (stdout_log_path, stderr_log_path) = prepare_local_runtime_log_files(app_dir)?;
    let args = split_command_args(&entry_command, "entry_command")?;

    // Pre-spawn runtime detection. If the binary the entry command needs is
    // not on PATH (with the venv/node_modules already merged in), bail with a
    // concrete remediation hint instead of letting the spawn produce ENOENT.
    // We never auto-install OS packages - that requires root and is a
    // security risk on a shared orchestrator.
    let runtime_path = runtime_env.get("PATH").map(|v| v.as_str());
    ensure_required_runtimes_available(app_id, "entry_command", &args, runtime_path)?;

    let (mut child, _resolved_program) = spawn_local_process_with_fallback(
        &args,
        "entry_command",
        app_dir,
        &runtime_env,
        &stdout_log_path,
        &stderr_log_path,
    )
    .await?;

    tokio::time::sleep(std::time::Duration::from_millis(450)).await;
    if let Some(status) = child
        .try_wait()
        .map_err(|e| anyhow::anyhow!("failed to check app {} process status: {}", app_id, e))?
    {
        let log_tail = read_local_runtime_log_tail(app_dir, LOCAL_RUNTIME_LOG_TAIL_BYTES).await;
        if log_tail.is_empty() {
            anyhow::bail!("app {} exited immediately with status {}", app_id, status);
        }
        anyhow::bail!(
            "app {} exited immediately with status {}. Recent runtime logs:\n{}",
            app_id,
            status,
            log_tail
        );
    }

    Ok(child)
}

pub enum DynamicRuntimeHandle {
    Container(String),
    Process(Box<tokio::process::Child>),
}

pub async fn stop_dynamic_runtime_handle(app_id: &str, handle: &mut DynamicRuntimeHandle) {
    match handle {
        DynamicRuntimeHandle::Container(container_id) => {
            if let Err(error) = stop_container(container_id.as_str()).await {
                tracing::warn!(
                    app_id = %app_id,
                    container_id = %container_id.as_str(),
                    error = %error,
                    "Failed to stop unregistered app container after readiness failure"
                );
            }
        }
        DynamicRuntimeHandle::Process(child) => {
            if let Err(error) = stop_child_process(child.as_mut(), app_id).await {
                tracing::warn!(
                    app_id = %app_id,
                    error = %error,
                    "Failed to stop unregistered app process after readiness failure"
                );
            }
        }
    }
}

pub struct DynamicRuntimeLaunch<'a> {
    pub app_id: &'a str,
    pub app_dir: &'a Path,
    pub entry_command: &'a str,
    pub install_command: Option<&'a str>,
    pub port: u16,
    pub extra_env: &'a HashMap<String, String>,
    pub runtime_image: Option<&'a str>,
    pub runtime_preference: RuntimePreference,
    pub stream_tx: Option<Sender<StreamEvent>>,
}

pub async fn launch_dynamic_runtime(
    request: DynamicRuntimeLaunch<'_>,
) -> Result<DynamicRuntimeHandle> {
    let DynamicRuntimeLaunch {
        app_id,
        app_dir,
        entry_command,
        install_command,
        port,
        extra_env,
        runtime_image,
        runtime_preference,
        stream_tx,
    } = request;

    // Decide once, per-app, whether Docker is genuinely required. This is
    // intent-driven - not a user-set env knob - so non-technical operators
    // don't have to reason about it: a custom `runtime_image` or compose
    // manifest requires containers; ordinary app dependencies still get a
    // local-process fallback when Docker is unavailable.
    let needs_docker = docker_required_for_app(app_dir, runtime_image);
    if needs_docker && !container_runtime_available() {
        anyhow::bail!(
            "app {} needs Docker (signals: {}). Start the Docker daemon to deploy this app, or remove the runtime_image/compose requirement to enable the host-process fallback.",
            app_id,
            describe_docker_signals(app_dir, runtime_image)
        );
    }
    let runtime_preference = if needs_docker {
        RuntimePreference::Container
    } else {
        runtime_preference
    };

    if matches!(runtime_preference, RuntimePreference::Local) && !needs_docker {
        match launch_dynamic_process(
            app_id,
            app_dir,
            entry_command,
            install_command,
            port,
            extra_env,
            stream_tx.clone(),
        )
        .await
        {
            Ok(child) => return Ok(DynamicRuntimeHandle::Process(Box::new(child))),
            Err(local_err) => {
                tracing::warn!(
                    "Local runtime launch unavailable for app {}: {}. Trying container fallback.",
                    app_id,
                    local_err
                );
                match launch_dynamic_container(
                    app_id,
                    app_dir,
                    entry_command,
                    install_command,
                    port,
                    extra_env,
                    runtime_image,
                )
                .await
                {
                    Ok(container_id) => return Ok(DynamicRuntimeHandle::Container(container_id)),
                    Err(container_err) => {
                        let docker_hint = if !container_runtime_available() {
                            " (No Docker daemon reachable from this orchestrator; install/start Docker to enable container fallback.)"
                        } else {
                            ""
                        };
                        return Err(anyhow::anyhow!(
                            "could not start app {}. Local runtime: {}. Container fallback: {}.{}",
                            app_id,
                            local_err,
                            container_err,
                            docker_hint
                        ));
                    }
                }
            }
        }
    }

    match launch_dynamic_container(
        app_id,
        app_dir,
        entry_command,
        install_command,
        port,
        extra_env,
        runtime_image,
    )
    .await
    {
        Ok(container_id) => Ok(DynamicRuntimeHandle::Container(container_id)),
        Err(container_err) => {
            if needs_docker {
                return Err(container_err);
            }
            tracing::warn!(
                "Container launch unavailable for app {}: {}. Falling back to local process runtime.",
                app_id,
                container_err
            );
            match launch_dynamic_process(
                app_id,
                app_dir,
                entry_command,
                install_command,
                port,
                extra_env,
                stream_tx.clone(),
            )
            .await
            {
                Ok(child) => Ok(DynamicRuntimeHandle::Process(Box::new(child))),
                Err(local_err) => {
                    let docker_hint = if !container_runtime_available() {
                        " (No Docker daemon reachable from this orchestrator; install/start Docker to make the container path available.)"
                    } else {
                        ""
                    };
                    Err(anyhow::anyhow!(
                        "could not start app {}. Container runtime: {}. Local fallback: {}.{}",
                        app_id,
                        container_err,
                        local_err,
                        docker_hint
                    ))
                }
            }
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum AppDeployProgressPhase {
    Planning,
    Deploying,
    GeneratingFiles,
    PreparingRuntime,
    Installing,
    StartingRuntime,
    WaitingForInputs,
    Completed,
}

impl AppDeployProgressPhase {
    fn as_str(self) -> &'static str {
        match self {
            AppDeployProgressPhase::Planning => "planning",
            AppDeployProgressPhase::Deploying => "deploying",
            AppDeployProgressPhase::GeneratingFiles => "generating_files",
            AppDeployProgressPhase::PreparingRuntime => "preparing_runtime",
            AppDeployProgressPhase::Installing => "installing",
            AppDeployProgressPhase::StartingRuntime => "starting_runtime",
            AppDeployProgressPhase::WaitingForInputs => "waiting_for_inputs",
            AppDeployProgressPhase::Completed => "completed",
        }
    }

    fn label(self) -> &'static str {
        match self {
            AppDeployProgressPhase::Planning => "Planning",
            AppDeployProgressPhase::Deploying => "Deploying",
            AppDeployProgressPhase::GeneratingFiles => "Generating files",
            AppDeployProgressPhase::PreparingRuntime => "Preparing runtime",
            AppDeployProgressPhase::Installing => "Installing",
            AppDeployProgressPhase::StartingRuntime => "Starting runtime",
            AppDeployProgressPhase::WaitingForInputs => "Waiting for inputs",
            AppDeployProgressPhase::Completed => "App ready",
        }
    }
}

fn app_deploy_phase_status_payload(
    phase: AppDeployProgressPhase,
    detail: &str,
) -> serde_json::Value {
    serde_json::json!({
        "kind": "phase_status",
        "phase": phase.as_str(),
        "label": phase.label(),
        "detail": detail,
        "elapsed_secs": 0,
        "stream_key": format!("phase-status:app_deploy:{}", phase.as_str()),
    })
}

async fn emit_phase_progress(
    stream_tx: &Option<Sender<StreamEvent>>,
    phase: AppDeployProgressPhase,
    message: impl Into<String>,
) {
    if let Some(tx) = stream_tx {
        let message = message.into();
        let _ = tx
            .send(StreamEvent::ToolProgress {
                name: "app_deploy".to_string(),
                content: message.clone(),
                payload: Some(app_deploy_phase_status_payload(phase, &message)),
            })
            .await;
    }
}

async fn emit_file_write_progress(
    stream_tx: &Option<Sender<StreamEvent>>,
    filename: &str,
    target_path: &Path,
    line: usize,
    total_lines: usize,
    text: &str,
    done: bool,
) {
    if let Some(tx) = stream_tx {
        let status = if total_lines > 0 {
            format!("writing {} line {}/{}", filename, line, total_lines)
        } else {
            format!("writing {} (empty file)", filename)
        };
        let payload = serde_json::json!({
            "kind": "file_write",
            "file": filename,
            "target_path": target_path.to_string_lossy(),
            "line": line,
            "total_lines": total_lines,
            "text": compact_progress_line(text, 240),
            "done": done,
        });
        let _ = tx
            .send(StreamEvent::ToolProgress {
                name: "app_deploy".to_string(),
                content: status,
                payload: Some(payload),
            })
            .await;
    }
}

async fn write_file_with_progress(
    file_path: &Path,
    filename: &str,
    content: &str,
    stream_tx: &Option<Sender<StreamEvent>>,
) -> Result<()> {
    if let Some(parent) = file_path.parent() {
        tokio::fs::create_dir_all(parent).await?;
    }
    let parent = file_path
        .parent()
        .ok_or_else(|| anyhow::anyhow!("Target file has no parent directory"))?;
    let final_name = file_path
        .file_name()
        .and_then(|value| value.to_str())
        .unwrap_or("app-file");
    let temp_path = parent.join(format!(
        ".{}.agentark-tmp-{}",
        final_name,
        uuid::Uuid::new_v4()
    ));
    let mut file = tokio::fs::File::create(&temp_path).await?;
    if content.is_empty() {
        emit_file_write_progress(stream_tx, filename, file_path, 0, 0, "", true).await;
        file.flush().await?;
        drop(file);
        replace_app_file(&temp_path, file_path).await?;
        return Ok(());
    }

    let segments: Vec<&str> = content.split_inclusive('\n').collect();
    let total_lines = segments.len();
    const FILE_WRITE_PROGRESS_MAX_EVENTS: usize = 8;
    let sampled_step = (total_lines / FILE_WRITE_PROGRESS_MAX_EVENTS).clamp(1, 250);
    for (idx, segment) in segments.iter().enumerate() {
        file.write_all(segment.as_bytes()).await?;
        let line_no = idx + 1;
        let is_last = line_no >= total_lines;
        if line_no == 1 || is_last || (line_no % sampled_step == 0) {
            let line_text = segment.trim_end_matches('\n').trim_end_matches('\r');
            emit_file_write_progress(
                stream_tx,
                filename,
                file_path,
                line_no,
                total_lines,
                line_text,
                is_last,
            )
            .await;
        }
    }
    file.flush().await?;
    drop(file);
    replace_app_file(&temp_path, file_path).await?;
    Ok(())
}

async fn replace_app_file(temp_path: &Path, file_path: &Path) -> Result<()> {
    match tokio::fs::rename(temp_path, file_path).await {
        Ok(_) => Ok(()),
        Err(first_error) => {
            if tokio::fs::try_exists(file_path).await.unwrap_or(false) {
                tokio::fs::remove_file(file_path)
                    .await
                    .with_context(|| format!("Failed to replace {}", file_path.display()))?;
                tokio::fs::rename(temp_path, file_path)
                    .await
                    .with_context(|| format!("Failed to install {}", file_path.display()))?;
                Ok(())
            } else {
                let _ = tokio::fs::remove_file(temp_path).await;
                Err(first_error).with_context(|| {
                    format!("Failed to move temporary file into {}", file_path.display())
                })
            }
        }
    }
}

fn app_source_workspace_root(data_dir: &Path) -> PathBuf {
    let fallback = std::env::current_dir()
        .ok()
        .unwrap_or_else(|| data_dir.to_path_buf());
    match std::env::var("AGENTARK_WORKSPACE_ROOT")
        .ok()
        .map(|value| value.trim().to_string())
        .filter(|value| !value.is_empty())
        .map(PathBuf::from)
    {
        Some(path) if path.is_absolute() => path,
        Some(path) => fallback.join(path),
        None => fallback,
    }
}

fn remap_app_source_alias_path(data_dir: &Path, raw: &str) -> Option<PathBuf> {
    let trimmed = raw.trim();
    const PREFIXES: &[&str] = &["/workspace", "/repo", "/project"];
    let matched = PREFIXES.iter().find(|prefix| {
        trimmed == **prefix
            || trimmed
                .strip_prefix(**prefix)
                .is_some_and(|rest| rest.starts_with('/'))
    })?;
    let suffix = trimmed.strip_prefix(matched).unwrap_or("");
    let relative = suffix.trim_start_matches('/');
    let root = app_source_workspace_root(data_dir);
    if relative.is_empty() {
        Some(root)
    } else {
        Some(root.join(relative))
    }
}

fn canonical_allowed_app_source_roots(data_dir: &Path) -> Vec<PathBuf> {
    let mut roots = vec![data_dir.to_path_buf(), app_source_workspace_root(data_dir)];
    if let Ok(cwd) = std::env::current_dir() {
        roots.push(cwd);
    }
    let mut deduped = Vec::new();
    for root in roots {
        let candidate = root.canonicalize().unwrap_or(root);
        if !deduped
            .iter()
            .any(|existing: &PathBuf| existing == &candidate)
        {
            deduped.push(candidate);
        }
    }
    deduped
}

fn resolve_staged_app_source_dir(data_dir: &Path, raw: &str) -> Result<PathBuf> {
    let trimmed = raw.trim();
    if trimmed.is_empty() {
        anyhow::bail!("source_dir cannot be empty");
    }
    let candidate = remap_app_source_alias_path(data_dir, trimmed).unwrap_or_else(|| {
        let path = PathBuf::from(trimmed);
        if path.is_absolute() {
            path
        } else {
            std::env::current_dir()
                .ok()
                .unwrap_or_else(|| data_dir.to_path_buf())
                .join(path)
        }
    });
    let resolved = candidate
        .canonicalize()
        .with_context(|| format!("source_dir '{}' does not exist", trimmed))?;
    if !resolved.is_dir() {
        anyhow::bail!("source_dir '{}' is not a directory", trimmed);
    }
    let allowed_roots = canonical_allowed_app_source_roots(data_dir);
    if allowed_roots.iter().any(|root| resolved.starts_with(root)) {
        Ok(resolved)
    } else {
        anyhow::bail!(
            "source_dir '{}' is outside the allowed workspace/data roots",
            trimmed
        );
    }
}

fn staged_app_source_path_looks_sensitive(path: &Path) -> bool {
    let Some(name) = path.file_name().and_then(|value| value.to_str()) else {
        return false;
    };
    let lower = name.trim().to_ascii_lowercase();
    lower == ".agentark_runtime_env"
        || lower == ".env"
        || lower.starts_with(".env.")
        || lower.ends_with(".pem")
        || lower.ends_with(".key")
        || lower.ends_with(".p12")
        || lower.ends_with(".pfx")
        || lower == "secrets.json"
        || lower == "credentials.json"
}

fn normalize_app_bundle_relative_path(raw: &str, field: &str) -> Result<String> {
    let normalized = raw.trim().replace('\\', "/");
    if normalized.is_empty() {
        anyhow::bail!("{} entries cannot be empty", field);
    }
    let path = Path::new(&normalized);
    if path.is_absolute() {
        anyhow::bail!("{} entries must be app-relative paths", field);
    }
    for component in path.components() {
        match component {
            std::path::Component::Normal(_) | std::path::Component::CurDir => {}
            _ => anyhow::bail!("{} path '{}' is not app-relative", field, raw),
        }
    }
    if normalized
        .split('/')
        .any(|part| part.is_empty() || part == ".")
    {
        anyhow::bail!("{} path '{}' contains an empty path segment", field, raw);
    }
    if is_app_internal_bundle_path(&normalized) {
        anyhow::bail!("{} path '{}' targets AgentArk app metadata", field, raw);
    }
    Ok(normalized)
}

fn normalize_staged_app_relative_path(raw: &str) -> Result<String> {
    normalize_app_bundle_relative_path(raw, "source_paths")
}

fn is_app_internal_bundle_path(path: &str) -> bool {
    let normalized = path.trim().replace('\\', "/");
    let mut parts = normalized.split('/').filter(|part| !part.is_empty());
    let first = parts.next().unwrap_or("");
    normalized == ".app_meta.json"
        || normalized == ".agentark_runtime_env"
        || normalized == LOCAL_RUNTIME_STDOUT_LOG_FILE
        || normalized == LOCAL_RUNTIME_STDERR_LOG_FILE
        || first == ".agentark"
        || first == ".git"
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum AppDeployMode {
    Replace,
    Patch,
}

impl AppDeployMode {
    fn from_arguments(arguments: &serde_json::Value) -> Result<Self> {
        let Some(raw) = arguments
            .get("mode")
            .and_then(|value| value.as_str())
            .map(str::trim)
            .filter(|value| !value.is_empty())
        else {
            return Ok(Self::Replace);
        };
        match raw {
            "replace" => Ok(Self::Replace),
            "patch" => Ok(Self::Patch),
            _ => anyhow::bail!("mode must be either 'replace' or 'patch'"),
        }
    }

    fn as_str(self) -> &'static str {
        match self {
            Self::Replace => "replace",
            Self::Patch => "patch",
        }
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct AppDeployFileWrite {
    pub path: String,
    pub content: String,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct AppDeployFilePatch {
    pub path: String,
    pub patch: String,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct AppDeployApplyPlan {
    pub mode: AppDeployMode,
    pub file_writes: Vec<AppDeployFileWrite>,
    pub file_patches: Vec<AppDeployFilePatch>,
    pub delete_paths: Vec<String>,
}

#[derive(Debug, Clone, PartialEq, Eq)]
struct AppDeployApplyOutcome {
    written_names: Vec<String>,
    deleted_names: Vec<String>,
}

fn parse_app_deploy_delete_paths(arguments: &serde_json::Value) -> Result<Vec<String>> {
    let Some(value) = arguments.get("delete_paths") else {
        return Ok(Vec::new());
    };
    let Some(paths) = value.as_array() else {
        anyhow::bail!("delete_paths must be an array of app-relative paths");
    };
    let mut out = Vec::new();
    let mut seen = HashSet::new();
    for raw in paths {
        let raw = raw
            .as_str()
            .ok_or_else(|| anyhow::anyhow!("delete_paths entries must be strings"))?;
        let path = normalize_app_bundle_relative_path(raw, "delete_paths")?;
        if !seen.insert(path.clone()) {
            anyhow::bail!("delete_paths contains duplicate path '{}'", path);
        }
        out.push(path);
    }
    Ok(out)
}

fn parse_app_deploy_file_patches(arguments: &serde_json::Value) -> Result<Vec<AppDeployFilePatch>> {
    let Some(value) = arguments.get("file_patches") else {
        return Ok(Vec::new());
    };
    let Some(items) = value.as_array() else {
        anyhow::bail!("file_patches must be an array of objects with path and patch");
    };
    let mut out = Vec::new();
    let mut seen = HashSet::new();
    for item in items {
        let obj = item
            .as_object()
            .ok_or_else(|| anyhow::anyhow!("file_patches entries must be objects"))?;
        let raw_path = obj
            .get("path")
            .and_then(|value| value.as_str())
            .ok_or_else(|| anyhow::anyhow!("file_patches entries require path"))?;
        let path = normalize_app_bundle_relative_path(raw_path, "file_patches")?;
        if !seen.insert(path.clone()) {
            anyhow::bail!("file_patches contains duplicate path '{}'", path);
        }
        let patch = obj
            .get("patch")
            .and_then(|value| value.as_str())
            .ok_or_else(|| anyhow::anyhow!("file_patches entries require patch"))?;
        if patch.trim().is_empty() {
            anyhow::bail!("file_patches entry for '{}' has an empty patch", path);
        }
        out.push(AppDeployFilePatch {
            path,
            patch: patch.to_string(),
        });
    }
    Ok(out)
}

async fn read_staged_app_source_files(
    data_dir: &Path,
    arguments: &serde_json::Value,
) -> Result<serde_json::Map<String, serde_json::Value>> {
    let source_dir = arguments
        .get("source_dir")
        .and_then(|value| value.as_str())
        .map(str::trim)
        .filter(|value| !value.is_empty())
        .ok_or_else(|| {
            anyhow::anyhow!(
                "Missing 'files': provide a files object or source_dir with source_paths"
            )
        })?;
    let source_paths = arguments
        .get("source_paths")
        .and_then(|value| value.as_array())
        .ok_or_else(|| {
            anyhow::anyhow!("Missing 'files': source_dir deployments also require source_paths")
        })?;
    if source_paths.is_empty() {
        anyhow::bail!("source_paths must contain at least one app-relative file path");
    }

    let root = resolve_staged_app_source_dir(data_dir, source_dir)?;
    let mut files = serde_json::Map::new();
    for raw_path in source_paths {
        let raw_path = raw_path
            .as_str()
            .ok_or_else(|| anyhow::anyhow!("source_paths entries must be strings"))?;
        let relative_path = normalize_staged_app_relative_path(raw_path)?;
        let candidate = root.join(&relative_path);
        let resolved = candidate.canonicalize().with_context(|| {
            format!(
                "Staged app source file '{}' was not found under source_dir",
                relative_path
            )
        })?;
        if !resolved.starts_with(&root) {
            anyhow::bail!("source path '{}' escapes source_dir", relative_path);
        }
        if !resolved.is_file() {
            anyhow::bail!("source path '{}' is not a file", relative_path);
        }
        if staged_app_source_path_looks_sensitive(&resolved) {
            anyhow::bail!(
                "Refusing to deploy sensitive staged file '{}'",
                relative_path
            );
        }
        let content = tokio::fs::read_to_string(&resolved)
            .await
            .with_context(|| format!("Failed to read staged app file '{}'", relative_path))?;
        files.insert(relative_path, serde_json::Value::String(content));
    }
    Ok(files)
}

async fn app_deploy_apply_plan_from_arguments(
    data_dir: &Path,
    arguments: &serde_json::Value,
) -> Result<AppDeployApplyPlan> {
    let mode = AppDeployMode::from_arguments(arguments)?;
    let staged_files: serde_json::Map<String, serde_json::Value>;
    let files = if let Some(files) = arguments.get("files").and_then(|v| v.as_object()) {
        Some(files)
    } else if arguments
        .get("source_dir")
        .and_then(|value| value.as_str())
        .map(str::trim)
        .is_some_and(|value| !value.is_empty())
    {
        staged_files = read_staged_app_source_files(data_dir, arguments).await?;
        Some(&staged_files)
    } else {
        None
    };

    let mut file_writes = Vec::new();
    let mut seen = HashSet::new();
    if let Some(files) = files {
        for (filename, content) in files {
            let path = normalize_app_bundle_relative_path(filename, "files")?;
            if !seen.insert(path.clone()) {
                anyhow::bail!("files contains duplicate path '{}'", path);
            }
            let content = content.as_str().ok_or_else(|| {
                anyhow::anyhow!(
                    "File '{}' must have string content; app_deploy does not accept nested file objects",
                    filename
                )
            })?;
            file_writes.push(AppDeployFileWrite {
                path,
                content: content.to_string(),
            });
        }
    }

    let file_patches = parse_app_deploy_file_patches(arguments)?;
    if !file_patches.is_empty() && mode != AppDeployMode::Patch {
        anyhow::bail!("file_patches require mode='patch'");
    }

    let delete_paths = parse_app_deploy_delete_paths(arguments)?;
    let delete_set = delete_paths.iter().collect::<HashSet<_>>();
    for write in &file_writes {
        if delete_set.contains(&write.path) {
            anyhow::bail!(
                "Path '{}' cannot be both written and deleted in one app_deploy call",
                write.path
            );
        }
    }
    for patch in &file_patches {
        if !seen.insert(patch.path.clone()) {
            anyhow::bail!("Path '{}' cannot be both written and patched", patch.path);
        }
        if delete_set.contains(&patch.path) {
            anyhow::bail!(
                "Path '{}' cannot be both patched and deleted in one app_deploy call",
                patch.path
            );
        }
    }

    Ok(AppDeployApplyPlan {
        mode,
        file_writes,
        file_patches,
        delete_paths,
    })
}

fn parse_unified_hunk_header(header: &str) -> Result<(usize, usize)> {
    let mut body = header.trim();
    if let Some(rest) = body.strip_prefix("@@") {
        body = rest;
    }
    if let Some((range_part, _)) = body.split_once("@@") {
        body = range_part;
    }
    let body = body.trim();
    let old_range = body
        .split_whitespace()
        .find(|part| part.starts_with('-'))
        .ok_or_else(|| anyhow::anyhow!("Unified diff hunk is missing original range"))?;
    let range = old_range.trim_start_matches('-');
    let mut parts = range.splitn(2, ',');
    let start = parts
        .next()
        .and_then(|value| value.parse::<usize>().ok())
        .ok_or_else(|| anyhow::anyhow!("Unified diff original range has invalid start"))?;
    let count = parts
        .next()
        .and_then(|value| value.parse::<usize>().ok())
        .unwrap_or(1);
    Ok((start, count))
}

fn split_text_lines_for_patch(text: &str) -> (Vec<String>, bool) {
    let had_trailing_newline = text.ends_with('\n');
    let lines = text
        .split('\n')
        .map(|line| line.strip_suffix('\r').unwrap_or(line).to_string())
        .collect::<Vec<_>>();
    if had_trailing_newline {
        (lines[..lines.len().saturating_sub(1)].to_vec(), true)
    } else {
        (lines, false)
    }
}

fn join_text_lines_after_patch(lines: &[String], trailing_newline: bool) -> String {
    let mut out = lines.join("\n");
    if trailing_newline {
        out.push('\n');
    }
    out
}

pub(crate) fn apply_unified_diff_to_text(original: &str, patch: &str) -> Result<String> {
    let (original_lines, had_trailing_newline) = split_text_lines_for_patch(original);
    let patch_lines = patch.lines().collect::<Vec<_>>();
    let mut out = Vec::<String>::new();
    let mut original_idx = 0usize;
    let mut patch_idx = 0usize;
    let mut saw_hunk = false;

    while patch_idx < patch_lines.len() {
        let line = patch_lines[patch_idx];
        if line.starts_with("--- ") || line.starts_with("+++ ") || line.trim().is_empty() {
            patch_idx += 1;
            continue;
        }
        if !line.starts_with("@@") {
            anyhow::bail!("Unified diff contains content outside a hunk: {}", line);
        }
        saw_hunk = true;
        let (old_start, _old_count) = parse_unified_hunk_header(line)?;
        let target_idx = old_start.saturating_sub(1);
        if target_idx < original_idx || target_idx > original_lines.len() {
            anyhow::bail!("Unified diff hunk range does not match current file");
        }
        out.extend(original_lines[original_idx..target_idx].iter().cloned());
        original_idx = target_idx;
        patch_idx += 1;

        while patch_idx < patch_lines.len() {
            let hunk_line = patch_lines[patch_idx];
            if hunk_line.starts_with("@@") {
                break;
            }
            if hunk_line == r"\ No newline at end of file" {
                patch_idx += 1;
                continue;
            }
            let Some(marker) = hunk_line.chars().next() else {
                anyhow::bail!("Unified diff hunk line is empty");
            };
            let content = &hunk_line[marker.len_utf8()..];
            match marker {
                ' ' => {
                    let Some(original_line) = original_lines.get(original_idx) else {
                        anyhow::bail!("Unified diff context extends beyond the file");
                    };
                    if original_line != content {
                        anyhow::bail!("Unified diff context did not match current file");
                    }
                    out.push(content.to_string());
                    original_idx += 1;
                }
                '-' => {
                    let Some(original_line) = original_lines.get(original_idx) else {
                        anyhow::bail!("Unified diff deletion extends beyond the file");
                    };
                    if original_line != content {
                        anyhow::bail!("Unified diff deletion did not match current file");
                    }
                    original_idx += 1;
                }
                '+' => out.push(content.to_string()),
                _ => anyhow::bail!("Unified diff hunk line must start with space, '+', or '-'"),
            }
            patch_idx += 1;
        }
    }

    if !saw_hunk {
        anyhow::bail!("Unified diff must contain at least one hunk");
    }
    out.extend(original_lines[original_idx..].iter().cloned());
    Ok(join_text_lines_after_patch(&out, had_trailing_newline))
}

fn app_meta_managed_files(meta: &Option<serde_json::Value>) -> Vec<String> {
    meta.as_ref()
        .and_then(|value| value.get("managed_files").and_then(|item| item.as_array()))
        .map(|items| {
            items
                .iter()
                .filter_map(|item| item.as_str())
                .filter_map(|item| normalize_app_bundle_relative_path(item, "managed_files").ok())
                .collect::<Vec<_>>()
        })
        .unwrap_or_default()
}

fn collect_existing_app_bundle_paths_sync(app_dir: &Path) -> Result<Vec<String>> {
    fn should_skip_restored_app_path(relative: &str) -> bool {
        let mut parts = relative.split('/').filter(|part| !part.is_empty());
        if let Some(first) = parts.next() {
            if matches!(
                first,
                ".agentark" | ".git" | ".venv" | "venv" | "node_modules" | "__pycache__" | "target"
            ) {
                return true;
            }
        }
        let file_name = relative.rsplit('/').next().unwrap_or(relative);
        file_name == ".app_meta.json"
            || file_name.starts_with(".agentark_runtime_")
            || file_name == "package-lock.json"
            || file_name == "yarn.lock"
            || file_name == "pnpm-lock.yaml"
    }

    fn visit(root: &Path, current: &Path, out: &mut Vec<String>) -> Result<()> {
        for entry in std::fs::read_dir(current)
            .with_context(|| format!("Failed to read app directory '{}'", current.display()))?
        {
            let entry = entry?;
            let path = entry.path();
            let file_type = entry.file_type()?;
            let relative = path
                .strip_prefix(root)
                .ok()
                .and_then(|value| value.to_str())
                .map(|value| value.replace('\\', "/"))
                .unwrap_or_default();
            if should_skip_restored_app_path(&relative) {
                continue;
            }
            if file_type.is_dir() {
                visit(root, &path, out)?;
                continue;
            }
            if !file_type.is_file() {
                continue;
            }
            let Ok(relative) = normalize_app_bundle_relative_path(&relative, "existing app files")
            else {
                continue;
            };
            out.push(relative);
        }
        Ok(())
    }

    let mut out = Vec::new();
    if app_dir.exists() {
        visit(app_dir, app_dir, &mut out)?;
    }
    out.sort_unstable();
    out.dedup();
    Ok(out)
}

fn collect_existing_app_text_files_sync(
    app_dir: &Path,
) -> Result<serde_json::Map<String, serde_json::Value>> {
    let mut out = serde_json::Map::new();
    for relative in collect_existing_app_bundle_paths_sync(app_dir)? {
        let path = app_dir.join(&relative);
        let Ok(content) = std::fs::read_to_string(&path) else {
            continue;
        };
        out.insert(relative, serde_json::Value::String(content));
    }
    Ok(out)
}

async fn load_existing_managed_app_text_files(
    app_dir: &Path,
    meta: &Option<serde_json::Value>,
) -> Result<serde_json::Map<String, serde_json::Value>> {
    let managed = app_meta_managed_files(meta);
    if managed.is_empty() {
        return collect_existing_app_text_files_sync(app_dir);
    }
    let mut out = serde_json::Map::new();
    for relative in managed {
        let path = app_dir.join(&relative);
        let Ok(content) = tokio::fs::read_to_string(&path).await else {
            continue;
        };
        out.insert(relative, serde_json::Value::String(content));
    }
    Ok(out)
}

async fn effective_app_files_for_validation(
    app_dir: Option<&Path>,
    meta: &Option<serde_json::Value>,
    plan: &AppDeployApplyPlan,
) -> Result<serde_json::Map<String, serde_json::Value>> {
    let mut files = if plan.mode == AppDeployMode::Patch {
        if let Some(app_dir) = app_dir {
            load_existing_managed_app_text_files(app_dir, meta).await?
        } else {
            serde_json::Map::new()
        }
    } else {
        serde_json::Map::new()
    };
    for path in &plan.delete_paths {
        files.remove(path);
    }
    for patch in &plan.file_patches {
        let current = files
            .get(&patch.path)
            .and_then(|value| value.as_str())
            .ok_or_else(|| {
                anyhow::anyhow!(
                    "Cannot patch '{}': file is not present in the current app bundle",
                    patch.path
                )
            })?;
        let patched = apply_unified_diff_to_text(current, &patch.patch)
            .with_context(|| format!("Failed to apply patch for {}", patch.path))?;
        files.insert(patch.path.clone(), serde_json::Value::String(patched));
    }
    for write in &plan.file_writes {
        files.insert(
            write.path.clone(),
            serde_json::Value::String(write.content.clone()),
        );
    }
    Ok(files)
}

fn app_deploy_allows_duplicate(arguments: &serde_json::Value) -> bool {
    arguments
        .get("allow_duplicate")
        .and_then(|value| value.as_bool())
        .unwrap_or(false)
        || arguments
            .get("duplicate_policy")
            .and_then(|value| value.as_str())
            .map(str::trim)
            .is_some_and(|value| value == "create_new")
}

fn update_hasher_with_json_value(hasher: &mut Sha256, value: &serde_json::Value) {
    match value {
        serde_json::Value::Null => hasher.update(b"null"),
        serde_json::Value::Bool(flag) => hasher.update(if *flag {
            b"true".as_slice()
        } else {
            b"false".as_slice()
        }),
        serde_json::Value::Number(number) => hasher.update(number.to_string().as_bytes()),
        serde_json::Value::String(text) => {
            hasher.update(b"\"");
            hasher.update(text.as_bytes());
            hasher.update(b"\"");
        }
        serde_json::Value::Array(items) => {
            hasher.update(b"[");
            for item in items {
                update_hasher_with_json_value(hasher, item);
                hasher.update(b",");
            }
            hasher.update(b"]");
        }
        serde_json::Value::Object(object) => {
            hasher.update(b"{");
            let mut keys = object.keys().collect::<Vec<_>>();
            keys.sort();
            for key in keys {
                hasher.update(key.as_bytes());
                hasher.update(b":");
                if let Some(value) = object.get(key) {
                    update_hasher_with_json_value(hasher, value);
                }
                hasher.update(b",");
            }
            hasher.update(b"}");
        }
    }
}

fn app_deploy_content_fingerprint(
    effective_files: &serde_json::Map<String, serde_json::Value>,
    title: &str,
    entry_command: Option<&str>,
    install_command: Option<&str>,
    stop_command: Option<&str>,
    runtime_required: bool,
    runtime_preference: RuntimePreference,
    runtime_image: Option<&str>,
    required_inputs: &[AppRequiredInput],
    config_values: &HashMap<String, String>,
    runtime_actions: &[String],
) -> String {
    let mut hasher = Sha256::new();
    hasher.update(b"agentark-app-content-v1\n");
    hasher.update(b"title\0");
    hasher.update(title.trim().as_bytes());
    hasher.update(b"\nentry\0");
    hasher.update(entry_command.unwrap_or("").trim().as_bytes());
    hasher.update(b"\ninstall\0");
    hasher.update(install_command.unwrap_or("").trim().as_bytes());
    hasher.update(b"\nstop\0");
    hasher.update(stop_command.unwrap_or("").trim().as_bytes());
    hasher.update(b"\nruntime_required\0");
    hasher.update(if runtime_required {
        b"true".as_slice()
    } else {
        b"false".as_slice()
    });
    hasher.update(b"\nruntime_preference\0");
    hasher.update(runtime_preference.as_str().as_bytes());
    hasher.update(b"\nruntime_image\0");
    hasher.update(runtime_image.unwrap_or("").trim().as_bytes());

    let required_inputs_json =
        serde_json::to_value(required_inputs).unwrap_or_else(|_| serde_json::json!([]));
    hasher.update(b"\nrequired_inputs\0");
    update_hasher_with_json_value(&mut hasher, &required_inputs_json);

    let config_values_json =
        serde_json::to_value(config_values).unwrap_or_else(|_| serde_json::json!({}));
    hasher.update(b"\nconfig_values\0");
    update_hasher_with_json_value(&mut hasher, &config_values_json);

    let runtime_actions_json =
        serde_json::to_value(runtime_actions).unwrap_or_else(|_| serde_json::json!([]));
    hasher.update(b"\nruntime_actions\0");
    update_hasher_with_json_value(&mut hasher, &runtime_actions_json);

    let mut file_paths = effective_files.keys().collect::<Vec<_>>();
    file_paths.sort();
    for path in file_paths {
        hasher.update(b"\nfile\0");
        hasher.update(path.as_bytes());
        hasher.update(b"\0");
        if let Some(body) = effective_files.get(path).and_then(|value| value.as_str()) {
            hasher.update(body.as_bytes());
        } else if let Some(value) = effective_files.get(path) {
            update_hasher_with_json_value(&mut hasher, value);
        }
    }

    hex::encode(hasher.finalize())
}

fn normalize_artifact_identity_text(value: &str) -> String {
    value
        .split_whitespace()
        .collect::<Vec<_>>()
        .join(" ")
        .trim()
        .to_ascii_lowercase()
}

fn push_identity_text(parts: &mut Vec<String>, label: &str, value: &str) {
    let normalized = normalize_artifact_identity_text(value);
    if !normalized.is_empty() {
        parts.push(format!("{}={}", label, normalized));
    }
}

fn collect_identity_urls_from_value(value: &serde_json::Value, out: &mut BTreeSet<String>) {
    match value {
        serde_json::Value::String(text) => {
            if text.starts_with("http://") || text.starts_with("https://") {
                out.insert(text.trim().to_ascii_lowercase());
            }
        }
        serde_json::Value::Array(items) => {
            for item in items {
                collect_identity_urls_from_value(item, out);
            }
        }
        serde_json::Value::Object(object) => {
            for value in object.values() {
                collect_identity_urls_from_value(value, out);
            }
        }
        _ => {}
    }
}

fn collect_identity_urls_from_text(text: &str, out: &mut BTreeSet<String>) {
    static URL_RE: std::sync::OnceLock<regex::Regex> = std::sync::OnceLock::new();
    let regex = URL_RE.get_or_init(|| {
        regex::Regex::new(r#"https?://[^\s"'<>)]+"#).expect("valid URL extraction regex")
    });
    for capture in regex.find_iter(text).take(32) {
        let url = capture
            .as_str()
            .trim_matches(|ch: char| matches!(ch, '.' | ',' | ';' | ')' | ']'))
            .to_ascii_lowercase();
        if !url.is_empty() {
            out.insert(url);
        }
    }
}

fn app_bundle_textual_data_signature(
    effective_files: &serde_json::Map<String, serde_json::Value>,
) -> Option<String> {
    let mut parts = Vec::new();
    static HTML_STYLE_RE: std::sync::OnceLock<regex::Regex> = std::sync::OnceLock::new();
    static HTML_SCRIPT_RE: std::sync::OnceLock<regex::Regex> = std::sync::OnceLock::new();
    let style_re = HTML_STYLE_RE.get_or_init(|| {
        regex::Regex::new(r"(?is)<style\b[^>]*>.*?</style>").expect("valid style block regex")
    });
    let script_re = HTML_SCRIPT_RE.get_or_init(|| {
        regex::Regex::new(r"(?is)<script\b[^>]*>.*?</script>").expect("valid script block regex")
    });
    for (path, value) in effective_files {
        let Some(text) = value.as_str() else {
            continue;
        };
        let path_lower = path.to_ascii_lowercase();
        if path_lower.ends_with(".css")
            || path_lower.ends_with(".svg")
            || path_lower.ends_with(".png")
            || path_lower.ends_with(".jpg")
            || path_lower.ends_with(".jpeg")
            || path_lower.ends_with(".webp")
            || path_lower.ends_with(".gif")
        {
            continue;
        }
        let body = if path_lower.ends_with(".html") || path_lower.ends_with(".htm") {
            let without_style = style_re.replace_all(text, " ");
            let without_script = script_re.replace_all(&without_style, " ");
            let html = Html::parse_document(&without_script);
            html.root_element().text().collect::<Vec<_>>().join(" ")
        } else {
            text.to_string()
        };
        let normalized = normalize_artifact_identity_text(&body);
        if !normalized.is_empty() {
            parts.push(format!("{}:{}", path_lower, normalized));
        }
    }
    if parts.is_empty() {
        return None;
    }
    parts.sort();
    let mut hasher = Sha256::new();
    hasher.update(b"agentark-app-data-v1\n");
    for part in parts {
        hasher.update(part.as_bytes());
        hasher.update(b"\n");
    }
    Some(hex::encode(hasher.finalize()))
}

fn app_deploy_artifact_identity_value(arguments: &serde_json::Value) -> Option<serde_json::Value> {
    let value = arguments
        .get("artifact_identity")
        .or_else(|| {
            arguments
                .get("metadata")
                .and_then(|metadata| metadata.get("artifact_identity"))
        })?
        .clone();
    match &value {
        serde_json::Value::Null => None,
        serde_json::Value::String(text) if text.trim().is_empty() => None,
        serde_json::Value::Array(items) if items.is_empty() => None,
        serde_json::Value::Object(object) if object.is_empty() => None,
        _ => Some(value),
    }
}

fn app_deploy_artifact_identity_signature(arguments: &serde_json::Value) -> Option<String> {
    let identity = app_deploy_artifact_identity_value(arguments)?;
    let mut hasher = Sha256::new();
    hasher.update(b"agentark-app-artifact-identity-v1\n");
    update_hasher_with_json_value(&mut hasher, &identity);
    Some(hex::encode(hasher.finalize()))
}

fn app_deploy_identity_fingerprint(
    arguments: &serde_json::Value,
    effective_files: &serde_json::Map<String, serde_json::Value>,
    title: &str,
) -> Option<String> {
    let mut parts = Vec::new();
    let structured_identity_signature = app_deploy_artifact_identity_signature(arguments);
    if structured_identity_signature.is_none() {
        push_identity_text(&mut parts, "title", title);
    }
    for key in [
        "repo_url",
        "repo_ref",
        "repo_subdir",
        "source_url",
        "data_url",
        "canonical_url",
        "artifact_key",
        "workflow_key",
    ] {
        if let Some(value) = arguments.get(key).and_then(|value| value.as_str()) {
            push_identity_text(&mut parts, key, value);
        }
    }
    let mut urls = BTreeSet::new();
    if let Some(identity) = app_deploy_artifact_identity_value(arguments) {
        collect_identity_urls_from_value(&identity, &mut urls);
    }
    if let Some(metadata) = arguments.get("metadata") {
        collect_identity_urls_from_value(metadata, &mut urls);
    }
    for value in effective_files.values() {
        if let Some(text) = value.as_str() {
            collect_identity_urls_from_text(text, &mut urls);
        }
    }
    for url in urls.iter().take(16) {
        push_identity_text(&mut parts, "url", url);
    }
    let has_source_identity = parts
        .iter()
        .any(|part| part.starts_with("url=") || part.starts_with("repo_url="));
    if !has_source_identity {
        return None;
    }
    if let Some(signature) = structured_identity_signature {
        parts.push(format!("artifact_identity={}", signature));
    } else {
        let data_signature = app_bundle_textual_data_signature(effective_files)?;
        parts.push(format!("data={}", data_signature));
    }
    parts.sort();
    let mut hasher = Sha256::new();
    hasher.update(b"agentark-app-identity-v1\n");
    for part in parts {
        hasher.update(part.as_bytes());
        hasher.update(b"\n");
    }
    Some(hex::encode(hasher.finalize()))
}

async fn app_deploy_existing_fingerprint(
    app_dir: &Path,
    meta: &serde_json::Value,
) -> Option<String> {
    if let Some(fingerprint) = meta
        .get("content_fingerprint")
        .and_then(|value| value.as_str())
        .map(str::trim)
        .filter(|value| !value.is_empty())
    {
        return Some(fingerprint.to_string());
    }

    let meta_opt = Some(meta.clone());
    let files = load_existing_managed_app_text_files(app_dir, &meta_opt)
        .await
        .ok()?;
    if files.is_empty() {
        return None;
    }
    let title = meta
        .get("title")
        .and_then(|value| value.as_str())
        .unwrap_or("App");
    let entry_command = app_meta_lifecycle_command(meta, "entry_command");
    let install_command = app_meta_lifecycle_command(meta, "install_command");
    let stop_command = app_meta_lifecycle_command(meta, "stop_command");
    let runtime_required = meta
        .get("runtime_required")
        .and_then(|value| value.as_bool())
        .unwrap_or_else(|| entry_command.is_some());
    let runtime_preference = runtime_preference_from_opt(
        meta.get("runtime_preference")
            .and_then(|value| value.as_str()),
    );
    let runtime_image = meta.get("runtime_image").and_then(|value| value.as_str());
    let required_inputs = parse_required_inputs(meta);
    let config_values = meta
        .get("config_values")
        .and_then(|value| value.as_object())
        .map(|object| {
            object
                .iter()
                .filter_map(|(key, value)| match value {
                    serde_json::Value::String(text) => Some((key.clone(), text.clone())),
                    serde_json::Value::Bool(flag) => Some((key.clone(), flag.to_string())),
                    serde_json::Value::Number(number) => Some((key.clone(), number.to_string())),
                    _ => None,
                })
                .collect::<HashMap<_, _>>()
        })
        .unwrap_or_default();
    let runtime_actions = parse_runtime_actions(meta);
    Some(app_deploy_content_fingerprint(
        &files,
        title,
        entry_command.as_deref(),
        install_command.as_deref(),
        stop_command.as_deref(),
        runtime_required,
        runtime_preference,
        runtime_image,
        &required_inputs,
        &config_values,
        &runtime_actions,
    ))
}

async fn app_deploy_existing_identity_fingerprint(
    app_dir: &Path,
    meta: &serde_json::Value,
) -> Option<String> {
    if let Some(fingerprint) = meta
        .get("artifact_identity_fingerprint")
        .and_then(|value| value.as_str())
        .map(str::trim)
        .filter(|value| !value.is_empty())
    {
        return Some(fingerprint.to_string());
    }
    let meta_opt = Some(meta.clone());
    let files = load_existing_managed_app_text_files(app_dir, &meta_opt)
        .await
        .ok()?;
    let title = meta
        .get("title")
        .and_then(|value| value.as_str())
        .unwrap_or("App");
    app_deploy_identity_fingerprint(meta, &files, title)
}

async fn find_duplicate_deployed_app(
    registry: &AppRegistry,
    fingerprint: &str,
    identity_fingerprint: Option<&str>,
    exclude_app_id: Option<&str>,
) -> Option<AppDuplicateMatch> {
    let apps = registry.list().await;
    for app in apps {
        let Some(app_id) = app
            .get("id")
            .and_then(|value| value.as_str())
            .map(str::trim)
            .filter(|value| !value.is_empty())
        else {
            continue;
        };
        if exclude_app_id.is_some_and(|exclude| exclude == app_id) {
            continue;
        }
        let Some(app_dir) = registry.get_dir(app_id).await else {
            continue;
        };
        let meta = load_app_meta_json(&app_dir).await;
        let content_matches = app_deploy_existing_fingerprint(&app_dir, &meta)
            .await
            .is_some_and(|existing| existing == fingerprint);
        let identity_matches = if let Some(identity) = identity_fingerprint {
            app_deploy_existing_identity_fingerprint(&app_dir, &meta)
                .await
                .is_some_and(|existing| existing == identity)
        } else {
            false
        };
        if !content_matches && !identity_matches {
            continue;
        }
        let title = app
            .get("title")
            .and_then(|value| value.as_str())
            .or_else(|| meta.get("title").and_then(|value| value.as_str()))
            .unwrap_or("App")
            .to_string();
        let url = app
            .get("url")
            .and_then(|value| value.as_str())
            .map(str::trim)
            .filter(|value| !value.is_empty())
            .map(ToString::to_string)
            .unwrap_or_else(|| format!("/apps/{}/", app_id));
        let access_url = app
            .get("access_url")
            .and_then(|value| value.as_str())
            .map(str::trim)
            .filter(|value| !value.is_empty())
            .map(ToString::to_string)
            .unwrap_or_else(|| url.clone());
        let app_type = if app
            .get("is_static")
            .and_then(|value| value.as_bool())
            .unwrap_or(false)
        {
            "static"
        } else {
            "dynamic"
        }
        .to_string();
        return Some(AppDuplicateMatch {
            app_id: app_id.to_string(),
            title,
            url,
            access_url,
            app_type,
            updated_existing: false,
            duplicate_match: if content_matches {
                "content"
            } else {
                "artifact_identity"
            }
            .to_string(),
        });
    }
    None
}

async fn app_deploy_duplicate_response(
    duplicate: AppDuplicateMatch,
    fingerprint: &str,
    deploy_started: std::time::Instant,
) -> Result<String> {
    tracing::info!(
        app_id = %duplicate.app_id,
        title = %duplicate.title,
        "Skipped duplicate app deployment because a matching app already exists"
    );
    tracing::debug!(
        target: "agentark.turn_timing",
        stage = "app_deploy_total",
        app_id = %duplicate.app_id,
        app_type = %duplicate.app_type,
        duration_ms = deploy_started.elapsed().as_millis() as u64,
        updated_existing = duplicate.updated_existing,
        duplicate_skipped = true,
        "app deploy timing total"
    );
    Ok(serde_json::json!({
        "status": "duplicate_skipped",
        "type": duplicate.app_type,
        "app_id": duplicate.app_id,
        "url": duplicate.url,
        "access_url": duplicate.access_url,
        "title": duplicate.title,
        "updated_existing": duplicate.updated_existing,
        "duplicate_skipped": true,
        "duplicate_match": duplicate.duplicate_match,
        "content_fingerprint": fingerprint,
        "apps_page_hint": APP_DEPLOY_CONTROL_HINT,
        "message": "A matching app already exists, so AgentArk skipped creating another duplicate deployment. Set allow_duplicate=true or duplicate_policy=create_new to create another copy."
    })
    .to_string())
}

async fn delete_empty_parent_dirs(app_dir: &Path, path: &str) {
    let Some(mut parent) = app_dir.join(path).parent().map(Path::to_path_buf) else {
        return;
    };
    while parent.starts_with(app_dir) && parent != app_dir {
        match tokio::fs::remove_dir(&parent).await {
            Ok(_) => {
                let Some(next) = parent.parent().map(Path::to_path_buf) else {
                    break;
                };
                parent = next;
            }
            Err(_) => break,
        }
    }
}

async fn app_deploy_apply(
    app_dir: &Path,
    plan: &AppDeployApplyPlan,
    previous_meta: &Option<serde_json::Value>,
    stream_tx: &Option<Sender<StreamEvent>>,
) -> Result<AppDeployApplyOutcome> {
    tokio::fs::create_dir_all(app_dir).await?;
    let previous_managed = app_meta_managed_files(previous_meta);
    let declared_paths = plan
        .file_writes
        .iter()
        .map(|write| write.path.clone())
        .chain(plan.file_patches.iter().map(|patch| patch.path.clone()))
        .collect::<HashSet<_>>();
    let mut removed = HashSet::new();
    let mut written_names = Vec::new();
    let mut deleted_names = Vec::new();

    for path in &plan.delete_paths {
        let target = app_dir.join(path);
        match tokio::fs::remove_file(&target).await {
            Ok(_) => {
                removed.insert(path.clone());
                deleted_names.push(path.clone());
                emit_phase_progress(
                    stream_tx,
                    AppDeployProgressPhase::GeneratingFiles,
                    format!("Deleted {}", path),
                )
                .await;
                delete_empty_parent_dirs(app_dir, path).await;
            }
            Err(error) if error.kind() == std::io::ErrorKind::NotFound => {}
            Err(error) => return Err(error).with_context(|| format!("Failed to delete {}", path)),
        }
    }

    if plan.mode == AppDeployMode::Replace {
        let orphan_candidates = if previous_managed.is_empty() {
            collect_existing_app_bundle_paths_sync(app_dir)?
        } else {
            previous_managed
        };
        for path in orphan_candidates {
            if declared_paths.contains(&path) || removed.contains(&path) {
                continue;
            }
            let target = app_dir.join(&path);
            match tokio::fs::remove_file(&target).await {
                Ok(_) => {
                    removed.insert(path.clone());
                    deleted_names.push(path.clone());
                    emit_phase_progress(
                        stream_tx,
                        AppDeployProgressPhase::GeneratingFiles,
                        format!("Deleted stale {}", path),
                    )
                    .await;
                    delete_empty_parent_dirs(app_dir, &path).await;
                }
                Err(error) if error.kind() == std::io::ErrorKind::NotFound => {}
                Err(error) => {
                    return Err(error).with_context(|| format!("Failed to delete stale {}", path));
                }
            }
        }
    }

    for patch in &plan.file_patches {
        let file_path = app_dir.join(&patch.path);
        let current = tokio::fs::read_to_string(&file_path)
            .await
            .with_context(|| format!("Cannot patch '{}': file is not readable", patch.path))?;
        let patched = apply_unified_diff_to_text(&current, &patch.patch)
            .with_context(|| format!("Failed to apply patch for {}", patch.path))?;
        write_file_with_progress(&file_path, &patch.path, &patched, stream_tx)
            .await
            .with_context(|| format!("Failed to write patched {}", patch.path))?;
        emit_phase_progress(
            stream_tx,
            AppDeployProgressPhase::GeneratingFiles,
            format!("Patched {}", patch.path),
        )
        .await;
        written_names.push(patch.path.clone());
    }

    for write in &plan.file_writes {
        let file_path = app_dir.join(&write.path);
        if let Some(parent) = file_path.parent() {
            tokio::fs::create_dir_all(parent).await?;
        }
        let byte_len = write.content.len();
        write_file_with_progress(&file_path, &write.path, &write.content, stream_tx)
            .await
            .with_context(|| format!("Failed to write {}", write.path))?;
        written_names.push(write.path.clone());
        emit_phase_progress(
            stream_tx,
            AppDeployProgressPhase::GeneratingFiles,
            format!("Wrote {} ({}B)", write.path, byte_len),
        )
        .await;
    }
    Ok(AppDeployApplyOutcome {
        written_names,
        deleted_names,
    })
}

#[allow(dead_code)]
pub async fn app_deploy_preflight(
    data_dir: &Path,
    arguments: &serde_json::Value,
    registry: &AppRegistry,
) -> Result<()> {
    if should_deploy_repo_bundle(arguments) {
        let repo_url = arguments
            .get("repo_url")
            .and_then(|value| value.as_str())
            .map(str::trim)
            .filter(|value| !value.is_empty())
            .ok_or_else(|| anyhow::anyhow!("repo_url is required for repo deployments"))?;
        reqwest::Url::parse(repo_url)
            .with_context(|| format!("repo_url '{}' is not a valid URL", repo_url))?;
        return Ok(());
    }

    let plan = app_deploy_apply_plan_from_arguments(data_dir, arguments).await?;

    let mut existing_meta: Option<serde_json::Value> = None;
    let mut existing_app_dir: Option<PathBuf> = None;
    if let Some(app_id) = arguments
        .get("app_id")
        .and_then(|value| value.as_str())
        .map(str::trim)
        .filter(|value| !value.is_empty())
    {
        let existing_app = registry
            .list()
            .await
            .into_iter()
            .any(|app| app.get("id").and_then(|value| value.as_str()) == Some(app_id));
        let app_dir = registry
            .get_dir(app_id)
            .await
            .unwrap_or_else(|| data_dir.join("apps").join(app_id));
        if !existing_app && !app_dir.exists() {
            anyhow::bail!("No deployed app found for app_id '{}'", app_id);
        }
        existing_app_dir = Some(app_dir.clone());
        existing_meta = tokio::fs::read(app_dir.join(".app_meta.json"))
            .await
            .ok()
            .and_then(|bytes| serde_json::from_slice::<serde_json::Value>(&bytes).ok())
            .filter(|value| value.is_object());
    }
    if plan.mode == AppDeployMode::Patch && existing_app_dir.is_none() {
        anyhow::bail!("mode='patch' requires app_id for an existing deployed app");
    }
    if plan.file_writes.is_empty() && plan.file_patches.is_empty() && plan.delete_paths.is_empty() {
        anyhow::bail!(
            "app_deploy requires at least one file write, file patch, or delete_paths entry"
        );
    }

    let has_explicit_entry_command = arguments
        .get("entry_command")
        .or_else(|| arguments.get("start_command"))
        .or_else(|| {
            arguments
                .get("commands")
                .and_then(|value| value.get("start").or_else(|| value.get("entry")))
        })
        .and_then(|value| value.as_str())
        .map(str::trim)
        .filter(|value| !value.is_empty())
        .is_some()
        || existing_meta
            .as_ref()
            .and_then(|value| app_meta_lifecycle_command(value, "entry_command"))
            .map(|value| !value.trim().is_empty())
            .unwrap_or(false);
    let effective_files =
        effective_app_files_for_validation(existing_app_dir.as_deref(), &existing_meta, &plan)
            .await?;
    if effective_files.is_empty() {
        anyhow::bail!("App bundle would be empty after applying this deploy");
    }
    let inferred_lifecycle = infer_generated_bundle_lifecycle(&effective_files);
    if !has_explicit_entry_command && inferred_lifecycle.is_none() {
        validate_static_app_asset_references(&effective_files)?;
    }
    Ok(())
}

/// A running app process
pub struct RunningApp {
    pub title: String,
    pub port: Option<u16>,
    pub process: Option<tokio::process::Child>,
    pub container_id: Option<String>,
    pub app_dir: PathBuf,
    pub is_static: bool,
    pub created_at: chrono::DateTime<chrono::Utc>,
    pub last_accessed: chrono::DateTime<chrono::Utc>,
    /// Rolling request count since last pulse check (for traffic monitoring)
    pub request_count: u64,
    /// Random access key for app authentication
    pub access_key: String,
    /// Whether access guard/key is enforced.
    pub access_guard_enabled: bool,
    /// Whether this app was requested for public exposure.
    pub expose_public: bool,
    /// Whether the user wants this app enabled and serveable.
    pub enabled: bool,
    /// Whether this app is still being restored in the background.
    pub restoring: bool,
    /// Most recent restore-time warning or failure detail, if any.
    pub restore_error: Option<String>,
}

#[derive(Debug, Default, Clone, serde::Serialize)]
pub struct AppRestoreSnapshot {
    pub active: bool,
    pub total: usize,
    pub pending: usize,
    pub ready: usize,
    pub degraded: usize,
    pub started_at: Option<String>,
    pub completed_at: Option<String>,
}

#[derive(Debug, Default)]
struct AppRestoreTracker {
    active: bool,
    total: usize,
    pending: usize,
    ready: usize,
    degraded: usize,
    started_at: Option<chrono::DateTime<chrono::Utc>>,
    completed_at: Option<chrono::DateTime<chrono::Utc>>,
}

impl AppRestoreTracker {
    fn begin(&mut self, total: usize) {
        self.active = total > 0;
        self.total = total;
        self.pending = total;
        self.ready = 0;
        self.degraded = 0;
        self.started_at = Some(chrono::Utc::now());
        self.completed_at = if total == 0 {
            Some(chrono::Utc::now())
        } else {
            None
        };
    }

    fn finish_one(&mut self, degraded: bool) {
        if self.pending > 0 {
            self.pending -= 1;
        }
        if degraded {
            self.degraded += 1;
        } else {
            self.ready += 1;
        }
        if self.pending == 0 {
            self.active = false;
            self.completed_at = Some(chrono::Utc::now());
        }
    }

    fn snapshot(&self) -> AppRestoreSnapshot {
        AppRestoreSnapshot {
            active: self.active,
            total: self.total,
            pending: self.pending,
            ready: self.ready,
            degraded: self.degraded,
            started_at: self.started_at.map(|value| value.to_rfc3339()),
            completed_at: self.completed_at.map(|value| value.to_rfc3339()),
        }
    }
}

#[derive(Debug, Clone)]
struct AppAccessBootstrapGrant {
    app_id: String,
    expires_at: i64,
}

#[derive(Debug, Clone)]
struct AppAccessSession {
    app_id: String,
    #[allow(dead_code)]
    issued_at: i64,
    expires_at: i64,
    last_seen_at: i64,
}

/// Generate a random access key for app authentication
pub fn generate_access_key() -> String {
    format!("ak_{}", uuid::Uuid::new_v4().simple())
}

pub const APP_DEPLOY_CONTROL_HINT: &str = "Open the Apps page for start, stop, restart, logs, App Guard, public exposure, and delete controls.";

fn app_unix_now_ts() -> i64 {
    chrono::Utc::now().timestamp()
}

fn app_access_secret_name(app_id: &str) -> String {
    format!("app_access_key:{}", app_id)
}

fn relative_app_root_url(app_id: &str) -> String {
    format!("/apps/{}/", app_id)
}

fn relative_app_bootstrap_url(app_id: &str, grant: &str) -> String {
    format!("/apps/{}/?grant={}", app_id, urlencoding::encode(grant))
}

fn load_persisted_access_key_sync(
    config_dir: &Path,
    data_dir: &Path,
    app_id: &str,
) -> Result<Option<String>> {
    let manager =
        crate::core::config::SecureConfigManager::new_with_data_dir(config_dir, Some(data_dir))?;
    manager.get_custom_secret(&app_access_secret_name(app_id))
}

async fn read_optional_app_json(path: &Path) -> Option<serde_json::Value> {
    let bytes = tokio::fs::read(path).await.ok()?;
    serde_json::from_slice::<serde_json::Value>(&bytes).ok()
}

fn persist_access_key_sync(
    config_dir: &Path,
    data_dir: &Path,
    app_id: &str,
    access_key: Option<&str>,
) -> Result<()> {
    let manager =
        crate::core::config::SecureConfigManager::new_with_data_dir(config_dir, Some(data_dir))?;
    manager.set_custom_secret(
        &app_access_secret_name(app_id),
        access_key.map(|value| value.to_string()),
    )
}

async fn load_app_meta_json(app_dir: &Path) -> serde_json::Value {
    let meta_path = app_dir.join(".app_meta.json");
    let mut meta = match tokio::fs::read(&meta_path).await {
        Ok(bytes) => serde_json::from_slice(&bytes).unwrap_or_else(|_| serde_json::json!({})),
        Err(_) => serde_json::json!({}),
    };
    if !meta.is_object() {
        meta = serde_json::json!({});
    }
    meta
}

async fn read_existing_app_meta_json(app_dir: &Path) -> Result<serde_json::Value> {
    let meta_path = app_dir.join(".app_meta.json");
    let bytes = tokio::fs::read(&meta_path)
        .await
        .with_context(|| format!("Failed to read app metadata '{}'", meta_path.display()))?;
    let meta: serde_json::Value = serde_json::from_slice(&bytes)
        .with_context(|| format!("Failed to parse app metadata '{}'", meta_path.display()))?;
    if !meta.is_object() {
        anyhow::bail!(
            "App metadata '{}' is not a JSON object",
            meta_path.display()
        );
    }
    Ok(meta)
}

async fn write_app_meta_json(app_dir: &Path, meta: &serde_json::Value) -> Result<()> {
    let meta_path = app_dir.join(".app_meta.json");
    let bytes = serde_json::to_vec_pretty(meta)?;
    tokio::fs::write(&meta_path, bytes).await?;
    Ok(())
}

async fn update_existing_app_meta_json<F>(app_dir: &Path, update: F) -> Result<()>
where
    F: FnOnce(&mut serde_json::Value),
{
    let mut meta = read_existing_app_meta_json(app_dir).await?;
    update(&mut meta);
    write_app_meta_json(app_dir, &meta).await?;
    Ok(())
}

async fn persist_app_access_guard_meta(app_dir: &Path, access_guard_enabled: bool) -> Result<()> {
    update_existing_app_meta_json(app_dir, |meta| {
        meta["access_guard_enabled"] = serde_json::Value::Bool(access_guard_enabled);
        if let Some(obj) = meta.as_object_mut() {
            obj.remove("access_key");
        }
    })
    .await
}

async fn persist_app_enabled_meta(app_dir: &Path, enabled: bool) -> Result<()> {
    update_existing_app_meta_json(app_dir, |meta| {
        meta["enabled"] = serde_json::Value::Bool(enabled);
    })
    .await
}

async fn persist_app_last_accessed_meta(
    app_dir: &Path,
    last_accessed: chrono::DateTime<chrono::Utc>,
) -> Result<()> {
    update_existing_app_meta_json(app_dir, |meta| {
        meta["last_accessed"] = serde_json::Value::String(last_accessed.to_rfc3339());
    })
    .await
}

fn parse_app_meta_datetime(
    meta: &Option<serde_json::Value>,
    key: &str,
) -> Option<chrono::DateTime<chrono::Utc>> {
    meta.as_ref()
        .and_then(|m| m.get(key).and_then(|value| value.as_str()))
        .and_then(|raw| chrono::DateTime::parse_from_rfc3339(raw).ok())
        .map(|value| value.with_timezone(&chrono::Utc))
}

/// Snapshot of an app's health for Pulse reporting
pub struct AppHealthSnapshot {
    pub id: String,
    pub title: String,
    pub is_static: bool,
    pub process_alive: bool,
    pub requests_since_last_check: u64,
    pub last_accessed: chrono::DateTime<chrono::Utc>,
}

/// Global app registry: tracks deployed apps and their processes
#[derive(Clone)]
pub struct AppRegistry {
    apps: Arc<RwLock<HashMap<String, Arc<RwLock<RunningApp>>>>>,
    restore_tracker: Arc<RwLock<AppRestoreTracker>>,
    config_dir: Option<PathBuf>,
    data_dir: Option<PathBuf>,
    access_bootstrap_grants: Arc<RwLock<HashMap<String, AppAccessBootstrapGrant>>>,
    access_sessions: Arc<RwLock<HashMap<String, AppAccessSession>>>,
}

pub struct DynamicAppRegistration {
    pub title: String,
    pub app_dir: PathBuf,
    pub child: Option<tokio::process::Child>,
    pub container_id: Option<String>,
    pub port: u16,
    pub access_key: String,
    pub access_guard_enabled: bool,
    pub expose_public: bool,
    pub enabled: bool,
    pub last_accessed: Option<chrono::DateTime<chrono::Utc>>,
}

pub struct StoredAppRegistration {
    pub title: String,
    pub app_dir: PathBuf,
    pub is_static: bool,
    pub access_key: String,
    pub access_guard_enabled: bool,
    pub expose_public: bool,
    pub enabled: bool,
    pub last_accessed: Option<chrono::DateTime<chrono::Utc>>,
}

struct ExistingAppDeployTarget {
    app_id: String,
    title: String,
    app_dir: PathBuf,
    meta: Option<serde_json::Value>,
    access_guard_enabled: bool,
    access_key: Option<String>,
    expose_public: bool,
}

#[derive(Debug, Clone)]
struct AppDuplicateMatch {
    app_id: String,
    title: String,
    url: String,
    access_url: String,
    app_type: String,
    updated_existing: bool,
    duplicate_match: String,
}

#[derive(Debug, Clone)]
struct RestoreAppCandidate {
    id: String,
    title: String,
    app_dir: PathBuf,
    entry_command: Option<String>,
    install_command: Option<String>,
    runtime_image: Option<String>,
    runtime_preference: RuntimePreference,
    required_inputs: Vec<AppRequiredInput>,
    config_values: HashMap<String, String>,
    access_guard_enabled: bool,
    access_key: String,
    expose_public: bool,
    enabled: bool,
    last_accessed: Option<chrono::DateTime<chrono::Utc>>,
}

#[derive(Debug, Default)]
pub struct AppBootReconciliationReport {
    pub valid_app_ids: HashSet<String>,
    pub quarantined_app_ids: HashSet<String>,
}

impl AppRegistry {
    #[cfg_attr(not(test), allow(dead_code))]
    pub fn new() -> Self {
        Self::with_optional_paths(None, None)
    }

    pub fn with_paths(config_dir: PathBuf, data_dir: PathBuf) -> Self {
        Self::with_optional_paths(Some(config_dir), Some(data_dir))
    }

    fn with_optional_paths(config_dir: Option<PathBuf>, data_dir: Option<PathBuf>) -> Self {
        Self {
            apps: Arc::new(RwLock::new(HashMap::new())),
            restore_tracker: Arc::new(RwLock::new(AppRestoreTracker::default())),
            config_dir,
            data_dir,
            access_bootstrap_grants: Arc::new(RwLock::new(HashMap::new())),
            access_sessions: Arc::new(RwLock::new(HashMap::new())),
        }
    }

    pub async fn restore_snapshot(&self) -> AppRestoreSnapshot {
        self.restore_tracker.read().await.snapshot()
    }

    async fn begin_restore_batch(&self, total: usize) {
        self.restore_tracker.write().await.begin(total);
    }

    async fn finish_restore_item(&self, degraded: bool) {
        self.restore_tracker.write().await.finish_one(degraded);
    }

    pub fn spawn_restore_from_disk(
        &self,
        config_dir: PathBuf,
        data_dir: PathBuf,
        llm_env: HashMap<String, String>,
    ) {
        let registry = self.clone();
        crate::spawn_logged!("src/actions/app.rs:3719", async move {
            registry
                .restore_from_disk(&config_dir, &data_dir, &llm_env)
                .await;
        });
    }

    pub async fn reconcile_on_boot(&self) -> AppBootReconciliationReport {
        let mut report = AppBootReconciliationReport::default();
        let Some(data_dir) = self.data_dir.as_deref() else {
            return report;
        };
        let apps_dir = data_dir.join("apps");
        let Ok(mut entries) = tokio::fs::read_dir(&apps_dir).await else {
            return report;
        };

        while let Ok(Some(entry)) = entries.next_entry().await {
            let path = entry.path();
            if !path.is_dir() {
                continue;
            }

            let app_id = path
                .file_name()
                .and_then(|name| name.to_str())
                .unwrap_or("")
                .trim()
                .to_string();
            if app_id.is_empty() || app_id.eq_ignore_ascii_case("new") {
                continue;
            }

            let meta_path = path.join(".app_meta.json");
            let invalid_reason = match tokio::fs::read(&meta_path).await {
                Ok(bytes) => match serde_json::from_slice::<serde_json::Value>(&bytes) {
                    Ok(meta) if meta.is_object() => {
                        report.valid_app_ids.insert(app_id);
                        continue;
                    }
                    Ok(_) => Some("metadata is not a JSON object".to_string()),
                    Err(error) => Some(format!("metadata is corrupt: {}", error)),
                },
                Err(error) if error.kind() == std::io::ErrorKind::NotFound => {
                    Some("metadata file is missing".to_string())
                }
                Err(error) => Some(format!("metadata could not be read: {}", error)),
            };

            let quarantine_root = data_dir.join("app_quarantine");
            let quarantine_target = quarantine_root.join(format!(
                "{}-{}-{}",
                app_id,
                chrono::Utc::now().format("%Y%m%dT%H%M%SZ"),
                uuid::Uuid::new_v4().simple()
            ));
            let quarantine_result = async {
                tokio::fs::create_dir_all(&quarantine_root).await?;
                tokio::fs::rename(&path, &quarantine_target).await
            }
            .await;

            match quarantine_result {
                Ok(_) => tracing::warn!(
                    app_id = %app_id,
                    destination = %quarantine_target.display(),
                    reason = %invalid_reason.as_deref().unwrap_or("unknown"),
                    "app_quarantined"
                ),
                Err(error) => tracing::warn!(
                    app_id = %app_id,
                    path = %path.display(),
                    reason = %invalid_reason.as_deref().unwrap_or("unknown"),
                    error = %error,
                    "app_quarantine_failed"
                ),
            }

            self.purge_deleted_app_state(&app_id).await;
            report.quarantined_app_ids.insert(app_id);
        }

        report
    }

    fn secure_config_paths(&self) -> Option<(&Path, &Path)> {
        match (self.config_dir.as_deref(), self.data_dir.as_deref()) {
            (Some(config_dir), Some(data_dir)) => Some((config_dir, data_dir)),
            _ => None,
        }
    }

    async fn persist_access_key_secret(
        &self,
        app_id: &str,
        access_key: Option<&str>,
    ) -> Result<()> {
        let Some((config_dir, data_dir)) = self.secure_config_paths() else {
            return Ok(());
        };
        persist_access_key_sync(config_dir, data_dir, app_id, access_key)
    }

    fn load_persisted_access_key(&self, app_id: &str) -> Option<String> {
        let (config_dir, data_dir) = self.secure_config_paths()?;
        load_persisted_access_key_sync(config_dir, data_dir, app_id)
            .ok()
            .flatten()
    }

    async fn clear_access_tokens_for_app(&self, app_id: &str) {
        self.access_bootstrap_grants
            .write()
            .await
            .retain(|_, grant| grant.app_id != app_id);
        self.access_sessions
            .write()
            .await
            .retain(|_, session| session.app_id != app_id);
    }

    async fn access_guard_enabled_for_surface(&self, app_id: &str, public_surface: bool) -> bool {
        let app_handle = {
            let apps = self.apps.read().await;
            apps.get(app_id).cloned()
        };
        if let Some(app) = app_handle {
            let app = app.read().await;
            return if public_surface {
                app.access_guard_enabled || app.expose_public
            } else {
                app.access_guard_enabled
            };
        }
        false
    }

    async fn issue_access_bootstrap_grant(
        &self,
        app_id: &str,
        public_surface: bool,
    ) -> Option<String> {
        if !self
            .access_guard_enabled_for_surface(app_id, public_surface)
            .await
            || !self.is_enabled(app_id).await
        {
            return None;
        }
        let now = app_unix_now_ts();
        let mut grants = self.access_bootstrap_grants.write().await;
        grants.retain(|_, grant| grant.expires_at > now);
        while grants.len() >= APP_ACCESS_BOOTSTRAP_MAX_TOKENS {
            let Some(oldest_token) = grants
                .iter()
                .min_by_key(|(_, grant)| grant.expires_at)
                .map(|(token, _)| token.clone())
            else {
                break;
            };
            grants.remove(&oldest_token);
        }
        let token = format!("ag_{}", uuid::Uuid::new_v4().simple());
        grants.insert(
            token.clone(),
            AppAccessBootstrapGrant {
                app_id: app_id.to_string(),
                expires_at: now + APP_ACCESS_BOOTSTRAP_TTL_SECS,
            },
        );
        Some(token)
    }

    pub async fn issue_access_url(&self, app_id: &str) -> Option<String> {
        if self.access_guard_enabled(app_id).await {
            let grant = self.issue_access_bootstrap_grant(app_id, false).await?;
            Some(relative_app_bootstrap_url(app_id, &grant))
        } else if self.get_dir(app_id).await.is_some() {
            Some(relative_app_root_url(app_id))
        } else {
            None
        }
    }

    pub async fn issue_public_access_url(&self, app_id: &str) -> Option<String> {
        if self.public_access_guard_enabled(app_id).await {
            let grant = self.issue_access_bootstrap_grant(app_id, true).await?;
            Some(relative_app_bootstrap_url(app_id, &grant))
        } else if self.get_dir(app_id).await.is_some() {
            Some(relative_app_root_url(app_id))
        } else {
            None
        }
    }

    pub async fn consume_access_bootstrap_grant(&self, app_id: &str, token: &str) -> bool {
        let trimmed = token.trim();
        if trimmed.is_empty() {
            return false;
        }
        let now = app_unix_now_ts();
        let mut grants = self.access_bootstrap_grants.write().await;
        grants.retain(|_, grant| grant.expires_at > now);
        matches!(
            grants.remove(trimmed),
            Some(grant) if grant.app_id == app_id && grant.expires_at > now
        )
    }

    pub async fn create_access_session(&self, app_id: &str) -> Option<String> {
        if !self.access_guard_enabled_for_surface(app_id, true).await
            || !self.is_enabled(app_id).await
        {
            return None;
        }
        let now = app_unix_now_ts();
        let mut sessions = self.access_sessions.write().await;
        sessions.retain(|_, session| session.expires_at > now);
        while sessions.len() >= APP_ACCESS_SESSION_MAX_TOKENS {
            let Some(oldest_token) = sessions
                .iter()
                .min_by_key(|(_, session)| session.last_seen_at)
                .map(|(token, _)| token.clone())
            else {
                break;
            };
            sessions.remove(&oldest_token);
        }
        let token = format!("as_{}", uuid::Uuid::new_v4().simple());
        sessions.insert(
            token.clone(),
            AppAccessSession {
                app_id: app_id.to_string(),
                issued_at: now,
                expires_at: now + APP_ACCESS_SESSION_TTL_SECS,
                last_seen_at: now,
            },
        );
        Some(token)
    }

    pub async fn validate_access_session(&self, app_id: &str, token: &str) -> bool {
        let trimmed = token.trim();
        if trimmed.is_empty() {
            return false;
        }
        let now = app_unix_now_ts();
        let mut sessions = self.access_sessions.write().await;
        sessions.retain(|_, session| session.expires_at > now);
        if let Some(session) = sessions.get_mut(trimmed) {
            if session.app_id == app_id && session.expires_at > now {
                session.last_seen_at = now;
                return true;
            }
        }
        false
    }

    pub async fn access_key(&self, app_id: &str) -> Option<String> {
        let app_handle = {
            let apps = self.apps.read().await;
            apps.get(app_id).cloned()
        }?;
        let app = app_handle.read().await;
        if !app.access_guard_enabled && !app.expose_public {
            return Some(String::new());
        }
        Some(app.access_key.clone())
    }

    /// List all deployed apps
    pub async fn list(&self) -> Vec<serde_json::Value> {
        let app_entries: Vec<(String, Arc<RwLock<RunningApp>>)> = {
            let apps = self.apps.read().await;
            apps.iter()
                .map(|(id, app)| (id.clone(), Arc::clone(app)))
                .collect()
        };
        let mut result = Vec::new();
        for (id, app) in app_entries {
            let mut app = app.write().await;
            let mut mark_stopped = false;
            let running = if !app.enabled || app.restoring {
                false
            } else if app.is_static {
                true
            } else if let Some(container_id) = app.container_id.as_ref() {
                let up = is_container_running(container_id).await;
                if !up {
                    mark_stopped = true;
                }
                up
            } else if let Some(child) = app.process.as_mut() {
                match child.try_wait() {
                    Ok(None) => true,
                    Ok(Some(_)) => {
                        mark_stopped = true;
                        false
                    }
                    Err(_) => false,
                }
            } else {
                false
            };
            if mark_stopped {
                app.process = None;
                app.container_id = None;
                app.port = None;
            }
            let runtime_mode = if app.restoring {
                "restoring"
            } else if !app.enabled {
                "disabled"
            } else if app.is_static {
                "static"
            } else if app.container_id.is_some() {
                "isolated_container"
            } else if app.process.is_some() {
                "local_process_fallback"
            } else {
                "stopped"
            };
            let title = app.title.clone();
            let port = app.port;
            let is_static = app.is_static;
            let app_dir = app.app_dir.clone();
            let is_isolated_runtime = app.container_id.is_some();
            let created_at = app.created_at.to_rfc3339();
            let access_key = if app.access_guard_enabled || app.expose_public {
                app.access_key.clone()
            } else {
                String::new()
            };
            let access_guard_enabled = app.access_guard_enabled;
            let expose_public = app.expose_public;
            let enabled = app.enabled;
            let restoring = app.restoring;
            let restore_error = app.restore_error.clone();
            drop(app);
            let start_command = read_app_lifecycle_command(&app_dir, "entry_command")
                .await
                .unwrap_or_default();
            let install_command = read_app_lifecycle_command(&app_dir, "install_command")
                .await
                .unwrap_or_default();
            let stop_command = read_app_lifecycle_command(&app_dir, "stop_command")
                .await
                .unwrap_or_default();
            let has_start_command = !start_command.is_empty();
            let has_stop_command = !stop_command.is_empty();
            let access_url = self
                .issue_access_url(&id)
                .await
                .unwrap_or_else(|| relative_app_root_url(&id));
            let quality_report =
                read_optional_app_json(&app_dir.join(APP_QUALITY_REPORT_FILE)).await;
            let sub_goals = read_optional_app_json(&app_dir.join(APP_SUB_GOALS_FILE)).await;
            let app_meta = load_app_meta_json(&app_dir).await;
            let external_deployments = app_meta
                .get("external_deployments")
                .cloned()
                .filter(|value| value.is_object())
                .unwrap_or_else(|| serde_json::json!({}));
            let vercel_deployment = external_deployments
                .get("vercel")
                .cloned()
                .unwrap_or(serde_json::Value::Null);
            let quality_status = quality_report
                .as_ref()
                .and_then(|value| value.get("status"))
                .and_then(|value| value.as_str())
                .unwrap_or("unavailable")
                .to_string();
            result.push(serde_json::json!({
                "id": id,
                "title": title,
                "port": port,
                "is_static": is_static,
                "running": running,
                "runtime_mode": runtime_mode,
                "is_isolated_runtime": is_isolated_runtime,
                "entry_command": start_command.clone(),
                "start_command": start_command,
                "install_command": install_command,
                "stop_command": stop_command,
                "has_start_command": has_start_command,
                "has_stop_command": has_stop_command,
                "created_at": created_at,
                "url": relative_app_root_url(&id),
                "access_url": access_url,
                "access_key": access_key,
                "access_password": access_key,
                "access_guard_enabled": access_guard_enabled,
                "expose_public": expose_public,
                "enabled": enabled,
                "restoring": restoring,
                "restore_error": restore_error,
                "external_deployments": external_deployments,
                "vercel_deployment": vercel_deployment,
                "quality_report_status": quality_status,
                "quality_report": quality_report,
                "sub_goals": sub_goals,
                "restore_status": if restoring {
                    "restoring"
                } else if !enabled {
                    "disabled"
                } else if restore_error.is_some() {
                    "degraded"
                } else {
                    "ready"
                },
            }));
        }
        result.sort_by(|a, b| {
            let a_title = a
                .get("title")
                .and_then(|value| value.as_str())
                .unwrap_or("");
            let b_title = b
                .get("title")
                .and_then(|value| value.as_str())
                .unwrap_or("");
            a_title.cmp(b_title).then_with(|| {
                let a_id = a.get("id").and_then(|value| value.as_str()).unwrap_or("");
                let b_id = b.get("id").and_then(|value| value.as_str()).unwrap_or("");
                a_id.cmp(b_id)
            })
        });
        result
    }

    /// Get the port for a dynamic app (for reverse proxy)
    pub async fn get_port(&self, app_id: &str) -> Option<u16> {
        let app_handle = {
            let apps = self.apps.read().await;
            apps.get(app_id).cloned()
        }?;
        let port = {
            let app = app_handle.read().await;
            if !app.enabled || app.is_static || app.restoring {
                return None;
            }
            app.port
        }?;
        runtime_port_accepts_connections(port).await.then_some(port)
    }

    /// Get the app directory path
    pub async fn get_dir(&self, app_id: &str) -> Option<PathBuf> {
        let app_handle = {
            let apps = self.apps.read().await;
            apps.get(app_id).cloned()
        }?;
        let app = app_handle.read().await;
        Some(app.app_dir.clone())
    }

    /// Check if app is static
    pub async fn is_static(&self, app_id: &str) -> bool {
        let app_handle = {
            let apps = self.apps.read().await;
            apps.get(app_id).cloned()
        };
        if let Some(app) = app_handle {
            return app.read().await.is_static;
        }
        false
    }

    /// Check runtime liveness for a dynamic app and clear stale runtime handles.
    pub async fn runtime_is_alive(&self, app_id: &str) -> bool {
        let app_handle = {
            let apps = self.apps.read().await;
            apps.get(app_id).cloned()
        };
        let Some(app_handle) = app_handle else {
            return false;
        };
        let mut app = app_handle.write().await;
        if !app.enabled {
            return false;
        }
        if app.is_static {
            return true;
        }

        let mut alive = false;
        if let Some(container_id) = app.container_id.as_ref() {
            alive = is_container_running(container_id).await;
        } else if let Some(child) = app.process.as_mut() {
            alive = matches!(child.try_wait(), Ok(None));
        }

        if !alive {
            app.process = None;
            app.container_id = None;
            app.port = None;
        }
        alive
    }

    /// Register and start a dynamic app
    pub async fn register_dynamic(&self, id: String, registration: DynamicAppRegistration) {
        let now = chrono::Utc::now();
        let access_key_to_persist = if (registration.access_guard_enabled
            || registration.expose_public)
            && !registration.access_key.trim().is_empty()
        {
            Some(registration.access_key.clone())
        } else {
            None
        };
        let app = RunningApp {
            title: registration.title,
            port: Some(registration.port),
            process: registration.child,
            container_id: registration.container_id,
            app_dir: registration.app_dir,
            is_static: false,
            created_at: now,
            last_accessed: registration.last_accessed.unwrap_or(now),
            request_count: 0,
            access_key: registration.access_key,
            access_guard_enabled: registration.access_guard_enabled,
            expose_public: registration.expose_public,
            enabled: registration.enabled,
            restoring: false,
            restore_error: None,
        };
        self.apps
            .write()
            .await
            .insert(id.clone(), Arc::new(RwLock::new(app)));
        if let Err(error) = self
            .persist_access_key_secret(&id, access_key_to_persist.as_deref())
            .await
        {
            tracing::warn!(
                "Failed to persist encrypted app access key for '{}': {}",
                id,
                error
            );
        }
        self.clear_access_tokens_for_app(&id).await;
    }

    pub async fn register_stored(&self, id: String, registration: StoredAppRegistration) {
        let now = chrono::Utc::now();
        let access_key_to_persist = if (registration.access_guard_enabled
            || registration.expose_public)
            && !registration.access_key.trim().is_empty()
        {
            Some(registration.access_key.clone())
        } else {
            None
        };
        let app = RunningApp {
            title: registration.title,
            port: None,
            process: None,
            container_id: None,
            app_dir: registration.app_dir,
            is_static: registration.is_static,
            created_at: now,
            last_accessed: registration.last_accessed.unwrap_or(now),
            request_count: 0,
            access_key: registration.access_key,
            access_guard_enabled: registration.access_guard_enabled,
            expose_public: registration.expose_public,
            enabled: registration.enabled,
            restoring: false,
            restore_error: None,
        };
        self.apps
            .write()
            .await
            .insert(id.clone(), Arc::new(RwLock::new(app)));
        if let Err(error) = self
            .persist_access_key_secret(&id, access_key_to_persist.as_deref())
            .await
        {
            tracing::warn!(
                "Failed to persist encrypted app access key for '{}': {}",
                id,
                error
            );
        }
        self.clear_access_tokens_for_app(&id).await;
    }

    pub async fn reserve_restoring_dynamic(
        &self,
        id: String,
        title: String,
        app_dir: PathBuf,
        access_key: String,
        access_guard_enabled: bool,
        expose_public: bool,
    ) -> Option<u16> {
        let now = chrono::Utc::now();
        let mut apps = self.apps.write().await;
        let used_ports: Vec<u16> = apps
            .values()
            .filter_map(|entry| entry.try_read().ok().and_then(|app| app.port))
            .collect();

        for port in PORT_RANGE_START..PORT_RANGE_END {
            if used_ports.contains(&port) {
                continue;
            }
            if std::net::TcpListener::bind(("127.0.0.1", port)).is_err() {
                continue;
            }

            let app = RunningApp {
                title,
                port: Some(port),
                process: None,
                container_id: None,
                app_dir,
                is_static: false,
                created_at: now,
                last_accessed: now,
                request_count: 0,
                access_key,
                access_guard_enabled,
                expose_public,
                enabled: true,
                restoring: true,
                restore_error: None,
            };
            apps.insert(id, Arc::new(RwLock::new(app)));
            return Some(port);
        }
        None
    }

    pub async fn mark_restore_error(&self, app_id: &str, error: impl Into<String>) {
        let app_handle = {
            let apps = self.apps.read().await;
            apps.get(app_id).cloned()
        };
        if let Some(app) = app_handle {
            let mut app = app.write().await;
            app.restoring = false;
            app.restore_error = Some(error.into());
        }
    }

    /// Verify access key for an app
    pub async fn verify_key(&self, app_id: &str, key: &str) -> bool {
        let app_handle = {
            let apps = self.apps.read().await;
            apps.get(app_id).cloned()
        };
        if let Some(app) = app_handle {
            let app = app.read().await;
            if !app.enabled {
                return false;
            }
            if !app.access_guard_enabled && !app.expose_public {
                return true;
            }
            return crate::security::constant_time_eq(
                app.access_key.trim().as_bytes(),
                key.trim().as_bytes(),
            );
        }
        false
    }

    /// Whether local app serving requires an access key guard.
    pub async fn access_guard_enabled(&self, app_id: &str) -> bool {
        let app_handle = {
            let apps = self.apps.read().await;
            apps.get(app_id).cloned()
        };
        if let Some(app) = app_handle {
            return app.read().await.access_guard_enabled;
        }
        false
    }

    /// Whether public app serving requires an access key guard.
    pub async fn public_access_guard_enabled(&self, app_id: &str) -> bool {
        self.access_guard_enabled_for_surface(app_id, true).await
    }

    /// Whether this app is explicitly exposed through the public app surface.
    pub async fn expose_public(&self, app_id: &str) -> bool {
        let app_handle = {
            let apps = self.apps.read().await;
            apps.get(app_id).cloned()
        };
        if let Some(app) = app_handle {
            return app.read().await.expose_public;
        }
        false
    }

    pub async fn is_enabled(&self, app_id: &str) -> bool {
        let app_handle = {
            let apps = self.apps.read().await;
            apps.get(app_id).cloned()
        };
        if let Some(app) = app_handle {
            return app.read().await.enabled;
        }
        false
    }

    pub async fn set_enabled(&self, app_id: &str, enabled: bool) -> Result<()> {
        let app_handle = {
            let apps = self.apps.read().await;
            apps.get(app_id).cloned()
        }
        .ok_or_else(|| anyhow::anyhow!("App not found"))?;

        let app_dir = {
            let mut app = app_handle.write().await;
            app.enabled = enabled;
            if !enabled {
                app.restoring = false;
                app.port = None;
            }
            app.app_dir.clone()
        };

        persist_app_enabled_meta(&app_dir, enabled).await?;
        if !enabled {
            self.clear_access_tokens_for_app(app_id).await;
        }
        Ok(())
    }

    /// Toggle access guard for an app and optionally rotate its access key.
    pub async fn set_access_guard(
        &self,
        app_id: &str,
        enabled: bool,
        access_secret: Option<&str>,
        regenerate_key: bool,
    ) -> Result<String> {
        let app_handle = {
            let apps = self.apps.read().await;
            apps.get(app_id).cloned()
        }
        .ok_or_else(|| anyhow::anyhow!("App not found"))?;

        let (app_dir, access_key) = {
            let mut app = app_handle.write().await;
            let explicit_secret = access_secret
                .map(str::trim)
                .filter(|value| !value.is_empty());
            let key_required = enabled || app.expose_public;
            let next_key = if key_required {
                if let Some(secret) = explicit_secret {
                    secret.to_string()
                } else if regenerate_key {
                    generate_access_key()
                } else if !app.access_key.trim().is_empty() {
                    app.access_key.clone()
                } else if app.expose_public {
                    generate_access_key()
                } else {
                    anyhow::bail!("Access password required");
                }
            } else {
                String::new()
            };
            app.access_guard_enabled = enabled;
            app.access_key = next_key.clone();
            (app.app_dir.clone(), next_key)
        };

        persist_app_access_guard_meta(&app_dir, enabled).await?;
        let persist_secret = enabled || self.public_access_guard_enabled(app_id).await;
        self.persist_access_key_secret(app_id, persist_secret.then_some(access_key.as_str()))
            .await?;
        self.clear_access_tokens_for_app(app_id).await;
        Ok(access_key)
    }

    /// Record an access (called when an app is served via HTTP)
    pub async fn touch(&self, app_id: &str) {
        let app_handle = {
            let apps = self.apps.read().await;
            apps.get(app_id).cloned()
        };
        if let Some(app) = app_handle {
            let persist_target = {
                let mut app = app.write().await;
                let previous = app.last_accessed;
                let now = chrono::Utc::now();
                app.last_accessed = now;
                app.request_count += 1;
                ((now - previous).num_seconds() >= 30).then(|| (app.app_dir.clone(), now))
            };
            if let Some((app_dir, last_accessed)) = persist_target {
                crate::spawn_logged!("src/actions/app.rs:4307", async move {
                    if let Err(error) =
                        persist_app_last_accessed_meta(&app_dir, last_accessed).await
                    {
                        tracing::warn!(
                            "Failed to persist app last_accessed for '{}': {}",
                            app_dir.display(),
                            error
                        );
                    }
                });
            }
        }
    }

    /// Get a health snapshot of all apps for Pulse, resetting request counters
    pub async fn pulse_snapshot(&self) -> Vec<AppHealthSnapshot> {
        let app_entries: Vec<(String, Arc<RwLock<RunningApp>>)> = {
            let apps = self.apps.read().await;
            apps.iter()
                .map(|(id, app)| (id.clone(), Arc::clone(app)))
                .collect()
        };
        let mut snapshots = Vec::new();
        for (id, app) in app_entries {
            let mut app = app.write().await;
            let mut mark_stopped = false;
            let process_alive = if !app.enabled {
                false
            } else if app.is_static {
                true
            } else if let Some(container_id) = app.container_id.as_ref() {
                let up = is_container_running(container_id).await;
                if !up {
                    mark_stopped = true;
                }
                up
            } else if let Some(child) = app.process.as_mut() {
                match child.try_wait() {
                    Ok(None) => true,
                    Ok(Some(_)) => {
                        mark_stopped = true;
                        false
                    }
                    Err(_) => false,
                }
            } else {
                false
            };
            if mark_stopped {
                app.process = None;
                app.container_id = None;
                app.port = None;
            }
            snapshots.push(AppHealthSnapshot {
                id,
                title: app.title.clone(),
                is_static: app.is_static,
                process_alive,
                requests_since_last_check: app.request_count,
                last_accessed: app.last_accessed,
            });
            app.request_count = 0; // Reset counter after snapshot
        }
        snapshots
    }

    /// Get apps that haven't been accessed in the given duration
    pub async fn get_unused_apps(
        &self,
        idle_hours: i64,
    ) -> Vec<(String, String, chrono::DateTime<chrono::Utc>)> {
        let cutoff = chrono::Utc::now() - chrono::Duration::hours(idle_hours);
        let app_entries: Vec<(String, Arc<RwLock<RunningApp>>)> = {
            let apps = self.apps.read().await;
            apps.iter()
                .map(|(id, app)| (id.clone(), Arc::clone(app)))
                .collect()
        };
        let mut unused = Vec::new();
        for (id, app) in app_entries {
            let app = app.read().await;
            if !app.enabled {
                continue;
            }
            if app.last_accessed < cutoff {
                unused.push((id, app.title.clone(), app.last_accessed));
            }
        }
        unused
    }

    /// Stop runtime process for a dynamic app but keep app metadata registered.
    pub async fn stop_runtime(&self, app_id: &str) -> Result<()> {
        let app_handle = {
            let apps = self.apps.read().await;
            apps.get(app_id).cloned()
        };
        let Some(app) = app_handle else {
            return Ok(());
        };
        let mut app = app.write().await;
        if app.is_static {
            return Ok(());
        }
        let app_dir = app.app_dir.clone();
        let mut child = app.process.take();
        let container_id = app.container_id.take();
        app.port = None;
        drop(app);

        if let Some(command) = read_app_lifecycle_command(&app_dir, "stop_command").await {
            run_app_stop_command(app_id, &app_dir, container_id.as_deref(), &command).await;
        }
        if let Some(ref cid) = container_id {
            stop_container(cid).await?;
            tracing::info!("Stopped app container: {} ({})", app_id, cid);
        }
        if let Some(ref mut c) = child {
            stop_child_process(c, app_id).await?;
            tracing::info!("Stopped app process: {}", app_id);
        }
        Ok(())
    }

    /// Stop and remove an app
    pub async fn stop(&self, app_id: &str) -> Result<()> {
        let app_handle = {
            let apps = self.apps.read().await;
            apps.get(app_id).cloned()
        };
        if let Some(app) = app_handle {
            let mut app = app.write().await;
            let is_static = app.is_static;
            let app_dir = app.app_dir.clone();
            let mut child = app.process.take();
            let container_id = app.container_id.take();
            app.port = None;
            drop(app);

            if !is_static {
                if let Some(command) = read_app_lifecycle_command(&app_dir, "stop_command").await {
                    run_app_stop_command(app_id, &app_dir, container_id.as_deref(), &command).await;
                }
            }
            if let Some(ref cid) = container_id {
                stop_container(cid).await?;
                tracing::info!("Stopped app container: {} ({})", app_id, cid);
            }
            if let Some(ref mut c) = child {
                stop_child_process(c, app_id).await?;
                tracing::info!("Stopped app process: {}", app_id);
            }
            self.apps.write().await.remove(app_id);
        }
        Ok(())
    }

    /// Remove all registry/auth state for an app that the user has explicitly deleted
    /// or that can no longer be served safely.
    pub async fn purge_deleted_app_state(&self, app_id: &str) {
        if let Err(error) = self.stop(app_id).await {
            tracing::warn!(
                "Failed to stop app '{}' while purging deleted app state: {}",
                app_id,
                error
            );
        }
        self.apps.write().await.remove(app_id);
        self.clear_access_tokens_for_app(app_id).await;
        if let Err(error) = self.persist_access_key_secret(app_id, None).await {
            tracing::warn!(
                "Failed to clear persisted access key for deleted app '{}': {}",
                app_id,
                error
            );
        }
    }

    /// Find an available port in the range
    pub async fn find_available_port(&self) -> Option<u16> {
        let apps = self.apps.read().await;
        let used_ports: Vec<u16> = apps
            .values()
            .filter_map(|a| {
                // We can't await inside filter_map in a sync context, so use try_read
                if let Ok(app) = a.try_read() {
                    app.port
                } else {
                    None
                }
            })
            .collect();

        for port in PORT_RANGE_START..PORT_RANGE_END {
            if !used_ports.contains(&port) {
                // Quick check if port is actually free
                if std::net::TcpListener::bind(("127.0.0.1", port)).is_ok() {
                    return Some(port);
                }
            }
        }
        None
    }

    async fn restore_dynamic_candidate_from_disk(
        &self,
        candidate: RestoreAppCandidate,
        port: u16,
        config_dir: &Path,
        data_dir: &Path,
        llm_env: &HashMap<String, String>,
    ) -> bool {
        let RestoreAppCandidate {
            id,
            title,
            app_dir,
            entry_command,
            install_command,
            runtime_image,
            runtime_preference,
            required_inputs,
            config_values,
            access_guard_enabled,
            access_key,
            expose_public,
            enabled: _enabled,
            last_accessed,
        } = candidate;
        let Some(entry_cmd) = entry_command else {
            return false;
        };

        let (resolved_env, missing_sensitive, missing_config) = match resolve_required_env_values(
            config_dir,
            data_dir,
            &required_inputs,
            llm_env,
            &config_values,
        )
        .await
        {
            Ok(out) => out,
            Err(e) => {
                let detail = format!("Restore failed while resolving config: {}", e);
                tracing::warn!("{} (app={})", detail, id);
                self.register_stored(
                    id.clone(),
                    StoredAppRegistration {
                        title,
                        app_dir,
                        is_static: true,
                        access_key: access_key.clone(),
                        access_guard_enabled,
                        expose_public,
                        enabled: true,
                        last_accessed,
                    },
                )
                .await;
                self.mark_restore_error(&id, detail).await;
                return true;
            }
        };

        if !missing_sensitive.is_empty() || !missing_config.is_empty() {
            let detail = format!(
                "Restore needs inputs before runtime can start (missing_sensitive={:?}, missing_config={:?})",
                missing_sensitive, missing_config
            );
            tracing::warn!("{} (app={})", detail, id);
            self.register_stored(
                id.clone(),
                StoredAppRegistration {
                    title,
                    app_dir,
                    is_static: true,
                    access_key: access_key.clone(),
                    access_guard_enabled,
                    expose_public,
                    enabled: true,
                    last_accessed,
                },
            )
            .await;
            self.mark_restore_error(&id, detail).await;
            return true;
        }

        match launch_dynamic_runtime(DynamicRuntimeLaunch {
            app_id: &id,
            app_dir: &app_dir,
            entry_command: &entry_cmd,
            install_command: install_command.as_deref(),
            port,
            extra_env: &resolved_env,
            runtime_image: runtime_image.as_deref(),
            runtime_preference,
            stream_tx: None,
        })
        .await
        {
            Ok(runtime_handle) => {
                let (container_id, child) = match runtime_handle {
                    DynamicRuntimeHandle::Container(container_id) => (Some(container_id), None),
                    DynamicRuntimeHandle::Process(child) => (None, Some(*child)),
                };
                self.register_dynamic(
                    id.clone(),
                    DynamicAppRegistration {
                        title: title.clone(),
                        app_dir: app_dir.clone(),
                        child,
                        container_id,
                        port,
                        access_key: access_key.clone(),
                        access_guard_enabled,
                        expose_public,
                        enabled: true,
                        last_accessed,
                    },
                )
                .await;
                let no_stream = None;
                if let Err(e) = wait_for_dynamic_runtime_ready(self, &id, port, &no_stream).await {
                    let detail = format!("Runtime did not become ready: {}", e);
                    tracing::warn!("{} (app={})", detail, id);
                    let _ = self.stop_runtime(&id).await;
                    self.register_stored(
                        id.clone(),
                        StoredAppRegistration {
                            title,
                            app_dir,
                            is_static: true,
                            access_key: access_key.clone(),
                            access_guard_enabled,
                            expose_public,
                            enabled: true,
                            last_accessed,
                        },
                    )
                    .await;
                    self.mark_restore_error(&id, detail).await;
                    return true;
                }
                tracing::info!("Restarted dynamic app: {}", id);
                false
            }
            Err(e) => {
                let detail = format!("Runtime launch failed: {}", e);
                tracing::warn!("{} (app={})", detail, id);
                self.register_stored(
                    id.clone(),
                    StoredAppRegistration {
                        title,
                        app_dir,
                        is_static: true,
                        access_key: access_key.clone(),
                        access_guard_enabled,
                        expose_public,
                        enabled: true,
                        last_accessed,
                    },
                )
                .await;
                self.mark_restore_error(&id, detail).await;
                true
            }
        }
    }

    /// Restore apps from disk on startup. Static apps are served immediately.
    /// Dynamic apps with entry_command are restarted in the background.
    pub async fn restore_from_disk(
        &self,
        config_dir: &Path,
        data_dir: &Path,
        llm_env: &HashMap<String, String>,
    ) {
        let apps_dir = data_dir.join("apps");
        if !apps_dir.exists() {
            return;
        }

        let mut candidates = Vec::new();
        if let Ok(mut entries) = tokio::fs::read_dir(&apps_dir).await {
            while let Ok(Some(entry)) = entries.next_entry().await {
                let path = entry.path();
                if !path.is_dir() {
                    continue;
                }

                let id = path
                    .file_name()
                    .and_then(|n| n.to_str())
                    .unwrap_or("")
                    .to_string();
                if id.is_empty() || id.eq_ignore_ascii_case("new") {
                    continue;
                }

                let meta_path = path.join(".app_meta.json");
                let mut meta: Option<serde_json::Value> = match tokio::fs::read(&meta_path).await {
                    Ok(bytes) => match serde_json::from_slice::<serde_json::Value>(&bytes) {
                        Ok(value) if value.is_object() => Some(value),
                        Ok(_) => {
                            tracing::warn!(
                                app_id = %id,
                                path = %meta_path.display(),
                                "Skipping app restore because metadata is not a JSON object"
                            );
                            continue;
                        }
                        Err(error) => {
                            tracing::warn!(
                                app_id = %id,
                                path = %meta_path.display(),
                                error = %error,
                                "Skipping app restore because metadata is corrupt"
                            );
                            continue;
                        }
                    },
                    Err(error) if error.kind() == std::io::ErrorKind::NotFound => {
                        tracing::warn!(
                            app_id = %id,
                            path = %path.display(),
                            "Skipping app restore because metadata is missing"
                        );
                        continue;
                    }
                    Err(error) => {
                        tracing::warn!(
                            app_id = %id,
                            path = %meta_path.display(),
                            error = %error,
                            "Skipping app restore because metadata could not be read"
                        );
                        continue;
                    }
                };

                let stored_entry_command = meta
                    .as_ref()
                    .and_then(|m| app_meta_lifecycle_command(m, "entry_command"));
                if stored_entry_command.is_none() {
                    if let Ok(effective_files) =
                        load_existing_managed_app_text_files(&path, &meta).await
                    {
                        if let Some(inferred) = infer_generated_bundle_lifecycle(&effective_files) {
                            let mut updated = meta
                                .clone()
                                .filter(|value| value.is_object())
                                .unwrap_or_else(|| serde_json::json!({}));
                            set_generated_app_lifecycle_meta(&mut updated, &inferred);
                            if let Err(error) = tokio::fs::write(
                                &meta_path,
                                serde_json::to_string_pretty(&updated)
                                    .unwrap_or_else(|_| "{}".to_string()),
                            )
                            .await
                            {
                                tracing::warn!(
                                    app_id = %id,
                                    error = %error,
                                    "Failed to persist inferred app lifecycle metadata during restore"
                                );
                            } else {
                                tracing::info!(
                                    app_id = %id,
                                    "Inferred dynamic app lifecycle from stored generated bundle"
                                );
                                meta = Some(updated);
                            }
                        }
                    }
                }

                let title = meta
                    .as_ref()
                    .and_then(|m| m.get("title").and_then(|t| t.as_str()))
                    .unwrap_or(&id)
                    .to_string();
                let entry_command = meta
                    .as_ref()
                    .and_then(|m| app_meta_lifecycle_command(m, "entry_command"));
                let install_command = meta
                    .as_ref()
                    .and_then(|m| app_meta_lifecycle_command(m, "install_command"));
                let runtime_image = meta
                    .as_ref()
                    .and_then(|m| m.get("runtime_image").and_then(|c| c.as_str()))
                    .map(|s| s.to_string());
                let runtime_preference = runtime_preference_from_opt(
                    meta.as_ref()
                        .and_then(|m| m.get("runtime_preference").and_then(|c| c.as_str())),
                );
                let required_inputs = meta.as_ref().map(parse_required_inputs).unwrap_or_default();
                let config_values: HashMap<String, String> = meta
                    .as_ref()
                    .and_then(|m| m.get("config_values").and_then(|v| v.as_object()))
                    .map(|obj| {
                        obj.iter()
                            .filter_map(|(k, v)| {
                                let value = match v {
                                    serde_json::Value::String(s) => s.clone(),
                                    serde_json::Value::Bool(b) => b.to_string(),
                                    serde_json::Value::Number(n) => n.to_string(),
                                    _ => return None,
                                };
                                Some((k.clone(), value))
                            })
                            .collect()
                    })
                    .unwrap_or_default();
                let persisted_access_key = self
                    .load_persisted_access_key(&id)
                    .filter(|value| !value.trim().is_empty());
                let expose_public = meta
                    .as_ref()
                    .and_then(|m| m.get("expose_public").and_then(|v| v.as_bool()))
                    .unwrap_or(false);
                let access_guard_enabled = meta
                    .as_ref()
                    .and_then(|m| m.get("access_guard_enabled").and_then(|v| v.as_bool()))
                    .unwrap_or_else(|| persisted_access_key.is_some());
                let access_key = if access_guard_enabled || expose_public {
                    persisted_access_key.unwrap_or_else(generate_access_key)
                } else {
                    String::new()
                };
                let enabled = meta
                    .as_ref()
                    .and_then(|m| m.get("enabled").and_then(|v| v.as_bool()))
                    .unwrap_or(true);
                let last_accessed = parse_app_meta_datetime(&meta, "last_accessed");

                candidates.push(RestoreAppCandidate {
                    id,
                    title,
                    app_dir: path,
                    entry_command,
                    install_command,
                    runtime_image,
                    runtime_preference,
                    required_inputs,
                    config_values,
                    access_guard_enabled,
                    access_key,
                    expose_public,
                    enabled,
                    last_accessed,
                });
            }
        }

        self.begin_restore_batch(candidates.len()).await;
        if candidates.is_empty() {
            return;
        }

        if control_plane_catalog_mode() {
            for candidate in candidates {
                if !candidate.enabled {
                    self.register_stored(
                        candidate.id.clone(),
                        StoredAppRegistration {
                            title: candidate.title.clone(),
                            app_dir: candidate.app_dir.clone(),
                            is_static: candidate.entry_command.is_none(),
                            access_key: candidate.access_key.clone(),
                            access_guard_enabled: candidate.access_guard_enabled,
                            expose_public: candidate.expose_public,
                            enabled: false,
                            last_accessed: candidate.last_accessed,
                        },
                    )
                    .await;
                    tracing::info!("Restored disabled app metadata: {}", candidate.id);
                } else {
                    self.register_stored(
                        candidate.id.clone(),
                        StoredAppRegistration {
                            title: candidate.title.clone(),
                            app_dir: candidate.app_dir.clone(),
                            is_static: candidate.entry_command.is_none(),
                            access_key: candidate.access_key.clone(),
                            access_guard_enabled: candidate.access_guard_enabled,
                            expose_public: candidate.expose_public,
                            enabled: true,
                            last_accessed: candidate.last_accessed,
                        },
                    )
                    .await;
                    tracing::info!(
                        "Restored app catalog entry without local runtime: {}",
                        candidate.id
                    );
                }
                self.finish_restore_item(false).await;
            }
            return;
        }

        let config_dir = config_dir.to_path_buf();
        let data_dir = data_dir.to_path_buf();
        let llm_env = Arc::new(llm_env.clone());
        let semaphore = Arc::new(tokio::sync::Semaphore::new(startup_restore_parallelism()));
        let mut join_set = tokio::task::JoinSet::new();

        for candidate in candidates {
            if !candidate.enabled {
                self.register_stored(
                    candidate.id.clone(),
                    StoredAppRegistration {
                        title: candidate.title.clone(),
                        app_dir: candidate.app_dir.clone(),
                        is_static: candidate.entry_command.is_none(),
                        access_key: candidate.access_key.clone(),
                        access_guard_enabled: candidate.access_guard_enabled,
                        expose_public: candidate.expose_public,
                        enabled: false,
                        last_accessed: candidate.last_accessed,
                    },
                )
                .await;
                self.finish_restore_item(false).await;
                tracing::info!("Restored disabled app metadata: {}", candidate.id);
                continue;
            }

            if candidate.entry_command.is_none() {
                self.register_stored(
                    candidate.id.clone(),
                    StoredAppRegistration {
                        title: candidate.title.clone(),
                        app_dir: candidate.app_dir.clone(),
                        is_static: true,
                        access_key: candidate.access_key.clone(),
                        access_guard_enabled: candidate.access_guard_enabled,
                        expose_public: candidate.expose_public,
                        enabled: true,
                        last_accessed: candidate.last_accessed,
                    },
                )
                .await;
                self.finish_restore_item(false).await;
                tracing::info!("Restored static app: {}", candidate.id);
                continue;
            }

            let Some(port) = self
                .reserve_restoring_dynamic(
                    candidate.id.clone(),
                    candidate.title.clone(),
                    candidate.app_dir.clone(),
                    candidate.access_key.clone(),
                    candidate.access_guard_enabled,
                    candidate.expose_public,
                )
                .await
            else {
                let detail = "No available runtime port for background restore.".to_string();
                tracing::warn!("{} (app={})", detail, candidate.id);
                self.register_stored(
                    candidate.id.clone(),
                    StoredAppRegistration {
                        title: candidate.title.clone(),
                        app_dir: candidate.app_dir.clone(),
                        is_static: true,
                        access_key: candidate.access_key.clone(),
                        access_guard_enabled: candidate.access_guard_enabled,
                        expose_public: candidate.expose_public,
                        enabled: true,
                        last_accessed: candidate.last_accessed,
                    },
                )
                .await;
                self.mark_restore_error(&candidate.id, detail).await;
                self.finish_restore_item(true).await;
                continue;
            };

            tracing::info!(
                "Queued background restore for app '{}' (id={}) on port {}",
                candidate.title,
                candidate.id,
                port
            );
            let registry = self.clone();
            let config_dir = config_dir.clone();
            let data_dir = data_dir.clone();
            let llm_env = Arc::clone(&llm_env);
            let semaphore = Arc::clone(&semaphore);
            join_set.spawn(async move {
                let _permit = semaphore
                    .acquire_owned()
                    .await
                    .expect("restore semaphore should stay alive");
                registry
                    .restore_dynamic_candidate_from_disk(
                        candidate,
                        port,
                        &config_dir,
                        &data_dir,
                        llm_env.as_ref(),
                    )
                    .await
            });
        }

        while let Some(result) = join_set.join_next().await {
            match result {
                Ok(degraded) => self.finish_restore_item(degraded).await,
                Err(err) => {
                    tracing::warn!("Background app restore task failed: {}", err);
                    self.finish_restore_item(true).await;
                }
            }
        }
    }
}

pub async fn runtime_port_accepts_connections(port: u16) -> bool {
    matches!(
        tokio::time::timeout(
            std::time::Duration::from_millis(800),
            tokio::net::TcpStream::connect(("127.0.0.1", port)),
        )
        .await,
        Ok(Ok(_))
    )
}

pub async fn wait_for_runtime_port_open(
    app_id: &str,
    port: u16,
    stream_tx: &Option<Sender<StreamEvent>>,
) -> Result<()> {
    let deadline = tokio::time::Instant::now()
        + std::time::Duration::from_secs(dynamic_runtime_ready_timeout_secs());
    let mut last_progress_at = tokio::time::Instant::now()
        - std::time::Duration::from_secs(DYNAMIC_RUNTIME_PROGRESS_INTERVAL_SECS);

    loop {
        if runtime_port_accepts_connections(port).await {
            return Ok(());
        }

        if tokio::time::Instant::now() >= deadline {
            anyhow::bail!(
                "App {} did not accept connections on port {} within {}s",
                app_id,
                port,
                dynamic_runtime_ready_timeout_secs()
            );
        }

        if last_progress_at.elapsed()
            >= std::time::Duration::from_secs(DYNAMIC_RUNTIME_PROGRESS_INTERVAL_SECS)
        {
            emit_phase_progress(
                stream_tx,
                AppDeployProgressPhase::StartingRuntime,
                format!("Waiting for server readiness on port {}", port),
            )
            .await;
            last_progress_at = tokio::time::Instant::now();
        }

        tokio::time::sleep(std::time::Duration::from_millis(
            DYNAMIC_RUNTIME_READY_POLL_MS,
        ))
        .await;
    }
}

async fn wait_for_dynamic_runtime_ready(
    registry: &AppRegistry,
    app_id: &str,
    port: u16,
    stream_tx: &Option<Sender<StreamEvent>>,
) -> Result<()> {
    let deadline = tokio::time::Instant::now()
        + std::time::Duration::from_secs(dynamic_runtime_ready_timeout_secs());
    loop {
        if !registry.runtime_is_alive(app_id).await {
            anyhow::bail!("App {} stopped before it opened port {}", app_id, port);
        }
        if runtime_port_accepts_connections(port).await {
            return Ok(());
        }
        if tokio::time::Instant::now() >= deadline {
            anyhow::bail!(
                "App {} did not accept connections on port {} within {}s",
                app_id,
                port,
                dynamic_runtime_ready_timeout_secs()
            );
        }
        emit_phase_progress(
            stream_tx,
            AppDeployProgressPhase::StartingRuntime,
            format!("Waiting for server readiness on port {}", port),
        )
        .await;
        tokio::time::sleep(std::time::Duration::from_millis(
            DYNAMIC_RUNTIME_READY_POLL_MS,
        ))
        .await;
    }
}

async fn maybe_publish_external_deployment(
    config_dir: &Path,
    data_dir: &Path,
    arguments: &serde_json::Value,
    app_id: &str,
    app_dir: &Path,
    title: &str,
    stream_tx: &Option<Sender<StreamEvent>>,
) -> Option<serde_json::Value> {
    let options = crate::actions::vercel::ExternalDeployOptions::from_arguments(arguments);
    if options.target == crate::actions::vercel::ExternalDeployTarget::Local {
        return None;
    }
    emit_phase_progress(
        stream_tx,
        AppDeployProgressPhase::Deploying,
        format!("Publishing '{}' to {}", title, options.target.as_str()),
    )
    .await;
    let meta = crate::actions::vercel::load_app_meta(app_dir).await;
    match crate::actions::vercel::publish_app_to_external_target(
        config_dir, data_dir, app_id, app_dir, &meta, title, &options,
    )
    .await
    {
        Ok(value) => value,
        Err(error) => Some(serde_json::json!({
            "provider": "vercel",
            "deploy_target": options.target.as_str(),
            "status": "error",
            "app_id": app_id,
            "title": title,
            "message": error.to_string(),
            "updated_at": chrono::Utc::now().to_rfc3339(),
        })),
    }
}

fn attach_external_deployment_result(
    response: &mut serde_json::Value,
    external: Option<serde_json::Value>,
) {
    let Some(external) = external else {
        return;
    };
    if let Some(obj) = response.as_object_mut() {
        obj.insert("external_deployment".to_string(), external.clone());
        let deployments = obj
            .entry("external_deployments")
            .or_insert_with(|| serde_json::json!({}));
        if !deployments.is_object() {
            *deployments = serde_json::json!({});
        }
        if let Some(map) = deployments.as_object_mut() {
            map.insert("vercel".to_string(), external.clone());
        }
        if let Some(url) = external.get("url").and_then(|value| value.as_str()) {
            if !url.trim().is_empty() {
                obj.insert(
                    "vercel_url".to_string(),
                    serde_json::Value::String(url.to_string()),
                );
            }
        }
    }
}

/// Deploy an app from agent-generated files.
///
/// Arguments (JSON):
/// - `files`: object mapping filename -> content, or `source_dir` +
///   `source_paths` pointing at files previously staged with file_write
/// - `title`: app name (optional, default: "App")
/// - `entry_command`: command to start the server (optional; if omitted, static)
/// - `port`: port the server listens on (optional; auto-assigned if dynamic)
/// - `install_command`: command to install deps (optional, e.g. "pip install -r requirements.txt")
///
/// Returns JSON with the app URL.
pub async fn app_deploy(
    config_dir: &Path,
    data_dir: &Path,
    arguments: &serde_json::Value,
    registry: &AppRegistry,
    llm_env: &HashMap<String, String>,
    stream_tx: Option<Sender<StreamEvent>>,
) -> Result<String> {
    let deploy_started = std::time::Instant::now();
    tracing::debug!(
        target: "agentark.turn_timing",
        stage = "app_deploy_start",
        repo_bundle = should_deploy_repo_bundle(arguments),
        argument_keys = arguments.as_object().map(|obj| obj.len()).unwrap_or(0),
        "app deploy timing start"
    );
    if should_deploy_repo_bundle(arguments) {
        return deploy_repo_bundle(
            config_dir, data_dir, arguments, registry, llm_env, stream_tx,
        )
        .await;
    }

    let stage_started = std::time::Instant::now();
    let plan = app_deploy_apply_plan_from_arguments(data_dir, arguments).await?;
    tracing::debug!(
        target: "agentark.turn_timing",
        stage = "app_deploy_parse_plan",
        duration_ms = stage_started.elapsed().as_millis() as u64,
        file_writes = plan.file_writes.len(),
        file_patches = plan.file_patches.len(),
        delete_paths = plan.delete_paths.len(),
        mode = %plan.mode.as_str(),
        "app deploy timing stage"
    );
    let file_count = plan
        .file_writes
        .len()
        .saturating_add(plan.file_patches.len())
        .saturating_add(plan.delete_paths.len());
    if file_count == 0 {
        anyhow::bail!(
            "app_deploy requires at least one file write, file patch, or delete_paths entry"
        );
    }
    let allow_duplicate = app_deploy_allows_duplicate(arguments);

    let requested_app_id = arguments
        .get("app_id")
        .and_then(|v| v.as_str())
        .map(str::trim)
        .filter(|value| !value.is_empty())
        .map(|value| value.to_string());
    let stage_started = std::time::Instant::now();
    let existing_target = if let Some(app_id) = requested_app_id.as_deref() {
        let existing_app = registry
            .list()
            .await
            .into_iter()
            .find(|app| app.get("id").and_then(|v| v.as_str()) == Some(app_id));
        let app_dir = registry
            .get_dir(app_id)
            .await
            .unwrap_or_else(|| data_dir.join("apps").join(app_id));
        if existing_app.is_none() && !app_dir.exists() {
            anyhow::bail!("No deployed app found for app_id '{}'", app_id);
        }
        let meta = tokio::fs::read(app_dir.join(".app_meta.json"))
            .await
            .ok()
            .and_then(|bytes| serde_json::from_slice::<serde_json::Value>(&bytes).ok())
            .filter(|value| value.is_object());
        let title = existing_app
            .as_ref()
            .and_then(|app| app.get("title").and_then(|v| v.as_str()))
            .or_else(|| {
                meta.as_ref()
                    .and_then(|value| value.get("title").and_then(|v| v.as_str()))
            })
            .unwrap_or("App")
            .to_string();
        let access_key = registry
            .access_key(app_id)
            .await
            .filter(|value| !value.trim().is_empty());
        let access_guard_enabled = existing_app
            .as_ref()
            .and_then(|app| app.get("access_guard_enabled").and_then(|v| v.as_bool()))
            .or_else(|| {
                meta.as_ref().and_then(|value| {
                    value
                        .get("access_guard_enabled")
                        .and_then(|flag| flag.as_bool())
                })
            })
            .unwrap_or(false);
        let expose_public = existing_app
            .as_ref()
            .and_then(|app| app.get("expose_public").and_then(|v| v.as_bool()))
            .or_else(|| {
                meta.as_ref()
                    .and_then(|value| value.get("expose_public").and_then(|v| v.as_bool()))
            })
            .unwrap_or(false);
        Some(ExistingAppDeployTarget {
            app_id: app_id.to_string(),
            title,
            app_dir,
            meta,
            access_guard_enabled,
            access_key,
            expose_public,
        })
    } else {
        None
    };
    tracing::debug!(
        target: "agentark.turn_timing",
        stage = "app_deploy_existing_target_lookup",
        duration_ms = stage_started.elapsed().as_millis() as u64,
        requested_app_id = requested_app_id.is_some(),
        updating_existing = existing_target.is_some(),
        "app deploy timing stage"
    );
    if plan.mode == AppDeployMode::Patch && existing_target.is_none() {
        anyhow::bail!("mode='patch' requires app_id for an existing deployed app");
    }
    let title = arguments
        .get("title")
        .and_then(|v| v.as_str())
        .or_else(|| existing_target.as_ref().map(|target| target.title.as_str()))
        .unwrap_or("App");
    let mut entry_command = arguments
        .get("entry_command")
        .or_else(|| arguments.get("start_command"))
        .or_else(|| {
            arguments
                .get("commands")
                .and_then(|value| value.get("start").or_else(|| value.get("entry")))
        })
        .and_then(|v| v.as_str())
        .map(|value| value.to_string())
        .or_else(|| {
            existing_target.as_ref().and_then(|target| {
                target
                    .meta
                    .as_ref()
                    .and_then(|value| app_meta_lifecycle_command(value, "entry_command"))
            })
        });
    let mut install_command = arguments
        .get("install_command")
        .or_else(|| {
            arguments
                .get("commands")
                .and_then(|value| value.get("install").or_else(|| value.get("setup")))
        })
        .and_then(|v| v.as_str())
        .map(|value| value.to_string())
        .or_else(|| {
            existing_target.as_ref().and_then(|target| {
                target
                    .meta
                    .as_ref()
                    .and_then(|value| app_meta_lifecycle_command(value, "install_command"))
            })
        });
    let stop_command = arguments
        .get("stop_command")
        .or_else(|| {
            arguments
                .get("commands")
                .and_then(|value| value.get("stop"))
        })
        .and_then(|v| v.as_str())
        .map(str::trim)
        .filter(|value| !value.is_empty())
        .map(|value| value.to_string())
        .or_else(|| {
            existing_target.as_ref().and_then(|target| {
                target
                    .meta
                    .as_ref()
                    .and_then(|value| app_meta_lifecycle_command(value, "stop_command"))
            })
        });
    let runtime_image = arguments
        .get("runtime_image")
        .and_then(|v| v.as_str())
        .map(|s| s.to_string())
        .or_else(|| {
            existing_target.as_ref().and_then(|target| {
                target
                    .meta
                    .as_ref()
                    .and_then(|value| value.get("runtime_image").and_then(|v| v.as_str()))
                    .map(|value| value.to_string())
            })
        });
    let runtime_preference = runtime_preference_from_opt(
        arguments
            .get("runtime_preference")
            .and_then(|v| v.as_str())
            .or_else(|| {
                existing_target.as_ref().and_then(|target| {
                    target
                        .meta
                        .as_ref()
                        .and_then(|value| value.get("runtime_preference").and_then(|v| v.as_str()))
                })
            }),
    );
    let requested_runtime_required = arguments.get("runtime_required").and_then(|v| v.as_bool());
    let persisted_runtime_required = existing_target.as_ref().and_then(|target| {
        target
            .meta
            .as_ref()
            .and_then(|value| value.get("runtime_required").and_then(|v| v.as_bool()))
    });
    let runtime_required_was_inferred =
        requested_runtime_required.is_none() && persisted_runtime_required.is_none();
    let mut runtime_reason = arguments
        .get("runtime_reason")
        .and_then(|v| v.as_str())
        .map(str::trim)
        .filter(|value| !value.is_empty())
        .map(|value| value.to_string())
        .or_else(|| {
            existing_target.as_ref().and_then(|target| {
                target
                    .meta
                    .as_ref()
                    .and_then(|value| value.get("runtime_reason").and_then(|v| v.as_str()))
                    .map(|value| value.to_string())
            })
        });
    let expose_public = arguments
        .get("expose_public")
        .and_then(|v| v.as_bool())
        .or_else(|| existing_target.as_ref().map(|target| target.expose_public))
        .unwrap_or(false);
    let explicit_access_guard = arguments.get("access_guard").and_then(|v| v.as_bool());
    let requested_access_guard_enabled = explicit_access_guard
        .or_else(|| {
            existing_target
                .as_ref()
                .map(|target| target.access_guard_enabled)
        })
        .unwrap_or(false);
    let mut access_secret = access_secret_from_arguments(arguments)?;
    if access_secret.is_none() && explicit_access_guard.is_none() {
        access_secret = existing_target
            .as_ref()
            .and_then(|target| target.access_key.clone())
            .filter(|value| !value.trim().is_empty());
    }
    let access_guard_enabled = app_access_guard_enabled_for_deploy(
        expose_public,
        requested_access_guard_enabled,
        access_secret.is_some(),
    );
    let mut required_inputs = parse_required_inputs(arguments);
    if required_inputs.is_empty() {
        required_inputs = existing_target
            .as_ref()
            .and_then(|target| target.meta.as_ref())
            .map(parse_required_inputs)
            .unwrap_or_default();
    }
    let mut config_values = parse_config_values(arguments);
    if config_values.is_empty() {
        config_values = existing_target
            .as_ref()
            .and_then(|target| {
                target
                    .meta
                    .as_ref()
                    .and_then(|value| value.get("config_values").and_then(|v| v.as_object()))
            })
            .map(|obj| {
                obj.iter()
                    .filter_map(|(key, value)| match value {
                        serde_json::Value::String(text) => Some((key.clone(), text.clone())),
                        serde_json::Value::Bool(flag) => Some((key.clone(), flag.to_string())),
                        serde_json::Value::Number(number) => {
                            Some((key.clone(), number.to_string()))
                        }
                        _ => None,
                    })
                    .collect::<HashMap<_, _>>()
            })
            .unwrap_or_default();
    }
    let mut runtime_actions = parse_runtime_actions(arguments);
    if runtime_actions.is_empty() {
        runtime_actions = existing_target
            .as_ref()
            .and_then(|target| target.meta.as_ref())
            .map(parse_runtime_actions)
            .unwrap_or_default();
    }
    let updating_existing = existing_target.is_some();
    let app_id = existing_target
        .as_ref()
        .map(|target| target.app_id.clone())
        .unwrap_or_else(|| uuid::Uuid::new_v4().to_string()[..8].to_string());
    let public_access_guard_enabled = access_guard_enabled || expose_public;
    let access_key = if public_access_guard_enabled {
        access_secret.unwrap_or_else(generate_access_key)
    } else {
        String::new()
    };
    let app_dir = existing_target
        .as_ref()
        .map(|target| target.app_dir.clone())
        .unwrap_or_else(|| data_dir.join("apps").join(&app_id));
    let previous_meta = existing_target
        .as_ref()
        .and_then(|target| target.meta.clone());
    let stage_started = std::time::Instant::now();
    let effective_files = effective_app_files_for_validation(
        if updating_existing {
            Some(app_dir.as_path())
        } else {
            None
        },
        &previous_meta,
        &plan,
    )
    .await?;
    tracing::debug!(
        target: "agentark.turn_timing",
        stage = "app_deploy_effective_file_graph",
        app_id = %app_id,
        duration_ms = stage_started.elapsed().as_millis() as u64,
        effective_file_count = effective_files.len(),
        "app deploy timing stage"
    );
    if effective_files.is_empty() {
        anyhow::bail!("App bundle would be empty after applying this deploy");
    }
    if let Some(inferred) = infer_generated_bundle_lifecycle(&effective_files) {
        if entry_command.is_none() {
            entry_command = Some(inferred.entry_command.clone());
        }
        if install_command.is_none() {
            install_command = inferred.install_command.clone();
        }
        if runtime_reason.is_none() {
            runtime_reason = Some(inferred.runtime_reason.clone());
        }
    }
    let mut runtime_required = requested_runtime_required
        .or(persisted_runtime_required)
        .unwrap_or_else(|| entry_command.is_some());
    if entry_command.is_none() {
        runtime_required = false;
    } else if !runtime_required {
        emit_phase_progress(
            &stream_tx,
            AppDeployProgressPhase::PreparingRuntime,
            "runtime_required=false was provided; treating this generated bundle as a static/local deploy",
        )
        .await;
        install_command = None;
        entry_command = None;
    } else if runtime_required_was_inferred {
        emit_phase_progress(
            &stream_tx,
            AppDeployProgressPhase::PreparingRuntime,
            "Runtime command provided; deploying this bundle as a persistent dynamic app",
        )
        .await;
    }
    if let Some(command) = stop_command.as_ref() {
        validate_app_command(command, "stop_command")?;
    }
    let is_static = entry_command.is_none();
    if is_static {
        let stage_started = std::time::Instant::now();
        validate_static_app_asset_references(&effective_files)?;
        tracing::debug!(
            target: "agentark.turn_timing",
            stage = "app_deploy_static_asset_validation",
            app_id = %app_id,
            duration_ms = stage_started.elapsed().as_millis() as u64,
            "app deploy timing stage"
        );
    }
    let content_fingerprint = app_deploy_content_fingerprint(
        &effective_files,
        title,
        entry_command.as_deref(),
        install_command.as_deref(),
        stop_command.as_deref(),
        runtime_required,
        runtime_preference,
        runtime_image.as_deref(),
        &required_inputs,
        &config_values,
        &runtime_actions,
    );
    let artifact_identity_fingerprint =
        app_deploy_identity_fingerprint(arguments, &effective_files, title);
    if !allow_duplicate {
        if let Some(target) = existing_target.as_ref() {
            if let Some(meta) = target.meta.as_ref() {
                let content_matches = app_deploy_existing_fingerprint(&target.app_dir, meta)
                    .await
                    .as_deref()
                    == Some(content_fingerprint.as_str());
                let identity_matches =
                    if let Some(identity_fingerprint) = artifact_identity_fingerprint.as_deref() {
                        app_deploy_existing_identity_fingerprint(&target.app_dir, meta)
                            .await
                            .as_deref()
                            == Some(identity_fingerprint)
                    } else {
                        false
                    };
                if content_matches || identity_matches {
                    let url = format!("/apps/{}/", target.app_id);
                    let access_url = registry
                        .issue_access_url(&target.app_id)
                        .await
                        .unwrap_or_else(|| url.clone());
                    return app_deploy_duplicate_response(
                        AppDuplicateMatch {
                            app_id: target.app_id.clone(),
                            title: target.title.clone(),
                            url,
                            access_url,
                            app_type: if is_static { "static" } else { "dynamic" }.to_string(),
                            updated_existing: true,
                            duplicate_match: if content_matches {
                                "content"
                            } else {
                                "artifact_identity"
                            }
                            .to_string(),
                        },
                        &content_fingerprint,
                        deploy_started,
                    )
                    .await;
                }
            }
        } else if let Some(duplicate) = find_duplicate_deployed_app(
            registry,
            &content_fingerprint,
            artifact_identity_fingerprint.as_deref(),
            None,
        )
        .await
        {
            return app_deploy_duplicate_response(duplicate, &content_fingerprint, deploy_started)
                .await;
        }
    }
    if updating_existing {
        let stage_started = std::time::Instant::now();
        registry.stop_runtime(&app_id).await?;
        tracing::debug!(
            target: "agentark.turn_timing",
            stage = "app_deploy_stop_existing_runtime",
            app_id = %app_id,
            duration_ms = stage_started.elapsed().as_millis() as u64,
            "app deploy timing stage"
        );
    }
    let stage_started = std::time::Instant::now();
    tokio::fs::create_dir_all(&app_dir).await?;
    tracing::debug!(
        target: "agentark.turn_timing",
        stage = "app_deploy_create_app_dir",
        app_id = %app_id,
        duration_ms = stage_started.elapsed().as_millis() as u64,
        "app deploy timing stage"
    );

    tracing::info!(
        "{} app '{}' (id={}, static={})",
        if updating_existing {
            "Updating"
        } else {
            "Deploying"
        },
        title,
        app_id,
        is_static
    );
    emit_phase_progress(
        &stream_tx,
        AppDeployProgressPhase::Deploying,
        format!(
            "{} '{}' ({})",
            if updating_existing {
                "Updating"
            } else {
                "Deploying"
            },
            title,
            if is_static { "static" } else { "dynamic" }
        ),
    )
    .await;
    let stage_started = std::time::Instant::now();
    let apply_outcome = app_deploy_apply(&app_dir, &plan, &previous_meta, &stream_tx).await?;
    tracing::debug!(
        target: "agentark.turn_timing",
        stage = "app_deploy_apply_files",
        app_id = %app_id,
        duration_ms = stage_started.elapsed().as_millis() as u64,
        written_files = apply_outcome.written_names.len(),
        deleted_files = apply_outcome.deleted_names.len(),
        "app deploy timing stage"
    );
    let changed_files = apply_outcome.written_names.len();
    let completed_ops = changed_files.saturating_add(apply_outcome.deleted_names.len());
    if completed_ops == 0 {
        anyhow::bail!("No app file changes were applied. Check paths and try again.");
    }
    let skipped_files = file_count.saturating_sub(completed_ops);
    let mut changed_names = apply_outcome.written_names.clone();
    changed_names.extend(
        apply_outcome
            .deleted_names
            .iter()
            .map(|path| format!("deleted {}", path)),
    );
    emit_phase_progress(
        &stream_tx,
        AppDeployProgressPhase::GeneratingFiles,
        format!(
            "{} / {} file operations applied (skipped {}): {}",
            completed_ops,
            file_count,
            skipped_files,
            changed_names.join(", ")
        ),
    )
    .await;

    let stage_started = std::time::Instant::now();
    let (resolved_env, missing_sensitive, missing_config) = resolve_required_env_values(
        config_dir,
        data_dir,
        &required_inputs,
        llm_env,
        &config_values,
    )
    .await?;
    tracing::debug!(
        target: "agentark.turn_timing",
        stage = "app_deploy_resolve_required_env",
        app_id = %app_id,
        duration_ms = stage_started.elapsed().as_millis() as u64,
        required_inputs = required_inputs.len(),
        resolved_env = resolved_env.len(),
        missing_sensitive = missing_sensitive.len(),
        missing_config = missing_config.len(),
        "app deploy timing stage"
    );

    let required_secret_keys: Vec<String> = required_inputs
        .iter()
        .filter(|r| r.sensitive)
        .map(|r| r.key.clone())
        .collect();
    let required_config_keys: Vec<String> = required_inputs
        .iter()
        .filter(|r| !r.sensitive)
        .map(|r| r.key.clone())
        .collect();

    let requirements_path = app_dir.join("requirements.txt");
    let has_requirements = requirements_path.exists()
        && tokio::fs::metadata(&requirements_path)
            .await
            .map(|m| m.len() > 0)
            .unwrap_or(false);
    let has_package_json = app_dir.join("package.json").exists();

    // Each Python app gets its own venv for isolation. Node apps use local node_modules.
    let effective_install_cmd = if let Some(cmd) = install_command.as_ref() {
        Some(cmd.to_string())
    } else if has_requirements {
        Some("pip install -r requirements.txt -q".to_string())
    } else if has_package_json {
        Some("npm install --omit=dev".to_string())
    } else {
        None
    };

    // Save metadata for restore on restart
    let created_at = existing_target
        .as_ref()
        .and_then(|target| {
            target
                .meta
                .as_ref()
                .and_then(|value| value.get("created_at").and_then(|v| v.as_str()))
        })
        .map(|value| value.to_string())
        .unwrap_or_else(|| chrono::Utc::now().to_rfc3339());
    let conversation_id = arguments
        .get("_conversation_id")
        .or_else(|| arguments.get("conversation_id"))
        .and_then(|value| value.as_str())
        .map(str::trim)
        .filter(|value| !value.is_empty())
        .map(|value| value.to_string())
        .or_else(|| {
            existing_target.as_ref().and_then(|target| {
                target
                    .meta
                    .as_ref()
                    .and_then(|value| value.get("conversation_id").and_then(|v| v.as_str()))
                    .map(str::trim)
                    .filter(|value| !value.is_empty())
                    .map(|value| value.to_string())
            })
        });
    let mut managed_files = effective_files.keys().cloned().collect::<Vec<_>>();
    managed_files.sort_unstable();
    let artifact_identity = app_deploy_artifact_identity_value(arguments);
    let meta = serde_json::json!({
        "title": title,
        "deploy_mode": plan.mode.as_str(),
        "managed_files": managed_files,
        "entry_command": entry_command.clone(),
        "start_command": entry_command.clone(),
        "install_command": effective_install_cmd.clone(),
        "stop_command": stop_command.clone(),
        "commands": {
            "install": effective_install_cmd.clone(),
            "start": entry_command.clone(),
            "stop": stop_command.clone(),
        },
        "runtime_image": runtime_image.clone(),
        "runtime_preference": runtime_preference.as_str(),
        "runtime_required": runtime_required,
        "runtime_reason": runtime_reason.clone(),
        "expose_public": expose_public,
        "repo_url": arguments.get("repo_url").cloned(),
        "repo_ref": arguments.get("repo_ref").cloned(),
        "repo_subdir": arguments.get("repo_subdir").cloned(),
        "repo_bundle_id": arguments.get("repo_bundle_id").cloned(),
        "repo_service_kind": arguments.get("repo_service_kind").cloned(),
        "repo_service_dir": arguments.get("repo_service_dir").cloned(),
        "required_inputs": required_inputs.clone(),
        "required_secrets": required_secret_keys.clone(),
        "required_env": required_secret_keys.clone(),
        "required_config": required_config_keys.clone(),
        "config_values": config_values.clone(),
        "runtime_actions": runtime_actions.clone(),
        "access_guard_enabled": access_guard_enabled,
        "public_access_guard_enabled": public_access_guard_enabled,
        "enabled": true,
        "conversation_id": conversation_id,
        "content_fingerprint": content_fingerprint,
        "artifact_identity_fingerprint": artifact_identity_fingerprint,
        "artifact_identity": artifact_identity,
        "created_at": created_at,
        "updated_at": chrono::Utc::now().to_rfc3339(),
        "last_accessed": chrono::Utc::now().to_rfc3339(),
    });
    let stage_started = std::time::Instant::now();
    tokio::fs::write(
        app_dir.join(".app_meta.json"),
        serde_json::to_string_pretty(&meta)?,
    )
    .await?;
    tracing::debug!(
        target: "agentark.turn_timing",
        stage = "app_deploy_write_metadata",
        app_id = %app_id,
        duration_ms = stage_started.elapsed().as_millis() as u64,
        "app deploy timing stage"
    );
    emit_phase_progress(
        &stream_tx,
        AppDeployProgressPhase::PreparingRuntime,
        "Saved app metadata",
    )
    .await;

    if is_static {
        // Static app: just register, served directly by HTTP server
        let app_dir_for_external = app_dir.clone();
        registry
            .register_stored(
                app_id.clone(),
                StoredAppRegistration {
                    title: title.to_string(),
                    app_dir,
                    is_static: true,
                    access_key: access_key.clone(),
                    access_guard_enabled,
                    expose_public,
                    enabled: true,
                    last_accessed: None,
                },
            )
            .await;
        let url = format!("/apps/{}/", app_id);
        let access_url = registry
            .issue_access_url(&app_id)
            .await
            .unwrap_or_else(|| url.clone());
        tracing::info!("Static app deployed at {}", url);
        emit_phase_progress(
            &stream_tx,
            AppDeployProgressPhase::Completed,
            format!("Static app ready at {}", url),
        )
        .await;
        let external_deployment = maybe_publish_external_deployment(
            config_dir,
            data_dir,
            arguments,
            &app_id,
            &app_dir_for_external,
            title,
            &stream_tx,
        )
        .await;
        let mut response = serde_json::json!({
            "status": "deployed",
            "type": "static",
            "app_id": app_id,
            "url": url,
            "access_url": access_url,
            "title": title,
            "updated_existing": updating_existing,
            "runtime_preference": runtime_preference.as_str(),
            "expose_public": expose_public,
            "access_key": access_key,
            "access_password": access_key,
            "access_guard_enabled": access_guard_enabled,
            "public_access_guard_enabled": public_access_guard_enabled,
            "apps_page_hint": APP_DEPLOY_CONTROL_HINT,
        });
        attach_external_deployment_result(&mut response, external_deployment);
        tracing::debug!(
            target: "agentark.turn_timing",
            stage = "app_deploy_total",
            app_id = %app_id,
            app_type = "static",
            duration_ms = deploy_started.elapsed().as_millis() as u64,
            updated_existing = updating_existing,
            "app deploy timing total"
        );
        return Ok(response.to_string());
    }

    if !missing_sensitive.is_empty() || !missing_config.is_empty() {
        let mut missing_all = missing_sensitive.clone();
        for m in &missing_config {
            if !missing_all.iter().any(|x| x == m) {
                missing_all.push(m.clone());
            }
        }
        registry
            .register_stored(
                app_id.clone(),
                StoredAppRegistration {
                    title: title.to_string(),
                    app_dir,
                    is_static: true,
                    access_key: access_key.clone(),
                    access_guard_enabled,
                    expose_public,
                    enabled: true,
                    last_accessed: None,
                },
            )
            .await;
        emit_phase_progress(
            &stream_tx,
            AppDeployProgressPhase::WaitingForInputs,
            format!(
                "App created but waiting for required inputs: {}",
                missing_all.join(", ")
            ),
        )
        .await;
        tracing::debug!(
            target: "agentark.turn_timing",
            stage = "app_deploy_total",
            app_id = %app_id,
            app_type = "dynamic_needs_inputs",
            duration_ms = deploy_started.elapsed().as_millis() as u64,
            updated_existing = updating_existing,
            missing_sensitive = missing_sensitive.len(),
            missing_config = missing_config.len(),
            "app deploy timing total"
        );
        return Ok(serde_json::json!({
            "status": "needs_secrets",
            "type": "dynamic",
            "app_id": app_id,
            "title": title,
            "url": format!("/apps/{}/", app_id),
            "updated_existing": updating_existing,
            "runtime_preference": runtime_preference.as_str(),
            "expose_public": expose_public,
            "access_key": access_key,
            "access_password": access_key,
            "access_guard_enabled": access_guard_enabled,
            "public_access_guard_enabled": public_access_guard_enabled,
            "apps_page_hint": APP_DEPLOY_CONTROL_HINT,
            "required_inputs": required_inputs,
            "required_secrets": required_secret_keys.clone(),
            "required_env": required_secret_keys,
            "required_config": required_config_keys,
            "missing_env": missing_sensitive,
            "missing_config": missing_config,
            "message": "Missing required inputs. Sensitive keys must be provided through the secure credential form or Settings for this app; AgentArk model/provider credentials are not inherited by generated apps. For non-sensitive values pass config.{KEY} when deploying/restarting."
        })
        .to_string());
    }

    if control_plane_catalog_mode() {
        let app_dir_for_external = app_dir.clone();
        registry
            .register_stored(
                app_id.clone(),
                StoredAppRegistration {
                    title: title.to_string(),
                    app_dir,
                    is_static: false,
                    access_key: access_key.clone(),
                    access_guard_enabled,
                    expose_public,
                    enabled: true,
                    last_accessed: None,
                },
            )
            .await;
        emit_phase_progress(
            &stream_tx,
            AppDeployProgressPhase::StartingRuntime,
            "Dynamic app files are ready on the control plane; runtime start will be delegated to the executor",
        )
        .await;
        let external_deployment = maybe_publish_external_deployment(
            config_dir,
            data_dir,
            arguments,
            &app_id,
            &app_dir_for_external,
            title,
            &stream_tx,
        )
        .await;
        let mut response = serde_json::json!({
            "status": "deployed",
            "type": "dynamic",
            "runtime": "delegated",
            "runtime_delegated": true,
            "app_id": app_id,
            "url": format!("/apps/{}/", app_id),
            "title": title,
            "updated_existing": updating_existing,
            "runtime_preference": runtime_preference.as_str(),
            "expose_public": expose_public,
            "access_key": access_key,
            "access_password": access_key,
            "access_guard_enabled": access_guard_enabled,
            "public_access_guard_enabled": public_access_guard_enabled,
            "apps_page_hint": APP_DEPLOY_CONTROL_HINT,
        });
        attach_external_deployment_result(&mut response, external_deployment);
        tracing::debug!(
            target: "agentark.turn_timing",
            stage = "app_deploy_total",
            app_id = %app_id,
            app_type = "dynamic_delegated",
            duration_ms = deploy_started.elapsed().as_millis() as u64,
            updated_existing = updating_existing,
            "app deploy timing total"
        );
        return Ok(response.to_string());
    }

    // Dynamic app: start server in isolated container runtime
    let port = arguments
        .get("port")
        .and_then(|v| v.as_u64())
        .map(|p| p as u16);

    let stage_started = std::time::Instant::now();
    let port = match port {
        Some(p) => p,
        None => registry.find_available_port().await.ok_or_else(|| {
            anyhow::anyhow!(
                "No available ports in range {}-{}",
                PORT_RANGE_START,
                PORT_RANGE_END
            )
        })?,
    };
    tracing::debug!(
        target: "agentark.turn_timing",
        stage = "app_deploy_port_selection",
        app_id = %app_id,
        duration_ms = stage_started.elapsed().as_millis() as u64,
        port,
        "app deploy timing stage"
    );
    emit_phase_progress(
        &stream_tx,
        AppDeployProgressPhase::PreparingRuntime,
        format!("Assigned port {}", port),
    )
    .await;

    if effective_install_cmd.is_some() {
        emit_phase_progress(
            &stream_tx,
            AppDeployProgressPhase::Installing,
            "Installing dependencies...",
        )
        .await;
    } else {
        emit_phase_progress(
            &stream_tx,
            AppDeployProgressPhase::Installing,
            "No dependencies to install",
        )
        .await;
    }

    // Start the server process in isolated container
    let entry = entry_command.as_deref().unwrap_or_default();
    tracing::info!(
        "Starting app {} on port {} in isolated runtime",
        app_id,
        port
    );
    emit_phase_progress(
        &stream_tx,
        AppDeployProgressPhase::StartingRuntime,
        format!("Starting server on port {}", port),
    )
    .await;

    let stage_started = std::time::Instant::now();
    let mut runtime_handle = launch_dynamic_runtime(DynamicRuntimeLaunch {
        app_id: &app_id,
        app_dir: &app_dir,
        entry_command: entry,
        install_command: effective_install_cmd.as_deref(),
        port,
        extra_env: &resolved_env,
        runtime_image: runtime_image.as_deref(),
        runtime_preference,
        stream_tx: stream_tx.clone(),
    })
    .await?;
    tracing::debug!(
        target: "agentark.turn_timing",
        stage = "app_deploy_launch_runtime",
        app_id = %app_id,
        duration_ms = stage_started.elapsed().as_millis() as u64,
        port,
        has_install_command = effective_install_cmd.is_some(),
        "app deploy timing stage"
    );
    let app_dir_for_diagnostics = app_dir.clone();
    let app_dir_for_external = app_dir.clone();
    let proxy_path_mode = proxy_path_mode_for_entry_command(Some(entry), &app_dir, &app_id);
    if let Err(error) = persist_app_proxy_path_mode_meta(&app_dir, proxy_path_mode).await {
        tracing::warn!(
            "Failed to persist app proxy path mode for '{}': {}",
            app_id,
            error
        );
    }

    emit_phase_progress(
        &stream_tx,
        AppDeployProgressPhase::StartingRuntime,
        "Waiting for server readiness",
    )
    .await;
    let stage_started = std::time::Instant::now();
    if let Err(wait_err) = wait_for_runtime_port_open(&app_id, port, &stream_tx).await {
        tracing::debug!(
            target: "agentark.turn_timing",
            stage = "app_deploy_wait_runtime_ready",
            app_id = %app_id,
            duration_ms = stage_started.elapsed().as_millis() as u64,
            port,
            success = false,
            error = %wait_err,
            "app deploy timing stage failed"
        );
        stop_dynamic_runtime_handle(&app_id, &mut runtime_handle).await;
        let log_tail =
            read_local_runtime_log_tail(&app_dir_for_diagnostics, LOCAL_RUNTIME_LOG_TAIL_BYTES)
                .await;
        if log_tail.is_empty() {
            anyhow::bail!("{}", wait_err);
        }
        anyhow::bail!("{}. Recent runtime logs:\n{}", wait_err, log_tail);
    }
    tracing::debug!(
        target: "agentark.turn_timing",
        stage = "app_deploy_wait_runtime_ready",
        app_id = %app_id,
        duration_ms = stage_started.elapsed().as_millis() as u64,
        port,
        success = true,
        "app deploy timing stage"
    );

    let (container_id, child, runtime_label) = match runtime_handle {
        DynamicRuntimeHandle::Container(container_id) => {
            emit_phase_progress(
                &stream_tx,
                AppDeployProgressPhase::StartingRuntime,
                "Server container is accepting connections",
            )
            .await;
            (Some(container_id), None, "container")
        }
        DynamicRuntimeHandle::Process(child) => {
            emit_phase_progress(
                &stream_tx,
                AppDeployProgressPhase::StartingRuntime,
                "Local app process is accepting connections",
            )
            .await;
            (None, Some(*child), "local_process")
        }
    };

    let stage_started = std::time::Instant::now();
    registry
        .register_dynamic(
            app_id.clone(),
            DynamicAppRegistration {
                title: title.to_string(),
                app_dir,
                child,
                container_id,
                port,
                access_key: access_key.clone(),
                access_guard_enabled,
                expose_public,
                enabled: true,
                last_accessed: None,
            },
        )
        .await;
    tracing::debug!(
        target: "agentark.turn_timing",
        stage = "app_deploy_register_dynamic",
        app_id = %app_id,
        duration_ms = stage_started.elapsed().as_millis() as u64,
        port,
        "app deploy timing stage"
    );

    let url = format!("/apps/{}/", app_id);
    let access_url = registry
        .issue_access_url(&app_id)
        .await
        .unwrap_or_else(|| url.clone());
    tracing::info!("Dynamic app deployed at {} (port {})", url, port);
    emit_phase_progress(
        &stream_tx,
        AppDeployProgressPhase::Completed,
        format!("Dynamic app ready at {}", url),
    )
    .await;

    let stage_started = std::time::Instant::now();
    let external_deployment = maybe_publish_external_deployment(
        config_dir,
        data_dir,
        arguments,
        &app_id,
        &app_dir_for_external,
        title,
        &stream_tx,
    )
    .await;
    tracing::debug!(
        target: "agentark.turn_timing",
        stage = "app_deploy_external_publish",
        app_id = %app_id,
        duration_ms = stage_started.elapsed().as_millis() as u64,
        requested = arguments.get("deploy_target").is_some(),
        "app deploy timing stage"
    );
    let mut response = serde_json::json!({
        "status": "deployed",
        "type": "dynamic",
        "runtime": runtime_label,
        "app_id": app_id,
        "url": url,
        "access_url": access_url,
        "port": port,
        "title": title,
        "updated_existing": updating_existing,
        "runtime_preference": runtime_preference.as_str(),
        "expose_public": expose_public,
        "access_key": access_key,
        "access_password": access_key,
        "access_guard_enabled": access_guard_enabled,
        "public_access_guard_enabled": public_access_guard_enabled,
        "apps_page_hint": APP_DEPLOY_CONTROL_HINT,
    });
    attach_external_deployment_result(&mut response, external_deployment);
    tracing::debug!(
        target: "agentark.turn_timing",
        stage = "app_deploy_total",
        app_id = %app_id,
        app_type = "dynamic",
        runtime = runtime_label,
        duration_ms = deploy_started.elapsed().as_millis() as u64,
        updated_existing = updating_existing,
        port,
        "app deploy timing total"
    );
    Ok(response.to_string())
}

#[cfg(test)]
mod tests {
    use super::*;

    static ENV_LOCK: std::sync::Mutex<()> = std::sync::Mutex::new(());

    #[test]
    fn deploy_access_guard_defaults_off_for_local_private_apps() {
        assert!(!app_access_guard_enabled_for_deploy(false, false, false));
    }

    #[test]
    fn deploy_access_guard_defaults_off_for_local_public_apps() {
        assert!(!app_access_guard_enabled_for_deploy(true, false, false));
    }

    #[test]
    fn public_exposure_does_not_force_local_guard() {
        assert!(!app_access_guard_enabled_for_deploy(true, false, true));
        assert!(app_access_guard_enabled_for_deploy(true, true, false));
    }

    #[test]
    fn deploy_access_guard_enables_for_explicit_local_guard_or_password() {
        assert!(app_access_guard_enabled_for_deploy(false, true, false));
        assert!(app_access_guard_enabled_for_deploy(false, false, true));
    }

    #[test]
    fn static_validation_accepts_root_relative_bundled_assets() {
        let mut files = serde_json::Map::new();
        files.insert(
            "index.html".to_string(),
            serde_json::Value::String(
                r#"<html><head><link rel="stylesheet" href="/style.css"></head><body><script src="/app.js"></script></body></html>"#
                    .to_string(),
            ),
        );
        files.insert(
            "style.css".to_string(),
            serde_json::Value::String(r#"body{background:url('/bg.png')}"#.to_string()),
        );
        files.insert(
            "app.js".to_string(),
            serde_json::Value::String("console.log('ok')".to_string()),
        );
        files.insert(
            "bg.png".to_string(),
            serde_json::Value::String("placeholder".to_string()),
        );

        validate_static_app_asset_references(&files)
            .expect("root-relative bundled files are valid");
    }

    #[test]
    fn static_validation_rejects_missing_root_relative_assets() {
        let mut files = serde_json::Map::new();
        files.insert(
            "index.html".to_string(),
            serde_json::Value::String(
                r#"<html><head><script src="/missing.js"></script></head><body></body></html>"#
                    .to_string(),
            ),
        );

        let error = validate_static_app_asset_references(&files)
            .expect_err("missing root-relative asset should be reported")
            .to_string();
        assert!(error.contains("missing root-relative local asset /missing.js"));
    }

    #[test]
    fn app_deploy_phase_status_payload_uses_explicit_phase_metadata() {
        let payload = app_deploy_phase_status_payload(
            AppDeployProgressPhase::GeneratingFiles,
            "Wrote index.html",
        );

        assert_eq!(
            payload.get("kind").and_then(|value| value.as_str()),
            Some("phase_status")
        );
        assert_eq!(
            payload.get("phase").and_then(|value| value.as_str()),
            Some("generating_files")
        );
        assert_eq!(
            payload.get("label").and_then(|value| value.as_str()),
            Some("Generating files")
        );
        assert_eq!(
            payload.get("detail").and_then(|value| value.as_str()),
            Some("Wrote index.html")
        );
    }

    fn write_package_json(dir: &Path, body: &str) {
        std::fs::write(dir.join("package.json"), body).expect("write package.json");
    }

    fn generated_node_server_bundle_files() -> serde_json::Map<String, serde_json::Value> {
        let mut files = serde_json::Map::new();
        files.insert(
            "package.json".to_string(),
            serde_json::Value::String(
                r#"{"scripts":{"start":"node server.js"},"dependencies":{"express":"^4.18.0"}}"#
                    .to_string(),
            ),
        );
        files.insert(
            "server.js".to_string(),
            serde_json::Value::String(
                "const express = require('express');\nconst app = express();\napp.use(express.static('public'));\napp.get('/api/papers', (_req, res) => res.json({papers: []}));\napp.listen(process.env.PORT || 3000);\n"
                    .to_string(),
            ),
        );
        files.insert(
            "public/index.html".to_string(),
            serde_json::Value::String(
                "<!doctype html><html><body><script src=\"/client.js\"></script></body></html>"
                    .to_string(),
            ),
        );
        files
    }

    #[test]
    fn generated_node_server_bundle_infers_dynamic_lifecycle() {
        let inference = infer_generated_bundle_lifecycle(&generated_node_server_bundle_files())
            .expect("node server bundle should infer a runtime lifecycle");

        assert_eq!(inference.entry_command, "npm run start");
        assert_eq!(
            inference.install_command.as_deref(),
            Some("npm install --omit=dev")
        );
        assert!(inference.runtime_reason.contains("Node package manifest"));
    }

    #[test]
    fn static_bundle_without_runtime_shape_does_not_infer_lifecycle() {
        let mut files = serde_json::Map::new();
        files.insert(
            "index.html".to_string(),
            serde_json::Value::String("<!doctype html><html><body>demo</body></html>".to_string()),
        );

        assert!(
            infer_generated_bundle_lifecycle(&files).is_none(),
            "plain static bundles should stay static"
        );
    }

    #[tokio::test]
    async fn app_deploy_preflight_accepts_generated_node_server_bundle_as_dynamic() {
        let data_dir = tempfile::tempdir().expect("data dir");
        let registry = AppRegistry::new();
        let mut arguments = serde_json::json!({
            "title": "Generated server bundle"
        });
        arguments.as_object_mut().expect("arguments object").insert(
            "files".to_string(),
            serde_json::Value::Object(generated_node_server_bundle_files()),
        );

        app_deploy_preflight(data_dir.path(), &arguments, &registry)
            .await
            .expect("dynamic generated bundles should not be rejected as broken static assets");
    }

    #[test]
    fn restored_bundle_scan_ignores_runtime_generated_files() {
        let app_dir = tempfile::tempdir().expect("app dir");
        std::fs::write(
            app_dir.path().join("package.json"),
            r#"{"scripts":{"start":"node server.js"}}"#,
        )
        .expect("package json");
        std::fs::write(app_dir.path().join("server.js"), "require('express')()\n")
            .expect("server js");
        std::fs::create_dir_all(app_dir.path().join("node_modules").join("express"))
            .expect("node_modules dir");
        std::fs::write(
            app_dir
                .path()
                .join("node_modules")
                .join("express")
                .join("index.js"),
            "module.exports = {}\n",
        )
        .expect("node module");
        std::fs::write(
            app_dir.path().join(".agentark_runtime_stderr.log"),
            "runtime log\n",
        )
        .expect("runtime log");

        let files =
            collect_existing_app_text_files_sync(app_dir.path()).expect("collect restored files");

        assert!(files.contains_key("package.json"));
        assert!(files.contains_key("server.js"));
        assert!(!files.contains_key("node_modules/express/index.js"));
        assert!(!files.contains_key(".agentark_runtime_stderr.log"));
    }

    #[test]
    fn populated_node_modules_skip_redundant_node_install_command() {
        let app_dir = tempfile::tempdir().expect("app dir");
        std::fs::write(
            app_dir.path().join("package.json"),
            r#"{"scripts":{"start":"node server.js"},"dependencies":{"express":"latest"}}"#,
        )
        .expect("package json");
        std::fs::create_dir_all(app_dir.path().join("node_modules").join("express"))
            .expect("node module dir");
        std::fs::write(
            app_dir
                .path()
                .join("node_modules")
                .join("express")
                .join("index.js"),
            "module.exports = {}\n",
        )
        .expect("node module file");

        assert!(should_skip_redundant_install_command(
            app_dir.path(),
            "npm install --omit=dev"
        ));
        assert!(should_skip_redundant_install_command(
            app_dir.path(),
            "pnpm install"
        ));
        assert!(!should_skip_redundant_install_command(
            app_dir.path(),
            "npm run build"
        ));
    }

    #[tokio::test]
    async fn reserved_restoring_dynamic_app_does_not_expose_unready_port() {
        let registry = AppRegistry::new();
        let app_dir = tempfile::tempdir().expect("app dir");
        let port = registry
            .reserve_restoring_dynamic(
                "demo".to_string(),
                "Demo".to_string(),
                app_dir.path().to_path_buf(),
                String::new(),
                false,
                false,
            )
            .await
            .expect("port should be reserved");

        assert_eq!(registry.get_port("demo").await, None);
        assert!(
            !runtime_port_accepts_connections(port).await,
            "reserved restore port should not be accepting connections"
        );
    }

    #[tokio::test]
    async fn app_meta_timestamp_update_preserves_lifecycle_metadata() {
        let app_dir = tempfile::tempdir().expect("app dir");
        tokio::fs::write(
            app_dir.path().join(".app_meta.json"),
            serde_json::to_vec_pretty(&serde_json::json!({
                "title": "Generated Bundle",
                "entry_command": "npm run start",
                "start_command": "npm run start",
                "install_command": "npm install --omit=dev",
                "commands": {
                    "start": "npm run start",
                    "install": "npm install --omit=dev"
                },
                "runtime_required": true
            }))
            .expect("meta should serialize"),
        )
        .await
        .expect("meta should be written");

        persist_app_last_accessed_meta(app_dir.path(), chrono::Utc::now())
            .await
            .expect("last_accessed update should succeed");

        let meta_raw = tokio::fs::read(app_dir.path().join(".app_meta.json"))
            .await
            .expect("meta should be readable");
        let meta: serde_json::Value =
            serde_json::from_slice(&meta_raw).expect("meta should parse as json");
        assert_eq!(
            app_meta_lifecycle_command(&meta, "entry_command").as_deref(),
            Some("npm run start")
        );
        assert_eq!(
            app_meta_lifecycle_command(&meta, "install_command").as_deref(),
            Some("npm install --omit=dev")
        );
        assert!(meta.get("last_accessed").and_then(|v| v.as_str()).is_some());
    }

    #[test]
    fn vite_direct_entry_command_gets_app_mount_base() {
        let temp = tempfile::tempdir().expect("tempdir");
        write_package_json(
            temp.path(),
            r#"{"scripts":{"dev":"vite"},"dependencies":{"vite":"^6.0.0"}}"#,
        );

        let command = apply_app_mount_base_to_vite_entry_command(
            "npx vite --host 0.0.0.0 --port {PORT}",
            temp.path(),
            "demo1234",
        )
        .expect("rewrite command");

        assert_eq!(
            command,
            "npx vite --host 0.0.0.0 --port {PORT} --base /apps/demo1234/"
        );
    }

    #[test]
    fn vite_npm_run_entry_command_passes_base_to_script() {
        let temp = tempfile::tempdir().expect("tempdir");
        write_package_json(
            temp.path(),
            r#"{"scripts":{"dev":"vite"},"dependencies":{"vite":"^6.0.0"}}"#,
        );

        let command =
            apply_app_mount_base_to_vite_entry_command("npm run dev", temp.path(), "abc12345")
                .expect("rewrite command");

        assert_eq!(command, "npm run dev -- --base /apps/abc12345/");
    }

    #[test]
    fn vite_entry_command_keeps_existing_base() {
        let temp = tempfile::tempdir().expect("tempdir");
        write_package_json(
            temp.path(),
            r#"{"scripts":{"preview":"vite preview"},"dependencies":{"vite":"^6.0.0"}}"#,
        );

        let command = apply_app_mount_base_to_vite_entry_command(
            "npm run preview -- --host 0.0.0.0 --base /custom/",
            temp.path(),
            "demo1234",
        )
        .expect("rewrite command");

        assert_eq!(command, "npm run preview -- --host 0.0.0.0 --base /custom/");
    }

    #[test]
    fn non_vite_entry_command_is_unchanged() {
        let temp = tempfile::tempdir().expect("tempdir");
        write_package_json(
            temp.path(),
            r#"{"scripts":{"start":"node server.js"},"dependencies":{"express":"^4.0.0"}}"#,
        );

        let command =
            apply_app_mount_base_to_vite_entry_command("npm run start", temp.path(), "demo1234")
                .expect("rewrite command");

        assert_eq!(command, "npm run start");
    }

    #[test]
    fn vite_runtime_uses_app_scoped_proxy_paths() {
        let temp = tempfile::tempdir().expect("tempdir");
        write_package_json(
            temp.path(),
            r#"{"scripts":{"dev":"vite"},"dependencies":{"vite":"^6.0.0"}}"#,
        );

        let mode = proxy_path_mode_for_entry_command(Some("npm run dev"), temp.path(), "abc12345");

        assert_eq!(mode, AppProxyPathMode::PreserveAppPrefix);
        assert_eq!(
            dynamic_app_upstream_path("abc12345", "", mode),
            "/apps/abc12345/"
        );
        assert_eq!(
            dynamic_app_upstream_path("abc12345", "src/main.tsx", mode),
            "/apps/abc12345/src/main.tsx"
        );
    }

    #[test]
    fn non_vite_runtime_keeps_stripped_proxy_paths() {
        let temp = tempfile::tempdir().expect("tempdir");
        write_package_json(
            temp.path(),
            r#"{"scripts":{"start":"node server.js"},"dependencies":{"express":"^4.0.0"}}"#,
        );

        let mode =
            proxy_path_mode_for_entry_command(Some("npm run start"), temp.path(), "demo1234");

        assert_eq!(mode, AppProxyPathMode::StripAppPrefix);
        assert_eq!(dynamic_app_upstream_path("demo1234", "", mode), "/");
        assert_eq!(
            dynamic_app_upstream_path("demo1234", "api/health", mode),
            "/api/health"
        );
    }

    #[test]
    fn default_runtime_preference_requires_usable_container_runtime() {
        let _guard = ENV_LOCK.lock().expect("env lock");
        let original_path = std::env::var_os("PATH");
        let original_docker_host = std::env::var_os("DOCKER_HOST");
        let original_default = std::env::var_os("AGENTARK_APP_RUNTIME_DEFAULT");
        let temp_path = tempfile::tempdir().expect("temp path");

        std::env::set_var("PATH", temp_path.path());
        std::env::set_var("DOCKER_HOST", "unix:///tmp/agentark-missing-docker.sock");
        std::env::remove_var("AGENTARK_APP_RUNTIME_DEFAULT");

        assert!(!docker_cli_available());
        assert!(!container_runtime_available());
        assert_eq!(default_runtime_preference(), RuntimePreference::Local);

        match original_path {
            Some(value) => std::env::set_var("PATH", value),
            None => std::env::remove_var("PATH"),
        }
        match original_docker_host {
            Some(value) => std::env::set_var("DOCKER_HOST", value),
            None => std::env::remove_var("DOCKER_HOST"),
        }
        match original_default {
            Some(value) => std::env::set_var("AGENTARK_APP_RUNTIME_DEFAULT", value),
            None => std::env::remove_var("AGENTARK_APP_RUNTIME_DEFAULT"),
        }
    }

    #[test]
    fn resolve_secret_value_requires_explicit_secret_mapping() {
        let custom = HashMap::new();
        let llm_env = HashMap::from([("OPENAI_API_KEY".to_string(), "sk-live-secret".to_string())]);

        assert_eq!(
            resolve_secret_value(&custom, &llm_env, "OPENAI_API_KEY"),
            None
        );
    }

    #[test]
    fn parse_runtime_actions_keeps_declared_action_names_only() {
        let actions = parse_runtime_actions(&serde_json::json!({
            "runtime_actions": [
                "google_drive_search",
                { "action": "api__linear__post-graphql" },
                "GOOGLE_DRIVE_SEARCH",
                "../bad",
                { "name": "ignored_without_action_key" },
                ""
            ]
        }));

        assert_eq!(
            actions,
            vec![
                "google_drive_search".to_string(),
                "api__linear__post-graphql".to_string()
            ]
        );
    }

    #[test]
    fn inherited_env_scrub_uses_secret_shape_not_dev_tool_allowlist() {
        assert!(is_orchestrator_secret_var("OPENAI_API_KEY"));
        assert!(is_orchestrator_secret_var("mySecretKey"));
        assert!(is_orchestrator_secret_var("DATABASE_URL"));
        assert!(is_orchestrator_secret_var("service-access-token"));

        assert!(!is_orchestrator_secret_var("JAVA_HOME"));
        assert!(!is_orchestrator_secret_var("NODE_PATH"));
        assert!(!is_orchestrator_secret_var("RUSTUP_HOME"));
        assert!(!is_orchestrator_secret_var("CUSTOM_CA_BUNDLE"));
        assert!(!is_orchestrator_secret_var("PUBLIC_API_BASE_URL"));
    }

    #[tokio::test]
    async fn app_scoped_required_api_secret_can_be_injected_explicitly() {
        let config_dir = tempfile::tempdir().expect("config dir");
        let data_dir = tempfile::tempdir().expect("data dir");
        let manager = crate::core::config::SecureConfigManager::new_with_data_dir(
            config_dir.path(),
            Some(data_dir.path()),
        )
        .expect("secure config");
        manager
            .set_custom_secret("env:OPENAI_API_KEY", Some("sk-user-app".to_string()))
            .expect("stored app secret");

        let required = vec![AppRequiredInput {
            key: "OPENAI_API_KEY".to_string(),
            sensitive: true,
        }];
        let llm_env = HashMap::from([(
            "OPENAI_API_KEY".to_string(),
            "sk-agentark-internal".to_string(),
        )]);
        let (resolved, missing_sensitive, missing_config) = resolve_required_env_values(
            config_dir.path(),
            data_dir.path(),
            &required,
            &llm_env,
            &HashMap::new(),
        )
        .await
        .expect("resolve env");

        assert_eq!(
            resolved.get("OPENAI_API_KEY").map(String::as_str),
            Some("sk-user-app")
        );
        assert!(missing_sensitive.is_empty());
        assert!(missing_config.is_empty());
    }

    #[test]
    fn docker_requirement_stays_narrow_for_local_fallback() {
        let app_dir = tempfile::tempdir().expect("app dir");
        std::fs::write(
            app_dir.path().join("package.json"),
            r#"{"dependencies":{"redis":"latest","pg":"latest"}}"#,
        )
        .expect("package json");

        assert!(!docker_required_for_app(app_dir.path(), None));
        assert!(docker_required_for_app(
            app_dir.path(),
            Some("agentark/custom-runner:latest")
        ));

        std::fs::write(app_dir.path().join("compose.yml"), "services: {}\n").expect("compose file");
        assert!(docker_required_for_app(app_dir.path(), None));
    }

    #[test]
    fn dynamic_container_run_args_include_hardening_flags() {
        let app_dir = Path::new("/tmp/agentark-demo");
        let args = build_dynamic_container_run_args(
            "demo",
            app_dir,
            9123,
            DEFAULT_FALLBACK_APP_RUNTIME_IMAGE,
            "agentark-app-demo".to_string(),
            Some(Path::new("/tmp/agentark-demo/.agentark.env")),
            None,
            "npm run start".to_string(),
        );

        for expected in [
            "--memory",
            "512m",
            "--memory-swap",
            "--cpus",
            "0.5",
            "--pids-limit",
            "128",
            "--security-opt",
            "no-new-privileges=true",
            "--cap-drop",
            "ALL",
            "--user",
            "65532:65532",
            "--tmpfs",
            "/tmp:size=64m,noexec,nosuid,nodev",
        ] {
            assert!(
                args.iter().any(|value| value == expected),
                "missing hardening arg {} in {:?}",
                expected,
                args
            );
        }
    }

    #[test]
    fn dynamic_container_run_args_share_executor_network_without_host_publish() {
        let app_dir = Path::new("/tmp/agentark-demo");
        let args = build_dynamic_container_run_args(
            "demo",
            app_dir,
            9123,
            DEFAULT_FALLBACK_APP_RUNTIME_IMAGE,
            "agentark-app-demo".to_string(),
            None,
            Some("executor-container-id"),
            "npm run start".to_string(),
        );

        assert!(args
            .windows(2)
            .any(|pair| pair[0] == "--network" && pair[1] == "container:executor-container-id"));
        assert!(
            !args.iter().any(|arg| arg == "-p"),
            "shared-network app containers should not publish host-loopback ports"
        );
    }

    #[test]
    fn dynamic_container_commands_bypass_agentark_image_entrypoint() {
        let app_dir = Path::new("/tmp/agentark-demo");
        let args = build_dynamic_container_run_args(
            "demo",
            app_dir,
            9123,
            DEFAULT_FALLBACK_APP_RUNTIME_IMAGE,
            "agentark-app-demo".to_string(),
            None,
            None,
            "npm run start".to_string(),
        );

        assert!(args
            .windows(2)
            .any(|pair| pair[0] == "--entrypoint" && pair[1] == "/bin/sh"));
        let image_index = args
            .iter()
            .position(|arg| arg == DEFAULT_FALLBACK_APP_RUNTIME_IMAGE)
            .expect("runtime image should be present");
        assert_eq!(args.get(image_index + 1).map(String::as_str), Some("-lc"));

        let install_args = build_dynamic_container_install_args(
            "demo",
            app_dir,
            9123,
            DEFAULT_FALLBACK_APP_RUNTIME_IMAGE,
            "agentark-app-demo-install".to_string(),
            None,
            "npm install".to_string(),
        );
        assert!(install_args
            .windows(2)
            .any(|pair| pair[0] == "--entrypoint" && pair[1] == "/bin/sh"));
        let image_index = install_args
            .iter()
            .position(|arg| arg == DEFAULT_FALLBACK_APP_RUNTIME_IMAGE)
            .expect("runtime image should be present");
        assert_eq!(
            install_args.get(image_index + 1).map(String::as_str),
            Some("-lc")
        );
    }

    #[test]
    fn validate_app_command_rejects_shell_operators() {
        let error = validate_app_command("npm run dev && echo hi", "entry_command")
            .expect_err("shell operators should be rejected");
        assert!(error.to_string().contains("shell operators"));
    }

    #[test]
    fn validate_app_command_rejects_shell_interpreters() {
        let error = validate_app_command("bash -c 'npm run dev'", "entry_command")
            .expect_err("shell interpreters should be rejected");
        assert!(error.to_string().contains("direct command"));
    }

    #[test]
    fn extract_readme_hints_detects_install_and_start_commands() {
        let readme = r#"
# Demo

```bash
$ npm install
$ npm run dev
```
"#;

        let hints = extract_readme_hints(readme);
        assert_eq!(hints.install_command.as_deref(), Some("npm install"));
        assert_eq!(hints.start_command.as_deref(), Some("npm run dev"));
        assert!(!hints.mentions_compose);
    }

    #[test]
    fn is_allowed_repo_url_rejects_localhost() {
        assert!(is_allowed_repo_url("http://127.0.0.1/repo").is_err());
        assert!(is_allowed_repo_url("http://localhost/repo").is_err());
        assert!(is_allowed_repo_url("https://github.com/openai/demo").is_ok());
    }

    #[test]
    fn plan_repo_services_detects_simple_frontend_and_backend_repo() {
        let repo = tempfile::tempdir().expect("temp repo");
        let frontend = repo.path().join("frontend");
        let backend = repo.path().join("backend");
        std::fs::create_dir_all(&frontend).expect("frontend dir");
        std::fs::create_dir_all(&backend).expect("backend dir");
        std::fs::write(
            repo.path().join("README.md"),
            "# Demo\n\nRun `npm install` then `npm run dev`.\n",
        )
        .expect("readme");
        std::fs::write(
            frontend.join("package.json"),
            r#"{
  "name": "demo-frontend",
  "scripts": { "dev": "vite" },
  "dependencies": { "vite": "^5.0.0", "react": "^18.0.0" }
}"#,
        )
        .expect("frontend manifest");
        std::fs::write(
            frontend.join("index.html"),
            "<!doctype html><html><body>demo</body></html>",
        )
        .expect("frontend html");
        std::fs::write(backend.join("requirements.txt"), "fastapi\nuvicorn\n")
            .expect("backend requirements");
        std::fs::write(
            backend.join("main.py"),
            "from fastapi import FastAPI\napp = FastAPI()\n",
        )
        .expect("backend main");

        let plans = plan_repo_services(repo.path(), "Demo Repo", RepoServiceMode::Auto)
            .expect("repo services");

        assert_eq!(plans.len(), 2);
        assert!(plans.iter().any(|plan| {
            plan.relative_dir == "frontend"
                && plan.kind == RepoServiceKind::Frontend
                && plan
                    .entry_command
                    .as_deref()
                    .is_some_and(|command| command.contains("npm"))
        }));
        assert!(plans.iter().any(|plan| {
            plan.relative_dir == "backend"
                && plan.kind == RepoServiceKind::Backend
                && plan
                    .entry_command
                    .as_deref()
                    .is_some_and(|command| command.contains("uvicorn"))
        }));
    }

    #[test]
    fn plan_repo_services_detects_cargo_manifest_repo() {
        let repo = tempfile::tempdir().expect("temp repo");
        std::fs::write(
            repo.path().join("Cargo.toml"),
            r#"[package]
name = "demo-server"
version = "0.1.0"
edition = "2021"
"#,
        )
        .expect("cargo manifest");
        std::fs::create_dir_all(repo.path().join("src")).expect("src dir");
        std::fs::write(repo.path().join("src").join("main.rs"), "fn main() {}\n").expect("main");

        let plans = plan_repo_services(repo.path(), "Rust Demo", RepoServiceMode::Auto)
            .expect("repo services");

        assert_eq!(plans.len(), 1);
        assert_eq!(plans[0].kind, RepoServiceKind::Backend);
        assert_eq!(plans[0].entry_command.as_deref(), Some("cargo run"));
        assert_eq!(plans[0].detection_reason, "cargo manifest");
    }

    #[tokio::test]
    async fn repo_deploy_inflight_guard_blocks_matching_request_until_release() {
        let data_dir = tempfile::tempdir().expect("data dir");
        let fingerprint = repo_deploy_fingerprint(
            "https://github.com/example/repo-template",
            None,
            None,
            "repo-template",
            RepoServiceMode::Auto,
            RuntimePreference::Container,
            false,
            false,
            None,
        );
        let metadata = build_repo_deploy_lock_metadata(
            "bundle123",
            &fingerprint,
            "https://github.com/example/repo-template",
            None,
            None,
            "repo-template",
            RepoServiceMode::Auto,
            RuntimePreference::Container,
            false,
            false,
            None,
        );

        let guard = RepoDeployInFlightGuard::acquire(data_dir.path(), &fingerprint, &metadata)
            .await
            .expect("first lock should succeed");
        let error = RepoDeployInFlightGuard::acquire(data_dir.path(), &fingerprint, &metadata)
            .await
            .expect_err("second matching lock should be blocked");
        assert!(error.to_string().contains("already in progress"));

        drop(guard);

        RepoDeployInFlightGuard::acquire(data_dir.path(), &fingerprint, &metadata)
            .await
            .expect("lock should be released after guard drop");
    }

    #[tokio::test]
    async fn repo_deploy_inflight_guard_reclaims_stale_lock() {
        let data_dir = tempfile::tempdir().expect("data dir");
        let fingerprint = repo_deploy_fingerprint(
            "https://github.com/example/repo-template",
            Some("main"),
            Some("web"),
            "repo-template",
            RepoServiceMode::Frontend,
            RuntimePreference::Container,
            false,
            false,
            None,
        );
        let lock_dir = data_dir.path().join("repo-deployments").join(".inflight");
        tokio::fs::create_dir_all(&lock_dir)
            .await
            .expect("lock dir should exist");
        let lock_path = lock_dir.join(format!("{fingerprint}.json"));
        tokio::fs::write(
            &lock_path,
            serde_json::to_string_pretty(&serde_json::json!({
                "bundle_id": "stale1234",
                "started_at": "2026-04-08T00:00:00Z",
                "started_at_unix": chrono::Utc::now().timestamp()
                    - REPO_DEPLOY_INFLIGHT_STALE_SECS as i64
                    - 1,
            }))
            .expect("stale lock should serialize"),
        )
        .await
        .expect("stale lock should be written");

        let metadata = build_repo_deploy_lock_metadata(
            "bundle5678",
            &fingerprint,
            "https://github.com/example/repo-template",
            Some("main"),
            Some("web"),
            "repo-template",
            RepoServiceMode::Frontend,
            RuntimePreference::Container,
            false,
            false,
            None,
        );

        let guard = RepoDeployInFlightGuard::acquire(data_dir.path(), &fingerprint, &metadata)
            .await
            .expect("stale lock should be reclaimed");
        let persisted = read_repo_deploy_lock_metadata(&lock_path)
            .await
            .expect("fresh lock metadata should be readable");
        assert_eq!(
            persisted.get("bundle_id").and_then(|value| value.as_str()),
            Some("bundle5678")
        );
        drop(guard);
    }

    #[tokio::test]
    async fn repo_deploy_workspace_guard_cleans_failed_bundle_dir() {
        let bundle_dir = tempfile::tempdir()
            .expect("parent dir")
            .path()
            .join("repo-deployments")
            .join("bundle1234");
        let guard = RepoDeployWorkspaceGuard::create(bundle_dir.clone())
            .await
            .expect("bundle dir should be created");
        tokio::fs::write(bundle_dir.join("partial.txt"), "partial")
            .await
            .expect("partial file should be written");

        drop(guard);

        assert!(
            !bundle_dir.exists(),
            "failed repo deploy workspace should be cleaned up"
        );
    }

    #[test]
    fn should_deploy_repo_bundle_for_top_level_repo_request_only() {
        assert!(should_deploy_repo_bundle(&serde_json::json!({
            "repo_url": "https://github.com/example/repo-template"
        })));

        assert!(!should_deploy_repo_bundle(&serde_json::json!({
            "repo_url": "https://github.com/example/repo-template",
            "repo_bundle_id": "bundle1234",
            "files": {
                "package.json": "{}"
            }
        })));
    }

    #[tokio::test]
    async fn disabled_app_lists_as_disabled_and_not_running() {
        let app_dir = tempfile::tempdir().expect("app dir");
        let registry = AppRegistry::new();
        registry
            .register_stored(
                "demo".to_string(),
                StoredAppRegistration {
                    title: "Demo".to_string(),
                    app_dir: app_dir.path().to_path_buf(),
                    is_static: false,
                    access_key: "ak_demo".to_string(),
                    access_guard_enabled: false,
                    expose_public: false,
                    enabled: false,
                    last_accessed: None,
                },
            )
            .await;

        let apps = registry.list().await;
        let row = apps
            .iter()
            .find(|row| row.get("id").and_then(|v| v.as_str()) == Some("demo"))
            .expect("disabled app should be listed");

        assert_eq!(row.get("enabled").and_then(|v| v.as_bool()), Some(false));
        assert_eq!(row.get("running").and_then(|v| v.as_bool()), Some(false));
        assert_eq!(
            row.get("runtime_mode").and_then(|v| v.as_str()),
            Some("disabled")
        );
    }

    #[tokio::test]
    async fn set_enabled_persists_app_meta_flag() {
        let app_dir = tempfile::tempdir().expect("app dir");
        tokio::fs::write(app_dir.path().join(".app_meta.json"), "{}")
            .await
            .expect("meta should be written");

        let registry = AppRegistry::new();
        registry
            .register_stored(
                "demo".to_string(),
                StoredAppRegistration {
                    title: "Demo".to_string(),
                    app_dir: app_dir.path().to_path_buf(),
                    is_static: true,
                    access_key: "ak_demo".to_string(),
                    access_guard_enabled: false,
                    expose_public: false,
                    enabled: true,
                    last_accessed: None,
                },
            )
            .await;

        registry
            .set_enabled("demo", false)
            .await
            .expect("app should be disabled");

        let meta_raw = tokio::fs::read(app_dir.path().join(".app_meta.json"))
            .await
            .expect("meta should be readable");
        let meta: serde_json::Value =
            serde_json::from_slice(&meta_raw).expect("meta should parse as json");
        assert_eq!(meta.get("enabled").and_then(|v| v.as_bool()), Some(false));
    }

    #[tokio::test]
    async fn restore_from_disk_keeps_disabled_dynamic_app_disabled() {
        let config_dir = tempfile::tempdir().expect("config dir");
        let data_dir = tempfile::tempdir().expect("data dir");
        let app_dir = data_dir.path().join("apps").join("demo");
        tokio::fs::create_dir_all(&app_dir)
            .await
            .expect("app dir should exist");
        tokio::fs::write(
            app_dir.join(".app_meta.json"),
            serde_json::to_vec_pretty(&serde_json::json!({
                "title": "Demo",
                "entry_command": "python server.py",
                "enabled": false,
                "access_guard_enabled": false
            }))
            .expect("meta should serialize"),
        )
        .await
        .expect("meta should be written");

        let registry = AppRegistry::new();
        registry
            .restore_from_disk(config_dir.path(), data_dir.path(), &HashMap::new())
            .await;

        let apps = registry.list().await;
        let row = apps
            .iter()
            .find(|row| row.get("id").and_then(|v| v.as_str()) == Some("demo"))
            .expect("restored app should be listed");
        assert_eq!(row.get("enabled").and_then(|v| v.as_bool()), Some(false));
        assert_eq!(row.get("running").and_then(|v| v.as_bool()), Some(false));
        assert_eq!(
            row.get("runtime_mode").and_then(|v| v.as_str()),
            Some("disabled")
        );
    }

    #[tokio::test]
    async fn restore_from_disk_infers_dynamic_lifecycle_for_generated_bundle_metadata() {
        let config_dir = tempfile::tempdir().expect("config dir");
        let data_dir = tempfile::tempdir().expect("data dir");
        let app_dir = data_dir.path().join("apps").join("demo");
        tokio::fs::create_dir_all(&app_dir)
            .await
            .expect("app dir should exist");
        let files = generated_node_server_bundle_files();
        for (relative, value) in &files {
            let path = app_dir.join(relative);
            if let Some(parent) = path.parent() {
                tokio::fs::create_dir_all(parent)
                    .await
                    .expect("parent dir should exist");
            }
            tokio::fs::write(
                &path,
                value.as_str().expect("generated test files are text"),
            )
            .await
            .expect("managed app file should be written");
        }
        let managed_files = files.keys().cloned().collect::<Vec<_>>();
        tokio::fs::write(
            app_dir.join(".app_meta.json"),
            serde_json::to_vec_pretty(&serde_json::json!({
                "title": "Generated Bundle",
                "runtime_required": false,
                "commands": {
                    "install": "npm install --omit=dev",
                    "start": null
                },
                "managed_files": managed_files,
                "enabled": false,
                "access_guard_enabled": false
            }))
            .expect("meta should serialize"),
        )
        .await
        .expect("meta should be written");

        let registry = AppRegistry::new();
        registry
            .restore_from_disk(config_dir.path(), data_dir.path(), &HashMap::new())
            .await;

        let apps = registry.list().await;
        let row = apps
            .iter()
            .find(|row| row.get("id").and_then(|v| v.as_str()) == Some("demo"))
            .expect("restored app should be listed");
        assert_eq!(row.get("enabled").and_then(|v| v.as_bool()), Some(false));
        assert_eq!(row.get("running").and_then(|v| v.as_bool()), Some(false));
        assert_eq!(
            row.get("runtime_mode").and_then(|v| v.as_str()),
            Some("disabled")
        );

        let meta_raw = tokio::fs::read(app_dir.join(".app_meta.json"))
            .await
            .expect("healed meta should be readable");
        let meta: serde_json::Value =
            serde_json::from_slice(&meta_raw).expect("healed meta should be json");
        assert_eq!(
            app_meta_lifecycle_command(&meta, "entry_command").as_deref(),
            Some("npm run start")
        );
        assert_eq!(
            app_meta_lifecycle_command(&meta, "install_command").as_deref(),
            Some("npm install --omit=dev")
        );
        assert_eq!(
            meta.get("runtime_required").and_then(|v| v.as_bool()),
            Some(true)
        );
    }

    #[tokio::test]
    async fn restore_from_disk_skips_app_without_metadata() {
        let config_dir = tempfile::tempdir().expect("config dir");
        let data_dir = tempfile::tempdir().expect("data dir");
        let app_dir = data_dir.path().join("apps").join("demo");
        tokio::fs::create_dir_all(&app_dir)
            .await
            .expect("app dir should exist");
        tokio::fs::write(app_dir.join("index.html"), "<html>demo</html>")
            .await
            .expect("html should be written");

        let registry = AppRegistry::new();
        registry
            .restore_from_disk(config_dir.path(), data_dir.path(), &HashMap::new())
            .await;

        assert!(
            registry.list().await.is_empty(),
            "apps without metadata should be ignored during restore"
        );
    }

    #[tokio::test]
    async fn reconcile_on_boot_quarantines_corrupt_app_metadata() {
        let config_dir = tempfile::tempdir().expect("config dir");
        let data_dir = tempfile::tempdir().expect("data dir");
        let app_dir = data_dir.path().join("apps").join("demo");
        tokio::fs::create_dir_all(&app_dir)
            .await
            .expect("app dir should exist");
        tokio::fs::write(app_dir.join(".app_meta.json"), "{not-json")
            .await
            .expect("corrupt metadata should be written");

        let registry = AppRegistry::with_paths(
            config_dir.path().to_path_buf(),
            data_dir.path().to_path_buf(),
        );
        let report = registry.reconcile_on_boot().await;

        assert!(
            report.quarantined_app_ids.contains("demo"),
            "corrupt app should be quarantined on boot"
        );
        assert!(
            !app_dir.exists(),
            "corrupt app directory should leave the live apps directory"
        );

        let quarantine_root = data_dir.path().join("app_quarantine");
        let mut entries = tokio::fs::read_dir(&quarantine_root)
            .await
            .expect("quarantine root should exist");
        let mut moved_demo = false;
        while let Ok(Some(entry)) = entries.next_entry().await {
            let path = entry.path();
            if path
                .file_name()
                .and_then(|name| name.to_str())
                .is_some_and(|name| name.starts_with("demo-"))
            {
                moved_demo = true;
                break;
            }
        }
        assert!(
            moved_demo,
            "quarantine should contain the moved app directory"
        );
    }

    #[tokio::test]
    async fn app_deploy_updates_existing_static_app_by_app_id() {
        let config_dir = tempfile::tempdir().expect("config dir");
        let data_dir = tempfile::tempdir().expect("data dir");
        let registry = AppRegistry::new();
        let llm_env = HashMap::new();

        let initial_result = app_deploy(
            config_dir.path(),
            data_dir.path(),
            &serde_json::json!({
                "title": "arXiv Live Feed",
                "files": {
                    "index.html": "<!doctype html><html><body class=\"dark\">dark</body></html>"
                }
            }),
            &registry,
            &llm_env,
            None,
        )
        .await
        .expect("initial deploy should succeed");
        let initial_json: serde_json::Value =
            serde_json::from_str(&initial_result).expect("initial deploy json");
        let app_id = initial_json
            .get("app_id")
            .and_then(|value| value.as_str())
            .expect("initial app id")
            .to_string();
        let app_dir = data_dir.path().join("apps").join(&app_id);
        let initial_meta_raw = tokio::fs::read(app_dir.join(".app_meta.json"))
            .await
            .expect("initial meta should exist");
        let initial_meta: serde_json::Value =
            serde_json::from_slice(&initial_meta_raw).expect("initial meta json");
        let created_at = initial_meta
            .get("created_at")
            .and_then(|value| value.as_str())
            .expect("created_at should be recorded")
            .to_string();

        let updated_result = app_deploy(
            config_dir.path(),
            data_dir.path(),
            &serde_json::json!({
                "app_id": app_id,
                "title": "arXiv Live Feed Light",
                "files": {
                    "index.html": "<!doctype html><html><body class=\"light\">light</body></html>"
                }
            }),
            &registry,
            &llm_env,
            None,
        )
        .await
        .expect("update deploy should succeed");
        let updated_json: serde_json::Value =
            serde_json::from_str(&updated_result).expect("update deploy json");

        assert_eq!(
            updated_json.get("app_id").and_then(|value| value.as_str()),
            Some(app_id.as_str())
        );
        assert_eq!(
            updated_json
                .get("updated_existing")
                .and_then(|value| value.as_bool()),
            Some(true)
        );
        assert_eq!(
            updated_json.get("title").and_then(|value| value.as_str()),
            Some("arXiv Live Feed Light")
        );

        let apps = registry.list().await;
        assert_eq!(
            apps.len(),
            1,
            "updating in place should not create a second app"
        );
        let row = apps
            .iter()
            .find(|value| value.get("id").and_then(|entry| entry.as_str()) == Some(app_id.as_str()))
            .expect("updated app should still be registered");
        assert_eq!(
            row.get("title").and_then(|value| value.as_str()),
            Some("arXiv Live Feed Light")
        );

        let updated_html = tokio::fs::read_to_string(app_dir.join("index.html"))
            .await
            .expect("updated html should be readable");
        assert!(updated_html.contains("light"));
        assert!(!updated_html.contains("dark"));

        let updated_meta_raw = tokio::fs::read(app_dir.join(".app_meta.json"))
            .await
            .expect("updated meta should exist");
        let updated_meta: serde_json::Value =
            serde_json::from_slice(&updated_meta_raw).expect("updated meta json");
        assert_eq!(
            updated_meta.get("title").and_then(|value| value.as_str()),
            Some("arXiv Live Feed Light")
        );
        assert_eq!(
            updated_meta
                .get("created_at")
                .and_then(|value| value.as_str()),
            Some(created_at.as_str())
        );
        assert!(
            updated_meta
                .get("updated_at")
                .and_then(|value| value.as_str())
                .is_some(),
            "updated deployments should stamp updated_at"
        );
    }

    #[tokio::test]
    async fn app_deploy_skips_identical_static_app_unless_duplicate_allowed() {
        let config_dir = tempfile::tempdir().expect("config dir");
        let data_dir = tempfile::tempdir().expect("data dir");
        let registry = AppRegistry::new();
        let llm_env = HashMap::new();
        let args = serde_json::json!({
            "title": "Duplicate-aware app",
            "files": {
                "index.html": "<!doctype html><html><body>same</body></html>"
            }
        });

        let initial_result = app_deploy(
            config_dir.path(),
            data_dir.path(),
            &args,
            &registry,
            &llm_env,
            None,
        )
        .await
        .expect("initial deploy should succeed");
        let initial_json: serde_json::Value =
            serde_json::from_str(&initial_result).expect("initial deploy json");
        let initial_app_id = initial_json
            .get("app_id")
            .and_then(|value| value.as_str())
            .expect("initial app id")
            .to_string();

        let duplicate_result = app_deploy(
            config_dir.path(),
            data_dir.path(),
            &args,
            &registry,
            &llm_env,
            None,
        )
        .await
        .expect("duplicate deploy should reuse existing app");
        let duplicate_json: serde_json::Value =
            serde_json::from_str(&duplicate_result).expect("duplicate deploy json");
        assert_eq!(
            duplicate_json
                .get("status")
                .and_then(|value| value.as_str()),
            Some("duplicate_skipped")
        );
        assert_eq!(
            duplicate_json
                .get("app_id")
                .and_then(|value| value.as_str()),
            Some(initial_app_id.as_str())
        );
        assert_eq!(registry.list().await.len(), 1);

        let allowed_result = app_deploy(
            config_dir.path(),
            data_dir.path(),
            &serde_json::json!({
                "title": "Duplicate-aware app",
                "allow_duplicate": true,
                "files": {
                    "index.html": "<!doctype html><html><body>same</body></html>"
                }
            }),
            &registry,
            &llm_env,
            None,
        )
        .await
        .expect("explicit duplicate deploy should succeed");
        let allowed_json: serde_json::Value =
            serde_json::from_str(&allowed_result).expect("allowed deploy json");
        assert_eq!(
            allowed_json.get("status").and_then(|value| value.as_str()),
            Some("deployed")
        );
        assert_ne!(
            allowed_json.get("app_id").and_then(|value| value.as_str()),
            Some(initial_app_id.as_str())
        );
        assert_eq!(registry.list().await.len(), 2);
    }

    #[tokio::test]
    async fn app_deploy_skips_same_source_identity_even_when_markup_differs() {
        let config_dir = tempfile::tempdir().expect("config dir");
        let data_dir = tempfile::tempdir().expect("data dir");
        let registry = AppRegistry::new();
        let llm_env = HashMap::new();

        let initial_result = app_deploy(
            config_dir.path(),
            data_dir.path(),
            &serde_json::json!({
                "title": "Source-backed dashboard",
                "files": {
                    "index.html": "<!doctype html><html><body><a href=\"https://example.com/data\">Data</a><section>same data</section></body></html>"
                }
            }),
            &registry,
            &llm_env,
            None,
        )
        .await
        .expect("initial deploy should succeed");
        let initial_json: serde_json::Value =
            serde_json::from_str(&initial_result).expect("initial deploy json");
        let initial_app_id = initial_json
            .get("app_id")
            .and_then(|value| value.as_str())
            .expect("initial app id")
            .to_string();

        let duplicate_result = app_deploy(
            config_dir.path(),
            data_dir.path(),
            &serde_json::json!({
                "title": "Source-backed dashboard",
                "files": {
                    "index.html": "<!doctype html><html><head><style>body{color:blue}</style></head><body><a href=\"https://example.com/data\">Data</a><section>same data</section></body></html>"
                }
            }),
            &registry,
            &llm_env,
            None,
        )
        .await
        .expect("same-source deploy should reuse existing app");
        let duplicate_json: serde_json::Value =
            serde_json::from_str(&duplicate_result).expect("duplicate deploy json");
        assert_eq!(
            duplicate_json
                .get("status")
                .and_then(|value| value.as_str()),
            Some("duplicate_skipped")
        );
        assert_eq!(
            duplicate_json
                .get("app_id")
                .and_then(|value| value.as_str()),
            Some(initial_app_id.as_str())
        );
        assert_eq!(
            duplicate_json
                .get("duplicate_match")
                .and_then(|value| value.as_str()),
            Some("artifact_identity")
        );
        assert_eq!(registry.list().await.len(), 1);

        let changed_data_result = app_deploy(
            config_dir.path(),
            data_dir.path(),
            &serde_json::json!({
                "title": "Source-backed dashboard",
                "files": {
                    "index.html": "<!doctype html><html><body><a href=\"https://example.com/data\">Data</a><section>changed data</section></body></html>"
                }
            }),
            &registry,
            &llm_env,
            None,
        )
        .await
        .expect("changed source data should create a new app");
        let changed_json: serde_json::Value =
            serde_json::from_str(&changed_data_result).expect("changed deploy json");
        assert_eq!(
            changed_json.get("status").and_then(|value| value.as_str()),
            Some("deployed")
        );
        assert_eq!(registry.list().await.len(), 2);
    }

    #[tokio::test]
    async fn app_deploy_skips_same_structured_artifact_identity_across_titles() {
        let config_dir = tempfile::tempdir().expect("config dir");
        let data_dir = tempfile::tempdir().expect("data dir");
        let registry = AppRegistry::new();
        let llm_env = HashMap::new();
        let artifact_identity = serde_json::json!({
            "source_urls": ["https://example.com/pricing"],
            "source_fingerprint": "same-source-data"
        });

        let initial_result = app_deploy(
            config_dir.path(),
            data_dir.path(),
            &serde_json::json!({
                "title": "First title",
                "artifact_identity": artifact_identity,
                "files": {
                    "index.html": "<!doctype html><html><body><a href=\"https://example.com/pricing\">source</a><table><tr><td>A</td><td>$1</td></tr></table></body></html>"
                }
            }),
            &registry,
            &llm_env,
            None,
        )
        .await
        .expect("initial deploy should succeed");
        let initial_json: serde_json::Value =
            serde_json::from_str(&initial_result).expect("initial deploy json");
        let initial_app_id = initial_json
            .get("app_id")
            .and_then(|value| value.as_str())
            .expect("initial app id")
            .to_string();

        let duplicate_result = app_deploy(
            config_dir.path(),
            data_dir.path(),
            &serde_json::json!({
                "title": "Second title",
                "metadata": {
                    "artifact_identity": {
                        "source_urls": ["https://example.com/pricing"],
                        "source_fingerprint": "same-source-data"
                    }
                },
                "files": {
                    "index.html": "<!doctype html><html><body><a href=\"https://example.com/pricing\">source</a><table><tr><td>A</td><td>$1</td></tr></table><p>Different generated prose.</p></body></html>"
                }
            }),
            &registry,
            &llm_env,
            None,
        )
        .await
        .expect("same artifact identity should reuse existing app");
        let duplicate_json: serde_json::Value =
            serde_json::from_str(&duplicate_result).expect("duplicate deploy json");
        assert_eq!(
            duplicate_json
                .get("status")
                .and_then(|value| value.as_str()),
            Some("duplicate_skipped")
        );
        assert_eq!(
            duplicate_json
                .get("app_id")
                .and_then(|value| value.as_str()),
            Some(initial_app_id.as_str())
        );
        assert_eq!(registry.list().await.len(), 1);
    }

    #[test]
    fn unified_diff_patch_updates_only_target_lines() {
        let original = "one\ntwo\nthree\n";
        let patch = "@@ -1,3 +1,3 @@\n one\n-two\n+TWO\n three\n";
        let patched = apply_unified_diff_to_text(original, patch).expect("patch should apply");
        assert_eq!(patched, "one\nTWO\nthree\n");
    }

    #[test]
    fn unified_diff_patch_rejects_context_mismatch() {
        let original = "one\ntwo\nthree\n";
        let patch = "@@ -1,3 +1,3 @@\n one\n-not-two\n+TWO\n three\n";
        let error = apply_unified_diff_to_text(original, patch).expect_err("patch should fail");
        assert!(
            error.to_string().contains("did not match"),
            "expected context mismatch, got: {}",
            error
        );
    }

    #[tokio::test]
    async fn app_deploy_patch_mode_applies_diff_and_preserves_other_files() {
        let config_dir = tempfile::tempdir().expect("config dir");
        let data_dir = tempfile::tempdir().expect("data dir");
        let registry = AppRegistry::new();
        let llm_env = HashMap::new();

        let initial_result = app_deploy(
            config_dir.path(),
            data_dir.path(),
            &serde_json::json!({
                "title": "Patchable app",
                "files": {
                    "index.html": "<!doctype html><html><head><link rel=\"stylesheet\" href=\"style.css\"></head><body><script src=\"app.js\"></script></body></html>",
                    "style.css": "body { color: black; }\n",
                    "app.js": "const label = 'old';\nconsole.log(label);\n",
                    "old.txt": "remove me\n"
                }
            }),
            &registry,
            &llm_env,
            None,
        )
        .await
        .expect("initial deploy should succeed");
        let initial_json: serde_json::Value =
            serde_json::from_str(&initial_result).expect("initial deploy json");
        let app_id = initial_json
            .get("app_id")
            .and_then(|value| value.as_str())
            .expect("app id")
            .to_string();

        app_deploy(
            config_dir.path(),
            data_dir.path(),
            &serde_json::json!({
                "app_id": app_id,
                "mode": "patch",
                "file_patches": [{
                    "path": "app.js",
                    "patch": "@@ -1,2 +1,2 @@\n-const label = 'old';\n+const label = 'new';\n console.log(label);\n"
                }],
                "delete_paths": ["old.txt"]
            }),
            &registry,
            &llm_env,
            None,
        )
        .await
        .expect("patch deploy should succeed");

        let app_dir = data_dir.path().join("apps").join(&app_id);
        let app_js = tokio::fs::read_to_string(app_dir.join("app.js"))
            .await
            .expect("patched js should be readable");
        let style_css = tokio::fs::read_to_string(app_dir.join("style.css"))
            .await
            .expect("unchanged css should remain readable");
        assert!(app_js.contains("'new'"));
        assert_eq!(style_css, "body { color: black; }\n");
        assert!(
            tokio::fs::metadata(app_dir.join("old.txt")).await.is_err(),
            "explicit delete_paths should remove the old file"
        );
    }

    #[tokio::test]
    async fn app_deploy_patch_mode_requires_existing_app_id() {
        let config_dir = tempfile::tempdir().expect("config dir");
        let data_dir = tempfile::tempdir().expect("data dir");
        let registry = AppRegistry::new();
        let llm_env = HashMap::new();

        let error = app_deploy(
            config_dir.path(),
            data_dir.path(),
            &serde_json::json!({
                "mode": "patch",
                "file_patches": [{
                    "path": "index.html",
                    "patch": "@@ -1 +1 @@\n-old\n+new\n"
                }]
            }),
            &registry,
            &llm_env,
            None,
        )
        .await
        .expect_err("patch mode without app_id must fail");

        assert!(
            error.to_string().contains("requires app_id"),
            "unexpected error: {}",
            error
        );
    }

    #[tokio::test]
    async fn app_deploy_rejects_static_bundle_with_missing_stylesheet() {
        let config_dir = tempfile::tempdir().expect("config dir");
        let data_dir = tempfile::tempdir().expect("data dir");
        let registry = AppRegistry::new();
        let llm_env = HashMap::new();

        let error = app_deploy(
            config_dir.path(),
            data_dir.path(),
            &serde_json::json!({
                "title": "Broken static app",
                "files": {
                    "index.html": "<!doctype html><html><head><link rel=\"stylesheet\" href=\"styles.css\"></head><body>demo</body></html>"
                }
            }),
            &registry,
            &llm_env,
            None,
        )
        .await
        .expect_err("missing local stylesheet should reject the static bundle");

        let message = error.to_string();
        assert!(
            message.contains("styles.css"),
            "error should name the missing asset, got: {}",
            message
        );
        assert!(
            registry.list().await.is_empty(),
            "invalid static bundles must not be registered"
        );
    }

    #[tokio::test]
    async fn app_deploy_rejects_static_bundle_with_unclosed_style_block() {
        let config_dir = tempfile::tempdir().expect("config dir");
        let data_dir = tempfile::tempdir().expect("data dir");
        let registry = AppRegistry::new();
        let llm_env = HashMap::new();

        let error = app_deploy(
            config_dir.path(),
            data_dir.path(),
            &serde_json::json!({
                "title": "Truncated static app",
                "files": {
                    "index.html": "<!doctype html><html><head><style>.hero { color: white;"
                }
            }),
            &registry,
            &llm_env,
            None,
        )
        .await
        .expect_err("unclosed style block should reject the static bundle");

        let message = error.to_string();
        assert!(
            message.contains("unclosed <style>"),
            "error should name the malformed raw text block, got: {}",
            message
        );
        assert!(
            registry.list().await.is_empty(),
            "malformed static bundles must not be registered"
        );
    }
}
