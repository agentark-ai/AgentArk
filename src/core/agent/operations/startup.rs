use super::*;

static AGENTARK_KNOWLEDGE_SYNC_LOCK: std::sync::OnceLock<std::sync::Arc<tokio::sync::Mutex<()>>> =
    std::sync::OnceLock::new();

fn normalize_custom_api_reconcile_ids(api_ids: Vec<String>) -> Vec<String> {
    let mut ids = api_ids
        .into_iter()
        .map(|id| id.trim().to_string())
        .filter(|id| !id.is_empty())
        .collect::<std::collections::BTreeSet<_>>()
        .into_iter()
        .collect::<Vec<_>>();
    ids.sort();
    ids
}

/// Per-api in-flight claims so concurrent credential-ready events (e.g. two
/// rapid saves, or a settings save racing an OAuth callback) run ONE probe and
/// ONE notification per integration instead of stacking duplicates.
static CUSTOM_API_RECONCILE_IN_FLIGHT: std::sync::OnceLock<
    std::sync::Mutex<std::collections::BTreeSet<String>>,
> = std::sync::OnceLock::new();

/// Claims ids not already being reconciled; released by `ReconcileClaim::drop`
/// so every exit path (including panics) frees the claim.
struct ReconcileClaim(Vec<String>);

impl ReconcileClaim {
    fn acquire(api_ids: Vec<String>) -> Self {
        let lock = CUSTOM_API_RECONCILE_IN_FLIGHT
            .get_or_init(|| std::sync::Mutex::new(std::collections::BTreeSet::new()));
        let Ok(mut in_flight) = lock.lock() else {
            return Self(api_ids);
        };
        Self(
            api_ids
                .into_iter()
                .filter(|id| in_flight.insert(id.clone()))
                .collect(),
        )
    }
}

impl Drop for ReconcileClaim {
    fn drop(&mut self) {
        if let Some(lock) = CUSTOM_API_RECONCILE_IN_FLIGHT.get() {
            if let Ok(mut in_flight) = lock.lock() {
                for id in &self.0 {
                    in_flight.remove(id);
                }
            }
        }
    }
}

async fn reconcile_custom_api_auth_ready(agent: Agent, api_ids: Vec<String>, reason: &'static str) {
    let claim = ReconcileClaim::acquire(normalize_custom_api_reconcile_ids(api_ids));
    let api_ids = claim.0.clone();
    if api_ids.is_empty() {
        return;
    }

    if let Err(error) = crate::custom_apis::sync_to_runtime(
        &agent.storage,
        &agent.config_dir,
        &agent.data_dir,
        &agent.runtime,
    )
    .await
    {
        tracing::warn!(
            "Custom API auth-ready runtime sync failed reason={}: {}",
            reason,
            error
        );
        return;
    }

    agent.refresh_action_catalog_index(reason).await;

    let views = match crate::custom_apis::list_custom_apis(
        &agent.storage,
        &agent.config_dir,
        &agent.data_dir,
    )
    .await
    {
        Ok(views) => views,
        Err(error) => {
            tracing::warn!(
                "Custom API auth-ready view load failed reason={}: {}",
                reason,
                error
            );
            return;
        }
    };
    let views_by_id = views
        .into_iter()
        .map(|view| (view.config.id.clone(), view))
        .collect::<std::collections::BTreeMap<_, _>>();

    for api_id in api_ids {
        let Some(view) = views_by_id.get(&api_id) else {
            tracing::warn!(
                "Custom API auth-ready reconcile skipped missing api_id={}",
                api_id
            );
            continue;
        };
        if !view.config.enabled {
            continue;
        }
        if !view.secret_configured
            && !matches!(
                view.config.auth_mode,
                crate::custom_apis::CustomApiAuthMode::None
            )
        {
            continue;
        }

        match crate::custom_apis::test_custom_api(
            &agent.storage,
            &agent.config_dir,
            &agent.data_dir,
            &agent.runtime,
            &api_id,
        )
        .await
        {
            Ok(result) if result.ok => {
                agent
                    .emit_notification_forced(
                        "Integration ready",
                        &format!(
                            "{} is connected, registered, and ready for agent use.",
                            view.config.name
                        ),
                        "info",
                        "custom_api",
                    )
                    .await;
            }
            Ok(result) => {
                agent
                    .emit_notification_forced(
                        "Integration registered",
                        &format!(
                            "{} is registered for agent use. {}",
                            view.config.name, result.detail
                        ),
                        "info",
                        "custom_api",
                    )
                    .await;
            }
            Err(error) => {
                agent
                    .emit_notification_forced(
                        "Integration needs attention",
                        &format!(
                            "{} credentials were saved, but the health probe did not pass: {}",
                            view.config.name, error
                        ),
                        "warning",
                        "custom_api",
                    )
                    .await;
            }
        }
    }
}

impl Agent {
    pub async fn snapshot(shared: &Arc<RwLock<Self>>) -> Self {
        shared.read().await.clone()
    }

