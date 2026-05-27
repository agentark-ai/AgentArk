use std::collections::{HashMap, HashSet};
use std::path::Path;

use anyhow::Result;
use serde_json::{json, Value};
use sha2::{Digest, Sha256};

use crate::storage::{
    experience_edge, experience_item, experience_run, learning_candidate, procedural_pattern,
    KvLeaseGuard, Storage,
};

pub const LEARNING_ENABLED_KEY: &str = "learning_enabled_v1";
pub const LEARNING_MODEL_SLOT_KEY: &str = "learning_model_slot_v1";
pub const LEARNING_QUEUE_CAP_KEY: &str = "learning_queue_cap_v1";
const LEARNING_CANDIDATE_GENERATION_LEASE_KEY: &str = "learning_candidate_generation_lease_v1";
const LEARNING_CANDIDATE_GENERATION_LEASE_TTL_SECS: i64 = 10 * 60;
const LEARNING_CANDIDATE_GENERATION_LEASE_HEARTBEAT_SECS: u64 = 60;
const CORRECTION_WINDOW_MINUTES: i64 = 30;
const DEFAULT_QUEUE_CAP: usize = 64;
pub(crate) const HEURISTIC_REFLECTION_ORIGIN: &str = "heuristic_reflection";
pub(crate) const HEURISTIC_REFLECTION_VERSION: &str = "heuristic-reflection-v1";

#[derive(Debug, Clone)]
pub(crate) struct ReflectedHeuristic {
    pub heuristic: String,
    pub polarity: String,
    pub confidence: f64,
    pub applicability: Option<String>,
}

#[derive(Debug, Clone)]
pub(crate) struct ReflectedHeuristicPersistOutcome {
    pub lesson_id: String,
    pub merged: bool,
}

fn safe_truncate(value: &str, max_chars: usize) -> String {
    if value.chars().count() <= max_chars {
        return value.to_string();
    }
    value.chars().take(max_chars).collect::<String>()
}

fn stable_id(prefix: &str, parts: &[&str]) -> String {
    let mut hasher = Sha256::new();
    hasher.update(prefix.as_bytes());
    for part in parts {
        hasher.update([0u8]);
        hasher.update(part.as_bytes());
    }
    let digest = hex::encode(hasher.finalize());
    format!("{}-{}", prefix, &digest[..24])
}

fn short_hash(parts: &[&str]) -> String {
    stable_id("h", parts)
        .rsplit('-')
        .next()
        .unwrap_or("candidate")
        .to_string()
}

#[cfg(test)]
pub(crate) fn derive_intent_key(request_text: &str, task_type: &str) -> String {
    let task = task_type
        .trim()
        .to_ascii_lowercase()
        .chars()
        .map(|ch| if ch.is_ascii_alphanumeric() { ch } else { '-' })
        .collect::<String>()
        .trim_matches('-')
        .to_string();
    let mut words = request_text
        .chars()
        .map(|ch| {
            if ch.is_ascii_alphanumeric() {
                ch.to_ascii_lowercase()
            } else {
                ' '
            }
        })
        .collect::<String>()
        .split_whitespace()
        .take(8)
        .map(str::to_string)
        .collect::<Vec<_>>();
    if words.is_empty() {
        words.push(short_hash(&[request_text]).chars().take(8).collect());
    }
    format!(
        "{}::{}",
        if task.is_empty() {
            "general"
        } else {
            task.as_str()
        },
        words.join("-")
    )
}

fn tool_names_from_value(value: &Value) -> Vec<String> {
    value
        .as_array()
        .map(|items| {
            items
                .iter()
                .filter_map(|item| item.get("tool_name").and_then(|value| value.as_str()))
                .map(|value| value.to_string())
                .collect::<Vec<_>>()
        })
        .unwrap_or_default()
}

fn suggested_steps_from_tools(tool_names: &[String]) -> Vec<String> {
    if tool_names.is_empty() {
        return vec!["Review the latest run context and complete the task directly.".to_string()];
    }
    tool_names
        .iter()
        .enumerate()
        .map(|(index, tool)| {
            if index == tool_names.len().saturating_sub(1) {
                format!("Finish with `{}` and return the concrete result.", tool)
            } else {
                format!(
                    "Use `{}` as step {} in the learned sequence.",
                    tool,
                    index + 1
                )
            }
        })
        .collect()
}

fn bool_like(value: &str) -> Option<bool> {
    match value.trim().to_ascii_lowercase().as_str() {
        "true" | "yes" | "on" | "1" => Some(true),
        "false" | "no" | "off" | "0" => Some(false),
        _ => None,
    }
}

fn humanize_fact_key(key: &str) -> String {
    key.split('_')
        .filter(|part| !part.trim().is_empty())
        .map(|part| {
            let mut chars = part.chars();
            match chars.next() {
                Some(first) => first.to_ascii_uppercase().to_string() + chars.as_str(),
                None => String::new(),
            }
        })
        .collect::<Vec<_>>()
        .join(" ")
}

fn describe_user_preference_memory(
    key: &str,
    value: &str,
) -> Option<(&'static str, String, String)> {
    let key = key.trim();
    let value = value.trim();
    if key.is_empty() || value.is_empty() {
        return None;
    }

    let normalized_value = safe_truncate(value, 220);
    let (kind, title, content) = match key {
        "user_name" => (
            "personal_fact",
            "Learned personal fact".to_string(),
            format!("The user's preferred name is {}.", normalized_value),
        ),
        "rule_require_explicit_approval_before_side_effects" => (
            "constraint",
            "Learned operating constraint".to_string(),
            if bool_like(value).unwrap_or(true) {
                "Require explicit approval before side-effecting actions.".to_string()
            } else {
                "Explicit approval before side-effecting actions is not required.".to_string()
            },
        ),
        "rule_show_plan_before_side_effects" => (
            "constraint",
            "Learned operating constraint".to_string(),
            if bool_like(value).unwrap_or(true) {
                "Show the plan before side-effecting actions.".to_string()
            } else {
                "Showing the plan before side-effecting actions is optional.".to_string()
            },
        ),
        _ if key.starts_with("likes_") => (
            "personal_fact",
            "Learned personal fact".to_string(),
            format!("The user likes {}.", normalized_value),
        ),
        _ if key.starts_with("dislikes_") => (
            "personal_fact",
            "Learned personal fact".to_string(),
            format!("The user dislikes {}.", normalized_value),
        ),
        _ if key.starts_with("rule_") => (
            "constraint",
            "Learned operating constraint".to_string(),
            format!("{}: {}.", humanize_fact_key(key), normalized_value),
        ),
        _ => (
            "personal_fact",
            "Learned personal fact".to_string(),
            format!("{}: {}.", humanize_fact_key(key), normalized_value),
        ),
    };

    Some((kind, title, content))
}

pub async fn load_learning_enabled(storage: &Storage) -> bool {
    storage
        .get(LEARNING_ENABLED_KEY)
        .await
        .ok()
        .flatten()
        .and_then(|bytes| String::from_utf8(bytes).ok())
        .map(|value| !value.trim().eq_ignore_ascii_case("false"))
        .unwrap_or(true)
}

pub async fn load_learning_model_slot(storage: &Storage) -> Option<String> {
    storage
        .get(LEARNING_MODEL_SLOT_KEY)
        .await
        .ok()
        .flatten()
        .and_then(|bytes| String::from_utf8(bytes).ok())
        .map(|value| value.trim().to_string())
        .filter(|value| !value.is_empty())
}

pub async fn load_learning_queue_cap(storage: &Storage) -> usize {
    storage
        .get(LEARNING_QUEUE_CAP_KEY)
        .await
        .ok()
        .flatten()
        .and_then(|bytes| String::from_utf8(bytes).ok())
        .and_then(|value| value.trim().parse::<usize>().ok())
        .filter(|value| *value > 0)
        .unwrap_or(DEFAULT_QUEUE_CAP)
}

fn build_item_id(
    kind: &str,
    scope: &str,
    project_id: Option<&str>,
    conversation_id: Option<&str>,
    normalized_key: &str,
) -> String {
    stable_id(
        "expitem",
        &[
            kind,
            scope,
            project_id.unwrap_or(""),
            conversation_id.unwrap_or(""),
            normalized_key,
        ],
    )
}

fn build_pattern_id(
    scope: &str,
    project_id: Option<&str>,
    conversation_id: Option<&str>,
    intent_key: &str,
    tool_sequence_digest: Option<&str>,
) -> String {
    stable_id(
        "pattern",
        &[
            scope,
            project_id.unwrap_or(""),
            conversation_id.unwrap_or(""),
            intent_key,
            tool_sequence_digest.unwrap_or(""),
        ],
    )
}

