use super::*;
use std::collections::BTreeMap;
use std::hash::{Hash, Hasher};

const CURATOR_USAGE_FILE: &str = ".usage.json";
const CURATOR_ARCHIVE_DIR: &str = ".archive";
const CURATOR_REVIEW_QUEUE_DIR: &str = ".review_queue";
const CURATOR_INTERVAL_HOURS: i64 = 12;
const CURATOR_IDLE_MINUTES: i64 = 30;
const CURATOR_STALE_AFTER_DAYS: i64 = 14;
const CURATOR_ARCHIVE_AFTER_DAYS: i64 = 30;

#[derive(Debug, Clone, Serialize, Deserialize)]
struct SkillUsageRecord {
    #[serde(default)]
    use_count: u64,
    #[serde(default)]
    view_count: u64,
    #[serde(default)]
    patch_count: u64,
    #[serde(default)]
    last_activity_at: Option<String>,
    #[serde(default = "default_skill_state")]
    state: String,
    #[serde(default)]
    pinned: bool,
    #[serde(default = "default_created_by")]
    created_by: String,
}

fn default_skill_state() -> String {
    "active".to_string()
}

fn default_created_by() -> String {
    "user".to_string()
}

#[derive(Debug, Clone, Default, Serialize, Deserialize)]
struct SkillUsageFile {
    #[serde(default)]
    skills: BTreeMap<String, SkillUsageRecord>,
    #[serde(default)]
    updated_at: Option<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
struct SkillCrystallizationDraft {
    id: String,
    intent_key: String,
    tool_sequence_digest: String,
    occurrence_count: usize,
    example_run_ids: Vec<String>,
    created_at: String,
    status: String,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct CuratorRunReport {
    pub checked: usize,
    pub stale_marked: usize,
    pub archived: usize,
    pub drafts_created: usize,
    pub usage_path: String,
}

impl Agent {
    pub(crate) fn spawn_curator_idle_worker(&self) {
        let agent = self.clone();
        crate::spawn_logged!("src/core/agent/curator.rs:idle_worker", async move {
            loop {
                tokio::time::sleep(std::time::Duration::from_secs(
                    (CURATOR_INTERVAL_HOURS.max(1) as u64) * 60 * 60,
                ))
                .await;
                if !agent.curator_idle_window_elapsed().await {
                    continue;
                }
                if agent.curator_is_paused().await {
                    continue;
                }
                if let Err(error) = agent.run_skill_curator_once().await {
                    tracing::warn!("Skill curator run failed: {}", error);
                }
            }
            #[allow(unreachable_code)]
            Ok::<(), anyhow::Error>(())
        });
    }

    async fn curator_idle_window_elapsed(&self) -> bool {
        let last_activity = self.last_activity.read().await.clone();
        let Some(last_activity) = last_activity else {
            return true;
        };
        chrono::Utc::now().signed_duration_since(last_activity)
            >= chrono::Duration::minutes(CURATOR_IDLE_MINUTES)
    }

    async fn curator_is_paused(&self) -> bool {
        tokio::fs::metadata(self.data_dir.join("skills").join(".curator_paused"))
            .await
            .is_ok()
    }

    pub(crate) async fn run_skill_curator_once(&self) -> Result<CuratorRunReport> {
        let skills_dir = self.data_dir.join("skills");
        let archive_dir = skills_dir.join(CURATOR_ARCHIVE_DIR);
        let review_dir = skills_dir.join(CURATOR_REVIEW_QUEUE_DIR);
        tokio::fs::create_dir_all(&skills_dir).await?;
        tokio::fs::create_dir_all(&archive_dir).await?;
        tokio::fs::create_dir_all(&review_dir).await?;

        let usage_path = skills_dir.join(CURATOR_USAGE_FILE);
        let mut usage = load_skill_usage_file(&usage_path).await.unwrap_or_default();
        let now = chrono::Utc::now();
        let skill_names = list_skill_names(&skills_dir).await?;
        for name in &skill_names {
            usage
                .skills
                .entry(name.clone())
                .or_insert_with(|| SkillUsageRecord {
                    use_count: 0,
                    view_count: 0,
                    patch_count: 0,
                    last_activity_at: Some(now.to_rfc3339()),
                    state: default_skill_state(),
                    pinned: false,
                    created_by: default_created_by(),
                });
        }

        let mut stale_marked = 0usize;
        let mut archived = 0usize;
        for (name, record) in usage.skills.iter_mut() {
            if record.pinned || record.created_by != "agent" {
                continue;
            }
            let last_activity = parse_rfc3339_utc(record.last_activity_at.as_deref())
                .unwrap_or_else(|| now.clone());
            let age = now.signed_duration_since(last_activity);
            if record.state == "active" && age >= chrono::Duration::days(CURATOR_STALE_AFTER_DAYS) {
                record.state = "stale".to_string();
                stale_marked += 1;
            }
            if record.state == "stale" && age >= chrono::Duration::days(CURATOR_ARCHIVE_AFTER_DAYS)
            {
                if archive_skill_dir(&skills_dir, &archive_dir, name).await? {
                    record.state = "archived".to_string();
                    archived += 1;
                }
            }
        }

        let drafts_created = self
            .draft_recurring_skill_patterns(&review_dir)
            .await
            .unwrap_or_else(|error| {
                tracing::warn!("Skill curator pattern review failed: {}", error);
                0
            });
        usage.updated_at = Some(now.to_rfc3339());
        save_skill_usage_file(&usage_path, &usage).await?;

        Ok(CuratorRunReport {
            checked: skill_names.len(),
            stale_marked,
            archived,
            drafts_created,
            usage_path: usage_path.display().to_string(),
        })
    }

