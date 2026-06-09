use super::*;
use crate::core::automation::{background_session, task, watcher};
use chrono::Timelike;

#[derive(Debug, Clone)]
pub(super) struct DailyBriefRunResult {
    pub(super) brief: String,
    pub(super) in_app: NotificationDispatchOutcome,
    pub(super) push_attempts: Vec<NotificationDispatchOutcome>,
}

#[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
pub(super) struct DailyBriefTaskCounts {
    pending: usize,
    awaiting_approval: usize,
    paused: usize,
    in_progress: usize,
    failed: usize,
}

impl DailyBriefTaskCounts {
    fn open(self) -> usize {
        self.pending + self.awaiting_approval + self.paused + self.in_progress
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub(super) enum DailyBriefCalendarSummary {
    NotConnected,
    LoadFailed,
    Clear,
    Meetings(Vec<String>),
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub(super) enum DailyBriefMailSummary {
    NotConnected,
    LoadFailed,
    Clear,
    Messages(Vec<String>),
}

#[derive(Debug, Clone, Copy)]
pub(super) struct DailyBriefFallbackInput<'a> {
    pub(super) counts: DailyBriefTaskCounts,
    pub(super) overdue: &'a [String],
    pub(super) due_today: &'a [String],
    pub(super) failed: &'a [String],
    pub(super) awaiting_approval: &'a [String],
    pub(super) important_events: &'a [String],
    pub(super) calendar_summary: &'a DailyBriefCalendarSummary,
    pub(super) mail_summary: &'a DailyBriefMailSummary,
}

impl Agent {
    fn daily_brief_timezone(profile: &UserProfile) -> Option<chrono_tz::Tz> {
        profile
            .timezone
            .as_deref()
            .and_then(|value| value.parse::<chrono_tz::Tz>().ok())
    }

    fn daily_brief_local_date(
        at: chrono::DateTime<chrono::Utc>,
        tz: Option<chrono_tz::Tz>,
    ) -> chrono::NaiveDate {
        match tz {
            Some(tz) => at.with_timezone(&tz).date_naive(),
            None => at.date_naive(),
        }
    }

    fn daily_brief_time_label(
        at: chrono::DateTime<chrono::Utc>,
        tz: Option<chrono_tz::Tz>,
    ) -> String {
        match tz {
            Some(tz) => at.with_timezone(&tz).format("%I:%M %p %Z").to_string(),
            None => at.format("%I:%M %p UTC").to_string(),
        }
    }

    fn daily_brief_datetime_label(
        at: chrono::DateTime<chrono::Utc>,
        tz: Option<chrono_tz::Tz>,
    ) -> String {
        match tz {
            Some(tz) => at
                .with_timezone(&tz)
                .format("%a, %b %d %I:%M %p %Z")
                .to_string(),
            None => at.format("%a, %b %d %I:%M %p UTC").to_string(),
        }
    }

    fn daily_brief_date_label(
        at: chrono::DateTime<chrono::Utc>,
        tz: Option<chrono_tz::Tz>,
    ) -> String {
        match tz {
            Some(tz) => at.with_timezone(&tz).format("%A, %B %-d").to_string(),
            None => at.format("%A, %B %-d").to_string(),
        }
    }

    fn daily_brief_local_hour(at: chrono::DateTime<chrono::Utc>, tz: Option<chrono_tz::Tz>) -> u32 {
        match tz {
            Some(tz) => at.with_timezone(&tz).hour(),
            None => at.hour(),
        }
    }

    fn daily_brief_period_label(
        at: chrono::DateTime<chrono::Utc>,
        tz: Option<chrono_tz::Tz>,
    ) -> &'static str {
        match Self::daily_brief_local_hour(at, tz) {
            0..=11 => "morning",
            12..=16 => "afternoon",
            _ => "evening",
        }
    }

    fn daily_brief_expected_greeting(
        at: chrono::DateTime<chrono::Utc>,
        tz: Option<chrono_tz::Tz>,
    ) -> &'static str {
        match Self::daily_brief_local_hour(at, tz) {
            0..=11 => "Good morning",
            12..=16 => "Good afternoon",
            _ => "Good evening",
        }
    }

    fn daily_brief_intro(at: chrono::DateTime<chrono::Utc>, tz: Option<chrono_tz::Tz>) -> String {
        format!(
            "{}. Here's your brief for {}.",
            Self::daily_brief_expected_greeting(at, tz),
            Self::daily_brief_date_label(at, tz)
        )
    }

    fn daily_brief_render(intro: &str, body: &str) -> String {
        let body = body.trim();
        if body.is_empty() {
            intro.trim().to_string()
        } else {
            format!("{}\n{}", intro.trim(), body)
        }
    }

    pub(super) fn daily_brief_is_visible_task(task: &task::Task) -> bool {
        !matches!(
            task.action.as_str(),
            "daily_brief" | "goal_reminder" | "goal_progress_report"
        )
    }

    fn daily_brief_format_task_line(
        task: &task::Task,
        now: chrono::DateTime<chrono::Utc>,
        tz: Option<chrono_tz::Tz>,
    ) -> String {
        let mut line = task.description.trim().to_string();
        if let Some(scheduled_for) = task.scheduled_for {
            let now_date = Self::daily_brief_local_date(now, tz);
            let due_date = Self::daily_brief_local_date(scheduled_for, tz);
            let days_delta = due_date.signed_duration_since(now_date).num_days();
            let due_note = if scheduled_for < now {
                if days_delta == 0 {
                    format!(
                        "overdue since today {}",
                        Self::daily_brief_time_label(scheduled_for, tz)
                    )
                } else if days_delta == -1 {
                    format!(
                        "overdue since yesterday {}",
                        Self::daily_brief_time_label(scheduled_for, tz)
                    )
                } else {
                    format!(
                        "overdue since {}",
                        Self::daily_brief_datetime_label(scheduled_for, tz)
                    )
                }
            } else if days_delta == 0 {
                format!(
                    "due today {}",
                    Self::daily_brief_time_label(scheduled_for, tz)
                )
            } else if days_delta == 1 {
                format!(
                    "due tomorrow {}",
                    Self::daily_brief_time_label(scheduled_for, tz)
                )
            } else {
                format!(
                    "due {}",
                    Self::daily_brief_datetime_label(scheduled_for, tz)
                )
            };
            line.push_str(&format!(" ({})", due_note));
        } else if task.cron.is_some() {
            line.push_str(" (recurring)");
        }
        line
    }

    fn daily_brief_format_trace_line(trace: &ExecutionTrace, tz: Option<chrono_tz::Tz>) -> String {
        let when = trace
            .completed_at
            .map(|at| Self::daily_brief_time_label(at, tz))
            .unwrap_or_else(|| "pending".to_string());
        format!("{} ({})", safe_truncate(&trace.message, 120), when)
    }

