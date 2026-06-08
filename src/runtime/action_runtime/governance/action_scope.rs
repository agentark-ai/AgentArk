use super::super::*;

impl ActionRuntime {
    pub(in crate::runtime) async fn action_requires_privileged_allow(
        &self,
        action_name: &str,
    ) -> bool {
        if matches!(
            action_name,
            "pipeline_run" | "pipeline_compile" | "manage_actions"
        ) {
            return true;
        }
        let (source, capabilities) = {
            let actions = self.actions.read().await;
            let Some(action) = actions.get(action_name) else {
                return true;
            };
            (action.info.source.clone(), action.info.capabilities.clone())
        };

        let has_dangerous_cap = capabilities.iter().any(|cap| {
            matches!(
                cap.to_ascii_lowercase().as_str(),
                "shell"
                    | "file_write"
                    | "clipboard_write"
                    | "gmail"
                    | "google_workspace"
                    | "code_execute"
                    | "app_hosting"
                    | "orchestration"
                    | "ssh"
            )
        });

        has_dangerous_cap || (source != ActionSource::System && capabilities.is_empty())
    }

    pub(in crate::runtime) fn action_scope_hint_for_loaded_action(
        _action_name: &str,
        loaded: &LoadedAction,
    ) -> ActionScopeHint {
        ActionScopeHint {
            mcp_server_id: loaded
                .mcp_binding
                .as_ref()
                .map(|binding| binding.server_id.clone()),
            custom_api_id: loaded
                .custom_api_binding
                .as_ref()
                .map(|binding| binding.api_id.clone()),
            integration_ids: loaded.info.authorization.access.integration_ids.clone(),
            extension_pack_ids: {
                let mut ids = loaded.info.authorization.access.extension_pack_ids.clone();
                if let Some(binding) = loaded.extension_pack_binding.as_ref() {
                    if !ids.iter().any(|value| value == &binding.pack_id) {
                        ids.push(binding.pack_id.clone());
                    }
                }
                ids
            },
            requires_ssh_connection: loaded.info.authorization.access.requires_ssh_connection,
            channel_targets: loaded.info.authorization.access.channel_targets.clone(),
        }
    }

    pub(in crate::runtime) fn fallback_action_scope_hint(_action_name: &str) -> ActionScopeHint {
        ActionScopeHint::default()
    }

    pub(in crate::runtime) fn normalize_scope_channel_target(
        value: Option<&str>,
        default_target: &str,
    ) -> String {
        match value
            .map(str::trim)
            .filter(|raw| !raw.is_empty())
            .map(|raw| raw.to_ascii_lowercase())
        {
            Some(channel) if matches!(channel.as_str(), "push" | "auto" | "default") => {
                "preferred".to_string()
            }
            Some(channel)
                if matches!(
                    channel.as_str(),
                    "app" | "app_notification" | "app_notifications" | "in_app"
                ) =>
            {
                String::new()
            }
            Some(channel) if channel == "http" => "web".to_string(),
            Some(channel) => channel,
            None => default_target.to_string(),
        }
    }

    pub(in crate::runtime) fn scoped_channel_target_for_hint(
        hint: &ActionScopeHint,
        arguments: &serde_json::Value,
    ) -> Option<String> {
        let target = hint.channel_targets.first()?;
        Some(Self::normalize_scope_channel_target(
            arguments
                .get(target.argument_key.as_str())
                .and_then(|value| value.as_str()),
            target.default_target.as_str(),
        ))
    }

    pub(in crate::runtime) fn uses_broad_network(action: &ActionDef) -> bool {
        let outbound = &action.authorization.outbound;
        outbound.outbound_write || outbound.public_publish
    }

    pub(in crate::runtime) fn builtin_dangerous_permissions(
        action: &ActionDef,
    ) -> Vec<crate::security::action_guard::Permission> {
        crate::security::action_guard::ActionGuard::permissions_from_capabilities(
            &action.capabilities,
        )
        .into_iter()
        .filter(|permission| {
            !matches!(
                permission,
                crate::security::action_guard::Permission::Custom(_)
            ) && Self::permission_needs_agent_approval(permission)
        })
        .collect()
    }

    pub(in crate::runtime) fn permission_needs_agent_approval(
        permission: &crate::security::action_guard::Permission,
    ) -> bool {
        crate::security::action_guard::ActionGuard::permission_risk(permission)
            == crate::security::action_guard::PermissionRisk::Dangerous
    }

