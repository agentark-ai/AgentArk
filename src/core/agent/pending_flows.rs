#![allow(dead_code)]

use super::*;

impl Agent {
    pub(super) fn integration_enabled_key(id: &str) -> String {
        format!("integration_enabled:{}", id)
    }

    pub(super) fn pending_integration_connect_key(conversation_id: &str) -> String {
        format!("pending_integration_connect:{}", conversation_id.trim())
    }

    pub(super) fn pending_secret_followup_key(conversation_id: &str) -> String {
        format!("pending_secret_followup:{}", conversation_id.trim())
    }

    pub(super) fn pending_chat_credential_prompt_key(conversation_id: &str) -> String {
        format!("pending_chat_credential_prompt:{}", conversation_id.trim())
    }

    pub(super) async fn persist_encrypted_json<T: serde::Serialize>(&self, key: &str, value: &T) {
        let Ok(encoded) = serde_json::to_vec(value) else {
            return;
        };
        let _ = self.storage.set_encrypted(key, &encoded).await;
    }

    pub(super) async fn load_encrypted_json<T: serde::de::DeserializeOwned>(
        &self,
        key: &str,
    ) -> Option<T> {
        let raw = self.storage.get_encrypted(key).await.ok().flatten()?;
        serde_json::from_slice(&raw).ok()
    }

    pub(super) async fn load_pending_integration_connect_flow(
        &self,
        conversation_id: &str,
    ) -> Option<crate::core::connect_flow::PendingIntegrationConnect> {
        if conversation_id.trim().is_empty() {
            return None;
        }
        if let Some(flow) = self
            .integration_connect_flows
            .read()
            .await
            .get(conversation_id)
            .cloned()
        {
            return Some(flow);
        }
        let key = Self::pending_integration_connect_key(conversation_id);
        let flow = self
            .load_encrypted_json::<crate::core::connect_flow::PendingIntegrationConnect>(&key)
            .await?;
        self.integration_connect_flows
            .write()
            .await
            .insert(conversation_id.to_string(), flow.clone());
        Some(flow)
    }

    pub(super) async fn clear_pending_integration_connect_flow(&self, conversation_id: &str) {
        if conversation_id.trim().is_empty() {
            return;
        }
        self.integration_connect_flows
            .write()
            .await
            .remove(conversation_id);
        let key = Self::pending_integration_connect_key(conversation_id);
        let _ = self.storage.delete(&key).await;
    }

    pub(super) async fn continue_integration_connect_flow_after_secret_save(
        &self,
        conversation_id: &str,
    ) -> Option<String> {
        let flow = self
            .load_pending_integration_connect_flow(conversation_id)
            .await?;

        // TTL cleanup (covers "user navigated away" cases).
        let now = chrono::Utc::now();
        if (now - flow.started_at).num_seconds() > crate::core::connect_flow::CONNECT_FLOW_TTL_SECS
        {
            self.clear_pending_integration_connect_flow(conversation_id)
                .await;
            return Some(
                "Setup expired due to inactivity. If you still want to connect an integration, request that setup again."
                    .to_string(),
            );
        }

        let spec = match crate::core::connect_flow::spec_by_id(&flow.integration_id) {
            Some(s) => s,
            None => {
                self.clear_pending_integration_connect_flow(conversation_id)
                    .await;
                return Some("Setup canceled (unknown integration).".to_string());
            }
        };

        let mgr = crate::core::config::SecureConfigManager::new_with_data_dir(
            &self.config_dir,
            Some(&self.data_dir),
        )
        .ok()?;

        let secret_present = |user_key: &str| -> bool {
            for storage_key in crate::core::secrets::storage_keys_for_user_key(user_key) {
                if let Ok(Some(v)) = mgr.get_custom_secret(&storage_key) {
                    if !v.trim().is_empty() {
                        return true;
                    }
                }
            }
            false
        };

        let required_ok = match spec.required.kind {
            crate::core::connect_flow::SecretRequirementKind::All => {
                spec.required.keys.iter().all(|k| secret_present(k))
            }
            crate::core::connect_flow::SecretRequirementKind::Any => {
                spec.required.keys.iter().any(|k| secret_present(k))
            }
        };

        if !required_ok {
            match spec.required.kind {
                crate::core::connect_flow::SecretRequirementKind::All => {
                    let missing: Vec<&str> = spec
                        .required
                        .keys
                        .iter()
                        .copied()
                        .filter(|k| !secret_present(k))
                        .collect();
                    if missing.is_empty() {
                        return None;
                    }
                    return Some(format!(
                        "Saved. Still missing required secret(s): {}",
                        missing
                            .into_iter()
                            .map(|k| format!("`{}`", k))
                            .collect::<Vec<_>>()
                            .join(", ")
                    ));
                }
                crate::core::connect_flow::SecretRequirementKind::Any => {
                    return Some(format!(
                        "Saved. Provide at least one of: {}",
                        spec.required
                            .keys
                            .iter()
                            .map(|k| format!("`{}`", k))
                            .collect::<Vec<_>>()
                            .join(", ")
                    ));
                }
            }
        }

        let integration = match self.integrations.get(spec.id) {
            Some(i) => i,
            None => {
                self.clear_pending_integration_connect_flow(conversation_id)
                    .await;
                return Some(format!("Integration '{}' not found.", spec.id));
            }
        };

        let status = integration.status().await;
        match status {
            crate::integrations::IntegrationStatus::Connected => {
                let _ = mgr.set_custom_secret(
                    &Self::integration_enabled_key(spec.id),
                    Some("true".to_string()),
                );
                self.clear_pending_integration_connect_flow(conversation_id)
                    .await;
                Some(format!("Connected and enabled {}.", spec.name))
            }
            crate::integrations::IntegrationStatus::Error(e) => {
                let _ = mgr.set_custom_secret(
                    &Self::integration_enabled_key(spec.id),
                    Some("false".to_string()),
                );
                Some(format!(
                    "Connection test failed for {}: {}. Retry by updating the credentials in the secure form or Settings.",
                    spec.name, e
                ))
            }
            crate::integrations::IntegrationStatus::NeedsAuth => {
                let _ = mgr.set_custom_secret(
                    &Self::integration_enabled_key(spec.id),
                    Some("false".to_string()),
                );
                self.clear_pending_integration_connect_flow(conversation_id)
                    .await;
                Some(format!(
                    "{} needs OAuth authorization. Use the web UI Integrations page to complete OAuth, then enable it.",
                    spec.name
                ))
            }
            crate::integrations::IntegrationStatus::NotConfigured => {
                let _ = mgr.set_custom_secret(
                    &Self::integration_enabled_key(spec.id),
                    Some("false".to_string()),
                );
                Some(format!(
                    "{} is still not configured. Double-check the required secret keys and try again.",
                    spec.name
                ))
            }
        }
    }

    pub(super) async fn remember_pending_secret_followup(
        &self,
        conversation_id: &str,
        kind: PendingSecretFollowupKind,
    ) {
        if conversation_id.trim().is_empty() {
            return;
        }
        let pending_followup = PendingSecretFollowup {
            kind,
            requested_at: chrono::Utc::now(),
        };
        self.pending_secret_followups
            .write()
            .await
            .insert(conversation_id.to_string(), pending_followup.clone());
        let key = Self::pending_secret_followup_key(conversation_id);
        self.persist_encrypted_json(&key, &pending_followup).await;
    }

    pub(super) async fn clear_pending_secret_followup(&self, conversation_id: &str) {
        if conversation_id.trim().is_empty() {
            return;
        }
        self.pending_secret_followups
            .write()
            .await
            .remove(conversation_id);
        let key = Self::pending_secret_followup_key(conversation_id);
        let _ = self.storage.delete(&key).await;
    }