    /// Initialize the agent with all subsystems.
    /// If `unified_key` is provided (from master password), it is used for ALL encryption.
    /// Otherwise falls back to legacy auto-generated keyfiles.
    pub async fn init(
        config_dir: &Path,
        data_dir: &Path,
        database_config: DatabaseConfig,
        unified_key: Option<Arc<crate::crypto::KeyManager>>,
    ) -> Result<Self> {
        // Initialize storage
        let storage = Storage::connect(database_config).await?;
        crate::core::runtime::config::set_global_settings_storage(storage.clone());
        let mut startup_issues = Vec::new();

        // Seed default specialist agents on first run
        if let Err(e) = storage.seed_default_agents().await {
            tracing::warn!("Failed to seed default agents: {}", e);
            startup_issues.push(StartupIssue::new(
                "specialists",
                "warning",
                "Default specialist seed failed during startup",
                e.to_string(),
            ));
        }

        // Initialize encryption - unified key (password-derived) or legacy keyfiles
        let key_manager: Arc<crate::crypto::KeyManager> = if let Some(key) = unified_key.clone() {
            tracing::info!("Using master-password-derived encryption key");
            key
        } else {
            tracing::info!("Using legacy keyfile encryption");
            Arc::new(crate::crypto::KeyManager::load_or_create(
                &data_dir.join("encryption.key"),
            )?)
        };
        crate::storage::install_storage_key_manager(key_manager.clone());
        let encrypted_storage =
            crate::storage::encrypted::EncryptedStorage::new(storage.clone(), key_manager.clone());
        tracing::info!("Encrypted storage initialized");
        let secure_config = if let Some(key) = unified_key.clone() {
            crate::core::runtime::config::SecureConfigManager::with_key_manager(config_dir, key)
        } else {
            crate::core::runtime::config::SecureConfigManager::new_with_data_dir(
                config_dir,
                Some(data_dir),
            )?
        };
        let key_lineage = secure_config.verify_or_initialize_storage_key_lineage()?;
        if key_lineage.initialized {
            tracing::info!(
                "Initialized settings storage key lineage fingerprint {}",
                key_lineage
                    .local_fingerprint
                    .chars()
                    .take(12)
                    .collect::<String>()
            );
        } else if let Some(stored) = key_lineage.stored_fingerprint.as_deref() {
            tracing::info!(
                "Verified settings storage key lineage fingerprint {}",
                stored.chars().take(12).collect::<String>()
            );
        }
        if key_lineage.mismatch {
            startup_issues.push(StartupIssue::new(
                "settings_storage",
                "high",
                "Settings storage does not match the active config key",
                key_lineage.detail.unwrap_or_else(|| {
                    "Postgres encrypted settings appear to belong to a different config volume or key lineage.".to_string()
                }),
            ));
        } else if storage
            .ensure_sensitive_payloads_encrypted(
                key_manager.as_ref(),
                &[
                    "user_profile",
                    crate::core::platform::observability::OBSERVABILITY_LOG_KEY,
                    crate::sentinel::PULSE_LOG_KEY,
                    crate::core::runtime::config::SETTINGS_CONFIG_KEY,
                    crate::core::runtime::config::SETTINGS_SECRETS_KEY,
                    crate::core::runtime::config::SETTINGS_SEARCH_KEY,
                    crate::core::runtime::config::SETTINGS_RUNTIME_KEY,
                    crate::core::runtime::config::SETTINGS_DISABLED_ACTIONS_KEY,
                    crate::core::runtime::config::SETTINGS_ACTION_REVIEWS_KEY,
                    crate::core::runtime::config::SETTINGS_REMOVED_BUNDLED_ACTIONS_KEY,
                    crate::core::runtime::config::SETTINGS_APPROVED_PERMISSIONS_KEY,
                ],
            )
            .await?
        {
            tracing::info!("Applied one-time sensitive payload encryption backfill");
        }

        // Initialize identity system
        let identity = IdentityManager::load_or_create(data_dir).await?;

        // Initialize safety engine
        let safety = Arc::new(SafetyEngine::new(config_dir)?);

        // Initialize proof system
        let proofs = Arc::new(ProofEngine::new(
            data_dir,
            identity.signing_key(),
            key_manager.clone(),
        )?);

        // Initialize action runtime
        let mut runtime = ActionRuntime::new(config_dir, data_dir).await?;
        runtime.set_safety_engine(safety.clone());

        let config_state = secure_config.load_runtime_state()?;
        if config_state.config_degraded {
            startup_issues.push(StartupIssue::new(
                "settings",
                "high",
                "Encrypted agent config could not be decrypted during startup",
                format!(
                    "{}. AgentArk started with safe defaults and blocked settings writes until the original key material is restored.",
                    config_state
                        .config_issue
                        .as_deref()
                        .unwrap_or("agent config payload is unreadable")
                ),
            ));
        }
        if config_state.secrets_degraded {
            startup_issues.push(StartupIssue::new(
                "secrets",
                "high",
                "Encrypted secrets could not be decrypted during startup",
                format!(
                    "{}. AgentArk started in recovery mode with empty runtime secrets; restore the original key material before updating secrets or integrations.",
                    config_state
                        .secrets_issue
                        .as_deref()
                        .unwrap_or("encrypted secrets payload is unreadable")
                ),
            ));
        }
        let mut config = config_state.config;

        if let Ok(stored_swarm_agents) = storage.get_swarm_agents().await {
            if !stored_swarm_agents.is_empty() {
                let fallback_swarm_provider = config.llm.clone();
                config.swarm.specialists = stored_swarm_agents
                    .iter()
                    .map(|agent| {
                        crate::core::swarm::persistence::specialist_config_from_storage_model(
                            agent,
                            &fallback_swarm_provider,
                        )
                    })
                    .collect();
            }
        }

        // Load HTTP API key from encrypted secrets
        let api_key = secure_config.get_api_key().unwrap_or(None);

        // Initialize LLM client (primary, for backward compat)
        let llm = LlmClient::new(&config.llm)?;

        // Build model pool from config
        let mut model_pool_map = std::collections::HashMap::new();
        let mut primary_model_id = String::new();
        for slot in &config.model_pool.slots {
            if !slot.enabled {
                continue;
            }
            match LlmClient::new(&slot.provider) {
                Ok(client) => {
                    if slot.role == ModelRole::Primary && primary_model_id.is_empty() {
                        primary_model_id = slot.id.clone();
                    }
                    model_pool_map.insert(slot.id.clone(), (slot.clone(), client));
                }
                Err(e) => {
                    tracing::warn!("Failed to init model slot '{}': {}", slot.id, e);
                }
            }
        }
        // If no primary found, use the first runtime-ready slot in config order.
        if primary_model_id.is_empty() {
            if let Some(first_id) = config
                .model_pool
                .slots
                .iter()
                .find(|slot| model_pool_map.contains_key(&slot.id))
                .map(|slot| slot.id.clone())
            {
                primary_model_id = first_id;
            }
        }
        tracing::info!(
            "Model pool initialized: {} slots, primary='{}'",
            model_pool_map.len(),
            primary_model_id
        );

        let embedding_client = EmbeddingClient::from_config(&config, data_dir)?.map(Arc::new);
        if let Some(client) = embedding_client.as_ref() {
            tracing::info!(
                "Embedding backend configured: {}",
                client.describe_backend()
            );
        } else {
            tracing::info!(
                "Embedding backend unavailable; durable memory and document retrieval will use lexical fallback until embeddings are configured"
            );
        }

        let persisted_model_override = storage
            .get(USER_SELECTED_MODEL_SLOT_KEY)
            .await
            .ok()
            .flatten()
            .and_then(|bytes| String::from_utf8(bytes).ok())
            .map(|s| s.trim().to_string())
            .filter(|s| !s.is_empty());
        let had_persisted_model_override = persisted_model_override.is_some();
        let user_selected_model_slot = persisted_model_override.and_then(|slot_id| {
            let ready = model_pool_map
                .get(&slot_id)
                .is_some_and(|(slot, _)| Self::provider_has_runtime_credentials(&slot.provider));
            if ready {
                Some(slot_id)
            } else {
                None
            }
        });
        if had_persisted_model_override && user_selected_model_slot.is_none() {
            let _ = storage.delete(USER_SELECTED_MODEL_SLOT_KEY).await;
        }
        if let Some(slot_id) = user_selected_model_slot.as_ref() {
            tracing::info!("Restored user-selected model slot override: {}", slot_id);
        }

        let mut app_provider_refs: Vec<&crate::core::LlmProvider> = Vec::new();
        if let Some(selected_slot_id) = user_selected_model_slot.as_ref() {
            if let Some(slot) = config
                .model_pool
                .slots
                .iter()
                .find(|slot| slot.id == *selected_slot_id && slot.enabled)
            {
                app_provider_refs.push(&slot.provider);
            }
        }
        if let Some(primary_slot) = config
            .model_pool
            .slots
            .iter()
            .find(|slot| slot.id == primary_model_id && slot.enabled)
        {
            app_provider_refs.push(&primary_slot.provider);
        }
        app_provider_refs.push(&config.llm);
        if let Some(fallback) = config.llm_fallback.as_ref() {
            app_provider_refs.push(fallback);
        }
        for slot in &config.model_pool.slots {
            if slot.enabled && slot.id != primary_model_id {
                app_provider_refs.push(&slot.provider);
            }
        }
        let app_llm_env = merge_app_llm_env_from_providers(&app_provider_refs);

        // Initialize task queue
        let tasks = Arc::new(RwLock::new(TaskQueue::new()));

        // Wire task queue into runtime so list_tasks action can access it
        runtime.set_task_queue(tasks.clone());

        // Wire storage into runtime for expense + entity operations
        runtime.set_storage(storage.clone());
        runtime.set_embedding_client(embedding_client.clone());

        // Wire active user identity (DID) for per-user features such as ArkOrbit.
        runtime.set_current_user_id(identity.did());

        // Initialize MCP registry and wire into runtime
        let mcp_registry = Arc::new(RwLock::new(crate::mcp::registry::McpRegistry::new(
            storage.clone(),
        )));
        runtime.set_mcp_registry(mcp_registry.clone());

        // Initialize plugin registry and wire into runtime
        let plugin_registry = Arc::new(RwLock::new(crate::plugins::registry::PluginRegistry::new(
            storage.clone(),
            config_dir.to_path_buf(),
            data_dir.to_path_buf(),
        )));
        runtime.set_plugin_registry(plugin_registry.clone());

        let extension_pack_registry = Arc::new(RwLock::new(
            crate::extension_packs::ExtensionPackRegistry::new(
                storage.clone(),
                config_dir.to_path_buf(),
                data_dir.to_path_buf(),
            ),
        ));
        runtime.set_extension_pack_registry(extension_pack_registry.clone());

        // Initialize action security guard (4-pillar defense)
        let action_guard = match crate::security::ActionGuard::new(
            identity.signing_key(),
            identity.did(),
            config_dir,
            data_dir,
        )
        .await
        {
            Ok(guard) => {
                tracing::info!("Action security guard initialized");
                let guard = Arc::new(guard.with_semantic_reviewer(llm.clone()));
                runtime.set_action_guard(guard.clone());
                Some(guard)
            }
            Err(e) => {
                tracing::warn!(
                    "Failed to initialize action security guard: {} - actions will load without security checks",
                    e
                );
                startup_issues.push(StartupIssue::new(
                    "action_security",
                    "high",
                    "Action security guard failed to initialize",
                    e.to_string(),
                ));
                None
            }
        };

        // Load all actions (with security guard active)
        runtime.load_all_actions().await?;

        if let Err(error) = plugin_registry
            .write()
            .await
            .sync_from_storage(&runtime)
            .await
        {
            tracing::warn!("Failed to sync plugin registry from storage: {}", error);
        }
        if let Err(error) =
            crate::custom_apis::sync_to_runtime(&storage, config_dir, data_dir, &runtime).await
        {
            tracing::warn!("Failed to sync custom APIs from storage: {}", error);
        }
        if let Err(error) = extension_pack_registry
            .write()
            .await
            .sync_from_storage()
            .await
        {
            tracing::warn!(
                "Failed to sync extension-pack registry from storage: {}",
                error
            );
        }
        if let Err(error) = extension_pack_registry
            .read()
            .await
            .sync_to_runtime(&runtime)
            .await
        {
            tracing::warn!(
                "Failed to sync extension-pack runtime actions from storage: {}",
                error
            );
        }

        // Add permission-gating safety rules for actions with unapproved dangerous permissions
        if let Some(ref guard) = action_guard {
            if let Ok(action_list) = runtime.list_actions().await {
                for action_def in &action_list {
                    let perms = crate::security::ActionGuard::permissions_from_capabilities(
                        &action_def.capabilities,
                    );
                    let unapproved = guard.check_permissions(&action_def.name, &perms).await;
                    if !unapproved.is_empty() {
                        let perm_names: Vec<String> =
                            unapproved.iter().map(|p| p.to_string()).collect();
                        safety.add_rule(crate::safety::SafetyRule {
                            name: format!("permission_gate_{}", action_def.name),
                            description: format!(
                                "Requires approval for action '{}' - unapproved permissions: {:?}",
                                action_def.name, perm_names
                            ),
                            trigger: crate::safety::RuleTrigger::Action {
                                name: action_def.name.clone(),
                            },
                            condition: None,
                            action: crate::safety::RuleAction::RequireApproval,
                            verified: true,
                        });
                        tracing::info!(
                            "Permission gate added for action '{}': {:?}",
                            action_def.name,
                            perm_names
                        );
                    }
                }
            }
        }

        // MCP servers are warmed in the background after the app starts so slow
        // stdio providers do not block overall startup.

        // Initialize orchestra for sub-agent delegation
        let orchestra = Orchestra::new(OrchestraConfig::default());

        // Initialize security guard for prompt injection/leakage protection
        let security = SecurityGuard::new(true); // Strict mode enabled

        // Load persisted user profile (encrypted at rest)
        let mut recovered_plaintext_user_profile = false;
        let mut user_profile = match encrypted_storage.get_decrypted("user_profile").await {
            Ok(Some(bytes)) => serde_json::from_slice::<UserProfile>(&bytes).unwrap_or_default(),
            Ok(None) => UserProfile::default(),
            Err(error) => match storage.get("user_profile").await {
                Ok(Some(bytes)) => match serde_json::from_slice::<UserProfile>(&bytes) {
                    Ok(profile) => {
                        tracing::warn!(
                            "Recovered plaintext user profile from storage; it will be re-encrypted"
                        );
                        recovered_plaintext_user_profile = true;
                        profile
                    }
                    Err(_) => {
                        return Err(anyhow::anyhow!(
                            "Failed to load encrypted user profile: {}",
                            error
                        ));
                    }
                },
                _ => {
                    return Err(anyhow::anyhow!(
                        "Failed to load encrypted user profile: {}",
                        error
                    ));
                }
            },
        };
        let mut user_profile_dirty = recovered_plaintext_user_profile;
        // Legacy cleanup: these fields were previously auto-extracted from chat and could be noisy.
        // Keep explicit settings fields (timezone/language/tone/email_format), and let the
        // cognitive-memory pipeline capture durable long-term memory instead.
        if user_profile.name.is_some()
            || user_profile.location.is_some()
            || user_profile.preferences.is_some()
        {
            user_profile.name = None;
            user_profile.location = None;
            user_profile.preferences = None;
            user_profile_dirty = true;
        }
        let saved_user_name = storage
            .get_user_preference("user_name", None)
            .await
            .ok()
            .flatten()
            .map(|item| item.value);
        let saved_priority_focus = storage
            .get_user_preference("assistant_priority_focus", None)
            .await
            .ok()
            .flatten()
            .map(|item| item.value);
        if !user_profile.onboarding_complete
            && Self::onboarding_profile_ready(
                &user_profile,
                saved_user_name.as_deref(),
                saved_priority_focus.as_deref(),
            )
        {
            user_profile.onboarding_complete = true;
            user_profile_dirty = true;
        }
        if user_profile_dirty {
            if let Ok(bytes) = serde_json::to_vec(&user_profile) {
                if let Err(e) = encrypted_storage
                    .set_encrypted("user_profile", &bytes)
                    .await
                {
                    tracing::warn!("Failed to persist updated user profile fields: {}", e);
                }
            }
        }

        let runtime_timezone = user_profile.timezone.as_deref();
        if let Some(timezone) = runtime_timezone {
            std::env::set_var("AGENTARK_LOG_TIMEZONE", timezone);
        }
        let llm = llm.with_runtime_timezone(runtime_timezone);
        let model_pool_map = model_pool_map
            .into_iter()
            .map(|(slot_id, (slot, client))| {
                (
                    slot_id,
                    (slot, client.with_runtime_timezone(runtime_timezone)),
                )
            })
            .collect();

        // Load persisted tasks (if any)
        if let Ok(stored_tasks) = storage.get_tasks().await {
            let mut queue = tasks.write().await;
            for t in stored_tasks {
                let id = uuid::Uuid::parse_str(&t.id).unwrap_or_else(|_| uuid::Uuid::new_v4());
                let arguments =
                    serde_json::from_str(&t.arguments).unwrap_or_else(|_| serde_json::json!({}));
                let approval = super::task::normalized_task_approval(
                    &serde_json::from_str(&t.approval).unwrap_or(super::task::TaskApproval::Auto),
                );
                let mut status =
                    serde_json::from_str(&t.status).unwrap_or(super::task::TaskStatus::Pending);
                if matches!(
                    status,
                    super::task::TaskStatus::AwaitingApproval
                        | super::task::TaskStatus::ExpiredNeedsReapproval
                ) {
                    status = super::task::TaskStatus::Pending;
                }
                if super::task::task_requires_explicit_approval(&approval)
                    && matches!(
                        status,
                        super::task::TaskStatus::Pending
                            | super::task::TaskStatus::AwaitingApproval
                    )
                {
                    status = super::task::TaskStatus::AwaitingApproval;
                }
                let created_at = chrono::DateTime::parse_from_rfc3339(&t.created_at)
                    .map(|d| d.with_timezone(&chrono::Utc))
                    .unwrap_or_else(|_| chrono::Utc::now());
                let scheduled_for = t
                    .scheduled_for
                    .as_deref()
                    .and_then(|s| chrono::DateTime::parse_from_rfc3339(s).ok())
                    .map(|d| d.with_timezone(&chrono::Utc));
                let proof_id = t
                    .proof_id
                    .as_deref()
                    .and_then(|s| uuid::Uuid::parse_str(s).ok());

                queue.add(super::task::Task {
                    id,
                    description: t.description,
                    action: t.action,
                    arguments,
                    approval,
                    capabilities: vec![],
                    status,
                    created_at,
                    scheduled_for,
                    cron: t.cron,
                    result: t.result,
                    proof_id,
                    priority: t.priority.map(|v| v as f32),
                    urgency: t.urgency.map(|v| v as f32),
                    importance: t.importance.map(|v| v as f32),
                    eisenhower_quadrant: t.eisenhower_quadrant.map(|v| v as u8),
                });
            }
        }

        // Initialize integration manager
        let integrations = Arc::new(crate::integrations::IntegrationManager::new(config_dir));

        // Configure media generation providers from saved config
        if !config.media_gen.provider_api_keys.is_empty() {
            if let Some(media_gen) = integrations.get("media_gen") {
                for (provider, api_key) in &config.media_gen.provider_api_keys {
                    if !api_key.is_empty() && api_key != "[ENCRYPTED]" {
                        let canonical_provider =
                            crate::integrations::media_gen::MediaProvider::parse(provider)
                                .map(|provider| provider.id().to_string())
                                .unwrap_or_else(|| provider.clone());
                        let base_url = config
                            .media_gen
                            .provider_base_urls
                            .get(&canonical_provider)
                            .or_else(|| config.media_gen.provider_base_urls.get(provider))
                            .cloned();
                        let mut payload = serde_json::json!({
                            "provider": canonical_provider,
                            "api_key": api_key
                        });
                        if let Some(base_url) = base_url
                            .as_deref()
                            .map(str::trim)
                            .filter(|value| !value.is_empty())
                        {
                            payload["base_url"] = serde_json::Value::String(base_url.to_string());
                        }
                        let _ = media_gen.execute("configure_provider", &payload).await;
                        tracing::info!("Configured media gen provider: {}", provider);
                    }
                }
            }
        }

        // Initialize swarm manager (always active - specialists are optional boosters)
        let swarm = match SwarmManager::new(config.swarm.clone()).await {
            Ok(manager) => {
                tracing::info!(
                    "Swarm manager initialized with {} specialists",
                    manager.config.specialists.len()
                );
                Some(manager)
            }
            Err(e) => {
                tracing::warn!("Failed to initialize swarm manager: {}", e);
                None
            }
        };

        // Restore persisted hooks/automations from storage.
        let persisted_hooks = match storage.get(HOOKS_STORAGE_KEY).await {
            Ok(Some(raw)) => match serde_json::from_slice::<Vec<crate::hooks::Hook>>(&raw) {
                Ok(hooks) => hooks,
                Err(e) => {
                    tracing::warn!("Failed to parse persisted hooks; starting empty: {}", e);
                    Vec::new()
                }
            },
            Ok(None) => Vec::new(),
            Err(e) => {
                tracing::warn!("Failed to load persisted hooks; starting empty: {}", e);
                Vec::new()
            }
        };

        let (notification_events, _) = broadcast::channel(256);

        // Sync auto-approve list from config into safety and runtime enforcement at startup.
        safety.set_auto_approved(&config.auto_approve);
        runtime.set_auto_approved_actions(&config.auto_approve);
        runtime.set_tool_args_guard_config(config.security.tool_args.clone());

        let app_registry = {
            let reg = crate::actions::app::AppRegistry::with_paths(
                config_dir.to_path_buf(),
                data_dir.to_path_buf(),
            );
            {
                let reg_for_boot = reg.clone();
                let storage_for_boot = storage.clone();
                let config_dir_for_boot = config_dir.to_path_buf();
                let data_dir_for_boot = data_dir.to_path_buf();
                let app_llm_env_for_boot = app_llm_env.clone();
                crate::spawn_logged!("src/core/agent/startup.rs:app_boot_reconcile", async move {
                    let boot_report = reg_for_boot.reconcile_on_boot().await;
                    match crate::sentinel::find_stale_app_references_in_pulse_events(
                        &storage_for_boot,
                        &boot_report.valid_app_ids,
                    )
                    .await
                    {
                        Ok(stale_report) => {
                            if !stale_report.event_ids.is_empty() {
                                if let Err(error) = storage_for_boot
                                    .delete_arkpulse_events_by_ids(&stale_report.event_ids)
                                    .await
                                {
                                    tracing::warn!(
                                        "Failed to delete stale Pulse app events during startup: {}",
                                        error
                                    );
                                }
                            }
                            let mut deleted_app_ids = boot_report.quarantined_app_ids.clone();
                            deleted_app_ids.extend(stale_report.missing_app_ids);
                            for app_id in deleted_app_ids {
                                reg_for_boot.purge_deleted_app_state(&app_id).await;
                                if let Err(error) = storage_for_boot
                                    .delete_app_notifications(&app_id, None)
                                    .await
                                {
                                    tracing::warn!(
                                        "Failed to delete stale app notifications during startup (app={}): {}",
                                        app_id,
                                        error
                                    );
                                }
                            }
                        }
                        Err(error) => tracing::warn!(
                            "Failed to reconcile stale Pulse app references during startup: {}",
                            error
                        ),
                    }
                    reg_for_boot
                        .restore_from_disk(
                            &config_dir_for_boot,
                            &data_dir_for_boot,
                            &app_llm_env_for_boot,
                        )
                        .await;
                });
            }
            reg
        };

        let agent = Self {
            _agent_id: AgentId::new(),
            storage: storage.clone(),
            encrypted_storage,
            identity,
            safety,
            proofs,
            runtime: Arc::new(runtime),
            mcp: mcp_registry,
            plugins: plugin_registry,
            extension_packs: extension_pack_registry,
            llm,
            embedding_client,
            model_pool: model_pool_map,
            execution_supervisor: super::ExecutionSupervisor::default(),
            primary_model_id,
            tasks,
            background_sessions: super::background_session::BackgroundSessionManager::new(Some(
                storage.clone(),
            ))
            .await,
            arkorbit: super::arkorbit::ArkOrbitService::with_filesystem(storage.clone(), data_dir),
            config,
            config_dir: config_dir.to_path_buf(),
            data_dir: data_dir.to_path_buf(),
            _orchestra: orchestra,
            swarm,
            task_router: super::task_router::TaskRouter::new(
                super::task_router::TaskRouterConfig::default(),
            ),
            swarm_activity: Arc::new(crate::core::swarm::SwarmActivityTracker::new(200)),
            security,
            conversation_history: Arc::new(RwLock::new(std::collections::HashMap::new())),
            integration_connect_flows: Arc::new(RwLock::new(HashMap::new())),
            pending_skill_imports: Arc::new(RwLock::new(HashMap::new())),
            pending_secret_followups: Arc::new(RwLock::new(HashMap::new())),
            pending_chat_credential_prompts: Arc::new(RwLock::new(HashMap::new())),
            user_profile: Arc::new(RwLock::new(user_profile)),
            last_trace: Arc::new(RwLock::new(ExecutionTrace::default())),
            trace_history: Arc::new(RwLock::new(Vec::new())),
            integrations,
            hooks: crate::hooks::HookManager::from_hooks(persisted_hooks),
            last_conversation_id: Arc::new(RwLock::new(None)),
            last_conversation_title: Arc::new(RwLock::new(None)),
            api_key,
            watcher_manager: super::watcher::WatcherManager::new(
                Some(data_dir),
                Some(storage.clone()),
            )
            .await,
            browser_sessions: super::browser_session::BrowserSessionManager::new(Some(
                storage.clone(),
            ))
            .await,
            last_activity: Arc::new(RwLock::new(None)),
            active_message_requests: Arc::new(AtomicUsize::new(0)),
            security_events: Arc::new(SecurityEvents::new()),
            user_selected_model_slot_id: Arc::new(std::sync::RwLock::new(user_selected_model_slot)),
            notification_events,
            live_runs: Arc::new(crate::core::LiveRunRegistry::new(Some(storage.clone()))),
            startup_issues: Arc::new(RwLock::new(startup_issues)),
            app_registry,
        };

        agent.spawn_gepa_idle_worker();
        agent.spawn_curator_idle_worker();
        {
            let agent_for_memory_backfill = agent.clone();
            crate::spawn_logged!(
                "src/core/agent/startup.rs:memory_capture_backfill",
                async move {
                    agent_for_memory_backfill
                        .backfill_recent_user_memory_capture_candidates()
                        .await;
                    agent_for_memory_backfill.kick_deferred_user_memory_capture_processing();
                }
            );
        }

        {
            let agent_for_approval_repair = agent.clone();
            crate::spawn_logged!("src/core/agent/startup.rs:approval_repair", async move {
                if let Err(error) = agent_for_approval_repair
                    .repair_unrecoverable_approval_tasks()
                    .await
                {
                    tracing::warn!(
                        "Failed to repair unrecoverable approval tasks during startup: {}",
                        error
                    );
                }
            });
        }

        {
            let agent_for_catalog = agent.clone();
            crate::spawn_logged!(
                "src/core/agent/startup.rs:action_catalog_warmup",
                async move {
                    match agent_for_catalog.load_action_catalog_actions().await {
                        Ok(actions) => {
                            agent_for_catalog.spawn_action_catalog_index_sync(actions, "startup")
                        }
                        Err(error) => {
                            tracing::warn!(
                                "Failed to load actions for catalog index sync: {}",
                                error
                            );
                            agent_for_catalog
                                .push_startup_issue(StartupIssue::new(
                                    "action_catalog_index",
                                    "warning",
                                    "Action catalog semantic index sync could not start",
                                    error.to_string(),
                                ))
                                .await;
                        }
                    }
                }
            );
        }

        {
            let agent_for_agentark_knowledge = agent.clone();
            crate::spawn_logged!(
                "src/core/agent/startup.rs:agentark_knowledge_sync",
                async move {
                    match agent_for_agentark_knowledge.sync_agentark_knowledge().await {
                        Ok(count) => {
                            tracing::info!("Synced {} AgentArk knowledge item(s)", count)
                        }
                        Err(error) => {
                            tracing::warn!("Failed to sync AgentArk knowledge: {}", error);
                            agent_for_agentark_knowledge
                                .push_startup_issue(StartupIssue::new(
                                    "agentark_knowledge",
                                    "warning",
                                    "AgentArk knowledge sync failed during startup",
                                    error.to_string(),
                                ))
                                .await;
                        }
                    }
                }
            );
        }

        {
            // ArkOrbit slice 3: walk <data_dir>/arkorbit/ vs the orbits
            // table at boot, logging any drift. Read-only — never deletes
            // anything behind the user's back.
            let arkorbit_for_reconcile = agent.arkorbit.clone();
            crate::spawn_logged!("src/core/agent/startup.rs:arkorbit_reconcile", async move {
                if let Err(error) = arkorbit_for_reconcile.reconcile_filesystem().await {
                    tracing::warn!("ArkOrbit filesystem reconcile failed at startup: {}", error);
                }
            });
        }

        Ok(agent)
    }

