use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicBool, Ordering};
use std::time::{Duration, SystemTime};

use once_cell::sync::Lazy;

const MANAGED_BACKUP_PREFIX: &str = "agentark-managed-";
const MANAGED_BACKUP_SUFFIX: &str = ".dump";
const MANAGED_DATA_ARCHIVE_SUFFIX: &str = ".data.tar.gz";
const MANAGED_CONFIG_ARCHIVE_SUFFIX: &str = ".config.tar.gz";
const DEFAULT_MANAGED_BACKUP_INTERVAL_SECS: u64 = 14 * 24 * 60 * 60;

static MANAGED_BACKUP_RUNNING: AtomicBool = AtomicBool::new(false);

static MANAGED_BACKUP_INTERVAL_SECS: Lazy<u64> = Lazy::new(|| {
    std::env::var("AGENTARK_MANAGED_BACKUP_INTERVAL_SECS")
        .ok()
        .and_then(|value| value.trim().parse::<u64>().ok())
        .filter(|value| *value > 0)
        .unwrap_or(DEFAULT_MANAGED_BACKUP_INTERVAL_SECS)
});

static MANAGED_BACKUP_TIMEOUT_SECS: Lazy<u64> = Lazy::new(|| {
    std::env::var("AGENTARK_MANAGED_BACKUP_TIMEOUT_SECS")
        .ok()
        .and_then(|value| value.trim().parse::<u64>().ok())
        .filter(|value| *value > 0)
        .unwrap_or(90)
});

#[derive(Debug)]
pub(super) struct ManagedBackupError {
    pub target: String,
    pub evidence: String,
}

#[derive(Debug, Clone, Copy)]
pub(super) struct ManagedBackupOptions {
    pub allow_backup_work: bool,
}

#[derive(Debug)]
pub(super) enum ManagedBackupOutcome {
    Fresh,
    Created { path: PathBuf, size_bytes: u64 },
    DeferredBusy,
    AlreadyRunning,
}

struct ManagedBackupRunGuard;

impl Drop for ManagedBackupRunGuard {
    fn drop(&mut self) {
        MANAGED_BACKUP_RUNNING.store(false, Ordering::Release);
    }
}

#[derive(Debug)]
struct ManagedBackupArtifact {
    path: PathBuf,
    modified_at: SystemTime,
    size_bytes: u64,
}

