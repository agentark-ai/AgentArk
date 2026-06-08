use super::super::*;

impl ActionRuntime {
    pub(in crate::runtime) async fn execute_capability_resolve(
        &self,
        arguments: &serde_json::Value,
    ) -> Result<String> {
        let goal = arguments
            .get("goal")
            .and_then(|value| value.as_str())
            .map(str::trim)
            .filter(|value| !value.is_empty())
            .ok_or_else(|| anyhow::anyhow!("Missing 'goal' for capability resolution"))?;
        let requested_capability = arguments
            .get("requested_capability")
            .and_then(|value| value.as_str())
            .map(str::trim)
            .filter(|value| !value.is_empty());
        let selected_action = arguments
            .get("selected_action")
            .or_else(|| arguments.get("requested_action"))
            .and_then(|value| value.as_str())
            .map(str::trim)
            .filter(|value| !value.is_empty());
        let failure_output = arguments
            .get("failure_output")
            .and_then(|value| value.as_str())
            .or_else(|| arguments.get("error").and_then(|value| value.as_str()))
            .unwrap_or("");
        let files = self.collect_code_execute_files(arguments).await?;
        let detected_inputs = files
            .iter()
            .map(|file| {
                Self::upload_signature(&file.filename, file.content_type.as_deref(), &file.bytes)
            })
            .collect::<Vec<_>>();

        let requested_capability_key =
            requested_capability.map(|value| value.to_ascii_lowercase().replace([' ', '-'], "_"));
        let selected_action_key = selected_action.map(str::to_ascii_lowercase);
        let has_audio_like_file = detected_inputs.iter().any(|input| {
            matches!(
                input.get("input_type").and_then(|value| value.as_str()),
                Some("audio") | Some("audio_video")
            )
        });
        let missing_binary = Self::detect_missing_binary_from_output(failure_output);

        let mut missing_capabilities = Vec::new();
        let mut routes = Vec::new();
        let mut next_actions = Vec::new();
        let mut notes = Vec::new();

        if let Some(binary) = missing_binary.as_deref() {
            missing_capabilities.push(serde_json::json!({
                "kind": "binary",
                "name": binary,
                "approval_required": true,
                "reason": "The previous execution failed because this executable is not present in the sandbox/runtime environment.",
            }));
            routes.push(serde_json::json!({
                "route": "host_install_approval",
                "approval_required": true,
                "auto_allowed": false,
                "reason": "Sandbox-local pip/npm installs are allowed, but OS/host binary installation is approval-gated.",
            }));
        }

        if selected_action_key.as_deref() == Some("transcribe_audio") && has_audio_like_file {
            next_actions.push(serde_json::json!({
                "name": "code_execute",
                "arguments": {
                    "language": "python",
                    "code": Self::build_sandbox_transcription_code(),
                    "files": arguments.get("files").cloned().unwrap_or_else(|| serde_json::json!([])),
                    "file_payloads": arguments.get("file_payloads").cloned().unwrap_or_else(|| serde_json::json!([])),
                    "network_access": true,
                    "timeout_secs": 600,
                    "execution_contract": {
                        "phase": "validate",
                        "target_validated_when_successful": true
                    }
                },
                "why": "Run the catalog-selected transcription action inside the code sandbox after byte-level media detection. The script checks for ffmpeg and emits a structured missing-binary marker instead of installing OS packages."
            }));
            routes.push(serde_json::json!({
                "route": "sandbox_code_execute",
                "approval_required": false,
                "auto_allowed": true,
                "reason": "Use sandbox-local Python packages first; do not run host installers unless the sandbox reports a missing binary.",
            }));
            notes.push("Audio-like upload detected by bytes; prefer the selected sandbox action path before any host install.".to_string());
        }

        let pack_query = requested_capability
            .or(selected_action)
            .unwrap_or(goal)
            .trim();
        if !pack_query.is_empty() {
            if let Some(registry) = self.extension_pack_registry.as_ref() {
                let pack_search = {
                    let guard = registry.read().await;
                    guard.search_packs(Some(pack_query), None).await.ok()
                };
                if let Some(pack_search) = pack_search {
                    let top_installed = pack_search
                        .installed
                        .into_iter()
                        .take(3)
                        .collect::<Vec<_>>();
                    let top_catalog = pack_search.catalog.into_iter().take(3).collect::<Vec<_>>();
                    let mut candidate_labels = top_installed
                        .iter()
                        .map(|pack| {
                            format!(
                                "{} ({})",
                                pack.manifest.name.as_str(),
                                pack.manifest.id.as_str()
                            )
                        })
                        .collect::<Vec<_>>();
                    candidate_labels.extend(top_catalog.iter().map(|pack| {
                        format!(
                            "{} ({})",
                            pack.manifest.name.as_str(),
                            pack.manifest.id.as_str()
                        )
                    }));
                    if !candidate_labels.is_empty() {
                        notes.push(format!(
                            "Extension-pack candidates for this capability: {}.",
                            candidate_labels.join(", ")
                        ));
                        routes.push(serde_json::json!({
                            "route": "extension_pack",
                            "approval_required": false,
                            "auto_allowed": true,
                            "query": pack_query,
                            "installed_matches": top_installed.len(),
                            "catalog_matches": top_catalog.len(),
                            "requires_confirmation": top_installed.is_empty() && top_catalog.len() > 1,
                            "reason": "Use the generic extension-pack lifecycle for pack discovery, installation, auth, and action registration.",
                        }));
                    }
                    for pack in &top_installed {
                        if !pack.enabled {
                            next_actions.push(serde_json::json!({
                                "name": "extension_pack_set_enabled",
                                "arguments": {
                                    "pack_id": pack.manifest.id.clone(),
                                    "enabled": true
                                },
                                "why": format!(
                                    "Enable the installed extension pack '{}' so its registered actions can be used.",
                                    pack.manifest.name.as_str()
                                )
                            }));
                            continue;
                        }
                        if pack.runtime_required
                            && pack.runtime_status
                                != crate::extension_packs::ExtensionPackRuntimeStatus::Ready
                        {
                            notes.push(format!(
                                "Installed pack '{}' declares a local runtime that is not ready; runtime lifecycle is a separate installed-pack maintenance step, not the install/discovery path.",
                                pack.manifest.name.as_str()
                            ));
                        }
                        if pack.needs_auth
                            && matches!(
                                pack.status.as_str(),
                                "needs_auth" | "runtime_missing" | "available"
                            )
                        {
                            next_actions.push(serde_json::json!({
                                "name": "extension_pack_connect",
                                "arguments": {
                                    "pack_id": pack.manifest.id.clone()
                                },
                                "why": format!(
                                    "Create or refresh the connection record for '{}'.",
                                    pack.manifest.name.as_str()
                                )
                            }));
                        }
                    }
                    if top_installed.is_empty() && top_catalog.len() == 1 {
                        let pack = &top_catalog[0];
                        next_actions.push(serde_json::json!({
                            "name": "extension_pack_install",
                            "arguments": {
                                "pack_id": pack.manifest.id.clone()
                            },
                            "why": format!(
                                "Install the catalog integration '{}' through the shared extension-pack flow.",
                                pack.manifest.name.as_str()
                            )
                        }));
                    }
                    if top_installed.is_empty() && top_catalog.is_empty() {
                        routes.push(serde_json::json!({
                            "route": "extension_pack_scaffold",
                            "approval_required": false,
                            "auto_allowed": true,
                            "query": pack_query,
                            "reason": "No installed or catalog pack matched. Gather authoritative docs or a spec, then scaffold a draft extension pack from that source."
                        }));
                        next_actions.push(serde_json::json!({
                            "name": "extension_pack_scaffold",
                            "arguments": {
                                "name": pack_query
                            },
                            "why": "Create a draft extension pack only after the desired provider/API/channel shape is understood from the user request or authoritative docs."
                        }));
                    }
                }
            }
        }

        if let Some(requested) = requested_capability_key.as_deref() {
            notes.push(format!("Requested capability hint: {}.", requested));
        }
        if let Some(action) = selected_action_key.as_deref() {
            notes.push(format!("Selected catalog action: {}.", action));
        }
        if detected_inputs.is_empty() {
            notes.push("No upload files were provided for byte-level inspection.".to_string());
        }
        if routes.is_empty() {
            routes.push(serde_json::json!({
                "route": "inspect_then_choose",
                "approval_required": false,
                "auto_allowed": true,
                "reason": "No concrete missing capability was detected yet. Inspect with the nearest read-only/workspace tool, then retry capability_resolve with any failure output.",
            }));
        }

        let approval_required = missing_capabilities.iter().any(|capability| {
            capability
                .get("approval_required")
                .and_then(|value| value.as_bool())
                .unwrap_or(false)
        });
        let mut result = serde_json::json!({
            "resolver": "capability_resolve",
            "status": if approval_required { "needs_approval" } else { "ready" },
            "policy": "sandbox_first",
            "goal": goal,
            "detected_inputs": detected_inputs,
            "missing_capabilities": missing_capabilities,
            "acquisition_routes": routes,
            "next_actions": next_actions,
            "verification": {
                "required": true,
                "evidence": "The next action should produce successful tool output, app health/log evidence, or a concrete approval blocker."
            },
            "notes": notes,
        });

        if approval_required {
            result["approval_request"] = serde_json::json!({
                "title": "Capability approval required",
                "summary": "AgentArk detected a missing host/system capability that is not safe to install automatically.",
                "reason": missing_binary
                    .as_ref()
                    .map(|binary| format!("Missing binary: {}.", binary))
                    .unwrap_or_else(|| "A host-level capability is required.".to_string()),
                "risk_level": "environment_change",
                "risk_score": 72,
                "source": "capability_resolve",
                "comment_supported": true
            });
        }

        Ok(serde_json::to_string_pretty(&result)?)
    }