    pub(super) async fn push_startup_issue(&self, issue: StartupIssue) {
        let mut issues = self.startup_issues.write().await;
        if issues.len() >= 64 {
            issues.remove(0);
        }
        issues.push(issue);
    }

    pub fn startup_issues_handle(&self) -> Arc<RwLock<Vec<StartupIssue>>> {
        Arc::clone(&self.startup_issues)
    }

    pub(super) async fn load_action_catalog_actions(
        &self,
    ) -> Result<Vec<crate::actions::ActionDef>> {
        let mut actions = self.runtime.list_enabled_actions().await?;
        self.append_dynamic_integration_actions(&mut actions).await;
        let calendar_available = self.calendar_integration_is_configured();
        let gmail_available = self.legacy_gmail_notification_is_configured()
            || self.workspace_gmail_notification_is_configured();
        let google_workspace_granted_bundles = if self.integrations.is_enabled("google_workspace") {
            crate::actions::google_workspace::granted_bundles(&self.config_dir).unwrap_or_default()
        } else {
            Vec::new()
        };
        Self::retain_actions_for_connected_integrations(
            &mut actions,
            calendar_available,
            gmail_available,
            &google_workspace_granted_bundles,
        );
        Ok(actions)
    }

    pub(super) fn spawn_action_catalog_index_sync(
        &self,
        actions: Vec<crate::actions::ActionDef>,
        reason: &'static str,
    ) {
        if actions.is_empty() {
            return;
        }
        if ACTION_CATALOG_SYNC_ACTIVE
            .compare_exchange(false, true, Ordering::AcqRel, Ordering::Acquire)
            .is_err()
        {
            tracing::debug!(
                "Action catalog index sync already active; skipped {} refresh",
                reason
            );
            return;
        }

        let agent = self.clone();
        crate::spawn_logged!("src/core/agent.rs:action_catalog_index_sync", async move {
            let result = agent.sync_action_catalog_index(&actions).await;
            ACTION_CATALOG_SYNC_ACTIVE.store(false, Ordering::Release);
            match result {
                Ok(stats) => tracing::info!(
                    "Action catalog index sync complete reason={} actions={} embedded={} reused={} missing_embeddings={} disabled={} embedding_failures={}",
                    reason,
                    stats.actions_seen,
                    stats.embedded,
                    stats.reused_embeddings,
                    stats.missing_embeddings,
                    stats.stale_disabled,
                    stats.embedding_failures
                ),
                Err(error) => {
                    tracing::warn!(
                        "Action catalog index sync failed reason={}: {}",
                        reason,
                        error
                    );
                    agent
                        .push_startup_issue(StartupIssue::new(
                            "action_catalog_index",
                            "warning",
                            "Action catalog semantic index sync failed",
                            error.to_string(),
                        ))
                        .await;
                }
            }
        });
    }

