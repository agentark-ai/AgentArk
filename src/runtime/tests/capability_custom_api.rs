use super::*;

#[test]

fn capability_acquire_graphql_operation_is_read_only_and_query_guarded() {
    let (_, operation, _) = ActionRuntime::capability_operation_draft(
        &serde_json::json!({

            "base_url": "https://api.example.com",

            "method": "post",

            "path": "/graphql"

        }),
        "graph-api",
        "Query a GraphQL API",
    )
    .expect("GraphQL capability draft should be created");

    assert!(operation.read_only);

    assert!(operation.body_required);

    assert_eq!(
        operation
            .default_headers
            .get("Content-Type")
            .map(String::as_str),
        Some("application/json")
    );

    assert!(operation.parameters.iter().any(|parameter| {
        parameter.name == "body"
            && matches!(
                parameter.location,
                crate::custom_apis::CustomApiParameterLocation::Body
            )
            && parameter.required
    }));
}

#[test]

fn capability_acquire_graphql_endpoint_without_body_shape_stores_post_json_body_contract() {
    let (_, operation, _) = ActionRuntime::capability_operation_draft(
        &serde_json::json!({

            "base_url": "https://api.example.com",

            "path": "/graphql"

        }),
        "graph-api",
        "Query a GraphQL API",
    )
    .expect("GraphQL capability draft should be created");

    assert_eq!(operation.method, "POST");

    assert!(operation.read_only);

    assert!(operation.body_required);

    assert_eq!(
        operation
            .default_headers
            .get("Content-Type")
            .map(String::as_str),
        Some("application/json")
    );

    assert!(operation.parameters.iter().any(|parameter| {
        parameter.name == "body"
            && matches!(
                parameter.location,
                crate::custom_apis::CustomApiParameterLocation::Body
            )
            && parameter.required
    }));
}

#[test]

fn capability_acquire_graphql_operation_preserves_default_read_body() {
    let (_, operation, _) = ActionRuntime::capability_operation_draft(
        &serde_json::json!({

            "base_url": "https://api.example.com",

            "operation": {

                "id": "viewer",

                "method": "post",

                "path": "/graphql",

                "body": {

                    "query": "query Viewer { viewer { id } }"

                }

            }

        }),
        "graph-api",
        "Query a GraphQL API",
    )
    .expect("GraphQL capability draft should be created");

    assert!(operation.read_only);

    assert!(operation.body_required);

    assert_eq!(
        operation
            .default_body
            .as_ref()
            .and_then(|body| body.get("query"))
            .and_then(|query| query.as_str()),
        Some("query Viewer { viewer { id } }")
    );
}

#[test]

fn capability_acquire_requires_source_for_new_custom_api_contract() {
    assert!(
        ActionRuntime::capability_acquire_needs_source_for_custom_api(
            &serde_json::json!({

                "name": "provider-api",

                "base_url": "https://api.example.com",

                "path": "/v1/items",

                "method": "get"

            }),
            false,
        )
    );
}

#[test]

fn capability_acquire_requires_source_for_generic_rest_default_body() {
    assert!(
        ActionRuntime::capability_acquire_needs_source_for_custom_api(
            &serde_json::json!({

                "id": "provider-api",

                "base_url": "https://api.example.com",

                "operation": {

                    "id": "create-item",

                    "method": "post",

                    "path": "/v1/items",

                    "body": {

                        "title": "$parameters.title"

                    }

                }

            }),
            false,
        )
    );
}

#[test]

fn capability_acquire_requires_source_for_existing_operation_contract_update() {
    assert!(
        ActionRuntime::capability_acquire_needs_source_for_custom_api(
            &serde_json::json!({

                "id": "provider-api",

                "operations": [

                    {

                        "id": "list-items",

                        "method": "get",

                        "path": "/v1/items"

                    }

                ]

            }),
            true,
        )
    );
}

#[test]

fn capability_acquire_allows_existing_auth_only_update_without_source() {
    assert!(
        !ActionRuntime::capability_acquire_needs_source_for_custom_api(
            &serde_json::json!({

                "id": "provider-api",

                "auth_mode": "api_key_header",

                "auth_header_name": "X-API-Key"

            }),
            true,
        )
    );
}

#[test]

