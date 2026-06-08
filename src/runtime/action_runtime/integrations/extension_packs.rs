use super::super::*;

impl ActionRuntime {
    pub(in crate::runtime) async fn sync_extension_pack_runtime_actions(&self) -> Result<()> {
        let registry = self
            .extension_pack_registry
            .clone()
            .ok_or_else(|| anyhow::anyhow!("Extension-pack registry not available"))?;
        let guard = registry.read().await;
        guard.sync_to_runtime(self).await
    }

    pub(in crate::runtime) async fn execute_extension_pack_list(
        &self,
        arguments: &serde_json::Value,
    ) -> Result<String> {
        let registry = self
            .extension_pack_registry
            .clone()
            .ok_or_else(|| anyhow::anyhow!("Extension-pack registry not available"))?;
        let query = arguments.get("query").and_then(|value| value.as_str());
        let kind = arguments.get("kind").and_then(|value| value.as_str());
        let guard = registry.read().await;
        Ok(serde_json::to_string_pretty(
            &guard.search_packs(query, kind).await?,
        )?)
    }

    pub(in crate::runtime) async fn execute_extension_pack_search(
        &self,
        arguments: &serde_json::Value,
    ) -> Result<String> {
        self.execute_extension_pack_list(arguments).await
    }

    pub(in crate::runtime) async fn execute_extension_pack_install(
        &self,
        arguments: &serde_json::Value,
    ) -> Result<String> {
        let request: crate::extension_packs::ExtensionPackInstallRequest =
            serde_json::from_value(arguments.clone()).map_err(|error| {
                anyhow::anyhow!("Invalid extension pack install arguments: {}", error)
            })?;
        let registry = self
            .extension_pack_registry
            .clone()
            .ok_or_else(|| anyhow::anyhow!("Extension-pack registry not available"))?;
        let requested_pack_id = request
            .pack_id
            .as_deref()
            .map(str::trim)
            .filter(|value| !value.is_empty())
            .map(str::to_string);
        let has_explicit_source = request
            .source_url
            .as_deref()
            .is_some_and(|value| !value.trim().is_empty())
            || request
                .source_path
                .as_deref()
                .is_some_and(|value| !value.trim().is_empty())
            || request
                .manifest_text
                .as_deref()
                .is_some_and(|value| !value.trim().is_empty())
            || request.manifest.is_some();
        if let Some(pack_id) = requested_pack_id
            .as_deref()
            .filter(|_| !has_explicit_source)
        {
            let existing_or_catalog = {
                let guard = registry.read().await;
                guard.get_pack(pack_id).await?
            };
            match existing_or_catalog {
                Some(view) if view.installed => {
                    return Ok(serde_json::to_string_pretty(&serde_json::json!({
                        "status": "already_installed",
                        "installed": true,
                        "pack_id": view.manifest.id,
                        "message": "Extension pack is already installed.",
                        "pack": view,
                    }))?);
                }
                None => {
                    return Ok(structured_tool_completion_output(
                        "extension_pack_install",
                        "not_found",
                        "No bundled extension pack matched this id, so no pack was installed from the catalog. Continue with pack search, authoritative docs/source lookup, extension_pack_scaffold, or capability_acquire as appropriate for the requested integration or channel.",
                        serde_json::json!({
                            "success": false,
                            "retryable": true,
                            "status": "catalog_miss",
                            "installed": false,
                            "pack_id": pack_id,
                            "next_steps": [
                                "Install from source_url, source_path, manifest_text, or manifest when a pack source exists.",
                                "Use extension_pack_scaffold for a draft manifest-based pack.",
                                "Use capability_acquire for HTTP/API integrations that should be saved as custom API integrations."
                            ]
                        }),
                    ));
                }
                _ => {}
            }
        }
        let pack = {
            let mut guard = registry.write().await;
            guard.install(request).await?
        };
        self.sync_extension_pack_runtime_actions().await?;
        Ok(serde_json::to_string_pretty(&pack)?)
    }

