//! Sentinel - AgentArk's Background Guardian
//!
//! A unified background daemon that keeps AgentArk alive and proactive 24/7:
//!
//! - **Process watchdog**: Monitors tunnel + WhatsApp bridge, auto-restarts on crash
//! - **Task scheduler**: Fires cron tasks (daily brief, recurring jobs) at the right time
//! - **Watcher poller**: Evaluates watch conditions and triggers on match
//! - **Experience learning**: Consolidates execution evidence into learned memory
//! - **Approval expiry**: Cleans up stale approval requests
//! - **Pulse**: Periodically wakes the agent to reflect and act proactively
//!
//! All loops run inside a single tokio task with staggered intervals.

use std::collections::{hash_map::DefaultHasher, HashMap, HashSet};
use std::future::Future;
use std::hash::{Hash, Hasher};
use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::Arc;
use std::time::{Duration, Instant};

use crate::channels;
use crate::core::runtime::data_lifecycle::load_data_lifecycle_settings;
use crate::core::{Agent, TaskStatus};
use chrono::{DateTime, Utc};
use once_cell::sync::Lazy;
use regex::Regex;
use serde::Serialize;
use tokio::sync::{RwLock, Semaphore};

mod config;
mod managed_backup;
mod types;

pub use config::SentinelConfig;
pub use types::*;

type SharedAgent = Arc<RwLock<Agent>>;

const MAINTENANCE_DEFER_MINUTES: i64 = 10;
const MAINTENANCE_MAX_DEFERS: u32 = 3;
const PULSE_DEFER_MINUTES: i64 = 5;
const PULSE_MAX_DEFERS: u32 = 3;
const SENTINEL_STARTUP_SETTLE_SECS: u64 = 120;
const PULSE_STARTUP_SETTLE_SECS: u64 = 60;
const AUTO_ANALYSIS_STAGGER_SECS: u64 = 5 * 60;
const STORAGE_HOUSEKEEPING_INTERVAL_SECS: u64 = 60 * 60;
const SENTINEL_JOB_LEASE_KEY_PREFIX: &str = "sentinel_job_lease_v1:";
const SENTINEL_JOB_STATUS_KEY_PREFIX: &str = "sentinel_job_status_v1:";
static PULSE_RUNNING: AtomicBool = AtomicBool::new(false);
static SENTINEL_MAINTENANCE_RUNNING: AtomicBool = AtomicBool::new(false);
static SCHEDULED_TASK_PERMITS: Lazy<Arc<Semaphore>> = Lazy::new(|| {
    Arc::new(Semaphore::new(
        std::env::var("AGENTARK_TASK_WORKER_CONCURRENCY")
            .ok()
            .and_then(|value| value.trim().parse::<usize>().ok())
            .filter(|value| *value > 0)
            .unwrap_or(4),
    ))
});
const AUTONOMY_PAUSE_NUDGE_TITLE: &str = "Autonomy still paused";
const AUTONOMY_PAUSE_NUDGE_SOURCE: &str = "autonomy";
static WATCHER_TRIGGER_PERMITS: Lazy<Arc<Semaphore>> = Lazy::new(|| {
    Arc::new(Semaphore::new(
        std::env::var("AGENTARK_WATCHER_TRIGGER_CONCURRENCY")
            .ok()
            .and_then(|value| value.trim().parse::<usize>().ok())
            .filter(|value| *value > 0)
            .unwrap_or(4),
    ))
});
static WATCHER_POLL_TIMEOUT_SECS: Lazy<u64> = Lazy::new(|| {
    std::env::var("AGENTARK_WATCHER_POLL_TIMEOUT_SECS")
        .ok()
        .and_then(|value| value.trim().parse::<u64>().ok())
        .filter(|value| *value > 0)
        .unwrap_or(30)
});
// Supervision budget for Sentinel's internal maintenance jobs only.
// This does not apply to foreground chat requests or long-running user work.
static SENTINEL_JOB_TIMEOUT_SECS: Lazy<u64> = Lazy::new(|| {
    std::env::var("AGENTARK_SENTINEL_JOB_TIMEOUT_SECS")
        .ok()
        .and_then(|value| value.trim().parse::<u64>().ok())
        .filter(|value| *value > 0)
        .unwrap_or(90)
});
static SENTINEL_NOTIFY_TIMEOUT_SECS: Lazy<u64> = Lazy::new(|| {
    std::env::var("AGENTARK_SENTINEL_NOTIFY_TIMEOUT_SECS")
        .ok()
        .and_then(|value| value.trim().parse::<u64>().ok())
        .filter(|value| *value > 0)
        .unwrap_or(8)
});
static SENTINEL_RECENT_ACTIVITY_BUSY_SECS: Lazy<i64> = Lazy::new(|| {
    std::env::var("AGENTARK_SENTINEL_RECENT_ACTIVITY_BUSY_SECS")
        .ok()
        .and_then(|value| value.trim().parse::<i64>().ok())
        .filter(|value| *value > 0)
        .unwrap_or(180)
});
static WATCHER_TRIGGER_TIMEOUT_SECS: Lazy<u64> = Lazy::new(|| {
    std::env::var("AGENTARK_WATCHER_TRIGGER_TIMEOUT_SECS")
        .ok()
        .and_then(|value| value.trim().parse::<u64>().ok())
        .filter(|value| *value > 0)
        .unwrap_or(45)
});

struct PulseRunGuard;

#[derive(Debug, Serialize)]
struct SentinelJobStatusRecord {
    label: String,
    status: String,
    owner_id: Option<String>,
    fence_token: Option<u64>,
    updated_at: String,
    error: Option<String>,
}

impl Drop for PulseRunGuard {
    fn drop(&mut self) {
        PULSE_RUNNING.store(false, Ordering::Release);
    }
}

fn sentinel_job_key_suffix(label: &str) -> String {
    let mut suffix = String::new();
    let mut previous_separator = false;
    for ch in label.trim().chars() {
        if ch.is_ascii_alphanumeric() {
            suffix.push(ch.to_ascii_lowercase());
            previous_separator = false;
        } else if !previous_separator {
            suffix.push('_');
            previous_separator = true;
        }
    }
    let suffix = suffix.trim_matches('_').to_string();
    if suffix.is_empty() {
        "job".to_string()
    } else {
        suffix
    }
}

fn sentinel_job_lease_key(label: &str) -> String {
    format!(
        "{}{}",
        SENTINEL_JOB_LEASE_KEY_PREFIX,
        sentinel_job_key_suffix(label)
    )
}

fn sentinel_job_status_key(label: &str) -> String {
    format!(
        "{}{}",
        SENTINEL_JOB_STATUS_KEY_PREFIX,
        sentinel_job_key_suffix(label)
    )
}

fn sentinel_job_lease_ttl_secs() -> i64 {
    ((*SENTINEL_JOB_TIMEOUT_SECS)
        .saturating_mul(2)
        .saturating_add(60)) as i64
}

fn sentinel_job_owner_id(label: &str) -> String {
    format!(
        "pid:{}:{}:{}",
        std::process::id(),
        sentinel_job_key_suffix(label),
        uuid::Uuid::new_v4()
    )
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn pulse_target_app_id_extracts_supported_target_forms() {
        assert_eq!(
            pulse_target_app_id("app:cad20c5e").as_deref(),
            Some("cad20c5e")
        );
        assert_eq!(
            pulse_target_app_id("/apps/cad20c5e/health").as_deref(),
            Some("cad20c5e")
        );
        assert_eq!(
            pulse_target_app_id("http://127.0.0.1:8990/api/apps/cad20c5e/restart").as_deref(),
            Some("cad20c5e")
        );
        assert_eq!(pulse_target_app_id("/health"), None);
    }

    #[test]
    fn watcher_timeout_routes_preferred_aliases_to_fallback() {
        for channel in ["", "preferred", "push", "auto", "default", " Preferred "] {
            assert!(watcher_timeout_uses_preferred_fallback(channel));
        }
        assert!(!watcher_timeout_uses_preferred_fallback("telegram"));
        assert!(!watcher_timeout_uses_preferred_fallback("web"));
    }

    fn test_watcher_with_poll_arguments(
        poll_arguments: serde_json::Value,
    ) -> crate::core::automation::watcher::Watcher {
        crate::core::automation::watcher::Watcher {
            id: uuid::Uuid::new_v4(),
            description: "Test watcher".to_string(),
            poll_action: "semantic_watcher_poll".to_string(),
            poll_arguments,
            condition: crate::core::automation::watcher::WatchCondition {
                description: "A meaningful change occurred".to_string(),
                evaluation_mode:
                    crate::core::automation::watcher::WatchConditionEvaluationMode::Change,
                matcher: crate::core::automation::watcher::WatchConditionMatcher::Llm,
            },
            on_trigger: "Notify the user".to_string(),
            interval_secs: 60,
            timeout_secs: 24 * 60 * 60,
            notify_channel: "in_app".to_string(),
            repeat_on_match: true,
            status: crate::core::automation::watcher::WatcherStatus::Active,
            created_at: chrono::Utc::now(),
            last_poll_at: None,
            poll_count: 0,
            trigger_result: None,
            last_result: None,
            last_error: None,
            consecutive_failures: 0,
            next_poll_not_before: None,
            last_poll_outcome: None,
            notification_attempts: Vec::new(),
        }
    }

    fn comparable_observation_result() -> serde_json::Value {
        let mut target = serde_json::Map::new();
        target.insert(
            "id".to_string(),
            serde_json::Value::String("target-1".to_string()),
        );
        target.insert(
            crate::core::automation::DURABLE_OBSERVATION_SOURCE_FIELD.to_string(),
            serde_json::json!({
                "kind": "http",
                "locator": "https://example.test/state"
            }),
        );
        target.insert(
            crate::core::automation::DURABLE_OBSERVATION_COMPARABLE_FIELD.to_string(),
            serde_json::Value::Bool(true),
        );
        target.insert(
            crate::core::automation::DURABLE_OBSERVATION_VALUES_FIELD.to_string(),
            serde_json::json!({
                "plan": "standard",
                "price": "10"
            }),
        );
        target.insert(
            crate::core::automation::DURABLE_OBSERVATION_BLOCKING_GAPS_FIELD.to_string(),
            serde_json::Value::Array(Vec::new()),
        );

        let mut payload = serde_json::Map::new();
        payload.insert(
            crate::core::automation::DURABLE_OBSERVATION_RESULT_FIELD.to_string(),
            serde_json::Value::Bool(true),
        );
        payload.insert(
            crate::core::automation::DURABLE_OBSERVATION_READY_FIELD.to_string(),
            serde_json::Value::Bool(true),
        );
        payload.insert(
            crate::core::automation::DURABLE_OBSERVATION_TARGETS_FIELD.to_string(),
            serde_json::Value::Array(vec![serde_json::Value::Object(target)]),
        );
        serde_json::Value::Object(payload)
    }

    #[test]
    fn watcher_poll_timeout_uses_default_without_policy() {
        let watcher = test_watcher_with_poll_arguments(serde_json::json!({}));

        assert_eq!(
            watcher_poll_timeout_secs(&watcher),
            (*WATCHER_POLL_TIMEOUT_SECS).min(sentinel_watcher_poll_timeout_cap_secs())
        );
    }

    #[test]
    fn watcher_poll_timeout_honors_stall_policy_with_sentinel_cap() {
        let cap = sentinel_watcher_poll_timeout_cap_secs();
        let requested = cap.saturating_sub(5).max(*WATCHER_POLL_TIMEOUT_SECS + 1);
        let watcher = test_watcher_with_poll_arguments(serde_json::json!({
            "_automation": {
                "policy": {
                    "max_attempts": 3,
                    "stall_timeout_secs": requested,
                    "retry_backoff_secs": 60,
                    "validation": {"mode": "none"}
                }
            }
        }));

        assert_eq!(watcher_poll_timeout_secs(&watcher), requested.min(cap));

        let capped = test_watcher_with_poll_arguments(serde_json::json!({
            "_automation": {
                "policy": {
                    "max_attempts": 3,
                    "stall_timeout_secs": cap.saturating_mul(10).max(cap + 1),
                    "retry_backoff_secs": 60,
                    "validation": {"mode": "none"}
                }
            }
        }));

        assert_eq!(watcher_poll_timeout_secs(&capped), cap);
    }

    #[test]
    fn semantic_watcher_readiness_rejects_unstructured_snapshot() {
        let watcher = test_watcher_with_poll_arguments(serde_json::json!({
            "semantic_watcher_poll": true,
        }));

        let error = semantic_watcher_poll_readiness_error(
            &watcher,
            "## Poll snapshot\n\nThe page was reachable, but exact values were not captured.",
        )
        .expect("unstructured semantic snapshots must not become baselines");

        assert!(error.contains("structured readiness"));
        assert!(error.contains(crate::core::automation::DURABLE_OBSERVATION_RESULT_FIELD));
    }

    #[test]
    fn semantic_watcher_readiness_accepts_comparable_observations() {
        let watcher = test_watcher_with_poll_arguments(serde_json::json!({
            "semantic_watcher_poll": true,
        }));
        let payload = comparable_observation_result();
        let result = payload.to_string();

        assert!(semantic_watcher_poll_readiness_error(&watcher, &result).is_none());
    }

    #[test]
    fn semantic_watcher_readiness_applies_to_declared_observation_contract() {
        let watcher = test_watcher_with_poll_arguments(serde_json::json!({
            "execution_contract": {
                "result_contract": crate::core::automation::durable_observation_result_contract()
            }
        }));

        let error = semantic_watcher_poll_readiness_error(
            &watcher,
            "A prose-only result cannot be used as a durable observation.",
        )
        .expect("declared durable observation contract must be enforced");

        assert!(error.contains(crate::core::automation::DURABLE_OBSERVATION_RESULT_FIELD));
    }

    #[test]
    fn semantic_watcher_readiness_rejects_incomparable_targets() {
        let watcher = test_watcher_with_poll_arguments(serde_json::json!({
            "semantic_watcher_poll": true,
        }));
        let mut payload = comparable_observation_result();
        payload[crate::core::automation::DURABLE_OBSERVATION_TARGETS_FIELD][0]
            [crate::core::automation::DURABLE_OBSERVATION_COMPARABLE_FIELD] =
            serde_json::Value::Bool(false);
        payload[crate::core::automation::DURABLE_OBSERVATION_TARGETS_FIELD][0]
            [crate::core::automation::DURABLE_OBSERVATION_VALUES_FIELD] = serde_json::json!({});
        payload[crate::core::automation::DURABLE_OBSERVATION_TARGETS_FIELD][0]
            [crate::core::automation::DURABLE_OBSERVATION_BLOCKING_GAPS_FIELD] =
            serde_json::json!(["exact observed values were not captured"]);
        let result = payload.to_string();

        let error = semantic_watcher_poll_readiness_error(&watcher, &result)
            .expect("incomparable target must not become a baseline");

        assert!(error.contains("target"));
        assert!(error.contains("comparable"));
    }

    #[test]
    fn sentinel_job_lease_and_status_keys_are_namespaced_and_stable() {
        assert_eq!(
            sentinel_job_lease_key("watchers"),
            "sentinel_job_lease_v1:watchers"
        );
        assert_eq!(
            sentinel_job_status_key("watchers"),
            "sentinel_job_status_v1:watchers"
        );
        assert_eq!(
            sentinel_job_lease_key("storage housekeeping"),
            "sentinel_job_lease_v1:storage_housekeeping"
        );
    }

    #[test]
    fn pulse_event_app_ids_collect_snapshot_and_doctor_refs() {
        let event = PulseEvent {
            timestamp: chrono::Utc::now().to_rfc3339(),
            status: "error".to_string(),
            message: "stale app".to_string(),
            summary: String::new(),
            flags: vec![],
            overdue_tasks: 0,
            failed_tasks: 0,
            details: PulseDetails {
                deployed_apps: vec![AppPulseInfo {
                    id: "cad20c5e".to_string(),
                    title: "arXiv".to_string(),
                    is_static: true,
                    process_alive: false,
                    requests_since_last_check: 0,
                    idle_hours: 0,
                }],
                doctor_findings: vec![DoctorFinding {
                    severity: "high".to_string(),
                    category: "app".to_string(),
                    target: "/apps/becf46bb/".to_string(),
                    title: "Restart app".to_string(),
                    evidence: String::new(),
                    root_cause: String::new(),
                    fix_command: "POST /api/apps/becf46bb/restart".to_string(),
                    remediation: Some(DoctorRemediationSpec::AppRestart {
                        app_id: "becf46bb".to_string(),
                    }),
                    user_actionable: true,
                }],
                ..PulseDetails::default()
            },
        };

        let ids = pulse_event_app_ids(&event);
        assert!(ids.contains("cad20c5e"));
        assert!(ids.contains("becf46bb"));
        assert_eq!(ids.len(), 2);
    }

    #[test]
    fn arkpulse_auth_failures_alone_are_not_security_incidents() {
        let thresholds = PulseSecurityThresholds {
            auth_failures: 2,
            rate_limit_hits: 10,
            unauthorized_channel: 10,
            combined: 12,
        };
        let auth_only = crate::core::SecuritySnapshot {
            auth_failures: 200,
            rate_limit_hits: 0,
            unauthorized_channel_attempts: 0,
            injection_attempts: 0,
        };
        assert!(!is_security_incident(Some(&auth_only), thresholds));

        let rate_limited = crate::core::SecuritySnapshot {
            rate_limit_hits: 10,
            ..auth_only.clone()
        };
        assert!(is_security_incident(Some(&rate_limited), thresholds));

        let mixed_spike = crate::core::SecuritySnapshot {
            auth_failures: 11,
            rate_limit_hits: 1,
            ..auth_only
        };
        assert!(is_security_incident(Some(&mixed_spike), thresholds));
    }

    #[test]
    fn arkpulse_auth_doctor_findings_need_correlated_attack_to_push() {
        let auth_finding = DoctorFinding {
            severity: "critical".to_string(),
            category: "attack_surface".to_string(),
            target: "/api/apps".to_string(),
            title: "Public app surface exposed protected inventory endpoint".to_string(),
            evidence: "GET /api/apps over public surface returned 200".to_string(),
            root_cause: "Sensitive management endpoint is reachable without auth.".to_string(),
            fix_command: "Require auth middleware for remotely reachable management routes"
                .to_string(),
            remediation: None,
            user_actionable: true,
        };
        assert!(doctor_finding_requires_correlated_attack(&auth_finding));
        assert!(!doctor_finding_counts_for_critical_alert(
            &auth_finding,
            false
        ));
        assert!(doctor_finding_counts_for_critical_alert(
            &auth_finding,
            true
        ));

        let secret_finding = DoctorFinding {
            severity: "critical".to_string(),
            category: "secrets".to_string(),
            title: "Potential secret exposure".to_string(),
            user_actionable: true,
            ..DoctorFinding::default()
        };
        assert!(doctor_finding_counts_for_critical_alert(
            &secret_finding,
            false
        ));
    }

    #[test]
    fn arkpulse_critical_message_omits_auth_fix_without_attack_signal() {
        let auth_finding = DoctorFinding {
            severity: "critical".to_string(),
            category: "attack_surface".to_string(),
            target: "/api/apps".to_string(),
            title: "Public app surface exposed protected inventory endpoint".to_string(),
            evidence: "GET /api/apps over public surface returned 200".to_string(),
            root_cause: "Sensitive management endpoint is reachable without auth.".to_string(),
            fix_command: "Require auth middleware for remotely reachable management routes"
                .to_string(),
            remediation: None,
            user_actionable: true,
        };
        let (message, flags) = build_critical_notification(
            2,
            0,
            0,
            None,
            PulseSecurityThresholds::default(),
            &[auth_finding],
        );
        assert!(message.contains("2 failed task(s)"));
        assert!(!message.contains("Require auth middleware"));
        assert!(!flags.contains(&"doctor_high".to_string()));
    }
}

struct SentinelMaintenanceGuard;

impl Drop for SentinelMaintenanceGuard {
    fn drop(&mut self) {
        SENTINEL_MAINTENANCE_RUNNING.store(false, Ordering::Release);
    }
}

pub fn is_pulse_running() -> bool {
    PULSE_RUNNING.load(Ordering::Relaxed)
}

fn try_start_sentinel_maintenance() -> Option<SentinelMaintenanceGuard> {
    SENTINEL_MAINTENANCE_RUNNING
        .compare_exchange(false, true, Ordering::AcqRel, Ordering::Acquire)
        .ok()
        .map(|_| SentinelMaintenanceGuard)
}

