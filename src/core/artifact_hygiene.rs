use std::collections::{HashMap, HashSet};
use std::path::{Path, PathBuf};
use std::time::{Duration as StdDuration, SystemTime};

use anyhow::{Context, Result};
use chrono::{DateTime, Utc};
use serde::{Deserialize, Serialize};
use sha2::Digest;

use crate::core::data_lifecycle::DataLifecycleSettings;

pub const ARTIFACT_ARCHIVE_DIR: &str = "artifact_archive";
pub const LEGACY_APP_ARCHIVE_DIR: &str = "app_archive";
pub const ARCHIVE_MANIFEST_FILE: &str = ".agentark_archive_manifest.json";
pub const ARCHIVE_RETENTION_DAYS: i64 = 14;
pub const ARTIFACT_RETENTION_SECS: u64 = 14 * 24 * 60 * 60;

const IDLE_APP_HOURS: i64 = 24;
const UNMANAGED_APP_FILE_RETENTION_SECS: u64 = 24 * 60 * 60;

#[derive(Debug, Clone)]
pub struct ManagedArtifactApp {
    pub id: String,
    pub title: String,
    pub app_dir: PathBuf,
    pub enabled: bool,
    pub running: bool,
    pub is_static: bool,
    pub created_at: Option<DateTime<Utc>>,
}

