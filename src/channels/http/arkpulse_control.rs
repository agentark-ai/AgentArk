use super::*;

static ARKPULSE_CLEANUP_ACTIVE: AtomicBool = AtomicBool::new(false);
static ARKPULSE_CLEANUP_JOBS: once_cell::sync::Lazy<
    RwLock<HashMap<String, PulseCleanupJobSnapshot>>,
> = once_cell::sync::Lazy::new(|| RwLock::new(HashMap::new()));
const ARKPULSE_CLEANUP_IDLE_APP_HOURS: i64 = 24;
const ARKPULSE_CLEANUP_JOB_HISTORY_LIMIT: usize = 20;

#[derive(Debug, Deserialize)]
pub(super) struct RunArkPulseFixRequest {
    #[serde(default)]
    fix_command: String,
    #[serde(default)]
    remediation: Option<crate::sentinel::DoctorRemediationSpec>,
    #[serde(default)]
    issue_title: Option<String>,
    #[serde(default)]
    target: Option<String>,
    #[serde(default)]
    event_timestamp: Option<String>,
    #[serde(default)]
    finding_index: Option<usize>,
}

#[derive(Debug, Deserialize)]
pub(super) struct PulseCleanupPreviewRequest {
    #[serde(default)]
    _refresh: bool,
}

#[derive(Debug, Deserialize)]
pub(super) struct RunArkPulseCleanupRequest {
    #[serde(default)]
    candidate_ids: Vec<String>,
    #[serde(default)]
    confirm_archive: bool,
    #[serde(default)]
    event_timestamp: Option<String>,
    #[serde(default)]
    finding_index: Option<usize>,
}

#[derive(Debug, Clone, Serialize)]
pub(super) struct PulseCleanupJobSnapshot {
    job_id: String,
    status: String,
    queued_at: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    started_at: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    finished_at: Option<String>,
    requested_count: usize,
    archived_count: usize,
    archived_bytes: u64,
    skipped_count: usize,
    #[serde(default)]
    archived: Vec<crate::core::artifact_hygiene::ArchivedArtifactOutcome>,
    #[serde(default)]
    errors: Vec<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    retention: Option<crate::core::artifact_hygiene::ArchiveRetentionSummary>,
}

#[derive(Debug, Clone)]
pub(super) enum PulseFixPlan {
    TunnelStartVerify,
    TunnelRestartVerify,
    AppRestart(String),
    ManagedAppOperation {
        app_id: String,
        operation: crate::sentinel::DoctorManagedAppOperation,
    },
    ReadonlyInvestigation {
        topic: crate::sentinel::DoctorReadonlyInvestigationTopic,
    },
}

pub(super) fn arkpulse_fix_plan_from_remediation(
    remediation: &crate::sentinel::DoctorRemediationSpec,
    _allow_shell_command: bool,
) -> Option<PulseFixPlan> {
    match remediation {
        crate::sentinel::DoctorRemediationSpec::TunnelStartVerify => {
            Some(PulseFixPlan::TunnelStartVerify)
        }
        crate::sentinel::DoctorRemediationSpec::TunnelRestartVerify => {
            Some(PulseFixPlan::TunnelRestartVerify)
        }
        crate::sentinel::DoctorRemediationSpec::AppRestart { app_id } => {
            if is_valid_app_id(app_id) {
                Some(PulseFixPlan::AppRestart(app_id.clone()))
            } else {
                None
            }
        }
        crate::sentinel::DoctorRemediationSpec::ReadonlyInvestigation { topic } => {
            Some(PulseFixPlan::ReadonlyInvestigation {
                topic: topic.clone(),
            })
        }
        crate::sentinel::DoctorRemediationSpec::ManagedAppOperation { app_id, operation } => {
            if is_valid_app_id(app_id) {
                Some(PulseFixPlan::ManagedAppOperation {
                    app_id: app_id.clone(),
                    operation: operation.clone(),
                })
            } else {
                None
            }
        }
        crate::sentinel::DoctorRemediationSpec::ShellCommand { .. } => None,
    }
}

pub(super) fn truncate_for_response(input: &str, max_chars: usize) -> String {
    if input.chars().count() <= max_chars {
        return input.to_string();
    }
    input.chars().take(max_chars).collect::<String>() + "..."
}

fn parse_cleanup_datetime(value: &str) -> Option<chrono::DateTime<chrono::Utc>> {
    chrono::DateTime::parse_from_rfc3339(value.trim())
        .ok()
        .map(|value| value.with_timezone(&chrono::Utc))
}

fn managed_artifact_apps_from_app_rows(
    app_rows: &[serde_json::Value],
    data_dir: &FsPath,
) -> Vec<crate::core::artifact_hygiene::ManagedArtifactApp> {
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
            Some(crate::core::artifact_hygiene::ManagedArtifactApp {
                app_dir: row
                    .get("app_dir")
                    .and_then(|value| value.as_str())
                    .map(PathBuf::from)
                    .unwrap_or_else(|| data_dir.join("apps").join(&id)),
                created_at: row
                    .get("created_at")
                    .and_then(|value| value.as_str())
                    .and_then(parse_cleanup_datetime),
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
                id,
                title,
            })
        })
        .collect()
}

async fn collect_arkpulse_cleanup_candidates_for_state(
    state: &AppState,
) -> Result<Vec<crate::core::artifact_hygiene::ArtifactCleanupCandidate>> {
    let (data_dir, storage) = {
        let agent = state.agent.read().await;
        (agent.data_dir().to_path_buf(), agent.storage.clone())
    };
    let lifecycle = load_data_lifecycle_settings(&storage).await;
    let app_rows = state.app_registry.list().await;
    let apps = managed_artifact_apps_from_app_rows(&app_rows, &data_dir);
    let idle_apps = state
        .app_registry
        .get_unused_apps(ARKPULSE_CLEANUP_IDLE_APP_HOURS)
        .await
        .into_iter()
        .map(|(id, _title, last_accessed)| (id, last_accessed))
        .collect::<HashMap<_, _>>();
    crate::core::artifact_hygiene::collect_artifact_cleanup_candidates(
        &data_dir, &apps, &idle_apps, &lifecycle,
    )
    .await
}

async fn cleanup_active_job_snapshot() -> Option<PulseCleanupJobSnapshot> {
    let jobs = ARKPULSE_CLEANUP_JOBS.read().await;
    jobs.values()
        .filter(|job| job.status == "queued" || job.status == "running")
        .max_by(|left, right| left.queued_at.cmp(&right.queued_at))
        .cloned()
}

