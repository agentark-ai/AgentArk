//! Action Runtime - WASM Sandbox + Docker + Transactional Execution
//!
//! Based on arXiv:2512.12806 "Fault-Tolerant Sandboxing"
//!
//! Features:
//! - WASM sandbox for lightweight, fast action execution
//! - Docker sandbox for heavier/untrusted operations
//! - Transactional filesystem with rollback capability

mod sandbox;
mod transaction;

pub use sandbox::{ActionSandbox, SandboxMode};
pub use transaction::TransactionManager;

use anyhow::Result;
#[cfg(feature = "docker")]
use futures::TryStreamExt;
use serde::{Deserialize, Serialize};
use std::collections::{BTreeMap, HashMap, HashSet};
use std::path::{Path, PathBuf};

use crate::actions::{ActionDef, ActionSource};
use crate::core::config::{AgentConfig, SecureConfigManager};

/// Runtime configuration
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct RuntimeConfig {
    pub default_sandbox: SandboxMode,
    pub wasm_memory_limit: u64,
    pub docker_image: String,
    pub enable_rollback: bool,
    pub snapshot_dir: PathBuf,
}

impl Default for RuntimeConfig {
    fn default() -> Self {
        Self {
            default_sandbox: SandboxMode::Wasm,
            wasm_memory_limit: 256 * 1024 * 1024, // 256MB
            docker_image: "agentark-sandbox:latest".to_string(),
            enable_rollback: true,
            snapshot_dir: PathBuf::from("snapshots"),
        }
    }
}

/// The action runtime that manages execution
pub struct ActionRuntime {
    config: RuntimeConfig,
    _sandbox: ActionSandbox,
    /// Transactions wrapped in Mutex for concurrent access
    transactions: tokio::sync::Mutex<TransactionManager>,
    /// Actions wrapped in RwLock for concurrent access
    actions: tokio::sync::RwLock<HashMap<String, LoadedAction>>,
    /// Bundled actions explicitly disabled by user (persisted on disk)
    disabled_actions: tokio::sync::RwLock<HashSet<String>>,
    disabled_actions_file: PathBuf,
    actions_dir: PathBuf,
    config_dir: PathBuf,
    /// Shared task queue for list_tasks action
    task_queue: Option<std::sync::Arc<tokio::sync::RwLock<crate::core::TaskQueue>>>,
    /// Action security guard for integrity, static analysis, permissions, injection detection
    action_guard: Option<std::sync::Arc<crate::security::ActionGuard>>,
    /// Shared storage for expense + entity operations
    storage: Option<crate::storage::Storage>,
    /// MCP registry for external tools/resources
    mcp_registry: Option<std::sync::Arc<tokio::sync::RwLock<crate::mcp::registry::McpRegistry>>>,
}

/// A loaded action ready for execution
struct LoadedAction {
    info: ActionDef,
    wasm_module: Option<Vec<u8>>,
    /// Workflow content from ACTION.md (for LLM-driven actions)
    workflow_content: Option<String>,
    /// Optional MCP binding (external tool/resource)
    mcp_binding: Option<McpBinding>,
}

#[derive(Debug, Clone)]
pub struct McpBinding {
    pub server_id: String,
    pub kind: McpBindingKind,
}

#[derive(Debug, Clone)]
pub enum McpBindingKind {
    Tool { name: String },
    Resource { uri: String },
}

pub const WORKFLOW_ACTION_MARKER: &str = "__WORKFLOW_ACTION__:";
pub const WORKFLOW_MISSING_INPUTS_MARKER: &str = "__WORKFLOW_MISSING_INPUTS__:";

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct WorkflowMissingInputsPayload {
    pub action: String,
    pub missing: Vec<String>,
    pub required: Vec<String>,
    pub provided: Vec<String>,
    pub query: String,
}

pub fn parse_workflow_action_marker(output: &str) -> Option<(String, String)> {
    let payload = output.strip_prefix(WORKFLOW_ACTION_MARKER)?;
    let mut parts = payload.splitn(2, ':');
    let action = parts.next()?.trim();
    if action.is_empty() {
        return None;
    }
    let query = parts.next().unwrap_or("").to_string();
    Some((action.to_string(), query))
}

pub fn parse_workflow_missing_inputs_marker(output: &str) -> Option<WorkflowMissingInputsPayload> {
    let payload = output.strip_prefix(WORKFLOW_MISSING_INPUTS_MARKER)?;
    serde_json::from_str::<WorkflowMissingInputsPayload>(payload).ok()
}

/// Isolation level for ephemeral Docker containers
#[cfg(feature = "docker")]
#[derive(Clone, Copy)]
enum ContainerIsolation {
    /// Strict: read-only root, no network, noexec /tmp. For shell commands.
    Strict,
    /// Standard: writable fs, network allowed (for pip/npm install), but still
    /// memory/CPU/PID limited, ephemeral, and auto-removed. For code execution.
    Standard,
}

impl ActionRuntime {
    fn load_disabled_actions(path: &Path) -> HashSet<String> {
        let raw = match std::fs::read(path) {
            Ok(v) => v,
            Err(_) => return HashSet::new(),
        };
        serde_json::from_slice::<Vec<String>>(&raw)
            .map(|v| {
                v.into_iter()
                    .map(|s| s.trim().to_string())
                    .filter(|s| !s.is_empty())
                    .collect()
            })
            .unwrap_or_default()
    }

    async fn save_disabled_actions(&self) -> Result<()> {
        let mut list: Vec<String> = self.disabled_actions.read().await.iter().cloned().collect();
        list.sort();
        let raw = serde_json::to_vec_pretty(&list)?;
        tokio::fs::write(&self.disabled_actions_file, raw).await?;
        Ok(())
    }

    /// Get the data directory (parent of actions_dir)
    fn data_dir(&self) -> &Path {
        self.actions_dir.parent().unwrap_or(&self.actions_dir)
    }

    pub async fn new(config_dir: &Path, data_dir: &Path) -> Result<Self> {
        let config_path = config_dir.join("runtime.toml");
        let config: RuntimeConfig = if config_path.exists() {
            let content = std::fs::read_to_string(&config_path)?;
            toml::from_str(&content)?
        } else {
            let default = RuntimeConfig::default();
            let content = toml::to_string_pretty(&default)?;
            std::fs::write(&config_path, content)?;
            default
        };

        // User skills go in data dir
        let actions_dir = data_dir.join("skills");
        std::fs::create_dir_all(&actions_dir)?;
        let disabled_actions_file = data_dir.join("disabled_actions.json");
        let disabled_actions = Self::load_disabled_actions(&disabled_actions_file);

        let snapshot_dir = data_dir.join(&config.snapshot_dir);
        std::fs::create_dir_all(&snapshot_dir)?;

        let sandbox = ActionSandbox::new(&config)?;
        let transactions = TransactionManager::new(snapshot_dir);

        let runtime = Self {
            config,
            _sandbox: sandbox,
            transactions: tokio::sync::Mutex::new(transactions),
            actions: tokio::sync::RwLock::new(HashMap::new()),
            disabled_actions: tokio::sync::RwLock::new(disabled_actions),
            disabled_actions_file,
            actions_dir: actions_dir.clone(),
            config_dir: config_dir.to_path_buf(),
            task_queue: None,
            action_guard: None,
            storage: None,
            mcp_registry: None,
        };

        Ok(runtime)
    }

    /// Set shared task queue reference (called from Agent::init)
    pub fn set_task_queue(
        &mut self,
        tasks: std::sync::Arc<tokio::sync::RwLock<crate::core::TaskQueue>>,
    ) {
        self.task_queue = Some(tasks);
    }

    /// Set action security guard (called from Agent::init before load_all_actions)
    pub fn set_action_guard(&mut self, guard: std::sync::Arc<crate::security::ActionGuard>) {
        self.action_guard = Some(guard);
    }

    /// Set shared storage reference for expense/entity operations (called from Agent::init)
    pub fn set_storage(&mut self, storage: crate::storage::Storage) {
        self.storage = Some(storage);
    }

    /// Set MCP registry (called from Agent::init)
    pub fn set_mcp_registry(
        &mut self,
        registry: std::sync::Arc<tokio::sync::RwLock<crate::mcp::registry::McpRegistry>>,
    ) {
        self.mcp_registry = Some(registry);
    }

    /// Load all actions (builtin + bundled + user). Call AFTER set_action_guard.
    pub async fn load_all_actions(&self) -> Result<()> {
        // Load built-in actions
        self.load_builtin_actions().await?;

        // Load markdown skills from the app's skills directory (bundled with app)
        let app_skills_dir = std::path::Path::new("/app/skills");
        if app_skills_dir.exists() {
            tracing::info!("Loading bundled skills from {:?}", app_skills_dir);
            self.load_markdown_actions(app_skills_dir, ActionSource::Bundled)
                .await?;
        }

        // Also check relative skills dir (for local development).
        let local_skills_dir = std::env::current_dir()
            .map(|d| d.join("skills"))
            .unwrap_or_else(|_| std::path::PathBuf::from("./skills"));
        if local_skills_dir.exists() && local_skills_dir != app_skills_dir {
            tracing::info!("Loading local skills from {:?}", local_skills_dir);
            self.load_markdown_actions(&local_skills_dir, ActionSource::Bundled)
                .await?;
        }

        // Load user-added skills from data dir
        if self.actions_dir.exists() {
            tracing::info!("Loading user skills from {:?}", self.actions_dir);
            self.load_markdown_actions(&self.actions_dir, ActionSource::Custom)
                .await?;
        }

        Ok(())
    }