    pub fn action_permission_ids(action: &ActionDef) -> Vec<String> {
        let mut permission_ids = action.authorization.access.permission_ids.clone();
        permission_ids.extend(
            Self::builtin_dangerous_permissions(action)
                .into_iter()
                .map(|permission| permission.to_string()),
        );
        if !action.authorization.access.channel_targets.is_empty() {
            permission_ids.push("messaging_send".to_string());
        }
        permission_ids
            .into_iter()
            .map(|permission| permission.trim().to_ascii_lowercase())
            .filter(|permission| !permission.is_empty())
            .collect()
    }

    pub(in crate::runtime) fn action_demands_broad_network_consent(action: &ActionDef) -> bool {
        Self::uses_broad_network(action)
            && !action
                .authorization
                .access
                .permission_ids
                .iter()
                .any(|permission| permission.trim().eq_ignore_ascii_case("broad_network"))
    }

    pub fn action_required_agent_permission_ids(action: &ActionDef) -> Vec<String> {
        let mut permission_ids = Self::action_permission_ids(action);
        if Self::action_demands_broad_network_consent(action) {
            permission_ids.push("broad_network".to_string());
        }
        permission_ids.sort();
        permission_ids.dedup();
        permission_ids
    }

    pub(in crate::runtime) fn scope_contains_exact_value(
        allowed: &[String],
        candidate: &str,
    ) -> bool {
        let candidate = candidate.trim();
        allowed.iter().any(|value| value.trim() == candidate)
    }

    pub(in crate::runtime) fn scope_contains_case_insensitive_value(
        allowed: &[String],
        candidate: &str,
    ) -> bool {
        let candidate = candidate.trim();
        allowed
            .iter()
            .any(|value| value.trim().eq_ignore_ascii_case(candidate))
    }

    pub(in crate::runtime) fn scope_contains_channel_target(
        allowed: &[String],
        candidate: &str,
    ) -> bool {
        let candidate = candidate.trim().to_ascii_lowercase();
        allowed.iter().any(|value| {
            Self::normalize_scope_channel_target(Some(value.as_str()), "")
                .trim()
                .eq_ignore_ascii_case(candidate.as_str())
        })
    }

    pub(in crate::runtime) fn scoped_actor_label(
        auth_context: &ActionAuthorizationContext,
    ) -> String {
        auth_context
            .agent_name
            .as_deref()
            .map(str::trim)
            .filter(|value| !value.is_empty())
            .map(|value| format!("Agent '{}'", value))
            .unwrap_or_else(|| "This agent".to_string())
    }

    pub(in crate::runtime) async fn authorize_action_scope(
        &self,
        action_name: &str,
        arguments: &serde_json::Value,
        auth_context: &ActionAuthorizationContext,
    ) -> Option<ActionAuthorizationDecision> {
        let scope = auth_context.agent_access_scope.as_ref()?;
        let hint = {
            let actions = self.actions.read().await;
            actions
                .get(action_name)
                .map(|loaded| Self::action_scope_hint_for_loaded_action(action_name, loaded))
        }
        .unwrap_or_else(|| Self::fallback_action_scope_hint(action_name));
        let actor = Self::scoped_actor_label(auth_context);

        if let Some(server_id) = hint.mcp_server_id.as_deref() {
            if !Self::scope_contains_exact_value(&scope.mcp_server_ids, server_id) {
                return Some(ActionAuthorizationDecision::deny(format!(
                    "{} is not allowed to use MCP server '{}'.",
                    actor, server_id
                )));
            }
        }

        if let Some(api_id) = hint.custom_api_id.as_deref() {
            if !Self::scope_contains_exact_value(&scope.custom_api_ids, api_id) {
                return Some(ActionAuthorizationDecision::deny(format!(
                    "{} is not allowed to use custom API '{}'.",
                    actor, api_id
                )));
            }
        }

        if !hint.integration_ids.is_empty()
            && !hint.integration_ids.iter().any(|integration_id| {
                Self::scope_contains_case_insensitive_value(&scope.integration_ids, integration_id)
            })
        {
            return Some(ActionAuthorizationDecision::deny(format!(
                "{} is not allowed to use integration(s): {}.",
                actor,
                hint.integration_ids.join(", ")
            )));
        }

        if !hint.extension_pack_ids.is_empty()
            && !hint.extension_pack_ids.iter().any(|pack_id| {
                Self::scope_contains_case_insensitive_value(&scope.extension_pack_ids, pack_id)
            })
        {
            return Some(ActionAuthorizationDecision::deny(format!(
                "{} is not allowed to use extension pack(s): {}.",
                actor,
                hint.extension_pack_ids.join(", ")
            )));
        }

        if hint.requires_ssh_connection {
            if scope.ssh_connection_names.is_empty() {
                return Some(ActionAuthorizationDecision::deny(format!(
                    "{} is not allowed to use SSH because no SSH connections are attached.",
                    actor
                )));
            }
            if action_name == "ssh" {
                if let Some(connection_name) = arguments
                    .get("connection")
                    .and_then(|value| value.as_str())
                    .map(str::trim)
                    .filter(|value| !value.is_empty())
                {
                    if !Self::scope_contains_exact_value(
                        &scope.ssh_connection_names,
                        connection_name,
                    ) {
                        return Some(ActionAuthorizationDecision::deny(format!(
                            "{} is not allowed to use SSH connection '{}'.",
                            actor, connection_name
                        )));
                    }
                }
            }
        }

        if !hint.channel_targets.is_empty() {
            let channel_target = Self::scoped_channel_target_for_hint(&hint, arguments)
                .unwrap_or_else(|| "preferred".to_string());
            if !channel_target.is_empty() {
                if channel_target == "preferred" {
                    if scope.channel_ids.is_empty() {
                        return Some(ActionAuthorizationDecision::deny(format!(
                            "{} is not allowed to use preferred-channel delivery because no messaging channels are attached.",
                            actor
                        )));
                    }
                } else if !Self::scope_contains_channel_target(&scope.channel_ids, &channel_target)
                {
                    return Some(ActionAuthorizationDecision::deny(format!(
                        "{} is not allowed to use messaging channel '{}'.",
                        actor, channel_target
                    )));
                }
            }
        }

        None
    }

