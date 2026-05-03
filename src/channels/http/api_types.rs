use super::*;

/// Chat response
#[derive(Debug, Serialize)]
pub(super) struct ChatResponse {
    pub response: String,
    pub proof_id: Option<String>,
    pub conversation_id: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub conversation_title: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub run_id: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub run_status: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub trace_id: Option<String>,
    pub total_tokens: i64,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub choices: Vec<crate::core::ClarificationChoice>,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub degradation: Vec<crate::core::DegradationNote>,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub attempted_models: Vec<crate::core::ModelAttemptRecord>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub user_outcome: Option<crate::core::UserFacingOutcome>,
}

/// Agent status response
#[derive(Debug, Serialize)]
pub(super) struct StatusResponse {
    pub did: String,
    pub memory_entries: usize,
    pub skills_loaded: usize,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub actions_loaded: Option<usize>,
    pub tasks_pending: usize,
    pub version: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub update: Option<UpdateStatusResponse>,
}

#[derive(Debug, Serialize, Clone)]
pub(super) struct UpdateStatusResponse {
    pub state: String,
    pub apply_supported: bool,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub apply_message: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub latest_version: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub latest_tag: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub release_url: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub checked_at: Option<String>,
}

#[derive(Debug, Clone)]
pub(super) struct ReleaseUpdateCache {
    pub(super) last_checked_at: Option<Instant>,
    pub(super) summary: UpdateStatusResponse,
    pub(super) refreshing: bool,
}

impl Default for ReleaseUpdateCache {
    fn default() -> Self {
        Self {
            last_checked_at: None,
            summary: UpdateStatusResponse::checking(),
            refreshing: false,
        }
    }
}

impl UpdateStatusResponse {
    fn checking() -> Self {
        Self {
            state: "checking".to_string(),
            apply_supported: false,
            apply_message: None,
            latest_version: None,
            latest_tag: None,
            release_url: None,
            checked_at: None,
        }
    }

    fn unavailable() -> Self {
        Self {
            state: "unavailable".to_string(),
            apply_supported: false,
            apply_message: None,
            latest_version: None,
            latest_tag: None,
            release_url: None,
            checked_at: Some(chrono::Utc::now().to_rfc3339()),
        }
    }

    fn with_apply_support(mut self, apply_supported: bool, apply_message: Option<String>) -> Self {
        self.apply_supported = apply_supported;
        self.apply_message = apply_message;
        self
    }
}

pub(super) fn release_update_apply_support(state: &AppState) -> (bool, Option<String>) {
    if !matches!(stack_role().as_deref(), Some("control-plane" | "control")) {
        return (
            false,
            Some("Web UI updates are available only for managed Docker installs.".to_string()),
        );
    }

    if !crate::core::release_updates::ui_update_supported_image(
        &crate::core::runtime_image::default_runtime_image(),
    ) {
        return (
            false,
            Some(
                "This deployment is using a local or custom image. Update it from the CLI instead."
                    .to_string(),
            ),
        );
    }

    if state.executor_client.is_none() && build_executor_client().ok().flatten().is_none() {
        return (
            false,
            Some("Internal executor service is unavailable for web-based updates.".to_string()),
        );
    }

    (true, None)
}

pub(super) async fn fetch_release_update_summary_uncached() -> UpdateStatusResponse {
    let client = match reqwest::Client::builder()
        .timeout(RELEASE_UPDATE_REQUEST_TIMEOUT)
        .build()
    {
        Ok(client) => client,
        Err(error) => {
            tracing::debug!("Release update client creation failed: {}", error);
            return UpdateStatusResponse::unavailable();
        }
    };

    let latest = match crate::core::release_updates::fetch_latest_release_info(
        &client,
        &crate::core::release_updates::release_repo_slug(),
    )
    .await
    {
        Ok(latest) => latest,
        Err(error) => {
            tracing::debug!("Release update check unavailable: {}", error);
            return UpdateStatusResponse::unavailable();
        }
    };

    let state = match crate::core::release_updates::is_release_version_newer(
        env!("CARGO_PKG_VERSION"),
        &latest.version,
    ) {
        Some(true) => "available",
        Some(false) => "current",
        None => {
            tracing::debug!(
                "Release update version comparison failed for current={} latest={}",
                env!("CARGO_PKG_VERSION"),
                latest.version
            );
            return UpdateStatusResponse::unavailable();
        }
    };

    UpdateStatusResponse {
        state: state.to_string(),
        apply_supported: false,
        apply_message: None,
        latest_version: Some(latest.version),
        latest_tag: Some(latest.tag_name),
        release_url: Some(latest.html_url),
        checked_at: Some(chrono::Utc::now().to_rfc3339()),
    }
}

pub(super) async fn current_release_update_summary(state: &AppState) -> UpdateStatusResponse {
    let (apply_supported, apply_message) = release_update_apply_support(state);

    {
        let cache = state.release_update_cache.read().await;
        let fresh = cache
            .last_checked_at
            .is_some_and(|checked| checked.elapsed() < RELEASE_UPDATE_CHECK_INTERVAL);
        if fresh || cache.refreshing {
            return cache
                .summary
                .clone()
                .with_apply_support(apply_supported, apply_message);
        }
    }

    {
        let mut cache = state.release_update_cache.write().await;
        let fresh = cache
            .last_checked_at
            .is_some_and(|checked| checked.elapsed() < RELEASE_UPDATE_CHECK_INTERVAL);
        if fresh || cache.refreshing {
            return cache
                .summary
                .clone()
                .with_apply_support(apply_supported, apply_message);
        }
        cache.refreshing = true;
    }

    let fresh = fetch_release_update_summary_uncached().await;
    let response = fresh
        .clone()
        .with_apply_support(apply_supported, apply_message.clone());

    let mut cache = state.release_update_cache.write().await;
    cache.last_checked_at = Some(Instant::now());
    cache.summary = fresh;
    cache.refreshing = false;
    response
}

