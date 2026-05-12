//! AgentArk - A personal AI Agent OS for memory, agents, apps, automations, and reviewable actions
//!
//! Features:
//! - Daily briefs, reminders, and channel delivery
//! - Durable memory for learned facts, preferences, user data, and knowledge
//! - Secure secrets, approvals, and cryptographic execution proofs
//! - Sandboxed action execution (WASM + Docker)
//! - Optional power features like tasks, apps, and sub-agents
//! - Native GUI (egui) + Telegram integration
//! - Local-first HTTP API

#![recursion_limit = "256"]

mod actions;
mod branding;
mod channels;
mod cli;
mod clients;
mod core;
mod crypto;
mod custom_apis;
mod custom_messaging_channels;
mod docs;
mod executor;
mod extension_packs;
mod hooks;
mod identity;
mod integrations;
mod mcp;
mod metrics;
mod plugins;
mod proofs;
mod runtime;
mod safety;
mod security;
mod sentinel;
mod storage;
mod workspace;

#[cfg(feature = "gui")]
mod gui;

use anyhow::Result;
use clap::{Parser, ValueEnum};
use crossterm::event::{self, Event, KeyCode, KeyEventKind, KeyModifiers};
use crossterm::terminal::{disable_raw_mode, enable_raw_mode};
use std::io::{BufRead, IsTerminal, Read, Write};
use std::path::{Path, PathBuf};
use tracing_subscriber::fmt::{format::Writer, time::FormatTime};
use tracing_subscriber::{layer::SubscriberExt, util::SubscriberInitExt};

#[derive(Parser, Debug)]
#[command(name = "agentark")]
#[command(author, version, about, long_about = None)]
struct Args {
    /// Run in headless mode (no GUI)
    #[arg(long)]
    headless: bool,

    /// Configuration directory
    #[arg(long, env = "AGENTARK_CONFIG")]
    config: Option<PathBuf>,

    /// Data directory
    #[arg(long, env = "AGENTARK_DATA")]
    data: Option<PathBuf>,

    /// PostgreSQL database URL
    #[arg(long, env = "AGENTARK_DATABASE_URL")]
    database_url: Option<String>,

    /// Service mode for split-architecture deployments
    #[arg(long, env = "AGENTARK_SERVICE_MODE", value_enum, default_value_t = cli::ServiceMode::Control)]
    service_mode: cli::ServiceMode,

    /// Run the setup wizard
    #[arg(long)]
    setup: bool,

    /// Log level (trace, debug, info, warn, error)
    #[arg(long, default_value = "info")]
    log_level: String,

    /// Enable debug logging (shows all internal details: LLM calls, actions, memory, Docker)
    #[arg(long, env = "AGENTARK_DEBUG")]
    debug: bool,

    /// Interactive CLI chat mode
    #[arg(long)]
    chat: bool,

    /// Run one ArkPulse health check and print the latest snapshot
    #[arg(long)]
    pulse: bool,

    /// Gateway and remote access management commands
    #[command(subcommand)]
    command: Option<cli::Command>,
}

fn startup_deployment_mode(config_dir: &Path) -> core::config::DeploymentMode {
    if let Ok(force_mode) = std::env::var("AGENTARK_DEPLOYMENT_MODE") {
        match force_mode.trim().to_ascii_lowercase().as_str() {
            "internet_facing" | "internet-facing" => {
                return core::config::DeploymentMode::InternetFacing;
            }
            "trusted_local" | "trusted-local" => {
                return core::config::DeploymentMode::TrustedLocal;
            }
            _ => {}
        }
    }

    core::config::load_bootstrap_deployment_mode(config_dir)
}

fn startup_master_password_secret() -> Result<Option<String>> {
    crypto::master::MasterPasswordManager::read_install_master_secret()
}

fn docker_stack_requires_install_master_secret() -> bool {
    crypto::master::MasterPasswordManager::docker_stack_requires_install_master_secret()
}

fn missing_install_master_secret_error() -> anyhow::Error {
    anyhow::anyhow!(
        "AgentArk Docker installs require the install-managed encryption secret at {}. Recreate the bundled compose stack; for this pre-release local data, run compose down -v before starting again.",
        crypto::master::INSTALL_MASTER_SECRET_PATH
    )
}

fn legacy_keyfile_bootstrap_error() -> anyhow::Error {
    anyhow::anyhow!(
        "This data still uses legacy keyfile bootstrap encryption. This pre-release build now uses the install-managed encryption secret at {}; run compose down -v before starting again.",
        crypto::master::INSTALL_MASTER_SECRET_PATH
    )
}

fn mismatched_install_master_secret_error() -> anyhow::Error {
    anyhow::anyhow!(
        "The install-managed encryption secret at {} did not unlock existing encryption metadata. Restore the matching agentark-secrets volume or run compose down -v for a fresh pre-release install.",
        crypto::master::INSTALL_MASTER_SECRET_PATH
    )
}

fn resolve_background_service_key(
    config_dir: &Path,
    data_dir: &Path,
    deployment_mode: core::config::DeploymentMode,
    _is_first_run: bool,
) -> Result<std::sync::Arc<crate::crypto::KeyManager>> {
    let master_mgr = crypto::master::MasterPasswordManager::new(config_dir, data_dir);
    let startup_master_password = startup_master_password_secret()?;
    let install_secret_required = docker_stack_requires_install_master_secret();

    if master_mgr.is_password_set() {
        if let Some(password) = startup_master_password.as_deref() {
            tracing::info!("Using startup-provided master password secret");
            if let Ok(key) = master_mgr.unlock(password) {
                return Ok(key);
            }
            tracing::warn!("Startup master password secret did not unlock master key");
        }

        if deployment_mode == core::config::DeploymentMode::InternetFacing
            && master_mgr.is_bootstrap_password_active()?
        {
            return Err(legacy_keyfile_bootstrap_error());
        }

        if install_secret_required {
            if master_mgr.is_bootstrap_password_active()? {
                return Err(legacy_keyfile_bootstrap_error());
            }
            if startup_master_password.is_none() {
                return Err(missing_install_master_secret_error());
            }
            return Err(mismatched_install_master_secret_error());
        }

        if let Some(password) = master_mgr.bootstrap_password_if_active()? {
            tracing::info!("Using local bootstrap password for service-mode encryption");
            return master_mgr.unlock(&password);
        }

        anyhow::bail!(
            "Background services require the install-managed encryption secret at {} when a master password is configured.",
            crypto::master::INSTALL_MASTER_SECRET_PATH
        );
    }

    if let Some(password) = startup_master_password.as_deref() {
        tracing::info!("Initializing master password from install-managed startup secret");
        return master_mgr.initialize_startup_password_if_needed(password);
    }

    if deployment_mode == core::config::DeploymentMode::InternetFacing || install_secret_required {
        return Err(missing_install_master_secret_error());
    }

    if let Some(key) = master_mgr.initialize_bootstrap_password_if_needed()? {
        tracing::info!("Initialized bootstrap encryption password for background service startup");
        return Ok(key);
    }

    master_mgr.prepare_keyfile_encryption()
}

async fn initialize_executor_service_globals(config_dir: &Path, data_dir: &Path) -> Result<()> {
    let deployment_mode = startup_deployment_mode(config_dir);
    let is_first_run = !core::config::bootstrap_metadata_exists(config_dir);
    let unified_key =
        resolve_background_service_key(config_dir, data_dir, deployment_mode, is_first_run)?;
    core::config::set_global_key_manager(unified_key.clone());
    crate::storage::install_storage_key_manager(unified_key);

    let database_config = storage::DatabaseConfig::from_env().map_err(|_| {
        anyhow::anyhow!("AGENTARK_DATABASE_URL is required for split-service executor startup")
    })?;
    let storage = storage::Storage::connect(database_config).await?;
    core::config::set_global_settings_storage(storage);
    Ok(())
}

const CLI_CHAT_COMMAND: &str = "agentark --chat";
const CLI_SETUP_COMMAND: &str = "agentark --setup";
const CLI_SETTINGS_URL: &str = "http://localhost:8990";
const CLI_SETUP_PROMPT: &str = "Launch setup wizard now? [Y/n]";

