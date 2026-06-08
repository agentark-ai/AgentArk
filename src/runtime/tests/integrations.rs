use super::*;

use crate::integrations::integration_enabled_key;

#[tokio::test]
async fn app_management_schemas_avoid_top_level_combinators() {
    let temp = tempfile::tempdir().unwrap();
    let runtime = ActionRuntime::new(temp.path(), temp.path()).await.unwrap();
    runtime.load_builtin_actions().await.unwrap();
    let actions = runtime.list_actions().await.unwrap();

    for action_name in ["app_deploy", "app_restart", "app_stop", "app_delete"] {
        let action = actions
            .iter()
            .find(|action| action.name == action_name)
            .unwrap_or_else(|| panic!("missing builtin action {}", action_name));
        let schema = &action.input_schema;
        assert_eq!(
            schema.get("type").and_then(|value| value.as_str()),
            Some("object"),
            "{} schema should stay a top-level object",
            action_name
        );
        for combinator in ["anyOf", "oneOf", "allOf", "not"] {
            assert!(
                schema.get(combinator).is_none(),
                "{} schema should not use top-level {}",
                action_name,
                combinator
            );
        }
    }
}

#[tokio::test]
async fn capability_resolve_is_builtin_and_inventory_scoped() {
    let temp = tempfile::tempdir().unwrap();
    let runtime = ActionRuntime::new(temp.path(), temp.path()).await.unwrap();
    runtime.load_builtin_actions().await.unwrap();
    let action = action_def_by_name(&runtime, "capability_resolve").await;

    assert_eq!(action.source, ActionSource::System);
    assert!(action.capabilities.iter().any(|cap| cap == "file_read"));
    assert!(action
        .capabilities
        .iter()
        .any(|cap| cap == "capability_inventory"));
}

#[tokio::test]
async fn manage_actions_declares_skill_management_capability() {
    let temp = tempfile::tempdir().unwrap();
    let runtime = ActionRuntime::new(temp.path(), temp.path()).await.unwrap();
    runtime.load_builtin_actions().await.unwrap();
    let action = action_def_by_name(&runtime, "manage_actions").await;

    assert_eq!(action.source, ActionSource::System);
    assert!(action
        .capabilities
        .iter()
        .any(|cap| cap == "skill_management"));
    assert_eq!(
        action.input_schema["properties"]["resource"]["enum"],
        serde_json::json!(["skill", "skill_marketplace"])
    );
    assert_eq!(
        action.input_schema["properties"]["security_confirmed"]["type"],
        serde_json::json!("boolean")
    );
}

#[tokio::test]
async fn delegate_is_builtin_multi_agent_capability() {
    let temp = tempfile::tempdir().unwrap();
    let runtime = ActionRuntime::new(temp.path(), temp.path()).await.unwrap();
    runtime.load_builtin_actions().await.unwrap();
    let action = action_def_by_name(&runtime, "delegate").await;

    assert_eq!(action.source, ActionSource::System);
    assert!(action.capabilities.iter().any(|cap| cap == "multi_agent"));
    assert!(action.capabilities.iter().any(|cap| cap == "swarm"));
    assert_eq!(
        action.input_schema["anyOf"],
        serde_json::json!([
            { "required": ["task"] },
            { "required": ["tasks"] }
        ])
    );
    let metadata = crate::actions::action_metadata_for_action(&action);
    assert_eq!(metadata.role, crate::actions::ActionRole::Orchestration);
    assert_eq!(
        metadata.orchestration_kind,
        crate::actions::ActionOrchestrationKind::MultiAgent
    );
    assert_eq!(
        metadata.side_effect_level,
        crate::actions::ActionSideEffectLevel::Write
    );
}

#[tokio::test]
async fn capability_acquire_requires_agent_permission() {
    let temp = tempfile::tempdir().unwrap();
    let runtime = ActionRuntime::new(temp.path(), temp.path()).await.unwrap();
    runtime.load_builtin_actions().await.unwrap();
    let action = action_def_by_name(&runtime, "capability_acquire").await;

    assert_eq!(action.source, ActionSource::System);
    assert!(ActionRuntime::action_required_agent_permission_ids(&action)
        .iter()
        .any(|permission| permission == "capability_acquire"));
}

#[tokio::test]
async fn system_action_review_visibility_does_not_hide_builtin_actions() {
    let temp = tempfile::tempdir().unwrap();
    let runtime = ActionRuntime::new(temp.path(), temp.path()).await.unwrap();
    runtime.load_builtin_actions().await.unwrap();
    runtime
        .upsert_action_review(ActionReviewSnapshot {
            action_name: "file_write".to_string(),
            status: ActionReviewStatus::Blocked,
            ready: false,
            allow_load: false,
            allow_execute: false,
            visible_in_catalog: false,
            blocked_reason: Some("stale persisted review".to_string()),
            ..ActionReviewSnapshot::default()
        })
        .await
        .unwrap();

    let enabled = runtime.list_enabled_actions().await.unwrap();

    assert!(enabled.iter().any(|action| action.name == "file_write"));
    assert!(runtime.is_action_enabled("file_write").await);
}