    pub(super) async fn load_pending_secret_followup(
        &self,
        conversation_id: &str,
    ) -> Option<PendingSecretFollowup> {
        if conversation_id.trim().is_empty() {
            return None;
        }
        if let Some(pending) = self
            .pending_secret_followups
            .read()
            .await
            .get(conversation_id)
            .cloned()
        {
            return Some(pending);
        }
        let key = Self::pending_secret_followup_key(conversation_id);
        let pending = self
            .load_encrypted_json::<PendingSecretFollowup>(&key)
            .await?;
        self.pending_secret_followups
            .write()
            .await
            .insert(conversation_id.to_string(), pending.clone());
        Some(pending)
    }

    pub(super) fn chat_credential_prompt_is_expired(
        requested_at: chrono::DateTime<chrono::Utc>,
        ttl: chrono::Duration,
    ) -> bool {
        (chrono::Utc::now() - requested_at) > ttl
    }

    pub(super) fn chat_credential_field_label(key: &str) -> String {
        let trimmed = key.trim();
        if trimmed.is_empty() {
            return "Secret".to_string();
        }
        let parts = trimmed
            .split(|ch: char| matches!(ch, '_' | '-' | '.' | ':'))
            .filter(|part| !part.trim().is_empty())
            .collect::<Vec<_>>();
        if parts.is_empty() {
            return trimmed.to_string();
        }
        parts
            .into_iter()
            .map(|part| {
                let lower = part.to_ascii_lowercase();
                match lower.as_str() {
                    "api" => "API".to_string(),
                    "id" => "ID".to_string(),
                    "oauth" => "OAuth".to_string(),
                    "url" => "URL".to_string(),
                    "uri" => "URI".to_string(),
                    _ => {
                        let mut chars = lower.chars();
                        match chars.next() {
                            Some(first) => {
                                let mut out = String::new();
                                out.extend(first.to_uppercase());
                                out.push_str(chars.as_str());
                                out
                            }
                            None => String::new(),
                        }
                    }
                }
            })
            .collect::<Vec<_>>()
            .join(" ")
    }

    pub(super) async fn remember_pending_chat_credential_prompt(
        &self,
        conversation_id: &str,
        kind: PendingChatCredentialPromptKind,
    ) {
        if conversation_id.trim().is_empty() {
            return;
        }
        let pending = PendingChatCredentialPrompt {
            kind,
            requested_at: chrono::Utc::now(),
        };
        self.pending_chat_credential_prompts
            .write()
            .await
            .insert(conversation_id.to_string(), pending.clone());
        let key = Self::pending_chat_credential_prompt_key(conversation_id);
        self.persist_encrypted_json(&key, &pending).await;
    }

    pub(super) async fn clear_pending_chat_credential_prompt(&self, conversation_id: &str) {
        if conversation_id.trim().is_empty() {
            return;
        }
        self.pending_chat_credential_prompts
            .write()
            .await
            .remove(conversation_id);
        let key = Self::pending_chat_credential_prompt_key(conversation_id);
        let _ = self.storage.delete(&key).await;
    }

    pub async fn dismiss_chat_credential_prompt(&self, conversation_id: &str) {
        self.clear_pending_chat_credential_prompt(conversation_id)
            .await;
    }

    pub(super) async fn load_pending_chat_credential_prompt(
        &self,
        conversation_id: &str,
    ) -> Option<PendingChatCredentialPrompt> {
        if conversation_id.trim().is_empty() {
            return None;
        }
        if let Some(pending) = self
            .pending_chat_credential_prompts
            .read()
            .await
            .get(conversation_id)
            .cloned()
        {
            return Some(pending);
        }
        let key = Self::pending_chat_credential_prompt_key(conversation_id);
        let pending = self
            .load_encrypted_json::<PendingChatCredentialPrompt>(&key)
            .await?;
        self.pending_chat_credential_prompts
            .write()
            .await
            .insert(conversation_id.to_string(), pending.clone());
        Some(pending)
    }

    pub async fn remember_extension_pack_chat_credential_prompt(
        &self,
        conversation_id: &str,
        pack_id: &str,
        pack_name: &str,
        connection_id: &str,
        required_keys: &[String],
    ) {
        let keys = required_keys
            .iter()
            .map(|value| value.trim())
            .filter(|value| !value.is_empty())
            .map(str::to_string)
            .collect::<Vec<_>>();
        if keys.is_empty() {
            return;
        }
        self.remember_pending_chat_credential_prompt(
            conversation_id,
            PendingChatCredentialPromptKind::ExtensionPackConnection {
                pack_id: pack_id.trim().to_string(),
                pack_name: pack_name.trim().to_string(),
                connection_id: connection_id.trim().to_string(),
                required_keys: keys,
            },
        )
        .await;
    }

    pub async fn clear_extension_pack_chat_credential_prompt(&self, conversation_id: &str) {
        self.clear_pending_chat_credential_prompt(conversation_id)
            .await;
    }

    /// Remember a manifest-driven credential prompt for the given conversation.
    /// Shares the same `pending_chat_credential_prompts` map as the extension-
    /// pack variant so there's a single inline UI surface and a single
    /// pause/resume path (see [`Self::on_secret_saved_followup`]).
    pub async fn remember_integration_auth_chat_prompt(
        &self,
        conversation_id: &str,
        integration_id: &str,
        tool_name: Option<&str>,
        trace_id: Option<&str>,
    ) {
        let trimmed_id = integration_id.trim();
        if trimmed_id.is_empty() {
            return;
        }
        let origin = match tool_name {
            Some(name) => IntegrationAuthPromptOrigin::ToolRuntime {
                tool_name: Some(name.to_string()),
                trace_id: trace_id.map(|value| value.to_string()),
            },
            None => IntegrationAuthPromptOrigin::InstallIntent,
        };
        self.remember_pending_chat_credential_prompt(
            conversation_id,
            PendingChatCredentialPromptKind::IntegrationAuth {
                integration_id: trimmed_id.to_string(),
                origin,
            },
        )
        .await;
    }

    pub async fn remember_raw_secret_chat_prompt(
        &self,
        conversation_id: &str,
        key: &str,
        tool_name: Option<&str>,
        trace_id: Option<&str>,
    ) {
        let trimmed_key = key.trim();
        if trimmed_key.is_empty() {
            return;
        }
        self.remember_pending_chat_credential_prompt(
            conversation_id,
            PendingChatCredentialPromptKind::RawSecret {
                key: trimmed_key.to_string(),
                origin: IntegrationAuthPromptOrigin::ToolRuntime {
                    tool_name: tool_name.map(|value| value.to_string()),
                    trace_id: trace_id.map(|value| value.to_string()),
                },
            },
        )
        .await;
    }

    pub async fn remember_mcp_server_auth_chat_prompt(
        &self,
        conversation_id: &str,
        server_id: &str,
        server_name: &str,
        auth_type: &str,
        auth_name: Option<&str>,
        settings_path: Option<&str>,
    ) {
        let server_id = server_id.trim();
        if conversation_id.trim().is_empty() || server_id.is_empty() {
            return;
        }
        self.remember_pending_chat_credential_prompt(
            conversation_id,
            PendingChatCredentialPromptKind::McpServerAuth {
                server_id: server_id.to_string(),
                server_name: server_name.trim().to_string(),
                auth_type: auth_type.trim().to_ascii_lowercase(),
                auth_name: auth_name
                    .map(str::trim)
                    .filter(|value| !value.is_empty())
                    .map(str::to_string),
                settings_path: settings_path
                    .map(str::trim)
                    .filter(|value| !value.is_empty())
                    .map(str::to_string),
            },
        )
        .await;
    }

