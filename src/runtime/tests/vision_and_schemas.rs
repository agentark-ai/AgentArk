use super::*;

#[tokio::test]

async fn vision_ocr_is_read_only_and_not_media_generation_gated() {
    let runtime = runtime_for_authorization_tests().await;

    let action = action_def_by_name(&runtime, "vision_ocr").await;

    assert!(action.capabilities.contains(&"vision_ocr".to_string()));

    assert!(!action
        .capabilities
        .contains(&"image_generation".to_string()));

    assert!(action.authorization.access.integration_ids.is_empty());
}

fn vision_test_slot(
    id: &str,

    role: crate::core::runtime::config::ModelRole,

    provider: crate::core::LlmProvider,
) -> crate::core::runtime::config::ModelSlot {
    crate::core::runtime::config::ModelSlot {
        id: id.to_string(),

        label: id.to_string(),

        role,

        provider,

        enabled: true,

        capability_tier: crate::core::runtime::config::ModelCapabilityTier::Balanced,

        cost_tier: crate::core::runtime::config::ModelCostTier::Medium,

        auto_escalate: true,

        escalation_rank: 0,

        health_scope: crate::core::runtime::config::ModelHealthScope::Provider,
    }
}

#[test]

fn configured_chat_vision_candidates_prefer_model_pool_primary() {
    let mut config = crate::core::runtime::config::AgentConfig {
        llm: crate::core::LlmProvider::OpenAI {
            api_key: "legacy-key".to_string(),

            model: "legacy-model".to_string(),

            base_url: None,
        },

        ..crate::core::runtime::config::AgentConfig::default()
    };

    config.model_pool.slots = vec![
        vision_test_slot(
            "fast",
            crate::core::runtime::config::ModelRole::Fast,
            crate::core::LlmProvider::OpenAI {
                api_key: "fast-key".to_string(),

                model: "fast-model".to_string(),

                base_url: Some(
                    crate::core::model::llm_provider::OPENROUTER_API_BASE_URL.to_string(),
                ),
            },
        ),
        vision_test_slot(
            "primary",
            crate::core::runtime::config::ModelRole::Primary,
            crate::core::LlmProvider::OpenAI {
                api_key: "primary-key".to_string(),

                model: "primary-model".to_string(),

                base_url: Some(
                    crate::core::model::llm_provider::OPENROUTER_API_BASE_URL.to_string(),
                ),
            },
        ),
    ];

    let candidates = ActionRuntime::openai_compatible_chat_vision_candidates(&config);

    assert_eq!(candidates[0].model, "primary-model");

    assert_eq!(candidates[0].provider_label(), "openrouter");

    assert!(candidates
        .iter()
        .any(|candidate| candidate.model == "legacy-model"));
}

#[test]

fn configured_chat_vision_candidates_skip_missing_managed_provider_keys() {
    let mut config = crate::core::runtime::config::AgentConfig {
        llm: crate::core::LlmProvider::Anthropic {
            api_key: "anthropic-key".to_string(),

            model: "text-model".to_string(),
        },

        ..crate::core::runtime::config::AgentConfig::default()
    };

    config.model_pool.slots = vec![
        vision_test_slot(
            "missing-openrouter",
            crate::core::runtime::config::ModelRole::Primary,
            crate::core::LlmProvider::OpenAI {
                api_key: String::new(),

                model: "openrouter-model".to_string(),

                base_url: Some(
                    crate::core::model::llm_provider::OPENROUTER_API_BASE_URL.to_string(),
                ),
            },
        ),
        vision_test_slot(
            "local-compatible",
            crate::core::runtime::config::ModelRole::Fallback,
            crate::core::LlmProvider::OpenAI {
                api_key: String::new(),

                model: "local-vision-model".to_string(),

                base_url: Some("http://127.0.0.1:11434/v1".to_string()),
            },
        ),
    ];

    let candidates = ActionRuntime::openai_compatible_chat_vision_candidates(&config);

    assert_eq!(candidates.len(), 1);

    assert_eq!(candidates[0].model, "local-vision-model");

    assert_eq!(candidates[0].provider_label(), "openai-compatible");
}

#[test]

fn configured_chat_vision_candidates_dedupe_legacy_primary_copy() {
    let mut config = crate::core::runtime::config::AgentConfig::default();

    let provider = crate::core::LlmProvider::OpenAI {
        api_key: "shared-key".to_string(),

        model: "same-model".to_string(),

        base_url: Some(crate::core::model::llm_provider::OPENROUTER_API_BASE_URL.to_string()),
    };

    config.llm = provider.clone();

    config.model_pool.slots = vec![vision_test_slot(
        "primary",
        crate::core::runtime::config::ModelRole::Primary,
        provider,
    )];

    let candidates = ActionRuntime::openai_compatible_chat_vision_candidates(&config);

    assert_eq!(candidates.len(), 1);

    assert_eq!(candidates[0].model, "same-model");
}