fn capability_acquire_allows_source_backed_operation_contract() {
    assert!(
        !ActionRuntime::capability_acquire_needs_source_for_custom_api(
            &serde_json::json!({

                "name": "provider-api",

                "docs_text": "GET https://api.example.com/v1/items\nAuthorization: Bearer <ACCESS_TOKEN>",

                "base_url": "https://api.example.com",

                "path": "/v1/items",

                "method": "get"

            }),
            false,
        )
    );
}

#[test]

fn capability_acquire_uses_shared_source_alias_contract_at_runtime() {
    let arguments = serde_json::json!({
        "name": "provider-api",
        "source_url": "https://provider.example.dev/api",
        "base_url": "https://api.example.com",
        "path": "/v1/items",
        "method": "get"
    });

    assert!(ActionRuntime::capability_acquire_has_http_endpoint(
        &arguments
    ));
    assert!(
        !ActionRuntime::capability_acquire_needs_source_for_custom_api(&arguments, false),
        "source aliases should count as source-backed evidence when operation shape is present"
    );
    let preview_request = ActionRuntime::custom_api_preview_request_from_source_contract(
        &arguments,
        "provider-api".to_string(),
    )
    .expect("source alias should build preview request");
    assert_eq!(
        preview_request.source.as_deref(),
        Some("https://provider.example.dev/api")
    );
}

#[tokio::test]

async fn custom_api_action_reports_missing_body_contract_before_network_send() {
    let temp = tempfile::tempdir().unwrap();
    let runtime = ActionRuntime::new(temp.path(), temp.path()).await.unwrap();
    let output = runtime
        .execute_custom_api_action(
            CustomApiBinding {
                api_id: "provider-api".to_string(),
                api_name: "Provider API".to_string(),
                operation_id: "query".to_string(),
                operation_name: "Query".to_string(),
                method: "POST".to_string(),
                base_url: "https://api.example.com".to_string(),
                path: "/graphql".to_string(),
                read_only: true,
                secret_key: "unused".to_string(),
                auth_profile_id: None,
                auth_mode: crate::custom_apis::CustomApiAuthMode::None,
                auth_header: None,
                auth_name: None,
                auth_username: None,
                default_headers: BTreeMap::new(),
                default_query: BTreeMap::new(),
                parameters: Vec::new(),
                body_required: true,
                default_body: None,
            },
            &serde_json::json!({}),
        )
        .await
        .expect("contract violation should be structured output");

    let payload: serde_json::Value = serde_json::from_str(
        output
            .strip_prefix(crate::runtime::TOOL_COMPLETION_MARKER)
            .expect("tool completion marker"),
    )
    .expect("structured payload");
    assert_eq!(payload["status"], serde_json::json!("needs_arguments"));
    assert!(payload["data"]["expected_contract"].is_object());
    assert_eq!(
        payload["data"]["violations"][0]["code"],
        serde_json::json!("missing_request_body")
    );
}

#[tokio::test]

async fn http_request_reports_missing_body_contract_before_network_send() {
    let temp = tempfile::tempdir().unwrap();
    let runtime = ActionRuntime::new(temp.path(), temp.path()).await.unwrap();
    let output = runtime
        .execute_http_request(&serde_json::json!({
            "url": "https://example.com/graphql",
            "method": "POST"
        }))
        .await
        .expect("contract violation should be structured output");

    let payload: serde_json::Value = serde_json::from_str(
        output
            .strip_prefix(crate::runtime::TOOL_COMPLETION_MARKER)
            .expect("tool completion marker"),
    )
    .expect("structured payload");
    assert_eq!(payload["status"], serde_json::json!("needs_arguments"));
    assert_eq!(
        payload["data"]["violations"][0]["code"],
        serde_json::json!("missing_request_body")
    );
}

#[tokio::test]