#[derive(Debug, Clone, Serialize)]
pub struct ArtifactCleanupCandidate {
    pub id: String,
    pub category: String,
    pub category_label: String,
    pub path_label: String,
    pub size_bytes: u64,
    pub age_seconds: u64,
    pub age_days: f64,
    pub risk: String,
    pub reason: String,
    pub selected_by_default: bool,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub app_id: Option<String>,
    #[serde(default)]
    pub requires_app_stop: bool,
    #[serde(skip)]
    pub source_path: PathBuf,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ArchivedArtifactManifest {
    pub source_path: String,
    pub source_path_label: String,
    pub category: String,
    pub archived_at: String,
    pub size_bytes: u64,
    pub reason: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub event_timestamp: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub finding_index: Option<usize>,
}

#[derive(Debug, Clone, Serialize)]
pub struct ArchivedArtifactOutcome {
    pub candidate_id: String,
    pub category: String,
    pub source_path_label: String,
    pub archive_path_label: String,
    pub size_bytes: u64,
}

#[derive(Debug, Clone, Default, Serialize)]
pub struct ArchiveRetentionSummary {
    pub roots_checked: usize,
    pub deleted_entries: u64,
    pub deleted_bytes: u64,
}

pub fn artifact_archive_root(data_dir: &Path) -> PathBuf {
    data_dir.join(ARTIFACT_ARCHIVE_DIR)
}

pub fn legacy_app_archive_root(data_dir: &Path) -> PathBuf {
    data_dir.join(LEGACY_APP_ARCHIVE_DIR)
}

pub async fn collect_artifact_cleanup_candidates(
    data_dir: &Path,
    apps: &[ManagedArtifactApp],
    idle_apps: &HashMap<String, DateTime<Utc>>,
    lifecycle: &DataLifecycleSettings,
) -> Result<Vec<ArtifactCleanupCandidate>> {
    let data_root = match tokio::fs::canonicalize(data_dir).await {
        Ok(path) => path,
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => return Ok(Vec::new()),
        Err(error) => return Err(error.into()),
    };

    let mut candidates = Vec::new();
    let mut seen_paths = HashSet::new();
    collect_managed_app_candidates(
        &data_root,
        apps,
        idle_apps,
        &mut seen_paths,
        &mut candidates,
    )
    .await?;
    collect_known_residue_roots(&data_root, lifecycle, &mut candidates).await?;
    collect_agentark_temp_residue(&data_root, &mut candidates).await?;

    candidates.sort_by(|left, right| {
        left.category
            .cmp(&right.category)
            .then_with(|| right.size_bytes.cmp(&left.size_bytes))
            .then_with(|| left.path_label.cmp(&right.path_label))
    });
    Ok(candidates)
}

pub async fn archive_cleanup_candidate(
    data_dir: &Path,
    candidate: &ArtifactCleanupCandidate,
    event_timestamp: Option<String>,
    finding_index: Option<usize>,
) -> Result<ArchivedArtifactOutcome> {
    let data_root = tokio::fs::canonicalize(data_dir)
        .await
        .with_context(|| format!("data directory is not accessible: {}", data_dir.display()))?;
    let source = canonical_source_for_cleanup(&candidate.source_path, &data_root).await?;
    ensure_cleanup_source_allowed(&source, &data_root)?;

    let metadata = tokio::fs::symlink_metadata(&source).await?;
    if metadata.file_type().is_symlink() {
        anyhow::bail!("refusing to archive symlink {}", source.display());
    }
    if !metadata.is_dir() && !metadata.is_file() {
        anyhow::bail!(
            "refusing to archive unsupported file type {}",
            source.display()
        );
    }

    let archive_category = sanitize_path_segment(&candidate.category);
    let category_root = artifact_archive_root(&data_root).join(&archive_category);
    tokio::fs::create_dir_all(&category_root).await?;
    let entry_name = archive_entry_name(candidate);
    let mut archive_entry = category_root.join(&entry_name);
    if tokio::fs::try_exists(&archive_entry).await.unwrap_or(false) {
        archive_entry =
            category_root.join(format!("{}-{}", entry_name, uuid::Uuid::new_v4().simple()));
    }

    if metadata.is_dir() {
        move_path_to_archive(&source, &archive_entry).await?;
    } else {
        tokio::fs::create_dir_all(&archive_entry).await?;
        let file_name = source
            .file_name()
            .and_then(|value| value.to_str())
            .map(sanitize_path_segment)
            .filter(|value| !value.is_empty())
            .unwrap_or_else(|| "artifact".to_string());
        move_path_to_archive(&source, &archive_entry.join(file_name)).await?;
    }

    let manifest = ArchivedArtifactManifest {
        source_path: source.display().to_string(),
        source_path_label: path_label(&source, &data_root),
        category: candidate.category.clone(),
        archived_at: Utc::now().to_rfc3339(),
        size_bytes: candidate.size_bytes,
        reason: candidate.reason.clone(),
        event_timestamp,
        finding_index,
    };
    let manifest_path = archive_entry.join(ARCHIVE_MANIFEST_FILE);
    let raw = serde_json::to_vec_pretty(&manifest)?;
    tokio::fs::write(&manifest_path, raw).await?;

    Ok(ArchivedArtifactOutcome {
        candidate_id: candidate.id.clone(),
        category: candidate.category.clone(),
        source_path_label: candidate.path_label.clone(),
        archive_path_label: path_label(&archive_entry, &data_root),
        size_bytes: candidate.size_bytes,
    })
}

pub async fn prune_archive_retention(data_dir: &Path) -> Result<ArchiveRetentionSummary> {
    let data_root = match tokio::fs::canonicalize(data_dir).await {
        Ok(path) => path,
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => {
            return Ok(ArchiveRetentionSummary::default());
        }
        Err(error) => return Err(error.into()),
    };
    let roots = [
        artifact_archive_root(&data_root),
        legacy_app_archive_root(&data_root),
    ];
    let mut summary = ArchiveRetentionSummary::default();
    for root in roots {
        let root = match tokio::fs::canonicalize(&root).await {
            Ok(root) => root,
            Err(error) if error.kind() == std::io::ErrorKind::NotFound => continue,
            Err(error) => return Err(error.into()),
        };
        summary.roots_checked += 1;
        prune_archive_root(&root, &mut summary).await?;
    }
    Ok(summary)
}

pub fn candidate_category_counts(
    candidates: &[ArtifactCleanupCandidate],
) -> Vec<(String, usize, u64)> {
    let mut counts: HashMap<String, (usize, u64)> = HashMap::new();
    for candidate in candidates {
        let entry = counts.entry(candidate.category_label.clone()).or_default();
        entry.0 += 1;
        entry.1 = entry.1.saturating_add(candidate.size_bytes);
    }
    let mut out = counts
        .into_iter()
        .map(|(label, (count, bytes))| (label, count, bytes))
        .collect::<Vec<_>>();
    out.sort_by(|left, right| left.0.cmp(&right.0));
    out
}

async fn collect_managed_app_candidates(
    data_root: &Path,
    apps: &[ManagedArtifactApp],
    idle_apps: &HashMap<String, DateTime<Utc>>,
    seen_paths: &mut HashSet<PathBuf>,
    candidates: &mut Vec<ArtifactCleanupCandidate>,
) -> Result<()> {
    let app_ids = apps
        .iter()
        .map(|app| app.id.trim().to_string())
        .filter(|app_id| !app_id.is_empty())
        .collect::<HashSet<_>>();

    for app in apps {
        if app.id.trim().is_empty() {
            continue;
        }
        let Some(source) = cleanup_child_path(&app.app_dir, data_root).await else {
            continue;
        };
        if seen_paths.contains(&source) {
            continue;
        }
        if !path_is_dir_no_symlink(&source).await {
            continue;
        }
        if !app.enabled {
            add_candidate(
                candidates,
                CandidateInput {
                    data_root,
                    source_path: source.clone(),
                    category: "managed_apps",
                    category_label: "Managed apps",
                    risk: "medium",
                    reason: format!(
                        "Disabled managed app '{}' can be archived from the live app set.",
                        display_app_name(app)
                    ),
                    selected_by_default: false,
                    app_id: Some(app.id.clone()),
                    requires_app_stop: app.running && !app.is_static,
                    age_reference: app.created_at.or_else(|| modified_at_datetime(&source)),
                },
            )
            .await?;
            seen_paths.insert(source);
            continue;
        }
        if let Some(last_accessed) = idle_apps.get(&app.id) {
            add_candidate(
                candidates,
                CandidateInput {
                    data_root,
                    source_path: source.clone(),
                    category: "managed_apps",
                    category_label: "Managed apps",
                    risk: if app.running { "medium" } else { "low" },
                    reason: format!(
                        "Managed app '{}' has been idle for at least {} hours.",
                        display_app_name(app),
                        IDLE_APP_HOURS
                    ),
                    selected_by_default: false,
                    app_id: Some(app.id.clone()),
                    requires_app_stop: app.running && !app.is_static,
                    age_reference: Some(*last_accessed),
                },
            )
            .await?;
            seen_paths.insert(source);
        }
    }

    let apps_dir = data_root.join("apps");
    let mut entries = match tokio::fs::read_dir(&apps_dir).await {
        Ok(entries) => entries,
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => return Ok(()),
        Err(error) => return Err(error.into()),
    };
    while let Some(entry) = entries.next_entry().await? {
        let path = entry.path();
        let metadata = match entry.metadata().await {
            Ok(metadata) => metadata,
            Err(error) if error.kind() == std::io::ErrorKind::NotFound => continue,
            Err(error) => return Err(error.into()),
        };
        let name = entry.file_name().to_string_lossy().trim().to_string();
        if name.is_empty() || name.eq_ignore_ascii_case("new") {
            continue;
        }
        if metadata.is_file() {
            let modified = metadata.modified().ok();
            if !older_than(modified, UNMANAGED_APP_FILE_RETENTION_SECS) {
                continue;
            }
            let Some(source) = cleanup_child_path(&path, data_root).await else {
                continue;
            };
            add_candidate(
                candidates,
                CandidateInput {
                    data_root,
                    source_path: source,
                    category: "managed_apps",
                    category_label: "Managed apps",
                    risk: "low",
                    reason: "Unmanaged file in the AgentArk apps root.".to_string(),
                    selected_by_default: true,
                    app_id: None,
                    requires_app_stop: false,
                    age_reference: modified.and_then(system_time_to_datetime),
                },
            )
            .await?;
            continue;
        }
        if !metadata.is_dir() {
            continue;
        }
        let Some(source) = cleanup_child_path(&path, data_root).await else {
            continue;
        };
        if seen_paths.contains(&source) {
            continue;
        }
        let meta_path = source.join(".app_meta.json");
        let meta_status = match tokio::fs::read(&meta_path).await {
            Ok(raw) => match serde_json::from_slice::<serde_json::Value>(&raw) {
                Ok(value) if value.is_object() => "valid",
                Ok(_) => "corrupt",
                Err(_) => "corrupt",
            },
            Err(error) if error.kind() == std::io::ErrorKind::NotFound => "missing",
            Err(_) => "unreadable",
        };
        if !app_ids.contains(&name) || meta_status != "valid" {
            add_candidate(
                candidates,
                CandidateInput {
                    data_root,
                    source_path: source.clone(),
                    category: "managed_apps",
                    category_label: "Managed apps",
                    risk: "medium",
                    reason: if !app_ids.contains(&name) {
                        "Orphan app directory is not registered as a live managed app.".to_string()
                    } else {
                        format!("Managed app metadata is {}.", meta_status)
                    },
                    selected_by_default: !app_ids.contains(&name),
                    app_id: app_ids.contains(&name).then_some(name.clone()),
                    requires_app_stop: app_ids.contains(&name),
                    age_reference: modified_at_datetime(&source),
                },
            )
            .await?;
            seen_paths.insert(source);
        }
    }
    Ok(())
}

async fn collect_known_residue_roots(
    data_root: &Path,
    lifecycle: &DataLifecycleSettings,
    candidates: &mut Vec<ArtifactCleanupCandidate>,
) -> Result<()> {
    let roots = [
        ResidueRoot {
            relative: "outputs",
            category: "runtime_residue",
            label: "Runtime residue",
            reason: "Stale code execution output past AgentArk runtime retention.",
            retention_secs: Some(ARTIFACT_RETENTION_SECS),
            risk: "low",
            selected_by_default: true,
        },
        ResidueRoot {
            relative: "runtime_bundles",
            category: "runtime_residue",
            label: "Runtime residue",
            reason: "Abandoned runtime bundle past AgentArk artifact retention.",
            retention_secs: Some(ARTIFACT_RETENTION_SECS),
            risk: "low",
            selected_by_default: true,
        },
        ResidueRoot {
            relative: "extracted_artifacts",
            category: "runtime_residue",
            label: "Runtime residue",
            reason: "Stale extracted artifact past AgentArk artifact retention.",
            retention_secs: Some(ARTIFACT_RETENTION_SECS),
            risk: "low",
            selected_by_default: true,
        },
        ResidueRoot {
            relative: "browser_sessions",
            category: "browser_session_residue",
            label: "Browser/session residue",
            reason: "Closed browser-session artifact past configured retention.",
            retention_secs: retention_secs(lifecycle.browser_session_retention_days),
            risk: "low",
            selected_by_default: true,
        },
        ResidueRoot {
            relative: "browser_scratch",
            category: "browser_session_residue",
            label: "Browser/session residue",
            reason: "Stale browser scratch artifact past configured retention.",
            retention_secs: retention_secs(lifecycle.browser_session_retention_days),
            risk: "low",
            selected_by_default: true,
        },
        ResidueRoot {
            relative: "run_scratch",
            category: "browser_session_residue",
            label: "Browser/session residue",
            reason: "Stale run scratch artifact past configured retention.",
            retention_secs: retention_secs(lifecycle.execution_run_retention_days),
            risk: "low",
            selected_by_default: true,
        },
        ResidueRoot {
            relative: "automation_artifacts",
            category: "automation_residue",
            label: "Automation residue",
            reason: "Automation run artifact past configured retention.",
            retention_secs: retention_secs(lifecycle.automation_run_retention_days),
            risk: "low",
            selected_by_default: true,
        },
        ResidueRoot {
            relative: "terminal_artifacts",
            category: "automation_residue",
            label: "Automation residue",
            reason: "Terminal task artifact past configured retention.",
            retention_secs: retention_secs(lifecycle.terminal_task_retention_days),
            risk: "low",
            selected_by_default: true,
        },
        ResidueRoot {
            relative: "watcher_artifacts",
            category: "automation_residue",
            label: "Automation residue",
            reason: "Watcher artifact past configured automation retention.",
            retention_secs: retention_secs(lifecycle.automation_run_retention_days),
            risk: "low",
            selected_by_default: true,
        },
        ResidueRoot {
            relative: "execution_scratch",
            category: "automation_residue",
            label: "Automation residue",
            reason: "Execution scratch artifact past configured run retention.",
            retention_secs: retention_secs(lifecycle.execution_run_retention_days),
            risk: "low",
            selected_by_default: true,
        },
    ];

    for root in roots {
        let Some(retention_secs) = root.retention_secs else {
            continue;
        };
        collect_residue_root(data_root, root, retention_secs, candidates).await?;
    }
    Ok(())
}

async fn collect_agentark_temp_residue(
    data_root: &Path,
    candidates: &mut Vec<ArtifactCleanupCandidate>,
) -> Result<()> {
    let temp_root = std::env::temp_dir();
    let mut entries = match tokio::fs::read_dir(&temp_root).await {
        Ok(entries) => entries,
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => return Ok(()),
        Err(error) => return Err(error.into()),
    };
    while let Some(entry) = entries.next_entry().await? {
        let name = entry.file_name().to_string_lossy().to_string();
        if !name.starts_with("agentark-exec-") {
            continue;
        }
        let path = entry.path();
        let metadata = match entry.metadata().await {
            Ok(metadata) => metadata,
            Err(error) if error.kind() == std::io::ErrorKind::NotFound => continue,
            Err(error) => return Err(error.into()),
        };
        if !metadata.is_dir() || !older_than(metadata.modified().ok(), ARTIFACT_RETENTION_SECS) {
            continue;
        }
        let Some(source) = cleanup_temp_child_path(&path).await else {
            continue;
        };
        add_candidate(
            candidates,
            CandidateInput {
                data_root,
                source_path: source,
                category: "runtime_residue",
                category_label: "Runtime residue",
                risk: "low",
                reason: "Stale native code execution scratch directory in AgentArk temp space."
                    .to_string(),
                selected_by_default: true,
                app_id: None,
                requires_app_stop: false,
                age_reference: metadata.modified().ok().and_then(system_time_to_datetime),
            },
        )
        .await?;
    }

    let temp_uploads = temp_root.join("agentark").join("uploads");
    collect_temp_upload_root(data_root, &temp_uploads, candidates).await?;
    Ok(())
}

async fn collect_temp_upload_root(
    data_root: &Path,
    root: &Path,
    candidates: &mut Vec<ArtifactCleanupCandidate>,
) -> Result<()> {
    let mut entries = match tokio::fs::read_dir(root).await {
        Ok(entries) => entries,
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => return Ok(()),
        Err(error) => return Err(error.into()),
    };
    while let Some(entry) = entries.next_entry().await? {
        let metadata = match entry.metadata().await {
            Ok(metadata) => metadata,
            Err(error) if error.kind() == std::io::ErrorKind::NotFound => continue,
            Err(error) => return Err(error.into()),
        };
        if !metadata.is_file() || !older_than(metadata.modified().ok(), ARTIFACT_RETENTION_SECS) {
            continue;
        }
        let Some(source) = cleanup_temp_child_path(&entry.path()).await else {
            continue;
        };
        add_candidate(
            candidates,
            CandidateInput {
                data_root,
                source_path: source,
                category: "runtime_residue",
                category_label: "Runtime residue",
                risk: "medium",
                reason: "Stale temporary upload in AgentArk temp space.".to_string(),
                selected_by_default: false,
                app_id: None,
                requires_app_stop: false,
                age_reference: metadata.modified().ok().and_then(system_time_to_datetime),
            },
        )
        .await?;
    }
    Ok(())
}

#[derive(Clone, Copy)]
struct ResidueRoot {
    relative: &'static str,
    category: &'static str,
    label: &'static str,
    reason: &'static str,
    retention_secs: Option<u64>,
    risk: &'static str,
    selected_by_default: bool,
}

async fn collect_residue_root(
    data_root: &Path,
    root: ResidueRoot,
    retention_secs_value: u64,
    candidates: &mut Vec<ArtifactCleanupCandidate>,
) -> Result<()> {
    let root_path = data_root.join(root.relative);
    let mut entries = match tokio::fs::read_dir(&root_path).await {
        Ok(entries) => entries,
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => return Ok(()),
        Err(error) => return Err(error.into()),
    };
    while let Some(entry) = entries.next_entry().await? {
        let path = entry.path();
        let metadata = match entry.metadata().await {
            Ok(metadata) => metadata,
            Err(error) if error.kind() == std::io::ErrorKind::NotFound => continue,
            Err(error) => return Err(error.into()),
        };
        if !metadata.is_file() && !metadata.is_dir() {
            continue;
        }
        if !older_than(metadata.modified().ok(), retention_secs_value) {
            continue;
        }
        let Some(source) = cleanup_child_path(&path, data_root).await else {
            continue;
        };
        add_candidate(
            candidates,
            CandidateInput {
                data_root,
                source_path: source,
                category: root.category,
                category_label: root.label,
                risk: root.risk,
                reason: root.reason.to_string(),
                selected_by_default: root.selected_by_default,
                app_id: None,
                requires_app_stop: false,
                age_reference: metadata.modified().ok().and_then(system_time_to_datetime),
            },
        )
        .await?;
    }
    Ok(())
}

struct CandidateInput<'a> {
    data_root: &'a Path,
    source_path: PathBuf,
    category: &'static str,
    category_label: &'static str,
    risk: &'static str,
    reason: String,
    selected_by_default: bool,
    app_id: Option<String>,
    requires_app_stop: bool,
    age_reference: Option<DateTime<Utc>>,
}

