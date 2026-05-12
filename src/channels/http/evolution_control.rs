use super::*;
use anyhow::Context;

static GEPA_AUTO_LOOP_STARTED: AtomicBool = AtomicBool::new(false);
const GEPA_AUTO_INITIAL_DELAY_SECS: u64 = 90;
const GEPA_AUTO_POLL_SECS: u64 = 30 * 60;
const GEPA_AUTO_QUIET_WINDOW_SECS: i64 = 5 * 60;
const GEPA_AUTO_COOLDOWN_HOURS: i64 = 18;
const GEPA_AUTO_MIN_FRESH_EXPERIENCE_RUNS: usize = 6;
const GEPA_AUTO_EVIDENCE_SCAN_LIMIT: u64 = 160;

pub(super) fn resolve_project_root() -> PathBuf {
    let app_path = FsPath::new("/app");
    if app_path.join("Cargo.toml").exists() {
        return app_path.to_path_buf();
    }
    if let Ok(cwd) = std::env::current_dir() {
        let mut dir = cwd.as_path();
        loop {
            if dir.join("Cargo.toml").exists() {
                return dir.to_path_buf();
            }
            match dir.parent() {
                Some(parent) => dir = parent,
                None => break,
            }
        }
    }
    PathBuf::from(".")
}

pub(super) fn round4(value: f64) -> f64 {
    (value * 10_000.0).round() / 10_000.0
}

pub(super) fn compute_p95(mut values: Vec<i64>) -> Option<i64> {
    if values.is_empty() {
        return None;
    }
    values.sort_unstable();
    let idx = (((values.len() as f64) * 0.95).ceil() as usize)
        .saturating_sub(1)
        .min(values.len().saturating_sub(1));
    Some(values[idx])
}

pub(super) fn compute_percentile_usize(mut values: Vec<usize>, percentile: f64) -> usize {
    if values.is_empty() {
        return 0;
    }
    values.sort_unstable();
    let pct = percentile.clamp(0.0, 1.0);
    let idx = (((values.len() as f64) * pct).ceil() as usize)
        .saturating_sub(1)
        .min(values.len().saturating_sub(1));
    values[idx]
}

pub(super) fn average_usize(values: &[usize]) -> f64 {
    if values.is_empty() {
        0.0
    } else {
        round4(values.iter().sum::<usize>() as f64 / values.len() as f64)
    }
}

pub(super) async fn load_evolution_canary_state(
    storage: &crate::storage::Storage,
) -> Option<crate::core::self_evolve::strategy_runtime::CanaryRolloutState> {
    let raw = storage
        .get(crate::core::self_evolve::strategy_runtime::ROUTING_COMPLEXITY_CANARY_STATE_KEY)
        .await
        .ok()
        .flatten()?;
    serde_json::from_slice::<crate::core::self_evolve::strategy_runtime::CanaryRolloutState>(&raw)
        .ok()
}

pub(super) async fn load_prompt_evolution_canary_state(
    storage: &crate::storage::Storage,
) -> Option<crate::core::self_evolve::strategy_runtime::CanaryRolloutState> {
    load_canary_state_by_key(
        storage,
        crate::core::self_evolve::PROMPT_BUNDLE_CANARY_STATE_KEY,
    )
    .await
}

pub(super) async fn load_canary_state_by_key(
    storage: &crate::storage::Storage,
    key: &str,
) -> Option<crate::core::self_evolve::strategy_runtime::CanaryRolloutState> {
    let raw = storage.get(key).await.ok().flatten()?;
    serde_json::from_slice::<crate::core::self_evolve::strategy_runtime::CanaryRolloutState>(&raw)
        .ok()
}

pub(super) async fn load_tool_strategy_profile_by_key(
    storage: &crate::storage::Storage,
    key: &str,
) -> Option<crate::core::self_evolve::strategy_runtime::ToolStrategyProfile> {
    let raw = storage.get(key).await.ok().flatten()?;
    serde_json::from_slice::<crate::core::self_evolve::strategy_runtime::ToolStrategyProfile>(&raw)
        .ok()
}

pub(super) fn parse_tool_strategy_candidate_profile(
    candidate: &crate::storage::learning_candidate::Model,
) -> Result<crate::core::self_evolve::strategy_runtime::ToolStrategyProfile> {
    serde_json::from_value(candidate.proposed_content.clone()).map_err(|error| {
        anyhow::anyhow!(
            "Invalid strategy candidate payload for '{}': {}",
            candidate.id,
            error
        )
    })
}

pub(super) async fn disable_tool_strategy_canary_for_version(
    storage: &crate::storage::Storage,
    candidate_version: &str,
) -> Result<bool> {
    storage
        .disable_strategy_canary_for_version(candidate_version)
        .await
}

pub(super) async fn promote_tool_strategy_candidate_to_baseline(
    storage: &crate::storage::Storage,
    candidate: &crate::storage::learning_candidate::Model,
) -> Result<String> {
    storage
        .promote_strategy_learning_candidate_to_baseline(&candidate.id)
        .await
}

pub(super) async fn rollback_tool_strategy_baseline(
    storage: &crate::storage::Storage,
) -> Result<String> {
    storage.rollback_tool_strategy_baseline().await
}

pub(super) async fn load_last_self_evolve_result(
    storage: &crate::storage::Storage,
) -> Option<serde_json::Value> {
    let raw = storage
        .get(crate::core::self_evolve::strategy_runtime::SELF_EVOLVE_LAST_RESULT_KEY)
        .await
        .ok()
        .flatten()?;
    serde_json::from_slice::<serde_json::Value>(&raw).ok()
}

pub(super) async fn load_json_value_by_key(
    storage: &crate::storage::Storage,
    key: &str,
) -> Option<serde_json::Value> {
    let raw = storage.get(key).await.ok().flatten()?;
    serde_json::from_slice::<serde_json::Value>(&raw).ok()
}

fn replay_gate_reasons_from_json(
    replay: &serde_json::Map<String, serde_json::Value>,
) -> Vec<crate::core::self_evolve::PromotionGateReason> {
    replay
        .get("reasons")
        .and_then(|value| value.as_array())
        .map(|items| {
            items
                .iter()
                .filter_map(|item| {
                    let obj = item.as_object()?;
                    let code = obj
                        .get("code")
                        .and_then(|value| value.as_str())
                        .map(str::trim)
                        .filter(|value| !value.is_empty())?;
                    let label = obj
                        .get("label")
                        .and_then(|value| value.as_str())
                        .map(str::trim)
                        .filter(|value| !value.is_empty())?;
                    Some(crate::core::self_evolve::PromotionGateReason::new(
                        code, label,
                    ))
                })
                .collect()
        })
        .unwrap_or_default()
}

fn promotion_gate_summary_from_result(
    obj: &serde_json::Map<String, serde_json::Value>,
) -> Option<String> {
    obj.get("promotion_gate_report")
        .and_then(|value| value.as_object())
        .and_then(|report| report.get("summary"))
        .and_then(|value| value.as_str())
        .or_else(|| {
            obj.get("promotion_gate_summary")
                .and_then(|value| value.as_str())
        })
        .map(str::trim)
        .filter(|value| !value.is_empty())
        .map(str::to_string)
        .or_else(|| {
            obj.get("promotion_gate")
                .and_then(|value| value.as_str())
                .map(str::trim)
                .filter(|value| !value.is_empty())
                .map(str::to_string)
        })
}

pub(super) async fn load_live_policy_replay_evaluation(
    storage: &crate::storage::Storage,
    canary_state: Option<&crate::core::self_evolve::strategy_runtime::CanaryRolloutState>,
) -> Option<crate::core::self_evolve::strategy_runtime::ReplayEvaluationResult> {
    let state = canary_state.filter(|state| state.enabled)?;
    let logs = storage
        .list_operational_logs_by_event("tool_call", 4_000)
        .await
        .ok()?;
    Some(
        crate::core::self_evolve::strategy_runtime::evaluate_canary_by_policy_version(
            &logs,
            &state.baseline_version,
            &state.candidate_version,
            state.min_samples_per_version,
            state.min_success_gain,
            state.max_sign_test_p_value,
        ),
    )
}

pub(super) async fn load_live_prompt_replay_evaluation(
    storage: &crate::storage::Storage,
    prompt_canary_state: Option<&crate::core::self_evolve::strategy_runtime::CanaryRolloutState>,
) -> Option<crate::core::self_evolve::strategy_runtime::ReplayEvaluationResult> {
    let state = prompt_canary_state.filter(|state| state.enabled)?;
    let runs = storage
        .list_recent_experience_runs_any_scope(PROMPT_REPLAY_EVAL_SAMPLE_LIMIT)
        .await
        .ok()?;
    Some(
        crate::core::self_evolve::strategy_runtime::evaluate_experience_canary_by_prompt_version(
            &runs,
            &state.baseline_version,
            &state.candidate_version,
            state.min_samples_per_version,
            state.min_success_gain,
            state.max_sign_test_p_value,
        ),
    )
}

pub(super) async fn load_live_metadata_prompt_replay_evaluation(
    storage: &crate::storage::Storage,
    canary_state: Option<&crate::core::self_evolve::strategy_runtime::CanaryRolloutState>,
    metadata_key: &str,
) -> Option<crate::core::self_evolve::strategy_runtime::ReplayEvaluationResult> {
    let state = canary_state.filter(|state| state.enabled)?;
    let runs = storage
        .list_recent_experience_runs_any_scope(PROMPT_REPLAY_EVAL_SAMPLE_LIMIT)
        .await
        .ok()?;
    Some(
        crate::core::self_evolve::strategy_runtime::evaluate_experience_canary_by_metadata_version(
            &runs,
            metadata_key,
            &state.baseline_version,
            &state.candidate_version,
            state.min_samples_per_version,
            state.min_success_gain,
            state.max_sign_test_p_value,
        ),
    )
}

pub(super) async fn load_live_trace_prompt_telemetry_replay_evaluation(
    storage: &crate::storage::Storage,
    canary_state: Option<&crate::core::self_evolve::strategy_runtime::CanaryRolloutState>,
    metadata_key: &str,
) -> Option<crate::core::self_evolve::strategy_runtime::ReplayEvaluationResult> {
    let state = canary_state.filter(|state| state.enabled)?;
    let traces = storage
        .list_execution_trace_summaries(None, PROMPT_REPLAY_EVAL_SAMPLE_LIMIT, 0)
        .await
        .ok()?;
    Some(
        crate::core::self_evolve::strategy_runtime::evaluate_trace_prompt_telemetry_canary_by_version(
            &traces,
            metadata_key,
            &state.baseline_version,
            &state.candidate_version,
            state.min_samples_per_version,
            state.min_success_gain,
            state.max_sign_test_p_value,
        ),
    )
}

pub(super) async fn load_deploy_guard_default(storage: &crate::storage::Storage) -> bool {
    storage
        .get(crate::core::self_evolve::strategy_runtime::APP_DEPLOY_ACCESS_GUARD_DEFAULT_KEY)
        .await
        .ok()
        .flatten()
        .and_then(|raw| String::from_utf8(raw).ok())
        .map(|s| s.trim().eq_ignore_ascii_case("true"))
        .unwrap_or(false)
}

pub(super) async fn load_learning_enabled(storage: &crate::storage::Storage) -> bool {
    storage
        .get(crate::core::learning::LEARNING_ENABLED_KEY)
        .await
        .ok()
        .flatten()
        .and_then(|raw| String::from_utf8(raw).ok())
        .map(|s| !s.trim().eq_ignore_ascii_case("false"))
        .unwrap_or(true)
}

pub(super) async fn load_learning_model_slot(storage: &crate::storage::Storage) -> Option<String> {
    storage
        .get(crate::core::learning::LEARNING_MODEL_SLOT_KEY)
        .await
        .ok()
        .flatten()
        .and_then(|raw| String::from_utf8(raw).ok())
        .map(|value| value.trim().to_string())
        .filter(|value| !value.is_empty())
}

pub(super) async fn load_learning_queue_cap(storage: &crate::storage::Storage) -> u64 {
    storage
        .get(crate::core::learning::LEARNING_QUEUE_CAP_KEY)
        .await
        .ok()
        .flatten()
        .and_then(|raw| String::from_utf8(raw).ok())
        .and_then(|value| value.trim().parse::<u64>().ok())
        .filter(|value| *value > 0)
        .unwrap_or(64)
}

pub(super) fn bool_setting_bytes(enabled: bool) -> &'static [u8] {
    if enabled {
        b"true"
    } else {
        b"false"
    }
}

pub(super) async fn store_bool_setting(
    storage: &crate::storage::Storage,
    key: &str,
    enabled: bool,
) -> std::result::Result<(), String> {
    storage
        .set(key, bool_setting_bytes(enabled))
        .await
        .map_err(|error| error.to_string())
}

pub(super) async fn disable_canary_state_if_present(
    storage: &crate::storage::Storage,
    key: &str,
) -> std::result::Result<(), String> {
    let Some(raw) = storage.get(key).await.map_err(|error| error.to_string())? else {
        return Ok(());
    };
    let mut state = serde_json::from_slice::<
        crate::core::self_evolve::strategy_runtime::CanaryRolloutState,
    >(&raw)
    .map_err(|error| format!("Failed to parse canary state '{}': {}", key, error))?;
    if !state.enabled {
        return Ok(());
    }
    state.enabled = false;
    let encoded = serde_json::to_vec(&state)
        .map_err(|error| format!("Failed to encode canary state '{}': {}", key, error))?;
    storage
        .set(key, &encoded)
        .await
        .map_err(|error| format!("Failed to persist canary state '{}': {}", key, error))
}

pub(super) async fn disable_all_evolution_canaries(
    storage: &crate::storage::Storage,
) -> std::result::Result<(), String> {
    for key in [
        crate::core::self_evolve::strategy_runtime::ROUTING_COMPLEXITY_CANARY_STATE_KEY,
        crate::core::self_evolve::strategy_runtime::TOOL_STRATEGY_CANARY_STATE_KEY,
        crate::core::self_evolve::PROMPT_BUNDLE_CANARY_STATE_KEY,
        crate::core::self_evolve::SPECIALIST_PROMPT_BUNDLE_CANARY_STATE_KEY,
        crate::core::prompt_fragments::PROMPT_FRAGMENT_BUNDLE_CANARY_STATE_KEY,
    ] {
        disable_canary_state_if_present(storage, key).await?;
    }
    Ok(())
}

pub(super) fn build_learning_candidate_summary(
    candidate: &crate::storage::learning_candidate::Model,
    replay_gate: Option<&crate::core::self_evolve::replay_gate::CandidateReplayGateResult>,
    readiness: Option<&crate::core::DevelopmentalReadiness>,
) -> serde_json::Value {
    let proposed_name = candidate
        .proposed_content
        .get("name")
        .and_then(|value| value.as_str())
        .map(crate::core::self_evolve::skill_evolution::canonicalize_skill_name);
    let strategy_version = candidate
        .proposed_content
        .get("version")
        .and_then(|value| value.as_str())
        .map(|value| value.to_string());
    let preview = match candidate.candidate_type.as_str() {
        "workflow" => candidate
            .proposed_content
            .get("content")
            .and_then(|value| value.as_str())
            .map(|value| value.lines().take(4).collect::<Vec<_>>().join(" ")),
        "strategy" => candidate
            .proposed_content
            .get("default_guidance")
            .and_then(|value| value.as_array())
            .map(|items| {
                items
                    .iter()
                    .filter_map(|item| item.as_str())
                    .take(3)
                    .collect::<Vec<_>>()
                    .join(" | ")
            })
            .filter(|value| !value.is_empty()),
        "skill_patch" => candidate
            .proposed_content
            .get("diff_summary")
            .and_then(|value| value.as_str())
            .map(|value| value.to_string())
            .or_else(|| {
                candidate
                    .proposed_content
                    .get("after_content")
                    .and_then(|value| value.as_str())
                    .map(|value| value.lines().take(4).collect::<Vec<_>>().join(" "))
            }),
        crate::core::self_evolve::ROUTING_CANONICAL_CANDIDATE_TYPE => candidate
            .proposed_content
            .get("add")
            .and_then(|value| value.as_array())
            .map(|items| {
                items
                    .iter()
                    .take(3)
                    .filter_map(|item| {
                        let category = item.get("category")?.as_str()?.trim();
                        let concept = item.get("concept")?.as_str()?.trim();
                        if category.is_empty() || concept.is_empty() {
                            None
                        } else {
                            Some(format!("{category}:{concept}"))
                        }
                    })
                    .collect::<Vec<_>>()
                    .join(" | ")
            })
            .filter(|value| !value.is_empty()),
        crate::core::self_evolve::ROUTER_LEARNING_CANDIDATE_TYPE => {
            let layer = candidate
                .proposed_content
                .get("router_layer")
                .and_then(|value| value.as_str())
                .unwrap_or("router");
            let objective = candidate
                .proposed_content
                .get("objective")
                .and_then(|value| value.as_str())
                .unwrap_or("router learning candidate");
            let evidence_count = candidate
                .proposed_content
                .get("evidence")
                .and_then(|value| value.as_array())
                .map(Vec::len)
                .unwrap_or_default();
            Some(format!(
                "{}: {} ({} evidence item(s))",
                layer,
                truncate_candidate_preview(objective, 180),
                evidence_count
            ))
        }
        _ => serde_json::to_string(&candidate.proposed_content).ok(),
    };
    let skill_name = candidate
        .proposed_content
        .get("skill_name")
        .and_then(|value| value.as_str())
        .map(crate::core::self_evolve::skill_evolution::canonicalize_skill_name);
    let skill_action = candidate
        .proposed_content
        .get("action")
        .and_then(|value| value.as_str())
        .map(|value| value.to_string());
    let diff_summary = candidate
        .proposed_content
        .get("diff_summary")
        .and_then(|value| value.as_str())
        .map(|value| value.to_string());
    let proposed_content_preview = build_learning_candidate_content_preview(candidate);
    serde_json::json!({
        "id": candidate.id,
        "candidate_type": candidate.candidate_type,
        "subject_key": crate::core::self_evolve::skill_evolution::canonicalize_skill_name(&candidate.subject_key),
        "title": candidate.title,
        "summary": candidate.summary,
        "pattern_id": candidate.pattern_id,
        "confidence": candidate.confidence,
        "approval_status": candidate.approval_status,
        "updated_at": candidate.updated_at,
        "review_notes": candidate.review_notes,
        "reviewed_at": candidate.reviewed_at,
        "approved_ref": candidate.approved_ref,
        "evidence_refs": candidate.evidence_refs,
        "proposed_name": proposed_name,
        "strategy_version": strategy_version,
        "skill_name": skill_name,
        "skill_action": skill_action,
        "diff_summary": diff_summary,
        "preview": preview,
        "proposed_content_preview": proposed_content_preview,
        "replay_gate": replay_gate,
        "readiness": readiness,
    })
}

pub(super) fn truncate_candidate_preview(value: &str, max_chars: usize) -> String {
    let normalized = value.split_whitespace().collect::<Vec<_>>().join(" ");
    if normalized.chars().count() <= max_chars {
        return normalized;
    }
    let keep = max_chars.saturating_sub(3);
    let mut truncated = normalized.chars().take(keep).collect::<String>();
    truncated.push_str("...");
    truncated
}

pub(super) fn json_scalar_preview(value: &serde_json::Value, max_chars: usize) -> Option<String> {
    match value {
        serde_json::Value::String(text) => {
            let trimmed = text.trim();
            if trimmed.is_empty() {
                None
            } else {
                Some(truncate_candidate_preview(trimmed, max_chars))
            }
        }
        serde_json::Value::Number(number) => Some(number.to_string()),
        serde_json::Value::Bool(flag) => Some(flag.to_string()),
        _ => None,
    }
}

pub(super) fn json_string_array_preview(
    value: Option<&serde_json::Value>,
    max_items: usize,
    max_chars: usize,
) -> Vec<String> {
    value
        .and_then(|raw| raw.as_array())
        .map(|items| {
            items
                .iter()
                .filter_map(|item| item.as_str())
                .map(str::trim)
                .filter(|item| !item.is_empty())
                .take(max_items)
                .map(|item| truncate_candidate_preview(item, max_chars))
                .collect::<Vec<_>>()
        })
        .unwrap_or_default()
}

pub(super) fn build_learning_candidate_content_preview(
    candidate: &crate::storage::learning_candidate::Model,
) -> serde_json::Value {
    match candidate.candidate_type.as_str() {
        "strategy" => {
            let strategy_version = candidate
                .proposed_content
                .get("version")
                .and_then(|value| value.as_str())
                .map(str::trim)
                .filter(|value| !value.is_empty())
                .unwrap_or_default()
                .to_string();
            let default_guidance = json_string_array_preview(
                candidate.proposed_content.get("default_guidance"),
                3,
                140,
            );
            let task_guidance = candidate
                .proposed_content
                .get("task_guidance")
                .and_then(|value| value.as_object())
                .map(|entries| {
                    entries
                        .iter()
                        .take(3)
                        .flat_map(|(task_type, lines)| {
                            lines
                                .as_array()
                                .into_iter()
                                .flatten()
                                .filter_map(|item| item.as_str())
                                .map(str::trim)
                                .filter(|line| !line.is_empty())
                                .take(2)
                                .map(move |line| {
                                    format!(
                                        "{}: {}",
                                        task_type,
                                        truncate_candidate_preview(line, 140)
                                    )
                                })
                        })
                        .collect::<Vec<_>>()
                })
                .unwrap_or_default();
            serde_json::json!({
                "strategy_version": strategy_version,
                "default_guidance": default_guidance,
                "task_guidance": task_guidance,
            })
        }
        "memory_add" | "memory_update" | "memory_retract" => {
            let looks_sensitive = candidate
                .proposed_content
                .get("looks_sensitive")
                .and_then(|value| value.as_bool())
                .unwrap_or(false);
            let value_preview = if looks_sensitive {
                None
            } else {
                candidate
                    .proposed_content
                    .get("value")
                    .and_then(|value| json_scalar_preview(value, 180))
            };
            serde_json::json!({
                "operation_type": candidate.proposed_content.get("operation_type").and_then(|value| value.as_str()),
                "semantic_key": candidate.proposed_content.get("semantic_key").and_then(|value| value.as_str()),
                "value_preview": value_preview,
                "memory_kind": candidate.proposed_content.get("memory_kind").and_then(|value| value.as_str()),
                "scope": candidate.proposed_content.get("scope").and_then(|value| value.as_str()),
                "durability": candidate.proposed_content.get("durability").and_then(|value| value.as_str()),
                "looks_sensitive": looks_sensitive,
                "sensitive_reason": candidate.proposed_content.get("sensitive_reason").and_then(|value| value.as_str()),
            })
        }
        "memory_deprecate" => serde_json::json!({
            "item_id": candidate.proposed_content.get("item_id").and_then(|value| value.as_str()),
            "next_status": candidate.proposed_content.get("next_status").and_then(|value| value.as_str()),
        }),
        "memory_merge" => serde_json::json!({
            "target_item_id": candidate.proposed_content.get("target_item_id").and_then(|value| value.as_str()),
            "source_item_id": candidate.proposed_content.get("source_item_id").and_then(|value| value.as_str()),
            "reason": candidate.proposed_content.get("reason").and_then(|value| value.as_str()),
        }),
        crate::core::self_evolve::ROUTER_LEARNING_CANDIDATE_TYPE => {
            let evidence_count = candidate
                .proposed_content
                .get("evidence")
                .and_then(|value| value.as_array())
                .map(Vec::len)
                .unwrap_or_default();
            let metrics = candidate
                .proposed_content
                .get("metric_deltas")
                .and_then(|value| value.as_array())
                .map(|items| {
                    items
                        .iter()
                        .take(4)
                        .filter_map(|item| {
                            let metric = item.get("metric")?.as_str()?;
                            let delta = item.get("delta")?.as_f64()?;
                            Some(format!("{metric}: {delta:+.3}"))
                        })
                        .collect::<Vec<_>>()
                })
                .unwrap_or_default();
            serde_json::json!({
                "router_layer": candidate.proposed_content.get("router_layer").and_then(|value| value.as_str()),
                "objective": candidate.proposed_content.get("objective").and_then(|value| value.as_str()).map(|value| truncate_candidate_preview(value, 220)),
                "evidence_count": evidence_count,
                "metrics": metrics,
                "proposes_canonical": candidate.proposed_content.get("proposed_canonical_payload").is_some(),
                "proposes_action_descriptor": candidate.proposed_content.get("proposed_action_descriptor_patch").is_some(),
                "proposes_benchmark_entries": candidate.proposed_content.get("proposed_benchmark_entries").and_then(|value| value.as_array()).map(|items| !items.is_empty()).unwrap_or(false),
                "proposes_policy": candidate.proposed_content.get("proposed_policy_patch").is_some(),
                "proposes_capability_graph": candidate.proposed_content.get("proposed_capability_graph_patch").is_some(),
            })
        }
        _ => serde_json::Value::Null,
    }
}

pub(super) fn skill_patch_string(
    candidate: &crate::storage::learning_candidate::Model,
    key: &str,
) -> Option<String> {
    candidate
        .proposed_content
        .get(key)
        .and_then(|value| value.as_str())
        .map(str::trim)
        .filter(|value| !value.is_empty())
        .map(|value| value.to_string())
}

