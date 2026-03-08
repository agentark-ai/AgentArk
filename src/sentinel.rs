//! ArkSentinel — AgentArk's Background Guardian
//!
//! A unified background daemon that keeps AgentArk alive and proactive 24/7:
//!
//! - **Process watchdog**: Monitors tunnel + WhatsApp bridge, auto-restarts on crash
//! - **Task scheduler**: Fires cron tasks (daily brief, recurring jobs) at the right time
//! - **Watcher poller**: Evaluates watch conditions and triggers on match
//! - **Memory consolidation**: Periodically compresses episodic memory
//! - **Approval expiry**: Cleans up stale approval requests
//! - **ArkPulse**: Periodically wakes the agent to reflect and act proactively
//!
//! All loops run inside a single tokio task with staggered intervals.

use std::collections::{HashMap, HashSet};
use std::future::Future;
use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::Arc;
use std::time::{Duration, Instant, SystemTime};

use crate::channels;
use crate::core::{Agent, TaskStatus};
use once_cell::sync::Lazy;
use regex::Regex;
use serde::{Deserialize, Serialize};
use tokio::sync::RwLock;

type SharedAgent = Arc<RwLock<Agent>>;

const MAINTENANCE_DEFER_MINUTES: i64 = 10;
const MAINTENANCE_MAX_DEFERS: u32 = 3;
const PULSE_DEFER_MINUTES: i64 = 5;
const PULSE_MAX_DEFERS: u32 = 3;
static PULSE_RUNNING: AtomicBool = AtomicBool::new(false);

struct PulseRunGuard;

impl Drop for PulseRunGuard {
    fn drop(&mut self) {
        PULSE_RUNNING.store(false, Ordering::Release);
    }
}

pub fn is_pulse_running() -> bool {
    PULSE_RUNNING.load(Ordering::Relaxed)
}

async fn sentinel_under_load(agent: &SharedAgent) -> bool {
    let pending_tasks = {
        let agent_guard = agent.read().await;
        let tasks = agent_guard.tasks.read().await;
        tasks
            .all()
            .iter()
            .filter(|t| {
                matches!(
                    t.status,
                    TaskStatus::Pending | TaskStatus::AwaitingApproval | TaskStatus::InProgress
                )
            })
            .count()
    };

    let (watcher_count, browser_sessions) = {
        let agent_guard = agent.read().await;
        let watchers = agent_guard
            .watcher_manager
            .list()
            .await
            .into_iter()
            .filter(|w| matches!(w.status, crate::core::watcher::WatcherStatus::Active))
            .count();
        (watchers, agent_guard.browser_sessions.active_count())
    };

    let running_apps = {
        let agent_guard = agent.read().await;
        agent_guard
            .app_registry
            .list()
            .await
            .into_iter()
            .filter(|v| v.get("running").and_then(|x| x.as_bool()).unwrap_or(false))
            .count()
    };

    pending_tasks > 25 || watcher_count > 30 || browser_sessions >= 2 || running_apps > 12
}

async fn run_with_busy_deferral<F, Fut>(
    agent: &SharedAgent,
    label: &str,
    defer_minutes: i64,
    max_defers: u32,
    mut job: F,
) where
    F: FnMut() -> Fut,
    Fut: Future<Output = ()>,
{
    let mut defers = 0u32;
    loop {
        if !sentinel_under_load(agent).await {
            job().await;
            return;
        }

        if defers >= max_defers {
            tracing::info!(
                "ArkSentinel: {} skipped (busy after {} defers)",
                label,
                max_defers
            );
            return;
        }

        defers += 1;
        tracing::info!(
            "ArkSentinel: {} busy; deferring {}/{} for {} minutes",
            label,
            defers,
            max_defers,
            defer_minutes
        );
        tokio::time::sleep(Duration::from_secs((defer_minutes * 60) as u64)).await;
    }
}

/// A single ArkPulse event for the UI log
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct PulseEvent {
    pub timestamp: String,
    pub status: String, // "ok", "alert", "error"
    pub message: String,
    #[serde(default)]
    pub summary: String,
    #[serde(default)]
    pub flags: Vec<String>,
    pub overdue_tasks: usize,
    pub failed_tasks: usize,
    /// Detailed snapshot captured at pulse time
    #[serde(default)]
    pub details: PulseDetails,
}

/// Detailed system snapshot captured by each pulse check
#[derive(Debug, Clone, Default, Serialize, Deserialize)]
pub struct PulseDetails {
    pub pending_tasks: usize,
    pub running_tasks: usize,
    pub completed_tasks: usize,
    pub total_tasks: usize,
    pub active_watchers: usize,
    pub total_memories: usize,
    pub overdue_list: Vec<String>,
    pub failed_list: Vec<String>,
    pub uptime_secs: u64,
    /// System health checks
    #[serde(default)]
    pub health_checks: Vec<HealthCheck>,
    /// Security event snapshot since last pulse
    #[serde(default)]
    pub security: Option<crate::core::SecuritySnapshot>,
    /// Deployed apps health
    #[serde(default)]
    pub deployed_apps: Vec<AppPulseInfo>,
    /// Deterministic ArkPulse doctor findings
    #[serde(default)]
    pub doctor_findings: Vec<DoctorFinding>,
    /// 0..100 score where 100 means healthier posture
    #[serde(default)]
    pub doctor_score: u32,
}

