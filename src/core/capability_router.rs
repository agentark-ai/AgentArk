use std::collections::HashSet;

use crate::actions::ActionDef;

#[derive(Debug, Clone)]
pub struct RankedCapabilityAction {
    pub action: ActionDef,
    pub score: f32,
    pub second_score: f32,
}

fn tokenize(value: &str) -> HashSet<String> {
    value
        .to_ascii_lowercase()
        .chars()
        .map(|ch| if ch.is_ascii_alphanumeric() { ch } else { ' ' })
        .collect::<String>()
        .split_whitespace()
        .filter(|token| token.len() >= 2)
        .map(ToString::to_string)
        .collect()
}

fn schema_tokens(value: &serde_json::Value, out: &mut HashSet<String>) {
    match value {
        serde_json::Value::String(text) => {
            out.extend(tokenize(text));
        }
        serde_json::Value::Array(items) => {
            for item in items {
                schema_tokens(item, out);
            }
        }
        serde_json::Value::Object(map) => {
            for (key, value) in map {
                out.extend(tokenize(key));
                schema_tokens(value, out);
            }
        }
        serde_json::Value::Null | serde_json::Value::Bool(_) | serde_json::Value::Number(_) => {}
    }
}

fn planner_metadata_tokens(action: &ActionDef) -> HashSet<String> {
    let metadata = action.planner_metadata();
    tokenize(&serde_json::to_string(&metadata).unwrap_or_else(|_| format!("{:?}", metadata)))
}

fn action_tokens(action: &ActionDef) -> HashSet<String> {
    let mut tokens = tokenize(&format!(
        "{} {} {}",
        action.name,
        action.description,
        action.capabilities.join(" ")
    ));
    schema_tokens(&action.input_schema, &mut tokens);
    tokens.extend(planner_metadata_tokens(action));
    tokens
}

pub fn score_action_intent(message: &str, action: &ActionDef) -> f32 {
    score_action_intent_with_reasons(message, action).0
}

pub fn score_action_intent_with_reasons(message: &str, action: &ActionDef) -> (f32, Vec<String>) {
    let request_tokens = tokenize(message);
    let action_tokens = action_tokens(action);
    let overlap_count = request_tokens.intersection(&action_tokens).count() as f32;
    let request_coverage = if request_tokens.is_empty() {
        0.0
    } else {
        overlap_count / request_tokens.len() as f32
    };
    let action_coverage = if action_tokens.is_empty() {
        0.0
    } else {
        overlap_count / action_tokens.len() as f32
    };

    let mut reasons = Vec::new();
    if overlap_count > 0.0 {
        reasons.push(format!("catalog metadata overlap {:.0}", overlap_count));
    }

    let score = (request_coverage + action_coverage) / 2.0;

    (score.clamp(0.0, 1.0), reasons)
}

pub fn ranked_action_candidates(
    message: &str,
    all_actions: &[ActionDef],
    boosted_action_names: &HashSet<String>,
) -> Vec<RankedCapabilityAction> {
    let mut ranked = all_actions
        .iter()
        .map(|action| {
            let score = score_action_intent(message, action);
            RankedCapabilityAction {
                action: action.clone(),
                score,
                second_score: 0.0,
            }
        })
        .collect::<Vec<_>>();

    ranked.sort_by(|left, right| {
        let left_boosted = boosted_action_names.contains(&left.action.name);
        let right_boosted = boosted_action_names.contains(&right.action.name);
        right_boosted
            .cmp(&left_boosted)
            .then_with(|| {
                right
                    .score
                    .partial_cmp(&left.score)
                    .unwrap_or(std::cmp::Ordering::Equal)
            })
            .then_with(|| left.action.name.cmp(&right.action.name))
    });

    let scores = ranked.iter().map(|item| item.score).collect::<Vec<_>>();
    for (index, item) in ranked.iter_mut().enumerate() {
        item.second_score = scores
            .iter()
            .enumerate()
            .find_map(|(candidate_index, score)| (candidate_index != index).then_some(*score))
            .unwrap_or(0.0);
    }
    ranked
}

#[cfg(test)]
mod tests {
    use super::*;

    fn action(name: &str, description: &str, capabilities: &[&str]) -> ActionDef {
        ActionDef {
            name: name.to_string(),
            description: description.to_string(),
            version: "1.0.0".to_string(),
            input_schema: serde_json::json!({
                "type": "object",
                "properties": {
                    "query": { "type": "string", "description": "Topic or query" }
                }
            }),
            capabilities: capabilities.iter().map(|value| value.to_string()).collect(),
            sandbox_mode: None,
            source: crate::actions::ActionSource::System,
            file_path: None,
            authorization: Default::default(),
        }
    }

    #[test]
    fn scorer_ranks_unknown_custom_action_from_live_metadata() {
        let actions = vec![
            action("generic_tool", "General utility", &[]),
            action(
                "custom_action_alpha",
                "Handles alpha beta gamma requests",
                &["alpha_capability"],
            ),
        ];

        let ranked =
            ranked_action_candidates("Please handle alpha beta gamma", &actions, &HashSet::new());

        assert_eq!(ranked[0].action.name, "custom_action_alpha");
    }
}
