use anyhow::{anyhow, Context, Result};
use chrono::Utc;
use serde::{Deserialize, Serialize};
use std::fs;
use std::path::{Path, PathBuf};
use std::process::Stdio;
use std::thread;
use std::time::Duration;
use tokio::process::Command;
use uuid::Uuid;

const SELF_UPDATE_DIR: &str = "self_update";
const SELF_UPDATE_JOBS_FILE: &str = "jobs.json";
const SELF_UPDATE_LOCK_FILE: &str = "jobs.lock";
const SELF_UPDATE_SNAPSHOTS_DIR: &str = "snapshots";
const SELF_UPDATE_RESTORE_DIR: &str = "restore";
const MAX_SELF_UPDATE_JOBS: usize = 50;
const COMMAND_OUTPUT_LIMIT: usize = 4000;
const DEFAULT_HEALTH_TIMEOUT_SECS: u64 = 180;
const DEFAULT_UPDATER_POLL_SECS: u64 = 3;

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct SelfUpdateStep {
    pub key: String,
    pub label: String,
    pub status: String,
    pub detail: String,
    pub started_at: Option<String>,
    pub completed_at: Option<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct SelfUpdateJob {
    pub id: String,
    pub kind: String,
    pub summary: String,
    pub state: String,
    pub requested_at: String,
    pub updated_at: String,
    pub requested_by: String,
    pub approved_at: Option<String>,
    pub approved_by: Option<String>,
    pub started_at: Option<String>,
    pub finished_at: Option<String>,
    pub conversation_id: Option<String>,
    pub project_id: Option<String>,
    pub rollback_source_job_id: Option<String>,
    pub workspace_path: String,
    pub compose_file: String,
    pub service_name: String,
    pub image_name: String,
    pub health_url: String,
    pub check_cargo: bool,
    pub check_frontend: bool,
    pub build_image: bool,
    pub restart_service: bool,
    pub active_step: Option<String>,
    pub status_message: String,
    pub last_error: Option<String>,
    pub snapshot_path: Option<String>,
    pub backup_image_tag: Option<String>,
    #[serde(default)]
    pub steps: Vec<SelfUpdateStep>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct CreateSelfUpdateJob {
    pub summary: String,
    pub requested_by: String,
    pub conversation_id: Option<String>,
    pub project_id: Option<String>,
    pub check_cargo: bool,
    pub check_frontend: bool,
    pub build_image: bool,
    pub restart_service: bool,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct CreateRollbackJob {
    pub requested_by: String,
    pub source_job_id: Option<String>,
    pub conversation_id: Option<String>,
    pub project_id: Option<String>,
}

#[derive(Debug, Clone, Serialize)]
pub struct SelfUpdateStatusSnapshot {
    pub enabled: bool,
    pub workspace_path: String,
    pub workspace_exists: bool,
    pub compose_file: String,
    pub compose_exists: bool,
    pub service_name: String,
    pub image_name: String,
    pub health_url: String,
    pub active_job: Option<SelfUpdateJob>,
    pub latest_job: Option<SelfUpdateJob>,
    pub jobs: Vec<SelfUpdateJob>,
}

pub fn self_update_enabled() -> bool {
    parse_bool_env("AGENTARK_SELF_UPDATE_ENABLED").unwrap_or(true)
}

pub fn workspace_path() -> String {
    std::env::var("AGENTARK_SELF_UPDATE_HOST_WORKTREE")
        .ok()
        .filter(|v| !v.trim().is_empty())
        .or_else(|| {
            std::env::var("AGENTARK_SELF_UPDATE_WORKTREE")
                .ok()
                .filter(|v| !v.trim().is_empty())
        })
        .unwrap_or_else(|| "/workspace/agentark".to_string())
}

pub fn display_workspace_path() -> String {
    std::env::var("AGENTARK_SELF_UPDATE_WORKTREE")
        .ok()
        .filter(|v| !v.trim().is_empty())
        .unwrap_or_else(workspace_path)
}

pub fn compose_file_path() -> String {
    std::env::var("AGENTARK_SELF_UPDATE_HOST_COMPOSE_FILE")
        .ok()
        .filter(|v| !v.trim().is_empty())
        .or_else(|| {
            std::env::var("AGENTARK_SELF_UPDATE_COMPOSE_FILE")
                .ok()
                .filter(|v| !v.trim().is_empty())
        })
        .unwrap_or_else(|| format!("{}/docker-compose.yml", workspace_path()))
}

pub fn display_compose_file_path() -> String {
    std::env::var("AGENTARK_SELF_UPDATE_COMPOSE_FILE")
        .ok()
        .filter(|v| !v.trim().is_empty())
        .unwrap_or_else(|| format!("{}/docker-compose.yml", display_workspace_path()))
}

pub fn service_name() -> String {
    std::env::var("AGENTARK_SELF_UPDATE_SERVICE")
        .ok()
        .filter(|v| !v.trim().is_empty())
        .unwrap_or_else(|| "agentark".to_string())
}

pub fn image_name() -> String {
    std::env::var("AGENTARK_SELF_UPDATE_IMAGE")
        .ok()
        .filter(|v| !v.trim().is_empty())
        .unwrap_or_else(|| "agentark:latest".to_string())
}

pub fn health_url() -> String {
    std::env::var("AGENTARK_SELF_UPDATE_HEALTH_URL")
        .ok()
        .filter(|v| !v.trim().is_empty())
        .unwrap_or_else(|| "http://agentark:8990/health".to_string())
}

pub fn updater_poll_secs() -> u64 {
    std::env::var("AGENTARK_SELF_UPDATE_UPDATER_POLL_SECS")
        .ok()
        .and_then(|v| v.trim().parse::<u64>().ok())
        .filter(|v| *v > 0)
        .unwrap_or(DEFAULT_UPDATER_POLL_SECS)
}

pub fn create_job(data_dir: &Path, request: CreateSelfUpdateJob) -> Result<SelfUpdateJob> {
    if request.summary.trim().is_empty() {
        return Err(anyhow!("summary is required"));
    }

    with_jobs_mut(data_dir, |jobs| {
        ensure_no_active_job(jobs)?;
        let now = Utc::now().to_rfc3339();
        let job = SelfUpdateJob {
            id: Uuid::new_v4().to_string(),
            kind: "deploy".to_string(),
            summary: request.summary.trim().to_string(),
            state: "pending_approval".to_string(),
            requested_at: now.clone(),
            updated_at: now.clone(),
            requested_by: normalize_actor(&request.requested_by),
            approved_at: None,
            approved_by: None,
            started_at: None,
            finished_at: None,
            conversation_id: normalize_optional(request.conversation_id),
            project_id: normalize_optional(request.project_id),
            rollback_source_job_id: None,
            workspace_path: workspace_path(),
            compose_file: compose_file_path(),
            service_name: service_name(),
            image_name: image_name(),
            health_url: health_url(),
            check_cargo: request.check_cargo,
            check_frontend: request.check_frontend,
            build_image: request.build_image,
            restart_service: request.restart_service,
            active_step: None,
            status_message: "Waiting for approval.".to_string(),
            last_error: None,
            snapshot_path: None,
            backup_image_tag: None,
            steps: Vec::new(),
        };
        jobs.push(job.clone());
        trim_jobs(jobs);
        Ok(job)
    })
}

pub fn create_rollback_job(data_dir: &Path, request: CreateRollbackJob) -> Result<SelfUpdateJob> {
    with_jobs_mut(data_dir, |jobs| {
        ensure_no_active_job(jobs)?;
        let source = choose_rollback_source(jobs, request.source_job_id.as_deref())?.clone();
        let source_label = source.summary.trim();
        let now = Utc::now().to_rfc3339();
        let job = SelfUpdateJob {
            id: Uuid::new_v4().to_string(),
            kind: "rollback".to_string(),
            summary: format!(
                "Rollback to snapshot from {}",
                if source_label.is_empty() {
                    source.id.as_str()
                } else {
                    source_label
                }
            ),
            state: "pending_approval".to_string(),
            requested_at: now.clone(),
            updated_at: now.clone(),
            requested_by: normalize_actor(&request.requested_by),
            approved_at: None,
            approved_by: None,
            started_at: None,
            finished_at: None,
            conversation_id: normalize_optional(request.conversation_id),
            project_id: normalize_optional(request.project_id),
            rollback_source_job_id: Some(source.id.clone()),
            workspace_path: source.workspace_path.clone(),
            compose_file: source.compose_file.clone(),
            service_name: source.service_name.clone(),
            image_name: source.image_name.clone(),
            health_url: source.health_url.clone(),
            check_cargo: false,
            check_frontend: false,
            build_image: true,
            restart_service: true,
            active_step: None,
            status_message: "Waiting for approval.".to_string(),
            last_error: None,
            snapshot_path: source.snapshot_path.clone(),
            backup_image_tag: source.backup_image_tag.clone(),
            steps: Vec::new(),
        };
        jobs.push(job.clone());
        trim_jobs(jobs);
        Ok(job)
    })
}

pub fn approve_job(data_dir: &Path, id: &str, approved_by: &str) -> Result<SelfUpdateJob> {
    with_job_mut(data_dir, id, |job| {
        if job.state != "pending_approval" {
            return Err(anyhow!(
                "self-update job is not awaiting approval (state={})",
                job.state
            ));
        }
        let now = Utc::now().to_rfc3339();
        job.state = "queued".to_string();
        job.approved_at = Some(now.clone());
        job.approved_by = Some(normalize_actor(approved_by));
        job.updated_at = now;
        job.status_message = "Queued for updater.".to_string();
        Ok(job.clone())
    })
}

pub fn cancel_job(data_dir: &Path, id: &str) -> Result<SelfUpdateJob> {
    with_job_mut(data_dir, id, |job| {
        if matches!(
            job.state.as_str(),
            "running" | "succeeded" | "failed" | "failed_rolled_back"
        ) {
            return Err(anyhow!(
                "cannot cancel self-update job in state {}",
                job.state
            ));
        }
        let now = Utc::now().to_rfc3339();
        job.state = "cancelled".to_string();
        job.finished_at = Some(now.clone());
        job.updated_at = now;
        job.active_step = None;
        job.status_message = "Cancelled.".to_string();
        Ok(job.clone())
    })
}

pub fn list_jobs(data_dir: &Path, limit: usize) -> Result<Vec<SelfUpdateJob>> {
    let mut jobs = load_jobs(data_dir)?;
    jobs.sort_by(|a, b| b.updated_at.cmp(&a.updated_at));
    if jobs.len() > limit {
        jobs.truncate(limit);
    }
    Ok(jobs)
}

pub fn status_snapshot(data_dir: &Path, limit: usize) -> Result<SelfUpdateStatusSnapshot> {
    let jobs = list_jobs(data_dir, limit)?;
    let workspace = display_workspace_path();
    let compose = display_compose_file_path();
    let active_job = jobs
        .iter()
        .find(|job| {
            matches!(
                job.state.as_str(),
                "pending_approval" | "queued" | "running"
            )
        })
        .cloned();
    let latest_job = jobs.first().cloned();
    Ok(SelfUpdateStatusSnapshot {
        enabled: self_update_enabled(),
        workspace_exists: Path::new(&workspace).exists(),
        compose_exists: Path::new(&compose).exists(),
        workspace_path: workspace,
        compose_file: compose,
        service_name: service_name(),
        image_name: image_name(),
        health_url: health_url(),
        active_job,
        latest_job,
        jobs,
    })
}

pub fn jobs_file_path(data_dir: &Path) -> PathBuf {
    data_dir.join(SELF_UPDATE_DIR).join(SELF_UPDATE_JOBS_FILE)
}

pub async fn run_updater_loop(data_dir: &Path, poll_secs: u64) -> Result<()> {
    let interval_secs = poll_secs.max(1);
    tracing::info!(
        "self-update updater loop started (enabled={}, poll_secs={})",
        self_update_enabled(),
        interval_secs
    );

    loop {
        match claim_next_job(data_dir)? {
            Some(job) => {
                let result = if job.kind == "rollback" {
                    process_rollback_job(data_dir, job.clone()).await
                } else {
                    process_deploy_job(data_dir, job.clone()).await
                };
                if let Err(err) = result {
                    tracing::error!(job_id = %job.id, "self-update job failed unexpectedly: {}", err);
                    if let Ok(current) = get_job(data_dir, &job.id) {
                        if current.state == "running" {
                            let _ = complete_job(
                                data_dir,
                                &job.id,
                                "failed",
                                "Updater crashed while processing the job.",
                                Some(err.to_string()),
                            );
                        }
                    }
                }
            }
            None => tokio::time::sleep(Duration::from_secs(interval_secs)).await,
        }
    }
}

fn parse_bool_env(key: &str) -> Option<bool> {
    let raw = std::env::var(key).ok()?;
    let normalized = raw.trim().to_ascii_lowercase();
    match normalized.as_str() {
        "1" | "true" | "yes" | "on" => Some(true),
        "0" | "false" | "no" | "off" => Some(false),
        _ => None,
    }
}

fn normalize_actor(value: &str) -> String {
    let trimmed = value.trim();
    if trimmed.is_empty() {
        "user".to_string()
    } else {
        trimmed.to_string()
    }
}

fn normalize_optional(value: Option<String>) -> Option<String> {
    value
        .map(|v| v.trim().to_string())
        .filter(|v| !v.is_empty())
}

fn lock_file_path(data_dir: &Path) -> PathBuf {
    data_dir.join(SELF_UPDATE_DIR).join(SELF_UPDATE_LOCK_FILE)
}

fn snapshots_dir(data_dir: &Path) -> PathBuf {
    data_dir
        .join(SELF_UPDATE_DIR)
        .join(SELF_UPDATE_SNAPSHOTS_DIR)
}

fn restore_dir(data_dir: &Path, job_id: &str) -> PathBuf {
    data_dir
        .join(SELF_UPDATE_DIR)
        .join(SELF_UPDATE_RESTORE_DIR)
        .join(job_id)
}

fn ensure_self_update_dir(data_dir: &Path) -> Result<PathBuf> {
    let dir = data_dir.join(SELF_UPDATE_DIR);
    fs::create_dir_all(&dir).with_context(|| format!("failed to create {}", dir.display()))?;
    Ok(dir)
}

fn ensure_snapshots_dir(data_dir: &Path) -> Result<PathBuf> {
    let dir = snapshots_dir(data_dir);
    fs::create_dir_all(&dir).with_context(|| format!("failed to create {}", dir.display()))?;
    Ok(dir)
}

fn with_jobs_mut<F, T>(data_dir: &Path, mutator: F) -> Result<T>
where
    F: FnOnce(&mut Vec<SelfUpdateJob>) -> Result<T>,
{
    let _guard = acquire_jobs_lock(data_dir)?;
    let mut jobs = load_jobs_unlocked(data_dir)?;
    let out = mutator(&mut jobs)?;
    save_jobs_unlocked(data_dir, &jobs)?;
    Ok(out)
}

fn with_job_mut<F, T>(data_dir: &Path, id: &str, mutator: F) -> Result<T>
where
    F: FnOnce(&mut SelfUpdateJob) -> Result<T>,
{
    with_jobs_mut(data_dir, |jobs| {
        let job = jobs
            .iter_mut()
            .find(|job| job.id == id)
            .ok_or_else(|| anyhow!("self-update job not found"))?;
        mutator(job)
    })
}

fn get_job(data_dir: &Path, id: &str) -> Result<SelfUpdateJob> {
    load_jobs(data_dir)?
        .into_iter()
        .find(|job| job.id == id)
        .ok_or_else(|| anyhow!("self-update job not found"))
}

fn ensure_no_active_job(jobs: &[SelfUpdateJob]) -> Result<()> {
    if let Some(active) = jobs.iter().find(|job| job_is_active(job)) {
        return Err(anyhow!(
            "self-update job '{}' is already {}",
            active.summary,
            active.state
        ));
    }
    Ok(())
}

fn choose_rollback_source<'a>(
    jobs: &'a [SelfUpdateJob],
    source_job_id: Option<&str>,
) -> Result<&'a SelfUpdateJob> {
    if let Some(source_id) = source_job_id {
        return jobs
            .iter()
            .find(|job| job.id == source_id)
            .filter(|job| rollback_source_is_valid(job))
            .ok_or_else(|| anyhow!("rollback source job not found or cannot be rolled back"));
    }

    jobs.iter()
        .rev()
        .find(|job| rollback_source_is_valid(job))
        .ok_or_else(|| anyhow!("no successful self-update job with a rollback snapshot was found"))
}

fn rollback_source_is_valid(job: &SelfUpdateJob) -> bool {
    job.kind == "deploy"
        && matches!(job.state.as_str(), "succeeded" | "failed_rolled_back")
        && job
            .snapshot_path
            .as_deref()
            .map(|v| !v.trim().is_empty())
            .unwrap_or(false)
}

fn job_is_active(job: &SelfUpdateJob) -> bool {
    matches!(
        job.state.as_str(),
        "pending_approval" | "queued" | "running"
    )
}

fn load_jobs(data_dir: &Path) -> Result<Vec<SelfUpdateJob>> {
    let _guard = acquire_jobs_lock(data_dir)?;
    load_jobs_unlocked(data_dir)
}

fn load_jobs_unlocked(data_dir: &Path) -> Result<Vec<SelfUpdateJob>> {
    let path = jobs_file_path(data_dir);
    if !path.exists() {
        return Ok(Vec::new());
    }
    let raw = fs::read(&path).with_context(|| format!("failed to read {}", path.display()))?;
    let jobs = serde_json::from_slice(&raw)
        .with_context(|| format!("failed to parse {}", path.display()))?;
    Ok(jobs)
}

fn save_jobs_unlocked(data_dir: &Path, jobs: &[SelfUpdateJob]) -> Result<()> {
    let dir = ensure_self_update_dir(data_dir)?;
    let path = jobs_file_path(data_dir);
    let tmp = dir.join(format!("{}.tmp", SELF_UPDATE_JOBS_FILE));
    let bytes = serde_json::to_vec_pretty(jobs)?;
    fs::write(&tmp, bytes).with_context(|| format!("failed to write {}", tmp.display()))?;
    fs::rename(&tmp, &path).with_context(|| format!("failed to replace {}", path.display()))?;
    Ok(())
}

fn trim_jobs(jobs: &mut Vec<SelfUpdateJob>) {
    jobs.sort_by(|a, b| b.updated_at.cmp(&a.updated_at));
    if jobs.len() > MAX_SELF_UPDATE_JOBS {
        jobs.truncate(MAX_SELF_UPDATE_JOBS);
    }
    jobs.sort_by(|a, b| a.requested_at.cmp(&b.requested_at));
}

struct JobsLockGuard {
    path: PathBuf,
}

impl Drop for JobsLockGuard {
    fn drop(&mut self) {
        let _ = fs::remove_file(&self.path);
    }
}

fn acquire_jobs_lock(data_dir: &Path) -> Result<JobsLockGuard> {
    let dir = ensure_self_update_dir(data_dir)?;
    let path = lock_file_path(data_dir);
    let mut attempts = 0usize;
    loop {
        match fs::OpenOptions::new()
            .create_new(true)
            .write(true)
            .open(&path)
        {
            Ok(_) => return Ok(JobsLockGuard { path }),
            Err(e) if e.kind() == std::io::ErrorKind::AlreadyExists && attempts < 100 => {
                attempts += 1;
                if attempts == 50 {
                    if let Ok(meta) = fs::metadata(&path) {
                        if let Ok(modified) = meta.modified() {
                            if modified
                                .elapsed()
                                .map(|age| age > Duration::from_secs(300))
                                .unwrap_or(false)
                            {
                                let _ = fs::remove_file(&path);
                            }
                        }
                    }
                }
                thread::sleep(Duration::from_millis(50));
            }
            Err(e) => {
                return Err(e).with_context(|| {
                    format!("failed to lock self-update journal in {}", dir.display())
                });
            }
        }
    }
}

fn claim_next_job(data_dir: &Path) -> Result<Option<SelfUpdateJob>> {
    with_jobs_mut(data_dir, |jobs| {
        let Some(job) = jobs.iter_mut().find(|job| job.state == "queued") else {
            return Ok(None);
        };
        let now = Utc::now().to_rfc3339();
        job.state = "running".to_string();
        job.started_at = Some(now.clone());
        job.updated_at = now;
        job.status_message = "Updater started.".to_string();
        job.last_error = None;
        job.active_step = None;
        Ok(Some(job.clone()))
    })
}

fn upsert_step(job: &mut SelfUpdateJob, key: &str, label: &str, status: &str, detail: String) {
    let now = Utc::now().to_rfc3339();
    if let Some(step) = job.steps.iter_mut().find(|step| step.key == key) {
        step.label = label.to_string();
        step.status = status.to_string();
        step.detail = detail;
        if status == "running" {
            step.started_at = Some(now.clone());
            step.completed_at = None;
        } else {
            if step.started_at.is_none() {
                step.started_at = Some(now.clone());
            }
            step.completed_at = Some(now.clone());
        }
    } else {
        job.steps.push(SelfUpdateStep {
            key: key.to_string(),
            label: label.to_string(),
            status: status.to_string(),
            detail,
            started_at: Some(now.clone()),
            completed_at: if status == "running" {
                None
            } else {
                Some(now.clone())
            },
        });
    }
    job.active_step = if status == "running" {
        Some(key.to_string())
    } else if job.active_step.as_deref() == Some(key) {
        None
    } else {
        job.active_step.clone()
    };
    job.updated_at = now;
}

fn set_step_state(
    data_dir: &Path,
    id: &str,
    key: &str,
    label: &str,
    status: &str,
    detail: String,
) -> Result<()> {
    with_job_mut(data_dir, id, |job| {
        upsert_step(job, key, label, status, detail.clone());
        if status == "failed" {
            job.last_error = Some(detail.clone());
        }
        job.status_message = if status == "running" {
            format!("{}...", label)
        } else {
            detail.clone()
        };
        Ok(())
    })
}

fn set_job_artifacts(
    data_dir: &Path,
    id: &str,
    snapshot_path: Option<String>,
    backup_image_tag: Option<String>,
) -> Result<()> {
    with_job_mut(data_dir, id, |job| {
        if let Some(path) = snapshot_path {
            job.snapshot_path = Some(path);
        }
        if let Some(tag) = backup_image_tag {
            job.backup_image_tag = Some(tag);
        }
        Ok(())
    })
}

fn complete_job(
    data_dir: &Path,
    id: &str,
    state: &str,
    status_message: &str,
    last_error: Option<String>,
) -> Result<()> {
    with_job_mut(data_dir, id, |job| {
        let now = Utc::now().to_rfc3339();
        job.state = state.to_string();
        job.finished_at = Some(now.clone());
        job.updated_at = now;
        job.active_step = None;
        job.status_message = status_message.trim().to_string();
        job.last_error = last_error;
        Ok(())
    })
}

async fn process_deploy_job(data_dir: &Path, job: SelfUpdateJob) -> Result<()> {
    if let Err(err) = validate_paths(data_dir, &job).await {
        complete_job(
            data_dir,
            &job.id,
            "failed",
            "Self-update configuration is invalid.",
            Some(err.to_string()),
        )?;
        return Ok(());
    }

    let snapshot_path = match create_snapshot_step(data_dir, &job).await {
        Ok(path) => path,
        Err(err) => {
            complete_job(
                data_dir,
                &job.id,
                "failed",
                "Failed to create rollback snapshot.",
                Some(err.to_string()),
            )?;
            return Ok(());
        }
    };

    let backup_tag = match backup_current_image_step(data_dir, &job).await {
        Ok(tag) => tag,
        Err(err) => {
            complete_job(
                data_dir,
                &job.id,
                "failed",
                "Failed while backing up the current image.",
                Some(err.to_string()),
            )?;
            return Ok(());
        }
    };

    let step_result = async {
        if job.check_cargo {
            run_shell_step(
                data_dir,
                &job.id,
                "cargo_check",
                "Rust build check",
                docker_build_target_command(&job.workspace_path, "builder"),
                "Rust build check completed.",
            )
            .await?;
        } else {
            set_step_state(
                data_dir,
                &job.id,
                "cargo_check",
                "Rust build check",
                "skipped",
                "Rust build check skipped for this job.".to_string(),
            )?;
        }

        if job.check_frontend {
            run_shell_step(
                data_dir,
                &job.id,
                "frontend_build",
                "Frontend build check",
                docker_build_target_command(&job.workspace_path, "frontend-builder"),
                "Frontend build check completed.",
            )
            .await?;
        } else {
            set_step_state(
                data_dir,
                &job.id,
                "frontend_build",
                "Frontend build check",
                "skipped",
                "Frontend build check skipped for this job.".to_string(),
            )?;
        }

        if job.build_image {
            run_shell_step(
                data_dir,
                &job.id,
                "build_image",
                "Build container image",
                compose_command(&job.compose_file, &format!("build {}", job.service_name)),
                "Container image built successfully.",
            )
            .await?;
        } else {
            set_step_state(
                data_dir,
                &job.id,
                "build_image",
                "Build container image",
                "skipped",
                "Container image build skipped for this job.".to_string(),
            )?;
        }

        if job.restart_service {
            run_shell_step(
                data_dir,
                &job.id,
                "restart_service",
                "Restart service",
                compose_command(
                    &job.compose_file,
                    &format!("up -d --no-deps {}", job.service_name),
                ),
                "Service restart command completed.",
            )
            .await?;
            wait_for_health_step(
                data_dir,
                &job.id,
                "health_check",
                "Health check",
                &job.health_url,
            )
            .await?;
        } else {
            set_step_state(
                data_dir,
                &job.id,
                "restart_service",
                "Restart service",
                "skipped",
                "Service restart skipped for this job.".to_string(),
            )?;
        }

        Ok::<(), anyhow::Error>(())
    }
    .await;

    match step_result {
        Ok(()) => {
            complete_job(
                data_dir,
                &job.id,
                "succeeded",
                "Self-update completed successfully.",
                None,
            )?;
        }
        Err(err) => {
            handle_failed_deploy(
                data_dir,
                &job,
                &snapshot_path,
                backup_tag.as_deref(),
                &err.to_string(),
            )
            .await?;
        }
    }

    Ok(())
}

async fn process_rollback_job(data_dir: &Path, job: SelfUpdateJob) -> Result<()> {
    if let Err(err) = validate_paths(data_dir, &job).await {
        complete_job(
            data_dir,
            &job.id,
            "failed",
            "Self-update configuration is invalid.",
            Some(err.to_string()),
        )?;
        return Ok(());
    }

    let source_job_id = match job.rollback_source_job_id.as_deref() {
        Some(id) => id,
        None => {
            complete_job(
                data_dir,
                &job.id,
                "failed",
                "Rollback source job is missing.",
                Some("rollback_source_job_id is missing".to_string()),
            )?;
            return Ok(());
        }
    };
    let source_job = get_job(data_dir, source_job_id)?;
    let snapshot_path = match source_job.snapshot_path.as_deref() {
        Some(path) if !path.trim().is_empty() => path,
        _ => {
            complete_job(
                data_dir,
                &job.id,
                "failed",
                "Rollback snapshot is not available.",
                Some("snapshot_path is missing for the selected rollback source".to_string()),
            )?;
            return Ok(());
        }
    };

    match restore_previous_version(
        data_dir,
        &job,
        snapshot_path,
        source_job.backup_image_tag.as_deref(),
        "rollback",
    )
    .await
    {
        Ok(()) => complete_job(
            data_dir,
            &job.id,
            "succeeded",
            "Rollback completed successfully.",
            None,
        )?,
        Err(err) => complete_job(
            data_dir,
            &job.id,
            "failed",
            "Rollback failed.",
            Some(err.to_string()),
        )?,
    }

    Ok(())
}

async fn validate_paths(data_dir: &Path, job: &SelfUpdateJob) -> Result<()> {
    if !Path::new(&job.workspace_path).exists() {
        set_step_state(
            data_dir,
            &job.id,
            "validate_paths",
            "Validate source paths",
            "failed",
            format!("Workspace path does not exist: {}", job.workspace_path),
        )?;
        return Err(anyhow!(
            "workspace path does not exist: {}",
            job.workspace_path
        ));
    }
    if !Path::new(&job.compose_file).exists() {
        set_step_state(
            data_dir,
            &job.id,
            "validate_paths",
            "Validate source paths",
            "failed",
            format!("Compose file does not exist: {}", job.compose_file),
        )?;
        return Err(anyhow!("compose file does not exist: {}", job.compose_file));
    }
    set_step_state(
        data_dir,
        &job.id,
        "validate_paths",
        "Validate source paths",
        "succeeded",
        "Workspace and compose file are available.".to_string(),
    )?;
    Ok(())
}

async fn create_snapshot_step(data_dir: &Path, job: &SelfUpdateJob) -> Result<String> {
    let snapshots = ensure_snapshots_dir(data_dir)?;
    let snapshot_path = snapshots.join(format!("{}.tar.gz", job.id));
    let command = create_snapshot_command(&job.workspace_path, &snapshot_path);
    run_shell_step(
        data_dir,
        &job.id,
        "snapshot",
        "Create rollback snapshot",
        command,
        &format!("Rollback snapshot saved to {}.", snapshot_path.display()),
    )
    .await?;
    let snapshot_string = snapshot_path.to_string_lossy().to_string();
    set_job_artifacts(data_dir, &job.id, Some(snapshot_string.clone()), None)?;
    Ok(snapshot_string)
}

async fn backup_current_image_step(data_dir: &Path, job: &SelfUpdateJob) -> Result<Option<String>> {
    let backup_tag = format!(
        "{}-backup-{}",
        sanitize_image_tag(&job.image_name),
        short_job_id(&job.id)
    );
    let command = format!(
        "if docker image inspect {image} >/dev/null 2>&1; then docker tag {image} {backup} && echo backed_up; else echo no_existing_image; fi",
        image = quote_shell(&job.image_name),
        backup = quote_shell(&backup_tag)
    );
    let output = run_shell_step(
        data_dir,
        &job.id,
        "backup_image",
        "Backup current image",
        command,
        "Current image snapshot recorded.",
    )
    .await?;
    if output.contains("backed_up") {
        set_job_artifacts(data_dir, &job.id, None, Some(backup_tag.clone()))?;
        Ok(Some(backup_tag))
    } else {
        set_step_state(
            data_dir,
            &job.id,
            "backup_image",
            "Backup current image",
            "succeeded",
            "No existing image tag was present; rollback will rebuild from snapshot if needed."
                .to_string(),
        )?;
        Ok(None)
    }
}

async fn handle_failed_deploy(
    data_dir: &Path,
    job: &SelfUpdateJob,
    snapshot_path: &str,
    backup_tag: Option<&str>,
    deploy_error: &str,
) -> Result<()> {
    match restore_previous_version(data_dir, job, snapshot_path, backup_tag, "auto_rollback").await
    {
        Ok(()) => complete_job(
            data_dir,
            &job.id,
            "failed_rolled_back",
            "Update failed; previous version was restored.",
            Some(deploy_error.to_string()),
        ),
        Err(rollback_err) => complete_job(
            data_dir,
            &job.id,
            "failed",
            "Update failed and automatic rollback also failed.",
            Some(format!(
                "deploy error: {}; rollback error: {}",
                deploy_error, rollback_err
            )),
        ),
    }
}

async fn restore_previous_version(
    data_dir: &Path,
    job: &SelfUpdateJob,
    snapshot_path: &str,
    backup_tag: Option<&str>,
    step_prefix: &str,
) -> Result<()> {
    let restore_workspace = restore_dir(data_dir, &job.id);
    if let Some(parent) = restore_workspace.parent() {
        fs::create_dir_all(parent)
            .with_context(|| format!("failed to create {}", parent.display()))?;
    }

    run_shell_step(
        data_dir,
        &job.id,
        &format!("{}_restore_code", step_prefix),
        "Restore source snapshot",
        restore_snapshot_command(&job.workspace_path, snapshot_path, &restore_workspace),
        "Source snapshot restored.",
    )
    .await?;

    if let Some(tag) = backup_tag {
        run_shell_step(
            data_dir,
            &job.id,
            &format!("{}_restore_image", step_prefix),
            "Restore previous image",
            format!(
                "docker image inspect {tag} >/dev/null 2>&1 && docker tag {tag} {image}",
                tag = quote_shell(tag),
                image = quote_shell(&job.image_name)
            ),
            "Previous image tag restored.",
        )
        .await?;
    } else {
        run_shell_step(
            data_dir,
            &job.id,
            &format!("{}_rebuild_image", step_prefix),
            "Rebuild restored image",
            compose_command(&job.compose_file, &format!("build {}", job.service_name)),
            "Restored source rebuilt successfully.",
        )
        .await?;
    }

    run_shell_step(
        data_dir,
        &job.id,
        &format!("{}_restart_service", step_prefix),
        "Restart restored service",
        compose_command(
            &job.compose_file,
            &format!("up -d --no-deps {}", job.service_name),
        ),
        "Restored service restart command completed.",
    )
    .await?;

    wait_for_health_step(
        data_dir,
        &job.id,
        &format!("{}_health_check", step_prefix),
        "Health check after restore",
        &job.health_url,
    )
    .await?;

    Ok(())
}

async fn run_shell_step(
    data_dir: &Path,
    id: &str,
    key: &str,
    label: &str,
    command: String,
    success_message: &str,
) -> Result<String> {
    set_step_state(data_dir, id, key, label, "running", format!("{}...", label))?;
    match run_shell_command(&command).await {
        Ok(output) => {
            let success_detail = if output.trim().is_empty() {
                success_message.trim().to_string()
            } else {
                format!("{} {}", success_message.trim(), output.trim())
            };
            set_step_state(
                data_dir,
                id,
                key,
                label,
                "succeeded",
                success_detail.clone(),
            )?;
            Ok(success_detail)
        }
        Err(err) => {
            set_step_state(data_dir, id, key, label, "failed", err.to_string())?;
            Err(err)
        }
    }
}

async fn wait_for_health_step(
    data_dir: &Path,
    id: &str,
    key: &str,
    label: &str,
    url: &str,
) -> Result<()> {
    set_step_state(
        data_dir,
        id,
        key,
        label,
        "running",
        format!("Polling {}", url),
    )?;

    let timeout_secs = std::env::var("AGENTARK_SELF_UPDATE_HEALTH_TIMEOUT_SECS")
        .ok()
        .and_then(|v| v.trim().parse::<u64>().ok())
        .filter(|v| *v > 0)
        .unwrap_or(DEFAULT_HEALTH_TIMEOUT_SECS);
    let client = reqwest::Client::builder()
        .timeout(Duration::from_secs(10))
        .build()
        .context("failed to build health-check client")?;
    let started = std::time::Instant::now();

    loop {
        match client.get(url).send().await {
            Ok(resp) if resp.status().is_success() => {
                set_step_state(
                    data_dir,
                    id,
                    key,
                    label,
                    "succeeded",
                    format!("Health check passed at {}.", url),
                )?;
                return Ok(());
            }
            Ok(resp) => {
                if started.elapsed() >= Duration::from_secs(timeout_secs) {
                    let message = format!(
                        "Health check failed with status {} at {}.",
                        resp.status(),
                        url
                    );
                    set_step_state(data_dir, id, key, label, "failed", message.clone())?;
                    return Err(anyhow!(message));
                }
            }
            Err(err) => {
                if started.elapsed() >= Duration::from_secs(timeout_secs) {
                    let message = format!("Health check failed at {}: {}", url, err);
                    set_step_state(data_dir, id, key, label, "failed", message.clone())?;
                    return Err(anyhow!(message));
                }
            }
        }
        tokio::time::sleep(Duration::from_secs(5)).await;
    }
}

async fn run_shell_command(command: &str) -> Result<String> {
    let mut shell = new_shell_command(command);
    shell.stdin(Stdio::null());
    let output = shell.output().await.context("failed to spawn command")?;
    let stdout = String::from_utf8_lossy(&output.stdout).to_string();
    let stderr = String::from_utf8_lossy(&output.stderr).to_string();
    let combined = format!("{}\n{}", stdout.trim(), stderr.trim())
        .trim()
        .to_string();
    if output.status.success() {
        Ok(truncate_output(&combined))
    } else {
        let detail = if combined.is_empty() {
            format!("command failed with status {}", output.status)
        } else {
            format!("command failed: {}", truncate_output(&combined))
        };
        Err(anyhow!(detail))
    }
}

#[cfg(target_os = "windows")]
fn new_shell_command(command: &str) -> Command {
    let mut shell = Command::new("powershell");
    shell
        .arg("-NoProfile")
        .arg("-NonInteractive")
        .arg("-Command")
        .arg(command);
    shell
}

#[cfg(not(target_os = "windows"))]
fn new_shell_command(command: &str) -> Command {
    let mut shell = Command::new("bash");
    shell.arg("-lc").arg(command);
    shell
}

fn truncate_output(value: &str) -> String {
    let trimmed = value.trim();
    if trimmed.chars().count() <= COMMAND_OUTPUT_LIMIT {
        return trimmed.to_string();
    }
    let suffix: String = trimmed
        .chars()
        .rev()
        .take(COMMAND_OUTPUT_LIMIT)
        .collect::<String>()
        .chars()
        .rev()
        .collect();
    format!("...{}", suffix)
}

fn compose_command(compose_file: &str, args: &str) -> String {
    format!(
        "if docker compose version >/dev/null 2>&1; then docker compose -f {} {}; else docker-compose -f {} {}; fi",
        quote_shell(compose_file),
        args.trim(),
        quote_shell(compose_file),
        args.trim()
    )
}

fn docker_build_target_command(workspace_path: &str, target: &str) -> String {
    let dockerfile = Path::new(workspace_path).join("Dockerfile");
    format!(
        "docker build --target {} -f {} {}",
        target,
        quote_shell(&dockerfile.to_string_lossy()),
        quote_shell(workspace_path)
    )
}

fn create_snapshot_command(workspace_path: &str, snapshot_path: &Path) -> String {
    format!(
        "mkdir -p {parent} && tar --exclude=.git --exclude=target --exclude=frontend/node_modules --exclude=node_modules --exclude=.venv -czf {snapshot} -C {workspace} .",
        parent = quote_shell(
            &snapshot_path
                .parent()
                .unwrap_or_else(|| Path::new("."))
                .to_string_lossy()
        ),
        snapshot = quote_shell(&snapshot_path.to_string_lossy()),
        workspace = quote_shell(workspace_path)
    )
}

fn restore_snapshot_command(
    workspace_path: &str,
    snapshot_path: &str,
    restore_workspace: &Path,
) -> String {
    format!(
        "rm -rf {restore} && mkdir -p {restore} && tar -xzf {snapshot} -C {restore} && find {workspace} -mindepth 1 -maxdepth 1 ! -name .git -exec rm -rf {{}} + && cp -a {restore}/. {workspace}/",
        restore = quote_shell(&restore_workspace.to_string_lossy()),
        snapshot = quote_shell(snapshot_path),
        workspace = quote_shell(workspace_path)
    )
}

#[cfg(target_os = "windows")]
fn quote_shell(value: &str) -> String {
    format!("'{}'", value.replace('\'', "''"))
}

#[cfg(not(target_os = "windows"))]
fn quote_shell(value: &str) -> String {
    format!("'{}'", value.replace('\'', "'\"'\"'"))
}

fn sanitize_image_tag(value: &str) -> String {
    value
        .chars()
        .map(|ch| match ch {
            'a'..='z' | 'A'..='Z' | '0'..='9' | '_' | '-' | '.' => ch,
            _ => '-',
        })
        .collect()
}

fn short_job_id(job_id: &str) -> String {
    job_id.chars().take(8).collect()
}
