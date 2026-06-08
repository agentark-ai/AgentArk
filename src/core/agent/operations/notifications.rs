use super::*;

const PUSH_NOTIFICATIONS_MUTE_UNTIL_KEY: &str = "push_notifications_mute_until_v1";
const PUSH_NOTIFICATIONS_LAST_SIGNATURE_KEY: &str = "push_notifications_last_signature_v1";
const PUSH_NOTIFICATIONS_LAST_SENT_AT_KEY: &str = "push_notifications_last_sent_at_v1";
const PUSH_NOTIFICATION_DUPLICATE_COOLDOWN_SECS: i64 = 30 * 60;

pub(super) fn notification_push_signature(message: &str) -> String {
    let mut out = String::with_capacity(message.len().min(240));
    let mut prev_space = false;
    for ch in message.chars() {
        if ch.is_whitespace() {
            if !prev_space {
                out.push(' ');
                prev_space = true;
            }
            continue;
        }
        prev_space = false;
        out.push(ch.to_ascii_lowercase());
        if out.len() >= 220 {
            break;
        }
    }
    out.trim().to_string()
}

const PUSH_NOTIFICATION_CHANNELS: &[&str] = &[
    "telegram",
    "whatsapp",
    "slack",
    "discord",
    "matrix",
    "teams",
    "google_chat",
    "signal",
    "imessage",
    "line",
    "wechat",
    "qq",
];

pub(super) fn is_push_notification_channel(channel: &str) -> bool {
    PUSH_NOTIFICATION_CHANNELS
        .iter()
        .any(|candidate| candidate.eq_ignore_ascii_case(channel.trim()))
}

pub(super) fn is_external_notification_channel(channel: &str) -> bool {
    let trimmed = channel.trim().to_ascii_lowercase();
    if trimmed.is_empty()
        || notification_channel_uses_preferred_fallback(&trimmed)
        || matches!(
            trimmed.as_str(),
            "web" | "app" | "app_notification" | "app_notifications" | "in_app"
        )
    {
        return false;
    }
    trimmed
        .bytes()
        .all(|byte| byte.is_ascii_alphanumeric() || matches!(byte, b'.' | b'_' | b'-' | b':'))
        && trimmed.bytes().any(|byte| byte.is_ascii_alphanumeric())
}

pub(super) fn notification_channel_uses_preferred_fallback(channel: &str) -> bool {
    matches!(
        channel.trim().to_ascii_lowercase().as_str(),
        "preferred" | "push" | "auto" | "default"
    )
}

pub(super) fn notification_channel_display_name(channel: &str) -> &str {
    match channel.trim().to_ascii_lowercase().as_str() {
        "web" => "local Web UI",
        "email" => "Email",
        "google_chat" => "Google Chat",
        "signal" => "Signal",
        "imessage" => "iMessage",
        "line" => "LINE",
        "wechat" => "WeChat",
        "qq" => "QQ",
        "telegram" => "Telegram",
        "whatsapp" => "WhatsApp",
        "slack" => "Slack",
        "discord" => "Discord",
        "matrix" => "Matrix",
        "teams" => "Teams",
        _ => channel,
    }
}

pub(super) fn notification_channel_not_connected_outcome(
    channel: &str,
) -> NotificationDispatchOutcome {
    let channel = channel.trim().to_ascii_lowercase();
    NotificationDispatchOutcome::pre_send_failure(
        channel.clone(),
        crate::channels::ChannelError::not_connected(
            channel.clone(),
            format!(
                "{} delivery is not connected",
                notification_channel_display_name(&channel)
            ),
        )
        .to_string(),
    )
}

pub(super) fn inbound_security_source_label(channel: &str) -> String {
    notification_channel_display_name(channel).to_string()
}

pub(super) fn telegram_notification_target_is_configured(
    config: &crate::core::runtime::config::TelegramConfig,
) -> bool {
    !config.bot_token.trim().is_empty()
        && config.allowed_users.len() == 1
        && config.allowed_users.first().copied().unwrap_or_default() != 0
}

pub(super) fn whatsapp_notification_target_is_configured(
    config: &crate::channels::whatsapp::WhatsAppChannelConfig,
) -> bool {
    let has_target = crate::channels::whatsapp::configured_notification_recipient(config).is_some();
    match config.mode {
        crate::channels::whatsapp::WhatsAppMode::CloudApi => {
            !config.access_token.trim().is_empty()
                && !config.phone_number_id.trim().is_empty()
                && has_target
        }
        crate::channels::whatsapp::WhatsAppMode::Baileys => {
            !config.bridge_url.trim().is_empty() && has_target
        }
    }
}

#[derive(Clone)]
pub struct NotificationStore {
    storage: Storage,
    notification_events: broadcast::Sender<NotificationEvent>,
    notifications_enabled: bool,
}

impl NotificationStore {
    #[allow(dead_code)]
    pub fn broadcast_event(&self, event: NotificationEvent) {
        let _ = self.notification_events.send(event);
    }

    async fn notifications_unlocked(&self) -> bool {
        if !self.notifications_enabled {
            return false;
        }

        match self.storage.has_user_chat_messages().await {
            Ok(true) => true,
            Ok(false) => self
                .storage
                .get("arkpulse_last_run_at")
                .await
                .ok()
                .flatten()
                .is_some(),
            Err(error) => {
                tracing::debug!(
                    "notifications_unlocked: failed to check chat history; suppressing notifications: {}",
                    error
                );
                false
            }
        }
    }

    pub async fn emit_notification_with_status(
        &self,
        title: &str,
        body: &str,
        level: &str,
        source: &str,
    ) -> NotificationDispatchOutcome {
        if !self.notifications_unlocked().await {
            tracing::debug!(
                "Notification suppressed (bootstrap gate): title='{}', source='{}'",
                title,
                source
            );
            return NotificationDispatchOutcome::pre_send_failure(
                "web",
                "Notification suppressed by bootstrap gate",
            );
        }
        let notif = crate::storage::entities::notification::Model {
            id: uuid::Uuid::new_v4().to_string(),
            title: title.to_string(),
            body: body.to_string(),
            level: level.to_string(),
            source: source.to_string(),
            read: false,
            created_at: chrono::Utc::now().to_rfc3339(),
        };
        match self.storage.insert_notification(&notif).await {
            Ok(_) => {
                let _ = self
                    .notification_events
                    .send(NotificationEvent::from_model(&notif));
                NotificationDispatchOutcome::full_success("web")
            }
            Err(error) => {
                tracing::warn!("Failed to emit notification: {}", error);
                NotificationDispatchOutcome::pre_send_failure(
                    "web",
                    format!("Failed to store notification: {}", error),
                )
            }
        }
    }