async fn upsert_cleanup_job(snapshot: PulseCleanupJobSnapshot) {
    let mut jobs = ARKPULSE_CLEANUP_JOBS.write().await;
    jobs.insert(snapshot.job_id.clone(), snapshot);
    if jobs.len() > ARKPULSE_CLEANUP_JOB_HISTORY_LIMIT {
        let mut ordered = jobs
            .values()
            .map(|job| (job.queued_at.clone(), job.job_id.clone()))
            .collect::<Vec<_>>();
        ordered.sort();
        for (_, job_id) in ordered.into_iter().take(
            jobs.len()
                .saturating_sub(ARKPULSE_CLEANUP_JOB_HISTORY_LIMIT),
        ) {
            jobs.remove(&job_id);
        }
    }
}

async fn get_cleanup_job_snapshot(job_id: &str) -> Option<PulseCleanupJobSnapshot> {
    ARKPULSE_CLEANUP_JOBS.read().await.get(job_id).cloned()
}

pub(super) fn describe_arkpulse_remediation(
    remediation: Option<&crate::sentinel::DoctorRemediationSpec>,
    fix_command: &str,
) -> String {
    let normalized = fix_command.trim();
    if !normalized.is_empty() {
        return normalized.to_string();
    }
    match remediation {
        Some(crate::sentinel::DoctorRemediationSpec::TunnelStartVerify) => {
            "Start tunnel and verify /tunnel/status returns active + URL".to_string()
        }
        Some(crate::sentinel::DoctorRemediationSpec::TunnelRestartVerify) => {
            "Restart tunnel and verify public reachability".to_string()
        }
        Some(crate::sentinel::DoctorRemediationSpec::AppRestart { app_id }) => {
            format!("Restart app {} and re-check health", app_id)
        }
        Some(crate::sentinel::DoctorRemediationSpec::ReadonlyInvestigation { topic }) => {
            describe_arkpulse_readonly_investigation(topic)
        }
        Some(crate::sentinel::DoctorRemediationSpec::ManagedAppOperation { app_id, operation }) => {
            describe_arkpulse_managed_app_operation(app_id, operation)
        }
        Some(crate::sentinel::DoctorRemediationSpec::ShellCommand { command }) => {
            command.trim().to_string()
        }
        None => String::new(),
    }
}

pub(super) fn describe_arkpulse_readonly_investigation(
    topic: &crate::sentinel::DoctorReadonlyInvestigationTopic,
) -> String {
    match topic {
        crate::sentinel::DoctorReadonlyInvestigationTopic::MemoryCaptureHealth => {
            "Review failed memory captures and model health".to_string()
        }
    }
}

pub(super) fn describe_arkpulse_managed_app_operation(
    app_id: &str,
    operation: &crate::sentinel::DoctorManagedAppOperation,
) -> String {
    match operation {
        crate::sentinel::DoctorManagedAppOperation::CompilePythonRequirements => {
            format!("Compile pinned Python requirements for app {}", app_id)
        }
        crate::sentinel::DoctorManagedAppOperation::GenerateCargoLockfile => {
            format!("Generate Cargo.lock for app {}", app_id)
        }
        crate::sentinel::DoctorManagedAppOperation::RemoveNpmInstallHooks => {
            format!("Remove npm install lifecycle hooks from app {}", app_id)
        }
    }
}

pub(super) fn apply_arkpulse_sanitized_env(command: &mut tokio::process::Command) {
    const SAFE_ENV_KEYS: &[&str] = &[
        "PATH",
        "PATHEXT",
        "SYSTEMROOT",
        "SystemRoot",
        "COMSPEC",
        "ComSpec",
        "TMP",
        "TEMP",
        "HOME",
        "USERPROFILE",
        "APPDATA",
        "LOCALAPPDATA",
        "PROGRAMDATA",
        "ProgramData",
        "LANG",
        "LC_ALL",
        "LC_CTYPE",
        "CARGO_HOME",
        "RUSTUP_HOME",
        "RUSTUP_TOOLCHAIN",
        "NPM_CONFIG_CACHE",
        "npm_config_cache",
        "PIP_CACHE_DIR",
    ];

    command.env_clear();
    for key in SAFE_ENV_KEYS {
        if let Ok(value) = std::env::var(key) {
            command.env(key, value);
        }
    }
}

type PulseFixHttpResult = (StatusCode, serde_json::Value);

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(super) enum HealSkipReason {
    MissingOnDisk,
    CorruptMetadata,
    ExplicitlyStopped,
}

pub(super) enum HealViability {
    Recoverable,
    NotRecoverable(HealSkipReason),
    TransientlyUnknown(String),
}

#[derive(Default)]
pub(super) struct DeletedAppCleanupSummary {
    pub(super) deleted_notifications: u64,
    pub(super) deleted_pulse_events: u64,
    pub(super) deleted_reflect_units: u64,
}

pub(super) fn arkpulse_error_result(
    status: StatusCode,
    error: impl Into<String>,
) -> PulseFixHttpResult {
    let error = error.into();
    (
        status,
        serde_json::json!({
            "status": "error",
            "error": error,
        }),
    )
}

pub(super) async fn resolve_arkpulse_app_dir(
    state: &AppState,
    raw_app_dir: &str,
) -> Result<PathBuf, String> {
    let trimmed = raw_app_dir.trim();
    if trimmed.is_empty() {
        return Err("App directory is required for Pulse shell operations".to_string());
    }
    let requested = tokio::fs::canonicalize(PathBuf::from(trimmed))
        .await
        .map_err(|e| format!("App directory is not accessible: {}", e))?;
    let apps_root = {
        let agent = state.agent.read().await;
        agent.data_dir.join("apps")
    };
    let apps_root = tokio::fs::canonicalize(&apps_root)
        .await
        .map_err(|e| format!("Managed apps root is not accessible: {}", e))?;
    if !requested.starts_with(&apps_root) {
        return Err("Pulse fixes may only run inside the managed apps directory".to_string());
    }
    Ok(requested)
}

pub(super) async fn resolve_arkpulse_app_dir_by_id(
    state: &AppState,
    app_id: &str,
) -> Result<PathBuf, String> {
    let trimmed = app_id.trim();
    if !is_valid_app_id(trimmed) {
        return Err("App id is required for Pulse app remediation".to_string());
    }
    let registry_dir = state.app_registry.get_dir(trimmed).await;
    let fallback_dir = {
        let agent = state.agent.read().await;
        agent.data_dir().join("apps").join(trimmed)
    };
    let app_dir = registry_dir.unwrap_or(fallback_dir);
    resolve_arkpulse_app_dir(state, &app_dir.display().to_string()).await
}