fn positive_procedure_summary(run: &experience_run::Model, tool_names: &[String]) -> String {
    if tool_names.is_empty() {
        format!(
            "For `{}`, the successful pattern was to solve the task directly and return the result.",
            run.intent_key
        )
    } else {
        format!(
            "For `{}`, the successful pattern was: {}.",
            run.intent_key,
            tool_names.join(" -> ")
        )
    }
}

fn negative_lesson_summary(run: &experience_run::Model, tool_names: &[String]) -> String {
    let sequence = if tool_names.is_empty() {
        "the recent approach".to_string()
    } else {
        tool_names.join(" -> ")
    };
    format!(
        "Avoid repeating `{}` with {} when the user is correcting or the run failed.",
        run.intent_key, sequence
    )
}

fn experience_run_learning_signal_bool(run: &experience_run::Model, key: &str) -> bool {
    run.metadata
        .get("learning_signal")
        .and_then(|value| value.get(key))
        .and_then(|value| value.as_bool())
        .unwrap_or(false)
}

fn experience_run_has_procedure_evidence(
    run: &experience_run::Model,
    tool_names: &[String],
) -> bool {
    experience_run_learning_signal_bool(run, "procedure_eligible")
        || !tool_names.is_empty()
        || run.correction_state == "corrected"
        || run.success_state == "failed"
}

fn experience_item_memory_kind(item: &experience_item::Model) -> Option<String> {
    item.metadata
        .get("memory_kind")
        .and_then(|value| value.as_str())
        .map(str::trim)
        .filter(|value| !value.is_empty())
        .map(str::to_string)
}

fn reconciled_memory_category(item: &experience_item::Model) -> Option<&'static str> {
    let semantic_kind = experience_item_memory_kind(item);
    let existing = item
        .metadata
        .get("memory_category")
        .and_then(|value| value.as_str())
        .map(str::trim)
        .filter(|value| !value.is_empty());
    let fallback =
        crate::core::memory_schema::normalize_memory_category(None, semantic_kind.as_deref());
    let normalized =
        crate::core::memory_schema::normalize_memory_category(existing, semantic_kind.as_deref());
    if existing.is_none() && fallback != crate::core::memory_schema::MEMORY_CATEGORY_OTHER {
        return Some(fallback);
    }
    if existing == Some(crate::core::memory_schema::MEMORY_CATEGORY_OTHER)
        && fallback != crate::core::memory_schema::MEMORY_CATEGORY_OTHER
    {
        return Some(fallback);
    }
    if existing.is_some_and(|raw| raw != normalized) {
        return Some(normalized);
    }
    None
}

async fn reconcile_memory_categories(storage: &Storage, cap: u64) -> Result<usize> {
    let items = storage
        .list_active_experience_items_any_scope(&["constraint", "personal_fact"], cap)
        .await?
        .into_iter()
        .filter(|item| !experience_item_is_external_source(item))
        .collect::<Vec<_>>();
    let mut changed = 0usize;
    for item in items {
        let Some(category) = reconciled_memory_category(&item) else {
            continue;
        };
        let mut updated = item.clone();
        let mut metadata = item.metadata.as_object().cloned().unwrap_or_default();
        metadata.insert(
            "memory_category".to_string(),
            serde_json::Value::String(category.to_string()),
        );
        updated.metadata = serde_json::Value::Object(metadata);
        updated.updated_at = chrono::Utc::now().to_rfc3339();
        storage.upsert_experience_item(&updated).await?;
        changed += 1;
    }
    Ok(changed)
}

pub async fn sync_user_preference_to_experience_item(
    storage: &Storage,
    key: &str,
    value: &str,
    confidence: f64,
    source: &str,
    sensitivity: Option<&str>,
) -> Result<()> {
    let Some((kind, title, content)) = describe_user_preference_memory(key, value) else {
        return Ok(());
    };
    let sensitivity =
        crate::storage::entities::user_preference::normalize_memory_sensitivity(sensitivity)
            .unwrap_or_else(|| {
                crate::storage::entities::user_preference::classify_user_preference_sensitivity(
                    key, value,
                )
            })
            .as_str();
    let normalized_key = format!("user_pref::{}", key.trim());
    let id = build_item_id(kind, "global", None, None, &normalized_key);
    let existing = storage.get_experience_item(&id).await?;
    let now = chrono::Utc::now().to_rfc3339();
    let support_count = existing
        .as_ref()
        .map(|item| item.support_count.saturating_add(1))
        .unwrap_or(1);
    let merged_confidence = existing
        .as_ref()
        .map(|item| item.confidence.max(confidence).min(0.99))
        .unwrap_or(confidence);

    storage
        .upsert_experience_item(&experience_item::Model {
            id,
            kind: kind.to_string(),
            scope: "global".to_string(),
            project_id: None,
            conversation_id: None,
            title,
            content,
            normalized_key,
            confidence: merged_confidence,
            support_count,
            contradiction_count: existing
                .as_ref()
                .map(|item| item.contradiction_count)
                .unwrap_or_default(),
            status: "active".to_string(),
            metadata: json!({
                "source": source,
                "user_preference_key": key.trim(),
                "user_preference_value": safe_truncate(value.trim(), 220),
                "memory_category": if kind == "constraint" {
                    crate::core::memory_schema::MEMORY_CATEGORY_WORK_PREFERENCE
                } else {
                    crate::core::memory_schema::MEMORY_CATEGORY_ASSISTANT_PREFERENCE
                },
                "memory_kind": if kind == "constraint" {
                    "constraint"
                } else {
                    "assistant_preference"
                },
                "sensitivity": sensitivity,
            }),
            last_supported_at: Some(now.clone()),
            last_contradicted_at: existing
                .as_ref()
                .and_then(|item| item.last_contradicted_at.clone()),
            created_at: existing
                .as_ref()
                .map(|item| item.created_at.clone())
                .unwrap_or_else(|| now.clone()),
            updated_at: now,
            embedding: existing.as_ref().and_then(|item| item.embedding.clone()),
        })
        .await?;

    Ok(())
}

async fn consolidate_run(storage: &Storage, run: &experience_run::Model) -> Result<()> {
    let tool_names = tool_names_from_value(&run.tool_sequence_json);
    let scope = "global";
    let project_id = None;
    let conversation_id = None;
    let now = chrono::Utc::now().to_rfc3339();

    let is_negative = run.correction_state == "corrected" || run.success_state == "failed";
    if !is_negative && !experience_run_has_procedure_evidence(run, &tool_names) {
        storage.mark_experience_run_consolidated(&run.id).await?;
        return Ok(());
    }
    if is_negative {
        let related_procedure_key = format!(
            "procedure::{}::{}",
            run.intent_key,
            run.tool_sequence_digest.as_deref().unwrap_or("direct")
        );
        let related_procedure_id = build_item_id(
            "procedure",
            scope,
            project_id,
            conversation_id,
            &related_procedure_key,
        );
        if let Some(mut procedure) = storage.get_experience_item(&related_procedure_id).await? {
            procedure.contradiction_count = procedure.contradiction_count.saturating_add(1);
            procedure.confidence = (procedure.confidence - 0.12).max(0.20);
            procedure.last_contradicted_at = Some(now.clone());
            procedure.updated_at = now.clone();
            storage.upsert_experience_item(&procedure).await?;
        }
    }
    let kind = if is_negative { "lesson" } else { "procedure" };
    let normalized_key = format!(
        "{}::{}::{}",
        kind,
        run.intent_key,
        run.tool_sequence_digest.as_deref().unwrap_or("direct")
    );
    let id = build_item_id(kind, scope, project_id, conversation_id, &normalized_key);
    let existing = storage.get_experience_item(&id).await?;
    let support_count = existing
        .as_ref()
        .map(|item| item.support_count.saturating_add(1))
        .unwrap_or(1);
    let contradiction_count = if is_negative {
        existing
            .as_ref()
            .map(|item| item.contradiction_count)
            .unwrap_or_default()
    } else {
        existing
            .as_ref()
            .map(|item| item.contradiction_count)
            .unwrap_or_default()
    };
    let confidence = if is_negative {
        existing
            .as_ref()
            .map(|item| (item.confidence + 0.05).min(0.94))
            .unwrap_or(0.64)
    } else {
        existing
            .as_ref()
            .map(|item| (item.confidence + 0.08).min(0.98))
            .unwrap_or(0.7)
    };
    let content = if is_negative {
        negative_lesson_summary(run, &tool_names)
    } else {
        positive_procedure_summary(run, &tool_names)
    };
    let steps = suggested_steps_from_tools(&tool_names);
    storage
        .upsert_experience_item(&experience_item::Model {
            id: id.clone(),
            kind: kind.to_string(),
            scope: scope.to_string(),
            project_id: None,
            conversation_id: None,
            title: if is_negative {
                format!("Lesson for {}", run.intent_key)
            } else {
                format!("Procedure for {}", run.intent_key)
            },
            content,
            normalized_key,
            confidence,
            support_count,
            contradiction_count,
            status: "active".to_string(),
            metadata: json!({
                "intent_key": run.intent_key,
                "task_type": run.task_type,
                "tool_sequence_digest": run.tool_sequence_digest,
                "tool_sequence": tool_names,
                "suggested_steps": steps,
                "source_run_id": run.id,
                "polarity": if is_negative { "negative" } else { "positive" },
                "learning_signal": run
                    .metadata
                    .get("learning_signal")
                    .cloned()
                    .unwrap_or(Value::Null),
                "global_learning": true,
            }),
            last_supported_at: Some(now.clone()),
            last_contradicted_at: existing
                .as_ref()
                .and_then(|item| item.last_contradicted_at.clone()),
            created_at: existing
                .as_ref()
                .map(|item| item.created_at.clone())
                .unwrap_or_else(|| now.clone()),
            updated_at: now.clone(),
            embedding: existing.as_ref().and_then(|item| item.embedding.clone()),
        })
        .await?;

    storage
        .upsert_experience_edge(&experience_edge::Model {
            id: stable_id("edge", &[run.id.as_str(), "derived_from", id.as_str()]),
            source_ref: id.clone(),
            source_kind: "experience_item".to_string(),
            target_ref: run.id.clone(),
            target_kind: "experience_run".to_string(),
            edge_type: "derived_from".to_string(),
            weight: 1.0,
            source_run_id: Some(run.id.clone()),
            metadata: json!({
                "intent_key": run.intent_key,
                "success_state": run.success_state,
                "correction_state": run.correction_state,
            }),
            created_at: now.clone(),
            updated_at: now.clone(),
        })
        .await?;

    storage.mark_experience_run_consolidated(&run.id).await?;
    Ok(())
}