    pub(in crate::runtime) async fn is_action_integration_ready(&self, action: &ActionDef) -> bool {
        self.action_integration_unready_reason(action)
            .await
            .is_none()
    }

    pub(in crate::runtime) async fn action_integration_unready_reason(
        &self,
        action: &ActionDef,
    ) -> Option<String> {
        let access = &action.authorization.access;
        let integration_ids = &access.integration_ids;
        let extension_pack_ids = &access.extension_pack_ids;
        if integration_ids.is_empty() && extension_pack_ids.is_empty() {
            return None;
        }
        let mut reasons = Vec::new();
        if !integration_ids.is_empty() {
            let manager = crate::integrations::IntegrationManager::new(&self.config_dir);
            let workspace_granted_bundles =
                if access.integration_features.contains_key("google_workspace") {
                    Some(
                        crate::actions::google_workspace::granted_bundles(&self.config_dir)
                            .unwrap_or_default(),
                    )
                } else {
                    None
                };
            for integration_id in integration_ids {
                if !manager.is_ready(integration_id).await {
                    reasons.push(format!(
                        "required integration '{}' is not connected or authenticated",
                        integration_id
                    ));
                    continue;
                }
                let features = access.integration_features.get(integration_id)?;
                if features.is_empty() {
                    return None;
                }
                let features_ready = match integration_id.as_str() {
                    "google_workspace" => {
                        workspace_granted_bundles.as_ref().is_some_and(|granted| {
                            features.iter().all(|feature| {
                                crate::actions::google_workspace::normalize_bundle_id(feature)
                                    .is_some_and(|normalized| {
                                        granted
                                            .iter()
                                            .any(|granted_bundle| granted_bundle == &normalized)
                                    })
                            })
                        })
                    }
                    _ => true,
                };
                if features_ready {
                    return None;
                }
                let missing_features = match integration_id.as_str() {
                    "google_workspace" => {
                        let granted = workspace_granted_bundles.as_deref().unwrap_or(&[]);
                        features
                            .iter()
                            .filter_map(|feature| {
                                crate::actions::google_workspace::normalize_bundle_id(feature)
                                    .or_else(|| Some(feature.trim().to_string()))
                            })
                            .filter(|feature| {
                                !feature.is_empty()
                                    && !granted
                                        .iter()
                                        .any(|granted_bundle| granted_bundle == feature)
                            })
                            .collect::<Vec<_>>()
                    }
                    _ => features
                        .iter()
                        .map(|feature| feature.trim().to_string())
                        .filter(|feature| !feature.is_empty())
                        .collect::<Vec<_>>(),
                };
                if missing_features.is_empty() {
                    reasons.push(format!(
                        "required integration '{}' is connected but required feature grants are not available",
                        integration_id
                    ));
                } else {
                    reasons.push(format!(
                        "required integration '{}' is connected but missing feature grant(s): {}",
                        integration_id,
                        missing_features.join(", ")
                    ));
                }
            }
        }
        if !extension_pack_ids.is_empty() {
            let Some(registry) = self.extension_pack_registry.as_ref() else {
                reasons.push("required extension pack registry is not available".to_string());
                return Some(reasons.join("; "));
            };
            let guard = registry.read().await;
            for pack_id in extension_pack_ids {
                let Ok(Some(pack)) = guard.get_pack(pack_id).await else {
                    reasons.push(format!(
                        "required extension pack '{}' is not installed",
                        pack_id
                    ));
                    continue;
                };
                if pack.enabled
                    && pack.installed
                    && matches!(pack.status.as_str(), "ready" | "connected")
                {
                    return None;
                }
                reasons.push(format!(
                    "required extension pack '{}' is not ready (status: {})",
                    pack_id, pack.status
                ));
            }
        }
        Some(if reasons.is_empty() {
            "required integration or extension capability is not ready".to_string()
        } else {
            reasons.join("; ")
        })
    }