    pub(in crate::runtime) async fn execute_extension_pack_scaffold(
        &self,
        arguments: &serde_json::Value,
    ) -> Result<String> {
        let request: crate::extension_packs::ExtensionPackScaffoldRequest =
            serde_json::from_value(arguments.clone()).map_err(|error| {
                anyhow::anyhow!("Invalid extension pack scaffold arguments: {}", error)
            })?;
        let registry = self
            .extension_pack_registry
            .clone()
            .ok_or_else(|| anyhow::anyhow!("Extension-pack registry not available"))?;
        let pack = {
            let mut guard = registry.write().await;
            guard.scaffold(request).await?
        };
        self.sync_extension_pack_runtime_actions().await?;
        Ok(serde_json::to_string_pretty(&pack)?)
    }

    pub(in crate::runtime) async fn execute_custom_messaging_channel_upsert(
        &self,
        arguments: &serde_json::Value,
    ) -> Result<String> {
        let violations = crate::core::request_contract::validate_json_object_envelope(
            "custom_messaging_channel",
            arguments,
        );
        if !violations.is_empty() {
            return Ok(structured_tool_completion_output(
                "custom_messaging_channel_upsert",
                "needs_arguments",
                "Custom messaging channel cannot be saved because the arguments envelope is invalid.",
                serde_json::json!({
                    "violations": violations,
                    "expected_contract": crate::core::request_contract::expected_arguments_envelope_contract("custom_messaging_channel"),
                    "assistant_instruction": "Satisfy the complete custom messaging channel arguments envelope in one corrected call. If a required field is genuinely absent from the conversation, ask the user for that missing value."
                }),
            ));
        }
        let storage = self.runtime_storage()?;
        let request: crate::custom_messaging_channels::CustomMessagingChannelUpsertRequest =
            serde_json::from_value(arguments.clone()).map_err(|error| {
                anyhow::anyhow!("Invalid custom messaging channel arguments: {}", error)
            })?;
        let requested_id = crate::custom_messaging_channels::config_id_for_request(&request);
        let existing = crate::custom_messaging_channels::get_custom_messaging_channel_config(
            &storage,
            &requested_id,
        )
        .await?;
        let operation = if existing.is_some() {
            "update"
        } else {
            "create"
        };
        let view = crate::custom_messaging_channels::upsert_custom_messaging_channel(
            &storage,
            &self.config_dir,
            self.data_dir(),
            request,
            existing.as_ref().map(|_| requested_id.as_str()),
        )
        .await?;
        self.record_custom_messaging_channel_upsert_event(&view, operation)
            .await;
        let integration_id = view
            .config
            .auth_manifest
            .as_ref()
            .map(|manifest| manifest.integration_id.clone());
        let channel_id = view.runtime_channel_id.clone();
        let config_id = view.config.id.clone();
        let channel_name = view.config.name.clone();
        let needs_credentials = view.requires_auth && !view.configured;
        let settings_path = format!(
            "Settings > Messaging Channels > Custom Messaging Channels > {}",
            channel_name
        );
        let message = if needs_credentials {
            format!(
                "Custom messaging channel saved. Credentials are still required. Use the secure credential form in chat or open {}. Do not paste secrets into normal chat.",
                settings_path
            )
        } else {
            "Custom messaging channel saved and ready.".to_string()
        };
        let credential_request = if needs_credentials {
            integration_id.as_ref().map(|integration_id| {
                serde_json::json!({
                    "kind": "integration_auth",
                    "integration_id": integration_id,
                    "display_name": channel_name,
                    "settings_path": settings_path,
                    "secure_input_required": true
                })
            })
        } else {
            None
        };
        // Structured completion (top-level tool + status) so the legacy tool
        // wrapper preserves the status verbatim and the spine classifies a
        // credential handoff as NeedsInput; raw JSON here used to be rewrapped
        // as "completed" with the real status buried, silently dropping the
        // secure-credential prompt.
        Ok(structured_tool_completion_output(
            "custom_messaging_channel_upsert",
            if needs_credentials {
                "needs_credentials"
            } else {
                "configured"
            },
            message.clone(),
            serde_json::json!({
                "channel_id": channel_id,
                "integration_id": integration_id,
                "settings_path": settings_path,
                "credential_request": credential_request,
                "custom_messaging_channel": {
                    "id": config_id,
                    "name": channel_name,
                    "configured": view.configured,
                    "requires_auth": view.requires_auth,
                },
                "message": message
            }),
        ))
    }