    /// Load built-in actions
    async fn load_builtin_actions(&self) -> Result<()> {
        // File operations
        self.register_builtin_action(ActionDef {
            name: "file_read".to_string(),
            description: "Read contents of a file".to_string(),
            version: "1.0.0".to_string(),
            input_schema: serde_json::json!({
                "type": "object",
                "properties": {
                    "path": { "type": "string", "description": "File path to read" }
                },
                "required": ["path"]
            }),
            capabilities: vec!["file_read".to_string()],
            sandbox_mode: Some(SandboxMode::Native),
            source: ActionSource::System,
            file_path: None,
        })
        .await;

        self.register_builtin_action(ActionDef {
            name: "file_write".to_string(),
            description: "Write contents to a file".to_string(),
            version: "1.0.0".to_string(),
            input_schema: serde_json::json!({
                "type": "object",
                "properties": {
                    "path": { "type": "string", "description": "File path to write" },
                    "content": { "type": "string", "description": "Content to write" }
                },
                "required": ["path", "content"]
            }),
            capabilities: vec!["file_write".to_string()],
            sandbox_mode: Some(SandboxMode::Native),
            source: ActionSource::System,
            file_path: None,
        })
        .await;

        // HTTP requests
        self.register_builtin_action(ActionDef {
            name: "http_get".to_string(),
            description: "Make an HTTP GET request".to_string(),
            version: "1.0.0".to_string(),
            input_schema: serde_json::json!({
                "type": "object",
                "properties": {
                    "url": { "type": "string", "description": "URL to fetch" },
                    "headers": { "type": "object", "description": "Optional headers" }
                },
                "required": ["url"]
            }),
            capabilities: vec!["network".to_string()],
            sandbox_mode: Some(SandboxMode::Wasm),
            source: ActionSource::System,
            file_path: None,
        })
        .await;

        self.register_builtin_action(ActionDef {
            name: "app_restart".to_string(),
            description: "Restart an existing deployed app from its saved metadata. Use after file_write edits to /app/data/apps/<id>/..., when a deployed app needs reload, or when the user asks to restart or re-run an existing app. Prefer app_id from app_inspect when available.".to_string(),
            version: "1.0.0".to_string(),
            input_schema: serde_json::json!({
                "type": "object",
                "properties": {
                    "app_id": {
                        "type": "string",
                        "description": "Exact deployed app ID to restart. Preferred when already known."
                    },
                    "query": {
                        "type": "string",
                        "description": "Optional app title or app ID to match when app_id is not known."
                    }
                },
                "anyOf": [
                    { "required": ["app_id"] },
                    { "required": ["query"] }
                ]
            }),
            capabilities: vec!["app_hosting".to_string()],
            sandbox_mode: Some(SandboxMode::Native),
            source: ActionSource::System,
            file_path: None,
        })
        .await;

        // Shell commands (requires approval by default)
        self.register_builtin_action(ActionDef {
            name: "shell".to_string(),
            description: "Execute a shell command".to_string(),
            version: "1.0.0".to_string(),
            input_schema: serde_json::json!({
                "type": "object",
                "properties": {
                    "command": { "type": "string", "description": "Command to execute" },
                    "cwd": { "type": "string", "description": "Working directory" }
                },
                "required": ["command"]
            }),
            capabilities: vec!["shell".to_string()],
            sandbox_mode: Some(SandboxMode::Docker),
            source: ActionSource::System,
            file_path: None,
        })
        .await;

        // Clipboard
        self.register_builtin_action(ActionDef {
            name: "clipboard_read".to_string(),
            description: "Read from clipboard".to_string(),
            version: "1.0.0".to_string(),
            input_schema: serde_json::json!({
                "type": "object",
                "properties": {}
            }),
            capabilities: vec!["clipboard_read".to_string()],
            sandbox_mode: Some(SandboxMode::Native),
            source: ActionSource::System,
            file_path: None,
        })
        .await;

        self.register_builtin_action(ActionDef {
            name: "clipboard_write".to_string(),
            description: "Write to clipboard".to_string(),
            version: "1.0.0".to_string(),
            input_schema: serde_json::json!({
                "type": "object",
                "properties": {
                    "content": { "type": "string", "description": "Content to copy" }
                },
                "required": ["content"]
            }),
            capabilities: vec!["clipboard_write".to_string()],
            sandbox_mode: Some(SandboxMode::Native),
            source: ActionSource::System,
            file_path: None,
        })
        .await;

        // Scheduler
        self.register_builtin_action(ActionDef {
            name: "schedule_task".to_string(),
            description: "Schedule a recurring or one-time task. Use 'cron' for recurring (e.g., daily at 9am = '0 9 * * *') or 'at' for one-time (ISO timestamp).".to_string(),
            version: "1.0.0".to_string(),
            input_schema: serde_json::json!({
                "type": "object",
                "properties": {
                    "task": { "type": "string", "description": "Task description - what to do" },
                    "cron": { "type": "string", "description": "Cron expression for recurring tasks. Format: 'minute hour day month weekday'. Examples: '0 9 * * *' = daily at 9am, '0 9 * * 1' = every Monday 9am, '*/30 * * * *' = every 30 minutes" },
                    "at": { "type": "string", "description": "ISO 8601 timestamp for one-time task. Example: '2026-02-06T09:00:00+05:30'" }
                },
                "required": ["task"]
            }),
            capabilities: vec!["scheduler".to_string()],
            sandbox_mode: Some(SandboxMode::Native),
            source: ActionSource::System,
            file_path: None,
        }).await;

        // Background watcher — poll an action until a condition is met, then act
        // Tunnel control for remote UI access
        self.register_builtin_action(ActionDef {
            name: "tunnel_control".to_string(),
            description: "Manage public UI tunnel. Use action=start to create an external link, action=status to check current URL, action=stop to disable it.".to_string(),
            version: "1.0.0".to_string(),
            input_schema: serde_json::json!({
                "type": "object",
                "properties": {
                    "action": { "type": "string", "enum": ["start", "stop", "status"], "description": "Tunnel operation" }
                },
                "required": ["action"]
            }),
            capabilities: vec!["network".to_string()],
            sandbox_mode: Some(SandboxMode::Native),
            source: ActionSource::System,
            file_path: None,
        }).await;
        self.register_builtin_action(ActionDef {
            name: "watch".to_string(),
            description: "Spawn a background watcher that polls an action at regular intervals until a condition is met, then executes follow-up instructions. Use when asked to 'watch for', 'wait for', 'monitor', 'let me know when', or 'poll until'. The watcher runs autonomously and notifies the user when triggered or timed out.".to_string(),
            version: "1.0.0".to_string(),
            input_schema: serde_json::json!({
                "type": "object",
                "properties": {
                    "description": { "type": "string", "description": "What this watcher does (shown in UI)" },
                    "poll_action": { "type": "string", "description": "Action to poll (e.g. 'gmail_scan', 'web_search', 'http_get')" },
                    "poll_arguments": { "type": "object", "description": "Arguments for the poll action" },
                    "condition_contains": { "type": "string", "description": "Trigger when result contains this keyword (case-insensitive)" },
                    "condition_matches": { "type": "string", "description": "Trigger when result matches this regex pattern" },
                    "condition_custom": { "type": "string", "description": "Natural language condition description" },
                    "on_trigger": { "type": "string", "description": "What to do when condition is met — natural language instructions for the agent" },
                    "interval_secs": { "type": "integer", "description": "Seconds between polls (default: 60)" },
                    "timeout_secs": { "type": "integer", "description": "Max seconds to watch before giving up (default: 1800 = 30 min)" },
                    "notify_channel": { "type": "string", "description": "Channel to notify: 'telegram' or 'http' (default: 'telegram')" }
                },
                "required": ["description", "poll_action", "on_trigger"]
            }),
            capabilities: vec!["watcher".to_string()],
            sandbox_mode: Some(SandboxMode::Native),
            source: ActionSource::System,
            file_path: None,
        }).await;

        // Generic connector scaffold: pagination + rate-limit + auth-refresh + retries.
        self.register_builtin_action(ActionDef {
            name: "connector_request".to_string(),
            description: "Reusable connector scaffold for API/data collectors. Executes HTTP requests with built-in pagination, rate-limit spacing, auth-refresh callbacks, and retry/backoff behavior. Use this to build new integrations dynamically without hardcoding providers.".to_string(),
            version: "1.0.0".to_string(),
            input_schema: serde_json::json!({
                "type": "object",
                "properties": {
                    "url": { "type": "string", "description": "Target URL" },
                    "method": { "type": "string", "enum": ["get", "post", "put", "patch", "delete"], "description": "HTTP method (default: get)" },
                    "headers": { "type": "object", "description": "HTTP headers" },
                    "query": { "type": "object", "description": "Query params" },
                    "body": { "description": "Optional JSON body" },
                    "timeout_secs": { "type": "integer", "description": "Per-request timeout seconds (default: 30)" },
                    "rate_limit_ms": { "type": "integer", "description": "Min delay between requests/pages in ms" },
                    "retry": {
                        "type": "object",
                        "properties": {
                            "max_attempts": { "type": "integer" },
                            "initial_backoff_ms": { "type": "integer" },
                            "max_backoff_ms": { "type": "integer" },
                            "jitter_ratio": { "type": "number" },
                            "retry_on_status": { "type": "array", "items": { "type": "integer" } }
                        }
                    },
                    "pagination": {
                        "type": "object",
                        "properties": {
                            "mode": { "type": "string", "enum": ["none", "page", "cursor"] },
                            "page_param": { "type": "string" },
                            "cursor_param": { "type": "string" },
                            "items_path": { "type": "string" },
                            "next_cursor_path": { "type": "string" },
                            "start_page": { "type": "integer" },
                            "max_pages": { "type": "integer" },
                            "page_size_param": { "type": "string" },
                            "page_size": { "type": "integer" }
                        }
                    },
                    "auth_refresh": {
                        "type": "object",
                        "properties": {
                            "action": { "type": "string", "description": "Action to call on auth expiry (401/403)" },
                            "arguments": { "description": "Arguments for refresh action" },
                            "retry_statuses": { "type": "array", "items": { "type": "integer" } }
                        },
                        "required": ["action"]
                    }
                },
                "required": ["url"]
            }),
            capabilities: vec!["network".to_string()],
            sandbox_mode: Some(SandboxMode::Native),
            source: ActionSource::System,
            file_path: None,
        }).await;

        // First-class pipeline DAG spec compiler.
        self.register_builtin_action(ActionDef {
            name: "pipeline_compile".to_string(),
            description: "Validate and compile a pipeline DAG spec (dependency checks + topological order). Optionally persist the spec for scheduled runs.".to_string(),
            version: "1.0.0".to_string(),
            input_schema: serde_json::json!({
                "type": "object",
                "properties": {
                    "spec": { "type": "object", "description": "Pipeline spec" },
                    "save": { "type": "boolean", "description": "Persist spec to storage (default: true)" }
                },
                "required": ["spec"]
            }),
            capabilities: vec!["orchestration".to_string()],
            sandbox_mode: Some(SandboxMode::Native),
            source: ActionSource::System,
            file_path: None,
        }).await;

        // Execute a compiled pipeline with retry/idempotency guards.
        self.register_builtin_action(ActionDef {
            name: "pipeline_run".to_string(),
            description: "Run a pipeline DAG from inline spec or saved pipeline_name. Supports retry/backoff/idempotency per node, dependency-aware execution, and persisted run traces.".to_string(),
            version: "1.0.0".to_string(),
            input_schema: serde_json::json!({
                "type": "object",
                "properties": {
                    "pipeline_name": { "type": "string", "description": "Saved pipeline name" },
                    "spec": { "type": "object", "description": "Inline pipeline spec (overrides pipeline_name)" },
                    "dry_run": { "type": "boolean", "description": "Validate/plan without executing" },
                    "context": { "type": "object", "description": "Template context values for node args/idempotency keys" },
                    "allow_privileged": { "type": "boolean", "description": "Allow privileged node actions (default: false)" }
                }
            }),
            capabilities: vec!["orchestration".to_string()],
            sandbox_mode: Some(SandboxMode::Native),
            source: ActionSource::System,
            file_path: None,
        }).await;

        // Typed signal ranking + consensus primitive.
        self.register_builtin_action(ActionDef {
            name: "signal_consensus".to_string(),
            description: "Rank and reconcile signals using typed scoring weights and optional reviewer perspectives. Returns top prioritized signals for daily decisioning.".to_string(),
            version: "1.0.0".to_string(),
            input_schema: serde_json::json!({
                "type": "object",
                "properties": {
                    "signals": { "type": "array", "items": { "type": "object" }, "description": "Signals with impact/confidence/effort + payload" },
                    "weights": {
                        "type": "object",
                        "properties": {
                            "impact": { "type": "number" },
                            "confidence": { "type": "number" },
                            "effort": { "type": "number" }
                        }
                    },
                    "perspectives": { "type": "array", "items": { "type": "object" }, "description": "Optional reviewer perspectives with custom weights" },
                    "top_k": { "type": "integer", "description": "Max ranked signals to return (default: 20)" }
                },
                "required": ["signals"]
            }),
            capabilities: vec!["analytics".to_string()],
            sandbox_mode: Some(SandboxMode::Native),
            source: ActionSource::System,
            file_path: None,
        }).await;

        // Gmail scan
        self.register_builtin_action(ActionDef {
            name: "gmail_scan".to_string(),
            description: "Read and scan the user's Gmail inbox. Use when asked to check email, find emails, look for meetings/invites/receipts, or anything email-related. Returns sender, subject, date, and labels for each message. When called with no query, automatically runs smart multi-query (important, primary, recent, starred) to surface all relevant emails. Only set query/labels when the user asks for something specific.".to_string(),
            version: "1.0.0".to_string(),
            input_schema: serde_json::json!({
                "type": "object",
                "properties": {
                    "query": { "type": "string", "description": "Gmail search query (e.g., 'from:sarah', 'subject:meeting', 'newer_than:2d'). Leave empty to use smart auto-scan which covers important, primary, recent, and starred emails automatically." },
                    "labels": { "type": "array", "items": { "type": "string" }, "description": "Label IDs to filter: INBOX (default), SPAM, IMPORTANT, UNREAD, STARRED, SENT, DRAFT, TRASH. Leave empty for smart auto-scan." },
                    "max_results": { "type": "integer", "description": "Max messages per query when using a specific query (default 20). Ignored for smart auto-scan." }
                }
            }),
            capabilities: vec!["gmail".to_string()],
            sandbox_mode: Some(SandboxMode::Native),
            source: ActionSource::System,
            file_path: None,
        }).await;

        self.register_builtin_action(ActionDef {
            name: "gmail_reply".to_string(),
            description: "Send an email or reply via the user's Gmail. Use when asked to send, reply to, compose, or draft an email. Can reply to existing threads using thread_id.".to_string(),
            version: "1.0.0".to_string(),
            input_schema: serde_json::json!({
                "type": "object",
                "properties": {
                    "to": { "type": "string", "description": "Recipient email address" },
                    "subject": { "type": "string", "description": "Email subject line" },
                    "body": { "type": "string", "description": "Email body text (plain text)" },
                    "thread_id": { "type": "string", "description": "Gmail thread ID to reply to (from gmail_scan results)" }
                },
                "required": ["to", "subject", "body"]
            }),
            capabilities: vec!["gmail".to_string()],
            sandbox_mode: Some(SandboxMode::Native),
            source: ActionSource::System,
            file_path: None,
        }).await;

        // Web search
        self.register_builtin_action(ActionDef {
            name: "web_search".to_string(),
            description: "Search the web for current information. Use when asked about news, facts, prices, weather, or anything that needs up-to-date data.".to_string(),
            version: "1.0.0".to_string(),
            input_schema: serde_json::json!({
                "type": "object",
                "properties": {
                    "query": { "type": "string", "description": "Search query" },
                    "num_results": { "type": "integer", "description": "Number of results (default 5)" },
                    "backend": { "type": "string", "description": "Search backend: duckduckgo, brave, serper, searxng" }
                },
                "required": ["query"]
            }),
            capabilities: vec!["network".to_string()],
            sandbox_mode: Some(SandboxMode::Native),
            source: ActionSource::System,
            file_path: None,
        }).await;

        // Research
        self.register_builtin_action(ActionDef {
            name: "research".to_string(),
            description: "Conduct deep research on a topic by searching and analyzing multiple sources. Use for complex questions that need thorough investigation beyond a simple web search.".to_string(),
            version: "1.0.0".to_string(),
            input_schema: serde_json::json!({
                "type": "object",
                "properties": {
                    "query": { "type": "string", "description": "Research topic or question" },
                    "max_sources": { "type": "integer", "description": "Maximum sources to examine (default 5)" },
                    "depth": { "type": "string", "description": "Research depth: quick, standard, deep" },
                    "include_sources": { "type": "boolean", "description": "Include source URLs" }
                },
                "required": ["query"]
            }),
            capabilities: vec!["network".to_string()],
            sandbox_mode: Some(SandboxMode::Native),
            source: ActionSource::System,
            file_path: None,
        }).await;

        // Code execution sandbox
        self.register_builtin_action(ActionDef {
            name: "code_execute".to_string(),
            description: "Execute code in an isolated Docker sandbox. Supports Python, JavaScript, TypeScript, Bash, Ruby, PHP, Perl, Lua, R, Java, C, C++, Go, Rust, Swift, Kotlin, and Jupyter notebooks (.ipynb). Use when the user asks to run, execute, or test code. Dependencies like pip/npm install work automatically. For ML/data science and EDA, use language='jupyter' to create executable notebooks with visualizations — they get executed and returned as downloadable .ipynb files. When the user has attached files, pass their local paths in the 'files' array — they'll be available at /data/<filename> inside the sandbox.".to_string(),
            version: "1.0.0".to_string(),
            input_schema: serde_json::json!({
                "type": "object",
                "properties": {
                    "language": { "type": "string", "description": "Programming language: python, javascript, typescript, bash, ruby, php, perl, lua, r, java, c, cpp, go, rust, swift, kotlin, jupyter. Use 'jupyter' for EDA/ML notebooks with visualizations." },
                    "code": { "type": "string", "description": "Code to execute. For jupyter: provide valid .ipynb JSON content (notebook format). For other languages: plain code. Can include dependency installation. When files are provided, access them at /data/<filename>." },
                    "env": { "type": "object", "description": "Optional environment variables (values may include {{secret:...}} / {{env:...}} placeholders).", "additionalProperties": { "type": "string" } },
                    "files": { "type": "array", "items": { "type": "string" }, "description": "Local file paths of user-attached files to inject into the sandbox at /data/. Pass the 'local_path' values from uploaded files." }
                },
                "required": ["language", "code"]
            }),
            capabilities: vec!["code_execute".to_string()],
            sandbox_mode: Some(SandboxMode::Docker),
            source: ActionSource::System,
            file_path: None,
        }).await;

        // List tasks/goals/routines
        self.register_builtin_action(ActionDef {
            name: "list_tasks".to_string(),
            description: "List pending tasks, goals, routines, and scheduled items. Use when the user asks about their pending goals, tasks, agenda, or what's scheduled.".to_string(),
            version: "1.0.0".to_string(),
            input_schema: serde_json::json!({
                "type": "object",
                "properties": {
                    "filter": { "type": "string", "description": "Filter: 'all', 'pending', 'goals', 'routines', 'completed', 'failed'. Default: 'pending'" }
                }
            }),
            capabilities: vec![],
            sandbox_mode: Some(SandboxMode::Native),
            source: ActionSource::System,
            file_path: None,
        }).await;

        // Browser automation - fetch and extract content from web pages
        self.register_builtin_action(ActionDef {
            name: "browse".to_string(),
            description: "Fetch a web page and extract content. Use when asked to visit a URL, read a web page, scrape content, or check a website. Returns extracted text, links, or page title depending on the 'extract' parameter.".to_string(),
            version: "1.0.0".to_string(),
            input_schema: serde_json::json!({
                "type": "object",
                "properties": {
                    "url": { "type": "string", "description": "URL to fetch (must include http:// or https://)" },
                    "extract": { "type": "string", "description": "What to extract: 'text' (default, main text content), 'links' (all hyperlinks), 'title' (page title), 'all' (text + links + title)" }
                },
                "required": ["url"]
            }),
            capabilities: vec!["network".to_string()],
            sandbox_mode: Some(SandboxMode::Native),
            source: ActionSource::System,
            file_path: None,
        }).await;

        // Image generation
        self.register_builtin_action(ActionDef {
            name: "generate_image".to_string(),
            description: "Generate an image using AI. Use when asked to create, generate, draw, or make an image, picture, illustration, or visual.".to_string(),
            version: "1.0.0".to_string(),
            input_schema: serde_json::json!({
                "type": "object",
                "properties": {
                    "prompt": { "type": "string", "description": "Description of the image to generate" },
                    "negative_prompt": { "type": "string", "description": "What NOT to include (optional)" },
                    "width": { "type": "integer", "description": "Image width in pixels (default 1024)" },
                    "height": { "type": "integer", "description": "Image height in pixels (default 1024)" },
                    "style": { "type": "string", "description": "Art style (optional)" }
                },
                "required": ["prompt"]
            }),
            capabilities: vec!["image_generation".to_string()],
            sandbox_mode: Some(SandboxMode::Native),
            source: ActionSource::System,
            file_path: None,
        }).await;

        // Action management — create/update/delete/list custom actions via chat
        self.register_builtin_action(ActionDef {
            name: "manage_actions".to_string(),
            description: "Create, update, delete, or list custom actions/workflows. Use when the user wants to add a new integration, action, or workflow.".to_string(),
            version: "1.0.0".to_string(),
            input_schema: serde_json::json!({
                "type": "object",
                "properties": {
                    "operation": {
                        "type": "string",
                        "enum": ["create", "update", "delete", "list"],
                        "description": "Operation to perform"
                    },
                    "name": {
                        "type": "string",
                        "description": "Action name in kebab-case (e.g. 'check-weather'). Required for create/update/delete."
                    },
                    "content": {
                        "type": "string",
                        "description": "ACTION.md content with YAML frontmatter. Required for create/update. Format:\n---\nname: action-name\ndescription: What this action does\nversion: \"1.0.0\"\n---\n\n# Action Title\n\n## Steps\n..."
                    }
                },
                "required": ["operation"]
            }),
            capabilities: vec![],
            sandbox_mode: Some(SandboxMode::Native),
            source: ActionSource::System,
            file_path: None,
        }).await;

        // PDF generation — creates PDF documents from content
        self.register_builtin_action(ActionDef {
            name: "pdf_generate".to_string(),
            description: "Generate a PDF document. Use when asked to create a PDF, report, invoice, or document. Supports styles: report, letter, invoice, plain.".to_string(),
            version: "1.0.0".to_string(),
            input_schema: serde_json::json!({
                "type": "object",
                "properties": {
                    "content": { "type": "string", "description": "Text content for the PDF" },
                    "title": { "type": "string", "description": "Document title (optional)" },
                    "filename": { "type": "string", "description": "Output filename (default: output.pdf)" },
                    "style": { "type": "string", "enum": ["report", "letter", "invoice", "plain"], "description": "PDF style/template (default: plain)" }
                },
                "required": ["content"]
            }),
            capabilities: vec!["file_write".to_string()],
            sandbox_mode: Some(SandboxMode::Native),
            source: ActionSource::System,
            file_path: None,
        }).await;

        // Expense tracking — add, list, summarize, delete expenses
        self.register_builtin_action(ActionDef {
            name: "expense".to_string(),
            description: "Track expenses and spending. Actions: add (record expense), list (view expenses with optional date/category filter), summary (spending summary by category), delete (remove expense by ID). Use when the user mentions spending, costs, expenses, budget, or purchases.".to_string(),
            version: "1.0.0".to_string(),
            input_schema: serde_json::json!({
                "type": "object",
                "properties": {
                    "action": { "type": "string", "enum": ["add", "list", "summary", "delete"], "description": "Operation to perform" },
                    "amount": { "type": "number", "description": "Amount spent (for add)" },
                    "currency": { "type": "string", "description": "Currency code, e.g. USD, INR (default: USD)" },
                    "category": { "type": "string", "description": "Category: food, transport, shopping, bills, entertainment, health, education, other" },
                    "description": { "type": "string", "description": "What was purchased" },
                    "date": { "type": "string", "description": "Date (YYYY-MM-DD). Default: today" },
                    "vendor": { "type": "string", "description": "Store/vendor name (optional)" },
                    "payment_method": { "type": "string", "description": "cash, card, upi, etc. (optional)" },
                    "tags": { "type": "string", "description": "Comma-separated tags (optional)" },
                    "id": { "type": "string", "description": "Expense ID (for delete)" },
                    "from_date": { "type": "string", "description": "Start date filter (YYYY-MM-DD, for list/summary)" },
                    "to_date": { "type": "string", "description": "End date filter (YYYY-MM-DD, for list/summary)" },
                    "filter_category": { "type": "string", "description": "Category filter (for list)" }
                },
                "required": ["action"]
            }),
            capabilities: vec![],
            sandbox_mode: Some(SandboxMode::Native),
            source: ActionSource::System,
            file_path: None,
        }).await;

        // Security logs — query security events from DB
        self.register_builtin_action(ActionDef {
            name: "security_logs".to_string(),
            description: "View security event logs. Shows recent security events like injection attempts, auth failures, rate limit breaches. Use when the user asks about security events, attack attempts, or system security status.".to_string(),
            version: "1.0.0".to_string(),
            input_schema: serde_json::json!({
                "type": "object",
                "properties": {
                    "limit": { "type": "integer", "description": "Max entries to return (default: 50)" }
                }
            }),
            capabilities: vec![],
            sandbox_mode: Some(SandboxMode::Native),
            source: ActionSource::System,
            file_path: None,
        }).await;

        // Audio transcription
        self.register_builtin_action(ActionDef {
            name: "transcribe_audio".to_string(),
            description: "Transcribe audio/video files to text using Whisper. Use when asked to transcribe, convert speech to text, or extract text from audio/video.".to_string(),
            version: "1.0.0".to_string(),
            input_schema: serde_json::json!({
                "type": "object",
                "properties": {
                    "file_path": { "type": "string", "description": "Path to audio/video file" },
                    "language": { "type": "string", "description": "Language code (e.g. en, hi). Default: auto-detect" },
                    "model": { "type": "string", "enum": ["tiny", "base", "small", "medium", "large"], "description": "Whisper model size (default: base)" }
                },
                "required": ["file_path"]
            }),
            capabilities: vec!["network".to_string()],
            sandbox_mode: Some(SandboxMode::Native),
            source: ActionSource::System,
            file_path: None,
        }).await;

        // Weekly review
        self.register_builtin_action(ActionDef {
            name: "weekly_review".to_string(),
            description: "Generate a weekly review summarizing completed tasks, key conversations, and progress. Use when asked for a weekly review, weekly summary, or progress report.".to_string(),
            version: "1.0.0".to_string(),
            input_schema: serde_json::json!({
                "type": "object",
                "properties": {
                    "period_days": { "type": "integer", "description": "Number of days to review (default: 7)" }
                }
            }),
            capabilities: vec![],
            sandbox_mode: Some(SandboxMode::Native),
            source: ActionSource::System,
            file_path: None,
        }).await;

        // === Integration-backed actions ===

        // GitHub
        self.register_builtin_action(ActionDef {
            name: "github".to_string(),
            description: "Interact with GitHub repositories, issues, and pull requests. Actions: list_repos, create_issue, list_issues, list_prs, create_pr, search. Use when the user mentions GitHub, repos, issues, pull requests, or PRs.".to_string(),
            version: "1.0.0".to_string(),
            input_schema: serde_json::json!({
                "type": "object",
                "properties": {
                    "action": { "type": "string", "enum": ["list_repos", "create_issue", "list_issues", "list_prs", "create_pr", "search"], "description": "GitHub action to perform" },
                    "owner": { "type": "string", "description": "Repository owner (username or org)" },
                    "repo": { "type": "string", "description": "Repository name" },
                    "title": { "type": "string", "description": "Issue/PR title (for create)" },
                    "body": { "type": "string", "description": "Issue/PR body (for create)" },
                    "labels": { "type": "string", "description": "Comma-separated labels (for create_issue)" },
                    "head": { "type": "string", "description": "Head branch (for create_pr)" },
                    "base": { "type": "string", "description": "Base branch (for create_pr, default: main)" },
                    "query": { "type": "string", "description": "Search query (for search)" },
                    "state": { "type": "string", "enum": ["open", "closed", "all"], "description": "Filter by state (for list)" }
                },
                "required": ["action"]
            }),
            capabilities: vec!["network".to_string()],
            sandbox_mode: Some(SandboxMode::Native),
            source: ActionSource::System,
            file_path: None,
        }).await;

        // Notion
        self.register_builtin_action(ActionDef {
            name: "notion".to_string(),
            description: "Interact with Notion pages, databases, and blocks. Actions: search, create_page, update_page, get_page, append_blocks. Use when the user mentions Notion, notes, wiki, or knowledge base.".to_string(),
            version: "1.0.0".to_string(),
            input_schema: serde_json::json!({
                "type": "object",
                "properties": {
                    "action": { "type": "string", "enum": ["search", "create_page", "update_page", "get_page", "append_blocks"], "description": "Notion action to perform" },
                    "query": { "type": "string", "description": "Search query (for search)" },
                    "page_id": { "type": "string", "description": "Page ID (for get/update/append)" },
                    "parent_id": { "type": "string", "description": "Parent page or database ID (for create)" },
                    "title": { "type": "string", "description": "Page title (for create)" },
                    "content": { "type": "string", "description": "Page content as markdown (for create/append)" },
                    "properties": { "type": "object", "description": "Page properties to update (for update)" }
                },
                "required": ["action"]
            }),
            capabilities: vec!["network".to_string()],
            sandbox_mode: Some(SandboxMode::Native),
            source: ActionSource::System,
            file_path: None,
        }).await;

        // Twitter/X
        self.register_builtin_action(ActionDef {
            name: "twitter".to_string(),
            description: "Read tweets, search Twitter/X, view bookmarks, and get user profiles. Actions: bookmarks, list_tweets, search, get_user. Use when the user mentions Twitter, X, tweets, or bookmarks.".to_string(),
            version: "1.0.0".to_string(),
            input_schema: serde_json::json!({
                "type": "object",
                "properties": {
                    "action": { "type": "string", "enum": ["bookmarks", "list_tweets", "search", "get_user"], "description": "Twitter action to perform" },
                    "query": { "type": "string", "description": "Search query (for search)" },
                    "username": { "type": "string", "description": "Twitter username (for get_user, list_tweets)" },
                    "max_results": { "type": "integer", "description": "Maximum results to return (default: 10)" }
                },
                "required": ["action"]
            }),
            capabilities: vec!["network".to_string()],
            sandbox_mode: Some(SandboxMode::Native),
            source: ActionSource::System,
            file_path: None,
        }).await;

        // 1Password
        self.register_builtin_action(ActionDef {
            name: "onepassword".to_string(),
            description: "Access 1Password vault for secure credential management. Actions: list_vaults, get_item (metadata only), search, create_item. Never exposes raw secrets to the LLM.".to_string(),
            version: "1.0.0".to_string(),
            input_schema: serde_json::json!({
                "type": "object",
                "properties": {
                    "action": { "type": "string", "enum": ["list_vaults", "get_item", "search", "create_item"], "description": "1Password action to perform" },
                    "vault_id": { "type": "string", "description": "Vault ID (optional filter)" },
                    "item_id": { "type": "string", "description": "Item ID (for get_item)" },
                    "query": { "type": "string", "description": "Search query (for search)" },
                    "title": { "type": "string", "description": "Item title (for create)" },
                    "category": { "type": "string", "description": "Item category: login, password, note, etc. (for create)" }
                },
                "required": ["action"]
            }),
            capabilities: vec!["network".to_string()],
            sandbox_mode: Some(SandboxMode::Native),
            source: ActionSource::System,
            file_path: None,
        }).await;

        // Google Places
        self.register_builtin_action(ActionDef {
            name: "places".to_string(),
            description: "Search for places, find nearby locations, get place details, and get directions using Google Places/Maps. Actions: search, nearby, details, directions. Use when the user asks about restaurants, shops, locations, directions, or nearby places.".to_string(),
            version: "1.0.0".to_string(),
            input_schema: serde_json::json!({
                "type": "object",
                "properties": {
                    "action": { "type": "string", "enum": ["search", "nearby", "details", "directions"], "description": "Places action to perform" },
                    "query": { "type": "string", "description": "Search query (for search)" },
                    "latitude": { "type": "number", "description": "Latitude (for nearby)" },
                    "longitude": { "type": "number", "description": "Longitude (for nearby)" },
                    "radius": { "type": "integer", "description": "Search radius in meters (for nearby, default: 1000)" },
                    "place_id": { "type": "string", "description": "Place ID (for details)" },
                    "origin": { "type": "string", "description": "Origin address (for directions)" },
                    "destination": { "type": "string", "description": "Destination address (for directions)" },
                    "type": { "type": "string", "description": "Place type filter: restaurant, cafe, hospital, atm, etc." }
                },
                "required": ["action"]
            }),
            capabilities: vec!["network".to_string()],
            sandbox_mode: Some(SandboxMode::Native),
            source: ActionSource::System,
            file_path: None,
        }).await;

        // Twilio (Voice & SMS)
        self.register_builtin_action(ActionDef {
            name: "twilio".to_string(),
            description: "Make phone calls and send SMS messages via Twilio. Actions: call, sms, list_calls, list_messages. Use when the user wants to call someone, send a text message, or check call/message history.".to_string(),
            version: "1.0.0".to_string(),
            input_schema: serde_json::json!({
                "type": "object",
                "properties": {
                    "action": { "type": "string", "enum": ["call", "sms", "list_calls", "list_messages"], "description": "Twilio action to perform" },
                    "to": { "type": "string", "description": "Phone number to call/text (E.164 format: +1234567890)" },
                    "message": { "type": "string", "description": "Message body (for sms)" },
                    "twiml": { "type": "string", "description": "TwiML instructions for the call (for call)" },
                    "limit": { "type": "integer", "description": "Number of records to return (for list, default: 20)" }
                },
                "required": ["action"]
            }),
            capabilities: vec!["network".to_string()],
            sandbox_mode: Some(SandboxMode::Native),
            source: ActionSource::System,
            file_path: None,
        }).await;

        // Ordering & Purchasing
        self.register_builtin_action(ActionDef {
            name: "ordering".to_string(),
            description: "Search products and place orders via Shopify or custom webhook. Actions: search_products, create_order, order_status, list_orders. Use when the user wants to buy, order, or shop for something.".to_string(),
            version: "1.0.0".to_string(),
            input_schema: serde_json::json!({
                "type": "object",
                "properties": {
                    "action": { "type": "string", "enum": ["search_products", "create_order", "order_status", "list_orders"], "description": "Ordering action to perform" },
                    "query": { "type": "string", "description": "Product search query (for search_products)" },
                    "product_id": { "type": "string", "description": "Product ID (for create_order)" },
                    "quantity": { "type": "integer", "description": "Quantity to order (default: 1)" },
                    "order_id": { "type": "string", "description": "Order ID (for order_status)" },
                    "shipping_address": { "type": "object", "description": "Shipping address (for create_order)" }
                },
                "required": ["action"]
            }),
            capabilities: vec!["network".to_string()],
            sandbox_mode: Some(SandboxMode::Native),
            source: ActionSource::System,
            file_path: None,
        }).await;

        // Browser automation — full headless browser control with human-in-the-loop
        // Curated connectors
        // Garmin
        self.register_builtin_action(ActionDef {
            name: "garmin".to_string(),
            description: "Retrieve Garmin fitness data. Actions: daily_summary, activities.".to_string(),
            version: "1.0.0".to_string(),
            input_schema: serde_json::json!({
                "type": "object",
                "properties": {
                    "action": { "type": "string", "enum": ["daily_summary", "activities"], "description": "Garmin action to perform" },
                    "date": { "type": "string", "description": "Date in YYYY-MM-DD (daily_summary)" },
                    "start_date": { "type": "string", "description": "Start date in YYYY-MM-DD (activities)" },
                    "end_date": { "type": "string", "description": "End date in YYYY-MM-DD (activities)" },
                    "limit": { "type": "integer", "description": "Maximum records (default: 50)" }
                },
                "required": ["action"]
            }),
            capabilities: vec!["network".to_string()],
            sandbox_mode: Some(SandboxMode::Native),
            source: ActionSource::System,
            file_path: None,
        }).await;

        // WHOOP
        self.register_builtin_action(ActionDef {
            name: "whoop".to_string(),
            description: "Retrieve WHOOP performance data. Actions: profile, recovery, sleep, workouts.".to_string(),
            version: "1.0.0".to_string(),
            input_schema: serde_json::json!({
                "type": "object",
                "properties": {
                    "action": { "type": "string", "enum": ["profile", "recovery", "sleep", "workouts"], "description": "WHOOP action to perform" },
                    "limit": { "type": "integer", "description": "Maximum records (default: 25)" }
                },
                "required": ["action"]
            }),
            capabilities: vec!["network".to_string()],
            sandbox_mode: Some(SandboxMode::Native),
            source: ActionSource::System,
            file_path: None,
        }).await;

        // GA4
        self.register_builtin_action(ActionDef {
            name: "ga4".to_string(),
            description: "Run GA4 Data API reports. Action: run_report.".to_string(),
            version: "1.0.0".to_string(),
            input_schema: serde_json::json!({
                "type": "object",
                "properties": {
                    "action": { "type": "string", "enum": ["run_report"], "description": "GA4 action to perform" },
                    "property_id": { "type": "string", "description": "GA4 property ID" },
                    "dimensions": { "type": "array", "items": { "type": "string" }, "description": "Dimension names" },
                    "metrics": { "type": "array", "items": { "type": "string" }, "description": "Metric names" },
                    "date_ranges": {
                        "type": "array",
                        "description": "GA4 date ranges payload",
                        "items": {
                            "type": "object",
                            "properties": {
                                "startDate": { "type": "string", "description": "Start date (e.g. 7daysAgo or YYYY-MM-DD)" },
                                "endDate": { "type": "string", "description": "End date (e.g. today or YYYY-MM-DD)" }
                            }
                        }
                    },
                    "limit": { "type": "integer", "description": "Maximum rows (default: 1000)" }
                },
                "required": ["action"]
            }),
            capabilities: vec!["network".to_string()],
            sandbox_mode: Some(SandboxMode::Native),
            source: ActionSource::System,
            file_path: None,
        }).await;

        // GSC
        self.register_builtin_action(ActionDef {
            name: "gsc".to_string(),
            description: "Query Google Search Console analytics. Action: query.".to_string(),
            version: "1.0.0".to_string(),
            input_schema: serde_json::json!({
                "type": "object",
                "properties": {
                    "action": { "type": "string", "enum": ["query"], "description": "GSC action to perform" },
                    "site_url": { "type": "string", "description": "Site URL (or sc-domain value)" },
                    "start_date": { "type": "string", "description": "Start date YYYY-MM-DD" },
                    "end_date": { "type": "string", "description": "End date YYYY-MM-DD" },
                    "dimensions": { "type": "array", "items": { "type": "string" }, "description": "Query dimensions" },
                    "row_limit": { "type": "integer", "description": "Maximum rows (default: 1000)" }
                },
                "required": ["action"]
            }),
            capabilities: vec!["network".to_string()],
            sandbox_mode: Some(SandboxMode::Native),
            source: ActionSource::System,
            file_path: None,
        }).await;

        // Social analytics
        self.register_builtin_action(ActionDef {
            name: "social_analytics".to_string(),
            description: "Cross-source social publishing analytics. Action: summary. Aggregates configured sources such as Twitter and GA4.".to_string(),
            version: "1.0.0".to_string(),
            input_schema: serde_json::json!({
                "type": "object",
                "properties": {
                    "action": { "type": "string", "enum": ["summary"], "description": "Social analytics action to perform" },
                    "days": { "type": "integer", "description": "Lookback window in days (default: 7)" },
                    "post_limit": { "type": "integer", "description": "Max posts to evaluate from Twitter (default: 100)" },
                    "include_twitter": { "type": "boolean", "description": "Include Twitter source (default: true)" },
                    "include_ga4": { "type": "boolean", "description": "Include GA4 source (default: true)" }
                },
                "required": ["action"]
            }),
            capabilities: vec!["network".to_string()],
            sandbox_mode: Some(SandboxMode::Native),
            source: ActionSource::System,
            file_path: None,
        }).await;

        // Moltbook (agent social network)
        self.register_builtin_action(ActionDef {
            name: "moltbook".to_string(),
            description: "Interact with Moltbook (agent social network). Actions: register, status, me, feed, search, create_post, comment, upvote_post. Outbound posting is privacy-guarded (no user/PII/secrets).".to_string(),
            version: "1.0.0".to_string(),
            input_schema: serde_json::json!({
                "type": "object",
                "properties": {
                    "action": { "type": "string", "enum": ["register", "status", "me", "feed", "search", "create_post", "comment", "upvote_post"], "description": "Moltbook action to perform" },
                    "name": { "type": "string", "description": "Agent name (register)" },
                    "description": { "type": "string", "description": "Agent description (register)" },
                    "sort": { "type": "string", "enum": ["hot", "new", "top", "rising"], "description": "Feed sort (feed)" },
                    "limit": { "type": "integer", "description": "Max items to fetch" },
                    "query": { "type": "string", "description": "Semantic search query" },
                    "submolt": { "type": "string", "description": "Community name for post" },
                    "title": { "type": "string", "description": "Post title" },
                    "content": { "type": "string", "description": "Post/comment content" },
                    "post_id": { "type": "string", "description": "Post ID for comment/upvote" },
                    "parent_id": { "type": "string", "description": "Parent comment ID for threaded reply" }
                },
                "required": ["action"]
            }),
            capabilities: vec!["network".to_string()],
            sandbox_mode: Some(SandboxMode::Native),
            source: ActionSource::System,
            file_path: None,
        }).await;

        self.register_builtin_action(ActionDef {
            name: "browser_auto".to_string(),
            description: "Automate browser tasks: navigate websites, fill forms, click buttons, take screenshots, read page content. Use when asked to 'go to a website', 'log into', 'fill out a form', 'book', 'check my account', or any web browsing/automation task. When stuck (CAPTCHA, 2FA, ambiguous UI), takes a screenshot and asks the user for help, then continues. Runs as a background session.".to_string(),
            version: "1.0.0".to_string(),
            input_schema: serde_json::json!({
                "type": "object",
                "properties": {
                    "action": {
                        "type": "string",
                        "enum": ["start_session", "navigate", "click", "type_text", "screenshot", "get_content", "scroll", "press_key", "ask_user", "end_session"],
                        "description": "Browser action to perform. Use start_session first, then chain actions."
                    },
                    "task": { "type": "string", "description": "High-level description of what to accomplish (for start_session)" },
                    "url": { "type": "string", "description": "URL to navigate to" },
                    "session_id": { "type": "string", "description": "Browser session ID (required after start_session)" },
                    "selector": { "type": "string", "description": "CSS selector for click/type target" },
                    "text": { "type": "string", "description": "Text to type or link text to click" },
                    "x": { "type": "integer", "description": "X coordinate for click" },
                    "y": { "type": "integer", "description": "Y coordinate for click" },
                    "clear": { "type": "boolean", "description": "Clear field before typing (default: false)" },
                    "direction": { "type": "string", "enum": ["up", "down"], "description": "Scroll direction" },
                    "key": { "type": "string", "description": "Key to press (Enter, Tab, Escape, etc.)" },
                    "question": { "type": "string", "description": "Question to ask the user (for ask_user)" },
                    "channel": { "type": "string", "description": "Channel to notify on (telegram, whatsapp, web)" }
                },
                "required": ["action"]
            }),
            capabilities: vec!["network".to_string()],
            sandbox_mode: Some(SandboxMode::Native),
            source: ActionSource::System,
            file_path: None,
        }).await;

        // Google Calendar — list, create, find free time
        self.register_builtin_action(ActionDef {
            name: "calendar_today".to_string(),
            description: "List today's calendar events. Use when the user asks 'what's on my calendar today', 'do I have any meetings', etc.".to_string(),
            version: "1.0.0".to_string(),
            input_schema: serde_json::json!({
                "type": "object",
                "properties": {}
            }),
            capabilities: vec!["network".to_string()],
            sandbox_mode: Some(SandboxMode::Native),
            source: ActionSource::System,
            file_path: None,
        }).await;

        self.register_builtin_action(ActionDef {
            name: "calendar_list".to_string(),
            description: "List calendar events in a date range. Use when asked about upcoming events, schedule for a specific date, etc.".to_string(),
            version: "1.0.0".to_string(),
            input_schema: serde_json::json!({
                "type": "object",
                "properties": {
                    "start": { "type": "string", "description": "Start datetime (ISO 8601). Defaults to now." },
                    "end": { "type": "string", "description": "End datetime (ISO 8601). Defaults to 7 days from now." }
                }
            }),
            capabilities: vec!["network".to_string()],
            sandbox_mode: Some(SandboxMode::Native),
            source: ActionSource::System,
            file_path: None,
        }).await;

        self.register_builtin_action(ActionDef {
            name: "calendar_create".to_string(),
            description: "Create a new calendar event. Use when asked to schedule a meeting, add an event, block time, etc.".to_string(),
            version: "1.0.0".to_string(),
            input_schema: serde_json::json!({
                "type": "object",
                "properties": {
                    "summary": { "type": "string", "description": "Event title" },
                    "start": { "type": "string", "description": "Start datetime (ISO 8601)" },
                    "end": { "type": "string", "description": "End datetime (ISO 8601)" },
                    "description": { "type": "string", "description": "Event description/notes" },
                    "location": { "type": "string", "description": "Event location" },
                    "attendees": { "type": "array", "items": { "type": "string" }, "description": "List of attendee email addresses" }
                },
                "required": ["summary", "start", "end"]
            }),
            capabilities: vec!["network".to_string()],
            sandbox_mode: Some(SandboxMode::Native),
            source: ActionSource::System,
            file_path: None,
        }).await;

        self.register_builtin_action(ActionDef {
            name: "calendar_free".to_string(),
            description: "Find free time slots in the calendar. Use when asked 'when am I free', 'find time for a meeting', etc.".to_string(),
            version: "1.0.0".to_string(),
            input_schema: serde_json::json!({
                "type": "object",
                "properties": {
                    "start": { "type": "string", "description": "Start of range (ISO 8601). Defaults to now." },
                    "end": { "type": "string", "description": "End of range (ISO 8601). Defaults to end of today." },
                    "min_duration_minutes": { "type": "integer", "description": "Minimum free slot duration in minutes (default: 30)" }
                }
            }),
            capabilities: vec!["network".to_string()],
            sandbox_mode: Some(SandboxMode::Native),
            source: ActionSource::System,
            file_path: None,
        }).await;

        // SSH — remote server execution (behind feature flag)
        #[cfg(feature = "ssh")]
        {
            self.register_builtin_action(ActionDef {
                name: "ssh".to_string(),
                description: "Execute a command on a configured remote server via SSH. Use when asked to check server status, deploy, manage services, run remote commands, or anything involving a remote server.".to_string(),
                version: "1.0.0".to_string(),
                input_schema: serde_json::json!({
                    "type": "object",
                    "properties": {
                        "connection": { "type": "string", "description": "Name of the SSH connection to use (from configured connections)" },
                        "command": { "type": "string", "description": "Shell command to execute on the remote server" }
                    },
                    "required": ["connection", "command"]
                }),
                capabilities: vec!["network".to_string(), "ssh".to_string()],
                sandbox_mode: Some(SandboxMode::Native),
                source: ActionSource::System,
                file_path: None,
            }).await;

            self.register_builtin_action(ActionDef {
                name: "ssh_connections".to_string(),
                description: "List available SSH connections. Use before ssh to know which servers are configured.".to_string(),
                version: "1.0.0".to_string(),
                input_schema: serde_json::json!({
                    "type": "object",
                    "properties": {}
                }),
                capabilities: vec![],
                sandbox_mode: Some(SandboxMode::Native),
                source: ActionSource::System,
                file_path: None,
            }).await;
        }

        self.register_builtin_action(ActionDef {
            name: "app_inspect".to_string(),
            description: "Inspect existing deployed apps. Use when the user asks which apps are deployed, refers to a deployed app by name/ID, or wants to debug, diagnose, fix, or update an existing app. Returns matched app metadata, its /app/data/apps/<id> directory, key files, runtime state, and recent logs so you can use file_read/file_write/app_restart/http_get on the live app.".to_string(),
            version: "1.0.0".to_string(),
            input_schema: serde_json::json!({
                "type": "object",
                "properties": {
                    "query": {
                        "type": "string",
                        "description": "Optional app title or app ID to match. Leave empty to list deployed apps."
                    },
                    "include_files": {
                        "type": "boolean",
                        "description": "Include a file inventory for the matched app. Default: true."
                    },
                    "include_logs": {
                        "type": "boolean",
                        "description": "Include recent runtime log tail for the matched app when available. Default: true."
                    },
                    "limit": {
                        "type": "integer",
                        "description": "Maximum number of apps to list in the summary. Default: 10."
                    }
                }
            }),
            capabilities: vec!["app_hosting".to_string(), "file_read".to_string()],
            sandbox_mode: Some(SandboxMode::Native),
            source: ActionSource::System,
            file_path: None,
        }).await;

        // App deployment — write files, start servers, return live URL
        self.register_builtin_action(ActionDef {
            name: "app_deploy".to_string(),
            description: "Deploy a web app or server and return a live URL. Supports ANY type of app: static HTML/JS/CSS, Python (FastAPI, Flask, Streamlit), Node.js (Express), or any language. Use when asked to build a dashboard, create a tool, make a website, build an app, or anything that should be accessible via a browser. For static apps, provide HTML/JS/CSS files. For dynamic apps (Python/Node servers), also provide entry_command to start the server. Dynamic apps default to local runtime with container fallback (runtime_preference=local). Public exposure defaults to enabled (expose_public=true) so the agent can return a tunnel-ready link. Declare required inputs via required_inputs and mark each item sensitive=true/false. Access guard is optional and defaults to false.".to_string(),
            version: "1.0.0".to_string(),
            input_schema: serde_json::json!({
                "type": "object",
                "properties": {
                    "files": {
                        "type": "object",
                        "description": "Object mapping filename to file content. e.g. {\"index.html\": \"<html>...\", \"style.css\": \"body{...}\", \"app.py\": \"from fastapi import...\"}"
                    },
                    "title": { "type": "string", "description": "App name/title (default: App)" },
                    "entry_command": {
                        "type": "string",
                        "description": "Command to start the server process (omit for static HTML apps). Use {PORT} placeholder or PORT env var for the port. Examples: 'python3 app.py', 'node server.js', 'uvicorn app:app --host 0.0.0.0 --port {PORT}'"
                    },
                    "install_command": {
                        "type": "string",
                        "description": "Command to install dependencies before starting (optional). Examples: 'pip install -r requirements.txt', 'npm install'"
                    },
                    "required_inputs": {
                        "type": "array",
                        "items": {
                            "oneOf": [
                                { "type": "string" },
                                {
                                    "type": "object",
                                    "properties": {
                                        "key": { "type": "string" },
                                        "sensitive": { "type": "boolean" }
                                    },
                                    "required": ["key"]
                                }
                            ]
                        },
                        "description": "Required runtime inputs. String entries default to sensitive=true. Use object entries for per-key sensitivity, e.g. [{\"key\":\"API_TOKEN\",\"sensitive\":true},{\"key\":\"BASE_URL\",\"sensitive\":false}]"
                    },
                    "required_secrets": {
                        "type": "array",
                        "items": { "type": "string" },
                        "description": "Compatibility alias for sensitive required inputs."
                    },
                    "required_config": {
                        "type": "array",
                        "items": { "type": "string" },
                        "description": "Compatibility alias for non-sensitive required inputs."
                    },
                    "required_env": {
                        "type": "array",
                        "items": { "type": "string" },
                        "description": "Legacy alias for required_secrets."
                    },
                    "config": {
                        "type": "object",
                        "additionalProperties": { "type": ["string", "number", "boolean"] },
                        "description": "Optional non-sensitive runtime config values (e.g. BASE_URL). Values are stored in app metadata for restart/restore."
                    },
                    "runtime_image": {
                        "type": "string",
                        "description": "Optional container image used to run the app (default: agentark-sandbox:latest)"
                    },
                    "runtime_preference": {
                        "type": "string",
                        "enum": ["local", "container"],
                        "description": "Preferred runtime for dynamic apps. Default: local (local process first, container fallback unless docker is required)."
                    },
                    "expose_public": {
                        "type": "boolean",
                        "description": "Whether to expose the app on the Cloudflare tunnel when available. Default: true."
                    },
                    "access_guard": {
                        "type": "boolean",
                        "description": "Enable access-key guard for the app URL. Default: false (public app URL)."
                    },
                    "replace_existing": {
                        "type": "boolean",
                        "description": "Force recreation even if a matching deployed app already exists. Default: false."
                    }
                },
                "required": ["files"]
            }),
            capabilities: vec!["app_hosting".to_string()],
            sandbox_mode: Some(SandboxMode::Native),
            source: ActionSource::System,
            file_path: None,
        }).await;

        // Video generation — Remotion-powered programmatic video rendering
        self.register_builtin_action(ActionDef {
            name: "video_generate".to_string(),
            description: "Generate a product showcase or scripted explainer video using Remotion (React-based). Use this when the user wants custom branded/product promo animation where the agent writes TSX animation code. If the user requests normal AI text-to-video and does not ask for showcase/Remotion style, prefer generate_video instead. If unclear, ask which mode they want.".to_string(),
            version: "1.0.0".to_string(),
            input_schema: serde_json::json!({
                "type": "object",
                "properties": {
                    "component": {
                        "type": "string",
                        "description": "React TSX component code for the video. Must export 'Main' as a named export. Use Remotion's useCurrentFrame(), useVideoConfig(), interpolate(), spring(), Sequence, AbsoluteFill for animations."
                    },
                    "duration_seconds": { "type": "integer", "minimum": 1, "maximum": 120, "description": "Video duration in seconds (default: 10, allowed: 1-120)" },
                    "width": { "type": "integer", "minimum": 256, "maximum": 3840, "description": "Video width in pixels (default: 1920, allowed: 256-3840)" },
                    "height": { "type": "integer", "minimum": 256, "maximum": 2160, "description": "Video height in pixels (default: 1080, allowed: 256-2160)" },
                    "fps": { "type": "integer", "minimum": 12, "maximum": 60, "description": "Frames per second (default: 30, allowed: 12-60)" },
                    "filename": { "type": "string", "description": "Output filename (default: video.mp4)" }
                },
                "required": ["component"]
            }),
            capabilities: vec!["video_generation".to_string()],
            sandbox_mode: Some(SandboxMode::Native),
            source: ActionSource::System,
            file_path: None,
        }).await;

        // Provider-based text/image-to-video generation (Runway/Luma/Fal/Veo/etc.)
        self.register_builtin_action(ActionDef {
            name: "generate_video".to_string(),
            description: "Generate a normal AI video via configured video providers (Runway, Luma, Fal, Sora, Veo, etc.). Use for general text-to-video or image-to-video requests when no custom Remotion coding is needed. If the user specifically asks for product showcase/scripted animation with custom scenes, use video_generate instead. If unclear, ask which mode they want.".to_string(),
            version: "1.0.0".to_string(),
            input_schema: serde_json::json!({
                "type": "object",
                "properties": {
                    "prompt": { "type": "string", "description": "Video prompt/description" },
                    "image_url": { "type": "string", "description": "Optional source image URL for image-to-video models" },
                    "duration_seconds": { "type": "integer", "minimum": 1, "maximum": 12, "description": "Desired duration in seconds (model-dependent; default 4)" },
                    "aspect_ratio": { "type": "string", "description": "Optional aspect ratio (e.g. 16:9, 9:16, 1:1)" },
                    "model": { "type": "string", "description": "Optional provider model override" },
                    "provider": { "type": "string", "description": "Optional provider override (replicate, runway, luma, fal, openai_sora, google_veo, etc.)" }
                },
                "required": ["prompt"]
            }),
            capabilities: vec!["video_generation".to_string()],
            sandbox_mode: Some(SandboxMode::Native),
            source: ActionSource::System,
            file_path: None,
        }).await;

        // Self-evolve - policy-first self-improvement
        self.register_builtin_action(ActionDef {
            name: "self_evolve".to_string(),
            description: "Evolve AgentArk behavior with an auditable promotion loop. Default mode is policy/strategy evolution (benchmark, lineage archive, statistical gating, canary rollout with replay gate, optional promotion). Code evolution is disabled by default and requires explicit allow_code_writes=true.".to_string(),
            version: "1.0.0".to_string(),
            input_schema: serde_json::json!({
                "type": "object",
                "properties": {
                    "request": {
                        "type": "string",
                        "description": "Natural language description of what should evolve"
                    },
                    "mode": {
                        "type": "string",
                        "enum": ["policy", "code"],
                        "description": "Evolution mode. policy (default) evolves runtime strategy; code enables source mutation mode."
                    },
                    "allow_code_writes": {
                        "type": "boolean",
                        "description": "Required true to run code mode. Ignored for policy mode."
                    },
                    "apply_promotion": {
                        "type": "boolean",
                        "description": "For policy mode: apply promoted policy by activating canary rollout and replay gate. Default true."
                    },
                    "canary_rollout_percent": {
                        "type": "integer",
                        "minimum": 1,
                        "maximum": 100,
                        "description": "Traffic percentage for candidate policy during canary rollout. Default 20."
                    },
                    "canary_min_samples_per_version": {
                        "type": "integer",
                        "minimum": 5,
                        "description": "Minimum baseline/candidate samples required for replay promotion. Default 25."
                    },
                    "canary_min_success_gain": {
                        "type": "number",
                        "minimum": 0.0,
                        "description": "Minimum success-rate improvement required for promotion. Default 0.03."
                    },
                    "canary_max_sign_test_p_value": {
                        "type": "number",
                        "minimum": 0.0001,
                        "maximum": 1.0,
                        "description": "Maximum one-sided sign-test p-value for promotion. Default 0.10."
                    },
                    "replay_log_limit": {
                        "type": "integer",
                        "minimum": 100,
                        "description": "Operational log window size used for replay evaluation. Default 4000."
                    }
                },
                "required": ["request"]
            }),
            capabilities: vec!["self_evolve".to_string()],
            sandbox_mode: Some(SandboxMode::Native),
            source: ActionSource::System,
            file_path: None,
        }).await;

        Ok(())
    }