#[derive(Debug, Clone, Default, Serialize, Deserialize)]
pub struct AppPulseInfo {
    pub id: String,
    pub title: String,
    pub is_static: bool,
    pub process_alive: bool,
    pub requests_since_last_check: u64,
    pub idle_hours: i64,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct HealthCheck {
    pub service: String,
    pub status: String, // "ok", "warn", "error"
    pub message: String,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(tag = "kind", rename_all = "snake_case")]
pub enum DoctorRemediationSpec {
    TunnelStartVerify,
    TunnelRestartVerify,
    ShellCommand { command: String },
}

#[derive(Debug, Clone, Default, Serialize, Deserialize)]
pub struct DoctorFinding {
    pub severity: String, // "critical" | "high" | "medium" | "low"
    pub category: String, // dependency, supply_chain, secrets, etc.
    pub target: String,   // file path, endpoint, app id, subsystem
    pub title: String,
    pub evidence: String,
    pub root_cause: String,
    pub fix_command: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub remediation: Option<DoctorRemediationSpec>,
    #[serde(default = "default_user_actionable_true")]
    pub user_actionable: bool,
}

fn default_user_actionable_true() -> bool {
    true
}

const PULSE_LOG_KEY: &str = "arkpulse_log";
const MAX_PULSE_EVENTS: usize = 100;
const MAX_PULSE_EVENT_AGE_DAYS: i64 = 30;
const ARKPULSE_LAST_RUN_AT_KEY: &str = "arkpulse_last_run_at";
const ARKPULSE_CRITICAL_LAST_SIG_KEY: &str = "arkpulse_critical_last_sig_v1";
const ARKPULSE_CRITICAL_LAST_SENT_KEY: &str = "arkpulse_critical_last_sent_v1";
const ARKPULSE_CRITICAL_NOTIFY_COOLDOWN_SECS: i64 = 24 * 3600;

fn normalize_arkpulse_alert_signature(text: &str) -> String {
    let mut out = String::with_capacity(text.len().min(240));
    let mut prev_space = false;
    for ch in text.trim().chars() {
        if ch.is_whitespace() {
            if !prev_space {
                out.push(' ');
                prev_space = true;
            }
            continue;
        }
        prev_space = false;
        out.push(ch.to_ascii_lowercase());
        if out.len() >= 220 {
            break;
        }
    }
    out.trim().to_string()
}

async fn should_emit_arkpulse_critical_notification(
    storage: &crate::storage::Storage,
    alert_text: &str,
) -> bool {
    let signature = normalize_arkpulse_alert_signature(alert_text);
    let now_ts = chrono::Utc::now().timestamp();
    let last_sent_ts = storage
        .get(ARKPULSE_CRITICAL_LAST_SENT_KEY)
        .await
        .ok()
        .flatten()
        .and_then(|raw| String::from_utf8(raw).ok())
        .and_then(|s| s.parse::<i64>().ok())
        .unwrap_or(0);

    let elapsed = if now_ts > last_sent_ts {
        now_ts - last_sent_ts
    } else {
        0
    };

    // Hard cap: emit at most one ArkPulse critical notification every 24h,
    // regardless of message/signature variance.
    if elapsed < ARKPULSE_CRITICAL_NOTIFY_COOLDOWN_SECS {
        return false;
    }

    if !signature.is_empty() {
        let _ = storage
            .set(ARKPULSE_CRITICAL_LAST_SIG_KEY, signature.as_bytes())
            .await;
    }
    let _ = storage
        .set(
            ARKPULSE_CRITICAL_LAST_SENT_KEY,
            now_ts.to_string().as_bytes(),
        )
        .await;
    true
}

fn control_plane_bases() -> (String, String) {
    let bind_addr = std::env::var("AGENTARK_BIND").unwrap_or_else(|_| "127.0.0.1:8990".to_string());
    let normalized = if bind_addr.starts_with("0.0.0.0:") {
        bind_addr.replacen("0.0.0.0", "127.0.0.1", 1)
    } else if bind_addr.starts_with("[::]:") {
        bind_addr.replacen("[::]", "127.0.0.1", 1)
    } else {
        bind_addr
    };
    (
        format!("http://{}", normalized),
        format!("ws://{}", normalized),
    )
}

/// Append a pulse event to the persistent log (capped at MAX_PULSE_EVENTS)
async fn log_pulse_event(agent: &Agent, event: PulseEvent) {
    let mut events: Vec<PulseEvent> = match agent.storage.get(PULSE_LOG_KEY).await {
        Ok(Some(data)) => serde_json::from_slice(&data).unwrap_or_default(),
        _ => Vec::new(),
    };
    events.push(event);
    events = prune_pulse_events(events);
    if let Ok(json) = serde_json::to_vec(&events) {
        let _ = agent.storage.set(PULSE_LOG_KEY, &json).await;
    }
}

/// Get the ArkPulse log from storage
pub async fn get_pulse_log(agent: &Agent) -> Vec<PulseEvent> {
    let raw: Vec<PulseEvent> = match agent.storage.get(PULSE_LOG_KEY).await {
        Ok(Some(data)) => serde_json::from_slice(&data).unwrap_or_default(),
        _ => Vec::new(),
    };
    let pruned = prune_pulse_events(raw.clone());
    if pruned.len() != raw.len() {
        if let Ok(json) = serde_json::to_vec(&pruned) {
            let _ = agent.storage.set(PULSE_LOG_KEY, &json).await;
        }
    }
    pruned
}

fn prune_pulse_events(mut events: Vec<PulseEvent>) -> Vec<PulseEvent> {
    let cutoff = chrono::Utc::now() - chrono::Duration::days(MAX_PULSE_EVENT_AGE_DAYS);
    events.retain(|event| {
        chrono::DateTime::parse_from_rfc3339(&event.timestamp)
            .map(|ts| ts.with_timezone(&chrono::Utc) >= cutoff)
            .unwrap_or(false)
    });
    if events.len() > MAX_PULSE_EVENTS {
        events.drain(0..events.len() - MAX_PULSE_EVENTS);
    }
    events
}

#[derive(Debug, Clone)]
struct AppEndpoint {
    id: String,
    title: String,
    is_static: bool,
    access_url: String,
    app_dir: PathBuf,
}

static RE_AWS_ACCESS_KEY: Lazy<Regex> =
    Lazy::new(|| Regex::new(r"\bAKIA[0-9A-Z]{16}\b").expect("valid regex"));
static RE_OPENAI_KEY: Lazy<Regex> =
    Lazy::new(|| Regex::new(r"\bsk-[A-Za-z0-9]{20,}\b").expect("valid regex"));
static RE_GITHUB_TOKEN: Lazy<Regex> =
    Lazy::new(|| Regex::new(r"\bgh[pousr]_[A-Za-z0-9]{20,}\b").expect("valid regex"));
static RE_PRIVATE_KEY: Lazy<Regex> = Lazy::new(|| {
    Regex::new(r"-----BEGIN (?:RSA|EC|OPENSSH|DSA|PRIVATE) PRIVATE KEY-----").expect("valid regex")
});
static RE_GENERIC_SECRET_ASSIGN: Lazy<Regex> = Lazy::new(|| {
    Regex::new(r#"(?i)\b(api[_-]?key|token|secret|password)\b\s*[:=]\s*["'][^"'\n]{12,}["']"#)
        .expect("valid regex")
});

fn severity_weight(severity: &str) -> u32 {
    match severity {
        "critical" => 12,
        "high" => 8,
        "medium" => 4,
        "low" => 1,
        _ => 2,
    }
}

fn compute_doctor_score(findings: &[DoctorFinding]) -> u32 {
    let penalty: u32 = findings
        .iter()
        .filter(|f| f.user_actionable)
        .map(|f| severity_weight(&f.severity))
        .sum();
    100u32.saturating_sub(penalty.min(100))
}

macro_rules! push_finding {
    ($findings:expr, $severity:expr, $category:expr, $target:expr, $title:expr, $evidence:expr, $root_cause:expr, $fix_command:expr, $remediation:expr $(,)?) => {{
        $findings.push(DoctorFinding {
            severity: ($severity).to_string(),
            category: ($category).to_string(),
            target: ($target).into(),
            title: ($title).into(),
            evidence: ($evidence).into(),
            root_cause: ($root_cause).into(),
            fix_command: ($fix_command).into(),
            remediation: Some($remediation),
            user_actionable: true,
        });
    }};
    ($findings:expr, $severity:expr, $category:expr, $target:expr, $title:expr, $evidence:expr, $root_cause:expr, $fix_command:expr $(,)?) => {{
        $findings.push(DoctorFinding {
            severity: ($severity).to_string(),
            category: ($category).to_string(),
            target: ($target).into(),
            title: ($title).into(),
            evidence: ($evidence).into(),
            root_cause: ($root_cause).into(),
            fix_command: ($fix_command).into(),
            remediation: None,
            user_actionable: true,
        });
    }};
}

macro_rules! push_internal_finding {
    ($findings:expr, $severity:expr, $category:expr, $target:expr, $title:expr, $evidence:expr, $root_cause:expr, $fix_command:expr, $remediation:expr $(,)?) => {{
        $findings.push(DoctorFinding {
            severity: ($severity).to_string(),
            category: ($category).to_string(),
            target: ($target).into(),
            title: ($title).into(),
            evidence: ($evidence).into(),
            root_cause: ($root_cause).into(),
            fix_command: ($fix_command).into(),
            remediation: Some($remediation),
            user_actionable: false,
        });
    }};
    ($findings:expr, $severity:expr, $category:expr, $target:expr, $title:expr, $evidence:expr, $root_cause:expr, $fix_command:expr $(,)?) => {{
        $findings.push(DoctorFinding {
            severity: ($severity).to_string(),
            category: ($category).to_string(),
            target: ($target).into(),
            title: ($title).into(),
            evidence: ($evidence).into(),
            root_cause: ($root_cause).into(),
            fix_command: ($fix_command).into(),
            remediation: None,
            user_actionable: false,
        });
    }};
}

fn parse_access_key(access_url: &str) -> Option<String> {
    let parsed = if access_url.starts_with("http://") || access_url.starts_with("https://") {
        url::Url::parse(access_url).ok()
    } else {
        url::Url::parse(&format!("http://local{}", access_url)).ok()
    }?;
    parsed.query_pairs().find_map(|(k, v)| {
        if k == "key" {
            Some(v.into_owned())
        } else {
            None
        }
    })
}

fn is_scan_text_file(path: &Path) -> bool {
    if let Some(name) = path.file_name().and_then(|n| n.to_str()) {
        if name.eq_ignore_ascii_case(".env") {
            return true;
        }
    }
    matches!(
        path.extension()
            .and_then(|e| e.to_str())
            .unwrap_or_default()
            .to_ascii_lowercase()
            .as_str(),
        "js" | "ts"
            | "jsx"
            | "tsx"
            | "py"
            | "rs"
            | "toml"
            | "json"
            | "yaml"
            | "yml"
            | "md"
            | "txt"
            | "sh"
            | "bash"
            | "zsh"
            | "html"
            | "css"
            | "ini"
            | "env"
    )
}

fn read_text_limited(path: &Path, max_bytes: usize) -> Option<String> {
    let bytes = std::fs::read(path).ok()?;
    let clipped = if bytes.len() > max_bytes {
        &bytes[..max_bytes]
    } else {
        &bytes
    };
    Some(String::from_utf8_lossy(clipped).to_string())
}

fn parse_app_endpoints(raw: &[serde_json::Value], data_dir: &Path) -> Vec<AppEndpoint> {
    raw.iter()
        .filter_map(|row| {
            let id = row.get("id")?.as_str()?.to_string();
            let access_url = row
                .get("access_url")
                .and_then(|v| v.as_str())
                .unwrap_or_default()
                .to_string();
            Some(AppEndpoint {
                title: row
                    .get("title")
                    .and_then(|v| v.as_str())
                    .unwrap_or(&id)
                    .to_string(),
                is_static: row
                    .get("is_static")
                    .and_then(|v| v.as_bool())
                    .unwrap_or(true),
                app_dir: data_dir.join("apps").join(&id),
                id,
                access_url,
            })
        })
        .collect()
}

fn detect_ws_hint(app_dir: &Path) -> bool {
    let package_json = app_dir.join("package.json");
    if let Some(text) = read_text_limited(&package_json, 256 * 1024) {
        if text.contains(r#""ws""#) || text.contains(r#""socket.io""#) {
            return true;
        }
    }
    let mut scanned = 0usize;
    for entry in walkdir::WalkDir::new(app_dir)
        .into_iter()
        .filter_map(|e| e.ok())
        .filter(|e| e.file_type().is_file())
    {
        scanned += 1;
        if scanned > 40 {
            break;
        }
        let path = entry.path();
        if !is_scan_text_file(path) {
            continue;
        }
        if let Some(text) = read_text_limited(path, 64 * 1024) {
            if text.contains("WebSocket")
                || text.contains("websocket")
                || text.contains("socket.io")
                || text.contains("/ws")
            {
                return true;
            }
        }
    }
    false
}

fn run_dependency_and_supply_checks_for_app(app: &AppEndpoint, findings: &mut Vec<DoctorFinding>) {
    let package_json = app.app_dir.join("package.json");
    let npm_lock_exists = app.app_dir.join("package-lock.json").exists()
        || app.app_dir.join("pnpm-lock.yaml").exists()
        || app.app_dir.join("yarn.lock").exists();
    if package_json.exists() {
        if !npm_lock_exists {
            push_finding!(
                findings,
                "high",
                "supply_chain",
                format!("app:{}", app.id),
                "Node lockfile missing",
                format!("{} has package.json but no lockfile", app.app_dir.display()),
                "Dependency tree can drift across installs, increasing supply-chain risk.",
                format!(
                    "cd {} && npm install --package-lock-only",
                    app.app_dir.display()
                ),
            );
        }
        if let (Ok(pkg_meta), Ok(lock_meta)) = (
            std::fs::metadata(&package_json),
            std::fs::metadata(app.app_dir.join("package-lock.json")),
        ) {
            if let (Ok(pkg_m), Ok(lock_m)) = (pkg_meta.modified(), lock_meta.modified()) {
                if pkg_m > lock_m {
                    push_finding!(
                        findings,
                        "medium",
                        "supply_chain",
                        format!("app:{}", app.id),
                        "Lockfile drift detected",
                        "package.json was modified after package-lock.json".to_string(),
                        "Manifest/lock mismatch can install unexpected versions.",
                        format!(
                            "cd {} && npm install --package-lock-only",
                            app.app_dir.display()
                        ),
                    );
                }
            }
        }
        if let Some(text) = read_text_limited(&package_json, 512 * 1024) {
            if let Ok(v) = serde_json::from_str::<serde_json::Value>(&text) {
                let mut risky_specs = Vec::new();
                for key in ["dependencies", "devDependencies"] {
                    if let Some(map) = v.get(key).and_then(|d| d.as_object()) {
                        for (name, spec_v) in map {
                            let spec = spec_v.as_str().unwrap_or_default();
                            if spec.contains("git+")
                                || spec.contains("github:")
                                || spec.starts_with("http://")
                                || spec.starts_with("https://")
                                || spec == "*"
                                || spec.eq_ignore_ascii_case("latest")
                            {
                                risky_specs.push(format!("{}={}", name, spec));
                            }
                        }
                    }
                }
                if !risky_specs.is_empty() {
                    push_finding!(
                        findings,
                        "high",
                        "dependency",
                        format!("app:{}", app.id),
                        "Unpinned/risky Node dependency spec",
                        risky_specs
                            .into_iter()
                            .take(8)
                            .collect::<Vec<_>>()
                            .join(", "),
                        "Git/URL/latest dependencies reduce reproducibility and trust guarantees.",
                        format!(
                            "cd {} && npm pkg set dependencies.<name>=<pinned-version>",
                            app.app_dir.display()
                        ),
                    );
                }
                if let Some(scripts) = v.get("scripts").and_then(|s| s.as_object()) {
                    let mut suspicious = Vec::new();
                    for key in ["preinstall", "install", "postinstall"] {
                        if let Some(cmd) = scripts.get(key).and_then(|v| v.as_str()) {
                            let lower = cmd.to_lowercase();
                            if lower.contains("curl ")
                                || lower.contains("wget ")
                                || lower.contains("powershell")
                                || lower.contains("invoke-webrequest")
                                || lower.contains("bash -c")
                                || lower.contains("sh -c")
                            {
                                suspicious.push(format!("{}: {}", key, cmd));
                            }
                        }
                    }
                    if !suspicious.is_empty() {
                        let fix_command = format!(
                            "cd {} && npm pkg delete scripts.preinstall scripts.install scripts.postinstall",
                            app.app_dir.display()
                        );
                        push_finding!(
                            findings,
                            "critical",
                            "supply_chain",
                            format!("app:{}", app.id),
                            "Suspicious install script detected",
                            suspicious.join(" | "),
                            "Install hooks can execute arbitrary code during deployment.",
                            fix_command.clone(),
                            DoctorRemediationSpec::ShellCommand {
                                command: fix_command,
                            },
                        );
                    }
                }
            }
        }
    }

    let cargo_toml = app.app_dir.join("Cargo.toml");
    if cargo_toml.exists() {
        if !app.app_dir.join("Cargo.lock").exists() {
            let fix_command = format!("cd {} && cargo generate-lockfile", app.app_dir.display());
            push_finding!(
                findings,
                "high",
                "dependency",
                format!("app:{}", app.id),
                "Cargo lockfile missing",
                format!("{} has Cargo.toml but no Cargo.lock", app.app_dir.display()),
                "Rust dependency set is not pinned for reproducible builds.",
                fix_command.clone(),
                DoctorRemediationSpec::ShellCommand {
                    command: fix_command,
                },
            );
        }
        if let Some(text) = read_text_limited(&cargo_toml, 512 * 1024) {
            if text.contains("git =") || text.contains("path =") {
                let fix_command = format!(
                    "cd {} && rg -n \"git\\s*=|path\\s*=\" Cargo.toml",
                    app.app_dir.display()
                );
                push_finding!(
                    findings,
                    "medium",
                    "supply_chain",
                    format!("app:{}", app.id),
                    "Git/path Rust dependency detected",
                    "Cargo.toml contains git/path dependency sources".to_string(),
                    "Non-registry dependency sources increase trust and drift risk.",
                    fix_command.clone(),
                    DoctorRemediationSpec::ShellCommand {
                        command: fix_command,
                    },
                );
            }
        }
    }

    let requirements = app.app_dir.join("requirements.txt");
    if requirements.exists() {
        if let Some(text) = read_text_limited(&requirements, 512 * 1024) {
            let mut unpinned = Vec::new();
            let mut remote_specs = Vec::new();
            for line in text
                .lines()
                .map(|l| l.trim())
                .filter(|l| !l.is_empty() && !l.starts_with('#'))
            {
                if line.contains("git+")
                    || line.starts_with("http://")
                    || line.starts_with("https://")
                {
                    remote_specs.push(line.to_string());
                } else if !(line.contains("==") || line.contains("@")) {
                    unpinned.push(line.to_string());
                }
            }
            if !unpinned.is_empty() {
                let fix_command = format!(
                    "cd {} && pip-compile requirements.txt",
                    app.app_dir.display()
                );
                push_finding!(
                    findings,
                    "medium",
                    "dependency",
                    format!("app:{}", app.id),
                    "Unpinned Python dependency",
                    unpinned.into_iter().take(8).collect::<Vec<_>>().join(", "),
                    "Floating versions can introduce breaking or vulnerable transitive updates.",
                    fix_command.clone(),
                    DoctorRemediationSpec::ShellCommand {
                        command: fix_command,
                    },
                );
            }
            if !remote_specs.is_empty() {
                let fix_command = format!(
                    "cd {} && rg -n \"git\\+|https?://\" requirements.txt",
                    app.app_dir.display()
                );
                push_finding!(
                    findings,
                    "high",
                    "supply_chain",
                    format!("app:{}", app.id),
                    "Remote Python dependency source",
                    remote_specs
                        .into_iter()
                        .take(6)
                        .collect::<Vec<_>>()
                        .join(", "),
                    "Direct remote package sources bypass curated index trust controls.",
                    fix_command.clone(),
                    DoctorRemediationSpec::ShellCommand {
                        command: fix_command,
                    },
                );
            }
        }
    }
}

fn run_secret_scan_for_app(app: &AppEndpoint, findings: &mut Vec<DoctorFinding>) {
    if app.app_dir.join(".env").exists() {
        push_finding!(
            findings,
            "high",
            "secrets",
            format!("app:{}", app.id),
            ".env file present in deployed app",
            format!("Found {}", app.app_dir.join(".env").display()),
            "Environment files in deployed app directories are easy to leak by misconfiguration.",
            format!(
                "cd {} && mv .env ../.env.backup && rotate exposed keys",
                app.app_dir.display()
            ),
        );
    }

    let mut scanned_files = 0usize;
    let mut hit_count = 0usize;
    for entry in walkdir::WalkDir::new(&app.app_dir)
        .into_iter()
        .filter_map(|e| e.ok())
        .filter(|e| e.file_type().is_file())
    {
        if scanned_files >= 200 || hit_count >= 8 {
            break;
        }
        let path = entry.path();
        if !is_scan_text_file(path) {
            continue;
        }
        scanned_files += 1;
        let Some(text) = read_text_limited(path, 256 * 1024) else {
            continue;
        };
        let rel = path
            .strip_prefix(&app.app_dir)
            .ok()
            .and_then(|p| p.to_str())
            .unwrap_or("unknown");
        let mut matched = None::<&str>;
        if RE_PRIVATE_KEY.is_match(&text) {
            matched = Some("private key material");
        } else if RE_AWS_ACCESS_KEY.is_match(&text) {
            matched = Some("AWS access key pattern");
        } else if RE_OPENAI_KEY.is_match(&text) {
            matched = Some("OpenAI key pattern");
        } else if RE_GITHUB_TOKEN.is_match(&text) {
            matched = Some("GitHub token pattern");
        } else if RE_GENERIC_SECRET_ASSIGN.is_match(&text) {
            matched = Some("generic secret assignment");
        }
        if let Some(kind) = matched {
            hit_count += 1;
            let fix_command = format!(
                "cd {} && rg -n \"(api[_-]?key|token|secret|BEGIN .*PRIVATE KEY)\" {}",
                app.app_dir.display(),
                rel
            );
            push_finding!(
                findings,
                "critical",
                "secrets",
                format!("app:{}:{}", app.id, rel),
                "Potential secret exposure",
                format!("Matched {} in {}", kind, rel),
                "Sensitive credentials may be hardcoded or stored in deploy artifact.",
                fix_command.clone(),
                DoctorRemediationSpec::ShellCommand {
                    command: fix_command,
                },
            );
        }
    }
}

async fn run_attack_surface_checks(
    http_client: &reqwest::Client,
    http_base: &str,
    has_deployed_apps: bool,
    public_base_url: Option<&str>,
    api_key: Option<&str>,
    findings: &mut Vec<DoctorFinding>,
) {
    let health = http_client
        .get(format!("{}/health", http_base))
        .send()
        .await;
    match health {
        Ok(resp) => {
            if !resp.status().is_success() {
                push_finding!(
                    findings,
                    "medium",
                    "attack_surface",
                    "/health",
                    "Core health endpoint degraded",
                    format!("GET /health returned {}", resp.status()),
                    "Control plane health endpoint is unhealthy.",
                    "Check server logs and restart service".to_string(),
                );
            }
        }
        Err(e) => {
            push_finding!(
                findings,
                "medium",
                "attack_surface",
                http_base,
                "Control plane probe failed",
                e.to_string(),
                "Local HTTP control plane unreachable for safety probes.",
                "Verify AgentArk HTTP server is running on port 8990".to_string(),
            );
            return;
        }
    }

    let protected_checks = vec![
        ("GET", "/api/apps"),
        ("GET", "/tunnel/status"),
        ("POST", "/tunnel/start"),
        ("GET", "/settings"),
    ];
    for (method, path) in protected_checks {
        let req = match method {
            "POST" => http_client.post(format!("{}{}", http_base, path)),
            _ => http_client.get(format!("{}{}", http_base, path)),
        };
        match req.send().await {
            Ok(resp) => {
                let code = resp.status().as_u16();
                if code != 401 && code != 403 {
                    push_finding!(
                        findings,
                        "critical",
                        "attack_surface",
                        path,
                        "Protected endpoint accessible without auth",
                        format!("{} {} returned {}", method, path, code),
                        "Management endpoint did not enforce authentication.",
                        format!(
                            "Move {} under auth middleware and add regression test",
                            path
                        ),
                    );
                }
            }
            Err(e) => {
                push_finding!(
                    findings,
                    "low",
                    "attack_surface",
                    path,
                    "Auth probe request failed",
                    e.to_string(),
                    "Could not verify endpoint authentication behavior.",
                    "Retry probe when control plane is stable".to_string(),
                );
            }
        }
    }

    if has_deployed_apps {
        let public_base = public_base_url
            .map(|v| v.trim().trim_end_matches('/').to_string())
            .filter(|v| !v.is_empty())
            .unwrap_or_else(|| {
                push_finding!(
                    findings,
                    "high",
                    "attack_surface",
                    "public_base_url",
                    "Public tunnel URL missing for deployed apps",
                    "No active public tunnel base URL found while apps are deployed".to_string(),
                    "Apps are not publicly reachable through the managed Cloudflare tunnel.",
                    "Start tunnel and verify /tunnel/status returns active + URL".to_string(),
                    DoctorRemediationSpec::TunnelStartVerify,
                );
                String::new()
            });
        if !public_base.is_empty() {
            let public_health_url = format!("{}/health", public_base);
            match http_client.get(&public_health_url).send().await {
                Ok(resp) if resp.status().is_success() => {}
                Ok(resp) => {
                    push_finding!(
                        findings,
                        "high",
                        "attack_surface",
                        public_health_url.clone(),
                        "Public tunnel health probe failed",
                        format!("GET {} returned {}", public_health_url, resp.status()),
                        "Tunnel endpoint is reachable but unhealthy for public traffic.",
                        "Restart tunnel and inspect cloudflared logs".to_string(),
                        DoctorRemediationSpec::TunnelRestartVerify,
                    );
                }
                Err(e) => {
                    push_finding!(
                        findings,
                        "high",
                        "attack_surface",
                        public_health_url.clone(),
                        "Public tunnel unreachable",
                        e.to_string(),
                        "Cannot reach service through the configured public tunnel URL.",
                        "Restart tunnel and verify DNS/TLS connectivity".to_string(),
                        DoctorRemediationSpec::TunnelRestartVerify,
                    );
                }
            }
        }

        let mut tunnel_status_req = http_client.get(format!("{}/tunnel/status", http_base));
        if let Some(key) = api_key.filter(|k| !k.trim().is_empty()) {
            tunnel_status_req = tunnel_status_req.bearer_auth(key);
        }
        match tunnel_status_req.send().await {
            Ok(resp) if resp.status().is_success() => {
                if let Ok(payload) = resp.json::<serde_json::Value>().await {
                    let active = payload
                        .get("active")
                        .and_then(|v| v.as_bool())
                        .unwrap_or(false);
                    let url_present = payload
                        .get("url")
                        .and_then(|v| v.as_str())
                        .map(|v| !v.trim().is_empty())
                        .unwrap_or(false);
                    if !active || !url_present {
                        push_finding!(
                            findings,
                            "high",
                            "attack_surface",
                            "/tunnel/status",
                            "Tunnel status degraded while apps are deployed",
                            format!(
                                "active={}, url_present={}",
                                active,
                                url_present
                            ),
                            "Managed tunnel should stay active while deployed apps need public access.",
                            "Restart the tunnel and confirm URL discovery".to_string(),
                            DoctorRemediationSpec::TunnelRestartVerify,
                        );
                    }
                }
            }
            Ok(resp) => {
                push_finding!(
                    findings,
                    "medium",
                    "attack_surface",
                    "/tunnel/status",
                    "Tunnel status probe failed",
                    format!("Authenticated status probe returned {}", resp.status()),
                    "ArkPulse could not verify managed tunnel state from control plane.",
                    "Verify API auth and tunnel control endpoints".to_string(),
                );
            }
            Err(e) => {
                push_finding!(
                    findings,
                    "medium",
                    "attack_surface",
                    "/tunnel/status",
                    "Tunnel status probe request failed",
                    e.to_string(),
                    "ArkPulse could not verify tunnel process state.",
                    "Check local control plane reachability".to_string(),
                );
            }
        }

        if !public_base.is_empty() {
            let tunnel_apps_probe = format!("{}/api/apps", public_base);
            match http_client.get(&tunnel_apps_probe).send().await {
                Ok(resp) => {
                    let code = resp.status().as_u16();
                    if code != 401 && code != 403 {
                        push_finding!(
                            findings,
                            "critical",
                            "attack_surface",
                            tunnel_apps_probe,
                            "Public tunnel exposed protected app inventory endpoint",
                            format!("GET /api/apps over tunnel returned {}", code),
                            "Sensitive management endpoint is reachable from public tunnel without auth.",
                            "Require auth middleware for tunneled management routes".to_string(),
                        );
                    }
                }
                Err(e) => {
                    push_finding!(
                        findings,
                        "low",
                        "attack_surface",
                        "/api/apps",
                        "Public tunnel app-inventory auth probe failed",
                        e.to_string(),
                        "Could not verify auth enforcement of /api/apps over public tunnel.",
                        "Retry when tunnel stabilizes".to_string(),
                    );
                }
            }
        }
    }
}

async fn run_runtime_hardening_checks(
    http_client: &reqwest::Client,
    http_base: &str,
    app_endpoints: &[AppEndpoint],
    findings: &mut Vec<DoctorFinding>,
) {
    if let Ok(resp) = http_client.get(format!("{}/", http_base)).send().await {
        let headers = resp.headers();
        let required = [
            "content-security-policy",
            "x-content-type-options",
            "x-frame-options",
            "referrer-policy",
        ];
        let missing: Vec<&str> = required
            .iter()
            .copied()
            .filter(|h| !headers.contains_key(*h))
            .collect();
        if !missing.is_empty() {
            push_internal_finding!(
                findings,
                "medium",
                "runtime_hardening",
                "/",
                "Missing security response headers",
                format!("Missing headers: {}", missing.join(", ")),
                "HTTP responses miss baseline browser hardening controls.",
                "Add headers via tower-http middleware in HTTP router".to_string(),
            );
        }

        let cookie_headers: Vec<String> = headers
            .get_all("set-cookie")
            .iter()
            .filter_map(|h| h.to_str().ok().map(|s| s.to_string()))
            .collect();
        if let Some(session_cookie) = cookie_headers
            .iter()
            .find(|c| c.to_ascii_lowercase().contains("agentark_session="))
        {
            let lower = session_cookie.to_ascii_lowercase();
            if !lower.contains("httponly") || !lower.contains("samesite") {
                push_finding!(
                    findings,
                    "high",
                    "runtime_hardening",
                    "agentark_session cookie",
                    "Session cookie missing hardening flags",
                    session_cookie.clone(),
                    "Session cookie can be exposed to script or cross-site abuse.",
                    "Set HttpOnly and SameSite on session cookie generation".to_string(),
                );
            }
        }
    }

    // Path traversal probes for static app file serving.
    // Probe all static app endpoints (not just the first) so newly introduced
    // deployment variants are continuously covered.
    let traversal_payloads = [
        "..%2F..%2FCargo.toml",
        "%2e%2e%2f%2e%2e%2fCargo.toml",
        "..%2F..%2F..%2F..%2Fetc%2Fhosts",
        "..%252F..%252FCargo.toml",
    ];
    for app in app_endpoints.iter().filter(|a| a.is_static) {
        let Some(key) = parse_access_key(&app.access_url) else {
            continue;
        };
        for payload in traversal_payloads {
            let traversal_url = format!("{}/apps/{}/{}?key={}", http_base, app.id, payload, key);
            if let Ok(resp) = http_client.get(&traversal_url).send().await {
                let status = resp.status();
                if status.is_success() {
                    let body = resp.text().await.unwrap_or_default();
                    if !body.to_lowercase().contains("access key required") {
                        push_finding!(
                            findings,
                            "critical",
                            "runtime_hardening",
                            traversal_url.clone(),
                            "Path traversal regression risk",
                            format!("Static app traversal payload returned {}", status.as_u16()),
                            "Static app file serving may be bypassing root path constraints.",
                            "Keep canonical-path prefix checks and deny traversal sequences"
                                .to_string(),
                        );
                        break;
                    }
                }
            }
        }
    }

    // Additional traversal probes for other file-serving surfaces.
    let generic_probes = [
        ("/api/uploads/..%2F..%2FCargo.toml", "upload file serving"),
        (
            "/api/outputs/00000000-0000-0000-0000-000000000000/..%2F..%2FCargo.toml",
            "output file serving",
        ),
        (
            "/api/outputs/not-a-uuid/Cargo.toml",
            "output file serving UUID validation",
        ),
    ];
    for (path, surface) in generic_probes {
        let url = format!("{}{}", http_base, path);
        if let Ok(resp) = http_client.get(&url).send().await {
            if resp.status().is_success() {
                push_finding!(
                    findings,
                    "critical",
                    "runtime_hardening",
                    url,
                    "Path traversal/validation bypass risk",
                    format!(
                        "{} probe unexpectedly returned success ({})",
                        surface,
                        resp.status().as_u16()
                    ),
                    "A file-serving surface accepted a traversal/invalid-path probe.",
                    "Harden path validation and keep canonical root checks on all file endpoints"
                        .to_string(),
                );
            }
        }
    }
}

fn run_resource_checks(
    data_dir: &Path,
    deployed_apps: &[AppPulseInfo],
    security: Option<&crate::core::SecuritySnapshot>,
    security_thresholds: ArkPulseSecurityThresholds,
    findings: &mut Vec<DoctorFinding>,
) {
    let db_path = data_dir.join("agentark.db");
    if let Ok(meta) = std::fs::metadata(&db_path) {
        let size_mb = meta.len() as f64 / (1024.0 * 1024.0);
        if size_mb > 1024.0 {
            push_finding!(
                findings,
                "high",
                "resource",
                db_path.display().to_string(),
                "Database size is very large",
                format!("{:.1} MB", size_mb),
                "Database growth can degrade query performance and backup times.",
                "Archive old rows and run VACUUM during maintenance window".to_string(),
            );
        } else if size_mb > 512.0 {
            push_finding!(
                findings,
                "medium",
                "resource",
                db_path.display().to_string(),
                "Database size growth warning",
                format!("{:.1} MB", size_mb),
                "Storage growth trend may indicate missing retention policies.",
                "Review retention windows for traces, logs, and notifications".to_string(),
            );
        }
    }

    for app in deployed_apps {
        if app.requests_since_last_check > 5000 {
            push_finding!(
                findings,
                "high",
                "resource",
                format!("app:{}", app.id),
                "Request flood anomaly",
                format!(
                    "{} requests in last pulse window",
                    app.requests_since_last_check
                ),
                "Traffic spike can indicate abuse or runaway client retries.",
                format!(
                    "Throttle /apps/{} via rate limits and inspect access logs",
                    app.id
                ),
            );
        } else if app.requests_since_last_check > 1200 {
            push_finding!(
                findings,
                "medium",
                "resource",
                format!("app:{}", app.id),
                "High request volume",
                format!(
                    "{} requests in last pulse window",
                    app.requests_since_last_check
                ),
                "Sustained traffic may require autoscaling and caching.",
                format!("Profile app {} and add caching/backpressure", app.id),
            );
        }
    }

    if let Some(sec) = security {
        if sec.auth_failures >= security_thresholds.auth_failures {
            push_finding!(
                findings,
                "high",
                "resource",
                "auth subsystem",
                "Auth-failure burst detected",
                format!(
                    "{} auth failures since previous pulse (threshold {})",
                    sec.auth_failures, security_thresholds.auth_failures
                ),
                "May indicate credential stuffing or stale automation credentials.",
                "Rotate API keys and tighten IP/rate limits".to_string(),
            );
        }
        if sec.injection_attempts > 0 {
            push_finding!(
                findings,
                "high",
                "resource",
                "prompt security",
                "Prompt-injection attempts detected",
                format!("{} attempts", sec.injection_attempts),
                "Active probing of prompt surface was observed.",
                "Review security logs and block offending sources".to_string(),
            );
        }
    }
}

async fn run_data_safety_checks(agent: &Agent, data_dir: &Path, findings: &mut Vec<DoctorFinding>) {
    let backup_dir = data_dir.join("backups");
    if !backup_dir.exists() {
        push_internal_finding!(
            findings,
            "medium",
            "data_safety",
            backup_dir.display().to_string(),
            "No backup directory found",
            "Expected backup directory does not exist".to_string(),
            "Data recovery posture is weak without regular backups.",
            "mkdir -p data/backups && configure periodic backups".to_string(),
        );
    } else {
        let mut latest: Option<SystemTime> = None;
        if let Ok(entries) = std::fs::read_dir(&backup_dir) {
            for entry in entries.flatten() {
                if let Ok(meta) = entry.metadata() {
                    if let Ok(m) = meta.modified() {
                        latest = Some(latest.map_or(m, |x| x.max(m)));
                    }
                }
            }
        }
        if let Some(ts) = latest {
            if let Ok(age) = ts.elapsed() {
                if age > Duration::from_secs(7 * 24 * 3600) {
                    push_finding!(
                        findings,
                        "high",
                        "data_safety",
                        backup_dir.display().to_string(),
                        "Backups are stale",
                        format!("Latest backup age: {:.1} days", age.as_secs_f64() / 86400.0),
                        "Recovery point objective is likely not met.",
                        "Run backup now and schedule daily backups".to_string(),
                    );
                }
            }
        } else {
            push_finding!(
                findings,
                "high",
                "data_safety",
                backup_dir.display().to_string(),
                "Backup directory is empty",
                "No backup artifacts found".to_string(),
                "No restore point is available if DB corruption occurs.",
                "Create first full backup of data/agentark.db".to_string(),
            );
        }
    }

    match agent.storage.sqlite_quick_check().await {
        Ok(result) => {
            if result.to_lowercase() != "ok" {
                push_finding!(
                    findings,
                    "critical",
                    "data_safety",
                    "sqlite",
                    "SQLite integrity check failed",
                    format!("PRAGMA quick_check => {}", result),
                    "Database may be corrupted or partially inconsistent.",
                    "Stop writes, restore from backup, and run PRAGMA integrity_check".to_string(),
                );
            }
        }
        Err(e) => {
            push_finding!(
                findings,
                "high",
                "data_safety",
                "sqlite",
                "SQLite integrity check unavailable",
                e.to_string(),
                "Could not validate database integrity.",
                "Investigate DB connectivity and permissions".to_string(),
            );
        }
    }

    let required_tables = [
        "kv_store",
        "tasks",
        "security_logs",
        "notifications",
        "messages",
        "conversations",
        "approval_log",
    ];
    match agent.storage.sqlite_table_names().await {
        Ok(tables) => {
            let table_set: HashSet<String> = tables.into_iter().collect();
            let missing: Vec<&str> = required_tables
                .iter()
                .copied()
                .filter(|t| !table_set.contains(*t))
                .collect();
            if !missing.is_empty() {
                push_finding!(
                    findings,
                    "critical",
                    "data_safety",
                    "sqlite schema",
                    "Schema drift or failed migration",
                    format!("Missing tables: {}", missing.join(", ")),
                    "Expected storage schema is incomplete.",
                    "Run migration/bootstrap table creation and restore missing schema".to_string(),
                );
            }
        }
        Err(e) => {
            push_finding!(
                findings,
                "medium",
                "data_safety",
                "sqlite schema",
                "Could not verify schema",
                e.to_string(),
                "Schema validation query failed.",
                "Check SQLite permissions and migration code path".to_string(),
            );
        }
    }
}

fn run_policy_compliance_checks(findings: &mut Vec<DoctorFinding>) {
    let agent_path = Path::new("src").join("core").join("agent.rs");
    let task_router_path = Path::new("src").join("core").join("task_router.rs");
    let parallel_path = Path::new("src").join("core").join("parallel.rs");

    if let Ok(agent_src) = std::fs::read_to_string(&agent_path) {
        if !has_bounded_app_validation_retry(&agent_src) {
            push_finding!(
                findings,
                "high",
                "policy",
                agent_path.display().to_string(),
                "Missing bounded retry cap for app validation",
                "App validation flow missing bounded-loop structure".to_string(),
                "App repair/validation loop may become unbounded.",
                "Use a finite attempt loop with explicit stop + terminal failure return"
                    .to_string(),
            );
        }
        if !has_bounded_self_heal_retry(&agent_src) {
            push_finding!(
                findings,
                "high",
                "policy",
                agent_path.display().to_string(),
                "Missing bounded retry cap for self-heal",
                "Self-heal flow missing bounded retry counter semantics".to_string(),
                "Code self-heal execution may loop too long without cap.",
                "Enforce incrementing retry counter + explicit cap check + hard stop".to_string(),
            );
        }
        if !has_tool_call_dedupe_guard(&agent_src) {
            push_finding!(
                findings,
                "medium",
                "policy",
                agent_path.display().to_string(),
                "Tool-call dedupe guard not detected",
                "No behavioral dedupe guard for duplicate tool calls".to_string(),
                "Merged responses may execute same side-effecting tool multiple times.",
                "Add signature-based dedupe set before tool execution".to_string(),
            );
        }
    }

    if let Ok(router_src) = std::fs::read_to_string(&task_router_path) {
        if !has_retry_cap_prompt_policy(&router_src) {
            push_finding!(
                findings,
                "medium",
                "policy",
                task_router_path.display().to_string(),
                "Retry-cap instruction missing in synthesis policy",
                "No robust bounded-retry policy language detected".to_string(),
                "Generated repair loops might not carry explicit stop conditions.",
                "Add bounded-retry rule to synthesis/router prompts".to_string(),
            );
        }
    }

    if let Ok(parallel_src) = std::fs::read_to_string(&parallel_path) {
        if !has_parallel_app_deploy_recovery(&parallel_src) {
            push_finding!(
                findings,
                "low",
                "policy",
                parallel_path.display().to_string(),
                "App tool recovery missing in parallel aggregator",
                "No behavior-based app_deploy recovery path detected".to_string(),
                "Parallel aggregation may drop deploy tool calls.",
                "Recover app_deploy tool call from successful paths".to_string(),
            );
        }
    }
}

fn extract_block_by_signature<'a>(src: &'a str, signature: &str) -> Option<&'a str> {
    let start = src.find(signature)?;
    let after_start = &src[start..];
    let rel_open = after_start.find('{')?;
    let open_idx = start + rel_open;
    let mut depth = 0usize;
    for (i, ch) in src[open_idx..].char_indices() {
        match ch {
            '{' => depth += 1,
            '}' => {
                depth = depth.saturating_sub(1);
                if depth == 0 {
                    return Some(&src[open_idx + 1..open_idx + i]);
                }
            }
            _ => {}
        }
    }
    None
}

