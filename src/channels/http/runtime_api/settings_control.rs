use super::*;
use std::sync::atomic::{AtomicBool, Ordering};

const SETTINGS_EMBEDDINGS_HEALTH_TIMEOUT_MS: u64 = 250;
const SETTINGS_PROCESS_RESTART_IDLE_GRACE_SECS: u64 = 2;
const SETTINGS_PROCESS_RESTART_MAX_DEFER_SECS: u64 = 600;
const SETTINGS_PROCESS_RESTART_POLL_MS: u64 = 500;
static SETTINGS_PROCESS_RESTART_PENDING: AtomicBool = AtomicBool::new(false);

async fn active_chat_stream_count_for_restart(state: &AppState) -> usize {
    let conversation_streams = state.chat_conversation_cancellations.read().await.len();
    let task_streams = state.chat_task_cancellations.read().await.len();
    conversation_streams.saturating_add(task_streams)
}

fn schedule_process_restart_after_chat_idle(state: AppState, reason: &'static str) -> bool {
    if SETTINGS_PROCESS_RESTART_PENDING
        .compare_exchange(false, true, Ordering::AcqRel, Ordering::Acquire)
        .is_err()
    {
        return false;
    }

    crate::spawn_logged!(
        "src/channels/http/settings_control.rs:deferred_restart",
        async move {
            let started = std::time::Instant::now();
            let max_defer = std::time::Duration::from_secs(SETTINGS_PROCESS_RESTART_MAX_DEFER_SECS);
            let poll_interval = std::time::Duration::from_millis(SETTINGS_PROCESS_RESTART_POLL_MS);
            let idle_grace =
                std::time::Duration::from_secs(SETTINGS_PROCESS_RESTART_IDLE_GRACE_SECS);
            let mut logged_wait = false;

            loop {
                let active = active_chat_stream_count_for_restart(&state).await;
                if active == 0 {
                    tokio::time::sleep(idle_grace).await;
                    if active_chat_stream_count_for_restart(&state).await == 0 {
                        break;
                    }
                    continue;
                }
                if !logged_wait {
                    tracing::info!(
                        active_chat_streams = active,
                        reason,
                        "Process restart is waiting for active chat streams to finish"
                    );
                    logged_wait = true;
                }
                if started.elapsed() >= max_defer {
                    tracing::warn!(
                        active_chat_streams = active,
                        reason,
                        "Process restart defer window elapsed; restarting with active stream(s)"
                    );
                    break;
                }
                tokio::time::sleep(poll_interval).await;
            }

            tracing::info!(reason, "Restarting process to apply settings changes");
            std::process::exit(0);
            #[allow(unreachable_code)]
            Ok::<(), anyhow::Error>(())
        }
    );
    true
}

/// Get current settings
pub(super) async fn get_settings(State(state): State<AppState>) -> Json<SettingsResponse> {
    let (config, storage, config_dir, data_dir, embedding_client, gmail_enabled, workspace_enabled) = {
        let agent = state.agent.read().await;
        (
            agent.config.clone(),
            agent.storage.clone(),
            agent.config_dir.clone(),
            agent.data_dir.clone(),
            agent.embedding_client.clone(),
            agent.integrations.is_enabled("gmail"),
            agent.integrations.is_enabled("google_workspace"),
        )
    };
    let profile = state.user_profile.read().await;
    let daily_brief_task = {
        let tasks = state.tasks.read().await;
        tasks
            .all()
            .iter()
            .find(|task| task.action == "daily_brief")
            .cloned()
    };
    let daily_brief_channel = match storage.get(DAILY_BRIEF_CHANNEL_KEY).await {
        Ok(Some(bytes)) => String::from_utf8(bytes).unwrap_or("telegram".to_string()),
        _ => "telegram".to_string(),
    };
    let stored_daily_brief_enabled =
        parse_bool_pref(storage.get(DAILY_BRIEF_ENABLED_KEY).await.ok().flatten());
    let arkreflect_daily_digest_enabled = parse_bool_pref(
        storage
            .get(ARKREFLECT_DAILY_DIGEST_ENABLED_KEY)
            .await
            .ok()
            .flatten(),
    );
    let stored_daily_brief_time = storage
        .get(DAILY_BRIEF_TIME_KEY)
        .await
        .ok()
        .flatten()
        .and_then(|bytes| String::from_utf8(bytes).ok())
        .and_then(|value| normalize_daily_brief_time(&value));
    let daily_brief_time = stored_daily_brief_time
        .or_else(|| {
            daily_brief_task
                .as_ref()
                .and_then(|task| task.cron.as_deref())
                .and_then(daily_brief_time_from_cron)
        })
        .unwrap_or(DEFAULT_DAILY_BRIEF_TIME.to_string());
    let daily_brief_enabled = stored_daily_brief_enabled || daily_brief_task.is_some();
    let data_lifecycle = load_data_lifecycle_settings(&storage).await;
    let mut search_cfg = Some(
        crate::runtime::load_persisted_search_config_async(
            config_dir.clone(),
            Some(data_dir.clone()),
        )
        .await,
    );
    if let Some(cfg) = search_cfg.as_mut() {
        cfg.ensure_default_chain();
    }
    let embeddings_cfg = config.embeddings_config();
    let embeddings_provider = match embeddings_cfg.provider {
        EmbeddingsProviderKind::Disabled => "disabled",
        EmbeddingsProviderKind::LocalHf => "local-hf",
        EmbeddingsProviderKind::Ollama => "ollama",
        EmbeddingsProviderKind::OpenaiCompatible => "openai-compatible",
    }
    .to_string();
    let embeddings_model = embeddings_cfg.model.clone();
    let embeddings_base_url = embeddings_cfg.base_url.clone();
    let embeddings_has_api_key =
        !embeddings_cfg.api_key.is_empty() && embeddings_cfg.api_key != "[ENCRYPTED]";
    let embeddings_status = if let Some(client) = embedding_client.as_ref() {
        match tokio::time::timeout(
            std::time::Duration::from_millis(SETTINGS_EMBEDDINGS_HEALTH_TIMEOUT_MS),
            client.health_check(),
        )
        .await
        {
            Ok(Ok(message)) => message,
            Ok(Err(error)) => error.to_string(),
            Err(_) => match embeddings_cfg.provider {
                EmbeddingsProviderKind::Disabled => {
                    "Embeddings are disabled; retrieval uses lexical fallback.".to_string()
                }
                EmbeddingsProviderKind::LocalHf => format!(
                    "Local embeddings sidecar is configured for {} and is still responding.",
                    embeddings_model
                ),
                EmbeddingsProviderKind::Ollama => {
                    "Ollama embeddings are configured and are still responding.".to_string()
                }
                EmbeddingsProviderKind::OpenaiCompatible => {
                    "External embeddings are configured.".to_string()
                }
            },
        }
    } else {
        match embeddings_cfg.provider {
            EmbeddingsProviderKind::Disabled => {
                "Embeddings are disabled; retrieval uses lexical fallback.".to_string()
            }
            EmbeddingsProviderKind::LocalHf => format!(
                "Local embeddings sidecar is configured for {} and initializes on first dense retrieval use",
                embeddings_model
            ),
            EmbeddingsProviderKind::Ollama => {
                "No Ollama embeddings URL is configured yet.".to_string()
            }
            EmbeddingsProviderKind::OpenaiCompatible => {
                if let Some(base_url) = embeddings_base_url.as_deref() {
                    format!("External embeddings are configured at {}.", base_url)
                } else {
                    "External embeddings will use OpenAI's default /embeddings endpoint."
                        .to_string()
                }
            }
        }
    };

    // Primary LLM - has_key is true if a real api_key is set (not the placeholder)
    let (provider, model, base_url, has_key) = match &config.llm {
        LlmProvider::Ollama { base_url, model } => (
            "ollama".to_string(),
            model.clone(),
            Some(base_url.clone()),
            false,
        ),
        LlmProvider::Anthropic { api_key, model } => (
            "anthropic".to_string(),
            model.clone(),
            None,
            !api_key.is_empty() && api_key != "[ENCRYPTED]",
        ),
        LlmProvider::OpenAI {
            api_key,
            model,
            base_url,
        } => {
            let provider = openai_provider_label(base_url.as_deref());
            let display_base_url = display_openai_base_url(base_url.as_ref());
            (
                provider.to_string(),
                model.clone(),
                display_base_url,
                !api_key.is_empty() && api_key != "[ENCRYPTED]",
            )
        }
    };

    // Fallback LLM
    let (fallback_provider, fallback_model, fallback_base_url, has_fallback_key) =
        match &config.llm_fallback {
            Some(LlmProvider::Ollama { base_url, model }) => (
                Some("ollama".to_string()),
                Some(model.clone()),
                Some(base_url.clone()),
                false,
            ),
            Some(LlmProvider::Anthropic { api_key, model }) => (
                Some("anthropic".to_string()),
                Some(model.clone()),
                None,
                !api_key.is_empty() && api_key != "[ENCRYPTED]",
            ),
            Some(LlmProvider::OpenAI {
                api_key,
                model,
                base_url,
            }) => {
                let provider = openai_provider_label(base_url.as_deref());
                let display_base_url = display_openai_base_url(base_url.as_ref());
                (
                    Some(provider.to_string()),
                    Some(model.clone()),
                    display_base_url,
                    !api_key.is_empty() && api_key != "[ENCRYPTED]",
                )
            }
            None => (None, None, None, false),
        };

    let (
        slack_enabled,
        has_slack_bot_token,
        has_slack_signing_secret,
        slack_api_base_url,
        slack_default_channel_id,
        slack_default_thread_ts,
        slack_workspace_id,
        slack_workspace_name,
        slack_delivery_ready,
    ) = match &config.slack {
        Some(slack) => (
            true,
            is_configured_secret(&slack.bot_token),
            is_configured_secret(&slack.signing_secret),
            slack.api_base_url.clone(),
            slack.default_channel_id.clone(),
            slack.default_thread_ts.clone(),
            slack.workspace_id.clone(),
            slack.workspace_name.clone(),
            is_configured_secret(&slack.bot_token) && is_configured_secret(&slack.signing_secret),
        ),
        None => (
            false,
            false,
            false,
            "https://slack.com/api".to_string(),
            String::new(),
            None,
            None,
            None,
            false,
        ),
    };

    let (
        discord_enabled,
        has_discord_bot_token,
        discord_api_base_url,
        discord_default_channel_id,
        discord_default_thread_id,
        discord_guild_id,
        discord_application_id,
        discord_webhook_url,
        discord_delivery_ready,
    ) = match &config.discord {
        Some(discord) => {
            let has_bot_token = is_configured_secret(&discord.bot_token);
            let has_scope = discord
                .guild_id
                .as_deref()
                .is_some_and(|value| !value.trim().is_empty())
                || !discord.default_channel_id.trim().is_empty()
                || discord
                    .default_thread_id
                    .as_deref()
                    .is_some_and(|value| !value.trim().is_empty());
            (
                true,
                has_bot_token,
                discord.api_base_url.clone(),
                discord.default_channel_id.clone(),
                discord.default_thread_id.clone(),
                discord.guild_id.clone(),
                discord.application_id.clone(),
                discord.webhook_url.clone(),
                has_bot_token && has_scope,
            )
        }
        None => (
            false,
            false,
            "https://discord.com/api".to_string(),
            String::new(),
            None,
            None,
            None,
            String::new(),
            false,
        ),
    };

    let (
        matrix_enabled,
        has_matrix_access_token,
        matrix_homeserver_url,
        matrix_user_id,
        matrix_device_id,
        matrix_account_id,
        matrix_default_room_id,
        matrix_sync_timeout_ms,
        matrix_limit,
        matrix_user_agent,
        matrix_delivery_ready,
    ) = match &config.matrix {
        Some(matrix) => (
            true,
            is_configured_secret(&matrix.access_token),
            matrix.homeserver_url.clone(),
            matrix.user_id.clone(),
            matrix.device_id.clone(),
            matrix.account_id.clone(),
            matrix.default_room_id.clone(),
            matrix.sync_timeout_ms,
            matrix.limit,
            matrix.user_agent.clone(),
            is_configured_secret(&matrix.access_token)
                && !matrix.homeserver_url.trim().is_empty()
                && !matrix.user_id.trim().is_empty(),
        ),
        None => (
            false,
            false,
            String::new(),
            String::new(),
            None,
            None,
            None,
            0,
            0,
            None,
            false,
        ),
    };

    let (
        teams_enabled,
        has_teams_access_token,
        teams_service_url,
        teams_bot_app_id,
        teams_bot_name,
        teams_tenant_id,
        teams_team_id,
        teams_channel_id,
        teams_chat_id,
        teams_graph_base_url,
        teams_delivery_mode,
        teams_timeout_secs,
        teams_user_agent,
        teams_delivery_ready,
    ) = match &config.teams {
        Some(teams) => (
            true,
            is_configured_secret(&teams.access_token),
            teams.service_url.clone(),
            teams.bot_app_id.clone(),
            teams.bot_name.clone(),
            teams.tenant_id.clone(),
            teams.team_id.clone(),
            teams.channel_id.clone(),
            teams.chat_id.clone(),
            teams.graph_base_url.clone(),
            match teams.delivery_mode {
                crate::channels::teams::TeamsDeliveryMode::Auto => "auto",
                crate::channels::teams::TeamsDeliveryMode::BotFramework => "bot_framework",
                crate::channels::teams::TeamsDeliveryMode::Graph => "graph",
            }
            .to_string(),
            teams.timeout_secs,
            teams.user_agent.clone(),
            is_configured_secret(&teams.access_token)
                && !teams.service_url.trim().is_empty()
                && teams
                    .bot_app_id
                    .as_deref()
                    .is_some_and(|value| !value.trim().is_empty()),
        ),
        None => (
            false,
            false,
            String::new(),
            None,
            None,
            None,
            None,
            None,
            None,
            Some("https://graph.microsoft.com/v1.0".to_string()),
            "auto".to_string(),
            15,
            None,
            false,
        ),
    };

    let (telegram_enabled, telegram_users, has_telegram_token, telegram_delivery_ready) =
        match &config.telegram {
            Some(tg) => (
                true,
                tg.allowed_users.clone(),
                !tg.bot_token.is_empty() && tg.bot_token != "[ENCRYPTED]",
                !tg.bot_token.is_empty()
                    && tg.bot_token != "[ENCRYPTED]"
                    && tg.allowed_users.len() == 1
                    && tg.allowed_users.first().copied().unwrap_or_default() != 0,
            ),
            None => (false, vec![], false, false),
        };

    let (
        whatsapp_enabled,
        whatsapp_mode_str,
        whatsapp_phone_id,
        whatsapp_bridge_runtime,
        whatsapp_bridge,
        whatsapp_dm,
        whatsapp_numbers,
        has_whatsapp_token,
        has_whatsapp_app_secret,
        has_whatsapp_verify_token,
        has_whatsapp_bridge_token,
        whatsapp_delivery_ready,
    ) = match &config.whatsapp {
        Some(wa) => {
            let has_token = is_configured_secret(&wa.access_token);
            let has_app_secret = is_configured_secret(&wa.app_secret);
            let has_verify_token = !wa.verify_token.trim().is_empty();
            let has_bridge_token = is_configured_secret(&wa.bridge_token);
            let bridge_runtime = wa.bridge_runtime();
            let inbound_ready = match wa.mode {
                crate::channels::whatsapp::WhatsAppMode::CloudApi => {
                    has_token && !wa.phone_number_id.trim().is_empty()
                }
                crate::channels::whatsapp::WhatsAppMode::Baileys => match bridge_runtime {
                    crate::channels::whatsapp::WhatsAppBridgeRuntime::Embedded => true,
                    crate::channels::whatsapp::WhatsAppBridgeRuntime::External => {
                        !wa.bridge_url.trim().is_empty()
                    }
                },
            };
            let mode = match wa.mode {
                crate::channels::whatsapp::WhatsAppMode::Baileys => "baileys",
                crate::channels::whatsapp::WhatsAppMode::CloudApi => "cloud_api",
            };
            let runtime = match bridge_runtime {
                crate::channels::whatsapp::WhatsAppBridgeRuntime::Embedded => "embedded",
                crate::channels::whatsapp::WhatsAppBridgeRuntime::External => "external",
            };
            (
                true,
                mode.to_string(),
                wa.phone_number_id.clone(),
                runtime.to_string(),
                wa.bridge_url.clone(),
                wa.dm_policy.clone(),
                wa.allowed_numbers.clone(),
                has_token,
                has_app_secret,
                has_verify_token,
                has_bridge_token,
                inbound_ready
                    && crate::channels::whatsapp::configured_notification_recipient(wa).is_some(),
            )
        }
        None => (
            false,
            "baileys".to_string(),
            String::new(),
            "embedded".to_string(),
            "http://127.0.0.1:8999".to_string(),
            "pairing".to_string(),
            vec![],
            false,
            false,
            false,
            false,
            false,
        ),
    };
    let (
        google_chat_enabled,
        has_google_chat_access_token,
        has_google_chat_verify_token,
        google_chat_api_base_url,
        google_chat_space,
        google_chat_thread_key,
        google_chat_app_id,
        google_chat_bot_name,
        google_chat_delivery_ready,
    ) = match &config.google_chat {
        Some(google_chat) => (
            true,
            is_configured_secret(&google_chat.access_token),
            is_configured_secret(&google_chat.verify_token),
            google_chat.api_base_url.clone(),
            google_chat.space.clone(),
            google_chat.thread_key.clone(),
            google_chat.app_id.clone(),
            google_chat.bot_name.clone(),
            is_configured_secret(&google_chat.access_token)
                && is_configured_secret(&google_chat.verify_token)
                && google_chat
                    .space
                    .as_deref()
                    .is_some_and(|value| !value.trim().is_empty()),
        ),
        None => (
            false,
            false,
            false,
            "https://chat.googleapis.com".to_string(),
            None,
            None,
            None,
            None,
            false,
        ),
    };
    let (
        signal_enabled,
        has_signal_bridge_token,
        signal_bridge_url,
        signal_default_recipient,
        signal_default_group_id,
        signal_delivery_ready,
    ) = match &config.signal {
        Some(signal) => (
            true,
            is_configured_secret(&signal.bridge_token),
            signal.bridge_url.clone(),
            signal.default_recipient.clone(),
            signal.default_group_id.clone(),
            is_configured_secret(&signal.bridge_token)
                && !signal.bridge_url.trim().is_empty()
                && (!signal.default_recipient.trim().is_empty()
                    || !signal.default_group_id.trim().is_empty()),
        ),
        None => (
            false,
            false,
            SignalChannelConfig::default().bridge_url,
            String::new(),
            String::new(),
            false,
        ),
    };
    let (
        imessage_enabled,
        has_imessage_bridge_token,
        imessage_bridge_url,
        imessage_default_chat_id,
        imessage_default_handle,
        imessage_delivery_ready,
    ) = match &config.imessage {
        Some(imessage) => (
            true,
            is_configured_secret(&imessage.bridge_token),
            imessage.bridge_url.clone(),
            imessage.default_chat_id.clone(),
            imessage.default_handle.clone(),
            is_configured_secret(&imessage.bridge_token)
                && !imessage.bridge_url.trim().is_empty()
                && (!imessage.default_chat_id.trim().is_empty()
                    || !imessage.default_handle.trim().is_empty()),
        ),
        None => (
            false,
            false,
            IMessageChannelConfig::default().bridge_url,
            String::new(),
            String::new(),
            false,
        ),
    };
    let (
        line_enabled,
        has_line_access_token,
        has_line_channel_secret,
        line_api_base_url,
        line_default_target,
        line_user_agent,
        line_delivery_ready,
    ) = match &config.line {
        Some(line) => (
            true,
            is_configured_secret(&line.channel_access_token),
            is_configured_secret(&line.channel_secret),
            line.api_base_url.clone(),
            line.default_target.clone(),
            line.user_agent.clone(),
            is_configured_secret(&line.channel_access_token)
                && is_configured_secret(&line.channel_secret)
                && line
                    .default_target
                    .as_deref()
                    .is_some_and(|value| !value.trim().is_empty()),
        ),
        None => (
            false,
            false,
            false,
            "https://api.line.me".to_string(),
            None,
            None,
            false,
        ),
    };
    let (
        wechat_enabled,
        has_wechat_bridge_token,
        wechat_bridge_url,
        wechat_default_target_id,
        wechat_delivery_ready,
    ) = match &config.wechat {
        Some(wechat) => (
            true,
            is_configured_secret(&wechat.bridge_token),
            wechat.bridge_url.clone(),
            wechat.default_target_id.clone(),
            is_configured_secret(&wechat.bridge_token)
                && !wechat.bridge_url.trim().is_empty()
                && !wechat.default_target_id.trim().is_empty(),
        ),
        None => (
            false,
            false,
            WeChatChannelConfig::default().bridge_url,
            String::new(),
            false,
        ),
    };
    let (qq_enabled, has_qq_bridge_token, qq_bridge_url, qq_default_target_id, qq_delivery_ready) =
        match &config.qq {
            Some(qq) => (
                true,
                is_configured_secret(&qq.bridge_token),
                qq.bridge_url.clone(),
                qq.default_target_id.clone(),
                is_configured_secret(&qq.bridge_token)
                    && !qq.bridge_url.trim().is_empty()
                    && !qq.default_target_id.trim().is_empty(),
            ),
            None => (
                false,
                false,
                QqChannelConfig::default().bridge_url,
                String::new(),
                false,
            ),
        };

    // Settings are complete if name is set AND at least one usable chat model is configured.
    let has_legacy_llm = settings_has_configured_legacy_llm(&config, has_key);
    let has_model_pool = !config.model_pool.slots.is_empty();
    let settings_complete =
        !config.name.trim().is_empty() && crate::core::chat_model_is_configured(&config);

    // Build model pool summary
    let model_pool_summary: Vec<ModelSlotSummary> = config
        .model_pool
        .slots
        .iter()
        .map(|slot| {
            let (prov, mdl, burl, has_key) = match &slot.provider {
                LlmProvider::Ollama { base_url, model } => (
                    "ollama".to_string(),
                    model.clone(),
                    Some(base_url.clone()),
                    false,
                ),
                LlmProvider::Anthropic { api_key, model } => (
                    "anthropic".to_string(),
                    model.clone(),
                    None,
                    !api_key.is_empty() && api_key != "[ENCRYPTED]",
                ),
                LlmProvider::OpenAI {
                    api_key,
                    model,
                    base_url,
                } => {
                    let p = openai_provider_label(base_url.as_deref());
                    (
                        p.to_string(),
                        model.clone(),
                        base_url.clone(),
                        !api_key.is_empty() && api_key != "[ENCRYPTED]",
                    )
                }
            };
            let role_str = match &slot.role {
                ModelRole::Primary => "primary",
                ModelRole::Fast => "fast",
                ModelRole::Code => "code",
                ModelRole::Research => "research",
                ModelRole::Fallback => "fallback",
            };
            ModelSlotSummary {
                id: slot.id.clone(),
                label: slot.label.clone(),
                role: role_str.to_string(),
                provider: prov,
                model: mdl,
                base_url: burl,
                has_api_key: has_key,
                enabled: slot.enabled,
            }
        })
        .collect();

    let (settings_llm_provider, settings_llm_model, settings_llm_base_url, settings_has_api_key) =
        if has_model_pool || has_legacy_llm {
            (provider, model, base_url, has_key)
        } else {
            (String::new(), String::new(), None, false)
        };

    Json(SettingsResponse {
        bot_name: config.name.clone(),
        personality: config.personality.clone(),
        timezone: profile.timezone.clone(),
        language: profile.language.clone(),
        tone: profile.tone.clone(),
        email_format: profile.email_format.clone(),
        email: build_email_settings_response(
            &config,
            &config_dir,
            gmail_enabled,
            workspace_enabled,
        ),
        daily_brief_enabled,
        daily_brief_time,
        daily_brief_channel,
        arkreflect_daily_digest_enabled,
        llm_provider: settings_llm_provider,
        llm_model: settings_llm_model,
        llm_base_url: settings_llm_base_url,
        has_api_key: settings_has_api_key,
        llm_fallback_provider: fallback_provider,
        llm_fallback_model: fallback_model,
        llm_fallback_base_url: fallback_base_url,
        has_fallback_api_key: has_fallback_key,
        default_model_input_mode: model_input_privacy_mode_label(
            config.model_privacy.default_model_input_mode,
        )
        .to_string(),
        current_chat_pii_policy: current_chat_pii_policy_label(
            config.model_privacy.current_chat_pii_policy,
        )
        .to_string(),
        request_scoped_sensitive_approval_enabled: config
            .model_privacy
            .request_scoped_sensitive_approval_enabled,
        model_pool: model_pool_summary,
        smart_routing: config.model_pool.smart_routing,
        embeddings_provider,
        embeddings_model,
        embeddings_base_url,
        embeddings_has_api_key,
        embeddings_status,
        telegram_enabled,
        has_telegram_token,
        telegram_delivery_ready,
        telegram_allowed_users: telegram_users,
        slack_enabled,
        has_slack_bot_token,
        has_slack_signing_secret,
        slack_api_base_url,
        slack_default_channel_id,
        slack_default_thread_ts,
        slack_workspace_id,
        slack_workspace_name,
        slack_delivery_ready,
        discord_enabled,
        has_discord_bot_token,
        discord_api_base_url,
        discord_default_channel_id,
        discord_default_thread_id,
        discord_guild_id,
        discord_application_id,
        discord_webhook_url,
        discord_delivery_ready,
        matrix_enabled,
        has_matrix_access_token,
        matrix_homeserver_url,
        matrix_user_id,
        matrix_device_id,
        matrix_account_id,
        matrix_default_room_id,
        matrix_sync_timeout_ms,
        matrix_limit,
        matrix_user_agent,
        matrix_delivery_ready,
        teams_enabled,
        has_teams_access_token,
        teams_service_url,
        teams_bot_app_id,
        teams_bot_name,
        teams_tenant_id,
        teams_team_id,
        teams_channel_id,
        teams_chat_id,
        teams_graph_base_url,
        teams_delivery_mode,
        teams_timeout_secs,
        teams_user_agent,
        teams_delivery_ready,
        whatsapp_enabled,
        whatsapp_mode: whatsapp_mode_str,
        has_whatsapp_token,
        has_whatsapp_app_secret,
        has_whatsapp_verify_token,
        has_whatsapp_bridge_token,
        whatsapp_delivery_ready,
        whatsapp_phone_number_id: whatsapp_phone_id,
        whatsapp_bridge_runtime,
        whatsapp_bridge_url: whatsapp_bridge,
        whatsapp_dm_policy: whatsapp_dm,
        whatsapp_allowed_numbers: whatsapp_numbers,
        google_chat_enabled,
        has_google_chat_access_token,
        has_google_chat_verify_token,
        google_chat_api_base_url,
        google_chat_space,
        google_chat_thread_key,
        google_chat_app_id,
        google_chat_bot_name,
        google_chat_delivery_ready,
        signal_enabled,
        has_signal_bridge_token,
        signal_bridge_url,
        signal_default_recipient,
        signal_default_group_id,
        signal_delivery_ready,
        imessage_enabled,
        has_imessage_bridge_token,
        imessage_bridge_url,
        imessage_default_chat_id,
        imessage_default_handle,
        imessage_delivery_ready,
        line_enabled,
        has_line_access_token,
        has_line_channel_secret,
        line_api_base_url,
        line_default_target,
        line_user_agent,
        line_delivery_ready,
        wechat_enabled,
        has_wechat_bridge_token,
        wechat_bridge_url,
        wechat_default_target_id,
        wechat_delivery_ready,
        qq_enabled,
        has_qq_bridge_token,
        qq_bridge_url,
        qq_default_target_id,
        qq_delivery_ready,
        auto_approve: crate::core::runtime::config::sanitize_auto_approve_actions(
            &config.auto_approve,
        ),
        search_provider_order: search_cfg
            .as_ref()
            .map(|cfg| cfg.provider_order.clone())
            .unwrap_or_default(),
        search_serper_configured: search_cfg
            .as_ref()
            .map(|cfg| cfg.serper.is_some())
            .unwrap_or(false),
        search_brave_configured: search_cfg
            .as_ref()
            .map(|cfg| cfg.brave.is_some())
            .unwrap_or(false),
        search_exa_configured: search_cfg
            .as_ref()
            .map(|cfg| cfg.exa.is_some())
            .unwrap_or(false),
        search_tavily_configured: search_cfg
            .as_ref()
            .map(|cfg| cfg.tavily.is_some())
            .unwrap_or(false),
        search_perplexity_configured: search_cfg
            .as_ref()
            .map(|cfg| cfg.perplexity.is_some())
            .unwrap_or(false),
        search_firecrawl_configured: search_cfg
            .as_ref()
            .map(|cfg| cfg.firecrawl.is_some())
            .unwrap_or(false),
        search_lightpanda_available: search_cfg
            .as_ref()
            .map(|cfg| cfg.lightpanda_available)
            .unwrap_or(false),
        search_searxng_base_url: search_cfg
            .as_ref()
            .and_then(|cfg| match cfg.searxng.as_ref() {
                Some(crate::actions::SearchBackend::Searxng { base_url }) => Some(base_url.clone()),
                _ => None,
            })
            .unwrap_or_default(),
        search_builtin_cooldown_hours: 24,
        settings_complete,
        tunnel_active: state.tunnel.read().await.active,
        deployment_mode: config.deployment_mode.as_str().to_string(),
        public_app_bind_addr: config.public_apps.bind_addr.clone(),
        public_app_base_url: config.public_apps.base_url.clone(),
        data_lifecycle,
        observability: observability::build_observability_settings_response(
            &config.observability,
            &config_dir,
            &data_dir,
        ),
    })
}