    pub async fn remember_custom_api_auth_chat_prompt(
        &self,
        conversation_id: &str,
        api_id: &str,
        api_name: &str,
        auth_mode: &str,
        auth_name: Option<&str>,
        settings_path: Option<&str>,
    ) {
        let api_id = api_id.trim();
        if conversation_id.trim().is_empty() || api_id.is_empty() {
            return;
        }
        self.remember_pending_chat_credential_prompt(
            conversation_id,
            PendingChatCredentialPromptKind::CustomApiAuth {
                api_id: api_id.to_string(),
                api_name: api_name.trim().to_string(),
                auth_mode: auth_mode.trim().to_ascii_lowercase(),
                auth_name: auth_name
                    .map(str::trim)
                    .filter(|value| !value.is_empty())
                    .map(str::to_string),
                settings_path: settings_path
                    .map(str::trim)
                    .filter(|value| !value.is_empty())
                    .map(str::to_string),
            },
        )
        .await;
    }

    /// Backing implementation for the `IntegrationAuth` submit path. Writes
    /// each submitted form-field value into the manifest's declared
    /// storage_targets, invokes the shared resume hook
    /// ([`Self::on_secret_saved_followup`]) so any waiting run picks up the
    /// new secrets, then clears the pending prompt. OAuth / Hybrid modes that
    /// still require a browser redirect to complete are not yet wired - we
    /// store any form fields the user already provided and surface a clear
    /// message indicating the launch step is queued for a follow-up.
    pub(super) async fn submit_pending_integration_auth_credentials(
        &self,
        conversation_id: &str,
        integration_id: String,
        values: &std::collections::BTreeMap<String, String>,
    ) -> Result<String> {
        use crate::core::integration_auth::{AuthMode, IntegrationAuthManifest};

        let manifest: IntegrationAuthManifest =
            match self.lookup_integration_auth_manifest(&integration_id).await {
                Some(manifest) => manifest,
                None => {
                    anyhow::bail!("No auth manifest is registered for this credential prompt.");
                }
            };

        let form_fields = match &manifest.mode {
            AuthMode::Secrets { fields } | AuthMode::Hybrid { fields, .. } => fields.clone(),
            AuthMode::OAuth2AuthorizationCode(_) | AuthMode::OAuth2DeviceCode(_) => Vec::new(),
        };

        let mut stored_count = 0usize;
        for field in &form_fields {
            if let Some(value) = values.get(&field.key).map(|value| value.trim()) {
                if value.is_empty() {
                    if field.required {
                        anyhow::bail!("Field `{}` is required.", field.key);
                    }
                    continue;
                }
                if let Some(validation) = field.validation.as_ref() {
                    if let Some(min) = validation.min_len {
                        if value.len() < min {
                            anyhow::bail!(
                                "Field `{}` must be at least {} characters.",
                                field.key,
                                min
                            );
                        }
                    }
                    if let Some(max) = validation.max_len {
                        if value.len() > max {
                            anyhow::bail!(
                                "Field `{}` must be at most {} characters.",
                                field.key,
                                max
                            );
                        }
                    }
                    if let Some(prefix) = validation.must_start_with.as_ref() {
                        if !value.starts_with(prefix.as_str()) {
                            anyhow::bail!("Field `{}` must start with `{}`.", field.key, prefix);
                        }
                    }
                }
                for target in &field.storage_targets {
                    crate::core::secrets::store_user_secret(
                        &self.config_dir,
                        Some(&self.data_dir),
                        target,
                        value,
                    )?;
                }
                stored_count += 1;
            } else if field.required {
                anyhow::bail!("Field `{}` is required.", field.key);
            }
        }

        let needs_oauth_launch = matches!(
            &manifest.mode,
            AuthMode::OAuth2AuthorizationCode(_)
                | AuthMode::OAuth2DeviceCode(_)
                | AuthMode::Hybrid { .. }
        );
        let mut response = if stored_count > 0 {
            format!(
                "Saved {} credential field(s) for {} securely. None of the values were sent to the assistant.",
                stored_count, manifest.display_name
            )
        } else {
            format!(
                "No new credential values were stored for {}.",
                manifest.display_name
            )
        };
        if needs_oauth_launch {
            response.push_str(
                "\n\nThe OAuth browser step for this integration is queued for a follow-up - the form values you just entered are saved.",
            );
        } else {
            self.clear_pending_chat_credential_prompt(conversation_id)
                .await;
            if let Some(followup) = self.on_secret_saved_followup(conversation_id).await {
                response.push_str("\n\n");
                response.push_str(&followup);
            }
        }
        Ok(response)
    }

    pub(super) async fn submit_pending_raw_secret_credentials(
        &self,
        conversation_id: &str,
        key: String,
        values: &std::collections::BTreeMap<String, String>,
    ) -> Result<String> {
        use crate::core::integration_auth::AuthMode;

        let manifest = crate::core::integration_auth::raw_key_manifest(&key);
        let AuthMode::Secrets { fields } = manifest.mode else {
            anyhow::bail!("Raw secret prompt is not configured as a secret form.");
        };
        let Some(field) = fields.first() else {
            anyhow::bail!("Raw secret prompt has no fields.");
        };
        let value = values
            .get(&field.key)
            .map(|value| value.trim())
            .filter(|value| !value.is_empty())
            .ok_or_else(|| anyhow::anyhow!("Field `{}` is required.", field.key))?;
        for target in &field.storage_targets {
            crate::core::secrets::store_user_secret(
                &self.config_dir,
                Some(&self.data_dir),
                target,
                value,
            )?;
        }
        self.clear_pending_chat_credential_prompt(conversation_id)
            .await;
        let mut response =
            "Saved the credential securely. The value was not sent to the assistant.".to_string();
        if let Some(followup) = self.on_secret_saved_followup(conversation_id).await {
            response.push_str("\n\n");
            response.push_str(&followup);
        }
        Ok(response)
    }

    pub(super) async fn submit_pending_mcp_server_auth_credentials(
        &self,
        conversation_id: &str,
        server_id: String,
        values: &std::collections::BTreeMap<String, String>,
    ) -> Result<String> {
        let manager = crate::core::config::SecureConfigManager::new_with_data_dir(
            &self.config_dir,
            Some(&self.data_dir),
        )?;
        let config = manager.load()?;
        let server = config
            .mcp
            .servers
            .iter()
            .find(|server| server.id == server_id)
            .cloned()
            .ok_or_else(|| anyhow::anyhow!("MCP server `{}` was not found.", server_id))?;
        let Some(auth) = server.auth.as_ref() else {
            anyhow::bail!("MCP server `{}` does not require stored auth.", server.name);
        };

        let mut secret = manager
            .load_secrets()?
            .mcp_auth
            .get(&server.id)
            .cloned()
            .unwrap_or_default();
        match auth {
            crate::core::config::McpAuthConfig::Bearer { .. }
            | crate::core::config::McpAuthConfig::Header { .. }
            | crate::core::config::McpAuthConfig::Query { .. } => {
                let token = values
                    .get("token")
                    .or_else(|| values.get("value"))
                    .map(|value| value.trim())
                    .filter(|value| !value.is_empty())
                    .ok_or_else(|| anyhow::anyhow!("Token is required."))?;
                secret.token = Some(token.to_string());
            }
            crate::core::config::McpAuthConfig::Basic => {
                let username = values
                    .get("username")
                    .map(|value| value.trim())
                    .filter(|value| !value.is_empty())
                    .ok_or_else(|| anyhow::anyhow!("Username is required."))?;
                let password = values
                    .get("password")
                    .map(|value| value.trim())
                    .filter(|value| !value.is_empty())
                    .ok_or_else(|| anyhow::anyhow!("Password is required."))?;
                secret.username = Some(username.to_string());
                secret.password = Some(password.to_string());
            }
        }

        manager.update_secrets(|secrets| {
            secrets.mcp_auth.insert(server.id.clone(), secret);
            Ok(())
        })?;
        self.clear_pending_chat_credential_prompt(conversation_id)
            .await;

        let refresh = self
            .runtime
            .execute_action_with_context(
                "mcp_server_manage",
                &serde_json::json!({
                    "operation": "refresh",
                    "id": server.id,
                }),
                &crate::actions::ActionAuthorizationContext {
                    principal: Some(crate::actions::ActionCallerPrincipal::local_admin(
                        "secure_credential_prompt",
                    )),
                    surface: crate::actions::ActionExecutionSurface::Chat,
                    direct_user_intent: true,
                    current_turn_is_explicit_approval: false,
                    agent_name: None,
                    agent_access_scope: None,
                    capability_context_id: Some(format!("mcp_credential_save:{}", server_id)),
                },
            )
            .await;

        let mut response = format!(
            "Saved credentials for {} securely. The value was not sent to the assistant.",
            server.name
        );
        match refresh {
            Ok(output) => {
                if output.contains("\"status\": \"refresh_requested\"")
                    || output.contains("\"status\":\"refresh_requested\"")
                {
                    response.push_str(" MCP sync was refreshed.");
                }
            }
            Err(error) => {
                response.push_str(&format!(
                    " MCP sync will retry from Settings; immediate refresh failed: {}",
                    error
                ));
            }
        }
        Ok(response)
    }

