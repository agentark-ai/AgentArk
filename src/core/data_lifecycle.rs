use anyhow::Result;
use serde::{Deserialize, Serialize};

use crate::storage::Storage;

const DATA_LIFECYCLE_SETTINGS_KEY: &str = "data_lifecycle_settings_v1";
const MAX_RETENTION_DAYS: u64 = 36_500;
const LEGACY_MEMORY_RETENTION_DAYS: u64 = 365;
const MAX_INTERVAL_SECS: u64 = 7 * 24 * 60 * 60;
const MIN_NOTIFICATION_INTERVAL_SECS: u64 = 300;
const MIN_HOUSEKEEPING_INTERVAL_SECS: u64 = 300;
const MAX_SECURITY_INTERVAL_DAYS: u64 = 3650;
const MIN_SECURITY_IDLE_THRESHOLD_SECS: u64 = 60;

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct DataLifecycleSettings {
    #[serde(default = "default_cleanup_enabled")]
    pub cleanup_enabled: bool,
    #[serde(default = "default_cleanup_enabled")]
    pub notifications_cleanup_enabled: bool,
    #[serde(default = "default_cleanup_enabled")]
    pub logs_cleanup_enabled: bool,
    #[serde(default = "default_notifications_retention_days")]
    pub notifications_retention_days: u64,
    #[serde(default = "default_notification_cleanup_interval_secs")]
    pub notification_cleanup_interval_secs: u64,
    #[serde(default = "default_execution_trace_retention_days")]
    pub execution_trace_retention_days: u64,
    #[serde(default = "default_execution_proof_retention_days")]
    pub execution_proof_retention_days: u64,
    #[serde(default = "default_operational_log_retention_days")]
    pub operational_log_retention_days: u64,
    #[serde(default = "default_security_log_retention_days")]
    pub security_log_retention_days: u64,
    #[serde(default = "default_approval_log_retention_days")]
    pub approval_log_retention_days: u64,
    #[serde(default = "default_swarm_delegation_retention_days")]
    pub swarm_delegation_retention_days: u64,
    #[serde(default = "default_llm_usage_retention_days")]
    pub llm_usage_retention_days: u64,
    #[serde(default = "default_terminal_task_retention_days")]
    pub terminal_task_retention_days: u64,
    #[serde(default = "default_execution_run_retention_days")]
    pub execution_run_retention_days: u64,
    #[serde(default = "default_background_session_retention_days")]
    pub background_session_retention_days: u64,
    #[serde(default = "default_browser_session_retention_days")]
    pub browser_session_retention_days: u64,
    #[serde(default = "default_automation_run_retention_days")]
    pub automation_run_retention_days: u64,
    #[serde(default = "default_message_retention_days")]
    pub message_retention_days: u64,
    #[serde(default = "default_experience_run_retention_days")]
    pub experience_run_retention_days: u64,
    #[serde(default = "default_experience_edge_retention_days")]
    pub experience_edge_retention_days: u64,
    #[serde(default = "default_learning_candidate_retention_days")]
    pub learning_candidate_retention_days: u64,
    #[serde(default = "default_experience_item_retention_days")]
    pub experience_item_retention_days: u64,
    #[serde(default = "default_procedural_pattern_retention_days")]
    pub procedural_pattern_retention_days: u64,
    #[serde(default = "default_recall_event_retention_days")]
    pub recall_event_retention_days: u64,
    #[serde(default = "default_recall_test_retention_days")]
    pub recall_test_retention_days: u64,
    #[serde(default = "default_housekeeping_interval_secs")]
    pub housekeeping_interval_secs: u64,
    #[serde(default = "default_security_cleanup_interval_days")]
    pub security_cleanup_interval_days: u64,
    #[serde(default = "default_security_cleanup_idle_threshold_secs")]
    pub security_cleanup_idle_threshold_secs: u64,
}

fn default_cleanup_enabled() -> bool {
    true
}

fn default_notifications_retention_days() -> u64 {
    7
}