async fn non_http_substrates_report_invalid_arguments_envelope() {
    let temp = tempfile::tempdir().unwrap();
    let runtime = ActionRuntime::new(temp.path(), temp.path()).await.unwrap();

    let mcp = runtime
        .execute_mcp_action(
            McpBinding {
                server_id: "server".to_string(),
                server_name: "Server".to_string(),
                warnings: Vec::new(),
                auth_profile_id: None,
                auth_required: false,
                auth_configured: false,
                kind: McpBindingKind::Tool {
                    name: "tool".to_string(),
                },
            },
            &serde_json::json!("not-an-object"),
        )
        .await
        .expect("invalid envelope should not need an MCP registry");
    let mcp_payload: serde_json::Value = serde_json::from_str(
        mcp.strip_prefix(crate::runtime::TOOL_COMPLETION_MARKER)
            .expect("tool completion marker"),
    )
    .expect("structured payload");
    assert_eq!(mcp_payload["status"], serde_json::json!("needs_arguments"));
    assert_eq!(
        mcp_payload["data"]["violations"][0]["code"],
        serde_json::json!("invalid_arguments_envelope")
    );

    let messaging = runtime
        .execute_custom_messaging_channel_upsert(&serde_json::json!("not-an-object"))
        .await
        .expect("invalid envelope should not need storage");
    let messaging_payload: serde_json::Value = serde_json::from_str(
        messaging
            .strip_prefix(crate::runtime::TOOL_COMPLETION_MARKER)
            .expect("tool completion marker"),
    )
    .expect("structured payload");
    assert_eq!(
        messaging_payload["status"],
        serde_json::json!("needs_arguments")
    );
    assert_eq!(
        messaging_payload["data"]["expected_contract"]["substrate"],
        serde_json::json!("custom_messaging_channel")
    );
}

#[test]

fn capability_acquire_requires_structured_contract_for_raw_docs_source() {
    assert!(
        ActionRuntime::capability_acquire_needs_source_for_custom_api(
            &serde_json::json!({

                "id": "provider-api",

                "_capability_source_evidence": ["docs"],

                "docs_url": "https://provider.example.dev/api"

            }),
            false,
        )
    );
}

#[tokio::test]

async fn capability_acquire_needs_source_output_reports_search_provider_state() {
    let temp = tempfile::tempdir().unwrap();

    let runtime = ActionRuntime::new(temp.path(), temp.path()).await.unwrap();

    let output = runtime
        .capability_acquire_needs_source_output(
            "provider-api",
            &serde_json::json!({

                "base_url": "https://api.example.com",

                "path": "/v1/items",

                "method": "get"

            }),
            false,
            false,
        )
        .await
        .unwrap();

    let parsed = parse_tool_completion_output(&output);

    assert_eq!(parsed["status"], serde_json::json!("needs_source"));
    assert_eq!(parsed["tool"], serde_json::json!("capability_acquire"));

    assert_eq!(
        parsed["data"]["reason"],
        serde_json::json!("unverified_integration_contract")
    );

    assert_eq!(
        parsed["data"]["search_provider_configured"],
        serde_json::json!(false)
    );

    assert!(parsed["data"]["search_provider_setup"]
        .as_str()
        .is_some_and(|value| value.contains("Configure a reachable SearXNG instance")));

    assert!(parsed["data"]["unverified_contract_fields"]
        .as_array()
        .is_some_and(|fields| fields.iter().any(|field| field.as_str() == Some("path"))));
}

#[test]

fn capability_acquire_graphql_body_template_string_is_stored_as_json_body() {
    let (_, operation, _) = ActionRuntime::capability_operation_draft(
        &serde_json::json!({

            "base_url": "https://api.example.com",

            "method": "post",

            "path": "/graphql",

            "body_template": "{\"query\":\"query Viewer { viewer { id } }\"}"

        }),
        "graph-api",
        "Query a GraphQL API",
    )
    .expect("GraphQL capability draft should be created");

    assert!(operation.read_only);

    assert_eq!(
        operation
            .default_body
            .as_ref()
            .and_then(|body| body.get("query"))
            .and_then(|query| query.as_str()),
        Some("query Viewer { viewer { id } }")
    );
}

#[test]

fn capability_acquire_auth_fields_preserve_existing_auth_when_omitted() {
    let (auth_mode, auth_header, auth_name, auth_username) =
        ActionRuntime::capability_auth_fields(&serde_json::json!({}));

    assert!(auth_mode.is_none());

    assert!(auth_header.is_none());

    assert!(auth_name.is_none());

    assert!(auth_username.is_none());
}

#[test]

