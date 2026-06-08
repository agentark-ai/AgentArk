use super::super::*;

impl ActionRuntime {
    /// Execute an action with given arguments
    pub async fn execute_action(
        &self,
        action_name: &str,
        arguments: &serde_json::Value,
    ) -> Result<String> {
        self.execute_action_with_context(
            action_name,
            arguments,
            &ActionAuthorizationContext::default(),
        )
        .await
    }

    pub async fn execute_action_payload(
        &self,
        action_name: &str,
        arguments: &serde_json::Value,
    ) -> Result<ToolPayload> {
        self.execute_action_payload_with_context(
            action_name,
            arguments,
            &ActionAuthorizationContext::default(),
        )
        .await
    }

    pub async fn validate_action_invocation_with_context(
        &self,
        action_name: &str,
        arguments: &serde_json::Value,
        auth_context: &ActionAuthorizationContext,
    ) -> Result<()> {
        let info = {
            let actions = self.actions.read().await;
            actions
                .get(action_name)
                .map(|action| action.info.clone())
                .ok_or_else(|| {
                    crate::actions::structured_action_error(
                        ActionErrorDomain::Action,
                        ActionErrorReason::NotFound,
                        format!("Unknown action: {}", action_name),
                    )
                })?
        };

        let authorization_decision = self
            .authorize_action_invocation(action_name, Some(&info), arguments, auth_context)
            .await?;
        if !authorization_decision.allowed {
            return Err(crate::actions::structured_action_error(
                ActionErrorDomain::Auth,
                ActionErrorReason::PermissionDenied,
                authorization_decision.reason,
            ));
        }
        let chat_override = Self::direct_trusted_chat_tool_override(auth_context);

        if !chat_override {
            match self.refresh_action_review_state(action_name).await? {
                Some(review) => {
                    if !review.allow_execute {
                        return Err(crate::actions::structured_action_error(
                            ActionErrorDomain::Action,
                            ActionErrorReason::Unavailable,
                            review.blocked_reason.unwrap_or_else(|| {
                                format!("Action '{}' is not ready to execute.", action_name)
                            }),
                        ));
                    }
                }
                None if info.source != ActionSource::System => {
                    return Err(crate::actions::structured_action_error(
                        ActionErrorDomain::Action,
                        ActionErrorReason::Unavailable,
                        format!(
                            "Action '{}' has no persisted security review and cannot execute.",
                            action_name
                        ),
                    ));
                }
                None => {}
            }
        }

        if !chat_override {
            if info.source != ActionSource::System {
                let disabled = self.disabled_actions.read().await;
                if disabled.contains(action_name) {
                    return Err(crate::actions::structured_action_error(
                        ActionErrorDomain::Action,
                        ActionErrorReason::Unavailable,
                        format!(
                            "Action '{}' is disabled. Re-enable it in the UI before running.",
                            action_name
                        ),
                    ));
                }
            } else if !self.is_action_integration_ready(&info).await {
                let integration_id = info
                    .authorization
                    .access
                    .integration_ids
                    .first()
                    .or_else(|| info.authorization.access.extension_pack_ids.first())
                    .map(String::as_str)
                    .unwrap_or("required");
                return Err(crate::actions::structured_action_error(
                    ActionErrorDomain::Integration,
                    ActionErrorReason::NotConnected,
                    format!(
                    "Action '{}' is unavailable because required integration '{}' is not ready.",
                    action_name, integration_id
                ),
                ));
            }
        }

        match action_name {
            "http_get" | "page_fetch" => {
                let url = arguments
                    .get("url")
                    .and_then(|value| value.as_str())
                    .ok_or_else(|| {
                        crate::actions::structured_action_error(
                            ActionErrorDomain::Action,
                            ActionErrorReason::MissingInput,
                            "Missing URL",
                        )
                    })?;
                self.resolve_http_get_url_for_context(url, auth_context)
                    .await?;
            }
            _ => {}
        }

        Ok(())
    }

    pub async fn execute_action_with_context(
        &self,
        action_name: &str,
        arguments: &serde_json::Value,
        auth_context: &ActionAuthorizationContext,
    ) -> Result<String> {
        self.execute_action_legacy_with_context(action_name, arguments, auth_context)
            .await
    }