#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) struct CliChatReadiness {
    pub chat_ready: bool,
    pub is_fresh_unconfigured: bool,
    pub configured_model_count: usize,
}

pub(crate) fn cli_chat_readiness(config: &core::config::AgentConfig) -> CliChatReadiness {
    let configured_model_count = if !config.model_pool.slots.is_empty() {
        config
            .model_pool
            .slots
            .iter()
            .filter(|slot| slot.enabled)
            .count()
    } else if core::chat_model_is_configured(config) {
        1
    } else {
        0
    };

    CliChatReadiness {
        chat_ready: configured_model_count > 0,
        is_fresh_unconfigured: configured_model_count == 0,
        configured_model_count,
    }
}

pub(crate) fn render_cli_chat_onboarding_message(
    readiness: &CliChatReadiness,
    include_prompt: bool,
) -> String {
    let mut lines = vec![
        "CLI chat setup required.".to_string(),
        String::new(),
        if readiness.is_fresh_unconfigured {
            "No chat model is configured yet, so this install is not ready for CLI chat."
                .to_string()
        } else {
            "No usable chat model is configured right now, so CLI chat cannot start.".to_string()
        },
        String::new(),
        "Next steps:".to_string(),
        format!(
            "  1. Run `{}` for the guided CLI setup wizard.",
            CLI_SETUP_COMMAND
        ),
        format!(
            "  2. Or open {} and go to Settings > Models.",
            CLI_SETTINGS_URL
        ),
        format!("  3. Re-run `{}` after setup.", CLI_CHAT_COMMAND),
    ];

    if include_prompt {
        lines.push(String::new());
        lines.push(CLI_SETUP_PROMPT.to_string());
    }

    lines.join("\n")
}

fn cli_chat_request_hints() -> core::RequestExecutionHints {
    core::RequestExecutionHints {
        turn_timing_id: None,
        caller_principal: Some(actions::ActionCallerPrincipal::local_admin("cli")),
        execution_surface: actions::ActionExecutionSurface::Chat,
        direct_user_intent: true,
        secret_offered: None,
        attachments: Vec::new(),
        saved_user_facts_context: None,
        recorded_user_message_id: None,
        arkorbit_context: None,
        accepted_suggestion_context: None,
    }
}

fn should_launch_cli_setup(choice: &str) -> bool {
    !matches!(choice.trim().to_ascii_lowercase().as_str(), "n" | "no")
}

const AGENTARK_UNIX_BANNER: &[&str] = &[
    r"    _                    _      _        _    ",
    r"   / \   __ _  ___ _ __ | |_   / \   _ _| | __",
    r"  / _ \ / _` |/ _ \ '_ \| __| / _ \ | '__| |/ /",
    r" / ___ \ (_| |  __/ | | | |_ / ___ \| |  |   < ",
    r"/_/   \_\__, |\___|_| |_|\__/_/   \_\_|  |_|\_\",
    r"        |___/                                    ",
];
const AGENTARK_UNIX_BANNER_MIN_WIDTH: usize = 60;

fn print_unix_cli_banner(mode: &str) {
    print_unix_cli_banner_with_color(mode, None);
}

fn print_unix_cli_banner_with_color(mode: &str, color: Option<&str>) {
    let title = format!(
        "{} v{} | {}",
        branding::PRODUCT_NAME,
        env!("CARGO_PKG_VERSION"),
        mode
    );
    let width = AGENTARK_UNIX_BANNER
        .iter()
        .map(|line| line.len())
        .max()
        .unwrap_or(title.len())
        .max(title.len())
        .max(AGENTARK_UNIX_BANNER_MIN_WIDTH);
    let rule = "-".repeat(width);
    let prefix = color.unwrap_or("");
    let suffix = if color.is_some() { "\x1b[0m" } else { "" };

    println!();
    for line in AGENTARK_UNIX_BANNER {
        println!("{}{}{}", prefix, line, suffix);
    }
    println!("{}{}{}", prefix, rule, suffix);
    println!("{}{:^width$}{}", prefix, title, suffix, width = width);
    println!("{}{}{}", prefix, rule, suffix);
}

#[derive(Debug, Clone, Copy, Default)]
struct HumanReadableLocalLogTime;

impl FormatTime for HumanReadableLocalLogTime {
    fn format_time(&self, w: &mut Writer<'_>) -> std::fmt::Result {
        let configured_tz = std::env::var("AGENTARK_LOG_TIMEZONE")
            .ok()
            .or_else(|| std::env::var("TZ").ok())
            .and_then(|value| value.trim().parse::<chrono_tz::Tz>().ok());
        if let Some(tz) = configured_tz {
            write!(
                w,
                "{}",
                chrono::Utc::now()
                    .with_timezone(&tz)
                    .format("%Y-%m-%d %H:%M:%S %Z")
            )
        } else {
            write!(w, "{}", chrono::Local::now().format("%Y-%m-%d %H:%M:%S %Z"))
        }
    }
}

