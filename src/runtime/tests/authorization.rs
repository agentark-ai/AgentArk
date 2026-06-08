use super::*;

#[tokio::test]
async fn trusted_chat_allows_high_risk_builtin_actions_without_approval() {
    let runtime = runtime_for_authorization_tests().await;
    let action = action_def_by_name(&runtime, "code_execute").await;
    let decision = runtime
        .authorize_action_invocation(
            "code_execute",
            Some(&action),
            &serde_json::json!({}),
            &ActionAuthorizationContext {
                principal: Some(ActionCallerPrincipal::local_admin("test")),
                surface: ActionExecutionSurface::Chat,
                direct_user_intent: true,
                current_turn_is_explicit_approval: false,
                agent_name: None,
                agent_access_scope: None,
                capability_context_id: None,
                ..ActionAuthorizationContext::default()
            },
        )
        .await
        .unwrap();

    assert!(decision.allowed);
    assert!(!decision.requires_explicit_approval);
}

#[tokio::test]
async fn lan_discover_allows_trusted_chat_without_approval() {
    let runtime = runtime_for_authorization_tests().await;
    let action = action_def_by_name(&runtime, "lan_discover").await;
    assert!(action
        .capabilities
        .iter()
        .any(|cap| cap == "local_network_discovery"));

    let decision = runtime
        .authorize_action_invocation(
            "lan_discover",
            Some(&action),
            &serde_json::json!({ "target": "sonos" }),
            &ActionAuthorizationContext {
                principal: Some(ActionCallerPrincipal::local_admin("test")),
                surface: ActionExecutionSurface::Chat,
                direct_user_intent: true,
                current_turn_is_explicit_approval: false,
                agent_name: None,
                agent_access_scope: None,
                capability_context_id: None,
                ..ActionAuthorizationContext::default()
            },
        )
        .await
        .unwrap();

    assert!(decision.allowed);
    assert!(!decision.requires_explicit_approval);
}

#[tokio::test]
async fn custom_permission_allows_trusted_chat_without_inline_approval() {
    let runtime = runtime_for_authorization_tests().await;
    let action = action_def_by_name(&runtime, "google_workspace_gws_command").await;
    let decision = runtime
        .authorize_action_invocation(
            "google_workspace_gws_command",
            Some(&action),
            &serde_json::json!({ "argv": ["about"] }),
            &ActionAuthorizationContext {
                principal: Some(ActionCallerPrincipal::local_admin("test")),
                surface: ActionExecutionSurface::Chat,
                direct_user_intent: true,
                current_turn_is_explicit_approval: false,
                agent_name: None,
                agent_access_scope: None,
                capability_context_id: None,
                ..ActionAuthorizationContext::default()
            },
        )
        .await
        .unwrap();

    assert!(decision.allowed);
    assert!(!decision.requires_explicit_approval);
    let redacted = crate::security::redact_secret_input(&decision.reason).text;
    assert!(!redacted.contains("[REDACTED_SECRET]"));
}

#[tokio::test]
async fn lan_discover_allows_turn_with_legacy_approval_marker() {
    let runtime = runtime_for_authorization_tests().await;
    let action = action_def_by_name(&runtime, "lan_discover").await;
    let decision = runtime
        .authorize_action_invocation(
            "lan_discover",
            Some(&action),
            &serde_json::json!({ "target": "sonos" }),
            &ActionAuthorizationContext {
                principal: Some(ActionCallerPrincipal::local_admin("test")),
                surface: ActionExecutionSurface::Chat,
                direct_user_intent: true,
                current_turn_is_explicit_approval: true,
                agent_name: None,
                agent_access_scope: None,
                capability_context_id: None,
                ..ActionAuthorizationContext::default()
            },
        )
        .await
        .unwrap();

    assert!(decision.allowed);
}

#[tokio::test]
async fn trusted_api_allows_high_risk_builtin_actions_without_approval() {
    let runtime = runtime_for_authorization_tests().await;
    let action = action_def_by_name(&runtime, "shell").await;
    let decision = runtime
        .authorize_action_invocation(
            "shell",
            Some(&action),
            &serde_json::json!({ "command": "pwd" }),
            &ActionAuthorizationContext {
                principal: Some(ActionCallerPrincipal::local_admin("test")),
                surface: ActionExecutionSurface::Api,
                direct_user_intent: true,
                current_turn_is_explicit_approval: false,
                agent_name: None,
                agent_access_scope: None,
                capability_context_id: None,
                ..ActionAuthorizationContext::default()
            },
        )
        .await
        .unwrap();

    assert!(decision.allowed);
    assert!(!decision.requires_explicit_approval);
}