fn capability_acquire_auth_fields_accept_auth_mode_alias_for_header_key() {
    let (auth_mode, auth_header, auth_name, auth_username) =
        ActionRuntime::capability_auth_fields(&serde_json::json!({

            "auth_mode": "api_key_header",

            "auth_header": "Authorization"

        }));

    assert_eq!(
        auth_mode,
        Some(crate::custom_apis::CustomApiAuthMode::ApiKeyHeader)
    );

    assert!(auth_header.is_none());

    assert_eq!(auth_name.as_deref(), Some("Authorization"));

    assert!(auth_username.is_none());
}

#[test]

fn capability_acquire_auth_fields_accept_auth_object_for_header_key() {
    let (auth_mode, auth_header, auth_name, auth_username) =
        ActionRuntime::capability_auth_fields(&serde_json::json!({

            "auth": {

                "type": "header",

                "name": "Authorization"

            }

        }));

    assert_eq!(
        auth_mode,
        Some(crate::custom_apis::CustomApiAuthMode::ApiKeyHeader)
    );

    assert!(auth_header.is_none());

    assert_eq!(auth_name.as_deref(), Some("Authorization"));

    assert!(auth_username.is_none());
}

#[test]

fn capability_acquire_accepts_operation_object_for_http_shape() {
    let (_, operation, headers) = ActionRuntime::capability_operation_draft(
        &serde_json::json!({

            "base_url": "https://api.example.com",

            "operation": {

                "method": "post",

                "path": "/graphql",

                "default_headers": {

                    "content-type": "application/json"

                },

                "body_required": true

            }

        }),
        "graph-api",
        "Query a saved API",
    )
    .expect("operation object should create a custom API operation");

    assert_eq!(operation.method, "POST");

    assert_eq!(operation.path, "/graphql");

    assert!(operation.body_required);

    assert_eq!(headers["content-type"], "application/json");
}

#[test]

fn capability_acquire_routes_existing_id_updates_as_http_shape() {
    assert!(ActionRuntime::capability_acquire_has_http_endpoint(
        &serde_json::json!({

            "id": "provider-api",

            "method": "post",

            "path": "/graphql"

        })
    ));
}

#[test]

fn capability_acquire_accepts_operations_array_for_http_shape() {
    let (base_url, operations) = ActionRuntime::capability_operation_drafts(
        &serde_json::json!({

            "base_url": "https://api.example.com",

            "operations": [

                {

                    "id": "list-items",

                    "method": "get",

                    "path": "/items"

                },

                {

                    "id": "query-graph",

                    "method": "post",

                    "path": "/graphql",

                    "default_headers": {

                        "content-type": "application/json"

                    },

                    "body_required": true

                }

            ]

        }),
        "provider-api",
        "Read a saved provider API",
    )
    .expect("operations array should create custom API operations");

    assert_eq!(base_url, "https://api.example.com");

    assert_eq!(operations.len(), 2);

    assert_eq!(operations[0].id, "list-items");

    assert_eq!(operations[1].method, "POST");

    assert!(operations[1].body_required);
}

#[test]

fn custom_api_inventory_marks_failed_test_not_connected() {
    let view = crate::custom_apis::CustomApiView {
        config: crate::custom_apis::CustomApiConfig {
            id: "provider-api".to_string(),

            name: "Provider API".to_string(),

            description: String::new(),

            base_url: "https://api.example.com".to_string(),

            enabled: true,

            auth_mode: crate::custom_apis::CustomApiAuthMode::Bearer,

            auth_profile_id: None,

            auth_header: Some("Authorization".to_string()),

            auth_name: None,

            auth_username: None,

            created_at: "2026-05-23T00:00:00Z".to_string(),

            updated_at: "2026-05-23T00:00:00Z".to_string(),

            last_tested_at: Some("2026-05-23T00:00:00Z".to_string()),

            last_test_outcome: Some("failure".to_string()),

            last_test_message: Some("HTTP 400".to_string()),

            operations: Vec::new(),
        },

        secret_configured: true,

        action_count: 1,

        capability_contract: serde_json::json!({}),

        test_action_name: None,
    };

    assert!(!ActionRuntime::custom_api_view_is_connected(&view));

    assert!(ActionRuntime::custom_api_view_is_callable(&view));
}

#[test]

