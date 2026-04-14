use super::*;
use std::collections::{HashMap, VecDeque};
use std::path::{Path as FsPath, PathBuf};
use tokio::io::{AsyncBufReadExt, BufReader};
use tokio::process::Child;
use tokio::sync::{Mutex, RwLock};

const APPLICATION_LOG_LIMIT: usize = 200;
const APPLICATION_LOG_TAIL: usize = 40;
#[derive(Clone, Default)]
pub(super) struct ApplicationLauncherRegistry {
    runtimes: Arc<RwLock<HashMap<String, Arc<ApplicationLauncherRuntime>>>>,
}

struct ApplicationLauncherRuntime {
    child: Mutex<Option<Child>>,
    status: RwLock<ApplicationLauncherStatus>,
}

impl ApplicationLauncherRuntime {
    fn new() -> Self {
        Self {
            child: Mutex::new(None),
            status: RwLock::new(ApplicationLauncherStatus::default()),
        }
    }
}

#[derive(Clone, Debug, Default)]
struct ApplicationLauncherStatus {
    state: String,
    mode: String,
    model: Option<String>,
    command: Option<String>,
    started_at: Option<String>,
    ended_at: Option<String>,
    exit_code: Option<i32>,
    message: Option<String>,
    logs: VecDeque<String>,
}

#[derive(Serialize)]
pub(super) struct ApplicationLaunchersResponse {
    runtime: ApplicationRuntimeSummary,
    applications: Vec<ApplicationLauncherSummary>,
}

#[derive(Serialize)]
struct ApplicationRuntimeSummary {
    ollama_cli_available: bool,
    ollama_version: Option<String>,
    ollama_base_url: Option<String>,
    ollama_base_url_source: String,
    ollama_reachable: bool,
    detail: String,
    docker_runtime: bool,
}

#[derive(Serialize)]
struct ApplicationLauncherSummary {
    id: String,
    label: String,
    tagline: String,
    description: String,
    docs_url: String,
    runtime_launch_command: String,
    runtime_config_command: Option<String>,
    host_launch_command: String,
    host_config_command: Option<String>,
    supports_config: bool,
    terminal_first: bool,
    model_hint: String,
    aliases: Vec<String>,
    recommended_models: Vec<String>,
    runtime: ApplicationLauncherRuntimeSnapshot,
}

#[derive(Clone, Serialize, Default)]
struct ApplicationLauncherRuntimeSnapshot {
    state: String,
    mode: String,
    model: Option<String>,
    command: Option<String>,
    started_at: Option<String>,
    ended_at: Option<String>,
    exit_code: Option<i32>,
    message: Option<String>,
    logs: Vec<String>,
}

#[derive(Debug, Deserialize)]
pub(super) struct LaunchApplicationRequest {
    #[serde(default)]
    mode: Option<String>,
    #[serde(default)]
    model: Option<String>,
}

#[derive(Clone, Copy)]
enum ApplicationLaunchMode {
    Launch,
    Config,
}

impl ApplicationLaunchMode {
    fn as_str(self) -> &'static str {
        match self {
            Self::Launch => "launch",
            Self::Config => "config",
        }
    }
}

struct ApplicationLauncherDefinition {
    id: &'static str,
    launch_slug: &'static str,
    label: &'static str,
    tagline: &'static str,
    description: &'static str,
    docs_url: &'static str,
    supports_config: bool,
    terminal_first: bool,
    model_hint: &'static str,
    aliases: &'static [&'static str],
    recommended_models: &'static [&'static str],
}