    pub async fn execute_action_payload_with_context(
        &self,
        action_name: &str,
        arguments: &serde_json::Value,
        auth_context: &ActionAuthorizationContext,
    ) -> Result<ToolPayload> {
        let output = self
            .execute_action_legacy_with_context(action_name, arguments, auth_context)
            .await?;
        let payload = Self::tool_payload_from_legacy_output(action_name, output);
        DurableStore::put_payload(
            self,
            payload,
            PersistHints {
                source_action: Some(action_name.to_string()),
                ..PersistHints::default()
            },
        )
        .await
    }

    pub(in crate::runtime) async fn execute_action_legacy_with_context(
        &self,
        action_name: &str,
        arguments: &serde_json::Value,
        auth_context: &ActionAuthorizationContext,
    ) -> Result<String> {
        let (
            sandbox_mode,
            cli_binding,
            mcp_binding,
            plugin_binding,
            custom_api_binding,
            extension_pack_binding,
            builtin_handler,
            supports_background,
            source,
            info,
        ) = {
            let actions = self.actions.read().await;
            let action = actions.get(action_name).ok_or_else(|| {
                crate::actions::structured_action_error(
                    ActionErrorDomain::Action,
                    ActionErrorReason::NotFound,
                    format!("Unknown action: {}", action_name),
                )
            })?;
            (
                action
                    .info
                    .sandbox_mode
                    .clone()
                    .unwrap_or(self.config.default_sandbox.clone()),
                action.cli_binding.clone(),
                action.mcp_binding.clone(),
                action.plugin_binding.clone(),
                action.custom_api_binding.clone(),
                action.extension_pack_binding.clone(),
                action.builtin_handler,
                action.supports_background,
                action.info.source.clone(),
                action.info.clone(),
            )
        };

        let authorization_decision = self
            .authorize_action_invocation(action_name, Some(&info), arguments, auth_context)
            .await?;
        if !authorization_decision.allowed {
            return Err(crate::actions::structured_action_error(
                ActionErrorDomain::Auth,
                ActionErrorReason::PermissionDenied,
                authorization_decision.reason,
            ));
        }
        let chat_override = Self::direct_trusted_chat_tool_override(auth_context);

        if !chat_override {
            match self.refresh_action_review_state(action_name).await? {
                Some(review) => {
                    if !review.allow_execute {
                        return Err(crate::actions::structured_action_error(
                            ActionErrorDomain::Action,
                            ActionErrorReason::Unavailable,
                            review.blocked_reason.unwrap_or_else(|| {
                                format!("Action '{}' is not ready to execute.", action_name)
                            }),
                        ));
                    }
                }
                None if source != ActionSource::System => {
                    return Err(crate::actions::structured_action_error(
                        ActionErrorDomain::Action,
                        ActionErrorReason::Unavailable,
                        format!(
                            "Action '{}' has no persisted security review and cannot execute.",
                            action_name
                        ),
                    ));
                }
                None => {}
            }
        }

        if !chat_override {
            if source != ActionSource::System {
                let disabled = self.disabled_actions.read().await;
                if disabled.contains(action_name) {
                    return Err(crate::actions::structured_action_error(
                        ActionErrorDomain::Action,
                        ActionErrorReason::Unavailable,
                        format!(
                            "Action '{}' is disabled. Re-enable it in the UI before running.",
                            action_name
                        ),
                    ));
                }
            } else if !self.is_action_integration_ready(&info).await {
                let integration_id = info
                    .authorization
                    .access
                    .integration_ids
                    .first()
                    .or_else(|| info.authorization.access.extension_pack_ids.first())
                    .map(String::as_str)
                    .unwrap_or("required");
                return Err(crate::actions::structured_action_error(
                    ActionErrorDomain::Integration,
                    ActionErrorReason::NotConnected,
                    format!(
                    "Action '{}' is unavailable because required integration '{}' is not ready.",
                    action_name, integration_id
                ),
                ));
            }
        }

        // Resolve secrets at execution time so they never appear in LLM-visible
        // tool-call arguments or execution traces.
        let resolved_args = self.resolve_secret_placeholders(action_name, arguments)?;

        if let Some(background_result) = self
            .enqueue_background_action_if_requested(&info, &resolved_args, supports_background)
            .await?
        {
            return Ok(background_result);
        }

        #[cfg(feature = "ssh")]
        if matches!(action_name, "ssh" | "ssh_connections") {
            let allowed_connections = auth_context
                .agent_access_scope
                .as_ref()
                .map(|scope| scope.ssh_connection_names.as_slice());
            return match action_name {
                "ssh" => {
                    crate::actions::ssh::ssh_execute_scoped(
                        &self.config_dir,
                        &resolved_args,
                        allowed_connections,
                    )
                    .await
                }
                "ssh_connections" => {
                    crate::actions::ssh::ssh_list_connections_scoped(
                        &self.config_dir,
                        allowed_connections,
                    )
                    .await
                }
                _ => unreachable!(),
            };
        }

        if let Some(binding) = cli_binding {
            return self
                .execute_cli_action(action_name, binding, &resolved_args)
                .await;
        }

        if let Some(binding) = mcp_binding {
            return self.execute_mcp_action(binding, &resolved_args).await;
        }

        if let Some(binding) = plugin_binding {
            let outbound_args = if Self::action_def_requires_outbound_gate(&info) {
                Self::sanitize_outbound_action_arguments(action_name, &resolved_args)?
            } else {
                resolved_args.clone()
            };
            return self.execute_plugin_action(binding, &outbound_args).await;
        }

        if let Some(binding) = custom_api_binding {
            let outbound_args = if binding.read_only {
                resolved_args.clone()
            } else {
                Self::sanitize_outbound_action_arguments(action_name, &resolved_args)?
            };
            return self
                .execute_custom_api_action(binding, &outbound_args)
                .await;
        }

        if let Some(binding) = extension_pack_binding {
            let outbound_args = if binding.read_only {
                resolved_args.clone()
            } else {
                Self::sanitize_outbound_action_arguments(action_name, &resolved_args)?
            };
            return self
                .execute_extension_pack_action(binding, &outbound_args)
                .await;
        }

        // Start transaction if rollback is enabled
        let transaction = if self.config.enable_rollback {
            let mut tx_guard = self.transactions.lock().await;
            let tx = tx_guard.begin().await?;
            tracing::debug!(action = action_name, transaction_id = %tx.id, "Started action transaction");
            Some(tx)
        } else {
            None
        };

        // Built-in actions carry their executor in the registry so metadata and
        // execution do not drift. Non-built-in workflow actions still use their
        // declared sandbox mode.
        let result = if let Some(handler) = builtin_handler {
            handler
                .execute(self, action_name, &resolved_args, auth_context)
                .await
        } else {
            match sandbox_mode {
                SandboxMode::Native => self.execute_native(action_name, &resolved_args).await,
                SandboxMode::Wasm => {
                    self.execute_wasm(action_name, &resolved_args, auth_context)
                        .await
                }
                SandboxMode::Docker => {
                    self.execute_docker(action_name, &resolved_args, auth_context)
                        .await
                }
            }
        };

        // Handle transaction
        match (&result, transaction) {
            (Ok(_), Some(tx)) => {
                let mut tx_guard = self.transactions.lock().await;
                tracing::debug!(action = action_name, transaction_id = %tx.id, "Committing action transaction");
                tx_guard.commit(tx).await?;
            }
            (Err(_), Some(tx)) => {
                tracing::warn!(action = action_name, transaction_id = %tx.id, "Rolling back action transaction due to error");
                let mut tx_guard = self.transactions.lock().await;
                tx_guard.rollback(tx).await?;
            }
            _ => {}
        }

        result
    }