pub(super) async fn run_arkpulse_process(
    app_dir: &FsPath,
    program: &str,
    args: &[String],
) -> Result<serde_json::Value, String> {
    let mut command = tokio::process::Command::new(program);
    apply_arkpulse_sanitized_env(&mut command);
    command
        .args(args)
        .current_dir(app_dir)
        .stdin(std::process::Stdio::null())
        .stdout(std::process::Stdio::piped())
        .stderr(std::process::Stdio::piped())
        .kill_on_drop(true);
    let output = tokio::time::timeout(Duration::from_secs(120), command.output())
        .await
        .map_err(|_| format!("{} timed out after 120 seconds", program))?
        .map_err(|e| format!("failed to start {}: {}", program, e))?;
    let stdout = String::from_utf8_lossy(&output.stdout).trim().to_string();
    let stderr = String::from_utf8_lossy(&output.stderr).trim().to_string();
    if !output.status.success() {
        let detail = if !stderr.is_empty() { stderr } else { stdout };
        return Err(format!(
            "{} failed with exit code {:?}: {}",
            program,
            output.status.code(),
            detail
        ));
    }
    Ok(serde_json::json!({
        "program": program,
        "args": args,
        "stdout": truncate_for_response(&stdout, 800),
        "stderr": truncate_for_response(&stderr, 800),
    }))
}

pub(super) async fn run_arkpulse_managed_app_operation_fix(
    state: &AppState,
    app_id: &str,
    operation: &crate::sentinel::DoctorManagedAppOperation,
) -> PulseFixHttpResult {
    let app_dir = match resolve_arkpulse_app_dir_by_id(state, app_id).await {
        Ok(path) => path,
        Err(error) => return arkpulse_error_result(StatusCode::BAD_REQUEST, error),
    };

    let (program, args): (&str, Vec<String>) = match operation {
        crate::sentinel::DoctorManagedAppOperation::CompilePythonRequirements => {
            ("pip-compile", vec!["requirements.txt".to_string()])
        }
        crate::sentinel::DoctorManagedAppOperation::GenerateCargoLockfile => {
            ("cargo", vec!["generate-lockfile".to_string()])
        }
        crate::sentinel::DoctorManagedAppOperation::RemoveNpmInstallHooks => (
            "npm",
            vec![
                "pkg".to_string(),
                "delete".to_string(),
                "scripts.preinstall".to_string(),
                "scripts.install".to_string(),
                "scripts.postinstall".to_string(),
            ],
        ),
    };

    match run_arkpulse_process(&app_dir, program, &args).await {
        Ok(detail) => (
            StatusCode::OK,
            serde_json::json!({
                "status": "ok",
                "mode": "managed_app_operation",
                "app_id": app_id,
                "operation": operation,
                "message": describe_arkpulse_managed_app_operation(app_id, operation),
                "detail": detail,
            }),
        ),
        Err(error) => arkpulse_error_result(StatusCode::INTERNAL_SERVER_ERROR, error),
    }
}

pub(super) async fn lookup_app_title_for_cleanup(state: &AppState, app_id: &str) -> Option<String> {
    let apps = state.app_registry.list().await;
    apps.iter()
        .find(|row| row.get("id").and_then(|value| value.as_str()) == Some(app_id))
        .and_then(|row| row.get("title").and_then(|value| value.as_str()))
        .map(|value| value.to_string())
}

pub(super) async fn cleanup_deleted_app_references(
    state: &AppState,
    app_id: &str,
    app_title: Option<&str>,
) -> DeletedAppCleanupSummary {
    state.app_registry.purge_deleted_app_state(app_id).await;
    let storage = {
        let agent = state.agent.read().await;
        agent.storage.clone()
    };
    let deleted_notifications = match storage.delete_app_notifications(app_id, app_title).await {
        Ok(count) => count,
        Err(error) => {
            tracing::warn!(
                "Failed to delete app notifications while cleaning '{}': {}",
                app_id,
                error
            );
            0
        }
    };
    let deleted_pulse_events =
        match crate::sentinel::delete_app_referenced_pulse_events(&storage, app_id).await {
            Ok(count) => count,
            Err(error) => {
                tracing::warn!(
                    "Failed to prune Pulse history while cleaning '{}': {}",
                    app_id,
                    error
                );
                0
            }
        };
    let deleted_reflect_units = match storage
        .delete_semantic_work_units_for_source("app", app_id)
        .await
    {
        Ok(count) => count,
        Err(error) => {
            tracing::warn!(
                "Failed to prune Reflect rows while cleaning '{}': {}",
                app_id,
                error
            );
            0
        }
    };
    DeletedAppCleanupSummary {
        deleted_notifications,
        deleted_pulse_events,
        deleted_reflect_units,
    }
}

pub(super) fn heal_skip_reason_code(reason: HealSkipReason) -> &'static str {
    match reason {
        HealSkipReason::MissingOnDisk => "missing_on_disk",
        HealSkipReason::CorruptMetadata => "corrupt_metadata",
        HealSkipReason::ExplicitlyStopped => "explicitly_stopped",
    }
}

pub(super) async fn assess_auto_heal_viability(state: &AppState, app_id: &str) -> HealViability {
    let registry_dir = state.app_registry.get_dir(app_id).await;
    let app_dir = if let Some(path) = registry_dir {
        path
    } else {
        let data_dir = {
            let agent = state.agent.read().await;
            agent.data_dir().to_path_buf()
        };
        data_dir.join("apps").join(app_id)
    };
    match tokio::fs::metadata(&app_dir).await {
        Ok(metadata) => {
            if !metadata.is_dir() {
                return HealViability::NotRecoverable(HealSkipReason::MissingOnDisk);
            }
        }
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => {
            return HealViability::NotRecoverable(HealSkipReason::MissingOnDisk);
        }
        Err(error) => {
            return HealViability::TransientlyUnknown(format!(
                "Could not inspect app directory for {}: {}",
                app_id, error
            ));
        }
    }
    if state.app_registry.get_dir(app_id).await.is_some()
        && !state.app_registry.is_enabled(app_id).await
    {
        return HealViability::NotRecoverable(HealSkipReason::ExplicitlyStopped);
    }
    match tokio::fs::read(app_dir.join(".app_meta.json")).await {
        Ok(bytes) => {
            if serde_json::from_slice::<serde_json::Value>(&bytes).is_err() {
                return HealViability::NotRecoverable(HealSkipReason::CorruptMetadata);
            }
        }
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => {
            return HealViability::NotRecoverable(HealSkipReason::CorruptMetadata);
        }
        Err(error) => {
            return HealViability::TransientlyUnknown(format!(
                "Could not read app metadata for {}: {}",
                app_id, error
            ));
        }
    }
    HealViability::Recoverable
}

pub(super) fn spawn_arkpulse_app_restart_fix(
    state: AppState,
    app_id: String,
    issue_title: String,
    target: String,
    fix_summary: String,
    event_timestamp: Option<String>,
    finding_index: Option<usize>,
) {
    crate::spawn_logged!("src/channels/http.rs:13879", async move {
        let started_at = std::time::Instant::now();
        let result = match tokio::time::timeout(
            Duration::from_secs(45),
            run_arkpulse_app_restart_fix(&state, &app_id),
        )
        .await
        {
            Ok(result) => result,
            Err(_) => arkpulse_error_result(
                StatusCode::GATEWAY_TIMEOUT,
                format!(
                    "App restart for {} timed out. Pulse aborted the remediation to keep the control plane responsive.",
                    app_id
                ),
            ),
        };
        let status = result.0;
        let body = result.1;
        let latency_ms = started_at.elapsed().as_millis().min(i64::MAX as u128) as i64;
        let plan = PulseFixPlan::AppRestart(app_id.clone());
        let audit = PulseFixAuditDetails {
            plan: &plan,
            issue_title: &issue_title,
            target: &target,
            fix_summary: &fix_summary,
            event_timestamp: event_timestamp.as_deref(),
            finding_index,
        };
        persist_arkpulse_fix_audit(&state, &audit, latency_ms, status, &body).await;
    });
}