#[tokio::test]
async fn list_enabled_actions_exposes_connected_google_workspace_without_load_time_mutation() {
    let temp = tempfile::tempdir().unwrap();
    let manager = crate::core::runtime::config::SecureConfigManager::new(temp.path()).unwrap();
    manager
        .set_custom_secret(
            crate::actions::google_workspace::GOOGLE_WORKSPACE_TOKENS_KEY,
            Some(
                serde_json::json!({
                    "access_token": "access",
                    "refresh_token": "refresh",
                    "expires_at": chrono::Utc::now().timestamp() + 3600,
                    "granted_scopes": [
                        "https://www.googleapis.com/auth/gmail.readonly",
                        "https://www.googleapis.com/auth/gmail.send",
                        "https://www.googleapis.com/auth/calendar"
                    ],
                    "granted_bundles": ["gmail", "calendar"]
                })
                .to_string(),
            ),
        )
        .unwrap();
    manager
        .set_custom_secret(
            crate::actions::google_workspace::GOOGLE_WORKSPACE_BUNDLES_KEY,
            Some(serde_json::json!(["gmail", "calendar"]).to_string()),
        )
        .unwrap();
    manager
        .set_custom_secret(
            &integration_enabled_key("google_workspace"),
            Some("false".to_string()),
        )
        .unwrap();

    let runtime = ActionRuntime::new(temp.path(), temp.path()).await.unwrap();
    runtime.load_builtin_actions().await.unwrap();
    let enabled = runtime.list_enabled_actions().await.unwrap();

    assert!(enabled.iter().any(|action| action.name == "gmail_scan"));
    assert!(enabled
        .iter()
        .any(|action| action.name == "google_workspace_gws_command"));
    assert!(enabled
        .iter()
        .any(|action| action.name == "google_workspace_gws_skills"));
    assert_eq!(
        manager
            .get_custom_secret(&integration_enabled_key("google_workspace"))
            .unwrap()
            .as_deref(),
        Some("false")
    );
}

#[tokio::test]
async fn list_integrations_includes_action_backed_workspace_surfaces() {
    let temp = tempfile::tempdir().unwrap();
    let manager = crate::core::runtime::config::SecureConfigManager::new(temp.path()).unwrap();
    manager
        .set_custom_secret(
            crate::actions::google_workspace::GOOGLE_WORKSPACE_TOKENS_KEY,
            Some(
                serde_json::json!({
                    "access_token": "access",
                    "refresh_token": "refresh",
                    "expires_at": chrono::Utc::now().timestamp() + 3600,
                    "granted_scopes": [
                        "https://www.googleapis.com/auth/gmail.readonly",
                        "https://www.googleapis.com/auth/gmail.send",
                        "https://www.googleapis.com/auth/drive.readonly"
                    ],
                    "granted_bundles": ["gmail", "drive"]
                })
                .to_string(),
            ),
        )
        .unwrap();
    manager
        .set_custom_secret(
            crate::actions::google_workspace::GOOGLE_WORKSPACE_BUNDLES_KEY,
            Some(serde_json::json!(["gmail", "drive"]).to_string()),
        )
        .unwrap();

    let runtime = ActionRuntime::new(temp.path(), temp.path()).await.unwrap();
    runtime.load_builtin_actions().await.unwrap();
    let output = runtime
        .execute_list_integrations(&serde_json::json!({
            "only_connected": true,
            "include_details": true
        }))
        .await
        .unwrap();
    let value: serde_json::Value = serde_json::from_str(&output).unwrap();
    let connected_ids = value["connected_agentark_surfaces"]["items"]
        .as_array()
        .unwrap()
        .iter()
        .filter_map(|item| item.get("id").and_then(|id| id.as_str()))
        .collect::<BTreeSet<_>>();

    assert!(connected_ids.contains("gmail"));
    assert!(connected_ids.contains("google_workspace"));

    let integrations = value["builtin_integrations"]["integrations"]
        .as_array()
        .unwrap();
    let gmail = integrations
        .iter()
        .find(|item| item.get("id").and_then(|id| id.as_str()) == Some("gmail"))
        .expect("gmail action-backed surface should be visible");
    let workspace = integrations
        .iter()
        .find(|item| item.get("id").and_then(|id| id.as_str()) == Some("google_workspace"))
        .expect("workspace action-backed surface should be visible");

    assert!(gmail["available_actions"]
        .as_array()
        .unwrap()
        .iter()
        .any(|action| action.as_str() == Some("gmail_scan")));
    assert!(workspace["available_actions"]
        .as_array()
        .unwrap()
        .iter()
        .any(|action| action.as_str() == Some("google_workspace_gws_command")));
}

#[tokio::test]
async fn list_integrations_does_not_report_workspace_connected_without_grants() {
    let temp = tempfile::tempdir().unwrap();
    let runtime = ActionRuntime::new(temp.path(), temp.path()).await.unwrap();
    runtime.load_builtin_actions().await.unwrap();

    let output = runtime
        .execute_list_integrations(&serde_json::json!({
            "query": "Google Gmail",
            "only_connected": true,
            "include_details": true
        }))
        .await
        .unwrap();
    let value: serde_json::Value = serde_json::from_str(&output).unwrap();
    let connected_ids = value["connected_agentark_surfaces"]["items"]
        .as_array()
        .unwrap()
        .iter()
        .filter_map(|item| item.get("id").and_then(|id| id.as_str()))
        .collect::<BTreeSet<_>>();

    assert!(!connected_ids.contains("gmail"));
    assert!(!connected_ids.contains("google_workspace"));
    assert_eq!(
        value["connected_agentark_surfaces"]["total"]
            .as_u64()
            .unwrap(),
        0
    );
}

#[tokio::test]
async fn integration_catalog_tools_register_as_read_only_inventory_actions() {
    let temp = tempfile::tempdir().unwrap();
    let runtime = ActionRuntime::new(temp.path(), temp.path()).await.unwrap();
    runtime.load_builtin_actions().await.unwrap();

    for action_name in [
        "integration_catalog_list",
        "integration_catalog_describe",
        "integration_catalog_status",
    ] {
        let action = action_def_by_name(&runtime, action_name).await;
        let metadata = action.action_metadata();
        assert_eq!(metadata.role, crate::actions::ActionRole::Inspection);
        assert_eq!(
            metadata.integration_class,
            crate::actions::ActionIntegrationClass::Internal
        );
        assert!(
            action
                .capabilities
                .iter()
                .any(|capability| capability == "integration_registry"),
            "{action_name} should be discoverable as an integration registry tool"
        );
    }
}