    pub async fn emit_notification(&self, title: &str, body: &str, level: &str, source: &str) {
        let _ = self
            .emit_notification_with_status(title, body, level, source)
            .await;
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum NotificationDeliveryDetail {
    FullSuccess,
    PreSendFailure,
    PartialFailure,
}

impl NotificationDeliveryDetail {
    pub fn as_str(&self) -> &'static str {
        match self {
            Self::FullSuccess => "full_success",
            Self::PreSendFailure => "pre_send_failure",
            Self::PartialFailure => "partial_failure",
        }
    }
}

#[derive(Debug, Clone)]
pub struct NotificationDispatchOutcome {
    pub channel: String,
    pub success: bool,
    pub error: Option<String>,
    pub delivery: NotificationDeliveryDetail,
}

impl NotificationDispatchOutcome {
    pub(crate) fn full_success(channel: impl Into<String>) -> Self {
        Self {
            channel: channel.into(),
            success: true,
            error: None,
            delivery: NotificationDeliveryDetail::FullSuccess,
        }
    }

    pub(crate) fn pre_send_failure(channel: impl Into<String>, error: impl Into<String>) -> Self {
        Self {
            channel: channel.into(),
            success: false,
            error: Some(error.into()),
            delivery: NotificationDeliveryDetail::PreSendFailure,
        }
    }

    pub(crate) fn partial_failure(channel: impl Into<String>, error: impl Into<String>) -> Self {
        Self {
            channel: channel.into(),
            success: false,
            error: Some(error.into()),
            delivery: NotificationDeliveryDetail::PartialFailure,
        }
    }

    pub fn is_partial_failure(&self) -> bool {
        matches!(self.delivery, NotificationDeliveryDetail::PartialFailure)
    }

    pub fn sent_any_external_chunk(&self) -> bool {
        self.success || self.is_partial_failure()
    }
}

#[derive(Debug, Clone, serde::Serialize)]
pub struct NotificationEvent {
    pub kind: String,
    pub id: String,
    pub title: String,
    pub body: String,
    pub level: String,
    pub source: String,
    pub read: bool,
    pub created_at: String,
}

impl NotificationEvent {
    fn from_model(notif: &crate::storage::entities::notification::Model) -> Self {
        Self {
            kind: "notification.created".to_string(),
            id: notif.id.clone(),
            title: notif.title.clone(),
            body: notif.body.clone(),
            level: notif.level.clone(),
            source: notif.source.clone(),
            read: notif.read,
            created_at: notif.created_at.clone(),
        }
    }
}

impl Agent {
    pub(super) async fn configured_push_channels(&self) -> Vec<String> {
        PUSH_NOTIFICATION_CHANNELS
            .iter()
            .filter(|channel| self.push_channel_is_configured(channel))
            .map(|channel| (*channel).to_string())
            .collect()
    }

    pub(super) fn calendar_integration_is_configured(&self) -> bool {
        if !self.integrations.is_enabled("google_calendar")
            && !self.integrations.is_enabled("google_workspace")
        {
            return false;
        }

        crate::core::runtime::config::SecureConfigManager::new(&self.config_dir)
            .ok()
            .and_then(|manager| manager.get_custom_secret("calendar_tokens").ok().flatten())
            .map(|value| !value.trim().is_empty())
            .unwrap_or(false)
            || crate::actions::google_workspace::granted_bundles(&self.config_dir)
                .map(|bundles| bundles.iter().any(|bundle| bundle == "calendar"))
                .unwrap_or(false)
    }

    pub(super) fn email_notification_is_configured(&self) -> bool {
        let available_backends = self.configured_email_backends();
        crate::core::connectivity::email_delivery::email_channel_is_ready(
            &self.config.email.provider,
            &available_backends,
        )
    }

    pub(super) fn legacy_gmail_notification_is_configured(&self) -> bool {
        if !self.integrations.is_enabled("gmail") {
            return false;
        }
        crate::core::runtime::config::SecureConfigManager::new(&self.config_dir)
            .ok()
            .and_then(|manager| manager.get_custom_secret("gmail_tokens").ok().flatten())
            .is_some_and(|value| !value.trim().is_empty())
    }

    pub(super) fn workspace_gmail_notification_is_configured(&self) -> bool {
        if !self.integrations.is_enabled("google_workspace") {
            return false;
        }
        crate::actions::google_workspace::granted_bundles(&self.config_dir)
            .map(|bundles| bundles.iter().any(|bundle| bundle == "gmail"))
            .unwrap_or(false)
    }

    pub(super) fn configured_email_backends(&self) -> Vec<String> {
        let mut backends = Vec::new();
        if self.legacy_gmail_notification_is_configured() {
            backends
                .push(crate::core::connectivity::email_delivery::EMAIL_PROVIDER_GMAIL.to_string());
        }
        if self.workspace_gmail_notification_is_configured() {
            backends.push(
                crate::core::connectivity::email_delivery::EMAIL_PROVIDER_GOOGLE_WORKSPACE
                    .to_string(),
            );
        }
        if crate::core::connectivity::email_delivery::external_email_delivery_is_ready(
            &self.config.email,
        ) {
            if let Some(provider_id) =
                crate::core::connectivity::email_delivery::external_email_provider_id(
                    &self.config.email,
                )
            {
                if !backends.iter().any(|existing| existing == &provider_id) {
                    backends.push(provider_id);
                }
            }
        }
        backends
    }

    pub(super) async fn configured_email_mailboxes(&self) -> Vec<(String, String)> {
        let mut mailboxes = Vec::new();
        let mut seen = std::collections::HashSet::new();
        for backend in self.configured_email_backends() {
            if !matches!(
                backend.as_str(),
                crate::core::connectivity::email_delivery::EMAIL_PROVIDER_GMAIL
                    | crate::core::connectivity::email_delivery::EMAIL_PROVIDER_GOOGLE_WORKSPACE
            ) {
                continue;
            }
            match self.email_sender_address_for_backend(&backend).await {
                Ok(address) => {
                    let dedupe_key = address.to_ascii_lowercase();
                    if seen.insert(dedupe_key) {
                        mailboxes.push((backend, address));
                    }
                }
                Err(error) => {
                    tracing::debug!(
                        "configured_email_mailboxes: failed to load mailbox for {}: {}",
                        backend,
                        error
                    );
                }
            }
        }
        mailboxes
    }

    pub(super) async fn email_sender_address_for_backend(
        &self,
        backend: &str,
    ) -> std::result::Result<String, String> {
        match backend {
            crate::core::connectivity::email_delivery::EMAIL_PROVIDER_GMAIL => {
                crate::actions::gmail::gmail_profile_email_for_source(
                    &self.config_dir,
                    crate::actions::gmail::GmailDeliverySource::Gmail,
                )
                .await
                .map_err(|error| format!("Failed to read Gmail sender address: {}", error))
            }
            crate::core::connectivity::email_delivery::EMAIL_PROVIDER_GOOGLE_WORKSPACE => {
                crate::actions::gmail::gmail_profile_email_for_source(
                    &self.config_dir,
                    crate::actions::gmail::GmailDeliverySource::GoogleWorkspace,
                )
                .await
                .map_err(|error| {
                    format!(
                        "Failed to read Google Workspace Gmail sender address: {}",
                        error
                    )
                })
            }
            _ => crate::core::connectivity::email_delivery::validate_optional_email_address(
                self.config.email.from_address.as_deref(),
            )
            .map_err(|error| error.to_string())?
            .ok_or_else(|| {
                "email.from_address is required for external email delivery".to_string()
            }),
        }
    }

