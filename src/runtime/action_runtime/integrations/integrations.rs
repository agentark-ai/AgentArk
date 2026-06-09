use super::super::*;

impl ActionRuntime {
    pub(in crate::runtime) async fn execute_manage_actions(
        &self,
        arguments: &serde_json::Value,
    ) -> Result<String> {
        let operation = arguments
            .get("operation")
            .and_then(|v| v.as_str())
            .ok_or_else(|| anyhow::anyhow!("Missing 'operation' parameter"))?;

        match operation {
            "create" => {
                let name = arguments
                    .get("name")
                    .and_then(|v| v.as_str())
                    .ok_or_else(|| anyhow::anyhow!("Missing 'name' for create"))?;
                let content = arguments
                    .get("content")
                    .and_then(|v| v.as_str())
                    .ok_or_else(|| anyhow::anyhow!("Missing 'content' for create"))?;
                if !name
                    .chars()
                    .all(|c| c.is_alphanumeric() || c == '-' || c == '_')
                {
                    return Err(anyhow::anyhow!(
                        "Invalid action name. Use kebab-case (e.g., 'check-weather')"
                    ));
                }
                if self.actions.read().await.contains_key(name) {
                    return Err(anyhow::anyhow!(
                        "Action '{}' already exists. Use 'update' instead.",
                        name
                    ));
                }
                let verdict = self.create_action(name, content, false).await?;
                let mut msg = format!("Action '{}' created and is immediately available.", name);
                if let Some(ref v) = verdict {
                    if !v.warnings.is_empty() {
                        msg.push_str(&format!("\nSecurity warnings: {}", v.warnings.join(", ")));
                    }
                    if !v.allow_load {
                        msg = format!(
                            "Action '{}' was blocked by security verification: {}",
                            name,
                            v.warnings.join(", ")
                        );
                    }
                }
                Ok(msg)
            }
            "update" => {
                let name = arguments
                    .get("name")
                    .and_then(|v| v.as_str())
                    .ok_or_else(|| anyhow::anyhow!("Missing 'name' for update"))?;
                let content = arguments
                    .get("content")
                    .and_then(|v| v.as_str())
                    .ok_or_else(|| anyhow::anyhow!("Missing 'content' for update"))?;
                match self.update_action_content(name, content).await? {
                    true => Ok(format!("Action '{}' updated.", name)),
                    false => Err(anyhow::anyhow!(
                        "Cannot update '{}'. System actions are read-only.",
                        name
                    )),
                }
            }
            "delete" => {
                let name = arguments
                    .get("name")
                    .and_then(|v| v.as_str())
                    .ok_or_else(|| anyhow::anyhow!("Missing 'name' for delete"))?;
                match self.delete_action(name).await? {
                    true => Ok(format!("Action '{}' deleted.", name)),
                    false => Err(anyhow::anyhow!(
                        "Cannot delete '{}'. Only custom actions can be deleted.",
                        name
                    )),
                }
            }
            "list" => {
                let actions = self.list_actions().await?;
                let list: Vec<String> = actions
                    .iter()
                    .map(|a| {
                        let source = match a.source {
                            ActionSource::System => "system",
                            ActionSource::Bundled => "bundled",
                            ActionSource::Custom => "custom",
                        };
                        format!("- **{}** ({}): {}", a.name, source, a.description)
                    })
                    .collect();
                Ok(format!(
                    "Available actions ({}):\n{}",
                    actions.len(),
                    list.join("\n")
                ))
            }
            _ => Err(anyhow::anyhow!(
                "Unknown operation '{}'. Use create, update, delete, or list.",
                operation
            )),
        }
    }

    pub(in crate::runtime) fn companion_device_is_connected(
        state: &crate::core::CompanionDeviceState,
    ) -> bool {
        matches!(
            state,
            crate::core::CompanionDeviceState::Online
                | crate::core::CompanionDeviceState::Idle
                | crate::core::CompanionDeviceState::Busy
        )
    }

    pub(in crate::runtime) fn connected_surface_item(
        surface: &str,
        id: impl Into<String>,
        name: impl Into<String>,
        kind: impl Into<String>,
        status: impl Into<String>,
    ) -> serde_json::Value {
        serde_json::json!({
            "surface": surface,
            "id": id.into(),
            "name": name.into(),
            "kind": kind.into(),
            "status": status.into(),
        })
    }

    pub(in crate::runtime) fn integration_inventory_section_counts(
        value: &serde_json::Value,
    ) -> serde_json::Value {
        fn array_len(value: &serde_json::Value, key: &str) -> usize {
            value
                .get(key)
                .and_then(|value| value.as_array())
                .map(Vec::len)
                .unwrap_or(0)
        }

        serde_json::json!({
            "builtin_integrations": array_len(value, "integrations"),
            "gateway_channels": array_len(value, "channels"),
            "notification_channels": array_len(value, "channels"),
            "custom_apis": array_len(value, "custom_apis"),
            "webhook_sources": array_len(value, "sources"),
            "companion_devices": array_len(value, "devices"),
        })
    }

    pub(in crate::runtime) async fn companion_device_inventory(
        &self,
        only_connected: bool,
    ) -> (serde_json::Value, Vec<serde_json::Value>) {
        let Some(storage) = self.storage() else {
            return (
                serde_json::json!({
                    "available": false,
                    "error": "AgentArk storage is not available in this runtime"
                }),
                Vec::new(),
            );
        };
        let plane = crate::core::CompanionControlPlane::new(storage);
        let devices = match plane.list_devices().await {
            Ok(devices) => devices,
            Err(error) => {
                return (
                    serde_json::json!({
                        "available": false,
                        "error": error.to_string()
                    }),
                    Vec::new(),
                );
            }
        };
        let overview = match plane.overview().await {
            Ok(overview) => Some(overview),
            Err(error) => {
                return (
                    serde_json::json!({
                        "available": false,
                        "error": error.to_string()
                    }),
                    Vec::new(),
                );
            }
        };
        let total = devices.len();
        let connected_total = devices
            .iter()
            .filter(|device| Self::companion_device_is_connected(&device.state))
            .count();
        let mut connected_items = Vec::new();
        let visible_devices = devices
            .into_iter()
            .filter(|device| !only_connected || Self::companion_device_is_connected(&device.state))
            .map(|device| {
                let connected = Self::companion_device_is_connected(&device.state);
                if connected {
                    connected_items.push(Self::connected_surface_item(
                        "companion_devices",
                        device.id.clone(),
                        device.display_name.clone(),
                        device.platform.clone(),
                        format!("{:?}", device.state).to_ascii_lowercase(),
                    ));
                }
                serde_json::json!({
                    "id": device.id,
                    "display_name": device.display_name,
                    "preset_id": device.preset_id,
                    "platform": device.platform,
                    "model": device.model,
                    "state": device.state,
                    "connected": connected,
                    "transport": device.transport,
                    "available_capabilities": device.available_capabilities,
                    "granted_capabilities": device.granted_capabilities,
                    "token_capabilities": device.token_capabilities,
                    "paired_at": device.paired_at,
                    "last_seen_at": device.last_seen_at,
                    "owner": device.owner,
                    "command_count": device.command_count,
                    "attestation": {
                        "verified": device.attestation.verified,
                        "provider": device.attestation.provider,
                        "platform": device.attestation.platform,
                        "verified_at": device.attestation.verified_at,
                        "reason": device.attestation.reason,
                    },
                    "trusted_unattested": device.trusted_unattested,
                })
            })
            .collect::<Vec<_>>();
        (
            serde_json::json!({
                "available": true,
                "surface": "companion_devices",
                "overview": overview,
                "total": total,
                "connected_total": connected_total,
                "filtered_to_connected": only_connected,
                "devices": visible_devices,
            }),
            connected_items,
        )
    }

    pub(in crate::runtime) fn integration_status_label(
        status: &crate::integrations::IntegrationStatus,
    ) -> String {
        match status {
            crate::integrations::IntegrationStatus::NotConfigured => "not_configured".to_string(),
            crate::integrations::IntegrationStatus::NeedsAuth => "needs_auth".to_string(),
            crate::integrations::IntegrationStatus::Connected => "connected".to_string(),
            crate::integrations::IntegrationStatus::Error(_) => "error".to_string(),
        }
    }

    pub(in crate::runtime) fn display_name_from_integration_id(integration_id: &str) -> String {
        integration_id
            .split(['_', '-', '.'])
            .filter(|part| !part.trim().is_empty())
            .map(|part| {
                let mut chars = part.chars();
                match chars.next() {
                    Some(first) => {
                        let mut word = String::new();
                        word.extend(first.to_uppercase());
                        word.push_str(chars.as_str());
                        word
                    }
                    None => String::new(),
                }
            })
            .collect::<Vec<_>>()
            .join(" ")
    }

    pub(in crate::runtime) fn stored_google_token_has_refresh_token(&self, key: &str) -> bool {
        self.settings_manager()
            .ok()
            .and_then(|manager| manager.get_custom_secret(key).ok().flatten())
            .and_then(|raw| serde_json::from_str::<serde_json::Value>(&raw).ok())
            .and_then(|value| {
                value
                    .get("refresh_token")
                    .and_then(|token| token.as_str())
                    .map(|token| !token.trim().is_empty())
            })
            .unwrap_or(false)
    }

    pub(in crate::runtime) fn google_workspace_granted_bundle_for_runtime(
        &self,
        bundle: &str,
    ) -> bool {
        if !crate::integrations::effective_integration_enabled(&self.config_dir, "google_workspace")
        {
            return false;
        }
        crate::actions::google_workspace::granted_bundles(&self.config_dir)
            .map(|bundles| bundles.iter().any(|granted| granted == bundle))
            .unwrap_or(false)
    }

    pub(in crate::runtime) fn google_workspace_connected_for_runtime(&self) -> bool {
        if !crate::integrations::effective_integration_enabled(&self.config_dir, "google_workspace")
        {
            return false;
        }
        crate::actions::google_workspace::granted_bundles(&self.config_dir)
            .map(|bundles| !bundles.is_empty())
            .unwrap_or(false)
    }

    pub(in crate::runtime) fn action_backed_integration_connected(
        &self,
        integration_id: &str,
    ) -> bool {
        match integration_id {
            "gmail" => {
                (crate::integrations::effective_integration_enabled(&self.config_dir, "gmail")
                    && self.stored_google_token_has_refresh_token("gmail_tokens"))
                    || self.google_workspace_granted_bundle_for_runtime("gmail")
            }
            "google_calendar" => {
                (crate::integrations::effective_integration_enabled(
                    &self.config_dir,
                    "google_calendar",
                ) && self.stored_google_token_has_refresh_token("calendar_tokens"))
                    || self.google_workspace_granted_bundle_for_runtime("calendar")
            }
            "google_workspace" => self.google_workspace_connected_for_runtime(),
            _ => true,
        }
    }

    pub(in crate::runtime) async fn action_backed_builtin_integrations(
        &self,
    ) -> BTreeMap<String, serde_json::Value> {
        let enabled_actions = self.list_enabled_actions().await.unwrap_or_default();
        let mut by_integration: BTreeMap<
            String,
            (BTreeSet<String>, BTreeMap<String, String>, bool, bool),
        > = BTreeMap::new();

        for action in enabled_actions {
            let access = &action.authorization.access;
            let mut integration_ids = BTreeSet::new();
            integration_ids.extend(
                access
                    .integration_ids
                    .iter()
                    .map(|value| value.trim())
                    .filter(|value| !value.is_empty())
                    .map(ToOwned::to_owned),
            );
            integration_ids.extend(
                access
                    .integration_features
                    .keys()
                    .map(|value| value.trim())
                    .filter(|value| !value.is_empty())
                    .map(ToOwned::to_owned),
            );
            if integration_ids.is_empty() {
                continue;
            }

            for integration_id in integration_ids {
                let (capabilities, actions, read_capable, write_capable) = by_integration
                    .entry(integration_id)
                    .or_insert_with(|| (BTreeSet::new(), BTreeMap::new(), false, false));
                capabilities.extend(
                    action
                        .capabilities
                        .iter()
                        .map(|value| value.trim())
                        .filter(|value| !value.is_empty())
                        .map(ToOwned::to_owned),
                );
                actions.insert(action.name.clone(), action.description.clone());
                let metadata = action.action_metadata();
                *read_capable |= action.authorization.outbound.read_only
                    || matches!(
                        metadata.role,
                        crate::actions::ActionRole::Inspection
                            | crate::actions::ActionRole::DataSource
                    );
                *write_capable |= action.authorization.outbound.outbound_write
                    || matches!(
                        metadata.role,
                        crate::actions::ActionRole::Mutation | crate::actions::ActionRole::Delivery
                    )
                    || matches!(
                        metadata.side_effect_level,
                        crate::actions::ActionSideEffectLevel::Write
                            | crate::actions::ActionSideEffectLevel::Notify
                    );
            }
        }

        by_integration
            .into_iter()
            .map(
                |(integration_id, (capabilities, actions, read_capable, write_capable))| {
                let action_summaries = actions
                    .iter()
                    .take(16)
                    .map(|(name, description)| {
                        serde_json::json!({
                            "name": name,
                            "description": description,
                        })
                    })
                    .collect::<Vec<_>>();
                let action_names = actions.keys().cloned().collect::<Vec<_>>();
                let capability_values = capabilities.into_iter().collect::<Vec<_>>();
                let enabled = crate::integrations::effective_integration_enabled(
                    &self.config_dir,
                    &integration_id,
                );
                let connected = self.action_backed_integration_connected(&integration_id);
                let status = if connected {
                    "connected"
                } else if enabled {
                    "needs_auth"
                } else {
                    "disabled"
                };
                let display_name = Self::display_name_from_integration_id(&integration_id);
                (
                    integration_id.clone(),
                    serde_json::json!({
                        "id": integration_id.clone(),
                        "name": display_name,
                        "description": "Action-backed integration surface discovered from the runtime action catalog.",
                        "icon": "",
                        "capabilities": capability_values,
                        "status": status,
                        "status_label": status,
                        "enabled_for_agent": enabled,
                        "connected": connected,
                        "action_backed": true,
                        "available_actions": action_names,
                        "available_action_summaries": action_summaries,
                        "read_capable": read_capable,
                        "write_capable": write_capable,
                    }),
                )
                },
            )
            .collect()
    }