const APPLICATION_LAUNCHERS: &[ApplicationLauncherDefinition] = &[
    ApplicationLauncherDefinition {
        id: "claude",
        launch_slug: "claude",
        label: "Claude Code",
        tagline: "Terminal-first coding assistant via Ollama Launch.",
        description: "Best for interactive coding sessions in a terminal. {PRODUCT_NAME} can prepare and launch it against your configured Ollama runtime.",
        docs_url: "https://ollama.com/blog/launch",
        supports_config: true,
        terminal_first: true,
        model_hint: "Optional model override, e.g. minimax-m2.5:cloud or kimi-k2.5:cloud",
        aliases: &["claude-code"],
        recommended_models: &[
            "minimax-m2.5:cloud",
            "kimi-k2.5:cloud",
            "glm-5:cloud",
            "qwen3.5:cloud",
        ],
    },
    ApplicationLauncherDefinition {
        id: "codex",
        launch_slug: "codex",
        label: "Codex",
        tagline: "OpenAI Codex CLI through Ollama Launch.",
        description: "Good for terminal-centric code edits and reviews. {PRODUCT_NAME} can run it on the server or generate the exact command for your own terminal.",
        docs_url: "https://ollama.com/blog/launch",
        supports_config: true,
        terminal_first: true,
        model_hint: "Optional model override, e.g. gpt-oss:120b or minimax-m2.5:cloud",
        aliases: &[],
        recommended_models: &["gpt-oss:120b", "minimax-m2.5:cloud"],
    },
    ApplicationLauncherDefinition {
        id: "opencode",
        launch_slug: "opencode",
        label: "OpenCode",
        tagline: "Open-source coding assistant with wide model support.",
        description: "A terminal agent that works especially well with larger-context models. {PRODUCT_NAME} can launch it with the same Ollama runtime you already configured.",
        docs_url: "https://ollama.com/blog/launch",
        supports_config: true,
        terminal_first: true,
        model_hint: "Use a model with 64K context or more when possible.",
        aliases: &[],
        recommended_models: &["qwen3.5:cloud", "kimi-k2.5:cloud", "glm-5:cloud"],
    },
];

impl ApplicationLauncherRegistry {
    async fn runtime_for(&self, app_id: &str) -> Arc<ApplicationLauncherRuntime> {
        if let Some(existing) = self.runtimes.read().await.get(app_id).cloned() {
            return existing;
        }
        let mut guard = self.runtimes.write().await;
        guard
            .entry(app_id.to_string())
            .or_insert_with(|| Arc::new(ApplicationLauncherRuntime::new()))
            .clone()
    }

    async fn snapshot_for(&self, app_id: &str) -> ApplicationLauncherRuntimeSnapshot {
        let runtime = self.runtime_for(app_id).await;
        poll_application_runtime(&runtime).await;
        status_snapshot(&runtime).await
    }

    async fn stop(&self, app_id: &str) -> Result<()> {
        let runtime = self.runtime_for(app_id).await;
        let mut child_guard = runtime.child.lock().await;
        let Some(child) = child_guard.as_mut() else {
            anyhow::bail!("No active launch for {}", app_id);
        };
        child.kill().await?;
        let ended_at = chrono::Utc::now().to_rfc3339();
        {
            let mut status = runtime.status.write().await;
            status.state = "stopped".to_string();
            status.ended_at = Some(ended_at);
            status.message = Some(format!("Stopped by {}.", crate::branding::PRODUCT_NAME));
            push_application_log(
                &mut status.logs,
                format!(
                    "[{}] Stopped by {}.",
                    crate::branding::PRODUCT_SLUG,
                    crate::branding::PRODUCT_NAME
                ),
            );
        }
        *child_guard = None;
        Ok(())
    }
}