    pub(in crate::runtime) async fn execute_capability_acquire(
        &self,
        arguments: &serde_json::Value,
    ) -> Result<String> {
        let arguments = Self::enrich_capability_acquisition_arguments(arguments).await;
        let raw_name = Self::capability_string_argument(&arguments, "name")
            .or_else(|| Self::capability_string_argument(&arguments, "id"));
        let normalized_name = raw_name
            .as_deref()
            .map(Self::normalize_generated_action_name)
            .filter(|name| !name.is_empty());
        let has_endpoint = Self::capability_acquire_has_http_endpoint(&arguments);
        // Whole-payload validation: every missing requirement is reported in
        // ONE structured envelope carrying the complete expected contract, so
        // a retry can satisfy it in a single corrected call instead of
        // discovering requirements serially ("needs source" then "missing
        // name" then ...), losing already-correct fields along the way.
        if normalized_name.is_none() || !has_endpoint {
            let mut violations: Vec<String> = Vec::new();
            if raw_name.is_none() {
                violations
                    .push("Missing `name` (or `id`) for the integration record.".to_string());
            } else if normalized_name.is_none() {
                violations.push(
                    "The provided `name`/`id` normalizes to an empty action name; provide a name with at least one alphanumeric character."
                        .to_string(),
                );
            }
            if !has_endpoint {
                violations.push(format!(
                    "Missing an HTTP endpoint or source: provide `base_url`, an operation entry with an absolute URL, or one source field ({}).",
                    crate::core::request_contract::SOURCE_ALIAS_KEYS.join(", ")
                ));
            }
            return Ok(structured_tool_completion_output(
                "capability_acquire",
                "needs_arguments",
                "API integration acquisition is missing required fields; satisfy the complete contract in one corrected call.",
                serde_json::json!({
                    "violations": violations,
                    "expected_contract": crate::core::request_contract::expected_custom_api_acquisition_contract(),
                    "provided": Self::capability_payload_key_summary(&arguments),
                    "assistant_instruction": "Repair ALL listed violations in one corrected call, reusing values already present in the conversation (for example a URL the user already provided). capability_acquire only saves API integrations; use the Skills import/create flow for user skills, or extension-pack actions for manifest-based integrations. Ask the user only when a required non-secret value is genuinely absent."
                }),
            ));
        }
        let name = normalized_name.expect("validated above");
        let description = Self::capability_string_argument(&arguments, "description")
            .unwrap_or_else(|| "Saved custom API integration".to_string());
        self.execute_capability_acquire_custom_api(&arguments, &name, &description)
            .await
    }

    /// Sorted top-level argument keys, so contract violations can show the
    /// model what it DID provide next to what is missing.
    fn capability_payload_key_summary(arguments: &serde_json::Value) -> serde_json::Value {
        let keys = arguments
            .as_object()
            .map(|object| {
                let mut keys = object.keys().cloned().collect::<Vec<_>>();
                keys.sort();
                keys
            })
            .unwrap_or_default();
        serde_json::json!({ "keys": keys })
    }