fn has_bounded_app_validation_retry(agent_src: &str) -> bool {
    let Some(body) =
        extract_block_by_signature(agent_src, "async fn validate_and_capture_app_preview")
    else {
        return false;
    };
    let has_finite_loop = Regex::new(r"for\s+\w+\s+in\s+1\.\.=")
        .ok()
        .map(|re| re.is_match(body))
        .unwrap_or(false);
    let has_terminal_failure_return =
        body.contains("return Ok((None, false") && body.contains("last_error");
    let has_attempt_feedback =
        body.contains("attempt {}/") || body.contains("Validated on attempt");
    has_finite_loop && has_terminal_failure_return && has_attempt_feedback
}

fn has_bounded_self_heal_retry(agent_src: &str) -> bool {
    let Some(body) = extract_block_by_signature(agent_src, "if call.name == \"code_execute\"")
    else {
        return false;
    };
    let has_counter = body.contains("total_retries");
    let has_increment = Regex::new(r"total_retries\s*\+=\s*1")
        .ok()
        .map(|re| re.is_match(body))
        .unwrap_or(false);
    let has_cap_check = Regex::new(r"total_retries\s*>=\s*([A-Z_][A-Z0-9_]*|\d+)")
        .ok()
        .map(|re| re.is_match(body))
        .unwrap_or(false);
    let has_hard_stop =
        Regex::new(r"(?s)if\s+total_retries\s*>=\s*([A-Z_][A-Z0-9_]*|\d+)\s*\{.*?break;")
            .ok()
            .map(|re| re.is_match(body))
            .unwrap_or(false)
            || body.contains("maximum attempts reached");
    has_counter && has_increment && has_cap_check && has_hard_stop
}