#[tokio::test]

async fn document_lookup_has_native_executor() {
    let runtime = runtime_for_authorization_tests().await;

    let error = runtime
        .execute_action_with_context(
            "document_lookup",
            &serde_json::json!({"query": "uploaded document"}),
            &trusted_chat_context("native-document-lookup", false),
        )
        .await
        .unwrap_err()
        .to_string();

    assert!(!error.contains("Unknown native action"));

    assert!(
        error.contains("storage") || error.contains("not available"),
        "unexpected document_lookup error: {error}"
    );
}

#[tokio::test]

async fn list_watchers_has_native_executor() {
    let runtime = runtime_for_authorization_tests().await;

    let error = runtime
        .execute_action_with_context(
            "list_watchers",
            &serde_json::json!({"filter": "all"}),
            &trusted_chat_context("native-list-watchers", false),
        )
        .await
        .unwrap_err()
        .to_string();

    assert!(!error.contains("Unknown native action"));

    assert!(
        error.contains("Storage not available") || error.contains("not available"),
        "unexpected list_watchers error: {error}"
    );
}

#[tokio::test]

async fn app_and_automation_action_contracts_separate_cadence_ownership() {
    let runtime = runtime_for_authorization_tests().await;

    let app_deploy = action_def_by_name(&runtime, "app_deploy").await;

    let schedule_task = action_def_by_name(&runtime, "schedule_task").await;

    let watch = action_def_by_name(&runtime, "watch").await;

    let background_session_manage = action_def_by_name(&runtime, "background_session_manage").await;

    assert!(app_deploy
        .description
        .contains("implement that behavior inside the artifact"));

    assert!(schedule_task
        .description
        .contains("cadence that belongs inside a generated app"));

    assert!(watch
        .description
        .contains("inside a generated app, dashboard, page, or tool's own UI"));

    assert!(background_session_manage
        .description
        .contains("durable AgentArk background session"));

    assert!(background_session_manage
        .description
        .contains("not app-internal refresh/poll cadence"));
}

#[tokio::test]

async fn watch_schema_advertises_semantic_contract_without_low_level_poll_fields() {
    let runtime = runtime_for_authorization_tests().await;

    let watch = action_def_by_name(&runtime, "watch").await;

    assert!(watch.description.contains("semantic poll contract"));

    let properties = watch.input_schema["properties"]
        .as_object()
        .expect("watch schema properties should be an object");

    for field in ["description", "condition", "on_trigger"] {
        let field_schema = properties
            .get(field)
            .and_then(|value| value.as_object())
            .unwrap_or_else(|| panic!("missing watch schema field {field}"));

        let description = field_schema
            .get("description")
            .and_then(|value| value.as_str())
            .unwrap_or_default();

        assert!(
            !description.trim().is_empty(),
            "watch schema field {field} should be described"
        );
    }

    let one_of = watch.input_schema["oneOf"]
        .as_array()
        .expect("watch schema should expose oneOf alternatives");

    assert!(
        one_of.iter().any(|alternative| {
            alternative["required"].as_array().is_some_and(|required| {
                required
                    == &vec![
                        serde_json::Value::String("description".to_string()),
                        serde_json::Value::String("condition".to_string()),
                        serde_json::Value::String("on_trigger".to_string()),
                    ]
            })
        }),
        "watch schema should allow semantic description+condition+on_trigger without poll_action/script"
    );
}

#[derive(Debug)]

struct DurableSchemaContract<'a> {
    action: &'a str,

    top_level_required: &'a [&'a str],

    top_level_alternatives: &'a [&'a [&'a str]],

    item_alternatives: &'a [&'a [&'a str]],

    conditional_required_described: &'a [&'a str],
}

fn schema_required_set(value: &serde_json::Value) -> std::collections::BTreeSet<String> {
    value
        .get("required")
        .and_then(|value| value.as_array())
        .into_iter()
        .flatten()
        .filter_map(|value| value.as_str())
        .map(str::to_string)
        .collect()
}

fn schema_required_alternatives(
    schema: &serde_json::Value,
) -> Vec<std::collections::BTreeSet<String>> {
    ["oneOf", "anyOf"]
        .into_iter()
        .filter_map(|key| schema.get(key).and_then(|value| value.as_array()))
        .flatten()
        .map(schema_required_set)
        .filter(|required| !required.is_empty())
        .collect()
}

fn assert_schema_field_is_described(
    action: &str,

    scope: &str,

    schema: &serde_json::Value,

    field: &str,
) {
    let description = schema
        .get("properties")
        .and_then(|value| value.as_object())
        .and_then(|properties| properties.get(field))
        .and_then(|field_schema| field_schema.get("description"))
        .and_then(|value| value.as_str())
        .map(str::trim)
        .unwrap_or_default();

    assert!(
        !description.is_empty(),
        "{action} {scope} required field `{field}` must exist in schema properties with a description"
    );
}