    pub(in crate::runtime) async fn execute_custom_api_request(
        &self,
        arguments: &serde_json::Value,
    ) -> Result<String> {
        let storage = self
            .storage
            .as_ref()
            .ok_or_else(|| anyhow::anyhow!("Storage is required to use saved custom APIs"))?;
        let id = Self::capability_string_argument(arguments, "id")
            .or_else(|| Self::capability_string_argument(arguments, "query"))
            .ok_or_else(|| anyhow::anyhow!("custom_api_request requires a saved custom API id"))?;
        let apis = crate::custom_apis::list_custom_apis(storage, &self.config_dir, self.data_dir())
            .await?;
        let api = apis
            .into_iter()
            .find(|item| Self::custom_api_view_matches_selector(item, &id))
            .ok_or_else(|| anyhow::anyhow!("Saved custom API '{}' was not found", id))?;
        if !Self::custom_api_view_is_callable(&api) {
            anyhow::bail!(
                "Saved custom API '{}' is not enabled or does not have callable credentials.",
                api.config.id
            );
        }

        let selector = Self::capability_string_argument(arguments, "operation")
            .or_else(|| Self::capability_string_argument(arguments, "operation_id"))
            .or_else(|| Self::capability_string_argument(arguments, "action_name"));
        let raw_explicit_request_body = arguments
            .get("arguments")
            .and_then(|value| value.get("body"))
            .or_else(|| arguments.get("body"))
            .cloned();
        let has_explicit_body = raw_explicit_request_body.is_some();
        let selection_body = raw_explicit_request_body.as_ref().map(|body| {
            let mut candidates = api
                .config
                .operations
                .iter()
                .filter(|operation| operation.draft.enabled)
                .map(|operation| {
                    Self::normalize_custom_api_request_body_value(operation, body.clone())
                });
            candidates.next().unwrap_or_else(|| body.clone())
        });
        let (operation, selection_reason) = match Self::select_custom_api_read_operation(
            api.config.operations.iter(),
            selector.as_deref(),
            selection_body.as_ref(),
            has_explicit_body,
        ) {
            Some(selection) => selection,
            None => {
                return Ok(structured_tool_completion_output(
                    "custom_api_request",
                    "needs_operation",
                    "Choose one saved read-only operation for this custom API request.",
                    serde_json::json!({
                        "integration_type": "custom_api",
                        "custom_api_id": api.config.id,
                        "reason": if selector.is_some() { "operation_not_found" } else { "operation_required" },
                        "message": "Choose one saved read-only operation for this custom API request.",
                        "capability_contract": crate::custom_apis::capability_contract(&api.config, api.secret_configured),
                        "available_operations": api.config.operations.iter()
                            .filter(|operation| operation.draft.enabled)
                            .map(crate::custom_apis::operation_contract)
                            .collect::<Vec<_>>(),
                    }),
                ));
            }
        };
        let normalized_explicit_request_body = raw_explicit_request_body
            .map(|body| Self::normalize_custom_api_request_body_value(operation, body));
        let request_body = normalized_explicit_request_body
            .as_ref()
            .or(operation.draft.default_body.as_ref());
        if !Self::custom_api_operation_allows_read_request(operation, request_body) {
            anyhow::bail!(
                "Saved custom API operation '{}' is not read-only for the supplied request; use its generated action with the normal approval path.",
                operation.action_name
            );
        }
        let api_id = api.config.id.clone();
        let operation_id = operation.draft.id.clone();
        let action_name = operation.action_name.clone();
        let requested_operation = selector.clone();

        let action_arguments = if let Some(object) = arguments
            .get("arguments")
            .and_then(|value| value.as_object())
        {
            serde_json::Value::Object(object.clone())
        } else {
            let mut object = serde_json::Map::new();
            if let Some(input) = arguments.as_object() {
                for (key, value) in input {
                    if matches!(
                        key.as_str(),
                        "id" | "query"
                            | "operation"
                            | "operation_id"
                            | "action_name"
                            | "integration"
                            | "kind"
                            | "op"
                    ) {
                        continue;
                    }
                    object.insert(key.clone(), value.clone());
                }
            }
            serde_json::Value::Object(object)
        };
        let action_arguments =
            Self::normalize_custom_api_request_action_arguments(operation, action_arguments);
        let missing_inputs =
            crate::custom_apis::operation_missing_required_inputs(operation, &action_arguments);
        if !missing_inputs.is_empty() {
            return Ok(structured_tool_completion_output(
                "custom_api_request",
                "needs_arguments",
                "This saved operation requires additional non-secret request arguments before it can run.",
                serde_json::json!({
                    "integration_type": "custom_api",
                    "custom_api_id": api_id,
                    "operation": operation_id,
                    "action_name": action_name,
                    "missing_inputs": missing_inputs,
                    "operation_contract": crate::custom_apis::operation_contract(operation),
                    "message": "This saved operation requires additional non-secret request arguments before it can run.",
                }),
            ));
        }
        let output = Box::pin(self.execute_action_with_context(
            &action_name,
            &action_arguments,
            &crate::actions::ActionAuthorizationContext {
                principal: Some(crate::actions::ActionCallerPrincipal::local_admin(
                    "custom_api_request",
                )),
                surface: crate::actions::ActionExecutionSurface::Chat,
                direct_user_intent: true,
                current_turn_is_explicit_approval: false,
                agent_name: None,
                agent_access_scope: None,
                capability_context_id: None,
                request_timezone: None,
                request_timezone_offset_minutes: None,
            },
        ))
        .await?;
        Ok(serde_json::to_string_pretty(&serde_json::json!({
            "status": "completed",
            "integration_type": "custom_api",
            "custom_api_id": api_id,
            "requested_operation": requested_operation,
            "operation": operation_id,
            "action_name": action_name,
            "operation_selection": selection_reason,
            "result": output,
        }))?)
    }

    pub(in crate::runtime) async fn execute_custom_api_manage(
        &self,
        arguments: &serde_json::Value,
    ) -> Result<String> {
        let storage = self
            .storage
            .as_ref()
            .ok_or_else(|| anyhow::anyhow!("Storage is required to manage saved custom APIs"))?;
        let id = Self::capability_string_argument(arguments, "id")
            .ok_or_else(|| anyhow::anyhow!("custom_api_manage requires id"))?;
        let operation = Self::capability_string_argument(arguments, "operation")
            .map(|value| value.to_ascii_lowercase())
            .ok_or_else(|| anyhow::anyhow!("custom_api_manage requires operation"))?;
        match operation.as_str() {
            "delete" => {
                crate::custom_apis::delete_custom_api(
                    storage,
                    &self.config_dir,
                    self.data_dir(),
                    self,
                    &id,
                )
                .await?;
                Ok(serde_json::to_string_pretty(&serde_json::json!({
                    "status": "deleted",
                    "integration_type": "custom_api",
                    "id": id,
                }))?)
            }
            "enable" | "disable" => {
                let api = crate::custom_apis::list_custom_apis(
                    storage,
                    &self.config_dir,
                    self.data_dir(),
                )
                .await?
                .into_iter()
                .find(|item| item.config.id == id)
                .ok_or_else(|| anyhow::anyhow!("Custom API '{}' was not found", id))?;
                let enabled = operation == "enable";
                let view = crate::custom_apis::upsert_custom_api(
                    storage,
                    &self.config_dir,
                    self.data_dir(),
                    self,
                    crate::custom_apis::CustomApiUpsertRequest {
                        id: Some(api.config.id.clone()),
                        name: api.config.name.clone(),
                        description: Some(api.config.description.clone()),
                        base_url: api.config.base_url.clone(),
                        enabled: Some(enabled),
                        auth_mode: Some(api.config.auth_mode),
                        auth_profile_id: api.config.auth_profile_id.clone(),
                        auth_header: api.config.auth_header.clone(),
                        auth_name: api.config.auth_name.clone(),
                        auth_username: api.config.auth_username.clone(),
                        secret: None,
                        clear_secret: None,
                        allow_missing_secret: Some(true),
                        operations: api
                            .config
                            .operations
                            .iter()
                            .map(|operation| operation.draft.clone())
                            .collect(),
                    },
                    Some(&api.config.id),
                )
                .await?;
                Ok(serde_json::to_string_pretty(&serde_json::json!({
                    "status": if enabled { "enabled" } else { "disabled" },
                    "integration_type": "custom_api",
                    "id": view.config.id,
                    "action_count": view.action_count,
                }))?)
            }
            _ => anyhow::bail!(
                "Unsupported custom API management operation '{}'",
                operation
            ),
        }
    }

    pub(in crate::runtime) fn capability_acquire_has_http_endpoint(
        arguments: &serde_json::Value,
    ) -> bool {
        let kind = Self::capability_string_argument(arguments, "kind")
            .unwrap_or_default()
            .to_ascii_lowercase();
        if kind == "web_automation" {
            return false;
        }
        Self::capability_string_argument(arguments, "base_url").is_some()
            || Self::capability_string_argument(arguments, "path")
                .as_deref()
                .is_some_and(|path| path.starts_with("http://") || path.starts_with("https://"))
            || crate::core::request_contract::has_source_alias(arguments)
            || Self::capability_string_argument(arguments, "id").is_some()
            || arguments
                .get("operation")
                .and_then(|value| value.as_object())
                .is_some_and(|object| {
                    object
                        .get("base_url")
                        .and_then(|value| value.as_str())
                        .is_some()
                        || object
                            .get("path")
                            .and_then(|value| value.as_str())
                            .is_some()
                        || object.get("url").and_then(|value| value.as_str()).is_some()
                })
            || arguments
                .get("operations")
                .and_then(|value| value.as_array())
                .is_some_and(|operations| {
                    operations.iter().any(|operation| {
                        operation.as_object().is_some_and(|object| {
                            object
                                .get("base_url")
                                .and_then(|value| value.as_str())
                                .is_some()
                                || object
                                    .get("path")
                                    .and_then(|value| value.as_str())
                                    .is_some()
                                || object.get("url").and_then(|value| value.as_str()).is_some()
                        })
                    })
                })
    }