pub async fn run_experience_consolidation(storage: &Storage) -> Result<usize> {
    if !load_learning_enabled(storage).await {
        return Ok(0);
    }
    let cap = load_learning_queue_cap(storage).await as u64;
    let _ = storage
        .finalize_stale_provisional_experience_runs(CORRECTION_WINDOW_MINUTES, cap)
        .await?;
    let runs = storage.list_experience_runs_for_consolidation(cap).await?;
    let mut processed = 0usize;
    for run in runs {
        consolidate_run(storage, &run).await?;
        processed += 1;
    }
    Ok(processed)
}

pub async fn run_pattern_induction(storage: &Storage) -> Result<usize> {
    if !load_learning_enabled(storage).await {
        return Ok(0);
    }
    let procedures = storage
        .list_active_experience_items_any_scope(
            &["procedure"],
            load_learning_queue_cap(storage).await as u64,
        )
        .await?
        .into_iter()
        .filter(|procedure| !experience_item_is_external_source(procedure))
        .filter(procedure_item_is_pattern_eligible)
        .collect::<Vec<_>>();
    let mut updated = 0usize;
    for procedure in procedures {
        let metadata = procedure.metadata.as_object().cloned().unwrap_or_default();
        let intent_key = metadata
            .get("intent_key")
            .and_then(|value| value.as_str())
            .unwrap_or(procedure.normalized_key.as_str());
        let tool_digest = metadata
            .get("tool_sequence_digest")
            .and_then(|value| value.as_str());
        let pattern_id = build_pattern_id("global", None, None, intent_key, tool_digest);
        let tool_sequence = metadata
            .get("tool_sequence")
            .cloned()
            .unwrap_or_else(|| Value::Array(Vec::new()));
        let steps_json = metadata
            .get("suggested_steps")
            .cloned()
            .unwrap_or_else(|| Value::Array(Vec::new()));
        let support_count = procedure.support_count.max(0);
        let correction_count = procedure.contradiction_count.max(0);
        let sample_count = support_count.saturating_add(correction_count).max(1);
        let success_rate = support_count as f64 / sample_count as f64;
        let now = chrono::Utc::now().to_rfc3339();
        storage
            .upsert_procedural_pattern(&procedural_pattern::Model {
                id: pattern_id.clone(),
                intent_key: intent_key.to_string(),
                scope: "global".to_string(),
                project_id: None,
                conversation_id: None,
                title: procedure.title.clone(),
                trigger_summary: format!(
                    "Use when the request matches `{}` and the learned evidence remains applicable.",
                    intent_key
                ),
                summary: procedure.content.clone(),
                tool_sequence_digest: tool_digest.map(|value| value.to_string()),
                steps_json,
                tool_sequence_json: tool_sequence,
                sample_count,
                success_count: support_count,
                correction_count,
                success_rate,
                last_validated_at: Some(now.clone()),
                status: if correction_count > support_count {
                    "deprecated".to_string()
                } else if support_count >= 2 {
                    "active".to_string()
                } else {
                    "draft".to_string()
                },
                metadata: json!({
                    "source_item_id": procedure.id,
                    "source_scope": procedure.scope,
                    "global_learning": true,
                    "task_type": metadata.get("task_type").cloned().unwrap_or(Value::Null),
                }),
                created_at: now.clone(),
                updated_at: now.clone(),
            })
            .await?;

        storage
            .upsert_experience_edge(&experience_edge::Model {
                id: stable_id(
                    "edge",
                    &[procedure.id.as_str(), "supports", pattern_id.as_str()],
                ),
                source_ref: procedure.id.clone(),
                source_kind: "experience_item".to_string(),
                target_ref: pattern_id.clone(),
                target_kind: "procedural_pattern".to_string(),
                edge_type: "supports".to_string(),
                weight: success_rate.max(0.1),
                source_run_id: None,
                metadata: json!({
                    "intent_key": intent_key,
                }),
                created_at: now.clone(),
                updated_at: now.clone(),
            })
            .await?;
        updated += 1;
    }
    Ok(updated)
}

fn build_strategy_candidate_profile(
    pattern: &procedural_pattern::Model,
    task_type: &str,
    tool_names: &[String],
) -> crate::core::self_evolve::strategy_runtime::ToolStrategyProfile {
    let mut task_guidance = std::collections::HashMap::new();
    let mut lines = vec![
        format!(
            "When the request matches `{}`, prefer the learned procedure `{}`.",
            task_type, pattern.title
        ),
        "Use this as guidance, not as a hard rule, and adapt when the context clearly differs."
            .to_string(),
    ];
    if !tool_names.is_empty() {
        lines.push(format!(
            "When the environment matches, start with tools in this order: {}.",
            tool_names.join(" -> ")
        ));
    }
    task_guidance.insert(task_type.to_string(), lines);
    crate::core::self_evolve::strategy_runtime::ToolStrategyProfile {
        version: format!(
            "learned-strategy-{}",
            short_hash(&[pattern.id.as_str(), task_type])
        ),
        updated_at: Some(chrono::Utc::now().to_rfc3339()),
        default_guidance: vec![
            "Prefer proven local procedures before improvising a new tool plan.".to_string(),
        ],
        task_guidance,
    }
}

fn normalize_semantic_text(value: &str) -> String {
    value
        .chars()
        .map(|ch| {
            if ch.is_ascii_alphanumeric() {
                ch.to_ascii_lowercase()
            } else {
                ' '
            }
        })
        .collect::<String>()
        .split_whitespace()
        .collect::<Vec<_>>()
        .join(" ")
}

fn semantic_token_set(value: &str) -> HashSet<String> {
    normalize_semantic_text(value)
        .split_whitespace()
        .filter(|token| token.len() >= 3)
        .map(|token| token.to_string())
        .collect()
}

fn semantic_similarity_score(left: &str, right: &str) -> f64 {
    let left_tokens = semantic_token_set(left);
    let right_tokens = semantic_token_set(right);
    if left_tokens.is_empty() || right_tokens.is_empty() {
        return 0.0;
    }
    let overlap = left_tokens.intersection(&right_tokens).count() as f64;
    let union = left_tokens.union(&right_tokens).count() as f64;
    if union <= f64::EPSILON {
        0.0
    } else {
        overlap / union
    }
}

fn experience_item_metadata_text<'a>(
    item: &'a experience_item::Model,
    field: &str,
) -> Option<&'a str> {
    item.metadata
        .get(field)
        .and_then(|value| value.as_str())
        .map(str::trim)
        .filter(|value| !value.is_empty())
}

pub(crate) fn experience_item_is_reflected_heuristic(item: &experience_item::Model) -> bool {
    item.kind == "lesson"
        && experience_item_metadata_text(item, "origin") == Some(HEURISTIC_REFLECTION_ORIGIN)
}

pub(crate) fn reflected_heuristic_task_type(item: &experience_item::Model) -> Option<&str> {
    experience_item_metadata_text(item, "task_type")
}

pub(crate) fn reflected_heuristic_polarity(item: &experience_item::Model) -> Option<&str> {
    experience_item_metadata_text(item, "polarity")
}

pub(crate) fn reflected_heuristic_applicability(item: &experience_item::Model) -> Option<&str> {
    experience_item_metadata_text(item, "applicability")
}