pub(super) fn compact_skill_metrics(metrics: &SkillMetricsSnapshot) -> String {
    format!(
        "{} matched, success {:.1}%, failure {:.1}%, tool errors {:.1}%",
        metrics.matched_runs,
        metrics.success_rate * 100.0,
        metrics.failure_rate * 100.0,
        metrics.tool_error_rate * 100.0
    )
}

pub(super) fn build_skill_candidate_evidence_markdown(
    candidate: &crate::storage::learning_candidate::Model,
) -> String {
    let action =
        skill_patch_string(candidate, "action").unwrap_or_else(|| "improve_skill".to_string());
    let skill_name = skill_patch_string(candidate, "skill_name")
        .unwrap_or_else(|| candidate.subject_key.clone());
    let diff_summary = skill_patch_string(candidate, "diff_summary").unwrap_or_else(|| {
        "Reviewable skill change generated from local session evidence.".to_string()
    });
    let baseline = candidate
        .proposed_content
        .get("impact_baseline")
        .cloned()
        .and_then(|value| serde_json::from_value::<SkillMetricsSnapshot>(value).ok())
        .unwrap_or_default();
    let evidence = candidate
        .proposed_content
        .get("evidence")
        .cloned()
        .unwrap_or(serde_json::Value::Null);
    let evidence_refs = candidate
        .evidence_refs
        .as_array()
        .map(|items| {
            items
                .iter()
                .filter_map(|item| item.as_str())
                .collect::<Vec<_>>()
                .join(", ")
        })
        .filter(|value| !value.is_empty())
        .unwrap_or_else(|| "none recorded".to_string());
    let history_versions_read = candidate
        .proposed_content
        .get("history_versions_read")
        .and_then(|value| value.as_u64())
        .unwrap_or(0);
    let selected_only_failures = evidence
        .get("selected_only_failures")
        .and_then(|value| value.as_u64())
        .unwrap_or(0);
    let executed_failures = evidence
        .get("executed_failures")
        .and_then(|value| value.as_u64())
        .unwrap_or(0);
    let recent_failure_reasons = evidence
        .get("recent_failure_reasons")
        .and_then(|value| value.as_array())
        .map(|items| {
            items
                .iter()
                .filter_map(|item| item.as_str())
                .collect::<Vec<_>>()
                .join(" | ")
        })
        .filter(|value| !value.is_empty())
        .unwrap_or_else(|| "none recorded".to_string());
    let recent_tool_errors = evidence
        .get("recent_tool_errors")
        .and_then(|value| value.as_array())
        .map(|items| {
            items
                .iter()
                .filter_map(|item| item.as_str())
                .collect::<Vec<_>>()
                .join(", ")
        })
        .filter(|value| !value.is_empty())
        .unwrap_or_else(|| "none recorded".to_string());
    format!(
        concat!(
            "# Skill evolution evidence\n\n",
            "## Decision summary\n",
            "- action: {action}\n",
            "- target skill: {skill_name}\n",
            "- why change is needed now: {diff_summary}\n\n",
            "## Session evidence\n",
            "- candidate id: {candidate_id}\n",
            "- evidence refs: {evidence_refs}\n",
            "- confidence: {confidence:.0}%\n",
            "- baseline window: {baseline_summary}\n",
            "- selected-only failures: {selected_only_failures}\n",
            "- executed failures: {executed_failures}\n",
            "- repeated failure notes: {recent_failure_reasons}\n",
            "- repeated tool errors: {recent_tool_errors}\n\n",
            "## Historical comparison\n",
            "- history entries read before applying: {history_versions_read}\n",
            "- previous skill content was snapshotted before this edit.\n\n",
            "## Edit plan\n",
            "- preserve the existing skill body except for the targeted diff proposed in this candidate.\n",
            "- write a versioned history snapshot before changing the live skill.\n\n",
            "## Open questions\n",
            "- monitor the first few post-approval runs before marking this skill as improved.\n"
        ),
        action = action,
        skill_name = skill_name,
        diff_summary = diff_summary,
        candidate_id = candidate.id,
        evidence_refs = evidence_refs,
        confidence = candidate.confidence * 100.0,
        baseline_summary = compact_skill_metrics(&baseline),
        selected_only_failures = selected_only_failures,
        executed_failures = executed_failures,
        recent_failure_reasons = recent_failure_reasons,
        recent_tool_errors = recent_tool_errors,
        history_versions_read = history_versions_read,
    )
}

pub(super) fn build_skill_evolution_entry(
    candidate: &crate::storage::learning_candidate::Model,
    recent_runs: &[crate::storage::entities::experience_run::Model],
    replay_gate: Option<&crate::core::self_evolve::replay_gate::CandidateReplayGateResult>,
    readiness: Option<&crate::core::DevelopmentalReadiness>,
) -> Option<serde_json::Value> {
    if candidate.candidate_type != "skill_patch" {
        return None;
    }
    let skill_name = skill_patch_string(candidate, "skill_name")
        .unwrap_or_else(|| candidate.subject_key.clone());
    let action =
        skill_patch_string(candidate, "action").unwrap_or_else(|| "improve_skill".to_string());
    let target_source =
        skill_patch_string(candidate, "target_source").unwrap_or_else(|| "custom".to_string());
    let before_content = skill_patch_string(candidate, "before_content").unwrap_or_default();
    let after_content = skill_patch_string(candidate, "after_content")
        .or_else(|| skill_patch_string(candidate, "content"))
        .unwrap_or_default();
    let diff_summary = skill_patch_string(candidate, "diff_summary").unwrap_or_default();
    let diff_preview = candidate
        .proposed_content
        .get("diff_preview")
        .cloned()
        .unwrap_or(serde_json::Value::Null);
    let evidence = candidate
        .proposed_content
        .get("evidence")
        .cloned()
        .unwrap_or(serde_json::Value::Null);
    let baseline = candidate
        .proposed_content
        .get("impact_baseline")
        .cloned()
        .and_then(|value| serde_json::from_value::<SkillMetricsSnapshot>(value).ok())
        .unwrap_or_default();
    let stored_status =
        skill_patch_string(candidate, "impact_status").unwrap_or_else(|| "pending".to_string());
    let (observed, assessment, impact_status) = if candidate.approval_status == "approved" {
        if let Some(reviewed_at) = candidate.reviewed_at.as_deref() {
            let observed = skill_evolution::compute_skill_metrics(
                recent_runs,
                &skill_name,
                Some(reviewed_at),
                SkillWindowDirection::Observed,
                8,
                64,
            );
            let assessment = skill_evolution::assess_skill_impact(&baseline, &observed);
            let status = assessment.status.clone();
            (observed, assessment, status)
        } else {
            let assessment = SkillImpactAssessment {
                status: "pending".to_string(),
                summary: vec!["Approved, but waiting for the first post-approval runs.".to_string()],
                ..SkillImpactAssessment::default()
            };
            (
                SkillMetricsSnapshot::default(),
                assessment,
                "pending".to_string(),
            )
        }
    } else {
        let assessment = SkillImpactAssessment {
            status: stored_status.clone(),
            summary: vec!["Waiting for review before impact tracking starts.".to_string()],
            ..SkillImpactAssessment::default()
        };
        (SkillMetricsSnapshot::default(), assessment, stored_status)
    };
    Some(serde_json::json!({
        "id": candidate.id,
        "candidate_type": candidate.candidate_type,
        "approval_status": candidate.approval_status,
        "title": candidate.title,
        "summary": candidate.summary,
        "skill_name": skill_name,
        "action": action,
        "target_source": target_source,
        "diff_summary": diff_summary,
        "diff_preview": diff_preview,
        "before_content": before_content,
        "after_content": after_content,
        "evidence": evidence,
        "impact_baseline": baseline,
        "impact_observed": observed,
        "impact_assessment": assessment,
        "impact_status": impact_status,
        "impact_delta": {
            "success_gain": assessment.success_gain,
            "failure_reduction": assessment.failure_reduction,
            "tool_error_reduction": assessment.tool_error_reduction,
        },
        "confidence": candidate.confidence,
        "approved_ref": candidate.approved_ref,
        "review_notes": candidate.review_notes,
        "reviewed_at": candidate.reviewed_at,
        "replay_gate": replay_gate,
        "readiness": readiness,
        "created_at": candidate.created_at,
        "updated_at": candidate.updated_at,
    }))
}

pub(super) fn build_experience_item_summary(
    item: &crate::storage::experience_item::Model,
) -> serde_json::Value {
    let suggested_steps = item
        .metadata
        .get("suggested_steps")
        .and_then(|value| value.as_array())
        .map(|items| {
            items
                .iter()
                .filter_map(|item| item.as_str())
                .take(3)
                .map(|value| value.to_string())
                .collect::<Vec<_>>()
        })
        .unwrap_or_default();
    let metadata = item.metadata.as_object().cloned().unwrap_or_default();
    serde_json::json!({
        "id": item.id,
        "kind": item.kind,
        "scope": "global",
        "title": item.title,
        "content": item.content,
        "confidence": item.confidence,
        "support_count": item.support_count,
        "contradiction_count": item.contradiction_count,
        "status": item.status,
        "conversation_id": serde_json::Value::Null,
        "updated_at": item.updated_at,
        "suggested_steps": suggested_steps,
        "intent_key": metadata.get("intent_key").cloned().unwrap_or(serde_json::Value::Null),
        "source": metadata.get("source").cloned().unwrap_or(serde_json::Value::Null),
        "origin": metadata.get("origin").cloned().unwrap_or(serde_json::Value::Null),
        "task_type": metadata.get("task_type").cloned().unwrap_or(serde_json::Value::Null),
        "polarity": metadata.get("polarity").cloned().unwrap_or(serde_json::Value::Null),
        "applicability": metadata.get("applicability").cloned().unwrap_or(serde_json::Value::Null),
        "reflection_confidence": metadata
            .get("reflection_confidence")
            .cloned()
            .unwrap_or(serde_json::Value::Null),
    })
}

fn experience_item_has_evolution_evidence(item: &crate::storage::experience_item::Model) -> bool {
    if item.kind != "procedure" {
        return true;
    }
    item.support_count >= 2
        || item
            .metadata
            .get("learning_signal")
            .and_then(|value| value.get("procedure_eligible"))
            .and_then(|value| value.as_bool())
            .unwrap_or(false)
}

pub(super) fn build_procedural_pattern_summary(
    pattern: &crate::storage::procedural_pattern::Model,
    readiness: Option<&crate::core::DevelopmentalReadiness>,
) -> serde_json::Value {
    let steps_preview = pattern
        .steps_json
        .as_array()
        .map(|items| {
            items
                .iter()
                .filter_map(|item| item.as_str())
                .take(4)
                .map(|value| value.to_string())
                .collect::<Vec<_>>()
        })
        .unwrap_or_default();
    let tool_sequence = pattern
        .tool_sequence_json
        .as_array()
        .map(|items| {
            items
                .iter()
                .filter_map(|item| item.as_str())
                .take(5)
                .map(|value| value.to_string())
                .collect::<Vec<_>>()
        })
        .unwrap_or_default();
    serde_json::json!({
        "id": pattern.id,
        "intent_key": pattern.intent_key,
        "scope": "global",
        "title": pattern.title,
        "trigger_summary": pattern.trigger_summary,
        "summary": pattern.summary,
        "sample_count": pattern.sample_count,
        "success_count": pattern.success_count,
        "correction_count": pattern.correction_count,
        "success_rate": pattern.success_rate,
        "status": pattern.status,
        "conversation_id": serde_json::Value::Null,
        "updated_at": pattern.updated_at,
        "last_validated_at": pattern.last_validated_at,
        "steps_preview": steps_preview,
        "tool_sequence": tool_sequence,
        "readiness": readiness,
    })
}

pub(super) fn experience_run_decision_event_payload<'a>(
    run: &'a crate::storage::experience_run::Model,
    event_key: &str,
) -> Option<&'a serde_json::Map<String, serde_json::Value>> {
    run.metadata
        .get("decision_episode")
        .and_then(|value| value.get(event_key))
        .and_then(|value| value.get("payload"))
        .and_then(|value| value.as_object())
}

pub(super) fn experience_run_decision_summary(
    run: &crate::storage::experience_run::Model,
) -> serde_json::Value {
    let turn_decision = run.metadata.get("turn_decision");
    let request_shape = experience_run_decision_event_payload(run, "request_shape");
    let action_selection = experience_run_decision_event_payload(run, "action_selection");
    let routing = experience_run_decision_event_payload(run, "routing");
    let tool_plan_validation = experience_run_decision_event_payload(run, "tool_plan_validation");
    let llm_decision = experience_run_decision_event_payload(run, "llm_decision");
    let outcome = experience_run_decision_event_payload(run, "outcome");

    serde_json::json!({
        "shape": request_shape
            .and_then(|payload| payload.get("shape"))
            .and_then(|value| value.as_str()),
        "execution_mode": request_shape
            .and_then(|payload| payload.get("execution_mode"))
            .and_then(|value| value.as_str()),
        "request_shape_confidence": request_shape
            .and_then(|payload| payload.get("confidence"))
            .and_then(|value| value.as_f64()),
        "preferred_actions": request_shape
            .and_then(|payload| payload.get("preferred_actions"))
            .cloned()
            .unwrap_or_else(|| serde_json::Value::Array(Vec::new())),
        "selected_actions": action_selection
            .and_then(|payload| payload.get("needed_actions"))
            .cloned()
            .unwrap_or_else(|| serde_json::Value::Array(Vec::new())),
        "should_clarify": action_selection
            .and_then(|payload| payload.get("should_clarify"))
            .and_then(|value| value.as_bool())
            .or_else(|| {
                tool_plan_validation
                    .and_then(|payload| payload.get("needs_clarification"))
                    .and_then(|value| value.as_bool())
            })
            .unwrap_or(false),
        "clarification_question": tool_plan_validation
            .and_then(|payload| payload.get("clarification_question"))
            .and_then(|value| value.as_str())
            .or_else(|| {
                action_selection
                    .and_then(|payload| payload.get("clarification_question"))
                    .and_then(|value| value.as_str())
            }),
        "reasoning": action_selection
            .and_then(|payload| payload.get("reasoning"))
            .and_then(|value| value.as_str())
            .or_else(|| {
                routing
                    .and_then(|payload| payload.get("reasoning"))
                    .and_then(|value| value.as_str())
            })
            .or_else(|| {
                tool_plan_validation
                    .and_then(|payload| payload.get("reasoning"))
                    .and_then(|value| value.as_str())
            }),
        "routing_complexity": routing
            .and_then(|payload| payload.get("complexity"))
            .and_then(|value| value.as_str()),
        "needs_delegation": routing
            .and_then(|payload| payload.get("needs_delegation"))
            .and_then(|value| value.as_bool())
            .unwrap_or(false),
        "routing_mode": routing
            .and_then(|payload| payload.get("mode"))
            .and_then(|value| value.as_str()),
        "llm_provider": llm_decision
            .and_then(|payload| payload.get("provider"))
            .and_then(|value| value.as_str()),
        "llm_model": llm_decision
            .and_then(|payload| payload.get("model"))
            .and_then(|value| value.as_str()),
        "completion_status": outcome
            .and_then(|payload| payload.get("status"))
            .and_then(|value| value.as_str())
            .unwrap_or(run.success_state.as_str()),
        "turn_decision_path": turn_decision
            .and_then(|value| value.get("path"))
            .and_then(|value| value.as_str()),
        "turn_decision_task_type": turn_decision
            .and_then(|value| value.get("task_type"))
            .and_then(|value| value.as_str()),
        "turn_decision_total_tokens": turn_decision
            .and_then(|value| value.get("usage_delta"))
            .and_then(|value| value.get("total_tokens"))
            .and_then(|value| value.as_i64()),
    })
}

pub(super) fn build_experience_run_summary(
    run: &crate::storage::experience_run::Model,
) -> serde_json::Value {
    let tool_names = run
        .tool_sequence_json
        .as_array()
        .map(|items| {
            items
                .iter()
                .filter_map(|item| item.get("tool_name").and_then(|value| value.as_str()))
                .take(6)
                .map(|value| value.to_string())
                .collect::<Vec<_>>()
        })
        .unwrap_or_default();
    serde_json::json!({
        "id": run.id,
        "execution_run_id": run.execution_run_id,
        "trace_id": run.trace_id,
        "scope": "global",
        "channel": run.channel,
        "intent_key": run.intent_key,
        "task_type": run.task_type,
        "request_text": run.request_text,
        "success_state": run.success_state,
        "correction_state": run.correction_state,
        "outcome_summary": run.outcome_summary,
        "failure_reason": run.failure_reason,
        "conversation_id": serde_json::Value::Null,
        "strategy_version": run.strategy_version,
        "policy_version": run.policy_version,
        "prompt_version": run.prompt_version,
        "specialist_prompt_version": crate::core::self_evolve::strategy_runtime::experience_run_metadata_version(run, "specialist_prompt_version"),
        "model_slot": run.model_slot,
        "consolidated": run.consolidated,
        "accepted_at": run.accepted_at,
        "corrected_at": run.corrected_at,
        "created_at": run.created_at,
        "updated_at": run.updated_at,
        "tool_names": tool_names,
        "decision_summary": experience_run_decision_summary(run),
        "decision_episode": run
            .metadata
            .get("decision_episode")
            .cloned()
            .unwrap_or(serde_json::Value::Null),
        "turn_decision": run
            .metadata
            .get("turn_decision")
            .cloned()
            .unwrap_or(serde_json::Value::Null),
        "prompt_telemetry": run
            .metadata
            .get("prompt_telemetry")
            .cloned()
            .unwrap_or(serde_json::Value::Null),
        "attempted_models": run
            .metadata
            .get("attempted_models")
            .cloned()
            .unwrap_or_else(|| serde_json::Value::Array(Vec::new())),
        "execution_status": run
            .metadata
            .get("execution_status")
            .cloned()
            .unwrap_or(serde_json::Value::Null),
    })
}

pub(super) fn build_experience_edge_summary(
    edge: &crate::storage::experience_edge::Model,
) -> serde_json::Value {
    serde_json::json!({
        "id": edge.id,
        "source": edge.source_ref,
        "source_kind": edge.source_kind,
        "target": edge.target_ref,
        "target_kind": edge.target_kind,
        "edge_type": edge.edge_type,
        "weight": edge.weight,
        "source_run_id": edge.source_run_id,
        "metadata": edge.metadata,
        "updated_at": edge.updated_at,
    })
}

fn push_experience_graph_node(
    nodes: &mut Vec<serde_json::Value>,
    refs: &mut Vec<String>,
    seen: &mut std::collections::HashSet<String>,
    id: String,
    kind: &str,
    label: String,
    status: Option<String>,
) {
    if id.trim().is_empty() || !seen.insert(id.clone()) {
        return;
    }
    refs.push(id.clone());
    nodes.push(serde_json::json!({
        "id": id,
        "kind": kind,
        "label": label,
        "status": status,
    }));
}

async fn build_experience_graph_summary(
    storage: &crate::storage::Storage,
    runs: &[crate::storage::experience_run::Model],
    items: &[crate::storage::experience_item::Model],
    patterns: &[crate::storage::procedural_pattern::Model],
    candidates: &[crate::storage::learning_candidate::Model],
) -> serde_json::Value {
    let mut seen = std::collections::HashSet::new();
    let mut refs = Vec::new();
    let mut nodes = Vec::new();
    for run in runs.iter().take(48) {
        push_experience_graph_node(
            &mut nodes,
            &mut refs,
            &mut seen,
            run.id.clone(),
            "experience_run",
            run.intent_key.clone(),
            Some(run.success_state.clone()),
        );
    }
    for item in items.iter().take(72) {
        push_experience_graph_node(
            &mut nodes,
            &mut refs,
            &mut seen,
            item.id.clone(),
            "experience_item",
            item.title.clone(),
            Some(item.status.clone()),
        );
    }
    for pattern in patterns.iter().take(48) {
        push_experience_graph_node(
            &mut nodes,
            &mut refs,
            &mut seen,
            pattern.id.clone(),
            "procedural_pattern",
            pattern.title.clone(),
            Some(pattern.status.clone()),
        );
    }
    for candidate in candidates.iter().take(48) {
        push_experience_graph_node(
            &mut nodes,
            &mut refs,
            &mut seen,
            candidate.id.clone(),
            "learning_candidate",
            candidate.title.clone(),
            Some(candidate.approval_status.clone()),
        );
    }
    let edge_rows = storage
        .list_experience_edges_for_refs(&refs, 240)
        .await
        .unwrap_or_default();
    for edge in &edge_rows {
        push_experience_graph_node(
            &mut nodes,
            &mut refs,
            &mut seen,
            edge.source_ref.clone(),
            &edge.source_kind,
            edge.source_ref.clone(),
            None,
        );
        push_experience_graph_node(
            &mut nodes,
            &mut refs,
            &mut seen,
            edge.target_ref.clone(),
            &edge.target_kind,
            edge.target_ref.clone(),
            None,
        );
    }
    let edges = edge_rows
        .into_iter()
        .map(|edge| build_experience_edge_summary(&edge))
        .collect::<Vec<_>>();
    let node_count = nodes.len();
    let edge_count = edges.len();
    serde_json::json!({
        "nodes": nodes,
        "edges": edges,
        "summary": {
            "nodes": node_count,
            "edges": edge_count,
            "global_learning": true,
        }
    })
}

pub(super) fn prompt_telemetry_usize(value: Option<&serde_json::Value>) -> Option<usize> {
    value.and_then(|entry| {
        entry.as_u64().map(|value| value as usize).or_else(|| {
            entry
                .as_i64()
                .filter(|value| *value >= 0)
                .map(|value| value as usize)
        })
    })
}

pub(super) fn prompt_telemetry_section_summary<'a>(
    summary: &'a PromptTelemetrySummary,
    section: &str,
) -> Option<&'a PromptTelemetrySectionSummary> {
    summary
        .top_sections
        .iter()
        .find(|item| item.section == section)
}

pub(super) fn prompt_section_coverage_label(
    section_chars: usize,
    total_chars: usize,
) -> Option<String> {
    if section_chars == 0 || total_chars == 0 {
        return None;
    }
    Some(format!(
        "This section accounts for about {:.1}% of the p95 final prompt size.",
        (section_chars as f64 / total_chars as f64) * 100.0
    ))
}

pub(super) fn format_char_count(value: usize) -> String {
    let digits = value.to_string();
    let mut grouped = String::with_capacity(digits.len() + digits.len() / 3);
    for (index, ch) in digits.chars().rev().enumerate() {
        if index > 0 && index % 3 == 0 {
            grouped.push(',');
        }
        grouped.push(ch);
    }
    let formatted = grouped.chars().rev().collect::<String>();
    format!("{} chars", formatted)
}

fn push_prompt_telemetry_sample(
    prompt_telemetry: &serde_json::Map<String, serde_json::Value>,
    success: Option<bool>,
    corrected: bool,
    final_prompt_chars: &mut Vec<usize>,
    tool_schema_chars: &mut Vec<usize>,
    estimated_total_request_chars: &mut Vec<usize>,
    tool_counts: &mut Vec<usize>,
    success_samples: &mut usize,
    corrected_samples: &mut usize,
    section_samples: &mut BTreeMap<String, Vec<usize>>,
) {
    if let Some(value) = prompt_telemetry_usize(prompt_telemetry.get("final_system_prompt_chars")) {
        final_prompt_chars.push(value);
    } else {
        return;
    }
    if let Some(value) = prompt_telemetry_usize(prompt_telemetry.get("tool_schema_chars")) {
        tool_schema_chars.push(value);
    }
    if let Some(value) =
        prompt_telemetry_usize(prompt_telemetry.get("estimated_total_request_chars"))
    {
        estimated_total_request_chars.push(value);
    }
    if let Some(value) = prompt_telemetry_usize(prompt_telemetry.get("tool_count")) {
        tool_counts.push(value);
    }
    if success.unwrap_or(false) {
        *success_samples = (*success_samples).saturating_add(1);
    }
    if corrected {
        *corrected_samples = (*corrected_samples).saturating_add(1);
    }
    if let Some(sections) = prompt_telemetry
        .get("sections")
        .and_then(|value| value.as_object())
    {
        for (section, value) in sections {
            if let Some(chars) = prompt_telemetry_usize(Some(value)) {
                section_samples
                    .entry(section.clone())
                    .or_default()
                    .push(chars);
            }
        }
    }
}

fn prompt_telemetry_samples_from_trace(
    trace: &crate::storage::ExecutionTraceSummaryRow,
) -> Vec<serde_json::Map<String, serde_json::Value>> {
    let steps = serde_json::from_str::<Vec<crate::core::ExecutionStep>>(&trace.steps_json)
        .unwrap_or_default();
    steps
        .into_iter()
        .filter_map(|step| {
            let data = step.data?;
            let value = serde_json::from_str::<serde_json::Value>(&data).ok()?;
            let object = value.as_object()?;
            let trace_kind = object
                .get("trace_kind")
                .and_then(|value| value.as_str())
                .map(str::trim);
            if trace_kind == Some("prompt_telemetry") {
                Some(object.clone())
            } else {
                None
            }
        })
        .collect()
}

fn trace_summary_has_error_step(trace: &crate::storage::ExecutionTraceSummaryRow) -> bool {
    serde_json::from_str::<Vec<crate::core::ExecutionStep>>(&trace.steps_json)
        .map(|steps| steps.iter().any(|step| step.step_type == "error"))
        .unwrap_or(false)
}