    pub(in crate::runtime) async fn execute_custom_messaging_channel_manage(
        &self,
        arguments: &serde_json::Value,
    ) -> Result<String> {
        let violations = crate::core::request_contract::validate_json_object_envelope(
            "custom_messaging_channel",
            arguments,
        );
        if !violations.is_empty() {
            return Ok(structured_tool_completion_output(
                "custom_messaging_channel_manage",
                "needs_arguments",
                "Custom messaging channel management cannot run because the arguments envelope is invalid.",
                serde_json::json!({
                    "violations": violations,
                    "expected_contract": crate::core::request_contract::expected_arguments_envelope_contract("custom_messaging_channel"),
                    "assistant_instruction": "Satisfy the complete custom messaging channel management arguments envelope in one corrected call. If a required field is genuinely absent from the conversation, ask the user for that missing value."
                }),
            ));
        }
        let storage = self.runtime_storage()?;
        let id = Self::capability_string_argument(arguments, "id")
            .or_else(|| Self::capability_string_argument(arguments, "channel_id"))
            .ok_or_else(|| anyhow::anyhow!("custom_messaging_channel_manage requires id"))?;
        let operation = Self::capability_string_argument(arguments, "operation")
            .map(|value| value.to_ascii_lowercase())
            .ok_or_else(|| anyhow::anyhow!("custom_messaging_channel_manage requires operation"))?;

        match operation.as_str() {
            "delete" => {
                crate::custom_messaging_channels::delete_custom_messaging_channel(
                    &storage,
                    &self.config_dir,
                    self.data_dir(),
                    &id,
                )
                .await?;
                self.record_security_event(
                    "custom_messaging_channel_delete",
                    "medium",
                    format!(
                        "Custom messaging channel deleted by runtime action. channel_id={}",
                        id
                    ),
                    Some(format!(
                        "actor=runtime_action;source_kind=custom_channel;channel_id={}",
                        id
                    )),
                )
                .await;
                Ok(serde_json::to_string_pretty(&serde_json::json!({
                    "status": "deleted",
                    "integration_type": "custom_messaging_channel",
                    "id": id,
                }))?)
            }
            "test" => {
                let result = crate::custom_messaging_channels::test_custom_messaging_channel(
                    &storage,
                    &self.config_dir,
                    self.data_dir(),
                    &id,
                )
                .await?;
                Ok(serde_json::to_string_pretty(&serde_json::json!({
                    "status": if result.ok { "ok" } else { "error" },
                    "integration_type": "custom_messaging_channel",
                    "id": id,
                    "test": result,
                }))?)
            }
            "enable" | "disable" => {
                let Some(existing) =
                    crate::custom_messaging_channels::get_custom_messaging_channel_config(
                        &storage, &id,
                    )
                    .await?
                else {
                    anyhow::bail!("Custom messaging channel '{}' was not found", id);
                };
                let enabled = operation == "enable";
                let view = crate::custom_messaging_channels::upsert_custom_messaging_channel(
                    &storage,
                    &self.config_dir,
                    self.data_dir(),
                    crate::custom_messaging_channels::CustomMessagingChannelUpsertRequest {
                        id: Some(existing.id.clone()),
                        name: existing.name.clone(),
                        description: Some(existing.description.clone()),
                        enabled: Some(enabled),
                        docs_url: existing.docs_url.clone(),
                        send: existing.send.clone(),
                        auth_manifest: existing.auth_manifest.clone(),
                        auth_profile_id: existing.auth_profile_id.clone(),
                        credential_fields: Vec::new(),
                        clear_secrets: None,
                    },
                    Some(&existing.id),
                )
                .await?;
                let config_id = view.config.id.clone();
                let runtime_channel_id = view.runtime_channel_id.clone();
                let configured = view.configured;
                self.record_security_event(
                    if enabled {
                        "custom_messaging_channel_enable"
                    } else {
                        "custom_messaging_channel_disable"
                    },
                    "medium",
                    format!(
                        "Custom messaging channel {} by runtime action. channel_id={}",
                        operation, view.id
                    ),
                    Some(format!(
                        "actor=runtime_action;source_kind=custom_channel;channel_id={}",
                        view.id
                    )),
                )
                .await;
                Ok(serde_json::to_string_pretty(&serde_json::json!({
                    "status": if enabled { "enabled" } else { "disabled" },
                    "integration_type": "custom_messaging_channel",
                    "id": config_id,
                    "channel_id": runtime_channel_id,
                    "configured": configured,
                }))?)
            }
            _ => anyhow::bail!(
                "Unsupported custom messaging channel management operation '{}'",
                operation
            ),
        }
    }