async fn add_candidate(
    candidates: &mut Vec<ArtifactCleanupCandidate>,
    input: CandidateInput<'_>,
) -> Result<()> {
    let source = canonical_source_for_cleanup(&input.source_path, input.data_root).await?;
    ensure_cleanup_source_allowed(&source, input.data_root)?;
    let metadata = tokio::fs::symlink_metadata(&source).await?;
    if metadata.file_type().is_symlink() {
        return Ok(());
    }
    let size_bytes = path_size_bytes(source.clone()).await.unwrap_or(0);
    let age_seconds = input
        .age_reference
        .map(|timestamp| {
            let age = Utc::now() - timestamp;
            age.num_seconds().max(0) as u64
        })
        .or_else(|| {
            metadata
                .modified()
                .ok()
                .and_then(|modified| SystemTime::now().duration_since(modified).ok())
                .map(|duration| duration.as_secs())
        })
        .unwrap_or(0);
    let path_label = path_label(&source, input.data_root);
    let id = cleanup_candidate_id(input.category, &path_label, &input.reason);
    candidates.push(ArtifactCleanupCandidate {
        id,
        category: input.category.to_string(),
        category_label: input.category_label.to_string(),
        path_label,
        size_bytes,
        age_seconds,
        age_days: (age_seconds as f64) / 86_400.0,
        risk: input.risk.to_string(),
        reason: input.reason,
        selected_by_default: input.selected_by_default,
        app_id: input.app_id,
        requires_app_stop: input.requires_app_stop,
        source_path: source,
    });
    Ok(())
}

