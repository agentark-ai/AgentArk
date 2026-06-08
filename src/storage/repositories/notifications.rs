use super::super::*;

impl Storage {
    // ==================== Notifications ====================

    // Deduplicate repetitive notifications (same root message) to avoid spamming users/UI.
    // This is separate from retention, which deletes old rows according to data lifecycle settings.
    pub(super) const NOTIFICATION_DEDUP_COOLDOWN_DAYS: i64 = 7;
    pub(super) const ARKPULSE_NOTIFICATION_WINDOW_HOURS: i64 = 24;
    pub(super) const NOTIFICATION_PURGE_LAST_RUN_KEY: &'static str =
        "notifications_retention_last_purge_v1";

    pub(super) fn notification_is_critical(level: &str, source: &str, title: &str) -> bool {
        let lvl = level.trim().to_ascii_lowercase();
        if lvl == "error" || lvl == "critical" {
            return true;
        }
        let src = source.trim().to_ascii_lowercase();
        if src.contains("security") || src.contains("auth") {
            return true;
        }
        let t = title.trim().to_ascii_lowercase();
        t.contains("security") || t.contains("intrusion") || t.contains("breach")
    }

    /// Generate a signature for notification body comparisons.
    /// Collapses whitespace and replaces digit runs with '#', so small counter changes
    /// (e.g. "5 unread") don't produce spammy near-duplicates.
    pub(super) fn notification_body_signature(body: &str) -> String {
        let mut out = String::with_capacity(body.len().min(240));
        let mut prev_space = false;
        let mut prev_digit = false;
        for ch in body.chars() {
            if ch.is_ascii_digit() {
                if !prev_digit {
                    out.push('#');
                }
                prev_digit = true;
                prev_space = false;
                continue;
            }
            prev_digit = false;
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

    pub(super) fn is_arkpulse_notification(source: &str) -> bool {
        source.trim().eq_ignore_ascii_case("arkpulse")
    }

    pub(super) fn arkpulse_recent_cutoff_rfc3339() -> String {
        (chrono::Utc::now() - chrono::Duration::hours(Self::ARKPULSE_NOTIFICATION_WINDOW_HOURS))
            .to_rfc3339()
    }

    pub(super) async fn maybe_purge_old_notifications(&self) -> Result<()> {
        let now = chrono::Utc::now().timestamp();
        let lifecycle =
            crate::core::runtime::data_lifecycle::load_data_lifecycle_settings(self).await;
        if !lifecycle.cleanup_enabled || !lifecycle.notifications_cleanup_enabled {
            return Ok(());
        }
        let last_run = self
            .get(Self::NOTIFICATION_PURGE_LAST_RUN_KEY)
            .await?
            .and_then(|bytes| String::from_utf8(bytes).ok())
            .and_then(|s| s.parse::<i64>().ok())
            .unwrap_or(0);

        if last_run > 0 && (now - last_run) < lifecycle.notification_cleanup_interval_secs as i64 {
            return Ok(());
        }

        let _ = self
            .set(
                Self::NOTIFICATION_PURGE_LAST_RUN_KEY,
                now.to_string().as_bytes(),
            )
            .await;

        if lifecycle.notifications_retention_days == 0 {
            return Ok(());
        }

        let cutoff = (chrono::Utc::now()
            - chrono::Duration::days(lifecycle.notifications_retention_days as i64))
        .to_rfc3339();

        let result = notification::Entity::delete_many()
            .filter(notification::Column::CreatedAt.lt(cutoff))
            .exec(&self.db)
            .await?;

        if result.rows_affected > 0 {
            tracing::info!(
                "Purged {} notifications older than {} days",
                result.rows_affected,
                lifecycle.notifications_retention_days
            );
        }

        Ok(())
    }

    /// Insert a notification
    pub async fn insert_notification(&self, notif: &notification::Model) -> Result<()> {
        if let Err(e) = self.maybe_purge_old_notifications().await {
            tracing::warn!("Notification retention purge failed: {}", e);
        }

        // Normalize fields to improve dedup reliability (avoid whitespace/case variants).
        let title_clean = notif.title.trim().to_string();
        let body_clean = notif.body.trim().to_string();
        let encrypted_title = encrypt_storage_string(&title_clean)?;
        let encrypted_body = encrypt_storage_string(&body_clean)?;
        let level_clean = notif.level.trim().to_string();
        let source_clean = notif.source.trim().to_string();
        let body_sig = Self::notification_body_signature(&body_clean);

        if Self::is_arkpulse_notification(&source_clean) {
            let recent = notification::Entity::find()
                .filter(notification::Column::Source.eq(source_clean.clone()))
                .filter(notification::Column::CreatedAt.gte(Self::arkpulse_recent_cutoff_rfc3339()))
                .order_by_desc(notification::Column::CreatedAt)
                .limit(25)
                .all(&self.db)
                .await?;
            for existing in recent {
                let existing_title = decrypt_storage_string(&existing.title);
                let existing_body = decrypt_storage_string(&existing.body);
                if existing_title == title_clean
                    && Self::notification_body_signature(&existing_body) == body_sig
                {
                    return Ok(());
                }
            }
        }

        // Best-effort deduplication to prevent repeated notifications from flooding the DB/UI.
        // Critical/security notifications bypass dedup.
        if !Self::notification_is_critical(&level_clean, &source_clean, &title_clean) {
            let cutoff = (chrono::Utc::now()
                - chrono::Duration::days(Self::NOTIFICATION_DEDUP_COOLDOWN_DAYS))
            .to_rfc3339();
            match notification::Entity::find()
                .filter(notification::Column::CreatedAt.gte(cutoff))
                .filter(notification::Column::Source.eq(source_clean.clone()))
                .order_by_desc(notification::Column::CreatedAt)
                .limit(50)
                .all(&self.db)
                .await
            {
                Ok(recent) => {
                    for existing in recent {
                        let existing_title = decrypt_storage_string(&existing.title);
                        let existing_body = decrypt_storage_string(&existing.body);
                        if existing_title == title_clean
                            && Self::notification_body_signature(&existing_body) == body_sig
                        {
                            // Suppress duplicates within the cooldown window.
                            return Ok(());
                        }
                    }
                }
                Err(e) => {
                    tracing::warn!("Notification dedup lookup failed: {}", e);
                }
            }
        }

        notification::ActiveModel {
            id: Set(notif.id.clone()),
            title: Set(encrypted_title),
            body: Set(encrypted_body),
            level: Set(level_clean),
            source: Set(source_clean),
            read: Set(notif.read),
            created_at: Set(notif.created_at.clone()),
        }
        .insert(&self.db)
        .await?;
        Ok(())
    }

    /// List notifications (newest first, paginated)
    pub async fn list_notifications(
        &self,
        limit: u64,
        offset: u64,
        unread_only: bool,
    ) -> Result<Vec<notification::Model>> {
        if let Err(e) = self.maybe_purge_old_notifications().await {
            tracing::warn!("Notification retention purge failed: {}", e);
        }
        let mut query = notification::Entity::find()
            .filter(notification::Column::Source.ne("arkpulse"))
            .order_by_desc(notification::Column::CreatedAt);
        if unread_only {
            query = query.filter(notification::Column::Read.eq(false));
        }
        let mut notifs = query
            .limit(Self::db_limit(limit))
            .offset(Self::db_offset(offset))
            .all(&self.db)
            .await?;
        for notif in &mut notifs {
            notif.title = decrypt_storage_string(&notif.title);
            notif.body = decrypt_storage_string(&notif.body);
        }
        Ok(notifs)
    }

    /// Count notifications
    pub async fn count_notifications(&self, unread_only: bool) -> Result<u64> {
        if let Err(e) = self.maybe_purge_old_notifications().await {
            tracing::warn!("Notification retention purge failed: {}", e);
        }
        let mut query =
            notification::Entity::find().filter(notification::Column::Source.ne("arkpulse"));
        if unread_only {
            query = query.filter(notification::Column::Read.eq(false));
        }
        query.count(&self.db).await.map_err(Into::into)
    }

    /// Mark notification as read
    pub async fn mark_notification_read(&self, id: &str) -> Result<()> {
        notification::ActiveModel {
            id: Set(id.to_string()),
            read: Set(true),
            ..Default::default()
        }
        .update(&self.db)
        .await?;
        Ok(())
    }

    /// Set notification read flag explicitly
    pub async fn set_notification_read(&self, id: &str, read: bool) -> Result<()> {
        notification::ActiveModel {
            id: Set(id.to_string()),
            read: Set(read),
            ..Default::default()
        }
        .update(&self.db)
        .await?;
        Ok(())
    }

    /// Mark all notifications as read
    pub async fn mark_all_notifications_read(&self) -> Result<()> {
        notification::Entity::update_many()
            .col_expr(notification::Column::Read, Expr::value(true))
            .exec(&self.db)
            .await?;
        Ok(())
    }

    /// Delete goal-related notifications that reference a specific goal text.
    pub async fn delete_goal_notifications(&self, goal_text: &str) -> Result<u64> {
        let trimmed = goal_text.trim();
        if trimmed.is_empty() {
            return Ok(0);
        }

        let source_filter = Condition::any()
            .add(notification::Column::Source.contains("goal"))
            .add(notification::Column::Source.eq("autonomy_goal_loop"));
        let candidates = notification::Entity::find()
            .filter(source_filter)
            .all(&self.db)
            .await?;
        let matching_ids = candidates
            .into_iter()
            .filter(|notif| {
                decrypt_storage_string(&notif.title).contains(trimmed)
                    || decrypt_storage_string(&notif.body).contains(trimmed)
            })
            .map(|notif| notif.id)
            .collect::<Vec<_>>();
        if matching_ids.is_empty() {
            return Ok(0);
        }
        let result = notification::Entity::delete_many()
            .filter(notification::Column::Id.is_in(matching_ids))
            .exec(&self.db)
            .await?;
        Ok(result.rows_affected)
    }

    /// Delete app-related notifications that reference a specific app id/title.
    pub async fn delete_app_notifications(
        &self,
        app_id: &str,
        app_title: Option<&str>,
    ) -> Result<u64> {
        let id_trimmed = app_id.trim();
        if id_trimmed.is_empty() {
            return Ok(0);
        }

        let source_filter = Condition::any()
            .add(notification::Column::Source.contains("app"))
            .add(notification::Column::Title.contains("App"))
            .add(notification::Column::Title.contains("app"));
        let titles = app_title
            .map(str::trim)
            .filter(|s| !s.is_empty())
            .map(|title| vec![id_trimmed.to_string(), title.to_string()])
            .unwrap_or_else(|| vec![id_trimmed.to_string()]);
        let candidates = notification::Entity::find()
            .filter(source_filter)
            .all(&self.db)
            .await?;
        let matching_ids = candidates
            .into_iter()
            .filter(|notif| {
                let title = decrypt_storage_string(&notif.title);
                let body = decrypt_storage_string(&notif.body);
                titles
                    .iter()
                    .any(|needle| title.contains(needle) || body.contains(needle))
            })
            .map(|notif| notif.id)
            .collect::<Vec<_>>();
        if matching_ids.is_empty() {
            return Ok(0);
        }
        let result = notification::Entity::delete_many()
            .filter(notification::Column::Id.is_in(matching_ids))
            .exec(&self.db)
            .await?;
        Ok(result.rows_affected)
    }

    /// Count unread notifications
    pub async fn count_unread_notifications(&self) -> Result<u64> {
        if let Err(e) = self.maybe_purge_old_notifications().await {
            tracing::warn!("Notification retention purge failed: {}", e);
        }
        notification::Entity::find()
            .filter(notification::Column::Source.ne("arkpulse"))
            .filter(notification::Column::Read.eq(false))
            .count(&self.db)
            .await
            .map_err(Into::into)
    }
}