    pub(in crate::runtime) async fn enqueue_background_action_if_requested(
        &self,
        info: &ActionDef,
        arguments: &serde_json::Value,
        supports_background: bool,
    ) -> Result<Option<String>> {
        if !supports_background {
            return Ok(None);
        }
        if !arguments
            .get("background")
            .and_then(|value| value.as_bool())
            .unwrap_or(false)
        {
            return Ok(None);
        }

        let Some(task_queue) = self.task_queue.as_ref() else {
            anyhow::bail!(
                "{} background execution was requested, but the task queue is unavailable",
                info.name
            );
        };

        let notify_on_complete = arguments
            .get("notify_on_complete")
            .and_then(|value| value.as_bool())
            .unwrap_or(false);
        let mut queued_arguments = arguments.clone();
        if let Some(map) = queued_arguments.as_object_mut() {
            map.insert("background".to_string(), serde_json::Value::Bool(false));
            map.insert(
                "_background_request".to_string(),
                serde_json::json!({
                    "queued_by": "runtime",
                    "notify_on_complete": notify_on_complete,
                }),
            );
        }

        let description = Self::background_task_description(info, arguments);

        let task = crate::core::Task {
            id: uuid::Uuid::new_v4(),
            description: description.clone(),
            action: info.name.clone(),
            arguments: queued_arguments,
            approval: crate::core::TaskApproval::Auto,
            capabilities: vec![info.name.clone()],
            status: crate::core::TaskStatus::Pending,
            created_at: chrono::Utc::now(),
            scheduled_for: None,
            cron: None,
            result: None,
            proof_id: None,
            priority: None,
            urgency: None,
            importance: None,
            eisenhower_quadrant: None,
        };
        let task_id = task.id.to_string();
        if let Some(storage) = self.storage.as_ref() {
            storage.insert_task(&task).await?;
        }
        task_queue.write().await.add(task);

        Ok(Some(
            serde_json::json!({
                "status": "queued",
                "tool": info.name.as_str(),
                "background": true,
                "task_id": task_id,
                "description": description,
                "notify_on_complete": notify_on_complete,
            })
            .to_string(),
        ))
    }

