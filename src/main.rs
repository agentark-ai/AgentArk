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
use std::io::{BufRead, Write};
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
}

#[tokio::main]
async fn main() -> Result<()> {
    let args = Args::parse();

    // Initialize tracing
    // --debug enables verbose logging for agentark while keeping noisy deps quiet
    let default_filter = if args.debug {
        "debug,agentark=trace,sqlx::query=info,sea_orm=info,hyper=warn,reqwest=info,bollard=debug,tower=warn,h2=warn,rustls=warn".to_string()
    } else {
        format!(
            "{},sqlx::query=warn,sea_orm=warn,hyper=warn,reqwest=warn",
            args.log_level
        )
    };
    tracing_subscriber::registry()
        .with(
            tracing_subscriber::EnvFilter::try_from_default_env()
                .unwrap_or_else(|_| default_filter.parse().expect("Invalid log filter")),
        )
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
        println!(
            "│  API Key: {}...  │",
            &api_key[..std::cmp::min(api_key.len(), 32)]
        );
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

    // Start HTTP server for local IPC
    let http_handle = {
        let agent = agent.clone();
        tokio::spawn(async move {
            if let Err(e) = channels::http::serve(agent).await {
                tracing::error!("HTTP server error: {}", e);
            }
        })
    };

    // Start Telegram bot if configured
    #[cfg(feature = "telegram")]
    let _telegram_handle = {
        let agent = agent.clone();
        tokio::spawn(async move {
            if let Err(e) = channels::telegram::serve(agent).await {
                tracing::error!("Telegram bot error: {}", e);
            }
        })
    };

    // Start ArkSentinel — unified background engine (scheduler, watchers, consolidation, ArkPulse)
    let sentinel_handles = sentinel::start(agent.clone(), sentinel::SentinelConfig::default());

    // Wait for shutdown signal
    tokio::signal::ctrl_c().await?;
    println!();
    tracing::info!("Shutdown signal received");

    http_handle.abort();
    for h in sentinel_handles {
        h.abort();
    }

    Ok(())
}