    async fn mcp_server_auth_prompt_is_satisfied(&self, server_id: &str) -> bool {
        let Ok(manager) = crate::core::config::SecureConfigManager::new_with_data_dir(
            &self.config_dir,
            Some(&self.data_dir),
        ) else {
            return false;
        };
        let Ok(config) = manager.load() else {
            return false;
        };
        let Some(server) = config
            .mcp
            .servers
            .iter()
            .find(|server| server.id == server_id)
        else {
            return true;
        };
        if let Some(profile_id) = server.auth_profile_id.as_deref() {
            return crate::core::auth_profiles::AuthProfileControlPlane::get(
                &self.storage,
                profile_id,
            )
            .await
            .ok()
            .flatten()
            .is_some_and(|profile| profile.ready);
        }
        let Some(auth) = server.auth.as_ref() else {
            return true;
        };
        let Ok(secrets) = manager.load_secrets() else {
            return false;
        };
        let Some(secret) = secrets.mcp_auth.get(server_id) else {
            return false;
        };
        match auth {
            crate::core::config::McpAuthConfig::Bearer { .. }
            | crate::core::config::McpAuthConfig::Header { .. }
            | crate::core::config::McpAuthConfig::Query { .. } => secret
                .token
                .as_deref()
                .is_some_and(|value| !value.trim().is_empty()),
            crate::core::config::McpAuthConfig::Basic => {
                secret
                    .username
                    .as_deref()
                    .is_some_and(|value| !value.trim().is_empty())
                    && secret
                        .password
                        .as_deref()
                        .is_some_and(|value| !value.trim().is_empty())
            }
        }
    }

    pub(super) async fn submit_pending_custom_api_auth_credentials(
        &self,
        conversation_id: &str,
        api_id: String,
        values: &std::collections::BTreeMap<String, String>,
    ) -> Result<String> {
        let views =
            crate::custom_apis::list_custom_apis(&self.storage, &self.config_dir, &self.data_dir)
                .await?;
        let view = views
            .into_iter()
            .find(|view| view.config.id == api_id)
            .ok_or_else(|| anyhow::anyhow!("Custom API `{}` was not found.", api_id))?;
        let api_id = view.config.id.clone();
        let api_name = view.config.name.clone();
        let auth_mode = view.config.auth_mode;
        let secret = values
            .get("secret")
            .or_else(|| values.get("token"))
            .or_else(|| values.get("password"))
            .or_else(|| values.get("value"))
            .map(|value| value.trim())
            .filter(|value| !value.is_empty())
            .ok_or_else(|| anyhow::anyhow!("Credential value is required."))?;
        let username = values
            .get("username")
            .map(|value| value.trim())
            .filter(|value| !value.is_empty());

        let manager = crate::core::config::SecureConfigManager::new_with_data_dir(
            &self.config_dir,
            Some(&self.data_dir),
        )?;
        manager.set_custom_secret(
            &crate::custom_apis::custom_api_secret_key(&api_id),
            Some(secret.to_string()),
        )?;

        if matches!(auth_mode, crate::custom_apis::CustomApiAuthMode::Basic) {
            let username = username
                .or(view.config.auth_username.as_deref())
                .ok_or_else(|| anyhow::anyhow!("Username is required for basic auth."))?;
            let request = crate::custom_apis::CustomApiUpsertRequest {
                id: Some(api_id.clone()),
                name: view.config.name.clone(),
                description: Some(view.config.description.clone()),
                base_url: view.config.base_url.clone(),
                enabled: Some(view.config.enabled),
                auth_mode: Some(auth_mode),
                auth_profile_id: view.config.auth_profile_id.clone(),
                auth_header: view.config.auth_header.clone(),
                auth_name: view.config.auth_name.clone(),
                auth_username: Some(username.to_string()),
                secret: None,
                clear_secret: None,
                allow_missing_secret: Some(true),
                operations: view
                    .config
                    .operations
                    .into_iter()
                    .map(|operation| operation.draft)
                    .collect(),
            };
            crate::custom_apis::upsert_custom_api(
                &self.storage,
                &self.config_dir,
                &self.data_dir,
                &self.runtime,
                request,
                Some(&api_id),
            )
            .await?;
        }

        self.clear_pending_chat_credential_prompt(conversation_id)
            .await;
        let mut response = format!(
            "Saved credentials for {} securely. The value was not sent to the assistant.",
            api_name
        );
        if let Some(followup) = self.on_secret_saved_followup(conversation_id).await {
            response.push_str("\n\n");
            response.push_str(&followup);
        }
        Ok(response)
    }

    async fn custom_api_auth_prompt_is_satisfied(&self, api_id: &str) -> bool {
        crate::custom_apis::list_custom_apis(&self.storage, &self.config_dir, &self.data_dir)
            .await
            .ok()
            .and_then(|views| views.into_iter().find(|view| view.config.id == api_id))
            .is_none_or(|view| {
                matches!(
                    view.config.auth_mode,
                    crate::custom_apis::CustomApiAuthMode::None
                ) || view.secret_configured
            })
    }

    /// Hydrate a [`ChatCredentialPrompt`] view from a manifest by
    /// integration id. Returns `None` if no manifest is registered for the id.
    pub(super) async fn load_integration_auth_prompt_view(
        &self,
        integration_id: &str,
    ) -> Option<ChatCredentialPrompt> {
        let manifest = self
            .lookup_integration_auth_manifest(integration_id)
            .await?;
        if matches!(
            &manifest.mode,
            crate::core::integration_auth::AuthMode::OAuth2AuthorizationCode(_)
                | crate::core::integration_auth::AuthMode::OAuth2DeviceCode(_)
        ) {
            return None;
        }
        Some(Self::chat_credential_prompt_from_manifest(&manifest))
    }

    /// Resolve an [`IntegrationAuthManifest`] for the given id. Walks the
    /// extension-pack registry first, then any registered synthetic
    /// manifests (e.g. the generic raw-key manifest). Returns `None` for
    /// bundled integrations - those keep their existing text-prompt flow in
    /// this PR.
    pub(crate) async fn lookup_integration_auth_manifest(
        &self,
        integration_id: &str,
    ) -> Option<crate::core::integration_auth::IntegrationAuthManifest> {
        let needle = integration_id.trim();
        if needle.is_empty() {
            return None;
        }
        if needle == crate::core::integration_auth::RAW_KEY_MANIFEST_ID {
            // Synthetic raw-key manifest carries the literal missing key in
            // its `ToolRuntime` origin metadata; the caller that raised the
            // prompt builds the manifest directly via `raw_key_manifest` and
            // stores nothing here, so this branch stays `None` and the
            // runtime-missing path hydrates from the stored origin instead.
            return None;
        }
        if let Some(channel_id) =
            crate::custom_messaging_channels::config_id_from_auth_integration_id(needle)
        {
            match crate::custom_messaging_channels::get_custom_messaging_channel_config(
                &self.storage,
                &channel_id,
            )
            .await
            {
                Ok(Some(config)) => return config.auth_manifest,
                Ok(None) => return None,
                Err(error) => {
                    tracing::warn!(
                        "Failed to load custom messaging auth manifest for '{}': {}",
                        channel_id,
                        error
                    );
                    return None;
                }
            }
        }
        let registry = self.extension_packs.read().await;
        if let Ok(Some(pack_view)) = registry.get_pack(needle).await {
            if let Some(manifest) =
                crate::core::integration_auth::manifest_from_extension_pack(&pack_view.manifest)
            {
                return Some(manifest);
            }
        }
        None
    }