#[tokio::test]
async fn integration_catalog_list_returns_agent_ready_entries() {
    let temp = tempfile::tempdir().unwrap();
    let manager = crate::core::runtime::config::SecureConfigManager::new(temp.path()).unwrap();
    manager
        .set_custom_secret(
            crate::actions::google_workspace::GOOGLE_WORKSPACE_TOKENS_KEY,
            Some(
                serde_json::json!({
                    "access_token": "access",
                    "refresh_token": "refresh",
                    "expires_at": chrono::Utc::now().timestamp() + 3600,
                    "granted_scopes": [
                        "https://www.googleapis.com/auth/gmail.readonly",
                        "https://www.googleapis.com/auth/gmail.send"
                    ],
                    "granted_bundles": ["gmail"]
                })
                .to_string(),
            ),
        )
        .unwrap();
    manager
        .set_custom_secret(
            crate::actions::google_workspace::GOOGLE_WORKSPACE_BUNDLES_KEY,
            Some(serde_json::json!(["gmail"]).to_string()),
        )
        .unwrap();

    let runtime = ActionRuntime::new(temp.path(), temp.path()).await.unwrap();
    runtime.load_builtin_actions().await.unwrap();
    let output = runtime
        .execute_action("integration_catalog_list", &serde_json::json!({}))
        .await
        .unwrap();
    let value: serde_json::Value = serde_json::from_str(&output).unwrap();
    let entries = value["entries"].as_array().unwrap();
    let gmail = entries
        .iter()
        .find(|entry| entry.get("id").and_then(|id| id.as_str()) == Some("gmail"))
        .expect("catalog should include the connected mail integration");

    assert_eq!(gmail["source_kind"], "native");
    assert_eq!(gmail["auth_mode"], "native");
    assert_eq!(gmail["connected"], true);
    assert_eq!(gmail["enabled"], true);
    assert_eq!(gmail["connection_required"], false);
    assert!(gmail["action_names"]
        .as_array()
        .unwrap()
        .iter()
        .any(|action| action.as_str() == Some("gmail_scan")));
}

#[tokio::test]
async fn integration_catalog_describe_and_status_target_one_entry() {
    let temp = tempfile::tempdir().unwrap();
    let runtime = ActionRuntime::new(temp.path(), temp.path()).await.unwrap();
    runtime.load_builtin_actions().await.unwrap();

    let describe = runtime
        .execute_action(
            "integration_catalog_describe",
            &serde_json::json!({ "id": "media_gen" }),
        )
        .await
        .unwrap();
    let describe_value: serde_json::Value = serde_json::from_str(&describe).unwrap();
    assert_eq!(describe_value["status"], "ok");
    assert_eq!(describe_value["entry"]["id"], "media_gen");
    assert!(describe_value["entry"]["capabilities"].is_array());

    let status = runtime
        .execute_action(
            "integration_catalog_status",
            &serde_json::json!({ "id": "media_gen" }),
        )
        .await
        .unwrap();
    let status_value: serde_json::Value = serde_json::from_str(&status).unwrap();
    assert_eq!(status_value["status"], "ok");
    assert_eq!(status_value["id"], "media_gen");
    assert!(status_value["connected"].is_boolean());
    assert!(status_value["enabled"].is_boolean());
}

#[tokio::test]
async fn inspect_integration_finds_action_backed_surface_by_action_description() {
    let temp = tempfile::tempdir().unwrap();
    let manager = crate::core::runtime::config::SecureConfigManager::new(temp.path()).unwrap();
    manager
        .set_custom_secret(
            crate::actions::google_workspace::GOOGLE_WORKSPACE_TOKENS_KEY,
            Some(
                serde_json::json!({
                    "access_token": "access",
                    "refresh_token": "refresh",
                    "expires_at": chrono::Utc::now().timestamp() + 3600,
                    "granted_scopes": [
                        "https://www.googleapis.com/auth/gmail.readonly",
                        "https://www.googleapis.com/auth/gmail.send"
                    ],
                    "granted_bundles": ["gmail"]
                })
                .to_string(),
            ),
        )
        .unwrap();
    manager
        .set_custom_secret(
            crate::actions::google_workspace::GOOGLE_WORKSPACE_BUNDLES_KEY,
            Some(serde_json::json!(["gmail"]).to_string()),
        )
        .unwrap();

    let runtime = ActionRuntime::new(temp.path(), temp.path()).await.unwrap();
    runtime.load_builtin_actions().await.unwrap();
    let output = runtime
        .execute_inspect_integration(&serde_json::json!({
            "query": "inbox messages",
            "run_check": true
        }))
        .await
        .unwrap();
    let value: serde_json::Value = serde_json::from_str(&output).unwrap();
    let matches = value["matches"].as_array().unwrap();
    let gmail = matches
        .iter()
        .find(|item| item["record"].get("id").and_then(|id| id.as_str()) == Some("gmail"))
        .expect("inspect should find Gmail from enabled action metadata");

    assert_eq!(gmail["safe_check"]["ready_for_agent"], true);
    assert!(gmail["safe_check"]["available_actions"]
        .as_array()
        .unwrap()
        .iter()
        .any(|action| action.as_str() == Some("gmail_scan")));
}