pub(super) fn google_workspace_client_source_label(source: &str) -> String {
    match source {
        "environment_google_workspace" => "Environment override".to_string(),
        "settings" => format!("Saved in {}", crate::branding::PRODUCT_NAME),
        "environment_legacy_google" => "Legacy environment override".to_string(),
        "legacy_integration" => "Legacy integration config".to_string(),
        _ => "Not configured".to_string(),
    }
}

pub(super) fn google_workspace_client_id_hint(client_id: &str) -> String {
    let trimmed = client_id.trim();
    if trimmed.len() <= 12 {
        return trimmed.to_string();
    }
    let prefix = &trimmed[..6];
    let suffix = &trimmed[trimmed.len().saturating_sub(4)..];
    format!("{}...{}", prefix, suffix)
}

pub(super) fn build_google_workspace_oauth_client_settings_response(
    config_dir: &std::path::Path,
    redirect_uri: String,
) -> GoogleWorkspaceOAuthClientSettingsResponse {
    let source = crate::actions::google_workspace::workspace_client_config_source(config_dir)
        .ok()
        .flatten()
        .unwrap_or("none")
        .to_string();
    let config = crate::actions::google_workspace::load_workspace_client_config(config_dir)
        .ok()
        .flatten();
    GoogleWorkspaceOAuthClientSettingsResponse {
        configured: config.is_some(),
        source_label: google_workspace_client_source_label(&source),
        managed_externally: source == "environment_google_workspace"
            || source == "environment_legacy_google",
        source,
        client_id_hint: config
            .as_ref()
            .map(|value| google_workspace_client_id_hint(&value.client_id)),
        secret_configured: config.is_some(),
        redirect_uri,
    }
}

pub(super) fn google_workspace_settings_redirect_uri(
    state: &AppState,
    headers: &HeaderMap,
) -> std::result::Result<String, String> {
    match oauth_redirect_uri_for_request(state, headers, None) {
        Ok(value) => Ok(value),
        Err(_) if state.deployment_mode == DeploymentMode::TrustedLocal => {
            Ok(crate::actions::google_workspace::oauth_redirect_uri().to_string())
        }
        Err(error) => Err(error),
    }
}

pub(super) async fn get_google_workspace_oauth_client_settings(
    State(state): State<AppState>,
    headers: HeaderMap,
) -> Response {
    let config_dir = { state.agent.read().await.config_dir.clone() };
    let redirect_uri = match google_workspace_settings_redirect_uri(&state, &headers) {
        Ok(value) => value,
        Err(error) => {
            return (StatusCode::BAD_REQUEST, Json(ErrorResponse { error })).into_response();
        }
    };
    Json(build_google_workspace_oauth_client_settings_response(
        &config_dir,
        redirect_uri,
    ))
    .into_response()
}

pub(super) async fn update_google_workspace_oauth_client_settings(
    State(state): State<AppState>,
    headers: HeaderMap,
    Json(request): Json<GoogleWorkspaceOAuthClientSettingsUpdate>,
) -> Response {
    let (config_dir, data_dir) = {
        let agent = state.agent.read().await;
        (agent.config_dir.clone(), agent.data_dir.clone())
    };
    let redirect_uri = match google_workspace_settings_redirect_uri(&state, &headers) {
        Ok(value) => value,
        Err(error) => {
            return (StatusCode::BAD_REQUEST, Json(ErrorResponse { error })).into_response();
        }
    };
    let manager = match crate::core::runtime::config::SecureConfigManager::new_with_data_dir(
        &config_dir,
        Some(&data_dir),
    ) {
        Ok(manager) => manager,
        Err(error) => {
            return (
                StatusCode::INTERNAL_SERVER_ERROR,
                Json(ErrorResponse {
                    error: format!("Config error: {}", error),
                }),
            )
                .into_response();
        }
    };

    if request.clear {
        if let Err(error) =
            crate::actions::google_workspace::clear_saved_workspace_client_config(&config_dir)
        {
            return (
                StatusCode::INTERNAL_SERVER_ERROR,
                Json(ErrorResponse {
                    error: format!("Failed to clear saved Google OAuth client: {}", error),
                }),
            )
                .into_response();
        }
        return (
            StatusCode::OK,
            Json(build_google_workspace_oauth_client_settings_response(
                &config_dir,
                redirect_uri.clone(),
            )),
        )
            .into_response();
    }

    let credentials_json = request
        .credentials_json
        .as_deref()
        .map(str::trim)
        .filter(|value| !value.is_empty());
    let manual_client_id = request
        .client_id
        .as_deref()
        .map(str::trim)
        .filter(|value| !value.is_empty());
    let manual_client_secret = request
        .client_secret
        .as_deref()
        .map(str::trim)
        .filter(|value| !value.is_empty());

    let next_config = if let Some(raw) = credentials_json {
        match crate::actions::google_workspace::parse_credentials_json(raw) {
            Ok(config) => config,
            Err(error) => {
                return (
                    StatusCode::BAD_REQUEST,
                    Json(ErrorResponse {
                        error: error.to_string(),
                    }),
                )
                    .into_response();
            }
        }
    } else if let (Some(client_id), Some(client_secret)) = (manual_client_id, manual_client_secret)
    {
        crate::actions::google_workspace::GoogleWorkspaceClientConfig {
            client_id: client_id.to_string(),
            client_secret: client_secret.to_string(),
        }
    } else {
        return (
            StatusCode::BAD_REQUEST,
            Json(ErrorResponse {
                error: "Provide a Google OAuth client JSON file, or enter both client ID and client secret."
                    .to_string(),
            }),
        )
            .into_response();
    };

    let previous_saved =
        crate::actions::google_workspace::load_saved_workspace_client_config(&config_dir)
            .ok()
            .flatten();
    let credentials_changed = previous_saved.as_ref().is_none_or(|existing| {
        existing.client_id != next_config.client_id
            || existing.client_secret != next_config.client_secret
    });

    if let Err(error) =
        crate::actions::google_workspace::save_workspace_client_config(&config_dir, &next_config)
    {
        return (
            StatusCode::INTERNAL_SERVER_ERROR,
            Json(ErrorResponse {
                error: format!("Failed to save Google OAuth client: {}", error),
            }),
        )
            .into_response();
    }

    if credentials_changed {
        for key in [
            crate::actions::google_workspace::GOOGLE_WORKSPACE_TOKENS_KEY,
            crate::actions::google_workspace::GOOGLE_WORKSPACE_PENDING_BUNDLES_KEY,
            "gmail_tokens",
            "calendar_tokens",
        ] {
            let _ = manager.set_custom_secret(key, None);
        }
        for key in [
            integrations::integration_enabled_key("google_workspace"),
            integrations::integration_enabled_key("gmail"),
            integrations::integration_enabled_key("google_calendar"),
        ] {
            let _ = manager.set_custom_secret(&key, Some("false".to_string()));
        }
    }
    super::integrations::refresh_connected_action_surfaces(
        &state,
        "prebuilt_integration_configured",
    )
    .await;

    (
        StatusCode::OK,
        Json(build_google_workspace_oauth_client_settings_response(
            &config_dir,
            redirect_uri,
        )),
    )
        .into_response()
}

/// Get media generation settings (which providers are configured)
pub(super) async fn get_media_settings(
    State(state): State<AppState>,
) -> Json<MediaSettingsResponse> {
    let (media_config, integrations) = {
        let agent = state.agent.read().await;
        (agent.config.media_gen.clone(), agent.integrations.clone())
    };

    // Check which media providers are configured (have API keys)
    let mut configured = Vec::new();
    for (provider, key) in &media_config.provider_api_keys {
        if !key.is_empty() && key != "[ENCRYPTED]" {
            configured.push(provider.clone());
        }
    }

    // Also check via integration (for runtime-configured providers)
    if let Some(media_gen) = integrations.get("media_gen") {
        match tokio::time::timeout(
            std::time::Duration::from_secs(3),
            media_gen.execute("list_providers", &serde_json::json!({})),
        )
        .await
        {
            Ok(Ok(result)) => {
                if let Some(providers) = result.get("providers").and_then(|p| p.as_array()) {
                    for p in providers {
                        if p.get("configured")
                            .and_then(|c| c.as_bool())
                            .unwrap_or(false)
                        {
                            if let Some(name) = p.get("provider").and_then(|n| n.as_str()) {
                                if !configured.contains(&name.to_string()) {
                                    configured.push(name.to_string());
                                }
                            }
                        }
                    }
                }
            }
            Ok(Err(e)) => tracing::warn!("media_gen list_providers failed: {}", e),
            Err(_) => tracing::warn!("media_gen list_providers timed out after 3s"),
        }
    }

    // Get default/fallback providers from config
    Json(MediaSettingsResponse {
        configured,
        default_image_provider: media_config.default_image_provider.clone(),
        image_model: media_config.image_model.clone(),
        fallback_image_provider: media_config.fallback_image_provider.clone(),
        default_video_provider: media_config.default_video_provider.clone(),
        fallback_video_provider: media_config.fallback_video_provider.clone(),
        provider_base_urls: media_config.provider_base_urls.clone(),
    })
}