pub(super) fn aggregate_prompt_telemetry_summary_with_traces(
    runs: &[crate::storage::experience_run::Model],
    traces: &[crate::storage::ExecutionTraceSummaryRow],
) -> PromptTelemetrySummary {
    let mut final_prompt_chars = Vec::new();
    let mut tool_schema_chars = Vec::new();
    let mut estimated_total_request_chars = Vec::new();
    let mut tool_counts = Vec::new();
    let mut success_samples = 0usize;
    let mut corrected_samples = 0usize;
    let mut section_samples: BTreeMap<String, Vec<usize>> = BTreeMap::new();
    let mut experience_prompt_trace_ids = std::collections::HashSet::new();

    for run in runs {
        let Some(prompt_telemetry) = run
            .metadata
            .get("prompt_telemetry")
            .and_then(|value| value.as_object())
        else {
            continue;
        };
        if let Some(trace_id) = run
            .trace_id
            .as_deref()
            .map(str::trim)
            .filter(|value| !value.is_empty())
        {
            experience_prompt_trace_ids.insert(trace_id.to_string());
        }

        push_prompt_telemetry_sample(
            prompt_telemetry,
            Some(run.success_state != "failed"),
            run.correction_state == "corrected",
            &mut final_prompt_chars,
            &mut tool_schema_chars,
            &mut estimated_total_request_chars,
            &mut tool_counts,
            &mut success_samples,
            &mut corrected_samples,
            &mut section_samples,
        );
    }

    for trace in traces {
        if experience_prompt_trace_ids.contains(trace.id.as_str()) {
            continue;
        }
        let success = !trace_summary_has_error_step(trace);
        for prompt_telemetry in prompt_telemetry_samples_from_trace(trace) {
            push_prompt_telemetry_sample(
                &prompt_telemetry,
                Some(success),
                false,
                &mut final_prompt_chars,
                &mut tool_schema_chars,
                &mut estimated_total_request_chars,
                &mut tool_counts,
                &mut success_samples,
                &mut corrected_samples,
                &mut section_samples,
            );
        }
    }

    let sample_count = final_prompt_chars.len();
    let mut top_sections = section_samples
        .into_iter()
        .map(|(section, values)| PromptTelemetrySectionSummary {
            section,
            samples: values.len(),
            avg_chars: average_usize(&values),
            p50_chars: compute_percentile_usize(values.clone(), 0.50),
            p95_chars: compute_percentile_usize(values, 0.95),
        })
        .collect::<Vec<_>>();
    top_sections.sort_by(|left, right| {
        right
            .p95_chars
            .cmp(&left.p95_chars)
            .then(right.samples.cmp(&left.samples))
            .then(left.section.cmp(&right.section))
    });
    top_sections.truncate(12);

    PromptTelemetrySummary {
        sample_count,
        p50_final_prompt_chars: compute_percentile_usize(final_prompt_chars.clone(), 0.50),
        p95_final_prompt_chars: compute_percentile_usize(final_prompt_chars, 0.95),
        p50_tool_schema_chars: compute_percentile_usize(tool_schema_chars.clone(), 0.50),
        p95_tool_schema_chars: compute_percentile_usize(tool_schema_chars, 0.95),
        p50_estimated_total_request_chars: compute_percentile_usize(
            estimated_total_request_chars.clone(),
            0.50,
        ),
        p95_estimated_total_request_chars: compute_percentile_usize(
            estimated_total_request_chars,
            0.95,
        ),
        avg_tool_count: average_usize(&tool_counts),
        success_samples,
        corrected_samples,
        top_sections,
    }
}

pub(super) async fn load_prompt_optimization_review_state(
    storage: &crate::storage::Storage,
) -> PromptOptimizationReviewState {
    match storage.get(PROMPT_OPTIMIZATION_REVIEW_STATE_KEY).await {
        Ok(Some(raw)) => {
            serde_json::from_slice::<PromptOptimizationReviewState>(&raw).unwrap_or_default()
        }
        _ => PromptOptimizationReviewState::default(),
    }
}

pub(super) async fn update_prompt_optimization_review_state(
    storage: &crate::storage::Storage,
    proposal_id: &str,
    status: &str,
) -> Result<()> {
    let proposal_id = proposal_id.trim();
    let status = status.trim();
    if proposal_id.is_empty() || status.is_empty() {
        return Ok(());
    }

    let mut state = load_prompt_optimization_review_state(storage).await;
    state.insert(
        proposal_id.to_string(),
        PromptOptimizationReviewEntry {
            status: status.to_string(),
            reviewed_at: Some(chrono::Utc::now().to_rfc3339()),
        },
    );
    let bytes = serde_json::to_vec(&state)?;
    storage
        .set(PROMPT_OPTIMIZATION_REVIEW_STATE_KEY, &bytes)
        .await?;
    Ok(())
}

pub(super) async fn load_prompt_canary_safety_events(
    storage: &crate::storage::Storage,
) -> Vec<crate::core::self_evolve::strategy_runtime::PromptProfileCanarySafetyEvent> {
    match storage
        .get(crate::core::self_evolve::strategy_runtime::PROMPT_PROFILE_CANARY_SAFETY_EVENTS_KEY)
        .await
    {
        Ok(Some(raw)) => serde_json::from_slice::<
            Vec<crate::core::self_evolve::strategy_runtime::PromptProfileCanarySafetyEvent>,
        >(&raw)
        .unwrap_or_default(),
        _ => Vec::new(),
    }
}

pub(super) async fn store_prompt_canary_safety_events(
    storage: &crate::storage::Storage,
    events: &[crate::core::self_evolve::strategy_runtime::PromptProfileCanarySafetyEvent],
) -> Result<()> {
    let bytes = serde_json::to_vec(events)?;
    storage
        .set(
            crate::core::self_evolve::strategy_runtime::PROMPT_PROFILE_CANARY_SAFETY_EVENTS_KEY,
            &bytes,
        )
        .await?;
    Ok(())
}

pub(super) fn prompt_canary_state_key_for_surface(
    surface: &str,
) -> Option<(&'static str, &'static str)> {
    match surface.trim() {
        "prompt" => Some((
            crate::core::self_evolve::PROMPT_BUNDLE_CANARY_STATE_KEY,
            "Prompt bundle",
        )),
        "specialist_prompt" => Some((
            crate::core::self_evolve::SPECIALIST_PROMPT_BUNDLE_CANARY_STATE_KEY,
            "Specialist prompt bundle",
        )),
        "prompt_fragment" => Some((
            crate::core::prompt_fragments::PROMPT_FRAGMENT_BUNDLE_CANARY_STATE_KEY,
            "Prompt fragment bundle",
        )),
        _ => None,
    }
}

struct PromptRuntimeSurface {
    surface: &'static str,
    label: &'static str,
    profile_key: &'static str,
    canary_profile_key: &'static str,
    canary_state_key: &'static str,
    baseline_snapshot_key: &'static str,
    last_result_key: &'static str,
}

fn prompt_runtime_surface(surface: &str) -> Result<PromptRuntimeSurface> {
    match surface.trim().to_ascii_lowercase().as_str() {
        "prompt" | "main_prompt" | "prompt_bundle" => Ok(PromptRuntimeSurface {
            surface: "prompt",
            label: "main prompt bundle",
            profile_key: crate::core::self_evolve::PROMPT_BUNDLE_PROFILE_KEY,
            canary_profile_key: crate::core::self_evolve::PROMPT_BUNDLE_PROFILE_CANARY_KEY,
            canary_state_key: crate::core::self_evolve::PROMPT_BUNDLE_CANARY_STATE_KEY,
            baseline_snapshot_key: crate::core::self_evolve::PROMPT_BUNDLE_BASELINE_SNAPSHOT_KEY,
            last_result_key: crate::core::self_evolve::PROMPT_BUNDLE_LAST_RESULT_KEY,
        }),
        "specialist" | "specialist_prompt" | "specialist_prompt_bundle" => {
            Ok(PromptRuntimeSurface {
                surface: "specialist_prompt",
                label: "specialist prompt bundle",
                profile_key: crate::core::self_evolve::SPECIALIST_PROMPT_BUNDLE_PROFILE_KEY,
                canary_profile_key:
                    crate::core::self_evolve::SPECIALIST_PROMPT_BUNDLE_PROFILE_CANARY_KEY,
                canary_state_key:
                    crate::core::self_evolve::SPECIALIST_PROMPT_BUNDLE_CANARY_STATE_KEY,
                baseline_snapshot_key:
                    crate::core::self_evolve::SPECIALIST_PROMPT_BUNDLE_BASELINE_SNAPSHOT_KEY,
                last_result_key: crate::core::self_evolve::SPECIALIST_PROMPT_BUNDLE_LAST_RESULT_KEY,
            })
        }
        "prompt_fragment" | "prompt-fragment" | "prompt-fragments" | "prompt_fragment_bundle" => {
            Ok(PromptRuntimeSurface {
                surface: "prompt_fragment",
                label: "prompt fragment bundle",
                profile_key: crate::core::prompt_fragments::PROMPT_FRAGMENT_BUNDLE_PROFILE_KEY,
                canary_profile_key:
                    crate::core::prompt_fragments::PROMPT_FRAGMENT_BUNDLE_PROFILE_CANARY_KEY,
                canary_state_key:
                    crate::core::prompt_fragments::PROMPT_FRAGMENT_BUNDLE_CANARY_STATE_KEY,
                baseline_snapshot_key:
                    crate::core::prompt_fragments::PROMPT_FRAGMENT_BUNDLE_BASELINE_SNAPSHOT_KEY,
                last_result_key:
                    crate::core::prompt_fragments::PROMPT_FRAGMENT_BUNDLE_LAST_RESULT_KEY,
            })
        }
        other => anyhow::bail!("Unsupported prompt evolution surface '{}'.", other),
    }
}

fn default_prompt_surface_snapshot(surface: &PromptRuntimeSurface) -> Result<Vec<u8>> {
    match surface.surface {
        "prompt" => Ok(serde_json::to_vec(
            &crate::core::self_evolve::PromptBundleProfile::default(),
        )?),
        "specialist_prompt" => Ok(serde_json::to_vec(
            &crate::core::self_evolve::SpecialistPromptBundleProfile::default(),
        )?),
        "prompt_fragment" => Ok(serde_json::to_vec(
            &crate::core::prompt_fragments::default_prompt_fragment_bundle(),
        )?),
        other => anyhow::bail!("Unsupported prompt evolution surface '{}'.", other),
    }
}

async fn load_prompt_canary_state_for_surface(
    storage: &crate::storage::Storage,
    surface: &PromptRuntimeSurface,
) -> Result<crate::core::self_evolve::strategy_runtime::CanaryRolloutState> {
    let raw = storage
        .get(surface.canary_state_key)
        .await?
        .ok_or_else(|| anyhow::anyhow!("No {} canary state found.", surface.label))?;
    serde_json::from_slice(&raw)
        .with_context(|| format!("Stored {} canary state is unreadable.", surface.label))
}

async fn record_prompt_runtime_decision(
    storage: &crate::storage::Storage,
    surface: &PromptRuntimeSurface,
    decision: &str,
    state: Option<&crate::core::self_evolve::strategy_runtime::CanaryRolloutState>,
) {
    let Ok(Some(raw)) = storage.get(surface.last_result_key).await else {
        return;
    };
    let Ok(mut value) = serde_json::from_slice::<serde_json::Value>(&raw) else {
        return;
    };
    let Some(obj) = value.as_object_mut() else {
        return;
    };
    let now = chrono::Utc::now().to_rfc3339();
    obj.insert(
        "user_runtime_decision".to_string(),
        serde_json::json!(decision),
    );
    obj.insert("user_decision_at".to_string(), serde_json::json!(now));
    obj.insert("surface".to_string(), serde_json::json!(surface.surface));
    if let Some(state) = state {
        obj.insert(
            "canary_state".to_string(),
            serde_json::to_value(state).unwrap_or(serde_json::Value::Null),
        );
    }
    if decision == "accepted_stable" {
        obj.insert("promotion_applied".to_string(), serde_json::json!(true));
        obj.insert("promotion_mode".to_string(), serde_json::json!("baseline"));
        obj.insert("rollback_available".to_string(), serde_json::json!(true));
    } else if decision == "rolled_back" {
        obj.insert(
            "promotion_mode".to_string(),
            serde_json::json!("rolled_back"),
        );
        obj.insert("rollback_applied".to_string(), serde_json::json!(true));
    } else if decision == "stopped_canary" {
        obj.insert(
            "promotion_mode".to_string(),
            serde_json::json!("canary_stopped"),
        );
    }
    if let Ok(bytes) = serde_json::to_vec(&value) {
        let _ = storage.set(surface.last_result_key, &bytes).await;
    }
}

async fn disable_prompt_canary_for_surface(
    storage: &crate::storage::Storage,
    surface_name: &str,
) -> Result<String> {
    let surface = prompt_runtime_surface(surface_name)?;
    let mut state = load_prompt_canary_state_for_surface(storage, &surface).await?;
    if !state.enabled {
        anyhow::bail!("No active {} canary is running.", surface.label);
    }
    state.enabled = false;
    storage
        .set(surface.canary_state_key, &serde_json::to_vec(&state)?)
        .await?;
    record_prompt_runtime_decision(storage, &surface, "stopped_canary", Some(&state)).await;
    Ok(format!(
        "Stopped the {} live test for '{}'.",
        surface.label, state.candidate_version
    ))
}

async fn promote_prompt_canary_to_baseline(
    storage: &crate::storage::Storage,
    surface_name: &str,
) -> Result<String> {
    let surface = prompt_runtime_surface(surface_name)?;
    let mut state = load_prompt_canary_state_for_surface(storage, &surface).await?;
    if !state.enabled {
        anyhow::bail!("No active {} canary is running.", surface.label);
    }
    let candidate = storage
        .get(surface.canary_profile_key)
        .await?
        .ok_or_else(|| anyhow::anyhow!("No {} candidate bundle is available.", surface.label))?;
    let rollback_snapshot = match storage.get(surface.profile_key).await? {
        Some(current) => current,
        None => default_prompt_surface_snapshot(&surface)?,
    };
    storage
        .set(surface.baseline_snapshot_key, &rollback_snapshot)
        .await
        .with_context(|| format!("Failed to snapshot current {}.", surface.label))?;
    storage
        .set(surface.profile_key, &candidate)
        .await
        .with_context(|| format!("Failed to promote {}.", surface.label))?;
    state.enabled = false;
    state.baseline_version = state.candidate_version.clone();
    storage
        .set(surface.canary_state_key, &serde_json::to_vec(&state)?)
        .await?;
    record_prompt_runtime_decision(storage, &surface, "accepted_stable", Some(&state)).await;
    Ok(format!(
        "Accepted {} '{}' as stable. Rollback is available.",
        surface.label, state.baseline_version
    ))
}

async fn rollback_prompt_baseline_for_surface(
    storage: &crate::storage::Storage,
    surface_name: &str,
) -> Result<String> {
    let surface = prompt_runtime_surface(surface_name)?;
    let snapshot = storage
        .get(surface.baseline_snapshot_key)
        .await?
        .ok_or_else(|| anyhow::anyhow!("No {} rollback snapshot is available.", surface.label))?;
    storage
        .set(surface.profile_key, &snapshot)
        .await
        .with_context(|| format!("Failed to restore previous {}.", surface.label))?;
    let state = match load_prompt_canary_state_for_surface(storage, &surface).await {
        Ok(mut state) => {
            state.enabled = false;
            storage
                .set(surface.canary_state_key, &serde_json::to_vec(&state)?)
                .await?;
            Some(state)
        }
        Err(_) => None,
    };
    record_prompt_runtime_decision(storage, &surface, "rolled_back", state.as_ref()).await;
    Ok(format!(
        "Rolled back the {} to the previous stable snapshot.",
        surface.label
    ))
}

pub(super) async fn update_prompt_canary_safety_review_status(
    storage: &crate::storage::Storage,
    event_id: &str,
    review_status: &str,
) -> Result<crate::core::self_evolve::strategy_runtime::PromptProfileCanarySafetyEvent> {
    let event_id = event_id.trim();
    if event_id.is_empty() {
        anyhow::bail!("candidate_id is required for prompt canary safety review.");
    }

    let mut events = load_prompt_canary_safety_events(storage).await;
    let Some(event) = events.iter_mut().find(|item| item.id == event_id) else {
        anyhow::bail!("Prompt canary safety event not found.");
    };
    if event.review_status == "auto_reverted" {
        anyhow::bail!("This canary was already reverted automatically.");
    }
    event.review_status = review_status.trim().to_string();
    event.reviewed_at = Some(chrono::Utc::now().to_rfc3339());
    let updated = event.clone();
    store_prompt_canary_safety_events(storage, &events).await?;
    Ok(updated)
}

pub(super) async fn disable_prompt_canary_from_safety_event(
    storage: &crate::storage::Storage,
    event_id: &str,
) -> Result<crate::core::self_evolve::strategy_runtime::PromptProfileCanarySafetyEvent> {
    let events = load_prompt_canary_safety_events(storage).await;
    let event = events
        .iter()
        .find(|item| item.id == event_id)
        .cloned()
        .ok_or_else(|| anyhow::anyhow!("Prompt canary safety event not found."))?;
    if event.review_status == "auto_reverted" {
        anyhow::bail!("This canary was already reverted automatically.");
    }

    let (state_key, surface_label) = prompt_canary_state_key_for_surface(&event.surface)
        .ok_or_else(|| anyhow::anyhow!("Unsupported prompt canary surface '{}'.", event.surface))?;
    let mut state = load_canary_state_by_key(storage, state_key)
        .await
        .ok_or_else(|| anyhow::anyhow!("No active {} canary state found.", surface_label))?;
    if !state.enabled || state.candidate_version != event.candidate_version {
        anyhow::bail!(
            "No matching active {} canary is running for candidate '{}'.",
            surface_label,
            event.candidate_version
        );
    }
    state.enabled = false;
    let bytes = serde_json::to_vec(&state)?;
    storage.set(state_key, &bytes).await?;

    update_prompt_canary_safety_review_status(storage, event_id, "disabled_by_user").await
}

pub(super) fn build_prompt_optimization_proposal(
    review_state: &PromptOptimizationReviewState,
    id: &str,
    title: &str,
    summary: &str,
    evidence: Vec<String>,
    expected_benefit: Vec<String>,
    caveats: Vec<String>,
    risk_level: &str,
    target_scope: &str,
    change_preview: EvolutionChangePreview,
) -> PromptOptimizationProposal {
    let review_entry = review_state.get(id);
    let review_status = review_entry
        .map(|entry| entry.status.trim())
        .filter(|status| !status.is_empty())
        .unwrap_or("open")
        .to_string();
    let reviewed_at = review_entry.and_then(|entry| entry.reviewed_at.clone());

    PromptOptimizationProposal {
        id: id.to_string(),
        title: title.to_string(),
        summary: summary.to_string(),
        evidence,
        expected_benefit,
        caveats,
        risk_level: risk_level.to_string(),
        target_scope: target_scope.to_string(),
        review_status,
        reviewed_at,
        reversible: true,
        change_preview,
    }
}

pub(super) fn build_prompt_optimization_opportunities(
    summary: &PromptTelemetrySummary,
    review_state: &PromptOptimizationReviewState,
) -> Vec<PromptOptimizationProposal> {
    if summary.sample_count < 6 {
        return Vec::new();
    }

    let mut proposals = Vec::new();

    if let Some(section) = prompt_telemetry_section_summary(summary, "runtime_access_summary")
        .filter(|section| section.p95_chars >= 900)
    {
        let mut impact_estimate = vec![
            format!(
                "Could remove up to {} from eligible prompts before any broader prompt changes are needed.",
                format_char_count(section.p95_chars)
            ),
            format!(
                "Measured across {} sampled turn{}.",
                section.samples,
                if section.samples == 1 { "" } else { "s" }
            ),
        ];
        if let Some(line) =
            prompt_section_coverage_label(section.p95_chars, summary.p95_final_prompt_chars)
        {
            impact_estimate.push(line);
        }
        proposals.push(build_prompt_optimization_proposal(
            review_state,
            "prompt-opt-runtime-summary-compact",
            "Shorten setup instructions on simple chats",
            "Your agent reads a block of setup instructions every turn. On simple chats it doesn't really need them. Skipping it there makes those replies faster and cheaper.",
            vec![
                format!("Observed {} prompt-telemetry samples.", summary.sample_count),
                format!(
                    "p95 runtime access summary size is {} chars.",
                    section.p95_chars
                ),
                format!(
                    "p95 final prompt size is {} chars.",
                    summary.p95_final_prompt_chars
                ),
            ],
            vec![
                "Faster, cheaper replies on everyday chats.".to_string(),
                "Keeps the change reviewable because it targets prompt/profile configuration rather than runtime code.".to_string(),
            ],
            vec![
                "If we shorten too aggressively, your agent may get confused on turns that do need the setup details.".to_string(),
                "This should only ship behind manual review, canarying, and a reversible prompt/profile change.".to_string(),
            ],
            "low",
            "prompt_profile",
            EvolutionChangePreview {
                before: vec![
                    format!(
                        "Full runtime-access prose reaches {} at p95 on recent turns.",
                        format_char_count(section.p95_chars)
                    ),
                    format!(
                        "Overall final prompt size reaches {} at p95.",
                        format_char_count(summary.p95_final_prompt_chars)
                    ),
                ],
                after: vec![
                    "Offer a compact runtime-access profile on turns that do not depend on the full environment prose.".to_string(),
                    "Keep the full runtime-access profile on environment-dependent turns.".to_string(),
                ],
                impact_estimate,
            },
        ));
    }

    let action_catalog_section = prompt_telemetry_section_summary(summary, "action_catalog")
        .filter(|section| section.p95_chars >= 2500 || summary.p95_tool_schema_chars >= 12_000);
    if let Some(section) = action_catalog_section {
        let mut impact_estimate = vec![
            format!(
                "Up to {} of prompt weight is tied directly to the action catalog block at p95.",
                format_char_count(section.p95_chars)
            ),
            format!(
                "Schema-heavy turns already carry {} of serialized tool schema at p95.",
                format_char_count(summary.p95_tool_schema_chars)
            ),
        ];
        if let Some(line) =
            prompt_section_coverage_label(section.p95_chars, summary.p95_final_prompt_chars)
        {
            impact_estimate.push(line);
        }
        proposals.push(build_prompt_optimization_proposal(
            review_state,
            "prompt-opt-action-catalog-compact",
            "Simplify tool descriptions on tool-heavy chats",
            "Your agent sees a long technical description of every tool it can use. When many tools are available, that adds up. A shorter version speeds things up without removing any tools.",
            vec![
                format!("p95 action catalog size is {} chars.", section.p95_chars),
                format!(
                    "p95 serialized tool schema size is {} chars.",
                    summary.p95_tool_schema_chars
                ),
                format!("Average exposed tool count is {:.2}.", summary.avg_tool_count),
            ],
            vec![
                "Faster replies when your agent has many tools available.".to_string(),
                "Creates a controlled path for later canary tests instead of runtime self-modification.".to_string(),
            ],
            vec![
                "Shorter tool descriptions can occasionally make your agent pick a slightly wrong tool on rare actions.".to_string(),
                "Any change must preserve exact action availability and stay manually reviewable.".to_string(),
            ],
            "medium",
            "prompt_profile",
            EvolutionChangePreview {
                before: vec![
                    format!(
                        "The action catalog prompt section reaches {} at p95.",
                        format_char_count(section.p95_chars)
                    ),
                    format!(
                        "Recent turns serialize {} of tool schema at p95 across {:.2} exposed tools on average.",
                        format_char_count(summary.p95_tool_schema_chars),
                        summary.avg_tool_count
                    ),
                ],
                after: vec![
                    "Introduce an explicit compact tool-detail mode for the largest schema-heavy turns.".to_string(),
                    "Keep the full tool set available while shortening parameter detail where a compact profile is sufficient.".to_string(),
                ],
                impact_estimate,
            },
        ));
    }

    if let Some(section) = prompt_telemetry_section_summary(summary, "deployed_app_hint")
        .filter(|section| section.p95_chars >= 700)
    {
        let mut impact_estimate = vec![
            format!(
                "Could remove up to {} from non-app turns by scoping this hint more tightly.",
                format_char_count(section.p95_chars)
            ),
            format!(
                "Measured across {} sampled turn{} with prompt telemetry.",
                summary.sample_count,
                if summary.sample_count == 1 { "" } else { "s" }
            ),
        ];
        if let Some(line) =
            prompt_section_coverage_label(section.p95_chars, summary.p95_final_prompt_chars)
        {
            impact_estimate.push(line);
        }
        proposals.push(build_prompt_optimization_proposal(
            review_state,
            "prompt-opt-deployed-app-scope",
            "Only mention your deployed apps when the chat is about apps",
            "Your agent gets a list of your deployed apps on every turn, even when the chat has nothing to do with apps. Sharing that list only on app-related chats makes other replies faster.",
            vec![
                format!("p95 deployed app hint size is {} chars.", section.p95_chars),
                format!("Observed {} samples with prompt telemetry.", summary.sample_count),
            ],
            vec![
                "Replies stay fast as you deploy more apps.".to_string(),
                "Keeps the decision transparent because the scope rule would be reviewed explicitly.".to_string(),
            ],
            vec![
                "On short follow-up questions, your agent might momentarily forget which apps you have.".to_string(),
                "This should remain a reviewed prompt/profile policy, not a self-editing runtime rule.".to_string(),
            ],
            "medium",
            "prompt_profile",
            EvolutionChangePreview {
                before: vec![
                    format!(
                        "Deployed-app hints contribute {} at p95 even when the turn is not strongly app-focused.",
                        format_char_count(section.p95_chars)
                    ),
                    format!(
                        "Recent final prompts reach {} at p95.",
                        format_char_count(summary.p95_final_prompt_chars)
                    ),
                ],
                after: vec![
                    "Inject deployed-app inventory hints only on turns that are clearly app-focused.".to_string(),
                    "Keep the broader inventory available for app-related requests and follow-ups.".to_string(),
                ],
                impact_estimate,
            },
        ));
    }

    if let Some(section) = prompt_telemetry_section_summary(summary, "document_excerpts")
        .filter(|section| section.p95_chars >= 1500)
    {
        let mut impact_estimate = vec![
            format!(
                "Could remove up to {} from mixed-context turns that do not need the full excerpt set.",
                format_char_count(section.p95_chars)
            ),
            format!(
                "The largest recent requests reach {} at p95.",
                format_char_count(summary.p95_estimated_total_request_chars)
            ),
        ];
        if let Some(line) =
            prompt_section_coverage_label(section.p95_chars, summary.p95_final_prompt_chars)
        {
            impact_estimate.push(line);
        }
        proposals.push(build_prompt_optimization_proposal(
            review_state,
            "prompt-opt-document-context-guard",
            "Trim long document quotes when the chat drifts off-topic",
            "When you share documents, your agent pastes big excerpts into every reply — even when the chat drifts to other topics. Trimming them on non-document turns speeds things up without affecting document-focused work.",
            vec![
                format!("p95 document excerpt size is {} chars.", section.p95_chars),
                format!(
                    "p95 estimated total request size is {} chars.",
                    summary.p95_estimated_total_request_chars
                ),
            ],
            vec![
                "Faster replies on chats where documents aren't the main topic.".to_string(),
                "Creates evidence-backed guardrails before any future compaction policy is considered.".to_string(),
            ],
            vec![
                "On chats that still quietly rely on your documents, your agent might lose track of details.".to_string(),
                "Document-centric turns should stay protected, with human review before any rollout.".to_string(),
            ],
            "high",
            "prompt_profile",
            EvolutionChangePreview {
                before: vec![
                    format!(
                        "Document excerpts contribute {} at p95 on recent turns.",
                        format_char_count(section.p95_chars)
                    ),
                    format!(
                        "Estimated total request size reaches {} at p95.",
                        format_char_count(summary.p95_estimated_total_request_chars)
                    ),
                ],
                after: vec![
                    "Offer a compact document-context profile when the turn is not primarily document-driven.".to_string(),
                    "Keep document-centric turns on the full excerpt profile.".to_string(),
                ],
                impact_estimate,
            },
        ));
    }

    proposals
}

