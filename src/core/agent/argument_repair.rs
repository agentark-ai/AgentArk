//! Generic argument-repair helpers shared by the chat agent loop validator
//! (`tool_execution.rs::repair_and_validate_tool_call_arguments`) and the task
//! automation argument normalizer (`task_runtime.rs::normalize_action_arguments`).
//!
//! All logic in this module is deliberately action-agnostic. Action-specific
//! static repairs (for example the `code_execute` language inference from code
//! syntax) live with their action's helpers and are invoked by callers *before*
//! they delegate to the generic helpers here. That separation enforces the
//! project rule against brittle action-name conditionals or hardcoded phrasing
//! in routing/validation paths.

use super::*;

/// Intent context passed into LLM-based argument repair so the model can fill
/// missing required fields semantically — using the user's underlying meaning
/// plus the routing classifier's signals plus the active turn-plan goals —
/// rather than guessing from the surface phrasing of the user message alone.
#[derive(Debug, Clone, Default)]
pub(crate) struct ArgumentRepairContext {
    pub user_message: String,
    pub routing_summary: Option<String>,
    pub goal_summaries: Vec<String>,
}

impl ArgumentRepairContext {
    pub(crate) fn from_message(user_message: impl Into<String>) -> Self {
        Self {
            user_message: user_message.into(),
            ..Self::default()
        }
    }

    /// Compose the full repair-time prompt context as a single string. Empty
    /// sections are omitted so the resulting text is only as long as the
    /// signals warrant.
    pub(crate) fn build_request_text(&self) -> String {
        let mut sections: Vec<String> = Vec::with_capacity(3);

        let user = self.user_message.trim();
        if !user.is_empty() {
            sections.push(format!("USER REQUEST:\n{}", user));
        }

        if let Some(summary) = self.routing_summary.as_deref() {
            let trimmed = summary.trim();
            if !trimmed.is_empty() {
                sections.push(format!("ROUTING INTENT:\n{}", trimmed));
            }
        }

        let goals: Vec<&str> = self
            .goal_summaries
            .iter()
            .map(|s| s.trim())
            .filter(|s| !s.is_empty())
            .collect();
        if !goals.is_empty() {
            let bullets = goals
                .into_iter()
                .map(|g| format!("- {}", g))
                .collect::<Vec<_>>()
                .join("\n");
            sections.push(format!("TURN GOALS:\n{}", bullets));
        }

        sections.join("\n\n")
    }
}

/// Result of an LLM-driven repair attempt against a tool call payload.
#[derive(Debug, Clone, Default)]
pub(crate) struct ArgumentRepairOutcome {
    pub repaired_payload: serde_json::Map<String, serde_json::Value>,
    pub still_missing: Vec<String>,
    pub partial_inference: serde_json::Map<String, serde_json::Value>,
}

/// Per-turn memo so identical repair attempts (same action, same missing
/// fields, same payload) do not invoke the LLM more than once within a single
/// user turn.
#[derive(Debug, Default)]
pub(crate) struct RepairMemo {
    seen: std::collections::HashMap<String, ArgumentRepairOutcome>,
    attempted_scopes: std::collections::HashSet<String>,
}

impl RepairMemo {
    pub(crate) fn lookup(&self, key: &str) -> Option<&ArgumentRepairOutcome> {
        self.seen.get(key)
    }

    pub(crate) fn record(&mut self, key: String, outcome: ArgumentRepairOutcome) {
        self.seen.insert(key, outcome);
    }

    pub(crate) fn claim_attempt_scope(&mut self, key: String) -> bool {
        self.attempted_scopes.insert(key)
    }
}

#[derive(Debug, Clone, Default)]
pub(crate) struct ArgumentRepairClarification {
    pub action_name: String,
    pub missing_fields: Vec<String>,
    pub partial_inference: serde_json::Map<String, serde_json::Value>,
}

impl ArgumentRepairClarification {
    pub(crate) fn payload(&self) -> serde_json::Value {
        serde_json::json!({
            "needs_clarification": true,
            "action": self.action_name,
            "missing_fields": self.missing_fields,
            "partial_inference": self.partial_inference,
        })
    }
}

pub(super) fn missing_fields_signature(action_name: &str, missing_fields: &[String]) -> String {
    let mut sorted = missing_fields.to_vec();
    sorted.sort();
    format!("{}::{}", action_name, sorted.join(","))
}

/// Stable de-duplication key for the per-turn repair memo. Identical
/// `(action, missing-field-set, payload)` combinations hash to the same key
/// regardless of field ordering inside the missing list.
pub(super) fn repair_memo_key(
    action_name: &str,
    missing_fields: &[String],
    payload: &serde_json::Map<String, serde_json::Value>,
) -> String {
    let mut sorted = missing_fields.to_vec();
    sorted.sort();
    let payload_text = serde_json::Value::Object(payload.clone()).to_string();
    let mut hasher = Sha256::new();
    hasher.update(payload_text.as_bytes());
    let payload_hash = hasher
        .finalize()
        .iter()
        .map(|byte| format!("{:02x}", byte))
        .collect::<String>();
    format!("{}::{}::{}", action_name, sorted.join(","), payload_hash)
}

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