/// Update settings
pub(super) async fn update_settings(
    State(state): State<AppState>,
    Json(settings): Json<SettingsUpdate>,
) -> Response {
    let search_provider_order = settings.search_provider_order.clone();
    let search_serper_key = settings.search_serper_key.clone();
    let clear_search_serper_key = settings.clear_search_serper_key.unwrap_or(false);
    let search_brave_key = settings.search_brave_key.clone();
    let clear_search_brave_key = settings.clear_search_brave_key.unwrap_or(false);
    let search_exa_key = settings.search_exa_key.clone();
    let clear_search_exa_key = settings.clear_search_exa_key.unwrap_or(false);
    let search_tavily_key = settings.search_tavily_key.clone();
    let clear_search_tavily_key = settings.clear_search_tavily_key.unwrap_or(false);
    let search_perplexity_key = settings.search_perplexity_key.clone();
    let clear_search_perplexity_key = settings.clear_search_perplexity_key.unwrap_or(false);
    let search_firecrawl_key = settings.search_firecrawl_key.clone();
    let clear_search_firecrawl_key = settings.clear_search_firecrawl_key.unwrap_or(false);
    let search_searxng_base_url = settings.search_searxng_base_url.clone();
    let observability_auth_token = settings
        .observability
        .as_ref()
        .and_then(|observability| observability.auth_token.clone());

    let mut needs_restart = false;
    let mut wa_start_bridge = false;
    let mut wa_stop_bridge = false;
    let mut wa_restart_bridge = false;
    let mut llm_connectivity_probe: Option<LlmProvider> = None;
    let mut media_provider_updates: Vec<(String, String, Option<String>)> = Vec::new();
    let (deferred_storage, deferred_encrypted_storage) = {
        let agent = state.agent.read().await;
        (agent.storage.clone(), agent.encrypted_storage.clone())
    };
    let mut deferred_profile_bytes: Option<Vec<u8>> = None;
    let mut deferred_runtime_timezone: Option<Option<String>> = None;
    let mut deferred_search_config_dir: Option<PathBuf> = None;
    let mut deferred_data_lifecycle_settings: Option<DataLifecycleSettings> = None;
    let existing_daily_brief_tasks = {
        let tasks = state.tasks.read().await;
        tasks
            .all()
            .iter()
            .filter(|task| task.action == "daily_brief")
            .cloned()
            .collect::<Vec<_>>()
    };
    let stored_daily_brief_enabled = parse_bool_pref(
        deferred_storage
            .get(DAILY_BRIEF_ENABLED_KEY)
            .await
            .ok()
            .flatten(),
    );
    let stored_daily_brief_time = deferred_storage
        .get(DAILY_BRIEF_TIME_KEY)
        .await
        .ok()
        .flatten()
        .and_then(|bytes| String::from_utf8(bytes).ok())
        .and_then(|value| normalize_daily_brief_time(&value))
        .or_else(|| {
            existing_daily_brief_tasks
                .first()
                .and_then(|task| task.cron.as_deref())
                .and_then(daily_brief_time_from_cron)
        })
        .unwrap_or(DEFAULT_DAILY_BRIEF_TIME.to_string());
    let stored_daily_brief_channel = deferred_storage
        .get(DAILY_BRIEF_CHANNEL_KEY)
        .await
        .ok()
        .flatten()
        .and_then(|bytes| String::from_utf8(bytes).ok())
        .unwrap_or("telegram".to_string());
    let requested_daily_brief_time = if let Some(value) = settings.daily_brief_time.as_ref() {
        let Some(normalized) = normalize_daily_brief_time(value) else {
            return (
                StatusCode::BAD_REQUEST,
                Json(ErrorResponse {
                    error: "Daily brief time must use HH:MM in 24-hour format".to_string(),
                }),
            )
                .into_response();
        };
        normalized
    } else {
        stored_daily_brief_time
    };
    let requested_daily_brief_enabled = settings
        .daily_brief_enabled
        .unwrap_or(!existing_daily_brief_tasks.is_empty() || stored_daily_brief_enabled);
    let stored_arkreflect_daily_digest_enabled = parse_bool_pref(
        deferred_storage
            .get(ARKREFLECT_DAILY_DIGEST_ENABLED_KEY)
            .await
            .ok()
            .flatten(),
    );
    let requested_arkreflect_daily_digest_enabled = settings
        .arkreflect_daily_digest_enabled
        .unwrap_or(stored_arkreflect_daily_digest_enabled);

    if let Some(timezone) = settings.timezone.as_ref() {
        if !timezone.trim().is_empty() && timezone.parse::<chrono_tz::Tz>().is_err() {
            return (
                StatusCode::BAD_REQUEST,
                Json(ErrorResponse {
                    error: "Invalid timezone. Use an IANA name like America/New_York".to_string(),
                }),
            )
                .into_response();
        }
    }

    if let Some(email) = settings.email.as_ref() {
        if let Some(to_address) = email.to_address.as_ref() {
            if !to_address.trim().is_empty() {
                if let Err(error) =
                    crate::core::connectivity::email_delivery::validate_email_address(to_address)
                {
                    return (
                        StatusCode::BAD_REQUEST,
                        Json(ErrorResponse {
                            error: error.to_string(),
                        }),
                    )
                        .into_response();
                }
            }
        }
        if let Some(from_address) = email.from_address.as_ref() {
            if !from_address.trim().is_empty() {
                if let Err(error) =
                    crate::core::connectivity::email_delivery::validate_email_address(from_address)
                {
                    return (
                        StatusCode::BAD_REQUEST,
                        Json(ErrorResponse {
                            error: error.to_string(),
                        }),
                    )
                        .into_response();
                }
            }
        }
    }

    if settings.timezone.is_some()
        || settings.language.is_some()
        || settings.tone.is_some()
        || settings.email_format.is_some()
    {
        let saved_user_name = deferred_storage
            .get_user_preference("user_name", None)
            .await
            .ok()
            .flatten()
            .map(|item| item.value);
        let saved_priority_focus = deferred_storage
            .get_user_preference("assistant_priority_focus", None)
            .await
            .ok()
            .flatten()
            .map(|item| item.value);
        let mut profile = state.user_profile.write().await;
        if let Some(timezone) = &settings.timezone {
            if timezone.trim().is_empty() {
                profile.timezone = None;
            } else {
                profile.timezone = Some(timezone.clone());
            }
        }
        if let Some(language) = &settings.language {
            profile.language = if language.trim().is_empty() {
                None
            } else {
                Some(language.clone())
            };
        }
        if let Some(tone) = &settings.tone {
            profile.tone = if tone.trim().is_empty() {
                None
            } else {
                Some(tone.clone())
            };
        }
        if let Some(email_format) = &settings.email_format {
            profile.email_format = if email_format.trim().is_empty() {
                None
            } else {
                Some(email_format.clone())
            };
        }
        if crate::core::Agent::onboarding_profile_ready(
            &profile,
            saved_user_name.as_deref(),
            saved_priority_focus.as_deref(),
        ) {
            profile.onboarding_complete = true;
        }
        deferred_runtime_timezone = Some(profile.timezone.clone());
        if let Ok(bytes) = serde_json::to_vec(&*profile) {
            deferred_profile_bytes = Some(bytes);
        }
    }

    if let Some(runtime_timezone) = deferred_runtime_timezone.as_ref() {
        if let Some(timezone) = runtime_timezone.as_deref() {
            std::env::set_var("AGENTARK_LOG_TIMEZONE", timezone);
        } else {
            std::env::remove_var("AGENTARK_LOG_TIMEZONE");
        }
        let mut agent = state.agent.write().await;
        agent.llm.set_runtime_timezone(runtime_timezone.as_deref());
        for (_, client) in agent.model_pool.values_mut() {
            client.set_runtime_timezone(runtime_timezone.as_deref());
        }
    }

    let requested_daily_brief_channel = if let Some(channel) = settings.daily_brief_channel.as_ref()
    {
        let normalized = channel.trim().to_lowercase();
        let bundled = crate::channels::messaging_registry::BUNDLED_CHANNEL_IDS
            .iter()
            .any(|candidate| candidate.eq_ignore_ascii_case(&normalized));
        let custom_or_pack = normalized
            .starts_with(crate::custom_messaging_channels::CUSTOM_CHANNEL_ID_PREFIX)
            || normalized
                .starts_with(crate::channels::messaging_registry::EXTENSION_CHANNEL_ID_PREFIX);
        if !bundled && !custom_or_pack {
            return (
                StatusCode::BAD_REQUEST,
                Json(ErrorResponse {
                    error: "Daily brief channel must be a bundled channel or a configured custom or extension-pack messaging channel"
                        .to_string(),
                }),
            )
                .into_response();
        }
        if custom_or_pack {
            let agent = state.agent.read().await;
            let config_manager =
                crate::core::runtime::config::SecureConfigManager::new_with_data_dir(
                    &agent.config_dir,
                    Some(&agent.data_dir),
                )
                .ok();
            let packs_guard = agent.extension_packs.read().await;
            let bundled_check: fn(&str) -> bool = |_| false;
            let ctx = crate::channels::messaging_registry::ChannelQueryContext {
                bundled_configured: &bundled_check,
                extension_packs: &packs_guard,
                storage: &agent.storage,
                config_dir: &agent.config_dir,
                data_dir: &agent.data_dir,
                config_manager: config_manager.as_ref(),
            };
            let ready = crate::channels::messaging_registry::MessagingChannelRegistry::new()
                .lookup(&ctx, &normalized)
                .await
                .ok()
                .flatten()
                .is_some_and(|descriptor| descriptor.configured);
            if !ready {
                return (
                    StatusCode::BAD_REQUEST,
                    Json(ErrorResponse {
                        error: "Custom daily brief channel is not ready yet".to_string(),
                    }),
                )
                    .into_response();
            }
        }
        normalized
    } else {
        stored_daily_brief_channel
    };
    let deferred_daily_brief_channel = Some(requested_daily_brief_channel.clone());
    let deferred_daily_brief_enabled = Some(requested_daily_brief_enabled);
    let deferred_daily_brief_time = Some(requested_daily_brief_time.clone());
    let deferred_arkreflect_daily_digest_enabled = Some(requested_arkreflect_daily_digest_enabled);

    if let Some(update) = settings.data_lifecycle.as_ref() {
        let mut current = load_data_lifecycle_settings(&deferred_storage).await;
        if let Some(v) = update.cleanup_enabled {
            current.cleanup_enabled = v;
        }
        if let Some(v) = update.notifications_cleanup_enabled {
            current.notifications_cleanup_enabled = v;
        }
        if let Some(v) = update.logs_cleanup_enabled {
            current.logs_cleanup_enabled = v;
        }
        if let Some(v) = update.notifications_retention_days {
            current.notifications_retention_days = v;
        }
        if let Some(v) = update.notification_cleanup_interval_secs {
            current.notification_cleanup_interval_secs = v;
        }
        if let Some(v) = update.execution_trace_retention_days {
            current.execution_trace_retention_days = v;
        }
        if let Some(v) = update.execution_proof_retention_days {
            current.execution_proof_retention_days = v;
        }
        if let Some(v) = update.operational_log_retention_days {
            current.operational_log_retention_days = v;
        }
        if let Some(v) = update.security_log_retention_days {
            current.security_log_retention_days = v;
        }
        if let Some(v) = update.approval_log_retention_days {
            current.approval_log_retention_days = v;
        }
        if let Some(v) = update.swarm_delegation_retention_days {
            current.swarm_delegation_retention_days = v;
        }
        if let Some(v) = update.llm_usage_retention_days {
            current.llm_usage_retention_days = v;
        }
        if let Some(v) = update.terminal_task_retention_days {
            current.terminal_task_retention_days = v;
        }
        if let Some(v) = update.execution_run_retention_days {
            current.execution_run_retention_days = v;
        }
        if let Some(v) = update.background_session_retention_days {
            current.background_session_retention_days = v;
        }
        if let Some(v) = update.browser_session_retention_days {
            current.browser_session_retention_days = v;
        }
        if let Some(v) = update.automation_run_retention_days {
            current.automation_run_retention_days = v;
        }
        if let Some(v) = update.message_retention_days {
            current.message_retention_days = v;
        }
        if let Some(v) = update.experience_run_retention_days {
            current.experience_run_retention_days = v;
        }
        if let Some(v) = update.experience_edge_retention_days {
            current.experience_edge_retention_days = v;
        }
        if let Some(v) = update.learning_candidate_retention_days {
            current.learning_candidate_retention_days = v;
        }
        if let Some(v) = update.experience_item_retention_days {
            current.experience_item_retention_days = v;
        }
        if let Some(v) = update.procedural_pattern_retention_days {
            current.procedural_pattern_retention_days = v;
        }
        if let Some(v) = update.recall_event_retention_days {
            current.recall_event_retention_days = v;
        }
        if let Some(v) = update.recall_test_retention_days {
            current.recall_test_retention_days = v;
        }
        if let Some(v) = update.readiness_retention_days {
            current.readiness_retention_days = v;
            current.readiness_evaluation_retention_days = v;
        }
        if let Some(v) = update.operational_memory_retention_days {
            current.operational_memory_retention_days = v;
            current.memory_capture_event_retention_days = v;
            current.memory_operation_retention_days = v;
            current.memory_evidence_link_retention_days = v;
            current.semantic_work_unit_retention_days = v;
        }
        if let Some(v) = update.readiness_evaluation_retention_days {
            current.readiness_evaluation_retention_days = v;
        }
        if let Some(v) = update.memory_capture_event_retention_days {
            current.memory_capture_event_retention_days = v;
        }
        if let Some(v) = update.memory_operation_retention_days {
            current.memory_operation_retention_days = v;
        }
        if let Some(v) = update.memory_evidence_link_retention_days {
            current.memory_evidence_link_retention_days = v;
        }
        if let Some(v) = update.semantic_work_unit_retention_days {
            current.semantic_work_unit_retention_days = v;
        }
        if let Some(v) = update.housekeeping_interval_secs {
            current.housekeeping_interval_secs = v;
        }
        if let Some(v) = update.security_cleanup_interval_days {
            current.security_cleanup_interval_days = v;
        }
        if let Some(v) = update.security_cleanup_idle_threshold_secs {
            current.security_cleanup_idle_threshold_secs = v;
        }
        deferred_data_lifecycle_settings = Some(current.normalized());
    }

    let result = {
        let mut agent_guard =
            match acquire_agent_write_for_config_mutation(&state, "saving settings").await {
                Ok(agent) => agent,
                Err(response) => return response,
            };

        // Snapshot current Telegram/WhatsApp config for change detection
        let old_telegram = agent_guard
            .config
            .telegram
            .as_ref()
            .map(|t| (t.bot_token.clone(), t.allowed_users.clone()));
        let old_whatsapp = agent_guard.config.whatsapp.clone();

        // Update bot name if provided
        if let Some(name) = &settings.bot_name {
            if !name.is_empty() {
                agent_guard.config.name = name.clone();
            }
        }

        // Update personality if provided
        if let Some(personality) = &settings.personality {
            if !personality.is_empty() {
                agent_guard.config.personality = personality.clone();
            }
        }

        if let Some(email_settings) = settings.email.as_ref() {
            let mut email = agent_guard.config.email.clone();
            if let Some(provider) = email_settings.provider.as_ref() {
                email.provider = if provider.trim().is_empty() {
                    crate::core::connectivity::email_delivery::EMAIL_PROVIDER_AUTO.to_string()
                } else {
                    crate::core::connectivity::email_delivery::normalize_email_provider(provider)
                };
            }
            if let Some(to_address) = email_settings.to_address.as_ref() {
                let trimmed = to_address.trim();
                email.to_address = if trimmed.is_empty() {
                    None
                } else {
                    Some(trimmed.to_string())
                };
            }
            if let Some(from_address) = email_settings.from_address.as_ref() {
                let trimmed = from_address.trim();
                email.from_address = if trimmed.is_empty() {
                    None
                } else {
                    Some(trimmed.to_string())
                };
            }
            if let Some(domain) = email_settings.domain.as_ref() {
                let trimmed = domain.trim();
                email.domain = if trimmed.is_empty() {
                    None
                } else {
                    Some(trimmed.to_string())
                };
            }
            if let Some(transport) = email_settings.transport.as_ref() {
                if let Some(kind) = transport.kind.as_ref() {
                    email.transport.kind =
                        crate::core::connectivity::email_delivery::normalize_transport_kind(kind);
                }
                if let Some(http) = transport.http.as_ref() {
                    if let Some(base_url) = http.base_url.as_ref() {
                        let trimmed = base_url.trim();
                        email.transport.http.base_url = if trimmed.is_empty() {
                            None
                        } else {
                            Some(trimmed.to_string())
                        };
                    }
                    if let Some(send_path) = http.send_path.as_ref() {
                        let trimmed = send_path.trim();
                        email.transport.http.send_path = if trimmed.is_empty() {
                            None
                        } else {
                            Some(trimmed.to_string())
                        };
                    }
                }
                if let Some(smtp) = transport.smtp.as_ref() {
                    if let Some(host) = smtp.host.as_ref() {
                        email.transport.smtp.host = host.trim().to_string();
                    }
                    if let Some(port) = smtp.port {
                        email.transport.smtp.port = port;
                    }
                    if let Some(security) = smtp.security.as_ref() {
                        email.transport.smtp.security = security.trim().to_string();
                    }
                }
            }
            if let Some(auth) = email_settings.auth.as_ref() {
                if let Some(kind) = auth.kind.as_ref() {
                    email.auth.kind =
                        crate::core::connectivity::email_delivery::normalize_auth_kind(kind);
                }
                if let Some(api_key) = auth.api_key.as_ref() {
                    email.auth.api_key = api_key.trim().to_string();
                }
                if let Some(header_name) = auth.header_name.as_ref() {
                    let trimmed = header_name.trim();
                    email.auth.header_name = if trimmed.is_empty() {
                        None
                    } else {
                        Some(trimmed.to_string())
                    };
                }
                if let Some(scheme) = auth.scheme.as_ref() {
                    let trimmed = scheme.trim();
                    email.auth.scheme = if trimmed.is_empty() {
                        None
                    } else {
                        Some(trimmed.to_string())
                    };
                }
                if let Some(basic_username) = auth.basic_username.as_ref() {
                    email.auth.basic_username = basic_username.trim().to_string();
                }
                if let Some(basic_password) = auth.basic_password.as_ref() {
                    email.auth.basic_password = basic_password.trim().to_string();
                }
                if let Some(aws_access_key_id) = auth.aws_access_key_id.as_ref() {
                    email.auth.aws_access_key_id = aws_access_key_id.trim().to_string();
                }
                if let Some(aws_secret_access_key) = auth.aws_secret_access_key.as_ref() {
                    email.auth.aws_secret_access_key = aws_secret_access_key.trim().to_string();
                }
                if let Some(aws_session_token) = auth.aws_session_token.as_ref() {
                    let trimmed = aws_session_token.trim();
                    email.auth.aws_session_token = if trimmed.is_empty() {
                        None
                    } else {
                        Some(trimmed.to_string())
                    };
                }
                if let Some(aws_region) = auth.aws_region.as_ref() {
                    let trimmed = aws_region.trim();
                    email.auth.aws_region = if trimmed.is_empty() {
                        None
                    } else {
                        Some(trimmed.to_string())
                    };
                }
                if let Some(aws_service) = auth.aws_service.as_ref() {
                    let trimmed = aws_service.trim();
                    email.auth.aws_service = if trimmed.is_empty() {
                        None
                    } else {
                        Some(trimmed.to_string())
                    };
                }
            }
            agent_guard.config.email = email;
        }

        // Get existing primary API key to preserve if not provided
        let existing_api_key_raw = match &agent_guard.config.llm {
            LlmProvider::Anthropic { api_key, .. } => Some(api_key.clone()),
            LlmProvider::OpenAI { api_key, .. } => Some(api_key.clone()),
            _ => None,
        };
        let mut existing_api_key = existing_api_key_raw.clone();
        let current_embeddings = agent_guard.config.embeddings_config();
        let mut existing_embeddings_api_key = if current_embeddings.api_key.is_empty()
            || current_embeddings.api_key == "[ENCRYPTED]"
        {
            None
        } else {
            Some(current_embeddings.api_key.clone())
        };

        // Get existing fallback API key to preserve if not provided
        let mut existing_fallback_api_key =
            agent_guard
                .config
                .llm_fallback
                .as_ref()
                .and_then(|fb| match fb {
                    LlmProvider::Anthropic { api_key, .. } => Some(api_key.clone()),
                    LlmProvider::OpenAI { api_key, .. } => Some(api_key.clone()),
                    _ => None,
                });
        if matches!(
            existing_api_key.as_deref(),
            None | Some("") | Some("[ENCRYPTED]")
        ) || matches!(
            existing_embeddings_api_key.as_deref(),
            None | Some("") | Some("[ENCRYPTED]")
        ) || matches!(
            existing_fallback_api_key.as_deref(),
            None | Some("") | Some("[ENCRYPTED]")
        ) {
            if let Ok(secure) = crate::core::runtime::config::SecureConfigManager::new_with_data_dir(
                &agent_guard.config_dir,
                Some(&agent_guard.data_dir),
            ) {
                if let Ok(secrets) = secure.load_secrets() {
                    if matches!(
                        existing_api_key.as_deref(),
                        None | Some("") | Some("[ENCRYPTED]")
                    ) {
                        existing_api_key = secrets.llm_api_key.clone();
                    }
                    if matches!(
                        existing_embeddings_api_key.as_deref(),
                        None | Some("") | Some("[ENCRYPTED]")
                    ) {
                        existing_embeddings_api_key = secrets.embeddings_api_key.clone();
                    }
                    if matches!(
                        existing_fallback_api_key.as_deref(),
                        None | Some("") | Some("[ENCRYPTED]")
                    ) {
                        existing_fallback_api_key = secrets.llm_fallback_api_key.clone();
                    }
                }
            }
        }

        let mut existing_telegram_token = agent_guard
            .config
            .telegram
            .as_ref()
            .map(|t| t.bot_token.clone());

        let mut existing_slack_bot_token = agent_guard
            .config
            .slack
            .as_ref()
            .map(|s| s.bot_token.clone());
        let mut existing_slack_signing_secret = agent_guard
            .config
            .slack
            .as_ref()
            .map(|s| s.signing_secret.clone());
        let mut existing_discord_bot_token = agent_guard
            .config
            .discord
            .as_ref()
            .map(|d| d.bot_token.clone());
        let mut existing_matrix_access_token = agent_guard
            .config
            .matrix
            .as_ref()
            .map(|m| m.access_token.clone());
        let mut existing_teams_access_token = agent_guard
            .config
            .teams
            .as_ref()
            .map(|t| t.access_token.clone());

        let mut existing_whatsapp_token = agent_guard
            .config
            .whatsapp
            .as_ref()
            .map(|w| w.access_token.clone());
        let mut existing_whatsapp_app_secret = agent_guard
            .config
            .whatsapp
            .as_ref()
            .map(|w| w.app_secret.clone());
        let mut existing_whatsapp_bridge_token = agent_guard
            .config
            .whatsapp
            .as_ref()
            .map(|w| w.bridge_token.clone());

        if matches!(
            existing_telegram_token.as_deref(),
            None | Some("") | Some("[ENCRYPTED]")
        ) || matches!(
            existing_slack_bot_token.as_deref(),
            None | Some("") | Some("[ENCRYPTED]")
        ) || matches!(
            existing_slack_signing_secret.as_deref(),
            None | Some("") | Some("[ENCRYPTED]")
        ) || matches!(
            existing_discord_bot_token.as_deref(),
            None | Some("") | Some("[ENCRYPTED]")
        ) || matches!(
            existing_matrix_access_token.as_deref(),
            None | Some("") | Some("[ENCRYPTED]")
        ) || matches!(
            existing_teams_access_token.as_deref(),
            None | Some("") | Some("[ENCRYPTED]")
        ) || matches!(
            existing_whatsapp_token.as_deref(),
            None | Some("") | Some("[ENCRYPTED]")
        ) || matches!(
            existing_whatsapp_app_secret.as_deref(),
            None | Some("") | Some("[ENCRYPTED]")
        ) || matches!(
            existing_whatsapp_bridge_token.as_deref(),
            None | Some("") | Some("[ENCRYPTED]")
        ) {
            if let Ok(secure) = crate::core::runtime::config::SecureConfigManager::new_with_data_dir(
                &agent_guard.config_dir,
                Some(&agent_guard.data_dir),
            ) {
                if let Ok(secrets) = secure.load_secrets() {
                    if matches!(
                        existing_telegram_token.as_deref(),
                        None | Some("") | Some("[ENCRYPTED]")
                    ) {
                        existing_telegram_token = secrets.telegram_bot_token.clone();
                    }
                    if matches!(
                        existing_slack_bot_token.as_deref(),
                        None | Some("") | Some("[ENCRYPTED]")
                    ) {
                        existing_slack_bot_token = secrets.slack_bot_token.clone();
                    }
                    if matches!(
                        existing_slack_signing_secret.as_deref(),
                        None | Some("") | Some("[ENCRYPTED]")
                    ) {
                        existing_slack_signing_secret = secrets.slack_signing_secret.clone();
                    }
                    if matches!(
                        existing_discord_bot_token.as_deref(),
                        None | Some("") | Some("[ENCRYPTED]")
                    ) {
                        existing_discord_bot_token = secrets.discord_bot_token.clone();
                    }
                    if matches!(
                        existing_matrix_access_token.as_deref(),
                        None | Some("") | Some("[ENCRYPTED]")
                    ) {
                        existing_matrix_access_token = secrets.matrix_access_token.clone();
                    }
                    if matches!(
                        existing_teams_access_token.as_deref(),
                        None | Some("") | Some("[ENCRYPTED]")
                    ) {
                        existing_teams_access_token = secrets.teams_access_token.clone();
                    }
                    if matches!(
                        existing_whatsapp_token.as_deref(),
                        None | Some("") | Some("[ENCRYPTED]")
                    ) {
                        existing_whatsapp_token = secrets.whatsapp_access_token.clone();
                    }
                    if matches!(
                        existing_whatsapp_app_secret.as_deref(),
                        None | Some("") | Some("[ENCRYPTED]")
                    ) {
                        existing_whatsapp_app_secret = secrets.whatsapp_app_secret.clone();
                    }
                    if matches!(
                        existing_whatsapp_bridge_token.as_deref(),
                        None | Some("") | Some("[ENCRYPTED]")
                    ) {
                        existing_whatsapp_bridge_token = secrets.whatsapp_bridge_token.clone();
                    }
                }
            }
        }

        // Use new API key if provided, otherwise preserve existing (filter out "[ENCRYPTED]" placeholders)
        let new_api_key = settings
            .llm_api_key
            .clone()
            .filter(|k| !k.is_empty() && k != "[ENCRYPTED]");
        let api_key = new_api_key
            .clone()
            .or(existing_api_key.filter(|k| k != "[ENCRYPTED]"))
            .unwrap_or_default();

        // Fallback API key
        let fallback_api_key = settings
            .llm_fallback_api_key
            .clone()
            .filter(|k| !k.is_empty() && k != "[ENCRYPTED]")
            .or(existing_fallback_api_key.filter(|k| k != "[ENCRYPTED]"))
            .unwrap_or_default();

        let llm_provider_raw = settings.llm_provider.clone().unwrap_or_default();
        let llm_model_raw = settings.llm_model.clone().unwrap_or_default();

        // Handle empty base_url as None
        let base_url = settings.llm_base_url.clone().and_then(|u| {
            let trimmed = u.trim();
            if trimmed.is_empty() {
                None
            } else {
                Some(trimmed.to_string())
            }
        });
        let new_embeddings_api_key = settings
            .embeddings_api_key
            .clone()
            .filter(|key| !key.is_empty() && key != "[ENCRYPTED]");
        let embeddings_api_key = new_embeddings_api_key
            .clone()
            .or(existing_embeddings_api_key.filter(|key| key != "[ENCRYPTED]"))
            .unwrap_or_default();
        let embeddings_provider_raw = settings.embeddings_provider.clone().unwrap_or_else(|| {
            match current_embeddings.provider {
                EmbeddingsProviderKind::Disabled => "disabled".to_string(),
                EmbeddingsProviderKind::LocalHf => "local-hf".to_string(),
                EmbeddingsProviderKind::Ollama => "ollama".to_string(),
                EmbeddingsProviderKind::OpenaiCompatible => "openai-compatible".to_string(),
            }
        });
        let embeddings_model_raw = settings
            .embeddings_model
            .clone()
            .unwrap_or_else(|| current_embeddings.model.clone());
        let embeddings_base_url = settings.embeddings_base_url.clone().and_then(|value| {
            let trimmed = value.trim();
            if trimmed.is_empty() {
                None
            } else {
                Some(trimmed.to_string())
            }
        });
        let new_embeddings = match embeddings_provider_raw.trim() {
            "" | "disabled" | "none" | "off" => EmbeddingsConfig {
                provider: EmbeddingsProviderKind::Disabled,
                model: if embeddings_model_raw.trim().is_empty() {
                    "BAAI/bge-small-en-v1.5".to_string()
                } else {
                    embeddings_model_raw.trim().to_string()
                },
                base_url: None,
                api_key: String::new(),
            },
            "local-hf" | "local_hf" => EmbeddingsConfig {
                provider: EmbeddingsProviderKind::LocalHf,
                model: if embeddings_model_raw.trim().is_empty() {
                    "BAAI/bge-small-en-v1.5".to_string()
                } else {
                    embeddings_model_raw.trim().to_string()
                },
                base_url: None,
                api_key: String::new(),
            },
            "ollama" => {
                let Some(url) = embeddings_base_url.clone() else {
                    return (
                        StatusCode::BAD_REQUEST,
                        Json(ErrorResponse {
                            error: "Embeddings base URL is required for Ollama".to_string(),
                        }),
                    )
                        .into_response();
                };
                if embeddings_model_raw.trim().is_empty() {
                    return (
                        StatusCode::BAD_REQUEST,
                        Json(ErrorResponse {
                            error: "Embeddings model is required for Ollama".to_string(),
                        }),
                    )
                        .into_response();
                }
                EmbeddingsConfig {
                    provider: EmbeddingsProviderKind::Ollama,
                    model: embeddings_model_raw.trim().to_string(),
                    base_url: Some(url),
                    api_key: String::new(),
                }
            }
            "openai-compatible" | "openai_compatible" | "openai-compatible-hosted" => {
                if embeddings_model_raw.trim().is_empty() {
                    return (
                        StatusCode::BAD_REQUEST,
                        Json(ErrorResponse {
                            error: "Embeddings model is required for the external provider"
                                .to_string(),
                        }),
                    )
                        .into_response();
                }
                let normalized_embeddings_base_url = if embeddings_base_url.is_some() {
                    match normalize_openai_base_url(
                        "openai-compatible",
                        embeddings_base_url.clone(),
                    ) {
                        Ok(url) => url,
                        Err(error) => {
                            return (StatusCode::BAD_REQUEST, Json(ErrorResponse { error }))
                                .into_response();
                        }
                    }
                } else {
                    None
                };
                EmbeddingsConfig {
                    provider: EmbeddingsProviderKind::OpenaiCompatible,
                    model: embeddings_model_raw.trim().to_string(),
                    base_url: normalized_embeddings_base_url,
                    api_key: embeddings_api_key.clone(),
                }
            }
            other => {
                return (
                    StatusCode::BAD_REQUEST,
                    Json(ErrorResponse {
                        error: format!("Unknown embeddings provider: {}", other),
                    }),
                )
                    .into_response();
            }
        };

        // Determine if the user actually changed LLM settings or is just saving other fields.
        // If the user didn't send a new API key AND a valid LLM config already exists in memory,
        // reuse the existing config so non-LLM saves (WhatsApp, Telegram, etc.) aren't blocked.
        // Also treat as unchanged if the Model Pool has a primary model (user manages LLM there).
        let has_model_pool_primary = agent_guard.config.model_pool.slots.iter().any(|s| {
            matches!(s.role, crate::core::runtime::config::ModelRole::Primary) && s.enabled
        });
        let llm_unchanged = new_api_key.is_none()
            && (has_model_pool_primary
                || (!matches!(agent_guard.config.llm, LlmProvider::Ollama { .. })
                    && !matches!(
                        existing_api_key_raw.as_deref(),
                        None | Some("") | Some("[ENCRYPTED]")
                    )));

        let llm_request_omitted = settings.llm_provider.is_none()
            && settings.llm_model.is_none()
            && settings.llm_base_url.is_none()
            && settings.llm_api_key.is_none();
        let llm_request_is_blank = llm_provider_raw.trim().is_empty()
            && llm_model_raw.trim().is_empty()
            && base_url.is_none()
            && new_api_key.is_none();

        let new_llm = if llm_request_omitted || llm_unchanged {
            // Preserve current LLM config as-is - user didn't change it
            agent_guard.config.llm.clone()
        } else if llm_request_is_blank {
            LlmProvider::default()
        } else {
            // Validate LLM fields
            if llm_provider_raw.trim().is_empty() {
                return (
                    StatusCode::BAD_REQUEST,
                    Json(ErrorResponse {
                        error: "LLM provider is required".to_string(),
                    }),
                )
                    .into_response();
            }
            if llm_model_raw.trim().is_empty() {
                return (
                    StatusCode::BAD_REQUEST,
                    Json(ErrorResponse {
                        error: "LLM model is required".to_string(),
                    }),
                )
                    .into_response();
            }
            let Some(llm_provider_id) = canonical_provider_id(llm_provider_raw.as_str()) else {
                return (
                    StatusCode::BAD_REQUEST,
                    Json(ErrorResponse {
                        error: format!("Unknown provider: {}", llm_provider_raw),
                    }),
                )
                    .into_response();
            };

            let mut api_key_for_provider = api_key.clone();
            if llm_provider_id == "openai-subscription" && api_key_for_provider.trim().is_empty() {
                let oauth_client = reqwest::Client::builder()
                    .timeout(std::time::Duration::from_secs(10))
                    .build();
                api_key_for_provider = match oauth_client {
                    Ok(client) => resolve_codex_cli_api_key(&client, false)
                        .await
                        .ok()
                        .flatten()
                        .unwrap_or_default(),
                    Err(_) => String::new(),
                };
            }
            if llm_provider_requires_api_key(llm_provider_id)
                && api_key_for_provider.trim().is_empty()
            {
                return (
                    StatusCode::BAD_REQUEST,
                    Json(ErrorResponse {
                        error: "API key is required for the selected provider".to_string(),
                    }),
                )
                    .into_response();
            }

            if llm_provider_id == "ollama" && base_url.as_deref().unwrap_or("").trim().is_empty() {
                return (
                    StatusCode::BAD_REQUEST,
                    Json(ErrorResponse {
                        error: "Ollama base URL is required".to_string(),
                    }),
                )
                    .into_response();
            }
            if llm_provider_id == "openai-compatible"
                && base_url.as_deref().unwrap_or("").trim().is_empty()
            {
                return (
                    StatusCode::BAD_REQUEST,
                    Json(ErrorResponse {
                        error: "Base URL is required for OpenAI-Compatible providers".to_string(),
                    }),
                )
                    .into_response();
            }
            let compat_base_url = match normalize_openai_base_url(llm_provider_id, base_url.clone())
            {
                Ok(url) => url,
                Err(error) => {
                    return (StatusCode::BAD_REQUEST, Json(ErrorResponse { error }))
                        .into_response();
                }
            };

            // Build new LLM provider
            match llm_provider_id {
                "ollama" => LlmProvider::Ollama {
                    base_url: base_url.unwrap_or_default(),
                    model: llm_model_raw.clone(),
                },
                "anthropic" => LlmProvider::Anthropic {
                    api_key: api_key_for_provider.clone(),
                    model: llm_model_raw.clone(),
                },
                "openai" => LlmProvider::OpenAI {
                    api_key: api_key_for_provider.clone(),
                    model: llm_model_raw.clone(),
                    base_url: None,
                },
                "openai-compatible" | "openrouter" | "huggingface" => LlmProvider::OpenAI {
                    api_key: api_key_for_provider.clone(),
                    model: llm_model_raw.clone(),
                    base_url: compat_base_url,
                },
                "openai-subscription" => LlmProvider::OpenAI {
                    api_key: api_key_for_provider.clone(),
                    model: llm_model_raw.clone(),
                    base_url: compat_base_url,
                },
                _ => {
                    return (
                        StatusCode::BAD_REQUEST,
                        Json(ErrorResponse {
                            error: format!("Unknown provider: {}", llm_provider_raw),
                        }),
                    )
                        .into_response();
                }
            }
        };

        // Build fallback LLM provider (optional)
        let fallback_base_url = settings.llm_fallback_base_url.clone().and_then(|u| {
            let trimmed = u.trim();
            if trimmed.is_empty() {
                None
            } else {
                Some(trimmed.to_string())
            }
        });
        let new_llm_fallback: Option<LlmProvider> = if let Some(fb_provider) =
            &settings.llm_fallback_provider
        {
            if !fb_provider.is_empty()
                && settings
                    .llm_fallback_model
                    .as_ref()
                    .map(|m| !m.is_empty())
                    .unwrap_or(false)
            {
                let fb_model = settings.llm_fallback_model.clone().unwrap_or_default();
                let Some(fb_provider_id) = canonical_provider_id(fb_provider.as_str()) else {
                    return (
                        StatusCode::BAD_REQUEST,
                        Json(ErrorResponse {
                            error: format!("Unknown fallback provider: {}", fb_provider),
                        }),
                    )
                        .into_response();
                };
                let mut resolved_fallback_api_key = fallback_api_key.clone();
                if fb_provider_id == "openai-subscription"
                    && resolved_fallback_api_key.trim().is_empty()
                {
                    let oauth_client = reqwest::Client::builder()
                        .timeout(std::time::Duration::from_secs(10))
                        .build();
                    resolved_fallback_api_key = match oauth_client {
                        Ok(client) => resolve_codex_cli_api_key(&client, false)
                            .await
                            .ok()
                            .flatten()
                            .unwrap_or_default(),
                        Err(_) => String::new(),
                    };
                }
                if llm_provider_requires_api_key(fb_provider_id)
                    && resolved_fallback_api_key.trim().is_empty()
                {
                    return (
                        StatusCode::BAD_REQUEST,
                        Json(ErrorResponse {
                            error: "Fallback API key is required for the selected provider"
                                .to_string(),
                        }),
                    )
                        .into_response();
                }
                let fallback_compat_base_url =
                    match normalize_openai_base_url(fb_provider_id, fallback_base_url.clone()) {
                        Ok(url) => url,
                        Err(error) => {
                            return (StatusCode::BAD_REQUEST, Json(ErrorResponse { error }))
                                .into_response();
                        }
                    };
                match fb_provider_id {
                    "ollama" => Some(LlmProvider::Ollama {
                        base_url: if let Some(url) = fallback_base_url {
                            url
                        } else {
                            return (
                                StatusCode::BAD_REQUEST,
                                Json(ErrorResponse {
                                    error: "Fallback Ollama base URL is required".to_string(),
                                }),
                            )
                                .into_response();
                        },
                        model: fb_model,
                    }),
                    "anthropic" => Some(LlmProvider::Anthropic {
                        api_key: resolved_fallback_api_key.clone(),
                        model: fb_model,
                    }),
                    "openai" => Some(LlmProvider::OpenAI {
                        api_key: resolved_fallback_api_key.clone(),
                        model: fb_model,
                        base_url: None,
                    }),
                    "openai-compatible" | "openrouter" | "openai-subscription" | "huggingface" => {
                        Some(LlmProvider::OpenAI {
                            api_key: resolved_fallback_api_key.clone(),
                            model: fb_model,
                            base_url: fallback_compat_base_url,
                        })
                    }
                    _ => None,
                }
            } else {
                None
            }
        } else {
            None
        };

        let telegram_update_requested = settings.telegram_enabled.is_some()
            || settings.telegram_bot_token.is_some()
            || settings.telegram_allowed_users.is_some();
        let current_telegram = agent_guard.config.telegram.clone();

        // Build telegram config only if that section was explicitly touched.
        let new_telegram = if telegram_update_requested {
            let telegram_enabled = settings
                .telegram_enabled
                .unwrap_or(current_telegram.is_some());
            if telegram_enabled {
                let token = settings
                    .telegram_bot_token
                    .clone()
                    .filter(|t| !t.is_empty() && t != "[ENCRYPTED]")
                    .or(existing_telegram_token.filter(|t| t != "[ENCRYPTED]"));

                if token.as_deref().unwrap_or("").trim().is_empty() {
                    return (
                        StatusCode::BAD_REQUEST,
                        Json(ErrorResponse {
                            error: "Telegram bot token is required when Telegram is enabled"
                                .to_string(),
                        }),
                    )
                        .into_response();
                }

                Some(TelegramConfig {
                    bot_token: token.unwrap(),
                    allowed_users: settings.telegram_allowed_users.clone().unwrap_or(
                        current_telegram
                            .as_ref()
                            .map(|t| t.allowed_users.clone())
                            .unwrap_or_default(),
                    ),
                    dm_policy: current_telegram
                        .as_ref()
                        .map(|t| t.dm_policy.clone())
                        .unwrap_or("pairing".to_string()),
                })
            } else {
                None
            }
        } else {
            current_telegram
        };

        let slack_update_requested = settings.slack_enabled.is_some()
            || settings.slack_bot_token.is_some()
            || settings.slack_signing_secret.is_some()
            || settings.slack_api_base_url.is_some()
            || settings.slack_default_channel_id.is_some()
            || settings.slack_default_thread_ts.is_some()
            || settings.slack_workspace_id.is_some()
            || settings.slack_workspace_name.is_some();
        let current_slack = agent_guard.config.slack.clone();
        let new_slack = if slack_update_requested {
            let slack_enabled = settings.slack_enabled.unwrap_or(
                current_slack.is_some()
                    || settings.slack_bot_token.is_some()
                    || settings.slack_signing_secret.is_some()
                    || settings.slack_api_base_url.is_some()
                    || settings.slack_default_channel_id.is_some()
                    || settings.slack_default_thread_ts.is_some()
                    || settings.slack_workspace_id.is_some()
                    || settings.slack_workspace_name.is_some(),
            );
            if slack_enabled {
                let bot_token = settings
                    .slack_bot_token
                    .clone()
                    .filter(|t| !t.is_empty() && t != "[ENCRYPTED]")
                    .or(existing_slack_bot_token.filter(|t| t != "[ENCRYPTED]"))
                    .unwrap_or_default();
                let signing_secret = settings
                    .slack_signing_secret
                    .clone()
                    .filter(|t| !t.is_empty() && t != "[ENCRYPTED]")
                    .or(existing_slack_signing_secret.filter(|t| t != "[ENCRYPTED]"))
                    .unwrap_or_default();
                if bot_token.trim().is_empty() {
                    return (
                        StatusCode::BAD_REQUEST,
                        Json(ErrorResponse {
                            error: "Slack bot token is required when Slack is enabled".to_string(),
                        }),
                    )
                        .into_response();
                }
                if signing_secret.trim().is_empty() {
                    return (
                        StatusCode::BAD_REQUEST,
                        Json(ErrorResponse {
                            error: "Slack signing secret is required when Slack is enabled"
                                .to_string(),
                        }),
                    )
                        .into_response();
                }

                Some(SlackChannelConfig {
                    bot_token,
                    signing_secret,
                    api_base_url: settings
                        .slack_api_base_url
                        .clone()
                        .or_else(|| current_slack.as_ref().map(|c| c.api_base_url.clone()))
                        .unwrap_or("https://slack.com/api".to_string()),
                    default_channel_id: settings
                        .slack_default_channel_id
                        .clone()
                        .or_else(|| current_slack.as_ref().map(|c| c.default_channel_id.clone()))
                        .unwrap_or_default(),
                    default_thread_ts: settings.slack_default_thread_ts.clone().or_else(|| {
                        current_slack
                            .as_ref()
                            .and_then(|c| c.default_thread_ts.clone())
                    }),
                    workspace_id: settings
                        .slack_workspace_id
                        .clone()
                        .or_else(|| current_slack.as_ref().and_then(|c| c.workspace_id.clone())),
                    workspace_name: settings.slack_workspace_name.clone().or_else(|| {
                        current_slack
                            .as_ref()
                            .and_then(|c| c.workspace_name.clone())
                    }),
                })
            } else {
                None
            }
        } else {
            current_slack
        };

        let discord_update_requested = settings.discord_enabled.is_some()
            || settings.discord_bot_token.is_some()
            || settings.discord_api_base_url.is_some()
            || settings.discord_default_channel_id.is_some()
            || settings.discord_default_thread_id.is_some()
            || settings.discord_guild_id.is_some()
            || settings.discord_application_id.is_some()
            || settings.discord_webhook_url.is_some();
        let current_discord = agent_guard.config.discord.clone();
        let new_discord = if discord_update_requested {
            let discord_enabled = settings.discord_enabled.unwrap_or(
                current_discord.is_some()
                    || settings.discord_bot_token.is_some()
                    || settings.discord_api_base_url.is_some()
                    || settings.discord_default_channel_id.is_some()
                    || settings.discord_default_thread_id.is_some()
                    || settings.discord_guild_id.is_some()
                    || settings.discord_application_id.is_some()
                    || settings.discord_webhook_url.is_some(),
            );
            if discord_enabled {
                let bot_token = settings
                    .discord_bot_token
                    .clone()
                    .filter(|t| !t.is_empty() && t != "[ENCRYPTED]")
                    .or(existing_discord_bot_token.filter(|t| t != "[ENCRYPTED]"))
                    .unwrap_or_default();
                let webhook_url = settings
                    .discord_webhook_url
                    .clone()
                    .or_else(|| current_discord.as_ref().map(|c| c.webhook_url.clone()))
                    .unwrap_or_default();
                if bot_token.trim().is_empty() {
                    return (
                        StatusCode::BAD_REQUEST,
                        Json(ErrorResponse {
                            error: "Discord bot token is required when Discord is enabled"
                                .to_string(),
                        }),
                    )
                        .into_response();
                }
                let default_channel_id = settings
                    .discord_default_channel_id
                    .clone()
                    .or_else(|| {
                        current_discord
                            .as_ref()
                            .map(|c| c.default_channel_id.clone())
                    })
                    .unwrap_or_default();
                let default_thread_id = settings.discord_default_thread_id.clone().or_else(|| {
                    current_discord
                        .as_ref()
                        .and_then(|c| c.default_thread_id.clone())
                });
                let guild_id = settings
                    .discord_guild_id
                    .clone()
                    .or_else(|| current_discord.as_ref().and_then(|c| c.guild_id.clone()));
                if guild_id
                    .as_deref()
                    .map(str::trim)
                    .filter(|value| !value.is_empty())
                    .is_none()
                    && default_thread_id
                        .as_deref()
                        .map(str::trim)
                        .filter(|value| !value.is_empty())
                        .is_none()
                    && default_channel_id.trim().is_empty()
                {
                    return (
                        StatusCode::BAD_REQUEST,
                        Json(ErrorResponse {
                            error: "Discord requires a guild, channel, or thread scope when it is enabled"
                                .to_string(),
                        }),
                    )
                        .into_response();
                }
                let application_id = settings.discord_application_id.clone().or_else(|| {
                    current_discord
                        .as_ref()
                        .and_then(|c| c.application_id.clone())
                });

                Some(DiscordChannelConfig {
                    bot_token,
                    webhook_url,
                    api_base_url: settings
                        .discord_api_base_url
                        .clone()
                        .or_else(|| current_discord.as_ref().map(|c| c.api_base_url.clone()))
                        .unwrap_or("https://discord.com/api".to_string()),
                    default_channel_id,
                    default_thread_id,
                    guild_id,
                    application_id,
                })
            } else {
                None
            }
        } else {
            current_discord
        };

        let matrix_update_requested = settings.matrix_enabled.is_some()
            || settings.matrix_homeserver_url.is_some()
            || settings.matrix_access_token.is_some()
            || settings.matrix_user_id.is_some()
            || settings.matrix_device_id.is_some()
            || settings.matrix_account_id.is_some()
            || settings.matrix_default_room_id.is_some()
            || settings.matrix_sync_timeout_ms.is_some()
            || settings.matrix_limit.is_some()
            || settings.matrix_user_agent.is_some();
        let current_matrix = agent_guard.config.matrix.clone();
        let new_matrix = if matrix_update_requested {
            let matrix_enabled = settings.matrix_enabled.unwrap_or(
                current_matrix.is_some()
                    || settings.matrix_homeserver_url.is_some()
                    || settings.matrix_access_token.is_some()
                    || settings.matrix_user_id.is_some()
                    || settings.matrix_device_id.is_some()
                    || settings.matrix_account_id.is_some()
                    || settings.matrix_default_room_id.is_some()
                    || settings.matrix_sync_timeout_ms.is_some()
                    || settings.matrix_limit.is_some()
                    || settings.matrix_user_agent.is_some(),
            );
            if matrix_enabled {
                let access_token = settings
                    .matrix_access_token
                    .clone()
                    .filter(|t| !t.is_empty() && t != "[ENCRYPTED]")
                    .or(existing_matrix_access_token.filter(|t| t != "[ENCRYPTED]"))
                    .unwrap_or_default();
                let homeserver_url = settings
                    .matrix_homeserver_url
                    .clone()
                    .or_else(|| current_matrix.as_ref().map(|c| c.homeserver_url.clone()))
                    .unwrap_or_default();
                let user_id = settings
                    .matrix_user_id
                    .clone()
                    .or_else(|| current_matrix.as_ref().map(|c| c.user_id.clone()))
                    .unwrap_or_default();
                if access_token.trim().is_empty() {
                    return (
                        StatusCode::BAD_REQUEST,
                        Json(ErrorResponse {
                            error: "Matrix access token is required when Matrix is enabled"
                                .to_string(),
                        }),
                    )
                        .into_response();
                }
                if homeserver_url.trim().is_empty() || user_id.trim().is_empty() {
                    return (
                        StatusCode::BAD_REQUEST,
                        Json(ErrorResponse {
                            error: "Matrix homeserver URL and user ID are required when Matrix is enabled"
                                .to_string(),
                        }),
                    )
                        .into_response();
                }

                Some(MatrixTransportConfig {
                    homeserver_url,
                    access_token,
                    user_id,
                    device_id: settings
                        .matrix_device_id
                        .clone()
                        .or_else(|| current_matrix.as_ref().and_then(|c| c.device_id.clone())),
                    account_id: settings
                        .matrix_account_id
                        .clone()
                        .or_else(|| current_matrix.as_ref().and_then(|c| c.account_id.clone())),
                    default_room_id: settings.matrix_default_room_id.clone().or_else(|| {
                        current_matrix
                            .as_ref()
                            .and_then(|c| c.default_room_id.clone())
                    }),
                    sync_timeout_ms: settings
                        .matrix_sync_timeout_ms
                        .or_else(|| current_matrix.as_ref().map(|c| c.sync_timeout_ms))
                        .unwrap_or(0),
                    limit: settings
                        .matrix_limit
                        .or_else(|| current_matrix.as_ref().map(|c| c.limit))
                        .unwrap_or(100),
                    user_agent: settings
                        .matrix_user_agent
                        .clone()
                        .or_else(|| current_matrix.as_ref().and_then(|c| c.user_agent.clone())),
                })
            } else {
                None
            }
        } else {
            current_matrix
        };

        let teams_update_requested = settings.teams_enabled.is_some()
            || settings.teams_service_url.is_some()
            || settings.teams_access_token.is_some()
            || settings.teams_bot_app_id.is_some()
            || settings.teams_bot_name.is_some()
            || settings.teams_tenant_id.is_some()
            || settings.teams_team_id.is_some()
            || settings.teams_channel_id.is_some()
            || settings.teams_chat_id.is_some()
            || settings.teams_graph_base_url.is_some()
            || settings.teams_delivery_mode.is_some()
            || settings.teams_timeout_secs.is_some()
            || settings.teams_user_agent.is_some();
        let current_teams = agent_guard.config.teams.clone();
        let new_teams = if teams_update_requested {
            let teams_enabled = settings.teams_enabled.unwrap_or(
                current_teams.is_some()
                    || settings.teams_service_url.is_some()
                    || settings.teams_access_token.is_some()
                    || settings.teams_bot_app_id.is_some()
                    || settings.teams_bot_name.is_some()
                    || settings.teams_tenant_id.is_some()
                    || settings.teams_team_id.is_some()
                    || settings.teams_channel_id.is_some()
                    || settings.teams_chat_id.is_some()
                    || settings.teams_graph_base_url.is_some()
                    || settings.teams_delivery_mode.is_some()
                    || settings.teams_timeout_secs.is_some()
                    || settings.teams_user_agent.is_some(),
            );
            if teams_enabled {
                let access_token = settings
                    .teams_access_token
                    .clone()
                    .filter(|t| !t.is_empty() && t != "[ENCRYPTED]")
                    .or(existing_teams_access_token.filter(|t| t != "[ENCRYPTED]"))
                    .unwrap_or_default();
                let service_url = settings
                    .teams_service_url
                    .clone()
                    .or_else(|| current_teams.as_ref().map(|c| c.service_url.clone()))
                    .unwrap_or_default();
                if access_token.trim().is_empty() {
                    return (
                        StatusCode::BAD_REQUEST,
                        Json(ErrorResponse {
                            error: "Teams access token is required when Teams is enabled"
                                .to_string(),
                        }),
                    )
                        .into_response();
                }
                if service_url.trim().is_empty() {
                    return (
                        StatusCode::BAD_REQUEST,
                        Json(ErrorResponse {
                            error: "Teams service URL is required when Teams is enabled"
                                .to_string(),
                        }),
                    )
                        .into_response();
                }
                let bot_app_id = settings
                    .teams_bot_app_id
                    .clone()
                    .or_else(|| current_teams.as_ref().and_then(|c| c.bot_app_id.clone()))
                    .unwrap_or_default();
                if bot_app_id.trim().is_empty() {
                    return (
                        StatusCode::BAD_REQUEST,
                        Json(ErrorResponse {
                            error: "Teams bot app ID is required when Teams is enabled".to_string(),
                        }),
                    )
                        .into_response();
                }

                let current_delivery_mode = current_teams.as_ref().map(|c| match c.delivery_mode {
                    crate::channels::teams::TeamsDeliveryMode::Auto => "auto",
                    crate::channels::teams::TeamsDeliveryMode::BotFramework => "bot_framework",
                    crate::channels::teams::TeamsDeliveryMode::Graph => "graph",
                });
                let delivery_mode = match settings
                    .teams_delivery_mode
                    .as_deref()
                    .or(current_delivery_mode)
                    .unwrap_or("auto")
                {
                    "bot_framework" | "botframework" => {
                        crate::channels::teams::TeamsDeliveryMode::BotFramework
                    }
                    "graph" => crate::channels::teams::TeamsDeliveryMode::Graph,
                    _ => crate::channels::teams::TeamsDeliveryMode::Auto,
                };

                Some(TeamsTransportConfig {
                    service_url,
                    access_token,
                    bot_app_id: Some(bot_app_id),
                    bot_name: settings
                        .teams_bot_name
                        .clone()
                        .or_else(|| current_teams.as_ref().and_then(|c| c.bot_name.clone())),
                    tenant_id: settings
                        .teams_tenant_id
                        .clone()
                        .or_else(|| current_teams.as_ref().and_then(|c| c.tenant_id.clone())),
                    team_id: settings
                        .teams_team_id
                        .clone()
                        .or_else(|| current_teams.as_ref().and_then(|c| c.team_id.clone())),
                    channel_id: settings
                        .teams_channel_id
                        .clone()
                        .or_else(|| current_teams.as_ref().and_then(|c| c.channel_id.clone())),
                    chat_id: settings
                        .teams_chat_id
                        .clone()
                        .or_else(|| current_teams.as_ref().and_then(|c| c.chat_id.clone())),
                    graph_base_url: settings.teams_graph_base_url.clone().or_else(|| {
                        current_teams
                            .as_ref()
                            .and_then(|c| c.graph_base_url.clone())
                    }),
                    delivery_mode,
                    timeout_secs: settings
                        .teams_timeout_secs
                        .or_else(|| current_teams.as_ref().map(|c| c.timeout_secs))
                        .unwrap_or(15),
                    user_agent: settings
                        .teams_user_agent
                        .clone()
                        .or_else(|| current_teams.as_ref().and_then(|c| c.user_agent.clone())),
                })
            } else {
                None
            }
        } else {
            current_teams
        };

        let whatsapp_update_requested = settings.whatsapp_enabled.is_some()
            || settings.whatsapp_mode.is_some()
            || settings.whatsapp_access_token.is_some()
            || settings.whatsapp_app_secret.is_some()
            || settings.whatsapp_phone_number_id.is_some()
            || settings.whatsapp_verify_token.is_some()
            || settings.whatsapp_bridge_runtime.is_some()
            || settings.whatsapp_bridge_token.is_some()
            || settings.whatsapp_bridge_url.is_some()
            || settings.whatsapp_dm_policy.is_some()
            || settings.whatsapp_allowed_numbers.is_some();
        let current_whatsapp = agent_guard.config.whatsapp.clone();

        // Build WhatsApp config only if that section was explicitly touched.
        let new_whatsapp = if whatsapp_update_requested {
            let whatsapp_enabled = settings
                .whatsapp_enabled
                .unwrap_or(current_whatsapp.is_some());
            if whatsapp_enabled {
                let mode_str = settings
                    .whatsapp_mode
                    .as_deref()
                    .or_else(|| {
                        current_whatsapp.as_ref().map(|w| match w.mode {
                            crate::channels::whatsapp::WhatsAppMode::CloudApi => "cloud_api",
                            crate::channels::whatsapp::WhatsAppMode::Baileys => "baileys",
                        })
                    })
                    .unwrap_or("baileys");
                let mode = match mode_str {
                    "cloud_api" => crate::channels::whatsapp::WhatsAppMode::CloudApi,
                    _ => crate::channels::whatsapp::WhatsAppMode::Baileys,
                };

                let token = settings
                    .whatsapp_access_token
                    .clone()
                    .filter(|t| !t.is_empty() && t != "[ENCRYPTED]")
                    .or(existing_whatsapp_token.filter(|t| t != "[ENCRYPTED]"))
                    .unwrap_or_default();
                let app_secret = settings
                    .whatsapp_app_secret
                    .clone()
                    .filter(|t| !t.is_empty() && t != "[ENCRYPTED]")
                    .or(existing_whatsapp_app_secret.filter(|t| t != "[ENCRYPTED]"))
                    .unwrap_or_default();
                let mut bridge_token = settings
                    .whatsapp_bridge_token
                    .clone()
                    .filter(|t| !t.is_empty() && t != "[ENCRYPTED]")
                    .or(existing_whatsapp_bridge_token.filter(|t| t != "[ENCRYPTED]"))
                    .unwrap_or_default();

                let phone_id = settings
                    .whatsapp_phone_number_id
                    .clone()
                    .or_else(|| current_whatsapp.as_ref().map(|w| w.phone_number_id.clone()))
                    .unwrap_or_default();

                let verify_tok = settings
                    .whatsapp_verify_token
                    .clone()
                    .or_else(|| current_whatsapp.as_ref().map(|w| w.verify_token.clone()))
                    .unwrap_or("agentark_verify".to_string());

                let bridge_runtime = settings
                    .whatsapp_bridge_runtime
                    .as_deref()
                    .map(|value| match value {
                        "external" => crate::channels::whatsapp::WhatsAppBridgeRuntime::External,
                        _ => crate::channels::whatsapp::WhatsAppBridgeRuntime::Embedded,
                    })
                    .or_else(|| current_whatsapp.as_ref().map(|w| w.bridge_runtime()))
                    .unwrap_or(crate::channels::whatsapp::WhatsAppBridgeRuntime::Embedded);

                let requested_bridge_url = settings
                    .whatsapp_bridge_url
                    .clone()
                    .or_else(|| current_whatsapp.as_ref().map(|w| w.bridge_url.clone()))
                    .unwrap_or(crate::channels::whatsapp::EMBEDDED_BRIDGE_URL.to_string());
                let bridge_url = match bridge_runtime {
                    crate::channels::whatsapp::WhatsAppBridgeRuntime::Embedded => {
                        if !crate::channels::whatsapp::is_loopback_bridge_url(&requested_bridge_url)
                        {
                            return (
                                StatusCode::BAD_REQUEST,
                                Json(ErrorResponse {
                                    error: "Bundled WhatsApp bridge mode only supports a local loopback URL"
                                        .to_string(),
                                }),
                            )
                                .into_response();
                        }
                        if mode == crate::channels::whatsapp::WhatsAppMode::Baileys
                            && bridge_token.trim().is_empty()
                        {
                            bridge_token = generate_whatsapp_bridge_token();
                        }
                        crate::channels::whatsapp::EMBEDDED_BRIDGE_URL.to_string()
                    }
                    crate::channels::whatsapp::WhatsAppBridgeRuntime::External => {
                        let trimmed = requested_bridge_url.trim().to_string();
                        if trimmed.is_empty() {
                            return (
                                StatusCode::BAD_REQUEST,
                                Json(ErrorResponse {
                                    error: "WhatsApp external bridge URL is required in external bridge mode"
                                        .to_string(),
                                }),
                            )
                                .into_response();
                        }
                        let legacy_external_missing_token =
                            current_whatsapp.as_ref().is_some_and(|config| {
                                config.uses_external_bridge()
                                    && config.bridge_token.trim().is_empty()
                                    && settings.whatsapp_bridge_token.is_none()
                            });
                        if bridge_token.trim().is_empty() && !legacy_external_missing_token {
                            return (
                                StatusCode::BAD_REQUEST,
                                Json(ErrorResponse {
                                    error: "WhatsApp external bridge token is required for new external bridge setups"
                                        .to_string(),
                                }),
                            )
                                .into_response();
                        }
                        trimmed
                    }
                };

                let dm_policy = settings
                    .whatsapp_dm_policy
                    .clone()
                    .or_else(|| current_whatsapp.as_ref().map(|w| w.dm_policy.clone()))
                    .unwrap_or("pairing".to_string());

                // Cloud API mode requires access token, app secret, verify token, and phone number ID.
                if mode == crate::channels::whatsapp::WhatsAppMode::CloudApi {
                    if token.trim().is_empty() {
                        return (
                            StatusCode::BAD_REQUEST,
                            Json(ErrorResponse {
                                error: "WhatsApp access token is required for Cloud API mode"
                                    .to_string(),
                            }),
                        )
                            .into_response();
                    }
                    if app_secret.trim().is_empty() {
                        return (
                            StatusCode::BAD_REQUEST,
                            Json(ErrorResponse {
                                error: "WhatsApp app secret is required for Cloud API mode"
                                    .to_string(),
                            }),
                        )
                            .into_response();
                    }
                    if phone_id.trim().is_empty() {
                        return (
                            StatusCode::BAD_REQUEST,
                            Json(ErrorResponse {
                                error: "WhatsApp Phone Number ID is required for Cloud API mode"
                                    .to_string(),
                            }),
                        )
                            .into_response();
                    }
                    if verify_tok.trim().is_empty() {
                        return (
                            StatusCode::BAD_REQUEST,
                            Json(ErrorResponse {
                                error: "WhatsApp verify token is required for Cloud API mode"
                                    .to_string(),
                            }),
                        )
                            .into_response();
                    }
                }

                Some(crate::channels::whatsapp::WhatsAppChannelConfig {
                    mode,
                    access_token: token,
                    app_secret,
                    phone_number_id: phone_id,
                    verify_token: verify_tok,
                    bridge_runtime: Some(bridge_runtime),
                    bridge_url,
                    bridge_token,
                    allowed_numbers: settings.whatsapp_allowed_numbers.clone().unwrap_or_else(
                        || {
                            current_whatsapp
                                .as_ref()
                                .map(|w| w.allowed_numbers.clone())
                                .unwrap_or_default()
                        },
                    ),
                    dm_policy,
                })
            } else {
                None
            }
        } else {
            current_whatsapp
        };

        let google_chat_update_requested = settings.google_chat_enabled.is_some()
            || settings.google_chat_access_token.is_some()
            || settings.google_chat_verify_token.is_some()
            || settings.google_chat_api_base_url.is_some()
            || settings.google_chat_space.is_some()
            || settings.google_chat_thread_key.is_some()
            || settings.google_chat_app_id.is_some()
            || settings.google_chat_bot_name.is_some();
        let current_google_chat = agent_guard.config.google_chat.clone();
        let new_google_chat = if google_chat_update_requested {
            let google_chat_enabled = settings.google_chat_enabled.unwrap_or(
                current_google_chat.is_some()
                    || settings.google_chat_access_token.is_some()
                    || settings.google_chat_verify_token.is_some()
                    || settings.google_chat_space.is_some(),
            );
            if google_chat_enabled {
                let access_token = settings
                    .google_chat_access_token
                    .clone()
                    .filter(|value| !value.is_empty() && value != "[ENCRYPTED]")
                    .or_else(|| {
                        current_google_chat
                            .as_ref()
                            .map(|cfg| cfg.access_token.clone())
                            .filter(|value| !value.is_empty() && value != "[ENCRYPTED]")
                    })
                    .unwrap_or_default();
                let verify_token = settings
                    .google_chat_verify_token
                    .clone()
                    .filter(|value| !value.is_empty() && value != "[ENCRYPTED]")
                    .or_else(|| {
                        current_google_chat
                            .as_ref()
                            .map(|cfg| cfg.verify_token.clone())
                            .filter(|value| !value.is_empty() && value != "[ENCRYPTED]")
                    })
                    .unwrap_or_default();
                let space = settings.google_chat_space.clone().or_else(|| {
                    current_google_chat
                        .as_ref()
                        .and_then(|cfg| cfg.space.clone())
                });
                if access_token.trim().is_empty() {
                    return (
                        StatusCode::BAD_REQUEST,
                        Json(ErrorResponse {
                            error:
                                "Google Chat access token is required when Google Chat is enabled"
                                    .to_string(),
                        }),
                    )
                        .into_response();
                }
                if verify_token.trim().is_empty() {
                    return (
                        StatusCode::BAD_REQUEST,
                        Json(ErrorResponse {
                            error:
                                "Google Chat verification token is required when Google Chat is enabled"
                                    .to_string(),
                        }),
                    )
                        .into_response();
                }
                if space
                    .as_deref()
                    .map(str::trim)
                    .filter(|value| !value.is_empty())
                    .is_none()
                {
                    return (
                        StatusCode::BAD_REQUEST,
                        Json(ErrorResponse {
                            error: "Google Chat space is required when Google Chat is enabled"
                                .to_string(),
                        }),
                    )
                        .into_response();
                }
                Some(GoogleChatChannelConfig {
                    api_base_url: settings
                        .google_chat_api_base_url
                        .clone()
                        .or_else(|| {
                            current_google_chat
                                .as_ref()
                                .map(|cfg| cfg.api_base_url.clone())
                        })
                        .unwrap_or_else(|| "https://chat.googleapis.com".to_string()),
                    access_token,
                    verify_token,
                    space,
                    thread_key: settings.google_chat_thread_key.clone().or_else(|| {
                        current_google_chat
                            .as_ref()
                            .and_then(|cfg| cfg.thread_key.clone())
                    }),
                    app_id: settings.google_chat_app_id.clone().or_else(|| {
                        current_google_chat
                            .as_ref()
                            .and_then(|cfg| cfg.app_id.clone())
                    }),
                    bot_name: settings.google_chat_bot_name.clone().or_else(|| {
                        current_google_chat
                            .as_ref()
                            .and_then(|cfg| cfg.bot_name.clone())
                    }),
                })
            } else {
                None
            }
        } else {
            current_google_chat
        };

        let line_update_requested = settings.line_enabled.is_some()
            || settings.line_channel_access_token.is_some()
            || settings.line_channel_secret.is_some()
            || settings.line_api_base_url.is_some()
            || settings.line_default_target.is_some()
            || settings.line_user_agent.is_some();
        let current_line = agent_guard.config.line.clone();
        let new_line = if line_update_requested {
            let line_enabled = settings.line_enabled.unwrap_or(
                current_line.is_some()
                    || settings.line_channel_access_token.is_some()
                    || settings.line_default_target.is_some(),
            );
            if line_enabled {
                let access_token = settings
                    .line_channel_access_token
                    .clone()
                    .filter(|value| !value.is_empty() && value != "[ENCRYPTED]")
                    .or_else(|| {
                        current_line
                            .as_ref()
                            .map(|cfg| cfg.channel_access_token.clone())
                            .filter(|value| !value.is_empty() && value != "[ENCRYPTED]")
                    })
                    .unwrap_or_default();
                let channel_secret = settings
                    .line_channel_secret
                    .clone()
                    .filter(|value| !value.is_empty() && value != "[ENCRYPTED]")
                    .or_else(|| {
                        current_line
                            .as_ref()
                            .map(|cfg| cfg.channel_secret.clone())
                            .filter(|value| !value.is_empty() && value != "[ENCRYPTED]")
                    })
                    .unwrap_or_default();
                let default_target = settings.line_default_target.clone().or_else(|| {
                    current_line
                        .as_ref()
                        .and_then(|cfg| cfg.default_target.clone())
                });
                if access_token.trim().is_empty() || channel_secret.trim().is_empty() {
                    return (
                        StatusCode::BAD_REQUEST,
                        Json(ErrorResponse {
                            error: "LINE access token and channel secret are required when LINE is enabled"
                                .to_string(),
                        }),
                    )
                        .into_response();
                }
                if default_target
                    .as_deref()
                    .map(str::trim)
                    .filter(|value| !value.is_empty())
                    .is_none()
                {
                    return (
                        StatusCode::BAD_REQUEST,
                        Json(ErrorResponse {
                            error: "LINE default target is required when LINE is enabled"
                                .to_string(),
                        }),
                    )
                        .into_response();
                }
                Some(LineChannelConfig {
                    api_base_url: settings
                        .line_api_base_url
                        .clone()
                        .or_else(|| current_line.as_ref().map(|cfg| cfg.api_base_url.clone()))
                        .unwrap_or_else(|| "https://api.line.me".to_string()),
                    channel_access_token: access_token,
                    channel_secret,
                    default_target,
                    user_agent: settings
                        .line_user_agent
                        .clone()
                        .or_else(|| current_line.as_ref().and_then(|cfg| cfg.user_agent.clone())),
                })
            } else {
                None
            }
        } else {
            current_line
        };

        let signal_update_requested = settings.signal_enabled.is_some()
            || settings.signal_bridge_token.is_some()
            || settings.signal_bridge_url.is_some()
            || settings.signal_default_recipient.is_some()
            || settings.signal_default_group_id.is_some();
        let current_signal = agent_guard.config.signal.clone();
        let new_signal = if signal_update_requested {
            let signal_enabled = settings.signal_enabled.unwrap_or(
                current_signal.is_some()
                    || settings.signal_bridge_token.is_some()
                    || settings.signal_default_recipient.is_some()
                    || settings.signal_default_group_id.is_some(),
            );
            if signal_enabled {
                let bridge_token = settings
                    .signal_bridge_token
                    .clone()
                    .filter(|value| !value.is_empty() && value != "[ENCRYPTED]")
                    .or_else(|| {
                        current_signal
                            .as_ref()
                            .map(|cfg| cfg.bridge_token.clone())
                            .filter(|value| !value.is_empty() && value != "[ENCRYPTED]")
                    })
                    .unwrap_or_default();
                let default_recipient = settings
                    .signal_default_recipient
                    .clone()
                    .or_else(|| {
                        current_signal
                            .as_ref()
                            .map(|cfg| cfg.default_recipient.clone())
                    })
                    .unwrap_or_default();
                let default_group_id = settings
                    .signal_default_group_id
                    .clone()
                    .or_else(|| {
                        current_signal
                            .as_ref()
                            .map(|cfg| cfg.default_group_id.clone())
                    })
                    .unwrap_or_default();
                if bridge_token.trim().is_empty() {
                    return (
                        StatusCode::BAD_REQUEST,
                        Json(ErrorResponse {
                            error: "Signal bridge token is required when Signal is enabled"
                                .to_string(),
                        }),
                    )
                        .into_response();
                }
                if default_recipient.trim().is_empty() && default_group_id.trim().is_empty() {
                    return (
                        StatusCode::BAD_REQUEST,
                        Json(ErrorResponse {
                            error:
                                "Signal needs a default recipient or group ID when it is enabled"
                                    .to_string(),
                        }),
                    )
                        .into_response();
                }
                Some(SignalChannelConfig {
                    bridge_url: settings
                        .signal_bridge_url
                        .clone()
                        .or_else(|| current_signal.as_ref().map(|cfg| cfg.bridge_url.clone()))
                        .unwrap_or_else(|| SignalChannelConfig::default().bridge_url),
                    bridge_token,
                    default_recipient,
                    default_group_id,
                })
            } else {
                None
            }
        } else {
            agent_guard.config.signal.clone()
        };

        let imessage_update_requested = settings.imessage_enabled.is_some()
            || settings.imessage_bridge_token.is_some()
            || settings.imessage_bridge_url.is_some()
            || settings.imessage_default_chat_id.is_some()
            || settings.imessage_default_handle.is_some();
        let current_imessage = agent_guard.config.imessage.clone();
        let new_imessage = if imessage_update_requested {
            let imessage_enabled = settings.imessage_enabled.unwrap_or(
                current_imessage.is_some()
                    || settings.imessage_bridge_token.is_some()
                    || settings.imessage_default_chat_id.is_some()
                    || settings.imessage_default_handle.is_some(),
            );
            if imessage_enabled {
                let bridge_token = settings
                    .imessage_bridge_token
                    .clone()
                    .filter(|value| !value.is_empty() && value != "[ENCRYPTED]")
                    .or_else(|| {
                        current_imessage
                            .as_ref()
                            .map(|cfg| cfg.bridge_token.clone())
                            .filter(|value| !value.is_empty() && value != "[ENCRYPTED]")
                    })
                    .unwrap_or_default();
                let default_chat_id = settings
                    .imessage_default_chat_id
                    .clone()
                    .or_else(|| {
                        current_imessage
                            .as_ref()
                            .map(|cfg| cfg.default_chat_id.clone())
                    })
                    .unwrap_or_default();
                let default_handle = settings
                    .imessage_default_handle
                    .clone()
                    .or_else(|| {
                        current_imessage
                            .as_ref()
                            .map(|cfg| cfg.default_handle.clone())
                    })
                    .unwrap_or_default();
                if bridge_token.trim().is_empty() {
                    return (
                        StatusCode::BAD_REQUEST,
                        Json(ErrorResponse {
                            error: "iMessage bridge token is required when iMessage is enabled"
                                .to_string(),
                        }),
                    )
                        .into_response();
                }
                if default_chat_id.trim().is_empty() && default_handle.trim().is_empty() {
                    return (
                        StatusCode::BAD_REQUEST,
                        Json(ErrorResponse {
                            error: "iMessage needs a default chat ID or handle when it is enabled"
                                .to_string(),
                        }),
                    )
                        .into_response();
                }
                Some(IMessageChannelConfig {
                    bridge_url: settings
                        .imessage_bridge_url
                        .clone()
                        .or_else(|| current_imessage.as_ref().map(|cfg| cfg.bridge_url.clone()))
                        .unwrap_or_else(|| IMessageChannelConfig::default().bridge_url),
                    bridge_token,
                    default_chat_id,
                    default_handle,
                })
            } else {
                None
            }
        } else {
            agent_guard.config.imessage.clone()
        };

        let wechat_update_requested = settings.wechat_enabled.is_some()
            || settings.wechat_bridge_token.is_some()
            || settings.wechat_bridge_url.is_some()
            || settings.wechat_default_target_id.is_some();
        let current_wechat = agent_guard.config.wechat.clone();
        let new_wechat = if wechat_update_requested {
            let wechat_enabled = settings.wechat_enabled.unwrap_or(
                current_wechat.is_some()
                    || settings.wechat_bridge_token.is_some()
                    || settings.wechat_default_target_id.is_some(),
            );
            if wechat_enabled {
                let bridge_token = settings
                    .wechat_bridge_token
                    .clone()
                    .filter(|value| !value.is_empty() && value != "[ENCRYPTED]")
                    .or_else(|| {
                        current_wechat
                            .as_ref()
                            .map(|cfg| cfg.bridge_token.clone())
                            .filter(|value| !value.is_empty() && value != "[ENCRYPTED]")
                    })
                    .unwrap_or_default();
                let default_target_id = settings
                    .wechat_default_target_id
                    .clone()
                    .or_else(|| {
                        current_wechat
                            .as_ref()
                            .map(|cfg| cfg.default_target_id.clone())
                    })
                    .unwrap_or_default();
                if bridge_token.trim().is_empty() {
                    return (
                        StatusCode::BAD_REQUEST,
                        Json(ErrorResponse {
                            error: "WeChat bridge token is required when WeChat is enabled"
                                .to_string(),
                        }),
                    )
                        .into_response();
                }
                if default_target_id.trim().is_empty() {
                    return (
                        StatusCode::BAD_REQUEST,
                        Json(ErrorResponse {
                            error: "WeChat needs a default target ID when it is enabled"
                                .to_string(),
                        }),
                    )
                        .into_response();
                }
                Some(WeChatChannelConfig {
                    bridge_url: settings
                        .wechat_bridge_url
                        .clone()
                        .or_else(|| current_wechat.as_ref().map(|cfg| cfg.bridge_url.clone()))
                        .unwrap_or_else(|| WeChatChannelConfig::default().bridge_url),
                    bridge_token,
                    default_target_id,
                })
            } else {
                None
            }
        } else {
            agent_guard.config.wechat.clone()
        };

        let qq_update_requested = settings.qq_enabled.is_some()
            || settings.qq_bridge_token.is_some()
            || settings.qq_bridge_url.is_some()
            || settings.qq_default_target_id.is_some();
        let current_qq = agent_guard.config.qq.clone();
        let new_qq = if qq_update_requested {
            let qq_enabled = settings.qq_enabled.unwrap_or(
                current_qq.is_some()
                    || settings.qq_bridge_token.is_some()
                    || settings.qq_default_target_id.is_some(),
            );
            if qq_enabled {
                let bridge_token = settings
                    .qq_bridge_token
                    .clone()
                    .filter(|value| !value.is_empty() && value != "[ENCRYPTED]")
                    .or_else(|| {
                        current_qq
                            .as_ref()
                            .map(|cfg| cfg.bridge_token.clone())
                            .filter(|value| !value.is_empty() && value != "[ENCRYPTED]")
                    })
                    .unwrap_or_default();
                let default_target_id = settings
                    .qq_default_target_id
                    .clone()
                    .or_else(|| current_qq.as_ref().map(|cfg| cfg.default_target_id.clone()))
                    .unwrap_or_default();
                if bridge_token.trim().is_empty() {
                    return (
                        StatusCode::BAD_REQUEST,
                        Json(ErrorResponse {
                            error: "QQ bridge token is required when QQ is enabled".to_string(),
                        }),
                    )
                        .into_response();
                }
                if default_target_id.trim().is_empty() {
                    return (
                        StatusCode::BAD_REQUEST,
                        Json(ErrorResponse {
                            error: "QQ needs a default target ID when it is enabled".to_string(),
                        }),
                    )
                        .into_response();
                }
                Some(QqChannelConfig {
                    bridge_url: settings
                        .qq_bridge_url
                        .clone()
                        .or_else(|| current_qq.as_ref().map(|cfg| cfg.bridge_url.clone()))
                        .unwrap_or_else(|| QqChannelConfig::default().bridge_url),
                    bridge_token,
                    default_target_id,
                })
            } else {
                None
            }
        } else {
            agent_guard.config.qq.clone()
        };

        // Defer network connectivity probing until after lock is released to avoid
        // blocking all agent reads/writes while waiting on upstream APIs.
        if !llm_unchanged {
            llm_connectivity_probe = Some(new_llm.clone());
        }

        // Update model pool routing behavior (doesn't require restart).
        if let Some(v) = settings.smart_routing {
            agent_guard.config.model_pool.smart_routing = v;
        }
        if let Some(mode) = settings.default_model_input_mode.as_ref() {
            let parsed_mode = match parse_model_input_privacy_mode(mode) {
                Ok(parsed) => parsed,
                Err(error) => {
                    return (StatusCode::BAD_REQUEST, Json(ErrorResponse { error }))
                        .into_response();
                }
            };
            agent_guard.config.model_privacy.default_model_input_mode = parsed_mode;
        }
        if let Some(policy) = settings.current_chat_pii_policy.as_ref() {
            let parsed_policy = match parse_current_chat_pii_policy(policy) {
                Ok(parsed) => parsed,
                Err(error) => {
                    return (StatusCode::BAD_REQUEST, Json(ErrorResponse { error }))
                        .into_response();
                }
            };
            agent_guard.config.model_privacy.current_chat_pii_policy = parsed_policy;
        }
        if let Some(enabled) = settings.request_scoped_sensitive_approval_enabled {
            agent_guard
                .config
                .model_privacy
                .request_scoped_sensitive_approval_enabled = enabled;
        }
        if let Some(mode) = settings.deployment_mode.as_ref() {
            let normalized = mode.trim().to_ascii_lowercase();
            let parsed_mode = match normalized.as_str() {
                "" | "trusted_local" | "trusted-local" => DeploymentMode::TrustedLocal,
                "internet_facing" | "internet-facing" => DeploymentMode::InternetFacing,
                _ => {
                    return (
                        StatusCode::BAD_REQUEST,
                        Json(ErrorResponse {
                            error: "deployment_mode must be 'trusted_local' or 'internet_facing'"
                                .to_string(),
                        }),
                    )
                        .into_response();
                }
            };
            if agent_guard.config.deployment_mode != parsed_mode {
                agent_guard.config.deployment_mode = parsed_mode;
                needs_restart = true;
            }
            if parsed_mode == DeploymentMode::InternetFacing
                && agent_guard.config.public_apps.bind_addr.is_none()
            {
                agent_guard.config.public_apps.bind_addr = Some("127.0.0.1:8992".to_string());
                needs_restart = true;
            }
        }
        if let Some(bind_addr) = settings.public_app_bind_addr.as_ref() {
            let normalized = bind_addr.trim();
            let next = if normalized.is_empty() {
                None
            } else {
                Some(normalized.to_string())
            };
            if agent_guard.config.public_apps.bind_addr != next {
                agent_guard.config.public_apps.bind_addr = next;
                needs_restart = true;
            }
        }
        if let Some(base_url) = settings.public_app_base_url.as_ref() {
            let next = normalize_optional_url(Some(base_url.as_str()));
            if agent_guard.config.public_apps.base_url != next {
                agent_guard.config.public_apps.base_url = next;
                needs_restart = true;
            }
        }

        // Update config
        agent_guard.config.llm = new_llm.clone();
        agent_guard.config.llm_fallback = new_llm_fallback;
        agent_guard.config.embeddings = Some(new_embeddings.clone());
        agent_guard.config.telegram = new_telegram.clone();
        agent_guard.config.slack = new_slack.clone();
        agent_guard.config.discord = new_discord.clone();
        agent_guard.config.matrix = new_matrix.clone();
        agent_guard.config.teams = new_teams.clone();
        agent_guard.config.whatsapp = new_whatsapp.clone();
        agent_guard.config.google_chat = new_google_chat.clone();
        agent_guard.config.line = new_line.clone();
        agent_guard.config.signal = new_signal.clone();
        agent_guard.config.imessage = new_imessage.clone();
        agent_guard.config.wechat = new_wechat.clone();
        agent_guard.config.qq = new_qq.clone();

        // Detect if Telegram config changed (needs process restart)
        let new_tg_snapshot = new_telegram
            .as_ref()
            .map(|t| (t.bot_token.clone(), t.allowed_users.clone()));
        if old_telegram != new_tg_snapshot {
            needs_restart = true;
        }

        // Detect WhatsApp config change (managed via bridge process, no full restart needed)
        let old_embedded_bridge = old_whatsapp
            .as_ref()
            .is_some_and(|config| config.uses_embedded_bridge());
        let new_embedded_bridge = new_whatsapp
            .as_ref()
            .is_some_and(|config| config.uses_embedded_bridge());
        if !old_embedded_bridge && new_embedded_bridge {
            wa_start_bridge = true;
        } else if old_embedded_bridge && !new_embedded_bridge {
            wa_stop_bridge = true;
        } else if old_embedded_bridge && new_embedded_bridge {
            let old_wa_snapshot = old_whatsapp.as_ref().map(|w| {
                (
                    w.mode,
                    w.bridge_runtime(),
                    w.bridge_token.clone(),
                    w.verify_token.clone(),
                )
            });
            let new_wa_snapshot = new_whatsapp.as_ref().map(|w| {
                (
                    w.mode,
                    w.bridge_runtime(),
                    w.bridge_token.clone(),
                    w.verify_token.clone(),
                )
            });
            if old_wa_snapshot != new_wa_snapshot {
                wa_restart_bridge = true;
            }
        }

        // Update auto_approve list (with validation - dangerous actions are rejected)
        if let Some(ref list) = settings.auto_approve {
            let (allowed, rejected) =
                crate::core::runtime::config::AgentConfig::validate_auto_approve(list);
            if !rejected.is_empty() {
                tracing::warn!("Rejected auto-approve entries: {:?}", rejected);
            }
            agent_guard.config.auto_approve = allowed.clone();
            // Sync to safety engine so RequireApproval rules are skipped for these actions
            agent_guard.safety.set_auto_approved(&allowed);
            agent_guard.runtime.set_auto_approved_actions(&allowed);
        }

        // Save media provider API keys to config (they will be encrypted by SecureConfigManager)
        for (provider, key) in &settings.media_providers {
            if !key.is_empty() && key != "[ENCRYPTED]" {
                let canonical_provider =
                    crate::integrations::media_gen::MediaProvider::parse(provider)
                        .map(|provider| provider.id().to_string())
                        .unwrap_or_else(|| provider.clone());
                agent_guard
                    .config
                    .media_gen
                    .provider_api_keys
                    .insert(canonical_provider.clone(), key.clone());
                let base_url = agent_guard
                    .config
                    .media_gen
                    .provider_base_urls
                    .get(&canonical_provider)
                    .cloned();
                media_provider_updates.push((canonical_provider, key.clone(), base_url));
            }
        }

        for (provider, base_url) in &settings.media_provider_base_urls {
            let Some(parsed_provider) =
                crate::integrations::media_gen::MediaProvider::parse(provider)
            else {
                return (
                    StatusCode::BAD_REQUEST,
                    Json(ErrorResponse {
                        error: format!("Unknown media provider '{}'", provider),
                    }),
                )
                    .into_response();
            };
            let canonical_provider = parsed_provider.id().to_string();
            let trimmed = base_url.trim();
            if trimmed.is_empty() {
                agent_guard
                    .config
                    .media_gen
                    .provider_base_urls
                    .remove(&canonical_provider);
                continue;
            }
            let parsed_url = match reqwest::Url::parse(trimmed) {
                Ok(url) if matches!(url.scheme(), "http" | "https") => url,
                _ => {
                    return (
                        StatusCode::BAD_REQUEST,
                        Json(ErrorResponse {
                            error: format!(
                                "Media provider endpoint for '{}' must be an http:// or https:// URL",
                                canonical_provider
                            ),
                        }),
                    )
                        .into_response();
                }
            };
            agent_guard.config.media_gen.provider_base_urls.insert(
                canonical_provider,
                parsed_url.as_str().trim_end_matches('/').to_string(),
            );
        }
        for (provider, _, base_url) in &mut media_provider_updates {
            *base_url = agent_guard
                .config
                .media_gen
                .provider_base_urls
                .get(provider)
                .cloned();
        }
        if !settings.media_provider_base_urls.is_empty() {
            let already_queued = media_provider_updates
                .iter()
                .map(|(provider, _, _)| provider.clone())
                .collect::<std::collections::HashSet<_>>();
            for (provider, key) in &agent_guard.config.media_gen.provider_api_keys {
                if key.is_empty() || key == "[ENCRYPTED]" || already_queued.contains(provider) {
                    continue;
                }
                let base_url = agent_guard
                    .config
                    .media_gen
                    .provider_base_urls
                    .get(provider)
                    .cloned();
                media_provider_updates.push((provider.clone(), key.clone(), base_url));
            }
        }

        // Update default/fallback media providers
        if let Some(ref provider) = settings.default_image_provider {
            agent_guard.config.media_gen.default_image_provider = Some(provider.clone());
        }
        if let Some(ref model) = settings.image_model {
            agent_guard.config.media_gen.image_model = Some(model.clone());
        }
        if let Some(ref provider) = settings.fallback_image_provider {
            agent_guard.config.media_gen.fallback_image_provider = Some(provider.clone());
        }
        if let Some(ref provider) = settings.default_video_provider {
            agent_guard.config.media_gen.default_video_provider = Some(provider.clone());
        }
        if let Some(ref provider) = settings.fallback_video_provider {
            agent_guard.config.media_gen.fallback_video_provider = Some(provider.clone());
        }

        let next_embedding_client = match crate::core::EmbeddingClient::from_config(
            &agent_guard.config,
            &agent_guard.data_dir,
        ) {
            Ok(client) => client.map(Arc::new),
            Err(error) => {
                return (
                    StatusCode::INTERNAL_SERVER_ERROR,
                    Json(ErrorResponse {
                        error: format!("Failed to initialize embeddings backend: {}", error),
                    }),
                )
                    .into_response();
            }
        };

        if let Some(observability) = settings.observability.as_ref() {
            if let Some(enabled) = observability.enabled {
                agent_guard.config.observability.enabled = enabled;
            }
            if let Some(provider) = observability.provider.as_ref() {
                agent_guard.config.observability.provider =
                    crate::core::platform::observability::normalize_observability_provider(
                        provider,
                    );
            }
            if let Some(endpoint) = observability.endpoint.as_ref() {
                agent_guard.config.observability.endpoint = endpoint.trim().to_string();
            }
            if let Some(service_name) = observability.service_name.as_ref() {
                agent_guard.config.observability.service_name = service_name.trim().to_string();
            }
            if let Some(header_name) = observability.header_name.as_ref() {
                agent_guard.config.observability.header_name =
                    crate::core::platform::observability::normalize_observability_header_name(
                        header_name,
                    );
            }
            if let Some(privacy_mode) = observability.privacy_mode.as_ref() {
                agent_guard.config.observability.privacy_mode =
                    crate::core::platform::observability::normalize_observability_privacy_mode(
                        privacy_mode,
                    );
            }
        }

        // Runtime media provider syncing is done after lock release.

        if search_provider_order.is_some()
            || search_serper_key.is_some()
            || clear_search_serper_key
            || search_brave_key.is_some()
            || clear_search_brave_key
            || search_exa_key.is_some()
            || clear_search_exa_key
            || search_tavily_key.is_some()
            || clear_search_tavily_key
            || search_perplexity_key.is_some()
            || clear_search_perplexity_key
            || search_firecrawl_key.is_some()
            || clear_search_firecrawl_key
            || search_searxng_base_url.is_some()
        {
            deferred_search_config_dir = Some(agent_guard.config_dir.clone());
        }

        // Save to disk
        let mut save_result = agent_guard
            .config
            .save(&agent_guard.config_dir, Some(&agent_guard.data_dir));

        if save_result.is_ok()
            && (observability_auth_token.is_some()
                || search_serper_key.is_some()
                || clear_search_serper_key
                || search_brave_key.is_some()
                || clear_search_brave_key
                || search_exa_key.is_some()
                || clear_search_exa_key
                || search_tavily_key.is_some()
                || clear_search_tavily_key
                || search_perplexity_key.is_some()
                || clear_search_perplexity_key
                || search_firecrawl_key.is_some()
                || clear_search_firecrawl_key)
        {
            let manager = crate::core::runtime::config::SecureConfigManager::new_with_data_dir(
                &agent_guard.config_dir,
                Some(&agent_guard.data_dir),
            );
            save_result = manager.and_then(|manager| {
                manager.update_custom_secrets(|custom| {
                    if let Some(auth_token) = observability_auth_token.as_ref() {
                        if auth_token.trim().is_empty() {
                            custom.remove(
                                crate::core::platform::observability::OBSERVABILITY_AUTH_TOKEN_SECRET_KEY,
                            );
                        } else {
                            custom.insert(
                                crate::core::platform::observability::OBSERVABILITY_AUTH_TOKEN_SECRET_KEY
                                    .to_string(),
                                auth_token.trim().to_string(),
                            );
                        }
                    }
                    if let Some(api_key) = search_serper_key.as_ref() {
                        if api_key.trim().is_empty() {
                            custom.remove("search_serper_key");
                        } else {
                            custom.insert(
                                "search_serper_key".to_string(),
                                api_key.trim().to_string(),
                            );
                        }
                    } else if clear_search_serper_key {
                        custom.remove("search_serper_key");
                    }
                    if let Some(api_key) = search_brave_key.as_ref() {
                        if api_key.trim().is_empty() {
                            custom.remove("search_brave_key");
                        } else {
                            custom
                                .insert("search_brave_key".to_string(), api_key.trim().to_string());
                        }
                    } else if clear_search_brave_key {
                        custom.remove("search_brave_key");
                    }
                    if let Some(api_key) = search_exa_key.as_ref() {
                        if api_key.trim().is_empty() {
                            custom.remove("search_exa_key");
                        } else {
                            custom.insert("search_exa_key".to_string(), api_key.trim().to_string());
                        }
                    } else if clear_search_exa_key {
                        custom.remove("search_exa_key");
                    }
                    if let Some(api_key) = search_tavily_key.as_ref() {
                        if api_key.trim().is_empty() {
                            custom.remove("search_tavily_key");
                        } else {
                            custom.insert(
                                "search_tavily_key".to_string(),
                                api_key.trim().to_string(),
                            );
                        }
                    } else if clear_search_tavily_key {
                        custom.remove("search_tavily_key");
                    }
                    if let Some(api_key) = search_perplexity_key.as_ref() {
                        if api_key.trim().is_empty() {
                            custom.remove("search_perplexity_key");
                        } else {
                            custom.insert(
                                "search_perplexity_key".to_string(),
                                api_key.trim().to_string(),
                            );
                        }
                    } else if clear_search_perplexity_key {
                        custom.remove("search_perplexity_key");
                    }
                    if let Some(api_key) = search_firecrawl_key.as_ref() {
                        if api_key.trim().is_empty() {
                            custom.remove("search_firecrawl_key");
                        } else {
                            custom.insert(
                                "search_firecrawl_key".to_string(),
                                api_key.trim().to_string(),
                            );
                        }
                    } else if clear_search_firecrawl_key {
                        custom.remove("search_firecrawl_key");
                    }
                    Ok(())
                })
            });
        }

        // Reinitialize LLM client (skip if unchanged / managed by model pool)
        if !llm_unchanged {
            match crate::core::LlmClient::new(&new_llm) {
                Ok(new_client) => {
                    agent_guard.llm = new_client;
                }
                Err(e) => {
                    return (
                        StatusCode::INTERNAL_SERVER_ERROR,
                        Json(ErrorResponse {
                            error: format!("Failed to initialize LLM: {}", e),
                        }),
                    )
                        .into_response();
                }
            }
        }

        agent_guard.embedding_client = next_embedding_client;

        save_result
    };

    if let Some(bytes) = deferred_profile_bytes {
        if let Err(e) = deferred_encrypted_storage
            .set_encrypted("user_profile", &bytes)
            .await
        {
            tracing::warn!("Failed to persist user profile updates: {}", e);
        }
    }
    if let Some(channel) = deferred_daily_brief_channel.as_ref() {
        if let Err(e) = deferred_storage
            .set(DAILY_BRIEF_CHANNEL_KEY, channel.as_bytes())
            .await
        {
            tracing::warn!("Failed to persist daily brief channel: {}", e);
        }
    }
    if let Some(enabled) = deferred_daily_brief_enabled {
        let stored_value = if enabled { "true" } else { "false" };
        if let Err(e) = deferred_storage
            .set(DAILY_BRIEF_ENABLED_KEY, stored_value.as_bytes())
            .await
        {
            tracing::warn!("Failed to persist daily brief enabled flag: {}", e);
        }
    }
    if let Some(time_value) = deferred_daily_brief_time.as_ref() {
        if let Err(e) = deferred_storage
            .set(DAILY_BRIEF_TIME_KEY, time_value.as_bytes())
            .await
        {
            tracing::warn!("Failed to persist daily brief time: {}", e);
        }
    }
    if let Some(enabled) = deferred_arkreflect_daily_digest_enabled {
        let stored_value = if enabled { "true" } else { "false" };
        if let Err(e) = deferred_storage
            .set(ARKREFLECT_DAILY_DIGEST_ENABLED_KEY, stored_value.as_bytes())
            .await
        {
            tracing::warn!("Failed to persist Reflect daily digest setting: {}", e);
        }
    }

    if let Some(config_dir) = deferred_search_config_dir.as_ref() {
        let mut search_config =
            crate::runtime::load_persisted_search_config_async(config_dir.clone(), None).await;

        if let Some(key) = &search_serper_key {
            search_config.serper = if key.trim().is_empty() {
                None
            } else {
                Some(crate::actions::SearchBackend::Serper {
                    api_key: String::new(),
                })
            };
        } else if clear_search_serper_key {
            search_config.serper = None;
        }
        if let Some(key) = &search_brave_key {
            search_config.brave = if key.trim().is_empty() {
                None
            } else {
                Some(crate::actions::SearchBackend::Brave {
                    api_key: String::new(),
                })
            };
        } else if clear_search_brave_key {
            search_config.brave = None;
        }
        if let Some(key) = &search_exa_key {
            search_config.exa = if key.trim().is_empty() {
                None
            } else {
                Some(crate::actions::SearchBackend::Exa {
                    api_key: String::new(),
                })
            };
        } else if clear_search_exa_key {
            search_config.exa = None;
        }
        if let Some(key) = &search_tavily_key {
            search_config.tavily = if key.trim().is_empty() {
                None
            } else {
                Some(crate::actions::SearchBackend::Tavily {
                    api_key: String::new(),
                })
            };
        } else if clear_search_tavily_key {
            search_config.tavily = None;
        }
        if let Some(key) = &search_perplexity_key {
            search_config.perplexity = if key.trim().is_empty() {
                None
            } else {
                Some(crate::actions::SearchBackend::Perplexity {
                    api_key: String::new(),
                })
            };
        } else if clear_search_perplexity_key {
            search_config.perplexity = None;
        }
        if let Some(key) = &search_firecrawl_key {
            search_config.firecrawl = if key.trim().is_empty() {
                None
            } else {
                Some(crate::actions::SearchBackend::Firecrawl {
                    api_key: String::new(),
                })
            };
        } else if clear_search_firecrawl_key {
            search_config.firecrawl = None;
        }
        if let Some(base_url) = &search_searxng_base_url {
            let normalized = base_url.trim().trim_end_matches('/').to_string();
            search_config.searxng = if normalized.is_empty() {
                None
            } else {
                Some(crate::actions::SearchBackend::Searxng {
                    base_url: normalized,
                })
            };
        }

        if let Some(order) = &search_provider_order {
            search_config.provider_order = order
                .iter()
                .filter_map(|value| {
                    let normalized = value.trim().to_ascii_lowercase();
                    if normalized.is_empty() {
                        None
                    } else {
                        Some(normalized)
                    }
                })
                .collect();
        }

        search_config.primary = None;
        search_config.fallback1 = None;
        search_config.fallback2 = None;
        search_config.ensure_default_chain();

        if let Err(e) = crate::runtime::save_persisted_search_config_async(
            config_dir.clone(),
            None,
            search_config,
        )
        .await
        {
            tracing::warn!("Failed to save search config: {}", e);
        }
    }

    if let Some(data_lifecycle_cfg) = deferred_data_lifecycle_settings.as_ref() {
        if let Err(e) = save_data_lifecycle_settings(&deferred_storage, data_lifecycle_cfg).await {
            tracing::warn!("Failed to persist data lifecycle settings: {}", e);
        }
    }

    match result {
        Ok(_) => {
            if !media_provider_updates.is_empty() {
                let agent_ref = state.agent.clone();
                let updates = media_provider_updates.clone();
                crate::spawn_logged!("src/channels/http.rs:27564", async move {
                    let agent = agent_ref.read().await;
                    if let Some(media_gen) = agent.integrations.get("media_gen") {
                        for (provider, api_key, base_url) in updates {
                            let mut payload = serde_json::json!({
                                "provider": provider,
                                "api_key": api_key
                            });
                            if let Some(base_url) = base_url
                                .as_deref()
                                .map(str::trim)
                                .filter(|value| !value.is_empty())
                            {
                                payload["base_url"] =
                                    serde_json::Value::String(base_url.to_string());
                            }
                            match tokio::time::timeout(
                                std::time::Duration::from_secs(3),
                                media_gen.execute("configure_provider", &payload),
                            )
                            .await
                            {
                                Ok(Ok(_)) => {}
                                Ok(Err(e)) => tracing::warn!(
                                    "Failed to sync media provider config to runtime: {}",
                                    e
                                ),
                                Err(_) => tracing::warn!(
                                    "Timed out syncing media provider config to runtime"
                                ),
                            }
                        }
                    }
                });
            }

            if let Some(provider) = llm_connectivity_probe {
                crate::spawn_logged!("src/channels/http.rs:27593", async move {
                    if let Err(e) = test_llm_connection(&provider).await {
                        tracing::warn!("LLM provider connectivity probe failed after save: {}", e);
                    }
                });
            }

            for task in &existing_daily_brief_tasks {
                if let Err(e) = deferred_storage.delete_task(&task.id.to_string()).await {
                    tracing::warn!(
                        "Failed to delete previous daily brief task {}: {}",
                        task.id,
                        e
                    );
                }
            }
            {
                let mut queue = state.tasks.write().await;
                for task in &existing_daily_brief_tasks {
                    queue.remove(task.id);
                }
            }
            if requested_daily_brief_enabled {
                let Some(daily_brief_cron) =
                    daily_brief_cron_from_time(&requested_daily_brief_time)
                else {
                    return (
                        StatusCode::INTERNAL_SERVER_ERROR,
                        Json(ErrorResponse {
                            error: "Failed to build the daily brief schedule".to_string(),
                        }),
                    )
                        .into_response();
                };
                let mut task = Task::new(
                    "Morning summary brief".to_string(),
                    "daily_brief".to_string(),
                    serde_json::json!({ "report_to": requested_daily_brief_channel.clone() }),
                );
                task.capabilities = vec!["daily_brief".to_string()];
                task.cron = Some(daily_brief_cron);
                if let Err(e) = deferred_storage.insert_task(&task).await {
                    return (
                        StatusCode::INTERNAL_SERVER_ERROR,
                        Json(ErrorResponse {
                            error: format!("Failed to save daily brief schedule: {}", e),
                        }),
                    )
                        .into_response();
                }
                let mut queue = state.tasks.write().await;
                queue.add(task);
            }

            // Handle WhatsApp bridge lifecycle (no full process restart needed)
            if wa_start_bridge || wa_restart_bridge {
                let state_for_bridge = state.clone();
                let wb = state.whatsapp_bridge.clone();
                crate::spawn_logged!("src/channels/http.rs:27651", async move {
                    if wa_restart_bridge {
                        stop_whatsapp_bridge(wb.clone()).await;
                        tokio::time::sleep(std::time::Duration::from_secs(1)).await;
                    }
                    tracing::info!("Starting WhatsApp bridge (user enabled in settings)...");
                    match spawn_whatsapp_bridge(state_for_bridge).await {
                        Ok(()) => tracing::info!("WhatsApp bridge started successfully"),
                        Err(e) => tracing::error!("Failed to start WhatsApp bridge: {}", e),
                    }
                });
            } else if wa_stop_bridge {
                let wb = state.whatsapp_bridge.clone();
                crate::spawn_logged!("src/channels/http.rs:27664", async move {
                    tracing::info!("Stopping WhatsApp bridge (user disabled in settings)...");
                    stop_whatsapp_bridge(wb).await;
                });
            }

            if needs_restart {
                let active_chat_streams = active_chat_stream_count_for_restart(&state).await;
                let restart_queued =
                    schedule_process_restart_after_chat_idle(state.clone(), "settings_restart");
                let message = if active_chat_streams > 0 {
                    "Settings saved. Restart will apply after active chat streams finish."
                } else if restart_queued {
                    "Settings saved. Restarting to apply channel changes..."
                } else {
                    "Settings saved. Restart is already pending."
                };
                (
                    StatusCode::OK,
                    Json(serde_json::json!({
                        "status": "ok",
                        "message": message,
                        "restart_scheduled": true,
                        "restart_deferred": active_chat_streams > 0,
                        "active_chat_streams": active_chat_streams
                    })),
                )
                    .into_response()
            } else {
                let msg = if wa_start_bridge {
                    "Settings saved. WhatsApp bridge starting..."
                } else if wa_stop_bridge {
                    "Settings saved. WhatsApp bridge stopped."
                } else if wa_restart_bridge {
                    "Settings saved. WhatsApp bridge restarting..."
                } else {
                    "Settings saved"
                };
                (
                    StatusCode::OK,
                    Json(serde_json::json!({"status": "ok", "message": msg})),
                )
                    .into_response()
            }
        }
        Err(e) => (
            StatusCode::INTERNAL_SERVER_ERROR,
            Json(ErrorResponse {
                error: format!("Failed to save settings: {}", e),
            }),
        )
            .into_response(),
    }
}