#[tokio::test]
async fn background_blocks_high_risk_builtin_actions() {
    let runtime = runtime_for_authorization_tests().await;
    let action = action_def_by_name(&runtime, "app_deploy").await;
    let decision = runtime
        .authorize_action_invocation(
            "app_deploy",
            Some(&action),
            &serde_json::json!({}),
            &ActionAuthorizationContext {
                principal: None,
                surface: ActionExecutionSurface::Background,
                direct_user_intent: false,
                current_turn_is_explicit_approval: false,
                agent_name: None,
                agent_access_scope: None,
                capability_context_id: None,
                ..ActionAuthorizationContext::default()
            },
        )
        .await
        .unwrap();

    assert!(!decision.allowed);
    assert!(decision.reason.contains("background or automation"));
}

#[tokio::test]
async fn read_only_background_actions_still_work() {
    let runtime = runtime_for_authorization_tests().await;
    let action = action_def_by_name(&runtime, "file_read").await;
    let decision = runtime
        .authorize_action_invocation(
            "file_read",
            Some(&action),
            &serde_json::json!({ "path": "README.md" }),
            &ActionAuthorizationContext {
                principal: None,
                surface: ActionExecutionSurface::Background,
                direct_user_intent: false,
                current_turn_is_explicit_approval: false,
                agent_name: None,
                agent_access_scope: None,
                capability_context_id: None,
                ..ActionAuthorizationContext::default()
            },
        )
        .await
        .unwrap();

    assert!(decision.allowed);
}

#[tokio::test]
async fn api_without_principal_blocks_high_risk_actions() {
    let runtime = runtime_for_authorization_tests().await;
    let action = action_def_by_name(&runtime, "shell").await;
    let decision = runtime
        .authorize_action_invocation(
            "shell",
            Some(&action),
            &serde_json::json!({ "command": "pwd" }),
            &ActionAuthorizationContext {
                principal: None,
                surface: ActionExecutionSurface::Api,
                direct_user_intent: true,
                current_turn_is_explicit_approval: false,
                agent_name: None,
                agent_access_scope: None,
                capability_context_id: None,
                ..ActionAuthorizationContext::default()
            },
        )
        .await
        .unwrap();

    assert!(!decision.allowed);
    assert!(decision.reason.contains("trusted local session"));
}

#[tokio::test]
async fn trusted_chat_has_no_interactive_permission_gate_for_code_execute() {
    let runtime = runtime_for_permission_gate_tests().await;
    let action = action_def_by_name(&runtime, "code_execute").await;
    let unapproved = runtime
        .unapproved_permissions_for_action(
            &action,
            &serde_json::json!({}),
            &ActionAuthorizationContext {
                principal: Some(ActionCallerPrincipal::local_admin("test")),
                surface: ActionExecutionSurface::Chat,
                direct_user_intent: true,
                current_turn_is_explicit_approval: false,
                agent_name: None,
                agent_access_scope: None,
                capability_context_id: None,
                ..ActionAuthorizationContext::default()
            },
        )
        .await;

    assert!(unapproved.is_empty());
}

#[tokio::test]
async fn all_surfaces_bypass_interactive_permission_gate_for_api_probes() {
    let runtime = runtime_for_permission_gate_tests().await;
    let action = ActionDef {
        name: "api__project_tool__post_items".to_string(),
        capabilities: vec![
            "custom_api".to_string(),
            "integration".to_string(),
            "network".to_string(),
            "external_write".to_string(),
        ],
        authorization: ActionAuthorization {
            outbound: crate::actions::ActionEgressPolicy {
                read_only: false,
                outbound_write: true,
                public_publish: false,
            },
            ..ActionAuthorization::default()
        },
        ..ActionDef::default()
    };

    for surface in [
        ActionExecutionSurface::Internal,
        ActionExecutionSurface::Test,
    ] {
        let unapproved = runtime
            .unapproved_permissions_for_action(
                &action,
                &serde_json::json!({}),
                &ActionAuthorizationContext {
                    surface: surface.clone(),
                    ..ActionAuthorizationContext::default()
                },
            )
            .await;
        assert!(
            unapproved.is_empty(),
            "surface {:?} should not require interactive permission approval: {:?}",
            surface,
            unapproved
        );
    }

    let unapproved_api = runtime
        .unapproved_permissions_for_action(
            &action,
            &serde_json::json!({}),
            &ActionAuthorizationContext {
                surface: ActionExecutionSurface::Api,
                ..ActionAuthorizationContext::default()
            },
        )
        .await;
    assert!(
        unapproved_api.is_empty(),
        "API surface should not require interactive permission approval: {:?}",
        unapproved_api
    );
}