    async fn register_builtin_action(&self, info: ActionDef) {
        self.actions.write().await.insert(
            info.name.clone(),
            LoadedAction {
                info,
                wasm_module: None,
                workflow_content: None,
                mcp_binding: None,
            },
        );
    }

    /// Register an action with workflow content (from ACTION.md)
    async fn register_workflow_action(&self, info: ActionDef, workflow: String) {
        self.actions.write().await.insert(
            info.name.clone(),
            LoadedAction {
                info,
                wasm_module: None,
                workflow_content: Some(workflow),
                mcp_binding: None,
            },
        );
    }

    /// Register an MCP-backed action (external tool/resource)
    pub async fn register_mcp_action(&self, info: ActionDef, binding: McpBinding) {
        self.actions.write().await.insert(
            info.name.clone(),
            LoadedAction {
                info,
                wasm_module: None,
                workflow_content: None,
                mcp_binding: Some(binding),
            },
        );
    }

    /// Remove all MCP-backed actions
    pub async fn unregister_mcp_actions(&self) -> usize {
        let mut actions = self.actions.write().await;
        let before = actions.len();
        actions.retain(|_, a| a.mcp_binding.is_none());
        before.saturating_sub(actions.len())
    }

    /// Remove MCP-backed actions for a specific server
    pub async fn unregister_mcp_actions_for_server(&self, server_id: &str) -> usize {
        let mut actions = self.actions.write().await;
        let before = actions.len();
        actions.retain(|_, a| {
            if let Some(binding) = &a.mcp_binding {
                binding.server_id != server_id
            } else {
                true
            }
        });
        before.saturating_sub(actions.len())
    }

    /// Execute an action with given arguments
    pub async fn execute_action(
        &self,
        action_name: &str,
        arguments: &serde_json::Value,
    ) -> Result<String> {
        let (sandbox_mode, mcp_binding, source) = {
            let actions = self.actions.read().await;
            let action = actions
                .get(action_name)
                .ok_or_else(|| anyhow::anyhow!("Unknown action: {}", action_name))?;
            (
                action
                    .info
                    .sandbox_mode
                    .clone()
                    .unwrap_or(self.config.default_sandbox.clone()),
                action.mcp_binding.clone(),
                action.info.source.clone(),
            )
        };

        if source != ActionSource::System {
            let disabled = self.disabled_actions.read().await;
            if disabled.contains(action_name) {
                return Err(anyhow::anyhow!(
                    "Action '{}' is disabled. Re-enable it in the UI before running.",
                    action_name
                ));
            }
        }

        // Resolve secrets at execution time so they never appear in LLM-visible
        // tool-call arguments or execution traces.
        let resolved_args = self.resolve_secret_placeholders(action_name, arguments)?;

        if let Some(binding) = mcp_binding {
            return self.execute_mcp_action(binding, &resolved_args).await;
        }

        // Start transaction if rollback is enabled
        let transaction = if self.config.enable_rollback {
            let mut tx_guard = self.transactions.lock().await;
            Some(tx_guard.begin().await?)
        } else {
            None
        };

        // Execute based on sandbox mode
        let result = match sandbox_mode {
            SandboxMode::Native => self.execute_native(action_name, &resolved_args).await,
            SandboxMode::Wasm => self.execute_wasm(action_name, &resolved_args).await,
            SandboxMode::Docker => self.execute_docker(action_name, &resolved_args).await,
        };

        // Handle transaction
        match (&result, transaction) {
            (Ok(_), Some(tx)) => {
                let mut tx_guard = self.transactions.lock().await;
                tx_guard.commit(tx).await?;
            }
            (Err(_), Some(tx)) => {
                tracing::warn!("Rolling back transaction due to error");
                let mut tx_guard = self.transactions.lock().await;
                tx_guard.rollback(tx).await?;
            }
            _ => {}
        }

        result
    }

    /// Resolve secret placeholders inside action arguments.
    ///
    /// Supported syntax:
    /// - `{{secret:KEY}}` looks up an encrypted custom secret:
    ///   - `secret:KEY` (preferred)
    ///   - `env:KEY` (compat)
    /// - `{{env:ENV_NAME}}` resolves ENV_NAME using an optional per-action binding:
    ///   - binding key: `action_envmap:{action}:{ENV_NAME}` -> {target}
    ///   - if target == "builtin", uses the agent's configured provider key(s) where applicable
    ///   - else looks up `env:{target}` in encrypted custom secrets
    ///
    /// NOTE: Returns the resolved arguments, but does not mutate the original `arguments`,
    /// so traces / tool calls remain safe.
    pub fn resolve_secret_placeholders(
        &self,
        action_name: &str,
        arguments: &serde_json::Value,
    ) -> Result<serde_json::Value> {
        let mgr = SecureConfigManager::new_with_data_dir(&self.config_dir, Some(self.data_dir()))?;
        let secrets = mgr.load_secrets().unwrap_or_default();
        let config = mgr.load().ok();

        fn builtin_env_from_config(cfg: &AgentConfig, env: &str) -> Option<String> {
            let mut providers: Vec<&crate::core::LlmProvider> = vec![&cfg.llm];
            if let Some(fb) = cfg.llm_fallback.as_ref() {
                providers.push(fb);
            }
            for slot in &cfg.model_pool.slots {
                if slot.enabled {
                    providers.push(&slot.provider);
                }
            }

            match env {
                "OPENAI_API_KEY" => providers.into_iter().find_map(|p| match p {
                    crate::core::LlmProvider::OpenAI { api_key, .. } if !api_key.is_empty() => {
                        Some(api_key.clone())
                    }
                    _ => None,
                }),
                "OPENROUTER_API_KEY" => providers
                    .into_iter()
                    .find_map(|p| match p {
                        crate::core::LlmProvider::OpenAI {
                            api_key, base_url, ..
                        } => {
                            if !api_key.is_empty()
                                && base_url.as_deref().unwrap_or("").contains("openrouter")
                            {
                                Some(api_key.clone())
                            } else {
                                None
                            }
                        }
                        _ => None,
                    })
                    .or_else(|| builtin_env_from_config(cfg, "OPENAI_API_KEY")),
                "ANTHROPIC_API_KEY" => providers.into_iter().find_map(|p| match p {
                    crate::core::LlmProvider::Anthropic { api_key, .. } if !api_key.is_empty() => {
                        Some(api_key.clone())
                    }
                    _ => None,
                }),
                _ => None,
            }
        }

        fn legacy_env_alias_lookup(
            custom: &std::collections::HashMap<String, String>,
            env: &str,
        ) -> Option<String> {
            // Compatibility: existing integrations store provider tokens under non-env keys.
            let legacy_key = match env {
                "GITHUB_TOKEN" => Some("github_token"),
                "NOTION_TOKEN" => Some("notion_token"),
                "TWITTER_BEARER_TOKEN" => Some("twitter_bearer_token"),
                "ONEPASSWORD_TOKEN" => Some("onepassword_token"),
                "GOOGLE_PLACES_API_KEY" => Some("google_places_api_key"),
                "TWILIO_AUTH_TOKEN" => Some("twilio_auth_token"),
                "TWILIO_ACCOUNT_SID" => Some("twilio_account_sid"),
                "GARMIN_TOKEN" => Some("garmin_token"),
                "GARMIN_API_BASE" => Some("garmin_api_base"),
                "WHOOP_TOKEN" => Some("whoop_token"),
                "GA4_ACCESS_TOKEN" => Some("ga4_access_token"),
                "GA4_PROPERTY_ID" => Some("ga4_property_id"),
                "GSC_ACCESS_TOKEN" => Some("gsc_access_token"),
                "GSC_SITE_URL" => Some("gsc_site_url"),
                "SOCIAL_TWITTER_BEARER_TOKEN" => Some("social_twitter_bearer_token"),
                "SOCIAL_GA4_ACCESS_TOKEN" => Some("social_ga4_access_token"),
                "SOCIAL_GA4_PROPERTY_ID" => Some("social_ga4_property_id"),
                _ => None,
            }?;
            custom.get(legacy_key).cloned()
        }

        let re = regex::Regex::new(r"\{\{\s*(secret|env)\s*:\s*([A-Za-z0-9_\-:.]+)\s*\}\}")
            .expect("valid placeholder regex");
        let custom = &secrets.custom;

        let resolve_secret = |key: &str| -> Option<String> {
            custom
                .get(&format!("secret:{}", key))
                .cloned()
                .or_else(|| custom.get(&format!("env:{}", key)).cloned())
                .or_else(|| {
                    config
                        .as_ref()
                        .and_then(|cfg| builtin_env_from_config(cfg, key))
                })
        };

        let resolve_env = |env: &str| -> Option<String> {
            let binding_key = format!("action_envmap:{}:{}", action_name, env);
            let target = custom
                .get(&binding_key)
                .cloned()
                .unwrap_or_else(|| env.to_string());

            if target == "builtin" {
                return config
                    .as_ref()
                    .and_then(|cfg| builtin_env_from_config(cfg, env));
            }

            custom
                .get(&format!("env:{}", target))
                .cloned()
                .or_else(|| custom.get(&format!("secret:{}", target)).cloned())
                .or_else(|| legacy_env_alias_lookup(custom, env))
                .or_else(|| {
                    config
                        .as_ref()
                        .and_then(|cfg| builtin_env_from_config(cfg, env))
                })
        };

        fn substitute_in_str(
            s: &str,
            re: &regex::Regex,
            action_name: &str,
            resolve_secret: &impl Fn(&str) -> Option<String>,
            resolve_env: &impl Fn(&str) -> Option<String>,
        ) -> Result<String> {
            let mut out = String::with_capacity(s.len());
            let mut last = 0usize;
            for caps in re.captures_iter(s) {
                let m = caps.get(0).unwrap();
                out.push_str(&s[last..m.start()]);
                let kind = caps.get(1).map(|m| m.as_str()).unwrap_or("");
                let key = caps.get(2).map(|m| m.as_str()).unwrap_or("");
                let val = match kind {
                    "secret" => resolve_secret(key),
                    "env" => resolve_env(key),
                    _ => None,
                }
                .ok_or_else(|| {
                    anyhow::anyhow!(
                        "Missing secret/env '{}:{}' for action '{}'",
                        kind,
                        key,
                        action_name
                    )
                })?;
                out.push_str(&val);
                last = m.end();
            }
            out.push_str(&s[last..]);
            Ok(out)
        }

        fn walk(
            v: &serde_json::Value,
            re: &regex::Regex,
            action_name: &str,
            resolve_secret: &impl Fn(&str) -> Option<String>,
            resolve_env: &impl Fn(&str) -> Option<String>,
        ) -> Result<serde_json::Value> {
            Ok(match v {
                serde_json::Value::String(s) => serde_json::Value::String(substitute_in_str(
                    s,
                    re,
                    action_name,
                    resolve_secret,
                    resolve_env,
                )?),
                serde_json::Value::Array(arr) => {
                    let mut out = Vec::with_capacity(arr.len());
                    for item in arr {
                        out.push(walk(item, re, action_name, resolve_secret, resolve_env)?);
                    }
                    serde_json::Value::Array(out)
                }
                serde_json::Value::Object(map) => {
                    let mut out = serde_json::Map::with_capacity(map.len());
                    for (k, val) in map {
                        out.insert(
                            k.clone(),
                            walk(val, re, action_name, resolve_secret, resolve_env)?,
                        );
                    }
                    serde_json::Value::Object(out)
                }
                other => other.clone(),
            })
        }

        walk(arguments, &re, action_name, &resolve_secret, &resolve_env)
    }