/// User profile response
#[derive(Debug, Serialize)]
pub(super) struct ProfileResponse {
    pub name: Option<String>,
    pub location: Option<String>,
    pub timezone: Option<String>,
    pub language: Option<String>,
    pub tone: Option<String>,
    pub email_format: Option<String>,
    pub preferences: Option<String>,
    pub priority_focus: Option<String>,
    pub onboarding_complete: bool,
    pub personalization_dismissed: bool,
}

#[derive(Debug, Serialize)]
pub(super) struct TaskInfo {
    pub id: String,
    pub description: String,
    pub action: String,
    pub arguments: serde_json::Value,
    pub status: String,
    pub task_kind: String,
    pub task_kind_label: String,
    pub scheduled_for: Option<String>,
    pub cron: Option<String>,
    pub result: Option<String>,
    pub created_at: String,
}

#[derive(Debug, Serialize)]
pub(super) struct AutomationObjectInfo {
    pub id: String,
    pub kind: String,
    pub title: String,
    pub subtitle: Option<String>,
    pub status: String,
    pub detail: Option<String>,
    pub created_at: Option<String>,
    pub next_run_at: Option<String>,
    pub view: String,
    pub url: Option<String>,
    pub enabled: Option<bool>,
    pub connected: Option<bool>,
}

#[derive(Debug, Serialize)]
pub(super) struct AutomationRunInfo {
    pub id: String,
    pub automation_id: String,
    pub kind: String,
    pub title: String,
    pub action: String,
    pub trigger: String,
    pub status: String,
    pub current_status: Option<String>,
    pub attempt: u32,
    pub started_at: String,
    pub completed_at: Option<String>,
    pub duration_ms: Option<u64>,
    pub summary: String,
    pub output_preview: Option<String>,
    pub error: Option<String>,
    pub next_retry_at: Option<String>,
    pub conversation_id: Option<String>,
    pub view: String,
}

#[derive(Debug, Default, Serialize)]
pub(super) struct AutomationInventoryTotals {
    pub total: usize,
    pub tasks: usize,
    pub watchers: usize,
    pub apps: usize,
    pub integrations: usize,
}

/// Create task request
#[derive(Debug, Deserialize)]
pub(super) struct CreateTaskRequest {
    pub description: String,
    pub action: String,
    pub arguments: serde_json::Value,
    /// Cron expression for scheduling (e.g., "*/5 * * * *" for every 5 minutes)
    pub cron: Option<String>,
    /// Approval policy: "auto" or "require"
    pub approval: Option<String>,
    #[serde(default)]
    pub allow_duplicate: bool,
}

/// Update task request
#[derive(Debug, Deserialize)]
pub(super) struct UpdateTaskRequest {
    pub description: Option<String>,
    pub arguments: Option<serde_json::Value>,
    pub cron: Option<String>,
}

#[derive(Debug, Deserialize, Default)]
pub(super) struct ApprovalDecisionRequest {
    pub comment: Option<String>,
    pub reason: Option<String>,
}

#[derive(Debug, Deserialize, Default)]
pub(super) struct CreateBackgroundSessionRequest {
    #[serde(default)]
    pub title: Option<String>,
    pub objective: String,
    #[serde(default)]
    pub summary: Option<String>,
    #[serde(default)]
    pub current_focus: Option<String>,
    #[serde(default)]
    pub waiting_on: Option<String>,
    #[serde(default)]
    pub next_expected_action: Option<String>,
    #[serde(default)]
    pub working_memory: Option<String>,
    #[serde(default)]
    pub preferred_delivery_channel: Option<String>,
    #[serde(default)]
    pub channel: Option<String>,
    #[serde(default)]
    pub conversation_id: Option<String>,
    #[serde(default)]
    pub task_ids: Vec<String>,
    #[serde(default)]
    pub watcher_ids: Vec<String>,
}

#[derive(Debug, Deserialize, Default)]
pub(super) struct UpdateBackgroundSessionRequest {
    #[serde(default)]
    pub title: Option<String>,
    #[serde(default)]
    pub objective: Option<String>,
    #[serde(default)]
    pub status: Option<String>,
    #[serde(default)]
    pub summary: Option<String>,
    #[serde(default)]
    pub current_focus: Option<String>,
    #[serde(default)]
    pub waiting_on: Option<String>,
    #[serde(default)]
    pub next_expected_action: Option<String>,
    #[serde(default)]
    pub working_memory: Option<String>,
    #[serde(default)]
    pub last_error: Option<String>,
    #[serde(default)]
    pub preferred_delivery_channel: Option<String>,
    #[serde(default)]
    pub policy: Option<crate::core::BackgroundSessionPolicy>,
}

#[derive(Debug, Deserialize, Default)]
pub(super) struct BackgroundSessionLinkRequest {
    #[serde(default)]
    pub task_ids: Vec<String>,
    #[serde(default)]
    pub watcher_ids: Vec<String>,
}

/// Plan task request (LLM-assisted)
#[derive(Debug, Deserialize)]
pub(super) struct PlanTaskRequest {
    pub description: String,
    pub prompt: Option<String>,
}

/// Plan task response
#[derive(Debug, Serialize)]
pub(super) struct PlanTaskResponse {
    pub plan: crate::core::ExecutionPlan,
}

#[derive(Debug, Serialize)]
pub(super) struct CodexCliOAuthStartResponse {
    pub started: bool,
    pub running: bool,
    pub opened_browser: bool,
    pub auth_url: String,
    pub device_code: String,
    pub message: String,
}

#[derive(Debug, Serialize)]
pub(super) struct CodexCliOAuthStatusResponse {
    pub connected: bool,
    pub has_api_key: bool,
    pub running: bool,
    pub auth_url: String,
    pub device_code: String,
    pub message: String,
}