#[tokio::test]
async fn capability_lookup_marks_connected_workspace_actions_ready_now() {
    let temp = tempfile::tempdir().unwrap();
    let manager = crate::core::runtime::config::SecureConfigManager::new(temp.path()).unwrap();
    manager
        .set_custom_secret(
            crate::actions::google_workspace::GOOGLE_WORKSPACE_TOKENS_KEY,
            Some(
                serde_json::json!({
                    "access_token": "access",
                    "refresh_token": "refresh",
                    "expires_at": chrono::Utc::now().timestamp() + 3600,
                    "granted_scopes": [
                        "https://www.googleapis.com/auth/gmail.readonly",
                        "https://www.googleapis.com/auth/gmail.send"
                    ],
                    "granted_bundles": ["gmail"]
                })
                .to_string(),
            ),
        )
        .unwrap();
    manager
        .set_custom_secret(
            crate::actions::google_workspace::GOOGLE_WORKSPACE_BUNDLES_KEY,
            Some(serde_json::json!(["gmail"]).to_string()),
        )
        .unwrap();

    let runtime = ActionRuntime::new(temp.path(), temp.path()).await.unwrap();
    runtime.load_builtin_actions().await.unwrap();
    let output = runtime
        .execute_agentark_capability_lookup(&serde_json::json!({
            "query": "read inbox messages",
            "limit": 4
        }))
        .await
        .unwrap();
    let value: serde_json::Value = serde_json::from_str(&output).unwrap();
    let results = value["results"].as_array().unwrap();
    let availability_index = results
        .iter()
        .position(|item| {
            item.get("result_type").and_then(|value| value.as_str())
                == Some("live_availability_summary")
        })
        .expect("capability lookup should summarize live ready actions");
    let live_action_index = results
        .iter()
        .position(|item| {
            item.get("result_type").and_then(|value| value.as_str()) == Some("live_action")
                && item.get("action_name").and_then(|value| value.as_str()) == Some("gmail_scan")
        })
        .expect("gmail_scan should be returned as a live action match");
    assert!(availability_index < live_action_index);

    let availability = &results[availability_index];
    assert_eq!(availability["ready_for_agent"], true);
    assert_eq!(
        availability["credential_state_scope"],
        "enabled_runtime_actions"
    );
    let matched_actions = availability["matched_actions"].as_array().unwrap();
    let gmail_match = matched_actions
        .iter()
        .find(|item| item.get("action_name").and_then(|value| value.as_str()) == Some("gmail_scan"))
        .expect("gmail_scan should be marked ready in the live summary");
    assert_eq!(gmail_match["ready_for_agent"], true);
    assert_eq!(gmail_match["credential_state"], "auth_config_satisfied");

    let live_action = &results[live_action_index];
    assert_eq!(live_action["availability"]["ready_for_agent"], true);
    assert_eq!(
        live_action["availability"]["credential_state"],
        "auth_config_satisfied"
    );
    assert_eq!(live_action["authorization"]["requires_auth"], true);
    assert!(live_action["content"]
        .as_str()
        .unwrap()
        .contains("ready_for_agent_now"));
}

#[tokio::test]
async fn list_enabled_actions_exposes_system_workspace_tools_for_execution_time_errors() {
    let temp = tempfile::tempdir().unwrap();
    let runtime = ActionRuntime::new(temp.path(), temp.path()).await.unwrap();
    runtime.load_builtin_actions().await.unwrap();
    let enabled = runtime.list_enabled_actions().await.unwrap();

    assert!(enabled.iter().any(|action| action.name == "gmail_scan"));
    assert!(enabled
        .iter()
        .any(|action| action.name == "calendar_create"));
    assert!(enabled
        .iter()
        .any(|action| action.name == "google_drive_search"));
    assert!(enabled
        .iter()
        .any(|action| action.name == "google_docs_read"));
    assert!(enabled
        .iter()
        .any(|action| action.name == "google_sheets_read"));
    assert!(enabled
        .iter()
        .any(|action| action.name == "google_chat_list_spaces"));
    assert!(enabled
        .iter()
        .any(|action| action.name == "google_admin_list_users"));
    assert!(enabled
        .iter()
        .any(|action| action.name == "google_workspace_gws_command"));
}

#[tokio::test]
async fn list_enabled_actions_exposes_workspace_system_tools_without_bundle_filtering() {
    let temp = tempfile::tempdir().unwrap();
    let manager = crate::core::runtime::config::SecureConfigManager::new(temp.path()).unwrap();
    manager
        .set_custom_secret(
            crate::actions::google_workspace::GOOGLE_WORKSPACE_TOKENS_KEY,
            Some(
                serde_json::json!({
                    "access_token": "access",
                    "refresh_token": "refresh",
                    "expires_at": chrono::Utc::now().timestamp() + 3600,
                    "granted_scopes": [
                        "https://www.googleapis.com/auth/gmail.readonly",
                        "https://www.googleapis.com/auth/gmail.send"
                    ],
                    "granted_bundles": ["gmail"]
                })
                .to_string(),
            ),
        )
        .unwrap();
    manager
        .set_custom_secret(
            crate::actions::google_workspace::GOOGLE_WORKSPACE_BUNDLES_KEY,
            Some(serde_json::json!(["gmail"]).to_string()),
        )
        .unwrap();

    let runtime = ActionRuntime::new(temp.path(), temp.path()).await.unwrap();
    runtime.load_builtin_actions().await.unwrap();
    let enabled = runtime.list_enabled_actions().await.unwrap();

    assert!(enabled.iter().any(|action| action.name == "gmail_scan"));
    assert!(enabled.iter().any(|action| action.name == "gmail_reply"));
    assert!(enabled.iter().any(|action| action.name == "calendar_today"));
    assert!(enabled
        .iter()
        .any(|action| action.name == "calendar_create"));
    assert!(enabled
        .iter()
        .any(|action| action.name == "google_drive_search"));
    assert!(enabled
        .iter()
        .any(|action| action.name == "google_docs_read"));
    assert!(enabled
        .iter()
        .any(|action| action.name == "google_sheets_read"));
    assert!(enabled
        .iter()
        .any(|action| action.name == "google_chat_list_spaces"));
    assert!(enabled
        .iter()
        .any(|action| action.name == "google_admin_list_users"));
}

