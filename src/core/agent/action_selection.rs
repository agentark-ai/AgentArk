use super::*;
use crate::actions::ActionDef;

#[allow(dead_code)]
pub(super) fn action_is_read_only(action: &ActionDef) -> bool {
    matches!(
        action.action_metadata().side_effect_level,
        ActionSideEffectLevel::None
    )
}

#[allow(dead_code)]
pub(super) fn action_is_read_only_knowledge_action(action: &ActionDef) -> bool {
    let metadata = action.action_metadata();
    action_is_read_only(action)
        && matches!(
            metadata.role,
            ActionRole::DataSource | ActionRole::Inspection
        )
        && matches!(
            metadata.integration_class,
            ActionIntegrationClass::Search
                | ActionIntegrationClass::Network
                | ActionIntegrationClass::Analytics
                | ActionIntegrationClass::Internal
                | ActionIntegrationClass::Workspace
                | ActionIntegrationClass::Filesystem
        )
}

pub(super) fn format_recent_dialogue_for_memory_context(
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