    pub(in crate::runtime) fn custom_api_preview_request_from_source_contract(
        arguments: &serde_json::Value,
        request_name: String,
    ) -> Option<crate::custom_apis::CustomApiPreviewRequest> {
        let (key, source) = crate::core::request_contract::source_alias_value(arguments)?;
        let mut request = crate::custom_apis::CustomApiPreviewRequest {
            name: Some(request_name),
            base_url: Self::capability_string_argument(arguments, "base_url"),
            source: None,
            openapi_url: None,
            openapi_text: None,
            curl_text: None,
        };
        match key {
            "openapi_url" => request.openapi_url = Some(source),
            "openapi_text" => request.openapi_text = Some(source),
            "curl_text" => request.curl_text = Some(source),
            _ => request.source = Some(source),
        }
        Some(request)
    }

    pub(in crate::runtime) fn capability_arguments_have_operation_shape(
        arguments: &serde_json::Value,
    ) -> bool {
        let has_operation_object = arguments
            .get("operation")
            .and_then(|value| value.as_object())
            .is_some();
        let has_operations_array = arguments
            .get("operations")
            .and_then(|value| value.as_array())
            .is_some_and(|items| !items.is_empty());
        has_operation_object
            || has_operations_array
            || [
                "path",
                "method",
                "default_headers",
                "headers",
                "default_query",
                "required_inputs",
                "read_only",
                "body_required",
                "body",
                "body_template",
                "parameters",
                "response_notes",
            ]
            .iter()
            .any(|key| arguments.get(*key).is_some())
    }

    pub(in crate::runtime) fn capability_acquire_source_evidence_kinds(
        arguments: &serde_json::Value,
    ) -> Vec<String> {
        let mut kinds = BTreeSet::new();
        if let Some((key, _)) = crate::core::request_contract::source_alias_value(arguments) {
            match key {
                "openapi_url" | "openapi_text" => {
                    kinds.insert("openapi".to_string());
                }
                "curl_text" => {
                    kinds.insert("curl".to_string());
                }
                _ => {
                    kinds.insert("docs".to_string());
                }
            }
        }
        if let Some(items) = arguments
            .get("_capability_source_evidence")
            .and_then(|value| value.as_array())
        {
            for item in items {
                if let Some(kind) = item
                    .as_str()
                    .map(str::trim)
                    .filter(|value| !value.is_empty())
                {
                    kinds.insert(kind.to_ascii_lowercase());
                }
            }
        }
        kinds.into_iter().collect()
    }

    pub(in crate::runtime) fn capability_acquire_needs_source_for_custom_api(
        arguments: &serde_json::Value,
        existing_config_present: bool,
    ) -> bool {
        let source_kinds = Self::capability_acquire_source_evidence_kinds(arguments);
        let has_operation_shape = Self::capability_arguments_have_operation_shape(arguments);
        let has_executable_source_contract =
            source_kinds.iter().any(|kind| kind == "source_contract")
                || (source_kinds
                    .iter()
                    .any(|kind| matches!(kind.as_str(), "openapi" | "docs"))
                    && has_operation_shape);
        if has_executable_source_contract {
            return false;
        }
        if !existing_config_present {
            return true;
        }
        has_operation_shape
    }

    pub(in crate::runtime) fn capability_acquire_unverified_contract_fields(
        arguments: &serde_json::Value,
    ) -> Vec<String> {
        let mut fields = BTreeSet::new();
        for key in [
            "operation",
            "operations",
            "path",
            "method",
            "default_headers",
            "headers",
            "default_query",
            "required_inputs",
            "read_only",
            "body_required",
            "body",
            "body_template",
            "parameters",
            "response_notes",
        ] {
            if arguments.get(key).is_some() {
                fields.insert(key.to_string());
            }
        }
        if let Some(operation) = arguments
            .get("operation")
            .and_then(|value| value.as_object())
        {
            for key in operation.keys() {
                fields.insert(format!("operation.{key}"));
            }
        }
        if let Some(operations) = arguments
            .get("operations")
            .and_then(|value| value.as_array())
        {
            if !operations.is_empty() {
                fields.insert("operations[]".to_string());
            }
        }
        fields.into_iter().collect()
    }

    pub(in crate::runtime) async fn capability_acquire_needs_source_output(
        &self,
        target_id: &str,
        arguments: &serde_json::Value,
        existing_config_present: bool,
        draft_saved: bool,
    ) -> Result<String> {
        let search_config = build_search_config(&self.config_dir, self.storage.as_ref()).await;
        let configured_source_backends = search_config.configured_source_backend_names();
        let search_provider_configured = search_config.has_configured_source_backend();
        let accepted_sources = crate::core::request_contract::SOURCE_ALIAS_KEYS.to_vec();
        let message = if draft_saved {
            "AgentArk needs authoritative source evidence before installing executable integration operations. The non-secret fields were saved as a disabled draft; retry with ONLY the missing source evidence — already-saved fields do not need to be restated."
        } else {
            "AgentArk needs authoritative source evidence before installing or changing executable integration operations. No custom API integration was saved or modified."
        };
        Ok(structured_tool_completion_output(
            "capability_acquire",
            "needs_source",
            message,
            serde_json::json!({
                "reason": "unverified_integration_contract",
                "integration_type": "custom_api",
                "id": target_id,
                "existing_config_present": existing_config_present,
                "draft_saved": draft_saved,
                "source_required": true,
                "accepted_sources": accepted_sources,
                "expected_contract": crate::core::request_contract::expected_custom_api_acquisition_contract(),
                "unverified_contract_fields": Self::capability_acquire_unverified_contract_fields(arguments),
                "search_provider_configured": search_provider_configured,
                "configured_source_backends": configured_source_backends,
                "search_provider_setup": if search_provider_configured {
                    serde_json::Value::Null
                } else {
                    serde_json::Value::String(crate::actions::search::SEARCH_PROVIDER_SETUP_REQUIRED_MESSAGE.to_string())
                },
                "message": message,
                "next_actions": [
                    "Provide an OpenAPI document, provider documentation text/URL, or provider manifest.",
                    "Configure a reliable API/SearXNG search provider so AgentArk can look up official integration docs.",
                    "Retry capability acquisition after source evidence is available."
                ]
            }),
        ))
    }

    /// PROTOCOL-SHAPE REGISTRY: some protocols fully define their request
    /// contract at the protocol level, so no per-operation derivation (and
    /// no source evidence beyond the endpoint itself) is needed to produce
    /// an executable integration. For those endpoints the substrate
    /// scaffolds the protocol's generic operation deterministically.
    ///
    /// Arms are detected purely by ENDPOINT SHAPE (base_url/path), never by
    /// docs URLs, provider names, or hostnames. Current arms:
    ///   - GraphQL: single endpoint, POST {query, variables} JSON body.
    /// Extension point: add new arms here for other self-describing
    /// protocols (e.g. JSON-RPC single-endpoint POST envelopes) — each arm
    /// must route through the draft normalizer so the protocol's guards
    /// (read-only, Content-Type, required body) apply uniformly.
    ///
    /// Returns None when no protocol arm matches; callers then degrade to
    /// declared operations or structured needs_source teaching.
    pub(in crate::runtime) fn protocol_defined_request_fallback(
        arguments: &serde_json::Value,
        existing_config: Option<&crate::custom_apis::CustomApiConfig>,
        target_id: &str,
        request_name: &str,
        request_description: &str,
    ) -> Option<Result<(crate::custom_apis::CustomApiUpsertRequest, usize)>> {
        let raw_base = Self::capability_string_argument(arguments, "base_url")
            .or_else(|| existing_config.map(|config| config.base_url.clone()))?;
        let parsed = reqwest::Url::parse(raw_base.trim()).ok()?;
        let url_path = parsed.path().to_string();
        let no_headers = std::collections::BTreeMap::new();
        let argument_path = Self::capability_string_argument(arguments, "path");
        let (root, graphql_path) = if crate::core::request_contract::endpoint_has_graphql_signal(
            &url_path,
            &no_headers,
        ) {
            let mut root = parsed.clone();
            root.set_path("");
            root.set_query(None);
            (root.as_str().trim_end_matches('/').to_string(), url_path)
        } else if argument_path.as_deref().is_some_and(|path| {
            crate::core::request_contract::endpoint_has_graphql_signal(path, &no_headers)
        }) {
            (
                raw_base.trim().trim_end_matches('/').to_string(),
                argument_path.unwrap_or_default(),
            )
        } else {
            return None;
        };
        let draft_arguments = serde_json::json!({
            "base_url": root,
            "method": "post",
            "path": graphql_path,
        });
        Some(
            Self::capability_operation_draft(&draft_arguments, target_id, request_description)
                .map(|(base_url, operation, _)| {
                    let (auth_mode, auth_header, auth_name, auth_username) =
                        Self::capability_auth_fields(arguments);
                    (
                        crate::custom_apis::CustomApiUpsertRequest {
                            id: Some(target_id.to_string()),
                            name: request_name.to_string(),
                            description: Some(request_description.to_string()),
                            base_url,
                            enabled: Some(true),
                            auth_mode: auth_mode
                                .or(existing_config.map(|config| config.auth_mode)),
                            auth_profile_id: None,
                            auth_header,
                            auth_name,
                            auth_username,
                            secret: None,
                            clear_secret: None,
                            allow_missing_secret: Some(true),
                            operations: vec![operation],
                        },
                        1usize,
                    )
                }),
        )
    }