/// Settings response (for GET)
#[derive(Debug, Serialize)]
pub(super) struct SettingsResponse {
    pub bot_name: String,
    pub personality: String,
    pub timezone: Option<String>,
    pub language: Option<String>,
    pub tone: Option<String>,
    pub email_format: Option<String>,
    pub email: EmailSettingsResponse,
    pub daily_brief_enabled: bool,
    pub daily_brief_time: String,
    pub daily_brief_channel: String,
    pub arkreflect_daily_digest_enabled: bool,
    // Primary LLM (legacy)
    pub llm_provider: String,
    pub llm_model: String,
    pub llm_base_url: Option<String>,
    pub has_api_key: bool,
    // Fallback LLM (legacy)
    pub llm_fallback_provider: Option<String>,
    pub llm_fallback_model: Option<String>,
    pub llm_fallback_base_url: Option<String>,
    pub has_fallback_api_key: bool,
    pub default_model_input_mode: String,
    pub current_chat_pii_policy: String,
    pub request_scoped_sensitive_approval_enabled: bool,
    // Model pool
    pub model_pool: Vec<ModelSlotSummary>,
    pub smart_routing: bool,
    // Embeddings
    pub embeddings_provider: String,
    pub embeddings_model: String,
    pub embeddings_base_url: Option<String>,
    pub embeddings_has_api_key: bool,
    pub embeddings_status: String,
    // Telegram
    pub telegram_enabled: bool,
    pub has_telegram_token: bool,
    pub telegram_delivery_ready: bool,
    pub telegram_allowed_users: Vec<i64>,
    // Slack
    pub slack_enabled: bool,
    pub has_slack_bot_token: bool,
    pub has_slack_signing_secret: bool,
    pub slack_api_base_url: String,
    pub slack_default_channel_id: String,
    pub slack_default_thread_ts: Option<String>,
    pub slack_workspace_id: Option<String>,
    pub slack_workspace_name: Option<String>,
    pub slack_delivery_ready: bool,
    // Discord
    pub discord_enabled: bool,
    pub has_discord_bot_token: bool,
    pub discord_api_base_url: String,
    pub discord_default_channel_id: String,
    pub discord_default_thread_id: Option<String>,
    pub discord_guild_id: Option<String>,
    pub discord_application_id: Option<String>,
    pub discord_webhook_url: String,
    pub discord_delivery_ready: bool,
    // Matrix
    pub matrix_enabled: bool,
    pub has_matrix_access_token: bool,
    pub matrix_homeserver_url: String,
    pub matrix_user_id: String,
    pub matrix_device_id: Option<String>,
    pub matrix_account_id: Option<String>,
    pub matrix_default_room_id: Option<String>,
    pub matrix_sync_timeout_ms: u64,
    pub matrix_limit: usize,
    pub matrix_user_agent: Option<String>,
    pub matrix_delivery_ready: bool,
    // Teams
    pub teams_enabled: bool,
    pub has_teams_access_token: bool,
    pub teams_service_url: String,
    pub teams_bot_app_id: Option<String>,
    pub teams_bot_name: Option<String>,
    pub teams_tenant_id: Option<String>,
    pub teams_team_id: Option<String>,
    pub teams_channel_id: Option<String>,
    pub teams_chat_id: Option<String>,
    pub teams_graph_base_url: Option<String>,
    pub teams_delivery_mode: String,
    pub teams_timeout_secs: u64,
    pub teams_user_agent: Option<String>,
    pub teams_delivery_ready: bool,
    // WhatsApp
    pub whatsapp_enabled: bool,
    pub whatsapp_mode: String,
    pub has_whatsapp_token: bool,
    pub has_whatsapp_app_secret: bool,
    pub has_whatsapp_verify_token: bool,
    pub has_whatsapp_bridge_token: bool,
    pub whatsapp_delivery_ready: bool,
    pub whatsapp_phone_number_id: String,
    pub whatsapp_bridge_runtime: String,
    pub whatsapp_bridge_url: String,
    pub whatsapp_dm_policy: String,
    pub whatsapp_allowed_numbers: Vec<String>,
    // Google Chat
    pub google_chat_enabled: bool,
    pub has_google_chat_access_token: bool,
    pub has_google_chat_verify_token: bool,
    pub google_chat_api_base_url: String,
    pub google_chat_space: Option<String>,
    pub google_chat_thread_key: Option<String>,
    pub google_chat_app_id: Option<String>,
    pub google_chat_bot_name: Option<String>,
    pub google_chat_delivery_ready: bool,
    // Signal
    pub signal_enabled: bool,
    pub has_signal_bridge_token: bool,
    pub signal_bridge_url: String,
    pub signal_default_recipient: String,
    pub signal_default_group_id: String,
    pub signal_delivery_ready: bool,
    // iMessage
    pub imessage_enabled: bool,
    pub has_imessage_bridge_token: bool,
    pub imessage_bridge_url: String,
    pub imessage_default_chat_id: String,
    pub imessage_default_handle: String,
    pub imessage_delivery_ready: bool,
    // LINE
    pub line_enabled: bool,
    pub has_line_access_token: bool,
    pub has_line_channel_secret: bool,
    pub line_api_base_url: String,
    pub line_default_target: Option<String>,
    pub line_user_agent: Option<String>,
    pub line_delivery_ready: bool,
    // WeChat
    pub wechat_enabled: bool,
    pub has_wechat_bridge_token: bool,
    pub wechat_bridge_url: String,
    pub wechat_default_target_id: String,
    pub wechat_delivery_ready: bool,
    // QQ
    pub qq_enabled: bool,
    pub has_qq_bridge_token: bool,
    pub qq_bridge_url: String,
    pub qq_default_target_id: String,
    pub qq_delivery_ready: bool,
    pub auto_approve: Vec<String>,
    // Search
    pub search_provider_order: Vec<String>,
    pub search_serper_configured: bool,
    pub search_brave_configured: bool,
    pub search_exa_configured: bool,
    pub search_tavily_configured: bool,
    pub search_perplexity_configured: bool,
    pub search_firecrawl_configured: bool,
    pub search_lightpanda_available: bool,
    pub search_searxng_base_url: String,
    pub search_builtin_cooldown_hours: u64,
    pub settings_complete: bool,
    pub tunnel_active: bool,
    pub deployment_mode: String,
    pub public_app_bind_addr: Option<String>,
    pub public_app_base_url: Option<String>,
    pub data_lifecycle: DataLifecycleSettings,
    pub observability: observability::ObservabilitySettingsResponse,
}