pub(super) async fn run_arkpulse_app_restart_fix(
    state: &AppState,
    app_id: &str,
) -> PulseFixHttpResult {
    let response = restart_app(State(state.clone()), Path(app_id.to_string())).await;
    let status = response.status();
    let body = response.into_body();
    let body_bytes = match axum::body::to_bytes(body, 1024 * 1024).await {
        Ok(bytes) => bytes,
        Err(error) => {
            return arkpulse_error_result(
                StatusCode::INTERNAL_SERVER_ERROR,
                format!("Failed to read app restart response: {}", error),
            );
        }
    };
    let payload = serde_json::from_slice::<serde_json::Value>(&body_bytes).unwrap_or_else(|_| {
        serde_json::json!({
            "raw": String::from_utf8_lossy(&body_bytes).to_string()
        })
    });

    if !status.is_success() {
        let error = payload
            .get("error")
            .and_then(|value| value.as_str())
            .or_else(|| payload.get("message").and_then(|value| value.as_str()))
            .unwrap_or("Failed to restart app");
        return (
            status,
            serde_json::json!({
                "status": "error",
                "mode": "app_restart",
                "app_id": app_id,
                "error": error,
                "details": payload,
            }),
        );
    }

    let title = payload
        .get("title")
        .and_then(|value| value.as_str())
        .unwrap_or(app_id);
    let url = payload
        .get("url")
        .and_then(|value| value.as_str())
        .unwrap_or_default();
    (
        StatusCode::OK,
        serde_json::json!({
            "status": "ok",
            "mode": "app_restart",
            "app_id": app_id,
            "message": format!("Restarted app {}.", title),
            "url": url,
            "details": payload,
        }),
    )
}

pub(super) async fn run_arkpulse_readonly_investigation_fix(
    state: &AppState,
    topic: &crate::sentinel::DoctorReadonlyInvestigationTopic,
) -> PulseFixHttpResult {
    match topic {
        crate::sentinel::DoctorReadonlyInvestigationTopic::MemoryCaptureHealth => {
            let (storage, config) = {
                let agent = state.agent.read().await;
                (agent.storage.clone(), agent.config.clone())
            };
            let failed_count = match storage
                .count_memory_capture_events_by_statuses_all_scopes(&["failed"])
                .await
            {
                Ok(count) => count,
                Err(error) => {
                    return arkpulse_error_result(
                        StatusCode::INTERNAL_SERVER_ERROR,
                        format!("Failed to count memory capture events: {}", error),
                    );
                }
            };
            let capture_query = crate::storage::ReadonlyTableQuery {
                table: "memory_capture_events".to_string(),
                columns: vec![
                    "id".to_string(),
                    "status".to_string(),
                    "capture_kind".to_string(),
                    "conversation_id".to_string(),
                    "created_at".to_string(),
                    "updated_at".to_string(),
                    "next_retry_at".to_string(),
                    "completed_at".to_string(),
                ],
                filters: vec![crate::storage::ReadonlyTableFilter {
                    column: "status".to_string(),
                    op: "eq".to_string(),
                    value: Some(serde_json::json!("failed")),
                }],
                order_by: vec![crate::storage::ReadonlyTableSort {
                    column: "updated_at".to_string(),
                    direction: Some("desc".to_string()),
                }],
                limit: Some(10),
            };
            let capture_rows = match storage.query_table_json(&capture_query).await {
                Ok(payload) => payload
                    .get("rows")
                    .and_then(|value| value.as_array())
                    .cloned()
                    .unwrap_or_default(),
                Err(error) => {
                    return arkpulse_error_result(
                        StatusCode::INTERNAL_SERVER_ERROR,
                        format!("Failed to inspect memory capture events: {}", error),
                    );
                }
            };
            let model_failover = match crate::core::model_failover::ModelFailoverControlPlane::list(
                &storage,
            )
            .await
            {
                Ok(value) => value,
                Err(error) => {
                    return arkpulse_error_result(
                        StatusCode::INTERNAL_SERVER_ERROR,
                        format!("Failed to inspect model health: {}", error),
                    );
                }
            };
            let chat_model_ready = crate::core::chat_model_is_configured(&config);
            let provider_summary = &model_failover.summary;
            let provider_issues = model_failover
                .provider_health
                .iter()
                .filter(|record| {
                    record.disabled
                        || record.cooldown_until.is_some()
                        || record.failure_count > 0
                        || record.last_error.is_some()
                })
                .take(3)
                .map(|record| {
                    let mut parts = Vec::new();
                    parts.push(record.provider_id.clone());
                    if record.disabled {
                        parts.push("disabled".to_string());
                    }
                    if record.cooldown_until.is_some() {
                        parts.push("cooling".to_string());
                    }
                    if record.failure_count > 0 {
                        parts.push(format!("failures={}", record.failure_count));
                    }
                    if let Some(error) = record.last_error.as_deref() {
                        parts.push(format!("last_error={}", truncate_for_response(error, 120)));
                    }
                    parts.join(", ")
                })
                .collect::<Vec<_>>();
            let recent_capture_lines = capture_rows
                .iter()
                .take(3)
                .map(|row| {
                    let updated_at = row
                        .get("updated_at")
                        .and_then(|value| value.as_str())
                        .unwrap_or("unknown time");
                    let capture_kind = row
                        .get("capture_kind")
                        .and_then(|value| value.as_str())
                        .unwrap_or("capture");
                    let conversation_id = row
                        .get("conversation_id")
                        .and_then(|value| value.as_str())
                        .map(|value| format!("conversation={}", value))
                        .unwrap_or_else(|| "conversation=global".to_string());
                    format!("- {} | {} | {}", updated_at, capture_kind, conversation_id)
                })
                .collect::<Vec<_>>();
            let mut output_lines = vec![format!(
                "Failed memory captures: {} total failed event(s); {} recent row(s) reviewed.",
                failed_count,
                capture_rows.len()
            )];
            if chat_model_ready {
                output_lines.push(format!(
                    "Model health: {} provider(s) tracked, {} disabled, {} cooling.",
                    provider_summary.providers,
                    provider_summary.disabled_providers,
                    provider_summary.cooling_providers
                ));
            } else {
                output_lines.push(
                    "Model health: no chat-capable model is configured, so memory capture cannot succeed until a model slot is available."
                        .to_string(),
                );
            }
            if !provider_issues.is_empty() {
                output_lines.push(format!(
                    "Providers needing attention: {}.",
                    provider_issues.join("; ")
                ));
            }
            if !recent_capture_lines.is_empty() {
                output_lines.push("Recent failed captures:".to_string());
                output_lines.extend(recent_capture_lines);
            }
            (
                StatusCode::OK,
                serde_json::json!({
                    "status": "ok",
                    "mode": "readonly_investigation",
                    "topic": "memory_capture_health",
                    "message": "Pulse diagnostic completed.",
                    "output": output_lines.join("\n"),
                    "details": {
                        "failed_count": failed_count,
                        "recent_failed_captures": capture_rows,
                        "chat_model_configured": chat_model_ready,
                        "model_failover_summary": provider_summary,
                        "provider_health": model_failover.provider_health,
                    }
                }),
            )
        }
    }
}

