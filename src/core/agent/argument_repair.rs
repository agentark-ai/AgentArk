//! Generic argument validation helpers for task automation.
//!
//! All logic in this module is deliberately action-agnostic. Action-specific
//! static repairs (for example the `code_execute` language inference from code
//! syntax) live with their action's helpers and are invoked by callers *before*
//! they delegate to the generic helpers here. That separation enforces the
//! project rule against brittle action-name conditionals or hardcoded phrasing
//! in validation paths.

use super::*;

/// Generic emptiness check for required-field presence. A field counts as
/// "present" only if it carries a non-empty value of any structural shape.
/// Mirrors the existing semantics of `Agent::required_action_argument_present`
/// in `task_runtime.rs:3096` so the two can share callers without drift.
pub(super) fn required_argument_present(value: Option<&serde_json::Value>) -> bool {
    match value {
        Some(serde_json::Value::Null) | None => false,
        Some(serde_json::Value::String(s)) => !s.trim().is_empty(),
        Some(serde_json::Value::Array(items)) => !items.is_empty(),
        Some(serde_json::Value::Object(map)) => !map.is_empty(),
        Some(_) => true,
    }
}

/// List of required schema fields that are absent from the payload.
pub(super) fn missing_required_fields(
    action: &crate::actions::ActionDef,
    payload: &serde_json::Map<String, serde_json::Value>,
) -> Vec<String> {
    let required = action
        .input_schema
        .get("required")
        .and_then(|value| value.as_array())
        .map(|items| {
            items
                .iter()
                .filter_map(|item| item.as_str())
                .map(ToString::to_string)
                .collect::<Vec<_>>()
        })
        .unwrap_or_default();

    required
        .into_iter()
        .filter(|field| !required_argument_present(payload.get(field.as_str())))
        .collect()
}