/// Shallow JSON-schema validation: catches enum violations on top-level fields
/// without doing full schema validation. Mirrors the existing helper in
/// `tool_execution.rs::shallow_action_argument_schema_error` so both validators
/// can share one source of truth.
pub(super) fn shallow_schema_violation(
    action: &crate::actions::ActionDef,
    arguments: &serde_json::Value,
) -> Option<String> {
    let properties = action
        .input_schema
        .get("properties")
        .and_then(|value| value.as_object())?;
    let argument_map = arguments.as_object()?;

    for (field, value) in argument_map {
        let Some(field_schema) = properties.get(field) else {
            continue;
        };
        let Some(allowed_values) = field_schema.get("enum").and_then(|item| item.as_array()) else {
            continue;
        };
        let Some(actual) = value.as_str() else {
            return Some(format!(
                "field `{}` must be one of the schema enum values.",
                field
            ));
        };
        let matches_enum = allowed_values
            .iter()
            .filter_map(|item| item.as_str())
            .any(|allowed| allowed == actual);
        if !matches_enum {
            let allowed = allowed_values
                .iter()
                .filter_map(|item| item.as_str())
                .take(20)
                .collect::<Vec<_>>()
                .join(", ");
            return Some(format!(
                "field `{}` has unsupported value `{}`; allowed values: {}.",
                field, actual, allowed
            ));
        }
    }

    None
}