    pub(super) async fn resolve_email_notification_recipient(
        &self,
        backend: &str,
    ) -> std::result::Result<String, String> {
        if let Some(address) = self.config.email.to_address.as_deref() {
            return crate::core::connectivity::email_delivery::validate_email_address(address)
                .map_err(|error| error.to_string());
        }
        if matches!(
            backend,
            crate::core::connectivity::email_delivery::EMAIL_PROVIDER_GMAIL
                | crate::core::connectivity::email_delivery::EMAIL_PROVIDER_GOOGLE_WORKSPACE
        ) {
            return self.email_sender_address_for_backend(backend).await;
        }
        let mailboxes = self.configured_email_mailboxes().await;
        match mailboxes.as_slice() {
            [(_, address)] => Ok(address.clone()),
            [] => Err(
                "email.to_address is required for external email delivery when no Gmail or Google Workspace mailbox is connected"
                    .to_string(),
            ),
            _ => Err(
                "email.to_address is required for external email delivery when multiple Gmail or Google Workspace mailboxes are connected"
                    .to_string(),
            ),
        }
    }

    pub(super) async fn send_email_notification_reported(
        &self,
        safe_message: &str,
    ) -> std::result::Result<(), String> {
        let available_backends = self.configured_email_backends();
        let backend = crate::core::connectivity::email_delivery::normalize_email_backend_selection(
            &self.config.email.provider,
            &available_backends,
        )
        .map_err(|error| error.to_string())?;
        let recipient = self.resolve_email_notification_recipient(&backend).await?;
        let (timezone, email_format) = {
            let profile = self.user_profile.read().await;
            (
                profile
                    .timezone
                    .as_deref()
                    .and_then(|value| value.parse::<chrono_tz::Tz>().ok()),
                profile.email_format.clone(),
            )
        };
        let now = chrono::Utc::now();
        let (subject_date, generated_at) = match timezone {
            Some(tz) => (
                now.with_timezone(&tz).format("%Y-%m-%d").to_string(),
                now.with_timezone(&tz)
                    .format("%Y-%m-%d %H:%M %Z")
                    .to_string(),
            ),
            None => (
                now.format("%Y-%m-%d").to_string(),
                now.format("%Y-%m-%d %H:%M UTC").to_string(),
            ),
        };
        let subject = format!("{} - {}", self.config.name, subject_date);
        let rendered = crate::core::connectivity::email_delivery::render_notification_email(
            &self.config.name,
            &subject,
            safe_message,
            Some(&generated_at),
            email_format.as_deref(),
        );
        match backend.as_str() {
            crate::core::connectivity::email_delivery::EMAIL_PROVIDER_GMAIL => {
                let args = serde_json::json!({
                    "to": recipient,
                    "subject": rendered.subject,
                    "body": rendered.text_body,
                    "html_body": rendered.html_body,
                    "delivery_source": "gmail",
                });
                self.runtime
                    .execute_action("gmail_reply", &args)
                    .await
                    .map(|_| ())
                    .map_err(|error| error.to_string())
            }
            crate::core::connectivity::email_delivery::EMAIL_PROVIDER_GOOGLE_WORKSPACE => {
                let args = serde_json::json!({
                    "to": recipient,
                    "subject": rendered.subject,
                    "body": rendered.text_body,
                    "html_body": rendered.html_body,
                    "delivery_source": "google_workspace",
                });
                self.runtime
                    .execute_action("gmail_reply", &args)
                    .await
                    .map(|_| ())
                    .map_err(|error| error.to_string())
            }
            _ => crate::core::connectivity::email_delivery::send_external_email(
                &self.config.email,
                &rendered,
                &recipient,
            )
            .await
            .map_err(|error| error.to_string()),
        }
    }

    pub(crate) fn notification_channel_is_configured(&self, channel: &str) -> bool {
        match channel.trim().to_ascii_lowercase().as_str() {
            "email" => self.email_notification_is_configured(),
            other => self.push_channel_is_configured(other),
        }
    }

    pub(super) async fn configured_notification_channels(&self) -> Vec<String> {
        let manager = crate::core::runtime::config::SecureConfigManager::new_with_data_dir(
            &self.config_dir,
            Some(&self.data_dir),
        )
        .ok();
        let packs_guard = self.extension_packs.read().await;
        struct AgentBundledCheck<'a>(&'a Agent);
        impl<'a> crate::channels::messaging_registry::BundledConfiguredCheck for AgentBundledCheck<'a> {
            fn is_configured(&self, channel_id: &str) -> bool {
                self.0.notification_channel_is_configured(channel_id)
            }
        }
        let bundled_check = AgentBundledCheck(self);
        let ctx = crate::channels::messaging_registry::ChannelQueryContext {
            bundled_configured: &bundled_check,
            extension_packs: &packs_guard,
            storage: &self.storage,
            config_dir: &self.config_dir,
            data_dir: &self.data_dir,
            config_manager: manager.as_ref(),
        };
        match crate::channels::messaging_registry::MessagingChannelRegistry::new()
            .list_configured(&ctx)
            .await
        {
            Ok(channels) => channels.into_iter().map(|channel| channel.id).collect(),
            Err(error) => {
                tracing::debug!(
                    "configured_notification_channels: registry failed: {}",
                    error
                );
                crate::channels::messaging_registry::BUNDLED_CHANNEL_IDS
                    .iter()
                    .filter(|channel| self.notification_channel_is_configured(channel))
                    .map(|channel| (*channel).to_string())
                    .collect()
            }
        }
    }

    pub(super) async fn notification_channel_is_configured_any(&self, channel: &str) -> bool {
        let channel = channel.trim().to_ascii_lowercase();
        if channel.is_empty() || !is_external_notification_channel(&channel) {
            return false;
        }
        if crate::channels::messaging_registry::BUNDLED_CHANNEL_IDS
            .iter()
            .any(|candidate| candidate.eq_ignore_ascii_case(&channel))
        {
            return self.notification_channel_is_configured(&channel);
        }
        let manager = crate::core::runtime::config::SecureConfigManager::new_with_data_dir(
            &self.config_dir,
            Some(&self.data_dir),
        )
        .ok();
        let packs_guard = self.extension_packs.read().await;
        let bundled_check: fn(&str) -> bool = |_| false;
        let ctx = crate::channels::messaging_registry::ChannelQueryContext {
            bundled_configured: &bundled_check,
            extension_packs: &packs_guard,
            storage: &self.storage,
            config_dir: &self.config_dir,
            data_dir: &self.data_dir,
            config_manager: manager.as_ref(),
        };
        crate::channels::messaging_registry::MessagingChannelRegistry::new()
            .lookup(&ctx, &channel)
            .await
            .ok()
            .flatten()
            .is_some_and(|descriptor| descriptor.configured)
            || self.integrations.get(&channel).is_some_and(|integration| {
                integration
                    .capabilities()
                    .contains(&crate::integrations::Capability::Notify)
            }) && self.integrations.is_ready(&channel).await
    }

    pub fn notification_store(&self) -> NotificationStore {
        NotificationStore {
            storage: self.storage.clone(),
            notification_events: self.notification_events.clone(),
            notifications_enabled: !self.model_pool.is_empty(),
        }
    }

