use super::*;

mod authorization;
mod capability_custom_api;
mod completion_and_files;
mod integrations;
mod vision_and_schemas;

pub(super) fn parse_tool_completion_output(output: &str) -> serde_json::Value {
    let payload = output
        .trim_start()
        .strip_prefix(TOOL_COMPLETION_MARKER)
        .expect("tool output should use completion marker");
    serde_json::from_str(payload).expect("completion marker should contain JSON")
}

pub(super) async fn runtime_for_authorization_tests() -> ActionRuntime {
    let temp = tempfile::tempdir().unwrap();
    let runtime = ActionRuntime::new(temp.path(), temp.path()).await.unwrap();
    runtime.load_builtin_actions().await.unwrap();
    runtime
}

pub(super) async fn runtime_for_permission_gate_tests() -> ActionRuntime {
    let temp = tempfile::tempdir().unwrap();
    let mut runtime = ActionRuntime::new(temp.path(), temp.path()).await.unwrap();
    let guard = crate::security::ActionGuard::new(
        &ed25519_dalek::SigningKey::from_bytes(&[7u8; 32]),
        "did:key:test",
        temp.path(),
        temp.path(),
    )
    .await
    .unwrap();
    runtime.set_action_guard(std::sync::Arc::new(guard));
    runtime.load_builtin_actions().await.unwrap();
    runtime
}

pub(super) async fn action_def_by_name(runtime: &ActionRuntime, name: &str) -> ActionDef {
    runtime
        .list_actions()
        .await
        .unwrap()
        .into_iter()
        .find(|action| action.name == name)
        .expect("action should exist")
}

pub(super) fn trusted_chat_context(
    capability_context_id: &str,
    current_turn_is_explicit_approval: bool,
) -> ActionAuthorizationContext {
    ActionAuthorizationContext {
        principal: Some(ActionCallerPrincipal::local_admin("test")),
        surface: ActionExecutionSurface::Chat,
        direct_user_intent: true,
        current_turn_is_explicit_approval,
        agent_name: None,
        agent_access_scope: None,
        capability_context_id: Some(capability_context_id.to_string()),
        ..ActionAuthorizationContext::default()
    }
}