async fn sentinel_under_load(agent: &SharedAgent) -> bool {
    let (
        tasks,
        watcher_manager,
        browser_sessions,
        app_registry,
        runtime,
        last_activity,
        active_message_requests,
    ) = {
        let agent_guard = agent.read().await;
        (
            agent_guard.tasks.clone(),
            agent_guard.watcher_manager.clone(),
            agent_guard.browser_sessions.clone(),
            agent_guard.app_registry.clone(),
            agent_guard.runtime.clone(),
            agent_guard.last_activity_at(),
            agent_guard.active_message_request_count(),
        )
    };

    let pending_tasks = {
        let tasks = tasks.read().await;
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

    let watcher_count = watcher_manager
        .list()
        .await
        .into_iter()
        .filter(|w| {
            matches!(
                w.status,
                crate::core::automation::watcher::WatcherStatus::Active
            )
        })
        .count();
    let browser_sessions = browser_sessions.active_count();
    let running_apps = app_registry
        .list()
        .await
        .into_iter()
        .filter(|v| v.get("running").and_then(|x| x.as_bool()).unwrap_or(false))
        .count();
    let active_sandbox_containers = runtime.active_container_count().await;
    let recent_user_activity = last_activity.is_some_and(|last| {
        let age_secs = (chrono::Utc::now() - last).num_seconds();
        age_secs >= 0 && age_secs < *SENTINEL_RECENT_ACTIVITY_BUSY_SECS
    });

    active_message_requests > 0
        || recent_user_activity
        || active_sandbox_containers > 0
        || pending_tasks > 25
        || watcher_count > 30
        || browser_sessions >= 2
        || running_apps > 12
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
            let Some(_maintenance_guard) = try_start_sentinel_maintenance() else {
                if defers >= max_defers {
                    tracing::debug!(
                        "Sentinel: {} skipped (another maintenance job still active after {} defers)",
                        label,
                        max_defers
                    );
                    return;
                }

                defers += 1;
                tracing::debug!(
                    "Sentinel: {} waiting for another maintenance job; deferring {}/{} for {} minutes",
                    label,
                    defers,
                    max_defers,
                    defer_minutes
                );
                tokio::time::sleep(Duration::from_secs((defer_minutes * 60) as u64)).await;
                continue;
            };
            let Some((storage, lease_key, lease_guard)) =
                acquire_sentinel_job_lease(agent, label).await
            else {
                return;
            };
            tracing::debug!("Sentinel: {} started", label);
            let result =
                tokio::time::timeout(Duration::from_secs(*SENTINEL_JOB_TIMEOUT_SECS), job()).await;
            match result {
                Ok(()) => {
                    tracing::debug!("Sentinel: {} completed", label);
                    finish_sentinel_job_lease(
                        storage,
                        lease_key,
                        lease_guard,
                        label,
                        "completed",
                        None,
                    )
                    .await;
                }
                Err(_) => {
                    tracing::warn!(
                        "Sentinel: {} timed out after {}s",
                        label,
                        *SENTINEL_JOB_TIMEOUT_SECS
                    );
                    finish_sentinel_job_lease(
                        storage,
                        lease_key,
                        lease_guard,
                        label,
                        "timed_out",
                        Some("job timed out"),
                    )
                    .await;
                }
            }
            return;
        }

        if defers >= max_defers {
            tracing::debug!(
                "Sentinel: {} skipped (busy after {} defers)",
                label,
                max_defers
            );
            return;
        }

        defers += 1;
        tracing::debug!(
            "Sentinel: {} busy; deferring {}/{} for {} minutes",
            label,
            defers,
            max_defers,
            defer_minutes
        );
        tokio::time::sleep(Duration::from_secs((defer_minutes * 60) as u64)).await;
    }
}

async fn acquire_sentinel_job_lease(
    agent: &SharedAgent,
    label: &str,
) -> Option<(
    crate::storage::Storage,
    String,
    crate::storage::KvLeaseGuard,
)> {
    let storage = { agent.read().await.storage.clone() };
    let lease_key = sentinel_job_lease_key(label);
    let owner_id = sentinel_job_owner_id(label);
    match storage
        .acquire_kv_lease_guard(&lease_key, &owner_id, sentinel_job_lease_ttl_secs())
        .await
    {
        Ok(Some(guard)) => {
            record_sentinel_job_status(&storage, label, "running", Some(&guard), None).await;
            Some((storage, lease_key, guard))
        }
        Ok(None) => {
            tracing::debug!(
                "Sentinel: {} skipped because another process holds its lease",
                label
            );
            None
        }
        Err(error) => {
            tracing::warn!("Sentinel: {} lease acquisition failed: {}", label, error);
            None
        }
    }
}

async fn finish_sentinel_job_lease(
    storage: crate::storage::Storage,
    lease_key: String,
    lease_guard: crate::storage::KvLeaseGuard,
    label: &str,
    status: &str,
    error: Option<&str>,
) {
    record_sentinel_job_status(&storage, label, status, Some(&lease_guard), error).await;
    if let Err(release_error) = storage
        .release_kv_lease_guard(&lease_key, &lease_guard)
        .await
    {
        tracing::warn!(
            "Sentinel: {} failed to release job lease '{}': {}",
            label,
            lease_key,
            release_error
        );
    }
}

async fn record_sentinel_job_status(
    storage: &crate::storage::Storage,
    label: &str,
    status: &str,
    guard: Option<&crate::storage::KvLeaseGuard>,
    error: Option<&str>,
) {
    let record = SentinelJobStatusRecord {
        label: label.to_string(),
        status: status.to_string(),
        owner_id: guard.map(|value| value.owner_id.clone()),
        fence_token: guard.map(|value| value.fence_token),
        updated_at: chrono::Utc::now().to_rfc3339(),
        error: error.map(|value| value.to_string()),
    };
    match serde_json::to_vec(&record) {
        Ok(raw) => {
            if let Err(error) = storage.set(&sentinel_job_status_key(label), &raw).await {
                tracing::debug!(
                    "Sentinel: failed to persist {} job status: {}",
                    label,
                    error
                );
            }
        }
        Err(error) => {
            tracing::debug!("Sentinel: failed to encode {} job status: {}", label, error);
        }
    }
}

async fn run_leased_loop_with_timeout<F>(agent: &SharedAgent, label: &str, future: F)
where
    F: Future<Output = ()>,
{
    let Some((storage, lease_key, lease_guard)) = acquire_sentinel_job_lease(agent, label).await
    else {
        return;
    };
    if tokio::time::timeout(Duration::from_secs(*SENTINEL_JOB_TIMEOUT_SECS), future)
        .await
        .is_err()
    {
        tracing::warn!(
            "Sentinel: {} loop timed out after {}s",
            label,
            *SENTINEL_JOB_TIMEOUT_SECS
        );
        finish_sentinel_job_lease(
            storage,
            lease_key,
            lease_guard,
            label,
            "timed_out",
            Some("loop timed out"),
        )
        .await;
    } else {
        finish_sentinel_job_lease(storage, lease_key, lease_guard, label, "completed", None).await;
    }
}

async fn run_storage_housekeeping(agent: &SharedAgent) {
    let storage = { agent.read().await.storage.clone() };
    if let Err(error) = storage.run_housekeeping_purge().await {
        tracing::warn!("Sentinel: storage housekeeping failed: {}", error);
    }
}

#[derive(Debug, Clone, Copy, Default)]
struct KnowledgeStoreCounts {
    facts: u64,
    documents: u64,
    document_chunks: u64,
}

const KNOWLEDGE_FACT_WARN_THRESHOLD: u64 = 10_000;
const KNOWLEDGE_FACT_HIGH_THRESHOLD: u64 = 50_000;
const KNOWLEDGE_DOCUMENT_WARN_THRESHOLD: u64 = 500;
const KNOWLEDGE_DOCUMENT_HIGH_THRESHOLD: u64 = 2_000;
const KNOWLEDGE_DOCUMENT_CHUNK_WARN_THRESHOLD: u64 = 25_000;
const KNOWLEDGE_DOCUMENT_CHUNK_HIGH_THRESHOLD: u64 = 100_000;

const MAX_PULSE_EVENTS: usize = 100;
const MAX_PULSE_EVENT_AGE_DAYS: i64 = 30;
const ARKPULSE_LAST_RUN_AT_KEY: &str = "arkpulse_last_run_at";
const ARKPULSE_CRITICAL_LAST_SIG_KEY: &str = "arkpulse_critical_last_sig_v1";
const ARKPULSE_CRITICAL_LAST_SENT_KEY: &str = "arkpulse_critical_last_sent_v1";
const ARKPULSE_CRITICAL_NOTIFY_COOLDOWN_SECS: i64 = 24 * 3600;
const ARKPULSE_GROWTH_LAST_SIG_KEY: &str = "arkpulse_growth_last_sig_v1";
const ARKPULSE_GROWTH_LAST_SENT_KEY: &str = "arkpulse_growth_last_sent_v1";
const ARKPULSE_GROWTH_NOTIFY_COOLDOWN_SECS: i64 = 7 * 24 * 3600;
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
    let last_sig = storage
        .get(ARKPULSE_CRITICAL_LAST_SIG_KEY)
        .await
        .ok()
        .flatten()
        .and_then(|raw| String::from_utf8(raw).ok())
        .unwrap_or_default();
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

    if !signature.is_empty()
        && signature == last_sig
        && elapsed < ARKPULSE_CRITICAL_NOTIFY_COOLDOWN_SECS
    {
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

async fn should_emit_arkpulse_growth_notification(
    storage: &crate::storage::Storage,
    signature: &str,
) -> bool {
    let signature = normalize_arkpulse_alert_signature(signature);
    let now_ts = chrono::Utc::now().timestamp();
    let last_sig = storage
        .get(ARKPULSE_GROWTH_LAST_SIG_KEY)
        .await
        .ok()
        .flatten()
        .and_then(|raw| String::from_utf8(raw).ok())
        .unwrap_or_default();
    let last_sent_ts = storage
        .get(ARKPULSE_GROWTH_LAST_SENT_KEY)
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

    if !signature.is_empty()
        && signature == last_sig
        && elapsed < ARKPULSE_GROWTH_NOTIFY_COOLDOWN_SECS
    {
        return false;
    }

    if !signature.is_empty() {
        let _ = storage
            .set(ARKPULSE_GROWTH_LAST_SIG_KEY, signature.as_bytes())
            .await;
    }
    let _ = storage
        .set(ARKPULSE_GROWTH_LAST_SENT_KEY, now_ts.to_string().as_bytes())
        .await;
    true
}

fn knowledge_store_counts_summary(counts: &KnowledgeStoreCounts) -> String {
    format!(
        "{} facts, {} documents, {} chunks",
        counts.facts, counts.documents, counts.document_chunks
    )
}

fn knowledge_store_growth_reasons(counts: &KnowledgeStoreCounts, high: bool) -> Vec<String> {
    let mut reasons = Vec::new();
    let (fact_threshold, document_threshold, chunk_threshold) = if high {
        (
            KNOWLEDGE_FACT_HIGH_THRESHOLD,
            KNOWLEDGE_DOCUMENT_HIGH_THRESHOLD,
            KNOWLEDGE_DOCUMENT_CHUNK_HIGH_THRESHOLD,
        )
    } else {
        (
            KNOWLEDGE_FACT_WARN_THRESHOLD,
            KNOWLEDGE_DOCUMENT_WARN_THRESHOLD,
            KNOWLEDGE_DOCUMENT_CHUNK_WARN_THRESHOLD,
        )
    };
    if counts.facts >= fact_threshold {
        reasons.push(format!("facts={}", counts.facts));
    }
    if counts.documents >= document_threshold {
        reasons.push(format!("documents={}", counts.documents));
    }
    if counts.document_chunks >= chunk_threshold {
        reasons.push(format!("chunks={}", counts.document_chunks));
    }
    reasons
}

fn knowledge_store_growth_severity(counts: &KnowledgeStoreCounts) -> Option<&'static str> {
    if !knowledge_store_growth_reasons(counts, true).is_empty() {
        Some("high")
    } else if !knowledge_store_growth_reasons(counts, false).is_empty() {
        Some("medium")
    } else {
        None
    }
}

fn build_knowledge_store_health_check(counts: &KnowledgeStoreCounts) -> HealthCheck {
    match knowledge_store_growth_severity(counts) {
        Some("high") => HealthCheck {
            service: "Knowledge store".to_string(),
            status: "warn".to_string(),
            message: format!(
                "Large durable knowledge footprint: {}",
                knowledge_store_counts_summary(counts)
            ),
        },
        Some("medium") => HealthCheck {
            service: "Knowledge store".to_string(),
            status: "warn".to_string(),
            message: format!(
                "Knowledge growth worth reviewing: {}",
                knowledge_store_counts_summary(counts)
            ),
        },
        _ => HealthCheck {
            service: "Knowledge store".to_string(),
            status: "ok".to_string(),
            message: knowledge_store_counts_summary(counts),
        },
    }
}

fn build_knowledge_growth_notification(findings: &[DoctorFinding]) -> Option<(String, String)> {
    let relevant: Vec<&DoctorFinding> = findings
        .iter()
        .filter(|finding| {
            finding.category == "resource"
                && finding.target == "knowledge_store"
                && finding.severity == "high"
        })
        .collect();
    if relevant.is_empty() {
        return None;
    }

    let mut signature_parts = relevant
        .iter()
        .map(|finding| format!("{}:{}:{}", finding.severity, finding.title, finding.target))
        .collect::<Vec<_>>();
    signature_parts.sort();
    signature_parts.dedup();
    let signature = signature_parts.join("|");

    let detail = relevant
        .iter()
        .map(|finding| finding.evidence.clone())
        .collect::<Vec<_>>()
        .join(" | ");
    let body = format!(
        "AgentArk kept your documents and memories intact, but the durable knowledge store is getting large ({detail}). Open Pulse to review capacity and Postgres maintenance before latency or backup times drift."
    );
    Some((signature, body))
}

fn parse_utc_rfc3339(value: &str) -> Option<DateTime<Utc>> {
    DateTime::parse_from_rfc3339(value.trim())
        .ok()
        .map(|value| value.with_timezone(&Utc))
}

fn managed_artifact_apps_from_rows(
    app_rows: &[serde_json::Value],
    data_dir: &Path,
) -> Vec<crate::core::platform::artifact_hygiene::ManagedArtifactApp> {
    app_rows
        .iter()
        .filter_map(|row| {
            let id = row.get("id")?.as_str()?.trim().to_string();
            if id.is_empty() {
                return None;
            }
            let title = row
                .get("title")
                .and_then(|value| value.as_str())
                .unwrap_or(&id)
                .to_string();
            let app_dir = row
                .get("app_dir")
                .and_then(|value| value.as_str())
                .map(PathBuf::from)
                .unwrap_or_else(|| data_dir.join("apps").join(&id));
            let created_at = row
                .get("created_at")
                .and_then(|value| value.as_str())
                .and_then(parse_utc_rfc3339);
            Some(
                crate::core::platform::artifact_hygiene::ManagedArtifactApp {
                    id,
                    title,
                    app_dir,
                    enabled: row
                        .get("enabled")
                        .and_then(|value| value.as_bool())
                        .unwrap_or(true),
                    running: row
                        .get("running")
                        .and_then(|value| value.as_bool())
                        .unwrap_or(false),
                    is_static: row
                        .get("is_static")
                        .and_then(|value| value.as_bool())
                        .unwrap_or(false),
                    created_at,
                },
            )
        })
        .collect()
}

fn artifact_cleanup_summary(
    candidates: &[crate::core::platform::artifact_hygiene::ArtifactCleanupCandidate],
) -> String {
    crate::core::platform::artifact_hygiene::candidate_category_counts(candidates)
        .into_iter()
        .map(|(label, count, bytes)| {
            format!(
                "{}: {} item(s), {:.1} MB",
                label,
                count,
                bytes as f64 / (1024.0 * 1024.0)
            )
        })
        .collect::<Vec<_>>()
        .join(" | ")
}