fn custom_api_view_state_contract_exposes_live_readiness_fields() {
    let view = crate::custom_apis::CustomApiView {
        config: crate::custom_apis::CustomApiConfig {
            id: "provider-api".to_string(),

            name: "Provider API".to_string(),

            description: String::new(),

            base_url: "https://api.example.com".to_string(),

            enabled: true,

            auth_mode: crate::custom_apis::CustomApiAuthMode::Bearer,

            auth_profile_id: None,

            auth_header: Some("Authorization".to_string()),

            auth_name: None,

            auth_username: None,

            created_at: "2026-05-23T00:00:00Z".to_string(),

            updated_at: "2026-05-23T00:00:00Z".to_string(),

            last_tested_at: None,

            last_test_outcome: None,

            last_test_message: None,

            operations: vec![crate::custom_apis::CustomApiOperation {
                draft: crate::custom_apis::CustomApiOperationDraft {
                    id: "read".to_string(),

                    name: "Read".to_string(),

                    method: "GET".to_string(),

                    path: "/items".to_string(),

                    description: String::new(),

                    read_only: true,

                    enabled: true,

                    default_headers: BTreeMap::new(),

                    default_query: BTreeMap::new(),

                    parameters: Vec::new(),

                    body_required: false,

                    default_body: None,
                },

                action_name: "api__provider-api__read".to_string(),
            }],
        },

        secret_configured: true,

        action_count: 1,

        capability_contract: serde_json::json!({}),

        test_action_name: None,
    };

    let contract = ActionRuntime::custom_api_view_state_contract(&view);

    assert_eq!(contract["registered"], true);

    assert_eq!(contract["enabled"], true);

    assert_eq!(contract["secret_configured"], true);

    assert_eq!(contract["auth_ready"], true);

    assert_eq!(contract["verified"], false);

    assert_eq!(contract["connected"], true);
}

#[test]

fn custom_api_view_selector_uses_canonical_identifier_matching() {
    let view = crate::custom_apis::CustomApiView {
        config: crate::custom_apis::CustomApiConfig {
            id: "provider-api".to_string(),

            name: "Provider API".to_string(),

            description: String::new(),

            base_url: "https://api.example.com".to_string(),

            enabled: true,

            auth_mode: crate::custom_apis::CustomApiAuthMode::None,

            auth_profile_id: None,

            auth_header: None,

            auth_name: None,

            auth_username: None,

            created_at: "2026-05-23T00:00:00Z".to_string(),

            updated_at: "2026-05-23T00:00:00Z".to_string(),

            last_tested_at: None,

            last_test_outcome: None,

            last_test_message: None,

            operations: Vec::new(),
        },

        secret_configured: false,

        action_count: 1,

        capability_contract: serde_json::json!({}),

        test_action_name: None,
    };

    assert!(ActionRuntime::custom_api_view_matches_selector(
        &view,
        "provider_api"
    ));

    assert!(ActionRuntime::custom_api_view_matches_selector(
        &view,
        "Provider API"
    ));

    assert!(!ActionRuntime::custom_api_view_matches_selector(
        &view,
        "other_api"
    ));
}

#[test]

fn custom_api_request_allows_stale_graphql_operation_for_read_query_body() {
    let operation = crate::custom_apis::CustomApiOperation {
        draft: crate::custom_apis::CustomApiOperationDraft {
            id: "post-graphql".to_string(),

            name: "POST /graphql".to_string(),

            method: "POST".to_string(),

            path: "/graphql".to_string(),

            description: String::new(),

            read_only: false,

            enabled: true,

            default_headers: BTreeMap::new(),

            default_query: BTreeMap::new(),

            parameters: Vec::new(),

            body_required: false,

            default_body: None,
        },

        action_name: "api__graph_api__post-graphql".to_string(),
    };

    assert!(ActionRuntime::custom_api_operation_allows_read_request(
        &operation,
        Some(&serde_json::json!({

            "query": "query Viewer { viewer { id } }"

        }))
    ));

    assert!(!ActionRuntime::custom_api_operation_allows_read_request(
        &operation,
        Some(&serde_json::json!({

            "query": "mutation Create { createThing { id } }"

        }))
    ));

    assert!(!ActionRuntime::custom_api_operation_allows_read_request(
        &operation, None
    ));
}

#[test]