#[derive(Debug, Serialize)]
pub(super) struct EmailSettingsResponse {
    pub provider: String,
    pub to_address: Option<String>,
    pub from_address: Option<String>,
    pub domain: Option<String>,
    pub available_backends: Vec<String>,
    pub auto_resolves_to: Option<String>,
    pub delivery_ready: bool,
    pub transport: EmailTransportSettingsResponse,
    pub auth: EmailAuthSettingsResponse,
}

#[derive(Debug, Serialize)]
pub(super) struct EmailTransportSettingsResponse {
    pub kind: String,
    pub http: EmailHttpTransportSettingsResponse,
    pub smtp: EmailSmtpTransportSettingsResponse,
}

#[derive(Debug, Serialize)]
pub(super) struct EmailHttpTransportSettingsResponse {
    pub base_url: Option<String>,
    pub send_path: Option<String>,
}

#[derive(Debug, Serialize)]
pub(super) struct EmailSmtpTransportSettingsResponse {
    pub host: String,
    pub port: u16,
    pub security: String,
}

#[derive(Debug, Serialize)]
pub(super) struct EmailAuthSettingsResponse {
    pub kind: String,
    pub header_name: Option<String>,
    pub scheme: Option<String>,
    pub basic_username: String,
    pub aws_access_key_id: String,
    pub aws_region: Option<String>,
    pub aws_service: Option<String>,
    pub has_api_key: bool,
    pub has_basic_password: bool,
    pub has_aws_secret_access_key: bool,
    pub has_aws_session_token: bool,
}

/// Model slot summary for API responses
#[derive(Debug, Serialize)]
pub(super) struct ModelSlotSummary {
    pub id: String,
    pub label: String,
    pub role: String,
    pub provider: String,
    pub model: String,
    pub base_url: Option<String>,
    pub has_api_key: bool,
    pub enabled: bool,
}

/// Request to create/update a model slot
#[derive(Debug, Deserialize)]
pub(super) struct ModelSlotRequest {
    pub label: String,
    pub role: String,
    pub provider: String,
    pub model: String,
    pub base_url: Option<String>,
    pub api_key: Option<String>,
    #[serde(default)]
    pub clear_api_key: Option<bool>,
    pub enabled: Option<bool>,
}

#[derive(Debug, Deserialize)]
pub(super) struct ModelConnectionTestRequest {
    pub id: Option<String>,
    #[serde(flatten)]
    pub request: ModelSlotRequest,
}