    pub(in crate::runtime) fn merge_action_backed_integration_row(
        row: &mut serde_json::Value,
        action_backed: Option<serde_json::Value>,
    ) {
        let Some(action_backed) = action_backed else {
            return;
        };
        let Some(row_object) = row.as_object_mut() else {
            return;
        };
        row_object.insert("action_backed".to_string(), serde_json::json!(true));
        for key in [
            "available_actions",
            "available_action_summaries",
            "capabilities",
            "read_capable",
            "write_capable",
        ] {
            if let Some(value) = action_backed.get(key) {
                row_object.insert(key.to_string(), value.clone());
            }
        }
        if action_backed
            .get("connected")
            .and_then(|value| value.as_bool())
            .unwrap_or(false)
        {
            row_object.insert("connected".to_string(), serde_json::json!(true));
            row_object.insert("status".to_string(), serde_json::json!("connected"));
            row_object.insert("status_label".to_string(), serde_json::json!("connected"));
        }
    }

    pub(in crate::runtime) fn connected_surface_item_from_integration_row(
        row: &serde_json::Value,
    ) -> Option<serde_json::Value> {
        if !row
            .get("connected")
            .and_then(|value| value.as_bool())
            .unwrap_or(false)
        {
            return None;
        }
        let id = row.get("id").and_then(|value| value.as_str())?;
        let name = row
            .get("name")
            .and_then(|value| value.as_str())
            .unwrap_or(id);
        let mut item = Self::connected_surface_item(
            "integrations",
            id.to_string(),
            name.to_string(),
            if row
                .get("action_backed")
                .and_then(|value| value.as_bool())
                .unwrap_or(false)
            {
                "action_backed_integration"
            } else {
                "builtin"
            },
            "connected",
        );
        if let Some(object) = item.as_object_mut() {
            if let Some(actions) = row.get("available_actions") {
                object.insert("available_actions".to_string(), actions.clone());
            }
            if let Some(capabilities) = row.get("capabilities") {
                object.insert("capabilities".to_string(), capabilities.clone());
            }
        }
        Some(item)
    }

    pub(in crate::runtime) async fn builtin_integrations_inventory(
        &self,
        only_connected: bool,
    ) -> (serde_json::Value, Vec<serde_json::Value>) {
        let manager = crate::integrations::IntegrationManager::new(&self.config_dir);
        let mut action_backed = self.action_backed_builtin_integrations().await;
        let mut rows = Vec::new();
        let mut connected_items = Vec::new();
        for info in manager.list().await {
            let enabled = manager.is_enabled(&info.id);
            let connected = enabled
                && matches!(
                    info.status,
                    crate::integrations::IntegrationStatus::Connected
                );
            let mut row = serde_json::json!({
                "id": info.id,
                "name": info.name,
                "description": info.description,
                "icon": info.icon,
                "capabilities": info.capabilities,
                "status": info.status,
                "status_label": Self::integration_status_label(&info.status),
                "enabled_for_agent": enabled,
                "connected": connected,
            });
            let row_id = row
                .get("id")
                .and_then(|value| value.as_str())
                .map(ToOwned::to_owned);
            if let Some(row_id) = row_id.as_deref() {
                Self::merge_action_backed_integration_row(&mut row, action_backed.remove(row_id));
            }
            if let Some(item) = Self::connected_surface_item_from_integration_row(&row) {
                connected_items.push(item);
            }
            let connected = row
                .get("connected")
                .and_then(|value| value.as_bool())
                .unwrap_or(false);
            if only_connected && !connected {
                continue;
            }
            rows.push(row);
        }
        for (_, row) in action_backed {
            if let Some(item) = Self::connected_surface_item_from_integration_row(&row) {
                connected_items.push(item);
            }
            if only_connected
                && !row
                    .get("connected")
                    .and_then(|value| value.as_bool())
                    .unwrap_or(false)
            {
                continue;
            }
            rows.push(row);
        }
        rows.sort_by(|left, right| {
            let left_key = left
                .get("id")
                .and_then(|value| value.as_str())
                .unwrap_or("");
            let right_key = right
                .get("id")
                .and_then(|value| value.as_str())
                .unwrap_or("");
            left_key.cmp(right_key)
        });
        let total = rows.len();
        (
            serde_json::json!({
                "available": true,
                "surface": "builtin_integrations",
                "filtered_to_connected": only_connected,
                "visible_total": total,
                "connected_total": connected_items.len(),
                "integrations": rows,
            }),
            connected_items,
        )
    }

    pub(in crate::runtime) fn email_notification_configured_from_config(
        &self,
        config: &crate::core::AgentConfig,
    ) -> bool {
        let mut backends = Vec::new();
        if crate::integrations::effective_integration_enabled(&self.config_dir, "gmail")
            && self
                .settings_manager()
                .ok()
                .and_then(|manager| manager.get_custom_secret("gmail_tokens").ok().flatten())
                .is_some_and(|value| !value.trim().is_empty())
        {
            backends
                .push(crate::core::connectivity::email_delivery::EMAIL_PROVIDER_GMAIL.to_string());
        }
        if crate::integrations::effective_integration_enabled(&self.config_dir, "google_workspace")
            && crate::actions::google_workspace::granted_bundles(&self.config_dir)
                .map(|bundles| bundles.iter().any(|bundle| bundle == "gmail"))
                .unwrap_or(false)
        {
            backends.push(
                crate::core::connectivity::email_delivery::EMAIL_PROVIDER_GOOGLE_WORKSPACE
                    .to_string(),
            );
        }
        if crate::core::connectivity::email_delivery::external_email_delivery_is_ready(
            &config.email,
        ) {
            if let Some(provider_id) =
                crate::core::connectivity::email_delivery::external_email_provider_id(&config.email)
            {
                if !backends.iter().any(|existing| existing == &provider_id) {
                    backends.push(provider_id);
                }
            }
        }
        crate::core::connectivity::email_delivery::email_channel_is_ready(
            &config.email.provider,
            &backends,
        )
    }

    pub(in crate::runtime) async fn gateway_channels_inventory(
        &self,
        only_connected: bool,
    ) -> (
        serde_json::Value,
        Vec<serde_json::Value>,
        BTreeMap<String, bool>,
    ) {
        let Some(storage) = self.storage() else {
            return (
                serde_json::json!({
                    "available": false,
                    "error": "AgentArk storage is not available in this runtime"
                }),
                Vec::new(),
                BTreeMap::new(),
            );
        };
        let config = match self.settings_manager().and_then(|manager| manager.load()) {
            Ok(config) => config,
            Err(error) => {
                return (
                    serde_json::json!({
                        "available": false,
                        "error": error.to_string()
                    }),
                    Vec::new(),
                    BTreeMap::new(),
                );
            }
        };
        let payload = match crate::core::load_gateway_channels(&storage, &config).await {
            Ok(payload) => payload,
            Err(error) => {
                return (
                    serde_json::json!({
                        "available": false,
                        "error": error.to_string()
                    }),
                    Vec::new(),
                    BTreeMap::new(),
                );
            }
        };
        let mut configured = BTreeMap::new();
        let mut connected_items = Vec::new();
        let mut channels = Vec::new();
        for channel in payload.channels {
            let connected = channel.enabled
                && (matches!(
                    channel.status.as_str(),
                    "connected" | "ready" | "configured"
                ) || channel.connected_account_count > 0);
            configured.insert(channel.id.clone(), channel.configured || connected);
            if connected {
                connected_items.push(Self::connected_surface_item(
                    "messaging_channels",
                    channel.id.clone(),
                    channel.name.clone(),
                    channel.kind.clone(),
                    channel.status.clone(),
                ));
            }
            if only_connected && !connected {
                continue;
            }
            let mut value = serde_json::to_value(channel).unwrap_or_default();
            if let Some(object) = value.as_object_mut() {
                object.insert("connected".to_string(), serde_json::json!(connected));
            }
            channels.push(value);
        }
        let accounts = if only_connected {
            payload
                .accounts
                .into_iter()
                .filter(|account| {
                    account.enabled
                        && matches!(
                            account.status.trim().to_ascii_lowercase().as_str(),
                            "connected" | "ready" | "syncing"
                        )
                })
                .collect::<Vec<_>>()
        } else {
            payload.accounts
        };
        (
            serde_json::json!({
                "available": true,
                "surface": "gateway_channels",
                "summary": payload.summary,
                "filtered_to_connected": only_connected,
                "channels": channels,
                "accounts": accounts,
            }),
            connected_items,
            configured,
        )
    }

    pub(in crate::runtime) async fn messaging_channels_inventory(
        &self,
        only_connected: bool,
        bundled_configured: &BTreeMap<String, bool>,
    ) -> (serde_json::Value, Vec<serde_json::Value>) {
        let Some(storage) = self.storage() else {
            return (
                serde_json::json!({
                    "available": false,
                    "error": "AgentArk storage is not available in this runtime"
                }),
                Vec::new(),
            );
        };
        let Some(registry) = self.extension_pack_registry.clone() else {
            return (
                serde_json::json!({
                    "available": false,
                    "error": "Extension-pack registry is not available in this runtime"
                }),
                Vec::new(),
            );
        };
        let config = self
            .settings_manager()
            .and_then(|manager| manager.load())
            .ok();
        let email_configured = config
            .as_ref()
            .is_some_and(|config| self.email_notification_configured_from_config(config));
        let config_manager = self.settings_manager().ok();
        let packs_guard = registry.read().await;
        let bundled_check = |channel_id: &str| -> bool {
            let normalized = channel_id.trim().to_ascii_lowercase();
            if normalized == "email" {
                return email_configured;
            }
            bundled_configured
                .get(&normalized)
                .copied()
                .unwrap_or(false)
        };
        let ctx = crate::channels::messaging_registry::ChannelQueryContext {
            bundled_configured: &bundled_check,
            extension_packs: &packs_guard,
            storage: &storage,
            config_dir: &self.config_dir,
            data_dir: self.data_dir(),
            config_manager: config_manager.as_ref(),
        };
        let descriptors = match crate::channels::messaging_registry::MessagingChannelRegistry::new()
            .list(&ctx)
            .await
        {
            Ok(descriptors) => descriptors,
            Err(error) => {
                return (
                    serde_json::json!({
                        "available": false,
                        "error": error.to_string()
                    }),
                    Vec::new(),
                );
            }
        };
        let mut connected_items = Vec::new();
        let mut channels = Vec::new();
        for descriptor in descriptors {
            if descriptor.configured {
                connected_items.push(Self::connected_surface_item(
                    "notification_channels",
                    descriptor.id.clone(),
                    descriptor.display_name.clone(),
                    match &descriptor.source {
                        crate::channels::messaging_registry::ChannelSource::Bundled => "bundled",
                        crate::channels::messaging_registry::ChannelSource::ExtensionPack { .. } => {
                            "extension_pack"
                        }
                        crate::channels::messaging_registry::ChannelSource::CustomMessagingChannel {
                            ..
                        } => "custom_messaging_channel",
                    },
                    "configured",
                ));
            }
            if only_connected && !descriptor.configured {
                continue;
            }
            let mut value = serde_json::to_value(descriptor).unwrap_or_default();
            if let Some(object) = value.as_object_mut() {
                object.insert(
                    "connected".to_string(),
                    object
                        .get("configured")
                        .cloned()
                        .unwrap_or(serde_json::Value::Bool(false)),
                );
            }
            channels.push(value);
        }
        (
            serde_json::json!({
                "available": true,
                "surface": "notification_channels",
                "filtered_to_connected": only_connected,
                "connected_total": connected_items.len(),
                "channels": channels,
            }),
            connected_items,
        )
    }