    /// Persists the validated non-secret fields of a rejected acquisition as
    /// a DISABLED draft so retries are deltas instead of restarts: the create
    /// path backfills base_url/name/description/auth from the existing
    /// record, so a follow-up call that only adds source evidence still
    /// lands a complete integration. Returns whether a draft was saved.
    async fn save_custom_api_acquisition_draft(
        &self,
        target_id: &str,
        request_name: &str,
        request_description: &str,
        arguments: &serde_json::Value,
    ) -> bool {
        let Some(storage) = self.storage.as_ref() else {
            return false;
        };
        let Some(base_url) = Self::capability_string_argument(arguments, "base_url")
            .map(|value| value.trim().to_string())
            .filter(|value| !value.is_empty())
        else {
            // Without a base_url there is nothing durable worth parking; the
            // whole-contract envelope already teaches every required field.
            return false;
        };
        let auth_header =
            Self::capability_auth_string_argument(arguments, &["auth_header_name", "auth_header"]);
        let request = crate::custom_apis::CustomApiUpsertRequest {
            id: Some(target_id.to_string()),
            name: request_name.to_string(),
            description: Some(request_description.to_string()),
            base_url,
            // Drafts are parked, never executable: disabled, no operations.
            enabled: Some(false),
            auth_mode: Self::capability_auth_mode(arguments),
            auth_profile_id: None,
            auth_header: auth_header.clone(),
            auth_name: auth_header,
            auth_username: None,
            secret: None,
            clear_secret: None,
            allow_missing_secret: Some(true),
            operations: Vec::new(),
        };
        match crate::custom_apis::upsert_custom_api(
            storage,
            &self.config_dir,
            self.data_dir(),
            self,
            request,
            None,
        )
        .await
        {
            Ok(_) => true,
            Err(error) => {
                tracing::debug!(
                    "Custom API acquisition draft save skipped id={}: {}",
                    target_id,
                    error
                );
                false
            }
        }
    }

    pub(in crate::runtime) fn capability_auth_mode(
        arguments: &serde_json::Value,
    ) -> Option<crate::custom_apis::CustomApiAuthMode> {
        let auth_mode =
            Self::capability_auth_string_argument(arguments, &["auth_type", "auth_mode"])?;
        match auth_mode.to_ascii_lowercase().as_str() {
            "bearer" => Some(crate::custom_apis::CustomApiAuthMode::Bearer),
            "api_key_header" => Some(crate::custom_apis::CustomApiAuthMode::ApiKeyHeader),
            "api_key" => Some(crate::custom_apis::CustomApiAuthMode::ApiKeyHeader),
            "header" => Some(crate::custom_apis::CustomApiAuthMode::ApiKeyHeader),
            "api_key_query" => Some(crate::custom_apis::CustomApiAuthMode::ApiKeyQuery),
            "query" => Some(crate::custom_apis::CustomApiAuthMode::ApiKeyQuery),
            "oauth2" => Some(crate::custom_apis::CustomApiAuthMode::OAuth2),
            "basic" => Some(crate::custom_apis::CustomApiAuthMode::Basic),
            "none" => Some(crate::custom_apis::CustomApiAuthMode::None),
            _ => None,
        }
    }

    pub(in crate::runtime) fn capability_auth_string_argument(
        arguments: &serde_json::Value,
        keys: &[&str],
    ) -> Option<String> {
        keys.iter()
            .find_map(|key| Self::capability_string_argument(arguments, key))
            .or_else(|| {
                arguments.get("auth").and_then(|auth| {
                    keys.iter().find_map(|key| {
                        auth.get(key)
                            .or_else(|| {
                                if *key == "auth_type" {
                                    auth.get("type").or_else(|| auth.get("kind"))
                                } else if *key == "auth_mode" {
                                    auth.get("mode")
                                } else if *key == "auth_header_name" {
                                    auth.get("header_name")
                                        .or_else(|| auth.get("header"))
                                        .or_else(|| auth.get("name"))
                                } else {
                                    None
                                }
                            })
                            .and_then(|value| value.as_str())
                            .map(str::trim)
                            .filter(|value| !value.is_empty())
                            .map(ToString::to_string)
                    })
                })
            })
    }

    pub(in crate::runtime) fn capability_object_to_string_map(
        value: Option<&serde_json::Value>,
    ) -> BTreeMap<String, String> {
        value
            .and_then(|value| value.as_object())
            .map(|object| {
                object
                    .iter()
                    .filter_map(|(key, value)| {
                        Self::value_to_http_string(value)
                            .map(|value| (key.trim().to_string(), value.trim().to_string()))
                    })
                    .filter(|(key, _)| !key.is_empty())
                    .collect::<BTreeMap<_, _>>()
            })
            .unwrap_or_default()
    }

    pub(in crate::runtime) fn normalized_default_body_value(
        value: serde_json::Value,
    ) -> serde_json::Value {
        match value {
            serde_json::Value::String(raw) => {
                let trimmed = raw.trim();
                if trimmed.starts_with('{') || trimmed.starts_with('[') {
                    serde_json::from_str(trimmed).unwrap_or_else(|_| serde_json::Value::String(raw))
                } else {
                    serde_json::Value::String(raw)
                }
            }
            other => other,
        }
    }

    pub(in crate::runtime) fn capability_endpoint_parts(
        arguments: &serde_json::Value,
    ) -> Result<(String, String, BTreeMap<String, String>)> {
        let raw_base = Self::capability_string_argument(arguments, "base_url");
        let raw_path = Self::capability_string_argument(arguments, "path");
        let endpoint = match (raw_base.as_deref(), raw_path.as_deref()) {
            (_, Some(path)) if path.starts_with("http://") || path.starts_with("https://") => {
                path.to_string()
            }
            (Some(base), Some(path)) if !path.trim().is_empty() => {
                format!(
                    "{}/{}",
                    base.trim_end_matches('/'),
                    path.trim_start_matches('/')
                )
            }
            (Some(base), _) => base.to_string(),
            _ => {
                return Err(anyhow::anyhow!(
                    "HTTP/API capability acquisition requires a base_url or absolute path"
                ));
            }
        };
        let parsed = reqwest::Url::parse(endpoint.as_str())
            .with_context(|| format!("Invalid API endpoint '{}'", endpoint))?;
        let host = parsed
            .host_str()
            .ok_or_else(|| anyhow::anyhow!("API endpoint must include a host"))?;
        let mut base_url = format!("{}://{}", parsed.scheme(), host);
        if let Some(port) = parsed.port() {
            base_url.push(':');
            base_url.push_str(&port.to_string());
        }
        let path = if parsed.path().trim().is_empty() {
            "/".to_string()
        } else {
            parsed.path().to_string()
        };
        let mut query = BTreeMap::new();
        for (key, value) in parsed.query_pairs() {
            query.insert(key.to_string(), value.to_string());
        }
        Ok((base_url, path, query))
    }