pub async fn run() -> Result<()> {
    let args = Args::parse();
    let requested_chat = args.chat || matches!(args.command.as_ref(), Some(cli::Command::Chat));
    let requested_setup = args.setup || matches!(args.command.as_ref(), Some(cli::Command::Setup));
    let requested_pulse = args.pulse || matches!(args.command.as_ref(), Some(cli::Command::Pulse));

    // Initialize tracing
    // --debug enables verbose logging for agentark while keeping noisy deps quiet
    let default_filter = if requested_chat {
        "error".to_string()
    } else if args.debug {
        "warn,agentark=debug,sqlx::query=warn,sea_orm=info,hyper=warn,reqwest=warn,bollard=info,tower=warn,h2=warn,rustls=warn".to_string()
    } else {
        format!(
            "warn,agentark={},sqlx::query=warn,sea_orm=warn,hyper=warn,reqwest=warn,tower=warn,h2=warn,rustls=warn",
            args.log_level
        )
    };
    let env_filter = if requested_chat {
        // In chat mode, force error-only; ignore RUST_LOG env var.
        "error".parse().expect("Invalid log filter")
    } else {
        tracing_subscriber::EnvFilter::try_from_default_env()
            .unwrap_or_else(|_| default_filter.parse().expect("Invalid log filter"))
    };
    tracing_subscriber::registry()
        .with(env_filter)
        .with(
            tracing_subscriber::fmt::layer()
                .with_timer(HumanReadableLocalLogTime)
                .with_target(false)
                .with_thread_ids(false)
                .with_thread_names(false),
        )
        .init();

    // Determine directories
    let dirs = branding::project_dirs().expect("Failed to determine project directories");

    let config_dir = args
        .config
        .unwrap_or_else(|| dirs.config_dir().to_path_buf());
    let data_dir = args.data.unwrap_or_else(|| dirs.data_dir().to_path_buf());

    std::env::set_var("AGENTARK_CONFIG", &config_dir);
    std::env::set_var("AGENTARK_DATA", &data_dir);

    // Ensure directories exist
    std::fs::create_dir_all(&config_dir)?;
    std::fs::create_dir_all(&data_dir)?;

    if let Some(cli::Command::LanHelper(helper_args)) = args.command.as_ref() {
        return actions::lan::run_lan_helper(helper_args.bind.clone(), helper_args.token.clone())
            .await;
    }

    let service_mode = effective_service_mode(args.service_mode);
    if service_mode != cli::ServiceMode::Control {
        return run_service_mode(service_mode, &config_dir, &data_dir).await;
    }

    // Check if this is first run (no bootstrap metadata exists yet)
    let is_first_run = !core::config::bootstrap_metadata_exists(&config_dir);
    let deployment_mode = startup_deployment_mode(&config_dir);

    if is_first_run && !requested_setup {
        // Print welcome message
        print_unix_cli_banner("Welcome");
        println!();
        println!("+---------------------------------------------------------+");
        println!("|                                                         |");
        println!(
            "|                Welcome to {} v{}                 |",
            branding::PRODUCT_NAME,
            env!("CARGO_PKG_VERSION")
        );
        println!("|                                                         |");
        println!(
            "|   A {} with:                                  |",
            branding::PRODUCT_CATEGORY
        );
        println!("|   - Chat, memory, agents, apps, and automations         |");
        println!("|   - Daily briefs, reminders, and follow-up              |");
        println!("|   - Safe actions with approvals and sandboxing          |");
        println!("|   - Connected tools and companion devices               |");
        println!("|                                                         |");
        println!("+---------------------------------------------------------+");
        println!();
    }

    tracing::info!(
        "Starting {} v{}",
        branding::PRODUCT_NAME,
        env!("CARGO_PKG_VERSION")
    );
    tracing::info!("Config directory: {}", config_dir.display());
    tracing::info!("Data directory: {}", data_dir.display());
    let cli_database_url = args
        .database_url
        .clone()
        .filter(|value| !value.trim().is_empty());
    let mut database_config = match storage::DatabaseConfig::from_env() {
        Ok(config) => config,
        Err(_) => {
            let mut config =
                storage::DatabaseConfig::new(cli_database_url.clone().ok_or_else(|| {
                    anyhow::anyhow!(
                        "AGENTARK_DATABASE_URL is required for {} startup",
                        branding::PRODUCT_NAME
                    )
                })?);
            config.apply_optional_env_overrides();
            config
        }
    };
    if let Some(database_url) = cli_database_url {
        database_config.url = database_url;
    }

    // Resolve master password to unified encryption key.
    let master_mgr = crypto::master::MasterPasswordManager::new(&config_dir, &data_dir);
    let startup_master_password = startup_master_password_secret()?;
    let install_secret_required = docker_stack_requires_install_master_secret();

    let unified_key = if master_mgr.is_password_set() {
        let mut unlocked_key = None;

        if let Some(password) = startup_master_password.as_deref() {
            tracing::info!("Using startup-provided master password secret");
            match master_mgr.unlock(password) {
                Ok(key) => unlocked_key = Some(key),
                Err(_) => {
                    tracing::warn!("Startup master password secret did not unlock master key")
                }
            }
        }

        if unlocked_key.is_none() {
            if deployment_mode == core::config::DeploymentMode::InternetFacing
                && master_mgr.is_bootstrap_password_active()?
            {
                return Err(legacy_keyfile_bootstrap_error());
            }
            if install_secret_required {
                if master_mgr.is_bootstrap_password_active()? {
                    return Err(legacy_keyfile_bootstrap_error());
                }
                if startup_master_password.is_none() {
                    return Err(missing_install_master_secret_error());
                }
                return Err(mismatched_install_master_secret_error());
            }
        }

        if unlocked_key.is_none() {
            // If this install is in bootstrap mode, unlock automatically via local keyfile-derived secret.
            if let Ok(Some(pw)) = master_mgr.bootstrap_password_if_active() {
                match master_mgr.unlock(&pw) {
                    Ok(key) => {
                        tracing::info!("Using local bootstrap password for first-run encryption");
                        unlocked_key = Some(key);
                    }
                    Err(e) => tracing::error!("Bootstrap password failed to unlock: {}", e),
                }
            }
        }

        if unlocked_key.is_none() {
            if deployment_mode == core::config::DeploymentMode::InternetFacing && args.headless {
                return Err(missing_install_master_secret_error());
            }
            if !args.headless {
                println!("Master password required.");
                print!("Enter master password: ");
                std::io::Write::flush(&mut std::io::stdout())?;
                let mut pw = String::new();
                std::io::stdin().lock().read_line(&mut pw)?;
                unlocked_key = Some(master_mgr.unlock(pw.trim())?);
            } else {
                tracing::warn!(
                    "Master password is set but no valid unlock source was provided - starting locked-mode server"
                );
                unlocked_key = Some(channels::http::serve_locked(&config_dir, &data_dir).await?);
            }
        }

        unlocked_key
    } else if let Some(password) = startup_master_password.as_deref() {
        tracing::info!("Initializing master password from install-managed startup secret");
        Some(master_mgr.initialize_startup_password_if_needed(password)?)
    } else if deployment_mode == core::config::DeploymentMode::InternetFacing
        || install_secret_required
    {
        return Err(missing_install_master_secret_error());
    } else if is_first_run {
        match master_mgr.initialize_bootstrap_password_if_needed()? {
            Some(key) => {
                tracing::info!(
                    "Initialized per-install bootstrap encryption password. Set a custom master password in Security settings."
                );
                Some(key)
            }
            None => None,
        }
    } else {
        // No master password exists; always initialize a bootstrap password so data is
        // encrypted by default, even when this is not the very first run (e.g. master.json
        // was removed or never created due to a crash).
        match master_mgr.initialize_bootstrap_password_if_needed()? {
            Some(key) => {
                tracing::info!(
                    "Initialized bootstrap encryption password. Set a custom master password in Security settings."
                );
                Some(key)
            }
            None => {
                tracing::warn!(
                    "No master password configured. Running with keyfile encryption; set one in Security settings."
                );
                None
            }
        }
    };
    // Set global key manager so all SecureConfigManager instances use the same key
    if let Some(ref key) = unified_key {
        core::config::set_global_key_manager(key.clone());
    }

    // Initialize core systems
    let agent = core::Agent::init(
        &config_dir,
        &data_dir,
        database_config.clone(),
        unified_key.clone(),
    )
    .await?;

    // Handle first run or explicit setup
    if requested_setup || (is_first_run && !requested_chat) {
        // In headless mode (Docker), skip interactive setup - just use defaults
        // Users can configure via the Web UI Settings page
        if args.headless && !requested_setup {
            tracing::info!("First run in headless mode - using default config");
            tracing::info!("Configure via Web UI at http://127.0.0.1:8990 -> Settings");
            // Config already has defaults, just save it
            agent.config.save(&config_dir, Some(&data_dir))?;
        } else {
            #[cfg(feature = "gui")]
            if !args.headless {
                println!("Launching setup wizard...");
                gui::run_setup_wizard(agent).await?;
                return Ok(());
            }

            // CLI setup for explicit --setup flag
            run_cli_setup(&config_dir, &agent).await?;

            // Reload the agent with new config and continue
            let agent = core::Agent::init(
                &config_dir,
                &data_dir,
                database_config.clone(),
                unified_key.clone(),
            )
            .await?;
            return run_headless(agent).await;
        }
    }

    if let Some(command) = args.command {
        match command {
            cli::Command::Chat => {}
            cli::Command::Setup => return Ok(()),
            cli::Command::Pulse => return run_cli_pulse(agent).await,
            command => return cli::run(agent, command).await,
        }
    }

    if requested_chat {
        let readiness = cli_chat_readiness(&agent.config);
        if !readiness.chat_ready {
            if std::io::stdin().is_terminal() {
                println!();
                println!("{}", render_cli_chat_onboarding_message(&readiness, false));
                println!();
                print!("{} ", CLI_SETUP_PROMPT);
                std::io::stdout().flush()?;

                let mut choice = String::new();
                std::io::stdin().lock().read_line(&mut choice)?;
                if should_launch_cli_setup(&choice) {
                    run_cli_setup(&config_dir, &agent).await?;
                    let agent = core::Agent::init(
                        &config_dir,
                        &data_dir,
                        database_config.clone(),
                        unified_key.clone(),
                    )
                    .await?;
                    let readiness = cli_chat_readiness(&agent.config);
                    if !readiness.chat_ready {
                        println!();
                        eprintln!("{}", render_cli_chat_onboarding_message(&readiness, false));
                        anyhow::bail!("CLI chat is not ready yet");
                    }
                    return run_chat_repl(agent).await;
                }

                println!();
                println!("Chat setup skipped.");
                println!(
                    "Run `{}` or open {} to finish setup.",
                    CLI_SETUP_COMMAND, CLI_SETTINGS_URL
                );
                return Ok(());
            }

            eprintln!("{}", render_cli_chat_onboarding_message(&readiness, false));
            anyhow::bail!("CLI chat is not ready yet");
        }

        return run_chat_repl(agent).await;
    }

    if requested_pulse {
        return run_cli_pulse(agent).await;
    }

    if args.headless {
        run_headless(agent).await
    } else {
        #[cfg(feature = "gui")]
        {
            gui::run(agent).await
        }
        #[cfg(not(feature = "gui"))]
        {
            tracing::warn!("GUI feature not enabled, running headless");
            run_headless(agent).await
        }
    }
}