    pub(in crate::runtime) fn background_task_description(
        info: &ActionDef,
        arguments: &serde_json::Value,
    ) -> String {
        let primary_argument = info
            .input_schema
            .pointer("/properties")
            .and_then(|value| value.as_object())
            .and_then(|properties| {
                properties.keys().find_map(|key| {
                    arguments
                        .get(key)
                        .and_then(|value| value.as_str())
                        .map(str::trim)
                        .filter(|value| !value.is_empty())
                })
            });
        match primary_argument {
            Some(value) => format!(
                "Background {}: {}",
                info.name,
                Self::truncate_audit_text(value, 140)
            ),
            None => format!("Background {}", info.name),
        }
    }

    /// Resolve secret placeholders inside action arguments.
    ///
    /// Supported syntax:
    /// - `{{secret:KEY}}` looks up an encrypted custom secret:
    ///   - `secret:KEY` (preferred)
    ///   - `env:KEY` (compat)
    /// - `{{env:ENV_NAME}}` resolves ENV_NAME using an optional per-action binding:
    ///   - binding key: `action_envmap:{action}:{ENV_NAME}` -> {target}
    ///   - if target == "builtin", uses the agent's configured provider key(s) where applicable
    ///   - else looks up `env:{target}` in encrypted custom secrets
    ///
    /// NOTE: Returns the resolved arguments, but does not mutate the original `arguments`,
    /// so traces / tool calls remain safe.
    pub fn resolve_secret_placeholders(
        &self,
        action_name: &str,
        arguments: &serde_json::Value,
    ) -> Result<serde_json::Value> {
        let mgr = SecureConfigManager::new_with_data_dir(&self.config_dir, Some(self.data_dir()))?;
        let secrets = mgr.load_secrets()?;
        let config = mgr.load().ok();

        fn builtin_env_from_config(cfg: &AgentConfig, env: &str) -> Option<String> {
            let mut providers: Vec<&crate::core::LlmProvider> = vec![&cfg.llm];
            if let Some(fb) = cfg.llm_fallback.as_ref() {
                providers.push(fb);
            }
            for slot in &cfg.model_pool.slots {
                if slot.enabled {
                    providers.push(&slot.provider);
                }
            }

            match env {
                "OPENAI_API_KEY" => providers.into_iter().find_map(|p| match p {
                    crate::core::LlmProvider::OpenAI { api_key, .. } if !api_key.is_empty() => {
                        Some(api_key.clone())
                    }
                    _ => None,
                }),
                "OPENROUTER_API_KEY" => providers
                    .into_iter()
                    .find_map(|p| match p {
                        crate::core::LlmProvider::OpenAI {
                            api_key, base_url, ..
                        } => {
                            if !api_key.is_empty()
                                && base_url.as_deref().unwrap_or("").contains("openrouter")
                            {
                                Some(api_key.clone())
                            } else {
                                None
                            }
                        }
                        _ => None,
                    })
                    .or_else(|| builtin_env_from_config(cfg, "OPENAI_API_KEY")),
                "ANTHROPIC_API_KEY" => providers.into_iter().find_map(|p| match p {
                    crate::core::LlmProvider::Anthropic { api_key, .. } if !api_key.is_empty() => {
                        Some(api_key.clone())
                    }
                    _ => None,
                }),
                _ => None,
            }
        }

        fn legacy_env_alias_lookup(
            custom: &std::collections::HashMap<String, String>,
            env: &str,
        ) -> Option<String> {
            // Compatibility: existing integrations store provider tokens under non-env keys.
            let legacy_key = match env {
                "GITHUB_TOKEN" => Some("github_token"),
                "NOTION_TOKEN" => Some("notion_token"),
                "TWITTER_BEARER_TOKEN" => Some("twitter_bearer_token"),
                "ONEPASSWORD_TOKEN" => Some("onepassword_token"),
                "GOOGLE_PLACES_API_KEY" => Some("google_places_api_key"),
                "TWILIO_AUTH_TOKEN" => Some("twilio_auth_token"),
                "TWILIO_ACCOUNT_SID" => Some("twilio_account_sid"),
                "GARMIN_TOKEN" => Some("garmin_token"),
                "GARMIN_API_BASE" => Some("garmin_api_base"),
                "WHOOP_TOKEN" => Some("whoop_token"),
                "GA4_ACCESS_TOKEN" => Some("ga4_access_token"),
                "GA4_PROPERTY_ID" => Some("ga4_property_id"),
                "GSC_ACCESS_TOKEN" => Some("gsc_access_token"),
                "GSC_SITE_URL" => Some("gsc_site_url"),
                "SOCIAL_TWITTER_BEARER_TOKEN" => Some("social_twitter_bearer_token"),
                "SOCIAL_GA4_ACCESS_TOKEN" => Some("social_ga4_access_token"),
                "SOCIAL_GA4_PROPERTY_ID" => Some("social_ga4_property_id"),
                _ => None,
            }?;
            custom.get(legacy_key).cloned()
        }

        let re = regex::Regex::new(r"\{\{\s*(secret|env)\s*:\s*([A-Za-z0-9_\-:.]+)\s*\}\}")
            .expect("valid placeholder regex");
        let custom = &secrets.custom;

        let resolve_secret = |key: &str| -> Option<String> {
            custom
                .get(&format!("secret:{}", key))
                .cloned()
                .or_else(|| custom.get(&format!("env:{}", key)).cloned())
                .or_else(|| {
                    config
                        .as_ref()
                        .and_then(|cfg| builtin_env_from_config(cfg, key))
                })
        };

        let resolve_env = |env: &str| -> Option<String> {
            let binding_key = format!("action_envmap:{}:{}", action_name, env);
            let target = custom
                .get(&binding_key)
                .cloned()
                .unwrap_or_else(|| env.to_string());

            if target == "builtin" {
                return config
                    .as_ref()
                    .and_then(|cfg| builtin_env_from_config(cfg, env));
            }

            custom
                .get(&format!("env:{}", target))
                .cloned()
                .or_else(|| custom.get(&format!("secret:{}", target)).cloned())
                .or_else(|| legacy_env_alias_lookup(custom, env))
                .or_else(|| {
                    config
                        .as_ref()
                        .and_then(|cfg| builtin_env_from_config(cfg, env))
                })
        };

        fn substitute_in_str(
            s: &str,
            re: &regex::Regex,
            action_name: &str,
            resolve_secret: &impl Fn(&str) -> Option<String>,
            resolve_env: &impl Fn(&str) -> Option<String>,
        ) -> Result<String> {
            let mut out = String::with_capacity(s.len());
            let mut last = 0usize;
            for caps in re.captures_iter(s) {
                let m = caps.get(0).unwrap();
                out.push_str(&s[last..m.start()]);
                let kind = caps.get(1).map(|m| m.as_str()).unwrap_or("");
                let key = caps.get(2).map(|m| m.as_str()).unwrap_or("");
                let placeholder_kind = match kind {
                    "secret" => MissingSecretPlaceholderKind::Secret,
                    "env" => MissingSecretPlaceholderKind::Env,
                    _ => MissingSecretPlaceholderKind::Secret,
                };
                let val = match placeholder_kind {
                    MissingSecretPlaceholderKind::Secret => resolve_secret(key),
                    MissingSecretPlaceholderKind::Env => resolve_env(key),
                }
                .ok_or_else(|| {
                    anyhow::Error::new(MissingSecretPlaceholder::new(
                        action_name,
                        placeholder_kind,
                        key,
                    ))
                })?;
                out.push_str(&val);
                last = m.end();
            }
            out.push_str(&s[last..]);
            Ok(out)
        }

        fn walk(
            v: &serde_json::Value,
            re: &regex::Regex,
            action_name: &str,
            resolve_secret: &impl Fn(&str) -> Option<String>,
            resolve_env: &impl Fn(&str) -> Option<String>,
        ) -> Result<serde_json::Value> {
            Ok(match v {
                serde_json::Value::String(s) => serde_json::Value::String(substitute_in_str(
                    s,
                    re,
                    action_name,
                    resolve_secret,
                    resolve_env,
                )?),
                serde_json::Value::Array(arr) => {
                    let mut out = Vec::with_capacity(arr.len());
                    for item in arr {
                        out.push(walk(item, re, action_name, resolve_secret, resolve_env)?);
                    }
                    serde_json::Value::Array(out)
                }
                serde_json::Value::Object(map) => {
                    let mut out = serde_json::Map::with_capacity(map.len());
                    for (k, val) in map {
                        out.insert(
                            k.clone(),
                            walk(val, re, action_name, resolve_secret, resolve_env)?,
                        );
                    }
                    serde_json::Value::Object(out)
                }
                other => other.clone(),
            })
        }

        walk(arguments, &re, action_name, &resolve_secret, &resolve_env)
    }

