use std::collections::HashSet;

use serde::{Deserialize, Serialize};

/// A single Pulse event for the UI log
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
    #[serde(default)]
    pub scan_started_at: String,
    #[serde(default)]
    pub scan_finished_at: String,
    #[serde(default)]
    pub scan_duration_ms: u64,
    #[serde(default)]
    pub notification_outcome: String,
    #[serde(default)]
    pub scan_log: Vec<PulseScanSection>,
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
    /// Deterministic Pulse doctor findings
    #[serde(default)]
    pub doctor_findings: Vec<DoctorFinding>,
    /// 0..100 score where 100 means healthier posture
    #[serde(default)]
    pub doctor_score: u32,
}

#[derive(Debug, Clone, Default, Serialize, Deserialize)]
pub struct PulseScanMetric {
    pub label: String,
    pub value: String,
}

#[derive(Debug, Clone, Default, Serialize, Deserialize)]
pub struct PulseScanSection {
    pub id: String,
    pub title: String,
    pub status: String,
    #[serde(default)]
    pub summary: String,
    #[serde(default)]
    pub detail: String,
    #[serde(default)]
    pub duration_ms: u64,
    #[serde(default)]
    pub metrics: Vec<PulseScanMetric>,
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
#[serde(rename_all = "snake_case")]
pub enum DoctorReadonlyInvestigationTopic {
    MemoryCaptureHealth,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum DoctorManagedAppOperation {
    CompilePythonRequirements,
    GenerateCargoLockfile,
    RemoveNpmInstallHooks,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(tag = "kind", rename_all = "snake_case")]
pub enum DoctorRemediationSpec {
    TunnelStartVerify,
    TunnelRestartVerify,
    AppRestart {
        app_id: String,
    },
    ShellCommand {
        command: String,
    },
    ManagedAppOperation {
        app_id: String,
        operation: DoctorManagedAppOperation,
    },
    ReadonlyInvestigation {
        topic: DoctorReadonlyInvestigationTopic,
    },
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

#[derive(Debug, Default)]
pub struct StalePulseAppReferenceReport {
    pub event_ids: Vec<String>,
    pub missing_app_ids: HashSet<String>,
}

pub const PULSE_LOG_KEY: &str = "arkpulse_log";
pub const SENTINEL_SCHEDULER_HEARTBEAT_KEY: &str = "sentinel_scheduler_heartbeat_v1";
pub const SENTINEL_WATCHER_HEARTBEAT_KEY: &str = "sentinel_watcher_heartbeat_v1";
pub const SENTINEL_INTEGRATION_SYNC_HEARTBEAT_KEY: &str = "sentinel_integration_sync_heartbeat_v1";
pub const SENTINEL_APPROVAL_EXPIRY_HEARTBEAT_KEY: &str = "sentinel_approval_expiry_heartbeat_v1";
pub const SENTINEL_ARKPULSE_HEARTBEAT_KEY: &str = "sentinel_arkpulse_heartbeat_v1";
pub const SENTINEL_AUTO_ANALYSIS_HEARTBEAT_KEY: &str = "sentinel_auto_analysis_heartbeat_v1";