    fn daily_brief_format_notification_line(
        notification: &crate::storage::entities::notification::Model,
        tz: Option<chrono_tz::Tz>,
    ) -> String {
        let title = notification.title.trim();
        let body = notification.body.trim();
        let mut line = if title.is_empty() {
            "Notification".to_string()
        } else if body.is_empty() || body == title {
            title.to_string()
        } else {
            format!("{} - {}", title, safe_truncate(body, 110))
        };

        let mut meta = Vec::new();
        if !notification.level.trim().is_empty() {
            meta.push(notification.level.trim().to_string());
        }
        if !notification.source.trim().is_empty() {
            meta.push(notification.source.trim().to_string());
        }
        if let Ok(parsed) = chrono::DateTime::parse_from_rfc3339(&notification.created_at) {
            meta.push(Self::daily_brief_time_label(
                parsed.with_timezone(&chrono::Utc),
                tz,
            ));
        }
        if !meta.is_empty() {
            line.push_str(&format!(" ({})", meta.join(", ")));
        }

        safe_truncate(&line, 180)
    }

    fn daily_brief_format_failed_task_line(
        task: &task::Task,
        error: &str,
        now: chrono::DateTime<chrono::Utc>,
        tz: Option<chrono_tz::Tz>,
    ) -> String {
        let mut line = Self::daily_brief_format_task_line(task, now, tz);
        let error = error.trim();
        if !error.is_empty() {
            line.push_str(&format!(" failed: {}", safe_truncate(error, 110)));
        }
        line
    }

    fn daily_brief_compact_list(items: &[String], limit: usize) -> String {
        if items.is_empty() {
            return "none".to_string();
        }
        let mut selected = items.iter().take(limit).cloned().collect::<Vec<_>>();
        let remaining = items.len().saturating_sub(selected.len());
        if remaining > 0 {
            selected.push(format!("{} more", remaining));
        }
        selected.join("; ")
    }

    fn daily_brief_bulletize_summary(text: &str, limit: usize) -> String {
        let mut lines = Vec::new();
        for line in text.lines().map(str::trim).filter(|line| !line.is_empty()) {
            let normalized = line
                .strip_prefix("- ")
                .or_else(|| line.strip_prefix("* "))
                .or_else(|| line.strip_prefix("• "))
                .or_else(|| {
                    let (prefix, rest) = line.split_once(". ")?;
                    prefix.chars().all(|c| c.is_ascii_digit()).then_some(rest)
                })
                .unwrap_or(line)
                .trim();
            if normalized.is_empty() {
                continue;
            }
            lines.push(format!("- {}", normalized));
            if lines.len() >= limit {
                break;
            }
        }
        lines.join("\n")
    }

    fn daily_brief_build_summary_fallback(input: DailyBriefFallbackInput<'_>) -> String {
        let mut lines = Vec::new();
        let priority = if !input.overdue.is_empty() {
            format!(
                "Overdue work needs attention: {}.",
                Self::daily_brief_compact_list(input.overdue, 3)
            )
        } else if !input.failed.is_empty() {
            format!(
                "Failed work needs triage: {}.",
                Self::daily_brief_compact_list(input.failed, 2)
            )
        } else if !input.awaiting_approval.is_empty() {
            format!(
                "Approvals are blocking progress: {}.",
                Self::daily_brief_compact_list(input.awaiting_approval, 2)
            )
        } else if !input.important_events.is_empty() {
            format!(
                "Important alerts need review: {}.",
                Self::daily_brief_compact_list(input.important_events, 3)
            )
        } else if !input.due_today.is_empty() {
            format!(
                "Today's deadlines: {}.",
                Self::daily_brief_compact_list(input.due_today, 3)
            )
        } else {
            "Task queue and immediate alerts are quiet right now.".to_string()
        };
        lines.push(format!("- {}", priority));
        lines.push(format!(
            "- Workload has {} open task(s): {} pending, {} in progress, {} awaiting approval, and {} paused.",
            input.counts.open(),
            input.counts.pending,
            input.counts.in_progress,
            input.counts.awaiting_approval,
            input.counts.paused
        ));
        match input.calendar_summary {
            DailyBriefCalendarSummary::NotConnected => lines.push(
                "- Calendar is not connected, so today's meetings were not checked.".to_string(),
            ),
            DailyBriefCalendarSummary::LoadFailed => lines.push(
                "- Calendar is connected, but today's meetings could not be loaded.".to_string(),
            ),
            DailyBriefCalendarSummary::Clear => {
                lines.push("- Calendar has no events today.".to_string())
            }
            DailyBriefCalendarSummary::Meetings(items) => lines.push(format!(
                "- Calendar has {} meeting(s) today: {}.",
                items.len(),
                Self::daily_brief_compact_list(items, 3)
            )),
        }
        match input.mail_summary {
            DailyBriefMailSummary::NotConnected => lines
                .push("- Mail is not connected, so unread inbox mail was not checked.".to_string()),
            DailyBriefMailSummary::LoadFailed => lines.push(
                "- Mail is connected, but unread inbox mail could not be loaded.".to_string(),
            ),
            DailyBriefMailSummary::Clear => {
                lines.push("- Mail has no unread inbox messages from the last day.".to_string())
            }
            DailyBriefMailSummary::Messages(items) => lines.push(format!(
                "- Mail has {} unread inbox item(s): {}.",
                items.len(),
                Self::daily_brief_compact_list(items, 3)
            )),
        }
        lines.join("\n")
    }

    fn daily_brief_format_calendar_window(
        start_raw: &str,
        end_raw: &str,
        tz: Option<chrono_tz::Tz>,
    ) -> String {
        if chrono::NaiveDate::parse_from_str(start_raw, "%Y-%m-%d").is_ok()
            && chrono::NaiveDate::parse_from_str(end_raw, "%Y-%m-%d").is_ok()
        {
            return "all day".to_string();
        }

        match (
            chrono::DateTime::parse_from_rfc3339(start_raw),
            chrono::DateTime::parse_from_rfc3339(end_raw),
        ) {
            (Ok(start), Ok(end)) => {
                let start_utc = start.with_timezone(&chrono::Utc);
                let end_utc = end.with_timezone(&chrono::Utc);
                match tz {
                    Some(tz) => {
                        let start_local = start_utc.with_timezone(&tz);
                        let end_local = end_utc.with_timezone(&tz);
                        format!(
                            "{}-{} {}",
                            start_local.format("%I:%M %p"),
                            end_local.format("%I:%M %p"),
                            start_local.format("%Z")
                        )
                    }
                    None => format!(
                        "{}-{} UTC",
                        start_utc.format("%I:%M %p"),
                        end_utc.format("%I:%M %p")
                    ),
                }
            }
            _ => format!("{} to {}", start_raw, end_raw),
        }
    }

    fn daily_brief_parse_calendar_event_item(line: &str, tz: Option<chrono_tz::Tz>) -> String {
        let trimmed = line.trim();
        let (main, location) = if let Some((left, right)) = trimmed.rsplit_once(") @ ") {
            (format!("{})", left), Some(right.trim()))
        } else {
            (trimmed.to_string(), None)
        };

        if let Some((summary, times)) = main.rsplit_once(" (") {
            if let Some((start_raw, end_raw)) = times.trim_end_matches(')').split_once(" to ") {
                let mut rendered = format!(
                    "{} ({})",
                    summary.trim(),
                    Self::daily_brief_format_calendar_window(start_raw, end_raw, tz)
                );
                if let Some(location) = location.filter(|value| !value.is_empty()) {
                    rendered.push_str(&format!(" @ {}", location));
                }
                return rendered;
            }
        }

        trimmed.to_string()
    }

