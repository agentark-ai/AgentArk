use super::*;

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "snake_case")]
pub(super) enum ContractInputCompleteness {
    Complete,
    Incomplete,
    Unknown,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "snake_case")]
pub(super) enum ContractIdempotency {
    Idempotent,
    CallerScoped,
    CreatesOrMutates,
    Unknown,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub(super) struct ToolContractSummary {
    pub action_name: String,
    #[serde(default)]
    pub required_input: Vec<String>,
    pub input_completeness: ContractInputCompleteness,
    pub side_effect_level: crate::actions::ActionSideEffectLevel,
    pub delivery_mode: crate::actions::ActionDeliveryMode,
    pub auth_required: bool,
    pub idempotency: ContractIdempotency,
    #[serde(default)]
    pub read_scopes: Vec<super::resource_locks::ResourceScope>,
    #[serde(default)]
    pub write_scopes: Vec<super::resource_locks::ResourceScope>,
    #[serde(default)]
    pub exclusive_scopes: Vec<super::resource_locks::ResourceScope>,
    pub cost: crate::actions::ActionCostTier,
    pub output_shape: String,
}

#[derive(Debug, Clone, Serialize, Deserialize, Default)]
pub(super) struct ToolContractFit {
    pub action_name: String,
    pub side_effect_match: bool,
    pub readiness_rank: u8,
    pub side_effect_rank: u8,
    pub input_rank: u8,
    pub auth_rank: u8,
    pub cost_rank: u8,
    pub support_rank: u8,
    #[serde(default)]
    pub notes: Vec<String>,
}

pub(super) fn contract_summary_for_action(
    action: &crate::actions::ActionDef,
    arguments: Option<&serde_json::Value>,
) -> ToolContractSummary {
    let metadata = action.action_metadata();
    let required_input = required_schema_fields(&action.input_schema);
    let required_options = required_schema_options(&action.input_schema);
    let input_completeness = match arguments {
        Some(arguments) => {
            if required_options
                .iter()
                .any(|fields| {
                    fields
                        .iter()
                        .all(|field| arguments.get(field).is_some_and(value_is_present))
                })
            {
                ContractInputCompleteness::Complete
            } else {
                ContractInputCompleteness::Incomplete
            }
        }
        None if required_input.is_empty() => ContractInputCompleteness::Complete,
        None => ContractInputCompleteness::Unknown,
    };
    let resource_contract =
        super::resource_locks::resource_contract_for_action(action, arguments);
    ToolContractSummary {
        action_name: action.name.clone(),
        required_input,
        input_completeness,
        side_effect_level: metadata.side_effect_level.clone(),
        delivery_mode: metadata.delivery_mode,
        auth_required: metadata.requires_auth
            || !action.authorization.access.integration_ids.is_empty()
            || !action.authorization.access.permission_ids.is_empty()
            || !action.authorization.access.channel_targets.is_empty(),
        idempotency: idempotency_for_action(action),
        read_scopes: resource_contract.read_scopes,
        write_scopes: resource_contract.write_scopes,
        exclusive_scopes: resource_contract.exclusive_scopes,
        cost: metadata.cost,
        output_shape: output_shape_for_action(action),
    }
}

pub(super) fn contract_fit_for_candidate(
    goal: &super::semantic_turn::SemanticGoal,
    action: &crate::actions::ActionDef,
    health: Option<&super::capability_health::CapabilityHealthEntry>,
) -> ToolContractFit {
    let metadata = action.action_metadata();
    let side_effect_match =
        super::semantic_turn::side_effect_matches(goal.side_effect, &metadata.side_effect_level);
    let mut notes = Vec::new();
    if !side_effect_match {
        notes.push("side_effect_mismatch".to_string());
    }

    let readiness_rank = health
        .map(|entry| readiness_sort_rank(&entry.readiness))
        .unwrap_or(2);
    if let Some(entry) = health {
        if !entry.reasons.is_empty() {
            notes.extend(entry.reasons.iter().take(3).cloned());
        }
    }

    ToolContractFit {
        action_name: action.name.clone(),
        side_effect_match,
        readiness_rank,
        side_effect_rank: if side_effect_match { 0 } else { 3 },
        input_rank: match contract_summary_for_action(action, None).input_completeness {
            ContractInputCompleteness::Complete => 0,
            ContractInputCompleteness::Unknown => 1,
            ContractInputCompleteness::Incomplete => 2,
        },
        auth_rank: if metadata.requires_auth
            || !action.authorization.access.integration_ids.is_empty()
            || !action.authorization.access.permission_ids.is_empty()
        {
            if health.is_some_and(|entry| {
                matches!(
                    &entry.readiness,
                    super::capability_health::CapabilityReadiness::Ready
                        | super::capability_health::CapabilityReadiness::Degraded
                )
            })
            {
                0
            } else {
                2
            }
        } else {
            0
        },
        cost_rank: cost_sort_rank(&metadata.cost),
        support_rank: if metadata.tool_role.is_support() { 2 } else { 0 },
        notes,
    }
}

pub(super) fn sort_candidates_by_contract_fit(
    goal: &super::semantic_turn::SemanticGoal,
    candidates: &mut [super::semantic_turn::CapabilityCandidate],
    action_by_name: &HashMap<String, crate::actions::ActionDef>,
    health_snapshot: Option<&super::capability_health::CapabilityHealthSnapshot>,
) {
    let semantic_rank = candidates
        .iter()
        .enumerate()
        .map(|(index, candidate)| (candidate.action_name.clone(), index))
        .collect::<HashMap<_, _>>();
    for candidate in candidates.iter_mut() {
        let Some(action) = action_by_name.get(&candidate.action_name) else {
            continue;
        };
        let health = health_snapshot.and_then(|snapshot| snapshot.entry(&candidate.action_name));
        let fit = contract_fit_for_candidate(goal, action, health);
        candidate.contract_fit = Some(fit.clone());
        candidate.readiness = health.map(|entry| entry.readiness.clone());
    }

    candidates.sort_by_key(|candidate| {
        let rank = semantic_rank
            .get(&candidate.action_name)
            .copied()
            .unwrap_or(usize::MAX);
        candidate.contract_fit.as_ref().map_or_else(
            || (9, 9, rank, 9, 9, 9, 9, candidate.action_name.clone()),
            |fit| {
                (
                    fit.side_effect_rank,
                    fit.support_rank,
                    rank,
                    fit.readiness_rank,
                    fit.auth_rank,
                    fit.input_rank,
                    fit.cost_rank,
                    candidate.action_name.clone(),
                )
            },
        )
    });
}

fn required_schema_fields(schema: &serde_json::Value) -> Vec<String> {
    let mut fields = Vec::new();
    collect_required_fields(schema, &mut fields, 0);
    fields.sort();
    fields.dedup();
    fields.truncate(16);
    fields
}

fn required_schema_options(schema: &serde_json::Value) -> Vec<Vec<String>> {
    for branch_key in ["oneOf", "anyOf"] {
        if let Some(items) = schema.get(branch_key).and_then(|item| item.as_array()) {
            let options = items
                .iter()
                .map(required_schema_fields)
                .filter(|fields| !fields.is_empty())
                .collect::<Vec<_>>();
            if !options.is_empty() {
                return options;
            }
        }
    }
    let fields = required_schema_fields(schema);
    if fields.is_empty() {
        vec![Vec::new()]
    } else {
        vec![fields]
    }
}

fn collect_required_fields(value: &serde_json::Value, fields: &mut Vec<String>, depth: usize) {
    if depth > 6 {
        return;
    }
    if let Some(required) = value.get("required").and_then(|item| item.as_array()) {
        fields.extend(
            required
                .iter()
                .filter_map(|item| item.as_str())
                .map(|item| item.trim().to_string())
                .filter(|item| !item.is_empty()),
        );
    }
    for branch_key in ["oneOf", "anyOf", "allOf"] {
        if let Some(items) = value.get(branch_key).and_then(|item| item.as_array()) {
            for item in items {
                collect_required_fields(item, fields, depth + 1);
            }
        }
    }
    if let Some(properties) = value.get("properties").and_then(|item| item.as_object()) {
        for child in properties.values() {
            collect_required_fields(child, fields, depth + 1);
        }
    }
}

fn value_is_present(value: &serde_json::Value) -> bool {
    match value {
        serde_json::Value::Null => false,
        serde_json::Value::String(text) => !text.trim().is_empty(),
        serde_json::Value::Array(items) => !items.is_empty(),
        serde_json::Value::Object(map) => !map.is_empty(),
        serde_json::Value::Bool(_) | serde_json::Value::Number(_) => true,
    }
}

fn idempotency_for_action(action: &crate::actions::ActionDef) -> ContractIdempotency {
    let metadata = action.action_metadata();
    if matches!(
        &metadata.side_effect_level,
        crate::actions::ActionSideEffectLevel::None
    ) {
        return ContractIdempotency::Idempotent;
    }
    let required = required_schema_fields(&action.input_schema);
    if required
        .iter()
        .any(|field| field.ends_with("_id") || field == "id")
    {
        return ContractIdempotency::CallerScoped;
    }
    if matches!(
        &metadata.side_effect_level,
        crate::actions::ActionSideEffectLevel::Write
    ) {
        ContractIdempotency::CreatesOrMutates
    } else {
        ContractIdempotency::Unknown
    }
}

fn output_shape_for_action(action: &crate::actions::ActionDef) -> String {
    let metadata = action.action_metadata();
    match metadata.delivery_mode {
        crate::actions::ActionDeliveryMode::Immediate => "immediate_tool_result".to_string(),
        crate::actions::ActionDeliveryMode::Async => "async_record_reference".to_string(),
        crate::actions::ActionDeliveryMode::Conditional => "conditional_monitor_reference".to_string(),
        crate::actions::ActionDeliveryMode::Either => "mode_dependent_result".to_string(),
    }
}

fn readiness_sort_rank(readiness: &super::capability_health::CapabilityReadiness) -> u8 {
    match readiness {
        super::capability_health::CapabilityReadiness::Ready => 0,
        super::capability_health::CapabilityReadiness::Degraded => 1,
        super::capability_health::CapabilityReadiness::Unknown => 2,
        super::capability_health::CapabilityReadiness::AuthRequired
        | super::capability_health::CapabilityReadiness::SetupRequired => 3,
        super::capability_health::CapabilityReadiness::Busy => 4,
        super::capability_health::CapabilityReadiness::RateLimited => 5,
    }
}

fn cost_sort_rank(cost: &crate::actions::ActionCostTier) -> u8 {
    match cost {
        crate::actions::ActionCostTier::Low => 0,
        crate::actions::ActionCostTier::Medium => 1,
        crate::actions::ActionCostTier::High => 2,
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn contract_extracts_required_fields_from_schema_branches() {
        let action = crate::actions::ActionDef {
            name: "schedule".to_string(),
            input_schema: serde_json::json!({
                "oneOf": [
                    {"required": ["task", "at"]},
                    {"required": ["task", "cron"]}
                ]
            }),
            ..crate::actions::ActionDef::default()
        };

        let contract = contract_summary_for_action(&action, None);

        assert!(contract.required_input.contains(&"task".to_string()));
        assert!(contract.required_input.contains(&"at".to_string()));
        assert!(contract.required_input.contains(&"cron".to_string()));
        assert_eq!(contract.input_completeness, ContractInputCompleteness::Unknown);
    }
}