    async fn execute_mcp_action(
        &self,
        binding: McpBinding,
        arguments: &serde_json::Value,
    ) -> Result<String> {
        let registry = self
            .mcp_registry
            .as_ref()
            .ok_or_else(|| anyhow::anyhow!("MCP registry not initialized"))?;
        let mut registry = registry.write().await;
        match binding.kind {
            McpBindingKind::Tool { name } => {
                registry
                    .call_tool(&binding.server_id, &name, arguments)
                    .await
            }
            McpBindingKind::Resource { uri } => {
                registry.read_resource(&binding.server_id, &uri).await
            }
        }
    }

    /// Execute an action natively (no sandbox)
    async fn execute_native(
        &self,
        action_name: &str,
        arguments: &serde_json::Value,
    ) -> Result<String> {
        match action_name {
            "file_read" => {
                let path = arguments["path"]
                    .as_str()
                    .ok_or_else(|| anyhow::anyhow!("Missing path"))?;
                let content = tokio::fs::read_to_string(path).await?;
                Ok(content)
            }
            "file_write" => {
                let path = arguments["path"]
                    .as_str()
                    .ok_or_else(|| anyhow::anyhow!("Missing path"))?;
                let content = arguments["content"]
                    .as_str()
                    .ok_or_else(|| anyhow::anyhow!("Missing content"))?;
                tokio::fs::write(path, content).await?;
                Ok(format!("Written to {}", path))
            }
            "clipboard_read" => {
                let mut clipboard = arboard::Clipboard::new()
                    .map_err(|e| anyhow::anyhow!("Failed to access clipboard: {}", e))?;
                let content = clipboard
                    .get_text()
                    .map_err(|e| anyhow::anyhow!("Failed to read clipboard: {}", e))?;
                Ok(content)
            }
            "clipboard_write" => {
                let content = arguments["content"]
                    .as_str()
                    .ok_or_else(|| anyhow::anyhow!("Missing content"))?;
                let mut clipboard = arboard::Clipboard::new()
                    .map_err(|e| anyhow::anyhow!("Failed to access clipboard: {}", e))?;
                clipboard
                    .set_text(content)
                    .map_err(|e| anyhow::anyhow!("Failed to write clipboard: {}", e))?;
                Ok("Content copied to clipboard".to_string())
            }
            "list_tasks" => {
                let queue = self
                    .task_queue
                    .as_ref()
                    .ok_or_else(|| anyhow::anyhow!("Task queue not available"))?;
                let tasks = queue.read().await;
                let filter = arguments
                    .get("filter")
                    .and_then(|v| v.as_str())
                    .unwrap_or("pending");

                let filtered: Vec<_> = tasks
                    .all()
                    .iter()
                    .filter(|t| match filter {
                        "pending" => matches!(
                            t.status,
                            crate::core::TaskStatus::Pending
                                | crate::core::TaskStatus::AwaitingApproval
                        ),
                        "goals" => t.action == "goal",
                        "routines" => t.cron.is_some(),
                        "completed" => matches!(t.status, crate::core::TaskStatus::Completed),
                        "failed" => matches!(t.status, crate::core::TaskStatus::Failed { .. }),
                        _ => true, // "all"
                    })
                    .collect();

                if filtered.is_empty() {
                    return Ok(format!("No {} items found.", filter));
                }

                let mut output = format!("Found {} {} item(s):\n\n", filtered.len(), filter);
                for t in &filtered {
                    let status_str = match &t.status {
                        crate::core::TaskStatus::Pending => "Pending",
                        crate::core::TaskStatus::AwaitingApproval => "Awaiting Approval",
                        crate::core::TaskStatus::InProgress => "In Progress",
                        crate::core::TaskStatus::Completed => "Completed",
                        crate::core::TaskStatus::Failed { .. } => "Failed",
                        crate::core::TaskStatus::Cancelled => "Cancelled",
                    };
                    output.push_str(&format!("- {} (status: {})\n", t.description, status_str));
                    if let Some(ref cron) = t.cron {
                        output.push_str(&format!("  Schedule: {}\n", cron));
                    }
                }
                Ok(output)
            }
            "tunnel_control" => self.execute_tunnel_control(arguments).await,
            "schedule_task" => {
                let task_desc = arguments["task"]
                    .as_str()
                    .ok_or_else(|| anyhow::anyhow!("Missing task description"))?;

                let schedule_info =
                    if let Some(cron_expr) = arguments.get("cron").and_then(|v| v.as_str()) {
                        // Auto-convert standard 5-field cron to 6-field (with seconds)
                        // Standard: "minute hour day month weekday" -> "0 9 * * *"
                        // Rust cron: "second minute hour day month weekday" -> "0 0 9 * * *"
                        let cron_6field = if cron_expr.split_whitespace().count() == 5 {
                            format!("0 {}", cron_expr) // Prepend "0 " for seconds
                        } else {
                            cron_expr.to_string()
                        };

                        // Validate cron expression
                        cron_6field.parse::<cron::Schedule>().map_err(|e| {
                            anyhow::anyhow!("Invalid cron expression '{}': {}", cron_6field, e)
                        })?;
                        format!("cron:{}", cron_6field)
                    } else if let Some(at_time) = arguments.get("at").and_then(|v| v.as_str()) {
                        // Validate ISO timestamp
                        chrono::DateTime::parse_from_rfc3339(at_time)
                            .map_err(|e| anyhow::anyhow!("Invalid timestamp: {}", e))?;
                        format!("at:{}", at_time)
                    } else {
                        return Err(anyhow::anyhow!(
                            "Must specify either 'cron' or 'at' for scheduling"
                        ));
                    };

                // Return scheduling info - actual scheduling is handled by the agent's task queue
                Ok(format!(
                    "Task scheduled: {} | Schedule: {}",
                    task_desc, schedule_info
                ))
            }
            "watch" => {
                // Return a marker — actual watcher creation is handled by Agent::handle_watch
                let desc = arguments
                    .get("description")
                    .and_then(|v| v.as_str())
                    .unwrap_or("watcher");
                Ok(format!("Watch created: {}", desc))
            }
            "connector_request" => self.execute_connector_request(arguments).await,
            "pipeline_compile" => self.execute_pipeline_compile(arguments).await,
            "pipeline_run" => self.execute_pipeline_run(arguments).await,
            "signal_consensus" => self.execute_signal_consensus(arguments).await,
            "gmail_scan" => crate::actions::gmail::gmail_scan(&self.config_dir, arguments).await,
            "gmail_reply" => crate::actions::gmail::gmail_reply(&self.config_dir, arguments).await,
            "web_search" => {
                let args: crate::actions::search::SearchArgs =
                    serde_json::from_value(arguments.clone())
                        .map_err(|e| anyhow::anyhow!("Invalid search arguments: {}", e))?;

                let config = build_search_config(&self.config_dir).await;
                crate::actions::search::execute_search(&args, &config).await
            }
            "research" => {
                let args: crate::actions::research::ResearchArgs =
                    serde_json::from_value(arguments.clone())
                        .map_err(|e| anyhow::anyhow!("Invalid research arguments: {}", e))?;

                let config = build_search_config(&self.config_dir).await;
                crate::actions::research::execute_research(&args, &config).await
            }
            "video-frames" => {
                // This action requires ffmpeg - check arguments
                let video = arguments
                    .get("video")
                    .and_then(|v| v.as_str())
                    .ok_or_else(|| anyhow::anyhow!("Missing 'video' path argument"))?;
                let time = arguments
                    .get("time")
                    .and_then(|v| v.as_str())
                    .unwrap_or("00:00:00");
                let output = arguments
                    .get("out")
                    .and_then(|v| v.as_str())
                    .map(|s| s.to_string())
                    .unwrap_or_else(|| format!("{}_frame.jpg", video.trim_end_matches(".mp4")));

                // Execute ffmpeg
                let output_result = tokio::process::Command::new("ffmpeg")
                    .args([
                        "-ss",
                        time,
                        "-i",
                        video,
                        "-frames:v",
                        "1",
                        "-q:v",
                        "2",
                        &output,
                        "-y",
                    ])
                    .output()
                    .await
                    .map_err(|e| anyhow::anyhow!("Failed to run ffmpeg: {}", e))?;

                if output_result.status.success() {
                    Ok(format!("Frame extracted to: {}", output))
                } else {
                    let stderr = String::from_utf8_lossy(&output_result.stderr);
                    Err(anyhow::anyhow!("ffmpeg failed: {}", stderr))
                }
            }
            "code_execute" => {
                // Native fallback for code execution (when Docker mode falls through)
                self.execute_code_native(arguments).await
            }
            "browse" => {
                let url = arguments["url"]
                    .as_str()
                    .ok_or_else(|| anyhow::anyhow!("Missing url"))?;
                let extract = arguments
                    .get("extract")
                    .and_then(|v| v.as_str())
                    .unwrap_or("text");

                // Fetch the page
                let client = reqwest::Client::builder()
                    .user_agent("AgentArk/0.1 (AI Agent Browser)")
                    .timeout(std::time::Duration::from_secs(30))
                    .redirect(reqwest::redirect::Policy::limited(5))
                    .build()
                    .map_err(|e| anyhow::anyhow!("Failed to build HTTP client: {}", e))?;

                let response = client
                    .get(url)
                    .send()
                    .await
                    .map_err(|e| anyhow::anyhow!("Failed to fetch URL: {}", e))?;

                let status = response.status();
                if !status.is_success() {
                    return Err(anyhow::anyhow!(
                        "HTTP error {}: {}",
                        status.as_u16(),
                        status.canonical_reason().unwrap_or("Unknown")
                    ));
                }

                let html = response
                    .text()
                    .await
                    .map_err(|e| anyhow::anyhow!("Failed to read response body: {}", e))?;

                // Extract content based on the extract parameter
                let title_re = regex::Regex::new(r"(?is)<title[^>]*>(.*?)</title>").unwrap();
                let title = title_re
                    .captures(&html)
                    .and_then(|c| c.get(1))
                    .map(|m| m.as_str().trim().to_string())
                    .unwrap_or_default();
                let tag_re = regex::Regex::new(r"<[^>]+>").unwrap();

                match extract {
                    "title" => Ok(if title.is_empty() {
                        "(no title found)".to_string()
                    } else {
                        title
                    }),
                    "links" => {
                        let link_re = regex::Regex::new(
                            r#"(?is)<a[^>]+href\s*=\s*["']([^"']+)["'][^>]*>(.*?)</a>"#,
                        )
                        .unwrap();
                        let mut links = Vec::new();
                        for cap in link_re.captures_iter(&html) {
                            let href = cap.get(1).map(|m| m.as_str()).unwrap_or("");
                            let text = cap.get(2).map(|m| m.as_str()).unwrap_or("");
                            // Strip HTML tags from link text
                            let clean_text = tag_re.replace_all(text, "").trim().to_string();
                            if !href.is_empty()
                                && !href.starts_with('#')
                                && !href.starts_with("javascript:")
                            {
                                links.push(format!(
                                    "[{}]({})",
                                    if clean_text.is_empty() {
                                        href
                                    } else {
                                        &clean_text
                                    },
                                    href
                                ));
                            }
                        }
                        if links.is_empty() {
                            Ok("(no links found)".to_string())
                        } else {
                            // Limit to 50 links to avoid overwhelming output
                            let display_links: Vec<&str> =
                                links.iter().take(50).map(|s| s.as_str()).collect();
                            Ok(format!(
                                "Found {} links (showing up to 50):\n{}",
                                links.len(),
                                display_links.join("\n")
                            ))
                        }
                    }
                    "all" => {
                        // Extract text
                        let text = Self::html_to_text(&html);
                        // Extract links
                        let link_re = regex::Regex::new(
                            r#"(?is)<a[^>]+href\s*=\s*["']([^"']+)["'][^>]*>(.*?)</a>"#,
                        )
                        .unwrap();
                        let mut links = Vec::new();
                        for cap in link_re.captures_iter(&html) {
                            let href = cap.get(1).map(|m| m.as_str()).unwrap_or("");
                            let link_text = cap.get(2).map(|m| m.as_str()).unwrap_or("");
                            let clean_text = tag_re.replace_all(link_text, "").trim().to_string();
                            if !href.is_empty()
                                && !href.starts_with('#')
                                && !href.starts_with("javascript:")
                            {
                                links.push(format!(
                                    "[{}]({})",
                                    if clean_text.is_empty() {
                                        href
                                    } else {
                                        &clean_text
                                    },
                                    href
                                ));
                            }
                        }
                        let links_section = if links.is_empty() {
                            "(no links found)".to_string()
                        } else {
                            let display_links: Vec<&str> =
                                links.iter().take(30).map(|s| s.as_str()).collect();
                            format!(
                                "{} links (showing up to 30):\n{}",
                                links.len(),
                                display_links.join("\n")
                            )
                        };
                        Ok(format!(
                            "## Title\n{}\n\n## Content\n{}\n\n## Links\n{}",
                            if title.is_empty() {
                                "(no title)"
                            } else {
                                &title
                            },
                            text,
                            links_section
                        ))
                    }
                    _ => {
                        // Default: extract text content
                        let text = Self::html_to_text(&html);
                        Ok(if text.is_empty() {
                            "(no text content extracted)".to_string()
                        } else {
                            text
                        })
                    }
                }
            }
            "pdf_generate" => {
                let content = arguments["content"]
                    .as_str()
                    .ok_or_else(|| anyhow::anyhow!("Missing content"))?;
                let title = arguments
                    .get("title")
                    .and_then(|v| v.as_str())
                    .unwrap_or("Document");
                let filename = arguments
                    .get("filename")
                    .and_then(|v| v.as_str())
                    .unwrap_or("output.pdf");
                let style = arguments
                    .get("style")
                    .and_then(|v| v.as_str())
                    .unwrap_or("plain");

                let output_path = self.data_dir().join("outputs").join(filename);
                if let Some(parent) = output_path.parent() {
                    tokio::fs::create_dir_all(parent).await?;
                }

                // Generate Python script that creates the PDF using fpdf2
                let escaped_content = content
                    .replace('\\', "\\\\")
                    .replace('"', "\\\"")
                    .replace('\n', "\\n");
                let escaped_title = title.replace('\\', "\\\\").replace('"', "\\\"");
                let out_str = output_path.to_string_lossy().replace('\\', "/");

                let python_code = format!(
                    r#"
import subprocess, sys
subprocess.check_call([sys.executable, "-m", "pip", "install", "-q", "fpdf2"], stdout=subprocess.DEVNULL, stderr=subprocess.DEVNULL)
from fpdf import FPDF

pdf = FPDF()
pdf.set_auto_page_break(auto=True, margin=15)
pdf.add_page()
style = "{style}"

if style == "invoice":
    pdf.set_font("Helvetica", "B", 20)
    pdf.cell(0, 15, "{escaped_title}", new_x="LMARGIN", new_y="NEXT")
    pdf.set_font("Helvetica", "", 10)
elif style == "report":
    pdf.set_font("Helvetica", "B", 16)
    pdf.cell(0, 12, "{escaped_title}", new_x="LMARGIN", new_y="NEXT")
    pdf.line(10, pdf.get_y(), 200, pdf.get_y())
    pdf.ln(5)
    pdf.set_font("Helvetica", "", 11)
elif style == "letter":
    pdf.set_font("Helvetica", "", 11)
    pdf.ln(20)
else:
    pdf.set_font("Helvetica", "", 11)

content = "{escaped_content}"
for line in content.split("\\n"):
    pdf.multi_cell(0, 6, line)

pdf.output("{out_str}")
print("PDF generated: {out_str}")
"#
                );
                let code_args = serde_json::json!({
                    "language": "python",
                    "code": python_code.trim()
                });
                self.execute_code_native(&code_args).await
            }
            "expense" => {
                let action = arguments["action"]
                    .as_str()
                    .ok_or_else(|| anyhow::anyhow!("Missing action parameter"))?;
                let storage = self
                    .storage
                    .as_ref()
                    .ok_or_else(|| anyhow::anyhow!("Storage not available"))?;

                match action {
                    "add" => {
                        let amount = arguments
                            .get("amount")
                            .and_then(|v| v.as_f64())
                            .ok_or_else(|| anyhow::anyhow!("Missing amount"))?;
                        let description = arguments
                            .get("description")
                            .and_then(|v| v.as_str())
                            .ok_or_else(|| anyhow::anyhow!("Missing description"))?;
                        let currency = arguments
                            .get("currency")
                            .and_then(|v| v.as_str())
                            .unwrap_or("USD");
                        let category = arguments
                            .get("category")
                            .and_then(|v| v.as_str())
                            .unwrap_or("other");
                        let date = arguments
                            .get("date")
                            .and_then(|v| v.as_str())
                            .map(|s| s.to_string())
                            .unwrap_or_else(|| chrono::Utc::now().format("%Y-%m-%d").to_string());

                        let id = format!(
                            "exp-{}",
                            uuid::Uuid::new_v4()
                                .to_string()
                                .split('-')
                                .next()
                                .unwrap_or("0")
                        );
                        let model = crate::storage::entities::expense::Model {
                            id: id.clone(),
                            amount,
                            currency: currency.to_string(),
                            category: category.to_string(),
                            description: description.to_string(),
                            date,
                            payment_method: arguments
                                .get("payment_method")
                                .and_then(|v| v.as_str())
                                .map(|s| s.to_string()),
                            vendor: arguments
                                .get("vendor")
                                .and_then(|v| v.as_str())
                                .map(|s| s.to_string()),
                            tags: arguments
                                .get("tags")
                                .and_then(|v| v.as_str())
                                .map(|s| s.to_string()),
                            split_with: None,
                            receipt_path: None,
                            created_at: chrono::Utc::now().to_rfc3339(),
                        };
                        storage.insert_expense(model).await?;
                        Ok(format!(
                            "Expense recorded: {} {} for '{}' (category: {}, id: {})",
                            currency, amount, description, category, id
                        ))
                    }
                    "list" => {
                        let from = arguments.get("from_date").and_then(|v| v.as_str());
                        let to = arguments.get("to_date").and_then(|v| v.as_str());
                        let cat = arguments.get("filter_category").and_then(|v| v.as_str());
                        let expenses = storage.get_expenses(from, to, cat).await?;
                        if expenses.is_empty() {
                            return Ok("No expenses found.".to_string());
                        }
                        let mut output = format!("Found {} expense(s):\n\n", expenses.len());
                        let mut total = 0.0f64;
                        for e in &expenses {
                            output.push_str(&format!(
                                "- [{}] {} {} — {} ({}){}\n",
                                e.id,
                                e.currency,
                                e.amount,
                                e.description,
                                e.category,
                                e.vendor
                                    .as_ref()
                                    .map(|v| format!(" @ {}", v))
                                    .unwrap_or_default()
                            ));
                            total += e.amount;
                        }
                        output.push_str(&format!("\nTotal: {:.2}", total));
                        Ok(output)
                    }
                    "summary" => {
                        let from = arguments.get("from_date").and_then(|v| v.as_str());
                        let to = arguments.get("to_date").and_then(|v| v.as_str());
                        let expenses = storage.get_expense_summary(from, to).await?;
                        if expenses.is_empty() {
                            return Ok("No expenses found for the period.".to_string());
                        }
                        // Aggregate by category
                        let mut by_category: std::collections::HashMap<String, f64> =
                            std::collections::HashMap::new();
                        for e in &expenses {
                            *by_category.entry(e.category.clone()).or_insert(0.0) += e.amount;
                        }
                        let mut output = "Expense Summary by Category:\n\n".to_string();
                        let mut grand_total = 0.0f64;
                        let mut cats: Vec<_> = by_category.into_iter().collect();
                        cats.sort_by(|a, b| {
                            b.1.partial_cmp(&a.1).unwrap_or(std::cmp::Ordering::Equal)
                        });
                        for (category, total) in &cats {
                            output.push_str(&format!("  {}: {:.2}\n", category, total));
                            grand_total += total;
                        }
                        output.push_str(&format!("\nGrand Total: {:.2}", grand_total));
                        Ok(output)
                    }
                    "delete" => {
                        let id = arguments
                            .get("id")
                            .and_then(|v| v.as_str())
                            .ok_or_else(|| anyhow::anyhow!("Missing expense ID"))?;
                        let deleted = storage.delete_expense(id).await?;
                        if deleted {
                            Ok(format!("Expense {} deleted.", id))
                        } else {
                            Ok(format!("Expense {} not found.", id))
                        }
                    }
                    _ => Err(anyhow::anyhow!("Unknown expense action: {}", action)),
                }
            }
            "security_logs" => {
                let storage = self
                    .storage
                    .as_ref()
                    .ok_or_else(|| anyhow::anyhow!("Storage not available"))?;
                let limit = arguments
                    .get("limit")
                    .and_then(|v| v.as_u64())
                    .unwrap_or(50);
                let logs = storage.list_security_logs(limit).await?;
                if logs.is_empty() {
                    return Ok("No security events recorded. All clear.".to_string());
                }
                let mut output = format!("Security Log ({} entries):\n\n", logs.len());
                for log in &logs {
                    output.push_str(&format!(
                        "- [{}] {} ({}): {} (count: {})\n",
                        log.created_at.split('T').next().unwrap_or(&log.created_at),
                        log.event_type,
                        log.severity,
                        log.message,
                        log.count,
                    ));
                }
                Ok(output)
            }
            "transcribe_audio" => {
                let file_path = arguments["file_path"]
                    .as_str()
                    .ok_or_else(|| anyhow::anyhow!("Missing file_path"))?;
                let language = arguments
                    .get("language")
                    .and_then(|v| v.as_str())
                    .unwrap_or("auto");
                let model = arguments
                    .get("model")
                    .and_then(|v| v.as_str())
                    .unwrap_or("base");

                let escaped_path = file_path.replace('\\', "/");
                let lang_arg = if language == "auto" {
                    "None".to_string()
                } else {
                    format!("\"{}\"", language)
                };

                let python_code = format!(
                    r#"
import subprocess, sys
subprocess.check_call([sys.executable, "-m", "pip", "install", "-q", "openai-whisper"], stdout=subprocess.DEVNULL, stderr=subprocess.DEVNULL)
import whisper

model = whisper.load_model("{model}")
result = model.transcribe("{escaped_path}", language={lang_arg})
print(result["text"])
"#
                );
                let code_args = serde_json::json!({
                    "language": "python",
                    "code": python_code.trim()
                });
                self.execute_code_native(&code_args).await
            }
            "weekly_review" => {
                let period = arguments
                    .get("period_days")
                    .and_then(|v| v.as_i64())
                    .unwrap_or(7);
                let queue = self
                    .task_queue
                    .as_ref()
                    .ok_or_else(|| anyhow::anyhow!("Task queue not available"))?;
                let tasks = queue.read().await;

                let mut output = format!("Weekly Review (last {} days):\n\n", period);

                // Completed tasks
                let completed: Vec<_> = tasks
                    .all()
                    .iter()
                    .filter(|t| matches!(t.status, crate::core::TaskStatus::Completed))
                    .collect();
                output.push_str(&format!("**Completed Tasks** ({})\n", completed.len()));
                for t in &completed {
                    output.push_str(&format!("  - {}\n", t.description));
                }

                // Pending tasks
                let pending: Vec<_> = tasks
                    .all()
                    .iter()
                    .filter(|t| {
                        matches!(
                            t.status,
                            crate::core::TaskStatus::Pending
                                | crate::core::TaskStatus::AwaitingApproval
                        )
                    })
                    .collect();
                output.push_str(&format!("\n**Pending Tasks** ({})\n", pending.len()));
                for t in &pending {
                    output.push_str(&format!("  - {}\n", t.description));
                }

                // Failed tasks
                let failed: Vec<_> = tasks
                    .all()
                    .iter()
                    .filter(|t| matches!(t.status, crate::core::TaskStatus::Failed { .. }))
                    .collect();
                if !failed.is_empty() {
                    output.push_str(&format!("\n**Failed Tasks** ({})\n", failed.len()));
                    for t in &failed {
                        output.push_str(&format!("  - {}\n", t.description));
                    }
                }

                // Expense summary if storage available
                if let Some(ref storage) = self.storage {
                    let from_date = (chrono::Utc::now() - chrono::Duration::days(period))
                        .format("%Y-%m-%d")
                        .to_string();
                    if let Ok(expenses) = storage.get_expense_summary(Some(&from_date), None).await
                    {
                        if !expenses.is_empty() {
                            let mut by_cat: std::collections::HashMap<String, f64> =
                                std::collections::HashMap::new();
                            for e in &expenses {
                                *by_cat.entry(e.category.clone()).or_insert(0.0) += e.amount;
                            }
                            output.push_str("\n**Spending Summary**\n");
                            let mut total = 0.0;
                            for (cat, amt) in &by_cat {
                                output.push_str(&format!("  {}: {:.2}\n", cat, amt));
                                total += amt;
                            }
                            output.push_str(&format!("  Total: {:.2}\n", total));
                        }
                    }
                }

                Ok(output)
            }
            // Google Calendar actions
            "calendar_today" => {
                crate::actions::calendar::calendar_today(&self.config_dir, arguments).await
            }
            "calendar_list" => {
                crate::actions::calendar::calendar_list(&self.config_dir, arguments).await
            }
            "calendar_create" => {
                crate::actions::calendar::calendar_create(&self.config_dir, arguments).await
            }
            "calendar_free" => {
                crate::actions::calendar::calendar_free(&self.config_dir, arguments).await
            }
            // SSH remote execution
            #[cfg(feature = "ssh")]
            "ssh" => crate::actions::ssh::ssh_execute(&self.config_dir, arguments).await,
            #[cfg(feature = "ssh")]
            "ssh_connections" => crate::actions::ssh::ssh_list_connections(&self.config_dir).await,
            // Video generation via Remotion
            "video_generate" => {
                crate::actions::video::video_generate(&self.config_dir, self.data_dir(), arguments)
                    .await
            }
            // Handle workflow actions - return marker for agent to process with LLM
            other => {
                let actions = self.actions.read().await;
                if let Some(action) = actions.get(other) {
                    if action.workflow_content.is_some() {
                        // Return a special marker that tells the agent to use LLM-driven execution.
                        let user_query = Self::build_workflow_user_query(arguments);
                        let has_freeform_query = arguments
                            .get("query")
                            .and_then(|v| v.as_str())
                            .is_some_and(|s| !s.trim().is_empty());
                        if !has_freeform_query {
                            let required = Self::collect_required_fields_from_schema(
                                &action.info.input_schema,
                            );
                            let missing: Vec<String> = required
                                .iter()
                                .filter(|k| !Self::has_non_empty_argument(arguments, k))
                                .cloned()
                                .collect();
                            if !missing.is_empty() {
                                let payload = WorkflowMissingInputsPayload {
                                    action: other.to_string(),
                                    missing,
                                    required,
                                    provided: Self::collect_provided_argument_keys(arguments),
                                    query: user_query,
                                };
                                return Ok(Self::build_workflow_missing_inputs_marker(&payload));
                            }
                        }
                        return Ok(format!(
                            "{}{}:{}",
                            WORKFLOW_ACTION_MARKER, other, user_query
                        ));
                    }
                }
                Err(anyhow::anyhow!("Unknown native action: {}", action_name))
            }
        }
    }

