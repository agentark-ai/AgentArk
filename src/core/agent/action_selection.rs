use super::*;
use crate::actions::ActionDef;

pub(super) fn action_is_read_only(action: &ActionDef) -> bool {
    matches!(
        action.planner_metadata().side_effect_level,
        PlannerSideEffectLevel::None
    )
}

pub(super) fn action_is_read_only_knowledge_action(action: &ActionDef) -> bool {
    let metadata = action.planner_metadata();
    action_is_read_only(action)
        && matches!(
            metadata.role,
            PlannerActionRole::DataSource | PlannerActionRole::Inspection
        )
        && matches!(
            metadata.integration_class,
            PlannerIntegrationClass::Search
                | PlannerIntegrationClass::Network
                | PlannerIntegrationClass::Analytics
                | PlannerIntegrationClass::Internal
                | PlannerIntegrationClass::Workspace
                | PlannerIntegrationClass::Filesystem
        )
}

pub(super) fn format_recent_dialogue_for_fast_path(
    history: &[ConversationMessage],
) -> Option<String> {
    let lines = history
        .iter()
        .rev()
        .take(6)
        .collect::<Vec<_>>()
        .into_iter()
        .rev()
        .filter_map(|message| {
            let content = message.content.trim();
            if content.is_empty() {
                return None;
            }
            Some(format!("{}: {}", message.role, safe_truncate(content, 240)))
        })
        .collect::<Vec<_>>();

    if lines.is_empty() {
        None
    } else {
        Some(lines.join("\n"))
    }
}
