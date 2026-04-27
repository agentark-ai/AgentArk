use super::*;

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

#[derive(Debug, Clone)]
pub(super) enum ArkPulseFixPlan {
    TunnelStartVerify,
    TunnelRestartVerify,
    AppRestart(String),
    ReadonlyInvestigation {
        topic: crate::sentinel::DoctorReadonlyInvestigationTopic,
    },
    ShellOperations {
        app_dir: String,
        operations: Vec<ArkPulseShellOperation>,
    },
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub(super) enum ArkPulseShellOperation {
    PipCompileRequirements,
    Ripgrep {
        pattern: String,
        path: Option<String>,
    },
    CargoGenerateLockfile,
    NpmPkgDelete {
        keys: Vec<String>,
    },
    MoveEnvBackup,
}

pub(super) fn arkpulse_fix_plan_from_remediation(
    remediation: &crate::sentinel::DoctorRemediationSpec,
    allow_shell_command: bool,
) -> Option<ArkPulseFixPlan> {
    match remediation {
        crate::sentinel::DoctorRemediationSpec::TunnelStartVerify => {
            Some(ArkPulseFixPlan::TunnelStartVerify)
        }
        crate::sentinel::DoctorRemediationSpec::TunnelRestartVerify => {
            Some(ArkPulseFixPlan::TunnelRestartVerify)
        }
        crate::sentinel::DoctorRemediationSpec::AppRestart { app_id } => {
            if is_valid_app_id(app_id) {
                Some(ArkPulseFixPlan::AppRestart(app_id.clone()))
            } else {
                None
            }
        }
        crate::sentinel::DoctorRemediationSpec::ReadonlyInvestigation { topic } => {
            Some(ArkPulseFixPlan::ReadonlyInvestigation {
                topic: topic.clone(),
            })
        }
        crate::sentinel::DoctorRemediationSpec::ShellCommand { command } if allow_shell_command => {
            parse_supported_arkpulse_shell_command(command.trim())
        }
        crate::sentinel::DoctorRemediationSpec::ShellCommand { .. } => None,
    }
}

pub(super) fn parse_arkpulse_app_restart(command: &str) -> Option<String> {
    let normalized = command.trim();
    let path = normalized
        .strip_prefix("POST ")
        .or_else(|| normalized.strip_prefix("post "))?
        .trim();
    let app_id = path.strip_prefix("/api/apps/")?.strip_suffix("/restart")?;
    if is_valid_app_id(app_id) {
        Some(app_id.to_string())
    } else {
        None
    }
}

pub(super) fn parse_supported_arkpulse_shell_segment(
    segment: &str,
) -> Option<ArkPulseShellOperation> {
    let trimmed = segment.trim();
    let lower = trimmed.to_ascii_lowercase();
    if lower == "pip-compile requirements.txt" {
        return Some(ArkPulseShellOperation::PipCompileRequirements);
    }
    if lower == "cargo generate-lockfile" {
        return Some(ArkPulseShellOperation::CargoGenerateLockfile);
    }
    if lower == "npm pkg delete scripts.preinstall scripts.install scripts.postinstall" {
        return Some(ArkPulseShellOperation::NpmPkgDelete {
            keys: vec![
                "scripts.preinstall".to_string(),
                "scripts.install".to_string(),
                "scripts.postinstall".to_string(),
            ],
        });
    }
    if lower == "mv .env ../.env.backup" || lower == "mv .env .env.backup" {
        return Some(ArkPulseShellOperation::MoveEnvBackup);
    }
    if let Some(rest) = trimmed.strip_prefix("rg -n ") {
        let quoted = rest.strip_prefix('"')?;
        let end = quoted.find('"')?;
        let pattern = quoted[..end].to_string();
        let remaining = quoted[end + 1..].trim();
        if remaining.contains('&')
            || remaining.contains(';')
            || remaining.contains('`')
            || remaining.contains("$(")
            || remaining.contains('|')
        {
            return None;
        }
        let path = (!remaining.is_empty()).then(|| remaining.to_string());
        return Some(ArkPulseShellOperation::Ripgrep { pattern, path });
    }
    None
}

pub(super) fn parse_supported_arkpulse_shell_command(command: &str) -> Option<ArkPulseFixPlan> {
    let normalized = command.trim();
    if normalized.is_empty() {
        return None;
    }
    let lower = normalized.to_ascii_lowercase();
    if lower.contains('\n')
        || lower.contains('\r')
        || lower.contains("||")
        || lower.contains(';')
        || lower.contains('`')
        || lower.contains("$(")
    {
        return None;
    }

    let segments: Vec<&str> = normalized
        .split("&&")
        .map(str::trim)
        .filter(|segment| !segment.is_empty())
        .collect();
    if segments.len() < 2 {
        return None;
    }

    let cd_segment = segments[0];
    let cd_prefix = cd_segment
        .strip_prefix("cd ")
        .or_else(|| cd_segment.strip_prefix("cd\t"))?;
    let cd_target = cd_prefix.trim();
    if cd_target.is_empty() {
        return None;
    }

    let mut operations = Vec::new();
    for segment in segments.iter().skip(1) {
        operations.push(parse_supported_arkpulse_shell_segment(segment)?);
    }

    Some(ArkPulseFixPlan::ShellOperations {
        app_dir: cd_target.to_string(),
        operations,
    })
}

pub(super) fn classify_arkpulse_fix_plan(command: &str) -> Option<ArkPulseFixPlan> {
    let normalized = command.trim();
    if normalized.is_empty() {
        return None;
    }
    let lower = normalized.to_ascii_lowercase();
    if lower.contains("start tunnel") && lower.contains("/tunnel/status") {
        return Some(ArkPulseFixPlan::TunnelStartVerify);
    }
    if lower.contains("restart") && lower.contains("tunnel") {
        return Some(ArkPulseFixPlan::TunnelRestartVerify);
    }
    if let Some(app_id) = parse_arkpulse_app_restart(normalized) {
        return Some(ArkPulseFixPlan::AppRestart(app_id));
    }
    parse_supported_arkpulse_shell_command(normalized)
}

pub(super) fn truncate_for_response(input: &str, max_chars: usize) -> String {
    if input.chars().count() <= max_chars {
        return input.to_string();
    }
    input.chars().take(max_chars).collect::<String>() + "..."
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

pub(super) fn describe_arkpulse_shell_operation(operation: &ArkPulseShellOperation) -> String {
    match operation {
        ArkPulseShellOperation::PipCompileRequirements => {
            "pip-compile requirements.txt".to_string()
        }
        ArkPulseShellOperation::Ripgrep { pattern, path } => match path {
            Some(path) => format!("rg -n \"{}\" {}", pattern, path),
            None => format!("rg -n \"{}\"", pattern),
        },
        ArkPulseShellOperation::CargoGenerateLockfile => "cargo generate-lockfile".to_string(),
        ArkPulseShellOperation::NpmPkgDelete { keys } => {
            format!("npm pkg delete {}", keys.join(" "))
        }
        ArkPulseShellOperation::MoveEnvBackup => "mv .env .env.backup".to_string(),
    }
}

pub(super) fn arkpulse_shell_operation_auto_run_error(
    operations: &[ArkPulseShellOperation],
) -> Option<String> {
    for operation in operations {
        if matches!(operation, ArkPulseShellOperation::Ripgrep { .. }) {
            return Some(
                "ArkPulse grep fixes are no longer auto-run because they can expose file contents. Review the finding and run the search manually if needed."
                    .to_string(),
            );
        }
    }
    None
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

type ArkPulseFixHttpResult = (StatusCode, serde_json::Value);

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
}

pub(super) fn arkpulse_error_result(
    status: StatusCode,
    error: impl Into<String>,
) -> ArkPulseFixHttpResult {
    let error = error.into();
    (
        status,
        serde_json::json!({
            "status": "error",
            "error": error,
        }),
    )
}

pub(super) fn validate_arkpulse_relative_path(raw: &str) -> Result<PathBuf, String> {
    let trimmed = raw.trim();
    if trimmed.is_empty() {
        return Err("ArkPulse file target cannot be empty".to_string());
    }
    let path = PathBuf::from(trimmed);
    if path.is_absolute() {
        return Err("ArkPulse file targets must stay relative to the app directory".to_string());
    }
    for component in path.components() {
        if matches!(
            component,
            std::path::Component::ParentDir
                | std::path::Component::RootDir
                | std::path::Component::Prefix(_)
        ) {
            return Err("ArkPulse file targets cannot escape the app directory".to_string());
        }
    }
    Ok(path)
}

pub(super) async fn resolve_arkpulse_app_dir(
    state: &AppState,
    raw_app_dir: &str,
) -> Result<PathBuf, String> {
    let trimmed = raw_app_dir.trim();
    if trimmed.is_empty() {
        return Err("App directory is required for ArkPulse shell operations".to_string());
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
        return Err("ArkPulse fixes may only run inside the managed apps directory".to_string());
    }
    Ok(requested)
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

pub(super) async fn run_arkpulse_shell_operations_fix(
    state: &AppState,
    raw_app_dir: &str,
    operations: &[ArkPulseShellOperation],
) -> ArkPulseFixHttpResult {
    let app_dir = match resolve_arkpulse_app_dir(state, raw_app_dir).await {
        Ok(path) => path,
        Err(error) => return arkpulse_error_result(StatusCode::BAD_REQUEST, error),
    };

    let mut results = Vec::new();
    for operation in operations {
        let detail = match operation {
            ArkPulseShellOperation::PipCompileRequirements => {
                let args = vec!["requirements.txt".to_string()];
                run_arkpulse_process(&app_dir, "pip-compile", &args).await
            }
            ArkPulseShellOperation::Ripgrep { pattern, path } => {
                let mut args = vec!["-n".to_string(), pattern.clone()];
                if let Some(path) = path {
                    let safe_path = match validate_arkpulse_relative_path(path) {
                        Ok(value) => value,
                        Err(error) => return arkpulse_error_result(StatusCode::BAD_REQUEST, error),
                    };
                    args.push(safe_path.to_string_lossy().to_string());
                }
                run_arkpulse_process(&app_dir, "rg", &args).await
            }
            ArkPulseShellOperation::CargoGenerateLockfile => {
                let args = vec!["generate-lockfile".to_string()];
                run_arkpulse_process(&app_dir, "cargo", &args).await
            }
            ArkPulseShellOperation::NpmPkgDelete { keys } => {
                let mut args = vec!["pkg".to_string(), "delete".to_string()];
                args.extend(keys.clone());
                run_arkpulse_process(&app_dir, "npm", &args).await
            }
            ArkPulseShellOperation::MoveEnvBackup => {
                let source = app_dir.join(".env");
                if !source.exists() {
                    return arkpulse_error_result(
                        StatusCode::BAD_REQUEST,
                        "No .env file exists in this app directory",
                    );
                };
                let target = app_dir.join(".env.backup");
                match tokio::fs::rename(&source, &target).await {
                    Ok(()) => Ok(serde_json::json!({
                        "action": "rename",
                        "from": source.display().to_string(),
                        "to": target.display().to_string(),
                    })),
                    Err(error) => Err(format!("failed to move .env to backup: {}", error)),
                }
            }
        };

        match detail {
            Ok(detail) => results.push(serde_json::json!({
                "operation": describe_arkpulse_shell_operation(operation),
                "detail": detail,
            })),
            Err(error) => return arkpulse_error_result(StatusCode::INTERNAL_SERVER_ERROR, error),
        }
    }

    (
        StatusCode::OK,
        serde_json::json!({
            "status": "ok",
            "mode": "shell_operations",
            "app_dir": app_dir.display().to_string(),
            "operations": results,
        }),
    )
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
                    "Failed to prune ArkPulse history while cleaning '{}': {}",
                    app_id,
                    error
                );
                0
            }
        };
    DeletedAppCleanupSummary {
        deleted_notifications,
        deleted_pulse_events,
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
                    "App restart for {} timed out. ArkPulse aborted the remediation to keep the control plane responsive.",
                    app_id
                ),
            ),
        };
        let status = result.0;
        let body = result.1;
        let latency_ms = started_at.elapsed().as_millis().min(i64::MAX as u128) as i64;
        let plan = ArkPulseFixPlan::AppRestart(app_id.clone());
        let audit = ArkPulseFixAuditDetails {
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
) -> ArkPulseFixHttpResult {
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
            "message": format!("Restarted app {} and queued a fresh ArkPulse run.", title),
            "url": url,
            "details": payload,
        }),
    )
}