    pub(in crate::runtime) async fn execute_extension_pack_connect(
        &self,
        arguments: &serde_json::Value,
    ) -> Result<String> {
        let pack_id = arguments
            .get("pack_id")
            .and_then(|value| value.as_str())
            .map(str::trim)
            .filter(|value| !value.is_empty())
            .ok_or_else(|| anyhow::anyhow!("Missing pack_id"))?;
        let request: crate::extension_packs::ExtensionPackConnectionUpsertRequest =
            serde_json::from_value(arguments.clone()).map_err(|error| {
                anyhow::anyhow!("Invalid extension pack connect arguments: {}", error)
            })?;
        let registry = self
            .extension_pack_registry
            .clone()
            .ok_or_else(|| anyhow::anyhow!("Extension-pack registry not available"))?;
        let (connection, oauth_hint) = {
            let mut guard = registry.write().await;
            let connection = guard.upsert_connection(pack_id, request).await?;
            let oauth_hint = guard.supports_connect_url(pack_id);
            (connection, oauth_hint)
        };
        self.sync_extension_pack_runtime_actions().await?;
        let redirect_uri = arguments
            .get("redirect_uri")
            .and_then(|value| value.as_str())
            .map(str::trim)
            .filter(|value| !value.is_empty());
        let connect_path = if oauth_hint {
            let mut path = format!(
                "/extension-packs/{}/connect-url",
                urlencoding::encode(pack_id)
            );
            if let Some(redirect_uri) = redirect_uri {
                let suffix = format!("redirect_uri={}", urlencoding::encode(redirect_uri));
                path.push('?');
                path.push_str(&suffix);
            }
            Some(path)
        } else {
            None
        };
        let connect_url = connect_path.as_deref().map(|path| {
            format!(
                "{}{}",
                crate::core::runtime::net::internal_api_base_url(),
                path
            )
        });
        let (pack_name, required_secrets) = {
            let guard = registry.read().await;
            let pack = guard.get_pack(pack_id).await?;
            let pack_name = pack
                .as_ref()
                .map(|pack| pack.manifest.name.clone())
                .unwrap_or_else(|| pack_id.to_string());
            let required_secrets = pack
                .as_ref()
                .map(|pack| {
                    pack.manifest
                        .auth
                        .required_secrets
                        .iter()
                        .map(|value| value.trim())
                        .filter(|value| !value.is_empty())
                        .map(str::to_string)
                        .collect::<Vec<_>>()
                })
                .filter(|items| !items.is_empty())
                .unwrap_or_else(|| match connection.auth_mode {
                    crate::extension_packs::ExtensionPackAuthMode::ApiKey => {
                        vec!["api_key".to_string(), "access_token".to_string()]
                    }
                    crate::extension_packs::ExtensionPackAuthMode::Basic => {
                        vec!["username".to_string(), "password".to_string()]
                    }
                    _ => Vec::new(),
                });
            (pack_name, required_secrets)
        };
        let needs_credentials = !oauth_hint
            && matches!(
                connection.state,
                crate::extension_packs::ExtensionConnectionState::NeedsAuth
            )
            && !required_secrets.is_empty();
        let settings_path = format!(
            "Settings > Integrations > Extension Pack Integrations > {}",
            pack_name
        );
        let message = if needs_credentials {
            format!(
                "Connection draft saved. Credentials are still required. Use the secure credential form in chat or open {}. Never paste secrets, API keys, passwords, or sensitive data into normal chat.",
                settings_path
            )
        } else if oauth_hint {
            "Connection record saved. Complete OAuth by opening the returned connect_url in a browser."
            .to_string()
        } else {
            "Connection saved.".to_string()
        };
        let credential_request = if needs_credentials {
            Some(serde_json::json!({
                "kind": "extension_pack_connection",
                "pack_id": pack_id,
                "pack_name": pack_name.clone(),
                "connection_id": connection.connection.id.clone(),
                "required_keys": required_secrets.clone(),
                "settings_path": settings_path.clone(),
                "secure_input_required": true
            }))
        } else {
            None
        };
        // Structured completion (top-level tool + status) so the legacy tool
        // wrapper preserves needs_credentials verbatim and the spine surfaces
        // the secure-credential handoff instead of burying it as "completed".
        Ok(structured_tool_completion_output(
            "extension_pack_connect",
            if needs_credentials {
                "needs_credentials"
            } else if oauth_hint {
                "oauth_pending"
            } else {
                "connected"
            },
            message.clone(),
            serde_json::json!({
                "pack_id": pack_id,
                "pack_name": pack_name,
                "connection": connection,
                "required_secrets": required_secrets,
                "oauth_connect_in_ui": oauth_hint,
                "connect_url_endpoint": connect_path,
                "connect_url": connect_url,
                "settings_path": settings_path,
                "credential_request": credential_request,
                "message": message
            }),
        ))
    }