fn assert_schema_exposes_required_alternative(
    action: &str,

    scope: &str,

    schema: &serde_json::Value,

    required: &[&str],
) {
    let expected = required
        .iter()
        .map(|field| field.to_string())
        .collect::<std::collections::BTreeSet<_>>();

    let alternatives = schema_required_alternatives(schema);

    assert!(
        alternatives.iter().any(|actual| actual == &expected),
        "{action} {scope} schema is missing required-field alternative {:?}; found {:?}",
        expected,
        alternatives
    );

    for field in required {
        assert_schema_field_is_described(action, scope, schema, field);
    }
}

fn durable_item_schema<'a>(action: &str, schema: &'a serde_json::Value) -> &'a serde_json::Value {
    schema
        .pointer("/properties/items/items")
        .unwrap_or_else(|| panic!("{action} schema must expose properties.items.items"))
}

#[tokio::test]

async fn durable_action_schemas_match_server_side_required_contracts() {
    let runtime = runtime_for_authorization_tests().await;

    let schedule_task_alternatives: &[&[&str]] = &[
        &["task", "cron"],
        &["task", "at"],
        &["task", "scheduled_for"],
        &["task", "local_time"],
        &["task_id", "cron"],
        &["task_id", "at"],
        &["task_id", "scheduled_for"],
        &["task_id", "local_time"],
        &["action_arguments", "cron"],
        &["action_arguments", "at"],
        &["action_arguments", "scheduled_for"],
        &["action_arguments", "local_time"],
    ];

    let durable_contracts = [
        DurableSchemaContract {
            action: "schedule_task",

            top_level_required: &[],

            top_level_alternatives: &[
                &["task", "cron"],
                &["task", "at"],
                &["task", "scheduled_for"],
                &["task", "local_time"],
                &["task_id", "cron"],
                &["task_id", "at"],
                &["task_id", "scheduled_for"],
                &["task_id", "local_time"],
                &["action_arguments", "cron"],
                &["action_arguments", "at"],
                &["action_arguments", "scheduled_for"],
                &["action_arguments", "local_time"],
                &["items"],
            ],

            item_alternatives: schedule_task_alternatives,

            conditional_required_described: &[],
        },
        DurableSchemaContract {
            action: "watch",

            top_level_required: &[],

            top_level_alternatives: &[
                &["description", "condition", "on_trigger"],
                &["description", "poll_action", "condition", "on_trigger"],
                &["description", "script", "condition", "on_trigger"],
                &["watcher_id"],
                &["items"],
            ],

            item_alternatives: &[
                &["description", "condition", "on_trigger"],
                &["description", "poll_action", "condition", "on_trigger"],
                &["description", "script", "condition", "on_trigger"],
                &["watcher_id"],
            ],

            conditional_required_described: &[],
        },
        DurableSchemaContract {
            action: "background_session_manage",

            top_level_required: &["operation"],

            top_level_alternatives: &[],

            item_alternatives: &[],

            conditional_required_described: &["delivery_channel"],
        },
    ];

    for contract in durable_contracts {
        let action = action_def_by_name(&runtime, contract.action).await;

        let schema = &action.input_schema;

        let required = schema_required_set(schema);

        for field in contract.top_level_required {
            assert!(
                required.contains(*field),
                "{} schema must mark `{}` as required",
                contract.action,
                field
            );

            assert_schema_field_is_described(contract.action, "top-level", schema, field);
        }

        for field in contract.conditional_required_described {
            assert_schema_field_is_described(contract.action, "top-level", schema, field);
        }

        for alternative in contract.top_level_alternatives {
            assert_schema_exposes_required_alternative(
                contract.action,
                "top-level",
                schema,
                alternative,
            );
        }

        if !contract.item_alternatives.is_empty() {
            let item_schema = durable_item_schema(contract.action, schema);

            for alternative in contract.item_alternatives {
                assert_schema_exposes_required_alternative(
                    contract.action,
                    "items[]",
                    item_schema,
                    alternative,
                );
            }
        }
    }
}

#[tokio::test]

async fn app_deploy_schema_exposes_acceptance_contract_fields() {
    let runtime = runtime_for_authorization_tests().await;

    let app_deploy = action_def_by_name(&runtime, "app_deploy").await;

    let properties = app_deploy.input_schema["properties"]
        .as_object()
        .expect("app_deploy schema properties should be an object");

    assert!(properties.contains_key("request_context"));

    assert!(properties.contains_key("acceptance_criteria"));

    assert!(app_deploy.description.contains("acceptance"));
}
