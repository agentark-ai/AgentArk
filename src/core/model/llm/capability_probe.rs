//! Model capability probes: verify at CONFIGURATION time that a model can do
//! what AgentArk's agent runs require (emit well-formed tool calls, return
//! non-empty finals, follow JSON output instructions), instead of letting an
//! unsupported model be discovered through opaque runtime failure loops.
//!
//! Detection is strictly behavioral — the probes call the model through the
//! SAME client the spine uses and judge only the response shape. No
//! model-name or provider lists, ever.

use super::LlmClient;

const PROBE_TOOL_NAME: &str = "probe_echo";

#[derive(Debug, Clone, serde::Serialize)]
pub struct CapabilityCheck {
    pub pass: bool,
    pub detail: String,
    pub latency_ms: u64,
}

#[derive(Debug, Clone, serde::Serialize)]
pub struct RoleFitness {
    /// Agent runs (the spine): requires tool calls + non-empty finals.
    pub primary_spine: bool,
    /// Verifier/decider roles: requires JSON output discipline.
    pub verifier: bool,
    /// Text-only helper roles (distill, summaries): requires non-empty finals.
    pub helper_text: bool,
}

#[derive(Debug, Clone, serde::Serialize)]
pub struct ModelCapabilityReport {
    pub tool_calls: CapabilityCheck,
    pub final_response: CapabilityCheck,
    pub json_output: CapabilityCheck,
    /// "supported" | "degraded" | "unsupported_for_agent_runs"
    pub verdict: &'static str,
    pub role_fitness: RoleFitness,
}

fn probe_echo_action() -> crate::actions::ActionDef {
    crate::actions::ActionDef {
        name: PROBE_TOOL_NAME.to_string(),
        description:
            "Echo the provided values back to the caller. Calling this tool is REQUIRED to complete the request."
                .to_string(),
        version: "1.0.0".to_string(),
        input_schema: serde_json::json!({
            "type": "object",
            "properties": {
                "text": {
                    "type": "string",
                    "description": "Any short text, for example the word ready."
                },
                "count": {
                    "type": "integer",
                    "description": "Any small whole number, for example 3."
                }
            },
            "required": ["text", "count"]
        }),
        capabilities: Vec::new(),
        sandbox_mode: None,
        source: crate::actions::ActionSource::System,
        file_path: None,
        authorization: Default::default(),
    }
}

/// Judge a tool-call probe response by shape: a tool call was emitted, it
/// targets the probe tool, its arguments parse as a JSON object, and every
/// required field is present with the right type. This is exactly the
/// contract the spine depends on per turn.
pub fn evaluate_tool_call_probe(response: &super::LlmResponse) -> (bool, String) {
    let Some(call) = response.tool_calls.first() else {
        return (
            false,
            format!(
                "no tool call emitted (content_chars={}, tool_calls=0)",
                response.content.chars().count()
            ),
        );
    };
    if call.name != PROBE_TOOL_NAME {
        return (
            false,
            format!("tool call targeted '{}' instead of '{PROBE_TOOL_NAME}'", call.name),
        );
    }
    let arguments = match &call.arguments {
        serde_json::Value::Object(map) => map.clone(),
        serde_json::Value::String(raw) => match serde_json::from_str::<serde_json::Value>(raw) {
            Ok(serde_json::Value::Object(map)) => map,
            _ => {
                return (
                    false,
                    "tool-call arguments were not a parseable JSON object".to_string(),
                )
            }
        },
        _ => {
            return (
                false,
                "tool-call arguments were not a JSON object".to_string(),
            )
        }
    };
    let text_ok = arguments
        .get("text")
        .and_then(|value| value.as_str())
        .is_some_and(|value| !value.trim().is_empty());
    let count_ok = arguments.get("count").is_some_and(|value| value.is_i64() || value.is_u64());
    if !text_ok || !count_ok {
        let mut missing = Vec::new();
        if !text_ok {
            missing.push("text (string)");
        }
        if !count_ok {
            missing.push("count (integer)");
        }
        return (
            false,
            format!(
                "tool call emitted but required arguments were missing or mistyped: {}",
                missing.join(", ")
            ),
        );
    }
    (true, "well-formed tool call with complete required arguments".to_string())
}

/// Judge a JSON-discipline probe response: content (after stripping markdown
/// fences) parses as a JSON object.
pub fn evaluate_json_probe(content: &str) -> (bool, String) {
    let trimmed = content.trim();
    let stripped = trimmed
        .strip_prefix("```json")
        .or_else(|| trimmed.strip_prefix("```"))
        .map(|rest| rest.trim_end_matches("```"))
        .unwrap_or(trimmed)
        .trim();
    match serde_json::from_str::<serde_json::Value>(stripped) {
        Ok(serde_json::Value::Object(_)) => (true, "parseable JSON object".to_string()),
        Ok(_) => (false, "parseable JSON but not an object".to_string()),
        Err(_) => (
            false,
            format!(
                "content was not parseable JSON ({} chars)",
                trimmed.chars().count()
            ),
        ),
    }
}