    async fn execute_pipeline_compile(&self, arguments: &serde_json::Value) -> Result<String> {
        let spec_value = arguments
            .get("spec")
            .ok_or_else(|| anyhow::anyhow!("Missing spec"))?;
        let spec: crate::core::pipeline::PipelineSpec = serde_json::from_value(spec_value.clone())
            .map_err(|e| anyhow::anyhow!("Invalid pipeline spec: {}", e))?;
        let compiled = crate::core::pipeline::compile_pipeline(&spec)?;

        let save = arguments
            .get("save")
            .and_then(|v| v.as_bool())
            .unwrap_or(true);
        let mut persisted = false;
        let mut warnings = compiled.warnings.clone();
        if save {
            if let Some(storage) = self.storage.as_ref() {
                let key = format!("pipeline:spec:{}", Self::pipeline_key_slug(&spec.name));
                storage.set(&key, &serde_json::to_vec(&spec)?).await?;
                persisted = true;
            } else {
                warnings.push("storage unavailable; pipeline spec not persisted".to_string());
            }
        }

        Ok(serde_json::to_string_pretty(&serde_json::json!({
            "status": "compiled",
            "pipeline": compiled.name,
            "node_count": compiled.node_count,
            "ordered_nodes": compiled.ordered_nodes,
            "warnings": warnings,
            "persisted": persisted,
        }))?)
    }

    async fn execute_signal_consensus(&self, arguments: &serde_json::Value) -> Result<String> {
        let request: crate::core::pipeline::SignalConsensusRequest =
            serde_json::from_value(arguments.clone())
                .map_err(|e| anyhow::anyhow!("Invalid signal_consensus arguments: {}", e))?;
        let result = crate::core::pipeline::run_signal_consensus(&request)?;
        Ok(serde_json::to_string_pretty(&result)?)
    }

    async fn execute_connector_request(&self, arguments: &serde_json::Value) -> Result<String> {
        let spec: crate::core::connector::ConnectorRequestSpec =
            serde_json::from_value(arguments.clone())
                .map_err(|e| anyhow::anyhow!("Invalid connector_request arguments: {}", e))?;

        if spec.url.trim().is_empty() {
            return Err(anyhow::anyhow!("connector_request requires non-empty url"));
        }

        let retry = spec.retry.normalized();
        let timeout_secs = spec.timeout_secs.clamp(1, 300);
        let client = reqwest::Client::builder()
            .timeout(std::time::Duration::from_secs(timeout_secs))
            .build()
            .map_err(|e| anyhow::anyhow!("Failed to build HTTP client: {}", e))?;

        let mut pages = Vec::new();
        let mut items = Vec::new();
        let mut total_requests = 0usize;

        let pagination = &spec.pagination;
        let max_pages = pagination.max_pages.clamp(1, 1_000);
        let mut page_no = pagination.start_page.max(1);
        let mut cursor: Option<String> = None;

        for _ in 0..max_pages {
            let mut query = spec.query.clone();
            if let Some(page_size) = pagination.page_size {
                if page_size > 0 {
                    query.insert(pagination.page_size_param.clone(), page_size.to_string());
                }
            }
            match pagination.mode {
                crate::core::connector::PaginationMode::Page => {
                    query.insert(pagination.page_param.clone(), page_no.to_string());
                }
                crate::core::connector::PaginationMode::Cursor => {
                    if let Some(ref c) = cursor {
                        query.insert(pagination.cursor_param.clone(), c.clone());
                    }
                }
                crate::core::connector::PaginationMode::None => {}
            }

            let mut attempt = 1u32;
            let mut backoff_ms = retry.initial_backoff_ms;
            let mut refreshed = false;
            let (status, body_text, request_url) = loop {
                total_requests += 1;
                match self.connector_send_once(&client, &spec, &query).await {
                    Ok((status, body_text, request_url)) => {
                        if !(200..300).contains(&status) {
                            if let Some(refresh) = spec.auth_refresh.as_ref() {
                                if refresh.retry_statuses.contains(&status) && !refreshed {
                                    if refresh.action.eq_ignore_ascii_case("connector_request") {
                                        return Err(anyhow::anyhow!(
                                            "auth_refresh.action cannot be connector_request"
                                        ));
                                    }
                                    // Break async recursion cycle:
                                    // execute_action -> execute_native -> execute_connector_request -> execute_action
                                    std::pin::Pin::from(Box::new(
                                        self.execute_action(&refresh.action, &refresh.arguments),
                                    ))
                                    .await?;
                                    refreshed = true;
                                    continue;
                                }
                            }
                            if attempt < retry.max_attempts
                                && retry.retry_on_status.contains(&status)
                            {
                                Self::sleep_with_backoff(backoff_ms, retry.jitter_ratio).await;
                                backoff_ms =
                                    (backoff_ms.saturating_mul(2)).min(retry.max_backoff_ms);
                                attempt += 1;
                                continue;
                            }
                            let snippet = if body_text.len() > 500 {
                                format!("{}...", &body_text[..500])
                            } else {
                                body_text.clone()
                            };
                            return Err(anyhow::anyhow!(
                                "Connector request failed (status {}): {}",
                                status,
                                snippet
                            ));
                        }
                        break (status, body_text, request_url);
                    }
                    Err(e) => {
                        if attempt < retry.max_attempts {
                            Self::sleep_with_backoff(backoff_ms, retry.jitter_ratio).await;
                            backoff_ms = (backoff_ms.saturating_mul(2)).min(retry.max_backoff_ms);
                            attempt += 1;
                            continue;
                        }
                        return Err(e);
                    }
                }
            };

            let payload: serde_json::Value = serde_json::from_str(&body_text)
                .unwrap_or_else(|_| serde_json::json!({ "raw_body": body_text }));

            let mut page_items =
                crate::core::connector::extract_items(&payload, &pagination.items_path);
            if page_items.is_empty()
                && pagination.mode == crate::core::connector::PaginationMode::None
            {
                page_items = match &payload {
                    serde_json::Value::Array(arr) => arr.clone(),
                    other => vec![other.clone()],
                };
            }

            let next_cursor =
                crate::core::connector::extract_next_cursor(&payload, &pagination.next_cursor_path);

            pages.push(crate::core::connector::ConnectorPageResult {
                request_url,
                status,
                item_count: page_items.len(),
                next_cursor: next_cursor.clone(),
            });
            items.extend(page_items);

            let done = match pagination.mode {
                crate::core::connector::PaginationMode::None => true,
                crate::core::connector::PaginationMode::Page => {
                    if pages.last().map(|p| p.item_count == 0).unwrap_or(true) {
                        true
                    } else {
                        page_no = page_no.saturating_add(1);
                        false
                    }
                }
                crate::core::connector::PaginationMode::Cursor => {
                    if next_cursor.as_ref().is_none() || next_cursor.as_ref() == cursor.as_ref() {
                        true
                    } else {
                        cursor = next_cursor;
                        false
                    }
                }
            };
            if done {
                break;
            }

            if spec.rate_limit_ms > 0 {
                tokio::time::sleep(std::time::Duration::from_millis(spec.rate_limit_ms)).await;
            }
        }

        let out = crate::core::connector::ConnectorRunResult {
            method: spec.method.as_str().to_string(),
            total_requests,
            total_items: items.len(),
            pages,
            items,
        };
        Ok(serde_json::to_string_pretty(&out)?)
    }

    async fn connector_send_once(
        &self,
        client: &reqwest::Client,
        spec: &crate::core::connector::ConnectorRequestSpec,
        query: &BTreeMap<String, String>,
    ) -> Result<(u16, String, String)> {
        let method = reqwest::Method::from_bytes(spec.method.as_str().as_bytes())
            .map_err(|e| anyhow::anyhow!("Invalid HTTP method: {}", e))?;
        let mut req = client.request(method.clone(), &spec.url);
        for (k, v) in &spec.headers {
            req = req.header(k, v);
        }
        if !query.is_empty() {
            req = req.query(query);
        }
        if method != reqwest::Method::GET && method != reqwest::Method::DELETE {
            if let Some(body) = spec.body.as_ref() {
                req = req.json(body);
            }
        }

        let resp = req
            .send()
            .await
            .map_err(|e| anyhow::anyhow!("Connector request network error: {}", e))?;
        let status = resp.status().as_u16();
        let request_url = resp.url().to_string();
        let body = resp.text().await.unwrap_or_default();
        Ok((status, body, request_url))
    }

    async fn execute_pipeline_run(&self, arguments: &serde_json::Value) -> Result<String> {
        #[derive(Debug, Clone, Serialize, Deserialize)]
        struct IdempotencyRecord {
            pipeline: String,
            node: String,
            status: String,
            stored_at: String,
            expires_at: String,
            output: serde_json::Value,
        }

        let allow_privileged = arguments
            .get("allow_privileged")
            .and_then(|v| v.as_bool())
            .unwrap_or(false);
        let dry_run = arguments
            .get("dry_run")
            .and_then(|v| v.as_bool())
            .unwrap_or(false);

        let spec = if let Some(spec_value) = arguments.get("spec") {
            serde_json::from_value::<crate::core::pipeline::PipelineSpec>(spec_value.clone())
                .map_err(|e| anyhow::anyhow!("Invalid pipeline spec: {}", e))?
        } else {
            let pipeline_name = arguments
                .get("pipeline_name")
                .and_then(|v| v.as_str())
                .ok_or_else(|| anyhow::anyhow!("Missing spec or pipeline_name"))?;
            self.load_saved_pipeline_spec(pipeline_name).await?
        };

        let compiled = crate::core::pipeline::compile_pipeline(&spec)?;

        if dry_run {
            return Ok(serde_json::to_string_pretty(&serde_json::json!({
                "status": "dry_run",
                "pipeline": spec.name,
                "node_count": compiled.node_count,
                "ordered_nodes": compiled.ordered_nodes,
                "warnings": compiled.warnings,
            }))?);
        }

        let mut context = Self::context_map_from_json(arguments.get("context"));
        let now = chrono::Utc::now();
        context.insert("pipeline".to_string(), spec.name.clone());
        context.insert("date".to_string(), now.format("%Y-%m-%d").to_string());
        context.insert("timestamp".to_string(), now.to_rfc3339());

        let nodes_by_id: HashMap<String, crate::core::pipeline::PipelineNode> = spec
            .nodes
            .iter()
            .cloned()
            .map(|n| (n.id.clone(), n))
            .collect();

        let run_id = uuid::Uuid::new_v4().to_string();
        let started_at = chrono::Utc::now();

        let mut outputs: HashMap<String, serde_json::Value> = HashMap::new();
        let mut failed_nodes: HashSet<String> = HashSet::new();
        let mut node_reports: Vec<serde_json::Value> = Vec::new();

        for node_id in &compiled.ordered_nodes {
            let node = nodes_by_id
                .get(node_id)
                .ok_or_else(|| anyhow::anyhow!("Missing compiled node '{}'", node_id))?;

            if let Some(dep) = node
                .depends_on
                .iter()
                .find(|dep| failed_nodes.contains(*dep))
                .cloned()
            {
                let msg = format!("Skipped: dependency '{}' failed", dep);
                node_reports.push(serde_json::json!({
                    "node_id": node.id,
                    "status": "skipped_dependency_failed",
                    "reason": msg,
                }));
                failed_nodes.insert(node.id.clone());
                if node.on_error == crate::core::pipeline::NodeErrorMode::Fail {
                    let failed = serde_json::json!({
                        "status": "failed",
                        "pipeline": spec.name.clone(),
                        "run_id": run_id.clone(),
                        "started_at": started_at.to_rfc3339(),
                        "finished_at": chrono::Utc::now().to_rfc3339(),
                        "node_reports": node_reports,
                    });
                    self.persist_pipeline_run(&spec.name, &run_id, &failed)
                        .await?;
                    return Err(anyhow::anyhow!(
                        "Pipeline '{}' failed: node '{}' blocked by failed dependency '{}'",
                        spec.name,
                        node.id,
                        dep
                    ));
                }
                continue;
            }

            let mut node_ctx = context.clone();
            node_ctx.insert("node".to_string(), node.id.clone());
            for dep in &node.depends_on {
                if let Some(dep_out) = outputs.get(dep) {
                    node_ctx.insert(format!("output_{}", dep), dep_out.to_string());
                }
            }

            let rendered_args = Self::render_json_templates(&node.arguments, &node_ctx);
            let action_name = match node.kind {
                crate::core::pipeline::NodeKind::Action => node.action.clone(),
                crate::core::pipeline::NodeKind::ConnectorRequest => {
                    "connector_request".to_string()
                }
                crate::core::pipeline::NodeKind::SignalConsensus => "signal_consensus".to_string(),
            };
            if action_name.trim().is_empty() {
                return Err(anyhow::anyhow!(
                    "Node '{}' resolved to empty action",
                    node.id
                ));
            }
            if action_name == "pipeline_run" || action_name == "pipeline_compile" {
                return Err(anyhow::anyhow!(
                    "Node '{}' uses forbidden nested orchestration action '{}'",
                    node.id,
                    action_name
                ));
            }
            if !allow_privileged && self.action_requires_privileged_allow(&action_name).await {
                let msg = format!(
                    "Node '{}' requires privileged action '{}'; set allow_privileged=true to run",
                    node.id, action_name
                );
                node_reports.push(serde_json::json!({
                    "node_id": node.id,
                    "action": action_name,
                    "status": "blocked_privileged",
                    "error": msg,
                }));
                failed_nodes.insert(node.id.clone());
                if node.on_error == crate::core::pipeline::NodeErrorMode::Fail {
                    let failed = serde_json::json!({
                        "status": "failed",
                        "pipeline": spec.name.clone(),
                        "run_id": run_id.clone(),
                        "started_at": started_at.to_rfc3339(),
                        "finished_at": chrono::Utc::now().to_rfc3339(),
                        "node_reports": node_reports,
                    });
                    self.persist_pipeline_run(&spec.name, &run_id, &failed)
                        .await?;
                    return Err(anyhow::anyhow!("{}", msg));
                }
                continue;
            }

            let mut idempotent_hit = false;
            if let (Some(storage), Some(idem)) = (self.storage.as_ref(), node.idempotency.as_ref())
            {
                let mut idem_ctx = node_ctx.clone();
                idem_ctx.insert("pipeline".to_string(), spec.name.clone());
                idem_ctx.insert("node".to_string(), node.id.clone());
                let idem_key =
                    crate::core::pipeline::render_template(&idem.key_template, &idem_ctx);
                let storage_key = format!("pipeline:idem:{}", idem_key);
                if let Some(raw) = storage.get(&storage_key).await? {
                    if let Ok(record) = serde_json::from_slice::<IdempotencyRecord>(&raw) {
                        if let Ok(expires) =
                            chrono::DateTime::parse_from_rfc3339(&record.expires_at)
                        {
                            if expires.with_timezone(&chrono::Utc) > chrono::Utc::now()
                                && record.status == "completed"
                            {
                                idempotent_hit = true;
                                outputs.insert(node.id.clone(), record.output.clone());
                                node_reports.push(serde_json::json!({
                                    "node_id": node.id,
                                    "action": action_name,
                                    "status": "idempotent_hit",
                                    "attempts": 0,
                                }));
                            } else if expires.with_timezone(&chrono::Utc) <= chrono::Utc::now() {
                                let _ = storage.delete(&storage_key).await;
                            }
                        }
                    }
                }
            }
            if idempotent_hit {
                continue;
            }

            let started = std::time::Instant::now();
            let retry = node.retry.normalized();
            match self
                .execute_action_with_retry(&action_name, &rendered_args, &retry)
                .await
            {
                Ok((output, attempts)) => {
                    let output_json = Self::coerce_to_json(&output);
                    outputs.insert(node.id.clone(), output_json.clone());

                    if let (Some(storage), Some(idem)) =
                        (self.storage.as_ref(), node.idempotency.as_ref())
                    {
                        let mut idem_ctx = node_ctx.clone();
                        idem_ctx.insert("pipeline".to_string(), spec.name.clone());
                        idem_ctx.insert("node".to_string(), node.id.clone());
                        let idem_key =
                            crate::core::pipeline::render_template(&idem.key_template, &idem_ctx);
                        let storage_key = format!("pipeline:idem:{}", idem_key);
                        let ttl_secs = idem.ttl_secs.clamp(60, 30 * 24 * 60 * 60);
                        let now_utc = chrono::Utc::now();
                        let expires_at = now_utc + chrono::Duration::seconds(ttl_secs as i64);
                        let record = IdempotencyRecord {
                            pipeline: spec.name.clone(),
                            node: node.id.clone(),
                            status: "completed".to_string(),
                            stored_at: now_utc.to_rfc3339(),
                            expires_at: expires_at.to_rfc3339(),
                            output: output_json,
                        };
                        storage
                            .set(&storage_key, &serde_json::to_vec(&record)?)
                            .await?;
                    }

                    node_reports.push(serde_json::json!({
                        "node_id": node.id,
                        "action": action_name,
                        "status": "completed",
                        "attempts": attempts,
                        "duration_ms": started.elapsed().as_millis(),
                    }));
                }
                Err(e) => {
                    failed_nodes.insert(node.id.clone());
                    let msg = e.to_string();
                    node_reports.push(serde_json::json!({
                        "node_id": node.id,
                        "action": action_name,
                        "status": if node.on_error == crate::core::pipeline::NodeErrorMode::Continue { "failed_continue" } else { "failed" },
                        "error": msg,
                        "duration_ms": started.elapsed().as_millis(),
                    }));
                    if node.on_error == crate::core::pipeline::NodeErrorMode::Fail {
                        let failed = serde_json::json!({
                            "status": "failed",
                            "pipeline": spec.name.clone(),
                            "run_id": run_id.clone(),
                            "started_at": started_at.to_rfc3339(),
                            "finished_at": chrono::Utc::now().to_rfc3339(),
                            "node_reports": node_reports,
                            "outputs": outputs,
                        });
                        self.persist_pipeline_run(&spec.name, &run_id, &failed)
                            .await?;
                        return Err(anyhow::anyhow!(
                            "Pipeline '{}' failed at node '{}': {}",
                            spec.name,
                            node.id,
                            e
                        ));
                    }
                }
            }
        }

        let mut selected_outputs = serde_json::Map::new();
        if spec.outputs.is_empty() {
            for (k, v) in &outputs {
                selected_outputs.insert(k.clone(), v.clone());
            }
        } else {
            for key in &spec.outputs {
                if let Some(v) = outputs.get(key) {
                    selected_outputs.insert(key.clone(), v.clone());
                }
            }
        }

        let status = if failed_nodes.is_empty() {
            "completed"
        } else {
            "completed_with_errors"
        };
        let result = serde_json::json!({
            "status": status,
            "pipeline": spec.name.clone(),
            "run_id": run_id.clone(),
            "started_at": started_at.to_rfc3339(),
            "finished_at": chrono::Utc::now().to_rfc3339(),
            "node_reports": node_reports,
            "outputs": selected_outputs,
        });
        self.persist_pipeline_run(&spec.name, &run_id, &result)
            .await?;
        Ok(serde_json::to_string_pretty(&result)?)
    }

    async fn load_saved_pipeline_spec(
        &self,
        pipeline_name: &str,
    ) -> Result<crate::core::pipeline::PipelineSpec> {
        let storage = self
            .storage
            .as_ref()
            .ok_or_else(|| anyhow::anyhow!("Storage is not available for saved pipelines"))?;
        let key = format!("pipeline:spec:{}", Self::pipeline_key_slug(pipeline_name));
        let raw = storage
            .get(&key)
            .await?
            .ok_or_else(|| anyhow::anyhow!("Saved pipeline '{}' not found", pipeline_name))?;
        let spec = serde_json::from_slice::<crate::core::pipeline::PipelineSpec>(&raw)
            .map_err(|e| anyhow::anyhow!("Saved pipeline '{}' is invalid: {}", pipeline_name, e))?;
        Ok(spec)
    }

    async fn persist_pipeline_run(
        &self,
        pipeline_name: &str,
        run_id: &str,
        payload: &serde_json::Value,
    ) -> Result<()> {
        if let Some(storage) = self.storage.as_ref() {
            let key = format!(
                "pipeline:run:{}:{}",
                Self::pipeline_key_slug(pipeline_name),
                run_id
            );
            storage.set(&key, &serde_json::to_vec(payload)?).await?;
        }
        Ok(())
    }

    async fn execute_action_with_retry(
        &self,
        action_name: &str,
        arguments: &serde_json::Value,
        retry: &crate::core::pipeline::RetryPolicy,
    ) -> Result<(String, u32)> {
        let policy = retry.normalized();
        let mut attempt = 1u32;
        let mut backoff_ms = policy.initial_backoff_ms;

        loop {
            match std::pin::Pin::from(Box::new(self.execute_action(action_name, arguments))).await {
                Ok(output) => return Ok((output, attempt)),
                Err(err) => {
                    let message = err.to_string();
                    if attempt >= policy.max_attempts
                        || !Self::is_retryable_error(&message, &policy)
                    {
                        return Err(anyhow::anyhow!("{}", message));
                    }
                    Self::sleep_with_backoff(backoff_ms, policy.jitter_ratio).await;
                    backoff_ms = (backoff_ms.saturating_mul(2)).min(policy.max_backoff_ms);
                    attempt += 1;
                }
            }
        }
    }

