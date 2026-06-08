use super::super::*;

impl ActionRuntime {
    pub(in crate::runtime) async fn execute_mcp_action(
        &self,
        binding: McpBinding,
        arguments: &serde_json::Value,
    ) -> Result<String> {
        let violations =
            crate::core::request_contract::validate_json_object_envelope("mcp", arguments);
        if !violations.is_empty() {
            return Ok(structured_tool_completion_output(
                "mcp",
                "needs_arguments",
                "MCP action cannot be sent because the arguments envelope is invalid.",
                serde_json::json!({
                    "server_id": binding.server_id,
                    "violations": violations,
                    "expected_contract": crate::core::request_contract::expected_arguments_envelope_contract("mcp"),
                    "assistant_instruction": "Satisfy the complete MCP arguments envelope in one corrected call. If a required field is genuinely absent from the conversation, ask the user for that missing value."
                }),
            ));
        }
        crate::security::tool_args_guard::check_outward_urls_in_json_anyhow(
            arguments,
            &self.tool_args_guard_config(),
        )
        .await
        .context("MCP action arguments denied by outbound URL guard")?;
        let registry = self
            .mcp_registry
            .as_ref()
            .ok_or_else(|| anyhow::anyhow!("MCP registry not initialized"))?;
        let mut registry = registry.write().await;
        match binding.kind {
            McpBindingKind::Tool { name } => {
                registry
                    .call_tool(&binding.server_id, &name, arguments)
                    .await
            }
            McpBindingKind::Resource { uri } => {
                registry.read_resource(&binding.server_id, &uri).await
            }
        }
    }

    pub(in crate::runtime) async fn execute_plugin_action(
        &self,
        binding: PluginBinding,
        arguments: &serde_json::Value,
    ) -> Result<String> {
        crate::security::tool_args_guard::check_outward_urls_in_json_anyhow(
            arguments,
            &self.tool_args_guard_config(),
        )
        .await
        .context("Plugin action arguments denied by outbound URL guard")?;
        let registry = self
            .plugin_registry
            .as_ref()
            .ok_or_else(|| anyhow::anyhow!("Plugin registry not initialized"))?;
        registry
            .write()
            .await
            .invoke_action(&binding.plugin_id, &binding.action_name, arguments)
            .await
    }