#[tokio::test]
async fn runtime_correlation_allows_non_direct_sensitive_read_then_external_send() {
    let runtime = runtime_for_authorization_tests().await;
    let memory = action_def_by_name(&runtime, "memory_lookup").await;
    let schedule = action_def_by_name(&runtime, "schedule_task").await;
    let mut context = trusted_chat_context("test-sensitive-send", false);
    context.direct_user_intent = false;

    let read_decision = runtime
        .authorize_action_invocation(
            "memory_lookup",
            Some(&memory),
            &serde_json::json!({ "query": "saved user context" }),
            &context,
        )
        .await
        .unwrap();
    assert!(read_decision.allowed);

    let send_decision = runtime
        .authorize_action_invocation(
            "schedule_task",
            Some(&schedule),
            &serde_json::json!({
                "task": "Send a summary",
                "at": "2026-04-18T12:00:00+05:30",
                "report_to": "ext.custom.ops"
            }),
            &context,
        )
        .await
        .unwrap();

    assert!(send_decision.allowed);
    assert!(!send_decision.requires_explicit_approval);
}

#[tokio::test]
async fn runtime_correlation_allows_direct_trusted_sensitive_read_then_external_send() {
    let runtime = runtime_for_authorization_tests().await;
    let memory = action_def_by_name(&runtime, "memory_lookup").await;
    let schedule = action_def_by_name(&runtime, "schedule_task").await;
    let context = trusted_chat_context("test-direct-sensitive-send", false);

    assert!(
        runtime
            .authorize_action_invocation(
                "memory_lookup",
                Some(&memory),
                &serde_json::json!({ "query": "saved user context" }),
                &context,
            )
            .await
            .unwrap()
            .allowed
    );

    let send_decision = runtime
        .authorize_action_invocation(
            "schedule_task",
            Some(&schedule),
            &serde_json::json!({
                "task": "Send a summary",
                "at": "2026-04-18T12:00:00+05:30",
                "report_to": "ext.custom.ops"
            }),
            &context,
        )
        .await
        .unwrap();

    assert!(send_decision.allowed);
    assert!(!send_decision.requires_explicit_approval);
}

#[tokio::test]
async fn runtime_correlation_does_not_block_direct_trusted_chat_tools() {
    let runtime = runtime_for_authorization_tests().await;
    let context = trusted_chat_context("test-direct-chat-unknown-risk", false);
    let read_decision = runtime
        .authorize_action_invocation(
            "file_read",
            Some(&crate::actions::ActionDef {
                name: "file_read".to_string(),
                capabilities: vec!["file_read".to_string()],
                ..crate::actions::ActionDef::default()
            }),
            &serde_json::json!({ "path": "README.md" }),
            &context,
        )
        .await
        .unwrap();
    assert!(read_decision.allowed);

    let tool = crate::actions::ActionDef {
        name: "custom_runtime_probe".to_string(),
        capabilities: vec!["unreviewed_host_capability".to_string()],
        ..crate::actions::ActionDef::default()
    };
    let decision = runtime
        .authorize_action_invocation(
            "custom_runtime_probe",
            Some(&tool),
            &serde_json::json!({}),
            &context,
        )
        .await
        .unwrap();

    assert!(decision.allowed);
    assert!(!decision.requires_explicit_approval);
}

#[tokio::test]
async fn runtime_correlation_allows_notify_user_delivery_for_non_direct_runs() {
    let runtime = runtime_for_authorization_tests().await;
    let memory = action_def_by_name(&runtime, "memory_lookup").await;
    let notify = action_def_by_name(&runtime, "notify_user").await;
    let mut context = trusted_chat_context("test-sensitive-notify-send", false);
    context.direct_user_intent = false;

    assert!(notify
        .authorization
        .access
        .channel_targets
        .iter()
        .any(|target| target.argument_key == "delivery_channel"));

    let read_decision = runtime
        .authorize_action_invocation(
            "memory_lookup",
            Some(&memory),
            &serde_json::json!({ "query": "saved user context" }),
            &context,
        )
        .await
        .unwrap();
    assert!(read_decision.allowed);

    let send_decision = runtime
        .authorize_action_invocation(
            "notify_user",
            Some(&notify),
            &serde_json::json!({
                "message": "Send the matched update",
                "delivery_channel": "ext.custom.ops"
            }),
            &context,
        )
        .await
        .unwrap();

    assert!(send_decision.allowed);
    assert!(!send_decision.requires_explicit_approval);
}