pub(super) async fn list_application_launchers(
    State(state): State<AppState>,
) -> Json<ApplicationLaunchersResponse> {
    let runtime = gather_application_runtime_summary(&state).await;
    let mut applications = Vec::with_capacity(APPLICATION_LAUNCHERS.len());
    for launcher in APPLICATION_LAUNCHERS {
        let snapshot = state.application_registry.snapshot_for(launcher.id).await;
        let runtime_launch_command =
            build_runtime_launcher_command(launcher, ApplicationLaunchMode::Launch, None);
        let runtime_config_command = launcher
            .supports_config
            .then(|| build_runtime_launcher_command(launcher, ApplicationLaunchMode::Config, None));
        let host_launch_command = build_host_launcher_command(
            runtime.docker_runtime,
            runtime.ollama_base_url.as_deref(),
            launcher,
            ApplicationLaunchMode::Launch,
            None,
        );
        let host_config_command = launcher.supports_config.then(|| {
            build_host_launcher_command(
                runtime.docker_runtime,
                runtime.ollama_base_url.as_deref(),
                launcher,
                ApplicationLaunchMode::Config,
                None,
            )
        });
        applications.push(ApplicationLauncherSummary {
            id: launcher.id.to_string(),
            label: launcher.label.to_string(),
            tagline: launcher.tagline.to_string(),
            description: launcher
                .description
                .replace("{PRODUCT_NAME}", crate::branding::PRODUCT_NAME),
            docs_url: launcher.docs_url.to_string(),
            runtime_launch_command,
            runtime_config_command,
            host_launch_command,
            host_config_command,
            supports_config: launcher.supports_config,
            terminal_first: launcher.terminal_first,
            model_hint: launcher.model_hint.to_string(),
            aliases: launcher
                .aliases
                .iter()
                .map(|value| value.to_string())
                .collect(),
            recommended_models: launcher
                .recommended_models
                .iter()
                .map(|value| value.to_string())
                .collect(),
            runtime: snapshot,
        });
    }

    Json(ApplicationLaunchersResponse {
        runtime,
        applications,
    })
}

pub(super) async fn launch_application(
    State(state): State<AppState>,
    Path(app_id): Path<String>,
    Json(request): Json<LaunchApplicationRequest>,
) -> Response {
    let Some(definition) = launcher_definition(&app_id) else {
        return (
            StatusCode::NOT_FOUND,
            Json(serde_json::json!({ "error": "Unknown application launcher" })),
        )
            .into_response();
    };

    let mode = match parse_launch_mode(request.mode.as_deref(), definition) {
        Ok(value) => value,
        Err(error) => {
            return (
                StatusCode::BAD_REQUEST,
                Json(serde_json::json!({ "error": error })),
            )
                .into_response();
        }
    };

    let runtime_summary = gather_application_runtime_summary(&state).await;
    if !runtime_summary.ollama_cli_available {
        return (
            StatusCode::BAD_REQUEST,
            Json(serde_json::json!({
                "error": format!(
                    "Ollama CLI is not installed in this {} runtime.",
                    crate::branding::PRODUCT_NAME
                ),
                "detail": runtime_summary.detail
            })),
        )
            .into_response();
    }
    if !runtime_summary.ollama_reachable {
        return (
            StatusCode::BAD_REQUEST,
            Json(serde_json::json!({
                "error": format!(
                    "Ollama is not reachable from this {} runtime.",
                    crate::branding::PRODUCT_NAME
                ),
                "detail": runtime_summary.detail
            })),
        )
            .into_response();
    }

    let runtime = state.application_registry.runtime_for(definition.id).await;
    poll_application_runtime(&runtime).await;

    {
        let child_guard = runtime.child.lock().await;
        if child_guard.is_some() {
            return (
                StatusCode::CONFLICT,
                Json(serde_json::json!({
                    "error": format!(
                        "{} is already running in this {} runtime.",
                        definition.label,
                        crate::branding::PRODUCT_NAME
                    )
                })),
            )
                .into_response();
        }
    }

    let requested_model = request
        .model
        .as_deref()
        .map(str::trim)
        .filter(|value| !value.is_empty())
        .map(ToString::to_string);
    let effective_model = if matches!(mode, ApplicationLaunchMode::Launch) {
        requested_model.clone()
    } else {
        None
    };
    let command_text = build_runtime_launcher_command(definition, mode, effective_model.as_deref());
    let launcher_root = {
        let agent = state.agent.read().await;
        agent.data_dir().join("applications")
    };
    let shell_context =
        build_launcher_shell_context(&launcher_root, runtime_summary.ollama_base_url.as_deref());

    if let Err(error) = ensure_launcher_directories(&launcher_root) {
        return (
            StatusCode::INTERNAL_SERVER_ERROR,
            Json(serde_json::json!({
                "error": format!("Failed to prepare launcher directories: {}", error)
            })),
        )
            .into_response();
    }

    let mut command = tokio::process::Command::new(ollama_cli_binary());
    apply_launcher_env(&mut command, &shell_context);
    command
        .arg("launch")
        .arg(definition.launch_slug)
        .stdout(std::process::Stdio::piped())
        .stderr(std::process::Stdio::piped())
        .stdin(std::process::Stdio::null())
        .kill_on_drop(true);
    if matches!(mode, ApplicationLaunchMode::Config) {
        command.arg("--config");
    }
    if let Some(model) = effective_model.as_deref() {
        command.arg("--model").arg(model);
    }

    match command.spawn() {
        Ok(mut child) => {
            let stdout = child.stdout.take();
            let stderr = child.stderr.take();
            {
                let mut child_guard = runtime.child.lock().await;
                *child_guard = Some(child);
            }
            {
                let mut status = runtime.status.write().await;
                *status = ApplicationLauncherStatus {
                    state: "running".to_string(),
                    mode: mode.as_str().to_string(),
                    model: effective_model.clone(),
                    command: Some(command_text.clone()),
                    started_at: Some(chrono::Utc::now().to_rfc3339()),
                    ended_at: None,
                    exit_code: None,
                    message: Some(format!(
                        "Launched from {}. These tools are terminal-first, so copy the command to your own terminal if you need the full interactive UI.",
                        crate::branding::PRODUCT_NAME
                    )),
                    logs: VecDeque::new(),
                };
                push_application_log(
                    &mut status.logs,
                    format!("[agentark] Launching {}", command_text),
                );
                if let Some(base_url) = shell_context.ollama_base_url.as_deref() {
                    push_application_log(
                        &mut status.logs,
                        format!("[agentark] OLLAMA_HOST={}", base_url),
                    );
                }
            }
            if let Some(stdout) = stdout {
                spawn_application_log_reader(runtime.clone(), stdout, "stdout");
            }
            if let Some(stderr) = stderr {
                spawn_application_log_reader(runtime.clone(), stderr, "stderr");
            }

            let snapshot = state.application_registry.snapshot_for(definition.id).await;
            (
                StatusCode::OK,
                Json(serde_json::json!({
                    "ok": true,
                    "message": format!("{} launch started.", definition.label),
                    "command": command_text,
                    "runtime": snapshot,
                })),
            )
                .into_response()
        }
        Err(error) => (
            StatusCode::INTERNAL_SERVER_ERROR,
            Json(serde_json::json!({
                "error": format!("Failed to spawn {}: {}", definition.label, error),
            })),
        )
            .into_response(),
    }
}