    pub(in crate::runtime) fn capability_operation_draft(
        arguments: &serde_json::Value,
        name: &str,
        description: &str,
    ) -> Result<(
        String,
        crate::custom_apis::CustomApiOperationDraft,
        BTreeMap<String, String>,
    )> {
        let operation_value = arguments.get("operation").cloned();
        let operation_object = operation_value.as_ref().and_then(|value| value.as_object());
        let mut endpoint_arguments = arguments.clone();
        if let (Some(root), Some(operation)) =
            (endpoint_arguments.as_object_mut(), operation_object)
        {
            for key in ["base_url", "path"] {
                if !root.contains_key(key) {
                    if let Some(value) = operation.get(key).cloned() {
                        root.insert(key.to_string(), value);
                    }
                }
            }
            if !root.contains_key("path") {
                if let Some(value) = operation.get("url").cloned() {
                    root.insert("path".to_string(), value);
                }
            }
        }
        let operation_string = |key: &str| {
            operation_object
                .and_then(|object| object.get(key))
                .and_then(|value| value.as_str())
                .map(|value| value.trim())
                .filter(|value| !value.is_empty())
                .map(ToString::to_string)
        };
        let operation_bool = |key: &str| {
            operation_object
                .and_then(|object| object.get(key))
                .and_then(|value| value.as_bool())
        };
        let (base_url, path, mut default_query) =
            Self::capability_endpoint_parts(&endpoint_arguments)?;
        default_query.extend(Self::capability_object_to_string_map(
            arguments.get("default_query"),
        ));
        if let Some(operation) = operation_object {
            default_query.extend(Self::capability_object_to_string_map(
                operation.get("default_query"),
            ));
        }
        let method = operation_string("method")
            .or_else(|| Self::capability_string_argument(arguments, "method"))
            .unwrap_or_else(|| "get".to_string())
            .to_ascii_uppercase();
        let mut default_headers =
            Self::capability_object_to_string_map(arguments.get("default_headers"));
        if let Some(operation) = operation_object {
            default_headers.extend(Self::capability_object_to_string_map(
                operation
                    .get("default_headers")
                    .or_else(|| operation.get("headers")),
            ));
        }
        let requested_read_only = operation_bool("read_only")
            .or_else(|| arguments.get("read_only").and_then(|value| value.as_bool()));
        let mut read_only = requested_read_only.unwrap_or(method == "GET");
        let required_inputs = if operation_object
            .and_then(|object| object.get("required_inputs"))
            .is_some()
            && arguments.get("required_inputs").is_none()
        {
            Self::capability_acquire_required_inputs(operation_value.as_ref().unwrap())
        } else {
            Self::capability_acquire_required_inputs(arguments)
        };
        let mut parameters = Vec::new();
        let mut body_required = operation_bool("body_required").unwrap_or(false);
        for input in required_inputs {
            if input.eq_ignore_ascii_case("body") {
                body_required = true;
                continue;
            }
            let location = if path.contains(&format!("{{{}}}", input))
                || path.contains(&format!(":{}", input))
            {
                crate::custom_apis::CustomApiParameterLocation::Path
            } else if method == "GET" {
                crate::custom_apis::CustomApiParameterLocation::Query
            } else {
                body_required = true;
                continue;
            };
            parameters.push(crate::custom_apis::CustomApiParameter {
                name: input,
                location,
                required: true,
                description: None,
                schema_type: Some("string".to_string()),
            });
        }
        if let Some(parameter_value) = operation_object
            .and_then(|object| object.get("parameters"))
            .cloned()
        {
            if let Ok(mut operation_parameters) = serde_json::from_value::<
                Vec<crate::custom_apis::CustomApiParameter>,
            >(parameter_value)
            {
                if operation_parameters.iter().any(|parameter| {
                    matches!(
                        parameter.location,
                        crate::custom_apis::CustomApiParameterLocation::Body
                    )
                }) {
                    body_required = true;
                }
                parameters.append(&mut operation_parameters);
            }
        }
        let default_body = operation_object
            .and_then(|object| object.get("body").cloned())
            .or_else(|| arguments.get("body").cloned())
            .or_else(|| operation_object.and_then(|object| object.get("body_template").cloned()))
            .or_else(|| arguments.get("body_template").cloned())
            .map(Self::normalized_default_body_value)
            .filter(|value| !value.is_null());
        let has_body_template = default_body.is_some();
        let graphql_body_endpoint = crate::custom_apis::custom_api_operation_supports_graphql_body(
            &method,
            &path,
            &default_headers,
            body_required || has_body_template || method == "POST",
        );
        if requested_read_only.is_none() && graphql_body_endpoint {
            read_only = default_body
                .as_ref()
                .map(crate::custom_apis::custom_api_body_is_read_only_graphql_query)
                .unwrap_or(true);
        }
        if method != "GET" && method != "DELETE" {
            body_required = body_required || has_body_template || graphql_body_endpoint;
            parameters.push(crate::custom_apis::CustomApiParameter {
                name: "body".to_string(),
                location: crate::custom_apis::CustomApiParameterLocation::Body,
                required: body_required,
                description: Some(if graphql_body_endpoint {
                    "GraphQL request body. Read-only actions accept query operations only."
                        .to_string()
                } else {
                    "JSON request body for this endpoint".to_string()
                }),
                schema_type: Some("object".to_string()),
            });
        }
        let operation_id = operation_string("id")
            .or_else(|| operation_string("operation_id"))
            .map(|value| Self::normalize_generated_action_name(&value))
            .unwrap_or_else(|| {
                Self::normalize_generated_action_name(&format!("{} {}", method, path))
            });
        let response_notes = operation_string("response_notes")
            .or_else(|| Self::capability_string_argument(arguments, "response_notes"));
        let operation_description = response_notes
            .filter(|notes| !notes.eq_ignore_ascii_case(description))
            .map(|notes| format!("{} {}", description.trim(), notes.trim()))
            .unwrap_or_else(|| description.trim().to_string());
        let operation_name =
            operation_string("name").unwrap_or_else(|| format!("{} {}", method, path));
        let draft = crate::custom_apis::normalize_operation_draft(
            crate::custom_apis::CustomApiOperationDraft {
                id: if operation_id.is_empty() {
                    format!("{}-request", name)
                } else {
                    operation_id
                },
                name: operation_name,
                method,
                path,
                description: operation_description,
                read_only,
                enabled: true,
                default_headers: default_headers.clone(),
                default_query,
                parameters,
                body_required,
                default_body,
            },
        );
        Ok((base_url, draft, default_headers))
    }