async fn run_service_mode(
    mode: cli::ServiceMode,
    config_dir: &Path,
    data_dir: &Path,
) -> Result<()> {
    match mode {
        cli::ServiceMode::Control => Ok(()),
        cli::ServiceMode::Executor => {
            initialize_executor_service_globals(config_dir, data_dir).await?;
            let config = executor::ExecutorServiceConfig::from_env_paths(
                config_dir.to_path_buf(),
                data_dir.to_path_buf(),
            )?;
            executor::run_service(config).await
        }
        cli::ServiceMode::Workspace => {
            let config = workspace::WorkspaceServiceConfig::from_env_paths(
                config_dir.to_path_buf(),
                data_dir.to_path_buf(),
            )?;
            workspace::run_service(config).await
        }
    }
}

fn effective_service_mode(parsed: cli::ServiceMode) -> cli::ServiceMode {
    if parsed != cli::ServiceMode::Control {
        return parsed;
    }

    std::env::var("AGENTARK_STACK_ROLE")
        .ok()
        .and_then(|value| {
            let normalized_value = value.trim().to_ascii_lowercase();
            let normalized = match normalized_value.as_str() {
                "control-plane" => "control",
                other => other,
            };
            cli::ServiceMode::from_str(normalized, true).ok()
        })
        .unwrap_or(parsed)
}

/// Interactive CLI chat mode: talk to the agent from your terminal.
#[derive(Default)]
struct CliStreamRenderState {
    assistant_inline_open: bool,
}

fn finish_cli_inline_response(state: &mut CliStreamRenderState) -> Result<()> {
    if state.assistant_inline_open {
        println!();
        std::io::stdout().flush()?;
        state.assistant_inline_open = false;
    }
    Ok(())
}

fn render_cli_stream_event(
    event: core::StreamEvent,
    state: &mut CliStreamRenderState,
) -> Result<()> {
    match event {
        core::StreamEvent::RunStarted { run_id, .. } => {
            finish_cli_inline_response(state)?;
            println!("\x1b[3;90mRun {}\x1b[0m", &run_id[..run_id.len().min(8)]);
        }
        core::StreamEvent::ChatTaskStarted {
            task_id,
            description,
            work_type,
            ..
        } => {
            finish_cli_inline_response(state)?;
            println!(
                "\x1b[35m[task]\x1b[0m {} [{}] {}",
                &task_id[..task_id.len().min(8)],
                work_type,
                description
            );
        }
        core::StreamEvent::Token(token) => {
            if !state.assistant_inline_open {
                print!("\x1b[32magentark ➜\x1b[0m ");
                state.assistant_inline_open = true;
            }
            print!("{}", token);
            std::io::stdout().flush()?;
        }
        core::StreamEvent::Thinking(detail) => {
            finish_cli_inline_response(state)?;
            println!("\x1b[3;90m{}\x1b[0m", detail);
        }
        core::StreamEvent::ReasoningDelta {
            phase,
            content_delta,
            done,
        } => {
            finish_cli_inline_response(state)?;
            if done {
                println!("\x1b[3;90m[reasoning:{}] done\x1b[0m", phase);
            } else if !content_delta.trim().is_empty() {
                println!(
                    "\x1b[3;90m[reasoning:{}] {}\x1b[0m",
                    phase,
                    content_delta.trim()
                );
            }
        }
        core::StreamEvent::ToolStart { name, .. } => {
            finish_cli_inline_response(state)?;
            println!("\x1b[36m[start]\x1b[0m {}", name);
        }
        core::StreamEvent::ToolProgress { name, content, .. } => {
            finish_cli_inline_response(state)?;
            if content.trim().is_empty() {
                println!("\x1b[36m[live]\x1b[0m {}", name);
            } else {
                println!("\x1b[36m[live]\x1b[0m {}: {}", name, content);
            }
        }
        core::StreamEvent::ToolResult { name, content } => {
            finish_cli_inline_response(state)?;
            if content.trim().is_empty() {
                println!("\x1b[32m[done]\x1b[0m {}", name);
            } else {
                println!("\x1b[32m[done]\x1b[0m {}: {}", name, content.trim());
            }
        }
        core::StreamEvent::PlanGenerated { plan } => {
            finish_cli_inline_response(state)?;
            println!(
                "\x1b[35m[plan]\x1b[0m {} step{}",
                plan.steps.len(),
                if plan.steps.len() == 1 { "" } else { "s" }
            );
        }
        core::StreamEvent::PlanStepUpdate {
            step_id,
            step_title,
            status,
            detail,
            ..
        } => {
            finish_cli_inline_response(state)?;
            let title = step_title.unwrap_or_else(|| format!("Step {}", step_id));
            let message = detail.unwrap_or_else(|| format!("{:?}", status));
            println!("\x1b[35m[step]\x1b[0m {} - {}", title, message);
        }
    }
    Ok(())
}

async fn run_cli_streamed_turn(
    agent: std::sync::Arc<core::Agent>,
    input: &str,
    conv_id: &str,
    auto_show_trace: bool,
) -> Result<()> {
    let trace_ref = std::sync::Arc::new(tokio::sync::RwLock::new(core::ExecutionTrace::default()));
    let (stream_tx, mut stream_rx) = tokio::sync::mpsc::channel::<core::StreamEvent>(4096);
    let render_task = tokio::spawn(async move {
        let mut render_state = CliStreamRenderState::default();
        let mut saw_token = false;
        while let Some(event) = stream_rx.recv().await {
            if matches!(event, core::StreamEvent::Token(_)) {
                saw_token = true;
            }
            let _ = render_cli_stream_event(event, &mut render_state);
        }
        let _ = finish_cli_inline_response(&mut render_state);
        saw_token
    });

    let processed = agent
        .process_message_stream_with_meta_and_hints(
            input,
            "cli",
            Some(conv_id),
            None,
            trace_ref.clone(),
            stream_tx,
            cli_chat_request_hints(),
        )
        .await?;

    let saw_token = render_task.await.unwrap_or(false);

    let trace = trace_ref.read().await.clone();
    if auto_show_trace && !trace.id.trim().is_empty() {
        print_cli_trace(&trace);
        println!();
    }

    if processed.response.trim().is_empty() {
        return Ok(());
    }

    if !saw_token {
        println!("\x1b[32magentark ➜\x1b[0m {}", processed.response);
    }

    Ok(())
}