    pub(super) async fn notifications_unlocked(&self) -> bool {
        if self.model_pool.is_empty() {
            return false;
        }

        match self.storage.has_user_chat_messages().await {
            Ok(true) => true,
            Ok(false) => self
                .storage
                .get("arkpulse_last_run_at")
                .await
                .ok()
                .flatten()
                .is_some(),
            Err(e) => {
                tracing::debug!(
                    "notifications_unlocked: failed to check chat history; suppressing notifications: {}",
                    e
                );
                false
            }
        }
    }

    pub async fn pause_push_notifications_for_hours(&self, hours: i64) -> Result<i64> {
        let clamped_hours = hours.clamp(1, 24 * 30);
        let until_ts = chrono::Utc::now().timestamp() + (clamped_hours * 3600);
        self.storage
            .set(
                PUSH_NOTIFICATIONS_MUTE_UNTIL_KEY,
                until_ts.to_string().as_bytes(),
            )
            .await?;
        Ok(until_ts)
    }

    pub async fn resume_push_notifications(&self) -> Result<()> {
        self.storage
            .delete(PUSH_NOTIFICATIONS_MUTE_UNTIL_KEY)
            .await?;
        Ok(())
    }

    pub async fn push_notifications_muted_until_ts(&self) -> Option<i64> {
        let now_ts = chrono::Utc::now().timestamp();
        let muted_until = self
            .storage
            .get(PUSH_NOTIFICATIONS_MUTE_UNTIL_KEY)
            .await
            .ok()
            .flatten()
            .and_then(|b| String::from_utf8(b).ok())
            .and_then(|s| s.trim().parse::<i64>().ok())
            .unwrap_or(0);

        if muted_until > now_ts {
            return Some(muted_until);
        }

        if muted_until > 0 {
            let _ = self.storage.delete(PUSH_NOTIFICATIONS_MUTE_UNTIL_KEY).await;
        }
        None
    }

    pub(super) async fn push_notifications_muted(&self) -> bool {
        self.push_notifications_muted_until_ts().await.is_some()
    }

    pub(super) async fn push_notification_in_cooldown(&self, message: &str) -> bool {
        let now_ts = chrono::Utc::now().timestamp();
        let current_sig = notification_push_signature(message);
        if current_sig.is_empty() {
            return false;
        }

        let last_sig = self
            .storage
            .get(PUSH_NOTIFICATIONS_LAST_SIGNATURE_KEY)
            .await
            .ok()
            .flatten()
            .and_then(|b| String::from_utf8(b).ok())
            .unwrap_or_default();

        let last_sent_at = self
            .storage
            .get(PUSH_NOTIFICATIONS_LAST_SENT_AT_KEY)
            .await
            .ok()
            .flatten()
            .and_then(|b| String::from_utf8(b).ok())
            .and_then(|s| s.trim().parse::<i64>().ok())
            .unwrap_or(0);

        !last_sig.is_empty()
            && last_sig == current_sig
            && last_sent_at > 0
            && (now_ts - last_sent_at) < PUSH_NOTIFICATION_DUPLICATE_COOLDOWN_SECS
    }

    pub(super) async fn remember_push_notification_sent(&self, message: &str) {
        let signature = notification_push_signature(message);
        if signature.is_empty() {
            return;
        }
        let now = chrono::Utc::now().timestamp().to_string();
        if let Err(e) = self
            .storage
            .set(PUSH_NOTIFICATIONS_LAST_SIGNATURE_KEY, signature.as_bytes())
            .await
        {
            tracing::debug!(
                "Failed to persist push notification signature (dedupe): {}",
                e
            );
        }
        if let Err(e) = self
            .storage
            .set(PUSH_NOTIFICATIONS_LAST_SENT_AT_KEY, now.as_bytes())
            .await
        {
            tracing::debug!(
                "Failed to persist push notification timestamp (dedupe): {}",
                e
            );
        }
    }

    pub(super) async fn store_notification_with_status(
        &self,
        title: &str,
        body: &str,
        level: &str,
        source: &str,
    ) -> NotificationDispatchOutcome {
        if !self.notifications_unlocked().await {
            tracing::debug!(
                "Notification suppressed (bootstrap gate): title='{}', source='{}'",
                title,
                source
            );
            return NotificationDispatchOutcome::pre_send_failure(
                "web",
                "Notification suppressed by bootstrap gate",
            );
        }
        let notif = crate::storage::entities::notification::Model {
            id: uuid::Uuid::new_v4().to_string(),
            title: title.to_string(),
            body: body.to_string(),
            level: level.to_string(),
            source: source.to_string(),
            read: false,
            created_at: chrono::Utc::now().to_rfc3339(),
        };
        match self.storage.insert_notification(&notif).await {
            Ok(_) => {
                let _ = self
                    .notification_events
                    .send(NotificationEvent::from_model(&notif));
                NotificationDispatchOutcome::full_success("web")
            }
            Err(e) => {
                tracing::warn!("Failed to emit notification: {}", e);
                NotificationDispatchOutcome::pre_send_failure(
                    "web",
                    format!("Failed to store notification: {}", e),
                )
            }
        }
    }

    /// Emit a notification (stored in DB, visible in UI).
    pub async fn emit_notification_with_status(
        &self,
        title: &str,
        body: &str,
        level: &str,
        source: &str,
    ) -> NotificationDispatchOutcome {
        if !self.notifications_unlocked().await {
            tracing::debug!(
                "Notification suppressed (bootstrap gate): title='{}', source='{}'",
                title,
                source
            );
            return NotificationDispatchOutcome::pre_send_failure(
                "web",
                "Notification suppressed by bootstrap gate",
            );
        }
        self.store_notification_with_status(title, body, level, source)
            .await
    }

    /// Emit a notification (stored in DB, visible in UI)
    pub async fn emit_notification(&self, title: &str, body: &str, level: &str, source: &str) {
        let _ = self
            .emit_notification_with_status(title, body, level, source)
            .await;
    }

    /// Emit a notification even when chat/bootstrap gating would normally suppress it.
    pub async fn emit_notification_forced_with_status(
        &self,
        title: &str,
        body: &str,
        level: &str,
        source: &str,
    ) -> NotificationDispatchOutcome {
        self.store_notification_with_status(title, body, level, source)
            .await
    }

    /// Emit a notification even when chat/bootstrap gating would normally suppress it.
    pub async fn emit_notification_forced(
        &self,
        title: &str,
        body: &str,
        level: &str,
        source: &str,
    ) {
        let _ = self
            .emit_notification_forced_with_status(title, body, level, source)
            .await;
    }

    pub fn subscribe_notification_events(&self) -> broadcast::Receiver<NotificationEvent> {
        self.notification_events.subscribe()
    }