pub(crate) fn reflected_heuristic_confidence(item: &experience_item::Model) -> f64 {
    item.metadata
        .get("reflection_confidence")
        .and_then(|value| value.as_f64())
        .unwrap_or(item.confidence)
        .clamp(0.0, 1.0)
}

fn reflected_heuristic_merge_score(
    existing: &experience_item::Model,
    run: &experience_run::Model,
    heuristic: &ReflectedHeuristic,
) -> f64 {
    if !experience_item_is_reflected_heuristic(existing) {
        return 0.0;
    }
    if reflected_heuristic_polarity(existing) != Some(heuristic.polarity.as_str()) {
        return 0.0;
    }
    let existing_task_type = reflected_heuristic_task_type(existing)
        .or(run.task_type.as_deref())
        .unwrap_or("general");
    let current_task_type = run.task_type.as_deref().unwrap_or("general");
    if existing_task_type != current_task_type {
        return 0.0;
    }

    let existing_text = format!(
        "{} {}",
        existing.content,
        reflected_heuristic_applicability(existing).unwrap_or("")
    );
    let incoming_text = format!(
        "{} {}",
        heuristic.heuristic,
        heuristic.applicability.as_deref().unwrap_or("")
    );
    let mut score = semantic_similarity_score(&existing_text, &incoming_text);
    if normalize_semantic_text(&existing.content) == normalize_semantic_text(&heuristic.heuristic) {
        score = score.max(1.0);
    }
    score
}

fn reflected_heuristic_title(run: &experience_run::Model, polarity: &str) -> String {
    let task_type = run.task_type.as_deref().unwrap_or("general");
    match polarity {
        "negative" => format!("Reflected caution for {}", task_type),
        _ => format!("Reflected heuristic for {}", task_type),
    }
}

fn build_reflected_heuristic_normalized_key(
    run: &experience_run::Model,
    heuristic: &ReflectedHeuristic,
) -> String {
    let task_type = run.task_type.as_deref().unwrap_or("general");
    let normalized = normalize_semantic_text(&heuristic.heuristic);
    let applicability = normalize_semantic_text(heuristic.applicability.as_deref().unwrap_or(""));
    format!(
        "lesson::heuristic::{}::{}::{}",
        task_type,
        heuristic.polarity,
        short_hash(&[normalized.as_str(), applicability.as_str()])
    )
}

fn build_reflected_heuristic_strategy_profile(
    task_type: &str,
    lessons: &[experience_item::Model],
) -> crate::core::self_evolve::strategy_runtime::ToolStrategyProfile {
    let mut task_guidance = HashMap::new();
    let mut lines = vec![
        format!(
            "Apply these learned heuristics when handling similar {} requests.",
            task_type
        ),
        "Use them as decision guidance, and adapt when the current context clearly differs."
            .to_string(),
    ];
    for lesson in lessons {
        let content = lesson.content.trim();
        if content.is_empty() {
            continue;
        }
        lines.push(content.to_string());
    }
    lines.truncate(6);
    task_guidance.insert(task_type.to_string(), lines);
    crate::core::self_evolve::strategy_runtime::ToolStrategyProfile {
        version: format!(
            "learned-strategy-{}",
            short_hash(&[
                task_type,
                &lessons
                    .iter()
                    .map(|lesson| lesson.id.as_str())
                    .collect::<Vec<_>>()
                    .join("|"),
            ])
        ),
        updated_at: Some(chrono::Utc::now().to_rfc3339()),
        default_guidance: vec![
            "Prefer reflected heuristics from successful and corrected runs before improvising a new tool plan."
                .to_string(),
        ],
        task_guidance,
    }
}

pub(crate) async fn upsert_reflected_heuristic_lesson(
    storage: &Storage,
    run: &experience_run::Model,
    heuristic: &ReflectedHeuristic,
) -> Result<ReflectedHeuristicPersistOutcome> {
    let now = chrono::Utc::now().to_rfc3339();
    let scope = "global";
    let project_id = None;
    let conversation_id = None;
    let current_task_type = run.task_type.as_deref().unwrap_or("general");
    let lessons = storage
        .list_active_experience_items(&["lesson"], project_id, conversation_id, 48)
        .await?
        .into_iter()
        .filter(|item| item.scope == scope)
        .filter(|item| item.project_id.as_deref() == project_id)
        .filter(|item| item.conversation_id.as_deref() == conversation_id)
        .filter(experience_item_is_reflected_heuristic)
        .collect::<Vec<_>>();

    let existing_match = lessons
        .into_iter()
        .map(|item| {
            let score = reflected_heuristic_merge_score(&item, run, heuristic);
            (item, score)
        })
        .filter(|(_, score)| *score >= 0.62)
        .max_by(|left, right| left.1.total_cmp(&right.1));

    let lesson_id;
    let merged;
    if let Some((mut existing, _)) = existing_match {
        let merged_confidence =
            ((existing.confidence + heuristic.confidence.clamp(0.0, 1.0)) / 2.0).clamp(0.35, 0.99);
        let existing_content = normalize_semantic_text(&existing.content);
        let incoming_content = normalize_semantic_text(&heuristic.heuristic);
        if incoming_content.len() > existing_content.len()
            && heuristic.confidence >= existing.confidence
        {
            existing.content = safe_truncate(&heuristic.heuristic, 260);
        }

        let mut metadata = existing.metadata.as_object().cloned().unwrap_or_default();
        metadata.insert(
            "origin".to_string(),
            Value::String(HEURISTIC_REFLECTION_ORIGIN.to_string()),
        );
        metadata.insert("source_run_id".to_string(), Value::String(run.id.clone()));
        metadata.insert(
            "intent_key".to_string(),
            Value::String(run.intent_key.clone()),
        );
        metadata.insert(
            "task_type".to_string(),
            Value::String(current_task_type.to_string()),
        );
        metadata.insert(
            "tool_sequence_digest".to_string(),
            run.tool_sequence_digest
                .as_ref()
                .map(|value| Value::String(value.clone()))
                .unwrap_or(Value::Null),
        );
        metadata.insert(
            "polarity".to_string(),
            Value::String(heuristic.polarity.clone()),
        );
        metadata.insert(
            "reflection_version".to_string(),
            Value::String(HEURISTIC_REFLECTION_VERSION.to_string()),
        );
        metadata.insert(
            "reflection_confidence".to_string(),
            json!(merged_confidence),
        );
        metadata.insert("last_reflected_at".to_string(), Value::String(now.clone()));
        if let Some(applicability) = heuristic
            .applicability
            .as_deref()
            .filter(|value| !value.trim().is_empty())
        {
            metadata.insert(
                "applicability".to_string(),
                Value::String(safe_truncate(applicability.trim(), 220)),
            );
        }

        existing.confidence = merged_confidence;
        existing.support_count = existing.support_count.saturating_add(1);
        existing.metadata = Value::Object(metadata);
        existing.last_supported_at = Some(now.clone());
        existing.updated_at = now.clone();
        storage.upsert_experience_item(&existing).await?;
        lesson_id = existing.id;
        merged = true;
    } else {
        let normalized_key = build_reflected_heuristic_normalized_key(run, heuristic);
        let id = build_item_id(
            "lesson",
            scope,
            project_id,
            conversation_id,
            &normalized_key,
        );
        let created_at = chrono::Utc::now().to_rfc3339();
        storage
            .upsert_experience_item(&experience_item::Model {
                id: id.clone(),
                kind: "lesson".to_string(),
                scope: scope.to_string(),
                project_id: None,
                conversation_id: None,
                title: reflected_heuristic_title(run, &heuristic.polarity),
                content: safe_truncate(&heuristic.heuristic, 260),
                normalized_key,
                confidence: heuristic.confidence.clamp(0.35, 0.99),
                support_count: 1,
                contradiction_count: 0,
                status: "active".to_string(),
                metadata: json!({
                    "origin": HEURISTIC_REFLECTION_ORIGIN,
                    "source_run_id": run.id.clone(),
                    "intent_key": run.intent_key.clone(),
                    "task_type": current_task_type,
                    "tool_sequence_digest": run.tool_sequence_digest.clone(),
                    "polarity": heuristic.polarity.clone(),
                    "reflection_version": HEURISTIC_REFLECTION_VERSION,
                    "reflection_confidence": heuristic.confidence.clamp(0.0, 1.0),
                    "applicability": heuristic.applicability.as_deref().map(|value| safe_truncate(value.trim(), 220)),
                    "last_reflected_at": now.clone(),
                }),
                last_supported_at: Some(created_at.clone()),
                last_contradicted_at: None,
                created_at: created_at.clone(),
                updated_at: created_at,
                embedding: None,
            })
            .await?;
        lesson_id = id;
        merged = false;
    }

    let edge_now = chrono::Utc::now().to_rfc3339();
    storage
        .upsert_experience_edge(&experience_edge::Model {
            id: stable_id(
                "edge",
                &[run.id.as_str(), "derived_from", lesson_id.as_str()],
            ),
            source_ref: lesson_id.clone(),
            source_kind: "experience_item".to_string(),
            target_ref: run.id.clone(),
            target_kind: "experience_run".to_string(),
            edge_type: "derived_from".to_string(),
            weight: heuristic.confidence.clamp(0.25, 1.0),
            source_run_id: Some(run.id.clone()),
            metadata: json!({
                "intent_key": run.intent_key.clone(),
                "task_type": current_task_type,
                "heuristic_reflection": true,
                "merged": merged,
            }),
            created_at: edge_now.clone(),
            updated_at: edge_now,
        })
        .await?;

    Ok(ReflectedHeuristicPersistOutcome { lesson_id, merged })
}