pub(super) fn normalize_evolution_dev_limit(limit: Option<u64>) -> u64 {
    limit
        .unwrap_or(EVOLUTION_DEV_DEFAULT_LIMIT)
        .clamp(24, EVOLUTION_DEV_MAX_LIMIT)
}

pub(super) async fn build_evolution_settings_response(
    storage: &crate::storage::Storage,
    agent_config: &crate::core::config::AgentConfig,
    primary_model_id: &str,
    project_root: &FsPath,
) -> EvolutionSettingsResponse {
    let canary_state = load_evolution_canary_state(storage).await;
    let strategy_canary_state = load_canary_state_by_key(
        storage,
        crate::core::self_evolve::strategy_runtime::TOOL_STRATEGY_CANARY_STATE_KEY,
    )
    .await;
    let last_result = load_last_self_evolve_result(storage).await;
    let prompt_canary_state = load_prompt_evolution_canary_state(storage).await;
    let specialist_prompt_canary_state = load_canary_state_by_key(
        storage,
        crate::core::self_evolve::SPECIALIST_PROMPT_BUNDLE_CANARY_STATE_KEY,
    )
    .await;
    let prompt_fragment_canary_state = load_canary_state_by_key(
        storage,
        crate::core::prompt_fragments::PROMPT_FRAGMENT_BUNDLE_CANARY_STATE_KEY,
    )
    .await;
    let prompt_last_result = load_json_value_by_key(
        storage,
        crate::core::self_evolve::PROMPT_BUNDLE_LAST_RESULT_KEY,
    )
    .await;
    let specialist_prompt_last_result = load_json_value_by_key(
        storage,
        crate::core::self_evolve::SPECIALIST_PROMPT_BUNDLE_LAST_RESULT_KEY,
    )
    .await;
    let prompt_fragment_last_result = load_json_value_by_key(
        storage,
        crate::core::prompt_fragments::PROMPT_FRAGMENT_BUNDLE_LAST_RESULT_KEY,
    )
    .await;

    let canary = if let Some(state) = canary_state.as_ref() {
        EvolutionCanarySummary {
            enabled: state.enabled,
            rollout_percent: state.rollout_percent,
            baseline_version: state.baseline_version.clone(),
            candidate_version: state.candidate_version.clone(),
        }
    } else {
        EvolutionCanarySummary {
            enabled: false,
            rollout_percent: 0,
            baseline_version: "routing-policy-default-v1".to_string(),
            candidate_version: "-".to_string(),
        }
    };
    let strategy_canary = if let Some(state) = strategy_canary_state.as_ref() {
        EvolutionCanarySummary {
            enabled: state.enabled,
            rollout_percent: state.rollout_percent,
            baseline_version: state.baseline_version.clone(),
            candidate_version: state.candidate_version.clone(),
        }
    } else {
        EvolutionCanarySummary {
            enabled: false,
            rollout_percent: 0,
            baseline_version: load_tool_strategy_profile_by_key(
                storage,
                crate::core::self_evolve::strategy_runtime::TOOL_STRATEGY_PROFILE_KEY,
            )
            .await
            .map(|profile| profile.version)
            .unwrap_or_else(|| "strategy-v1".to_string()),
            candidate_version: "-".to_string(),
        }
    };
    let prompt_canary = if let Some(state) = prompt_canary_state.as_ref() {
        EvolutionCanarySummary {
            enabled: state.enabled,
            rollout_percent: state.rollout_percent,
            baseline_version: state.baseline_version.clone(),
            candidate_version: state.candidate_version.clone(),
        }
    } else {
        EvolutionCanarySummary {
            enabled: false,
            rollout_percent: 0,
            baseline_version: crate::core::self_evolve::prompt_evolution::compose_prompt_version(
                "prompt-bundle-default-v1",
            ),
            candidate_version: "-".to_string(),
        }
    };
    let specialist_prompt_canary = if let Some(state) = specialist_prompt_canary_state.as_ref() {
        EvolutionCanarySummary {
            enabled: state.enabled,
            rollout_percent: state.rollout_percent,
            baseline_version: state.baseline_version.clone(),
            candidate_version: state.candidate_version.clone(),
        }
    } else {
        EvolutionCanarySummary {
            enabled: false,
            rollout_percent: 0,
            baseline_version:
                crate::core::self_evolve::specialist_prompt_evolution::compose_specialist_prompt_version(
                    "specialist-prompt-bundle-default-v1",
                ),
            candidate_version: "-".to_string(),
        }
    };
    let prompt_fragment_canary = if let Some(state) = prompt_fragment_canary_state.as_ref() {
        EvolutionCanarySummary {
            enabled: state.enabled,
            rollout_percent: state.rollout_percent,
            baseline_version: state.baseline_version.clone(),
            candidate_version: state.candidate_version.clone(),
        }
    } else {
        EvolutionCanarySummary {
            enabled: false,
            rollout_percent: 0,
            baseline_version: crate::core::prompt_fragments::compose_prompt_fragment_version(
                "prompt-fragments-default-v1",
            ),
            candidate_version: "-".to_string(),
        }
    };

    let mut replay_gate_result: Option<String> = None;
    let mut replay_gate_reasons: Vec<crate::core::self_evolve::PromotionGateReason> = Vec::new();
    let mut promotion_mode = if canary.enabled {
        "canary".to_string()
    } else {
        "none".to_string()
    };
    let mut prompt_replay_gate_result: Option<String> = None;
    let mut prompt_replay_gate_reasons: Vec<crate::core::self_evolve::PromotionGateReason> =
        Vec::new();
    let mut prompt_promotion_mode = if prompt_canary.enabled {
        "canary".to_string()
    } else {
        "none".to_string()
    };
    let mut specialist_prompt_replay_gate_result: Option<String> = None;
    let mut specialist_prompt_replay_gate_reasons: Vec<
        crate::core::self_evolve::PromotionGateReason,
    > = Vec::new();
    let mut specialist_prompt_promotion_mode = if specialist_prompt_canary.enabled {
        "canary".to_string()
    } else {
        "none".to_string()
    };
    let mut prompt_fragment_replay_gate_result: Option<String> = None;
    let mut prompt_fragment_replay_gate_reasons: Vec<
        crate::core::self_evolve::PromotionGateReason,
    > = Vec::new();
    let mut prompt_fragment_promotion_mode = if prompt_fragment_canary.enabled {
        "canary".to_string()
    } else {
        "none".to_string()
    };
    let last_promotion_result = if let Some(obj) = last_result.as_ref().and_then(|v| v.as_object())
    {
        if let Some(mode) = obj.get("promotion_mode").and_then(|v| v.as_str()) {
            if !mode.trim().is_empty() {
                promotion_mode = mode.to_string();
            }
        }
        if let Some(replay) = obj.get("replay_evaluation").and_then(|v| v.as_object()) {
            if replay
                .get("promote")
                .and_then(|v| v.as_bool())
                .unwrap_or(false)
            {
                replay_gate_result = Some("passed".to_string());
            } else if let Some(reason) = replay.get("reason").and_then(|v| v.as_str()) {
                replay_gate_result = Some(reason.to_string());
            }
            replay_gate_reasons = replay_gate_reasons_from_json(replay);
        }
        let promoted = obj
            .get("promoted")
            .and_then(|v| v.as_bool())
            .unwrap_or(false);
        let gate_summary = promotion_gate_summary_from_result(obj);
        if promoted {
            "Promoted candidate policy".to_string()
        } else if let Some(gate) = gate_summary.as_deref() {
            format!("Not promoted ({})", gate)
        } else {
            "Evolution completed".to_string()
        }
    } else {
        "No evolution runs yet".to_string()
    };
    let prompt_last_promotion_result =
        if let Some(obj) = prompt_last_result.as_ref().and_then(|v| v.as_object()) {
            if let Some(mode) = obj.get("promotion_mode").and_then(|v| v.as_str()) {
                if !mode.trim().is_empty() {
                    prompt_promotion_mode = mode.to_string();
                }
            }
            if let Some(replay) = obj.get("replay_evaluation").and_then(|v| v.as_object()) {
                if replay
                    .get("promote")
                    .and_then(|v| v.as_bool())
                    .unwrap_or(false)
                {
                    prompt_replay_gate_result = Some("passed".to_string());
                } else if let Some(reason) = replay.get("reason").and_then(|v| v.as_str()) {
                    prompt_replay_gate_result = Some(reason.to_string());
                }
                prompt_replay_gate_reasons = replay_gate_reasons_from_json(replay);
            }
            let promoted = obj
                .get("promoted")
                .and_then(|v| v.as_bool())
                .unwrap_or(false);
            let gate_summary = promotion_gate_summary_from_result(obj);
            if prompt_promotion_mode == "baseline" {
                "Promoted candidate prompt bundle".to_string()
            } else if prompt_promotion_mode == "canary" {
                "Activated candidate prompt bundle in canary".to_string()
            } else if promoted {
                "Candidate prompt bundle passed offline benchmark gate".to_string()
            } else if let Some(gate) = gate_summary.as_deref() {
                format!("Not promoted ({})", gate)
            } else {
                "Prompt evolution completed".to_string()
            }
        } else {
            "No prompt evolution runs yet".to_string()
        };
    let specialist_prompt_last_promotion_result = if let Some(obj) = specialist_prompt_last_result
        .as_ref()
        .and_then(|v| v.as_object())
    {
        if let Some(mode) = obj.get("promotion_mode").and_then(|v| v.as_str()) {
            if !mode.trim().is_empty() {
                specialist_prompt_promotion_mode = mode.to_string();
            }
        }
        if let Some(replay) = obj.get("replay_evaluation").and_then(|v| v.as_object()) {
            if replay
                .get("promote")
                .and_then(|v| v.as_bool())
                .unwrap_or(false)
            {
                specialist_prompt_replay_gate_result = Some("passed".to_string());
            } else if let Some(reason) = replay.get("reason").and_then(|v| v.as_str()) {
                specialist_prompt_replay_gate_result = Some(reason.to_string());
            }
            specialist_prompt_replay_gate_reasons = replay_gate_reasons_from_json(replay);
        }
        let promoted = obj
            .get("promoted")
            .and_then(|v| v.as_bool())
            .unwrap_or(false);
        let gate_summary = promotion_gate_summary_from_result(obj);
        if specialist_prompt_promotion_mode == "baseline" {
            "Promoted candidate specialist prompt bundle".to_string()
        } else if specialist_prompt_promotion_mode == "canary" {
            "Activated candidate specialist prompt bundle in canary".to_string()
        } else if promoted {
            "Candidate specialist prompt bundle passed offline benchmark gate".to_string()
        } else if let Some(gate) = gate_summary.as_deref() {
            format!("Not promoted ({})", gate)
        } else {
            "Specialist prompt evolution completed".to_string()
        }
    } else {
        "No specialist prompt evolution runs yet".to_string()
    };
    let prompt_fragment_last_promotion_result = if let Some(obj) = prompt_fragment_last_result
        .as_ref()
        .and_then(|v| v.as_object())
    {
        if let Some(mode) = obj.get("promotion_mode").and_then(|v| v.as_str()) {
            if !mode.trim().is_empty() {
                prompt_fragment_promotion_mode = mode.to_string();
            }
        }
        if let Some(replay) = obj.get("replay_evaluation").and_then(|v| v.as_object()) {
            if replay
                .get("promote")
                .and_then(|v| v.as_bool())
                .unwrap_or(false)
            {
                prompt_fragment_replay_gate_result = Some("passed".to_string());
            } else if let Some(reason) = replay.get("reason").and_then(|v| v.as_str()) {
                prompt_fragment_replay_gate_result = Some(reason.to_string());
            }
            prompt_fragment_replay_gate_reasons = replay_gate_reasons_from_json(replay);
        }
        let promoted = obj
            .get("promoted")
            .and_then(|v| v.as_bool())
            .unwrap_or(false);
        let gate_summary = promotion_gate_summary_from_result(obj);
        if prompt_fragment_promotion_mode == "baseline" {
            "Promoted candidate prompt fragment bundle".to_string()
        } else if prompt_fragment_promotion_mode == "canary" {
            "Activated candidate prompt fragment bundle in canary".to_string()
        } else if promoted {
            "Candidate prompt fragment bundle passed structural canary gate".to_string()
        } else if let Some(gate) = gate_summary.as_deref() {
            format!("Not promoted ({})", gate)
        } else {
            "Prompt fragment evolution completed".to_string()
        }
    } else {
        "No prompt fragment evolution runs yet".to_string()
    };
    if let Some(replay_eval) =
        load_live_policy_replay_evaluation(storage, canary_state.as_ref()).await
    {
        replay_gate_result = Some(if replay_eval.promote {
            "passed".to_string()
        } else {
            replay_eval.reason.clone()
        });
        replay_gate_reasons = replay_eval.reasons.clone();
    }
    if let Some(replay_eval) =
        load_live_prompt_replay_evaluation(storage, prompt_canary_state.as_ref()).await
    {
        prompt_replay_gate_result = Some(if replay_eval.promote {
            "passed".to_string()
        } else {
            replay_eval.reason.clone()
        });
        prompt_replay_gate_reasons = replay_eval.reasons.clone();
    }
    if let Some(replay_eval) = load_live_metadata_prompt_replay_evaluation(
        storage,
        specialist_prompt_canary_state.as_ref(),
        "specialist_prompt_version",
    )
    .await
    {
        specialist_prompt_replay_gate_result = Some(if replay_eval.promote {
            "passed".to_string()
        } else {
            replay_eval.reason.clone()
        });
        specialist_prompt_replay_gate_reasons = replay_eval.reasons.clone();
    }
    if let Some(replay_eval) = load_live_trace_prompt_telemetry_replay_evaluation(
        storage,
        prompt_fragment_canary_state.as_ref(),
        "prompt_fragment_version",
    )
    .await
    {
        prompt_fragment_replay_gate_result = Some(if replay_eval.promote {
            "passed".to_string()
        } else {
            replay_eval.reason.clone()
        });
        prompt_fragment_replay_gate_reasons = replay_eval.reasons.clone();
    }

    let learning_enabled = load_learning_enabled(storage).await;
    let self_evolve_enabled = storage
        .get(crate::core::self_evolve::strategy_runtime::SELF_EVOLVE_ENABLED_KEY)
        .await
        .ok()
        .flatten()
        .and_then(|raw| String::from_utf8(raw).ok())
        .map(|s| !s.trim().eq_ignore_ascii_case("false"))
        .unwrap_or(true)
        && learning_enabled;
    let learning_queue = storage.learning_queue_counts().await.unwrap_or_default();
    let readiness_policy = crate::core::readiness::load_readiness_policy(storage).await;
    let routing_rollback_available = storage
        .get(
            crate::core::self_evolve::strategy_runtime::ROUTING_COMPLEXITY_POLICY_BASELINE_SNAPSHOT_KEY,
        )
        .await
        .ok()
        .flatten()
        .is_some();
    let prompt_rollback_available = storage
        .get(crate::core::self_evolve::PROMPT_BUNDLE_BASELINE_SNAPSHOT_KEY)
        .await
        .ok()
        .flatten()
        .is_some();
    let specialist_prompt_rollback_available = storage
        .get(crate::core::self_evolve::SPECIALIST_PROMPT_BUNDLE_BASELINE_SNAPSHOT_KEY)
        .await
        .ok()
        .flatten()
        .is_some();
    let prompt_fragment_rollback_available = storage
        .get(crate::core::prompt_fragments::PROMPT_FRAGMENT_BUNDLE_BASELINE_SNAPSHOT_KEY)
        .await
        .ok()
        .flatten()
        .is_some();
    let gepa_config =
        crate::core::self_evolve::gepa_bridge::load_gepa_optimizer_config(storage).await;
    let gepa_auto_state =
        crate::core::self_evolve::gepa_bridge::load_gepa_auto_run_state(storage).await;
    let gepa_last_result =
        crate::core::self_evolve::gepa_bridge::load_gepa_last_result(storage).await;
    let gepa_readiness = crate::core::self_evolve::gepa_bridge::check_gepa_readiness(
        storage,
        project_root,
        agent_config,
        primary_model_id,
    )
    .await;
    let gepa_queue = match crate::core::self_evolve::gepa_bridge::queue_status_snapshot(
        project_root,
        12,
    )
    .await
    {
        Ok(value) => value,
        Err(error) => serde_json::json!({
            "status": "unavailable",
            "error": error.to_string(),
        }),
    };

    EvolutionSettingsResponse {
        self_evolve_enabled,
        learning_enabled,
        learning_model_slot: load_learning_model_slot(storage).await,
        learning_queue_cap: load_learning_queue_cap(storage).await,
        learning_queue,
        canary,
        strategy_canary,
        prompt_canary,
        specialist_prompt_canary,
        prompt_fragment_canary,
        last_promotion_result,
        replay_gate_result,
        replay_gate_reasons,
        promotion_mode,
        prompt_last_promotion_result,
        prompt_replay_gate_result,
        prompt_replay_gate_reasons,
        prompt_promotion_mode,
        specialist_prompt_last_promotion_result,
        specialist_prompt_replay_gate_result,
        specialist_prompt_replay_gate_reasons,
        specialist_prompt_promotion_mode,
        prompt_fragment_last_promotion_result,
        prompt_fragment_replay_gate_result,
        prompt_fragment_replay_gate_reasons,
        prompt_fragment_promotion_mode,
        routing_rollback_available,
        prompt_rollback_available,
        specialist_prompt_rollback_available,
        prompt_fragment_rollback_available,
        deploy_guard_default: load_deploy_guard_default(storage).await,
        readiness_policy,
        gepa_config,
        gepa_readiness,
        gepa_auto_state,
        gepa_last_result,
        gepa_queue,
    }
}

pub(super) fn aggregate_version_metrics(
    logs: &[crate::storage::OperationalLogVersionMetricRow],
    selector: impl Fn(&crate::storage::OperationalLogVersionMetricRow) -> Option<&str>,
) -> Vec<EvolutionVersionMetric> {
    let mut buckets: HashMap<String, Vec<&crate::storage::OperationalLogVersionMetricRow>> =
        HashMap::new();
    for row in logs {
        let Some(version) = selector(row).map(|v| v.trim()).filter(|v| !v.is_empty()) else {
            continue;
        };
        buckets.entry(version.to_string()).or_default().push(row);
    }

    let mut out = Vec::with_capacity(buckets.len());
    for (version, rows) in buckets {
        let samples = rows.len();
        if samples == 0 {
            continue;
        }
        let successes = rows.iter().filter(|row| row.success).count();
        let errors = samples.saturating_sub(successes);
        let latencies: Vec<i64> = rows.iter().filter_map(|row| row.latency_ms).collect();
        out.push(EvolutionVersionMetric {
            version,
            samples,
            success_rate: round4(successes as f64 / samples as f64),
            error_rate: round4(errors as f64 / samples as f64),
            p95_latency_ms: compute_p95(latencies),
        });
    }
    out.sort_by(|a, b| {
        b.samples
            .cmp(&a.samples)
            .then_with(|| a.version.cmp(&b.version))
    });
    out
}

pub(super) fn routing_policy_metric_fallback_version(
    canary_state: Option<&crate::core::self_evolve::strategy_runtime::CanaryRolloutState>,
) -> String {
    canary_state
        .map(|state| state.baseline_version.trim())
        .filter(|value| !value.is_empty())
        .unwrap_or("routing-policy-default-v1")
        .to_string()
}

pub(super) fn aggregate_policy_version_metrics(
    logs: &[crate::storage::OperationalLogVersionMetricRow],
    fallback_version: &str,
) -> Vec<EvolutionVersionMetric> {
    let metrics = aggregate_version_metrics(logs, |row| row.policy_version.as_deref());
    if !metrics.is_empty() {
        return metrics;
    }

    let fallback_version = fallback_version.trim();
    if fallback_version.is_empty() || logs.is_empty() {
        return Vec::new();
    }

    let fallback_logs = logs
        .iter()
        .cloned()
        .map(|mut row| {
            row.policy_version = Some(fallback_version.to_string());
            row
        })
        .collect::<Vec<_>>();
    aggregate_version_metrics(&fallback_logs, |row| row.policy_version.as_deref())
}

pub(super) fn aggregate_trace_policy_metrics(
    traces: &[crate::storage::ExecutionTraceSummaryRow],
    fallback_version: &str,
) -> Vec<EvolutionVersionMetric> {
    let fallback_version = fallback_version.trim();
    if fallback_version.is_empty() || traces.is_empty() {
        return Vec::new();
    }

    let samples = traces.len();
    let successes = traces
        .iter()
        .filter(|row| {
            row.completed_at
                .as_deref()
                .is_some_and(|value| !value.trim().is_empty())
        })
        .count();
    let errors = samples.saturating_sub(successes);
    let latencies = traces
        .iter()
        .filter_map(|row| row.duration_ms.map(i64::from))
        .collect::<Vec<_>>();

    vec![EvolutionVersionMetric {
        version: fallback_version.to_string(),
        samples,
        success_rate: round4(successes as f64 / samples as f64),
        error_rate: round4(errors as f64 / samples as f64),
        p95_latency_ms: compute_p95(latencies),
    }]
}

pub(super) fn parse_operational_payload(
    row: &crate::storage::entities::operational_log::Model,
) -> serde_json::Value {
    row.payload
        .as_deref()
        .and_then(|raw| serde_json::from_str::<serde_json::Value>(raw).ok())
        .unwrap_or(serde_json::Value::Null)
}

pub(super) async fn read_recent_jsonl(path_rel: &str, limit: usize) -> Vec<serde_json::Value> {
    let path = resolve_project_root().join(path_rel);
    let raw = match tokio::fs::read_to_string(&path).await {
        Ok(content) => content,
        Err(_) => return Vec::new(),
    };
    let mut parsed = Vec::new();
    for line in raw.lines() {
        if line.trim().is_empty() {
            continue;
        }
        if let Ok(value) = serde_json::from_str::<serde_json::Value>(line) {
            parsed.push(value);
        }
    }
    if parsed.len() <= limit {
        return parsed;
    }
    parsed.split_off(parsed.len().saturating_sub(limit))
}

pub(super) async fn read_recent_lineage(limit: usize) -> Vec<serde_json::Value> {
    read_recent_jsonl(ROUTING_POLICY_LINEAGE_REL_PATH, limit).await
}