async fn canonical_source_for_cleanup(path: &Path, data_root: &Path) -> Result<PathBuf> {
    let source = tokio::fs::canonicalize(path).await?;
    ensure_cleanup_source_allowed(&source, data_root)?;
    Ok(source)
}

fn ensure_cleanup_source_allowed(source: &Path, data_root: &Path) -> Result<()> {
    if source == data_root {
        anyhow::bail!("refusing to clean data directory root");
    }
    if source.starts_with(data_root) {
        if protected_data_dir_child(source, data_root) {
            anyhow::bail!("refusing to clean protected data category");
        }
        return Ok(());
    }

    let temp_root = canonicalize_existing_sync(&std::env::temp_dir());
    if source.starts_with(&temp_root) {
        let file_name = source
            .file_name()
            .and_then(|value| value.to_str())
            .unwrap_or_default();
        let temp_agentark_root = canonicalize_existing_sync(&std::env::temp_dir().join("agentark"));
        if file_name.starts_with("agentark-exec-") || source.starts_with(temp_agentark_root) {
            return Ok(());
        }
    }
    anyhow::bail!("refusing to clean path outside AgentArk-owned roots");
}

async fn cleanup_child_path(path: &Path, data_root: &Path) -> Option<PathBuf> {
    let meta = tokio::fs::symlink_metadata(path).await.ok()?;
    if meta.file_type().is_symlink() {
        return None;
    }
    let canonical = tokio::fs::canonicalize(path).await.ok()?;
    if canonical == data_root || !canonical.starts_with(data_root) {
        return None;
    }
    if protected_data_dir_child(&canonical, data_root) {
        return None;
    }
    Some(canonical)
}