fn default_notification_cleanup_interval_secs() -> u64 {
    3600
}

fn default_execution_trace_retention_days() -> u64 {
    30
}

fn default_execution_proof_retention_days() -> u64 {
    30
}

fn default_operational_log_retention_days() -> u64 {
    30
}

fn default_security_log_retention_days() -> u64 {
    30
}

fn default_approval_log_retention_days() -> u64 {
    30
}

fn default_swarm_delegation_retention_days() -> u64 {
    30
}

fn default_llm_usage_retention_days() -> u64 {
    30
}

fn default_terminal_task_retention_days() -> u64 {
    90
}

fn default_execution_run_retention_days() -> u64 {
    90
}

fn default_background_session_retention_days() -> u64 {
    90
}

fn default_browser_session_retention_days() -> u64 {
    30
}

fn default_automation_run_retention_days() -> u64 {
    90
}

fn default_message_retention_days() -> u64 {
    365
}

fn default_experience_run_retention_days() -> u64 {
    90
}

fn default_experience_edge_retention_days() -> u64 {
    90
}

fn default_learning_candidate_retention_days() -> u64 {
    30
}

fn default_experience_item_retention_days() -> u64 {
    0
}

fn default_procedural_pattern_retention_days() -> u64 {
    0
}

fn default_recall_event_retention_days() -> u64 {
    365
}

fn default_recall_test_retention_days() -> u64 {
    365
}

fn default_housekeeping_interval_secs() -> u64 {
    3600
}

fn default_security_cleanup_interval_days() -> u64 {
    15
}

fn default_security_cleanup_idle_threshold_secs() -> u64 {
    300
}

impl Default for DataLifecycleSettings {
    fn default() -> Self {
        Self {
            cleanup_enabled: default_cleanup_enabled(),
            notifications_cleanup_enabled: default_cleanup_enabled(),
            logs_cleanup_enabled: default_cleanup_enabled(),
            notifications_retention_days: default_notifications_retention_days(),
            notification_cleanup_interval_secs: default_notification_cleanup_interval_secs(),
            execution_trace_retention_days: default_execution_trace_retention_days(),
            execution_proof_retention_days: default_execution_proof_retention_days(),
            operational_log_retention_days: default_operational_log_retention_days(),
            security_log_retention_days: default_security_log_retention_days(),
            approval_log_retention_days: default_approval_log_retention_days(),
            swarm_delegation_retention_days: default_swarm_delegation_retention_days(),
            llm_usage_retention_days: default_llm_usage_retention_days(),
            terminal_task_retention_days: default_terminal_task_retention_days(),
            execution_run_retention_days: default_execution_run_retention_days(),
            background_session_retention_days: default_background_session_retention_days(),
            browser_session_retention_days: default_browser_session_retention_days(),
            automation_run_retention_days: default_automation_run_retention_days(),
            message_retention_days: default_message_retention_days(),
            experience_run_retention_days: default_experience_run_retention_days(),
            experience_edge_retention_days: default_experience_edge_retention_days(),
            learning_candidate_retention_days: default_learning_candidate_retention_days(),
            experience_item_retention_days: default_experience_item_retention_days(),
            procedural_pattern_retention_days: default_procedural_pattern_retention_days(),
            recall_event_retention_days: default_recall_event_retention_days(),
            recall_test_retention_days: default_recall_test_retention_days(),
            housekeeping_interval_secs: default_housekeeping_interval_secs(),
            security_cleanup_interval_days: default_security_cleanup_interval_days(),
            security_cleanup_idle_threshold_secs: default_security_cleanup_idle_threshold_secs(),
        }
    }
}