fn has_tool_call_dedupe_guard(agent_src: &str) -> bool {
    let Some(body) = extract_block_by_signature(agent_src, "async fn execute_tool_calls") else {
        return false;
    };
    body.contains("response.tool_calls")
        && body.contains("HashSet")
        && body.contains("seen_signatures.insert")
        && body.contains("unique_calls.push")
        && body.contains("for call in unique_calls")
}

fn has_retry_cap_prompt_policy(router_src: &str) -> bool {
    let has_retry_language = Regex::new(r"(?i)\b(retry|repair)\b")
        .ok()
        .map(|re| re.is_match(router_src))
        .unwrap_or(false);
    let has_cap_language = Regex::new(
        r"(?i)\b(max(?:imum)?\s+attempts?|bounded\s+retr(?:y|ies)|stop\s+when\s+reached)\b",
    )
    .ok()
    .map(|re| re.is_match(router_src))
    .unwrap_or(false);
    has_retry_language && has_cap_language
}

fn has_parallel_app_deploy_recovery(parallel_src: &str) -> bool {
    parallel_src.contains("has_action_intent_default")
        && parallel_src.contains("app_deploy")
        && parallel_src.contains("tool_calls.is_empty")
        && parallel_src.contains("final_response.tool_calls.push")
}

async fn run_app_health_checks(
    http_client: &reqwest::Client,
    http_base: &str,
    ws_base: &str,
    app_endpoints: &[AppEndpoint],
    deployed_apps: &[AppPulseInfo],
    findings: &mut Vec<DoctorFinding>,
) {
    let app_state: HashMap<String, &AppPulseInfo> =
        deployed_apps.iter().map(|a| (a.id.clone(), a)).collect();

    for app in app_endpoints {
        if let Some(snapshot) = app_state.get(&app.id) {
            if !snapshot.process_alive && !snapshot.is_static {
                push_finding!(
                    findings,
                    "critical",
                    "app_health",
                    format!("app:{}", app.id),
                    "Dynamic app process is down",
                    format!("{} ({}) is not running", app.title, app.id),
                    "Runtime process exited or crashed.",
                    format!("POST /api/apps/{}/restart", app.id),
                );
            }
        }

        let root_url = format!("{}{}", http_base, app.access_url);
        let started = Instant::now();
        match tokio::time::timeout(Duration::from_secs(5), http_client.get(&root_url).send()).await
        {
            Ok(Ok(resp)) => {
                let elapsed_ms = started.elapsed().as_millis();
                if resp.status().as_u16() >= 500 {
                    push_finding!(
                        findings,
                        "high",
                        "app_health",
                        root_url.clone(),
                        "App root probe failed",
                        format!("HTTP {} in {} ms", resp.status(), elapsed_ms),
                        "App endpoint returns server-side errors.",
                        format!("POST /api/apps/{}/restart", app.id),
                    );
                } else if elapsed_ms > 2500 {
                    push_finding!(
                        findings,
                        "medium",
                        "app_health",
                        root_url.clone(),
                        "High app latency",
                        format!("{} ms", elapsed_ms),
                        "App response latency exceeded healthy threshold.",
                        format!("Inspect app {} runtime logs and optimize hot path", app.id),
                    );
                }
            }
            Ok(Err(e)) => {
                push_finding!(
                    findings,
                    "high",
                    "app_health",
                    root_url.clone(),
                    "App probe connection failure",
                    e.to_string(),
                    "App endpoint is unreachable from control plane.",
                    format!("POST /api/apps/{}/restart", app.id),
                );
            }
            Err(_) => {
                push_finding!(
                    findings,
                    "high",
                    "app_health",
                    root_url.clone(),
                    "App probe timeout",
                    "Timed out after 5s".to_string(),
                    "App endpoint is unresponsive.",
                    format!("POST /api/apps/{}/restart", app.id),
                );
            }
        }

        if let Some(key) = parse_access_key(&app.access_url) {
            let health_url = format!("{}/apps/{}/health?key={}", http_base, app.id, key);
            if let Ok(Ok(resp)) =
                tokio::time::timeout(Duration::from_secs(4), http_client.get(&health_url).send())
                    .await
            {
                if resp.status().as_u16() >= 500 {
                    push_finding!(
                        findings,
                        "high",
                        "app_health",
                        health_url,
                        "App /health endpoint failing",
                        format!("HTTP {}", resp.status()),
                        "Health endpoint reports degraded runtime state.",
                        format!("POST /api/apps/{}/restart", app.id),
                    );
                }
            }

            if !app.is_static && detect_ws_hint(&app.app_dir) {
                let ws_url = format!("{}/apps/{}/ws?key={}", ws_base, app.id, key);
                match tokio::time::timeout(
                    Duration::from_secs(4),
                    tokio_tungstenite::connect_async(&ws_url),
                )
                .await
                {
                    Ok(Ok((mut stream, _))) => {
                        let _ = futures::SinkExt::close(&mut stream).await;
                    }
                    Ok(Err(e)) => {
                        push_finding!(
                            findings,
                            "medium",
                            "app_health",
                            ws_url,
                            "WebSocket proxy validation failed",
                            e.to_string(),
                            "WS endpoint appears expected by app but handshake failed.",
                            "Verify /apps/{id}/ws proxy path and upstream WS server route"
                                .to_string(),
                        );
                    }
                    Err(_) => {
                        push_finding!(
                            findings,
                            "medium",
                            "app_health",
                            ws_url,
                            "WebSocket proxy probe timed out",
                            "Handshake timed out after 4s".to_string(),
                            "WS endpoint is slow or blocked.",
                            "Inspect WS server startup and reverse-proxy upgrade path".to_string(),
                        );
                    }
                }
            }
        }
    }
}

async fn run_doctor_checks(
    agent: &Agent,
    http_client: &reqwest::Client,
    deployed_apps: &[AppPulseInfo],
    security: Option<&crate::core::SecuritySnapshot>,
    security_thresholds: ArkPulseSecurityThresholds,
) -> Vec<DoctorFinding> {
    let mut findings: Vec<DoctorFinding> = Vec::new();
    let data_dir = agent.data_dir().to_path_buf();
    let app_rows = agent.app_registry.list().await;
    let app_endpoints = parse_app_endpoints(&app_rows, &data_dir);
    let (http_base, ws_base) = control_plane_bases();
    let has_deployed_apps = !deployed_apps.is_empty();
    let public_base_url = agent
        .storage
        .get("public_base_url")
        .await
        .ok()
        .flatten()
        .and_then(|raw| String::from_utf8(raw).ok())
        .map(|s| s.trim().trim_end_matches('/').to_string())
        .filter(|s| !s.is_empty());
    let api_key = agent.api_key.as_deref();

    run_attack_surface_checks(
        http_client,
        &http_base,
        has_deployed_apps,
        public_base_url.as_deref(),
        api_key,
        &mut findings,
    )
    .await;
    run_runtime_hardening_checks(http_client, &http_base, &app_endpoints, &mut findings).await;
    run_resource_checks(
        &data_dir,
        deployed_apps,
        security,
        security_thresholds,
        &mut findings,
    );
    run_data_safety_checks(agent, &data_dir, &mut findings).await;
    run_policy_compliance_checks(&mut findings);

    for app in &app_endpoints {
        run_dependency_and_supply_checks_for_app(app, &mut findings);
        run_secret_scan_for_app(app, &mut findings);
    }
    run_app_health_checks(
        http_client,
        &http_base,
        &ws_base,
        &app_endpoints,
        deployed_apps,
        &mut findings,
    )
    .await;

    findings.sort_by(|a, b| severity_weight(&b.severity).cmp(&severity_weight(&a.severity)));
    if findings.len() > 40 {
        findings.truncate(40);
    }
    findings
}

/// ArkSentinel configuration (loaded from settings, with sensible defaults)
pub struct SentinelConfig {
    /// How often to check process health (seconds) — used by http.rs process watchdog
    pub _process_check_interval: u64,
    /// How often to check for due tasks (seconds)
    pub scheduler_interval: u64,
    /// How often to poll watchers (seconds)
    pub watcher_interval: u64,
    /// How often to run memory consolidation (seconds)
    pub consolidation_interval: u64,
    /// How often to expire old approvals (seconds)
    pub approval_expiry_interval: u64,
    /// How often to run ArkPulse (seconds, 0 = disabled)
    pub pulse_interval: u64,
    /// How often to check if Mem0 cleanup should run (seconds).
    /// Actual cleanup only runs once per month when the server is idle.
    pub mem0_cleanup_check_interval: u64,
    /// How often to check for unused deployed apps (seconds).
    /// Notifications sent once per day per unused app.
    pub unused_app_check_interval: u64,
    /// How often to run proactive autonomy analysis scans (seconds).
    pub auto_analysis_interval: u64,
}

impl Default for SentinelConfig {
    fn default() -> Self {
        Self {
            _process_check_interval: 30,
            scheduler_interval: 30,
            watcher_interval: 15,
            consolidation_interval: 600,
            approval_expiry_interval: 300,
            pulse_interval: 1800,              // 30 minutes
            mem0_cleanup_check_interval: 3600, // Check hourly, but only run monthly when idle
            unused_app_check_interval: 3600,   // Check hourly, notify once daily per unused app
            auto_analysis_interval: 900,       // 15 minutes
        }
    }
}