    pub(in crate::runtime) async fn execute_extension_pack_set_enabled(
        &self,
        arguments: &serde_json::Value,
    ) -> Result<String> {
        let pack_id = arguments
            .get("pack_id")
            .and_then(|value| value.as_str())
            .map(str::trim)
            .filter(|value| !value.is_empty())
            .ok_or_else(|| anyhow::anyhow!("Missing pack_id"))?;
        let enabled = arguments
            .get("enabled")
            .and_then(|value| value.as_bool())
            .ok_or_else(|| anyhow::anyhow!("Missing enabled"))?;
        let registry = self
            .extension_pack_registry
            .clone()
            .ok_or_else(|| anyhow::anyhow!("Extension-pack registry not available"))?;
        let pack = {
            let mut guard = registry.write().await;
            guard.set_pack_enabled(pack_id, enabled).await?
        };
        self.sync_extension_pack_runtime_actions().await?;
        Ok(serde_json::to_string_pretty(&pack)?)
    }

    pub(in crate::runtime) async fn execute_extension_pack_delete(
        &self,
        arguments: &serde_json::Value,
    ) -> Result<String> {
        let pack_id = arguments
            .get("pack_id")
            .and_then(|value| value.as_str())
            .map(str::trim)
            .filter(|value| !value.is_empty())
            .ok_or_else(|| anyhow::anyhow!("Missing pack_id"))?;
        let remove_connections = arguments
            .get("remove_connections")
            .and_then(|value| value.as_bool())
            .unwrap_or(true);
        let registry = self
            .extension_pack_registry
            .clone()
            .ok_or_else(|| anyhow::anyhow!("Extension-pack registry not available"))?;
        {
            let mut guard = registry.write().await;
            guard.delete_pack(pack_id, remove_connections).await?
        };
        self.sync_extension_pack_runtime_actions().await?;
        Ok(serde_json::to_string_pretty(&serde_json::json!({
            "status": "deleted",
            "pack_id": pack_id,
            "remove_connections": remove_connections,
        }))?)
    }