pub(super) async fn read_recent_prompt_lineage(limit: usize) -> Vec<serde_json::Value> {
    read_recent_jsonl(PROMPT_BUNDLE_LINEAGE_REL_PATH, limit).await
}

pub(super) async fn read_recent_specialist_prompt_lineage(limit: usize) -> Vec<serde_json::Value> {
    read_recent_jsonl(SPECIALIST_PROMPT_BUNDLE_LINEAGE_REL_PATH, limit).await
}

pub(super) async fn read_recent_prompt_fragment_lineage(limit: usize) -> Vec<serde_json::Value> {
    read_recent_jsonl(PROMPT_FRAGMENT_BUNDLE_LINEAGE_REL_PATH, limit).await
}

pub(super) fn experience_run_resolved_for_prompt_metrics(
    row: &crate::storage::entities::experience_run::Model,
) -> bool {
    row.correction_state == "corrected"
        || row.success_state == "accepted"
        || row.success_state == "failed"
}

pub(super) fn experience_run_success_for_prompt_metrics(
    row: &crate::storage::entities::experience_run::Model,
) -> bool {
    row.correction_state != "corrected" && row.success_state == "accepted"
}

pub(super) fn operational_payload_string(
    row: &crate::storage::entities::operational_log::Model,
    key: &str,
) -> Option<String> {
    row.payload
        .as_deref()
        .and_then(|raw| serde_json::from_str::<serde_json::Value>(raw).ok())
        .and_then(|payload| {
            payload
                .get(key)
                .and_then(|value| value.as_str())
                .map(str::trim)
                .filter(|value| !value.is_empty())
                .map(str::to_string)
        })
}

pub(super) fn aggregate_bundle_metrics_by_selectors(
    experience_runs: &[crate::storage::experience_run::Model],
    tool_logs: &[crate::storage::entities::operational_log::Model],
    routing_logs: &[crate::storage::entities::operational_log::Model],
    llm_logs: &[crate::storage::entities::operational_log::Model],
    experience_version: impl Fn(&crate::storage::experience_run::Model) -> Option<String>,
    operational_version: impl Fn(&crate::storage::entities::operational_log::Model) -> Option<String>,
) -> Vec<PromptEvolutionMetric> {
    let mut versions = std::collections::BTreeSet::new();
    for run in experience_runs {
        if let Some(version) = experience_version(run) {
            versions.insert(version);
        }
    }
    for row in tool_logs {
        if let Some(version) = operational_version(row) {
            versions.insert(version);
        }
    }
    for row in routing_logs {
        if let Some(version) = operational_version(row) {
            versions.insert(version);
        }
    }
    for row in llm_logs {
        if let Some(version) = operational_version(row) {
            versions.insert(version);
        }
    }

    let mut out = Vec::new();
    for version in versions {
        let experience_rows = experience_runs
            .iter()
            .filter(|run| experience_version(run).as_deref() == Some(version.as_str()))
            .collect::<Vec<_>>();
        let resolved_experience_rows = experience_rows
            .iter()
            .copied()
            .filter(|run| experience_run_resolved_for_prompt_metrics(run))
            .collect::<Vec<_>>();
        let tool_rows = tool_logs
            .iter()
            .filter(|row| operational_version(row).as_deref() == Some(version.as_str()))
            .collect::<Vec<_>>();
        let routing_rows = routing_logs
            .iter()
            .filter(|row| operational_version(row).as_deref() == Some(version.as_str()))
            .collect::<Vec<_>>();
        let llm_rows = llm_logs
            .iter()
            .filter(|row| operational_version(row).as_deref() == Some(version.as_str()))
            .collect::<Vec<_>>();

        let samples = resolved_experience_rows.len();
        let successes = resolved_experience_rows
            .iter()
            .filter(|run| experience_run_success_for_prompt_metrics(run))
            .count();
        let errors = samples.saturating_sub(successes);
        let latencies = tool_rows
            .iter()
            .filter_map(|row| row.latency_ms)
            .collect::<Vec<_>>();
        let routing_decisions = routing_rows.len();
        let delegation_count = routing_rows
            .iter()
            .filter(|row| {
                parse_operational_payload(row)
                    .get("needs_delegation")
                    .and_then(|value| value.as_bool())
                    .unwrap_or(false)
            })
            .count();
        let clarification_count = routing_rows
            .iter()
            .filter(|row| {
                parse_operational_payload(row)
                    .get("should_clarify")
                    .and_then(|value| value.as_bool())
                    .unwrap_or(false)
            })
            .count();
        let tool_calls_per_request = {
            let total_tool_calls = llm_rows
                .iter()
                .map(|row| {
                    parse_operational_payload(row)
                        .get("tool_calls")
                        .and_then(|value| value.as_u64())
                        .unwrap_or(0)
                })
                .sum::<u64>();
            if llm_rows.is_empty() {
                0.0
            } else {
                total_tool_calls as f64 / llm_rows.len() as f64
            }
        };
        let tool_success_rate = if tool_rows.is_empty() {
            0.0
        } else {
            round4(
                tool_rows.iter().filter(|row| row.success).count() as f64 / tool_rows.len() as f64,
            )
        };

        out.push(PromptEvolutionMetric {
            version,
            samples,
            success_rate: if samples == 0 {
                0.0
            } else {
                round4(successes as f64 / samples as f64)
            },
            error_rate: if samples == 0 {
                0.0
            } else {
                round4(errors as f64 / samples as f64)
            },
            p95_latency_ms: compute_p95(latencies),
            routing_decisions,
            delegation_rate: if routing_decisions == 0 {
                0.0
            } else {
                round4(delegation_count as f64 / routing_decisions as f64)
            },
            clarification_rate: if routing_decisions == 0 {
                0.0
            } else {
                round4(clarification_count as f64 / routing_decisions as f64)
            },
            avg_tool_calls_per_request: round4(tool_calls_per_request),
            tool_success_rate,
        });
    }

    out.sort_by(|a, b| {
        b.samples
            .cmp(&a.samples)
            .then_with(|| a.version.cmp(&b.version))
    });
    out
}

pub(super) fn aggregate_prompt_metrics(
    experience_runs: &[crate::storage::experience_run::Model],
    tool_logs: &[crate::storage::entities::operational_log::Model],
    routing_logs: &[crate::storage::entities::operational_log::Model],
    llm_logs: &[crate::storage::entities::operational_log::Model],
) -> Vec<PromptEvolutionMetric> {
    aggregate_bundle_metrics_by_selectors(
        experience_runs,
        tool_logs,
        routing_logs,
        llm_logs,
        |run| {
            run.prompt_version
                .as_deref()
                .map(str::trim)
                .filter(|value| !value.is_empty())
                .map(str::to_string)
        },
        |row| {
            row.prompt_version
                .as_deref()
                .map(str::trim)
                .filter(|value| !value.is_empty())
                .map(str::to_string)
        },
    )
}

pub(super) fn build_prompt_insights(
    prompt_metrics: &[PromptEvolutionMetric],
    prompt_canary_state: Option<&crate::core::self_evolve::strategy_runtime::CanaryRolloutState>,
) -> PromptEvolutionInsights {
    let Some(canary_state) = prompt_canary_state else {
        return PromptEvolutionInsights::default();
    };
    let baseline = prompt_metrics
        .iter()
        .find(|metric| metric.version == canary_state.baseline_version);
    let candidate = prompt_metrics
        .iter()
        .find(|metric| metric.version == canary_state.candidate_version);

    let mut regressions = Vec::new();
    let mut summary = Vec::new();
    let experience_success_uplift = match (baseline, candidate) {
        (Some(base), Some(cand)) => cand.success_rate - base.success_rate,
        _ => 0.0,
    };
    let delegation_avoided = match (baseline, candidate) {
        (Some(base), Some(cand)) => {
            (base.delegation_rate - cand.delegation_rate) * cand.routing_decisions as f64
        }
        _ => 0.0,
    };
    let clarification_avoided = match (baseline, candidate) {
        (Some(base), Some(cand)) => {
            (base.clarification_rate - cand.clarification_rate) * cand.routing_decisions as f64
        }
        _ => 0.0,
    };
    let successful_direct_resolution_uplift = match (baseline, candidate) {
        (Some(base), Some(cand)) => {
            ((1.0 - cand.delegation_rate) * cand.tool_success_rate)
                - ((1.0 - base.delegation_rate) * base.tool_success_rate)
        }
        _ => 0.0,
    };
    let tool_success_uplift = match (baseline, candidate) {
        (Some(base), Some(cand)) => cand.tool_success_rate - base.tool_success_rate,
        _ => 0.0,
    };
    let latency_savings_p95_ms = match (
        baseline.and_then(|m| m.p95_latency_ms),
        candidate.and_then(|m| m.p95_latency_ms),
    ) {
        (Some(base), Some(cand)) => Some(base - cand),
        _ => None,
    };
    let failed_delegation_reduction = match (baseline, candidate) {
        (Some(base), Some(cand)) => {
            (base.delegation_rate * base.error_rate) - (cand.delegation_rate * cand.error_rate)
        }
        _ => 0.0,
    };

    if experience_success_uplift > 0.0 {
        summary.push(format!(
            "End-to-end experience success improved by {:.1} points.",
            experience_success_uplift * 100.0
        ));
    } else if experience_success_uplift < 0.0 {
        regressions.push(format!(
            "End-to-end experience success is down {:.1} points.",
            experience_success_uplift.abs() * 100.0
        ));
    }
    if delegation_avoided > 0.0 {
        summary.push(format!(
            "Saved an estimated {:.1} delegated runs versus baseline.",
            delegation_avoided
        ));
    }
    if clarification_avoided > 0.0 {
        summary.push(format!(
            "Avoided an estimated {:.1} clarification turns.",
            clarification_avoided
        ));
    }
    if tool_success_uplift > 0.0 {
        summary.push(format!(
            "Improved tool success by {:.1} points.",
            tool_success_uplift * 100.0
        ));
    } else if tool_success_uplift < 0.0 {
        regressions.push(format!(
            "Tool success is down {:.1} points.",
            tool_success_uplift.abs() * 100.0
        ));
    }
    if successful_direct_resolution_uplift > 0.0 {
        summary.push(format!(
            "Direct resolution uplift is {:.1} points.",
            successful_direct_resolution_uplift * 100.0
        ));
    }
    if let Some(delta_ms) = latency_savings_p95_ms {
        if delta_ms > 0 {
            summary.push(format!("Reduced p95 latency by {} ms.", delta_ms));
        } else if delta_ms < 0 {
            regressions.push(format!("p95 latency regressed by {} ms.", delta_ms.abs()));
        }
    }
    if failed_delegation_reduction > 0.0 {
        summary.push(format!(
            "Reduced estimated failed delegation rate by {:.1} points.",
            failed_delegation_reduction * 100.0
        ));
    }

    PromptEvolutionInsights {
        baseline_version: Some(canary_state.baseline_version.clone()),
        candidate_version: Some(canary_state.candidate_version.clone()),
        rollout_percent: canary_state.rollout_percent,
        delegation_avoided: round4(delegation_avoided),
        clarification_avoided: round4(clarification_avoided),
        successful_direct_resolution_uplift: round4(successful_direct_resolution_uplift),
        tool_success_uplift: round4(tool_success_uplift),
        latency_savings_p95_ms,
        failed_delegation_reduction: round4(failed_delegation_reduction),
        regressions,
        summary,
    }
}

pub(super) fn build_specialist_prompt_insights(
    prompt_metrics: &[PromptEvolutionMetric],
    prompt_canary_state: Option<&crate::core::self_evolve::strategy_runtime::CanaryRolloutState>,
) -> PromptEvolutionInsights {
    let Some(canary_state) = prompt_canary_state else {
        return PromptEvolutionInsights::default();
    };
    let baseline = prompt_metrics
        .iter()
        .find(|metric| metric.version == canary_state.baseline_version);
    let candidate = prompt_metrics
        .iter()
        .find(|metric| metric.version == canary_state.candidate_version);

    let mut regressions = Vec::new();
    let mut summary = Vec::new();
    let experience_success_uplift = match (baseline, candidate) {
        (Some(base), Some(cand)) => cand.success_rate - base.success_rate,
        _ => 0.0,
    };
    let tool_success_uplift = match (baseline, candidate) {
        (Some(base), Some(cand)) => cand.tool_success_rate - base.tool_success_rate,
        _ => 0.0,
    };
    let latency_savings_p95_ms = match (
        baseline.and_then(|m| m.p95_latency_ms),
        candidate.and_then(|m| m.p95_latency_ms),
    ) {
        (Some(base), Some(cand)) => Some(base - cand),
        _ => None,
    };
    let error_rate_reduction = match (baseline, candidate) {
        (Some(base), Some(cand)) => base.error_rate - cand.error_rate,
        _ => 0.0,
    };

    if experience_success_uplift > 0.0 {
        summary.push(format!(
            "End-to-end experience success improved by {:.1} points.",
            experience_success_uplift * 100.0
        ));
    } else if experience_success_uplift < 0.0 {
        regressions.push(format!(
            "End-to-end experience success is down {:.1} points.",
            experience_success_uplift.abs() * 100.0
        ));
    }
    if tool_success_uplift > 0.0 {
        summary.push(format!(
            "Improved tool success by {:.1} points.",
            tool_success_uplift * 100.0
        ));
    } else if tool_success_uplift < 0.0 {
        regressions.push(format!(
            "Tool success is down {:.1} points.",
            tool_success_uplift.abs() * 100.0
        ));
    }
    if error_rate_reduction > 0.0 {
        summary.push(format!(
            "Resolved-run error rate improved by {:.1} points.",
            error_rate_reduction * 100.0
        ));
    } else if error_rate_reduction < 0.0 {
        regressions.push(format!(
            "Resolved-run error rate regressed by {:.1} points.",
            error_rate_reduction.abs() * 100.0
        ));
    }
    if let Some(delta_ms) = latency_savings_p95_ms {
        if delta_ms > 0 {
            summary.push(format!("Reduced p95 latency by {} ms.", delta_ms));
        } else if delta_ms < 0 {
            regressions.push(format!("p95 latency regressed by {} ms.", delta_ms.abs()));
        }
    }

    PromptEvolutionInsights {
        baseline_version: Some(canary_state.baseline_version.clone()),
        candidate_version: Some(canary_state.candidate_version.clone()),
        rollout_percent: canary_state.rollout_percent,
        delegation_avoided: 0.0,
        clarification_avoided: 0.0,
        successful_direct_resolution_uplift: 0.0,
        tool_success_uplift: round4(tool_success_uplift),
        latency_savings_p95_ms,
        failed_delegation_reduction: 0.0,
        regressions,
        summary,
    }
}

pub(super) async fn build_evolution_dev_response(
    storage: &crate::storage::Storage,
    limit: u64,
    include_superseded: bool,
) -> EvolutionDevResponse {
    let limit = limit.clamp(24, EVOLUTION_DEV_MAX_LIMIT);
    let canary_state = load_evolution_canary_state(storage).await;
    let routing_policy_metric_version =
        routing_policy_metric_fallback_version(canary_state.as_ref());
    let logs = storage
        .list_operational_log_version_metrics_by_event("tool_call", limit)
        .await
        .unwrap_or_default();
    let response_logs = storage
        .list_operational_log_version_metrics_by_event("response_complete", limit)
        .await
        .unwrap_or_default();
    let recent_trace_rows = storage
        .list_execution_trace_summaries(None, limit.max(24), 0)
        .await
        .unwrap_or_default();
    let mut policy_metrics =
        aggregate_policy_version_metrics(&response_logs, &routing_policy_metric_version);
    if policy_metrics.is_empty() {
        policy_metrics = aggregate_policy_version_metrics(&logs, &routing_policy_metric_version);
    }
    if policy_metrics.is_empty() {
        policy_metrics =
            aggregate_trace_policy_metrics(&recent_trace_rows, &routing_policy_metric_version);
    }
    let strategy_metrics = aggregate_version_metrics(&logs, |row| row.strategy_version.as_deref());
    let strategy_canary_state = load_canary_state_by_key(
        storage,
        crate::core::self_evolve::strategy_runtime::TOOL_STRATEGY_CANARY_STATE_KEY,
    )
    .await;
    let prompt_tool_logs = storage
        .list_operational_logs_by_event("tool_call", limit)
        .await
        .unwrap_or_default();
    let prompt_routing_logs = storage
        .list_operational_logs_by_event("routing_decision", limit)
        .await
        .unwrap_or_default();
    let prompt_llm_logs = storage
        .list_operational_logs_by_event("llm_decision", limit)
        .await
        .unwrap_or_default();
    let learning_candidate_rows = storage
        .list_learning_candidates_with_options(None, include_superseded, 24)
        .await
        .unwrap_or_default();
    let recent_experience_run_rows = storage
        .list_recent_experience_runs_any_scope(limit.max(24))
        .await
        .unwrap_or_default();
    let readiness_policy = crate::core::readiness::load_readiness_policy(storage).await;
    let mut learning_candidate_replay_gates = HashMap::new();
    for candidate in &learning_candidate_rows {
        match crate::core::self_evolve::replay_gate::evaluate_candidate_replay_gate(
            storage, candidate,
        )
        .await
        {
            Ok(gate) => {
                learning_candidate_replay_gates.insert(candidate.id.clone(), gate);
            }
            Err(error) => {
                tracing::warn!(
                    "Failed to evaluate replay gate for learning candidate '{}': {}",
                    candidate.id,
                    error
                );
            }
        }
    }
    let mut learning_candidate_readiness = HashMap::new();
    for candidate in &learning_candidate_rows {
        let readiness = crate::core::readiness::evaluate_learning_candidate_readiness(
            candidate,
            learning_candidate_replay_gates.get(&candidate.id),
            &readiness_policy,
        );
        if let Err(error) = crate::core::readiness::record_readiness_evaluation(
            storage,
            "learning_candidate",
            &candidate.id,
            &readiness,
        )
        .await
        {
            tracing::warn!(
                candidate_id = %candidate.id,
                error = %error,
                "Failed to record learning candidate readiness evaluation"
            );
        }
        learning_candidate_readiness.insert(candidate.id.clone(), readiness);
    }
    let learning_candidates = learning_candidate_rows
        .iter()
        .map(|candidate| {
            build_learning_candidate_summary(
                candidate,
                learning_candidate_replay_gates.get(&candidate.id),
                learning_candidate_readiness.get(&candidate.id),
            )
        })
        .collect::<Vec<_>>();
    let skill_evolutions = learning_candidate_rows
        .iter()
        .filter_map(|candidate| {
            build_skill_evolution_entry(
                candidate,
                &recent_experience_run_rows,
                learning_candidate_replay_gates.get(&candidate.id),
                learning_candidate_readiness.get(&candidate.id),
            )
        })
        .collect::<Vec<_>>();
    let learning_item_rows = storage
        .list_active_experience_items_any_scope(
            &["constraint", "personal_fact", "lesson", "procedure"],
            72,
        )
        .await
        .unwrap_or_default()
        .into_iter()
        .filter(experience_item_has_evolution_evidence)
        .collect::<Vec<_>>();
    let learning_items = learning_item_rows
        .iter()
        .map(|item| build_experience_item_summary(&item))
        .collect::<Vec<_>>();
    let learning_pattern_rows = storage
        .list_procedural_patterns_any_scope(&["active", "draft"], 48)
        .await
        .unwrap_or_default();
    let mut learning_patterns = Vec::with_capacity(learning_pattern_rows.len());
    for pattern in &learning_pattern_rows {
        let readiness = crate::core::readiness::evaluate_procedural_pattern_readiness(
            pattern,
            &readiness_policy,
        );
        if let Err(error) = crate::core::readiness::record_readiness_evaluation(
            storage,
            "procedural_pattern",
            &pattern.id,
            &readiness,
        )
        .await
        {
            tracing::warn!(
                pattern_id = %pattern.id,
                error = %error,
                "Failed to record procedural pattern readiness evaluation"
            );
        }
        learning_patterns.push(build_procedural_pattern_summary(pattern, Some(&readiness)));
    }
    let experience_graph = build_experience_graph_summary(
        storage,
        &recent_experience_run_rows,
        &learning_item_rows,
        &learning_pattern_rows,
        &learning_candidate_rows,
    )
    .await;
    let prompt_telemetry_summary = aggregate_prompt_telemetry_summary_with_traces(
        &recent_experience_run_rows,
        &recent_trace_rows,
    );
    let prompt_optimization_review_state = load_prompt_optimization_review_state(storage).await;
    let prompt_canary_safety_events = load_prompt_canary_safety_events(storage).await;
    let prompt_optimization_opportunities = build_prompt_optimization_opportunities(
        &prompt_telemetry_summary,
        &prompt_optimization_review_state,
    );
    let prompt_metrics = aggregate_prompt_metrics(
        &recent_experience_run_rows,
        &prompt_tool_logs,
        &prompt_routing_logs,
        &prompt_llm_logs,
    );
    let prompt_canary_state = load_prompt_evolution_canary_state(storage).await;
    let prompt_insights = build_prompt_insights(&prompt_metrics, prompt_canary_state.as_ref());
    let specialist_prompt_canary_state = load_canary_state_by_key(
        storage,
        crate::core::self_evolve::SPECIALIST_PROMPT_BUNDLE_CANARY_STATE_KEY,
    )
    .await;
    let prompt_fragment_canary_state = load_canary_state_by_key(
        storage,
        crate::core::prompt_fragments::PROMPT_FRAGMENT_BUNDLE_CANARY_STATE_KEY,
    )
    .await;
    let specialist_prompt_metrics = aggregate_bundle_metrics_by_selectors(
        &recent_experience_run_rows,
        &prompt_tool_logs,
        &prompt_routing_logs,
        &prompt_llm_logs,
        |run| {
            crate::core::self_evolve::strategy_runtime::experience_run_metadata_version(
                run,
                "specialist_prompt_version",
            )
            .map(str::to_string)
        },
        |row| operational_payload_string(row, "specialist_prompt_version"),
    );
    let specialist_prompt_insights = build_specialist_prompt_insights(
        &specialist_prompt_metrics,
        specialist_prompt_canary_state.as_ref(),
    );
    let prompt_fragment_metrics = aggregate_prompt_telemetry_metrics_by_version(
        &recent_trace_rows,
        "prompt_fragment_version",
    );
    let prompt_fragment_insights = build_specialist_prompt_insights(
        &prompt_fragment_metrics,
        prompt_fragment_canary_state.as_ref(),
    );
    let recent_prompt_runs = recent_experience_run_rows
        .iter()
        .filter(|run| {
            run.prompt_version
                .as_deref()
                .map(str::trim)
                .filter(|value| !value.is_empty())
                .is_some()
        })
        .take(24)
        .map(build_experience_run_summary)
        .collect::<Vec<_>>();
    let recent_specialist_prompt_runs = recent_experience_run_rows
        .iter()
        .filter(|run| {
            crate::core::self_evolve::strategy_runtime::experience_run_metadata_version(
                run,
                "specialist_prompt_version",
            )
            .is_some()
        })
        .take(24)
        .map(build_experience_run_summary)
        .collect::<Vec<_>>();
    let recent_prompt_fragment_runs =
        recent_prompt_telemetry_runs_by_version(&recent_trace_rows, "prompt_fragment_version", 24);
    let recent_experience_runs = recent_experience_run_rows
        .into_iter()
        .take(EVOLUTION_DEV_RECENT_RUN_RESPONSE_LIMIT)
        .map(|run| build_experience_run_summary(&run))
        .collect::<Vec<_>>();
    EvolutionDevResponse {
        canary_state,
        strategy_canary_state,
        last_result: load_last_self_evolve_result(storage).await,
        lineage_recent: read_recent_lineage(40).await,
        policy_metrics,
        strategy_metrics,
        prompt_canary_state,
        prompt_last_result: load_json_value_by_key(
            storage,
            crate::core::self_evolve::PROMPT_BUNDLE_LAST_RESULT_KEY,
        )
        .await,
        prompt_lineage_recent: read_recent_prompt_lineage(40).await,
        prompt_metrics,
        prompt_insights,
        specialist_prompt_canary_state,
        specialist_prompt_last_result: load_json_value_by_key(
            storage,
            crate::core::self_evolve::SPECIALIST_PROMPT_BUNDLE_LAST_RESULT_KEY,
        )
        .await,
        specialist_prompt_lineage_recent: read_recent_specialist_prompt_lineage(40).await,
        specialist_prompt_metrics,
        specialist_prompt_insights,
        prompt_fragment_canary_state,
        prompt_fragment_last_result: load_json_value_by_key(
            storage,
            crate::core::prompt_fragments::PROMPT_FRAGMENT_BUNDLE_LAST_RESULT_KEY,
        )
        .await,
        prompt_fragment_lineage_recent: read_recent_prompt_fragment_lineage(40).await,
        prompt_fragment_metrics,
        prompt_fragment_insights,
        learning_queue: storage.learning_queue_counts().await.unwrap_or_default(),
        learning_candidates,
        skill_evolutions,
        learning_items,
        learning_patterns,
        experience_graph,
        recent_prompt_runs,
        recent_specialist_prompt_runs,
        recent_prompt_fragment_runs,
        recent_experience_runs,
        prompt_canary_safety_events,
        prompt_telemetry_summary,
        prompt_optimization_opportunities,
    }
}