pub(super) async fn stop_application(
    State(state): State<AppState>,
    Path(app_id): Path<String>,
) -> Response {
    let Some(definition) = launcher_definition(&app_id) else {
        return (
            StatusCode::NOT_FOUND,
            Json(serde_json::json!({ "error": "Unknown application launcher" })),
        )
            .into_response();
    };

    match state.application_registry.stop(definition.id).await {
        Ok(()) => {
            let snapshot = state.application_registry.snapshot_for(definition.id).await;
            (
                StatusCode::OK,
                Json(serde_json::json!({
                    "ok": true,
                    "message": format!("Stopped {}.", definition.label),
                    "runtime": snapshot,
                })),
            )
                .into_response()
        }
        Err(error) => (
            StatusCode::BAD_REQUEST,
            Json(serde_json::json!({
                "error": error.to_string(),
            })),
        )
            .into_response(),
    }
}

fn launcher_definition(app_id: &str) -> Option<&'static ApplicationLauncherDefinition> {
    APPLICATION_LAUNCHERS.iter().find(|launcher| {
        launcher.id == app_id
            || launcher
                .aliases
                .iter()
                .any(|alias| alias.eq_ignore_ascii_case(app_id))
    })
}

fn parse_launch_mode(
    raw_mode: Option<&str>,
    launcher: &ApplicationLauncherDefinition,
) -> std::result::Result<ApplicationLaunchMode, String> {
    match raw_mode.map(str::trim).filter(|value| !value.is_empty()) {
        None => Ok(ApplicationLaunchMode::Launch),
        Some(value) if value.eq_ignore_ascii_case("launch") => Ok(ApplicationLaunchMode::Launch),
        Some(value) if value.eq_ignore_ascii_case("config") && launcher.supports_config => {
            Ok(ApplicationLaunchMode::Config)
        }
        Some(value) if value.eq_ignore_ascii_case("config") => Err(format!(
            "{} does not expose a documented config-only command.",
            launcher.label
        )),
        Some(_) => Err("Unsupported launcher mode. Use 'launch' or 'config'.".to_string()),
    }
}

