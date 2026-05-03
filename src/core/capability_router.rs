use std::collections::HashSet;

use crate::actions::ActionDef;

#[cfg(test)]
#[derive(Debug, Clone)]
pub struct RankedCapabilityAction {
    pub action: ActionDef,
    pub score: f32,
    pub second_score: f32,
}

fn normalized_text(value: &str) -> String {
    value
        .to_ascii_lowercase()
        .chars()
        .map(|ch| if ch.is_ascii_alphanumeric() { ch } else { ' ' })
        .collect()
}

fn tokenize(value: &str) -> HashSet<String> {
    normalized_text(value)
        .split_whitespace()
        .filter(|token| token.len() >= 2)
        .map(ToString::to_string)
        .collect()
}

fn char_ngrams(value: &str, width: usize) -> HashSet<String> {
    let compact = normalized_text(value)
        .split_whitespace()
        .collect::<Vec<_>>()
        .join(" ");
    if compact.is_empty() {
        return HashSet::new();
    }
    let chars = compact.chars().collect::<Vec<_>>();
    if chars.len() <= width {
        return [compact].into_iter().collect();
    }
    (0..=chars.len().saturating_sub(width))
        .map(|index| chars[index..index + width].iter().collect::<String>())
        .collect()
}

fn jaccard_similarity(left: &HashSet<String>, right: &HashSet<String>) -> f32 {
    if left.is_empty() || right.is_empty() {
        return 0.0;
    }
    let overlap = left.intersection(right).count() as f32;
    let union = left.union(right).count() as f32;
    if union <= 0.0 { 0.0 } else { overlap / union }
}

fn token_similarity(left: &str, right: &str) -> f32 {
    if left == right {
        return 1.0;
    }
    let min_len = left.len().min(right.len()) as f32;
    let max_len = left.len().max(right.len()) as f32;
    if max_len <= 0.0 {
        return 0.0;
    }
    if left.starts_with(right) || right.starts_with(left) {
        return (0.75 + (min_len / max_len) * 0.25).clamp(0.0, 1.0);
    }
    let left_ngrams = char_ngrams(left, 3);
    let right_ngrams = char_ngrams(right, 3);
    jaccard_similarity(&left_ngrams, &right_ngrams)
}

fn soft_token_overlap(left: &HashSet<String>, right: &HashSet<String>) -> f32 {
    if left.is_empty() || right.is_empty() {
        return 0.0;
    }
    left.iter()
        .map(|left_token| {
            right
                .iter()
                .map(|right_token| token_similarity(left_token, right_token))
                .fold(0.0f32, f32::max)
        })
        .sum::<f32>()
        / left.len() as f32
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

fn action_descriptor_text(action: &ActionDef) -> String {
    format!(
        "{} {} {} {} {}",
        action.name,
        action.description,
        action.capabilities.join(" "),
        serde_json::to_string(&action.input_schema).unwrap_or_default(),
        serde_json::to_string(&action.planner_metadata()).unwrap_or_default(),
    )
}

fn action_tokens(action: &ActionDef) -> HashSet<String> {
    let mut tokens = tokenize(&action_descriptor_text(action));
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
    let fuzzy_request_coverage = soft_token_overlap(&request_tokens, &action_tokens);
    let fuzzy_action_coverage = soft_token_overlap(&action_tokens, &request_tokens);
    let request_ngrams = char_ngrams(message, 3);
    let action_ngrams = char_ngrams(&action_descriptor_text(action), 3);
    let trigram_similarity = jaccard_similarity(&request_ngrams, &action_ngrams);

    let mut reasons = Vec::new();
    if overlap_count > 0.0 {
        reasons.push(format!("catalog metadata overlap {:.0}", overlap_count));
    }
    if fuzzy_request_coverage >= 0.18 || fuzzy_action_coverage >= 0.18 {
        reasons.push(format!(
            "fuzzy intent overlap {:.2}",
            ((fuzzy_request_coverage + fuzzy_action_coverage) / 2.0)
        ));
    }
    if trigram_similarity >= 0.12 {
        reasons.push(format!("phrase similarity {:.2}", trigram_similarity));
    }

    let exact_score = (request_coverage + action_coverage) / 2.0;
    let fuzzy_score = (fuzzy_request_coverage + fuzzy_action_coverage) / 2.0;
    let score = exact_score * 0.4 + fuzzy_score * 0.4 + trigram_similarity * 0.2;

    (score.clamp(0.0, 1.0), reasons)
}

#[cfg(test)]
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

    #[test]
    fn scorer_handles_typos_and_paraphrase_without_exact_token_overlap() {
        let actions = vec![
            action(
                "watch",
                "Monitor a source repeatedly and alert on changes",
                &["watcher"],
            ),
            action(
                "app_deploy",
                "Build, deploy, and expose an application",
                &["app_hosting"],
            ),
        ];

        let ranked = ranked_action_candidates(
            "montior this every 10 sec and tell me when something changes",
            &actions,
            &HashSet::new(),
        );

        assert_eq!(ranked[0].action.name, "watch");
    }
}