    pub(in crate::runtime) fn capability_operation_drafts(
        arguments: &serde_json::Value,
        name: &str,
        description: &str,
    ) -> Result<(String, Vec<crate::custom_apis::CustomApiOperationDraft>)> {
        if let Some(operations) = arguments
            .get("operations")
            .and_then(|value| value.as_array())
            .filter(|items| !items.is_empty())
        {
            let mut base_url: Option<String> = None;
            let mut drafts = Vec::new();
            for operation in operations {
                let mut operation_arguments = arguments.clone();
                if let Some(object) = operation_arguments.as_object_mut() {
                    object.insert("operation".to_string(), operation.clone());
                    object.remove("operations");
                }
                let (operation_base_url, draft, _) =
                    Self::capability_operation_draft(&operation_arguments, name, description)?;
                if base_url
                    .as_deref()
                    .is_some_and(|existing| existing != operation_base_url)
                {
                    anyhow::bail!(
                        "Custom API operation updates must use one base URL per integration."
                    );
                }
                base_url = Some(operation_base_url);
                drafts.push(draft);
            }
            let base_url = base_url.ok_or_else(|| {
                anyhow::anyhow!("HTTP/API capability acquisition requires at least one operation")
            })?;
            return Ok((base_url, drafts));
        }

        let (base_url, operation, _) =
            Self::capability_operation_draft(arguments, name, description)?;
        Ok((base_url, vec![operation]))
    }

    pub(in crate::runtime) fn capability_auth_fields(
        arguments: &serde_json::Value,
    ) -> (
        Option<crate::custom_apis::CustomApiAuthMode>,
        Option<String>,
        Option<String>,
        Option<String>,
    ) {
        let Some(mode) = Self::capability_auth_mode(arguments) else {
            return (None, None, None, None);
        };
        let header = Self::capability_auth_string_argument(
            arguments,
            &["auth_header_name", "auth_header", "auth_name"],
        );
        match mode {
            crate::custom_apis::CustomApiAuthMode::Bearer
            | crate::custom_apis::CustomApiAuthMode::OAuth2 => (
                Some(mode),
                Some(header.unwrap_or_else(|| "Authorization".to_string())),
                None,
                None,
            ),
            crate::custom_apis::CustomApiAuthMode::ApiKeyHeader => (
                Some(mode),
                None,
                Some(header.unwrap_or_else(|| "X-API-Key".to_string())),
                None,
            ),
            crate::custom_apis::CustomApiAuthMode::ApiKeyQuery => (Some(mode), None, header, None),
            crate::custom_apis::CustomApiAuthMode::Basic => (Some(mode), None, None, None),
            crate::custom_apis::CustomApiAuthMode::None => (Some(mode), None, None, None),
        }
    }