    pub(in crate::runtime) async fn execute_custom_api_action(
        &self,
        binding: CustomApiBinding,
        arguments: &serde_json::Value,
    ) -> Result<String> {
        crate::security::tool_args_guard::check_outward_urls_in_json_anyhow(
            arguments,
            &self.tool_args_guard_config(),
        )
        .await
        .context("Custom API action arguments denied by outbound URL guard")?;
        let mut path = binding.path.clone();
        let mut query_pairs = binding.default_query.clone();
        let mut dynamic_headers = reqwest::header::HeaderMap::new();

        for parameter in &binding.parameters {
            let maybe_value = arguments.get(&parameter.name);
            match parameter.location {
                crate::custom_apis::CustomApiParameterLocation::Path => {
                    let value = maybe_value
                        .and_then(Self::value_to_http_string)
                        .filter(|value| !value.is_empty());
                    let value = match value {
                        Some(value) => value,
                        None if parameter.required => {
                            return Err(anyhow::anyhow!(
                                "Missing required path parameter '{}'",
                                parameter.name
                            ));
                        }
                        None => continue,
                    };
                    let encoded = urlencoding::encode(&value).to_string();
                    path = path.replace(&format!("{{{}}}", parameter.name), encoded.as_str());
                    path = path.replace(&format!(":{}", parameter.name), encoded.as_str());
                }
                crate::custom_apis::CustomApiParameterLocation::Query => {
                    if let Some(value) = maybe_value
                        .and_then(Self::value_to_http_string)
                        .filter(|v| !v.is_empty())
                    {
                        query_pairs.insert(parameter.name.clone(), value);
                    } else if parameter.required && !query_pairs.contains_key(&parameter.name) {
                        return Err(anyhow::anyhow!(
                            "Missing required query parameter '{}'",
                            parameter.name
                        ));
                    }
                }
                crate::custom_apis::CustomApiParameterLocation::Header => {
                    if let Some(value) = maybe_value
                        .and_then(Self::value_to_http_string)
                        .filter(|v| !v.is_empty())
                    {
                        let header_name =
                            reqwest::header::HeaderName::from_bytes(parameter.name.as_bytes())
                                .map_err(|_| {
                                    anyhow::anyhow!(
                                        "Invalid header parameter name '{}'",
                                        parameter.name
                                    )
                                })?;
                        let header_value =
                            reqwest::header::HeaderValue::from_str(&value).map_err(|_| {
                                anyhow::anyhow!("Invalid header value for '{}'", parameter.name)
                            })?;
                        dynamic_headers.insert(header_name, header_value);
                    } else if parameter.required {
                        return Err(anyhow::anyhow!(
                            "Missing required header parameter '{}'",
                            parameter.name
                        ));
                    }
                }
                crate::custom_apis::CustomApiParameterLocation::Body => {}
            }
        }

        let body_argument = arguments.get("body").or(binding.default_body.as_ref());
        let mut contract_headers = binding.default_headers.clone();
        for (key, value) in dynamic_headers.iter() {
            if let Ok(value) = value.to_str() {
                contract_headers.insert(key.as_str().to_string(), value.to_string());
            }
        }
        let graphql_signal = Self::custom_api_binding_supports_graphql_body(&binding)
            || body_argument.is_some_and(crate::core::request_contract::body_has_graphql_signal);
        let shape = crate::core::request_contract::normalize_request_shape(
            &binding.method,
            crate::core::request_contract::request_headers_have_content_type(&contract_headers),
            body_argument,
            graphql_signal,
        );
        let normalized_body = shape.body.clone();
        if (binding.body_required || graphql_signal) && normalized_body.is_none() {
            let violation = crate::core::request_contract::missing_required_body_violation();
            return Ok(structured_tool_completion_output(
                "custom_api",
                "needs_arguments",
                "Custom API request cannot be sent because the request body contract is incomplete.",
                serde_json::json!({
                    "integration_type": "custom_api",
                    "custom_api_id": binding.api_id,
                    "operation": binding.operation_id,
                    "action_name": binding.operation_name,
                    "violations": [violation],
                    "expected_contract": crate::core::request_contract::expected_request_contract(),
                    "assistant_instruction": "Satisfy the complete request contract in one corrected call. If the required body is genuinely absent from the conversation, ask the user for that missing request body."
                }),
            ));
        }

        if binding.read_only
            && Self::custom_api_binding_supports_graphql_body(&binding)
            && normalized_body.as_ref().is_some_and(|body| {
                !crate::custom_apis::custom_api_body_is_read_only_graphql_query(body)
            })
        {
            anyhow::bail!(
                "Read-only GraphQL custom API actions only accept GraphQL query operations."
            );
        }

        let base = binding.base_url.trim_end_matches('/');
        let path = if path.starts_with('/') {
            path
        } else {
            format!("/{}", path)
        };
        let mut url = reqwest::Url::parse(&format!("{}{}", base, path))
            .map_err(|e| anyhow::anyhow!("Invalid custom API URL: {}", e))?;
        {
            let mut pairs = url.query_pairs_mut();
            for (key, value) in &query_pairs {
                pairs.append_pair(key, value);
            }
        }
        let auth_overlay = if let Some(auth_profile_id) = binding.auth_profile_id.as_deref() {
            Some(
                self.resolve_auth_profile_http(auth_profile_id)
                    .await?
                    .overlay,
            )
        } else {
            None
        };
        if let Some(overlay) = auth_overlay.as_ref() {
            overlay.apply_to_url(&mut url);
        }
        url = crate::security::tool_args_guard::check_outward_url_anyhow(
            url.as_str(),
            &self.tool_args_guard_config(),
        )
        .await
        .with_context(|| format!("custom API URL denied for '{}'", binding.operation_name))?;

        let client = reqwest::Client::builder()
            .timeout(std::time::Duration::from_secs(20))
            .build()
            .map_err(|e| anyhow::anyhow!("Failed to build HTTP client: {}", e))?;
        let method = reqwest::Method::from_bytes(shape.method.as_bytes())
            .map_err(|e| anyhow::anyhow!("Invalid HTTP method '{}': {}", shape.method, e))?;
        let manager =
            SecureConfigManager::new_with_data_dir(&self.config_dir, Some(self.data_dir()))?;
        let secret = manager.get_custom_secret(&binding.secret_key)?;
        let secret = secret.as_deref().filter(|value| !value.trim().is_empty());

        if matches!(
            binding.auth_mode,
            crate::custom_apis::CustomApiAuthMode::ApiKeyQuery
        ) {
            let token = secret.ok_or_else(|| {
                anyhow::anyhow!(
                    "Auth secret '{}' is not configured for '{}'",
                    binding.secret_key,
                    binding.api_name
                )
            })?;
            let query_name = binding.auth_name.as_deref().unwrap_or("api_key");
            url.query_pairs_mut().append_pair(query_name, token.trim());
        }

        let mut request = client.request(method, url.clone());
        for (key, value) in &binding.default_headers {
            let header_name = reqwest::header::HeaderName::from_bytes(key.as_bytes())
                .map_err(|_| anyhow::anyhow!("Invalid default header name '{}'", key))?;
            let header_value = reqwest::header::HeaderValue::from_str(value)
                .map_err(|_| anyhow::anyhow!("Invalid default header value for '{}'", key))?;
            request = request.header(header_name, header_value);
        }
        for (key, value) in dynamic_headers.iter() {
            request = request.header(key, value);
        }
        if let Some(content_type) = shape.content_type {
            request = request.header(reqwest::header::CONTENT_TYPE, content_type);
        }

        request = if let Some(overlay) = auth_overlay.as_ref() {
            overlay.apply_to_request_builder(request)?
        } else {
            match binding.auth_mode {
                crate::custom_apis::CustomApiAuthMode::None
                | crate::custom_apis::CustomApiAuthMode::ApiKeyQuery => request,
                crate::custom_apis::CustomApiAuthMode::Bearer
                | crate::custom_apis::CustomApiAuthMode::OAuth2 => {
                    let token = secret.ok_or_else(|| {
                        anyhow::anyhow!(
                            "Auth secret '{}' is not configured for '{}'",
                            binding.secret_key,
                            binding.api_name
                        )
                    })?;
                    let header_name = binding.auth_header.as_deref().unwrap_or("Authorization");
                    if header_name.eq_ignore_ascii_case("authorization") {
                        request.bearer_auth(token.trim())
                    } else {
                        request.header(header_name, format!("Bearer {}", token.trim()))
                    }
                }
                crate::custom_apis::CustomApiAuthMode::ApiKeyHeader => {
                    let token = secret.ok_or_else(|| {
                        anyhow::anyhow!(
                            "Auth secret '{}' is not configured for '{}'",
                            binding.secret_key,
                            binding.api_name
                        )
                    })?;
                    let header_name = binding
                        .auth_name
                        .as_deref()
                        .or(binding.auth_header.as_deref())
                        .unwrap_or("X-API-Key");
                    request.header(header_name, token.trim())
                }
                crate::custom_apis::CustomApiAuthMode::Basic => {
                    let password = secret.ok_or_else(|| {
                        anyhow::anyhow!(
                            "Auth secret '{}' is not configured for '{}'",
                            binding.secret_key,
                            binding.api_name
                        )
                    })?;
                    request.basic_auth(
                        binding.auth_username.clone().unwrap_or_default(),
                        Some(password.trim().to_string()),
                    )
                }
            }
        };

        if let Some(body) = normalized_body {
            if binding.read_only
                && Self::custom_api_binding_supports_graphql_body(&binding)
                && !crate::custom_apis::custom_api_body_is_read_only_graphql_query(&body)
            {
                anyhow::bail!(
                    "Read-only GraphQL custom API actions only accept GraphQL query operations."
                );
            }
            request = request.json(&body);
        }

        let response = request
            .send()
            .await
            .with_context(|| format!("custom API call '{}' failed", binding.operation_name))?;
        let status = response.status();
        let content_type = response
            .headers()
            .get(reqwest::header::CONTENT_TYPE)
            .and_then(|value| value.to_str().ok())
            .unwrap_or("")
            .to_string();
        let body = response.text().await.unwrap_or_default();
        let rendered = if content_type.contains("json") {
            serde_json::from_str::<serde_json::Value>(&body)
                .ok()
                .and_then(|value| serde_json::to_string_pretty(&value).ok())
                .unwrap_or_else(|| body.clone())
        } else {
            body.clone()
        };
        let rendered = if rendered.chars().count() > 6_000 {
            format!("{}...", rendered.chars().take(6_000).collect::<String>())
        } else {
            rendered
        };
        let rendered = crate::security::redact_secret_input(&rendered).text;
        let rendered = crate::security::sanitize_untrusted_output("custom_api", &rendered);
        if !status.is_success() {
            // Status-class routing: an auth rejection is a credential problem,
            // not a request-shape problem — steering the model to mutate the
            // request on a 401/403 only burns turns. Other 4xx feed the
            // model-repair gate; 5xx are provider-side and retryable.
            if matches!(
                status,
                reqwest::StatusCode::UNAUTHORIZED | reqwest::StatusCode::FORBIDDEN
            ) {
                if let Some(storage) = self.storage() {
                    let _ = crate::custom_apis::record_custom_api_runtime_health(
                        &storage,
                        &binding.api_id,
                        false,
                        format!(
                            "Custom API '{}' returned HTTP {}.",
                            binding.operation_name, status
                        ),
                    )
                    .await;
                }
                let settings_path = format!(
                    "Settings > Integrations > Custom APIs > {}",
                    binding.api_id
                );
                return Ok(structured_tool_completion_output(
                    "custom_api",
                    "needs_credentials",
                    format!(
                        "Custom API '{}' rejected the request as unauthorized (HTTP {}). The stored credential is missing, expired, or invalid.",
                        binding.operation_name, status
                    ),
                    serde_json::json!({
                        "integration_type": "custom_api",
                        "custom_api_id": binding.api_id,
                        "operation": binding.operation_id,
                        "action_name": binding.operation_name,
                        "http_status": status.as_u16(),
                        "response_preview": rendered,
                        "settings_path": settings_path,
                        "credential_request": {
                            "kind": "custom_api_auth",
                            "api_id": binding.api_id,
                            "settings_path": settings_path,
                            "fields": [
                                {
                                    "key": "secret",
                                    "label": "API credential",
                                    "input_type": "password",
                                    "required": true
                                }
                            ],
                            "secure_input_required": true
                        },
                        "recoverable_by_model": false,
                        "retryable": false,
                        "assistant_instruction": "The provider rejected the credential, not the request shape. Do not change method, headers, query, or body to work around this. Surface the secure credential step to the user (secure credential form in chat or the settings path) and retry only after the credential is updated."
                    }),
                ));
            }
            let retryable = status.is_server_error();
            return Ok(structured_tool_completion_output(
                "custom_api",
                "failed",
                format!(
                    "Custom API '{}' returned HTTP {}.",
                    binding.operation_name, status
                ),
                serde_json::json!({
                    "integration_type": "custom_api",
                    "custom_api_id": binding.api_id,
                    "operation": binding.operation_id,
                    "action_name": binding.operation_name,
                    "http_status": status.as_u16(),
                    "response_preview": rendered,
                    "recoverable_by_model": status.is_client_error(),
                    "retryable": retryable,
                    "expected_contract": crate::core::request_contract::expected_request_contract(),
                    "assistant_instruction": if retryable {
                        "The provider failed server-side; the request shape may be fine. Retry once, and if it persists report the provider error to the user instead of mutating the request."
                    } else {
                        "Treat this as request-contract evidence. Repair method, headers, query, and body shape in one corrected call when the conversation contains enough information; ask the user only when a required non-secret request value is genuinely absent."
                    }
                }),
            ));
        }
        if Self::custom_api_binding_supports_graphql_body(&binding)
            && Self::graphql_response_has_errors(&body)
        {
            return Ok(structured_tool_completion_output(
                "custom_api",
                "failed",
                format!(
                    "Custom API '{}' returned GraphQL errors.",
                    binding.operation_name
                ),
                serde_json::json!({
                    "integration_type": "custom_api",
                    "custom_api_id": binding.api_id,
                    "operation": binding.operation_id,
                    "action_name": binding.operation_name,
                    "response_preview": rendered,
                    "recoverable_by_model": true,
                    "retryable": false,
                    "expected_contract": crate::core::request_contract::expected_request_contract(),
                    "assistant_instruction": "Treat this as request-contract evidence. Repair the GraphQL query and variables body in one corrected call when the conversation contains enough information; ask the user only when a required non-secret request value is genuinely absent."
                }),
            ));
        }
        if let Some(storage) = self.storage() {
            let _ = crate::custom_apis::record_custom_api_runtime_health(
                &storage,
                &binding.api_id,
                true,
                format!("Custom API '{}' succeeded.", binding.operation_name),
            )
            .await;
        }
        if let Some(auth_profile_id) = binding.auth_profile_id.as_deref() {
            if let Some(storage) = self.storage() {
                let _ =
                    crate::core::connectivity::auth_profiles::AuthProfileControlPlane::mark_used(
                        &storage,
                        auth_profile_id,
                    )
                    .await;
            }
        }
        Ok(format!(
            "{} {} succeeded.\n{}",
            shape.method.to_ascii_uppercase(),
            binding.operation_name,
            rendered
        ))
    }