fn canonical_memory_merge_signature(item: &experience_item::Model) -> Option<String> {
    if item.status != "active" {
        return None;
    }
    let content = item.content.trim();
    if content.is_empty() {
        return None;
    }
    let normalized = content
        .chars()
        .map(|ch| {
            if ch.is_ascii_alphanumeric() {
                ch.to_ascii_lowercase()
            } else {
                ' '
            }
        })
        .collect::<String>()
        .split_whitespace()
        .collect::<Vec<_>>()
        .join(" ");
    if normalized.len() < 16 {
        return None;
    }
    Some(format!(
        "{}::{}::{}::{}::{}",
        item.kind,
        item.scope,
        item.project_id.as_deref().unwrap_or(""),
        item.conversation_id.as_deref().unwrap_or(""),
        normalized
    ))
}

fn memory_merge_sort_key(item: &experience_item::Model) -> (i32, i32, String, String) {
    (
        item.support_count.max(0),
        (item.confidence * 1000.0) as i32,
        item.updated_at.clone(),
        item.id.clone(),
    )
}

fn memory_items_share_reconciliation_scope(
    left: &experience_item::Model,
    right: &experience_item::Model,
) -> bool {
    left.kind == right.kind
        && left.scope == right.scope
        && left.project_id == right.project_id
        && left.conversation_id == right.conversation_id
}

fn memory_pair_similarity(left: &experience_item::Model, right: &experience_item::Model) -> f32 {
    crate::core::document_search::normalized_embedding_similarity(
        left.embedding
            .as_ref()
            .map(|embedding| embedding.as_slice())
            .unwrap_or(&[]),
        right
            .embedding
            .as_ref()
            .map(|embedding| embedding.as_slice())
            .unwrap_or(&[]),
    )
    .unwrap_or(0.0)
    .clamp(0.0, 1.0)
}

fn metadata_marks_external_source(metadata: &Value) -> bool {
    let Some(object) = metadata.as_object() else {
        return false;
    };

    let contains_external_marker = |value: &str| {
        let lowered = value.trim().to_ascii_lowercase();
        !lowered.is_empty()
            && [
                "external-source",
                "community-learning",
                "learning-fenced",
                "moltbook",
            ]
            .iter()
            .any(|marker| lowered.contains(marker))
    };

    if object
        .get("external_source")
        .and_then(|value| value.as_bool())
        .unwrap_or(false)
        || object
            .get("fenced_external_source")
            .and_then(|value| value.as_bool())
            .unwrap_or(false)
        || object
            .get("learning_fenced")
            .and_then(|value| value.as_bool())
            .unwrap_or(false)
    {
        return true;
    }

    if object
        .get("source_scope")
        .and_then(|value| value.as_str())
        .is_some_and(|value| value.trim().eq_ignore_ascii_case("external"))
        || object
            .get("source_kind")
            .and_then(|value| value.as_str())
            .is_some_and(|value| value.trim().eq_ignore_ascii_case("external_source"))
    {
        return true;
    }

    if object
        .get("source")
        .and_then(|value| value.as_str())
        .is_some_and(contains_external_marker)
        || object
            .get("tags")
            .and_then(|value| value.as_str())
            .is_some_and(contains_external_marker)
    {
        return true;
    }

    object
        .get("tags")
        .and_then(|value| value.as_array())
        .is_some_and(|tags| {
            tags.iter()
                .filter_map(|value| value.as_str())
                .any(contains_external_marker)
        })
}

fn experience_item_is_external_source(item: &experience_item::Model) -> bool {
    metadata_marks_external_source(&item.metadata)
}

fn experience_item_learning_signal_bool(item: &experience_item::Model, key: &str) -> bool {
    item.metadata
        .get("learning_signal")
        .and_then(|value| value.get(key))
        .and_then(|value| value.as_bool())
        .unwrap_or(false)
}

fn procedure_item_is_pattern_eligible(item: &experience_item::Model) -> bool {
    experience_item_learning_signal_bool(item, "procedure_eligible") || item.support_count >= 2
}

fn procedural_pattern_is_external_source(pattern: &procedural_pattern::Model) -> bool {
    metadata_marks_external_source(&pattern.metadata)
}

fn canonical_json_string(value: &Value) -> String {
    match value {
        Value::Null => "null".to_string(),
        Value::Bool(value) => value.to_string(),
        Value::Number(value) => value.to_string(),
        Value::String(value) => serde_json::to_string(value).unwrap_or_else(|_| "\"\"".to_string()),
        Value::Array(items) => format!(
            "[{}]",
            items
                .iter()
                .map(canonical_json_string)
                .collect::<Vec<_>>()
                .join(",")
        ),
        Value::Object(map) => {
            let mut entries = map.iter().collect::<Vec<_>>();
            entries.sort_by(|left, right| left.0.cmp(right.0));
            format!(
                "{{{}}}",
                entries
                    .into_iter()
                    .map(|(key, value)| format!(
                        "{}:{}",
                        serde_json::to_string(key).unwrap_or_else(|_| "\"\"".to_string()),
                        canonical_json_string(value)
                    ))
                    .collect::<Vec<_>>()
                    .join(",")
            )
        }
    }
}

fn semantic_candidate_proposed_content(candidate: &learning_candidate::Model) -> Value {
    match candidate.candidate_type.as_str() {
        "strategy" => {
            let mut payload = candidate.proposed_content.clone();
            if let Some(object) = payload.as_object_mut() {
                object.remove("updated_at");
            }
            payload
        }
        _ => candidate.proposed_content.clone(),
    }
}

fn candidate_confidence_bucket(confidence: f64) -> String {
    let bounded = confidence.clamp(0.0, 1.0);
    let rounded = (bounded * 20.0).round() / 20.0;
    format!("{rounded:.2}")
}

fn learning_candidate_material_signature(candidate: &learning_candidate::Model) -> String {
    let pattern_id = candidate.pattern_id.as_deref().unwrap_or("");
    let evidence = canonical_json_string(&candidate.evidence_refs);
    let proposed = canonical_json_string(&semantic_candidate_proposed_content(candidate));
    let confidence = candidate_confidence_bucket(candidate.confidence);
    short_hash(&[
        candidate.candidate_type.as_str(),
        candidate.subject_key.as_str(),
        pattern_id,
        evidence.as_str(),
        proposed.as_str(),
        confidence.as_str(),
    ])
}

fn learning_candidate_revision_id(
    candidate: &learning_candidate::Model,
    signature: &str,
) -> String {
    stable_id(
        "candidate",
        &[
            candidate.candidate_type.as_str(),
            candidate.subject_key.as_str(),
            signature,
        ],
    )
}

fn learning_candidate_needs_refresh(
    existing: &learning_candidate::Model,
    candidate: &learning_candidate::Model,
) -> bool {
    existing.title != candidate.title
        || existing.summary != candidate.summary
        || existing.project_id != candidate.project_id
        || existing.conversation_id != candidate.conversation_id
        || existing.pattern_id != candidate.pattern_id
        || existing.evidence_refs != candidate.evidence_refs
        || semantic_candidate_proposed_content(existing)
            != semantic_candidate_proposed_content(candidate)
        || (existing.confidence - candidate.confidence).abs() > f64::EPSILON
}

fn learning_candidate_is_draft(status: &str) -> bool {
    status.eq_ignore_ascii_case("draft")
}

fn learning_candidate_is_reviewed(status: &str) -> bool {
    status.eq_ignore_ascii_case("approved") || status.eq_ignore_ascii_case("rejected")
}