pub(super) async fn run_arkpulse_readonly_investigation_fix(
    state: &AppState,
    topic: &crate::sentinel::DoctorReadonlyInvestigationTopic,
) -> ArkPulseFixHttpResult {
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
                    "project_id".to_string(),
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
                    let project_id = row
                        .get("project_id")
                        .and_then(|value| value.as_str())
                        .map(|value| format!("project={}", value))
                        .unwrap_or_else(|| "project=global".to_string());
                    format!(
                        "- {} | {} | {} | {}",
                        updated_at, capture_kind, conversation_id, project_id
                    )
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
                    "message": "ArkPulse diagnostic completed.",
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

pub(super) fn arkpulse_fix_plan_label(plan: &ArkPulseFixPlan) -> &'static str {
    match plan {
        ArkPulseFixPlan::TunnelStartVerify => "tunnel_start_verify",
        ArkPulseFixPlan::TunnelRestartVerify => "tunnel_restart_verify",
        ArkPulseFixPlan::AppRestart(_) => "app_restart",
        ArkPulseFixPlan::ReadonlyInvestigation { .. } => "readonly_investigation",
        ArkPulseFixPlan::ShellOperations { .. } => "shell_operations",
    }
}

pub(super) struct ArkPulseFixAuditDetails<'a> {
    plan: &'a ArkPulseFixPlan,
    issue_title: &'a str,
    target: &'a str,
    fix_summary: &'a str,
    event_timestamp: Option<&'a str>,
    finding_index: Option<usize>,
}