fn build_runtime_launcher_command(
    launcher: &ApplicationLauncherDefinition,
    mode: ApplicationLaunchMode,
    model: Option<&str>,
) -> String {
    let mut command = format!("ollama launch {}", launcher.launch_slug);
    if matches!(mode, ApplicationLaunchMode::Config) {
        command.push_str(" --config");
    }
    if let Some(model) = model.map(str::trim).filter(|value| !value.is_empty()) {
        command.push_str(" --model ");
        command.push_str(model);
    }
    command
}

fn build_host_launcher_command(
    docker_runtime: bool,
    ollama_base_url: Option<&str>,
    launcher: &ApplicationLauncherDefinition,
    mode: ApplicationLaunchMode,
    model: Option<&str>,
) -> String {
    let runtime_command = build_runtime_launcher_command(launcher, mode, model);
    if !docker_runtime {
        return runtime_command;
    }

    let mut command = String::from("docker exec -it");
    if let Some(base_url) = ollama_base_url
        .map(str::trim)
        .filter(|value| !value.is_empty())
    {
        command.push_str(" -e OLLAMA_HOST=");
        command.push_str(base_url);
    }
    command.push_str(" agentark ");
    command.push_str(&runtime_command);
    command
}

fn ollama_cli_binary() -> String {
    std::env::var("AGENTARK_OLLAMA_CLI")
        .ok()
        .map(|value| value.trim().to_string())
        .filter(|value| !value.is_empty())
        .unwrap_or_else(|| "ollama".to_string())
}

fn ollama_cli_available() -> bool {
    let binary = ollama_cli_binary();
    if binary.contains('/') || binary.contains('\\') {
        return std::path::Path::new(&binary).exists();
    }
    std::process::Command::new(binary)
        .arg("--version")
        .stdout(std::process::Stdio::null())
        .stderr(std::process::Stdio::null())
        .status()
        .map(|status| status.success())
        .unwrap_or(false)
}

async fn ollama_cli_version() -> Option<String> {
    if !ollama_cli_available() {
        return None;
    }
    let binary = ollama_cli_binary();
    let output = tokio::process::Command::new(binary)
        .arg("--version")
        .stdout(std::process::Stdio::piped())
        .stderr(std::process::Stdio::null())
        .output()
        .await
        .ok()?;
    if !output.status.success() {
        return None;
    }
    let stdout = String::from_utf8_lossy(&output.stdout).trim().to_string();
    if stdout.is_empty() {
        None
    } else {
        Some(stdout)
    }
}

async fn gather_application_runtime_summary(state: &AppState) -> ApplicationRuntimeSummary {
    let agent = state.agent.read().await;
    let (base_url, source) = derive_ollama_base_url(&agent.config);
    drop(agent);
    let cli_available = ollama_cli_available();
    let version = ollama_cli_version().await;
    let reachable = if let Some(url) = base_url.as_deref() {
        ollama_reachable(url).await
    } else {
        false
    };
    let docker_runtime = std::path::Path::new("/.dockerenv").exists();
    let detail = if !cli_available {
        format!(
            "Install the Ollama CLI in this {} runtime before using application launchers.",
            crate::branding::PRODUCT_NAME
        )
    } else if let Some(url) = base_url.as_deref() {
        if reachable {
            format!("Ollama is reachable at {}.", url)
        } else if docker_runtime && is_local_ollama_url(url) {
            format!(
                "Ollama CLI is installed, but {} points at container localhost. In Docker, use an Ollama URL reachable from the container such as http://host.docker.internal:11434.",
                url
            )
        } else {
            format!(
                "Ollama CLI is installed, but the runtime could not reach {}.",
                url
            )
        }
    } else {
        "No Ollama base URL is configured yet. Add an Ollama model in Settings or set OLLAMA_HOST."
            .to_string()
    };

    ApplicationRuntimeSummary {
        ollama_cli_available: cli_available,
        ollama_version: version,
        ollama_base_url: base_url,
        ollama_base_url_source: source,
        ollama_reachable: reachable,
        detail,
        docker_runtime,
    }
}