    pub(super) fn daily_brief_parse_calendar_events(
        raw: &str,
        tz: Option<chrono_tz::Tz>,
    ) -> Vec<String> {
        raw.lines()
            .filter_map(|line| line.trim().strip_prefix("- "))
            .map(|line| Self::daily_brief_parse_calendar_event_item(line, tz))
            .take(4)
            .collect()
    }

    fn daily_brief_parse_mail_summaries(raw: &str) -> Vec<String> {
        raw.split("\n\n")
            .filter_map(|block| {
                let mut from = String::new();
                let mut subject = String::new();
                let mut date = String::new();
                let mut snippet = String::new();
                for line in block.lines().map(str::trim) {
                    if let Some(value) = line.strip_prefix("- From:") {
                        from = value.trim().to_string();
                    } else if let Some(value) = line.strip_prefix("Subject:") {
                        subject = value.trim().to_string();
                    } else if let Some(value) = line.strip_prefix("Date:") {
                        date = value.trim().to_string();
                    } else if let Some(value) = line.strip_prefix("Snippet:") {
                        snippet = value.trim().to_string();
                    }
                }

                if from.is_empty() && subject.is_empty() && snippet.is_empty() {
                    return None;
                }

                let mut rendered = if subject.is_empty() {
                    from
                } else if from.is_empty() {
                    subject
                } else {
                    format!("{} - {}", from, subject)
                };
                if !date.is_empty() {
                    rendered.push_str(&format!(" ({})", safe_truncate(&date, 60)));
                }
                if !snippet.is_empty() {
                    rendered.push_str(&format!(": {}", safe_truncate(&snippet, 120)));
                }
                Some(safe_truncate(&rendered, 220))
            })
            .take(5)
            .collect()
    }

    fn daily_brief_format_calendar_context(summary: &DailyBriefCalendarSummary) -> String {
        match summary {
            DailyBriefCalendarSummary::NotConnected => {
                "Calendar not connected or disabled. Today's meetings were not checked.".to_string()
            }
            DailyBriefCalendarSummary::LoadFailed => {
                "Calendar appears connected, but today's meetings could not be loaded.".to_string()
            }
            DailyBriefCalendarSummary::Clear => "Calendar checked: no meetings today.".to_string(),
            DailyBriefCalendarSummary::Meetings(items) => {
                let mut lines = vec![format!(
                    "Calendar checked: {} meeting(s) today.",
                    items.len()
                )];
                lines.extend(items.iter().map(|item| format!("- {}", item)));
                lines.join("\n")
            }
        }
    }

    fn daily_brief_format_mail_context(summary: &DailyBriefMailSummary) -> String {
        match summary {
            DailyBriefMailSummary::NotConnected => {
                "Gmail/Google Workspace mail is not connected or disabled. New mail was not checked."
                    .to_string()
            }
            DailyBriefMailSummary::LoadFailed => {
                "Gmail/Google Workspace mail appears connected, but unread mail could not be loaded."
                    .to_string()
            }
            DailyBriefMailSummary::Clear => {
                "Gmail/Google Workspace mail checked: no unread inbox messages from the last day."
                    .to_string()
            }
            DailyBriefMailSummary::Messages(items) => {
                let mut lines = vec![format!(
                    "Gmail/Google Workspace mail checked: {} unread inbox item(s).",
                    items.len()
                )];
                lines.extend(items.iter().map(|item| format!("- {}", item)));
                lines.join("\n")
            }
        }
    }

    async fn load_daily_brief_calendar_summary(
        &self,
        tz: Option<chrono_tz::Tz>,
    ) -> DailyBriefCalendarSummary {
        if !self.integrations.is_enabled("google_calendar")
            && !self.integrations.is_enabled("google_workspace")
        {
            return DailyBriefCalendarSummary::NotConnected;
        }

        let has_calendar_tokens =
            crate::core::runtime::config::SecureConfigManager::new(&self.config_dir)
                .ok()
                .and_then(|manager| manager.get_custom_secret("calendar_tokens").ok().flatten())
                .is_some()
                || crate::actions::google_workspace::granted_bundles(&self.config_dir)
                    .map(|bundles| bundles.iter().any(|bundle| bundle == "calendar"))
                    .unwrap_or(false);
        if !has_calendar_tokens {
            return DailyBriefCalendarSummary::NotConnected;
        }

        match tokio::time::timeout(
            std::time::Duration::from_secs(12),
            crate::actions::calendar::calendar_today(&self.config_dir, &serde_json::json!({})),
        )
        .await
        {
            Ok(Ok(raw)) => {
                let lowered = raw.to_ascii_lowercase();
                if lowered.contains("no events found") {
                    DailyBriefCalendarSummary::Clear
                } else {
                    let meetings = Self::daily_brief_parse_calendar_events(&raw, tz);
                    if meetings.is_empty() {
                        DailyBriefCalendarSummary::LoadFailed
                    } else {
                        DailyBriefCalendarSummary::Meetings(meetings)
                    }
                }
            }
            _ => DailyBriefCalendarSummary::LoadFailed,
        }
    }

    async fn load_daily_brief_mail_summary(&self) -> DailyBriefMailSummary {
        if !self.integrations.is_enabled("gmail")
            && !self.integrations.is_enabled("google_workspace")
        {
            return DailyBriefMailSummary::NotConnected;
        }

        let has_mail_tokens =
            crate::core::runtime::config::SecureConfigManager::new(&self.config_dir)
                .ok()
                .and_then(|manager| manager.get_custom_secret("gmail_tokens").ok().flatten())
                .is_some()
                || crate::actions::google_workspace::granted_bundles(&self.config_dir)
                    .map(|bundles| bundles.iter().any(|bundle| bundle == "gmail"))
                    .unwrap_or(false);
        if !has_mail_tokens {
            return DailyBriefMailSummary::NotConnected;
        }

        let args = serde_json::json!({
            "mode": "search",
            "query": "is:unread newer_than:1d",
            "labels": ["INBOX"],
            "max_results": 5
        });
        match tokio::time::timeout(
            std::time::Duration::from_secs(15),
            crate::actions::gmail::gmail_scan(&self.config_dir, &args),
        )
        .await
        {
            Ok(Ok(raw)) => {
                if raw.to_ascii_lowercase().contains("no messages found") {
                    DailyBriefMailSummary::Clear
                } else {
                    let messages = Self::daily_brief_parse_mail_summaries(&raw);
                    if messages.is_empty() {
                        DailyBriefMailSummary::LoadFailed
                    } else {
                        DailyBriefMailSummary::Messages(messages)
                    }
                }
            }
            _ => DailyBriefMailSummary::LoadFailed,
        }
    }