    pub(in crate::runtime) fn action_def_requires_outbound_gate(info: &ActionDef) -> bool {
        let outbound = &info.authorization.outbound;
        !outbound.read_only && (outbound.outbound_write || outbound.public_publish)
    }

    pub(in crate::runtime) fn sanitize_outbound_action_arguments(
        action_name: &str,
        arguments: &serde_json::Value,
    ) -> Result<serde_json::Value> {
        let privacy = crate::security::sanitize_outbound_json(
            arguments,
            &crate::security::OutboundPrivacyPolicy::default(),
        );
        match privacy.decision {
            crate::security::OutboundPrivacyDecision::Allow => Ok(arguments.clone()),
            crate::security::OutboundPrivacyDecision::RedactedAllow => {
                tracing::warn!(
                    action = action_name,
                    redactions = ?privacy.redactions,
                    reasons = ?privacy.reasons,
                    "Outbound privacy gate redacted action arguments"
                );
                Ok(privacy.sanitized_value)
            }
            crate::security::OutboundPrivacyDecision::Block => Err(anyhow::anyhow!(
                "{}",
                crate::security::format_outbound_privacy_block(
                    &format!("action '{}'", action_name),
                    &privacy.reasons,
                )
            )),
        }
    }

    pub(in crate::runtime) fn custom_api_binding_supports_graphql_body(
        binding: &CustomApiBinding,
    ) -> bool {
        crate::core::request_contract::endpoint_has_graphql_signal(
            &binding.path,
            &binding.default_headers,
        ) || binding
            .default_body
            .as_ref()
            .is_some_and(crate::core::request_contract::body_has_graphql_signal)
    }

    pub(in crate::runtime) fn graphql_response_has_errors(body: &str) -> bool {
        serde_json::from_str::<serde_json::Value>(body)
            .ok()
            .and_then(|value| value.get("errors").cloned())
            .and_then(|errors| errors.as_array().map(|items| !items.is_empty()))
            .unwrap_or(false)
    }
}