#[tokio::test]
async fn list_enabled_actions_exposes_all_workspace_system_tools_with_partial_grants() {
    let temp = tempfile::tempdir().unwrap();
    let manager = crate::core::runtime::config::SecureConfigManager::new(temp.path()).unwrap();
    manager
        .set_custom_secret(
            crate::actions::google_workspace::GOOGLE_WORKSPACE_TOKENS_KEY,
            Some(
                serde_json::json!({
                    "access_token": "access",
                    "refresh_token": "refresh",
                    "expires_at": chrono::Utc::now().timestamp() + 3600,
                    "granted_scopes": [
                        "https://www.googleapis.com/auth/gmail.readonly",
                        "https://www.googleapis.com/auth/gmail.send",
                        "https://www.googleapis.com/auth/drive.metadata.readonly"
                    ],
                    "granted_bundles": ["gmail", "drive"]
                })
                .to_string(),
            ),
        )
        .unwrap();
    manager
        .set_custom_secret(
            crate::actions::google_workspace::GOOGLE_WORKSPACE_BUNDLES_KEY,
            Some(serde_json::json!(["gmail", "drive"]).to_string()),
        )
        .unwrap();

    let runtime = ActionRuntime::new(temp.path(), temp.path()).await.unwrap();
    runtime.load_builtin_actions().await.unwrap();
    let enabled = runtime.list_enabled_actions().await.unwrap();

    assert!(enabled.iter().any(|action| action.name == "gmail_scan"));
    assert!(enabled
        .iter()
        .any(|action| action.name == "google_drive_search"));
    assert!(enabled.iter().any(|action| action.name == "calendar_today"));
    assert!(enabled
        .iter()
        .any(|action| action.name == "google_docs_read"));
    assert!(enabled
        .iter()
        .any(|action| action.name == "google_sheets_read"));
    assert!(enabled
        .iter()
        .any(|action| action.name == "google_chat_list_spaces"));
    assert!(enabled
        .iter()
        .any(|action| action.name == "google_admin_list_users"));
}

#[tokio::test]
async fn list_enabled_actions_exposes_unconfigured_system_connectors_for_execution_time_errors() {
    let temp = tempfile::tempdir().unwrap();
    let runtime = ActionRuntime::new(temp.path(), temp.path()).await.unwrap();
    runtime.load_builtin_actions().await.unwrap();
    let enabled = runtime.list_enabled_actions().await.unwrap();

    assert!(enabled.iter().any(|action| action.name == "places"));
    assert!(enabled.iter().any(|action| action.name == "twilio"));
    assert!(enabled.iter().any(|action| action.name == "github"));
    assert!(enabled.iter().any(|action| action.name == "moltbook"));
    let places_error = runtime
        .execute_action(
            "places",
            &serde_json::json!({ "action": "search", "query": "coffee" }),
        )
        .await
        .unwrap_err();
    let places_action_error = places_error
        .downcast_ref::<crate::actions::ActionError>()
        .expect("unconfigured connector should return a typed action error");
    assert_eq!(
        places_action_error.domain(),
        crate::actions::ActionErrorDomain::Integration
    );
    assert_eq!(
        places_action_error.reason(),
        crate::actions::ActionErrorReason::NotConnected
    );
    let status = runtime
        .execute_action("moltbook", &serde_json::json!({ "action": "status" }))
        .await
        .unwrap();
    assert!(status.contains("not_configured"));
}

#[tokio::test]
async fn list_enabled_actions_exposes_ready_external_connector_tools() {
    let temp = tempfile::tempdir().unwrap();
    let manager = crate::core::runtime::config::SecureConfigManager::new(temp.path()).unwrap();
    manager
        .set_custom_secret("google_places_api_key", Some("test-key".to_string()))
        .unwrap();

    let runtime = ActionRuntime::new(temp.path(), temp.path()).await.unwrap();
    runtime.load_builtin_actions().await.unwrap();
    let enabled = runtime.list_enabled_actions().await.unwrap();

    assert!(enabled.iter().any(|action| action.name == "places"));
}

#[test]
fn loopback_http_get_rejects_non_app_paths() {
    let url = reqwest::Url::parse("http://127.0.0.1:8990/api/secret").unwrap();
    let err = ActionRuntime::loopback_http_get_allowed(&url).unwrap_err();
    assert!(err.to_string().contains("/apps/"));
}

#[test]
fn loopback_http_get_allows_local_app_paths() {
    let url = reqwest::Url::parse("http://localhost:8990/apps/demo/health").unwrap();
    assert!(ActionRuntime::loopback_http_get_allowed(&url).is_ok());
}

#[tokio::test]
async fn connector_request_rejects_local_and_private_targets() {
    let temp = tempfile::tempdir().unwrap();
    let runtime = ActionRuntime::new(temp.path(), temp.path()).await.unwrap();

    let localhost = runtime
        .validate_connector_request_url("http://127.0.0.1:8990/health")
        .await
        .unwrap_err();
    assert!(
        localhost.to_string().contains("localhost") || localhost.to_string().contains("loopback")
    );

    let private_ip = runtime
        .validate_connector_request_url("http://10.0.0.8/internal")
        .await
        .unwrap_err();
    assert!(private_ip.to_string().contains("private"));
}

#[tokio::test]
async fn http_get_private_targets_require_trusted_direct_user_intent() {
    let temp = tempfile::tempdir().unwrap();
    let runtime = ActionRuntime::new(temp.path(), temp.path()).await.unwrap();
    let private_url = "http://192.168.29.61:8080/snapshot";

    let blocked = runtime
        .resolve_http_get_url_for_context(private_url, &ActionAuthorizationContext::default())
        .await
        .unwrap_err();
    assert!(blocked.to_string().contains("private"));

    let trusted_direct_context = ActionAuthorizationContext {
        principal: Some(ActionCallerPrincipal::local_admin("session")),
        surface: ActionExecutionSurface::Chat,
        direct_user_intent: true,
        ..ActionAuthorizationContext::default()
    };
    let allowed = runtime
        .resolve_http_get_url_for_context(private_url, &trusted_direct_context)
        .await
        .unwrap();
    assert_eq!(allowed.as_str(), private_url);
}