    pub(crate) async fn integration_auth_manifests(
        &self,
    ) -> Vec<crate::core::integration_auth::IntegrationAuthManifest> {
        let registry = self.extension_packs.read().await;
        let mut manifests = match registry.list_installed(None).await {
            Ok(packs) => packs
                .into_iter()
                .filter_map(|pack| {
                    crate::core::integration_auth::manifest_from_extension_pack(&pack.manifest)
                })
                .collect(),
            Err(error) => {
                tracing::warn!("Failed to load integration auth manifests: {}", error);
                Vec::new()
            }
        };
        match crate::custom_messaging_channels::list_custom_messaging_channels(
            &self.storage,
            &self.config_dir,
            &self.data_dir,
        )
        .await
        {
            Ok(channels) => {
                manifests.extend(
                    channels
                        .into_iter()
                        .filter_map(|channel| channel.config.auth_manifest),
                );
            }
            Err(error) => {
                tracing::warn!("Failed to load custom messaging auth manifests: {}", error);
            }
        }
        manifests
    }

    /// Convert an [`crate::core::integration_auth::IntegrationAuthManifest`]
    /// into the wire-level [`ChatCredentialPrompt`] view consumed by the
    /// frontend. Rendered fields carry the manifest's input-type hints and
    /// per-field help so the inline prompt can render password vs. text vs.
    /// textarea without having to guess. Associated function (no `self`) so
    /// it can be called from manifest-driven hydration without a storage
    /// round-trip.
    pub(super) fn chat_credential_prompt_from_manifest(
        manifest: &crate::core::integration_auth::IntegrationAuthManifest,
    ) -> ChatCredentialPrompt {
        use crate::core::integration_auth::{AuthMode, FieldInputType, RAW_KEY_MANIFEST_ID};

        let is_raw_key = manifest.integration_id == RAW_KEY_MANIFEST_ID;
        let settings_path =
            Self::integration_auth_settings_path(&manifest.integration_id, &manifest.display_name);
        let (fields, mode_kind) = match &manifest.mode {
            AuthMode::Secrets { fields } => (
                fields.clone(),
                if is_raw_key { "raw_key" } else { "secrets" },
            ),
            AuthMode::Hybrid { fields, .. } => (fields.clone(), "hybrid"),
            AuthMode::OAuth2AuthorizationCode(_) => (Vec::new(), "oauth2_authorization_code"),
            AuthMode::OAuth2DeviceCode(_) => (Vec::new(), "oauth2_device_code"),
        };
        let wire_fields = fields
            .into_iter()
            .map(|field| {
                let (input_type, options) = match &field.input_type {
                    FieldInputType::Text => (Some("text".to_string()), None),
                    FieldInputType::Password => (Some("password".to_string()), None),
                    FieldInputType::Textarea => (Some("textarea".to_string()), None),
                    FieldInputType::Select { options } => {
                        (Some("select".to_string()), Some(options.clone()))
                    }
                };
                ChatCredentialPromptField {
                    key: field.key,
                    label: field.label,
                    required: field.required,
                    input_type,
                    placeholder: field.placeholder,
                    help: field.help,
                    options,
                }
            })
            .collect::<Vec<_>>();
        let description = manifest.description.clone().unwrap_or_else(|| {
            format!(
                "Enter the credentials required to connect {}.",
                manifest.display_name
            )
        });
        let warning = manifest.warning.clone().unwrap_or_else(|| {
            "The assistant does not see these values. They are sent directly to AgentArk over the inline prompt and stored encrypted.".to_string()
        });
        ChatCredentialPrompt {
            kind: if is_raw_key {
                "raw_secret".to_string()
            } else {
                "integration_auth".to_string()
            },
            title: if is_raw_key {
                "Credential required".to_string()
            } else {
                format!("Connect {}", manifest.display_name)
            },
            description,
            warning,
            submit_label: manifest.post_submit.label.clone(),
            fallback_command: String::new(),
            fields: wire_fields,
            integration_id: Some(manifest.integration_id.clone()),
            mode_kind: Some(mode_kind.to_string()),
            docs_url: manifest.docs_url.clone(),
            settings_path,
        }
    }

    pub(super) fn extension_pack_settings_path(pack_name: &str) -> String {
        format!(
            "Settings > Integrations > Extension Pack Integrations > {}",
            pack_name.trim()
        )
    }

    pub(super) fn integration_auth_settings_path(
        integration_id: &str,
        display_name: &str,
    ) -> Option<String> {
        if integration_id == crate::core::integration_auth::RAW_KEY_MANIFEST_ID {
            return None;
        }
        if crate::custom_messaging_channels::config_id_from_auth_integration_id(integration_id)
            .is_some()
        {
            return Some(format!(
                "Settings > Messaging Channels > Custom Messaging Channels > {}",
                display_name.trim()
            ));
        }
        Some(Self::extension_pack_settings_path(display_name))
    }

    pub(super) fn build_chat_credential_prompt(
        &self,
        title: String,
        description: String,
        fields: Vec<ChatCredentialPromptField>,
        kind: &str,
    ) -> Option<ChatCredentialPrompt> {
        if fields.is_empty() {
            return None;
        }
        Some(ChatCredentialPrompt {
            kind: kind.to_string(),
            title,
            description,
            warning: "Never paste secrets, API keys, passwords, or sensitive data into normal chat. Use the secure credential form shown in this conversation.".to_string(),
            submit_label: "Save securely".to_string(),
            fallback_command: String::new(),
            fields,
            integration_id: None,
            mode_kind: None,
            docs_url: None,
            settings_path: None,
        })
    }