fn derive_ollama_base_url(config: &crate::core::config::AgentConfig) -> (Option<String>, String) {
    if let crate::core::LlmProvider::Ollama { base_url, .. } = &config.llm {
        return (Some(normalize_base_url(base_url)), "primary".to_string());
    }
    for slot in &config.model_pool.slots {
        if !slot.enabled {
            continue;
        }
        if let crate::core::LlmProvider::Ollama { base_url, .. } = &slot.provider {
            return (
                Some(normalize_base_url(base_url)),
                format!("model slot: {}", slot.label),
            );
        }
    }
    if let Some(crate::core::LlmProvider::Ollama { base_url, .. }) = &config.llm_fallback {
        return (Some(normalize_base_url(base_url)), "fallback".to_string());
    }
    if let Ok(env_host) = std::env::var("OLLAMA_HOST") {
        let trimmed = env_host.trim();
        if !trimmed.is_empty() {
            return (Some(normalize_base_url(trimmed)), "environment".to_string());
        }
    }
    (None, String::new())
}

fn normalize_base_url(value: &str) -> String {
    value.trim().trim_end_matches('/').to_string()
}

fn is_local_ollama_url(url: &str) -> bool {
    let lower = url.trim().to_ascii_lowercase();
    lower.contains("://localhost")
        || lower.contains("://127.0.0.1")
        || lower.contains("://[::1]")
        || lower.starts_with("localhost:")
        || lower.starts_with("127.0.0.1:")
}

async fn ollama_reachable(base_url: &str) -> bool {
    let client = reqwest::Client::builder()
        .timeout(Duration::from_secs(3))
        .build()
        .ok();
    let Some(client) = client else {
        return false;
    };
    client
        .get(format!("{}/api/tags", base_url.trim_end_matches('/')))
        .send()
        .await
        .map(|response| response.status().is_success())
        .unwrap_or(false)
}

struct LauncherShellContext {
    ollama_base_url: Option<String>,
    home: String,
    xdg_config_home: String,
    xdg_data_home: String,
    xdg_cache_home: String,
    npm_prefix: String,
    path: String,
}

fn build_launcher_shell_context(
    launcher_root: &FsPath,
    ollama_base_url: Option<&str>,
) -> LauncherShellContext {
    let home = launcher_root.join("home");
    let xdg_config_home = launcher_root.join("config");
    let xdg_data_home = launcher_root.join("data");
    let xdg_cache_home = launcher_root.join("cache");
    let npm_prefix = launcher_root.join("npm-global");
    let npm_bin = npm_prefix.join(if cfg!(windows) { "" } else { "bin" });
    let mut path_entries: Vec<PathBuf> = std::env::var_os("PATH")
        .map(|value| std::env::split_paths(&value).collect())
        .unwrap_or_default();
    if !npm_bin.as_os_str().is_empty() {
        path_entries.insert(0, npm_bin);
    }
    let path = std::env::join_paths(path_entries)
        .map(|value| value.to_string_lossy().into_owned())
        .unwrap_or_default();
    LauncherShellContext {
        ollama_base_url: ollama_base_url.map(normalize_base_url),
        home: home.to_string_lossy().into_owned(),
        xdg_config_home: xdg_config_home.to_string_lossy().into_owned(),
        xdg_data_home: xdg_data_home.to_string_lossy().into_owned(),
        xdg_cache_home: xdg_cache_home.to_string_lossy().into_owned(),
        npm_prefix: npm_prefix.to_string_lossy().into_owned(),
        path,
    }
}