pub(super) async fn test_llm_connection(provider: &LlmProvider) -> Result<(), String> {
    let client = reqwest::Client::builder()
        .timeout(std::time::Duration::from_secs(20))
        .build()
        .map_err(|e| format!("Failed to build HTTP client: {}", e))?;

    match provider {
        LlmProvider::Ollama { base_url, model } => {
            let url = format!("{}/api/chat", base_url.trim_end_matches('/'));
            let payload = serde_json::json!({
                "model": model,
                "messages": [
                    {
                        "role": "system",
                        "content": "You are checking whether this model endpoint is reachable. Reply with OK."
                    },
                    {
                        "role": "user",
                        "content": "Connection check"
                    }
                ],
                "stream": false,
                "options": {
                    "num_predict": 1
                }
            });
            let resp = client
                .post(url)
                .header("Content-Type", "application/json")
                .json(&payload)
                .send()
                .await
                .map_err(|e| e.to_string())?;
            if resp.status().is_success() {
                Ok(())
            } else {
                let status = resp.status();
                let body = resp.text().await.unwrap_or_default();
                Err(format!(
                    "Ollama model check returned {}: {}",
                    status,
                    body.trim()
                ))
            }
        }
        LlmProvider::OpenAI {
            api_key,
            base_url,
            model,
        } => {
            let mut request_config =
                resolve_openai_request_config(&client, api_key, base_url.as_deref(), model)
                    .await
                    .map_err(|e| e.to_string())?;
            if request_config.uses_codex_cli_oauth {
                let endpoint = format!(
                    "{}/responses",
                    request_config.base_url.trim_end_matches('/')
                );
                let payload = serde_json::json!({
                    "model": model,
                    "instructions": "You are checking whether this model endpoint is reachable. Reply with OK.",
                    "input": [{
                        "type": "message",
                        "role": "user",
                        "content": [{ "type": "input_text", "text": "Connection check" }]
                    }],
                    "stream": true,
                    "store": false,
                });
                let mut resp = client
                    .post(&endpoint)
                    .bearer_auth(&request_config.api_key)
                    .header("Content-Type", "application/json")
                    .header("Accept", "text/event-stream")
                    .json(&payload)
                    .send()
                    .await
                    .map_err(|e| e.to_string())?;
                if resp.status() == reqwest::StatusCode::UNAUTHORIZED {
                    let refreshed = force_refresh_codex_cli_api_key(&client)
                        .await
                        .map_err(|e| e.to_string())?
                        .unwrap_or_default();
                    request_config.api_key = refreshed;
                    resp = client
                        .post(&endpoint)
                        .bearer_auth(&request_config.api_key)
                        .header("Content-Type", "application/json")
                        .header("Accept", "text/event-stream")
                        .json(&payload)
                        .send()
                        .await
                        .map_err(|e| e.to_string())?;
                }
                if resp.status().is_success() {
                    return Ok(());
                }
                let status = resp.status();
                let body = resp.text().await.unwrap_or_default();
                return Err(format!(
                    "{} returned {}: {}",
                    request_config.provider_label,
                    status,
                    body.trim()
                ));
            }
            let base = request_config.base_url.trim_end_matches('/');
            let url = format!("{}/chat/completions", base);
            let payload = serde_json::json!({
                "model": model,
                "messages": [
                    {
                        "role": "system",
                        "content": "You are checking whether this model endpoint is reachable. Reply with OK."
                    },
                    {
                        "role": "user",
                        "content": "Connection check"
                    }
                ],
                "stream": false
            });
            let mut request = client
                .post(url)
                .header("Content-Type", "application/json")
                .json(&payload);
            if !request_config.api_key.trim().is_empty() {
                request = request.bearer_auth(&request_config.api_key);
            }
            if request_config.is_openrouter {
                request = request
                    .header("HTTP-Referer", crate::branding::REPOSITORY_URL)
                    .header("X-Title", crate::branding::PRODUCT_NAME);
            }
            let resp = request.send().await.map_err(|e| e.to_string())?;
            if resp.status().is_success() {
                Ok(())
            } else {
                let status = resp.status();
                let body = resp.text().await.unwrap_or_default();
                Err(format!(
                    "{} model check returned {}: {}",
                    request_config.provider_label,
                    status,
                    body.trim()
                ))
            }
        }
        LlmProvider::Anthropic { api_key, model } => {
            let url = "https://api.anthropic.com/v1/messages";
            let payload = serde_json::json!({
                "model": model,
                "max_tokens": 1,
                "system": "You are checking whether this model endpoint is reachable. Reply with OK.",
                "messages": [
                    {
                        "role": "user",
                        "content": "Connection check"
                    }
                ]
            });
            let resp = client
                .post(url)
                .header("x-api-key", api_key)
                .header("anthropic-version", "2023-06-01")
                .header("Content-Type", "application/json")
                .json(&payload)
                .send()
                .await
                .map_err(|e| e.to_string())?;
            if resp.status().is_success() {
                Ok(())
            } else {
                let status = resp.status();
                let body = resp.text().await.unwrap_or_default();
                Err(format!(
                    "Anthropic model check returned {}: {}",
                    status,
                    body.trim()
                ))
            }
        }
    }
}

