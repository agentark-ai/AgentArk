use std::collections::{BTreeMap, BTreeSet, HashMap, HashSet};
use std::path::{Path, PathBuf};

use anyhow::{Context, Result};
use serde::{Deserialize, Serialize};

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct SkillCatalogEntry {
    pub name: String,
    pub description: String,
    pub source: String,
    pub path: PathBuf,
    pub content: String,
    pub history_dir: PathBuf,
}

#[derive(Debug, Clone, Serialize, Deserialize, Default, PartialEq)]
pub struct SkillMetricsSnapshot {
    pub samples: usize,
    pub matched_runs: usize,
    pub total_window_runs: usize,
    pub successful_runs: usize,
    pub failed_runs: usize,
    pub corrected_runs: usize,
    pub success_rate: f64,
    pub failure_rate: f64,
    pub corrected_rate: f64,
    pub tool_error_rate: f64,
    pub avg_tool_calls: f64,
    pub selection_rate: f64,
}

#[derive(Debug, Clone, Serialize, Deserialize, Default, PartialEq)]
pub struct SkillImpactAssessment {
    pub status: String,
    pub summary: Vec<String>,
    pub success_gain: f64,
    pub failure_reduction: f64,
    pub tool_error_reduction: f64,
}

#[derive(Debug, Clone, Serialize, Deserialize, Default, PartialEq)]
pub struct SkillHistoryEntry {
    pub version: u32,
    pub snapshot_path: Option<PathBuf>,
    pub evidence_path: PathBuf,
    pub snapshot_content: Option<String>,
    pub evidence_content: String,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum SkillWindowDirection {
    Baseline,
    Observed,
}

fn round4(value: f64) -> f64 {
    (value * 10_000.0).round() / 10_000.0
}

fn parse_frontmatter(raw: &str) -> Option<(&str, &str)> {
    let stripped = raw.strip_prefix("---")?;
    let rest = stripped
        .strip_prefix("\r\n")
        .or_else(|| stripped.strip_prefix('\n'))?;
    let end = rest.find("\n---").or_else(|| rest.find("\r\n---"))?;
    let frontmatter = &rest[..end];
    let body = &rest[end + 4..];
    Some((frontmatter, body))
}

pub fn extract_frontmatter_value(raw: &str, key: &str) -> Option<String> {
    let (frontmatter, _) = parse_frontmatter(raw)?;
    frontmatter.lines().find_map(|line| {
        let trimmed = line.trim();
        if !trimmed.starts_with(key) {
            return None;
        }
        let remainder = trimmed.strip_prefix(key)?.trim_start();
        let remainder = remainder.strip_prefix(':')?.trim();
        Some(remainder.trim_matches('"').trim_matches('\'').to_string())
    })
}

pub fn canonicalize_skill_name(raw: &str) -> String {
    let trimmed = raw.trim();
    if trimmed.is_empty() {
        return String::new();
    }
    let compact = trimmed
        .chars()
        .filter(|ch| ch.is_ascii_alphanumeric())
        .collect::<String>()
        .to_ascii_lowercase();
    if compact == "trendprophet" {
        return "trend-prophet".to_string();
    }
    trimmed.to_string()
}

fn canonical_skill_key(raw: &str) -> String {
    canonicalize_skill_name(raw).to_ascii_lowercase()
}

fn yaml_quote(value: &str) -> String {
    format!("\"{}\"", value.replace('\\', "\\\\").replace('"', "\\\""))
}

pub fn replace_frontmatter_value(raw: &str, key: &str, value: &str) -> String {
    let Some((frontmatter, body)) = parse_frontmatter(raw) else {
        return raw.to_string();
    };
    let mut replaced = false;
    let mut new_lines = Vec::new();
    for line in frontmatter.lines() {
        let trimmed = line.trim_start();
        if trimmed.starts_with(&format!("{key}:")) {
            let indent = line.len().saturating_sub(trimmed.len());
            new_lines.push(format!(
                "{}{}: {}",
                " ".repeat(indent),
                key,
                yaml_quote(value)
            ));
            replaced = true;
        } else {
            new_lines.push(line.to_string());
        }
    }
    if !replaced {
        new_lines.push(format!("{key}: {}", yaml_quote(value)));
    }
    let body = body.trim_start_matches(['\r', '\n']);
    format!("---\n{}\n---\n\n{}", new_lines.join("\n"), body)
}

pub fn append_not_for_clause(description: &str, exclusions: &[String]) -> Option<String> {
    if exclusions.is_empty() {
        return None;
    }
    if description.to_ascii_lowercase().contains("not for:") {
        return None;
    }
    let mut cleaned = exclusions
        .iter()
        .map(|value| value.trim().to_string())
        .filter(|value| !value.is_empty())
        .collect::<Vec<_>>();
    cleaned.sort();
    cleaned.dedup();
    if cleaned.is_empty() {
        return None;
    }
    Some(format!(
        "{} NOT for: {}.",
        description.trim().trim_end_matches('.'),
        cleaned.join(", ")
    ))
}

pub fn upsert_markdown_section(content: &str, heading: &str, section_body: &str) -> String {
    let section_body = section_body.trim();
    if section_body.is_empty() {
        return content.to_string();
    }
    let new_section = format!("{heading}\n\n{section_body}\n");
    let lines = content.lines().collect::<Vec<_>>();
    let heading_trimmed = heading.trim();
    let start = lines.iter().position(|line| line.trim() == heading_trimmed);
    if let Some(start_idx) = start {
        let mut end_idx = lines.len();
        for (idx, line) in lines.iter().enumerate().skip(start_idx + 1) {
            if line.trim_start().starts_with("## ") {
                end_idx = idx;
                break;
            }
        }
        let mut out = String::new();
        for line in &lines[..start_idx] {
            out.push_str(line);
            out.push('\n');
        }
        out.push_str(&new_section);
        if end_idx < lines.len() {
            if !out.ends_with('\n') {
                out.push('\n');
            }
            for line in &lines[end_idx..] {
                out.push_str(line);
                out.push('\n');
            }
        }
        return out.trim_end().to_string() + "\n";
    }

    let mut out = content.trim_end().to_string();
    if !out.is_empty() {
        out.push_str("\n\n");
    }
    out.push_str(&new_section);
    out
}

pub fn build_skill_diff_preview(before: &str, after: &str) -> serde_json::Value {
    let before_lines = before.lines().map(str::trim).collect::<BTreeSet<_>>();
    let after_lines = after.lines().map(str::trim).collect::<BTreeSet<_>>();
    let added = after
        .lines()
        .map(str::trim)
        .filter(|line| !line.is_empty() && !before_lines.contains(line))
        .take(8)
        .map(|line| line.to_string())
        .collect::<Vec<_>>();
    let removed = before
        .lines()
        .map(str::trim)
        .filter(|line| !line.is_empty() && !after_lines.contains(line))
        .take(8)
        .map(|line| line.to_string())
        .collect::<Vec<_>>();
    let headings = after
        .lines()
        .map(str::trim)
        .filter(|line| line.starts_with('#'))
        .take(8)
        .map(|line| line.to_string())
        .collect::<Vec<_>>();
    serde_json::json!({
        "added": added,
        "removed": removed,
        "headings": headings,
        "before_preview": before.lines().take(8).collect::<Vec<_>>(),
        "after_preview": after.lines().take(8).collect::<Vec<_>>(),
    })
}

pub fn load_skill_catalog(data_dir: &Path) -> Result<Vec<SkillCatalogEntry>> {
    let mut catalog = BTreeMap::<String, SkillCatalogEntry>::new();
    for (root, source) in [(data_dir.join("skills"), "custom")] {
        if !root.exists() {
            continue;
        }
        for entry in std::fs::read_dir(&root)
            .with_context(|| format!("failed reading skill dir {:?}", root))?
            .flatten()
        {
            let path = entry.path();
            if !path.is_dir() {
                continue;
            }
            let skill_path = path.join("SKILL.md");
            if !skill_path.exists() {
                continue;
            }
            let content = std::fs::read_to_string(&skill_path)
                .with_context(|| format!("failed reading {:?}", skill_path))?;
            let name = canonicalize_skill_name(
                &extract_frontmatter_value(&content, "name")
                    .filter(|value| !value.trim().is_empty())
                    .unwrap_or_else(|| {
                        path.file_name()
                            .and_then(|value| value.to_str())
                            .unwrap_or("skill")
                            .to_string()
                    }),
            );
            let description =
                extract_frontmatter_value(&content, "description").unwrap_or_default();
            catalog.insert(
                name.clone(),
                SkillCatalogEntry {
                    name,
                    description,
                    source: source.to_string(),
                    path: skill_path,
                    content,
                    history_dir: path.join("history"),
                },
            );
        }
    }
    Ok(catalog.into_values().collect())
}

fn selected_actions_for_run(
    run: &crate::storage::entities::experience_run::Model,
) -> HashSet<String> {
    let mut out = HashSet::new();
    let sources = [
        run.metadata
            .get("decision_episode")
            .and_then(|value| value.get("action_selection"))
            .and_then(|value| value.get("payload"))
            .and_then(|value| value.get("needed_actions")),
        run.metadata
            .get("decision_episode")
            .and_then(|value| value.get("request_shape"))
            .and_then(|value| value.get("payload"))
            .and_then(|value| value.get("preferred_actions")),
    ];
    for source in sources.into_iter().flatten() {
        if let Some(items) = source.as_array() {
            for item in items {
                if let Some(name) = item.as_str() {
                    out.insert(canonical_skill_key(name));
                }
            }
        }
    }
    out
}

fn executed_actions_for_run(
    run: &crate::storage::entities::experience_run::Model,
) -> HashSet<String> {
    run.tool_sequence_json
        .as_array()
        .map(|items| {
            items
                .iter()
                .filter_map(|item| item.get("tool_name").and_then(|value| value.as_str()))
                .map(canonical_skill_key)
                .collect::<HashSet<_>>()
        })
        .unwrap_or_default()
}

pub fn skill_selected_by_run(
    run: &crate::storage::entities::experience_run::Model,
    skill_name: &str,
) -> bool {
    let needle = canonical_skill_key(skill_name);
    selected_actions_for_run(run).contains(&needle)
}

pub fn skill_executed_by_run(
    run: &crate::storage::entities::experience_run::Model,
    skill_name: &str,
) -> bool {
    let needle = canonical_skill_key(skill_name);
    executed_actions_for_run(run).contains(&needle)
}

pub fn skill_matches_run(
    run: &crate::storage::entities::experience_run::Model,
    skill_name: &str,
) -> bool {
    skill_selected_by_run(run, skill_name) || skill_executed_by_run(run, skill_name)
}

fn run_is_resolved(run: &crate::storage::entities::experience_run::Model) -> bool {
    run.correction_state == "corrected"
        || run.success_state == "accepted"
        || run.success_state == "failed"
}

fn run_is_success(run: &crate::storage::entities::experience_run::Model) -> bool {
    run.correction_state != "corrected" && run.success_state == "accepted"
}

fn run_is_failure(run: &crate::storage::entities::experience_run::Model) -> bool {
    run.correction_state == "corrected" || run.success_state == "failed"
}

fn compare_rfc3339(left: &str, right: &str) -> std::cmp::Ordering {
    match (
        chrono::DateTime::parse_from_rfc3339(left),
        chrono::DateTime::parse_from_rfc3339(right),
    ) {
        (Ok(l), Ok(r)) => l.cmp(&r),
        _ => left.cmp(right),
    }
}

pub fn compute_skill_metrics(
    runs: &[crate::storage::entities::experience_run::Model],
    skill_name: &str,
    anchor_time: Option<&str>,
    direction: SkillWindowDirection,
    sample_limit: usize,
    overall_window_limit: usize,
) -> SkillMetricsSnapshot {
    let mut filtered = runs
        .iter()
        .filter(|run| {
            let created_at = run.created_at.as_str();
            match (anchor_time, direction) {
                (Some(anchor), SkillWindowDirection::Baseline) => {
                    compare_rfc3339(created_at, anchor).is_lt()
                }
                (Some(anchor), SkillWindowDirection::Observed) => {
                    compare_rfc3339(created_at, anchor).is_gt()
                }
                (None, _) => true,
            }
        })
        .collect::<Vec<_>>();
    filtered.sort_by(|a, b| match direction {
        SkillWindowDirection::Baseline => compare_rfc3339(&b.created_at, &a.created_at),
        SkillWindowDirection::Observed => compare_rfc3339(&a.created_at, &b.created_at),
    });
    let window = filtered
        .into_iter()
        .take(overall_window_limit.max(sample_limit))
        .collect::<Vec<_>>();
    let matched = window
        .iter()
        .copied()
        .filter(|run| skill_matches_run(run, skill_name))
        .take(sample_limit)
        .collect::<Vec<_>>();
    let resolved = matched
        .iter()
        .copied()
        .filter(|run| run_is_resolved(run))
        .collect::<Vec<_>>();
    let successful_runs = resolved.iter().filter(|run| run_is_success(run)).count();
    let failed_runs = resolved.iter().filter(|run| run_is_failure(run)).count();
    let corrected_runs = resolved
        .iter()
        .filter(|run| run.correction_state == "corrected")
        .count();
    let tool_attempts = matched
        .iter()
        .flat_map(|run| run.tool_sequence_json.as_array().into_iter().flatten())
        .collect::<Vec<_>>();
    let tool_error_count = tool_attempts
        .iter()
        .filter(|item| {
            item.get("status")
                .and_then(|value| value.as_str())
                .map(|status| status != "success")
                .unwrap_or(false)
        })
        .count();
    let avg_tool_calls = if matched.is_empty() {
        0.0
    } else {
        matched
            .iter()
            .map(|run| {
                run.tool_sequence_json
                    .as_array()
                    .map(|items| items.len())
                    .unwrap_or(0)
            })
            .sum::<usize>() as f64
            / matched.len() as f64
    };
    let resolved_count = resolved.len();
    SkillMetricsSnapshot {
        samples: resolved_count,
        matched_runs: matched.len(),
        total_window_runs: window.len(),
        successful_runs,
        failed_runs,
        corrected_runs,
        success_rate: if resolved_count == 0 {
            0.0
        } else {
            round4(successful_runs as f64 / resolved_count as f64)
        },
        failure_rate: if resolved_count == 0 {
            0.0
        } else {
            round4(failed_runs as f64 / resolved_count as f64)
        },
        corrected_rate: if resolved_count == 0 {
            0.0
        } else {
            round4(corrected_runs as f64 / resolved_count as f64)
        },
        tool_error_rate: if tool_attempts.is_empty() {
            0.0
        } else {
            round4(tool_error_count as f64 / tool_attempts.len() as f64)
        },
        avg_tool_calls: round4(avg_tool_calls),
        selection_rate: if window.is_empty() {
            0.0
        } else {
            round4(matched.len() as f64 / window.len() as f64)
        },
    }
}

pub fn assess_skill_impact(
    baseline: &SkillMetricsSnapshot,
    observed: &SkillMetricsSnapshot,
) -> SkillImpactAssessment {
    let success_gain = round4(observed.success_rate - baseline.success_rate);
    let failure_reduction = round4(baseline.failure_rate - observed.failure_rate);
    let tool_error_reduction = round4(baseline.tool_error_rate - observed.tool_error_rate);
    if baseline.samples < 2 {
        return SkillImpactAssessment {
            status: "inconclusive".to_string(),
            summary: vec!["Baseline evidence is too small to score impact yet.".to_string()],
            success_gain,
            failure_reduction,
            tool_error_reduction,
        };
    }
    if observed.samples < 3 {
        return SkillImpactAssessment {
            status: "pending".to_string(),
            summary: vec![format!(
                "Waiting for more post-approval runs ({} of 3 resolved so far).",
                observed.samples
            )],
            success_gain,
            failure_reduction,
            tool_error_reduction,
        };
    }
    let improved = success_gain >= 0.08
        || failure_reduction >= 0.08
        || tool_error_reduction >= 0.08
        || (success_gain >= 0.04 && tool_error_reduction >= 0.04);
    let regressed =
        success_gain <= -0.08 || failure_reduction <= -0.08 || tool_error_reduction <= -0.08;
    if improved && !regressed {
        let mut summary = Vec::new();
        if success_gain > 0.0 {
            summary.push(format!(
                "Success rate improved by {:.1} pts.",
                success_gain * 100.0
            ));
        }
        if failure_reduction > 0.0 {
            summary.push(format!(
                "Failure rate fell by {:.1} pts.",
                failure_reduction * 100.0
            ));
        }
        if tool_error_reduction > 0.0 {
            summary.push(format!(
                "Tool-error rate fell by {:.1} pts.",
                tool_error_reduction * 100.0
            ));
        }
        return SkillImpactAssessment {
            status: "improved".to_string(),
            summary,
            success_gain,
            failure_reduction,
            tool_error_reduction,
        };
    }
    if regressed {
        let mut summary = Vec::new();
        if success_gain < 0.0 {
            summary.push(format!(
                "Success rate dropped by {:.1} pts.",
                success_gain.abs() * 100.0
            ));
        }
        if failure_reduction < 0.0 {
            summary.push(format!(
                "Failure rate increased by {:.1} pts.",
                failure_reduction.abs() * 100.0
            ));
        }
        if tool_error_reduction < 0.0 {
            summary.push(format!(
                "Tool-error rate increased by {:.1} pts.",
                tool_error_reduction.abs() * 100.0
            ));
        }
        return SkillImpactAssessment {
            status: "regressed".to_string(),
            summary,
            success_gain,
            failure_reduction,
            tool_error_reduction,
        };
    }
    SkillImpactAssessment {
        status: "unchanged".to_string(),
        summary: vec![
            "Post-approval runs have not cleared the improvement threshold yet.".to_string(),
        ],
        success_gain,
        failure_reduction,
        tool_error_reduction,
    }
}

pub fn load_skill_history(history_dir: &Path) -> Result<Vec<SkillHistoryEntry>> {
    if !history_dir.exists() {
        return Ok(Vec::new());
    }
    let mut snapshots = HashMap::<u32, PathBuf>::new();
    let mut evidences = HashMap::<u32, PathBuf>::new();
    let mut create_evidence = None::<PathBuf>;
    for entry in std::fs::read_dir(history_dir)
        .with_context(|| format!("failed reading history dir {:?}", history_dir))?
        .flatten()
    {
        let path = entry.path();
        let Some(name) = path.file_name().and_then(|value| value.to_str()) else {
            continue;
        };
        if name == "v0_evidence.md" {
            create_evidence = Some(path);
            continue;
        }
        if let Some(version) = name
            .strip_prefix('v')
            .and_then(|value| value.strip_suffix(".md"))
            .and_then(|value| value.parse::<u32>().ok())
        {
            snapshots.insert(version, path);
            continue;
        }
        if let Some(version) = name
            .strip_prefix('v')
            .and_then(|value| value.strip_suffix("_evidence.md"))
            .and_then(|value| value.parse::<u32>().ok())
        {
            evidences.insert(version, path);
        }
    }
    let mut entries = Vec::new();
    if let Some(path) = create_evidence {
        entries.push(SkillHistoryEntry {
            version: 0,
            snapshot_path: None,
            evidence_path: path.clone(),
            snapshot_content: None,
            evidence_content: std::fs::read_to_string(&path).unwrap_or_default(),
        });
    }
    let versions = snapshots
        .keys()
        .chain(evidences.keys())
        .copied()
        .collect::<BTreeSet<_>>();
    for version in versions.iter().copied() {
        let snapshot_path = snapshots.get(&version).cloned();
        let evidence_path = evidences
            .get(&version)
            .cloned()
            .unwrap_or_else(|| history_dir.join(format!("v{}_evidence.md", version)));
        entries.push(SkillHistoryEntry {
            version,
            snapshot_content: snapshot_path
                .as_ref()
                .and_then(|path| std::fs::read_to_string(path).ok()),
            snapshot_path,
            evidence_content: std::fs::read_to_string(&evidence_path).unwrap_or_default(),
            evidence_path,
        });
    }
    entries.sort_by_key(|entry| entry.version);
    Ok(entries)
}

pub fn next_skill_history_version(history_dir: &Path) -> Result<u32> {
    let history = load_skill_history(history_dir)?;
    Ok(history
        .iter()
        .filter(|entry| entry.version > 0)
        .map(|entry| entry.version)
        .max()
        .unwrap_or(0)
        + 1)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn replace_frontmatter_value_updates_description() {
        let raw = "---\nname: demo\ndescription: old\n---\n\n# Demo\n";
        let updated = replace_frontmatter_value(raw, "description", "new text");
        assert!(updated.contains("description: \"new text\""));
    }

    #[test]
    fn upsert_markdown_section_replaces_existing_section() {
        let raw = "# Demo\n\n## Common failure checks\n\nOld body\n\n## Workflow\n\nBody\n";
        let updated = upsert_markdown_section(raw, "## Common failure checks", "- New body");
        assert!(updated.contains("- New body"));
        assert!(!updated.contains("Old body"));
    }

    #[test]
    fn assess_skill_impact_marks_clear_improvement() {
        let baseline = SkillMetricsSnapshot {
            samples: 4,
            success_rate: 0.25,
            failure_rate: 0.75,
            tool_error_rate: 0.5,
            ..SkillMetricsSnapshot::default()
        };
        let observed = SkillMetricsSnapshot {
            samples: 4,
            success_rate: 0.75,
            failure_rate: 0.25,
            tool_error_rate: 0.1,
            ..SkillMetricsSnapshot::default()
        };
        let assessment = assess_skill_impact(&baseline, &observed);
        assert_eq!(assessment.status, "improved");
    }
}
