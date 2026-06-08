use serde::{Deserialize, Serialize};
use serde_json::Value;
use std::collections::BTreeMap;

pub const SOURCE_ALIAS_KEYS: &[&str] = &[
    "source",
    "source_url",
    "spec_url",
    "api_url",
    "docs_url",
    "docs_text",
    "openapi_url",
    "openapi_text",
    "curl_text",
    "manifest",
];

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct RequestContractViolation {
    pub code: &'static str,
    pub message: String,
}

#[derive(Debug, Clone, PartialEq)]
pub struct ShapeFix {
    pub method: String,
    pub body: Option<Value>,
    pub content_type: Option<&'static str>,
}

impl ShapeFix {
    pub fn requires_content_type(&self) -> bool {
        self.content_type.is_some()
    }
}

pub fn source_alias_value(value: &Value) -> Option<(&'static str, String)> {
    SOURCE_ALIAS_KEYS.iter().find_map(|key| {
        let raw = value.get(*key)?;
        if let Some(text) = raw
            .as_str()
            .map(str::trim)
            .filter(|value| !value.is_empty())
        {
            return Some((*key, text.to_string()));
        }
        if matches!(*key, "source" | "openapi_text" | "manifest") && !raw.is_null() {
            return serde_json::to_string(raw)
                .ok()
                .filter(|value| !value.trim().is_empty())
                .map(|value| (*key, value));
        }
        None
    })
}

pub fn has_source_alias(value: &Value) -> bool {
    source_alias_value(value).is_some()
}

pub fn request_headers_have_content_type(headers: &BTreeMap<String, String>) -> bool {
    headers
        .keys()
        .any(|key| key.trim().eq_ignore_ascii_case("content-type"))
}

pub fn endpoint_has_graphql_signal(path: &str, headers: &BTreeMap<String, String>) -> bool {
    let path_has_graphql_segment = path
        .split(|ch: char| !ch.is_ascii_alphanumeric())
        .any(|segment| segment.eq_ignore_ascii_case("graphql"));
    let content_type_declares_graphql = headers.iter().any(|(key, value)| {
        key.eq_ignore_ascii_case("content-type") && value.to_ascii_lowercase().contains("graphql")
    });
    path_has_graphql_segment || content_type_declares_graphql
}

pub fn body_has_graphql_signal(body: &Value) -> bool {
    let query = body
        .as_str()
        .or_else(|| body.get("query").and_then(Value::as_str))
        .map(str::trim)
        .filter(|query| !query.is_empty());
    let Some(query) = query else {
        return false;
    };
    graphql_document_has_operation_signal(query)
}

fn graphql_document_has_operation_signal(document: &str) -> bool {
    let mut chars = document.char_indices().peekable();
    while let Some((start, ch)) = chars.next() {
        if ch.is_ascii_alphabetic() || ch == '_' {
            let mut end = start + ch.len_utf8();
            while let Some((index, next)) = chars.peek().copied() {
                if next.is_ascii_alphanumeric() || next == '_' {
                    end = index + next.len_utf8();
                    chars.next();
                } else {
                    break;
                }
            }
            if matches!(&document[start..end], "query" | "mutation" | "subscription") {
                return true;
            }
        }
    }
    false
}

pub fn coerce_json_encoded_body_value(body: Value) -> Value {
    match body {
        Value::String(raw) => {
            let trimmed = raw.trim();
            if trimmed.starts_with('{') || trimmed.starts_with('[') {
                serde_json::from_str::<Value>(trimmed).unwrap_or(Value::String(raw))
            } else {
                Value::String(raw)
            }
        }
        other => other,
    }
}

pub fn normalize_request_shape(
    method: &str,
    has_content_type: bool,
    body: Option<&Value>,
    graphql_signal: bool,
) -> ShapeFix {
    let method = method.trim();
    let mut method = if method.is_empty() {
        "GET".to_string()
    } else {
        method.to_ascii_uppercase()
    };
    let mut body = body.cloned().map(coerce_json_encoded_body_value);
    if graphql_signal {
        if matches!(method.as_str(), "GET" | "HEAD") {
            method = "POST".to_string();
        }
        if let Some(Value::String(query)) = body.as_ref().map(Value::clone) {
            let query = query.trim();
            if !query.is_empty() && graphql_document_has_operation_signal(query) {
                body = Some(serde_json::json!({ "query": query }));
            }
        }
    }

    let content_type = if body.is_some() && !has_content_type {
        Some("application/json")
    } else {
        None
    };

    ShapeFix {
        method,
        body,
        content_type,
    }
}

pub fn missing_required_body_violation() -> RequestContractViolation {
    RequestContractViolation {
        code: "missing_request_body",
        message: "A body-bearing request contract cannot be executed without a request body."
            .to_string(),
    }
}

pub fn validate_json_object_envelope(
    substrate: &'static str,
    arguments: &Value,
) -> Vec<RequestContractViolation> {
    if arguments.is_object() {
        Vec::new()
    } else {
        vec![RequestContractViolation {
            code: "invalid_arguments_envelope",
            message: format!(
                "{} actions require a JSON object arguments envelope before transport.",
                substrate
            ),
        }]
    }
}