pub(super) fn arkpulse_fix_plan_label(plan: &PulseFixPlan) -> &'static str {
    match plan {
        PulseFixPlan::TunnelStartVerify => "tunnel_start_verify",
        PulseFixPlan::TunnelRestartVerify => "tunnel_restart_verify",
        PulseFixPlan::AppRestart(_) => "app_restart",
        PulseFixPlan::ManagedAppOperation { .. } => "managed_app_operation",
        PulseFixPlan::ReadonlyInvestigation { .. } => "readonly_investigation",
    }
}

pub(super) struct PulseFixAuditDetails<'a> {
    plan: &'a PulseFixPlan,
    issue_title: &'a str,
    target: &'a str,
    fix_summary: &'a str,
    event_timestamp: Option<&'a str>,
    finding_index: Option<usize>,
}

pub(super) async fn persist_arkpulse_fix_audit(
    state: &AppState,
    audit: &PulseFixAuditDetails<'_>,
    latency_ms: i64,
    status: StatusCode,
    body: &serde_json::Value,
) {
    let storage = {
        let agent = state.agent.read().await;
        agent.storage.clone()
    };
    let success = status.is_success()
        && body
            .get("status")
            .and_then(|value| value.as_str())
            .map(|value| !value.eq_ignore_ascii_case("error"))
            .unwrap_or(true);
    let mode = body
        .get("mode")
        .and_then(|value| value.as_str())
        .unwrap_or_else(|| arkpulse_fix_plan_label(audit.plan));
    let arguments = serde_json::json!({
        "issue_title": audit.issue_title,
        "target": audit.target,
        "fix_summary": truncate_for_response(audit.fix_summary, 400),
        "plan": arkpulse_fix_plan_label(audit.plan),
        "event_timestamp": audit.event_timestamp,
        "finding_index": audit.finding_index,
    });
    let payload = truncate_for_response(&body.to_string(), 4000);
    let log = crate::storage::operational_log::Model {
        id: uuid::Uuid::new_v4().to_string(),
        created_at: chrono::Utc::now().to_rfc3339(),
        trace_id: None,
        conversation_id: None,
        channel: "web".to_string(),
        event_type: "arkpulse_fix".to_string(),
        success,
        outcome: if success {
            format!("Pulse fix completed ({mode})")
        } else {
            format!("Pulse fix failed ({mode})")
        },
        tool_name: Some(mode.to_string()),
        latency_ms: Some(latency_ms),
        arguments: Some(arguments.to_string()),
        payload: Some(payload),
        strategy_version: None,
        policy_version: None,
        prompt_version: None,
        model_slot: None,
    };
    if let Err(error) = storage.insert_operational_log(&log).await {
        tracing::warn!("Failed to persist Pulse fix audit log: {}", error);
    }
}

pub(super) struct PulseFixResponseContext<'a> {
    audit: PulseFixAuditDetails<'a>,
    started_at: std::time::Instant,
}

pub(super) async fn respond_arkpulse_fix(
    state: &AppState,
    context: PulseFixResponseContext<'_>,
    result: PulseFixHttpResult,
) -> Response {
    let (status, body) = result;
    let latency_ms = context
        .started_at
        .elapsed()
        .as_millis()
        .min(i64::MAX as u128) as i64;
    persist_arkpulse_fix_audit(state, &context.audit, latency_ms, status, &body).await;
    (status, Json(body)).into_response()
}

pub(super) async fn arkpulse_cleanup_preview(
    State(state): State<AppState>,
    Json(_request): Json<PulseCleanupPreviewRequest>,
) -> Response {
    let state_for_worker = state.clone();
    let worker = tokio::spawn(async move {
        collect_arkpulse_cleanup_candidates_for_state(&state_for_worker).await
    });
    let candidates = match tokio::time::timeout(Duration::from_secs(20), worker).await {
        Ok(Ok(Ok(candidates))) => candidates,
        Ok(Ok(Err(error))) => {
            return (
                StatusCode::INTERNAL_SERVER_ERROR,
                Json(serde_json::json!({ "error": error.to_string() })),
            )
                .into_response();
        }
        Ok(Err(error)) => {
            return (
                StatusCode::INTERNAL_SERVER_ERROR,
                Json(serde_json::json!({ "error": format!("cleanup preview worker failed: {}", error) })),
            )
                .into_response();
        }
        Err(_) => {
            return (
                StatusCode::ACCEPTED,
                Json(serde_json::json!({
                    "status": "running",
                    "message": "Cleanup preview is still running on the Pulse worker. Try again shortly."
                })),
            )
                .into_response();
        }
    };

    let total_size_bytes: u64 = candidates
        .iter()
        .map(|candidate| candidate.size_bytes)
        .sum();
    let category_counts = crate::core::artifact_hygiene::candidate_category_counts(&candidates)
        .into_iter()
        .map(|(category, count, size_bytes)| {
            serde_json::json!({
                "category": category,
                "count": count,
                "size_bytes": size_bytes,
            })
        })
        .collect::<Vec<_>>();
    Json(serde_json::json!({
        "status": "ok",
        "archive_root": "data_dir/artifact_archive",
        "legacy_archive_root": "data_dir/app_archive",
        "archive_retention_days": crate::core::artifact_hygiene::ARCHIVE_RETENTION_DAYS,
        "candidates": candidates,
        "category_counts": category_counts,
        "total_size_bytes": total_size_bytes,
        "active_job": cleanup_active_job_snapshot().await,
    }))
    .into_response()
}

pub(super) async fn get_arkpulse_cleanup_job(Path(job_id): Path<String>) -> Response {
    match get_cleanup_job_snapshot(job_id.trim()).await {
        Some(snapshot) => Json(snapshot).into_response(),
        None => (
            StatusCode::NOT_FOUND,
            Json(serde_json::json!({ "error": "Cleanup job not found" })),
        )
            .into_response(),
    }
}