fn custom_api_request_normalizes_json_encoded_body_without_stringifying_values() {
    let operation = crate::custom_apis::CustomApiOperation {
        draft: crate::custom_apis::CustomApiOperationDraft {
            id: "search-items".to_string(),

            name: "Search Items".to_string(),

            method: "POST".to_string(),

            path: "/items/search".to_string(),

            description: String::new(),

            read_only: true,

            enabled: true,

            default_headers: BTreeMap::new(),

            default_query: BTreeMap::new(),

            parameters: vec![crate::custom_apis::CustomApiParameter {
                name: "body".to_string(),

                location: crate::custom_apis::CustomApiParameterLocation::Body,

                required: true,

                description: None,

                schema_type: Some("object".to_string()),
            }],

            body_required: true,

            default_body: None,
        },

        action_name: "api__provider__search-items".to_string(),
    };

    let supplied = serde_json::json!({

        "body": "{\"filters\":{\"state\":\"open\"},\"limit\":50}"

    });

    let normalized =
        ActionRuntime::normalize_custom_api_request_action_arguments(&operation, supplied);

    assert_eq!(normalized["body"]["limit"], serde_json::json!(50));

    assert!(normalized["body"]["limit"].as_i64().is_some());

    assert!(ActionRuntime::custom_api_operation_allows_read_request(
        &operation,
        normalized.get("body"),
    ));
}

#[test]

fn custom_api_operation_selector_uses_canonical_identifier_matching() {
    let operation = crate::custom_apis::CustomApiOperation {
        draft: crate::custom_apis::CustomApiOperationDraft {
            id: "issues-list".to_string(),

            name: "Issues List".to_string(),

            method: "POST".to_string(),

            path: "/graphql".to_string(),

            description: String::new(),

            read_only: true,

            enabled: true,

            default_headers: BTreeMap::new(),

            default_query: BTreeMap::new(),

            parameters: Vec::new(),

            body_required: true,

            default_body: Some(serde_json::json!({

                "query": "query { issues { nodes { id title } } }"

            })),
        },

        action_name: "api__linear__issues-list".to_string(),
    };

    assert!(ActionRuntime::custom_api_operation_matches_selector(
        &operation,
        "issues_list"
    ));

    assert!(ActionRuntime::custom_api_operation_matches_selector(
        &operation,
        "Issues List"
    ));

    assert!(ActionRuntime::custom_api_operation_matches_selector(
        &operation,
        "POST /graphql"
    ));

    assert!(!ActionRuntime::custom_api_operation_matches_selector(
        &operation,
        "teams_list"
    ));
}

#[test]

fn custom_api_request_selector_miss_uses_single_safe_compatible_operation() {
    let operation = crate::custom_apis::CustomApiOperation {
        draft: crate::custom_apis::CustomApiOperationDraft {
            id: "post-graphql".to_string(),

            name: "POST /graphql".to_string(),

            method: "POST".to_string(),

            path: "/graphql".to_string(),

            description: String::new(),

            read_only: true,

            enabled: true,

            default_headers: BTreeMap::new(),

            default_query: BTreeMap::new(),

            parameters: Vec::new(),

            body_required: true,

            default_body: Some(serde_json::json!({

                "query": "query { viewer { id } }"

            })),
        },

        action_name: "api__provider__post-graphql".to_string(),
    };

    let selected = ActionRuntime::select_custom_api_read_operation(
        [&operation],
        Some("viewer_query_alias"),
        None,
        false,
    )
    .expect("one safe compatible operation should be selected");

    assert_eq!(selected.0.draft.id, "post-graphql");

    assert_eq!(selected.1, "single_compatible_operation");
}

#[test]

fn graphql_response_error_detection_treats_nonempty_errors_as_failure() {
    assert!(ActionRuntime::graphql_response_has_errors(
        r#"{"errors":[{"message":"Unknown type StringFilter"}],"data":null}"#
    ));

    assert!(!ActionRuntime::graphql_response_has_errors(
        r#"{"errors":[],"data":{"viewer":{"id":"user_1"}}}"#
    ));

    assert!(!ActionRuntime::graphql_response_has_errors(
        r#"{"data":{"viewer":{"id":"user_1"}}}"#
    ));
}

#[test]