async fn run_chat_repl(agent: core::Agent) -> Result<()> {
    if !std::io::stdin().is_terminal() {
        return run_chat_repl_noninteractive(agent).await;
    }

    let agent = std::sync::Arc::new(agent);
    let conversation_id = uuid::Uuid::new_v4().to_string();
    let mut auto_show_trace = false;

    print_unix_cli_banner("CLI Chat");
    println!();
    println!("Type your message and press Enter.");
    println!("Commands: /exit  /new  /help");
    println!("Shortcuts: Ctrl+T toggles trace mode, Ctrl+D exits the chat.");
    println!("When trace mode is enabled, the full trace prints before each agent reply.");
    println!();

    let mut conv_id = conversation_id;

    loop {
        let input = match read_cli_input_line().await? {
            CliReadAction::Exit => break,
            CliReadAction::ToggleTrace => {
                auto_show_trace = !auto_show_trace;
                println!();
                if auto_show_trace {
                    println!(
                        "\x1b[3;35mTrace mode enabled.\x1b[0m \x1b[3;90mFull traces will print before each agent reply.\x1b[0m"
                    );
                } else {
                    println!("\x1b[3;35mTrace mode disabled.\x1b[0m");
                }
                println!();
                continue;
            }
            CliReadAction::Submit(value) => value,
        };
        if input.is_empty() {
            continue;
        }
        let lowered = input.to_ascii_lowercase();

        if matches!(
            lowered.as_str(),
            "/trace" | "/t" | "/trace on" | "/trace off" | "/t on" | "/t off"
        ) {
            println!();
            println!(
                "\x1b[3;35mTrace mode is controlled with Ctrl+T now.\x1b[0m \x1b[3;90mToggle it on to show the full trace before each reply.\x1b[0m"
            );
            println!();
            continue;
        }

        match lowered.as_str() {
            "/exit" | "/quit" | "/q" => {
                println!("Goodbye!");
                break;
            }
            "/new" => {
                conv_id = uuid::Uuid::new_v4().to_string();
                println!("\x1b[33mNew conversation started\x1b[0m");
                println!();
                continue;
            }
            "/help" => {
                println!();
                println!("  Ctrl+T - Toggle full trace mode before replies");
                println!("  Ctrl+D - Exit the chat");
                println!("  Tab    - Autocomplete slash commands");
                println!("  /new   - Start a new conversation");
                println!("  /exit  - Quit the CLI");
                println!("  /help  - Show this help");
                println!();
                continue;
            }
            _ => {}
        }

        if let Err(e) =
            run_cli_streamed_turn(agent.clone(), input.as_str(), &conv_id, auto_show_trace).await
        {
            eprintln!("\x1b[31merror:\x1b[0m {}", e);
        }
        println!();
        continue;
    }

    Ok(())
}

fn pulse_status_color(status: &str) -> &'static str {
    match status.trim().to_ascii_lowercase().as_str() {
        "ok" => "\x1b[32m",
        "alert" | "warning" => "\x1b[33m",
        "error" | "failed" => "\x1b[31m",
        _ => "\x1b[36m",
    }
}

fn format_pulse_timestamp(raw: &str) -> String {
    chrono::DateTime::parse_from_rfc3339(raw)
        .map(|ts| ts.with_timezone(&chrono::Utc))
        .map(|ts| ts.format("%Y-%m-%d %H:%M:%S UTC").to_string())
        .unwrap_or_else(|_| raw.to_string())
}

fn latest_pulse_event(
    events: Vec<crate::sentinel::PulseEvent>,
) -> Option<crate::sentinel::PulseEvent> {
    events.into_iter().max_by_key(|event| {
        chrono::DateTime::parse_from_rfc3339(&event.timestamp)
            .map(|ts| ts.timestamp_millis())
            .unwrap_or(0)
    })
}

fn describe_cli_pulse_remediation(finding: &crate::sentinel::DoctorFinding) -> Option<String> {
    match finding.remediation.as_ref() {
        Some(crate::sentinel::DoctorRemediationSpec::TunnelStartVerify) => {
            Some("Start tunnel and verify /tunnel/status returns active + URL".to_string())
        }
        Some(crate::sentinel::DoctorRemediationSpec::TunnelRestartVerify) => {
            Some("Restart tunnel and verify public reachability".to_string())
        }
        Some(crate::sentinel::DoctorRemediationSpec::AppRestart { app_id }) => {
            Some(format!("Restart app {} and re-check health", app_id))
        }
        Some(crate::sentinel::DoctorRemediationSpec::ReadonlyInvestigation { topic }) => {
            Some(match topic {
                crate::sentinel::DoctorReadonlyInvestigationTopic::MemoryCaptureHealth => {
                    "Review failed memory captures and model health".to_string()
                }
            })
        }
        Some(crate::sentinel::DoctorRemediationSpec::ManagedAppOperation { app_id, operation }) => {
            Some(match operation {
                crate::sentinel::DoctorManagedAppOperation::CompilePythonRequirements => {
                    format!("Compile pinned Python requirements for app {}", app_id)
                }
                crate::sentinel::DoctorManagedAppOperation::GenerateCargoLockfile => {
                    format!("Generate Cargo.lock for app {}", app_id)
                }
                crate::sentinel::DoctorManagedAppOperation::RemoveNpmInstallHooks => {
                    format!("Remove npm install lifecycle hooks from app {}", app_id)
                }
            })
        }
        Some(crate::sentinel::DoctorRemediationSpec::ShellCommand { command }) => {
            let normalized = command.trim();
            if normalized.is_empty() {
                None
            } else {
                Some(normalized.to_string())
            }
        }
        None => {
            let normalized = finding.fix_command.trim();
            if normalized.is_empty() {
                None
            } else {
                Some(normalized.to_string())
            }
        }
    }
}

fn print_cli_pulse_event(event: &crate::sentinel::PulseEvent) {
    let summary = if event.summary.trim().is_empty() {
        event.message.trim()
    } else {
        event.summary.trim()
    };
    let details = &event.details;
    let status = event.status.trim().to_ascii_lowercase();
    let status_color = pulse_status_color(&status);
    print_unix_cli_banner_with_color("ArkPulse", Some(status_color));
    println!();
    println!(
        "\x1b[36mStatus:\x1b[0m {}{}\x1b[0m",
        status_color,
        status.to_ascii_uppercase()
    );
    println!(
        "\x1b[36mCaptured:\x1b[0m {}",
        format_pulse_timestamp(&event.timestamp)
    );
    println!("\x1b[36mSummary:\x1b[0m {}", summary);
    println!(
        "\x1b[36mTasks:\x1b[0m pending {} | running {} | done {} | total {}",
        details.pending_tasks, details.running_tasks, details.completed_tasks, details.total_tasks
    );
    println!(
        "\x1b[36mWatchers:\x1b[0m {} active",
        details.active_watchers
    );
    if details.doctor_score > 0 {
        println!("\x1b[36mHealth score:\x1b[0m {}", details.doctor_score);
    }

    if !details.health_checks.is_empty() {
        println!();
        println!("\x1b[35mHealth checks\x1b[0m");
        for check in &details.health_checks {
            let color = pulse_status_color(&check.status);
            println!(
                "  - {}{}{}\x1b[0m: {}",
                color,
                check.service,
                if check.status.trim().is_empty() {
                    String::new()
                } else {
                    format!(" ({})", check.status)
                },
                check.message
            );
        }
    }

    let findings = details
        .doctor_findings
        .iter()
        .filter(|finding| finding.user_actionable)
        .take(5)
        .collect::<Vec<_>>();
    if !findings.is_empty() {
        println!();
        println!("\x1b[35mTop issues\x1b[0m");
        for finding in findings {
            let color = pulse_status_color(&finding.severity);
            println!(
                "  - {}[{}]\x1b[0m {}",
                color,
                finding.severity.to_ascii_uppercase(),
                finding.title
            );
            if !finding.target.trim().is_empty() {
                println!("      target: {}", finding.target.trim());
            }
            if let Some(remediation) = describe_cli_pulse_remediation(finding) {
                println!("      next: {}", remediation);
            }
        }
    }

    if !details.overdue_list.is_empty() {
        println!();
        println!("\x1b[35mOverdue tasks\x1b[0m");
        for item in details.overdue_list.iter().take(5) {
            println!("  - {}", item);
        }
    }

    if !details.failed_list.is_empty() {
        println!();
        println!("\x1b[35mRecent failures\x1b[0m");
        for item in details.failed_list.iter().take(5) {
            println!("  - {}", item);
        }
    }
}

async fn run_cli_pulse(agent: core::Agent) -> Result<()> {
    println!("Running ArkPulse health check...");
    println!();

    let agent = std::sync::Arc::new(tokio::sync::RwLock::new(agent));
    crate::sentinel::run_pulse(&agent).await;

    let latest = {
        let guard = agent.read().await;
        latest_pulse_event(crate::sentinel::get_pulse_log(&guard).await)
    };

    if let Some(event) = latest {
        print_cli_pulse_event(&event);
    } else {
        println!("\x1b[3;90mNo ArkPulse snapshot is available yet.\x1b[0m");
    }

    Ok(())
}