/// Start all ArkSentinel background loops. Returns join handles for graceful shutdown.
pub fn start(agent: SharedAgent, config: SentinelConfig) -> Vec<tokio::task::JoinHandle<()>> {
    let mut handles = Vec::new();

    // ── Task Scheduler ──────────────────────────────────────────────────
    handles.push({
        let agent = agent.clone();
        tokio::spawn(async move {
            let mut interval =
                tokio::time::interval(std::time::Duration::from_secs(config.scheduler_interval));
            loop {
                interval.tick().await;
                if is_agent_autonomy_paused(&agent).await {
                    continue;
                }
                run_scheduler(&agent).await;
            }
        })
    });

    // ── Watcher Poller ──────────────────────────────────────────────────
    handles.push({
        let agent = agent.clone();
        tokio::spawn(async move {
            let mut interval =
                tokio::time::interval(std::time::Duration::from_secs(config.watcher_interval));
            loop {
                interval.tick().await;
                if is_agent_autonomy_paused(&agent).await {
                    continue;
                }
                run_watchers(&agent).await;
            }
        })
    });

    // ── Memory Consolidation ────────────────────────────────────────────
    handles.push({
        let agent = agent.clone();
        tokio::spawn(async move {
            let mut interval = tokio::time::interval(std::time::Duration::from_secs(
                config.consolidation_interval,
            ));
            interval.tick().await; // Skip first immediate tick
            loop {
                interval.tick().await;
                run_with_busy_deferral(
                    &agent,
                    "consolidation",
                    MAINTENANCE_DEFER_MINUTES,
                    MAINTENANCE_MAX_DEFERS,
                    || {
                        let agent = agent.clone();
                        async move { run_consolidation(&agent).await }
                    },
                )
                .await;
            }
        })
    });

    // ── Approval Expiry ─────────────────────────────────────────────────
    handles.push({
        let agent = agent.clone();
        tokio::spawn(async move {
            let mut interval = tokio::time::interval(std::time::Duration::from_secs(
                config.approval_expiry_interval,
            ));
            interval.tick().await; // Skip first immediate tick
            loop {
                interval.tick().await;
                run_approval_expiry(&agent).await;
            }
        })
    });

    // ── ArkPulse (proactive agent wake-up) ───────────────────────────────
    if config.pulse_interval > 0 {
        handles.push({
            let agent = agent.clone();
            tokio::spawn(async move {
                // Wait for initial startup to settle
                tokio::time::sleep(std::time::Duration::from_secs(60)).await;
                let mut interval =
                    tokio::time::interval(std::time::Duration::from_secs(config.pulse_interval));
                interval.tick().await; // Skip first tick (we already waited)
                loop {
                    interval.tick().await;
                    if is_agent_autonomy_paused(&agent).await {
                        continue;
                    }
                    run_with_busy_deferral(
                        &agent,
                        "arkpulse",
                        PULSE_DEFER_MINUTES,
                        PULSE_MAX_DEFERS,
                        || {
                            let agent = agent.clone();
                            async move { run_pulse(&agent).await }
                        },
                    )
                    .await;
                }
            })
        });
    }

    // ── Autonomy Auto-Analysis (periodic insight generation) ─────────────
    if config.auto_analysis_interval > 0 {
        handles.push({
            let agent = agent.clone();
            tokio::spawn(async move {
                tokio::time::sleep(std::time::Duration::from_secs(45)).await;
                let mut interval = tokio::time::interval(std::time::Duration::from_secs(
                    config.auto_analysis_interval,
                ));
                interval.tick().await;
                loop {
                    interval.tick().await;
                    if is_agent_autonomy_paused(&agent).await {
                        continue;
                    }
                    run_with_busy_deferral(
                        &agent,
                        "auto_analysis",
                        MAINTENANCE_DEFER_MINUTES,
                        MAINTENANCE_MAX_DEFERS,
                        || {
                            let agent = agent.clone();
                            async move {
                                let _ = channels::http::run_autonomy_analysis_tick(
                                    agent.clone(),
                                    "sentinel_periodic",
                                )
                                .await;
                            }
                        },
                    )
                    .await;
                }
            })
        });
    }

    // ── Mem0 Memory Decay Cleanup (monthly, idle-only) ─────────────────
    handles.push({
        let agent = agent.clone();
        tokio::spawn(async move {
            // Wait for startup to settle
            tokio::time::sleep(std::time::Duration::from_secs(300)).await;
            let mut interval = tokio::time::interval(std::time::Duration::from_secs(
                config.mem0_cleanup_check_interval,
            ));
            interval.tick().await; // Skip first tick
            loop {
                interval.tick().await;
                run_with_busy_deferral(
                    &agent,
                    "mem0_cleanup",
                    MAINTENANCE_DEFER_MINUTES,
                    MAINTENANCE_MAX_DEFERS,
                    || {
                        let agent = agent.clone();
                        async move { run_mem0_cleanup(&agent).await }
                    },
                )
                .await;
            }
        })
    });

    // Mem0 retry queue drain (frequent, lightweight)
    handles.push({
        let agent = agent.clone();
        tokio::spawn(async move {
            tokio::time::sleep(std::time::Duration::from_secs(30)).await;
            let mut interval = tokio::time::interval(std::time::Duration::from_secs(45));
            interval.tick().await;
            loop {
                interval.tick().await;
                run_mem0_retry_drain(&agent).await;
            }
        })
    });

    // ── Unused App Notifications ────────────────────────────────────────
    // Episodic memory retention cleanup (safe-by-default, idle-only, bounded).
    handles.push({
        let agent = agent.clone();
        tokio::spawn(async move {
            // Wait for startup to settle.
            tokio::time::sleep(std::time::Duration::from_secs(600)).await;
            // Check a few times a day; function is internally rate-limited (days).
            let mut interval = tokio::time::interval(std::time::Duration::from_secs(6 * 3600));
            interval.tick().await;
            loop {
                interval.tick().await;
                run_with_busy_deferral(
                    &agent,
                    "episode_retention_cleanup",
                    MAINTENANCE_DEFER_MINUTES,
                    MAINTENANCE_MAX_DEFERS,
                    || {
                        let agent = agent.clone();
                        async move { run_episode_retention_cleanup(&agent).await }
                    },
                )
                .await;
            }
        })
    });

    handles.push({
        let agent = agent.clone();
        tokio::spawn(async move {
            // Wait for startup to settle
            tokio::time::sleep(std::time::Duration::from_secs(120)).await;
            let mut interval = tokio::time::interval(std::time::Duration::from_secs(
                config.unused_app_check_interval,
            ));
            interval.tick().await; // Skip first tick
            loop {
                interval.tick().await;
                if is_agent_autonomy_paused(&agent).await {
                    continue;
                }
                run_with_busy_deferral(
                    &agent,
                    "unused_app_check",
                    MAINTENANCE_DEFER_MINUTES,
                    MAINTENANCE_MAX_DEFERS,
                    || {
                        let agent = agent.clone();
                        async move { run_unused_app_check(&agent).await }
                    },
                )
                .await;
            }
        })
    });

    // ── Security Log Cleanup (every 15 days, idle-only) ─────────────────
    handles.push({
        let agent = agent.clone();
        tokio::spawn(async move {
            // Check every 6 hours, but only actually cleanup every 15 days when idle
            tokio::time::sleep(std::time::Duration::from_secs(600)).await;
            let mut interval = tokio::time::interval(std::time::Duration::from_secs(6 * 3600));
            interval.tick().await;
            loop {
                interval.tick().await;
                run_with_busy_deferral(
                    &agent,
                    "security_log_cleanup",
                    MAINTENANCE_DEFER_MINUTES,
                    MAINTENANCE_MAX_DEFERS,
                    || {
                        let agent = agent.clone();
                        async move { run_security_log_cleanup(&agent).await }
                    },
                )
                .await;
            }
        })
    });

    tracing::info!(
        "ArkSentinel started: scheduler={}s, watchers={}s, consolidation={}s, pulse={}s, auto_analysis={}s, mem0_cleanup=monthly",
        config.scheduler_interval,
        config.watcher_interval,
        config.consolidation_interval,
        if config.pulse_interval > 0 {
            config.pulse_interval.to_string()
        } else {
            "off".to_string()
        },
        if config.auto_analysis_interval > 0 {
            config.auto_analysis_interval.to_string()
        } else {
            "off".to_string()
        },
    );

    handles
}

// ═══════════════════════════════════════════════════════════════════════════
// Task Scheduler — execute cron/scheduled tasks when due
// ═══════════════════════════════════════════════════════════════════════════

async fn run_scheduler(agent: &SharedAgent) {
    if is_agent_autonomy_paused(agent).await {
        tracing::debug!("ArkSentinel: scheduler skipped (agent paused)");
        return;
    }

    let due_tasks = {
        let agent = agent.read().await;
        agent.take_due_tasks().await
    };

    if !due_tasks.is_empty() {
        tracing::info!("ArkSentinel: {} scheduled task(s) due", due_tasks.len());
    }

    for task in due_tasks {
        tracing::info!(
            "ArkSentinel: executing task '{}' (action={})",
            task.description,
            task.action
        );
        let task_start = std::time::Instant::now();

        let result = {
            let agent = agent.read().await;
            agent.execute_task(&task).await
        };

        let task_elapsed = task_start.elapsed();
        let (status, output) = match result {
            Ok(out) => {
                tracing::info!(
                    "ArkSentinel: task '{}' completed ({}ms, output={}chars)",
                    task.description,
                    task_elapsed.as_millis(),
                    out.len()
                );
                (TaskStatus::Completed, Some(out))
            }
            Err(e) => {
                tracing::error!(
                    "ArkSentinel: task '{}' failed ({}ms): {}",
                    task.description,
                    task_elapsed.as_millis(),
                    e
                );
                (
                    TaskStatus::Failed {
                        error: e.to_string(),
                    },
                    Some(format!("Error: {}", e)),
                )
            }
        };

        let agent_guard = agent.read().await;
        let _ = agent_guard
            .finalize_task(task.id, status, output.clone())
            .await;

        // Push result to the configured channel (generic dispatch)
        let report_to = task
            .arguments
            .get("report_to")
            .and_then(|v| v.as_str())
            .unwrap_or("");

        if let Some(ref text) = output {
            if !report_to.is_empty() {
                tracing::info!("ArkSentinel: sending task result to channel={}", report_to);
                agent_guard.try_send_notification(report_to, text).await;
            } else if task.action == "daily_brief" {
                agent_guard.notify_preferred_channel(text).await;
            }
        }
    }
}

// ═══════════════════════════════════════════════════════════════════════════
// Watcher Poller — check conditions and fire triggers
// ═══════════════════════════════════════════════════════════════════════════

async fn run_watchers(agent: &SharedAgent) {
    if is_agent_autonomy_paused(agent).await {
        tracing::debug!("ArkSentinel: watchers skipped (agent paused)");
        return;
    }

    // Expire timed-out watchers
    let expired = {
        let agent = agent.read().await;
        agent.watcher_manager.expire_watchers().await
    };
    for w in &expired {
        let agent = agent.read().await;
        let msg = format!(
            "Watcher timed out: **{}**\n\nPolled `{}` {} times over {} minutes without finding a match.",
            w.description, w.poll_action, w.poll_count, w.timeout_secs / 60
        );
        agent
            .emit_notification("Watcher Timed Out", &msg, "warning", "watcher")
            .await;
        if !w.notify_channel.is_empty() {
            agent.try_send_notification(&w.notify_channel, &msg).await;
        } else {
            agent.notify_preferred_channel(&msg).await;
        }
    }

    // Poll due watchers
    let due_watchers = {
        let agent = agent.read().await;
        agent.watcher_manager.get_due_watchers().await
    };

    for watcher in due_watchers {
        let poll_result = {
            let agent = agent.read().await;
            agent
                .runtime
                .execute_action(&watcher.poll_action, &watcher.poll_arguments)
                .await
        };

        let new_count = watcher.poll_count + 1;
        {
            let agent = agent.read().await;
            agent
                .watcher_manager
                .update_poll(watcher.id, new_count)
                .await;
        }

        match poll_result {
            Ok(result) => {
                let matched = watcher.condition.evaluate(&result);
                tracing::info!(
                    "Watcher '{}' poll #{}: action={}, result_len={}, condition_matched={}",
                    watcher.description,
                    new_count,
                    watcher.poll_action,
                    result.len(),
                    matched
                );
                if matched {
                    {
                        let agent = agent.read().await;
                        agent
                            .watcher_manager
                            .mark_triggered(watcher.id, result.clone())
                            .await;
                    }

                    let trigger_prompt = format!(
                        "[WATCHER TRIGGERED] {}\n\nPoll result:\n{}\n\nInstructions: {}",
                        watcher.description, result, watcher.on_trigger
                    );

                    let response = {
                        let agent = agent.read().await;
                        agent
                            .process_message(&trigger_prompt, "watcher", None, None)
                            .await
                    };

                    let notify_text = match response {
                        Ok(resp) => format!("**{}**\n\n{}", watcher.description, resp),
                        Err(e) => format!(
                            "Watcher triggered for **{}** but follow-up failed: {}\n\nRaw result:\n{}",
                            watcher.description, e, result
                        ),
                    };

                    let agent = agent.read().await;
                    agent
                        .emit_notification("Watcher Triggered", &notify_text, "info", "watcher")
                        .await;
                    if !watcher.notify_channel.is_empty() {
                        agent
                            .try_send_notification(&watcher.notify_channel, &notify_text)
                            .await;
                    } else {
                        agent.notify_preferred_channel(&notify_text).await;
                    }
                }
            }
            Err(e) => {
                tracing::debug!("ArkSentinel: watcher {} poll error: {}", watcher.id, e);
            }
        }
    }

    // Cleanup old watchers
    {
        let agent = agent.read().await;
        agent.watcher_manager.cleanup().await;
    }
}

// ═══════════════════════════════════════════════════════════════════════════
// Memory Consolidation
// ═══════════════════════════════════════════════════════════════════════════

async fn run_consolidation(agent: &SharedAgent) {
    let agent = agent.read().await;
    let llm = agent.llm.clone();
    match agent.memory.run_llm_consolidation(&llm).await {
        Ok(summary) => {
            if !summary.contains("No unconsolidated") {
                tracing::info!("ArkSentinel: auto-consolidation: {}", summary);
            }
        }
        Err(e) => tracing::debug!("ArkSentinel: consolidation skipped: {}", e),
    }
}

// ═══════════════════════════════════════════════════════════════════════════
// Approval Expiry
// ═══════════════════════════════════════════════════════════════════════════

async fn run_approval_expiry(agent: &SharedAgent) {
    let agent = agent.read().await;
    if let Err(e) = agent.storage.expire_old_approvals(3600).await {
        tracing::debug!("ArkSentinel: approval expiry check: {}", e);
    }
    agent.safety.clear_expired_approvals();
}