fn read_only_graphql_operation_with_mutation_default_body_is_not_callable_as_read() {
    let operation = crate::custom_apis::CustomApiOperation {
        draft: crate::custom_apis::CustomApiOperationDraft {
            id: "post-graphql".to_string(),

            name: "POST /graphql".to_string(),

            method: "POST".to_string(),

            path: "/graphql".to_string(),

            description: String::new(),

            read_only: true,

            enabled: true,

            default_headers: BTreeMap::new(),

            default_query: BTreeMap::new(),

            parameters: Vec::new(),

            body_required: true,

            default_body: Some(serde_json::json!({

                "query": "mutation Create { createThing { id } }"

            })),
        },

        action_name: "api__graph_api__post-graphql".to_string(),
    };

    assert!(!ActionRuntime::custom_api_operation_allows_read_request(
        &operation,
        operation.draft.default_body.as_ref()
    ));
}

#[tokio::test]

async fn read_only_graphql_custom_api_rejects_mutation_body_before_network() {
    let runtime = runtime_for_authorization_tests().await;

    let binding = CustomApiBinding {
        api_id: "graph-api".to_string(),

        api_name: "Graph API".to_string(),

        operation_id: "post-graphql".to_string(),

        operation_name: "POST /graphql".to_string(),

        method: "POST".to_string(),

        base_url: "https://api.example.com".to_string(),

        path: "/graphql".to_string(),

        read_only: true,

        secret_key: "custom_api_secret:graph-api".to_string(),

        auth_profile_id: None,

        auth_mode: crate::custom_apis::CustomApiAuthMode::None,

        auth_header: None,

        auth_name: None,

        auth_username: None,

        default_headers: BTreeMap::new(),

        default_query: BTreeMap::new(),

        parameters: vec![crate::custom_apis::CustomApiParameter {
            name: "body".to_string(),

            location: crate::custom_apis::CustomApiParameterLocation::Body,

            required: true,

            description: None,

            schema_type: Some("object".to_string()),
        }],

        body_required: true,

        default_body: None,
    };

    let error = runtime
        .execute_custom_api_action(
            binding,
            &serde_json::json!({

                "body": {

                    "query": "mutation Create { createThing { id } }"

                }

            }),
        )
        .await
        .expect_err("read-only GraphQL mutation should be rejected");

    assert!(error
        .to_string()
        .contains("Read-only GraphQL custom API actions only accept GraphQL query"));
}


#[tokio::test]
async fn capability_acquire_reports_whole_contract_on_missing_fields() {
    let temp = tempfile::tempdir().unwrap();
    let runtime = ActionRuntime::new(temp.path(), temp.path()).await.unwrap();
    let output = runtime
        .execute_capability_acquire(&serde_json::json!({
            "description": "An integration with no name and no endpoint"
        }))
        .await
        .unwrap();
    let parsed = parse_tool_completion_output(&output);
    assert_eq!(parsed["status"], serde_json::json!("needs_arguments"));
    assert_eq!(parsed["tool"], serde_json::json!("capability_acquire"));
    let violations = parsed["data"]["violations"].as_array().unwrap();
    assert_eq!(
        violations.len(),
        2,
        "every missing requirement is reported at once, not drip-fed: {violations:?}"
    );
    assert!(
        parsed["data"]["expected_contract"]["accepted_source_keys"]
            .as_array()
            .unwrap()
            .iter()
            .any(|key| key == "source_url"),
        "the taught contract enumerates the real accepted source keys"
    );
}