    pub(in crate::runtime) fn pipeline_key_slug(input: &str) -> String {
        let mut out = String::with_capacity(input.len());
        for ch in input.chars() {
            if ch.is_ascii_alphanumeric() || ch == '-' || ch == '_' {
                out.push(ch.to_ascii_lowercase());
            } else {
                out.push('_');
            }
        }
        out.trim_matches('_').to_string()
    }

    pub(in crate::runtime) fn context_map_from_json(
        value: Option<&serde_json::Value>,
    ) -> BTreeMap<String, String> {
        let mut out = BTreeMap::new();
        let Some(obj) = value.and_then(|v| v.as_object()) else {
            return out;
        };
        for (k, v) in obj {
            let val = match v {
                serde_json::Value::String(s) => s.clone(),
                serde_json::Value::Null => String::new(),
                other => other.to_string(),
            };
            out.insert(k.clone(), val);
        }
        out
    }

    pub(in crate::runtime) fn render_json_templates(
        value: &serde_json::Value,
        context: &BTreeMap<String, String>,
    ) -> serde_json::Value {
        match value {
            serde_json::Value::String(s) => serde_json::Value::String(
                crate::core::orchestration::pipeline::render_template(s, context),
            ),
            serde_json::Value::Array(arr) => serde_json::Value::Array(
                arr.iter()
                    .map(|v| Self::render_json_templates(v, context))
                    .collect(),
            ),
            serde_json::Value::Object(obj) => {
                let mut map = serde_json::Map::with_capacity(obj.len());
                for (k, v) in obj {
                    map.insert(k.clone(), Self::render_json_templates(v, context));
                }
                serde_json::Value::Object(map)
            }
            other => other.clone(),
        }
    }

    pub(in crate::runtime) fn coerce_to_json(output: &str) -> serde_json::Value {
        serde_json::from_str(output)
            .unwrap_or_else(|_| serde_json::Value::String(output.to_string()))
    }

    pub(in crate::runtime) fn extract_status_code(message: &str) -> Option<u16> {
        for token in message.split(|c: char| !c.is_ascii_digit()) {
            if token.len() == 3 {
                if let Ok(code) = token.parse::<u16>() {
                    if (100..=599).contains(&code) {
                        return Some(code);
                    }
                }
            }
        }
        None
    }

    pub(in crate::runtime) fn is_retryable_error(
        message: &str,
        retry: &crate::core::orchestration::pipeline::RetryPolicy,
    ) -> bool {
        if let Some(status) = Self::extract_status_code(message) {
            return retry.retry_on_status.contains(&status);
        }
        let lower = message.to_ascii_lowercase();
        if lower.contains("missing ")
            || lower.contains("invalid ")
            || lower.contains("unknown action")
            || lower.contains("permission")
            || lower.contains("denied")
            || lower.contains("not found")
        {
            return false;
        }
        true
    }

    pub(in crate::runtime) async fn sleep_with_backoff(backoff_ms: u64, jitter_ratio: f64) {
        let sleep_ms = if jitter_ratio <= 0.0 {
            backoff_ms.max(25)
        } else {
            use rand::RngExt;
            let span = ((backoff_ms as f64) * jitter_ratio).round() as i64;
            if span <= 0 {
                backoff_ms.max(25)
            } else {
                let mut rng = rand::rng();
                let jitter = rng.random_range(-span..=span);
                ((backoff_ms as i64 + jitter).max(25)) as u64
            }
        };
        tokio::time::sleep(std::time::Duration::from_millis(sleep_ms)).await;
    }
}