fn build_pulse_log_summary(
    overdue_tasks: usize,
    failed_tasks: usize,
    approaching_goals: usize,
    security: Option<&crate::core::SecuritySnapshot>,
    dead_apps: usize,
    doctor_high_findings: usize,
) -> String {
    let security_events = security
        .map(|s| {
            s.injection_attempts
                + s.auth_failures
                + s.rate_limit_hits
                + s.unauthorized_channel_attempts
        })
        .unwrap_or(0);

    let mut parts: Vec<String> = Vec::new();
    if overdue_tasks > 0 {
        parts.push(format!("{} overdue task(s)", overdue_tasks));
    }
    if failed_tasks > 0 {
        parts.push(format!("{} failed task(s)", failed_tasks));
    }
    if approaching_goals > 0 {
        parts.push(format!("{} goal deadline(s)", approaching_goals));
    }
    if security_events > 0 {
        parts.push(format!("{} security event(s)", security_events));
    }
    if dead_apps > 0 {
        parts.push(format!("{} app process issue(s)", dead_apps));
    }
    if doctor_high_findings > 0 {
        parts.push(format!(
            "{} high-risk doctor finding(s)",
            doctor_high_findings
        ));
    }

    if parts.is_empty() {
        "All clear, no issues detected".to_string()
    } else {
        format!("Alert: {}", parts.join(", "))
    }
}

#[derive(Debug, Clone, Copy)]
struct ArkPulseSecurityThresholds {
    auth_failures: u64,
    rate_limit_hits: u64,
    unauthorized_channel: u64,
    combined: u64,
}

impl Default for ArkPulseSecurityThresholds {
    fn default() -> Self {
        let defaults = crate::core::AutonomySettings::default();
        Self {
            auth_failures: defaults.arkpulse_auth_failures_threshold.max(1) as u64,
            rate_limit_hits: defaults.arkpulse_rate_limit_hits_threshold.max(1) as u64,
            unauthorized_channel: defaults.arkpulse_unauthorized_channel_threshold.max(1) as u64,
            combined: defaults.arkpulse_combined_security_threshold.max(1) as u64,
        }
    }
}

async fn load_autonomy_settings_snapshot(
    storage: &crate::storage::Storage,
) -> crate::core::AutonomySettings {
    if let Ok(Some(raw)) = storage.get("autonomy_settings_v1").await {
        if let Ok(parsed) = serde_json::from_slice::<crate::core::AutonomySettings>(&raw) {
            return parsed;
        }
    }
    crate::core::AutonomySettings::default()
}

async fn load_arkpulse_security_thresholds(
    storage: &crate::storage::Storage,
) -> ArkPulseSecurityThresholds {
    let parsed = load_autonomy_settings_snapshot(storage).await;
    ArkPulseSecurityThresholds {
        auth_failures: parsed.arkpulse_auth_failures_threshold.max(1) as u64,
        rate_limit_hits: parsed.arkpulse_rate_limit_hits_threshold.max(1) as u64,
        unauthorized_channel: parsed.arkpulse_unauthorized_channel_threshold.max(1) as u64,
        combined: parsed.arkpulse_combined_security_threshold.max(1) as u64,
    }
}

async fn is_agent_autonomy_paused(agent: &SharedAgent) -> bool {
    let storage = {
        let guard = agent.read().await;
        guard.storage.clone()
    };
    let settings = load_autonomy_settings_snapshot(&storage).await;
    settings.agent_paused
}

fn is_security_incident(
    security: Option<&crate::core::SecuritySnapshot>,
    thresholds: ArkPulseSecurityThresholds,
) -> bool {
    let Some(sec) = security else {
        return false;
    };
    if sec.injection_attempts > 0 {
        return true;
    }
    if sec.auth_failures >= thresholds.auth_failures {
        return true;
    }
    if sec.rate_limit_hits >= thresholds.rate_limit_hits {
        return true;
    }
    if sec.unauthorized_channel_attempts >= thresholds.unauthorized_channel {
        return true;
    }
    let combined = sec.auth_failures + sec.rate_limit_hits + sec.unauthorized_channel_attempts;
    combined >= thresholds.combined
}

fn build_noncritical_summary(
    overdue_tasks: usize,
    approaching_goals: usize,
    security: Option<&crate::core::SecuritySnapshot>,
    doctor_medium: usize,
    doctor_low: usize,
    health_warns: usize,
) -> String {
    let mut parts: Vec<String> = Vec::new();
    if overdue_tasks > 0 {
        parts.push(format!("{} overdue task(s)", overdue_tasks));
    }
    if approaching_goals > 0 {
        parts.push(format!(
            "{} goal deadline(s) approaching",
            approaching_goals
        ));
    }
    if let Some(sec) = security {
        let total = sec.auth_failures
            + sec.rate_limit_hits
            + sec.unauthorized_channel_attempts
            + sec.injection_attempts;
        if total > 0 {
            parts.push(format!("{} security event(s)", total));
        }
    }
    if doctor_medium > 0 || doctor_low > 0 {
        let mut d = Vec::new();
        if doctor_medium > 0 {
            d.push(format!("{} medium", doctor_medium));
        }
        if doctor_low > 0 {
            d.push(format!("{} low", doctor_low));
        }
        parts.push(format!("doctor findings: {}", d.join(", ")));
    }
    if health_warns > 0 {
        parts.push(format!("{} health warning(s)", health_warns));
    }
    if parts.is_empty() {
        "No critical incidents. Monitoring only.".to_string()
    } else {
        format!("No critical incidents. Monitoring: {}.", parts.join(", "))
    }
}

fn build_critical_notification(
    failed_tasks: usize,
    dead_apps: usize,
    health_errors: usize,
    security: Option<&crate::core::SecuritySnapshot>,
    thresholds: ArkPulseSecurityThresholds,
    doctor_findings: &[DoctorFinding],
) -> (String, Vec<String>) {
    let mut flags: Vec<String> = Vec::new();
    let mut reasons: Vec<String> = Vec::new();
    let mut actions: Vec<String> = Vec::new();

    if dead_apps > 0 {
        flags.push("broken_app".to_string());
        reasons.push(format!("{} deployed app process(es) down", dead_apps));
        actions.push("Restart affected app(s) from Apps and inspect runtime logs.".to_string());
    }
    if failed_tasks > 0 {
        flags.push("failed_tasks".to_string());
        reasons.push(format!("{} failed task(s)", failed_tasks));
        actions.push("Review failed tasks and retry after fixing root cause.".to_string());
    }
    if health_errors > 0 {
        flags.push("service_error".to_string());
        reasons.push(format!("{} service health check error(s)", health_errors));
        actions.push("Fix failing service checks shown in ArkPulse details.".to_string());
    }

    if let Some(sec) = security {
        if sec.injection_attempts > 0 {
            flags.push("security_injection".to_string());
            reasons.push(format!(
                "{} prompt injection attempt(s)",
                sec.injection_attempts
            ));
        }
        if sec.auth_failures >= thresholds.auth_failures
            || sec.rate_limit_hits >= thresholds.rate_limit_hits
            || sec.unauthorized_channel_attempts >= thresholds.unauthorized_channel
            || (sec.auth_failures + sec.rate_limit_hits + sec.unauthorized_channel_attempts)
                >= thresholds.combined
        {
            flags.push("security_ddos".to_string());
            reasons.push(format!(
                "security spike (auth_fail={}, rate_limit={}, unauthorized={})",
                sec.auth_failures, sec.rate_limit_hits, sec.unauthorized_channel_attempts
            ));
            actions.push(
                "Check Security Logs immediately, block abusive sources, and rotate at-risk secrets."
                    .to_string(),
            );
        }
    }

    let mut high_or_critical: Vec<&DoctorFinding> = doctor_findings
        .iter()
        .filter(|f| f.user_actionable && (f.severity == "critical" || f.severity == "high"))
        .collect();
    if !high_or_critical.is_empty() {
        flags.push("doctor_high".to_string());
        reasons.push(format!(
            "{} high-risk doctor finding(s)",
            high_or_critical.len()
        ));
        high_or_critical.sort_by(|a, b| a.severity.cmp(&b.severity));
        if let Some(f) = high_or_critical.first() {
            let fix = f.fix_command.trim();
            if !fix.is_empty() {
                actions.push(format!("Apply priority fix: {}", fix));
            }
        }
    }

    flags.sort();
    flags.dedup();
    actions.sort();
    actions.dedup();

    if reasons.is_empty() {
        (
            "ArkPulse critical incident detected. Open ArkPulse details for diagnostics."
                .to_string(),
            vec!["critical".to_string()],
        )
    } else if actions.is_empty() {
        (
            format!("ArkPulse critical incident: {}.", reasons.join(", ")),
            flags,
        )
    } else {
        (
            format!(
                "ArkPulse critical incident: {}. Immediate actions: {}",
                reasons.join(", "),
                actions.join(" ")
            ),
            flags,
        )
    }
}

// ═══════════════════════════════════════════════════════════════════════════
// ArkPulse — proactive agent wake-up
// ═══════════════════════════════════════════════════════════════════════════