    async fn action_requires_privileged_allow(&self, action_name: &str) -> bool {
        if matches!(
            action_name,
            "pipeline_run" | "pipeline_compile" | "manage_actions"
        ) {
            return true;
        }
        let (source, capabilities) = {
            let actions = self.actions.read().await;
            let Some(action) = actions.get(action_name) else {
                return true;
            };
            (action.info.source.clone(), action.info.capabilities.clone())
        };

        let has_dangerous_cap = capabilities.iter().any(|cap| {
            matches!(
                cap.to_ascii_lowercase().as_str(),
                "shell"
                    | "file_write"
                    | "clipboard_write"
                    | "gmail"
                    | "code_execute"
                    | "app_hosting"
                    | "orchestration"
                    | "ssh"
            )
        });

        has_dangerous_cap || (source != ActionSource::System && capabilities.is_empty())
    }

    fn pipeline_key_slug(input: &str) -> String {
        let mut out = String::with_capacity(input.len());
        for ch in input.chars() {
            if ch.is_ascii_alphanumeric() || ch == '-' || ch == '_' {
                out.push(ch.to_ascii_lowercase());
            } else {
                out.push('_');
            }
        }
        out.trim_matches('_').to_string()
    }

    fn context_map_from_json(value: Option<&serde_json::Value>) -> BTreeMap<String, String> {
        let mut out = BTreeMap::new();
        let Some(obj) = value.and_then(|v| v.as_object()) else {
            return out;
        };
        for (k, v) in obj {
            let val = match v {
                serde_json::Value::String(s) => s.clone(),
                serde_json::Value::Null => String::new(),
                other => other.to_string(),
            };
            out.insert(k.clone(), val);
        }
        out
    }

    fn render_json_templates(
        value: &serde_json::Value,
        context: &BTreeMap<String, String>,
    ) -> serde_json::Value {
        match value {
            serde_json::Value::String(s) => {
                serde_json::Value::String(crate::core::pipeline::render_template(s, context))
            }
            serde_json::Value::Array(arr) => serde_json::Value::Array(
                arr.iter()
                    .map(|v| Self::render_json_templates(v, context))
                    .collect(),
            ),
            serde_json::Value::Object(obj) => {
                let mut map = serde_json::Map::with_capacity(obj.len());
                for (k, v) in obj {
                    map.insert(k.clone(), Self::render_json_templates(v, context));
                }
                serde_json::Value::Object(map)
            }
            other => other.clone(),
        }
    }

    fn coerce_to_json(output: &str) -> serde_json::Value {
        serde_json::from_str(output)
            .unwrap_or_else(|_| serde_json::Value::String(output.to_string()))
    }

    fn extract_status_code(message: &str) -> Option<u16> {
        for token in message.split(|c: char| !c.is_ascii_digit()) {
            if token.len() == 3 {
                if let Ok(code) = token.parse::<u16>() {
                    if (100..=599).contains(&code) {
                        return Some(code);
                    }
                }
            }
        }
        None
    }

    fn is_retryable_error(message: &str, retry: &crate::core::pipeline::RetryPolicy) -> bool {
        if let Some(status) = Self::extract_status_code(message) {
            return retry.retry_on_status.contains(&status);
        }
        let lower = message.to_ascii_lowercase();
        if lower.contains("missing ")
            || lower.contains("invalid ")
            || lower.contains("unknown action")
            || lower.contains("permission")
            || lower.contains("denied")
            || lower.contains("not found")
        {
            return false;
        }
        true
    }

    async fn sleep_with_backoff(backoff_ms: u64, jitter_ratio: f64) {
        let sleep_ms = if jitter_ratio <= 0.0 {
            backoff_ms.max(25)
        } else {
            use rand::Rng;
            let span = ((backoff_ms as f64) * jitter_ratio).round() as i64;
            if span <= 0 {
                backoff_ms.max(25)
            } else {
                let mut rng = rand::thread_rng();
                let jitter = rng.gen_range(-span..=span);
                ((backoff_ms as i64 + jitter).max(25)) as u64
            }
        };
        tokio::time::sleep(std::time::Duration::from_millis(sleep_ms)).await;
    }

    async fn execute_tunnel_control(&self, arguments: &serde_json::Value) -> Result<String> {
        let action = arguments
            .get("action")
            .and_then(|v| v.as_str())
            .unwrap_or("status")
            .to_ascii_lowercase();

        let bind_addr =
            std::env::var("AGENTARK_BIND").unwrap_or_else(|_| "127.0.0.1:8990".to_string());
        let tls_enabled = std::env::var("AGENTARK_TLS_CERT")
            .ok()
            .filter(|s| !s.trim().is_empty())
            .is_some()
            && std::env::var("AGENTARK_TLS_KEY")
                .ok()
                .filter(|s| !s.trim().is_empty())
                .is_some();
        let scheme = if tls_enabled { "https" } else { "http" };
        let base_url = format!("{}://{}", scheme, bind_addr);

        let client = reqwest::Client::builder()
            .danger_accept_invalid_certs(true)
            .build()
            .map_err(|e| anyhow::anyhow!("Failed to build HTTP client: {}", e))?;

        let endpoint = match action.as_str() {
            "start" => "/tunnel/start",
            "stop" => "/tunnel/stop",
            "status" => "/tunnel/status",
            other => {
                return Err(anyhow::anyhow!(
                    "Invalid action '{}'. Use start, stop, or status.",
                    other
                ))
            }
        };

        let mut req = match action.as_str() {
            "status" => client.get(format!("{}{}", base_url, endpoint)),
            _ => client.post(format!("{}{}", base_url, endpoint)),
        };

        if let Ok(mgr) =
            SecureConfigManager::new_with_data_dir(&self.config_dir, Some(self.data_dir()))
        {
            if let Ok(Some(key)) = mgr.get_api_key() {
                if !key.trim().is_empty() {
                    req = req.bearer_auth(key);
                }
            }
        }

        let resp = req
            .send()
            .await
            .map_err(|e| anyhow::anyhow!("Failed to reach tunnel controller: {}", e))?;
        let status = resp.status();
        let payload: serde_json::Value =
            resp.json().await.unwrap_or_else(|_| serde_json::json!({}));
        if !status.is_success() {
            let err = payload
                .get("error")
                .and_then(|v| v.as_str())
                .unwrap_or("unknown error");
            return Err(anyhow::anyhow!("Tunnel command failed: {}", err));
        }

        match action.as_str() {
            "start" => {
                let url = payload.get("url").and_then(|v| v.as_str()).unwrap_or("");
                if !url.is_empty() {
                    Ok(format!("Tunnel started.\nExternal URL: {}", url))
                } else {
                    Ok(payload
                        .get("message")
                        .and_then(|v| v.as_str())
                        .unwrap_or("Tunnel is starting; URL pending.")
                        .to_string())
                }
            }
            "stop" => Ok(payload
                .get("message")
                .and_then(|v| v.as_str())
                .unwrap_or("Tunnel stopped.")
                .to_string()),
            _ => {
                let active = payload
                    .get("active")
                    .and_then(|v| v.as_bool())
                    .unwrap_or(false);
                let mut out = format!(
                    "Tunnel status: {}",
                    if active { "active" } else { "inactive" }
                );
                if let Some(url) = payload.get("url").and_then(|v| v.as_str()) {
                    if !url.is_empty() {
                        out.push_str(&format!("\nExternal URL: {}", url));
                    }
                }
                if let Some(err) = payload.get("error").and_then(|v| v.as_str()) {
                    if !err.is_empty() {
                        out.push_str(&format!("\nLast error: {}", err));
                    }
                }
                Ok(out)
            }
        }
    }

    /// Convert HTML to plain text by stripping tags and decoding entities
    fn html_to_text(html: &str) -> String {
        // Remove script and style blocks entirely
        let script_re =
            regex::Regex::new(r"(?is)<(script|style|noscript)[^>]*>.*?</(script|style|noscript)>")
                .unwrap();
        let cleaned = script_re.replace_all(html, "");

        // Remove HTML comments
        let comment_re = regex::Regex::new(r"(?s)<!--.*?-->").unwrap();
        let cleaned = comment_re.replace_all(&cleaned, "");

        // Replace block-level elements with newlines
        let block_re = regex::Regex::new(r"(?i)</(p|div|h[1-6]|li|tr|br|hr)[^>]*>").unwrap();
        let cleaned = block_re.replace_all(&cleaned, "\n");
        let br_re = regex::Regex::new(r"(?i)<br[^>]*/?>").unwrap();
        let cleaned = br_re.replace_all(&cleaned, "\n");

        // Strip all remaining HTML tags
        let tag_re = regex::Regex::new(r"<[^>]+>").unwrap();
        let cleaned = tag_re.replace_all(&cleaned, "");

        // Decode common HTML entities
        let text = cleaned
            .replace("&amp;", "&")
            .replace("&lt;", "<")
            .replace("&gt;", ">")
            .replace("&quot;", "\"")
            .replace("&apos;", "'")
            .replace("&#39;", "'")
            .replace("&nbsp;", " ")
            .replace("&#x27;", "'")
            .replace("&#x2F;", "/");

        // Collapse multiple whitespace/newlines
        let ws_re = regex::Regex::new(r"[ \t]+").unwrap();
        let text = ws_re.replace_all(&text, " ");
        let nl_re = regex::Regex::new(r"\n{3,}").unwrap();
        let text = nl_re.replace_all(&text, "\n\n");

        // Trim lines and overall result
        let text: String = text
            .lines()
            .map(|l| l.trim())
            .collect::<Vec<_>>()
            .join("\n");

        // Truncate to reasonable size (10000 chars)
        if text.len() > 10000 {
            format!(
                "{}...\n\n(content truncated at 10000 characters)",
                &text[..10000]
            )
        } else {
            text.trim().to_string()
        }
    }

    /// Execute an action in WASM sandbox
    async fn execute_wasm(
        &self,
        action_name: &str,
        arguments: &serde_json::Value,
    ) -> Result<String> {
        // For built-in actions, fall back to native with some wrapping
        match action_name {
            "http_get" => {
                let url = arguments["url"]
                    .as_str()
                    .ok_or_else(|| anyhow::anyhow!("Missing url"))?;

                let client = reqwest::Client::new();
                let mut req = client.get(url);
                if let Some(headers) = arguments.get("headers").and_then(|v| v.as_object()) {
                    for (k, v) in headers {
                        if let Some(s) = v.as_str() {
                            req = req.header(k, s);
                        }
                    }
                }
                let response = req.send().await?;
                let body = response.text().await?;

                Ok(body)
            }
            "manage_actions" => {
                let operation = arguments
                    .get("operation")
                    .and_then(|v| v.as_str())
                    .ok_or_else(|| anyhow::anyhow!("Missing 'operation' parameter"))?;

                match operation {
                    "create" => {
                        let name = arguments
                            .get("name")
                            .and_then(|v| v.as_str())
                            .ok_or_else(|| anyhow::anyhow!("Missing 'name' for create"))?;
                        let content = arguments
                            .get("content")
                            .and_then(|v| v.as_str())
                            .ok_or_else(|| anyhow::anyhow!("Missing 'content' for create"))?;
                        if !name
                            .chars()
                            .all(|c| c.is_alphanumeric() || c == '-' || c == '_')
                        {
                            return Err(anyhow::anyhow!(
                                "Invalid action name. Use kebab-case (e.g., 'check-weather')"
                            ));
                        }
                        if self.actions.read().await.contains_key(name) {
                            return Err(anyhow::anyhow!(
                                "Action '{}' already exists. Use 'update' instead.",
                                name
                            ));
                        }
                        let verdict = self.create_action(name, content, false).await?;
                        let mut msg =
                            format!("Action '{}' created and is immediately available.", name);
                        if let Some(ref v) = verdict {
                            if !v.warnings.is_empty() {
                                msg.push_str(&format!(
                                    "\n⚠️ Security warnings: {}",
                                    v.warnings.join(", ")
                                ));
                            }
                            if !v.allow_load {
                                msg = format!(
                                    "Action '{}' was blocked by security verification: {}",
                                    name,
                                    v.warnings.join(", ")
                                );
                            }
                        }
                        Ok(msg)
                    }
                    "update" => {
                        let name = arguments
                            .get("name")
                            .and_then(|v| v.as_str())
                            .ok_or_else(|| anyhow::anyhow!("Missing 'name' for update"))?;
                        let content = arguments
                            .get("content")
                            .and_then(|v| v.as_str())
                            .ok_or_else(|| anyhow::anyhow!("Missing 'content' for update"))?;
                        match self.update_action_content(name, content).await? {
                            true => Ok(format!("Action '{}' updated.", name)),
                            false => Err(anyhow::anyhow!(
                                "Cannot update '{}'. System actions are read-only.",
                                name
                            )),
                        }
                    }
                    "delete" => {
                        let name = arguments
                            .get("name")
                            .and_then(|v| v.as_str())
                            .ok_or_else(|| anyhow::anyhow!("Missing 'name' for delete"))?;
                        match self.delete_action(name).await? {
                            true => Ok(format!("Action '{}' deleted.", name)),
                            false => Err(anyhow::anyhow!(
                                "Cannot delete '{}'. Only custom actions can be deleted.",
                                name
                            )),
                        }
                    }
                    "list" => {
                        let actions = self.list_actions().await?;
                        let list: Vec<String> = actions
                            .iter()
                            .map(|a| {
                                let source = match a.source {
                                    ActionSource::System => "system",
                                    ActionSource::Bundled => "bundled",
                                    ActionSource::Custom => "custom",
                                };
                                format!("- **{}** ({}): {}", a.name, source, a.description)
                            })
                            .collect();
                        Ok(format!(
                            "Available actions ({}):\n{}",
                            actions.len(),
                            list.join("\n")
                        ))
                    }
                    _ => Err(anyhow::anyhow!(
                        "Unknown operation '{}'. Use create, update, delete, or list.",
                        operation
                    )),
                }
            }
            _ => {
                // Check if we have a WASM module for this action
                let actions = self.actions.read().await;
                if let Some(action) = actions.get(action_name) {
                    if let Some(wasm_bytes) = &action.wasm_module {
                        let wasm = wasm_bytes.clone();
                        drop(actions); // Release lock before async call
                        return self.run_wasm_module(&wasm, arguments).await;
                    }
                }
                drop(actions); // Release lock before async call
                               // Fall back to native
                self.execute_native(action_name, arguments).await
            }
        }
    }

    /// Connect to Docker, preferring DOCKER_HOST env var (for socket proxy) over local defaults
    #[cfg(feature = "docker")]
    fn connect_docker() -> Result<bollard::Docker> {
        if let Ok(host) = std::env::var("DOCKER_HOST") {
            tracing::debug!("Connecting to Docker via DOCKER_HOST={}", host);
            bollard::Docker::connect_with_http_defaults()
                .map_err(|e| anyhow::anyhow!("Failed to connect to Docker at {}: {}", host, e))
        } else {
            bollard::Docker::connect_with_local_defaults()
                .map_err(|e| anyhow::anyhow!("Failed to connect to Docker: {}", e))
        }
    }

    /// Execute an action in Docker sandbox
    async fn execute_docker(
        &self,
        action_name: &str,
        arguments: &serde_json::Value,
    ) -> Result<String> {
        #[cfg(feature = "docker")]
        {
            // Check Docker availability first — fall back to native if unavailable
            let docker_available = Self::connect_docker().is_ok();

            if !docker_available {
                tracing::warn!("Docker not available (socket not found), falling back to native execution for '{}'", action_name);
                return self.execute_native(action_name, arguments).await;
            }

            match action_name {
                "shell" => {
                    self.run_isolated_container(
                        &self.config.docker_image,
                        vec![
                            "sh".to_string(),
                            "-c".to_string(),
                            arguments["command"]
                                .as_str()
                                .ok_or_else(|| anyhow::anyhow!("Missing command"))?
                                .to_string(),
                        ],
                        None,
                        30,
                        ContainerIsolation::Strict,
                    )
                    .await
                }
                "code_execute" => self.execute_code_docker(arguments).await,
                _ => Err(anyhow::anyhow!("Unknown docker action: {}", action_name)),
            }
        }

        #[cfg(not(feature = "docker"))]
        {
            // Fall back to native sandboxed execution when Docker is not available
            if action_name == "code_execute" {
                return self.execute_code_native(arguments).await;
            }
            Err(anyhow::anyhow!(
                "Docker support not enabled. Recompile with --features docker"
            ))
        }
    }

    /// Force-remove a Docker container (stop + remove), ignoring errors.
    /// Guaranteed to not leave containers behind.
    #[cfg(feature = "docker")]
    async fn force_remove_container(docker: &bollard::Docker, id: &str) {
        // Kill first (faster than stop for stuck containers)
        let _ = docker.kill_container::<String>(id, None).await;
        // Stop as fallback (handles already-stopped containers)
        let _ = docker
            .stop_container(id, Some(bollard::container::StopContainerOptions { t: 0 }))
            .await;
        // Force remove — deletes container, volumes, and anonymous volumes
        let _ = docker
            .remove_container(
                id,
                Some(bollard::container::RemoveContainerOptions {
                    force: true,
                    v: true, // Remove anonymous volumes attached to the container
                    ..Default::default()
                }),
            )
            .await;
        tracing::debug!("Cleaned up container {}", &id[..12.min(id.len())]);
    }

    /// Ensure a Docker image is available locally, pulling it if necessary.
    #[cfg(feature = "docker")]
    async fn ensure_image(docker: &bollard::Docker, image: &str) -> Result<()> {
        // Check if image exists locally
        if docker.inspect_image(image).await.is_ok() {
            return Ok(());
        }

        tracing::info!("Pulling Docker image '{}' (first-time download)...", image);

        let mut stream = docker.create_image(
            Some(bollard::image::CreateImageOptions {
                from_image: image,
                ..Default::default()
            }),
            None,
            None,
        );

        use futures::StreamExt;
        while let Some(result) = stream.next().await {
            match result {
                Ok(info) => {
                    if let Some(status) = &info.status {
                        tracing::debug!("Pull {}: {}", image, status);
                    }
                }
                Err(e) => {
                    return Err(anyhow::anyhow!("Failed to pull image '{}': {}", image, e));
                }
            }
        }

        tracing::info!("Image '{}' pulled successfully", image);
        Ok(())
    }

    /// Run a command in a fully isolated, ephemeral Docker container.
    /// Automatically pulls the image if not available locally.
    /// Container is ALWAYS destroyed after execution — no leftovers.
    #[cfg(feature = "docker")]
    async fn run_isolated_container(
        &self,
        image: &str,
        cmd: Vec<String>,
        env: Option<Vec<String>>,
        timeout_secs: u64,
        isolation: ContainerIsolation,
    ) -> Result<String> {
        let docker = Self::connect_docker()?;

        // Auto-pull image if not available
        Self::ensure_image(&docker, image).await?;

        let host_config = match isolation {
            ContainerIsolation::Strict => {
                // Full lockdown: no network, read-only root, noexec /tmp
                bollard::models::HostConfig {
                    memory: Some(256 * 1024 * 1024),
                    memory_swap: Some(256 * 1024 * 1024),
                    cpu_period: Some(100_000),
                    cpu_quota: Some(50_000),
                    pids_limit: Some(64),
                    network_mode: Some("none".to_string()),
                    readonly_rootfs: Some(true),
                    tmpfs: Some(HashMap::from([(
                        "/tmp".to_string(),
                        "size=64M,noexec".to_string(),
                    )])),
                    auto_remove: Some(false),
                    ..Default::default()
                }
            }
            ContainerIsolation::Standard => {
                // Allows pip/npm install: writable fs, network enabled, but still
                // resource-limited and ephemeral (destroyed after execution)
                bollard::models::HostConfig {
                    memory: Some(512 * 1024 * 1024), // 512MB for installs
                    memory_swap: Some(512 * 1024 * 1024),
                    cpu_period: Some(100_000),
                    cpu_quota: Some(50_000),
                    pids_limit: Some(128), // More PIDs for package managers
                    auto_remove: Some(false),
                    ..Default::default()
                }
            }
        };

        let network_disabled = matches!(isolation, ContainerIsolation::Strict);

        let container_config = bollard::container::Config {
            image: Some(image.to_string()),
            cmd: Some(cmd),
            env,
            host_config: Some(host_config),
            network_disabled: Some(network_disabled),
            working_dir: Some("/tmp".to_string()),
            ..Default::default()
        };

        let container = docker
            .create_container::<String, String>(None, container_config)
            .await
            .map_err(|e| anyhow::anyhow!("Failed to create container: {}", e))?;

        let container_id = container.id.clone();
        tracing::info!(
            "Created isolated container {} for code execution",
            &container_id[..12.min(container_id.len())]
        );

        // Start container — if this fails, clean up immediately
        if let Err(e) = docker.start_container::<String>(&container_id, None).await {
            Self::force_remove_container(&docker, &container_id).await;
            return Err(anyhow::anyhow!("Failed to start container: {}", e));
        }

        // Wait for container by polling inspect (more reliable than wait_container stream)
        let deadline = std::time::Instant::now() + std::time::Duration::from_secs(timeout_secs);
        let exit_code = loop {
            if std::time::Instant::now() > deadline {
                Self::force_remove_container(&docker, &container_id).await;
                return Err(anyhow::anyhow!(
                    "Code execution timed out after {} seconds",
                    timeout_secs
                ));
            }

            match docker.inspect_container(&container_id, None).await {
                Ok(info) => {
                    let running = info.state.as_ref().and_then(|s| s.running).unwrap_or(false);
                    if !running {
                        let code = info.state.as_ref().and_then(|s| s.exit_code).unwrap_or(-1);
                        tracing::debug!(
                            "Container {} exited with code {}",
                            &container_id[..12.min(container_id.len())],
                            code
                        );
                        break code;
                    }
                }
                Err(e) => {
                    tracing::warn!("Failed to inspect container: {:?}", e);
                    Self::force_remove_container(&docker, &container_id).await;
                    return Err(anyhow::anyhow!("Container inspection failed: {}", e));
                }
            }

            tokio::time::sleep(std::time::Duration::from_millis(500)).await;
        };

        // Collect stdout and stderr before cleanup
        let logs = docker
            .logs::<String>(
                &container_id,
                Some(bollard::container::LogsOptions {
                    stdout: true,
                    stderr: true,
                    ..Default::default()
                }),
            )
            .try_collect::<Vec<_>>()
            .await
            .unwrap_or_default();

        // Always destroy the container — no leftovers
        Self::force_remove_container(&docker, &container_id).await;

        let mut stdout = String::new();
        let mut stderr = String::new();
        for log in &logs {
            match log {
                bollard::container::LogOutput::StdOut { message } => {
                    stdout.push_str(&String::from_utf8_lossy(message));
                }
                bollard::container::LogOutput::StdErr { message } => {
                    stderr.push_str(&String::from_utf8_lossy(message));
                }
                _ => {}
            }
        }

        let result = serde_json::json!({
            "output": stdout,
            "error": if stderr.is_empty() { serde_json::Value::Null } else { serde_json::Value::String(stderr.clone()) },
            "exit_code": exit_code,
        });

        Ok(serde_json::to_string(&result)?)
    }

    /// Resolve a language name to (docker_image, file_extension, build_cmd, run_cmd).
    /// build_cmd is optional (for compiled languages like Java, Go, Rust, C).
    /// Returns None only if the language is completely unrecognized.
    fn resolve_language(
        lang: &str,
    ) -> Option<(
        &'static str,
        &'static str,
        Option<&'static str>,
        &'static str,
    )> {
        // (image, extension, optional_build_cmd, run_cmd)
        // {file} is replaced with /tmp/code.{ext} at runtime
        match lang {
            // Interpreted
            "python" | "python3" | "py"
                => Some(("python:3-slim", "py", None, "PIP_ROOT_USER_ACTION=ignore PIP_DISABLE_PIP_VERSION_CHECK=1 python3 {file}")),
            "javascript" | "js" | "node"
                => Some(("node:22-slim", "js", None, "node {file}")),
            "typescript" | "ts"
                => Some(("node:22-slim", "ts", Some("npm i -g tsx 2>/dev/null"), "npx tsx {file}")),
            "bash" | "sh" | "shell"
                => Some(("bash:latest", "sh", None, "bash {file}")),
            "ruby" | "rb"
                => Some(("ruby:3-slim", "rb", None, "ruby {file}")),
            "php"
                => Some(("php:8-cli", "php", None, "php {file}")),
            "perl" | "pl"
                => Some(("perl:5-slim", "pl", None, "perl {file}")),
            "lua"
                => Some(("nickblah/lua:5.4", "lua", None, "lua {file}")),
            "r" | "rlang"
                => Some(("r-base:latest", "R", None, "Rscript {file}")),

            // Compiled
            "java"
                => Some(("eclipse-temurin:21-jdk", "java", Some("javac {file}"), "java -cp /tmp Main")),
            "c"
                => Some(("gcc:latest", "c", Some("gcc {file} -o /tmp/a.out -lm"), "/tmp/a.out")),
            "cpp" | "c++"
                => Some(("gcc:latest", "cpp", Some("g++ {file} -o /tmp/a.out -lm"), "/tmp/a.out")),
            "go" | "golang"
                => Some(("golang:1-bookworm", "go", None, "go run {file}")),
            "rust" | "rs"
                => Some(("rust:1-slim-bookworm", "rs", Some("rustc {file} -o /tmp/a.out"), "/tmp/a.out")),
            "swift"
                => Some(("swift:latest", "swift", None, "swift {file}")),
            "kotlin" | "kt"
                => Some(("zenika/kotlin:latest", "kt", Some("kotlinc {file} -include-runtime -d /tmp/out.jar 2>/dev/null"), "java -jar /tmp/out.jar")),

            // Jupyter notebook — execute in-place and output results
            "jupyter" | "notebook" | "ipynb"
                => Some(("python:3-slim", "ipynb",
                    Some("PIP_ROOT_USER_ACTION=ignore PIP_DISABLE_PIP_VERSION_CHECK=1 pip install -q jupyter nbconvert nbformat matplotlib pandas numpy scikit-learn seaborn 2>/dev/null"),
                    "jupyter nbconvert --to notebook --execute --inplace {file} 2>&1 && python3 -c \"import json; nb=json.load(open('{file}')); [print(o.get('text','')) for c in nb['cells'] for o in c.get('outputs',[]) if o.get('output_type')=='stream']\" ")),

            _ => None,
        }
    }