pub(super) async fn trigger_runtime_restart(
    state: &AppState,
) -> std::result::Result<serde_json::Value, (StatusCode, serde_json::Value)> {
    if matches!(stack_role().as_deref(), Some("control-plane" | "control")) {
        tracing::info!("Restart requested via API - delegating split-stack restart to executor");
        let executor_client = state
            .executor_client
            .clone()
            .or_else(|| build_executor_client().ok().flatten());
        let Some(executor_client) = executor_client else {
            return Err((
                StatusCode::SERVICE_UNAVAILABLE,
                serde_json::json!({
                    "status": "error",
                    "message": "Internal executor client is unavailable for split-stack restart."
                }),
            ));
        };

        match executor_client
            .request(reqwest::Method::POST, "/internal/v1/system/restart-stack")
            .send()
            .await
        {
            Ok(response) => {
                let status = response.status();
                let payload = response
                    .json::<serde_json::Value>()
                    .await
                    .unwrap_or_else(|_| serde_json::json!({}));
                if !status.is_success() {
                    return Err((
                        StatusCode::from_u16(status.as_u16()).unwrap_or(StatusCode::BAD_GATEWAY),
                        serde_json::json!({
                            "status": "error",
                            "message": payload.get("message").and_then(|value| value.as_str()).unwrap_or("Failed to restart AgentArk services."),
                            "details": payload
                        }),
                    ));
                }

                return Ok(serde_json::json!({
                    "status": payload.get("status").and_then(|value| value.as_str()).unwrap_or("restarting"),
                    "message": payload.get("message").and_then(|value| value.as_str()).unwrap_or("AgentArk services are restarting."),
                    "services": payload.get("services").cloned().unwrap_or_else(|| serde_json::json!([]))
                }));
            }
            Err(error) => {
                return Err((
                    StatusCode::BAD_GATEWAY,
                    serde_json::json!({
                        "status": "error",
                        "message": format!("Failed to reach executor for split-stack restart: {}", error)
                    }),
                ));
            }
        }
    }

    tracing::info!("Restart requested via API - shutting down local server process");
    crate::spawn_logged!("src/channels/http.rs:27910", async {
        tokio::time::sleep(tokio::time::Duration::from_millis(500)).await;
        std::process::exit(0);
        #[allow(unreachable_code)]
        Ok::<(), anyhow::Error>(())
    });

    Ok(serde_json::json!({
        "status": "ok",
        "message": "Server is restarting..."
    }))
}