    async fn build_daily_brief(&self) -> Result<String> {
        let now = chrono::Utc::now();
        let (language, tone, email_format, tz) = {
            let profile = self.user_profile.read().await;
            (
                profile.language.clone(),
                profile.tone.clone(),
                profile.email_format.clone(),
                Self::daily_brief_timezone(&profile),
            )
        };
        let generated_at = Self::daily_brief_datetime_label(now, tz);
        let local_time_label = Self::daily_brief_time_label(now, tz);
        let daypart = Self::daily_brief_period_label(now, tz);
        let expected_greeting = Self::daily_brief_expected_greeting(now, tz);

        let (
            counts,
            overdue,
            due_today,
            due_soon,
            in_progress,
            failed,
            awaiting_approval,
            paused,
            backlog,
        ) = {
            let tasks = self.tasks.read().await;
            let today = Self::daily_brief_local_date(now, tz);
            let upcoming_cutoff = today + chrono::Duration::days(3);
            let mut counts = DailyBriefTaskCounts::default();
            let mut overdue = Vec::new();
            let mut due_today = Vec::new();
            let mut due_soon = Vec::new();
            let mut in_progress = Vec::new();
            let mut failed = Vec::new();
            let mut awaiting_approval = Vec::new();
            let mut paused = Vec::new();
            let mut backlog = Vec::new();

            for task in tasks
                .all()
                .iter()
                .filter(|task| Self::daily_brief_is_visible_task(task))
            {
                match task.status {
                    task::TaskStatus::Pending => counts.pending += 1,
                    task::TaskStatus::AwaitingApproval => counts.awaiting_approval += 1,
                    task::TaskStatus::ExpiredNeedsReapproval => counts.awaiting_approval += 1,
                    task::TaskStatus::Paused => counts.paused += 1,
                    task::TaskStatus::InProgress => counts.in_progress += 1,
                    task::TaskStatus::Failed { .. } => counts.failed += 1,
                    task::TaskStatus::Completed | task::TaskStatus::Cancelled => {
                        continue;
                    }
                }

                let rendered = Self::daily_brief_format_task_line(task, now, tz);
                match task.status {
                    task::TaskStatus::Failed { ref error } => {
                        failed.push(Self::daily_brief_format_failed_task_line(
                            task, error, now, tz,
                        ));
                        continue;
                    }
                    task::TaskStatus::InProgress => {
                        in_progress.push(rendered);
                        continue;
                    }
                    task::TaskStatus::AwaitingApproval => {
                        awaiting_approval.push(rendered);
                        continue;
                    }
                    task::TaskStatus::ExpiredNeedsReapproval => {
                        awaiting_approval.push(format!("{} (needs reapproval)", rendered));
                        continue;
                    }
                    task::TaskStatus::Paused => {
                        paused.push(rendered);
                        continue;
                    }
                    _ => {}
                }

                if let Some(scheduled_for) = task.scheduled_for {
                    let due_date = Self::daily_brief_local_date(scheduled_for, tz);
                    if scheduled_for < now {
                        overdue.push(rendered);
                    } else if due_date == today {
                        due_today.push(rendered);
                    } else if due_date <= upcoming_cutoff {
                        due_soon.push(rendered);
                    } else {
                        backlog.push(rendered);
                    }
                } else {
                    backlog.push(rendered);
                }
            }

            (
                counts,
                overdue,
                due_today,
                due_soon,
                in_progress,
                failed,
                awaiting_approval,
                paused,
                backlog,
            )
        };

        let recent = {
            let trace = self.trace_history.read().await;
            trace
                .iter()
                .rev()
                .filter(|entry| {
                    let lower = entry.message.to_ascii_lowercase();
                    !lower.contains("daily brief") && !lower.contains("daily briefing")
                })
                .take(4)
                .map(|entry| Self::daily_brief_format_trace_line(entry, tz))
                .collect::<Vec<_>>()
        };

        let (calendar_summary, mail_summary) = tokio::join!(
            self.load_daily_brief_calendar_summary(tz),
            self.load_daily_brief_mail_summary()
        );

        let mut important_events = {
            let mut notifications = self
                .storage
                .list_notifications(30, 0, true)
                .await
                .unwrap_or_default();
            notifications.retain(|notification| notification.source != "daily_brief");
            notifications.sort_by_key(|notification| {
                match notification.level.trim().to_ascii_lowercase().as_str() {
                    "critical" => 0,
                    "error" => 1,
                    "warning" => 2,
                    _ => 3,
                }
            });
            notifications
                .iter()
                .take(5)
                .map(|notification| Self::daily_brief_format_notification_line(notification, tz))
                .collect::<Vec<_>>()
        };

        let watchers = self.watcher_manager.list().await;
        let watcher_attention = watchers
            .iter()
            .filter_map(|watcher| match &watcher.status {
                watcher::WatcherStatus::Triggered => Some(format!(
                    "{} triggered",
                    safe_truncate(watcher.description.trim(), 120)
                )),
                watcher::WatcherStatus::TimedOut => Some(format!(
                    "{} timed out",
                    safe_truncate(watcher.description.trim(), 120)
                )),
                watcher::WatcherStatus::Failed { error } => Some(format!(
                    "{} failed: {}",
                    safe_truncate(watcher.description.trim(), 90),
                    safe_truncate(error.trim(), 90)
                )),
                watcher::WatcherStatus::Paused => Some(format!(
                    "{} paused",
                    safe_truncate(watcher.description.trim(), 120)
                )),
                watcher::WatcherStatus::Active if watcher.consecutive_failures > 0 => {
                    Some(format!(
                        "{} has {} consecutive poll failure(s)",
                        safe_truncate(watcher.description.trim(), 90),
                        watcher.consecutive_failures
                    ))
                }
                _ => None,
            })
            .filter(|description| !description.trim().is_empty())
            .collect::<Vec<_>>();
        if !watcher_attention.is_empty() {
            important_events.push(format!(
                "Watcher attention: {}",
                Self::daily_brief_compact_list(&watcher_attention, 3)
            ));
        }

        let security_snapshot = self.security_events.snapshot();
        if security_snapshot.has_events() {
            let mut security_parts = Vec::new();
            if security_snapshot.injection_attempts > 0 {
                security_parts.push(format!(
                    "{} injection attempt(s)",
                    security_snapshot.injection_attempts
                ));
            }
            if security_snapshot.auth_failures > 0 {
                security_parts.push(format!(
                    "{} auth failure(s)",
                    security_snapshot.auth_failures
                ));
            }
            if security_snapshot.rate_limit_hits > 0 {
                security_parts.push(format!(
                    "{} rate limit hit(s)",
                    security_snapshot.rate_limit_hits
                ));
            }
            if security_snapshot.unauthorized_channel_attempts > 0 {
                security_parts.push(format!(
                    "{} unauthorized channel attempt(s)",
                    security_snapshot.unauthorized_channel_attempts
                ));
            }
            if !security_parts.is_empty() {
                important_events.push(format!("Security: {}", security_parts.join(", ")));
            }
        }

        let mut module_events = Vec::new();
        let integrations = self.integrations.list().await;
        let integration_attention = integrations
            .iter()
            .filter(|info| self.integrations.is_enabled(&info.id))
            .filter_map(|info| match &info.status {
                crate::integrations::IntegrationStatus::NeedsAuth => Some(format!(
                    "{} needs auth",
                    safe_truncate(info.name.trim(), 80)
                )),
                crate::integrations::IntegrationStatus::Error(message) => Some(format!(
                    "{} error: {}",
                    safe_truncate(info.name.trim(), 70),
                    safe_truncate(message.trim(), 100)
                )),
                _ => None,
            })
            .collect::<Vec<_>>();
        if !integration_attention.is_empty() {
            module_events.push(format!(
                "Integrations needing attention: {}",
                Self::daily_brief_compact_list(&integration_attention, 4)
            ));
        }

        let extension_pack_attention = {
            let registry = self.extension_packs.read().await;
            registry
                .list_installed(None)
                .await
                .unwrap_or_default()
                .iter()
                .filter_map(|pack| {
                    if pack.needs_auth {
                        return Some(format!(
                            "{} needs auth",
                            safe_truncate(pack.manifest.name.trim(), 80)
                        ));
                    }
                    if pack.status == "error" {
                        return Some(format!(
                            "{} error{}",
                            safe_truncate(pack.manifest.name.trim(), 80),
                            pack.status_detail
                                .as_deref()
                                .map(|detail| format!(": {}", safe_truncate(detail.trim(), 100)))
                                .unwrap_or_default()
                        ));
                    }
                    if pack.enabled && pack.status == "draft" {
                        return Some(format!(
                            "{} is a draft pack awaiting review",
                            safe_truncate(pack.manifest.name.trim(), 80)
                        ));
                    }
                    if pack.enabled && pack.verification_status != "verified" {
                        return Some(format!(
                            "{} is {}",
                            safe_truncate(pack.manifest.name.trim(), 80),
                            safe_truncate(pack.verification_status.trim(), 80)
                        ));
                    }
                    None
                })
                .collect::<Vec<_>>()
        };
        if !extension_pack_attention.is_empty() {
            module_events.push(format!(
                "Extension packs needing attention: {}",
                Self::daily_brief_compact_list(&extension_pack_attention, 4)
            ));
        }

        let plugin_attention = {
            let registry = self.plugins.read().await;
            registry
                .list_plugins()
                .await
                .unwrap_or_default()
                .iter()
                .filter_map(|plugin| {
                    if let Some(error) = plugin.plugin.last_error.as_deref() {
                        return Some(format!(
                            "{} error: {}",
                            safe_truncate(plugin.plugin.manifest.name.trim(), 70),
                            safe_truncate(error.trim(), 100)
                        ));
                    }
                    if plugin.plugin.enabled
                        && !plugin.token_configured
                        && !matches!(
                            plugin.plugin.auth_mode,
                            crate::plugins::registry::PluginAuthMode::None
                        )
                    {
                        return Some(format!(
                            "{} is enabled but missing auth token",
                            safe_truncate(plugin.plugin.manifest.name.trim(), 80)
                        ));
                    }
                    None
                })
                .collect::<Vec<_>>()
        };
        if !plugin_attention.is_empty() {
            module_events.push(format!(
                "Plugins needing attention: {}",
                Self::daily_brief_compact_list(&plugin_attention, 4)
            ));
        }

        let mcp_attention = {
            let registry = self.mcp.read().await;
            registry
                .list_servers(false)
                .await
                .unwrap_or_default()
                .iter()
                .filter_map(|server| {
                    if let Some(error) = server.last_error.as_deref() {
                        return Some(format!(
                            "{} error: {}",
                            safe_truncate(server.name.trim(), 70),
                            safe_truncate(error.trim(), 100)
                        ));
                    }
                    if !server.warnings.is_empty() {
                        return Some(format!(
                            "{} warning: {}",
                            safe_truncate(server.name.trim(), 70),
                            safe_truncate(&server.warnings.join("; "), 100)
                        ));
                    }
                    if server.enabled && server.tool_count == 0 && server.resource_count == 0 {
                        return Some(format!(
                            "{} is enabled but exposes no tools or resources",
                            safe_truncate(server.name.trim(), 80)
                        ));
                    }
                    None
                })
                .collect::<Vec<_>>()
        };
        if !mcp_attention.is_empty() {
            module_events.push(format!(
                "MCP servers needing attention: {}",
                Self::daily_brief_compact_list(&mcp_attention, 4)
            ));
        }

        let startup_issues = self.startup_issues.read().await;
        let startup_issue_count = startup_issues.len();
        if !startup_issues.is_empty() {
            let summaries = startup_issues
                .iter()
                .take(3)
                .map(|issue| {
                    format!(
                        "{} [{}]: {}",
                        safe_truncate(issue.subsystem.trim(), 50),
                        issue.severity.trim(),
                        safe_truncate(issue.summary.trim(), 100)
                    )
                })
                .collect::<Vec<_>>();
            module_events.push(format!(
                "Startup issues needing attention: {}",
                summaries.join("; ")
            ));
        }
        drop(startup_issues);

        let background_sessions = self.background_sessions.list().await;
        let attention_background_sessions = background_sessions
            .iter()
            .filter(|session| {
                matches!(
                    session.status,
                    background_session::BackgroundSessionStatus::Waiting
                        | background_session::BackgroundSessionStatus::NeedsInput
                        | background_session::BackgroundSessionStatus::Paused
                        | background_session::BackgroundSessionStatus::Failed
                ) || (!session.status.is_closed()
                    && now
                        .signed_duration_since(session.last_activity_at)
                        .num_hours()
                        <= 30)
            })
            .collect::<Vec<_>>();
        if !attention_background_sessions.is_empty() {
            let summaries = attention_background_sessions
                .iter()
                .take(3)
                .map(|session| {
                    let focus = session
                        .current_focus
                        .as_deref()
                        .or(session.waiting_on.as_deref())
                        .or(session.next_expected_action.as_deref())
                        .unwrap_or(session.objective.as_str());
                    format!(
                        "{} [{}]: {}",
                        safe_truncate(session.title.trim(), 60),
                        session.status.label(),
                        safe_truncate(focus.trim(), 100)
                    )
                })
                .collect::<Vec<_>>();
            module_events.push(format!(
                "Background sessions needing attention: {}",
                summaries.join("; ")
            ));
        }

        let browser_sessions = self.browser_sessions.list_sessions();
        let active_browser_sessions = browser_sessions
            .iter()
            .filter(|(_, _, status)| {
                matches!(
                    status.as_str(),
                    "active" | "waiting_for_operator" | "operator_claimed" | "awaiting_resume"
                )
            })
            .collect::<Vec<_>>();
        if !active_browser_sessions.is_empty() {
            let summaries = active_browser_sessions
                .iter()
                .take(3)
                .map(|(_, task, status)| {
                    format!("{} [{}]", safe_truncate(task.trim(), 100), status)
                })
                .collect::<Vec<_>>();
            module_events.push(format!(
                "Browser sessions needing attention: {}",
                summaries.join("; ")
            ));
        }

        if let Some(ref swarm) = self.swarm {
            let status = swarm.status().await;
            if status.active_agents > 0 {
                module_events.push("Swarm has specialist work in progress".to_string());
            }
        }

        if let Ok(delegations) = self.storage.get_active_swarm_delegations(5).await {
            if !delegations.is_empty() {
                let summaries = delegations
                    .iter()
                    .take(3)
                    .map(|delegation| safe_truncate(delegation.task_description.trim(), 100))
                    .collect::<Vec<_>>();
                module_events.push(format!("Active swarm delegation: {}", summaries.join("; ")));
            }
        }

        if let Ok(apps) =
            tokio::time::timeout(std::time::Duration::from_secs(8), self.app_registry.list()).await
        {
            if !apps.is_empty() {
                let app_attention = apps
                    .iter()
                    .filter_map(|row| {
                        let title = row
                            .get("title")
                            .and_then(|value| value.as_str())
                            .unwrap_or("App");
                        let enabled = row
                            .get("enabled")
                            .and_then(|value| value.as_bool())
                            .unwrap_or(true);
                        let running = row
                            .get("running")
                            .and_then(|value| value.as_bool())
                            .unwrap_or(false);
                        let restore_status = row
                            .get("restore_status")
                            .and_then(|value| value.as_str())
                            .unwrap_or("ready");
                        let restore_error =
                            row.get("restore_error").and_then(|value| value.as_str());

                        if restore_status == "degraded" {
                            return Some(format!(
                                "{} degraded{}",
                                safe_truncate(title.trim(), 80),
                                restore_error
                                    .map(|error| format!(": {}", safe_truncate(error.trim(), 100)))
                                    .unwrap_or_default()
                            ));
                        }
                        if restore_status == "restoring" {
                            return Some(format!(
                                "{} is restoring",
                                safe_truncate(title.trim(), 80)
                            ));
                        }
                        if enabled && !running {
                            return Some(format!(
                                "{} is enabled but not running",
                                safe_truncate(title.trim(), 80)
                            ));
                        }
                        None
                    })
                    .collect::<Vec<_>>();
                if !app_attention.is_empty() {
                    module_events.push(format!(
                        "Apps needing attention: {}",
                        Self::daily_brief_compact_list(&app_attention, 4)
                    ));
                }
            }
        } else {
            module_events.push("Apps: status check timed out".to_string());
        }

        let (
            fact_count,
            document_count,
            preference_count,
            user_data_count,
            knowledge_count,
            document_chunk_count,
        ) = tokio::join!(
            self.storage.count_facts(None),
            self.storage.count_documents(None),
            self.storage.count_user_preferences(None),
            self.storage.count_user_data_items(None, None),
            self.storage.count_visible_knowledge_items(None),
            self.storage.count_document_chunks()
        );
        let fact_count = fact_count.unwrap_or(0);
        let document_count = document_count.unwrap_or(0);
        let preference_count = preference_count.unwrap_or(0);
        let user_data_count = user_data_count.unwrap_or(0);
        let knowledge_count = knowledge_count.unwrap_or(0);
        let document_chunk_count = document_chunk_count.unwrap_or(0);

        let learning_queue = self
            .storage
            .learning_queue_counts()
            .await
            .unwrap_or_default();
        let learning_enabled =
            crate::core::knowledge::learning::load_learning_enabled(&self.storage).await;
        let self_evolve_enabled_pref = self
            .storage
            .get(crate::core::self_evolve::strategy_runtime::SELF_EVOLVE_ENABLED_KEY)
            .await
            .ok()
            .flatten()
            .and_then(|raw| String::from_utf8(raw).ok())
            .map(|value| !value.trim().eq_ignore_ascii_case("false"))
            .unwrap_or(true);
        let self_evolve_enabled = learning_enabled && self_evolve_enabled_pref;
        let mut active_canary_count = 0usize;
        for key in [
            crate::core::self_evolve::strategy_runtime::ROUTING_COMPLEXITY_CANARY_STATE_KEY,
            crate::core::self_evolve::strategy_runtime::TOOL_STRATEGY_CANARY_STATE_KEY,
            crate::core::self_evolve::PROMPT_BUNDLE_CANARY_STATE_KEY,
            crate::core::self_evolve::SPECIALIST_PROMPT_BUNDLE_CANARY_STATE_KEY,
            crate::core::model::prompt_fragments::PROMPT_FRAGMENT_BUNDLE_CANARY_STATE_KEY,
        ] {
            if let Ok(Some(raw)) = self.storage.get(key).await {
                if serde_json::from_slice::<
                    crate::core::self_evolve::strategy_runtime::CanaryRolloutState,
                >(&raw)
                .map(|state| state.enabled)
                .unwrap_or(false)
                {
                    active_canary_count += 1;
                }
            }
        }

        let reflect_from = (now - chrono::Duration::hours(24)).to_rfc3339();
        let reflect_to = now.to_rfc3339();
        let reflect_units = self
            .storage
            .list_semantic_work_units_between(&reflect_from, &reflect_to, 500)
            .await
            .unwrap_or_default();
        let reflect_message_count: i32 = reflect_units
            .iter()
            .map(|unit| unit.message_count.max(0))
            .sum();

        let (arkpulse_total, latest_arkpulse_events) = tokio::join!(
            self.storage.count_arkpulse_events(),
            self.storage.list_arkpulse_events(1)
        );
        let arkpulse_total = arkpulse_total.unwrap_or(0);
        let latest_arkpulse = latest_arkpulse_events
            .unwrap_or_default()
            .into_iter()
            .next();
        let arkpulse_summary = latest_arkpulse
            .as_ref()
            .map(|event| {
                let when = chrono::DateTime::parse_from_rfc3339(&event.timestamp)
                    .map(|timestamp| {
                        Self::daily_brief_time_label(timestamp.with_timezone(&chrono::Utc), tz)
                    })
                    .unwrap_or_else(|_| "time unknown".to_string());
                let status = event.status.trim();
                let status = if status.is_empty() {
                    "recorded"
                } else {
                    status
                };
                format!("{} at {}", status, when)
            })
            .unwrap_or_else(|| "no previous run".to_string());

        let security_event_count = security_snapshot.injection_attempts
            + security_snapshot.auth_failures
            + security_snapshot.rate_limit_hits
            + security_snapshot.unauthorized_channel_attempts;
        let arkcore_items = [
            format!(
                "Ark Core: {} open task(s), {} active background session(s), and {} active browser session(s).",
                counts.open(),
                background_sessions
                    .iter()
                    .filter(|session| !session.status.is_closed())
                    .count(),
                active_browser_sessions.len()
            ),
            format!(
                "Memory: {} fact(s), {} preference(s), {} document(s), {} chunk(s), {} user-data item(s), and {} knowledge item(s).",
                fact_count,
                preference_count,
                document_count,
                document_chunk_count,
                user_data_count,
                knowledge_count
            ),
            format!(
                "Sentinel: {} watcher(s), {} watcher attention item(s), {} security event(s), and {} startup issue(s).",
                watchers.len(),
                watcher_attention.len(),
                security_event_count,
                startup_issue_count
            ),
            format!(
                "Evolve: {}, {} draft candidate(s), {} pending consolidation item(s), {} pending reflection item(s), {} active pattern(s), and {} active canary run(s).",
                if self_evolve_enabled {
                    "enabled"
                } else {
                    "paused"
                },
                learning_queue.draft_candidates,
                learning_queue.pending_consolidation,
                learning_queue.pending_reflection,
                learning_queue.active_patterns,
                active_canary_count
            ),
            format!(
                "Reflect: {} semantic work unit(s) and {} source message(s) indexed from the last 24 hours.",
                reflect_units.len(),
                reflect_message_count
            ),
            format!(
                "Pulse: {} stored run(s); latest status is {}.",
                arkpulse_total, arkpulse_summary
            ),
        ];

        let mut detail_lines = vec![format!(
            "- Tasks: {} open task(s), {} pending, {} in progress, {} awaiting approval, {} paused, and {} failed.",
            counts.open(),
            counts.pending,
            counts.in_progress,
            counts.awaiting_approval,
            counts.paused,
            counts.failed
        )];
        if !overdue.is_empty() {
            detail_lines.push(format!(
                "- Overdue: {}.",
                Self::daily_brief_compact_list(&overdue, 4)
            ));
        }
        if !due_today.is_empty() {
            detail_lines.push(format!(
                "- Due today: {}.",
                Self::daily_brief_compact_list(&due_today, 4)
            ));
        }
        if !due_soon.is_empty() {
            detail_lines.push(format!(
                "- Due soon: {}.",
                Self::daily_brief_compact_list(&due_soon, 3)
            ));
        }
        match &calendar_summary {
            DailyBriefCalendarSummary::NotConnected => detail_lines.push(
                "- Calendar: not connected or disabled; today's meetings were not checked."
                    .to_string(),
            ),
            DailyBriefCalendarSummary::LoadFailed => detail_lines.push(
                "- Calendar: connected, but today's meetings could not be loaded.".to_string(),
            ),
            DailyBriefCalendarSummary::Clear => {
                detail_lines.push("- Calendar: no events today.".to_string())
            }
            DailyBriefCalendarSummary::Meetings(items) => detail_lines.push(format!(
                "- Calendar: {} meeting(s) today: {}.",
                items.len(),
                Self::daily_brief_compact_list(items, 3)
            )),
        }
        match &mail_summary {
            DailyBriefMailSummary::NotConnected => detail_lines.push(
                "- Mail: Gmail/Google Workspace is not connected or disabled; unread mail was not checked."
                    .to_string(),
            ),
            DailyBriefMailSummary::LoadFailed => detail_lines.push(
                "- Mail: connected, but unread inbox mail could not be loaded.".to_string(),
            ),
            DailyBriefMailSummary::Clear => detail_lines
                .push("- Mail: no unread inbox messages from the last day.".to_string()),
            DailyBriefMailSummary::Messages(items) => detail_lines.push(format!(
                "- Mail: {} unread inbox item(s): {}.",
                items.len(),
                Self::daily_brief_compact_list(items, 3)
            )),
        }
        if important_events.is_empty() {
            detail_lines.push(
                "- Important events: no unread AgentArk alerts, watcher issues, or security counters."
                    .to_string(),
            );
        } else {
            detail_lines.push(format!(
                "- Important events: {}.",
                Self::daily_brief_compact_list(&important_events, 5)
            ));
        }
        detail_lines.extend(arkcore_items.iter().map(|item| format!("- {}", item)));
        if module_events.is_empty() {
            detail_lines.push("- Module attention: no module-level attention signals.".to_string());
        } else {
            detail_lines.push(format!(
                "- Module attention: {}.",
                Self::daily_brief_compact_list(&module_events, 5)
            ));
        }
        if recent.is_empty() {
            detail_lines.push("- Recent execution: no recent runs recorded.".to_string());
        } else {
            detail_lines.push(format!(
                "- Recent execution: {}.",
                Self::daily_brief_compact_list(&recent, 4)
            ));
        }

        let mut style = Vec::new();
        if let Some(lang) = language.as_ref().filter(|value| !value.trim().is_empty()) {
            style.push(format!("Language: {}", lang.trim()));
        }
        if let Some(tone) = tone.as_ref().filter(|value| !value.trim().is_empty()) {
            style.push(format!("Tone: {}", tone.trim()));
        }
        if let Some(format) = email_format
            .as_ref()
            .filter(|value| !value.trim().is_empty())
        {
            style.push(format!("Format: {}", format.trim()));
        }
        let style_block = if style.is_empty() {
            "Use a neutral, direct tone.".to_string()
        } else {
            style.join(" | ")
        };

        let mut task_lines = vec![format!(
            "- Open tasks: {} total ({} pending, {} in progress, {} awaiting approval, {} paused)",
            counts.open(),
            counts.pending,
            counts.in_progress,
            counts.awaiting_approval,
            counts.paused
        )];
        if failed.is_empty() {
            task_lines.push("- Failed: none".to_string());
        } else {
            task_lines.push("- Failed:".to_string());
            task_lines.extend(failed.iter().take(3).map(|item| format!("  - {}", item)));
        }
        if in_progress.is_empty() {
            task_lines.push("- In progress: none".to_string());
        } else {
            task_lines.push("- In progress:".to_string());
            task_lines.extend(
                in_progress
                    .iter()
                    .take(3)
                    .map(|item| format!("  - {}", item)),
            );
        }
        if overdue.is_empty() {
            task_lines.push("- Overdue: none".to_string());
        } else {
            task_lines.push("- Overdue:".to_string());
            task_lines.extend(overdue.iter().take(4).map(|item| format!("  - {}", item)));
        }
        if due_today.is_empty() {
            task_lines.push("- Due today: none".to_string());
        } else {
            task_lines.push("- Due today:".to_string());
            task_lines.extend(due_today.iter().take(4).map(|item| format!("  - {}", item)));
        }
        if due_soon.is_empty() {
            task_lines.push("- Due in the next 3 days: none".to_string());
        } else {
            task_lines.push("- Due in the next 3 days:".to_string());
            task_lines.extend(due_soon.iter().take(3).map(|item| format!("  - {}", item)));
        }
        if awaiting_approval.is_empty() {
            task_lines.push("- Awaiting approval: none".to_string());
        } else {
            task_lines.push("- Awaiting approval:".to_string());
            task_lines.extend(
                awaiting_approval
                    .iter()
                    .take(3)
                    .map(|item| format!("  - {}", item)),
            );
        }
        if paused.is_empty() {
            task_lines.push("- Paused: none".to_string());
        } else {
            task_lines.push("- Paused:".to_string());
            task_lines.extend(paused.iter().take(3).map(|item| format!("  - {}", item)));
        }
        if backlog.is_empty() {
            task_lines.push("- Backlog: none".to_string());
        } else {
            task_lines.push("- Backlog candidates:".to_string());
            task_lines.extend(backlog.iter().take(3).map(|item| format!("  - {}", item)));
        }

        let recent_block = if recent.is_empty() {
            "No recent runs recorded.".to_string()
        } else {
            recent
                .iter()
                .map(|item| format!("- {}", item))
                .collect::<Vec<_>>()
                .join("\n")
        };
        let calendar_block = Self::daily_brief_format_calendar_context(&calendar_summary);
        let mail_block = Self::daily_brief_format_mail_context(&mail_summary);
        let events_block = if important_events.is_empty() {
            "No unread AgentArk alerts, watcher issues, or security counters.".to_string()
        } else {
            important_events
                .iter()
                .map(|item| format!("- {}", item))
                .collect::<Vec<_>>()
                .join("\n")
        };
        let modules_block = if module_events.is_empty() {
            "No module-level attention signals.".to_string()
        } else {
            module_events
                .iter()
                .map(|item| format!("- {}", item))
                .collect::<Vec<_>>()
                .join("\n")
        };
        let arkcore_block = arkcore_items
            .iter()
            .map(|item| format!("- {}", item))
            .collect::<Vec<_>>()
            .join("\n");

        let prompt = format!(
            "Create the top summary for a daily command brief.\n{}\n\nBrief requirements:\n- This run was generated at the user's local time shown below; the deterministic wrapper will handle the greeting, so do not write a greeting, title, sign-off, or section heading.\n- Return 2-4 compact bullet points only.\n- Summarize the highest-impact facts across tasks, approvals, failures, alerts, meetings, mail, monitoring, Ark Core modules, and recent execution.\n- Make the summary useful even when queues are quiet by naming what was checked and what was not connected or unavailable.\n- Use only the facts below. Do not invent external news, events, counts, services, or statuses.\n\nGenerated at:\n{}\nLocal time:\n{}\nLocal daypart:\n{}\nExpected wrapper greeting:\n{}\n\nTask snapshot:\n{}\n\nImportant events:\n{}\n\nArk Core status facts:\n{}\n\nModule attention signals:\n{}\n\nRecent execution:\n{}\n\nCalendar:\n{}\n\nMail:\n{}",
            style_block,
            generated_at,
            local_time_label,
            daypart,
            expected_greeting,
            task_lines.join("\n"),
            events_block,
            arkcore_block,
            modules_block,
            recent_block,
            calendar_block,
            mail_block
        );

        let empty_actions: Vec<crate::actions::ActionDef> = Vec::new();
        let Some(response) = self
            .supervised_internal_chat(
                "daily_brief",
                "daily_brief",
                "daily_brief",
                &ModelRole::Primary,
                vec![],
                "You are a concise assistant creating factual daily command brief summaries.",
                &prompt,
                &[],
                &empty_actions,
                internal_llm_timeout_ms("AGENTARK_DAILY_BRIEF_TIMEOUT_MS", 20_000),
                2,
            )
            .await
        else {
            let summary = Self::daily_brief_build_summary_fallback(DailyBriefFallbackInput {
                counts,
                overdue: &overdue,
                due_today: &due_today,
                failed: &failed,
                awaiting_approval: &awaiting_approval,
                important_events: &important_events,
                calendar_summary: &calendar_summary,
                mail_summary: &mail_summary,
            });
            let body = format!(
                "**Summary**\n{}\n\n**Details**\n{}",
                summary,
                detail_lines.join("\n")
            );
            return Ok(Self::daily_brief_render(
                &Self::daily_brief_intro(now, tz),
                &body,
            ));
        };

        let content = response.content.trim().to_string();
        let summary = if content.is_empty() {
            Self::daily_brief_build_summary_fallback(DailyBriefFallbackInput {
                counts,
                overdue: &overdue,
                due_today: &due_today,
                failed: &failed,
                awaiting_approval: &awaiting_approval,
                important_events: &important_events,
                calendar_summary: &calendar_summary,
                mail_summary: &mail_summary,
            })
        } else {
            let bulletized = Self::daily_brief_bulletize_summary(&content, 4);
            if bulletized.trim().is_empty() {
                Self::daily_brief_build_summary_fallback(DailyBriefFallbackInput {
                    counts,
                    overdue: &overdue,
                    due_today: &due_today,
                    failed: &failed,
                    awaiting_approval: &awaiting_approval,
                    important_events: &important_events,
                    calendar_summary: &calendar_summary,
                    mail_summary: &mail_summary,
                })
            } else {
                bulletized
            }
        };
        let body = format!(
            "**Summary**\n{}\n\n**Details**\n{}",
            summary,
            detail_lines.join("\n")
        );
        Ok(Self::daily_brief_render(
            &Self::daily_brief_intro(now, tz),
            &body,
        ))
    }