fn lifecycle_retention_warnings(
    lifecycle: &crate::core::runtime::data_lifecycle::DataLifecycleSettings,
) -> Vec<String> {
    let mut warnings = Vec::new();
    if !lifecycle.cleanup_enabled {
        warnings.push("data lifecycle cleanup is disabled".to_string());
    }
    if !lifecycle.logs_cleanup_enabled {
        warnings.push("log/history cleanup is disabled".to_string());
    }
    let loose = [
        (
            "execution traces",
            lifecycle.execution_trace_retention_days,
            180,
        ),
        (
            "operational logs",
            lifecycle.operational_log_retention_days,
            180,
        ),
        ("security logs", lifecycle.security_log_retention_days, 365),
        ("LLM usage", lifecycle.llm_usage_retention_days, 365),
        (
            "terminal tasks",
            lifecycle.terminal_task_retention_days,
            365,
        ),
        (
            "execution runs",
            lifecycle.execution_run_retention_days,
            365,
        ),
        (
            "browser sessions",
            lifecycle.browser_session_retention_days,
            180,
        ),
        (
            "automation runs",
            lifecycle.automation_run_retention_days,
            365,
        ),
    ];
    for (label, days, threshold) in loose {
        if days > threshold {
            warnings.push(format!("{} retained for {} days", label, days));
        }
    }
    warnings
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

fn websocket_base_from_http_base(http_base: &str) -> String {
    if let Some(rest) = http_base.strip_prefix("https://") {
        format!("wss://{}", rest)
    } else if let Some(rest) = http_base.strip_prefix("http://") {
        format!("ws://{}", rest)
    } else {
        http_base.to_string()
    }
}

fn default_http_base_for_bind_addr(bind_addr: &str) -> Option<String> {
    let trimmed = bind_addr.trim();
    if trimmed.is_empty() {
        return None;
    }
    let normalized = if trimmed.starts_with("0.0.0.0:") {
        trimmed.replacen("0.0.0.0", "127.0.0.1", 1)
    } else if trimmed == "0.0.0.0" {
        "127.0.0.1".to_string()
    } else if trimmed.starts_with("[::]:") {
        trimmed.replacen("[::]", "127.0.0.1", 1)
    } else if trimmed == "[::]" || trimmed == "::" {
        "127.0.0.1".to_string()
    } else {
        trimmed.to_string()
    };
    Some(format!("http://{}", normalized.trim_end_matches('/')))
}

fn public_base_url_is_local(base_url: &str) -> bool {
    let Ok(parsed) = url::Url::parse(base_url) else {
        return false;
    };
    matches!(
        parsed
            .host_str()
            .unwrap_or_default()
            .to_ascii_lowercase()
            .as_str(),
        "localhost" | "127.0.0.1" | "::1"
    )
}

/// Append a pulse event to the persistent log (capped at MAX_PULSE_EVENTS)
async fn log_pulse_event(storage: &crate::storage::Storage, event: PulseEvent) {
    migrate_legacy_pulse_log_storage(storage).await;

    let Some(row) = pulse_event_to_row(&event) else {
        tracing::warn!("Pulse event could not be serialized for persistent storage");
        return;
    };
    if let Err(error) = storage.insert_arkpulse_event(&row).await {
        tracing::warn!("Failed to persist Pulse event row: {}", error);
        return;
    }
    prune_pulse_event_rows(storage).await;
}

/// Get the Pulse log from storage
pub async fn get_pulse_log(agent: &Agent) -> Vec<PulseEvent> {
    migrate_legacy_pulse_log(agent).await;

    match agent
        .storage
        .list_arkpulse_events(MAX_PULSE_EVENTS as u64)
        .await
    {
        Ok(rows) => {
            let live_app_ids = live_app_ids_for_pulse(agent).await;
            let mut stale_event_ids = Vec::new();
            let mut events = Vec::new();
            for row in rows {
                let Some(event) = pulse_event_from_row(row.clone()) else {
                    continue;
                };
                if pulse_event_has_missing_app_reference(&event, &live_app_ids) {
                    stale_event_ids.push(row.id);
                    continue;
                }
                events.push(event);
            }
            if !stale_event_ids.is_empty() {
                if let Err(error) = agent
                    .storage
                    .delete_arkpulse_events_by_ids(&stale_event_ids)
                    .await
                {
                    tracing::warn!(
                        "Failed to prune stale Pulse app events while loading log: {}",
                        error
                    );
                }
            }
            events.sort_by(|left, right| left.timestamp.cmp(&right.timestamp));
            events
        }
        Err(error) => {
            tracing::warn!("Failed to load Pulse event rows: {}", error);
            Vec::new()
        }
    }
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

fn pulse_path_references_app(path: &str, app_id: &str) -> bool {
    let segments: Vec<&str> = path
        .trim()
        .trim_matches('/')
        .split('/')
        .filter(|segment| !segment.is_empty())
        .collect();
    match segments.as_slice() {
        ["apps", id, ..] => *id == app_id,
        ["api", "apps", id, ..] => *id == app_id,
        _ => false,
    }
}

fn pulse_target_app_id(target: &str) -> Option<String> {
    let trimmed = target.trim();
    if trimmed.is_empty() {
        return None;
    }
    if let Some(id) = trimmed.strip_prefix("app:") {
        let id = id.trim();
        return (!id.is_empty()).then(|| id.to_string());
    }
    if let Ok(url) = url::Url::parse(trimmed) {
        return match url
            .path()
            .trim()
            .trim_matches('/')
            .split('/')
            .filter(|segment| !segment.is_empty())
            .collect::<Vec<_>>()
            .as_slice()
        {
            ["apps", id, ..] => Some((*id).to_string()),
            ["api", "apps", id, ..] => Some((*id).to_string()),
            _ => None,
        };
    }
    match trimmed
        .trim_matches('/')
        .split('/')
        .filter(|segment| !segment.is_empty())
        .collect::<Vec<_>>()
        .as_slice()
    {
        ["apps", id, ..] => Some((*id).to_string()),
        ["api", "apps", id, ..] => Some((*id).to_string()),
        _ => None,
    }
}

fn pulse_target_references_app(target: &str, app_id: &str) -> bool {
    let trimmed = target.trim();
    if trimmed.is_empty() {
        return false;
    }
    if let Some(id) = trimmed.strip_prefix("app:") {
        return id == app_id;
    }
    if let Ok(url) = url::Url::parse(trimmed) {
        return pulse_path_references_app(url.path(), app_id);
    }
    pulse_path_references_app(trimmed, app_id)
}

fn doctor_finding_references_app(finding: &DoctorFinding, app_id: &str) -> bool {
    matches!(
        finding.remediation.as_ref(),
        Some(DoctorRemediationSpec::AppRestart { app_id: target_app_id }) if target_app_id == app_id
    ) || pulse_target_references_app(&finding.target, app_id)
}

fn doctor_finding_app_ids(finding: &DoctorFinding) -> HashSet<String> {
    let mut ids = HashSet::new();
    if let Some(DoctorRemediationSpec::AppRestart { app_id }) = finding.remediation.as_ref() {
        let trimmed = app_id.trim();
        if !trimmed.is_empty() {
            ids.insert(trimmed.to_string());
        }
    }
    if let Some(target_app_id) = pulse_target_app_id(&finding.target) {
        ids.insert(target_app_id);
    }
    ids
}

fn pulse_event_app_ids(event: &PulseEvent) -> HashSet<String> {
    let mut ids = event
        .details
        .deployed_apps
        .iter()
        .map(|app| app.id.trim())
        .filter(|app_id| !app_id.is_empty())
        .map(|app_id| app_id.to_string())
        .collect::<HashSet<_>>();
    for finding in &event.details.doctor_findings {
        ids.extend(doctor_finding_app_ids(finding));
    }
    ids
}

async fn live_app_ids_for_pulse(agent: &Agent) -> HashSet<String> {
    let mut ids = agent
        .app_registry
        .list()
        .await
        .into_iter()
        .filter_map(|row| {
            row.get("id")
                .and_then(|value| value.as_str())
                .map(|value| value.trim().to_string())
        })
        .filter(|app_id| !app_id.is_empty())
        .collect::<HashSet<_>>();

    let apps_dir = agent.data_dir().join("apps");
    if let Ok(mut entries) = tokio::fs::read_dir(&apps_dir).await {
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
            let valid_meta = match tokio::fs::read(&meta_path).await {
                Ok(bytes) => serde_json::from_slice::<serde_json::Value>(&bytes)
                    .map(|value| value.is_object())
                    .unwrap_or(false),
                Err(_) => false,
            };
            if valid_meta {
                ids.insert(app_id);
            }
        }
    }

    ids
}

fn pulse_event_has_missing_app_reference(
    event: &PulseEvent,
    live_app_ids: &HashSet<String>,
) -> bool {
    let referenced_app_ids = pulse_event_app_ids(event);
    !referenced_app_ids.is_empty()
        && referenced_app_ids
            .iter()
            .any(|app_id| !live_app_ids.contains(app_id))
}

fn pulse_event_references_app(event: &PulseEvent, app_id: &str) -> bool {
    pulse_event_app_ids(event).contains(app_id)
        || event
            .details
            .doctor_findings
            .iter()
            .any(|finding| doctor_finding_references_app(finding, app_id))
}

pub async fn delete_app_referenced_pulse_events(
    storage: &crate::storage::Storage,
    app_id: &str,
) -> anyhow::Result<u64> {
    let trimmed = app_id.trim();
    if trimmed.is_empty() {
        return Ok(0);
    }
    let rows = storage
        .list_arkpulse_events(MAX_PULSE_EVENTS as u64)
        .await?;
    let matching_ids = rows
        .into_iter()
        .filter_map(|row| {
            let event = pulse_event_from_row(row.clone())?;
            pulse_event_references_app(&event, trimmed).then_some(row.id)
        })
        .collect::<Vec<_>>();
    storage.delete_arkpulse_events_by_ids(&matching_ids).await
}

pub async fn find_stale_app_references_in_pulse_events(
    storage: &crate::storage::Storage,
    live_app_ids: &HashSet<String>,
) -> anyhow::Result<StalePulseAppReferenceReport> {
    let rows = storage
        .list_arkpulse_events(MAX_PULSE_EVENTS as u64)
        .await?;
    let mut report = StalePulseAppReferenceReport::default();
    for row in rows {
        let Some(event) = pulse_event_from_row(row.clone()) else {
            continue;
        };
        let referenced_app_ids = pulse_event_app_ids(&event);
        if referenced_app_ids.is_empty() {
            continue;
        }
        let missing_app_ids = referenced_app_ids
            .into_iter()
            .filter(|app_id| !live_app_ids.contains(app_id))
            .collect::<HashSet<_>>();
        if missing_app_ids.is_empty() {
            continue;
        }
        report.event_ids.push(row.id);
        report.missing_app_ids.extend(missing_app_ids);
    }
    Ok(report)
}

#[derive(Debug, Clone)]
struct AppEndpoint {
    id: String,
    title: String,
    is_static: bool,
    access_url: String,
    access_key: Option<String>,
    app_dir: PathBuf,
}

#[derive(Clone)]
struct PulseDoctorContext {
    storage: crate::storage::Storage,
    data_dir: PathBuf,
    allow_managed_backup_work: bool,
    app_registry: crate::actions::app::AppRegistry,
    config: crate::core::runtime::config::AgentConfig,
    embedding_client: Option<Arc<crate::core::EmbeddingClient>>,
    model_pool: HashMap<
        String,
        (
            crate::core::runtime::config::ModelSlot,
            crate::core::LlmClient,
        ),
    >,
    primary_model_id: String,
    llm: crate::core::LlmClient,
    api_key: Option<String>,
}

fn pulse_event_storage_id(event: &PulseEvent) -> String {
    let mut hasher = DefaultHasher::new();
    event.timestamp.hash(&mut hasher);
    event.status.hash(&mut hasher);
    event.message.hash(&mut hasher);
    event.summary.hash(&mut hasher);
    event.flags.hash(&mut hasher);
    event.overdue_tasks.hash(&mut hasher);
    event.failed_tasks.hash(&mut hasher);
    let hash = hasher.finish();
    format!("arkpulse-{hash:016x}")
}

fn pulse_event_to_row(event: &PulseEvent) -> Option<crate::storage::arkpulse_event::Model> {
    let flags_json = serde_json::to_string(&event.flags).ok()?;
    let details_json = serde_json::to_string(&event.details).ok()?;
    Some(crate::storage::arkpulse_event::Model {
        id: pulse_event_storage_id(event),
        timestamp: event.timestamp.clone(),
        status: event.status.clone(),
        message: event.message.clone(),
        summary: event.summary.clone(),
        flags_json,
        overdue_tasks: event.overdue_tasks.min(i32::MAX as usize) as i32,
        failed_tasks: event.failed_tasks.min(i32::MAX as usize) as i32,
        details_json,
    })
}

fn pulse_event_from_row(row: crate::storage::arkpulse_event::Model) -> Option<PulseEvent> {
    Some(PulseEvent {
        timestamp: row.timestamp,
        status: row.status,
        message: row.message,
        summary: row.summary,
        flags: serde_json::from_str(&row.flags_json).ok()?,
        overdue_tasks: row.overdue_tasks.max(0) as usize,
        failed_tasks: row.failed_tasks.max(0) as usize,
        details: serde_json::from_str(&row.details_json).ok()?,
    })
}

async fn prune_pulse_event_rows(storage: &crate::storage::Storage) {
    let cutoff =
        (chrono::Utc::now() - chrono::Duration::days(MAX_PULSE_EVENT_AGE_DAYS)).to_rfc3339();
    if let Err(error) = storage.delete_arkpulse_events_before(&cutoff).await {
        tracing::warn!("Failed to prune stale Pulse rows: {}", error);
    }
    match storage
        .list_arkpulse_event_ids_beyond_latest(MAX_PULSE_EVENTS as u64)
        .await
    {
        Ok(extra_ids) if !extra_ids.is_empty() => {
            if let Err(error) = storage.delete_arkpulse_events_by_ids(&extra_ids).await {
                tracing::warn!("Failed to prune excess Pulse rows: {}", error);
            }
        }
        Ok(_) => {}
        Err(error) => {
            tracing::warn!("Failed to enumerate excess Pulse rows: {}", error);
        }
    }
}

async fn migrate_legacy_pulse_log_storage(storage: &crate::storage::Storage) {
    let existing_rows = match storage.count_arkpulse_events().await {
        Ok(count) => count,
        Err(error) => {
            tracing::warn!("Failed to count Pulse history rows: {}", error);
            return;
        }
    };
    if existing_rows > 0 {
        return;
    }

    let Some(bytes) = storage.get_encrypted(PULSE_LOG_KEY).await.ok().flatten() else {
        return;
    };
    let Ok(raw_events) = serde_json::from_slice::<Vec<PulseEvent>>(&bytes) else {
        return;
    };
    let events = prune_pulse_events(raw_events);
    if events.is_empty() {
        let _ = storage.delete(PULSE_LOG_KEY).await;
        return;
    }

    for event in &events {
        let Some(row) = pulse_event_to_row(event) else {
            tracing::warn!("Skipping legacy Pulse event migration due to serialization failure");
            return;
        };
        if let Err(error) = storage.insert_arkpulse_event(&row).await {
            tracing::warn!("Failed to migrate legacy Pulse event row: {}", error);
            return;
        }
    }
    prune_pulse_event_rows(storage).await;
    let _ = storage.delete(PULSE_LOG_KEY).await;
}

async fn migrate_legacy_pulse_log(agent: &Agent) {
    migrate_legacy_pulse_log_storage(&agent.storage).await;
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

#[derive(Debug, Default)]
struct PulseDoctorReport {
    findings: Vec<DoctorFinding>,
    sections: Vec<PulseScanSection>,
}

fn pulse_metric(label: impl Into<String>, value: impl Into<String>) -> PulseScanMetric {
    PulseScanMetric {
        label: label.into(),
        value: value.into(),
    }
}

fn pulse_section_status_from_findings(findings: &[DoctorFinding]) -> &'static str {
    if findings.iter().any(|finding| {
        finding.user_actionable && matches!(finding.severity.as_str(), "critical" | "high")
    }) {
        "error"
    } else if findings.is_empty() {
        "ok"
    } else {
        "warning"
    }
}

fn highest_finding_severity(findings: &[DoctorFinding]) -> &'static str {
    match findings
        .iter()
        .max_by_key(|finding| severity_weight(&finding.severity))
        .map(|finding| finding.severity.as_str())
    {
        Some("critical") => "critical",
        Some("high") => "high",
        Some("medium") => "medium",
        Some("low") => "low",
        Some("warning") => "warning",
        Some("info") => "info",
        Some("none") | None => "none",
        Some(_) => "warning",
    }
}

fn build_scan_section(
    id: &str,
    title: &str,
    duration: Duration,
    findings: &[DoctorFinding],
    ok_summary: impl Into<String>,
    detail: impl Into<String>,
    mut metrics: Vec<PulseScanMetric>,
) -> PulseScanSection {
    let actionable = findings
        .iter()
        .filter(|finding| finding.user_actionable)
        .count();
    let status = pulse_section_status_from_findings(findings).to_string();
    let summary = if findings.is_empty() {
        ok_summary.into()
    } else {
        let preview = findings
            .iter()
            .take(3)
            .map(|finding| finding.title.clone())
            .collect::<Vec<_>>()
            .join(" | ");
        format!(
            "{} finding{}: {}",
            findings.len(),
            if findings.len() == 1 { "" } else { "s" },
            preview
        )
    };
    let detail = if findings.is_empty() {
        detail.into()
    } else {
        let categories = findings
            .iter()
            .map(|finding| finding.category.as_str())
            .collect::<HashSet<_>>()
            .into_iter()
            .take(6)
            .collect::<Vec<_>>()
            .join(", ");
        let targets = findings
            .iter()
            .map(|finding| finding.target.as_str())
            .filter(|target| !target.trim().is_empty())
            .take(4)
            .collect::<Vec<_>>()
            .join(" | ");
        format!(
            "Highest severity: {}. Categories: {}. Targets: {}.",
            highest_finding_severity(findings),
            if categories.is_empty() {
                "none".to_string()
            } else {
                categories
            },
            if targets.is_empty() {
                "not provided".to_string()
            } else {
                targets
            }
        )
    };
    metrics.push(pulse_metric(
        "Duration",
        format!("{} ms", duration.as_millis()),
    ));
    metrics.push(pulse_metric("Findings", findings.len().to_string()));
    if !findings.is_empty() {
        metrics.push(pulse_metric("Actionable", actionable.to_string()));
        metrics.push(pulse_metric(
            "Highest severity",
            highest_finding_severity(findings),
        ));
    }
    PulseScanSection {
        id: id.to_string(),
        title: title.to_string(),
        status,
        summary,
        detail,
        duration_ms: duration.as_millis() as u64,
        metrics,
    }
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
        if k == "password" || k == "key" {
            Some(v.into_owned())
        } else {
            None
        }
    })
}

fn strip_access_key(access_url: &str) -> String {
    let mut parsed = if access_url.starts_with("http://") || access_url.starts_with("https://") {
        match url::Url::parse(access_url) {
            Ok(url) => url,
            Err(_) => return access_url.to_string(),
        }
    } else {
        match url::Url::parse(&format!("http://local{}", access_url)) {
            Ok(url) => url,
            Err(_) => return access_url.to_string(),
        }
    };
    let filtered: Vec<(String, String)> = parsed
        .query_pairs()
        .filter(|(key, _)| key != "password" && key != "key" && key != "grant")
        .map(|(key, value)| (key.into_owned(), value.into_owned()))
        .collect();
    if filtered.is_empty() {
        parsed.set_query(None);
    } else {
        let joined = filtered
            .iter()
            .map(|(key, value)| format!("{}={}", key, value))
            .collect::<Vec<_>>()
            .join("&");
        parsed.set_query(Some(&joined));
    }
    let mut value = parsed.path().to_string();
    if let Some(query) = parsed.query() {
        value.push('?');
        value.push_str(query);
    }
    value
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

fn should_descend_app_scan_entry(entry: &walkdir::DirEntry) -> bool {
    if !entry.file_type().is_dir() {
        return true;
    }
    let name = entry
        .file_name()
        .to_str()
        .unwrap_or_default()
        .to_ascii_lowercase();
    !matches!(
        name.as_str(),
        ".git"
            | ".hg"
            | ".svn"
            | ".next"
            | ".cache"
            | "node_modules"
            | "dist"
            | "build"
            | "target"
            | "vendor"
            | ".venv"
            | "venv"
            | "__pycache__"
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
                access_key: row
                    .get("access_password")
                    .or_else(|| row.get("access_key"))
                    .and_then(|v| v.as_str())
                    .map(str::trim)
                    .filter(|value| !value.is_empty())
                    .map(ToString::to_string),
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
        .filter_entry(should_descend_app_scan_entry)
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

async fn detect_ws_hint_async(app_dir: PathBuf) -> bool {
    match tokio::time::timeout(
        Duration::from_secs(5),
        tokio::task::spawn_blocking(move || detect_ws_hint(&app_dir)),
    )
    .await
    {
        Ok(Ok(has_hint)) => has_hint,
        Ok(Err(error)) => {
            tracing::warn!("Pulse WS hint worker failed: {}", error);
            false
        }
        Err(_) => {
            tracing::warn!("Pulse WS hint worker timed out after 5s");
            false
        }
    }
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
                            DoctorRemediationSpec::ManagedAppOperation {
                                app_id: app.id.clone(),
                                operation: DoctorManagedAppOperation::RemoveNpmInstallHooks,
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
                DoctorRemediationSpec::ManagedAppOperation {
                    app_id: app.id.clone(),
                    operation: DoctorManagedAppOperation::GenerateCargoLockfile,
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
                    fix_command,
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
                    DoctorRemediationSpec::ManagedAppOperation {
                        app_id: app.id.clone(),
                        operation: DoctorManagedAppOperation::CompilePythonRequirements,
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
                    fix_command,
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
                "cd {} && mv .env .env.backup && rotate exposed keys",
                app.app_dir.display()
            ),
        );
    }

    let mut scanned_files = 0usize;
    let mut hit_count = 0usize;
    for entry in walkdir::WalkDir::new(&app.app_dir)
        .into_iter()
        .filter_entry(should_descend_app_scan_entry)
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
                fix_command,
            );
        }
    }
}

