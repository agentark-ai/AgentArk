pub fn default_hidden_action(action_name: &str) -> bool {
    matches!(
        action_name.trim(),
        "pipeline_compile"
            | "pipeline_run"
            | "goal_manage"
            | "app_deploy"
            | "app_restart"
            | "app_stop"
            | "app_delete"
            | "watch"
            | "list_watchers"
            | "watcher_delete"
            | "background_session_manage"
    )
}