/// Settings update request (for POST)
#[derive(Debug, Deserialize)]
pub(super) struct SettingsUpdate {
    pub bot_name: Option<String>,
    pub personality: Option<String>,
    pub timezone: Option<String>,
    pub language: Option<String>,
    pub tone: Option<String>,
    pub email_format: Option<String>,
    #[serde(default)]
    pub email: Option<EmailSettingsUpdate>,
    #[serde(default)]
    pub daily_brief_enabled: Option<bool>,
    #[serde(default)]
    pub daily_brief_time: Option<String>,
    pub daily_brief_channel: Option<String>,
    #[serde(default)]
    pub arkreflect_daily_digest_enabled: Option<bool>,
    /// Model pool routing behavior (if false, always use primary)
    #[serde(default)]
    pub smart_routing: Option<bool>,
    // Embeddings
    pub embeddings_provider: Option<String>,
    pub embeddings_model: Option<String>,
    pub embeddings_base_url: Option<String>,
    pub embeddings_api_key: Option<String>,
    // Primary LLM
    #[serde(default)]
    pub llm_provider: Option<String>,
    #[serde(default)]
    pub llm_model: Option<String>,
    pub llm_base_url: Option<String>,
    pub llm_api_key: Option<String>,
    // Fallback LLM (used if primary fails)
    pub llm_fallback_provider: Option<String>,
    pub llm_fallback_model: Option<String>,
    pub llm_fallback_base_url: Option<String>,
    pub llm_fallback_api_key: Option<String>,
    #[serde(default)]
    pub default_model_input_mode: Option<String>,
    #[serde(default)]
    pub current_chat_pii_policy: Option<String>,
    #[serde(default)]
    pub request_scoped_sensitive_approval_enabled: Option<bool>,
    // Telegram
    #[serde(default)]
    pub telegram_enabled: Option<bool>,
    pub telegram_bot_token: Option<String>,
    pub telegram_allowed_users: Option<Vec<i64>>,
    // Slack
    #[serde(default)]
    pub slack_enabled: Option<bool>,
    pub slack_bot_token: Option<String>,
    pub slack_signing_secret: Option<String>,
    pub slack_api_base_url: Option<String>,
    pub slack_default_channel_id: Option<String>,
    pub slack_default_thread_ts: Option<String>,
    pub slack_workspace_id: Option<String>,
    pub slack_workspace_name: Option<String>,
    // Discord
    #[serde(default)]
    pub discord_enabled: Option<bool>,
    pub discord_bot_token: Option<String>,
    pub discord_api_base_url: Option<String>,
    pub discord_default_channel_id: Option<String>,
    pub discord_default_thread_id: Option<String>,
    pub discord_guild_id: Option<String>,
    pub discord_application_id: Option<String>,
    pub discord_webhook_url: Option<String>,
    // Matrix
    #[serde(default)]
    pub matrix_enabled: Option<bool>,
    pub matrix_homeserver_url: Option<String>,
    pub matrix_access_token: Option<String>,
    pub matrix_user_id: Option<String>,
    pub matrix_device_id: Option<String>,
    pub matrix_account_id: Option<String>,
    pub matrix_default_room_id: Option<String>,
    pub matrix_sync_timeout_ms: Option<u64>,
    pub matrix_limit: Option<usize>,
    pub matrix_user_agent: Option<String>,
    // Teams
    #[serde(default)]
    pub teams_enabled: Option<bool>,
    pub teams_service_url: Option<String>,
    pub teams_access_token: Option<String>,
    pub teams_bot_app_id: Option<String>,
    pub teams_bot_name: Option<String>,
    pub teams_tenant_id: Option<String>,
    pub teams_team_id: Option<String>,
    pub teams_channel_id: Option<String>,
    pub teams_chat_id: Option<String>,
    pub teams_graph_base_url: Option<String>,
    pub teams_delivery_mode: Option<String>,
    pub teams_timeout_secs: Option<u64>,
    pub teams_user_agent: Option<String>,
    // WhatsApp
    #[serde(default)]
    pub whatsapp_enabled: Option<bool>,
    #[serde(default)]
    pub whatsapp_mode: Option<String>,
    pub whatsapp_access_token: Option<String>,
    pub whatsapp_app_secret: Option<String>,
    pub whatsapp_phone_number_id: Option<String>,
    pub whatsapp_verify_token: Option<String>,
    pub whatsapp_bridge_runtime: Option<String>,
    pub whatsapp_bridge_token: Option<String>,
    pub whatsapp_bridge_url: Option<String>,
    #[serde(default)]
    pub whatsapp_dm_policy: Option<String>,
    #[serde(default)]
    pub whatsapp_allowed_numbers: Option<Vec<String>>,
    // Google Chat
    #[serde(default)]
    pub google_chat_enabled: Option<bool>,
    pub google_chat_access_token: Option<String>,
    pub google_chat_verify_token: Option<String>,
    pub google_chat_api_base_url: Option<String>,
    pub google_chat_space: Option<String>,
    pub google_chat_thread_key: Option<String>,
    pub google_chat_app_id: Option<String>,
    pub google_chat_bot_name: Option<String>,
    // Signal
    #[serde(default)]
    pub signal_enabled: Option<bool>,
    pub signal_bridge_token: Option<String>,
    pub signal_bridge_url: Option<String>,
    pub signal_default_recipient: Option<String>,
    pub signal_default_group_id: Option<String>,
    // iMessage
    #[serde(default)]
    pub imessage_enabled: Option<bool>,
    pub imessage_bridge_token: Option<String>,
    pub imessage_bridge_url: Option<String>,
    pub imessage_default_chat_id: Option<String>,
    pub imessage_default_handle: Option<String>,
    // LINE
    #[serde(default)]
    pub line_enabled: Option<bool>,
    pub line_channel_access_token: Option<String>,
    pub line_channel_secret: Option<String>,
    pub line_api_base_url: Option<String>,
    pub line_default_target: Option<String>,
    pub line_user_agent: Option<String>,
    // WeChat
    #[serde(default)]
    pub wechat_enabled: Option<bool>,
    pub wechat_bridge_token: Option<String>,
    pub wechat_bridge_url: Option<String>,
    pub wechat_default_target_id: Option<String>,
    // QQ
    #[serde(default)]
    pub qq_enabled: Option<bool>,
    pub qq_bridge_token: Option<String>,
    pub qq_bridge_url: Option<String>,
    pub qq_default_target_id: Option<String>,
    /// Actions that run without approval
    #[serde(default)]
    pub auto_approve: Option<Vec<String>>,
    #[serde(default)]
    pub deployment_mode: Option<String>,
    #[serde(default)]
    pub public_app_bind_addr: Option<String>,
    #[serde(default)]
    pub public_app_base_url: Option<String>,
    /// Media generation provider API keys (all stored encrypted)
    #[serde(default)]
    pub media_providers: std::collections::HashMap<String, String>,
    /// Optional compatible endpoint overrides for known media provider adapters.
    #[serde(default)]
    pub media_provider_base_urls: std::collections::HashMap<String, String>,
    /// Default provider for image generation
    pub default_image_provider: Option<String>,
    /// Image model name
    pub image_model: Option<String>,
    /// Fallback provider for image generation
    pub fallback_image_provider: Option<String>,
    /// Default provider for video generation
    pub default_video_provider: Option<String>,
    /// Fallback provider for video generation
    pub fallback_video_provider: Option<String>,
    /// Search: configured provider precedence
    #[serde(default)]
    pub search_provider_order: Option<Vec<String>>,
    /// Search: Serper API key
    #[serde(default)]
    pub search_serper_key: Option<String>,
    #[serde(default)]
    pub clear_search_serper_key: Option<bool>,
    /// Search: Brave API key
    #[serde(default)]
    pub search_brave_key: Option<String>,
    #[serde(default)]
    pub clear_search_brave_key: Option<bool>,
    /// Search: Exa API key
    #[serde(default)]
    pub search_exa_key: Option<String>,
    #[serde(default)]
    pub clear_search_exa_key: Option<bool>,
    /// Search: Tavily API key
    #[serde(default)]
    pub search_tavily_key: Option<String>,
    #[serde(default)]
    pub clear_search_tavily_key: Option<bool>,
    /// Search: Perplexity API key
    #[serde(default)]
    pub search_perplexity_key: Option<String>,
    #[serde(default)]
    pub clear_search_perplexity_key: Option<bool>,
    /// Search: Firecrawl API key
    #[serde(default)]
    pub search_firecrawl_key: Option<String>,
    #[serde(default)]
    pub clear_search_firecrawl_key: Option<bool>,
    /// Search: SearXNG base URL
    #[serde(default)]
    pub search_searxng_base_url: Option<String>,
    #[serde(default)]
    pub data_lifecycle: Option<DataLifecycleSettingsUpdate>,
    #[serde(default)]
    pub observability: Option<observability::ObservabilitySettingsUpdate>,
}

#[derive(Debug, Deserialize, Default)]
pub(super) struct EmailSettingsUpdate {
    pub provider: Option<String>,
    pub to_address: Option<String>,
    pub from_address: Option<String>,
    pub domain: Option<String>,
    #[serde(default)]
    pub transport: Option<EmailTransportSettingsUpdate>,
    #[serde(default)]
    pub auth: Option<EmailAuthSettingsUpdate>,
}

#[derive(Debug, Deserialize, Default)]
pub(super) struct EmailTransportSettingsUpdate {
    pub kind: Option<String>,
    #[serde(default)]
    pub http: Option<EmailHttpTransportSettingsUpdate>,
    #[serde(default)]
    pub smtp: Option<EmailSmtpTransportSettingsUpdate>,
}

#[derive(Debug, Deserialize, Default)]
pub(super) struct EmailHttpTransportSettingsUpdate {
    pub base_url: Option<String>,
    pub send_path: Option<String>,
}

#[derive(Debug, Deserialize, Default)]
pub(super) struct EmailSmtpTransportSettingsUpdate {
    pub host: Option<String>,
    pub port: Option<u16>,
    pub security: Option<String>,
}