impl Agent {
    /// Invoke the existing LLM-based argument inferer to fill missing required
    /// fields semantically. Memoized per `(action, missing-field-set, payload)`
    /// for the duration of one user turn so identical repeat attempts do not
    /// re-call the model.
    ///
    /// Returns the outcome with `still_missing` empty when fully repaired.
    /// On failure the original payload is preserved unchanged.
    pub(super) async fn fill_missing_required_fields_via_inference(
        &self,
        action: &crate::actions::ActionDef,
        payload: &mut serde_json::Map<String, serde_json::Value>,
        ctx: &ArgumentRepairContext,
        missing_fields: &[String],
        memo: &mut RepairMemo,
    ) -> ArgumentRepairOutcome {
        if missing_fields.is_empty() {
            return ArgumentRepairOutcome {
                repaired_payload: payload.clone(),
                still_missing: Vec::new(),
                partial_inference: serde_json::Map::new(),
            };
        }

        let key = repair_memo_key(&action.name, missing_fields, payload);
        if let Some(cached) = memo.lookup(&key) {
            for (k, v) in &cached.partial_inference {
                payload.entry(k.clone()).or_insert_with(|| v.clone());
            }
            return cached.clone();
        }

        let request_text = ctx.build_request_text();
        let inferred = self
            .infer_missing_action_arguments(action, payload, &request_text, missing_fields)
            .await
            .unwrap_or_default();

        for (k, v) in &inferred {
            payload.entry(k.clone()).or_insert_with(|| v.clone());
        }

        let still_missing = missing_required_fields(action, payload);

        let outcome = ArgumentRepairOutcome {
            repaired_payload: payload.clone(),
            still_missing,
            partial_inference: inferred,
        };

        memo.record(key, outcome.clone());
        outcome
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn action_with_required(name: &str, required: &[&str]) -> crate::actions::ActionDef {
        let required_json: Vec<serde_json::Value> = required
            .iter()
            .map(|s| serde_json::Value::String((*s).to_string()))
            .collect();
        let mut properties = serde_json::Map::new();
        for field in required {
            properties.insert(
                (*field).to_string(),
                serde_json::json!({"type": "string"}),
            );
        }
        crate::actions::ActionDef {
            name: name.to_string(),
            input_schema: serde_json::json!({
                "type": "object",
                "properties": serde_json::Value::Object(properties),
                "required": required_json,
            }),
            ..crate::actions::ActionDef::default()
        }
    }

    #[test]
    fn missing_required_fields_returns_empty_when_all_present() {
        let action = action_with_required("x", &["path", "content"]);
        let mut payload = serde_json::Map::new();
        payload.insert("path".into(), serde_json::Value::String("a.html".into()));
        payload.insert("content".into(), serde_json::Value::String("<x/>".into()));

        let missing = missing_required_fields(&action, &payload);
        assert!(missing.is_empty());
    }

    #[test]
    fn missing_required_fields_lists_each_absent_field() {
        let action = action_with_required("x", &["path", "content"]);
        let mut payload = serde_json::Map::new();
        payload.insert("content".into(), serde_json::Value::String("<x/>".into()));

        let missing = missing_required_fields(&action, &payload);
        assert_eq!(missing, vec!["path".to_string()]);
    }

    #[test]
    fn required_argument_present_treats_blank_strings_as_absent() {
        assert!(!required_argument_present(Some(&serde_json::Value::String(
            "   ".into()
        ))));
        assert!(required_argument_present(Some(&serde_json::Value::String(
            "a".into()
        ))));
        assert!(!required_argument_present(None));
        assert!(!required_argument_present(Some(&serde_json::Value::Null)));
        assert!(!required_argument_present(Some(&serde_json::Value::Array(
            vec![]
        ))));
        assert!(required_argument_present(Some(&serde_json::Value::Bool(
            false
        ))));
    }

    #[test]
    fn repair_memo_key_is_stable_under_field_reordering() {
        let payload: serde_json::Map<String, serde_json::Value> = serde_json::Map::new();
        let key_a = repair_memo_key(
            "file_write",
            &["path".into(), "content".into()],
            &payload,
        );
        let key_b = repair_memo_key(
            "file_write",
            &["content".into(), "path".into()],
            &payload,
        );
        assert_eq!(key_a, key_b);
    }

    #[test]
    fn repair_memo_key_changes_when_payload_changes() {
        let mut a = serde_json::Map::new();
        a.insert("k".into(), serde_json::Value::String("v1".into()));
        let mut b = serde_json::Map::new();
        b.insert("k".into(), serde_json::Value::String("v2".into()));
        let key_a = repair_memo_key("x", &["m".into()], &a);
        let key_b = repair_memo_key("x", &["m".into()], &b);
        assert_ne!(key_a, key_b);
    }

    #[test]
    fn repair_context_request_text_includes_all_sections_present() {
        let ctx = ArgumentRepairContext {
            user_message: "build a landing page".into(),
            routing_summary: Some("durable_work=true; capability=app_hosting".into()),
            goal_summaries: vec![
                "deliver a runnable site".into(),
                "include hero + dashboards".into(),
            ],
        };
        let text = ctx.build_request_text();
        assert!(text.contains("USER REQUEST:"));
        assert!(text.contains("build a landing page"));
        assert!(text.contains("ROUTING INTENT:"));
        assert!(text.contains("TURN GOALS:"));
        assert!(text.contains("- deliver a runnable site"));
    }

    #[test]
    fn repair_context_omits_empty_sections() {
        let ctx = ArgumentRepairContext::from_message("hi");
        let text = ctx.build_request_text();
        assert!(text.contains("USER REQUEST:"));
        assert!(!text.contains("ROUTING INTENT:"));
        assert!(!text.contains("TURN GOALS:"));
    }

    #[test]
    fn shallow_schema_violation_passes_for_legal_enum_value() {
        let action = crate::actions::ActionDef {
            name: "x".into(),
            input_schema: serde_json::json!({
                "type": "object",
                "properties": {
                    "language": {"type": "string", "enum": ["python", "javascript"]}
                },
                "required": ["language"],
            }),
            ..crate::actions::ActionDef::default()
        };
        let args = serde_json::json!({"language": "python"});
        assert!(shallow_schema_violation(&action, &args).is_none());
    }

    #[test]
    fn shallow_schema_violation_flags_unknown_enum_value() {
        let action = crate::actions::ActionDef {
            name: "x".into(),
            input_schema: serde_json::json!({
                "type": "object",
                "properties": {
                    "language": {"type": "string", "enum": ["python", "javascript"]}
                },
                "required": ["language"],
            }),
            ..crate::actions::ActionDef::default()
        };
        let args = serde_json::json!({"language": "klingon"});
        let issue = shallow_schema_violation(&action, &args).expect("expected violation");
        assert!(issue.contains("unsupported"));
    }

    /// Forbid action-name string-literal conditionals inside this module. The
    /// new module is action-agnostic by contract; per-action repair must live
    /// with its action's helpers, not here. If anyone adds `if name == "..."`
    /// or `match name { "..." => }` to this file, the build fails.
    #[test]
    fn argument_repair_module_has_no_action_name_conditionals() {
        let source = include_str!("argument_repair.rs");
        // Strip the `#[cfg(test)]` block so this test does not flag itself.
        let scan_end = source.find("#[cfg(test)]").unwrap_or(source.len());
        let body = &source[..scan_end];

        let forbidden_patterns = [
            "if call.name ==",
            "if action.name ==",
            "if action_name ==",
            "match call.name",
            "match action_name",
            "match action.name",
        ];

        for pattern in forbidden_patterns {
            assert!(
                !body.contains(pattern),
                "argument_repair.rs is action-agnostic by contract; \
                 forbidden pattern detected: `{}`",
                pattern
            );
        }
    }
}