pub(super) async fn persist_arkpulse_fix_audit(
    state: &AppState,
    audit: &ArkPulseFixAuditDetails<'_>,
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
            format!("ArkPulse fix completed ({mode})")
        } else {
            format!("ArkPulse fix failed ({mode})")
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
        tracing::warn!("Failed to persist ArkPulse fix audit log: {}", error);
    }
}

pub(super) struct ArkPulseFixResponseContext<'a> {
    audit: ArkPulseFixAuditDetails<'a>,
    started_at: std::time::Instant,
}

pub(super) async fn respond_arkpulse_fix(
    state: &AppState,
    context: ArkPulseFixResponseContext<'_>,
    result: ArkPulseFixHttpResult,
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

/// Execute a supported ArkPulse remediation directly (without going through Chat).
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
    if request_fix_command.is_empty() && remediation.is_none() {
        return (
            StatusCode::BAD_REQUEST,
            Json(ErrorResponse {
                error: "remediation or fix_command is required".to_string(),
            }),
        )
            .into_response();
    }

    let mut effective_fix_command = request_fix_command.clone();
    let mut effective_remediation = remediation.clone();
    let request_has_event_context = event_timestamp
        .as_deref()
        .is_some_and(|value| !value.trim().is_empty())
        || finding_index.is_some();
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
                    error: "ArkPulse event not found".to_string(),
                }),
            )
                .into_response();
        };
        let Some(finding) = event.details.doctor_findings.get(finding_index) else {
            return (
                StatusCode::NOT_FOUND,
                Json(ErrorResponse {
                    error: "ArkPulse finding index is out of range".to_string(),
                }),
            )
                .into_response();
        };
        if !finding.user_actionable {
            return (
                StatusCode::BAD_REQUEST,
                Json(ErrorResponse {
                    error: "This ArkPulse finding is advisory-only and must be fixed manually"
                        .to_string(),
                }),
            )
                .into_response();
        }
        if !request_fix_command.is_empty() && finding.fix_command.trim() != request_fix_command {
            return (
                StatusCode::BAD_REQUEST,
                Json(ErrorResponse {
                    error: "fix_command does not match the selected ArkPulse finding".to_string(),
                }),
            )
                .into_response();
        }
        if let (Some(requested_remediation), Some(stored_remediation)) =
            (remediation.as_ref(), finding.remediation.as_ref())
        {
            if stored_remediation != requested_remediation {
                return (
                    StatusCode::BAD_REQUEST,
                    Json(ErrorResponse {
                        error: "remediation does not match the selected ArkPulse finding"
                            .to_string(),
                    }),
                )
                    .into_response();
            }
        }
        selected_event_timestamp = Some(event_timestamp.to_string());
        selected_finding_index = Some(finding_index);
        effective_fix_command = finding.fix_command.trim().to_string();
        effective_remediation = finding.remediation.clone();
        effective_remediation
            .as_ref()
            .and_then(|value| arkpulse_fix_plan_from_remediation(value, true))
            .or_else(|| classify_arkpulse_fix_plan(&effective_fix_command))
    } else {
        effective_remediation
            .as_ref()
            .and_then(|value| arkpulse_fix_plan_from_remediation(value, false))
            .or_else(|| classify_arkpulse_fix_plan(&request_fix_command))
    };

    let Some(plan) = plan else {
        return (
            StatusCode::BAD_REQUEST,
            Json(ErrorResponse {
                error:
                    "This fix cannot be auto-run directly. Copy the remediation and run it manually."
                        .to_string(),
            }),
        )
            .into_response();
    };

    if let ArkPulseFixPlan::ShellOperations { operations, .. } = &plan {
        if !request_has_event_context {
            return (
                StatusCode::BAD_REQUEST,
                Json(ErrorResponse {
                    error: "Shell-style ArkPulse fixes must come from a stored ArkPulse finding. Open the finding in ArkPulse and run it from there."
                        .to_string(),
                }),
            )
                .into_response();
        }
        if let Some(error) = arkpulse_shell_operation_auto_run_error(operations) {
            return (StatusCode::BAD_REQUEST, Json(ErrorResponse { error })).into_response();
        }
    }

    let issue_title = issue_title.unwrap_or_default();
    let target = target.unwrap_or_default();
    let fix_summary =
        describe_arkpulse_remediation(effective_remediation.as_ref(), &effective_fix_command);
    tracing::info!(
        "ArkPulse fix requested: issue='{}' target='{}' command='{}'",
        issue_title,
        target,
        truncate_for_response(&fix_summary, 220)
    );

    let audit = ArkPulseFixAuditDetails {
        plan: &plan,
        issue_title: &issue_title,
        target: &target,
        fix_summary: &fix_summary,
        event_timestamp: selected_event_timestamp.as_deref(),
        finding_index: selected_finding_index,
    };

    if let ArkPulseFixPlan::AppRestart(app_id) = &plan {
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
                        "message": "Queued app restart. ArkPulse will apply it in the background while the control plane stays responsive."
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
                    ArkPulseFixResponseContext { audit, started_at },
                    result,
                )
                .await;
            }
            HealViability::NotRecoverable(reason) => {
                let app_title = lookup_app_title_for_cleanup(&state, app_id).await;
                let cleanup =
                    cleanup_deleted_app_references(&state, app_id, app_title.as_deref()).await;
                trigger_arkpulse_after_app_change(&state, "app_delete").await;
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
                    ArkPulseFixResponseContext { audit, started_at },
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
                    ArkPulseFixResponseContext { audit, started_at },
                    result,
                )
                .await;
            }
        }
    }

    let execution = async {
        match &plan {
            ArkPulseFixPlan::TunnelStartVerify => {
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
            ArkPulseFixPlan::TunnelRestartVerify => {
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
            ArkPulseFixPlan::AppRestart(_) => unreachable!("app_restart handled above"),
            ArkPulseFixPlan::ReadonlyInvestigation { topic } => {
                run_arkpulse_readonly_investigation_fix(&state, topic).await
            }
            ArkPulseFixPlan::ShellOperations {
                app_dir,
                operations,
            } => run_arkpulse_shell_operations_fix(&state, app_dir, operations).await,
        }
    };
    let result = match tokio::time::timeout(Duration::from_secs(60), execution).await {
        Ok(result) => result,
        Err(_) => arkpulse_error_result(
            StatusCode::GATEWAY_TIMEOUT,
            format!(
                "ArkPulse {} timed out. The control plane aborted the remediation to stay responsive.",
                arkpulse_fix_plan_label(&plan)
            ),
        ),
    };

    respond_arkpulse_fix(
        &state,
        ArkPulseFixResponseContext { audit, started_at },
        result,
    )
    .await
}

/// Return the ArkPulse event log (last 100 events)
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
            Some("A persisted ArkPulse history payload exists but could not be read. New runs will appear normally.")
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
        tracing::info!("ArkPulse auto-triggered after {}", reason);
        crate::sentinel::run_pulse(&agent).await;
    });
}

/// Trigger an ArkPulse check immediately
pub(super) async fn trigger_pulse(State(state): State<AppState>) -> Json<serde_json::Value> {
    if crate::sentinel::is_pulse_running() {
        return Json(serde_json::json!({
            "status": "running",
            "message": "ArkPulse is already running"
        }));
    }
    {
        let agent_guard = state.agent.read().await;
        let autonomy = load_autonomy_settings(&agent_guard).await;
        if autonomy_background_disabled(&autonomy) {
            return Json(serde_json::json!({
                "status": "paused",
                "message": "Autonomy is disabled. Re-enable autonomy to run ArkPulse."
            }));
        }
    }
    let agent = state.agent.clone();
    crate::spawn_logged!("src/channels/http.rs:13538", async move {
        crate::sentinel::run_pulse(&agent).await;
    });
    Json(serde_json::json!({ "status": "triggered", "message": "ArkPulse check started" }))
}