pub(super) fn aggregate_prompt_telemetry_metrics_by_version(
    traces: &[crate::storage::ExecutionTraceSummaryRow],
    metadata_key: &str,
) -> Vec<PromptEvolutionMetric> {
    #[derive(Default)]
    struct Bucket {
        samples: usize,
        successes: usize,
        latencies: Vec<i64>,
        tool_counts: Vec<usize>,
    }

    let mut buckets: BTreeMap<String, Bucket> = BTreeMap::new();
    for trace in traces {
        let success = !trace_summary_has_error_step(trace);
        for prompt_telemetry in prompt_telemetry_samples_from_trace(trace) {
            let Some(version) = prompt_telemetry
                .get(metadata_key)
                .and_then(|value| value.as_str())
                .map(str::trim)
                .filter(|value| !value.is_empty())
            else {
                continue;
            };
            let bucket = buckets.entry(version.to_string()).or_default();
            bucket.samples = bucket.samples.saturating_add(1);
            if success {
                bucket.successes = bucket.successes.saturating_add(1);
            }
            if let Some(duration_ms) = trace.duration_ms {
                bucket.latencies.push(i64::from(duration_ms));
            }
            if let Some(tool_count) = prompt_telemetry_usize(prompt_telemetry.get("tool_count")) {
                bucket.tool_counts.push(tool_count);
            }
        }
    }

    let mut out = buckets
        .into_iter()
        .filter_map(|(version, bucket)| {
            if bucket.samples == 0 {
                return None;
            }
            let errors = bucket.samples.saturating_sub(bucket.successes);
            let success_rate = round4(bucket.successes as f64 / bucket.samples as f64);
            Some(PromptEvolutionMetric {
                version,
                samples: bucket.samples,
                success_rate,
                error_rate: round4(errors as f64 / bucket.samples as f64),
                p95_latency_ms: compute_p95(bucket.latencies),
                routing_decisions: 0,
                delegation_rate: 0.0,
                clarification_rate: 0.0,
                avg_tool_calls_per_request: average_usize(&bucket.tool_counts),
                tool_success_rate: success_rate,
            })
        })
        .collect::<Vec<_>>();
    out.sort_by(|a, b| {
        b.samples
            .cmp(&a.samples)
            .then_with(|| a.version.cmp(&b.version))
    });
    out
}

pub(super) fn recent_prompt_telemetry_runs_by_version(
    traces: &[crate::storage::ExecutionTraceSummaryRow],
    metadata_key: &str,
    limit: usize,
) -> Vec<serde_json::Value> {
    let mut rows = Vec::new();
    for trace in traces {
        let success = !trace_summary_has_error_step(trace);
        for prompt_telemetry in prompt_telemetry_samples_from_trace(trace) {
            let Some(version) = prompt_telemetry
                .get(metadata_key)
                .and_then(|value| value.as_str())
                .map(str::trim)
                .filter(|value| !value.is_empty())
            else {
                continue;
            };
            rows.push(serde_json::json!({
                "trace_id": trace.id.clone(),
                "channel": trace.channel.clone(),
                "created_at": trace.created_at.clone(),
                "duration_ms": trace.duration_ms,
                "success": success,
                "version": version,
                "request_mode": prompt_telemetry
                    .get("request_mode")
                    .and_then(|value| value.as_str())
                    .unwrap_or("agent_loop"),
                "estimated_total_request_chars": prompt_telemetry_usize(
                    prompt_telemetry.get("estimated_total_request_chars")
                ).unwrap_or_default(),
                "final_system_prompt_chars": prompt_telemetry_usize(
                    prompt_telemetry.get("final_system_prompt_chars")
                ).unwrap_or_default(),
                "tool_count": prompt_telemetry_usize(prompt_telemetry.get("tool_count"))
                    .unwrap_or_default(),
            }));
            if rows.len() >= limit {
                return rows;
            }
        }
    }
    rows
}

pub(super) async fn get_evolution_settings(State(state): State<AppState>) -> Response {
    let (storage, agent_config, primary_model_id) = {
        let agent = state.agent.read().await;
        (
            agent.storage.clone(),
            agent.config.clone(),
            agent.primary_model_id.clone(),
        )
    };
    let project_root = resolve_project_root();
    Json(
        build_evolution_settings_response(
            &storage,
            &agent_config,
            &primary_model_id,
            &project_root,
        )
        .await,
    )
    .into_response()
}

pub(super) async fn update_evolution_settings(
    State(state): State<AppState>,
    Json(request): Json<EvolutionSettingsUpdateRequest>,
) -> Response {
    let (storage, agent_config, primary_model_id) = {
        let agent = state.agent.read().await;
        (
            agent.storage.clone(),
            agent.config.clone(),
            agent.primary_model_id.clone(),
        )
    };
    let project_root = resolve_project_root();
    if let Some(enabled) = request.self_evolve_enabled.or(request.learning_enabled) {
        if let Err(e) = store_bool_setting(
            &storage,
            crate::core::self_evolve::strategy_runtime::SELF_EVOLVE_ENABLED_KEY,
            enabled,
        )
        .await
        {
            return (
                StatusCode::INTERNAL_SERVER_ERROR,
                Json(ErrorResponse {
                    error: format!("Failed to update self_evolve_enabled: {}", e),
                }),
            )
                .into_response();
        }
        if let Err(e) = store_bool_setting(
            &storage,
            crate::core::learning::LEARNING_ENABLED_KEY,
            enabled,
        )
        .await
        {
            return (
                StatusCode::INTERNAL_SERVER_ERROR,
                Json(ErrorResponse {
                    error: format!("Failed to update learning_enabled: {}", e),
                }),
            )
                .into_response();
        }
        if !enabled {
            if let Err(e) = disable_all_evolution_canaries(&storage).await {
                return (
                    StatusCode::INTERNAL_SERVER_ERROR,
                    Json(ErrorResponse {
                        error: format!("Failed to disable evolution canaries: {}", e),
                    }),
                )
                    .into_response();
            }
        }
    }
    if let Some(enabled) = request.deploy_guard_default {
        if let Err(e) = store_bool_setting(
            &storage,
            crate::core::self_evolve::strategy_runtime::APP_DEPLOY_ACCESS_GUARD_DEFAULT_KEY,
            enabled,
        )
        .await
        {
            return (
                StatusCode::INTERNAL_SERVER_ERROR,
                Json(ErrorResponse {
                    error: format!("Failed to update evolution settings: {}", e),
                }),
            )
                .into_response();
        }
    }
    if let Some(slot) = request.learning_model_slot.as_deref() {
        if let Err(e) = storage
            .set(
                crate::core::learning::LEARNING_MODEL_SLOT_KEY,
                slot.trim().as_bytes(),
            )
            .await
        {
            return (
                StatusCode::INTERNAL_SERVER_ERROR,
                Json(ErrorResponse {
                    error: format!("Failed to update learning_model_slot: {}", e),
                }),
            )
                .into_response();
        }
    }
    if let Some(cap) = request.learning_queue_cap {
        let cap_value = cap.max(1).to_string();
        if let Err(e) = storage
            .set(
                crate::core::learning::LEARNING_QUEUE_CAP_KEY,
                cap_value.as_bytes(),
            )
            .await
        {
            return (
                StatusCode::INTERNAL_SERVER_ERROR,
                Json(ErrorResponse {
                    error: format!("Failed to update learning_queue_cap: {}", e),
                }),
            )
                .into_response();
        }
    }
    if let Some(policy) = request.readiness_policy.as_ref() {
        if let Err(e) = crate::core::readiness::save_readiness_policy(&storage, policy).await {
            return (
                StatusCode::INTERNAL_SERVER_ERROR,
                Json(ErrorResponse {
                    error: format!("Failed to update readiness_policy: {}", e),
                }),
            )
                .into_response();
        }
    }
    let mut gepa_config =
        crate::core::self_evolve::gepa_bridge::load_gepa_optimizer_config(&storage).await;
    let mut gepa_config_changed = false;
    if let Some(enabled) = request.gepa_enabled {
        gepa_config.enabled = enabled;
        gepa_config_changed = true;
    }
    if let Some(mode) = request.gepa_auto_mode.as_deref() {
        gepa_config.auto_mode = mode.trim().to_string();
        gepa_config_changed = true;
    }
    if let Some(value) = request.gepa_daily_budget_usd {
        gepa_config.daily_budget_usd = value;
        gepa_config_changed = true;
    }
    if let Some(value) = request.gepa_per_run_budget_usd {
        gepa_config.per_run_budget_usd = value;
        gepa_config_changed = true;
    }
    if let Some(value) = request.gepa_max_runs_per_day {
        gepa_config.max_runs_per_day = value;
        gepa_config_changed = true;
    }
    if let Some(value) = request.gepa_max_metric_calls {
        gepa_config.max_metric_calls = value;
        gepa_config_changed = true;
    }
    if gepa_config_changed {
        if let Err(e) = crate::core::self_evolve::gepa_bridge::save_gepa_optimizer_config(
            &storage,
            &gepa_config,
        )
        .await
        {
            return (
                StatusCode::INTERNAL_SERVER_ERROR,
                Json(ErrorResponse {
                    error: format!("Failed to update GEPA optimizer settings: {}", e),
                }),
            )
                .into_response();
        }
    }
    Json(
        build_evolution_settings_response(
            &storage,
            &agent_config,
            &primary_model_id,
            &project_root,
        )
        .await,
    )
    .into_response()
}

pub(super) async fn get_evolution_dev(
    State(state): State<AppState>,
    Query(query): Query<EvolutionDevQuery>,
) -> Response {
    let storage = {
        let agent = state.agent.read().await;
        agent.storage.clone()
    };
    let limit = normalize_evolution_dev_limit(query.limit);
    Json(
        build_evolution_dev_response(&storage, limit, query.include_superseded.unwrap_or(false))
            .await,
    )
    .into_response()
}

pub(super) async fn persist_evolution_action_trace(
    state: &AppState,
    action: &str,
    message: &str,
    detail_payload: serde_json::Value,
) -> Option<String> {
    let started_at = chrono::Utc::now();
    let trace_id = uuid::Uuid::new_v4().to_string();
    let detail_data = serde_json::to_string_pretty(&detail_payload).ok();
    let trace_ref = Arc::new(RwLock::new(ExecutionTrace {
        id: trace_id.clone(),
        message: format!("Evolution action: {}", action),
        channel: "evolution".to_string(),
        started_at: Some(started_at),
        completed_at: Some(started_at),
        steps: vec![
            crate::core::ExecutionStep {
                icon: "[evolve]".to_string(),
                title: "Evolution Manual Action".to_string(),
                detail: "Applied a manual evolution control from the Evolution panel.".to_string(),
                step_type: "info".to_string(),
                data: Some(
                    serde_json::to_string_pretty(&serde_json::json!({
                        "trace_kind": "self_evolve.manual_action.request",
                        "action": action,
                        "message": message,
                    }))
                    .unwrap_or_default(),
                ),
                timestamp: started_at,
                duration_ms: Some(0),
            },
            crate::core::ExecutionStep {
                icon: "[ok]".to_string(),
                title: "Evolution Decision Applied".to_string(),
                detail: message.to_string(),
                step_type: "success".to_string(),
                data: detail_data,
                timestamp: started_at,
                duration_ms: Some(0),
            },
        ],
        proof_id: None,
        response: Some(message.to_string()),
        model: Some("internal:evolution".to_string()),
        input_tokens: 0,
        output_tokens: 0,
        total_tokens: 0,
        cost_usd: 0.0,
        complexity: Some("evolution".to_string()),
        plan: None,
    }));

    let agent = state.agent.read().await;
    agent.persist_completed_trace(&trace_ref).await;
    Some(trace_id)
}

pub(super) fn truncate_trace_text(text: &str, max_chars: usize) -> String {
    let mut out = String::new();
    for (idx, ch) in text.chars().enumerate() {
        if idx >= max_chars {
            out.push_str("...");
            break;
        }
        out.push(ch);
    }
    out
}

pub(super) fn collapse_trace_preview(text: &str, max_chars: usize) -> String {
    let collapsed = text.split_whitespace().collect::<Vec<_>>().join(" ");
    truncate_trace_text(collapsed.trim(), max_chars)
}

pub(super) fn humanize_trace_channel(channel: &str) -> String {
    let mut words = Vec::new();
    for part in channel.trim().split('_').filter(|part| !part.is_empty()) {
        let mut chars = part.chars();
        if let Some(first) = chars.next() {
            let mut word = String::new();
            word.push(first.to_ascii_uppercase());
            word.extend(chars);
            words.push(word);
        }
    }
    if words.is_empty() {
        "Push".to_string()
    } else {
        words.join(" ")
    }
}

pub(super) fn summarize_daily_brief_delivery(result: &serde_json::Value) -> String {
    let delivery = result.get("delivery");
    let in_app_notification_suppressed = delivery
        .and_then(|value| value.get("in_app_notification_suppressed"))
        .and_then(|value| value.as_bool())
        .unwrap_or(false);
    let notification_suppressed = delivery
        .and_then(|value| value.get("notification_suppressed"))
        .or_else(|| delivery.and_then(|value| value.get("suppressed")))
        .and_then(|value| value.as_bool())
        .unwrap_or(false);
    if notification_suppressed {
        return "No notification sent.".to_string();
    }
    let in_app_success = delivery
        .and_then(|value| value.get("in_app"))
        .and_then(|value| value.get("success"))
        .and_then(|value| value.as_bool())
        .unwrap_or(false);
    let attempts = delivery
        .and_then(|value| value.get("push_attempts"))
        .and_then(|value| value.as_array());

    if let Some(attempts) = attempts {
        if let Some(successful) = attempts.iter().find(|attempt| {
            attempt.get("success").and_then(|value| value.as_bool()) == Some(true)
                && attempt
                    .get("channel")
                    .and_then(|value| value.as_str())
                    .map(|channel| {
                        let normalized = channel.trim().to_ascii_lowercase();
                        normalized != "web" && normalized != "in_app"
                    })
                    .unwrap_or(false)
        }) {
            let channel = successful
                .get("channel")
                .and_then(|value| value.as_str())
                .unwrap_or("push");
            return format!("Push delivered via {}.", humanize_trace_channel(channel));
        }

        if let Some(first_attempt) = attempts.first() {
            let channel = first_attempt
                .get("channel")
                .and_then(|value| value.as_str())
                .unwrap_or("push");
            let error = first_attempt
                .get("error")
                .and_then(|value| value.as_str())
                .map(|value| collapse_trace_preview(value, 90))
                .filter(|value| !value.is_empty());
            let failure = match error {
                Some(error) => format!(
                    "{} delivery failed: {}.",
                    humanize_trace_channel(channel),
                    error
                ),
                None => format!("{} delivery failed.", humanize_trace_channel(channel)),
            };
            if in_app_success {
                return format!("Saved in-app only. {}", failure);
            }
            if in_app_notification_suppressed {
                return format!("No in-app notification created. {}", failure);
            }
            return failure;
        }
    }

    if in_app_success {
        "Saved in-app only.".to_string()
    } else if in_app_notification_suppressed {
        "No in-app notification created.".to_string()
    } else {
        "Delivery status unavailable.".to_string()
    }
}

pub(super) fn summarize_autonomy_action_result(
    action: &RecommendedAction,
    result: &serde_json::Value,
) -> String {
    let status = result
        .get("status")
        .and_then(|value| value.as_str())
        .unwrap_or("")
        .trim()
        .to_ascii_lowercase();
    if status == "queued_for_approval" {
        return format!("Queued '{}' for approval.", action.title.trim());
    }

    let kind = result
        .get("kind")
        .and_then(|value| value.as_str())
        .unwrap_or(action.action_kind.as_str())
        .trim()
        .to_ascii_lowercase();

    match kind.as_str() {
        "daily_brief_now" => {
            let preview = result
                .get("brief")
                .and_then(|value| value.as_str())
                .unwrap_or("")
                .trim();
            let delivery_summary = summarize_daily_brief_delivery(result);
            if preview.is_empty() {
                format!("Daily brief generated. {}", delivery_summary)
            } else {
                format!(
                    "Daily brief generated. {} Preview: {}",
                    delivery_summary,
                    collapse_trace_preview(preview, 180)
                )
            }
        }
        "create_task" => {
            let task_id = result
                .get("task_id")
                .and_then(|value| value.as_str())
                .unwrap_or("")
                .trim();
            if result
                .get("reused_existing")
                .and_then(|value| value.as_bool())
                .unwrap_or(false)
            {
                if task_id.is_empty() {
                    "Reused an existing task for this suggestion.".to_string()
                } else {
                    format!("Reused existing task {}.", task_id)
                }
            } else if task_id.is_empty() {
                "Created a task from this suggestion.".to_string()
            } else {
                format!("Created task {}.", task_id)
            }
        }
        "watch" => {
            let watcher_id = result
                .get("watcher_id")
                .and_then(|value| value.as_str())
                .unwrap_or("")
                .trim();
            if watcher_id.is_empty() {
                "Created a watcher from this suggestion.".to_string()
            } else {
                format!("Created watcher {}.", watcher_id)
            }
        }
        "activate_mode" => {
            let mode_name = result
                .get("result")
                .and_then(|value| value.get("mode_name"))
                .and_then(|value| value.as_str())
                .unwrap_or("")
                .trim();
            if mode_name.is_empty() {
                format!("Activated '{}'.", action.title.trim())
            } else {
                format!("Activated mode '{}'.", mode_name)
            }
        }
        "chat_prompt" => {
            let response = result
                .get("response")
                .and_then(|value| value.as_str())
                .unwrap_or("")
                .trim();
            if response.is_empty() {
                format!("Ran '{}'.", action.title.trim())
            } else {
                truncate_trace_text(response, 220)
            }
        }
        "delegate" => {
            let final_result = result
                .get("final_result")
                .and_then(|value| value.as_str())
                .unwrap_or("")
                .trim();
            if final_result.is_empty() {
                "Delegated the requested work.".to_string()
            } else {
                truncate_trace_text(final_result, 220)
            }
        }
        _ => result
            .get("message")
            .and_then(|value| value.as_str())
            .map(|value| value.trim())
            .filter(|value| !value.is_empty())
            .map(|value| truncate_trace_text(value, 220))
            .unwrap_or_else(|| format!("Ran '{}'.", action.title.trim())),
    }
}

pub(super) async fn persist_autonomy_action_trace(
    state: &AppState,
    action: &RecommendedAction,
    status: &str,
    summary: &str,
    detail_payload: serde_json::Value,
) -> Option<String> {
    let started_at = chrono::Utc::now();
    let trace_id = uuid::Uuid::new_v4().to_string();
    let status_normalized = status.trim().to_ascii_lowercase();
    let (step_type, icon, title) = if status_normalized == "error" {
        ("error", "[err]", "Autonomy Action Failed")
    } else if status_normalized == "queued_for_approval" {
        ("warning", "[wait]", "Autonomy Action Queued")
    } else {
        ("success", "[ok]", "Autonomy Action Completed")
    };
    let detail_data = serde_json::to_string_pretty(&detail_payload)
        .ok()
        .map(|text| crate::security::redact_pii(&truncate_trace_text(&text, 12000)));
    let request_data = serde_json::to_string_pretty(&serde_json::json!({
        "trace_kind": "autonomy.action.request",
        "action_id": action.id,
        "title": action.title,
        "kind": action.action_kind,
        "payload": action.payload,
    }))
    .ok()
    .map(|text| crate::security::redact_pii(&truncate_trace_text(&text, 12000)));

    let trace_ref = Arc::new(RwLock::new(ExecutionTrace {
        id: trace_id.clone(),
        message: format!("Autonomy action: {}", action.title),
        channel: "autonomy".to_string(),
        started_at: Some(started_at),
        completed_at: Some(started_at),
        steps: vec![
            crate::core::ExecutionStep {
                icon: "[auto]".to_string(),
                title: "Autonomy Action Requested".to_string(),
                detail: format!("{} ({})", action.title.trim(), action.action_kind.trim()),
                step_type: "info".to_string(),
                data: request_data,
                timestamp: started_at,
                duration_ms: Some(0),
            },
            crate::core::ExecutionStep {
                icon: icon.to_string(),
                title: title.to_string(),
                detail: summary.to_string(),
                step_type: step_type.to_string(),
                data: detail_data,
                timestamp: started_at,
                duration_ms: Some(0),
            },
        ],
        proof_id: None,
        response: Some(summary.to_string()),
        model: Some("internal:autonomy".to_string()),
        input_tokens: 0,
        output_tokens: 0,
        total_tokens: 0,
        cost_usd: 0.0,
        complexity: Some("autonomy".to_string()),
        plan: None,
    }));

    let agent = state.agent.read().await;
    agent.persist_completed_trace(&trace_ref).await;
    Some(trace_id)
}

fn gepa_auto_latest_activity_at(
    state: &crate::core::self_evolve::gepa_bridge::GepaAutoRunState,
) -> Option<chrono::DateTime<chrono::Utc>> {
    [
        state.last_queued_at.as_deref(),
        state.last_completed_at.as_deref(),
    ]
    .into_iter()
    .flatten()
    .filter_map(parse_rfc3339_utc)
    .max()
}

async fn save_gepa_auto_skip(
    storage: &crate::storage::Storage,
    mut state: crate::core::self_evolve::gepa_bridge::GepaAutoRunState,
    now: chrono::DateTime<chrono::Utc>,
    reason: &str,
    evidence_samples: usize,
    next_check_after: Option<chrono::DateTime<chrono::Utc>>,
) {
    state.last_checked_at = Some(now.to_rfc3339());
    state.last_status = Some("waiting".to_string());
    state.last_reason = Some(reason.to_string());
    state.last_evidence_samples = evidence_samples;
    state.next_check_after = next_check_after
        .map(|value| value.to_rfc3339())
        .or_else(|| {
            Some((now + chrono::Duration::seconds(GEPA_AUTO_POLL_SECS as i64)).to_rfc3339())
        });
    if let Err(error) =
        crate::core::self_evolve::gepa_bridge::save_gepa_auto_run_state(storage, &state).await
    {
        tracing::warn!("Failed to save GEPA auto-run state: {}", error);
    }
}

async fn run_gepa_auto_tick(state: &AppState) -> Result<()> {
    let agent = {
        let agent = state.agent.read().await;
        agent.clone()
    };
    let storage = agent.storage.clone();
    let project_root = resolve_project_root();
    let now = chrono::Utc::now();
    let auto_state =
        crate::core::self_evolve::gepa_bridge::load_gepa_auto_run_state(&storage).await;

    let learning_enabled = load_learning_enabled(&storage).await;
    let self_evolve_enabled = storage
        .get(crate::core::self_evolve::strategy_runtime::SELF_EVOLVE_ENABLED_KEY)
        .await
        .ok()
        .flatten()
        .and_then(|raw| String::from_utf8(raw).ok())
        .map(|value| !value.trim().eq_ignore_ascii_case("false"))
        .unwrap_or(true)
        && learning_enabled;
    if !self_evolve_enabled {
        save_gepa_auto_skip(&storage, auto_state, now, "learning_paused", 0, None).await;
        return Ok(());
    }

    let readiness = crate::core::self_evolve::gepa_bridge::check_gepa_readiness(
        &storage,
        &project_root,
        &agent.config,
        &agent.primary_model_id,
    )
    .await;
    if !readiness.enabled {
        save_gepa_auto_skip(&storage, auto_state, now, "gepa_disabled", 0, None).await;
        return Ok(());
    }
    if !readiness.ready {
        save_gepa_auto_skip(
            &storage,
            auto_state,
            now,
            "model_or_runtime_not_ready",
            0,
            None,
        )
        .await;
        return Ok(());
    }
    if !readiness.budget.allowed {
        save_gepa_auto_skip(&storage, auto_state, now, "budget_paused", 0, None).await;
        return Ok(());
    }

    let (pending_jobs, running_jobs) =
        crate::core::self_evolve::gepa_bridge::active_job_counts(&project_root).await?;
    if pending_jobs > 0 || running_jobs > 0 {
        agent.spawn_gepa_idle_worker();
        save_gepa_auto_skip(&storage, auto_state, now, "work_already_scheduled", 0, None).await;
        return Ok(());
    }

    let idle = agent
        .gepa_background_idle_check(GEPA_AUTO_QUIET_WINDOW_SECS)
        .await;
    if !idle.idle {
        save_gepa_auto_skip(&storage, auto_state, now, "waiting_for_quiet_time", 0, None).await;
        return Ok(());
    }

    if let Some(last_activity_at) = gepa_auto_latest_activity_at(&auto_state) {
        let cooldown_until = last_activity_at + chrono::Duration::hours(GEPA_AUTO_COOLDOWN_HOURS);
        if now < cooldown_until {
            save_gepa_auto_skip(
                &storage,
                auto_state,
                now,
                "cooling_down",
                0,
                Some(cooldown_until),
            )
            .await;
            return Ok(());
        }
    }

    let recent_runs = storage
        .list_recent_experience_runs_any_scope(GEPA_AUTO_EVIDENCE_SCAN_LIMIT)
        .await?;
    let since = gepa_auto_latest_activity_at(&auto_state);
    let usable_runs = recent_runs
        .iter()
        .filter(|run| !run.success_state.trim().eq_ignore_ascii_case("provisional"));
    let fresh_evidence = usable_runs
        .filter(|run| {
            since
                .map(|cutoff| {
                    parse_rfc3339_utc(&run.updated_at)
                        .map(|updated_at| updated_at > cutoff)
                        .unwrap_or(false)
                })
                .unwrap_or(true)
        })
        .count();
    if fresh_evidence < GEPA_AUTO_MIN_FRESH_EXPERIENCE_RUNS {
        save_gepa_auto_skip(
            &storage,
            auto_state,
            now,
            "waiting_for_more_evidence",
            fresh_evidence,
            None,
        )
        .await;
        return Ok(());
    }

    let pending_path = agent
        .queue_gepa_seed_run(
            "Generate safer prompt candidates from recent private ArkEvolve evidence.",
            GEPA_AUTO_QUIET_WINDOW_SECS,
        )
        .await?;
    let mut next_state =
        crate::core::self_evolve::gepa_bridge::load_gepa_auto_run_state(&storage).await;
    next_state.last_checked_at = Some(now.to_rfc3339());
    next_state.last_queued_at = Some(now.to_rfc3339());
    next_state.last_status = Some("queued".to_string());
    next_state.last_reason = Some("queued_for_quiet_time".to_string());
    next_state.last_evidence_samples = fresh_evidence;
    next_state.next_check_after =
        Some((now + chrono::Duration::hours(GEPA_AUTO_COOLDOWN_HOURS)).to_rfc3339());
    crate::core::self_evolve::gepa_bridge::save_gepa_auto_run_state(&storage, &next_state).await?;
    record_background_learning_job_result(
        &storage,
        &BackgroundLearningJobUpdate {
            key: "gepa_optimizer".to_string(),
            status: "queued".to_string(),
            started_at: Some(now.to_rfc3339()),
            completed_at: None,
            summary: "Background prompt improvement queued for quiet time.".to_string(),
            changed: false,
            stats: serde_json::json!({
                "pending_job_path": pending_path,
                "fresh_evidence_samples": fresh_evidence,
                "quiet_window_seconds": GEPA_AUTO_QUIET_WINDOW_SECS,
            }),
        },
    )
    .await;
    Ok(())
}