    pub(in crate::runtime) async fn execute_extension_pack_runtime_install(
        &self,
        arguments: &serde_json::Value,
    ) -> Result<String> {
        let pack_id = arguments
            .get("pack_id")
            .and_then(|value| value.as_str())
            .map(str::trim)
            .filter(|value| !value.is_empty())
            .ok_or_else(|| anyhow::anyhow!("Missing pack_id"))?;
        let registry = self
            .extension_pack_registry
            .clone()
            .ok_or_else(|| anyhow::anyhow!("Extension-pack registry not available"))?;
        let result = {
            let mut guard = registry.write().await;
            guard.install_runtime(pack_id).await?
        };
        self.sync_extension_pack_runtime_actions().await?;
        Ok(serde_json::to_string_pretty(&result)?)
    }

    pub(in crate::runtime) async fn execute_extension_pack_runtime_verify(
        &self,
        arguments: &serde_json::Value,
    ) -> Result<String> {
        let pack_id = arguments
            .get("pack_id")
            .and_then(|value| value.as_str())
            .map(str::trim)
            .filter(|value| !value.is_empty())
            .ok_or_else(|| anyhow::anyhow!("Missing pack_id"))?;
        let registry = self
            .extension_pack_registry
            .clone()
            .ok_or_else(|| anyhow::anyhow!("Extension-pack registry not available"))?;
        let result = {
            let mut guard = registry.write().await;
            guard.verify_runtime(pack_id).await?
        };
        self.sync_extension_pack_runtime_actions().await?;
        Ok(serde_json::to_string_pretty(&result)?)
    }

    pub(in crate::runtime) async fn execute_extension_pack_runtime_update(
        &self,
        arguments: &serde_json::Value,
    ) -> Result<String> {
        let pack_id = arguments
            .get("pack_id")
            .and_then(|value| value.as_str())
            .map(str::trim)
            .filter(|value| !value.is_empty())
            .ok_or_else(|| anyhow::anyhow!("Missing pack_id"))?;
        let registry = self
            .extension_pack_registry
            .clone()
            .ok_or_else(|| anyhow::anyhow!("Extension-pack registry not available"))?;
        let result = {
            let mut guard = registry.write().await;
            guard.update_runtime(pack_id).await?
        };
        self.sync_extension_pack_runtime_actions().await?;
        Ok(serde_json::to_string_pretty(&result)?)
    }

    pub(in crate::runtime) async fn execute_extension_pack_runtime_uninstall(
        &self,
        arguments: &serde_json::Value,
    ) -> Result<String> {
        let pack_id = arguments
            .get("pack_id")
            .and_then(|value| value.as_str())
            .map(str::trim)
            .filter(|value| !value.is_empty())
            .ok_or_else(|| anyhow::anyhow!("Missing pack_id"))?;
        let registry = self
            .extension_pack_registry
            .clone()
            .ok_or_else(|| anyhow::anyhow!("Extension-pack registry not available"))?;
        let result = {
            let mut guard = registry.write().await;
            guard.uninstall_runtime(pack_id).await?
        };
        self.sync_extension_pack_runtime_actions().await?;
        Ok(serde_json::to_string_pretty(&result)?)
    }