#[derive(Debug, Deserialize, Default)]
pub(super) struct EmailAuthSettingsUpdate {
    pub kind: Option<String>,
    pub api_key: Option<String>,
    pub header_name: Option<String>,
    pub scheme: Option<String>,
    pub basic_username: Option<String>,
    pub basic_password: Option<String>,
    pub aws_access_key_id: Option<String>,
    pub aws_secret_access_key: Option<String>,
    pub aws_session_token: Option<String>,
    pub aws_region: Option<String>,
    pub aws_service: Option<String>,
}

#[derive(Debug, Deserialize)]
pub(super) struct ProfileOnboardingUpdate {
    pub preferred_name: String,
    pub timezone: String,
    pub tone: String,
    pub priority_focus: String,
}

#[derive(Debug, Clone, Default, Deserialize)]
pub(super) struct DataLifecycleSettingsUpdate {
    #[serde(default)]
    pub cleanup_enabled: Option<bool>,
    #[serde(default)]
    pub notifications_cleanup_enabled: Option<bool>,
    #[serde(default)]
    pub logs_cleanup_enabled: Option<bool>,
    #[serde(default)]
    pub notifications_retention_days: Option<u64>,
    #[serde(default)]
    pub notification_cleanup_interval_secs: Option<u64>,
    #[serde(default)]
    pub execution_trace_retention_days: Option<u64>,
    #[serde(default)]
    pub execution_proof_retention_days: Option<u64>,
    #[serde(default)]
    pub operational_log_retention_days: Option<u64>,
    #[serde(default)]
    pub security_log_retention_days: Option<u64>,
    #[serde(default)]
    pub approval_log_retention_days: Option<u64>,
    #[serde(default)]
    pub swarm_delegation_retention_days: Option<u64>,
    #[serde(default)]
    pub llm_usage_retention_days: Option<u64>,
    #[serde(default)]
    pub terminal_task_retention_days: Option<u64>,
    #[serde(default)]
    pub execution_run_retention_days: Option<u64>,
    #[serde(default)]
    pub background_session_retention_days: Option<u64>,
    #[serde(default)]
    pub browser_session_retention_days: Option<u64>,
    #[serde(default)]
    pub automation_run_retention_days: Option<u64>,
    #[serde(default)]
    pub message_retention_days: Option<u64>,
    #[serde(default)]
    pub experience_run_retention_days: Option<u64>,
    #[serde(default)]
    pub experience_edge_retention_days: Option<u64>,
    #[serde(default)]
    pub learning_candidate_retention_days: Option<u64>,
    #[serde(default)]
    pub experience_item_retention_days: Option<u64>,
    #[serde(default)]
    pub procedural_pattern_retention_days: Option<u64>,
    #[serde(default)]
    pub recall_event_retention_days: Option<u64>,
    #[serde(default)]
    pub recall_test_retention_days: Option<u64>,
    #[serde(default)]
    pub housekeeping_interval_secs: Option<u64>,
    #[serde(default)]
    pub security_cleanup_interval_days: Option<u64>,
    #[serde(default)]
    pub security_cleanup_idle_threshold_secs: Option<u64>,
}

pub(super) fn model_input_privacy_mode_label(
    mode: crate::security::ModelInputPrivacyMode,
) -> &'static str {
    match mode {
        crate::security::ModelInputPrivacyMode::DefaultRedact => "default_redact",
        crate::security::ModelInputPrivacyMode::ZeroExposure => "zero_exposure",
        crate::security::ModelInputPrivacyMode::SecretsOnly => "secrets_only",
    }
}

pub(super) fn parse_model_input_privacy_mode(
    value: &str,
) -> std::result::Result<crate::security::ModelInputPrivacyMode, String> {
    match value.trim().to_ascii_lowercase().as_str() {
        "" | "default_redact" | "default-redact" => {
            Ok(crate::security::ModelInputPrivacyMode::DefaultRedact)
        }
        "zero_exposure" | "zero-exposure" => {
            Ok(crate::security::ModelInputPrivacyMode::ZeroExposure)
        }
        "secrets_only" | "secrets-only" => Ok(crate::security::ModelInputPrivacyMode::SecretsOnly),
        other => Err(format!(
            "default_model_input_mode must be one of default_redact, zero_exposure, or secrets_only (got '{other}')"
        )),
    }
}

pub(super) fn current_chat_pii_policy_label(
    policy: crate::security::CurrentChatPiiPolicy,
) -> &'static str {
    match policy {
        crate::security::CurrentChatPiiPolicy::RawCurrentTurn => "raw_current_turn",
        crate::security::CurrentChatPiiPolicy::MaskChatPii => "mask_chat_pii",
        crate::security::CurrentChatPiiPolicy::BlockSensitiveChat => "block_sensitive_chat",
    }
}

pub(super) fn parse_current_chat_pii_policy(
    value: &str,
) -> std::result::Result<crate::security::CurrentChatPiiPolicy, String> {
    match value.trim().to_ascii_lowercase().as_str() {
        "" | "raw_current_turn" | "raw-current-turn" => {
            Ok(crate::security::CurrentChatPiiPolicy::RawCurrentTurn)
        }
        "mask_chat_pii" | "mask-chat-pii" => Ok(crate::security::CurrentChatPiiPolicy::MaskChatPii),
        "block_sensitive_chat" | "block-sensitive-chat" => {
            Ok(crate::security::CurrentChatPiiPolicy::BlockSensitiveChat)
        }
        other => Err(format!(
            "current_chat_pii_policy must be one of raw_current_turn, mask_chat_pii, or block_sensitive_chat (got '{other}')"
        )),
    }
}

#[derive(Debug, Serialize)]
pub(super) struct GoogleWorkspaceOAuthClientSettingsResponse {
    pub(super) configured: bool,
    pub(super) source: String,
    pub(super) source_label: String,
    pub(super) managed_externally: bool,
    pub(super) client_id_hint: Option<String>,
    pub(super) secret_configured: bool,
    pub(super) redirect_uri: String,
}