async fn gepa_auto_loop(state: AppState) {
    tokio::time::sleep(std::time::Duration::from_secs(GEPA_AUTO_INITIAL_DELAY_SECS)).await;
    let mut interval = tokio::time::interval(std::time::Duration::from_secs(GEPA_AUTO_POLL_SECS));
    interval.set_missed_tick_behavior(tokio::time::MissedTickBehavior::Delay);
    loop {
        interval.tick().await;
        if let Err(error) = run_gepa_auto_tick(&state).await {
            tracing::warn!("GEPA auto-run scheduler tick failed: {}", error);
        }
    }
}

pub(super) fn spawn_gepa_auto_loop(state: AppState) {
    if GEPA_AUTO_LOOP_STARTED
        .compare_exchange(false, true, Ordering::AcqRel, Ordering::Acquire)
        .is_err()
    {
        return;
    }
    crate::spawn_logged!(
        "src/channels/http/evolution_control.rs:gepa_auto_loop",
        async move {
            gepa_auto_loop(state).await;
        }
    );
}

async fn run_guided_routing_optimization(
    state: &AppState,
    storage: &crate::storage::Storage,
) -> std::result::Result<String, String> {
    let enabled = storage
        .get(crate::core::self_evolve::strategy_runtime::SELF_EVOLVE_ENABLED_KEY)
        .await
        .ok()
        .flatten()
        .and_then(|raw| String::from_utf8(raw).ok())
        .map(|s| !s.trim().eq_ignore_ascii_case("false"))
        .unwrap_or(true)
        && crate::core::learning::load_learning_enabled(storage).await;
    if !enabled {
        return Err(
            "ArkEvolve is off. Turn on Self-evolve before running optimization.".to_string(),
        );
    }

    let llm = {
        let agent = state.agent.read().await;
        agent.llm.clone()
    };
    let current_policy_raw = storage
        .get(crate::core::self_evolve::ROUTING_COMPLEXITY_POLICY_KEY)
        .await
        .map_err(|error| format!("Failed to load current routing policy: {}", error))?;
    let config = crate::core::self_evolve::PolicyEvolutionConfig {
        project_root: resolve_project_root(),
        ..Default::default()
    };
    let engine = crate::core::self_evolve::PolicyEvolutionEngine::new(config, llm);
    let result = engine
        .evolve_routing_policy(
            "Improve AgentArk turn-routing accuracy from recent typed turn evidence. Keep accuracy and safety ahead of token, latency, and cost savings.",
            current_policy_raw.as_deref(),
        )
        .await
        .map_err(|error| format!("Optimization failed: {}", error))?;

    let mut promotion_mode = "none";
    let mut canary_state: Option<crate::core::self_evolve::strategy_runtime::CanaryRolloutState> =
        None;
    let mut replay_result: Option<
        crate::core::self_evolve::strategy_runtime::ReplayEvaluationResult,
    > = None;

    if result.promoted {
        if let Some(policy_json) = result.promoted_policy.as_ref() {
            let candidate_serialized = serde_json::to_vec(policy_json)
                .map_err(|error| format!("Failed to encode candidate policy: {}", error))?;
            if let Some(existing_baseline) = current_policy_raw.as_ref() {
                storage
                    .set(
                        crate::core::self_evolve::strategy_runtime::ROUTING_COMPLEXITY_POLICY_BASELINE_SNAPSHOT_KEY,
                        existing_baseline,
                    )
                    .await
                    .map_err(|error| {
                        format!("Failed to snapshot current routing policy: {}", error)
                    })?;
            }

            let baseline_version = storage
                .get(
                    crate::core::self_evolve::strategy_runtime::ROUTING_COMPLEXITY_CANARY_STATE_KEY,
                )
                .await
                .ok()
                .flatten()
                .and_then(|raw| {
                    serde_json::from_slice::<
                        crate::core::self_evolve::strategy_runtime::CanaryRolloutState,
                    >(&raw)
                    .ok()
                    .map(|state| state.baseline_version)
                })
                .unwrap_or_else(|| "routing-policy-default-v1".to_string());
            let candidate_version = format!("routing-candidate-{}", result.lineage_entry_id);

            storage
                .set(
                    crate::core::self_evolve::strategy_runtime::ROUTING_COMPLEXITY_POLICY_CANARY_KEY,
                    &candidate_serialized,
                )
                .await
                .map_err(|error| format!("Failed to save candidate routing policy: {}", error))?;
            let state = crate::core::self_evolve::strategy_runtime::CanaryRolloutState {
                enabled: true,
                baseline_version,
                candidate_version,
                rollout_percent: 20,
                min_samples_per_version: 25,
                min_success_gain: 0.03,
                max_sign_test_p_value: 0.10,
                activated_at: Some(chrono::Utc::now().to_rfc3339()),
            };
            let state_bytes = serde_json::to_vec(&state)
                .map_err(|error| format!("Failed to encode canary state: {}", error))?;
            storage
                .set(
                    crate::core::self_evolve::strategy_runtime::ROUTING_COMPLEXITY_CANARY_STATE_KEY,
                    &state_bytes,
                )
                .await
                .map_err(|error| format!("Failed to activate routing canary: {}", error))?;
            canary_state = Some(state.clone());
            promotion_mode = "canary";

            if let Ok(logs) = storage
                .list_operational_logs_by_event("tool_call", 4_000)
                .await
            {
                let replay_eval =
                    crate::core::self_evolve::strategy_runtime::evaluate_canary_by_policy_version(
                        &logs,
                        &state.baseline_version,
                        &state.candidate_version,
                        state.min_samples_per_version,
                        state.min_success_gain,
                        state.max_sign_test_p_value,
                    );
                if replay_eval.promote {
                    promotion_mode = "canary";
                }
                replay_result = Some(replay_eval);
            }
        }
    }

    let mut value = serde_json::to_value(&result)
        .map_err(|error| format!("Failed to serialize optimization result: {}", error))?;
    if let serde_json::Value::Object(obj) = &mut value {
        obj.insert("mode".to_string(), serde_json::json!("policy"));
        obj.insert(
            "promotion_applied".to_string(),
            serde_json::json!(promotion_mode != "none"),
        );
        obj.insert(
            "apply_promotion_requested".to_string(),
            serde_json::json!(true),
        );
        obj.insert(
            "promotion_mode".to_string(),
            serde_json::json!(promotion_mode),
        );
        obj.insert(
            "canary_state".to_string(),
            serde_json::to_value(&canary_state).unwrap_or(serde_json::Value::Null),
        );
        obj.insert(
            "replay_evaluation".to_string(),
            serde_json::to_value(&replay_result).unwrap_or(serde_json::Value::Null),
        );
    }
    if let Ok(bytes) = serde_json::to_vec(&value) {
        let _ = storage
            .set(
                crate::core::self_evolve::strategy_runtime::SELF_EVOLVE_LAST_RESULT_KEY,
                &bytes,
            )
            .await;
    }

    if !result.success {
        return Ok(format!(
            "Optimization ran but could not finish: {}",
            result.error.as_deref().unwrap_or("unknown error")
        ));
    }
    if promotion_mode == "canary" {
        return Ok(format!(
            "Optimization found a routing improvement and started a 20% canary. Accuracy changed from {:.0}% to {:.0}%.",
            result.baseline_accuracy * 100.0,
            result.best_candidate_accuracy * 100.0,
        ));
    }
    Ok(format!(
        "Optimization checked {} routing candidates. No candidate beat the current behavior, so nothing changed.",
        result.evaluated_candidates
    ))
}