    async fn draft_recurring_skill_patterns(&self, review_dir: &std::path::Path) -> Result<usize> {
        let recent_runs = self
            .storage
            .list_recent_experience_runs_any_scope(240)
            .await?;
        let mut grouped: BTreeMap<(String, String), Vec<String>> = BTreeMap::new();
        for run in recent_runs {
            let success_state = run.success_state.trim();
            if !matches!(success_state, "accepted" | "completed" | "provisional") {
                continue;
            }
            let intent_key = run.intent_key.trim();
            let digest = run
                .tool_sequence_digest
                .as_deref()
                .map(str::trim)
                .unwrap_or_default();
            if intent_key.is_empty() || digest.is_empty() {
                continue;
            }
            grouped
                .entry((intent_key.to_string(), digest.to_string()))
                .or_default()
                .push(run.id);
        }

        let mut created = 0usize;
        for ((intent_key, digest), run_ids) in grouped {
            if run_ids.len() < 3 {
                continue;
            }
            let draft_id = stable_curator_draft_id(&intent_key, &digest);
            let draft_path = review_dir.join(format!("{draft_id}.json"));
            if tokio::fs::metadata(&draft_path).await.is_ok() {
                continue;
            }
            let draft = SkillCrystallizationDraft {
                id: draft_id,
                intent_key,
                tool_sequence_digest: digest,
                occurrence_count: run_ids.len(),
                example_run_ids: run_ids.into_iter().take(8).collect(),
                created_at: chrono::Utc::now().to_rfc3339(),
                status: "needs_review".to_string(),
            };
            let body = serde_json::to_vec_pretty(&draft)?;
            tokio::fs::write(draft_path, body).await?;
            created += 1;
        }
        Ok(created)
    }
}

async fn load_skill_usage_file(path: &std::path::Path) -> Result<SkillUsageFile> {
    let bytes = tokio::fs::read(path).await?;
    Ok(serde_json::from_slice(&bytes)?)
}

async fn save_skill_usage_file(path: &std::path::Path, usage: &SkillUsageFile) -> Result<()> {
    let body = serde_json::to_vec_pretty(usage)?;
    tokio::fs::write(path, body).await?;
    Ok(())
}

async fn list_skill_names(skills_dir: &std::path::Path) -> Result<Vec<String>> {
    let mut names = Vec::new();
    let mut entries = tokio::fs::read_dir(skills_dir).await?;
    while let Some(entry) = entries.next_entry().await? {
        let path = entry.path();
        if !entry.file_type().await?.is_dir() {
            continue;
        }
        let Some(name) = path.file_name().and_then(|value| value.to_str()) else {
            continue;
        };
        if name.starts_with('.') {
            continue;
        }
        if tokio::fs::metadata(path.join("SKILL.md")).await.is_ok() {
            names.push(name.to_string());
        }
    }
    names.sort();
    Ok(names)
}

async fn archive_skill_dir(
    skills_dir: &std::path::Path,
    archive_dir: &std::path::Path,
    name: &str,
) -> Result<bool> {
    let source = skills_dir.join(name);
    if tokio::fs::metadata(source.join("SKILL.md")).await.is_err() {
        return Ok(false);
    }
    let target = archive_dir.join(format!(
        "{}-{}",
        name,
        chrono::Utc::now().format("%Y%m%d%H%M%S")
    ));
    tokio::fs::rename(source, target).await?;
    Ok(true)
}

fn parse_rfc3339_utc(value: Option<&str>) -> Option<chrono::DateTime<chrono::Utc>> {
    chrono::DateTime::parse_from_rfc3339(value?)
        .ok()
        .map(|dt| dt.with_timezone(&chrono::Utc))
}

fn stable_curator_draft_id(intent_key: &str, digest: &str) -> String {
    let mut hasher = std::collections::hash_map::DefaultHasher::new();
    intent_key.hash(&mut hasher);
    digest.hash(&mut hasher);
    format!("skill-draft-{:016x}", hasher.finish())
}