    pub(super) fn push_channel_is_configured(&self, channel: &str) -> bool {
        match channel {
            "telegram" => self
                .config
                .telegram
                .as_ref()
                .map(telegram_notification_target_is_configured)
                .unwrap_or(false),
            "whatsapp" => self
                .config
                .whatsapp
                .as_ref()
                .map(whatsapp_notification_target_is_configured)
                .unwrap_or(false),
            "slack" => self
                .config
                .slack
                .as_ref()
                .map(|cfg| {
                    !cfg.bot_token.trim().is_empty() && !cfg.default_channel_id.trim().is_empty()
                })
                .unwrap_or(false),
            "discord" => self
                .config
                .discord
                .as_ref()
                .map(|cfg| {
                    (!cfg.bot_token.trim().is_empty() || !cfg.webhook_url.trim().is_empty())
                        && !cfg.default_channel_id.trim().is_empty()
                })
                .unwrap_or(false),
            "matrix" => self
                .config
                .matrix
                .as_ref()
                .map(|cfg| {
                    !cfg.access_token.trim().is_empty()
                        && cfg
                            .default_room_id
                            .as_deref()
                            .map(str::trim)
                            .filter(|value| !value.is_empty())
                            .is_some()
                })
                .unwrap_or(false),
            "teams" => self
                .config
                .teams
                .as_ref()
                .map(|cfg| {
                    !cfg.access_token.trim().is_empty()
                        && (cfg
                            .channel_id
                            .as_deref()
                            .map(str::trim)
                            .filter(|value| !value.is_empty())
                            .is_some()
                            || cfg
                                .chat_id
                                .as_deref()
                                .map(str::trim)
                                .filter(|value| !value.is_empty())
                                .is_some())
                })
                .unwrap_or(false),
            "google_chat" => self
                .config
                .google_chat
                .as_ref()
                .map(|cfg| {
                    !cfg.access_token.trim().is_empty()
                        && cfg
                            .space
                            .as_deref()
                            .map(str::trim)
                            .filter(|value| !value.is_empty())
                            .is_some()
                })
                .unwrap_or(false),
            "signal" => self
                .config
                .signal
                .as_ref()
                .map(|cfg| {
                    !cfg.bridge_token.trim().is_empty()
                        && (!cfg.default_recipient.trim().is_empty()
                            || !cfg.default_group_id.trim().is_empty())
                })
                .unwrap_or(false),
            "imessage" => self
                .config
                .imessage
                .as_ref()
                .map(|cfg| {
                    !cfg.bridge_token.trim().is_empty()
                        && (!cfg.default_chat_id.trim().is_empty()
                            || !cfg.default_handle.trim().is_empty())
                })
                .unwrap_or(false),
            "line" => self
                .config
                .line
                .as_ref()
                .map(|cfg| {
                    !cfg.channel_access_token.trim().is_empty()
                        && !cfg.channel_secret.trim().is_empty()
                        && cfg
                            .default_target
                            .as_deref()
                            .map(str::trim)
                            .filter(|value| !value.is_empty())
                            .is_some()
                })
                .unwrap_or(false),
            "wechat" => self
                .config
                .wechat
                .as_ref()
                .map(|cfg| {
                    !cfg.bridge_token.trim().is_empty()
                        && !cfg.bridge_url.trim().is_empty()
                        && !cfg.default_target_id.trim().is_empty()
                })
                .unwrap_or(false),
            "qq" => self
                .config
                .qq
                .as_ref()
                .map(|cfg| {
                    !cfg.bridge_token.trim().is_empty()
                        && !cfg.bridge_url.trim().is_empty()
                        && !cfg.default_target_id.trim().is_empty()
                })
                .unwrap_or(false),
            _ => false,
        }
    }

    pub(super) fn preferred_notification_override_value(
        preferred_override: Option<&str>,
    ) -> Option<String> {
        preferred_override
            .map(str::trim)
            .filter(|value| !value.is_empty())
            .map(|value| value.to_ascii_lowercase())
            .filter(|value| !notification_channel_uses_preferred_fallback(value))
    }

    pub(super) async fn stored_preferred_notification_override(&self) -> Option<String> {
        self.storage
            .get("daily_brief_channel")
            .await
            .ok()
            .flatten()
            .and_then(|bytes| String::from_utf8(bytes).ok())
            .map(|value| value.trim().to_ascii_lowercase())
            .filter(|value| !value.is_empty())
            .filter(|value| !notification_channel_uses_preferred_fallback(value))
    }

    pub(super) async fn preferred_notification_candidates(
        &self,
        preferred_override: Option<&str>,
        push_only: bool,
    ) -> Vec<String> {
        let mut candidates = Vec::new();
        let mut seen = HashSet::new();

        let stored_preferred = self.stored_preferred_notification_override().await;
        let hinted_preferred = Self::preferred_notification_override_value(preferred_override);
        for preferred in [hinted_preferred, stored_preferred].into_iter().flatten() {
            let eligible = if push_only {
                is_push_notification_channel(&preferred)
                    && self.push_channel_is_configured(&preferred)
            } else {
                !is_external_notification_channel(&preferred)
                    || self
                        .notification_channel_is_configured_any(&preferred)
                        .await
            };
            if eligible && seen.insert(preferred.clone()) {
                candidates.push(preferred);
            }
        }

        let configured = if push_only {
            self.configured_push_channels().await
        } else {
            self.configured_notification_channels().await
        };
        for channel in configured {
            if seen.insert(channel.clone()) {
                candidates.push(channel);
            }
        }

        if !push_only {
            for integration_id in self.integrations.notifiable_integrations().await {
                if seen.insert(integration_id.clone()) {
                    candidates.push(integration_id);
                }
            }
        }

        candidates
    }

    pub(super) async fn notify_preferred_channel_with_hint(
        &self,
        message: &str,
        preferred_override: Option<&str>,
        enforce_duplicate_cooldown: bool,
    ) -> bool {
        if !self.notifications_unlocked().await {
            tracing::debug!("notify_preferred_channel suppressed (bootstrap gate)");
            return false;
        }
        if self.push_notifications_muted().await {
            tracing::debug!("notify_preferred_channel suppressed (mute active)");
            return false;
        }
        if enforce_duplicate_cooldown && self.push_notification_in_cooldown(message).await {
            tracing::debug!(
                "notify_preferred_channel suppressed (duplicate within {}s cooldown)",
                PUSH_NOTIFICATION_DUPLICATE_COOLDOWN_SECS
            );
            return false;
        }

        for channel in self
            .preferred_notification_candidates(preferred_override, false)
            .await
        {
            tracing::info!("notify_preferred_channel: trying '{}'", channel);
            if self.try_send_notification(&channel, message).await {
                if enforce_duplicate_cooldown {
                    self.remember_push_notification_sent(message).await;
                }
                return true;
            }
        }

        tracing::info!(
            "notify_preferred_channel: no external channel delivered, notification stored in DB"
        );
        false
    }