async fn cleanup_temp_child_path(path: &Path) -> Option<PathBuf> {
    let meta = tokio::fs::symlink_metadata(path).await.ok()?;
    if meta.file_type().is_symlink() {
        return None;
    }
    tokio::fs::canonicalize(path).await.ok()
}

fn protected_data_dir_child(path: &Path, data_root: &Path) -> bool {
    let Ok(relative) = path.strip_prefix(data_root) else {
        return false;
    };
    let Some(first) = relative.components().next() else {
        return true;
    };
    let first = first.as_os_str().to_string_lossy().to_ascii_lowercase();
    matches!(
        first.as_str(),
        "chats"
            | "conversations"
            | "messages"
            | "memory"
            | "memories"
            | "documents"
            | "document_store"
            | "knowledge"
            | "credentials"
            | "secrets"
            | "approvals"
            | "approval_log"
            | "approval_logs"
            | "auth"
            | "profiles"
            | "config"
            | "settings"
            | "uploads"
            | ARTIFACT_ARCHIVE_DIR
            | LEGACY_APP_ARCHIVE_DIR
    )
}

async fn path_is_dir_no_symlink(path: &Path) -> bool {
    match tokio::fs::symlink_metadata(path).await {
        Ok(meta) => meta.is_dir() && !meta.file_type().is_symlink(),
        Err(_) => false,
    }
}

fn display_app_name(app: &ManagedArtifactApp) -> String {
    if app.title.trim().is_empty() {
        app.id.clone()
    } else {
        format!("{} ({})", app.title.trim(), app.id)
    }
}

fn retention_secs(days: u64) -> Option<u64> {
    if days == 0 {
        None
    } else {
        Some(days.saturating_mul(24 * 60 * 60))
    }
}