/// Restart the server or split-stack services.
pub(super) async fn restart_server(State(state): State<AppState>) -> Response {
    match trigger_runtime_restart(&state).await {
        Ok(payload) => (StatusCode::OK, Json(payload)).into_response(),
        Err((status, payload)) => (status, Json(payload)).into_response(),
    }
}

/// Update to the latest tagged release and restart the managed Docker stack.
pub(super) async fn update_server(State(state): State<AppState>) -> Response {
    let summary = current_release_update_summary(&state).await;
    if summary.state != "available" {
        let message = match summary.state.as_str() {
            "current" => "AgentArk is already on the latest release.",
            "unavailable" => "Update status is unavailable for this deployment.",
            _ => "AgentArk is still checking for updates.",
        };
        return (
            StatusCode::SERVICE_UNAVAILABLE,
            Json(serde_json::json!({
                "status": "error",
                "message": message
            })),
        )
            .into_response();
    }

    if !summary.apply_supported {
        return (
            StatusCode::BAD_REQUEST,
            Json(serde_json::json!({
                "status": "error",
                "message": summary
                    .apply_message
                    .clone()
                    .unwrap_or_else(|| "Web UI updates are unavailable for this deployment.".to_string())
            })),
        )
            .into_response();
    }

    let Some(release_tag) = summary.latest_tag.clone() else {
        return (
            StatusCode::SERVICE_UNAVAILABLE,
            Json(serde_json::json!({
                "status": "error",
                "message": "Latest release metadata is incomplete."
            })),
        )
            .into_response();
    };
    let Some(release_version) = summary.latest_version.clone() else {
        return (
            StatusCode::SERVICE_UNAVAILABLE,
            Json(serde_json::json!({
                "status": "error",
                "message": "Latest release version metadata is incomplete."
            })),
        )
            .into_response();
    };

    let executor_client = state
        .executor_client
        .clone()
        .or_else(|| build_executor_client().ok().flatten());
    let Some(executor_client) = executor_client else {
        return (
            StatusCode::SERVICE_UNAVAILABLE,
            Json(serde_json::json!({
                "status": "error",
                "message": "Internal executor client is unavailable for updates."
            })),
        )
            .into_response();
    };

    let request = crate::executor::protocol::StackUpdateRequest {
        release_tag: release_tag.clone(),
        release_version: release_version.clone(),
        release_repo: crate::core::runtime::release_updates::release_repo_slug(),
        image_repository: crate::core::runtime::release_updates::runtime_image_repository(),
    };

    match executor_client
        .request(reqwest::Method::POST, "/internal/v1/system/update-stack")
        .json(&request)
        .send()
        .await
    {
        Ok(response) => {
            let status = response.status();
            let payload = response
                .json::<serde_json::Value>()
                .await
                .unwrap_or_else(|_| serde_json::json!({}));
            if !status.is_success() {
                return (
                    StatusCode::from_u16(status.as_u16()).unwrap_or(StatusCode::BAD_GATEWAY),
                    Json(serde_json::json!({
                        "status": "error",
                        "message": payload
                            .get("message")
                            .and_then(|value| value.as_str())
                            .unwrap_or("Failed to start the AgentArk update."),
                        "details": payload
                    })),
                )
                    .into_response();
            }

            (
                StatusCode::OK,
                Json(serde_json::json!({
                    "status": payload
                        .get("status")
                        .and_then(|value| value.as_str())
                        .unwrap_or("updating"),
                    "message": payload
                        .get("message")
                        .and_then(|value| value.as_str())
                        .unwrap_or("AgentArk is updating and restarting."),
                    "release_tag": payload
                        .get("release_tag")
                        .and_then(|value| value.as_str())
                        .unwrap_or(release_tag.as_str()),
                    "release_version": payload
                        .get("release_version")
                        .and_then(|value| value.as_str())
                        .unwrap_or(release_version.as_str())
                })),
            )
                .into_response()
        }
        Err(error) => (
            StatusCode::BAD_GATEWAY,
            Json(serde_json::json!({
                "status": "error",
                "message": format!("Failed to reach executor for update: {}", error)
            })),
        )
            .into_response(),
    }
}