pub(super) async fn ensure_managed_postgres_backup(
    data_dir: &Path,
    options: ManagedBackupOptions,
) -> Result<ManagedBackupOutcome, ManagedBackupError> {
    let backup_dir = data_dir.join("backups");
    tokio::fs::create_dir_all(&backup_dir)
        .await
        .map_err(|error| {
            managed_backup_error(
                &backup_dir,
                format!("Could not create managed backup directory: {}", error),
            )
        })?;

    let latest = latest_managed_backup(&backup_dir)
        .await
        .map_err(|error| managed_backup_error(&backup_dir, error))?;
    if latest
        .as_ref()
        .map(managed_backup_artifact_is_fresh)
        .unwrap_or(false)
    {
        return Ok(ManagedBackupOutcome::Fresh);
    }

    if !options.allow_backup_work {
        tracing::info!(
            target: "agentark::sentinel",
            backup_dir = %backup_dir.display(),
            "Deferring managed Postgres backup because AgentArk is busy"
        );
        return Ok(ManagedBackupOutcome::DeferredBusy);
    }

    let Some(_backup_guard) = try_start_managed_backup() else {
        tracing::info!(
            target: "agentark::sentinel",
            backup_dir = %backup_dir.display(),
            "Deferring managed Postgres backup because another backup is already running"
        );
        return Ok(ManagedBackupOutcome::AlreadyRunning);
    };

    let database_url = std::env::var("AGENTARK_DATABASE_URL").map_err(|_| {
        managed_backup_error(
            &backup_dir,
            "AGENTARK_DATABASE_URL is not available to the backup worker".to_string(),
        )
    })?;

    let timestamp = chrono::Utc::now().format("%Y%m%dT%H%M%SZ");
    let backup_file = format!(
        "{}{}-{}{}",
        MANAGED_BACKUP_PREFIX,
        timestamp,
        uuid::Uuid::new_v4(),
        MANAGED_BACKUP_SUFFIX
    );
    let final_path = backup_dir.join(&backup_file);
    let tmp_path = backup_dir.join(format!("{backup_file}.tmp"));
    let data_archive = backup_file.replace(MANAGED_BACKUP_SUFFIX, MANAGED_DATA_ARCHIVE_SUFFIX);
    let final_data_archive_path = backup_dir.join(&data_archive);
    let tmp_data_archive_path = backup_dir.join(format!("{data_archive}.tmp"));
    let config_archive = backup_file.replace(MANAGED_BACKUP_SUFFIX, MANAGED_CONFIG_ARCHIVE_SUFFIX);
    let final_config_archive_path = backup_dir.join(&config_archive);
    let tmp_config_archive_path = backup_dir.join(format!("{config_archive}.tmp"));

    if let Err(error) = create_postgres_dump(&database_url, &tmp_path).await {
        cleanup_partial_backup_files(&[
            &tmp_path,
            &tmp_data_archive_path,
            &tmp_config_archive_path,
            &final_path,
            &final_data_archive_path,
            &final_config_archive_path,
        ])
        .await;
        let prefix = latest_backup_context(latest.as_ref());
        return Err(managed_backup_error(
            &backup_dir,
            format!("{prefix}Backup command failed: {error}"),
        ));
    }

    if let Err(error) = create_data_archive(data_dir, &tmp_data_archive_path).await {
        cleanup_partial_backup_files(&[
            &tmp_path,
            &tmp_data_archive_path,
            &tmp_config_archive_path,
            &final_path,
            &final_data_archive_path,
            &final_config_archive_path,
        ])
        .await;
        let prefix = latest_backup_context(latest.as_ref());
        return Err(managed_backup_error(
            &backup_dir,
            format!("{prefix}Data archive failed: {error}"),
        ));
    }

    let config_archive_created = match create_config_archive(&tmp_config_archive_path).await {
        Ok(created) => created,
        Err(error) => {
            cleanup_partial_backup_files(&[
                &tmp_path,
                &tmp_data_archive_path,
                &tmp_config_archive_path,
                &final_path,
                &final_data_archive_path,
                &final_config_archive_path,
            ])
            .await;
            let prefix = latest_backup_context(latest.as_ref());
            return Err(managed_backup_error(
                &backup_dir,
                format!("{prefix}Config archive failed: {error}"),
            ));
        }
    };

    let metadata = tokio::fs::metadata(&tmp_path).await.map_err(|error| {
        managed_backup_error(
            &backup_dir,
            format!("Backup command finished but the dump file could not be inspected: {error}"),
        )
    })?;
    if metadata.len() == 0 {
        cleanup_partial_backup_files(&[
            &tmp_path,
            &tmp_data_archive_path,
            &tmp_config_archive_path,
            &final_path,
            &final_data_archive_path,
            &final_config_archive_path,
        ])
        .await;
        return Err(managed_backup_error(
            &backup_dir,
            "Backup command produced an empty dump file".to_string(),
        ));
    }

    if let Err(error) = ensure_nonempty_file(&tmp_data_archive_path, "Data archive").await {
        cleanup_partial_backup_files(&[
            &tmp_path,
            &tmp_data_archive_path,
            &tmp_config_archive_path,
            &final_path,
            &final_data_archive_path,
            &final_config_archive_path,
        ])
        .await;
        return Err(error);
    }
    if config_archive_created {
        if let Err(error) = ensure_nonempty_file(&tmp_config_archive_path, "Config archive").await {
            cleanup_partial_backup_files(&[
                &tmp_path,
                &tmp_data_archive_path,
                &tmp_config_archive_path,
                &final_path,
                &final_data_archive_path,
                &final_config_archive_path,
            ])
            .await;
            return Err(error);
        }
    }

    if let Err(error) = tokio::fs::rename(&tmp_path, &final_path).await {
        cleanup_partial_backup_files(&[
            &tmp_path,
            &tmp_data_archive_path,
            &tmp_config_archive_path,
            &final_path,
            &final_data_archive_path,
            &final_config_archive_path,
        ])
        .await;
        return Err(managed_backup_error(
            &backup_dir,
            format!("Backup dump was created but could not be committed atomically: {error}"),
        ));
    }
    if let Err(error) = tokio::fs::rename(&tmp_data_archive_path, &final_data_archive_path).await {
        cleanup_partial_backup_files(&[
            &tmp_path,
            &tmp_data_archive_path,
            &tmp_config_archive_path,
            &final_path,
            &final_data_archive_path,
            &final_config_archive_path,
        ])
        .await;
        return Err(managed_backup_error(
            &backup_dir,
            format!("Data archive was created but could not be committed atomically: {error}"),
        ));
    }
    if config_archive_created {
        if let Err(error) =
            tokio::fs::rename(&tmp_config_archive_path, &final_config_archive_path).await
        {
            cleanup_partial_backup_files(&[
                &tmp_path,
                &tmp_data_archive_path,
                &tmp_config_archive_path,
                &final_path,
                &final_data_archive_path,
                &final_config_archive_path,
            ])
            .await;
            return Err(managed_backup_error(
                &backup_dir,
                format!("Config archive was created but could not be committed atomically: {error}"),
            ));
        }
    }

    tracing::info!(
        target: "agentark::sentinel",
        path = %final_path.display(),
        size_bytes = metadata.len(),
        config_archive_created,
        "Created managed AgentArk backup"
    );
    Ok(ManagedBackupOutcome::Created {
        path: final_path,
        size_bytes: metadata.len(),
    })
}