    pub async fn pending_chat_credential_prompt(
        &self,
        conversation_id: &str,
    ) -> Option<ChatCredentialPrompt> {
        let conversation_id = conversation_id.trim();
        if conversation_id.is_empty() {
            return None;
        }
        if let Some(pending) = self
            .load_pending_chat_credential_prompt(conversation_id)
            .await
        {
            if Self::chat_credential_prompt_is_expired(
                pending.requested_at,
                chrono::Duration::minutes(30),
            ) {
                self.clear_pending_chat_credential_prompt(conversation_id)
                    .await;
            } else {
                match pending.kind {
                    PendingChatCredentialPromptKind::ExtensionPackConnection {
                        pack_name,
                        required_keys,
                        ..
                    } => {
                        let fields = required_keys
                            .into_iter()
                            .map(|key| ChatCredentialPromptField {
                                label: Self::chat_credential_field_label(&key),
                                key,
                                required: true,
                                input_type: None,
                                placeholder: None,
                                help: None,
                                options: None,
                            })
                            .collect::<Vec<_>>();
                        return self.build_chat_credential_prompt(
                            format!("{} credentials required", pack_name),
                            format!(
                                "AgentArk created the connection draft for {}. Save the credential here, or open {}. The values are stored encrypted and are not exposed in chat.",
                                pack_name,
                                Self::extension_pack_settings_path(&pack_name)
                            ),
                            fields,
                            "extension_pack_connection",
                        ).map(|mut prompt| {
                            prompt.settings_path =
                                Some(Self::extension_pack_settings_path(&pack_name));
                            prompt
                        });
                    }
                    PendingChatCredentialPromptKind::IntegrationAuth { integration_id, .. } => {
                        if let Some(prompt) = self
                            .load_integration_auth_prompt_view(&integration_id)
                            .await
                        {
                            return Some(prompt);
                        }
                        self.clear_pending_chat_credential_prompt(conversation_id)
                            .await;
                    }
                    PendingChatCredentialPromptKind::RawSecret { key, origin } => {
                        if matches!(
                            origin,
                            IntegrationAuthPromptOrigin::ToolRuntime {
                                tool_name: None,
                                trace_id: None
                            }
                        ) {
                            self.clear_pending_chat_credential_prompt(conversation_id)
                                .await;
                            return None;
                        }
                        let manifest = crate::core::integration_auth::raw_key_manifest(&key);
                        return Some(Self::chat_credential_prompt_from_manifest(&manifest));
                    }
                    PendingChatCredentialPromptKind::McpServerAuth {
                        server_id,
                        server_name,
                        auth_type,
                        auth_name,
                        settings_path,
                    } => {
                        if self.mcp_server_auth_prompt_is_satisfied(&server_id).await {
                            self.clear_pending_chat_credential_prompt(conversation_id)
                                .await;
                            return None;
                        }
                        let fields = match auth_type.as_str() {
                            "basic" => vec![
                                ChatCredentialPromptField {
                                    key: "username".to_string(),
                                    label: "Username".to_string(),
                                    required: true,
                                    input_type: Some("text".to_string()),
                                    placeholder: None,
                                    help: None,
                                    options: None,
                                },
                                ChatCredentialPromptField {
                                    key: "password".to_string(),
                                    label: "Password".to_string(),
                                    required: true,
                                    input_type: Some("password".to_string()),
                                    placeholder: None,
                                    help: None,
                                    options: None,
                                },
                            ],
                            "header" | "query" => vec![ChatCredentialPromptField {
                                key: "token".to_string(),
                                label: auth_name
                                    .as_deref()
                                    .map(Self::chat_credential_field_label)
                                    .unwrap_or_else(|| "Credential value".to_string()),
                                required: true,
                                input_type: Some("password".to_string()),
                                placeholder: None,
                                help: None,
                                options: None,
                            }],
                            _ => vec![ChatCredentialPromptField {
                                key: "token".to_string(),
                                label: "Bearer token".to_string(),
                                required: true,
                                input_type: Some("password".to_string()),
                                placeholder: None,
                                help: None,
                                options: None,
                            }],
                        };
                        return self
                            .build_chat_credential_prompt(
                                format!("Connect {}", server_name),
                                format!(
                                    "AgentArk configured {}. Save the required MCP credential here; it is stored encrypted and never sent through normal chat.",
                                    server_name
                                ),
                                fields,
                                "mcp_server_auth",
                            )
                            .map(|mut prompt| {
                                prompt.settings_path = settings_path;
                                prompt.mode_kind = Some("mcp_server_auth".to_string());
                                prompt
                            });
                    }
                    PendingChatCredentialPromptKind::CustomApiAuth {
                        api_id,
                        api_name,
                        auth_mode,
                        auth_name,
                        settings_path,
                    } => {
                        if self.custom_api_auth_prompt_is_satisfied(&api_id).await {
                            self.clear_pending_chat_credential_prompt(conversation_id)
                                .await;
                            return None;
                        }
                        let fields = match auth_mode.as_str() {
                            "basic" => vec![
                                ChatCredentialPromptField {
                                    key: "username".to_string(),
                                    label: "Username".to_string(),
                                    required: true,
                                    input_type: Some("text".to_string()),
                                    placeholder: None,
                                    help: None,
                                    options: None,
                                },
                                ChatCredentialPromptField {
                                    key: "password".to_string(),
                                    label: "Password".to_string(),
                                    required: true,
                                    input_type: Some("password".to_string()),
                                    placeholder: None,
                                    help: None,
                                    options: None,
                                },
                            ],
                            "api_key_header" | "api_key_query" => vec![ChatCredentialPromptField {
                                key: "secret".to_string(),
                                label: auth_name
                                    .as_deref()
                                    .map(Self::chat_credential_field_label)
                                    .unwrap_or_else(|| "API key".to_string()),
                                required: true,
                                input_type: Some("password".to_string()),
                                placeholder: None,
                                help: None,
                                options: None,
                            }],
                            "oauth2" => vec![ChatCredentialPromptField {
                                key: "secret".to_string(),
                                label: "OAuth access token".to_string(),
                                required: true,
                                input_type: Some("password".to_string()),
                                placeholder: None,
                                help: Some(
                                    "Use a secure auth profile instead when a browser OAuth flow is required."
                                        .to_string(),
                                ),
                                options: None,
                            }],
                            _ => vec![ChatCredentialPromptField {
                                key: "secret".to_string(),
                                label: "Bearer token".to_string(),
                                required: true,
                                input_type: Some("password".to_string()),
                                placeholder: None,
                                help: None,
                                options: None,
                            }],
                        };
                        return self
                            .build_chat_credential_prompt(
                                format!("Connect {}", api_name),
                                format!(
                                    "AgentArk configured {}. Save the required API credential here; it is stored encrypted and never sent through normal chat.",
                                    api_name
                                ),
                                fields,
                                "custom_api_auth",
                            )
                            .map(|mut prompt| {
                                prompt.settings_path = settings_path;
                                prompt.mode_kind = Some("custom_api_auth".to_string());
                                prompt
                            });
                    }
                }
            }
        }

        if let Some(flow) = self
            .load_pending_integration_connect_flow(conversation_id)
            .await
        {
            if Self::chat_credential_prompt_is_expired(
                flow.started_at,
                chrono::Duration::seconds(crate::core::connect_flow::CONNECT_FLOW_TTL_SECS),
            ) {
                self.clear_pending_integration_connect_flow(conversation_id)
                    .await;
            } else if let Some(spec) = crate::core::connect_flow::spec_by_id(&flow.integration_id) {
                let mut fields = spec
                    .required
                    .keys
                    .iter()
                    .map(|key| ChatCredentialPromptField {
                        key: (*key).to_string(),
                        label: Self::chat_credential_field_label(key),
                        required: matches!(
                            spec.required.kind,
                            crate::core::connect_flow::SecretRequirementKind::All
                        ),
                        input_type: None,
                        placeholder: None,
                        help: None,
                        options: None,
                    })
                    .collect::<Vec<_>>();
                fields.extend(spec.optional.iter().map(|key| ChatCredentialPromptField {
                    key: (*key).to_string(),
                    label: Self::chat_credential_field_label(key),
                    required: false,
                    input_type: None,
                    placeholder: None,
                    help: None,
                    options: None,
                }));
                let description = match spec.required.kind {
                    crate::core::connect_flow::SecretRequirementKind::All => format!(
                        "Finish connecting {} by entering the required secrets below. AgentArk will test and enable it after the values are saved.",
                        spec.name
                    ),
                    crate::core::connect_flow::SecretRequirementKind::Any => format!(
                        "Finish connecting {} by entering at least one of the accepted secrets below. AgentArk will test it after the values are saved.",
                        spec.name
                    ),
                };
                return self.build_chat_credential_prompt(
                    format!("Connect {}", spec.name),
                    description,
                    fields,
                    "integration_connect",
                );
            }
        }

        let pending = self.load_pending_secret_followup(conversation_id).await?;
        if Self::chat_credential_prompt_is_expired(
            pending.requested_at,
            chrono::Duration::minutes(30),
        ) {
            self.clear_pending_secret_followup(conversation_id).await;
            return None;
        }
        match pending.kind {
            PendingSecretFollowupKind::EnableSkill { action_name } => {
                let Some((_, content)) = self
                    .runtime
                    .get_action_content(&action_name)
                    .await
                    .ok()
                    .flatten()
                else {
                    return None;
                };
                let fields = self
                    .missing_skill_required_envs(&action_name, &content)
                    .await
                    .unwrap_or_default()
                    .into_iter()
                    .map(|key| ChatCredentialPromptField {
                        label: Self::chat_credential_field_label(&key),
                        key,
                        required: true,
                        input_type: None,
                        placeholder: None,
                        help: None,
                        options: None,
                    })
                    .collect::<Vec<_>>();
                self.build_chat_credential_prompt(
                    format!("Secrets required for {}", action_name),
                    format!(
                        "AgentArk paused before enabling `{}`. Save the required secret values here, then it will continue automatically.",
                        action_name
                    ),
                    fields,
                    "skill_secret_followup",
                )
            }
            PendingSecretFollowupKind::RetryWorkflow { payload } => {
                let fields = payload
                    .sensitive_missing
                    .clone()
                    .into_iter()
                    .map(|key| ChatCredentialPromptField {
                        label: Self::chat_credential_field_label(&key),
                        key,
                        required: true,
                        input_type: None,
                        placeholder: None,
                        help: None,
                        options: None,
                    })
                    .collect::<Vec<_>>();
                self.build_chat_credential_prompt(
                    format!("Input required for {}", payload.action),
                    format!(
                        "AgentArk paused before continuing `{}`. Save the missing sensitive input here and it will resume from the waiting step.",
                        payload.action
                    ),
                    fields,
                    "workflow_secret_followup",
                )
            }
            PendingSecretFollowupKind::RestartApp {
                title, missing_env, ..
            } => {
                let fields = missing_env
                    .into_iter()
                    .map(|key| ChatCredentialPromptField {
                        label: Self::chat_credential_field_label(&key),
                        key,
                        required: true,
                        input_type: None,
                        placeholder: None,
                        help: None,
                        options: None,
                    })
                    .collect::<Vec<_>>();
                self.build_chat_credential_prompt(
                    format!("Secrets required for {}", title),
                    format!(
                        "AgentArk paused before restarting `{}`. Save the missing environment secrets here and it will continue automatically.",
                        title
                    ),
                    fields,
                    "app_secret_followup",
                )
            }
        }
    }