    pub(in crate::runtime) async fn execute_extension_pack_test_connection(
        &self,
        arguments: &serde_json::Value,
    ) -> Result<String> {
        let pack_id = arguments
            .get("pack_id")
            .and_then(|value| value.as_str())
            .map(str::trim)
            .filter(|value| !value.is_empty())
            .ok_or_else(|| anyhow::anyhow!("Missing pack_id"))?;
        let requested_connection_id = arguments
            .get("connection_id")
            .and_then(|value| value.as_str())
            .map(str::trim)
            .filter(|value| !value.is_empty())
            .map(str::to_string);
        let registry = self
            .extension_pack_registry
            .clone()
            .ok_or_else(|| anyhow::anyhow!("Extension-pack registry not available"))?;
        let mut guard = registry.write().await;
        let pick_connection_id =
            |connections: Vec<crate::extension_packs::ExtensionPackConnectionView>| {
                connections
                    .iter()
                    .find(|item| {
                        item.connection.enabled
                            && matches!(
                                item.state,
                                crate::extension_packs::ExtensionConnectionState::Ready
                            )
                    })
                    .or_else(|| connections.iter().find(|item| item.connection.enabled))
                    .or_else(|| connections.first())
                    .map(|item| item.connection.id.clone())
            };
        let resolved_connection_id = if let Some(connection_id) = requested_connection_id.clone() {
            connection_id
        } else {
            pick_connection_id(guard.list_connections(pack_id).await?).ok_or_else(|| {
                anyhow::anyhow!("No connection is configured for pack '{}'", pack_id)
            })?
        };
        let (resolved_connection_id, result) = match guard
            .test_connection(
                pack_id,
                &resolved_connection_id,
                self.mcp_registry.clone(),
                self.plugin_registry.clone(),
            )
            .await
        {
            Ok(result) => (resolved_connection_id, result),
            Err(error)
                if requested_connection_id
                    .as_deref()
                    .is_some_and(|value| value.eq_ignore_ascii_case(pack_id))
                    && error
                        .to_string()
                        .contains(&format!("Connection '{}' was not found", pack_id)) =>
            {
                let fallback_id = pick_connection_id(guard.list_connections(pack_id).await?)
                    .ok_or_else(|| {
                        anyhow::anyhow!("No connection is configured for pack '{}'", pack_id)
                    })?;
                let result = guard
                    .test_connection(
                        pack_id,
                        &fallback_id,
                        self.mcp_registry.clone(),
                        self.plugin_registry.clone(),
                    )
                    .await?;
                (fallback_id, result)
            }
            Err(error) => return Err(error),
        };
        Ok(serde_json::to_string_pretty(&serde_json::json!({
            "resolved_connection_id": resolved_connection_id,
            "result": result,
        }))?)
    }

    pub(in crate::runtime) async fn execute_extension_pack_list_events(
        &self,
        arguments: &serde_json::Value,
    ) -> Result<String> {
        let pack_id = arguments
            .get("pack_id")
            .and_then(|value| value.as_str())
            .map(str::trim)
            .filter(|value| !value.is_empty())
            .ok_or_else(|| anyhow::anyhow!("Missing pack_id"))?;
        let limit = arguments
            .get("limit")
            .and_then(|value| value.as_u64())
            .unwrap_or(25) as usize;
        let registry = self
            .extension_pack_registry
            .clone()
            .ok_or_else(|| anyhow::anyhow!("Extension-pack registry not available"))?;
        let guard = registry.read().await;
        Ok(serde_json::to_string_pretty(
            &guard.list_events(pack_id, limit).await?,
        )?)
    }

    pub(in crate::runtime) async fn execute_extension_pack_invoke(
        &self,
        arguments: &serde_json::Value,
    ) -> Result<String> {
        let request: crate::extension_packs::ExtensionPackInvokeRequest =
            serde_json::from_value(arguments.clone()).map_err(|error| {
                anyhow::anyhow!("Invalid extension pack invoke arguments: {}", error)
            })?;
        let registry = self
            .extension_pack_registry
            .clone()
            .ok_or_else(|| anyhow::anyhow!("Extension-pack registry not available"))?;
        let mut guard = registry.write().await;
        let result = guard
            .invoke_feature(
                request,
                self.mcp_registry.clone(),
                self.plugin_registry.clone(),
            )
            .await?;
        Ok(serde_json::to_string_pretty(&result)?)
    }
}