    pub(in crate::runtime) async fn custom_apis_inventory(
        &self,
        only_connected: bool,
    ) -> (serde_json::Value, Vec<serde_json::Value>) {
        let Some(storage) = self.storage() else {
            return (
                serde_json::json!({
                    "available": false,
                    "error": "AgentArk storage is not available in this runtime"
                }),
                Vec::new(),
            );
        };
        let apis =
            match crate::custom_apis::list_custom_apis(&storage, &self.config_dir, self.data_dir())
                .await
            {
                Ok(apis) => apis,
                Err(error) => {
                    return (
                        serde_json::json!({
                            "available": false,
                            "error": error.to_string()
                        }),
                        Vec::new(),
                    );
                }
            };
        let total = apis.len();
        let mut rows = Vec::new();
        let mut connected_items = Vec::new();
        let observed_at = chrono::Utc::now().to_rfc3339();
        for api in apis {
            let connected = Self::custom_api_view_is_connected(&api);
            if connected {
                connected_items.push(Self::connected_surface_item(
                    "custom_apis",
                    api.config.id.clone(),
                    api.config.name.clone(),
                    "custom_api",
                    "connected",
                ));
            }
            if only_connected && !connected {
                continue;
            }
            let capability_contract =
                crate::custom_apis::capability_contract(&api.config, api.secret_configured);
            let state_contract = Self::custom_api_view_state_contract(&api);
            let mut value = serde_json::to_value(api).unwrap_or_default();
            if let Some(object) = value.as_object_mut() {
                object.insert("connected".to_string(), serde_json::json!(connected));
                object.insert("capability_contract".to_string(), capability_contract);
                object.insert("state".to_string(), state_contract.clone());
                object.insert(
                    "registered".to_string(),
                    state_contract
                        .get("registered")
                        .cloned()
                        .unwrap_or(serde_json::Value::Bool(true)),
                );
                object.insert(
                    "enabled".to_string(),
                    state_contract
                        .get("enabled")
                        .cloned()
                        .unwrap_or(serde_json::Value::Bool(false)),
                );
                object.insert(
                    "auth_ready".to_string(),
                    state_contract
                        .get("auth_ready")
                        .cloned()
                        .unwrap_or(serde_json::Value::Bool(false)),
                );
                object.insert(
                    "verified".to_string(),
                    state_contract
                        .get("verified")
                        .cloned()
                        .unwrap_or(serde_json::Value::Bool(false)),
                );
                object.insert(
                    "observed_at".to_string(),
                    serde_json::Value::String(observed_at.clone()),
                );
            }
            rows.push(value);
        }
        (
            serde_json::json!({
                "available": true,
                "surface": "custom_apis",
                "total": total,
                "connected_total": connected_items.len(),
                "filtered_to_connected": only_connected,
                "custom_apis": rows,
            }),
            connected_items,
        )
    }

    pub(in crate::runtime) fn custom_api_view_state_contract(
        api: &crate::custom_apis::CustomApiView,
    ) -> serde_json::Value {
        let auth_ready = matches!(
            api.config.auth_mode,
            crate::custom_apis::CustomApiAuthMode::None
        ) || api.secret_configured;
        let verified = api
            .config
            .last_test_outcome
            .as_deref()
            .map(str::trim)
            .is_some_and(|status| status.eq_ignore_ascii_case("success"));
        let connected = Self::custom_api_view_is_connected(api);
        serde_json::json!({
            "surface": "custom_apis",
            "kind": "custom_api",
            "id": api.config.id.clone(),
            "name": api.config.name.clone(),
            "registered": true,
            "enabled": api.config.enabled,
            "action_count": api.action_count,
            "secret_configured": api.secret_configured,
            "auth_ready": auth_ready,
            "verified": verified,
            "connected": connected,
            "last_tested_at": api.config.last_tested_at.clone(),
            "last_test_outcome": api.config.last_test_outcome.clone(),
            "last_test_message": api.config.last_test_message.clone(),
        })
    }

    pub(in crate::runtime) fn custom_api_view_is_connected(
        api: &crate::custom_apis::CustomApiView,
    ) -> bool {
        if !Self::custom_api_view_is_callable(api) {
            return false;
        }
        !api.config
            .last_test_outcome
            .as_deref()
            .map(str::trim)
            .is_some_and(|status| status.eq_ignore_ascii_case("failure"))
    }

    pub(in crate::runtime) fn custom_api_view_is_callable(
        api: &crate::custom_apis::CustomApiView,
    ) -> bool {
        let auth_ready = matches!(
            api.config.auth_mode,
            crate::custom_apis::CustomApiAuthMode::None
        ) || api.secret_configured;
        api.config.enabled && api.action_count > 0 && auth_ready
    }

    pub(in crate::runtime) fn custom_api_view_matches_selector(
        api: &crate::custom_apis::CustomApiView,
        selector: &str,
    ) -> bool {
        let selector = selector.trim();
        if selector.is_empty() {
            return false;
        }
        if api.config.id.eq_ignore_ascii_case(selector)
            || api.config.name.eq_ignore_ascii_case(selector)
        {
            return true;
        }
        let canonical_selector = Self::normalize_generated_action_name(selector);
        !canonical_selector.is_empty()
            && [
                api.config.id.as_str(),
                api.config.name.as_str(),
                api.config.description.as_str(),
            ]
            .iter()
            .any(|candidate| Self::normalize_generated_action_name(candidate) == canonical_selector)
    }

    pub(in crate::runtime) fn custom_api_operation_allows_read_request(
        operation: &crate::custom_apis::CustomApiOperation,
        body: Option<&serde_json::Value>,
    ) -> bool {
        if operation.draft.read_only {
            if let Some(body) = body {
                if crate::custom_apis::custom_api_operation_supports_graphql_body(
                    &operation.draft.method,
                    &operation.draft.path,
                    &operation.draft.default_headers,
                    operation.draft.body_required || operation.draft.default_body.is_some(),
                ) {
                    return crate::custom_apis::custom_api_body_is_read_only_graphql_query(body);
                }
            }
            return true;
        }
        let Some(body) = body else {
            return false;
        };
        crate::custom_apis::custom_api_operation_supports_graphql_body(
            &operation.draft.method,
            &operation.draft.path,
            &operation.draft.default_headers,
            true,
        ) && crate::custom_apis::custom_api_body_is_read_only_graphql_query(body)
    }

    pub(in crate::runtime) fn normalize_custom_api_request_action_arguments(
        operation: &crate::custom_apis::CustomApiOperation,
        mut arguments: serde_json::Value,
    ) -> serde_json::Value {
        let Some(object) = arguments.as_object_mut() else {
            return arguments;
        };
        let Some(body) = object.get("body").cloned() else {
            return arguments;
        };
        object.insert(
            "body".to_string(),
            Self::normalize_custom_api_request_body_value(operation, body),
        );
        arguments
    }

    pub(in crate::runtime) fn normalize_custom_api_request_body_value(
        _operation: &crate::custom_apis::CustomApiOperation,
        body: serde_json::Value,
    ) -> serde_json::Value {
        crate::core::request_contract::coerce_json_encoded_body_value(body)
    }

    pub(in crate::runtime) fn custom_api_operation_matches_selector(
        operation: &crate::custom_apis::CustomApiOperation,
        selector: &str,
    ) -> bool {
        let selector = selector.trim();
        if selector.is_empty() {
            return false;
        }
        let endpoint_label = format!("{} {}", operation.draft.method, operation.draft.path);
        let candidates = [
            operation.action_name.as_str(),
            operation.draft.id.as_str(),
            operation.draft.name.as_str(),
            endpoint_label.as_str(),
        ];
        if candidates
            .iter()
            .any(|candidate| candidate.eq_ignore_ascii_case(selector))
        {
            return true;
        }
        let canonical_selector = Self::normalize_generated_action_name(selector);
        !canonical_selector.is_empty()
            && candidates.iter().any(|candidate| {
                Self::normalize_generated_action_name(candidate) == canonical_selector
            })
    }