fn ark_evolve_candidate_type_allowed(candidate_type: &str) -> bool {
    candidate_type.trim() != "skill_patch"
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum CandidateWriteOutcome {
    Unchanged,
    Changed,
    LeaseLost,
}

async fn supersede_stale_draft_candidates(
    storage: &Storage,
    lease_guard: &KvLeaseGuard,
    existing_candidates: &[learning_candidate::Model],
    keep_id: Option<&str>,
    reviewed_candidate_id: &str,
) -> Result<CandidateWriteOutcome> {
    let mut updated = 0usize;
    for draft in existing_candidates.iter().filter(|candidate| {
        learning_candidate_is_draft(&candidate.approval_status)
            && keep_id
                .map(|keep_id| candidate.id != keep_id)
                .unwrap_or(true)
    }) {
        let note = format!(
            "Auto-superseded because generation returned to reviewed candidate `{}`.",
            reviewed_candidate_id
        );
        if !storage
            .update_learning_candidate_review_guarded(
                LEARNING_CANDIDATE_GENERATION_LEASE_KEY,
                lease_guard,
                &draft.id,
                "superseded",
                Some(&note),
                None,
            )
            .await?
        {
            return Ok(CandidateWriteOutcome::LeaseLost);
        }
        updated += 1;
    }
    Ok(if updated > 0 {
        CandidateWriteOutcome::Changed
    } else {
        CandidateWriteOutcome::Unchanged
    })
}

async fn upsert_generated_learning_candidate(
    storage: &Storage,
    lease_guard: &KvLeaseGuard,
    mut candidate: learning_candidate::Model,
) -> Result<CandidateWriteOutcome> {
    if !ark_evolve_candidate_type_allowed(&candidate.candidate_type) {
        tracing::warn!(
            candidate_type = %candidate.candidate_type,
            subject_key = %candidate.subject_key,
            "Rejected Evolve learning candidate because skills are not a learning artifact"
        );
        return Ok(CandidateWriteOutcome::Unchanged);
    }

    let existing_candidates = storage
        .list_learning_candidates_for_subject(&candidate.candidate_type, &candidate.subject_key, 64)
        .await?;
    let material_signature = learning_candidate_material_signature(&candidate);

    if let Some(existing) = existing_candidates.iter().find(|existing| {
        learning_candidate_is_reviewed(&existing.approval_status)
            && learning_candidate_material_signature(existing) == material_signature
    }) {
        let superseded = supersede_stale_draft_candidates(
            storage,
            lease_guard,
            &existing_candidates,
            None,
            &existing.id,
        )
        .await?;
        return Ok(superseded);
    }

    if let Some(existing) = existing_candidates.iter().find(|existing| {
        learning_candidate_is_draft(&existing.approval_status)
            && learning_candidate_material_signature(existing) == material_signature
    }) {
        candidate.id = existing.id.clone();
        candidate.created_at = existing.created_at.clone();
        candidate.approval_status = existing.approval_status.clone();
        candidate.review_notes = existing.review_notes.clone();
        candidate.reviewed_at = existing.reviewed_at.clone();
        candidate.approved_ref = existing.approved_ref.clone();
        let superseded = if let Some(reviewed_candidate) = existing_candidates
            .iter()
            .find(|candidate| learning_candidate_is_reviewed(&candidate.approval_status))
        {
            match supersede_stale_draft_candidates(
                storage,
                lease_guard,
                &existing_candidates,
                Some(&existing.id),
                &reviewed_candidate.id,
            )
            .await?
            {
                CandidateWriteOutcome::LeaseLost => return Ok(CandidateWriteOutcome::LeaseLost),
                CandidateWriteOutcome::Changed => true,
                CandidateWriteOutcome::Unchanged => false,
            }
        } else {
            false
        };
        if !learning_candidate_needs_refresh(existing, &candidate) {
            return Ok(if superseded {
                CandidateWriteOutcome::Changed
            } else {
                CandidateWriteOutcome::Unchanged
            });
        }
        if !storage
            .upsert_learning_candidate_guarded(
                LEARNING_CANDIDATE_GENERATION_LEASE_KEY,
                lease_guard,
                &candidate,
            )
            .await?
        {
            return Ok(CandidateWriteOutcome::LeaseLost);
        }
        return Ok(CandidateWriteOutcome::Changed);
    }

    if existing_candidates.is_empty() {
        if !storage
            .upsert_learning_candidate_guarded(
                LEARNING_CANDIDATE_GENERATION_LEASE_KEY,
                lease_guard,
                &candidate,
            )
            .await?
        {
            return Ok(CandidateWriteOutcome::LeaseLost);
        }
        return Ok(CandidateWriteOutcome::Changed);
    }

    if let Some(existing_draft) = existing_candidates
        .iter()
        .find(|existing| learning_candidate_is_draft(&existing.approval_status))
    {
        candidate.id = existing_draft.id.clone();
        candidate.created_at = existing_draft.created_at.clone();
        candidate.approval_status = existing_draft.approval_status.clone();
        candidate.review_notes = existing_draft.review_notes.clone();
        candidate.reviewed_at = existing_draft.reviewed_at.clone();
        candidate.approved_ref = existing_draft.approved_ref.clone();
        if !storage
            .upsert_learning_candidate_guarded(
                LEARNING_CANDIDATE_GENERATION_LEASE_KEY,
                lease_guard,
                &candidate,
            )
            .await?
        {
            return Ok(CandidateWriteOutcome::LeaseLost);
        }
        if let Some(reviewed_candidate) = existing_candidates
            .iter()
            .find(|candidate| learning_candidate_is_reviewed(&candidate.approval_status))
        {
            if matches!(
                supersede_stale_draft_candidates(
                    storage,
                    lease_guard,
                    &existing_candidates,
                    Some(&existing_draft.id),
                    &reviewed_candidate.id,
                )
                .await?,
                CandidateWriteOutcome::LeaseLost
            ) {
                return Ok(CandidateWriteOutcome::LeaseLost);
            }
        }
        return Ok(CandidateWriteOutcome::Changed);
    }

    if let Some(reviewed_candidate) = existing_candidates
        .iter()
        .find(|existing| learning_candidate_is_reviewed(&existing.approval_status))
    {
        let note = format!(
            "Auto-reopened for review after material change from reviewed candidate `{}`.",
            reviewed_candidate.id
        );
        candidate.summary = Some(match candidate.summary.take() {
            Some(summary) if !summary.trim().is_empty() => format!("{summary} {note}"),
            _ => note,
        });
    }
    let revision_id = learning_candidate_revision_id(&candidate, &material_signature);
    candidate.id = if existing_candidates
        .iter()
        .any(|existing| existing.id == revision_id)
    {
        let reopen_marker = chrono::Utc::now().to_rfc3339();
        stable_id(
            "candidate",
            &[
                candidate.candidate_type.as_str(),
                candidate.subject_key.as_str(),
                material_signature.as_str(),
                reopen_marker.as_str(),
            ],
        )
    } else {
        revision_id
    };
    if !storage
        .upsert_learning_candidate_guarded(
            LEARNING_CANDIDATE_GENERATION_LEASE_KEY,
            lease_guard,
            &candidate,
        )
        .await?
    {
        return Ok(CandidateWriteOutcome::LeaseLost);
    }
    Ok(CandidateWriteOutcome::Changed)
}

fn apply_candidate_write_outcome(
    outcome: CandidateWriteOutcome,
    generated: &mut usize,
    lease_alive: &std::sync::Arc<std::sync::atomic::AtomicBool>,
) -> bool {
    match outcome {
        CandidateWriteOutcome::Changed => {
            *generated += 1;
            true
        }
        CandidateWriteOutcome::Unchanged => true,
        CandidateWriteOutcome::LeaseLost => {
            lease_alive.store(false, std::sync::atomic::Ordering::Relaxed);
            tracing::warn!("Stopping learning candidate generation after lease ownership was lost");
            false
        }
    }
}

pub async fn run_candidate_generation(storage: &Storage, _data_dir: &Path) -> Result<usize> {
    if !load_learning_enabled(storage).await {
        return Ok(0);
    }
    let lease_owner = uuid::Uuid::new_v4().to_string();
    let Some(lease_guard) = storage
        .acquire_kv_lease_guard(
            LEARNING_CANDIDATE_GENERATION_LEASE_KEY,
            &lease_owner,
            LEARNING_CANDIDATE_GENERATION_LEASE_TTL_SECS,
        )
        .await?
    else {
        return Ok(0);
    };
    let lease_alive = std::sync::Arc::new(std::sync::atomic::AtomicBool::new(true));
    let heartbeat_storage = storage.clone();
    let heartbeat_guard = lease_guard.clone();
    let heartbeat_lease_alive = lease_alive.clone();
    let (lease_stop_tx, mut lease_stop_rx) = tokio::sync::watch::channel(false);
    let lease_heartbeat = tokio::spawn(async move {
        loop {
            tokio::select! {
                changed = lease_stop_rx.changed() => {
                    if changed.is_err() || *lease_stop_rx.borrow() {
                        break;
                    }
                }
                _ = tokio::time::sleep(std::time::Duration::from_secs(
                    LEARNING_CANDIDATE_GENERATION_LEASE_HEARTBEAT_SECS,
                )) => {
                    match heartbeat_storage
                        .refresh_kv_lease_guard(
                            LEARNING_CANDIDATE_GENERATION_LEASE_KEY,
                            &heartbeat_guard,
                            LEARNING_CANDIDATE_GENERATION_LEASE_TTL_SECS,
                        )
                        .await
                    {
                        Ok(true) => {}
                        Ok(false) => {
                            heartbeat_lease_alive
                                .store(false, std::sync::atomic::Ordering::Relaxed);
                            tracing::warn!(
                                "Learning candidate generation lease heartbeat lost ownership"
                            );
                            break;
                        }
                        Err(error) => {
                            heartbeat_lease_alive
                                .store(false, std::sync::atomic::Ordering::Relaxed);
                            tracing::warn!(
                                "Learning candidate generation lease heartbeat refresh failed: {}",
                                error
                            );
                            break;
                        }
                    }
                }
            }
        }
    });

    let result = async {
        let cap = load_learning_queue_cap(storage).await as u64;
        let patterns = storage
            .list_candidate_ready_patterns(3, 0.66, cap)
            .await?
            .into_iter()
            .filter(|pattern| !procedural_pattern_is_external_source(pattern))
            .collect::<Vec<_>>();
        let mut generated = 0usize;
        generated += reconcile_memory_categories(storage, cap).await.unwrap_or_else(|error| {
            tracing::warn!("Memory category reconciliation skipped: {}", error);
            0
        });
        for pattern in patterns {
            if !lease_alive.load(std::sync::atomic::Ordering::Relaxed) {
                tracing::warn!(
                    "Stopping learning candidate generation after lease ownership was lost"
                );
                break;
            }

            let tool_names = pattern
                .tool_sequence_json
                .as_array()
                .map(|items| {
                    items
                        .iter()
                        .filter_map(|item| item.as_str().map(|value| value.to_string()))
                        .collect::<Vec<_>>()
                })
                .unwrap_or_default();

            let metadata = pattern.metadata.as_object().cloned().unwrap_or_default();
            let task_type = metadata
                .get("task_type")
                .and_then(|value| value.as_str())
                .unwrap_or("general");
            let strategy_profile =
                build_strategy_candidate_profile(&pattern, task_type, &tool_names);
            let strategy_candidate_id = stable_id("candidate", &["strategy", pattern.id.as_str()]);
            let now = chrono::Utc::now().to_rfc3339();
            if !apply_candidate_write_outcome(
                upsert_generated_learning_candidate(
                    storage,
                    &lease_guard,
                    learning_candidate::Model {
                        id: strategy_candidate_id,
                        candidate_type: "strategy".to_string(),
                        subject_key: pattern.id.clone(),
                        title: format!("Save a handling approach your agent uses: {}", pattern.title),
                        summary: Some(
                            "Your agent has found a repeatable approach for this kind of request. Save it so this becomes the default way it handles similar ones."
                                .to_string(),
                        ),
                        project_id: None,
                        conversation_id: None,
                        pattern_id: Some(pattern.id.clone()),
                        evidence_refs: json!([pattern.id]),
                        proposed_content: serde_json::to_value(strategy_profile).unwrap_or(Value::Null),
                        confidence: (pattern.success_rate * 0.92).min(0.98),
                        approval_status: "draft".to_string(),
                        review_notes: None,
                        reviewed_at: None,
                        approved_ref: None,
                        created_at: now.clone(),
                        updated_at: now.clone(),
                    },
                )
                .await?,
                &mut generated,
                &lease_alive,
            ) {
                break;
            }
        }

        let reflected_lessons = storage
            .list_active_experience_items_any_scope(&["lesson"], cap)
            .await?
            .into_iter()
            .filter(experience_item_is_reflected_heuristic)
            .collect::<Vec<_>>();
        let mut heuristic_groups: HashMap<String, Vec<experience_item::Model>> = HashMap::new();
        for lesson in reflected_lessons {
            if lesson.support_count < 2 || reflected_heuristic_confidence(&lesson) < 0.72 {
                continue;
            }
            let task_type = reflected_heuristic_task_type(&lesson).unwrap_or("general");
            let subject_key = format!("heuristic_strategy::global::{}", task_type);
            heuristic_groups.entry(subject_key).or_default().push(lesson);
        }
        for (subject_key, mut lessons) in heuristic_groups {
            if !lease_alive.load(std::sync::atomic::Ordering::Relaxed) {
                tracing::warn!(
                    "Stopping learning candidate generation after lease ownership was lost"
                );
                break;
            }
            lessons.sort_by(|left, right| {
                reflected_heuristic_confidence(right)
                    .total_cmp(&reflected_heuristic_confidence(left))
                    .then_with(|| right.support_count.cmp(&left.support_count))
                    .then_with(|| right.updated_at.cmp(&left.updated_at))
            });

            let mut selected = Vec::new();
            let mut seen = HashSet::new();
            for lesson in lessons {
                let dedupe_key = normalize_semantic_text(&lesson.content);
                if !seen.insert(dedupe_key) {
                    continue;
                }
                selected.push(lesson);
                if selected.len() >= 4 {
                    break;
                }
            }
            if selected.is_empty() {
                continue;
            }

            let task_type = reflected_heuristic_task_type(&selected[0]).unwrap_or("general");
            let avg_confidence = selected
                .iter()
                .map(reflected_heuristic_confidence)
                .sum::<f64>()
                / selected.len() as f64;
            let strategy_profile =
                build_reflected_heuristic_strategy_profile(task_type, &selected);
            let evidence_refs = Value::Array(
                selected
                    .iter()
                    .map(|lesson| Value::String(lesson.id.clone()))
                    .collect(),
            );
            let now = chrono::Utc::now().to_rfc3339();
            if !apply_candidate_write_outcome(
                upsert_generated_learning_candidate(
                    storage,
                    &lease_guard,
                    learning_candidate::Model {
                        id: stable_id("candidate", &["strategy", subject_key.as_str()]),
                        candidate_type: "strategy".to_string(),
                        subject_key: subject_key.clone(),
                        title: format!("Save a rule of thumb your agent learned for {} requests", task_type),
                        summary: Some(
                            "Your agent noticed a consistent pattern across similar requests. Save it so it applies by default on future ones like these."
                                .to_string(),
                        ),
                        project_id: None,
                        conversation_id: None,
                        pattern_id: None,
                        evidence_refs,
                        proposed_content: serde_json::to_value(strategy_profile)
                            .unwrap_or(Value::Null),
                        confidence: avg_confidence.clamp(0.45, 0.98),
                        approval_status: "draft".to_string(),
                        review_notes: None,
                        reviewed_at: None,
                        approved_ref: None,
                        created_at: now.clone(),
                        updated_at: now,
                    },
                )
                .await?,
                &mut generated,
                &lease_alive,
            ) {
                break;
            }
        }

        let at_risk_procedures = storage
            .list_active_experience_items_any_scope(&["procedure"], cap)
            .await?
            .into_iter()
            .filter(|item| !experience_item_is_external_source(item))
            .collect::<Vec<_>>();
        for item in at_risk_procedures
            .into_iter()
            .filter(|item| item.contradiction_count > item.support_count && item.status == "active")
        {
            if !lease_alive.load(std::sync::atomic::Ordering::Relaxed) {
                tracing::warn!(
                    "Stopping learning candidate generation after lease ownership was lost"
                );
                break;
            }

            let now = chrono::Utc::now().to_rfc3339();
            if !apply_candidate_write_outcome(
                upsert_generated_learning_candidate(
                    storage,
                    &lease_guard,
                    learning_candidate::Model {
                        id: stable_id("candidate", &["memory_deprecate", item.id.as_str()]),
                        candidate_type: "memory_deprecate".to_string(),
                        subject_key: item.id.clone(),
                        title: format!("Retire an outdated note: {}", item.title),
                        summary: Some(
                            "This note has been contradicted more often than it has been useful. Safe to retire so it stops influencing future requests."
                                .to_string(),
                        ),
                        project_id: None,
                        conversation_id: None,
                        pattern_id: None,
                        evidence_refs: json!([item.id]),
                        proposed_content: json!({
                            "item_id": item.id,
                            "next_status": "deprecated",
                        }),
                        confidence: 0.74,
                        approval_status: "draft".to_string(),
                        review_notes: None,
                        reviewed_at: None,
                        approved_ref: None,
                        created_at: now.clone(),
                        updated_at: now.clone(),
                    },
                )
                .await?,
                &mut generated,
                &lease_alive,
            ) {
                break;
            }
        }

        let mergeable_items = storage
            .list_active_experience_items_any_scope(
                &["constraint", "personal_fact", "lesson", "procedure"],
                cap,
            )
            .await?
            .into_iter()
            .filter(|item| !experience_item_is_external_source(item))
            .collect::<Vec<_>>();
        let mut merge_groups: std::collections::HashMap<String, Vec<experience_item::Model>> =
            std::collections::HashMap::new();
        for item in mergeable_items {
            let Some(signature) = canonical_memory_merge_signature(&item) else {
                continue;
            };
            merge_groups.entry(signature).or_default().push(item);
        }
        for group in merge_groups.into_values() {
            if !lease_alive.load(std::sync::atomic::Ordering::Relaxed) {
                tracing::warn!(
                    "Stopping learning candidate generation after lease ownership was lost"
                );
                break;
            }
            if group.len() < 2 {
                continue;
            }
            let mut sorted = group;
            sorted.sort_by_key(|item| std::cmp::Reverse(memory_merge_sort_key(item)));
            let target = sorted[0].clone();
            for source in sorted.into_iter().skip(1) {
                if !lease_alive.load(std::sync::atomic::Ordering::Relaxed) {
                    tracing::warn!(
                        "Stopping learning candidate generation after lease ownership was lost"
                    );
                    break;
                }
                if source.id == target.id {
                    continue;
                }
                let now = chrono::Utc::now().to_rfc3339();
                if !apply_candidate_write_outcome(
                    upsert_generated_learning_candidate(
                        storage,
                        &lease_guard,
                        learning_candidate::Model {
                            id: stable_id(
                                "candidate",
                                &["memory_merge", source.id.as_str(), target.id.as_str()],
                            ),
                            candidate_type: "memory_merge".to_string(),
                            subject_key: target.id.clone(),
                            title: format!("Combine a duplicate note into '{}'", target.title),
                            summary: Some(
                                "Two notes are saying substantially the same thing. Merging keeps things tidy without losing either version."
                                    .to_string(),
                            ),
                            project_id: None,
                            conversation_id: None,
                            pattern_id: None,
                            evidence_refs: json!([target.id, source.id]),
                            proposed_content: json!({
                                "target_item_id": target.id,
                                "source_item_id": source.id,
                                "reason": "duplicate_content",
                            }),
                            confidence: ((target.confidence + source.confidence) / 2.0).min(0.96),
                            approval_status: "draft".to_string(),
                            review_notes: None,
                            reviewed_at: None,
                            approved_ref: None,
                            created_at: now.clone(),
                            updated_at: now.clone(),
                        },
                    )
                    .await?,
                    &mut generated,
                    &lease_alive,
                ) {
                    break;
                }
            }
        }

        let semantic_mergeable_items = storage
            .list_active_experience_items_any_scope(
                &["constraint", "personal_fact"],
                cap.min(256),
            )
            .await?
            .into_iter()
            .filter(|item| !experience_item_is_external_source(item))
            .filter(|item| item.embedding.is_some())
            .collect::<Vec<_>>();
        let mut proposed_semantic_pairs = HashSet::new();
        'semantic_pairs: for (left_index, left) in semantic_mergeable_items.iter().enumerate() {
            for right in semantic_mergeable_items.iter().skip(left_index + 1) {
                if !lease_alive.load(std::sync::atomic::Ordering::Relaxed) {
                    tracing::warn!(
                        "Stopping learning candidate generation after lease ownership was lost"
                    );
                    break 'semantic_pairs;
                }
                if generated >= cap as usize {
                    break 'semantic_pairs;
                }
                if !memory_items_share_reconciliation_scope(left, right) {
                    continue;
                }
                if canonical_memory_merge_signature(left)
                    .zip(canonical_memory_merge_signature(right))
                    .is_some_and(|(left_sig, right_sig)| left_sig == right_sig)
                {
                    continue;
                }
                let similarity = memory_pair_similarity(left, right);
                if similarity < 0.94 {
                    continue;
                }
                let (target, source) =
                    if memory_merge_sort_key(left) >= memory_merge_sort_key(right) {
                        (left, right)
                    } else {
                        (right, left)
                    };
                let pair_key = if source.id < target.id {
                    format!("{}::{}", source.id, target.id)
                } else {
                    format!("{}::{}", target.id, source.id)
                };
                if !proposed_semantic_pairs.insert(pair_key) {
                    continue;
                }
                let now = chrono::Utc::now().to_rfc3339();
                if !apply_candidate_write_outcome(
                    upsert_generated_learning_candidate(
                        storage,
                        &lease_guard,
                        learning_candidate::Model {
                            id: stable_id(
                                "candidate",
                                &[
                                    "memory_semantic_merge",
                                    source.id.as_str(),
                                    target.id.as_str(),
                                ],
                            ),
                            candidate_type: "memory_merge".to_string(),
                            subject_key: target.id.clone(),
                            title: format!("Combine a near-duplicate note into '{}'", target.title),
                            summary: Some(
                                "Two notes are semantically very close. Review and merge them if keeping both would duplicate memory."
                                    .to_string(),
                            ),
                            project_id: None,
                            conversation_id: None,
                            pattern_id: None,
                            evidence_refs: json!([target.id.clone(), source.id.clone()]),
                            proposed_content: json!({
                                "target_item_id": target.id.clone(),
                                "source_item_id": source.id.clone(),
                                "reason": "semantic_near_duplicate",
                                "similarity": similarity,
                            }),
                            confidence: similarity as f64,
                            approval_status: "draft".to_string(),
                            review_notes: None,
                            reviewed_at: None,
                            approved_ref: None,
                            created_at: now.clone(),
                            updated_at: now.clone(),
                        },
                    )
                    .await?,
                    &mut generated,
                    &lease_alive,
                ) {
                    break 'semantic_pairs;
                }
            }
        }

        Ok(generated)
    }
    .await;
    let _ = lease_stop_tx.send(true);
    if let Err(error) = lease_heartbeat.await {
        tracing::warn!(
            "Learning candidate generation lease heartbeat join failed: {}",
            error
        );
    }

    if let Err(error) = storage
        .release_kv_lease_guard(LEARNING_CANDIDATE_GENERATION_LEASE_KEY, &lease_guard)
        .await
    {
        tracing::warn!(
            "Failed to release learning candidate generation lease: {}",
            error
        );
    }

    result
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn derive_intent_key_is_task_aware_and_stable() {
        let key = derive_intent_key(
            "Please fix the Rust bug in the tool execution flow",
            "coding",
        );
        assert!(key.starts_with("coding::"));
        assert!(key.contains("fix"));
        assert!(key.contains("rust"));
    }

    #[test]
    fn describe_user_preference_memory_maps_name_to_personal_fact() {
        let mapped = describe_user_preference_memory("user_name", "Ava")
            .expect("user_name should map to a personal fact");
        assert_eq!(mapped.0, "personal_fact");
        assert!(mapped.2.contains("Ava"));
    }

    #[test]
    fn ark_evolve_rejects_skill_patch_candidate_type() {
        assert!(!ark_evolve_candidate_type_allowed("skill_patch"));
        assert!(ark_evolve_candidate_type_allowed("strategy"));
        assert!(ark_evolve_candidate_type_allowed("memory_merge"));
    }

    #[test]
    fn canonical_memory_merge_signature_normalizes_equivalent_content() {
        let base = experience_item::Model {
            id: "item-1".to_string(),
            kind: "constraint".to_string(),
            scope: "global".to_string(),
            project_id: None,
            conversation_id: None,
            title: "Constraint".to_string(),
            content: "Require explicit approval before side-effecting actions.".to_string(),
            normalized_key: "constraint::one".to_string(),
            confidence: 0.9,
            support_count: 2,
            contradiction_count: 0,
            status: "active".to_string(),
            metadata: Value::Null,
            last_supported_at: None,
            last_contradicted_at: None,
            created_at: "2026-01-01T00:00:00Z".to_string(),
            updated_at: "2026-01-01T00:00:00Z".to_string(),
            embedding: None,
        };
        let mut variant = base.clone();
        variant.id = "item-2".to_string();
        variant.content = "Require explicit approval before side effecting actions".to_string();

        assert_eq!(
            canonical_memory_merge_signature(&base),
            canonical_memory_merge_signature(&variant)
        );
    }

    #[test]
    fn external_source_metadata_is_fenced_from_learning() {
        let item = experience_item::Model {
            id: "item-ext".to_string(),
            kind: "lesson".to_string(),
            scope: "global".to_string(),
            project_id: None,
            conversation_id: None,
            title: "External insight".to_string(),
            content: "Community learning".to_string(),
            normalized_key: "lesson::external".to_string(),
            confidence: 0.6,
            support_count: 1,
            contradiction_count: 0,
            status: "active".to_string(),
            metadata: serde_json::json!({
                "source": "moltbook",
                "tags": ["external-source", "community-learning", "learning-fenced"],
                "fenced_external_source": true
            }),
            last_supported_at: None,
            last_contradicted_at: None,
            created_at: "2026-01-01T00:00:00Z".to_string(),
            updated_at: "2026-01-01T00:00:00Z".to_string(),
            embedding: None,
        };
        assert!(experience_item_is_external_source(&item));
    }
}