    /// Detect non-stdlib Python imports and return a pip install command.
    /// Scans `import X` and `from X import` statements, filters out stdlib modules.
    fn detect_python_deps(code: &str) -> String {
        // Python stdlib modules (comprehensive but not exhaustive — errs on side of not installing)
        const STDLIB: &[&str] = &[
            "abc",
            "aifc",
            "argparse",
            "array",
            "ast",
            "asynchat",
            "asyncio",
            "asyncore",
            "atexit",
            "base64",
            "bdb",
            "binascii",
            "binhex",
            "bisect",
            "builtins",
            "bz2",
            "calendar",
            "cgi",
            "cgitb",
            "chunk",
            "cmath",
            "cmd",
            "code",
            "codecs",
            "codeop",
            "collections",
            "colorsys",
            "compileall",
            "concurrent",
            "configparser",
            "contextlib",
            "contextvars",
            "copy",
            "copyreg",
            "cProfile",
            "crypt",
            "csv",
            "ctypes",
            "curses",
            "dataclasses",
            "datetime",
            "dbm",
            "decimal",
            "difflib",
            "dis",
            "distutils",
            "doctest",
            "email",
            "encodings",
            "enum",
            "errno",
            "faulthandler",
            "fcntl",
            "filecmp",
            "fileinput",
            "fnmatch",
            "formatter",
            "fractions",
            "ftplib",
            "functools",
            "gc",
            "getopt",
            "getpass",
            "gettext",
            "glob",
            "grp",
            "gzip",
            "hashlib",
            "heapq",
            "hmac",
            "html",
            "http",
            "idlelib",
            "imaplib",
            "imghdr",
            "imp",
            "importlib",
            "inspect",
            "io",
            "ipaddress",
            "itertools",
            "json",
            "keyword",
            "lib2to3",
            "linecache",
            "locale",
            "logging",
            "lzma",
            "mailbox",
            "mailcap",
            "marshal",
            "math",
            "mimetypes",
            "mmap",
            "modulefinder",
            "multiprocessing",
            "netrc",
            "nis",
            "nntplib",
            "numbers",
            "operator",
            "optparse",
            "os",
            "ossaudiodev",
            "parser",
            "pathlib",
            "pdb",
            "pickle",
            "pickletools",
            "pipes",
            "pkgutil",
            "platform",
            "plistlib",
            "poplib",
            "posix",
            "posixpath",
            "pprint",
            "profile",
            "pstats",
            "pty",
            "pwd",
            "py_compile",
            "pyclbr",
            "pydoc",
            "queue",
            "quopri",
            "random",
            "re",
            "readline",
            "reprlib",
            "resource",
            "rlcompleter",
            "runpy",
            "sched",
            "secrets",
            "select",
            "selectors",
            "shelve",
            "shlex",
            "shutil",
            "signal",
            "site",
            "smtpd",
            "smtplib",
            "sndhdr",
            "socket",
            "socketserver",
            "sqlite3",
            "ssl",
            "stat",
            "statistics",
            "string",
            "stringprep",
            "struct",
            "subprocess",
            "sunau",
            "symtable",
            "sys",
            "sysconfig",
            "syslog",
            "tabnanny",
            "tarfile",
            "telnetlib",
            "tempfile",
            "termios",
            "test",
            "textwrap",
            "threading",
            "time",
            "timeit",
            "tkinter",
            "token",
            "tokenize",
            "trace",
            "traceback",
            "tracemalloc",
            "tty",
            "turtle",
            "turtledemo",
            "types",
            "typing",
            "unicodedata",
            "unittest",
            "urllib",
            "uu",
            "uuid",
            "venv",
            "warnings",
            "wave",
            "weakref",
            "webbrowser",
            "winreg",
            "winsound",
            "wsgiref",
            "xdrlib",
            "xml",
            "xmlrpc",
            "zipapp",
            "zipfile",
            "zipimport",
            "zlib",
            "_thread",
            "__future__",
        ];

        // Well-known pip package name mappings (import name -> pip package)
        fn pip_name(module: &str) -> String {
            match module {
                "PIL" | "Pillow" => "Pillow".to_string(),
                "cv2" => "opencv-python".to_string(),
                "sklearn" | "scikit_learn" => "scikit-learn".to_string(),
                "bs4" => "beautifulsoup4".to_string(),
                "yaml" => "pyyaml".to_string(),
                "dotenv" => "python-dotenv".to_string(),
                "gi" => "PyGObject".to_string(),
                "attr" | "attrs" => "attrs".to_string(),
                "dateutil" => "python-dateutil".to_string(),
                "jwt" => "PyJWT".to_string(),
                "crypto" | "Crypto" => "pycryptodome".to_string(),
                "serial" => "pyserial".to_string(),
                "usb" => "pyusb".to_string(),
                "wx" => "wxPython".to_string(),
                "skimage" => "scikit-image".to_string(),
                _ => module.to_string(),
            }
        }

        // Valid pip package names: alphanumeric, hyphens, underscores, dots
        fn is_valid_package_name(name: &str) -> bool {
            !name.is_empty()
                && name.len() <= 100
                && name
                    .chars()
                    .all(|c| c.is_alphanumeric() || c == '-' || c == '_' || c == '.')
                && name.chars().next().is_some_and(|c| c.is_alphanumeric())
        }

        let mut deps = std::collections::HashSet::new();

        for line in code.lines() {
            let line = line.trim();
            // Skip lines inside strings/comments (heuristic: skip if line starts with #, ', ", or is indented code with non-import content)
            if line.starts_with('#') || line.starts_with('"') || line.starts_with('\'') {
                continue;
            }
            // import X, Y, Z
            if let Some(rest) = line.strip_prefix("import ") {
                for part in rest.split(',') {
                    let module = part
                        .split_whitespace()
                        .next()
                        .unwrap_or("")
                        .split('.')
                        .next()
                        .unwrap_or("");
                    if !module.is_empty()
                        && !STDLIB.contains(&module)
                        && is_valid_package_name(module)
                    {
                        deps.insert(pip_name(module));
                    }
                }
            }
            // from X import ...
            else if let Some(rest) = line.strip_prefix("from ") {
                let module = rest
                    .split_whitespace()
                    .next()
                    .unwrap_or("")
                    .split('.')
                    .next()
                    .unwrap_or("");
                if !module.is_empty() && !STDLIB.contains(&module) && is_valid_package_name(module)
                {
                    deps.insert(pip_name(module));
                }
            }
        }

        if deps.is_empty() {
            return String::new();
        }

        let dep_list: Vec<&str> = deps.iter().map(|s| s.as_str()).collect();
        tracing::info!("Auto-detected Python deps: {:?}", dep_list);
        format!(
            "PIP_ROOT_USER_ACTION=ignore PIP_DISABLE_PIP_VERSION_CHECK=1 pip install -q {} && ",
            dep_list.join(" ")
        )
    }

    /// Detect non-builtin Node.js requires/imports and return an npm install command.
    fn detect_node_deps(code: &str) -> String {
        // Node.js built-in modules
        const BUILTINS: &[&str] = &[
            "assert",
            "buffer",
            "child_process",
            "cluster",
            "console",
            "constants",
            "crypto",
            "dgram",
            "dns",
            "domain",
            "events",
            "fs",
            "http",
            "https",
            "module",
            "net",
            "os",
            "path",
            "perf_hooks",
            "process",
            "punycode",
            "querystring",
            "readline",
            "repl",
            "stream",
            "string_decoder",
            "sys",
            "timers",
            "tls",
            "tty",
            "url",
            "util",
            "v8",
            "vm",
            "worker_threads",
            "zlib",
        ];

        let mut deps = std::collections::HashSet::new();

        for line in code.lines() {
            let line = line.trim();
            // require('pkg') or require("pkg")
            if line.contains("require(") {
                for cap in line.split("require(").skip(1) {
                    let pkg = cap
                        .trim_start_matches(['\'', '"'])
                        .split(['\'', '"'])
                        .next()
                        .unwrap_or("");
                    let root = pkg.split('/').next().unwrap_or("");
                    if !root.is_empty() && !root.starts_with('.') && !BUILTINS.contains(&root) {
                        deps.insert(root.to_string());
                    }
                }
            }
            // import ... from 'pkg'
            if line.starts_with("import ") {
                if let Some(from_part) = line.rsplit("from ").next() {
                    let pkg = from_part.trim().trim_matches([' ', '\'', '"', ';']);
                    let root = pkg.split('/').next().unwrap_or("");
                    if !root.is_empty() && !root.starts_with('.') && !BUILTINS.contains(&root) {
                        deps.insert(root.to_string());
                    }
                }
            }
        }

        if deps.is_empty() {
            return String::new();
        }

        let dep_list: Vec<&str> = deps.iter().map(|s| s.as_str()).collect();
        tracing::info!("Auto-detected Node.js deps: {:?}", dep_list);
        format!(
            "npm install --no-fund --no-audit -q {} 2>/dev/null && ",
            dep_list.join(" ")
        )
    }

    /// Execute code in an isolated Docker container.
    /// Supports any language with a Docker image — auto-pulls if needed.
    /// Container is ephemeral — fully destroyed after execution.
    /// Output files (images, CSVs, etc.) are extracted before container cleanup.
    #[cfg(feature = "docker")]
    async fn execute_code_docker(&self, arguments: &serde_json::Value) -> Result<String> {
        let language = arguments["language"]
            .as_str()
            .ok_or_else(|| anyhow::anyhow!("Missing 'language' argument"))?
            .to_lowercase();
        let code_raw = arguments["code"]
            .as_str()
            .ok_or_else(|| anyhow::anyhow!("Missing 'code' argument"))?;

        // Strip Jupyter magic commands (!pip, !apt, !conda, %pip, %conda, etc.)
        // LLMs often generate these in regular Python scripts — our auto-dependency
        // detection handles installs, so these lines are unnecessary and cause SyntaxError.
        let code = if matches!(language.as_str(), "python" | "python3" | "py") {
            let cleaned: Vec<&str> = code_raw
                .lines()
                .filter(|line| {
                    let trimmed = line.trim();
                    !trimmed.starts_with("!pip ")
                        && !trimmed.starts_with("!pip3 ")
                        && !trimmed.starts_with("!apt ")
                        && !trimmed.starts_with("!apt-get ")
                        && !trimmed.starts_with("!conda ")
                        && !trimmed.starts_with("%pip ")
                        && !trimmed.starts_with("%conda ")
                        && !trimmed.starts_with("!sudo ")
                })
                .collect();
            cleaned.join("\n")
        } else {
            code_raw.to_string()
        };
        let code = code.as_str();

        let (image, ext, build_cmd, run_cmd) = Self::resolve_language(&language)
            .ok_or_else(|| anyhow::anyhow!(
                "Unsupported language '{}'. Supported: python, javascript, typescript, bash, ruby, php, perl, lua, r, java, c, cpp, go, rust, swift, kotlin",
                language
            ))?;

        let code_b64 = base64::Engine::encode(&base64::engine::general_purpose::STANDARD, code);
        let file_path = format!("/tmp/code.{}", ext);
        // Java needs the file named Main.java
        let file_path = if language == "java" {
            "/tmp/Main.java".to_string()
        } else {
            file_path
        };

        // Build file injection commands for user-attached files
        // Files are read from host, base64-encoded, and decoded into /data/ inside container
        let mut file_inject_cmds = String::new();
        if let Some(files_arr) = arguments.get("files").and_then(|v| v.as_array()) {
            file_inject_cmds.push_str("mkdir -p /data && ");
            for file_val in files_arr {
                if let Some(local_path) = file_val.as_str() {
                    let path = std::path::Path::new(local_path);
                    // Validate path exists and get filename
                    if let Ok(data) = tokio::fs::read(path).await {
                        let filename = path.file_name().and_then(|n| n.to_str()).unwrap_or("file");
                        let data_b64 = base64::Engine::encode(
                            &base64::engine::general_purpose::STANDARD,
                            &data,
                        );
                        file_inject_cmds.push_str(&format!(
                            "echo '{}' | base64 -d > /data/{} && ",
                            data_b64, filename
                        ));
                        tracing::info!(
                            "Injecting file into container: {} ({} bytes)",
                            filename,
                            data.len()
                        );
                    } else {
                        tracing::warn!("Could not read attached file: {}", local_path);
                    }
                }
            }
        }

        // Auto-detect dependencies for Python: scan imports, pip install non-stdlib packages
        let auto_install_cmd = if matches!(language.as_str(), "python" | "python3" | "py") {
            Self::detect_python_deps(code)
        } else if matches!(
            language.as_str(),
            "javascript" | "js" | "node" | "typescript" | "ts"
        ) {
            Self::detect_node_deps(code)
        } else {
            String::new()
        };

        let run = run_cmd.replace("{file}", &file_path);
        let main_cmd = if let Some(build) = build_cmd {
            let build = build.replace("{file}", &file_path);
            format!(
                "{}{}echo '{}' | base64 -d > {} && {} && {}",
                file_inject_cmds, auto_install_cmd, code_b64, file_path, build, run
            )
        } else {
            format!(
                "{}{}echo '{}' | base64 -d > {} && {}",
                file_inject_cmds, auto_install_cmd, code_b64, file_path, run
            )
        };

        // Append file extraction: finds files created/modified during execution,
        // excludes build artifacts, base64-encodes each file (up to 5MB) and
        // outputs them with markers so we can extract before container dies.
        // For notebooks (.ipynb), we also capture the executed notebook itself.
        let is_notebook = ext == "ipynb";
        let notebook_extra = if is_notebook {
            // Also extract the executed notebook file
            format!(
                r#" echo "FILE:$(basename {file}):$(base64 {file} | tr -d '\n')";"#,
                file = file_path
            )
        } else {
            String::new()
        };
        let shell_cmd = format!(
            r#"{}; __AGENTARK_EXIT=$?; echo; echo '__AGENTARK_OUTPUT_FILES__';{} find /tmp -maxdepth 2 -type f ! -name 'code.*' ! -name 'a.out' ! -name 'Main.*' ! -name '*.class' ! -name 'out.jar' ! -name '*.ipynb' -newer {} 2>/dev/null | head -20 | while IFS= read -r __f; do __sz=$(stat -c%s "$__f" 2>/dev/null || echo 999999999); if [ "$__sz" -lt 5242880 ]; then echo "FILE:$(basename "$__f"):$(base64 "$__f" | tr -d '\n')"; fi; done; exit $__AGENTARK_EXIT"#,
            main_cmd, notebook_extra, file_path
        );

        // Notebooks get 10 min (install deps + execute all cells + ML training)
        // Compiled languages get 120s (build + run), interpreted get 60s
        let timeout = if is_notebook {
            600
        } else if build_cmd.is_some() {
            120
        } else {
            60
        };

        // Optional env vars for execution (resolved placeholders are already applied by the runtime).
        let env_vec: Option<Vec<String>> = arguments
            .get("env")
            .and_then(|v| v.as_object())
            .map(|obj| {
                obj.iter()
                    .filter_map(|(k, v)| v.as_str().map(|s| format!("{}={}", k, s)))
                    .collect::<Vec<_>>()
            })
            .filter(|v| !v.is_empty());

        let raw_result = self
            .run_isolated_container(
                image,
                vec!["sh".to_string(), "-c".to_string(), shell_cmd],
                env_vec,
                timeout,
                ContainerIsolation::Standard,
            )
            .await?;

        // Parse result and extract output files from stdout
        let parsed: serde_json::Value = serde_json::from_str(&raw_result)?;
        let output = parsed["output"].as_str().unwrap_or("");

        let exec_id = uuid::Uuid::new_v4().to_string();
        let output_dir = self.data_dir().join("outputs").join(&exec_id);

        let (user_output, saved_files) = if let Some(marker_pos) =
            output.find("__AGENTARK_OUTPUT_FILES__")
        {
            let user_output = output[..marker_pos].trim_end().to_string();
            let files_section = &output[marker_pos..];

            let mut saved = Vec::new();

            // Save the code file first so user can download it
            {
                let _ = tokio::fs::create_dir_all(&output_dir).await;
                let code_filename = format!("code.{}", ext);
                let code_path = output_dir.join(&code_filename);
                if tokio::fs::write(&code_path, code).await.is_ok() {
                    saved.push(format!("/api/outputs/{}/{}", exec_id, code_filename));
                    tracing::debug!("Saved code file: {}", code_path.display());
                }
            }

            // Extract output files from container stdout
            for line in files_section.lines() {
                if let Some(rest) = line.strip_prefix("FILE:") {
                    let parts: Vec<&str> = rest.splitn(2, ':').collect();
                    if parts.len() == 2 {
                        let filename = parts[0];
                        let b64_data = parts[1];
                        use base64::Engine as _;
                        if let Ok(data) = base64::engine::general_purpose::STANDARD.decode(b64_data)
                        {
                            let out_path = output_dir.join(filename);
                            if let Ok(()) = tokio::fs::write(&out_path, &data).await {
                                let web_path = format!("/api/outputs/{}/{}", exec_id, filename);
                                saved.push(web_path);
                                tracing::info!(
                                    "Extracted output file: {} ({} bytes)",
                                    out_path.display(),
                                    data.len()
                                );
                            }
                        }
                    }
                }
            }

            (user_output, saved)
        } else {
            // No file marker found — still save the code file
            let mut saved = Vec::new();
            let _ = tokio::fs::create_dir_all(&output_dir).await;
            let code_filename = format!("code.{}", ext);
            let code_path = output_dir.join(&code_filename);
            if tokio::fs::write(&code_path, code).await.is_ok() {
                saved.push(format!("/api/outputs/{}/{}", exec_id, code_filename));
            }
            (output.to_string(), saved)
        };

        // Build final result with file paths
        let mut result = serde_json::json!({
            "output": user_output,
            "error": parsed.get("error").cloned().unwrap_or(serde_json::Value::Null),
            "exit_code": parsed.get("exit_code").cloned().unwrap_or(serde_json::json!(-1)),
        });

        if !saved_files.is_empty() {
            result["files"] = serde_json::json!(saved_files);
        }

        Ok(serde_json::to_string(&result)?)
    }

    /// Fallback: execute code natively in an isolated temp directory (no Docker)
    async fn execute_code_native(&self, arguments: &serde_json::Value) -> Result<String> {
        let language = arguments["language"]
            .as_str()
            .ok_or_else(|| anyhow::anyhow!("Missing 'language' argument"))?
            .to_lowercase();
        let code = arguments["code"]
            .as_str()
            .ok_or_else(|| anyhow::anyhow!("Missing 'code' argument"))?;

        // Native fallback: try to find the runtime on the host
        let (program, args): (&str, Vec<String>) = match language.as_str() {
            "python" | "python3" | "py" => ("python3", vec!["-c".to_string(), code.to_string()]),
            "javascript" | "js" | "node" => ("node", vec!["-e".to_string(), code.to_string()]),
            "bash" | "sh" | "shell" => ("bash", vec!["-c".to_string(), code.to_string()]),
            "ruby" | "rb" => ("ruby", vec!["-e".to_string(), code.to_string()]),
            "php" => ("php", vec!["-r".to_string(), code.to_string()]),
            "perl" | "pl" => ("perl", vec!["-e".to_string(), code.to_string()]),
            _ => {
                return Err(anyhow::anyhow!(
                "Native fallback only supports interpreted languages. Docker required for '{}'.",
                language
            ))
            }
        };

        // Create isolated temp directory for execution
        let temp_dir = std::env::temp_dir().join(format!("agentark-exec-{}", uuid::Uuid::new_v4()));
        tokio::fs::create_dir_all(&temp_dir).await?;

        // Execute with timeout, cleared env, isolated working dir
        let mut cmd = tokio::process::Command::new(program);
        cmd.args(&args)
            .current_dir(&temp_dir)
            .env_clear()
            .env("PATH", std::env::var("PATH").unwrap_or_default())
            .env("HOME", temp_dir.to_string_lossy().to_string())
            .env("TMPDIR", temp_dir.to_string_lossy().to_string())
            .stdout(std::process::Stdio::piped())
            .stderr(std::process::Stdio::piped());

        if let Some(obj) = arguments.get("env").and_then(|v| v.as_object()) {
            for (k, v) in obj {
                if let Some(s) = v.as_str() {
                    cmd.env(k, s);
                }
            }
        }

        let result = tokio::time::timeout(std::time::Duration::from_secs(30), cmd.output()).await;

        // Always clean up the temp directory
        let _ = tokio::fs::remove_dir_all(&temp_dir).await;

        let output = result
            .map_err(|_| anyhow::anyhow!("Code execution timed out after 30 seconds"))?
            .map_err(|e| anyhow::anyhow!("Failed to execute {}: {}", program, e))?;

        let stdout = String::from_utf8_lossy(&output.stdout).to_string();
        let stderr = String::from_utf8_lossy(&output.stderr).to_string();
        let exit_code = output.status.code().unwrap_or(-1);

        let result = serde_json::json!({
            "output": stdout,
            "error": if stderr.is_empty() { serde_json::Value::Null } else { serde_json::Value::String(stderr) },
            "exit_code": exit_code,
        });

        Ok(serde_json::to_string(&result)?)
    }

    /// List available actions
    pub async fn list_actions(&self) -> Result<Vec<ActionDef>> {
        let actions = self.actions.read().await;
        Ok(actions.values().map(|s| s.info.clone()).collect())
    }

    /// List only actions that are currently executable by the agent.
    /// System actions are always included; non-system actions honor the disabled set.
    pub async fn list_enabled_actions(&self) -> Result<Vec<ActionDef>> {
        let actions = self.actions.read().await;
        let disabled = self.disabled_actions.read().await;
        Ok(actions
            .values()
            .filter(|loaded| {
                loaded.info.source == ActionSource::System
                    || !disabled.contains(loaded.info.name.as_str())
            })
            .map(|loaded| loaded.info.clone())
            .collect())
    }

    /// Returns true if an action is enabled (not in the disabled set).
    pub async fn is_action_enabled(&self, name: &str) -> bool {
        let disabled = self.disabled_actions.read().await;
        !disabled.contains(name)
    }

    /// Enable or disable an action without deleting it.
    /// - System actions cannot be disabled.
    pub async fn set_action_enabled(&self, name: &str, enabled: bool) -> Result<bool> {
        let source = {
            let actions = self.actions.read().await;
            match actions.get(name) {
                Some(action) => action.info.source.clone(),
                None => return Ok(false),
            }
        };

        if source == ActionSource::System {
            return Ok(false);
        }

        {
            let mut disabled = self.disabled_actions.write().await;
            if enabled {
                disabled.remove(name);
            } else {
                disabled.insert(name.to_string());
            }
        }

        self.save_disabled_actions().await?;
        Ok(true)
    }

    /// Get action count
    pub async fn action_count(&self) -> usize {
        self.actions.read().await.len()
    }

    /// Get action info and content for editing
    pub async fn get_action_content(&self, name: &str) -> Result<Option<(ActionDef, String)>> {
        let actions = self.actions.read().await;
        if let Some(action) = actions.get(name) {
            let info = action.info.clone();
            let file_path = action.info.file_path.clone();
            let workflow = action.workflow_content.clone();
            drop(actions); // Release lock before async file I/O

            if let Some(ref fp) = file_path {
                let content = tokio::fs::read_to_string(fp).await?;
                return Ok(Some((info, content)));
            } else if let Some(wf) = workflow {
                return Ok(Some((info, wf)));
            }
            return Ok(Some((info, String::new())));
        }
        Ok(None)
    }