    async fn dispatch_daily_brief_push_report(
        &self,
        brief: &str,
        report_to: Option<&str>,
    ) -> Vec<NotificationDispatchOutcome> {
        let requested = report_to
            .map(str::trim)
            .filter(|value| !value.is_empty())
            .map(|value| value.to_ascii_lowercase());
        let Some(requested) = requested else {
            let attempts = self
                .notify_preferred_channel_reported_with_hint(brief, None, true)
                .await;
            return Self::daily_brief_push_attempts_without_web_fallback(attempts);
        };

        if requested == "preferred" {
            let attempts = self
                .notify_preferred_channel_reported_with_hint(brief, None, true)
                .await;
            return Self::daily_brief_push_attempts_without_web_fallback(attempts);
        }

        let mut attempts = Vec::new();
        if is_external_notification_channel(&requested)
            && !self
                .notification_channel_is_configured_any(&requested)
                .await
        {
            attempts.push(NotificationDispatchOutcome::pre_send_failure(
                requested.clone(),
                format!(
                    "{} delivery is not connected",
                    notification_channel_display_name(&requested)
                ),
            ));
        }

        attempts.extend(
            self.notify_preferred_channel_reported_with_hint(brief, Some(&requested), true)
                .await,
        );
        Self::daily_brief_push_attempts_without_web_fallback(attempts)
    }

    fn daily_brief_push_attempts_without_web_fallback(
        attempts: Vec<NotificationDispatchOutcome>,
    ) -> Vec<NotificationDispatchOutcome> {
        if attempts
            .iter()
            .any(|attempt| attempt.channel != "web" && attempt.channel != "in_app")
        {
            return attempts;
        }
        vec![NotificationDispatchOutcome::pre_send_failure(
            "push",
            "No connected notification integrations available",
        )]
    }

    pub(super) async fn run_daily_brief_reported_with_hint(
        &self,
        report_to: Option<&str>,
    ) -> Result<DailyBriefRunResult> {
        let brief = self.build_daily_brief().await?;
        let in_app = NotificationDispatchOutcome::pre_send_failure(
            "web",
            "Daily brief generation does not create in-app notifications",
        );
        let push_attempts = self
            .dispatch_daily_brief_push_report(&brief, report_to)
            .await;
        Ok(DailyBriefRunResult {
            brief,
            in_app,
            push_attempts,
        })
    }
}