    pub async fn refresh_action_catalog_index(&self, reason: &'static str) {
        self.invalidate_spine_tool_directory(reason).await;
        match self.load_action_catalog_actions().await {
            Ok(actions) => {
                self.spawn_action_catalog_index_sync(actions, reason);
                self.spawn_agentark_knowledge_sync(reason);
            }
            Err(error) => tracing::warn!(
                "Failed to load actions for catalog index refresh reason={}: {}",
                reason,
                error
            ),
        }
    }

    pub(crate) fn spawn_custom_api_auth_ready_reconcile(
        &self,
        api_ids: Vec<String>,
        reason: &'static str,
    ) {
        let api_ids = normalize_custom_api_reconcile_ids(api_ids);
        if api_ids.is_empty() {
            return;
        }
        let agent = self.clone();
        crate::spawn_logged!(
            "src/core/agent/operations/startup.rs:custom_api_auth_ready_reconcile",
            async move {
                reconcile_custom_api_auth_ready(agent, api_ids, reason).await;
            }
        );
    }

    pub(crate) fn spawn_custom_api_auth_profile_ready_reconcile(
        &self,
        auth_profile_id: String,
        reason: &'static str,
    ) {
        let auth_profile_id = auth_profile_id.trim().to_string();
        if auth_profile_id.is_empty() {
            return;
        }
        let agent = self.clone();
        crate::spawn_logged!(
            "src/core/agent/operations/startup.rs:custom_api_auth_profile_ready_reconcile",
            async move {
                let api_ids = match crate::custom_apis::custom_api_ids_for_auth_profile(
                    &agent.storage,
                    &auth_profile_id,
                )
                .await
                {
                    Ok(api_ids) => api_ids,
                    Err(error) => {
                        tracing::warn!(
                            "Custom API auth-profile reconcile lookup failed profile_id={}: {}",
                            auth_profile_id,
                            error
                        );
                        return;
                    }
                };
                reconcile_custom_api_auth_ready(agent, api_ids, reason).await;
            }
        );
    }