pub(super) async fn run_arkpulse_cleanup(
    State(state): State<AppState>,
    Json(request): Json<RunArkPulseCleanupRequest>,
) -> Response {
    if !request.confirm_archive {
        return (
            StatusCode::BAD_REQUEST,
            Json(serde_json::json!({
                "error": "confirm_archive must be true before Pulse archives managed artifacts"
            })),
        )
            .into_response();
    }
    if ARKPULSE_CLEANUP_ACTIVE
        .compare_exchange(false, true, Ordering::AcqRel, Ordering::Acquire)
        .is_err()
    {
        return (
            StatusCode::CONFLICT,
            Json(serde_json::json!({
                "status": "running",
                "message": "An Pulse cleanup worker is already running.",
                "active_job": cleanup_active_job_snapshot().await,
            })),
        )
            .into_response();
    }

    let candidate_ids = request
        .candidate_ids
        .into_iter()
        .map(|value| value.trim().to_string())
        .filter(|value| !value.is_empty())
        .collect::<Vec<_>>();
    let job_id = uuid::Uuid::new_v4().to_string();
    let queued_at = chrono::Utc::now().to_rfc3339();
    let snapshot = PulseCleanupJobSnapshot {
        job_id: job_id.clone(),
        status: "queued".to_string(),
        queued_at,
        started_at: None,
        finished_at: None,
        requested_count: candidate_ids.len(),
        archived_count: 0,
        archived_bytes: 0,
        skipped_count: 0,
        archived: Vec::new(),
        errors: Vec::new(),
        retention: None,
    };
    upsert_cleanup_job(snapshot.clone()).await;

    let state_for_worker = state.clone();
    let event_timestamp = request.event_timestamp;
    let finding_index = request.finding_index;
    crate::spawn_logged!(
        "src/channels/http/arkpulse_control.rs:cleanup_worker",
        async move {
            execute_arkpulse_cleanup_job(
                state_for_worker,
                snapshot,
                candidate_ids,
                event_timestamp,
                finding_index,
            )
            .await;
            ARKPULSE_CLEANUP_ACTIVE.store(false, Ordering::Release);
        }
    );

    (
        StatusCode::ACCEPTED,
        Json(serde_json::json!({
            "status": "accepted",
            "message": "Pulse cleanup is running on its background worker.",
            "job_id": job_id,
        })),
    )
        .into_response()
}

async fn execute_arkpulse_cleanup_job(
    state: AppState,
    mut snapshot: PulseCleanupJobSnapshot,
    candidate_ids: Vec<String>,
    event_timestamp: Option<String>,
    finding_index: Option<usize>,
) {
    snapshot.status = "running".to_string();
    snapshot.started_at = Some(chrono::Utc::now().to_rfc3339());
    upsert_cleanup_job(snapshot.clone()).await;

    let result =
        run_arkpulse_cleanup_worker(&state, &candidate_ids, event_timestamp, finding_index).await;
    match result {
        Ok((archived, skipped_count, errors, retention)) => {
            snapshot.status = if errors.is_empty() {
                "completed".to_string()
            } else {
                "completed_with_errors".to_string()
            };
            snapshot.archived_bytes = archived
                .iter()
                .map(|outcome| outcome.size_bytes)
                .sum::<u64>();
            snapshot.archived_count = archived.len();
            snapshot.skipped_count = skipped_count;
            snapshot.archived = archived;
            snapshot.errors = errors;
            snapshot.retention = Some(retention);
        }
        Err(error) => {
            snapshot.status = "failed".to_string();
            snapshot.errors = vec![error.to_string()];
        }
    }
    snapshot.finished_at = Some(chrono::Utc::now().to_rfc3339());
    upsert_cleanup_job(snapshot).await;
}

async fn run_arkpulse_cleanup_worker(
    state: &AppState,
    candidate_ids: &[String],
    event_timestamp: Option<String>,
    finding_index: Option<usize>,
) -> Result<(
    Vec<crate::core::artifact_hygiene::ArchivedArtifactOutcome>,
    usize,
    Vec<String>,
    crate::core::artifact_hygiene::ArchiveRetentionSummary,
)> {
    let data_dir = {
        let agent = state.agent.read().await;
        agent.data_dir().to_path_buf()
    };
    let candidates = collect_arkpulse_cleanup_candidates_for_state(state).await?;
    let selected_ids = candidate_ids.iter().cloned().collect::<HashSet<String>>();
    let candidate_ids_by_id = candidates
        .iter()
        .map(|candidate| candidate.id.clone())
        .collect::<HashSet<_>>();
    let mut skipped_count = selected_ids
        .iter()
        .filter(|id| !candidate_ids_by_id.contains(*id))
        .count();
    let selected = candidates
        .into_iter()
        .filter(|candidate| {
            if selected_ids.is_empty() {
                candidate.selected_by_default
            } else {
                selected_ids.contains(&candidate.id)
            }
        })
        .collect::<Vec<_>>();

    let mut archived = Vec::new();
    let mut errors = Vec::new();
    for candidate in selected {
        if let Some(app_id) = candidate.app_id.as_deref() {
            if candidate.requires_app_stop {
                if let Err(error) = stop_app_runtime_for_artifact_cleanup(state, app_id).await {
                    errors.push(format!(
                        "{}: failed to stop app runtime before archive: {}",
                        candidate.path_label, error
                    ));
                    skipped_count += 1;
                    continue;
                }
            }
        }
        match crate::core::artifact_hygiene::archive_cleanup_candidate(
            &data_dir,
            &candidate,
            event_timestamp.clone(),
            finding_index,
        )
        .await
        {
            Ok(outcome) => {
                if let Some(app_id) = candidate.app_id.as_deref() {
                    if let Err(error) = state.app_registry.stop(app_id).await {
                        errors.push(format!(
                            "{}: archived but failed to remove app registry entry: {}",
                            candidate.path_label, error
                        ));
                    }
                }
                archived.push(outcome);
            }
            Err(error) => {
                errors.push(format!("{}: {}", candidate.path_label, error));
                skipped_count += 1;
            }
        }
    }

    let retention = crate::core::artifact_hygiene::prune_archive_retention(&data_dir).await?;
    Ok((archived, skipped_count, errors, retention))
}

async fn stop_app_runtime_for_artifact_cleanup(
    state: &AppState,
    app_id: &str,
) -> Result<(), String> {
    if let Some(executor) = state.executor_client.as_ref() {
        match executor
            .request(
                reqwest::Method::POST,
                &format!("/internal/v1/apps/{}/stop", app_id),
            )
            .json(&crate::clients::AppLifecycleRequest {
                title: None,
                query: None,
            })
            .send()
            .await
        {
            Ok(response)
                if response.status().is_success()
                    || response.status() == reqwest::StatusCode::NOT_FOUND => {}
            Ok(response) => {
                return Err(format!("executor returned {}", response.status()));
            }
            Err(error) => return Err(error.to_string()),
        }
    } else if let Err(error) = state.app_registry.stop_runtime(app_id).await {
        return Err(error.to_string());
    }
    Ok(())
}

