use super::*;

#[derive(Debug, Deserialize)]
pub(super) struct SecretsVaultRevealRequest {}

#[derive(Debug, Deserialize)]
pub(super) struct SecretsVaultUpsertRequest {
    key: String,
    value: String,
    #[serde(default)]
    password: Option<String>,
    #[serde(default)]
    conversation_id: Option<String>,
}

#[derive(Debug, Deserialize)]
pub(super) struct SecretsVaultDeleteRequest {
    key: String,
    #[serde(default)]
    password: Option<String>,
}

#[derive(Debug, Deserialize)]
pub(super) struct ChatCredentialPromptQuery {
    conversation_id: String,
}

#[derive(Debug, Deserialize)]
pub(super) struct ChatCredentialPromptSubmitRequest {
    conversation_id: String,
    values: BTreeMap<String, String>,
}

#[derive(Debug, Deserialize)]
pub(super) struct ChatRawSecretSubmitRequest {
    #[serde(default)]
    conversation_id: Option<String>,
    key: String,
    value: String,
}

#[derive(Debug, Deserialize)]
pub(super) struct ChatReuseModelCredentialRequest {
    #[serde(default)]
    conversation_id: Option<String>,
    key: String,
}

pub(super) fn is_internal_secret_key(key: &str) -> bool {
    key.starts_with("integration_enabled:") || key.starts_with("action_envmap:")
}

pub(super) fn is_valid_user_secret_key(key: &str) -> bool {
    if key.is_empty() || key.len() > 160 {
        return false;
    }
    key.chars()
        .all(|c| c.is_ascii_alphanumeric() || matches!(c, '_' | '-' | ':' | '.'))
}

pub(super) fn mask_secret_value(value: &str) -> String {
    let len = value.chars().count();
    if len == 0 {
        return String::new();
    }
    if len <= 6 {
        return "*".repeat(len);
    }
    let prefix: String = value.chars().take(3).collect();
    let suffix: String = value
        .chars()
        .rev()
        .take(3)
        .collect::<String>()
        .chars()
        .rev()
        .collect();
    format!("{}...{}", prefix, suffix)
}

pub(super) fn settings_secret_source_for_custom_key(key: &str) -> &'static str {
    match key {
        "moltbook_api_key" => "moltbook",
        "search_serper_key"
        | "search_brave_key"
        | "search_exa_key"
        | "search_tavily_key"
        | "search_perplexity_key"
        | "search_firecrawl_key" => "search",
        crate::core::observability::OBSERVABILITY_AUTH_TOKEN_SECRET_KEY => "observability",
        _ => "custom",
    }
}