pub(super) async fn run_evolution_dev_action(
    State(state): State<AppState>,
    Json(request): Json<EvolutionDevActionRequest>,
) -> Response {
    let (storage, agent_config, primary_model_id) = {
        let agent = state.agent.read().await;
        (
            agent.storage.clone(),
            agent.config.clone(),
            agent.primary_model_id.clone(),
        )
    };
    let project_root = resolve_project_root();
    let action = request.action.trim().to_ascii_lowercase();

    let message = match action.as_str() {
        "run_gepa_seed" => {
            let agent = {
                let agent = state.agent.read().await;
                agent.clone()
            };
            let trace_ref = Arc::new(RwLock::new(ExecutionTrace::default()));
            let call = crate::core::ToolCall {
                id: uuid::Uuid::new_v4().to_string(),
                name: "self_evolve".to_string(),
                arguments: serde_json::json!({
                    "mode": "gepa_run",
                    "request": "Use recent ArkEvolve evidence to generate GEPA prompt candidates.",
                    "gepa_quiet_window_seconds": 60,
                    "apply_promotion": false,
                    "import_after_run": true,
                }),
            };
            let raw = match agent
                .handle_self_evolve_tool_call(&call, &trace_ref, None)
                .await
            {
                Ok(raw) => raw,
                Err(error) => {
                    return (
                        StatusCode::BAD_REQUEST,
                        Json(ErrorResponse {
                            error: format!("GEPA run failed: {}", error),
                        }),
                    )
                        .into_response();
                }
            };
            let result = serde_json::from_str::<serde_json::Value>(&raw)
                .unwrap_or_else(|_| serde_json::json!({ "raw": raw }));
            let status = result
                .get("status")
                .and_then(|value| value.as_str())
                .unwrap_or("completed");
            match status {
                "queued" => "GEPA run queued and will start when AgentArk is idle.".to_string(),
                "blocked" => result
                    .get("error")
                    .and_then(|value| value.as_str())
                    .map(|error| format!("GEPA run blocked: {}", error))
                    .unwrap_or_else(|| {
                        "GEPA run was blocked by readiness or budget gates.".to_string()
                    }),
                "completed" => {
                    "GEPA run completed and tracked candidates for user review; no runtime behavior changed.".to_string()
                }
                "timed_out" => "GEPA run timed out; it was recorded for review.".to_string(),
                "failed" => result
                    .get("stderr_tail")
                    .and_then(|value| value.as_str())
                    .map(|error| format!("GEPA run failed: {}", error))
                    .unwrap_or_else(|| "GEPA run failed; check ArkEvolve status.".to_string()),
                _ => "GEPA run finished; check ArkEvolve status for details.".to_string(),
            }
        }
        "run_guided_optimization" => {
            match run_guided_routing_optimization(&state, &storage).await {
                Ok(message) => message,
                Err(error) => {
                    return (StatusCode::BAD_REQUEST, Json(ErrorResponse { error }))
                        .into_response();
                }
            }
        }
        "disable_canary" => {
            if let Some(candidate_id) = request
                .candidate_id
                .as_deref()
                .map(str::trim)
                .filter(|value| !value.is_empty())
            {
                let Some(candidate) = (match storage.get_learning_candidate(candidate_id).await {
                    Ok(value) => value,
                    Err(e) => {
                        return (
                            StatusCode::INTERNAL_SERVER_ERROR,
                            Json(ErrorResponse {
                                error: format!("Failed to load learning candidate: {}", e),
                            }),
                        )
                            .into_response();
                    }
                }) else {
                    return (
                        StatusCode::NOT_FOUND,
                        Json(ErrorResponse {
                            error: "Learning candidate not found.".to_string(),
                        }),
                    )
                        .into_response();
                };
                if candidate.candidate_type != "strategy" {
                    return (
                        StatusCode::BAD_REQUEST,
                        Json(ErrorResponse {
                            error: "candidate_id is only supported for strategy canary controls."
                                .to_string(),
                        }),
                    )
                        .into_response();
                }
                let profile = match parse_tool_strategy_candidate_profile(&candidate) {
                    Ok(profile) => profile,
                    Err(error) => {
                        return (
                            StatusCode::BAD_REQUEST,
                            Json(ErrorResponse {
                                error: error.to_string(),
                            }),
                        )
                            .into_response();
                    }
                };
                match disable_tool_strategy_canary_for_version(&storage, &profile.version).await {
                    Ok(true) => {
                        format!("Tool-strategy canary disabled for '{}'.", profile.version)
                    }
                    Ok(false) => {
                        return (
                            StatusCode::BAD_REQUEST,
                            Json(ErrorResponse {
                                error:
                                    "No matching tool-strategy canary is active for that candidate."
                                        .to_string(),
                            }),
                        )
                            .into_response();
                    }
                    Err(error) => {
                        return (
                            StatusCode::INTERNAL_SERVER_ERROR,
                            Json(ErrorResponse {
                                error: format!("Failed to disable tool-strategy canary: {}", error),
                            }),
                        )
                            .into_response();
                    }
                }
            } else {
                let mut canary = match load_evolution_canary_state(&storage).await {
                    Some(state) => state,
                    None => {
                        return (
                            StatusCode::BAD_REQUEST,
                            Json(ErrorResponse {
                                error: "No canary state found.".to_string(),
                            }),
                        )
                            .into_response();
                    }
                };
                canary.enabled = false;
                let bytes = match serde_json::to_vec(&canary) {
                    Ok(v) => v,
                    Err(e) => {
                        return (
                            StatusCode::INTERNAL_SERVER_ERROR,
                            Json(ErrorResponse {
                                error: format!("Failed to serialize canary state: {}", e),
                            }),
                        )
                            .into_response();
                    }
                };
                if let Err(e) = storage
                    .set(
                        crate::core::self_evolve::strategy_runtime::ROUTING_COMPLEXITY_CANARY_STATE_KEY,
                        &bytes,
                    )
                    .await
                {
                    return (
                        StatusCode::INTERNAL_SERVER_ERROR,
                        Json(ErrorResponse {
                            error: format!("Failed to disable canary: {}", e),
                        }),
                    )
                        .into_response();
                }
                "Canary rollout disabled.".to_string()
            }
        }
        "promote_candidate" => {
            if let Some(candidate_id) = request
                .candidate_id
                .as_deref()
                .map(str::trim)
                .filter(|value| !value.is_empty())
            {
                let Some(candidate) = (match storage.get_learning_candidate(candidate_id).await {
                    Ok(value) => value,
                    Err(e) => {
                        return (
                            StatusCode::INTERNAL_SERVER_ERROR,
                            Json(ErrorResponse {
                                error: format!("Failed to load learning candidate: {}", e),
                            }),
                        )
                            .into_response();
                    }
                }) else {
                    return (
                        StatusCode::NOT_FOUND,
                        Json(ErrorResponse {
                            error: "Learning candidate not found.".to_string(),
                        }),
                    )
                        .into_response();
                };
                if candidate.candidate_type != "strategy" {
                    return (
                        StatusCode::BAD_REQUEST,
                        Json(ErrorResponse {
                            error: "candidate_id is only supported for strategy promotions."
                                .to_string(),
                        }),
                    )
                        .into_response();
                }
                let replay_gate =
                    match crate::core::self_evolve::replay_gate::evaluate_candidate_replay_gate(
                        &storage, &candidate,
                    )
                    .await
                    {
                        Ok(gate) => gate,
                        Err(error) => {
                            return (
                                StatusCode::INTERNAL_SERVER_ERROR,
                                Json(ErrorResponse {
                                    error: format!(
                                        "Failed to evaluate strategy replay gate: {}",
                                        error
                                    ),
                                }),
                            )
                                .into_response();
                        }
                    };
                if !replay_gate.allow_approval {
                    return (
                        StatusCode::BAD_REQUEST,
                        Json(ErrorResponse {
                            error: format!(
                                "Replay gate blocked promotion for '{}': {}",
                                candidate.title, replay_gate.reason
                            ),
                        }),
                    )
                        .into_response();
                }
                let promoted_version =
                    match promote_tool_strategy_candidate_to_baseline(&storage, &candidate).await {
                        Ok(version) => version,
                        Err(error) => {
                            return (
                                StatusCode::BAD_REQUEST,
                                Json(ErrorResponse {
                                    error: format!(
                                        "Failed to promote tool-strategy candidate: {}",
                                        error
                                    ),
                                }),
                            )
                                .into_response();
                        }
                    };
                format!(
                    "Tool-strategy candidate '{}' promoted to baseline.",
                    promoted_version
                )
            } else {
                let candidate_bytes = match storage
                    .get(crate::core::self_evolve::strategy_runtime::ROUTING_COMPLEXITY_POLICY_CANARY_KEY)
                    .await
                {
                    Ok(Some(v)) => v,
                    _ => {
                        return (
                            StatusCode::BAD_REQUEST,
                            Json(ErrorResponse {
                                error: "No candidate policy found to promote.".to_string(),
                            }),
                        )
                            .into_response();
                    }
                };
                if let Err(e) = storage
                    .set(
                        crate::core::self_evolve::ROUTING_COMPLEXITY_POLICY_KEY,
                        &candidate_bytes,
                    )
                    .await
                {
                    return (
                        StatusCode::INTERNAL_SERVER_ERROR,
                        Json(ErrorResponse {
                            error: format!("Failed to promote candidate: {}", e),
                        }),
                    )
                        .into_response();
                }
                if let Some(mut canary) = load_evolution_canary_state(&storage).await {
                    canary.enabled = false;
                    canary.baseline_version = canary.candidate_version.clone();
                    if let Ok(bytes) = serde_json::to_vec(&canary) {
                        let _ = storage
                            .set(
                                crate::core::self_evolve::strategy_runtime::ROUTING_COMPLEXITY_CANARY_STATE_KEY,
                                &bytes,
                            )
                            .await;
                    }
                }
                "Candidate policy promoted to baseline.".to_string()
            }
        }
        "rollback_baseline" => {
            if let Some(candidate_id) = request
                .candidate_id
                .as_deref()
                .map(str::trim)
                .filter(|value| !value.is_empty())
            {
                let Some(candidate) = (match storage.get_learning_candidate(candidate_id).await {
                    Ok(value) => value,
                    Err(e) => {
                        return (
                            StatusCode::INTERNAL_SERVER_ERROR,
                            Json(ErrorResponse {
                                error: format!("Failed to load learning candidate: {}", e),
                            }),
                        )
                            .into_response();
                    }
                }) else {
                    return (
                        StatusCode::NOT_FOUND,
                        Json(ErrorResponse {
                            error: "Learning candidate not found.".to_string(),
                        }),
                    )
                        .into_response();
                };
                if candidate.candidate_type != "strategy" {
                    return (
                        StatusCode::BAD_REQUEST,
                        Json(ErrorResponse {
                            error: "candidate_id is only supported for strategy rollback."
                                .to_string(),
                        }),
                    )
                        .into_response();
                }
                let restored_version = match rollback_tool_strategy_baseline(&storage).await {
                    Ok(version) => version,
                    Err(error) => {
                        return (
                            StatusCode::BAD_REQUEST,
                            Json(ErrorResponse {
                                error: format!(
                                    "Failed to rollback tool-strategy baseline: {}",
                                    error
                                ),
                            }),
                        )
                            .into_response();
                    }
                };
                format!(
                    "Tool-strategy baseline rolled back to '{}'.",
                    restored_version
                )
            } else {
                let snapshot = match storage
                    .get(
                        crate::core::self_evolve::strategy_runtime::ROUTING_COMPLEXITY_POLICY_BASELINE_SNAPSHOT_KEY,
                    )
                    .await
                {
                    Ok(Some(v)) => v,
                    _ => {
                        return (
                            StatusCode::BAD_REQUEST,
                            Json(ErrorResponse {
                                error: "No baseline snapshot available for rollback.".to_string(),
                            }),
                        )
                            .into_response();
                    }
                };
                if let Err(e) = storage
                    .set(
                        crate::core::self_evolve::ROUTING_COMPLEXITY_POLICY_KEY,
                        &snapshot,
                    )
                    .await
                {
                    return (
                        StatusCode::INTERNAL_SERVER_ERROR,
                        Json(ErrorResponse {
                            error: format!("Failed to rollback baseline policy: {}", e),
                        }),
                    )
                        .into_response();
                }
                if let Some(mut canary) = load_evolution_canary_state(&storage).await {
                    canary.enabled = false;
                    if let Ok(bytes) = serde_json::to_vec(&canary) {
                        let _ = storage
                            .set(
                                crate::core::self_evolve::strategy_runtime::ROUTING_COMPLEXITY_CANARY_STATE_KEY,
                                &bytes,
                            )
                            .await;
                    }
                }
                "Rolled back to the stored baseline snapshot.".to_string()
            }
        }
        "approve_learning_candidate" => {
            let Some(candidate_id) = request
                .candidate_id
                .as_deref()
                .map(str::trim)
                .filter(|value| !value.is_empty())
            else {
                return (
                    StatusCode::BAD_REQUEST,
                    Json(ErrorResponse {
                        error: "candidate_id is required for learning approvals.".to_string(),
                    }),
                )
                    .into_response();
            };
            let Some(candidate) = (match storage.get_learning_candidate(candidate_id).await {
                Ok(value) => value,
                Err(e) => {
                    return (
                        StatusCode::INTERNAL_SERVER_ERROR,
                        Json(ErrorResponse {
                            error: format!("Failed to load learning candidate: {}", e),
                        }),
                    )
                        .into_response();
                }
            }) else {
                return (
                    StatusCode::NOT_FOUND,
                    Json(ErrorResponse {
                        error: "Learning candidate not found.".to_string(),
                    }),
                )
                    .into_response();
            };
            let candidate = if arkmemory_candidate_is_memory(&candidate.candidate_type) {
                let candidate =
                    match arkmemory_ensure_latest_open_candidate(&storage, &candidate).await {
                        Ok(candidate) => candidate,
                        Err(error) => {
                            return (
                                StatusCode::BAD_REQUEST,
                                Json(ErrorResponse {
                                    error: error.to_string(),
                                }),
                            )
                                .into_response();
                        }
                    };
                if candidate.approval_status != "draft" {
                    return (
                        StatusCode::BAD_REQUEST,
                        Json(ErrorResponse {
                            error: "Learning candidate is no longer pending review.".to_string(),
                        }),
                    )
                        .into_response();
                }
                candidate
            } else {
                candidate
            };

            let replay_gate =
                match crate::core::self_evolve::replay_gate::evaluate_candidate_replay_gate(
                    &storage, &candidate,
                )
                .await
                {
                    Ok(gate) => gate,
                    Err(error) => {
                        return (
                            StatusCode::INTERNAL_SERVER_ERROR,
                            Json(ErrorResponse {
                                error: format!(
                                    "Failed to evaluate learning replay gate: {}",
                                    error
                                ),
                            }),
                        )
                            .into_response();
                    }
                };
            if !replay_gate.allow_approval {
                return (
                    StatusCode::BAD_REQUEST,
                    Json(ErrorResponse {
                        error: format!(
                            "Replay gate blocked approval for '{}': {}",
                            candidate.title, replay_gate.reason
                        ),
                    }),
                )
                    .into_response();
            }
            let readiness_policy = crate::core::readiness::load_readiness_policy(&storage).await;
            let readiness = crate::core::readiness::evaluate_learning_candidate_readiness(
                &candidate,
                Some(&replay_gate),
                &readiness_policy,
            );
            if let Err(error) = crate::core::readiness::record_readiness_evaluation(
                &storage,
                "learning_candidate_approval",
                &candidate.id,
                &readiness,
            )
            .await
            {
                tracing::warn!(
                    candidate_id = %candidate.id,
                    error = %error,
                    "Failed to record learning candidate approval readiness"
                );
            }
            if !readiness.allows_review {
                return (
                    StatusCode::BAD_REQUEST,
                    Json(ErrorResponse {
                        error: format!(
                            "Readiness gate is still watching '{}': {}",
                            candidate.title, readiness.plain_summary
                        ),
                    }),
                )
                    .into_response();
            }

            let approved_ref = match candidate.candidate_type.as_str() {
                "workflow" => {
                    let name = candidate
                        .proposed_content
                        .get("name")
                        .and_then(|value| value.as_str())
                        .map(str::trim)
                        .filter(|value| !value.is_empty())
                        .ok_or_else(|| anyhow::anyhow!("workflow candidate missing proposed name"));
                    let content = candidate
                        .proposed_content
                        .get("content")
                        .and_then(|value| value.as_str())
                        .map(str::trim)
                        .filter(|value| !value.is_empty())
                        .ok_or_else(|| anyhow::anyhow!("workflow candidate missing content"));
                    let (name, content) = match (name, content) {
                        (Ok(name), Ok(content)) => (name, content),
                        (Err(error), _) | (_, Err(error)) => {
                            return (
                                StatusCode::BAD_REQUEST,
                                Json(ErrorResponse {
                                    error: error.to_string(),
                                }),
                            )
                                .into_response();
                        }
                    };
                    let agent = state.agent.read().await;
                    let semantic_review =
                        crate::security::skill_review::review_skill_import_with_configured_model(
                            &agent.llm,
                            &agent.config_dir,
                            "learning-candidate://workflow",
                            name,
                            content,
                        )
                        .await;
                    if semantic_review.policy.blocked {
                        return (
                            StatusCode::BAD_REQUEST,
                            Json(ErrorResponse {
                                error: "Workflow candidate was blocked by semantic skill security policy."
                                    .to_string(),
                            }),
                        )
                            .into_response();
                    }
                    let review = match agent
                        .runtime
                        .install_semantically_reviewed_action(
                            name,
                            content,
                            &semantic_review,
                            false,
                        )
                        .await
                    {
                        Ok(review) => review,
                        Err(error) => {
                            return (
                                StatusCode::BAD_REQUEST,
                                Json(ErrorResponse {
                                    error: format!(
                                        "Failed to materialize workflow candidate as a custom action: {}",
                                        error
                                    ),
                                }),
                            )
                                .into_response();
                        }
                    };
                    if !review.allow_load {
                        return (
                            StatusCode::BAD_REQUEST,
                            Json(ErrorResponse {
                                error:
                                    "Workflow candidate was blocked by the action security guard."
                                        .to_string(),
                            }),
                        )
                            .into_response();
                    }
                    name.to_string()
                }
                "skill_patch" => {
                    let action = skill_patch_string(&candidate, "action")
                        .ok_or_else(|| anyhow::anyhow!("skill patch candidate missing action"));
                    let skill_name = skill_patch_string(&candidate, "skill_name")
                        .ok_or_else(|| anyhow::anyhow!("skill patch candidate missing skill_name"));
                    let content = skill_patch_string(&candidate, "after_content")
                        .or_else(|| skill_patch_string(&candidate, "content"))
                        .ok_or_else(|| anyhow::anyhow!("skill patch candidate missing content"));
                    let (action, skill_name, content) = match (action, skill_name, content) {
                        (Ok(action), Ok(skill_name), Ok(content)) => (action, skill_name, content),
                        (Err(error), _, _) | (_, Err(error), _) | (_, _, Err(error)) => {
                            return (
                                StatusCode::BAD_REQUEST,
                                Json(ErrorResponse {
                                    error: error.to_string(),
                                }),
                            )
                                .into_response();
                        }
                    };
                    let evidence_markdown = build_skill_candidate_evidence_markdown(&candidate);
                    let agent = state.agent.read().await;
                    let semantic_review =
                        crate::security::skill_review::review_skill_import_with_configured_model(
                            &agent.llm,
                            &agent.config_dir,
                            "learning-candidate://skill-patch",
                            &skill_name,
                            &content,
                        )
                        .await;
                    if semantic_review.policy.blocked {
                        return (
                            StatusCode::BAD_REQUEST,
                            Json(ErrorResponse {
                                error: "Skill patch candidate was blocked by semantic skill security policy."
                                    .to_string(),
                            }),
                        )
                            .into_response();
                    }
                    let result = match agent
                        .runtime
                        .apply_semantically_reviewed_skill_evolution_candidate(
                            &action,
                            &skill_name,
                            &content,
                            &evidence_markdown,
                            &semantic_review,
                        )
                        .await
                    {
                        Ok(result) => result,
                        Err(error) => {
                            return (
                                StatusCode::BAD_REQUEST,
                                Json(ErrorResponse {
                                    error: format!(
                                        "Failed to apply skill evolution candidate: {}",
                                        error
                                    ),
                                }),
                            )
                                .into_response();
                        }
                    };
                    result.approved_ref
                }
                "strategy" => {
                    if let Err(error) = parse_tool_strategy_candidate_profile(&candidate) {
                        return (
                            StatusCode::BAD_REQUEST,
                            Json(ErrorResponse {
                                error: error.to_string(),
                            }),
                        )
                            .into_response();
                    }
                    match storage
                        .approve_strategy_learning_candidate(
                            candidate_id,
                            Some("Approved from Evolution developer controls."),
                        )
                        .await
                    {
                        Ok(version) => version,
                        Err(error) => {
                            return (
                                StatusCode::INTERNAL_SERVER_ERROR,
                                Json(ErrorResponse {
                                    error: format!(
                                        "Failed to approve strategy learning candidate: {}",
                                        error
                                    ),
                                }),
                            )
                                .into_response();
                        }
                    }
                }
                crate::core::self_evolve::ROUTING_CANONICAL_CANDIDATE_TYPE => {
                    let data_dir = {
                        let agent = state.agent.read().await;
                        agent.data_dir.clone()
                    };
                    match crate::core::self_evolve::routing_canonical_evolution::promote_routing_canonical_candidate(
                        &data_dir,
                        &candidate,
                    )
                    .await
                    {
                        Ok(promoted) => format!(
                            "{}:{promoted}",
                            crate::core::self_evolve::ROUTING_CANONICAL_SUBJECT_KEY
                        ),
                        Err(error) => {
                            return (
                                StatusCode::BAD_REQUEST,
                                Json(ErrorResponse {
                                    error: format!(
                                        "Failed to promote routing canonical candidate: {}",
                                        error
                                    ),
                                }),
                            )
                                .into_response();
                        }
                    }
                }
                "memory_add" | "memory_update" | "memory_retract" => {
                    let operation_id = candidate
                        .proposed_content
                        .get("operation_id")
                        .and_then(|value| value.as_str())
                        .map(str::trim)
                        .filter(|value| !value.is_empty());
                    let Some(operation_id) = operation_id else {
                        return (
                            StatusCode::BAD_REQUEST,
                            Json(ErrorResponse {
                                error: "memory operation candidate missing operation_id."
                                    .to_string(),
                            }),
                        )
                            .into_response();
                    };
                    let claimed = match storage
                        .update_learning_candidate_review_if_status(
                            candidate_id,
                            "draft",
                            "applying",
                            Some("Applying from Evolution developer controls."),
                            None,
                        )
                        .await
                    {
                        Ok(claimed) => claimed,
                        Err(error) => {
                            return (
                                StatusCode::INTERNAL_SERVER_ERROR,
                                Json(ErrorResponse {
                                    error: format!(
                                        "Failed to claim memory operation candidate: {}",
                                        error
                                    ),
                                }),
                            )
                                .into_response();
                        }
                    };
                    if !claimed {
                        return (
                            StatusCode::BAD_REQUEST,
                            Json(ErrorResponse {
                                error: "Learning candidate is no longer pending review."
                                    .to_string(),
                            }),
                        )
                            .into_response();
                    }
                    let agent = state.agent.read().await;
                    match agent
                        .apply_memory_operation_by_id_with_source(
                            operation_id,
                            "evolution_controls_review",
                        )
                        .await
                    {
                        Ok(approved_ref) => {
                            match storage
                                .update_learning_candidate_review_if_status(
                                    candidate_id,
                                    "applying",
                                    "approved",
                                    Some("Approved from Evolution developer controls."),
                                    Some(&approved_ref),
                                )
                                .await
                            {
                                Ok(true) => approved_ref,
                                Ok(false) => {
                                    return (
                                        StatusCode::BAD_REQUEST,
                                        Json(ErrorResponse {
                                            error: "Learning candidate changed while it was being applied."
                                                .to_string(),
                                        }),
                                    )
                                        .into_response();
                                }
                                Err(error) => {
                                    return (
                                        StatusCode::INTERNAL_SERVER_ERROR,
                                        Json(ErrorResponse {
                                            error: format!(
                                                "Failed to record memory operation approval: {}",
                                                error
                                            ),
                                        }),
                                    )
                                        .into_response();
                                }
                            }
                        }
                        Err(error) => {
                            let note = format!("Apply failed: {}", error);
                            let _ = storage
                                .update_learning_candidate_review_if_status(
                                    candidate_id,
                                    "applying",
                                    "draft",
                                    Some(&note),
                                    None,
                                )
                                .await;
                            return (
                                StatusCode::INTERNAL_SERVER_ERROR,
                                Json(ErrorResponse {
                                    error: format!(
                                        "Failed to apply memory operation candidate: {}",
                                        error
                                    ),
                                }),
                            )
                                .into_response();
                        }
                    }
                }
                "memory_deprecate" => {
                    let item_id = candidate
                        .proposed_content
                        .get("item_id")
                        .and_then(|value| value.as_str())
                        .map(str::trim)
                        .filter(|value| !value.is_empty());
                    let next_status = candidate
                        .proposed_content
                        .get("next_status")
                        .and_then(|value| value.as_str())
                        .unwrap_or("deprecated");
                    let Some(item_id) = item_id else {
                        return (
                            StatusCode::BAD_REQUEST,
                            Json(ErrorResponse {
                                error: "memory_deprecate candidate missing item_id.".to_string(),
                            }),
                        )
                            .into_response();
                    };
                    if let Err(error) = storage
                        .update_experience_item_status(item_id, next_status)
                        .await
                    {
                        return (
                            StatusCode::INTERNAL_SERVER_ERROR,
                            Json(ErrorResponse {
                                error: format!(
                                    "Failed to apply memory deprecation candidate: {}",
                                    error
                                ),
                            }),
                        )
                            .into_response();
                    }
                    item_id.to_string()
                }
                "memory_merge" => {
                    let target_item_id = candidate
                        .proposed_content
                        .get("target_item_id")
                        .and_then(|value| value.as_str())
                        .map(str::trim)
                        .filter(|value| !value.is_empty());
                    let source_item_id = candidate
                        .proposed_content
                        .get("source_item_id")
                        .and_then(|value| value.as_str())
                        .map(str::trim)
                        .filter(|value| !value.is_empty());
                    let (Some(target_item_id), Some(source_item_id)) =
                        (target_item_id, source_item_id)
                    else {
                        return (
                            StatusCode::BAD_REQUEST,
                            Json(ErrorResponse {
                                error: "memory_merge candidate missing source/target item ids."
                                    .to_string(),
                            }),
                        )
                            .into_response();
                    };
                    if let Err(error) = storage
                        .update_experience_item_status(source_item_id, "deprecated")
                        .await
                    {
                        return (
                            StatusCode::INTERNAL_SERVER_ERROR,
                            Json(ErrorResponse {
                                error: format!(
                                    "Failed to deprecate merged source memory: {}",
                                    error
                                ),
                            }),
                        )
                            .into_response();
                    }
                    if let Err(error) = storage
                        .upsert_experience_edge(&crate::storage::experience_edge::Model {
                            id: uuid::Uuid::new_v4().to_string(),
                            source_ref: target_item_id.to_string(),
                            source_kind: "experience_item".to_string(),
                            target_ref: source_item_id.to_string(),
                            target_kind: "experience_item".to_string(),
                            edge_type: "supersedes".to_string(),
                            weight: 1.0,
                            source_run_id: None,
                            metadata: serde_json::json!({ "approved_via": "evolution" }),
                            created_at: chrono::Utc::now().to_rfc3339(),
                            updated_at: chrono::Utc::now().to_rfc3339(),
                        })
                        .await
                    {
                        return (
                            StatusCode::INTERNAL_SERVER_ERROR,
                            Json(ErrorResponse {
                                error: format!("Failed to connect merged memory edge: {}", error),
                            }),
                        )
                            .into_response();
                    }
                    target_item_id.to_string()
                }
                other => {
                    return (
                        StatusCode::BAD_REQUEST,
                        Json(ErrorResponse {
                            error: format!("Unsupported learning candidate type '{}'.", other),
                        }),
                    )
                        .into_response();
                }
            };
            if candidate.candidate_type != "strategy"
                && !matches!(
                    candidate.candidate_type.as_str(),
                    "memory_add" | "memory_update" | "memory_retract"
                )
            {
                if let Err(error) = storage
                    .update_learning_candidate_review(
                        candidate_id,
                        "approved",
                        Some("Approved from Evolution developer controls."),
                        Some(&approved_ref),
                    )
                    .await
                {
                    return (
                        StatusCode::INTERNAL_SERVER_ERROR,
                        Json(ErrorResponse {
                            error: format!("Failed to record candidate approval: {}", error),
                        }),
                    )
                        .into_response();
                }
            }
            format!("Approved learning candidate '{}'.", candidate.title)
        }
        "reject_learning_candidate" => {
            let Some(candidate_id) = request
                .candidate_id
                .as_deref()
                .map(str::trim)
                .filter(|value| !value.is_empty())
            else {
                return (
                    StatusCode::BAD_REQUEST,
                    Json(ErrorResponse {
                        error: "candidate_id is required for learning rejections.".to_string(),
                    }),
                )
                    .into_response();
            };
            let Some(candidate) = (match storage.get_learning_candidate(candidate_id).await {
                Ok(value) => value,
                Err(e) => {
                    return (
                        StatusCode::INTERNAL_SERVER_ERROR,
                        Json(ErrorResponse {
                            error: format!("Failed to load learning candidate: {}", e),
                        }),
                    )
                        .into_response();
                }
            }) else {
                return (
                    StatusCode::NOT_FOUND,
                    Json(ErrorResponse {
                        error: "Learning candidate not found.".to_string(),
                    }),
                )
                    .into_response();
            };
            let candidate = if arkmemory_candidate_is_memory(&candidate.candidate_type) {
                let candidate =
                    match arkmemory_ensure_latest_open_candidate(&storage, &candidate).await {
                        Ok(candidate) => candidate,
                        Err(error) => {
                            return (
                                StatusCode::BAD_REQUEST,
                                Json(ErrorResponse {
                                    error: error.to_string(),
                                }),
                            )
                                .into_response();
                        }
                    };
                if candidate.approval_status != "draft" {
                    return (
                        StatusCode::BAD_REQUEST,
                        Json(ErrorResponse {
                            error: "Learning candidate is no longer pending review.".to_string(),
                        }),
                    )
                        .into_response();
                }
                candidate
            } else {
                candidate
            };
            if candidate.candidate_type == "strategy" {
                if let Err(error) = parse_tool_strategy_candidate_profile(&candidate) {
                    return (
                        StatusCode::BAD_REQUEST,
                        Json(ErrorResponse {
                            error: error.to_string(),
                        }),
                    )
                        .into_response();
                }
                if let Err(error) = storage
                    .reject_strategy_learning_candidate(
                        candidate_id,
                        Some("Rejected from Evolution developer controls."),
                    )
                    .await
                {
                    return (
                        StatusCode::INTERNAL_SERVER_ERROR,
                        Json(ErrorResponse {
                            error: format!(
                                "Failed to reject strategy learning candidate: {}",
                                error
                            ),
                        }),
                    )
                        .into_response();
                }
            } else if arkmemory_candidate_is_memory(&candidate.candidate_type) {
                match storage
                    .update_learning_candidate_review_if_status(
                        candidate_id,
                        "draft",
                        "rejected",
                        Some("Rejected from Evolution developer controls."),
                        None,
                    )
                    .await
                {
                    Ok(true) => {}
                    Ok(false) => {
                        return (
                            StatusCode::BAD_REQUEST,
                            Json(ErrorResponse {
                                error: "Learning candidate is no longer pending review."
                                    .to_string(),
                            }),
                        )
                            .into_response();
                    }
                    Err(error) => {
                        return (
                            StatusCode::INTERNAL_SERVER_ERROR,
                            Json(ErrorResponse {
                                error: format!("Failed to record candidate rejection: {}", error),
                            }),
                        )
                            .into_response();
                    }
                }
            } else if let Err(error) = storage
                .update_learning_candidate_review(
                    candidate_id,
                    "rejected",
                    Some("Rejected from Evolution developer controls."),
                    None,
                )
                .await
            {
                return (
                    StatusCode::INTERNAL_SERVER_ERROR,
                    Json(ErrorResponse {
                        error: format!("Failed to record candidate rejection: {}", error),
                    }),
                )
                    .into_response();
            }
            if arkmemory_candidate_is_memory(&candidate.candidate_type) {
                if let Some(operation_id) = candidate
                    .proposed_content
                    .get("operation_id")
                    .and_then(|value| value.as_str())
                    .map(str::trim)
                    .filter(|value| !value.is_empty())
                {
                    if let Ok(Some(mut operation)) =
                        storage.get_memory_operation(operation_id).await
                    {
                        operation.status = "rejected".to_string();
                        operation.reviewed_at = Some(chrono::Utc::now().to_rfc3339());
                        operation.review_notes =
                            Some("Rejected from Evolution developer controls.".to_string());
                        operation.updated_at = chrono::Utc::now().to_rfc3339();
                        let _ = storage.upsert_memory_operation(&operation).await;
                    }
                }
            }
            format!("Rejected learning candidate '{}'.", candidate.title)
        }
        "approve_prompt_optimization_proposal" => {
            let Some(proposal_id) = request
                .candidate_id
                .as_deref()
                .map(str::trim)
                .filter(|value| !value.is_empty())
            else {
                return (
                    StatusCode::BAD_REQUEST,
                    Json(ErrorResponse {
                        error: "candidate_id is required for prompt optimization approvals."
                            .to_string(),
                    }),
                )
                    .into_response();
            };
            if let Err(error) =
                update_prompt_optimization_review_state(&storage, proposal_id, "approved").await
            {
                return (
                    StatusCode::INTERNAL_SERVER_ERROR,
                    Json(ErrorResponse {
                        error: format!("Failed to record prompt optimization approval: {}", error),
                    }),
                )
                    .into_response();
            }
            format!(
                "Recorded approval for prompt optimization proposal '{}'. No runtime prompt behavior changed.",
                proposal_id
            )
        }
        "reject_prompt_optimization_proposal" => {
            let Some(proposal_id) = request
                .candidate_id
                .as_deref()
                .map(str::trim)
                .filter(|value| !value.is_empty())
            else {
                return (
                    StatusCode::BAD_REQUEST,
                    Json(ErrorResponse {
                        error: "candidate_id is required for prompt optimization rejections."
                            .to_string(),
                    }),
                )
                    .into_response();
            };
            if let Err(error) =
                update_prompt_optimization_review_state(&storage, proposal_id, "rejected").await
            {
                return (
                    StatusCode::INTERNAL_SERVER_ERROR,
                    Json(ErrorResponse {
                        error: format!("Failed to record prompt optimization rejection: {}", error),
                    }),
                )
                    .into_response();
            }
            format!(
                "Recorded rejection for prompt optimization proposal '{}'. Runtime prompt behavior remains unchanged.",
                proposal_id
            )
        }
        "disable_prompt_canary" => {
            let Some(surface) = request
                .candidate_id
                .as_deref()
                .map(str::trim)
                .filter(|value| !value.is_empty())
            else {
                return (
                    StatusCode::BAD_REQUEST,
                    Json(ErrorResponse {
                        error:
                            "candidate_id must be prompt, specialist_prompt, or prompt_fragment."
                                .to_string(),
                    }),
                )
                    .into_response();
            };
            match disable_prompt_canary_for_surface(&storage, surface).await {
                Ok(message) => message,
                Err(error) => {
                    return (
                        StatusCode::BAD_REQUEST,
                        Json(ErrorResponse {
                            error: error.to_string(),
                        }),
                    )
                        .into_response();
                }
            }
        }
        "promote_prompt_canary_candidate" => {
            let Some(surface) = request
                .candidate_id
                .as_deref()
                .map(str::trim)
                .filter(|value| !value.is_empty())
            else {
                return (
                    StatusCode::BAD_REQUEST,
                    Json(ErrorResponse {
                        error:
                            "candidate_id must be prompt, specialist_prompt, or prompt_fragment."
                                .to_string(),
                    }),
                )
                    .into_response();
            };
            match promote_prompt_canary_to_baseline(&storage, surface).await {
                Ok(message) => message,
                Err(error) => {
                    return (
                        StatusCode::BAD_REQUEST,
                        Json(ErrorResponse {
                            error: error.to_string(),
                        }),
                    )
                        .into_response();
                }
            }
        }
        "rollback_prompt_baseline" => {
            let Some(surface) = request
                .candidate_id
                .as_deref()
                .map(str::trim)
                .filter(|value| !value.is_empty())
            else {
                return (
                    StatusCode::BAD_REQUEST,
                    Json(ErrorResponse {
                        error:
                            "candidate_id must be prompt, specialist_prompt, or prompt_fragment."
                                .to_string(),
                    }),
                )
                    .into_response();
            };
            match rollback_prompt_baseline_for_surface(&storage, surface).await {
                Ok(message) => message,
                Err(error) => {
                    return (
                        StatusCode::BAD_REQUEST,
                        Json(ErrorResponse {
                            error: error.to_string(),
                        }),
                    )
                        .into_response();
                }
            }
        }
        "disable_prompt_canary_candidate" => {
            let Some(event_id) = request
                .candidate_id
                .as_deref()
                .map(str::trim)
                .filter(|value| !value.is_empty())
            else {
                return (
                    StatusCode::BAD_REQUEST,
                    Json(ErrorResponse {
                        error: "candidate_id is required for prompt canary disable actions."
                            .to_string(),
                    }),
                )
                    .into_response();
            };
            let event = match disable_prompt_canary_from_safety_event(&storage, event_id).await {
                Ok(event) => event,
                Err(error) => {
                    return (
                        StatusCode::BAD_REQUEST,
                        Json(ErrorResponse {
                            error: error.to_string(),
                        }),
                    )
                        .into_response();
                }
            };
            format!(
                "Disabled {} canary for candidate '{}'.",
                event.surface_label, event.candidate_version
            )
        }
        "keep_prompt_canary_candidate" => {
            let Some(event_id) = request
                .candidate_id
                .as_deref()
                .map(str::trim)
                .filter(|value| !value.is_empty())
            else {
                return (
                    StatusCode::BAD_REQUEST,
                    Json(ErrorResponse {
                        error: "candidate_id is required to keep a prompt canary active."
                            .to_string(),
                    }),
                )
                    .into_response();
            };
            let event =
                match update_prompt_canary_safety_review_status(&storage, event_id, "kept_active")
                    .await
                {
                    Ok(event) => event,
                    Err(error) => {
                        return (
                            StatusCode::BAD_REQUEST,
                            Json(ErrorResponse {
                                error: error.to_string(),
                            }),
                        )
                            .into_response();
                    }
                };
            format!(
                "Recorded decision to keep {} canary '{}' active.",
                event.surface_label, event.candidate_version
            )
        }
        _ => {
            return (
                StatusCode::BAD_REQUEST,
                Json(ErrorResponse {
                    error: "Unsupported action. Use run_guided_optimization, disable_canary, promote_candidate, rollback_baseline, approve_learning_candidate, reject_learning_candidate, approve_prompt_optimization_proposal, reject_prompt_optimization_proposal, disable_prompt_canary, promote_prompt_canary_candidate, rollback_prompt_baseline, disable_prompt_canary_candidate, or keep_prompt_canary_candidate."
                        .to_string(),
                }),
            )
                .into_response();
        }
    };

    let evolution = build_evolution_settings_response(
        &storage,
        &agent_config,
        &primary_model_id,
        &project_root,
    )
    .await;
    let dev = build_evolution_dev_response(&storage, EVOLUTION_DEV_DEFAULT_LIMIT, false).await;
    let trace_id = persist_evolution_action_trace(
        &state,
        &action,
        &message,
        serde_json::json!({
            "trace_kind": "self_evolve.manual_action.result",
            "action": action.clone(),
            "message": message.clone(),
            "self_evolve_enabled": evolution.self_evolve_enabled,
            "deploy_guard_default": evolution.deploy_guard_default,
            "canary_state": dev.canary_state.clone(),
            "last_result": dev.last_result.clone(),
            "prompt_canary_safety_events": dev.prompt_canary_safety_events.clone(),
            "prompt_telemetry_summary": dev.prompt_telemetry_summary.clone(),
            "prompt_fragment_metrics": serde_json::to_value(&dev.prompt_fragment_metrics)
                .unwrap_or_else(|_| serde_json::Value::Array(Vec::new())),
            "prompt_fragment_canary_state": dev.prompt_fragment_canary_state.clone(),
            "prompt_optimization_opportunities": dev.prompt_optimization_opportunities.clone(),
        }),
    )
    .await;
    Json(serde_json::json!({
        "status": "ok",
        "message": message,
        "trace_id": trace_id,
        "evolution": evolution,
        "dev": dev
    }))
    .into_response()
}