    pub(crate) async fn notify_preferred_channel_reported_with_hint(
        &self,
        message: &str,
        preferred_override: Option<&str>,
        enforce_duplicate_cooldown: bool,
    ) -> Vec<NotificationDispatchOutcome> {
        let mut attempts = Vec::new();

        if !self.notifications_unlocked().await {
            tracing::debug!("notify_preferred_channel suppressed (bootstrap gate)");
            attempts.push(NotificationDispatchOutcome::pre_send_failure(
                "push",
                "Notification suppressed by bootstrap gate",
            ));
            return attempts;
        }
        if self.push_notifications_muted().await {
            tracing::debug!("notify_preferred_channel suppressed (mute active)");
            attempts.push(NotificationDispatchOutcome::pre_send_failure(
                "push",
                "Push notifications are currently muted",
            ));
            return attempts;
        }
        if enforce_duplicate_cooldown && self.push_notification_in_cooldown(message).await {
            tracing::debug!(
                "notify_preferred_channel suppressed (duplicate within {}s cooldown)",
                PUSH_NOTIFICATION_DUPLICATE_COOLDOWN_SECS
            );
            attempts.push(NotificationDispatchOutcome::pre_send_failure(
                "push",
                format!(
                    "Duplicate notification suppressed within {} second cooldown",
                    PUSH_NOTIFICATION_DUPLICATE_COOLDOWN_SECS
                ),
            ));
            return attempts;
        }

        for channel in self
            .preferred_notification_candidates(preferred_override, false)
            .await
        {
            tracing::info!("notify_preferred_channel: trying '{}'", channel);
            let outcome = self.try_send_notification_reported(&channel, message).await;
            let stop_after_attempt = outcome.sent_any_external_chunk();
            attempts.push(outcome);
            if stop_after_attempt {
                if enforce_duplicate_cooldown {
                    self.remember_push_notification_sent(message).await;
                }
                return attempts;
            }
        }

        tracing::info!(
            "notify_preferred_channel: no external channel delivered, notification stored in DB"
        );
        if attempts.is_empty() {
            attempts.push(NotificationDispatchOutcome::full_success("web"));
        }
        attempts
    }

    /// Send a message to the user's preferred notification channel (non-blocking).
    /// Reads daily_brief_channel from settings to determine where to send.
    /// Falls back to any connected integration with Notify capability.
    pub async fn notify_preferred_channel(&self, message: &str) {
        let _ = self
            .notify_preferred_channel_with_hint(message, None, true)
            .await;
    }

    pub(super) async fn execute_direct_notify_user_tool(
        &self,
        arguments: &serde_json::Value,
    ) -> anyhow::Result<String> {
        let message = arguments
            .get("message")
            .and_then(|value| value.as_str())
            .map(str::trim)
            .filter(|value| !value.is_empty())
            .ok_or_else(|| {
                crate::actions::structured_action_error(
                    crate::actions::ActionErrorDomain::Channel,
                    crate::actions::ActionErrorReason::MissingInput,
                    "notify_user requires a non-empty `message`",
                )
            })?;
        let title = arguments
            .get("title")
            .and_then(|value| value.as_str())
            .map(str::trim)
            .filter(|value| !value.is_empty());
        let delivery_channel = arguments
            .get("delivery_channel")
            .and_then(|value| value.as_str())
            .map(str::trim)
            .filter(|value| !value.is_empty())
            .map(|value| value.to_ascii_lowercase());

        let in_app_source = direct_notification_in_app_source(arguments);
        let full_message = direct_notification_external_message(&in_app_source, title, message);

        let Some(delivery_channel) = delivery_channel else {
            return Ok(full_message);
        };

        let in_app_title =
            direct_notification_in_app_title(arguments, title, in_app_source.as_str());
        let in_app = self
            .emit_notification_with_status(&in_app_title, message, "info", &in_app_source)
            .await;

        let mut attempts = Vec::new();
        if delivery_channel == "preferred" {
            attempts.extend(
                self.notify_preferred_channel_reported_with_hint(&full_message, None, true)
                    .await,
            );
        } else if is_external_notification_channel(&delivery_channel) {
            if self
                .notification_channel_is_configured_any(&delivery_channel)
                .await
            {
                attempts.push(
                    self.try_send_notification_reported(&delivery_channel, &full_message)
                        .await,
                );
            } else {
                attempts.push(notification_channel_not_connected_outcome(
                    &delivery_channel,
                ));
            }
        } else if delivery_channel != "web" && delivery_channel != "in_app" {
            attempts.push(
                self.try_send_notification_reported(&delivery_channel, &full_message)
                    .await,
            );
        }

        let delivered_channel = attempts
            .iter()
            .find(|attempt| attempt.sent_any_external_chunk() && attempt.channel != "web")
            .map(|attempt| attempt.channel.clone());
        let detail = if let Some(channel) = delivered_channel.as_deref() {
            if attempts
                .iter()
                .any(|attempt| attempt.channel == channel && attempt.is_partial_failure())
            {
                format!(
                    "Notification partially delivered via {}; full message kept in-app.",
                    notification_channel_display_name(channel)
                )
            } else {
                format!(
                    "Notification delivered via {}.",
                    notification_channel_display_name(channel)
                )
            }
        } else if in_app.success {
            "Notification kept in-app.".to_string()
        } else {
            "Notification delivery failed.".to_string()
        };

        Ok(format!(
            "{}{}",
            crate::runtime::TOOL_COMPLETION_MARKER,
            serde_json::json!({
                "tool": "notify_user",
                "status": if delivered_channel.is_some() || in_app.success { "completed" } else { "failed" },
                "detail": detail,
                "data": {
                    "requested_channel": delivery_channel,
                    "delivered_channel": delivered_channel,
                    "in_app": {
                        "channel": in_app.channel,
                        "success": in_app.success,
                        "error": in_app.error,
                    },
                    "attempts": attempts
                        .into_iter()
                        .map(|attempt| serde_json::json!({
                            "channel": attempt.channel,
                            "success": attempt.success,
                            "error": attempt.error,
                            "delivery": attempt.delivery.as_str(),
                        }))
                        .collect::<Vec<_>>(),
                }
            })
        ))
    }

    pub(super) fn sanitize_outbound_notification_message(
        channel: &str,
        message: &str,
    ) -> std::result::Result<String, String> {
        if channel == "web" {
            return Ok(message.to_string());
        }

        let privacy = crate::security::check_outbound_text(
            message,
            &crate::security::OutboundPrivacyPolicy::default(),
        );
        match privacy.decision {
            crate::security::OutboundPrivacyDecision::Allow => Ok(message.to_string()),
            crate::security::OutboundPrivacyDecision::RedactedAllow => {
                tracing::warn!(
                    channel = channel,
                    redactions = ?privacy.redactions,
                    reasons = ?privacy.reasons,
                    "Outbound privacy gate redacted notification message"
                );
                Ok(privacy.sanitized_text)
            }
            crate::security::OutboundPrivacyDecision::Block => {
                Err(crate::security::format_outbound_privacy_block(
                    &format!("notification via '{}'", channel),
                    &privacy.reasons,
                ))
            }
        }
    }