/// Execute a supported Pulse remediation directly (without going through Chat).
pub(super) async fn run_arkpulse_fix(
    State(state): State<AppState>,
    Json(request): Json<RunArkPulseFixRequest>,
) -> Response {
    let RunArkPulseFixRequest {
        fix_command,
        remediation,
        issue_title,
        target,
        event_timestamp,
        finding_index,
    } = request;
    let started_at = std::time::Instant::now();

    let request_fix_command = fix_command.trim().to_string();
    let request_has_event_context = event_timestamp
        .as_deref()
        .is_some_and(|value| !value.trim().is_empty())
        || finding_index.is_some();
    if request_fix_command.is_empty() && remediation.is_none() && !request_has_event_context {
        return (
            StatusCode::BAD_REQUEST,
            Json(ErrorResponse {
                error: "structured remediation or Pulse finding context is required".to_string(),
            }),
        )
            .into_response();
    }

    let mut effective_fix_command = request_fix_command.clone();
    let mut effective_remediation = remediation.clone();
    let mut selected_event_timestamp: Option<String> = None;
    let mut selected_finding_index: Option<usize> = None;

    let plan = if event_timestamp.is_some() || finding_index.is_some() {
        let event_timestamp = event_timestamp
            .as_deref()
            .map(str::trim)
            .filter(|value| !value.is_empty());
        let Some(event_timestamp) = event_timestamp else {
            return (
                StatusCode::BAD_REQUEST,
                Json(ErrorResponse {
                    error: "event_timestamp is required when finding_index is provided".to_string(),
                }),
            )
                .into_response();
        };
        let Some(finding_index) = finding_index else {
            return (
                StatusCode::BAD_REQUEST,
                Json(ErrorResponse {
                    error: "finding_index is required when event_timestamp is provided".to_string(),
                }),
            )
                .into_response();
        };

        let agent = state.agent.read().await;
        let events = crate::sentinel::get_pulse_log(&agent).await;
        let Some(event) = events
            .iter()
            .find(|event| event.timestamp == event_timestamp)
        else {
            return (
                StatusCode::NOT_FOUND,
                Json(ErrorResponse {
                    error: "Pulse event not found".to_string(),
                }),
            )
                .into_response();
        };
        let Some(finding) = event.details.doctor_findings.get(finding_index) else {
            return (
                StatusCode::NOT_FOUND,
                Json(ErrorResponse {
                    error: "Pulse finding index is out of range".to_string(),
                }),
            )
                .into_response();
        };
        if !finding.user_actionable {
            return (
                StatusCode::BAD_REQUEST,
                Json(ErrorResponse {
                    error: "This Pulse finding is advisory-only and must be fixed manually"
                        .to_string(),
                }),
            )
                .into_response();
        }
        selected_event_timestamp = Some(event_timestamp.to_string());
        selected_finding_index = Some(finding_index);
        effective_fix_command = finding.fix_command.trim().to_string();
        effective_remediation = finding.remediation.clone();
        effective_remediation
            .as_ref()
            .and_then(|value| arkpulse_fix_plan_from_remediation(value, true))
    } else {
        effective_remediation
            .as_ref()
            .and_then(|value| arkpulse_fix_plan_from_remediation(value, false))
    };

    let Some(plan) = plan else {
        return (
            StatusCode::BAD_REQUEST,
            Json(ErrorResponse {
                error: "This finding has no executable Pulse auto-fix.".to_string(),
            }),
        )
            .into_response();
    };

    if matches!(plan, PulseFixPlan::ManagedAppOperation { .. }) && !request_has_event_context {
        return (
            StatusCode::BAD_REQUEST,
            Json(ErrorResponse {
                error: "Managed app Pulse fixes must come from a stored Pulse finding.".to_string(),
            }),
        )
            .into_response();
    }

    let issue_title = issue_title.unwrap_or_default();
    let target = target.unwrap_or_default();
    let fix_summary =
        describe_arkpulse_remediation(effective_remediation.as_ref(), &effective_fix_command);
    tracing::info!(
        "Pulse fix requested: issue='{}' target='{}' command='{}'",
        issue_title,
        target,
        truncate_for_response(&fix_summary, 220)
    );

    let audit = PulseFixAuditDetails {
        plan: &plan,
        issue_title: &issue_title,
        target: &target,
        fix_summary: &fix_summary,
        event_timestamp: selected_event_timestamp.as_deref(),
        finding_index: selected_finding_index,
    };

    if let PulseFixPlan::AppRestart(app_id) = &plan {
        match assess_auto_heal_viability(&state, app_id).await {
            HealViability::Recoverable => {
                spawn_arkpulse_app_restart_fix(
                    state.clone(),
                    app_id.clone(),
                    issue_title.clone(),
                    target.clone(),
                    fix_summary.clone(),
                    selected_event_timestamp.clone(),
                    selected_finding_index,
                );
                return (
                    StatusCode::ACCEPTED,
                    Json(serde_json::json!({
                        "status": "accepted",
                        "mode": "app_restart",
                        "app_id": app_id,
                        "message": "Queued app restart. Pulse will apply it in the background while the control plane stays responsive."
                    })),
                )
                    .into_response();
            }
            HealViability::NotRecoverable(HealSkipReason::ExplicitlyStopped) => {
                let result = (
                    StatusCode::OK,
                    serde_json::json!({
                        "status": "ok",
                        "mode": "app_restart",
                        "app_id": app_id,
                        "skipped": true,
                        "reason": heal_skip_reason_code(HealSkipReason::ExplicitlyStopped),
                        "message": "App restart skipped because the app is explicitly stopped."
                    }),
                );
                return respond_arkpulse_fix(
                    &state,
                    PulseFixResponseContext { audit, started_at },
                    result,
                )
                .await;
            }
            HealViability::NotRecoverable(reason) => {
                let app_title = lookup_app_title_for_cleanup(&state, app_id).await;
                let cleanup =
                    cleanup_deleted_app_references(&state, app_id, app_title.as_deref()).await;
                let result = (
                    StatusCode::OK,
                    serde_json::json!({
                        "status": "ok",
                        "mode": "app_restart",
                        "app_id": app_id,
                        "skipped": true,
                        "reason": heal_skip_reason_code(reason),
                        "message": "App is no longer recoverable. Cleaned stale state and skipped restart.",
                        "deleted_notifications": cleanup.deleted_notifications,
                        "deleted_pulse_events": cleanup.deleted_pulse_events
                    }),
                );
                return respond_arkpulse_fix(
                    &state,
                    PulseFixResponseContext { audit, started_at },
                    result,
                )
                .await;
            }
            HealViability::TransientlyUnknown(error) => {
                let result = (
                    StatusCode::SERVICE_UNAVAILABLE,
                    serde_json::json!({
                        "status": "error",
                        "mode": "app_restart",
                        "app_id": app_id,
                        "reason": "transiently_unknown",
                        "error": error
                    }),
                );
                return respond_arkpulse_fix(
                    &state,
                    PulseFixResponseContext { audit, started_at },
                    result,
                )
                .await;
            }
        }
    }

    let execution = async {
        match &plan {
            PulseFixPlan::TunnelStartVerify => {
                let tunnel_arc = state.tunnel.clone();
                if let Err(error) = tunnel::spawn_tunnel(&state, None).await {
                    arkpulse_error_result(StatusCode::INTERNAL_SERVER_ERROR, error)
                } else {
                    let discovered_url = tunnel::wait_for_tunnel_url(tunnel_arc.clone(), 12).await;
                    if let Some(url) = discovered_url.as_ref() {
                        tunnel::persist_public_tunnel_state(&state, Some(url), None).await;
                    }

                    let tunnel = tunnel_arc.read().await;
                    let active = tunnel.active;
                    let url = tunnel.url.clone();
                    let message = if active
                        && url.as_ref().is_some_and(|value| !value.trim().is_empty())
                    {
                        format!(
                            "Remote access is active at {}.",
                            url.clone().unwrap_or_default()
                        )
                    } else {
                        "Tunnel start requested. URL is pending; re-check /tunnel/status shortly."
                            .to_string()
                    };
                    (
                        StatusCode::OK,
                        serde_json::json!({
                            "status": "ok",
                            "mode": "tunnel_start_verify",
                            "message": message,
                            "active": active,
                            "url": url
                        }),
                    )
                }
            }
            PulseFixPlan::TunnelRestartVerify => {
                tunnel::stop_tunnel_internal(&state).await;
                let tunnel_arc = state.tunnel.clone();
                if let Err(error) = tunnel::spawn_tunnel(&state, None).await {
                    arkpulse_error_result(StatusCode::INTERNAL_SERVER_ERROR, error)
                } else {
                    let discovered_url = tunnel::wait_for_tunnel_url(tunnel_arc.clone(), 12).await;
                    if let Some(url) = discovered_url.as_ref() {
                        tunnel::persist_public_tunnel_state(&state, Some(url), None).await;
                    }

                    let tunnel = tunnel_arc.read().await;
                    let active = tunnel.active;
                    let url = tunnel.url.clone();
                    let message = if active
                        && url.as_ref().is_some_and(|value| !value.trim().is_empty())
                    {
                        format!(
                            "Remote access restarted successfully at {}.",
                            url.clone().unwrap_or_default()
                        )
                    } else {
                        "Tunnel restart requested. URL is pending; re-check /tunnel/status shortly."
                            .to_string()
                    };
                    (
                        StatusCode::OK,
                        serde_json::json!({
                            "status": "ok",
                            "mode": "tunnel_restart_verify",
                            "message": message,
                            "active": active,
                            "url": url
                        }),
                    )
                }
            }
            PulseFixPlan::AppRestart(_) => unreachable!("app_restart handled above"),
            PulseFixPlan::ManagedAppOperation { app_id, operation } => {
                run_arkpulse_managed_app_operation_fix(&state, app_id, operation).await
            }
            PulseFixPlan::ReadonlyInvestigation { topic } => {
                run_arkpulse_readonly_investigation_fix(&state, topic).await
            }
        }
    };
    let result = match tokio::time::timeout(Duration::from_secs(60), execution).await {
        Ok(result) => result,
        Err(_) => arkpulse_error_result(
            StatusCode::GATEWAY_TIMEOUT,
            format!(
                "Pulse {} timed out. The control plane aborted the remediation to stay responsive.",
                arkpulse_fix_plan_label(&plan)
            ),
        ),
    };

    respond_arkpulse_fix(
        &state,
        PulseFixResponseContext { audit, started_at },
        result,
    )
    .await
}