fn older_than(modified: Option<SystemTime>, retention_secs_value: u64) -> bool {
    let Some(modified) = modified else {
        return false;
    };
    SystemTime::now()
        .duration_since(modified)
        .map(|age| age.as_secs() >= retention_secs_value)
        .unwrap_or(false)
}

fn modified_at_datetime(path: &Path) -> Option<DateTime<Utc>> {
    std::fs::metadata(path)
        .ok()
        .and_then(|metadata| metadata.modified().ok())
        .and_then(system_time_to_datetime)
}

fn system_time_to_datetime(time: SystemTime) -> Option<DateTime<Utc>> {
    let duration = time.duration_since(SystemTime::UNIX_EPOCH).ok()?;
    DateTime::<Utc>::from_timestamp(duration.as_secs() as i64, duration.subsec_nanos())
}

async fn path_size_bytes(path: PathBuf) -> Result<u64> {
    tokio::task::spawn_blocking(move || path_size_bytes_sync(&path))
        .await
        .context("path size worker failed")?
}

fn path_size_bytes_sync(path: &Path) -> Result<u64> {
    let metadata = std::fs::symlink_metadata(path)?;
    if metadata.file_type().is_symlink() {
        return Ok(0);
    }
    if metadata.is_file() {
        return Ok(metadata.len());
    }
    if !metadata.is_dir() {
        return Ok(0);
    }
    let mut total = 0u64;
    for entry in walkdir::WalkDir::new(path).follow_links(false) {
        let entry = match entry {
            Ok(entry) => entry,
            Err(_) => continue,
        };
        let Ok(metadata) = entry.metadata() else {
            continue;
        };
        if metadata.is_file() {
            total = total.saturating_add(metadata.len());
        }
    }
    Ok(total)
}

async fn move_path_to_archive(source: &Path, destination: &Path) -> Result<()> {
    match tokio::fs::rename(source, destination).await {
        Ok(_) => return Ok(()),
        Err(rename_error) => {
            copy_path_no_symlink(source.to_path_buf(), destination.to_path_buf()).await?;
            if source.is_dir() {
                tokio::fs::remove_dir_all(source).await.with_context(|| {
                    format!(
                        "failed to remove original after archive copy fallback: {}",
                        rename_error
                    )
                })?;
            } else {
                tokio::fs::remove_file(source).await.with_context(|| {
                    format!(
                        "failed to remove original after archive copy fallback: {}",
                        rename_error
                    )
                })?;
            }
        }
    }
    Ok(())
}

async fn copy_path_no_symlink(source: PathBuf, destination: PathBuf) -> Result<()> {
    tokio::task::spawn_blocking(move || copy_path_no_symlink_sync(&source, &destination))
        .await
        .context("archive copy worker failed")?
}

fn copy_path_no_symlink_sync(source: &Path, destination: &Path) -> Result<()> {
    let metadata = std::fs::symlink_metadata(source)?;
    if metadata.file_type().is_symlink() {
        anyhow::bail!("refusing to copy symlink into archive");
    }
    if metadata.is_file() {
        if let Some(parent) = destination.parent() {
            std::fs::create_dir_all(parent)?;
        }
        std::fs::copy(source, destination)?;
        return Ok(());
    }
    std::fs::create_dir_all(destination)?;
    for entry in walkdir::WalkDir::new(source).follow_links(false) {
        let entry = entry?;
        let path = entry.path();
        let rel = path.strip_prefix(source)?;
        if rel.as_os_str().is_empty() {
            continue;
        }
        let dest = destination.join(rel);
        let metadata = std::fs::symlink_metadata(path)?;
        if metadata.file_type().is_symlink() {
            continue;
        }
        if metadata.is_dir() {
            std::fs::create_dir_all(&dest)?;
        } else if metadata.is_file() {
            if let Some(parent) = dest.parent() {
                std::fs::create_dir_all(parent)?;
            }
            std::fs::copy(path, &dest)?;
        }
    }
    Ok(())
}

fn archive_entry_name(candidate: &ArtifactCleanupCandidate) -> String {
    let timestamp = Utc::now().format("%Y%m%dT%H%M%SZ");
    let basename = candidate
        .source_path
        .file_name()
        .and_then(|value| value.to_str())
        .map(sanitize_path_segment)
        .filter(|value| !value.is_empty())
        .unwrap_or_else(|| "artifact".to_string());
    format!(
        "{}-{}-{}",
        timestamp,
        &candidate.id.chars().take(16).collect::<String>(),
        basename
    )
}

fn cleanup_candidate_id(category: &str, path_label: &str, reason: &str) -> String {
    let mut hasher = sha2::Sha256::new();
    hasher.update(category.as_bytes());
    hasher.update(b"\0");
    hasher.update(path_label.as_bytes());
    hasher.update(b"\0");
    hasher.update(reason.as_bytes());
    let digest = hex::encode(hasher.finalize());
    format!("artifact-{}", &digest[..24])
}

fn sanitize_path_segment(input: &str) -> String {
    let mut out = String::with_capacity(input.len().min(96));
    for ch in input.chars() {
        if ch.is_ascii_alphanumeric() || ch == '-' || ch == '_' || ch == '.' {
            out.push(ch);
        } else {
            out.push('_');
        }
        if out.len() >= 96 {
            break;
        }
    }
    out.trim_matches('.').trim_matches('_').to_string()
}

