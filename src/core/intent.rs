use crate::actions::ActionDef;

pub const DEFAULT_ACTION_INTENT_THRESHOLD: f32 = 0.45;

#[derive(Debug, Clone, Default)]
pub struct RankedActionIntent {
    pub action_name: String,
    pub score: f32,
    pub second_score: f32,
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

pub fn action_intent_score(message: &str, action: &ActionDef) -> f32 {
    let _ = (message, action);
    0.0
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

#[cfg(test)]
mod tests {
    use super::*;

    fn action(name: &str, description: &str) -> ActionDef {
        ActionDef {
            name: name.to_string(),
            description: description.to_string(),
            version: "1.0.0".to_string(),
            input_schema: serde_json::json!({}),
            capabilities: vec![],
            sandbox_mode: None,
            source: crate::actions::ActionSource::System,
            file_path: None,
            authorization: Default::default(),
        }
    }

    #[test]
    fn preferred_direct_action_stays_none_for_action_discussion() {
        let actions = vec![
            action("app_deploy", "Deploy an app"),
            action("file_write", "Write files"),
        ];

        assert_eq!(
            preferred_direct_action_name(
                "Explain when app_deploy should win over file_write in routing.",
                &actions
            ),
            None
        );
    }
}
