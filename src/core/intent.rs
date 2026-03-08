use std::collections::HashSet;

use crate::actions::ActionDef;

pub const DEFAULT_ACTION_INTENT_THRESHOLD: f32 = 0.45;

#[derive(Debug, Clone, Copy, Default)]
pub struct ActionIntentSignal {
    pub target_score: f32,
    pub best_score: f32,
    pub best_non_target_score: f32,
    pub rank: usize,
}

#[derive(Debug, Clone, Default)]
pub struct RankedActionIntent {
    pub action_name: String,
    pub score: f32,
    pub second_score: f32,
}

impl ActionIntentSignal {
    pub fn margin_vs_best_non_target(self) -> f32 {
        self.target_score - self.best_non_target_score
    }
}

impl RankedActionIntent {
    pub fn margin_vs_next(&self) -> f32 {
        self.score - self.second_score
    }

    pub fn is_clear_top(&self) -> bool {
        self.score >= DEFAULT_ACTION_INTENT_THRESHOLD
            || (self.score >= 0.28 && self.margin_vs_next() >= 0.08)
    }
}

fn tokenize(text: &str) -> HashSet<String> {
    text.to_lowercase()
        .split(|c: char| !c.is_alphanumeric())
        .filter_map(|w| {
            let w = w.trim();
            if w.len() < 3 || w.chars().all(|c| c.is_ascii_digit()) {
                None
            } else {
                Some(w.to_string())
            }
        })
        .collect()
}

fn collect_json_tokens(
    value: &serde_json::Value,
    out: &mut HashSet<String>,
    remaining: &mut usize,
) {
    if *remaining == 0 {
        return;
    }
    match value {
        serde_json::Value::Object(map) => {
            for (key, inner) in map {
                if *remaining == 0 {
                    break;
                }
                for token in tokenize(key) {
                    if out.insert(token) {
                        *remaining = remaining.saturating_sub(1);
                        if *remaining == 0 {
                            return;
                        }
                    }
                }
                collect_json_tokens(inner, out, remaining);
            }
        }
        serde_json::Value::Array(items) => {
            for item in items {
                if *remaining == 0 {
                    break;
                }
                collect_json_tokens(item, out, remaining);
            }
        }
        serde_json::Value::String(text) => {
            for token in tokenize(text) {
                if out.insert(token) {
                    *remaining = remaining.saturating_sub(1);
                    if *remaining == 0 {
                        return;
                    }
                }
            }
        }
        _ => {}
    }
}

fn action_tokens(action: &ActionDef) -> HashSet<String> {
    let mut tokens = HashSet::new();
    let name_words = action.name.replace('_', " ");
    tokens.extend(tokenize(&name_words));
    tokens.extend(tokenize(&action.description));
    for cap in &action.capabilities {
        tokens.extend(tokenize(cap));
    }
    let mut remaining = 96usize;
    collect_json_tokens(&action.input_schema, &mut tokens, &mut remaining);
    tokens
}

pub fn action_intent_score(message: &str, action: &ActionDef) -> f32 {
    let msg_tokens = tokenize(message);
    if msg_tokens.is_empty() {
        return 0.0;
    }

    let action_tokens = action_tokens(action);
    if action_tokens.is_empty() {
        return 0.0;
    }

    let overlap = msg_tokens.intersection(&action_tokens).count() as f32;
    let coverage = overlap / msg_tokens.len() as f32;
    let precision = overlap / action_tokens.len() as f32;
    // Coverage dominates because action descriptions are often long.
    let mut score = (0.8 * coverage) + (0.2 * (precision * 6.0).min(1.0));

    let message_lc = message.to_lowercase();
    let exact_name = action.name.to_lowercase();
    if message_lc.contains(&exact_name) {
        score = score.max(0.95);
    } else {
        let spaced_name = exact_name.replace('_', " ");
        if message_lc.contains(&spaced_name) {
            score = score.max(0.90);
        }
    }

    score.clamp(0.0, 1.0)
}

pub fn action_intent_signal(
    message: &str,
    actions: &[ActionDef],
    action_name: &str,
) -> Option<ActionIntentSignal> {
    let mut target_score: Option<f32> = None;
    let mut best_score = 0.0_f32;
    let mut best_non_target_score = 0.0_f32;
    let mut scored: Vec<(bool, f32)> = Vec::with_capacity(actions.len());

    for action in actions {
        let score = action_intent_score(message, action);
        let is_target = action.name == action_name;
        if is_target {
            target_score = Some(score);
        } else {
            best_non_target_score = best_non_target_score.max(score);
        }
        best_score = best_score.max(score);
        scored.push((is_target, score));
    }

    let target_score = target_score?;
    let rank = 1 + scored.iter().filter(|(_, s)| *s > target_score).count();

    Some(ActionIntentSignal {
        target_score,
        best_score,
        best_non_target_score,
        rank,
    })
}

pub fn has_action_intent_adaptive(message: &str, actions: &[ActionDef], action_name: &str) -> bool {
    let Some(signal) = action_intent_signal(message, actions, action_name) else {
        return false;
    };

    if signal.target_score >= DEFAULT_ACTION_INTENT_THRESHOLD {
        return true;
    }

    let near_top = signal.target_score + 0.03 >= signal.best_score;
    let clear_lead = signal.margin_vs_best_non_target() >= 0.06;

    (signal.rank == 1 && signal.target_score >= 0.22 && clear_lead)
        || (signal.rank <= 2 && signal.target_score >= 0.32 && near_top)
}

pub fn top_ranked_action_intent(
    message: &str,
    actions: &[ActionDef],
) -> Option<RankedActionIntent> {
    let mut scored: Vec<(f32, &str)> = actions
        .iter()
        .map(|action| (action_intent_score(message, action), action.name.as_str()))
        .collect();
    scored.sort_by(|a, b| {
        b.0.partial_cmp(&a.0)
            .unwrap_or(std::cmp::Ordering::Equal)
            .then_with(|| a.1.cmp(b.1))
    });

    let (score, action_name) = scored.first().copied()?;
    let second_score = scored.get(1).map(|(s, _)| *s).unwrap_or(0.0);
    Some(RankedActionIntent {
        action_name: action_name.to_string(),
        score,
        second_score,
    })
}

pub fn preferred_direct_action_name(message: &str, actions: &[ActionDef]) -> Option<String> {
    let top = top_ranked_action_intent(message, actions)?;
    if top.is_clear_top() {
        Some(top.action_name)
    } else {
        None
    }
}