    /// Update action content - for bundled actions, creates a custom copy first
    pub async fn update_action_content(&self, name: &str, content: &str) -> Result<bool> {
        let actions = self.actions.read().await;
        if let Some(action) = actions.get(name) {
            // System actions cannot be edited
            if action.info.source == ActionSource::System {
                return Ok(false);
            }

            // For Bundled actions, create a custom copy in the data directory
            if action.info.source == ActionSource::Bundled {
                drop(actions); // Release lock before async file I/O

                // Create custom action directory
                let custom_action_dir = self.actions_dir.join(name);
                tokio::fs::create_dir_all(&custom_action_dir).await?;

                // Write content to custom location
                let custom_action_file = custom_action_dir.join("ACTION.md");
                tokio::fs::write(&custom_action_file, content).await?;

                // Re-sign the action manifest after edit
                if let Some(ref guard) = self.action_guard {
                    if let Err(e) = guard.resign_action(&custom_action_dir, name).await {
                        tracing::warn!("Failed to re-sign action '{}': {}", name, e);
                    }
                }

                tracing::info!(
                    "Created custom copy of bundled action '{}' at {:?}",
                    name,
                    custom_action_file
                );

                // Update the in-memory action to point to the new custom location
                let mut actions = self.actions.write().await;
                if let Some(action) = actions.get_mut(name) {
                    action.info.source = ActionSource::Custom;
                    action.info.file_path = Some(custom_action_file.to_string_lossy().to_string());
                    action.workflow_content = Some(content.to_string());
                }

                return Ok(true);
            }

            // Custom actions - edit in place and update in-memory
            if let Some(ref file_path) = action.info.file_path {
                let fp = file_path.clone();
                drop(actions); // Release lock before async file I/O
                tokio::fs::write(&fp, content).await?;

                // Re-sign the action manifest after edit
                let action_dir = std::path::Path::new(&fp)
                    .parent()
                    .unwrap_or(std::path::Path::new("."));
                if let Some(ref guard) = self.action_guard {
                    if let Err(e) = guard.resign_action(action_dir, name).await {
                        tracing::warn!("Failed to re-sign action '{}': {}", name, e);
                    }
                }

                // Re-parse and update in-memory action
                if let Ok((new_info, new_content, _frontmatter)) = self
                    .parse_action_md(std::path::Path::new(&fp), ActionSource::Custom)
                    .await
                {
                    let mut actions = self.actions.write().await;
                    if let Some(action) = actions.get_mut(name) {
                        action.info = new_info;
                        action.workflow_content = Some(new_content);
                    }
                }

                return Ok(true);
            }
        }
        Ok(false)
    }

    /// Create a new custom action with security verification
    /// Returns the security verdict so the caller can present it to the user.
    /// If `force` is true, the action is loaded even if security blocks it.
    pub async fn create_action(
        &self,
        name: &str,
        content: &str,
        force: bool,
    ) -> Result<Option<crate::security::action_guard::ActionSecurityVerdict>> {
        let action_dir = self.actions_dir.join(name);
        tokio::fs::create_dir_all(&action_dir).await?;

        let action_file = action_dir.join("ACTION.md");
        tokio::fs::write(&action_file, content).await?;

        // Sign the new action manifest
        if let Some(ref guard) = self.action_guard {
            if let Err(e) = guard.resign_action(&action_dir, name).await {
                tracing::warn!("Failed to sign new action '{}': {}", name, e);
            }
        }

        // Immediately register into runtime (no restart needed)
        match self
            .parse_action_md(&action_file, ActionSource::Custom)
            .await
        {
            Ok((info, workflow_content, frontmatter)) => {
                // Security evaluation for newly created actions
                let verdict = if let Some(ref guard) = self.action_guard {
                    match guard
                        .evaluate_action(&action_dir, name, &workflow_content, &frontmatter)
                        .await
                    {
                        Ok(v) => Some(v),
                        Err(e) => {
                            tracing::warn!(
                                "Security check failed for new action '{}': {} — loading anyway",
                                name,
                                e
                            );
                            None
                        }
                    }
                } else {
                    None
                };

                // Check if blocked
                let blocked = verdict.as_ref().map(|v| !v.allow_load).unwrap_or(false);

                if blocked && !force {
                    tracing::warn!(
                        "New action '{}' BLOCKED by security guard: {:?}",
                        name,
                        verdict.as_ref().map(|v| &v.warnings)
                    );
                    // Delete the saved files since we're not loading it
                    let _ = tokio::fs::remove_dir_all(&action_dir).await;
                    return Ok(verdict);
                }

                if let Some(ref v) = verdict {
                    for w in &v.warnings {
                        tracing::warn!("Action '{}': {}", name, w);
                    }
                    if blocked && force {
                        tracing::warn!("Action '{}' force-loaded despite security warnings", name);
                    }
                }

                self.register_workflow_action(info, workflow_content).await;
                tracing::info!(
                    "Created and registered action '{}' at {:?}",
                    name,
                    action_file
                );
                Ok(verdict)
            }
            Err(e) => {
                tracing::warn!("Created action file but failed to parse: {}", e);
                Err(anyhow::anyhow!("Failed to parse action: {}", e))
            }
        }
    }

    /// Preview security verdict for an action without persisting or registering it.
    pub async fn preview_action_security(
        &self,
        name: &str,
        content: &str,
    ) -> Result<Option<crate::security::action_guard::ActionSecurityVerdict>> {
        let preview_dir = self.actions_dir.join(format!(
            ".preview-{}-{}",
            name,
            uuid::Uuid::new_v4().simple()
        ));
        tokio::fs::create_dir_all(&preview_dir).await?;
        let action_file = preview_dir.join("ACTION.md");
        tokio::fs::write(&action_file, content).await?;

        let parsed = self
            .parse_action_md(&action_file, ActionSource::Custom)
            .await
            .map_err(|e| anyhow::anyhow!("Failed to parse action preview: {}", e));

        let verdict = match parsed {
            Ok((_info, workflow_content, frontmatter)) => {
                if let Some(ref guard) = self.action_guard {
                    match guard
                        .evaluate_action(&preview_dir, name, &workflow_content, &frontmatter)
                        .await
                    {
                        Ok(v) => Some(v),
                        Err(e) => {
                            tracing::warn!(
                                "Security check failed for action preview '{}': {}",
                                name,
                                e
                            );
                            None
                        }
                    }
                } else {
                    None
                }
            }
            Err(e) => {
                let _ = tokio::fs::remove_dir_all(&preview_dir).await;
                return Err(e);
            }
        };

        let _ = tokio::fs::remove_dir_all(&preview_dir).await;
        Ok(verdict)
    }

    /// Delete/disable an action.
    /// - Custom actions: deleted from disk and runtime.
    /// - Bundled actions: persisted as disabled and removed from runtime.
    /// - System actions: cannot be deleted/disabled.
    pub async fn delete_action(&self, name: &str) -> Result<bool> {
        let (source, file_path) = {
            let actions = self.actions.read().await;
            match actions.get(name) {
                Some(action) => (action.info.source.clone(), action.info.file_path.clone()),
                None => return Ok(false),
            }
        };

        match source {
            ActionSource::System => Ok(false),
            ActionSource::Bundled => {
                {
                    let mut disabled = self.disabled_actions.write().await;
                    disabled.insert(name.to_string());
                }
                self.save_disabled_actions().await?;
                tracing::info!("Disabled bundled action '{}'", name);
                Ok(true)
            }
            ActionSource::Custom => {
                if let Some(fp) = file_path {
                    let action_path = std::path::Path::new(&fp);
                    if let Some(action_dir) = action_path.parent() {
                        let dir_path = action_dir.to_path_buf();
                        if dir_path.exists() {
                            tokio::fs::remove_dir_all(&dir_path).await?;
                        }
                    }
                }
                let mut actions = self.actions.write().await;
                actions.remove(name);
                tracing::info!("Deleted custom action '{}'", name);
                Ok(true)
            }
        }
    }

    /// Check if an action is a workflow action (LLM-driven) and get its workflow content
    /// Returns None if action doesn't exist or has no workflow content
    pub async fn get_workflow_content(&self, action_name: &str) -> Option<String> {
        self.actions
            .read()
            .await
            .get(action_name)
            .and_then(|s| s.workflow_content.clone())
    }

    /// Execute a workflow action with LLM orchestration
    /// This performs web searches based on the workflow, then passes everything to the LLM
    pub async fn execute_workflow_action(
        &self,
        action_name: &str,
        workflow_content: &str,
        user_query: &str,
        llm: &crate::core::LlmClient,
    ) -> Result<String> {
        tracing::info!("Executing LLM-driven workflow action: {}", action_name);

        // Step 1: Extract search queries from the workflow
        let search_queries = self.extract_search_queries(workflow_content, action_name, user_query);

        // Step 2: Perform web searches
        let mut search_results = Vec::new();
        let search_config = build_search_config(&self.config_dir).await;

        for query in &search_queries {
            tracing::debug!("Searching: {}", query);
            let args = crate::actions::search::SearchArgs {
                query: query.clone(),
                num_results: 5,
                backend: None,
            };
            match crate::actions::search::execute_search(&args, &search_config).await {
                Ok(results) => {
                    search_results.push(format!("### Search: {}\n{}", query, results));
                }
                Err(e) => {
                    tracing::warn!("Search failed for '{}': {}", query, e);
                    search_results.push(format!("### Search: {} (failed: {})", query, e));
                }
            }
        }

        // Step 3: Build the LLM prompt with workflow instructions and search results
        let combined_results = search_results.join("\n\n");

        let system_prompt = format!(
            r#"You are executing an action workflow. Your task is to analyze the search results and produce output that EXACTLY follows the output format specified in the workflow.

## ACTION WORKFLOW INSTRUCTIONS
{}

## IMPORTANT RULES
1. Follow the "Output Format" section EXACTLY - use the same structure, headings, and formatting
2. Fill in all placeholder sections with actual content based on the search results
3. The LinkedIn post must be 800-1200 characters as specified
4. Include real data, trends, and insights from the search results
5. If search results are insufficient, note this but still produce the best output possible
6. Use today's date where [Date] is specified: {}

## SEARCH RESULTS TO ANALYZE
{}
"#,
            workflow_content,
            chrono::Utc::now().format("%Y-%m-%d"),
            combined_results
        );

        let user_prompt = format!(
            "Execute the workflow above. User's additional context/query: '{}'. Generate the complete output following the exact format specified in the workflow.",
            if user_query.is_empty() { "none" } else { user_query }
        );

        // Step 4: Call LLM to generate the formatted output
        let response = llm
            .chat(
                &system_prompt,
                &user_prompt,
                &[], // No memory entries needed
                &[], // No additional tools
            )
            .await?;

        Ok(response.content)
    }

    fn build_workflow_user_query(arguments: &serde_json::Value) -> String {
        arguments
            .get("query")
            .and_then(|v| v.as_str())
            .map(|s| s.to_string())
            .unwrap_or_else(|| {
                // Backward compatibility: if no explicit "query" exists, pass structured
                // arguments through as JSON so workflow actions can still consume fields.
                if let Some(obj) = arguments.as_object() {
                    if !obj.is_empty() {
                        return serde_json::to_string(arguments).unwrap_or_default();
                    }
                }
                String::new()
            })
    }

    fn collect_required_fields_from_schema(schema: &serde_json::Value) -> Vec<String> {
        schema
            .get("required")
            .and_then(|v| v.as_array())
            .map(|arr| {
                arr.iter()
                    .filter_map(|v| v.as_str())
                    .map(|s| s.trim().to_string())
                    .filter(|s| !s.is_empty())
                    .collect::<Vec<_>>()
            })
            .unwrap_or_default()
    }

    fn has_non_empty_argument(arguments: &serde_json::Value, key: &str) -> bool {
        let Some(value) = arguments.get(key) else {
            return false;
        };
        match value {
            serde_json::Value::Null => false,
            serde_json::Value::String(s) => !s.trim().is_empty(),
            serde_json::Value::Array(items) => !items.is_empty(),
            serde_json::Value::Object(map) => !map.is_empty(),
            _ => true,
        }
    }

    fn collect_provided_argument_keys(arguments: &serde_json::Value) -> Vec<String> {
        let Some(obj) = arguments.as_object() else {
            return Vec::new();
        };
        obj.iter()
            .filter(|(_, v)| match v {
                serde_json::Value::Null => false,
                serde_json::Value::String(s) => !s.trim().is_empty(),
                serde_json::Value::Array(items) => !items.is_empty(),
                serde_json::Value::Object(map) => !map.is_empty(),
                _ => true,
            })
            .map(|(k, _)| k.to_string())
            .collect()
    }

    fn build_workflow_missing_inputs_marker(payload: &WorkflowMissingInputsPayload) -> String {
        let json = serde_json::to_string(payload).unwrap_or_else(|_| {
            let fallback = serde_json::json!({
                "action": payload.action,
                "missing": payload.missing,
                "required": payload.required,
                "provided": payload.provided,
                "query": payload.query
            });
            fallback.to_string()
        });
        format!("{}{}", WORKFLOW_MISSING_INPUTS_MARKER, json)
    }

    fn dedupe_non_empty<I>(items: I) -> Vec<String>
    where
        I: IntoIterator<Item = String>,
    {
        let mut out = Vec::new();
        let mut seen = std::collections::HashSet::new();
        for item in items {
            let cleaned = item
                .split('#')
                .next()
                .unwrap_or("")
                .trim()
                .trim_matches('`')
                .trim_matches('"')
                .trim_matches('\'')
                .trim_end_matches(',')
                .to_string();
            if cleaned.is_empty() {
                continue;
            }
            let key = cleaned.to_ascii_lowercase();
            if seen.insert(key) {
                out.push(cleaned);
            }
        }
        out
    }

    fn parse_required_fields_from_frontmatter(frontmatter: &str) -> Vec<String> {
        let mut required = Vec::new();
        let lines: Vec<&str> = frontmatter.lines().collect();
        let mut i = 0usize;

        while i < lines.len() {
            let raw = lines[i];
            let line = raw.trim();
            let is_required_key = line.starts_with("required:")
                || line.starts_with("required_inputs:")
                || line.starts_with("requiredInputs:");
            if is_required_key {
                let rhs = line
                    .split_once(':')
                    .map(|(_, rhs)| rhs.trim())
                    .unwrap_or("");
                if rhs.starts_with('[') && rhs.ends_with(']') {
                    let inner = &rhs[1..rhs.len().saturating_sub(1)];
                    for part in inner.split(',') {
                        required.push(part.trim().trim_matches('"').trim_matches('\'').to_string());
                    }
                } else if !rhs.is_empty() {
                    required.push(rhs.trim_matches('"').trim_matches('\'').to_string());
                }

                let mut j = i + 1;
                while j < lines.len() {
                    let next_raw = lines[j];
                    let next_trim = next_raw.trim();
                    if next_trim.starts_with("- ") {
                        required.push(
                            next_trim
                                .trim_start_matches("- ")
                                .trim()
                                .trim_matches('"')
                                .trim_matches('\'')
                                .to_string(),
                        );
                        j += 1;
                        continue;
                    }
                    if next_raw.starts_with(' ')
                        || next_raw.starts_with('\t')
                        || next_trim.is_empty()
                    {
                        j += 1;
                        continue;
                    }
                    break;
                }
                i = j;
                continue;
            }
            i += 1;
        }

        Self::dedupe_non_empty(required)
    }

    fn parse_required_fields_from_workflow(workflow: &str) -> Vec<String> {
        let mut required = Vec::new();
        let mut in_required_section = false;

        for raw_line in workflow.lines() {
            let line = raw_line.trim();
            if line.is_empty() {
                continue;
            }

            if line.starts_with('#') {
                let heading = line.trim_start_matches('#').trim().to_ascii_lowercase();
                in_required_section = heading.contains("required input")
                    || heading == "required"
                    || heading == "input contract";
                continue;
            }

            if line.to_ascii_lowercase().starts_with("required inputs:") {
                in_required_section = true;
                continue;
            }

            if in_required_section
                && !line.starts_with("- ")
                && !line.starts_with("* ")
                && line.ends_with(':')
            {
                in_required_section = false;
                continue;
            }

            if !in_required_section {
                continue;
            }

            let candidate = if line.starts_with("- ") {
                line.trim_start_matches("- ").trim()
            } else if let Some(rest) = line.strip_prefix("* ") {
                rest.trim()
            } else {
                continue;
            };

            let mut field =
                if let (Some(start), Some(end)) = (candidate.find('`'), candidate.rfind('`')) {
                    if end > start {
                        candidate[start + 1..end].trim().to_string()
                    } else {
                        candidate.to_string()
                    }
                } else {
                    candidate.to_string()
                };

            if let Some((left, _)) = field.split_once(':') {
                field = left.trim().to_string();
            }
            field = field
                .trim_matches('{')
                .trim_matches('}')
                .trim_matches('[')
                .trim_matches(']')
                .trim()
                .to_string();

            if field
                .chars()
                .all(|c| c.is_ascii_alphanumeric() || c == '_' || c == '-')
            {
                required.push(field);
            }
        }

        Self::dedupe_non_empty(required)
    }

    fn build_workflow_input_schema(frontmatter: &str, workflow_content: &str) -> serde_json::Value {
        let mut required = Self::parse_required_fields_from_frontmatter(frontmatter);
        if required.is_empty() {
            required = Self::parse_required_fields_from_workflow(workflow_content);
        }
        let required = Self::dedupe_non_empty(required);

        let mut properties = serde_json::Map::new();
        properties.insert(
            "query".to_string(),
            serde_json::json!({
                "type": "string",
                "description": "Optional free-form input/context for the action"
            }),
        );

        for key in &required {
            if key.eq_ignore_ascii_case("query") {
                continue;
            }
            properties.insert(
                key.clone(),
                serde_json::json!({
                    "type": "string",
                    "description": format!("Required input: {}", key)
                }),
            );
        }

        serde_json::json!({
            "type": "object",
            "properties": properties,
            "required": required
        })
    }

    /// Extract search queries from workflow content based on action type
    fn extract_search_queries(
        &self,
        workflow: &str,
        action_name: &str,
        user_query: &str,
    ) -> Vec<String> {
        let mut queries = Vec::new();
        let year = chrono::Utc::now().format("%Y");
        let month = chrono::Utc::now().format("%B");

        // Look for search queries in the workflow (lines starting with - "...")
        for line in workflow.lines() {
            let line = line.trim();
            if line.starts_with("- \"") && line.ends_with("\"") {
                let query = line.trim_start_matches("- \"").trim_end_matches("\"");
                // Replace placeholders
                let query = query
                    .replace("2026", &year.to_string())
                    .replace("February", &month.to_string());
                queries.push(query.to_string());
            }
        }

        // If the workflow does not declare explicit queries, fall back to a
        // generic topic-based set rather than hardcoding skill names here.
        if queries.is_empty() {
            let topic = if user_query.trim().is_empty() {
                action_name.replace('-', " ")
            } else {
                user_query.trim().to_string()
            };
            queries.push(format!("{} latest news {}", topic, year));
            queries.push(format!("{} trends analysis {}", topic, year));
            if !user_query.trim().is_empty() {
                queries.push(format!("{} {}", user_query.trim(), year));
            }
        }

        queries
    }

    /// Load markdown-defined actions from a directory
    /// Looks for ACTION.md files in subdirectories
    /// These are registered as workflow actions for LLM-driven execution
    pub async fn load_markdown_actions(&self, dir: &Path, source: ActionSource) -> Result<()> {
        if !dir.exists() {
            return Ok(());
        }

        // Read directory entries
        let entries = match std::fs::read_dir(dir) {
            Ok(e) => e,
            Err(e) => {
                tracing::warn!("Could not read skills directory {:?}: {}", dir, e);
                return Ok(());
            }
        };

        for entry in entries.flatten() {
            let path = entry.path();
            if !path.is_dir() {
                continue;
            }

            let action_md = path.join("ACTION.md");
            if !action_md.exists() {
                continue;
            }
            let md_file = action_md;

            match self.parse_action_md(&md_file, source.clone()).await {
                Ok((info, workflow_content, frontmatter)) => {
                    if source == ActionSource::Bundled {
                        let disabled = self.disabled_actions.read().await;
                        if disabled.contains(&info.name) {
                            tracing::info!(
                                "Loaded bundled action '{}' as disabled from {:?}",
                                info.name,
                                md_file
                            );
                        }
                    }

                    // Run security evaluation if action guard is set
                    if let Some(ref guard) = self.action_guard {
                        match guard
                            .evaluate_action(&path, &info.name, &workflow_content, &frontmatter)
                            .await
                        {
                            Ok(verdict) => {
                                if !verdict.allow_load {
                                    tracing::warn!(
                                        "Action '{}' BLOCKED by security guard: {:?}",
                                        info.name,
                                        verdict.warnings
                                    );
                                    continue; // skip registration
                                }
                                for w in &verdict.warnings {
                                    tracing::warn!("Action '{}': {}", info.name, w);
                                }
                            }
                            Err(e) => {
                                tracing::warn!(
                                    "Security check failed for '{}': {} — loading anyway",
                                    info.name,
                                    e
                                );
                            }
                        }
                    }

                    tracing::info!("Loaded workflow action '{}' from {:?}", info.name, md_file);
                    self.register_workflow_action(info, workflow_content).await;
                }
                Err(e) => {
                    tracing::warn!("Failed to load action from {:?}: {}", md_file, e);
                }
            }
        }

        Ok(())
    }

    /// Parse an ACTION.md file to extract action information and full content
    /// Returns (ActionDef, full_workflow_content, frontmatter_text)
    async fn parse_action_md(
        &self,
        path: &Path,
        source: ActionSource,
    ) -> Result<(ActionDef, String, String)> {
        let content = tokio::fs::read_to_string(path).await?;

        // Parse YAML frontmatter (between --- markers)
        let mut name = String::new();
        let mut description = String::new();
        let mut version = "1.0.0".to_string();
        let mut frontmatter_text = String::new();

        if let Some(stripped) = content.strip_prefix("---") {
            if let Some(end_pos) = stripped.find("---") {
                let frontmatter = &stripped[..end_pos];
                frontmatter_text = frontmatter.to_string();
                for line in frontmatter.lines() {
                    let line = line.trim();
                    if let Some(val) = line.strip_prefix("name:") {
                        name = val.trim().trim_matches('"').to_string();
                    } else if let Some(val) = line.strip_prefix("description:") {
                        description = val.trim().trim_matches('"').to_string();
                    } else if let Some(val) = line.strip_prefix("version:") {
                        version = val.trim().trim_matches('"').to_string();
                    }
                }
            }
        }

        // Fallback: use directory name as action name
        if name.is_empty() {
            name = path
                .parent()
                .and_then(|p| p.file_name())
                .and_then(|n| n.to_str())
                .unwrap_or("unknown")
                .to_string();
        }

        // Extract first heading as description if not in frontmatter
        if description.is_empty() {
            for line in content.lines() {
                if let Some(stripped) = line.strip_prefix("# ") {
                    description = stripped.trim().to_string();
                    break;
                }
            }
        }
        if description.is_empty() {
            description = format!("Custom skill '{}'", name);
        }

        // Parse permissions from frontmatter
        let permissions = crate::security::ActionGuard::parse_permissions(&frontmatter_text);
        let mut capabilities: Vec<String> = permissions.iter().map(|p| p.to_string()).collect();
        if capabilities.is_empty() {
            capabilities.push("research".to_string());
        }

        let info = ActionDef {
            name,
            description,
            version,
            input_schema: Self::build_workflow_input_schema(&frontmatter_text, &content),
            capabilities,
            sandbox_mode: Some(SandboxMode::Native),
            source,
            file_path: Some(path.to_string_lossy().to_string()),
        };

        // Return the info, full content, and frontmatter for security evaluation
        Ok((info, content, frontmatter_text))
    }

    /// Execute a WASM module with given arguments
    async fn run_wasm_module(
        &self,
        wasm_bytes: &[u8],
        arguments: &serde_json::Value,
    ) -> Result<String> {
        use wasmtime::*;

        let engine = Engine::default();

        // Create a basic store without WASI for simple modules
        let mut store = Store::new(&engine, ());

        // Compile the module
        let module = Module::new(&engine, wasm_bytes)?;

        // Create a linker and instantiate
        let linker = Linker::new(&engine);
        let instance = linker.instantiate(&mut store, &module)?;

        // Try to find entry points
        let result = if let Ok(run_fn) = instance.get_typed_func::<(), ()>(&mut store, "_start") {
            run_fn.call(&mut store, ())?;
            format!(
                "WASM execution completed successfully. Args: {}",
                serde_json::to_string(arguments)?
            )
        } else if let Ok(run_fn) = instance.get_typed_func::<(), i32>(&mut store, "run") {
            let exit_code = run_fn.call(&mut store, ())?;
            format!("WASM execution completed with exit code: {}", exit_code)
        } else if let Ok(run_fn) = instance.get_typed_func::<i32, i32>(&mut store, "main") {
            let exit_code = run_fn.call(&mut store, 0)?;
            format!("WASM execution completed with exit code: {}", exit_code)
        } else {
            // List available exports for debugging
            let exports: Vec<String> = instance
                .exports(&mut store)
                .map(|e| e.name().to_string())
                .collect();
            return Err(anyhow::anyhow!(
                "WASM module has no _start, run, or main entry point. Available exports: {:?}",
                exports
            ));
        };

        Ok(result)
    }
}

/// Build search config: loads user settings from search.toml, then auto-detects Playwright.
/// Priority: user-configured backends from settings + Playwright auto-detection as default.
async fn build_search_config(config_dir: &Path) -> crate::actions::SearchConfig {
    // Load saved search config (from Settings UI)
    let mut config = match std::fs::read_to_string(config_dir.join("search.toml")) {
        Ok(content) => toml::from_str::<crate::actions::SearchConfig>(&content).unwrap_or_default(),
        Err(_) => crate::actions::SearchConfig::default(),
    };

    // Auto-detect Playwright bridge if not already set
    if config.playwright.is_none() {
        let bridge_url = std::env::var("PLAYWRIGHT_BRIDGE_URL")
            .unwrap_or_else(|_| "http://127.0.0.1:3100".to_string());

        let available = reqwest::Client::new()
            .get(format!("{}/health", bridge_url))
            .timeout(std::time::Duration::from_secs(1))
            .send()
            .await
            .map(|r| r.status().is_success())
            .unwrap_or(false);

        if available {
            tracing::debug!("Playwright bridge available at {}", bridge_url);
            config.playwright =
                Some(crate::actions::search::SearchBackend::Playwright { bridge_url });
        } else {
            tracing::debug!("Playwright bridge unavailable, falling back to DuckDuckGo");
        }
    }

    config
}