impl DataLifecycleSettings {
    pub fn normalized(mut self) -> Self {
        self.notifications_retention_days =
            self.notifications_retention_days.min(MAX_RETENTION_DAYS);
        self.execution_trace_retention_days =
            self.execution_trace_retention_days.min(MAX_RETENTION_DAYS);
        self.execution_proof_retention_days =
            self.execution_proof_retention_days.min(MAX_RETENTION_DAYS);
        self.operational_log_retention_days =
            self.operational_log_retention_days.min(MAX_RETENTION_DAYS);
        self.security_log_retention_days = self.security_log_retention_days.min(MAX_RETENTION_DAYS);
        self.approval_log_retention_days = self.approval_log_retention_days.min(MAX_RETENTION_DAYS);
        self.swarm_delegation_retention_days =
            self.swarm_delegation_retention_days.min(MAX_RETENTION_DAYS);
        self.llm_usage_retention_days = self.llm_usage_retention_days.min(MAX_RETENTION_DAYS);
        self.terminal_task_retention_days =
            self.terminal_task_retention_days.min(MAX_RETENTION_DAYS);
        self.execution_run_retention_days =
            self.execution_run_retention_days.min(MAX_RETENTION_DAYS);
        self.background_session_retention_days = self
            .background_session_retention_days
            .min(MAX_RETENTION_DAYS);
        self.browser_session_retention_days =
            self.browser_session_retention_days.min(MAX_RETENTION_DAYS);
        self.automation_run_retention_days =
            self.automation_run_retention_days.min(MAX_RETENTION_DAYS);
        self.message_retention_days = self.message_retention_days.min(MAX_RETENTION_DAYS);
        self.experience_run_retention_days =
            self.experience_run_retention_days.min(MAX_RETENTION_DAYS);
        self.experience_edge_retention_days =
            self.experience_edge_retention_days.min(MAX_RETENTION_DAYS);
        self.learning_candidate_retention_days = self
            .learning_candidate_retention_days
            .min(MAX_RETENTION_DAYS);
        self.experience_item_retention_days =
            self.experience_item_retention_days.min(MAX_RETENTION_DAYS);
        self.procedural_pattern_retention_days = self
            .procedural_pattern_retention_days
            .min(MAX_RETENTION_DAYS);
        if self.experience_item_retention_days == LEGACY_MEMORY_RETENTION_DAYS {
            self.experience_item_retention_days = 0;
        }
        if self.procedural_pattern_retention_days == LEGACY_MEMORY_RETENTION_DAYS {
            self.procedural_pattern_retention_days = 0;
        }
        self.recall_event_retention_days = self.recall_event_retention_days.min(MAX_RETENTION_DAYS);
        self.recall_test_retention_days = self.recall_test_retention_days.min(MAX_RETENTION_DAYS);
        self.notification_cleanup_interval_secs = self
            .notification_cleanup_interval_secs
            .clamp(MIN_NOTIFICATION_INTERVAL_SECS, MAX_INTERVAL_SECS);
        self.housekeeping_interval_secs = self
            .housekeeping_interval_secs
            .clamp(MIN_HOUSEKEEPING_INTERVAL_SECS, MAX_INTERVAL_SECS);
        self.security_cleanup_interval_days = self
            .security_cleanup_interval_days
            .clamp(1, MAX_SECURITY_INTERVAL_DAYS);
        self.security_cleanup_idle_threshold_secs = self
            .security_cleanup_idle_threshold_secs
            .clamp(MIN_SECURITY_IDLE_THRESHOLD_SECS, MAX_INTERVAL_SECS);
        self
    }
}

pub async fn load_data_lifecycle_settings(storage: &Storage) -> DataLifecycleSettings {
    match storage.get(DATA_LIFECYCLE_SETTINGS_KEY).await {
        Ok(Some(raw)) => match serde_json::from_slice::<DataLifecycleSettings>(&raw) {
            Ok(parsed) => parsed.normalized(),
            Err(_) => DataLifecycleSettings::default(),
        },
        _ => DataLifecycleSettings::default(),
    }
}

pub async fn save_data_lifecycle_settings(
    storage: &Storage,
    settings: &DataLifecycleSettings,
) -> Result<()> {
    let normalized = settings.clone().normalized();
    let raw = serde_json::to_vec(&normalized)?;
    storage.set(DATA_LIFECYCLE_SETTINGS_KEY, &raw).await?;
    Ok(())
}