fn try_start_managed_backup() -> Option<ManagedBackupRunGuard> {
    MANAGED_BACKUP_RUNNING
        .compare_exchange(false, true, Ordering::AcqRel, Ordering::Acquire)
        .ok()
        .map(|_| ManagedBackupRunGuard)
}

async fn latest_managed_backup(backup_dir: &Path) -> Result<Option<ManagedBackupArtifact>, String> {
    let mut entries = tokio::fs::read_dir(backup_dir)
        .await
        .map_err(|error| format!("Could not read managed backup directory: {error}"))?;
    let mut latest: Option<ManagedBackupArtifact> = None;
    loop {
        let entry = match entries.next_entry().await {
            Ok(Some(entry)) => entry,
            Ok(None) => break,
            Err(error) => {
                return Err(format!("Could not enumerate managed backup directory: {error}"));
            }
        };
        let file_name = entry.file_name();
        let file_name = file_name.to_string_lossy();
        if !file_name.starts_with(MANAGED_BACKUP_PREFIX)
            || !file_name.ends_with(MANAGED_BACKUP_SUFFIX)
        {
            continue;
        }
        let Ok(metadata) = entry.metadata().await else {
            continue;
        };
        if !metadata.is_file() || metadata.len() == 0 {
            continue;
        }
        let data_archive_name =
            file_name.replace(MANAGED_BACKUP_SUFFIX, MANAGED_DATA_ARCHIVE_SUFFIX);
        let data_archive_path = entry.path().with_file_name(data_archive_name);
        let Ok(data_archive_metadata) = tokio::fs::metadata(&data_archive_path).await else {
            continue;
        };
        if !data_archive_metadata.is_file() || data_archive_metadata.len() == 0 {
            continue;
        }
        let Ok(modified_at) = metadata.modified() else {
            continue;
        };
        let candidate = ManagedBackupArtifact {
            path: entry.path(),
            modified_at,
            size_bytes: metadata.len(),
        };
        if latest
            .as_ref()
            .map(|current| candidate.modified_at > current.modified_at)
            .unwrap_or(true)
        {
            latest = Some(candidate);
        }
    }
    Ok(latest)
}

fn managed_backup_artifact_is_fresh(artifact: &ManagedBackupArtifact) -> bool {
    artifact
        .modified_at
        .elapsed()
        .map(|age| age <= Duration::from_secs(*MANAGED_BACKUP_INTERVAL_SECS))
        .unwrap_or(true)
}

fn latest_backup_context(latest: Option<&ManagedBackupArtifact>) -> String {
    let Some(latest) = latest else {
        return "No previous managed backup is available. ".to_string();
    };
    let age = latest
        .modified_at
        .elapsed()
        .map(|age| format!("{:.1}h", age.as_secs_f64() / 3600.0))
        .unwrap_or_else(|_| "unknown age".to_string());
    format!(
        "Previous managed backup: {} ({} bytes, {}). ",
        latest.path.display(),
        latest.size_bytes,
        age
    )
}

async fn create_postgres_dump(database_url: &str, output_path: &Path) -> Result<(), String> {
    let mut command = tokio::process::Command::new("pg_dump");
    configure_pg_dump_env(&mut command, database_url)?;
    command
        .arg("--format=custom")
        .arg("--no-owner")
        .arg("--no-acl")
        .arg("--file")
        .arg(output_path)
        .env("PGCONNECT_TIMEOUT", "10");
    run_backup_command(command, "pg_dump").await
}

async fn create_data_archive(data_dir: &Path, output_path: &Path) -> Result<(), String> {
    let metadata = tokio::fs::metadata(data_dir)
        .await
        .map_err(|error| format!("Data directory could not be inspected: {error}"))?;
    if !metadata.is_dir() {
        return Err(format!("Data path is not a directory: {}", data_dir.display()));
    }
    let mut command = tokio::process::Command::new("tar");
    command
        .arg("-czf")
        .arg(output_path)
        .arg("--exclude=./backups")
        .arg("--exclude=backups")
        .arg("-C")
        .arg(data_dir)
        .arg(".");
    run_backup_command(command, "data archive").await
}