pub fn expected_request_contract() -> Value {
    serde_json::json!({
        "required_when_body_bearing": {
            "method": "A concrete HTTP method. Body-bearing protocol requests are sent with a method that can carry the body.",
            "headers": "Content-Type must be present when a JSON body is sent.",
            "body": "JSON request body, or a JSON-encoded string that can be coerced before transport."
        },
        "normalization": [
            "JSON bodies get Content-Type: application/json when no Content-Type is supplied.",
            "JSON-encoded string bodies are parsed before transport.",
            "Body-bearing protocol requests are not sent as zero-length requests when a body is required."
        ]
    })
}

/// Complete acquisition contract for saving an API integration — taught WHOLE
/// on the first violation so a retry can satisfy it in one corrected call
/// instead of discovering requirements serially across failed attempts
/// ("missing source" then "missing name" then ...). Accepted source keys are
/// enumerated from SOURCE_ALIAS_KEYS so the contract can never advertise a
/// field the code does not consume.
pub fn expected_custom_api_acquisition_contract() -> Value {
    serde_json::json!({
        "integration_type": "custom_api",
        "required": {
            "name": "Integration name (or `id` when updating an existing record).",
            "endpoint_or_source": format!(
                "At least one of: `base_url`, an operation entry with an absolute URL, or one source field ({}).",
                SOURCE_ALIAS_KEYS.join(", ")
            )
        },
        "recommended": {
            "auth": "auth_type/auth_mode plus auth_header or auth_name when the provider needs credentials. Secrets are never passed here; they are saved through the secure Settings form.",
            "operations": "Operation entries (method, path, body shape) when no machine-readable source is supplied."
        },
        "accepted_source_keys": SOURCE_ALIAS_KEYS,
        "normalization": [
            "Every accepted source key folds into the same source slot; field-name guessing is unnecessary.",
            "Reuse values already present in the conversation (for example a URL the user supplied) instead of asking again."
        ]
    })
}

pub fn expected_arguments_envelope_contract(substrate: &'static str) -> Value {
    serde_json::json!({
        "substrate": substrate,
        "required": {
            "arguments": "JSON object envelope containing the action's declared fields."
        },
        "normalization": [
            "Non-HTTP substrates validate the AgentArk arguments envelope.",
            "HTTP-only wire rules such as Content-Type are not applied to JSON-RPC or messaging management calls."
        ]
    })
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    #[test]
    fn source_aliases_accept_generic_source_slots() {
        for key in SOURCE_ALIAS_KEYS {
            let mut object = serde_json::Map::new();
            object.insert(
                (*key).to_string(),
                Value::String(" https://provider.example/docs ".to_string()),
            );
            let value = Value::Object(object);
            assert_eq!(
                source_alias_value(&value),
                Some((*key, "https://provider.example/docs".to_string()))
            );
        }
    }

    #[test]
    fn source_aliases_accept_structured_manifest_payloads() {
        let value = json!({
            "manifest": {
                "openapi": "3.0.0",
                "info": { "title": "Provider" },
                "paths": {}
            }
        });

        let (_, source) = source_alias_value(&value).expect("structured source should be accepted");
        let parsed: Value = serde_json::from_str(&source).expect("source should be JSON");
        assert_eq!(parsed["openapi"], "3.0.0");
    }

    #[test]
    fn body_without_content_type_gets_json_content_type() {
        let fix = normalize_request_shape("POST", false, Some(&json!({"a": 1})), false);

        assert_eq!(fix.method, "POST");
        assert_eq!(fix.body, Some(json!({"a": 1})));
        assert_eq!(fix.content_type, Some("application/json"));
    }

    #[test]
    fn json_encoded_body_string_is_coerced_to_json() {
        let fix = normalize_request_shape(
            "POST",
            true,
            Some(&Value::String(
                r#"{"query":"query Probe { viewer { id } }"}"#.to_string(),
            )),
            true,
        );

        assert_eq!(
            fix.body,
            Some(json!({"query": "query Probe { viewer { id } }"}))
        );
    }

    #[test]
    fn graphql_get_with_bare_query_becomes_post_json_body() {
        let fix = normalize_request_shape(
            "GET",
            false,
            Some(&Value::String("query Probe { viewer { id } }".to_string())),
            true,
        );

        assert_eq!(fix.method, "POST");
        assert_eq!(
            fix.body,
            Some(json!({"query": "query Probe { viewer { id } }"}))
        );
        assert_eq!(fix.content_type, Some("application/json"));
    }

    #[test]
    fn non_http_substrates_validate_arguments_envelope_only() {
        assert!(validate_json_object_envelope("mcp", &json!({"query": "ok"})).is_empty());
        let violations = validate_json_object_envelope("mcp", &json!("not-an-object"));
        assert_eq!(violations[0].code, "invalid_arguments_envelope");
    }
}