pub async fn run_pulse(agent: &SharedAgent) {
    if is_agent_autonomy_paused(agent).await {
        tracing::debug!("ArkPulse skipped (agent paused)");
        return;
    }
    if PULSE_RUNNING
        .compare_exchange(false, true, Ordering::AcqRel, Ordering::Acquire)
        .is_err()
    {
        tracing::debug!("ArkPulse skipped (already running)");
        return;
    }
    let _pulse_guard = PulseRunGuard;

    // ── Code-only checks first (zero LLM tokens) ────────────────────────
    // Only wake the LLM if there's actually something worth acting on.

    let http_client = reqwest::Client::builder()
        .timeout(std::time::Duration::from_secs(3))
        .build()
        .unwrap_or_default();

    let storage = {
        let agent_guard = agent.read().await;
        agent_guard.storage.clone()
    };
    let now_marker = chrono::Utc::now().to_rfc3339();
    let _ = storage
        .set(ARKPULSE_LAST_RUN_AT_KEY, now_marker.as_bytes())
        .await;
    let security_thresholds = load_arkpulse_security_thresholds(&storage).await;

    let (overdue_tasks, failed_tasks, approaching_goals, brief_channel, details, deployed_apps) = {
        let agent = agent.read().await;
        let now = chrono::Utc::now();
        let tasks = agent.tasks.read().await;
        let all_tasks = tasks.all();

        // Task counts
        let pending = all_tasks
            .iter()
            .filter(|t| matches!(t.status, TaskStatus::Pending))
            .count();
        let running = all_tasks
            .iter()
            .filter(|t| matches!(t.status, TaskStatus::InProgress))
            .count();
        let completed = all_tasks
            .iter()
            .filter(|t| matches!(t.status, TaskStatus::Completed))
            .count();

        // Find overdue tasks (scheduled time passed by >1 hour, still pending)
        let overdue: Vec<String> = all_tasks
            .iter()
            .filter(|t| {
                matches!(t.status, TaskStatus::Pending)
                    && t.scheduled_for
                        .map(|dt| dt < now - chrono::Duration::hours(1))
                        .unwrap_or(false)
            })
            .take(5)
            .map(|t| {
                let due = t
                    .scheduled_for
                    .map(|dt| dt.format("%Y-%m-%d %H:%M").to_string())
                    .unwrap_or_else(|| "?".to_string());
                format!("{} (due: {})", t.description, due)
            })
            .collect();

        // Find recently failed tasks
        let failed: Vec<String> = all_tasks
            .iter()
            .filter(|t| matches!(t.status, TaskStatus::Failed { .. }))
            .take(3)
            .map(|t| {
                let err = match &t.status {
                    TaskStatus::Failed { error } => error.as_str(),
                    _ => "unknown",
                };
                format!("{} (error: {})", t.description, &err[..err.len().min(80)])
            })
            .collect();

        // Find goals with approaching deadlines (≤3 days or overdue)
        let approaching_goals: Vec<String> = all_tasks
            .iter()
            .filter(|t| {
                t.action == "goal"
                    && matches!(t.status, TaskStatus::Pending | TaskStatus::InProgress)
                    && t.scheduled_for
                        .map(|dt| (dt - now).num_days() <= 3)
                        .unwrap_or(false)
            })
            .take(5)
            .map(|t| {
                let days_left = t.scheduled_for.map(|dt| (dt - now).num_days()).unwrap_or(0);
                let urgency = if days_left < 0 {
                    format!("OVERDUE by {} day(s)", days_left.abs())
                } else if days_left == 0 {
                    "DUE TODAY".to_string()
                } else {
                    format!("due in {} day(s)", days_left)
                };
                format!("{} ({})", t.description, urgency)
            })
            .collect();

        // Watcher count
        let active_watchers = agent.watcher_manager.list().await.len();

        // ── Health checks ────────────────────────────────────────────────
        let mut health_checks = Vec::new();

        // Mem0 bridge
        let mem0_url = format!("{}/health", agent.config.mem0.bridge_url);
        let mem0_check = match http_client.get(&mem0_url).send().await {
            Ok(resp) if resp.status().is_success() => {
                let body: serde_json::Value = resp.json().await.unwrap_or_default();
                let memories = body.get("memories").and_then(|v| v.as_u64()).unwrap_or(0);
                HealthCheck {
                    service: "Mem0".to_string(),
                    status: "ok".to_string(),
                    message: format!("{} memories", memories),
                }
            }
            Ok(resp) => HealthCheck {
                service: "Mem0".to_string(),
                status: "error".to_string(),
                message: format!("HTTP {}", resp.status()),
            },
            Err(e) => HealthCheck {
                service: "Mem0".to_string(),
                status: "error".to_string(),
                message: format!("{}", e),
            },
        };
        let total_memories = if mem0_check.status == "ok" {
            mem0_check
                .message
                .split_whitespace()
                .next()
                .and_then(|n| n.parse::<usize>().ok())
                .unwrap_or(0)
        } else {
            0
        };
        health_checks.push(mem0_check);

        // Playwright bridge
        let pw_url = std::env::var("PLAYWRIGHT_BRIDGE_URL")
            .unwrap_or_else(|_| "http://127.0.0.1:3100".to_string());
        let pw_check = match http_client.get(format!("{}/health", pw_url)).send().await {
            Ok(resp) if resp.status().is_success() => HealthCheck {
                service: "Playwright".to_string(),
                status: "ok".to_string(),
                message: "Running".to_string(),
            },
            _ => HealthCheck {
                service: "Playwright".to_string(),
                status: "warn".to_string(),
                message: "Not available".to_string(),
            },
        };
        health_checks.push(pw_check);

        // LLM connectivity (report the currently active slot from model pool)
        let llm_check = {
            // Prefer enabled Primary from config order, then primary_model_id, then any enabled slot.
            let selected_slot_id = agent
                .config
                .model_pool
                .slots
                .iter()
                .find(|s| {
                    s.enabled
                        && matches!(s.role, crate::core::config::ModelRole::Primary)
                        && agent.model_pool.contains_key(&s.id)
                })
                .map(|s| s.id.clone())
                .or_else(|| {
                    if !agent.primary_model_id.is_empty()
                        && agent.model_pool.contains_key(&agent.primary_model_id)
                    {
                        Some(agent.primary_model_id.clone())
                    } else {
                        None
                    }
                })
                .or_else(|| {
                    agent
                        .config
                        .model_pool
                        .slots
                        .iter()
                        .find(|s| s.enabled && agent.model_pool.contains_key(&s.id))
                        .map(|s| s.id.clone())
                })
                .or_else(|| agent.model_pool.keys().next().cloned());

            if let Some(slot_id) = selected_slot_id {
                if let Some((slot, client)) = agent.model_pool.get(&slot_id) {
                    let provider_label = match &slot.provider {
                        crate::core::LlmProvider::Anthropic { .. } => "anthropic".to_string(),
                        crate::core::LlmProvider::Ollama { .. } => "ollama".to_string(),
                        crate::core::LlmProvider::OpenAI { base_url, .. } => {
                            if let Some(url) = base_url {
                                if url.to_lowercase().contains("openrouter") {
                                    "openrouter".to_string()
                                } else if let Ok(parsed) = url::Url::parse(url) {
                                    parsed
                                        .host_str()
                                        .map(|h| h.trim_start_matches("api.").to_string())
                                        .unwrap_or_else(|| "openai-compatible".to_string())
                                } else {
                                    "openai-compatible".to_string()
                                }
                            } else {
                                "openai".to_string()
                            }
                        }
                    };

                    let role_label = match slot.role {
                        crate::core::config::ModelRole::Primary => "primary",
                        crate::core::config::ModelRole::Fast => "fast",
                        crate::core::config::ModelRole::Code => "code",
                        crate::core::config::ModelRole::Research => "research",
                        crate::core::config::ModelRole::Fallback => "fallback",
                    };

                    let model = client.model_name();
                    let model_display = if model.contains('/') {
                        model.to_string()
                    } else {
                        format!("{}/{}", provider_label, model)
                    };

                    HealthCheck {
                        service: "LLM".to_string(),
                        status: "ok".to_string(),
                        message: format!("{} ({})", model_display, role_label),
                    }
                } else {
                    HealthCheck {
                        service: "LLM".to_string(),
                        status: "warn".to_string(),
                        message: "Configured slot not loaded".to_string(),
                    }
                }
            } else {
                // Legacy fallback
                HealthCheck {
                    service: "LLM".to_string(),
                    status: "ok".to_string(),
                    message: agent.llm.model_name().to_string(),
                }
            }
        };
        health_checks.push(llm_check);

        // ── Security snapshot ────────────────────────────────────────────
        let sec_snapshot = agent.security_events.snapshot_and_reset();

        // Persist security events to DB if any occurred
        if sec_snapshot.has_events() {
            let now_str = now.to_rfc3339();
            let events = [
                (
                    "injection",
                    "high",
                    sec_snapshot.injection_attempts,
                    "Prompt injection/leakage attempts",
                ),
                (
                    "auth_failure",
                    "medium",
                    sec_snapshot.auth_failures,
                    "Authentication failures",
                ),
                (
                    "rate_limit",
                    "low",
                    sec_snapshot.rate_limit_hits,
                    "Rate limit breaches",
                ),
                (
                    "unauthorized_channel",
                    "medium",
                    sec_snapshot.unauthorized_channel_attempts,
                    "Unauthorized channel attempts",
                ),
            ];
            for (event_type, severity, count, desc) in &events {
                if *count > 0 {
                    let log = crate::storage::security_log::Model {
                        id: uuid::Uuid::new_v4().to_string(),
                        event_type: event_type.to_string(),
                        severity: severity.to_string(),
                        message: format!("{}: {} event(s)", desc, count),
                        source: Some("arkpulse".to_string()),
                        count: *count as i64,
                        created_at: now_str.clone(),
                    };
                    if let Err(e) = agent.storage.insert_security_log(&log).await {
                        tracing::debug!("Failed to persist security log: {}", e);
                    }
                }
            }
            tracing::info!(
                "ArkPulse security: injections={}, auth_fail={}, rate_limit={}, unauth={}",
                sec_snapshot.injection_attempts,
                sec_snapshot.auth_failures,
                sec_snapshot.rate_limit_hits,
                sec_snapshot.unauthorized_channel_attempts,
            );
        }

        let channel = agent
            .storage
            .get("daily_brief_channel")
            .await
            .ok()
            .flatten()
            .and_then(|b| String::from_utf8(b).ok())
            .unwrap_or_else(|| "web".to_string());

        // Deployed apps health snapshot
        let now_ts = chrono::Utc::now();
        let app_snapshots = agent.app_registry.pulse_snapshot().await;
        let deployed_apps: Vec<AppPulseInfo> = app_snapshots
            .iter()
            .map(|s| AppPulseInfo {
                id: s.id.clone(),
                title: s.title.clone(),
                is_static: s.is_static,
                process_alive: s.process_alive,
                requests_since_last_check: s.requests_since_last_check,
                idle_hours: (now_ts - s.last_accessed).num_hours(),
            })
            .collect();

        let security_snapshot = if sec_snapshot.has_events() {
            Some(sec_snapshot.clone())
        } else {
            None
        };
        let doctor_findings = run_doctor_checks(
            &agent,
            &http_client,
            &deployed_apps,
            security_snapshot.as_ref(),
            security_thresholds,
        )
        .await;
        let doctor_score = compute_doctor_score(&doctor_findings);

        let details = PulseDetails {
            pending_tasks: pending,
            running_tasks: running,
            completed_tasks: completed,
            total_tasks: all_tasks.len(),
            active_watchers,
            total_memories,
            overdue_list: overdue.clone(),
            failed_list: failed.clone(),
            uptime_secs: 0,
            health_checks,
            security: security_snapshot,
            deployed_apps: deployed_apps.clone(),
            doctor_findings,
            doctor_score,
        };

        (
            overdue,
            failed,
            approaching_goals,
            channel,
            details,
            deployed_apps,
        )
    };

    // Deterministic pulse classification: notify only on critical breakage or security spikes.
    let has_overdue = !overdue_tasks.is_empty();
    let has_failures = !failed_tasks.is_empty();
    let has_security = details.security.as_ref().is_some_and(|s| s.has_events());
    let has_security_incident =
        is_security_incident(details.security.as_ref(), security_thresholds);
    let has_goal_deadlines = !approaching_goals.is_empty();
    let has_dead_apps = deployed_apps
        .iter()
        .any(|a| !a.process_alive && !a.is_static);
    let dead_app_count = deployed_apps
        .iter()
        .filter(|a| !a.process_alive && !a.is_static)
        .count();
    let doctor_high_count = details
        .doctor_findings
        .iter()
        .filter(|f| f.user_actionable && (f.severity == "critical" || f.severity == "high"))
        .count();
    let doctor_critical_count = details
        .doctor_findings
        .iter()
        .filter(|f| f.user_actionable && f.severity == "critical")
        .count();
    let doctor_medium_count = details
        .doctor_findings
        .iter()
        .filter(|f| f.user_actionable && f.severity == "medium")
        .count();
    let doctor_low_count = details
        .doctor_findings
        .iter()
        .filter(|f| f.user_actionable && f.severity == "low")
        .count();
    let has_doctor_alert = doctor_high_count > 0;
    let has_doctor_findings = !details.doctor_findings.is_empty();
    let health_error_count = details
        .health_checks
        .iter()
        .filter(|h| h.status.eq_ignore_ascii_case("error"))
        .count();
    let health_warn_count = details
        .health_checks
        .iter()
        .filter(|h| h.status.eq_ignore_ascii_case("warn"))
        .count();
    let is_breakage = has_failures || has_dead_apps || has_doctor_alert || health_error_count > 0;
    let has_user_visible_alert = is_breakage || has_security_incident;
    let should_notify_user = has_security_incident
        || has_dead_apps
        || health_error_count > 0
        || doctor_critical_count > 0;
    let has_any_signal = has_overdue
        || has_failures
        || has_security
        || has_goal_deadlines
        || has_dead_apps
        || has_doctor_findings
        || health_warn_count > 0
        || health_error_count > 0;

    if !has_any_signal {
        tracing::debug!("ArkPulse: all clear");
        let summary = "All clear, no issues detected".to_string();
        let event = PulseEvent {
            timestamp: chrono::Utc::now().to_rfc3339(),
            status: "ok".to_string(),
            message: summary.clone(),
            summary,
            flags: Vec::new(),
            overdue_tasks: 0,
            failed_tasks: 0,
            details,
        };
        let agent_guard = agent.read().await;
        log_pulse_event(&agent_guard, event).await;
        return;
    }

    if !has_user_visible_alert {
        let summary = build_noncritical_summary(
            overdue_tasks.len(),
            approaching_goals.len(),
            details.security.as_ref(),
            doctor_medium_count,
            doctor_low_count,
            health_warn_count,
        );
        let event = PulseEvent {
            timestamp: chrono::Utc::now().to_rfc3339(),
            status: "ok".to_string(),
            message: summary.clone(),
            summary,
            flags: vec!["non_critical".to_string()],
            overdue_tasks: overdue_tasks.len(),
            failed_tasks: failed_tasks.len(),
            details,
        };
        let agent_guard = agent.read().await;
        log_pulse_event(&agent_guard, event).await;
        tracing::info!("ArkPulse: non-critical signals recorded, no user notification sent");
        return;
    }

    let (alert_text, mut flags) = build_critical_notification(
        failed_tasks.len(),
        dead_app_count,
        health_error_count,
        details.security.as_ref(),
        security_thresholds,
        &details.doctor_findings,
    );
    if flags.is_empty() {
        flags.push("critical".to_string());
    }
    let summary = build_pulse_log_summary(
        overdue_tasks.len(),
        failed_tasks.len(),
        approaching_goals.len(),
        details.security.as_ref(),
        dead_app_count,
        doctor_high_count,
    );
    let should_emit_alert = if should_notify_user {
        should_emit_arkpulse_critical_notification(&storage, &alert_text).await
    } else {
        false
    };
    let agent_guard = agent.read().await;
    if should_emit_alert {
        agent_guard
            .emit_notification("ArkPulse Critical", &alert_text, "error", "arkpulse")
            .await;
    } else if should_notify_user {
        tracing::info!(
            "ArkPulse: suppressed duplicate critical notification within {}s cooldown",
            ARKPULSE_CRITICAL_NOTIFY_COOLDOWN_SECS
        );
    } else {
        tracing::info!(
            "ArkPulse: alert recorded without user notification (below ultra-severe threshold)"
        );
    }
    let event = PulseEvent {
        timestamp: chrono::Utc::now().to_rfc3339(),
        status: "alert".to_string(),
        message: summary.clone(),
        summary,
        flags,
        overdue_tasks: overdue_tasks.len(),
        failed_tasks: failed_tasks.len(),
        details: details.clone(),
    };
    log_pulse_event(&agent_guard, event).await;
    if should_emit_alert {
        agent_guard.notify_preferred_channel(&alert_text).await;
        tracing::info!(
            "ArkPulse: critical alert sent to preferred channel ({})",
            brief_channel
        );
    } else if should_notify_user {
        tracing::info!(
            "ArkPulse: duplicate critical alert not pushed to preferred channel ({})",
            brief_channel
        );
    } else {
        tracing::info!(
            "ArkPulse: preferred-channel notification skipped (below ultra-severe threshold) ({})",
            brief_channel
        );
    }
}

// ═══════════════════════════════════════════════════════════════════════════
// Mem0 Memory Decay Cleanup — prune stale ephemeral memories, keep core facts
// Runs once per month, only when server is idle (no recent activity).
// ═══════════════════════════════════════════════════════════════════════════

// ═══════════════════════════════════════════════════════════════════════════
// Security Log Cleanup — prune entries older than 15 days
// Runs every 15 days, only when server is idle.
// ═══════════════════════════════════════════════════════════════════════════

const SECURITY_CLEANUP_KEY: &str = "security_log_last_cleanup";
/// 15 days between cleanups
const SECURITY_CLEANUP_INTERVAL_SECS: i64 = 15 * 24 * 3600;
/// Only run if no user activity in the last 5 minutes
const SECURITY_IDLE_THRESHOLD_SECS: i64 = 300;

async fn run_security_log_cleanup(agent: &SharedAgent) {
    let (last_cleanup_bytes, last_activity) = {
        let agent_guard = agent.read().await;
        let lc = agent_guard
            .storage
            .get(SECURITY_CLEANUP_KEY)
            .await
            .unwrap_or(None);
        let la = agent_guard.last_activity_at();
        (lc, la)
    };

    let now = chrono::Utc::now();

    // Check if enough time has passed since last cleanup
    if let Some(bytes) = last_cleanup_bytes {
        if let Ok(ts_str) = String::from_utf8(bytes) {
            if let Ok(last_ts) = ts_str.parse::<chrono::DateTime<chrono::Utc>>() {
                if (now - last_ts).num_seconds() < SECURITY_CLEANUP_INTERVAL_SECS {
                    return;
                }
            }
        }
    }

    // Check if server is idle
    if let Some(last) = last_activity {
        if (now - last).num_seconds() < SECURITY_IDLE_THRESHOLD_SECS {
            return;
        }
    }

    let agent_guard = agent.read().await;
    match agent_guard.storage.cleanup_old_security_logs(15).await {
        Ok(deleted) => {
            if deleted > 0 {
                tracing::info!(
                    "Security log cleanup: pruned {} entries older than 15 days",
                    deleted
                );
            }
            let _ = agent_guard
                .storage
                .set(SECURITY_CLEANUP_KEY, now.to_rfc3339().as_bytes())
                .await;
        }
        Err(e) => {
            tracing::debug!("Security log cleanup failed: {}", e);
        }
    }
}

// ═══════════════════════════════════════════════════════════════════════════
// Unused App Notifications — notify user about idle deployed apps daily
// ═══════════════════════════════════════════════════════════════════════════

const UNUSED_APP_NOTIFY_PREFIX: &str = "unused_app_last_notified:";
/// Apps idle for more than 24 hours get a notification
const UNUSED_APP_IDLE_HOURS: i64 = 24;
/// Only send one notification per app per 24 hours
const UNUSED_APP_NOTIFY_COOLDOWN_SECS: i64 = 24 * 3600;

async fn run_unused_app_check(agent: &SharedAgent) {
    if is_agent_autonomy_paused(agent).await {
        tracing::debug!("Unused app check skipped (agent paused)");
        return;
    }

    let agent_guard = agent.read().await;
    let unused_apps = agent_guard
        .app_registry
        .get_unused_apps(UNUSED_APP_IDLE_HOURS)
        .await;

    if unused_apps.is_empty() {
        return;
    }

    let now = chrono::Utc::now();

    for (app_id, title, last_accessed) in &unused_apps {
        // Check cooldown — don't spam the same app notification every hour
        let notify_key = format!("{}{}", UNUSED_APP_NOTIFY_PREFIX, app_id);
        let last_notified = agent_guard
            .storage
            .get(&notify_key)
            .await
            .ok()
            .flatten()
            .and_then(|b| String::from_utf8(b).ok())
            .and_then(|s| s.parse::<chrono::DateTime<chrono::Utc>>().ok());

        if let Some(last) = last_notified {
            if (now - last).num_seconds() < UNUSED_APP_NOTIFY_COOLDOWN_SECS {
                continue; // Already notified recently
            }
        }

        let idle_hours = (now - *last_accessed).num_hours();
        let idle_display = if idle_hours >= 48 {
            format!("{} days", idle_hours / 24)
        } else {
            format!("{} hours", idle_hours)
        };

        let message = format!(
            "Your deployed app \"{}\" (id: {}) has had no traffic for {}. \
            Do you want to keep it running or should I shut it down?",
            title, app_id, idle_display
        );

        // In-app notification
        agent_guard
            .emit_notification("Unused App", &message, "info", "app_cleanup")
            .await;

        // Push to preferred channel
        agent_guard.notify_preferred_channel(&message).await;

        // Record notification time
        let _ = agent_guard
            .storage
            .set(&notify_key, now.to_rfc3339().as_bytes())
            .await;

        tracing::info!(
            "Sent unused app notification for '{}' (idle {})",
            title,
            idle_display
        );
    }
}

