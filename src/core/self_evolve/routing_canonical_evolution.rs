//! Routing-canonical evolution support.
//!
//! This module treats routing canonicals as data artifacts. Evolve can
//! propose semantic canonical changes from replay evidence, gate them, then
//! promote them into the existing security canonical overlay without changing
//! router prompt text.

use anyhow::{Context, Result, anyhow};
use serde::{Deserialize, Serialize};
use std::path::{Path, PathBuf};

use crate::storage::learning_candidate;

pub const ROUTING_CANONICAL_CANDIDATE_TYPE: &str = "routing_canonical";
pub const ROUTING_CANONICAL_SUBJECT_KEY: &str = "security.routing.canonicals";

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct RoutingCanonicalOverlayEntry {
    pub category: String,
    pub concept: String,
    pub text: String,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct RoutingCanonicalCandidatePayload {
    #[serde(default)]
    pub add: Vec<RoutingCanonicalOverlayEntry>,
    #[serde(default)]
    pub remove_concepts: Vec<String>,
    #[serde(default)]
    pub evidence_summary: Option<String>,
}

fn overlay_path(data_dir: &Path) -> PathBuf {
    data_dir.join("security").join("canonicals.json")
}

fn normalize_overlay_label(raw: &str) -> String {
    raw.trim()
        .chars()
        .map(|ch| {
            if ch.is_ascii_alphanumeric() {
                ch.to_ascii_lowercase()
            } else {
                '_'
            }
        })
        .collect::<String>()
        .split('_')
        .filter(|part| !part.is_empty())
        .collect::<Vec<_>>()
        .join("_")
}

fn valid_category(raw: &str) -> bool {
    matches!(
        normalize_overlay_label(raw).as_str(),
        "conversational" | "tool_use" | "durable_work" | "managed_app_delivery" | "security_block"
    )
}

fn valid_concept(raw: &str) -> bool {
    let trimmed = raw.trim();
    !trimmed.is_empty()
        && trimmed.chars().count() <= 80
        && trimmed
            .chars()
            .all(|ch| ch.is_ascii_alphanumeric() || matches!(ch, '_' | '-' | '.'))
}

fn valid_canonical_text(raw: &str) -> bool {
    let trimmed = raw.trim();
    trimmed.chars().count() >= 40 && trimmed.chars().count() <= 700
}

pub fn parse_routing_canonical_candidate(
    candidate: &learning_candidate::Model,
) -> Result<RoutingCanonicalCandidatePayload> {
    if candidate.candidate_type != ROUTING_CANONICAL_CANDIDATE_TYPE {
        return Err(anyhow!(
            "expected candidate_type '{}', got '{}'",
            ROUTING_CANONICAL_CANDIDATE_TYPE,
            candidate.candidate_type
        ));
    }
    let payload: RoutingCanonicalCandidatePayload =
        serde_json::from_value(candidate.proposed_content.clone())
            .context("routing canonical candidate payload is invalid")?;
    validate_routing_canonical_candidate(&payload)?;
    Ok(payload)
}

pub fn validate_routing_canonical_candidate(
    payload: &RoutingCanonicalCandidatePayload,
) -> Result<()> {
    if payload.add.is_empty() && payload.remove_concepts.is_empty() {
        return Err(anyhow!(
            "routing canonical candidate must add or remove at least one canonical"
        ));
    }
    if payload.add.len() > 16 || payload.remove_concepts.len() > 32 {
        return Err(anyhow!(
            "routing canonical candidate is too large for one promotion"
        ));
    }
    for entry in &payload.add {
        if !valid_category(&entry.category) {
            return Err(anyhow!("routing canonical category is not supported"));
        }
        if !valid_concept(&entry.concept) {
            return Err(anyhow!(
                "routing canonical concept is not structurally valid"
            ));
        }
        if !valid_canonical_text(&entry.text) {
            return Err(anyhow!(
                "routing canonical text must be a concise semantic descriptor"
            ));
        }
    }
    for concept in &payload.remove_concepts {
        if !valid_concept(concept) {
            return Err(anyhow!(
                "routing canonical removal target is not structurally valid"
            ));
        }
    }
    Ok(())
}

pub async fn load_routing_canonical_overlay(
    data_dir: &Path,
) -> Result<Vec<RoutingCanonicalOverlayEntry>> {
    let path = overlay_path(data_dir);
    let raw = match tokio::fs::read(&path).await {
        Ok(raw) => raw,
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => return Ok(Vec::new()),
        Err(error) => return Err(error).with_context(|| format!("failed to read {:?}", path)),
    };
    serde_json::from_slice::<Vec<RoutingCanonicalOverlayEntry>>(&raw)
        .with_context(|| format!("failed to parse {:?}", path))
}

pub async fn write_routing_canonical_overlay(
    data_dir: &Path,
    entries: &[RoutingCanonicalOverlayEntry],
) -> Result<()> {
    let path = overlay_path(data_dir);
    if let Some(parent) = path.parent() {
        tokio::fs::create_dir_all(parent).await?;
    }
    let mut normalized = entries.to_vec();
    normalized.sort_by(|left, right| {
        left.category
            .cmp(&right.category)
            .then_with(|| left.concept.cmp(&right.concept))
    });
    let raw = serde_json::to_vec_pretty(&normalized)?;
    tokio::fs::write(&path, raw)
        .await
        .with_context(|| format!("failed to write {:?}", path))
}

pub async fn promote_routing_canonical_candidate(
    data_dir: &Path,
    candidate: &learning_candidate::Model,
) -> Result<usize> {
    let payload = parse_routing_canonical_candidate(candidate)?;
    let mut entries = load_routing_canonical_overlay(data_dir).await?;
    let removals = payload
        .remove_concepts
        .iter()
        .map(|value| value.trim().to_string())
        .collect::<std::collections::BTreeSet<_>>();
    entries.retain(|entry| !removals.contains(entry.concept.trim()));
    for entry in payload.add {
        entries.retain(|existing| existing.concept.trim() != entry.concept.trim());
        entries.push(RoutingCanonicalOverlayEntry {
            category: normalize_overlay_label(&entry.category),
            concept: entry.concept.trim().to_string(),
            text: entry.text.split_whitespace().collect::<Vec<_>>().join(" "),
        });
    }
    let promoted = entries.len();
    write_routing_canonical_overlay(data_dir, &entries).await?;
    Ok(promoted)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn candidate(payload: serde_json::Value) -> learning_candidate::Model {
        learning_candidate::Model {
            id: "candidate-1".to_string(),
            candidate_type: ROUTING_CANONICAL_CANDIDATE_TYPE.to_string(),
            subject_key: ROUTING_CANONICAL_SUBJECT_KEY.to_string(),
            title: "Routing canonical".to_string(),
            summary: None,
            project_id: None,
            conversation_id: None,
            pattern_id: None,
            evidence_refs: serde_json::json!([]),
            proposed_content: payload,
            confidence: 0.8,
            approval_status: "draft".to_string(),
            review_notes: None,
            reviewed_at: None,
            approved_ref: None,
            created_at: "2026-01-01T00:00:00Z".to_string(),
            updated_at: "2026-01-01T00:00:00Z".to_string(),
        }
    }

    #[test]
    fn routing_canonical_candidate_validates_semantic_entries() {
        let payload = parse_routing_canonical_candidate(&candidate(serde_json::json!({
            "add": [{
                "category": "durable_work",
                "concept": "background_monitoring_goal",
                "text": "The user wants durable background monitoring that persists independently of the current response and reports only when its condition is met."
            }],
            "evidence_summary": "Corrected routing evidence showed a durable monitor was missed."
        })))
        .expect("valid candidate");

        assert_eq!(payload.add[0].category, "durable_work");
    }

    #[test]
    fn routing_canonical_candidate_rejects_unsupported_category() {
        let error = parse_routing_canonical_candidate(&candidate(serde_json::json!({
            "add": [{
                "category": "unknown",
                "concept": "background_monitoring_goal",
                "text": "The user wants durable background monitoring that persists independently of the current response and reports only when its condition is met."
            }]
        })))
        .expect_err("unsupported category should fail");

        assert!(error.to_string().contains("category"));
    }

    #[tokio::test]
    async fn promote_routing_canonical_candidate_rewrites_overlay_data() {
        let data_dir = tempfile::tempdir().expect("tempdir");
        write_routing_canonical_overlay(
            data_dir.path(),
            &[RoutingCanonicalOverlayEntry {
                category: "durable_work".to_string(),
                concept: "old_monitoring_goal".to_string(),
                text: "The user wants an old durable monitoring semantic descriptor that should be removed."
                    .to_string(),
            }],
        )
        .await
        .expect("seed overlay");

        let promoted = promote_routing_canonical_candidate(
            data_dir.path(),
            &candidate(serde_json::json!({
                "add": [{
                    "category": "durable_work",
                    "concept": "background_monitoring_goal",
                    "text": "The user wants durable background monitoring that persists independently of the current response and reports only when its condition is met."
                }],
                "remove_concepts": ["old_monitoring_goal"],
                "evidence_summary": "Replay evidence showed the old canonical was too broad."
            })),
        )
        .await
        .expect("promote candidate");
        let entries = load_routing_canonical_overlay(data_dir.path())
            .await
            .expect("load overlay");

        assert_eq!(promoted, 1);
        assert_eq!(entries.len(), 1);
        assert_eq!(entries[0].category, "durable_work");
        assert_eq!(entries[0].concept, "background_monitoring_goal");
    }
}