#[cfg_attr(
    not(feature = "db-tests"),
    ignore = "requires explicit isolated Postgres test database (custom API drafts persist through Storage)"
)]
#[tokio::test]
async fn capability_acquire_persists_amendable_draft_and_completes_from_delta_retry() {
    let temp = tempfile::tempdir().unwrap();
    let mut runtime = ActionRuntime::new(temp.path(), temp.path()).await.unwrap();
    let storage = crate::storage::Storage::connect(
        crate::storage::DatabaseConfig::for_tests()
            .expect("test database config should initialize"),
    )
    .await
    .expect("test database should connect");
    runtime.set_storage(storage);

    // Attempt 1: name + endpoint + auth but no source evidence -> needs_source,
    // with the non-secret fields parked as a disabled draft.
    let output = runtime
        .execute_capability_acquire(&serde_json::json!({
            "name": "Example Provider",
            "base_url": "https://api.example.com",
            "auth_type": "bearer"
        }))
        .await
        .unwrap();
    let parsed = parse_tool_completion_output(&output);
    assert_eq!(parsed["status"], serde_json::json!("needs_source"));
    assert_eq!(parsed["data"]["draft_saved"], serde_json::json!(true));

    let draft_id = parsed["data"]["id"].as_str().unwrap().to_string();
    let storage = runtime.storage.clone().unwrap();
    let views =
        crate::custom_apis::list_custom_apis(&storage, &runtime.config_dir, runtime.data_dir())
            .await
            .unwrap();
    let draft = views
        .iter()
        .find(|view| view.config.id == draft_id)
        .expect("rejected acquisition should persist an amendable draft");
    assert!(!draft.config.enabled, "drafts are parked disabled");
    assert_eq!(draft.config.base_url, "https://api.example.com");

    // Attempt 2 is a DELTA: only the id plus source evidence. Name, base_url,
    // and auth flow in from the draft instead of being restated.
    let openapi_text = serde_json::json!({
        "openapi": "3.0.0",
        "info": { "title": "Example Provider", "version": "1.0.0" },
        "servers": [ { "url": "https://api.example.com" } ],
        "paths": {
            "/v1/items": {
                "get": { "operationId": "listItems", "summary": "List items" }
            }
        }
    })
    .to_string();
    let output = runtime
        .execute_capability_acquire(&serde_json::json!({
            "id": draft_id,
            "openapi_text": openapi_text
        }))
        .await
        .unwrap();
    let parsed = parse_tool_completion_output(&output);
    assert_eq!(
        parsed["status"],
        serde_json::json!("needs_credentials"),
        "bearer auth without a saved secret surfaces the credential handoff"
    );
    assert!(parsed["data"]["credential_request"].is_object());
}


#[test]
fn protocol_defined_request_fallback_scaffolds_post_query_contract() {
    let (request, operation_count) = ActionRuntime::protocol_defined_request_fallback(
        &serde_json::json!({
            "base_url": "https://api.example.com/graphql",
            "auth_type": "api_key_header",
            "auth_header_name": "Authorization"
        }),
        None,
        "example-graphql",
        "Example GraphQL",
        "GraphQL provider",
    )
    .expect("graphql-signaled endpoint should produce a fallback")
    .expect("fallback request should build");

    assert_eq!(operation_count, 1);
    assert_eq!(request.base_url, "https://api.example.com");
    assert_eq!(request.enabled, Some(true));
    let operation = &request.operations[0];
    assert_eq!(operation.method, "POST");
    assert!(operation.body_required, "generic GraphQL op must require a body");
    assert_eq!(
        operation
            .default_headers
            .get("Content-Type")
            .map(String::as_str),
        Some("application/json")
    );

    // A non-GraphQL endpoint must NOT trigger the fallback (shape-based only).
    assert!(ActionRuntime::protocol_defined_request_fallback(
        &serde_json::json!({ "base_url": "https://api.example.com/v2" }),
        None,
        "plain-api",
        "Plain API",
        "REST provider",
    )
    .is_none());
}


#[tokio::test]
async fn capability_acquire_scaffolds_graphql_endpoint_in_one_shot_without_source() {
    // The user-facing one-shot: "install <provider> - <graphql endpoint>".
    // A GraphQL-signaled endpoint must NOT bounce to needs_source â€” the
    // protocol defines its one executable operation.
    let temp = tempfile::tempdir().unwrap();
    let runtime = ActionRuntime::new(temp.path(), temp.path()).await.unwrap();
    let output = runtime
        .execute_capability_acquire(&serde_json::json!({
            "name": "Example GraphQL",
            "base_url": "https://api.example.com/graphql",
            "auth_type": "api_key_header",
            "auth_header_name": "Authorization"
        }))
        .await;
    // Without storage the create path errors on the storage requirement, but
    // it must NOT return the needs_source envelope and must NOT be the old
    // bare "Missing name" class. With storage (db-tests) this completes to a
    // needs_credentials handoff; here we assert the gate routing only.
    match output {
        Ok(rendered) => {
            let parsed = parse_tool_completion_output(&rendered);
            assert_ne!(
                parsed["status"],
                serde_json::json!("needs_source"),
                "graphql endpoint must bypass the source-evidence gate"
            );
        }
        Err(error) => {
            assert!(
                error.to_string().contains("Storage is required"),
                "only the storage requirement may fail in this fixture, got: {error}"
            );
        }
    }
}