/// Return the Pulse event log (last 100 events)
pub(super) async fn get_pulse_log(
    State(state): State<AppState>,
    axum::extract::Query(params): axum::extract::Query<std::collections::HashMap<String, String>>,
) -> Json<serde_json::Value> {
    let limit = params
        .get("limit")
        .and_then(|s| s.parse().ok())
        .unwrap_or(20usize);
    let offset = params
        .get("offset")
        .and_then(|s| s.parse().ok())
        .unwrap_or(0usize);
    let agent = state.agent.read().await;
    let mut all_events = crate::sentinel::get_pulse_log(&agent).await;
    let has_persisted_payload = agent
        .storage
        .get(crate::sentinel::PULSE_LOG_KEY)
        .await
        .ok()
        .flatten()
        .is_some();
    all_events.sort_by(|a, b| {
        let a_ts = chrono::DateTime::parse_from_rfc3339(&a.timestamp)
            .map(|ts| ts.timestamp_millis())
            .unwrap_or(0);
        let b_ts = chrono::DateTime::parse_from_rfc3339(&b.timestamp)
            .map(|ts| ts.timestamp_millis())
            .unwrap_or(0);
        b_ts.cmp(&a_ts)
    });
    let total = all_events.len();
    let events: Vec<_> = all_events.into_iter().skip(offset).take(limit).collect();
    Json(serde_json::json!({
        "events": events,
        "total": total,
        "limit": limit,
        "offset": offset,
        "running": crate::sentinel::is_pulse_running(),
        "history_unavailable": total == 0 && has_persisted_payload,
        "history_unavailable_reason": if total == 0 && has_persisted_payload {
            Some("A persisted Pulse history payload exists but could not be read. New runs will appear normally.")
        } else {
            None::<&str>
        }
    }))
}

pub(super) async fn trigger_arkpulse_after_app_change(state: &AppState, reason: &'static str) {
    if crate::sentinel::is_pulse_running() {
        return;
    }
    {
        let agent_guard = state.agent.read().await;
        let autonomy = load_autonomy_settings(&agent_guard).await;
        if autonomy_background_disabled(&autonomy) {
            return;
        }
    }
    let agent = state.agent.clone();
    crate::spawn_logged!("src/channels/http.rs:13513", async move {
        tracing::info!("Pulse auto-triggered after {}", reason);
        crate::sentinel::run_pulse(&agent).await;
    });
}

/// Trigger an Pulse check immediately
pub(super) async fn trigger_pulse(State(state): State<AppState>) -> Json<serde_json::Value> {
    if crate::sentinel::is_pulse_running() {
        return Json(serde_json::json!({
            "status": "running",
            "message": "Pulse is already running"
        }));
    }
    {
        let agent_guard = state.agent.read().await;
        let autonomy = load_autonomy_settings(&agent_guard).await;
        if autonomy_background_disabled(&autonomy) {
            return Json(serde_json::json!({
                "status": "paused",
                "message": "Autonomy is disabled. Re-enable autonomy to run Pulse."
            }));
        }
    }
    let agent = state.agent.clone();
    crate::spawn_logged!("src/channels/http.rs:13538", async move {
        crate::sentinel::run_pulse(&agent).await;
    });
    Json(serde_json::json!({ "status": "triggered", "message": "Pulse check started" }))
}
