//! AgentArk - A secure, self-improving AI agent
//!
//! Features:
//! - Parallel thinking with multiple reasoning paths
//! - Sub-agent orchestration (Researcher, Coder, Analyst, etc.)
//! - Cognitive memory (episodic/semantic/procedural)
//! - Cryptographic execution proofs
//! - Sandboxed action execution (WASM + Docker)
//! - Native GUI (egui) + Telegram integration
//! - Local-first HTTP API

mod actions;
mod channels;
mod core;
mod crypto;
mod hooks;
mod identity;
mod integrations;
mod mcp;
mod memory;
mod proofs;
mod runtime;
mod safety;
mod security;
mod self_update;
mod sentinel;
mod storage;

#[cfg(feature = "gui")]
mod gui;

use anyhow::Result;
use clap::Parser;
use crossterm::event::{self, Event, KeyCode, KeyEventKind, KeyModifiers};
use crossterm::terminal::{disable_raw_mode, enable_raw_mode};
use std::io::{BufRead, IsTerminal, Read, Write};
use std::path::{Path, PathBuf};
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

    /// Run the setup wizard
    #[arg(long)]
    setup: bool,

    /// Log level (trace, debug, info, warn, error)
    #[arg(long, default_value = "info")]
    log_level: String,

    /// Enable debug logging (shows all internal details: LLM calls, actions, memory, Docker)
    #[arg(long, env = "AGENTARK_DEBUG")]
    debug: bool,

    /// Interactive CLI chat mode (like OpenClaw)
    #[arg(long)]
    chat: bool,

    /// Run one ArkPulse health check and print the latest snapshot
    #[arg(long)]
    pulse: bool,
}