#[tokio::test]
async fn runtime_allows_user_originated_background_notify_delivery() {
    let runtime = runtime_for_authorization_tests().await;
    let notify = action_def_by_name(&runtime, "notify_user").await;
    let mut context = trusted_chat_context("test-background-notify-send", false);
    context.surface = ActionExecutionSurface::Background;

    let send_decision = runtime
        .authorize_action_invocation(
            "notify_user",
            Some(&notify),
            &serde_json::json!({
                "message": "Send the matched update",
                "delivery_channel": "telegram"
            }),
            &context,
        )
        .await
        .unwrap();

    assert!(send_decision.allowed);
    assert!(!send_decision.requires_explicit_approval);
}

#[tokio::test]
async fn watch_authorization_allows_direct_trusted_nested_poll_action_and_delivery() {
    let runtime = runtime_for_authorization_tests().await;
    let watch = action_def_by_name(&runtime, "watch").await;
    let context = trusted_chat_context("test-watch-read-send", false);

    let decision = runtime
        .authorize_action_invocation(
            "watch",
            Some(&watch),
            &serde_json::json!({
                "description": "Monitor connected workspace files",
                "poll_action": "gmail_scan",
                "poll_arguments": { "mode": "recent" },
                "condition": {
                    "description": "new relevant item appears",
                    "type": "not_empty"
                },
                "on_trigger": "Notify me with the matched item.",
                "notify_channel": "telegram"
            }),
            &context,
        )
        .await
        .unwrap();

    assert!(decision.allowed);
    assert!(!decision.requires_explicit_approval);
}

#[tokio::test]
async fn watch_authorization_allows_non_direct_nested_poll_action_and_delivery() {
    let runtime = runtime_for_authorization_tests().await;
    let watch = action_def_by_name(&runtime, "watch").await;
    let mut context = trusted_chat_context("test-watch-read-send-non-direct", false);
    context.direct_user_intent = false;

    let decision = runtime
        .authorize_action_invocation(
            "watch",
            Some(&watch),
            &serde_json::json!({
                "description": "Monitor connected workspace files",
                "poll_action": "gmail_scan",
                "poll_arguments": { "mode": "recent" },
                "condition": {
                    "description": "new relevant item appears",
                    "type": "not_empty"
                },
                "on_trigger": "Notify me with the matched item.",
                "notify_channel": "telegram"
            }),
            &context,
        )
        .await
        .unwrap();

    assert!(decision.allowed);
    assert!(!decision.requires_explicit_approval);
}

#[tokio::test]
async fn watch_authorization_allows_nested_sensitive_poll_with_in_app_delivery() {
    let runtime = runtime_for_authorization_tests().await;
    let watch = action_def_by_name(&runtime, "watch").await;

    let decision = runtime
        .authorize_action_invocation(
            "watch",
            Some(&watch),
            &serde_json::json!({
                "description": "Monitor connected workspace files locally",
                "poll_action": "gmail_scan",
                "poll_arguments": { "mode": "recent" },
                "condition": {
                    "description": "new relevant item appears",
                    "type": "not_empty"
                },
                "on_trigger": "Notify me with the matched item.",
                "notify_channel": "in_app"
            }),
            &trusted_chat_context("test-watch-read-in-app", false),
        )
        .await
        .unwrap();

    assert!(decision.allowed);
}

#[tokio::test]
async fn runtime_correlation_allows_sensitive_send_after_explicit_approval() {
    let runtime = runtime_for_authorization_tests().await;
    let memory = action_def_by_name(&runtime, "memory_lookup").await;
    let schedule = action_def_by_name(&runtime, "schedule_task").await;
    let context = trusted_chat_context("test-sensitive-send-approved", false);

    assert!(
        runtime
            .authorize_action_invocation(
                "memory_lookup",
                Some(&memory),
                &serde_json::json!({ "query": "saved user context" }),
                &context,
            )
            .await
            .unwrap()
            .allowed
    );

    let approved_context = trusted_chat_context("test-sensitive-send-approved", true);
    let send_decision = runtime
        .authorize_action_invocation(
            "schedule_task",
            Some(&schedule),
            &serde_json::json!({
                "task": "Send a summary",
                "at": "2026-04-18T12:00:00+05:30",
                "report_to": "ext.custom.ops"
            }),
            &approved_context,
        )
        .await
        .unwrap();

    assert!(send_decision.allowed);
}