fn path_label(path: &Path, data_root: &Path) -> String {
    if let Ok(relative) = path.strip_prefix(data_root) {
        let rel = relative.to_string_lossy().replace('\\', "/");
        if rel.is_empty() {
            return "data_dir".to_string();
        }
        return format!("data_dir/{}", rel);
    }
    let temp_root = canonicalize_existing_sync(&std::env::temp_dir());
    if let Ok(relative) = path.strip_prefix(&temp_root) {
        let rel = relative.to_string_lossy().replace('\\', "/");
        return format!("temp/{}", rel);
    }
    path.display().to_string()
}

fn canonicalize_existing_sync(path: &Path) -> PathBuf {
    std::fs::canonicalize(path).unwrap_or_else(|_| path.to_path_buf())
}

async fn prune_archive_root(root: &Path, summary: &mut ArchiveRetentionSummary) -> Result<()> {
    let mut candidates = Vec::new();
    collect_archive_entries(root, root, &mut candidates).await?;
    let cutoff = SystemTime::now()
        .checked_sub(StdDuration::from_secs(
            ARCHIVE_RETENTION_DAYS as u64 * 24 * 60 * 60,
        ))
        .unwrap_or(SystemTime::UNIX_EPOCH);

    for entry in candidates {
        let metadata = match tokio::fs::symlink_metadata(&entry).await {
            Ok(metadata) => metadata,
            Err(error) if error.kind() == std::io::ErrorKind::NotFound => continue,
            Err(error) => return Err(error.into()),
        };
        if metadata.file_type().is_symlink() {
            continue;
        }
        let archived_at = archive_entry_time(&entry, &metadata).await;
        if archived_at > cutoff {
            continue;
        }
        let size = path_size_bytes(entry.clone()).await.unwrap_or(0);
        let canonical = match tokio::fs::canonicalize(&entry).await {
            Ok(path) => path,
            Err(error) if error.kind() == std::io::ErrorKind::NotFound => continue,
            Err(error) => return Err(error.into()),
        };
        if !canonical.starts_with(root) || canonical == root {
            continue;
        }
        if metadata.is_dir() {
            tokio::fs::remove_dir_all(&entry).await?;
        } else {
            tokio::fs::remove_file(&entry).await?;
        }
        summary.deleted_entries = summary.deleted_entries.saturating_add(1);
        summary.deleted_bytes = summary.deleted_bytes.saturating_add(size);
    }
    remove_empty_archive_category_dirs(root).await?;
    Ok(())
}

async fn collect_archive_entries(root: &Path, dir: &Path, out: &mut Vec<PathBuf>) -> Result<()> {
    let mut pending = vec![dir.to_path_buf()];
    while let Some(current_dir) = pending.pop() {
        let mut entries = match tokio::fs::read_dir(&current_dir).await {
            Ok(entries) => entries,
            Err(error) if error.kind() == std::io::ErrorKind::NotFound => continue,
            Err(error) => return Err(error.into()),
        };
        while let Some(entry) = entries.next_entry().await? {
            let path = entry.path();
            let metadata = match tokio::fs::symlink_metadata(&path).await {
                Ok(metadata) => metadata,
                Err(error) if error.kind() == std::io::ErrorKind::NotFound => continue,
                Err(error) => return Err(error.into()),
            };
            if metadata.file_type().is_symlink() {
                continue;
            }
            if !metadata.is_dir() {
                out.push(path);
                continue;
            }
            if path.join(ARCHIVE_MANIFEST_FILE).is_file() {
                out.push(path);
                continue;
            }
            if path.parent() == Some(root) {
                pending.push(path);
            } else {
                out.push(path);
            }
        }
    }
    Ok(())
}

async fn archive_entry_time(path: &Path, metadata: &std::fs::Metadata) -> SystemTime {
    if let Ok(raw) = tokio::fs::read(path.join(ARCHIVE_MANIFEST_FILE)).await {
        if let Ok(manifest) = serde_json::from_slice::<ArchivedArtifactManifest>(&raw) {
            if let Ok(timestamp) = DateTime::parse_from_rfc3339(&manifest.archived_at) {
                return datetime_to_system_time(timestamp.with_timezone(&Utc));
            }
        }
    }
    if let Some(from_name) = archive_time_from_name(path) {
        return from_name;
    }
    metadata.modified().unwrap_or(SystemTime::now())
}

fn archive_time_from_name(path: &Path) -> Option<SystemTime> {
    let name = path.file_name()?.to_str()?;
    let stamp = name.split('-').next()?;
    let parsed = chrono::NaiveDateTime::parse_from_str(stamp, "%Y%m%dT%H%M%SZ").ok()?;
    Some(datetime_to_system_time(
        DateTime::<Utc>::from_naive_utc_and_offset(parsed, Utc),
    ))
}

fn datetime_to_system_time(timestamp: DateTime<Utc>) -> SystemTime {
    let seconds = timestamp.timestamp();
    if seconds >= 0 {
        SystemTime::UNIX_EPOCH + StdDuration::from_secs(seconds as u64)
    } else {
        SystemTime::UNIX_EPOCH
    }
}