    pub(super) async fn try_send_registry_messaging_channel(
        &self,
        channel: &str,
        safe_message: &str,
    ) -> Option<std::result::Result<(), String>> {
        let normalized = channel.trim().to_ascii_lowercase();
        if !(normalized
            .starts_with(crate::channels::messaging_registry::EXTENSION_CHANNEL_ID_PREFIX)
            || normalized.starts_with(crate::custom_messaging_channels::CUSTOM_CHANNEL_ID_PREFIX))
        {
            return None;
        }
        let manager = match crate::core::runtime::config::SecureConfigManager::new_with_data_dir(
            &self.config_dir,
            Some(&self.data_dir),
        ) {
            Ok(manager) => manager,
            Err(error) => return Some(Err(format!("Credential store unavailable: {}", error))),
        };
        let packs_guard = self.extension_packs.read().await;
        let bundled_check: fn(&str) -> bool = |_| false;
        let ctx = crate::channels::messaging_registry::ChannelQueryContext {
            bundled_configured: &bundled_check,
            extension_packs: &packs_guard,
            storage: &self.storage,
            config_dir: &self.config_dir,
            data_dir: &self.data_dir,
            config_manager: Some(&manager),
        };
        let descriptor = match crate::channels::messaging_registry::MessagingChannelRegistry::new()
            .lookup(&ctx, &normalized)
            .await
        {
            Ok(Some(descriptor)) => descriptor,
            Ok(None) => return Some(Err("Messaging channel is not installed.".to_string())),
            Err(error) => return Some(Err(format!("Messaging channel lookup failed: {}", error))),
        };
        if matches!(
            descriptor.source,
            crate::channels::messaging_registry::ChannelSource::Bundled
        ) {
            return None;
        }
        if !descriptor.configured {
            return Some(Err(format!(
                "{} is not connected yet.",
                descriptor.display_name
            )));
        }
        let Some(send_spec) = descriptor.send_spec.as_ref() else {
            return Some(Err(format!(
                "{} does not declare a send endpoint.",
                descriptor.display_name
            )));
        };
        let overlay = if let Some(profile_id) = descriptor.auth_profile_id.as_deref() {
            match crate::core::connectivity::auth_profiles::AuthProfileControlPlane::resolve_http(
                &self.storage,
                profile_id,
            )
            .await
            {
                Ok(resolved) => Some(resolved.overlay),
                Err(error) => return Some(Err(format!("Auth profile is not ready: {}", error))),
            }
        } else {
            None
        };
        let client = match reqwest::Client::builder()
            .timeout(std::time::Duration::from_secs(20))
            .build()
        {
            Ok(client) => client,
            Err(error) => return Some(Err(format!("Failed to build HTTP client: {}", error))),
        };
        let inputs = crate::channels::messaging_dispatch::DispatchInputs {
            text: safe_message,
            to: None,
            conversation_id: None,
            subject: safe_message.lines().next(),
        };
        let result = crate::channels::messaging_dispatch::dispatch_pack_channel_with_overlay(
            &client,
            &manager,
            send_spec,
            &inputs,
            overlay.as_ref(),
        )
        .await;
        Some(
            result
                .map(|_| ())
                .map_err(|error| crate::security::redact_secret_input(&error.to_string()).text),
        )
    }

    /// Attempt to send a notification via a specific channel/integration.
    /// Returns true on success, false on failure.
    pub async fn try_send_notification(&self, channel: &str, message: &str) -> bool {
        self.try_send_notification_reported(channel, message)
            .await
            .sent_any_external_chunk()
    }

    pub async fn notify_preferred_channel_reported(
        &self,
        message: &str,
    ) -> Vec<NotificationDispatchOutcome> {
        self.notify_preferred_channel_reported_with_hint(message, None, true)
            .await
    }

    pub async fn try_send_notification_reported(
        &self,
        channel: &str,
        message: &str,
    ) -> NotificationDispatchOutcome {
        let channel_name = channel.to_string();
        let safe_message = match Self::sanitize_outbound_notification_message(channel, message) {
            Ok(value) => value,
            Err(error) => {
                tracing::warn!("{}", error);
                return NotificationDispatchOutcome::pre_send_failure(channel_name, error);
            }
        };

        let chunks = notification_chunks_for_channel(channel, &safe_message);
        let total_chunks = chunks.len();
        let mut sent_chunks = 0usize;
        for (idx, chunk) in chunks.iter().enumerate() {
            match self.send_notification_chunk_once(channel, chunk).await {
                Ok(()) => {
                    sent_chunks += 1;
                }
                Err(error) if sent_chunks == 0 => {
                    let safe_error = crate::security::redact_secret_input(&error).text;
                    tracing::warn!(
                        channel = %channel_name,
                        error = %safe_error,
                        "Notification delivery failed before any external chunk was sent"
                    );
                    return NotificationDispatchOutcome::pre_send_failure(channel_name, safe_error);
                }
                Err(error) => {
                    let safe_error = crate::security::redact_secret_input(&error).text;
                    let detail = format!(
                        "Chunk {}/{} failed after {} chunk(s) were sent: {}",
                        idx + 1,
                        total_chunks,
                        sent_chunks,
                        safe_error
                    );
                    tracing::warn!(
                        channel = %channel_name,
                        sent_chunks = sent_chunks,
                        total_chunks = total_chunks,
                        error = %safe_error,
                        "Notification delivery partially failed"
                    );
                    return NotificationDispatchOutcome::partial_failure(channel_name, detail);
                }
            }
        }

        tracing::info!(
            channel = %channel_name,
            chunks = total_chunks,
            "Notification delivery succeeded"
        );
        NotificationDispatchOutcome::full_success(channel_name)
    }

    async fn send_notification_chunk_once(
        &self,
        channel: &str,
        safe_message: &str,
    ) -> std::result::Result<(), String> {
        match channel {
            #[cfg(feature = "telegram")]
            "telegram" => crate::channels::telegram::send_message(self, safe_message)
                .await
                .map_err(|e| e.to_string()),
            "slack" => crate::channels::slack::send_message(self, safe_message)
                .await
                .map_err(|e| e.to_string()),
            "discord" => crate::channels::discord::send_message(self, safe_message)
                .await
                .map_err(|e| e.to_string()),
            "matrix" => crate::channels::matrix::send_message(self, safe_message)
                .await
                .map_err(|e| e.to_string()),
            "teams" => crate::channels::teams::send_message(self, safe_message)
                .await
                .map_err(|e| e.to_string()),
            "whatsapp" => crate::channels::whatsapp::send_message(self, safe_message)
                .await
                .map_err(|e| e.to_string()),
            "google_chat" => crate::channels::google_chat::send_message(self, safe_message)
                .await
                .map_err(|e| e.to_string()),
            "signal" => crate::channels::signal::send_message(self, safe_message)
                .await
                .map_err(|e| e.to_string()),
            "imessage" => crate::channels::imessage::send_message(self, safe_message)
                .await
                .map_err(|e| e.to_string()),
            "line" => crate::channels::line::send_message(self, safe_message)
                .await
                .map_err(|e| e.to_string()),
            "wechat" => crate::channels::wechat::send_message(self, safe_message)
                .await
                .map_err(|e| e.to_string()),
            "qq" => crate::channels::qq::send_message(self, safe_message)
                .await
                .map_err(|e| e.to_string()),
            "email" => self.send_email_notification_reported(safe_message).await,
            "web" => Ok(()),
            other => {
                if let Some(result) = self
                    .try_send_registry_messaging_channel(other, safe_message)
                    .await
                {
                    result
                } else {
                    self.integrations
                        .execute(
                            other,
                            "notify",
                            &serde_json::json!({"message": safe_message}),
                        )
                        .await
                        .map(|_| ())
                        .map_err(|e| e.to_string())
                }
            }
        }
    }
}