fn ensure_launcher_directories(launcher_root: &FsPath) -> Result<()> {
    for dir in [
        launcher_root.to_path_buf(),
        launcher_root.join("home"),
        launcher_root.join("config"),
        launcher_root.join("data"),
        launcher_root.join("cache"),
        launcher_root.join("npm-global"),
    ] {
        std::fs::create_dir_all(dir)?;
    }
    Ok(())
}

fn apply_launcher_env(command: &mut tokio::process::Command, context: &LauncherShellContext) {
    if let Some(base_url) = context.ollama_base_url.as_deref() {
        command.env("OLLAMA_HOST", base_url);
    }
    command
        .env("HOME", &context.home)
        .env("XDG_CONFIG_HOME", &context.xdg_config_home)
        .env("XDG_DATA_HOME", &context.xdg_data_home)
        .env("XDG_CACHE_HOME", &context.xdg_cache_home)
        .env("NPM_CONFIG_PREFIX", &context.npm_prefix)
        .env("PATH", &context.path)
        .env("TERM", "xterm-256color")
        .env("CI", "1");
}

fn spawn_application_log_reader<R>(
    runtime: Arc<ApplicationLauncherRuntime>,
    reader: R,
    source: &'static str,
) where
    R: tokio::io::AsyncRead + Unpin + Send + 'static,
{
    tokio::spawn(async move {
        let mut lines = BufReader::new(reader).lines();
        loop {
            match lines.next_line().await {
                Ok(Some(line)) => {
                    let mut status = runtime.status.write().await;
                    push_application_log(&mut status.logs, format!("[{}] {}", source, line));
                }
                Ok(None) => break,
                Err(error) => {
                    let mut status = runtime.status.write().await;
                    push_application_log(
                        &mut status.logs,
                        format!("[{}] log read error: {}", source, error),
                    );
                    break;
                }
            }
        }
    });
}

async fn poll_application_runtime(runtime: &Arc<ApplicationLauncherRuntime>) {
    let mut child_guard = runtime.child.lock().await;
    let Some(child) = child_guard.as_mut() else {
        return;
    };
    match child.try_wait() {
        Ok(Some(exit)) => {
            let exit_code = exit.code();
            let mut status = runtime.status.write().await;
            status.exit_code = exit_code;
            status.ended_at = Some(chrono::Utc::now().to_rfc3339());
            status.state = if exit.success() {
                "completed".to_string()
            } else {
                "failed".to_string()
            };
            status.message = Some(if exit.success() {
                "Process exited normally.".to_string()
            } else {
                format!("Process exited with {:?}", exit_code)
            });
            push_application_log(
                &mut status.logs,
                format!("[agentark] Process exited with {:?}", exit_code),
            );
            *child_guard = None;
        }
        Ok(None) => {}
        Err(error) => {
            let mut status = runtime.status.write().await;
            status.ended_at = Some(chrono::Utc::now().to_rfc3339());
            status.state = "failed".to_string();
            status.message = Some(format!("Failed to poll process state: {}", error));
            push_application_log(
                &mut status.logs,
                format!("[agentark] Failed to poll process state: {}", error),
            );
            *child_guard = None;
        }
    }
}

async fn status_snapshot(
    runtime: &Arc<ApplicationLauncherRuntime>,
) -> ApplicationLauncherRuntimeSnapshot {
    let status = runtime.status.read().await;
    ApplicationLauncherRuntimeSnapshot {
        state: if status.state.is_empty() {
            "idle".to_string()
        } else {
            status.state.clone()
        },
        mode: status.mode.clone(),
        model: status.model.clone(),
        command: status.command.clone(),
        started_at: status.started_at.clone(),
        ended_at: status.ended_at.clone(),
        exit_code: status.exit_code,
        message: status.message.clone(),
        logs: status
            .logs
            .iter()
            .rev()
            .take(APPLICATION_LOG_TAIL)
            .cloned()
            .collect::<Vec<_>>()
            .into_iter()
            .rev()
            .collect(),
    }
}

fn push_application_log(logs: &mut VecDeque<String>, line: impl Into<String>) {
    logs.push_back(line.into());
    while logs.len() > APPLICATION_LOG_LIMIT {
        logs.pop_front();
    }
}