#[test]
fn docker_unavailable_error_fails_closed() {
    let shell_error = ActionRuntime::docker_required_error("shell").to_string();
    let code_error = ActionRuntime::docker_required_error("code_execute").to_string();

    assert!(shell_error.contains("Docker is required"));
    assert!(shell_error.contains("shell"));
    assert!(code_error.contains("code_execute"));
}

#[test]
fn native_env_overrides_block_runtime_control_keys() {
    let args = serde_json::json!({
        "env": {
            "PATH": "/tmp/bin"
        }
    });
    let err = ActionRuntime::collect_native_env_overrides(&args).unwrap_err();
    assert!(err.to_string().contains("not allowed"));
}

#[test]
fn workspace_root_defaults_to_data_owned_workspace() {
    let data_dir = Path::new("/tmp/agentark-data");
    assert_eq!(
        ActionRuntime::workspace_root_from_config(data_dir, None),
        data_dir.join("workspace")
    );
    assert_eq!(
        ActionRuntime::workspace_root_from_config(data_dir, Some("custom-work")),
        data_dir.join("custom-work")
    );
}

#[test]
fn workspace_root_ignores_configured_source_checkout() {
    let temp = tempfile::tempdir().unwrap();
    let data_dir = temp.path().join("data");
    let source = temp.path().join("agentark");
    std::fs::create_dir_all(source.join("src")).unwrap();
    std::fs::write(source.join("Cargo.toml"), "[workspace]\n").unwrap();

    assert_eq!(
        ActionRuntime::workspace_root_from_config(&data_dir, source.to_str()),
        data_dir.join("workspace")
    );
}

#[tokio::test]
async fn file_tools_do_not_fall_back_to_current_workspace_checkout() {
    let temp = tempfile::tempdir().unwrap();
    let config_dir = temp.path().join("config");
    let data_dir = temp.path().join("data");
    let checkout = temp.path().join("agentark");
    std::fs::create_dir_all(checkout.join("src")).unwrap();
    std::fs::write(checkout.join("Cargo.toml"), "[workspace]\n").unwrap();
    let target = checkout.join("generated-app").join("index.html");

    let runtime = ActionRuntime::new(&config_dir, &data_dir).await.unwrap();
    let error = runtime
        .resolve_tool_write_path(target.to_str().unwrap())
        .expect_err("file writes must not be allowed into the current/source checkout");
    assert!(error.to_string().contains("outside allowed roots"));
}

#[test]
fn workspace_alias_paths_remap_to_workspace_root() {
    let runtime = ActionRuntime {
        config: RuntimeConfig::default(),
        transactions: tokio::sync::Mutex::new(TransactionManager::new(PathBuf::from("snapshots"))),
        actions: tokio::sync::RwLock::new(HashMap::new()),
        disabled_actions: tokio::sync::RwLock::new(HashSet::new()),
        disabled_actions_file: PathBuf::from("./disabled_actions.json"),
        action_reviews: tokio::sync::RwLock::new(HashMap::new()),
        action_reviews_file: PathBuf::from("./action_reviews.json"),
        capability_run_contexts: tokio::sync::RwLock::new(HashMap::new()),
        removed_bundled_actions: tokio::sync::RwLock::new(HashSet::new()),
        removed_bundled_actions_file: PathBuf::from("./removed_bundled_actions.json"),
        actions_dir: PathBuf::from("./skills"),
        cli_skills_dir: PathBuf::from("./cli_skills"),
        config_dir: PathBuf::from("."),
        auto_approved_actions: std::sync::RwLock::new(HashSet::new()),
        tool_args_guard_config: std::sync::RwLock::new(Default::default()),
        task_queue: None,
        action_guard: None,
        safety_engine: None,
        storage: None,
        embedding_client: None,
        current_user_id: None,
        mcp_registry: None,
        plugin_registry: None,
        extension_pack_registry: None,
        #[cfg(feature = "docker")]
        active_sandbox_containers: tokio::sync::RwLock::new(HashSet::new()),
        #[cfg(feature = "docker")]
        container_reaper_status: tokio::sync::RwLock::new(ContainerReaperStatus::default()),
    };
    let workspace_root = runtime.workspace_root();
    assert_eq!(
        runtime
            .absolutize_tool_path("/workspace/demo/index.html")
            .unwrap(),
        workspace_root.join("demo").join("index.html")
    );
    assert_eq!(
        runtime.absolutize_tool_path("demo/index.html").unwrap(),
        workspace_root.join("demo").join("index.html")
    );
}

#[test]
fn find_project_root_from_path_walks_up_to_cargo_toml() {
    let temp = tempfile::tempdir().unwrap();
    let root = temp.path().join("repo");
    let nested = root.join("target").join("release");
    std::fs::create_dir_all(&nested).unwrap();
    std::fs::write(
        root.join("Cargo.toml"),
        "[package]\nname = \"demo\"\nversion = \"0.1.0\"\n",
    )
    .unwrap();
    let exe_path = nested.join(if cfg!(windows) {
        "agentark.exe"
    } else {
        "agentark"
    });
    std::fs::write(&exe_path, "").unwrap();

    let detected = ActionRuntime::find_project_root_from_path(&exe_path)
        .expect("project root should be detected");
    assert_eq!(detected, root);
}

#[test]
fn runtime_owned_bundled_dirs_are_disabled() {
    assert!(!ActionRuntime::is_runtime_owned_bundled_dir(Path::new(
        "/app/repo-skills"
    )));
}

#[test]
fn private_ips_are_not_treated_as_public() {
    assert!(!ActionRuntime::ip_is_public(IpAddr::V4(Ipv4Addr::new(
        127, 0, 0, 1
    ))));
    assert!(!ActionRuntime::ip_is_public(IpAddr::V4(Ipv4Addr::new(
        169, 254, 1, 10
    ))));
    assert!(ActionRuntime::ip_is_public(IpAddr::V4(Ipv4Addr::new(
        1, 1, 1, 1
    ))));
}