async fn run_chat_repl_noninteractive(agent: core::Agent) -> Result<()> {
    let mut input = String::new();
    std::io::stdin().read_to_string(&mut input)?;
    let input = input.trim();
    if input.is_empty() {
        return Ok(());
    }

    let conversation_id = uuid::Uuid::new_v4().to_string();
    run_cli_streamed_turn(std::sync::Arc::new(agent), input, &conversation_id, false).await?;
    Ok(())
}

enum CliReadAction {
    Submit(String),
    ToggleTrace,
    Exit,
}

struct CliRawModeGuard;

impl CliRawModeGuard {
    fn enable() -> Result<Self> {
        enable_raw_mode()?;
        Ok(Self)
    }
}

impl Drop for CliRawModeGuard {
    fn drop(&mut self) {
        let _ = disable_raw_mode();
    }
}

fn render_cli_prompt(buffer: &str) -> Result<()> {
    print!("\r\x1b[K\x1b[36myou ➜\x1b[0m {}", buffer);
    std::io::stdout().flush()?;
    Ok(())
}

const CLI_COMMANDS: &[&str] = &["/exit", "/quit", "/q", "/new", "/help"];

fn common_prefix(left: &str, right: &str) -> String {
    left.chars()
        .zip(right.chars())
        .take_while(|(a, b)| a.eq_ignore_ascii_case(b))
        .map(|(c, _)| c)
        .collect()
}

fn complete_cli_command(buffer: &str) -> Option<String> {
    let trimmed = buffer.trim_start();
    if !trimmed.starts_with('/') {
        return None;
    }
    let lowered = trimmed.to_ascii_lowercase();
    let mut matches = CLI_COMMANDS
        .iter()
        .copied()
        .filter(|candidate| candidate.starts_with(&lowered))
        .collect::<Vec<_>>();
    if matches.is_empty() {
        return None;
    }
    matches.sort_unstable();
    if matches.len() == 1 {
        return Some(matches[0].to_string());
    }
    let mut prefix = matches[0].to_string();
    for candidate in matches.iter().skip(1) {
        prefix = common_prefix(&prefix, candidate);
        if prefix.is_empty() {
            break;
        }
    }
    if prefix.len() > lowered.len() {
        Some(prefix)
    } else {
        None
    }
}

fn matching_cli_commands(buffer: &str) -> Vec<&'static str> {
    let trimmed = buffer.trim_start();
    if !trimmed.starts_with('/') {
        return Vec::new();
    }
    let lowered = trimmed.to_ascii_lowercase();
    CLI_COMMANDS
        .iter()
        .copied()
        .filter(|candidate| candidate.starts_with(&lowered))
        .collect()
}

async fn read_cli_input_line() -> Result<CliReadAction> {
    let _raw_mode = CliRawModeGuard::enable()?;
    let mut buffer = String::new();
    render_cli_prompt(&buffer)?;

    loop {
        match event::read()? {
            Event::Key(key) if matches!(key.kind, KeyEventKind::Press | KeyEventKind::Repeat) => {
                match (key.code, key.modifiers) {
                    (KeyCode::Char('d'), modifiers)
                        if modifiers.contains(KeyModifiers::CONTROL) =>
                    {
                        print!("\r\x1b[K");
                        std::io::stdout().flush()?;
                        println!("Goodbye!");
                        return Ok(CliReadAction::Exit);
                    }
                    (KeyCode::Char('t'), modifiers)
                        if modifiers.contains(KeyModifiers::CONTROL) =>
                    {
                        print!("\r\x1b[K");
                        std::io::stdout().flush()?;
                        return Ok(CliReadAction::ToggleTrace);
                    }
                    (KeyCode::Enter, _) => {
                        print!("\r\x1b[K");
                        std::io::stdout().flush()?;
                        println!("\x1b[36myou ➜\x1b[0m {}", buffer);
                        return Ok(CliReadAction::Submit(buffer.trim().to_string()));
                    }
                    (KeyCode::Backspace, _) => {
                        buffer.pop();
                        render_cli_prompt(&buffer)?;
                    }
                    (KeyCode::Tab, _) => {
                        if let Some(completed) = complete_cli_command(&buffer) {
                            buffer = completed;
                            render_cli_prompt(&buffer)?;
                        } else {
                            let matches = matching_cli_commands(&buffer);
                            if !matches.is_empty() {
                                print!("\r\x1b[K");
                                std::io::stdout().flush()?;
                                println!();
                                println!("\x1b[90mcommands: {}\x1b[0m", matches.join("  "));
                                render_cli_prompt(&buffer)?;
                            }
                        }
                    }
                    (KeyCode::Esc, _) => {
                        buffer.clear();
                        render_cli_prompt(&buffer)?;
                    }
                    (KeyCode::Char(c), modifiers)
                        if !modifiers.intersects(KeyModifiers::CONTROL | KeyModifiers::ALT) =>
                    {
                        buffer.push(c);
                        render_cli_prompt(&buffer)?;
                    }
                    _ => {}
                }
            }
            Event::Paste(text) => {
                buffer.push_str(&text);
                render_cli_prompt(&buffer)?;
            }
            _ => {}
        }
    }
}

fn cli_trace_status(trace: &core::ExecutionTrace) -> &'static str {
    if let Some(last_step) = trace.steps.last() {
        let title = last_step.title.to_ascii_lowercase();
        let step_type = last_step.step_type.to_ascii_lowercase();
        if step_type == "error" || title.contains("failed") {
            return "failed";
        }
        if step_type == "warning" || title.contains("blocked") {
            return "warning";
        }
    }
    if trace.completed_at.is_some() {
        "completed"
    } else {
        "running"
    }
}

fn colorize_trace_status(status: &str) -> String {
    match status {
        "completed" => format!("\x1b[32m{}\x1b[0m", status),
        "failed" => format!("\x1b[31m{}\x1b[0m", status),
        "warning" => format!("\x1b[33m{}\x1b[0m", status),
        _ => format!("\x1b[36m{}\x1b[0m", status),
    }
}

fn colorize_trace_step_title(step_type: &str, title: &str) -> String {
    match step_type.to_ascii_lowercase().as_str() {
        "error" => format!("\x1b[31m{}\x1b[0m", title),
        "warning" => format!("\x1b[33m{}\x1b[0m", title),
        "success" => format!("\x1b[32m{}\x1b[0m", title),
        "thinking" => format!("\x1b[34m{}\x1b[0m", title),
        _ => format!("\x1b[36m{}\x1b[0m", title),
    }
}

fn print_cli_trace(trace: &core::ExecutionTrace) {
    if trace.id.trim().is_empty() {
        println!("\x1b[3;90mNo execution trace is available yet.\x1b[0m");
        return;
    }

    let started_at = trace
        .started_at
        .map(|value| value.format("%Y-%m-%d %H:%M:%S UTC").to_string())
        .unwrap_or_else(|| "-".to_string());
    let duration_ms = trace.started_at.and_then(|start| {
        trace
            .completed_at
            .map(|end| (end - start).num_milliseconds().max(0) as u64)
    });

    println!("\x1b[3;35mTrace\x1b[0m");
    println!("\x1b[3;36m  ID:\x1b[0m {}", trace.id);
    println!(
        "\x1b[3;36m  Status:\x1b[0m {}",
        colorize_trace_status(cli_trace_status(trace))
    );
    println!("\x1b[3;36m  Channel:\x1b[0m {}", trace.channel);
    println!("\x1b[3;36m  Started:\x1b[0m {}", started_at);
    println!("\x1b[3;36m  Steps:\x1b[0m {}", trace.steps.len());
    if let Some(duration_ms) = duration_ms {
        println!("\x1b[3;36m  Duration:\x1b[0m {} ms", duration_ms);
    }
    if let Some(model) = trace.model.as_deref() {
        println!("\x1b[3;36m  Model:\x1b[0m {}", model);
    }
    if trace.total_tokens > 0 {
        println!(
            "\x1b[3;36m  Tokens:\x1b[0m in {} | out {} | total {}",
            trace.input_tokens, trace.output_tokens, trace.total_tokens
        );
    }
    if trace.cost_usd > 0.0 {
        println!("\x1b[3;36m  Cost:\x1b[0m ${:.6}", trace.cost_usd);
    }
    if let Some(complexity) = trace.complexity.as_deref() {
        println!("\x1b[3;36m  Complexity:\x1b[0m {}", complexity);
    }
    println!();

    for (index, step) in trace.steps.iter().enumerate() {
        println!(
            "\x1b[3;33m{}. [{}]\x1b[0m {}",
            index + 1,
            step.timestamp.format("%H:%M:%S"),
            colorize_trace_step_title(&step.step_type, &step.title)
        );
        if !step.detail.trim().is_empty() {
            println!("\x1b[3;90m   {}\x1b[0m", step.detail);
        }
        if let Some(data) = step
            .data
            .as_deref()
            .map(str::trim)
            .filter(|value| !value.is_empty())
        {
            println!("\x1b[3;90m   Data:\x1b[0m");
            for line in data.lines() {
                println!("\x1b[3;90m     {}\x1b[0m", line);
            }
        }
        if let Some(duration_ms) = step.duration_ms {
            println!("\x1b[3;90m   Duration:\x1b[0m {} ms", duration_ms);
        }
    }
}