async fn remove_empty_archive_category_dirs(root: &Path) -> Result<()> {
    let mut entries = match tokio::fs::read_dir(root).await {
        Ok(entries) => entries,
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => return Ok(()),
        Err(error) => return Err(error.into()),
    };
    while let Some(entry) = entries.next_entry().await? {
        let path = entry.path();
        let metadata = match entry.metadata().await {
            Ok(metadata) => metadata,
            Err(error) if error.kind() == std::io::ErrorKind::NotFound => continue,
            Err(error) => return Err(error.into()),
        };
        if !metadata.is_dir() || path.join(ARCHIVE_MANIFEST_FILE).is_file() {
            continue;
        }
        let mut nested = tokio::fs::read_dir(&path).await?;
        if nested.next_entry().await?.is_none() {
            let _ = tokio::fs::remove_dir(&path).await;
        }
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::fs;
    use std::io::Write;

    fn old_datetime() -> DateTime<Utc> {
        Utc::now() - chrono::Duration::days(30)
    }

    #[tokio::test]
    async fn protected_user_data_roots_never_become_cleanup_candidates() {
        let temp = tempfile::tempdir().unwrap();
        let data_dir = temp.path();
        for protected in [
            "memory",
            "memories",
            "documents",
            "credentials",
            "approval_logs",
            "uploads",
            "settings",
        ] {
            let path = data_dir.join(protected).join("old-runtime-looking-file");
            fs::create_dir_all(path.parent().unwrap()).unwrap();
            fs::write(path, b"keep").unwrap();
        }
        let candidates = collect_artifact_cleanup_candidates(
            data_dir,
            &[],
            &HashMap::new(),
            &DataLifecycleSettings::default(),
        )
        .await
        .unwrap();
        assert!(candidates.is_empty());
    }

    #[tokio::test]
    async fn orphan_app_dir_is_a_managed_app_candidate() {
        let temp = tempfile::tempdir().unwrap();
        let app_dir = temp.path().join("apps").join("orphan-app");
        fs::create_dir_all(&app_dir).unwrap();
        fs::write(app_dir.join(".app_meta.json"), br#"{"title":"Old"}"#).unwrap();
        let candidates = collect_artifact_cleanup_candidates(
            temp.path(),
            &[],
            &HashMap::new(),
            &DataLifecycleSettings::default(),
        )
        .await
        .unwrap();
        assert_eq!(candidates.len(), 1);
        assert_eq!(candidates[0].category, "managed_apps");
        assert!(candidates[0].reason.contains("Orphan app directory"));
    }

    #[tokio::test]
    async fn archive_candidate_writes_manifest_under_artifact_archive() {
        let temp = tempfile::tempdir().unwrap();
        let source = temp.path().join("outputs").join("old-run");
        fs::create_dir_all(&source).unwrap();
        fs::write(source.join("result.txt"), b"done").unwrap();
        let candidate = ArtifactCleanupCandidate {
            id: "artifact-test".to_string(),
            category: "runtime_residue".to_string(),
            category_label: "Runtime residue".to_string(),
            path_label: "data_dir/outputs/old-run".to_string(),
            size_bytes: 4,
            age_seconds: 30 * 24 * 60 * 60,
            age_days: 30.0,
            risk: "low".to_string(),
            reason: "test archive".to_string(),
            selected_by_default: true,
            app_id: None,
            requires_app_stop: false,
            source_path: source.clone(),
        };
        let outcome =
            archive_cleanup_candidate(temp.path(), &candidate, Some("event".into()), Some(2))
                .await
                .unwrap();
        assert!(
            outcome
                .archive_path_label
                .starts_with("data_dir/artifact_archive/")
        );
        assert!(!source.exists());
        let archive_root = temp
            .path()
            .join(ARTIFACT_ARCHIVE_DIR)
            .join("runtime_residue");
        let entry = fs::read_dir(archive_root)
            .unwrap()
            .next()
            .unwrap()
            .unwrap()
            .path();
        let manifest: ArchivedArtifactManifest =
            serde_json::from_slice(&fs::read(entry.join(ARCHIVE_MANIFEST_FILE)).unwrap()).unwrap();
        assert_eq!(manifest.category, "runtime_residue");
        assert_eq!(manifest.finding_index, Some(2));
    }

    #[tokio::test]
    async fn archive_retention_prunes_artifact_and_legacy_roots() {
        let temp = tempfile::tempdir().unwrap();
        for root in [ARTIFACT_ARCHIVE_DIR, LEGACY_APP_ARCHIVE_DIR] {
            let entry = temp
                .path()
                .join(root)
                .join("managed_apps")
                .join("old-entry");
            fs::create_dir_all(&entry).unwrap();
            fs::write(entry.join("payload.txt"), b"old").unwrap();
            let manifest = ArchivedArtifactManifest {
                source_path: "source".to_string(),
                source_path_label: "data_dir/apps/old".to_string(),
                category: "managed_apps".to_string(),
                archived_at: old_datetime().to_rfc3339(),
                size_bytes: 3,
                reason: "old".to_string(),
                event_timestamp: None,
                finding_index: None,
            };
            let mut file = fs::File::create(entry.join(ARCHIVE_MANIFEST_FILE)).unwrap();
            file.write_all(&serde_json::to_vec(&manifest).unwrap())
                .unwrap();
        }
        let summary = prune_archive_retention(temp.path()).await.unwrap();
        assert_eq!(summary.deleted_entries, 2);
        assert!(
            !temp
                .path()
                .join(ARTIFACT_ARCHIVE_DIR)
                .join("managed_apps")
                .join("old-entry")
                .exists()
        );
        assert!(
            !temp
                .path()
                .join(LEGACY_APP_ARCHIVE_DIR)
                .join("managed_apps")
                .join("old-entry")
                .exists()
        );
    }
}