async fn run_attack_surface_checks(
    http_client: &reqwest::Client,
    http_base: &str,
    has_deployed_apps: bool,
    configured_public_base_url: Option<&str>,
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
                format!(
                    "Verify {} HTTP server is running on port 8990",
                    crate::branding::PRODUCT_NAME
                ),
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

    if !has_deployed_apps {
        return;
    }

    let mut tunnel_status_req = http_client.get(format!("{}/tunnel/status", http_base));
    if let Some(key) = api_key.filter(|k| !k.trim().is_empty()) {
        tunnel_status_req = tunnel_status_req.bearer_auth(key);
    }

    let mut tunnel_active = false;
    let mut tunnel_url: Option<String> = None;
    let mut tunnel_url_present = false;
    match tunnel_status_req.send().await {
        Ok(resp) if resp.status().is_success() => {
            if let Ok(payload) = resp.json::<serde_json::Value>().await {
                tunnel_active = payload
                    .get("active")
                    .and_then(|v| v.as_bool())
                    .unwrap_or(false);
                tunnel_url = payload
                    .get("url")
                    .and_then(|v| v.as_str())
                    .map(str::trim)
                    .filter(|value| !value.is_empty())
                    .map(|value| value.trim_end_matches('/').to_string());
                tunnel_url_present = tunnel_url.is_some();
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
                "Pulse could not verify managed tunnel state from control plane.",
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
                "Pulse could not verify tunnel process state.",
                "Check local control plane reachability".to_string(),
            );
        }
    }

    let explicit_public_base = configured_public_base_url
        .map(|value| value.trim().trim_end_matches('/').to_string())
        .filter(|value| !value.is_empty())
        .filter(|value| !public_base_url_is_local(value));
    let managed_tunnel_base = tunnel_url.clone();
    let uses_managed_tunnel = explicit_public_base.is_none() && managed_tunnel_base.is_some();
    let public_probe_base = explicit_public_base
        .clone()
        .or_else(|| managed_tunnel_base.clone());

    let Some(public_base) = public_probe_base else {
        return;
    };

    let public_health_url = format!("{}/health", public_base);
    match http_client.get(&public_health_url).send().await {
        Ok(resp) if resp.status().is_success() => {}
        Ok(resp) => {
            if uses_managed_tunnel {
                push_finding!(
                    findings,
                    "high",
                    "attack_surface",
                    public_health_url.clone(),
                    "Managed tunnel health probe failed",
                    format!("GET {} returned {}", public_health_url, resp.status()),
                    "Tunnel endpoint is reachable but unhealthy for remote access traffic.",
                    "Restart tunnel and inspect cloudflared logs".to_string(),
                    DoctorRemediationSpec::TunnelRestartVerify,
                );
            } else {
                push_finding!(
                    findings,
                    "high",
                    "attack_surface",
                    public_health_url.clone(),
                    "Public app endpoint health probe failed",
                    format!("GET {} returned {}", public_health_url, resp.status()),
                    "Configured public app endpoint is reachable but unhealthy.",
                    "Verify public_apps.base_url and the upstream reverse proxy".to_string(),
                );
            }
        }
        Err(e) => {
            if uses_managed_tunnel {
                push_finding!(
                    findings,
                    "high",
                    "attack_surface",
                    public_health_url.clone(),
                    "Managed tunnel unreachable",
                    e.to_string(),
                    "Cannot reach service through the currently active managed tunnel URL.",
                    "Restart tunnel and verify DNS/TLS connectivity".to_string(),
                    DoctorRemediationSpec::TunnelRestartVerify,
                );
            } else {
                push_finding!(
                    findings,
                    "high",
                    "attack_surface",
                    public_health_url.clone(),
                    "Configured public app endpoint unreachable",
                    e.to_string(),
                    "Configured public app base URL is unreachable from the control plane.",
                    "Verify public_apps.base_url and external proxy/DNS reachability".to_string(),
                );
            }
        }
    }

    if uses_managed_tunnel && (!tunnel_active || !tunnel_url_present) {
        push_finding!(
            findings,
            "high",
            "attack_surface",
            "/tunnel/status",
            "Tunnel status degraded while apps are deployed",
            format!(
                "active={}, url_present={}",
                tunnel_active, tunnel_url_present
            ),
            "Managed tunnel should stay active while shared app access is in use.",
            "Restart the tunnel and confirm URL discovery".to_string(),
            DoctorRemediationSpec::TunnelRestartVerify,
        );
    }

    let public_apps_probe = format!("{}/api/apps", public_base);
    match http_client.get(&public_apps_probe).send().await {
        Ok(resp) => {
            let code = resp.status().as_u16();
            if code != 401 && code != 403 {
                push_finding!(
                    findings,
                    "critical",
                    "attack_surface",
                    public_apps_probe,
                    "Public app surface exposed protected inventory endpoint",
                    format!("GET /api/apps over public surface returned {}", code),
                    "Sensitive management endpoint is reachable from the shared app surface without auth.",
                    "Require auth middleware for remotely reachable management routes".to_string(),
                );
            }
        }
        Err(e) => {
            push_finding!(
                findings,
                "low",
                "attack_surface",
                "/api/apps",
                "Public app-inventory auth probe failed",
                e.to_string(),
                "Could not verify auth enforcement of /api/apps over the shared app surface.",
                "Retry when the shared app surface is stable".to_string(),
            );
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
        if let Some(session_cookie) = cookie_headers.iter().find(|c| {
            c.to_ascii_lowercase()
                .contains(&format!("{}=", crate::branding::SESSION_COOKIE_NAME))
        }) {
            let lower = session_cookie.to_ascii_lowercase();
            if !lower.contains("httponly") || !lower.contains("samesite") {
                push_finding!(
                    findings,
                    "high",
                    "runtime_hardening",
                    &format!("{} cookie", crate::branding::SESSION_COOKIE_NAME),
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
        let Some(key) = app
            .access_key
            .clone()
            .or_else(|| parse_access_key(&app.access_url))
        else {
            continue;
        };
        for payload in traversal_payloads {
            let traversal_url = format!("{}/apps/{}/{}", http_base, app.id, payload);
            if let Ok(resp) = http_client
                .get(&traversal_url)
                .header("x-agentark-app-password", key.clone())
                .send()
                .await
            {
                let status = resp.status();
                if status.is_success() {
                    let body = resp.text().await.unwrap_or_default();
                    if !body.to_lowercase().contains("agentark app guard") {
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

#[derive(Debug, Clone, Copy)]
struct BackendMemoryPressure {
    rss_bytes: Option<u64>,
    cgroup_current_bytes: Option<u64>,
    cgroup_max_bytes: Option<u64>,
}

fn read_u64_text(path: &str) -> Option<u64> {
    let raw = std::fs::read_to_string(path).ok()?;
    let trimmed = raw.trim();
    if trimmed.eq_ignore_ascii_case("max") {
        return None;
    }
    trimmed.parse::<u64>().ok()
}

fn read_process_rss_bytes() -> Option<u64> {
    let raw = std::fs::read_to_string("/proc/self/status").ok()?;
    raw.lines().find_map(|line| {
        let line = line.trim();
        let value = line.strip_prefix("VmRSS:")?.trim();
        let kb = value.split_whitespace().next()?.parse::<u64>().ok()?;
        Some(kb.saturating_mul(1024))
    })
}

fn read_backend_memory_pressure() -> BackendMemoryPressure {
    BackendMemoryPressure {
        rss_bytes: read_process_rss_bytes(),
        cgroup_current_bytes: read_u64_text("/sys/fs/cgroup/memory.current"),
        cgroup_max_bytes: read_u64_text("/sys/fs/cgroup/memory.max"),
    }
}

fn format_mb(bytes: u64) -> String {
    format!("{:.1} MB", bytes as f64 / (1024.0 * 1024.0))
}

async fn run_resource_checks(
    ctx: &PulseDoctorContext,
    deployed_apps: &[AppPulseInfo],
    security: Option<&crate::core::SecuritySnapshot>,
    security_thresholds: PulseSecurityThresholds,
    findings: &mut Vec<DoctorFinding>,
) {
    match ctx.storage.database_size_bytes().await {
        Ok(Some(size_bytes)) => {
            let size_mb = size_bytes as f64 / (1024.0 * 1024.0);
            if size_mb > 1024.0 {
                push_finding!(
                    findings,
                    "high",
                    "resource",
                    "postgres",
                    "Database size is very large",
                    format!("{:.1} MB", size_mb),
                    "Database growth can degrade query performance and backup times.",
                    "Archive old rows and review Postgres vacuum/backup cadence".to_string(),
                );
            } else if size_mb > 512.0 {
                push_finding!(
                    findings,
                    "medium",
                    "resource",
                    "postgres",
                    "Database size growth warning",
                    format!("{:.1} MB", size_mb),
                    "Storage growth trend may indicate missing retention policies.",
                    "Review retention windows for traces, logs, and notifications".to_string(),
                );
            }
        }
        Ok(None) => {}
        Err(error) => {
            push_finding!(
                findings,
                "medium",
                "resource",
                "postgres",
                "Could not read database size",
                error.to_string(),
                "Database growth could not be evaluated.",
                "Verify Postgres connectivity and permissions for size introspection".to_string(),
            );
        }
    }

    let knowledge_counts = match (
        ctx.storage.count_facts(None).await,
        ctx.storage.count_documents(None).await,
        ctx.storage.count_document_chunks().await,
    ) {
        (Ok(facts), Ok(documents), Ok(document_chunks)) => Some(KnowledgeStoreCounts {
            facts,
            documents,
            document_chunks,
        }),
        (fact_res, document_res, chunk_res) => {
            let mut errors = Vec::new();
            if let Err(error) = fact_res {
                errors.push(format!("facts: {}", error));
            }
            if let Err(error) = document_res {
                errors.push(format!("documents: {}", error));
            }
            if let Err(error) = chunk_res {
                errors.push(format!("chunks: {}", error));
            }
            tracing::warn!(
                "Knowledge store growth inspection failed: {}",
                errors.join(" | ")
            );
            push_finding!(
                findings,
                "medium",
                "resource",
                "knowledge_store",
                "Could not inspect durable knowledge growth",
                errors.join(" | "),
                "Storage growth could not be evaluated for durable documents and memories.",
                "Verify Postgres connectivity and count-query permissions, then review durable knowledge growth."
                    .to_string(),
            );
            None
        }
    };
    if let Some(counts) = knowledge_counts {
        match knowledge_store_growth_severity(&counts) {
            Some("high") => {
                let reasons = knowledge_store_growth_reasons(&counts, true).join(", ");
                tracing::warn!(
                    "Knowledge store growth warning (high): {} ({})",
                    knowledge_store_counts_summary(&counts),
                    reasons
                );
                push_finding!(
                    findings,
                    "high",
                    "resource",
                    "knowledge_store",
                    "Durable knowledge store is large",
                    format!("{} | thresholds crossed: {}", knowledge_store_counts_summary(&counts), reasons),
                    "Documents and memories are durable by design, so growth needs monitoring and Postgres capacity planning rather than silent deletion.",
                    "Open Pulse, review knowledge volume, and schedule Postgres vacuum/backup capacity checks".to_string(),
                );
            }
            Some("medium") => {
                let reasons = knowledge_store_growth_reasons(&counts, false).join(", ");
                tracing::info!(
                    "Knowledge store growth warning (medium): {} ({})",
                    knowledge_store_counts_summary(&counts),
                    reasons
                );
                push_finding!(
                    findings,
                    "medium",
                    "resource",
                    "knowledge_store",
                    "Durable knowledge growth warning",
                    format!("{} | planning thresholds crossed: {}", knowledge_store_counts_summary(&counts), reasons),
                    "Documents and memories are being retained as intended, but the knowledge store is large enough to justify capacity monitoring.",
                    "Review Pulse trends and plan Postgres maintenance before latency or backup times drift".to_string(),
                );
            }
            _ => {}
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

    let memory = read_backend_memory_pressure();
    if let (Some(current), Some(max)) = (memory.cgroup_current_bytes, memory.cgroup_max_bytes) {
        if max > 0 {
            let ratio = current as f64 / max as f64;
            if ratio >= 0.85 {
                push_finding!(
                    findings,
                    "high",
                    "resource",
                    "backend_memory",
                    "Backend memory pressure is high",
                    format!(
                        "cgroup memory is {} of {} ({:.0}%)",
                        format_mb(current),
                        format_mb(max),
                        ratio * 100.0
                    ),
                    "Runtime memory pressure can cause request latency, failed background jobs, or container eviction.",
                    "Review live traffic, running apps, and retention settings before raising memory limits".to_string(),
                );
            } else if ratio >= 0.70 {
                push_finding!(
                    findings,
                    "medium",
                    "resource",
                    "backend_memory",
                    "Backend memory pressure is elevated",
                    format!(
                        "cgroup memory is {} of {} ({:.0}%)",
                        format_mb(current),
                        format_mb(max),
                        ratio * 100.0
                    ),
                    "Sustained memory growth can degrade Pulse, background workers, and API responsiveness.",
                    "Review retention settings and recent runtime workload before memory pressure reaches eviction thresholds".to_string(),
                );
            }
        }
    } else if let Some(rss) = memory.rss_bytes {
        if rss >= 1536 * 1024 * 1024 {
            push_finding!(
                findings,
                "medium",
                "resource",
                "backend_memory",
                "Backend RSS is large",
                format!("process RSS is {}", format_mb(rss)),
                "Large resident memory can indicate runtime growth that deserves operator review.",
                "Inspect active background jobs, app runtimes, and retention posture".to_string(),
            );
        }
    }
}

async fn run_data_safety_checks(
    storage: &crate::storage::Storage,
    data_dir: &Path,
    allow_backup_work: bool,
    findings: &mut Vec<DoctorFinding>,
) -> String {
    let backup_status = match managed_backup::ensure_managed_postgres_backup(
        data_dir,
        managed_backup::ManagedBackupOptions { allow_backup_work },
    )
    .await
    {
        Ok(managed_backup::ManagedBackupOutcome::Fresh) => "fresh".to_string(),
        Ok(managed_backup::ManagedBackupOutcome::Created { path, size_bytes }) => {
            tracing::info!(
                target: "agentark::sentinel",
                path = %path.display(),
                size_bytes,
                "Managed backup was refreshed during data safety checks"
            );
            "created".to_string()
        }
        Ok(managed_backup::ManagedBackupOutcome::DeferredBusy) => "deferred_busy".to_string(),
        Ok(managed_backup::ManagedBackupOutcome::AlreadyRunning) => "already_running".to_string(),
        Err(error) => {
            push_finding!(
                findings,
                "critical",
                "data_safety",
                error.target,
                "Managed backup failed",
                error.evidence,
                "AgentArk could not create or refresh its framework-managed Postgres backup.",
                "Check the AgentArk data volume permissions and Postgres backup tooling; Pulse will retry automatically.".to_string(),
            );
            "failed".to_string()
        }
    };

    match storage.latest_migration_version().await {
        Ok(Some(version)) => {
            if version < 1 {
                push_finding!(
                    findings,
                    "critical",
                    "data_safety",
                    "postgres schema",
                    "Migration state is behind baseline",
                    format!("Latest migration version: {}", version),
                    "Expected Postgres schema baseline was not applied.",
                    "Run migrations before accepting traffic".to_string(),
                );
            }
        }
        Ok(None) => {
            // Postgres bootstrap currently does not persist schema versions in a migrations table.
            // Treat actual table discovery as the authoritative readiness signal instead.
        }
        Err(e) => {
            push_finding!(
                findings,
                "high",
                "data_safety",
                "postgres schema",
                "Migration status unavailable",
                e.to_string(),
                "Could not validate database migration state.",
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
        "execution_runs",
        "run_checkpoints",
        "tool_attempts",
        "watchers",
        "webhook_events",
        "arkpulse_events",
        "memory_capture_events",
        "memory_operations",
        "memory_evidence_links",
    ];
    match storage.database_table_names().await {
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
                    "postgres schema",
                    "Schema drift or failed migration",
                    format!("Missing tables: {}", missing.join(", ")),
                    "Expected storage schema is incomplete.",
                    "Run Postgres migrations and restore missing schema".to_string(),
                );
            }
        }
        Err(e) => {
            push_finding!(
                findings,
                "medium",
                "data_safety",
                "postgres schema",
                "Could not verify schema",
                e.to_string(),
                "Schema validation query failed.",
                "Check Postgres permissions and migration code path".to_string(),
            );
        }
    }

    match storage
        .count_memory_capture_events_by_statuses_all_scopes(&["failed"])
        .await
    {
        Ok(count) if count > 0 => {
            push_finding!(
                findings,
                "medium",
                "data_safety",
                "memory capture pipeline",
                "Failed memory captures detected",
                format!("{} memory capture event(s) are in failed state", count),
                "Some user facts may be missing from Memory until the capture pipeline is reviewed.",
                "Review failed memory captures and model health".to_string(),
                DoctorRemediationSpec::ReadonlyInvestigation {
                    topic: DoctorReadonlyInvestigationTopic::MemoryCaptureHealth,
                },
            );
        }
        Ok(_) => {}
        Err(e) => {
            push_finding!(
                findings,
                "low",
                "data_safety",
                "memory capture pipeline",
                "Could not inspect memory capture health",
                e.to_string(),
                "Memory capture audit state could not be queried.",
                "Check Postgres access for memory_capture_events".to_string(),
            );
        }
    }

    match storage
        .count_memory_operations_by_statuses_all_scopes(&["queued_review", "apply_failed"])
        .await
    {
        Ok(count) if count > 0 => {
            push_finding!(
                findings,
                "medium",
                "data_safety",
                "arkmemory queue",
                "Memory operations need review",
                format!("{} staged memory operation(s) are queued or failed", count),
                "Long-lived review backlog can delay or block user-memory corrections.",
                "Open Memory queue and resolve staged operations".to_string(),
            );
        }
        Ok(_) => {}
        Err(e) => {
            push_finding!(
                findings,
                "low",
                "data_safety",
                "arkmemory queue",
                "Could not inspect memory operation health",
                e.to_string(),
                "Staged memory operations could not be queried.",
                "Check Postgres access for memory_operations".to_string(),
            );
        }
    }

    backup_status
}

async fn run_managed_artifact_hygiene_checks(
    ctx: &PulseDoctorContext,
    app_rows: &[serde_json::Value],
    findings: &mut Vec<DoctorFinding>,
) -> crate::core::platform::artifact_hygiene::ArchiveRetentionSummary {
    let data_dir = ctx.data_dir.clone();
    let retention_summary =
        match crate::core::platform::artifact_hygiene::prune_archive_retention(&data_dir).await {
            Ok(summary) => summary,
            Err(error) => {
                push_internal_finding!(
                    findings,
                    "medium",
                    "artifact_hygiene",
                    "artifact_archive",
                    "Archive retention could not be pruned",
                    error.to_string(),
                    "AgentArk could not inspect or prune managed archive entries.",
                    "Check data directory permissions; Pulse will retry on the next run"
                        .to_string(),
                );
                crate::core::platform::artifact_hygiene::ArchiveRetentionSummary::default()
            }
        };

    let lifecycle = load_data_lifecycle_settings(&ctx.storage).await;
    let lifecycle_warnings = lifecycle_retention_warnings(&lifecycle);
    if !lifecycle_warnings.is_empty() {
        push_finding!(
            findings,
            "medium",
            "retention_policy",
            "data_lifecycle",
            "Data lifecycle retention needs review",
            lifecycle_warnings.join(" | "),
            "Loose or disabled retention lets logs, traces, sessions, and automation history grow in Postgres.",
            "Open Settings > Data Lifecycle and tighten retention windows; Pulse will not bypass DB policy".to_string(),
        );
    }

    let managed_apps = managed_artifact_apps_from_rows(app_rows, &data_dir);
    let idle_apps = ctx
        .app_registry
        .get_unused_apps(UNUSED_APP_IDLE_HOURS)
        .await
        .into_iter()
        .map(|(id, _title, last_accessed)| (id, last_accessed))
        .collect::<HashMap<_, _>>();
    match crate::core::platform::artifact_hygiene::collect_artifact_cleanup_candidates(
        &data_dir,
        &managed_apps,
        &idle_apps,
        &lifecycle,
    )
    .await
    {
        Ok(candidates) if !candidates.is_empty() => {
            let total_size: u64 = candidates
                .iter()
                .map(|candidate| candidate.size_bytes)
                .sum();
            let high_risk = candidates
                .iter()
                .filter(|candidate| candidate.risk == "high" || candidate.risk == "medium")
                .count();
            push_finding!(
                findings,
                if high_risk > 0 { "medium" } else { "low" },
                "artifact_hygiene",
                "managed_artifacts",
                "Managed artifact cleanup is available",
                format!(
                    "{} candidate(s), {:.1} MB | {}",
                    candidates.len(),
                    total_size as f64 / (1024.0 * 1024.0),
                    artifact_cleanup_summary(&candidates)
                ),
                "AgentArk-owned runtime artifacts can outlive their active use, but user data and durable memory are excluded from cleanup.",
                "Open Pulse cleanup review and archive selected managed artifacts".to_string(),
            );
        }
        Ok(_) => {}
        Err(error) => {
            push_internal_finding!(
                findings,
                "medium",
                "artifact_hygiene",
                "managed_artifacts",
                "Managed artifact cleanup could not be evaluated",
                error.to_string(),
                "AgentArk could not inspect owned runtime artifact roots.",
                "Check data directory permissions; Pulse will retry on the next run".to_string(),
            );
        }
    }

    retention_summary
}

async fn run_policy_compliance_checks(findings: &mut Vec<DoctorFinding>) {
    let agent_path = Path::new("src").join("core").join("agent.rs");
    let task_router_path = Path::new("src").join("core").join("task_router.rs");
    let parallel_path = Path::new("src").join("core").join("parallel.rs");

    if let Ok(agent_src) = tokio::fs::read_to_string(&agent_path).await {
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

    if let Ok(router_src) = tokio::fs::read_to_string(&task_router_path).await {
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

    if let Ok(parallel_src) = tokio::fs::read_to_string(&parallel_path).await {
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
    app_probe_base_url: Option<&str>,
    app_endpoints: &[AppEndpoint],
    deployed_apps: &[AppPulseInfo],
    findings: &mut Vec<DoctorFinding>,
) {
    let effective_http_base = app_probe_base_url
        .unwrap_or(http_base)
        .trim_end_matches('/');
    let effective_ws_base = app_probe_base_url
        .map(websocket_base_from_http_base)
        .unwrap_or_else(|| ws_base.to_string());
    let app_state: HashMap<String, &AppPulseInfo> =
        deployed_apps.iter().map(|a| (a.id.clone(), a)).collect();

    for app in app_endpoints.iter().cloned() {
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
                    DoctorRemediationSpec::AppRestart {
                        app_id: app.id.clone(),
                    },
                );
            }
        }

        let access_key = app
            .access_key
            .clone()
            .or_else(|| parse_access_key(&app.access_url));
        let sanitized_access_url = strip_access_key(&app.access_url);
        let root_url = format!("{}{}", effective_http_base, sanitized_access_url);
        let started = Instant::now();
        let mut root_request = http_client.get(&root_url);
        if let Some(key) = access_key.as_deref() {
            root_request = root_request.header("x-agentark-app-password", key);
        }
        match tokio::time::timeout(Duration::from_secs(5), root_request.send()).await {
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
                        DoctorRemediationSpec::AppRestart {
                            app_id: app.id.clone(),
                        },
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
                    DoctorRemediationSpec::AppRestart {
                        app_id: app.id.clone(),
                    },
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
                    DoctorRemediationSpec::AppRestart {
                        app_id: app.id.clone(),
                    },
                );
            }
        }

        if let Some(key) = access_key {
            let health_url = format!("{}/apps/{}/health", effective_http_base, app.id);
            if let Ok(Ok(resp)) = tokio::time::timeout(
                Duration::from_secs(4),
                http_client
                    .get(&health_url)
                    .header("x-agentark-app-password", key.clone())
                    .send(),
            )
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
                        DoctorRemediationSpec::AppRestart {
                            app_id: app.id.clone(),
                        },
                    );
                }
            }

            let has_ws_hint = if app.is_static {
                false
            } else {
                detect_ws_hint_async(app.app_dir.clone()).await
            };
            if has_ws_hint {
                let ws_url = format!("{}/apps/{}/ws", effective_ws_base, app.id);
                let ws_request = match tokio_tungstenite::tungstenite::client::IntoClientRequest::into_client_request(
                    ws_url.clone(),
                ) {
                    Ok(mut request) => {
                        if let Ok(value) =
                            axum::http::HeaderValue::from_str(&key)
                        {
                            request
                                .headers_mut()
                                .insert("x-agentark-app-key", value);
                        }
                        Some(request)
                    }
                    Err(_) => None,
                };
                match tokio::time::timeout(Duration::from_secs(4), async {
                    match ws_request {
                        Some(request) => tokio_tungstenite::connect_async(request).await,
                        None => tokio_tungstenite::connect_async(&ws_url).await,
                    }
                })
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
    ctx: &PulseDoctorContext,
    http_client: &reqwest::Client,
    deployed_apps: &[AppPulseInfo],
    security: Option<&crate::core::SecuritySnapshot>,
    security_thresholds: PulseSecurityThresholds,
) -> PulseDoctorReport {
    let mut findings: Vec<DoctorFinding> = Vec::new();
    let mut sections: Vec<PulseScanSection> = Vec::new();
    let data_dir = ctx.data_dir.clone();
    let app_rows = ctx.app_registry.list().await;
    let app_endpoints = parse_app_endpoints(&app_rows, &data_dir);
    let (http_base, ws_base) = control_plane_bases();
    let has_deployed_apps = !deployed_apps.is_empty();
    let configured_public_base_url = ctx
        .config
        .public_apps
        .base_url
        .as_deref()
        .map(str::trim)
        .filter(|value| !value.is_empty())
        .map(|configured| configured.trim_end_matches('/').to_string());
    let app_probe_base_url = configured_public_base_url.clone().or_else(|| {
        ctx.config
            .public_apps
            .bind_addr
            .as_deref()
            .and_then(default_http_base_for_bind_addr)
    });
    let api_key = ctx.api_key.as_deref();

    let attack_surface_started = Instant::now();
    let findings_before = findings.len();
    run_attack_surface_checks(
        http_client,
        &http_base,
        has_deployed_apps,
        configured_public_base_url.as_deref(),
        api_key,
        &mut findings,
    )
    .await;
    sections.push(build_scan_section(
        "attack_surface",
        "Attack surface probes",
        attack_surface_started.elapsed(),
        &findings[findings_before..],
        "Probed externally reachable surfaces for unsafe exposure or traversal behavior.",
        "Checked tunnel/app exposure and file-serving surfaces.",
        vec![
            pulse_metric("Managed apps", app_endpoints.len().to_string()),
            pulse_metric(
                "Public base URL",
                configured_public_base_url
                    .clone()
                    .unwrap_or_else(|| "not configured".to_string()),
            ),
        ],
    ));

    let runtime_hardening_started = Instant::now();
    let findings_before = findings.len();
    run_runtime_hardening_checks(http_client, &http_base, &app_endpoints, &mut findings).await;
    sections.push(build_scan_section(
        "runtime_hardening",
        "Runtime hardening",
        runtime_hardening_started.elapsed(),
        &findings[findings_before..],
        "Validated managed runtime surfaces and gateway protections.",
        "Checked runtime-facing routes and hardening expectations.",
        vec![pulse_metric(
            "Managed apps",
            app_endpoints.len().to_string(),
        )],
    ));

    let resource_checks_started = Instant::now();
    let findings_before = findings.len();
    run_resource_checks(
        ctx,
        deployed_apps,
        security,
        security_thresholds,
        &mut findings,
    )
    .await;
    sections.push(build_scan_section(
        "resource_posture",
        "Resource posture",
        resource_checks_started.elapsed(),
        &findings[findings_before..],
        "Reviewed database size, durable knowledge growth, traffic spikes, and security counters.",
        "Checked capacity pressure and burst behavior across storage and runtime counters.",
        vec![
            pulse_metric("Deployed apps", deployed_apps.len().to_string()),
            pulse_metric(
                "Security snapshot",
                if security.is_some() {
                    "present"
                } else {
                    "none"
                },
            ),
        ],
    ));

    let data_safety_started = Instant::now();
    let findings_before = findings.len();
    let managed_backup_status = run_data_safety_checks(
        &ctx.storage,
        &data_dir,
        ctx.allow_managed_backup_work,
        &mut findings,
    )
    .await;
    sections.push(build_scan_section(
        "data_safety",
        "Data safety",
        data_safety_started.elapsed(),
        &findings[findings_before..],
        "Checked managed backup creation and durable schema readiness.",
        "Prepared or refreshed framework-managed Postgres backups and verified expected storage tables.",
        vec![
            pulse_metric("Data dir", data_dir.display().to_string()),
            pulse_metric("Managed backup", managed_backup_status),
        ],
    ));

    let artifact_hygiene_started = Instant::now();
    let findings_before = findings.len();
    let archive_retention =
        run_managed_artifact_hygiene_checks(ctx, &app_rows, &mut findings).await;
    sections.push(build_scan_section(
        "artifact_hygiene",
        "Managed artifact hygiene",
        artifact_hygiene_started.elapsed(),
        &findings[findings_before..],
        "Reviewed AgentArk-owned runtime artifacts and pruned expired archive entries.",
        "Checked managed apps, runtime residue, browser/session scratch, automation artifacts, and archive retention without touching user data.",
        vec![
            pulse_metric(
                "Archive entries pruned",
                archive_retention.deleted_entries.to_string(),
            ),
            pulse_metric(
                "Archive bytes pruned",
                format!("{:.1} MB", archive_retention.deleted_bytes as f64 / (1024.0 * 1024.0)),
            ),
        ],
    ));

    let policy_started = Instant::now();
    let findings_before = findings.len();
    run_policy_compliance_checks(&mut findings).await;
    sections.push(build_scan_section(
        "policy_compliance",
        "Policy compliance",
        policy_started.elapsed(),
        &findings[findings_before..],
        "Read policy-critical source files for bounded retry and recovery safeguards.",
        "Validated code paths that can create loops, duplicate tool calls, or miss recovery behavior.",
        vec![
            pulse_metric("Files", "3"),
            pulse_metric("Source", "agent/task_router/parallel"),
        ],
    ));

    let app_scan_started = Instant::now();
    let findings_before = findings.len();
    let mut app_scan_tasks = Vec::new();
    let app_count = app_endpoints.len();
    let has_app_endpoints = !app_endpoints.is_empty();
    for app in app_endpoints.clone() {
        app_scan_tasks.push(tokio::task::spawn_blocking(move || {
            let mut findings = Vec::new();
            run_dependency_and_supply_checks_for_app(&app, &mut findings);
            run_secret_scan_for_app(&app, &mut findings);
            findings
        }));
    }
    for task in app_scan_tasks {
        match tokio::time::timeout(Duration::from_secs(20), task).await {
            Ok(Ok(app_findings)) => findings.extend(app_findings),
            Ok(Err(error)) => {
                tracing::warn!("Pulse app scan worker failed: {}", error);
            }
            Err(_) => {
                tracing::warn!("Pulse app scan worker timed out after 20s");
            }
        }
    }
    sections.push(build_scan_section(
        "app_code_scan",
        "Managed app code scan",
        app_scan_started.elapsed(),
        &findings[findings_before..],
        if !has_app_endpoints {
            "No managed apps were available for dependency or secret scanning.".to_string()
        } else {
            "Scanned managed app directories for dependency drift, risky install hooks, and secret exposure.".to_string()
        },
        "Reviewed app manifests and a bounded subset of source files for code-level risk indicators.",
        vec![pulse_metric("Apps scanned", app_count.to_string())],
    ));

    let app_health_started = Instant::now();
    let findings_before = findings.len();
    run_app_health_checks(
        http_client,
        &http_base,
        &ws_base,
        app_probe_base_url.as_deref(),
        &app_endpoints,
        deployed_apps,
        &mut findings,
    )
    .await;
    sections.push(build_scan_section(
        "app_runtime_health",
        "Managed app runtime health",
        app_health_started.elapsed(),
        &findings[findings_before..],
        if app_endpoints.is_empty() {
            "No managed apps were available for health probes.".to_string()
        } else {
            "Probed app entrypoints, guarded health routes, and WebSocket paths where expected."
                .to_string()
        },
        "Validated deployed app reachability and latency from the control plane.",
        vec![
            pulse_metric("Apps probed", app_endpoints.len().to_string()),
            pulse_metric("Runtime app rows", deployed_apps.len().to_string()),
        ],
    ));

    findings.sort_by(|a, b| severity_weight(&b.severity).cmp(&severity_weight(&a.severity)));
    if findings.len() > 40 {
        findings.truncate(40);
    }
    PulseDoctorReport { findings, sections }
}

async fn sleep_or_shutdown(
    duration: std::time::Duration,
    shutdown_rx: &mut tokio::sync::watch::Receiver<bool>,
) -> bool {
    tokio::select! {
        _ = shutdown_rx.changed() => false,
        _ = tokio::time::sleep(duration) => true,
    }
}

async fn tick_or_shutdown(
    interval: &mut tokio::time::Interval,
    shutdown_rx: &mut tokio::sync::watch::Receiver<bool>,
) -> bool {
    tokio::select! {
        _ = shutdown_rx.changed() => false,
        _ = interval.tick() => true,
    }
}

fn sentinel_interval_secs(seconds: u64) -> tokio::time::Interval {
    let mut interval = tokio::time::interval(Duration::from_secs(seconds.max(1)));
    interval.set_missed_tick_behavior(tokio::time::MissedTickBehavior::Delay);
    interval
}

fn watcher_sleep_duration(
    next_wakeup_at: Option<chrono::DateTime<chrono::Utc>>,
    max_sleep: Duration,
) -> Duration {
    let Some(next_wakeup_at) = next_wakeup_at else {
        return max_sleep;
    };
    let now = chrono::Utc::now();
    if next_wakeup_at <= now {
        return Duration::ZERO;
    }
    (next_wakeup_at - now)
        .to_std()
        .unwrap_or(max_sleep)
        .min(max_sleep)
}

/// Start all Sentinel background loops. Returns join handles for graceful shutdown.
pub fn start(
    agent: SharedAgent,
    config: SentinelConfig,
    shutdown_rx: tokio::sync::watch::Receiver<bool>,
) -> Vec<tokio::task::JoinHandle<()>> {
    let mut handles = Vec::new();

    // -- Task Scheduler --------------------------------------------------
    handles.push({
        let agent = agent.clone();
        let mut shutdown = shutdown_rx.clone();
        crate::spawn_logged!("src/sentinel.rs:2928", async move {
            if !sleep_or_shutdown(
                std::time::Duration::from_secs(SENTINEL_STARTUP_SETTLE_SECS),
                &mut shutdown,
            )
            .await
            {
                return;
            }
            let mut interval = sentinel_interval_secs(config.scheduler_interval);
            interval.tick().await;
            loop {
                if !tick_or_shutdown(&mut interval, &mut shutdown).await {
                    break;
                }
                record_loop_heartbeat(&agent, SENTINEL_SCHEDULER_HEARTBEAT_KEY).await;
                run_leased_loop_with_timeout(&agent, "scheduler", run_scheduler(&agent)).await;
            }
        })
    });

    // -- Watcher Poller --------------------------------------------------
    handles.push({
        let agent = agent.clone();
        let mut shutdown = shutdown_rx.clone();
        crate::spawn_logged!("src/sentinel.rs:2948", async move {
            if !sleep_or_shutdown(
                std::time::Duration::from_secs(SENTINEL_STARTUP_SETTLE_SECS),
                &mut shutdown,
            )
            .await
            {
                return;
            }
            let max_sleep = Duration::from_secs(config.watcher_interval.max(1));
            loop {
                record_loop_heartbeat(&agent, SENTINEL_WATCHER_HEARTBEAT_KEY).await;
                let watcher_manager = {
                    let agent_guard = agent.read().await;
                    agent_guard.watcher_manager.clone()
                };
                let autonomy_paused = is_agent_autonomy_paused(&agent).await;
                if !autonomy_paused {
                    run_leased_loop_with_timeout(&agent, "watchers", run_watchers(&agent)).await;
                }
                let sleep_for = if autonomy_paused {
                    max_sleep.min(Duration::from_secs(60))
                } else {
                    watcher_sleep_duration(watcher_manager.next_wakeup_at().await, max_sleep)
                };
                tokio::select! {
                    _ = shutdown.changed() => break,
                    _ = watcher_manager.wait_for_change() => continue,
                    _ = tokio::time::sleep(sleep_for) => continue,
                }
            }
        })
    });

    if config.integration_sync_interval > 0 {
        handles.push({
            let agent = agent.clone();
            let mut shutdown = shutdown_rx.clone();
            crate::spawn_logged!("src/sentinel.rs:2968", async move {
                if !sleep_or_shutdown(std::time::Duration::from_secs(35), &mut shutdown).await {
                    return;
                }
                let mut interval = sentinel_interval_secs(config.integration_sync_interval);
                interval.tick().await;
                loop {
                    if !tick_or_shutdown(&mut interval, &mut shutdown).await {
                        break;
                    }
                    record_loop_heartbeat(&agent, SENTINEL_INTEGRATION_SYNC_HEARTBEAT_KEY).await;
                    if is_agent_autonomy_paused(&agent).await {
                        continue;
                    }
                    run_with_busy_deferral(
                        &agent,
                        "integration_sync",
                        MAINTENANCE_DEFER_MINUTES,
                        MAINTENANCE_MAX_DEFERS,
                        || {
                            let agent = agent.clone();
                            async move { run_integration_sync(&agent).await }
                        },
                    )
                    .await;
                }
            })
        });
    }

    // -- Memory Consolidation --------------------------------------------
    handles.push({
        let agent = agent.clone();
        let mut shutdown = shutdown_rx.clone();
        crate::spawn_logged!("src/sentinel.rs:3004", async move {
            let mut interval = sentinel_interval_secs(config.experience_consolidation_interval);
            interval.tick().await;
            loop {
                if !tick_or_shutdown(&mut interval, &mut shutdown).await {
                    break;
                }
                if is_agent_autonomy_paused(&agent).await {
                    continue;
                }
                run_with_busy_deferral(
                    &agent,
                    "experience_consolidation",
                    MAINTENANCE_DEFER_MINUTES,
                    MAINTENANCE_MAX_DEFERS,
                    || {
                        let agent = agent.clone();
                        async move { run_experience_consolidation_job(&agent).await }
                    },
                )
                .await;
            }
        })
    });

    handles.push({
        let agent = agent.clone();
        let mut shutdown = shutdown_rx.clone();
        crate::spawn_logged!(
            "src/sentinel.rs:background_session_consolidation",
            async move {
                let mut interval =
                    sentinel_interval_secs(config.background_session_consolidation_interval);
                interval.tick().await;
                loop {
                    if !tick_or_shutdown(&mut interval, &mut shutdown).await {
                        break;
                    }
                    if is_agent_autonomy_paused(&agent).await {
                        continue;
                    }
                    run_with_busy_deferral(
                        &agent,
                        "background_session_consolidation",
                        MAINTENANCE_DEFER_MINUTES,
                        MAINTENANCE_MAX_DEFERS,
                        || {
                            let agent = agent.clone();
                            async move { run_background_session_consolidation_job(&agent).await }
                        },
                    )
                    .await;
                }
            }
        )
    });

    handles.push({
        let agent = agent.clone();
        let mut shutdown = shutdown_rx.clone();
        crate::spawn_logged!("src/sentinel.rs:3034", async move {
            let mut interval = sentinel_interval_secs(config.heuristic_reflection_interval);
            interval.tick().await;
            loop {
                if !tick_or_shutdown(&mut interval, &mut shutdown).await {
                    break;
                }
                if is_agent_autonomy_paused(&agent).await {
                    continue;
                }
                run_with_busy_deferral(
                    &agent,
                    "reflection_pass",
                    MAINTENANCE_DEFER_MINUTES,
                    MAINTENANCE_MAX_DEFERS,
                    || {
                        let agent = agent.clone();
                        async move { run_heuristic_reflection_job(&agent).await }
                    },
                )
                .await;
            }
        })
    });

    handles.push({
        let agent = agent.clone();
        let mut shutdown = shutdown_rx.clone();
        crate::spawn_logged!("src/sentinel.rs:3064", async move {
            let mut interval = sentinel_interval_secs(config.pattern_induction_interval);
            interval.tick().await;
            loop {
                if !tick_or_shutdown(&mut interval, &mut shutdown).await {
                    break;
                }
                if is_agent_autonomy_paused(&agent).await {
                    continue;
                }
                run_with_busy_deferral(
                    &agent,
                    "pattern_induction",
                    MAINTENANCE_DEFER_MINUTES,
                    MAINTENANCE_MAX_DEFERS,
                    || {
                        let agent = agent.clone();
                        async move { run_pattern_induction_job(&agent).await }
                    },
                )
                .await;
            }
        })
    });

    handles.push({
        let agent = agent.clone();
        let mut shutdown = shutdown_rx.clone();
        crate::spawn_logged!("src/sentinel.rs:3094", async move {
            let mut interval = sentinel_interval_secs(config.candidate_generation_interval);
            interval.tick().await;
            loop {
                if !tick_or_shutdown(&mut interval, &mut shutdown).await {
                    break;
                }
                if is_agent_autonomy_paused(&agent).await {
                    continue;
                }
                run_with_busy_deferral(
                    &agent,
                    "candidate_generation",
                    MAINTENANCE_DEFER_MINUTES,
                    MAINTENANCE_MAX_DEFERS,
                    || {
                        let agent = agent.clone();
                        async move { run_candidate_generation_job(&agent).await }
                    },
                )
                .await;
            }
        })
    });

    if config.arkmemory_learned_review_interval > 0 {
        handles.push({
            let agent = agent.clone();
            let mut shutdown = shutdown_rx.clone();
            crate::spawn_logged!("src/sentinel.rs:arkmemory_learned_review", async move {
                if !sleep_or_shutdown(std::time::Duration::from_secs(600), &mut shutdown).await {
                    return;
                }
                let mut interval = sentinel_interval_secs(config.arkmemory_learned_review_interval);
                interval.tick().await;
                loop {
                    if is_agent_autonomy_paused(&agent).await {
                        if !tick_or_shutdown(&mut interval, &mut shutdown).await {
                            break;
                        }
                        continue;
                    }
                    run_with_busy_deferral(
                        &agent,
                        "arkmemory_learned_review",
                        MAINTENANCE_DEFER_MINUTES,
                        MAINTENANCE_MAX_DEFERS,
                        || {
                            let agent = agent.clone();
                            async move { run_arkmemory_learned_review_job(&agent).await }
                        },
                    )
                    .await;
                    if !tick_or_shutdown(&mut interval, &mut shutdown).await {
                        break;
                    }
                }
            })
        });
    }

    // -- Approval Expiry -------------------------------------------------
    handles.push({
        let agent = agent.clone();
        let mut shutdown = shutdown_rx.clone();
        crate::spawn_logged!("src/sentinel.rs:3095", async move {
            let mut interval = sentinel_interval_secs(config.approval_expiry_interval);
            interval.tick().await; // Skip first immediate tick
            loop {
                if !tick_or_shutdown(&mut interval, &mut shutdown).await {
                    break;
                }
                record_loop_heartbeat(&agent, SENTINEL_APPROVAL_EXPIRY_HEARTBEAT_KEY).await;
                tracing::debug!("Sentinel: approval_expiry started");
                run_leased_loop_with_timeout(
                    &agent,
                    "approval_expiry",
                    run_approval_expiry(&agent),
                )
                .await;
            }
        })
    });

    // -- Pulse (proactive agent wake-up) -------------------------------
    if config.pulse_interval > 0 {
        handles.push({
            let agent = agent.clone();
            let mut shutdown = shutdown_rx.clone();
            crate::spawn_logged!("src/sentinel.rs:3127", async move {
                // Wait for initial startup to settle
                if !sleep_or_shutdown(
                    std::time::Duration::from_secs(PULSE_STARTUP_SETTLE_SECS),
                    &mut shutdown,
                )
                .await
                {
                    return;
                }
                let mut interval = sentinel_interval_secs(config.pulse_interval);
                interval.tick().await; // Skip first tick (we already waited)
                loop {
                    if !tick_or_shutdown(&mut interval, &mut shutdown).await {
                        break;
                    }
                    record_loop_heartbeat(&agent, SENTINEL_ARKPULSE_HEARTBEAT_KEY).await;
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

    // -- Autonomy Auto-Analysis (periodic insight generation) -------------
    if config.auto_analysis_interval > 0 {
        handles.push({
            let agent = agent.clone();
            let mut shutdown = shutdown_rx.clone();
            crate::spawn_logged!("src/sentinel.rs:3164", async move {
                let initial_wait_secs =
                    PULSE_STARTUP_SETTLE_SECS.saturating_add(AUTO_ANALYSIS_STAGGER_SECS);
                if !sleep_or_shutdown(
                    std::time::Duration::from_secs(initial_wait_secs),
                    &mut shutdown,
                )
                .await
                {
                    return;
                }
                let mut interval = sentinel_interval_secs(config.auto_analysis_interval);
                interval.tick().await;
                loop {
                    if !tick_or_shutdown(&mut interval, &mut shutdown).await {
                        break;
                    }
                    record_loop_heartbeat(&agent, SENTINEL_AUTO_ANALYSIS_HEARTBEAT_KEY).await;
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
                                tracing::debug!("Sentinel: auto_analysis tick started");
                                match tokio::time::timeout(
                                    std::time::Duration::from_secs(45),
                                    channels::http::run_autonomy_analysis_tick(
                                        agent.clone(),
                                        "sentinel_periodic",
                                    ),
                                )
                                .await
                                {
                                    Ok(result) => {
                                        tracing::debug!(
                                            status = result
                                                .get("status")
                                                .and_then(|value| value.as_str())
                                                .unwrap_or("unknown"),
                                            skipped = result
                                                .get("skipped")
                                                .and_then(|value| value.as_bool())
                                                .unwrap_or(false),
                                            "Sentinel: auto_analysis tick completed"
                                        );
                                    }
                                    Err(_) => {
                                        tracing::warn!(
                                            timeout_secs = 45,
                                            "Sentinel: auto_analysis tick timed out"
                                        );
                                    }
                                }
                            }
                        },
                    )
                    .await;
                }
            })
        });
    }

    // -- Vector memory cleanup (monthly, idle-only) ----------------------
    // -- Unused App Notifications ----------------------------------------
    handles.push({
        let agent = agent.clone();
        let mut shutdown = shutdown_rx.clone();
        crate::spawn_logged!("src/sentinel.rs:3207", async move {
            // Wait for startup to settle
            if !sleep_or_shutdown(std::time::Duration::from_secs(120), &mut shutdown).await {
                return;
            }
            let mut interval = sentinel_interval_secs(config.unused_app_check_interval);
            interval.tick().await; // Skip first tick
            loop {
                if !tick_or_shutdown(&mut interval, &mut shutdown).await {
                    break;
                }
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

    if config.container_reaper_interval > 0 {
        handles.push({
            let agent = agent.clone();
            let mut shutdown = shutdown_rx.clone();
            crate::spawn_logged!("src/sentinel.rs:3242", async move {
                if !sleep_or_shutdown(std::time::Duration::from_secs(45), &mut shutdown).await {
                    return;
                }
                let mut interval = sentinel_interval_secs(config.container_reaper_interval);
                interval.tick().await;
                loop {
                    if !tick_or_shutdown(&mut interval, &mut shutdown).await {
                        break;
                    }
                    let runtime = {
                        let agent_guard = agent.read().await;
                        agent_guard.runtime.clone()
                    };
                    run_leased_loop_with_timeout(&agent, "container_reaper", async move {
                        tracing::debug!("Sentinel: container_reaper started");
                        match runtime.reconcile_orphan_containers().await {
                            Ok(_) => tracing::debug!("Sentinel: container_reaper completed"),
                            Err(error) => {
                                tracing::warn!(
                                    "Sentinel: sandbox container reconciliation failed: {}",
                                    error
                                );
                            }
                        }
                    })
                    .await;
                }
            })
        });
    }

    // -- Security Log Cleanup (every 15 days, idle-only) -----------------
    handles.push({
        let agent = agent.clone();
        let mut shutdown = shutdown_rx.clone();
        crate::spawn_logged!("src/sentinel.rs:3286", async move {
            // Check every 6 hours, but only actually cleanup every 15 days when idle
            if !sleep_or_shutdown(std::time::Duration::from_secs(600), &mut shutdown).await {
                return;
            }
            let mut interval = sentinel_interval_secs(6 * 3600);
            interval.tick().await;
            loop {
                if !tick_or_shutdown(&mut interval, &mut shutdown).await {
                    break;
                }
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

    // -- Storage Housekeeping (leased, scheduled retention cleanup) ---------
    handles.push({
        let agent = agent.clone();
        let mut shutdown = shutdown_rx.clone();
        crate::spawn_logged!("src/sentinel.rs:storage_housekeeping", async move {
            if !sleep_or_shutdown(std::time::Duration::from_secs(300), &mut shutdown).await {
                return;
            }
            let mut interval = sentinel_interval_secs(STORAGE_HOUSEKEEPING_INTERVAL_SECS);
            interval.tick().await;
            loop {
                if !tick_or_shutdown(&mut interval, &mut shutdown).await {
                    break;
                }
                run_with_busy_deferral(
                    &agent,
                    "storage_housekeeping",
                    MAINTENANCE_DEFER_MINUTES,
                    MAINTENANCE_MAX_DEFERS,
                    || {
                        let agent = agent.clone();
                        async move { run_storage_housekeeping(&agent).await }
                    },
                )
                .await;
            }
        })
    });

    tracing::info!(
        "Sentinel started: scheduler={}s, watchers={}s, integration_sync={}s, experience_learning={}s, background_session_consolidation={}s, heuristic_reflection={}s, pattern_induction={}s, candidate_generation={}s, arkmemory_learned_review={}s, pulse={}s, auto_analysis={}s, container_reaper={}s",
        config.scheduler_interval,
        config.watcher_interval,
        if config.integration_sync_interval > 0 {
            config.integration_sync_interval.to_string()
        } else {
            "off".to_string()
        },
        config.experience_consolidation_interval,
        config.background_session_consolidation_interval,
        config.heuristic_reflection_interval,
        config.pattern_induction_interval,
        config.candidate_generation_interval,
        if config.arkmemory_learned_review_interval > 0 {
            config.arkmemory_learned_review_interval.to_string()
        } else {
            "off".to_string()
        },
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
        if config.container_reaper_interval > 0 {
            config.container_reaper_interval.to_string()
        } else {
            "off".to_string()
        },
    );

    handles
}

// ===========================================================================
// Task Scheduler - execute cron/scheduled tasks when due
// ===========================================================================

async fn run_scheduler(agent: &SharedAgent) {
    let autonomy_paused = is_agent_autonomy_paused(agent).await;
    maybe_emit_autonomy_pause_nudge(agent, autonomy_paused).await;

    let agent_snapshot = Agent::snapshot(agent).await;
    let due_tasks = agent_snapshot.take_due_tasks(autonomy_paused).await;

    if autonomy_paused && due_tasks.is_empty() {
        tracing::debug!("Sentinel: scheduler paused for non-reminder tasks");
    }

    if !due_tasks.is_empty() {
        tracing::info!("Sentinel: {} scheduled task(s) due", due_tasks.len());
    }

    for task in due_tasks {
        tracing::info!(
            "Sentinel: queueing supervised task '{}' (action={})",
            task.description,
            task.action
        );
        let agent = Arc::clone(agent);
        let permits = Arc::clone(&SCHEDULED_TASK_PERMITS);
        crate::spawn_logged!("src/sentinel.rs:3371", async move {
            let Ok(_permit) = permits.acquire_owned().await else {
                return;
            };
            crate::core::Agent::execute_task_supervised_shared(&agent, task).await;
        });
    }
}

// ===========================================================================
// Watcher Poller - check conditions and fire triggers
// ===========================================================================

async fn record_loop_heartbeat(agent: &SharedAgent, key: &str) {
    let storage = { agent.read().await.storage.clone() };
    let now = chrono::Utc::now().to_rfc3339();
    if let Err(error) = storage.set(key, now.as_bytes()).await {
        tracing::debug!("Failed to persist sentinel heartbeat '{}': {}", key, error);
    }
}

async fn notify_preferred_channel_bounded(
    agent: &SharedAgent,
    message: &str,
    context: &'static str,
) {
    let message = message.to_string();
    match tokio::time::timeout(
        Duration::from_secs(*SENTINEL_NOTIFY_TIMEOUT_SECS),
        async move {
            let agent_guard = agent.read().await;
            agent_guard.notify_preferred_channel(&message).await;
        },
    )
    .await
    {
        Ok(()) => {}
        Err(_) => tracing::warn!(
            "Sentinel: {} preferred-channel notification timed out after {}s",
            context,
            *SENTINEL_NOTIFY_TIMEOUT_SECS
        ),
    }
}

async fn persist_watcher_notification_attempt(
    watcher_manager: &crate::core::automation::watcher::WatcherManager,
    watcher_id: uuid::Uuid,
    channel: String,
    success: bool,
    message: &str,
    error: Option<String>,
) {
    watcher_manager
        .push_notification_attempt(
            watcher_id,
            crate::core::automation::watcher::WatcherNotificationAttempt {
                attempted_at: chrono::Utc::now(),
                channel,
                success,
                message: message.to_string(),
                error,
            },
        )
        .await;
}

fn watcher_timeout_uses_preferred_fallback(channel: &str) -> bool {
    matches!(
        channel.trim().to_ascii_lowercase().as_str(),
        "" | "preferred" | "push" | "auto" | "default"
    )
}

fn sentinel_watcher_poll_timeout_cap_secs() -> u64 {
    (*SENTINEL_JOB_TIMEOUT_SECS).saturating_sub(5).max(1)
}

fn watcher_poll_timeout_secs(watcher: &crate::core::automation::watcher::Watcher) -> u64 {
    let default_timeout = *WATCHER_POLL_TIMEOUT_SECS;
    let policy = crate::core::automation::policy_from_arguments(
        &watcher.poll_arguments,
        crate::core::automation::AutomationValidation::default(),
    );
    let requested_timeout = if policy.stall_timeout_secs > 0 {
        default_timeout.max(policy.stall_timeout_secs)
    } else {
        default_timeout
    };
    requested_timeout.min(sentinel_watcher_poll_timeout_cap_secs())
}

fn watcher_uses_semantic_observation_contract(
    watcher: &crate::core::automation::watcher::Watcher,
) -> bool {
    if crate::core::automation::arguments_require_change_detection_observation(
        &watcher.poll_arguments,
    ) {
        return true;
    }

    watcher
        .poll_arguments
        .get("semantic_watcher_poll")
        .and_then(serde_json::Value::as_bool)
        .unwrap_or(false)
}

fn semantic_watcher_poll_readiness_error(
    watcher: &crate::core::automation::watcher::Watcher,
    result: &str,
) -> Option<String> {
    if !watcher_uses_semantic_observation_contract(watcher) {
        return None;
    }

    let Some(payload) = Agent::extract_structured_watcher_condition_payload(result) else {
        return Some(format!(
            "Durable semantic observation is missing structured readiness JSON. Return a single JSON object satisfying this contract: {}",
            crate::core::automation::durable_observation_result_contract()
        ));
    };

    crate::core::automation::validate_change_detection_observation_payload(&payload).err()
}

async fn run_watchers(agent: &SharedAgent) {
    if is_agent_autonomy_paused(agent).await {
        tracing::debug!("Sentinel: watchers skipped (agent paused)");
        return;
    }
    let agent_snapshot = Agent::snapshot(agent).await;
    let (watcher_manager, notification_store) = {
        let agent_guard = agent.read().await;
        (
            agent_guard.watcher_manager.clone(),
            agent_guard.notification_store(),
        )
    };

    // Expire timed-out watchers
    let expired = watcher_manager.expire_watchers().await;
    for w in &expired {
        let msg = format!(
            "Watcher timed out: **{}**\n\nPolled `{}` {} times over {} minutes without finding a match.",
            w.description,
            w.poll_action,
            w.poll_count,
            w.timeout_secs / 60
        );
        let web_outcome = notification_store
            .emit_notification_with_status("Watcher Timed Out", &msg, "warning", "watcher")
            .await;
        persist_watcher_notification_attempt(
            &watcher_manager,
            w.id,
            web_outcome.channel,
            web_outcome.success,
            &msg,
            web_outcome.error,
        )
        .await;
        let notify_channel = w.notify_channel.trim();
        if watcher_timeout_uses_preferred_fallback(notify_channel) {
            for outcome in agent_snapshot.notify_preferred_channel_reported(&msg).await {
                if outcome.channel.eq_ignore_ascii_case("web") {
                    continue;
                }
                persist_watcher_notification_attempt(
                    &watcher_manager,
                    w.id,
                    outcome.channel,
                    outcome.success,
                    &msg,
                    outcome.error,
                )
                .await;
            }
        } else if !notify_channel.eq_ignore_ascii_case("web")
            && !notify_channel.eq_ignore_ascii_case("in_app")
        {
            for outcome in agent_snapshot
                .notify_preferred_channel_reported_with_hint(&msg, Some(notify_channel), true)
                .await
            {
                if outcome.channel.eq_ignore_ascii_case("web") {
                    continue;
                }
                persist_watcher_notification_attempt(
                    &watcher_manager,
                    w.id,
                    outcome.channel,
                    outcome.success,
                    &msg,
                    outcome.error,
                )
                .await;
            }
        }
        agent_snapshot
            .sync_watcher_supervisor_state(w, Some("timed_out"), None)
            .await;
    }

    // Poll due watchers
    let due_watchers = watcher_manager.get_due_watchers().await;

    for watcher in due_watchers {
        let Some(watcher) = watcher_manager.begin_poll(watcher.id).await else {
            continue;
        };
        let poll_timeout_secs = watcher_poll_timeout_secs(&watcher);
        let poll_result = match tokio::time::timeout(
            Duration::from_secs(poll_timeout_secs),
            agent_snapshot.run_model_routed_spine_for_watcher_poll(&watcher),
        )
        .await
        {
            Ok(result) => result,
            Err(_) => Err(anyhow::anyhow!(
                "Watcher spine poll timed out after {} seconds",
                poll_timeout_secs
            )),
        };

        let new_count = watcher.poll_count;

        match poll_result {
            Ok(result) => {
                let policy = crate::core::automation::policy_from_arguments(
                    &watcher.poll_arguments,
                    crate::core::automation::AutomationValidation::default(),
                );
                let critique = crate::core::automation::critique_result(
                    &policy.validation,
                    Some(&result),
                    None,
                );
                if !critique.validation_passed {
                    let error_text = critique.summary.clone();
                    watcher_manager
                        .record_poll_error(watcher.id, new_count, error_text.clone())
                        .await;
                    tracing::warn!(
                        "Watcher '{}' poll #{} failed validation: {}",
                        watcher.description,
                        new_count,
                        error_text
                    );
                    continue;
                }
                if let Some(error_text) = semantic_watcher_poll_readiness_error(&watcher, &result) {
                    watcher_manager
                        .record_poll_error(watcher.id, new_count, error_text.clone())
                        .await;
                    tracing::warn!(
                        "Watcher '{}' poll #{} failed durable observation readiness: {}",
                        watcher.description,
                        new_count,
                        error_text
                    );
                    continue;
                }
                let matched = if let Some(outcome) = Agent::evaluate_watch_condition_without_llm(
                    &watcher.condition,
                    &result,
                    watcher.last_result.as_deref(),
                ) {
                    match outcome {
                        Ok(matched) => matched,
                        Err(error_text) => {
                            watcher_manager
                                .record_poll_error(watcher.id, new_count, error_text.clone())
                                .await;
                            tracing::warn!(
                                "Watcher '{}' poll #{} failed condition evaluation: {}",
                                watcher.description,
                                new_count,
                                error_text
                            );
                            continue;
                        }
                    }
                } else {
                    match agent_snapshot
                        .evaluate_watcher_condition(
                            &watcher.description,
                            &watcher.condition,
                            &result,
                            watcher.last_result.as_deref(),
                        )
                        .await
                    {
                        Ok(matched) => matched,
                        Err(error_text) => {
                            watcher_manager
                                .record_poll_error(watcher.id, new_count, error_text.clone())
                                .await;
                            tracing::warn!(
                                "Watcher '{}' poll #{} failed condition evaluation: {}",
                                watcher.description,
                                new_count,
                                error_text
                            );
                            continue;
                        }
                    }
                };
                watcher_manager
                    .record_poll_success(watcher.id, new_count, result.clone(), matched)
                    .await;
                tracing::info!(
                    "Watcher '{}' poll #{}: action={}, result_len={}, condition_matched={}",
                    watcher.description,
                    new_count,
                    watcher.poll_action,
                    result.len(),
                    matched
                );
                if matched {
                    let trigger_result = result.clone();
                    watcher_manager
                        .mark_triggered(watcher.id, trigger_result.clone())
                        .await;

                    let followup_worker = agent_snapshot.watcher_followup_worker();
                    let followup_agent = agent_snapshot.clone();
                    let permits = Arc::clone(&WATCHER_TRIGGER_PERMITS);
                    crate::spawn_logged!("src/sentinel.rs:3645", async move {
                        let Ok(_permit) = permits.acquire_owned().await else {
                            return;
                        };
                        let result = tokio::time::timeout(
                            Duration::from_secs(*WATCHER_TRIGGER_TIMEOUT_SECS),
                            async move {
                                let prepared = followup_worker
                                    .prepare_watcher_followup(&watcher, &trigger_result)
                                    .await;
                                followup_agent
                                    .handle_watcher_trigger_supervised(
                                        watcher,
                                        trigger_result,
                                        prepared,
                                    )
                                    .await;
                            },
                        )
                        .await;
                        if result.is_err() {
                            tracing::warn!(
                                "Sentinel: watcher trigger follow-up timed out after {}s",
                                *WATCHER_TRIGGER_TIMEOUT_SECS
                            );
                        }
                    });
                }
            }
            Err(e) => {
                let error_text = e.to_string();
                watcher_manager
                    .record_poll_error(watcher.id, new_count, error_text.clone())
                    .await;
                tracing::debug!("Sentinel: watcher {} poll error: {}", watcher.id, e);
            }
        }
    }

    // Cleanup old watchers
    watcher_manager.cleanup().await;
}

// ===========================================================================
// Background Learning
// ===========================================================================

async fn persist_background_learning_job_result(
    storage: &crate::storage::Storage,
    update: crate::channels::http::BackgroundLearningJobUpdate,
) {
    channels::http::record_background_learning_job_result(storage, &update).await;
}

fn background_learning_job_update(
    key: &str,
    status: &str,
    started_at: chrono::DateTime<chrono::Utc>,
    completed_at: chrono::DateTime<chrono::Utc>,
    summary: String,
    changed: bool,
    stats: serde_json::Value,
) -> crate::channels::http::BackgroundLearningJobUpdate {
    crate::channels::http::BackgroundLearningJobUpdate {
        key: key.to_string(),
        status: status.to_string(),
        started_at: Some(started_at.to_rfc3339()),
        completed_at: Some(completed_at.to_rfc3339()),
        summary,
        changed,
        stats,
    }
}

async fn run_integration_sync(agent: &SharedAgent) {
    let shared_agent = agent.clone();
    let ctx = {
        let agent = shared_agent.read().await;
        crate::core::connectivity::integration_sync::context_from_agent(
            &agent,
            Some(shared_agent.clone()),
        )
    };
    if let Err(error) = crate::core::connectivity::integration_sync::run_due_syncs(&ctx).await {
        tracing::warn!("Sentinel: integration sync failed: {}", error);
    }
}

async fn run_experience_consolidation_job(agent: &SharedAgent) {
    let started_at = chrono::Utc::now();
    let storage = {
        let agent = agent.read().await;
        agent.storage.clone()
    };
    let deferred_memory_processed = {
        let agent_snapshot = Agent::snapshot(agent).await;
        agent_snapshot
            .process_deferred_user_memory_capture_candidates()
            .await
    };
    match crate::core::knowledge::learning::run_experience_consolidation(&storage).await {
        Ok(processed) if processed > 0 => {
            let completed_at = chrono::Utc::now();
            tracing::info!(
                "Sentinel: experience consolidation processed {} run(s), {} memory candidate(s)",
                processed,
                deferred_memory_processed
            );
            persist_background_learning_job_result(
                &storage,
                background_learning_job_update(
                    "experience_consolidation",
                    "completed",
                    started_at,
                    completed_at,
                    format!(
                        "Consolidated {} experience run(s) and {} deferred memory candidate(s).",
                        processed, deferred_memory_processed
                    ),
                    true,
                    serde_json::json!({
                        "experience_runs_processed": processed,
                        "deferred_memory_candidates_processed": deferred_memory_processed,
                    }),
                ),
            )
            .await;
        }
        Ok(_) if deferred_memory_processed > 0 => {
            let completed_at = chrono::Utc::now();
            tracing::info!(
                "Sentinel: experience consolidation processed {} deferred memory candidate(s)",
                deferred_memory_processed
            );
            persist_background_learning_job_result(
                &storage,
                background_learning_job_update(
                    "experience_consolidation",
                    "completed",
                    started_at,
                    completed_at,
                    format!(
                        "Processed {} deferred memory candidate(s).",
                        deferred_memory_processed
                    ),
                    true,
                    serde_json::json!({
                        "experience_runs_processed": 0,
                        "deferred_memory_candidates_processed": deferred_memory_processed,
                    }),
                ),
            )
            .await;
        }
        Ok(_) => {
            let completed_at = chrono::Utc::now();
            persist_background_learning_job_result(
                &storage,
                background_learning_job_update(
                    "experience_consolidation",
                    "completed",
                    started_at,
                    completed_at,
                    "No experience runs were ready for consolidation.".to_string(),
                    false,
                    serde_json::json!({
                        "experience_runs_processed": 0,
                        "deferred_memory_candidates_processed": deferred_memory_processed,
                    }),
                ),
            )
            .await;
        }
        Err(error) => {
            let completed_at = chrono::Utc::now();
            tracing::debug!("Sentinel: experience consolidation skipped: {}", error);
            persist_background_learning_job_result(
                &storage,
                background_learning_job_update(
                    "experience_consolidation",
                    "failed",
                    started_at,
                    completed_at,
                    format!("Experience consolidation skipped: {}", error),
                    false,
                    serde_json::json!({
                        "error": error.to_string(),
                    }),
                ),
            )
            .await;
        }
    }
}

async fn run_background_session_consolidation_job(agent: &SharedAgent) {
    let agent_snapshot = Agent::snapshot(agent).await;
    agent_snapshot
        .maybe_consolidate_idle_background_sessions()
        .await;
}

async fn run_pattern_induction_job(agent: &SharedAgent) {
    let started_at = chrono::Utc::now();
    let storage = {
        let agent = agent.read().await;
        agent.storage.clone()
    };
    match crate::core::knowledge::learning::run_pattern_induction(&storage).await {
        Ok(processed) if processed > 0 => {
            let completed_at = chrono::Utc::now();
            tracing::info!(
                "Sentinel: pattern induction updated {} pattern(s)",
                processed
            );
            persist_background_learning_job_result(
                &storage,
                background_learning_job_update(
                    "pattern_induction",
                    "completed",
                    started_at,
                    completed_at,
                    format!("Updated {} reusable pattern(s).", processed),
                    true,
                    serde_json::json!({
                        "patterns_updated": processed,
                    }),
                ),
            )
            .await;
        }
        Ok(_) => {
            let completed_at = chrono::Utc::now();
            persist_background_learning_job_result(
                &storage,
                background_learning_job_update(
                    "pattern_induction",
                    "completed",
                    started_at,
                    completed_at,
                    "No reusable patterns were ready for induction.".to_string(),
                    false,
                    serde_json::json!({
                        "patterns_updated": 0,
                    }),
                ),
            )
            .await;
        }
        Err(error) => {
            let completed_at = chrono::Utc::now();
            tracing::debug!("Sentinel: pattern induction skipped: {}", error);
            persist_background_learning_job_result(
                &storage,
                background_learning_job_update(
                    "pattern_induction",
                    "failed",
                    started_at,
                    completed_at,
                    format!("Pattern induction skipped: {}", error),
                    false,
                    serde_json::json!({
                        "error": error.to_string(),
                    }),
                ),
            )
            .await;
        }
    }
}

async fn run_heuristic_reflection_job(agent: &SharedAgent) {
    let started_at = chrono::Utc::now();
    let agent_snapshot = Agent::snapshot(agent).await;
    let storage = agent_snapshot.storage.clone();
    let result = agent_snapshot.run_heuristic_reflection_pass().await;
    match result {
        Ok(stats) if stats.changed() => {
            let completed_at = chrono::Utc::now();
            tracing::info!(
                "Sentinel: heuristic reflection created {} and merged {} heuristic(s)",
                stats.heuristics_created,
                stats.heuristics_merged
            );
            persist_background_learning_job_result(
                &storage,
                background_learning_job_update(
                    "reflection_pass",
                    "completed",
                    started_at,
                    completed_at,
                    stats.summary(),
                    true,
                    serde_json::json!({
                        "runs_examined": stats.runs_examined,
                        "heuristics_created": stats.heuristics_created,
                        "heuristics_merged": stats.heuristics_merged,
                        "skipped": stats.skipped,
                        "failed": stats.failed,
                    }),
                ),
            )
            .await;
        }
        Ok(stats) => {
            let completed_at = chrono::Utc::now();
            persist_background_learning_job_result(
                &storage,
                background_learning_job_update(
                    "reflection_pass",
                    "completed",
                    started_at,
                    completed_at,
                    stats.summary(),
                    false,
                    serde_json::json!({
                        "runs_examined": stats.runs_examined,
                        "heuristics_created": stats.heuristics_created,
                        "heuristics_merged": stats.heuristics_merged,
                        "skipped": stats.skipped,
                        "failed": stats.failed,
                    }),
                ),
            )
            .await;
        }
        Err(error) => {
            let completed_at = chrono::Utc::now();
            let status = if error.to_string().contains("no_learning_model") {
                "completed"
            } else {
                "failed"
            };
            persist_background_learning_job_result(
                &storage,
                background_learning_job_update(
                    "reflection_pass",
                    status,
                    started_at,
                    completed_at,
                    if status == "completed" {
                        "No learning model was available for heuristic reflection.".to_string()
                    } else {
                        format!("Heuristic reflection failed: {}", error)
                    },
                    false,
                    serde_json::json!({
                        "error": error.to_string(),
                    }),
                ),
            )
            .await;
        }
    }
}

async fn run_candidate_generation_job(agent: &SharedAgent) {
    let started_at = chrono::Utc::now();
    let (storage, data_dir) = {
        let agent = agent.read().await;
        (agent.storage.clone(), agent.data_dir.clone())
    };
    match crate::core::knowledge::learning::run_candidate_generation(&storage, &data_dir).await {
        Ok(processed) if processed > 0 => {
            let completed_at = chrono::Utc::now();
            tracing::info!(
                "Sentinel: candidate generation updated {} draft(s)",
                processed
            );
            persist_background_learning_job_result(
                &storage,
                background_learning_job_update(
                    "candidate_generation",
                    "completed",
                    started_at,
                    completed_at,
                    format!("Prepared {} candidate draft(s).", processed),
                    true,
                    serde_json::json!({
                        "candidates_generated": processed,
                    }),
                ),
            )
            .await;
        }
        Ok(_) => {
            let completed_at = chrono::Utc::now();
            persist_background_learning_job_result(
                &storage,
                background_learning_job_update(
                    "candidate_generation",
                    "completed",
                    started_at,
                    completed_at,
                    "No candidate drafts were ready for generation.".to_string(),
                    false,
                    serde_json::json!({
                        "candidates_generated": 0,
                    }),
                ),
            )
            .await;
        }
        Err(error) => {
            let completed_at = chrono::Utc::now();
            tracing::debug!("Sentinel: candidate generation skipped: {}", error);
            persist_background_learning_job_result(
                &storage,
                background_learning_job_update(
                    "candidate_generation",
                    "failed",
                    started_at,
                    completed_at,
                    format!("Candidate generation skipped: {}", error),
                    false,
                    serde_json::json!({
                        "error": error.to_string(),
                    }),
                ),
            )
            .await;
        }
    }
}

async fn run_arkmemory_learned_review_job(agent: &SharedAgent) {
    let started_at = chrono::Utc::now();
    let (storage, llm, embedding_client) = {
        let agent = agent.read().await;
        (
            agent.storage.clone(),
            agent.llm.clone(),
            agent.embedding_client.clone(),
        )
    };
    match channels::http::run_arkmemory_learned_review_pass(
        &storage,
        &llm,
        embedding_client.as_deref(),
    )
    .await
    {
        Ok(stats) => {
            let completed_at = chrono::Utc::now();
            let auto_reviewed = stats
                .get("auto_reviewed")
                .and_then(|value| value.as_u64())
                .unwrap_or(0);
            let failed_examined = stats
                .get("failed_examined")
                .and_then(|value| value.as_u64())
                .unwrap_or(0);
            let semantic_judgments = stats
                .get("semantic_judgments")
                .and_then(|value| value.as_u64())
                .unwrap_or(0);
            if auto_reviewed > 0 {
                tracing::info!(
                    "Sentinel: Memory learned reviewer auto-reviewed {} capture finding(s)",
                    auto_reviewed
                );
            }
            persist_background_learning_job_result(
                &storage,
                background_learning_job_update(
                    "arkmemory_learned_review",
                    "completed",
                    started_at,
                    completed_at,
                    if auto_reviewed > 0 {
                        format!(
                            "Applied learned review feedback to {} memory capture finding(s).",
                            auto_reviewed
                        )
                    } else {
                        format!(
                            "Examined {} capture finding(s); {} needed semantic judgment and none were confident enough to auto-review.",
                            failed_examined, semantic_judgments
                        )
                    },
                    auto_reviewed > 0,
                    stats,
                ),
            )
            .await;
        }
        Err(error) => {
            let completed_at = chrono::Utc::now();
            tracing::debug!("Sentinel: Memory learned review skipped: {}", error);
            persist_background_learning_job_result(
                &storage,
                background_learning_job_update(
                    "arkmemory_learned_review",
                    "failed",
                    started_at,
                    completed_at,
                    format!("Memory learned review skipped: {}", error),
                    false,
                    serde_json::json!({
                        "error": error.to_string(),
                    }),
                ),
            )
            .await;
        }
    }
}

// ===========================================================================
// Approval Expiry
// ===========================================================================

async fn run_approval_expiry(agent: &SharedAgent) {
    const APPROVAL_EXPIRY_SECS: i64 = 7 * 24 * 60 * 60;
    let (storage, tasks) = {
        let agent_guard = agent.read().await;
        (agent_guard.storage.clone(), agent_guard.tasks.clone())
    };
    if let Err(e) = storage.expire_old_approvals(APPROVAL_EXPIRY_SECS).await {
        tracing::debug!("Sentinel: approval expiry check: {}", e);
    }
    if let Err(e) =
        Agent::expire_stale_approval_tasks_shared(&storage, &tasks, APPROVAL_EXPIRY_SECS).await
    {
        tracing::debug!("Sentinel: stale approval task expiry check: {}", e);
    }
    {
        let agent_guard = agent.read().await;
        agent_guard.safety.clear_expired_approvals();
    }
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
struct PulseSecurityThresholds {
    auth_failures: u64,
    rate_limit_hits: u64,
    unauthorized_channel: u64,
    combined: u64,
}

impl Default for PulseSecurityThresholds {
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
            let mut settings = parsed;
            settings.enforce_dependencies();
            return settings;
        }
    }
    let mut settings = crate::core::AutonomySettings::default();
    settings.enforce_dependencies();
    settings
}

async fn maybe_emit_autonomy_pause_nudge(agent: &SharedAgent, autonomy_paused: bool) {
    let storage = {
        let guard = agent.read().await;
        guard.storage.clone()
    };

    if !autonomy_paused {
        let _ = storage
            .delete(crate::core::automation::autonomy::AUTONOMY_PAUSED_SINCE_KEY)
            .await;
        let _ = storage
            .delete(crate::core::automation::autonomy::AUTONOMY_PAUSE_NUDGE_LAST_SENT_AT_KEY)
            .await;
        return;
    }

    let now_ts = chrono::Utc::now().timestamp();
    let paused_since = match storage
        .get(crate::core::automation::autonomy::AUTONOMY_PAUSED_SINCE_KEY)
        .await
    {
        Ok(Some(raw)) => String::from_utf8(raw)
            .ok()
            .and_then(|value| value.trim().parse::<i64>().ok())
            .unwrap_or(0),
        Ok(None) => 0,
        Err(error) => {
            tracing::debug!("Failed to read autonomy pause start: {}", error);
            0
        }
    };

    if paused_since <= 0 {
        let now = now_ts.to_string();
        if let Err(error) = storage
            .set(
                crate::core::automation::autonomy::AUTONOMY_PAUSED_SINCE_KEY,
                now.as_bytes(),
            )
            .await
        {
            tracing::debug!("Failed to persist autonomy pause start: {}", error);
        }
        return;
    }

    if now_ts - paused_since < crate::core::automation::autonomy::AUTONOMY_PAUSE_NUDGE_INTERVAL_SECS
    {
        return;
    }

    let last_sent_at = match storage
        .get(crate::core::automation::autonomy::AUTONOMY_PAUSE_NUDGE_LAST_SENT_AT_KEY)
        .await
    {
        Ok(Some(raw)) => String::from_utf8(raw)
            .ok()
            .and_then(|value| value.trim().parse::<i64>().ok())
            .unwrap_or(0),
        Ok(None) => 0,
        Err(error) => {
            tracing::debug!("Failed to read autonomy pause nudge timestamp: {}", error);
            0
        }
    };

    if last_sent_at > 0
        && (now_ts - last_sent_at)
            < crate::core::automation::autonomy::AUTONOMY_PAUSE_NUDGE_INTERVAL_SECS
    {
        return;
    }

    let message = "Autonomy has been paused for at least 7 days. Consider enabling it again so AgentArk can resume Pulse health checks, watchers, background learning, suggestion scans, and proactive optimizations. Scheduled reminders still fire while autonomy is paused.";
    {
        let guard = agent.read().await;
        guard
            .emit_notification(
                AUTONOMY_PAUSE_NUDGE_TITLE,
                message,
                "warning",
                AUTONOMY_PAUSE_NUDGE_SOURCE,
            )
            .await;
    }

    let now = now_ts.to_string();
    if let Err(error) = storage
        .set(
            crate::core::automation::autonomy::AUTONOMY_PAUSE_NUDGE_LAST_SENT_AT_KEY,
            now.as_bytes(),
        )
        .await
    {
        tracing::debug!(
            "Failed to persist autonomy pause nudge timestamp: {}",
            error
        );
    }
}

async fn load_arkpulse_security_thresholds(
    storage: &crate::storage::Storage,
) -> PulseSecurityThresholds {
    let parsed = load_autonomy_settings_snapshot(storage).await;
    PulseSecurityThresholds {
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
    crate::core::automation::autonomy::autonomy_background_paused(&settings)
}

fn is_security_incident(
    security: Option<&crate::core::SecuritySnapshot>,
    thresholds: PulseSecurityThresholds,
) -> bool {
    let Some(sec) = security else {
        return false;
    };
    if sec.injection_attempts > 0 {
        return true;
    }
    has_correlated_abuse_spike(sec, thresholds)
}

fn has_correlated_abuse_spike(
    sec: &crate::core::SecuritySnapshot,
    thresholds: PulseSecurityThresholds,
) -> bool {
    if sec.rate_limit_hits >= thresholds.rate_limit_hits {
        return true;
    }
    if sec.unauthorized_channel_attempts >= thresholds.unauthorized_channel {
        return true;
    }
    let non_auth = sec.rate_limit_hits + sec.unauthorized_channel_attempts;
    let combined = sec.auth_failures + non_auth;
    non_auth > 0 && combined >= thresholds.combined
}

fn doctor_finding_requires_correlated_attack(finding: &DoctorFinding) -> bool {
    let category = finding.category.trim().to_ascii_lowercase();
    if category != "attack_surface" && category != "resource" {
        return false;
    }
    let mut text = String::new();
    for part in [
        finding.title.as_str(),
        finding.target.as_str(),
        finding.evidence.as_str(),
        finding.root_cause.as_str(),
        finding.fix_command.as_str(),
    ] {
        if !text.is_empty() {
            text.push(' ');
        }
        text.push_str(part);
    }
    let text = text.to_ascii_lowercase();
    text.contains("without auth")
        || text.contains("auth-failure")
        || text.contains("auth failure")
        || text.contains("auth failures")
        || text.contains("auth middleware")
        || text.contains("authentication")
        || text.contains("credential stuffing")
}

fn doctor_finding_counts_for_critical_alert(
    finding: &DoctorFinding,
    has_correlated_attack: bool,
) -> bool {
    finding.user_actionable
        && matches!(finding.severity.as_str(), "critical" | "high")
        && (!doctor_finding_requires_correlated_attack(finding) || has_correlated_attack)
}

fn build_noncritical_summary(
    overdue_tasks: usize,
    approaching_goals: usize,
    security: Option<&crate::core::SecuritySnapshot>,
    doctor_high: usize,
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
    if doctor_high > 0 || doctor_medium > 0 || doctor_low > 0 {
        let mut d = Vec::new();
        if doctor_high > 0 {
            d.push(format!("{} high", doctor_high));
        }
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
    thresholds: PulseSecurityThresholds,
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
        actions.push("Fix failing service checks shown in Pulse details.".to_string());
    }

    if let Some(sec) = security {
        if sec.injection_attempts > 0 {
            flags.push("security_injection".to_string());
            reasons.push(format!(
                "{} prompt injection attempt(s)",
                sec.injection_attempts
            ));
        }
        if has_correlated_abuse_spike(sec, thresholds) {
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

    let has_correlated_attack =
        security.is_some_and(|sec| has_correlated_abuse_spike(sec, thresholds));
    let mut high_or_critical: Vec<&DoctorFinding> = doctor_findings
        .iter()
        .filter(|f| doctor_finding_counts_for_critical_alert(f, has_correlated_attack))
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
            "Pulse critical incident detected. Open Pulse details for diagnostics.".to_string(),
            vec!["critical".to_string()],
        )
    } else if actions.is_empty() {
        (
            format!("Pulse critical incident: {}.", reasons.join(", ")),
            flags,
        )
    } else {
        (
            format!(
                "Pulse critical incident: {}. Immediate actions: {}",
                reasons.join(", "),
                actions.join(" ")
            ),
            flags,
        )
    }
}

// ===========================================================================
// Pulse - proactive agent wake-up
// ===========================================================================

pub async fn run_pulse(agent: &SharedAgent) {
    if is_agent_autonomy_paused(agent).await {
        tracing::debug!("Pulse skipped (agent paused)");
        return;
    }
    if PULSE_RUNNING
        .compare_exchange(false, true, Ordering::AcqRel, Ordering::Acquire)
        .is_err()
    {
        tracing::debug!("Pulse skipped (already running)");
        return;
    }
    let _pulse_guard = PulseRunGuard;
    let pulse_started_at = chrono::Utc::now();
    let pulse_started = Instant::now();
    let mut pulse_scan_log = Vec::new();

    {
        let integration_started = Instant::now();
        let shared_agent = agent.clone();
        let ctx = {
            let guard = shared_agent.read().await;
            crate::core::connectivity::integration_sync::context_from_agent(
                &guard,
                Some(shared_agent.clone()),
            )
        };
        match crate::core::connectivity::integration_sync::run_due_syncs(&ctx).await {
            Ok(()) => pulse_scan_log.push(PulseScanSection {
                id: "integration_sync".to_string(),
                title: "Integration sync".to_string(),
                status: "ok".to_string(),
                summary: "Checked due integration syncs before diagnostics.".to_string(),
                detail:
                    "Ran the connector scheduler first so Pulse used the freshest integration state."
                        .to_string(),
                duration_ms: integration_started.elapsed().as_millis() as u64,
                metrics: vec![pulse_metric(
                    "Duration",
                    format!("{} ms", integration_started.elapsed().as_millis()),
                )],
            }),
            Err(error) => {
                tracing::debug!("Pulse integration probe skipped: {}", error);
                pulse_scan_log.push(PulseScanSection {
                    id: "integration_sync".to_string(),
                    title: "Integration sync".to_string(),
                    status: "warning".to_string(),
                    summary: "Integration sync probe was skipped for this run.".to_string(),
                    detail: error.to_string(),
                    duration_ms: integration_started.elapsed().as_millis() as u64,
                    metrics: vec![pulse_metric(
                        "Duration",
                        format!("{} ms", integration_started.elapsed().as_millis()),
                    )],
                });
            }
        }
    }

    // -- Code-only checks first (zero LLM tokens) ------------------------
    // Only wake the LLM if there's actually something worth acting on.

    let http_client = reqwest::Client::builder()
        .timeout(std::time::Duration::from_secs(3))
        .build()
        .unwrap_or_default();

    let allow_managed_backup_work = !sentinel_under_load(agent).await;
    let (pulse_ctx, tasks, watcher_manager, security_events, notification_store) = {
        let agent_guard = agent.read().await;
        (
            PulseDoctorContext {
                storage: agent_guard.storage.clone(),
                data_dir: agent_guard.data_dir.clone(),
                allow_managed_backup_work,
                app_registry: agent_guard.app_registry.clone(),
                config: agent_guard.config.clone(),
                embedding_client: agent_guard.embedding_client.clone(),
                model_pool: agent_guard.model_pool.clone(),
                primary_model_id: agent_guard.primary_model_id.clone(),
                llm: agent_guard.llm.clone(),
                api_key: agent_guard.api_key.clone(),
            },
            agent_guard.tasks.clone(),
            agent_guard.watcher_manager.clone(),
            agent_guard.security_events.clone(),
            agent_guard.notification_store(),
        )
    };
    let storage = pulse_ctx.storage.clone();
    let now_marker = chrono::Utc::now().to_rfc3339();
    let _ = storage
        .set(ARKPULSE_LAST_RUN_AT_KEY, now_marker.as_bytes())
        .await;
    let security_thresholds = load_arkpulse_security_thresholds(&storage).await;

    let (overdue_tasks, failed_tasks, approaching_goals, brief_channel, mut details, deployed_apps) = {
        let now = chrono::Utc::now();
        let all_tasks = {
            let tasks = tasks.read().await;
            tasks.all().to_vec()
        };
        let mut run_sections = Vec::new();
        let task_snapshot_started = Instant::now();

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

        // Find goals with approaching deadlines (<=3 days or overdue)
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
        let active_watchers = watcher_manager.list().await.len();
        run_sections.push(PulseScanSection {
            id: "task_snapshot".to_string(),
            title: "Task and watcher snapshot".to_string(),
            status: if overdue.is_empty() && failed.is_empty() {
                "ok".to_string()
            } else {
                "warning".to_string()
            },
            summary: if overdue.is_empty() && failed.is_empty() {
                "Captured task, goal, and watcher counts with no overdue or failed work."
                    .to_string()
            } else {
                format!(
                    "Captured runtime workload with {} overdue item(s) and {} failed task(s).",
                    overdue.len(),
                    failed.len()
                )
            },
            detail: format!(
                "Pending: {} | Running: {} | Completed: {} | Goal deadlines close: {} | Active watchers: {}",
                pending,
                running,
                completed,
                approaching_goals.len(),
                active_watchers
            ),
            duration_ms: task_snapshot_started.elapsed().as_millis() as u64,
            metrics: vec![
                pulse_metric("Pending", pending.to_string()),
                pulse_metric("Running", running.to_string()),
                pulse_metric("Completed", completed.to_string()),
                pulse_metric("Overdue", overdue.len().to_string()),
                pulse_metric("Failed", failed.len().to_string()),
                pulse_metric("Watchers", active_watchers.to_string()),
            ],
        });

        // -- Health checks ------------------------------------------------
        let mut health_checks = Vec::new();
        let health_snapshot_started = Instant::now();

        // Postgres-backed pgvector retrieval
        let fact_count = match pulse_ctx.storage.count_facts(None).await {
            Ok(count) => count as usize,
            Err(error) => {
                tracing::warn!("Pulse failed to count learned facts: {}", error);
                0
            }
        };
        let document_count = match pulse_ctx.storage.count_documents(None).await {
            Ok(count) => count,
            Err(error) => {
                tracing::warn!("Pulse failed to count documents: {}", error);
                0
            }
        };
        let document_chunk_count = match pulse_ctx.storage.count_document_chunks().await {
            Ok(count) => count,
            Err(error) => {
                tracing::warn!("Pulse failed to count document chunks: {}", error);
                0
            }
        };
        let total_memories = fact_count;
        let knowledge_counts = KnowledgeStoreCounts {
            facts: fact_count as u64,
            documents: document_count,
            document_chunks: document_chunk_count,
        };
        health_checks.push(build_knowledge_store_health_check(&knowledge_counts));
        let pgvector_retrieval_check = match pulse_ctx.storage.pgvector_health_check().await {
            Ok(()) => {
                if let Some(client) = pulse_ctx.embedding_client.as_ref() {
                    match client.health_check().await {
                        Ok(message) => HealthCheck {
                            service: "Postgres pgvector retrieval".to_string(),
                            status: "ok".to_string(),
                            message: format!(
                                "pgvector ready, embeddings healthy ({}) | {} learned facts",
                                message, fact_count
                            ),
                        },
                        Err(error) => HealthCheck {
                            service: "Postgres pgvector retrieval".to_string(),
                            status: "warn".to_string(),
                            message: format!(
                                "pgvector ready, embeddings unavailable: {} | {} learned facts",
                                error, fact_count
                            ),
                        },
                    }
                } else {
                    HealthCheck {
                        service: "Postgres pgvector retrieval".to_string(),
                        status: "warn".to_string(),
                        message: format!(
                            "pgvector ready, but retrieval is lexical-only until embeddings are configured | {} learned facts",
                            fact_count
                        ),
                    }
                }
            }
            Err(error) => HealthCheck {
                service: "Postgres pgvector retrieval".to_string(),
                status: "warn".to_string(),
                message: format!(
                    "pgvector unavailable: {} | {} learned facts",
                    error, fact_count
                ),
            },
        };
        health_checks.push(pgvector_retrieval_check);

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
            let selected_slot_id = pulse_ctx
                .config
                .model_pool
                .slots
                .iter()
                .find(|s| {
                    s.enabled
                        && matches!(s.role, crate::core::runtime::config::ModelRole::Primary)
                        && pulse_ctx.model_pool.contains_key(&s.id)
                })
                .map(|s| s.id.clone())
                .or_else(|| {
                    if !pulse_ctx.primary_model_id.is_empty()
                        && pulse_ctx
                            .model_pool
                            .contains_key(&pulse_ctx.primary_model_id)
                    {
                        Some(pulse_ctx.primary_model_id.clone())
                    } else {
                        None
                    }
                })
                .or_else(|| {
                    pulse_ctx
                        .config
                        .model_pool
                        .slots
                        .iter()
                        .find(|s| s.enabled && pulse_ctx.model_pool.contains_key(&s.id))
                        .map(|s| s.id.clone())
                })
                .or_else(|| pulse_ctx.model_pool.keys().next().cloned());

            if let Some(slot_id) = selected_slot_id {
                if let Some((slot, client)) = pulse_ctx.model_pool.get(&slot_id) {
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
                        crate::core::runtime::config::ModelRole::Primary => "primary",
                        crate::core::runtime::config::ModelRole::Fast => "fast",
                        crate::core::runtime::config::ModelRole::Code => "code",
                        crate::core::runtime::config::ModelRole::Research => "research",
                        crate::core::runtime::config::ModelRole::Fallback => "fallback",
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
                    message: pulse_ctx.llm.model_name().to_string(),
                }
            }
        };
        health_checks.push(llm_check);
        run_sections.push(PulseScanSection {
            id: "health_snapshot".to_string(),
            title: "Runtime health snapshot".to_string(),
            status: if health_checks
                .iter()
                .any(|check| check.status.eq_ignore_ascii_case("error"))
            {
                "error".to_string()
            } else if health_checks
                .iter()
                .any(|check| check.status.eq_ignore_ascii_case("warn"))
            {
                "warning".to_string()
            } else {
                "ok".to_string()
            },
            summary: format!(
                "Collected {} health checks across memory retrieval, browser tooling, and model connectivity.",
                health_checks.len()
            ),
            detail: health_checks
                .iter()
                .map(|check| format!("{}: {} ({})", check.service, check.message, check.status))
                .take(6)
                .collect::<Vec<_>>()
                .join(" | "),
            duration_ms: health_snapshot_started.elapsed().as_millis() as u64,
            metrics: vec![
                pulse_metric("Checks", health_checks.len().to_string()),
                pulse_metric("Memories", total_memories.to_string()),
                pulse_metric("Documents", document_count.to_string()),
                pulse_metric("Chunks", document_chunk_count.to_string()),
            ],
        });

        // -- Security snapshot --------------------------------------------
        let security_snapshot_started = Instant::now();
        let sec_snapshot = security_events.snapshot();
        let mut security_persisted = false;

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
            let logs: Vec<crate::storage::security_log::Model> = events
                .iter()
                .filter(|(_, _, count, _)| *count > 0)
                .map(
                    |(event_type, severity, count, desc)| crate::storage::security_log::Model {
                        id: uuid::Uuid::new_v4().to_string(),
                        event_type: event_type.to_string(),
                        severity: severity.to_string(),
                        message: format!("{}: {} event(s)", desc, count),
                        source: Some("arkpulse".to_string()),
                        count: (*count).min(i32::MAX as u64) as i32,
                        created_at: now_str.clone(),
                    },
                )
                .collect();
            match pulse_ctx.storage.insert_security_logs(&logs).await {
                Ok(()) => {
                    security_persisted = true;
                    security_events.commit_snapshot(&sec_snapshot);
                    tracing::info!(
                        "Pulse security: injections={}, auth_fail={}, rate_limit={}, unauth={}",
                        sec_snapshot.injection_attempts,
                        sec_snapshot.auth_failures,
                        sec_snapshot.rate_limit_hits,
                        sec_snapshot.unauthorized_channel_attempts,
                    );
                }
                Err(error) => {
                    tracing::warn!(
                        "Failed to persist Pulse security logs; counters retained for retry: {}",
                        error
                    );
                }
            }
        }
        run_sections.push(PulseScanSection {
            id: "security_snapshot".to_string(),
            title: "Security snapshot".to_string(),
            status: if sec_snapshot.injection_attempts > 0 {
                "error".to_string()
            } else if sec_snapshot.has_events() {
                "warning".to_string()
            } else {
                "ok".to_string()
            },
            summary: if sec_snapshot.has_events() {
                "Recorded security counters observed since the previous pulse.".to_string()
            } else {
                "No new security events were recorded since the previous pulse.".to_string()
            },
            detail: format!(
                "Injections: {} | Auth failures: {} | Rate-limit hits: {} | Unauthorized channel attempts: {} | Persisted: {}",
                sec_snapshot.injection_attempts,
                sec_snapshot.auth_failures,
                sec_snapshot.rate_limit_hits,
                sec_snapshot.unauthorized_channel_attempts,
                if security_persisted {
                    "yes"
                } else if sec_snapshot.has_events() {
                    "no"
                } else {
                    "not needed"
                }
            ),
            duration_ms: security_snapshot_started.elapsed().as_millis() as u64,
            metrics: vec![
                pulse_metric("Injections", sec_snapshot.injection_attempts.to_string()),
                pulse_metric("Auth failures", sec_snapshot.auth_failures.to_string()),
                pulse_metric("Rate limits", sec_snapshot.rate_limit_hits.to_string()),
                pulse_metric(
                    "Unauthorized channels",
                    sec_snapshot.unauthorized_channel_attempts.to_string(),
                ),
            ],
        });

        let channel = pulse_ctx
            .storage
            .get("daily_brief_channel")
            .await
            .ok()
            .flatten()
            .and_then(|b| String::from_utf8(b).ok())
            .unwrap_or_else(|| "web".to_string());

        // Deployed apps health snapshot
        let now_ts = chrono::Utc::now();
        let app_snapshots = pulse_ctx.app_registry.pulse_snapshot().await;
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
        run_sections.push(PulseScanSection {
            id: "app_inventory".to_string(),
            title: "Managed app inventory".to_string(),
            status: if deployed_apps
                .iter()
                .any(|app| !app.is_static && !app.process_alive)
            {
                "warning".to_string()
            } else {
                "ok".to_string()
            },
            summary: if deployed_apps.is_empty() {
                "No managed apps are currently deployed.".to_string()
            } else {
                format!(
                    "Captured runtime inventory for {} managed app(s).",
                    deployed_apps.len()
                )
            },
            detail: deployed_apps
                .iter()
                .take(6)
                .map(|app| {
                    format!(
                        "{}: {} | {} | idle {}h",
                        app.title,
                        if app.process_alive || app.is_static {
                            "reachable"
                        } else {
                            "process down"
                        },
                        if app.is_static { "static" } else { "runtime" },
                        app.idle_hours
                    )
                })
                .collect::<Vec<_>>()
                .join(" | "),
            duration_ms: 0,
            metrics: vec![
                pulse_metric("Apps", deployed_apps.len().to_string()),
                pulse_metric(
                    "Managed runtimes",
                    deployed_apps
                        .iter()
                        .filter(|app| !app.is_static)
                        .count()
                        .to_string(),
                ),
                pulse_metric(
                    "Down",
                    deployed_apps
                        .iter()
                        .filter(|app| !app.is_static && !app.process_alive)
                        .count()
                        .to_string(),
                ),
            ],
        });

        let security_snapshot = if sec_snapshot.has_events() {
            Some(sec_snapshot.clone())
        } else {
            None
        };
        let doctor_report = run_doctor_checks(
            &pulse_ctx,
            &http_client,
            &deployed_apps,
            security_snapshot.as_ref(),
            security_thresholds,
        )
        .await;
        run_sections.extend(doctor_report.sections.clone());
        let doctor_findings = doctor_report.findings;
        let doctor_score = compute_doctor_score(&doctor_findings);

        let details = PulseDetails {
            scan_started_at: pulse_started_at.to_rfc3339(),
            scan_finished_at: String::new(),
            scan_duration_ms: 0,
            notification_outcome: String::new(),
            scan_log: run_sections,
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
    pulse_scan_log.extend(details.scan_log.clone());
    details.scan_log = pulse_scan_log;

    // Deterministic pulse classification: notify only on critical breakage or security spikes.
    let has_overdue = !overdue_tasks.is_empty();
    let has_failures = !failed_tasks.is_empty();
    let has_security = details.security.as_ref().is_some_and(|s| s.has_events());
    let has_security_incident =
        is_security_incident(details.security.as_ref(), security_thresholds);
    let has_correlated_attack = details
        .security
        .as_ref()
        .is_some_and(|s| has_correlated_abuse_spike(s, security_thresholds));
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
    let doctor_alert_high_count = details
        .doctor_findings
        .iter()
        .filter(|f| doctor_finding_counts_for_critical_alert(f, has_correlated_attack))
        .count();
    let doctor_alert_critical_count = details
        .doctor_findings
        .iter()
        .filter(|f| {
            f.severity == "critical"
                && doctor_finding_counts_for_critical_alert(f, has_correlated_attack)
        })
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
    let has_doctor_alert = doctor_alert_high_count > 0;
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
        || doctor_alert_critical_count > 0;
    let growth_notification = if !should_notify_user {
        build_knowledge_growth_notification(&details.doctor_findings)
    } else {
        None
    };
    let has_any_signal = has_overdue
        || has_failures
        || has_security
        || has_goal_deadlines
        || has_dead_apps
        || has_doctor_findings
        || health_warn_count > 0
        || health_error_count > 0;

    if !has_any_signal {
        tracing::debug!("Pulse: all clear");
        let summary = "All clear, no issues detected".to_string();
        details.scan_finished_at = chrono::Utc::now().to_rfc3339();
        details.scan_duration_ms = pulse_started.elapsed().as_millis() as u64;
        details.notification_outcome = "none".to_string();
        details.scan_log.push(PulseScanSection {
            id: "notification_outcome".to_string(),
            title: "Notification outcome".to_string(),
            status: "ok".to_string(),
            summary: "No user notification was sent because the run was clear.".to_string(),
            detail: "Pulse logged the run silently for operator review.".to_string(),
            duration_ms: 0,
            metrics: vec![pulse_metric("Outcome", "none")],
        });
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
        log_pulse_event(&storage, event).await;
        return;
    }

    if !has_user_visible_alert {
        let summary = build_noncritical_summary(
            overdue_tasks.len(),
            approaching_goals.len(),
            details.security.as_ref(),
            doctor_high_count,
            doctor_medium_count,
            doctor_low_count,
            health_warn_count,
        );
        details.scan_finished_at = chrono::Utc::now().to_rfc3339();
        details.scan_duration_ms = pulse_started.elapsed().as_millis() as u64;
        details.notification_outcome = if growth_notification.is_some() {
            "in_app_growth_warning".to_string()
        } else {
            "none".to_string()
        };
        details.scan_log.push(PulseScanSection {
            id: "notification_outcome".to_string(),
            title: "Notification outcome".to_string(),
            status: if growth_notification.is_some() {
                "warning".to_string()
            } else {
                "ok".to_string()
            },
            summary: if growth_notification.is_some() {
                "Pulse emitted a throttled growth warning without escalating a critical alert."
                    .to_string()
            } else {
                "Pulse recorded non-critical context without sending a user-facing alert."
                    .to_string()
            },
            detail: "Run history was saved so you can review the context later.".to_string(),
            duration_ms: 0,
            metrics: vec![pulse_metric(
                "Outcome",
                if growth_notification.is_some() {
                    "growth warning"
                } else {
                    "logged only"
                },
            )],
        });
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
        log_pulse_event(&storage, event).await;
        tracing::debug!("Pulse: non-critical signals recorded, no user notification sent");
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
        doctor_alert_high_count,
    );
    let should_emit_alert = if should_notify_user {
        should_emit_arkpulse_critical_notification(&storage, &alert_text).await
    } else {
        false
    };
    details.scan_finished_at = chrono::Utc::now().to_rfc3339();
    details.scan_duration_ms = pulse_started.elapsed().as_millis() as u64;
    details.notification_outcome = if should_emit_alert {
        "in_app_and_preferred_channel".to_string()
    } else if should_notify_user {
        "suppressed_duplicate".to_string()
    } else if growth_notification.is_some() {
        "in_app_growth_warning".to_string()
    } else {
        "logged_only".to_string()
    };
    details.scan_log.push(PulseScanSection {
        id: "notification_outcome".to_string(),
        title: "Notification outcome".to_string(),
        status: if should_emit_alert {
            "error".to_string()
        } else {
            "warning".to_string()
        },
        summary: if should_emit_alert {
            "Pulse emitted a critical alert for this run.".to_string()
        } else if should_notify_user {
            "Pulse suppressed a duplicate alert inside the cooldown window.".to_string()
        } else if growth_notification.is_some() {
            "Pulse emitted a growth warning without escalating a critical alert.".to_string()
        } else {
            "Pulse recorded the issue in history without pushing a user-facing alert.".to_string()
        },
        detail: format!("Preferred channel target: {}", brief_channel),
        duration_ms: 0,
        metrics: vec![pulse_metric(
            "Outcome",
            details.notification_outcome.clone(),
        )],
    });
    if should_emit_alert {
        notification_store
            .emit_notification("Pulse Critical", &alert_text, "error", "arkpulse")
            .await;
    } else if should_notify_user {
        tracing::debug!(
            "Pulse: suppressed duplicate critical notification within {}s cooldown",
            ARKPULSE_CRITICAL_NOTIFY_COOLDOWN_SECS
        );
    } else {
        tracing::debug!(
            "Pulse: alert recorded without user notification (below ultra-severe threshold)"
        );
    }
    if let Some((signature, body)) = growth_notification {
        if should_emit_arkpulse_growth_notification(&storage, &signature).await {
            notification_store
                .emit_notification(
                    "Knowledge growth warning",
                    &body,
                    "warning",
                    "arkpulse_growth",
                )
                .await;
            tracing::warn!(
                "Pulse: emitted throttled knowledge growth notification (cooldown={}s)",
                ARKPULSE_GROWTH_NOTIFY_COOLDOWN_SECS
            );
        } else {
            tracing::debug!(
                "Pulse: suppressed duplicate knowledge growth notification within {}s cooldown",
                ARKPULSE_GROWTH_NOTIFY_COOLDOWN_SECS
            );
        }
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
    log_pulse_event(&storage, event).await;
    if should_emit_alert {
        notify_preferred_channel_bounded(agent, &alert_text, "arkpulse").await;
        tracing::info!(
            "Pulse: critical alert sent to preferred channel ({})",
            brief_channel
        );
    } else if should_notify_user {
        tracing::debug!(
            "Pulse: duplicate critical alert not pushed to preferred channel ({})",
            brief_channel
        );
    } else {
        tracing::debug!(
            "Pulse: preferred-channel notification skipped (below ultra-severe threshold) ({})",
            brief_channel
        );
    }
}

// ===========================================================================
// Vector memory cleanup - prune stale ephemeral memories, keep core facts
// Runs once per month, only when server is idle (no recent activity).
// ===========================================================================

// ===========================================================================
// Security Log Cleanup - prune entries older than 15 days
// Runs every 15 days, only when server is idle.
// ===========================================================================

const SECURITY_CLEANUP_KEY: &str = "security_log_last_cleanup";

async fn run_security_log_cleanup(agent: &SharedAgent) {
    let (storage, last_activity) = {
        let agent_guard = agent.read().await;
        let storage = agent_guard.storage.clone();
        let la = agent_guard.last_activity_at();
        (storage, la)
    };
    let lifecycle = load_data_lifecycle_settings(&storage).await;
    let last_cleanup_bytes = storage.get(SECURITY_CLEANUP_KEY).await.unwrap_or(None);

    if !lifecycle.cleanup_enabled
        || !lifecycle.logs_cleanup_enabled
        || lifecycle.security_log_retention_days == 0
    {
        return;
    }

    let now = chrono::Utc::now();
    let cleanup_interval_secs = (lifecycle.security_cleanup_interval_days as i64) * 24 * 3600;
    let idle_threshold_secs = lifecycle.security_cleanup_idle_threshold_secs as i64;

    // Check if enough time has passed since last cleanup
    if let Some(bytes) = last_cleanup_bytes {
        if let Ok(ts_str) = String::from_utf8(bytes) {
            if let Ok(last_ts) = ts_str.parse::<chrono::DateTime<chrono::Utc>>() {
                if (now - last_ts).num_seconds() < cleanup_interval_secs {
                    return;
                }
            }
        }
    }

    // Check if server is idle
    if let Some(last) = last_activity {
        if (now - last).num_seconds() < idle_threshold_secs {
            return;
        }
    }

    match storage
        .cleanup_old_security_logs(lifecycle.security_log_retention_days as i64)
        .await
    {
        Ok(deleted) => {
            if deleted > 0 {
                tracing::info!(
                    "Security log cleanup: pruned {} entries older than {} days",
                    deleted,
                    lifecycle.security_log_retention_days
                );
            }
            let _ = storage
                .set(SECURITY_CLEANUP_KEY, now.to_rfc3339().as_bytes())
                .await;
        }
        Err(e) => {
            tracing::debug!("Security log cleanup failed: {}", e);
        }
    }
}

// ===========================================================================
// Unused App Notifications - notify user about idle deployed apps daily
// ===========================================================================

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

    let (app_registry, storage, notification_store) = {
        let agent_guard = agent.read().await;
        (
            agent_guard.app_registry.clone(),
            agent_guard.storage.clone(),
            agent_guard.notification_store(),
        )
    };
    let unused_apps = app_registry.get_unused_apps(UNUSED_APP_IDLE_HOURS).await;

    if unused_apps.is_empty() {
        return;
    }

    let now = chrono::Utc::now();

    for (app_id, title, last_accessed) in &unused_apps {
        // Check cooldown - don't spam the same app notification every hour
        let notify_key = format!("{}{}", UNUSED_APP_NOTIFY_PREFIX, app_id);
        let last_notified = storage
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
        notification_store
            .emit_notification("Unused App", &message, "info", "app_cleanup")
            .await;

        // Push to preferred channel
        notify_preferred_channel_bounded(agent, &message, "unused_app_check").await;

        // Record notification time
        let _ = storage.set(&notify_key, now.to_rfc3339().as_bytes()).await;

        tracing::info!(
            "Sent unused app notification for '{}' (idle {})",
            title,
            idle_display
        );
    }
}