#[tokio::test]
async fn install_cli_skill_action_persists_and_reloads() {
    let temp = tempfile::tempdir().unwrap();
    let runtime = ActionRuntime::new(temp.path(), temp.path()).await.unwrap();
    let skill_markdown = r#"---
name: officecli
description: Office CLI
version: "1.2.3"
---
# officecli
"#;
    let manifest = InstalledCliSkillManifest {
        name: "officecli".to_string(),
        description: "Office CLI".to_string(),
        version: "1.2.3".to_string(),
        executable_path: temp.path().join("officecli").display().to_string(),
        verify_args: vec!["--version".to_string()],
        source_url: Some("https://officecli.ai/SKILL.md".to_string()),
    };

    runtime
        .install_cli_skill_action(manifest.clone(), skill_markdown)
        .await
        .unwrap();

    let actions = runtime.list_actions().await.unwrap();
    assert!(actions.iter().any(|action| action.name == "officecli"));

    let reloaded = ActionRuntime::new(temp.path(), temp.path()).await.unwrap();
    reloaded.load_all_actions().await.unwrap();
    let reloaded_actions = reloaded.list_actions().await.unwrap();
    assert!(reloaded_actions
        .iter()
        .any(|action| action.name == "officecli"));
}

#[tokio::test]
async fn startup_cli_skill_load_uses_deterministic_review_without_semantic_model() {
    let config_dir = tempfile::tempdir().unwrap();
    let data_dir = tempfile::tempdir().unwrap();
    let executable = data_dir.path().join("generic-cli");
    std::fs::write(&executable, "").unwrap();
    let skill_dir = data_dir.path().join("cli_skills").join("generic-cli");
    std::fs::create_dir_all(&skill_dir).unwrap();
    std::fs::write(
        skill_dir.join("manifest.json"),
        serde_json::to_vec_pretty(&InstalledCliSkillManifest {
            name: "generic-cli".to_string(),
            description: "Generic CLI".to_string(),
            version: "1.0.0".to_string(),
            executable_path: executable.display().to_string(),
            verify_args: vec!["--version".to_string()],
            source_url: None,
        })
        .unwrap(),
    )
    .unwrap();
    std::fs::write(
        skill_dir.join("SKILL.md"),
        "---\nname: generic-cli\ndescription: Generic CLI\n---\n# Generic CLI\n",
    )
    .unwrap();

    let mut runtime = ActionRuntime::new(config_dir.path(), data_dir.path())
        .await
        .unwrap();
    let guard = crate::security::ActionGuard::new(
        &ed25519_dalek::SigningKey::from_bytes(&[8u8; 32]),
        "did:key:test",
        config_dir.path(),
        data_dir.path(),
    )
    .await
    .unwrap();
    runtime.set_action_guard(std::sync::Arc::new(guard));

    runtime.load_all_actions().await.unwrap();
    let review = runtime
        .get_action_review("generic-cli")
        .await
        .expect("startup-loaded CLI skill should have a review");

    assert!(review.allow_load);
    assert_ne!(review.status, ActionReviewStatus::Blocked);
    assert!(!review
        .warnings
        .iter()
        .chain(review.notes.iter())
        .any(|item| item.contains("semantic-review-unavailable")
            || item.contains("configured model")));
}

#[tokio::test]
async fn structured_custom_api_review_does_not_require_action_guard() {
    let temp = tempfile::tempdir().unwrap();
    let runtime = ActionRuntime::new(temp.path(), temp.path()).await.unwrap();
    let info = ActionDef {
        name: "generic_custom_api_action".to_string(),
        description: "Call a saved custom API operation.".to_string(),
        input_schema: serde_json::json!({ "type": "object" }),
        capabilities: vec!["network".to_string()],
        source: ActionSource::Custom,
        ..ActionDef::default()
    };
    let binding = CustomApiBinding {
        api_id: "api-generic".to_string(),
        api_name: "Generic API".to_string(),
        operation_id: "op-generic".to_string(),
        operation_name: "Generic operation".to_string(),
        method: "GET".to_string(),
        base_url: "https://api.example.com".to_string(),
        path: "/items".to_string(),
        read_only: true,
        secret_key: String::new(),
        auth_profile_id: None,
        auth_mode: crate::custom_apis::CustomApiAuthMode::None,
        auth_header: None,
        auth_name: None,
        auth_username: None,
        default_headers: BTreeMap::new(),
        default_query: BTreeMap::new(),
        parameters: Vec::new(),
        body_required: false,
        default_body: None,
    };

    let review = runtime
        .review_custom_api_action(&info, &binding)
        .await
        .unwrap();

    assert!(review.allow_load);
    assert_ne!(review.status, ActionReviewStatus::Blocked);
    assert_ne!(
        review.blocked_reason.as_deref(),
        Some("Action security is unavailable, so custom API actions are not loadable.")
    );
}

#[tokio::test]
async fn cli_skill_action_executes_bound_command() {
    let temp = tempfile::tempdir().unwrap();
    let runtime = ActionRuntime::new(temp.path(), temp.path()).await.unwrap();
    let manifest = InstalledCliSkillManifest {
        name: "echo-cli".to_string(),
        description: "Echo CLI".to_string(),
        version: "1.0.0".to_string(),
        executable_path: if cfg!(windows) {
            std::env::var("ComSpec")
                .or_else(|_| std::env::var("COMSPEC"))
                .unwrap_or_else(|_| "C:\\WINDOWS\\system32\\cmd.exe".to_string())
        } else {
            "sh".to_string()
        },
        verify_args: vec![],
        source_url: None,
    };

    runtime
        .install_cli_skill_action(
            manifest.clone(),
            "---\nname: echo-cli\ndescription: Echo CLI\n---\n# echo-cli\n",
        )
        .await
        .unwrap();

    let args = if cfg!(windows) {
        serde_json::json!({ "args": ["/C", "echo", "ready"] })
    } else {
        serde_json::json!({ "args": ["-lc", "printf ready"] })
    };
    let output = runtime
        .execute_cli_action(
            "echo-cli",
            CliToolBinding {
                executable_path: manifest.executable_path.clone(),
                verify_args: manifest.verify_args.clone(),
                auth_profile_id: None,
                auth_env_exports: BTreeMap::new(),
            },
            &args,
        )
        .await
        .unwrap();
    assert!(output.contains("ready"));
}