pub(super) fn titleize_secret_label(value: &str) -> String {
    let trimmed = value.trim();
    if trimmed.is_empty() {
        return String::new();
    }
    trimmed
        .split(|ch: char| matches!(ch, '_' | '-' | ':' | '.'))
        .filter(|part| !part.trim().is_empty())
        .map(|part| {
            let lower = part.to_ascii_lowercase();
            match lower.as_str() {
                "api" => "API".to_string(),
                "id" => "ID".to_string(),
                "oauth" => "OAuth".to_string(),
                "url" => "URL".to_string(),
                "uri" => "URI".to_string(),
                "cli" => "CLI".to_string(),
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

pub(super) fn parse_extension_pack_secret_key(key: &str) -> Option<(&str, &str)> {
    let remainder = key.strip_prefix("extension_pack_secret:")?;
    let (pack_id, connection_id) = remainder.split_once(':')?;
    let pack_id = pack_id.trim();
    let connection_id = connection_id.trim();
    if pack_id.is_empty() || connection_id.is_empty() {
        return None;
    }
    Some((pack_id, connection_id))
}

pub(super) fn extension_pack_secret_display_key(pack_id: &str, connection_id: &str) -> String {
    let pack_label = titleize_secret_label(pack_id);
    let default_connection_id = format!("{}-default", pack_id);
    if connection_id.eq_ignore_ascii_case(&default_connection_id) {
        return format!("{} credentials", pack_label);
    }
    let connection_label = titleize_secret_label(connection_id);
    if connection_label.is_empty() || connection_label.eq_ignore_ascii_case(&pack_label) {
        format!("{} credentials", pack_label)
    } else {
        format!("{} credentials ({})", pack_label, connection_label)
    }
}

pub(super) fn extension_pack_secret_masked_value(value: &str) -> String {
    let Ok(parsed) = serde_json::from_str::<serde_json::Value>(value) else {
        return mask_secret_value(value);
    };
    let Some(obj) = parsed.as_object() else {
        return mask_secret_value(value);
    };
    if obj.is_empty() {
        return "No secrets saved".to_string();
    }
    if obj.len() == 1 {
        let Some((field, field_value)) = obj.iter().next() else {
            return mask_secret_value(value);
        };
        let Some(secret) = field_value.as_str() else {
            return "1 secret field saved".to_string();
        };
        return format!("{}: {}", field, mask_secret_value(secret));
    }
    format!("{} secret fields saved", obj.len())
}

pub(super) fn push_settings_secret_entry_with_display(
    entries: &mut Vec<serde_json::Value>,
    storage_key: String,
    display_key: String,
    masked: String,
    source: &str,
    source_label: Option<String>,
    deletable: bool,
    value_length: usize,
) {
    entries.push(serde_json::json!({
        "storage_key": storage_key,
        "key": display_key,
        "masked": masked,
        "length": value_length,
        "source": source,
        "source_label": source_label,
        "deletable": deletable,
    }));
}

pub(super) fn is_configured_secret(value: &str) -> bool {
    !value.trim().is_empty() && value != "[ENCRYPTED]"
}

pub(super) fn trimmed_option_string(value: Option<&str>) -> Option<String> {
    value
        .map(str::trim)
        .filter(|value| !value.is_empty())
        .map(ToOwned::to_owned)
}

pub(super) fn settings_configured_email_backends(
    config: &crate::core::config::AgentConfig,
    config_dir: &FsPath,
    gmail_enabled: bool,
    workspace_enabled: bool,
) -> Vec<String> {
    let mut backends = Vec::new();
    let has_legacy_gmail = gmail_enabled
        && crate::core::config::SecureConfigManager::new(config_dir)
            .ok()
            .and_then(|manager| manager.get_custom_secret("gmail_tokens").ok().flatten())
            .is_some_and(|value| !value.trim().is_empty());
    if has_legacy_gmail {
        backends.push(crate::core::email_delivery::EMAIL_PROVIDER_GMAIL.to_string());
    }
    let has_workspace_gmail = workspace_enabled
        && crate::actions::google_workspace::granted_bundles(config_dir)
            .map(|bundles| bundles.iter().any(|bundle| bundle == "gmail"))
            .unwrap_or(false);
    if has_workspace_gmail {
        backends.push(crate::core::email_delivery::EMAIL_PROVIDER_GOOGLE_WORKSPACE.to_string());
    }
    if crate::core::email_delivery::external_email_delivery_is_ready(&config.email) {
        if let Some(provider_id) =
            crate::core::email_delivery::external_email_provider_id(&config.email)
        {
            if !backends.iter().any(|existing| existing == &provider_id) {
                backends.push(provider_id);
            }
        }
    }
    backends
}

pub(super) fn build_email_settings_response(
    config: &crate::core::config::AgentConfig,
    config_dir: &FsPath,
    gmail_enabled: bool,
    workspace_enabled: bool,
) -> EmailSettingsResponse {
    let available_backends =
        settings_configured_email_backends(config, config_dir, gmail_enabled, workspace_enabled);
    let provider = crate::core::email_delivery::normalize_email_provider(&config.email.provider);
    let auto_resolves_to = if provider == crate::core::email_delivery::EMAIL_PROVIDER_AUTO {
        crate::core::email_delivery::normalize_email_backend_selection(
            &provider,
            &available_backends,
        )
        .ok()
    } else {
        None
    };
    EmailSettingsResponse {
        provider,
        to_address: trimmed_option_string(config.email.to_address.as_deref()),
        from_address: trimmed_option_string(config.email.from_address.as_deref()),
        domain: trimmed_option_string(config.email.domain.as_deref()),
        available_backends: available_backends.clone(),
        auto_resolves_to,
        delivery_ready: crate::core::email_delivery::email_channel_is_ready(
            &config.email.provider,
            &available_backends,
        ),
        transport: EmailTransportSettingsResponse {
            kind: crate::core::email_delivery::normalize_transport_kind(
                &config.email.transport.kind,
            ),
            http: EmailHttpTransportSettingsResponse {
                base_url: trimmed_option_string(config.email.transport.http.base_url.as_deref()),
                send_path: trimmed_option_string(config.email.transport.http.send_path.as_deref()),
            },
            smtp: EmailSmtpTransportSettingsResponse {
                host: config.email.transport.smtp.host.clone(),
                port: config.email.transport.smtp.port,
                security: config.email.transport.smtp.security.clone(),
            },
        },
        auth: EmailAuthSettingsResponse {
            kind: crate::core::email_delivery::normalize_auth_kind(&config.email.auth.kind),
            header_name: trimmed_option_string(config.email.auth.header_name.as_deref()),
            scheme: trimmed_option_string(config.email.auth.scheme.as_deref()),
            basic_username: config.email.auth.basic_username.clone(),
            aws_access_key_id: config.email.auth.aws_access_key_id.clone(),
            aws_region: trimmed_option_string(config.email.auth.aws_region.as_deref()),
            aws_service: trimmed_option_string(config.email.auth.aws_service.as_deref()),
            has_api_key: is_configured_secret(&config.email.auth.api_key),
            has_basic_password: is_configured_secret(&config.email.auth.basic_password),
            has_aws_secret_access_key: is_configured_secret(
                &config.email.auth.aws_secret_access_key,
            ),
            has_aws_session_token: config
                .email
                .auth
                .aws_session_token
                .as_deref()
                .is_some_and(is_configured_secret),
        },
    }
}

pub(super) fn custom_settings_secret_is_deletable(_key: &str) -> bool {
    true
}

pub(super) fn push_settings_secret_entry(
    entries: &mut Vec<serde_json::Value>,
    key: String,
    value: &str,
    source: &str,
    deletable: bool,
) {
    if value.trim().is_empty() {
        return;
    }
    push_settings_secret_entry_with_display(
        entries,
        key.clone(),
        key,
        mask_secret_value(value),
        source,
        None,
        deletable,
        value.chars().count(),
    );
}

pub(super) fn collect_settings_secret_entries(
    secrets: &crate::core::config::Secrets,
) -> Vec<serde_json::Value> {
    let mut entries = Vec::new();

    if let Some(value) = secrets.llm_api_key.as_deref() {
        push_settings_secret_entry(
            &mut entries,
            "llm_api_key".to_string(),
            value,
            "model-primary",
            false,
        );
    }
    if let Some(value) = secrets.llm_fallback_api_key.as_deref() {
        push_settings_secret_entry(
            &mut entries,
            "llm_fallback_api_key".to_string(),
            value,
            "model-fallback",
            false,
        );
    }
    if let Some(value) = secrets.telegram_bot_token.as_deref() {
        push_settings_secret_entry(
            &mut entries,
            "telegram_bot_token".to_string(),
            value,
            "telegram",
            false,
        );
    }
    if let Some(value) = secrets.slack_bot_token.as_deref() {
        push_settings_secret_entry(
            &mut entries,
            "slack_bot_token".to_string(),
            value,
            "slack",
            false,
        );
    }
    if let Some(value) = secrets.slack_signing_secret.as_deref() {
        push_settings_secret_entry(
            &mut entries,
            "slack_signing_secret".to_string(),
            value,
            "slack",
            false,
        );
    }
    if let Some(value) = secrets.discord_bot_token.as_deref() {
        push_settings_secret_entry(
            &mut entries,
            "discord_bot_token".to_string(),
            value,
            "discord",
            false,
        );
    }
    if let Some(value) = secrets.matrix_access_token.as_deref() {
        push_settings_secret_entry(
            &mut entries,
            "matrix_access_token".to_string(),
            value,
            "matrix",
            false,
        );
    }
    if let Some(value) = secrets.teams_access_token.as_deref() {
        push_settings_secret_entry(
            &mut entries,
            "teams_access_token".to_string(),
            value,
            "teams",
            false,
        );
    }
    if let Some(value) = secrets.whatsapp_access_token.as_deref() {
        push_settings_secret_entry(
            &mut entries,
            "whatsapp_access_token".to_string(),
            value,
            "whatsapp",
            false,
        );
    }
    if let Some(value) = secrets.whatsapp_app_secret.as_deref() {
        push_settings_secret_entry(
            &mut entries,
            "whatsapp_app_secret".to_string(),
            value,
            "whatsapp",
            false,
        );
    }
    if let Some(value) = secrets.whatsapp_bridge_token.as_deref() {
        push_settings_secret_entry(
            &mut entries,
            "whatsapp_bridge_token".to_string(),
            value,
            "whatsapp",
            false,
        );
    }
    if let Some(value) = secrets.custom.get("email.auth.api_key") {
        push_settings_secret_entry(
            &mut entries,
            "email.auth.api_key".to_string(),
            value,
            "email",
            false,
        );
    }
    if let Some(value) = secrets.custom.get("email.auth.basic_password") {
        push_settings_secret_entry(
            &mut entries,
            "email.auth.basic_password".to_string(),
            value,
            "email",
            false,
        );
    }
    if let Some(value) = secrets.custom.get("email.auth.aws_secret_access_key") {
        push_settings_secret_entry(
            &mut entries,
            "email.auth.aws_secret_access_key".to_string(),
            value,
            "email",
            false,
        );
    }
    if let Some(value) = secrets.custom.get("email.auth.aws_session_token") {
        push_settings_secret_entry(
            &mut entries,
            "email.auth.aws_session_token".to_string(),
            value,
            "email",
            false,
        );
    }
    if let Some(value) = secrets.tunnel_ngrok_authtoken.as_deref() {
        push_settings_secret_entry(
            &mut entries,
            "tunnel_ngrok_authtoken".to_string(),
            value,
            "tunnel",
            false,
        );
    }
    if let Some(value) = secrets.tunnel_tailscale_auth_key.as_deref() {
        push_settings_secret_entry(
            &mut entries,
            "tunnel_tailscale_auth_key".to_string(),
            value,
            "tunnel",
            false,
        );
    }
    if let Some(value) = secrets.api_key.as_deref() {
        push_settings_secret_entry(
            &mut entries,
            "http_api_key".to_string(),
            value,
            "api",
            false,
        );
    }

    let mut media_keys: Vec<_> = secrets.media_provider_keys.iter().collect();
    media_keys.sort_by(|(left, _), (right, _)| left.cmp(right));
    for (provider, value) in media_keys {
        push_settings_secret_entry(
            &mut entries,
            format!("media_provider:{}", provider),
            value,
            "media",
            false,
        );
    }

    let mut model_keys: Vec<_> = secrets.model_pool_keys.iter().collect();
    model_keys.sort_by(|(left, _), (right, _)| left.cmp(right));
    for (slot_id, value) in model_keys {
        push_settings_secret_entry(
            &mut entries,
            format!("model_slot:{}", slot_id),
            value,
            "model-slot",
            false,
        );
    }

    let mut mcp_auth: Vec<_> = secrets.mcp_auth.iter().collect();
    mcp_auth.sort_by(|(left, _), (right, _)| left.cmp(right));
    for (server_id, auth) in mcp_auth {
        if let Some(token) = auth.token.as_deref() {
            push_settings_secret_entry(
                &mut entries,
                format!("mcp:{}:token", server_id),
                token,
                "mcp",
                false,
            );
        }
        if let Some(password) = auth.password.as_deref() {
            push_settings_secret_entry(
                &mut entries,
                format!("mcp:{}:password", server_id),
                password,
                "mcp",
                false,
            );
        }
    }

    let mut custom_entries: Vec<_> = secrets
        .custom
        .iter()
        .filter(|(key, _)| !is_internal_secret_key(key))
        .collect();
    custom_entries.sort_by(|(left, _), (right, _)| left.cmp(right));
    for (key, value) in custom_entries {
        if let Some((pack_id, connection_id)) = parse_extension_pack_secret_key(key) {
            let source_label = titleize_secret_label(pack_id);
            push_settings_secret_entry_with_display(
                &mut entries,
                key.clone(),
                extension_pack_secret_display_key(pack_id, connection_id),
                extension_pack_secret_masked_value(value),
                "extension-pack",
                Some(source_label),
                custom_settings_secret_is_deletable(key),
                value.chars().count(),
            );
        } else {
            push_settings_secret_entry(
                &mut entries,
                key.clone(),
                value,
                settings_secret_source_for_custom_key(key),
                custom_settings_secret_is_deletable(key),
            );
        }
    }

    entries.sort_by_key(|row| {
        row.get("key")
            .and_then(|value| value.as_str())
            .unwrap_or("")
            .to_string()
    });
    entries
}

pub(super) fn require_master_password_for_secrets(
    config_dir: &FsPath,
    data_dir: &FsPath,
    password: Option<&str>,
) -> std::result::Result<(), String> {
    let master_mgr = crate::crypto::master::MasterPasswordManager::new(config_dir, data_dir);
    let custom_master_password_set =
        master_mgr.is_password_set() && !master_mgr.is_bootstrap_password_active().unwrap_or(false);
    if !custom_master_password_set {
        return Ok(());
    }
    let supplied = password.unwrap_or("").trim();
    if supplied.is_empty() {
        return Err("Master password is required.".to_string());
    }
    master_mgr
        .unlock(supplied)
        .map(|_| ())
        .map_err(|_| "Master password is incorrect.".to_string())
}

pub(super) async fn chat_secret_prompt_block_message(
    state: &AppState,
    conversation_id: Option<&str>,
    message: &str,
) -> Option<String> {
    let conversation_id = conversation_id
        .map(str::trim)
        .filter(|value| !value.is_empty())?;
    let redaction = crate::security::redact_secret_input(message);
    if !redaction.had_secret() {
        return None;
    }
    let agent = state.agent.read().await;
    if agent
        .pending_chat_credential_prompt(conversation_id)
        .await
        .is_none()
    {
        return None;
    }
    Some(
        "Never paste secrets, API keys, passwords, or sensitive data into normal chat. Use the secure credential form shown in this conversation.".to_string(),
    )
}

pub(super) async fn get_chat_credential_prompt(
    State(state): State<AppState>,
    Query(query): Query<ChatCredentialPromptQuery>,
) -> Response {
    let conversation_id = query.conversation_id.trim();
    if conversation_id.is_empty() {
        return error_response(StatusCode::BAD_REQUEST, "conversation_id is required");
    }
    let agent = state.agent.read().await;
    let prompt = agent.pending_chat_credential_prompt(conversation_id).await;
    (
        StatusCode::OK,
        Json(serde_json::json!({
            "present": prompt.is_some(),
            "prompt": prompt,
        })),
    )
        .into_response()
}

pub(super) async fn submit_chat_credential_prompt(
    State(state): State<AppState>,
    Json(request): Json<ChatCredentialPromptSubmitRequest>,
) -> Response {
    let conversation_id = request.conversation_id.trim();
    if conversation_id.is_empty() {
        return error_response(StatusCode::BAD_REQUEST, "conversation_id is required");
    }
    if request.values.is_empty() {
        return error_response(
            StatusCode::BAD_REQUEST,
            "Provide at least one credential value",
        );
    }
    let agent = state.agent.read().await;
    if agent
        .pending_chat_credential_prompt(conversation_id)
        .await
        .is_none()
    {
        return error_response(
            StatusCode::BAD_REQUEST,
            "No secure credential request is pending for this conversation",
        );
    }
    match agent
        .submit_chat_credential_values(Some(conversation_id), &request.values)
        .await
    {
        Ok(followup) => {
            let prompt = agent.pending_chat_credential_prompt(conversation_id).await;
            (
                StatusCode::OK,
                Json(serde_json::json!({
                    "status": "ok",
                    "followup": followup,
                    "present": prompt.is_some(),
                    "prompt": prompt,
                })),
            )
                .into_response()
        }
        Err(error) => error_response(StatusCode::BAD_REQUEST, error),
    }
}

pub(super) async fn submit_chat_raw_secret(
    State(state): State<AppState>,
    Json(request): Json<ChatRawSecretSubmitRequest>,
) -> Response {
    let key = request.key.trim().to_string();
    if !is_valid_user_secret_key(&key) {
        return error_response(
            StatusCode::BAD_REQUEST,
            "Invalid key. Use letters, numbers, '_', '-', ':' or '.'.",
        );
    }
    if is_internal_secret_key(&key) {
        return error_response(
            StatusCode::BAD_REQUEST,
            "This key is reserved for internal settings.",
        );
    }

    let value = request.value.trim().to_string();
    if value.is_empty() {
        return error_response(
            StatusCode::BAD_REQUEST,
            "Enter the credential value before saving.",
        );
    }

    let mut values = BTreeMap::new();
    values.insert(key.clone(), value);
    let agent = state.agent.read().await;
    match agent
        .submit_chat_credential_values(request.conversation_id.as_deref(), &values)
        .await
    {
        Ok(followup) => (
            StatusCode::OK,
            Json(serde_json::json!({
                "status": "ok",
                "key": key,
                "followup": followup,
            })),
        )
            .into_response(),
        Err(error) => error_response(StatusCode::BAD_REQUEST, error),
    }
}

pub(super) async fn reuse_model_credential_for_chat(
    State(state): State<AppState>,
    Json(request): Json<ChatReuseModelCredentialRequest>,
) -> Response {
    let key = request.key.trim().to_string();
    if !is_valid_user_secret_key(&key) {
        return error_response(
            StatusCode::BAD_REQUEST,
            "Invalid key. Use letters, numbers, '_', '-', ':' or '.'.",
        );
    }
    if is_internal_secret_key(&key) {
        return error_response(
            StatusCode::BAD_REQUEST,
            "This key is reserved for internal settings.",
        );
    }

    let llm_env = {
        let agent = state.agent.read().await;
        agent.app_model_env_vars()
    };
    let Some(value) = llm_env
        .get(&key)
        .cloned()
        .filter(|value| !value.trim().is_empty())
    else {
        let mut available: Vec<String> = llm_env
            .iter()
            .filter_map(|(candidate, value)| {
                if value.trim().is_empty() {
                    None
                } else if candidate.ends_with("_API_KEY")
                    || candidate.ends_with("_BASE_URL")
                    || candidate == "LLM_MODEL"
                    || candidate == "LLM_PROVIDER"
                {
                    Some(candidate.clone())
                } else {
                    None
                }
            })
            .collect();
        available.sort();
        let available_text = if available.is_empty() {
            "none".to_string()
        } else {
            available.join(", ")
        };
        return error_response(
            StatusCode::BAD_REQUEST,
            format!(
                "Can't map '{}' from current model settings. Available model-backed keys: {}",
                key, available_text
            ),
        );
    };

    let mut values = BTreeMap::new();
    values.insert(key.clone(), value);
    let agent = state.agent.read().await;
    match agent
        .submit_chat_credential_values(request.conversation_id.as_deref(), &values)
        .await
    {
        Ok(followup) => (
            StatusCode::OK,
            Json(serde_json::json!({
                "status": "ok",
                "key": key,
                "followup": followup,
            })),
        )
            .into_response(),
        Err(error) => error_response(StatusCode::BAD_REQUEST, error),
    }
}

pub(super) async fn list_settings_secrets(State(state): State<AppState>) -> Response {
    let (config_dir, data_dir) = {
        let agent = state.agent.read().await;
        (agent.config_dir.clone(), agent.data_dir.clone())
    };
    let manager = match crate::core::config::SecureConfigManager::new_with_data_dir(
        &config_dir,
        Some(&data_dir),
    ) {
        Ok(m) => m,
        Err(e) => {
            return (
                StatusCode::INTERNAL_SERVER_ERROR,
                Json(ErrorResponse {
                    error: format!("Config error: {}", e),
                }),
            )
                .into_response();
        }
    };
    let secrets = match manager.load_secrets() {
        Ok(s) => s,
        Err(e) => {
            return (
                StatusCode::INTERNAL_SERVER_ERROR,
                Json(ErrorResponse {
                    error: format!("Failed to load encrypted secrets: {}", e),
                }),
            )
                .into_response();
        }
    };

    let entries = collect_settings_secret_entries(&secrets);

    let count = entries.len();
    (
        StatusCode::OK,
        Json(serde_json::json!({
            "entries": entries,
            "count": count
        })),
    )
        .into_response()
}

pub(super) async fn reveal_settings_secrets(
    State(state): State<AppState>,
    Json(request): Json<SecretsVaultRevealRequest>,
) -> Response {
    let _ = state;
    let _ = request;
    (
        StatusCode::FORBIDDEN,
        Json(ErrorResponse {
            error: "Full secret reveal is disabled. Secrets Vault only returns masked snippets."
                .to_string(),
        }),
    )
        .into_response()
}

pub(super) async fn upsert_settings_secret(
    State(state): State<AppState>,
    Json(request): Json<SecretsVaultUpsertRequest>,
) -> Response {
    let key = request.key.trim();
    if !is_valid_user_secret_key(key) {
        return (
            StatusCode::BAD_REQUEST,
            Json(ErrorResponse {
                error: "Invalid key. Use letters, numbers, '_', '-', ':' or '.'.".to_string(),
            }),
        )
            .into_response();
    }
    if is_internal_secret_key(key) {
        return (
            StatusCode::BAD_REQUEST,
            Json(ErrorResponse {
                error: "This key is reserved for internal settings.".to_string(),
            }),
        )
            .into_response();
    }

    let value = request.value.trim();
    if value.is_empty() {
        return (
            StatusCode::BAD_REQUEST,
            Json(ErrorResponse {
                error: "Value cannot be empty. Use delete to remove a secret.".to_string(),
            }),
        )
            .into_response();
    }

    let (config_dir, data_dir) = {
        let agent = state.agent.read().await;
        (agent.config_dir.clone(), agent.data_dir.clone())
    };
    if let Err(msg) =
        require_master_password_for_secrets(&config_dir, &data_dir, request.password.as_deref())
    {
        return (StatusCode::UNAUTHORIZED, Json(ErrorResponse { error: msg })).into_response();
    }
    let manager = match crate::core::config::SecureConfigManager::new_with_data_dir(
        &config_dir,
        Some(&data_dir),
    ) {
        Ok(m) => m,
        Err(e) => {
            return (
                StatusCode::INTERNAL_SERVER_ERROR,
                Json(ErrorResponse {
                    error: format!("Config error: {}", e),
                }),
            )
                .into_response();
        }
    };

    if let Err(e) = manager.set_custom_secret(key, Some(value.to_string())) {
        return (
            StatusCode::INTERNAL_SERVER_ERROR,
            Json(ErrorResponse {
                error: format!("Failed to store secret: {}", e),
            }),
        )
            .into_response();
    }

    let followup = if let Some(conversation_id) = request
        .conversation_id
        .as_deref()
        .map(str::trim)
        .filter(|value| !value.is_empty())
    {
        let agent = state.agent.read().await;
        agent.on_secret_saved_followup(conversation_id).await
    } else {
        None
    };

    (
        StatusCode::OK,
        Json(serde_json::json!({
            "status": "ok",
            "key": key,
            "masked": mask_secret_value(value),
            "followup": followup,
        })),
    )
        .into_response()
}

pub(super) async fn delete_settings_secret(
    State(state): State<AppState>,
    Json(request): Json<SecretsVaultDeleteRequest>,
) -> Response {
    let key = request.key.trim();
    if !is_valid_user_secret_key(key) {
        return (
            StatusCode::BAD_REQUEST,
            Json(ErrorResponse {
                error: "Invalid key.".to_string(),
            }),
        )
            .into_response();
    }
    if is_internal_secret_key(key) {
        return (
            StatusCode::BAD_REQUEST,
            Json(ErrorResponse {
                error: "This key is reserved for internal settings.".to_string(),
            }),
        )
            .into_response();
    }

    let (config_dir, data_dir) = {
        let agent = state.agent.read().await;
        (agent.config_dir.clone(), agent.data_dir.clone())
    };
    if let Err(msg) =
        require_master_password_for_secrets(&config_dir, &data_dir, request.password.as_deref())
    {
        return (StatusCode::UNAUTHORIZED, Json(ErrorResponse { error: msg })).into_response();
    }
    let manager = match crate::core::config::SecureConfigManager::new_with_data_dir(
        &config_dir,
        Some(&data_dir),
    ) {
        Ok(m) => m,
        Err(e) => {
            return (
                StatusCode::INTERNAL_SERVER_ERROR,
                Json(ErrorResponse {
                    error: format!("Config error: {}", e),
                }),
            )
                .into_response();
        }
    };

    if let Err(e) = manager.set_custom_secret(key, None) {
        return (
            StatusCode::INTERNAL_SERVER_ERROR,
            Json(ErrorResponse {
                error: format!("Failed to delete secret: {}", e),
            }),
        )
            .into_response();
    }

    (
        StatusCode::OK,
        Json(serde_json::json!({
            "status": "ok",
            "key": key,
            "deleted": true
        })),
    )
        .into_response()
}

/// Get the HTTP API key (masked + full for copying)
pub(super) async fn get_api_key_endpoint(State(state): State<AppState>) -> impl IntoResponse {
    match auth::sync_http_api_key_state(&state, true).await {
        Ok((Some(info), rotated)) => {
            let now = auth::unix_now_ts();
            let remaining_seconds = (info.expires_at - now).max(0);
            Json(serde_json::json!({
                "set": true,
                "masked": auth::mask_api_key_value(&info.key),
                "key": info.key,
                "issued_at_unix": info.issued_at,
                "expires_at_unix": info.expires_at,
                "ttl_seconds": crate::core::config::HTTP_API_KEY_TTL_SECS,
                "remaining_seconds": remaining_seconds,
                "rotated": rotated,
            }))
            .into_response()
        }
        Ok((None, _)) => Json(serde_json::json!({
            "set": false,
            "masked": null,
            "key": null,
        }))
        .into_response(),
        Err(e) => (
            StatusCode::INTERNAL_SERVER_ERROR,
            Json(serde_json::json!({
                "set": false,
                "masked": null,
                "key": null,
                "error": e,
            })),
        )
            .into_response(),
    }
}

/// Regenerate the HTTP API key
pub(super) async fn regenerate_api_key_endpoint(
    State(state): State<AppState>,
) -> impl IntoResponse {
    let (config_dir, data_dir) = {
        let agent = state.agent.read().await;
        (agent.config_dir.clone(), agent.data_dir.clone())
    };
    let secure_config =
        crate::core::config::SecureConfigManager::new_with_data_dir(&config_dir, Some(&data_dir));
    match secure_config.and_then(|sc| sc.regenerate_api_key_info()) {
        Ok(info) => {
            {
                let mut key_guard = state.api_key.write().await;
                *key_guard = Some(info.key.clone());
            }
            {
                let mut exp_guard = state.api_key_expires_at.write().await;
                *exp_guard = Some(info.expires_at);
            }
            {
                let mut agent = state.agent.write().await;
                agent.api_key = Some(info.key.clone());
            }
            let now = auth::unix_now_ts();
            let remaining_seconds = (info.expires_at - now).max(0);
            Json(serde_json::json!({
                "ok": true,
                "masked": auth::mask_api_key_value(&info.key),
                "key": info.key,
                "issued_at_unix": info.issued_at,
                "expires_at_unix": info.expires_at,
                "ttl_seconds": crate::core::config::HTTP_API_KEY_TTL_SECS,
                "remaining_seconds": remaining_seconds,
                "rotated": true,
            }))
            .into_response()
        }
        Err(e) => (
            StatusCode::INTERNAL_SERVER_ERROR,
            Json(serde_json::json!({
                "ok": false,
                "error": e.to_string(),
            })),
        )
            .into_response(),
    }
}