    pub(in crate::runtime) async fn execute_capability_acquire_custom_api(
        &self,
        arguments: &serde_json::Value,
        name: &str,
        description: &str,
    ) -> Result<String> {
        let storage = self
            .storage
            .as_ref()
            .ok_or_else(|| anyhow::anyhow!("Storage is required to save custom integrations"))?;
        let allow_duplicate = arguments
            .get("allow_duplicate")
            .and_then(|value| value.as_bool())
            .unwrap_or(false);
        let target_id = Self::capability_string_argument(arguments, "id")
            .map(|value| Self::normalize_generated_action_name(&value))
            .filter(|value| !value.is_empty())
            .unwrap_or_else(|| name.to_string());
        let existing_config =
            crate::custom_apis::list_custom_apis(storage, &self.config_dir, self.data_dir())
                .await?
                .into_iter()
                .find(|item| item.config.id == target_id)
                .map(|item| item.config);
        let request_name = if Self::capability_string_argument(arguments, "name").is_some() {
            name.to_string()
        } else {
            existing_config
                .as_ref()
                .map(|config| config.name.clone())
                .unwrap_or_else(|| name.to_string())
        };
        let request_description =
            if Self::capability_string_argument(arguments, "description").is_some() {
                description.to_string()
            } else {
                existing_config
                    .as_ref()
                    .map(|config| config.description.clone())
                    .filter(|value| !value.trim().is_empty())
                    .unwrap_or_else(|| description.to_string())
            };
        let mut effective_arguments = arguments.clone();
        if let (Some(object), Some(existing)) = (
            effective_arguments.as_object_mut(),
            existing_config.as_ref(),
        ) {
            object
                .entry("base_url".to_string())
                .or_insert_with(|| serde_json::Value::String(existing.base_url.clone()));
        }
        let graphql_request_without_source = if Self::capability_acquire_needs_source_for_custom_api(
            &effective_arguments,
            existing_config.is_some(),
        ) {
            // A protocol-defined endpoint (e.g. GraphQL) needs no derived
            // operation set — the protocol itself specifies the executable
            // request shape — so the source-evidence gate does not apply and
            // the integration scaffolds in one shot.
            match Self::protocol_defined_request_fallback(
                &effective_arguments,
                existing_config.as_ref(),
                &target_id,
                &request_name,
                &request_description,
            ) {
                Some(fallback) => Some(fallback?),
                None => {
                    // Park the validated non-secret fields as a disabled
                    // draft so the retry only needs to add the missing source
                    // evidence (the create path backfills name/base_url/auth
                    // from existing records).
                    let draft_saved = if existing_config.is_none() {
                        self.save_custom_api_acquisition_draft(
                            &target_id,
                            &request_name,
                            &request_description,
                            &effective_arguments,
                        )
                        .await
                    } else {
                        false
                    };
                    return self
                        .capability_acquire_needs_source_output(
                            &target_id,
                            &effective_arguments,
                            existing_config.is_some() || draft_saved,
                            draft_saved,
                        )
                        .await;
                }
            }
        } else {
            None
        };
        let (mut request, operation_count) = if let Some(prebuilt) = graphql_request_without_source
        {
            prebuilt
        } else if let Some(preview_request) =
            Self::custom_api_preview_request_from_source_contract(
                &effective_arguments,
                request_name.clone(),
            ) {
            match crate::custom_apis::preview_custom_api(preview_request).await {
                Ok(preview) => {
                    let auth_mode = Self::capability_auth_mode(&effective_arguments)
                        .or_else(|| existing_config.as_ref().map(|config| config.auth_mode))
                        .unwrap_or(preview.auth_mode);
                    let auth_header =
                        Self::capability_string_argument(&effective_arguments, "auth_header_name")
                            .or(preview.auth_header);
                    let auth_name = if matches!(
                        auth_mode,
                        crate::custom_apis::CustomApiAuthMode::ApiKeyHeader
                            | crate::custom_apis::CustomApiAuthMode::ApiKeyQuery
                    ) {
                        auth_header.clone().or(preview.auth_name)
                    } else {
                        preview.auth_name
                    };
                    let operation_count = preview.operations.len();
                    (
                        crate::custom_apis::CustomApiUpsertRequest {
                            id: Some(target_id.clone()),
                            name: preview.suggested_name,
                            description: Some(request_description.clone()),
                            base_url: preview.base_url,
                            enabled: Some(true),
                            auth_mode: Some(auth_mode),
                            auth_profile_id: None,
                            auth_header,
                            auth_name,
                            auth_username: preview.auth_username,
                            secret: None,
                            clear_secret: None,
                            allow_missing_secret: Some(true),
                            operations: preview.operations,
                        },
                        operation_count,
                    )
                }
                Err(error) => {
                    // Source derivation failed (e.g. a docs page that is not
                    // machine-readable). Degrade deterministically, in order:
                    // (1) protocol-defined contract (the protocol itself
                    //     fully specifies the request shape — no derivation
                    //     needed);
                    // (2) the caller's DECLARED operations[] contract — the
                    //     gate already accepted source+operations as
                    //     executable, and a source that fails to parse must
                    //     not retroactively invalidate the declared contract;
                    // (3) structured needs_source teaching + parked draft.
                    // Never a bare error the spine cannot teach from.
                    if let Some(fallback) = Self::protocol_defined_request_fallback(
                        &effective_arguments,
                        existing_config.as_ref(),
                        &target_id,
                        &request_name,
                        &request_description,
                    ) {
                        fallback?
                    } else if Self::capability_arguments_have_operation_shape(
                        &effective_arguments,
                    ) {
                        let (base_url, operations) = Self::capability_operation_drafts(
                            &effective_arguments,
                            name,
                            &request_description,
                        )?;
                        let operation_count = operations.len();
                        let (auth_mode, auth_header, auth_name, auth_username) =
                            Self::capability_auth_fields(&effective_arguments);
                        (
                            crate::custom_apis::CustomApiUpsertRequest {
                                id: Some(target_id.clone()),
                                name: request_name.clone(),
                                description: Some(request_description.clone()),
                                base_url,
                                enabled: Some(true),
                                auth_mode: auth_mode.or_else(|| {
                                    existing_config.as_ref().map(|config| config.auth_mode)
                                }),
                                auth_profile_id: None,
                                auth_header,
                                auth_name,
                                auth_username,
                                secret: None,
                                clear_secret: None,
                                allow_missing_secret: Some(true),
                                operations,
                            },
                            operation_count,
                        )
                    } else {
                        // Non-GraphQL endpoint: park the validated fields as
                        // a draft and teach the full contract — never a bare
                        // rollback.
                        let draft_saved = if existing_config.is_none() {
                            self.save_custom_api_acquisition_draft(
                                &target_id,
                                &request_name,
                                &request_description,
                                &effective_arguments,
                            )
                            .await
                        } else {
                            false
                        };
                        return Ok(structured_tool_completion_output(
                            "capability_acquire",
                            "needs_source",
                            format!(
                                "The supplied source could not be turned into a machine-readable contract: {error}"
                            ),
                            serde_json::json!({
                                "reason": "source_derivation_failed",
                                "integration_type": "custom_api",
                                "id": target_id,
                                "draft_saved": draft_saved,
                                "source_error": error.to_string(),
                                "accepted_sources": crate::core::request_contract::SOURCE_ALIAS_KEYS,
                                "expected_contract": crate::core::request_contract::expected_custom_api_acquisition_contract(),
                                "assistant_instruction": "The source did not parse as OpenAPI and documentation inference produced no operations. Provide openapi_text, a provider manifest, or an explicit operations[] contract. For GraphQL providers, pass the GraphQL endpoint itself as base_url — the generic GraphQL operation is scaffolded automatically."
                            }),
                        ));
                    }
                }
            }
        } else {
            let (base_url, operations) =
                if Self::capability_arguments_have_operation_shape(&effective_arguments) {
                    Self::capability_operation_drafts(
                        &effective_arguments,
                        name,
                        &request_description,
                    )?
                } else if let Some(existing) = existing_config.as_ref() {
                    (
                        existing.base_url.clone(),
                        existing
                            .operations
                            .iter()
                            .map(|operation| operation.draft.clone())
                            .collect(),
                    )
                } else {
                    Self::capability_operation_drafts(
                        &effective_arguments,
                        name,
                        &request_description,
                    )?
                };
            let operation_count = operations.len();
            let (auth_mode, auth_header, auth_name, auth_username) =
                Self::capability_auth_fields(&effective_arguments);
            (
                crate::custom_apis::CustomApiUpsertRequest {
                    id: Some(target_id.clone()),
                    name: request_name.clone(),
                    description: Some(request_description.clone()),
                    base_url,
                    enabled: Some(true),
                    auth_mode,
                    auth_profile_id: None,
                    auth_header,
                    auth_name,
                    auth_username,
                    secret: None,
                    clear_secret: None,
                    allow_missing_secret: Some(true),
                    operations,
                },
                operation_count,
            )
        };

        if allow_duplicate {
            let existing =
                crate::custom_apis::list_custom_apis(storage, &self.config_dir, self.data_dir())
                    .await?;
            if existing.iter().any(|item| item.config.id == name) {
                request.id = Some(format!("{}-{}", name, uuid::Uuid::new_v4().simple()));
            }
        }
        let request_id = request.id.clone().unwrap_or_else(|| target_id.clone());
        let existing =
            crate::custom_apis::list_custom_apis(storage, &self.config_dir, self.data_dir())
                .await?
                .into_iter()
                .any(|item| item.config.id == request_id);
        let path_id = if existing && !allow_duplicate {
            Some(request_id.as_str())
        } else {
            None
        };
        let view = crate::custom_apis::upsert_custom_api(
            storage,
            &self.config_dir,
            self.data_dir(),
            self,
            request,
            path_id,
        )
        .await?;
        let settings_path = format!(
            "Settings > Integrations > Custom APIs > {}",
            view.config.name
        );
        let needs_credentials = view.config.auth_profile_id.is_none()
            && !matches!(
                view.config.auth_mode,
                crate::custom_apis::CustomApiAuthMode::None
            )
            && !view.secret_configured;
        let auth_mode = serde_json::to_value(view.config.auth_mode)
            .ok()
            .and_then(|value| value.as_str().map(str::to_string))
            .unwrap_or_else(|| "none".to_string());
        let credential_request = if needs_credentials {
            let fields = match view.config.auth_mode {
                crate::custom_apis::CustomApiAuthMode::Basic => serde_json::json!([
                    {
                        "key": "username",
                        "label": "Username",
                        "input_type": "text",
                        "required": true
                    },
                    {
                        "key": "password",
                        "label": "Password",
                        "input_type": "password",
                        "required": true
                    }
                ]),
                crate::custom_apis::CustomApiAuthMode::ApiKeyHeader
                | crate::custom_apis::CustomApiAuthMode::ApiKeyQuery => serde_json::json!([
                    {
                        "key": "secret",
                        "label": view
                            .config
                            .auth_name
                            .as_deref()
                            .or(view.config.auth_header.as_deref())
                            .unwrap_or("API key"),
                        "input_type": "password",
                        "required": true
                    }
                ]),
                crate::custom_apis::CustomApiAuthMode::OAuth2 => serde_json::json!([
                    {
                        "key": "secret",
                        "label": "OAuth access token",
                        "input_type": "password",
                        "required": true
                    }
                ]),
                crate::custom_apis::CustomApiAuthMode::Bearer => serde_json::json!([
                    {
                        "key": "secret",
                        "label": "Bearer token",
                        "input_type": "password",
                        "required": true
                    }
                ]),
                crate::custom_apis::CustomApiAuthMode::None => serde_json::json!([]),
            };
            Some(serde_json::json!({
                "kind": "custom_api_auth",
                "api_id": view.config.id.clone(),
                "api_name": view.config.name.clone(),
                "auth_mode": auth_mode.clone(),
                "auth_name": view
                    .config
                    .auth_name
                    .as_deref()
                    .or(view.config.auth_header.as_deref()),
                "settings_path": settings_path.clone(),
                "fields": fields,
                "secure_input_required": true
            }))
        } else {
            None
        };
        let message = if needs_credentials {
            format!(
                "Custom API integration saved. Credentials are still required through the secure credential form or {}.",
                settings_path
            )
        } else if !matches!(
            view.config.auth_mode,
            crate::custom_apis::CustomApiAuthMode::None
        ) {
            "Custom API integration saved with authentication configured.".to_string()
        } else {
            "Custom API integration saved.".to_string()
        };
        Ok(structured_tool_completion_output(
            "capability_acquire",
            if needs_credentials {
                "needs_credentials"
            } else {
                "configured"
            },
            message.clone(),
            serde_json::json!({
                "integration_type": "custom_api",
                "operation": if path_id.is_some() { "update" } else { "create" },
                "settings_path": settings_path,
                "credential_request": credential_request,
                "custom_api": {
                    "id": view.config.id,
                    "name": view.config.name,
                    "base_url": view.config.base_url,
                    "auth_mode": auth_mode,
                    "auth_configured": view.secret_configured,
                    "action_count": view.action_count,
                    "imported_operation_count": operation_count,
                },
                "message": message
            }),
        ))
    }
}