#[tokio::test]
async fn runtime_correlation_allows_sensitive_read_then_read_only_custom_api_query() {
    let runtime = runtime_for_authorization_tests().await;
    let memory = action_def_by_name(&runtime, "memory_lookup").await;
    let context = trusted_chat_context("test-sensitive-read-custom-api-query", false);

    let read_decision = runtime
        .authorize_action_invocation(
            "memory_lookup",
            Some(&memory),
            &serde_json::json!({ "query": "saved user context" }),
            &context,
        )
        .await
        .unwrap();
    assert!(read_decision.allowed);

    let action = ActionDef {
        name: "api__project_tool__post-graphql".to_string(),
        capabilities: vec![
            "custom_api".to_string(),
            "integration".to_string(),
            "network".to_string(),
        ],
        authorization: ActionAuthorization {
            outbound: crate::actions::ActionEgressPolicy {
                read_only: true,
                outbound_write: false,
                public_publish: false,
            },
            ..ActionAuthorization::default()
        },
        ..ActionDef::default()
    };
    let query_decision = runtime
        .authorize_action_invocation(
            "api__project_tool__post-graphql",
            Some(&action),
            &serde_json::json!({
                "body": {
                    "query": "query Viewer { viewer { id } }"
                }
            }),
            &context,
        )
        .await
        .unwrap();

    assert!(query_decision.allowed);
    assert!(!query_decision.requires_explicit_approval);
}

#[tokio::test]
async fn anonymous_context_has_no_interactive_code_execute_permission_gate() {
    let runtime = runtime_for_permission_gate_tests().await;
    let action = action_def_by_name(&runtime, "code_execute").await;
    let unapproved = runtime
        .unapproved_permissions_for_action(
            &action,
            &serde_json::json!({}),
            &ActionAuthorizationContext::default(),
        )
        .await;

    assert!(unapproved.is_empty());
}

#[tokio::test]
async fn scoped_agent_blocks_unattached_channel_targets() {
    let runtime = runtime_for_authorization_tests().await;
    let decision = runtime
        .authorize_action_invocation(
            "schedule_task",
            None,
            &serde_json::json!({
                "task": "Send me a daily summary",
                "cron": "0 9 * * *",
                "report_to": "slack"
            }),
            &ActionAuthorizationContext {
                principal: Some(ActionCallerPrincipal::local_admin("test")),
                surface: ActionExecutionSurface::Chat,
                direct_user_intent: true,
                current_turn_is_explicit_approval: false,
                agent_name: Some("Ops Bot".to_string()),
                agent_access_scope: Some(crate::core::swarm::AgentAccessScope {
                    channel_ids: vec!["teams".to_string()],
                    ..Default::default()
                }),
                capability_context_id: None,
                ..ActionAuthorizationContext::default()
            },
        )
        .await
        .unwrap();

    assert!(!decision.allowed);
    assert!(decision.reason.contains("slack"));
}

#[cfg(feature = "ssh")]
#[tokio::test]
async fn scoped_agent_blocks_unattached_ssh_connection_names() {
    let runtime = runtime_for_authorization_tests().await;
    let decision = runtime
        .authorize_action_invocation(
            "ssh",
            None,
            &serde_json::json!({
                "connection": "staging-box",
                "command": "pwd"
            }),
            &ActionAuthorizationContext {
                principal: Some(ActionCallerPrincipal::local_admin("test")),
                surface: ActionExecutionSurface::Chat,
                direct_user_intent: true,
                current_turn_is_explicit_approval: false,
                agent_name: Some("Infra Bot".to_string()),
                agent_access_scope: Some(crate::core::swarm::AgentAccessScope {
                    ssh_connection_names: vec!["prod-box".to_string()],
                    ..Default::default()
                }),
                capability_context_id: None,
                ..ActionAuthorizationContext::default()
            },
        )
        .await
        .unwrap();

    assert!(!decision.allowed);
    assert!(decision.reason.contains("staging-box"));
}

#[test]
fn outbound_gate_respects_explicit_action_metadata() {
    let mut action = ActionDef::default();
    action.authorization.outbound.read_only = true;
    assert!(!ActionRuntime::action_def_requires_outbound_gate(&action));

    action.authorization.outbound.read_only = false;
    action.authorization.outbound.outbound_write = true;
    assert!(ActionRuntime::action_def_requires_outbound_gate(&action));

    action.authorization.outbound.outbound_write = false;
    action.authorization.outbound.public_publish = true;
    assert!(ActionRuntime::action_def_requires_outbound_gate(&action));
}