    pub(in crate::runtime) fn select_custom_api_read_operation<'a>(
        operations: impl IntoIterator<Item = &'a crate::custom_apis::CustomApiOperation>,
        selector: Option<&str>,
        explicit_request_body: Option<&serde_json::Value>,
        has_explicit_body: bool,
    ) -> Option<(&'a crate::custom_apis::CustomApiOperation, &'static str)> {
        let enabled = operations
            .into_iter()
            .filter(|operation| operation.draft.enabled)
            .collect::<Vec<_>>();

        if let Some(selector) = selector {
            if let Some(operation) = enabled
                .iter()
                .copied()
                .find(|operation| Self::custom_api_operation_matches_selector(operation, selector))
            {
                return Some((operation, "selector_match"));
            }

            let mut compatible = enabled.iter().copied().filter(|operation| {
                let request_body = explicit_request_body.or(operation.draft.default_body.as_ref());
                Self::custom_api_operation_allows_read_request(operation, request_body)
            });
            let first = compatible.next()?;
            if compatible.next().is_none() {
                return Some((first, "single_compatible_operation"));
            }
            return None;
        }

        let mut compatible = enabled.iter().copied().filter(|operation| {
            let request_body = explicit_request_body.or(operation.draft.default_body.as_ref());
            Self::custom_api_operation_allows_read_request(operation, request_body)
        });
        let first = compatible.next()?;
        if compatible.next().is_none() {
            return Some((first, "single_compatible_operation"));
        }

        if !has_explicit_body {
            return None;
        }

        enabled
            .iter()
            .copied()
            .find(|operation| {
                let request_body = explicit_request_body.or(operation.draft.default_body.as_ref());
                Self::custom_api_operation_allows_read_request(operation, request_body)
                    && (operation.draft.body_required
                        || operation.draft.parameters.iter().any(|parameter| {
                            matches!(
                                parameter.location,
                                crate::custom_apis::CustomApiParameterLocation::Body
                            )
                        })
                        || !operation.draft.read_only)
            })
            .map(|operation| (operation, "body_compatible_operation"))
    }

    pub(in crate::runtime) async fn webhook_sources_inventory(
        &self,
        only_connected: bool,
    ) -> (serde_json::Value, Vec<serde_json::Value>) {
        let Some(storage) = self.storage() else {
            return (
                serde_json::json!({
                    "available": false,
                    "error": "AgentArk storage is not available in this runtime"
                }),
                Vec::new(),
            );
        };
        let payload = match crate::channels::http::webhooks::list_webhook_source_inventory(
            &storage,
            &self.config_dir,
            self.data_dir(),
            only_connected,
        )
        .await
        {
            Ok(payload) => payload,
            Err(error) => {
                return (
                    serde_json::json!({
                        "available": false,
                        "error": error.to_string()
                    }),
                    Vec::new(),
                );
            }
        };
        let connected_items = payload
            .get("sources")
            .and_then(|value| value.as_array())
            .into_iter()
            .flatten()
            .filter(|source| {
                source
                    .get("connected")
                    .and_then(|value| value.as_bool())
                    .unwrap_or(false)
            })
            .map(|source| {
                Self::connected_surface_item(
                    "webhook_sources",
                    source
                        .get("id")
                        .and_then(|value| value.as_str())
                        .unwrap_or_default(),
                    source
                        .get("name")
                        .and_then(|value| value.as_str())
                        .unwrap_or_default(),
                    source
                        .get("provider")
                        .and_then(|value| value.as_str())
                        .unwrap_or("webhook"),
                    "connected",
                )
            })
            .collect::<Vec<_>>();
        (payload, connected_items)
    }

    pub(in crate::runtime) fn plugin_connected(
        plugin: &crate::plugins::registry::PluginView,
    ) -> bool {
        plugin.plugin.enabled
            && plugin.plugin.last_error.is_none()
            && (matches!(
                plugin.plugin.auth_mode,
                crate::plugins::registry::PluginAuthMode::None
            ) || plugin.token_configured)
    }

    pub(in crate::runtime) async fn plugins_inventory(
        &self,
        only_connected: bool,
    ) -> (
        Option<Vec<crate::plugins::registry::PluginView>>,
        Vec<serde_json::Value>,
    ) {
        let Some(registry) = self.plugin_registry.clone() else {
            return (None, Vec::new());
        };
        let guard = registry.read().await;
        let plugins = match guard.list_plugins().await {
            Ok(plugins) => plugins,
            Err(_) => return (None, Vec::new()),
        };
        let mut visible = Vec::new();
        let mut connected_items = Vec::new();
        for plugin in plugins {
            let connected = Self::plugin_connected(&plugin);
            if connected {
                connected_items.push(Self::connected_surface_item(
                    "plugins",
                    plugin.plugin.id.clone(),
                    plugin.plugin.name.clone(),
                    "plugin",
                    "connected",
                ));
            }
            if !only_connected || connected {
                visible.push(plugin);
            }
        }
        (Some(visible), connected_items)
    }

    pub(in crate::runtime) fn mcp_server_connected(
        server: &crate::mcp::registry::McpServerView,
    ) -> bool {
        server.enabled
            && server.last_error.is_none()
            && (server.tool_count > 0 || (server.resources_enabled && server.resource_count > 0))
    }

    pub(in crate::runtime) async fn mcp_servers_inventory(
        &self,
        only_connected: bool,
    ) -> (
        Option<Vec<crate::mcp::registry::McpServerView>>,
        Vec<serde_json::Value>,
    ) {
        let Some(registry) = self.mcp_registry.clone() else {
            return (None, Vec::new());
        };
        let guard = registry.read().await;
        let servers = match guard.list_servers(false).await {
            Ok(servers) => servers,
            Err(_) => return (None, Vec::new()),
        };
        let mut visible = Vec::new();
        let mut connected_items = Vec::new();
        for server in servers {
            let connected = Self::mcp_server_connected(&server);
            if connected {
                connected_items.push(Self::connected_surface_item(
                    "mcp_servers",
                    server.id.clone(),
                    server.name.clone(),
                    "mcp_server",
                    "connected",
                ));
            }
            if !only_connected || connected {
                visible.push(server);
            }
        }
        (Some(visible), connected_items)
    }

    pub(in crate::runtime) fn string_array_argument(
        arguments: &serde_json::Value,
        key: &str,
    ) -> Vec<String> {
        arguments
            .get(key)
            .and_then(|value| value.as_array())
            .map(|items| {
                items
                    .iter()
                    .filter_map(|item| item.as_str())
                    .map(str::trim)
                    .filter(|value| !value.is_empty())
                    .map(str::to_string)
                    .collect::<Vec<_>>()
            })
            .unwrap_or_default()
    }

    pub(in crate::runtime) fn string_map_argument(
        value: Option<&serde_json::Value>,
    ) -> Option<std::collections::HashMap<String, String>> {
        value.and_then(|value| value.as_object()).map(|object| {
            object
                .iter()
                .filter_map(|(key, value)| {
                    Self::value_to_http_string(value)
                        .map(|value| (key.trim().to_string(), value.trim().to_string()))
                })
                .filter(|(key, value)| !key.is_empty() && !value.is_empty())
                .collect::<std::collections::HashMap<_, _>>()
        })
    }

    pub(in crate::runtime) fn mcp_transport_from_arguments(
        arguments: &serde_json::Value,
        existing: Option<&crate::core::runtime::config::McpServerConfig>,
    ) -> Result<(
        crate::core::runtime::config::McpTransportConfig,
        Option<std::collections::HashMap<String, String>>,
    )> {
        let transport = arguments
            .get("transport")
            .and_then(|value| value.as_object());
        let transport_type = transport
            .and_then(|object| object.get("type"))
            .and_then(|value| value.as_str())
            .map(str::trim)
            .filter(|value| !value.is_empty())
            .map(str::to_ascii_lowercase);
        let url = transport
            .and_then(|object| object.get("url"))
            .and_then(|value| value.as_str())
            .map(str::trim)
            .filter(|value| !value.is_empty())
            .map(str::to_string)
            .or_else(|| Self::capability_string_argument(arguments, "url"));
        let command = transport
            .and_then(|object| object.get("command"))
            .and_then(|value| value.as_str())
            .map(str::trim)
            .filter(|value| !value.is_empty())
            .map(str::to_string)
            .or_else(|| Self::capability_string_argument(arguments, "command"));

        if transport_type.as_deref() == Some("http") || url.is_some() {
            let url = url
                .or_else(|| {
                    existing.and_then(|server| match &server.transport {
                        crate::core::runtime::config::McpTransportConfig::Http { url } => {
                            Some(url.clone())
                        }
                        _ => None,
                    })
                })
                .ok_or_else(|| anyhow::anyhow!("HTTP MCP server configuration requires url"))?;
            let parsed =
                reqwest::Url::parse(&url).map_err(|_| anyhow::anyhow!("Invalid MCP URL"))?;
            if parsed.scheme() != "http" && parsed.scheme() != "https" {
                anyhow::bail!("MCP URL must use http or https");
            }
            return Ok((
                crate::core::runtime::config::McpTransportConfig::Http { url },
                Some(std::collections::HashMap::new()),
            ));
        }

        if transport_type.as_deref() == Some("stdio") || command.is_some() {
            let command = command
                .or_else(|| {
                    existing.and_then(|server| match &server.transport {
                        crate::core::runtime::config::McpTransportConfig::Stdio {
                            command, ..
                        } => Some(command.clone()),
                        _ => None,
                    })
                })
                .ok_or_else(|| {
                    anyhow::anyhow!("stdio MCP server configuration requires command")
                })?;
            let args = transport
                .and_then(|object| object.get("args"))
                .or_else(|| arguments.get("args"))
                .and_then(|value| value.as_array())
                .map(|items| {
                    items
                        .iter()
                        .filter_map(|item| item.as_str())
                        .map(str::to_string)
                        .collect::<Vec<_>>()
                })
                .or_else(|| {
                    existing.and_then(|server| match &server.transport {
                        crate::core::runtime::config::McpTransportConfig::Stdio {
                            args, ..
                        } => Some(args.clone()),
                        _ => None,
                    })
                })
                .unwrap_or_default();
            let working_dir = transport
                .and_then(|object| object.get("working_dir"))
                .and_then(|value| value.as_str())
                .map(str::trim)
                .filter(|value| !value.is_empty())
                .map(str::to_string)
                .or_else(|| Self::capability_string_argument(arguments, "working_dir"))
                .or_else(|| {
                    existing.and_then(|server| match &server.transport {
                        crate::core::runtime::config::McpTransportConfig::Stdio {
                            working_dir,
                            ..
                        } => working_dir.clone(),
                        _ => None,
                    })
                });
            let env = Self::string_map_argument(
                transport
                    .and_then(|object| object.get("env"))
                    .or_else(|| arguments.get("env")),
            );
            let env_keys = env
                .as_ref()
                .map(|env| {
                    let mut keys = env.keys().cloned().collect::<Vec<_>>();
                    keys.sort();
                    keys.dedup();
                    keys
                })
                .or_else(|| {
                    existing.and_then(|server| match &server.transport {
                        crate::core::runtime::config::McpTransportConfig::Stdio {
                            env_keys,
                            ..
                        } => Some(env_keys.clone()),
                        _ => None,
                    })
                })
                .unwrap_or_default();
            return Ok((
                crate::core::runtime::config::McpTransportConfig::Stdio {
                    command,
                    args,
                    working_dir,
                    env_keys,
                },
                env,
            ));
        }

        if let Some(existing) = existing {
            return Ok((existing.transport.clone(), None));
        }
        Err(anyhow::anyhow!(
            "MCP server configuration requires an HTTP url or stdio command"
        ))
    }

    pub(in crate::runtime) fn mcp_auth_from_arguments(
        arguments: &serde_json::Value,
        existing: Option<&crate::core::runtime::config::McpServerConfig>,
        auth_profile_id: Option<&str>,
    ) -> Result<(
        Option<crate::core::runtime::config::McpAuthConfig>,
        Option<crate::core::runtime::config::McpAuthSecret>,
        bool,
    )> {
        if auth_profile_id
            .map(str::trim)
            .is_some_and(|value| !value.is_empty())
        {
            return Ok((None, None, false));
        }

        let auth = arguments.get("auth").and_then(|value| value.as_object());
        let auth_type = auth
            .and_then(|object| object.get("type"))
            .and_then(|value| value.as_str())
            .or_else(|| arguments.get("auth_type").and_then(|value| value.as_str()))
            .map(str::trim)
            .filter(|value| !value.is_empty())
            .map(|value| match value.to_ascii_lowercase().as_str() {
                "api_key_header" => "header".to_string(),
                "api_key_query" => "query".to_string(),
                other => other.to_string(),
            });
        let Some(auth_type) = auth_type else {
            return Ok((existing.and_then(|server| server.auth.clone()), None, false));
        };
        if auth_type == "none" {
            return Ok((None, None, true));
        }

        let secret_text = |primary: &str, secondary: &str| -> Option<String> {
            auth.and_then(|object| object.get(primary))
                .or_else(|| auth.and_then(|object| object.get(secondary)))
                .and_then(|value| value.as_str())
                .map(str::trim)
                .filter(|value| !value.is_empty() && *value != "[ENCRYPTED]")
                .map(str::to_string)
                .or_else(|| Self::capability_string_argument(arguments, primary))
                .or_else(|| Self::capability_string_argument(arguments, secondary))
        };
        let (config, secret_update) = match auth_type.as_str() {
            "bearer" => {
                let token = secret_text("token", "auth_value");
                let header = auth
                    .and_then(|object| object.get("header"))
                    .and_then(|value| value.as_str())
                    .map(str::trim)
                    .filter(|value| !value.is_empty())
                    .map(str::to_string)
                    .or_else(|| Self::capability_string_argument(arguments, "auth_header_name"))
                    .unwrap_or_else(|| "Authorization".to_string());
                (
                    crate::core::runtime::config::McpAuthConfig::Bearer { header },
                    token.map(|token| crate::core::runtime::config::McpAuthSecret {
                        token: Some(token),
                        username: None,
                        password: None,
                    }),
                )
            }
            "basic" => {
                let username = secret_text("username", "auth_username");
                let password = secret_text("password", "auth_password");
                let secret_update = if username.is_some() || password.is_some() {
                    Some(crate::core::runtime::config::McpAuthSecret {
                        token: None,
                        username,
                        password,
                    })
                } else {
                    None
                };
                (
                    crate::core::runtime::config::McpAuthConfig::Basic,
                    secret_update,
                )
            }
            "header" => {
                let token = secret_text("value", "auth_value");
                let name = auth
                    .and_then(|object| object.get("name"))
                    .and_then(|value| value.as_str())
                    .map(str::trim)
                    .filter(|value| !value.is_empty())
                    .map(str::to_string)
                    .or_else(|| Self::capability_string_argument(arguments, "auth_name"))
                    .or_else(|| Self::capability_string_argument(arguments, "auth_header_name"))
                    .ok_or_else(|| anyhow::anyhow!("Header auth requires auth name"))?;
                (
                    crate::core::runtime::config::McpAuthConfig::Header { name },
                    token.map(|token| crate::core::runtime::config::McpAuthSecret {
                        token: Some(token),
                        username: None,
                        password: None,
                    }),
                )
            }
            "query" => {
                let token = secret_text("value", "auth_value");
                let name = auth
                    .and_then(|object| object.get("name"))
                    .and_then(|value| value.as_str())
                    .map(str::trim)
                    .filter(|value| !value.is_empty())
                    .map(str::to_string)
                    .or_else(|| Self::capability_string_argument(arguments, "auth_name"))
                    .ok_or_else(|| anyhow::anyhow!("Query auth requires auth name"))?;
                (
                    crate::core::runtime::config::McpAuthConfig::Query { name },
                    token.map(|token| crate::core::runtime::config::McpAuthSecret {
                        token: Some(token),
                        username: None,
                        password: None,
                    }),
                )
            }
            _ => anyhow::bail!("Unsupported MCP auth type `{}`", auth_type),
        };
        Ok((Some(config), secret_update, false))
    }

    pub(in crate::runtime) fn mcp_credential_request(
        server_id: &str,
        server_name: &str,
        auth: Option<&crate::core::runtime::config::McpAuthConfig>,
        settings_path: &str,
    ) -> Option<serde_json::Value> {
        let (auth_type, auth_name, fields) = match auth? {
            crate::core::runtime::config::McpAuthConfig::Bearer { header } => (
                "bearer",
                Some(header.clone()),
                serde_json::json!([
                    {
                        "key": "token",
                        "label": "Bearer token",
                        "input_type": "password",
                        "required": true
                    }
                ]),
            ),
            crate::core::runtime::config::McpAuthConfig::Basic => (
                "basic",
                None,
                serde_json::json!([
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
            ),
            crate::core::runtime::config::McpAuthConfig::Header { name } => (
                "header",
                Some(name.clone()),
                serde_json::json!([
                    {
                        "key": "token",
                        "label": name,
                        "input_type": "password",
                        "required": true
                    }
                ]),
            ),
            crate::core::runtime::config::McpAuthConfig::Query { name } => (
                "query",
                Some(name.clone()),
                serde_json::json!([
                    {
                        "key": "token",
                        "label": name,
                        "input_type": "password",
                        "required": true
                    }
                ]),
            ),
        };
        Some(serde_json::json!({
            "kind": "mcp_server_auth",
            "server_id": server_id,
            "server_name": server_name,
            "auth_type": auth_type,
            "auth_name": auth_name,
            "settings_path": settings_path,
            "fields": fields,
            "secure_input_required": true
        }))
    }

    pub(in crate::runtime) fn mcp_server_id_from_arguments(
        arguments: &serde_json::Value,
        transport: &crate::core::runtime::config::McpTransportConfig,
    ) -> String {
        let seed = Self::capability_string_argument(arguments, "id")
            .or_else(|| Self::capability_string_argument(arguments, "name"))
            .or_else(|| match transport {
                crate::core::runtime::config::McpTransportConfig::Http { url } => {
                    reqwest::Url::parse(url)
                        .ok()
                        .and_then(|parsed| parsed.host_str().map(str::to_string))
                }
                crate::core::runtime::config::McpTransportConfig::Stdio { command, .. } => {
                    Some(command.clone())
                }
            })
            .unwrap_or_else(|| "mcp-server".to_string());
        let id = Self::normalize_generated_action_name(&seed);
        if id.is_empty() {
            "mcp-server".to_string()
        } else {
            id
        }
    }

    pub(in crate::runtime) fn mcp_transport_same_endpoint(
        left: &crate::core::runtime::config::McpTransportConfig,
        right: &crate::core::runtime::config::McpTransportConfig,
    ) -> bool {
        match (left, right) {
            (
                crate::core::runtime::config::McpTransportConfig::Http { url: left },
                crate::core::runtime::config::McpTransportConfig::Http { url: right },
            ) => left.trim_end_matches('/') == right.trim_end_matches('/'),
            (
                crate::core::runtime::config::McpTransportConfig::Stdio {
                    command: left_cmd,
                    args: left_args,
                    working_dir: left_dir,
                    ..
                },
                crate::core::runtime::config::McpTransportConfig::Stdio {
                    command: right_cmd,
                    args: right_args,
                    working_dir: right_dir,
                    ..
                },
            ) => left_cmd == right_cmd && left_args == right_args && left_dir == right_dir,
            _ => false,
        }
    }

    pub(in crate::runtime) async fn sync_mcp_registry_from_saved_config(
        &self,
        config: &crate::core::runtime::config::AgentConfig,
        secrets: &crate::core::runtime::config::Secrets,
    ) -> Result<serde_json::Value> {
        let Some(registry) = self.mcp_registry.clone() else {
            return Ok(serde_json::json!({
                "sync_status": "not_available",
                "message": "MCP configuration was saved; registry sync will occur after restart."
            }));
        };
        let Some(safety) = self.safety_engine.as_deref() else {
            return Ok(serde_json::json!({
                "sync_status": "not_available",
                "message": "MCP configuration was saved; runtime safety engine is not available for immediate tool registration."
            }));
        };
        let mut guard = registry.write().await;
        guard
            .sync_from_config(config, secrets, self, safety)
            .await
            .map_err(|error| anyhow::anyhow!("MCP registry sync failed: {}", error))?;
        Ok(serde_json::json!({ "sync_status": "synced" }))
    }

    pub(in crate::runtime) async fn execute_mcp_server_manage(
        &self,
        arguments: &serde_json::Value,
    ) -> Result<String> {
        let requested_operation = Self::capability_string_argument(arguments, "operation")
            .or_else(|| Self::capability_string_argument(arguments, "op"))
            .unwrap_or_else(|| "status".to_string())
            .to_ascii_lowercase();
        let operation = match requested_operation.as_str() {
            "install" | "connect" => "create".to_string(),
            "read" => "status".to_string(),
            other => other.to_string(),
        };
        let manager = self.settings_manager()?;
        let mut config = manager.load()?;

        if matches!(operation.as_str(), "list" | "status") {
            let include_details = matches!(operation.as_str(), "status");
            if let Some(registry) = self.mcp_registry.clone() {
                let guard = registry.read().await;
                let mut servers = guard.list_servers(include_details).await?;
                if let Some(id) = Self::capability_string_argument(arguments, "id") {
                    servers.retain(|server| server.id == id);
                } else if let Some(query) = Self::capability_string_argument(arguments, "query") {
                    let terms = Self::integration_inspect_terms(Some(&query));
                    servers.retain(|server| {
                        serde_json::to_value(server).ok().is_some_and(|value| {
                            Self::integration_value_matches(&value, None, &terms)
                        })
                    });
                }
                return Ok(serde_json::to_string_pretty(&serde_json::json!({
                    "status": "ok",
                    "operation": operation,
                    "count": servers.len(),
                    "servers": servers,
                }))?);
            }
            return Ok(serde_json::to_string_pretty(&serde_json::json!({
                "status": "ok",
                "operation": operation,
                "count": config.mcp.servers.len(),
                "servers": config.mcp.servers,
                "sync_status": "registry_not_available",
            }))?);
        }

        if operation == "delete" {
            let id = Self::capability_string_argument(arguments, "id")
                .ok_or_else(|| anyhow::anyhow!("mcp_server delete requires id"))?;
            let before = config.mcp.servers.len();
            config.mcp.servers.retain(|server| server.id != id);
            if config.mcp.servers.len() == before {
                anyhow::bail!("MCP server `{}` was not found", id);
            }
            manager.update_secrets(|secrets| {
                secrets.mcp_auth.remove(&id);
                secrets.mcp_env.remove(&id);
                Ok(())
            })?;
            manager.save(&config)?;
            let secrets = manager.load_secrets()?;
            let sync = self
                .sync_mcp_registry_from_saved_config(&config, &secrets)
                .await?;
            return Ok(serde_json::to_string_pretty(&serde_json::json!({
                "status": "deleted",
                "server_id": id,
                "sync": sync,
            }))?);
        }

        if operation == "refresh" {
            let id = Self::capability_string_argument(arguments, "id")
                .ok_or_else(|| anyhow::anyhow!("mcp_server refresh requires id"))?;
            if !config.mcp.servers.iter().any(|server| server.id == id) {
                anyhow::bail!("MCP server `{}` was not found", id);
            }
            let secrets = manager.load_secrets()?;
            let sync = self
                .sync_mcp_registry_from_saved_config(&config, &secrets)
                .await?;
            let server = if let Some(registry) = self.mcp_registry.clone() {
                let guard = registry.read().await;
                guard.get_server(&id, true).await?
            } else {
                None
            };
            return Ok(serde_json::to_string_pretty(&serde_json::json!({
                "status": "refresh_requested",
                "server_id": id,
                "server": server,
                "sync": sync,
            }))?);
        }

        if !matches!(operation.as_str(), "create" | "update") {
            anyhow::bail!("Unsupported MCP server operation `{}`", operation);
        }

        let allow_duplicate = arguments
            .get("allow_duplicate")
            .and_then(|value| value.as_bool())
            .unwrap_or(false);
        let requested_id = Self::capability_string_argument(arguments, "id");
        let existing_by_id = requested_id.as_ref().and_then(|id| {
            config
                .mcp
                .servers
                .iter()
                .position(|server| server.id == *id)
        });
        let existing_for_parse = existing_by_id.and_then(|index| config.mcp.servers.get(index));
        let (transport, env_update) =
            Self::mcp_transport_from_arguments(arguments, existing_for_parse)?;
        let existing_by_transport = if allow_duplicate {
            None
        } else {
            config
                .mcp
                .servers
                .iter()
                .enumerate()
                .find(|(index, server)| {
                    existing_by_id != Some(*index)
                        && Self::mcp_transport_same_endpoint(&server.transport, &transport)
                })
                .map(|(index, _)| index)
        };
        let target_index = existing_by_id.or(existing_by_transport);
        let existing = target_index.and_then(|index| config.mcp.servers.get(index));
        let mut server_id = requested_id.unwrap_or_else(|| {
            existing
                .map(|server| server.id.clone())
                .unwrap_or_else(|| Self::mcp_server_id_from_arguments(arguments, &transport))
        });
        if allow_duplicate
            && config
                .mcp
                .servers
                .iter()
                .any(|server| server.id == server_id)
        {
            server_id = format!("{}-{}", server_id, uuid::Uuid::new_v4().simple());
        }
        let name = Self::capability_string_argument(arguments, "name")
            .or_else(|| existing.map(|server| server.name.clone()))
            .unwrap_or_else(|| server_id.replace('-', " "));
        let auth_profile_id = Self::capability_string_argument(arguments, "auth_profile_id")
            .or_else(|| existing.and_then(|server| server.auth_profile_id.clone()));
        let (auth, auth_secret_update, clear_auth) =
            Self::mcp_auth_from_arguments(arguments, existing, auth_profile_id.as_deref())?;
        let server_config = crate::core::runtime::config::McpServerConfig {
            id: server_id.clone(),
            name: name.trim().to_string(),
            description: Self::capability_string_argument(arguments, "description")
                .or_else(|| existing.and_then(|server| server.description.clone())),
            transport,
            enabled: arguments
                .get("enabled")
                .and_then(|value| value.as_bool())
                .or_else(|| existing.map(|server| server.enabled))
                .unwrap_or(true),
            resources_enabled: arguments
                .get("resources_enabled")
                .and_then(|value| value.as_bool())
                .or_else(|| existing.map(|server| server.resources_enabled))
                .unwrap_or(false),
            auth,
            auth_profile_id,
            tool_allowlist: {
                let value = Self::string_array_argument(arguments, "tool_allowlist");
                if value.is_empty() {
                    existing
                        .map(|server| server.tool_allowlist.clone())
                        .unwrap_or_default()
                } else {
                    value
                }
            },
            tool_blocklist: {
                let value = Self::string_array_argument(arguments, "tool_blocklist");
                if value.is_empty() {
                    existing
                        .map(|server| server.tool_blocklist.clone())
                        .unwrap_or_default()
                } else {
                    value
                }
            },
            resource_allowlist: {
                let value = Self::string_array_argument(arguments, "resource_allowlist");
                if value.is_empty() {
                    existing
                        .map(|server| server.resource_allowlist.clone())
                        .unwrap_or_default()
                } else {
                    value
                }
            },
            timeout_secs: arguments
                .get("timeout_secs")
                .and_then(|value| value.as_u64())
                .or_else(|| existing.map(|server| server.timeout_secs))
                .unwrap_or(15),
            max_response_bytes: arguments
                .get("max_response_bytes")
                .and_then(|value| value.as_u64())
                .and_then(|value| usize::try_from(value).ok())
                .or_else(|| existing.map(|server| server.max_response_bytes))
                .unwrap_or(1024 * 1024),
        };

        if let Some(index) = target_index {
            config.mcp.servers[index] = server_config.clone();
        } else {
            config.mcp.servers.push(server_config.clone());
        }
        manager.update_secrets(|secrets| {
            if clear_auth {
                secrets.mcp_auth.remove(&server_id);
            }
            if let Some(secret) = auth_secret_update.clone() {
                secrets.mcp_auth.insert(server_id.clone(), secret);
            }
            if let Some(env) = env_update.clone() {
                if env.is_empty() {
                    secrets.mcp_env.remove(&server_id);
                } else {
                    secrets.mcp_env.insert(server_id.clone(), env);
                }
            }
            Ok(())
        })?;
        manager.save(&config)?;
        let secrets = manager.load_secrets()?;
        let sync = self
            .sync_mcp_registry_from_saved_config(&config, &secrets)
            .await?;
        let server = if let Some(registry) = self.mcp_registry.clone() {
            let guard = registry.read().await;
            guard.get_server(&server_id, true).await?
        } else {
            None
        };
        let needs_credentials = server
            .as_ref()
            .map(|server| {
                matches!(
                    server.auth.auth_type.as_str(),
                    "bearer" | "basic" | "header" | "query" | "auth_profile"
                ) && !server.auth.has_auth
            })
            .unwrap_or_else(|| {
                server_config.auth.is_some()
                    || server_config
                        .auth_profile_id
                        .as_deref()
                        .is_some_and(|value| !value.trim().is_empty())
            });
        let settings_path = format!(
            "Settings > Integrations > MCP Servers > {}",
            server_config.name
        );
        let credential_request = if needs_credentials {
            Self::mcp_credential_request(
                &server_id,
                &server_config.name,
                server_config.auth.as_ref(),
                &settings_path,
            )
        } else {
            None
        };
        let message = if needs_credentials {
            format!(
                "MCP server configuration saved. Credentials are still required through the secure credential form or {}.",
                settings_path
            )
        } else {
            "MCP server configuration saved.".to_string()
        };
        // Structured completion (top-level tool + status) so the legacy tool
        // wrapper preserves needs_credentials verbatim and the spine surfaces
        // the secure-credential handoff instead of burying it as "completed".
        Ok(structured_tool_completion_output(
            "mcp_server_manage",
            if needs_credentials {
                "needs_credentials"
            } else {
                "configured"
            },
            message.clone(),
            serde_json::json!({
                "operation": if target_index.is_some() { "update" } else { "create" },
                "server_id": server_id,
                "settings_path": settings_path,
                "server": server,
                "sync": sync,
                "credential_request": credential_request,
                "message": message,
            }),
        ))
    }

    pub(in crate::runtime) fn extension_pack_view_is_connected(
        pack: &crate::extension_packs::ExtensionPackView,
    ) -> bool {
        pack.enabled && matches!(pack.status.as_str(), "ready" | "connected")
    }

    pub(in crate::runtime) fn integration_catalog_sorted_strings(
        mut values: Vec<String>,
    ) -> Vec<String> {
        values.retain(|value| !value.trim().is_empty());
        for value in &mut values {
            *value = value.trim().to_string();
        }
        values.sort();
        values.dedup();
        values
    }

    pub(in crate::runtime) fn integration_catalog_string_array(
        value: Option<&serde_json::Value>,
    ) -> Vec<String> {
        value
            .and_then(|value| value.as_array())
            .into_iter()
            .flatten()
            .filter_map(|value| value.as_str())
            .map(str::trim)
            .filter(|value| !value.is_empty())
            .map(ToOwned::to_owned)
            .collect()
    }

    pub(in crate::runtime) fn integration_catalog_native_auth_mode(
        integration_id: &str,
    ) -> &'static str {
        if crate::core::connectivity::connect_flow::spec_by_id(integration_id).is_some() {
            "secret"
        } else {
            "native"
        }
    }

    pub(in crate::runtime) fn integration_catalog_entry_from_builtin(
        &self,
        record: &serde_json::Value,
    ) -> Option<serde_json::Value> {
        let id = record.get("id").and_then(|value| value.as_str())?;
        let name = record
            .get("name")
            .and_then(|value| value.as_str())
            .unwrap_or(id);
        let capabilities = Self::integration_catalog_sorted_strings(
            Self::integration_catalog_string_array(record.get("capabilities"))
                .into_iter()
                .map(|value| value.to_ascii_lowercase())
                .collect(),
        );
        let action_names = Self::integration_catalog_sorted_strings(
            Self::integration_catalog_string_array(record.get("available_actions")),
        );
        let auth_mode = Self::integration_catalog_native_auth_mode(id);
        let enabled = crate::integrations::effective_integration_enabled(&self.config_dir, id);
        let connected = enabled
            && record
                .get("connected")
                .and_then(|value| value.as_bool())
                .unwrap_or(false);
        let read_capable = record
            .get("read_capable")
            .and_then(|value| value.as_bool())
            .unwrap_or_else(|| {
                capabilities
                    .iter()
                    .any(|capability| matches!(capability.as_str(), "read" | "search"))
            });
        let write_capable = record
            .get("write_capable")
            .and_then(|value| value.as_bool())
            .unwrap_or_else(|| {
                capabilities.iter().any(|capability| {
                    matches!(
                        capability.as_str(),
                        "write" | "delete" | "notify" | "external_write"
                    )
                })
            });
        let status = record
            .get("status_label")
            .or_else(|| record.get("status"))
            .and_then(|value| value.as_str())
            .unwrap_or("unknown");

        Some(serde_json::json!({
            "id": id,
            "name": name,
            "description": record.get("description").and_then(|value| value.as_str()).unwrap_or(""),
            "source_kind": "native",
            "auth_mode": auth_mode,
            "connected": connected,
            "enabled": enabled,
            "status": status,
            "status_detail": record.get("status_detail"),
            "connection_required": !connected && !matches!(auth_mode, "none"),
            "read_capable": read_capable,
            "write_capable": write_capable,
            "capabilities": capabilities,
            "action_names": action_names,
            "required_scopes": [],
            "connection_required_for_actions": !connected && !action_names.is_empty(),
            "source_record": record,
        }))
    }

    pub(in crate::runtime) fn extension_pack_auth_mode_label(
        mode: crate::extension_packs::ExtensionPackAuthMode,
    ) -> &'static str {
        match mode {
            crate::extension_packs::ExtensionPackAuthMode::None => "none",
            crate::extension_packs::ExtensionPackAuthMode::ApiKey => "api_key",
            crate::extension_packs::ExtensionPackAuthMode::Basic => "basic",
            crate::extension_packs::ExtensionPackAuthMode::OAuth2External => "oauth",
        }
    }

    pub(in crate::runtime) fn integration_catalog_entry_from_pack(
        view: &crate::extension_packs::ExtensionPackView,
    ) -> serde_json::Value {
        let mut required_scopes = view.manifest.auth.required_scopes.clone();
        if let Some(oauth2) = view.manifest.auth.oauth2.as_ref() {
            required_scopes.extend(oauth2.scopes.clone());
        }
        required_scopes = Self::integration_catalog_sorted_strings(required_scopes);

        let action_names = Self::integration_catalog_sorted_strings(
            view.manifest
                .features
                .iter()
                .filter(|feature| !feature.kind.eq_ignore_ascii_case("event"))
                .filter(|feature| {
                    feature.binding.as_ref().is_some_and(|binding| {
                        !binding.kind.trim().is_empty()
                            && !binding.kind.eq_ignore_ascii_case("unsupported")
                    })
                })
                .map(|feature| format!("{}.{}", view.manifest.id, feature.id))
                .collect(),
        );
        let mut capabilities = Vec::new();
        capabilities.extend(view.manifest.tags.clone());
        capabilities.extend(
            view.manifest
                .features
                .iter()
                .map(|feature| feature.kind.clone()),
        );
        if view
            .manifest
            .features
            .iter()
            .any(|feature| feature.read_only)
        {
            capabilities.push("read".to_string());
        }
        if view
            .manifest
            .features
            .iter()
            .any(|feature| !feature.read_only)
        {
            capabilities.push("write".to_string());
        }
        capabilities = Self::integration_catalog_sorted_strings(
            capabilities
                .into_iter()
                .map(|value| value.to_ascii_lowercase())
                .collect(),
        );
        let connected = Self::extension_pack_view_is_connected(view);
        let auth_mode = Self::extension_pack_auth_mode_label(view.manifest.auth.mode);

        serde_json::json!({
            "id": view.manifest.id.clone(),
            "name": view.manifest.name.clone(),
            "description": view.manifest.description.clone(),
            "source_kind": if view.installed { "extension_pack" } else { "catalog_extension_pack" },
            "auth_mode": auth_mode,
            "connected": connected,
            "enabled": view.enabled,
            "status": view.status.clone(),
            "status_detail": view.status_detail.clone(),
            "connection_required": view.needs_auth && !connected,
            "read_capable": view.manifest.features.iter().any(|feature| feature.read_only),
            "write_capable": view.manifest.features.iter().any(|feature| !feature.read_only),
            "capabilities": capabilities,
            "action_names": action_names,
            "required_scopes": required_scopes,
            "supports_connect_url": view.supports_connect_url,
            "runtime_required": view.runtime_required,
            "runtime_status": view.runtime_status,
            "features": view.feature_summaries.clone(),
        })
    }

    pub(in crate::runtime) async fn integration_catalog_entries(
        &self,
    ) -> Result<Vec<serde_json::Value>> {
        let (builtin_integrations, _) = self.builtin_integrations_inventory(false).await;
        let mut entries = builtin_integrations
            .get("integrations")
            .and_then(|value| value.as_array())
            .into_iter()
            .flatten()
            .filter_map(|record| self.integration_catalog_entry_from_builtin(record))
            .collect::<Vec<_>>();

        if let Some(registry) = self.extension_pack_registry.clone() {
            let guard = registry.read().await;
            let packs = guard.search_packs(None, None).await?;
            entries.extend(
                packs
                    .installed
                    .iter()
                    .map(Self::integration_catalog_entry_from_pack),
            );
            entries.extend(
                packs
                    .catalog
                    .iter()
                    .map(Self::integration_catalog_entry_from_pack),
            );
        }

        entries.sort_by(|left, right| {
            let left_key = format!(
                "{}:{}",
                left.get("source_kind")
                    .and_then(|value| value.as_str())
                    .unwrap_or(""),
                left.get("id")
                    .and_then(|value| value.as_str())
                    .unwrap_or("")
            );
            let right_key = format!(
                "{}:{}",
                right
                    .get("source_kind")
                    .and_then(|value| value.as_str())
                    .unwrap_or(""),
                right
                    .get("id")
                    .and_then(|value| value.as_str())
                    .unwrap_or("")
            );
            left_key.cmp(&right_key)
        });
        Ok(entries)
    }

    pub(in crate::runtime) fn integration_catalog_filter_entries(
        mut entries: Vec<serde_json::Value>,
        ids: Vec<String>,
        source_kind: Option<&str>,
        only_connected: bool,
    ) -> Vec<serde_json::Value> {
        if !ids.is_empty() {
            let ids = ids
                .into_iter()
                .map(|value| value.trim().to_ascii_lowercase())
                .filter(|value| !value.is_empty())
                .collect::<BTreeSet<_>>();
            entries.retain(|entry| {
                entry
                    .get("id")
                    .and_then(|value| value.as_str())
                    .map(|value| value.to_ascii_lowercase())
                    .is_some_and(|id| ids.contains(&id))
            });
        }
        if let Some(source_kind) = source_kind.map(str::trim).filter(|value| !value.is_empty()) {
            entries.retain(|entry| {
                entry
                    .get("source_kind")
                    .and_then(|value| value.as_str())
                    .is_some_and(|value| value.eq_ignore_ascii_case(source_kind))
            });
        }
        if only_connected {
            entries.retain(|entry| {
                entry
                    .get("connected")
                    .and_then(|value| value.as_bool())
                    .unwrap_or(false)
                    && entry
                        .get("enabled")
                        .and_then(|value| value.as_bool())
                        .unwrap_or(false)
            });
        }
        entries
    }

    pub(in crate::runtime) async fn integration_catalog_find_entry(
        &self,
        arguments: &serde_json::Value,
    ) -> Result<Option<serde_json::Value>> {
        let id = arguments
            .get("id")
            .and_then(|value| value.as_str())
            .map(str::trim)
            .filter(|value| !value.is_empty());
        let Some(id) = id else {
            anyhow::bail!("integration catalog lookup requires an id");
        };
        Ok(self
            .integration_catalog_entries()
            .await?
            .into_iter()
            .find(|entry| {
                entry
                    .get("id")
                    .and_then(|value| value.as_str())
                    .is_some_and(|entry_id| entry_id.eq_ignore_ascii_case(id))
            }))
    }

    pub(in crate::runtime) async fn execute_integration_catalog_list(
        &self,
        arguments: &serde_json::Value,
    ) -> Result<String> {
        let ids = Self::integration_catalog_string_array(arguments.get("ids"));
        let source_kind = arguments
            .get("source_kind")
            .and_then(|value| value.as_str())
            .map(str::trim)
            .filter(|value| !value.is_empty());
        let only_connected = arguments
            .get("only_connected")
            .and_then(|value| value.as_bool())
            .unwrap_or(false);
        let entries = Self::integration_catalog_filter_entries(
            self.integration_catalog_entries().await?,
            ids.clone(),
            source_kind,
            only_connected,
        );
        let mut source_counts = serde_json::Map::new();
        for entry in &entries {
            let source = entry
                .get("source_kind")
                .and_then(|value| value.as_str())
                .unwrap_or("unknown")
                .to_string();
            let next = source_counts
                .get(&source)
                .and_then(|value| value.as_u64())
                .unwrap_or_default()
                + 1;
            source_counts.insert(source, serde_json::json!(next));
        }
        let connected_total = entries
            .iter()
            .filter(|entry| {
                entry
                    .get("connected")
                    .and_then(|value| value.as_bool())
                    .unwrap_or(false)
            })
            .count();

        Ok(serde_json::to_string_pretty(&serde_json::json!({
            "status": "ok",
            "ids": ids,
            "only_connected": only_connected,
            "source_kind": source_kind,
            "total": entries.len(),
            "connected_total": connected_total,
            "source_counts": source_counts,
            "entries": entries,
            "detail_available_via": "integration_catalog_describe",
            "status_available_via": "integration_catalog_status",
        }))?)
    }

    pub(in crate::runtime) async fn execute_integration_catalog_describe(
        &self,
        arguments: &serde_json::Value,
    ) -> Result<String> {
        let entry = self.integration_catalog_find_entry(arguments).await?;
        Ok(serde_json::to_string_pretty(&match entry {
            Some(entry) => serde_json::json!({
                "status": "ok",
                "entry": entry,
            }),
            None => serde_json::json!({
                "status": "not_found",
                "id": arguments.get("id"),
            }),
        })?)
    }

    pub(in crate::runtime) async fn execute_integration_catalog_status(
        &self,
        arguments: &serde_json::Value,
    ) -> Result<String> {
        let entry = self.integration_catalog_find_entry(arguments).await?;
        let Some(entry) = entry else {
            return Ok(serde_json::to_string_pretty(&serde_json::json!({
                "status": "not_found",
                "id": arguments.get("id"),
            }))?);
        };

        Ok(serde_json::to_string_pretty(&serde_json::json!({
            "status": "ok",
            "id": entry.get("id"),
            "name": entry.get("name"),
            "source_kind": entry.get("source_kind"),
            "auth_mode": entry.get("auth_mode"),
            "connected": entry.get("connected"),
            "enabled": entry.get("enabled"),
            "integration_status": entry.get("status"),
            "status_detail": entry.get("status_detail"),
            "connection_required": entry.get("connection_required"),
            "read_capable": entry.get("read_capable"),
            "write_capable": entry.get("write_capable"),
            "capabilities": entry.get("capabilities"),
            "action_names": entry.get("action_names"),
            "required_scopes": entry.get("required_scopes"),
        }))?)
    }

    pub(in crate::runtime) async fn execute_list_integrations(
        &self,
        arguments: &serde_json::Value,
    ) -> Result<String> {
        let query = arguments.get("query").and_then(|value| value.as_str());
        let query_terms = Self::integration_inspect_terms(query);
        let kind = arguments.get("kind").and_then(|value| value.as_str());
        let only_connected = arguments
            .get("only_connected")
            .and_then(|value| value.as_bool())
            .unwrap_or(false);
        let include_details = arguments
            .get("include_details")
            .and_then(|value| value.as_bool())
            .unwrap_or(false);
        let mut packs = if let Some(registry) = self.extension_pack_registry.clone() {
            let guard = registry.read().await;
            Some(guard.search_packs(query, kind).await?)
        } else {
            None
        };
        let mut connected_items = Vec::new();
        if let Some(packs) = packs.as_mut() {
            if only_connected {
                packs
                    .installed
                    .retain(Self::extension_pack_view_is_connected);
            }
            for pack in &packs.installed {
                let connected = Self::extension_pack_view_is_connected(pack);
                if connected {
                    connected_items.push(Self::connected_surface_item(
                        "extension_packs",
                        pack.manifest.id.clone(),
                        pack.manifest.name.clone(),
                        "extension_pack",
                        pack.status.clone(),
                    ));
                }
            }
        }
        let (plugins, plugin_connected) = self.plugins_inventory(only_connected).await;
        connected_items.extend(plugin_connected);
        let (mcp_servers, mcp_connected) = self.mcp_servers_inventory(only_connected).await;
        connected_items.extend(mcp_connected);
        let (mut builtin_integrations, builtin_connected) =
            self.builtin_integrations_inventory(only_connected).await;
        connected_items.extend(builtin_connected);
        let (mut gateway_channels, gateway_connected, bundled_configured) =
            self.gateway_channels_inventory(only_connected).await;
        connected_items.extend(gateway_connected);
        let (mut messaging_channels, messaging_connected) = self
            .messaging_channels_inventory(only_connected, &bundled_configured)
            .await;
        connected_items.extend(messaging_connected);
        let (mut custom_apis, custom_api_connected) =
            self.custom_apis_inventory(only_connected).await;
        connected_items.extend(custom_api_connected);
        let (mut webhook_sources, webhook_connected) =
            self.webhook_sources_inventory(only_connected).await;
        connected_items.extend(webhook_connected);
        let (mut companion_devices, companion_connected) =
            self.companion_device_inventory(only_connected).await;
        connected_items.extend(companion_connected);
        if !query_terms.is_empty() {
            connected_items
                .retain(|item| Self::integration_value_matches(item, None, &query_terms));
            Self::filter_inventory_array_field(
                &mut builtin_integrations,
                "integrations",
                &query_terms,
            );
            Self::filter_inventory_array_field(&mut gateway_channels, "channels", &query_terms);
            Self::filter_inventory_array_field(&mut messaging_channels, "channels", &query_terms);
            Self::filter_inventory_array_field(&mut custom_apis, "custom_apis", &query_terms);
            Self::filter_inventory_array_field(&mut webhook_sources, "sources", &query_terms);
            Self::filter_inventory_array_field(&mut companion_devices, "devices", &query_terms);
        }
        connected_items.sort_by(|left, right| {
            let left_key = format!(
                "{}:{}",
                left.get("surface")
                    .and_then(|value| value.as_str())
                    .unwrap_or(""),
                left.get("id")
                    .and_then(|value| value.as_str())
                    .unwrap_or("")
            );
            let right_key = format!(
                "{}:{}",
                right
                    .get("surface")
                    .and_then(|value| value.as_str())
                    .unwrap_or(""),
                right
                    .get("id")
                    .and_then(|value| value.as_str())
                    .unwrap_or("")
            );
            left_key.cmp(&right_key)
        });
        let mut section_counts = serde_json::Map::new();
        section_counts.insert(
            "builtin_integrations".to_string(),
            Self::integration_inventory_section_counts(&builtin_integrations)
                .get("builtin_integrations")
                .cloned()
                .unwrap_or_else(|| serde_json::json!(0)),
        );
        section_counts.insert(
            "gateway_channels".to_string(),
            Self::integration_inventory_section_counts(&gateway_channels)
                .get("gateway_channels")
                .cloned()
                .unwrap_or_else(|| serde_json::json!(0)),
        );
        section_counts.insert(
            "notification_channels".to_string(),
            Self::integration_inventory_section_counts(&messaging_channels)
                .get("notification_channels")
                .cloned()
                .unwrap_or_else(|| serde_json::json!(0)),
        );
        section_counts.insert(
            "custom_apis".to_string(),
            Self::integration_inventory_section_counts(&custom_apis)
                .get("custom_apis")
                .cloned()
                .unwrap_or_else(|| serde_json::json!(0)),
        );
        section_counts.insert(
            "webhook_sources".to_string(),
            Self::integration_inventory_section_counts(&webhook_sources)
                .get("webhook_sources")
                .cloned()
                .unwrap_or_else(|| serde_json::json!(0)),
        );
        section_counts.insert(
            "companion_devices".to_string(),
            Self::integration_inventory_section_counts(&companion_devices)
                .get("companion_devices")
                .cloned()
                .unwrap_or_else(|| serde_json::json!(0)),
        );
        section_counts.insert(
            "extension_packs_installed".to_string(),
            serde_json::json!(packs
                .as_ref()
                .map(|packs| packs.installed.len())
                .unwrap_or_default()),
        );
        section_counts.insert(
            "plugins".to_string(),
            serde_json::json!(plugins.as_ref().map(Vec::len).unwrap_or_default()),
        );
        section_counts.insert(
            "mcp_servers".to_string(),
            serde_json::json!(mcp_servers.as_ref().map(Vec::len).unwrap_or_default()),
        );

        let mut payload = serde_json::json!({
            "connected_agentark_surfaces": {
                "total": connected_items.len(),
                "items": connected_items,
            },
            "section_counts": section_counts,
            "detail_available_via": "inspect_integration",
        });
        if include_details {
            if let Some(object) = payload.as_object_mut() {
                object.insert("builtin_integrations".to_string(), builtin_integrations);
                object.insert("gateway_channels".to_string(), gateway_channels);
                object.insert("notification_channels".to_string(), messaging_channels);
                object.insert("custom_apis".to_string(), custom_apis);
                object.insert("webhook_sources".to_string(), webhook_sources);
                object.insert("companion_devices".to_string(), companion_devices);
                object.insert("extension_packs".to_string(), serde_json::to_value(packs)?);
                object.insert("plugins".to_string(), serde_json::to_value(plugins)?);
                object.insert(
                    "mcp_servers".to_string(),
                    serde_json::to_value(mcp_servers)?,
                );
            }
        }
        Ok(serde_json::to_string_pretty(&payload)?)
    }

    pub(in crate::runtime) fn filter_inventory_array_field(
        value: &mut serde_json::Value,
        field: &str,
        query_terms: &[String],
    ) {
        if query_terms.is_empty() {
            return;
        }
        let Some(items) = value.get_mut(field).and_then(|value| value.as_array_mut()) else {
            return;
        };
        items.retain(|item| Self::integration_value_matches(item, None, query_terms));
    }

    pub(in crate::runtime) fn integration_inspect_terms(value: Option<&str>) -> Vec<String> {
        value
            .unwrap_or_default()
            .split(|ch: char| !ch.is_alphanumeric())
            .map(|part| part.trim().to_ascii_lowercase())
            .filter(|part| part.chars().count() >= 2)
            .collect()
    }

    pub(in crate::runtime) fn integration_value_text(value: &serde_json::Value, out: &mut String) {
        match value {
            serde_json::Value::Null | serde_json::Value::Bool(_) | serde_json::Value::Number(_) => {
            }
            serde_json::Value::String(text) => {
                out.push(' ');
                out.push_str(&text.to_ascii_lowercase());
            }
            serde_json::Value::Array(items) => {
                for item in items {
                    Self::integration_value_text(item, out);
                }
            }
            serde_json::Value::Object(map) => {
                for (key, item) in map {
                    out.push(' ');
                    out.push_str(&key.to_ascii_lowercase());
                    Self::integration_value_text(item, out);
                }
            }
        }
    }

    pub(in crate::runtime) fn integration_value_matches(
        value: &serde_json::Value,
        id: Option<&str>,
        query_terms: &[String],
    ) -> bool {
        if let Some(id) = id.map(str::trim).filter(|id| !id.is_empty()) {
            let id_lower = id.to_ascii_lowercase();
            if let Some(map) = value.as_object() {
                for key in [
                    "id",
                    "name",
                    "display_name",
                    "runtime_channel_id",
                    "channel_id",
                    "pack_id",
                ] {
                    if map
                        .get(key)
                        .and_then(|item| item.as_str())
                        .is_some_and(|candidate| candidate.eq_ignore_ascii_case(id))
                    {
                        return true;
                    }
                }
                if let Some(manifest) = map.get("manifest").and_then(|item| item.as_object()) {
                    for key in ["id", "name"] {
                        if manifest
                            .get(key)
                            .and_then(|item| item.as_str())
                            .is_some_and(|candidate| candidate.eq_ignore_ascii_case(id))
                        {
                            return true;
                        }
                    }
                }
                if let Some(connection) = map.get("connection").and_then(|item| item.as_object()) {
                    for key in ["id", "name", "pack_id"] {
                        if connection
                            .get(key)
                            .and_then(|item| item.as_str())
                            .is_some_and(|candidate| candidate.eq_ignore_ascii_case(id))
                        {
                            return true;
                        }
                    }
                }
            }
            let mut text = String::new();
            Self::integration_value_text(value, &mut text);
            if text.split_whitespace().any(|part| part == id_lower) {
                return true;
            }
            // Canonical-slug identity, mirroring the execution-path resolver
            // (custom_api_view_matches_selector): "Linear GraphQL API" and
            // "linear graphql api" both canonicalize to "linear-graphql-api",
            // and a single canonical token ("linear") matches any record whose
            // canonical identity contains it as a token. Without this, inspect
            // and execution resolve the SAME record differently — the
            // dual-resolver divergence that produced false "not installed"
            // verdicts for installed integrations.
            if Self::integration_identity_matches_canonical(value, id) {
                return true;
            }
        }

        if query_terms.is_empty() {
            return false;
        }
        let mut text = String::new();
        Self::integration_value_text(value, &mut text);
        if query_terms.iter().all(|term| text.contains(term)) {
            return true;
        }
        // Whole-phrase slug equality: terms rejoined with '-' reconstruct the
        // canonical phrase ("linear graphql api" -> "linear-graphql-api").
        if Self::integration_identity_matches_canonical(value, &query_terms.join("-")) {
            return true;
        }
        // Token-level fallback: free-text queries (often verifier-authored)
        // carry descriptive words ("integration", "installed") that no record
        // contains, which made the all-substrings rule miss EVERY record.
        // Match when any sufficiently-specific canonical query token equals a
        // canonical identity token of the record.
        query_terms
            .iter()
            .filter(|term| term.chars().count() >= 4)
            .any(|term| Self::integration_identity_matches_canonical(value, term))
    }

    /// Slug-canonical identity comparison shared with the execution-path
    /// resolver semantics: exact canonical equality against the record's
    /// identity fields, plus single-token membership for tokens of length
    /// >= 4 (so "linear" resolves "linear-graphql-api" without "api"
    /// matching every API record).
    pub(in crate::runtime) fn integration_identity_matches_canonical(
        value: &serde_json::Value,
        raw: &str,
    ) -> bool {
        let canonical = Self::normalize_generated_action_name(raw);
        if canonical.is_empty() {
            return false;
        }
        let Some(map) = value.as_object() else {
            return false;
        };
        let mut candidates: Vec<&str> = Vec::new();
        for key in [
            "id",
            "name",
            "display_name",
            "description",
            "runtime_channel_id",
            "channel_id",
            "pack_id",
        ] {
            if let Some(text) = map.get(key).and_then(|item| item.as_str()) {
                candidates.push(text);
            }
        }
        for nested in ["manifest", "connection"] {
            if let Some(object) = map.get(nested).and_then(|item| item.as_object()) {
                for key in ["id", "name", "pack_id"] {
                    if let Some(text) = object.get(key).and_then(|item| item.as_str()) {
                        candidates.push(text);
                    }
                }
            }
        }
        let single_token = !canonical.contains('-') && canonical.chars().count() >= 4;
        candidates.iter().any(|candidate| {
            let candidate_canonical = Self::normalize_generated_action_name(candidate);
            candidate_canonical == canonical
                || (single_token
                    && candidate_canonical
                        .split('-')
                        .any(|token| token == canonical))
        })
    }

    pub(in crate::runtime) fn integration_find_matches(
        items: impl IntoIterator<Item = serde_json::Value>,
        id: Option<&str>,
        query_terms: &[String],
        limit: usize,
    ) -> Vec<serde_json::Value> {
        items
            .into_iter()
            .filter(|item| Self::integration_value_matches(item, id, query_terms))
            .take(limit)
            .collect()
    }

    /// Scan-aware variant of `integration_find_matches`: records what was
    /// examined (per-surface totals + a small id sample) before filtering, so
    /// a no-match inspect can prove it searched real records.
    pub(in crate::runtime) fn integration_scan_matches(
        scan: &mut IntegrationInspectScan,
        surface: &str,
        items: Vec<serde_json::Value>,
        id: Option<&str>,
        query_terms: &[String],
        limit: usize,
    ) -> Vec<serde_json::Value> {
        scan.note(surface, &items);
        Self::integration_find_matches(items, id, query_terms, limit)
    }

    pub(in crate::runtime) fn requested_surface_matches(
        requested: Option<&str>,
        candidates: &[&str],
    ) -> bool {
        let Some(requested) = requested.map(str::trim).filter(|value| !value.is_empty()) else {
            return true;
        };
        candidates
            .iter()
            .any(|candidate| requested.eq_ignore_ascii_case(candidate))
    }

    pub(in crate::runtime) async fn execute_inspect_integration(
        &self,
        arguments: &serde_json::Value,
    ) -> Result<String> {
        let surface = arguments
            .get("surface")
            .and_then(|value| value.as_str())
            .map(str::trim)
            .filter(|value| !value.is_empty());
        let id = arguments
            .get("id")
            .and_then(|value| value.as_str())
            .map(str::trim)
            .filter(|value| !value.is_empty());
        let query = arguments
            .get("query")
            .and_then(|value| value.as_str())
            .map(str::trim)
            .filter(|value| !value.is_empty());
        let run_check = arguments
            .get("run_check")
            .and_then(|value| value.as_bool())
            .unwrap_or(false);
        if id.is_none() && query.is_none() {
            anyhow::bail!("inspect_integration requires an id or query");
        }
        let query_terms = Self::integration_inspect_terms(query);
        let mut matches = Vec::new();
        let mut checks = Vec::new();
        let mut scan = IntegrationInspectScan::default();

        if Self::requested_surface_matches(surface, &["companion_devices", "companion_device"]) {
            let (payload, _) = self.companion_device_inventory(false).await;
            let devices = payload
                .get("devices")
                .and_then(|value| value.as_array())
                .cloned()
                .unwrap_or_default();
            for record in Self::integration_scan_matches(
                &mut scan,
                "companion_devices",
                devices,
                id,
                &query_terms,
                8,
            ) {
                matches.push(serde_json::json!({
                    "surface": "companion_devices",
                    "record": record,
                    "safe_check": {
                        "ran": true,
                        "kind": "stored_websocket_presence",
                        "connected": record.get("connected").and_then(|value| value.as_bool()).unwrap_or(false),
                        "state": record.get("state"),
                        "last_seen_at": record.get("last_seen_at"),
                    }
                }));
            }
        }

        if Self::requested_surface_matches(surface, &["integrations", "builtin_integrations"]) {
            let manager = crate::integrations::IntegrationManager::new(&self.config_dir);
            let (payload, _) = self.builtin_integrations_inventory(false).await;
            let integrations = payload
                .get("integrations")
                .and_then(|value| value.as_array())
                .cloned()
                .unwrap_or_default();
            for record in Self::integration_scan_matches(
                &mut scan,
                "integrations",
                integrations,
                id,
                &query_terms,
                8,
            ) {
                let integration_id = record.get("id").and_then(|value| value.as_str());
                if integration_id.is_none() {
                    continue;
                }
                let safe_check = if run_check {
                    let ready_for_agent = if record
                        .get("action_backed")
                        .and_then(|value| value.as_bool())
                        .unwrap_or(false)
                    {
                        record
                            .get("connected")
                            .and_then(|value| value.as_bool())
                            .unwrap_or(false)
                    } else if let Some(integration_id) = integration_id {
                        manager.is_ready(integration_id).await
                    } else {
                        false
                    };
                    Some(serde_json::json!({
                        "ran": true,
                        "kind": "readiness_status",
                        "ready_for_agent": ready_for_agent,
                        "available_actions": record.get("available_actions"),
                    }))
                } else {
                    None
                };
                matches.push(serde_json::json!({
                    "surface": "integrations",
                    "record": record,
                    "safe_check": safe_check,
                }));
            }
        }

        if Self::requested_surface_matches(surface, &["gateway_channels", "messaging_channels"]) {
            let (payload, _, _) = self.gateway_channels_inventory(false).await;
            let channels = payload
                .get("channels")
                .and_then(|value| value.as_array())
                .cloned()
                .unwrap_or_default();
            for record in Self::integration_scan_matches(
                &mut scan,
                "gateway_channels",
                channels,
                id,
                &query_terms,
                8,
            ) {
                matches.push(serde_json::json!({
                    "surface": "gateway_channels",
                    "record": record,
                    "safe_check": {
                        "ran": true,
                        "kind": "stored_channel_status",
                        "connected": record.get("connected").and_then(|value| value.as_bool()).unwrap_or(false),
                        "status": record.get("status"),
                    }
                }));
            }
        }

        if Self::requested_surface_matches(surface, &["notification_channels"]) {
            let (_, _, bundled_configured) = self.gateway_channels_inventory(false).await;
            let (payload, _) = self
                .messaging_channels_inventory(false, &bundled_configured)
                .await;
            let channels = payload
                .get("channels")
                .and_then(|value| value.as_array())
                .cloned()
                .unwrap_or_default();
            for record in Self::integration_scan_matches(
                &mut scan,
                "notification_channels",
                channels,
                id,
                &query_terms,
                8,
            ) {
                matches.push(serde_json::json!({
                    "surface": "notification_channels",
                    "record": record,
                    "safe_check": {
                        "ran": true,
                        "kind": "configured_state",
                        "connected": record.get("connected").and_then(|value| value.as_bool()).unwrap_or(false),
                    }
                }));
            }
        }

        if Self::requested_surface_matches(surface, &["custom_apis", "custom_api"]) {
            let (payload, _) = self.custom_apis_inventory(false).await;
            // An unavailable inventory must be visibly different from "zero
            // matching records" — otherwise a storage hiccup reads as
            // integration absence downstream.
            if payload.get("available").and_then(|value| value.as_bool()) == Some(false) {
                scan.note_unavailable(
                    "custom_apis",
                    payload.get("error").and_then(|value| value.as_str()),
                );
            }
            let apis = payload
                .get("custom_apis")
                .and_then(|value| value.as_array())
                .cloned()
                .unwrap_or_default();
            for record in
                Self::integration_scan_matches(&mut scan, "custom_apis", apis, id, &query_terms, 8)
            {
                let mut safe_check = serde_json::json!({
                    "ran": false,
                    "kind": "custom_api_test",
                    "reason": "run_check was false",
                });
                if run_check {
                    if let (Some(storage), Some(api_id)) = (
                        self.storage(),
                        record.get("id").and_then(|value| value.as_str()),
                    ) {
                        safe_check = match Box::pin(crate::custom_apis::test_custom_api(
                            &storage,
                            &self.config_dir,
                            self.data_dir(),
                            self,
                            api_id,
                        ))
                        .await
                        {
                            Ok(result) => serde_json::json!({
                                "ran": true,
                                "kind": "custom_api_test",
                                "ok": result.ok,
                                "action_name": result.action_name,
                                "detail": result.detail,
                            }),
                            Err(error) => serde_json::json!({
                                "ran": true,
                                "kind": "custom_api_test",
                                "ok": false,
                                "error": error.to_string(),
                            }),
                        };
                    }
                }
                matches.push(serde_json::json!({
                    "surface": "custom_apis",
                    "record": record,
                    "safe_check": safe_check,
                }));
            }
        }

        if Self::requested_surface_matches(surface, &["webhook_sources", "webhooks"]) {
            let (payload, _) = self.webhook_sources_inventory(false).await;
            let sources = payload
                .get("sources")
                .and_then(|value| value.as_array())
                .cloned()
                .unwrap_or_default();
            for record in Self::integration_scan_matches(
                &mut scan,
                "webhook_sources",
                sources,
                id,
                &query_terms,
                8,
            ) {
                matches.push(serde_json::json!({
                    "surface": "webhook_sources",
                    "record": record,
                    "safe_check": {
                        "ran": true,
                        "kind": "stored_secret_and_enabled_state",
                        "connected": record.get("connected").and_then(|value| value.as_bool()).unwrap_or(false),
                        "secret_configured": record.get("secret_configured"),
                    }
                }));
            }
        }

        if Self::requested_surface_matches(surface, &["extension_packs", "extension_pack"]) {
            if let Some(registry) = self.extension_pack_registry.clone() {
                let guard = registry.read().await;
                let packs = guard.search_packs(None, None).await?;
                let installed = packs
                    .installed
                    .into_iter()
                    .map(serde_json::to_value)
                    .collect::<std::result::Result<Vec<_>, _>>()?;
                for record in Self::integration_scan_matches(
                    &mut scan,
                    "extension_packs",
                    installed,
                    id,
                    &query_terms,
                    8,
                ) {
                    let pack_id = record
                        .get("manifest")
                        .and_then(|manifest| manifest.get("id"))
                        .and_then(|value| value.as_str())
                        .unwrap_or_default();
                    let connections = guard
                        .list_connections(pack_id)
                        .await
                        .ok()
                        .and_then(|value| serde_json::to_value(value).ok())
                        .unwrap_or_else(|| serde_json::json!([]));
                    let events = guard
                        .list_events(pack_id, 10)
                        .await
                        .ok()
                        .and_then(|value| serde_json::to_value(value).ok());
                    matches.push(serde_json::json!({
                        "surface": "extension_packs",
                        "record": record,
                        "connections": connections,
                        "recent_events": events,
                        "safe_check": {
                            "ran": true,
                            "kind": "connection_state",
                        }
                    }));
                }
            }
        }

        if Self::requested_surface_matches(surface, &["plugins"]) {
            if let Some(registry) = self.plugin_registry.clone() {
                let guard = registry.read().await;
                let plugins = guard
                    .list_plugins()
                    .await?
                    .into_iter()
                    .map(serde_json::to_value)
                    .collect::<std::result::Result<Vec<_>, _>>()?;
                for record in Self::integration_scan_matches(
                    &mut scan,
                    "plugins",
                    plugins,
                    id,
                    &query_terms,
                    8,
                ) {
                    matches.push(serde_json::json!({
                        "surface": "plugins",
                        "record": record,
                        "safe_check": {
                            "ran": true,
                            "kind": "stored_plugin_status",
                            "connected": record.get("enabled").and_then(|value| value.as_bool()).unwrap_or(false)
                                && record.get("last_error").is_none(),
                        }
                    }));
                }
            }
        }

        if Self::requested_surface_matches(surface, &["mcp_servers", "mcp"]) {
            if let Some(registry) = self.mcp_registry.clone() {
                let guard = registry.read().await;
                let servers = guard
                    .list_servers(true)
                    .await?
                    .into_iter()
                    .map(serde_json::to_value)
                    .collect::<std::result::Result<Vec<_>, _>>()?;
                for record in Self::integration_scan_matches(
                    &mut scan,
                    "mcp_servers",
                    servers,
                    id,
                    &query_terms,
                    8,
                ) {
                    matches.push(serde_json::json!({
                        "surface": "mcp_servers",
                        "record": record,
                        "safe_check": {
                            "ran": true,
                            "kind": "registered_tool_resource_state",
                            "connected": record.get("enabled").and_then(|value| value.as_bool()).unwrap_or(false)
                                && record.get("last_error").is_none(),
                        }
                    }));
                }
            }
        }

        checks.push(serde_json::json!({
            "run_check_requested": run_check,
            "matches_returned": matches.len(),
            "truncation_avoidance": "list_integrations returns compact overview by default; this action returns targeted detail.",
        }));

        // A query miss over real records is NOT evidence of absence — say so
        // structurally, with proof of what was searched, so completion
        // verifiers can never conclude "not installed" from a matcher miss.
        let status = if !matches.is_empty() {
            "ok"
        } else if scan.total_records > 0 {
            "no_match_for_query"
        } else {
            "not_found"
        };
        let mut envelope = serde_json::json!({
            "status": status,
            "surface": surface,
            "id": id,
            "query": query,
            "matches": matches,
            "records_searched": scan.total_records,
            "surfaces_searched": scan.surfaces,
            "unavailable_surfaces": scan.unavailable,
            "diagnostics": checks,
        });
        if status == "no_match_for_query" {
            if let Some(object) = envelope.as_object_mut() {
                object.insert(
                    "available_records_sample".to_string(),
                    serde_json::json!(scan.sample_ids),
                );
                object.insert(
                    "guidance".to_string(),
                    serde_json::json!(
                        "No record matched this query, but integration records DO exist (see available_records_sample and records_searched). A query miss is not evidence the capability is absent. Resolve by exact id from the sample, or run resource_rw kind=integration op=list for the full inventory."
                    ),
                );
            }
        }
        Ok(serde_json::to_string_pretty(&envelope)?)
    }
}