    pub(in crate::runtime) async fn execute_extension_pack_action(
        &self,
        binding: ExtensionPackActionBinding,
        arguments: &serde_json::Value,
    ) -> Result<String> {
        crate::security::tool_args_guard::check_outward_urls_in_json_anyhow(
            arguments,
            &self.tool_args_guard_config(),
        )
        .await
        .context("Extension-pack action arguments denied by outbound URL guard")?;
        let registry = self
            .extension_pack_registry
            .as_ref()
            .ok_or_else(|| anyhow::anyhow!("Extension-pack registry not initialized"))?;
        let mut registry = registry.write().await;
        let result = registry
            .invoke_feature(
                crate::extension_packs::ExtensionPackInvokeRequest {
                    pack_id: Some(binding.pack_id.clone()),
                    connection_id: binding.connection_id.clone(),
                    feature_id: binding.feature_id.clone(),
                    arguments: arguments.clone(),
                },
                self.mcp_registry.clone(),
                self.plugin_registry.clone(),
            )
            .await?;
        if !result.ok {
            anyhow::bail!(
                "{}",
                result
                    .message
                    .or(result.error)
                    .unwrap_or_else(|| "Extension-pack invocation failed".to_string())
            );
        }
        let payload = serde_json::to_string_pretty(&result.data.unwrap_or(serde_json::Value::Null))
            .unwrap_or_else(|_| "null".to_string());
        Ok(crate::security::sanitize_untrusted_output(
            "extension_pack",
            &crate::security::redact_secret_input(&payload).text,
        ))
    }

    pub(in crate::runtime) fn value_to_http_string(value: &serde_json::Value) -> Option<String> {
        match value {
            serde_json::Value::Null => None,
            serde_json::Value::String(s) => Some(s.clone()),
            serde_json::Value::Bool(v) => Some(v.to_string()),
            serde_json::Value::Number(v) => Some(v.to_string()),
            other => serde_json::to_string(other).ok(),
        }
    }
}