    pub(super) fn spawn_agentark_knowledge_sync(&self, reason: &'static str) {
        let agent = self.clone();
        crate::spawn_logged!(
            "src/core/agent/startup.rs:agentark_knowledge_refresh",
            async move {
                match agent.sync_agentark_knowledge().await {
                    Ok(count) => tracing::info!(
                        "AgentArk knowledge sync complete reason={} items={}",
                        reason,
                        count
                    ),
                    Err(error) => {
                        tracing::warn!(
                            "AgentArk knowledge sync failed reason={}: {}",
                            reason,
                            error
                        );
                        agent
                            .push_startup_issue(StartupIssue::new(
                                "agentark_knowledge",
                                "warning",
                                "AgentArk knowledge sync failed",
                                error.to_string(),
                            ))
                            .await;
                    }
                }
            }
        );
    }

    pub(super) async fn sync_action_catalog_index(
        &self,
        actions: &[crate::actions::ActionDef],
    ) -> Result<ActionCatalogSyncStats> {
        let descriptors = actions
            .iter()
            .map(build_action_catalog_descriptor)
            .collect::<Vec<_>>();
        let action_names = descriptors
            .iter()
            .map(|descriptor| descriptor.action_name.clone())
            .collect::<Vec<_>>();
        let existing = self
            .storage
            .action_catalog_index_entries(&action_names)
            .await?;
        let mut stats = ActionCatalogSyncStats {
            actions_seen: descriptors.len(),
            ..Default::default()
        };
        let mut embeddings_by_action: HashMap<String, Option<PgVector>> = HashMap::new();
        let mut embed_inputs = Vec::new();

        for descriptor in &descriptors {
            let existing_row = existing.get(&descriptor.action_name);
            if action_catalog_entry_needs_embedding(descriptor, existing_row) {
                embed_inputs.push((
                    descriptor.action_name.clone(),
                    descriptor.descriptor_text.clone(),
                ));
            } else if let Some(embedding) = existing_row
                .and_then(|row| row.embedding.clone())
                .filter(action_catalog_embedding_has_default_dim)
            {
                stats.reused_embeddings += 1;
                embeddings_by_action.insert(descriptor.action_name.clone(), Some(embedding));
            }
        }

        if !embed_inputs.is_empty() {
            if let Some(embedder) = self.embedding_client.as_deref() {
                let texts = embed_inputs
                    .iter()
                    .map(|(_, text)| text.clone())
                    .collect::<Vec<_>>();
                match embedder.embed_texts(&texts).await {
                    Ok(embeddings) if embeddings.len() == embed_inputs.len() => {
                        for ((action_name, _), embedding) in
                            embed_inputs.into_iter().zip(embeddings.into_iter())
                        {
                            if action_catalog_embedding_has_default_dim(&embedding) {
                                stats.embedded += 1;
                                embeddings_by_action.insert(action_name, Some(embedding));
                            } else {
                                stats.embedding_failures += 1;
                                stats.missing_embeddings += 1;
                                embeddings_by_action.insert(action_name, None);
                            }
                        }
                    }
                    Ok(embeddings) => {
                        tracing::warn!(
                            "Action catalog embedding batch returned {} vectors for {} descriptors",
                            embeddings.len(),
                            embed_inputs.len()
                        );
                        stats.embedding_failures += embed_inputs.len();
                        stats.missing_embeddings += embed_inputs.len();
                        for (action_name, _) in embed_inputs {
                            embeddings_by_action.insert(action_name, None);
                        }
                    }
                    Err(error) => {
                        tracing::warn!("Action catalog embedding batch failed: {}", error);
                        stats.embedding_failures += embed_inputs.len();
                        stats.missing_embeddings += embed_inputs.len();
                        for (action_name, _) in embed_inputs {
                            embeddings_by_action.insert(action_name, None);
                        }
                    }
                }
            } else {
                stats.missing_embeddings += embed_inputs.len();
                for (action_name, _) in embed_inputs {
                    embeddings_by_action.insert(action_name, None);
                }
            }
        }

        for descriptor in descriptors {
            let embedding = embeddings_by_action
                .remove(&descriptor.action_name)
                .flatten();
            self.storage
                .upsert_action_catalog_index_entry(&crate::storage::ActionCatalogIndexEntry {
                    action_name: descriptor.action_name,
                    source: descriptor.source,
                    version: descriptor.version,
                    descriptor_hash: descriptor.descriptor_hash,
                    descriptor_text: descriptor.descriptor_text,
                    enabled: true,
                    metadata_json: descriptor.metadata_json,
                    embedding,
                })
                .await?;
        }

        stats.stale_disabled = self
            .storage
            .mark_unavailable_action_catalog_entries_disabled(&action_names)
            .await?;
        Ok(stats)
    }