/// CLI-based setup wizard for headless mode
async fn run_cli_setup(config_dir: &Path, agent: &core::Agent) -> Result<()> {
    print_unix_cli_banner("Setup Wizard");
    println!();
    println!("Your Agent Identity (DID):");
    println!("  {}", agent.identity.did());
    println!();

    let stdin = std::io::stdin();
    let mut stdout = std::io::stdout();

    // Step 1: LLM Provider
    println!("=== Step 1: LLM Configuration ===");
    println!();
    println!("Choose your LLM provider:");
    println!("  1) Ollama (local, free, private)");
    println!("  2) Anthropic Claude (cloud, most capable)");
    println!("  3) OpenAI GPT (cloud)");
    println!("  4) OpenAI-Compatible (LMStudio, vLLM, etc.)");
    println!();

    print!("Enter choice [1-4] (default: 1): ");
    stdout.flush()?;

    let mut choice = String::new();
    stdin.lock().read_line(&mut choice)?;
    let choice = choice.trim();

    let llm = match choice {
        "2" => {
            print!("Anthropic API Key: ");
            stdout.flush()?;
            let mut api_key = String::new();
            stdin.lock().read_line(&mut api_key)?;
            core::LlmProvider::Anthropic {
                api_key: api_key.trim().to_string(),
                model: "claude-sonnet-4-20250514".to_string(),
            }
        }
        "3" => {
            print!("OpenAI API Key: ");
            stdout.flush()?;
            let mut api_key = String::new();
            stdin.lock().read_line(&mut api_key)?;
            core::LlmProvider::OpenAI {
                api_key: api_key.trim().to_string(),
                model: "gpt-4o".to_string(),
                base_url: None,
            }
        }
        "4" => {
            print!("Base URL (e.g., http://localhost:1234/v1): ");
            stdout.flush()?;
            let mut base_url = String::new();
            stdin.lock().read_line(&mut base_url)?;

            print!("Model name: ");
            stdout.flush()?;
            let mut model = String::new();
            stdin.lock().read_line(&mut model)?;

            core::LlmProvider::OpenAI {
                api_key: "not-needed".to_string(),
                model: model.trim().to_string(),
                base_url: Some(base_url.trim().to_string()),
            }
        }
        _ => {
            print!("Ollama URL: ");
            stdout.flush()?;
            let mut url = String::new();
            stdin.lock().read_line(&mut url)?;
            let url = url.trim();

            print!("Model: ");
            stdout.flush()?;
            let mut model = String::new();
            stdin.lock().read_line(&mut model)?;
            let model = model.trim();

            core::LlmProvider::Ollama {
                base_url: url.to_string(),
                model: model.to_string(),
            }
        }
    };

    println!();

    // Step 2: Telegram (optional)
    println!("=== Step 2: Telegram Configuration (Optional) ===");
    println!();
    print!("Configure Telegram bot? [y/N]: ");
    stdout.flush()?;

    let mut telegram_choice = String::new();
    stdin.lock().read_line(&mut telegram_choice)?;

    let telegram = if telegram_choice.trim().to_lowercase() == "y" {
        print!("Bot Token (from @BotFather): ");
        stdout.flush()?;
        let mut token = String::new();
        stdin.lock().read_line(&mut token)?;

        print!("Allowed User IDs (comma-separated, or empty for pairing mode): ");
        stdout.flush()?;
        let mut users = String::new();
        stdin.lock().read_line(&mut users)?;

        let allowed_users: Vec<i64> = users
            .trim()
            .split(',')
            .filter_map(|s| s.trim().parse().ok())
            .collect();

        Some(core::config::TelegramConfig {
            bot_token: token.trim().to_string(),
            allowed_users,
            dm_policy: "pairing".to_string(),
        })
    } else {
        None
    };

    println!();

    // Save configuration
    let mut config = agent.config.clone();
    config.llm = llm;
    config.telegram = telegram;
    config.save(config_dir, None)?;
    print_unix_cli_banner("Setup Complete");
    println!();
    println!("Configuration saved to: {}", config_dir.display());
    println!();
    println!("To start your agent:");
    println!("  Headless mode: agentark --headless");
    println!("  Native GUI:    build with --features gui, then run agentark");
    println!();
    println!("HTTP API will be available at: http://127.0.0.1:8990");
    println!();

    Ok(())
}

async fn wait_for_shutdown_signal() -> Result<()> {
    #[cfg(unix)]
    {
        let mut sigterm =
            tokio::signal::unix::signal(tokio::signal::unix::SignalKind::terminate())?;
        tokio::select! {
            res = tokio::signal::ctrl_c() => res?,
            _ = sigterm.recv() => {}
        }
        Ok(())
    }

    #[cfg(not(unix))]
    {
        tokio::signal::ctrl_c().await?;
        Ok(())
    }
}

fn mask_api_key_for_console(key: &str) -> String {
    let trimmed = key.trim();
    if trimmed.is_empty() {
        return "<empty>".to_string();
    }
    if trimmed.len() <= 8 {
        return "configured".to_string();
    }
    format!("{}...{}", &trimmed[..4], &trimmed[trimmed.len() - 4..])
}