pub(super) async fn rotate_internal_service_tokens(State(state): State<AppState>) -> Response {
    let config_dir = { state.agent.read().await.config_dir.clone() };
    let token_status =
        match crate::clients::describe_internal_service_tokens_async(&config_dir).await {
            Ok(value) => value,
            Err(error) => {
                return (
                    StatusCode::INTERNAL_SERVER_ERROR,
                    Json(serde_json::json!({
                        "status": "error",
                        "message": format!("Failed to inspect internal credentials: {}", error)
                    })),
                )
                    .into_response();
            }
        };

    if token_status.iter().any(|item| item.managed_by_env) {
        let managed_by_env = token_status
            .iter()
            .filter(|item| item.managed_by_env)
            .map(|item| format!("{} ({})", item.label, item.env_var))
            .collect::<Vec<_>>()
            .join(", ");
        return (
            StatusCode::BAD_REQUEST,
            Json(serde_json::json!({
                "status": "error",
                "message": format!(
                    "Internal credential rotation is managed by environment variables for {}. Update the deployment environment and restart AgentArk instead.",
                    managed_by_env
                )
            })),
        )
            .into_response();
    }

    let previous_executor = match crate::clients::read_persisted_internal_service_token_async(
        &config_dir,
        crate::clients::InternalServiceKind::Executor,
    )
    .await
    {
        Ok(value) => value,
        Err(error) => {
            return (
                StatusCode::INTERNAL_SERVER_ERROR,
                Json(serde_json::json!({
                    "status": "error",
                    "message": format!("Failed to read current executor credential: {}", error)
                })),
            )
                .into_response();
        }
    };
    let previous_workspace = match crate::clients::read_persisted_internal_service_token_async(
        &config_dir,
        crate::clients::InternalServiceKind::Workspace,
    )
    .await
    {
        Ok(value) => value,
        Err(error) => {
            return (
                StatusCode::INTERNAL_SERVER_ERROR,
                Json(serde_json::json!({
                    "status": "error",
                    "message": format!("Failed to read current workspace credential: {}", error)
                })),
            )
                .into_response();
        }
    };

    let executor_rotation = crate::clients::rotate_internal_service_token_async(
        &config_dir,
        crate::clients::InternalServiceKind::Executor,
    )
    .await;
    if let Err(error) = executor_rotation {
        return (
            StatusCode::INTERNAL_SERVER_ERROR,
            Json(serde_json::json!({
                "status": "error",
                "message": format!("Failed to rotate executor credential: {}", error)
            })),
        )
            .into_response();
    }

    let workspace_rotation = crate::clients::rotate_internal_service_token_async(
        &config_dir,
        crate::clients::InternalServiceKind::Workspace,
    )
    .await;
    if let Err(error) = workspace_rotation {
        let restore_result = crate::clients::restore_internal_service_token_async(
            &config_dir,
            crate::clients::InternalServiceKind::Executor,
            previous_executor.as_deref(),
        )
        .await;
        if let Err(restore_error) = restore_result {
            tracing::error!(
                "Failed to roll back executor credential after workspace rotation failure: {}",
                restore_error
            );
        }
        return (
            StatusCode::INTERNAL_SERVER_ERROR,
            Json(serde_json::json!({
                "status": "error",
                "message": format!("Failed to rotate workspace credential: {}", error)
            })),
        )
            .into_response();
    }

    match trigger_runtime_restart(&state).await {
        Ok(mut payload) => {
            payload["message"] = serde_json::Value::String(
                "Internal executor and workspace credentials rotated. AgentArk is restarting to apply them."
                    .to_string(),
            );
            payload["rotated_services"] = serde_json::json!(["executor", "workspace"]);
            spawn_security_log(
                state.agent.clone(),
                "security_rotation",
                "medium",
                "Rotated internal executor and workspace credentials".to_string(),
                Some("scope=internal_services".to_string()),
            );
            (StatusCode::OK, Json(payload)).into_response()
        }
        Err((status, payload)) => {
            for (service, previous) in [
                (
                    crate::clients::InternalServiceKind::Executor,
                    previous_executor.as_deref(),
                ),
                (
                    crate::clients::InternalServiceKind::Workspace,
                    previous_workspace.as_deref(),
                ),
            ] {
                let restore_result = crate::clients::restore_internal_service_token_async(
                    &config_dir,
                    service,
                    previous,
                )
                .await;
                if let Err(error) = restore_result {
                    tracing::error!(
                        "Failed to roll back {} credential after restart delegation failure: {}",
                        service.label(),
                        error
                    );
                }
            }
            spawn_security_log(
                state.agent.clone(),
                "security_rotation_failed",
                "high",
                "Failed to restart AgentArk after rotating internal credentials".to_string(),
                None,
            );
            (status, Json(payload)).into_response()
        }
    }
}

// ============================================================================
// OAuth & Integrations
// ============================================================================

/// Shared form field metadata for integration and tunnel settings.
#[derive(Debug, Serialize)]
pub struct IntegrationConfigField {
    pub key: String,
    pub label: String,
    /// "text" | "password" | "textarea" | "select"
    pub input_type: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub placeholder: Option<String>,
    pub required: bool,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub options: Option<Vec<String>>,
}

// ==================== SSH API ====================

#[cfg(not(feature = "ssh"))]
fn ssh_feature_unavailable_response() -> Response {
    (
        StatusCode::NOT_IMPLEMENTED,
        Json(ErrorResponse {
            error: "SSH support is disabled in this build".to_string(),
        }),
    )
        .into_response()
}

#[cfg(feature = "ssh")]
pub(super) async fn ssh_list_connections(State(state): State<AppState>) -> Response {
    let config_dir = { state.agent.read().await.config_dir.clone() };
    match crate::actions::ssh::ssh_list_connections(&config_dir).await {
        Ok(text) => (
            StatusCode::OK,
            Json(serde_json::json!({ "connections": text })),
        )
            .into_response(),
        Err(e) => (
            StatusCode::INTERNAL_SERVER_ERROR,
            Json(ErrorResponse {
                error: e.to_string(),
            }),
        )
            .into_response(),
    }
}

#[cfg(not(feature = "ssh"))]
pub(super) async fn ssh_list_connections(State(_state): State<AppState>) -> Response {
    ssh_feature_unavailable_response()
}

#[cfg(feature = "ssh")]
pub(super) async fn ssh_add_connection(
    State(state): State<AppState>,
    Json(request): Json<serde_json::Value>,
) -> Response {
    let config_dir = { state.agent.read().await.config_dir.clone() };

    let name = match request.get("name").and_then(|v| v.as_str()) {
        Some(v) if !v.is_empty() => v.to_string(),
        _ => {
            return (
                StatusCode::BAD_REQUEST,
                Json(ErrorResponse {
                    error: "Missing connection name".to_string(),
                }),
            )
                .into_response();
        }
    };
    let host = match request.get("host").and_then(|v| v.as_str()) {
        Some(v) if !v.is_empty() => v.to_string(),
        _ => {
            return (
                StatusCode::BAD_REQUEST,
                Json(ErrorResponse {
                    error: "Missing host".to_string(),
                }),
            )
                .into_response();
        }
    };
    let port = request.get("port").and_then(|v| v.as_u64()).unwrap_or(22) as u16;
    let username = match request.get("username").and_then(|v| v.as_str()) {
        Some(v) if !v.is_empty() => v.to_string(),
        _ => {
            return (
                StatusCode::BAD_REQUEST,
                Json(ErrorResponse {
                    error: "Missing username".to_string(),
                }),
            )
                .into_response();
        }
    };
    let key_name = match request.get("key_name").and_then(|v| v.as_str()) {
        Some(v) if !v.is_empty() => v.to_string(),
        _ => {
            return (
                StatusCode::BAD_REQUEST,
                Json(ErrorResponse {
                    error: "Missing key_name".to_string(),
                }),
            )
                .into_response();
        }
    };

    let conn = crate::actions::ssh::SshConnection {
        name: name.clone(),
        host,
        port,
        username,
        key_name,
    };

    match crate::actions::ssh::add_connection(&config_dir, conn) {
        Ok(_) => (
            StatusCode::OK,
            Json(serde_json::json!({ "status": "ok", "name": name })),
        )
            .into_response(),
        Err(e) => (
            StatusCode::INTERNAL_SERVER_ERROR,
            Json(ErrorResponse {
                error: e.to_string(),
            }),
        )
            .into_response(),
    }
}

#[cfg(not(feature = "ssh"))]
pub(super) async fn ssh_add_connection(
    State(_state): State<AppState>,
    Json(_request): Json<serde_json::Value>,
) -> Response {
    ssh_feature_unavailable_response()
}

#[cfg(feature = "ssh")]
pub(super) async fn ssh_remove_connection(
    State(state): State<AppState>,
    axum::extract::Path(name): axum::extract::Path<String>,
) -> Response {
    let config_dir = { state.agent.read().await.config_dir.clone() };
    match crate::actions::ssh::remove_connection(&config_dir, &name) {
        Ok(true) => (
            StatusCode::OK,
            Json(serde_json::json!({ "status": "removed" })),
        )
            .into_response(),
        Ok(false) => (
            StatusCode::NOT_FOUND,
            Json(ErrorResponse {
                error: format!("Connection '{}' not found", name),
            }),
        )
            .into_response(),
        Err(e) => (
            StatusCode::INTERNAL_SERVER_ERROR,
            Json(ErrorResponse {
                error: e.to_string(),
            }),
        )
            .into_response(),
    }
}

#[cfg(not(feature = "ssh"))]
pub(super) async fn ssh_remove_connection(
    State(_state): State<AppState>,
    axum::extract::Path(_name): axum::extract::Path<String>,
) -> Response {
    ssh_feature_unavailable_response()
}

#[cfg(feature = "ssh")]
pub(super) async fn ssh_list_keys(State(state): State<AppState>) -> Response {
    let config_dir = { state.agent.read().await.config_dir.clone() };
    match crate::actions::ssh::list_key_names(&config_dir) {
        Ok(keys) => (StatusCode::OK, Json(serde_json::json!({ "keys": keys }))).into_response(),
        Err(e) => (
            StatusCode::INTERNAL_SERVER_ERROR,
            Json(ErrorResponse {
                error: e.to_string(),
            }),
        )
            .into_response(),
    }
}

#[cfg(not(feature = "ssh"))]
pub(super) async fn ssh_list_keys(State(_state): State<AppState>) -> Response {
    ssh_feature_unavailable_response()
}

#[cfg(feature = "ssh")]
pub(super) async fn ssh_upload_key(
    State(state): State<AppState>,
    Json(request): Json<serde_json::Value>,
) -> Response {
    let config_dir = { state.agent.read().await.config_dir.clone() };

    let name = match request.get("name").and_then(|v| v.as_str()) {
        Some(v) if !v.is_empty() => v.to_string(),
        _ => {
            return (
                StatusCode::BAD_REQUEST,
                Json(ErrorResponse {
                    error: "Missing key name".to_string(),
                }),
            )
                .into_response();
        }
    };
    let pem_content = match request.get("pem_content").and_then(|v| v.as_str()) {
        Some(v) if !v.is_empty() => v.to_string(),
        _ => {
            return (
                StatusCode::BAD_REQUEST,
                Json(ErrorResponse {
                    error: "Missing pem_content".to_string(),
                }),
            )
                .into_response();
        }
    };

    if let Err(e) = crate::actions::ssh::validate_private_key_pem(&name, &pem_content) {
        return (
            StatusCode::BAD_REQUEST,
            Json(ErrorResponse {
                error: e.to_string(),
            }),
        )
            .into_response();
    }

    match crate::actions::ssh::store_key(&config_dir, &name, &pem_content) {
        Ok(_) => (
            StatusCode::OK,
            Json(serde_json::json!({ "status": "ok", "name": name })),
        )
            .into_response(),
        Err(e) => (
            StatusCode::INTERNAL_SERVER_ERROR,
            Json(ErrorResponse {
                error: e.to_string(),
            }),
        )
            .into_response(),
    }
}

#[cfg(not(feature = "ssh"))]
pub(super) async fn ssh_upload_key(
    State(_state): State<AppState>,
    Json(_request): Json<serde_json::Value>,
) -> Response {
    ssh_feature_unavailable_response()
}

#[cfg(feature = "ssh")]
pub(super) async fn ssh_remove_key(
    State(state): State<AppState>,
    axum::extract::Path(name): axum::extract::Path<String>,
) -> Response {
    let config_dir = { state.agent.read().await.config_dir.clone() };
    match crate::actions::ssh::remove_key(&config_dir, &name) {
        Ok(true) => (
            StatusCode::OK,
            Json(serde_json::json!({ "status": "removed" })),
        )
            .into_response(),
        Ok(false) => (
            StatusCode::NOT_FOUND,
            Json(ErrorResponse {
                error: format!("Key '{}' not found", name),
            }),
        )
            .into_response(),
        Err(e) => (
            StatusCode::INTERNAL_SERVER_ERROR,
            Json(ErrorResponse {
                error: e.to_string(),
            }),
        )
            .into_response(),
    }
}

#[cfg(not(feature = "ssh"))]
pub(super) async fn ssh_remove_key(
    State(_state): State<AppState>,
    axum::extract::Path(_name): axum::extract::Path<String>,
) -> Response {
    ssh_feature_unavailable_response()
}

#[cfg(feature = "ssh")]
pub(super) async fn ssh_test_connection(
    State(state): State<AppState>,
    Json(request): Json<serde_json::Value>,
) -> Response {
    let config_dir = { state.agent.read().await.config_dir.clone() };
    let args = serde_json::json!({
        "connection": request.get("connection").and_then(|v| v.as_str()).unwrap_or(""),
        "command": "echo ok"
    });
    match crate::actions::ssh::ssh_execute(&config_dir, &args).await {
        Ok(output) => (
            StatusCode::OK,
            Json(serde_json::json!({ "status": "ok", "output": output })),
        )
            .into_response(),
        Err(e) => (
            StatusCode::BAD_REQUEST,
            Json(ErrorResponse {
                error: e.to_string(),
            }),
        )
            .into_response(),
    }
}

#[cfg(not(feature = "ssh"))]
pub(super) async fn ssh_test_connection(
    State(_state): State<AppState>,
    Json(_request): Json<serde_json::Value>,
) -> Response {
    ssh_feature_unavailable_response()
}

// ==================== Model Pool API ====================

/// List all model pool slots
pub(super) async fn list_models(State(state): State<AppState>) -> Response {
    let agent = state.agent.read().await;
    let slots: Vec<ModelSlotSummary> = agent
        .config
        .model_pool
        .slots
        .iter()
        .enumerate()
        .map(|(idx, slot)| {
            let (prov, mdl, burl, has_key) = match &slot.provider {
                LlmProvider::Ollama { base_url, model } => (
                    "ollama".to_string(),
                    model.clone(),
                    Some(base_url.clone()),
                    false,
                ),
                LlmProvider::Anthropic { api_key, model } => (
                    "anthropic".to_string(),
                    model.clone(),
                    None,
                    !api_key.is_empty(),
                ),
                LlmProvider::OpenAI {
                    api_key,
                    model,
                    base_url,
                } => {
                    let p = openai_provider_label(base_url.as_deref());
                    let display_base_url = display_openai_base_url(base_url.as_ref());
                    (
                        p.to_string(),
                        model.clone(),
                        display_base_url,
                        !api_key.is_empty(),
                    )
                }
            };
            let role_str = match &slot.role {
                ModelRole::Primary => "primary",
                ModelRole::Fast => "fast",
                ModelRole::Code => "code",
                ModelRole::Research => "research",
                ModelRole::Fallback => "fallback",
            };
            ModelSlotSummary {
                id: model_slot_response_id(slot, idx),
                label: slot.label.clone(),
                role: role_str.to_string(),
                provider: prov,
                model: mdl,
                base_url: burl,
                has_api_key: has_key,
                enabled: slot.enabled,
            }
        })
        .collect();

    (
        StatusCode::OK,
        Json(serde_json::json!({
            "models": slots,
            "smart_routing": agent.config.model_pool.smart_routing,
        })),
    )
        .into_response()
}

pub(super) const MODEL_SLOT_INDEX_ID_PREFIX: &str = "__slot_idx_";

pub(super) fn model_slot_response_id(slot: &ModelSlot, index: usize) -> String {
    let trimmed = slot.id.trim();
    if !trimmed.is_empty() {
        trimmed.to_string()
    } else {
        format!("{}{}", MODEL_SLOT_INDEX_ID_PREFIX, index)
    }
}

pub(super) fn model_role_from_external_id(value: &str) -> Option<ModelRole> {
    match value.trim().to_ascii_lowercase().as_str() {
        "primary" => Some(ModelRole::Primary),
        "fast" => Some(ModelRole::Fast),
        "code" => Some(ModelRole::Code),
        "research" => Some(ModelRole::Research),
        "fallback" => Some(ModelRole::Fallback),
        _ => None,
    }
}

pub(super) fn resolve_model_slot_index(slots: &[ModelSlot], requested_id: &str) -> Option<usize> {
    let requested = requested_id.trim();
    if requested.is_empty() {
        return None;
    }

    if let Some(idx) = slots.iter().position(|slot| slot.id.trim() == requested) {
        return Some(idx);
    }

    if let Some(raw_idx) = requested.strip_prefix(MODEL_SLOT_INDEX_ID_PREFIX) {
        if let Ok(idx) = raw_idx.parse::<usize>() {
            if idx < slots.len() {
                return Some(idx);
            }
        }
    }

    if let Some(idx) = slots
        .iter()
        .position(|slot| slot.label.trim().eq_ignore_ascii_case(requested))
    {
        return Some(idx);
    }

    if let Some(role) = model_role_from_external_id(requested) {
        let mut matches = slots
            .iter()
            .enumerate()
            .filter_map(|(idx, slot)| (slot.role == role).then_some(idx));
        let first = matches.next();
        if matches.next().is_none() {
            return first;
        }

        let requested_prefix = format!("{}_", requested.to_ascii_lowercase());
        if let Some(idx) = slots.iter().position(|slot| {
            slot.id
                .trim()
                .to_ascii_lowercase()
                .starts_with(&requested_prefix)
        }) {
            return Some(idx);
        }
    }

    None
}

pub(super) fn model_slot_has_runtime_credentials(slot: &ModelSlot) -> bool {
    match &slot.provider {
        LlmProvider::Ollama { base_url, .. } => !base_url.trim().is_empty(),
        LlmProvider::Anthropic { api_key, .. } | LlmProvider::OpenAI { api_key, .. } => {
            !api_key.trim().is_empty() && api_key != "[ENCRYPTED]"
        }
    }
}

pub(super) fn model_slot_is_runtime_ready(agent: &Agent, slot: &ModelSlot) -> bool {
    slot.enabled
        && agent.model_pool.contains_key(&slot.id)
        && model_slot_has_runtime_credentials(slot)
}

pub(super) fn resolve_primary_model_slot_id(
    agent: &Agent,
    preferred_slot_id: Option<&str>,
) -> String {
    let slots = &agent.config.model_pool.slots;
    let preferred = preferred_slot_id.and_then(|slot_id| {
        slots
            .iter()
            .find(|slot| slot.id == slot_id && model_slot_is_runtime_ready(agent, slot))
            .or_else(|| slots.iter().find(|slot| slot.id == slot_id && slot.enabled))
    });
    if let Some(slot) = preferred {
        return slot.id.clone();
    }

    let current = slots
        .iter()
        .find(|slot| slot.id == agent.primary_model_id && model_slot_is_runtime_ready(agent, slot));
    if let Some(slot) = current {
        return slot.id.clone();
    }

    let current_enabled = slots
        .iter()
        .find(|slot| slot.id == agent.primary_model_id && slot.enabled);
    if let Some(slot) = current_enabled {
        return slot.id.clone();
    }

    for predicate in [
        ModelRole::Primary,
        ModelRole::Fast,
        ModelRole::Code,
        ModelRole::Research,
        ModelRole::Fallback,
    ] {
        if let Some(slot) = slots
            .iter()
            .find(|slot| slot.role == predicate && model_slot_is_runtime_ready(agent, slot))
        {
            return slot.id.clone();
        }
    }

    if let Some(slot) = slots
        .iter()
        .find(|slot| model_slot_is_runtime_ready(agent, slot))
    {
        return slot.id.clone();
    }

    if let Some(slot) = slots
        .iter()
        .find(|slot| slot.role == ModelRole::Primary && slot.enabled)
    {
        return slot.id.clone();
    }

    if let Some(slot) = slots.iter().find(|slot| slot.enabled) {
        return slot.id.clone();
    }

    slots
        .first()
        .map(|slot| slot.id.clone())
        .unwrap_or_default()
}

pub(super) fn sync_legacy_chat_model_from_slots(agent: &mut Agent) {
    let chosen_slot = agent
        .config
        .model_pool
        .slots
        .iter()
        .find(|slot| slot.id == agent.primary_model_id)
        .or_else(|| agent.config.model_pool.slots.first())
        .cloned();

    if let Some(slot) = chosen_slot {
        agent.config.llm = slot.provider.clone();
        if let Ok(client) = crate::core::LlmClient::new(&slot.provider) {
            agent.llm = client;
        }
    } else {
        agent.config.llm = crate::core::LlmProvider::Ollama {
            base_url: String::new(),
            model: String::new(),
        };
        agent.config.llm_fallback = None;
        if let Ok(client) = crate::core::LlmClient::new(&agent.config.llm) {
            agent.llm = client;
        }
    }
}

pub(super) fn reconcile_model_pool_state(
    agent: &mut Agent,
    preferred_primary_slot_id: Option<&str>,
) {
    agent.primary_model_id = resolve_primary_model_slot_id(agent, preferred_primary_slot_id);
    sync_legacy_chat_model_from_slots(agent);
}

pub(super) fn legacy_llm_is_unconfigured_placeholder(
    config: &crate::core::runtime::config::AgentConfig,
) -> bool {
    config.model_pool.slots.is_empty()
        && config.llm_fallback.is_none()
        && matches!(
            &config.llm,
            LlmProvider::Ollama { base_url, model }
                if model.trim().is_empty() && base_url.trim().is_empty()
        )
}

pub(super) fn legacy_llm_is_explicitly_configured(
    config: &crate::core::runtime::config::AgentConfig,
) -> bool {
    match &config.llm {
        LlmProvider::Ollama { base_url, model } => {
            !base_url.trim().is_empty() && !model.trim().is_empty()
        }
        LlmProvider::Anthropic { model, .. } => !model.trim().is_empty(),
        LlmProvider::OpenAI {
            model, base_url, ..
        } => {
            !model.trim().is_empty()
                && (base_url.is_none()
                    || base_url.as_ref().is_some_and(|url| !url.trim().is_empty()))
        }
    }
}

pub(super) fn settings_has_configured_legacy_llm(
    config: &crate::core::runtime::config::AgentConfig,
    has_key: bool,
) -> bool {
    if legacy_llm_is_unconfigured_placeholder(config) {
        return false;
    }

    match &config.llm {
        LlmProvider::Ollama { .. } => legacy_llm_is_explicitly_configured(config),
        LlmProvider::Anthropic { .. } => has_key && legacy_llm_is_explicitly_configured(config),
        LlmProvider::OpenAI { base_url, .. } => {
            has_key
                && legacy_llm_is_explicitly_configured(config)
                && (base_url.is_none()
                    || !base_url.as_ref().is_some_and(|url| url.trim().is_empty()))
        }
    }
}

pub(super) fn well_known_anthropic_models() -> Vec<serde_json::Value> {
    vec![
        "claude-opus-4-20250514",
        "claude-sonnet-4-20250514",
        "claude-3-7-sonnet-latest",
        "claude-3-5-haiku-latest",
    ]
    .into_iter()
    .map(|id| serde_json::json!({ "id": id }))
    .collect()
}

pub(super) fn collect_model_catalog_rows(body: &serde_json::Value) -> Vec<serde_json::Value> {
    body.get("data")
        .or_else(|| body.get("models"))
        .and_then(|value| value.as_array())
        .cloned()
        .unwrap_or_default()
}

pub(super) fn filter_openai_chat_model_ids(ids: &mut Vec<String>) {
    ids.retain(|id| {
        let is_o_series = id
            .strip_prefix('o')
            .and_then(|rest| rest.chars().next())
            .is_some_and(|ch| ch.is_ascii_digit());
        (id.starts_with("gpt-") || is_o_series || id.starts_with("chatgpt-"))
            && !id.contains("codex")
            && !id.contains("realtime")
            && !id.contains("audio")
            && !id.contains("tts")
            && !id.contains("whisper")
            && !id.contains("dall-e")
            && !id.contains("embedding")
            && !id.contains("moderation")
            && !id.ends_with("-instruct")
    });
    ids.sort();
    ids.dedup();
}