fn notification_chunks_for_channel(channel: &str, safe_message: &str) -> Vec<String> {
    match channel.trim().to_ascii_lowercase().as_str() {
        "email" | "web" => vec![safe_message.to_string()],
        _ => crate::channels::outbound_split::split_for_push_notification(safe_message),
    }
}

pub(super) fn direct_notification_external_message(
    source: &str,
    title: Option<&str>,
    message: &str,
) -> String {
    let message = message.trim();
    if let Some(label) = direct_notification_source_label(source) {
        return direct_notification_apply_source_label(label, message);
    }
    if let Some(title) = title.map(str::trim).filter(|value| !value.is_empty()) {
        if notification_text_key(title) == notification_text_key(message) {
            message.to_string()
        } else {
            format!("{}\n\n{}", title, message)
        }
    } else {
        message.to_string()
    }
}

fn direct_notification_source_label(source: &str) -> Option<&'static str> {
    match source.trim().to_ascii_lowercase().as_str() {
        "reminder" => Some("⏰ Reminder"),
        "watcher" => Some("🔔 Watcher"),
        _ => None,
    }
}

fn direct_notification_apply_source_label(label: &str, message: &str) -> String {
    let message = message.trim();
    if message.is_empty() {
        return label.to_string();
    }
    let label_key = notification_text_key(label);
    let message_key = notification_text_key(message);
    if message_key == label_key || message_key.starts_with(&format!("{}:", label_key)) {
        message.to_string()
    } else {
        format!("{}: {}", label, message)
    }
}

fn notification_text_key(value: &str) -> String {
    value
        .split_whitespace()
        .collect::<Vec<_>>()
        .join(" ")
        .to_ascii_lowercase()
}

fn direct_notification_in_app_source(arguments: &serde_json::Value) -> String {
    [
        "source",
        "notification_source",
        "notification_type",
        "category",
        "type",
    ]
    .iter()
    .find_map(|key| {
        arguments
            .get(*key)
            .and_then(|value| value.as_str())
            .and_then(normalize_notification_source)
    })
    .unwrap_or_else(|| "agent".to_string())
}

fn direct_notification_in_app_title(
    arguments: &serde_json::Value,
    title: Option<&str>,
    source: &str,
) -> String {
    if let Some(in_app_title) = arguments
        .get("in_app_title")
        .and_then(|value| value.as_str())
        .map(str::trim)
        .filter(|value| !value.is_empty())
    {
        return in_app_title.to_string();
    }
    if let Some(title) = title {
        return title.to_string();
    }
    if source == "reminder" {
        "Reminder".to_string()
    } else {
        "AgentArk Notification".to_string()
    }
}

fn normalize_notification_source(value: &str) -> Option<String> {
    let normalized = value.trim().to_ascii_lowercase();
    if normalized.is_empty() || normalized.chars().count() > 64 {
        return None;
    }
    if normalized
        .bytes()
        .all(|byte| byte.is_ascii_alphanumeric() || matches!(byte, b'_' | b'-' | b'.' | b':'))
        && normalized.bytes().any(|byte| byte.is_ascii_alphanumeric())
    {
        Some(normalized)
    } else {
        None
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn preferred_override_ignores_fallback_aliases() {
        for alias in ["preferred", "push", "auto", "default", " Preferred "] {
            assert_eq!(
                Agent::preferred_notification_override_value(Some(alias)),
                None
            );
        }
        assert_eq!(
            Agent::preferred_notification_override_value(Some(" Telegram ")).as_deref(),
            Some("telegram")
        );
    }

    #[test]
    fn notification_chunks_split_push_but_not_email() {
        let long = "x".repeat(crate::channels::outbound_split::PUSH_NOTIFICATION_MAX_CHARS + 50);
        let push_chunks = notification_chunks_for_channel("telegram", &long);
        let email_chunks = notification_chunks_for_channel("email", &long);

        assert!(push_chunks.len() > 1);
        assert!(push_chunks[0].starts_with("[1/"));
        assert_eq!(email_chunks, vec![long]);
    }

    #[test]
    fn external_notification_channel_accepts_structural_route_ids() {
        for channel in [
            "slack",
            "whatsapp",
            "sms",
            "pagerduty",
            "custom.ops-alerts",
            "ext.team-hub",
            "vendor:incident-room",
        ] {
            assert!(is_external_notification_channel(channel), "{channel}");
        }

        for channel in ["", " ", "preferred", "push", "web", "in_app", "../secret"] {
            assert!(!is_external_notification_channel(channel), "{channel}");
        }
    }

    #[test]
    fn direct_notification_full_message_does_not_duplicate_identical_title_and_body() {
        assert_eq!(
            direct_notification_external_message(
                "agent",
                Some("Meeting with Mark"),
                "Meeting with Mark"
            ),
            "Meeting with Mark"
        );
        assert_eq!(
            direct_notification_external_message("agent", Some("Reminder"), "Meeting with Mark"),
            "Reminder\n\nMeeting with Mark"
        );
    }

    #[test]
    fn direct_notification_external_message_decorates_structured_reminders() {
        let bodies = [
            "Meeting reminder: Asif at 11:00 AM",
            "ping Asif eleven-ish",
            "call ASIF, 11?",
        ];

        for body in bodies {
            assert_eq!(
                direct_notification_external_message("reminder", None, body),
                format!("⏰ Reminder: {}", body)
            );
        }
    }

    #[test]
    fn direct_notification_external_message_does_not_infer_reminders_from_body_text() {
        assert_eq!(
            direct_notification_external_message("agent", None, "Reminder: call Asif at 11"),
            "Reminder: call Asif at 11"
        );
    }

    #[test]
    fn direct_notification_external_message_avoids_structured_label_duplication() {
        assert_eq!(
            direct_notification_external_message(
                "reminder",
                Some("Reminder"),
                "call Asif at 11:00 AM"
            ),
            "⏰ Reminder: call Asif at 11:00 AM"
        );
        assert_eq!(
            direct_notification_external_message(
                "reminder",
                None,
                "⏰ Reminder: call Asif at 11:00 AM"
            ),
            "⏰ Reminder: call Asif at 11:00 AM"
        );
    }

    #[test]
    fn direct_notification_external_message_decorates_structured_watchers() {
        assert_eq!(
            direct_notification_external_message("watcher", None, "Background signal ready"),
            "🔔 Watcher: Background signal ready"
        );
    }

    #[test]
    fn direct_notification_in_app_source_uses_structured_type() {
        assert_eq!(
            direct_notification_in_app_source(&serde_json::json!({
                "message": "Meeting with Mark",
                "source": "reminder"
            })),
            "reminder"
        );
        assert_eq!(
            direct_notification_in_app_source(&serde_json::json!({
                "message": "Build failed",
                "notification_type": "automation_failure"
            })),
            "automation_failure"
        );
        assert_eq!(
            direct_notification_in_app_source(&serde_json::json!({
                "message": "Meeting with Mark"
            })),
            "agent"
        );
    }
}