async fn run_headless(agent: core::Agent) -> Result<()> {
    tracing::info!("Running in headless mode");

    print_unix_cli_banner("Headless Mode");
    println!();
    println!("+---------------------------------------------------------+");
    println!(
        "|             {} v{} - Headless Mode               |",
        branding::PRODUCT_NAME,
        env!("CARGO_PKG_VERSION")
    );
    println!("+---------------------------------------------------------+");
    println!();
    if let Some(ref api_key) = agent.api_key {
        println!("+---------------------------------------------------------+");
        println!("|  Authentication enabled                                 |");
        println!("|  API Key: {}...  |", mask_api_key_for_console(api_key));
        println!("|  Use: Authorization: Bearer <key>                       |");
        println!("+---------------------------------------------------------+");
        println!();
    }
    let bind_addr = std::env::var("AGENTARK_BIND").unwrap_or_else(|_| "127.0.0.1:8990".to_string());
    println!("+---------------------------------------------------------+");
    println!("|  Web UI:   http://{}                       |", bind_addr);
    println!("+---------------------------------------------------------+");
    println!("|  API Endpoints:                                         |");
    println!("|    GET  /health  - Health check (no auth)               |");
    println!("|    GET  /status  - Agent status                         |");
    println!("|    POST /chat    - Chat with agent                      |");
    println!("|    GET  /skills  - List skills                          |");
    println!("|    GET  /tasks   - List tasks                           |");
    println!("+---------------------------------------------------------+");
    println!();
    let tunnel_auto = std::env::var("AGENTARK_TUNNEL")
        .map(|v| v.eq_ignore_ascii_case("true") || v == "1")
        .unwrap_or(false);
    if tunnel_auto {
        println!("Remote access: ENABLED (AGENTARK_TUNNEL=true)");
        println!("  Tunnel URL will appear in logs shortly...");
    } else {
        println!("Remote access (VPS): AGENTARK_TUNNEL=true docker compose up -d");
        println!("  Or enable in Settings > Advanced > Remote Access");
    }
    println!();
    println!("Press Ctrl+C to stop");
    println!();

    let agent = std::sync::Arc::new(tokio::sync::RwLock::new(agent));

    // Daily brief task is opt-in via Settings; it is not auto-created on fresh install.

    let (shutdown_tx, shutdown_rx) = tokio::sync::watch::channel(false);

    // Start HTTP server for local IPC
    let http_handle = {
        let agent = agent.clone();
        let shutdown = shutdown_rx.clone();
        crate::spawn_logged!("src/lib.rs:1634", async move {
            if let Err(e) = channels::http::serve(agent, shutdown).await {
                tracing::error!("HTTP server error: {}", e);
            }
        })
    };

    // Start Matrix sync runtime if configured
    let matrix_handle = {
        let agent = agent.clone();
        let shutdown = shutdown_rx.clone();
        crate::spawn_logged!("src/lib.rs:1645", async move {
            if let Err(e) = channels::matrix::serve(agent, shutdown).await {
                tracing::error!("Matrix runtime error: {}", e);
            }
        })
    };

    // Start Discord gateway runtime if configured
    let discord_handle = {
        let agent = agent.clone();
        crate::spawn_logged!("src/lib.rs:1655", async move {
            if let Err(e) = channels::discord::run_gateway(agent).await {
                tracing::error!("Discord runtime error: {}", e);
            }
        })
    };

    // Start Telegram bot if configured
    #[cfg(feature = "telegram")]
    let telegram_handle = {
        let agent = agent.clone();
        crate::spawn_logged!("src/lib.rs:1666", async move {
            if let Err(e) = channels::telegram::serve(agent).await {
                tracing::error!("Telegram bot error: {}", e);
            }
        })
    };

    // Start ArkSentinel - unified background engine (scheduler, watchers, experience learning, ArkPulse)
    let sentinel_handles = sentinel::start(
        agent.clone(),
        sentinel::SentinelConfig::default(),
        shutdown_rx.clone(),
    );

    // Wait for shutdown signal
    wait_for_shutdown_signal().await?;
    println!();
    tracing::info!("Shutdown signal received");

    let _ = shutdown_tx.send(true);

    let mut http_handle = http_handle;
    match tokio::time::timeout(std::time::Duration::from_secs(10), &mut http_handle).await {
        Ok(Ok(())) => {}
        Ok(Err(e)) => tracing::warn!("HTTP server join failed during shutdown: {}", e),
        Err(_) => {
            tracing::warn!("HTTP server did not stop within 10s; aborting task");
            http_handle.abort();
        }
    }

    for mut handle in sentinel_handles {
        match tokio::time::timeout(std::time::Duration::from_secs(10), &mut handle).await {
            Ok(Ok(())) => {}
            Ok(Err(e)) => tracing::warn!("Sentinel task join failed during shutdown: {}", e),
            Err(_) => {
                tracing::warn!("Sentinel task did not stop within 10s; aborting task");
                handle.abort();
            }
        }
    }

    let mut matrix_handle = matrix_handle;
    match tokio::time::timeout(std::time::Duration::from_secs(10), &mut matrix_handle).await {
        Ok(Ok(())) => {}
        Ok(Err(e)) => tracing::warn!("Matrix task join failed during shutdown: {}", e),
        Err(_) => {
            tracing::warn!("Matrix task did not stop within 10s; aborting task");
            matrix_handle.abort();
        }
    }

    let mut discord_handle = discord_handle;
    match tokio::time::timeout(std::time::Duration::from_secs(10), &mut discord_handle).await {
        Ok(Ok(())) => {}
        Ok(Err(e)) => tracing::warn!("Discord task join failed during shutdown: {}", e),
        Err(_) => {
            tracing::warn!("Discord task did not stop within 10s; aborting task");
            discord_handle.abort();
        }
    }

    #[cfg(feature = "telegram")]
    telegram_handle.abort();

    Ok(())
}

#[cfg(test)]
mod tests {
    use super::{
        cli_chat_readiness, cli_chat_request_hints, render_cli_chat_onboarding_message,
        should_launch_cli_setup, CliChatReadiness, CLI_SETTINGS_URL, CLI_SETUP_COMMAND,
        CLI_SETUP_PROMPT,
    };
    use crate::core::config::{
        AgentConfig, ModelCapabilityTier, ModelCostTier, ModelHealthScope, ModelRole, ModelSlot,
    };
    use crate::core::LlmProvider;

    #[test]
    fn cli_chat_readiness_treats_default_config_as_unconfigured() {
        let readiness = cli_chat_readiness(&AgentConfig::default());

        assert_eq!(
            readiness,
            CliChatReadiness {
                chat_ready: false,
                is_fresh_unconfigured: true,
                configured_model_count: 0,
            }
        );
    }

    #[test]
    fn cli_chat_readiness_accepts_legacy_provider_setup() {
        let readiness = cli_chat_readiness(&AgentConfig {
            llm: LlmProvider::OpenAI {
                api_key: "test-key".to_string(),
                model: "gpt-4o".to_string(),
                base_url: None,
            },
            ..AgentConfig::default()
        });

        assert!(readiness.chat_ready);
        assert_eq!(readiness.configured_model_count, 1);
    }

    #[test]
    fn cli_chat_readiness_accepts_enabled_model_pool_slots() {
        let mut config = AgentConfig::default();
        config.model_pool.slots.push(ModelSlot {
            id: "primary".to_string(),
            label: "Primary".to_string(),
            role: ModelRole::Primary,
            provider: LlmProvider::OpenAI {
                api_key: "test-key".to_string(),
                model: "gpt-4o".to_string(),
                base_url: None,
            },
            enabled: true,
            capability_tier: ModelCapabilityTier::Balanced,
            cost_tier: ModelCostTier::Medium,
            auto_escalate: true,
            escalation_rank: 0,
            health_scope: ModelHealthScope::Provider,
        });

        let readiness = cli_chat_readiness(&config);

        assert!(readiness.chat_ready);
        assert_eq!(readiness.configured_model_count, 1);
    }

    #[test]
    fn interactive_cli_onboarding_message_includes_setup_prompt() {
        let message = render_cli_chat_onboarding_message(
            &CliChatReadiness {
                chat_ready: false,
                is_fresh_unconfigured: true,
                configured_model_count: 0,
            },
            true,
        );

        assert!(message.contains(CLI_SETUP_COMMAND));
        assert!(message.contains(CLI_SETTINGS_URL));
        assert!(message.contains(CLI_SETUP_PROMPT));
        assert!(!message.contains("CLI Chat"));
    }

    #[test]
    fn noninteractive_cli_onboarding_message_omits_setup_prompt() {
        let message = render_cli_chat_onboarding_message(
            &CliChatReadiness {
                chat_ready: false,
                is_fresh_unconfigured: true,
                configured_model_count: 0,
            },
            false,
        );

        assert!(message.contains(CLI_SETUP_COMMAND));
        assert!(message.contains(CLI_SETTINGS_URL));
        assert!(!message.contains(CLI_SETUP_PROMPT));
    }

    #[test]
    fn cli_chat_request_hints_match_direct_trusted_chat_surface() {
        let hints = cli_chat_request_hints();

        assert_eq!(
            hints.execution_surface,
            crate::actions::ActionExecutionSurface::Chat
        );
        assert!(hints.direct_user_intent);
        let principal = hints
            .caller_principal
            .expect("CLI chat should run as a trusted local operator request");
        assert_eq!(principal.auth_source, "cli");
        assert!(principal.trusted);
    }

    #[test]
    fn cli_setup_prompt_defaults_to_launch_unless_user_declines() {
        assert!(should_launch_cli_setup(""));
        assert!(should_launch_cli_setup("y"));
        assert!(!should_launch_cli_setup("n"));
        assert!(!should_launch_cli_setup("No"));
    }
}