fn check(pass: bool, detail: String, started: std::time::Instant) -> CapabilityCheck {
    CapabilityCheck {
        pass,
        detail,
        latency_ms: started.elapsed().as_millis() as u64,
    }
}

/// Run all capability probes through the SAME client the spine uses. Each
/// probe is one small request; a transport error fails the probe with the
/// error text as detail (a model that cannot answer the probe cannot answer
/// the spine either).
pub async fn probe_model_capabilities(
    client: &LlmClient,
    policy: &crate::security::ModelPrivacyConfig,
) -> ModelCapabilityReport {
    let probe_tool = vec![probe_echo_action()];

    let started = std::time::Instant::now();
    let tool_calls = match client
        .chat_with_history_for_helper(
            "You are a connectivity probe. You MUST call the probe_echo tool with text=\"ready\" and count=3. Do not answer in plain text.",
            "Call the probe_echo tool now.",
            &[],
            &[],
            &probe_tool,
            policy,
            false,
        )
        .await
    {
        Ok(response) => {
            let (pass, detail) = evaluate_tool_call_probe(&response);
            check(pass, detail, started)
        }
        Err(error) => check(false, format!("probe request failed: {error}"), started),
    };

    let started = std::time::Instant::now();
    let final_response = match client
        .chat_with_history_for_helper(
            "You are a connectivity probe.",
            "Reply with the single word READY.",
            &[],
            &[],
            &[],
            policy,
            false,
        )
        .await
    {
        Ok(response) => {
            let empty = response.content.trim().is_empty();
            check(
                !empty,
                if empty {
                    "model returned an empty final response".to_string()
                } else {
                    "non-empty final response".to_string()
                },
                started,
            )
        }
        Err(error) => check(false, format!("probe request failed: {error}"), started),
    };

    let started = std::time::Instant::now();
    let json_output = match client
        .chat_with_history_for_helper(
            "You are a connectivity probe. Reply with exactly the requested JSON and nothing else.",
            "Reply with exactly this JSON object and nothing else: {\"ok\": true}",
            &[],
            &[],
            &[],
            policy,
            false,
        )
        .await
    {
        Ok(response) => {
            let (pass, detail) = evaluate_json_probe(&response.content);
            check(pass, detail, started)
        }
        Err(error) => check(false, format!("probe request failed: {error}"), started),
    };

    let role_fitness = RoleFitness {
        primary_spine: tool_calls.pass && final_response.pass,
        verifier: json_output.pass,
        helper_text: final_response.pass,
    };
    let verdict = if role_fitness.primary_spine {
        if json_output.pass {
            "supported"
        } else {
            "degraded"
        }
    } else {
        "unsupported_for_agent_runs"
    };

    ModelCapabilityReport {
        tool_calls,
        final_response,
        json_output,
        verdict,
        role_fitness,
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn response_with(
        content: &str,
        tool_calls: Vec<super::super::ToolCall>,
    ) -> super::super::LlmResponse {
        super::super::LlmResponse {
            content: content.to_string(),
            tool_calls,
            reasoning: None,
            usage: None,
            provider: "test".to_string(),
            model: "test-model".to_string(),
        }
    }

    #[test]
    fn tool_probe_passes_only_on_complete_well_typed_arguments() {
        let good = response_with(
            "",
            vec![super::super::ToolCall {
                id: "call_1".to_string(),
                name: PROBE_TOOL_NAME.to_string(),
                arguments: serde_json::json!({ "text": "ready", "count": 3 }),
                activity_label: None,
            }],
        );
        assert!(evaluate_tool_call_probe(&good).0);

        // The minimax failure shape: a tool call with dropped/missing fields.
        let dropped_field = response_with(
            "",
            vec![super::super::ToolCall {
                id: "call_1".to_string(),
                name: PROBE_TOOL_NAME.to_string(),
                arguments: serde_json::json!({ "text": "ready" }),
                activity_label: None,
            }],
        );
        let (pass, detail) = evaluate_tool_call_probe(&dropped_field);
        assert!(!pass);
        assert!(detail.contains("count"));

        // The other minimax failure shape: no tool call at all.
        let none = response_with("READY", Vec::new());
        assert!(!evaluate_tool_call_probe(&none).0);

        // JSON-encoded string arguments are accepted (provider variance).
        let string_args = response_with(
            "",
            vec![super::super::ToolCall {
                id: "call_1".to_string(),
                name: PROBE_TOOL_NAME.to_string(),
                arguments: serde_json::Value::String(
                    "{\"text\":\"ready\",\"count\":3}".to_string(),
                ),
                activity_label: None,
            }],
        );
        assert!(evaluate_tool_call_probe(&string_args).0);
    }

    #[test]
    fn json_probe_accepts_fenced_and_bare_objects_only() {
        assert!(evaluate_json_probe("{\"ok\": true}").0);
        assert!(evaluate_json_probe("```json\n{\"ok\": true}\n```").0);
        assert!(!evaluate_json_probe("READY").0);
        assert!(!evaluate_json_probe("[1, 2]").0);
        assert!(!evaluate_json_probe("").0);
    }
}