#[derive(Debug, Deserialize)]
pub(super) struct GoogleWorkspaceOAuthClientSettingsUpdate {
    #[serde(default)]
    pub(super) credentials_json: Option<String>,
    #[serde(default)]
    pub(super) client_id: Option<String>,
    #[serde(default)]
    pub(super) client_secret: Option<String>,
    #[serde(default)]
    pub(super) clear: bool,
}

#[derive(Debug, Serialize)]
pub(super) struct MediaSettingsResponse {
    pub(super) configured: Vec<String>,
    pub(super) default_image_provider: Option<String>,
    pub(super) image_model: Option<String>,
    pub(super) fallback_image_provider: Option<String>,
    pub(super) default_video_provider: Option<String>,
    pub(super) fallback_video_provider: Option<String>,
    pub(super) provider_base_urls: std::collections::HashMap<String, String>,
}

#[derive(Debug, Deserialize)]
pub(super) struct GoalLoopRequest {
    pub(super) goal: String,
    #[serde(default)]
    pub(super) constraints: Option<String>,
    #[serde(default)]
    pub(super) due_date: Option<String>,
    #[serde(default)]
    pub(super) report_cron: Option<String>,
    #[serde(default)]
    pub(super) preview_only: bool,
    #[serde(default)]
    pub(super) plan_override: Option<serde_json::Value>,
}

#[derive(Debug, Deserialize)]
pub(super) struct GoalReportNowRequest {
    pub(super) goal_id: String,
}

#[derive(Debug, Deserialize, Default)]
pub(super) struct InboxTriageRequest {
    #[serde(default)]
    pub(super) messages: Vec<serde_json::Value>,
    #[serde(default)]
    pub(super) labels: Option<Vec<String>>,
}

#[derive(Debug, Deserialize)]
pub(super) struct TimelineRollbackRequest {
    pub(super) event_id: String,
    #[serde(default)]
    pub(super) operation: Option<String>,
}

#[derive(Debug, Deserialize)]
pub(super) struct KnowledgeQueryRequest {
    pub(super) query: String,
    #[serde(default)]
    pub(super) limit: Option<usize>,
}

#[derive(Debug, Deserialize)]
pub(super) struct TrustEvaluateRequest {
    pub(super) action_kind: String,
    #[serde(default)]
    pub(super) payload: serde_json::Value,
}

#[derive(Debug, Deserialize)]
pub(super) struct VoiceCommandRequest {
    pub(super) command: String,
    #[serde(default)]
    pub(super) action_id: Option<String>,
    #[serde(default)]
    pub(super) conversation_id: Option<String>,
}

#[derive(Debug, Deserialize)]
pub(super) struct CodeExecuteRequest {
    pub(super) language: String,
    pub(super) code: String,
    #[serde(default)]
    pub(super) env: HashMap<String, String>,
    #[serde(default)]
    pub(super) files: Vec<String>,
    #[serde(default)]
    pub(super) network_access: bool,
}

#[derive(Debug, Serialize)]
pub(super) struct CodeExecuteResponse {
    pub(super) output: String,
    pub(super) exit_code: i64,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub(super) error: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub(super) files: Option<Vec<String>>,
}

#[derive(Debug, Serialize)]
pub(super) struct EvolutionCanarySummary {
    pub(super) enabled: bool,
    pub(super) rollout_percent: u8,
    pub(super) baseline_version: String,
    pub(super) candidate_version: String,
}

#[derive(Debug, Serialize)]
pub(super) struct EvolutionSettingsResponse {
    pub(super) self_evolve_enabled: bool,
    pub(super) learning_enabled: bool,
    pub(super) learning_model_slot: Option<String>,
    pub(super) learning_queue_cap: u64,
    pub(super) learning_queue: crate::storage::LearningQueueCounts,
    pub(super) canary: EvolutionCanarySummary,
    pub(super) strategy_canary: EvolutionCanarySummary,
    pub(super) prompt_canary: EvolutionCanarySummary,
    pub(super) specialist_prompt_canary: EvolutionCanarySummary,
    pub(super) last_promotion_result: String,
    pub(super) replay_gate_result: Option<String>,
    pub(super) promotion_mode: String,
    pub(super) prompt_last_promotion_result: String,
    pub(super) prompt_replay_gate_result: Option<String>,
    pub(super) prompt_promotion_mode: String,
    pub(super) specialist_prompt_last_promotion_result: String,
    pub(super) specialist_prompt_replay_gate_result: Option<String>,
    pub(super) specialist_prompt_promotion_mode: String,
    pub(super) routing_rollback_available: bool,
    pub(super) deploy_guard_default: bool,
    pub(super) readiness_policy: crate::core::ReadinessPolicy,
    pub(super) gepa_config: crate::core::self_evolve::gepa_bridge::GepaOptimizerConfig,
    pub(super) gepa_readiness: crate::core::self_evolve::gepa_bridge::GepaReadiness,
    pub(super) gepa_auto_state: crate::core::self_evolve::gepa_bridge::GepaAutoRunState,
    pub(super) gepa_last_result: Option<serde_json::Value>,
    pub(super) gepa_queue: serde_json::Value,
}

#[derive(Debug, Deserialize)]
pub(super) struct EvolutionSettingsUpdateRequest {
    pub(super) deploy_guard_default: Option<bool>,
    pub(super) self_evolve_enabled: Option<bool>,
    pub(super) learning_enabled: Option<bool>,
    pub(super) learning_model_slot: Option<String>,
    pub(super) learning_queue_cap: Option<u64>,
    pub(super) readiness_policy: Option<crate::core::ReadinessPolicy>,
    pub(super) gepa_auto_mode: Option<String>,
    pub(super) gepa_daily_budget_usd: Option<f64>,
    pub(super) gepa_per_run_budget_usd: Option<f64>,
    pub(super) gepa_max_runs_per_day: Option<u32>,
    pub(super) gepa_max_metric_calls: Option<u32>,
}

#[derive(Debug, Deserialize)]
pub(super) struct EvolutionDevQuery {
    pub(super) limit: Option<u64>,
    pub(super) include_superseded: Option<bool>,
}