/// Records what an inspect pass actually examined, so a no-match result can
/// say "searched N records across these surfaces; none matched this query"
/// instead of an absence-shaped not_found that completion verifiers read as
/// proof the capability does not exist.
#[derive(Default)]
pub(in crate::runtime) struct IntegrationInspectScan {
    pub total_records: usize,
    pub surfaces: Vec<serde_json::Value>,
    pub sample_ids: Vec<String>,
    pub unavailable: Vec<serde_json::Value>,
}

impl IntegrationInspectScan {
    fn note(&mut self, surface: &str, records: &[serde_json::Value]) {
        self.total_records += records.len();
        self.surfaces.push(serde_json::json!({
            "surface": surface,
            "records": records.len(),
        }));
        for record in records.iter().take(5) {
            if self.sample_ids.len() >= 12 {
                break;
            }
            let identity = record
                .get("id")
                .and_then(|value| value.as_str())
                .or_else(|| record.get("name").and_then(|value| value.as_str()))
                .or_else(|| {
                    record
                        .get("manifest")
                        .and_then(|manifest| manifest.get("id"))
                        .and_then(|value| value.as_str())
                });
            if let Some(identity) = identity {
                self.sample_ids.push(format!("{surface}:{identity}"));
            }
        }
    }

    fn note_unavailable(&mut self, surface: &str, error: Option<&str>) {
        self.unavailable.push(serde_json::json!({
            "surface": surface,
            "error": error.unwrap_or("inventory unavailable"),
        }));
    }
}