    pub(super) async fn sync_extension_pack_runtime_warning(&self) -> Option<String> {
        let registry = self.extension_packs.clone();
        let guard = registry.read().await;
        let warning = guard
            .sync_to_runtime(&self.runtime)
            .await
            .err()
            .map(|e| e.to_string());
        drop(guard);
        if warning.is_none() {
            self.refresh_action_catalog_index("extension_pack_credential_sync")
                .await;
        }
        warning
    }

    pub(super) async fn submit_pending_extension_pack_credentials(
        &self,
        conversation_id: &str,
        pending: PendingChatCredentialPrompt,
        values: &BTreeMap<String, String>,
    ) -> Result<String> {
        match pending.kind {
            PendingChatCredentialPromptKind::ExtensionPackConnection {
                pack_id,
                pack_name,
                connection_id,
                required_keys,
            } => {
                let requested_values = values
                    .iter()
                    .filter_map(|(key, value)| {
                        let canonical = required_keys
                            .iter()
                            .find(|candidate| candidate.eq_ignore_ascii_case(key.trim()))?;
                        let trimmed = value.trim();
                        if trimmed.is_empty() {
                            return None;
                        }
                        Some((canonical.clone(), trimmed.to_string()))
                    })
                    .collect::<Vec<_>>();
                if requested_values.is_empty() {
                    anyhow::bail!(
                        "Expected one of these credential keys: {}",
                        required_keys
                            .iter()
                            .map(|key| format!("`{}`", key))
                            .collect::<Vec<_>>()
                            .join(", ")
                    );
                }
                let (connection_id, still_missing, test_result, test_error) = {
                    let mut guard = self.extension_packs.write().await;
                    let existing_secret = guard
                        .get_connection_secret(&pack_id, &connection_id)?
                        .unwrap_or_else(|| serde_json::Value::Object(serde_json::Map::new()));
                    let mut merged = existing_secret
                        .as_object()
                        .cloned()
                        .unwrap_or_else(serde_json::Map::new);
                    for (key, value) in requested_values {
                        merged.insert(key, serde_json::Value::String(value));
                    }
                    let still_missing = required_keys
                        .iter()
                        .filter(|key| {
                            merged
                                .get(key.as_str())
                                .and_then(|value| value.as_str())
                                .map(str::trim)
                                .filter(|value| !value.is_empty())
                                .is_none()
                        })
                        .cloned()
                        .collect::<Vec<_>>();
                    let connection = guard
                        .upsert_connection(
                            &pack_id,
                            crate::extension_packs::ExtensionPackConnectionUpsertRequest {
                                connection_id: Some(connection_id.clone()),
                                name: None,
                                enabled: Some(true),
                                metadata: None,
                                secret: Some(serde_json::Value::Object(merged)),
                                clear_secret: false,
                            },
                        )
                        .await?;
                    let (test_result, test_error) = if still_missing.is_empty() {
                        match guard
                            .test_connection(
                                &pack_id,
                                &connection.connection.id,
                                Some(self.mcp.clone()),
                                Some(self.plugins.clone()),
                            )
                            .await
                        {
                            Ok(result) => (Some(result), None),
                            Err(error) => (None, Some(error.to_string())),
                        }
                    } else {
                        (None, None)
                    };
                    (
                        connection.connection.id,
                        still_missing,
                        test_result,
                        test_error,
                    )
                };
                let sync_warning = self.sync_extension_pack_runtime_warning().await;
                if !still_missing.is_empty() {
                    return Ok(format!(
                        "Saved the provided credential values for {}. Still missing: {}",
                        pack_name,
                        still_missing
                            .iter()
                            .map(|key| format!("`{}`", key))
                            .collect::<Vec<_>>()
                            .join(", ")
                    ));
                }
                if let Some(error) = test_error {
                    let mut response = format!(
                        "Saved credentials for {}, but the connection test could not run: {}. The connection record is updated; retry from the secure form or Integrations page.",
                        pack_name, error
                    );
                    if let Some(warning) = sync_warning {
                        response.push_str("\n\nRuntime hot-sync warning: ");
                        response.push_str(&warning);
                    }
                    return Ok(response);
                }
                if let Some(result) = test_result {
                    if result.ok {
                        self.clear_pending_chat_credential_prompt(conversation_id)
                            .await;
                        let mut response = format!("{} is now connected and saved.", pack_name);
                        if let Some(message) = result
                            .message
                            .as_deref()
                            .map(str::trim)
                            .filter(|value| !value.is_empty())
                        {
                            response.push_str("\n\n");
                            response.push_str(message);
                        }
                        if let Some(warning) = sync_warning {
                            response.push_str("\n\nRuntime hot-sync warning: ");
                            response.push_str(&warning);
                        }
                        return Ok(response);
                    }
                    let detail = result
                        .message
                        .as_deref()
                        .or(result.error.as_deref())
                        .map(str::trim)
                        .filter(|value| !value.is_empty())
                        .unwrap_or("connection validation failed");
                    let mut response = format!(
                        "Saved credentials for {}, but the connection test failed: {}. Update the secure form below and retry.",
                        pack_name, detail
                    );
                    if let Some(warning) = sync_warning {
                        response.push_str("\n\nRuntime hot-sync warning: ");
                        response.push_str(&warning);
                    }
                    return Ok(response);
                }
                let mut response = format!(
                    "Saved credentials for {} using connection `{}`.",
                    pack_name, connection_id
                );
                if let Some(warning) = sync_warning {
                    response.push_str("\n\nRuntime hot-sync warning: ");
                    response.push_str(&warning);
                }
                Ok(response)
            }
            _ => anyhow::bail!("Pending credential prompt is not an extension-pack prompt."),
        }
    }