pub(super) fn openai_subscription_catalog_models() -> Vec<serde_json::Value> {
    ["gpt-5.4", "gpt-5.3-codex", "gpt-5.3-codex-spark"]
        .into_iter()
        .map(|id| serde_json::json!({ "id": id }))
        .collect()
}

pub(super) async fn fetch_openai_catalog_models(
    client: &reqwest::Client,
    provider_id: &str,
    base_url: Option<&str>,
    api_key: &str,
) -> std::result::Result<Vec<serde_json::Value>, String> {
    let mut request_config = resolve_openai_request_config(client, api_key, base_url, "")
        .await
        .map_err(|e| e.to_string())?;
    let endpoint = format!("{}/models", request_config.base_url);

    let mut request = client.get(&endpoint);
    if !request_config.api_key.trim().is_empty() {
        request = request.bearer_auth(&request_config.api_key);
    }
    if request_config.is_openrouter {
        request = request
            .header("HTTP-Referer", crate::branding::REPOSITORY_URL)
            .header("X-Title", crate::branding::PRODUCT_NAME);
    }

    let mut response = request.send().await.map_err(|e| e.to_string())?;
    if response.status() == reqwest::StatusCode::UNAUTHORIZED && request_config.uses_codex_cli_oauth
    {
        let refreshed = force_refresh_codex_cli_api_key(client)
            .await
            .map_err(|e| e.to_string())?
            .unwrap_or_default();
        request_config.api_key = refreshed;
        let mut retry = client.get(&endpoint);
        if !request_config.api_key.trim().is_empty() {
            retry = retry.bearer_auth(&request_config.api_key);
        }
        if request_config.is_openrouter {
            retry = retry
                .header("HTTP-Referer", crate::branding::REPOSITORY_URL)
                .header("X-Title", crate::branding::PRODUCT_NAME);
        }
        response = retry.send().await.map_err(|e| e.to_string())?;
    }

    if !response.status().is_success() {
        let status = response.status().as_u16();
        let text = response.text().await.unwrap_or_default();
        return Err(format!("Provider returned {}: {}", status, text.trim()));
    }

    let body: serde_json::Value = response.json().await.unwrap_or_default();
    let rows = collect_model_catalog_rows(&body);
    if provider_id == "openai" || provider_id == "openai-subscription" {
        let mut ids: Vec<String> = rows
            .iter()
            .filter_map(|row| row.get("id").and_then(|value| value.as_str()))
            .map(|value| value.to_string())
            .collect();
        filter_openai_chat_model_ids(&mut ids);
        return Ok(ids
            .into_iter()
            .map(|id| serde_json::json!({ "id": id }))
            .collect());
    }

    let mut rows: Vec<(String, Option<String>)> = rows
        .into_iter()
        .filter_map(|row| {
            let id = row
                .get("id")
                .and_then(|value| value.as_str())
                .map(str::trim)
                .filter(|value| !value.is_empty())?
                .to_string();
            let name = row
                .get("name")
                .and_then(|value| value.as_str())
                .map(str::trim)
                .filter(|value| !value.is_empty())
                .map(|value| value.to_string());
            Some((id, name))
        })
        .collect();
    rows.sort_by(|(left_id, left_name), (right_id, right_name)| {
        left_id.cmp(right_id).then_with(|| {
            left_name
                .as_deref()
                .unwrap_or("")
                .cmp(right_name.as_deref().unwrap_or(""))
        })
    });
    rows.dedup_by(|left, right| left.0 == right.0);
    Ok(rows
        .into_iter()
        .map(|(id, name)| match name {
            Some(name) => serde_json::json!({ "id": id, "name": name }),
            None => serde_json::json!({ "id": id }),
        })
        .collect())
}

pub(super) async fn fetch_huggingface_catalog_models(
    client: &reqwest::Client,
    api_key: &str,
) -> std::result::Result<Vec<serde_json::Value>, String> {
    let mut request = client.get("https://huggingface.co/api/models").query(&[
        ("pipeline_tag", "text-generation"),
        ("sort", "trending"),
        ("limit", "80"),
    ]);
    if !api_key.trim().is_empty() {
        request = request.bearer_auth(api_key.trim());
    }

    let response = request.send().await.map_err(|e| e.to_string())?;
    if !response.status().is_success() {
        let status = response.status().as_u16();
        let text = response.text().await.unwrap_or_default();
        return Err(format!("Hugging Face returned {}: {}", status, text.trim()));
    }

    let body: serde_json::Value = response.json().await.unwrap_or_default();
    let rows = body
        .as_array()
        .cloned()
        .or_else(|| {
            body.get("models")
                .and_then(|value| value.as_array())
                .cloned()
        })
        .unwrap_or_default();
    let mut models: Vec<(String, Option<String>)> = rows
        .into_iter()
        .filter_map(|row| {
            let id = row
                .get("id")
                .or_else(|| row.get("modelId"))
                .and_then(|value| value.as_str())
                .map(str::trim)
                .filter(|value| !value.is_empty())?
                .to_string();
            let name = row
                .get("name")
                .and_then(|value| value.as_str())
                .map(str::trim)
                .filter(|value| !value.is_empty())
                .map(|value| value.to_string());
            Some((id, name))
        })
        .collect();
    models.sort_by(|(left_id, _), (right_id, _)| left_id.cmp(right_id));
    models.dedup_by(|left, right| left.0 == right.0);
    Ok(models
        .into_iter()
        .map(|(id, name)| match name {
            Some(name) => serde_json::json!({ "id": id, "name": name }),
            None => serde_json::json!({ "id": id }),
        })
        .collect())
}

/// Discover available models from a provider API
pub(super) async fn discover_provider_models(
    Path(provider): Path<String>,
    Query(params): Query<std::collections::HashMap<String, String>>,
) -> Response {
    let Some(provider_id) = canonical_provider_id(provider.as_str()) else {
        return (
            StatusCode::BAD_REQUEST,
            Json(serde_json::json!({ "error": format!("Unknown provider: {}", provider) })),
        )
            .into_response();
    };
    if !provider_allows_model_discovery(provider_id) {
        return (
            StatusCode::BAD_REQUEST,
            Json(serde_json::json!({ "error": format!("Provider does not support discovery: {}", provider_id) })),
        )
            .into_response();
    }

    let client = reqwest::Client::builder()
        .timeout(std::time::Duration::from_secs(10))
        .build()
        .unwrap_or_default();

    let models: Vec<serde_json::Value> = match provider_id {
        "openai" => {
            let api_key = params
                .get("api_key")
                .map(String::as_str)
                .unwrap_or_default();
            if api_key.trim().is_empty() {
                return (
                    StatusCode::BAD_REQUEST,
                    Json(serde_json::json!({ "error": "No API key available" })),
                )
                    .into_response();
            }
            match fetch_openai_catalog_models(&client, provider_id, None, api_key).await {
                Ok(models) => models,
                Err(error) => {
                    return (
                        StatusCode::BAD_GATEWAY,
                        Json(serde_json::json!({ "error": error })),
                    )
                        .into_response();
                }
            }
        }
        "openai-subscription" => {
            let connected = resolve_codex_cli_api_key(&client, false)
                .await
                .ok()
                .flatten()
                .is_some();
            if !connected {
                return (
                    StatusCode::BAD_REQUEST,
                    Json(
                        serde_json::json!({ "error": "OpenAI Subscription is not connected yet" }),
                    ),
                )
                    .into_response();
            }
            openai_subscription_catalog_models()
        }
        "openai-compatible" => {
            let Some(base_url) = params
                .get("base_url")
                .map(String::as_str)
                .map(str::trim)
                .filter(|value| !value.is_empty())
            else {
                return (
                    StatusCode::BAD_REQUEST,
                    Json(serde_json::json!({ "error": "Base URL is required for OpenAI-Compatible providers" })),
                )
                    .into_response();
            };
            match fetch_openai_catalog_models(
                &client,
                provider_id,
                Some(base_url),
                params
                    .get("api_key")
                    .map(String::as_str)
                    .unwrap_or_default(),
            )
            .await
            {
                Ok(models) => models,
                Err(error) => {
                    return (
                        StatusCode::BAD_GATEWAY,
                        Json(serde_json::json!({ "error": error })),
                    )
                        .into_response();
                }
            }
        }
        "anthropic" => {
            let api_key = params.get("api_key").cloned().unwrap_or_default();
            if api_key.is_empty() {
                return (
                    StatusCode::OK,
                    Json(serde_json::json!({ "models": well_known_anthropic_models() })),
                )
                    .into_response();
            }
            let resp = client
                .get("https://api.anthropic.com/v1/models")
                .header("x-api-key", &api_key)
                .header("anthropic-version", "2023-06-01")
                .send()
                .await;
            match resp {
                Ok(r) if r.status().is_success() => {
                    let body: serde_json::Value = r.json().await.unwrap_or_default();
                    let mut ids: Vec<String> = collect_model_catalog_rows(&body)
                        .into_iter()
                        .filter_map(|m| {
                            m.get("id")
                                .and_then(|value| value.as_str())
                                .map(|s| s.to_string())
                        })
                        .collect();
                    ids.sort();
                    ids.into_iter()
                        .map(|id| serde_json::json!({ "id": id }))
                        .collect()
                }
                _ => well_known_anthropic_models(),
            }
        }
        "ollama" => {
            let Some(base) = params.get("base_url").map(|s| s.as_str()) else {
                return (
                    StatusCode::BAD_REQUEST,
                    Json(serde_json::json!({ "error": "Base URL is required for Ollama" })),
                )
                    .into_response();
            };
            let resp = client.get(format!("{}/api/tags", base)).send().await;
            match resp {
                Ok(r) if r.status().is_success() => {
                    let body: serde_json::Value = r.json().await.unwrap_or_default();
                    body["models"]
                        .as_array()
                        .map(|arr| {
                            arr.iter()
                                .filter_map(|m| {
                                    m["name"].as_str().map(|s| serde_json::json!({ "id": s }))
                                })
                                .collect()
                        })
                        .unwrap_or_default()
                }
                _ => vec![],
            }
        }
        "openrouter" => {
            let resp = client
                .get(OPENROUTER_API_BASE_URL.to_string() + "/models")
                .send()
                .await;
            match resp {
                Ok(r) if r.status().is_success() => {
                    let body: serde_json::Value = r.json().await.unwrap_or_default();
                    body["data"]
                        .as_array()
                        .map(|arr| {
                            arr.iter()
                                .filter_map(|m| {
                                    m["id"].as_str().map(
                                        |id| serde_json::json!({ "id": id, "name": m["name"] }),
                                    )
                                })
                                .collect()
                        })
                        .unwrap_or_default()
                }
                _ => vec![],
            }
        }
        "huggingface" => {
            let api_key = params
                .get("api_key")
                .map(String::as_str)
                .unwrap_or_default();
            let base_url = params
                .get("base_url")
                .map(String::as_str)
                .map(str::trim)
                .filter(|value| !value.is_empty())
                .unwrap_or(HUGGINGFACE_API_BASE_URL);
            match fetch_huggingface_catalog_models(&client, api_key).await {
                Ok(models) if !models.is_empty() => models,
                _ => fetch_openai_catalog_models(&client, provider_id, Some(base_url), api_key)
                    .await
                    .unwrap_or_default(),
            }
        }
        _ => {
            return (
                StatusCode::BAD_REQUEST,
                Json(serde_json::json!({ "error": format!("Unknown provider: {}", provider_id) })),
            )
                .into_response();
        }
    };

    (
        StatusCode::OK,
        Json(serde_json::json!({ "models": models })),
    )
        .into_response()
}

pub(super) async fn test_model_connection(
    State(state): State<AppState>,
    Json(payload): Json<ModelConnectionTestRequest>,
) -> Response {
    let existing_key = if let Some(raw_id) = payload.id.as_deref() {
        let id = raw_id.trim();
        if id.is_empty() {
            None
        } else {
            match resolve_requested_model_slot_api_key(&state, id, &payload.request).await {
                Ok(key) => key,
                Err(response) => return response,
            }
        }
    } else {
        None
    };

    let provider = match provider_from_model_slot_request(&payload.request, existing_key).await {
        Ok(provider) => provider,
        Err(error) => {
            return (StatusCode::BAD_REQUEST, Json(ErrorResponse { error })).into_response();
        }
    };

    let (connectivity_ok, connectivity_error) = match tokio::time::timeout(
        std::time::Duration::from_secs(25),
        test_llm_connection(&provider),
    )
    .await
    {
        Ok(Ok(())) => (true, None),
        Ok(Err(error)) => (false, Some(error)),
        Err(_) => (false, Some("Connection test timed out.".to_string())),
    };

    // Capability probes: connectivity alone passes models that cannot emit
    // well-formed tool calls or non-empty finals — gaps otherwise discovered
    // only through opaque agent-run failure loops. Probe the actual contract
    // behaviorally, through the same client the spine uses.
    let capabilities = if connectivity_ok {
        match crate::core::model::llm::LlmClient::new(&provider) {
            Ok(client) => {
                let policy = {
                    let agent = state.agent.read().await;
                    agent.config.model_privacy.clone()
                };
                tokio::time::timeout(
                    std::time::Duration::from_secs(60),
                    crate::core::model::llm::capability_probe::probe_model_capabilities(
                        &client, &policy,
                    ),
                )
                .await
                .ok()
            }
            Err(_) => None,
        }
    } else {
        None
    };

    let mut body = serde_json::json!({ "ok": connectivity_ok });
    if let Some(error) = connectivity_error {
        body["error"] = serde_json::Value::String(error);
    }
    match capabilities {
        Some(report) => {
            body["capabilities"] = serde_json::json!({
                "tool_calls": report.tool_calls,
                "final_response": report.final_response,
                "json_output": report.json_output,
            });
            body["verdict"] = serde_json::Value::String(report.verdict.to_string());
            body["role_fitness"] =
                serde_json::to_value(&report.role_fitness).unwrap_or(serde_json::Value::Null);
            if report.verdict == "unsupported_for_agent_runs" {
                body["warning"] = serde_json::Value::String(
                    "This model did not produce well-formed tool calls and/or non-empty final responses in the capability probe. Agent runs (the primary model role) are likely to fail with it; it may still work for text-only roles."
                        .to_string(),
                );
            }
        }
        None if connectivity_ok => {
            body["capabilities_error"] = serde_json::Value::String(
                "Capability probes did not complete (client build failure or probe timeout); result reflects connectivity only."
                    .to_string(),
            );
        }
        None => {}
    }
    (StatusCode::OK, Json(body)).into_response()
}

/// Add a new model slot
pub(super) async fn add_model(
    State(state): State<AppState>,
    Json(request): Json<ModelSlotRequest>,
) -> Response {
    let role = match request.role.as_str() {
        "primary" => ModelRole::Primary,
        "fast" => ModelRole::Fast,
        "code" => ModelRole::Code,
        "research" => ModelRole::Research,
        "fallback" => ModelRole::Fallback,
        _ => {
            return (
                StatusCode::BAD_REQUEST,
                Json(ErrorResponse {
                    error: format!("Unknown role: {}", request.role),
                }),
            )
                .into_response();
        }
    };

    let provider = match provider_from_model_slot_request(&request, None).await {
        Ok(provider) => provider,
        Err(error) => {
            return (StatusCode::BAD_REQUEST, Json(ErrorResponse { error })).into_response();
        }
    };

    let (config_snapshot, config_dir, data_dir) = {
        let mut agent =
            match acquire_agent_write_for_config_mutation(&state, "saving model changes").await {
                Ok(agent) => agent,
                Err(response) => return response,
            };

        let slot_id = format!(
            "{}_{}",
            request.role,
            uuid::Uuid::new_v4()
                .to_string()
                .split('-')
                .next()
                .unwrap_or("x")
        );

        let slot = ModelSlot {
            id: slot_id.clone(),
            label: request.label.clone(),
            role,
            provider: provider.clone(),
            enabled: request.enabled.unwrap_or(true),
            capability_tier: crate::core::runtime::config::ModelCapabilityTier::Balanced,
            cost_tier: crate::core::runtime::config::ModelCostTier::Medium,
            auto_escalate: true,
            escalation_rank: 0,
            health_scope: crate::core::runtime::config::ModelHealthScope::Provider,
        };

        agent.config.model_pool.slots.push(slot.clone());

        match crate::core::LlmClient::new(&provider) {
            Ok(client) => {
                agent.model_pool.insert(slot_id.clone(), (slot, client));
            }
            Err(e) => {
                return (
                    StatusCode::INTERNAL_SERVER_ERROR,
                    Json(ErrorResponse {
                        error: format!("Failed to initialize model: {}", e),
                    }),
                )
                    .into_response();
            }
        }

        let preferred_primary = if request.role == "primary" {
            Some(slot_id.as_str())
        } else {
            None
        };
        reconcile_model_pool_state(&mut agent, preferred_primary);

        (
            agent.config.clone(),
            agent.config_dir.clone(),
            agent.data_dir.clone(),
        )
    };

    match save_agent_config_snapshot(config_snapshot, config_dir, data_dir).await {
        Ok(()) => {
            tracing::debug!(
                "Vector memory backend retains its current configuration after model add."
            );
            (
                StatusCode::OK,
                Json(serde_json::json!({
                    "status": "ok",
                    "message": "Model added"
                })),
            )
                .into_response()
        }
        Err(e) => (
            StatusCode::INTERNAL_SERVER_ERROR,
            Json(ErrorResponse {
                error: format!("Failed to save: {}", e),
            }),
        )
            .into_response(),
    }
}

/// Update a model slot
pub(super) async fn update_model(
    State(state): State<AppState>,
    Path(id): Path<String>,
    Json(request): Json<ModelSlotRequest>,
) -> Response {
    let role = match request.role.as_str() {
        "primary" => ModelRole::Primary,
        "fast" => ModelRole::Fast,
        "code" => ModelRole::Code,
        "research" => ModelRole::Research,
        "fallback" => ModelRole::Fallback,
        _ => {
            return (
                StatusCode::BAD_REQUEST,
                Json(ErrorResponse {
                    error: format!("Unknown role: {}", request.role),
                }),
            )
                .into_response();
        }
    };

    let (
        previous_slot_id_for_lookup,
        config_dir_for_lookup,
        data_dir_for_lookup,
        can_reuse_existing_key,
        existing_key_hint,
    ) = {
        let agent =
            match acquire_agent_write_for_config_mutation(&state, "saving model changes").await {
                Ok(agent) => agent,
                Err(response) => return response,
            };

        let slot_idx = resolve_model_slot_index(&agent.config.model_pool.slots, &id);
        let Some(idx) = slot_idx else {
            return (
                StatusCode::NOT_FOUND,
                Json(ErrorResponse {
                    error: "Model slot not found".to_string(),
                }),
            )
                .into_response();
        };

        let current_slot = agent.config.model_pool.slots[idx].clone();
        let can_reuse_existing_key = match can_reuse_model_slot_api_key(&current_slot, &request) {
            Ok(value) => value,
            Err(error) => {
                return (StatusCode::BAD_REQUEST, Json(ErrorResponse { error })).into_response();
            }
        };
        let existing_key_hint = match &current_slot.provider {
            LlmProvider::Anthropic { api_key, .. } => Some(api_key.clone()),
            LlmProvider::OpenAI { api_key, .. } => Some(api_key.clone()),
            _ => None,
        };

        (
            current_slot.id.trim().to_string(),
            agent.config_dir.clone(),
            agent.data_dir.clone(),
            can_reuse_existing_key,
            existing_key_hint,
        )
    };

    let existing_key = if can_reuse_existing_key {
        if matches!(
            existing_key_hint.as_deref(),
            None | Some("") | Some("[ENCRYPTED]")
        ) {
            load_saved_model_slot_api_key(
                config_dir_for_lookup,
                data_dir_for_lookup,
                previous_slot_id_for_lookup,
                id.clone(),
                request.role.clone(),
                request.label.clone(),
            )
            .await
            .or(existing_key_hint)
        } else {
            existing_key_hint
        }
    } else {
        None
    };

    let provider = match provider_from_model_slot_request(&request, existing_key).await {
        Ok(provider) => provider,
        Err(error) => {
            return (StatusCode::BAD_REQUEST, Json(ErrorResponse { error })).into_response();
        }
    };

    let (config_snapshot, config_dir, data_dir) = {
        let mut agent =
            match acquire_agent_write_for_config_mutation(&state, "saving model changes").await {
                Ok(agent) => agent,
                Err(response) => return response,
            };

        let slot_idx = resolve_model_slot_index(&agent.config.model_pool.slots, &id);
        let Some(idx) = slot_idx else {
            return (
                StatusCode::NOT_FOUND,
                Json(ErrorResponse {
                    error: "Model slot not found".to_string(),
                }),
            )
                .into_response();
        };

        let previous_slot_id = agent.config.model_pool.slots[idx].id.trim().to_string();
        let slot_id = if !previous_slot_id.is_empty() {
            previous_slot_id.clone()
        } else if !id.trim().is_empty() && !id.trim().starts_with(MODEL_SLOT_INDEX_ID_PREFIX) {
            id.trim().to_string()
        } else {
            format!(
                "{}_{}",
                request.role,
                uuid::Uuid::new_v4()
                    .to_string()
                    .split('-')
                    .next()
                    .unwrap_or("x")
            )
        };

        let enabled = request
            .enabled
            .unwrap_or(agent.config.model_pool.slots[idx].enabled);

        let slot = ModelSlot {
            id: slot_id.clone(),
            label: request.label.clone(),
            role,
            provider: provider.clone(),
            enabled,
            capability_tier: crate::core::runtime::config::ModelCapabilityTier::Balanced,
            cost_tier: crate::core::runtime::config::ModelCostTier::Medium,
            auto_escalate: true,
            escalation_rank: 0,
            health_scope: crate::core::runtime::config::ModelHealthScope::Provider,
        };

        agent.config.model_pool.slots[idx] = slot.clone();

        if !previous_slot_id.is_empty() {
            agent.model_pool.remove(&previous_slot_id);
        }
        if enabled {
            match crate::core::LlmClient::new(&provider) {
                Ok(client) => {
                    agent.model_pool.insert(slot_id.clone(), (slot, client));
                }
                Err(e) => {
                    return (
                        StatusCode::INTERNAL_SERVER_ERROR,
                        Json(ErrorResponse {
                            error: format!("Failed to initialize model: {}", e),
                        }),
                    )
                        .into_response();
                }
            }
        }

        let preferred_primary = if request.role == "primary" && enabled {
            Some(slot_id.as_str())
        } else {
            None
        };
        reconcile_model_pool_state(&mut agent, preferred_primary);

        (
            agent.config.clone(),
            agent.config_dir.clone(),
            agent.data_dir.clone(),
        )
    };

    match save_agent_config_snapshot(config_snapshot, config_dir, data_dir).await {
        Ok(()) => {
            tracing::debug!(
                "Vector memory backend retains its current configuration after model update."
            );
            (
                StatusCode::OK,
                Json(serde_json::json!({
                    "status": "ok",
                    "message": "Model updated"
                })),
            )
                .into_response()
        }
        Err(e) => (
            StatusCode::INTERNAL_SERVER_ERROR,
            Json(ErrorResponse {
                error: format!("Failed to save: {}", e),
            }),
        )
            .into_response(),
    }
}

/// Delete a model slot
pub(super) async fn delete_model(
    State(state): State<AppState>,
    Path(id): Path<String>,
) -> Response {
    let (storage, config_snapshot, config_dir, data_dir, requested_id, resolved_slot_id) = {
        let mut agent =
            match acquire_agent_write_for_config_mutation(&state, "removing a model").await {
                Ok(agent) => agent,
                Err(response) => return response,
            };

        let slot_idx = resolve_model_slot_index(&agent.config.model_pool.slots, &id);
        let Some(idx) = slot_idx else {
            return (
                StatusCode::NOT_FOUND,
                Json(ErrorResponse {
                    error: "Model slot not found".to_string(),
                }),
            )
                .into_response();
        };

        let requested_id = id.trim().to_string();
        let resolved_slot_id = agent.config.model_pool.slots[idx].id.trim().to_string();
        agent.config.model_pool.slots.remove(idx);
        if !resolved_slot_id.is_empty() {
            agent.model_pool.remove(&resolved_slot_id);
        }
        if !requested_id.is_empty() && requested_id != resolved_slot_id {
            agent.model_pool.remove(&requested_id);
        }

        if let Ok(mut selected_slot_id) = agent.user_selected_model_slot_id.write() {
            if selected_slot_id.as_deref() == Some(resolved_slot_id.as_str())
                || (!requested_id.is_empty()
                    && selected_slot_id.as_deref() == Some(requested_id.as_str()))
            {
                *selected_slot_id = None;
            }
        }

        reconcile_model_pool_state(&mut agent, None);

        (
            agent.storage.clone(),
            agent.config.clone(),
            agent.config_dir.clone(),
            agent.data_dir.clone(),
            requested_id,
            resolved_slot_id,
        )
    };

    let _ = storage
        .delete(crate::core::USER_SELECTED_MODEL_SLOT_KEY)
        .await;

    if let Err(error) = remove_saved_model_slot_api_keys(
        config_dir.clone(),
        data_dir.clone(),
        resolved_slot_id.clone(),
        requested_id.clone(),
    )
    .await
    {
        tracing::warn!(
            "Failed to remove saved model keys for deleted slot: {}",
            error
        );
    }

    match save_agent_config_snapshot(config_snapshot, config_dir, data_dir).await {
        Ok(()) => (
            StatusCode::OK,
            Json(serde_json::json!({"status": "ok", "message": "Model removed"})),
        )
            .into_response(),
        Err(e) => (
            StatusCode::INTERNAL_SERVER_ERROR,
            Json(ErrorResponse {
                error: format!("Failed to save: {}", e),
            }),
        )
            .into_response(),
    }
}