#[tokio::main]
async fn main() -> Result<()> {
    let args = Args::parse();

    // Initialize tracing
    // --debug enables verbose logging for agentark while keeping noisy deps quiet
    let default_filter = if args.chat {
        "error".to_string()
    } else if args.debug {
        "debug,agentark=trace,sqlx::query=info,sea_orm=info,hyper=warn,reqwest=info,bollard=debug,tower=warn,h2=warn,rustls=warn".to_string()
    } else {
        format!(
            "{},sqlx::query=warn,sea_orm=warn,hyper=warn,reqwest=warn",
            args.log_level
        )
    };
    let env_filter = if args.chat {
        // In chat mode, force error-only — ignore RUST_LOG env var
        "error".parse().expect("Invalid log filter")
    } else {
        tracing_subscriber::EnvFilter::try_from_default_env()
            .unwrap_or_else(|_| default_filter.parse().expect("Invalid log filter"))
    };
    tracing_subscriber::registry()
        .with(env_filter)
        .with(tracing_subscriber::fmt::layer())
        .init();

    // Determine directories
    let dirs = directories::ProjectDirs::from("com", "agentark", "AgentArk")
        .expect("Failed to determine project directories");

    let config_dir = args
        .config
        .unwrap_or_else(|| dirs.config_dir().to_path_buf());
    let data_dir = args.data.unwrap_or_else(|| dirs.data_dir().to_path_buf());

    // Ensure directories exist
    std::fs::create_dir_all(&config_dir)?;
    std::fs::create_dir_all(&data_dir)?;

    // Check if this is first run (no config file exists)
    let config_path = config_dir.join("config.toml");
    let is_first_run = !config_path.exists();

    if is_first_run && !args.setup {
        // Print welcome message
        println!();
        println!("╔═══════════════════════════════════════════════════════════╗");
        println!("║                                                           ║");
        println!(
            "║                Welcome to AgentArk v{}                 ║",
            env!("CARGO_PKG_VERSION")
        );
        println!("║                                                           ║");
        println!("║   A secure, self-improving AI agent with:                 ║");
        println!("║   • Parallel thinking (multiple reasoning paths)          ║");
        println!("║   • Sub-agent orchestration                               ║");
        println!("║   • Cognitive memory (episodic/semantic/procedural)       ║");
        println!("║   • Sandboxed action execution (WASM/Docker)              ║");
        println!("║                                                           ║");
        println!("╚═══════════════════════════════════════════════════════════╝");
        println!();
    }

    tracing::info!("Starting AgentArk v{}", env!("CARGO_PKG_VERSION"));
    tracing::info!("Config directory: {}", config_dir.display());
    tracing::info!("Data directory: {}", data_dir.display());

    // Resolve master password → unified encryption key
    let master_mgr = crypto::master::MasterPasswordManager::new(&config_dir, &data_dir);

    let unified_key = if master_mgr.is_password_set() {
        let mut unlocked_key = None;

        // Try explicit credentials first.
        if let Ok(pw) = std::env::var("AGENTARK_MASTER_PASSWORD") {
            let pw = pw.trim().to_string();
            if !pw.is_empty() {
                tracing::info!("Using AGENTARK_MASTER_PASSWORD env var");
                match master_mgr.unlock(&pw) {
                    Ok(key) => unlocked_key = Some(key),
                    Err(_) => tracing::warn!("AGENTARK_MASTER_PASSWORD did not unlock master key"),
                }
            }
        }

        if unlocked_key.is_none() {
            if let Ok(pw) = std::fs::read_to_string("/run/secrets/agentark_master_key") {
                let pw = pw.trim().to_string();
                if !pw.is_empty() {
                    tracing::info!("Using Docker secret for unlock");
                    match master_mgr.unlock(&pw) {
                        Ok(key) => unlocked_key = Some(key),
                        Err(_) => tracing::warn!("Docker secret did not unlock master key"),
                    }
                }
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
    } else if is_first_run {
        // Optional explicit initial password for headless/bootstrap deployments.
        let initial_password = std::env::var("AGENTARK_MASTER_PASSWORD")
            .ok()
            .map(|s| s.trim().to_string())
            .filter(|s| !s.is_empty())
            .or_else(|| {
                std::fs::read_to_string("/run/secrets/agentark_master_key")
                    .ok()
                    .map(|s| s.trim().to_string())
                    .filter(|s| !s.is_empty())
            });

        if let Some(password) = initial_password {
            tracing::info!("Initializing master password from provided startup secret");
            Some(master_mgr.set_password(&password)?)
        } else {
            match master_mgr.initialize_bootstrap_password_if_needed()? {
                Some(key) => {
                    tracing::info!(
                        "Initialized per-install bootstrap encryption password. Set a custom master password in Security settings."
                    );
                    Some(key)
                }
                None => None,
            }
        }
    } else {
        // No master password exists — always initialize a bootstrap password so data is
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
    let agent = core::Agent::init(&config_dir, &data_dir, unified_key.clone()).await?;
    tracing::info!("Agent DID: {}", agent.identity.did());

    // Handle first run or explicit setup
    if args.setup || is_first_run {
        // In headless mode (Docker), skip interactive setup - just use defaults
        // Users can configure via the Web UI Settings page
        if args.headless && !args.setup {
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
            let agent = core::Agent::init(&config_dir, &data_dir, unified_key.clone()).await?;
            return run_headless(agent).await;
        }
    }

    if args.chat {
        return run_chat_repl(agent).await;
    }

    if args.pulse {
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

/// Interactive CLI chat mode — talk to the agent from your terminal.
async fn run_chat_repl(agent: core::Agent) -> Result<()> {
    if !std::io::stdin().is_terminal() {
        return run_chat_repl_noninteractive(agent).await;
    }

    let agent = std::sync::Arc::new(agent);
    let conversation_id = uuid::Uuid::new_v4().to_string();
    let mut auto_show_trace = false;

    println!();
    println!("╔═══════════════════════════════════════════════════════════╗");
    println!(
        "║           AgentArk v{} — CLI Chat                    ║",
        env!("CARGO_PKG_VERSION")
    );
    println!("╠═══════════════════════════════════════════════════════════╣");
    println!("║  Type your message and press Enter.                      ║");
    println!("║  Commands: /exit  /new  /help                            ║");
    println!("╚═══════════════════════════════════════════════════════════╝");
    println!();

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
                println!("\x1b[33m— New conversation started —\x1b[0m");
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

        println!("\x1b[3;90mThinking...\x1b[0m");
        std::io::stdout().flush()?;

        match agent
            .process_message(input.as_str(), "cli", Some(&conv_id), None)
            .await
        {
            Ok(response) => {
                let trace = agent.last_trace.read().await.clone();
                if auto_show_trace && !trace.id.trim().is_empty() {
                    print_cli_trace(&trace);
                    println!();
                }
                println!("\x1b[32magentark ➜\x1b[0m {}", response);
            }
            Err(e) => {
                eprintln!("\x1b[31merror:\x1b[0m {}", e);
            }
        }
        println!();
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

    println!(
        "{}╔═══════════════════════════════════════════════════════════╗\x1b[0m",
        status_color
    );
    println!(
        "{}║           AgentArk v{} — ArkPulse                 ║\x1b[0m",
        status_color,
        env!("CARGO_PKG_VERSION")
    );
    println!(
        "{}╚═══════════════════════════════════════════════════════════╝\x1b[0m",
        status_color
    );
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
    let response = agent
        .process_message(input, "cli", Some(&conversation_id), None)
        .await?;
    println!("{}", response);
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
    println!();
    println!("═══════════════════════════════════════════════════════════");
    println!("                    SETUP WIZARD");
    println!("═══════════════════════════════════════════════════════════");
    println!();
    println!("Your Agent Identity (DID):");
    println!("  {}", agent.identity.did());
    println!();

    let stdin = std::io::stdin();
    let mut stdout = std::io::stdout();

    // Step 1: LLM Provider
    println!("═══ Step 1: LLM Configuration ═══");
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
            print!("Ollama URL (default: http://localhost:11434): ");
            stdout.flush()?;
            let mut url = String::new();
            stdin.lock().read_line(&mut url)?;
            let url = url.trim();
            let url = if url.is_empty() {
                "http://localhost:11434"
            } else {
                url
            };

            print!("Model (default: llama3.2): ");
            stdout.flush()?;
            let mut model = String::new();
            stdin.lock().read_line(&mut model)?;
            let model = model.trim();
            let model = if model.is_empty() { "llama3.2" } else { model };

            core::LlmProvider::Ollama {
                base_url: url.to_string(),
                model: model.to_string(),
            }
        }
    };

    println!();

    // Step 2: Telegram (optional)
    println!("═══ Step 2: Telegram Configuration (Optional) ═══");
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

    println!("═══════════════════════════════════════════════════════════");
    println!("                  SETUP COMPLETE!");
    println!("═══════════════════════════════════════════════════════════");
    println!();
    println!("Configuration saved to: {}", config_dir.display());
    println!();
    println!("To start your agent:");
    println!("  GUI mode:      agentark");
    println!("  Headless mode: agentark --headless");
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

    println!();
    println!("╔═══════════════════════════════════════════════════════════╗");
    println!(
        "║             AgentArk v{} - Headless Mode               ║",
        env!("CARGO_PKG_VERSION")
    );
    println!("╚═══════════════════════════════════════════════════════════╝");
    println!();
    println!("DID: {}", agent.identity.did());
    println!();
    if let Some(ref api_key) = agent.api_key {
        println!("┌─────────────────────────────────────────────────────────┐");
        println!("│  Authentication enabled                                 │");
        println!("│  API Key: {}...  │", mask_api_key_for_console(api_key));
        println!("│  Use: Authorization: Bearer <key>                       │");
        println!("└─────────────────────────────────────────────────────────┘");
        println!();
    }
    let bind_addr = std::env::var("AGENTARK_BIND").unwrap_or_else(|_| "127.0.0.1:8990".to_string());
    println!("┌─────────────────────────────────────────────────────────┐");
    println!("│  Web UI:   http://{}                       │", bind_addr);
    println!("├─────────────────────────────────────────────────────────┤");
    println!("│  API Endpoints:                                         │");
    println!("│    GET  /health  - Health check (no auth)               │");
    println!("│    GET  /status  - Agent status                         │");
    println!("│    POST /chat    - Chat with agent                      │");
    println!("│    GET  /skills  - List skills                          │");
    println!("│    GET  /tasks   - List tasks                           │");
    println!("└─────────────────────────────────────────────────────────┘");
    println!();
    let tunnel_auto = std::env::var("AGENTARK_TUNNEL")
        .map(|v| v.eq_ignore_ascii_case("true") || v == "1")
        .unwrap_or(false);
    if tunnel_auto {
        println!("Remote access: ENABLED (AGENTARK_TUNNEL=true)");
        println!("  Tunnel URL will appear in logs shortly...");
    } else {
        println!("Remote access (VPS): AGENTARK_TUNNEL=true docker compose up -d");
        println!("  Or enable in Settings → Advanced → Remote Access");
    }
    println!();
    println!("Press Ctrl+C to stop");
    println!();

    let agent = std::sync::Arc::new(tokio::sync::RwLock::new(agent));

    // Daily brief task is opt-in via Settings — not auto-created on fresh install

    let (shutdown_tx, shutdown_rx) = tokio::sync::watch::channel(false);

    // Start HTTP server for local IPC
    let http_handle = {
        let agent = agent.clone();
        let shutdown = shutdown_rx.clone();
        tokio::spawn(async move {
            if let Err(e) = channels::http::serve(agent, shutdown).await {
                tracing::error!("HTTP server error: {}", e);
            }
        })
    };

    // Start Telegram bot if configured
    #[cfg(feature = "telegram")]
    let telegram_handle = {
        let agent = agent.clone();
        tokio::spawn(async move {
            if let Err(e) = channels::telegram::serve(agent).await {
                tracing::error!("Telegram bot error: {}", e);
            }
        })
    };

    // Start ArkSentinel — unified background engine (scheduler, watchers, consolidation, ArkPulse)
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

    #[cfg(feature = "telegram")]
    telegram_handle.abort();

    Ok(())
}