async fn run_mem0_retry_drain(agent: &SharedAgent) {
    let drained = {
        let agent_guard = agent.read().await;
        agent_guard.flush_mem0_retry_queue(8).await
    };
    if drained > 0 {
        tracing::debug!("Mem0 retry queue drained {} entries", drained);
    }
}

// =====================================================================
// Episodic Memory Retention Cleanup (safe-by-default)
// - Disabled by default (memory.retention_enabled=false)
// - Only runs when episode count exceeds memory.max_episodes
// - Only deletes low-importance, low-access episodes
// - Strongly prefers deleting only consolidated episodes
// - Protects newest N episodes and episodes referenced by semantic-fact sources
// =====================================================================

const EPISODE_RETENTION_CLEANUP_KEY: &str = "episode_retention_last_cleanup";
const EPISODE_RETENTION_EMERGENCY_CLEANUP_KEY: &str = "episode_retention_emergency_last_cleanup";
/// Trigger emergency prune when free disk drops below this threshold.
const EPISODE_RETENTION_EMERGENCY_MIN_FREE_BYTES: u64 = 2 * 1024 * 1024 * 1024; // 2 GiB
/// Emergency runs use a short cooldown instead of the normal day-level interval.
const EPISODE_RETENTION_EMERGENCY_COOLDOWN_SECS: i64 = 30 * 60; // 30 minutes
/// In emergency mode, allow pruning episodes as recent as this age.
const EPISODE_RETENTION_EMERGENCY_MIN_AGE_DAYS: u64 = 2;
/// Hard ceiling so emergency mode cannot delete unbounded rows in one pass.
const EPISODE_RETENTION_EMERGENCY_MAX_DELETE_PER_RUN: u64 = 20_000;

fn parse_rfc3339(s: &str) -> Option<chrono::DateTime<chrono::Utc>> {
    chrono::DateTime::parse_from_rfc3339(s)
        .map(|dt| dt.with_timezone(&chrono::Utc))
        .ok()
}

fn available_disk_bytes(path: &Path) -> Option<u64> {
    #[cfg(unix)]
    {
        let path_str = path.to_str()?;
        let output = std::process::Command::new("df")
            .args(["-Pk", path_str])
            .output()
            .ok()?;
        if !output.status.success() {
            return None;
        }
        let stdout = String::from_utf8_lossy(&output.stdout);
        // POSIX df -Pk columns: Filesystem 1024-blocks Used Available Capacity Mounted on
        let line = stdout.lines().nth(1)?;
        let available_kb = line.split_whitespace().nth(3)?.parse::<u64>().ok()?;
        return Some(available_kb.saturating_mul(1024));
    }

    #[cfg(target_os = "windows")]
    {
        let path_lit = path.to_string_lossy().replace('\'', "''");
        let script = format!(
            "$d=(Get-Item -LiteralPath '{}').PSDrive; if ($d) {{ [string]$d.Free }}",
            path_lit
        );
        let output = std::process::Command::new("powershell")
            .args(["-NoProfile", "-Command", &script])
            .output()
            .ok()?;
        if !output.status.success() {
            return None;
        }
        let stdout = String::from_utf8_lossy(&output.stdout);
        return stdout.trim().parse::<u64>().ok();
    }

    #[allow(unreachable_code)]
    None
}

fn collect_protected_episode_ids_from_fact_sources(
    sources: &[String],
) -> std::collections::HashSet<String> {
    let mut protected = std::collections::HashSet::new();
    for blob in sources {
        if let Ok(v) = serde_json::from_str::<serde_json::Value>(blob) {
            if let Some(arr) = v.as_array() {
                for item in arr {
                    if let Some(id) = item.as_str() {
                        protected.insert(id.to_string());
                    }
                }
            }
        }
    }
    protected
}

async fn run_episode_retention_cleanup(agent: &SharedAgent) {
    let (storage, mem_cfg, last_activity, data_dir) = {
        let agent_guard = agent.read().await;
        (
            agent_guard.storage.clone(),
            agent_guard.config.memory.clone(),
            agent_guard.last_activity_at(),
            agent_guard.data_dir().to_path_buf(),
        )
    };
    let now = chrono::Utc::now();
    let free_bytes = available_disk_bytes(&data_dir);
    let emergency_mode = free_bytes
        .map(|b| b <= EPISODE_RETENTION_EMERGENCY_MIN_FREE_BYTES)
        .unwrap_or(false);

    // Normal retention can remain disabled by default; emergency mode still activates
    // under real disk pressure to avoid hard outages.
    if !mem_cfg.retention_enabled && !emergency_mode {
        return;
    }

    if !emergency_mode {
        // Only run normal retention when server is idle.
        if let Some(last) = last_activity {
            if (now - last).num_seconds() < mem_cfg.retention_idle_threshold_secs as i64 {
                return;
            }
        }
        // Rate-limit normal runs.
        if let Ok(Some(bytes)) = storage.get(EPISODE_RETENTION_CLEANUP_KEY).await {
            if let Ok(ts) = String::from_utf8(bytes) {
                if let Some(last_ts) = parse_rfc3339(&ts) {
                    let min_secs = (mem_cfg.retention_run_interval_days as i64) * 24 * 3600;
                    if (now - last_ts).num_seconds() < min_secs {
                        return;
                    }
                }
            }
        }
    } else if let Ok(Some(bytes)) = storage.get(EPISODE_RETENTION_EMERGENCY_CLEANUP_KEY).await {
        // Separate cooldown for emergency mode (much shorter than normal cadence).
        if let Ok(ts) = String::from_utf8(bytes) {
            if let Some(last_ts) = parse_rfc3339(&ts) {
                if (now - last_ts).num_seconds() < EPISODE_RETENTION_EMERGENCY_COOLDOWN_SECS {
                    return;
                }
            }
        }
    }

    // Normal retention only prunes above max_episodes.
    // Emergency mode prunes regardless of max_episodes to recover free disk.
    let count = storage.count_episodes().await.unwrap_or(0) as i64;
    if !emergency_mode && count <= mem_cfg.max_episodes as i64 {
        return;
    }
    if emergency_mode && count <= mem_cfg.retention_keep_last as i64 {
        return;
    }

    let effective_cutoff_days = if emergency_mode {
        EPISODE_RETENTION_EMERGENCY_MIN_AGE_DAYS
    } else {
        mem_cfg.retention_min_age_days
    };
    let cutoff = now - chrono::Duration::days(effective_cutoff_days as i64);
    let cutoff_rfc3339 = cutoff.to_rfc3339();

    // Protect newest N episodes (always keep).
    let keep_newest = storage
        .list_newest_episode_ids(mem_cfg.retention_keep_last as u64)
        .await
        .unwrap_or_default();
    let keep_newest_protected: std::collections::HashSet<String> =
        keep_newest.into_iter().collect();
    let mut protected = keep_newest_protected.clone();

    // Protect episode ids referenced as sources by semantic facts (optional, default true).
    let mut fact_protected: std::collections::HashSet<String> = std::collections::HashSet::new();
    if mem_cfg.retention_protect_fact_sources {
        let sources = storage
            .list_all_semantic_fact_sources()
            .await
            .unwrap_or_default();
        fact_protected = collect_protected_episode_ids_from_fact_sources(&sources);
        protected.extend(fact_protected.clone());
    }

    // Delete in bounded batches.
    let target = if emergency_mode {
        let emergency_cap = mem_cfg
            .retention_max_delete_per_run
            .saturating_mul(4)
            .clamp(200, EPISODE_RETENTION_EMERGENCY_MAX_DELETE_PER_RUN);
        let max_deletable = (count - mem_cfg.retention_keep_last as i64).max(0) as u64;
        max_deletable.min(emergency_cap)
    } else {
        let needed = (count - mem_cfg.max_episodes as i64).max(0) as u64;
        (needed + 100).min(mem_cfg.retention_max_delete_per_run.max(1))
    };
    if target == 0 {
        return;
    }

    let candidates = storage
        .list_episode_prune_candidates(
            &cutoff_rfc3339,
            if emergency_mode {
                false
            } else {
                mem_cfg.retention_require_consolidated
            },
            if emergency_mode {
                1.0
            } else {
                mem_cfg.retention_max_importance
            },
            if emergency_mode {
                i32::MAX
            } else {
                mem_cfg.retention_max_access_count
            },
            target,
        )
        .await
        .unwrap_or_default();

    let mut delete_ids: Vec<String> = candidates
        .into_iter()
        .filter(|id| !protected.contains(id))
        .collect();

    // Emergency fallback: if fact-source protection blocks all candidates under disk pressure,
    // relax only that protection (still preserves newest N) to free space.
    if emergency_mode && delete_ids.is_empty() && !fact_protected.is_empty() {
        let relaxed_candidates = storage
            .list_episode_prune_candidates(&cutoff_rfc3339, false, 1.0, i32::MAX, target)
            .await
            .unwrap_or_default();
        delete_ids = relaxed_candidates
            .into_iter()
            .filter(|id| !keep_newest_protected.contains(id))
            .collect();
    }

    if delete_ids.is_empty() {
        // Record attempts so we don't spin aggressively.
        let key = if emergency_mode {
            EPISODE_RETENTION_EMERGENCY_CLEANUP_KEY
        } else {
            EPISODE_RETENTION_CLEANUP_KEY
        };
        let _ = storage.set(key, now.to_rfc3339().as_bytes()).await;
        return;
    }

    match storage.delete_episodes_by_ids(&delete_ids).await {
        Ok(deleted) => {
            if emergency_mode {
                tracing::warn!(
                    "Episode emergency prune: deleted={} (count={}, keep_last={}, cutoff_days={}, free_bytes={:?})",
                    deleted,
                    count,
                    mem_cfg.retention_keep_last,
                    effective_cutoff_days,
                    free_bytes
                );
            } else {
                tracing::info!(
                    "Episode retention cleanup: deleted={} (count={}, max_episodes={}, cutoff_days={}, consolidated_required={})",
                    deleted,
                    count,
                    mem_cfg.max_episodes,
                    mem_cfg.retention_min_age_days,
                    mem_cfg.retention_require_consolidated
                );
            }
            let key = if emergency_mode {
                EPISODE_RETENTION_EMERGENCY_CLEANUP_KEY
            } else {
                EPISODE_RETENTION_CLEANUP_KEY
            };
            let _ = storage.set(key, now.to_rfc3339().as_bytes()).await;
        }
        Err(e) => {
            if emergency_mode {
                tracing::warn!("Episode emergency prune failed: {}", e);
            } else {
                tracing::debug!("Episode retention cleanup failed: {}", e);
            }
        }
    }
}

const MEM0_CLEANUP_KEY: &str = "mem0_last_cleanup";
const MEM0_SCOPE_INDEX_KEY: &str = "mem0_scope_index";
/// Minimum 30 days between cleanups
const MEM0_CLEANUP_INTERVAL_SECS: i64 = 30 * 24 * 3600;
/// Only run if no user activity in the last 10 minutes
const MEM0_IDLE_THRESHOLD_SECS: i64 = 600;
/// Bound each Mem0 scope cleanup to avoid hanging the sentinel loop.
const MEM0_CLEANUP_SCOPE_TIMEOUT_SECS: u64 = 180;

async fn run_mem0_cleanup(agent: &SharedAgent) {
    // Quick non-blocking check: is mem0 available?
    let (mem0, storage, last_cleanup_bytes, last_activity) = {
        let agent_guard = agent.read().await;
        if !agent_guard.mem0.is_available() {
            return;
        }
        let lc = agent_guard
            .storage
            .get(MEM0_CLEANUP_KEY)
            .await
            .unwrap_or(None);
        let la = agent_guard.last_activity_at();
        (
            agent_guard.mem0.clone(),
            agent_guard.storage.clone(),
            lc,
            la,
        )
    };
    // Drop the lock immediately; never hold it during cleanup

    // Check if enough time has passed since last cleanup (monthly)
    let now = chrono::Utc::now();
    if let Some(bytes) = last_cleanup_bytes {
        if let Ok(ts_str) = String::from_utf8(bytes) {
            if let Ok(last_ts) = ts_str.parse::<chrono::DateTime<chrono::Utc>>() {
                if (now - last_ts).num_seconds() < MEM0_CLEANUP_INTERVAL_SECS {
                    return; // Not yet time
                }
            }
        }
    }

    // Check if server is idle (no recent user messages)
    if let Some(last) = last_activity {
        if (now - last).num_seconds() < MEM0_IDLE_THRESHOLD_SECS {
            return; // Server is busy, skip
        }
    }

    tracing::info!("Mem0 monthly cleanup starting (server idle)...");

    let mut scopes = storage
        .get(MEM0_SCOPE_INDEX_KEY)
        .await
        .ok()
        .flatten()
        .and_then(|raw| serde_json::from_slice::<Vec<String>>(&raw).ok())
        .unwrap_or_default();
    if scopes.is_empty() {
        scopes.push("channel:web".to_string());
    }

    let mut total_deleted = 0usize;
    let mut last_remaining = 0usize;
    let mut last_core_facts = 0usize;
    let mut had_error = false;
    for scope in scopes {
        match tokio::time::timeout(
            Duration::from_secs(MEM0_CLEANUP_SCOPE_TIMEOUT_SECS),
            mem0.cleanup(&scope),
        )
        .await
        {
            Ok(Ok(r)) => {
                total_deleted += r.deleted;
                last_remaining = r.remaining;
                last_core_facts = r.core_facts;
            }
            Ok(Err(e)) => {
                had_error = true;
                tracing::debug!("Mem0 cleanup failed for scope '{}': {}", scope, e);
            }
            Err(_) => {
                had_error = true;
                tracing::warn!(
                    "Mem0 cleanup timed out for scope '{}' after {}s",
                    scope,
                    MEM0_CLEANUP_SCOPE_TIMEOUT_SECS
                );
            }
        }
    }

    if !had_error {
        tracing::info!(
            "Mem0 cleanup done: pruned {} memories ({} remaining, {} core facts)",
            total_deleted,
            last_remaining,
            last_core_facts
        );
        let _ = storage
            .set(MEM0_CLEANUP_KEY, chrono::Utc::now().to_rfc3339().as_bytes())
            .await;
    }
}