    pub async fn submit_chat_credential_values(
        &self,
        conversation_id: Option<&str>,
        values: &BTreeMap<String, String>,
    ) -> Result<String> {
        let cleaned = values
            .iter()
            .filter_map(|(key, value)| {
                let clean_key = key.trim();
                let clean_value = value.trim();
                if clean_key.is_empty() || clean_value.is_empty() {
                    return None;
                }
                Some((clean_key.to_string(), clean_value.to_string()))
            })
            .collect::<BTreeMap<_, _>>();
        if cleaned.is_empty() {
            anyhow::bail!("Provide at least one non-empty secret value.");
        }
        if let Some(cid) = conversation_id
            .map(str::trim)
            .filter(|value| !value.is_empty())
        {
            if let Some(pending) = self.load_pending_chat_credential_prompt(cid).await {
                if !Self::chat_credential_prompt_is_expired(
                    pending.requested_at,
                    chrono::Duration::minutes(30),
                ) {
                    match &pending.kind {
                        PendingChatCredentialPromptKind::ExtensionPackConnection { .. } => {
                            return self
                                .submit_pending_extension_pack_credentials(cid, pending, &cleaned)
                                .await;
                        }
                        PendingChatCredentialPromptKind::IntegrationAuth {
                            integration_id, ..
                        } => {
                            return self
                                .submit_pending_integration_auth_credentials(
                                    cid,
                                    integration_id.clone(),
                                    &cleaned,
                                )
                                .await;
                        }
                        PendingChatCredentialPromptKind::RawSecret { key, .. } => {
                            return self
                                .submit_pending_raw_secret_credentials(cid, key.clone(), &cleaned)
                                .await;
                        }
                        PendingChatCredentialPromptKind::McpServerAuth { server_id, .. } => {
                            return self
                                .submit_pending_mcp_server_auth_credentials(
                                    cid,
                                    server_id.clone(),
                                    &cleaned,
                                )
                                .await;
                        }
                        PendingChatCredentialPromptKind::CustomApiAuth { api_id, .. } => {
                            return self
                                .submit_pending_custom_api_auth_credentials(
                                    cid,
                                    api_id.clone(),
                                    &cleaned,
                                )
                                .await;
                        }
                    }
                } else {
                    self.clear_pending_chat_credential_prompt(cid).await;
                }
            }
        }
        for (key, value) in &cleaned {
            crate::core::secrets::store_user_secret(
                &self.config_dir,
                Some(&self.data_dir),
                key,
                value,
            )?;
        }
        let mut response = if cleaned.len() == 1 {
            let key = cleaned.keys().next().cloned().unwrap_or_default();
            format!(
                "Saved secret '{}' (stored encrypted). This value was not sent to the LLM.",
                key
            )
        } else {
            format!(
                "Saved {} secrets (stored encrypted). These values were not sent to the LLM.",
                cleaned.len()
            )
        };
        if let Some(cid) = conversation_id
            .map(str::trim)
            .filter(|value| !value.is_empty())
        {
            if let Some(followup) = self.on_secret_saved_followup(cid).await {
                response.push_str("\n\n");
                response.push_str(&followup);
            }
        }
        Ok(response)
    }

    pub(super) async fn complete_pending_secret_followup(
        &self,
        conversation_id: &str,
    ) -> Option<String> {
        if conversation_id.trim().is_empty() {
            return None;
        }

        let pending = self.load_pending_secret_followup(conversation_id).await?;

        if (chrono::Utc::now() - pending.requested_at) > chrono::Duration::minutes(30) {
            self.clear_pending_secret_followup(conversation_id).await;
            return Some(
                "Saved. The previous follow-up expired due to inactivity, so I cleared it. If you still want me to continue, ask again in this chat."
                    .to_string(),
            );
        }

        match pending.kind {
            PendingSecretFollowupKind::EnableSkill { action_name } => {
                let Some((_, content)) = self
                    .runtime
                    .get_action_content(&action_name)
                    .await
                    .ok()
                    .flatten()
                else {
                    self.clear_pending_secret_followup(conversation_id).await;
                    return Some(format!(
                        "Saved. I can't find skill '{}' anymore, so I cleared the pending follow-up.",
                        action_name
                    ));
                };
                let missing = self
                    .missing_skill_required_envs(&action_name, &content)
                    .await
                    .unwrap_or_default();
                if !missing.is_empty() {
                    return Some(format!(
                        "Saved. Skill '{}' still needs secret(s): {}",
                        action_name,
                        missing
                            .iter()
                            .map(|key| format!("`{}`", key))
                            .collect::<Vec<_>>()
                            .join(", ")
                    ));
                }
                let _ = self.runtime.set_action_enabled(&action_name, true).await;
                self.clear_pending_secret_followup(conversation_id).await;
                Some(format!(
                    "Saved. Skill '{}' is now enabled and ready. You can run it by saying: run {} ...",
                    action_name, action_name
                ))
            }
            PendingSecretFollowupKind::RetryWorkflow { payload } => {
                let rerun = self
                    .execute_workflow_marker_action(&payload.action, &payload.query)
                    .await;
                match rerun {
                    Ok(output) => {
                        if let Some(next_payload) = parse_workflow_missing_inputs_marker(&output) {
                            if next_payload.sensitive_missing.is_empty() {
                                self.clear_pending_secret_followup(conversation_id).await;
                            } else {
                                self.remember_pending_secret_followup(
                                    conversation_id,
                                    PendingSecretFollowupKind::RetryWorkflow {
                                        payload: next_payload.clone(),
                                    },
                                )
                                .await;
                            }
                            return Some(Self::format_missing_inputs_prompt(&next_payload));
                        }
                        self.clear_pending_secret_followup(conversation_id).await;
                        Some(output)
                    }
                    Err(error) => Some(format!(
                        "Saved. I still couldn't continue `{}` yet: {}",
                        payload.action, error
                    )),
                }
            }
            PendingSecretFollowupKind::RestartApp {
                app_id,
                title,
                missing_env,
            } => {
                let manager = crate::core::config::SecureConfigManager::new_with_data_dir(
                    &self.config_dir,
                    Some(&self.data_dir),
                )
                .ok()?;
                let secrets = manager.load_secrets().ok()?;
                let custom = &secrets.custom;
                let still_missing: Vec<String> = missing_env
                    .into_iter()
                    .filter(|key| {
                        !crate::core::secrets::has_user_secret(custom, key)
                            && !Self::builtin_env_available_for_skill_import(&self.config, key)
                    })
                    .collect();
                if !still_missing.is_empty() {
                    return Some(format!(
                        "Saved. App '{}' still needs secret(s): {}",
                        title,
                        still_missing
                            .iter()
                            .map(|key| format!("`{}`", key))
                            .collect::<Vec<_>>()
                            .join(", ")
                    ));
                }
                match self.restart_deployed_app_from_metadata(&app_id, None).await {
                    Ok(out) => {
                        self.clear_pending_secret_followup(conversation_id).await;
                        let message = out
                            .get("message")
                            .and_then(|value| value.as_str())
                            .map(str::trim)
                            .filter(|value| !value.is_empty())
                            .map(str::to_string)
                            .unwrap_or_else(|| format!("Restarted app '{}'.", title));
                        Some(message)
                    }
                    Err(error) => Some(format!(
                        "Saved. I couldn't restart app '{}' yet: {}",
                        title, error
                    )),
                }
            }
        }
    }

    /// Called after a secret is stored via a chat-safe command.
    /// Resumes any conversation-scoped follow-up that was waiting on that secret.
    pub async fn on_secret_saved_followup(&self, conversation_id: &str) -> Option<String> {
        let integration = self
            .continue_integration_connect_flow_after_secret_save(conversation_id)
            .await;
        let pending = self.complete_pending_secret_followup(conversation_id).await;
        match (integration, pending) {
            (Some(a), Some(b)) => Some(format!("{}\n\n{}", a, b)),
            (Some(a), None) => Some(a),
            (None, Some(b)) => Some(b),
            (None, None) => None,
        }
    }
}