#[derive(Debug, Serialize)]
pub(super) struct EvolutionVersionMetric {
    pub(super) version: String,
    pub(super) samples: usize,
    pub(super) success_rate: f64,
    pub(super) error_rate: f64,
    pub(super) p95_latency_ms: Option<i64>,
}

#[derive(Debug, Serialize)]
pub(super) struct PromptEvolutionMetric {
    pub(super) version: String,
    pub(super) samples: usize,
    pub(super) success_rate: f64,
    pub(super) error_rate: f64,
    pub(super) p95_latency_ms: Option<i64>,
    pub(super) routing_decisions: usize,
    pub(super) delegation_rate: f64,
    pub(super) clarification_rate: f64,
    pub(super) avg_tool_calls_per_request: f64,
    pub(super) tool_success_rate: f64,
}

#[derive(Debug, Serialize, Default)]
pub(super) struct PromptEvolutionInsights {
    pub(super) baseline_version: Option<String>,
    pub(super) candidate_version: Option<String>,
    pub(super) rollout_percent: u8,
    pub(super) delegation_avoided: f64,
    pub(super) clarification_avoided: f64,
    pub(super) successful_direct_resolution_uplift: f64,
    pub(super) tool_success_uplift: f64,
    pub(super) latency_savings_p95_ms: Option<i64>,
    pub(super) failed_delegation_reduction: f64,
    pub(super) regressions: Vec<String>,
    pub(super) summary: Vec<String>,
}

#[derive(Debug, Clone, Serialize, Default)]
pub(super) struct PromptTelemetrySectionSummary {
    pub(super) section: String,
    pub(super) samples: usize,
    pub(super) avg_chars: f64,
    pub(super) p50_chars: usize,
    pub(super) p95_chars: usize,
}

#[derive(Debug, Clone, Serialize, Default)]
pub(super) struct PromptTelemetrySummary {
    pub(super) sample_count: usize,
    pub(super) p50_final_prompt_chars: usize,
    pub(super) p95_final_prompt_chars: usize,
    pub(super) p50_tool_schema_chars: usize,
    pub(super) p95_tool_schema_chars: usize,
    pub(super) p50_estimated_total_request_chars: usize,
    pub(super) p95_estimated_total_request_chars: usize,
    pub(super) avg_tool_count: f64,
    pub(super) success_samples: usize,
    pub(super) corrected_samples: usize,
    pub(super) top_sections: Vec<PromptTelemetrySectionSummary>,
}

#[derive(Debug, Clone, Serialize, Deserialize, Default)]
pub(super) struct PromptOptimizationReviewEntry {
    #[serde(default)]
    pub(super) status: String,
    #[serde(default)]
    pub(super) reviewed_at: Option<String>,
}

pub(super) type PromptOptimizationReviewState = BTreeMap<String, PromptOptimizationReviewEntry>;

#[derive(Debug, Clone, Serialize, Default)]
pub(super) struct EvolutionChangePreview {
    pub(super) before: Vec<String>,
    pub(super) after: Vec<String>,
    pub(super) impact_estimate: Vec<String>,
}

#[derive(Debug, Clone, Serialize)]
pub(super) struct PromptOptimizationProposal {
    pub(super) id: String,
    pub(super) title: String,
    pub(super) summary: String,
    pub(super) evidence: Vec<String>,
    pub(super) expected_benefit: Vec<String>,
    pub(super) caveats: Vec<String>,
    pub(super) risk_level: String,
    pub(super) target_scope: String,
    pub(super) review_status: String,
    pub(super) reviewed_at: Option<String>,
    pub(super) reversible: bool,
    pub(super) change_preview: EvolutionChangePreview,
}

#[derive(Debug, Serialize)]
pub(super) struct EvolutionDevResponse {
    pub(super) canary_state: Option<crate::core::self_evolve::strategy_runtime::CanaryRolloutState>,
    pub(super) strategy_canary_state:
        Option<crate::core::self_evolve::strategy_runtime::CanaryRolloutState>,
    pub(super) last_result: Option<serde_json::Value>,
    pub(super) lineage_recent: Vec<serde_json::Value>,
    pub(super) policy_metrics: Vec<EvolutionVersionMetric>,
    pub(super) strategy_metrics: Vec<EvolutionVersionMetric>,
    pub(super) prompt_canary_state:
        Option<crate::core::self_evolve::strategy_runtime::CanaryRolloutState>,
    pub(super) prompt_last_result: Option<serde_json::Value>,
    pub(super) prompt_lineage_recent: Vec<serde_json::Value>,
    pub(super) prompt_metrics: Vec<PromptEvolutionMetric>,
    pub(super) prompt_insights: PromptEvolutionInsights,
    pub(super) specialist_prompt_canary_state:
        Option<crate::core::self_evolve::strategy_runtime::CanaryRolloutState>,
    pub(super) specialist_prompt_last_result: Option<serde_json::Value>,
    pub(super) specialist_prompt_lineage_recent: Vec<serde_json::Value>,
    pub(super) specialist_prompt_metrics: Vec<PromptEvolutionMetric>,
    pub(super) specialist_prompt_insights: PromptEvolutionInsights,
    pub(super) learning_queue: crate::storage::LearningQueueCounts,
    pub(super) learning_candidates: Vec<serde_json::Value>,
    pub(super) skill_evolutions: Vec<serde_json::Value>,
    pub(super) learning_items: Vec<serde_json::Value>,
    pub(super) learning_patterns: Vec<serde_json::Value>,
    pub(super) experience_graph: serde_json::Value,
    pub(super) recent_prompt_runs: Vec<serde_json::Value>,
    pub(super) recent_specialist_prompt_runs: Vec<serde_json::Value>,
    pub(super) recent_experience_runs: Vec<serde_json::Value>,
    pub(super) prompt_canary_safety_events:
        Vec<crate::core::self_evolve::strategy_runtime::PromptProfileCanarySafetyEvent>,
    pub(super) prompt_telemetry_summary: PromptTelemetrySummary,
    pub(super) prompt_optimization_opportunities: Vec<PromptOptimizationProposal>,
}

#[derive(Debug, Deserialize)]
pub(super) struct EvolutionDevActionRequest {
    pub(super) action: String,
    pub(super) candidate_id: Option<String>,
}