    pub async fn sync_agentark_knowledge(&self) -> Result<usize> {
        let sync_lock = AGENTARK_KNOWLEDGE_SYNC_LOCK
            .get_or_init(|| std::sync::Arc::new(tokio::sync::Mutex::new(())))
            .clone();
        let _sync_guard = sync_lock.lock().await;
        let actions = self.load_action_catalog_actions().await?;
        let items =
            crate::core::knowledge::agentark_knowledge::build_seed_knowledge_items(&actions);
        let documents =
            crate::core::knowledge::agentark_knowledge::build_seed_agentark_knowledge_documents(
                &actions,
            );

        self.storage
            .delete_knowledge_items_by_source(
                crate::core::knowledge::agentark_knowledge::CURATED_SOURCE,
            )
            .await?;
        self.storage
            .delete_knowledge_items_by_source(
                crate::core::knowledge::agentark_knowledge::RUNTIME_SOURCE,
            )
            .await?;

        let mut inserted = 0usize;
        for item in items {
            self.storage
                .create_knowledge_item(
                    &item.title,
                    &item.content,
                    Some(item.source),
                    item.url.as_deref(),
                    item.tags.as_deref(),
                    None,
                )
                .await?;
            inserted += 1;
        }

        let now = chrono::Utc::now().to_rfc3339();
        let mut document_rows = Vec::with_capacity(documents.len());
        let mut embedded_chunks = 0usize;
        let mut missing_embeddings = 0usize;
        for document in documents {
            let doc = crate::storage::entities::document::Model {
                id: document.id.clone(),
                filename: document.filename.clone(),
                content_type: document.content_type.to_string(),
                project_id: None,
                chunk_count: document.chunks.len().min(i32::MAX as usize) as i32,
                file_size: document.content.len().min(i64::MAX as usize) as i64,
                created_at: now.clone(),
            };
            let mut chunks = document
                .chunks
                .iter()
                .enumerate()
                .map(
                    |(index, content)| crate::storage::entities::document_chunk::Model {
                        id: format!("{}:chunk:{}", document.id, index),
                        document_id: document.id.clone(),
                        chunk_index: index.min(i32::MAX as usize) as i32,
                        content: content.clone(),
                        embedding: None,
                    },
                )
                .collect::<Vec<_>>();
            match crate::core::knowledge::document_search::embed_document_chunks(
                self.embedding_client.as_deref(),
                &document.filename,
                document.content_type,
                None,
                &mut chunks,
            )
            .await
            {
                Ok(count) => {
                    embedded_chunks += count;
                    missing_embeddings += chunks.len().saturating_sub(count);
                }
                Err(error) => {
                    missing_embeddings += chunks.len();
                    tracing::warn!(
                        title = document.title.as_str(),
                        "AgentArk knowledge document embedding failed: {}",
                        error
                    );
                }
            }
            document_rows.push((doc, chunks));
        }
        let document_count = document_rows.len();
        let chunk_count = document_rows
            .iter()
            .map(|(_, chunks)| chunks.len())
            .sum::<usize>();
        self.storage
            .replace_documents_by_id_prefix(
                crate::core::knowledge::agentark_knowledge::DOCUMENT_ID_PREFIX,
                &document_rows,
            )
            .await?;
        tracing::info!(
            "Synced AgentArk knowledge document index docs={} chunks={} embedded={} missing_embeddings={}",
            document_count,
            chunk_count,
            embedded_chunks,
            missing_embeddings
        );

        Ok(inserted)
    }
}