async fn create_config_archive(output_path: &Path) -> Result<bool, String> {
    let Some(config_dir) = std::env::var("AGENTARK_CONFIG")
        .ok()
        .map(PathBuf::from)
        .filter(|path| !path.as_os_str().is_empty())
    else {
        return Ok(false);
    };
    let Ok(metadata) = tokio::fs::metadata(&config_dir).await else {
        return Ok(false);
    };
    if !metadata.is_dir() {
        return Ok(false);
    }
    let mut command = tokio::process::Command::new("tar");
    command
        .arg("-czf")
        .arg(output_path)
        .arg("-C")
        .arg(&config_dir)
        .arg(".");
    run_backup_command(command, "config archive").await?;
    Ok(true)
}

async fn run_backup_command(
    mut command: tokio::process::Command,
    label: &str,
) -> Result<(), String> {
    command
        .stdout(std::process::Stdio::piped())
        .stderr(std::process::Stdio::piped())
        .kill_on_drop(true);
    let timeout = Duration::from_secs(*MANAGED_BACKUP_TIMEOUT_SECS);
    let output = tokio::time::timeout(timeout, command.output())
        .await
        .map_err(|_| format!("{label} timed out after {} seconds", timeout.as_secs()))?
        .map_err(|error| format!("{label} could not be started: {error}"))?;
    if output.status.success() {
        return Ok(());
    }
    let status = output
        .status
        .code()
        .map(|code| code.to_string())
        .unwrap_or_else(|| "terminated by signal".to_string());
    Err(format!(
        "{label} exited with status {}; {}",
        status,
        compact_process_output(&output.stderr, &output.stdout)
    ))
}

async fn ensure_nonempty_file(path: &Path, label: &str) -> Result<(), ManagedBackupError> {
    let metadata = tokio::fs::metadata(path).await.map_err(|error| {
        managed_backup_error(
            path,
            format!("{label} could not be inspected after creation: {error}"),
        )
    })?;
    if metadata.is_file() && metadata.len() > 0 {
        Ok(())
    } else {
        Err(managed_backup_error(
            path,
            format!("{label} was empty after backup creation"),
        ))
    }
}

async fn cleanup_partial_backup_files(paths: &[&Path]) {
    for path in paths {
        let _ = tokio::fs::remove_file(path).await;
    }
}

fn configure_pg_dump_env(
    command: &mut tokio::process::Command,
    database_url: &str,
) -> Result<(), String> {
    let parsed = url::Url::parse(database_url)
        .map_err(|error| format!("Configured Postgres URL could not be parsed: {error}"))?;
    let host = parsed
        .host_str()
        .ok_or_else(|| "Configured Postgres URL has no host".to_string())?;
    let user = parsed.username();
    if user.trim().is_empty() {
        return Err("Configured Postgres URL has no user".to_string());
    }
    let database = parsed
        .path_segments()
        .and_then(|mut segments| segments.find(|segment| !segment.is_empty()))
        .ok_or_else(|| "Configured Postgres URL has no database name".to_string())?;

    command
        .env("PGHOST", host)
        .env(
            "PGPORT",
            parsed.port_or_known_default().unwrap_or(5432).to_string(),
        )
        .env("PGUSER", decode_url_component(user))
        .env("PGDATABASE", decode_url_component(database));
    if let Some(password) = parsed.password() {
        command.env("PGPASSWORD", decode_url_component(password));
    }
    for (key, value) in parsed.query_pairs() {
        if key == "sslmode" {
            command.env("PGSSLMODE", value.as_ref());
        }
    }
    Ok(())
}

fn decode_url_component(value: &str) -> String {
    urlencoding::decode(value)
        .map(|decoded| decoded.into_owned())
        .unwrap_or_else(|_| value.to_string())
}

fn compact_process_output(stderr: &[u8], stdout: &[u8]) -> String {
    let mut lines = Vec::new();
    for raw in [stderr, stdout] {
        let text = String::from_utf8_lossy(raw);
        lines.extend(
            text.lines()
                .map(str::trim)
                .filter(|line| !line.is_empty())
                .take(4)
                .map(ToString::to_string),
        );
        if lines.len() >= 4 {
            break;
        }
    }
    if lines.is_empty() {
        "no process output".to_string()
    } else {
        lines.truncate(4);
        lines.join("; ")
    }
}

fn managed_backup_error(target: &Path, evidence: String) -> ManagedBackupError {
    ManagedBackupError {
        target: target.display().to_string(),
        evidence,
    }
}