#[tokio::test]
async fn mcp_server_manage_saves_http_config_without_plain_secret() {
    let temp = tempfile::tempdir().unwrap();
    let runtime = ActionRuntime::new(temp.path(), temp.path()).await.unwrap();
    let output = runtime
        .execute_mcp_server_manage(&serde_json::json!({
            "operation": "create",
            "name": "Example MCP",
            "url": "https://mcp.example.com/mcp",
            "auth_type": "bearer"
        }))
        .await
        .unwrap();
    // The REAL emitter output must survive the legacy wrapper path with its
    // needs_credentials status intact (round-3 regression: tool-less raw JSON
    // was rewrapped as "completed", silently dropping the credential prompt).
    let payload = crate::runtime::ActionRuntime::tool_payload_from_legacy_output(
        "mcp_server_manage",
        output.clone(),
    );
    let rendered = crate::runtime::ActionRuntime::render_tool_payload_for_legacy(
        "mcp_server_manage",
        payload,
    );
    let parsed = parse_tool_completion_output(&rendered);
    assert_eq!(parsed["status"], "needs_credentials");
    assert_eq!(parsed["tool"], "mcp_server_manage");
    assert_eq!(parsed["data"]["server_id"], "example-mcp");
    assert_eq!(
        parsed["data"]["credential_request"]["kind"],
        serde_json::json!("mcp_server_auth")
    );
    assert_eq!(
        parsed["data"]["credential_request"]["auth_type"],
        serde_json::json!("bearer")
    );

    let manager = crate::core::runtime::config::SecureConfigManager::new_with_data_dir(
        temp.path(),
        Some(temp.path()),
    )
    .unwrap();
    let saved = manager.load().unwrap();
    assert_eq!(saved.mcp.servers.len(), 1);
    assert_eq!(saved.mcp.servers[0].name, "Example MCP");
    match &saved.mcp.servers[0].transport {
        crate::core::runtime::config::McpTransportConfig::Http { url } => {
            assert_eq!(url, "https://mcp.example.com/mcp");
        }
        other => panic!("unexpected transport: {other:?}"),
    }
    let secrets = manager.load_secrets().unwrap();
    assert!(secrets
        .mcp_auth
        .get("example-mcp")
        .and_then(|secret| secret.token.as_ref())
        .is_none());
}

#[tokio::test]
async fn mcp_server_manage_accepts_install_as_create_alias() {
    let temp = tempfile::tempdir().unwrap();
    let runtime = ActionRuntime::new(temp.path(), temp.path()).await.unwrap();
    let output = runtime
        .execute_mcp_server_manage(&serde_json::json!({
            "operation": "install",
            "name": "Example MCP",
            "url": "https://mcp.example.com/mcp",
            "auth_type": "bearer"
        }))
        .await
        .unwrap();
    let parsed = parse_tool_completion_output(&output);
    assert_eq!(parsed["data"]["operation"], "create");
    assert_eq!(parsed["data"]["server_id"], "example-mcp");
}


#[test]
fn integration_matcher_resolves_human_phrasing_to_slug_identity() {
    // Round-4 regression: inspect and execution must resolve the SAME record
    // from the same human phrasing (dual-resolver divergence produced false
    // "not installed" verdicts for installed integrations).
    let record = serde_json::json!({
        "id": "linear-graphql-api",
        "name": "Linear GraphQL API",
        "base_url": "https://api.example.com/graphql",
        "status": "connected"
    });
    let terms = |query: &str| ActionRuntime::integration_inspect_terms(Some(query));

    for id in ["linear-graphql-api", "linear", "Linear GraphQL API"] {
        assert!(
            ActionRuntime::integration_value_matches(&record, Some(id), &[]),
            "id selector {id:?} must resolve the record"
        );
    }
    for query in ["linear", "Linear GraphQL", "Linear GraphQL API", "linear integration"] {
        assert!(
            ActionRuntime::integration_value_matches(&record, None, &terms(query)),
            "query {query:?} must resolve the record"
        );
    }
    assert!(
        !ActionRuntime::integration_value_matches(&record, Some("notion"), &[]),
        "unrelated id must not match"
    );
    assert!(
        !ActionRuntime::integration_value_matches(&record, None, &terms("notion workspace")),
        "unrelated query must not match"
    );
}

#[tokio::test]
async fn inspect_no_match_reports_query_miss_not_absence() {
    // An inspect that searched real records but matched none must NOT emit an
    // absence-shaped result: completion verifiers read bare not_found as
    // proof the integration does not exist and force re-verification loops.
    let temp = tempfile::tempdir().unwrap();
    let runtime = ActionRuntime::new(temp.path(), temp.path()).await.unwrap();
    let output = runtime
        .execute_inspect_integration(&serde_json::json!({
            "query": "completely-unrelated-integration-zzz"
        }))
        .await
        .unwrap();
    let parsed: serde_json::Value = serde_json::from_str(&output).unwrap();
    let records_searched = parsed["records_searched"].as_u64().unwrap_or(0);
    if records_searched > 0 {
        assert_eq!(
            parsed["status"],
            serde_json::json!("no_match_for_query"),
            "a query miss over real records must not read as absence"
        );
        assert!(
            parsed["guidance"].as_str().unwrap_or_default().contains("not evidence"),
            "guidance must teach that a miss is not absence"
        );
    } else {
        // Fixture has no registered integrations at all; absence is honest,
        // but surfaces_searched must still prove what was examined.
        assert_eq!(parsed["status"], serde_json::json!("not_found"));
        assert!(parsed["surfaces_searched"].is_array());
    }
}
